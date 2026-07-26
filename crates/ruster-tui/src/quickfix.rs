//! A shared quickfix list — a navigable set of `(path, line, col, message,
//! severity)` locations produced by diagnostics, build/test output, etc. It is
//! rendered as a picker and stepped through with `:cnext`/`:cprev` (`]q`/`[q`).

use std::path::PathBuf;

/// One quickfix entry. `line`/`col` are 1-based (editor coordinates).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickfixItem {
    pub path: PathBuf,
    pub line: usize,
    pub col: usize,
    pub message: String,
    /// LSP-style severity: 1 = error, 2 = warning, 3 = info, 4 = hint.
    pub severity: u8,
}

/// A quickfix list plus a selection cursor for `:cnext`/`:cprev`.
#[derive(Debug, Clone, Default)]
pub struct QuickfixList {
    items: Vec<QuickfixItem>,
    sel: usize,
}

impl QuickfixList {
    pub fn new(items: Vec<QuickfixItem>) -> Self {
        QuickfixList { items, sel: 0 }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn items(&self) -> &[QuickfixItem] {
        &self.items
    }

    /// The 0-based index of the current selection.
    pub fn selected(&self) -> usize {
        self.sel
    }

    /// Point the selection at `idx` (clamped), e.g. after choosing from the picker.
    pub fn select(&mut self, idx: usize) {
        if !self.items.is_empty() {
            self.sel = idx.min(self.items.len() - 1);
        }
    }

    pub fn current(&self) -> Option<&QuickfixItem> {
        self.items.get(self.sel)
    }

    /// Advance to the next entry (clamped at the end) and return it.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<&QuickfixItem> {
        if self.items.is_empty() {
            return None;
        }
        if self.sel + 1 < self.items.len() {
            self.sel += 1;
        }
        self.items.get(self.sel)
    }

    /// Step to the previous entry (clamped at the start) and return it.
    pub fn prev(&mut self) -> Option<&QuickfixItem> {
        if self.items.is_empty() {
            return None;
        }
        self.sel = self.sel.saturating_sub(1);
        self.items.get(self.sel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(line: usize) -> QuickfixItem {
        QuickfixItem {
            path: PathBuf::from("src/lib.rs"),
            line,
            col: 1,
            message: format!("problem on {line}"),
            severity: 1,
        }
    }

    #[test]
    fn next_and_prev_clamp_at_the_ends() {
        let mut q = QuickfixList::new(vec![item(1), item(2), item(3)]);
        assert_eq!(q.current().unwrap().line, 1);
        assert_eq!(q.next().unwrap().line, 2);
        assert_eq!(q.next().unwrap().line, 3);
        assert_eq!(q.next().unwrap().line, 3, "clamps at the last entry");
        assert_eq!(q.prev().unwrap().line, 2);
        assert_eq!(q.prev().unwrap().line, 1);
        assert_eq!(q.prev().unwrap().line, 1, "clamps at the first entry");
    }

    #[test]
    fn select_points_at_an_index_and_clamps() {
        let mut q = QuickfixList::new(vec![item(1), item(2)]);
        q.select(5);
        assert_eq!(q.selected(), 1);
        assert_eq!(q.current().unwrap().line, 2);
    }

    #[test]
    fn empty_list_navigates_to_nothing() {
        let mut q = QuickfixList::default();
        assert!(q.is_empty());
        assert!(q.current().is_none());
        assert!(q.next().is_none());
        assert!(q.prev().is_none());
    }
}
