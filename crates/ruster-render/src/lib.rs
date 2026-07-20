pub enum CursorKind { Block, Bar }

pub struct EditorState<'a> {
    pub lines: Vec<String>,
    pub cursor: (u16, u16),
    pub cursor_kind: CursorKind,
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
    use crate::{CursorKind, EditorState, Renderer};

    struct TestRenderer;
    impl Renderer for TestRenderer {
        fn render_frame(&mut self, _state: &EditorState) {}
    }

    #[test]
    fn renderer_trait_is_object_safe() {
        let state = EditorState {
            lines: vec!["hello".to_string()],
            cursor: (0, 0),
            cursor_kind: CursorKind::Block,
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
