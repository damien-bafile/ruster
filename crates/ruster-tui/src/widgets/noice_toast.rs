use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::Widget;
use ruster_render::Color as RColor;

fn to_tui(c: &RColor) -> Color {
    match c {
        RColor::Default => Color::Reset,
        RColor::Rgb(r, g, b) => Color::Rgb(*r, *g, *b),
    }
}

fn dim(c: Color, bg: Color, factor: f32) -> Color {
    match (c, bg) {
        (Color::Rgb(r, g, b), Color::Rgb(br, bg_, bb)) => {
            Color::Rgb(
                (r as f32 * factor + br as f32 * (1.0 - factor)) as u8,
                (g as f32 * factor + bg_ as f32 * (1.0 - factor)) as u8,
                (b as f32 * factor + bb as f32 * (1.0 - factor)) as u8,
            )
        }
        _ => c,
    }
}

fn parse_level(text: &str) -> (char, Color) {
    match text.chars().next() {
        Some('✓') => ('✓', Color::Rgb(80, 200, 120)),
        Some('⚠') => ('⚠', Color::Rgb(255, 180, 50)),
        Some('✗') => ('✗', Color::Rgb(255, 80, 80)),
        _ => (' ', Color::Rgb(137, 180, 250)),
    }
}

pub struct NoiceToast<'a> {
    text: &'a str,
    fade: f32,
    theme: Option<ruster_render::Theme>,
}

impl<'a> NoiceToast<'a> {
    pub fn new(text: &'a str) -> Self {
        NoiceToast { text, fade: 1.0, theme: None }
    }

    pub fn with_fade(mut self, fade: f32) -> Self {
        self.fade = fade.clamp(0.0, 1.0);
        self
    }

    pub fn with_theme(mut self, theme: &ruster_render::Theme) -> Self {
        self.theme = Some(*theme);
        self
    }
}

impl Widget for NoiceToast<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let default_theme = ruster_render::Theme::default();
        let theme = self.theme.as_ref().unwrap_or(&default_theme);
        let bg = to_tui(&theme.cmdline_bg);
        let fg = to_tui(&theme.cmdline_fg);
        let f = self.fade;

        let (icon, level_color) = parse_level(self.text);
        let has_icon = icon != ' ';

        let msg = if has_icon {
            let mut cs = self.text.chars();
            cs.next();
            cs.next();
            cs.as_str()
        } else {
            self.text
        };

        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, area.y)) {
                cell.set_char(' ');
                cell.set_bg(bg);
            }
        }

        if has_icon {
            let lc = if f < 1.0 { dim(level_color, bg, f) } else { level_color };
            if let Some(cell) = buf.cell_mut((area.x, area.y)) {
                cell.set_char(icon);
                cell.set_fg(lc);
                cell.set_bg(bg);
            }
        }

        let text_fg = if f < 1.0 { dim(fg, bg, f) } else { fg };
        let offset = if has_icon { 2u16 } else { 0u16 };
        for (i, ch) in msg.chars().enumerate() {
            let x = area.x + offset + i as u16;
            if x >= area.right() {
                break;
            }
            if let Some(cell) = buf.cell_mut((x, area.y)) {
                cell.set_char(ch);
                cell.set_fg(text_fg);
                cell.set_bg(bg);
            }
        }
    }
}
