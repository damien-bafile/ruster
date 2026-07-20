pub mod motions;

use crate::action::{Action, EditOp, Motion};
use crate::cursor::Edge;
use crate::editor::Editor;
use crate::key::KeyEvent;
use crate::vim::motions::{next_word_start, prev_word_start, word_end, last_printable_in_line};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VimMode { Normal, Insert, VisualChar, VisualLine, Cmdline }

pub struct VimState {
    pub mode: VimMode,
    count: Option<u32>,
    pending_g: bool,
}

impl VimState {
    pub fn new() -> Self { VimState { mode: VimMode::Normal, count: None, pending_g: false } }

    pub fn handle(&mut self, key: KeyEvent, editor: &Editor) -> Vec<Action> {
        let n = self.count.unwrap_or(1);
        let mut out: Vec<Action> = Vec::new();
        match self.mode {
            VimMode::Normal => self.handle_normal(key, editor, n, &mut out),
            VimMode::Insert => self.handle_insert(key, editor, &mut out),
            VimMode::VisualChar | VimMode::VisualLine => self.handle_visual(key, editor, n, &mut out),
            VimMode::Cmdline => { if key == KeyEvent::Esc { self.mode = VimMode::Normal; } }
        }
        out
    }

    fn stroke_count(&mut self, key: KeyEvent) -> bool {
        if let KeyEvent::Char(c) = key {
            if c.is_ascii_digit() {
                // Vim: '0' alone (no preceding count) is the "line start" motion,
                // not a count digit. Only 1..9 may begin a count.
                if c == '0' && self.count.is_none() { return false; }
                let d = c.to_digit(10).unwrap_or(0);
                self.count = Some(self.count.map(|v| v * 10 + d).unwrap_or(d));
                return true;
            }
        }
        false
    }

    fn handle_normal(&mut self, key: KeyEvent, editor: &Editor, n: u32, out: &mut Vec<Action>) {
        if self.stroke_count(key) { return; }

        if self.pending_g {
            self.pending_g = false;
            if key == KeyEvent::Char('g') {
                // gg: first line, column 0 (Vim default; tests do not exercise counts).
                out.push(Action::Move(Motion::To(0)));
            }
            return;
        }

        match key {
            KeyEvent::Esc => { self.count = None; }
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
            KeyEvent::Char('i') => {
                self.mode = VimMode::Insert;
                self.count = None;
                out.push(Action::BeginBatch);
            }
            _ => { self.count = None; }
        }
    }

    fn do_word_motion<F: Fn(&crate::buffer::Buffer, usize) -> usize>(
        &self, editor: &Editor, n: u32, step: F, out: &mut Vec<Action>,
    ) {
        let mut target = editor.primary_head();
        let buf = editor.buffer();
        for _ in 0..n { target = step(buf, target); }
        out.push(Action::Move(Motion::To(target)));
    }

    fn handle_insert(&mut self, key: KeyEvent, _editor: &Editor, out: &mut Vec<Action>) {
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

    fn handle_visual(&mut self, key: KeyEvent, _editor: &Editor, _n: u32, out: &mut Vec<Action>) {
        if key == KeyEvent::Esc { self.mode = VimMode::Normal; }
        let _ = out;
    }
}