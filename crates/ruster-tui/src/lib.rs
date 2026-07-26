pub mod app;
pub mod key;
pub mod picker;
pub mod quickfix;
pub mod runner;
pub mod renderer;
pub mod settings;
pub mod widgets;

#[cfg(test)]
mod tests {
    use ruster_render::{
        CursorKind, FrameState, GutterView, Rect, Renderer, StatuslineView, StyledLine, WindowView,
    };

    #[test]
    fn tui_renderer_accepts_frame_state() {
        let mut r = crate::renderer::TuiRenderer::dummy();
        let view = WindowView {
            rect: Rect::new(0, 0, 80, 24),
            lines: vec![StyledLine { text: "hi".to_string(), highlights: vec![] }],
            cursor: (0, 1),
            extra_cursors: Vec::new(),
            cursor_kind: CursorKind::Bar,
            cursor_visible: true,
            cursor_smooth: None,
            scroll_offset: 0,
            gutter: GutterView::default(),
            signs: ruster_render::SignsView::default(),
            statusline: StatuslineView {
                left: "INSERT".into(),
                center: "f".into(),
                right: "1,2".into(),
                active: true,
            },
            active: true,
            selection: None,
            terminal: None,
        };
        let state = FrameState { windows: vec![view], cmdline: None, message: None, picker: None, whichkey: None, hover: None, settings: None };
        // Dummy renderer has no terminal; this exercises the type wiring.
        r.render_frame(&state);
    }
}
