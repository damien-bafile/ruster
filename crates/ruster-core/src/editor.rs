use crate::action::{Action, EditOp, Motion};
use crate::buffer::Buffer;
use crate::cursor::CursorSet;
use crate::undo::UndoStack;

pub struct Editor {
    buffer: Buffer,
    cursors: CursorSet,
    undo: UndoStack,
}

impl Editor {
    pub fn from_str(s: &str) -> Self {
        let len = s.chars().count();
        Editor {
            buffer: Buffer::from_str(s),
            cursors: CursorSet::single(len),
            undo: UndoStack::new(),
        }
    }

    pub fn buffer(&self) -> &Buffer { &self.buffer }
    pub fn cursors(&self) -> &CursorSet { &self.cursors }
    pub fn primary_head(&self) -> usize { self.cursors.head() }

    pub fn execute(&mut self, action: Action) {
        match action {
            Action::BeginBatch => self.undo.begin_batch(),
            Action::EndBatch => self.undo.end_batch(),
            Action::Undo => {
                self.undo.undo(&mut self.buffer);
                let clamp = (self.buffer.len_chars()).saturating_sub(0);
                let at = 0.min(clamp);
                self.cursors.set_head(at, &self.buffer);
            }
            Action::Redo => {
                self.undo.redo(&mut self.buffer);
                let clamp = (self.buffer.len_chars()).saturating_sub(0);
                let at = 0.min(clamp);
                self.cursors.set_head(at, &self.buffer);
            }
            Action::Move(m) => self.apply_motion(m),
            Action::Edit(e) => self.apply_edit(e),
        }
    }

    fn apply_motion(&mut self, m: Motion) {
        match m {
            Motion::Grapheme(d) => self.cursors.move_grapheme(&self.buffer, d),
            Motion::Line(d) => self.cursors.move_line(&self.buffer, d),
            Motion::LineEdge(edge) => self.cursors.move_line_edge(&self.buffer, edge),
            Motion::To(target) => self.cursors.set_head(target, &self.buffer),
        }
    }

    fn apply_edit(&mut self, e: EditOp) {
        let at = self.cursors.head();
        match e {
            EditOp::InsertChar(c) => {
                let mut buf = [0u8; 4];
                let text = c.encode_utf8(&mut buf);
                let ch = self.buffer.insert(at, text);
                self.undo.push(ch);
                self.cursors.set_head(at + 1, &self.buffer);
            }
            EditOp::InsertString(s) => {
                let n = s.chars().count();
                let ch = self.buffer.insert(at, &s);
                self.undo.push(ch);
                self.cursors.set_head(at + n, &self.buffer);
            }
            EditOp::DeleteRange(start, end) if end > start => {
                let ch = self.buffer.delete(start..end);
                self.undo.push(ch);
                self.cursors.set_head(start, &self.buffer);
            }
            EditOp::DeleteRange(_, _) => {}
            EditOp::Backspace => {
                if at > 0 {
                    let ch = self.buffer.delete(at - 1..at);
                    self.undo.push(ch);
                    self.cursors.set_head(at - 1, &self.buffer);
                }
            }
        }
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
        // End edge lands past the last char (consistent with cursor.rs
        // `move_line_edge_to_end` semantics: "hello world" -> 11).
        assert_eq!(e.primary_head(), 3);
    }
}