use std::path::Path;

#[derive(Debug, Clone)]
pub struct AdapterConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub launch_config: serde_json::Value,
}

/// LLVM renamed `lldb-vscode` to `lldb-dap` in LLVM 18; both names are still in
/// the wild, so prefer whichever is actually installed and fall back to the new
/// name when neither is (the spawn error then names the current binary).
fn lldb_adapter() -> String {
    for cmd in ["lldb-dap", "lldb-vscode"] {
        if on_path(cmd) {
            return cmd.to_string();
        }
    }
    "lldb-dap".to_string()
}

fn on_path(cmd: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| dir.join(cmd).is_file())
}

pub fn detect_config(language: &str, root: &Path, program: Option<&str>) -> Option<AdapterConfig> {
    let lang = language.to_lowercase();
    if lang.contains("rust") || lang.contains("rs") {
        // No program means the caller could not name one; an empty string at
        // least reaches the adapter as an obvious error instead of the old
        // literal `target/debug/<binary>` placeholder, which looked like a
        // path and could never be one.
        let program = program.unwrap_or("");
        let command = lldb_adapter();
        Some(AdapterConfig {
            name: command.clone(),
            command,
            args: vec![],
            launch_config: serde_json::json!({
                "type": "lldb",
                "request": "launch",
                "program": program,
                "args": [],
                "cwd": root.to_string_lossy().to_string(),
                "stopOnEntry": false,
            }),
        })
    } else if lang.contains("python") || lang.contains("py") {
        Some(AdapterConfig {
            name: "debugpy".to_string(),
            command: "python".to_string(),
            args: vec!["-m".to_string(), "debugpy.adapter".to_string()],
            launch_config: serde_json::json!({
                "type": "python",
                "request": "launch",
                "program": program.unwrap_or("${file}"),
                "console": "integratedTerminal",
            }),
        })
    } else if lang.contains("go") || lang.contains("golang") {
        Some(AdapterConfig {
            name: "dlv-dap".to_string(),
            command: "dlv".to_string(),
            args: vec!["dap".to_string()],
            launch_config: serde_json::json!({
                "type": "go",
                "request": "launch",
                "mode": "auto",
                "program": program.unwrap_or("."),
            }),
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The program the caller resolved has to survive into `launch_config` —
    /// that object is the whole launch request, and a session that reaches the
    /// adapter without a program reports RUNNING and then does nothing.
    #[test]
    fn the_resolved_program_reaches_the_launch_config() {
        let cfg = detect_config(
            "rust",
            Path::new("/proj"),
            Some("/proj/target/debug/widget"),
        )
        .expect("rust has an adapter");
        assert_eq!(cfg.launch_config["program"], "/proj/target/debug/widget");
        assert_eq!(cfg.launch_config["cwd"], "/proj");
        assert_eq!(cfg.launch_config["request"], "launch");
    }

    /// The old default was the literal string `target/debug/<binary>`, which is
    /// not a path and cannot become one.
    #[test]
    fn an_unresolved_program_is_empty_not_a_placeholder() {
        let cfg = detect_config("rs", Path::new("/proj"), None).unwrap();
        assert_eq!(cfg.launch_config["program"], "");
    }

    #[test]
    fn the_rust_adapter_is_an_lldb_binary() {
        let cfg = detect_config("rust", Path::new("/proj"), None).unwrap();
        assert!(
            matches!(cfg.command.as_str(), "lldb-dap" | "lldb-vscode"),
            "unexpected adapter {}",
            cfg.command
        );
        assert_eq!(cfg.name, cfg.command);
    }

    #[test]
    fn unknown_languages_have_no_adapter() {
        assert!(detect_config("brainfuck", Path::new("/proj"), None).is_none());
    }
}
