use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::Widget;
use ruster_core::vim::VimMode;
use ruster_render::{CursorKind, GutterView, StatuslineView, StyledLine, Color as RColor};

/// Convert a VimMode to a display string.
pub fn mode_label(mode: &VimMode) -> &'static str {
    match mode {
        VimMode::Normal => "-- NORMAL --",
        VimMode::Insert => "-- INSERT --",
        VimMode::VisualChar => "-- VISUAL --",
        VimMode::VisualLine => "-- V-LINE --",
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

/// Renders buffer text with cursor highlight and optional syntax highlighting.
///
/// Text is drawn starting from `scroll_offset` (the first visible buffer line)
/// and offset horizontally by the gutter width; the gutter's line-number column
/// is drawn on the left.
pub struct BufferWidget {
    lines: Vec<StyledLine>,
    cursor: (u16, u16),
    syntax: bool,
    cursor_visible: bool,
    cursor_kind: CursorKind,
    scroll_offset: u16,
    gutter: GutterView,
}

impl BufferWidget {
    pub fn new(lines: Vec<StyledLine>, cursor: (u16, u16)) -> Self {
        BufferWidget {
            lines,
            cursor,
            syntax: false,
            cursor_visible: true,
            cursor_kind: CursorKind::Block,
            scroll_offset: 0,
            gutter: GutterView::default(),
        }
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

        for (row, line) in self.lines.iter().skip(scroll).enumerate() {
            if row as u16 >= area.height { break; }
            let y = area.y + row as u16;
            let buffer_line = row + scroll;
            let is_cursor_line = buffer_line as u16 == self.cursor.0;
            let line_len = line.text.chars().count() as u16;
            for (j, ch) in line.text.chars().enumerate() {
                let x = text_x + j as u16;
                if x >= area.right() { break; }
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    if is_cursor_line && j as u16 == self.cursor.1 && self.cursor_visible {
                        apply_cursor(cell, self.cursor_kind);
                    } else if let Some((fg, bg)) = style_map.get(&(row as u16, j as u16)) {
                        cell.set_fg(ruster_render_color_to_tui(fg));
                        if !matches!(bg, RColor::Default) {
                            cell.set_bg(ruster_render_color_to_tui(bg));
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

#[cfg(test)]
mod tests {
    use crate::widgets::{cmdline_label, mode_label};
    use ruster_core::vim::VimMode;

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
