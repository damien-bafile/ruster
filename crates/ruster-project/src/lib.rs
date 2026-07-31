//! Project awareness for ruster (Phase 5): find the project root from marker
//! files, read an optional `ruster.toml` (tasks + build/test commands), and
//! track recently-opened projects. Pure filesystem/parsing — no UI.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Files that mark a project root, most-specific first.
pub const ROOT_MARKERS: &[&str] =
    &["ruster.toml", ".git", "Cargo.toml", "package.json", "go.mod", "Makefile", "pyproject.toml"];

/// Walk up from `from` (a file or directory) to the nearest ancestor containing
/// a [`ROOT_MARKERS`] entry. Returns `None` if none is found up to the filesystem
/// root (callers typically fall back to the current directory).
///
/// `from` is resolved against the current directory first. A bare filename has
/// an empty parent, and `Path::new("").join(marker)` quietly resolves against
/// the cwd — so walking a relative path directly would "find" a root and return
/// an empty path, which every caller then treats as a real directory.
pub fn project_root(from: &Path) -> Option<PathBuf> {
    let abs = absolutize(from)?;
    let mut dir: &Path = if abs.is_dir() { abs.as_path() } else { abs.parent()? };
    loop {
        if ROOT_MARKERS.iter().any(|m| dir.join(m).exists()) {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}

/// Make `path` absolute against the current directory, without requiring it to
/// exist (so a not-yet-created file still resolves to the right parent).
fn absolutize(path: &Path) -> Option<PathBuf> {
    if path.is_absolute() {
        Some(path.to_path_buf())
    } else {
        Some(std::env::current_dir().ok()?.join(path))
    }
}

/// A build/test command override from `ruster.toml`.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct CommandSpec {
    pub command: Option<String>,
}

/// A user-defined task (`[tasks.<name>]`).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Task {
    /// The shell command to run.
    pub cmd: String,
    /// Working directory (relative to the project root); defaults to the root.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Run in the embedded terminal (`true`, default) vs. a background thread.
    #[serde(default = "default_true")]
    pub use_terminal: bool,
}

fn default_true() -> bool {
    true
}

/// The parsed `ruster.toml` for a project.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct ProjectConfig {
    #[serde(default)]
    pub build: CommandSpec,
    #[serde(default)]
    pub test: CommandSpec,
    /// Named tasks, kept sorted for a stable picker order.
    #[serde(default)]
    pub tasks: BTreeMap<String, Task>,
}

impl ProjectConfig {
    /// Parse `ruster.toml` text; `Err` carries a human-readable message.
    pub fn parse(toml_str: &str) -> Result<Self, String> {
        toml::from_str(toml_str).map_err(|e| e.to_string())
    }

    /// Load `<root>/ruster.toml`, or the defaults if it's absent/invalid.
    pub fn load(root: &Path) -> Self {
        std::fs::read_to_string(root.join("ruster.toml"))
            .ok()
            .and_then(|s| Self::parse(&s).ok())
            .unwrap_or_default()
    }

    /// The build command for a project type, honoring an override.
    pub fn build_command(&self, root: &Path) -> String {
        self.build.command.clone().unwrap_or_else(|| default_build_command(root))
    }

    /// The test command for a project type, honoring an override.
    pub fn test_command(&self, root: &Path) -> String {
        self.test.command.clone().unwrap_or_else(|| default_test_command(root))
    }
}

/// A sensible default build command based on the project's marker files.
pub fn default_build_command(root: &Path) -> String {
    if root.join("Cargo.toml").exists() {
        "cargo build".into()
    } else if root.join("package.json").exists() {
        "npm run build".into()
    } else if root.join("go.mod").exists() {
        "go build ./...".into()
    } else if root.join("Makefile").exists() {
        "make".into()
    } else {
        String::new()
    }
}

/// A sensible default test command based on the project's marker files.
pub fn default_test_command(root: &Path) -> String {
    if root.join("Cargo.toml").exists() {
        "cargo test".into()
    } else if root.join("package.json").exists() {
        "npm test".into()
    } else if root.join("go.mod").exists() {
        "go test ./...".into()
    } else if root.join("Makefile").exists() {
        "make test".into()
    } else {
        String::new()
    }
}

/// Record `root` as the most-recently-opened project in
/// `<state_dir>/recent-projects` (newest first, de-duplicated, capped at `max`).
pub fn record_recent(state_dir: &Path, root: &Path, max: usize) {
    let mut list = recent_projects(state_dir);
    let root = root.to_path_buf();
    list.retain(|p| p != &root);
    list.insert(0, root);
    list.truncate(max);
    let body: String = list.iter().map(|p| format!("{}\n", p.display())).collect();
    let _ = std::fs::create_dir_all(state_dir);
    let _ = std::fs::write(state_dir.join("recent-projects"), body);
}

/// The recently-opened project roots (newest first) that still exist.
pub fn recent_projects(state_dir: &Path) -> Vec<PathBuf> {
    std::fs::read_to_string(state_dir.join("recent-projects"))
        .map(|s| {
            s.lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .map(PathBuf::from)
                .filter(|p| p.exists())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_root_by_marker() {
        let tmp = std::env::temp_dir().join(format!("ruster_proj_{}", std::process::id()));
        let nested = tmp.join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(tmp.join("Cargo.toml"), "[package]\n").unwrap();

        let root = project_root(&nested).unwrap();
        assert_eq!(root.canonicalize().unwrap(), tmp.canonicalize().unwrap());
        // Passing a file resolves via its parent.
        let file = nested.join("main.rs");
        std::fs::write(&file, "").unwrap();
        assert_eq!(
            project_root(&file).unwrap().canonicalize().unwrap(),
            tmp.canonicalize().unwrap()
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    /// A bare filename has an empty parent, and `Path::new("").join(marker)`
    /// resolves against the cwd — so this used to "succeed" with `Some("")`.
    /// Callers treated that as a real directory, which left the sidebar empty
    /// and pointed `:Rg`, the pickers and tasks at nothing.
    #[test]
    fn a_relative_path_resolves_to_an_absolute_root() {
        // Tests run with the cwd set to the crate root, which has a Cargo.toml.
        let cwd = std::env::current_dir().unwrap();
        let root = project_root(Path::new("Cargo.toml")).expect("cwd is a project root");
        assert!(root.is_absolute(), "root must be absolute, got {root:?}");
        assert!(!root.as_os_str().is_empty(), "root must not be the empty path");
        assert_eq!(root.canonicalize().unwrap(), cwd.canonicalize().unwrap());
    }

    #[test]
    fn parses_ruster_toml() {
        let cfg = ProjectConfig::parse(
            r#"
            [build]
            command = "cargo build --release"

            [tasks.lint]
            cmd = "cargo clippy"

            [tasks.deploy]
            cmd = "./deploy.sh"
            cwd = "scripts"
            use_terminal = false
        "#,
        )
        .expect("parse");
        assert_eq!(cfg.build.command.as_deref(), Some("cargo build --release"));
        assert!(cfg.test.command.is_none());
        assert_eq!(cfg.tasks.len(), 2);
        let lint = &cfg.tasks["lint"];
        assert_eq!(lint.cmd, "cargo clippy");
        assert!(lint.use_terminal, "use_terminal defaults to true");
        let deploy = &cfg.tasks["deploy"];
        assert_eq!(deploy.cwd.as_deref(), Some("scripts"));
        assert!(!deploy.use_terminal);
    }

    #[test]
    fn default_commands_by_project_type() {
        let tmp = std::env::temp_dir().join(format!("ruster_cmd_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("Cargo.toml"), "").unwrap();
        let cfg = ProjectConfig::default();
        assert_eq!(cfg.build_command(&tmp), "cargo build");
        assert_eq!(cfg.test_command(&tmp), "cargo test");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn recent_projects_dedup_and_cap() {
        let tmp = std::env::temp_dir().join(format!("ruster_recent_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let (a, b, c) = (tmp.join("a"), tmp.join("b"), tmp.join("c"));
        for p in [&a, &b, &c] {
            std::fs::create_dir_all(p).unwrap();
        }
        record_recent(&tmp, &a, 2);
        record_recent(&tmp, &b, 2);
        record_recent(&tmp, &a, 2); // re-open a → moves to front, dedup
        assert_eq!(recent_projects(&tmp), vec![a.clone(), b.clone()]);
        record_recent(&tmp, &c, 2);
        assert_eq!(recent_projects(&tmp), vec![c, a]); // b dropped by the cap
        std::fs::remove_dir_all(&tmp).ok();
    }
}
