use crate::cursor::Edge;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Motion {
    Grapheme(i32),
    Line(i32),
    LineEdge(crate::cursor::Edge),
    To(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditOp {
    InsertChar(char),
    InsertString(String),
    DeleteRange(usize, usize),
    Backspace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Move(Motion),
    Edit(EditOp),
    BeginBatch,
    EndBatch,
    Undo,
    Redo,
}