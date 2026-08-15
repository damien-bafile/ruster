//! The Super+Space launcher: a query engine with pluggable providers.
//!
//! Applications are the first provider, not the point of it. The request was for
//! something expandable — maths was the named example — so the shape is a query
//! fanned out to providers that score on one scale, with the results grouped
//! under their headings.
//!
//! Every row resolves to an [`Action`](crate::lua::Action), the same funnel a
//! keybind, the `:` prompt and `ruster.wm.*` go through. `minibuffer.rs` states
//! the rule this inherits: no route into the WM can do something the others
//! cannot.

pub mod desktop;
pub mod luaprov;
pub mod math;
pub mod provider;

pub use provider::{
    order_groups, Activation, Candidate, Group, Provider, ProviderCtx, ProviderSet,
};

/// The open launcher: what has been typed, and what answered.
pub struct Launcher {
    /// The providers' input — not a filter over a fixed list, which is why it
    /// does not live in the `PickerState` below.
    pub query: String,
    /// Row storage, wrapping selection and accept, from the shared picker.
    ///
    /// Its own filter is deliberately left empty. Providers have already scored
    /// against the query, and filtering again by label would drop the maths row
    /// answering `6*7`, whose label is `42` and matches nothing the user typed.
    rows: ruster_picker::PickerState<Activation>,
    /// The group each row belongs to, parallel to the rows themselves. Carried
    /// per row rather than as a tree so the selection stays a flat index and the
    /// scroll arithmetic has one thing to count.
    groups: Vec<String>,
    details: Vec<String>,
    scroll: usize,
    /// Shown when nothing answered.
    pub message: String,
}

impl Default for Launcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Launcher {
    pub fn new() -> Self {
        Launcher {
            query: String::new(),
            rows: ruster_picker::PickerState::new("launcher", Vec::new()),
            groups: Vec::new(),
            details: Vec::new(),
            scroll: 0,
            message: String::new(),
        }
    }

    pub fn push(&mut self, c: char) {
        self.query.push(c);
    }

    /// Delete a character. `true` when the query is now empty, which closes the
    /// launcher — the same rule the mini-buffer follows when you backspace past
    /// its sigil.
    pub fn backspace(&mut self) -> bool {
        self.query.pop();
        self.query.is_empty()
    }

    pub fn move_selection(&mut self, delta: i32) {
        self.rows.move_selection(delta);
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn selected(&self) -> usize {
        self.rows.selected
    }

    /// Replace the results, flattening the groups into one list.
    pub fn set_groups(&mut self, groups: Vec<Group>) {
        let mut items = Vec::new();
        self.groups.clear();
        self.details.clear();
        for group in groups {
            for row in group.rows {
                self.groups.push(group.name.clone());
                self.details.push(row.detail);
                items.push(ruster_picker::PickerItem::new(row.label, row.activation));
            }
        }
        self.rows.set_items(items);
        self.scroll = 0;
        self.message = if self.rows.is_empty() && !self.query.is_empty() {
            "no matches".to_string()
        } else {
            String::new()
        };
    }

    pub fn accept(&mut self) -> Option<Activation> {
        self.rows.accept()
    }

    /// Build the view for a panel `viewport` rows tall.
    pub fn view(&mut self, viewport: usize) -> ruster_render::LauncherView {
        let total = self.rows.len();
        let selected = self.rows.selected.min(total.saturating_sub(1));
        self.scroll = ruster_render::list_scroll(self.scroll, selected, viewport, total);
        let view = self.rows.view();
        let rows = view
            .rows
            .into_iter()
            .enumerate()
            .skip(self.scroll)
            .take(viewport.max(1))
            .map(|(i, row)| ruster_render::LauncherRow {
                label: row.label,
                detail: self.details.get(i).cloned().unwrap_or_default(),
                // A heading is emitted only where the group changes, so a run of
                // rows from one provider sits under one header.
                group: match i {
                    0 => self.groups.first().cloned().unwrap_or_default(),
                    n if self.groups.get(n) != self.groups.get(n - 1) => {
                        self.groups.get(n).cloned().unwrap_or_default()
                    }
                    _ => String::new(),
                },
                selected: row.selected,
            })
            .collect();
        ruster_render::LauncherView {
            query: self.query.clone(),
            rows,
            message: self.message.clone(),
            scrolled: self.scroll,
            total,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(name: &str, labels: &[&str]) -> Group {
        Group {
            name: name.to_string(),
            rows: labels
                .iter()
                .map(|l| Candidate {
                    label: l.to_string(),
                    detail: format!("detail for {l}"),
                    score: 500,
                    activation: Activation::Report(l.to_string()),
                })
                .collect(),
        }
    }

    #[test]
    fn a_maths_answer_survives_a_query_it_does_not_look_like() {
        // The property that decides whether reusing the picker was right. Its
        // filter matches labels against what was typed; the maths row answering
        // `6*7` is labelled `42`, which matches none of it. Filtering twice
        // would drop the only certain answer on screen.
        let mut l = Launcher::new();
        for c in "6*7".chars() {
            l.push(c);
        }
        l.set_groups(vec![group("maths", &["42"])]);
        assert_eq!(l.row_count(), 1);
        assert_eq!(l.accept(), Some(Activation::Report("42".into())));
    }

    #[test]
    fn the_selection_wraps_across_group_boundaries() {
        // Groups are a drawing concern. Moving up from the first row must reach
        // the last row of the last group, not the last row of the first one.
        let mut l = Launcher::new();
        l.set_groups(vec![
            group("a", &["a1", "a2"]),
            group("b", &["b1", "b2"]),
            group("c", &["c1", "c2"]),
        ]);
        assert_eq!(l.row_count(), 6);
        assert_eq!(l.selected(), 0);
        l.move_selection(-1);
        assert_eq!(l.selected(), 5, "wraps to the last row overall");
        assert_eq!(l.accept(), Some(Activation::Report("c2".into())));
        l.move_selection(1);
        assert_eq!(l.selected(), 0);
    }

    #[test]
    fn a_heading_is_drawn_once_per_run_of_rows() {
        // Not per row: three rows from one provider under three identical
        // headers is noise, and the header is how a reader tells a computed
        // answer from a matched one.
        let mut l = Launcher::new();
        l.set_groups(vec![
            group("apps", &["Firefox", "Files"]),
            group("maths", &["42"]),
        ]);
        let view = l.view(10);
        let headers: Vec<&str> = view.rows.iter().map(|r| r.group.as_str()).collect();
        assert_eq!(headers, vec!["apps", "", "maths"]);
        assert_eq!(view.rows.iter().filter(|r| r.selected).count(), 1);
        assert_eq!(view.rows[0].detail, "detail for Firefox");
    }

    #[test]
    fn the_view_shows_a_window_around_the_selection() {
        let mut l = Launcher::new();
        let labels: Vec<String> = (0..20).map(|i| format!("row{i}")).collect();
        let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
        l.set_groups(vec![group("many", &refs)]);

        let view = l.view(5);
        assert_eq!(view.rows.len(), 5, "only the viewport is emitted");
        assert_eq!(view.total, 20, "but the whole count is reported");
        assert_eq!(view.scrolled, 0);

        // Move past the bottom of the viewport and it must follow.
        for _ in 0..7 {
            l.move_selection(1);
        }
        let view = l.view(5);
        assert!(view.scrolled > 0, "the window followed the selection");
        assert!(
            view.rows.iter().any(|r| r.selected),
            "and the selected row is inside it"
        );
    }

    #[test]
    fn an_empty_result_says_so_only_once_something_was_typed() {
        let mut l = Launcher::new();
        l.set_groups(vec![]);
        assert_eq!(l.message, "", "an empty launcher is not a failed search");
        l.push('z');
        l.set_groups(vec![]);
        assert_eq!(l.message, "no matches");
    }

    #[test]
    fn backspacing_past_the_last_character_closes_it() {
        let mut l = Launcher::new();
        l.push('a');
        assert!(l.backspace(), "empty now, so the launcher closes");
        l.push('a');
        l.push('b');
        assert!(!l.backspace(), "still has 'a', so it stays open");
    }
}
