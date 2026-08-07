//! Pure builders for LSP request/notification params, plus small protocol
//! helpers. Kept free of I/O so they can be unit-tested directly.

use std::path::Path;

use serde_json::{json, Value};

use crate::position::LspPosition;

/// `file://` URI for a filesystem path, with symlinks resolved.
///
/// Resolving matters because the server resolves too, and a workspace root and
/// a document that disagree about the same directory are not the same place to
/// it. On macOS `/var` and `/tmp` are symlinks (`/private/var`, `/private/tmp`),
/// so a project opened under either got `rootUri: file:///var/…` while its
/// documents — which were already canonicalised at the call site — arrived as
/// `file:///private/var/…`. rust-analyzer put every one of them outside the
/// workspace and answered `null`, which reads on screen as hover, definition
/// and references all being broken rather than as a path mismatch.
///
/// Canonicalising here rather than at each call site is deliberate: the root
/// and the documents have to agree, and they only reliably agree if the same
/// function decides for both.
///
/// A path that does not exist is left alone — `canonicalize` needs a real file,
/// and a best-effort URI beats none.
pub fn uri_from_path(path: &Path) -> String {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text = resolved.to_string_lossy();
    // Windows canonicalisation yields a verbatim prefix (`\\?\C:\…`) that no
    // language server accepts inside a `file://` URI.
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    format!("file://{}", text)
}

/// `initialize` params advertising the capabilities ruster actually uses.
pub fn initialize_params(root: &Path) -> Value {
    let root_uri = uri_from_path(root);
    json!({
        "processId": std::process::id(),
        "rootUri": root_uri,
        "capabilities": {
            "textDocument": {
                "synchronization": { "didSave": true, "dynamicRegistration": false },
                "hover": { "contentFormat": ["markdown", "plaintext"] },
                "definition": {},
                "references": {},
                "rename": {},
                "formatting": {},
                "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
                "callHierarchy": { "dynamicRegistration": false },
                "publishDiagnostics": {}
            },
            "workspace": { "symbol": {} }
        }
    })
}

pub fn did_open_params(uri: &str, language_id: &str, version: i64, text: &str) -> Value {
    json!({
        "textDocument": {
            "uri": uri,
            "languageId": language_id,
            "version": version,
            "text": text
        }
    })
}

/// Full-text (non-incremental) `didChange`.
pub fn did_change_params(uri: &str, version: i64, text: &str) -> Value {
    json!({
        "textDocument": { "uri": uri, "version": version },
        "contentChanges": [ { "text": text } ]
    })
}

pub fn did_close_params(uri: &str) -> Value {
    json!({ "textDocument": { "uri": uri } })
}

/// `TextDocumentPositionParams` used by hover/definition/references/rename.
pub fn text_document_position(uri: &str, pos: LspPosition) -> Value {
    json!({
        "textDocument": { "uri": uri },
        "position": { "line": pos.line, "character": pos.character }
    })
}

pub fn references_params(uri: &str, pos: LspPosition) -> Value {
    json!({
        "textDocument": { "uri": uri },
        "position": { "line": pos.line, "character": pos.character },
        "context": { "includeDeclaration": false }
    })
}

pub fn rename_params(uri: &str, pos: LspPosition, new_name: &str) -> Value {
    json!({
        "textDocument": { "uri": uri },
        "position": { "line": pos.line, "character": pos.character },
        "newName": new_name
    })
}

pub fn formatting_params(uri: &str, tab_size: u32, insert_spaces: bool) -> Value {
    json!({
        "textDocument": { "uri": uri },
        "options": { "tabSize": tab_size, "insertSpaces": insert_spaces }
    })
}

pub fn document_symbol_params(uri: &str) -> Value {
    json!({ "textDocument": { "uri": uri } })
}

/// Step 1 of call hierarchy: resolve the symbol at `pos` into a hierarchy item.
pub fn prepare_call_hierarchy_params(uri: &str, pos: LspPosition) -> Value {
    json!({
        "textDocument": { "uri": uri },
        "position": { "line": pos.line, "character": pos.character }
    })
}

/// Step 2: given a `CallHierarchyItem` (from prepare), request its incoming or
/// outgoing calls. The method name selects the direction.
pub fn call_hierarchy_calls_params(item: &Value) -> Value {
    json!({ "item": item })
}

pub fn workspace_symbol_params(query: &str) -> Value {
    json!({ "query": query })
}

/// How to reply to a server-initiated request so the server doesn't block.
/// Returns the `result` value to send, or `None` to ignore.
pub fn server_request_reply(method: &str) -> Option<Value> {
    match method {
        // We register nothing dynamically; acknowledge with null.
        "client/registerCapability" | "client/unregisterCapability" => Some(Value::Null),
        // No workspace configuration to report; reply one null per requested item.
        "workspace/configuration" => Some(json!([Value::Null])),
        // Work-done progress create — acknowledge.
        "window/workDoneProgress/create" => Some(Value::Null),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_has_file_scheme() {
        // Nonexistent: canonicalisation cannot apply, so the path is used as-is.
        assert_eq!(uri_from_path(Path::new("/nonexistent-uri-test/x.rs")), "file:///nonexistent-uri-test/x.rs");
    }

    /// A workspace root and its documents must resolve to the same directory.
    ///
    /// The root was left unresolved while documents were canonicalised at the
    /// call site, so on macOS — where `/var` and `/tmp` are symlinks — a project
    /// under either got `rootUri: file:///var/…` and documents at
    /// `file:///private/var/…`. rust-analyzer placed every document outside the
    /// workspace and answered `null`, which looks exactly like hover, goto and
    /// references being broken.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_root_and_its_documents_resolve_to_the_same_place() {
        use std::os::unix::fs::symlink;

        let base = std::env::temp_dir().join(format!("ruster_uri_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let real = base.join("real");
        std::fs::create_dir_all(real.join("src")).unwrap();
        std::fs::write(real.join("src").join("main.rs"), "fn main() {}\n").unwrap();

        let link = base.join("link");
        symlink(&real, &link).unwrap();

        // The root reached through the symlink, the document through the same.
        let root_uri = uri_from_path(&link);
        let doc_uri = uri_from_path(&link.join("src").join("main.rs"));

        assert!(
            doc_uri.starts_with(&format!("{root_uri}/")),
            "the document must sit inside the root it was opened from:\n  root {root_uri}\n  doc  {doc_uri}"
        );
        // And the root reached directly must name that same place.
        assert_eq!(root_uri, uri_from_path(&real), "both routes name one directory");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn initialize_advertises_root_and_capabilities() {
        let p = initialize_params(Path::new("/proj"));
        assert_eq!(p["rootUri"], "file:///proj");
        assert!(p["capabilities"]["textDocument"]["hover"].is_object());
    }

    #[test]
    fn did_open_and_change_shapes() {
        let o = did_open_params("file:///a.rs", "rust", 0, "fn main(){}");
        assert_eq!(o["textDocument"]["languageId"], "rust");
        assert_eq!(o["textDocument"]["version"], 0);
        let c = did_change_params("file:///a.rs", 1, "fn main(){ }");
        assert_eq!(c["textDocument"]["version"], 1);
        assert_eq!(c["contentChanges"][0]["text"], "fn main(){ }");
    }

    #[test]
    fn position_params_carry_line_and_character() {
        let p = text_document_position(
            "file:///a.rs",
            LspPosition {
                line: 3,
                character: 7,
            },
        );
        assert_eq!(p["position"]["line"], 3);
        assert_eq!(p["position"]["character"], 7);
    }

    #[test]
    fn server_requests_get_appropriate_replies() {
        assert_eq!(
            server_request_reply("client/registerCapability"),
            Some(Value::Null)
        );
        assert_eq!(
            server_request_reply("workspace/configuration"),
            Some(json!([Value::Null]))
        );
        assert_eq!(server_request_reply("textDocument/hover"), None);
    }
}
