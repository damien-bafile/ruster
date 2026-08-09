//! Drives a real language server end to end.
//!
//! Ignored by default: it needs `rust-analyzer` on PATH and takes tens of
//! seconds. It exists because every layer below it can be green while the whole
//! produces nothing — the unit tests assert that we *write* an initialize and
//! that a handcrafted response is parsed, and neither notices if a real server
//! never answers.
//!
//! Run with `cargo test -p ruster-lsp --test live_server -- --ignored --nocapture`.

use std::path::Path;
use std::time::{Duration, Instant};

use ruster_lsp::ServerMessage;

#[test]
#[ignore = "needs rust-analyzer and a real workspace"]
fn a_real_server_reports_diagnostics_for_a_broken_file() {
    let root = Path::new("/tmp/lsp-demo");
    if !root.join("Cargo.toml").exists() {
        eprintln!("skipping: {} is not a cargo project", root.display());
        return;
    }
    let mut manager = ruster_lsp::manager::LspManager::new();
    assert!(
        manager.ensure("rust", root),
        "no server configured for rust"
    );

    let path = root.join("src/main.rs");
    let text = std::fs::read_to_string(&path).unwrap();
    let uri = ruster_lsp::protocol::uri_from_path(&path);
    let key = ruster_lsp::manager::ServerKey::new("rust", root);
    manager.did_open(&key, &uri, "rust", 1, &text);

    let deadline = Instant::now() + Duration::from_secs(90);
    let mut methods: Vec<String> = Vec::new();
    let mut diagnostics = 0usize;
    while Instant::now() < deadline {
        for routed in manager.poll() {
            if let ServerMessage::Notification { method, params } = &routed.message {
                methods.push(method.clone());
                if method == "textDocument/publishDiagnostics" {
                    let (p, diags) = ruster_lsp::parse_diagnostics(params);
                    eprintln!("diagnostics for {p}: {}", diags.len());
                    diagnostics += diags.len();
                }
            }
        }
        if diagnostics > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    eprintln!("methods seen: {methods:?}");
    assert!(
        diagnostics > 0,
        "no diagnostics after 90s; methods seen: {methods:?}"
    );
}
