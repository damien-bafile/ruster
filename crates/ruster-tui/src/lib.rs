pub mod key;
pub mod renderer;
pub mod widgets;

#[cfg(test)]
mod tests {
    use ruster_render::{CursorKind, EditorState, Renderer};

    #[test]
    fn tui_renderer_accepts_editor_state() {
        let mut r = crate::renderer::TuiRenderer::dummy();
        let state = EditorState {
            lines: vec!["hi".to_string()],
            cursor: (0, 1),
            cursor_kind: CursorKind::Bar,
            mode_label: "INSERT",
            file_path: "f",
            modified: false,
            cmdline: None,
            message: None,
        };
        r.render_frame(&state);
    }
}
