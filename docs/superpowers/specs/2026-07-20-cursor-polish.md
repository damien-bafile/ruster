# Cursor Polish — Solid Cursor with Mode-Based Shape

**Date:** 2026-07-20
**Status:** Draft
**Phase:** Phase 0 (remainder)

## Goal

Replace the blinking cursor timer with an always-solid cursor that changes shape based on editor mode: block in Normal mode, vertical bar in Insert/Cmdline modes.

## Rationale

The blinking cursor adds visual noise and the `tachyonfx` dependency is heavy for a single timer. Removing the blink system simplifies the animation layer and prepares for the GUI backend migration.

## Changes

### 1. Remove AnimationSystem

- Delete `AnimationState` struct from `crates/ruster-tui/src/app.rs`
- Remove `anim` field from `App` struct
- Remove `self.anim.tick(delta)` from `async_run()`
- Remove `tachyonfx` import and dependency
- Remove the `animation_state_cursor_toggles` test

### 2. Cursor Always Visible

`EditorState.cursor_visible` is always `true`. In `App::render()`:

```rust
cursor_visible: true,
```

### 3. BufferWidget Respects cursor_kind

Add `cursor_kind` field to `BufferWidget` and its builder:

```rust
pub struct BufferWidget {
    lines: Vec<StyledLine>,
    cursor: (u16, u16),
    syntax: bool,
    cursor_visible: bool,
    cursor_kind: CursorKind,
}

impl BufferWidget {
    pub fn with_cursor_kind(mut self, kind: CursorKind) -> Self {
        self.cursor_kind = kind;
        self
    }
}
```

In the `render` method:
- `CursorKind::Block` → white background on cursor cell (current behavior)
- `CursorKind::Bar` → replace the cursor cell char with `▏` (U+258F, left one-eighth block), white foreground, default background

### 4. Wire cursor_kind Through Renderer

In `renderer.rs`, pass `state.cursor_kind` to the `BufferWidget`:

```rust
let buf_widget = crate::widgets::BufferWidget::new(...)
    .with_cursor_visible(state.cursor_visible)
    .with_cursor_kind(state.cursor_kind);
```

### 5. Cargo.toml Cleanup

Remove `tachyonfx` from `crates/ruster-tui/Cargo.toml`. The blink timer was the only use.

## Files Touched

| File | Change |
|------|--------|
| `crates/ruster-tui/Cargo.toml` | Remove `tachyonfx` |
| `crates/ruster-tui/src/app.rs` | Remove `AnimationState`, `anim`, `tick()`, tachyonfx imports. Always `cursor_visible: true`. |
| `crates/ruster-tui/src/widgets.rs` | Add `cursor_kind` field + builder + Bar rendering |
| `crates/ruster-tui/src/renderer.rs` | Wire `cursor_kind` to `BufferWidget` |

## Test Impact

- Remove `animation_state_cursor_toggles` test (tests deleted `AnimationState`)
- All remaining 85 existing tests must pass unchanged
- No new tests needed (purely visual change)

## Edge Cases

- **Wide chars at cursor with Bar:** The `▏` replaces the cell char. This is acceptable — terminal cursor always overwrites the cell visually. Block mode keeps the original char visible (white-on-white).
- **Cursor at line end:** Same behavior — the Bar appears at the final column position.
- **Cmdline mode:** Already gets `CursorKind::Bar` from the mode match in `render()`.
