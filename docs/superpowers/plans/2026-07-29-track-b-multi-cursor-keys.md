# Multi-Cursor Keybindings — Implementation Plan

> **Status:** delivered; boxes resolved 2026-08-07.
>
> This plan was executed but never ticked as it went, leaving 10 boxes
> that read as outstanding work. They are **plain bullets now, not back-ticked**:
> a box ticked long after the fact asserts a verification that did not happen,
> and this tree has already been bitten by exactly that. The bullets stand as a
> record of what was built.
>
> Evidence it shipped: the three identifiers this plan names (`App`, `Buffer`,
> `Window`) are too generic to prove anything, so the behaviour was driven
> instead — `drive.rs::ctrl_d_adds_a_second_cursor_on_the_next_match` places a
> second caret through the real frame loop, and
> `docs/verification/multicursor-{tui.txt,gui.png}` capture it in both backends.


> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Wire Ctrl+D to add cursors at next word occurrences and Ctrl+click/Alt+click for mouse-based cursor addition.

**Architecture:** Add Ctrl+D dispatch to Normal mode key handling in `App::handle_key()`. Add a word-find helper. Enable mouse capture in crossterm and wire Alt+click to add a cursor. The multi-cursor engine and rendering already exist in ruster-core.

**Tech Stack:** Rust, ruster-tui (app.rs), ruster-core (cursor.rs, buffer.rs, editor.rs)

## Global Constraints

- Ctrl+D in Normal mode = add cursor at next word occurrence
- Alt+click = add cursor at clicked position (TUI only, VTE-encoded mouse events)
- Follow existing dispatch patterns

---

### Task 1: Add Ctrl+D key dispatch for multi-cursor

**Files:**
- Modify: `crates/ruster-tui/src/app.rs`
- Modify: `crates/ruster-tui/src/key.rs` (crossterm translation if needed)

**Interfaces:**
- Consumes: `Action::AddCursor(usize)`, `Action::ClearExtraCursors`
- Produces: `self.buffer_word_at(pos) -> Option<String>` (or use a buffer helper)

- **Step 1: Add a word-at-position helper to App (or use existing)**

Check if there is already a buffer helper to get the word/identifier under a cursor. If not, add:

```rust
/// Get the word (identifier) surrounding the given byte offset in the active buffer.
fn word_at(&self, pos: usize) -> Option<String> {
    use std::ops::Range;
    let buf = self.active_buffer()?;
    let text = buf.slice(0..buf.len_bytes()).to_string();
    if pos >= text.len() { return None; }
    let is_word_char = |c: char| c.is_alphanumeric() || c == '_';
    let bytes = text.as_bytes();
    // walk left
    let start = (0..pos).rev()
        .find(|&i| !is_word_char(bytes[i] as char))
        .map(|i| i + 1)
        .unwrap_or(0);
    // walk right
    let end = (pos..text.len())
        .find(|&i| !is_word_char(bytes[i] as char))
        .unwrap_or(text.len());
    if start >= end { return None; }
    Some(text[start..end].to_string())
}
```

Place it as a private method on `App`, near the multi-cursor section.

- **Step 2: Add Ctrl+D dispatch in handle_key**

In `handle_key()`, in the Normal-mode dispatch area (before the Vim state machine section), add:

```rust
KeyCode::Char('d') if ck.modifiers.contains(KeyModifiers::CONTROL) => {
    // Add cursor at next word occurrence.
    let pos = self.vim.cursor_head(); // or self.active_window().cursors.primary().head
    let word = self.word_at(pos);
    if let Some(w) = word {
        let buf = self.active_buffer().unwrap();
        let total = buf.len_bytes();
        // Search forward from pos+1, wrap around.
        let text = buf.slice(0..total).to_string();
        let search_from = (pos + 1) % total;
        let doubled = format!("{}{}", text, text);
        if let Some(found) = doubled[search_from..].find(&w) {
            let offset = (search_from + found) % total;
            self.vim.add_cursor(offset);
            return true;
        }
    }
    false // fall through
}
```

Note: `self.vim.add_cursor()` may not exist — check the actual API. Use `self.active_window_mut().cursors.add_cursor(offset)` or `Action::AddCursor(offset)` queued into the action buffer. Follow the existing action-dispatch pattern.

- **Step 3: Build to verify**

```
cargo build -p ruster-tui 2>&1 | tail -5
```

- **Step 4: Commit**

```
git add crates/ruster-tui/src/app.rs
git commit -m "feat(multicursor): add Ctrl+D to add cursor at next word"
```

---

### Task 2: Enable mouse capture and wire Alt+click

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` (event loop, key handling)

**Interfaces:**
- Consumes: `crossterm::event::Event::Mouse`, `Action::AddCursor`
- Modifies: event loop to enable mouse mode

- **Step 1: Enable mouse capture on startup and cleanup**

In the `run()` method, before the event loop, enable mouse capture:

```rust
use crossterm::event::{EnableMouseCapture, DisableMouseCapture};
execute!(std::io::stdout(), EnableMouseCapture)?;
```

In the cleanup/exit code (where the terminal is reset), add:

```rust
execute!(std::io::stdout(), DisableMouseCapture)?;
```

- **Step 2: Handle mouse events in the event loop**

In the event loop where `Event::Key` is matched, replace the `_ => continue` with:

```rust
crossterm::event::Event::Mouse(me) => {
    self.handle_mouse_event(me);
    continue;
}
_ => continue,
```

Then add the handler method:

```rust
fn handle_mouse_event(&mut self, me: crossterm::event::MouseEvent) {
    // Alt+click (or Ctrl+click) adds a cursor at the clicked position.
    // crossterm reports Alt via modifiers.
    if me.kind == MouseEventKind::Down(MouseButton::Left)
        && me.modifiers.contains(KeyModifiers::ALT)
    {
        // Convert (row, col) to buffer offset and add cursor.
        let row = me.row as usize;
        let col = me.column as usize;
        // Find the window at this screen position...
        if let Some((win_id, buf_off)) = self.screen_pos_to_buffer_offset(row, col) {
            self.windows.window_mut(win_id).cursors.add_cursor(buf_off);
        }
    }
}
```

- **Step 3: Add screen_pos_to_buffer_offset helper**

This converts a terminal (row, col) to a (window_id, buffer_offset). It maps the screen position to a window rect, then uses the window's scroll offset and line-wrapping to find the buffer position.

```rust
fn screen_pos_to_buffer_offset(&self, row: usize, col: usize) -> Option<(usize, usize)> {
    for (id, win) in self.windows.iter() {
        let r = win.rect;
        if row >= r.y as usize && row < (r.y + r.h) as usize
            && col >= r.x as usize && col < (r.x + r.w) as usize
        {
            let win_row = row - r.y as usize;
            let buf_line = win_row + win.scroll as usize;
            let buf = self.buffers.get(win.buffer)?;
            let line_start = buf.line_to_offset(buf_line)?;
            let col_clamped = (col - r.x as usize).min(buf.line_length(buf_line).unwrap_or(0).saturating_sub(1));
            let offset = line_start + col_clamped;
            return Some((id, offset));
        }
    }
    None
}
```

Note: use the actual `Window` struct fields and `Buffer` methods. Adjust types to match (u16 vs usize, etc.).

- **Step 4: Build to verify**

```
cargo build -p ruster-tui 2>&1 | tail -5
```

- **Step 5: Run existing tests**

```
cargo test -p ruster-tui 2>&1 | tail -5
```

- **Step 6: Commit**

```
git add crates/ruster-tui/src/app.rs
git commit -m "feat(multicursor): enable mouse capture and Alt+click cursor add"
```
