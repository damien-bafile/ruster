//! The `:Git` status view (Phase 7 Task 3, stage 1) — read-only for now.
//!
//! Staged, unstaged and untracked files in three foldable sections. All of the
//! behaviour is here and pure: grouping, folding, and resolving a screen row
//! back to a file. `App` supplies the parsed status and turns a resolved path
//! into an action, so none of this needs a repository — or an editor — to test.
//!
//! Deliberately the same shape as [`crate::trouble`], which already solves
//! "foldable sectioned list, screen row → item". A third independent copy of
//! that logic is how the three drift apart.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use ruster_git::{FileStatus, Status, StatusEntry};
use ruster_render::StyledLine;

/// The three groups a file can appear in. A file edited, staged, then edited
/// again appears in **both** `Staged` and `Unstaged` — that is what the `XY`
/// pair means, not a contradiction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Section {
    Staged,
    Unstaged,
    Untracked,
}

impl Section {
    pub fn heading(self) -> &'static str {
        match self {
            Section::Staged => "Staged",
            Section::Unstaged => "Unstaged",
            Section::Untracked => "Untracked",
        }
    }

    const ALL: [Section; 3] = [Section::Staged, Section::Unstaged, Section::Untracked];
}

/// One rendered row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Section { section: Section, count: usize, collapsed: bool },
    /// A file within a section, by index into that section's list.
    Entry { section: Section, index: usize },
}

#[derive(Debug, Default)]
pub struct GitStatusState {
    status: Status,
    /// Folded sections, kept across refreshes so a rebuild does not silently
    /// expand what the user collapsed.
    collapsed: BTreeSet<Section>,
}

impl GitStatusState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_status(&mut self, status: Status) {
        self.status = status;
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    /// The entries in one section.
    pub fn entries(&self, section: Section) -> Vec<&StatusEntry> {
        match section {
            Section::Staged => self.status.staged(),
            Section::Unstaged => self.status.unstaged(),
            Section::Untracked => self.status.untracked(),
        }
    }

    /// The visible rows, in display order. Empty sections are omitted — a
    /// heading with nothing under it is noise.
    pub fn rows(&self) -> Vec<Row> {
        let mut out = Vec::new();
        for section in Section::ALL {
            let count = self.entries(section).len();
            if count == 0 {
                continue;
            }
            let collapsed = self.collapsed.contains(&section);
            out.push(Row::Section { section, count, collapsed });
            if !collapsed {
                out.extend((0..count).map(|index| Row::Entry { section, index }));
            }
        }
        out
    }

    /// Fold or unfold the section a row belongs to, so `za` on a file collapses
    /// its section.
    pub fn toggle_at(&mut self, row: usize) {
        let Some(section) = self.rows().get(row).map(|r| match r {
            Row::Section { section, .. } | Row::Entry { section, .. } => *section,
        }) else {
            return;
        };
        if !self.collapsed.remove(&section) {
            self.collapsed.insert(section);
        }
    }

    /// The file a row refers to, or `None` for a heading.
    pub fn path_at(&self, row: usize) -> Option<PathBuf> {
        match self.rows().get(row)? {
            Row::Section { .. } => None,
            Row::Entry { section, index } => {
                self.entries(*section).get(*index).map(|e| e.path.clone())
            }
        }
    }

    /// The buffer text.
    pub fn render(&self, root: Option<&Path>) -> String {
        self.layout(root).into_iter().map(|(text, _)| text).collect::<Vec<_>>().join("\n")
    }

    /// The row a rendered line belongs to, or `None` for the branch header and
    /// blank lines.
    ///
    /// A section heading *does* resolve, to its own row: folding has to work
    /// with the cursor on a heading, which is where it naturally sits. Asking
    /// whether that row is a file is [`path_at`](Self::path_at)'s job, and it
    /// answers `None` — so `Enter` on a heading still does nothing.
    ///
    /// Derived from the same layout `render` emits rather than by offsetting a
    /// constant: the view puts a blank line before every section after the
    /// first, so line and row numbers drift apart by one per section. Computing
    /// it separately is how `Enter` on the second section opens the wrong file.
    pub fn row_at_line(&self, line: usize) -> Option<usize> {
        self.layout(None).get(line).and_then(|(_, row)| *row)
    }

    /// Every rendered line, paired with the row index it represents.
    fn layout(&self, root: Option<&Path>) -> Vec<(String, Option<usize>)> {
        let mut out: Vec<(String, Option<usize>)> = vec![(self.header(), None), (String::new(), None)];

        if self.status.is_clean() {
            out.push(("Nothing to commit, working tree clean.".to_string(), None));
            return out;
        }

        for (row_index, row) in self.rows().into_iter().enumerate() {
            match row {
                Row::Section { section, count, collapsed } => {
                    if !out.last().is_some_and(|(t, _)| t.is_empty()) {
                        out.push((String::new(), None));
                    }
                    out.push((
                        format!(
                            "{} {} ({count})",
                            if collapsed { '▸' } else { '▾' },
                            section.heading()
                        ),
                        Some(row_index),
                    ));
                }
                Row::Entry { section, index } => {
                    let entries = self.entries(section);
                    let Some(e) = entries.get(index) else { continue };
                    out.push((format!("    {}", entry_line(e, section, root)), Some(row_index)));
                }
            }
        }
        out
    }

    fn header(&self) -> String {
        let branch = self.status.branch.as_deref().unwrap_or("(detached)");
        let mut s = format!("On branch {branch}");
        if let Some(up) = &self.status.upstream {
            s.push_str(&format!(" → {up}"));
            match (self.status.ahead, self.status.behind) {
                (0, 0) => {}
                (a, 0) => s.push_str(&format!(" (ahead {a})")),
                (0, b) => s.push_str(&format!(" (behind {b})")),
                (a, b) => s.push_str(&format!(" (ahead {a}, behind {b})")),
            }
        }
        s
    }
}

/// One file's line: its status letter for *this* section, then the path.
fn entry_line(e: &StatusEntry, section: Section, root: Option<&Path>) -> String {
    let status = match section {
        Section::Staged => e.staged,
        Section::Unstaged | Section::Untracked => e.unstaged,
    };
    let letter = status.map_or(' ', FileStatus::letter);
    let name = display_path(&e.path, root);
    match &e.orig_path {
        // A rename is only meaningful with both names.
        Some(old) if section == Section::Staged => {
            format!("{letter} {} ← {}", name, display_path(old, root))
        }
        _ => format!("{letter} {name}"),
    }
}

fn display_path(p: &Path, root: Option<&Path>) -> String {
    root.and_then(|r| p.strip_prefix(r).ok()).unwrap_or(p).display().to_string()
}

/// Colour a unified diff the way every other diff tool does: additions green,
/// deletions red, hunk headers accented, file headers dim.
///
/// Colours come from the `diff` pseudo-language in `ruster-syntax`, so they
/// follow the active theme and honour `ruster.config.syntax.diff.*` like every
/// other syntax group — rather than being four hardcoded values that no theme
/// can reach.
pub fn diff_styled_lines(text: &str) -> Vec<StyledLine> {
    // Overrides are looked up per language, so name the one being drawn.
    ruster_syntax::set_current_lang("diff");
    text.lines()
        .map(|line| {
            let kind = if line.starts_with("+++") || line.starts_with("---") {
                // File headers first: they start with +/- but are not changes.
                Some("header")
            } else if line.starts_with('+') {
                Some("added")
            } else if line.starts_with('-') {
                Some("removed")
            } else if line.starts_with("@@") {
                Some("hunk")
            } else if line.starts_with("diff --git") || line.starts_with("index ") {
                Some("header")
            } else {
                None
            };
            let highlights = match kind {
                Some(k) => vec![(0, line.chars().count(), ruster_syntax::diff_style(k))],
                None => Vec::new(),
            };
            StyledLine { text: line.to_string(), highlights }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# branch.head main
# branch.upstream origin/main
# branch.ab +2 -1
1 A. N... 000000 100644 100644 0000000 3e75765 added.txt
1 MM N... 100644 100644 100644 814f4a4 05b65e8 both.txt
1 .M N... 100644 100644 100644 aaa1111 aaa1111 edited.txt
2 R. N... 100644 100644 100644 7b26523 7b26523 R100 new-name.txt\told-name.txt
? untracked.txt
";

    fn state() -> GitStatusState {
        let mut s = GitStatusState::new();
        s.set_status(ruster_git::parse_status(SAMPLE));
        s
    }

    #[test]
    fn a_diff_is_coloured_by_line_kind() {
        let diff = "\
diff --git a/f b/f
index 1..2 100644
--- a/f
+++ b/f
@@ -1,2 +1,2 @@
 context
-removed
+added
";
        let lines = diff_styled_lines(diff);
        let kind = |i: usize| lines[i].highlights.first().map(|(_, _, s)| s.fg);
        let of = |g: &str| ruster_syntax::diff_style(g).fg;

        assert_eq!(kind(0), Some(of("header")), "diff --git");
        assert_eq!(kind(1), Some(of("header")), "index");
        // `---`/`+++` start with -/+ but are headers, not changes. Testing
        // these before the change cases is the whole subtlety.
        assert_eq!(kind(2), Some(of("header")), "--- header, not a deletion");
        assert_eq!(kind(3), Some(of("header")), "+++ header, not an addition");
        assert_eq!(kind(4), Some(of("hunk")), "@@");
        assert_eq!(kind(5), None, "context is unstyled");
        assert_eq!(kind(6), Some(of("removed")));
        assert_eq!(kind(7), Some(of("added")));
        // The four are actually distinct, or the colouring says nothing.
        let all = [of("added"), of("removed"), of("hunk"), of("header")];
        for (i, a) in all.iter().enumerate() {
            for b in all.iter().skip(i + 1) {
                assert_ne!(a, b, "diff groups must not share a colour");
            }
        }
    }

    /// The colours are themeable, not hardcoded: an override reaches the diff.
    #[test]
    fn a_syntax_override_recolours_the_diff() {
        let mut ov = ruster_syntax::SyntaxOverrides::new();
        let mut groups = std::collections::HashMap::new();
        groups.insert("added".to_string(), ruster_render::Color::Rgb(1, 2, 3));
        ov.insert("diff".to_string(), groups);
        ruster_syntax::set_syntax_overrides(ov);

        let lines = diff_styled_lines("+added\n");
        assert_eq!(
            lines[0].highlights.first().map(|(_, _, s)| s.fg),
            Some(ruster_render::Color::Rgb(1, 2, 3)),
            "ruster.config.syntax.diff.added took effect"
        );

        // Leave the global clean for other tests.
        ruster_syntax::set_syntax_overrides(ruster_syntax::SyntaxOverrides::new());
    }

    #[test]
    fn sections_list_the_right_files() {
        let s = state();
        let names = |sec| {
            s.entries(sec).iter().map(|e| e.path.display().to_string()).collect::<Vec<_>>()
        };
        assert_eq!(names(Section::Staged), ["added.txt", "both.txt", "new-name.txt"]);
        assert_eq!(names(Section::Unstaged), ["both.txt", "edited.txt"]);
        assert_eq!(names(Section::Untracked), ["untracked.txt"]);
    }

    /// The `MM` case, at the level the user sees it.
    #[test]
    fn a_partially_staged_file_appears_in_two_sections() {
        let out = state().render(None);
        let lines: Vec<&str> = out.lines().filter(|l| l.contains("both.txt")).collect();
        assert_eq!(lines.len(), 2, "once staged, once unstaged: {out}");
    }

    #[test]
    fn the_header_shows_branch_upstream_and_divergence() {
        let h = state().render(None);
        assert!(h.starts_with("On branch main → origin/main (ahead 2, behind 1)"), "{h}");
    }

    #[test]
    fn folding_a_section_hides_its_files_but_keeps_the_heading() {
        let mut s = state();
        let before = s.rows().len();
        s.toggle_at(0); // the Staged heading
        let rows = s.rows();
        assert_eq!(rows.len(), before - 3, "three staged files hidden");
        assert!(matches!(rows[0], Row::Section { collapsed: true, .. }));
        s.toggle_at(0);
        assert_eq!(s.rows().len(), before, "unfolds again");
    }

    #[test]
    fn folding_from_inside_a_section_collapses_that_section() {
        let mut s = state();
        s.toggle_at(1); // a file, not the heading
        assert!(matches!(s.rows()[0], Row::Section { collapsed: true, .. }));
    }

    /// Folds are per section and must survive a refresh, or every rebuild
    /// silently reopens what the user closed.
    #[test]
    fn folds_survive_a_refresh() {
        let mut s = state();
        s.toggle_at(0);
        let folded = s.rows().len();
        s.set_status(ruster_git::parse_status(SAMPLE));
        assert_eq!(s.rows().len(), folded, "still folded after a rebuild");
    }

    #[test]
    fn a_heading_resolves_to_no_file_but_a_row_does() {
        let s = state();
        assert_eq!(s.path_at(0), None, "headings are not files");
        assert_eq!(s.path_at(1), Some(PathBuf::from("added.txt")));
        assert_eq!(s.path_at(9999), None);
    }

    #[test]
    fn a_rename_shows_both_names_in_the_staged_section() {
        let out = state().render(None);
        let line = out.lines().find(|l| l.contains("new-name.txt")).expect("listed");
        assert!(line.contains("old-name.txt"), "shows where it came from: {line:?}");
        assert!(line.contains('R'), "and that it is a rename: {line:?}");
    }

    #[test]
    fn an_empty_section_is_omitted_entirely() {
        let mut s = GitStatusState::new();
        s.set_status(ruster_git::parse_status("# branch.head main\n? only-untracked.txt\n"));
        let out = s.render(None);
        assert!(out.contains("Untracked"), "{out}");
        assert!(!out.contains("Staged"), "no empty heading: {out}");
    }

    #[test]
    fn a_clean_tree_says_so() {
        let mut s = GitStatusState::new();
        s.set_status(ruster_git::parse_status("# branch.head main\n"));
        let out = s.render(None);
        assert!(out.contains("working tree clean"), "{out}");
        assert!(s.rows().is_empty());
    }

    #[test]
    fn paths_are_shown_relative_to_the_project_root() {
        let mut s = GitStatusState::new();
        s.set_status(ruster_git::parse_status("# branch.head m\n? /proj/src/deep.rs\n"));
        let out = s.render(Some(Path::new("/proj")));
        assert!(out.contains("src/deep.rs"), "{out}");
        assert!(!out.contains("/proj/src"), "root stripped: {out}");
    }
}
