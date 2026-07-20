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
    /// Emitted when the user presses Enter in Cmdline mode.
    /// Contains the full cmdline string (e.g. ":w", ":q").
    CmdlineResult(String),
    /// Tree-sitter-backed structural textobject.
    /// op is the operator ('d', 'c', 'y'), kind is 'i' (inner) or 'a' (outer),
    /// target is 'f' (function), 'c' (class), 'l' (loop), 'a' (parameter/argument).
    Textobject { op: char, kind: char, target: char, count: u32 },
}