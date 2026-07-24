use crate::action::{Action, EditOp, Motion};
use crate::buffer::Buffer;
use crate::cursor::CursorSet;
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
        let all: Vec<usize> = if self.cursors.count() > 1 {
            let mut positions: Vec<usize> = (0..self.cursors.count())
                .map(|i| self.cursors.cursors[i].head)
                .collect();
            positions.sort_unstable_by(|a, b| b.cmp(a));
            positions
        } else {
            vec![self.cursors.head()]
        };

        for &at in &all {
            match e.clone() {
                EditOp::InsertChar(c) => {
                    let mut buf = [0u8; 4];
                    let text = c.encode_utf8(&mut buf);
                    let ch = self.buffer.insert(at, text);
                    self.undo.push(ch);
                    if all.len() == 1 {
                        self.cursors.set_head(at + 1, self.buffer);
                    }
                }
                EditOp::InsertString(s) => {
                    let n = s.chars().count();
                    let ch = self.buffer.insert(at, &s);
                    self.undo.push(ch);
                    if all.len() == 1 {
                        self.cursors.set_head(at + n, self.buffer);
                    }
                }
                EditOp::DeleteRange(start, end) if end > start => {
                    let safe_end = end.min(self.buffer.len_chars());
                    let ch = self.buffer.delete(start..safe_end);
                    self.undo.push(ch);
                    if all.len() == 1 {
                        self.cursors.set_head(start, self.buffer);
                    }
                }
                EditOp::DeleteRange(_, _) => {}
                EditOp::Backspace => {
                    if at > 0 {
                        let ch = self.buffer.delete(at - 1..at);
                        self.undo.push(ch);
                        if all.len() == 1 {
                            self.cursors.set_head(at - 1, self.buffer);
                        }
                    }
                }
            }
        }
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