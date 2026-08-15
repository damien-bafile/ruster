/// A stable identity path for one element in a scene tree.
///
/// The root element has the empty path; a child's key is its parent's key plus
/// one segment (the element's `.id()` when set, otherwise its sibling index).
/// Because a key encodes *position in the tree* rather than the element's
/// identity, reordering siblings remaps which key points at which element — the
/// same GPUI footgun, preserved deliberately so ids stay cheap to derive.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct ElementKey(pub Vec<String>);

impl ElementKey {
    /// The key of a child under this key, appending `seg` to the path.
    pub fn child(&self, seg: &str) -> ElementKey {
        let mut v = self.0.clone();
        v.push(seg.to_string());
        ElementKey(v)
    }

    /// Push a segment onto this key in place.
    pub fn push(&mut self, seg: &str) {
        self.0.push(seg.to_string());
    }

    /// The most recently appended segment, if any.
    pub fn last(&self) -> Option<&str> {
        self.0.last().map(|s| s.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_key_is_empty() {
        assert!(ElementKey::default().0.is_empty());
    }

    #[test]
    fn child_appends_a_segment_and_leaves_the_parent_untouched() {
        let root = ElementKey::default();
        let a = root.child("a");
        assert_eq!(a, ElementKey(vec!["a".to_string()]));
        assert!(root.0.is_empty(), "child() must not mutate the parent");
        assert_eq!(
            root.child("a").child("b"),
            ElementKey(vec!["a".to_string(), "b".to_string()])
        );
    }

    #[test]
    fn last_is_the_most_recent_segment() {
        assert_eq!(ElementKey::default().last(), None);
        assert_eq!(
            ElementKey::default().child("a").child("b").last(),
            Some("b")
        );
    }
}
