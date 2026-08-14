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
    if s.bold {
        style = style.add_modifier(ratatui::style::Modifier::BOLD);
    }
    if s.italic {
        style = style.add_modifier(ratatui::style::Modifier::ITALIC);
    }
    style
}

pub struct TuiRenderer {
    terminal: Option<Terminal<CrosstermBackend<Stdout>>>,
    settings_scroll: usize,
    picker_scroll: usize,
}

impl TuiRenderer {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let stdout = std::io::stdout();
        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        Ok(TuiRenderer {
            terminal: Some(terminal),
            settings_scroll: 0,
            picker_scroll: 0,
        })
    }

    pub fn dummy() -> Self {
        TuiRenderer {
            terminal: None,
            settings_scroll: 0,
            picker_scroll: 0,
        }
    }
}

impl Renderer for TuiRenderer {
    fn viewport_cells(&self) -> (u16, u16) {
        crossterm::terminal::size().unwrap_or((80, 24))
    }

    fn set_gui_config(&mut self, _gui: &ruster_render::GuiConfig, _font: Option<&str>) {
        // Theme is now consumed from FrameState, no need to store locally.
    }

    fn render_frame(&mut self, state: &FrameState) {
        let sscroll = &mut self.settings_scroll;
        let pscroll = &mut self.picker_scroll;
        let term = match &mut self.terminal {
            Some(t) => t,
            None => return,
        };
        let _ = term.draw(|frame| {
            let area = frame.area();
            let panel_bg = ruster_color_to_ratatui(&state.theme.bg);
            let divider_color = ruster_color_to_ratatui(&state.theme.divider);
            let accent = ruster_color_to_ratatui(&state.theme.accent);

            // Above the windows, whose area the app already shortened by this
            // row — so this paints its own strip rather than over anything.
            if let Some(bl) = &state.bufferline {
                let r = Rect::new(bl.rect.x, bl.rect.y, bl.rect.width, bl.rect.height);
                frame.render_widget(
                    crate::widgets::BufferlineWidget::new(bl.clone()).with_theme(&state.theme),
                    r,
                );
            }

            for view in &state.windows {
                let buf_h = view.rect.height.saturating_sub(2);
                let hdr_area = Rect::new(view.rect.x, view.rect.y, view.rect.width, 1);
                let buf_area = Rect::new(view.rect.x, view.rect.y + 1, view.rect.width, buf_h);
                let sl_area = Rect::new(view.rect.x, view.rect.y + 1 + buf_h, view.rect.width, 1);

                // Panel header: a dark ruled line with the filename as a stencil
                // label. Shared with the picker and settings page so every
                // titled surface reads the same.
                let label = &view.header;
                let cap = if label.is_empty() { "untitled" } else { label };
                let hdr_fg = if view.active { accent } else { divider_color };
                crate::widgets::ruled_header(
                    frame.buffer_mut(),
                    hdr_area,
                    cap,
                    hdr_fg,
                    divider_color,
                    panel_bg,
                );

                if state.welcome.as_ref().is_some_and(|w| w.visible) {
                    let ww =
                        crate::widgets::WelcomeWidget::new(state.welcome.as_ref().unwrap().clone())
                            .with_theme(&state.theme);
                    frame.render_widget(ww, buf_area);
                } else if let Some(grid) = &view.terminal {
                    let term_widget = crate::widgets::TerminalWidget::new(grid.clone())
                        .with_cursor_visible(view.cursor_visible && view.active)
                        .with_theme(&state.theme);
                    frame.render_widget(term_widget, buf_area);
                } else {
                    let has_highlights = view.lines.iter().any(|l| !l.highlights.is_empty());
                    let buf_widget =
                        crate::widgets::BufferWidget::new(view.lines.clone(), view.cursor)
                            .with_syntax(has_highlights)
                            .with_cursor_visible(view.cursor_visible)
                            .with_cursor_kind(view.cursor_kind)
                            .with_scroll(view.scroll_offset)
                            .with_gutter(view.gutter.clone())
                            .with_signs(view.signs.clone())
                            .with_extra_cursors(view.extra_cursors.clone())
                            .with_selection(view.selection)
                            .with_active(view.active)
                            .with_theme(&state.theme);
                    frame.render_widget(buf_widget, buf_area);
                }

                // Flash jump labels overlay. Labels sit on top of the buffer
                // text, so they share the layout every backend and the mouse
                // hit-test use.
                let text_area =
                    ruster_render::TextArea::of(view.rect, view.signs.width, view.gutter.width);
                for fl in &view.flash_labels {
                    if fl.row >= text_area.height {
                        continue;
                    }
                    let y = text_area.y + fl.row;
                    let x = text_area.x + fl.col;
                    // Clip labels that would spill past the window's right edge
                    // into a neighbouring split.
                    let w = fl.text.chars().count() as u16;
                    if x + w > text_area.right() {
                        continue;
                    }
                    let fg_color = ruster_color_to_ratatui(&fl.color);
                    let bg_color = ratatui::style::Color::Indexed(214);
                    let style = ratatui::style::Style::default().fg(fg_color).bg(bg_color);
                    let area = Rect::new(x, y, w, 1);
                    let line =
                        ratatui::text::Line::from(ratatui::text::Span::styled(&fl.text, style));
                    frame.render_widget(ratatui::widgets::Paragraph::new(line), area);
                }

                let sl = crate::widgets::StatuslineWidget::new(view.statusline.clone())
                    .with_theme(&state.theme);
                frame.render_widget(sl, sl_area);
            }

            if let Some(text) = state.cmdline {
                let cl_area = Rect::new(0, area.height.saturating_sub(1), area.width, 1);
                let cmd = crate::widgets::CmdlineWidget::new(text).with_theme(&state.theme);
                frame.render_widget(cmd, cl_area);
            }
            for (i, text) in state.noice_mini.iter().enumerate() {
                let w = text.len() as u16;
                let x = area.width.saturating_sub(w + 2);
                // The GUI paints both of these with `theme.whichkey_bg`; the
                // TUI used to hardcode a near-but-not-equal grey, so a themed
                // GUI and an unthemed TUI disagreed. Same source now.
                let toast = crate::widgets::noice_toast::NoiceToast::new(
                    text,
                    ruster_color_to_ratatui(&state.theme.whichkey_bg),
                    ratatui::style::Color::White,
                );
                frame.render_widget(toast, Rect::new(x, 1 + i as u16, w, 1));
            }

            if let Some(notify_lines) = &state.noice_notify {
                let panel_width = 40.min(area.width / 3);
                let panel_area = Rect::new(
                    area.width.saturating_sub(panel_width),
                    0,
                    panel_width,
                    area.height,
                );
                let bg = ratatui::style::Style::default()
                    .bg(ruster_color_to_ratatui(&state.theme.whichkey_bg));
                frame.render_widget(ratatui::widgets::Clear, panel_area);
                for (i, line) in notify_lines.iter().enumerate() {
                    if i as u16 >= panel_area.height {
                        break;
                    }
                    frame.render_widget(
                        ratatui::widgets::Paragraph::new(ratatui::text::Line::from(
                            ratatui::text::Span::styled(&line.text, bg),
                        )),
                        Rect::new(panel_area.x, panel_area.y + i as u16, panel_width, 1),
                    );
                }
            }

            if let Some(wk) = &state.whichkey {
                let full = wk.rows.len() as u16 + 1;
                let visible = ((full as f32) * wk.anim).round() as u16;
                if visible > 0 {
                    let h = visible.min(area.height);
                    let py = area.height.saturating_sub(h);
                    let parea = Rect::new(0, py, area.width, h);
                    frame.render_widget(
                        crate::widgets::WhichKeyWidget::new(wk.clone()).with_theme(&state.theme),
                        parea,
                    );
                }
            }

            // Picker overlay: a centered floating box, or a full-width strip
            // docked at the bottom (the command palette's "bottom" mode).
            if let Some(picker) = &state.picker {
                // +3: title row, query row and the box's bottom edge.
                let rows = (picker.rows.len() as u16 + 3).max(picker.preview.len() as u16 + 2);
                let parea = if picker.placement == ruster_render::PickerPlacement::Bottom {
                    let ph = rows
                        .clamp(3, area.height.saturating_sub(1))
                        .min(area.height / 2);
                    Rect::new(area.x, area.height.saturating_sub(ph), area.width, ph)
                } else {
                    let frac = if picker.preview.is_empty() { 6 } else { 9 };
                    let pw = (area.width * frac / 10).clamp(20, area.width.saturating_sub(2));
                    let ph = rows.clamp(3, area.height.saturating_sub(2));
                    let px = area.x + (area.width.saturating_sub(pw)) / 2;
                    let py = area.y + (area.height.saturating_sub(ph)) / 2;
                    Rect::new(px, py, pw, ph)
                };
                // Keep the selection on screen; a wrap to the last item has to
                // take the view with it.
                let vis = parea.height.saturating_sub(3) as usize;
                let sel = picker.rows.iter().position(|r| r.selected).unwrap_or(0);
                *pscroll = ruster_render::list_scroll(*pscroll, sel, vis, picker.rows.len());
                frame.render_widget(
                    crate::widgets::PickerWidget::new(picker.clone())
                        .with_theme(&state.theme)
                        .with_scroll(*pscroll),
                    parea,
                );
            }

            // Debugger panel, docked right so it doesn't cover the stopped line.
            if let Some(dbg) = &state.debug_overlay {
                let dw = 44.min(area.width / 2).max(1);
                let dh = (dbg.rows().len() as u16 + 1).min(area.height);
                let darea = Rect::new(area.width.saturating_sub(dw), area.y, dw, dh);
                frame.render_widget(ratatui::widgets::Clear, darea);
                frame.render_widget(
                    crate::widgets::debug_overlay::DebugOverlayWidget::new(dbg)
                        .with_theme(&state.theme),
                    darea,
                );
            }

            if let Some(settings) = &state.settings {
                let sw = (area.width * 8 / 10).clamp(30, area.width.saturating_sub(2));
                let sh = (area.height * 9 / 10).clamp(6, area.height.saturating_sub(1));
                let sx = area.x + (area.width.saturating_sub(sw)) / 2;
                let sy = area.y + (area.height.saturating_sub(sh)) / 2;
                let sarea = Rect::new(sx, sy, sw, sh);
                frame.render_widget(
                    crate::widgets::SettingsWidget::new(settings.clone(), sscroll)
                        .with_theme(&state.theme),
                    sarea,
                );
            }

            // Floats, then the dialog: a modal is the thing with focus, so it
            // draws last and obscures anything under it. Both backends had this
            // the other way round while their comments claimed otherwise —
            // inert, since the only float is the hover popup and it never
            // coexists with a dialog, but they disagreed and one had to be wrong.
            //
            // Lowest z first.
            for f in ruster_render::floats_in_draw_order(&state.floats) {
                let r = Rect::new(f.rect.x, f.rect.y, f.rect.width, f.rect.height);
                frame.render_widget(
                    crate::widgets::FloatWidget::new(f.clone()).with_theme(&state.theme),
                    r,
                );
            }

            if let Some(dlg) = &state.dialog {
                let dw = (area.width * 6 / 10).clamp(30, area.width.saturating_sub(4));
                let dh = (dlg.rows.len() as u16 + 4).min(area.height.saturating_sub(2));
                let dx = area.x + (area.width.saturating_sub(dw)) / 2;
                let dy = area.y + (area.height.saturating_sub(dh)) / 2;
                frame.render_widget(
                    crate::widgets::DialogWidget::new(dlg.clone()).with_theme(&state.theme),
                    Rect::new(dx, dy, dw, dh),
                );
            }
        });
    }
}
