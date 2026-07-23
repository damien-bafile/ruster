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

#[derive(Copy, Clone)]
pub enum CursorKind { Block, Bar }

/// A rectangle in cell coordinates (origin top-left). Mirrors
/// `ruster_core::windows::Rect`; kept local so this crate stays dependency-free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    pub fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Rect { x, y, width, height }
    }
}

/// The rendered line-number column for one window. `rows` are pre-formatted,
/// right-aligned strings aligned to the window's visible text rows. `width` is
/// 0 when the gutter is disabled.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GutterView {
    pub width: u16,
    pub rows: Vec<String>,
}

/// One window's statusline, split into left/center/right groups. `active`
/// selects the highlighted vs dimmed style.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatuslineView {
    pub left: String,
    pub center: String,
    pub right: String,
    pub active: bool,
}

/// Everything needed to draw a single window into its rectangle.
pub struct WindowView {
    pub rect: Rect,
    pub lines: Vec<StyledLine>,
    /// Absolute cursor position in buffer coords: (line, col).
    pub cursor: (u16, u16),
    pub cursor_kind: CursorKind,
    pub cursor_visible: bool,
    pub cursor_smooth: Option<(f32, f32)>,
    /// First visible buffer line (vertical scroll).
    pub scroll_offset: u16,
    pub gutter: GutterView,
    pub statusline: StatuslineView,
    pub active: bool,
}

/// A full frame: every visible window, plus the shared cmdline/message line.
pub struct FrameState<'a> {
    pub windows: Vec<WindowView>,
    pub cmdline: Option<&'a str>,
    pub message: Option<&'a str>,
}

pub trait Renderer {
    fn render_frame(&mut self, state: &FrameState);
    /// The drawable area in text cells: (columns, rows). The app uses this to
    /// compute window rectangles. Default suits a headless/dummy renderer.
    fn viewport_cells(&self) -> (u16, u16) {
        (80, 24)
    }
    fn poll_input(&mut self) -> Option<crossterm::event::KeyEvent> {
        None
    }
    fn should_close(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        CursorKind, FrameState, GutterView, Rect, Renderer, StatuslineView, StyledLine, WindowView,
    };

    struct TestRenderer;
    impl Renderer for TestRenderer {
        fn render_frame(&mut self, _state: &FrameState) {}
    }

    fn sample_window() -> WindowView {
        WindowView {
            rect: Rect::new(0, 0, 80, 24),
            lines: vec![StyledLine { text: "hello".to_string(), highlights: vec![] }],
            cursor: (0, 0),
            cursor_kind: CursorKind::Block,
            cursor_visible: true,
            cursor_smooth: None,
            scroll_offset: 0,
            gutter: GutterView::default(),
            statusline: StatuslineView {
                left: "NORMAL".into(),
                center: "test.txt".into(),
                right: "1,1".into(),
                active: true,
            },
            active: true,
        }
    }

    #[test]
    fn renderer_trait_is_object_safe() {
        let state = FrameState {
            windows: vec![sample_window()],
            cmdline: None,
            message: None,
        };
        let mut r = TestRenderer;
        r.render_frame(&state);
        assert_eq!(r.viewport_cells(), (80, 24));
    }
}
