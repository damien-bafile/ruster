use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use ruster_render::{Color, EditorState, Renderer, SyntaxStyle};
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
    fn render_frame(&mut self, state: &EditorState) {
        let term = match &mut self.terminal {
            Some(t) => t,
            None => return,
        };
        let _ = term.draw(|frame| {
            let area = frame.area();
            let has_cmdline = state.cmdline.is_some() || state.message.is_some();
            let constraints: Vec<ratatui::layout::Constraint> = if has_cmdline {
                vec![
                    ratatui::layout::Constraint::Fill(1),
                    ratatui::layout::Constraint::Length(1),
                    ratatui::layout::Constraint::Length(1),
                ]
            } else {
                vec![
                    ratatui::layout::Constraint::Fill(1),
                    ratatui::layout::Constraint::Length(1),
                ]
            };
            let chunks = ratatui::layout::Layout::default()
                .direction(ratatui::layout::Direction::Vertical)
                .constraints(constraints)
                .split(area);

            // Buffer area
            let has_highlights = state.lines.iter().any(|l| !l.highlights.is_empty());
            let buf_widget = crate::widgets::BufferWidget::new(
                state.lines.clone(),
                state.cursor,
            )
            .with_syntax(has_highlights)
            .with_cursor_visible(state.cursor_visible)
            .with_cursor_kind(state.cursor_kind);
            frame.render_widget(buf_widget, chunks[0]);

            // Statusline
            let sl = crate::widgets::StatuslineWidget::new(
                state.mode_label,
                state.file_path,
                state.cursor,
            );
            frame.render_widget(sl, chunks[1]);

            // Cmdline / message area
            if let Some(cmd) = state.cmdline {
                let cl = crate::widgets::CmdlineWidget::new(cmd);
                frame.render_widget(cl, chunks.last().copied().unwrap_or(chunks[1]));
            } else if let Some(msg) = state.message {
                let cl = crate::widgets::CmdlineWidget::new(msg);
                frame.render_widget(cl, chunks.last().copied().unwrap_or(chunks[1]));
            }
        });
    }
}
