//! Minimal directory listing model backing the dired file-explorer buffer.

use std::path::{Path, PathBuf};

/// One entry in a directory listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
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
        .map(|name| DirEntry { name, is_dir: true })
        .collect()
}

#[cfg(not(windows))]
pub fn drive_roots() -> Vec<DirEntry> {
    Vec::new()
}

/// List `path`: `..` first (unless `path` is a top-level root), then
/// directories, then files, each group sorted alphabetically. For the
/// [`drives_view`] sentinel, lists the drive roots with no `..`.
pub fn list(path: &Path) -> Vec<DirEntry> {
    if is_drives_view(path) {
        return drive_roots();
    }

    let mut dirs: Vec<DirEntry> = Vec::new();
    let mut files: Vec<DirEntry> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(path) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                dirs.push(DirEntry { name, is_dir: true });
            } else {
                files.push(DirEntry { name, is_dir: false });
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
        out.push(DirEntry { name: "..".to_string(), is_dir: true });
    }
    out.extend(dirs);
    out.extend(files);
    out
}

/// Render a directory listing as buffer text, one entry per line. Directories
/// are shown with a trailing `/` (unless the name already ends in a path
/// separator, e.g. a drive root). The line index of each entry matches the
/// index returned by [`list`].
pub fn render(path: &Path) -> String {
    list(path)
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

        let entries = list(&tmp);
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
    fn root_omits_dotdot() {
        let entries = list(Path::new("/"));
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
        assert!(!list(&drives_view()).iter().any(|e| e.name == ".."));
    }

    #[cfg(windows)]
    #[test]
    fn drives_view_lists_drive_roots() {
        let entries = list(&drives_view());
        assert!(entries.iter().all(|e| e.is_dir));
        // The test host always has a C: drive.
        assert!(entries.iter().any(|e| e.name.starts_with("C:")));
    }

    #[cfg(windows)]
    #[test]
    fn drive_root_offers_dotdot_to_reach_drives() {
        let entries = list(Path::new("C:\\"));
        assert_eq!(entries[0].name, "..", "drive root can ascend to the drive picker");
    }

    #[test]
    fn render_marks_directories_with_slash() {
        let tmp = std::env::temp_dir().join("ruster_dired_test_2");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("d")).unwrap();
        std::fs::write(tmp.join("f"), "x").unwrap();
        let text = render(&tmp);
        // ".." then "d/" then "f"
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "../");
        assert_eq!(lines[1], "d/");
        assert_eq!(lines[2], "f");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
