use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use ratatui::widgets::Widget;
use ruster_core::vim::VimMode;
use ruster_render::{
    ControlKind, CursorKind, GutterView, SettingRowView, SettingsView, StatuslineView, StyledLine,
    TermGridView, Color as RColor,
};

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
    if buf.is_empty() { ":".to_string() } else { buf.to_string() }
}

fn apply_cursor(cell: &mut ratatui::buffer::Cell, kind: CursorKind) {
    match kind {
        CursorKind::Bar => {
            cell.set_bg(Color::DarkGray);
            cell.set_fg(Color::White);
        }
        CursorKind::Block => {
            cell.set_bg(Color::White);
            cell.set_fg(Color::Black);
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
}

impl TerminalWidget {
    pub fn new(grid: TermGridView) -> Self {
        TerminalWidget { grid, cursor_visible: true }
    }

    pub fn with_cursor_visible(mut self, visible: bool) -> Self {
        self.cursor_visible = visible;
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
                    apply_cursor(cell, CursorKind::Block);
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
    selection: Option<ruster_render::SelectionView>,
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
            selection: None,
        }
    }

    pub fn with_extra_cursors(mut self, extra: Vec<(u16, u16)>) -> Self {
        self.extra_cursors = extra;
        self
    }

    pub fn with_selection(mut self, selection: Option<ruster_render::SelectionView>) -> Self {
        self.selection = selection;
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
}

impl Widget for BufferWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let gutter_w = self.gutter.width.min(area.width);
        let text_x = area.x + gutter_w;
        let scroll = self.scroll_offset as usize;

        // Gutter column.
        for (row, label) in self.gutter.rows.iter().enumerate() {
            if row as u16 >= area.height { break; }
            let y = area.y + row as u16;
            // Right-align within the gutter width (labels already padded to fit).
            let start = gutter_w.saturating_sub(label.chars().count() as u16);
            for (i, ch) in label.chars().enumerate() {
                let x = area.x + start + i as u16;
                if x >= text_x { break; }
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    cell.set_fg(Color::DarkGray);
                }
            }
        }

        let mut style_map: std::collections::HashMap<(u16, u16), (RColor, RColor)> =
            std::collections::HashMap::new();
        if self.syntax {
            for (row, line) in self.lines.iter().skip(scroll).enumerate() {
                let y = row as u16;
                if y >= area.height { break; }
                for (offset, length, style) in &line.highlights {
                    for c in 0..*length {
                        let x = (offset + c) as u16;
                        style_map.insert((y, x), (style.fg, style.bg));
                    }
                }
            }
        }

        let selection_bg = Color::Rgb(88, 91, 112);
        for (row, line) in self.lines.iter().skip(scroll).enumerate() {
            if row as u16 >= area.height { break; }
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
                    if x >= area.right() { break; }
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_bg(selection_bg);
                    }
                }
            }
            for (j, ch) in line.text.chars().enumerate() {
                let x = text_x + j as u16;
                if x >= area.right() { break; }
                let selected = sel_span
                    .map(|(s, e)| j as u16 >= s && j as u16 <= e)
                    .unwrap_or(false);
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    if is_cursor_line && j as u16 == self.cursor.1 && self.cursor_visible {
                        apply_cursor(cell, self.cursor_kind);
                    } else {
                        if let Some((fg, bg)) = style_map.get(&(row as u16, j as u16)) {
                            cell.set_fg(ruster_render_color_to_tui(fg));
                            if !matches!(bg, RColor::Default) {
                                cell.set_bg(ruster_render_color_to_tui(bg));
                            }
                        }
                        if selected {
                            cell.set_bg(selection_bg);
                        }
                    }
                }
            }
            if is_cursor_line && self.cursor_visible && self.cursor.1 >= line_len {
                let x = text_x + self.cursor.1;
                if x < area.right() {
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_char(' ');
                        apply_cursor(cell, self.cursor_kind);
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
                apply_cursor(cell, self.cursor_kind);
            }
        }
    }
}

/// Renders one window's statusline from a [`StatuslineView`] (left / center /
/// right groups). The active window's statusline is brighter than inactive ones.
pub struct StatuslineWidget {
    view: StatuslineView,
}

impl StatuslineWidget {
    pub fn new(view: StatuslineView) -> Self {
        StatuslineWidget { view }
    }
}

impl Widget for StatuslineWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let bg = if self.view.active { Color::DarkGray } else { Color::Rgb(35, 35, 35) };
        let fg = if self.view.active { Color::White } else { Color::Gray };

        let put = |buf: &mut Buffer, x: u16, ch: char| {
            if x >= area.left() && x < area.right() {
                if let Some(cell) = buf.cell_mut((x, area.y)) {
                    cell.set_char(ch);
                    cell.set_fg(fg);
                    cell.set_bg(bg);
                }
            }
        };

        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, area.y)) {
                cell.set_bg(bg);
            }
        }

        let left = format!(" {} ", self.view.left);
        for (i, ch) in left.chars().enumerate() {
            put(buf, area.x + i as u16, ch);
        }

        let right = format!(" {} ", self.view.right);
        let rstart = area.right().saturating_sub(right.chars().count() as u16);
        for (i, ch) in right.chars().enumerate() {
            put(buf, rstart + i as u16, ch);
        }

        // Center group: placed after the left group, clipped before the right.
        let center_start = area.x + left.chars().count() as u16 + 1;
        let center_limit = rstart.saturating_sub(1);
        for (i, ch) in self.view.center.chars().enumerate() {
            let x = center_start + i as u16;
            if x >= center_limit { break; }
            put(buf, x, ch);
        }
    }
}

/// Renders the cmdline prompt line.
pub struct CmdlineWidget<'a> {
    text: &'a str,
}

impl<'a> CmdlineWidget<'a> {
    pub fn new(text: &'a str) -> Self {
        CmdlineWidget { text }
    }
}

impl Widget for CmdlineWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        for (i, ch) in self.text.chars().enumerate() {
            let x = area.x + i as u16;
            if x >= area.right() { break; }
            if let Some(cell) = buf.cell_mut((x, area.y)) {
                cell.set_char(ch);
            }
        }
    }
}

/// Renders a floating picker overlay (title, query line, selectable rows).
pub struct PickerWidget {
    view: ruster_render::PickerView,
}

impl PickerWidget {
    pub fn new(view: ruster_render::PickerView) -> Self {
        PickerWidget { view }
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
    match row.kind {
        ControlKind::Toggle => {
            if row.value == "on" { "[x] on".to_string() } else { "[ ] off".to_string() }
        }
        ControlKind::Enum => format!("< {} >", row.value),
        ControlKind::Number | ControlKind::Text => {
            if row.editing {
                format!("{}▏", row.value)
            } else {
                row.value.clone()
            }
        }
    }
}

/// One rendered line of the settings body: a group header or a setting row.
enum SettingsLine<'a> {
    Header(&'a str),
    Row(&'a SettingRowView),
}

pub struct SettingsWidget {
    view: SettingsView,
}

impl SettingsWidget {
    pub fn new(view: SettingsView) -> Self {
        SettingsWidget { view }
    }
}

impl Widget for SettingsWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let bg = Color::Rgb(30, 30, 46);
        let accent = Color::Rgb(137, 180, 250);
        let fg = Color::Rgb(205, 214, 244);
        let dim = Color::Rgb(127, 132, 156);
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

        // Title row.
        let title = format!(" Settings{} ", if self.view.dirty { " [+]" } else { "" });
        for x in area.left()..area.right() {
            put(buf, x, area.y, ' ', Color::Black, accent);
        }
        text(buf, area.x + 1, area.y, &title, Color::Black, accent);

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

        // Scroll so the selected line stays visible (body between title/footer).
        let body_top = area.y + 1;
        let body_h = area.height.saturating_sub(3) as usize; // title + footer + help
        let scroll = selected.saturating_sub(body_h.saturating_sub(1)).min(lines.len());
        let value_col = area.x + 32.min(area.width / 2);

        for (i, line) in lines.iter().skip(scroll).take(body_h).enumerate() {
            let y = body_top + i as u16;
            match line {
                SettingsLine::Header(name) => {
                    text(buf, area.x + 1, y, &format!("── {} ", name.to_uppercase()), accent, bg);
                }
                SettingsLine::Row(r) => {
                    let (row_fg, row_bg) = if r.selected {
                        for x in area.left()..area.right() {
                            put(buf, x, y, ' ', Color::Black, Color::Rgb(69, 71, 90));
                        }
                        (fg, Color::Rgb(69, 71, 90))
                    } else {
                        (fg, bg)
                    };
                    text(buf, area.x + 2, y, &r.label, row_fg, row_bg);
                    let ctrl = control_display(r);
                    let cfg = if r.editing { accent } else { row_fg };
                    text(buf, value_col, y, &ctrl, cfg, row_bg);
                    // A swatch after a hex color value.
                    if let Some((cr, cg, cb)) = hex_rgb(&r.value) {
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
            put(buf, x, fy, ' ', dim, Color::Rgb(24, 24, 37));
        }
        text(buf, area.x + 1, fy, &self.view.footer, dim, Color::Rgb(24, 24, 37));

        // Selected row's help just above the footer.
        if let Some(SettingsLine::Row(r)) = lines.get(selected) {
            let hy = area.bottom().saturating_sub(2);
            text(buf, area.x + 1, hy, &r.help, dim, bg);
        }
    }
}

impl Widget for PickerWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let bg = Color::Rgb(30, 30, 46);
        let preview_bg = Color::Rgb(24, 24, 37);
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
        let list_w = if has_preview { area.width * 2 / 5 } else { area.width };
        let list_right = area.x + list_w;

        // Title row.
        let title = format!(" {} ", self.view.title);
        for (i, ch) in title.chars().enumerate() {
            put(buf, area.x + i as u16, area.y, ch, Color::Black, Color::Rgb(137, 180, 250));
        }
        // Query row.
        let query = format!(" > {}", self.view.query);
        for (i, ch) in query.chars().enumerate() {
            put(buf, area.x + i as u16, area.y + 1, ch, Color::White, bg);
        }
        // Item rows (clipped to the list column).
        for (row, item) in self.view.rows.iter().enumerate() {
            let y = area.y + 2 + row as u16;
            if y >= area.bottom() { break; }
            let (fg, row_bg) = if item.selected {
                (Color::Black, Color::Rgb(137, 180, 250))
            } else {
                (Color::Rgb(205, 214, 244), bg)
            };
            for x in area.left()..list_right.min(area.right()) {
                put(buf, x, y, ' ', fg, row_bg);
            }
            let label = format!(" {}", item.label);
            for (i, ch) in label.chars().enumerate() {
                let x = area.x + i as u16;
                if x >= list_right { break; }
                put(buf, x, y, ch, fg, row_bg);
            }
        }

        // Preview column: highlighted file contents.
        if has_preview {
            let px = list_right + 1;
            for y in area.top()..area.bottom() {
                for x in px..area.right() {
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_bg(preview_bg);
                    }
                }
                // Divider between the two columns.
                put(buf, list_right, y, '│', Color::Rgb(69, 71, 90), bg);
            }
            for (row, line) in self.view.preview.iter().enumerate() {
                let y = area.y + row as u16;
                if y >= area.bottom() { break; }
                let mut colors: std::collections::HashMap<usize, RColor> =
                    std::collections::HashMap::new();
                for (offset, len, style) in &line.highlights {
                    for c in 0..*len {
                        colors.insert(offset + c, style.fg);
                    }
                }
                for (i, ch) in line.text.chars().enumerate() {
                    let x = px + 1 + i as u16;
                    if x >= area.right() { break; }
                    let fg = colors
                        .get(&i)
                        .map(ruster_render_color_to_tui)
                        .unwrap_or(Color::Rgb(205, 214, 244));
                    put(buf, x, y, ch, fg, preview_bg);
                }
            }
        }
    }
}

/// Renders the bottom which-key panel (title on top, one binding per line).
/// The caller sizes `area` to the currently-visible height for the slide-up.
pub struct WhichKeyWidget {
    view: ruster_render::WhichKeyView,
}

impl WhichKeyWidget {
    pub fn new(view: ruster_render::WhichKeyView) -> Self {
        WhichKeyWidget { view }
    }
}

impl Widget for WhichKeyWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let bg = Color::Rgb(30, 30, 46);
        let accent = Color::Rgb(137, 180, 250);
        let fg = Color::Rgb(205, 214, 244);
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
        // One binding per line below the title.
        for (row, entry) in self.view.rows.iter().enumerate() {
            let y = area.y + 1 + row as u16;
            if y >= area.bottom() { break; }
            for (i, ch) in format!("  {}", entry).chars().enumerate() {
                put(buf, area.x + i as u16, y, ch, fg);
            }
        }
    }
}

/// Renders an LSP hover popup: a bordered box of syntax-highlighted lines.
pub struct HoverWidget {
    lines: Vec<StyledLine>,
}

impl HoverWidget {
    pub fn new(lines: Vec<StyledLine>) -> Self {
        HoverWidget { lines }
    }
}

impl Widget for HoverWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let bg = Color::Rgb(24, 24, 37);
        let default_fg = Color::Rgb(205, 214, 244);
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(' ');
                    cell.set_bg(bg);
                }
            }
        }
        for (row, line) in self.lines.iter().enumerate() {
            let y = area.y + row as u16;
            if y >= area.bottom() { break; }
            // Map char index -> highlight fg for this line.
            let mut colors: std::collections::HashMap<usize, RColor> = std::collections::HashMap::new();
            for (offset, len, style) in &line.highlights {
                for c in 0..*len {
                    colors.insert(offset + c, style.fg);
                }
            }
            for (i, ch) in line.text.chars().enumerate() {
                let x = area.x + 1 + i as u16;
                if x >= area.right() { break; }
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    let fg = colors
                        .get(&i)
                        .map(ruster_render_color_to_tui)
                        .unwrap_or(default_fg);
                    cell.set_fg(fg);
                    cell.set_bg(bg);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BufferWidget, TerminalWidget};
    use crate::widgets::{cmdline_label, mode_label};
    use ratatui::buffer::Buffer as RBuffer;
    use ratatui::layout::Rect;
    use ratatui::widgets::Widget;
    use ruster_core::vim::VimMode;
    use ruster_render::{Color as RColor, StyledLine, TermCellView, TermGridView};

    #[test]
    fn terminal_widget_draws_cells_colors_and_cursor() {
        let grid = TermGridView {
            cols: 3,
            rows: 1,
            cells: vec![
                TermCellView { c: 'h', fg: RColor::Rgb(10, 20, 30), ..TermCellView::default() },
                TermCellView { c: 'i', ..TermCellView::default() },
                TermCellView { c: '!', ..TermCellView::default() },
            ],
            cursor: (0, 1),
        };
        let area = Rect::new(0, 0, 3, 1);
        let mut buf = RBuffer::empty(area);
        TerminalWidget::new(grid).render(area, &mut buf);

        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "h");
        assert_eq!(buf.cell((0, 0)).unwrap().fg, ratatui::style::Color::Rgb(10, 20, 30));
        // The cursor cell (col 1) is painted as a block.
        assert_ne!(buf.cell((1, 0)).unwrap().bg, ratatui::style::Color::Reset);
        assert_eq!(buf.cell((2, 0)).unwrap().symbol(), "!");
    }

    #[test]
    fn extra_cursors_paint_a_block_over_their_cell() {
        let line = StyledLine { text: "abcd".to_string(), highlights: vec![] };
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = RBuffer::empty(area);
        // Primary at col 0, an extra caret at col 2 ('c').
        BufferWidget::new(vec![line], (0, 0))
            .with_extra_cursors(vec![(0, 2)])
            .render(area, &mut buf);
        // The extra caret reverses fg/bg on its cell; the char is preserved.
        let cell = buf.cell((2, 0)).unwrap();
        assert_eq!(cell.symbol(), "c");
        assert_ne!(cell.bg, ratatui::style::Color::Reset, "extra caret paints a block");
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
}
