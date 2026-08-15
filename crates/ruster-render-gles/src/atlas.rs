use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent};

pub use ruster_render::FontFamily;

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
type GlyphKey = (u32, [u8; 3], char, FontFamily);

/// The cosmic-text `Attrs` a [`FontFamily`] shapes with. [`FontFamily`] itself
/// lives in `ruster-render` — it is portable — and this is the Linux-only half
/// that maps it onto a cosmic-text family.
pub fn family_attrs(family: FontFamily) -> Attrs<'static> {
    match family {
        FontFamily::Ui => Attrs::new(),
        FontFamily::Mono => Attrs::new().family(Family::Monospace),
    }
}

/// CPU-side glyph atlas: rasterizes glyphs on demand into a single RGBA image
/// that the renderer uploads as one texture, and hands back the UV rect of each.
///
/// Packing is shelf-based — glyphs are laid left to right in a row whose height
/// is the tallest glyph in it, and a new row starts when the current one fills.
/// Once the image is full, further new glyphs draw as nothing rather than
/// evicting live ones. That is not a failure anyone would notice from the
/// screen — text simply stops appearing — so [`Atlas::dropped_glyphs`] and
/// [`Atlas::fill_fraction`] make it something a caller (or a test) can read.
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
    exhaustion: Exhaustion,
}

/// Bytes per pixel in the atlas image.
const BPP: usize = 4;
/// Padding between packed glyphs, so neighbours don't bleed in under filtering.
const PAD: u32 = 1;

/// The atlas image is this many pixels on a side.
///
/// Syntax highlighting was expected to force this up, because colour is part of
/// the cell key and a theme has a dozen of them. Measured instead of assumed
/// (`atlas_budget.rs` in `ruster-compositor`, against the real highlighter and
/// the default theme): one screenful of Rust is **218** distinct
/// `(size, colour, char)` cells, one whole 1,389-line file **356**, and every
/// source file in this workspace — 233 of them, the ceiling for an atlas that
/// never evicts — **852**, over 176 distinct characters and just **10** distinct
/// colours. That last packs to **12%** of 1024², or under half of it with a
/// generous chrome load on top.
///
/// The count saturates because a theme's palette is small and source code is
/// ASCII: the product that looked like an explosion is bounded by the alphabet.
/// Quadrupling to 16 MiB would buy headroom against nothing measured, and would
/// not save the case that could genuinely overflow either size — thousands of
/// distinct CJK glyphs — which is what [`Atlas::dropped_glyphs`] is for.
const TEXTURE_SIZE: u32 = 1024;

/// At most one atlas-exhaustion report per this interval.
///
/// The alternative is what this replaced: one warning per dropped glyph, which
/// on a pane of highlighted code is per glyph *per frame*. This project has
/// already lost a 2 MB log to a flood of that shape — 11,746 lines in five
/// minutes — on a VT boot, where the log is the only diagnostic channel there
/// is. A report that is throttled still says the atlas is full; a report that
/// buries every other line does not.
const REPORT_INTERVAL: Duration = Duration::from_secs(10);

/// Glyphs dropped for want of room in the atlas, and when that was last said
/// out loud.
#[derive(Debug, Default)]
struct Exhaustion {
    dropped: u64,
    /// `dropped` as of the last report, so a report counts what is new rather
    /// than repeating a running total that only grows.
    reported: u64,
    last: Option<Instant>,
}

impl Exhaustion {
    /// Count one dropped glyph. Returns how many have been dropped since the
    /// last report, but only when the interval has elapsed — otherwise
    /// `None`, and the caller stays quiet.
    ///
    /// Takes `now` rather than reading the clock so the throttle is testable
    /// without sleeping for the interval.
    fn record(&mut self, now: Instant) -> Option<u64> {
        self.dropped += 1;
        if self
            .last
            .is_some_and(|last| now.duration_since(last) < REPORT_INTERVAL)
        {
            return None;
        }
        self.last = Some(now);
        let since = self.dropped - self.reported;
        self.reported = self.dropped;
        Some(since)
    }
}

impl Atlas {
    pub fn new() -> Self {
        Atlas::with_texture_size(TEXTURE_SIZE)
    }

    /// An atlas of a given edge length. Only tests want this — filling a
    /// 2048² image a glyph at a time to prove exhaustion is reported takes
    /// long enough to be worth not doing.
    pub fn with_texture_size(texture_size: u32) -> Self {
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
            exhaustion: Exhaustion::default(),
        }
    }

    /// How many glyphs have been dropped because the atlas had no room.
    ///
    /// Non-zero means text is missing from the screen. Nothing else reports
    /// that: a dropped glyph is [`Glyph::EMPTY`], which is also what a space
    /// and a control character are.
    pub fn dropped_glyphs(&self) -> u64 {
        self.exhaustion.dropped
    }

    /// How much of the atlas image the packed shelves take up, `0.0..=1.0`.
    ///
    /// Measured by shelf rather than by glyph area — the rows a full shelf has
    /// consumed are gone whether or not every pixel in them holds a bitmap, so
    /// this is the number that predicts when the next glyph is refused.
    pub fn fill_fraction(&self) -> f32 {
        if self.texture_size == 0 {
            return 1.0;
        }
        ((self.shelf_y + self.shelf_h) as f32 / self.texture_size as f32).min(1.0)
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
        self.contains_in(font_size_px, color, c, FontFamily::Ui)
    }

    /// As [`contains`](Self::contains), for a specific family.
    pub fn contains_in(
        &self,
        font_size_px: u32,
        color: [u8; 3],
        c: char,
        family: FontFamily,
    ) -> bool {
        self.glyphs.contains_key(&(font_size_px, color, c, family))
    }

    /// The glyph for `c` at `font_size_px` in `color`, rasterizing and packing
    /// it into the atlas on first use.
    pub fn glyph(&mut self, font_size_px: u32, color: [u8; 3], c: char) -> Glyph {
        self.glyph_in(font_size_px, color, c, FontFamily::Ui)
    }

    /// As [`glyph`](Self::glyph), for a specific family.
    pub fn glyph_in(
        &mut self,
        font_size_px: u32,
        color: [u8; 3],
        c: char,
        family: FontFamily,
    ) -> Glyph {
        let key = (font_size_px, color, c, family);
        if let Some(g) = self.glyphs.get(&key) {
            return *g;
        }
        let g = self.rasterize(font_size_px, color, c, family);
        self.glyphs.insert(key, g);
        g
    }

    /// Shape `c` on its own, rasterize it, and blit it into the atlas.
    fn rasterize(
        &mut self,
        font_size_px: u32,
        color: [u8; 3],
        c: char,
        family: FontFamily,
    ) -> Glyph {
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
            exhaustion,
            ..
        } = self;

        let metrics = Metrics::new(font_size_px as f32, font_size_px as f32 + 4.0);
        let mut buf = Buffer::new(font_system, metrics);
        buf.set_text(&String::from(c), &family_attrs(family), Shaping::Advanced, None);
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
            // Counted always, said out loud rarely. Naming the character was
            // the old warning's whole payload and also what made it a flood:
            // there is one per missing glyph per frame, and the count plus the
            // fill is what actually tells you what to do about it.
            if let Some(dropped) = exhaustion.record(Instant::now()) {
                let fill_percent = (*shelf_y + *shelf_h) as f32 / *texture_size as f32 * 100.0;
                tracing::warn!(
                    fill_percent,
                    dropped,
                    texture_size,
                    "glyph atlas is full; text is not being drawn"
                );
            }
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
    layout_text_in(text, font_size_px, wrap_width, FontFamily::Ui)
}

/// The size of one character cell in the monospace face: advance width and line
/// height, in pixels.
///
/// A text grid needs this before it can lay anything out — how many columns fit
/// a tile, which cell a click landed in, where the cursor block goes. Measured
/// rather than assumed, because the advance depends on whichever monospace font
/// fontconfig resolves on this machine.
pub fn cell_metrics(font_size_px: u32) -> (f32, f32) {
    let layout = layout_text_in("M", font_size_px, None, FontFamily::Mono);
    (layout.width_px, font_size_px as f32 + 4.0)
}

/// Lay out a string in a specific family.
pub fn layout_text_in(
    text: &str,
    font_size_px: u32,
    wrap_width: Option<f32>,
    family: FontFamily,
) -> TextLayout {
    FONT_SYSTEM.with(|thread| {
        let mut fs = thread.borrow_mut();
        let metrics = Metrics::new(font_size_px as f32, font_size_px as f32 + 4.0);
        let mut buf = Buffer::new(&mut fs, metrics);
        buf.set_text(text, &family_attrs(family), Shaping::Basic, None);
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
    fn the_mono_family_gives_every_character_the_same_advance() {
        // The point of the family at all. `Attrs::new()` is cosmic-text's
        // sans-serif default, so a text grid built on the chrome path would
        // render code proportionally — and column alignment, cursor placement,
        // the gutter and click-to-position all assume a fixed advance.
        let narrow = layout_text_in("iiii", 16, None, FontFamily::Mono).width_px;
        let wide = layout_text_in("WWWW", 16, None, FontFamily::Mono).width_px;
        assert!(
            (narrow - wide).abs() < 0.5,
            "monospace advances differ: iiii={narrow} WWWW={wide}"
        );
    }

    #[test]
    fn the_ui_family_is_actually_proportional() {
        // The negative half: if the machine resolved a monospace font for both,
        // the test above would pass while proving nothing.
        let narrow = layout_text_in("iiii", 16, None, FontFamily::Ui).width_px;
        let wide = layout_text_in("WWWW", 16, None, FontFamily::Ui).width_px;
        assert!(
            wide > narrow + 1.0,
            "the UI family should be proportional: iiii={narrow} WWWW={wide}"
        );
    }

    #[test]
    fn cell_metrics_report_a_usable_cell() {
        let (advance, line_h) = cell_metrics(16);
        assert!(advance > 1.0, "advance {advance} is not a cell width");
        assert!(line_h > advance, "a cell should be taller than it is wide");
        // Every character must fit the cell the metrics promise, or a grid
        // laid out from them overlaps.
        for c in ['M', 'i', '@', '#'] {
            let w = layout_text_in(&c.to_string(), 16, None, FontFamily::Mono).width_px;
            assert!(
                (w - advance).abs() < 0.5,
                "{c:?} is {w} wide but the cell is {advance}"
            );
        }
    }

    #[test]
    fn the_same_character_in_two_families_gets_two_atlas_cells() {
        // The key includes the family. Sharing one cell would hand whichever
        // asked second the other family's bitmap — the same class of bug as the
        // glyph-id collision that made every glyph draw the atlas itself.
        let mut atlas = Atlas::new();
        let ui = atlas.glyph_in(16, [255, 255, 255], 'M', FontFamily::Ui);
        let mono = atlas.glyph_in(16, [255, 255, 255], 'M', FontFamily::Mono);
        assert!(atlas.contains_in(16, [255, 255, 255], 'M', FontFamily::Ui));
        assert!(atlas.contains_in(16, [255, 255, 255], 'M', FontFamily::Mono));
        assert_ne!(
            (ui.u0, ui.v0),
            (mono.u0, mono.v0),
            "the two families must occupy different cells"
        );
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

    /// A colour nothing else uses, so every request is a fresh cell rather
    /// than a cache hit.
    fn nth_color(i: u32) -> [u8; 3] {
        [(i % 251) as u8, ((i / 251) % 251) as u8, (i / 63001) as u8]
    }

    /// Fill a small atlas until it refuses glyphs. Small because the real
    /// 2048² image would take tens of thousands of rasterizations to fill.
    fn filled_atlas() -> Atlas {
        let mut atlas = Atlas::with_texture_size(64);
        for i in 0..400 {
            atlas.glyph(14, nth_color(i), 'M');
        }
        atlas
    }

    #[test]
    fn a_full_atlas_counts_the_glyphs_it_drops() {
        // Exhaustion is otherwise invisible: a dropped glyph is `Glyph::EMPTY`,
        // which is also what a space is, so nothing downstream can tell that
        // text went missing rather than that there was none.
        let mut atlas = filled_atlas();
        assert!(
            atlas.fill_fraction() > 0.8,
            "a refusing atlas should read as nearly full, not {}",
            atlas.fill_fraction()
        );
        let before = atlas.dropped_glyphs();
        assert!(before > 0, "400 cells into a 64px image must not all fit");

        for i in 400..450 {
            atlas.glyph(14, nth_color(i), 'M');
        }
        assert_eq!(
            atlas.dropped_glyphs(),
            before + 50,
            "every refused glyph counts, not just the first"
        );
    }

    #[test]
    fn a_full_atlas_reports_once_rather_than_once_per_glyph() {
        // The failure this replaces: `tracing::warn!` per missing glyph, per
        // frame, on the only diagnostic channel a VT boot has.
        let atlas = filled_atlas();
        assert!(atlas.dropped_glyphs() > 1, "several glyphs were dropped");
        assert_eq!(
            atlas.exhaustion.reported, 1,
            "but only the first was reported; the rest are inside the interval"
        );
    }

    #[test]
    fn a_report_covers_everything_dropped_since_the_last_one() {
        // Driven with an explicit clock: the throttle is only worth anything if
        // it eventually speaks again, and a test that waited ten real seconds
        // to prove it would not be run.
        let mut exhaustion = Exhaustion::default();
        let t0 = Instant::now();
        assert_eq!(exhaustion.record(t0), Some(1), "the first drop is news");
        for ms in 1..1000 {
            assert_eq!(
                exhaustion.record(t0 + Duration::from_millis(ms)),
                None,
                "a drop {ms}ms into the interval must stay quiet"
            );
        }
        assert_eq!(
            exhaustion.record(t0 + REPORT_INTERVAL),
            Some(1000),
            "the next report counts every drop since the last, not just its own"
        );
        assert_eq!(exhaustion.dropped, 1001, "and none went uncounted");
    }

    #[test]
    fn layout_text_measures_width() {
        let layout = layout_text("hello", 20, None);
        assert!(layout.width_px > 0.0);
        assert_eq!(layout.glyphs.len(), 5);
    }
}
