//! Pure parsers for LSP response/notification payloads into plain ruster types.
//! No I/O — unit-tested directly against sample JSON.

use serde_json::Value;

use crate::position::{position_to_offset, LspPosition};

/// A resolved source location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub uri: String,
    pub start: LspPositionEq,
}

/// `LspPosition` with derived `Eq` for test convenience.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LspPositionEq {
    pub line: u32,
    pub character: u32,
}

impl From<LspPositionEq> for LspPosition {
    fn from(p: LspPositionEq) -> Self {
        LspPosition { line: p.line, character: p.character }
    }
}

fn pos_from(v: &Value) -> LspPositionEq {
    LspPositionEq {
        line: v.get("line").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
        character: v.get("character").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
    }
}

fn strip_file_uri(uri: &str) -> String {
    uri.strip_prefix("file://").unwrap_or(uri).to_string()
}

/// One diagnostic mapped to a range and severity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub start: LspPositionEq,
    pub end: LspPositionEq,
    pub severity: u8, // 1=error 2=warning 3=info 4=hint
    pub message: String,
}

/// A document/workspace symbol (flattened).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolEntry {
    pub name: String,
    pub kind: u8,
    pub start: LspPositionEq,
    pub uri: Option<String>, // set for workspace symbols
    pub depth: u16,
}

/// A single text edit (range + replacement).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    pub start: LspPositionEq,
    pub end: LspPositionEq,
    pub new_text: String,
}

/// Extract hover text (handles `MarkupContent`, `MarkedString`, and arrays).
pub fn parse_hover(v: &Value) -> Option<String> {
    let contents = v.get("contents")?;
    fn one(v: &Value) -> Option<String> {
        if let Some(s) = v.as_str() {
            return Some(s.to_string());
        }
        if let Some(s) = v.get("value").and_then(|x| x.as_str()) {
            return Some(s.to_string()); // MarkupContent / MarkedString{language,value}
        }
        None
    }
    let text = match contents {
        Value::Array(arr) => arr.iter().filter_map(one).collect::<Vec<_>>().join("\n"),
        other => one(other).unwrap_or_default(),
    };
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Parse a definition/references result: `Location`, `Location[]`, or
/// `LocationLink[]`. Paths are returned with the `file://` scheme stripped.
pub fn parse_locations(v: &Value) -> Vec<Location> {
    fn from_one(v: &Value) -> Option<Location> {
        // LocationLink uses targetUri/targetRange; Location uses uri/range.
        if let Some(uri) = v.get("uri").and_then(|x| x.as_str()) {
            let start = pos_from(&v["range"]["start"]);
            return Some(Location { uri: strip_file_uri(uri), start });
        }
        if let Some(uri) = v.get("targetUri").and_then(|x| x.as_str()) {
            let start = pos_from(&v["targetSelectionRange"]["start"]);
            return Some(Location { uri: strip_file_uri(uri), start });
        }
        None
    }
    match v {
        Value::Array(arr) => arr.iter().filter_map(from_one).collect(),
        Value::Null => Vec::new(),
        other => from_one(other).into_iter().collect(),
    }
}

/// One node in a call hierarchy — the caller (incoming) or callee (outgoing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallEntry {
    pub name: String,
    pub detail: Option<String>,
    pub uri: String,
    pub start: LspPositionEq,
}

/// Parse a `textDocument/prepareCallHierarchy` result into the raw
/// `CallHierarchyItem` values, to be handed back verbatim in step 2.
pub fn parse_call_hierarchy_prepare(v: &Value) -> Vec<Value> {
    match v {
        Value::Array(arr) => arr.clone(),
        Value::Null => Vec::new(),
        other => vec![other.clone()],
    }
}

/// Parse an `incomingCalls`/`outgoingCalls` result. `incoming` selects whether
/// each element's endpoint is under `from` (callers) or `to` (callees).
pub fn parse_call_hierarchy_calls(v: &Value, incoming: bool) -> Vec<CallEntry> {
    let key = if incoming { "from" } else { "to" };
    let arr = match v.as_array() {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .filter_map(|call| {
            let item = call.get(key)?;
            let uri = item.get("uri").and_then(|x| x.as_str())?;
            // Prefer the selection range (the name) over the full range.
            let range = item.get("selectionRange").or_else(|| item.get("range"))?;
            Some(CallEntry {
                name: item.get("name").and_then(|x| x.as_str()).unwrap_or("?").to_string(),
                detail: item.get("detail").and_then(|x| x.as_str()).map(str::to_string),
                uri: strip_file_uri(uri),
                start: pos_from(&range["start"]),
            })
        })
        .collect()
}

/// Parse a `publishDiagnostics` notification's params into (path, diagnostics).
pub fn parse_diagnostics(params: &Value) -> (String, Vec<Diagnostic>) {
    let uri = params
        .get("uri")
        .and_then(|x| x.as_str())
        .map(strip_file_uri)
        .unwrap_or_default();
    let diags = params
        .get("diagnostics")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .map(|d| Diagnostic {
                    start: pos_from(&d["range"]["start"]),
                    end: pos_from(&d["range"]["end"]),
                    severity: d.get("severity").and_then(|s| s.as_u64()).unwrap_or(1) as u8,
                    message: d.get("message").and_then(|m| m.as_str()).unwrap_or("").to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    (uri, diags)
}

/// Parse `documentSymbol` (hierarchical `DocumentSymbol[]` or flat
/// `SymbolInformation[]`) into a depth-tagged, pre-order list.
pub fn parse_document_symbols(v: &Value) -> Vec<SymbolEntry> {
    let mut out = Vec::new();
    let arr = match v.as_array() {
        Some(a) => a,
        None => return out,
    };
    // Hierarchical entries have "selectionRange"/"children"; flat ones have "location".
    fn walk(node: &Value, depth: u16, out: &mut Vec<SymbolEntry>) {
        let name = node.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let kind = node.get("kind").and_then(|x| x.as_u64()).unwrap_or(0) as u8;
        if let Some(loc) = node.get("location") {
            // SymbolInformation
            out.push(SymbolEntry {
                name,
                kind,
                start: pos_from(&loc["range"]["start"]),
                uri: loc.get("uri").and_then(|x| x.as_str()).map(strip_file_uri),
                depth,
            });
        } else {
            let start = pos_from(&node["selectionRange"]["start"]);
            out.push(SymbolEntry { name, kind, start, uri: None, depth });
            if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
                for child in children {
                    walk(child, depth + 1, out);
                }
            }
        }
    }
    for node in arr {
        walk(node, 0, &mut out);
    }
    out
}

/// Parse `workspace/symbol` (`SymbolInformation[]`) into entries with uris.
pub fn parse_workspace_symbols(v: &Value) -> Vec<SymbolEntry> {
    v.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|s| {
                    let loc = s.get("location")?;
                    Some(SymbolEntry {
                        name: s.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                        kind: s.get("kind").and_then(|x| x.as_u64()).unwrap_or(0) as u8,
                        start: pos_from(&loc["range"]["start"]),
                        uri: loc.get("uri").and_then(|x| x.as_str()).map(strip_file_uri),
                        depth: 0,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse a formatting result: `TextEdit[]`.
pub fn parse_text_edits(v: &Value) -> Vec<TextEdit> {
    v.as_array()
        .map(|arr| {
            arr.iter()
                .map(|e| TextEdit {
                    start: pos_from(&e["range"]["start"]),
                    end: pos_from(&e["range"]["end"]),
                    new_text: e.get("newText").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse a `WorkspaceEdit` (rename result) into per-file edits. Handles the
/// `changes` map form (`documentChanges` is normalized to the same shape).
pub fn parse_workspace_edit(v: &Value) -> Vec<(String, Vec<TextEdit>)> {
    let mut out = Vec::new();
    if let Some(changes) = v.get("changes").and_then(|c| c.as_object()) {
        for (uri, edits) in changes {
            out.push((strip_file_uri(uri), parse_text_edits(edits)));
        }
    } else if let Some(doc_changes) = v.get("documentChanges").and_then(|c| c.as_array()) {
        for dc in doc_changes {
            if let Some(uri) = dc.get("textDocument").and_then(|t| t.get("uri")).and_then(|u| u.as_str()) {
                let edits = dc.get("edits").cloned().unwrap_or(Value::Null);
                out.push((strip_file_uri(uri), parse_text_edits(&edits)));
            }
        }
    }
    out
}

/// Apply a set of LSP text edits to `text`, returning the new text. Edits are
/// applied bottom-to-top so earlier offsets stay valid; overlapping edits are
/// applied in range order (last wins on ties).
pub fn apply_edits(text: &str, edits: &[TextEdit]) -> String {
    let mut resolved: Vec<(usize, usize, &str)> = edits
        .iter()
        .map(|e| {
            let start = position_to_offset(text, e.start.into());
            let end = position_to_offset(text, e.end.into());
            (start, end, e.new_text.as_str())
        })
        .collect();
    // Apply from the bottom of the document upward.
    resolved.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));

    let chars: Vec<char> = text.chars().collect();
    let mut result = chars;
    for (start, end, new_text) in resolved {
        let start = start.min(result.len());
        let end = end.min(result.len()).max(start);
        let replacement: Vec<char> = new_text.chars().collect();
        result.splice(start..end, replacement);
    }
    result.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn edit(sl: u32, sc: u32, el: u32, ec: u32, t: &str) -> TextEdit {
        TextEdit {
            start: LspPositionEq { line: sl, character: sc },
            end: LspPositionEq { line: el, character: ec },
            new_text: t.to_string(),
        }
    }

    #[test]
    fn apply_edits_bottom_up() {
        let text = "let x = 1;\nlet y = 2;\n";
        // rename both to `z`: replace 'x' (0,4-0,5) and 'y' (1,4-1,5)
        let edits = vec![edit(0, 4, 0, 5, "z"), edit(1, 4, 1, 5, "z")];
        let out = apply_edits(text, &edits);
        assert_eq!(out, "let z = 1;\nlet z = 2;\n");
    }

    #[test]
    fn apply_edits_insertion_and_deletion() {
        let text = "abcd";
        // insert "X" at start, delete "cd"
        let edits = vec![edit(0, 0, 0, 0, "X"), edit(0, 2, 0, 4, "")];
        assert_eq!(apply_edits(text, &edits), "Xab");
    }

    #[test]
    fn hover_markup_and_array() {
        assert_eq!(
            parse_hover(&json!({"contents": {"kind": "markdown", "value": "**x**: i32"}})),
            Some("**x**: i32".to_string())
        );
        assert_eq!(
            parse_hover(&json!({"contents": ["a", {"value": "b"}]})),
            Some("a\nb".to_string())
        );
        assert_eq!(parse_hover(&json!({"contents": []})), None);
    }

    #[test]
    fn locations_all_shapes() {
        let single = json!({"uri": "file:///a.rs", "range": {"start": {"line": 2, "character": 4}, "end": {"line": 2, "character": 8}}});
        let locs = parse_locations(&single);
        assert_eq!(locs.len(), 1);
        assert_eq!(locs[0].uri, "/a.rs");
        assert_eq!(locs[0].start, LspPositionEq { line: 2, character: 4 });

        let link = json!([{"targetUri": "file:///b.rs", "targetSelectionRange": {"start": {"line": 1, "character": 0}, "end": {"line": 1, "character": 3}}}]);
        let locs = parse_locations(&link);
        assert_eq!(locs[0].uri, "/b.rs");
        assert_eq!(locs[0].start, LspPositionEq { line: 1, character: 0 });

        assert!(parse_locations(&Value::Null).is_empty());
    }

    #[test]
    fn call_hierarchy_prepare_and_calls() {
        // prepare returns items to hand back verbatim.
        let prep = json!([{"name": "foo", "uri": "file:///a.rs",
            "range": {"start": {"line": 1, "character": 0}, "end": {"line": 1, "character": 3}}}]);
        let items = parse_call_hierarchy_prepare(&prep);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["name"], "foo");
        assert!(parse_call_hierarchy_prepare(&Value::Null).is_empty());

        // Incoming calls read the caller under `from`; prefer selectionRange.
        let incoming = json!([{
            "from": {"name": "caller", "detail": "mod::caller", "uri": "file:///b.rs",
                "range": {"start": {"line": 9, "character": 0}, "end": {"line": 20, "character": 0}},
                "selectionRange": {"start": {"line": 9, "character": 4}, "end": {"line": 9, "character": 10}}},
            "fromRanges": []
        }]);
        let calls = parse_call_hierarchy_calls(&incoming, true);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "caller");
        assert_eq!(calls[0].detail.as_deref(), Some("mod::caller"));
        assert_eq!(calls[0].uri, "/b.rs");
        assert_eq!(calls[0].start, LspPositionEq { line: 9, character: 4 });

        // Outgoing calls read the callee under `to`.
        let outgoing = json!([{
            "to": {"name": "callee", "uri": "file:///c.rs",
                "range": {"start": {"line": 3, "character": 0}, "end": {"line": 3, "character": 6}}},
            "fromRanges": []
        }]);
        let calls = parse_call_hierarchy_calls(&outgoing, false);
        assert_eq!(calls[0].name, "callee");
        assert_eq!(calls[0].uri, "/c.rs");
        // Falls back to `range` when selectionRange is absent.
        assert_eq!(calls[0].start, LspPositionEq { line: 3, character: 0 });

        assert!(parse_call_hierarchy_calls(&Value::Null, true).is_empty());
    }

    #[test]
    fn diagnostics_parse() {
        let params = json!({
            "uri": "file:///a.rs",
            "diagnostics": [
                {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 5}}, "severity": 1, "message": "boom"}
            ]
        });
        let (path, diags) = parse_diagnostics(&params);
        assert_eq!(path, "/a.rs");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, 1);
        assert_eq!(diags[0].message, "boom");
    }

    #[test]
    fn document_symbols_hierarchical_and_flat() {
        let hier = json!([
            {"name": "Foo", "kind": 5, "selectionRange": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 3}},
             "children": [
                {"name": "bar", "kind": 6, "selectionRange": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 7}}}
             ]}
        ]);
        let syms = parse_document_symbols(&hier);
        assert_eq!(syms.len(), 2);
        assert_eq!(syms[0].name, "Foo");
        assert_eq!(syms[0].depth, 0);
        assert_eq!(syms[1].name, "bar");
        assert_eq!(syms[1].depth, 1);

        let flat = json!([
            {"name": "g", "kind": 12, "location": {"uri": "file:///a.rs", "range": {"start": {"line": 3, "character": 0}, "end": {"line": 3, "character": 1}}}}
        ]);
        let syms = parse_document_symbols(&flat);
        assert_eq!(syms[0].name, "g");
        assert_eq!(syms[0].start, LspPositionEq { line: 3, character: 0 });
    }

    #[test]
    fn text_edits_and_workspace_edit() {
        let edits = json!([
            {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}, "newText": "x"}
        ]);
        assert_eq!(parse_text_edits(&edits)[0].new_text, "x");

        let we = json!({"changes": {"file:///a.rs": edits}});
        let per_file = parse_workspace_edit(&we);
        assert_eq!(per_file.len(), 1);
        assert_eq!(per_file[0].0, "/a.rs");
        assert_eq!(per_file[0].1[0].new_text, "x");
    }
}
