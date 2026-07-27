use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;
use ruster_render::{Color, FrameState, Renderer, SyntaxStyle, Theme};
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
    settings_scroll: usize,
    theme: Theme,
}

impl TuiRenderer {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let stdout = std::io::stdout();
        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        Ok(TuiRenderer { terminal: Some(terminal), settings_scroll: 0, theme: Theme::default() })
    }

    pub fn dummy() -> Self {
        TuiRenderer { terminal: None, settings_scroll: 0, theme: Theme::default() }
    }
}

impl Renderer for TuiRenderer {
    fn viewport_cells(&self) -> (u16, u16) {
        crossterm::terminal::size().unwrap_or((80, 24))
    }

    fn set_gui_config(&mut self, gui: &ruster_render::GuiConfig, _font: Option<&str>) {
        self.theme = gui.theme;
    }

    fn render_frame(&mut self, state: &FrameState) {
        let sscroll = &mut self.settings_scroll;
        let term = match &mut self.terminal {
            Some(t) => t,
            None => return,
        };
        let _ = term.draw(|frame| {
            let area = frame.area();
            let panel_bg = ruster_color_to_ratatui(&self.theme.bg);
            let divider_color = ruster_color_to_ratatui(&self.theme.divider);
            let accent = ruster_color_to_ratatui(&self.theme.accent);

            for view in &state.windows {
                let buf_h = view.rect.height.saturating_sub(2);
                let hdr_area = Rect::new(view.rect.x, view.rect.y, view.rect.width, 1);
                let buf_area = Rect::new(view.rect.x, view.rect.y + 1, view.rect.width, buf_h);
                let sl_area = Rect::new(view.rect.x, view.rect.y + 1 + buf_h, view.rect.width, 1);

                // Panel header: a dark ruled line with the filename as a stencil label.
                let label = &view.header;
                let cap = if label.is_empty() { "untitled" } else { label };
                let hdr = format!("─ {} ─", cap);
                let w = hdr.chars().count().min(view.rect.width as usize) as u16;
                let hdr_fg = if view.active { accent } else { divider_color };
                for (i, ch) in hdr.chars().enumerate().take(w as usize) {
                    if let Some(cell) = frame.buffer_mut().cell_mut((hdr_area.x + i as u16, hdr_area.y)) {
                        cell.set_char(ch);
                        cell.set_fg(hdr_fg);
                        cell.set_bg(panel_bg);
                    }
                }
                for x in (hdr_area.x + w)..hdr_area.right() {
                    if let Some(cell) = frame.buffer_mut().cell_mut((x, hdr_area.y)) {
                        cell.set_char('─');
                        cell.set_fg(divider_color);
                        cell.set_bg(panel_bg);
                    }
                }

                if state.welcome.as_ref().is_some_and(|w| w.visible) {
                    let ww = crate::widgets::WelcomeWidget::new(
                        state.welcome.as_ref().unwrap().clone(),
                    ).with_theme(&self.theme);
                    frame.render_widget(ww, buf_area);
                } else if let Some(grid) = &view.terminal {
                    let term_widget = crate::widgets::TerminalWidget::new(grid.clone())
                        .with_cursor_visible(view.cursor_visible && view.active);
                    frame.render_widget(term_widget, buf_area);
                } else {
                    let has_highlights = view.lines.iter().any(|l| !l.highlights.is_empty());
                    let buf_widget = crate::widgets::BufferWidget::new(view.lines.clone(), view.cursor)
                        .with_syntax(has_highlights)
                        .with_cursor_visible(view.cursor_visible)
                        .with_cursor_kind(view.cursor_kind)
                        .with_scroll(view.scroll_offset)
                        .with_gutter(view.gutter.clone())
                        .with_signs(view.signs.clone())
                        .with_extra_cursors(view.extra_cursors.clone())
                        .with_selection(view.selection);
                    frame.render_widget(buf_widget, buf_area);
                }

                let sl = crate::widgets::StatuslineWidget::new(view.statusline.clone())
                    .with_theme(&self.theme);
                frame.render_widget(sl, sl_area);
            }

            let cmd_text = state.cmdline.or(state.message);
            if let Some(text) = cmd_text {
                let cl_area = Rect::new(0, area.height.saturating_sub(1), area.width, 1);
                let cmd = crate::widgets::CmdlineWidget::new(text)
                    .with_theme(&self.theme);
                frame.render_widget(cmd, cl_area);
            }

            if let Some(wk) = &state.whichkey {
                let full = wk.rows.len() as u16 + 1;
                let visible = ((full as f32) * wk.anim).round() as u16;
                if visible > 0 {
                    let h = visible.min(area.height);
                    let py = area.height.saturating_sub(h);
                    let parea = Rect::new(0, py, area.width, h);
                    frame.render_widget(crate::widgets::WhichKeyWidget::new(wk.clone()), parea);
                }
            }

            if let Some(lines) = &state.hover {
                if !lines.is_empty() {
                    let w = lines.iter().map(|l| l.text.chars().count()).max().unwrap_or(0) as u16 + 2;
                    let w = w.clamp(8, area.width.saturating_sub(2));
                    let h = (lines.len() as u16 + 1).min(area.height.saturating_sub(2));
                    let x = area.x + (area.width.saturating_sub(w)) / 2;
                    let y = area.y + 1;
                    frame.render_widget(
                        crate::widgets::HoverWidget::new(lines.clone()),
                        Rect::new(x, y, w, h),
                    );
                }
            }

            if let Some(picker) = &state.picker {
                let frac = if picker.preview.is_empty() { 6 } else { 9 };
                let pw = (area.width * frac / 10).clamp(20, area.width.saturating_sub(2));
                let rows = picker.rows.len() as u16 + 2;
                let rows = rows.max(picker.preview.len() as u16);
                let ph = rows.clamp(3, area.height.saturating_sub(2));
                let px = area.x + (area.width.saturating_sub(pw)) / 2;
                let py = area.y + (area.height.saturating_sub(ph)) / 2;
                let parea = Rect::new(px, py, pw, ph);
                frame.render_widget(crate::widgets::PickerWidget::new(picker.clone()), parea);
            }

            if let Some(settings) = &state.settings {
                let sw = (area.width * 8 / 10).clamp(30, area.width.saturating_sub(2));
                let sh = (area.height * 9 / 10).clamp(6, area.height.saturating_sub(1));
                let sx = area.x + (area.width.saturating_sub(sw)) / 2;
                let sy = area.y + (area.height.saturating_sub(sh)) / 2;
                let sarea = Rect::new(sx, sy, sw, sh);
                frame.render_widget(
                    crate::widgets::SettingsWidget::new(settings.clone(), sscroll),
                    sarea,
                );
            }
        });
    }
}
