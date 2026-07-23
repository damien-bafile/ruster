use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::document::{BufferId, DocKind, Document, SpecialKind};

/// Registry of all open [`Document`]s (vim "buffers").
///
/// Ids are stable for the lifetime of a document. [`order`](Self::ids) tracks
/// creation order and is used by buffer-cycling commands and the buffer list.
pub struct BufferStore {
    docs: HashMap<BufferId, Document>,
    order: Vec<BufferId>,
    next: u32,
}

/// Best-effort canonicalization for path identity. Falls back to the path as
/// given when the file does not yet exist on disk.
fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

impl BufferStore {
    pub fn new() -> Self {
        BufferStore {
            docs: HashMap::new(),
            order: Vec::new(),
            next: 1,
        }
    }

    fn alloc_id(&mut self) -> BufferId {
        let id = BufferId(self.next);
        self.next += 1;
        id
    }

    fn insert(&mut self, doc: Document) -> BufferId {
        let id = self.alloc_id();
        self.docs.insert(id, doc);
        self.order.push(id);
        id
    }

    /// Open `path` with `content` already read from disk. If a document for the
    /// same (canonical) path is already open, returns its existing id and
    /// ignores `content`.
    pub fn open_file(&mut self, path: PathBuf, content: String) -> BufferId {
        let target = canonical(&path);
        for (id, doc) in &self.docs {
            if matches!(doc.kind, DocKind::File) {
                if let Some(existing) = &doc.file_path {
                    if canonical(existing) == target {
                        return *id;
                    }
                }
            }
        }
        self.insert(Document::from_file(path, &content))
    }

    /// Create a new unnamed scratch document.
    pub fn create_scratch(&mut self, name: &str) -> BufferId {
        self.insert(Document::scratch(name))
    }

    /// Create a new ruster-managed special document (ibuffer, dired, picker).
    pub fn create_special(&mut self, kind: SpecialKind, name: &str) -> BufferId {
        self.insert(Document::special(kind, name))
    }

    pub fn get(&self, id: BufferId) -> Option<&Document> {
        self.docs.get(&id)
    }

    pub fn get_mut(&mut self, id: BufferId) -> Option<&mut Document> {
        self.docs.get_mut(&id)
    }

    /// Close a document. Refuses to close the last remaining document while it
    /// is modified (returns `false`, leaving the store untouched).
    pub fn close(&mut self, id: BufferId) -> bool {
        let modified = match self.docs.get(&id) {
            Some(d) => d.modified,
            None => return false,
        };
        if self.docs.len() == 1 && modified {
            return false;
        }
        self.docs.remove(&id);
        self.order.retain(|&x| x != id);
        true
    }

    /// All open ids, in creation order.
    pub fn ids(&self) -> &[BufferId] {
        &self.order
    }

    pub fn len(&self) -> usize {
        self.docs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }
}

impl Default for BufferStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_two_distinct_files_yields_two_ids() {
        let mut s = BufferStore::new();
        let a = s.open_file(PathBuf::from("/tmp/ruster_ws_a.txt"), "a".into());
        let b = s.open_file(PathBuf::from("/tmp/ruster_ws_b.txt"), "b".into());
        assert_ne!(a, b);
        assert_eq!(s.len(), 2);
        assert_eq!(s.ids(), &[a, b]);
    }

    #[test]
    fn reopening_same_path_returns_same_id() {
        let mut s = BufferStore::new();
        let a = s.open_file(PathBuf::from("/tmp/ruster_ws_a.txt"), "a".into());
        let again = s.open_file(PathBuf::from("/tmp/ruster_ws_a.txt"), "ignored".into());
        assert_eq!(a, again);
        assert_eq!(s.len(), 1);
        // Content of the second open is ignored — first wins.
        assert_eq!(s.get(a).unwrap().buffer.to_string(), "a");
    }

    #[test]
    fn scratch_and_special_have_expected_kinds() {
        let mut s = BufferStore::new();
        let sc = s.create_scratch("[No Name]");
        let sp = s.create_special(SpecialKind::Ibuffer, "*ibuffer*");
        assert_eq!(s.get(sc).unwrap().kind, DocKind::Scratch);
        assert_eq!(
            s.get(sp).unwrap().kind,
            DocKind::Special(SpecialKind::Ibuffer)
        );
    }

    #[test]
    fn close_nonexistent_is_false() {
        let mut s = BufferStore::new();
        assert!(!s.close(BufferId(999)));
    }

    #[test]
    fn close_removes_from_order() {
        let mut s = BufferStore::new();
        let a = s.create_scratch("a");
        let b = s.create_scratch("b");
        assert!(s.close(a));
        assert_eq!(s.ids(), &[b]);
        assert!(s.get(a).is_none());
    }

    #[test]
    fn refuses_to_close_last_modified_buffer() {
        let mut s = BufferStore::new();
        let a = s.create_scratch("a");
        s.get_mut(a).unwrap().modified = true;
        assert!(!s.close(a));
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn closes_last_buffer_when_clean() {
        let mut s = BufferStore::new();
        let a = s.create_scratch("a");
        assert!(s.close(a));
        assert!(s.is_empty());
    }

    #[test]
    fn modified_flag_round_trips() {
        let mut s = BufferStore::new();
        let a = s.create_scratch("a");
        assert!(!s.get(a).unwrap().modified);
        s.get_mut(a).unwrap().modified = true;
        assert!(s.get(a).unwrap().modified);
    }
}
