//! The compositor's UI chrome: statusline, editor frame, which-key overlay.
//!
//! Chrome is drawn as two kinds of quad, collected into a [`ChromeBatch`]: flat
//! vertex geometry for panels and bars (`Vertex` = x, y, reserved, reserved, r,
//! g, b, a), and textured glyph quads carrying a UV rect into the glyph atlas.
//! The `draw_*` methods are pure and testable — they never touch GL — and are
//! the geometry source of truth. `render_frame` turns the batch into smithay
//! render elements and composites it above the client surface.

use std::any::Any;

use ruster_render::Theme;
use ruster_render_gles::atlas::{layout_text, Atlas};
use ruster_render_gles::cursor::CursorBitmap;
use ruster_render_gles::geometry::{rect_verts, rounded_rect_verts, GlyphQuad, Vertex};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};
use smithay::backend::renderer::element::texture::TextureRenderElement;
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::{Color32F, ImportMem, Renderer};
use smithay::utils::{Physical, Point, Transform};

/// One frame's worth of chrome geometry: solid quads and textured glyph quads,
/// both in physical pixels with the origin at the output's top-left.
///
/// The two lists are appended in painter's order — a panel's background, then
/// the glyphs that sit on it — and `render_frame` reverses them into smithay's
/// front-to-back element order.
#[derive(Debug, Default)]
pub struct ChromeBatch {
    pub verts: Vec<Vertex>,
    pub glyphs: Vec<GlyphQuad>,
}

/// How much of a [`ChromeBatch`] had been drawn at some point, so a later
/// [`ChromeBatch::translate_since`] can move everything drawn after it.
#[derive(Debug, Clone, Copy)]
pub struct BatchMark {
    verts: usize,
    glyphs: usize,
}

impl ChromeBatch {
    /// Record the current end of the batch.
    pub fn mark(&self) -> BatchMark {
        BatchMark {
            verts: self.verts.len(),
            glyphs: self.glyphs.len(),
        }
    }

    /// Shift everything appended since `mark` by `(dx, dy)`. `render_frame` uses
    /// this to centre the welcome editor frame after laying it out at the origin.
    pub fn translate_since(&mut self, mark: BatchMark, dx: f32, dy: f32) {
        for v in &mut self.verts[mark.verts..] {
            v[0] += dx;
            v[1] += dy;
        }
        for g in &mut self.glyphs[mark.glyphs..] {
            g.x += dx;
            g.y += dy;
        }
    }
}

/// The compositor's UI chrome: statusline, editor frame, which-key overlay.
pub struct Chrome {
    pub atlas: Atlas,
    pub theme: Theme,
    line_h: i32,
    /// The atlas as uploaded to the GPU, with the atlas generation it was built
    /// from. Boxed as `Any` because the texture type belongs to the renderer,
    /// and the two backends composite through different ones (`GlesRenderer`
    /// nested, `MultiRenderer` on DRM) while `Chrome` itself is shared.
    texture: Option<(u64, Box<dyn Any>)>,
    /// The built-in pointer image, and its uploaded texture once a renderer has
    /// been seen. Boxed as `Any` for the same reason the atlas texture is.
    cursor: CursorBitmap,
    cursor_texture: Option<Box<dyn Any>>,
    cursor_id: Id,
    /// One render-element id per glyph slot, reused across frames.
    ///
    /// The damage tracker keys element state by id, so glyphs sharing one id
    /// would collapse into a single tracked element and damage the wrong
    /// regions. Ids are handed out by position in the batch and kept stable, so
    /// a glyph that does not move reports no damage.
    glyph_ids: Vec<Id>,
}

impl Chrome {
    pub fn new(theme: Theme) -> Self {
        Chrome {
            atlas: Atlas::new(),
            theme,
            line_h: 24,
            texture: None,
            cursor: CursorBitmap::arrow(),
            cursor_texture: None,
            cursor_id: Id::new(),
            glyph_ids: Vec::new(),
        }
    }

    /// The uploaded glyph atlas for `renderer`, re-uploading when the atlas has
    /// rasterized new glyphs since the last upload. Returns `None` if the
    /// texture cannot be imported, in which case chrome text is skipped for the
    /// frame rather than failing it.
    pub fn atlas_texture<R>(&mut self, renderer: &mut R) -> Option<R::TextureId>
    where
        R: Renderer + ImportMem,
        R::TextureId: Clone + 'static,
    {
        let generation = self.atlas.generation();
        if let Some((uploaded, cached)) = self.texture.as_ref() {
            if *uploaded == generation {
                if let Some(texture) = cached.downcast_ref::<R::TextureId>() {
                    return Some(texture.clone());
                }
            }
        }
        let size = self.atlas.texture_size as i32;
        let texture = renderer
            .import_memory(
                self.atlas.pixels(),
                Fourcc::Abgr8888,
                (size, size).into(),
                false,
            )
            .inspect_err(|_| {
                tracing::warn!("failed to upload the glyph atlas; chrome text skipped")
            })
            .ok()?;
        self.texture = Some((generation, Box::new(texture.clone())));
        Some(texture)
    }

    /// A stable render-element id for the glyph at `index` in the batch.
    pub fn glyph_id(&mut self, index: usize) -> Id {
        while self.glyph_ids.len() <= index {
            self.glyph_ids.push(Id::new());
        }
        self.glyph_ids[index].clone()
    }

    /// The built-in arrow cursor, positioned so its hotspot sits at `location`.
    ///
    /// Uploaded once and cached like the glyph atlas; the id is stable across
    /// frames so the damage tracker sees a moving element rather than a new one
    /// each frame.
    pub fn cursor_element<R>(
        &mut self,
        renderer: &mut R,
        location: Point<f64, Physical>,
    ) -> Option<TextureRenderElement<R::TextureId>>
    where
        R: Renderer + ImportMem,
        R::TextureId: Clone + 'static,
    {
        let texture = match self.cursor_texture.as_ref() {
            Some(cached) => cached.downcast_ref::<R::TextureId>()?.clone(),
            None => {
                let texture = renderer
                    .import_memory(
                        self.cursor.pixels(),
                        Fourcc::Abgr8888,
                        (self.cursor.width, self.cursor.height).into(),
                        false,
                    )
                    .inspect_err(|_| tracing::warn!("failed to upload the cursor image"))
                    .ok()?;
                self.cursor_texture = Some(Box::new(texture.clone()));
                texture
            }
        };
        let (hx, hy) = self.cursor.hotspot;
        let origin = Point::<f64, Physical>::from((location.x - hx as f64, location.y - hy as f64));
        Some(TextureRenderElement::from_static_texture(
            self.cursor_id.clone(),
            renderer.context_id(),
            origin,
            texture,
            1,
            Transform::Normal,
            None,
            None,
            None,
            None,
            Kind::Cursor,
        ))
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
        batch: &mut ChromeBatch,
    ) -> i32 {
        let bar_h = crate::render::chrome_height(h);
        let y = (h - bar_h) as f32;
        let bar_w = w as f32;
        let bg: (f32, f32, f32, f32) = self.theme.statusline_bg.into();
        let fg: (f32, f32, f32, f32) = self.theme.statusline_fg.into();
        let accent: (f32, f32, f32, f32) = self.theme.accent.into();
        let accent_fg: (f32, f32, f32, f32) = self.theme.accent_fg.into();

        batch
            .verts
            .extend(rect_verts(0.0, y, bar_w, bar_h as f32, bg));

        // Mode segment: accent background, "N" (Normal) in the accent foreground.
        let mode_w = 64.0;
        let pad = (bar_h as f32 - 16.0) / 2.0;
        batch
            .verts
            .extend(rect_verts(0.0, y, mode_w, bar_h as f32, accent));
        self.text("N", 16, (mode_w - 16.0) / 2.0, y + pad, accent_fg, batch);

        // Workspace label + focused title in the statusline foreground.
        let ws = format!("WS {workspace}");
        let title = if focused_title.is_empty() {
            "(no client)"
        } else {
            focused_title
        };
        let cursor = mode_w + 12.0;
        let ws_w = self.text(&ws, 16, cursor, y + pad, fg, batch);
        self.text(title, 16, cursor + ws_w + 20.0, y + pad, fg, batch);

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
        batch: &mut ChromeBatch,
    ) {
        let bar_h = 28;
        let bg: (f32, f32, f32, f32) = self.theme.bg.into();
        let fg: (f32, f32, f32, f32) = self.theme.fg.into();
        let accent: (f32, f32, f32, f32) = self.theme.accent.into();
        let accent_fg: (f32, f32, f32, f32) = self.theme.accent_fg.into();

        batch
            .verts
            .extend(rounded_rect_verts(0.0, 0.0, w as f32, h as f32, 4.0, bg));
        batch
            .verts
            .extend(rect_verts(0.0, 0.0, w as f32, bar_h as f32, accent));
        self.text(
            title,
            16,
            6.0,
            (bar_h as f32 - 16.0) / 2.0,
            accent_fg,
            batch,
        );

        let rows = (h - bar_h - 8) / self.line_h;
        let shown = rows.min(buffer.len() as i32);
        for line in 0..shown {
            let text = &buffer[line as usize];
            let gy = (bar_h + 6 + line * self.line_h) as f32;
            self.text(text, 14, 6.0, gy, fg, batch);
        }
    }

    /// Bottom which-key overlay panel.
    pub fn draw_whichkey(&mut self, binds: &[(String, String)], batch: &mut ChromeBatch) {
        let w = 420.0;
        let row_h = 20.0;
        let h = 12.0 + binds.len() as f32 * row_h;
        let x = 12.0;
        let y = 12.0;
        let bg: (f32, f32, f32, f32) = self.theme.whichkey_bg.into();
        let fg: (f32, f32, f32, f32) = self.theme.whichkey_fg.into();

        batch.verts.extend(rounded_rect_verts(x, y, w, h, 6.0, bg));
        for (i, (key, desc)) in binds.iter().enumerate() {
            let ty = y + 10.0 + i as f32 * row_h;
            self.text(&format!("{key}  {desc}"), 14, x + 10.0, ty, fg, batch);
        }
    }

    /// Lay `text` out and append one textured quad per glyph, positioned at
    /// `(x, y)` — the pen position and the top of the line box — plus the
    /// layout's per-glyph advance and the glyph's own bearing. Returns the run's
    /// advance width so callers can chain text to the right.
    ///
    /// Glyphs with no bitmap (spaces, control chars) advance the pen and draw
    /// nothing.
    fn text(
        &mut self,
        text: &str,
        font_size: u32,
        x: f32,
        y: f32,
        color: (f32, f32, f32, f32),
        batch: &mut ChromeBatch,
    ) -> f32 {
        let rgb = rgb8(color);
        let layout = layout_text(text, font_size, None);
        for (gx, _, c) in layout.glyphs {
            let g = self.atlas.glyph(font_size, rgb, c);
            if g.is_empty() {
                continue;
            }
            batch.glyphs.push(GlyphQuad {
                x: x + gx + g.x,
                y: y + g.y,
                w: g.w,
                h: g.h,
                u0: g.u0,
                v0: g.v0,
                u1: g.u1,
                v1: g.v1,
            });
        }
        layout.width_px
    }
}

/// A chrome colour as the 8-bit RGB the atlas bakes into a glyph cell.
fn rgb8(color: (f32, f32, f32, f32)) -> [u8; 3] {
    let to_u8 = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    [to_u8(color.0), to_u8(color.1), to_u8(color.2)]
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
        let mut batch = ChromeBatch::default();
        let h = chrome.draw_statusline(800, 600, 1, "foot", &mut batch);
        assert!(h > 0);
        assert!(batch.verts.len() >= 6);
        // every vertex within the bar band
        for v in &batch.verts {
            assert!(v[1] >= 600.0 - h as f32 - 1.0 && v[1] <= 600.0 + 1.0);
        }
    }

    #[test]
    fn statusline_draws_its_text_as_glyphs() {
        // The mode letter, the workspace label and the focused title all reach
        // the batch as glyph quads inside the bar — not as solid blocks, and not
        // dropped on the floor.
        let mut chrome = Chrome::new(theme());
        let mut batch = ChromeBatch::default();
        let h = chrome.draw_statusline(800, 600, 1, "foot", &mut batch);
        assert!(
            batch.glyphs.len() >= "N".len() + "WS 1".len(),
            "expected a glyph per drawn character, got {}",
            batch.glyphs.len()
        );
        for g in &batch.glyphs {
            assert!(g.w > 0.0 && g.h > 0.0, "glyph quads carry a bitmap");
            assert!(
                g.y >= 600.0 - h as f32 - 1.0 && g.y + g.h <= 600.0 + 1.0,
                "glyph at y={} escapes the {h}px bar",
                g.y
            );
        }
    }

    #[test]
    fn editor_frame_renders_title_and_lines() {
        let mut chrome = Chrome::new(theme());
        let buf: Vec<String> = (0..20).map(|i| format!("line {i}")).collect();
        let mut batch = ChromeBatch::default();
        chrome.draw_editor_frame(400, 300, &buf, "welcome", &mut batch);
        assert!(batch.verts.len() >= 6 * 2); // frame + title bar
        assert!(!batch.glyphs.is_empty(), "title and rows draw glyphs");
    }

    #[test]
    fn whichkey_panel_renders_binds() {
        let mut chrome = Chrome::new(theme());
        let binds = vec![
            ("M-q".into(), "quit".into()),
            ("M-t".into(), "cycle workspace".into()),
        ];
        let mut batch = ChromeBatch::default();
        chrome.draw_whichkey(&binds, &mut batch);
        assert!(!batch.verts.is_empty());
        assert!(!batch.glyphs.is_empty());
    }

    #[test]
    fn translate_since_moves_both_panels_and_glyphs() {
        let mut chrome = Chrome::new(theme());
        let mut batch = ChromeBatch::default();
        chrome.draw_editor_frame(400, 300, &["hi".to_string()], "welcome", &mut batch);
        let (vert, glyph) = (batch.verts[0], batch.glyphs[0]);

        let mark = batch.mark();
        chrome.draw_editor_frame(400, 300, &["hi".to_string()], "welcome", &mut batch);
        batch.translate_since(mark, 10.0, 20.0);

        // Everything before the mark stays put; everything after it shifts.
        assert_eq!(batch.verts[0], vert);
        assert_eq!(batch.glyphs[0], glyph);
        let moved_vert = batch.verts[vert_count(&batch)];
        assert_eq!(
            (moved_vert[0], moved_vert[1]),
            (vert[0] + 10.0, vert[1] + 20.0)
        );
    }

    fn vert_count(batch: &ChromeBatch) -> usize {
        batch.verts.len() / 2
    }

    #[test]
    fn statusline_colors_are_legible() {
        // Text on the statusline is drawn in the statusline foreground (and the
        // mode glyph in the accent foreground), never in the accent color that
        // fills the mode segment — the brief flagged amber-on-amber as a bug.
        // Glyph colour is baked into the atlas cell, so ask the atlas which
        // cells the draw actually produced.
        let mut chrome = Chrome::new(theme());
        let mut batch = ChromeBatch::default();
        let _ = chrome.draw_statusline(800, 600, 1, "foot", &mut batch);
        let quad = |i: usize| &batch.verts[i * 6..i * 6 + 6];
        assert!(
            quad(0).iter().all(|v| v[4] == 69.0 / 255.0),
            "bar = statusline_bg"
        );
        assert!(
            quad(1).iter().all(|v| v[4] == 243.0 / 255.0),
            "mode segment = accent"
        );

        let accent = [243, 139, 168];
        let accent_fg = [30, 30, 30];
        let statusline_fg = [205, 214, 244];
        assert!(
            chrome.atlas.contains(16, accent_fg, 'N'),
            "the mode letter is drawn in the accent foreground"
        );
        assert!(
            chrome.atlas.contains(16, statusline_fg, 'W'),
            "the workspace label is drawn in the statusline foreground"
        );
        for c in "NWS1foot".chars() {
            assert!(
                !chrome.atlas.contains(16, accent, c),
                "no statusline text is drawn in the accent fill color ({c})"
            );
        }
    }
}
