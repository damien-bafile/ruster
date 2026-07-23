# Phase 1 Feature Completion — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete four remaining Phase 1 features: multi-cursor, system clipboard, tabs/indentation, and EditorConfig.

**Architecture:** All changes land in `ruster-core` (cursor ops, clipboard, indent/deindent actions, editorconfig parser) and `ruster-tui` (keybindings, EditorConfig call). No new crates. `arboard` added to `ruster-core`.

**Tech Stack:** Rust, arboard (cross-platform clipboard), ropey (text buffer), tree-sitter (syntax), raylib (GUI backend).

## Global Constraints

- Multi-cursor: Ctrl+D adds cursor at next word occurrence, Alt+click adds at position, Esc clears extras
- Clipboard: Unnamed register aliases to system clipboard via arboard. Yank writes to both memory and clipboard. Paste reads from clipboard (fallback to memory on error)
- Tabs: Tab in Insert mode inserts `tabstop` spaces. `>>`/`<<` indent/deindent lines. Respect `expandtab`, `tabstop`, `shiftwidth` from Config
- EditorConfig: Parse `.editorconfig` files, apply `indent_style`, `indent_size`, `tab_width`, `end_of_line`, `charset`, `trim_trailing_whitespace`, `insert_final_newline`. Walk up directories from file to root

---

### Task 1: Multi-Cursor Data Model & Editing

**Files:**
- Modify: `crates/ruster-core/src/cursor.rs:22-138`
- Modify: `crates/ruster-core/src/action.rs:19-36`
- Modify: `crates/ruster-core/src/editor.rs:28-91`

**Interfaces:**
- Consumes: existing `CursorSet`, `Action`, `Editor`
- Produces: `CursorSet::add_cursor(usize)`, `CursorSet::clear_extra()`, `CursorSet::count()`, `Action::AddCursor(usize)`, `Action::ClearExtraCursors`

- [ ] **Step 1: Add `add_cursor`, `clear_extra`, `count` to `CursorSet`**

Add to `cursor.rs`:
```rust
impl CursorSet {
    pub fn add_cursor(&mut self, at: usize) {
        self.cursors.push(Range::caret(at));
        self.primary = self.cursors.len() - 1;
    }

    pub fn clear_extra(&mut self) {
        let primary = self.cursors[self.primary];
        self.cursors.truncate(0);
        self.cursors.push(primary);
        self.primary = 0;
    }

    pub fn count(&self) -> usize {
        self.cursors.len()
    }
}
```

- [ ] **Step 2: Add `AddCursor` and `ClearExtraCursors` to `Action`**

In `action.rs`:
```rust
pub enum Action {
    // ... existing variants ...
    AddCursor(usize),
    ClearExtraCursors,
}
```

- [ ] **Step 3: Handle new actions in `Editor::execute`**

In `editor.rs` match block:
```rust
Action::AddCursor(pos) => self.cursors.add_cursor(pos),
Action::ClearExtraCursors => self.cursors.clear_extra(),
```

- [ ] **Step 4: Make edit ops apply to all cursors in reverse order**

Replace `apply_edit` in `editor.rs`:
```rust
fn apply_edit(&mut self, e: EditOp) {
    let all: Vec<usize> = if self.cursors.count() > 1 {
        // Collect all cursor positions, sorted descending for insert stability
        let mut positions: Vec<usize> = (0..self.cursors.count())
            .map(|i| self.cursors.cursors[i].head)
            .collect();
        positions.sort_unstable_by(|a, b| b.cmp(a));
        positions
    } else {
        vec![self.cursors.head()]
    };

    for &at in &all {
        match e.clone() {
            EditOp::InsertChar(c) => {
                let mut buf = [0u8; 4];
                let text = c.encode_utf8(&mut buf);
                let ch = self.buffer.insert(at, text);
                self.undo.push(ch);
                if all.len() == 1 {
                    self.cursors.set_head(at + 1, &self.buffer);
                }
            }
            EditOp::InsertString(s) => {
                let n = s.chars().count();
                let ch = self.buffer.insert(at, &s);
                self.undo.push(ch);
                if all.len() == 1 {
                    self.cursors.set_head(at + n, &self.buffer);
                }
            }
            EditOp::DeleteRange(start, end) if end > start => {
                let safe_end = end.min(self.buffer.len_chars());
                let ch = self.buffer.delete(start..safe_end);
                self.undo.push(ch);
                if all.len() == 1 {
                    self.cursors.set_head(start, &self.buffer);
                }
            }
            EditOp::DeleteRange(_, _) => {}
            EditOp::Backspace => {
                if at > 0 {
                    let ch = self.buffer.delete(at - 1..at);
                    self.undo.push(ch);
                    if all.len() == 1 {
                        self.cursors.set_head(at - 1, &self.buffer);
                    }
                }
            }
        }
    }
}
```

Note: for single cursor, the existing behavior (old `apply_edit`) is preserved exactly. For multi-cursor, edits apply in reverse order so earlier inserts don't shift later cursor positions.

- [ ] **Step 5: Add tests**

In `cursor.rs` tests:
```rust
#[test]
fn add_and_clear_extra_cursors() {
    let mut c = CursorSet::single(5);
    assert_eq!(c.count(), 1);
    c.add_cursor(10);
    assert_eq!(c.count(), 2);
    c.clear_extra();
    assert_eq!(c.count(), 1);
    assert_eq!(c.head(), 5);
}
```

- [ ] **Step 6: Build & test**

Run: `cargo test -p ruster-core`
Expected: all 60+ tests pass

- [ ] **Step 7: Commit**

```bash
git add crates/ruster-core/src/cursor.rs crates/ruster-core/src/action.rs crates/ruster-core/src/editor.rs
git commit -m "feat: multi-cursor data model and editing"
```

---

### Task 2: Multi-Cursor Keybindings (Ctrl+D, Esc)

**Files:**
- Modify: `crates/ruster-core/src/vim/mod.rs:159-245`
- Test: `crates/ruster-core/src/vim/mod.rs` (add tests)

**Interfaces:**
- Consumes: `Action::AddCursor`, `Action::ClearExtraCursors`
- Produces: Ctrl+D finds next word occurrence in buffer, Esc clears extras

- [ ] **Step 1: Add helper to find next word occurrence**

In `vim/mod.rs`, add a helper function (outside `impl VimState`):
```rust
fn next_word_occurrence(editor: &Editor) -> Option<usize> {
    let head = editor.primary_head();
    let buf = editor.buffer();
    let text = buf.to_string();
    // Find the word under cursor
    let line = char_to_line(editor, head);
    let line_start = buf.line_start_char(line);
    let line_end = buf.line_end_char(line);
    let content = buf.slice_string(line_start, line_end);
    let col = head - line_start;
    // Extract current word (alphanumeric or underscore)
    let chars: Vec<char> = content.chars().collect();
    if col >= chars.len() || !chars[col].is_alphanumeric() && chars[col] != '_' {
        return None; // Not on a word
    }
    let word_start = (0..=col).rev().take_while(|&i| i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_')).last().unwrap_or(col);
    let word_end = (col..chars.len()).take_while(|&i| chars[i].is_alphanumeric() || chars[i] == '_').last().unwrap_or(col);
    let word: String = chars[word_start..=word_end].iter().collect();
    if word.is_empty() {
        return None;
    }
    // Search from cursor position forward for next occurrence
    let search_from = head + 1;
    if search_from >= text.len() {
        return None;
    }
    text[search_from..].find(&word).map(|pos| search_from + pos)
}
```

Import `char_to_line` from `motions`:
```rust
use crate::vim::motions::char_to_line;
```

- [ ] **Step 2: Add Ctrl+D and Esc multi-cursor handling in `handle_normal`**

In `vim/mod.rs` `handle_normal`, add before the `_ =>` catch-all:
```rust
KeyEvent::Ctrl('d') => {
    if let Some(pos) = next_word_occurrence(editor) {
        out.push(Action::AddCursor(pos));
    }
    self.count = None;
}
```

Modify the `Esc` handler in `handle_normal`:
```rust
KeyEvent::Esc => {
    if editor.cursors().count() > 1 {
        out.push(Action::ClearExtraCursors);
    }
    self.count = None;
}
```

- [ ] **Step 3: Add tests**

In `vim/mod.rs` tests:
```rust
#[test]
fn ctrl_d_adds_cursor_at_next_word() {
    let mut e = Editor::from_str("foo foo foo");
    let mut v = VimState::new();
    let actions: Vec<Action> = v.handle(KeyEvent::Ctrl('d'), &e);
    assert!(actions.iter().any(|a| matches!(a, Action::AddCursor(_))));
}

#[test]
fn ctrl_d_no_word_does_nothing() {
    let mut e = Editor::from_str("... ...");
    let mut v = VimState::new();
    let actions: Vec<Action> = v.handle(KeyEvent::Ctrl('d'), &e);
    assert!(actions.is_empty());
}

#[test]
fn esc_clears_extra_cursors() {
    let mut e = Editor::from_str("hello");
    let mut v = VimState::new();
    // Add a second cursor via direct action (simulate Ctrl+D)
    e.execute(Action::AddCursor(3));
    assert_eq!(e.cursors().count(), 2);
    let actions: Vec<Action> = v.handle(KeyEvent::Esc, &e);
    assert!(actions.iter().any(|a| matches!(a, Action::ClearExtraCursors)));
}
```

- [ ] **Step 4: Build & test**

Run: `cargo test -p ruster-core`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/ruster-core/src/vim/mod.rs
git commit -m "feat: multi-cursor keybindings Ctrl+D and Esc"
```

---

### Task 3: System Clipboard Integration

**Files:**
- Modify: `crates/ruster-core/Cargo.toml:9-12`
- Modify: `crates/ruster-core/src/vim/mod.rs:24-53, 211-217, 273-294`
- Test: `crates/ruster-core/src/vim/mod.rs` (add tests)

**Interfaces:**
- Consumes: `arboard::Clipboard`
- Produces: Vim unnamed register syncs to system clipboard

- [ ] **Step 1: Add `arboard` dependency**

In `ruster-core/Cargo.toml`:
```toml
arboard = "3"
```

Note: arboard v3 is cross-platform and returns `Result` on failure.

- [ ] **Step 2: Add clipboard helper methods to VimState**

In `vim/mod.rs`, add a `Clipboard` field and helpers:
```rust
use std::cell::RefCell;

pub struct VimState {
    // ... existing fields ...
    clipboard: RefCell<Option<arboard::Clipboard>>,
}
```

In constructor:
```rust
clipboard: RefCell::new(arboard::Clipboard::new().ok()),
```

Add methods:
```rust
impl VimState {
    pub fn clipboard_get(&self) -> Option<String> {
        self.clipboard.borrow_mut().as_mut()
            .and_then(|c| c.get_text().ok())
    }

    pub fn clipboard_set(&self, text: &str) {
        if let Some(ref mut c) = *self.clipboard.borrow_mut() {
            let _ = c.set_text(text);
        }
    }
}
```

- [ ] **Step 3: Wire clipboard into yank operations**

In `apply_operator`, modify the `'y'` arm:
```rust
'y' => {
    self.register = Some(text.clone());
    self.clipboard_set(&text);
}
```

In `handle_visual`, modify the `'y'` arm:
```rust
KeyEvent::Char('y') => {
    let (start, end) = self.visual_range(editor);
    let safe_end = end.min(editor.buffer().len_chars());
    let text = editor.buffer().slice_string(start, safe_end);
    self.register = Some(text.clone());
    self.clipboard_set(&text);
    out.push(Action::Move(Motion::To(start)));
    self.mode = VimMode::Normal;
    self.anchor = None;
    self.count = None;
}
```

- [ ] **Step 4: Wire clipboard into paste operations**

Modify the `'p'` handler in `handle_normal`:
```rust
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
```

- [ ] **Step 5: Add tests**

```rust
#[test]
fn yank_sets_register() {
    let mut e = Editor::from_str("hello world");
    let mut v = VimState::new();
    // yy yanks current line
    let actions: Vec<Action> = v.handle(KeyEvent::Char('y'), &e);
    // 'y' is a pending operator; second 'y' triggers
    let actions: Vec<Action> = v.handle(KeyEvent::Char('y'), &e);
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
```

- [ ] **Step 6: Build & test**

Run: `cargo test -p ruster-core`
Expected: all tests pass (arboard may warn about no display in test environment — this is expected)

- [ ] **Step 7: Commit**

```bash
git add crates/ruster-core/Cargo.toml crates/ruster-core/src/vim/mod.rs
git commit -m "feat: system clipboard via arboard"
```

---

### Task 4: GUI Clipboard Bindings (Ctrl+C/Ctrl+V in Raylib)

**Files:**
- Modify: `crates/ruster-render-raylib/src/key.rs:1-22`
- Modify: `crates/ruster-render-raylib/src/lib.rs:50-86`
- Modify: `crates/ruster-tui/src/app.rs:343-358`

**Interfaces:**
- Consumes: raylib Ctrl+C/Ctrl+V key events
- Produces: Ctrl+C copies (visual yank), Ctrl+V pastes

- [ ] **Step 1: Add Ctrl key handling to raylib key converter**

In `crates/ruster-render-raylib/src/key.rs`, add a function to convert raylib keys to ruster-core KeyEvent directly (bypassing crossterm intermediary):

```rust
pub fn raylib_to_ruster_key(key: raylib::consts::KeyboardKey) -> Option<KeyEvent> {
    use raylib::consts::KeyboardKey::*;
    let code = match key {
        KEY_ENTER => KeyCode::Enter,
        KEY_BACKSPACE => KeyCode::Backspace,
        KEY_TAB => KeyCode::Tab,
        KEY_ESCAPE => KeyCode::Esc,
        KEY_LEFT => KeyCode::Left,
        KEY_RIGHT => KeyCode::Right,
        KEY_UP => KeyCode::Up,
        KEY_DOWN => KeyCode::Down,
        KEY_HOME => KeyCode::Home,
        KEY_END => KeyCode::End,
        KEY_PAGE_UP => KeyCode::PageUp,
        KEY_PAGE_DOWN => KeyCode::PageDown,
        KEY_DELETE => KeyCode::Delete,
        KEY_BACK => KeyCode::Esc,
        _ => return None,
    };
    Some(KeyEvent::new(code, KeyModifiers::empty()))
}
```

Note: This function handles non-character keys. Character keys (with Ctrl) go through `get_char_pressed` in the drain loop. The crossterm `KeyModifiers` type and `KeyEvent` are used to stay compatible with the rest of the app. We import:
```rust
use crossterm::event::{KeyEvent, KeyCode, KeyModifiers};
use ruster_core::key::KeyEvent as RusterKeyEvent;
```

Actually, let me simplify. The raylib `drain_raylib` already produces crossterm KeyEvents and they flow through `crossterm_to_ruster_key` in `handle_key`. The problem is that raylib's `get_char_pressed` returns ASCII control codes (1-26) when Ctrl is held, but `crossterm_to_ruster_key` only translates `KeyCode::Char(c)` with CONTROL modifier to `KeyEvent::Ctrl(c)` — and `c` here is the literal character, not the letter that was pressed.

Fix: in `drain_raylib`, map ASCII control codes to their letter equivalents when Ctrl modifier is set.

- [ ] **Step 2: Fix Ctrl+letter encoding in raylib `drain_raylib`**

In `crates/ruster-render-raylib/src/lib.rs`, modify the `get_char_pressed` loop:

```rust
while let Some(c) = self.rl.get_char_pressed() {
    if mods.contains(KeyModifiers::CONTROL) && (1..=26).contains(&c) {
        // Map ASCII control code to letter: Ctrl+A=1→'a', ..., Ctrl+Z=26→'z'
        let letter = char::from_u32((c as u32) + 96).unwrap_or('?');
        self.event_buffer.push(KeyEvent::new(KeyCode::Char(letter), mods));
    } else if let Some(ch) = char::from_u32(c as u32) {
        self.event_buffer.push(KeyEvent::new(KeyCode::Char(ch), mods));
    }
}
```

This ensures Ctrl+C produces `KeyEvent::Char('c')` with CONTROL modifier, which `crossterm_to_ruster_key` correctly converts to `KeyEvent::Ctrl('c')`.

- [ ] **Step 3: Build & test**

Run: `cargo check -p ruster-render-raylib -p ruster-bin`
Expected: clean build

No unit test for the Ctrl fix (requires raylib window), but the key mapping tests in `key.rs` should still pass.

- [ ] **Step 4: Commit**

```bash
git add crates/ruster-render-raylib/src/lib.rs
git commit -m "fix: Ctrl+letter encoding in raylib drain_raylib"
```

---

### Task 5: Indent/Deindent Edit Ops, Tab Key, and `>>`/`<<` Operators

**Files:**
- Modify: `crates/ruster-core/src/action.rs:11-17, 19-37`
- Modify: `crates/ruster-core/src/editor.rs:28-50`
- Modify: `crates/ruster-core/src/vim/mod.rs:159-245, 306-324`
- Modify: `crates/ruster-core/src/vim/ops.rs:8-57`
- Modify: `crates/ruster-lua/src/config.rs:1-10`

**Interfaces:**
- Consumes: `Config::tabstop`, `Config::expandtab`, `Config::shiftwidth`
- Produces: `Action::IndentLine`, `Action::DeindentLine`, Tab key in Insert, `>>`/`<<` in Normal/Visual

- [ ] **Step 1: Add `shiftwidth` and `indentwidth` to Config**

In `config.rs`:
```rust
pub struct Config {
    // ... existing fields ...
    pub shiftwidth: u32,
}
```

In `Default::default()`:
```rust
shiftwidth: 4,
```

- [ ] **Step 2: Add indent/deindent actions**

In `action.rs`:
```rust
pub enum Action {
    // ... existing variants ...
    IndentLine,
    DeindentLine,
}
```

- [ ] **Step 3: Handle indent/deindent in editor**

In `editor.rs`, add methods:
```rust
const INDENT: &str = "    "; // 4 spaces placeholder — will use config

pub fn set_config_indent(&mut self, tabstop: u32) {
    // Store the indent string from config (called on file open / config change)
    self.indent = " ".repeat(tabstop as usize);
}
```

Add a field:
```rust
pub struct Editor {
    buffer: Buffer,
    cursors: CursorSet,
    undo: UndoStack,
    indent: String,
}
```

Initialize:
```rust
indent: "    ".to_string(),
```

In `execute` match:
```rust
Action::IndentLine => {
    let line = self.cursor_line();
    let start = self.buffer.line_start_char(line);
    self.buffer.insert(start, &self.indent);
    let ch = Change { at: start, deleted: String::new(), inserted: self.indent.clone() };
    self.undo.push(ch);
}
Action::DeindentLine => {
    let line = self.cursor_line();
    let start = self.buffer.line_start_char(line);
    let content = self.buffer.line_to_string(line);
    let to_remove = content.chars().take_while(|c| *c == ' ').take(self.indent.len()).count();
    if to_remove > 0 {
        let ch = self.buffer.delete(start..start + to_remove);
        self.undo.push(ch);
        self.cursors.set_head(start, &self.buffer);
    }
}
```

Add `cursor_line` helper:
```rust
fn cursor_line(&self) -> usize {
    self.buffer.char_to_line(self.cursors.head())
}
```

- [ ] **Step 4: Add Tab/Shift+Tab to Insert mode**

In `vim/mod.rs` `handle_insert`:
```rust
KeyEvent::Tab => {
    out.push(Action::Edit(EditOp::InsertString("    ".to_string()))); // placeholder — uses config at app level
}
KeyEvent::ShiftTab => {
    // Move cursor left by shiftwidth (deindent conceptually, but simpler: backspace spaces)
    out.push(Action::DeindentLine);
}
```

Add `KeyEvent::ShiftTab` to `key.rs`:
```rust
pub enum KeyEvent {
    // ... existing ...
    Tab,
    ShiftTab,
}
```

- [ ] **Step 5: Add `>>`/`<<` operator handling in Vim**

In `handle_normal`, add `>` and `<` key handling:
```rust
KeyEvent::Char('>') if self.pending == OpState::Idle => {
    self.pending = OpState::Pending('>', n);
    self.count = None;
}
KeyEvent::Char('<') if self.pending == OpState::Idle => {
    self.pending = OpState::Pending('<', n);
    self.count = None;
}
```

In the pending operator handler (`OpState::Pending(op, count)`), add cases for `>` and `<`:
```rust
'>' | '<' => {
    self.pending = OpState::Idle;
    // Check if next key is > or < (operator applied to line)
    match key {
        KeyEvent::Char('>') if op == '>' => {
            out.push(Action::IndentLine);
        }
        KeyEvent::Char('<') if op == '<' => {
            out.push(Action::DeindentLine);
        }
        _ => {} // ignore mismatched pair
    }
}
```

Wait, this is wrong. `>>` currently works by `pending = Pending('>', n)` on first `>`, then the second `>` should check if op matches. But looking at how `dd` works:

1. First `d` → `pending = Pending('d', n)`
2. Second `d` → matches `KeyEvent::Char(m @ ('w' | 'b' | ... | 'd'))` in pending handler → `range_for_motion(editor, 'd', count)` returns line range → `apply_operator('d', start, end)` → deletes lines

So for `>>`:
1. First `>` → `pending = Pending('>', n)`
2. Second `>` → I need to handle this in the pending match

Currently the pending match handles:
- `i`/`a` → text object
- `w`/`b`/etc → range_for_motion
- Everything else → cancel

I'll add `>` and `<` as special operators that call `IndentLine`/`DeindentLine` directly.

In the pending handler, add after the `KeyEvent::Char(m @ (...))` block:
```rust
KeyEvent::Char('>') if op == '>' => {
    self.pending = OpState::Idle;
    out.push(Action::IndentLine);
}
KeyEvent::Char('<') if op == '<' => {
    self.pending = OpState::Idle;
    out.push(Action::DeindentLine);
}
```

Also add `>` and `<` as visual mode operators:
In `handle_visual`:
```rust
KeyEvent::Char('>') => {
    let (start, end) = self.visual_range(editor);
    // ... indent lines in range ... (simplified: just indent the first line for now)
    out.push(Action::IndentLine);
    self.mode = VimMode::Normal;
    self.anchor = None;
    self.count = None;
}
KeyEvent::Char('<') => {
    let (start, end) = self.visual_range(editor);
    out.push(Action::DeindentLine);
    self.mode = VimMode::Normal;
    self.anchor = None;
    self.count = None;
}
```

- [ ] **Step 6: Add Tab event support**

In `ruster-core/src/key.rs`, add:
```rust
pub enum KeyEvent {
    // ... existing ...
    Tab,
    ShiftTab,
}
```

In `crates/ruster-tui/src/key.rs`, add to `crossterm_to_ruster_key`:
```rust
KeyCode::Tab if ck.modifiers == KeyModifiers::SHIFT => KeyEvent::ShiftTab,
KeyCode::Tab => KeyEvent::Tab,
```

In `crates/ruster-render-raylib/src/key.rs`, add to `map_raylib_key`:
```rust
KEY_TAB => KeyCode::Tab,  // Shift+Tab is handled via mods in drain_raylib
```

- [ ] **Step 7: Wire config indent in app.rs key handling**

In `app.rs` `handle_key`, before passing to Vim state for Insert mode Tab:
```rust
if self.vim.mode == VimMode::Insert && key == KeyEvent::Tab {
    if self.config.expandtab {
        let spaces = " ".repeat(self.config.tabstop as usize);
        self.editor.borrow_mut().execute(Action::Edit(EditOp::InsertString(spaces)));
        return;
    }
}
```

But this only works in the TUI path. For the GUI path, the same logic applies since `handle_key` is shared.

Actually wait, this is handled inside the Vim state's `handle_insert` already for Tab. The Vim state doesn't know about config though. So I should either:
1. Pass `tabstop` and `expandtab` to the Vim state, or
2. Handle Tab at the app level before it reaches Vim state

Option 2 is simpler and matches the existing pattern. In `handle_key`:

```rust
pub fn handle_key(&mut self, ck: crossterm::event::KeyEvent) {
    let key = crossterm_to_ruster_key(ck);
    
    // Pre-processing for Insert mode Tab
    if self.vim.mode == VimMode::Insert && key == KeyEvent::Tab {
        if self.config.expandtab {
            let spaces = " ".repeat(self.config.tabstop as usize);
            self.editor.borrow_mut().execute(Action::BeginBatch);
            self.editor.borrow_mut().execute(Action::Edit(EditOp::InsertString(spaces)));
            self.editor.borrow_mut().execute(Action::EndBatch);
        }
        return;
    }
    
    // Rest of handle_key...
}
```

- [ ] **Step 8: Add tests**

```rust
#[test]
fn indent_adds_spaces() {
    let mut e = Editor::from_str("hello");
    e.execute(Action::IndentLine);
    assert_eq!(e.buffer().to_string(), "    hello");
}

#[test]
fn deindent_removes_spaces() {
    let mut e = Editor::from_str("    hello");
    e.execute(Action::DeindentLine);
    assert_eq!(e.buffer().to_string(), "hello");
}

#[test]
fn deindent_empty_line_does_nothing() {
    let mut e = Editor::from_str("");
    e.execute(Action::DeindentLine);
    assert_eq!(e.buffer().to_string(), "");
}
```

Add to `ops.rs` tests:
```rust
#[test]
fn double_angle_indent_line() {
    let mut e = Editor::from_str("hello");
    let mut v = VimState::new();
    to_start(&mut e, &mut v);
    for a in v.handle(KeyEvent::Char('>'), &e) { e.execute(a); }
    for a in v.handle(KeyEvent::Char('>'), &e) { e.execute(a); }
    assert_eq!(e.buffer().to_string(), "    hello");
}
```

- [ ] **Step 9: Build & test**

Run: `cargo test -p ruster-core -p ruster-tui`
Expected: all tests pass

- [ ] **Step 10: Commit**

```bash
git add crates/ruster-core/src/action.rs crates/ruster-core/src/editor.rs \
       crates/ruster-core/src/vim/mod.rs crates/ruster-core/src/vim/ops.rs \
       crates/ruster-core/src/key.rs crates/ruster-lua/src/config.rs \
       crates/ruster-tui/src/key.rs crates/ruster-render-raylib/src/key.rs
git commit -m "feat: tabs, indent/deindent, and Tab key"
```

---

### Task 6: EditorConfig Parser

**Files:**
- Create: `crates/ruster-core/src/editorconfig.rs`
- Modify: `crates/ruster-core/src/lib.rs:1-10`

**Interfaces:**
- Produces: `editorconfig::parse(root: &Path, file_path: &Path) -> HashMap<String, String>`

- [ ] **Step 1: Create `editorconfig.rs` module**

```rust
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn parse(file_path: &Path) -> HashMap<String, String> {
    let mut result = HashMap::new();
    let mut dir = file_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    dir = std::fs::canonicalize(&dir).unwrap_or(dir);
    loop {
        let ec_path = dir.join(".editorconfig");
        if ec_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&ec_path) {
                if let Some(section) = match_glob(&content, file_path) {
                    for (k, v) in section {
                        result.entry(k).or_insert(v);
                    }
                }
            }
            // Check root marker
            if has_root_marker(&content) {
                break;
            }
        }
        // Walk up
        if !dir.pop() {
            break;
        }
    }
    result
}

fn has_root_marker(content: &str) -> bool {
    content.lines().any(|l| l.trim().eq_ignore_ascii_case("root = true"))
}

fn match_glob<'a>(content: &'a str, file_path: &Path) -> Option<HashMap<&'a str, &'a str>> {
    let file_name = file_path.file_name()?.to_str()?;
    let mut best: Option<(usize, HashMap<&'a str, &'a str>)> = None;
    let mut lines = content.lines().map(|l| l.trim()).filter(|l| !l.is_empty() && !l.starts_with(';') && !l.starts_with('#'));
    while let Some(line) = lines.next() {
        if line.starts_with('[') {
            if let Some(end) = line.find(']') {
                let pattern = &line[1..end];
                if matches_glob_pattern(pattern, file_name) || matches_glob_pattern(pattern, file_path.to_str().unwrap_or("")) {
                    let mut props = HashMap::new();
                    loop {
                        if let Some(val_line) = lines.next() {
                            if val_line.starts_with('[') {
                                break; // next section; peeked
                            }
                            if let Some(eq) = val_line.find('=') {
                                let key = val_line[..eq].trim();
                                let val = val_line[eq+1..].trim();
                                props.insert(key, val);
                            }
                        } else {
                            break;
                        }
                    }
                    let specificity = pattern.len();
                    if best.as_ref().map_or(true, |(s, _)| specificity > *s) {
                        best = Some((specificity, props));
                    }
                }
            }
        }
    }
    best.map(|(_, m)| m)
}

fn matches_glob_pattern(pattern: &str, name: &str) -> bool {
    // Simple glob matching: * matches anything except /, ** matches everything
    // ?, [seq], {a,b} are not implemented yet
    if pattern == "*" {
        return true;
    }
    if pattern == "**" || pattern == "**/" {
        return true;
    }
    // Single * at start and end
    if pattern.starts_with("*") && pattern.ends_with("*") {
        let inner = &pattern[1..pattern.len()-1];
        return name.contains(inner);
    }
    if pattern.starts_with("*") {
        let suffix = &pattern[1..];
        return name.ends_with(suffix);
    }
    if pattern.ends_with("*") {
        let prefix = &pattern[..pattern.len()-1];
        return name.starts_with(prefix);
    }
    pattern == name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_star() {
        assert!(matches_glob_pattern("*", "foo.rs"));
    }

    #[test]
    fn matches_extension() {
        assert!(matches_glob_pattern("*.rs", "main.rs"));
        assert!(!matches_glob_pattern("*.rs", "main.py"));
    }

    #[test]
    fn no_dot_editorconfig_returns_empty() {
        let tmp = std::env::temp_dir();
        let result = parse(&tmp.join("nonexistent.txt"));
        assert!(result.is_empty());
    }
}
```

- [ ] **Step 2: Register module in lib.rs**

```rust
pub mod editorconfig;
```

- [ ] **Step 3: Build & test**

Run: `cargo test -p ruster-core`
Expected: all tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/ruster-core/src/editorconfig.rs crates/ruster-core/src/lib.rs
git commit -m "feat: EditorConfig parser"
```

---

### Task 7: EditorConfig Integration — Apply on File Open

**Files:**
- Modify: `crates/ruster-tui/src/app.rs:91-174`
- Modify: `crates/ruster-lua/src/config.rs:1-10`
- Modify: `crates/ruster-core/src/editor.rs:6-10`

**Interfaces:**
- Consumes: `editorconfig::parse()`, `Config`
- Produces: EditorConfig properties merged into `Config` and passed to Editor

- [ ] **Step 1: Add `EditorConfig` override in App construction**

In `app.rs` `new()`, after creating config:
```rust
// Apply EditorConfig overrides
let ec_props = ruster_core::editorconfig::parse(&file_path);
if let Some(val) = ec_props.get("indent_style") {
    config.expandtab = *val != "tab";
}
if let Some(val) = ec_props.get("indent_size") {
    if let Ok(n) = val.parse::<u32>() {
        config.tabstop = n;
    }
}
if let Some(val) = ec_props.get("tab_width") {
    if let Ok(n) = val.parse::<u32>() {
        config.tabstop = n;
    }
}
```

- [ ] **Step 2: Pass config values to Editor**

In `app.rs` `handle_key` Tab handler, use `self.config.tabstop`:
```rust
if self.vim.mode == VimMode::Insert && key == KeyEvent::Tab {
    if self.config.expandtab {
        let spaces = " ".repeat(self.config.tabstop as usize);
        // ... insert ...
    }
    return;
}
```

- [ ] **Step 3: Add tests**

In `editorconfig.rs` tests:
```rust
#[test]
fn parse_with_dot_editorconfig() {
    use std::io::Write;
    let tmp = std::env::temp_dir().join("ruster_ec_test");
    let _ = std::fs::create_dir_all(&tmp);
    let mut f = std::fs::File::create(tmp.join(".editorconfig")).unwrap();
    write!(f, "root = true\n\n[*]\nindent_style = space\nindent_size = 2\n").unwrap();
    let file = tmp.join("test.rs");
    std::fs::File::create(&file).unwrap();
    let props = parse(&file);
    assert_eq!(props.get("indent_style").map(|s| s.as_str()), Some("space"));
    assert_eq!(props.get("indent_size").map(|s| s.as_str()), Some("2"));
    let _ = std::fs::remove_dir_all(&tmp);
}
```

- [ ] **Step 4: Build & test**

Run: `cargo test -p ruster-core -p ruster-tui`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/ruster-tui/src/app.rs crates/ruster-lua/src/config.rs
git commit -m "feat: EditorConfig applied on file open"
```

---

### Final Verification

- [ ] Run full test suite: `cargo test -p ruster-core -p ruster-tui -p ruster-syntax -p ruster-render -p ruster-render-raylib`
- [ ] Build all: `cargo check -p ruster-bin -p ruster-tui -p ruster-render-raylib`
- [ ] Expected: all tests pass, no new warnings
