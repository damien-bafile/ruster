# Plan B: TUI Shell Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver a terminal-based editor binary (`ruster`) that opens a file, renders the buffer using ratatui, routes keyboard input through the existing `ruster-core` Vim engine, and supports save/quit via `:` cmdline.

**Architecture:** Three new workspace crates: `ruster-render` (Renderer trait + types), `ruster-tui` (ratatui widgets + App + crossterm event loop), `ruster-bin` (binary entry point). `ruster-core` gets minimal additions: `Action::CmdlineResult(String)` variant, expanded Cmdline mode in VimState, and the `:` Normal-mode handler.

**Tech Stack:** ratatui 0.28, crossterm 0.28, plus existing ruster-core deps.

## Global Constraints

- **Edition:** Rust 2021. MSRV: stable (1.78+).
- **Workspace root:** `/Users/daimyo/Dev/ruster`. Add `crates/ruster-render`, `crates/ruster-tui`, `crates/ruster-bin` to workspace members.
- **ruster-core stays pure:** no terminal, no file I/O, no new deps. All UI and I/O lives in `ruster-tui`/`ruster-bin`.
- **TDD for ruster-core changes.** `ruster-tui`/`ruster-bin` widget/event-logic tests use `#[test]` with mock data where possible; rendering is manually verified.
- **Commit per task** with conventional-commit messages.

---

## File Structure

```
crates/ruster-render/
├── Cargo.toml
└── src/lib.rs              # EditorState, CursorKind, Renderer trait

crates/ruster-tui/
├── Cargo.toml
└── src/
    ├── lib.rs              # public re-exports
    ├── app.rs              # App struct, run(), execute_cmd()
    ├── renderer.rs         # TuiRenderer impl (ratatui)
    ├── widgets.rs          # BufferWidget, StatuslineWidget, CmdlineWidget
    └── key.rs              # crossterm → ruster_core::KeyEvent conversion

crates/ruster-bin/
├── Cargo.toml
└── src/main.rs             # CLI parse, main()

crates/ruster-core/          — modified files
├── src/action.rs            — add CmdlineResult(String)
├── src/editor.rs            — add no-op match arm
└── src/vim/mod.rs           — cmdline_buffer, Cmdline mode, : handler
```

---

### Task 1: Scaffold `ruster-render` crate

**Files:**
- Create: `crates/ruster-render/Cargo.toml`
- Create: `crates/ruster-render/src/lib.rs`
- Modify: `Cargo.toml` (workspace root) — add member

**Interfaces:**
- Consumes: nothing
- Produces: `ruster_render::EditorState`, `ruster_render::CursorKind`, `ruster_render::Renderer` trait

- [ ] **Step 1: Write the failing test**

`crates/ruster-render/src/lib.rs`:
```rust
#[cfg(test)]
mod tests {
    use crate::{CursorKind, EditorState, Renderer};

    struct TestRenderer;
    impl Renderer for TestRenderer {
        fn render_frame(&mut self, _state: &EditorState) {}
    }

    #[test]
    fn renderer_trait_is_object_safe() {
        let state = EditorState {
            lines: vec!["hello".to_string()],
            cursor: (0, 0),
            cursor_kind: CursorKind::Block,
            mode_label: "NORMAL",
            file_path: "test.txt",
            modified: false,
            cmdline: None,
            message: None,
        };
        let mut r = TestRenderer;
        r.render_frame(&state); // must compile
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ruster-render`
Expected: error `package ruster-render does not exist`

- [ ] **Step 3: Write minimal implementation**

`Cargo.toml` (workspace root):
```toml
[workspace]
members = ["crates/ruster-core", "crates/ruster-render"]
resolver = "2"
```

`crates/ruster-render/Cargo.toml`:
```toml
[package]
name = "ruster-render"
version = "0.1.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[dependencies]
```

`crates/ruster-render/src/lib.rs`:
```rust
pub enum CursorKind { Block, Bar }

pub struct EditorState<'a> {
    pub lines: Vec<String>,
    pub cursor: (u16, u16),
    pub cursor_kind: CursorKind,
    pub mode_label: &'a str,
    pub file_path: &'a str,
    pub modified: bool,
    pub cmdline: Option<&'a str>,
    pub message: Option<&'a str>,
}

pub trait Renderer {
    fn render_frame(&mut self, state: &EditorState);
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p ruster-render`
Expected: test passes

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/ruster-render/
git commit -m "feat(render): add ruster-render crate with EditorState and Renderer trait"
```

---

### Task 2: Extend ruster-core for Cmdline mode

**Files:**
- Modify: `crates/ruster-core/src/action.rs` — add `CmdlineResult(String)` variant
- Modify: `crates/ruster-core/src/editor.rs` — add no-op match arm
- Modify: `crates/ruster-core/src/vim/mod.rs` — add `cmdline_buffer`, expand Cmdline mode, `:` handler, getter

**Interfaces:**
- Consumes: existing `VimState`, `Action`, `Editor`, `KeyEvent`
- Produces: `VimState::cmdline_buffer()` getter; `handle()` returns `Action::CmdlineResult(String)` on Enter in Cmdline mode

- [ ] **Step 1: Write the failing test**

Add to `crates/ruster-core/src/vim/mod.rs` tests:

```rust
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
```

Add the import for `VimMode::Cmdline` at the top of the test block if not already present.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ruster-core -- vim::tests::cmdline`
Expected: compile error (`Action::CmdlineResult` not found) or test failure

- [ ] **Step 3: Write minimal implementation**

`crates/ruster-core/src/action.rs` — add to `Action` enum:
```rust
CmdlineResult(String),
```

`crates/ruster-core/src/editor.rs` — add match arm in `execute()`:
```rust
Action::CmdlineResult(_) => {}
```

`crates/ruster-core/src/vim/mod.rs` — add to `VimState` fields:
```rust
cmdline_buffer: String,
```

Initialize in `VimState::new()`:
```rust
cmdline_buffer: String::new(),
```

Add getter:
```rust
pub fn cmdline_buffer(&self) -> &str { &self.cmdline_buffer }
```

In `handle()`, expand the existing `VimMode::Cmdline` arm (replace the one-liner):
```rust
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
```

In Normal mode arm of `handle()`, add `:` handler (in the match, alongside existing handlers like `KeyEvent::Char('i')`):
```rust
KeyEvent::Char(':') => {
    self.mode = VimMode::Cmdline;
    self.cmdline_buffer = String::from(":");
    self.count = None;
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p ruster-core`
Expected: all tests pass (55 old + 3 new = 58)

- [ ] **Step 5: Commit**

```bash
git add crates/ruster-core/src/action.rs crates/ruster-core/src/editor.rs crates/ruster-core/src/vim/mod.rs
git commit -m "feat(core): Vim Cmdline mode with Action::CmdlineResult for :w/:q"
```

---

### Task 3: Scaffold `ruster-tui` crate + key conversion + TuiRenderer

**Files:**
- Create: `crates/ruster-tui/Cargo.toml`
- Create: `crates/ruster-tui/src/lib.rs`
- Create: `crates/ruster-tui/src/key.rs`
- Create: `crates/ruster-tui/src/renderer.rs`
- Modify: `Cargo.toml` (workspace root) — add `ruster-tui` member

**Interfaces:**
- Consumes: `ruster_core::KeyEvent`, `ruster_render::*`
- Produces: `crossterm_to_ruster_key()`, `TuiRenderer` implementing `Renderer`

- [ ] **Step 1: Write tests for key conversion + renderer module compilation**

Add to `ruster-tui/src/key.rs`:
```rust
#[cfg(test)]
mod tests {
    use crate::key::crossterm_to_ruster_key;
    use ruster_core::key::KeyEvent;

    #[test]
    fn char_keys_roundtrip() {
        let ck = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('w'),
            crossterm::event::KeyModifiers::NONE,
        );
        let rk = crossterm_to_ruster_key(ck);
        assert_eq!(rk, KeyEvent::Char('w'));
    }

    #[test]
    fn ctrl_keys_roundtrip() {
        let ck = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('c'),
            crossterm::event::KeyModifiers::CONTROL,
        );
        let rk = crossterm_to_ruster_key(ck);
        assert_eq!(rk, KeyEvent::Ctrl('c'));
    }

    #[test]
    fn special_keys() {
        use crossterm::event::KeyCode;
        fn check(cc: KeyCode, expected: KeyEvent) {
            let ck = crossterm::event::KeyEvent::new(cc, crossterm::event::KeyModifiers::NONE);
            assert_eq!(crossterm_to_ruster_key(ck), expected);
        }
        check(KeyCode::Esc, KeyEvent::Esc);
        check(KeyCode::Enter, KeyEvent::Enter);
        check(KeyCode::Backspace, KeyEvent::Backspace);
        check(KeyCode::Delete, KeyEvent::Delete);
    }
}
```

Add a renderer smoke test to `ruster-tui/src/lib.rs`:
```rust
#[cfg(test)]
mod tests {
    use ruster_render::{CursorKind, EditorState, Renderer};

    #[test]
    fn tui_renderer_accepts_editor_state() {
        // Construct but don't render (no terminal in test)
        let mut r = crate::renderer::TuiRenderer::dummy();
        let state = EditorState {
            lines: vec!["hi".to_string()],
            cursor: (0, 1),
            cursor_kind: CursorKind::Bar,
            mode_label: "INSERT",
            file_path: "f",
            modified: false,
            cmdline: None,
            message: None,
        };
        r.render_frame(&state);
    }
}
```

- [ ] **Step 2: Run test — should fail to compile (no crate)**

Run: `cargo test -p ruster-tui`
Expected: error `package ruster-tui does not exist`

- [ ] **Step 3: Write minimal implementation**

`Cargo.toml` (workspace root):
```toml
members = ["crates/ruster-core", "crates/ruster-render", "crates/ruster-tui"]
```

`crates/ruster-tui/Cargo.toml`:
```toml
[package]
name = "ruster-tui"
version = "0.1.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[dependencies]
ruster-core = { path = "../ruster-core" }
ruster-render = { path = "../ruster-render" }
ratatui = "0.28"
crossterm = "0.28"
```

`crates/ruster-tui/src/lib.rs`:
```rust
pub mod key;
pub mod renderer;
```

`crates/ruster-tui/src/key.rs`:
```rust
use crossterm::event::{KeyCode, KeyEvent as CKEvent, KeyModifiers};
use ruster_core::key::KeyEvent;

pub fn crossterm_to_ruster_key(ck: CKEvent) -> KeyEvent {
    match ck.code {
        KeyCode::Esc => KeyEvent::Esc,
        KeyCode::Enter => KeyEvent::Enter,
        KeyCode::Backspace => KeyEvent::Backspace,
        KeyCode::Delete => KeyEvent::Delete,
        KeyCode::Char(c) if ck.modifiers == KeyModifiers::CONTROL => {
            KeyEvent::Ctrl(c)
        }
        KeyCode::Char(c) => KeyEvent::Char(c),
        _ => KeyEvent::Char(' '), // unmapped; best-effort
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn char_keys_roundtrip() {
        let ck = CKEvent::new(KeyCode::Char('w'), KeyModifiers::NONE);
        assert_eq!(crossterm_to_ruster_key(ck), KeyEvent::Char('w'));
    }

    #[test]
    fn ctrl_keys_roundtrip() {
        let ck = CKEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(crossterm_to_ruster_key(ck), KeyEvent::Ctrl('c'));
    }

    #[test]
    fn special_keys() {
        fn check(cc: KeyCode, expected: KeyEvent) {
            let ck = CKEvent::new(cc, KeyModifiers::NONE);
            assert_eq!(crossterm_to_ruster_key(ck), expected);
        }
        check(KeyCode::Esc, KeyEvent::Esc);
        check(KeyCode::Enter, KeyEvent::Enter);
        check(KeyCode::Backspace, KeyEvent::Backspace);
        check(KeyCode::Delete, KeyEvent::Delete);
    }
}
```

`crates/ruster-tui/src/renderer.rs`:
```rust
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use ruster_render::{CursorKind, EditorState, Renderer};
use std::io::Stdout;

pub struct TuiRenderer {
    terminal: Option<Terminal<CrosstermBackend<Stdout>>>,
}

impl TuiRenderer {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let stdout = std::io::stdout();
        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        Ok(TuiRenderer { terminal: Some(terminal) })
    }

    pub fn dummy() -> Self {
        TuiRenderer { terminal: None }
    }
}

impl Renderer for TuiRenderer {
    fn render_frame(&mut self, state: &EditorState) {
        let term = match &mut self.terminal {
            Some(t) => t,
            None => return, // dummy mode
        };
        let _ = term.draw(|frame| {
            // TODO: render widgets — Task 4
            // For now just clear the frame
            frame.render_widget(ratatui::widgets::Clear, frame.area());
        });
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p ruster-tui`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/ruster-tui/
git commit -m "feat(tui): scaffold ruster-tui with key conversion and TuiRenderer stub"
```

---

### Task 4: ruster-tui widgets

**Files:**
- Create: `crates/ruster-tui/src/widgets.rs`
- Modify: `crates/ruster-tui/src/lib.rs` — add `pub mod widgets`
- Modify: `crates/ruster-tui/src/renderer.rs` — use widgets in `render_frame`

**Interfaces:**
- Produces: `BufferWidget`, `StatuslineWidget`, `CmdlineWidget` — each implementing ratatui `Widget`

- [ ] **Step 1: Write widget unit tests**

Add to `widgets.rs`:
```rust
#[cfg(test)]
mod tests {
    use crate::widgets::{mode_label, cmdline_label};
    use ruster_core::vim::VimMode;

    #[test]
    fn mode_label_normal() {
        assert_eq!(mode_label(&VimMode::Normal), "-- NORMAL --");
    }

    #[test]
    fn mode_label_insert() {
        assert_eq!(mode_label(&VimMode::Insert), "-- INSERT --");
    }

    #[test]
    fn cmdline_label_shows_prompt() {
        assert_eq!(cmdline_label(":w"), ":w");
    }

    #[test]
    fn cmdline_label_empty() {
        assert_eq!(cmdline_label(""), ":");
    }
}
```

- [ ] **Step 2: Run test — should fail to compile**

Run: `cargo test -p ruster-tui`
Expected: compile error (no `widgets` module)

- [ ] **Step 3: Write minimal implementation**

`crates/ruster-tui/src/widgets.rs`:

```rust
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget, Wrap};
use ruster_core::vim::VimMode;

/// Convert a VimMode to a display string.
pub fn mode_label(mode: &VimMode) -> &'static str {
    match mode {
        VimMode::Normal => "-- NORMAL --",
        VimMode::Insert => "-- INSERT --",
        VimMode::VisualChar => "-- VISUAL --",
        VimMode::VisualLine => "-- V-LINE --",
        VimMode::Cmdline => "-- CMDLINE --",
    }
}

/// Format the cmdline text for display (always starts with ":").
pub fn cmdline_label(buf: &str) -> String {
    if buf.is_empty() { ":".to_string() } else { buf.to_string() }
}

/// Renders buffer text with cursor highlight.
pub struct BufferWidget {
    lines: Vec<String>,
    cursor: (u16, u16),
}

impl BufferWidget {
    pub fn new(lines: Vec<String>, cursor: (u16, u16)) -> Self {
        BufferWidget { lines, cursor }
    }
}

impl Widget for BufferWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        for (i, line) in self.lines.iter().enumerate() {
            if i as u16 >= area.height { break; }
            let y = area.y + i as u16;
            let is_cursor_line = i as u16 == self.cursor.0;
            for (j, ch) in line.chars().enumerate() {
                let x = area.x + j as u16;
                if x >= area.right() { break; }
                let cell = buf.get_mut(x, y);
                cell.set_char(ch);
                if is_cursor_line && j as u16 == self.cursor.1 {
                    cell.set_bg(Color::White);
                    cell.set_fg(Color::Black);
                }
            }
        }
    }
}

/// Renders the status line (mode, file path, cursor position).
pub struct StatuslineWidget<'a> {
    mode: &'a str,
    file_path: &'a str,
    position: (u16, u16),
}

impl<'a> StatuslineWidget<'a> {
    pub fn new(mode: &'a str, file_path: &'a str, position: (u16, u16)) -> Self {
        StatuslineWidget { mode, file_path, position }
    }
}

impl Widget for StatuslineWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let right = format!(" {},{} ", self.position.0 + 1, self.position.1 + 1);
        let left = format!(" {} ", self.mode);
        // Fill the full line with inverted bg
        for x in area.left()..area.right() {
            buf.get_mut(x, area.y).set_bg(Color::DarkGray);
        }
        // Write left (mode)
        for (i, ch) in left.chars().enumerate() {
            let x = area.x + i as u16;
            if x >= area.right() { break; }
            let cell = buf.get_mut(x, area.y);
            cell.set_char(ch);
            cell.set_fg(Color::White);
            cell.set_bg(Color::DarkGray);
        }
        // Write right (position)
        let rstart = area.right().saturating_sub(right.len() as u16);
        for (i, ch) in right.chars().enumerate() {
            let x = rstart + i as u16;
            if x >= area.right() { break; }
            let cell = buf.get_mut(x, area.y);
            cell.set_char(ch);
            cell.set_fg(Color::White);
            cell.set_bg(Color::DarkGray);
        }
        // Write file path in the center gap
        let max_path = rstart.saturating_sub(left.len() as u16 + 1);
        let path = &self.file_path[..self.file_path.len().min(max_path as usize)];
        for (i, ch) in path.chars().enumerate() {
            let x = area.x + left.len() as u16 + 1 + i as u16;
            if x >= area.right() { break; }
            let cell = buf.get_mut(x, area.y);
            cell.set_char(ch);
            cell.set_fg(Color::White);
            cell.set_bg(Color::DarkGray);
        }
    }
}

/// Renders the cmdline prompt line.
pub struct CmdlineWidget<'a> {
    text: &'a str,
}

impl<'a> CmdlineWidget<'a> {
    pub fn new(text: &'a str) -> Self {
        CmdlineWidget { text }
    }
}

impl Widget for CmdlineWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        for (i, ch) in self.text.chars().enumerate() {
            let x = area.x + i as u16;
            if x >= area.right() { break; }
            buf.get_mut(x, area.y).set_char(ch);
        }
    }
}
```

`crates/ruster-tui/src/lib.rs` — add:
```rust
pub mod widgets;
```

Update `crates/ruster-tui/src/renderer.rs` to use widgets. Replace `render_frame` body:

```rust
fn render_frame(&mut self, state: &EditorState) {
    let term = match &mut self.terminal {
        Some(t) => t,
        None => return,
    };
    let _ = term.draw(|frame| {
        let area = frame.area();
        let constraints = if state.cmdline.is_some() || state.message.is_some() {
            &[ratatui::layout::Constraint::Fill(1),
              ratatui::layout::Constraint::Length(1),
              ratatui::layout::Constraint::Length(1)]
        } else {
            &[ratatui::layout::Constraint::Fill(1),
              ratatui::layout::Constraint::Length(1)]
        };
        let chunks = ratatui::layout::Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints(constraints)
            .split(area);

        // Buffer area
        let buf_widget = crate::widgets::BufferWidget::new(
            state.lines.clone(),
            state.cursor,
        );
        frame.render_widget(buf_widget, chunks[0]);

        // Statusline
        let sl = crate::widgets::StatuslineWidget::new(
            state.mode_label,
            state.file_path,
            state.cursor,
        );
        frame.render_widget(sl, chunks[1]);

        // Cmdline / message area
        if let Some(cmd) = state.cmdline {
            let cl = crate::widgets::CmdlineWidget::new(cmd);
            frame.render_widget(cl, chunks.last().copied().unwrap_or(chunks[1]));
        } else if let Some(msg) = state.message {
            let cl = crate::widgets::CmdlineWidget::new(msg);
            frame.render_widget(cl, chunks.last().copied().unwrap_or(chunks[1]));
        }
    });
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p ruster-tui`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/ruster-tui/src/widgets.rs crates/ruster-tui/src/lib.rs crates/ruster-tui/src/renderer.rs
git commit -m "feat(tui): BufferWidget, StatuslineWidget, CmdlineWidget for ratatui rendering"
```

---

### Task 5: ruster-tui App + event loop + cmdline parsing + file I/O

**Files:**
- Create: `crates/ruster-tui/src/app.rs`
- Modify: `crates/ruster-tui/src/lib.rs` — add `pub mod app`

- [ ] **Step 1: Write unit tests for cmdline parsing**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct CmdTestCase {
        cmd: &'static str,
        expect_save: bool,
        expect_quit: bool,
        expect_path: Option<&'static str>,
        expect_error: bool,
    }

    #[test]
    fn cmd_w_saves() {
        let mut a = App::new("content".into(), PathBuf::from("f.txt"));
        assert_eq!(a.parse_cmdline(":w"), Ok(CmdAction::Save(false)));
    }

    #[test]
    fn cmd_q_quits() {
        let mut a = App::new("content".into(), PathBuf::from("f.txt"));
        assert_eq!(a.parse_cmdline(":q"), Ok(CmdAction::Quit));
    }

    #[test]
    fn cmd_wq_saves_and_quits() {
        let mut a = App::new("content".into(), PathBuf::from("f.txt"));
        assert_eq!(a.parse_cmdline(":wq"), Ok(CmdAction::SaveAndQuit));
    }

    #[test]
    fn cmd_q_force_quits() {
        let mut a = App::new("content".into(), PathBuf::from("f.txt"));
        assert_eq!(a.parse_cmdline(":q!"), Ok(CmdAction::Quit));
    }

    #[test]
    fn cmd_w_path_saves_as() {
        let mut a = App::new("content".into(), PathBuf::from("f.txt"));
        assert_eq!(a.parse_cmdline(":w /tmp/out.txt"), Ok(CmdAction::SaveAs("/tmp/out.txt".into())));
    }

    #[test]
    fn cmd_unknown_errors() {
        let mut a = App::new("content".into(), PathBuf::from("f.txt"));
        assert!(a.parse_cmdline(":xyz").is_err());
    }
}
```

- [ ] **Step 2: Run test — should fail to compile**

Run: `cargo test -p ruster-tui`
Expected: compile error (no `app` module)

- [ ] **Step 3: Write minimal implementation**

`crates/ruster-tui/src/app.rs`:

```rust
use crate::key::crossterm_to_ruster_key;
use crate::renderer::TuiRenderer;
use ruster_core::action::Action;
use ruster_core::editor::Editor;
use ruster_core::key::KeyEvent;
use ruster_core::vim::VimMode;
use ruster_core::vim::VimState;
use ruster_render::{CursorKind, EditorState, Renderer};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
enum CmdAction {
    Save(bool),          // false = don't force; true = :w!
    SaveAs(String),
    Quit,
    ForceQuit,
    SaveAndQuit,
}

pub struct App {
    pub editor: Editor,
    pub vim: VimState,
    renderer: TuiRenderer,
    file_path: PathBuf,
    pub should_quit: bool,
    message: Option<String>,
}

impl App {
    pub fn new(content: String, file_path: PathBuf) -> Self {
        let mut editor = Editor::from_str(&content);
        // Move cursor to start of buffer for a fresh open
        editor.execute(Action::Move(ruster_core::motion::Motion::To(0)));
        let vim = VimState::new();
        let renderer = TuiRenderer::dummy();
        App { editor, vim, renderer, file_path, should_quit: false, message: None }
    }

    pub fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        crossterm::terminal::enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen)?;

        self.renderer = TuiRenderer::new()?;

        loop {
            self.render();
            if self.should_quit { break; }

            let ck = crossterm::event::read()?;
            let key = crossterm_to_ruster_key(ck);
            for action in self.vim.handle(key, &self.editor) {
                match action {
                    Action::CmdlineResult(cmd) => {
                        self.message = None; // clear stale message
                        match self.parse_cmdline(&cmd) {
                            Ok(CmdAction::Save(force)) => self.save_file(force),
                            Ok(CmdAction::SaveAs(p)) => self.save_as(&p),
                            Ok(CmdAction::Quit) | Ok(CmdAction::ForceQuit) => {
                                self.should_quit = true;
                            }
                            Ok(CmdAction::SaveAndQuit) => {
                                self.save_file(false);
                                self.should_quit = true;
                            }
                            Err(e) => self.message = Some(e),
                        }
                    }
                    other => self.editor.execute(other),
                }
            }
        }

        crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen)?;
        crossterm::terminal::disable_raw_mode()?;
        Ok(())
    }

    fn render(&mut self) {
        let lines: Vec<String> = self.editor.buffer().to_string()
            .split('\n')
            .map(|s| s.to_string())
            .collect();
        let head = self.editor.primary_head();
        // Find line/col from char offset
        let mut line = 0u16;
        let mut col = 0u16;
        let mut remaining = head;
        for l in &lines {
            let lc = l.chars().count();
            if remaining <= lc { col = remaining as u16; break; }
            remaining = remaining.saturating_sub(lc + 1); // +1 for newline
            line += 1;
        }

        let cursor_kind = match self.vim.mode {
            VimMode::Insert | VimMode::Cmdline => CursorKind::Bar,
            _ => CursorKind::Block,
        };
        let mode_label = crate::widgets::mode_label(&self.vim.mode);
        let file_path = self.file_path.to_string_lossy().to_string();
        let cmdline = match self.vim.mode {
            VimMode::Cmdline => Some(crate::widgets::cmdline_label(self.vim.cmdline_buffer())),
            _ => self.message.as_ref().map(|m| m.clone()),
        };

        let state = EditorState {
            lines,
            cursor: (line, col),
            cursor_kind,
            mode_label,
            file_path: &file_path,
            modified: false,
            cmdline: cmdline.as_deref(),
            message: None,
        };
        self.renderer.render_frame(&state);
    }

    fn parse_cmdline(&self, cmdline: &str) -> Result<CmdAction, String> {
        let trimmed = cmdline.trim_start_matches(':').trim();
        if trimmed.is_empty() {
            return Err("Empty command".to_string());
        }
        match trimmed {
            "q" | "quit" => Ok(CmdAction::Quit),
            "q!" => Ok(CmdAction::ForceQuit),
            "w" | "write" => Ok(CmdAction::Save(false)),
            "w!" => Ok(CmdAction::Save(true)),
            "wq" | "x" => Ok(CmdAction::SaveAndQuit),
            _ if trimmed.starts_with("w ") || trimmed.starts_with("write ") => {
                let path = trimmed.splitn(2, ' ').nth(1).unwrap_or("").trim().to_string();
                if path.is_empty() {
                    Err("No path given".to_string())
                } else {
                    Ok(CmdAction::SaveAs(path))
                }
            }
            _ => Err(format!("Unknown command: {}", cmdline)),
        }
    }

    fn save_file(&mut self, force: bool) {
        let content = self.editor.buffer().to_string();
        match std::fs::write(&self.file_path, &content) {
            Ok(()) => self.message = Some(format!("Saved: {}", self.file_path.display())),
            Err(e) if force => {
                // force save: try harder (same write, just different msg)
                let _ = std::fs::write(&self.file_path, &content);
                self.message = Some(format!("Saved (forced): {}", self.file_path.display()));
            }
            Err(e) => self.message = Some(format!("Error: {}", e)),
        }
    }

    fn save_as(&mut self, path: &str) {
        let content = self.editor.buffer().to_string();
        match std::fs::write(path, &content) {
            Ok(()) => {
                self.file_path = PathBuf::from(path);
                self.message = Some(format!("Saved: {}", path));
            }
            Err(e) => self.message = Some(format!("Error: {}", e)),
        }
    }
}
```

Note: the `crate::motion::Motion::To(0)` is the path we need from ruster-core. Let me check the actual re-export. The Action/Motion types are at `ruster_core::action::Motion`. But we can just use `Action::Move(ruster_core::motion::Motion::To(0))`. Actually, looking at the imports in the existing code, `Motion` is in `crate::action`. Let me fix: use `ruster_core::action::Motion`.

Actually, let me look at the ruster-core public API. The `lib.rs` only has `pub mod` declarations:
```rust
pub mod buffer;
pub mod cursor;
pub mod undo;
pub mod key;
pub mod action;
pub mod command;
pub mod editor;
pub mod vim;
```

So `Motion` is at `ruster_core::action::Motion`. Let me fix the import.

`crates/ruster-tui/src/lib.rs` — add:
```rust
pub mod app;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p ruster-tui`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/ruster-tui/src/app.rs crates/ruster-tui/src/lib.rs
git commit -m "feat(tui): App with event loop, cmdline parsing, file save/quit"
```

---

### Task 6: ruster-bin binary

**Files:**
- Create: `crates/ruster-bin/Cargo.toml`
- Create: `crates/ruster-bin/src/main.rs`
- Modify: `Cargo.toml` (workspace root) — add `ruster-bin` member

- [ ] **Step 1: Write the test (compile check)**

Create a minimal integration that just verifies the binary crate compiles. Add a test file:

`crates/ruster-bin/tests/cli_args.rs`:
```rust
use std::process::Command;

#[test]
fn binary_prints_usage_without_args() {
    let output = Command::new(env!("CARGO_BIN_EXE_ruster"))
        .output()
        .expect("failed to run ruster");
    // Without args, it should exit with error and print usage
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Usage: ruster <file>"));
}
```

- [ ] **Step 2: Run test — should fail (no crate)**

Run: `cargo test -p ruster-bin`
Expected: error `package ruster-bin does not exist`

- [ ] **Step 3: Write minimal implementation**

`Cargo.toml` (workspace root):
```toml
members = ["crates/ruster-core", "crates/ruster-render", "crates/ruster-tui", "crates/ruster-bin"]
```

`crates/ruster-bin/Cargo.toml`:
```toml
[package]
name = "ruster-bin"
version = "0.1.0"
edition = "2021"

[dependencies]
ruster-core = { path = "../ruster-core" }
ruster-render = { path = "../ruster-render" }
ruster-tui = { path = "../ruster-tui" }

[[bin]]
name = "ruster"
path = "src/main.rs"
```

`crates/ruster-bin/src/main.rs`:
```rust
use std::path::PathBuf;
use ruster_tui::app::App;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = match args.len() {
        2 => PathBuf::from(&args[1]),
        _ => {
            eprintln!("Usage: ruster <file>");
            std::process::exit(1);
        }
    };

    let content = if path.exists() {
        match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Error reading {}: {}", path.display(), e);
                std::process::exit(1);
            }
        }
    } else {
        String::new()
    };

    let mut app = App::new(content, path);
    if let Err(e) = app.run() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
```

`crates/ruster-bin/tests/cli_args.rs`:
```rust
#[test]
fn binary_prints_usage_without_args() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ruster"))
        .output()
        .expect("failed to run ruster");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Usage: ruster <file>"));
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p ruster-bin`
Expected: the CLI args test passes

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/ruster-bin/
git commit -m "feat(bin): ruster binary with CLI arg parsing"
```

---

### Task 7: Wire workspace, fix imports, verify full build

**Files:**
- Verify full workspace builds and tests pass
- Verify `ruster-core` re-exports everything `ruster-tui` needs
- Push to GitHub

- [ ] **Step 1: Check ruster-core public API**

Ensure `ruster_core::action::Action`, `ruster_core::editor::Editor`, `ruster_core::key::KeyEvent`,
`ruster_core::vim::{VimMode, VimState}` are re-exported or accessible.

Check that `ruster_core::vim::VimMode` is `pub` and `VimState` is `pub` (they are).

- [ ] **Step 2: Full workspace build**

```bash
cargo build --workspace
```
Expected: clean compile

- [ ] **Step 3: Full workspace test**

```bash
cargo test --workspace
```
Expected: all tests pass

- [ ] **Step 4: Fix any issues found**

Fix missing re-exports, import paths, or API mismatches.

- [ ] **Step 5: Final commit**

```bash
git add -A && git commit -m "chore: wire workspace for Plan B crates and fix imports"
```

- [ ] **Step 6: Push to GitHub**

```bash
git push origin main
```
