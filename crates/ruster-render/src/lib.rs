#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Default,
    Rgb(u8, u8, u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyntaxStyle {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub italic: bool,
}

impl Default for SyntaxStyle {
    fn default() -> Self {
        SyntaxStyle { fg: Color::Default, bg: Color::Default, bold: false, italic: false }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledLine {
    pub text: String,
    pub highlights: Vec<(usize, usize, SyntaxStyle)>,
}

pub enum CursorKind { Block, Bar }

pub struct EditorState<'a> {
    pub lines: Vec<StyledLine>,
    pub cursor: (u16, u16),
    pub cursor_kind: CursorKind,
    pub cursor_visible: bool,
    pub mode_label: &'a str,
    pub file_path: &'a str,
    pub modified: bool,
    pub cmdline: Option<&'a str>,
    pub message: Option<&'a str>,
}

pub trait Renderer {
    fn render_frame(&mut self, state: &EditorState);
}

#[cfg(test)]
mod tests {
    use crate::{CursorKind, EditorState, Renderer, StyledLine};

    struct TestRenderer;
    impl Renderer for TestRenderer {
        fn render_frame(&mut self, _state: &EditorState) {}
    }

    #[test]
    fn renderer_trait_is_object_safe() {
        let state = EditorState {
            lines: vec![StyledLine { text: "hello".to_string(), highlights: vec![] }],
            cursor: (0, 0),
            cursor_kind: CursorKind::Block,
            cursor_visible: true,
            mode_label: "NORMAL",
            file_path: "test.txt",
            modified: false,
            cmdline: None,
            message: None,
        };
        let mut r = TestRenderer;
        r.render_frame(&state);
    }
}
