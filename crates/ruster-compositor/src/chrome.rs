//! The compositor's UI chrome: statusline, editor frame, which-key overlay.
//!
//! Phase 0 draws chrome as flat vertex geometry (`Vertex` = x, y, reserved,
//! reserved, r, g, b, a). The `draw_*` methods are pure and testable — they
//! never touch GL — and are the geometry source of truth. `render_frame`
//! converts the collected vertex batch into smithay render elements via
//! [`solid_elements_from_verts`] and composites it above the client surface.
//!
//! Text is currently drawn as solid-color blocks sized to each glyph's pixel
//! box (the atlas provides metrics, not pixels yet); real glyph texture
//! rendering is deferred to the next phase (see the `TODO(next phase)` marker
//! on `Chrome::text`).

use ruster_render::Theme;
use ruster_render_gles::atlas::{layout_text, Atlas};
use ruster_render_gles::geometry::{rect_verts, rounded_rect_verts, Vertex};
use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::Color32F;

/// The compositor's UI chrome: statusline, editor frame, which-key overlay.
/// Phase 0 builds vertex lists; the render loop uploads them to the GLES
/// renderer (Task 7/8 render.rs).
pub struct Chrome {
    pub atlas: Atlas,
    pub theme: Theme,
    line_h: i32,
}

impl Chrome {
    pub fn new(theme: Theme) -> Self {
        Chrome {
            atlas: Atlas::new(),
            theme,
            line_h: 24,
        }
    }

    /// Bottom statusline: returns its height in px.
    ///
    /// Layout (left→right): an accent mode segment with the mode letter in the
    /// accent foreground, then the workspace label and the focused toplevel's
    /// title in the statusline foreground — all legible against the statusline
    /// background. Height is derived from the output via [`crate::render::chrome_height`].
    pub fn draw_statusline(
        &mut self,
        w: i32,
        h: i32,
        workspace: u32,
        focused_title: &str,
        verts: &mut Vec<Vertex>,
    ) -> i32 {
        let bar_h = crate::render::chrome_height(h);
        let y = (h - bar_h) as f32;
        let bar_w = w as f32;
        let bg: (f32, f32, f32, f32) = self.theme.statusline_bg.into();
        let fg: (f32, f32, f32, f32) = self.theme.statusline_fg.into();
        let accent: (f32, f32, f32, f32) = self.theme.accent.into();
        let accent_fg: (f32, f32, f32, f32) = self.theme.accent_fg.into();

        verts.extend(rect_verts(0.0, y, bar_w, bar_h as f32, bg));

        // Mode segment: accent background, "N" (Normal) in the accent foreground.
        let mode_w = 64.0;
        let pad = (bar_h as f32 - 16.0) / 2.0;
        verts.extend(rect_verts(0.0, y, mode_w, bar_h as f32, accent));
        self.text("N", 16, (mode_w - 16.0) / 2.0, y + pad, accent_fg, verts);

        // Workspace label + focused title in the statusline foreground.
        let ws = format!("WS {workspace}");
        let title = if focused_title.is_empty() {
            "(no client)"
        } else {
            focused_title
        };
        let cursor = mode_w + 12.0;
        let ws_w = self.text(&ws, 16, cursor, y + pad, fg, verts);
        self.text(title, 16, cursor + ws_w + 20.0, y + pad, fg, verts);

        bar_h
    }

    /// A synthetic editor frame: mode-line title + buffer rows. Phase 0 shows a
    /// welcome buffer; buffer-driven content lands with the embedded editor.
    pub fn draw_editor_frame(
        &mut self,
        w: i32,
        h: i32,
        buffer: &[String],
        title: &str,
        verts: &mut Vec<Vertex>,
    ) {
        let bar_h = 28;
        let bg: (f32, f32, f32, f32) = self.theme.bg.into();
        let fg: (f32, f32, f32, f32) = self.theme.fg.into();
        let accent: (f32, f32, f32, f32) = self.theme.accent.into();
        let accent_fg: (f32, f32, f32, f32) = self.theme.accent_fg.into();

        verts.extend(rounded_rect_verts(0.0, 0.0, w as f32, h as f32, 4.0, bg));
        verts.extend(rect_verts(0.0, 0.0, w as f32, bar_h as f32, accent));
        self.text(
            title,
            16,
            6.0,
            (bar_h as f32 - 16.0) / 2.0,
            accent_fg,
            verts,
        );

        let rows = (h - bar_h - 8) / self.line_h;
        let shown = rows.min(buffer.len() as i32);
        for line in 0..shown {
            let text = &buffer[line as usize];
            let gy = (bar_h + 6 + line * self.line_h) as f32;
            self.text(text, 14, 6.0, gy, fg, verts);
        }
    }

    /// Bottom which-key overlay panel.
    pub fn draw_whichkey(&mut self, binds: &[(String, String)], verts: &mut Vec<Vertex>) {
        let w = 420.0;
        let row_h = 20.0;
        let h = 12.0 + binds.len() as f32 * row_h;
        let x = 12.0;
        let y = 12.0;
        let bg: (f32, f32, f32, f32) = self.theme.whichkey_bg.into();
        let fg: (f32, f32, f32, f32) = self.theme.whichkey_fg.into();

        verts.extend(rounded_rect_verts(x, y, w, h, 6.0, bg));
        for (i, (key, desc)) in binds.iter().enumerate() {
            let ty = y + 10.0 + i as f32 * row_h;
            self.text(&format!("{key}  {desc}"), 14, x + 10.0, ty, fg, verts);
        }
    }

    /// Lay `text` out and append one solid quad per glyph, sized to the glyph's
    /// pixel box, at `(x, y)` plus the layout's glyph offsets. Returns the run's
    /// advance width so callers can chain text to the right.
    ///
    /// // TODO(next phase): rasterize the atlas glyphs and draw real glyph textures
    /// // via `TextureRenderElement` instead of solid-color blocks.
    fn text(
        &mut self,
        text: &str,
        font_size: u32,
        x: f32,
        y: f32,
        color: (f32, f32, f32, f32),
        verts: &mut Vec<Vertex>,
    ) -> f32 {
        let layout = layout_text(text, font_size, None);
        for (gx, _, c) in layout.glyphs {
            let g = self.atlas.glyph(font_size, c);
            if g.w <= 0.0 || g.h <= 0.0 {
                continue;
            }
            verts.extend(rect_verts(x + gx, y, g.w, g.h, color));
        }
        layout.width_px
    }
}

/// Shift a batch of chrome vertices by `(dx, dy)` in place. `render_frame` uses
/// this to centre the welcome editor frame on the output after it is laid out
/// at the origin.
pub fn translate_verts(verts: &mut [Vertex], dx: f32, dy: f32) {
    for v in verts {
        v[0] += dx;
        v[1] += dy;
    }
}

/// Convert a chrome vertex batch (physical pixels, origin top-left) into
/// smithay solid-color render elements, one per 6-vertex quad.
///
/// Each quad becomes a [`SolidColorRenderElement`] carrying its bounding box and
/// the quad's color (read from the first vertex). `render_frame` collects these
/// and composites them above the client surface in the same
/// [`OutputDamageTracker::render_output`] pass.
pub fn solid_elements_from_verts(verts: &[Vertex]) -> Vec<SolidColorRenderElement> {
    let mut elements = Vec::new();
    for quad in verts.chunks(6) {
        if quad.len() < 6 {
            break;
        }
        let mut x0 = f32::MAX;
        let mut y0 = f32::MAX;
        let mut x1 = f32::MIN;
        let mut y1 = f32::MIN;
        for v in quad {
            x0 = x0.min(v[0]);
            y0 = y0.min(v[1]);
            x1 = x1.max(v[0]);
            y1 = y1.max(v[1]);
        }
        let w = x1 - x0;
        let h = y1 - y0;
        if w <= 0.0 || h <= 0.0 {
            continue;
        }
        let color = Color32F::new(quad[0][4], quad[0][5], quad[0][6], quad[0][7]);
        let buffer = SolidColorBuffer::new((w as i32, h as i32), color);
        elements.push(SolidColorRenderElement::from_buffer(
            &buffer,
            (x0 as i32, y0 as i32),
            1.0,
            1.0,
            Kind::Unspecified,
        ));
    }
    elements
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruster_render::Theme;

    fn theme() -> Theme {
        Theme::default()
    }

    #[test]
    fn statusline_emits_quads() {
        let mut chrome = Chrome::new(theme());
        let mut verts = Vec::new();
        let h = chrome.draw_statusline(800, 600, 1, "foot", &mut verts);
        assert!(h > 0);
        assert!(verts.len() >= 6);
        // every vertex within the bar band
        for v in &verts {
            assert!(v[1] >= 600.0 - h as f32 - 1.0 && v[1] <= 600.0 + 1.0);
        }
    }

    #[test]
    fn editor_frame_renders_title_and_lines() {
        let mut chrome = Chrome::new(theme());
        let buf: Vec<String> = (0..20).map(|i| format!("line {i}")).collect();
        let mut verts = Vec::new();
        chrome.draw_editor_frame(400, 300, &buf, "welcome", &mut verts);
        assert!(verts.len() >= 6 * 2); // title bar + at least one text row
    }

    #[test]
    fn whichkey_panel_renders_binds() {
        let mut chrome = Chrome::new(theme());
        let binds = vec![
            ("M-q".into(), "quit".into()),
            ("M-t".into(), "cycle workspace".into()),
        ];
        let mut verts = Vec::new();
        chrome.draw_whichkey(&binds, &mut verts);
        assert!(!verts.is_empty());
    }

    #[test]
    fn statusline_colors_are_legible() {
        // Text on the statusline is drawn in the statusline foreground (and the
        // mode glyph in the accent foreground), never in the accent color that
        // fills the mode segment — the brief flagged amber-on-amber as a bug.
        let mut chrome = Chrome::new(theme());
        let mut verts = Vec::new();
        let _ = chrome.draw_statusline(800, 600, 1, "foot", &mut verts);
        let quad = |i: usize| &verts[i * 6..i * 6 + 6];
        assert!(
            quad(0).iter().all(|v| v[4] == 69.0 / 255.0),
            "bar = statusline_bg"
        );
        assert!(
            quad(1).iter().all(|v| v[4] == 243.0 / 255.0),
            "mode segment = accent"
        );
        for v in &verts[12..] {
            assert_ne!(v[4], 243.0 / 255.0, "text never uses the accent color");
        }
    }
}
