//! Minimal directory listing model backing the dired file-explorer buffer.

use std::path::Path;

/// One entry in a directory listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

/// List `path`: `..` first (unless `path` is a filesystem root), then
/// directories, then files, each group sorted alphabetically.
pub fn list(path: &Path) -> Vec<DirEntry> {
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
    if path.parent().is_some() {
        out.push(DirEntry { name: "..".to_string(), is_dir: true });
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
