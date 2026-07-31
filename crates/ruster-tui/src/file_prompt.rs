//! The mini-buffer prompt for file operations (create / rename / delete).
//!
//! Both the dired listing and the sidebar tree drive the same prompt, so it
//! belongs to neither: `App` owns one `Option<FilePrompt>` and dispatches it
//! ahead of both surfaces' key handlers.
//!
//! The originating surface is recorded in the prompt itself ([`PromptOrigin`])
//! rather than inferred from a side field, and the directory the operation
//! resolves against is captured when the prompt is created. That keeps "which
//! surface asked for this, and where does it apply" from drifting out of sync
//! with the prompt's own lifetime.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent};
use ruster_core::message::MessageLevel;

/// Which surface started the prompt, so the right one is refreshed afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptOrigin {
    Dired,
    Sidebar,
}

/// The operation awaiting confirmation or a name.
#[derive(Debug)]
pub enum FilePromptKind {
    /// Unified create: a trailing `/` makes a directory, otherwise a file.
    Create,
    Rename(String),
    Delete(PathBuf),
}

#[derive(Debug)]
pub struct FilePrompt {
    pub kind: FilePromptKind,
    /// The directory the operation resolves against, captured at creation.
    pub dir: PathBuf,
    pub origin: PromptOrigin,
    pub input: String,
}

/// What the caller should do after feeding a key to the prompt.
#[derive(Debug, PartialEq, Eq)]
pub enum PromptStep {
    /// Still editing — keep the prompt up.
    Pending,
    /// Abandoned; drop the prompt without touching the filesystem.
    Cancelled,
    /// Ready to run; the caller should [`commit`] it and refresh.
    Commit,
}

impl FilePrompt {
    pub fn create(dir: PathBuf, origin: PromptOrigin) -> Self {
        Self { kind: FilePromptKind::Create, dir, origin, input: String::new() }
    }

    pub fn rename(dir: PathBuf, old: String, origin: PromptOrigin) -> Self {
        // Seed the input with the old name so it can be edited in place.
        Self { kind: FilePromptKind::Rename(old.clone()), dir, origin, input: old }
    }

    /// `dir` is taken from `path`'s parent, so the refresh lands where the
    /// deleted entry was.
    pub fn delete(path: PathBuf, origin: PromptOrigin) -> Self {
        let dir = path.parent().unwrap_or(Path::new("")).to_path_buf();
        Self { kind: FilePromptKind::Delete(path), dir, origin, input: String::new() }
    }

    /// The line shown in the mini-buffer.
    pub fn display(&self) -> String {
        match &self.kind {
            FilePromptKind::Create => format!("Create (end with / for dir): {}", self.input),
            FilePromptKind::Rename(old) => format!("Rename '{}' to: {}", old, self.input),
            FilePromptKind::Delete(path) => format!(
                "Delete '{}'? (y/n)",
                path.file_name().and_then(|n| n.to_str()).unwrap_or("")
            ),
        }
    }

    /// Delete is a y/n confirmation rather than a text field.
    pub fn is_confirm(&self) -> bool {
        matches!(self.kind, FilePromptKind::Delete(_))
    }

    pub fn press(&mut self, ck: KeyEvent) -> PromptStep {
        if self.is_confirm() {
            return match ck.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => PromptStep::Commit,
                _ => PromptStep::Cancelled,
            };
        }
        match ck.code {
            KeyCode::Char(c) => {
                self.input.push(c);
                PromptStep::Pending
            }
            KeyCode::Backspace => {
                self.input.pop();
                PromptStep::Pending
            }
            KeyCode::Esc => PromptStep::Cancelled,
            KeyCode::Enter => PromptStep::Commit,
            _ => PromptStep::Pending,
        }
    }
}

/// Perform the filesystem operation, returning the message to report (or `None`
/// when there is nothing to say).
///
/// Deliberately free of editor state — no workspace, no notification manager, no
/// subsystem — so the caller decides what to refresh and this stays testable on
/// its own.
pub fn commit(prompt: &FilePrompt) -> Option<(MessageLevel, String)> {
    let input = prompt.input.trim();
    match &prompt.kind {
        FilePromptKind::Create => {
            if input.is_empty() {
                return None;
            }
            let is_dir = input.ends_with('/');
            let name = input.trim_end_matches('/').to_string();
            if name.is_empty() {
                return Some((MessageLevel::Warning, "No name given".to_string()));
            }
            let target = prompt.dir.join(&name);
            if target.exists() {
                return Some((MessageLevel::Info, format!("'{}' already exists", name)));
            }
            let result = if is_dir {
                std::fs::create_dir_all(&target)
            } else {
                if let Some(parent) = target.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                std::fs::File::create(&target).map(|_| ())
            };
            Some(match result {
                Ok(()) => (
                    MessageLevel::Success,
                    format!("Created {} '{}'", if is_dir { "directory" } else { "file" }, name),
                ),
                Err(e) => (MessageLevel::Error, format!("Create failed: {}", e)),
            })
        }
        FilePromptKind::Rename(old) => {
            if input.is_empty() {
                return None;
            }
            let target = prompt.dir.join(input);
            if target.exists() {
                return Some((MessageLevel::Info, format!("'{}' already exists", input)));
            }
            match std::fs::rename(prompt.dir.join(old), &target) {
                Ok(()) => None,
                Err(e) => Some((MessageLevel::Error, format!("Rename failed: {}", e))),
            }
        }
        FilePromptKind::Delete(path) => {
            let result =
                if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
            Some(match result {
                Ok(()) => (MessageLevel::Success, format!("Deleted '{}'", name)),
                Err(e) => (MessageLevel::Error, format!("Delete failed for '{}': {}", name, e)),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};

    static FIXTURE_COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// A unique empty temp dir per call, so tests stay parallel-safe.
    fn fixture() -> PathBuf {
        let id = FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("ruster_file_prompt_{}", id));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::from(KeyCode::Char(c))
    }

    #[test]
    fn typing_then_enter_creates_a_file() {
        let root = fixture();
        let mut p = FilePrompt::create(root.clone(), PromptOrigin::Dired);
        for c in "hi.txt".chars() {
            assert_eq!(p.press(key(c)), PromptStep::Pending);
        }
        assert_eq!(p.press(KeyEvent::from(KeyCode::Enter)), PromptStep::Commit);
        let (level, msg) = commit(&p).expect("reports the creation");
        assert_eq!(level, MessageLevel::Success);
        assert!(msg.contains("hi.txt"), "{msg}");
        assert!(root.join("hi.txt").is_file());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_trailing_slash_creates_a_directory() {
        let root = fixture();
        let mut p = FilePrompt::create(root.clone(), PromptOrigin::Sidebar);
        for c in "sub/".chars() {
            p.press(key(c));
        }
        let (level, _) = commit(&p).expect("reports the creation");
        assert_eq!(level, MessageLevel::Success);
        assert!(root.join("sub").is_dir());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn backspace_edits_and_esc_cancels() {
        let root = fixture();
        let mut p = FilePrompt::create(root.clone(), PromptOrigin::Dired);
        p.press(key('a'));
        p.press(key('b'));
        p.press(KeyEvent::from(KeyCode::Backspace));
        assert_eq!(p.input, "a");
        assert_eq!(p.press(KeyEvent::from(KeyCode::Esc)), PromptStep::Cancelled);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn create_reports_an_existing_name_without_clobbering_it() {
        let root = fixture();
        std::fs::write(root.join("taken"), "original").unwrap();
        let mut p = FilePrompt::create(root.clone(), PromptOrigin::Dired);
        for c in "taken".chars() {
            p.press(key(c));
        }
        let (level, _) = commit(&p).expect("reports the clash");
        assert_eq!(level, MessageLevel::Info);
        assert_eq!(std::fs::read_to_string(root.join("taken")).unwrap(), "original");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn empty_input_does_nothing() {
        let root = fixture();
        let p = FilePrompt::create(root.clone(), PromptOrigin::Dired);
        assert!(commit(&p).is_none());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn rename_moves_the_file_and_stays_quiet_on_success() {
        let root = fixture();
        std::fs::write(root.join("before"), "x").unwrap();
        let mut p = FilePrompt::rename(root.clone(), "before".to_string(), PromptOrigin::Dired);
        // The old name is pre-filled for editing.
        assert_eq!(p.input, "before");
        p.input.clear();
        for c in "after".chars() {
            p.press(key(c));
        }
        assert_eq!(commit(&p), None);
        assert!(root.join("after").is_file());
        assert!(!root.join("before").exists());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn delete_confirms_with_y_and_cancels_on_anything_else() {
        let root = fixture();
        let victim = root.join("gone");
        std::fs::write(&victim, "x").unwrap();

        let mut p = FilePrompt::delete(victim.clone(), PromptOrigin::Sidebar);
        assert!(p.is_confirm());
        assert_eq!(p.press(key('n')), PromptStep::Cancelled);
        assert!(victim.exists(), "cancelling leaves the file alone");

        assert_eq!(p.press(key('y')), PromptStep::Commit);
        let (level, _) = commit(&p).expect("reports the deletion");
        assert_eq!(level, MessageLevel::Success);
        assert!(!victim.exists());
        std::fs::remove_dir_all(&root).ok();
    }

    /// The delete prompt's refresh directory is the deleted entry's parent, not
    /// the entry itself.
    #[test]
    fn delete_resolves_its_dir_to_the_parent() {
        let root = fixture();
        let p = FilePrompt::delete(root.join("child"), PromptOrigin::Sidebar);
        assert_eq!(p.dir, root);
        std::fs::remove_dir_all(&root).ok();
    }
}
