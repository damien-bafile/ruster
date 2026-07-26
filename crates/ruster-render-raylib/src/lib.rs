mod key;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use raylib::consts::KeyboardKey;
use raylib::prelude::*;
use ruster_render::{ControlKind, CursorKind, FrameState, GuiConfig, Renderer, SettingRowView};

pub struct RaylibRenderer {
    rl: RaylibHandle,
    thread: RaylibThread,
    font: WeakFont,
    char_w: f32,
    font_size: i32,
    line_h: i32,
    pad_x: i32,
    pad_y: i32,
    theme: ruster_render::Theme,
    /// The loaded font's (path, size) so live re-theming only reloads the atlas
    /// when the font actually changed (color-only tweaks skip the reload).
    font_sig: (Option<String>, i32),
    /// Top visible line of the Settings overlay, persisted across frames so the
    /// list scrolls like a normal widget (holds until the selection hits an edge).
    settings_scroll: usize,
    event_buffer: Vec<KeyEvent>,
}

/// Map a render-neutral color to a raylib color, using `fallback` for `Default`.
fn to_raylib(c: ruster_render::Color, fallback: Color) -> Color {
    match c {
        ruster_render::Color::Rgb(r, g, b) => Color::new(r, g, b, 255),
        ruster_render::Color::Default => fallback,
    }
}

/// Parse a `#RRGGBB` string into RGB, for drawing color-setting swatches.
fn hex_rgb(s: &str) -> Option<(u8, u8, u8)> {
    let b = s.as_bytes();
    if b.len() != 7 || b[0] != b'#' {
        return None;
    }
    let r = u8::from_str_radix(&s[1..3], 16).ok()?;
    let g = u8::from_str_radix(&s[3..5], 16).ok()?;
    let bl = u8::from_str_radix(&s[5..7], 16).ok()?;
    Some((r, g, bl))
}

/// The RGB channels of a render color, or `fallback` for `Default`.
fn rgb_of(c: ruster_render::Color, fallback: (u8, u8, u8)) -> (u8, u8, u8) {
    match c {
        ruster_render::Color::Rgb(r, g, b) => (r, g, b),
        ruster_render::Color::Default => fallback,
    }
}

/// Candidate monospaced font files to try, most-preferred first: an explicit
/// `gui_font` override, then a user-installed Nerd font (via the platform font
/// dir), then per-OS system monospaced fonts. This lets the GUI render a real
/// mono font — and, with a Nerd font, icon glyphs — on Windows, macOS, and Linux
/// instead of raylib's low-resolution default.
fn mono_font_candidates(font_override: Option<&str>) -> Vec<String> {
    let mut out = Vec::new();
    // An explicit override wins: absolute/relative path used as-is, a bare
    // filename resolved against the user font dir.
    if let Some(f) = font_override.filter(|s| !s.is_empty()) {
        if std::path::Path::new(f).is_absolute() || f.contains(std::path::MAIN_SEPARATOR) {
            out.push(f.to_string());
        } else if let Some(dir) = dirs::font_dir() {
            out.push(dir.join(f).to_string_lossy().into_owned());
        }
    }
    // On macOS/Linux this resolves the user font dir (e.g. ~/Library/Fonts);
    // it returns None on Windows, where the system paths below cover it.
    if let Some(font_dir) = dirs::font_dir() {
        // Common Nerd font filenames, so icons work out of the box if any of
        // these are installed (Homebrew casks / nerdfonts.com use these names).
        // Prefer the "Mono" Nerd font variants: their icons are single-cell
        // width, which keeps the fixed-width grid aligned.
        for name in [
            "JetBrainsMonoNerdFontMono-Regular.ttf",
            "FiraCodeNerdFontMono-Regular.ttf",
            "CaskaydiaCoveNerdFontMono-Regular.ttf",
            "HackNerdFontMono-Regular.ttf",
            "MesloLGSNerdFontMono-Regular.ttf",
            "JetBrainsMonoNerdFont-Regular.ttf",
            "FiraCodeNerdFont-Regular.ttf",
            "CascadiaCodeNF.ttf",
            "JetBrainsMono-Regular.ttf",
        ] {
            out.push(font_dir.join(name).to_string_lossy().into_owned());
        }
    }
    #[cfg(target_os = "windows")]
    {
        out.push(r"C:\Windows\Fonts\consola.ttf".to_string()); // Consolas
        out.push(r"C:\Windows\Fonts\lucon.ttf".to_string()); // Lucida Console
    }
    #[cfg(target_os = "macos")]
    {
        out.push("/System/Library/Fonts/SFNSMono.ttf".to_string());
        out.push("/System/Library/Fonts/Supplemental/Andale Mono.ttf".to_string());
    }
    #[cfg(target_os = "linux")]
    {
        out.push("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf".to_string());
        out.push("/usr/share/fonts/TTF/DejaVuSansMono.ttf".to_string());
    }
    out
}

impl RaylibRenderer {
    /// Glyphs to bake into the font atlas. Raylib's default is only the 95
    /// printable ASCII codepoints, so anything else (en/em dashes, curly
    /// quotes, ellipsis, bullets, arrows, box-drawing) renders as `?`. We add
    /// Latin-1, the common Unicode punctuation the docs and UI use, and — for
    /// terminal output — box-drawing plus the Nerd Font icon ranges (Private Use
    /// Area). Codepoints a font lacks fall back to a blank; a non-Nerd font
    /// simply won't have the icon glyphs. `load_font_ex` takes the set as a
    /// string. The plane-1 Material Design range is intentionally omitted to
    /// keep the atlas small.
    fn font_chars() -> String {
        let mut s = String::new();
        // Ranges of codepoints to bake, as inclusive (start, end) pairs.
        const RANGES: &[(u32, u32)] = &[
            (0x20, 0x7E),     // printable ASCII
            (0xA0, 0xFF),     // Latin-1 supplement
            (0x2500, 0x259F), // box drawing + block elements
            (0x2600, 0x26FF), // misc symbols (⚡ etc.)
            (0xE000, 0xE00D), // Pomicons
            (0xE0A0, 0xE0D7), // Powerline + extras
            (0xE200, 0xE2A9), // Font Awesome extension
            (0xE300, 0xE3E3), // Weather
            (0xE5FA, 0xE6B7), // Seti-UI + custom
            (0xE700, 0xE7C5), // Devicons
            (0xEA60, 0xEC1E), // Codicons
            (0xF000, 0xF2FF), // Font Awesome
            (0xF300, 0xF375), // Font Logos
            (0xF400, 0xF533), // Octicons
        ];
        for &(start, end) in RANGES {
            for c in start..=end {
                if let Some(ch) = char::from_u32(c) {
                    s.push(ch);
                }
            }
        }
        s.push_str("–—‘’“”•…←↑→↓✓✗");
        s
    }

    fn try_load_mono_font(
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        font_override: Option<&str>,
        font_size: i32,
    ) -> WeakFont {
        let chars = Self::font_chars();
        for path in mono_font_candidates(font_override) {
            if let Ok(font) = rl.load_font_ex(thread, &path, font_size, Some(&chars)) {
                return font.make_weak();
            }
        }
        rl.get_font_default()
    }

    pub fn new(title: &str, gui: GuiConfig, font_override: Option<&str>) -> Self {
        let (mut rl, thread) = raylib::init()
            .size(gui.window_width, gui.window_height)
            .title(title)
            // Errors only — silence raylib's per-glyph font warnings and info logs.
            .log_level(raylib::ffi::TraceLogLevel::LOG_ERROR)
            .resizable()
            .build();
        rl.set_target_fps(gui.target_fps as u32);
        rl.set_exit_key(None);
        let font = Self::try_load_mono_font(&mut rl, &thread, font_override, gui.font_size);
        let char_w = font.measure_text("m", gui.font_size as f32, 1.0).x;
        RaylibRenderer {
            rl,
            thread,
            font,
            char_w,
            font_size: gui.font_size,
            line_h: gui.line_height,
            pad_x: gui.padding_x,
            pad_y: gui.padding_y,
            theme: gui.theme,
            font_sig: (font_override.map(str::to_string), gui.font_size),
            settings_scroll: 0,
            event_buffer: Vec::new(),
        }
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
        let cols = ((self.rl.get_screen_width() as f32 - self.pad_x as f32) / self.char_w).max(1.0);
        let rows = ((self.rl.get_screen_height() - self.pad_y) / self.line_h).max(1);
        (cols as u16, rows as u16)
    }

    fn render_frame(&mut self, state: &FrameState) {
        let screen_w = self.rl.get_screen_width();
        let screen_h = self.rl.get_screen_height();
        let char_w = self.char_w;
        // Read the config-driven metrics + palette into locals so they stay
        // disjoint from the &mut self.rl borrow the draw handle holds.
        let (font_size, line_h, pad_x, pad_y) = (self.font_size, self.line_h, self.pad_x, self.pad_y);
        let theme = self.theme;
        let font = &self.font;
        let measure = |s: &str| font.measure_text(s, font_size as f32, 1.0).x;

        let default_color = to_raylib(theme.fg, Color::new(205, 214, 244, 255));
        let gutter_color = to_raylib(theme.gutter, Color::new(108, 112, 134, 255));
        let divider = to_raylib(theme.divider, Color::new(69, 71, 90, 255));
        let bg = to_raylib(theme.bg, Color::new(30, 30, 30, 255));
        let gutter_bg = to_raylib(theme.gutter_bg, bg);
        let selection_bg = to_raylib(theme.selection, Color::new(88, 91, 112, 255));
        let selection_fg = to_raylib(theme.selection_fg, default_color);
        let (cur_r, cur_g, cur_b) = rgb_of(theme.cursor, (245, 224, 220));
        let cursor_fg = to_raylib(theme.cursor_fg, bg);
        let accent = to_raylib(theme.accent, Color::new(243, 139, 168, 255));
        let accent_fg = to_raylib(theme.accent_fg, bg);
        let statusline_fg = to_raylib(theme.statusline_fg, default_color);

        let mut d = self.rl.begin_drawing(&self.thread);
        d.clear_background(bg);

        for view in &state.windows {
            if view.rect.width == 0 || view.rect.height == 0 {
                continue;
            }
            let px = pad_x + (view.rect.x as f32 * char_w) as i32;
            let py = pad_y + view.rect.y as i32 * line_h;
            let pw = (view.rect.width as f32 * char_w) as i32;
            // The window's last cell-row is its statusline.
            let buf_rows = view.rect.height.saturating_sub(1) as usize;
            let text_x = px + (view.gutter.width as f32 * char_w) as i32;
            let scroll = view.scroll_offset as usize;
            let win_h = view.rect.height as i32 * line_h;

            // Clip everything in this window to its own rect so text/statusline
            // can't bleed past the divider into a neighbouring pane.
            {
                let mut s = d.begin_scissor_mode(px, py, pw, win_h);

                if let Some(grid) = &view.terminal {
                    // An embedded terminal: a background quad per cell, then the
                    // glyph, then a block cursor. No gutter/scroll/selection.
                    for r in 0..grid.rows.min(buf_rows) {
                        let gy = py + r as i32 * line_h;
                        for c in 0..grid.cols {
                            let tc = grid.cells[r * grid.cols + c];
                            let cx = px + (c as f32 * char_w) as i32;
                            let (mut fg, mut bg) = (tc.fg, tc.bg);
                            if tc.inverse {
                                std::mem::swap(&mut fg, &mut bg);
                            }
                            if let ruster_render::Color::Rgb(rr, gg, bb) = bg {
                                s.draw_rectangle(cx, gy, char_w.ceil() as i32, line_h, Color::new(rr, gg, bb, 255));
                            }
                            if tc.c != ' ' && tc.c != '\0' {
                                let color = match fg {
                                    ruster_render::Color::Rgb(rr, gg, bb) => Color::new(rr, gg, bb, 255),
                                    ruster_render::Color::Default => default_color,
                                };
                                let mut ch = [0u8; 4];
                                s.draw_text_ex(font, tc.c.encode_utf8(&mut ch), Vector2::new(cx as f32, gy as f32), font_size as f32, 1.0, color);
                            }
                        }
                    }
                    if view.cursor_visible && view.active {
                        let (cr, cc) = grid.cursor;
                        if cr < buf_rows && cc < grid.cols {
                            let cx = px + (cc as f32 * char_w) as i32;
                            let cy = py + cr as i32 * line_h;
                            s.draw_rectangle(cx, cy, char_w as i32, line_h, Color::new(cur_r, cur_g, cur_b, 160));
                        }
                    }
                } else {
                // Gutter background (only when a gutter is shown).
                if view.gutter.width > 0 && gutter_bg != bg {
                    s.draw_rectangle(px, py, text_x - px, buf_rows as i32 * line_h, gutter_bg);
                }
                // Gutter column.
                for (row, label) in view.gutter.rows.iter().take(buf_rows).enumerate() {
                    let gy = py + row as i32 * line_h;
                    s.draw_text_ex(font, label, Vector2::new(px as f32, gy as f32), font_size as f32, 1.0, gutter_color);
                }

                // Visual-mode selection background, behind the text.
                if let Some(sel) = view.selection {
                    for (row, line) in view.lines.iter().skip(scroll).take(buf_rows).enumerate() {
                        let buffer_line = (row + scroll) as u16;
                        let line_len = line.text.chars().count() as u16;
                        if let Some((sel_start, sel_end)) = sel.span_on(buffer_line, line_len) {
                            let gy = py + row as i32 * line_h;
                            let sx = text_x as f32 + sel_start as f32 * char_w;
                            // End is inclusive; empty lines still get a sliver.
                            let cols = sel_end.saturating_sub(sel_start) + 1;
                            let width = (cols as f32 * char_w).max(char_w / 2.0);
                            s.draw_rectangle(sx as i32, gy, width as i32, line_h, selection_bg);
                        }
                    }
                }

                // Buffer text (this window's own scroll).
                for (row, line) in view.lines.iter().skip(scroll).take(buf_rows).enumerate() {
                    let gy = py + row as i32 * line_h;
                    let n = line.text.len();
                    if n == 0 {
                        continue;
                    }
                    // The selection span on this line, so selected glyphs take
                    // the selection text color.
                    let sel_span = view.selection.and_then(|sel| {
                        let buffer_line = (row + scroll) as u16;
                        sel.span_on(buffer_line, line.text.chars().count() as u16)
                    });
                    if line.highlights.is_empty() && sel_span.is_none() {
                        s.draw_text_ex(font, &line.text, Vector2::new(text_x as f32, gy as f32), font_size as f32, 1.0, default_color);
                        continue;
                    }
                    // Color per *character* (highlight offsets are char offsets),
                    // then draw same-color runs — safe for multibyte lines.
                    let chars: Vec<char> = line.text.chars().collect();
                    let nchars = chars.len();
                    let mut char_colors: Vec<Color> = vec![default_color; nchars];
                    for &(offset, len, ref style) in &line.highlights {
                        let fg = match style.fg {
                            ruster_render::Color::Rgb(r, g, b) => Color::new(r, g, b, 255),
                            ruster_render::Color::Default => default_color,
                        };
                        let end = (offset + len).min(nchars);
                        if offset < end {
                            char_colors[offset..end].fill(fg);
                        }
                    }
                    if let Some((ss, se)) = sel_span {
                        let end = (se as usize + 1).min(nchars);
                        if (ss as usize) < end {
                            char_colors[ss as usize..end].fill(selection_fg);
                        }
                    }
                    let mut x_offset = text_x as f32;
                    let mut i = 0;
                    while i < nchars {
                        let c = char_colors[i];
                        let start = i;
                        while i < nchars && char_colors[i] == c {
                            i += 1;
                        }
                        let seg: String = chars[start..i].iter().collect();
                        s.draw_text_ex(font, &seg, Vector2::new(x_offset, gy as f32), font_size as f32, 1.0, c);
                        x_offset += measure(&seg);
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
                                // `col` is a character column; find its byte offset
                                // so multibyte lines don't slice mid-character.
                                let end = l
                                    .text
                                    .char_indices()
                                    .nth(col)
                                    .map(|(i, _)| i)
                                    .unwrap_or(l.text.len());
                                &l.text[..end]
                            })
                            .unwrap_or("");
                        let mut cx = text_x as f32 + measure(text_before);
                        let mut cy = py + vis_row * line_h;
                        if let Some((dcx, dcy)) = view.cursor_smooth {
                            cx += dcx * char_w;
                            cy = (cy as f32 + dcy * line_h as f32) as i32;
                        }
                        let cx = cx as i32;
                        match view.cursor_kind {
                            CursorKind::Block => {
                                // Solid block, then redraw the glyph under it in the
                                // cursor text color (classic block-cursor look).
                                s.draw_rectangle(cx, cy, char_w as i32, line_h, Color::new(cur_r, cur_g, cur_b, 255));
                                if let Some(ch) = view.lines.get(cline).and_then(|l| l.text.chars().nth(col)) {
                                    if ch != ' ' {
                                        let mut buf = [0u8; 4];
                                        s.draw_text_ex(font, ch.encode_utf8(&mut buf), Vector2::new(cx as f32, cy as f32), font_size as f32, 1.0, cursor_fg);
                                    }
                                }
                            }
                            CursorKind::Bar => s.draw_rectangle(cx, cy, 2, line_h, Color::new(cur_r, cur_g, cur_b, 255)),
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
                                // `col` is a character column; find its byte offset
                                // so multibyte lines don't slice mid-character.
                                let end = l
                                    .text
                                    .char_indices()
                                    .nth(col)
                                    .map(|(i, _)| i)
                                    .unwrap_or(l.text.len());
                                &l.text[..end]
                            })
                            .unwrap_or("");
                        let cx = text_x as f32 + measure(text_before);
                        let cy = py + vis_row * line_h;
                        s.draw_rectangle(cx as i32, cy, char_w as i32, line_h, Color::new(cur_r, cur_g, cur_b, 140));
                    }
                }
                } // end: buffer vs. terminal drawing

                // Per-window statusline on its bottom row.
                let sl_y = py + buf_rows as i32 * line_h;
                let (sl_bg, sl_fg) = if view.active {
                    (divider, statusline_fg)
                } else {
                    (Color::new(40, 40, 48, 255), Color::new(120, 120, 130, 255))
                };
                s.draw_rectangle(px, sl_y, pw, line_h, sl_bg);
                let left = format!(" {} ", view.statusline.left);
                s.draw_text_ex(font, &left, Vector2::new(px as f32, sl_y as f32), font_size as f32, 1.0, sl_fg);
                let right = format!(" {} ", view.statusline.right);
                let right_x = (px + pw) as f32 - measure(&right);
                s.draw_text_ex(font, &right, Vector2::new(right_x, sl_y as f32), font_size as f32, 1.0, sl_fg);
                if !view.statusline.center.is_empty() {
                    let center_w = measure(&view.statusline.center);
                    let center_x = px as f32 + (pw as f32 - center_w) / 2.0;
                    // Only draw the center group if it fits between left and right.
                    let left_w = measure(&left);
                    let right_w = measure(&right);
                    if pw as f32 > left_w + right_w + center_w {
                        s.draw_text_ex(font, &view.statusline.center, Vector2::new(center_x, sl_y as f32), font_size as f32, 1.0, sl_fg);
                    }
                }
            }

            // Divider on the right edge for side-by-side windows.
            if px + pw < screen_w - 2 {
                d.draw_rectangle(px + pw, py, 1, view.rect.height as i32 * line_h, divider);
            }
        }

        // Shared cmdline / message. The app only reserves a bottom row (shrinking
        // the windows) while one is shown, so draw it flush at that reserved row
        // and only when present — otherwise it would overpaint the last window's
        // statusline, which now fills the bottom row itself.
        if let Some(cmd) = state.cmdline.or(state.message) {
            let rows = ((screen_h - pad_y) / line_h).max(1);
            let cmd_y = pad_y + (rows - 1) * line_h;
            d.draw_rectangle(0, cmd_y, screen_w, screen_h - cmd_y, bg);
            d.draw_text_ex(font, cmd, Vector2::new(pad_x as f32, cmd_y as f32), font_size as f32, 1.0, default_color);
        }

        // Floating picker overlay, centered.
        if let Some(picker) = &state.picker {
            let box_bg = Color::new(30, 30, 46, 255);
            let preview_bg = Color::new(24, 24, 37, 255);
            let has_preview = !picker.preview.is_empty();
            let frac = if has_preview { 9 } else { 6 };
            let box_w = (screen_w * frac / 10).clamp(240.min(screen_w), screen_w - 20);
            let n_rows = (picker.rows.len() as i32 + 2).max(picker.preview.len() as i32);
            let box_h = (n_rows * line_h).clamp(3 * line_h, (screen_h - 40).max(3 * line_h));
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
                s.draw_text_ex(font, &format!(" {} ", picker.title), Vector2::new(box_x as f32 + 4.0, box_y as f32), font_size as f32, 1.0, accent);
                s.draw_text_ex(font, &format!(" > {}", picker.query), Vector2::new(box_x as f32 + 4.0, (box_y + line_h) as f32), font_size as f32, 1.0, default_color);
                let max_visible = ((box_h - 2 * line_h) / line_h).max(0) as usize;
                for (i, row) in picker.rows.iter().take(max_visible).enumerate() {
                    let ry = box_y + (2 + i as i32) * line_h;
                    if row.selected {
                        s.draw_rectangle(box_x, ry, list_clip_w, line_h, accent);
                        s.draw_text_ex(font, &format!(" {}", row.label), Vector2::new(box_x as f32 + 4.0, ry as f32), font_size as f32, 1.0, box_bg);
                    } else {
                        s.draw_text_ex(font, &format!(" {}", row.label), Vector2::new(box_x as f32 + 4.0, ry as f32), font_size as f32, 1.0, default_color);
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
                    let ly = box_y + i as i32 * line_h;
                    if ly > box_y + box_h {
                        break;
                    }
                    let n = line.text.len();
                    if n == 0 {
                        continue;
                    }
                    if line.highlights.is_empty() {
                        s.draw_text_ex(font, &line.text, Vector2::new(px as f32, ly as f32), font_size as f32, 1.0, default_color);
                        continue;
                    }
                    let chars: Vec<char> = line.text.chars().collect();
                    let nchars = chars.len();
                    let mut char_colors: Vec<Color> = vec![default_color; nchars];
                    for &(offset, len, ref style) in &line.highlights {
                        let fg = match style.fg {
                            ruster_render::Color::Rgb(r, g, b) => Color::new(r, g, b, 255),
                            ruster_render::Color::Default => default_color,
                        };
                        let end = (offset + len).min(nchars);
                        if offset < end {
                            char_colors[offset..end].fill(fg);
                        }
                    }
                    let mut x_off = px as f32;
                    let mut ci = 0;
                    while ci < nchars {
                        let c = char_colors[ci];
                        let start = ci;
                        while ci < nchars && char_colors[ci] == c {
                            ci += 1;
                        }
                        let seg: String = chars[start..ci].iter().collect();
                        s.draw_text_ex(font, &seg, Vector2::new(x_off, ly as f32), font_size as f32, 1.0, c);
                        x_off += measure(&seg);
                    }
                }
            }
        }

        // Hover popup, near the top-center (syntax-highlighted).
        if let Some(lines) = &state.hover {
            if !lines.is_empty() {
                let box_bg = Color::new(24, 24, 37, 255);
                let longest = lines.iter().map(|l| l.text.chars().count()).max().unwrap_or(0);
                let box_w = ((longest as f32 * char_w) as i32 + 16).min(screen_w - 20);
                let box_h = (lines.len() as i32 * line_h + 8).min(screen_h - 20);
                let box_x = (screen_w - box_w) / 2;
                let box_y = line_h;
                d.draw_rectangle(box_x, box_y, box_w, box_h, box_bg);
                d.draw_rectangle_lines(box_x, box_y, box_w, box_h, accent);
                let mut s = d.begin_scissor_mode(box_x + 1, box_y + 1, box_w - 2, box_h - 2);
                for (i, line) in lines.iter().enumerate() {
                    let ly = box_y + 4 + i as i32 * line_h;
                    let n = line.text.len();
                    if n == 0 {
                        continue;
                    }
                    if line.highlights.is_empty() {
                        s.draw_text_ex(font, &line.text, Vector2::new(box_x as f32 + 6.0, ly as f32), font_size as f32, 1.0, default_color);
                        continue;
                    }
                    let chars: Vec<char> = line.text.chars().collect();
                    let nchars = chars.len();
                    let mut char_colors: Vec<Color> = vec![default_color; nchars];
                    for &(offset, len, ref style) in &line.highlights {
                        let fg = match style.fg {
                            ruster_render::Color::Rgb(r, g, b) => Color::new(r, g, b, 255),
                            ruster_render::Color::Default => default_color,
                        };
                        let end = (offset + len).min(nchars);
                        if offset < end {
                            char_colors[offset..end].fill(fg);
                        }
                    }
                    let mut x_off = box_x as f32 + 6.0;
                    let mut ci = 0;
                    while ci < nchars {
                        let c = char_colors[ci];
                        let start = ci;
                        while ci < nchars && char_colors[ci] == c {
                            ci += 1;
                        }
                        let seg: String = chars[start..ci].iter().collect();
                        s.draw_text_ex(font, &seg, Vector2::new(x_off, ly as f32), font_size as f32, 1.0, c);
                        x_off += measure(&seg);
                    }
                }
            }
        }

        // Bottom which-key panel, sliding up from the screen edge by `anim`.
        if let Some(wk) = &state.whichkey {
            let box_bg = Color::new(30, 30, 46, 255);
            let panel_h = (wk.rows.len() as i32 + 1) * line_h + 8;
            let panel_top = screen_h - (panel_h as f32 * wk.anim.clamp(0.0, 1.0)) as i32;
            // Clip to the visible (slid-in) region so nothing draws above it.
            let mut s = d.begin_scissor_mode(0, panel_top, screen_w, screen_h - panel_top);
            s.draw_rectangle(0, panel_top, screen_w, screen_h - panel_top, box_bg);
            s.draw_rectangle(0, panel_top, screen_w, 2, accent);
            s.draw_text_ex(font, &format!(" {} ", wk.title), Vector2::new(pad_x as f32, (panel_top + 4) as f32), font_size as f32, 1.0, accent);
            for (i, entry) in wk.rows.iter().enumerate() {
                let ry = panel_top + 4 + (i as i32 + 1) * line_h;
                s.draw_text_ex(font, &format!("   {}", entry), Vector2::new(pad_x as f32, ry as f32), font_size as f32, 1.0, default_color);
            }
        }

        // Settings page — a large centered overlay, themed from the live palette.
        if let Some(settings) = &state.settings {
            let sbg = bg;
            let sel_bg = selection_bg;
            let dim = gutter_color;
            let bar_bg = divider;
            let bw = screen_w * 8 / 10;
            let bh = screen_h * 9 / 10;
            let bx = (screen_w - bw) / 2;
            let by = (screen_h - bh) / 2;
            let mut s = d.begin_scissor_mode(bx, by, bw, bh);
            s.draw_rectangle(bx, by, bw, bh, sbg);
            s.draw_rectangle(bx, by, bw, line_h, accent);
            let title = format!(" Settings{} ", if settings.dirty { " [+]" } else { "" });
            s.draw_text_ex(font, &title, Vector2::new((bx + 4) as f32, by as f32), font_size as f32, 1.0, accent_fg);

            // Flatten groups into header/row lines.
            let mut lines: Vec<(bool, String, Option<&SettingRowView>)> = Vec::new();
            for g in &settings.groups {
                lines.push((true, g.name.clone(), None));
                for r in &g.rows {
                    lines.push((false, r.label.clone(), Some(r)));
                }
            }
            let selected = lines
                .iter()
                .position(|(_, _, r)| r.map(|x| x.selected).unwrap_or(false))
                .unwrap_or(0);
            // Reserve the title, help and footer rows so the last item can't
            // overlap them; scroll like a normal list (hold until an edge).
            let body_rows = ((bh - 3 * line_h) / line_h).max(1) as usize;
            self.settings_scroll =
                ruster_render::settings_scroll(self.settings_scroll, selected, body_rows, lines.len());
            let scroll = self.settings_scroll;
            let value_x = (bx + (32.0 * char_w) as i32).min(bx + bw / 2);

            for (i, (is_h, label, row)) in lines.iter().skip(scroll).take(body_rows).enumerate() {
                let ry = by + line_h + i as i32 * line_h;
                if *is_h {
                    s.draw_text_ex(font, &format!("── {} ", label.to_uppercase()), Vector2::new((bx + 4) as f32, ry as f32), font_size as f32, 1.0, accent);
                } else if let Some(r) = row {
                    if r.selected {
                        s.draw_rectangle(bx, ry, bw, line_h, sel_bg);
                    }
                    s.draw_text_ex(font, label, Vector2::new((bx + 8) as f32, ry as f32), font_size as f32, 1.0, default_color);
                    let ctrl = match r.kind {
                        ControlKind::Toggle => {
                            if r.value == "on" { "[x] on".to_string() } else { "[ ] off".to_string() }
                        }
                        ControlKind::Enum => format!("< {} >", r.value),
                        ControlKind::Number | ControlKind::Text => {
                            if r.editing { format!("{}▏", r.value) } else { r.value.clone() }
                        }
                    };
                    let cc = if r.editing { accent } else { default_color };
                    s.draw_text_ex(font, &ctrl, Vector2::new(value_x as f32, ry as f32), font_size as f32, 1.0, cc);
                    // A swatch after a hex color value, so the picker shows it.
                    if let Some((cr, cg, cb)) = r.swatch.as_deref().and_then(hex_rgb) {
                        let sw = font_size;
                        let swx = value_x + (ctrl.chars().count() as f32 * char_w) as i32 + 6;
                        s.draw_rectangle(swx, ry + 2, sw, sw - 2, Color::new(cr, cg, cb, 255));
                    }
                }
            }

            // Selected help + footer.
            if let Some((_, _, Some(r))) = lines.get(selected) {
                let hy = by + bh - 2 * line_h;
                s.draw_text_ex(font, &r.help, Vector2::new((bx + 4) as f32, hy as f32), font_size as f32, 1.0, dim);
            }
            let fy = by + bh - line_h;
            s.draw_rectangle(bx, fy, bw, line_h, bar_bg);
            s.draw_text_ex(font, &settings.footer, Vector2::new((bx + 4) as f32, fy as f32), font_size as f32, 1.0, default_color);
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

    fn set_gui_config(&mut self, gui: &GuiConfig, font: Option<&str>) {
        // Only reload the font atlas when the font/size actually changed — so a
        // color-only live preview (per keystroke) stays cheap.
        let sig = (font.map(str::to_string), gui.font_size);
        if sig != self.font_sig {
            self.font = Self::try_load_mono_font(&mut self.rl, &self.thread, font, gui.font_size);
            self.char_w = self.font.measure_text("m", gui.font_size as f32, 1.0).x;
            self.font_sig = sig;
        }
        self.font_size = gui.font_size;
        self.line_h = gui.line_height;
        self.pad_x = gui.padding_x;
        self.pad_y = gui.padding_y;
        self.theme = gui.theme;
        self.rl.set_target_fps(gui.target_fps as u32);
    }
}
