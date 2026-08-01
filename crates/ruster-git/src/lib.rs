//! Git awareness for ruster (Phase 6): which lines of a file differ from the
//! index, so the gutter can mark them.
//!
//! The parsing is pure and is where all the behaviour lives — [`parse_hunks`]
//! takes the text of `git diff --no-color -U0` and needs no repository, so the
//! tests run anywhere. [`diff_hunks`] is the thin shell-out that feeds it, and
//! is deliberately the only part that touches the filesystem.

use std::path::{Path, PathBuf};
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

/// A hunk in **raw diff coordinates**: both sides, 0-based, unadjusted.
///
/// Separate from [`Hunk`] on purpose. `Hunk` is the *gutter's* view — one side
/// only, with a deletion pulled back onto the preceding line so it has somewhere
/// to draw. A side-by-side diff needs the position the deletion actually
/// occupies and needs the old side too, so it reads this instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiffHunk {
    pub old_start: u32,
    pub old_count: u32,
    pub new_start: u32,
    pub new_count: u32,
}

/// Parse `@@` headers into raw two-sided coordinates.
///
/// `@@ -old_start[,old_count] +new_start[,new_count] @@`, where an omitted count
/// means 1 and a count of 0 means the range is empty — in which case git reports
/// the 1-based position the lines *would* have taken, so the 0-based start is
/// that number rather than one less.
pub fn parse_diff_hunks(diff: &str) -> Vec<DiffHunk> {
    diff.lines().filter_map(parse_diff_header).collect()
}

fn parse_diff_header(line: &str) -> Option<DiffHunk> {
    let rest = line.strip_prefix("@@ ")?;
    let end = rest.find(" @@")?;
    let mut parts = rest[..end].split_whitespace();
    let (old_start, old_count) = split_range(parts.next()?.strip_prefix('-')?)?;
    let (new_start, new_count) = split_range(parts.next()?.strip_prefix('+')?)?;
    // An empty range's reported position is already where the lines belong; a
    // non-empty one is 1-based and needs converting.
    let zero = |start: u32, count: u32| if count == 0 { start } else { start - 1 };
    Some(DiffHunk {
        old_start: zero(old_start, old_count),
        old_count,
        new_start: zero(new_start, new_count),
        new_count,
    })
}

/// Pair up the lines of the two sides so they render level with each other.
///
/// Returns one row per display line as `(old_line, new_line)`, 0-based, with
/// `None` where that side has nothing — an added line has no old counterpart and
/// a deleted one has no new counterpart. Unchanged runs pair 1:1, so the two
/// panes stay in step and a hunk that adds more than it removes pushes both
/// sides down together rather than sliding out of alignment.
pub fn align(hunks: &[DiffHunk], old_len: u32, new_len: u32) -> Vec<(Option<u32>, Option<u32>)> {
    let mut rows = Vec::new();
    let (mut o, mut n) = (0u32, 0u32);
    let mut sorted: Vec<DiffHunk> = hunks.to_vec();
    sorted.sort_by_key(|h| (h.new_start, h.old_start));

    for h in sorted {
        // Context before the hunk, which corresponds line for line.
        while o < h.old_start && n < h.new_start {
            rows.push((Some(o), Some(n)));
            o += 1;
            n += 1;
        }
        // The hunk itself: both sides run in parallel, the shorter one padded.
        for i in 0..h.old_count.max(h.new_count) {
            let old = (i < h.old_count).then(|| h.old_start + i);
            let new = (i < h.new_count).then(|| h.new_start + i);
            rows.push((old, new));
        }
        o = h.old_start + h.old_count;
        n = h.new_start + h.new_count;
    }

    // Trailing context, then whatever is left over if the two disagree.
    while o < old_len && n < new_len {
        rows.push((Some(o), Some(n)));
        o += 1;
        n += 1;
    }
    while o < old_len {
        rows.push((Some(o), None));
        o += 1;
    }
    while n < new_len {
        rows.push((None, Some(n)));
        n += 1;
    }
    rows
}

/// The committed contents of `path` at HEAD, or `None` when the file is
/// untracked, HEAD does not exist (an empty repository), or git is unavailable.
pub fn file_at_head(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("show")
        .arg(format!("HEAD:{}", rel.to_str()?.replace('\\', "/")))
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
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
    parse_diff_hunks(diff).into_iter().map(Hunk::from).collect()
}

impl From<DiffHunk> for Hunk {
    /// Reduce a two-sided hunk to the gutter's one-sided view.
    fn from(h: DiffHunk) -> Hunk {
        if h.old_count == 0 {
            Hunk { kind: HunkKind::Added, start: h.new_start, count: h.new_count }
        } else if h.new_count == 0 {
            // A deletion leaves no lines to mark, so the sign goes on the line
            // above the gap. `new_start` is where the removed lines *would* have
            // sat, so stepping back one both lands on the preceding line and
            // keeps a deletion at end-of-file inside the buffer, instead of one
            // line past it where it would never render.
            Hunk { kind: HunkKind::Removed, start: h.new_start.saturating_sub(1), count: 0 }
        } else {
            Hunk { kind: HunkKind::Modified, start: h.new_start, count: h.new_count }
        }
    }
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
    Some(parse_hunks(&raw_diff(root, path)?))
}

/// `git diff --no-color -U0` for one path, or `None` when git is unavailable,
/// the path is untracked, or the command fails — all normal, none an error.
fn raw_diff(root: &Path, path: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["diff", "--no-color", "-U0", "--"])
        .arg(path)
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// [`diff_hunks`] in raw two-sided coordinates, for the side-by-side view.
pub fn diff_hunks_two_sided(root: &Path, path: &Path) -> Option<Vec<DiffHunk>> {
    Some(parse_diff_hunks(&raw_diff(root, path)?))
}

/// What happened to a file, as one half of a `porcelain=v2` `XY` pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Untracked,
    Unmerged,
}

impl FileStatus {
    /// `.` means "nothing on this side", which is `None` rather than a status.
    fn from_code(c: char) -> Option<FileStatus> {
        Some(match c {
            'M' => FileStatus::Modified,
            'A' => FileStatus::Added,
            'D' => FileStatus::Deleted,
            'R' => FileStatus::Renamed,
            'C' => FileStatus::Copied,
            'T' => FileStatus::TypeChanged,
            'U' => FileStatus::Unmerged,
            _ => return None,
        })
    }

    /// The single letter shown in the status list.
    pub fn letter(self) -> char {
        match self {
            FileStatus::Added => 'A',
            FileStatus::Modified => 'M',
            FileStatus::Deleted => 'D',
            FileStatus::Renamed => 'R',
            FileStatus::Copied => 'C',
            FileStatus::TypeChanged => 'T',
            FileStatus::Untracked => '?',
            FileStatus::Unmerged => 'U',
        }
    }
}

/// One file in `git status`.
///
/// `staged` and `unstaged` are independent: a file edited, staged, then edited
/// again is `Some(Modified)` in **both**, and belongs in both sections of the
/// status view. Collapsing them into one status is the classic way to get this
/// wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEntry {
    pub path: PathBuf,
    /// The previous name, for a rename or copy.
    pub orig_path: Option<PathBuf>,
    /// `X` — what is staged for the next commit.
    pub staged: Option<FileStatus>,
    /// `Y` — what is changed in the working tree but not staged.
    pub unstaged: Option<FileStatus>,
}

/// A parsed `git status --porcelain=v2 --branch`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    pub branch: Option<String>,
    pub upstream: Option<String>,
    /// Commits ahead of / behind the upstream.
    pub ahead: u32,
    pub behind: u32,
    pub entries: Vec<StatusEntry>,
}

impl Status {
    /// Files with something staged, in path order.
    pub fn staged(&self) -> Vec<&StatusEntry> {
        self.entries.iter().filter(|e| e.staged.is_some()).collect()
    }

    /// Files with unstaged changes — tracked only; untracked files are their
    /// own section because `git add` means something different for them.
    pub fn unstaged(&self) -> Vec<&StatusEntry> {
        self.entries
            .iter()
            .filter(|e| e.unstaged.is_some_and(|s| s != FileStatus::Untracked))
            .collect()
    }

    pub fn untracked(&self) -> Vec<&StatusEntry> {
        self.entries
            .iter()
            .filter(|e| e.unstaged == Some(FileStatus::Untracked))
            .collect()
    }

    pub fn is_clean(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Parse `git status --porcelain=v2 --branch`.
///
/// Line kinds: `#` header, `1` ordinary change, `2` rename/copy, `?` untracked,
/// `u` unmerged. Unrecognised lines are skipped — git adds header fields over
/// time and an unknown one must not lose the entries around it.
pub fn parse_status(text: &str) -> Status {
    let mut out = Status::default();
    for line in text.lines() {
        let Some((kind, rest)) = line.split_once(' ') else { continue };
        match kind {
            "#" => parse_header(rest, &mut out),
            "1" => {
                if let Some(e) = parse_ordinary(rest) {
                    out.entries.push(e);
                }
            }
            "2" => {
                if let Some(e) = parse_rename(rest) {
                    out.entries.push(e);
                }
            }
            "u" => {
                // An unmerged path is conflicted on both sides at once.
                if let Some(path) = rest.split_whitespace().last() {
                    out.entries.push(StatusEntry {
                        path: PathBuf::from(path),
                        orig_path: None,
                        staged: Some(FileStatus::Unmerged),
                        unstaged: Some(FileStatus::Unmerged),
                    });
                }
            }
            "?" => out.entries.push(StatusEntry {
                path: PathBuf::from(rest),
                orig_path: None,
                staged: None,
                unstaged: Some(FileStatus::Untracked),
            }),
            _ => {}
        }
    }
    out.entries.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn parse_header(rest: &str, out: &mut Status) {
    let Some((key, value)) = rest.split_once(' ') else { return };
    match key {
        "branch.head" if value != "(detached)" => out.branch = Some(value.to_string()),
        "branch.upstream" => out.upstream = Some(value.to_string()),
        "branch.ab" => {
            // `+N -M`
            for part in value.split_whitespace() {
                match part.split_at(1) {
                    ("+", n) => out.ahead = n.parse().unwrap_or(0),
                    ("-", n) => out.behind = n.parse().unwrap_or(0),
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// `1 XY sub mH mI mW hH hI path`
fn parse_ordinary(rest: &str) -> Option<StatusEntry> {
    let mut f = rest.splitn(8, ' ');
    let xy = f.next()?;
    let (x, y) = split_xy(xy)?;
    // Skip sub, mH, mI, mW, hH, hI.
    for _ in 0..6 {
        f.next()?;
    }
    let path = f.next()?;
    Some(StatusEntry {
        path: PathBuf::from(path),
        orig_path: None,
        staged: x,
        unstaged: y,
    })
}

/// `2 XY sub mH mI mW hH hI score path<TAB>origPath`
///
/// The tab matters: a rename's two paths are **tab**-separated, so splitting on
/// whitespace silently mangles every rename (and any path containing a space).
fn parse_rename(rest: &str) -> Option<StatusEntry> {
    let mut f = rest.splitn(9, ' ');
    let (x, y) = split_xy(f.next()?)?;
    for _ in 0..7 {
        f.next()?; // sub, mH, mI, mW, hH, hI, score
    }
    let paths = f.next()?;
    let (new, old) = paths.split_once('\t')?;
    Some(StatusEntry {
        path: PathBuf::from(new),
        orig_path: Some(PathBuf::from(old)),
        staged: x,
        unstaged: y,
    })
}

/// `XY` — X is the **staged** status, Y the **unstaged** one.
fn split_xy(xy: &str) -> Option<(Option<FileStatus>, Option<FileStatus>)> {
    let mut c = xy.chars();
    let x = c.next()?;
    let y = c.next()?;
    Some((FileStatus::from_code(x), FileStatus::from_code(y)))
}

/// `git status --porcelain=v2 --branch` for `root`, or `None` when git is
/// unavailable or this is not a repository.
pub fn status(root: &Path) -> Option<Status> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain=v2", "--branch"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| parse_status(&String::from_utf8_lossy(&out.stdout)))
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

    /// Captured verbatim from a real repository — the rename line's paths are
    /// separated by a **tab**, which is the detail a whitespace-splitting parser
    /// gets wrong.
    const STATUS: &str = "\
# branch.oid 76fd70c99b302a57e5c24987709af4b683e9c72e
# branch.head main
# branch.upstream origin/main
# branch.ab +2 -1
1 A. N... 000000 100644 100644 0000000 3e75765 staged.txt
1 MM N... 100644 100644 100644 814f4a4 05b65e8 tracked.txt
1 .D N... 100644 100644 000000 aaa1111 aaa1111 removed.txt
2 R. N... 100644 100644 100644 7b26523 7b26523 R100 moved.txt\ttracked-old.txt
u UU N... 100644 100644 100644 100644 df967b9 ba2906d e45c9c2 conflict.txt
? untracked.txt
";

    #[test]
    fn the_branch_header_is_parsed() {
        let s = parse_status(STATUS);
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(s.upstream.as_deref(), Some("origin/main"));
        assert_eq!((s.ahead, s.behind), (2, 1));
    }

    /// `XY` is staged-then-unstaged. Getting this backwards makes both sections
    /// of the status view lie, and only shows up once someone stages half a file.
    #[test]
    fn xy_splits_into_staged_and_unstaged() {
        let s = parse_status(STATUS);
        let by = |n: &str| s.entries.iter().find(|e| e.path.ends_with(n)).unwrap().clone();

        // `A.` — staged addition, nothing unstaged.
        let a = by("staged.txt");
        assert_eq!((a.staged, a.unstaged), (Some(FileStatus::Added), None));

        // `MM` — modified, staged, then modified again. In *both* sections.
        let m = by("tracked.txt");
        assert_eq!(m.staged, Some(FileStatus::Modified));
        assert_eq!(m.unstaged, Some(FileStatus::Modified));

        // `.D` — deleted in the working tree, nothing staged.
        let d = by("removed.txt");
        assert_eq!((d.staged, d.unstaged), (None, Some(FileStatus::Deleted)));
    }

    #[test]
    fn a_file_modified_and_restaged_appears_in_both_sections() {
        let s = parse_status(STATUS);
        assert!(s.staged().iter().any(|e| e.path.ends_with("tracked.txt")));
        assert!(s.unstaged().iter().any(|e| e.path.ends_with("tracked.txt")));
    }

    /// A rename's two paths are tab-separated; splitting on spaces would take
    /// the whole `new<TAB>old` blob as one path.
    #[test]
    fn a_rename_keeps_both_paths() {
        let s = parse_status(STATUS);
        let r = s.entries.iter().find(|e| e.staged == Some(FileStatus::Renamed)).expect("a rename");
        assert_eq!(r.path, PathBuf::from("moved.txt"));
        assert_eq!(r.orig_path, Some(PathBuf::from("tracked-old.txt")));
    }

    #[test]
    fn untracked_and_unmerged_are_their_own_kinds() {
        let s = parse_status(STATUS);
        let u = s.untracked();
        assert_eq!(u.len(), 1);
        assert_eq!(u[0].path, PathBuf::from("untracked.txt"));
        // An untracked file is not "unstaged" — `git add` means something
        // different for it, so it gets its own section.
        assert!(s.unstaged().iter().all(|e| !e.path.ends_with("untracked.txt")));

        let c = s.entries.iter().find(|e| e.path.ends_with("conflict.txt")).unwrap();
        assert_eq!(c.staged, Some(FileStatus::Unmerged));
        assert_eq!(c.unstaged, Some(FileStatus::Unmerged));
    }

    #[test]
    fn entries_are_sorted_by_path_so_the_list_is_stable() {
        let s = parse_status(STATUS);
        let paths: Vec<_> = s.entries.iter().map(|e| e.path.clone()).collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted);
    }

    #[test]
    fn a_clean_repository_has_no_entries() {
        let s = parse_status("# branch.oid abc\n# branch.head main\n");
        assert!(s.is_clean());
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert!(s.staged().is_empty() && s.unstaged().is_empty() && s.untracked().is_empty());
    }

    /// git grows new header fields over time; an unknown one must not take the
    /// entries with it.
    #[test]
    fn unknown_lines_are_skipped_not_fatal() {
        let s = parse_status(
            "# branch.head main\n# some.future.field whatever\nx nonsense\n1 M. N... 1 1 1 a b keep.txt\n",
        );
        assert_eq!(s.entries.len(), 1);
        assert_eq!(s.entries[0].path, PathBuf::from("keep.txt"));
        assert_eq!(s.branch.as_deref(), Some("main"));
    }

    #[test]
    fn a_detached_head_reports_no_branch() {
        assert_eq!(parse_status("# branch.head (detached)\n").branch, None);
    }

    #[test]
    fn empty_output_is_a_clean_status() {
        assert_eq!(parse_status(""), Status::default());
    }

    #[test]
    fn navigation_on_an_unchanged_file_finds_nothing() {
        assert!(next_hunk(&[], 3).is_none());
        assert!(prev_hunk(&[], 3).is_none());
    }

    #[test]
    fn diff_hunks_keep_both_sides_unadjusted() {
        let h = parse_diff_hunks(SAMPLE);
        assert_eq!(h.len(), 3);
        // `@@ -12 +12 @@` — one line replaced one, both 1-based.
        assert_eq!(h[0], DiffHunk { old_start: 11, old_count: 1, new_start: 11, new_count: 1 });
        // `@@ -20,0 +21,3 @@` — the old side is empty, so its position is as
        // reported rather than one less.
        assert_eq!(h[1], DiffHunk { old_start: 20, old_count: 0, new_start: 20, new_count: 3 });
        // `@@ -30,2 +32,0 @@` — and the new side is the empty one here.
        assert_eq!(h[2], DiffHunk { old_start: 29, old_count: 2, new_start: 32, new_count: 0 });
    }

    /// An unchanged file is one row per line, both sides in step.
    #[test]
    fn align_pairs_unchanged_lines_one_to_one() {
        let rows = align(&[], 3, 3);
        assert_eq!(rows, [(Some(0), Some(0)), (Some(1), Some(1)), (Some(2), Some(2))]);
    }

    /// The case the panes have to survive: a hunk that removes two lines and
    /// adds five. The short side pads, and — the point of the whole exercise —
    /// the context *after* the hunk is still level on both sides.
    #[test]
    fn align_pads_the_short_side_of_an_unbalanced_hunk() {
        // 6-line old file, 9-line new file: lines 2-3 became lines 2-6.
        let h = DiffHunk { old_start: 2, old_count: 2, new_start: 2, new_count: 5 };
        let rows = align(&[h], 6, 9);

        assert_eq!(&rows[..2], &[(Some(0), Some(0)), (Some(1), Some(1))], "context before");
        assert_eq!(
            &rows[2..7],
            &[
                (Some(2), Some(2)),
                (Some(3), Some(3)),
                (None, Some(4)),
                (None, Some(5)),
                (None, Some(6)),
            ],
            "the old side runs out and pads"
        );
        assert_eq!(
            &rows[7..],
            &[(Some(4), Some(7)), (Some(5), Some(8))],
            "context after is level again despite the offset"
        );
    }

    #[test]
    fn align_handles_a_pure_addition_and_a_pure_deletion() {
        // Three lines added after the first: the old side pads.
        let add = DiffHunk { old_start: 1, old_count: 0, new_start: 1, new_count: 3 };
        let rows = align(&[add], 1, 4);
        assert_eq!(rows[0], (Some(0), Some(0)));
        assert_eq!(&rows[1..], &[(None, Some(1)), (None, Some(2)), (None, Some(3))]);

        // Two lines deleted: the new side pads instead.
        let del = DiffHunk { old_start: 1, old_count: 2, new_start: 1, new_count: 0 };
        let rows = align(&[del], 3, 1);
        assert_eq!(rows, [(Some(0), Some(0)), (Some(1), None), (Some(2), None)]);
    }

    /// Every line of both files must appear exactly once, whatever the hunks —
    /// otherwise a pane silently drops content.
    #[test]
    fn align_never_loses_or_repeats_a_line() {
        let hunks = [
            DiffHunk { old_start: 1, old_count: 2, new_start: 1, new_count: 1 },
            DiffHunk { old_start: 6, old_count: 0, new_start: 5, new_count: 3 },
        ];
        let (old_len, new_len) = (9, 11);
        let rows = align(&hunks, old_len, new_len);
        let olds: Vec<u32> = rows.iter().filter_map(|r| r.0).collect();
        let news: Vec<u32> = rows.iter().filter_map(|r| r.1).collect();
        assert_eq!(olds, (0..old_len).collect::<Vec<_>>(), "old side complete and in order");
        assert_eq!(news, (0..new_len).collect::<Vec<_>>(), "new side complete and in order");
    }

    #[test]
    fn align_handles_empty_files() {
        assert!(align(&[], 0, 0).is_empty());
        assert_eq!(align(&[], 0, 2), [(None, Some(0)), (None, Some(1))], "a new file");
        assert_eq!(align(&[], 2, 0), [(Some(0), None), (Some(1), None)], "a deleted file");
    }

    /// Hunks arrive in order from git, but alignment must not depend on it.
    #[test]
    fn align_sorts_hunks_before_walking_them() {
        let a = DiffHunk { old_start: 0, old_count: 1, new_start: 0, new_count: 2 };
        let b = DiffHunk { old_start: 4, old_count: 1, new_start: 5, new_count: 1 };
        assert_eq!(align(&[a, b], 6, 7), align(&[b, a], 6, 7));
    }
}
