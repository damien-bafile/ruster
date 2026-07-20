# Cursor Polish Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the blinking cursor system and make the cursor always-solid with mode-based shape (block Normal, bar Insert/Cmdline).

**Architecture:** `AnimationState` (with `tachyonfx` timer) is deleted. `BufferWidget` gets a `cursor_kind` field, rendering a `▏` character for Bar mode and the existing white block for Block mode.

**Tech Stack:** ratatui, crossterm

## Global Constraints

- All 85 existing tests must pass after changes
- `animation_state_cursor_toggles` test is removed (tested deleted code)

---

### Task 1: Remove blink system, wire cursor_kind

**Files:**
- Modify: `crates/ruster-tui/Cargo.toml`
- Modify: `crates/ruster-tui/src/app.rs`
- Modify: `crates/ruster-tui/src/widgets.rs`
- Modify: `crates/ruster-tui/src/renderer.rs`

**Interfaces:**
- Consumes: `EditorState.cursor_kind: CursorKind`, `EditorState.cursor_visible: bool` from `ruster-render`
- Produces: `BufferWidget` with `with_cursor_kind()` builder rendering Block/Bar shapes

- [ ] **Step 1: Remove tachyonfx from Cargo.toml**

In `crates/ruster-tui/Cargo.toml`, remove the `tachyonfx = "0.25"` line. Keep all other deps.

- [ ] **Step 2: Remove AnimationState and tachyonfx imports from app.rs**

In `crates/ruster-tui/src/app.rs`:

Delete the `AnimationState` struct and its impl block (lines 28-50).

Delete these lines:
```rust
use tachyonfx::EffectTimer;
use tachyonfx::Interpolation;
```

Remove `anim: AnimationState` from the `App` struct. Remove `let anim = AnimationState::new();` and the `anim` entry from the struct literal in `App::new()`. `App::new()` now looks like:
```rust
App { editor, vim, renderer, file_path, should_quit: false, message: None, syntax }
```

In `async_run()`, remove:
```rust
self.anim.tick(delta);
```

In `App::render()`, change:
```rust
cursor_visible: self.anim.cursor_visible,
```
to:
```rust
cursor_visible: true,
```

- [ ] **Step 3: Remove AnimationState test**

In `crates/ruster-tui/src/app.rs`, delete the `animation_state_cursor_toggles` test (lines 315-326).

- [ ] **Step 4: Add cursor_kind to BufferWidget**

In `crates/ruster-tui/src/widgets.rs`:

Add the import:
```rust
use ruster_render::CursorKind;
```

Add `cursor_kind` field to `BufferWidget`:
```rust
pub struct BufferWidget {
    lines: Vec<StyledLine>,
    cursor: (u16, u16),
    syntax: bool,
    cursor_visible: bool,
    cursor_kind: CursorKind,
}
```

Initialize in `new()`:
```rust
pub fn new(lines: Vec<StyledLine>, cursor: (u16, u16)) -> Self {
    BufferWidget { lines, cursor, syntax: false, cursor_visible: true, cursor_kind: CursorKind::Block }
}
```

Add builder:
```rust
pub fn with_cursor_kind(mut self, kind: CursorKind) -> Self {
    self.cursor_kind = kind;
    self
}
```

In the `render` method, replace the cursor block (line 81-83):
```rust
if is_cursor_line && j as u16 == self.cursor.1 && self.cursor_visible {
    match self.cursor_kind {
        CursorKind::Bar => {
            cell.set_char('\u{258f}'); // ▏
            cell.set_fg(Color::White);
            cell.set_bg(Color::Reset);
        }
        CursorKind::Block => {
            cell.set_bg(Color::White);
            cell.set_fg(Color::Black);
        }
    }
}
```

- [ ] **Step 5: Wire cursor_kind through renderer**

In `crates/ruster-tui/src/renderer.rs`, chain the new builder:
```rust
let buf_widget = crate::widgets::BufferWidget::new(
    state.lines.clone(),
    state.cursor,
)
.with_syntax(has_highlights)
.with_cursor_visible(state.cursor_visible)
.with_cursor_kind(state.cursor_kind);
```

- [ ] **Step 6: Build and test**

```bash
cargo test --workspace 2>&1
```

Expected: 85 tests passed (the removed animation_state_cursor_toggles drops the test count from 86). All existing tests pass.

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "feat: always-solid cursor with mode-based shape (Block/Bar)"
```
