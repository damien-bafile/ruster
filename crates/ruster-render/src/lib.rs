/// Editing mode for statusline coloring.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum UIMode {
    #[default]
    Normal,
    Insert,
    Visual,
    Cmdline,
    Emacs,
}

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
    /// Gutter background (defaults to `bg` when a theme doesn't set it).
    pub gutter_bg: Color,
    /// Block / bar cursor background.
    pub cursor_bg: Color,
    /// Text over the cursor block.
    pub cursor_fg: Color,
    /// Selection highlight background.
    pub selection_bg: Color,
    /// Text over the selection highlight.
    pub selection_fg: Color,
    pub divider: Color,
    /// Statusline / bar text (defaults to `fg`).
    pub statusline_fg: Color,
    /// Statusline / bar background (defaults to `divider`).
    pub statusline_bg: Color,
    /// Statusline background in Normal mode (defaults to `statusline_bg`).
    pub mode_normal_bg: Color,
    /// Statusline text in Normal mode (defaults to `statusline_fg`).
    pub mode_normal_fg: Color,
    /// Statusline background in Insert mode (defaults to `statusline_bg`).
    pub mode_insert_bg: Color,
    /// Statusline text in Insert mode (defaults to `statusline_fg`).
    pub mode_insert_fg: Color,
    /// Statusline background in Visual mode (defaults to `statusline_bg`).
    pub mode_visual_bg: Color,
    /// Statusline text in Visual mode (defaults to `statusline_fg`).
    pub mode_visual_fg: Color,
    /// Statusline background in Cmdline mode (defaults to `statusline_bg`).
    pub mode_cmdline_bg: Color,
    /// Statusline text in Cmdline mode (defaults to `statusline_fg`).
    pub mode_cmdline_fg: Color,
    /// Statusline background in Emacs mode (defaults to `statusline_bg`).
    pub mode_emacs_bg: Color,
    /// Statusline text in Emacs mode (defaults to `statusline_fg`).
    pub mode_emacs_fg: Color,
    pub accent: Color,
    /// Text on accent-colored bars (defaults to `bg`).
    pub accent_fg: Color,
    /// Which-key panel background.
    pub whichkey_bg: Color,
    /// Which-key panel text.
    pub whichkey_fg: Color,
    /// Cmdline / mini-buffer background.
    pub cmdline_bg: Color,
    /// Cmdline / mini-buffer text.
    pub cmdline_fg: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            bg: Color::Rgb(30, 30, 30),
            fg: Color::Rgb(205, 214, 244),
            gutter: Color::Rgb(108, 112, 134),
            gutter_bg: Color::Rgb(30, 30, 30),
            cursor_bg: Color::Rgb(245, 224, 220),
            cursor_fg: Color::Rgb(30, 30, 30),
            selection_bg: Color::Rgb(88, 91, 112),
            selection_fg: Color::Rgb(205, 214, 244),
            divider: Color::Rgb(69, 71, 90),
            statusline_fg: Color::Rgb(205, 214, 244),
            statusline_bg: Color::Rgb(69, 71, 90),
            mode_normal_bg: Color::Rgb(69, 71, 90),
            mode_normal_fg: Color::Rgb(205, 214, 244),
            mode_insert_bg: Color::Rgb(40, 72, 50),
            mode_insert_fg: Color::Rgb(205, 214, 244),
            mode_visual_bg: Color::Rgb(72, 50, 80),
            mode_visual_fg: Color::Rgb(205, 214, 244),
            mode_cmdline_bg: Color::Rgb(60, 55, 40),
            mode_cmdline_fg: Color::Rgb(205, 214, 244),
            mode_emacs_bg: Color::Rgb(50, 50, 70),
            mode_emacs_fg: Color::Rgb(205, 214, 244),
            accent: Color::Rgb(243, 139, 168),
            accent_fg: Color::Rgb(30, 30, 30),
            whichkey_bg: Color::Rgb(30, 30, 46),
            whichkey_fg: Color::Rgb(205, 214, 244),
            cmdline_bg: Color::Rgb(30, 30, 30),
            cmdline_fg: Color::Rgb(205, 214, 244),
        }
    }
}

impl Theme {
    pub fn mode_bg(&self, mode: UIMode) -> Color {
        match mode {
            UIMode::Normal => self.mode_normal_bg,
            UIMode::Insert => self.mode_insert_bg,
            UIMode::Visual => self.mode_visual_bg,
            UIMode::Cmdline => self.mode_cmdline_bg,
            UIMode::Emacs => self.mode_emacs_bg,
        }
    }

    pub fn mode_fg(&self, mode: UIMode) -> Color {
        match mode {
            UIMode::Normal => self.mode_normal_fg,
            UIMode::Insert => self.mode_insert_fg,
            UIMode::Visual => self.mode_visual_fg,
            UIMode::Cmdline => self.mode_cmdline_fg,
            UIMode::Emacs => self.mode_emacs_fg,
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

impl SyntaxStyle {
    pub fn error() -> Self {
        SyntaxStyle { fg: Color::Rgb(243, 139, 168), bg: Color::Default, bold: false, italic: false }
    }
    pub fn warning() -> Self {
        SyntaxStyle { fg: Color::Rgb(249, 226, 175), bg: Color::Default, bold: false, italic: false }
    }
    pub fn info() -> Self {
        SyntaxStyle { fg: Color::Rgb(137, 180, 250), bg: Color::Default, bold: false, italic: false }
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

/// A sign column drawn to the **left of the line-number gutter** — one glyph per
/// buffer line, used for diagnostics, test results and (later) DAP breakpoints.
/// `width` is the reserved cell width (0 when there are no signs), and each sign
/// is `(buffer_line, glyph, color)`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SignsView {
    pub width: u16,
    pub signs: Vec<(u16, char, Color)>,
}

impl SignsView {
    /// The sign (glyph, color) for a buffer line, if any. Later signs win, so a
    /// higher-severity sign pushed last overrides a lower one on the same line.
    pub fn at(&self, line: u16) -> Option<(char, Color)> {
        self.signs.iter().rev().find(|(l, _, _)| *l == line).map(|(_, g, c)| (*g, *c))
    }
}

/// Where a window's buffer text actually starts, and how much room it has.
///
/// A window's rect covers a header row, the text rows, and a statusline row;
/// within the text area, columns are laid out sign column, then line-number
/// gutter, then text. Every backend must agree on this, and so must mouse
/// hit-testing — computing it by hand at each site is what previously put flash
/// labels and click targets in the wrong column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextArea {
    /// First column of buffer text (past the sign column and gutter).
    pub x: u16,
    /// First row of buffer text (past the header row).
    pub y: u16,
    /// Text columns available after the sign column and gutter.
    pub width: u16,
    /// Text rows available between the header and the statusline.
    pub height: u16,
}

impl TextArea {
    /// Derive the text area of a window from its rect and column widths.
    /// `sign_width`/`gutter_width` are clamped to the rect, so an oversized
    /// gutter yields a zero-width text area rather than an out-of-bounds origin.
    pub fn of(rect: Rect, sign_width: u16, gutter_width: u16) -> Self {
        let sign_w = sign_width.min(rect.width);
        let gutter_w = gutter_width.min(rect.width - sign_w);
        TextArea {
            x: rect.x + sign_w + gutter_w,
            // One header row above, one statusline row below.
            y: rect.y + 1,
            width: rect.width - sign_w - gutter_w,
            height: rect.height.saturating_sub(2),
        }
    }

    /// One past the last text column.
    pub fn right(&self) -> u16 {
        self.x + self.width
    }

    /// The (row, column) within the text area for a screen cell, or `None` when
    /// the cell is outside it (in the header, statusline, sign column or gutter).
    pub fn cell_at(&self, screen_x: u16, screen_y: u16) -> Option<(u16, u16)> {
        if screen_x < self.x
            || screen_x >= self.right()
            || screen_y < self.y
            || screen_y >= self.y + self.height
        {
            return None;
        }
        Some((screen_y - self.y, screen_x - self.x))
    }
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
    pub mode: UIMode,
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

/// A single flash jump label to render at a screen position.
#[derive(Debug, Clone)]
pub struct FlashLabelRender {
    pub row: u16,
    pub col: u16,
    pub text: String,
    pub color: Color,
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
    /// Sign column drawn left of the gutter (diagnostics, test results, …).
    pub signs: SignsView,
    pub statusline: StatuslineView,
    pub active: bool,
    /// Visual-mode selection to highlight (active window only).
    pub selection: Option<SelectionView>,
    /// When set, this window is a terminal: draw the grid instead of `lines`.
    pub terminal: Option<TermGridView>,
    /// Panel header label shown in the window's top chrome row (e.g. filename).
    pub header: String,
    /// Flash jump overlay labels rendered on top of the buffer text.
    pub flash_labels: Vec<FlashLabelRender>,
}

/// One row of a floating picker overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerRow {
    pub label: String,
    pub selected: bool,
}

/// Where a picker overlay is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PickerPlacement {
    /// A floating box centered over the window frame (the default).
    #[default]
    Center,
    /// A full-width strip docked to the bottom, like the which-key panel — used
    /// for the `:`-Tab command palette when configured.
    Bottom,
}

/// A floating fuzzy-list overlay (buffer list, file finder, which-key, ...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerView {
    pub title: String,
    pub query: String,
    pub rows: Vec<PickerRow>,
    /// Syntax-highlighted preview of the selected entry (empty = no preview pane).
    pub preview: Vec<StyledLine>,
    /// Center (floating) or bottom (docked) placement.
    pub placement: PickerPlacement,
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

/// Scroll a list so `selected` stays visible **without recentering**: the view
/// holds its position until the selection crosses an edge, then scrolls just
/// enough to keep it on-screen — the behavior of a normal list widget. `prev`
/// is the last frame's top line; returns the new top line.
pub fn settings_scroll(prev: usize, selected: usize, viewport: usize, total: usize) -> usize {
    let viewport = viewport.max(1);
    let mut top = prev;
    if selected < top {
        top = selected;
    } else if selected >= top + viewport {
        top = selected + 1 - viewport;
    }
    let max = total.saturating_sub(viewport);
    top.min(max)
}

/// The welcome/start screen ("Dashboard"), shown when no file is open. Rendered
/// as a centered panel in the first window when present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WelcomeView {
    pub visible: bool,
    pub recent_projects: Vec<String>,
    pub version: String,
    pub lsp_status: String,
    pub edit_mode: String,
}

impl Default for WelcomeView {
    fn default() -> Self {
        WelcomeView {
            visible: false,
            recent_projects: Vec::new(),
            version: String::new(),
            lsp_status: "● Ready".into(),
            edit_mode: "Neovim".into(),
        }
    }
}

/// A full frame: every visible window, the shared cmdline/message line, an
/// optional centered picker overlay, optional bottom which-key panel, and an
/// optional welcome screen.
pub struct FrameState<'a> {
    pub windows: Vec<WindowView>,
    pub cmdline: Option<&'a str>,
    /// Mini toast overlay lines (one per visible notification).
    pub noice_mini: Vec<String>,
    /// Notify stacking panel view, if visible.
    pub noice_notify: Option<Vec<StyledLine>>,
    pub picker: Option<PickerView>,
    pub whichkey: Option<WhichKeyView>,
    /// LSP hover popup lines (syntax-highlighted), in a floating box near the top.
    pub hover: Option<Vec<StyledLine>>,
    /// The settings page overlay, when open.
    pub settings: Option<SettingsView>,
    /// Welcome/start screen ("Dashboard"), shown when no file is open.
    pub welcome: Option<WelcomeView>,
    /// The theme palette for this frame. Used by both the TUI and GUI backends
    /// so that every widget and overlay reads a consistent set of colours.
    pub theme: Theme,
    /// Debugger overlay (toolbar + stack/variables), shown while a debug
    /// session is active.
    pub debug_overlay: Option<DebugOverlayView>,
}

/// Debugger overlay rendered above the window area.
pub struct DebugOverlayView {
    /// Toolbar row: status + action hints.
    pub toolbar: String,
    /// Stack frames: (depth, name, file:line) tuples.
    pub stack: Vec<(u16, String, String)>,
    /// Visible scopes and their variables.
    pub scopes: Vec<(String, Vec<(String, String)>)>,
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
        SignsView, StatuslineView, StyledLine, TermCellView, TermGridView, Theme, UIMode,
        WindowView,
    };

    struct TestRenderer;
    impl Renderer for TestRenderer {
        fn render_frame(&mut self, _state: &FrameState) {}
    }

    #[test]
    fn text_area_skips_header_statusline_and_columns() {
        use crate::TextArea;
        // 80x24 window, 1-wide sign column, 4-wide number gutter.
        let t = TextArea::of(Rect::new(0, 0, 80, 24), 1, 4);
        assert_eq!(t.x, 5, "past the sign column and gutter");
        assert_eq!(t.y, 1, "past the header row");
        assert_eq!(t.width, 75);
        assert_eq!(t.height, 22, "header and statusline excluded");
        assert_eq!(t.right(), 80);
    }

    #[test]
    fn text_area_cell_at_rejects_cells_outside_the_text() {
        use crate::TextArea;
        let t = TextArea::of(Rect::new(10, 0, 40, 10), 0, 3);
        // Origin maps to row 0, col 0.
        assert_eq!(t.cell_at(13, 1), Some((0, 0)));
        assert_eq!(t.cell_at(15, 3), Some((2, 2)));
        // Header row, gutter column, statusline row, and the next split over.
        assert_eq!(t.cell_at(13, 0), None, "header");
        assert_eq!(t.cell_at(12, 1), None, "gutter");
        assert_eq!(t.cell_at(13, 9), None, "statusline");
        assert_eq!(t.cell_at(50, 1), None, "past the right edge");
    }

    #[test]
    fn text_area_survives_columns_wider_than_the_window() {
        use crate::TextArea;
        // A gutter wider than the window must not push the origin out of bounds.
        let t = TextArea::of(Rect::new(0, 0, 4, 5), 2, 99);
        assert_eq!(t.width, 0);
        assert_eq!(t.x, 4, "clamped to the window's right edge");
        assert_eq!(t.cell_at(0, 1), None);
    }

    #[test]
    fn settings_scroll_holds_until_edge_then_follows() {
        use crate::settings_scroll;
        // 20 items, viewport 5. Moving down within view doesn't scroll.
        assert_eq!(settings_scroll(0, 3, 5, 20), 0);
        // Crossing the bottom edge scrolls just enough to keep it visible.
        assert_eq!(settings_scroll(0, 5, 5, 20), 1);
        assert_eq!(settings_scroll(0, 6, 5, 20), 2);
        // Moving back up while still on-screen holds position (no re-center).
        assert_eq!(settings_scroll(2, 4, 5, 20), 2);
        // Crossing the top edge scrolls up.
        assert_eq!(settings_scroll(2, 1, 5, 20), 1);
        // Never scrolls past the end (max top = total - viewport).
        assert_eq!(settings_scroll(99, 19, 5, 20), 15);
        // Fewer items than the viewport: no scroll.
        assert_eq!(settings_scroll(3, 0, 5, 3), 0);
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
            signs: SignsView::default(),
            statusline: StatuslineView {
                left: "NORMAL".into(),
                center: "test.txt".into(),
                right: "1,1".into(),
                active: true,
                mode: UIMode::Normal,
            },
            active: true,
            selection: None,
            terminal: None,
            header: "test.txt".into(),
            flash_labels: Vec::new(),
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
            noice_mini: vec![],
            noice_notify: None,
            picker: None,
            whichkey: None,
            hover: None,
            settings: None,
            welcome: None,
            theme: Theme::default(),
            debug_overlay: None,
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
