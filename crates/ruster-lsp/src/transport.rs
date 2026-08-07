//! JSON-RPC 2.0 message framing for LSP (`Content-Length` headers over stdio).

use std::io::{self, BufRead, Read, Write};
use std::sync::mpsc::Sender;

use serde_json::Value;

/// A message received from a language server.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerMessage {
    /// A response to a request we sent (matched by `id`).
    Response {
        id: i64,
        result: Value,
        error: Option<Value>,
    },
    /// A server-initiated notification (e.g. `textDocument/publishDiagnostics`).
    Notification { method: String, params: Value },
    /// A server-initiated request that expects a response (e.g.
    /// `client/registerCapability`). We reply with a null result for now.
    Request {
        id: i64,
        method: String,
        params: Value,
    },
}

/// Classify a decoded JSON-RPC object into a [`ServerMessage`].
pub fn classify(value: Value) -> ServerMessage {
    let has_method = value.get("method").is_some();
    let id = value.get("id").and_then(|v| v.as_i64());
    match (has_method, id) {
        (true, Some(id)) => ServerMessage::Request {
            id,
            method: value["method"].as_str().unwrap_or("").to_string(),
            params: value.get("params").cloned().unwrap_or(Value::Null),
        },
        (true, None) => ServerMessage::Notification {
            method: value["method"].as_str().unwrap_or("").to_string(),
            params: value.get("params").cloned().unwrap_or(Value::Null),
        },
        (false, Some(id)) => ServerMessage::Response {
            id,
            result: value.get("result").cloned().unwrap_or(Value::Null),
            error: value.get("error").cloned(),
        },
        (false, None) => ServerMessage::Notification {
            method: String::new(),
            params: value,
        },
    }
}

/// Write a single JSON-RPC message with the `Content-Length` framing.
pub fn write_message(w: &mut impl Write, value: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(value)?;
    write!(w, "Content-Length: {}\r\n\r\n", body.len())?;
    w.write_all(&body)?;
    w.flush()
}

/// Read one framed JSON-RPC message. Returns `Ok(None)` at clean EOF.
pub fn read_message(r: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = r.read_line(&mut line)?;
        if n == 0 {
            return Ok(None); // EOF
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break; // end of headers
        }
        if let Some(rest) = trimmed
            .strip_prefix("Content-Length:")
            .or_else(|| trimmed.strip_prefix("content-length:"))
        {
            content_length = rest.trim().parse().ok();
        }
        // Other headers (Content-Type) are ignored.
    }
    let len = match content_length {
        Some(n) => n,
        None => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "missing Content-Length header",
            ))
        }
    };
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    let value =
        serde_json::from_slice(&buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(Some(value))
}

/// Loop reading framed messages from `reader`, classifying each and sending it
/// over `tx`, until EOF or the receiver is dropped. Intended to run on its own
/// thread.
pub fn read_loop(reader: impl Read, tx: Sender<ServerMessage>) {
    let mut buf = io::BufReader::new(reader);
    loop {
        match read_message(&mut buf) {
            Ok(Some(value)) => {
                if tx.send(classify(value)).is_err() {
                    break; // client dropped
                }
            }
            Ok(None) => break, // EOF
            Err(_) => break,   // malformed / broken pipe
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;

    #[test]
    fn write_then_read_round_trips() {
        let msg = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"});
        let mut buf = Vec::new();
        write_message(&mut buf, &msg).unwrap();
        // Header is present and body follows.
        let text = String::from_utf8_lossy(&buf);
        assert!(text.starts_with("Content-Length: "));
        let mut cursor = Cursor::new(buf);
        let read = read_message(&mut cursor).unwrap().unwrap();
        assert_eq!(read, msg);
    }

    #[test]
    fn read_at_eof_returns_none() {
        let mut cursor = Cursor::new(Vec::new());
        assert_eq!(read_message(&mut cursor).unwrap(), None);
    }

    #[test]
    fn classify_response_notification_request() {
        assert!(matches!(
            classify(json!({"id": 3, "result": {"ok": true}})),
            ServerMessage::Response { id: 3, .. }
        ));
        assert!(matches!(
            classify(json!({"method": "textDocument/publishDiagnostics", "params": {}})),
            ServerMessage::Notification { .. }
        ));
        assert!(matches!(
            classify(json!({"id": 5, "method": "client/registerCapability", "params": {}})),
            ServerMessage::Request { id: 5, .. }
        ));
    }

    #[test]
    fn read_loop_streams_messages_until_eof() {
        let mut bytes = Vec::new();
        write_message(&mut bytes, &json!({"id": 1, "result": 1})).unwrap();
        write_message(&mut bytes, &json!({"method": "note", "params": {}})).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        read_loop(Cursor::new(bytes), tx);
        let msgs: Vec<_> = rx.iter().collect();
        assert_eq!(msgs.len(), 2);
        assert!(matches!(msgs[0], ServerMessage::Response { id: 1, .. }));
        assert!(matches!(msgs[1], ServerMessage::Notification { .. }));
    }
}
