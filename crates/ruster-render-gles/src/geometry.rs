/// One vertex: x, y, (reserved), r, g, b, a.
///
/// Sized for a `gl::VertexAttribPointer` of two attributes: a 2f32 position
/// (`[0..2]`) and a 4f32 color (`[4..8]`). The middle two slots are reserved
/// for future UV use; solid quads leave them zero.
pub type Vertex = [f32; 8];

/// A textured quad: where to draw a glyph, and where to sample it from.
///
/// `x`/`y`/`w`/`h` are the destination rect in physical pixels with the origin
/// at the output's top-left; `u0`..`v1` are the source rect in the glyph atlas,
/// normalized to the atlas texture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphQuad {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
}

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

pub fn rect_verts(x: f32, y: f32, w: f32, h: f32, color: (f32, f32, f32, f32)) -> Vec<Vertex> {
    let (r, g, b, a) = color;
    vec![
        [x, y, 0.0, 0.0, r, g, b, a],
        [x + w, y, 0.0, 0.0, r, g, b, a],
        [x + w, y + h, 0.0, 0.0, r, g, b, a],
        [x, y, 0.0, 0.0, r, g, b, a],
        [x + w, y + h, 0.0, 0.0, r, g, b, a],
        [x, y + h, 0.0, 0.0, r, g, b, a],
    ]
}

pub fn rounded_rect_verts(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    radius: f32,
    color: (f32, f32, f32, f32),
) -> Vec<Vertex> {
    let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    let (rr, g, b, a) = color;
    if r <= 0.001 {
        return rect_verts(x, y, w, h, color);
    }
    // Center + 4 corner fan: draw an inner rect + 4 rounded corners as quads.
    let mut verts = rect_verts(x + r, y, w - 2.0 * r, h, color); // middle band
    verts.extend(rect_verts(x, y + r, w, h - 2.0 * r, color)); // vertical band
                                                               // Corners: for each corner draw a 2x2 grid of squares, skipping the outer
                                                               // corner cell (the one beyond radius). 64 verts is fine for a chrome bar.
    let corners = [
        (x, y, 1.0, 1.0),
        (x + w - r, y, -1.0, 1.0),
        (x, y + h - r, 1.0, -1.0),
        (x + w - r, y + h - r, -1.0, -1.0),
    ];
    for (cx, cy, fx, fy) in corners {
        for gy in 0..2 {
            for gx in 0..2 {
                let dx = (gx as f32 + 0.5) * fx;
                let dy = (gy as f32 + 0.5) * fy;
                let inside = dx * dx + dy * dy <= r * r;
                if !inside {
                    continue;
                }
                // Keep each sub-cell fully inside the rounded rect's bounding
                // box; this guarantees every vertex stays within [x, x+w] ×
                // [y, y+h] even for the bottom/right corners.
                let cell_x = (cx + gx as f32 * fx * r).clamp(x, x + w - r);
                let cell_y = (cy + gy as f32 * fy * r).clamp(y, y + h - r);
                verts.extend(rect_verts(cell_x, cell_y, r, r, (rr, g, b, a)));
            }
        }
    }
    verts
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruster_render::Color;

    #[test]
    fn rect_verts_make_two_triangles() {
        let v = rect_verts(0.0, 0.0, 10.0, 5.0, (1.0, 0.0, 0.0, 1.0));
        assert_eq!(v.len(), 6);
        for vert in &v {
            assert_eq!(vert[4], 1.0); // r
            assert_eq!(vert[7], 1.0); // a
        }
        let xs: Vec<f32> = v.iter().map(|t| t[0]).collect();
        assert!(xs.iter().all(|x| (0.0..=10.0).contains(x)));
    }

    #[test]
    fn rounded_rect_corner_radius_is_clamped() {
        let v = rounded_rect_verts(0.0, 0.0, 10.0, 10.0, 100.0, (0.0, 1.0, 0.0, 1.0));
        // clamp r to min(w,h)/2 → r=5; verts must all stay inside [0,10].
        assert!(v
            .iter()
            .all(|t| (0.0..=10.0).contains(&t[0]) && (0.0..=10.0).contains(&t[1])));
    }

    #[test]
    fn color_rgb_converts_to_normalized_rgba() {
        let c: (f32, f32, f32, f32) = Color::Rgb(255, 128, 0).into();
        assert_eq!(c, (1.0, 128.0 / 255.0, 0.0, 1.0));
        let d: (f32, f32, f32, f32) = Color::Default.into();
        assert_eq!(d, (1.0, 1.0, 1.0, 1.0));
    }
}
