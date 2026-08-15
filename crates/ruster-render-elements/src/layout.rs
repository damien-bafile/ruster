//! Layout scene types: the pure, GPU-free output of laying an [`Elem`] tree
//! through taffy. Task 2 lands the data structures; the `layout()` walk that
//! fills them arrives in Task 3.

use crate::id::ElementKey;
use ruster_render::{FontFamily, StyledLine};

/// How wide/tall a text leaf would be, in physical px. Injected into the layout
/// walk so text measurement is backend-agnostic.
pub trait TextMeasurer {
    fn measure(&mut self, line: &StyledLine, font_size: f32, family: FontFamily) -> (f32, f32);
}

/// A rectangle in physical px, origin top-left.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PxRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// A painted rectangle in painter's order: a container's background or border.
#[derive(Debug, Clone, PartialEq)]
pub struct BoxNode {
    pub rect: PxRect,
    pub radius: f32,
    pub fill: (f32, f32, f32, f32),
    pub border_width: f32,
    pub border_color: (f32, f32, f32, f32),
    pub key: ElementKey,
}

/// A text leaf positioned and styled for painting.
#[derive(Debug, Clone, PartialEq)]
pub struct TextNode {
    pub rect: PxRect,
    pub line: StyledLine,
    pub font_size: f32,
    pub family: FontFamily,
    pub fg: (f32, f32, f32, f32),
    pub bold: bool,
    pub key: ElementKey,
}

/// One frame of chrome, emitted in painter's order: every `BoxNode` before its
/// children, `TextNode`s among them. Pure data — unit-testable with no GPU.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LayoutScene {
    pub boxes: Vec<BoxNode>,
    pub texts: Vec<TextNode>,
}
