//! Saving and restoring what you had open (Phase 7 Task 1).
//!
//! A session is the set of files open in a project, the shape of the window
//! layout, and where the cursor sat in each. Nothing else: no buffer contents
//! (they are on disk), no terminals (their shells are gone), and no unsaved
//! scratch (there is nowhere to put it).
//!
//! # Why a hand-written format
//!
//! `ruster-core` has no serde dependency and this does not justify adding one to
//! the crate every other crate depends on. The format is line-based like
//! `recent-projects`, and the layout tree is written in **preorder** — a split
//! line is followed by its two subtrees — which is unambiguous without brackets
//! and parses with a single cursor over the lines.
//!
//! ```text
//! ruster-session 1
//! buf /home/me/proj/src/main.rs
//! buf /home/me/proj/README.md
//! split v 0.50
//! leaf 0 431 12 1
//! leaf 1 0 0 0
//! ```
//!
//! Unknown leading keywords and malformed lines make the whole file invalid
//! rather than silently restoring half a layout — a wrong layout is worse than
//! none, and the caller opens an empty editor instead.

use std::path::{Path, PathBuf};

use crate::windows::{LayoutSnapshot, SplitDir};

/// The format version written into the header. Bumped only on a breaking
/// change; an older or newer file is refused rather than guessed at.
const VERSION: u32 = 1;
const MAGIC: &str = "ruster-session";

/// One saved session.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    /// Absolute paths of the files that were open, in the order leaves refer to.
    pub files: Vec<PathBuf>,
    pub layout: LayoutSnapshot,
}

impl Session {
    pub fn encode(&self) -> String {
        let mut out = format!("{MAGIC} {VERSION}\n");
        for f in &self.files {
            out.push_str(&format!("buf {}\n", f.display()));
        }
        write_node(&self.layout, &mut out);
        out
    }

    /// Parse a session file, or `None` if it is not one we understand.
    pub fn decode(text: &str) -> Option<Session> {
        let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
        let header = lines.next()?;
        let (magic, version) = header.split_once(' ')?;
        if magic != MAGIC || version.parse::<u32>().ok()? != VERSION {
            return None;
        }

        let rest: Vec<&str> = lines.collect();
        let files: Vec<PathBuf> = rest
            .iter()
            .filter_map(|l| l.strip_prefix("buf "))
            .map(PathBuf::from)
            .collect();
        let mut nodes = rest.iter().copied().skip_while(|l| l.starts_with("buf "));
        let layout = read_node(&mut nodes)?;
        // Trailing junk means the file is not what we think it is.
        if nodes.next().is_some() {
            return None;
        }
        // A leaf pointing past the end of the file list is corrupt.
        if !leaves_in_range(&layout, files.len()) {
            return None;
        }
        Some(Session { files, layout })
    }
}

fn write_node(node: &LayoutSnapshot, out: &mut String) {
    match node {
        LayoutSnapshot::Leaf {
            buffer,
            cursor,
            scroll,
            active,
        } => {
            out.push_str(&format!(
                "leaf {buffer} {cursor} {scroll} {}\n",
                u8::from(*active)
            ));
        }
        LayoutSnapshot::Split {
            dir,
            ratio,
            first,
            second,
        } => {
            let d = if *dir == SplitDir::Vertical { 'v' } else { 'h' };
            out.push_str(&format!("split {d} {ratio:.4}\n"));
            write_node(first, out);
            write_node(second, out);
        }
    }
}

fn read_node<'a>(lines: &mut impl Iterator<Item = &'a str>) -> Option<LayoutSnapshot> {
    let line = lines.next()?;
    let mut f = line.split_whitespace();
    match f.next()? {
        "leaf" => Some(LayoutSnapshot::Leaf {
            buffer: f.next()?.parse().ok()?,
            cursor: f.next()?.parse().ok()?,
            scroll: f.next()?.parse().ok()?,
            active: f.next()? == "1",
        }),
        "split" => {
            let dir = match f.next()? {
                "v" => SplitDir::Vertical,
                "h" => SplitDir::Horizontal,
                _ => return None,
            };
            let ratio: f32 = f.next()?.parse().ok()?;
            if !(0.0..=1.0).contains(&ratio) {
                return None;
            }
            // Preorder: both children follow immediately.
            let first = Box::new(read_node(lines)?);
            let second = Box::new(read_node(lines)?);
            Some(LayoutSnapshot::Split {
                dir,
                ratio,
                first,
                second,
            })
        }
        _ => None,
    }
}

fn leaves_in_range(node: &LayoutSnapshot, len: usize) -> bool {
    match node {
        LayoutSnapshot::Leaf { buffer, .. } => *buffer < len,
        LayoutSnapshot::Split { first, second, .. } => {
            leaves_in_range(first, len) && leaves_in_range(second, len)
        }
    }
}

/// Where a project's session lives: one file per project root, named by a hash
/// of the root so two projects never collide and the name stays a valid
/// filename whatever the path contains.
pub fn session_path(state_dir: &Path, root: &Path) -> PathBuf {
    // FNV-1a: stable across runs and platforms, which `DefaultHasher` is not
    // guaranteed to be — a session must still be found tomorrow.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in root.to_string_lossy().as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    state_dir
        .join("sessions")
        .join(format!("{hash:016x}.session"))
}

/// Write a session, creating the directory if needed. Errors are returned so the
/// caller can report them; losing a session is not worth crashing over.
pub fn save(state_dir: &Path, root: &Path, session: &Session) -> std::io::Result<()> {
    let path = session_path(state_dir, root);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, session.encode())
}

/// Read a project's session, or `None` when there is none or it is unreadable.
pub fn load(state_dir: &Path, root: &Path) -> Option<Session> {
    Session::decode(&std::fs::read_to_string(session_path(state_dir, root)).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(buffer: usize, active: bool) -> LayoutSnapshot {
        LayoutSnapshot::Leaf {
            buffer,
            cursor: buffer * 10,
            scroll: buffer,
            active,
        }
    }

    fn nested() -> Session {
        Session {
            files: vec!["/p/a.rs".into(), "/p/b.rs".into(), "/p/c.md".into()],
            layout: LayoutSnapshot::Split {
                dir: SplitDir::Vertical,
                ratio: 0.5,
                first: Box::new(leaf(0, false)),
                second: Box::new(LayoutSnapshot::Split {
                    dir: SplitDir::Horizontal,
                    ratio: 0.25,
                    first: Box::new(leaf(1, true)),
                    second: Box::new(leaf(2, false)),
                }),
            },
        }
    }

    #[test]
    fn a_nested_layout_round_trips() {
        let s = nested();
        assert_eq!(Session::decode(&s.encode()), Some(s));
    }

    #[test]
    fn a_single_window_round_trips() {
        let s = Session {
            files: vec!["/p/only.rs".into()],
            layout: leaf(0, true),
        };
        assert_eq!(Session::decode(&s.encode()), Some(s));
    }

    /// Preorder has to be unambiguous: two different shapes over the same leaves
    /// must not encode identically.
    #[test]
    fn different_shapes_encode_differently() {
        let left = LayoutSnapshot::Split {
            dir: SplitDir::Vertical,
            ratio: 0.5,
            first: Box::new(LayoutSnapshot::Split {
                dir: SplitDir::Vertical,
                ratio: 0.5,
                first: Box::new(leaf(0, true)),
                second: Box::new(leaf(1, false)),
            }),
            second: Box::new(leaf(2, false)),
        };
        let right = nested().layout; // the same three leaves, nested the other way
        assert_ne!(left, right);
        let files = vec!["/a".into(), "/b".into(), "/c".into()];
        let a = Session {
            files: files.clone(),
            layout: left,
        }
        .encode();
        let b = Session {
            files,
            layout: right,
        }
        .encode();
        assert_ne!(a, b);
    }

    #[test]
    fn a_file_that_is_not_a_session_is_refused() {
        assert_eq!(Session::decode(""), None);
        assert_eq!(Session::decode("hello\nworld\n"), None);
        assert_eq!(
            Session::decode("ruster-session 99\nbuf /a\nleaf 0 0 0 1\n"),
            None,
            "version"
        );
        assert_eq!(Session::decode("ruster-session\n"), None, "no version");
    }

    /// Half a layout is worse than none — it would silently reopen the wrong
    /// windows.
    #[test]
    fn a_truncated_or_corrupt_layout_is_refused() {
        // A split promising two children but supplying one.
        assert_eq!(
            Session::decode("ruster-session 1\nbuf /a\nsplit v 0.5\nleaf 0 0 0 1\n"),
            None
        );
        // A leaf pointing past the end of the file list.
        assert_eq!(
            Session::decode("ruster-session 1\nbuf /a\nleaf 7 0 0 1\n"),
            None
        );
        // Junk after a complete tree.
        assert_eq!(
            Session::decode("ruster-session 1\nbuf /a\nleaf 0 0 0 1\nwat\n"),
            None
        );
        // An unknown node keyword.
        assert_eq!(Session::decode("ruster-session 1\nbuf /a\nfrob 0\n"), None);
        // A nonsense ratio.
        assert_eq!(
            Session::decode(
                "ruster-session 1\nbuf /a\nbuf /b\nsplit v 9.9\nleaf 0 0 0 1\nleaf 1 0 0 0\n"
            ),
            None
        );
    }

    #[test]
    fn a_session_with_no_files_is_refused() {
        // Every leaf must point at something.
        assert_eq!(Session::decode("ruster-session 1\nleaf 0 0 0 1\n"), None);
    }

    #[test]
    fn paths_with_spaces_survive() {
        let s = Session {
            files: vec!["/p/my documents/a b.rs".into()],
            layout: leaf(0, true),
        };
        assert_eq!(Session::decode(&s.encode()), Some(s));
    }

    #[test]
    fn each_project_gets_its_own_file_and_the_name_is_stable() {
        let dir = Path::new("/state");
        let a = session_path(dir, Path::new("/home/me/one"));
        let b = session_path(dir, Path::new("/home/me/two"));
        assert_ne!(a, b, "different projects do not collide");
        assert_eq!(
            a,
            session_path(dir, Path::new("/home/me/one")),
            "and the name is stable"
        );
        assert!(a.to_string_lossy().ends_with(".session"));
        assert!(a.starts_with("/state/sessions"));
    }

    #[test]
    fn saving_then_loading_returns_the_same_session() {
        let dir = std::env::temp_dir().join("ruster_session_rt");
        let _ = std::fs::remove_dir_all(&dir);
        let root = Path::new("/proj/x");
        assert_eq!(load(&dir, root), None, "nothing saved yet");
        let s = nested();
        save(&dir, root, &s).unwrap();
        assert_eq!(load(&dir, root), Some(s));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
