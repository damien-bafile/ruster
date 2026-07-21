# Phase 0 Completion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish Phase 0: tachyonfx animation system, cursor-on-empty-line fix, and raylib GUI backend.

**Architecture:** tachyonfx `Timer` replaces bare tokio interval for frame timing. Raylib gets a new `ruster-render-raylib` crate implementing the `Renderer` trait; `App` uses `Box<dyn Renderer>` to switch TUI/GUI modes.

**Tech Stack:** tachyonfx, raylib, crossterm, tokio

## Global Constraints

- tachyonfx dependency added to `ruster-tui` only
- No cursor blink animation (user explicitly declined)
- raylib handles all window/input/rendering — no winit
- `Renderer` trait stays minimal; `poll_input()` and `should_close()` get default no-op impls so TuiRenderer is unaffected
- raylib key events mapped to `crossterm::event::KeyEvent` so existing `App::handle_key()` works

---

### Task 1: tachyonfx Animation Timer

**Files:**
- Modify: `crates/ruster-tui/Cargo.toml:15` (add tachyonfx)
- Modify: `crates/ruster-tui/src/app.rs:30` (add Timer field)
- Modify: `crates/ruster-tui/src/app.rs:227-243` (wire tick call)

**Interfaces:**
- Consumes: `App` struct and `async_run` loop
- Produces: `self.timer: tachyonfx::Timer` field; every frame calls `self.timer.tick()` returning frame delta `Duration`

- [ ] **Step 1: Add tachyonfx dependency**

Edit `crates/ruster-tui/Cargo.toml`, add after line 13 (`ruster-lua`):

```toml
tachyonfx = "0.2"
```

- [ ] **Step 2: Add Timer field to App struct**

In `crates/ruster-tui/src/app.rs`, add field to `App`:

```rust
pub struct App {
    pub editor: Rc<RefCell<Editor>>,
    pub vim: VimState,
    renderer: TuiRenderer,
    file_path: PathBuf,
    pub should_quit: bool,
    message: Option<String>,
    syntax: Option<SyntaxEngine>,
    lua: LuaRuntime,
    config: Config,
    timer: tachyonfx::Timer,
}
```

Initialize in `App::new()` after `config` assignment, before return:

```rust
let timer = tachyonfx::Timer::from_duration(Duration::from_secs_f64(1.0 / 60.0));
```

- [ ] **Step 3: Wire timer tick in async_run**

In `async_run()` method (around line 227), after the `tokio::select!` block and before `self.render()`, add:

```rust
let _dt = self.timer.tick();
```

Replace the old interval setup. The `mut interval` and `interval.tick().await` stay — they provide the async wakeup. The `tachyonfx::Timer` is ticked synchronously each frame for downstream animation effects.

- [ ] **Step 4: Run tests**

```bash
cargo test -p ruster-tui
```
Expected: 14 passed, 0 failed.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: add tachyonfx animation timer to frame loop"
```

---

### Task 2: Raylib GUI Backend

**Files:**
- Create: `crates/ruster-render-raylib/Cargo.toml`
- Create: `crates/ruster-render-raylib/src/lib.rs`
- Create: `crates/ruster-render-raylib/src/key.rs`
- Modify: `Cargo.toml:2` (add workspace member)
- Modify: `crates/ruster-render/src/lib.rs:42-58` (add default methods to Renderer trait)
- Modify: `crates/ruster-bin/Cargo.toml` (add deps)
- Modify: `crates/ruster-bin/src/main.rs` (--gui flag, renderer selection)
- Modify: `crates/ruster-tui/src/lib.rs` (export App)
- Modify: `crates/ruster-tui/src/app.rs` (change renderer field to `Box<dyn Renderer>`, add run_gui)

**Interfaces:**
- Consumes: `Renderer` trait from `ruster-render`, `App::handle_key()`, `App::render()`
- Produces: `Box<dyn Renderer>` passed to `App::new()`, CLI `--gui` flag

- [ ] **Step 1: Add default methods to Renderer trait**

First add crossterm dep to `crates/ruster-render/Cargo.toml`:
```toml
crossterm = "0.28"
```

Then edit `crates/ruster-render/src/lib.rs`:

```rust
pub trait Renderer {
    fn render_frame(&mut self, state: &EditorState);
    fn poll_input(&mut self) -> Option<crossterm::event::KeyEvent> { None }
    fn should_close(&self) -> bool { false }
}
```

- [ ] **Step 2: Create ruster-render-raylib crate**

Create `crates/ruster-render-raylib/Cargo.toml`:

```toml
[package]
name = "ruster-render-raylib"
version = "0.1.0"
edition = "2021"

[dependencies]
ruster-render = { path = "../ruster-render" }
raylib = { version = "3.7", features = ["vendored"] }
crossterm = "0.28"
```

- [ ] **Step 3: Create key mapping module**

Create `crates/ruster-render-raylib/src/key.rs`:

```rust
use crossterm::event::{KeyEvent, KeyCode, KeyModifiers};

pub fn map_raylib_key(key: raylib::consts::KeyboardKey) -> Option<KeyEvent> {
    use raylib::consts::KeyboardKey::*;
    let code = match key {
        KEY_A => KeyCode::Char('a'),
        KEY_B => KeyCode::Char('b'),
        KEY_C => KeyCode::Char('c'),
        KEY_D => KeyCode::Char('d'),
        KEY_E => KeyCode::Char('e'),
        KEY_F => KeyCode::Char('f'),
        KEY_G => KeyCode::Char('g'),
        KEY_H => KeyCode::Char('h'),
        KEY_I => KeyCode::Char('i'),
        KEY_J => KeyCode::Char('j'),
        KEY_K => KeyCode::Char('k'),
        KEY_L => KeyCode::Char('l'),
        KEY_M => KeyCode::Char('m'),
        KEY_N => KeyCode::Char('n'),
        KEY_O => KeyCode::Char('o'),
        KEY_P => KeyCode::Char('p'),
        KEY_Q => KeyCode::Char('q'),
        KEY_R => KeyCode::Char('r'),
        KEY_S => KeyCode::Char('s'),
        KEY_T => KeyCode::Char('t'),
        KEY_U => KeyCode::Char('u'),
        KEY_V => KeyCode::Char('v'),
        KEY_W => KeyCode::Char('w'),
        KEY_X => KeyCode::Char('x'),
        KEY_Y => KeyCode::Char('y'),
        KEY_Z => KeyCode::Char('z'),
        KEY_ZERO => KeyCode::Char('0'),
        KEY_ONE => KeyCode::Char('1'),
        KEY_TWO => KeyCode::Char('2'),
        KEY_THREE => KeyCode::Char('3'),
        KEY_FOUR => KeyCode::Char('4'),
        KEY_FIVE => KeyCode::Char('5'),
        KEY_SIX => KeyCode::Char('6'),
        KEY_SEVEN => KeyCode::Char('7'),
        KEY_EIGHT => KeyCode::Char('8'),
        KEY_NINE => KeyCode::Char('9'),
        KEY_SPACE => KeyCode::Char(' '),
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
        KEY_COMMA => KeyCode::Char(','),
        KEY_PERIOD => KeyCode::Char('.'),
        KEY_SLASH => KeyCode::Char('/'),
        KEY_SEMICOLON => KeyCode::Char(';'),
        KEY_APOSTROPHE => KeyCode::Char('\''),
        KEY_LEFT_BRACKET => KeyCode::Char('['),
        KEY_RIGHT_BRACKET => KeyCode::Char(']'),
        KEY_GRAVE => KeyCode::Char('`'),
        KEY_MINUS => KeyCode::Char('-'),
        KEY_EQUAL => KeyCode::Char('='),
        KEY_BACKSLASH => KeyCode::Char('\\'),
        KEY_BACK => KeyCode::Esc,
        _ => return None,
    };
    Some(KeyEvent::new(code, KeyModifiers::empty()))
}
```

- [ ] **Step 4: Create RaylibRenderer**

Create `crates/ruster-render-raylib/src/lib.rs`:

```rust
mod key;

use crossterm::event::KeyEvent;
use raylib::prelude::*;
use ruster_render::{CursorKind, EditorState, Renderer};

const FONT_SIZE: i32 = 20;
const CHAR_W: i32 = 12;
const LINE_H: i32 = 24;
const PAD_X: i32 = 8;
const PAD_Y: i32 = 4;

pub struct RaylibRenderer {
    rl: RaylibHandle,
    thread: RaylibThread,
    font: Font,
}

impl RaylibRenderer {
    pub fn new(width: i32, height: i32, title: &str) -> Self {
        let (mut rl, thread) = raylib::init()
            .size(width, height)
            .title(title)
            .build();
        rl.set_target_fps(60);
        let font = rl.get_font_default();
        RaylibRenderer { rl, thread, font }
    }
}

impl Renderer for RaylibRenderer {
    fn render_frame(&mut self, state: &EditorState) {
        let mut d = self.rl.begin_drawing(&self.thread);
        d.clear_background(Color::new(30, 30, 30, 255));

        for (i, line) in state.lines.iter().enumerate() {
            let y = PAD_Y + i as i32 * LINE_H;
            d.draw_text_ex(
                &self.font,
                &line.text,
                Vector2::new(PAD_X as f32, y as f32),
                FONT_SIZE as f32,
                1.0,
                Color::new(205, 214, 244, 255),
            );
        }

        // Cursor
        if state.cursor_visible {
            let cx = PAD_X + state.cursor.1 as i32 * CHAR_W;
            let cy = PAD_Y + state.cursor.0 as i32 * LINE_H;
            match state.cursor_kind {
                CursorKind::Block => {
                    d.draw_rectangle(cx, cy, CHAR_W, LINE_H, Color::new(245, 224, 220, 200));
                }
                CursorKind::Bar => {
                    d.draw_rectangle(cx, cy, 2, LINE_H, Color::new(245, 224, 220, 255));
                }
            }
        }
    }

    fn poll_input(&mut self) -> Option<KeyEvent> {
        let k = self.rl.get_key_pressed()?;
        key::map_raylib_key(k)
    }

    fn should_close(&self) -> bool {
        self.rl.window_should_close()
    }
}
```

- [ ] **Step 5: Update TUI renderer — no changes needed**

The default trait methods for `poll_input()` and `should_close()` return `None` and `false`, so existing `TuiRenderer` is unaffected.

- [ ] **Step 6: Modify App to use Box<dyn Renderer>**

In `crates/ruster-tui/src/app.rs`:

Change field:
```rust
renderer: Box<dyn Renderer>,
```

Update `App::new()`:
```rust
pub fn new(content: String, file_path: PathBuf) -> Self {
    // ... existing setup ...
    let config = lua.config();
    App {
        editor, vim, renderer: Box::new(TuiRenderer::dummy()),
        file_path, should_quit: false, message: None,
        syntax, lua, config, timer,
    }
}
```

Update `run_async()`:
```rust
self.renderer = Box::new(TuiRenderer::new()?);
```

- [ ] **Step 7: Add run_gui to App**

Add to `impl App` block in `crates/ruster-tui/src/app.rs`:

```rust
pub fn run_gui(&mut self) {
    loop {
        while let Some(key) = self.renderer.poll_input() {
            self.handle_key(key);
        }
        self.render();
        if self.renderer.should_close() || self.should_quit { break; }
        std::thread::sleep(Duration::from_millis(16));
    }
}
```

- [ ] **Step 8: Add crate to workspace and binary deps**

Edit `Cargo.toml` workspace members:
```toml
members = ["crates/ruster-core", "crates/ruster-render", "crates/ruster-syntax", "crates/ruster-lua", "crates/ruster-tui", "crates/ruster-bin", "crates/ruster-render-raylib"]
```

Create or edit `crates/ruster-bin/Cargo.toml`:
```toml
[package]
name = "ruster-bin"
version = "0.1.0"
edition = "2021"

[dependencies]
ruster-tui = { path = "../ruster-tui" }
ruster-render-raylib = { path = "../ruster-render-raylib" }
```

- [ ] **Step 9: Update main.rs**

Replace `crates/ruster-bin/src/main.rs`:

```rust
use std::path::PathBuf;
use ruster_tui::app::App;
use ruster_render::Renderer;
use ruster_render_raylib::RaylibRenderer;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let gui = args.iter().any(|a| a == "--gui");
    let path_idx = args.iter().position(|a| !a.starts_with('-')).unwrap_or(1);
    let path = match args.get(path_idx) {
        Some(p) => PathBuf::from(p),
        None => {
            eprintln!("Usage: ruster [--gui] <file>");
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

    if gui {
        let renderer: Box<dyn Renderer> = Box::new(RaylibRenderer::new(800, 600, "ruster"));
        let mut app = App::new(content, path);
        app.renderer = renderer;
        app.run_gui();
    } else {
        let mut app = App::new(content, path);
        if let Err(e) = app.run_async() {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}
```

- [ ] **Step 10: Export App from ruster-tui**

Check `crates/ruster-tui/src/lib.rs` — ensure `pub mod app;` is present.

- [ ] **Step 11: Build and test**

```bash
cargo build 2>&1
```

Fix any compilation errors. Then run tests:
```bash
cargo test 2>&1
```
Expected: all tests pass.

- [ ] **Step 12: Commit**

```bash
git add -A && git commit -m "feat: add raylib GUI backend with --gui flag"
```

---

### Self-Review

**Spec coverage:**
- tachyonfx animation: ✅ Task 1
- Cursor on empty lines: ✅ already fixed before this plan
- Raylib GUI backend: ✅ Task 2, all spec items covered
- Box<dyn Renderer> for TUI/GUI switch: ✅ Step 6
- --gui flag: ✅ Step 9
- Key mapping: ✅ Step 3

**Placeholder scan:** All steps contain concrete code. No TBDs, TODOs, or fill-in-later patterns.

**Type consistency:** `Box<dyn Renderer>` used consistently in App, main.rs, RaylibRenderer. Trait default methods match. `RaylibRenderer::new()` signature consistent with construction site. `run_gui()` types match existing `handle_key()` and `render()` signatures.
