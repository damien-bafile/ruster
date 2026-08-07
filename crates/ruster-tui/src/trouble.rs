//! The Trouble problem list: diagnostics, quickfix entries and TODO markers
//! aggregated into one pinned buffer, grouped by file and foldable.
//!
//! All of the behaviour is here and pure — grouping, folding and resolving a
//! screen row back to a jump target. `App` supplies the items and turns a
//! resolved target into a jump, so none of this needs a workspace to test.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Where an entry came from. Kept so the list can say why a line is listed —
/// an LSP error and a `TODO` note are both "problems" but not the same kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    Diagnostic,
    Quickfix,
    Todo,
}

impl Source {
    /// How much this source is trusted to describe a problem, lowest first.
    ///
    /// The quickfix list is a *container* — `:TodoList` and the build runner
    /// copy their entries into it — so the same problem can arrive twice. When
    /// it does, the specific source wins and the quickfix copy is dropped.
    fn rank(self) -> u8 {
        match self {
            Source::Quickfix => 0,
            Source::Todo => 1,
            Source::Diagnostic => 2,
        }
    }

    /// The one-character tag shown before a message.
    pub fn tag(self) -> char {
        match self {
            Source::Diagnostic => 'D',
            Source::Quickfix => 'Q',
            Source::Todo => 'T',
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TroubleItem {
    pub path: PathBuf,
    /// 0-based.
    pub line: usize,
    /// 0-based.
    pub col: usize,
    pub message: String,
    /// LSP-style: 1 = error, 2 = warning, 3 = info, 4 = hint.
    pub severity: u8,
    pub source: Source,
}

/// One rendered row: either a file heading or an entry beneath one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Group { path: PathBuf, count: usize, collapsed: bool },
    /// Index into [`TroubleState::items`].
    Item(usize),
}

#[derive(Debug, Default)]
pub struct TroubleState {
    items: Vec<TroubleItem>,
    /// Files whose entries are hidden. Keyed by path so folds survive a refresh
    /// — otherwise every rebuild would silently expand everything again.
    collapsed: BTreeSet<PathBuf>,
}

impl TroubleState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Replace the contents, keeping which files are folded.
    ///
    /// Sorted by path, then position, then source, so the list is stable across
    /// refreshes rather than reordering under the cursor.
    pub fn set_items(&mut self, mut items: Vec<TroubleItem>) {
        // Highest-ranked source first at each position, so the dedupe below
        // keeps it and drops the quickfix copy of the same problem.
        items.sort_by(|a, b| {
            (&a.path, a.line, a.col, std::cmp::Reverse(a.source.rank()), &a.message).cmp(&(
                &b.path,
                b.line,
                b.col,
                std::cmp::Reverse(b.source.rank()),
                &b.message,
            ))
        });
        items.dedup_by(|a, b| {
            a.path == b.path && a.line == b.line && a.col == b.col && a.message == b.message
        });
        items.sort_by(|a, b| {
            (&a.path, a.line, a.col, a.source).cmp(&(&b.path, b.line, b.col, b.source))
        });
        self.items = items;
    }

    /// The visible rows, in display order.
    pub fn rows(&self) -> Vec<Row> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.items.len() {
            let path = &self.items[i].path;
            let count = self.items[i..].iter().take_while(|it| &it.path == path).count();
            let collapsed = self.collapsed.contains(path);
            out.push(Row::Group { path: path.clone(), count, collapsed });
            if !collapsed {
                out.extend((i..i + count).map(Row::Item));
            }
            i += count;
        }
        out
    }

    /// Fold or unfold the group a row belongs to. A row inside a group folds
    /// that group, so `za` on an entry collapses its file.
    pub fn toggle_at(&mut self, row: usize) {
        let Some(path) = self.rows().get(row).map(|r| match r {
            Row::Group { path, .. } => path.clone(),
            Row::Item(i) => self.items[*i].path.clone(),
        }) else {
            return;
        };
        if !self.collapsed.remove(&path) {
            self.collapsed.insert(path);
        }
    }

    /// The jump target for a row, or `None` for a group heading.
    pub fn target_at(&self, row: usize) -> Option<(PathBuf, usize, usize)> {
        match self.rows().get(row)? {
            Row::Group { .. } => None,
            Row::Item(i) => {
                let it = self.items.get(*i)?;
                Some((it.path.clone(), it.line, it.col))
            }
        }
    }

    /// The buffer text: one line per row.
    pub fn render(&self, root: Option<&Path>) -> String {
        if self.items.is_empty() {
            return "No problems.".to_string();
        }
        self.rows()
            .iter()
            .map(|r| match r {
                Row::Group { path, count, collapsed } => {
                    let name = root
                        .and_then(|r| path.strip_prefix(r).ok())
                        .unwrap_or(path)
                        .display()
                        .to_string();
                    format!("{} {} ({})", if *collapsed { '▸' } else { '▾' }, name, count)
                }
                Row::Item(i) => {
                    let it = &self.items[*i];
                    let sev = match it.severity {
                        1 => 'E',
                        2 => 'W',
                        3 => 'I',
                        _ => 'H',
                    };
                    // Lines are 0-based internally and 1-based on screen, as
                    // everywhere else the editor shows a position.
                    format!(
                        "    {}{} {}:{}  {}",
                        it.source.tag(),
                        sev,
                        it.line + 1,
                        it.col + 1,
                        it.message
                    )
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(path: &str, line: usize, source: Source) -> TroubleItem {
        TroubleItem {
            path: PathBuf::from(path),
            line,
            col: 0,
            message: format!("{path}:{line}"),
            severity: 2,
            source,
        }
    }

    fn state() -> TroubleState {
        let mut t = TroubleState::new();
        t.set_items(vec![
            item("b.rs", 5, Source::Todo),
            item("a.rs", 10, Source::Quickfix),
            item("a.rs", 2, Source::Diagnostic),
        ]);
        t
    }

    #[test]
    fn items_group_by_file_and_sort_within_a_group() {
        let t = state();
        let rows = t.rows();
        // a.rs heading, its two entries in line order, then b.rs and its one.
        assert_eq!(rows.len(), 5);
        assert!(matches!(&rows[0], Row::Group { path, count: 2, .. } if path.ends_with("a.rs")));
        assert_eq!(t.target_at(1).unwrap().1, 2, "lower line first");
        assert_eq!(t.target_at(2).unwrap().1, 10);
        assert!(matches!(&rows[3], Row::Group { path, count: 1, .. } if path.ends_with("b.rs")));
    }

    #[test]
    fn a_group_heading_has_no_jump_target() {
        let t = state();
        assert_eq!(t.target_at(0), None, "headings are not jumpable");
        assert_eq!(t.target_at(1), Some((PathBuf::from("a.rs"), 2, 0)));
    }

    #[test]
    fn folding_a_group_hides_its_entries_but_keeps_the_heading() {
        let mut t = state();
        assert_eq!(t.rows().len(), 5);
        t.toggle_at(0);
        let rows = t.rows();
        assert_eq!(rows.len(), 3, "a.rs collapsed to its heading");
        assert!(matches!(&rows[0], Row::Group { collapsed: true, .. }));
        // b.rs is untouched and still resolves.
        assert_eq!(t.target_at(2), Some((PathBuf::from("b.rs"), 5, 0)));
        t.toggle_at(0);
        assert_eq!(t.rows().len(), 5, "unfolds again");
    }

    #[test]
    fn folding_from_inside_a_group_collapses_that_group() {
        let mut t = state();
        t.toggle_at(1); // an entry, not the heading
        assert!(matches!(&t.rows()[0], Row::Group { collapsed: true, .. }));
    }

    /// Folds are keyed by path, so a refresh must not silently expand
    /// everything the user had collapsed.
    #[test]
    fn folds_survive_a_refresh() {
        let mut t = state();
        t.toggle_at(0);
        assert_eq!(t.rows().len(), 3);
        t.set_items(vec![item("a.rs", 2, Source::Diagnostic), item("b.rs", 5, Source::Todo)]);
        assert!(
            matches!(&t.rows()[0], Row::Group { collapsed: true, .. }),
            "a.rs stays folded across a rebuild"
        );
    }

    /// `:TodoList` copies markers into the quickfix list, so the same problem
    /// reaches the panel twice. The specific source wins; the container loses.
    #[test]
    fn the_same_problem_from_two_sources_is_listed_once() {
        let mut t = TroubleState::new();
        let mut q = item("a.rs", 3, Source::Quickfix);
        q.message = "TODO: thing".into();
        let mut td = item("a.rs", 3, Source::Todo);
        td.message = "TODO: thing".into();
        t.set_items(vec![q, td]);
        assert_eq!(t.len(), 1, "deduplicated");
        assert!(t.render(None).contains('T'), "kept the Todo tag, not the Quickfix one");

        // A diagnostic outranks both.
        let mut t2 = TroubleState::new();
        let mut d = item("a.rs", 3, Source::Diagnostic);
        d.message = "same".into();
        let mut q2 = item("a.rs", 3, Source::Quickfix);
        q2.message = "same".into();
        t2.set_items(vec![q2, d]);
        assert_eq!(t2.len(), 1);
        assert!(t2.render(None).contains("DW"), "{}", t2.render(None));
    }

    /// Different messages at the same position are different problems.
    #[test]
    fn different_messages_at_one_position_both_survive() {
        let mut t = TroubleState::new();
        let mut a = item("a.rs", 3, Source::Diagnostic);
        a.message = "unused variable".into();
        let mut b = item("a.rs", 3, Source::Todo);
        b.message = "TODO: fix".into();
        t.set_items(vec![a, b]);
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn render_shows_source_severity_and_a_one_based_position() {
        let t = state();
        let text = t.render(None);
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with("▾ a.rs (2)"), "{:?}", lines[0]);
        // Diagnostic + warning, at 0-based line 2 → shown as line 3.
        assert!(lines[1].contains("DW 3:1"), "{:?}", lines[1]);
        assert!(lines[3].starts_with("▸ b.rs") || lines[3].starts_with("▾ b.rs"));
    }

    #[test]
    fn render_strips_the_project_root_from_headings() {
        let mut t = TroubleState::new();
        t.set_items(vec![item("/proj/src/main.rs", 0, Source::Todo)]);
        let text = t.render(Some(Path::new("/proj")));
        assert!(text.starts_with("▾ src/main.rs"), "{text:?}");
    }

    #[test]
    fn an_empty_list_says_so_rather_than_rendering_nothing() {
        assert_eq!(TroubleState::new().render(None), "No problems.");
    }

    #[test]
    fn out_of_range_rows_resolve_to_nothing() {
        let t = state();
        assert_eq!(t.target_at(99), None);
        let mut t2 = state();
        t2.toggle_at(99); // must not panic
    }
}
