use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use ruster_render::{EditorState, Renderer};
use std::io::Stdout;

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
    fn render_frame(&mut self, _state: &EditorState) {
        let term = match &mut self.terminal {
            Some(t) => t,
            None => return,
        };
        let _ = term.draw(|frame| {
            frame.render_widget(ratatui::widgets::Clear, frame.area());
        });
    }
}
