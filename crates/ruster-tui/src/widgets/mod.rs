pub mod debug_overlay;
pub mod noice_toast;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use ratatui::widgets::Widget;
use ruster_core::vim::VimMode;
use ruster_render::{
    Color as RColor, CursorKind, GutterView, SettingRowView, SettingsView, StatuslineView,
    StyledLine, TermGridView, WelcomeView,
};

/// Draw a bordered overlay box whose **top edge carries the title**:
/// `╭─ Settings ─────╮`, sides down each edge, `╰────╯` along the bottom.
///
/// Used by the floating overlays (picker, settings). Tiled buffer windows use
/// [`ruled_header`] alone — they sit flush against each other, so they have no
/// sides of their own to join.
pub(crate) fn titled_box(
    buf: &mut Buffer,
    area: Rect,
    label: &str,
    label_fg: Color,
    rule_fg: Color,
    bg: Color,
) {
    if area.width < 2 || area.height < 2 {
        return;
    }
    let (l, r, t, b) = (area.left(), area.right() - 1, area.top(), area.bottom() - 1);
    let mut put = |x: u16, y: u16, ch: char, fg: Color| {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_char(ch);
            cell.set_fg(fg);
            cell.set_bg(bg);
        }
    };
    // Top edge, with the title inset so the corners stay intact.
    let hdr: Vec<char> = format!("─ {} ─", label).chars().collect();
    for x in l..=r {
        let i = (x - l) as usize;
        let (ch, fg) = match hdr.get(i.wrapping_sub(1)) {
            Some(&c) if x > l && x < r => (c, label_fg),
            _ => ('─', rule_fg),
        };
        put(x, t, ch, fg);
    }
    put(l, t, '╭', rule_fg);
    put(r, t, '╮', rule_fg);
    for y in (t + 1)..b {
        put(l, y, '│', rule_fg);
        put(r, y, '│', rule_fg);
    }
    for x in l..=r {
        put(x, b, '─', rule_fg);
    }
    put(l, b, '╰', rule_fg);
    put(r, b, '╯', rule_fg);
}

/// Draw the standard panel header — `─ label ─` then ruled to the full width.
///
/// Every titled surface uses this: buffer windows, the picker, the settings
/// page. Keeping one implementation is the point — the alternative is each
/// overlay inventing its own title bar, which is how the picker ended up with a
/// filled accent strip while the window beside it had a ruled line.
pub(crate) fn ruled_header(
    buf: &mut Buffer,
    area: Rect,
    label: &str,
    label_fg: Color,
    rule_fg: Color,
    bg: Color,
) {
    let hdr = format!("─ {} ─", label);
    let mut x = area.x;
    for ch in hdr.chars() {
        if x >= area.right() {
            break;
        }
        if let Some(cell) = buf.cell_mut((x, area.y)) {
            cell.set_char(ch);
            cell.set_fg(label_fg);
            cell.set_bg(bg);
        }
        x += 1;
    }
    for fill in x..area.right() {
        if let Some(cell) = buf.cell_mut((fill, area.y)) {
            cell.set_char('─');
            cell.set_fg(rule_fg);
            cell.set_bg(bg);
        }
    }
}

/// Convert a VimMode to a display string.
pub fn mode_label(mode: &VimMode) -> &'static str {
    match mode {
        VimMode::Normal => "-- NORMAL --",
        VimMode::Insert => "-- INSERT --",
        VimMode::VisualChar => "-- VISUAL --",
        VimMode::VisualLine => "-- V-LINE --",
        VimMode::VisualBlock => "-- V-BLOCK --",
        VimMode::Cmdline => "-- CMDLINE --",
    }
}

/// Format the cmdline text for display (always starts with ":").
pub fn cmdline_label(buf: &str) -> String {
    if buf.is_empty() {
        ":".to_string()
    } else {
        buf.to_string()
    }
}

fn apply_cursor(cell: &mut ratatui::buffer::Cell, kind: CursorKind, theme: &ruster_render::Theme) {
    let bg = ruster_render_color_to_tui(&theme.cursor_bg);
    let fg = ruster_render_color_to_tui(&theme.cursor_fg);
    match kind {
        CursorKind::Bar | CursorKind::Block => {
            cell.set_bg(bg);
            cell.set_fg(fg);
        }
    }
}

fn apply_dim_cursor(
    cell: &mut ratatui::buffer::Cell,
    kind: CursorKind,
    theme: &ruster_render::Theme,
) {
    let bg = ruster_render_color_to_tui(&theme.cursor_bg);
    let fg = ruster_render_color_to_tui(&theme.cursor_fg);
    match kind {
        CursorKind::Bar | CursorKind::Block => {
            cell.set_bg(bg);
            cell.set_fg(fg);
        }
    }
}

fn ruster_render_color_to_tui(c: &RColor) -> Color {
    match c {
        RColor::Default => Color::Reset,
        RColor::Rgb(r, g, b) => Color::Rgb(*r, *g, *b),
    }
}

/// Draws an embedded terminal's grid (from `WindowView.terminal`) cell-by-cell,
/// with per-cell colors/attributes and a block cursor.
pub struct TerminalWidget {
    grid: TermGridView,
    cursor_visible: bool,
    _theme: Option<ruster_render::Theme>,
}

impl TerminalWidget {
    pub fn new(grid: TermGridView) -> Self {
        TerminalWidget {
            grid,
            cursor_visible: true,
            _theme: None,
        }
    }

    pub fn with_cursor_visible(mut self, visible: bool) -> Self {
        self.cursor_visible = visible;
        self
    }

    pub fn with_theme(mut self, theme: &ruster_render::Theme) -> Self {
        self._theme = Some(*theme);
        self
    }
}

impl Widget for TerminalWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let rows = self.grid.rows.min(area.height as usize);
        let cols = self.grid.cols.min(area.width as usize);
        for r in 0..rows {
            for c in 0..cols {
                let tc = self.grid.cells[r * self.grid.cols + c];
                let x = area.x + c as u16;
                let y = area.y + r as u16;
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(tc.c);
                    let mut fg = ruster_render_color_to_tui(&tc.fg);
                    let mut bg = ruster_render_color_to_tui(&tc.bg);
                    if tc.inverse {
                        std::mem::swap(&mut fg, &mut bg);
                    }
                    cell.set_fg(fg);
                    cell.set_bg(bg);
                    if tc.bold {
                        cell.modifier.insert(Modifier::BOLD);
                    }
                    if tc.italic {
                        cell.modifier.insert(Modifier::ITALIC);
                    }
                    if tc.underline {
                        cell.modifier.insert(Modifier::UNDERLINED);
                    }
                }
            }
        }
        if self.cursor_visible {
            let (cr, cc) = self.grid.cursor;
            if cr < rows && cc < cols {
                if let Some(cell) = buf.cell_mut((area.x + cc as u16, area.y + cr as u16)) {
                    let default_theme = ruster_render::Theme::default();
                    let t = self._theme.as_ref().unwrap_or(&default_theme);
                    apply_cursor(cell, CursorKind::Block, t);
                }
            }
        }
    }
}

/// Renders buffer text with cursor highlight and optional syntax highlighting.
///
/// Text is drawn starting from `scroll_offset` (the first visible buffer line)
/// and offset horizontally by the gutter width; the gutter's line-number column
/// is drawn on the left.
pub struct BufferWidget {
    lines: Vec<StyledLine>,
    cursor: (u16, u16),
    extra_cursors: Vec<(u16, u16)>,
    syntax: bool,
    cursor_visible: bool,
    cursor_kind: CursorKind,
    scroll_offset: u16,
    gutter: GutterView,
    signs: ruster_render::SignsView,
    selection: Option<ruster_render::SelectionView>,
    active: bool,
    theme: Option<ruster_render::Theme>,
}

impl BufferWidget {
    pub fn new(lines: Vec<StyledLine>, cursor: (u16, u16)) -> Self {
        BufferWidget {
            lines,
            cursor,
            extra_cursors: Vec::new(),
            syntax: false,
            cursor_visible: true,
            cursor_kind: CursorKind::Block,
            scroll_offset: 0,
            gutter: GutterView::default(),
            signs: ruster_render::SignsView::default(),
            selection: None,
            active: true,
            theme: None,
        }
    }

    pub fn with_signs(mut self, signs: ruster_render::SignsView) -> Self {
        self.signs = signs;
        self
    }

    pub fn with_extra_cursors(mut self, extra: Vec<(u16, u16)>) -> Self {
        self.extra_cursors = extra;
        self
    }

    pub fn with_selection(mut self, selection: Option<ruster_render::SelectionView>) -> Self {
        self.selection = selection;
        self
    }

    pub fn with_active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn with_syntax(mut self, yes: bool) -> Self {
        self.syntax = yes;
        self
    }

    pub fn with_cursor_visible(mut self, visible: bool) -> Self {
        self.cursor_visible = visible;
        self
    }

    pub fn with_cursor_kind(mut self, kind: CursorKind) -> Self {
        self.cursor_kind = kind;
        self
    }

    pub fn with_scroll(mut self, offset: u16) -> Self {
        self.scroll_offset = offset;
        self
    }

    pub fn with_gutter(mut self, gutter: GutterView) -> Self {
        self.gutter = gutter;
        self
    }

    pub fn with_theme(mut self, theme: &ruster_render::Theme) -> Self {
        self.theme = Some(*theme);
        self
    }
}

impl Widget for BufferWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let default_theme = ruster_render::Theme::default();
        let theme = self.theme.as_ref().unwrap_or(&default_theme);
        // Layout: sign column, then line-number gutter, then text.
        let sign_w = self.signs.width.min(area.width);
        let gutter_x = area.x + sign_w;
        let gutter_w = self.gutter.width.min(area.width.saturating_sub(sign_w));
        let text_x = gutter_x + gutter_w;
        let scroll = self.scroll_offset as usize;

        // Sign column, left of the gutter.
        if sign_w > 0 {
            for row in 0..area.height {
                let line = row + scroll as u16;
                if let Some((glyph, c)) = self.signs.at(line) {
                    let y = area.y + row;
                    if let Some(cell) = buf.cell_mut((area.x, y)) {
                        cell.set_char(glyph);
                        cell.set_fg(ruster_render_color_to_tui(&c));
                    }
                }
            }
        }

        let gutter_fg = self
            .theme
            .map(|t| ruster_render_color_to_tui(&t.gutter))
            .unwrap_or(Color::DarkGray);
        let gutter_bg = self
            .theme
            .map(|t| ruster_render_color_to_tui(&t.gutter_bg))
            .unwrap_or(Color::Reset);
        // Gutter background — only paint when it differs from main bg.
        if gutter_bg != Color::Reset {
            for row in 0..area.height {
                let y = area.y + row;
                for x in gutter_x..text_x {
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_bg(gutter_bg);
                    }
                }
            }
        }
        // Gutter column.
        for (row, label) in self.gutter.rows.iter().enumerate() {
            if row as u16 >= area.height {
                break;
            }
            let y = area.y + row as u16;
            // Right-align within the gutter width (labels already padded to fit).
            let start = gutter_w.saturating_sub(label.chars().count() as u16);
            for (i, ch) in label.chars().enumerate() {
                let x = gutter_x + start + i as u16;
                if x >= text_x {
                    break;
                }
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    cell.set_fg(gutter_fg);
                }
            }
        }

        let mut style_map: std::collections::HashMap<(u16, u16), (RColor, RColor)> =
            std::collections::HashMap::new();
        if self.syntax {
            for (row, line) in self.lines.iter().skip(scroll).enumerate() {
                let y = row as u16;
                if y >= area.height {
                    break;
                }
                for (offset, length, style) in &line.highlights {
                    for c in 0..*length {
                        let x = (offset + c) as u16;
                        style_map.insert((y, x), (style.fg, style.bg));
                    }
                }
            }
        }

        let selection_bg = self
            .theme
            .map(|t| ruster_render_color_to_tui(&t.selection_bg))
            .unwrap_or(Color::Rgb(88, 91, 112));
        let selection_fg = self
            .theme
            .map(|t| ruster_render_color_to_tui(&t.selection_fg))
            .unwrap_or(Color::Reset);
        for (row, line) in self.lines.iter().skip(scroll).enumerate() {
            if row as u16 >= area.height {
                break;
            }
            let y = area.y + row as u16;
            let buffer_line = row + scroll;
            let is_cursor_line = buffer_line as u16 == self.cursor.0;
            let line_len = line.text.chars().count() as u16;
            // Columns selected on this line, if any.
            let sel_span = self
                .selection
                .and_then(|s| s.span_on(buffer_line as u16, line_len));
            // Paint the selection background first (covers empty lines too).
            if let Some((sel_start, sel_end)) = sel_span {
                for col in sel_start..=sel_end.min(line_len.max(sel_start)) {
                    let x = text_x + col;
                    if x >= area.right() {
                        break;
                    }
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_bg(selection_bg);
                    }
                }
            }
            for (j, ch) in line.text.chars().enumerate() {
                let x = text_x + j as u16;
                if x >= area.right() {
                    break;
                }
                let selected = sel_span
                    .map(|(s, e)| j as u16 >= s && j as u16 <= e)
                    .unwrap_or(false);
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    if is_cursor_line && j as u16 == self.cursor.1 && self.cursor_visible {
                        if self.active {
                            apply_cursor(cell, self.cursor_kind, theme);
                        } else {
                            apply_dim_cursor(cell, self.cursor_kind, theme);
                        }
                    } else {
                        if let Some((fg, bg)) = style_map.get(&(row as u16, j as u16)) {
                            cell.set_fg(ruster_render_color_to_tui(fg));
                            if !matches!(bg, RColor::Default) {
                                cell.set_bg(ruster_render_color_to_tui(bg));
                            }
                        }
                        if selected {
                            cell.set_bg(selection_bg);
                            cell.set_fg(selection_fg);
                        }
                    }
                }
            }
            if is_cursor_line && self.cursor_visible && self.cursor.1 >= line_len {
                let x = text_x + self.cursor.1;
                if x < area.right() {
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_char(' ');
                        if self.active {
                            apply_cursor(cell, self.cursor_kind, theme);
                        } else {
                            apply_dim_cursor(cell, self.cursor_kind, theme);
                        }
                    }
                }
            }
        }

        // Extra multi-cursor carets, painted over the text as solid blocks so
        // they're visible without being the terminal's focus cursor.
        for &(cl, cc) in &self.extra_cursors {
            if (cl as usize) < scroll {
                continue;
            }
            let row = cl as usize - scroll;
            if row as u16 >= area.height {
                continue;
            }
            let x = text_x + cc;
            if x >= area.right() {
                continue;
            }
            if let Some(cell) = buf.cell_mut((x, area.y + row as u16)) {
                apply_cursor(cell, self.cursor_kind, theme);
            }
        }
    }
}

/// Renders one window's statusline from a [`StatuslineView`] (left / center /
/// right groups). The active window's statusline reads as a panel strip with an
/// amber mode badge on its left edge.
pub struct StatuslineWidget {
    view: StatuslineView,
    bar_bg: Color,
    bar_fg: Color,
    dim_bg: Color,
    dim_fg: Color,
    mode_bg: Color,
    mode_fg: Color,
}

impl StatuslineWidget {
    pub fn new(view: StatuslineView) -> Self {
        StatuslineWidget {
            view,
            bar_bg: Color::Rgb(17, 26, 17),
            bar_fg: Color::White,
            dim_bg: Color::Rgb(17, 26, 17),
            dim_fg: Color::Gray,
            mode_bg: Color::Rgb(255, 136, 0),
            mode_fg: Color::Rgb(10, 14, 10),
        }
    }

    pub fn with_theme(mut self, theme: &ruster_render::Theme) -> Self {
        self.bar_bg = ruster_render_color_to_tui(&theme.statusline_bg);
        self.bar_fg = ruster_render_color_to_tui(&theme.statusline_fg);
        self.dim_bg = ruster_render_color_to_tui(&theme.statusline_bg);
        self.dim_fg = ruster_render_color_to_tui(&theme.gutter);
        self.mode_bg = ruster_render_color_to_tui(&theme.mode_bg(self.view.mode));
        self.mode_fg = ruster_render_color_to_tui(&theme.mode_fg(self.view.mode));
        self
    }
}

impl Widget for StatuslineWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let (bg, fg) = if self.view.active {
            (self.bar_bg, self.bar_fg)
        } else {
            (self.dim_bg, self.dim_fg)
        };

        let put = |buf: &mut Buffer, x: u16, y: u16, ch: char, cf: Color, cb: Color| {
            if x >= area.left() && x < area.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    cell.set_fg(cf);
                    cell.set_bg(cb);
                }
            }
        };

        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, area.y)) {
                cell.set_bg(bg);
            }
        }

        let left = &self.view.left;
        let right = &self.view.right;
        let center = &self.view.center;

        let mut x = area.x;
        if !left.is_empty() {
            put(buf, x, area.y, ' ', self.mode_fg, self.mode_bg);
            x += 1;
            for (i, ch) in left.chars().enumerate() {
                put(buf, x + i as u16, area.y, ch, self.mode_fg, self.mode_bg);
            }
            x += left.chars().count() as u16;
            put(buf, x, area.y, ' ', self.mode_fg, self.mode_bg);
            x += 1;
            put(buf, x, area.y, '│', fg, bg);
            x += 1;
        }

        let right_len = right.chars().count() as u16;
        let right_start = area.right().saturating_sub(right_len + 2);
        if right_start > x {
            for (i, ch) in center.chars().enumerate() {
                let cx = x + i as u16;
                if cx >= right_start {
                    break;
                }
                put(buf, cx, area.y, ch, fg, bg);
            }
            put(buf, right_start, area.y, ' ', fg, bg);
            for (i, ch) in right.chars().enumerate() {
                put(buf, right_start + 1 + i as u16, area.y, ch, fg, bg);
            }
            put(buf, right_start + 1 + right_len, area.y, ' ', fg, bg);
        }
    }
}

/// Renders the cmdline prompt line.
pub struct CmdlineWidget<'a> {
    text: &'a str,
    fg: Color,
    bg: Color,
    is_message: bool,
    theme: Option<ruster_render::Theme>,
}

impl<'a> CmdlineWidget<'a> {
    pub fn new(text: &'a str) -> Self {
        CmdlineWidget {
            text,
            fg: Color::White,
            bg: Color::Black,
            is_message: false,
            theme: None,
        }
    }

    pub fn with_message_style(mut self) -> Self {
        self.is_message = true;
        self
    }

    pub fn with_theme(mut self, theme: &ruster_render::Theme) -> Self {
        self.fg = ruster_render_color_to_tui(&theme.cmdline_fg);
        self.bg = ruster_render_color_to_tui(&theme.cmdline_bg);
        self.theme = Some(*theme);
        self
    }
}

/// Whether cmdline text opens with a prompt sigil the compositor should tint.
///
/// `:` is a command, `/` and `?` are forward and backward search. Anything else
/// on this row is output rather than a prompt — an echoed message, an error —
/// and gets no accent, which is the distinction the accent exists to draw.
pub fn is_prompt_sigil(text: &str) -> bool {
    matches!(text.chars().next(), Some(':') | Some('/') | Some('?'))
}

impl Widget for CmdlineWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let fg = if self.is_message {
            self.theme
                .as_ref()
                .map(|t| ruster_render_color_to_tui(&t.accent))
                .unwrap_or(self.fg)
        } else {
            self.fg
        };
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, area.y)) {
                cell.set_char(' ');
                cell.set_bg(self.bg);
            }
        }
        // The leading sigil is tinted so a prompt reads differently from a
        // message at a glance — the two share this row, and until now shared a
        // colour too. Messages have no sigil and are left alone.
        let accent = self
            .theme
            .as_ref()
            .filter(|_| !self.is_message)
            .filter(|_| is_prompt_sigil(self.text))
            .map(|t| ruster_render_color_to_tui(&t.cmdline_accent));

        for (i, ch) in self.text.chars().enumerate() {
            let x = area.x + i as u16;
            if x >= area.right() {
                break;
            }
            if let Some(cell) = buf.cell_mut((x, area.y)) {
                cell.set_char(ch);
                cell.set_fg(if i == 0 { accent.unwrap_or(fg) } else { fg });
                cell.set_bg(self.bg);
            }
        }
    }
}

/// Renders the welcome / "Dashboard" screen — a centered panel shown when no
/// file is open, styled as a starship crew terminal readout.
pub struct WelcomeWidget {
    view: WelcomeView,
    theme: Option<ruster_render::Theme>,
}

impl WelcomeWidget {
    pub fn new(view: WelcomeView) -> Self {
        WelcomeWidget { view, theme: None }
    }

    pub fn with_theme(mut self, theme: &ruster_render::Theme) -> Self {
        self.theme = Some(*theme);
        self
    }

    fn c(&self, fallback: Color, get: impl FnOnce(&ruster_render::Theme) -> RColor) -> Color {
        self.theme
            .as_ref()
            .map(|t| ruster_render_color_to_tui(&get(t)))
            .unwrap_or(fallback)
    }
}

impl Widget for WelcomeWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let bg = self.c(Color::Rgb(10, 14, 10), |t| t.bg);
        let fg = self.c(Color::Rgb(51, 255, 102), |t| t.fg);
        let accent = self.c(Color::Rgb(255, 136, 0), |t| t.accent);
        let dim = self.c(Color::Rgb(26, 102, 51), |t| t.gutter);
        let _panel = self.c(Color::Rgb(17, 26, 17), |t| t.divider);

        let put = |buf: &mut Buffer, x: u16, y: u16, ch: char, cf: Color, cb: Color| {
            if x < area.right() && y < area.bottom() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    cell.set_fg(cf);
                    cell.set_bg(cb);
                }
            }
        };
        let text = |buf: &mut Buffer, x: u16, y: u16, s: &str, cf: Color, cb: Color| {
            for (i, ch) in s.chars().enumerate() {
                put(buf, x + i as u16, y, ch, cf, cb);
            }
        };

        // Clear the whole area.
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(' ');
                    cell.set_bg(bg);
                }
            }
        }

        let cx = area.width / 2;
        let mut row = area.y + 1;

        // Title: RUSTER
        let title = format!("RUSTER  {}", self.view.version);
        let tx = cx.saturating_sub(title.chars().count() as u16 / 2);
        text(buf, tx, row, &title, fg, bg);
        row += 1;

        // Divider line
        let div = "─".repeat(area.width as usize - 4);
        text(buf, area.x + 2, row, &div, dim, bg);
        row += 1;

        // "DASHBOARD" in accent
        let rr = "DASHBOARD";
        let rx = cx.saturating_sub(rr.chars().count() as u16 / 2);
        text(buf, rx, row, rr, accent, bg);
        row += 2;

        // Section: Recent Projects
        row = draw_section_header(
            buf,
            put,
            text,
            area,
            row,
            "RECENT PROJECTS",
            accent,
            dim,
            bg,
        );
        if self.view.recent_projects.is_empty() {
            text(buf, area.x + 4, row, "  No recent projects", dim, bg);
            row += 1;
        } else {
            for (i, proj) in self.view.recent_projects.iter().enumerate() {
                let n = (i + 1).to_string();
                text(buf, area.x + 4, row, &format!(" {}. {}", n, proj), fg, bg);
                row += 1;
            }
        }
        row += 1;

        // Section: Quick Actions
        row = draw_section_header(buf, put, text, area, row, "QUICK ACTIONS", accent, dim, bg);
        let actions = [
            (":e <path>", "Open file (Tab to complete)"),
            (":Dired", "File Explorer"),
            (":Files", "Find Files"),
            (":term", "Terminal"),
        ];
        for (cmd, desc) in &actions {
            text(buf, area.x + 4, row, cmd, fg, bg);
            let dl = cmd.chars().count() as u16 + 2;
            text(buf, area.x + 4 + dl, row, desc, dim, bg);
            row += 1;
        }
        row += 1;

        // Section: System Status
        row = draw_section_header(buf, put, text, area, row, "SYSTEM STATUS", accent, dim, bg);
        text(
            buf,
            area.x + 4,
            row,
            &format!("  LSP  {}", self.view.lsp_status),
            fg,
            bg,
        );
        row += 1;
        text(
            buf,
            area.x + 4,
            row,
            &format!("  Mode: {}", self.view.edit_mode),
            fg,
            bg,
        );
        row += 1;
        row += 1;

        // Section: Keybinds
        row = draw_section_header(buf, put, text, area, row, "KEYBINDS", accent, dim, bg);
        let binds = [
            ("Ctrl+P", "Fuzzy Finder"),
            ("Ctrl+S", "Save"),
            ("Ctrl+W", "Window Commands"),
            (":help", "Help"),
        ];
        for (key, desc) in &binds {
            text(
                buf,
                area.x + 4,
                row,
                &format!("  {:<12}{}", key, desc),
                fg,
                bg,
            );
            row += 1;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_section_header(
    buf: &mut Buffer,
    _put: impl Fn(&mut Buffer, u16, u16, char, Color, Color),
    text: impl Fn(&mut Buffer, u16, u16, &str, Color, Color),
    area: Rect,
    row: u16,
    label: &str,
    accent: Color,
    _dim: Color,
    bg: Color,
) -> u16 {
    let mut r = row;
    let hdr = format!(" ▌{}▐ ", label);
    text(buf, area.x + 2, r, &hdr, accent, bg);
    r += 1;
    r
}

/// Renders a floating picker overlay (title, query line, selectable rows).
pub struct PickerWidget {
    /// First visible row, so a selection past the box stays on screen.
    scroll: usize,
    view: ruster_render::PickerView,
    theme: Option<ruster_render::Theme>,
}

impl PickerWidget {
    pub fn new(view: ruster_render::PickerView) -> Self {
        PickerWidget {
            view,
            theme: None,
            scroll: 0,
        }
    }

    pub fn with_theme(mut self, theme: &ruster_render::Theme) -> Self {
        self.theme = Some(*theme);
        self
    }

    pub fn with_scroll(mut self, scroll: usize) -> Self {
        self.scroll = scroll;
        self
    }
}

/// Parse a `#RRGGBB` value into RGB, for the color-setting swatch.
fn hex_rgb(s: &str) -> Option<(u8, u8, u8)> {
    let b = s.as_bytes();
    if b.len() != 7 || b[0] != b'#' {
        return None;
    }
    let r = u8::from_str_radix(&s[1..3], 16).ok()?;
    let g = u8::from_str_radix(&s[3..5], 16).ok()?;
    let bl = u8::from_str_radix(&s[5..7], 16).ok()?;
    Some((r, g, bl))
}

/// Renders the label/value string for a settings control.
pub fn control_display(row: &SettingRowView) -> String {
    ruster_render::control_display(row)
}

/// One rendered line of the settings body: a group header or a setting row.
enum SettingsLine<'a> {
    Header(&'a str),
    Row(&'a SettingRowView),
}

pub struct SettingsWidget<'a> {
    view: SettingsView,
    scroll: &'a mut usize,
    theme: Option<ruster_render::Theme>,
}

impl<'a> SettingsWidget<'a> {
    pub fn new(view: SettingsView, scroll: &'a mut usize) -> Self {
        SettingsWidget {
            view,
            scroll,
            theme: None,
        }
    }

    pub fn with_theme(mut self, theme: &ruster_render::Theme) -> Self {
        self.theme = Some(*theme);
        self
    }
}

impl Widget for SettingsWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let c = |fallback: Color, get: fn(&ruster_render::Theme) -> RColor| -> Color {
            self.theme
                .as_ref()
                .map(|t| ruster_render_color_to_tui(&get(t)))
                .unwrap_or(fallback)
        };
        let bg = c(Color::Rgb(30, 30, 46), |t| t.bg);
        let accent = c(Color::Rgb(137, 180, 250), |t| t.accent);
        let fg = c(Color::Rgb(205, 214, 244), |t| t.fg);
        let dim = c(Color::Rgb(127, 132, 156), |t| t.gutter);
        let sel = c(Color::Rgb(88, 91, 112), |t| t.selection_bg);
        let sel_fg = c(Color::Rgb(205, 214, 244), |t| t.selection_fg);
        let divider = c(Color::Rgb(69, 71, 90), |t| t.divider);
        let footer_bg = c(Color::Rgb(24, 24, 37), |t| t.bg);
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(' ');
                    cell.set_bg(bg);
                }
            }
        }
        let put = |buf: &mut Buffer, x: u16, y: u16, ch: char, c_fg: Color, c_bg: Color| {
            if x >= area.left() && x < area.right() && y >= area.top() && y < area.bottom() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    cell.set_fg(c_fg);
                    cell.set_bg(c_bg);
                }
            }
        };
        let text = |buf: &mut Buffer, x: u16, y: u16, s: &str, c_fg: Color, c_bg: Color| {
            for (i, ch) in s.chars().enumerate() {
                put(buf, x + i as u16, y, ch, c_fg, c_bg);
            }
        };

        // A bordered box whose top edge carries the title, so the sides meet the
        // rule at the corners rather than starting under it.
        let title = format!("Settings{}", if self.view.dirty { " [+]" } else { "" });
        titled_box(buf, area, &title, accent, divider, bg);

        // Flatten groups into header + row lines.
        let mut lines: Vec<SettingsLine> = Vec::new();
        for g in &self.view.groups {
            lines.push(SettingsLine::Header(&g.name));
            for r in &g.rows {
                lines.push(SettingsLine::Row(r));
            }
        }
        let selected = lines
            .iter()
            .position(|l| matches!(l, SettingsLine::Row(r) if r.selected))
            .unwrap_or(0);

        // Scroll so the selected line stays visible (body between title/footer),
        // holding position until the selection crosses an edge.
        let body_top = area.y + 1;
        let body_h = area.height.saturating_sub(3) as usize; // title + footer + help
        *self.scroll = ruster_render::list_scroll(*self.scroll, selected, body_h, lines.len());
        let scroll = *self.scroll;
        let value_col = area.x + 32.min(area.width / 2);

        for (i, line) in lines.iter().skip(scroll).take(body_h).enumerate() {
            let y = body_top + i as u16;
            match line {
                SettingsLine::Header(name) => {
                    text(
                        buf,
                        area.x + 1,
                        y,
                        &format!("── {} ", name.to_uppercase()),
                        accent,
                        bg,
                    );
                }
                SettingsLine::Row(r) => {
                    let (row_fg, row_bg) = if r.selected {
                        // Inset by one so the highlight stops at the border
                        // instead of painting over it.
                        for x in (area.left() + 1)..area.right().saturating_sub(1) {
                            put(buf, x, y, ' ', sel_fg, sel);
                        }
                        (sel_fg, sel)
                    } else {
                        (fg, bg)
                    };
                    text(buf, area.x + 2, y, &r.label, row_fg, row_bg);
                    let ctrl = control_display(r);
                    let cfg = if r.editing { accent } else { row_fg };
                    text(buf, value_col, y, &ctrl, cfg, row_bg);
                    // A swatch after a hex color value.
                    if let Some((cr, cg, cb)) = r.swatch.as_deref().and_then(hex_rgb) {
                        let sx = value_col + ctrl.chars().count() as u16 + 1;
                        for dx in 0..2 {
                            put(buf, sx + dx, y, ' ', row_fg, Color::Rgb(cr, cg, cb));
                        }
                    }
                }
            }
        }

        // Help/footer at the bottom.
        let fy = area.bottom().saturating_sub(1);
        for x in area.left()..area.right() {
            put(buf, x, fy, ' ', dim, footer_bg);
        }
        text(buf, area.x + 1, fy, &self.view.footer, dim, footer_bg);

        // Selected row's help just above the footer.
        if let Some(SettingsLine::Row(r)) = lines.get(selected) {
            let hy = area.bottom().saturating_sub(2);
            text(buf, area.x + 1, hy, &r.help, dim, bg);
        }
    }
}

impl Widget for PickerWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let c = |fallback: Color, get: fn(&ruster_render::Theme) -> RColor| -> Color {
            self.theme
                .as_ref()
                .map(|t| ruster_render_color_to_tui(&get(t)))
                .unwrap_or(fallback)
        };
        let bg = c(Color::Rgb(30, 30, 46), |t| t.bg);
        let preview_bg = c(Color::Rgb(24, 24, 37), |t| t.bg);
        let accent = c(Color::Rgb(137, 180, 250), |t| t.accent);
        let fg = c(Color::Rgb(205, 214, 244), |t| t.fg);
        let divider = c(Color::Rgb(69, 71, 90), |t| t.divider);
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(' ');
                    cell.set_bg(bg);
                }
            }
        }
        let put = |buf: &mut Buffer, x: u16, y: u16, ch: char, fg: Color, cell_bg: Color| {
            if x >= area.left() && x < area.right() && y >= area.top() && y < area.bottom() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    cell.set_fg(fg);
                    cell.set_bg(cell_bg);
                }
            }
        };

        // Split into a list column and (when there's a preview) a preview column.
        let has_preview = !self.view.preview.is_empty();
        let list_w = if has_preview {
            area.width * 2 / 5
        } else {
            area.width
        };
        let list_right = area.x + list_w;

        // A bordered box whose top edge carries the title, matching the settings
        // overlay. Everything below starts a row down so the rule stays whole.
        titled_box(buf, area, &self.view.title, accent, divider, bg);
        // Query row.
        let query = format!(" > {}", self.view.query);
        for (i, ch) in query.chars().enumerate() {
            put(buf, area.x + 1 + i as u16, area.y + 1, ch, Color::White, bg);
        }
        // Item rows (clipped to the list column).
        for (row, item) in self.view.rows.iter().skip(self.scroll).enumerate() {
            let y = area.y + 2 + row as u16;
            if y >= area.bottom().saturating_sub(1) {
                break;
            }
            let (row_fg, row_bg) = if item.selected {
                (Color::Black, accent)
            } else {
                (fg, bg)
            };
            for x in (area.left() + 1)..list_right.min(area.right().saturating_sub(1)) {
                put(buf, x, y, ' ', row_fg, row_bg);
            }
            let label = format!(" {}", item.label);
            for (i, ch) in label.chars().enumerate() {
                let x = area.x + 1 + i as u16;
                if x >= list_right {
                    break;
                }
                put(buf, x, y, ch, row_fg, row_bg);
            }
        }

        // Preview column: highlighted file contents.
        if has_preview {
            let px = list_right + 1;
            // Start below the header so the rule stays unbroken across the top.
            for y in (area.top() + 1)..area.bottom().saturating_sub(1) {
                for x in px..area.right().saturating_sub(1) {
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_bg(preview_bg);
                    }
                }
                // Divider between the two columns.
                put(buf, list_right, y, '│', divider, bg);
            }
            for (row, line) in self.view.preview.iter().enumerate() {
                let y = area.y + 1 + row as u16;
                if y >= area.bottom().saturating_sub(1) {
                    break;
                }
                let mut colors: std::collections::HashMap<usize, RColor> =
                    std::collections::HashMap::new();
                for (offset, len, style) in &line.highlights {
                    for c in 0..*len {
                        colors.insert(offset + c, style.fg);
                    }
                }
                for (i, ch) in line.text.chars().enumerate() {
                    let x = px + 1 + i as u16;
                    if x >= area.right().saturating_sub(1) {
                        break;
                    }
                    let pfg = colors.get(&i).map(ruster_render_color_to_tui).unwrap_or(fg);
                    put(buf, x, y, ch, pfg, preview_bg);
                }
            }
        }
    }
}

/// Renders the bottom which-key panel (title on top, one binding per line).
/// The caller sizes `area` to the currently-visible height for the slide-up.
pub struct WhichKeyWidget {
    view: ruster_render::WhichKeyView,
    theme: Option<ruster_render::Theme>,
}

impl WhichKeyWidget {
    pub fn new(view: ruster_render::WhichKeyView) -> Self {
        WhichKeyWidget { view, theme: None }
    }

    pub fn with_theme(mut self, theme: &ruster_render::Theme) -> Self {
        self.theme = Some(*theme);
        self
    }
}

impl Widget for WhichKeyWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let c = |fallback: Color, get: fn(&ruster_render::Theme) -> RColor| -> Color {
            self.theme
                .as_ref()
                .map(|t| ruster_render_color_to_tui(&get(t)))
                .unwrap_or(fallback)
        };
        let bg = c(Color::Rgb(30, 30, 46), |t| t.whichkey_bg);
        let accent = c(Color::Rgb(137, 180, 250), |t| t.accent);
        let fg = c(Color::Rgb(205, 214, 244), |t| t.whichkey_fg);
        let key_accent = c(accent, |t| t.whichkey_key);
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(' ');
                    cell.set_bg(bg);
                }
            }
        }
        let put = |buf: &mut Buffer, x: u16, y: u16, ch: char, color: Color| {
            if x < area.right() && y < area.bottom() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    cell.set_fg(color);
                    cell.set_bg(bg);
                }
            }
        };
        // Title row.
        let title = format!(" {} ", self.view.title);
        for (i, ch) in title.chars().enumerate() {
            put(buf, area.x + i as u16, area.y, ch, accent);
        }
        // One binding per line below the title. The key letter takes its own
        // accent colour, then two spaces and the description in the panel fg.
        for (row, entry) in self.view.rows.iter().enumerate() {
            let y = area.y + 1 + row as u16;
            if y >= area.bottom() {
                break;
            }
            let mut x = area.x + 2;
            for ch in entry.key.chars() {
                put(buf, x, y, ch, key_accent);
                x += 1;
            }
            x += 2;
            for ch in entry.desc.chars() {
                put(buf, x, y, ch, fg);
                x += 1;
            }
        }
    }
}

/// Renders a [`FloatView`](ruster_render::FloatView): a bordered box of
/// syntax-highlighted lines, drawn above the window views.
///
/// The rect is already resolved and clamped by `FloatView::anchored`, so this
/// only paints — no geometry decisions live here, which is what keeps the two
/// backends from drifting.
pub struct FloatWidget {
    float: ruster_render::FloatView,
    theme: Option<ruster_render::Theme>,
}

impl FloatWidget {
    pub fn new(float: ruster_render::FloatView) -> Self {
        FloatWidget { float, theme: None }
    }

    pub fn with_theme(mut self, theme: &ruster_render::Theme) -> Self {
        self.theme = Some(*theme);
        self
    }
}

impl Widget for FloatWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let c = |fallback: Color, get: fn(&ruster_render::Theme) -> RColor| -> Color {
            self.theme
                .as_ref()
                .map(|t| ruster_render_color_to_tui(&get(t)))
                .unwrap_or(fallback)
        };
        let bg = c(Color::Rgb(24, 24, 37), |t| t.bg);
        let fg = c(Color::Rgb(205, 214, 244), |t| t.fg);
        let border = c(Color::Rgb(243, 139, 168), |t| t.accent);

        // Fill first, so whatever was underneath does not show through.
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(' ');
                    cell.set_bg(bg);
                    cell.set_fg(fg);
                }
            }
        }

        if self.float.border && area.width >= 2 && area.height >= 2 {
            let (l, r, t, b) = (area.left(), area.right() - 1, area.top(), area.bottom() - 1);
            let mut put = |x: u16, y: u16, ch: char| {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    cell.set_fg(border);
                    cell.set_bg(bg);
                }
            };
            for x in l..=r {
                put(x, t, '─');
                put(x, b, '─');
            }
            for y in t..=b {
                put(l, y, '│');
                put(r, y, '│');
            }
            put(l, t, '╭');
            put(r, t, '╮');
            put(l, b, '╰');
            put(r, b, '╯');

            if let Some(title) = &self.float.title {
                // Inset by two so the title never overwrites a corner.
                for (i, ch) in title.chars().enumerate() {
                    let x = l + 2 + i as u16;
                    if x >= r {
                        break;
                    }
                    put(x, t, ch);
                }
            }
        }

        let inner = self.float.inner();
        for (row, line) in self.float.lines.iter().enumerate() {
            let y = inner.y + row as u16;
            if y >= inner.y.saturating_add(inner.height) {
                break;
            }
            let mut colors: std::collections::HashMap<usize, RColor> =
                std::collections::HashMap::new();
            for (offset, len, style) in &line.highlights {
                for i in 0..*len {
                    colors.insert(offset + i, style.fg);
                }
            }
            for (i, ch) in line.text.chars().enumerate() {
                let x = inner.x + i as u16;
                if x >= inner.x.saturating_add(inner.width) {
                    break;
                }
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    cell.set_fg(colors.get(&i).map(ruster_render_color_to_tui).unwrap_or(fg));
                    cell.set_bg(bg);
                }
            }
        }
    }
}

/// Renders a [`DialogView`](ruster_render::DialogView): a titled box of setting
/// rows. Shares `titled_box` and the row layout with the settings page, because
/// it is the same vocabulary — a dialog is a small settings page.
pub struct DialogWidget {
    view: ruster_render::DialogView,
    theme: Option<ruster_render::Theme>,
}

impl DialogWidget {
    pub fn new(view: ruster_render::DialogView) -> Self {
        DialogWidget { view, theme: None }
    }

    pub fn with_theme(mut self, theme: &ruster_render::Theme) -> Self {
        self.theme = Some(*theme);
        self
    }
}

impl Widget for DialogWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let c = |fallback: Color, get: fn(&ruster_render::Theme) -> RColor| -> Color {
            self.theme
                .as_ref()
                .map(|t| ruster_render_color_to_tui(&get(t)))
                .unwrap_or(fallback)
        };
        let bg = c(Color::Rgb(30, 30, 46), |t| t.bg);
        let fg = c(Color::Rgb(205, 214, 244), |t| t.fg);
        let accent = c(Color::Rgb(137, 180, 250), |t| t.accent);
        let divider = c(Color::Rgb(69, 71, 90), |t| t.divider);
        let sel = c(Color::Rgb(88, 91, 112), |t| t.selection_bg);
        let sel_fg = c(Color::Rgb(205, 214, 244), |t| t.selection_fg);
        let dim = c(Color::Rgb(127, 132, 156), |t| t.gutter);

        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(' ');
                    cell.set_bg(bg);
                }
            }
        }
        titled_box(buf, area, &self.view.title, accent, divider, bg);

        let text = |buf: &mut Buffer, x: u16, y: u16, s: &str, fg: Color, cb: Color| {
            for (i, ch) in s.chars().enumerate() {
                let cx = x + i as u16;
                if cx >= area.right().saturating_sub(1) {
                    break;
                }
                if let Some(cell) = buf.cell_mut((cx, y)) {
                    cell.set_char(ch);
                    cell.set_fg(fg);
                    cell.set_bg(cb);
                }
            }
        };

        let value_col = area.x + 26.min(area.width / 2);
        for (i, r) in self.view.rows.iter().enumerate() {
            let y = area.y + 1 + i as u16;
            if y >= area.bottom().saturating_sub(2) {
                break;
            }
            let (rfg, rbg) = if r.selected { (sel_fg, sel) } else { (fg, bg) };
            if r.selected {
                for x in (area.left() + 1)..area.right().saturating_sub(1) {
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_char(' ');
                        cell.set_bg(rbg);
                        cell.set_fg(rfg);
                    }
                }
            }
            let shown = control_display(r);
            if r.kind == ruster_render::ControlKind::Button {
                // A button is one thing, not a label with a value beside it.
                text(buf, area.x + 2, y, &shown, rfg, rbg);
            } else {
                text(buf, area.x + 2, y, &r.label, rfg, rbg);
                text(
                    buf,
                    value_col,
                    y,
                    &shown,
                    if r.editing { accent } else { rfg },
                    rbg,
                );
            }
        }
        let fy = area.bottom().saturating_sub(2);
        text(buf, area.x + 2, fy, &self.view.footer, dim, bg);
    }
}

#[cfg(test)]
mod tests {
    use super::{BufferWidget, FloatWidget, TerminalWidget};
    use crate::widgets::{cmdline_label, mode_label};
    use ratatui::buffer::Buffer as RBuffer;
    use ratatui::layout::Rect;
    use ratatui::widgets::Widget;
    use ruster_core::vim::VimMode;
    use ruster_render::{Color as RColor, StyledLine, TermCellView, TermGridView};

    #[test]
    fn the_cmdline_sigil_is_accented_and_the_rest_is_not() {
        use super::Widget as _;
        let theme = ruster_render::Theme::default();
        let mut buf = super::Buffer::empty(super::Rect::new(0, 0, 8, 1));
        super::CmdlineWidget::new(":write")
            .with_theme(&theme)
            .render(super::Rect::new(0, 0, 8, 1), &mut buf);

        let accent = super::ruster_render_color_to_tui(&theme.cmdline_accent);
        let text = super::ruster_render_color_to_tui(&theme.cmdline_fg);
        assert_eq!(buf.cell((0, 0)).unwrap().fg, accent, "the `:` is accented");
        assert_eq!(buf.cell((1, 0)).unwrap().fg, text, "the command is not");
    }

    #[test]
    fn a_message_gets_no_sigil_accent() {
        use super::Widget as _;
        // Messages share this row with prompts. Accenting their first character
        // would be actively misleading — it would read as a prompt awaiting
        // input when the editor is not waiting for anything.
        let theme = ruster_render::Theme::default();
        let mut buf = super::Buffer::empty(super::Rect::new(0, 0, 12, 1));
        super::CmdlineWidget::new("written")
            .with_theme(&theme)
            .with_message_style()
            .render(super::Rect::new(0, 0, 12, 1), &mut buf);

        // Not "the first cell is some other colour" — a message is already
        // drawn in the theme accent, and cmdline_accent defaults to the same
        // value, so that assertion would be vacuously true or false depending
        // on the theme. The property is that a message is drawn in *one*
        // colour: no character is singled out the way a prompt sigil is.
        let first = buf.cell((0, 0)).unwrap().fg;
        for (i, ch) in "written".chars().enumerate() {
            assert_eq!(
                buf.cell((i as u16, 0)).unwrap().fg,
                first,
                "message character {ch:?} at {i} should match the rest"
            );
        }
    }

    #[test]
    fn terminal_widget_draws_cells_colors_and_cursor() {
        let grid = TermGridView {
            cols: 3,
            rows: 1,
            cells: vec![
                TermCellView {
                    c: 'h',
                    fg: RColor::Rgb(10, 20, 30),
                    ..TermCellView::default()
                },
                TermCellView {
                    c: 'i',
                    ..TermCellView::default()
                },
                TermCellView {
                    c: '!',
                    ..TermCellView::default()
                },
            ],
            cursor: (0, 1),
        };
        let area = Rect::new(0, 0, 3, 1);
        let mut buf = RBuffer::empty(area);
        TerminalWidget::new(grid).render(area, &mut buf);

        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "h");
        assert_eq!(
            buf.cell((0, 0)).unwrap().fg,
            ratatui::style::Color::Rgb(10, 20, 30)
        );
        // The cursor cell (col 1) is painted as a block.
        assert_ne!(buf.cell((1, 0)).unwrap().bg, ratatui::style::Color::Reset);
        assert_eq!(buf.cell((2, 0)).unwrap().symbol(), "!");
    }

    #[test]
    fn extra_cursors_paint_a_block_over_their_cell() {
        let line = StyledLine {
            text: "abcd".to_string(),
            highlights: vec![],
        };
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = RBuffer::empty(area);
        // Primary at col 0, an extra caret at col 2 ('c').
        BufferWidget::new(vec![line], (0, 0))
            .with_extra_cursors(vec![(0, 2)])
            .render(area, &mut buf);
        // The extra caret reverses fg/bg on its cell; the char is preserved.
        let cell = buf.cell((2, 0)).unwrap();
        assert_eq!(cell.symbol(), "c");
        assert_ne!(
            cell.bg,
            ratatui::style::Color::Reset,
            "extra caret paints a block"
        );
    }

    #[test]
    fn mode_label_normal() {
        assert_eq!(mode_label(&VimMode::Normal), "-- NORMAL --");
    }

    #[test]
    fn mode_label_insert() {
        assert_eq!(mode_label(&VimMode::Insert), "-- INSERT --");
    }

    #[test]
    fn cmdline_label_shows_prompt() {
        assert_eq!(cmdline_label(":w"), ":w");
    }

    #[test]
    fn cmdline_label_empty() {
        assert_eq!(cmdline_label(""), ":");
    }

    fn row_text(buf: &RBuffer, y: u16, x0: u16, x1: u16) -> String {
        (x0..x1)
            .filter_map(|x| buf.cell((x, y)).map(|c| c.symbol().to_string()))
            .collect()
    }

    /// A float must paint over whatever is underneath, border included — that
    /// is the whole point of the primitive.
    #[test]
    fn dialog_widget_draws_a_button_row() {
        use crate::widgets::DialogWidget;
        let area = Rect::new(0, 0, 50, 6);
        let mut buf = RBuffer::empty(area);
        let view = ruster_render::DialogView {
            title: "T".into(),
            rows: vec![ruster_render::SettingRowView {
                label: "OK".into(),
                kind: ruster_render::ControlKind::Button,
                value: String::new(),
                selected: true,
                editing: false,
                help: String::new(),
                swatch: None,
            }],
            footer: "f".into(),
        };
        DialogWidget::new(view).render(area, &mut buf);
        let row: String = (0..area.width)
            .filter_map(|x| buf.cell((x, 1)).map(|c| c.symbol().to_string()))
            .collect();
        assert!(row.contains("[ OK ]"), "button row was {row:?}");
    }

    #[test]
    fn float_widget_paints_a_bordered_box_over_the_background() {
        let area = Rect::new(0, 0, 20, 6);
        let mut buf = RBuffer::filled(area, ratatui::buffer::Cell::new("."));

        let f = ruster_render::FloatView::anchored(
            ruster_render::Rect::new(0, 0, 20, 6),
            ruster_render::FloatAnchor::Center,
            vec![StyledLine {
                text: "hi".into(),
                highlights: vec![],
            }],
        );
        let r = Rect::new(f.rect.x, f.rect.y, f.rect.width, f.rect.height);
        FloatWidget::new(f.clone()).render(r, &mut buf);

        // Corners and edges drawn.
        assert_eq!(buf.cell((r.x, r.y)).unwrap().symbol(), "\u{256d}");
        assert_eq!(buf.cell((r.right() - 1, r.y)).unwrap().symbol(), "\u{256e}");
        assert_eq!(
            buf.cell((r.x, r.bottom() - 1)).unwrap().symbol(),
            "\u{2570}"
        );
        assert_eq!(
            buf.cell((r.right() - 1, r.bottom() - 1)).unwrap().symbol(),
            "\u{256f}"
        );
        // Content sits inside the border.
        let inner = f.inner();
        assert_eq!(row_text(&buf, inner.y, inner.x, inner.x + 2), "hi");
        // The background outside the box is untouched.
        assert_eq!(
            buf.cell((0, 0)).unwrap().symbol(),
            if r.x == 0 && r.y == 0 {
                "\u{256d}"
            } else {
                "."
            }
        );
    }

    #[test]
    fn float_widget_draws_its_title_on_the_top_border() {
        let area = Rect::new(0, 0, 30, 6);
        let mut buf = RBuffer::empty(area);
        let f = ruster_render::FloatView::anchored_titled(
            ruster_render::Rect::new(0, 0, 30, 6),
            ruster_render::FloatAnchor::Center,
            vec![StyledLine {
                text: "body".into(),
                highlights: vec![],
            }],
            Some("Title".into()),
        );
        let r = Rect::new(f.rect.x, f.rect.y, f.rect.width, f.rect.height);
        FloatWidget::new(f).render(r, &mut buf);
        assert_eq!(row_text(&buf, r.y, r.x + 2, r.x + 7), "Title");
    }

    /// Content longer than the inner width is clipped at the border rather than
    /// bleeding past it.
    #[test]
    fn float_widget_clips_content_to_the_inner_area() {
        let area = Rect::new(0, 0, 12, 4);
        let mut buf = RBuffer::filled(area, ratatui::buffer::Cell::new("."));
        let f = ruster_render::FloatView::anchored(
            ruster_render::Rect::new(0, 0, 12, 4),
            ruster_render::FloatAnchor::Center,
            vec![StyledLine {
                text: "x".repeat(50),
                highlights: vec![],
            }],
        );
        let r = Rect::new(f.rect.x, f.rect.y, f.rect.width, f.rect.height);
        FloatWidget::new(f).render(r, &mut buf);
        // The right border survives; nothing was written past it.
        assert_eq!(
            buf.cell((r.right() - 1, r.y + 1)).unwrap().symbol(),
            "\u{2502}"
        );
    }

    /// The key letter and its description are distinct colours: the key takes
    /// `whichkey_key` (its own accent), the description `whichkey_fg`.
    #[test]
    fn whichkey_widget_accentuates_the_key_letter() {
        use crate::widgets::WhichKeyWidget;
        let area = Rect::new(0, 0, 30, 3);
        let mut buf = RBuffer::empty(area);
        let view = ruster_render::WhichKeyView {
            title: "SPC".into(),
            rows: vec![
                ruster_render::WhichKeyEntry {
                    key: "h".into(),
                    desc: "windows".into(),
                },
                ruster_render::WhichKeyEntry {
                    key: "q".into(),
                    desc: "quit".into(),
                },
            ],
            anim: 1.0,
        };
        let theme = ruster_render::Theme {
            whichkey_fg: RColor::Rgb(200, 200, 200),
            whichkey_key: RColor::Rgb(10, 200, 10),
            ..ruster_render::Theme::default()
        };
        WhichKeyWidget::new(view)
            .with_theme(&theme)
            .render(area, &mut buf);
        let (k, d) = (buf.cell((2, 1)).unwrap(), buf.cell((5, 1)).unwrap());
        assert_eq!(k.symbol(), "h");
        assert_eq!(
            k.fg,
            ratatui::style::Color::Rgb(10, 200, 10),
            "key letter takes whichkey_key"
        );
        assert_eq!(d.symbol(), "w");
        assert_eq!(
            d.fg,
            ratatui::style::Color::Rgb(200, 200, 200),
            "description takes whichkey_fg"
        );
    }
}
