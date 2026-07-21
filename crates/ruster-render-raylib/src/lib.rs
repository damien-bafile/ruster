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
        let screen_w = self.rl.get_screen_width();
        let screen_h = self.rl.get_screen_height();
        let mut d = self.rl.begin_drawing(&self.thread);
        d.clear_background(Color::new(30, 30, 30, 255));

        let has_cmdline = state.cmdline.is_some() || state.message.is_some();
        let status_h = if has_cmdline { 2 * LINE_H } else { LINE_H };
        let max_lines = (screen_h - PAD_Y - status_h) / LINE_H;

        let default_color = Color::new(205, 214, 244, 255);
        for (i, line) in state.lines.iter().enumerate().take(max_lines as usize) {
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

        // Statusline
        let status_y = screen_h - status_h;
        let sl_color = Color::new(205, 214, 244, 255);
        d.draw_rectangle(0, status_y, screen_w, status_h, Color::new(45, 45, 45, 255));

        // Left: mode label
        d.draw_text_ex(
            &self.font,
            state.mode_label,
            Vector2::new(PAD_X as f32, status_y as f32),
            FONT_SIZE as f32,
            1.0,
            sl_color,
        );

        // Right: cursor position (1-indexed)
        let right_str = format!(" ({},{}) ", state.cursor.0 + 1, state.cursor.1 + 1);
        let right_w = self.char_w * right_str.len() as f32;
        let right_x = screen_w as f32 - right_w - PAD_X as f32;
        d.draw_text_ex(
            &self.font,
            &right_str,
            Vector2::new(right_x, status_y as f32),
            FONT_SIZE as f32,
            1.0,
            sl_color,
        );

        // Center: file path (truncated to fit between left and right text)
        let left_w = self.char_w * state.mode_label.len() as f32;
        let gap = screen_w as f32 - left_w - right_w - 3.0 * PAD_X as f32;
        let center_str = if gap > 0.0 && state.file_path.len() as f32 * self.char_w > gap {
            let max_chars = (gap / self.char_w) as usize;
            if max_chars > 3 {
                let mut s = String::from("...");
                s.push_str(&state.file_path[state.file_path.len().saturating_sub(max_chars - 3)..]);
                s
            } else {
                String::new()
            }
        } else {
            state.file_path.to_string()
        };
        if !center_str.is_empty() {
            let center_x = PAD_X as f32 + left_w + PAD_X as f32
                + (gap - self.char_w * center_str.len() as f32) / 2.0;
            d.draw_text_ex(
                &self.font,
                &center_str,
                Vector2::new(center_x, status_y as f32),
                FONT_SIZE as f32,
                1.0,
                sl_color,
            );
        }

        // Cmdline / message (if present)
        let cmd_text = state.cmdline.or(state.message);
        if let Some(cmd) = cmd_text {
            let cmd_y = screen_h - LINE_H;
            d.draw_rectangle(0, cmd_y, screen_w, LINE_H, Color::new(30, 30, 30, 255));
            d.draw_text_ex(
                &self.font,
                cmd,
                Vector2::new(PAD_X as f32, cmd_y as f32),
                FONT_SIZE as f32,
                1.0,
                sl_color,
            );
        }

        // Cursor
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
