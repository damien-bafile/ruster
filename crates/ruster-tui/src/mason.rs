//! `:Mason` — which external tools ruster can use, whether they are installed,
//! and how to install them (Phase 6 Task 13).
//!
//! Deliberately *not* a package manager. ruster bundles no binaries, downloads
//! nothing itself, and runs no install command without the user confirming the
//! exact text first. Every entry in the registry is the tool's own documented
//! install method — the same line the user would have typed — so `:Mason` is a
//! reminder and a shortcut, not a privileged installer.
//!
//! Everything here is pure. [`parse_registry`] turns the embedded table into
//! rows and [`on_path`] decides "installed" against a caller-supplied lookup,
//! so both test without touching the filesystem or spawning anything.

use std::path::{Path, PathBuf};

/// What a tool is for. Shown as a group heading in the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ToolKind {
    Lsp,
    Dap,
    Formatter,
}

impl ToolKind {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "lsp" => Some(ToolKind::Lsp),
            "dap" => Some(ToolKind::Dap),
            "fmt" => Some(ToolKind::Formatter),
            _ => None,
        }
    }

    pub fn heading(self) -> &'static str {
        match self {
            ToolKind::Lsp => "Language servers",
            ToolKind::Dap => "Debug adapters",
            ToolKind::Formatter => "Formatters",
        }
    }
}

/// One row of the registry: a tool, and how to install it on one platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tool {
    pub kind: ToolKind,
    pub name: String,
    /// The executable probed on `PATH` to decide whether it is installed.
    pub binary: String,
    /// `any`, `macos`, `linux` or `windows`.
    pub platform: String,
    /// Run verbatim in a shell, only after the user confirms.
    pub install: String,
}

impl Tool {
    /// Whether this row applies to the host.
    pub fn applies_to(&self, host: &str) -> bool {
        self.platform == "any" || self.platform == host
    }
}

/// The platform name this build matches registry rows against.
pub fn host_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

/// Parse the registry table.
///
/// Blank lines and `#` comments are skipped. A row is
/// `kind | name | binary | platform | install`; anything malformed is skipped
/// rather than failing the whole table, so one bad line cannot take `:Mason`
/// down — but see the test asserting the shipped table has no skipped rows.
pub fn parse_registry(text: &str) -> Vec<Tool> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|line| {
            let f: Vec<&str> = line.split('|').map(str::trim).collect();
            let [kind, name, binary, platform, install] = f[..] else { return None };
            if name.is_empty() || binary.is_empty() || install.is_empty() {
                return None;
            }
            Some(Tool {
                kind: ToolKind::parse(kind)?,
                name: name.to_string(),
                binary: binary.to_string(),
                platform: platform.to_string(),
                install: install.to_string(),
            })
        })
        .collect()
}

/// The tools offered on `host`: one row per tool, preferring a platform-specific
/// install over the `any` fallback, and dropping tools with neither.
pub fn tools_for(text: &str, host: &str) -> Vec<Tool> {
    let rows = parse_registry(text);
    let mut out: Vec<Tool> = Vec::new();
    for row in rows.into_iter().filter(|r| r.applies_to(host)) {
        match out.iter_mut().find(|t| t.name == row.name) {
            // A platform-specific row beats the `any` fallback, whatever order
            // they appear in.
            Some(existing) if existing.platform == "any" && row.platform != "any" => {
                *existing = row;
            }
            Some(_) => {}
            None => out.push(row),
        }
    }
    out.sort_by(|a, b| (a.kind, &a.name).cmp(&(b.kind, &b.name)));
    out
}

/// The registry ruster ships.
pub fn builtin_tools() -> Vec<Tool> {
    tools_for(include_str!("mason_registry.txt"), host_platform())
}

/// Whether `binary` is findable on a `PATH`-style string.
///
/// `exists` decides whether a candidate is really there, so the probe is
/// testable without creating executables. On Windows the usual suffixes are
/// tried, since `clangd` on `PATH` is `clangd.exe`.
pub fn on_path(binary: &str, path: &str, exists: impl Fn(&Path) -> bool) -> bool {
    if binary.is_empty() {
        return false;
    }
    let sep = if cfg!(windows) { ';' } else { ':' };
    let suffixes: &[&str] = if cfg!(windows) { &["", ".exe", ".cmd", ".bat"] } else { &[""] };
    path.split(sep)
        .filter(|d| !d.is_empty())
        .any(|dir| suffixes.iter().any(|sfx| exists(&PathBuf::from(dir).join(format!("{binary}{sfx}")))))
}

/// [`on_path`] against the real `PATH`, counting a candidate as present only if
/// it is a file — a directory named `clangd` is not a program.
pub fn is_installed(binary: &str) -> bool {
    let path = std::env::var("PATH").unwrap_or_default();
    on_path(binary, &path, |p| p.is_file())
}

/// One line of the `:Mason` listing.
pub fn render_row(tool: &Tool, installed: bool) -> String {
    format!("  {} {:<32} {}", if installed { '✓' } else { '·' }, tool.name, tool.install)
}

/// The whole listing, grouped by kind.
pub fn render(tools: &[Tool], installed: impl Fn(&str) -> bool) -> String {
    let mut out = Vec::new();
    let mut kind = None;
    let (mut have, total) = (0usize, tools.len());
    for t in tools {
        if kind != Some(t.kind) {
            if kind.is_some() {
                out.push(String::new());
            }
            out.push(format!("{}:", t.kind.heading()));
            kind = Some(t.kind);
        }
        let ok = installed(&t.binary);
        have += usize::from(ok);
        out.push(render_row(t, ok));
    }
    if tools.is_empty() {
        return "No tools known for this platform.".to_string();
    }
    out.push(String::new());
    out.push(format!("{have} of {total} installed. Press Enter on a row to install it."));
    out.join("\n")
}

/// The tool a screen row refers to, for `Enter` on the listing.
pub fn tool_at_row(tools: &[Tool], rendered: &str, row: usize) -> Option<Tool> {
    let line = rendered.lines().nth(row)?;
    // Rows are the indented ones; headings and the summary are not selectable.
    let name = line.strip_prefix("  ")?.get(2..)?.trim_start();
    tools.iter().find(|t| name.starts_with(t.name.as_str())).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# a comment

lsp | rust-analyzer | rust-analyzer | any | rustup component add rust-analyzer
lsp | clangd | clangd | macos | brew install llvm
lsp | clangd | clangd | linux | apt-get install -y clangd
fmt | black | black | any | pip install black
   | broken row
dap | nokind-x | x | any |
";

    #[test]
    fn parsing_skips_comments_blanks_and_malformed_rows() {
        let rows = parse_registry(SAMPLE);
        assert_eq!(rows.len(), 4, "{rows:#?}");
        assert_eq!(rows[0].name, "rust-analyzer");
        assert_eq!(rows[0].kind, ToolKind::Lsp);
        assert_eq!(rows[0].install, "rustup component add rust-analyzer");
        assert!(rows.iter().all(|r| !r.install.is_empty()), "no row installs nothing");
    }

    /// The point of the platform column: the same tool installs differently.
    #[test]
    fn a_platform_row_wins_over_the_any_fallback() {
        let mac = tools_for(SAMPLE, "macos");
        let clangd = mac.iter().find(|t| t.name == "clangd").expect("offered on macos");
        assert_eq!(clangd.install, "brew install llvm");

        let linux = tools_for(SAMPLE, "linux");
        let clangd = linux.iter().find(|t| t.name == "clangd").expect("offered on linux");
        assert_eq!(clangd.install, "apt-get install -y clangd");
    }

    /// A tool with no row for this platform is not offered at all — better a
    /// missing entry than an install command that cannot work here.
    #[test]
    fn a_tool_with_no_row_for_this_platform_is_dropped() {
        let win = tools_for(SAMPLE, "windows");
        assert!(win.iter().all(|t| t.name != "clangd"), "clangd has no windows row");
        assert!(win.iter().any(|t| t.name == "rust-analyzer"), "but `any` rows still apply");
    }

    #[test]
    fn each_tool_appears_once_per_platform() {
        for host in ["macos", "linux", "windows"] {
            let tools = tools_for(SAMPLE, host);
            let mut names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
            let before = names.len();
            names.sort();
            names.dedup();
            assert_eq!(names.len(), before, "duplicate tool on {host}");
        }
    }

    /// The shipped table must parse completely — a silently skipped row would
    /// be a tool that quietly vanishes from the list.
    #[test]
    fn the_shipped_registry_has_no_malformed_rows() {
        const SRC: &str = include_str!("mason_registry.txt");
        let meaningful = SRC
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .count();
        assert_eq!(parse_registry(SRC).len(), meaningful, "some row failed to parse");
        assert!(meaningful > 10, "the registry should not be nearly empty");
    }

    /// Every shipped install command must be a real one, not a placeholder.
    #[test]
    fn every_shipped_tool_has_a_plausible_install_command() {
        for t in parse_registry(include_str!("mason_registry.txt")) {
            assert!(
                ["rustup", "npm", "cargo", "brew", "python3", "sudo", "pip"]
                    .iter()
                    .any(|p| t.install.starts_with(p)),
                "{} installs via an unrecognised method: {:?}",
                t.name,
                t.install
            );
            assert!(!t.install.contains("curl "), "{}: no piping installers", t.name);
        }
    }

    #[test]
    fn path_probing_finds_a_binary_in_any_directory() {
        let sep = if cfg!(windows) { ';' } else { ':' };
        let path = ["/a", "/b", "/c"].join(&sep.to_string());
        let hit = |p: &Path| p.ends_with("b/black") || p.ends_with("b\\black.exe");
        assert!(on_path("black", &path, hit));
        assert!(!on_path("missing", &path, hit));
    }

    #[test]
    fn path_probing_handles_empty_input() {
        assert!(!on_path("x", "", |_| true), "an empty PATH finds nothing");
        assert!(!on_path("", "/a", |_| true), "an empty binary name is not a tool");
    }

    #[test]
    fn the_listing_groups_by_kind_and_marks_what_is_installed() {
        let tools = tools_for(SAMPLE, "macos");
        let out = render(&tools, |b| b == "rust-analyzer");
        assert!(out.contains("Language servers:"), "{out}");
        assert!(out.contains("Formatters:"), "{out}");
        assert!(out.lines().any(|l| l.contains("✓") && l.contains("rust-analyzer")));
        assert!(out.lines().any(|l| l.contains("·") && l.contains("black")));
        assert!(out.contains("1 of 3 installed"), "{out}");
        // The command is visible before anyone agrees to run it.
        assert!(out.contains("rustup component add rust-analyzer"), "{out}");
    }

    #[test]
    fn an_empty_listing_says_so() {
        assert!(render(&[], |_| false).starts_with("No tools"));
    }

    #[test]
    fn a_row_resolves_back_to_its_tool_and_headings_do_not() {
        let tools = tools_for(SAMPLE, "macos");
        let out = render(&tools, |_| false);
        let rows: Vec<&str> = out.lines().collect();
        let idx = rows.iter().position(|l| l.contains("rust-analyzer")).unwrap();
        assert_eq!(tool_at_row(&tools, &out, idx).unwrap().name, "rust-analyzer");

        let heading = rows.iter().position(|l| l.ends_with("servers:")).unwrap();
        assert!(tool_at_row(&tools, &out, heading).is_none(), "headings are not installable");
        assert!(tool_at_row(&tools, &out, 9999).is_none(), "out of range is not a tool");
    }
}
