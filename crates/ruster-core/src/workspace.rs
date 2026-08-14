use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::action::Action;
use crate::buffer::Buffer;
use crate::cursor::CursorSet;
use crate::document::{BufferId, DocKind, Document, SpecialKind};
use crate::editor::{EditSession, EditorView};
use crate::windows::{SplitDir, Window, WindowId, WindowTree};

/// Whether an action changes buffer text (and so is refused on a read-only
/// document). Navigation, scrolling, cursor, and batch markers are not mutating.
fn action_mutates(a: &Action) -> bool {
    matches!(
        a,
        Action::Edit(_)
            | Action::IndentLine
            | Action::DeindentLine
            | Action::Undo
            | Action::Redo
            | Action::UndoTime(_)
            | Action::Textobject { .. }
    )
}

/// Registry of all open [`Document`]s (vim "buffers").
///
/// Ids are stable for the lifetime of a document. [`order`](Self::ids) tracks
/// creation order and is used by buffer-cycling commands and the buffer list.
pub struct BufferStore {
    docs: HashMap<BufferId, Document>,
    order: Vec<BufferId>,
    next: u32,
}

/// Best-effort canonicalization for path identity. Falls back to the path as
/// given when the file does not yet exist on disk.
fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

impl BufferStore {
    pub fn new() -> Self {
        BufferStore {
            docs: HashMap::new(),
            order: Vec::new(),
            next: 1,
        }
    }

    fn alloc_id(&mut self) -> BufferId {
        let id = BufferId(self.next);
        self.next += 1;
        id
    }

    fn insert(&mut self, doc: Document) -> BufferId {
        let id = self.alloc_id();
        self.docs.insert(id, doc);
        self.order.push(id);
        id
    }

    /// Open `path` with `content` already read from disk. If a document for the
    /// same (canonical) path is already open, returns its existing id and
    /// ignores `content`.
    pub fn open_file(&mut self, path: PathBuf, content: String) -> BufferId {
        let target = canonical(&path);
        for (id, doc) in &self.docs {
            if matches!(doc.kind, DocKind::File) {
                if let Some(existing) = &doc.file_path {
                    if canonical(existing) == target {
                        return *id;
                    }
                }
            }
        }
        self.insert(Document::from_file(path, &content))
    }

    /// Create a new unnamed scratch document.
    pub fn create_scratch(&mut self, name: &str) -> BufferId {
        self.insert(Document::scratch(name))
    }

    /// Create a new ruster-managed special document (ibuffer, dired, picker).
    pub fn create_special(&mut self, kind: SpecialKind, name: &str) -> BufferId {
        self.insert(Document::special(kind, name))
    }

    pub fn get(&self, id: BufferId) -> Option<&Document> {
        self.docs.get(&id)
    }

    pub fn get_mut(&mut self, id: BufferId) -> Option<&mut Document> {
        self.docs.get_mut(&id)
    }

    /// Close a document. Refuses to close the last remaining document while it
    /// is modified (returns `false`, leaving the store untouched). Also refuses
    /// to close pinned documents.
    pub fn close(&mut self, id: BufferId) -> bool {
        let doc = match self.docs.get(&id) {
            Some(d) => d,
            None => return false,
        };
        if doc.pinned {
            return false;
        }
        if self.docs.len() == 1 && doc.modified {
            return false;
        }
        self.docs.remove(&id);
        self.order.retain(|&x| x != id);
        true
    }

    /// All open ids, in creation order.
    pub fn ids(&self) -> &[BufferId] {
        &self.order
    }

    pub fn len(&self) -> usize {
        self.docs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }
}

impl Default for BufferStore {
    fn default() -> Self {
        Self::new()
    }
}

/// The whole editing workspace: every open [`Document`] plus the tree of
/// windows viewing them. This is the single shared object the app and the Lua
/// runtime both operate on. Editing always targets the *active window's*
/// buffer, using the active window's per-window cursor set.
pub struct Workspace {
    pub buffers: BufferStore,
    pub windows: WindowTree,
}

impl Workspace {
    /// Create a workspace with `path`/`content` opened in a single window.
    pub fn from_file(path: PathBuf, content: String) -> Self {
        let mut buffers = BufferStore::new();
        let id = buffers.open_file(path, content);
        let windows = WindowTree::single(id);
        Workspace { buffers, windows }
    }

    /// Create a workspace with a single pinned Dashboard buffer (shows the
    /// welcome screen).
    pub fn scratch() -> Self {
        let mut buffers = BufferStore::new();
        let id = buffers.create_special(SpecialKind::Dashboard, "Dashboard");
        if let Some(doc) = buffers.get_mut(id) {
            doc.pinned = true;
        }
        let windows = WindowTree::single(id);
        Workspace { buffers, windows }
    }

    /// The buffer id of the active window.
    pub fn active_buffer(&self) -> BufferId {
        self.windows.active_window().buffer
    }

    pub fn active_window(&self) -> &Window {
        self.windows.active_window()
    }

    pub fn active_doc(&self) -> &Document {
        let id = self.active_buffer();
        self.buffers.get(id).expect("active buffer exists")
    }

    pub fn active_doc_mut(&mut self) -> &mut Document {
        let id = self.active_buffer();
        self.buffers.get_mut(id).expect("active buffer exists")
    }

    /// Head char offset of the active window's primary cursor.
    pub fn primary_head(&self) -> usize {
        self.active_window().cursors.head()
    }

    /// Run an editing action against the active window/document.
    ///
    /// Actions that change buffer text (see [`action_mutates`]) are no-ops on a
    /// read-only document; navigation actions always apply.
    pub fn execute(&mut self, action: Action) {
        if let Action::Scroll(delta) = action {
            let win = self.windows.active_window_mut();
            win.scroll_top = win.scroll_top.saturating_add_signed(delta as isize);
            let last = self
                .buffers
                .get(self.windows.active_window().buffer)
                .map(|d| d.buffer.line_count().saturating_sub(1))
                .unwrap_or(0);
            let win = self.windows.active_window_mut();
            win.scroll_top = win.scroll_top.min(last);
            return;
        }
        if let Action::ScrollHorizontal(delta) = action {
            self.scroll_columns(self.windows.active(), delta);
            return;
        }
        // Read-only (ruster-managed) buffers ignore mutating actions, so global
        // keys can fall through to them safely — search and motion still work.
        let read_only = self
            .buffers
            .get(self.windows.active_window().buffer)
            .map(Document::read_only)
            .unwrap_or(false);
        if read_only && action_mutates(&action) {
            return;
        }
        let marks_modified = matches!(
            action,
            Action::Edit(_)
                | Action::IndentLine
                | Action::DeindentLine
                | Action::Undo
                | Action::Redo
        );
        // Disjoint field borrows: windows for the cursor set, buffers for the doc.
        let win = self.windows.active_window_mut();
        let doc = self
            .buffers
            .get_mut(win.buffer)
            .expect("active buffer exists");
        EditSession::new(
            &mut doc.buffer,
            &mut win.cursors,
            &mut doc.undo,
            &doc.indent,
        )
        .execute(action);
        if marks_modified {
            doc.modified = true;
        }
    }

    /// Scroll `wid` sideways by `delta` columns, dragging its cursor only as far
    /// as it must go to stay in view.
    ///
    /// Moving the cursor at all looks heavy-handed for a scroll command, but a
    /// scroll that strands the cursor off-screen does not survive: the renderer
    /// clamps `scroll_left` to keep the cursor visible and writes the clamp
    /// back, so `zl` would be undone before it ever reached the screen. Vim
    /// drags the cursor for the same reason, and the sideways mouse wheel needs
    /// the identical treatment — hence a window id rather than the active one.
    ///
    /// Scrolling stops once the cursor's line ends at the right edge of the
    /// view: past that there is only blank space, and a line that already fits
    /// entirely on screen does not scroll at all. That limit is measured against
    /// the cursor's own line rather than the buffer's longest, because finding
    /// the longest means walking the whole rope on every keystroke — and it is
    /// exactly the limit the renderer's clamp arrives at from the other
    /// direction, which is what keeps the two from fighting.
    pub fn scroll_columns(&mut self, wid: WindowId, delta: i32) {
        let Some((buffer, left, head, width)) = self
            .windows
            .window(wid)
            .map(|w| (w.buffer, w.scroll_left, w.cursors.head(), w.width))
        else {
            return;
        };
        let Some(doc) = self.buffers.get(buffer) else {
            return;
        };
        let buf = &doc.buffer;
        let head = head.min(buf.len_chars());
        let line = buf.char_to_line(head);
        let line_start = buf.line_start_char(line);
        let last_col = buf.line_content_len(line).saturating_sub(1);
        // Unrendered windows get a conventional width, the way
        // `viewport_height` invents rows — clamping against a width of zero
        // would drag the cursor to the left edge of a view nobody has drawn.
        let width = match width {
            0 => 80,
            w => w,
        };
        let max_left = (last_col + 1).saturating_sub(width);
        let left = left.saturating_add_signed(delta as isize).min(max_left);
        // `left + width - 1` is on the line by construction: `left` is capped so
        // that the line's last column falls no further right than the edge.
        let col = (head - line_start).clamp(left, left + width - 1);
        let Some(win) = self.windows.window_mut(wid) else {
            return;
        };
        win.scroll_left = left;
        win.cursors.set_head(line_start + col, buf);
    }

    /// Set the active document's indent to `n` spaces.
    pub fn set_active_indent_width(&mut self, n: u32) {
        self.active_doc_mut().set_indent_width(n);
    }

    /// Split the active window in `dir` (new window views the same buffer).
    pub fn split(&mut self, dir: SplitDir) {
        self.windows.split(dir);
    }

    /// Point the active window at `id`, clamping its cursor(s) to the new
    /// buffer's bounds so a stale position from a longer buffer can't index out
    /// of range (which would panic in `char_to_line` during render).
    pub fn set_active_buffer(&mut self, id: BufferId) {
        let max = match self.buffers.get(id) {
            Some(doc) => doc.buffer.len_chars(),
            None => return,
        };
        let win = self.windows.active_window_mut();
        win.buffer = id;
        win.cursors.clamp_to(max);
    }
}

impl EditorView for Workspace {
    fn buffer(&self) -> &Buffer {
        &self.active_doc().buffer
    }
    fn primary_head(&self) -> usize {
        self.active_window().cursors.head()
    }
    fn cursors(&self) -> &CursorSet {
        &self.active_window().cursors
    }
    fn viewport_height(&self) -> usize {
        match self.active_window().height {
            0 => 24, // not rendered yet
            h => h,
        }
    }
}

#[cfg(test)]
mod workspace_tests {
    use super::*;
    use crate::action::{Action, EditOp};
    use crate::windows::SplitDir;

    fn ws() -> Workspace {
        Workspace::from_file(PathBuf::from("/tmp/ruster_ws_ws.txt"), "hello".into())
    }

    #[test]
    fn edit_marks_active_doc_modified() {
        let mut w = ws();
        assert!(!w.active_doc().modified);
        w.execute(Action::Edit(EditOp::InsertChar('!')));
        assert!(w.active_doc().modified);
    }

    /// A workspace whose active window views a dired special buffer seeded with
    /// three lines of listing text.
    fn ws_special() -> (Workspace, BufferId) {
        let mut w = ws();
        let id = w.buffers.create_special(SpecialKind::Dired, "*dired*");
        w.buffers.get_mut(id).unwrap().buffer = Buffer::from_str("a\nb\nc");
        w.set_active_buffer(id);
        (w, id)
    }

    #[test]
    fn edits_are_noops_on_a_read_only_special_buffer() {
        let (mut w, _) = ws_special();
        for a in [
            Action::Edit(EditOp::InsertChar('X')),
            Action::Edit(EditOp::Backspace),
            Action::IndentLine,
            Action::DeindentLine,
            Action::Undo,
            Action::Redo,
        ] {
            w.execute(a);
        }
        assert_eq!(w.active_doc().buffer.to_string(), "a\nb\nc");
        assert!(!w.active_doc().modified);
    }

    #[test]
    fn navigation_still_works_on_a_read_only_special_buffer() {
        let (mut w, _) = ws_special();
        // Move is allowed even though edits are blocked.
        w.execute(Action::Move(crate::action::Motion::To(2)));
        assert_eq!(w.primary_head(), 2);
    }

    #[test]
    fn edits_still_mutate_a_normal_file_buffer() {
        // Regression: the read-only guard must not affect ordinary buffers.
        let mut w = ws();
        w.execute(Action::Move(crate::action::Motion::To(0)));
        w.execute(Action::Edit(EditOp::InsertChar('X')));
        assert_eq!(w.active_doc().buffer.to_string(), "Xhello");
        assert!(w.active_doc().modified);
    }

    #[test]
    fn split_shares_buffer_independent_cursor() {
        let mut w = ws();
        // Move cursor in the first window.
        w.execute(Action::Move(crate::action::Motion::To(0)));
        let buf_before = w.active_buffer();
        w.split(SplitDir::Vertical);
        // New window views the same buffer.
        assert_eq!(w.active_buffer(), buf_before);
        // Editing in one window changes the shared text.
        w.execute(Action::Edit(EditOp::InsertChar('X')));
        assert_eq!(w.active_doc().buffer.to_string(), "Xhello");
    }

    #[test]
    fn set_active_buffer_switches_view() {
        let mut w = ws();
        let scratch = w.buffers.create_scratch("scratch");
        w.set_active_buffer(scratch);
        assert_eq!(w.active_buffer(), scratch);
    }

    #[test]
    fn switching_to_shorter_buffer_clamps_cursor() {
        // Long buffer, cursor pushed near the end.
        let mut w = Workspace::from_file(
            PathBuf::from("long.txt"),
            "0123456789\nabcdefghij\nklmnopqrst".into(),
        );
        let end = w.buffer().len_chars();
        w.execute(Action::Move(crate::action::Motion::To(end)));
        assert!(w.active_window().cursors.head() > 2);

        // Switch to a much shorter buffer; the stale head must be clamped so
        // later char_to_line(head) does not index out of bounds and panic.
        let short = w.buffers.open_file(PathBuf::from("short.txt"), "hi".into());
        w.set_active_buffer(short);

        let head = w.active_window().cursors.head();
        let max = w.buffer().len_chars();
        assert!(
            head <= max,
            "cursor {head} must be clamped to buffer len {max}"
        );
        // Would panic before the fix.
        let _ = w.buffer().char_to_line(head);
    }

    /// A workspace on one long line, in a window `width` columns wide with the
    /// cursor parked at `col`.
    ///
    /// `scroll_left` starts where a frame would have left it — the renderer
    /// clamps it to keep the cursor visible — so these tests begin from a state
    /// the editor can actually be in.
    fn ws_wide(text: &str, width: usize, col: usize) -> Workspace {
        let mut w = Workspace::from_file(PathBuf::from("wide.txt"), text.into());
        w.execute(Action::Move(crate::action::Motion::To(col)));
        let win = w.windows.active_window_mut();
        win.width = width;
        win.scroll_left = (col + 1).saturating_sub(width.max(1));
        w
    }

    /// The active window's first visible column and cursor column.
    fn view(w: &Workspace) -> (usize, usize) {
        let win = w.active_window();
        let head = win.cursors.head();
        let line = w.buffer().char_to_line(head);
        (win.scroll_left, head - w.buffer().line_start_char(line))
    }

    #[test]
    fn scrolling_right_moves_the_view_and_pulls_the_cursor_to_the_left_edge() {
        let mut w = ws_wide(&"x".repeat(200), 10, 0);
        w.execute(Action::ScrollHorizontal(4));
        assert_eq!(view(&w), (4, 4), "the cursor came along to stay on screen");
    }

    /// The cursor is only dragged as far as it has to be: one still inside the
    /// scrolled view stays exactly where it was.
    #[test]
    fn scrolling_right_leaves_a_cursor_that_is_still_visible_alone() {
        let mut w = ws_wide(&"x".repeat(200), 10, 8);
        w.execute(Action::ScrollHorizontal(4));
        assert_eq!(view(&w), (4, 8), "columns 4..13 are shown; 8 is inside");
    }

    /// Scrolling back left pulls the cursor in off the right edge, the mirror of
    /// scrolling right pulling it off the left.
    #[test]
    fn scrolling_left_pulls_a_cursor_that_falls_off_the_right_edge() {
        let mut w = ws_wide(&"x".repeat(200), 10, 30);
        assert_eq!(view(&w), (21, 30), "the cursor starts on the right edge");
        w.execute(Action::ScrollHorizontal(-1));
        assert_eq!(view(&w), (20, 29), "it had to come back a column");
    }

    #[test]
    fn scrolling_left_at_the_first_column_stays_there() {
        let mut w = ws_wide(&"x".repeat(200), 10, 3);
        w.execute(Action::ScrollHorizontal(-5));
        assert_eq!(view(&w), (0, 3));
    }

    /// Scrolling stops with the end of the line at the right edge; going
    /// further would only pull blank space into view.
    #[test]
    fn scrolling_right_stops_with_the_end_of_the_line_at_the_edge() {
        let mut w = ws_wide("abcdefghij", 4, 0);
        w.execute(Action::ScrollHorizontal(100));
        assert_eq!(view(&w), (6, 6), "columns 6..9 — 'ghij' fills the view");
    }

    /// A line that already fits does not scroll at all: there is nothing to its
    /// right to bring into view, and hiding its start would be pure loss.
    #[test]
    fn a_line_that_fits_in_the_window_does_not_scroll() {
        let mut w = ws_wide("short", 40, 0);
        w.execute(Action::ScrollHorizontal(3));
        assert_eq!(view(&w), (0, 0));
    }

    /// An empty line has no column to scroll to; without the clamp the cursor
    /// would be pushed past the newline into the next line.
    #[test]
    fn scrolling_right_on_an_empty_line_does_nothing() {
        let mut w = Workspace::from_file(PathBuf::from("empty.txt"), "\nsecond\n".into());
        w.execute(Action::Move(crate::action::Motion::To(0)));
        w.windows.active_window_mut().width = 10;
        w.execute(Action::ScrollHorizontal(5));
        assert_eq!(view(&w), (0, 0));
        assert_eq!(w.active_window().cursors.head(), 0, "still on line one");
    }

    /// Columns are characters. Scrolling two columns into a line of three-byte
    /// characters must land on the third character, not two bytes in.
    #[test]
    fn columns_are_characters_not_bytes() {
        let mut w = ws_wide("日本語ですね", 3, 0);
        w.execute(Action::ScrollHorizontal(2));
        assert_eq!(view(&w), (2, 2));
        let head = w.active_window().cursors.head();
        assert_eq!(w.buffer().char_at(head), '語');
    }

    /// A window nobody has rendered has no width recorded. Clamping the cursor
    /// against a width of zero would drag it to the left edge of a view that
    /// does not exist; the conventional fallback keeps it where it is.
    #[test]
    fn scrolling_an_unrendered_window_does_not_yank_the_cursor_back() {
        let mut w = Workspace::from_file(PathBuf::from("wide.txt"), "x".repeat(200));
        w.execute(Action::Move(crate::action::Motion::To(40)));
        assert_eq!(w.active_window().width, 0, "never rendered");
        w.execute(Action::ScrollHorizontal(1));
        assert_eq!(view(&w), (1, 40));
    }

    /// Every window scrolls on its own, which is what the window id is for —
    /// the sideways wheel scrolls whatever is under the pointer.
    #[test]
    fn scroll_columns_moves_the_named_window_not_the_active_one() {
        let mut w = ws_wide(&"x".repeat(200), 10, 0);
        let first = w.windows.active();
        let second = w.windows.split(SplitDir::Vertical);
        w.scroll_columns(first, 6);
        assert_eq!(w.windows.window(first).unwrap().scroll_left, 6);
        assert_eq!(w.windows.window(second).unwrap().scroll_left, 0);
        assert_eq!(w.windows.active(), second, "focus did not move either");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_two_distinct_files_yields_two_ids() {
        let mut s = BufferStore::new();
        let a = s.open_file(PathBuf::from("/tmp/ruster_ws_a.txt"), "a".into());
        let b = s.open_file(PathBuf::from("/tmp/ruster_ws_b.txt"), "b".into());
        assert_ne!(a, b);
        assert_eq!(s.len(), 2);
        assert_eq!(s.ids(), &[a, b]);
    }

    #[test]
    fn reopening_same_path_returns_same_id() {
        let mut s = BufferStore::new();
        let a = s.open_file(PathBuf::from("/tmp/ruster_ws_a.txt"), "a".into());
        let again = s.open_file(PathBuf::from("/tmp/ruster_ws_a.txt"), "ignored".into());
        assert_eq!(a, again);
        assert_eq!(s.len(), 1);
        // Content of the second open is ignored — first wins.
        assert_eq!(s.get(a).unwrap().buffer.to_string(), "a");
    }

    #[test]
    fn scratch_and_special_have_expected_kinds() {
        let mut s = BufferStore::new();
        let sc = s.create_scratch("[No Name]");
        let sp = s.create_special(SpecialKind::Ibuffer, "*ibuffer*");
        assert_eq!(s.get(sc).unwrap().kind, DocKind::Scratch);
        assert_eq!(
            s.get(sp).unwrap().kind,
            DocKind::Special(SpecialKind::Ibuffer)
        );
    }

    #[test]
    fn close_nonexistent_is_false() {
        let mut s = BufferStore::new();
        assert!(!s.close(BufferId(999)));
    }

    #[test]
    fn close_removes_from_order() {
        let mut s = BufferStore::new();
        let a = s.create_scratch("a");
        let b = s.create_scratch("b");
        assert!(s.close(a));
        assert_eq!(s.ids(), &[b]);
        assert!(s.get(a).is_none());
    }

    #[test]
    fn refuses_to_close_last_modified_buffer() {
        let mut s = BufferStore::new();
        let a = s.create_scratch("a");
        s.get_mut(a).unwrap().modified = true;
        assert!(!s.close(a));
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn closes_last_buffer_when_clean() {
        let mut s = BufferStore::new();
        let a = s.create_scratch("a");
        assert!(s.close(a));
        assert!(s.is_empty());
    }

    #[test]
    fn modified_flag_round_trips() {
        let mut s = BufferStore::new();
        let a = s.create_scratch("a");
        assert!(!s.get(a).unwrap().modified);
        s.get_mut(a).unwrap().modified = true;
        assert!(s.get(a).unwrap().modified);
    }

    #[test]
    fn refuses_to_close_pinned_document() {
        let mut s = BufferStore::new();
        let a = s.create_scratch("pinned_test");
        s.get_mut(a).unwrap().pinned = true;
        assert!(!s.close(a));
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn unpinned_document_can_still_be_closed() {
        let mut s = BufferStore::new();
        let a = s.create_scratch("a");
        let b = s.create_scratch("b");
        s.get_mut(a).unwrap().pinned = true;
        assert!(!s.close(a));
        assert!(s.close(b));
        assert_eq!(s.len(), 1);
    }
}
