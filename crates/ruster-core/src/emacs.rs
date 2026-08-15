//! Emacs-style modeless editing.
//!
//! Where [`crate::vim::VimState`] is modal, `EmacsState` is always "inserting":
//! a self-inserting character produces an [`EditOp::InsertChar`], while
//! `Ctrl`/`Alt` chords are commands. Like the vim layer it is pure — it reads
//! an [`EditorView`] and returns [`Action`]s the caller applies — so the same
//! headless tests apply. App-level chords (the `C-x` prefix, `M-x`, isearch,
//! `C-g`) are handled by the frontend before delegation, mirroring how the vim
//! path intercepts the leader and `Ctrl-w` prefixes.

use std::cell::RefCell;

use crate::action::{Action, EditOp, Motion};
use crate::cursor::Edge;
use crate::editor::EditorView;
use crate::key::KeyEvent;
use crate::vim::motions::{next_word_start, prev_word_start};

pub struct EmacsState {
    /// The mark (region anchor). The region spans mark..point; `None` = inactive.
    mark: Option<usize>,
    /// Kill ring, newest last. `C-y` yanks the last entry; `M-y` rotates.
    kill_ring: Vec<String>,
    /// Index of the last yank within the ring, for `M-y` (yank-pop).
    yank_index: Option<usize>,
    /// Position of the last yank, so `M-y` can replace exactly what it inserted.
    last_yank: Option<(usize, usize)>,
    /// Universal-argument count being typed (`C-u`, then digits).
    prefix_arg: Option<u32>,
    /// True while reading digits of a `C-u` prefix.
    reading_prefix: bool,
    /// Mirror of the kill-ring top on the system clipboard, like the vim yank.
    clipboard: RefCell<Option<arboard::Clipboard>>,
}

impl Default for EmacsState {
    fn default() -> Self {
        Self::new()
    }
}

impl EmacsState {
    pub fn new() -> Self {
        EmacsState {
            mark: None,
            kill_ring: Vec::new(),
            yank_index: None,
            last_yank: None,
            prefix_arg: None,
            reading_prefix: false,
            clipboard: RefCell::new(arboard::Clipboard::new().ok()),
        }
    }

    pub fn mark(&self) -> Option<usize> {
        self.mark
    }

    /// Plant the mark, as `C-SPC` does. A mouse drag needs this: the region it
    /// builds has to be the same region the kill and yank commands act on.
    pub fn set_mark(&mut self, at: usize) {
        self.mark = Some(at);
    }

    /// Clear the mark and any pending prefix (the `C-g` "keyboard-quit").
    pub fn cancel(&mut self) {
        self.mark = None;
        self.prefix_arg = None;
        self.reading_prefix = false;
    }

    /// Seed the kill ring / clipboard from the frontend (e.g. an external copy).
    pub fn push_kill(&mut self, text: String) {
        self.set_clipboard(&text);
        self.kill_ring.push(text);
    }

    /// The text a paste should insert: the top of the kill ring, falling back
    /// to the system clipboard.
    ///
    /// Ring first so that `:paste` and `C-y` always agree. They would diverge
    /// otherwise: every kill mirrors itself onto the system clipboard, but
    /// `arboard` may be absent (a headless session, a Wayland compositor with
    /// no data device), and then the ring is the only record that a kill
    /// happened at all.
    pub fn paste_text(&self) -> Option<String> {
        self.kill_ring.last().cloned().or_else(|| {
            self.clipboard
                .borrow_mut()
                .as_mut()
                .and_then(|c| c.get_text().ok())
        })
    }

    fn set_clipboard(&self, text: &str) {
        if let Some(cb) = self.clipboard.borrow_mut().as_mut() {
            let _ = cb.set_text(text.to_string());
        }
    }

    /// Number of times the next command should repeat (the prefix arg, else 1).
    fn take_count(&mut self) -> u32 {
        self.reading_prefix = false;
        self.prefix_arg.take().unwrap_or(1)
    }

    /// Handle one key, returning the actions to apply. `Enter`, `Tab`, and the
    /// app-level chords are dealt with by the frontend and never reach here.
    pub fn handle(&mut self, key: KeyEvent, editor: &dyn EditorView) -> Vec<Action> {
        // Reading the digits of a universal argument (`C-u 1 2 <cmd>`).
        if self.reading_prefix {
            if let KeyEvent::Char(c) = key {
                if let Some(d) = c.to_digit(10) {
                    self.prefix_arg = Some(self.prefix_arg.unwrap_or(0) * 10 + d);
                    return Vec::new();
                }
            }
            // Any non-digit ends prefix reading and is handled normally below.
            self.reading_prefix = false;
        }

        let head = editor.primary_head();
        let buf = editor.buffer();
        let len = buf.len_chars();

        match key {
            // --- Universal argument -------------------------------------------------
            KeyEvent::Ctrl('u') => {
                self.reading_prefix = true;
                self.prefix_arg = Some(0);
                Vec::new()
            }

            // --- Movement -----------------------------------------------------------
            KeyEvent::Ctrl('f') => self.repeat_move(Motion::Grapheme(1)),
            KeyEvent::Ctrl('b') => self.repeat_move(Motion::Grapheme(-1)),
            KeyEvent::Ctrl('n') => self.repeat_move(Motion::Line(1)),
            KeyEvent::Ctrl('p') => self.repeat_move(Motion::Line(-1)),
            KeyEvent::Ctrl('a') => {
                self.take_count();
                vec![Action::Move(Motion::LineEdge(Edge::Start))]
            }
            KeyEvent::Ctrl('e') => {
                self.take_count();
                vec![Action::Move(Motion::LineEdge(Edge::End))]
            }
            KeyEvent::Alt('f') => {
                let n = self.take_count();
                let mut at = head;
                for _ in 0..n {
                    at = next_word_start(buf, at);
                }
                vec![Action::Move(Motion::To(at))]
            }
            KeyEvent::Alt('b') => {
                let n = self.take_count();
                let mut at = head;
                for _ in 0..n {
                    at = prev_word_start(buf, at);
                }
                vec![Action::Move(Motion::To(at))]
            }
            KeyEvent::Alt('<') => {
                self.take_count();
                vec![Action::Move(Motion::To(0))]
            }
            KeyEvent::Alt('>') => {
                self.take_count();
                vec![Action::Move(Motion::To(len))]
            }
            KeyEvent::Ctrl('v') => {
                self.take_count();
                let half = (editor.viewport_height() as i32).max(1);
                vec![Action::Move(Motion::Line(half)), Action::Scroll(half)]
            }
            KeyEvent::Alt('v') => {
                self.take_count();
                let half = (editor.viewport_height() as i32).max(1);
                vec![Action::Move(Motion::Line(-half)), Action::Scroll(-half)]
            }

            // --- Mark & region ------------------------------------------------------
            KeyEvent::Ctrl(' ') => {
                self.mark = Some(head);
                self.take_count();
                Vec::new()
            }
            KeyEvent::Ctrl('w') => {
                // kill-region: cut mark..point onto the ring.
                self.take_count();
                match self.region(head) {
                    Some((lo, hi)) if hi > lo => {
                        self.push_kill(buf.slice_string(lo, hi));
                        self.mark = None;
                        vec![
                            Action::BeginBatch,
                            Action::Edit(EditOp::DeleteRange(lo, hi)),
                            Action::EndBatch,
                        ]
                    }
                    _ => Vec::new(),
                }
            }
            KeyEvent::Alt('w') => {
                // copy-region: ring only, buffer untouched.
                self.take_count();
                if let Some((lo, hi)) = self.region(head) {
                    if hi > lo {
                        self.push_kill(buf.slice_string(lo, hi));
                    }
                }
                self.mark = None;
                Vec::new()
            }

            // --- Kill & yank --------------------------------------------------------
            KeyEvent::Ctrl('k') => {
                self.take_count();
                let line = buf.char_to_line(head);
                // `line_end_char` points past the newline; step back over it to
                // get the end of the visible content.
                let mut content_end = buf.line_end_char(line);
                if content_end > head && buf.char_at(content_end - 1) == '\n' {
                    content_end -= 1;
                }
                // At the line end, kill the newline; otherwise kill to it.
                let (lo, hi) = if head >= content_end && head < len {
                    (head, head + 1)
                } else {
                    (head, content_end)
                };
                if hi > lo {
                    self.push_kill(buf.slice_string(lo, hi));
                    vec![
                        Action::BeginBatch,
                        Action::Edit(EditOp::DeleteRange(lo, hi)),
                        Action::EndBatch,
                    ]
                } else {
                    Vec::new()
                }
            }
            KeyEvent::Ctrl('y') => {
                self.take_count();
                match self.kill_ring.last().cloned() {
                    Some(text) => {
                        let n = text.chars().count();
                        self.yank_index = Some(self.kill_ring.len() - 1);
                        self.last_yank = Some((head, head + n));
                        vec![
                            Action::BeginBatch,
                            Action::Edit(EditOp::InsertString(text)),
                            Action::EndBatch,
                        ]
                    }
                    None => Vec::new(),
                }
            }
            KeyEvent::Alt('y') => {
                // yank-pop: only valid right after a yank/yank-pop.
                self.take_count();
                match (self.last_yank, self.yank_index) {
                    (Some((start, end)), Some(idx)) if !self.kill_ring.is_empty() => {
                        let new_idx = (idx + self.kill_ring.len() - 1) % self.kill_ring.len();
                        let text = self.kill_ring[new_idx].clone();
                        let n = text.chars().count();
                        self.yank_index = Some(new_idx);
                        self.last_yank = Some((start, start + n));
                        vec![
                            Action::BeginBatch,
                            Action::Edit(EditOp::DeleteRange(start, end)),
                            Action::Edit(EditOp::InsertString(text)),
                            Action::EndBatch,
                        ]
                    }
                    _ => Vec::new(),
                }
            }

            // --- Deletion -----------------------------------------------------------
            KeyEvent::Ctrl('d') | KeyEvent::Delete => {
                let n = self.take_count();
                let end = (head + n as usize).min(len);
                self.reset_yank();
                if end > head {
                    vec![
                        Action::BeginBatch,
                        Action::Edit(EditOp::DeleteRange(head, end)),
                        Action::EndBatch,
                    ]
                } else {
                    Vec::new()
                }
            }
            KeyEvent::Backspace => {
                let n = self.take_count();
                self.reset_yank();
                let mut out = vec![Action::BeginBatch];
                for _ in 0..n {
                    out.push(Action::Edit(EditOp::Backspace));
                }
                out.push(Action::EndBatch);
                out
            }
            KeyEvent::Alt('d') => {
                // kill-word forward.
                self.take_count();
                let to = next_word_start(buf, head);
                if to > head {
                    self.push_kill(buf.slice_string(head, to));
                    vec![
                        Action::BeginBatch,
                        Action::Edit(EditOp::DeleteRange(head, to)),
                        Action::EndBatch,
                    ]
                } else {
                    Vec::new()
                }
            }

            // --- Undo ---------------------------------------------------------------
            KeyEvent::Ctrl('/') | KeyEvent::Ctrl('_') => {
                self.take_count();
                self.reset_yank();
                vec![Action::Undo]
            }

            // --- Newline & self-insert ---------------------------------------------
            KeyEvent::Enter => {
                let n = self.take_count();
                self.reset_yank();
                let mut out = vec![Action::BeginBatch];
                for _ in 0..n {
                    out.push(Action::Edit(EditOp::InsertChar('\n')));
                }
                out.push(Action::EndBatch);
                out
            }
            KeyEvent::Char(c) => {
                let n = self.take_count();
                self.reset_yank();
                let mut out = vec![Action::BeginBatch];
                for _ in 0..n {
                    out.push(Action::Edit(EditOp::InsertChar(c)));
                }
                out.push(Action::EndBatch);
                out
            }

            _ => {
                self.take_count();
                Vec::new()
            }
        }
    }

    /// Repeat a simple motion `count` times.
    fn repeat_move(&mut self, m: Motion) -> Vec<Action> {
        let n = self.take_count();
        (0..n).map(|_| Action::Move(m.clone())).collect()
    }

    /// The active region as an ordered `(lo, hi)`, or `None` with no mark.
    pub fn region(&self, point: usize) -> Option<(usize, usize)> {
        self.mark.map(|m| (m.min(point), m.max(point)))
    }

    /// End a yank sequence; any edit other than `M-y` invalidates yank-pop.
    fn reset_yank(&mut self) {
        self.yank_index = None;
        self.last_yank = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Editor;

    /// Drive an editor through emacs keys, returning the final buffer text.
    fn run(src: &str, start: usize, keys: &[KeyEvent]) -> (Editor, EmacsState) {
        let mut e = Editor::from_str(src);
        e.execute(Action::Move(Motion::To(start)));
        let mut em = EmacsState::new();
        for &k in keys {
            for a in em.handle(k, &e) {
                e.execute(a);
            }
        }
        (e, em)
    }

    #[test]
    fn self_insert_and_ctrl_a_e() {
        let (e, _) = run("hello", 5, &[KeyEvent::Char('!')]);
        assert_eq!(e.buffer().to_string(), "hello!");
        // C-a to line start, insert, C-e to end, insert.
        let (e, _) = run(
            "hello",
            5,
            &[
                KeyEvent::Ctrl('a'),
                KeyEvent::Char('>'),
                KeyEvent::Ctrl('e'),
                KeyEvent::Char('<'),
            ],
        );
        assert_eq!(e.buffer().to_string(), ">hello<");
    }

    #[test]
    fn ctrl_d_and_backspace() {
        let (e, _) = run("abc", 1, &[KeyEvent::Ctrl('d')]);
        assert_eq!(e.buffer().to_string(), "ac");
        let (e, _) = run("abc", 2, &[KeyEvent::Backspace]);
        assert_eq!(e.buffer().to_string(), "ac");
    }

    #[test]
    fn ctrl_k_kills_to_line_end_then_the_newline() {
        // With text after point, kill to the line end.
        let (e, _) = run("hello world\nnext", 6, &[KeyEvent::Ctrl('k')]);
        assert_eq!(e.buffer().to_string(), "hello \nnext");
        // At the line end, a second kill removes the newline, joining the lines.
        let (e, _) = run("hello \nnext", 6, &[KeyEvent::Ctrl('k')]);
        assert_eq!(e.buffer().to_string(), "hello next");
    }

    #[test]
    fn kill_region_and_yank_roundtrip() {
        // Set mark at 0, move to 5 (C-f x5 via prefix), kill region, yank.
        let mut e = Editor::from_str("hello world");
        e.execute(Action::Move(Motion::To(0)));
        let mut em = EmacsState::new();
        for a in em.handle(KeyEvent::Ctrl(' '), &e) {
            e.execute(a);
        }
        // Move point to offset 5 with C-u 5 C-f.
        for &k in &[
            KeyEvent::Ctrl('u'),
            KeyEvent::Char('5'),
            KeyEvent::Ctrl('f'),
        ] {
            for a in em.handle(k, &e) {
                e.execute(a);
            }
        }
        for a in em.handle(KeyEvent::Ctrl('w'), &e) {
            e.execute(a);
        }
        assert_eq!(e.buffer().to_string(), " world");
        // Yank restores the killed "hello" at point (offset 0).
        e.execute(Action::Move(Motion::To(0)));
        for a in em.handle(KeyEvent::Ctrl('y'), &e) {
            e.execute(a);
        }
        assert_eq!(e.buffer().to_string(), "hello world");
    }

    #[test]
    fn universal_argument_repeats_insert() {
        let (e, _) = run(
            "",
            0,
            &[
                KeyEvent::Ctrl('u'),
                KeyEvent::Char('3'),
                KeyEvent::Char('x'),
            ],
        );
        assert_eq!(e.buffer().to_string(), "xxx");
    }

    #[test]
    fn yank_pop_rotates_the_ring() {
        let mut e = Editor::from_str("");
        let mut em = EmacsState::new();
        em.push_kill("first".to_string());
        em.push_kill("second".to_string());
        // C-y yanks the newest ("second").
        for a in em.handle(KeyEvent::Ctrl('y'), &e) {
            e.execute(a);
        }
        assert_eq!(e.buffer().to_string(), "second");
        // M-y replaces it with the previous ("first").
        for a in em.handle(KeyEvent::Alt('y'), &e) {
            e.execute(a);
        }
        assert_eq!(e.buffer().to_string(), "first");
    }

    #[test]
    fn alt_f_b_word_motions() {
        let (e, _) = run("foo bar baz", 0, &[KeyEvent::Alt('f'), KeyEvent::Char('|')]);
        // M-f lands at the start of "bar" (offset 4).
        assert_eq!(e.buffer().to_string(), "foo |bar baz");
    }

    #[test]
    fn paste_text_reads_the_top_of_the_kill_ring() {
        let mut em = EmacsState::new();
        em.push_kill("older".to_string());
        em.push_kill("newer".to_string());
        assert_eq!(em.paste_text().as_deref(), Some("newer"));
    }

    #[test]
    fn paste_text_is_none_with_no_kills_and_no_clipboard() {
        let em = EmacsState::new();
        // Detached rather than merely empty: an attached clipboard would answer
        // with whatever the host machine happens to be holding.
        *em.clipboard.borrow_mut() = None;
        assert_eq!(em.paste_text(), None);
    }

    #[test]
    fn region_is_ordered_and_absent_without_a_mark() {
        let mut em = EmacsState::new();
        assert_eq!(em.region(3), None);
        em.set_mark(7);
        // Point before mark: still reported low-to-high.
        assert_eq!(em.region(3), Some((3, 7)));
        assert_eq!(em.region(9), Some((7, 9)));
    }
}
