//! Minimal directory listing model backing the dired file-explorer buffer.

use std::path::{Path, PathBuf};

/// One entry in a directory listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    /// Has an executable permission bit (always false off unix).
    pub is_exec: bool,
    pub is_symlink: bool,
}

impl DirEntry {
    fn dir(name: &str) -> Self {
        DirEntry {
            name: name.to_string(),
            is_dir: true,
            is_exec: false,
            is_symlink: false,
        }
    }
}

#[cfg(unix)]
fn executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn executable(_meta: &std::fs::Metadata) -> bool {
    false
}

/// The sentinel path for the "drives view" — the top-level listing of drive
/// roots (Windows `C:\`, `D:\`, ...). Represented as an empty path so it
/// round-trips through the existing path-keyed dired state. Above this there is
/// nowhere to go up to.
pub fn drives_view() -> PathBuf {
    PathBuf::new()
}

/// Whether `path` is the [`drives_view`] sentinel.
pub fn is_drives_view(path: &Path) -> bool {
    path.as_os_str().is_empty()
}

/// The available drive roots as directory entries (e.g. `C:\`, `D:\`). Windows
/// only; empty elsewhere, where there is a single `/` root instead.
#[cfg(windows)]
pub fn drive_roots() -> Vec<DirEntry> {
    (b'A'..=b'Z')
        .map(|l| format!("{}:\\", l as char))
        .filter(|root| Path::new(root).exists())
        .map(|name| DirEntry::dir(&name))
        .collect()
}

#[cfg(not(windows))]
pub fn drive_roots() -> Vec<DirEntry> {
    Vec::new()
}

/// List `path`: `..` first (unless `path` is a filesystem root), then
/// directories, then files, each group sorted alphabetically. Dot-files are
/// included only when `show_hidden` is true (`..` always shows). For the
/// [`drives_view`] sentinel, lists the drive roots with no `..`.
pub fn list(path: &Path, show_hidden: bool) -> Vec<DirEntry> {
    if is_drives_view(path) {
        return drive_roots();
    }

    let mut dirs: Vec<DirEntry> = Vec::new();
    let mut files: Vec<DirEntry> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(path) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !show_hidden && name.starts_with('.') {
                continue;
            }
            let file_type = entry.file_type().ok();
            let is_symlink = file_type.map(|t| t.is_symlink()).unwrap_or(false);
            // Resolve through symlinks so a link to a directory sorts as one.
            let is_dir = entry.path().is_dir();
            let is_exec = entry
                .metadata()
                .ok()
                .map(|m| !m.is_dir() && executable(&m))
                .unwrap_or(false);
            let item = DirEntry {
                name,
                is_dir,
                is_exec,
                is_symlink,
            };
            if is_dir {
                dirs.push(item);
            } else {
                files.push(item);
            }
        }
    }
    dirs.sort_by(|a, b| a.name.cmp(&b.name));
    files.sort_by(|a, b| a.name.cmp(&b.name));

    let mut out = Vec::new();
    // Offer `..` when there's a parent directory, and on Windows also at a drive
    // root (`C:\`, whose `parent()` is `None`) so the user can ascend to the
    // drive picker.
    if path.parent().is_some() || cfg!(windows) {
        out.push(DirEntry::dir(".."));
    }
    out.extend(dirs);
    out.extend(files);
    out
}

/// Render already-listed entries as buffer text, one per line. Directories are
/// shown with a trailing `/` (unless the name already ends in a path separator,
/// e.g. a drive root); line index matches the entry index.
pub fn render_entries(entries: &[DirEntry]) -> String {
    entries
        .iter()
        .map(|e| {
            if e.is_dir && !e.name.ends_with(['/', '\\']) {
                format!("{}/", e.name)
            } else {
                e.name.clone()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// List and render `path` in one step.
pub fn render(path: &Path, show_hidden: bool) -> String {
    render_entries(&list(path, show_hidden))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirs_first_then_files_with_dotdot() {
        let tmp = std::env::temp_dir().join("ruster_dired_test_1");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("subdir")).unwrap();
        std::fs::write(tmp.join("zfile.txt"), "y").unwrap();
        std::fs::write(tmp.join("afile.txt"), "x").unwrap();

        let entries = list(&tmp, true);
        assert_eq!(entries[0].name, "..");
        assert!(entries[0].is_dir);
        assert_eq!(entries[1].name, "subdir");
        assert!(entries[1].is_dir);
        assert_eq!(entries[2].name, "afile.txt");
        assert_eq!(entries[3].name, "zfile.txt");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(unix)]
    #[test]
    fn hidden_files_are_filtered_unless_requested() {
        let tmp = std::env::temp_dir().join("ruster_dired_hidden");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join(".git")).unwrap();
        std::fs::write(tmp.join(".env"), "x").unwrap();
        std::fs::write(tmp.join("visible.txt"), "y").unwrap();

        let shown = list(&tmp, false);
        assert!(shown.iter().any(|e| e.name == "visible.txt"));
        assert!(!shown.iter().any(|e| e.name == ".env"));
        assert!(!shown.iter().any(|e| e.name == ".git"));
        // ".." is navigation, not a hidden entry.
        assert!(shown.iter().any(|e| e.name == ".."));

        let all = list(&tmp, true);
        assert!(all.iter().any(|e| e.name == ".env"));
        assert!(all.iter().any(|e| e.name == ".git"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // `/` is a Unix filesystem root. On Windows `/` is the root of the current
    // drive and we intentionally offer `..` there to reach the drive picker, so
    // this invariant is Unix-only.
    #[cfg(unix)]
    #[test]
    fn root_omits_dotdot() {
        let entries = list(Path::new("/"), true);
        assert!(!entries.iter().any(|e| e.name == ".."));
    }

    #[test]
    fn drives_view_sentinel_is_detected() {
        assert!(is_drives_view(&drives_view()));
        assert!(!is_drives_view(Path::new("/")));
    }

    #[test]
    fn drives_view_has_no_dotdot() {
        // The drives view is the top; nowhere to go up to.
        assert!(!list(&drives_view(), true).iter().any(|e| e.name == ".."));
    }

    #[cfg(windows)]
    #[test]
    fn drives_view_lists_drive_roots() {
        let entries = list(&drives_view(), true);
        assert!(entries.iter().all(|e| e.is_dir));
        // The test host always has a C: drive.
        assert!(entries.iter().any(|e| e.name.starts_with("C:")));
    }

    #[cfg(windows)]
    #[test]
    fn drive_root_offers_dotdot_to_reach_drives() {
        let entries = list(Path::new("C:\\"), true);
        assert_eq!(
            entries[0].name, "..",
            "drive root can ascend to the drive picker"
        );
    }

    #[test]
    fn render_marks_directories_with_slash() {
        let tmp = std::env::temp_dir().join("ruster_dired_test_2");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("d")).unwrap();
        std::fs::write(tmp.join("f"), "x").unwrap();
        let text = render(&tmp, true);
        // ".." then "d/" then "f"
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "../");
        assert_eq!(lines[1], "d/");
        assert_eq!(lines[2], "f");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
