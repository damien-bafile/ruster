//! The editor's picker vocabulary.
//!
//! The state machine itself lives in `ruster-picker`, shared with the
//! compositor's launcher; what stays here is [`PickerAction`], which is editor
//! vocabulary and means nothing to a window manager. The aliases below keep
//! every call site reading exactly as it did — associated functions resolve
//! through a type alias of a generic struct, so `PickerState::new(..)` is
//! unchanged at all of them.

use std::path::PathBuf;

use ruster_core::document::BufferId;

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

/// A live picker over editor actions.
pub type PickerState = ruster_picker::PickerState<PickerAction>;
/// One selectable entry carrying an editor action.
pub type PickerItem = ruster_picker::PickerItem<PickerAction>;

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
