//! Tessellate a portable [`LayoutScene`] into the renderer's [`ChromeBatch`].
//!
//! This is the Linux-only bridge between the pure `ruster-render-elements`
//! layout output and the smithay GL renderer: it consumes cosmic-text through
//! the [`Atlas`], producing the same geometry the compositor's `Chrome::text_in`
//! used to — pixel-for-pixel, so Task 5's parity test can compare the two paths
//! until the old one is deleted.

use crate::atlas::{layout_text_in, Atlas};
use crate::geometry::{rounded_rect_verts, rect_verts, ChromeBatch, GlyphQuad};
use ruster_render::{FontFamily, StyledLine};
use ruster_render_elements::{LayoutScene, TextMeasurer};

/// Measures a line the way the atlas will tessellate it: the cosmic-text layout
/// width at the given family, and `font_size + 4.0` for the line-box height —
/// the same height rule [`cell_metrics`](crate::atlas::cell_metrics) uses.
pub struct GlesTextMeasurer;

impl TextMeasurer for GlesTextMeasurer {
    fn measure(&mut self, line: &StyledLine, font_size: f32, family: FontFamily) -> (f32, f32) {
        let width = layout_text_in(&line.text, font_size as u32, None, family).width_px;
        (width, font_size + 4.0)
    }
}

/// A chrome colour as the 8-bit RGB the atlas bakes into a glyph cell.
///
/// The compositor keeps its own copy (`chrome.rs`) until the Task 6 flip deletes
/// `Chrome::text_in`; the two are in different crates so neither shadows the other.
fn rgb8(color: (f32, f32, f32, f32)) -> [u8; 3] {
    let to_u8 = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    [to_u8(color.0), to_u8(color.1), to_u8(color.2)]
}

/// Tessellate a layout scene into drawable chrome geometry.
///
/// Boxes become solid quads (rounded when the node has a radius, with a 4-rect
/// border on top when `border_width > 0`); texts become one atlas glyph quad per
/// non-empty glyph, exactly as `Chrome::text_in` did, offset by the node's rect.
/// Every emitted glyph also records the element it came from in
/// [`ChromeBatch::glyph_keys`], so a caller can later identify which quad
/// belongs to which element.
pub fn scene_to_chrome_batch(scene: &LayoutScene, atlas: &mut Atlas) -> ChromeBatch {
    let mut batch = ChromeBatch::default();
    for node in &scene.boxes {
        let (x, y, w, h) = (node.rect.x, node.rect.y, node.rect.w, node.rect.h);
        if node.radius > 0.0 {
            batch
                .verts
                .extend(rounded_rect_verts(x, y, w, h, node.radius, node.fill));
        } else {
            batch.verts.extend(rect_verts(x, y, w, h, node.fill));
        }
        if node.border_width > 0.0 {
            let bw = node.border_width;
            let bc = node.border_color;
            batch.verts.extend(rect_verts(x, y, w, bw, bc)); // top
            batch.verts.extend(rect_verts(x, y + h - bw, w, bw, bc)); // bottom
            batch.verts.extend(rect_verts(x, y, bw, h, bc)); // left
            batch.verts.extend(rect_verts(x + w - bw, y, bw, h, bc)); // right
        }
    }
    for node in &scene.texts {
        let font_size = node.font_size as u32;
        let rgb = rgb8(node.fg);
        let layout = layout_text_in(&node.line.text, font_size, None, node.family);
        for (gx, _, c) in layout.glyphs {
            let g = atlas.glyph_in(font_size, rgb, c, node.family);
            if g.is_empty() {
                continue;
            }
            batch.glyphs.push(GlyphQuad {
                x: node.rect.x + gx + g.x,
                y: node.rect.y + g.y,
                w: g.w,
                h: g.h,
                u0: g.u0,
                v0: g.v0,
                u1: g.u1,
                v1: g.v1,
            });
            batch.glyph_keys.push(node.key.clone());
        }
    }
    batch
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruster_render_elements::{BoxNode, ElementKey, LayoutScene, PxRect, TextNode};
    use ruster_render::{FontFamily, StyledLine};

    fn box_node(rect: PxRect, radius: f32, fill: (f32, f32, f32, f32)) -> BoxNode {
        BoxNode {
            rect,
            radius,
            fill,
            border_width: 0.0,
            border_color: (0.0, 0.0, 0.0, 1.0),
            key: ElementKey::default(),
        }
    }

    fn text_node(
        rect: PxRect,
        text: &str,
        fg: (f32, f32, f32, f32),
        key: ElementKey,
    ) -> TextNode {
        TextNode {
            rect,
            line: StyledLine {
                text: text.to_string(),
                highlights: Vec::new(),
            },
            font_size: 16.0,
            family: FontFamily::Mono,
            fg,
            bold: false,
            key,
        }
    }

    fn scene(nodes: Vec<BoxNode>, texts: Vec<TextNode>) -> LayoutScene {
        LayoutScene { boxes: nodes, texts }
    }

    #[test]
    fn plain_box_emits_one_quad() {
        let mut atlas = Atlas::default();
        let s = scene(vec![box_node(PxRect { x: 0.0, y: 0.0, w: 10.0, h: 5.0 }, 0.0, (1.0, 0.0, 0.0, 1.0))], vec![]);
        let batch = scene_to_chrome_batch(&s, &mut atlas);
        assert_eq!(batch.verts.len(), 6);
    }

    #[test]
    fn rounded_box_matches_rounded_rect_verts_and_clamps_radius() {
        let mut atlas = Atlas::default();
        let fill = (0.0, 1.0, 0.0, 1.0);
        let s = scene(vec![box_node(PxRect { x: 0.0, y: 0.0, w: 40.0, h: 20.0 }, 8.0, fill)], vec![]);
        let batch = scene_to_chrome_batch(&s, &mut atlas);
        assert_eq!(
            batch.verts.len(),
            rounded_rect_verts(0.0, 0.0, 40.0, 20.0, 8.0, fill).len(),
        );
        assert!(batch.verts.len() > 6, "a rounded rect is more than one quad");

        // A huge radius clamps to w/2, h/2 — the vert count must match what
        // `rounded_rect_verts` emits for the clamped radius.
        let mut atlas = Atlas::default();
        let s = scene(vec![box_node(PxRect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }, 100.0, fill)], vec![]);
        let batch = scene_to_chrome_batch(&s, &mut atlas);
        assert_eq!(
            batch.verts.len(),
            rounded_rect_verts(0.0, 0.0, 10.0, 10.0, 100.0, fill).len(),
        );
    }

    #[test]
    fn border_pushes_four_rects() {
        let mut atlas = Atlas::default();
        let s = scene(
            vec![BoxNode {
                rect: PxRect { x: 0.0, y: 0.0, w: 40.0, h: 20.0 },
                radius: 0.0,
                fill: (1.0, 1.0, 1.0, 1.0),
                border_width: 2.0,
                border_color: (0.0, 0.0, 1.0, 1.0),
                key: ElementKey::default(),
            }],
            vec![],
        );
        let batch = scene_to_chrome_batch(&s, &mut atlas);
        // One fill quad plus four border quads.
        assert_eq!(batch.verts.len(), 5 * 6);
    }

    #[test]
    fn text_emits_the_same_quads_as_the_old_text_in() {
        let rect = PxRect { x: 40.0, y: 20.0, w: 100.0, h: 20.0 };
        let fg = (1.0, 0.5, 0.25, 1.0);
        let key = ElementKey(vec!["title".into()]);
        let mut atlas = Atlas::default();

        let batch = scene_to_chrome_batch(
            &scene(vec![], vec![text_node(rect, "hi!", fg, key.clone())]),
            &mut atlas,
        );

        // The old `Chrome::text_in` glyph loop, replicated verbatim: this is the
        // parity contract Task 5 tests against until the flip.
        let reference = reference_text_quads("hi!", 16, rect.x, rect.y, fg, FontFamily::Mono, &mut atlas);
        assert_eq!(batch.glyphs, reference);
        assert!(
            reference.iter().all(|q| q.u1 > q.u0 && q.v1 > q.v0),
            "every drawn glyph must sample a non-empty UV rect"
        );
    }

    #[test]
    fn glyph_keys_track_emitted_glyphs_and_carry_the_key() {
        // "a b" includes a space, whose glyph is empty and must be skipped — the
        // invariant still has to hold.
        let key = ElementKey(vec!["label".into()]);
        let mut atlas = Atlas::default();
        let batch = scene_to_chrome_batch(
            &scene(
                vec![],
                vec![text_node(
                    PxRect { x: 0.0, y: 0.0, w: 60.0, h: 20.0 },
                    "a b",
                    (1.0, 1.0, 1.0, 1.0),
                    key.clone(),
                )],
            ),
            &mut atlas,
        );
        assert_eq!(batch.glyphs.len(), batch.glyph_keys.len());
        assert!(
            batch.glyph_keys.iter().all(|k| *k == key),
            "every emitted glyph is tagged with its TextNode's key"
        );
    }

    #[test]
    fn glyph_keys_are_stable_across_runs() {
        let scene = scene(
            vec![],
            vec![text_node(
                PxRect { x: 10.0, y: 5.0, w: 100.0, h: 20.0 },
                "stable",
                (1.0, 1.0, 1.0, 1.0),
                ElementKey(vec!["t".into()]),
            )],
        );
        let mut atlas = Atlas::default();
        let first = scene_to_chrome_batch(&scene, &mut atlas);
        let second = scene_to_chrome_batch(&scene, &mut atlas);
        assert_eq!(first.glyph_keys, second.glyph_keys);
        assert_eq!(first.glyphs, second.glyphs, "the atlas caches, so quads repeat");
    }

    #[test]
    fn measurer_reports_what_the_atlas_will_tessellate() {
        let mut m = GlesTextMeasurer;
        let line = StyledLine { text: "hello".into(), highlights: Vec::new() };
        let (width, height) = m.measure(&line, 16.0, FontFamily::Mono);
        let layout = layout_text_in("hello", 16, None, FontFamily::Mono);
        assert!((width - layout.width_px).abs() < 0.001);
        assert_eq!(height, 20.0, "font_size + 4.0, the cell_metrics height rule");
    }

    /// The old `Chrome::text_in` glyph loop, replicated for the parity contract.
    fn reference_text_quads(
        text: &str,
        font_size: u32,
        x: f32,
        y: f32,
        color: (f32, f32, f32, f32),
        family: FontFamily,
        atlas: &mut Atlas,
    ) -> Vec<GlyphQuad> {
        let mut out = Vec::new();
        let layout = layout_text_in(text, font_size, None, family);
        for (gx, _, c) in layout.glyphs {
            let g = atlas.glyph_in(font_size, ref_rgb8(color), c, family);
            if g.is_empty() {
                continue;
            }
            out.push(GlyphQuad {
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
        out
    }

    /// The old chrome `rgb8` helper, replicated verbatim.
    fn ref_rgb8(color: (f32, f32, f32, f32)) -> [u8; 3] {
        let to_u8 = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
        [to_u8(color.0), to_u8(color.1), to_u8(color.2)]
    }
}
