//! Default language-server commands per language.

/// A language server invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub cmd: String,
    pub args: Vec<String>,
}

impl ServerConfig {
    fn new(cmd: &str, args: &[&str]) -> Self {
        ServerConfig {
            cmd: cmd.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
        }
    }
}

/// The default server for a language key (matching `ruster-syntax`'s keys), or
/// `None` when ruster has no built-in default. Users can override via config.
pub fn default_server(lang: &str) -> Option<ServerConfig> {
    Some(match lang {
        "rust" => ServerConfig::new("rust-analyzer", &[]),
        "python" => ServerConfig::new("pyright-langserver", &["--stdio"]),
        "typescript" | "javascript" => {
            ServerConfig::new("typescript-language-server", &["--stdio"])
        }
        "c" => ServerConfig::new("clangd", &[]),
        "lua" => ServerConfig::new("lua-language-server", &[]),
        "scheme" => ServerConfig::new("scheme-lsp-server", &["--stdio"]),
        _ => return None,
    })
}

/// The LSP `languageId` string for a language key (sent in `didOpen`).
pub fn language_id(lang: &str) -> &str {
    match lang {
        "rust" => "rust",
        "python" => "python",
        "javascript" => "javascript",
        "typescript" => "typescript",
        "c" => "c",
        "lua" => "lua",
        "json" => "json",
        "toml" => "toml",
        "yaml" => "yaml",
        "scheme" => "scheme",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_defaults_to_rust_analyzer() {
        let cfg = default_server("rust").unwrap();
        assert_eq!(cfg.cmd, "rust-analyzer");
        assert!(cfg.args.is_empty());
    }

    #[test]
    fn python_defaults_to_pyright_stdio() {
        let cfg = default_server("python").unwrap();
        assert_eq!(cfg.cmd, "pyright-langserver");
        assert_eq!(cfg.args, vec!["--stdio".to_string()]);
    }

    #[test]
    fn js_and_ts_share_a_server() {
        assert_eq!(default_server("javascript"), default_server("typescript"));
    }

    #[test]
    fn unknown_language_has_no_default() {
        assert!(default_server("brainfuck").is_none());
    }
}
