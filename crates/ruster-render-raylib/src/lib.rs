mod key;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use raylib::consts::KeyboardKey;
use raylib::prelude::*;
use ruster_render::{CursorKind, FrameState, GuiConfig, Renderer, SettingRowView};

/// Draw `chars` on the cell grid, one `char_w` apart, colouring each by `colors`.
///
/// The GUI is a cell grid: the cursor, the selection quads, the sign column and
/// the viewport's column count are all placed at `n * char_w`. Text has to be
/// placed the same way. Letting the font's own advances accumulate instead —
/// drawing runs and stepping by their measured width — drifts the moment a glyph
/// whose advance is not `char_w` appears, such as a box-drawing marker or a
/// glyph the font substitutes. Everything after it then sits slightly off from
/// the highlight behind it, which is how the sidebar's selection came to clip
/// its last character.
///
/// One call per glyph costs little more than one per run: `draw_text_ex` already
/// emits a quad per glyph internally, and the embedded terminal has always drawn
/// this way.
fn draw_text_cells<D: RaylibDraw>(
    d: &mut D,
    m: TextMetrics<'_>,
    at: (f32, f32),
    chars: &[char],
    colors: &[Color],
) {
    let (x0, y) = at;
    let mut buf = [0u8; 4];
    for (i, ch) in chars.iter().enumerate() {
        if *ch == ' ' || *ch == '\0' {
            continue; // nothing to draw, and skipping keeps the glyph count down
        }
        let color = colors.get(i).copied().unwrap_or(Color::WHITE);
        let x = x0 + i as f32 * m.char_w;
        d.draw_text_ex(
            m.font,
            ch.encode_utf8(&mut buf),
            Vector2::new(x, y),
            m.size as f32,
            1.0,
            color,
        );
    }
}

/// Read the framebuffer and write it out as a PNG.
///
/// Takes the *draw handle* rather than the raylib handle so it can be called
/// mid-frame, before `EndDrawing` swaps the buffers away. `RaylibDrawHandle`
/// derefs to `RaylibHandle`, so this is the same read either way — only the
/// timing differs, and the timing is the whole point.
fn capture_screen(
    d: &RaylibDrawHandle<'_>,
    thread: &RaylibThread,
    path: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let name = path
        .to_str()
        .ok_or_else(|| format!("{} is not valid UTF-8", path.display()))?;
    // SAFETY: flushes raylib's own draw batch, which is the missing step.
    // raylib queues draw calls and only submits them in `EndDrawing`; reading
    // pixels syncs *GL* but knows nothing about that queue, so whatever was
    // drawn most recently — the dialog, the last overlay — is still pending and
    // simply absent from the image. This is the same call `EndDrawing` makes.
    unsafe { raylib::ffi::rlDrawRenderBatchActive() };
    d.load_image_from_screen(thread).export_image(name);
    // raylib reports a failed export only through its own log, so confirm the
    // file arrived rather than claiming a save that never happened.
    if path.is_file() {
        Ok(path.to_path_buf())
    } else {
        Err(format!("could not write {}", path.display()))
    }
}

/// A 1px border rectangle: `(x, y, w, h)`.
type Edge = (i32, i32, i32, i32);

/// The pixel geometry of a titled box, separated from the drawing so it can be
/// checked without a window — which is the only way these properties get tested
/// at all, since creating a GL context is not available in a unit test.
#[derive(Debug, PartialEq, Eq)]
struct BoxEdges {
    /// The top rule, as one run when untitled and two when a title splits it.
    top: Vec<Edge>,
    left: Edge,
    right: Edge,
    bottom: Edge,
    /// Where the title is drawn, when there is one.
    label_at: Option<(i32, i32)>,
}

/// Lay out a bordered overlay box whose top edge carries the title.
///
/// `label_w` is the measured pixel width of the title, or `None` for an
/// untitled box. Returns `None` when the rect is too small to draw a border in.
///
/// Every edge is a continuous 1px line, and the top one sits at the vertical
/// middle of the header row — where a `─` glyph would render. Mixing the two (a
/// glyph top rule, pixel sides) puts the horizontal half a row above where the
/// verticals begin, so the corners never meet; drawing the sides as per-row `│`
/// glyphs instead makes them a dashed line. Pixels for all four edges is the
/// only combination that is both continuous and joined.
fn box_edges(rect: Edge, line_h: i32, char_w: f32, label_w: Option<i32>) -> Option<BoxEdges> {
    let (x, y, w, h) = rect;
    if w < 4 || h < 2 * line_h {
        return None;
    }
    let rule_y = y + line_h / 2;
    let bottom_y = y + h - 1;
    // Sides start on the rule, so each corner is a single joined pixel.
    let side_h = (bottom_y - rule_y).max(0);

    let (top, label_at) = match label_w {
        Some(label_w) => {
            let label_x = x + (2.0 * char_w) as i32;
            let pad = (char_w * 0.5) as i32;
            let gap_start = label_x - pad;
            let gap_end = (label_x + label_w + pad).min(x + w);
            let mut runs = vec![(x, rule_y, (gap_start - x).max(0), 1)];
            // A title wide enough to reach the far edge leaves no second run.
            if gap_end < x + w {
                runs.push((gap_end, rule_y, x + w - gap_end, 1));
            }
            (runs, Some((label_x, y)))
        }
        None => (vec![(x, rule_y, w, 1)], None),
    };

    Some(BoxEdges {
        top,
        left: (x, rule_y, 1, side_h),
        right: (x + w - 1, rule_y, 1, side_h),
        bottom: (x, bottom_y, w, 1),
        label_at,
    })
}

/// Draw a bordered overlay box whose top edge carries the title, with the sides
/// meeting that rule rather than starting beneath it. The GUI counterpart of the
/// TUI's `titled_box`. Geometry lives in [`box_edges`].
fn draw_titled_box<D: RaylibDraw>(
    d: &mut D,
    m: TextMetrics<'_>,
    rect: Edge,
    line_h: i32,
    label: Option<&str>,
    label_fg: Color,
    rule_fg: Color,
) {
    let label = label.filter(|l| !l.is_empty());
    let label_w = label.map(|l| m.font.measure_text(l, m.size as f32, 1.0).x as i32);
    let Some(edges) = box_edges(rect, line_h, m.char_w, label_w) else {
        return;
    };

    for (x, y, w, h) in edges.top {
        d.draw_rectangle(x, y, w, h, rule_fg);
    }
    if let (Some(label), Some((lx, ly))) = (label, edges.label_at) {
        d.draw_text_ex(
            m.font,
            label,
            Vector2::new(lx as f32, ly as f32),
            m.size as f32,
            1.0,
            label_fg,
        );
    }
    for (x, y, w, h) in [edges.left, edges.right, edges.bottom] {
        d.draw_rectangle(x, y, w, h, rule_fg);
    }
}

/// Draw the standard panel header — `─ label ─` then ruled to the full width.
///
/// Shared by buffer windows, the picker and the settings page so every titled
/// surface reads as the same kind of thing. Placed on the cell grid, like all
/// other text.
#[allow(clippy::too_many_arguments)]
fn draw_ruled_header<D: RaylibDraw>(
    d: &mut D,
    m: TextMetrics<'_>,
    at: (i32, i32),
    width_px: i32,
    label: &str,
    label_fg: Color,
    rule_fg: Color,
) {
    let (x0, y) = at;
    let hdr: Vec<char> = format!("─ {} ─", label).chars().collect();
    let cols = (width_px as f32 / m.char_w).floor().max(0.0) as usize;
    let mut colors = vec![label_fg; hdr.len().min(cols)];
    let mut chars: Vec<char> = hdr.into_iter().take(cols).collect();
    while chars.len() < cols {
        chars.push('─');
        colors.push(rule_fg);
    }
    draw_text_cells(d, m, (x0 as f32, y as f32), &chars, &colors);
}

/// The font metrics every text draw needs; they always travel together.
#[derive(Clone, Copy)]
struct TextMetrics<'a> {
    font: &'a WeakFont,
    size: i32,
    char_w: f32,
}

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
    picker_scroll: usize,
    event_buffer: Vec<KeyEvent>,
    /// Where to write, and how many frames to let settle first.
    ///
    /// The countdown is not cosmetic. A capture on the very first frame after
    /// the window opens comes back black — the GL surface is not ready, and
    /// nothing about the draw calls says so. Letting a couple of frames go by
    /// costs nothing a user would notice and makes the result reliable.
    pending_screenshot: Option<(std::path::PathBuf, u8)>,
    /// The result of that capture, waiting to be polled by the run loop.
    screenshot_result: Option<Result<std::path::PathBuf, String>>,
}

/// Split cmdline text into its prompt sigil and the rest.
///
/// `:` is a command, `/` and `?` are searches. Anything else on this row is
/// output — an echoed message, an error — and gets an empty sigil, so it draws
/// in one colour exactly as before.
pub fn split_prompt_sigil(text: &str) -> (&str, &str) {
    match text.chars().next() {
        Some(c @ (':' | '/' | '?')) => text.split_at(c.len_utf8()),
        _ => ("", text),
    }
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
    pub(crate) fn font_chars() -> String {
        let mut s = String::new();
        // Ranges of codepoints to bake, as inclusive (start, end) pairs.
        const RANGES: &[(u32, u32)] = &[
            (0x20, 0x7E), // printable ASCII
            (0xA0, 0xFF), // Latin-1 supplement
            // Box drawing, block elements *and* geometric shapes. The last of
            // these is easy to stop short of at 0x259F, which silently drops the
            // sidebar's ▸/▾ markers and the debugger's ● breakpoint — they render
            // as `?` in the GUI while looking fine in the TUI.
            (0x2500, 0x25FF),
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

    /// The window icon, embedded so it travels with the binary.
    ///
    /// A path would work from a bundle and fail from `cargo run`, and the icon
    /// is 40 KB — not worth a runtime file lookup that can be wrong.
    const ICON_PNG: &'static [u8] = include_bytes!("../../../assets/icon.png");

    /// Set the window and taskbar icon.
    ///
    /// Downscaled from the 1024x1024 master: this ends up as a taskbar entry,
    /// and handing the compositor a megapixel image for a 32px slot wastes both
    /// memory and the scaler's quality.
    ///
    /// A no-op on macOS by design — GLFW cannot set a window icon there, and
    /// the Dock reads the `.icns` from the `.app` bundle instead, which
    /// `scripts/bundle-macos.sh` already installs. Failure is silent for the
    /// same reason: a missing icon must never stop the editor opening.
    fn apply_window_icon(rl: &mut RaylibHandle) {
        if let Ok(mut img) = raylib::texture::Image::load_image_from_mem(".png", Self::ICON_PNG) {
            img.resize(64, 64);
            rl.set_window_icon(&img);
        }
    }

    pub fn new(title: &str, gui: GuiConfig, font_override: Option<&str>) -> Self {
        let (mut rl, thread) = raylib::init()
            .size(gui.window_width, gui.window_height)
            .title(title)
            // Errors only — silence raylib's per-glyph font warnings and info logs.
            .log_level(raylib::ffi::TraceLogLevel::LOG_ERROR)
            .resizable()
            .build();
        Self::apply_window_icon(&mut rl);
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
            picker_scroll: 0,
            event_buffer: Vec::new(),
            pending_screenshot: None,
            screenshot_result: None,
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
                    self.event_buffer
                        .push(KeyEvent::new(KeyCode::Char(ch), mods));
                } else if let Some(event) = key::map_raylib_key(k) {
                    self.event_buffer.push(KeyEvent::new(event.code, mods));
                }
            }
        } else {
            while let Some(c) = self.rl.get_char_pressed() {
                if let Some(ch) = char::from_u32(c as u32) {
                    self.event_buffer
                        .push(KeyEvent::new(KeyCode::Char(ch), mods));
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
        let (font_size, line_h, pad_x, pad_y) =
            (self.font_size, self.line_h, self.pad_x, self.pad_y);
        let theme = state.theme;
        let font = &self.font;
        let measure = |s: &str| font.measure_text(s, font_size as f32, 1.0).x;
        let metrics = TextMetrics {
            font,
            size: font_size,
            char_w,
        };

        let default_color = to_raylib(theme.fg, Color::new(205, 214, 244, 255));
        let gutter_color = to_raylib(theme.gutter, Color::new(108, 112, 134, 255));
        let divider = to_raylib(theme.divider, Color::new(69, 71, 90, 255));
        let bg = to_raylib(theme.bg, Color::new(30, 30, 30, 255));
        let gutter_bg = to_raylib(theme.gutter_bg, bg);
        let selection_bg = to_raylib(theme.selection_bg, Color::new(88, 91, 112, 255));
        let selection_fg = to_raylib(theme.selection_fg, default_color);
        let (cur_r, cur_g, cur_b) = rgb_of(theme.cursor_bg, (245, 224, 220));
        let cursor_fg = to_raylib(theme.cursor_fg, bg);
        let accent = to_raylib(theme.accent, Color::new(243, 139, 168, 255));
        let accent_fg = to_raylib(theme.accent_fg, bg);
        let statusline_fg = to_raylib(theme.statusline_fg, default_color);
        let statusline_bg = to_raylib(theme.statusline_bg, divider);
        let whichkey_bg = to_raylib(theme.whichkey_bg, Color::new(30, 30, 46, 255));
        let whichkey_fg = to_raylib(theme.whichkey_fg, default_color);
        let whichkey_key = to_raylib(theme.whichkey_key, accent);
        let cmdline_bg = to_raylib(theme.cmdline_bg, bg);
        let cmdline_fg = to_raylib(theme.cmdline_fg, default_color);

        let mut d = self.rl.begin_drawing(&self.thread);
        d.clear_background(bg);

        // Collect and sort by x to find adjacent windows for vertical seams.
        let mut window_list: Vec<&ruster_render::WindowView> = state
            .windows
            .iter()
            .filter(|v| v.rect.width > 0 && v.rect.height > 0)
            .collect();
        window_list.sort_by_key(|v| v.rect.x);

        for (win_idx, view) in window_list.iter().enumerate() {
            let px = pad_x + (view.rect.x as f32 * char_w) as i32;
            let py = pad_y + view.rect.y as i32 * line_h;
            let pw = (view.rect.width as f32 * char_w) as i32;
            // Header row + content rows + statusline row.
            let buf_rows = view.rect.height.saturating_sub(2) as usize;
            let content_y = py + line_h; // after the header
                                         // Layout left-to-right: sign column, then line-number gutter, then text.
            let sign_x = px;
            let gutter_x = px + (view.signs.width as f32 * char_w) as i32;
            let text_x = gutter_x + (view.gutter.width as f32 * char_w) as i32;
            let scroll = view.scroll_offset as usize;
            let win_h = view.rect.height as i32 * line_h;

            // Panel header: draw a ruled line with the filename as stencil label
            // before the scissor region (the header spans the full window width).
            let label = if view.header.is_empty() {
                "untitled"
            } else {
                &view.header
            };
            let hdr_color = if view.active { accent } else { divider };
            d.draw_rectangle(px, py, pw, line_h, bg);
            draw_ruled_header(&mut d, metrics, (px, py), pw, label, hdr_color, divider);

            // Vertical seam on the right edge of each window (drawn over the gap).
            if win_idx < window_list.len() - 1 {
                let seam_x = px + pw - 1;
                for iy in py..py + win_h {
                    d.draw_rectangle(seam_x, iy, 1, 1, divider);
                }
            }

            // Clip everything in this window's content + statusline to its rect.
            let clip_h = win_h - line_h; // exclude header
            {
                let mut s = d.begin_scissor_mode(px, content_y, pw, clip_h);

                // Welcome / "Dashboard" screen — replaces buffer content when
                // no named file is open.
                if let Some(welcome) = &state.welcome {
                    if welcome.visible {
                        // Fill the content area with the background colour.
                        s.draw_rectangle(px, content_y, pw, clip_h, bg);
                        let mut row = 0;
                        let cx = px + (pw as f32 / 2.0) as i32;
                        let draw_text =
                            |s: &mut RaylibDrawHandle, x: i32, r: i32, text: &str, color: Color| {
                                s.draw_text_ex(
                                    font,
                                    text,
                                    Vector2::new(x as f32, (content_y + r * line_h) as f32),
                                    font_size as f32,
                                    1.0,
                                    color,
                                );
                            };
                        let _dimmer = Color::new(0, 0, 0, 0);

                        let title = format!("RUSTER  {}", welcome.version);
                        let tx = cx - (measure(&title) / 2.0) as i32;
                        draw_text(&mut s, tx, row, &title, default_color);
                        row += 1;
                        let rr = "DASHBOARD";
                        let rx = cx - (measure(rr) / 2.0) as i32;
                        draw_text(&mut s, rx, row, rr, accent);
                        row += 2;

                        let section =
                            |s: &mut RaylibDrawHandle, r: &mut i32, label: &str, color: Color| {
                                let hdr = format!(" ▌{}▐ ", label);
                                draw_text(s, px + 4, *r, &hdr, color);
                                *r += 1;
                            };

                        section(&mut s, &mut row, "RECENT PROJECTS", accent);
                        if welcome.recent_projects.is_empty() {
                            draw_text(&mut s, px + 8, row, "  No recent projects", gutter_color);
                            row += 1;
                        } else {
                            for (i, proj) in welcome.recent_projects.iter().enumerate() {
                                draw_text(
                                    &mut s,
                                    px + 8,
                                    row,
                                    &format!(" {}. {}", i + 1, proj),
                                    default_color,
                                );
                                row += 1;
                            }
                        }
                        row += 1;

                        section(&mut s, &mut row, "QUICK ACTIONS", accent);
                        for (cmd, desc) in &[
                            (":e <path>", "Open file (Tab to complete)"),
                            (":Dired", "File Explorer"),
                            (":Files", "Find Files"),
                            (":term", "Terminal"),
                        ] {
                            let dl = measure(cmd) + 4.0;
                            draw_text(&mut s, px + 8, row, cmd, default_color);
                            draw_text(&mut s, px + 8 + dl as i32, row, desc, gutter_color);
                            row += 1;
                        }
                        row += 1;

                        section(&mut s, &mut row, "SYSTEM STATUS", accent);
                        let lsp_text = format!("  LSP  {}", welcome.lsp_status);
                        draw_text(&mut s, px + 8, row, &lsp_text, default_color);
                        row += 1;
                        let mode_text = format!("  Mode: {}", welcome.edit_mode);
                        draw_text(&mut s, px + 8, row, &mode_text, default_color);
                        row += 2;

                        section(&mut s, &mut row, "KEYBINDS", accent);
                        for (key, desc) in &[
                            ("Ctrl+P  ", "Fuzzy Finder"),
                            ("Ctrl+S  ", "Save"),
                            ("Ctrl+W  ", "Window Commands"),
                            (":help  ", "Help"),
                        ] {
                            draw_text(&mut s, px + 8, row, key, default_color);
                            let kx = px + 8 + measure(key) as i32;
                            draw_text(&mut s, kx, row, desc, gutter_color);
                            row += 1;
                        }
                    }
                } else if let Some(grid) = &view.terminal {
                    // An embedded terminal: a background quad per cell, then the
                    // glyph, then a block cursor. No gutter/scroll/selection.
                    for r in 0..grid.rows.min(buf_rows) {
                        let gy = content_y + r as i32 * line_h;
                        for c in 0..grid.cols {
                            let tc = grid.cells[r * grid.cols + c];
                            let cx = px + (c as f32 * char_w) as i32;
                            let (mut fg, mut bg) = (tc.fg, tc.bg);
                            if tc.inverse {
                                std::mem::swap(&mut fg, &mut bg);
                            }
                            if let ruster_render::Color::Rgb(rr, gg, bb) = bg {
                                s.draw_rectangle(
                                    cx,
                                    gy,
                                    char_w.ceil() as i32,
                                    line_h,
                                    Color::new(rr, gg, bb, 255),
                                );
                            }
                            if tc.c != ' ' && tc.c != '\0' {
                                let color = match fg {
                                    ruster_render::Color::Rgb(rr, gg, bb) => {
                                        Color::new(rr, gg, bb, 255)
                                    }
                                    ruster_render::Color::Default => default_color,
                                };
                                let mut ch = [0u8; 4];
                                s.draw_text_ex(
                                    font,
                                    tc.c.encode_utf8(&mut ch),
                                    Vector2::new(cx as f32, gy as f32),
                                    font_size as f32,
                                    1.0,
                                    color,
                                );
                            }
                        }
                    }
                    if view.cursor_visible && view.active {
                        let (cr, cc) = grid.cursor;
                        if cr < buf_rows && cc < grid.cols {
                            let cx = px + (cc as f32 * char_w) as i32;
                            let cy = content_y + cr as i32 * line_h;
                            s.draw_rectangle(
                                cx,
                                cy,
                                char_w as i32,
                                line_h,
                                Color::new(cur_r, cur_g, cur_b, 160),
                            );
                        }
                    }
                } else {
                    // Gutter background (only when a gutter is shown).
                    if view.gutter.width > 0 && gutter_bg != bg {
                        s.draw_rectangle(
                            gutter_x,
                            content_y,
                            text_x - gutter_x,
                            buf_rows as i32 * line_h,
                            gutter_bg,
                        );
                    }
                    // Sign column, left of the gutter.
                    if view.signs.width > 0 {
                        for row in 0..buf_rows {
                            let line = (row + scroll) as u16;
                            if let Some((glyph, c)) = view.signs.at(line) {
                                let gy = content_y + row as i32 * line_h;
                                let color = to_raylib(c, default_color);
                                let mut b = [0u8; 4];
                                s.draw_text_ex(
                                    font,
                                    glyph.encode_utf8(&mut b),
                                    Vector2::new(sign_x as f32, gy as f32),
                                    font_size as f32,
                                    1.0,
                                    color,
                                );
                            }
                        }
                    }
                    // Gutter column.
                    for (row, label) in view.gutter.rows.iter().take(buf_rows).enumerate() {
                        let gy = content_y + row as i32 * line_h;
                        s.draw_text_ex(
                            font,
                            label,
                            Vector2::new(gutter_x as f32, gy as f32),
                            font_size as f32,
                            1.0,
                            gutter_color,
                        );
                    }

                    // Visual-mode selection background, behind the text.
                    if let Some(sel) = view.selection {
                        for (row, line) in view.lines.iter().skip(scroll).take(buf_rows).enumerate()
                        {
                            let buffer_line = (row + scroll) as u16;
                            let line_len = line.text.chars().count() as u16;
                            if let Some((sel_start, sel_end)) = sel.span_on(buffer_line, line_len) {
                                let gy = content_y + row as i32 * line_h;
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
                        let gy = content_y + row as i32 * line_h;
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
                            s.draw_text_ex(
                                font,
                                &line.text,
                                Vector2::new(text_x as f32, gy as f32),
                                font_size as f32,
                                1.0,
                                default_color,
                            );
                            continue;
                        }
                        // Color per *character* (highlight offsets are char offsets),
                        // then draw same-color runs — safe for multibyte lines.
                        let chars: Vec<char> = line.text.chars().collect();
                        let nchars = chars.len();
                        // Draw highlight backgrounds first.
                        for &(offset, len, ref style) in &line.highlights {
                            if let ruster_render::Color::Rgb(r, g, b) = style.bg {
                                let end = (offset + len).min(nchars);
                                if offset < end {
                                    let sx = text_x as f32 + offset as f32 * char_w;
                                    let sw = (end - offset) as f32 * char_w;
                                    s.draw_rectangle(
                                        sx as i32,
                                        gy,
                                        sw as i32,
                                        line_h,
                                        Color::new(r, g, b, 255),
                                    );
                                }
                            }
                        }
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
                        draw_text_cells(
                            &mut s,
                            metrics,
                            (text_x as f32, gy as f32),
                            &chars,
                            &char_colors,
                        );
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
                            let mut cy = content_y + vis_row * line_h;
                            if let Some((dcx, dcy)) = view.cursor_smooth {
                                cx += dcx * char_w;
                                cy = (cy as f32 + dcy * line_h as f32) as i32;
                            }
                            let cx = cx as i32;
                            match view.cursor_kind {
                                CursorKind::Block => {
                                    // Solid block, then redraw the glyph under it in the
                                    // cursor text color (classic block-cursor look).
                                    s.draw_rectangle(
                                        cx,
                                        cy,
                                        char_w as i32,
                                        line_h,
                                        Color::new(cur_r, cur_g, cur_b, 255),
                                    );
                                    if let Some(ch) =
                                        view.lines.get(cline).and_then(|l| l.text.chars().nth(col))
                                    {
                                        if ch != ' ' {
                                            let mut buf = [0u8; 4];
                                            s.draw_text_ex(
                                                font,
                                                ch.encode_utf8(&mut buf),
                                                Vector2::new(cx as f32, cy as f32),
                                                font_size as f32,
                                                1.0,
                                                cursor_fg,
                                            );
                                        }
                                    }
                                }
                                CursorKind::Bar => s.draw_rectangle(
                                    cx,
                                    cy,
                                    2,
                                    line_h,
                                    Color::new(cur_r, cur_g, cur_b, 255),
                                ),
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
                            let cy = content_y + vis_row * line_h;
                            s.draw_rectangle(
                                cx as i32,
                                cy,
                                char_w as i32,
                                line_h,
                                Color::new(cur_r, cur_g, cur_b, 140),
                            );
                        }
                    }

                    // Flash jump labels, painted over the text they target.
                    for fl in &view.flash_labels {
                        if fl.row as usize >= buf_rows {
                            continue;
                        }
                        let lx = text_x + (fl.col as f32 * char_w) as i32;
                        let ly = content_y + fl.row as i32 * line_h;
                        let lw = (measure(&fl.text) as i32).max(char_w as i32);
                        let color = match fl.color {
                            ruster_render::Color::Rgb(r, g, b) => Color::new(r, g, b, 255),
                            ruster_render::Color::Default => default_color,
                        };
                        s.draw_rectangle(lx, ly, lw, line_h, accent);
                        s.draw_text_ex(
                            font,
                            &fl.text,
                            Vector2::new(lx as f32, ly as f32),
                            font_size as f32,
                            1.0,
                            color,
                        );
                    }
                } // end: buffer vs. terminal drawing

                // Per-window statusline on its bottom row (below header + content).
                let sl_y = content_y + buf_rows as i32 * line_h;
                let mode_bg = to_raylib(theme.mode_bg(view.statusline.mode), statusline_bg);
                let mode_fg = to_raylib(theme.mode_fg(view.statusline.mode), statusline_fg);
                // Fill entire statusline with neutral bg.
                s.draw_rectangle(px, sl_y, pw, line_h, statusline_bg);
                // Mode label (left section) — per-mode bg + fg.
                let left = format!(" {} ", view.statusline.left);
                let left_w = measure(&left);
                let (sl_bg, sl_fg) = if view.active {
                    (mode_bg, mode_fg)
                } else {
                    let c = mode_bg;
                    (
                        Color::new(
                            c.r.saturating_sub(20),
                            c.g.saturating_sub(20),
                            c.b.saturating_sub(20),
                            c.a,
                        ),
                        gutter_color,
                    )
                };
                s.draw_rectangle(px, sl_y, left_w as i32, line_h, sl_bg);
                s.draw_text_ex(
                    font,
                    &left,
                    Vector2::new(px as f32, sl_y as f32),
                    font_size as f32,
                    1.0,
                    sl_fg,
                );
                // Right section — neutral bg + statusline_fg (or gutter for inactive).
                let right = format!(" {} ", view.statusline.right);
                let right_x = (px + pw) as f32 - measure(&right);
                let right_fg = if view.active {
                    statusline_fg
                } else {
                    gutter_color
                };
                s.draw_text_ex(
                    font,
                    &right,
                    Vector2::new(right_x, sl_y as f32),
                    font_size as f32,
                    1.0,
                    right_fg,
                );
                // Center section — neutral bg + statusline_fg (or gutter for inactive).
                if !view.statusline.center.is_empty() {
                    let center_w = measure(&view.statusline.center);
                    let right_w = measure(&right);
                    if let Some(off) =
                        ruster_render::statusline_center_x(pw as f32, left_w, center_w, right_w)
                    {
                        s.draw_text_ex(
                            font,
                            &view.statusline.center,
                            Vector2::new(px as f32 + off, sl_y as f32),
                            font_size as f32,
                            1.0,
                            right_fg,
                        );
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
        if let Some(cmd) = state.cmdline {
            let rows = ((screen_h - pad_y) / line_h).max(1);
            let cmd_y = pad_y + (rows - 1) * line_h;
            d.draw_rectangle(0, cmd_y, screen_w, screen_h - cmd_y, cmdline_bg);
            // Tint the leading sigil so a prompt is distinguishable from a
            // message, which shares this row. Drawn as two runs because a glyph
            // cannot carry two colours; the rest starts one cell in, which is
            // the same fixed advance the rest of this renderer lays text on.
            let (sigil, rest) = split_prompt_sigil(cmd);
            let mut x = pad_x as f32;
            if !sigil.is_empty() {
                d.draw_text_ex(
                    font,
                    sigil,
                    Vector2::new(x, cmd_y as f32),
                    font_size as f32,
                    1.0,
                    to_raylib(theme.cmdline_accent, cmdline_fg),
                );
                x += char_w;
            }
            d.draw_text_ex(
                font,
                rest,
                Vector2::new(x, cmd_y as f32),
                font_size as f32,
                1.0,
                cmdline_fg,
            );
        }

        // Picker overlay: centered floating box, or a full-width strip docked at
        // the bottom (the command palette's "bottom" mode).
        if let Some(picker) = &state.picker {
            let has_preview = !picker.preview.is_empty();
            let frac = if has_preview { 9 } else { 6 };
            let n_rows = (picker.rows.len() as i32 + 2).max(picker.preview.len() as i32);
            let bottom = matches!(picker.placement, ruster_render::PickerPlacement::Bottom);
            let (box_x, box_y, box_w, box_h) = if bottom {
                let h = (n_rows * line_h).clamp(3 * line_h, (screen_h / 2).max(3 * line_h));
                (0, screen_h - h, screen_w, h)
            } else {
                let w = (screen_w * frac / 10).clamp(240.min(screen_w), screen_w - 20);
                let h = (n_rows * line_h).clamp(3 * line_h, (screen_h - 40).max(3 * line_h));
                ((screen_w - w) / 2, ((screen_h - h) / 2).max(0), w, h)
            };
            let list_w = if has_preview { box_w * 2 / 5 } else { box_w };
            d.draw_rectangle(box_x, box_y, box_w, box_h, bg);
            if has_preview {
                d.draw_rectangle(box_x + list_w, box_y, box_w - list_w, box_h, bg);
                // Divider starts below the header, which rules across the top.
                // Meets the top rule at its midpoint, like the outer edges.
                let div_y = box_y + line_h / 2;
                d.draw_rectangle(box_x + list_w, div_y, 1, box_h - line_h / 2, accent);
            }
            // Drawn before the column scissors so it spans the whole box.
            draw_titled_box(
                &mut d,
                metrics,
                (box_x, box_y, box_w, box_h),
                line_h,
                Some(&picker.title),
                accent,
                divider,
            );
            // List column — title, query, and rows, clipped to the list width
            // so long labels don't bleed across the divider into the preview.
            let list_clip_w = if has_preview { list_w } else { box_w };
            {
                let mut s =
                    d.begin_scissor_mode(box_x + 1, box_y + 1, (list_clip_w - 2).max(1), box_h - 2);
                s.draw_text_ex(
                    font,
                    &format!(" > {}", picker.query),
                    Vector2::new(box_x as f32 + 4.0, (box_y + line_h) as f32),
                    font_size as f32,
                    1.0,
                    default_color,
                );
                let max_visible = ((box_h - 2 * line_h) / line_h).max(0) as usize;
                // Keep the selection on screen; a wrap to the last item has to
                // take the view with it.
                let sel = picker.rows.iter().position(|r| r.selected).unwrap_or(0);
                self.picker_scroll = ruster_render::list_scroll(
                    self.picker_scroll,
                    sel,
                    max_visible,
                    picker.rows.len(),
                );
                let pscroll = self.picker_scroll;
                for (i, row) in picker
                    .rows
                    .iter()
                    .skip(pscroll)
                    .take(max_visible)
                    .enumerate()
                {
                    let ry = box_y + (2 + i as i32) * line_h;
                    if row.selected {
                        s.draw_rectangle(box_x, ry, list_clip_w, line_h, accent);
                        s.draw_text_ex(
                            font,
                            &format!(" {}", row.label),
                            Vector2::new(box_x as f32 + 4.0, ry as f32),
                            font_size as f32,
                            1.0,
                            accent_fg,
                        );
                    } else {
                        s.draw_text_ex(
                            font,
                            &format!(" {}", row.label),
                            Vector2::new(box_x as f32 + 4.0, ry as f32),
                            font_size as f32,
                            1.0,
                            default_color,
                        );
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
                    let ly = box_y + (1 + i as i32) * line_h;
                    if ly > box_y + box_h {
                        break;
                    }
                    let n = line.text.len();
                    if n == 0 {
                        continue;
                    }
                    if line.highlights.is_empty() {
                        s.draw_text_ex(
                            font,
                            &line.text,
                            Vector2::new(px as f32, ly as f32),
                            font_size as f32,
                            1.0,
                            default_color,
                        );
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
                    draw_text_cells(
                        &mut s,
                        metrics,
                        (px as f32, ly as f32),
                        &chars,
                        &char_colors,
                    );
                }
            }
        }

        // Noice mini toasts, stacked down the top-right corner.
        for (i, text) in state.noice_mini.iter().enumerate() {
            let tw = measure(text) as i32 + 12;
            let tx = screen_w - tw - pad_x;
            let ty = pad_y + i as i32 * line_h;
            if ty + line_h > screen_h {
                break;
            }
            d.draw_rectangle(tx, ty, tw, line_h, whichkey_bg);
            d.draw_rectangle(tx, ty, 2, line_h, accent);
            d.draw_text_ex(
                font,
                text,
                Vector2::new((tx + 6) as f32, ty as f32),
                font_size as f32,
                1.0,
                whichkey_fg,
            );
        }

        // Noice notify panel: the notification history, docked right.
        if let Some(lines) = &state.noice_notify {
            let panel_w = (screen_w / 3).min(420);
            let panel_x = screen_w - panel_w;
            d.draw_rectangle(panel_x, 0, panel_w, screen_h, whichkey_bg);
            d.draw_rectangle(panel_x, 0, 2, screen_h, accent);
            let mut s = d.begin_scissor_mode(panel_x, 0, panel_w, screen_h);
            for (i, line) in lines.iter().enumerate() {
                let ly = pad_y + i as i32 * line_h;
                if ly + line_h > screen_h {
                    break;
                }
                s.draw_text_ex(
                    font,
                    &line.text,
                    Vector2::new((panel_x + 6) as f32, ly as f32),
                    font_size as f32,
                    1.0,
                    whichkey_fg,
                );
            }
        }

        // Debugger panel, docked right so it doesn't cover the stopped line.
        if let Some(dbg) = &state.debug_overlay {
            let rows = dbg.rows();
            let panel_w = (screen_w / 3).min(460);
            let panel_x = screen_w - panel_w;
            let panel_h = ((rows.len() as i32 + 1) * line_h + 8).min(screen_h);
            d.draw_rectangle(panel_x, 0, panel_w, panel_h, whichkey_bg);
            // Toolbar bar across the top of the panel.
            d.draw_rectangle(panel_x, 0, panel_w, line_h, accent);
            let mut s = d.begin_scissor_mode(panel_x, 0, panel_w, panel_h);
            s.draw_text_ex(
                font,
                &dbg.toolbar,
                Vector2::new((panel_x + 6) as f32, 0.0),
                font_size as f32,
                1.0,
                accent_fg,
            );
            for (i, row) in rows.iter().enumerate() {
                let ry = (i as i32 + 1) * line_h + 4;
                if ry + line_h > panel_h {
                    break;
                }
                // Detail rows are dimmed so section headings stand out.
                let color = if row.starts_with(' ') || row.starts_with(|c: char| c.is_ascii_digit())
                {
                    gutter_color
                } else {
                    whichkey_fg
                };
                s.draw_text_ex(
                    font,
                    row,
                    Vector2::new((panel_x + 6) as f32, ry as f32),
                    font_size as f32,
                    1.0,
                    color,
                );
            }
        }

        // Hover popup, near the top-center (syntax-highlighted).
        // Bottom which-key panel, sliding up from the screen edge by `anim`.
        if let Some(wk) = &state.whichkey {
            let panel_h = (wk.rows.len() as i32 + 1) * line_h + 8;
            let panel_top = screen_h - (panel_h as f32 * wk.anim.clamp(0.0, 1.0)) as i32;
            // Clip to the visible (slid-in) region so nothing draws above it.
            let mut s = d.begin_scissor_mode(0, panel_top, screen_w, screen_h - panel_top);
            s.draw_rectangle(0, panel_top, screen_w, screen_h - panel_top, whichkey_bg);
            s.draw_rectangle(0, panel_top, screen_w, 2, accent);
            s.draw_text_ex(
                font,
                &format!(" {} ", wk.title),
                Vector2::new(pad_x as f32, (panel_top + 4) as f32),
                font_size as f32,
                1.0,
                accent,
            );
            for (i, entry) in wk.rows.iter().enumerate() {
                let ry = panel_top + 4 + (i as i32 + 1) * line_h;
                s.draw_text_ex(
                    font,
                    &entry.key,
                    Vector2::new(pad_x as f32, ry as f32),
                    font_size as f32,
                    1.0,
                    whichkey_key,
                );
                let kx = pad_x + 6 + font.measure_text(&entry.key, font_size as f32, 1.0).x as i32;
                s.draw_text_ex(
                    font,
                    &entry.desc,
                    Vector2::new(kx as f32, ry as f32),
                    font_size as f32,
                    1.0,
                    whichkey_fg,
                );
            }
        }

        // Settings page — a large centered overlay, themed from the live palette.
        if let Some(settings) = &state.settings {
            let sbg = bg;
            let sel_bg = selection_bg;
            let bar_bg = statusline_bg;
            let bw = screen_w * 8 / 10;
            let bh = screen_h * 9 / 10;
            let bx = (screen_w - bw) / 2;
            let by = (screen_h - bh) / 2;
            let mut s = d.begin_scissor_mode(bx, by, bw, bh);
            s.draw_rectangle(bx, by, bw, bh, sbg);
            let title = format!("Settings{}", if settings.dirty { " [+]" } else { "" });
            draw_titled_box(
                &mut s,
                metrics,
                (bx, by, bw, bh),
                line_h,
                Some(&title),
                accent,
                divider,
            );

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
                ruster_render::list_scroll(self.settings_scroll, selected, body_rows, lines.len());
            let scroll = self.settings_scroll;
            let value_x = (bx + (32.0 * char_w) as i32).min(bx + bw / 2);

            for (i, (is_h, label, row)) in lines.iter().skip(scroll).take(body_rows).enumerate() {
                let ry = by + line_h + i as i32 * line_h;
                if *is_h {
                    s.draw_text_ex(
                        font,
                        &format!("── {} ", label.to_uppercase()),
                        Vector2::new((bx + 4) as f32, ry as f32),
                        font_size as f32,
                        1.0,
                        accent,
                    );
                } else if let Some(r) = row {
                    // The selected row sits on the selection bar, so its text
                    // uses the selection-text colour.
                    let row_fg = if r.selected {
                        selection_fg
                    } else {
                        default_color
                    };
                    if r.selected {
                        s.draw_rectangle(bx, ry, bw, line_h, sel_bg);
                    }
                    s.draw_text_ex(
                        font,
                        label,
                        Vector2::new((bx + 8) as f32, ry as f32),
                        font_size as f32,
                        1.0,
                        row_fg,
                    );
                    let ctrl = ruster_render::control_display(r);
                    let cc = if r.editing { accent } else { row_fg };
                    s.draw_text_ex(
                        font,
                        &ctrl,
                        Vector2::new(value_x as f32, ry as f32),
                        font_size as f32,
                        1.0,
                        cc,
                    );
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
                s.draw_text_ex(
                    font,
                    &r.help,
                    Vector2::new((bx + 4) as f32, hy as f32),
                    font_size as f32,
                    1.0,
                    gutter_color,
                );
            }
            let fy = by + bh - line_h;
            s.draw_rectangle(bx, fy, bw, line_h, bar_bg);
            // The footer is a bar, so its text uses the bar/divider text colour.
            s.draw_text_ex(
                font,
                &settings.footer,
                Vector2::new((bx + 4) as f32, fy as f32),
                font_size as f32,
                1.0,
                statusline_fg,
            );
        }

        // Floats, then the dialog: a modal is the thing with focus, so it
        // draws last and obscures anything under it. The order used to be the
        // other way round while the comment below claimed this one — inert,
        // because the only float is the hover popup and it never coexists
        // with a dialog, but the code and the comment disagreed and one of
        // them had to be wrong.
        //
        // Lowest z first. The rects are already resolved and clamped in cell
        // coordinates by FloatView, so this only converts to pixels and
        // paints — the geometry is shared with the TUI backend rather than
        // reimplemented here.
        for f in ruster_render::floats_in_draw_order(&state.floats) {
            let fx = pad_x + (f.rect.x as f32 * char_w) as i32;
            let fy = pad_y + f.rect.y as i32 * line_h;
            let fw = (f.rect.width as f32 * char_w) as i32;
            let fh = f.rect.height as i32 * line_h;
            d.draw_rectangle(fx, fy, fw, fh, bg);
            if f.border {
                // Same box as every other overlay, so a float's title sits in a
                // gap in its top edge rather than painted over the border.
                draw_titled_box(
                    &mut d,
                    metrics,
                    (fx, fy, fw, fh),
                    line_h,
                    f.title.as_deref(),
                    accent,
                    accent,
                );
            }
            let inner = f.inner();
            let ix = pad_x + (inner.x as f32 * char_w) as i32;
            let iy = pad_y + inner.y as i32 * line_h;
            let mut s = d.begin_scissor_mode(
                ix,
                iy,
                (inner.width as f32 * char_w) as i32,
                inner.height as i32 * line_h,
            );
            for (row, line) in f.lines.iter().enumerate() {
                if row as u16 >= inner.height {
                    break;
                }
                let ly = iy + row as i32 * line_h;
                if line.text.is_empty() {
                    continue;
                }
                if line.highlights.is_empty() {
                    s.draw_text_ex(
                        font,
                        &line.text,
                        Vector2::new(ix as f32, ly as f32),
                        font_size as f32,
                        1.0,
                        default_color,
                    );
                    continue;
                }
                let chars: Vec<char> = line.text.chars().collect();
                let mut char_colors: Vec<Color> = vec![default_color; chars.len()];
                for &(offset, len, ref style) in &line.highlights {
                    let fg = to_raylib(style.fg, default_color);
                    for c in char_colors.iter_mut().skip(offset).take(len) {
                        *c = fg;
                    }
                }
                draw_text_cells(
                    &mut s,
                    metrics,
                    (ix as f32, ly as f32),
                    &chars,
                    &char_colors,
                );
            }
        }

        // The dialog, above the floats: same titled box and the same setting-row
        // vocabulary the settings page uses.
        if let Some(dlg) = &state.dialog {
            let dw = (screen_w * 6 / 10).clamp(300.min(screen_w), screen_w - 40);
            let dh = ((dlg.rows.len() as i32 + 4) * line_h).min(screen_h - 40);
            let dx = (screen_w - dw) / 2;
            let dy = (screen_h - dh) / 2;
            d.draw_rectangle(dx, dy, dw, dh, bg);
            draw_titled_box(
                &mut d,
                metrics,
                (dx, dy, dw, dh),
                line_h,
                Some(&dlg.title),
                accent,
                divider,
            );
            let value_x = dx + (26.0 * char_w) as i32;
            for (i, r) in dlg.rows.iter().enumerate() {
                let ry = dy + (1 + i as i32) * line_h;
                if ry > dy + dh - 2 * line_h {
                    break;
                }
                let (rfg, rbg) = if r.selected {
                    (selection_fg, Some(selection_bg))
                } else {
                    (default_color, None)
                };
                if let Some(b) = rbg {
                    d.draw_rectangle(dx + 1, ry, dw - 2, line_h, b);
                }
                let shown = ruster_render::control_display(r);
                if r.kind == ruster_render::ControlKind::Button {
                    // A button is one thing, not a label with a value beside it.
                    d.draw_text_ex(
                        font,
                        &shown,
                        Vector2::new((dx + 8) as f32, ry as f32),
                        font_size as f32,
                        1.0,
                        rfg,
                    );
                } else {
                    d.draw_text_ex(
                        font,
                        &r.label,
                        Vector2::new((dx + 8) as f32, ry as f32),
                        font_size as f32,
                        1.0,
                        rfg,
                    );
                    let vfg = if r.editing { accent } else { rfg };
                    d.draw_text_ex(
                        font,
                        &shown,
                        Vector2::new(value_x as f32, ry as f32),
                        font_size as f32,
                        1.0,
                        vfg,
                    );
                }
            }
            let fy = dy + dh - 2 * line_h;
            d.draw_text_ex(
                font,
                &dlg.footer,
                Vector2::new((dx + 8) as f32, fy as f32),
                font_size as f32,
                1.0,
                gutter_color,
            );
        }

        // Capture *before* the draw handle drops.
        //
        // Dropping it runs `EndDrawing`, which swaps the buffers — so reading
        // afterwards reads the new back buffer, which holds the frame from two
        // ago, or nothing at all on the first frame. That is a black image, and
        // it is what this originally produced. Everything for this frame is
        // drawn by now, so reading here gets the completed picture.
        match self.pending_screenshot.take() {
            Some((path, 0)) => {
                let result = capture_screen(&d, &self.thread, &path);
                drop(d);
                self.screenshot_result = Some(result);
            }
            // Not settled yet — see the field's comment.
            Some((path, waiting)) => {
                self.pending_screenshot = Some((path, waiting - 1));
                drop(d);
            }
            None => drop(d),
        }
    }

    fn request_screenshot(&mut self, path: &std::path::Path) -> bool {
        // Two frames of settling: enough for a window that has just opened.
        self.pending_screenshot = Some((path.to_path_buf(), 1));
        true
    }

    fn poll_screenshot(&mut self) -> Option<Result<std::path::PathBuf, String>> {
        self.screenshot_result.take()
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

#[cfg(test)]
mod tests {
    /// The sigil split decides which cmdline rows get a tinted first glyph.
    /// Getting it wrong either paints an echoed message as though it were a
    /// prompt, or leaves a prompt looking like output.
    #[test]
    fn only_prompt_sigils_are_split_off() {
        use super::split_prompt_sigil;
        assert_eq!(split_prompt_sigil(":w"), (":", "w"));
        assert_eq!(split_prompt_sigil("/needle"), ("/", "needle"));
        assert_eq!(split_prompt_sigil("?needle"), ("?", "needle"));
        // A bare sigil is still a prompt — it is what you see mid-keystroke.
        assert_eq!(split_prompt_sigil(":"), (":", ""));
        // Output, not a prompt.
        assert_eq!(
            split_prompt_sigil("written 3 lines"),
            ("", "written 3 lines")
        );
        assert_eq!(
            split_prompt_sigil("E486: pattern not found"),
            ("", "E486: pattern not found")
        );
        assert_eq!(split_prompt_sigil(""), ("", ""));
    }

    use super::{box_edges, RaylibRenderer};

    /// Metrics matching the shipped defaults, so the numbers below are the ones
    /// the GUI actually uses.
    const LINE_H: i32 = 20;
    const CHAR_W: f32 = 10.0;

    /// A titled float: 200px wide at (100, 50), 6 rows tall, title 40px wide.
    fn titled() -> super::BoxEdges {
        box_edges((100, 50, 200, 6 * LINE_H), LINE_H, CHAR_W, Some(40)).expect("big enough")
    }

    /// The regression this whole rework was for: the sides used to begin below
    /// the top rule, leaving both upper corners visibly open.
    #[test]
    fn all_four_corners_meet() {
        let e = titled();
        let rule_y = e.top[0].1;
        let (lx, ly, _, lh) = e.left;
        let (rx, ry, _, rh) = e.right;
        let (bx, by, bw, _) = e.bottom;

        // Sides start *on* the rule row, not under it.
        assert_eq!(ly, rule_y, "left side starts on the top rule");
        assert_eq!(ry, rule_y, "right side starts on the top rule");
        // And run down to the bottom rule, which spans the full width.
        assert_eq!(ly + lh, by, "left side reaches the bottom rule");
        assert_eq!(ry + rh, by, "right side reaches the bottom rule");
        assert_eq!((bx, bx + bw), (lx, rx + 1), "bottom spans both sides");
    }

    /// An untitled float — which is every float the editor draws today, the
    /// hover popup being the only one — gets one unbroken line, not a run of
    /// glyphs with gaps between them.
    #[test]
    fn an_untitled_box_has_one_continuous_top_rule() {
        let e = box_edges((0, 0, 200, 6 * LINE_H), LINE_H, CHAR_W, None).expect("big enough");
        assert_eq!(
            e.top,
            vec![(0, LINE_H / 2, 200, 1)],
            "a single full-width run"
        );
        assert_eq!(e.label_at, None);
    }

    /// A title interrupts the rule and nothing else: the two runs plus the gap
    /// must tile the full width exactly, with no overlap and no missing pixels.
    #[test]
    fn a_title_splits_the_rule_without_shortening_it() {
        let e = titled();
        assert_eq!(e.top.len(), 2, "a left stub and a run past the title");
        let (x0, y0, w0, _) = e.top[0];
        let (x1, y1, w1, _) = e.top[1];
        assert_eq!(y0, y1, "both runs sit on the same row");
        assert_eq!(x0, 100, "the left stub starts at the left edge");
        assert_eq!(x1 + w1, 300, "the right run ends at the right edge");
        // The gap is exactly the title plus its padding — the label sits in it.
        let (label_x, label_y) = e.label_at.expect("titled");
        assert!(
            x0 + w0 <= label_x && label_x + 40 <= x1,
            "the title fits the gap"
        );
        assert_eq!(
            label_y, 50,
            "the title is drawn on the header row, above the rule"
        );
    }

    /// A title too wide for the box must not produce a negative-width run that
    /// draws backwards across the border.
    #[test]
    fn an_overlong_title_drops_the_right_hand_run() {
        let e = box_edges((0, 0, 100, 6 * LINE_H), LINE_H, CHAR_W, Some(500)).expect("big enough");
        assert_eq!(e.top.len(), 1, "no run past the title");
        assert!(e.top[0].2 >= 0, "and the stub is never negative");
    }

    /// Too small to hold a border: draw nothing rather than overlapping edges.
    #[test]
    fn a_box_with_no_room_for_a_border_is_skipped() {
        assert!(
            box_edges((0, 0, 3, 100), LINE_H, CHAR_W, None).is_none(),
            "too narrow"
        );
        assert!(
            box_edges((0, 0, 100, LINE_H), LINE_H, CHAR_W, None).is_none(),
            "too short"
        );
    }

    /// Every glyph the editor draws has to be baked into the font atlas, or
    /// raylib substitutes `?` — a failure that shows up only in the GUI, never
    /// in the TUI or in any headless test.
    ///
    /// This caught ▸/▾ (sidebar markers) and ● (breakpoints) sitting just past
    /// the end of the box-drawing range. Add a glyph here whenever the editor
    /// starts drawing one.
    #[test]
    fn every_glyph_the_editor_draws_is_in_the_font_atlas() {
        let atlas: std::collections::HashSet<char> = RaylibRenderer::font_chars().chars().collect();

        let glyphs = [
            ('▸', "sidebar: collapsed directory"),
            ('▾', "sidebar: expanded directory"),
            ('●', "debugger: breakpoint"),
            ('✓', "test runner: pass"),
            ('✗', "test runner: fail"),
            ('⚠', "notifications: warning"),
            ('+', "git signs: added"),
            ('~', "git signs: modified"),
            ('_', "git signs: removed"),
            ('─', "window chrome: horizontal rule"),
            ('│', "float border: vertical"),
            ('╭', "float border: top-left"),
            ('╮', "float border: top-right"),
            ('╰', "float border: bottom-left"),
            ('╯', "float border: bottom-right"),
        ];
        let missing: Vec<_> = glyphs.iter().filter(|(c, _)| !atlas.contains(c)).collect();
        assert!(
            missing.is_empty(),
            "glyphs absent from the font atlas, so the GUI draws `?`: {missing:?}"
        );
    }
}
