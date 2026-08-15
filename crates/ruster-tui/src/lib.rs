pub mod app;
pub mod debug_state;
pub mod dialog;
pub mod dired;
pub mod file_prompt;
pub mod git_gutter;
pub mod git_status;
pub mod help;
pub mod key;
/// The language-server surface, now shared with the compositor.
///
/// Re-exported under its old name so the 21 call sites in `app.rs` did not have
/// to move with it.
pub mod lsp_state {
    pub use ruster_lsp::state::*;
}
pub mod mason;
pub mod mouse;
pub mod picker;
pub mod quickfix;
pub mod renderer;
pub mod runner;
pub mod settings;
pub mod sidebar;
pub mod trouble;
pub mod widgets;

#[cfg(test)]
mod tests {
    use ruster_render::{
        CursorKind, FrameState, Rect, Renderer, StatusSection, StatuslineView, StyledLine, UIMode,
        WindowView,
    };

    #[test]
    fn tui_renderer_accepts_frame_state() {
        let mut r = crate::renderer::TuiRenderer::dummy();
        let view = WindowView {
            rect: Rect::new(0, 0, 80, 24),
            header: "f".to_string(),
            lines: vec![StyledLine {
                text: "hi".to_string(),
                highlights: vec![],
            }],
            cursor: (0, 1),
            cursor_kind: CursorKind::Bar,
            cursor_visible: true,
            statusline: StatuslineView {
                left: vec![StatusSection::new("mode", "INSERT")],
                center: vec![StatusSection::new("file", "f")],
                right: vec![StatusSection::new("position", "1,2")],
                active: true,
                mode: UIMode::default(),
            },
            active: true,
            ..Default::default()
        };
        let state = FrameState {
            windows: vec![view],
            ..Default::default()
        };
        // Dummy renderer has no terminal; this exercises the type wiring.
        r.render_frame(&state);
    }
}
