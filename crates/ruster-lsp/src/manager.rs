//! Manages one language server per language: spawns lazily, drives the
//! `initialize` handshake, queues document notifications until the server is
//! ready, and routes incoming messages.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::client::LspClient;
use crate::protocol;
use crate::registry::{default_server, ServerConfig};
use crate::transport::ServerMessage;

/// Identifies one server process: a language, in a project root.
///
/// The root is part of the identity, not a start-up detail. A server is
/// initialised against exactly one workspace and answers `null` for anything
/// outside it, so two projects open at once need two servers even when they
/// share a language — keying on the language alone silently pointed the second
/// project's files at the first project's server.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerKey {
    pub lang: String,
    pub root: PathBuf,
}

impl ServerKey {
    pub fn new(lang: &str, root: &Path) -> Self {
        ServerKey {
            lang: lang.to_string(),
            root: root.to_path_buf(),
        }
    }
}

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

/// A message from a server, tagged with the server that sent it.
pub struct RoutedMessage {
    pub key: ServerKey,
    pub message: ServerMessage,
}

#[derive(Default)]
pub struct LspManager {
    clients: HashMap<ServerKey, Managed>,
    /// Command overrides stay keyed by language: `ruster.lsp.servers` names a
    /// program per language, not per project.
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

    pub fn has_client(&self, key: &ServerKey) -> bool {
        self.clients.contains_key(key)
    }

    /// Whether any server is running at all.
    ///
    /// The event loop asks: servers deliver over an mpsc channel rather than a
    /// pollable fd, so their messages are the one thing that only arrives when
    /// something thinks to look. With no server running there is nothing to
    /// look for, and the loop can sleep until a real event source wakes it.
    pub fn has_servers(&self) -> bool {
        !self.clients.is_empty()
    }

    /// Ensure a server for `lang` rooted at `root` is spawned and initializing.
    /// Returns whether a client is now present (false if there's no server for
    /// the language or spawning failed).
    pub fn ensure(&mut self, lang: &str, root: &Path) -> bool {
        let key = ServerKey::new(lang, root);
        if self.clients.contains_key(&key) {
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
                    key,
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
    fn notify_or_queue(&mut self, key: &ServerKey, method: &str, params: Value) {
        if let Some(m) = self.clients.get_mut(key) {
            match m.state {
                State::Ready => m.client.notify(method, params),
                State::Initializing { .. } => m.queued.push((method.to_string(), params)),
            }
        }
    }

    pub fn did_open(
        &mut self,
        key: &ServerKey,
        uri: &str,
        language_id: &str,
        version: i64,
        text: &str,
    ) {
        self.notify_or_queue(
            key,
            "textDocument/didOpen",
            protocol::did_open_params(uri, language_id, version, text),
        );
    }

    pub fn did_change(&mut self, key: &ServerKey, uri: &str, version: i64, text: &str) {
        self.notify_or_queue(
            key,
            "textDocument/didChange",
            protocol::did_change_params(uri, version, text),
        );
    }

    pub fn did_close(&mut self, key: &ServerKey, uri: &str) {
        self.notify_or_queue(
            key,
            "textDocument/didClose",
            protocol::did_close_params(uri),
        );
    }

    /// Send a request to that server if ready; returns the request id.
    pub fn request(&mut self, key: &ServerKey, method: &str, params: Value) -> Option<i64> {
        let m = self.clients.get_mut(key)?;
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
        for (key, m) in self.clients.iter_mut() {
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
                        key: key.clone(),
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
        assert!(!mgr.has_client(&ServerKey::new("brainfuck", Path::new("/tmp"))));
    }

    /// A server is identified by its root as well as its language: two
    /// projects open in one session get one server each, and a file from the
    /// second is no longer sent to the first — which answers `null` for
    /// anything outside the workspace it loaded.
    #[test]
    fn the_same_language_in_two_projects_is_two_servers() {
        let a = ServerKey::new("rust", Path::new("/a"));
        let b = ServerKey::new("rust", Path::new("/b"));
        assert_ne!(a, b);

        let mut mgr = LspManager::new();
        let (_tx, rx) = std::sync::mpsc::channel();
        mgr.clients.insert(
            a.clone(),
            Managed {
                client: LspClient::from_parts(Box::new(std::io::sink()), rx),
                state: State::Ready,
                queued: Vec::new(),
            },
        );
        assert!(mgr.has_client(&a));
        assert!(
            !mgr.has_client(&b),
            "the second project must not reuse the first's server"
        );
        assert!(
            !mgr.has_client(&ServerKey::new("python", Path::new("/a"))),
            "nor may another language in the same root"
        );
    }
}
