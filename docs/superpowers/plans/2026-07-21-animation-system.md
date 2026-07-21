# Animation System Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose per-frame delta to Lua and add Neovide-style smooth cursor animation in GUI mode.

**Architecture:** Frame delta is stored on `LuaRuntime` and set each frame before render. `CursorAnim` in `App` tracks fractional-cell cursor position and eases toward the real cursor each frame. `EditorState.cursor_smooth` carries the cell offset to the renderer.

**Tech Stack:** rust, mlua 0.10, raylib 6.0, ratatui 0.28

## Global Constraints

- All existing 100 tests must pass unchanged
- TUI mode (ratatui) must be unaffected — cursor_smooth always None
- Cursor animation only in GUI mode — controlled by has_smooth_cursor bool on App
- Config fields read from `ruster.config` Lua table with sensible defaults

---

### Task 1: Lua Frame Delta API

**Files:**
- Modify: `crates/ruster-lua/src/config.rs`
- Modify: `crates/ruster-lua/src/runtime.rs`
- Modify: `crates/ruster-lua/src/api.rs`
- Test: Add inline test in `crates/ruster-lua/src/api.rs`

**Interfaces:**
- Produces: `LuaRuntime::set_frame_dt(dt: f64)` — stores dt and fires `"Frame"` event
- Produces: `ruster.api.get_frame_delta()` → Lua number
- Produces: `ruster.on("Frame", function(dt) end)` — fires with dt
- Produces: `Config.cursor_anim_enabled: bool`, `Config.cursor_anim_speed: f32`

- [ ] **Step 1: Add config fields**

In `crates/ruster-lua/src/config.rs`, add to `Config`:
```rust
pub cursor_anim_enabled: bool,
pub cursor_anim_speed: f32,
```

Add defaults in `Default::default()`:
```rust
cursor_anim_enabled: true,
cursor_anim_speed: 12.0,
```

- [ ] **Step 2: Add current_dt + set_frame_dt to LuaRuntime**

In `crates/ruster-lua/src/runtime.rs`, add field:
```rust
pub current_dt: RefCell<f64>,
```

Initialize in `new()`:
```rust
current_dt: RefCell::new(0.0),
```

Add method:
```rust
pub fn set_frame_dt(&self, dt: f64) {
    *self.current_dt.borrow_mut() = dt;
    let val = mlua::Value::Number(dt);
    self.fire_event("Frame", &[val]);
}
```

- [ ] **Step 3: Add get_frame_delta to API table**

In `crates/ruster-lua/src/api.rs`, inside the `api` table creation block (after `nvim_win_set_cursor`), add:
```rust
// ruster.api.get_frame_delta()
let rt = runtime as *const LuaRuntime;
let get_frame_delta = runtime.lua.create_function(move |_, ()| {
    unsafe {
        let dt = (*rt).current_dt.borrow();
        Ok(*dt)
    }
})?;
api.set("get_frame_delta", get_frame_delta)?;
```

- [ ] **Step 4: Add config read for new fields**

In `crates/ruster-lua/src/runtime.rs`, in the `config()` method, add after existing fields:
```rust
cursor_anim_enabled: cfg.get("cursor_anim_enabled").unwrap_or(defaults.cursor_anim_enabled),
cursor_anim_speed: cfg.get("cursor_anim_speed").unwrap_or(defaults.cursor_anim_speed),
```

- [ ] **Step 5: Run existing tests**

```bash
cargo test -p ruster-lua
```
Expected: all pass

- [ ] **Step 6: Commit**

```bash
git add crates/ruster-lua/src/config.rs crates/ruster-lua/src/runtime.rs crates/ruster-lua/src/api.rs
git commit -m "feat: add frame delta API to Lua (get_frame_delta, Frame event, cursor anim config)"
```

---

### Task 2: EditorState + CursorAnim + Frame Loop Wiring

**Files:**
- Modify: `crates/ruster-render/src/lib.rs` — add `cursor_smooth` field
- Modify: `crates/ruster-tui/src/app.rs` — CursorAnim, frame loop wiring, has_smooth_cursor

**Interfaces:**
- Consumes: `LuaRuntime::set_frame_dt(f64)`
- Consumes: `Config::cursor_anim_enabled`, `Config::cursor_anim_speed`
- Produces: `EditorState.cursor_smooth: Option<(f32, f32)>`
- Produces: `App.cursor_anim: CursorAnim`
- Produces: `App.has_smooth_cursor: bool`

- [ ] **Step 1: Add cursor_smooth to EditorState**

In `crates/ruster-render/src/lib.rs`, add to `EditorState` struct after `cursor_visible`:
```rust
pub cursor_smooth: Option<(f32, f32)>,
```

Update the test in the same file to include `cursor_smooth: None` in the EditorState constructor.

- [ ] **Step 2: Add CursorAnim struct and update method to app.rs**

In `crates/ruster-tui/src/app.rs`, add after the `FrameTimer` struct:
```rust
struct CursorAnim {
    cell_x: f32,
    cell_y: f32,
}

impl CursorAnim {
    fn new() -> Self {
        Self { cell_x: 0.0, cell_y: 0.0 }
    }

    fn update(&mut self, dt: Duration, target_col: u16, target_line: u16, enabled: bool, speed: f32) {
        let dt = dt.as_secs_f32();
        let tx = target_col as f32;
        let ty = target_line as f32;

        if !enabled {
            self.cell_x = tx;
            self.cell_y = ty;
            return;
        }

        let dx = tx - self.cell_x;
        let dy = ty - self.cell_y;
        let dist = (dx * dx + dy * dy).sqrt();
        let s = speed / (1.0 + dist * 0.1);
        self.cell_x += dx * (1.0 - (-s * dt).exp());
        self.cell_y += dy * (1.0 - (-s * dt).exp());
    }
}
```

- [ ] **Step 3: Add fields to App struct**

Add to `App` struct:
```rust
timer: FrameTimer,
has_smooth_cursor: bool,
cursor_anim: CursorAnim,
```

(Note: `timer` already exists — confirm it's there. Add the other two.)

Initialize in `App::new()` after `let timer = FrameTimer::new();`:
```rust
let cursor_anim = CursorAnim::new();
```

Add to the App construction:
```rust
has_smooth_cursor: false,
cursor_anim,
```

- [ ] **Step 4: Update render() to pass cursor_smooth**

In `App::render()`, after computing `line` and `col` from `head`, add:
```rust
let cursor_smooth = if self.has_smooth_cursor {
    Some((self.cursor_anim.cell_x - col as f32, self.cursor_anim.cell_y - line as f32))
} else {
    None
};
```

Add to `EditorState`:
```rust
cursor_smooth,
```

- [ ] **Step 5: Add cursor_line_col helper to App**

In `crates/ruster-tui/src/app.rs`, add method:
```rust
fn cursor_line_col(&self) -> (u16, u16) {
    let editor = self.editor.borrow();
    let head = editor.primary_head();
    let buf = editor.buffer();
    let line = buf.char_to_line(head);
    let col = head - buf.line_start_char(line);
    (line as u16, col as u16)
}
```

- [ ] **Step 6: Make has_smooth_cursor pub**

In the `App` struct, change:
```rust
has_smooth_cursor: bool,
```
to:
```rust
pub has_smooth_cursor: bool,
```

- [ ] **Step 7: Wire dt + cursor anim in run_gui()**

In `App::run_gui()`, add at top of the loop, before input polling:
```rust
let dt = self.timer.tick();
```

After input processing and before `self.render()`, add:
```rust
let secs = dt.as_secs_f64();
self.lua.set_frame_dt(secs);

let (line, col) = self.cursor_line_col();
self.cursor_anim.update(dt, col, line, self.config.cursor_anim_enabled, self.config.cursor_anim_speed);
```

- [ ] **Step 8: Wire dt + cursor anim in async_run()**

In `App::async_run()`, find `let _dt = self.timer.tick();` and change to `let dt = self.timer.tick();`. After the tick and before `self.render()`, add:
```rust
let secs = dt.as_secs_f64();
self.lua.set_frame_dt(secs);

let (line, col) = self.cursor_line_col();
self.cursor_anim.update(dt, col, line, self.config.cursor_anim_enabled, self.config.cursor_anim_speed);
```

- [ ] **Step 9: Wire dt in sync run()**

In `App::run()`, add at top of the loop, before `self.render()`:
```rust
let dt = self.timer.tick();
let secs = dt.as_secs_f64();
self.lua.set_frame_dt(secs);
```

(No cursor anim in sync TUI mode, but dt should still flow to Lua.)

- [ ] **Step 10: Run tests**

```bash
cargo test
```
Expected: all 100+ tests pass. (EditorState test now includes `cursor_smooth: None`.)

- [ ] **Step 11: Commit**

```bash
git add crates/ruster-render/src/lib.rs crates/ruster-tui/src/app.rs
git commit -m "feat: add cursor_smooth to EditorState, CursorAnim, wire frame delta to Lua"
```

---

### Task 3: Raylib Renderer + main.rs Flag

**Files:**
- Modify: `crates/ruster-render-raylib/src/lib.rs` — use `cursor_smooth`
- Modify: `crates/ruster-bin/src/main.rs` — set `has_smooth_cursor = true`

- [ ] **Step 1: Offset cursor by cursor_smooth in raylib renderer**

In `crates/ruster-render-raylib/src/lib.rs`, in `render_frame()`, replace the cursor drawing section:

```rust
if state.cursor_visible {
    let col = state.cursor.1 as i32;
    let line = state.cursor.0 as i32;
    let mut cx = PAD_X + col * CHAR_W;
    let mut cy = PAD_Y + line * LINE_H;
    if let Some((dcx, dcy)) = state.cursor_smooth {
        cx = (cx as f32 + dcx * CHAR_W as f32) as i32;
        cy = (cy as f32 + dcy * LINE_H as f32) as i32;
    }
    match state.cursor_kind {
        CursorKind::Block => {
            d.draw_rectangle(cx, cy, CHAR_W, LINE_H, Color::new(245, 224, 220, 200));
        }
        CursorKind::Bar => {
            d.draw_rectangle(cx, cy, 2, LINE_H, Color::new(245, 224, 220, 255));
        }
    }
}
```

- [ ] **Step 2: Set has_smooth_cursor in main.rs**

In `crates/ruster-bin/src/main.rs`, after `app.renderer = renderer;`, add:
```rust
app.has_smooth_cursor = true;
```

- [ ] **Step 3: Check compile**

```bash
cargo check
```
Expected: no errors

- [ ] **Step 4: Run tests**

```bash
cargo test
```
Expected: all pass

- [ ] **Step 5: Commit**

```bash
git add crates/ruster-render-raylib/src/lib.rs crates/ruster-bin/src/main.rs
git commit -m "feat: smooth cursor animation in raylib renderer"
```
