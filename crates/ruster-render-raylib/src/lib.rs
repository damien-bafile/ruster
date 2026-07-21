mod key;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use raylib::consts::KeyboardKey;
use raylib::prelude::*;
use ruster_render::{CursorKind, EditorState, Renderer};

const FONT_SIZE: i32 = 20;
const CHAR_W: i32 = 12;
const LINE_H: i32 = 24;
const PAD_X: i32 = 8;
const PAD_Y: i32 = 4;

pub struct RaylibRenderer {
    rl: RaylibHandle,
    thread: RaylibThread,
    font: WeakFont,
    event_buffer: Vec<KeyEvent>,
}

impl RaylibRenderer {
    pub fn new(width: i32, height: i32, title: &str) -> Self {
        let (mut rl, thread) = raylib::init()
            .size(width, height)
            .title(title)
            .build();
        rl.set_target_fps(60);
        let font = rl.get_font_default();
        RaylibRenderer { rl, thread, font, event_buffer: Vec::new() }
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

        for (i, line) in state.lines.iter().enumerate() {
            let y = PAD_Y + i as i32 * LINE_H;
            d.draw_text_ex(
                &self.font,
                &line.text,
                Vector2::new(PAD_X as f32, y as f32),
                FONT_SIZE as f32,
                1.0,
                Color::new(205, 214, 244, 255),
            );
        }

        if state.cursor_visible {
            let cx = PAD_X + state.cursor.1 as i32 * CHAR_W;
            let cy = PAD_Y + state.cursor.0 as i32 * LINE_H;
            match state.cursor_kind {
                CursorKind::Block => {
                    d.draw_rectangle(cx, cy, CHAR_W, LINE_H, Color::new(245, 224, 220, 200));
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
