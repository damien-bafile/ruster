//! UI state for the file-explorer side panel.
//!
//! Wraps the headless [`SidebarTree`](ruster_core::sidebar::SidebarTree) with
//! the things a terminal needs: which row is selected, how far it is scrolled,
//! whether it has focus, how wide it is, and the `gg` pending-key machine.
//!
//! Keys that need the rest of the editor — opening a file, arming a file prompt
//! — come back as a [`SidebarResponse`] for `App` to act on, rather than the
//! panel reaching back into `App` itself.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ruster_core::sidebar::{SidebarRow, SidebarTree};
use ruster_core::windows::Rect as CoreRect;
use ruster_render::{
    Rect as RRect, StatusSection, StatuslineView, StyledLine, SyntaxStyle, Theme, UIMode,
    WindowView,
};

use crate::file_prompt::{FilePrompt, PromptOrigin};

/// Rows kept above the selection when revealing a path, so it doesn't land hard
/// against the top edge.
const REVEAL_CONTEXT: usize = 8;

/// Whether `m` represents an unmodified keypress.
///
/// Terminals report an uppercase letter as the character *plus* SHIFT, so a
/// literal `is_empty()` check silently rejects bindings like `G` and `R`. SHIFT
/// is part of the character here, not a chord.
fn is_plain(m: KeyModifiers) -> bool {
    m.difference(KeyModifiers::SHIFT).is_empty()
}

/// What `App` must do after the sidebar has seen a key.
pub enum SidebarResponse {
    /// Not claimed — the caller must fall through to the main key handler.
    Ignored,
    /// Fully handled inside the sidebar.
    Handled,
    /// Handled; `App` should open this file.
    OpenFile(PathBuf),
    /// Handled; `App` should install this file prompt.
    Prompt(FilePrompt),
    /// Handled; `App` should close the sidebar.
    Close,
}

pub struct SidebarState {
    /// `None` when hidden. Width and focus outlive visibility, so this is an
    /// inner `Option` rather than the whole struct being optional.
    tree: Option<SidebarTree>,
    selected: usize,
    scroll: usize,
    focused: bool,
    width: u16,
    /// Set after a lone `g`, so the next `g` completes a `gg` jump-to-top.
    pending_g: bool,
}

impl Default for SidebarState {
    fn default() -> Self {
        Self {
            tree: None,
            selected: 0,
            scroll: 0,
            focused: false,
            width: 30,
            pending_g: false,
        }
    }
}

impl SidebarState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_open(&self) -> bool {
        self.tree.is_some()
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    pub fn set_focused(&mut self, v: bool) {
        self.focused = v;
        if !v {
            self.pending_g = false;
        }
    }

    /// Show the panel rooted at `root`, resetting position and taking focus.
    pub fn open(&mut self, root: PathBuf) {
        self.tree = Some(SidebarTree::new(root, false));
        self.selected = 0;
        self.scroll = 0;
        self.focused = true;
        self.pending_g = false;
    }

    pub fn close(&mut self) {
        self.tree = None;
        self.focused = false;
        self.pending_g = false;
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn set_width(&mut self, n: u16) {
        self.width = n.clamp(16, 60);
    }

    pub fn rows(&self) -> Vec<SidebarRow> {
        self.tree.as_ref().map(|t| t.rows()).unwrap_or_default()
    }

    /// Re-read the tree from disk, keeping the selection in range.
    pub fn refresh(&mut self) {
        if let Some(tree) = self.tree.as_mut() {
            tree.refresh();
            let len = tree.rows().len();
            self.selected = self.selected.min(len.saturating_sub(1));
        }
    }

    /// Carve the sidebar column off the left of `area`, shrinking `area` in
    /// place. Returns `None` when hidden.
    ///
    /// This runs before the window rects are computed and before `last_layout`
    /// is rebuilt, so the mouse hit-test sees the same offset the user does.
    pub fn carve(&self, area: &mut CoreRect) -> Option<CoreRect> {
        if !self.is_open() {
            return None;
        }
        let w = self.width.min(area.width.saturating_sub(4));
        let rect = CoreRect::new(area.x, area.y, w, area.height);
        area.x += w;
        area.width = area.width.saturating_sub(w);
        Some(rect)
    }

    /// Expand every ancestor of `path`, select it, and scroll it into view.
    /// No-op when hidden or when `path` lies outside the tree's root.
    pub fn reveal(&mut self, path: &Path) {
        let Some(tree) = self.tree.as_mut() else {
            return;
        };
        let path = if path.is_relative() {
            std::env::current_dir()
                .ok()
                .map(|cwd| cwd.join(path))
                .unwrap_or_else(|| path.to_path_buf())
        } else {
            path.to_path_buf()
        };
        if !path.starts_with(&tree.root) {
            return;
        }
        tree.reveal(&path);
        if let Some(idx) = tree.rows().iter().position(|r| r.path == path) {
            self.selected = idx;
            self.scroll = idx.saturating_sub(REVEAL_CONTEXT);
        }
    }

    pub fn handle_key(&mut self, ck: KeyEvent) -> SidebarResponse {
        // `q` closes, which drops the tree — decided before borrowing it.
        if matches!(ck.code, KeyCode::Char('q')) && is_plain(ck.modifiers) {
            return SidebarResponse::Close;
        }
        // Enter on a file opens it; Enter on a directory toggles it below.
        if matches!(ck.code, KeyCode::Enter) && is_plain(ck.modifiers) {
            let target = self.tree.as_ref().and_then(|t| {
                let rows = t.rows();
                if rows.is_empty() {
                    return None;
                }
                let r = &rows[self.selected.min(rows.len() - 1)];
                (!r.is_dir).then(|| r.path.clone())
            });
            if let Some(path) = target {
                self.focused = false;
                return SidebarResponse::OpenFile(path);
            }
        }

        let Some(tree) = self.tree.as_mut() else {
            return SidebarResponse::Ignored;
        };
        let rows = tree.rows();
        if rows.is_empty() {
            self.focused = false;
            return SidebarResponse::Ignored;
        }

        let mut prompt = None;
        let handled = match ck.code {
            KeyCode::Char('j') | KeyCode::Down if is_plain(ck.modifiers) => {
                if self.selected + 1 < rows.len() {
                    self.selected += 1;
                }
                true
            }
            KeyCode::Char('k') | KeyCode::Up if is_plain(ck.modifiers) => {
                self.selected = self.selected.saturating_sub(1);
                true
            }
            KeyCode::Enter if is_plain(ck.modifiers) => {
                let row = &rows[self.selected];
                if row.is_dir {
                    tree.toggle(&row.path);
                }
                true
            }
            KeyCode::Char('h') | KeyCode::Left if is_plain(ck.modifiers) => {
                let row = &rows[self.selected];
                if row.is_dir && row.expanded {
                    tree.collapse(&row.path);
                } else if let Some(parent_depth) = row.depth.checked_sub(1) {
                    // Hop up to the enclosing directory's row.
                    for i in (0..self.selected).rev() {
                        if rows[i].depth == parent_depth {
                            self.selected = i;
                            break;
                        }
                    }
                }
                true
            }
            KeyCode::Char('l') | KeyCode::Right if is_plain(ck.modifiers) => {
                let row = &rows[self.selected];
                if row.is_dir {
                    tree.expand(&row.path);
                }
                true
            }
            KeyCode::Esc | KeyCode::Char('c') if ck.modifiers.contains(KeyModifiers::CONTROL) => {
                self.focused = false;
                true
            }
            KeyCode::Tab => {
                self.focused = false;
                true
            }
            KeyCode::Char('h') | KeyCode::Char('l') if ck.modifiers == KeyModifiers::CONTROL => {
                self.focused = false;
                true
            }
            KeyCode::Char('a') if is_plain(ck.modifiers) => {
                let row = &rows[self.selected.min(rows.len() - 1)];
                // New entries land inside a directory, or beside a file.
                let dir = if row.is_dir {
                    row.path.clone()
                } else {
                    row.path.parent().unwrap_or(&row.path).to_path_buf()
                };
                prompt = Some(FilePrompt::create(dir, PromptOrigin::Sidebar));
                true
            }
            KeyCode::Char('r') if is_plain(ck.modifiers) => {
                let row = &rows[self.selected.min(rows.len() - 1)];
                let dir = row.path.parent().unwrap_or(&row.path).to_path_buf();
                let name = row
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                prompt = Some(FilePrompt::rename(dir, name, PromptOrigin::Sidebar));
                true
            }
            KeyCode::Char('d') if is_plain(ck.modifiers) => {
                let row = &rows[self.selected.min(rows.len() - 1)];
                prompt = Some(FilePrompt::delete(row.path.clone(), PromptOrigin::Sidebar));
                true
            }
            KeyCode::Char('g') if is_plain(ck.modifiers) => {
                if self.pending_g {
                    self.selected = 0;
                    self.pending_g = false;
                } else {
                    self.pending_g = true;
                }
                true
            }
            KeyCode::Char('G') if is_plain(ck.modifiers) => {
                self.selected = rows.len() - 1;
                self.pending_g = false;
                true
            }
            KeyCode::Char('.') if is_plain(ck.modifiers) => {
                tree.set_show_hidden(!tree.show_hidden());
                true
            }
            KeyCode::Char('R') if is_plain(ck.modifiers) => {
                tree.refresh();
                true
            }
            _ => false,
        };

        // Any key other than `g` breaks a pending `gg`.
        if !matches!(ck.code, KeyCode::Char('g')) {
            self.pending_g = false;
        }
        let len = tree.rows().len();
        if len > 0 {
            self.selected = self.selected.min(len - 1);
        }

        match (handled, prompt) {
            (_, Some(p)) => SidebarResponse::Prompt(p),
            (true, None) => SidebarResponse::Handled,
            (false, None) => SidebarResponse::Ignored,
        }
    }

    /// The panel as an ordinary [`WindowView`].
    ///
    /// Deliberately not its own view type: the sidebar reaches the GUI backend
    /// purely by being a window in `FrameState`, so introducing a bespoke view
    /// would mean a second draw path and a parity regression.
    pub fn view(&self, rect: CoreRect, mode: UIMode, theme: &Theme) -> WindowView {
        let rows = self.rows();
        let selected = self.selected.min(rows.len().saturating_sub(1));
        let scroll = self
            .scroll
            .min(selected.saturating_sub((rect.height as usize).saturating_sub(2) / 2));
        let lines: Vec<StyledLine> = rows
            .iter()
            .enumerate()
            .skip(scroll)
            .take(rect.height as usize)
            .map(|(i, r)| {
                let indent = "  ".repeat(r.depth);
                let marker = if r.is_dir {
                    if r.expanded {
                        "▾ "
                    } else {
                        "▸ "
                    }
                } else {
                    "  "
                };
                let mut text = format!("{}{}{}", indent, marker, r.name);
                let highlights = if i == selected {
                    // Pad out to the panel width so the selection reads as a
                    // full row, the way every other list highlights. Stopping at
                    // the end of the name also exposed a rendering mismatch: the
                    // GUI draws text with measured advances but fills highlight
                    // rects at a fixed `char_w`, so a row led by the ▸/▾ marker
                    // drifted and clipped its last character.
                    let width = rect.width as usize;
                    let len = text.chars().count();
                    if len < width {
                        text.push_str(&" ".repeat(width - len));
                    }
                    // Offsets are char indices, not bytes — the ▸/▾ markers are
                    // three bytes each, so `len()` would overshoot.
                    vec![(
                        0,
                        text.chars().count(),
                        SyntaxStyle {
                            fg: theme.selection_fg,
                            bg: theme.selection_bg,
                            bold: false,
                            italic: false,
                        },
                    )]
                } else {
                    vec![]
                };
                StyledLine { text, highlights }
            })
            .collect();
        // No cursor, gutter or flash labels — the rest comes from Default.
        WindowView {
            rect: RRect::new(rect.x, rect.y, rect.width, rect.height),
            // Without this the shared window header falls back to "untitled".
            // The root's own name says which project is open.
            header: self
                .tree
                .as_ref()
                .and_then(|t| t.root.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Explorer".to_string()),
            lines,
            statusline: StatuslineView {
                left: vec![StatusSection::new("mode", "Sidebar")],
                center: Vec::new(),
                right: vec![StatusSection::new(
                    "position",
                    format!("{} items", rows.len()),
                )],
                active: self.focused,
                mode,
            },
            active: self.focused,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};

    static FIXTURE_COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// `root/{a/{x.txt}, b.txt}` in a unique temp dir per call.
    fn fixture() -> PathBuf {
        let id = FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("ruster_tui_sidebar_{}", id));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::write(root.join("a").join("x.txt"), "x").unwrap();
        std::fs::write(root.join("b.txt"), "b").unwrap();
        root
    }

    fn open_on(root: &Path) -> SidebarState {
        let mut s = SidebarState::new();
        s.open(root.to_path_buf());
        s
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::from(KeyCode::Char(c))
    }

    #[test]
    fn width_survives_close_and_reopen() {
        let root = fixture();
        let mut s = open_on(&root);
        s.set_width(45);
        s.close();
        assert!(!s.is_open());
        s.open(root.clone());
        assert_eq!(
            s.width(),
            45,
            "width is panel config, not per-session state"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn set_width_clamps_to_the_usable_range() {
        let mut s = SidebarState::new();
        s.set_width(2);
        assert_eq!(s.width(), 16);
        s.set_width(500);
        assert_eq!(s.width(), 60);
    }

    #[test]
    fn carve_takes_a_column_off_the_left() {
        let root = fixture();
        let mut s = open_on(&root);
        s.set_width(20);
        let mut area = CoreRect::new(0, 0, 100, 24);
        let rect = s.carve(&mut area).expect("open sidebar carves");
        assert_eq!((rect.x, rect.width), (0, 20));
        assert_eq!((area.x, area.width), (20, 80), "buffer area shifts right");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn carve_is_a_no_op_when_hidden() {
        let s = SidebarState::new();
        let mut area = CoreRect::new(0, 0, 100, 24);
        assert!(s.carve(&mut area).is_none());
        assert_eq!((area.x, area.width), (0, 100));
    }

    #[test]
    fn jk_moves_and_clamps_at_both_ends() {
        let root = fixture();
        let mut s = open_on(&root);
        // Rows: "a" (dir), "b.txt".
        assert_eq!(s.selected, 0);
        s.handle_key(key('k'));
        assert_eq!(s.selected, 0, "clamped at the top");
        s.handle_key(key('j'));
        assert_eq!(s.selected, 1);
        s.handle_key(key('j'));
        assert_eq!(s.selected, 1, "clamped at the bottom");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn gg_needs_two_presses_and_any_other_key_breaks_it() {
        let root = fixture();
        let mut s = open_on(&root);
        s.handle_key(key('G'));
        assert_eq!(s.selected, 1);

        s.handle_key(key('g'));
        assert_eq!(s.selected, 1, "one g only arms the sequence");
        s.handle_key(key('g'));
        assert_eq!(s.selected, 0, "gg jumps to the top");

        // A stray key between the two g's cancels it.
        s.handle_key(key('G'));
        s.handle_key(key('g'));
        s.handle_key(key('j'));
        s.handle_key(key('g'));
        assert_ne!(s.selected, 0, "interrupted gg does not jump");
        std::fs::remove_dir_all(&root).ok();
    }

    /// Terminals send an uppercase letter with SHIFT held, so a guard of
    /// `modifiers.is_empty()` silently dropped `G` and `R`.
    #[test]
    fn shifted_keys_are_still_plain_keys() {
        let root = fixture();
        let mut s = open_on(&root);
        let shifted = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT);

        assert_eq!(s.selected, 0);
        assert!(matches!(
            s.handle_key(shifted('G')),
            SidebarResponse::Handled
        ));
        assert_eq!(s.selected, 1, "G jumps to the last row even with SHIFT set");

        // R (refresh) shares the guard; it must be claimed, not fall through.
        assert!(matches!(
            s.handle_key(shifted('R')),
            SidebarResponse::Handled
        ));
        std::fs::remove_dir_all(&root).ok();
    }

    /// SHIFT is part of the character, but a real chord is not: C-h belongs to
    /// the window-focus handler.
    #[test]
    fn ctrl_chords_are_not_treated_as_plain_keys() {
        let root = fixture();
        let mut s = open_on(&root);
        s.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
        assert!(!s.is_focused(), "C-l moves focus out rather than expanding");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn l_expands_a_dir_and_h_collapses_it() {
        let root = fixture();
        let mut s = open_on(&root);
        assert_eq!(s.rows().len(), 2);
        s.handle_key(key('l'));
        assert_eq!(s.rows().len(), 3, "x.txt revealed under a/");
        s.handle_key(key('h'));
        assert_eq!(s.rows().len(), 2);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn h_on_a_child_hops_to_its_parent_row() {
        let root = fixture();
        let mut s = open_on(&root);
        s.handle_key(key('l')); // expand a/
        s.handle_key(key('j')); // onto x.txt (depth 1)
        assert_eq!(s.rows()[s.selected].name, "x.txt");
        s.handle_key(key('h'));
        assert_eq!(s.rows()[s.selected].name, "a", "hopped up to the parent");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn enter_on_a_file_asks_the_app_to_open_it_and_drops_focus() {
        let root = fixture();
        let mut s = open_on(&root);
        s.handle_key(key('j')); // onto b.txt
        match s.handle_key(KeyEvent::from(KeyCode::Enter)) {
            SidebarResponse::OpenFile(p) => assert_eq!(p, root.join("b.txt")),
            _ => panic!("expected OpenFile"),
        }
        assert!(!s.is_focused(), "focus moves to the opened buffer");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn file_operation_keys_return_a_prompt_rooted_correctly() {
        let root = fixture();
        let mut s = open_on(&root);
        // On the directory `a`, a new entry goes inside it.
        match s.handle_key(key('a')) {
            SidebarResponse::Prompt(p) => assert_eq!(p.dir, root.join("a")),
            _ => panic!("expected a create prompt"),
        }
        // On the file `b.txt`, a new entry goes beside it.
        s.handle_key(key('j'));
        match s.handle_key(key('a')) {
            SidebarResponse::Prompt(p) => assert_eq!(p.dir, root),
            _ => panic!("expected a create prompt"),
        }
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn q_asks_to_close_and_unknown_keys_fall_through() {
        let root = fixture();
        let mut s = open_on(&root);
        assert!(matches!(s.handle_key(key('q')), SidebarResponse::Close));
        assert!(
            matches!(s.handle_key(key('z')), SidebarResponse::Ignored),
            "unclaimed keys must reach the main handler"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn dot_toggles_hidden_files() {
        let root = fixture();
        std::fs::write(root.join(".secret"), "s").unwrap();
        let mut s = open_on(&root);
        assert_eq!(s.rows().len(), 2);
        s.handle_key(key('.'));
        assert_eq!(s.rows().len(), 3, "dotfile shown");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn reveal_expands_ancestors_and_selects_the_file() {
        let root = fixture();
        let mut s = open_on(&root);
        s.reveal(&root.join("a").join("x.txt"));
        assert_eq!(s.rows()[s.selected].name, "x.txt");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn reveal_ignores_paths_outside_the_root() {
        let root = fixture();
        let mut s = open_on(&root);
        let before = s.selected;
        s.reveal(Path::new("/definitely/elsewhere/z.txt"));
        assert_eq!(s.selected, before);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn view_marks_the_selected_row_and_reports_the_count() {
        let root = fixture();
        let mut s = open_on(&root);
        s.handle_key(key('j'));
        let theme = Theme::default();
        let v = s.view(CoreRect::new(0, 0, 30, 10), UIMode::default(), &theme);
        assert_eq!(v.statusline.right_text(), "2 items");
        assert!(v.lines[0].highlights.is_empty(), "unselected row is plain");
        assert!(
            !v.lines[1].highlights.is_empty(),
            "selected row is highlighted"
        );
        // The highlight must come from the theme, not a hardcoded grey, or the
        // sidebar looks foreign against every other selectable list.
        let (off, len, style) = v.lines[1].highlights[0];
        assert_eq!(style.bg, theme.selection_bg);
        assert_eq!(style.fg, theme.selection_fg);
        assert_eq!(off, 0);
        assert_eq!(
            len,
            v.lines[1].text.chars().count(),
            "highlight length is in chars: the ▸/▾ markers are multi-byte"
        );
        // The selected row is padded out so the highlight spans the panel,
        // rather than stopping at the end of the file name.
        assert_eq!(len, 30, "selection covers the full row width");
        assert!(
            v.lines[1].text.starts_with("  b.txt"),
            "{:?}",
            v.lines[1].text
        );
        assert!(v.lines[1].text.ends_with(' '), "padded to the panel width");
        // Unselected rows are not padded.
        assert!(!v.lines[0].text.ends_with(' '));
        assert!(
            v.lines[0].text.contains("▸ a"),
            "collapsed dir marker: {:?}",
            v.lines[0].text
        );
        std::fs::remove_dir_all(&root).ok();
    }
}
