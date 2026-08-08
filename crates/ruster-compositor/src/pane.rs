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

use crate::chrome::FrameBody;
use ruster_core::buffer::Buffer;
use ruster_core::cursor::CursorSet;
use ruster_core::editor::{EditSession, EditorView};
use ruster_core::key::KeyEvent;
use ruster_core::undo::UndoStack;
use ruster_core::vim::VimState;
use ruster_shell::WindowId;

/// One editor pane.
///
/// Not `Clone` or `PartialEq`: a `Buffer` is neither, and a pane is identified
/// by the `WindowId` that keys it rather than by its contents anyway.
pub struct EditorPane {
    /// What the frame is titled. Becomes the buffer name in Stage 2.
    pub title: String,
    /// The character grid the pane's rectangle works out to, from
    /// [`cell_metrics`](ruster_render_gles::atlas::cell_metrics).
    ///
    /// Stored rather than derived at draw time because Stage 2 needs it to
    /// decide how much of a buffer is visible, and a scroll clamp that
    /// disagreed with what was drawn would put the cursor off screen.
    pub cols: u16,
    pub rows: u16,
    /// The text this pane is showing.
    ///
    /// Owned per pane for now. Stage 4 moves documents into a shared
    /// `BufferStore` so two panes can show one file and scroll independently —
    /// which is why `scroll_top` lives here and not on the buffer.
    pub buffer: Buffer,
    /// First visible buffer line.
    pub scroll_top: usize,
    /// Where the carets are. A `CursorSet` rather than one position because
    /// that is what `EditSession` edits through, and multi-cursor comes free
    /// with it rather than as a later retrofit.
    pub cursors: CursorSet,
    pub undo: UndoStack,
    /// Modal state. The compositor drives the same `VimState` the editor does,
    /// so a motion cannot behave differently here than it does in the TUI.
    pub vim: VimState,
}

impl EditorView for EditorPane {
    fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    fn primary_head(&self) -> usize {
        self.cursors.primary().head
    }

    fn cursors(&self) -> &CursorSet {
        &self.cursors
    }

    /// The pane's own row count, so half-page motions move by what is on
    /// screen rather than by the trait's headless default of 24.
    fn viewport_height(&self) -> usize {
        self.rows.max(1) as usize
    }
}

impl EditorPane {
    pub fn new(title: impl Into<String>) -> Self {
        EditorPane {
            title: title.into(),
            cols: 0,
            rows: 0,
            buffer: Buffer::new(),
            scroll_top: 0,
            cursors: CursorSet::single(0),
            undo: UndoStack::new(),
            vim: VimState::default(),
        }
    }

    /// A pane showing `text`, titled `title`.
    pub fn with_text(title: impl Into<String>, text: &str) -> Self {
        EditorPane {
            buffer: Buffer::from_str(text),
            ..EditorPane::new(title)
        }
    }

    /// Feed one key to the pane's editor.
    ///
    /// The `VimState` is taken out for the duration because `handle` wants the
    /// pane as an `&dyn EditorView` — an immutable borrow — while the state
    /// doing the handling lives on the pane itself. Putting it back is not
    /// optional and there is no early return between the two.
    pub fn handle_key(&mut self, key: KeyEvent, indent: &str) {
        let mut vim = std::mem::take(&mut self.vim);
        let actions = vim.handle(key, self);
        self.vim = vim;
        {
            let mut session =
                EditSession::new(&mut self.buffer, &mut self.cursors, &mut self.undo, indent);
            for action in actions {
                session.execute(action);
            }
        }
        self.follow_cursor();
    }

    /// Scroll just enough to keep the primary cursor on screen.
    ///
    /// Without this the cursor walks off the top or bottom and the pane looks
    /// frozen — the buffer is changing, just not where anyone is looking.
    pub fn follow_cursor(&mut self) {
        let rows = self.rows as usize;
        if rows == 0 {
            return;
        }
        let line = self.buffer.char_to_line(self.cursors.primary().head);
        if line < self.scroll_top {
            self.scroll_top = line;
        } else if line >= self.scroll_top + rows {
            self.scroll_top = line + 1 - rows;
        }
    }

    /// The buffer lines currently on screen, and the number of the first.
    ///
    /// Clamped to the buffer rather than to the last scroll position, so a pane
    /// that has been scrolled to the bottom and then grown shows the extra rows
    /// instead of leaving a gap it still believes is scrolled past.
    pub fn visible_lines(&self) -> (usize, Vec<String>) {
        let total = self.buffer.line_count();
        let first = self.scroll_top.min(total.saturating_sub(1));
        let last = (first + self.rows as usize).min(total);
        let lines = (first..last)
            .map(|i| {
                // Without the terminator. It draws as nothing, so the screen
                // would look right either way — but every line would measure
                // one character too long, and Stage 3 puts the cursor and
                // click-to-position on exactly that measurement.
                let line = self.buffer.line_to_string(i);
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
    pub fn scroll_by(&mut self, delta: i64) {
        let total = self.buffer.line_count();
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
    pub fn offset_at(&self, x: f32, y: f32) -> usize {
        let (first, lines) = self.visible_lines();
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
        self.buffer.line_start_char(first + row) + col
    }

    /// Put the caret where a click at `(x, y)` — physical pixels from the
    /// pane's top-left — landed.
    ///
    /// Extra cursors go: a click is how a multi-cursor session is ended in
    /// every editor that has one, and leaving them would type in places the
    /// user has just pointed away from.
    pub fn click_at(&mut self, x: f32, y: f32) {
        let at = self.offset_at(x, y);
        self.cursors.clear_extra();
        self.cursors.set_head(at, &self.buffer);
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

    #[test]
    fn typing_in_insert_mode_changes_the_buffer() {
        // The whole point of the stage: keys reach `VimState` and `EditSession`
        // rather than a private editing implementation living in the
        // compositor.
        let mut pane = EditorPane::with_text("f.rs", "abc");
        pane.rows = 10;
        pane.handle_key(KeyEvent::Char('i'), "    ");
        for c in "hi ".chars() {
            pane.handle_key(KeyEvent::Char(c), "    ");
        }
        assert_eq!(pane.visible_lines().1[0], "hi abc");
    }

    #[test]
    fn normal_mode_motions_move_without_editing() {
        let mut pane = EditorPane::with_text("f.rs", "abc\ndef");
        pane.rows = 10;
        for _ in 0..2 {
            pane.handle_key(KeyEvent::Char('l'), "    ");
        }
        pane.handle_key(KeyEvent::Char('j'), "    ");
        assert_eq!(pane.visible_lines().1, vec!["abc", "def"], "text unchanged");
        assert_eq!(
            pane.buffer.char_to_line(pane.cursors.primary().head),
            1,
            "the cursor moved down a line"
        );
    }

    #[test]
    fn undo_puts_back_what_was_typed() {
        // The pane owns an `UndoStack` and hands it to `EditSession`, so undo
        // is the editor's undo rather than something re-implemented here.
        let mut pane = EditorPane::with_text("f.rs", "abc");
        pane.rows = 10;
        pane.handle_key(KeyEvent::Char('i'), "    ");
        pane.handle_key(KeyEvent::Char('X'), "    ");
        assert_eq!(pane.visible_lines().1[0], "Xabc");
        pane.handle_key(KeyEvent::Esc, "    ");
        pane.handle_key(KeyEvent::Char('u'), "    ");
        assert_eq!(pane.visible_lines().1[0], "abc");
    }

    #[test]
    fn the_view_follows_the_cursor_off_the_bottom() {
        // Without this the cursor walks past the last visible row and the pane
        // looks frozen: the buffer is changing, just not where anyone is
        // looking.
        let text = (1..=50)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let mut pane = EditorPane::with_text("f.rs", &text);
        pane.rows = 5;
        for _ in 0..10 {
            pane.handle_key(KeyEvent::Char('j'), "    ");
        }
        let (first, lines) = pane.visible_lines();
        let cursor_line = pane.buffer.char_to_line(pane.cursors.primary().head);
        assert!(
            (first..first + lines.len()).contains(&cursor_line),
            "cursor on line {cursor_line} but showing {first}..{}",
            first + lines.len()
        );
    }

    #[test]
    fn the_view_follows_the_cursor_back_up() {
        let text = (1..=50)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let mut pane = EditorPane::with_text("f.rs", &text);
        pane.rows = 5;
        pane.scroll_top = 30;
        pane.cursors = CursorSet::single(0);
        pane.follow_cursor();
        assert_eq!(pane.scroll_top, 0);
    }

    /// The pixel at the start of `row` (0-based, from the first visible line)
    /// and `col`, half a cell in so the test names a character rather than a
    /// boundary between two.
    fn cell_pixel(pane: &EditorPane, row: usize, col: usize) -> (f32, f32) {
        let (first, lines) = pane.visible_lines();
        let body = crate::chrome::FrameBody::new(first, lines.len());
        let (cw, ch) = ruster_render_gles::atlas::cell_metrics(crate::compositor::PANE_FONT_PX);
        (
            body.x + col as f32 * cw + cw / 2.0,
            body.y + row as f32 * ch + ch / 2.0,
        )
    }

    #[test]
    fn clicking_a_character_puts_the_cursor_on_it() {
        let mut pane = EditorPane::with_text("f.rs", "alpha\nbravo\ncharlie");
        pane.rows = 10;
        let (x, y) = cell_pixel(&pane, 1, 3);
        assert_eq!(pane.offset_at(x, y), pane.buffer.line_start_char(1) + 3);
    }

    #[test]
    fn clicking_the_gutter_names_that_line_not_the_one_before_it() {
        // The gutter is left of the text, so a naive conversion gives a
        // negative column and wraps onto the previous line.
        let mut pane = EditorPane::with_text("f.rs", "alpha\nbravo\ncharlie");
        pane.rows = 10;
        let (_, y) = cell_pixel(&pane, 2, 0);
        assert_eq!(pane.offset_at(1.0, y), pane.buffer.line_start_char(2));
    }

    #[test]
    fn clicking_past_the_end_of_a_line_stops_at_its_end() {
        // Without the clamp the empty space right of a short line runs into the
        // line below — the click lands on text that is nowhere near the pointer.
        let mut pane = EditorPane::with_text("f.rs", "ab\nlonger line here");
        pane.rows = 10;
        let (x, y) = cell_pixel(&pane, 0, 40);
        assert_eq!(pane.offset_at(x, y), pane.buffer.line_start_char(0) + 2);
    }

    #[test]
    fn clicking_below_the_last_line_lands_on_the_last_line() {
        // A pane is usually taller than its text, and a click in that space
        // doing nothing would feel like a dead region of the window.
        let mut pane = EditorPane::with_text("f.rs", "one\ntwo");
        pane.rows = 20;
        let (x, y) = cell_pixel(&pane, 15, 0);
        assert_eq!(pane.offset_at(x, y), pane.buffer.line_start_char(1));
    }

    #[test]
    fn clicking_a_scrolled_pane_accounts_for_the_scroll() {
        // The row clicked is an offset from the first *visible* line, not from
        // the top of the buffer.
        let text = (0..40)
            .map(|n| format!("line{n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut pane = EditorPane::with_text("f.rs", &text);
        pane.rows = 10;
        pane.scroll_by(12);
        let (x, y) = cell_pixel(&pane, 2, 1);
        assert_eq!(pane.offset_at(x, y), pane.buffer.line_start_char(14) + 1);
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
        let mut pane = EditorPane::with_text("f.rs", &text);
        pane.rows = 10;
        for scroll in [0usize, 95, 350] {
            pane.scroll_top = scroll;
            let (x, y) = cell_pixel(&pane, 0, 2);
            assert_eq!(
                pane.offset_at(x, y),
                pane.buffer.line_start_char(scroll) + 2,
                "clicking column 2 at scroll {scroll}"
            );
        }
    }

    #[test]
    fn a_pane_shows_the_lines_its_grid_has_room_for() {
        let mut pane = EditorPane::with_text("f.rs", "one\ntwo\nthree\nfour\nfive");
        pane.rows = 3;
        let (first, lines) = pane.visible_lines();
        assert_eq!(first, 0);
        assert_eq!(lines, vec!["one", "two", "three"]);
    }

    #[test]
    fn scrolling_moves_the_window_over_the_buffer() {
        let mut pane = EditorPane::with_text("f.rs", "one\ntwo\nthree\nfour\nfive");
        pane.rows = 2;
        pane.scroll_by(2);
        let (first, lines) = pane.visible_lines();
        assert_eq!(first, 2);
        assert_eq!(lines, vec!["three", "four"]);
    }

    #[test]
    fn scrolling_stops_at_the_ends_rather_than_running_off() {
        // Past the end would show blank space below a buffer that has more to
        // read above it; before the start is simply not a position.
        let mut pane = EditorPane::with_text("f.rs", "one\ntwo\nthree");
        pane.rows = 2;
        pane.scroll_by(-5);
        assert_eq!(pane.scroll_top, 0);
        pane.scroll_by(500);
        assert_eq!(pane.scroll_top, 2, "the last line stays reachable");
        let (_, lines) = pane.visible_lines();
        assert_eq!(lines, vec!["three"]);
    }

    #[test]
    fn growing_a_scrolled_pane_reveals_more_rather_than_leaving_a_gap() {
        // The clamp is applied when reading, not when scrolling, so a pane that
        // was scrolled to the bottom and then resized taller fills the new rows
        // instead of holding a position that is no longer the bottom.
        let mut pane = EditorPane::with_text("f.rs", "1\n2\n3\n4\n5\n6");
        pane.rows = 2;
        pane.scroll_by(4);
        assert_eq!(pane.visible_lines().1, vec!["5", "6"]);
        pane.rows = 6;
        assert_eq!(pane.visible_lines().1, vec!["5", "6"]);
        pane.scroll_by(-4);
        assert_eq!(pane.visible_lines().1.len(), 6);
    }

    #[test]
    fn an_empty_pane_shows_nothing_and_does_not_panic() {
        let pane = EditorPane::new("scratch");
        let (first, lines) = pane.visible_lines();
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
        let pane = EditorPane::new("scratch");
        assert_eq!((pane.cols, pane.rows), (0, 0));
        assert_eq!(pane.title, "scratch");
    }
}
