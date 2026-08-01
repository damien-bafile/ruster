pub mod app;
pub mod dired;
pub mod file_prompt;
pub mod key;
pub mod picker;
pub mod quickfix;
pub mod runner;
pub mod renderer;
pub mod settings;
pub mod sidebar;
pub mod trouble;
pub mod widgets;

#[cfg(test)]
mod tests {
    use ruster_render::{
        CursorKind, FrameState, Rect, Renderer, StatuslineView, StyledLine, UIMode, WindowView,
    };

    #[test]
    fn tui_renderer_accepts_frame_state() {
        let mut r = crate::renderer::TuiRenderer::dummy();
        let view = WindowView {
            rect: Rect::new(0, 0, 80, 24),
            header: "f".to_string(),
            lines: vec![StyledLine { text: "hi".to_string(), highlights: vec![] }],
            cursor: (0, 1),
            cursor_kind: CursorKind::Bar,
            cursor_visible: true,
            statusline: StatuslineView {
                left: "INSERT".into(),
                center: "f".into(),
                right: "1,2".into(),
                active: true,
                mode: UIMode::default(),
            },
            active: true,
            ..Default::default()
        };
        let state = FrameState { windows: vec![view], ..Default::default() };
        // Dummy renderer has no terminal; this exercises the type wiring.
        r.render_frame(&state);
    }
}
