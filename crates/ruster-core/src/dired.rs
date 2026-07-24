//! Minimal directory listing model backing the dired file-explorer buffer.

use std::path::Path;

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
        DirEntry { name: name.to_string(), is_dir: true, is_exec: false, is_symlink: false }
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

/// List `path`: `..` first (unless `path` is a filesystem root), then
/// directories, then files, each group sorted alphabetically.
pub fn list(path: &Path) -> Vec<DirEntry> {
    let mut dirs: Vec<DirEntry> = Vec::new();
    let mut files: Vec<DirEntry> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(path) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let file_type = entry.file_type().ok();
            let is_symlink = file_type.map(|t| t.is_symlink()).unwrap_or(false);
            // Resolve through symlinks so a link to a directory sorts as one.
            let is_dir = entry.path().is_dir();
            let is_exec = entry
                .metadata()
                .ok()
                .map(|m| !m.is_dir() && executable(&m))
                .unwrap_or(false);
            let item = DirEntry { name, is_dir, is_exec, is_symlink };
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
    if path.parent().is_some() {
        out.push(DirEntry::dir(".."));
    }
    out.extend(dirs);
    out.extend(files);
    out
}

/// Render a directory listing as buffer text, one entry per line. Directories
/// are shown with a trailing `/`. The line index of each entry matches the
/// index returned by [`list`].
pub fn render(path: &Path) -> String {
    list(path)
        .iter()
        .map(|e| if e.is_dir { format!("{}/", e.name) } else { e.name.clone() })
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

    #[test]
    fn root_omits_dotdot() {
        let entries = list(Path::new("/"));
        assert!(!entries.iter().any(|e| e.name == ".."));
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
