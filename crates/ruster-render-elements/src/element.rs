use crate::id::ElementKey;
use crate::style::Style;
pub use crate::style::Styled;
use ruster_render::{FontFamily, StyledLine};

/// One node in a declarative chrome scene. `div()` builds a container, `text()`
/// a leaf; `children()` attaches children to a container.
pub struct Elem {
    pub style: Style,
    pub kind: ElemKind,
}

/// What an [`Elem`] is: a container holding more elements, or a text leaf.
pub enum ElemKind {
    Container { children: Vec<Elem> },
    Text { line: StyledLine },
}

/// A blank container element with the default style.
pub fn div() -> Elem {
    Elem {
        style: Style::default(),
        kind: ElemKind::Container {
            children: Vec::new(),
        },
    }
}

/// Anything that can become a [`StyledLine`].
///
/// `ruster-render` defines no `From<String>`/`From<&str>` for `StyledLine`, and
/// the orphan rule forbids adding one here, so `text()` takes this local trait
/// instead of the brief's `impl Into<StyledLine>`. Callers pass a `&str`, an
/// owned `String`, or an already-highlighted `StyledLine`.
pub trait IntoLine {
    fn into_line(self) -> StyledLine;
}

impl IntoLine for String {
    fn into_line(self) -> StyledLine {
        StyledLine {
            text: self,
            highlights: Vec::new(),
        }
    }
}

impl IntoLine for &str {
    fn into_line(self) -> StyledLine {
        StyledLine {
            text: self.to_string(),
            highlights: Vec::new(),
        }
    }
}

impl IntoLine for StyledLine {
    fn into_line(self) -> StyledLine {
        self
    }
}

/// A text leaf. The default font size/family are set explicitly so a bare
/// `text("x")` renders sensibly without any surrounding style to inherit from.
pub fn text(line: impl IntoLine) -> Elem {
    Elem {
        style: Style {
            font_size: 14.0,
            font_family: FontFamily::Ui,
            ..Style::default()
        },
        kind: ElemKind::Text {
            line: line.into_line(),
        },
    }
}

impl Elem {
    /// Replace this element's children. Panics on a text leaf — text cannot
    /// have children.
    pub fn children(&mut self, children: Vec<Elem>) -> &mut Self {
        match &mut self.kind {
            ElemKind::Container { children: slot } => {
                *slot = children;
            }
            ElemKind::Text { .. } => panic!("a text element cannot have children"),
        }
        self
    }

    /// This element's [`ElementKey`] when it is the `index`-th child of
    /// `parent`: the parent's key plus this element's `.id()` segment, or its
    /// sibling index when un-named. The layout walk (Task 3) calls this per
    /// child while descending the tree.
    pub fn key(&self, parent: &ElementKey, index: usize) -> ElementKey {
        parent.child(self.style.id.as_deref().unwrap_or(&index.to_string()))
    }
}

impl Styled for Elem {
    fn style(&mut self) -> &mut Style {
        &mut self.style
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::ElementKey;

    #[test]
    fn div_starts_as_an_empty_container() {
        let e = div();
        assert!(matches!(e.kind, ElemKind::Container { .. }));
        match e.kind {
            ElemKind::Container { children } => assert!(children.is_empty()),
            ElemKind::Text { .. } => unreachable!(),
        }
    }

    #[test]
    fn text_sets_sensible_default_font() {
        let t = text("x");
        assert_eq!(t.style.font_size, 14.0);
        assert_eq!(t.style.font_family, FontFamily::Ui);
    }

    #[test]
    fn text_leaf_rejects_children() {
        let mut t = text("x");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            t.children(vec![div()]);
        }));
        assert!(result.is_err(), "children() on a text leaf must panic");
    }

    #[test]
    fn container_accepts_children() {
        let mut root = div();
        root.children(vec![text("a"), text("b")]);
        match &root.kind {
            ElemKind::Container { children } => assert_eq!(children.len(), 2),
            ElemKind::Text { .. } => unreachable!(),
        }
    }

    #[test]
    fn key_uses_the_element_id_or_its_index() {
        let root = ElementKey::default();
        let mut named = div();
        named.id("pane");
        assert_eq!(named.key(&root, 0), ElementKey(vec!["pane".to_string()]));
        assert_eq!(
            div().key(&root, 3),
            ElementKey(vec!["3".to_string()]),
            "un-named children derive their key from the sibling index"
        );
    }

    #[test]
    fn reordering_siblings_remaps_their_keys() {
        let mut a = text("a");
        a.id("a");
        let mut b = text("b");
        b.id("b");
        let mut root = div();
        root.children(vec![a, b]);

        let keys = |root: &Elem| -> Vec<ElementKey> {
            match &root.kind {
                ElemKind::Container { children } => children
                    .iter()
                    .enumerate()
                    .map(|(i, c)| c.key(&ElementKey::default(), i))
                    .collect(),
                ElemKind::Text { .. } => unreachable!(),
            }
        };

        let before = keys(&root);
        assert_eq!(
            before,
            vec![
                ElementKey(vec!["a".to_string()]),
                ElementKey(vec!["b".to_string()]),
            ]
        );

        // Swap the two siblings under the same parent.
        let (second, first) = match &mut root.kind {
            ElemKind::Container { children } => {
                let second = children.remove(1);
                let first = children.remove(0);
                (second, first)
            }
            ElemKind::Text { .. } => unreachable!(),
        };
        root.children(vec![second, first]);

        let after = keys(&root);
        assert_eq!(
            after,
            vec![
                ElementKey(vec!["b".to_string()]),
                ElementKey(vec!["a".to_string()]),
            ]
        );
        assert_ne!(
            before, after,
            "reordering siblings must remap which key points at which element"
        );
    }
}
