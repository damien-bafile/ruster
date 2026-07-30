use std::path::Path;

#[derive(Debug, Clone)]
pub struct AdapterConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub launch_config: serde_json::Value,
}

pub fn detect_config(language: &str, root: &Path, program: Option<&str>) -> Option<AdapterConfig> {
    let lang = language.to_lowercase();
    if lang.contains("rust") || lang.contains("rs") {
        let program = program.unwrap_or("target/debug/<binary>");
        Some(AdapterConfig {
            name: "lldb-vscode".to_string(),
            command: "lldb-vscode".to_string(),
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
