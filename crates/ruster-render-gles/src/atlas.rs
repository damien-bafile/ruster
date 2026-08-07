use std::cell::RefCell;
use std::collections::HashMap;

use cosmic_text::{Attrs, Buffer, FontSystem, Metrics, Shaping, SwashCache, SwashContent};

/// A single glyph's pixel rect (in the destination) and UV rect (in the atlas).
///
/// `x`/`y` are the offset from the glyph's layout origin — the pen position on
/// the line, and the top of the line box — to the top-left of its bitmap, so a
/// caller draws at `(pen_x + glyph.x, line_top + glyph.y)`. `w`/`h` are the
/// bitmap's size, which is *not* the advance width: a glyph with nothing to
/// draw (a space, a control char, one the font lacks) has a zero-size bitmap
/// but still advances the pen. Ask [`layout_text`] for advances.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glyph {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
}

impl Glyph {
    /// The glyph that draws nothing.
    const EMPTY: Glyph = Glyph {
        x: 0.0,
        y: 0.0,
        w: 0.0,
        h: 0.0,
        u0: 0.0,
        v0: 0.0,
        u1: 0.0,
        v1: 0.0,
    };

    /// Whether this glyph has a bitmap to draw.
    pub fn is_empty(&self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }
}

/// One laid-out text run: per-glyph pen offsets plus the run's total width.
pub struct TextLayout {
    pub glyphs: Vec<(f32, f32, char)>,
    pub width_px: f32,
}

/// Glyphs are cached — and stored in the atlas — per size *and* colour.
///
/// Tinting at draw time would be the usual trick, but a `TextureRenderElement`
/// carries no colour multiplier, and reaching for a custom GLES shader would tie
/// chrome rendering to one renderer type (the DRM path composites through a
/// `MultiRenderer`). Chrome uses a handful of theme colours at two sizes, so
/// baking the colour into the cell costs a few hundred small cells and keeps the
/// draw path renderer-agnostic.
type GlyphKey = (u32, [u8; 3], char);

/// CPU-side glyph atlas: rasterizes glyphs on demand into a single RGBA image
/// that the renderer uploads as one texture, and hands back the UV rect of each.
///
/// Packing is shelf-based — glyphs are laid left to right in a row whose height
/// is the tallest glyph in it, and a new row starts when the current one fills.
/// Once the image is full, further new glyphs draw as nothing rather than
/// evicting live ones; at 1024² with chrome-sized text that is not reachable in
/// practice.
pub struct Atlas {
    pub texture_size: u32,
    glyphs: HashMap<GlyphKey, Glyph>,
    font_system: FontSystem,
    swash: SwashCache,
    /// RGBA8, premultiplied, `texture_size * texture_size * 4` bytes.
    pixels: Vec<u8>,
    /// Bumped whenever `pixels` changes, so the renderer knows to re-upload.
    generation: u64,
    shelf_x: u32,
    shelf_y: u32,
    shelf_h: u32,
}

/// Bytes per pixel in the atlas image.
const BPP: usize = 4;
/// Padding between packed glyphs, so neighbours don't bleed in under filtering.
const PAD: u32 = 1;

impl Atlas {
    pub fn new() -> Self {
        let texture_size = 1024;
        Atlas {
            texture_size,
            glyphs: HashMap::new(),
            font_system: FontSystem::new(),
            swash: SwashCache::new(),
            pixels: vec![0; (texture_size as usize).pow(2) * BPP],
            generation: 0,
            shelf_x: 0,
            shelf_y: 0,
            shelf_h: 0,
        }
    }

    /// The atlas image: RGBA8, premultiplied alpha, row-major, no padding.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Bumped on every change to [`Atlas::pixels`]. A renderer holding an
    /// uploaded copy re-uploads when this no longer matches what it uploaded.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Whether `c` has been rasterized at this size and colour. Lets callers
    /// assert what a draw pass actually put on screen, since the colour a glyph
    /// was drawn in is baked into its cell and not recoverable from the quad.
    pub fn contains(&self, font_size_px: u32, color: [u8; 3], c: char) -> bool {
        self.glyphs.contains_key(&(font_size_px, color, c))
    }

    /// The glyph for `c` at `font_size_px` in `color`, rasterizing and packing
    /// it into the atlas on first use.
    pub fn glyph(&mut self, font_size_px: u32, color: [u8; 3], c: char) -> Glyph {
        let key = (font_size_px, color, c);
        if let Some(g) = self.glyphs.get(&key) {
            return *g;
        }
        let g = self.rasterize(font_size_px, color, c);
        self.glyphs.insert(key, g);
        g
    }

    /// Shape `c` on its own, rasterize it, and blit it into the atlas.
    fn rasterize(&mut self, font_size_px: u32, color: [u8; 3], c: char) -> Glyph {
        if c.is_control() || c == ' ' {
            return Glyph::EMPTY;
        }
        // Disjoint field borrows: shaping needs the font system, rasterizing
        // needs both it and the swash cache.
        let Atlas {
            font_system,
            swash,
            pixels,
            texture_size,
            shelf_x,
            shelf_y,
            shelf_h,
            generation,
            ..
        } = self;

        let metrics = Metrics::new(font_size_px as f32, font_size_px as f32 + 4.0);
        let mut buf = Buffer::new(font_system, metrics);
        buf.set_text(&String::from(c), &Attrs::new(), Shaping::Advanced, None);
        buf.shape_until_scroll(font_system, false);

        // The first laid-out glyph of the first run is our character. `line_y`
        // is the baseline measured from the top of the line box, which is what
        // callers position against.
        let Some((glyph, line_y)) = buf
            .layout_runs()
            .next()
            .and_then(|run| run.glyphs.first().map(|g| (g.clone(), run.line_y)))
        else {
            return Glyph::EMPTY;
        };

        let physical = glyph.physical((0.0, 0.0), 1.0);
        let Some(image) = swash.get_image(font_system, physical.cache_key).as_ref() else {
            return Glyph::EMPTY;
        };
        let (bw, bh) = (image.placement.width, image.placement.height);
        if bw == 0 || bh == 0 {
            return Glyph::EMPTY;
        }

        // Shelf packing: advance along the current row, drop to a new one when
        // this glyph would overhang the right edge.
        if *shelf_x + bw + PAD > *texture_size {
            *shelf_x = 0;
            *shelf_y += *shelf_h + PAD;
            *shelf_h = 0;
        }
        if *shelf_y + bh + PAD > *texture_size {
            tracing::warn!(?c, "glyph atlas is full; glyph will not be drawn");
            return Glyph::EMPTY;
        }
        let (ox, oy) = (*shelf_x, *shelf_y);
        *shelf_x += bw + PAD;
        *shelf_h = (*shelf_h).max(bh);

        blit(pixels, *texture_size, ox, oy, image, color);
        *generation += 1;

        let size = *texture_size as f32;
        Glyph {
            x: (physical.x + image.placement.left) as f32,
            y: (line_y - image.placement.top as f32).round(),
            w: bw as f32,
            h: bh as f32,
            u0: ox as f32 / size,
            v0: oy as f32 / size,
            u1: (ox + bw) as f32 / size,
            v1: (oy + bh) as f32 / size,
        }
    }
}

/// Write one rasterized glyph into the atlas image at `(ox, oy)`, tinted with
/// `color` and premultiplied so it composites correctly over the chrome behind
/// it. Swash hands back either 8-bit coverage (the normal case) or full colour
/// (emoji), and the two are blitted differently.
fn blit(
    pixels: &mut [u8],
    texture_size: u32,
    ox: u32,
    oy: u32,
    image: &cosmic_text::SwashImage,
    color: [u8; 3],
) {
    let (bw, bh) = (
        image.placement.width as usize,
        image.placement.height as usize,
    );
    let stride = texture_size as usize * BPP;
    for row in 0..bh {
        for col in 0..bw {
            let dst = (oy as usize + row) * stride + (ox as usize + col) * BPP;
            let Some(px) = pixels.get_mut(dst..dst + BPP) else {
                continue;
            };
            match image.content {
                SwashContent::Mask | SwashContent::SubpixelMask => {
                    let Some(&coverage) = image.data.get(row * bw + col) else {
                        continue;
                    };
                    let a = coverage as u32;
                    px[0] = (color[0] as u32 * a / 255) as u8;
                    px[1] = (color[1] as u32 * a / 255) as u8;
                    px[2] = (color[2] as u32 * a / 255) as u8;
                    px[3] = coverage;
                }
                SwashContent::Color => {
                    let src = (row * bw + col) * 4;
                    let Some(rgba) = image.data.get(src..src + 4) else {
                        continue;
                    };
                    let a = rgba[3] as u32;
                    px[0] = (rgba[0] as u32 * a / 255) as u8;
                    px[1] = (rgba[1] as u32 * a / 255) as u8;
                    px[2] = (rgba[2] as u32 * a / 255) as u8;
                    px[3] = rgba[3];
                }
            }
        }
    }
}

impl Default for Atlas {
    fn default() -> Self {
        Self::new()
    }
}

thread_local! {
    static FONT_SYSTEM: RefCell<FontSystem> = RefCell::new(FontSystem::new());
}

/// Lay out a string, measuring its pixel width. Used by chrome drawing and by
/// the atlas for glyph metrics.
///
/// Shaped [`Shaping::Basic`], one glyph per character, because that is the only
/// thing [`Atlas`] can store: its cache is keyed by `char`, and it rasterizes
/// each one by shaping it alone. Ask for `Advanced` and the shaper returns a
/// single `fl` ligature glyph spanning two characters, [`glyph_cluster_char`]
/// keeps only the leading `f`, and the `l` is dropped while the pen still
/// advances the full ligature width — "toggle floating" reaches the screen as
/// "toggle f oating". Storing ligatures would mean keying the atlas by the
/// shaped glyph instead, which is the better design and a bigger change than
/// the bug warrants.
pub fn layout_text(text: &str, font_size_px: u32, wrap_width: Option<f32>) -> TextLayout {
    FONT_SYSTEM.with(|thread| {
        let mut fs = thread.borrow_mut();
        let metrics = Metrics::new(font_size_px as f32, font_size_px as f32 + 4.0);
        let mut buf = Buffer::new(&mut fs, metrics);
        buf.set_text(text, &Attrs::new(), Shaping::Basic, None);
        if let Some(w) = wrap_width {
            buf.set_size(Some(w), None);
        }
        buf.shape_until_scroll(&mut fs, false);
        let mut layout = TextLayout {
            glyphs: Vec::new(),
            width_px: 0.0,
        };
        for run in buf.layout_runs() {
            for glyph in run.glyphs.iter() {
                let ch = glyph_cluster_char(run.text, glyph.start, glyph.end);
                layout.glyphs.push((glyph.x, glyph.y, ch));
            }
            layout.width_px = layout.width_px.max(run.line_w);
        }
        layout
    })
}

/// The leading `char` of a layout-glyph's cluster byte range within a run's text.
fn glyph_cluster_char(text: &str, start: usize, end: usize) -> char {
    text.get(start..end)
        .and_then(|s| s.chars().next())
        .unwrap_or(' ')
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: [u8; 3] = [255, 255, 255];

    #[test]
    fn glyph_returns_zero_size_for_unknown() {
        let mut atlas = Atlas::new();
        let g = atlas.glyph(20, WHITE, '\0');
        assert_eq!((g.w, g.h), (0.0, 0.0));
        assert!(g.is_empty());
    }

    #[test]
    fn space_has_no_bitmap() {
        let mut atlas = Atlas::new();
        assert!(atlas.glyph(20, WHITE, ' ').is_empty());
    }

    #[test]
    fn glyphs_are_cached() {
        let mut atlas = Atlas::new();
        let a = atlas.glyph(20, WHITE, 'a');
        let generation = atlas.generation();
        let b = atlas.glyph(20, WHITE, 'a');
        assert_eq!((a.u0, a.v0, a.u1, a.v1), (b.u0, b.v0, b.u1, b.v1));
        // The second lookup is a cache hit, so it must not touch the image.
        assert_eq!(atlas.generation(), generation);
    }

    #[test]
    fn rasterizing_writes_pixels_and_bumps_generation() {
        let mut atlas = Atlas::new();
        assert_eq!(atlas.generation(), 0);
        assert!(atlas.pixels().iter().all(|&b| b == 0));

        let g = atlas.glyph(20, WHITE, 'M');
        assert!(!g.is_empty(), "M should rasterize to a bitmap");
        assert!(atlas.generation() > 0);
        assert!(
            atlas.pixels().iter().any(|&b| b != 0),
            "rasterizing must write coverage into the atlas image"
        );
    }

    #[test]
    fn glyph_uvs_are_normalized_and_match_the_bitmap() {
        let mut atlas = Atlas::new();
        let g = atlas.glyph(20, WHITE, 'M');
        for uv in [g.u0, g.v0, g.u1, g.v1] {
            assert!((0.0..=1.0).contains(&uv), "uv {uv} out of range");
        }
        let size = atlas.texture_size as f32;
        assert!(((g.u1 - g.u0) * size - g.w).abs() < 0.01);
        assert!(((g.v1 - g.v0) * size - g.h).abs() < 0.01);
    }

    #[test]
    fn same_glyph_in_two_colors_gets_two_cells() {
        let mut atlas = Atlas::new();
        let white = atlas.glyph(20, WHITE, 'M');
        let pink = atlas.glyph(20, [243, 139, 168], 'M');
        assert_ne!((white.u0, white.v0), (pink.u0, pink.v0));
    }

    #[test]
    fn glyphs_are_packed_without_overlapping() {
        let mut atlas = Atlas::new();
        let a = atlas.glyph(16, WHITE, 'a');
        let b = atlas.glyph(16, WHITE, 'b');
        let horizontally_apart = a.u1 <= b.u0 || b.u1 <= a.u0;
        let vertically_apart = a.v1 <= b.v0 || b.v1 <= a.v0;
        assert!(horizontally_apart || vertically_apart);
    }

    #[test]
    fn every_character_gets_a_glyph_even_when_the_font_would_ligate() {
        // "fl", "fi" and "ffi" are the common ligatures. With advanced shaping
        // each collapses to one glyph spanning several chars, and since the
        // atlas is keyed by char the trailing ones are silently dropped: the
        // which-key overlay drew "toggle floating" as "toggle f oating".
        for word in ["floating", "file", "office", "flat"] {
            let layout = layout_text(word, 16, None);
            assert_eq!(
                layout.glyphs.len(),
                word.chars().count(),
                "{word:?} lost a character to a ligature"
            );
            let laid_out: String = layout.glyphs.iter().map(|(_, _, c)| *c).collect();
            assert_eq!(laid_out, word, "and it must be the same characters");
        }
    }

    #[test]
    fn layout_text_measures_width() {
        let layout = layout_text("hello", 20, None);
        assert!(layout.width_px > 0.0);
        assert_eq!(layout.glyphs.len(), 5);
    }
}
