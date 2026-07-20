# Phase 0 — Async Event Loop & Animation System

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the blocking crossterm event loop with a tokio-based 60fps async loop and integrate tachyonfx for cursor blinking.

**Architecture:** `App::run_async()` creates a `new_current_thread` tokio runtime internally, spawns a blocking crossterm reader that feeds an `mpsc` channel, and runs a `tokio::select!` loop combining channel events with a 16ms interval tick. `AnimationState` uses `tachyonfx::EffectTimer` for cursor blink timing.

**Tech Stack:** tokio (rt + time), tachyonfx 0.2+, crossterm, ratatui

## Global Constraints

- Keep `App::run()` intact for test compatibility — tests parse cmdlines without needing a terminal
- `tokio` goes in `ruster-tui` only (not `ruster-core` or `ruster-render`)
- `tachyonfx` goes in `ruster-tui` only
- `crossterm_to_ruster_key` stays in `key.rs` — shared by both `run()` and `run_async()`
- All existing 84 tests must continue to pass

---

### Task 1: Dependencies + EditorState.cursor_visible

**Files:**
- Modify: `crates/ruster-tui/Cargo.toml`
- Modify: `crates/ruster-render/src/lib.rs`
- Test: existing tests still pass

**Interfaces:**
- Consumes: `EditorState` struct from `ruster-render`
- Produces: `EditorState` with new `cursor_visible: bool` field

- [ ] **Step 1: Add tokio and tachyonfx to ruster-tui Cargo.toml**

```toml
[dependencies]
ruster-core = { path = "../ruster-core" }
ruster-render = { path = "../ruster-render" }
ruster-syntax = { path = "../ruster-syntax" }
ratatui = "0.28"
crossterm = "0.28"
tokio = { version = "1", features = ["rt", "time"] }
tachyonfx = "0.25"
```

- [ ] **Step 2: Add `cursor_visible` to `EditorState`**

In `crates/ruster-render/src/lib.rs`, add the field after `cursor_kind`:

```rust
pub struct EditorState<'a> {
    pub lines: Vec<StyledLine>,
    pub cursor: (u16, u16),
    pub cursor_kind: CursorKind,
    pub cursor_visible: bool,
    pub mode_label: &'a str,
    pub file_path: &'a str,
    pub modified: bool,
    pub cmdline: Option<&'a str>,
    pub message: Option<&'a str>,
}
```

- [ ] **Step 3: Update the test in ruster-render**

In `crates/ruster-render/src/lib.rs`, the `renderer_trait_is_object_safe` test:

```rust
let state = EditorState {
    lines: vec![StyledLine { text: "hello".to_string(), highlights: vec![] }],
    cursor: (0, 0),
    cursor_kind: CursorKind::Block,
    cursor_visible: true,
    mode_label: "NORMAL",
    file_path: "test.txt",
    modified: false,
    cmdline: None,
    message: None,
};
```

- [ ] **Step 4: Build check**

```bash
cargo build -p ruster-render -p ruster-tui 2>&1
```
Expected: builds with no errors. Warning about `Highlighter.language` is pre-existing.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: add tokio/tachyonfx deps, EditorState.cursor_visible field"
```

---

### Task 2: AnimationState + BufferWidget cursor_visible

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` — add `AnimationState`
- Modify: `crates/ruster-tui/src/widgets.rs` — `BufferWidget` respects `cursor_visible`

**Interfaces:**
- Consumes: `EditorState.cursor_visible: bool`
- Produces: `AnimationState` struct, `BufferWidget::with_cursor_visible()`

- [ ] **Step 1: Add AnimationState to app.rs**

```rust
use tachyonfx::EffectTimer;
use tachyonfx::Interpolation;

#[derive(Clone)]
pub struct AnimationState {
    cursor_visible: bool,
    cursor_timer: EffectTimer,
}

impl AnimationState {
    pub fn new() -> Self {
        AnimationState {
            cursor_visible: true,
            cursor_timer: EffectTimer::from_ms(500, Interpolation::Linear),
        }
    }

    pub fn tick(&mut self, delta: std::time::Duration) {
        let remaining = self.cursor_timer
            .process(tachyonfx::Duration::from_secs_f64(delta.as_secs_f64()));
        if remaining.is_some() {
            self.cursor_timer = EffectTimer::from_ms(500, Interpolation::Linear);
            self.cursor_visible = !self.cursor_visible;
        }
    }
}
```

Note: `tachyonfx` re-exports a deprecated `Duration` type alongside `std::time::Duration`. Use the fully qualified `tachyonfx::Duration::from_secs_f64()` for the timer's `process()` call.

- [ ] **Step 2: Write test for AnimationState**

In `crates/ruster-tui/src/app.rs` tests module:

```rust
#[test]
fn animation_state_cursor_toggles() {
    use std::time::Duration;
    let mut anim = AnimationState::new();
    assert!(anim.cursor_visible);
    // Advance 600ms — should toggle to invisible
    for _ in 0..36 { anim.tick(Duration::from_secs_f64(1.0 / 60.0)); }
    assert!(!anim.cursor_visible);
    // Advance another 600ms — should toggle back
    for _ in 0..36 { anim.tick(Duration::from_secs_f64(1.0 / 60.0)); }
    assert!(anim.cursor_visible);
}
```

- [ ] **Step 3: Add `cursor_visible` to BufferWidget**

In `crates/ruster-tui/src/widgets.rs`:

```rust
pub struct BufferWidget {
    lines: Vec<StyledLine>,
    cursor: (u16, u16),
    syntax: bool,
    cursor_visible: bool,
}

impl BufferWidget {
    pub fn new(lines: Vec<StyledLine>, cursor: (u16, u16)) -> Self {
        BufferWidget { lines, cursor, syntax: false, cursor_visible: true }
    }

    pub fn with_syntax(mut self, yes: bool) -> Self {
        self.syntax = yes;
        self
    }

    pub fn with_cursor_visible(mut self, visible: bool) -> Self {
        self.cursor_visible = visible;
        self
    }
}
```

In the `render` method, wrap the cursor highlight:

```rust
if is_cursor_line && j as u16 == self.cursor.1 && self.cursor_visible {
    cell.set_bg(Color::White);
    cell.set_fg(Color::Black);
}
```

- [ ] **Step 4: Build and test**

```bash
cargo test -p ruster-tui animation_state_cursor_toggles -- --nocapture 2>&1
```
Expected: PASS

```bash
cargo test -p ruster-tui -p ruster-render 2>&1
```
Expected: all tests pass (15 in ruster-tui, 1 in ruster-render)

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: AnimationState with cursor blink, BufferWidget.cursor_visible"
```

---

### Task 3: Async event loop

**Files:**
- Modify: `crates/ruster-tui/src/app.rs`

- [ ] **Step 1: Extract `handle_key` method**

Current `run()` loop processes key events inline. Extract the body into a method:

```rust
impl App {
    pub fn handle_key(&mut self, ck: crossterm::event::KeyEvent) {
        let key = crossterm_to_ruster_key(ck);
        for action in self.vim.handle(key, &self.editor) {
            match action {
                Action::Textobject { op, kind, target, count: _ } => {
                    let cursor = self.editor.primary_head();
                    if let Some((start, end)) = self.syntax.as_ref()
                        .and_then(|s| s.ts_textobject(kind, target, cursor))
                    {
                        self.exec_operator(op, start, end);
                    }
                }
                Action::CmdlineResult(cmd) => {
                    self.message = None;
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
}
```

Then simplify `run()`:

```rust
pub fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen)?;
    self.renderer = TuiRenderer::new()?;

    loop {
        self.render();
        if self.should_quit { break; }
        let ev = crossterm::event::read()?;
        let ck = match ev {
            crossterm::event::Event::Key(k) => k,
            _ => continue,
        };
        self.handle_key(ck);
    }

    crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen)?;
    crossterm::terminal::disable_raw_mode()?;
    Ok(())
}
```

- [ ] **Step 2: Add `AppEvent` enum and `anim` field**

After the `CmdAction` enum:

```rust
enum AppEvent {
    Input(crossterm::event::Event),
}
```

Add `anim` to the `App` struct:

```rust
pub struct App {
    pub editor: Editor,
    pub vim: VimState,
    renderer: TuiRenderer,
    file_path: PathBuf,
    pub should_quit: bool,
    message: Option<String>,
    syntax: Option<SyntaxEngine>,
    anim: AnimationState,
}
```

Initialize in `App::new()`:

```rust
let anim = AnimationState::new();
App { editor, vim, renderer, file_path, should_quit: false, message: None, syntax, anim }
```

- [ ] **Step 3: Write `run_async` and `async_run`**

```rust
use std::time::{Duration, Instant};

impl App {
    pub fn run_async(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        crossterm::terminal::enable_raw_mode()?;
        crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen)?;
        self.renderer = TuiRenderer::new()?;

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()?;

        let result = rt.block_on(self.async_run());

        crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen)?;
        crossterm::terminal::disable_raw_mode()?;
        result
    }

    async fn async_run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        // Spawn blocking reader
        let tx_reader = tx.clone();
        tokio::task::spawn_blocking(move || {
            loop {
                match crossterm::event::read() {
                    Ok(ev) => {
                        if tx_reader.send(AppEvent::Input(ev)).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        let mut interval = tokio::time::interval(Duration::from_secs_f64(1.0 / 60.0));
        interval.tick().await; // discard first immediate tick

        let mut last_frame = Instant::now();

        loop {
            tokio::select! {
                event = rx.recv() => {
                    match event {
                        Some(AppEvent::Input(ev)) => {
                            match ev {
                                crossterm::event::Event::Key(k) => self.handle_key(k),
                                _ => {}
                            }
                        }
                        None => break,
                    }
                }
                _ = interval.tick() => {}
            }

            let now = Instant::now();
            let delta = now.duration_since(last_frame);
            last_frame = now;
            self.anim.tick(delta);
            self.render();
            if self.should_quit { break; }
        }

        Ok(())
    }

    fn tick_animations(&mut self) {
        // (deprecated — animation ticked inline in async_run now)
    }
}
```

- [ ] **Step 4: Update `render()` to pass `cursor_visible`**

In `App::render()`:

```rust
let state = EditorState {
    lines: styled_lines,
    cursor: (line, col),
    cursor_kind,
    cursor_visible: self.anim.cursor_visible,
    mode_label,
    file_path: &file_path,
    modified: false,
    cmdline: cmdline.as_deref(),
    message: None,
};
self.renderer.render_frame(&state);
```

- [ ] **Step 5: Wire cursor_visible through BufferWidget in renderer**

In `crates/ruster-tui/src/renderer.rs`:

```rust
let buf_widget = crate::widgets::BufferWidget::new(
    state.lines.clone(),
    state.cursor,
)
.with_syntax(has_highlights)
.with_cursor_visible(state.cursor_visible);
```

- [ ] **Step 6: Build and test**

```bash
cargo test -p ruster-tui -p ruster-render 2>&1
```
Expected: all tests pass

```bash
cargo build -p ruster-tui 2>&1
```
Expected: clean build

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "feat: async event loop with tokio, handle_key extraction"
```

---

### Task 4: Binary entry point

**Files:**
- Modify: `crates/ruster-bin/src/main.rs`

- [ ] **Step 1: Change `app.run()` to `app.run_async()`**

```rust
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
    if let Err(e) = app.run_async() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
```

- [ ] **Step 2: Full workspace build and test**

```bash
cargo build --workspace 2>&1
```
Expected: clean build (warning about `Highlighter.language` is pre-existing)

```bash
cargo test --workspace 2>&1 | tail -20
```
Expected: all 84+ tests pass

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "feat: switch binary to run_async event loop"
```
