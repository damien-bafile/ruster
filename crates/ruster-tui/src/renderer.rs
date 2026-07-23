use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;
use ruster_render::{Color, FrameState, Renderer, SyntaxStyle};
use std::io::Stdout;

fn ruster_color_to_ratatui(c: &Color) -> ratatui::style::Color {
    match c {
        Color::Default => ratatui::style::Color::Reset,
        Color::Rgb(r, g, b) => ratatui::style::Color::Rgb(*r, *g, *b),
    }
}

pub fn ruster_style_to_ratatui(s: &SyntaxStyle) -> ratatui::style::Style {
    let mut style = ratatui::style::Style::default()
        .fg(ruster_color_to_ratatui(&s.fg))
        .bg(ruster_color_to_ratatui(&s.bg));
    if s.bold { style = style.add_modifier(ratatui::style::Modifier::BOLD); }
    if s.italic { style = style.add_modifier(ratatui::style::Modifier::ITALIC); }
    style
}

pub struct TuiRenderer {
    terminal: Option<Terminal<CrosstermBackend<Stdout>>>,
}

impl TuiRenderer {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let stdout = std::io::stdout();
        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        Ok(TuiRenderer { terminal: Some(terminal) })
    }

    pub fn dummy() -> Self {
        TuiRenderer { terminal: None }
    }
}

impl Renderer for TuiRenderer {
    fn viewport_cells(&self) -> (u16, u16) {
        crossterm::terminal::size().unwrap_or((80, 24))
    }

    fn render_frame(&mut self, state: &FrameState) {
        let term = match &mut self.terminal {
            Some(t) => t,
            None => return,
        };
        let _ = term.draw(|frame| {
            let area = frame.area();

            for view in &state.windows {
                if view.rect.width == 0 || view.rect.height == 0 {
                    continue;
                }
                let buf_h = view.rect.height.saturating_sub(1);
                let buf_area = Rect::new(view.rect.x, view.rect.y, view.rect.width, buf_h);
                let sl_area = Rect::new(view.rect.x, view.rect.y + buf_h, view.rect.width, 1);

                let has_highlights = view.lines.iter().any(|l| !l.highlights.is_empty());
                let buf_widget = crate::widgets::BufferWidget::new(view.lines.clone(), view.cursor)
                    .with_syntax(has_highlights)
                    .with_cursor_visible(view.cursor_visible)
                    .with_cursor_kind(view.cursor_kind)
                    .with_scroll(view.scroll_offset)
                    .with_gutter(view.gutter.clone());
                frame.render_widget(buf_widget, buf_area);

                let sl = crate::widgets::StatuslineWidget::new(view.statusline.clone());
                frame.render_widget(sl, sl_area);
            }

            // Shared cmdline / message on the bottom row.
            let cmd_text = state.cmdline.or(state.message);
            if let Some(text) = cmd_text {
                let cl_area = Rect::new(0, area.height.saturating_sub(1), area.width, 1);
                frame.render_widget(crate::widgets::CmdlineWidget::new(text), cl_area);
            }

            // Floating picker overlay, centered.
            if let Some(picker) = &state.picker {
                let pw = (area.width * 6 / 10).clamp(20, area.width.saturating_sub(2));
                let rows = picker.rows.len() as u16 + 2; // title + query + rows
                let ph = rows.clamp(3, area.height.saturating_sub(2));
                let px = area.x + (area.width.saturating_sub(pw)) / 2;
                let py = area.y + (area.height.saturating_sub(ph)) / 3;
                let parea = Rect::new(px, py, pw, ph);
                frame.render_widget(crate::widgets::PickerWidget::new(picker.clone()), parea);
            }
        });
    }
}
