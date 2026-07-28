//! Tree model for the file-explorer sidebar (Phase 5). A rooted, lazily-expanded
//! directory tree built on top of [`crate::dired`] listings — pure filesystem +
//! state, no UI. The app flattens it to rows and renders them in a side window.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::dired;

/// One visible row of the flattened tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarRow {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    /// Indentation level (0 = a direct child of the root).
    pub depth: usize,
    /// For directories, whether they are currently expanded.
    pub expanded: bool,
}

/// A rooted directory tree with a set of expanded directories.
#[derive(Debug, Clone)]
pub struct SidebarTree {
    pub root: PathBuf,
    expanded: BTreeSet<PathBuf>,
    show_hidden: bool,
}

impl SidebarTree {
    pub fn new(root: PathBuf, show_hidden: bool) -> Self {
        SidebarTree { root, expanded: BTreeSet::new(), show_hidden }
    }

    pub fn is_expanded(&self, path: &Path) -> bool {
        self.expanded.contains(path)
    }

    pub fn expand(&mut self, path: &Path) {
        if path.is_dir() {
            self.expanded.insert(path.to_path_buf());
        }
    }

    pub fn collapse(&mut self, path: &Path) {
        self.expanded.remove(path);
    }

    /// Toggle a directory's expansion; returns the new state.
    pub fn toggle(&mut self, path: &Path) -> bool {
        if self.is_expanded(path) {
            self.collapse(path);
            false
        } else {
            self.expand(path);
            true
        }
    }

    /// Expand every ancestor of `path` (up to the root) so it becomes visible —
    /// used to reveal the active file.
    pub fn reveal(&mut self, path: &Path) {
        let mut cur = path.parent();
        while let Some(dir) = cur {
            if !dir.starts_with(&self.root) && dir != self.root {
                break;
            }
            self.expanded.insert(dir.to_path_buf());
            if dir == self.root {
                break;
            }
            cur = dir.parent();
        }
    }

    pub fn set_show_hidden(&mut self, v: bool) {
        self.show_hidden = v;
    }

    pub fn show_hidden(&self) -> bool {
        self.show_hidden
    }

    /// Discard expanded-state cache so the tree re-reads from disk on the
    /// next [`rows()`](Self::rows) call. Call after file-system mutations.
    pub fn refresh(&mut self) {
        self.expanded.clear();
        let root = self.root.clone();
        self.expand(&root);
    }

    /// The visible rows, depth-first: the root's entries, recursing into expanded
    /// directories. Directories sort before files (via [`dired::list`]); the `..`
    /// entry is omitted (the sidebar is rooted).
    pub fn rows(&self) -> Vec<SidebarRow> {
        let mut out = Vec::new();
        self.push_rows(&self.root, 0, &mut out);
        out
    }

    fn push_rows(&self, dir: &Path, depth: usize, out: &mut Vec<SidebarRow>) {
        for e in dired::list(dir, self.show_hidden) {
            if e.name == ".." {
                continue;
            }
            let path = dir.join(&e.name);
            let expanded = e.is_dir && self.is_expanded(&path);
            out.push(SidebarRow {
                path: path.clone(),
                name: e.name,
                is_dir: e.is_dir,
                depth,
                expanded,
            });
            if expanded {
                self.push_rows(&path, depth + 1, out);
            }
        }
    }

    /// Render the tree as buffer text: indented, with `▾`/`▸` for open/closed
    /// directories and a space for files. One line per [`rows`](Self::rows) entry.
    pub fn render(&self) -> String {
        self.rows()
            .iter()
            .map(|r| {
                let marker = if r.is_dir {
                    if r.expanded { "▾ " } else { "▸ " }
                } else {
                    "  "
                };
                format!("{}{}{}", "  ".repeat(r.depth), marker, r.name)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};

    static FIXTURE_COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// Build `root/{a/{x.txt}, b.txt}` in a temp dir.
    /// Each call gets a unique directory (atomic counter) to avoid
    /// race conditions when tests run in parallel.
    fn fixture() -> PathBuf {
        let id = FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("ruster_sidebar_{}", id));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::write(root.join("a").join("x.txt"), "x").unwrap();
        std::fs::write(root.join("b.txt"), "b").unwrap();
        root
    }

    #[test]
    fn collapsed_root_shows_top_level_only() {
        let root = fixture();
        let tree = SidebarTree::new(root.clone(), false);
        let rows = tree.rows();
        // `a/` (dir, sorts first) then `b.txt`.
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "a");
        assert!(rows[0].is_dir && !rows[0].expanded && rows[0].depth == 0);
        assert_eq!(rows[1].name, "b.txt");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn expanding_a_dir_reveals_its_children_indented() {
        let root = fixture();
        let mut tree = SidebarTree::new(root.clone(), false);
        let a = root.join("a");
        assert!(tree.toggle(&a), "toggle expands");
        let rows = tree.rows();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].name, "x.txt");
        assert_eq!(rows[1].depth, 1);
        // Collapsing hides them again.
        assert!(!tree.toggle(&a));
        assert_eq!(tree.rows().len(), 2);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn reveal_expands_ancestors_of_a_file() {
        let root = fixture();
        let mut tree = SidebarTree::new(root.clone(), false);
        tree.reveal(&root.join("a").join("x.txt"));
        assert!(tree.is_expanded(&root.join("a")));
        assert!(tree.rows().iter().any(|r| r.name == "x.txt"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn render_indents_and_marks_dirs() {
        let root = fixture();
        let mut tree = SidebarTree::new(root.clone(), false);
        tree.expand(&root.join("a"));
        let text = tree.render();
        assert!(text.contains("▾ a"));
        assert!(text.contains("    x.txt"), "child indented under a: {text:?}");
        std::fs::remove_dir_all(&root).ok();
    }
}
