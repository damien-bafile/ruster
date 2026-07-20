use crate::cursor::Edge;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Motion {
    Grapheme(i32),
    Line(i32),
    LineEdge(Edge),
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
    /// Set the anchor (start) of the primary cursor's visual selection,
    /// keeping the current head. Used to extend a selection in visual mode.
    BeginVisual(usize),
}