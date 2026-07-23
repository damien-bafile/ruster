pub mod motions;
pub mod ops;
pub mod textobj;

use crate::action::{Action, EditOp, Motion};
use crate::cursor::Edge;
use crate::editor::EditorView;
use crate::key::KeyEvent;
use crate::vim::motions::{next_word_start, prev_word_start, word_end, last_printable_in_line, char_to_line};
use std::cell::RefCell;



#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VimMode { Normal, Insert, VisualChar, VisualLine, Cmdline }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpState { Idle, Pending(char, u32) }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LastChange {
    OperatorMotion { op: char, motion: char, count: u32 },
    OperatorTextobj { op: char, kind: char, target: char },
    DeleteChar,
}

fn next_word_occurrence(editor: &dyn EditorView) -> Option<usize> {
    let head = editor.primary_head();
    let buf = editor.buffer();
    let text = buf.to_string();
    let line = char_to_line(editor, head);
    let line_start = buf.line_start_char(line);
    let line_end = buf.line_end_char(line);
    let content = buf.slice_string(line_start, line_end);
    let col = head - line_start;
    let chars: Vec<char> = content.chars().collect();
    if col >= chars.len() || !chars[col].is_alphanumeric() && chars[col] != '_' {
        return None;
    }
    let word_start = (0..=col).rev().take_while(|&i| i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_')).last().unwrap_or(col);
    let word_end = (col..chars.len()).take_while(|&i| chars[i].is_alphanumeric() || chars[i] == '_').last().unwrap_or(col);
    let word: String = chars[word_start..=word_end].iter().collect();
    if word.is_empty() {
        return None;
    }
    let search_from = head + 1;
    if search_from >= text.len() {
        return None;
    }
    text[search_from..].find(&word).map(|pos| search_from + pos)
}

pub struct VimState {
    pub mode: VimMode,
    count: Option<u32>,
    pending_g: bool,
    pending: OpState,
    pending_textobj: Option<char>,
    register: Option<String>,
    last_change: Option<LastChange>,
    anchor: Option<usize>,
    cmdline_buffer: String,
    clipboard: RefCell<Option<arboard::Clipboard>>,
    clipboard_buf: RefCell<Option<String>>,
}

impl VimState {
    pub fn new() -> Self {
        VimState {
            mode: VimMode::Normal,
            count: None,
            pending_g: false,
            pending: OpState::Idle,
            pending_textobj: None,
            register: None,
            last_change: None,
            anchor: None,
            cmdline_buffer: String::new(),
            clipboard: RefCell::new(arboard::Clipboard::new().ok()),
            clipboard_buf: RefCell::new(None),
        }
    }

    pub fn cmdline_buffer(&self) -> &str { &self.cmdline_buffer }

    pub fn set_register(&mut self, text: String) { self.register = Some(text); }

    pub fn clipboard_get(&self) -> Option<String> {
        self.clipboard_buf.borrow().clone()
            .or_else(|| {
                self.clipboard.borrow_mut().as_mut()
                    .and_then(|c| c.get_text().ok())
            })
    }

    pub fn clipboard_set(&self, text: &str) {
        *self.clipboard_buf.borrow_mut() = Some(text.to_string());
        if let Some(ref mut c) = *self.clipboard.borrow_mut() {
            let _ = c.set_text(text);
        }
    }

    pub fn handle(&mut self, key: KeyEvent, editor: &dyn EditorView) -> Vec<Action> {
        let n = self.count.unwrap_or(1);
        let mut out: Vec<Action> = Vec::new();
        match self.mode {
            VimMode::Normal => self.handle_normal(key, editor, n, &mut out),
            VimMode::Insert => self.handle_insert(key, editor, &mut out),
            VimMode::VisualChar | VimMode::VisualLine => self.handle_visual(key, editor, n, &mut out),
            VimMode::Cmdline => {
                match key {
                    KeyEvent::Esc => {
                        self.mode = VimMode::Normal;
                        self.cmdline_buffer.clear();
                    }
                    KeyEvent::Enter => {
                        let cmd = std::mem::take(&mut self.cmdline_buffer);
                        self.mode = VimMode::Normal;
                        out.push(Action::CmdlineResult(cmd));
                    }
                    KeyEvent::Backspace => {
                        self.cmdline_buffer.pop();
                    }
                    KeyEvent::Char(c) if !c.is_control() => {
                        self.cmdline_buffer.push(c);
                    }
                    _ => {}
                }
            }
        }
        out
    }

    fn stroke_count(&mut self, key: KeyEvent) -> bool {
        if let KeyEvent::Char(c) = key {
            if c.is_ascii_digit() {
                if c == '0' && self.count.is_none() { return false; }
                let d = c.to_digit(10).unwrap_or(0);
                self.count = Some(self.count.map(|v| v * 10 + d).unwrap_or(d));
                return true;
            }
            }
        false
    }

    fn handle_normal(&mut self, key: KeyEvent, editor: &dyn EditorView, n: u32, out: &mut Vec<Action>) {
        if self.stroke_count(key) { return; }

        let pending_now = self.pending;
        if let OpState::Pending(op, count) = pending_now {
            if let Some(kind) = self.pending_textobj {
                self.pending_textobj = None;
                self.pending = OpState::Idle;
                match key {
                    KeyEvent::Char(c2 @ ('w' | '"' | '\'' | '(' | ')' | '{' | '}')) => {
                        if let Some((start, end)) = crate::vim::textobj::range_for_textobj(kind, c2, editor) {
                            self.apply_operator(op, start, end, editor, out);
                            if op == 'd' || op == 'c' {
                                self.last_change = Some(LastChange::OperatorTextobj { op, kind, target: c2 });
                            }
                        }
                        return;
                    }
                    KeyEvent::Char(c2 @ ('f' | 'c' | 'l' | 'a')) => {
                        self.pending = OpState::Idle;
                        self.pending_textobj = None;
                        let count = self.count.unwrap_or(1);
                        self.count = None;
                        out.push(Action::Textobject { op, kind, target: c2, count });
                        return;
                    }
                    _ => { return; }
                }
            }
            match key {
                KeyEvent::Char(i @ ('i' | 'a')) => {
                    self.pending_textobj = Some(i);
                    self.pending = OpState::Pending(op, count);
                    return;
                }
                KeyEvent::Char(m @ ('w' | 'b' | 'e' | '0' | '$' | 'G' | 'd' | 'y' | 'c')) => {
                    self.pending = OpState::Idle;
                    if let Some((start, end)) = crate::vim::ops::range_for_motion(editor, m, count) {
                        self.apply_operator(op, start, end, editor, out);
                        if op == 'd' || op == 'c' {
                            // for `c`, `.` will re-enter Insert without replaying typed text (Plan A scope cut).
                            self.last_change = Some(LastChange::OperatorMotion { op, motion: m, count });
                        }
                    }
                    return;
                }
                KeyEvent::Char('>') if op == '>' => {
                    self.pending = OpState::Idle;
                    out.push(Action::IndentLine);
                }
                KeyEvent::Char('<') if op == '<' => {
                    self.pending = OpState::Idle;
                    out.push(Action::DeindentLine);
                }
                _ => {
                    self.pending = OpState::Idle;
                    return;
                }
            }
        }

        if self.pending_g {
            self.pending_g = false;
            if key == KeyEvent::Char('g') {
                out.push(Action::Move(Motion::To(0)));
            }
            return;
        }

        match key {
            KeyEvent::Esc => {
                if editor.cursors().count() > 1 {
                    out.push(Action::ClearExtraCursors);
                }
                self.count = None;
            }
            KeyEvent::Char(':') => {
                self.mode = VimMode::Cmdline;
                self.cmdline_buffer = String::from(":");
                self.count = None;
            }
            KeyEvent::Char('h') => { for _ in 0..n { out.push(Action::Move(Motion::Grapheme(-1))); } self.count = None; }
            KeyEvent::Char('l') => { for _ in 0..n { out.push(Action::Move(Motion::Grapheme(1))); } self.count = None; }
            KeyEvent::Char('j') => { out.push(Action::Move(Motion::Line(n as i32))); self.count = None; }
            KeyEvent::Char('k') => { out.push(Action::Move(Motion::Line(-(n as i32)))); self.count = None; }
            KeyEvent::Char('0') => { out.push(Action::Move(Motion::LineEdge(Edge::Start))); self.count = None; }
            KeyEvent::Char('$') => {
                out.push(Action::Move(Motion::To(last_printable_in_line(editor))));
                self.count = None;
            }
            KeyEvent::Char('G') => {
                let last_line = editor.buffer().line_count().saturating_sub(1);
                out.push(Action::Move(Motion::To(editor.buffer().line_start_char(last_line))));
                self.count = None;
            }
            KeyEvent::Char('g') => { self.pending_g = true; }
            KeyEvent::Char('w') => { self.do_word_motion(editor, n, next_word_start, out); self.count = None; }
            KeyEvent::Char('b') => { self.do_word_motion(editor, n, prev_word_start, out); self.count = None; }
            KeyEvent::Char('e') => { self.do_word_motion(editor, n, word_end, out); self.count = None; }
            KeyEvent::Char('i') if self.pending == OpState::Idle && self.pending_textobj.is_none() && self.anchor.is_none() => {
                self.mode = VimMode::Insert;
                self.count = None;
                out.push(Action::BeginBatch);
            }
            KeyEvent::Char('d') if self.pending == OpState::Idle => {
                self.pending = OpState::Pending('d', n);
                self.count = None;
            }
            KeyEvent::Char('y') if self.pending == OpState::Idle => {
                self.pending = OpState::Pending('y', n);
                self.count = None;
            }
            KeyEvent::Char('c') if self.pending == OpState::Idle => {
                self.pending = OpState::Pending('c', n);
                self.count = None;
            }
            KeyEvent::Char('>') if self.pending == OpState::Idle => {
                self.pending = OpState::Pending('>', n);
                self.count = None;
            }
            KeyEvent::Char('<') if self.pending == OpState::Idle => {
                self.pending = OpState::Pending('<', n);
                self.count = None;
            }
            KeyEvent::Char('x') => {
                let at = editor.primary_head();
                if at < editor.buffer().len_chars() {
                    out.push(Action::BeginBatch);
                    out.push(Action::Edit(EditOp::DeleteRange(at, at + 1)));
                    out.push(Action::EndBatch);
                    self.last_change = Some(LastChange::DeleteChar);
                }
                self.count = None;
            }
            KeyEvent::Char('p') => {
                let text = self.clipboard_get()
                    .or_else(|| self.register.clone())
                    .unwrap_or_default();
                if !text.is_empty() {
                    out.push(Action::BeginBatch);
                    out.push(Action::Edit(EditOp::InsertString(text)));
                    out.push(Action::EndBatch);
                }
                self.count = None;
            }
            KeyEvent::Char('.') => {
                self.replay_last_change(editor, out);
                self.count = None;
            }
            KeyEvent::Char('u') => {
                out.push(Action::Undo);
                self.count = None;
            }
            KeyEvent::Ctrl('r') => {
                out.push(Action::Redo);
                self.count = None;
            }
            KeyEvent::Char('v') => {
                let at = editor.primary_head();
                self.mode = VimMode::VisualChar;
                self.anchor = Some(at);
                out.push(Action::BeginVisual(at));
                self.count = None;
            }
            KeyEvent::Char('V') => {
                let at = editor.primary_head();
                self.mode = VimMode::VisualLine;
                self.anchor = Some(at);
                out.push(Action::BeginVisual(at));
                self.count = None;
            }
            KeyEvent::Ctrl('d') => {
                if let Some(pos) = next_word_occurrence(editor) {
                    out.push(Action::AddCursor(pos));
                }
                self.count = None;
            }
            _ => { self.count = None; }
        }
    }

    fn replay_last_change(&mut self, editor: &dyn EditorView, out: &mut Vec<Action>) {
        let lc = match self.last_change { Some(lc) => lc, None => return };
        match lc {
            LastChange::OperatorMotion { op, motion, count } => {
                if let Some((start, end)) = crate::vim::ops::range_for_motion(editor, motion, count) {
                    self.apply_operator(op, start, end, editor, out);
                }
            }
            LastChange::OperatorTextobj { op, kind, target } => {
                if let Some((start, end)) = crate::vim::textobj::range_for_textobj(kind, target, editor) {
                    self.apply_operator(op, start, end, editor, out);
                }
            }
            LastChange::DeleteChar => {
                let at = editor.primary_head();
                if at < editor.buffer().len_chars() {
                    out.push(Action::BeginBatch);
                    out.push(Action::Edit(EditOp::DeleteRange(at, at + 1)));
                    out.push(Action::EndBatch);
                }
            }
        }
    }

    fn apply_operator(&mut self, op: char, start: usize, end: usize, editor: &dyn EditorView, out: &mut Vec<Action>) {
        let safe_end = end.min(editor.buffer().len_chars());
        let text = editor.buffer().slice_string(start, safe_end);
        match op {
            'd' => {
                out.push(Action::BeginBatch);
                out.push(Action::Edit(EditOp::DeleteRange(start, end)));
                out.push(Action::EndBatch);
            }
            'y' => {
                self.register = Some(text.clone());
                self.clipboard_set(&text);
            }
            'c' => {
                out.push(Action::BeginBatch);
                out.push(Action::Edit(EditOp::DeleteRange(start, end)));
                self.mode = VimMode::Insert;
                // No EndBatch + re-BeginBatch: the Insert-mode edits that follow
                // must group with the deletion into a single undo unit (real Vim
                // `c{motion}...Esc` is one undo). Esc issues the EndBatch.
            }
            _ => {}
        }
    }

    fn do_word_motion<F: Fn(&crate::buffer::Buffer, usize) -> usize>(
        &self, editor: &dyn EditorView, n: u32, step: F, out: &mut Vec<Action>,
    ) {
        let mut target = editor.primary_head();
        let buf = editor.buffer();
        for _ in 0..n { target = step(buf, target); }
        out.push(Action::Move(Motion::To(target)));
    }

    fn handle_insert(&mut self, key: KeyEvent, _editor: &dyn EditorView, out: &mut Vec<Action>) {
        match key {
            KeyEvent::Esc => {
                out.push(Action::EndBatch);
                out.push(Action::Move(Motion::Grapheme(-1)));
                self.mode = VimMode::Normal;
            }
            KeyEvent::Char(c) if !c.is_control() => {
                out.push(Action::Edit(EditOp::InsertChar(c)));
            }
            KeyEvent::Enter => {
                out.push(Action::Edit(EditOp::InsertChar('\n')));
            }
            KeyEvent::Backspace => {
                out.push(Action::Edit(EditOp::Backspace));
            }
            _ => {}
        }
    }

    fn handle_visual(&mut self, key: KeyEvent, editor: &dyn EditorView, n: u32, out: &mut Vec<Action>) {
        if self.stroke_count(key) { return; }
        let anchor = match self.anchor { Some(a) => a, None => editor.primary_head() };
        match key {
            KeyEvent::Esc => {
                self.mode = VimMode::Normal;
                self.anchor = None;
                self.count = None;
            }
            KeyEvent::Char('h') => { for _ in 0..n { out.push(Action::Move(Motion::Grapheme(-1))); } out.push(Action::BeginVisual(anchor)); self.count = None; }
            KeyEvent::Char('l') => { for _ in 0..n { out.push(Action::Move(Motion::Grapheme(1))); } out.push(Action::BeginVisual(anchor)); self.count = None; }
            KeyEvent::Char('j') => { out.push(Action::Move(Motion::Line(n as i32))); out.push(Action::BeginVisual(anchor)); self.count = None; }
            KeyEvent::Char('k') => { out.push(Action::Move(Motion::Line(-(n as i32)))); out.push(Action::BeginVisual(anchor)); self.count = None; }
            KeyEvent::Char('w') => {
                let target = crate::vim::motions::next_word_start(editor.buffer(), editor.primary_head());
                out.push(Action::Move(Motion::To(target)));
                out.push(Action::BeginVisual(anchor));
                self.count = None;
            }
            KeyEvent::Char('b') => {
                let target = crate::vim::motions::prev_word_start(editor.buffer(), editor.primary_head());
                out.push(Action::Move(Motion::To(target)));
                out.push(Action::BeginVisual(anchor));
                self.count = None;
            }
            KeyEvent::Char('e') => {
                let target = crate::vim::motions::word_end(editor.buffer(), editor.primary_head());
                out.push(Action::Move(Motion::To(target)));
                out.push(Action::BeginVisual(anchor));
                self.count = None;
            }
            KeyEvent::Char('$') => {
                out.push(Action::Move(Motion::To(last_printable_in_line(editor))));
                out.push(Action::BeginVisual(anchor));
                self.count = None;
            }
            KeyEvent::Char('0') => {
                out.push(Action::Move(Motion::LineEdge(Edge::Start)));
                out.push(Action::BeginVisual(anchor));
                self.count = None;
            }
            KeyEvent::Char('x') | KeyEvent::Char('d') => {
                let (start, end) = self.visual_range(editor);
                out.push(Action::BeginBatch);
                out.push(Action::Edit(EditOp::DeleteRange(start, end)));
                out.push(Action::EndBatch);
                self.mode = VimMode::Normal;
                self.anchor = None;
                self.count = None;
            }
            KeyEvent::Char('y') => {
                let (start, end) = self.visual_range(editor);
                let safe_end = end.min(editor.buffer().len_chars());
                let text = editor.buffer().slice_string(start, safe_end);
                self.register = Some(text.clone());
                self.clipboard_set(&text);
                // Vim: after visual yank, cursor jumps to start of selection.
                out.push(Action::Move(Motion::To(start)));
                self.mode = VimMode::Normal;
                self.anchor = None;
                self.count = None;
            }
            KeyEvent::Char('c') => {
                let (start, end) = self.visual_range(editor);
                out.push(Action::BeginBatch);
                out.push(Action::Edit(EditOp::DeleteRange(start, end)));
                self.mode = VimMode::Insert;
                self.anchor = None;
                // No EndBatch + re-BeginBatch: Insert-mode edits group with the
                // deletion; Esc fires EndBatch so the whole visual-c is one undo unit.
                self.count = None;
            }
            KeyEvent::Char('>') => {
                out.push(Action::IndentLine);
                self.mode = VimMode::Normal;
                self.anchor = None;
                self.count = None;
            }
            KeyEvent::Char('<') => {
                out.push(Action::DeindentLine);
                self.mode = VimMode::Normal;
                self.anchor = None;
                self.count = None;
            }
            _ => {}
        }
    }

    fn visual_range(&self, editor: &dyn EditorView) -> (usize, usize) {
        let r = editor.cursors().primary();
        let s = r.start();
        let e = r.end();
        if matches!(self.mode, VimMode::VisualLine) {
            let s_line = crate::vim::motions::char_to_line(editor, s);
            let e_line = crate::vim::motions::char_to_line(editor, e);
            let start = editor.buffer().line_start_char(s_line);
            let end = editor.buffer().line_end_char(e_line);
            (start, end)
        } else {
            (s, e + 1) // visual char selection is inclusive on the cursor side
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Editor;
    use crate::key::KeyEvent;

    fn to_start(e: &mut Editor, v: &mut VimState) {
        for a in v.handle(KeyEvent::Char('g'), e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('g'), e) { e.execute(a); }
    }

    #[test]
    fn v_l_extends_selection_to_next_char_then_x_deletes() {
        let mut e = Editor::from_str("hello");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('v'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('l'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('x'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "llo"); // deletes "he"
    }

    #[test]
    fn v_2l_extends_two_chars_then_x_deletes() {
        let mut e = Editor::from_str("hello");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('v'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('l'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('l'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('x'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "lo"); // deletes "hel"
    }

    #[test]
    fn capital_v_line_visual_then_d() {
        let mut e = Editor::from_str("abc\ndef\nghi");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('V'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('d'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "def\nghi");
    }

    #[test]
    fn esc_exits_visual_without_change() {
        let mut e = Editor::from_str("hello");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('v'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Esc, &e) { e.execute(a); }
        assert_eq!(v.mode, VimMode::Normal);
        assert_eq!(e.buffer().to_string(), "hello");
    }

    #[test]
    fn v_w_extends_to_word_then_x_deletes_partial_word_inclusive() {
        let mut e = Editor::from_str("hello world");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('v'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('w'), &e) { e.execute(a); } // cursor at 6 ('w'); selection includes 'w'
        for a in v.handle(KeyEvent::Char('x'), &e) { e.execute(a); }
        // visual char selection includes char under cursor; deletes "hello w" -> "orld"
        assert_eq!(e.buffer().to_string(), "orld");
    }

    #[test]
    fn visual_y_then_p_pastes_after() {
        let mut e = Editor::from_str("abc");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        // v, l, y yanks 2 chars ("ab"); exits visual; cursor at start = 0
        for a in v.handle(KeyEvent::Char('v'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('l'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('y'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "abc"); // unchanged by yank
        // p inserts register "ab" at cursor 0 -> "ababc"
        for a in v.handle(KeyEvent::Char('p'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "ababc");
    }

    #[test]
    fn yank_sets_register() {
        let e = Editor::from_str("hello world");
        let mut v = VimState::new();
        // yy yanks current line
        let _actions: Vec<Action> = v.handle(KeyEvent::Char('y'), &e);
        // 'y' is a pending operator; second 'y' triggers
        let _actions: Vec<Action> = v.handle(KeyEvent::Char('y'), &e);
        assert!(v.register.is_some());
        // Note: clipboard write is best-effort, can't test in CI without display
    }

    #[test]
    fn paste_uses_register_fallback() {
        let mut e = Editor::from_str("ab");
        let mut v = VimState::new();
        v.set_register("X".to_string());
        let actions: Vec<Action> = v.handle(KeyEvent::Char('p'), &e);
        for a in actions { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "abX");
    }

    // Dot-repeat regression tests (Task 10) — preserved when the test module was rebuilt for Task 11.
    #[test]
    fn dot_repeats_dw_at_cursor() {
        let mut e = Editor::from_str("foo bar baz");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('d'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('w'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "bar baz");
        for a in v.handle(KeyEvent::Char('w'), &e) { e.execute(a); } // cursor -> 4 (on 'b' of baz)
        assert_eq!(e.primary_head(), 4);
        for a in v.handle(KeyEvent::Char('.'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "bar "); // re-applies dw at cursor 4
    }

    #[test]
    fn dot_repeats_x_at_cursor() {
        let mut e = Editor::from_str("abc");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('x'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "bc");
        for a in v.handle(KeyEvent::Char('.'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "c");
    }

    #[test]
    fn dot_repeats_di_paren_textobj() {
        let mut e = Editor::from_str("(a)(b)");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('d'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('i'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('('), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "()(b)");
        assert_eq!(e.primary_head(), 1);
        for a in v.handle(KeyEvent::Char('l'), &e) { e.execute(a); } // cursor -> 2 (on '(')
        assert_eq!(e.primary_head(), 2);
        for a in v.handle(KeyEvent::Char('.'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "()()");
    }

    #[test]
    fn dot_does_nothing_without_prior_change() {
        let mut e = Editor::from_str("hello");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('.'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "hello");
    }

    #[test]
    fn cmdline_colon_enters_cmdline_mode() {
        let mut e = Editor::from_str("hello");
        let mut v = VimState::new();
        for a in v.handle(KeyEvent::Char(':'), &e) { e.execute(a); }
        assert_eq!(v.mode, VimMode::Cmdline);
        assert_eq!(v.cmdline_buffer(), ":");
    }

    #[test]
    fn cmdline_escape_returns_to_normal() {
        let mut e = Editor::from_str("hello");
        let mut v = VimState::new();
        for a in v.handle(KeyEvent::Char(':'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Esc, &e) { e.execute(a); }
        assert_eq!(v.mode, VimMode::Normal);
        assert_eq!(v.cmdline_buffer(), "");
    }

    #[test]
    fn cmdline_enter_emits_result_and_returns_to_normal() {
        let mut e = Editor::from_str("hello");
        let mut v = VimState::new();
        for a in v.handle(KeyEvent::Char(':'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('w'), &e) { e.execute(a); }
        let actions: Vec<Action> = v.handle(KeyEvent::Enter, &e);
        assert_eq!(v.mode, VimMode::Normal);
        assert!(actions.iter().any(|a| matches!(a, Action::CmdlineResult(c) if c == ":w")));
    }

    #[test]
    fn di_f_triggers_textobject_action() {
        let mut e = Editor::from_str("fn foo() { let x = 1; }");
        let mut v = VimState::new();
        for a in v.handle(KeyEvent::Char('d'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('i'), &e) { e.execute(a); }
        let actions = v.handle(KeyEvent::Char('f'), &e);
        assert!(actions.iter().any(|a| matches!(a, Action::Textobject { op: 'd', kind: 'i', target: 'f', .. })));
    }

    #[test]
    fn ctrl_d_adds_cursor_at_next_word() {
        let mut e = Editor::from_str("foo foo foo");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        let actions: Vec<Action> = v.handle(KeyEvent::Ctrl('d'), &e);
        assert!(actions.iter().any(|a| matches!(a, Action::AddCursor(_))));
    }

    #[test]
    fn ctrl_d_no_word_does_nothing() {
        let mut e = Editor::from_str("... ...");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        let actions: Vec<Action> = v.handle(KeyEvent::Ctrl('d'), &e);
        assert!(actions.is_empty());
    }

    #[test]
    fn esc_clears_extra_cursors() {
        let mut e = Editor::from_str("hello");
        let mut v = VimState::new();
        e.execute(Action::AddCursor(3));
        assert_eq!(e.cursors().count(), 2);
        let actions: Vec<Action> = v.handle(KeyEvent::Esc, &e);
        assert!(actions.iter().any(|a| matches!(a, Action::ClearExtraCursors)));
    }
}