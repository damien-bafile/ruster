//! The language-server surface, lifted out of `App`.
//!
//! Four fields moved here — the manager, the per-buffer document sync state,
//! the diagnostics map and the in-flight request table. They were already
//! well-encapsulated (21 touch points across a 7,600-line file), which is
//! exactly why this is the right first extraction: the boundary already
//! existed and only needed naming.
//!
//! **What deliberately did not move**: interpreting a response. Jumping to a
//! definition, opening a symbol picker, applying a rename across files — those
//! are editor effects reaching into windows, pickers and the quickfix list, and
//! pulling them in here would just move the entanglement rather than remove it.
//! This module answers "what did the server say"; `App` decides what to do
//! about it. That is the same split `sidebar` and `dired` already follow.
//!
//! Generic over the pending-action type so `App`'s `LspAction` enum can stay in
//! `app.rs` where it is dispatched. The alternative — moving the enum here —
//! would drag the whole response `match` with it.

use ruster_core::document::BufferId;
use ruster_lsp::{Diagnostic, LspManager, RoutedMessage, ServerKey};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// What the editor last told a server about one buffer.
#[derive(Debug, Clone)]
pub struct LspDoc {
    pub uri: String,
    /// Which server owns this document. Held rather than re-derived so later
    /// requests reach the same process the `didOpen` went to, even after the
    /// active buffer has moved to a different project.
    pub key: ServerKey,
    pub version: i64,
    /// The text last sent. Compared to decide whether a `didChange` is due at
    /// all — without it every frame would send the whole document.
    pub synced: String,
}

/// Outcome of syncing one buffer, so the caller can tell "nothing to do" from
/// "the server now knows about this file".
#[derive(Debug, PartialEq, Eq)]
pub enum Sync {
    /// No server for this language, or nothing worth sending.
    Idle,
    Opened,
    Changed,
    /// The server already has this exact text.
    Unchanged,
}

pub struct LspState<A> {
    manager: LspManager,
    docs: HashMap<BufferId, LspDoc>,
    diagnostics: HashMap<BufferId, Vec<Diagnostic>>,
    /// In-flight requests, keyed by `(server, request id)` — ids are only
    /// unique per server, so the server has to be part of the key.
    pending: HashMap<(ServerKey, i64), A>,
}

impl<A> Default for LspState<A> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A> LspState<A> {
    pub fn new() -> Self {
        LspState {
            manager: LspManager::new(),
            docs: HashMap::new(),
            diagnostics: HashMap::new(),
            pending: HashMap::new(),
        }
    }

    /// Tell the server for `lang` about `buffer`, starting it if needed.
    ///
    /// Sends `didOpen` the first time and `didChange` only when the text
    /// actually differs from what was last sent — this runs every frame, and
    /// an unconditional `didChange` would ship the whole document each time.
    pub fn sync(
        &mut self,
        buffer: BufferId,
        path: &Path,
        lang: &str,
        text: &str,
        root: &Path,
    ) -> Sync {
        if !self.manager.ensure(lang, root) {
            return Sync::Idle;
        }
        // Servers match their index on an absolute URI; a relative path
        // silently resolves to nothing.
        let abs = std::fs::canonicalize(path).unwrap_or_else(|_| {
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                root.join(path)
            }
        });
        let uri = ruster_lsp::protocol::uri_from_path(&abs);
        let key = ServerKey::new(lang, root);
        match self.docs.get_mut(&buffer) {
            None => {
                let language_id = ruster_lsp::registry::language_id(lang).to_string();
                self.manager.did_open(&key, &uri, &language_id, 0, text);
                self.docs
                    .insert(buffer, LspDoc { uri, key, version: 0, synced: text.to_string() });
                Sync::Opened
            }
            Some(doc) if doc.synced != text => {
                doc.version += 1;
                let (version, uri, key) = (doc.version, doc.uri.clone(), doc.key.clone());
                doc.synced = text.to_string();
                self.manager.did_change(&key, &uri, version, text);
                Sync::Changed
            }
            Some(_) => Sync::Unchanged,
        }
    }

    /// Send a request, recording `action` so the reply can be interpreted.
    ///
    /// Returns whether it went out: no server for that language means no
    /// request, and the caller should not wait for an answer.
    pub fn request(
        &mut self,
        key: &ServerKey,
        method: &str,
        params: serde_json::Value,
        action: A,
    ) -> bool {
        match self.manager.request(key, method, params) {
            Some(id) => {
                self.pending.insert((key.clone(), id), action);
                true
            }
            None => false,
        }
    }

    /// Take the action recorded for a reply, if this was a request we sent.
    pub fn take_pending(&mut self, key: &ServerKey, id: i64) -> Option<A> {
        self.pending.remove(&(key.clone(), id))
    }

    /// Override the command used for a language, from `ruster.lsp.servers`.
    /// Must be called before the server for that language is first started.
    pub fn set_server(&mut self, lang: &str, cfg: ruster_lsp::ServerConfig) {
        self.manager.set_server(lang, cfg);
    }

    pub fn poll(&mut self) -> Vec<RoutedMessage> {
        self.manager.poll()
    }

    pub fn shutdown_all(&mut self) {
        self.manager.shutdown_all();
    }

    /// Whether this buffer has been registered with a server — the test for
    /// "can this file have LSP actions run on it".
    pub fn is_tracked(&self, buffer: BufferId) -> bool {
        self.docs.contains_key(&buffer)
    }

    pub fn doc(&self, buffer: BufferId) -> Option<&LspDoc> {
        self.docs.get(&buffer)
    }

    pub fn diagnostics(&self, buffer: BufferId) -> &[Diagnostic] {
        self.diagnostics.get(&buffer).map(Vec::as_slice).unwrap_or_default()
    }

    pub fn set_diagnostics(&mut self, buffer: BufferId, diags: Vec<Diagnostic>) {
        self.diagnostics.insert(buffer, diags);
    }

    /// Every buffer with diagnostics — for the Trouble list and the quickfix.
    pub fn all_diagnostics(&self) -> impl Iterator<Item = (BufferId, &[Diagnostic])> {
        self.diagnostics.iter().map(|(b, d)| (*b, d.as_slice()))
    }

    /// Drop everything held for a closed buffer.
    ///
    /// Both maps, not one. They were cleared together at the single call site
    /// before this existed, and keeping them in step is now this module's job
    /// rather than something each caller has to remember.
    pub fn forget(&mut self, buffer: BufferId) {
        self.docs.remove(&buffer);
        self.diagnostics.remove(&buffer);
    }

    /// The workspace root a server should be started in: the root of the
    /// project `path` belongs to, falling back to the process cwd.
    ///
    /// This has to follow the *file*, not the process. A server initialised
    /// against a directory the file does not live under loads a workspace the
    /// file is not part of, and then answers every request with `null` — so
    /// the editor reports "No hover info" and looks like it lacks the feature
    /// rather than like it is pointed at the wrong tree. Editing anything
    /// outside the directory ruster was launched from used to hit exactly
    /// that.
    ///
    /// Servers are still keyed by language alone, so the first project opened
    /// in a session owns the server for its language; a file from a second
    /// project reuses it.
    pub fn root_for(path: &Path) -> PathBuf {
        ruster_project::project_root(path)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in for `App`'s `LspAction`, to prove the generic really is one.
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    enum TestAction {
        Hover,
        Rename,
    }

    fn state() -> LspState<TestAction> {
        LspState::new()
    }

    /// The root used to be `current_dir()`, so opening a file from anywhere
    /// other than the launch directory initialised the server against a tree
    /// the file was not in. rust-analyzer then answered every request with
    /// `null` and the only symptom was "No hover info".
    #[test]
    fn the_server_root_follows_the_file_not_the_cwd() {
        let tmp = std::env::temp_dir().join(format!("ruster_lsproot_{}", std::process::id()));
        let src = tmp.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(tmp.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        let file = src.join("main.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();

        let root = LspState::<TestAction>::root_for(&file);
        assert_eq!(root.canonicalize().unwrap(), tmp.canonicalize().unwrap());
        // The cwd during tests is the crate root, which is *not* this project.
        assert_ne!(root.canonicalize().unwrap(), std::env::current_dir().unwrap());

        std::fs::remove_dir_all(&tmp).ok();
    }

    /// A file belonging to no project still has to produce a usable root.
    #[test]
    fn a_file_outside_any_project_falls_back_to_the_cwd() {
        let root = LspState::<TestAction>::root_for(Path::new("/nonexistent-xyz/stray.rs"));
        assert_eq!(root, std::env::current_dir().unwrap());
    }

    #[test]
    fn diagnostics_default_to_empty_rather_than_absent() {
        // Callers render a gutter from this every frame; an `Option` there
        // bought nothing but a `unwrap_or_default` at each site.
        let mut s = state();
        let buf = BufferId(1);
        assert!(s.diagnostics(buf).is_empty());
        s.set_diagnostics(buf, vec![diag(1)]);
        assert_eq!(s.diagnostics(buf).len(), 1);
    }

    #[test]
    fn forgetting_a_buffer_clears_both_maps() {
        let mut s = state();
        let buf = BufferId(7);
        s.set_diagnostics(buf, vec![diag(1)]);
        s.docs.insert(
            buf,
            LspDoc {
                uri: "file:///x".into(),
                key: ServerKey::new("rust", Path::new("/proj")),
                version: 0,
                synced: String::new(),
            },
        );
        s.forget(buf);
        assert!(s.diagnostics(buf).is_empty(), "diagnostics survived");
        assert!(!s.is_tracked(buf), "the document registration survived");
    }

    #[test]
    fn a_pending_action_is_keyed_by_server_as_well_as_id() {
        // Request ids are only unique per server. Two servers will both issue
        // id 1, and keying on the id alone would hand rust-analyzer's reply to
        // whatever pyright asked for.
        let mut s = state();
        let rust = ServerKey::new("rust", Path::new("/proj"));
        let python = ServerKey::new("python", Path::new("/proj"));
        s.pending.insert((rust.clone(), 1), TestAction::Hover);
        s.pending.insert((python.clone(), 1), TestAction::Rename);
        assert_eq!(s.take_pending(&rust, 1), Some(TestAction::Hover));
        assert_eq!(s.take_pending(&python, 1), Some(TestAction::Rename));
    }

    /// Same language, two projects: two servers, each numbering from 1. The
    /// root has to be part of the key or one project's reply is applied to the
    /// other's request.
    #[test]
    fn two_projects_in_one_language_do_not_share_a_reply_slot() {
        let mut s = state();
        let a = ServerKey::new("rust", Path::new("/a"));
        let b = ServerKey::new("rust", Path::new("/b"));
        assert_ne!(a, b);
        s.pending.insert((a.clone(), 1), TestAction::Hover);
        s.pending.insert((b.clone(), 1), TestAction::Rename);
        assert_eq!(s.take_pending(&a, 1), Some(TestAction::Hover));
        assert_eq!(s.take_pending(&b, 1), Some(TestAction::Rename));
    }

    #[test]
    fn taking_a_pending_action_consumes_it() {
        let mut s = state();
        let rust = ServerKey::new("rust", Path::new("/proj"));
        s.pending.insert((rust.clone(), 1), TestAction::Hover);
        assert_eq!(s.take_pending(&rust, 1), Some(TestAction::Hover));
        assert_eq!(s.take_pending(&rust, 1), None, "a reply must not fire twice");
    }

    #[test]
    fn an_unknown_reply_is_ignored_rather_than_guessed() {
        let mut s = state();
        assert_eq!(s.take_pending(&ServerKey::new("rust", Path::new("/proj")), 99), None);
    }

    #[test]
    fn syncing_without_a_server_is_idle_not_a_panic() {
        // No language server for a made-up language: the editor must carry on.
        let mut s = state();
        let root = std::env::temp_dir();
        let r = s.sync(BufferId(1), Path::new("f.zzz"), "nosuchlang", "text", &root);
        assert_eq!(r, Sync::Idle);
        assert!(!s.is_tracked(BufferId(1)), "nothing was registered");
    }

    #[test]
    fn all_diagnostics_sees_every_buffer() {
        let mut s = state();
        s.set_diagnostics(BufferId(1), vec![diag(1)]);
        s.set_diagnostics(BufferId(2), vec![diag(2), diag(1)]);
        let total: usize = s.all_diagnostics().map(|(_, d)| d.len()).sum();
        assert_eq!(total, 3);
        assert_eq!(s.all_diagnostics().count(), 2);
    }

    fn diag(severity: u8) -> Diagnostic {
        Diagnostic {
            start: ruster_lsp::results::LspPositionEq { line: 0, character: 0 },
            end: ruster_lsp::results::LspPositionEq { line: 0, character: 1 },
            severity,
            message: "x".into(),
        }
    }
}
