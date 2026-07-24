use crate::action::{Action, EditOp, Motion};
use crate::buffer::Buffer;
use crate::cursor::{CursorSet, Range};
use crate::undo::UndoStack;

/// Read-only view of editable state that key handling (vim/emacs) reads.
///
/// Decouples the input layer from *where* the buffer and cursor live: the
/// owned [`Editor`] provides it directly, while multi-window code provides it
/// over the active window's document + cursor set.
pub trait EditorView {
    fn buffer(&self) -> &Buffer;
    /// Visible text rows, for half-page motions. Defaults to a conventional
    /// terminal height for headless callers that have no viewport.
    fn viewport_height(&self) -> usize {
        24
    }
    fn primary_head(&self) -> usize;
    fn cursors(&self) -> &CursorSet;
    fn char_to_line(&self, char_idx: usize) -> usize {
        self.buffer().char_to_line(char_idx)
    }
}

impl EditorView for Editor {
    fn buffer(&self) -> &Buffer { &self.buffer }
    fn primary_head(&self) -> usize { self.cursors.head() }
    fn cursors(&self) -> &CursorSet { &self.cursors }
}

/// A transient editing session over borrowed state.
///
/// This is the single place editing [`Action`]s are interpreted. It borrows the
/// document's [`Buffer`]/[`UndoStack`]/indent together with a [`CursorSet`] —
/// which, once windows exist, lives on the active window rather than the
/// document. The owned [`Editor`] below is a thin wrapper that constructs an
/// `EditSession` over its own fields.
pub struct EditSession<'a> {
    pub buffer: &'a mut Buffer,
    pub cursors: &'a mut CursorSet,
    pub undo: &'a mut UndoStack,
    pub indent: &'a str,
}

impl<'a> EditSession<'a> {
    pub fn new(
        buffer: &'a mut Buffer,
        cursors: &'a mut CursorSet,
        undo: &'a mut UndoStack,
        indent: &'a str,
    ) -> Self {
        EditSession { buffer, cursors, undo, indent }
    }

    fn cursor_line(&self) -> usize {
        self.buffer.char_to_line(self.cursors.head())
    }

    pub fn execute(&mut self, action: Action) {
        match action {
            Action::BeginBatch => self.undo.begin_batch(),
            Action::EndBatch => self.undo.end_batch(),
            Action::Undo => {
                if let Some((_n, at)) = self.undo.undo(self.buffer) {
                    self.cursors.set_head(at, self.buffer);
                }
            }
            Action::Redo => {
                if let Some((_n, at)) = self.undo.redo(self.buffer) {
                    self.cursors.set_head(at, self.buffer);
                }
            }
            Action::UndoTime(forward) => {
                if let Some((_n, at)) = self.undo.undo_time(self.buffer, forward) {
                    self.cursors.set_head(at, self.buffer);
                }
            }
            Action::BeginVisual(anchor) => {
                self.cursors.set_visual_anchor(anchor);
            }
            Action::Move(m) => self.apply_motion(m),
            Action::Edit(e) => self.apply_edit(e),
            Action::AddCursor(pos) => self.cursors.add_cursor(pos),
            Action::ClearExtraCursors => self.cursors.clear_extra(),
            // Scrolling is window state, which an EditSession does not borrow;
            // Workspace handles it before delegating here.
            Action::Scroll(_) => {}
            Action::CmdlineResult(_) => {}
            Action::Textobject { .. } => {}
            Action::IndentLine => {
                let line = self.cursor_line();
                let start = self.buffer.line_start_char(line);
                let ch = self.buffer.insert(start, self.indent);
                self.undo.push(ch);
            }
            Action::DeindentLine => {
                let line = self.cursor_line();
                let start = self.buffer.line_start_char(line);
                let content = self.buffer.line_to_string(line);
                let to_remove = content.chars().take_while(|c| *c == ' ').take(self.indent.len()).count();
                if to_remove > 0 {
                    let ch = self.buffer.delete(start..start + to_remove);
                    self.undo.push(ch);
                    self.cursors.set_head(start, self.buffer);
                }
            }
        }
    }

    fn apply_motion(&mut self, m: Motion) {
        match m {
            Motion::Grapheme(d) => self.cursors.move_grapheme(self.buffer, d),
            Motion::Line(d) => self.cursors.move_line(self.buffer, d),
            Motion::LineEdge(edge) => self.cursors.move_line_edge(self.buffer, edge),
            Motion::To(target) => self.cursors.set_head(target, self.buffer),
        }
    }

    fn apply_edit(&mut self, e: EditOp) {
        // Single cursor: apply directly and move the caret via `set_head`, which
        // also refreshes `desired_col` for later vertical motion.
        if self.cursors.count() == 1 {
            let at = self.cursors.head();
            if let Some(new_head) = self.apply_edit_at(at, &e) {
                self.cursors.set_head(new_head, self.buffer);
            }
            return;
        }
        self.apply_edit_multi(e);
    }

    /// Apply one edit at `at`, pushing the change onto the undo batch. Returns
    /// where the cursor that owned this edit should land, or `None` for a no-op.
    fn apply_edit_at(&mut self, at: usize, e: &EditOp) -> Option<usize> {
        match e {
            EditOp::InsertChar(c) => {
                let mut buf = [0u8; 4];
                let text = c.encode_utf8(&mut buf);
                self.undo.push(self.buffer.insert(at, text));
                Some(at + 1)
            }
            EditOp::InsertString(s) => {
                let n = s.chars().count();
                self.undo.push(self.buffer.insert(at, s));
                Some(at + n)
            }
            EditOp::DeleteRange(start, end) if end > start => {
                let safe_end = (*end).min(self.buffer.len_chars());
                self.undo.push(self.buffer.delete(*start..safe_end));
                Some(*start)
            }
            EditOp::DeleteRange(_, _) => None,
            EditOp::Backspace => {
                if at > 0 {
                    self.undo.push(self.buffer.delete(at - 1..at));
                    Some(at - 1)
                } else {
                    None
                }
            }
        }
    }

    /// Apply the same edit at every cursor. Cursors are processed low offset to
    /// high, and each later position is shifted by the net length change of all
    /// earlier edits — so every cursor lands where its own text ended up and no
    /// edit disturbs a position that hasn't been applied yet. `DeleteRange` is
    /// reinterpreted as "delete this many chars at each cursor" (its absolute
    /// bounds mean nothing once there are several carets).
    fn apply_edit_multi(&mut self, e: EditOp) {
        let mut order: Vec<usize> = (0..self.cursors.count()).collect();
        order.sort_by_key(|&i| self.cursors.cursors[i].head);

        let mut shift: isize = 0;
        for i in order {
            let base = self.cursors.cursors[i].head as isize + shift;
            let at = base.max(0) as usize;
            let before = self.buffer.len_chars();
            let new_head = match &e {
                EditOp::DeleteRange(start, end) if end > start => {
                    // Delete `end - start` chars starting at this cursor.
                    let del_end = (at + (end - start)).min(self.buffer.len_chars());
                    if del_end > at {
                        self.undo.push(self.buffer.delete(at..del_end));
                    }
                    Some(at)
                }
                _ => self.apply_edit_at(at, &e),
            };
            // Track how the buffer length changed so later cursors stay aligned.
            shift += self.buffer.len_chars() as isize - before as isize;
            self.cursors.cursors[i] = Range::caret(new_head.unwrap_or(at));
        }
        self.cursors.merge_overlaps();
    }
}

/// Owned single-buffer editing context.
///
/// Retained for tests and the single-window code path; delegates all editing to
/// [`EditSession`]. Multi-window code constructs an `EditSession` directly over
/// a [`crate::document::Document`] and the active [`crate::windows::Window`].
pub struct Editor {
    buffer: Buffer,
    cursors: CursorSet,
    undo: UndoStack,
    indent: String,
}

impl Editor {
    pub fn from_str(s: &str) -> Self {
        let len = s.chars().count();
        Editor {
            buffer: Buffer::from_str(s),
            cursors: CursorSet::single(len),
            undo: UndoStack::new(),
            indent: "    ".to_string(),
        }
    }

    pub fn buffer(&self) -> &Buffer { &self.buffer }
    pub fn buffer_mut(&mut self) -> &mut Buffer { &mut self.buffer }
    pub fn cursors(&self) -> &CursorSet { &self.cursors }
    pub fn primary_head(&self) -> usize { self.cursors.head() }

    pub fn char_to_line(&self, char_idx: usize) -> usize { self.buffer.char_to_line(char_idx) }

    pub fn set_config_indent(&mut self, tabstop: u32) {
        self.indent = " ".repeat(tabstop as usize);
    }

    pub fn execute(&mut self, action: Action) {
        EditSession::new(&mut self.buffer, &mut self.cursors, &mut self.undo, &self.indent)
            .execute(action);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::Edge;

    #[test]
    fn insert_char_then_backspace_roundtrips_via_undo() {
        let mut e = Editor::from_str("ab");
        e.execute(Action::BeginBatch);
        e.execute(Action::Edit(EditOp::InsertChar('!')));
        e.execute(Action::EndBatch);
        assert_eq!(e.buffer().to_string(), "ab!");
        assert_eq!(e.primary_head(), 3);
        e.execute(Action::Edit(EditOp::Backspace));
        assert_eq!(e.buffer().to_string(), "ab");
        assert_eq!(e.primary_head(), 2);
        e.execute(Action::Undo);
        assert_eq!(e.buffer().to_string(), "ab!");
        e.execute(Action::Undo);
        assert_eq!(e.buffer().to_string(), "ab");
    }

    #[test]
    fn multi_cursor_insert_lands_at_every_cursor_without_drift() {
        // Two cursors on the two 'x's. Typing "ab" at both must produce the
        // same text at each — not the drifting corruption a stale-offset apply
        // gives ("xax b" etc.).
        let mut e = Editor::from_str("x.x");
        e.execute(Action::Move(Motion::To(0))); // primary at 0
        e.execute(Action::AddCursor(2)); // second cursor at the other 'x'
        e.execute(Action::BeginBatch);
        e.execute(Action::Edit(EditOp::InsertChar('a')));
        e.execute(Action::Edit(EditOp::InsertChar('b')));
        e.execute(Action::EndBatch);
        assert_eq!(e.buffer().to_string(), "abx.abx");
    }

    #[test]
    fn multi_cursor_backspace_deletes_at_every_cursor() {
        let mut e = Editor::from_str("ax.bx");
        e.execute(Action::Move(Motion::To(1))); // after 'a'
        e.execute(Action::AddCursor(4)); // after 'b'
        e.execute(Action::BeginBatch);
        e.execute(Action::Edit(EditOp::Backspace));
        e.execute(Action::EndBatch);
        assert_eq!(e.buffer().to_string(), "x.x");
    }

    #[test]
    fn multi_cursor_insert_undoes_as_one_step() {
        let mut e = Editor::from_str("x.x");
        e.execute(Action::Move(Motion::To(0)));
        e.execute(Action::AddCursor(2));
        e.execute(Action::BeginBatch);
        e.execute(Action::Edit(EditOp::InsertChar('a')));
        e.execute(Action::EndBatch);
        assert_eq!(e.buffer().to_string(), "ax.ax");
        e.execute(Action::Undo);
        assert_eq!(e.buffer().to_string(), "x.x");
    }

    #[test]
    fn move_then_delete_range() {
        let mut e = Editor::from_str("hello");
        e.execute(Action::Move(Motion::Grapheme(1)));
        e.execute(Action::Move(Motion::Grapheme(1)));
        e.execute(Action::Edit(EditOp::DeleteRange(2, 4)));
        assert_eq!(e.buffer().to_string(), "heo");
        assert_eq!(e.primary_head(), 2);
    }

    #[test]
    fn line_edge_end_motion() {
        let mut e = Editor::from_str("abc");
        e.execute(Action::Move(Motion::LineEdge(Edge::End)));
        assert_eq!(e.primary_head(), 3);
    }

    #[test]
    fn begin_visual_extends_selection_anchor_preserved_on_motion() {
        let mut e = Editor::from_str("hello");
        // cursor starts at end (5); move left twice to 3
        e.execute(Action::Move(Motion::Grapheme(-1)));
        e.execute(Action::Move(Motion::Grapheme(-1)));
        let head_before = e.primary_head();
        e.execute(Action::BeginVisual(head_before));
        // extend right by 1
        e.execute(Action::Move(Motion::Grapheme(1)));
        e.execute(Action::BeginVisual(head_before));
        let r = e.cursors().primary();
        assert_eq!(r.anchor, head_before);
        assert_eq!(r.head, head_before + 1);
    }

    #[test]
    fn indent_adds_spaces() {
        let mut e = Editor::from_str("hello");
        e.execute(Action::IndentLine);
        assert_eq!(e.buffer().to_string(), "    hello");
    }

    #[test]
    fn deindent_removes_spaces() {
        let mut e = Editor::from_str("    hello");
        e.execute(Action::DeindentLine);
        assert_eq!(e.buffer().to_string(), "hello");
    }

    #[test]
    fn deindent_empty_line_does_nothing() {
        let mut e = Editor::from_str("");
        e.execute(Action::DeindentLine);
        assert_eq!(e.buffer().to_string(), "");
    }
}