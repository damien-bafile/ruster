use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::Widget;
use ruster_render::{DebugOverlayView, Theme};

use crate::widgets::ruster_render_color_to_tui;

/// The debugger panel: a highlighted toolbar row, then the call stack and the
/// variables of each visible scope. Docked to the right so it does not cover the
/// line the debugger is stopped on.
pub struct DebugOverlayWidget<'a> {
    view: &'a DebugOverlayView,
    theme: Option<&'a Theme>,
}

impl<'a> DebugOverlayWidget<'a> {
    pub fn new(view: &'a DebugOverlayView) -> Self {
        DebugOverlayWidget { view, theme: None }
    }

    pub fn with_theme(mut self, theme: &'a Theme) -> Self {
        self.theme = Some(theme);
        self
    }
}

impl Widget for DebugOverlayWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let c = |pick: fn(&Theme) -> ruster_render::Color, fallback: Color| {
            self.theme
                .map(|t| ruster_render_color_to_tui(&pick(t)))
                .unwrap_or(fallback)
        };
        let panel_bg = c(|t| t.whichkey_bg, Color::Rgb(30, 30, 46));
        let panel_fg = c(|t| t.whichkey_fg, Color::Rgb(205, 214, 244));
        let bar_bg = c(|t| t.accent, Color::Rgb(243, 139, 168));
        let bar_fg = c(|t| t.accent_fg, Color::Rgb(30, 30, 30));
        let dim = c(|t| t.gutter, Color::DarkGray);

        let put = |buf: &mut Buffer, y: u16, text: &str, fg: Color, bg: Color| {
            for x in area.left()..area.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(' ');
                    cell.set_bg(bg);
                }
            }
            for (i, ch) in text.chars().enumerate() {
                let x = area.x + i as u16;
                if x >= area.right() {
                    break;
                }
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    cell.set_fg(fg);
                    cell.set_bg(bg);
                }
            }
        };

        put(buf, area.y, &self.view.toolbar, bar_fg, bar_bg);

        for (i, row) in self.view.rows().iter().enumerate() {
            let y = area.y + 1 + i as u16;
            if y >= area.bottom() {
                break;
            }
            // Section headings (call stack, scope names) are unindented; dim the
            // detail rows so the structure reads at a glance.
            let fg = if row.starts_with(' ') || row.starts_with(|c: char| c.is_ascii_digit()) {
                dim
            } else {
                panel_fg
            };
            put(buf, y, row, fg, panel_bg);
        }

        // Fill any remaining rows so the panel reads as one block.
        for y in (area.y + 1 + self.view.rows().len() as u16).min(area.bottom())..area.bottom() {
            put(buf, y, "", panel_fg, panel_bg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> DebugOverlayView {
        DebugOverlayView {
            toolbar: "[Debug: PAUSED] F5:Continue".into(),
            stack: vec![(0, "main".into(), "src/main.rs:12".into())],
            scopes: vec![("Locals".into(), vec![("count".into(), "3".into())])],
        }
    }

    /// Read one row of a rendered buffer back as text.
    fn row(buf: &Buffer, y: u16, area: Rect) -> String {
        (area.left()..area.right())
            .map(|x| {
                buf.cell((x, y))
                    .map(|c| c.symbol().to_string())
                    .unwrap_or_default()
            })
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn draws_toolbar_then_stack_then_scope() {
        let area = Rect::new(0, 0, 44, 10);
        let mut buf = Buffer::empty(area);
        DebugOverlayWidget::new(&view()).render(area, &mut buf);

        assert!(
            row(&buf, 0, area).contains("Debug: PAUSED"),
            "toolbar on the first row"
        );
        assert_eq!(row(&buf, 1, area), "Call stack");
        assert!(row(&buf, 2, area).contains("main") && row(&buf, 2, area).contains("main.rs:12"));
        assert_eq!(row(&buf, 4, area), "Locals");
        assert!(row(&buf, 5, area).contains("count = 3"));
    }

    /// A panel shorter than its content must clip, not index out of the buffer.
    #[test]
    fn clips_to_a_short_panel() {
        let area = Rect::new(0, 0, 20, 2);
        let mut buf = Buffer::empty(area);
        DebugOverlayWidget::new(&view()).render(area, &mut buf);
        assert!(row(&buf, 0, area).contains("Debug"));
        assert_eq!(row(&buf, 1, area), "Call stack");
    }

    #[test]
    fn zero_sized_area_is_a_no_op() {
        let area = Rect::new(0, 0, 0, 0);
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 1));
        DebugOverlayWidget::new(&view()).render(area, &mut buf);
    }
}
