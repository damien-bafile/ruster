//! A minimal LSP client: spawns a language server, writes JSON-RPC
//! requests/notifications, and exposes incoming messages via a non-blocking poll.

use std::io::{self, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, TryIter};

use serde_json::{json, Value};

use crate::transport::{self, ServerMessage};

/// A client connected to one language server over its stdio.
pub struct LspClient {
    child: Option<Child>,
    writer: Box<dyn Write + Send>,
    rx: Receiver<ServerMessage>,
    next_id: i64,
}

impl LspClient {
    /// Spawn `cmd args` as a language server rooted at `root`, wiring its stdio
    /// to a background reader thread.
    pub fn spawn(cmd: &str, args: &[String], root: &Path) -> io::Result<Self> {
        let mut child = Command::new(cmd)
            .args(args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;

        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || transport::read_loop(stdout, tx));

        Ok(LspClient {
            child: Some(child),
            writer: Box::new(stdin),
            rx,
            next_id: 0,
        })
    }

    /// Send a request; returns its id so the caller can match the response.
    pub fn request(&mut self, method: &str, params: Value) -> i64 {
        self.next_id += 1;
        let id = self.next_id;
        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let _ = transport::write_message(&mut self.writer, &msg);
        id
    }

    /// Send a notification (no response expected).
    pub fn notify(&mut self, method: &str, params: Value) {
        let msg = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        let _ = transport::write_message(&mut self.writer, &msg);
    }

    /// Reply to a server-initiated request with a result (or null).
    pub fn respond(&mut self, id: i64, result: Value) {
        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        });
        let _ = transport::write_message(&mut self.writer, &msg);
    }

    /// Drain all messages received so far (non-blocking).
    pub fn poll(&self) -> Vec<ServerMessage> {
        let iter: TryIter<ServerMessage> = self.rx.try_iter();
        iter.collect()
    }

    /// Ask the server to shut down and terminate the process.
    pub fn shutdown(&mut self) {
        self.request("shutdown", Value::Null);
        self.notify("exit", Value::Null);
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    #[cfg(test)]
    fn from_parts(writer: Box<dyn Write + Send>, rx: Receiver<ServerMessage>) -> Self {
        LspClient { child: None, writer, rx, next_id: 0 }
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        // Ensure the server process is not orphaned.
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    // A writer that records everything written, for asserting on sent frames.
    #[derive(Clone)]
    struct SharedBuf(Arc<Mutex<Vec<u8>>>);
    impl Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn request_ids_increment_and_serialize_method() {
        let sink = SharedBuf(Arc::new(Mutex::new(Vec::new())));
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut client = LspClient::from_parts(Box::new(sink.clone()), rx);

        let id1 = client.request("initialize", json!({}));
        let id2 = client.request("textDocument/hover", json!({}));
        assert_eq!((id1, id2), (1, 2));

        let written = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
        assert!(written.contains("initialize"));
        assert!(written.contains("textDocument/hover"));
        assert!(written.contains("Content-Length:"));
    }

    #[test]
    fn poll_drains_incoming_messages() {
        let sink = SharedBuf(Arc::new(Mutex::new(Vec::new())));
        let (tx, rx) = std::sync::mpsc::channel();
        let client = LspClient::from_parts(Box::new(sink), rx);

        tx.send(ServerMessage::Notification { method: "note".into(), params: Value::Null })
            .unwrap();
        tx.send(ServerMessage::Response { id: 1, result: Value::Null, error: None })
            .unwrap();
        let msgs = client.poll();
        assert_eq!(msgs.len(), 2);
        assert!(client.poll().is_empty(), "second poll drains nothing");
    }
}
