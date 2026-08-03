use std::cell::RefCell;
use std::collections::HashMap;

use cosmic_text::{Attrs, Buffer, FontSystem, Metrics, Shaping};

/// A single glyph's pixel rect (in the destination) and UV rect (in the atlas).
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

/// One laid-out text run: glyph ids plus char offsets. Phase 0 uses this only
/// to measure width; Task 7 rasterizes from the same layout.
pub struct TextLayout {
    pub glyphs: Vec<(f32, f32, char)>,
    pub width_px: f32,
}

/// CPU-side glyph atlas. Phase 0 keeps a fixed 512px texture with 64px cells
/// (8x8 grid); Task 7 uploads the cells to a GL texture and draws glyph quads.
pub struct Atlas {
    pub texture_size: u32,
    glyphs: HashMap<(u32, char), Glyph>,
    font_system: FontSystem,
}

const CELL: u32 = 64;
const GRID: u32 = 8;

impl Atlas {
    pub fn new() -> Self {
        Atlas {
            texture_size: 512,
            glyphs: HashMap::new(),
            font_system: FontSystem::new(),
        }
    }

    pub fn glyph(&mut self, font_size_px: u32, c: char) -> Glyph {
        if let Some(g) = self.glyphs.get(&(font_size_px, c)) {
            return *g;
        }
        let zero = Glyph {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            u0: 0.0,
            v0: 0.0,
            u1: 0.0,
            v1: 0.0,
        };
        if c == '\0' || c.is_control() {
            self.glyphs.insert((font_size_px, c), zero);
            return zero;
        }
        let w = self.measure(font_size_px, c);
        if w <= 0.0 {
            self.glyphs.insert((font_size_px, c), zero);
            return zero;
        }
        // Round-robin cell on the 8x8 grid, keyed by the cache length.
        let cell = (self.glyphs.len() % (GRID * GRID) as usize) as u32;
        let col = cell % GRID;
        let row = cell / GRID;
        let x = col * CELL;
        let y = row * CELL;
        let u0 = x as f32 / self.texture_size as f32;
        let v0 = y as f32 / self.texture_size as f32;
        let delta = CELL as f32 / self.texture_size as f32;
        let g = Glyph {
            x: 0.0,
            y: 0.0,
            w,
            h: font_size_px as f32,
            u0,
            v0,
            u1: u0 + delta,
            v1: v0 + delta,
        };
        self.glyphs.insert((font_size_px, c), g);
        g
    }

    fn measure(&mut self, font_size_px: u32, c: char) -> f32 {
        let metrics = Metrics::new(font_size_px as f32, font_size_px as f32 + 4.0);
        let mut buf = Buffer::new(&mut self.font_system, metrics);
        let s = String::from(c);
        buf.set_text(&s, &Attrs::new(), Shaping::Advanced, None);
        buf.shape_until_scroll(&mut self.font_system, false);
        let mut w: f32 = 0.0;
        for run in buf.layout_runs() {
            w = w.max(run.line_w);
        }
        w
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
pub fn layout_text(text: &str, font_size_px: u32, wrap_width: Option<f32>) -> TextLayout {
    FONT_SYSTEM.with(|thread| {
        let mut fs = thread.borrow_mut();
        let metrics = Metrics::new(font_size_px as f32, font_size_px as f32 + 4.0);
        let mut buf = Buffer::new(&mut fs, metrics);
        buf.set_text(text, &Attrs::new(), Shaping::Advanced, None);
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

    #[test]
    fn glyph_returns_zero_size_for_unknown() {
        let mut atlas = Atlas::new();
        let g = atlas.glyph(20, '\0');
        assert_eq!((g.w, g.h), (0.0, 0.0));
    }

    #[test]
    fn glyphs_are_cached() {
        let mut atlas = Atlas::new();
        let a = atlas.glyph(20, 'a');
        let b = atlas.glyph(20, 'a');
        assert_eq!((a.u0, a.v0, a.u1, a.v1), (b.u0, b.v0, b.u1, b.v1));
    }

    #[test]
    fn layout_text_measures_width() {
        let layout = layout_text("hello", 20, None);
        assert!(layout.width_px > 0.0);
        assert_eq!(layout.glyphs.len(), 5);
    }
}
