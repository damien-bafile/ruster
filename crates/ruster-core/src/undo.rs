use crate::buffer::{Buffer, Change};

pub struct UndoStack {
    undo: Vec<Vec<Change>>,
    redo: Vec<Vec<Change>>,
    open: Vec<Change>,
}

impl UndoStack {
    pub fn new() -> Self {
        UndoStack { undo: Vec::new(), redo: Vec::new(), open: Vec::new() }
    }

    pub fn is_empty(&self) -> bool { self.undo.is_empty() && self.open.is_empty() }

    pub fn begin_batch(&mut self) {
        if !self.open.is_empty() {
            let closed = std::mem::take(&mut self.open);
            self.undo.push(closed);
            self.redo.clear();
        }
    }

    pub fn push(&mut self, ch: Change) {
        self.open.push(ch);
    }

    pub fn end_batch(&mut self) {
        if !self.open.is_empty() {
            let closed = std::mem::take(&mut self.open);
            self.undo.push(closed);
            self.redo.clear();
        }
    }

    pub fn undo(&mut self, buffer: &mut Buffer) -> Option<(usize, usize)> {
        self.end_batch(); // close any open batch so it's undoable too
        let batch = self.undo.pop()?;
        let mut inverses = Vec::with_capacity(batch.len());
        for ch in batch.into_iter().rev() {
            let inv = buffer.apply(&ch);
            inverses.push(inv);
        }
        inverses.reverse();
        let at = inverses[0].at;
        let n = inverses.len();
        self.redo.push(inverses);
        Some((n, at))
    }

    pub fn redo(&mut self, buffer: &mut Buffer) -> Option<(usize, usize)> {
        let batch = self.redo.pop()?;
        let mut inverses = Vec::with_capacity(batch.len());
        for ch in batch.into_iter().rev() {
            let inv = buffer.apply(&ch);
            inverses.push(inv);
        }
        inverses.reverse();
        let at = inverses[0].at;
        let n = inverses.len();
        self.undo.push(inverses);
        Some((n, at))
    }
}

impl Default for UndoStack {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack_with_editor() -> (Buffer, UndoStack) {
        (Buffer::from_str("abc"), UndoStack::new())
    }

    #[test]
    fn batched_inserts_undo_as_one_unit() {
        let (mut b, mut u) = stack_with_editor();
        u.begin_batch();
        u.push(b.insert(3, "!"));
        u.push(b.insert(4, "?"));
        u.end_batch();
        assert_eq!(b.to_string(), "abc!?");
        let (n, at) = u.undo(&mut b).unwrap();
        assert_eq!(n, 2);
        assert_eq!(at, 3);
        assert_eq!(b.to_string(), "abc");
    }

    #[test]
    fn new_batch_closes_previous() {
        let (mut b, mut u) = stack_with_editor();
        u.begin_batch();
        u.push(b.insert(3, "!"));
        u.begin_batch(); // opening another batch auto-closes the prior open
        u.push(b.insert(4, "?"));
        u.end_batch();
        assert_eq!(b.to_string(), "abc!?");
        u.undo(&mut b);
        assert_eq!(b.to_string(), "abc!");
        u.undo(&mut b);
        assert_eq!(b.to_string(), "abc");
    }

    #[test]
    fn redo_reapplies_undone_batch() {
        let (mut b, mut u) = stack_with_editor();
        u.begin_batch();
        u.push(b.insert(3, "!"));
        u.end_batch();
        u.undo(&mut b);
        assert_eq!(b.to_string(), "abc");
        let (n, at) = u.redo(&mut b).unwrap();
        assert_eq!(n, 1);
        assert_eq!(at, 3);
        assert_eq!(b.to_string(), "abc!");
    }

    #[test]
    fn new_change_clears_redo() {
        let (mut b, mut u) = stack_with_editor();
        u.begin_batch();
        u.push(b.insert(3, "!"));
        u.end_batch();
        u.undo(&mut b);
        u.begin_batch();
        u.push(b.insert(3, "?"));
        u.end_batch();
        assert!(u.redo(&mut b).is_none(), "redo stack cleared after new edit");
        assert_eq!(b.to_string(), "abc?");
    }
}