//! GL rendering primitives for the ruster compositor: glyph-atlas text,
//! quad/rounded-rect geometry, and the Smithay render elements that draw
//! ruster's chrome (statusline, editor frames, which-key) in the same GL
//! scene as client surfaces.
//!
//! Smithay is Linux-only, so on other platforms this crate compiles to an empty
//! library rather than failing the workspace build.
#![cfg(target_os = "linux")]

pub mod atlas;
pub mod cursor;
pub mod geometry;
pub mod tessellate;
