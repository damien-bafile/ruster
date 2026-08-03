//! GL rendering primitives for the ruster compositor: glyph-atlas text,
//! quad/rounded-rect geometry, and the Smithay render elements that draw
//! ruster's chrome (statusline, editor frames, which-key) in the same GL
//! scene as client surfaces.

pub mod atlas;
pub mod geometry;
