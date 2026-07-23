//! Pure builders for LSP request/notification params, plus small protocol
//! helpers. Kept free of I/O so they can be unit-tested directly.

use std::path::Path;

use serde_json::{json, Value};

use crate::position::LspPosition;

/// `file://` URI for a filesystem path (absolute paths only; best-effort).
pub fn uri_from_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    if s.starts_with('/') {
        format!("file://{}", s)
    } else {
        format!("file://{}", s) // relative — servers generally still accept it
    }
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
        assert_eq!(uri_from_path(Path::new("/tmp/x.rs")), "file:///tmp/x.rs");
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
        let p = text_document_position("file:///a.rs", LspPosition { line: 3, character: 7 });
        assert_eq!(p["position"]["line"], 3);
        assert_eq!(p["position"]["character"], 7);
    }

    #[test]
    fn server_requests_get_appropriate_replies() {
        assert_eq!(server_request_reply("client/registerCapability"), Some(Value::Null));
        assert_eq!(server_request_reply("workspace/configuration"), Some(json!([Value::Null])));
        assert_eq!(server_request_reply("textDocument/hover"), None);
    }
}
