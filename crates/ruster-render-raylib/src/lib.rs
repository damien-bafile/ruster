mod key;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use raylib::consts::KeyboardKey;
use raylib::prelude::*;
use ruster_render::{CursorKind, FrameState, Renderer};
use std::path::PathBuf;

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
    /// Glyphs to bake into the font atlas. Raylib's default is only the 95
    /// printable ASCII codepoints, so anything else (en/em dashes, curly
    /// quotes, ellipsis, bullets, arrows, box-drawing) renders as `?`. We add
    /// Latin-1 and the common Unicode punctuation the docs and UI actually use.
    /// `load_font_ex` takes the character set as a string.
    fn font_chars() -> String {
        let mut s = String::new();
        for c in (0x20u32..=0x7E).chain(0xA0..=0xFF) {
            if let Some(ch) = char::from_u32(c) {
                s.push(ch);
            }
        }
        s.push_str("–—‘’“”•…←↑→↓─│✓✗");
        s
    }

    fn try_load_mono_font(rl: &mut RaylibHandle, thread: &RaylibThread) -> WeakFont {
        let home = std::env::var("HOME").unwrap_or_default();
        let candidates = [
            PathBuf::from(&home).join("Library/Fonts/JetBrainsMonoNerdFont-Regular.ttf").to_string_lossy().to_string(),
            "/System/Library/Fonts/SFNSMono.ttf".to_string(),
            "/System/Library/Fonts/Supplemental/Andale Mono.ttf".to_string(),
        ];
        let chars = Self::font_chars();
        for path in &candidates {
            if let Ok(font) = rl.load_font_ex(thread, path, FONT_SIZE, Some(&chars)) {
                return font.make_weak();
            }
        }
        rl.get_font_default()
    }

    pub fn new(width: i32, height: i32, title: &str) -> Self {
        let (mut rl, thread) = raylib::init()
            .size(width, height)
            .title(title)
            .resizable()
            .build();
        rl.set_target_fps(60);
        rl.set_exit_key(None);
        let font = Self::try_load_mono_font(&mut rl, &thread);
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

        // With Ctrl or Alt held, the OS text layer can't be trusted to produce
        // the base letter (it drops or composes modified keys), so Emacs/vim
        // chords are reconstructed from the physical key. Otherwise, plain
        // typing goes through the char queue for correct layout and casing.
        if mods.contains(KeyModifiers::CONTROL) || mods.contains(KeyModifiers::ALT) {
            // Discard the (absent or mangled) char events for this frame.
            while self.rl.get_char_pressed().is_some() {}
            let shift = mods.contains(KeyModifiers::SHIFT);
            while let Some(k) = self.rl.get_key_pressed() {
                if let Some(ch) = key::modified_char_for_key(k, shift) {
                    self.event_buffer.push(KeyEvent::new(KeyCode::Char(ch), mods));
                } else if let Some(event) = key::map_raylib_key(k) {
                    self.event_buffer.push(KeyEvent::new(event.code, mods));
                }
            }
        } else {
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
        }

        self.event_buffer.reverse();
    }
}

impl Renderer for RaylibRenderer {
    fn viewport_cells(&self) -> (u16, u16) {
        let cols = ((self.rl.get_screen_width() as f32 - PAD_X as f32) / self.char_w).max(1.0);
        let rows = ((self.rl.get_screen_height() - PAD_Y) / LINE_H).max(1);
        (cols as u16, rows as u16)
    }

    fn render_frame(&mut self, state: &FrameState) {
        let screen_w = self.rl.get_screen_width();
        let screen_h = self.rl.get_screen_height();
        let char_w = self.char_w;
        // Borrow font as a local so it stays disjoint from the &mut self.rl
        // borrow held by the draw handle.
        let font = &self.font;
        let measure = |s: &str| font.measure_text(s, FONT_SIZE as f32, 1.0).x;

        let default_color = Color::new(205, 214, 244, 255);
        let gutter_color = Color::new(108, 112, 134, 255);
        let divider = Color::new(69, 71, 90, 255);

        let mut d = self.rl.begin_drawing(&self.thread);
        d.clear_background(Color::new(30, 30, 30, 255));

        for view in &state.windows {
            if view.rect.width == 0 || view.rect.height == 0 {
                continue;
            }
            let px = PAD_X + (view.rect.x as f32 * char_w) as i32;
            let py = PAD_Y + view.rect.y as i32 * LINE_H;
            let pw = (view.rect.width as f32 * char_w) as i32;
            // The window's last cell-row is its statusline.
            let buf_rows = view.rect.height.saturating_sub(1) as usize;
            let text_x = px + (view.gutter.width as f32 * char_w) as i32;
            let scroll = view.scroll_offset as usize;
            let win_h = view.rect.height as i32 * LINE_H;

            // Clip everything in this window to its own rect so text/statusline
            // can't bleed past the divider into a neighbouring pane.
            {
                let mut s = d.begin_scissor_mode(px, py, pw, win_h);

                // Gutter column.
                for (row, label) in view.gutter.rows.iter().take(buf_rows).enumerate() {
                    let gy = py + row as i32 * LINE_H;
                    s.draw_text_ex(font, label, Vector2::new(px as f32, gy as f32), FONT_SIZE as f32, 1.0, gutter_color);
                }

                // Visual-mode selection background, behind the text.
                if let Some(sel) = view.selection {
                    let selection_bg = Color::new(88, 91, 112, 255);
                    for (row, line) in view.lines.iter().skip(scroll).take(buf_rows).enumerate() {
                        let buffer_line = (row + scroll) as u16;
                        let line_len = line.text.chars().count() as u16;
                        if let Some((sel_start, sel_end)) = sel.span_on(buffer_line, line_len) {
                            let gy = py + row as i32 * LINE_H;
                            let sx = text_x as f32 + sel_start as f32 * char_w;
                            // End is inclusive; empty lines still get a sliver.
                            let cols = sel_end.saturating_sub(sel_start) + 1;
                            let width = (cols as f32 * char_w).max(char_w / 2.0);
                            s.draw_rectangle(sx as i32, gy, width as i32, LINE_H, selection_bg);
                        }
                    }
                }

                // Buffer text (this window's own scroll).
                for (row, line) in view.lines.iter().skip(scroll).take(buf_rows).enumerate() {
                    let gy = py + row as i32 * LINE_H;
                    let n = line.text.len();
                    if n == 0 {
                        continue;
                    }
                    if line.highlights.is_empty() {
                        s.draw_text_ex(font, &line.text, Vector2::new(text_x as f32, gy as f32), FONT_SIZE as f32, 1.0, default_color);
                        continue;
                    }
                    let mut char_colors: Vec<Color> = vec![default_color; n];
                    for &(offset, len, ref style) in &line.highlights {
                        let fg = match style.fg {
                            ruster_render::Color::Rgb(r, g, b) => Color::new(r, g, b, 255),
                            ruster_render::Color::Default => default_color,
                        };
                        let end = (offset + len).min(n);
                        char_colors[offset..end].fill(fg);
                    }
                    let mut x_offset = text_x as f32;
                    let mut pos = 0;
                    while pos < n {
                        let c = char_colors[pos];
                        let start = pos;
                        while pos < n && char_colors[pos] == c {
                            pos += 1;
                        }
                        let seg = &line.text[start..pos];
                        s.draw_text_ex(font, seg, Vector2::new(x_offset, gy as f32), FONT_SIZE as f32, 1.0, c);
                        x_offset += measure(seg);
                    }
                }

                // Cursor (only the active window sets cursor_visible).
                if view.cursor_visible {
                    let cline = view.cursor.0 as usize;
                    if cline >= scroll && cline < scroll + buf_rows {
                        let vis_row = (cline - scroll) as i32;
                        let col = view.cursor.1 as usize;
                        let text_before = view
                            .lines
                            .get(cline)
                            .map(|l| {
                                let end = col.min(l.text.len());
                                &l.text[..end]
                            })
                            .unwrap_or("");
                        let mut cx = text_x as f32 + measure(text_before);
                        let mut cy = py + vis_row * LINE_H;
                        if let Some((dcx, dcy)) = view.cursor_smooth {
                            cx += dcx * char_w;
                            cy = (cy as f32 + dcy * LINE_H as f32) as i32;
                        }
                        let cx = cx as i32;
                        match view.cursor_kind {
                            CursorKind::Block => s.draw_rectangle(cx, cy, char_w as i32, LINE_H, Color::new(245, 224, 220, 200)),
                            CursorKind::Bar => s.draw_rectangle(cx, cy, 2, LINE_H, Color::new(245, 224, 220, 255)),
                        }
                    }

                    // Extra multi-cursor carets, always drawn as blocks.
                    for &(cl, cc) in &view.extra_cursors {
                        let cl = cl as usize;
                        if cl < scroll || cl >= scroll + buf_rows {
                            continue;
                        }
                        let vis_row = (cl - scroll) as i32;
                        let col = cc as usize;
                        let text_before = view
                            .lines
                            .get(cl)
                            .map(|l| {
                                let end = col.min(l.text.len());
                                &l.text[..end]
                            })
                            .unwrap_or("");
                        let cx = text_x as f32 + measure(text_before);
                        let cy = py + vis_row * LINE_H;
                        s.draw_rectangle(cx as i32, cy, char_w as i32, LINE_H, Color::new(245, 224, 220, 140));
                    }
                }

                // Per-window statusline on its bottom row.
                let sl_y = py + buf_rows as i32 * LINE_H;
                let (sl_bg, sl_fg) = if view.active {
                    (Color::new(69, 71, 90, 255), Color::new(205, 214, 244, 255))
                } else {
                    (Color::new(40, 40, 48, 255), Color::new(120, 120, 130, 255))
                };
                s.draw_rectangle(px, sl_y, pw, LINE_H, sl_bg);
                let left = format!(" {} ", view.statusline.left);
                s.draw_text_ex(font, &left, Vector2::new(px as f32, sl_y as f32), FONT_SIZE as f32, 1.0, sl_fg);
                let right = format!(" {} ", view.statusline.right);
                let right_x = (px + pw) as f32 - measure(&right);
                s.draw_text_ex(font, &right, Vector2::new(right_x, sl_y as f32), FONT_SIZE as f32, 1.0, sl_fg);
                if !view.statusline.center.is_empty() {
                    let center_w = measure(&view.statusline.center);
                    let center_x = px as f32 + (pw as f32 - center_w) / 2.0;
                    // Only draw the center group if it fits between left and right.
                    let left_w = measure(&left);
                    let right_w = measure(&right);
                    if pw as f32 > left_w + right_w + center_w {
                        s.draw_text_ex(font, &view.statusline.center, Vector2::new(center_x, sl_y as f32), FONT_SIZE as f32, 1.0, sl_fg);
                    }
                }
            }

            // Divider on the right edge for side-by-side windows.
            if px + pw < screen_w - 2 {
                d.draw_rectangle(px + pw, py, 1, view.rect.height as i32 * LINE_H, divider);
            }
        }

        // Shared cmdline / message. The app only reserves a bottom row (shrinking
        // the windows) while one is shown, so draw it flush at that reserved row
        // and only when present — otherwise it would overpaint the last window's
        // statusline, which now fills the bottom row itself.
        if let Some(cmd) = state.cmdline.or(state.message) {
            let rows = ((screen_h - PAD_Y) / LINE_H).max(1);
            let cmd_y = PAD_Y + (rows - 1) * LINE_H;
            d.draw_rectangle(0, cmd_y, screen_w, screen_h - cmd_y, Color::new(30, 30, 30, 255));
            d.draw_text_ex(font, cmd, Vector2::new(PAD_X as f32, cmd_y as f32), FONT_SIZE as f32, 1.0, default_color);
        }

        // Floating picker overlay, centered.
        if let Some(picker) = &state.picker {
            let accent = Color::new(137, 180, 250, 255);
            let box_bg = Color::new(30, 30, 46, 255);
            let preview_bg = Color::new(24, 24, 37, 255);
            let has_preview = !picker.preview.is_empty();
            let frac = if has_preview { 9 } else { 6 };
            let box_w = (screen_w * frac / 10).clamp(240.min(screen_w), screen_w - 20);
            let n_rows = (picker.rows.len() as i32 + 2).max(picker.preview.len() as i32);
            let box_h = (n_rows * LINE_H).clamp(3 * LINE_H, (screen_h - 40).max(3 * LINE_H));
            let box_x = (screen_w - box_w) / 2;
            let box_y = ((screen_h - box_h) / 2).max(0);
            let list_w = if has_preview { box_w * 2 / 5 } else { box_w };
            d.draw_rectangle(box_x, box_y, box_w, box_h, box_bg);
            if has_preview {
                d.draw_rectangle(box_x + list_w, box_y, box_w - list_w, box_h, preview_bg);
                d.draw_rectangle(box_x + list_w, box_y, 1, box_h, accent);
            }
            d.draw_rectangle_lines(box_x, box_y, box_w, box_h, accent);
            // List column — title, query, and rows, clipped to the list width
            // so long labels don't bleed across the divider into the preview.
            let list_clip_w = if has_preview { list_w } else { box_w };
            {
                let mut s = d.begin_scissor_mode(
                    box_x + 1,
                    box_y + 1,
                    (list_clip_w - 2).max(1),
                    box_h - 2,
                );
                s.draw_text_ex(font, &format!(" {} ", picker.title), Vector2::new(box_x as f32 + 4.0, box_y as f32), FONT_SIZE as f32, 1.0, accent);
                s.draw_text_ex(font, &format!(" > {}", picker.query), Vector2::new(box_x as f32 + 4.0, (box_y + LINE_H) as f32), FONT_SIZE as f32, 1.0, default_color);
                let max_visible = ((box_h - 2 * LINE_H) / LINE_H).max(0) as usize;
                for (i, row) in picker.rows.iter().take(max_visible).enumerate() {
                    let ry = box_y + (2 + i as i32) * LINE_H;
                    if row.selected {
                        s.draw_rectangle(box_x, ry, list_clip_w, LINE_H, accent);
                        s.draw_text_ex(font, &format!(" {}", row.label), Vector2::new(box_x as f32 + 4.0, ry as f32), FONT_SIZE as f32, 1.0, box_bg);
                    } else {
                        s.draw_text_ex(font, &format!(" {}", row.label), Vector2::new(box_x as f32 + 4.0, ry as f32), FONT_SIZE as f32, 1.0, default_color);
                    }
                }
            }
            // Preview column (syntax-highlighted), clipped to its own pane.
            if has_preview {
                let mut s = d.begin_scissor_mode(
                    box_x + list_w + 1,
                    box_y + 1,
                    (box_w - list_w - 2).max(1),
                    box_h - 2,
                );
                let px = box_x + list_w + 6;
                for (i, line) in picker.preview.iter().enumerate() {
                    let ly = box_y + i as i32 * LINE_H;
                    if ly > box_y + box_h {
                        break;
                    }
                    let n = line.text.len();
                    if n == 0 {
                        continue;
                    }
                    if line.highlights.is_empty() {
                        s.draw_text_ex(font, &line.text, Vector2::new(px as f32, ly as f32), FONT_SIZE as f32, 1.0, default_color);
                        continue;
                    }
                    let mut char_colors: Vec<Color> = vec![default_color; n];
                    for &(offset, len, ref style) in &line.highlights {
                        let fg = match style.fg {
                            ruster_render::Color::Rgb(r, g, b) => Color::new(r, g, b, 255),
                            ruster_render::Color::Default => default_color,
                        };
                        let end = (offset + len).min(n);
                        char_colors[offset..end].fill(fg);
                    }
                    let mut x_off = px as f32;
                    let mut pos = 0;
                    while pos < n {
                        let c = char_colors[pos];
                        let start = pos;
                        while pos < n && char_colors[pos] == c {
                            pos += 1;
                        }
                        let seg = &line.text[start..pos];
                        s.draw_text_ex(font, seg, Vector2::new(x_off, ly as f32), FONT_SIZE as f32, 1.0, c);
                        x_off += measure(seg);
                    }
                }
            }
        }

        // Hover popup, near the top-center (syntax-highlighted).
        if let Some(lines) = &state.hover {
            if !lines.is_empty() {
                let accent = Color::new(137, 180, 250, 255);
                let box_bg = Color::new(24, 24, 37, 255);
                let longest = lines.iter().map(|l| l.text.chars().count()).max().unwrap_or(0);
                let box_w = ((longest as f32 * char_w) as i32 + 16).min(screen_w - 20);
                let box_h = (lines.len() as i32 * LINE_H + 8).min(screen_h - 20);
                let box_x = (screen_w - box_w) / 2;
                let box_y = LINE_H;
                d.draw_rectangle(box_x, box_y, box_w, box_h, box_bg);
                d.draw_rectangle_lines(box_x, box_y, box_w, box_h, accent);
                let mut s = d.begin_scissor_mode(box_x + 1, box_y + 1, box_w - 2, box_h - 2);
                for (i, line) in lines.iter().enumerate() {
                    let ly = box_y + 4 + i as i32 * LINE_H;
                    let n = line.text.len();
                    if n == 0 {
                        continue;
                    }
                    if line.highlights.is_empty() {
                        s.draw_text_ex(font, &line.text, Vector2::new(box_x as f32 + 6.0, ly as f32), FONT_SIZE as f32, 1.0, default_color);
                        continue;
                    }
                    let mut char_colors: Vec<Color> = vec![default_color; n];
                    for &(offset, len, ref style) in &line.highlights {
                        let fg = match style.fg {
                            ruster_render::Color::Rgb(r, g, b) => Color::new(r, g, b, 255),
                            ruster_render::Color::Default => default_color,
                        };
                        let end = (offset + len).min(n);
                        char_colors[offset..end].fill(fg);
                    }
                    let mut x_off = box_x as f32 + 6.0;
                    let mut pos = 0;
                    while pos < n {
                        let c = char_colors[pos];
                        let start = pos;
                        while pos < n && char_colors[pos] == c {
                            pos += 1;
                        }
                        let seg = &line.text[start..pos];
                        s.draw_text_ex(font, seg, Vector2::new(x_off, ly as f32), FONT_SIZE as f32, 1.0, c);
                        x_off += measure(seg);
                    }
                }
            }
        }

        // Bottom which-key panel, sliding up from the screen edge by `anim`.
        if let Some(wk) = &state.whichkey {
            let accent = Color::new(137, 180, 250, 255);
            let box_bg = Color::new(30, 30, 46, 255);
            let panel_h = (wk.rows.len() as i32 + 1) * LINE_H + 8;
            let panel_top = screen_h - (panel_h as f32 * wk.anim.clamp(0.0, 1.0)) as i32;
            // Clip to the visible (slid-in) region so nothing draws above it.
            let mut s = d.begin_scissor_mode(0, panel_top, screen_w, screen_h - panel_top);
            s.draw_rectangle(0, panel_top, screen_w, screen_h - panel_top, box_bg);
            s.draw_rectangle(0, panel_top, screen_w, 2, accent);
            s.draw_text_ex(font, &format!(" {} ", wk.title), Vector2::new(PAD_X as f32, (panel_top + 4) as f32), FONT_SIZE as f32, 1.0, accent);
            for (i, entry) in wk.rows.iter().enumerate() {
                let ry = panel_top + 4 + (i as i32 + 1) * LINE_H;
                s.draw_text_ex(font, &format!("   {}", entry), Vector2::new(PAD_X as f32, ry as f32), FONT_SIZE as f32, 1.0, Color::new(205, 214, 244, 255));
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
