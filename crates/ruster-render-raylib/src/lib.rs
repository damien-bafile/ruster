mod key;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use raylib::consts::KeyboardKey;
use raylib::prelude::*;
use ruster_render::{CursorKind, EditorState, Renderer};

const FONT_SIZE: i32 = 20;
const LINE_H: i32 = 24;
const PAD_X: i32 = 8;
const PAD_Y: i32 = 4;

pub struct RaylibRenderer {
    rl: RaylibHandle,
    thread: RaylibThread,
    font: WeakFont,
    char_w: f32,
    event_buffer: Vec<KeyEvent>,
}

impl RaylibRenderer {
    pub fn new(width: i32, height: i32, title: &str) -> Self {
        let (mut rl, thread) = raylib::init()
            .size(width, height)
            .title(title)
            .build();
        rl.set_target_fps(60);
        rl.set_exit_key(None);
        let font = rl.get_font_default();
        let char_w = font.measure_text("m", FONT_SIZE as f32, 1.0).x;
        RaylibRenderer { rl, thread, font, char_w, event_buffer: Vec::new() }
    }

    fn drain_raylib(&mut self) {
        let mut mods = KeyModifiers::empty();
        if self.rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT)
            || self.rl.is_key_down(KeyboardKey::KEY_RIGHT_SHIFT)
        {
            mods |= KeyModifiers::SHIFT;
        }
        if self.rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL)
            || self.rl.is_key_down(KeyboardKey::KEY_RIGHT_CONTROL)
        {
            mods |= KeyModifiers::CONTROL;
        }
        if self.rl.is_key_down(KeyboardKey::KEY_LEFT_ALT)
            || self.rl.is_key_down(KeyboardKey::KEY_RIGHT_ALT)
        {
            mods |= KeyModifiers::ALT;
        }
        if self.rl.is_key_down(KeyboardKey::KEY_LEFT_SUPER)
            || self.rl.is_key_down(KeyboardKey::KEY_RIGHT_SUPER)
        {
            mods |= KeyModifiers::SUPER;
        }

        while let Some(c) = self.rl.get_char_pressed() {
            if let Some(ch) = char::from_u32(c as u32) {
                self.event_buffer.push(KeyEvent::new(KeyCode::Char(ch), mods));
            }
        }

        while let Some(k) = self.rl.get_key_pressed() {
            if let Some(event) = key::map_raylib_key(k) {
                self.event_buffer.push(KeyEvent::new(event.code, mods));
            }
        }

        self.event_buffer.reverse();
    }
}

impl Renderer for RaylibRenderer {
    fn render_frame(&mut self, state: &EditorState) {
        let mut d = self.rl.begin_drawing(&self.thread);
        d.clear_background(Color::new(30, 30, 30, 255));

        let default_color = Color::new(205, 214, 244, 255);
        for (i, line) in state.lines.iter().enumerate() {
            let y = PAD_Y + i as i32 * LINE_H;
            let n = line.text.len();
            if n == 0 {
                continue;
            }

            let mut char_colors: Vec<Color> = vec![default_color; n];
            for &(offset, len, ref style) in &line.highlights {
                let fg = match style.fg {
                    ruster_render::Color::Rgb(r, g, b) => Color::new(r, g, b, 255),
                    ruster_render::Color::Default => default_color,
                };
                let end = (offset + len).min(n);
                for pos in offset..end {
                    char_colors[pos] = fg;
                }
            }

            let mut x_offset = PAD_X as f32;
            let mut pos = 0;
            while pos < n {
                let c = char_colors[pos];
                let start = pos;
                while pos < n && char_colors[pos] == c {
                    pos += 1;
                }
                d.draw_text_ex(
                    &self.font,
                    &line.text[start..pos],
                    Vector2::new(x_offset, y as f32),
                    FONT_SIZE as f32,
                    1.0,
                    c,
                );
                x_offset += self.char_w * (pos - start) as f32;
            }
        }

        if state.cursor_visible {
            let col = state.cursor.1 as i32;
            let line = state.cursor.0 as i32;
            let mut cx = PAD_X as f32 + col as f32 * self.char_w;
            let mut cy = PAD_Y + line * LINE_H;
            if let Some((dcx, dcy)) = state.cursor_smooth {
                cx += dcx * self.char_w;
                cy = (cy as f32 + dcy * LINE_H as f32) as i32;
            }
            let cx = cx as i32;
            match state.cursor_kind {
                CursorKind::Block => {
                    d.draw_rectangle(cx, cy, self.char_w as i32, LINE_H, Color::new(245, 224, 220, 200));
                }
                CursorKind::Bar => {
                    d.draw_rectangle(cx, cy, 2, LINE_H, Color::new(245, 224, 220, 255));
                }
            }
        }
    }

    fn poll_input(&mut self) -> Option<KeyEvent> {
        if self.event_buffer.is_empty() {
            self.drain_raylib();
        }
        self.event_buffer.pop()
    }

    fn should_close(&self) -> bool {
        self.rl.window_should_close()
    }
}
