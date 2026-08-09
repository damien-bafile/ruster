//! Editor panes: tree leaves that are not Wayland clients.
//!
//! A pane takes an ordinary [`WindowId`] from the same allocator a client does
//! and goes into the same container tree as an ordinary leaf. The tree never
//! asks what a leaf *is* — it needs `Copy + PartialEq` and nothing else — so a
//! pane already lays out, already gets a focus border, already hit-tests, and is
//! already skipped by every path that looks a leaf up in `toplevels` and
//! `continue`s when it is absent.
//!
//! That is why there is no `Node::Leaf(Client | Buffer)` enum. The alternative
//! was to teach the tree the difference, which would have meant touching
//! `geometry`, `tile_under`, focus, the renderer and 94 tests in `ruster-shell`
//! — and, worse, would have re-opened the question of whether `tile_under` and
//! `geometry` agree at every one of those sites. They are the same list today,
//! and a side table keeps them that way by construction. The compositor has had
//! the pointer land somewhere other than where it looked exactly once, and it
//! was because two functions computed one rectangle two ways.
//!
//! Stage 1 gave a pane its place in the tree; Stage 2 put a buffer in it; Stage
//! 3 makes it editable, driving the same `VimState` and `EditSession` the editor
//! does rather than a second editing implementation living in the compositor.
//!
//! Stage 4 moved the text out. A pane names a [`Document`] in the compositor's
//! [`BufferStore`](ruster_core::workspace::BufferStore) instead of owning a
//! `Buffer`, which is what makes two panes on one file possible — and what makes
//! `:w` possible at all, since a `Buffer` does not know where it came from and a
//! `Document` does. Cursor and scroll deliberately stayed here: they are what
//! differs between two panes showing the same document, which is the same
//! division `ruster_core::windows::Window` makes for the editor's own windows.
//!
//! The consequence is that every method that reads text takes the buffer as an
//! argument. That is the point rather than a cost — a pane cannot read a
//! document it is not pointed at, and the pane and the renderer cannot end up
//! looking at two different copies of the same file.

use crate::chrome::FrameBody;
use ruster_core::buffer::Buffer;
use ruster_core::cursor::CursorSet;
use ruster_core::document::{BufferId, Document};
use ruster_core::editor::{EditSession, EditorView};
use ruster_core::key::KeyEvent;
use ruster_core::vim::VimState;
use ruster_shell::WindowId;

/// One editor pane: a view onto a [`Document`], with its own cursor and scroll.
///
/// Not `Clone` or `PartialEq`: a pane is identified by the `WindowId` that keys
/// it rather than by its contents.
pub struct EditorPane {
    /// The document this pane is showing, in the compositor's `BufferStore`.
    ///
    /// A handle rather than the text, so `:b` is a field assignment and two
    /// panes on one file are two panes holding the same id — with the cursor
    /// and scroll below still their own.
    pub doc: BufferId,
    /// The character grid the pane's rectangle works out to, from
    /// [`cell_metrics`](ruster_render_gles::atlas::cell_metrics).
    ///
    /// Stored rather than derived at draw time because it decides how much of a
    /// buffer is visible, and a scroll clamp that disagreed with what was drawn
    /// would put the cursor off screen.
    pub cols: u16,
    pub rows: u16,
    /// First visible buffer line.
    pub scroll_top: usize,
    /// Where the carets are. A `CursorSet` rather than one position because
    /// that is what `EditSession` edits through, and multi-cursor comes free
    /// with it rather than as a later retrofit.
    pub cursors: CursorSet,
    /// Modal state. The compositor drives the same `VimState` the editor does,
    /// so a motion cannot behave differently here than it does in the TUI.
    pub vim: VimState,
}

/// A pane and the text it is pointed at, which is what `VimState` needs to
/// answer a motion.
///
/// The pair exists because the two halves live in different places now: the
/// cursor on the pane, the buffer in the store. `ruster_core::workspace::
/// Workspace` implements the same trait over the same split — windows for the
/// cursor, buffers for the text — and this is that shape with the compositor's
/// tree in place of the editor's.
struct PaneView<'a> {
    pane: &'a EditorPane,
    buffer: &'a Buffer,
}

impl EditorView for PaneView<'_> {
    fn buffer(&self) -> &Buffer {
        self.buffer
    }

    fn primary_head(&self) -> usize {
        self.pane.cursors.primary().head
    }

    fn cursors(&self) -> &CursorSet {
        &self.pane.cursors
    }

    /// The pane's own row count, so half-page motions move by what is on
    /// screen rather than by the trait's headless default of 24.
    fn viewport_height(&self) -> usize {
        self.pane.rows.max(1) as usize
    }
}

impl EditorPane {
    /// A pane showing `doc`, from the top.
    pub fn new(doc: BufferId) -> Self {
        EditorPane {
            doc,
            cols: 0,
            rows: 0,
            scroll_top: 0,
            cursors: CursorSet::single(0),
            vim: VimState::default(),
        }
    }

    /// Feed one key to the pane's editor.
    ///
    /// The `VimState` is taken out for the duration because `handle` wants an
    /// `&dyn EditorView` over the pane — an immutable borrow — while the state
    /// doing the handling lives on the pane itself. Putting it back is not
    /// optional and there is no early return between the two.
    ///
    /// `modified` is set from the buffer's revision rather than from which
    /// actions were dispatched: the classification of "does this action change
    /// text" already exists in `ruster_core`, and a second copy of it here would
    /// be the one that fell behind.
    pub fn handle_key(&mut self, key: KeyEvent, doc: &mut Document) {
        let mut vim = std::mem::take(&mut self.vim);
        let actions = vim.handle(
            key,
            &PaneView {
                pane: self,
                buffer: &doc.buffer,
            },
        );
        self.vim = vim;
        let revision = doc.buffer.revision();
        {
            let mut session = EditSession::new(
                &mut doc.buffer,
                &mut self.cursors,
                &mut doc.undo,
                &doc.indent,
            );
            for action in actions {
                session.execute(action);
            }
        }
        if doc.buffer.revision() != revision {
            doc.modified = true;
        }
        self.follow_cursor(&doc.buffer);
    }

    /// Point this pane at `doc`, keeping the cursor somewhere that document has.
    ///
    /// The clamp is not defensive: a cursor left over from a longer buffer would
    /// index out of range in `char_to_line` the next time the pane drew, taking
    /// the display server down with it. Scrolling to the cursor afterwards is
    /// what stops a switch landing on a screenful of nothing.
    pub fn show(&mut self, id: BufferId, buffer: &Buffer) {
        self.doc = id;
        self.cursors.clear_extra();
        self.cursors.clamp_to(buffer.len_chars());
        self.follow_cursor(buffer);
    }

    /// Scroll just enough to keep the primary cursor on screen.
    ///
    /// Without this the cursor walks off the top or bottom and the pane looks
    /// frozen — the buffer is changing, just not where anyone is looking.
    pub fn follow_cursor(&mut self, buffer: &Buffer) {
        let rows = self.rows as usize;
        if rows == 0 {
            return;
        }
        let line = buffer.char_to_line(self.cursors.primary().head);
        if line < self.scroll_top {
            self.scroll_top = line;
        } else if line >= self.scroll_top + rows {
            self.scroll_top = line + 1 - rows;
        }
    }

    /// The worst diagnostic severity on each visible line, if any.
    ///
    /// Severity is what decides the sign and its colour, and *worst* rather than
    /// first: a line with a warning and an error is an error line, and showing
    /// whichever the server happened to send first would make the sign depend on
    /// message order.
    pub fn line_severities(
        &self,
        diagnostics: &[ruster_lsp::Diagnostic],
        first_line: usize,
        shown: usize,
    ) -> Vec<Option<u8>> {
        let mut out: Vec<Option<u8>> = vec![None; shown];
        for diag in diagnostics {
            let line = diag.start.line as usize;
            if line < first_line || line >= first_line + shown {
                continue;
            }
            let slot = &mut out[line - first_line];
            // Lower is worse in LSP: 1 is an error, 4 a hint.
            *slot = Some(match *slot {
                Some(existing) => existing.min(diag.severity),
                None => diag.severity,
            });
        }
        out
    }

    /// The buffer lines currently on screen, and the number of the first.
    ///
    /// Clamped to the buffer rather than to the last scroll position, so a pane
    /// that has been scrolled to the bottom and then grown shows the extra rows
    /// instead of leaving a gap it still believes is scrolled past.
    pub fn visible_lines(&self, buffer: &Buffer) -> (usize, Vec<String>) {
        let total = buffer.line_count();
        let first = self.scroll_top.min(total.saturating_sub(1));
        let last = (first + self.rows as usize).min(total);
        let lines = (first..last)
            .map(|i| {
                // Without the terminator. It draws as nothing, so the screen
                // would look right either way — but every line would measure
                // one character too long, and Stage 3 puts the cursor and
                // click-to-position on exactly that measurement.
                let line = buffer.line_to_string(i);
                line.trim_end_matches(['\n', '\r']).to_string()
            })
            .collect();
        (first, lines)
    }

    /// Scroll by `delta` lines, stopping at the ends.
    ///
    /// The last line stays reachable rather than the last *screenful*: a buffer
    /// shorter than the pane must not scroll at all, and one longer should stop
    /// with its final line visible rather than scrolling into blank space.
    pub fn scroll_by(&mut self, buffer: &Buffer, delta: i64) {
        let total = buffer.line_count();
        let max = total.saturating_sub(1);
        let next = self.scroll_top as i64 + delta;
        self.scroll_top = next.clamp(0, max as i64) as usize;
    }

    /// The buffer offset a point inside the pane's frame names, in physical
    /// pixels from the frame's top-left.
    ///
    /// The grid comes from [`visible_lines`](Self::visible_lines) and the frame
    /// layout from [`FrameBody`], which are the two things the renderer draws
    /// from — so the character under the pointer is the character on screen,
    /// including after a scroll and including when the gutter has just grown a
    /// digit and shifted every column right.
    pub fn offset_at(&self, buffer: &Buffer, x: f32, y: f32) -> usize {
        let (first, lines) = self.visible_lines(buffer);
        let (row, col) = FrameBody::new(first, lines.len()).cell_at(x, y);
        // Below the last line is the last line: a pane is usually taller than
        // the text in it, and a click in that space that did nothing would feel
        // like a dead region of the window.
        let row = row.min(lines.len().saturating_sub(1));
        let Some(line) = lines.get(row) else {
            // No rows at all — an unlaid-out pane. There is no position to name
            // but the start of the buffer.
            return 0;
        };
        // `visible_lines` strips the terminator, so the end of a line is the
        // last position on *this* line. Without the clamp, clicking the empty
        // space to the right of a short line would run into the line below it.
        let col = col.min(line.chars().count());
        buffer.line_start_char(first + row) + col
    }

    /// Put the caret where a click at `(x, y)` — physical pixels from the
    /// pane's top-left — landed.
    ///
    /// Extra cursors go: a click is how a multi-cursor session is ended in
    /// every editor that has one, and leaving them would type in places the
    /// user has just pointed away from.
    pub fn click_at(&mut self, buffer: &Buffer, x: f32, y: f32) {
        let at = self.offset_at(buffer, x, y);
        self.cursors.clear_extra();
        self.cursors.set_head(at, buffer);
    }

    /// The grid a rectangle of `width`x`height` logical pixels works out to.
    ///
    /// Saturating rather than panicking on a zero or negative rectangle: a tile
    /// can legitimately be measured before the output is configured, and a pane
    /// that took the session down for being briefly 0x0 would be a poor trade
    /// for a grid nobody was looking at yet.
    pub fn grid_for(width: i32, height: i32, cell_w: f32, cell_h: f32) -> (u16, u16) {
        if cell_w <= 0.0 || cell_h <= 0.0 {
            return (0, 0);
        }
        let cols = (width.max(0) as f32 / cell_w).floor().max(0.0);
        let rows = (height.max(0) as f32 / cell_h).floor().max(0.0);
        (
            cols.min(u16::MAX as f32) as u16,
            rows.min(u16::MAX as f32) as u16,
        )
    }
}

/// The panes a compositor holds, keyed the same way `toplevels` is.
///
/// A leaf is a client if `toplevels` has it and a pane if this does. Exactly one
/// of the two, which [`Panes::debug_assert_disjoint`] states.
pub type Panes = std::collections::HashMap<WindowId, EditorPane>;

/// Panics in debug builds if any id is both a client and a pane.
///
/// The invariant the whole side-table design rests on. Both maps are keyed from
/// the same id allocator, so a collision means someone inserted a pane over a
/// live window — which would show up as a window that renders as an empty frame
/// rather than as anything obviously wrong.
pub fn debug_assert_disjoint<T>(panes: &Panes, toplevels: &std::collections::HashMap<WindowId, T>) {
    debug_assert!(
        !panes.keys().any(|id| toplevels.contains_key(id)),
        "a leaf is a client or a pane, never both"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A pane over a document holding `text`, laid out `rows` tall.
    ///
    /// Returned as a pair because that is how the compositor holds them: the
    /// document in the store, the pane in the side table beside the tree.
    fn pane_over(text: &str, rows: u16) -> (EditorPane, Document) {
        let mut pane = EditorPane::new(BufferId(1));
        pane.rows = rows;
        (pane, Document::from_file(PathBuf::from("f.rs"), text))
    }

    /// `n` lines numbered 1..=n.
    fn numbered(n: usize) -> String {
        (1..=n)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn typing_in_insert_mode_changes_the_buffer() {
        // The whole point of the stage: keys reach `VimState` and `EditSession`
        // rather than a private editing implementation living in the
        // compositor.
        let (mut pane, mut doc) = pane_over("abc", 10);
        pane.handle_key(KeyEvent::Char('i'), &mut doc);
        for c in "hi ".chars() {
            pane.handle_key(KeyEvent::Char(c), &mut doc);
        }
        assert_eq!(pane.visible_lines(&doc.buffer).1[0], "hi abc");
    }

    #[test]
    fn normal_mode_motions_move_without_editing() {
        let (mut pane, mut doc) = pane_over("abc\ndef", 10);
        for _ in 0..2 {
            pane.handle_key(KeyEvent::Char('l'), &mut doc);
        }
        pane.handle_key(KeyEvent::Char('j'), &mut doc);
        assert_eq!(
            pane.visible_lines(&doc.buffer).1,
            vec!["abc", "def"],
            "text unchanged"
        );
        assert_eq!(
            doc.buffer.char_to_line(pane.cursors.primary().head),
            1,
            "the cursor moved down a line"
        );
    }

    #[test]
    fn undo_puts_back_what_was_typed() {
        // The undo stack is the document's, so it is the editor's undo rather
        // than something re-implemented here — and it stays with the file when
        // the pane is pointed somewhere else and back.
        let (mut pane, mut doc) = pane_over("abc", 10);
        pane.handle_key(KeyEvent::Char('i'), &mut doc);
        pane.handle_key(KeyEvent::Char('X'), &mut doc);
        assert_eq!(pane.visible_lines(&doc.buffer).1[0], "Xabc");
        pane.handle_key(KeyEvent::Esc, &mut doc);
        pane.handle_key(KeyEvent::Char('u'), &mut doc);
        assert_eq!(pane.visible_lines(&doc.buffer).1[0], "abc");
    }

    #[test]
    fn two_panes_on_one_document_scroll_and_point_independently() {
        // The reason cursor and scroll stayed on the pane. Sharing them would
        // make a second view of a file a mirror of the first, which is the one
        // thing a split is for.
        let (mut top, mut doc) = pane_over(&numbered(50), 5);
        let mut bottom = EditorPane::new(top.doc);
        bottom.rows = 5;

        bottom.scroll_by(&doc.buffer, 20);
        for _ in 0..3 {
            top.handle_key(KeyEvent::Char('j'), &mut doc);
        }

        assert_eq!(top.scroll_top, 0, "the top pane has not scrolled");
        assert_eq!(bottom.scroll_top, 20);
        assert_eq!(bottom.visible_lines(&doc.buffer).1[0], "21");
        assert_eq!(
            doc.buffer.char_to_line(top.cursors.primary().head),
            3,
            "only the pane that got the keys moved its cursor"
        );
    }

    #[test]
    fn editing_through_one_pane_is_seen_by_the_other() {
        // One `Document`, not a copy each: two panes on a file that disagreed
        // about its contents would each save over the other.
        let (mut left, mut doc) = pane_over("abc", 10);
        let mut right = EditorPane::new(left.doc);
        // Its own viewport: the grid is per pane, and a pane with no rows shows
        // nothing however much text the document has.
        right.rows = 10;
        left.handle_key(KeyEvent::Char('i'), &mut doc);
        left.handle_key(KeyEvent::Char('X'), &mut doc);
        assert_eq!(right.visible_lines(&doc.buffer).1[0], "Xabc");
    }

    #[test]
    fn typing_marks_the_document_modified_and_a_motion_does_not() {
        // What `:w` reports on. Derived from the buffer's revision rather than
        // from a second list of which actions edit — which is the only reason a
        // motion is not caught by it.
        let (mut pane, mut doc) = pane_over("abc\ndef", 10);
        pane.handle_key(KeyEvent::Char('j'), &mut doc);
        assert!(!doc.modified, "moving the cursor changed no text");
        pane.handle_key(KeyEvent::Char('i'), &mut doc);
        assert!(!doc.modified, "entering insert mode is not an edit either");
        pane.handle_key(KeyEvent::Char('X'), &mut doc);
        assert!(doc.modified, "typing a character is");
    }

    #[test]
    fn showing_a_shorter_document_clamps_a_cursor_that_would_be_out_of_range() {
        // Not defensive: `char_to_line` past the end panics, and this one runs
        // in the process that owns the screen.
        let (mut pane, long) = pane_over(&numbered(50), 5);
        pane.cursors = CursorSet::single(long.buffer.len_chars());
        let short = Document::from_file(PathBuf::from("short.rs"), "hi");

        pane.show(BufferId(2), &short.buffer);

        assert_eq!(pane.doc, BufferId(2));
        assert!(
            pane.cursors.primary().head <= short.buffer.len_chars(),
            "a cursor from the longer document survived the switch"
        );
        let _ = short.buffer.char_to_line(pane.cursors.primary().head);
    }

    #[test]
    fn showing_another_document_scrolls_to_where_the_cursor_ended_up() {
        // A pane scrolled deep into a long file and then pointed at a short one
        // would otherwise show blank space, which looks like an empty file.
        let (mut pane, _long) = pane_over(&numbered(200), 5);
        pane.scroll_top = 150;
        let short = Document::from_file(PathBuf::from("short.rs"), "a\nb\nc");
        pane.show(BufferId(2), &short.buffer);
        assert_eq!(pane.visible_lines(&short.buffer).1, vec!["a", "b", "c"]);
    }

    #[test]
    fn the_view_follows_the_cursor_off_the_bottom() {
        // Without this the cursor walks past the last visible row and the pane
        // looks frozen: the buffer is changing, just not where anyone is
        // looking.
        let (mut pane, mut doc) = pane_over(&numbered(50), 5);
        for _ in 0..10 {
            pane.handle_key(KeyEvent::Char('j'), &mut doc);
        }
        let (first, lines) = pane.visible_lines(&doc.buffer);
        let cursor_line = doc.buffer.char_to_line(pane.cursors.primary().head);
        assert!(
            (first..first + lines.len()).contains(&cursor_line),
            "cursor on line {cursor_line} but showing {first}..{}",
            first + lines.len()
        );
    }

    #[test]
    fn the_view_follows_the_cursor_back_up() {
        let (mut pane, doc) = pane_over(&numbered(50), 5);
        pane.scroll_top = 30;
        pane.cursors = CursorSet::single(0);
        pane.follow_cursor(&doc.buffer);
        assert_eq!(pane.scroll_top, 0);
    }

    /// The pixel at the start of `row` (0-based, from the first visible line)
    /// and `col`, half a cell in so the test names a character rather than a
    /// boundary between two.
    fn cell_pixel(pane: &EditorPane, buffer: &Buffer, row: usize, col: usize) -> (f32, f32) {
        let (first, lines) = pane.visible_lines(buffer);
        let body = crate::chrome::FrameBody::new(first, lines.len());
        let (cw, ch) = ruster_render_gles::atlas::cell_metrics(crate::compositor::PANE_FONT_PX);
        (
            body.x + col as f32 * cw + cw / 2.0,
            body.y + row as f32 * ch + ch / 2.0,
        )
    }

    #[test]
    fn clicking_a_character_puts_the_cursor_on_it() {
        let (pane, doc) = pane_over("alpha\nbravo\ncharlie", 10);
        let (x, y) = cell_pixel(&pane, &doc.buffer, 1, 3);
        assert_eq!(
            pane.offset_at(&doc.buffer, x, y),
            doc.buffer.line_start_char(1) + 3
        );
    }

    #[test]
    fn clicking_the_gutter_names_that_line_not_the_one_before_it() {
        // The gutter is left of the text, so a naive conversion gives a
        // negative column and wraps onto the previous line.
        let (pane, doc) = pane_over("alpha\nbravo\ncharlie", 10);
        let (_, y) = cell_pixel(&pane, &doc.buffer, 2, 0);
        assert_eq!(
            pane.offset_at(&doc.buffer, 1.0, y),
            doc.buffer.line_start_char(2)
        );
    }

    #[test]
    fn clicking_past_the_end_of_a_line_stops_at_its_end() {
        // Without the clamp the empty space right of a short line runs into the
        // line below — the click lands on text that is nowhere near the pointer.
        let (pane, doc) = pane_over("ab\nlonger line here", 10);
        let (x, y) = cell_pixel(&pane, &doc.buffer, 0, 40);
        assert_eq!(
            pane.offset_at(&doc.buffer, x, y),
            doc.buffer.line_start_char(0) + 2
        );
    }

    #[test]
    fn clicking_below_the_last_line_lands_on_the_last_line() {
        // A pane is usually taller than its text, and a click in that space
        // doing nothing would feel like a dead region of the window.
        let (pane, doc) = pane_over("one\ntwo", 20);
        let (x, y) = cell_pixel(&pane, &doc.buffer, 15, 0);
        assert_eq!(
            pane.offset_at(&doc.buffer, x, y),
            doc.buffer.line_start_char(1)
        );
    }

    #[test]
    fn clicking_a_scrolled_pane_accounts_for_the_scroll() {
        // The row clicked is an offset from the first *visible* line, not from
        // the top of the buffer.
        let text = (0..40)
            .map(|n| format!("line{n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let (mut pane, doc) = pane_over(&text, 10);
        pane.scroll_by(&doc.buffer, 12);
        let (x, y) = cell_pixel(&pane, &doc.buffer, 2, 1);
        assert_eq!(
            pane.offset_at(&doc.buffer, x, y),
            doc.buffer.line_start_char(14) + 1
        );
    }

    #[test]
    fn the_body_origin_moves_as_the_line_numbers_widen() {
        // The gutter grows a column at every power of ten, moving the first
        // text pixel. This asserts the geometry *independently* rather than
        // through a click: a click test computes its pixel from the same
        // `FrameBody` the code reads, so the two shift together and a fixed
        // gutter would sail through it. (It did — that version of this test
        // passed with the width hard-coded.)
        let cw = ruster_render_gles::atlas::cell_metrics(crate::compositor::PANE_FONT_PX).0;
        let narrow = crate::chrome::FrameBody::new(0, 10).x;
        let wide = crate::chrome::FrameBody::new(350, 10).x;
        assert!(
            wide > narrow + cw * 0.9,
            "a 3-digit gutter should be at least a column wider: {narrow} vs {wide}"
        );
    }

    #[test]
    fn a_click_lands_on_the_same_character_however_wide_the_gutter_is() {
        let text = (0..400)
            .map(|n| format!("line{n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let (mut pane, doc) = pane_over(&text, 10);
        for scroll in [0usize, 95, 350] {
            pane.scroll_top = scroll;
            let (x, y) = cell_pixel(&pane, &doc.buffer, 0, 2);
            assert_eq!(
                pane.offset_at(&doc.buffer, x, y),
                doc.buffer.line_start_char(scroll) + 2,
                "clicking column 2 at scroll {scroll}"
            );
        }
    }

    fn diag(line: u32, severity: u8) -> ruster_lsp::results::Diagnostic {
        ruster_lsp::Diagnostic {
            start: ruster_lsp::results::LspPositionEq { line, character: 0 },
            end: ruster_lsp::results::LspPositionEq { line, character: 1 },
            severity,
            message: String::new(),
        }
    }

    #[test]
    fn a_diagnostic_marks_the_line_it_is_on() {
        let pane = EditorPane::new(BufferId(1));
        let sev = pane.line_severities(&[diag(2, 1)], 0, 5);
        assert_eq!(sev, vec![None, None, Some(1), None, None]);
    }

    #[test]
    fn the_worst_severity_on_a_line_is_the_one_shown() {
        // A line with a warning and an error is an error line. Taking whichever
        // arrived first would make the sign depend on the order the server
        // happened to send them in.
        let pane = EditorPane::new(BufferId(1));
        assert_eq!(
            pane.line_severities(&[diag(0, 2), diag(0, 1)], 0, 1),
            vec![Some(1)]
        );
        assert_eq!(
            pane.line_severities(&[diag(0, 1), diag(0, 2)], 0, 1),
            vec![Some(1)],
            "and the other order gives the same answer"
        );
    }

    #[test]
    fn diagnostics_are_placed_relative_to_the_first_visible_line() {
        // The server counts from the top of the file; the pane draws from
        // wherever it is scrolled to. Without the offset every sign lands on
        // the wrong row the moment a pane scrolls.
        let pane = EditorPane::new(BufferId(1));
        let sev = pane.line_severities(&[diag(12, 1)], 10, 4);
        assert_eq!(sev, vec![None, None, Some(1), None]);
    }

    #[test]
    fn diagnostics_off_screen_are_not_drawn_anywhere() {
        // Above or below the viewport must produce nothing rather than being
        // clamped onto the first or last row, which would report a problem on
        // a line that has none.
        let pane = EditorPane::new(BufferId(1));
        assert_eq!(pane.line_severities(&[diag(3, 1)], 10, 3), vec![None; 3]);
        assert_eq!(pane.line_severities(&[diag(99, 1)], 10, 3), vec![None; 3]);
    }

    #[test]
    fn a_pane_shows_the_lines_its_grid_has_room_for() {
        let (pane, doc) = pane_over("one\ntwo\nthree\nfour\nfive", 3);
        let (first, lines) = pane.visible_lines(&doc.buffer);
        assert_eq!(first, 0);
        assert_eq!(lines, vec!["one", "two", "three"]);
    }

    #[test]
    fn scrolling_moves_the_window_over_the_buffer() {
        let (mut pane, doc) = pane_over("one\ntwo\nthree\nfour\nfive", 2);
        pane.scroll_by(&doc.buffer, 2);
        let (first, lines) = pane.visible_lines(&doc.buffer);
        assert_eq!(first, 2);
        assert_eq!(lines, vec!["three", "four"]);
    }

    #[test]
    fn scrolling_stops_at_the_ends_rather_than_running_off() {
        // Past the end would show blank space below a buffer that has more to
        // read above it; before the start is simply not a position.
        let (mut pane, doc) = pane_over("one\ntwo\nthree", 2);
        pane.scroll_by(&doc.buffer, -5);
        assert_eq!(pane.scroll_top, 0);
        pane.scroll_by(&doc.buffer, 500);
        assert_eq!(pane.scroll_top, 2, "the last line stays reachable");
        let (_, lines) = pane.visible_lines(&doc.buffer);
        assert_eq!(lines, vec!["three"]);
    }

    #[test]
    fn growing_a_scrolled_pane_reveals_more_rather_than_leaving_a_gap() {
        // The clamp is applied when reading, not when scrolling, so a pane that
        // was scrolled to the bottom and then resized taller fills the new rows
        // instead of holding a position that is no longer the bottom.
        let (mut pane, doc) = pane_over("1\n2\n3\n4\n5\n6", 2);
        pane.scroll_by(&doc.buffer, 4);
        assert_eq!(pane.visible_lines(&doc.buffer).1, vec!["5", "6"]);
        pane.rows = 6;
        assert_eq!(pane.visible_lines(&doc.buffer).1, vec!["5", "6"]);
        pane.scroll_by(&doc.buffer, -4);
        assert_eq!(pane.visible_lines(&doc.buffer).1.len(), 6);
    }

    #[test]
    fn an_empty_pane_shows_nothing_and_does_not_panic() {
        let (pane, doc) = pane_over("", 0);
        let (first, lines) = pane.visible_lines(&doc.buffer);
        assert_eq!(first, 0);
        assert!(lines.is_empty(), "no rows yet, so nothing visible");
    }

    #[test]
    fn a_rectangle_becomes_the_grid_that_fits_inside_it() {
        // Floor, not round: a partial column has nowhere to put its right-hand
        // half, and rounding up would draw text past the tile's edge.
        assert_eq!(EditorPane::grid_for(800, 600, 10.0, 20.0), (80, 30));
        assert_eq!(EditorPane::grid_for(805, 609, 10.0, 20.0), (80, 30));
    }

    #[test]
    fn a_tile_too_small_for_one_cell_is_an_empty_grid() {
        assert_eq!(EditorPane::grid_for(5, 5, 10.0, 20.0), (0, 0));
    }

    #[test]
    fn a_degenerate_rectangle_does_not_take_the_session_down() {
        // `output_rect()` is 0x0 until the first output is configured, and a
        // pane can be measured in that window.
        assert_eq!(EditorPane::grid_for(0, 0, 10.0, 20.0), (0, 0));
        assert_eq!(EditorPane::grid_for(-100, -100, 10.0, 20.0), (0, 0));
        assert_eq!(EditorPane::grid_for(800, 600, 0.0, 0.0), (0, 0));
    }

    #[test]
    fn a_pane_starts_with_no_grid_until_it_is_laid_out() {
        // The layout is what decides the size; a pane that guessed one would
        // disagree with its own rectangle for the first frame.
        let pane = EditorPane::new(BufferId(7));
        assert_eq!((pane.cols, pane.rows), (0, 0));
        assert_eq!(pane.doc, BufferId(7));
    }
}
