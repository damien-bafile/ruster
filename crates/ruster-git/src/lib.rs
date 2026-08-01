//! Git awareness for ruster (Phase 6): which lines of a file differ from the
//! index, so the gutter can mark them.
//!
//! The parsing is pure and is where all the behaviour lives — [`parse_hunks`]
//! takes the text of `git diff --no-color -U0` and needs no repository, so the
//! tests run anywhere. [`diff_hunks`] is the thin shell-out that feeds it, and
//! is deliberately the only part that touches the filesystem.

use std::path::Path;
use std::process::Command;

/// What happened to a run of lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunkKind {
    Added,
    Modified,
    /// Lines were deleted here. [`Hunk::count`] is 0 — a deletion has no lines
    /// of its own in the working file, only a boundary to mark.
    Removed,
}

/// A run of changed lines, in **0-based working-file** coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hunk {
    pub kind: HunkKind,
    pub start: u32,
    pub count: u32,
}

impl Hunk {
    /// The lines this hunk covers. Empty for a deletion, which marks the
    /// boundary line via [`start`](Self::start) instead.
    pub fn lines(&self) -> impl Iterator<Item = u32> {
        self.start..self.start + self.count
    }
}

/// Parse the hunk headers out of `git diff --no-color -U0` output.
///
/// Only `@@` lines matter at `-U0`: with no context, every header's line counts
/// describe exactly the changed run. A header is
/// `@@ -old_start[,old_count] +new_start[,new_count] @@`, where an omitted count
/// means 1.
///
/// The mapping to [`HunkKind`]:
/// - `old_count == 0` — nothing was replaced, so the new lines are **added**.
/// - `new_count == 0` — nothing remains, so the lines were **removed**, and the
///   sign goes on the line above the gap (see below).
/// - otherwise the new lines replaced old ones, so they are **modified**.
pub fn parse_hunks(diff: &str) -> Vec<Hunk> {
    diff.lines().filter_map(parse_hunk_header).collect()
}

fn parse_hunk_header(line: &str) -> Option<Hunk> {
    let rest = line.strip_prefix("@@ ")?;
    let end = rest.find(" @@")?;
    let mut parts = rest[..end].split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let (_, old_count) = split_range(old)?;
    let (new_start, new_count) = split_range(new)?;

    Some(if old_count == 0 {
        Hunk {
            kind: HunkKind::Added,
            // `+N,M` counts from 1; the gutter indexes from 0.
            start: new_start.saturating_sub(1),
            count: new_count,
        }
    } else if new_count == 0 {
        // A deletion leaves no lines to mark, so the sign goes on the line above
        // the gap. git reports the 1-based position the removed lines *would*
        // have occupied, so subtracting one both converts to 0-based and steps
        // onto the preceding line — which keeps a deletion at end-of-file inside
        // the buffer instead of one line past it, where it would never render.
        Hunk { kind: HunkKind::Removed, start: new_start.saturating_sub(1), count: 0 }
    } else {
        Hunk {
            kind: HunkKind::Modified,
            start: new_start.saturating_sub(1),
            count: new_count,
        }
    })
}

/// `start[,count]` — an absent count means 1, as in the diff format.
fn split_range(s: &str) -> Option<(u32, u32)> {
    match s.split_once(',') {
        Some((a, b)) => Some((a.parse().ok()?, b.parse().ok()?)),
        None => Some((s.parse().ok()?, 1)),
    }
}

/// Hunks for `path` against the index, or `None` when git is unavailable, the
/// path is untracked, or the command fails — all of which are normal and must
/// not surface as errors.
pub fn diff_hunks(root: &Path, path: &Path) -> Option<Vec<Hunk>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["diff", "--no-color", "-U0", "--"])
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(parse_hunks(&String::from_utf8_lossy(&out.stdout)))
}

/// Whether `root` looks like a git working tree. Cheap enough to call on a
/// buffer switch.
pub fn is_repo(root: &Path) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// The next hunk strictly after `line`, wrapping to the first. `None` when
/// there are no hunks.
pub fn next_hunk(hunks: &[Hunk], line: u32) -> Option<&Hunk> {
    hunks.iter().find(|h| h.start > line).or_else(|| hunks.first())
}

/// The previous hunk strictly before `line`, wrapping to the last.
pub fn prev_hunk(hunks: &[Hunk], line: u32) -> Option<&Hunk> {
    hunks.iter().rev().find(|h| h.start < line).or_else(|| hunks.last())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real `git diff -U0`, trimmed to the parts the parser reads.
    const SAMPLE: &str = "\
diff --git a/src/main.rs b/src/main.rs
index 83db48f..bf269f4 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -12 +12 @@ fn main() {
-    let x = 1;
+    let x = 2;
@@ -20,0 +21,3 @@ fn main() {
+    added one
+    added two
+    added three
@@ -30,2 +32,0 @@ fn main() {
-    gone one
-    gone two
";

    #[test]
    fn parses_a_mixed_diff() {
        let h = parse_hunks(SAMPLE);
        assert_eq!(h.len(), 3);
        // `@@ -12 +12 @@` — one line replaced one line.
        assert_eq!(h[0], Hunk { kind: HunkKind::Modified, start: 11, count: 1 });
        // `@@ -20,0 +21,3 @@` — three lines added, nothing replaced.
        assert_eq!(h[1], Hunk { kind: HunkKind::Added, start: 20, count: 3 });
        // `@@ -30,2 +32,0 @@` — two lines deleted; the sign sits on the line above.
        assert_eq!(h[2], Hunk { kind: HunkKind::Removed, start: 31, count: 0 });
    }

    #[test]
    fn an_absent_count_means_one_line() {
        let h = parse_hunks("@@ -5 +5 @@\n");
        assert_eq!(h, [Hunk { kind: HunkKind::Modified, start: 4, count: 1 }]);
    }

    #[test]
    fn a_new_file_is_all_additions() {
        // What git emits for a file that did not exist before.
        let h = parse_hunks("@@ -0,0 +1,717 @@\n");
        assert_eq!(h, [Hunk { kind: HunkKind::Added, start: 0, count: 717 }]);
    }

    #[test]
    fn a_fully_deleted_file_marks_the_top() {
        let h = parse_hunks("@@ -1,40 +0,0 @@\n");
        assert_eq!(h, [Hunk { kind: HunkKind::Removed, start: 0, count: 0 }]);
    }

    #[test]
    fn a_shrinking_change_is_modified_over_the_lines_that_remain() {
        // Five lines became two: still a modification of those two.
        let h = parse_hunks("@@ -10,5 +10,2 @@\n");
        assert_eq!(h, [Hunk { kind: HunkKind::Modified, start: 9, count: 2 }]);
    }

    #[test]
    fn hunk_lines_covers_the_run_and_a_deletion_covers_nothing() {
        let added = Hunk { kind: HunkKind::Added, start: 20, count: 3 };
        assert_eq!(added.lines().collect::<Vec<_>>(), vec![20, 21, 22]);
        let removed = Hunk { kind: HunkKind::Removed, start: 32, count: 0 };
        assert_eq!(removed.lines().count(), 0, "a deletion has no lines of its own");
    }

    /// Non-`@@` lines include `+++ b/file` and `--- a/file`, which start with
    /// the same characters as content lines and must not be mistaken for hunks.
    #[test]
    fn ignores_everything_that_is_not_a_hunk_header() {
        let noise = "\
diff --git a/x b/x
index 1..2 100644
--- a/x
+++ b/x
+added content
-removed content
 context
@@ malformed
@@ -1 +1 @@
";
        assert_eq!(parse_hunks(noise).len(), 1, "only the well-formed header counts");
    }

    #[test]
    fn empty_diff_yields_no_hunks() {
        assert!(parse_hunks("").is_empty());
    }

    fn hunks() -> Vec<Hunk> {
        vec![
            Hunk { kind: HunkKind::Added, start: 5, count: 1 },
            Hunk { kind: HunkKind::Modified, start: 20, count: 2 },
            Hunk { kind: HunkKind::Removed, start: 40, count: 0 },
        ]
    }

    /// A deletion at end-of-file must stay inside the buffer, or its sign is
    /// dropped by the renderer and the change looks invisible.
    #[test]
    fn a_deletion_at_end_of_file_marks_the_last_line() {
        // Captured from a 7-line working file (0-based 0..=6) whose final line
        // was deleted.
        let h = parse_hunks("@@ -6 +7,0 @@ five\n");
        assert_eq!(h, [Hunk { kind: HunkKind::Removed, start: 6, count: 0 }]);
    }

    #[test]
    fn next_hunk_advances_then_wraps() {
        let h = hunks();
        assert_eq!(next_hunk(&h, 0).unwrap().start, 5);
        assert_eq!(next_hunk(&h, 5).unwrap().start, 20, "strictly after the current line");
        assert_eq!(next_hunk(&h, 39).unwrap().start, 40);
        assert_eq!(next_hunk(&h, 999).unwrap().start, 5, "wraps to the first");
    }

    #[test]
    fn prev_hunk_retreats_then_wraps() {
        let h = hunks();
        assert_eq!(prev_hunk(&h, 999).unwrap().start, 40);
        assert_eq!(prev_hunk(&h, 20).unwrap().start, 5, "strictly before the current line");
        assert_eq!(prev_hunk(&h, 0).unwrap().start, 40, "wraps to the last");
    }

    #[test]
    fn navigation_on_an_unchanged_file_finds_nothing() {
        assert!(next_hunk(&[], 3).is_none());
        assert!(prev_hunk(&[], 3).is_none());
    }
}
