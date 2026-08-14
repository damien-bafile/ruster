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
    AddCursor(usize),
    ClearExtraCursors,
    BeginBatch,
    EndBatch,
    Undo,
    Redo,
    /// Scroll the active window by `n` lines without moving the cursor within
    /// the text. Paired with a matching cursor motion by `C-d`/`C-u` so the
    /// cursor keeps its row on screen, the way vim behaves.
    Scroll(i32),
    /// Scroll the active window by `n` columns — `zl`/`zh`.
    ///
    /// Unlike [`Action::Scroll`], no caller pairs this with a cursor motion:
    /// the cursor is dragged along only when the scroll would push it out of
    /// the view, which is what vim does and what the renderer's clamp needs to
    /// see, since it would otherwise undo the scroll to keep the cursor
    /// visible.
    ScrollHorizontal(i32),
    /// Move through the undo tree in creation order rather than along the
    /// current branch — `g-` (`false`) and `g+` (`true`). Reaches states that
    /// were abandoned by branching, which plain redo cannot.
    UndoTime(bool),
    /// Set the anchor (start) of the primary cursor's visual selection,
    /// keeping the current head. Used to extend a selection in visual mode.
    BeginVisual(usize),
    /// Emitted when the user presses Enter in Cmdline mode.
    /// Contains the full cmdline string (e.g. ":w", ":q").
    CmdlineResult(String),
    /// Tree-sitter-backed structural textobject.
    /// op is the operator ('d', 'c', 'y'), kind is 'i' (inner) or 'a' (outer),
    /// target is 'f' (function), 'c' (class), 'l' (loop), 'a' (parameter/argument).
    Textobject {
        op: char,
        kind: char,
        target: char,
        count: u32,
    },
    IndentLine,
    DeindentLine,
    /// Select the word a double-click landed on. Bounds are resolved by the
    /// caller via [`CursorSet::select_word`](crate::cursor::CursorSet::select_word).
    SelectWord {
        anchor: usize,
        head: usize,
    },
    /// Select the line a triple-click landed on.
    SelectLine {
        anchor: usize,
        head: usize,
    },
    /// Set the Emacs mark, which a subsequent motion extends the region from.
    SetMark(usize),
    /// Grow or shrink a window by dragging the split edge it shares with its
    /// neighbour. Deltas are in cells.
    ResizeWindow {
        wid: crate::windows::WindowId,
        dy: i32,
        dx: i32,
    },
}
