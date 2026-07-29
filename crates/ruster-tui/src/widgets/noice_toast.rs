use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::Widget;

fn dim(c: Color, bg: Color, factor: f32) -> Color {
    let lerp = |a: u8, b: u8| (a as f32 * factor + b as f32 * (1.0 - factor)) as u8;
    match (c, bg) {
        (Color::Rgb(_, _, _), _) if factor >= 1.0 => c,
        (Color::Rgb(r, g, b), Color::Rgb(br, _, _)) => Color::Rgb(lerp(r, br), lerp(g, 0), lerp(b, 0)),
        (Color::Rgb(r, g, b), _) => Color::Rgb(lerp(r, 0), lerp(g, 0), lerp(b, 0)),
        _ => c,
    }
}

pub struct NoiceToast<'a> {
    text: &'a str,
    fade: f32,
    bg: Color,
    fg: Color,
}

impl<'a> NoiceToast<'a> {
    pub fn new(text: &'a str, bg: Color, fg: Color) -> Self {
        NoiceToast { text, fade: 1.0, bg, fg }
    }

    pub fn with_fade(mut self, fade: f32) -> Self {
        self.fade = fade.clamp(0.0, 1.0);
        self
    }
}

impl Widget for NoiceToast<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let bg = self.bg;
        let fg = if self.fade < 1.0 { dim(self.fg, bg, self.fade) } else { self.fg };

        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, area.y)) {
                cell.set_char(' ');
                cell.set_bg(bg);
            }
        }

        for (i, ch) in self.text.chars().enumerate() {
            let x = area.x + i as u16;
            if x >= area.right() {
                break;
            }
            if let Some(cell) = buf.cell_mut((x, area.y)) {
                cell.set_char(ch);
                cell.set_fg(fg);
                cell.set_bg(bg);
            }
        }
    }
}
