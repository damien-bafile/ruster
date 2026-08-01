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

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub enum CursorKind { #[default] Block, Bar }

/// A rectangle in cell coordinates (origin top-left). Mirrors
/// `ruster_core::windows::Rect`; kept local so this crate stays dependency-free.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
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
///
/// `Default` exists so construction sites can fill in only what they care about
/// with `..Default::default()`; adding a field here otherwise means editing every
/// literal, which is how `flash_labels` came to be missing from some of them.
#[derive(Default)]
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
    /// An action, not a value: activating it submits the dialog it sits in.
    /// Only dialogs use this — the settings page has no buttons.
    Button,
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
///
/// Shared by the settings page and the picker; a wrapping selection relies on
/// it to jump the view to the far end rather than leaving the cursor off-screen.
pub fn list_scroll(prev: usize, selected: usize, viewport: usize, total: usize) -> usize {
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
///
/// `Default` gives an empty frame, so callers set only the fields they exercise.
#[derive(Default)]
pub struct FrameState<'a> {
    pub windows: Vec<WindowView>,
    pub cmdline: Option<&'a str>,
    /// Mini toast overlay lines (one per visible notification).
    pub noice_mini: Vec<String>,
    /// Notify stacking panel view, if visible.
    pub noice_notify: Option<Vec<StyledLine>>,
    pub picker: Option<PickerView>,
    pub whichkey: Option<WhichKeyView>,
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
    /// Floating boxes drawn above the window views, lowest `z` first.
    pub floats: Vec<FloatView>,
    /// A modal form, drawn above everything else.
    pub dialog: Option<DialogView>,
}

/// Where a float wants to sit. Resolved against the frame area by
/// [`FloatView::anchored`], which also clamps it on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatAnchor {
    /// Centred in the frame.
    Center,
    /// Just below the given cell, flipping above it when there is no room —
    /// how a completion or signature popup follows the caret.
    Cursor { col: u16, row: u16 },
    /// Pinned to a corner or edge, inset by one cell.
    Edge(FloatEdge),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatEdge {
    Top,
    Bottom,
    TopRight,
    BottomRight,
}

/// A floating box above the window views: popups, modals, confirmations.
///
/// The rect is resolved and clamped when the float is built, so a backend only
/// has to draw it. Keeping the geometry here means one implementation, unit
/// tested, rather than one per backend that can drift.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FloatView {
    pub rect: Rect,
    pub lines: Vec<StyledLine>,
    pub title: Option<String>,
    pub border: bool,
    /// Draw order among floats; higher sits on top. Ties keep insertion order.
    pub z: i32,
}

impl FloatView {
    /// Size to `lines`, place per `anchor`, and clamp inside `area`.
    ///
    /// A bordered float reserves one cell on each side, so its content width is
    /// `rect.width - 2`. When `area` is too small to hold the content the box is
    /// truncated rather than pushed off screen.
    pub fn anchored(area: Rect, anchor: FloatAnchor, lines: Vec<StyledLine>) -> Self {
        Self::anchored_titled(area, anchor, lines, None)
    }

    pub fn anchored_titled(
        area: Rect,
        anchor: FloatAnchor,
        lines: Vec<StyledLine>,
        title: Option<String>,
    ) -> Self {
        let pad = 2; // border on both sides
        let content_w = lines.iter().map(|l| l.text.chars().count()).max().unwrap_or(0);
        let mut w = (content_w as u16).saturating_add(pad);
        // The title sits on the top border, inset two cells from the left, so it
        // needs its length plus that inset plus the closing corner — otherwise a
        // title longer than the content is clipped by its own box.
        if let Some(t) = &title {
            w = w.max(t.chars().count() as u16 + 3);
        }
        let w = w.clamp(pad, area.width.max(pad));
        let h = (lines.len() as u16 + pad).clamp(pad, area.height.max(pad));

        let (x, y) = match anchor {
            FloatAnchor::Center => (
                area.x + (area.width.saturating_sub(w)) / 2,
                area.y + (area.height.saturating_sub(h)) / 2,
            ),
            FloatAnchor::Cursor { col, row } => {
                // Prefer below the cell; flip above when the bottom would clip.
                let below = row.saturating_add(1);
                let y = if below + h <= area.y + area.height {
                    below
                } else {
                    row.saturating_sub(h)
                };
                (col, y)
            }
            FloatAnchor::Edge(edge) => {
                let right = (area.x + area.width).saturating_sub(w + 1);
                let bottom = (area.y + area.height).saturating_sub(h + 1);
                match edge {
                    FloatEdge::Top => (area.x + (area.width.saturating_sub(w)) / 2, area.y + 1),
                    FloatEdge::Bottom => (area.x + (area.width.saturating_sub(w)) / 2, bottom),
                    FloatEdge::TopRight => (right, area.y + 1),
                    FloatEdge::BottomRight => (right, bottom),
                }
            }
        };

        // Clamp last, so every anchor lands on screen.
        let max_x = (area.x + area.width).saturating_sub(w);
        let max_y = (area.y + area.height).saturating_sub(h);
        let x = x.clamp(area.x, max_x.max(area.x));
        let y = y.clamp(area.y, max_y.max(area.y));

        FloatView { rect: Rect::new(x, y, w, h), lines, title, border: true, z: 0 }
    }

    pub fn with_z(mut self, z: i32) -> Self {
        self.z = z;
        self
    }

    /// The inner area available to content, excluding the border.
    pub fn inner(&self) -> Rect {
        if self.border {
            Rect::new(
                self.rect.x + 1,
                self.rect.y + 1,
                self.rect.width.saturating_sub(2),
                self.rect.height.saturating_sub(2),
            )
        } else {
            self.rect
        }
    }
}

/// Sort floats into draw order: ascending `z`, stable so ties keep the order
/// they were pushed in.
pub fn floats_in_draw_order(floats: &[FloatView]) -> Vec<&FloatView> {
    let mut out: Vec<&FloatView> = floats.iter().collect();
    out.sort_by_key(|f| f.z);
    out
}

/// How a setting row's value is shown: `[x] on`, `< enum >`, or the raw text
/// with a caret while editing.
///
/// Lives here, beside the type it formats, because both backends draw these
/// rows and each previously kept an identical private copy — the sort of
/// duplication that silently drifts.
pub fn control_display(row: &SettingRowView) -> String {
    match row.kind {
        ControlKind::Toggle => {
            if row.value == "on" { "[x] on".to_string() } else { "[ ] off".to_string() }
        }
        ControlKind::Enum => format!("< {} >", row.value),
        ControlKind::Button => format!("[ {} ]", row.label),
        ControlKind::Number | ControlKind::Text => {
            if row.editing {
                format!("{}▏", row.value)
            } else {
                row.value.clone()
            }
        }
    }
}

/// A modal form: a title, rows built from the same [`SettingRowView`] the
/// settings page uses, and a footer of key hints.
///
/// Deliberately the settings page's vocabulary rather than a new widget set —
/// both backends already draw these rows, and a widget crate could only serve
/// the ratatui half of the editor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DialogView {
    pub title: String,
    pub rows: Vec<SettingRowView>,
    pub footer: String,
}

/// Debugger overlay rendered above the window area.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DebugOverlayView {
    /// Toolbar row: status + action hints.
    pub toolbar: String,
    /// Stack frames: (depth, name, file:line) tuples.
    pub stack: Vec<(u16, String, String)>,
    /// Visible scopes and their variables.
    pub scopes: Vec<(String, Vec<(String, String)>)>,
}

impl DebugOverlayView {
    /// The overlay body as display rows: the call stack, then each scope with
    /// its variables. Both backends draw the same text, so the flattening lives
    /// here instead of being reimplemented per renderer.
    ///
    /// The toolbar is not included — it is drawn as a highlighted bar.
    pub fn rows(&self) -> Vec<String> {
        let mut rows = Vec::new();
        if !self.stack.is_empty() {
            rows.push("Call stack".to_string());
            for (depth, name, loc) in &self.stack {
                rows.push(format!("{:>2}  {}  {}", depth, name, loc));
            }
        }
        for (scope, vars) in &self.scopes {
            if !rows.is_empty() {
                rows.push(String::new());
            }
            rows.push(scope.clone());
            for (name, value) in vars {
                rows.push(format!("  {} = {}", name, value));
            }
        }
        if rows.is_empty() {
            rows.push("(no frames)".to_string());
        }
        rows
    }
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
        floats_in_draw_order, Color, FloatAnchor, FloatEdge, FloatView, FrameState, Rect,
        Renderer, SelectionKind, SelectionView, StatuslineView, StyledLine, TermCellView,
        TermGridView, UIMode, WindowView,
    };

    struct TestRenderer;
    impl Renderer for TestRenderer {
        fn render_frame(&mut self, _state: &FrameState) {}
    }

    #[test]
    fn debug_overlay_rows_lists_stack_then_scopes() {
        use crate::DebugOverlayView;
        let v = DebugOverlayView {
            toolbar: "[Debug: PAUSED]".into(),
            stack: vec![(0, "main".into(), "src/main.rs:12".into())],
            scopes: vec![("Locals".into(), vec![("x".into(), "1".into())])],
        };
        let rows = v.rows();
        assert_eq!(rows[0], "Call stack");
        assert!(rows[1].contains("main") && rows[1].contains("src/main.rs:12"));
        // A blank spacer separates the stack from the first scope.
        assert_eq!(rows[2], "");
        assert_eq!(rows[3], "Locals");
        assert_eq!(rows[4], "  x = 1");
        // The toolbar is drawn as a bar, not as a row.
        assert!(!rows.iter().any(|r| r.contains("Debug: PAUSED")));
    }

    #[test]
    fn debug_overlay_rows_never_empty() {
        use crate::DebugOverlayView;
        // A session that has started but not stopped yet has no frames; the
        // panel still needs something to draw.
        assert_eq!(DebugOverlayView::default().rows(), vec!["(no frames)".to_string()]);
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
    fn list_scroll_holds_until_edge_then_follows() {
        use crate::list_scroll;
        // 20 items, viewport 5. Moving down within view doesn't scroll.
        assert_eq!(list_scroll(0, 3, 5, 20), 0);
        // Crossing the bottom edge scrolls just enough to keep it visible.
        assert_eq!(list_scroll(0, 5, 5, 20), 1);
        assert_eq!(list_scroll(0, 6, 5, 20), 2);
        // Moving back up while still on-screen holds position (no re-center).
        assert_eq!(list_scroll(2, 4, 5, 20), 2);
        // Crossing the top edge scrolls up.
        assert_eq!(list_scroll(2, 1, 5, 20), 1);
        // Never scrolls past the end (max top = total - viewport).
        assert_eq!(list_scroll(99, 19, 5, 20), 15);
        // Fewer items than the viewport: no scroll.
        assert_eq!(list_scroll(3, 0, 5, 3), 0);
    }

    /// A picker wraps its selection from the first item to the last. The view
    /// has to follow, or the cursor lands off-screen and the list looks stuck
    /// at the top.
    #[test]
    fn list_scroll_follows_a_selection_that_wraps_to_the_end() {
        use crate::list_scroll;
        // 40 items, 10 visible, sitting at the top; selection wraps to the last.
        let top = list_scroll(0, 39, 10, 40);
        assert_eq!(top, 30, "scrolled so item 39 is the last visible row");
        assert!(39 >= top && 39 < top + 10, "selection is on screen");
    }

    #[test]
    fn list_scroll_follows_a_selection_that_wraps_back_to_the_start() {
        use crate::list_scroll;
        let top = list_scroll(30, 0, 10, 40);
        assert_eq!(top, 0, "jumped back to the top with the selection");
    }

    fn sample_window() -> WindowView {
        WindowView {
            rect: Rect::new(0, 0, 80, 24),
            lines: vec![StyledLine { text: "hello".to_string(), highlights: vec![] }],
            cursor_visible: true,
            statusline: StatuslineView {
                left: "NORMAL".into(),
                center: "test.txt".into(),
                right: "1,1".into(),
                active: true,
                mode: UIMode::Normal,
            },
            active: true,
            header: "test.txt".into(),
            ..Default::default()
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
        let state = FrameState { windows: vec![sample_window()], ..Default::default() };
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

    fn sl(t: &str) -> StyledLine {
        StyledLine { text: t.to_string(), highlights: vec![] }
    }

    const AREA: Rect = Rect { x: 0, y: 0, width: 80, height: 24 };

    #[test]
    fn float_sizes_to_its_content_plus_border() {
        let f = FloatView::anchored(AREA, FloatAnchor::Center, vec![sl("hello"), sl("hi")]);
        assert_eq!(f.rect.width, 7, "widest line + 2 for the border");
        assert_eq!(f.rect.height, 4, "2 lines + 2 for the border");
        assert_eq!(f.inner().width, 5);
        assert_eq!(f.inner().height, 2);
    }

    #[test]
    fn a_title_widens_the_box_when_it_is_longer_than_the_content() {
        let f = FloatView::anchored_titled(
            AREA,
            FloatAnchor::Center,
            vec![sl("ok")],
            Some("a much longer title".to_string()),
        );
        assert!(f.rect.width >= "a much longer title".len() as u16 + 2);
    }

    #[test]
    fn centered_float_is_centered() {
        let f = FloatView::anchored(AREA, FloatAnchor::Center, vec![sl("12345678")]);
        assert_eq!(f.rect.x, (80 - f.rect.width) / 2);
        assert_eq!(f.rect.y, (24 - f.rect.height) / 2);
    }

    #[test]
    fn cursor_float_sits_below_the_cell_when_there_is_room() {
        let f = FloatView::anchored(AREA, FloatAnchor::Cursor { col: 10, row: 5 }, vec![sl("x")]);
        assert_eq!(f.rect.x, 10);
        assert_eq!(f.rect.y, 6, "one row below the cursor");
    }

    #[test]
    fn cursor_float_flips_above_when_it_would_clip_the_bottom() {
        // Row 22 of a 24-row area: a 3-row box below would run off.
        let f = FloatView::anchored(
            AREA,
            FloatAnchor::Cursor { col: 4, row: 22 },
            vec![sl("a"), sl("b")],
        );
        assert!(f.rect.y < 22, "flipped above the cursor, got y={}", f.rect.y);
        assert!(f.rect.y + f.rect.height <= 24);
    }

    /// Every anchor must land fully inside the area — the clamp the plan calls
    /// for, checked at each edge.
    ///
    /// Deliberately uses an area that does **not** start at the origin, so the
    /// left/top assertions are real rather than trivially true for `u16`, and so
    /// anything assuming a `0,0` origin is caught.
    #[test]
    fn floats_are_clamped_inside_the_area_at_every_edge() {
        const OFF: Rect = Rect { x: 7, y: 3, width: 60, height: 20 };
        let wide = vec![sl(&"x".repeat(200))];
        let tall: Vec<StyledLine> = (0..100).map(|i| sl(&format!("line {i}"))).collect();
        let anchors = [
            FloatAnchor::Center,
            FloatAnchor::Cursor { col: OFF.x + OFF.width - 1, row: OFF.y + OFF.height - 1 },
            FloatAnchor::Cursor { col: 0, row: 0 },
            FloatAnchor::Cursor { col: 200, row: 200 },
            FloatAnchor::Edge(FloatEdge::Top),
            FloatAnchor::Edge(FloatEdge::Bottom),
            FloatAnchor::Edge(FloatEdge::TopRight),
            FloatAnchor::Edge(FloatEdge::BottomRight),
        ];
        for a in anchors {
            for lines in [vec![sl("ok")], wide.clone(), tall.clone()] {
                let f = FloatView::anchored(OFF, a, lines);
                assert!(f.rect.x >= OFF.x, "{a:?} left edge: x={}", f.rect.x);
                assert!(f.rect.y >= OFF.y, "{a:?} top edge: y={}", f.rect.y);
                assert!(
                    f.rect.x + f.rect.width <= OFF.x + OFF.width,
                    "{a:?} right edge: x={} w={}",
                    f.rect.x, f.rect.width
                );
                assert!(
                    f.rect.y + f.rect.height <= OFF.y + OFF.height,
                    "{a:?} bottom edge: y={} h={}",
                    f.rect.y, f.rect.height
                );
            }
        }
    }

    #[test]
    fn content_wider_than_the_screen_is_truncated_not_pushed_off() {
        let f = FloatView::anchored(AREA, FloatAnchor::Center, vec![sl(&"y".repeat(500))]);
        assert_eq!(f.rect.width, 80);
        assert_eq!(f.rect.x, 0);
    }

    #[test]
    fn a_tiny_area_still_produces_an_on_screen_rect() {
        let tiny = Rect::new(0, 0, 1, 1);
        let f = FloatView::anchored(tiny, FloatAnchor::Center, vec![sl("wide content")]);
        assert_eq!(f.rect.x, 0);
        assert_eq!(f.rect.y, 0);
    }

    #[test]
    fn draw_order_is_by_z_and_stable_within_a_z() {
        let mk = |name: &str, z: i32| {
            FloatView::anchored(AREA, FloatAnchor::Center, vec![sl(name)]).with_z(z)
        };
        let floats = vec![mk("top", 10), mk("first", 0), mk("second", 0), mk("mid", 5)];
        let order: Vec<&str> = floats_in_draw_order(&floats)
            .iter()
            .map(|f| f.lines[0].text.as_str())
            .collect();
        assert_eq!(order, ["first", "second", "mid", "top"]);
    }

    #[test]
    fn frame_state_defaults_to_no_floats() {
        let st = FrameState::default();
        assert!(st.floats.is_empty());
    }
}
