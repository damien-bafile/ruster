//! Manages one language server per language: spawns lazily, drives the
//! `initialize` handshake, queues document notifications until the server is
//! ready, and routes incoming messages.

use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

use crate::client::LspClient;
use crate::protocol;
use crate::registry::{default_server, ServerConfig};
use crate::transport::ServerMessage;

enum State {
    /// Waiting for the response to the `initialize` request with this id.
    Initializing {
        init_id: i64,
    },
    Ready,
}

struct Managed {
    client: LspClient,
    state: State,
    /// (method, params) notifications queued until the server is `Ready`.
    queued: Vec<(String, Value)>,
}

/// A message from a server, tagged with the language whose server sent it.
pub struct RoutedMessage {
    pub lang: String,
    pub message: ServerMessage,
}

#[derive(Default)]
pub struct LspManager {
    clients: HashMap<String, Managed>,
    overrides: HashMap<String, ServerConfig>,
}

impl LspManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Override (or set) the server command for a language (from Lua config).
    pub fn set_server(&mut self, lang: &str, cfg: ServerConfig) {
        self.overrides.insert(lang.to_string(), cfg);
    }

    fn server_for(&self, lang: &str) -> Option<ServerConfig> {
        self.overrides
            .get(lang)
            .cloned()
            .or_else(|| default_server(lang))
    }

    pub fn has_client(&self, lang: &str) -> bool {
        self.clients.contains_key(lang)
    }

    /// Ensure a server for `lang` is spawned and initializing. Returns whether a
    /// client is now present (false if there's no server or spawning failed).
    pub fn ensure(&mut self, lang: &str, root: &Path) -> bool {
        if self.clients.contains_key(lang) {
            return true;
        }
        let cfg = match self.server_for(lang) {
            Some(c) => c,
            None => return false,
        };
        match LspClient::spawn(&cfg.cmd, &cfg.args, root) {
            Ok(mut client) => {
                let init_id = client.request("initialize", protocol::initialize_params(root));
                self.clients.insert(
                    lang.to_string(),
                    Managed {
                        client,
                        state: State::Initializing { init_id },
                        queued: Vec::new(),
                    },
                );
                true
            }
            Err(_) => false,
        }
    }

    /// Send a notification now if the server is ready, else queue it.
    fn notify_or_queue(&mut self, lang: &str, method: &str, params: Value) {
        if let Some(m) = self.clients.get_mut(lang) {
            match m.state {
                State::Ready => m.client.notify(method, params),
                State::Initializing { .. } => m.queued.push((method.to_string(), params)),
            }
        }
    }

    pub fn did_open(&mut self, lang: &str, uri: &str, language_id: &str, version: i64, text: &str) {
        self.notify_or_queue(
            lang,
            "textDocument/didOpen",
            protocol::did_open_params(uri, language_id, version, text),
        );
    }

    pub fn did_change(&mut self, lang: &str, uri: &str, version: i64, text: &str) {
        self.notify_or_queue(
            lang,
            "textDocument/didChange",
            protocol::did_change_params(uri, version, text),
        );
    }

    pub fn did_close(&mut self, lang: &str, uri: &str) {
        self.notify_or_queue(
            lang,
            "textDocument/didClose",
            protocol::did_close_params(uri),
        );
    }

    /// Send a request to `lang`'s server if ready; returns the request id.
    pub fn request(&mut self, lang: &str, method: &str, params: Value) -> Option<i64> {
        let m = self.clients.get_mut(lang)?;
        if matches!(m.state, State::Ready) {
            Some(m.client.request(method, params))
        } else {
            None
        }
    }

    /// Poll every server. Handles the `initialize` handshake and server-initiated
    /// requests internally; returns the remaining messages (responses to our
    /// requests and notifications like diagnostics) for the app to dispatch.
    pub fn poll(&mut self) -> Vec<RoutedMessage> {
        let mut out = Vec::new();
        for (lang, m) in self.clients.iter_mut() {
            for msg in m.client.poll() {
                match msg {
                    ServerMessage::Response { id, .. } if matches!(m.state, State::Initializing { init_id } if init_id == id) =>
                    {
                        // Handshake complete: announce initialized, flush queue.
                        m.client.notify("initialized", serde_json::json!({}));
                        m.state = State::Ready;
                        for (method, params) in m.queued.drain(..) {
                            m.client.notify(&method, params);
                        }
                    }
                    ServerMessage::Request { id, ref method, .. } => {
                        if let Some(result) = protocol::server_request_reply(method) {
                            m.client.respond(id, result);
                        }
                        // Not surfaced to the app.
                    }
                    other => out.push(RoutedMessage {
                        lang: lang.clone(),
                        message: other,
                    }),
                }
            }
        }
        out
    }

    pub fn shutdown_all(&mut self) {
        for m in self.clients.values_mut() {
            m.client.shutdown();
        }
        self.clients.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_takes_precedence_over_default() {
        let mut mgr = LspManager::new();
        assert_eq!(mgr.server_for("rust").unwrap().cmd, "rust-analyzer");
        mgr.set_server(
            "rust",
            ServerConfig {
                cmd: "my-ra".into(),
                args: vec![],
            },
        );
        assert_eq!(mgr.server_for("rust").unwrap().cmd, "my-ra");
    }

    #[test]
    fn no_server_for_unknown_language() {
        let mgr = LspManager::new();
        assert!(mgr.server_for("brainfuck").is_none());
    }

    #[test]
    fn ensure_is_false_when_no_server_configured() {
        let mut mgr = LspManager::new();
        // No default + no override for this language.
        assert!(!mgr.ensure("brainfuck", Path::new("/tmp")));
        assert!(!mgr.has_client("brainfuck"));
    }
}
