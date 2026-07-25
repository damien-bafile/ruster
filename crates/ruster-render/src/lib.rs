#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Default,
    Rgb(u8, u8, u8),
}

/// The GUI color palette. Defaults mirror the previously hardcoded raylib values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub bg: Color,
    pub fg: Color,
    pub gutter: Color,
    pub selection: Color,
    pub cursor: Color,
    pub divider: Color,
    pub accent: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            bg: Color::Rgb(30, 30, 30),
            fg: Color::Rgb(205, 214, 244),
            gutter: Color::Rgb(108, 112, 134),
            selection: Color::Rgb(88, 91, 112),
            cursor: Color::Rgb(245, 224, 220),
            divider: Color::Rgb(69, 71, 90),
            accent: Color::Rgb(243, 139, 168),
        }
    }
}

/// Config-driven GUI metrics + palette handed to the raylib renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuiConfig {
    pub font_size: i32,
    pub line_height: i32,
    pub padding_x: i32,
    pub padding_y: i32,
    pub window_width: i32,
    pub window_height: i32,
    pub target_fps: i32,
    pub cursor_kind: CursorKind,
    pub theme: Theme,
}

impl Default for GuiConfig {
    fn default() -> Self {
        GuiConfig {
            font_size: 20,
            line_height: 24,
            padding_x: 8,
            padding_y: 4,
            window_width: 800,
            window_height: 600,
            target_fps: 60,
            cursor_kind: CursorKind::Block,
            theme: Theme::default(),
        }
    }
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

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
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
    // Hybrid (both on) gets an extra column of padding so the absolute number on
    // the cursor line and the relative numbers elsewhere are easier to tell apart.
    let pad = if number && relativenumber { 2 } else { 1 };
    let width = (num_w + pad) as u16;

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
        rows.push(format!("{:>num_w$}{}", value, " ".repeat(pad)));
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

/// How a visual selection covers the lines it spans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionKind {
    /// From the start column on the first line to the end column on the last.
    Char,
    /// Whole lines, ignoring columns.
    Line,
    /// The same column range on every line (rectangle).
    Block,
}

/// A visual-mode selection in buffer coordinates. `start`/`end` are
/// `(line, col)` and both ends are **inclusive**.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionView {
    pub start: (u16, u16),
    pub end: (u16, u16),
    pub kind: SelectionKind,
}

impl SelectionView {
    /// The inclusive column span selected on `line`, if any. `line_len` is the
    /// line's character count.
    pub fn span_on(&self, line: u16, line_len: u16) -> Option<(u16, u16)> {
        if line < self.start.0 || line > self.end.0 {
            return None;
        }
        match self.kind {
            SelectionKind::Line => Some((0, line_len)),
            SelectionKind::Block => {
                // A rectangle: the same columns on every line, clipped to it.
                let (lo, hi) = if self.start.1 <= self.end.1 {
                    (self.start.1, self.end.1)
                } else {
                    (self.end.1, self.start.1)
                };
                if lo > line_len {
                    None
                } else {
                    Some((lo, hi.min(line_len)))
                }
            }
            SelectionKind::Char => {
                let start = if line == self.start.0 { self.start.1 } else { 0 };
                let end = if line == self.end.0 { self.end.1 } else { line_len };
                Some((start, end))
            }
        }
    }
}

/// One cell of a rendered terminal grid: a character plus its colors and
/// attributes. `fg`/`bg` of `Color::Default` mean "use the theme default".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermCellView {
    pub c: char,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
}

impl Default for TermCellView {
    fn default() -> Self {
        TermCellView {
            c: ' ',
            fg: Color::Default,
            bg: Color::Default,
            bold: false,
            italic: false,
            underline: false,
            inverse: false,
        }
    }
}

/// A terminal grid to draw in place of a window's styled text. Cells are stored
/// row-major (`rows * cols`); `cursor` is `(row, col)` within the grid. When a
/// `WindowView` carries one of these, renderers draw the grid and ignore
/// `lines`/`gutter`/`selection`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermGridView {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<TermCellView>,
    pub cursor: (usize, usize),
}

/// Everything needed to draw a single window into its rectangle.
pub struct WindowView {
    pub rect: Rect,
    pub lines: Vec<StyledLine>,
    /// Absolute cursor position in buffer coords: (line, col).
    pub cursor: (u16, u16),
    /// Additional multi-cursor carets in buffer coords, drawn as blocks. Empty
    /// in the common single-cursor case.
    pub extra_cursors: Vec<(u16, u16)>,
    pub cursor_kind: CursorKind,
    pub cursor_visible: bool,
    pub cursor_smooth: Option<(f32, f32)>,
    /// First visible buffer line (vertical scroll).
    pub scroll_offset: u16,
    pub gutter: GutterView,
    pub statusline: StatuslineView,
    pub active: bool,
    /// Visual-mode selection to highlight (active window only).
    pub selection: Option<SelectionView>,
    /// When set, this window is a terminal: draw the grid instead of `lines`.
    pub terminal: Option<TermGridView>,
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
    /// Syntax-highlighted preview of the selected entry (empty = no preview pane).
    pub preview: Vec<StyledLine>,
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

/// The kind of control a settings row renders as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlKind {
    Toggle,
    Enum,
    Number,
    Text,
}

/// One row on the settings page: a label, its control, and current value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingRowView {
    pub label: String,
    pub kind: ControlKind,
    /// The value as displayed (e.g. "on"/"off", "neovim", "20", "#1e1e1e").
    pub value: String,
    /// True while the user is typing into this field.
    pub editing: bool,
    pub selected: bool,
    pub help: String,
    /// A `#RRGGBB` color to draw as a swatch beside the value (color rows).
    pub swatch: Option<String>,
}

/// A named group of settings rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsGroup {
    pub name: String,
    pub rows: Vec<SettingRowView>,
}

/// The settings page: grouped rows, a dirty flag, and a footer hint line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsView {
    pub groups: Vec<SettingsGroup>,
    pub dirty: bool,
    pub footer: String,
}

/// A full frame: every visible window, the shared cmdline/message line, an
/// optional centered picker overlay, and an optional bottom which-key panel.
pub struct FrameState<'a> {
    pub windows: Vec<WindowView>,
    pub cmdline: Option<&'a str>,
    pub message: Option<&'a str>,
    pub picker: Option<PickerView>,
    pub whichkey: Option<WhichKeyView>,
    /// LSP hover popup lines (syntax-highlighted), in a floating box near the top.
    pub hover: Option<Vec<StyledLine>>,
    /// The settings page overlay, when open.
    pub settings: Option<SettingsView>,
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
    /// Re-apply GUI metrics + theme (and reload the font) at runtime, for live
    /// re-theming from the Settings page. Default no-op (e.g. the TUI backend).
    fn set_gui_config(&mut self, _gui: &GuiConfig, _font: Option<&str>) {}
}

#[cfg(test)]
mod tests {
    use crate::{
        Color, CursorKind, FrameState, GutterView, Rect, Renderer, SelectionKind, SelectionView,
        StatuslineView, StyledLine, TermCellView, TermGridView, WindowView,
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
            extra_cursors: Vec::new(),
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
            selection: None,
            terminal: None,
        }
    }

    #[test]
    fn terminal_grid_view_holds_cells() {
        let grid = TermGridView {
            cols: 2,
            rows: 1,
            cells: vec![
                TermCellView { c: 'h', fg: Color::Rgb(1, 2, 3), ..TermCellView::default() },
                TermCellView { c: 'i', ..TermCellView::default() },
            ],
            cursor: (0, 1),
        };
        let mut w = sample_window();
        w.terminal = Some(grid.clone());
        assert_eq!(w.terminal.as_ref().unwrap().cells[0].c, 'h');
        assert_eq!(w.terminal.as_ref().unwrap().cursor, (0, 1));
    }

    #[test]
    fn selection_spans_per_line() {
        // charwise from (1,4) to (3,2)
        let sel = SelectionView { start: (1, 4), end: (3, 2), kind: SelectionKind::Char };
        assert_eq!(sel.span_on(0, 10), None, "before the selection");
        assert_eq!(sel.span_on(1, 10), Some((4, 10)), "first line: from start col");
        assert_eq!(sel.span_on(2, 10), Some((0, 10)), "middle line: whole line");
        assert_eq!(sel.span_on(3, 10), Some((0, 2)), "last line: up to end col");
        assert_eq!(sel.span_on(4, 10), None, "after the selection");

        // single-line charwise
        let one = SelectionView { start: (2, 3), end: (2, 7), kind: SelectionKind::Char };
        assert_eq!(one.span_on(2, 20), Some((3, 7)));

        // line-wise ignores columns
        let lw = SelectionView { start: (1, 5), end: (2, 1), kind: SelectionKind::Line };
        assert_eq!(lw.span_on(1, 8), Some((0, 8)));
        assert_eq!(lw.span_on(2, 4), Some((0, 4)));

        // block-wise selects the same columns on every line, clipped per line
        let blk = SelectionView { start: (1, 2), end: (3, 5), kind: SelectionKind::Block };
        assert_eq!(blk.span_on(1, 10), Some((2, 5)));
        assert_eq!(blk.span_on(2, 10), Some((2, 5)));
        assert_eq!(blk.span_on(3, 4), Some((2, 4)), "clipped to a short line");
        assert_eq!(blk.span_on(3, 1), None, "line ends before the block starts");
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
            settings: None,
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
    fn gutter_hybrid_is_wider_than_single() {
        let single = gutter_view(0, 100, 0, true, false, 3).width;
        let hybrid = gutter_view(0, 100, 0, true, true, 3).width;
        assert!(hybrid > single, "hybrid gutter is wider: {hybrid} vs {single}");
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
