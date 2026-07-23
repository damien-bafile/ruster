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

/// Build the line-number gutter for a window.
///
/// - `number` only → absolute line numbers
/// - `relativenumber` only → distance from `cursor_line` (0 on the cursor line)
/// - both → hybrid: absolute on the cursor line, relative elsewhere
/// - neither → empty gutter (width 0)
///
/// `first_line` is the first visible buffer line (scroll top); `height` is the
/// number of visible text rows. Rows are right-aligned and padded to `width`,
/// which is `max(3, digits(line_count)) + 1` (one trailing space).
pub fn gutter_view(
    first_line: usize,
    line_count: usize,
    cursor_line: usize,
    number: bool,
    relativenumber: bool,
    height: usize,
) -> GutterView {
    if !number && !relativenumber {
        return GutterView::default();
    }
    let digits = line_count.max(1).to_string().len();
    let num_w = digits.max(3);
    let width = (num_w + 1) as u16;

    let mut rows = Vec::new();
    for row in 0..height {
        let line = first_line + row;
        if line >= line_count.max(1) {
            break;
        }
        let value = if number && !relativenumber {
            line + 1
        } else if !number && relativenumber {
            line.abs_diff(cursor_line)
        } else if line == cursor_line {
            line + 1
        } else {
            line.abs_diff(cursor_line)
        };
        rows.push(format!("{:>width$} ", value, width = num_w));
    }
    GutterView { width, rows }
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

/// One row of a floating picker overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerRow {
    pub label: String,
    pub selected: bool,
}

/// A floating fuzzy-list overlay (buffer list, file finder, which-key, ...),
/// drawn centered over the window frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerView {
    pub title: String,
    pub query: String,
    pub rows: Vec<PickerRow>,
}

/// A which-key hint panel that slides up from the bottom mini-buffer. `anim` is
/// the slide progress in `0.0..=1.0` (0 = fully hidden below the screen edge,
/// 1 = fully visible).
#[derive(Debug, Clone, PartialEq)]
pub struct WhichKeyView {
    pub title: String,
    pub rows: Vec<String>,
    pub anim: f32,
}

/// A full frame: every visible window, the shared cmdline/message line, an
/// optional centered picker overlay, and an optional bottom which-key panel.
pub struct FrameState<'a> {
    pub windows: Vec<WindowView>,
    pub cmdline: Option<&'a str>,
    pub message: Option<&'a str>,
    pub picker: Option<PickerView>,
    pub whichkey: Option<WhichKeyView>,
    /// LSP hover popup lines, shown in a floating box near the top.
    pub hover: Option<Vec<String>>,
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
            picker: None,
            whichkey: None,
            hover: None,
        };
        let mut r = TestRenderer;
        r.render_frame(&state);
        assert_eq!(r.viewport_cells(), (80, 24));
    }

    use crate::gutter_view;

    #[test]
    fn gutter_disabled_has_zero_width() {
        let g = gutter_view(0, 10, 0, false, false, 5);
        assert_eq!(g.width, 0);
        assert!(g.rows.is_empty());
    }

    #[test]
    fn gutter_absolute_numbers() {
        let g = gutter_view(0, 3, 0, true, false, 3);
        assert_eq!(g.width, 4); // max(3,1)+1
        assert_eq!(g.rows, vec!["  1 ".to_string(), "  2 ".to_string(), "  3 ".to_string()]);
    }

    #[test]
    fn gutter_relative_distance_from_cursor() {
        // cursor on line index 2, lines 0..5 visible
        let g = gutter_view(0, 5, 2, false, true, 5);
        let vals: Vec<&str> = g.rows.iter().map(|s| s.trim()).collect();
        assert_eq!(vals, vec!["2", "1", "0", "1", "2"]);
    }

    #[test]
    fn gutter_hybrid_absolute_on_cursor_line() {
        // cursor on line index 1
        let g = gutter_view(0, 4, 1, true, true, 4);
        let vals: Vec<&str> = g.rows.iter().map(|s| s.trim()).collect();
        // line0 -> rel 1, line1 -> abs 2, line2 -> rel 1, line3 -> rel 2
        assert_eq!(vals, vec!["1", "2", "1", "2"]);
    }

    #[test]
    fn gutter_width_scales_with_line_count() {
        let g = gutter_view(0, 1000, 0, true, false, 1);
        assert_eq!(g.width, 5); // digits(1000)=4, +1
    }

    #[test]
    fn gutter_respects_scroll_and_stops_at_eof() {
        // 3 lines total, scrolled so first visible is line 2, height 5
        let g = gutter_view(2, 3, 2, true, false, 5);
        assert_eq!(g.rows.len(), 1); // only line index 2 exists
        assert_eq!(g.rows[0].trim(), "3");
    }
}
