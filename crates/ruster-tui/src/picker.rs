//! Shared floating-list picker: a fuzzy-filtered selectable list reused by the
//! buffer list, file finder, live grep, and which-key. Rendering happens via
//! [`ruster_render::PickerView`]; this module owns the state and matching.

use std::path::PathBuf;

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use ruster_core::document::BufferId;
use ruster_render::{PickerRow, PickerView};

/// What happens when a picker item is accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerAction {
    /// Switch the active window to this buffer.
    OpenBuffer(BufferId),
    /// Open this file path in the active window.
    OpenPath(PathBuf),
    /// Open a file and move the cursor to (line, col), both 1-indexed.
    OpenLocation(PathBuf, usize, usize),
    /// Run a cmdline command string (without the leading ':').
    RunCmd(String),
    /// Run a named `ruster.toml` task.
    RunTask(String),
    /// Apply a theme by name. Previewed as the selection moves, committed on
    /// accept.
    SetTheme(String),
}

/// One selectable entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerItem {
    pub label: String,
    pub action: PickerAction,
}

impl PickerItem {
    pub fn new(label: impl Into<String>, action: PickerAction) -> Self {
        PickerItem { label: label.into(), action }
    }
}

/// A live picker: a title, the full item list, the typed filter, and the
/// current selection (an index into the *filtered* view).
pub struct PickerState {
    pub title: String,
    items: Vec<PickerItem>,
    pub filter: String,
    pub selected: usize,
    /// Center (floating) or bottom (docked) placement.
    pub placement: ruster_render::PickerPlacement,
    matcher: Matcher,
}

impl PickerState {
    pub fn new(title: impl Into<String>, items: Vec<PickerItem>) -> Self {
        PickerState {
            title: title.into(),
            items,
            filter: String::new(),
            selected: 0,
            placement: ruster_render::PickerPlacement::Center,
            matcher: Matcher::new(Config::DEFAULT),
        }
    }

    /// Indices into `items` matching the current filter, best match first.
    /// An empty filter keeps the original order.
    pub fn filtered(&mut self) -> Vec<usize> {
        if self.filter.is_empty() {
            return (0..self.items.len()).collect();
        }
        let pattern = Pattern::parse(&self.filter, CaseMatching::Smart, Normalization::Smart);
        let mut buf: Vec<char> = Vec::new();
        let mut scored: Vec<(usize, u32)> = Vec::new();
        for (i, item) in self.items.iter().enumerate() {
            let hay = Utf32Str::new(&item.label, &mut buf);
            if let Some(score) = pattern.score(hay, &mut self.matcher) {
                scored.push((i, score));
            }
        }
        // Higher score first; stable on ties by original index.
        scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        scored.into_iter().map(|(i, _)| i).collect()
    }

    fn filtered_len(&mut self) -> usize {
        self.filtered().len()
    }

    /// Move the selection by `delta`, wrapping within the filtered list.
    pub fn move_selection(&mut self, delta: i32) {
        let len = self.filtered_len();
        if len == 0 {
            self.selected = 0;
            return;
        }
        let cur = self.selected.min(len - 1) as i32;
        let next = (cur + delta).rem_euclid(len as i32);
        self.selected = next as usize;
    }

    /// Append an item (used to stream results into an open picker).
    pub fn push_item(&mut self, item: PickerItem) {
        self.items.push(item);
    }

    /// Number of items currently loaded.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn push_char(&mut self, c: char) {
        self.filter.push(c);
        self.selected = 0;
    }

    pub fn pop_char(&mut self) {
        self.filter.pop();
        self.selected = 0;
    }

    /// The action of the currently-selected item without consuming it (used to
    /// build the preview pane).
    pub fn selected_action(&mut self) -> Option<PickerAction> {
        let filtered = self.filtered();
        let idx = *filtered.get(self.selected)?;
        Some(self.items[idx].action.clone())
    }

    /// The action of the currently-selected item, if any.
    pub fn accept(&mut self) -> Option<PickerAction> {
        let filtered = self.filtered();
        let idx = *filtered.get(self.selected)?;
        Some(self.items[idx].action.clone())
    }

    /// Build the render view for the current state.
    pub fn view(&mut self) -> PickerView {
        let filtered = self.filtered();
        let selected = self.selected.min(filtered.len().saturating_sub(1));
        let rows = filtered
            .iter()
            .enumerate()
            .map(|(row, &item_idx)| PickerRow {
                label: self.items[item_idx].label.clone(),
                selected: row == selected,
            })
            .collect();
        PickerView {
            title: self.title.clone(),
            query: self.filter.clone(),
            rows,
            preview: Vec::new(), // filled in by the app
            placement: self.placement,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<PickerItem> {
        vec![
            PickerItem::new("main.rs", PickerAction::RunCmd("1".into())),
            PickerItem::new("lib.rs", PickerAction::RunCmd("2".into())),
            PickerItem::new("cargo.toml", PickerAction::RunCmd("3".into())),
        ]
    }

    #[test]
    fn empty_filter_shows_all_in_order() {
        let mut p = PickerState::new("files", items());
        assert_eq!(p.filtered(), vec![0, 1, 2]);
    }

    #[test]
    fn filtering_narrows_and_ranks() {
        let mut p = PickerState::new("files", items());
        for c in "rs".chars() {
            p.push_char(c);
        }
        let f = p.filtered();
        // Only the two .rs entries match; cargo.toml is excluded.
        assert_eq!(f.len(), 2);
        assert!(f.contains(&0));
        assert!(f.contains(&1));
        assert!(!f.contains(&2));
    }

    #[test]
    fn move_selection_wraps() {
        let mut p = PickerState::new("files", items());
        assert_eq!(p.selected, 0);
        p.move_selection(-1); // wrap to last
        assert_eq!(p.selected, 2);
        p.move_selection(1); // wrap to first
        assert_eq!(p.selected, 0);
    }

    #[test]
    fn accept_returns_selected_action() {
        let mut p = PickerState::new("files", items());
        p.move_selection(1);
        assert_eq!(p.accept(), Some(PickerAction::RunCmd("2".into())));
    }

    #[test]
    fn accept_on_empty_filtered_is_none() {
        let mut p = PickerState::new("files", items());
        for c in "zzz".chars() {
            p.push_char(c);
        }
        assert_eq!(p.accept(), None);
    }

    #[test]
    fn push_item_streams_into_picker() {
        let mut p = PickerState::new("files", Vec::new());
        assert!(p.is_empty());
        p.push_item(PickerItem::new("a.rs", PickerAction::RunCmd("1".into())));
        p.push_item(PickerItem::new("b.rs", PickerAction::RunCmd("2".into())));
        assert_eq!(p.len(), 2);
        assert_eq!(p.filtered(), vec![0, 1]);
    }

    #[test]
    fn view_marks_selected_row() {
        let mut p = PickerState::new("files", items());
        p.move_selection(1);
        let v = p.view();
        assert_eq!(v.rows.len(), 3);
        assert!(v.rows[1].selected);
        assert!(!v.rows[0].selected);
    }
}
