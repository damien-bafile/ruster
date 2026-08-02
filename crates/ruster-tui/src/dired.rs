//! UI state for dired file-explorer buffers.
//!
//! Wraps the headless [`ruster_core::dired`] listing model with the per-buffer
//! caches a session needs — which directory each dired buffer shows, its
//! coloured lines, its entry lookup table — plus the yank clipboard and the
//! `yy` / `dd` / `g` pending-key machines.
//!
//! Everything here takes a `&Workspace` or `&mut Workspace` rather than the
//! `Rc<RefCell<Workspace>>` that `App` holds. Keeping the `RefCell` out of this
//! module turns a double borrow into a compile error instead of a runtime panic.

use std::collections::HashMap;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ruster_core::action::{Action, Motion};
use ruster_core::buffer::Buffer;
use ruster_core::cursor::CursorSet;
use ruster_core::dired as core;
use ruster_core::document::{BufferId, DocKind, SpecialKind};
use ruster_core::editor::EditorView;
use ruster_core::message::{MessageLevel, MessageSource};
use ruster_core::workspace::Workspace;
use ruster_notify::{Notification, NotificationManager};
use ruster_render::{Color, StyledLine, SyntaxStyle};

use crate::file_prompt::{FilePrompt, PromptOrigin};

/// What `App` must do after dired has seen a key.
pub enum DiredResponse {
    /// Not claimed — the caller must fall through to the main key handler, which
    /// is what keeps `:`, `/`, `n`, motions and the leader working in a listing.
    Ignored,
    /// Fully handled inside dired.
    Handled,
    /// Handled; `App` should open this file in a window.
    OpenFile(PathBuf),
    /// Handled; `App` should install this file prompt.
    Prompt(FilePrompt),
    /// `g?` — `App` should show [`help_lines`] in the hover slot.
    ShowHelp,
}

#[derive(Default)]
pub struct DiredState {
    /// The directory each dired buffer is listing.
    dirs: HashMap<BufferId, PathBuf>,
    /// Pre-coloured lines, which override syntax highlighting for these buffers.
    styled: HashMap<BufferId, Vec<StyledLine>>,
    /// Line-index lookup back to the listed entry.
    entries: HashMap<BufferId, Vec<core::DirEntry>>,
    show_hidden: bool,
    /// `(path, is_cut)` awaiting a paste.
    clipboard: Option<(PathBuf, bool)>,
    pending_y: bool,
    pending_d: bool,
    pending_g: bool,
}

impl DiredState {
    /// `show_hidden` seeds from `dired.show_hidden`.
    pub fn new(show_hidden: bool) -> Self {
        Self { show_hidden, ..Default::default() }
    }

    // --- queries ---

    /// Pre-coloured lines for a dired buffer, if it is one.
    pub fn styled_lines(&self, id: BufferId) -> Option<&[StyledLine]> {
        self.styled.get(&id).map(|v| v.as_slice())
    }

    /// The directory `id` is listing.
    pub fn dir_of(&self, id: BufferId) -> Option<&PathBuf> {
        self.dirs.get(&id)
    }

    pub fn show_hidden(&self) -> bool {
        self.show_hidden
    }

    /// Whether the active buffer is a dired listing.
    pub fn active_is_dired(ws: &Workspace) -> bool {
        matches!(ws.active_doc().kind, DocKind::Special(SpecialKind::Dired))
    }

    /// The directory the active dired buffer lists, for resolving a prompt.
    pub fn active_dir(&self, ws: &Workspace) -> PathBuf {
        self.dirs.get(&ws.active_buffer()).cloned().unwrap_or_default()
    }

    /// The `(path, name)` under the cursor, or `None` for `..` / an empty
    /// listing.
    pub fn current_target(&self, ws: &Workspace) -> Option<(PathBuf, String)> {
        let id = ws.active_buffer();
        let dir = self.dirs.get(&id)?;
        let line = ws.buffer().char_to_line(ws.primary_head());
        let entry = self.entries.get(&id)?.get(line)?;
        if entry.name == ".." {
            return None;
        }
        Some((dir.join(&entry.name), entry.name.clone()))
    }

    // --- workspace-mutating ---

    /// Create a dired buffer for `path` and make it active.
    pub fn open(&mut self, ws: &mut Workspace, path: PathBuf) {
        let path = path.canonicalize().unwrap_or(path);
        let id = ws.buffers.create_special(SpecialKind::Dired, &path.to_string_lossy());
        ws.set_active_buffer(id);
        self.refresh(ws, id, path);
    }

    /// Reload `id`'s listing for `path` and reset its window cursor.
    pub fn refresh(&mut self, ws: &mut Workspace, id: BufferId, path: PathBuf) {
        // List once, then derive the text, colours and lookup table from it.
        let entries = core::list(&path, self.show_hidden);
        let text = core::render_entries(&entries);
        self.styled.insert(id, styled_lines(&entries));
        self.entries.insert(id, entries);
        if let Some(doc) = ws.buffers.get_mut(id) {
            doc.buffer = Buffer::from_str(&text);
            doc.name = if core::is_drives_view(&path) {
                "Drives".to_string()
            } else {
                path.to_string_lossy().into_owned()
            };
            doc.modified = false;
        }
        if ws.active_buffer() == id {
            ws.windows.active_window_mut().cursors = CursorSet::single(0);
            ws.windows.active_window_mut().scroll_top = 0;
        }
        self.dirs.insert(id, path);
    }

    /// Reload the active dired buffer (after a mutation).
    pub fn refresh_current(&mut self, ws: &mut Workspace) {
        let id = ws.active_buffer();
        if let Some(dir) = self.dirs.get(&id).cloned() {
            self.refresh(ws, id, dir);
        }
    }

    /// Drop the caches for a closed buffer.
    pub fn forget(&mut self, id: BufferId) {
        self.dirs.remove(&id);
        self.styled.remove(&id);
        self.entries.remove(&id);
    }

    // Test-only views of the pending-key and clipboard machines, so the
    // app-level dispatch tests can assert on them without widening the
    // production surface.
    #[cfg(test)]
    pub(crate) fn pending_y(&self) -> bool {
        self.pending_y
    }

    #[cfg(test)]
    pub(crate) fn clipboard(&self) -> Option<&(PathBuf, bool)> {
        self.clipboard.as_ref()
    }

    // --- input ---

    pub fn handle_key(
        &mut self,
        ck: KeyEvent,
        ws: &mut Workspace,
        notify: &mut NotificationManager,
    ) -> DiredResponse {
        let ctrl = ck.modifiers.contains(KeyModifiers::CONTROL);

        // Pending `yy` copy / `dd` cut: a matching second key completes it.
        if self.pending_y {
            self.pending_y = false;
            if ck.code == KeyCode::Char('y') {
                self.yank_under_cursor(ws, notify, false);
                return DiredResponse::Handled;
            }
        }
        if self.pending_d {
            self.pending_d = false;
            if ck.code == KeyCode::Char('d') {
                self.yank_under_cursor(ws, notify, true);
                return DiredResponse::Handled;
            }
        }
        // Pending `g`: `gg` jumps to the top, `g?` shows help. Handled locally
        // (rather than falling through to vim) so `?` stays free for
        // reverse-search. Any other key is a no-op that ends the prefix.
        if self.pending_g {
            self.pending_g = false;
            return match ck.code {
                KeyCode::Char('g') => {
                    ws.execute(Action::Move(Motion::To(0)));
                    DiredResponse::Handled
                }
                KeyCode::Char('?') => DiredResponse::ShowHelp,
                _ => DiredResponse::Handled,
            };
        }
        // Ctrl chords are decided here and never fall into the plain bindings
        // below, which match on the bare character. Otherwise C-h would read as
        // `h` (ascend a directory) instead of moving window focus, C-l as
        // "open", and C-d as the start of a `dd` cut.
        if ctrl {
            return match ck.code {
                KeyCode::Char('n') => {
                    ws.execute(Action::Move(Motion::Line(1)));
                    DiredResponse::Handled
                }
                KeyCode::Char('p') => {
                    ws.execute(Action::Move(Motion::Line(-1)));
                    DiredResponse::Handled
                }
                // Window focus (C-h/C-l), half-page scroll, C-w — all the main
                // handler's business.
                _ => DiredResponse::Ignored,
            };
        }

        match ck.code {
            KeyCode::Enter | KeyCode::Char('l') => match self.open_at_cursor(ws) {
                Some(path) => DiredResponse::OpenFile(path),
                None => DiredResponse::Handled,
            },
            KeyCode::Char('h') | KeyCode::Char('-') | KeyCode::Char('^') => {
                self.go_up(ws);
                DiredResponse::Handled
            }
            KeyCode::Char('+') => {
                DiredResponse::Prompt(FilePrompt::create(self.active_dir(ws), PromptOrigin::Dired))
            }
            KeyCode::Char('R') => match self.current_target(ws) {
                Some((_, name)) => DiredResponse::Prompt(FilePrompt::rename(
                    self.active_dir(ws),
                    name,
                    PromptOrigin::Dired,
                )),
                None => DiredResponse::Handled,
            },
            KeyCode::Char('D') => match self.current_target(ws) {
                Some((path, _)) => {
                    DiredResponse::Prompt(FilePrompt::delete(path, PromptOrigin::Dired))
                }
                None => DiredResponse::Handled,
            },
            KeyCode::Char('y') => {
                self.pending_y = true;
                DiredResponse::Handled
            }
            KeyCode::Char('d') => {
                self.pending_d = true;
                DiredResponse::Handled
            }
            KeyCode::Char('p') => {
                self.paste(ws, notify);
                DiredResponse::Handled
            }
            KeyCode::Char('.') => {
                self.show_hidden = !self.show_hidden;
                self.refresh_current(ws);
                echo(
                    notify,
                    MessageLevel::Info,
                    format!("Hidden files {}", if self.show_hidden { "shown" } else { "hidden" }),
                );
                DiredResponse::Handled
            }
            // `g` starts the dired prefix (`gg` top, `g?` help).
            KeyCode::Char('g') => {
                self.pending_g = true;
                DiredResponse::Handled
            }
            // Everything else falls through to normal handling. The buffer is
            // read-only (edits are no-ops), so this safely enables `:` commands,
            // `/`/`?`/`n`/`N` search, motions, the Space leader, and — in Emacs
            // mode — `C-s`/`M-x`, all operating over the listing.
            _ => DiredResponse::Ignored,
        }
    }

    /// Enter the entry under the cursor. Returns the file to open, or `None`
    /// when it descended into a directory (or ascended via `..`).
    fn open_at_cursor(&mut self, ws: &mut Workspace) -> Option<PathBuf> {
        let id = ws.active_buffer();
        let dir = self.dirs.get(&id)?.clone();
        let line = ws.buffer().char_to_line(ws.primary_head());
        let entry = self.entries.get(&id).and_then(|e| e.get(line))?.clone();
        // `..` ascends (and, at a drive root, reaches the drive picker).
        if entry.name == ".." {
            self.go_up(ws);
            return None;
        }
        let target = dir.join(&entry.name);
        let target = target.canonicalize().unwrap_or(target);
        if entry.is_dir {
            self.refresh(ws, id, target);
            None
        } else {
            Some(target)
        }
    }

    fn go_up(&mut self, ws: &mut Workspace) {
        let id = ws.active_buffer();
        let Some(dir) = self.dirs.get(&id).cloned() else { return };
        if core::is_drives_view(&dir) {
            return; // already at the top
        }
        let target = match dir.parent() {
            Some(parent) => parent.to_path_buf(),
            // At a drive root (e.g. C:\, whose parent is None) on Windows,
            // ascend to the drive picker instead of staying put.
            None if cfg!(windows) => core::drives_view(),
            None => return,
        };
        self.refresh(ws, id, target);
    }

    /// Record the entry under the cursor for a later paste. `cut` moves on paste.
    fn yank_under_cursor(
        &mut self,
        ws: &Workspace,
        notify: &mut NotificationManager,
        cut: bool,
    ) {
        match self.current_target(ws) {
            Some((path, name)) => {
                self.clipboard = Some((path, cut));
                echo(
                    notify,
                    MessageLevel::Info,
                    format!("{} '{}'", if cut { "Cut" } else { "Copied" }, name),
                );
            }
            None => echo(notify, MessageLevel::Warning, "Nothing selected".to_string()),
        }
    }

    /// Paste the clipboard into the current directory (copy, or move for a cut).
    fn paste(&mut self, ws: &mut Workspace, notify: &mut NotificationManager) {
        let Some((src, cut)) = self.clipboard.clone() else {
            echo(notify, MessageLevel::Info, "Clipboard empty".to_string());
            return;
        };
        let id = ws.active_buffer();
        let Some(dir) = self.dirs.get(&id).cloned() else { return };
        let Some(name) = src.file_name().map(|n| n.to_os_string()) else { return };
        let dest = dir.join(&name);
        if dest.exists() {
            echo(
                notify,
                MessageLevel::Info,
                format!("'{}' already exists", name.to_string_lossy()),
            );
            return;
        }
        let result = if cut {
            // Try a rename first; fall back to copy+remove across filesystems.
            std::fs::rename(&src, &dest).or_else(|_| {
                let copied = if src.is_dir() {
                    copy_dir_recursive(&src, &dest)
                } else {
                    std::fs::copy(&src, &dest).map(|_| ())
                };
                copied.and_then(|()| {
                    if src.is_dir() {
                        std::fs::remove_dir_all(&src)
                    } else {
                        std::fs::remove_file(&src)
                    }
                })
            })
        } else if src.is_dir() {
            copy_dir_recursive(&src, &dest)
        } else {
            std::fs::copy(&src, &dest).map(|_| ())
        };
        match result {
            Ok(()) => {
                echo(
                    notify,
                    MessageLevel::Info,
                    format!("{} '{}'", if cut { "Moved" } else { "Pasted" }, name.to_string_lossy()),
                );
                if cut {
                    self.clipboard = None; // a cut is consumed by the paste
                }
            }
            Err(e) => echo(notify, MessageLevel::Error, format!("Paste failed: {}", e)),
        }
        self.refresh_current(ws);
    }
}

fn echo(notify: &mut NotificationManager, level: MessageLevel, text: String) {
    notify.push(Notification::new(level, MessageSource::Echo, text));
}

/// Recursively copy a directory tree.
fn copy_dir_recursive(src: &std::path::Path, dest: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Colour a directory listing by entry type: directories blue, executables
/// green, symlinks teal, regular files default.
pub fn styled_lines(entries: &[core::DirEntry]) -> Vec<StyledLine> {
    // Colours come from the `dired` pseudo-language in `ruster-syntax`, so they
    // follow the active theme and honour `ruster.config.syntax.dired.*` like
    // every other syntax group.
    entries
        .iter()
        .map(|e| {
            let text = if e.is_dir { format!("{}/", e.name) } else { e.name.clone() };
            let s = if e.is_symlink {
                ruster_syntax::dired_style("symlink")
            } else if e.is_dir {
                ruster_syntax::dired_style("directory")
            } else if e.is_exec {
                ruster_syntax::dired_style("executable")
            } else {
                SyntaxStyle::default()
            };
            let len = text.chars().count();
            let highlights = if matches!(s.fg, Color::Default) { Vec::new() } else { vec![(0, len, s)] };
            StyledLine { text, highlights }
        })
        .collect()
}

/// The dired keymap, shown as a popup by `g?`.
pub fn help_lines() -> Vec<StyledLine> {
    let entries = [
        "Enter / l    open file or enter directory",
        "h / -        parent directory",
        "j / k        move cursor",
        "C-n / C-p    move cursor down / up",
        "yy           copy entry",
        "dd           cut entry",
        "p            paste into this directory",
        "R            rename entry",
        "D            delete entry (confirm)",
        "+            new file, or dir if name ends with /",
        ".            toggle hidden files",
        "/ ? n N      search the listing (as in a normal buffer)",
        ": commands   run any :command",
        "g?           this help",
    ];
    std::iter::once(StyledLine { text: " dired keys".to_string(), highlights: vec![] })
        .chain(entries.iter().map(|e| StyledLine { text: format!("  {}", e), highlights: vec![] }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::Path;

    use std::sync::atomic::{AtomicUsize, Ordering};

    static FIXTURE_COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// A unique temp dir per call containing `names`, so tests stay
    /// parallel-safe.
    fn fixture(names: &[&str]) -> PathBuf {
        let id = FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("ruster_tui_dired_{}", id));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        for n in names {
            if let Some(d) = n.strip_suffix('/') {
                std::fs::create_dir_all(root.join(d)).unwrap();
            } else {
                std::fs::write(root.join(n), "x").unwrap();
            }
        }
        root.canonicalize().unwrap_or(root)
    }

    /// A workspace with a dired buffer open on `root`.
    fn open_on(root: &Path) -> (DiredState, Workspace) {
        let mut ws = Workspace::scratch();
        let mut d = DiredState::new(false);
        d.open(&mut ws, root.to_path_buf());
        (d, ws)
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::from(KeyCode::Char(c))
    }

    fn notify() -> NotificationManager {
        NotificationManager::new(ruster_notify::NoiceSettings::default())
    }

    /// Put the cursor on listing line `line` (0 = "..").
    fn goto_line(ws: &mut Workspace, line: usize) {
        let off = ws.buffer().line_start_char(line);
        ws.execute(Action::Move(Motion::To(off)));
    }

    #[test]
    fn open_lists_the_directory_and_marks_it_dired() {
        let root = fixture(&["b.txt", "a/"]);
        let (d, ws) = open_on(&root);
        assert!(DiredState::active_is_dired(&ws));
        let text = ws.buffer().to_string();
        assert!(text.contains("a/"), "{text}");
        assert!(text.contains("b.txt"), "{text}");
        assert_eq!(d.dir_of(ws.active_buffer()), Some(&root));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn current_target_skips_the_parent_entry() {
        let root = fixture(&["only.txt"]);
        let (d, mut ws) = open_on(&root);
        // Line 0 is "..".
        assert_eq!(d.current_target(&ws), None);
        goto_line(&mut ws, 1);
        let (path, name) = d.current_target(&ws).expect("an entry under the cursor");
        assert_eq!(name, "only.txt");
        assert_eq!(path, root.join("only.txt"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn yy_then_p_copies_into_another_directory() {
        let root = fixture(&["src.txt", "sub/"]);
        let (mut d, mut ws) = open_on(&root);
        let mut n = notify();

        goto_line(&mut ws, 2); // ".." , "sub/", "src.txt"
        assert_eq!(d.current_target(&ws).unwrap().1, "src.txt");
        d.handle_key(key('y'), &mut ws, &mut n);
        d.handle_key(key('y'), &mut ws, &mut n);

        // Descend into sub/ and paste.
        goto_line(&mut ws, 1);
        d.handle_key(KeyEvent::from(KeyCode::Enter), &mut ws, &mut n);
        d.handle_key(key('p'), &mut ws, &mut n);

        assert!(root.join("sub").join("src.txt").is_file(), "copied in");
        assert!(root.join("src.txt").is_file(), "original still there");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn dd_then_p_moves_and_consumes_the_clipboard() {
        let root = fixture(&["move.txt", "sub/"]);
        let (mut d, mut ws) = open_on(&root);
        let mut n = notify();

        goto_line(&mut ws, 2);
        d.handle_key(key('d'), &mut ws, &mut n);
        d.handle_key(key('d'), &mut ws, &mut n);
        goto_line(&mut ws, 1);
        d.handle_key(KeyEvent::from(KeyCode::Enter), &mut ws, &mut n);
        d.handle_key(key('p'), &mut ws, &mut n);

        assert!(root.join("sub").join("move.txt").is_file(), "moved in");
        assert!(!root.join("move.txt").exists(), "original gone");
        assert!(d.clipboard.is_none(), "a cut is consumed by the paste");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_lone_y_does_not_yank() {
        let root = fixture(&["f.txt"]);
        let (mut d, mut ws) = open_on(&root);
        let mut n = notify();
        goto_line(&mut ws, 1);
        d.handle_key(key('y'), &mut ws, &mut n);
        d.handle_key(key('j'), &mut ws, &mut n); // breaks the pending yy
        assert!(d.clipboard.is_none());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn enter_on_a_file_returns_it_for_the_app_to_open() {
        let root = fixture(&["doc.txt"]);
        let (mut d, mut ws) = open_on(&root);
        let mut n = notify();
        goto_line(&mut ws, 1);
        match d.handle_key(KeyEvent::from(KeyCode::Enter), &mut ws, &mut n) {
            DiredResponse::OpenFile(p) => assert_eq!(p, root.join("doc.txt")),
            _ => panic!("expected OpenFile"),
        }
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn enter_on_a_dir_descends_in_place() {
        let root = fixture(&["sub/"]);
        std::fs::write(root.join("sub").join("inner.txt"), "i").unwrap();
        let (mut d, mut ws) = open_on(&root);
        let mut n = notify();
        goto_line(&mut ws, 1);
        d.handle_key(KeyEvent::from(KeyCode::Enter), &mut ws, &mut n);
        assert!(ws.buffer().to_string().contains("inner.txt"));
        assert_eq!(d.dir_of(ws.active_buffer()), Some(&root.join("sub")));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn h_ascends_to_the_parent() {
        let root = fixture(&["sub/"]);
        let (mut d, mut ws) = open_on(&root);
        let mut n = notify();
        goto_line(&mut ws, 1);
        d.handle_key(KeyEvent::from(KeyCode::Enter), &mut ws, &mut n);
        assert_eq!(d.dir_of(ws.active_buffer()), Some(&root.join("sub")));
        d.handle_key(key('h'), &mut ws, &mut n);
        assert_eq!(d.dir_of(ws.active_buffer()), Some(&root));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn dot_toggles_hidden_files_and_relists() {
        let root = fixture(&["visible.txt", ".hidden"]);
        let (mut d, mut ws) = open_on(&root);
        let mut n = notify();
        assert!(!ws.buffer().to_string().contains(".hidden"));
        d.handle_key(key('.'), &mut ws, &mut n);
        assert!(d.show_hidden());
        assert!(ws.buffer().to_string().contains(".hidden"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn g_question_asks_for_help_and_gg_jumps_to_the_top() {
        let root = fixture(&["a.txt", "b.txt"]);
        let (mut d, mut ws) = open_on(&root);
        let mut n = notify();

        d.handle_key(key('g'), &mut ws, &mut n);
        assert!(matches!(
            d.handle_key(KeyEvent::from(KeyCode::Char('?')), &mut ws, &mut n),
            DiredResponse::ShowHelp
        ));

        goto_line(&mut ws, 2);
        d.handle_key(key('g'), &mut ws, &mut n);
        d.handle_key(key('g'), &mut ws, &mut n);
        assert_eq!(ws.primary_head(), 0, "gg jumps to the top");
        std::fs::remove_dir_all(&root).ok();
    }

    /// Unclaimed keys must fall through, or `:`, `/` and the leader stop working
    /// inside a listing.
    #[test]
    fn unhandled_keys_fall_through() {
        let root = fixture(&["a.txt"]);
        let (mut d, mut ws) = open_on(&root);
        let mut n = notify();
        for c in [':', '/', 'n', ' ', 'N'] {
            assert!(
                matches!(d.handle_key(key(c), &mut ws, &mut n), DiredResponse::Ignored),
                "{c:?} must reach the main handler"
            );
        }
        std::fs::remove_dir_all(&root).ok();
    }

    /// The plain bindings match on the bare character, so a Ctrl chord used to
    /// land on them: C-h read as `h` and ascended a directory instead of moving
    /// window focus, making a dired buffer impossible to leave leftward.
    #[test]
    fn ctrl_chords_fall_through_to_the_main_handler() {
        let root = fixture(&["sub/"]);
        let (mut d, mut ws) = open_on(&root);
        let mut n = notify();
        let ctrl = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);

        for c in ['h', 'l', 'd', 'w', 'u'] {
            assert!(
                matches!(d.handle_key(ctrl(c), &mut ws, &mut n), DiredResponse::Ignored),
                "C-{c} must reach the main handler"
            );
        }
        assert_eq!(d.dir_of(ws.active_buffer()), Some(&root), "C-h did not ascend");

        // C-n / C-p remain dired's own line motions.
        assert!(matches!(
            d.handle_key(ctrl('n'), &mut ws, &mut n),
            DiredResponse::Handled
        ));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn forget_drops_the_per_buffer_caches() {
        let root = fixture(&["a.txt"]);
        let (mut d, ws) = open_on(&root);
        let id = ws.active_buffer();
        assert!(d.styled_lines(id).is_some());
        d.forget(id);
        assert!(d.styled_lines(id).is_none());
        assert!(d.dir_of(id).is_none());
        std::fs::remove_dir_all(&root).ok();
    }

}
