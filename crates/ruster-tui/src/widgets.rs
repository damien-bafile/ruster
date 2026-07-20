use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::Widget;
use ruster_core::vim::VimMode;

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

/// Renders buffer text with cursor highlight.
pub struct BufferWidget {
    lines: Vec<String>,
    cursor: (u16, u16),
}

impl BufferWidget {
    pub fn new(lines: Vec<String>, cursor: (u16, u16)) -> Self {
        BufferWidget { lines, cursor }
    }
}

impl Widget for BufferWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        for (i, line) in self.lines.iter().enumerate() {
            if i as u16 >= area.height { break; }
            let y = area.y + i as u16;
            let is_cursor_line = i as u16 == self.cursor.0;
            for (j, ch) in line.chars().enumerate() {
                let x = area.x + j as u16;
                if x >= area.right() { break; }
                let cell = buf.get_mut(x, y);
                cell.set_char(ch);
                if is_cursor_line && j as u16 == self.cursor.1 {
                    cell.set_bg(Color::White);
                    cell.set_fg(Color::Black);
                }
            }
        }
    }
}

/// Renders the status line (mode, file path, cursor position).
pub struct StatuslineWidget<'a> {
    mode: &'a str,
    file_path: &'a str,
    position: (u16, u16),
}

impl<'a> StatuslineWidget<'a> {
    pub fn new(mode: &'a str, file_path: &'a str, position: (u16, u16)) -> Self {
        StatuslineWidget { mode, file_path, position }
    }
}

impl Widget for StatuslineWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let right = format!(" {},{} ", self.position.0 + 1, self.position.1 + 1);
        let left = format!(" {} ", self.mode);
        // Fill the full line with inverted bg
        for x in area.left()..area.right() {
            buf.get_mut(x, area.y).set_bg(Color::DarkGray);
        }
        // Write left (mode)
        for (i, ch) in left.chars().enumerate() {
            let x = area.x + i as u16;
            if x >= area.right() { break; }
            let cell = buf.get_mut(x, area.y);
            cell.set_char(ch);
            cell.set_fg(Color::White);
            cell.set_bg(Color::DarkGray);
        }
        // Write right (position)
        let rstart = area.right().saturating_sub(right.len() as u16);
        for (i, ch) in right.chars().enumerate() {
            let x = rstart + i as u16;
            if x >= area.right() { break; }
            let cell = buf.get_mut(x, area.y);
            cell.set_char(ch);
            cell.set_fg(Color::White);
            cell.set_bg(Color::DarkGray);
        }
        // Write file path in the center gap
        let max_path = rstart.saturating_sub(left.len() as u16 + 1);
        let path = &self.file_path[..self.file_path.len().min(max_path as usize)];
        for (i, ch) in path.chars().enumerate() {
            let x = area.x + left.len() as u16 + 1 + i as u16;
            if x >= area.right() { break; }
            let cell = buf.get_mut(x, area.y);
            cell.set_char(ch);
            cell.set_fg(Color::White);
            cell.set_bg(Color::DarkGray);
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
            buf.get_mut(x, area.y).set_char(ch);
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
