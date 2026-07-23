# Animation System — Frame Delta API + Smooth Cursor

**Date:** 2026-07-21
**Status:** Draft
**Phase:** Phase 0 (completion)

## Goal

Two deliverables to close out Phase 0:

1. Expose the per-frame delta time to Lua so plugins can drive smooth animations (minibuffers, transitions, etc.)
2. Implement Neovide-style smooth cursor animation in the GUI renderer

---

## 1. Lua Frame Delta API

### API Surface

```lua
local dt = ruster.api.get_frame_delta()  -- float seconds since last frame
```

```lua
ruster.on("Frame", function(dt)
    -- dt is float seconds
end)
```

### Implementation

**`crates/ruster-lua/src/runtime.rs`:**
- Add `current_dt: RefCell<f64>` field to `LuaRuntime`
- Add `LuaRuntime::set_frame_dt(dt: f64)` method:
  - Stores the value in `current_dt`
  - Calls `self.fire_event("Frame", &[Value::Number(dt)])`

**`crates/ruster-lua/src/api.rs`:**
- In `create_table()`, add `get_frame_delta` closure under the `api` table:
  ```rust
  let rt = runtime as *const LuaRuntime;
  let get_frame_delta = runtime.lua.create_function(move |_, ()| {
      unsafe { Ok((*rt).current_dt.borrow().clone()) }
  })?;
  api.set("get_frame_delta", get_frame_delta)?;
  ```

**`crates/ruster-tui/src/app.rs`:**
- `run_gui()`: add `self.timer.tick()` at top of frame loop (currently missing)
- `async_run()`: change `let _dt = self.timer.tick()` to `let dt = self.timer.tick()`
- In both loops, after `timer.tick()` and before `self.render()`:
  ```rust
  let secs = dt.as_secs_f64();
  self.lua.set_frame_dt(secs);
  ```
- `run()` (sync TUI fallback): add `self.timer.tick()` + `set_frame_dt()` at top of loop

---

## 2. Smooth Cursor Animation (GUI Only)

### New Types

**In `crates/ruster-tui/src/app.rs`:**
```rust
struct CursorAnim {
    cell_x: f32,   // fractional cell column (e.g. 5.3 = between col 5 and 6)
    cell_y: f32,   // fractional cell row
}
```

**In `crates/ruster-render/src/lib.rs`:**
Add to `EditorState`:
```rust
pub cursor_smooth: Option<(f32, f32)>,
// None = snap to cell (TUI)
// Some((dx, dy)) = pixel offset from cell origin (GUI)
```

### Animation Logic

CursorAnim works in **cell-relative coordinates** (fractional cells), so App never needs to know pixel sizes. Each frame, after `timer.tick()` and before `render()`:

```rust
fn update_cursor_anim(&mut self, dt: Duration) {
    let dt = dt.as_secs_f32();
    let col = self.editor.borrow().primary_head_col();
    let line = self.editor.borrow().primary_head_line();
    let target_x = col as f32;
    let target_y = line as f32;

    let dx = target_x - self.cursor_anim.cell_x;
    let dy = target_y - self.cursor_anim.cell_y;
    let dist = (dx * dx + dy * dy).sqrt();

    // Scale speed by distance so big jumps animate faster
    let speed = self.config.cursor_anim_speed / (1.0 + dist * 0.1);

    // Exponential ease-out
    self.cursor_anim.cell_x += dx * (1.0 - (-speed * dt).exp());
    self.cursor_anim.cell_y += dy * (1.0 - (-speed * dt).exp());
}
```

`cursor_smooth` pixel offset computed from cell delta:
```rust
let dx_cells = self.cursor_anim.cell_x - col as f32;
let dy_cells = self.cursor_anim.cell_y - line as f32;
cursor_smooth = Some((dx_cells, dy_cells));
```

When disabled (`cursor_anim_enabled = false`), snap instantly:
```rust
self.cursor_anim.cell_x = target_x;
self.cursor_anim.cell_y = target_y;
```

### Render Changes

**`App::render()`:**
Build `cursor_smooth` from `CursorAnim` delta:
```rust
let (line, col) = get_cursor_line_col(); // from editor state
let cursor_smooth = if self.has_smooth_cursor {
    Some((
        self.cursor_anim.cell_x - col as f32,
        self.cursor_anim.cell_y - line as f32,
    ))
} else {
    None
};
```

Since the renderer is a trait object, the cleanest approach is a fixed constant or check — `cursor_smooth` is `Some(…)` when the renderer is raylib (determined by a flag on App), `None` for TUI.

Add a `has_smooth_cursor: bool` field to `App`:
- Set to `true` in `main.rs` when creating a `RaylibRenderer`
- Defaults to `false` (TuiRenderer)

### EditorState changes

```rust
pub cursor_smooth: Option<(f32, f32)>,
// The renderer interprets as:
// draw_cursor_x = PAD_X + cursor.1 * CHAR_W + cursor_smooth.unwrap_or((0,0)).0
// draw_cursor_y = PAD_Y + cursor.0 * LINE_H  + cursor_smooth.unwrap_or((0,0)).1
```

### Raylib Renderer Changes

**`crates/ruster-render-raylib/src/lib.rs`:**
Offset cursor rectangle by `cursor_smooth` (fractional cells → pixels):
```rust
let col = state.cursor.1 as i32;
let line = state.cursor.0 as i32;
let cx = PAD_X + col * CHAR_W;
let cy = PAD_Y + line * LINE_H;
if let Some((dcx, dcy)) = state.cursor_smooth {
    cx = (cx as f32 + dcx * CHAR_W as f32) as i32;
    cy = (cy as f32 + dcy * LINE_H as f32) as i32;
}
```

### Lua Config

Add to `Config`:
```rust
pub cursor_anim_enabled: bool,   // default: true
pub cursor_anim_speed: f32,      // default: 12.0
```

```lua
ruster.config = {
    cursor_anim_enabled = true,
    cursor_anim_speed = 12.0,
}
```

---

## Files Touched

| File | Change |
|------|--------|
| `crates/ruster-lua/src/runtime.rs` | Add `current_dt` field, `set_frame_dt()` method |
| `crates/ruster-lua/src/api.rs` | Add `get_frame_delta` closure to `api` table |
| `crates/ruster-lua/src/config.rs` | Add `cursor_anim_enabled`, `cursor_anim_speed` |
| `crates/ruster-render/src/lib.rs` | Add `cursor_smooth` to `EditorState`; update test |
| `crates/ruster-render-raylib/src/lib.rs` | Offset cursor by `cursor_smooth` |
| `crates/ruster-tui/src/app.rs` | Add `CursorAnim`, `has_smooth_cursor`, animation logic, wire dt to Lua in all loops |
| `crates/ruster-bin/src/main.rs` | Set `has_smooth_cursor = true` when using RaylibRenderer |

---

## Test Impact

- Existing 100 tests must pass unchanged
- `EditorState` test needs `cursor_smooth: None` added
- New test for `CursorAnim` basic easing behavior
- New test for `get_frame_delta` returning correct value

---

## Edge Cases

- **First frame**: `CursorAnim` initialized to `cell_x: 0.0, cell_y: 0.0`, snaps to first cursor position immediately (dist is large, speed scales up, animation is sub-frame)
- **Huge jumps** (`G` to EOF): distance scaling makes animation fast enough to not feel sluggish (~0.3s for full-screen jump at speed 12)
- **Disabled animation**: snaps instantly, no visual difference from current behavior
- **TUI mode**: `cursor_smooth` is always `None`, no visual change

---

## Success Criteria

1. `ruster.api.get_frame_delta()` returns plausible values (~0.016 in GUI, ~0.016 in TUI)
2. `ruster.on("Frame", ...)` fires each frame with correct dt
3. Cursor slides smoothly between positions in GUI mode
4. TUI mode unaffected (cursor still snaps)
5. Setting `cursor_anim_enabled = false` disables smooth cursor
6. All tests pass
