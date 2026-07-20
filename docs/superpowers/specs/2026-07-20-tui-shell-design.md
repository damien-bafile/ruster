# Ruster TUI Shell Design (Plan B)

> **Goal:** Deliver a terminal-based editor binary (`ruster`) that opens a file,
> renders the buffer using ratatui, routes keyboard input through the existing
> `ruster-core` Vim engine, and supports save/quit via `:` cmdline.
>
> **Depends on:** `ruster-core` (Plan A) — a headless Vim-compatible editing engine.
>
> **Exclusions:** GUI (`ruster-gui`, Plan C), Lua scripting (`ruster-lua`, Plan D),
> configuration file (`ruster.toml`), window splits, gutter line numbers,
> file explorer. These are all later plans.

---

## Architecture

Three new crates in the existing workspace:

```
crates/ruster-render/   — Renderer trait + EditorState snapshot + CursorKind
crates/ruster-tui/      — ratatui widgets, App struct, crossterm event loop
crates/ruster-bin/      — binary entry point, CLI arg parsing, error exit
```

`ruster-core` is **not modified** except for:
- Adding `Action::CmdlineResult(String)` variant (and the corresponding no-op match arm in `Editor::execute`)
- Extending `VimState` with `cmdline_buffer` and full Cmdline-mode key handling (existing `VimMode::Cmdline` match arm is expanded from "absorb Esc" to full text capture + Enter parsing)
- Adding a public `VimState::cmdline_buffer()` getter

All other additions are in the new crates.

---

## Crate: `ruster-render`

**Dependencies:** none beyond workspace members (this crate is pure types).

`src/lib.rs` exports:

```rust
pub enum CursorKind { Block, Bar }

pub struct EditorState<'a> {
    pub lines: Vec<String>,
    pub cursor: (u16, u16),          // (line, col), 0-indexed
    pub cursor_kind: CursorKind,
    pub mode_label: &'a str,
    pub file_path: &'a str,
    pub modified: bool,
    pub cmdline: Option<&'a str>,    // ":" prompt text when in cmdline mode
    pub message: Option<&'a str>,    // transient status message
}

pub trait Renderer {
    fn render_frame(&mut self, state: &EditorState);
}
```

The trait lets a future GUI crate implement the same interface. For Plan B only the ratatui backend exists.

---

## Crate: `ruster-tui`

**Dependencies:** `ruster-core`, `ruster-render`, `ratatui`, `crossterm`.

### App struct

```rust
pub struct App {
    pub editor: Editor,
    pub vim: VimState,
    pub renderer: TuiRenderer,
    pub file_path: PathBuf,
    pub should_quit: bool,
    pub message: Option<String>,  // cleared after next keypress
}
```

### Event loop

The `App::run()` method is the top-level loop:

```
enable raw mode, alternate screen
loop {
    build EditorState from current app state
    renderer.render_frame(&state)

    read crossterm event, convert to ruster_core::KeyEvent

    // Single path for all modes — VimState internally routes Cmdline/Insert/Visual/Normal
    for action in vim.handle(key, &editor) {
        match action {
            Action::CmdlineResult(cmd) => execute_cmd(&cmd),
            other => editor.execute(other),
        }
    }

    if should_quit { break }
}
disable raw mode, restore screen
```

The event loop simplifies to one path regardless of mode:

### Cmdline parsing in App (`execute_cmd`)

```rust
fn execute_cmd(&mut self, cmd: &str) {
    // cmd is like ":w", ":q", ":wq", ":q!", ":w /path"
    let trimmed = cmd.trim_start_matches(':').trim();
    match trimmed {
        "q" | "quit"         => self.should_quit = true,
        "q!"                 => self.should_quit = true,
        "w" | "write"        => self.save_current_file(),
        "wq"                 => { self.save_current_file(); self.should_quit = true; }
        "wq!"                => { self.save_current_file(); self.should_quit = true; }
        _ if trimmed.starts_with("w ") || trimmed.starts_with("write ") => {
            let path = trimmed.splitn(2, ' ').nth(1).unwrap_or("").trim();
            self.save_as(path);
        }
        _ => self.message = Some(format!("Unknown command: {}", cmd)),
    }
}
```

### Screen layout

Three vertical chunks via ratatui `Layout`:

1. **Buffer area** — `Layout::default().direction(Vertical).constraints([Fill(1), Length(1), Length(1)])`
2. **Statusline** — 1 line, always visible. Contains: mode label left, file path + modified marker center, "line,col" right.
3. **Cmdline** — 1 line, shown only when `vim.mode == Cmdline` or when `message` is set. Shows ":" prompt + cmdline text + cursor.

### Widgets

**BufferWidget** renders the text lines. The cursor is drawn by constructing the line string with a styled character at the cursor column position (ratatui `Span` with `bg: inverted`).

Simplified approach: a single ratatui `Paragraph` containing the buffer lines. The cursor column is communicated via `crossterm::queue!(cursor::MoveTo(col, line))` in the render method after drawing the paragraph. This avoids the complexity of per-character styling.

**StatuslineWidget** is a single `Paragraph` built with styled `Span`s for mode, file, and position:

```
-- NORMAL --  ruster/src/main.rs  +         12,34
```

Mode labels: `NORMAL`, `INSERT`, `VISUAL`, `V-LINE`, `CMDLINE`.

**CmdlineWidget** shows a `:` prompt followed by the cmdline buffer text. When not active, it's either empty or shows the last status message.

### TuiRenderer

```rust
pub struct TuiRenderer {
    // ratatui terminal handle
    terminal: Terminal<CrosstermBackend<Stdout>>,
}
```

Implements `Renderer::render_frame` by:
1. `terminal.draw(|frame| { /* lay out chunks, render widgets */ })`
2. `crossterm::queue!(stdout, cursor::MoveTo(col, line))` for cursor positioning
3. `crossterm::queue!(stdout, cursor::SetCursorShape(Block or Bar))` for cursor shape

### KeyEvent conversion

Convert `crossterm::event::KeyEvent` → `ruster_core::KeyEvent`:

```
crossterm KeyCode::Char('c') + Ctrl → ruster KeyEvent::Ctrl('c')
crossterm KeyCode::Char(c)          → ruster KeyEvent::Char(c)
crossterm KeyCode::Enter            → ruster KeyEvent::Enter
crossterm KeyCode::Backspace        → ruster KeyEvent::Backspace
crossterm KeyCode::Esc              → ruster KeyEvent::Esc
```

---

## Crate: `ruster-bin`

**Dependencies:** `ruster-core`, `ruster-render`, `ruster-tui` (or just `ruster-tui` which pulls in the rest).

`src/main.rs`:

```rust
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let path = match args.len() {
        2 => PathBuf::from(&args[1]),
        1 => { eprintln!("Usage: ruster <file>"); std::process::exit(1); }
        _ => { eprintln!("Usage: ruster <file>"); std::process::exit(1); }
    };

    let content = if path.exists() {
        std::fs::read_to_string(&path).map_err(|e| ...)?
    } else {
        String::new()
    };

    let mut app = App::new(content, path);
    app.run()?;
    Ok(())
}
```

After `run()` returns, write the buffer back if modified (or the user explicitly saved via `:w`).

---

## Changes to `ruster-core`

### `action.rs`

Add to `Action` enum:

```rust
/// Returned when the user presses Enter in Cmdline mode.
/// The string is the full cmdline text including the leading ":".
CmdlineResult(String),
```

### `editor.rs`

Add no-op arm in `execute()`:

```rust
Action::CmdlineResult(_) => {}
```

### `vim/mod.rs`

Add to `VimState`:

```rust
cmdline_buffer: String,  // current cmdline text
```

Expand the `VimMode::Cmdline` match arm in `handle()` (the `handle` method receives `out: &mut Vec<Action>`):

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

Add `:` handler in Normal mode:

```rust
KeyEvent::Char(':') => {
    self.mode = VimMode::Cmdline;
    self.cmdline_buffer = String::from(":");
    self.count = None;
}
```

Add public getter/taker:

```rust
pub fn cmdline_buffer(&self) -> &str { &self.cmdline_buffer }
```

---

## Error handling

- File-not-found on open: create new empty buffer (user gets a blank canvas; saving creates the file).
- File read error: print to stderr and exit with status 1.
- File write error: status message `"Error: <reason>"` in the cmdline area. User stays in the editor.
- crossterm init/deinit error: print to stderr and exit with status 1.

---

## Testing

- **Unit tests** for `VimState` cmdline capture: verify that `:` enters Cmdline mode, chars are captured, Enter emits `Action::CmdlineResult`, Esc clears it, and the mode returns to Normal.
- **Unit tests** for cmdline parsing: verify `:w`, `:q`, `:wq`, `:q!`, `:w /path` map to the correct actions.
- **Scenarios**: extend `scenario.rs` with key scripts that exercise the `:` → Enter → Esc flow.
- **Manual smoke test:** open a file, navigate with `hjkl`, edit text, `:w` save, `:q` quit.

---

## Out of scope (for this plan)

- Gutter line numbers (Plan C or later Phase 2 refinement)
- Window splits and tab management
- Modified-unsaved guard (`:q` without `!` warns)
- File explorer sidebar
- Configuration file or `:set` options
- Unicode box-drawing / treesitter syntax highlighting
- Emacs editing paradigm (exists in spec but defers to Plan D+)
- GUI rendering backend
