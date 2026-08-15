//! A portable, GPUI-style declarative element builder for ruster's chrome.
//!
//! `div()`/`text()` build an [`Elem`] tree, [`Styled`] chains a style onto any
//! element, and [`ElementKey`] gives every node a stable identity path. This
//! crate depends only on `ruster-render` and `taffy`, so it compiles anywhere —
//! no smithay, GLES, or cosmic-text. Task 3 adds the `layout()` walk that turns
//! a tree into a [`LayoutScene`].

pub mod element;
pub mod id;
pub mod layout;
pub mod style;

pub use element::{div, text, Elem, ElemKind, IntoLine, Styled};
pub use id::ElementKey;
pub use layout::{layout, BoxNode, LayoutScene, PxRect, TextMeasurer, TextNode};
pub use style::Style;
