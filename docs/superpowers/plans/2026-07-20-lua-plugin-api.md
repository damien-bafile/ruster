# Lua Plugin API Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Integrate a Lua scripting engine (`mlua`) providing the `ruster.*` plugin API for configuration, keymaps, commands, and event hooks.

**Architecture:** New `ruster-lua` crate owns the `mlua::Lua` state. `LuaRuntime` stores Lua keymap callbacks and queued actions (`Cmd`, `Print`) in a `RefCell<Vec<LuaAction>>`. `App` calls into `LuaRuntime` on each keystroke (check keymaps first) and drains queued actions between ticks.

**Tech Stack:** mlua 0.10 (Lua 5.4), ruster-core

## Global Constraints

- All 85 existing tests must pass unchanged
- `ruster.*` namespace, not `vim.*`
- Keymaps use angle-bracket notation: `<C-s>`, `<Esc>`, `<CR>`, etc.
- Plugin loading order: `init.lua`, then `plugins/*.lua` sorted

---

### Task 1: Crate setup, LuaRuntime, and basic API

**Files:**
- Create: `crates/ruster-lua/Cargo.toml`
- Create: `crates/ruster-lua/src/lib.rs`
- Create: `crates/ruster-lua/src/runtime.rs`
- Create: `crates/ruster-lua/src/api.rs`
- Create: `crates/ruster-lua/src/keymap.rs`
- Modify: `crates/ruster-tui/Cargo.toml`
- Modify: `crates/ruster-bin/Cargo.toml`
- Modify: `crates/ruster-tui/src/app.rs`
- Modify: `Cargo.toml` (workspace)

**Interfaces:**
- Consumes: `ruster-core` types (`Editor`, `VimState`, `Action`)
- Produces: `LuaRuntime` struct with `new()`, `load_init()`, `check_keymaps()`, `drain_actions()`
- Produces: `LuaAction` enum: `Cmd(String)`, `Print(String)`

- [ ] **Step 1: Create the crate skeleton**

Create `crates/ruster-lua/Cargo.toml`:
```toml
[package]
name = "ruster-lua"
version = "0.1.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[dependencies]
mlua = { version = "0.10", features = ["lua54"] }
ruster-core = { path = "../ruster-core" }
```

Add `"crates/ruster-lua"` to the workspace members in `Cargo.toml`.

Create `crates/ruster-lua/src/lib.rs`:
```rust
mod api;
pub mod keymap;
pub mod runtime;

pub use keymap::{parse_lua_key, LuaKey, LuaKeymap};
pub use runtime::{LuaAction, LuaRuntime};
```

- [ ] **Step 2: Implement keymap parsing**

Create `crates/ruster-lua/src/keymap.rs`:
```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LuaKey {
    Char(char),
    Ctrl(char),
    Esc,
    Enter,
    Tab,
    BackTab,
    Backspace,
    Delete,
    Home,
    End,
    Left,
    Right,
    Up,
    Down,
    F(u8),
}

#[derive(Debug, Clone)]
pub struct LuaKeymap {
    pub mode: String,
    pub keys: Vec<LuaKey>,
    pub callback: mlua::RegistryKey,
}

/// Parse an angle-bracket key string like "<C-s>" or "j".
/// Returns None for unrecognized sequences.
pub fn parse_lua_key(s: &str) -> Option<LuaKey> {
    if s.len() == 1 {
        return Some(LuaKey::Char(s.chars().next().unwrap()));
    }
    if !s.starts_with('<') || !s.ends_with('>') {
        return None;
    }
    let inner = &s[1..s.len()-1];
    match inner {
        "Esc" => Some(LuaKey::Esc),
        "CR" | "Enter" => Some(LuaKey::Enter),
        "Tab" => Some(LuaKey::Tab),
        "S-Tab" => Some(LuaKey::BackTab),
        "BS" | "Backspace" => Some(LuaKey::Backspace),
        "Del" | "Delete" => Some(LuaKey::Delete),
        "Home" => Some(LuaKey::Home),
        "End" => Some(LuaKey::End),
        "Left" => Some(LuaKey::Left),
        "Right" => Some(LuaKey::Right),
        "Up" => Some(LuaKey::Up),
        "Down" => Some(LuaKey::Down),
        _ if inner.len() == 3 && inner.starts_with('C') && inner.as_bytes()[1] == b'-' => {
            let c = inner.as_bytes()[2] as char;
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                Some(LuaKey::Ctrl(c))
            } else {
                None
            }
        }
        _ if inner.len() >= 2 && inner.starts_with('F') => {
            inner[1..].parse::<u8>().ok().filter(|&n| n >= 1 && n <= 12).map(LuaKey::F)
        }
        _ => None,
    }
}

/// Convert a LuaKey to a single crossterm event for matching.
/// Returns None for multi-key sequences (handled at the LuaKeymap level).
pub fn lua_key_to_crossterm(key: &LuaKey) -> crossterm::event::KeyEvent {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    match key {
        LuaKey::Char(c) => KeyEvent::new(KeyCode::Char(*c), KeyModifiers::NONE),
        LuaKey::Ctrl(c) => KeyEvent::new(KeyCode::Char(*c), KeyModifiers::CONTROL),
        LuaKey::Esc => KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        LuaKey::Enter => KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        LuaKey::Tab => KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
        LuaKey::BackTab => KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE),
        LuaKey::Backspace => KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        LuaKey::Delete => KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE),
        LuaKey::Home => KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
        LuaKey::End => KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
        LuaKey::Left => KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
        LuaKey::Right => KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
        LuaKey::Up => KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
        LuaKey::Down => KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        LuaKey::F(n) => KeyEvent::new(KeyCode::F(*n), KeyModifiers::NONE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_char() {
        assert_eq!(parse_lua_key("j"), Some(LuaKey::Char('j')));
        assert_eq!(parse_lua_key(":"), Some(LuaKey::Char(':')));
    }

    #[test]
    fn parse_ctrl_key() {
        assert_eq!(parse_lua_key("<C-s>"), Some(LuaKey::Ctrl('s')));
        assert_eq!(parse_lua_key("<C-a>"), Some(LuaKey::Ctrl('a')));
    }

    #[test]
    fn parse_special_keys() {
        assert_eq!(parse_lua_key("<Esc>"), Some(LuaKey::Esc));
        assert_eq!(parse_lua_key("<CR>"), Some(LuaKey::Enter));
        assert_eq!(parse_lua_key("<Tab>"), Some(LuaKey::Tab));
        assert_eq!(parse_lua_key("<S-Tab>"), Some(LuaKey::BackTab));
        assert_eq!(parse_lua_key("<BS>"), Some(LuaKey::Backspace));
        assert_eq!(parse_lua_key("<Del>"), Some(LuaKey::Delete));
    }

    #[test]
    fn parse_function_keys() {
        assert_eq!(parse_lua_key("<F1>"), Some(LuaKey::F(1)));
        assert_eq!(parse_lua_key("<F12>"), Some(LuaKey::F(12)));
    }

    #[test]
    fn parse_invalid_returns_none() {
        assert_eq!(parse_lua_key("<invalid>"), None);
        assert_eq!(parse_lua_key(""), None);
    }
}
```

- [ ] **Step 3: Run tests to verify keymap parsing works**

```bash
cargo test -p ruster-lua 2>&1
```
Expected: 5 tests pass.

- [ ] **Step 4: Implement LuaRuntime and basic API**

Create `crates/ruster-lua/src/runtime.rs`:
```rust
use std::cell::RefCell;
use std::path::Path;
use mlua::{Function, Lua, RegistryKey, Table, Value};
use crate::keymap::{parse_lua_key, LuaKey, LuaKeymap};

#[derive(Debug)]
pub enum LuaAction {
    Cmd(String),
    Print(String),
}

pub struct LuaRuntime {
    lua: Lua,
    pub(crate) keymaps: Vec<LuaKeymap>,
    pending: RefCell<Vec<LuaAction>>,
}

impl LuaRuntime {
    pub fn new() -> mlua::Result<Self> {
        let lua = Lua::new();
        let pending = RefCell::new(Vec::new());
        let mut runtime = LuaRuntime { lua, keymaps: Vec::new(), pending };

        let ruster = self::api::create_table(&runtime)?;
        runtime.lua.globals().set("ruster", ruster)?;
        Ok(runtime)
    }

    pub fn load_init(&mut self, path: &Path) -> Result<(), String> {
        let code = std::fs::read_to_string(path).map_err(|e| format!("Cannot read {}: {}", path.display(), e))?;
        self.lua.load(&code).exec().map_err(|e| format!("Lua error in {}: {}", path.display(), e))
    }

    pub fn drain_actions(&self) -> Vec<LuaAction> {
        self.pending.borrow_mut().drain(..).collect()
    }

    /// Check if a Lua keymap matches for the given mode and key.
    /// Returns true if matched (consumed the key).
    pub fn handle_key(&self, mode: &str, ck: &crossterm::event::KeyEvent) -> bool {
        for km in &self.keymaps {
            if km.mode != mode { continue; }
            if km.keys.len() != 1 { continue; } // multi-keys in future
            let expected = crate::keymap::lua_key_to_crossterm(&km.keys[0]);
            if expected == *ck {
                if let Ok(func) = self.lua.registry_value::<Function>(&km.callback) {
                    let _ = func.call::<(), ()>(());
                    return true;
                }
            }
        }
        false
    }
}

pub(crate) fn queue_action(runtime: &LuaRuntime, action: LuaAction) {
    runtime.pending.borrow_mut().push(action);
}
```

Create `crates/ruster-lua/src/api.rs`:
```rust
use mlua::{Function, Table, Value};
use crate::runtime::{self, LuaRuntime};
use crate::keymap::{parse_lua_key, LuaKeymap};

pub fn create_table(runtime: &LuaRuntime) -> mlua::Result<Table> {
    let t = runtime.lua.create_table()?;

    // ruster.print(...)
    let rt = runtime as *const LuaRuntime;
    let print_fn = runtime.lua.create_function(move |_, args: mlua::MultiValue| {
        let parts: Vec<String> = args.iter().map(|v| format_value(v)).collect();
        let msg = parts.join("\t");
        unsafe { (*rt).pending.borrow_mut().push(runtime::LuaAction::Print(msg)); }
        Ok(())
    })?;
    t.set("print", print_fn)?;

    // ruster.cmd(str)
    let rt = runtime as *const LuaRuntime;
    let cmd_fn = runtime.lua.create_function(move |_, cmd: String| {
        unsafe { (*rt).pending.borrow_mut().push(runtime::LuaAction::Cmd(cmd)); }
        Ok(())
    })?;
    t.set("cmd", cmd_fn)?;

    // ruster.keymap.set(mode, lhs, callback)
    let rt = runtime as *const LuaRuntime;
    let keymap_set = runtime.lua.create_function(move |_, (mode, lhs, func): (String, String, Function)| {
        let keys: Vec<_> = lhs.split_inclusive(|c: char| c == '>')
            .filter(|s| !s.is_empty())
            .filter_map(|s| {
                if s.ends_with('>') { parse_lua_key(s.trim()) }
                else { s.chars().map(|c| parse_lua_key(&c.to_string())).collect::<Option<Vec<_>>>()?.into_iter().next() }
            })
            .collect();
        if keys.is_empty() { return Err(mlua::Error::external("Cannot parse key sequence")); }
        let key = unsafe { (*rt).lua.registry_value::<mlua::RegistryKey>(&func).ok() };
        if key.is_none() {
            let reg = unsafe { (*rt).lua.create_registry_value(func) };
            match reg {
                Ok(r) => unsafe { (*rt).keymaps.push(LuaKeymap { mode, keys, callback: r }) },
                Err(e) => return Err(e),
            }
        }
        Ok(())
    })?;
    let keymap = runtime.lua.create_table()?;
    keymap.set("set", keymap_set)?;
    t.set("keymap", keymap)?;

    // ruster.g
    let g = runtime.lua.create_table()?;
    t.set("g", g)?;

    // ruster.mode - read-only, set by App
    t.set("mode", "normal")?;

    Ok(t)
}

fn format_value(v: &Value) -> String {
    match v {
        Value::Nil => "nil".into(),
        Value::String(s) => s.to_str().unwrap_or("?").to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Boolean(b) => b.to_string(),
        _ => format!("{:?}", v),
    }
}
```

- [ ] **Step 5: Wire LuaRuntime into App**

Add to `crates/ruster-tui/Cargo.toml`:
```toml
ruster-lua = { path = "../ruster-lua" }
```

Add to `crates/ruster-bin/Cargo.toml`:
```toml
ruster-lua = { path = "../ruster-lua" }
```

In `crates/ruster-tui/src/app.rs`:

Add `lua: LuaRuntime` field to `App` struct. Initialize in `App::new()`:
```rust
let lua = LuaRuntime::new().unwrap_or_else(|e| {
    eprintln!("Lua init failed: {}", e);
    // Create a minimal runtime that won't crash
    panic!("Lua init required");
});
App { editor, vim, renderer, file_path, should_quit: false, message: None, syntax, lua }
```

In `handle_key()`, before `self.vim.handle()`:
```rust
pub fn handle_key(&mut self, ck: crossterm::event::KeyEvent) {
    let mode = match self.vim.mode {
        VimMode::Normal => "n",
        VimMode::Insert => "i",
        VimMode::VisualChar | VimMode::VisualLine => "v",
        VimMode::Cmdline => "x",
    };
    if self.lua.handle_key(mode, &ck) {
        // Lua keymap consumed the key
        return;
    }
    // existing key handling...
}
```

In `async_run()`, before `self.render()`:
```rust
// Process queued Lua actions
for action in self.lua.drain_actions() {
    match action {
        LuaAction::Cmd(cmd) => {
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
        LuaAction::Print(msg) => {
            self.message = Some(msg);
        }
    }
}
```

- [ ] **Step 6: Load init.lua in App::new() if it exists**

In `App::new()`, after creating the `LuaRuntime`:
```rust
let config_path = dirs::config_dir()
    .unwrap_or_else(|| PathBuf::from("~/.config"))
    .join("ruster")
    .join("init.lua");
if config_path.exists() {
    if let Err(e) = lua.load_init(&config_path) {
        eprintln!("Lua config: {}", e);
    }
}
```

- [ ] **Step 7: Build and test**

```bash
cargo build --workspace 2>&1
```
Expected: clean build (pre-existing dead_code warning on `Highlighter.language`)

```bash
cargo test --workspace 2>&1 | grep -E "^(test result:)"
```
Expected: 85 + 5 = 90 tests pass (new ruster-lua tests)

- [ ] **Step 8: Commit**

```bash
git add -A && git commit -m "feat: Lua plugin API with keymaps, cmd, print (ruster.* namespace)"
```

---

### Task 2: Buffer/cursor API and events

**Files:**
- Modify: `crates/ruster-lua/src/api.rs`
- Create: `crates/ruster-lua/src/event.rs`
- Modify: `crates/ruster-lua/src/lib.rs`
- Modify: `crates/ruster-tui/src/app.rs`

**Interfaces:**
- Consumes: `LuaRuntime` from Task 1
- Produces: `ruster.api.*` functions, `ruster.on()` event system
- Produces: Updated `App` that dispatches events to `LuaRuntime`

- [ ] **Step 1: Add ruster.api.* functions**

In `crates/ruster-lua/src/api.rs`, after `ruster.mode` setup, add. Each closure receives `lua` (the `&Lua`) as its first argument, then the user-supplied args:

```rust
// ruster.api table
let api = runtime.lua.create_table()?;

// nvim_buf_get_lines(buf, start, end) — read buffer lines
let rt = runtime as *const LuaRuntime;
let get_lines = runtime.lua.create_function(move |lua, (_buf, start, end_opt): (i32, i32, Option<i32>)| {
    let mut cb = unsafe { (*rt).get_lines.borrow_mut() };
    let lines = match &mut *cb {
        Some(f) => f(start, end_opt),
        None => Vec::new(),
    };
    let t = lua.create_table()?;
    for (i, line) in lines.iter().enumerate() {
        t.set(i as i32 + 1, line.as_str())?;
    }
    Ok(mlua::Value::Table(t))
})?;
api.set("nvim_buf_get_lines", get_lines)?;

// nvim_buf_set_lines(buf, start, end, lines)
let rt = runtime as *const LuaRuntime;
let set_lines = runtime.lua.create_function(move |_, (_buf, start, end, lines): (i32, i32, i32, mlua::Value)| {
    let lines_vec: Vec<String> = match lines {
        mlua::Value::String(s) => vec![s.to_str().unwrap_or("").to_string()],
        mlua::Value::Table(t) => {
            let mut v = Vec::new();
            for i in 1..=t.len()? {
                if let Ok(s) = t.get::<String>(i) { v.push(s); }
            }
            v
        }
        _ => return Err(mlua::Error::external("set_lines expects string or table")),
    };
    let mut cb = unsafe { (*rt).set_lines.borrow_mut() };
    if let Some(f) = cb.as_mut() {
        f(start, end, lines_vec);
    }
    Ok(())
})?;
api.set("nvim_buf_set_lines", set_lines)?;

// nvim_win_get_cursor(win) — returns {row, col}
let rt = runtime as *const LuaRuntime;
let get_cursor = runtime.lua.create_function(move |lua, _win: i32| {
    let mut cb = unsafe { (*rt).get_cursor.borrow_mut() };
    match &mut *cb {
        Some(f) => {
            let (row, col) = f();
            let t = lua.create_table()?;
            t.set("row", row)?;
            t.set("col", col)?;
            Ok(mlua::Value::Table(t))
        }
        None => Ok(mlua::Value::Nil),
    }
})?;
api.set("nvim_win_get_cursor", get_cursor)?;

// nvim_win_set_cursor(win, {row, col})
let rt = runtime as *const LuaRuntime;
let set_cursor = runtime.lua.create_function(move |_, (_win, pos): (i32, Table)| {
    let row: i32 = pos.get("row").unwrap_or(0);
    let col: i32 = pos.get("col").unwrap_or(0);
    let mut cb = unsafe { (*rt).set_cursor.borrow_mut() };
    if let Some(f) = cb.as_mut() {
        f(row, col);
    }
    Ok(())
})?;
api.set("nvim_win_set_cursor", set_cursor)?;

t.set("api", api)?;
```

- [ ] **Step 2: Wire Editor callbacks into LuaRuntime**

In `crates/ruster-lua/src/runtime.rs`, add callback fields:

```rust
pub struct LuaRuntime {
    lua: Lua,
    pub(crate) keymaps: Vec<LuaKeymap>,
    pending: RefCell<Vec<LuaAction>>,
    // NEW:
    pub(crate) get_lines: RefCell<Option<Box<dyn FnMut(i32, Option<i32>) -> Vec<String>>>>,
    pub(crate) set_lines: RefCell<Option<Box<dyn FnMut(i32, i32, Vec<String>)>>>,
    pub(crate) get_cursor: RefCell<Option<Box<dyn FnMut() -> (i32, i32)>>>,
    pub(crate) set_cursor: RefCell<Option<Box<dyn FnMut(i32, i32)>>>,
}
```

Add setter methods:
```rust
pub fn set_buffer_callbacks(
    &mut self,
    get_lines: Box<dyn FnMut(i32, Option<i32>) -> Vec<String>>,
    set_lines: Box<dyn FnMut(i32, i32, Vec<String>)>,
    get_cursor: Box<dyn FnMut() -> (i32, i32)>,
    set_cursor: Box<dyn FnMut(i32, i32)>,
) {
    self.get_lines = RefCell::new(Some(get_lines));
    self.set_lines = RefCell::new(Some(set_lines));
    self.get_cursor = RefCell::new(Some(get_cursor));
    self.set_cursor = RefCell::new(Some(set_cursor));
}
```

Update the `nvim_buf_get_lines` and similar API closures in `api.rs` to call these callbacks.

- [ ] **Step 3: Implement event system**

Create `crates/ruster-lua/src/event.rs`:
```rust
use std::collections::HashMap;
use mlua::{Function, Lua, RegistryKey};

pub struct EventBus {
    handlers: HashMap<String, Vec<RegistryKey>>,
}

impl EventBus {
    pub fn new() -> Self {
        EventBus { handlers: HashMap::new() }
    }

    pub fn on(&mut self, lua: &Lua, event: &str, func: Function) -> mlua::Result<()> {
        let key = lua.create_registry_value(func)?;
        self.handlers.entry(event.to_string()).or_default().push(key);
        Ok(())
    }

    pub fn emit(&self, lua: &Lua, event: &str, args: &[mlua::Value]) {
        if let Some(handlers) = self.handlers.get(event) {
            for key in handlers {
                if let Ok(func) = lua.registry_value::<Function>(key) {
                    let _ = func.call::<_, ()>(args.to_vec());
                }
            }
        }
    }
}
```

Add `events: RefCell<EventBus>` to `LuaRuntime` struct.

Add `ruster.on()` to `api.rs`:
```rust
let rt = runtime as *const LuaRuntime;
let on_fn = runtime.lua.create_function(move |_, (event, func): (String, Function)| {
    unsafe {
        let mut events = (*rt).events.borrow_mut();
        events.on(&(*rt).lua, &event, func)
    }
})?;
t.set("on", on_fn)?;
```

- [ ] **Step 4: Wire events into App**

In `app.rs`, after `App::new()` initializes the LuaRuntime, emit `VimEnter`:
```rust
if let Some(lua) = self.lua.as_ref() {
    lua.events.borrow().emit(&lua.lua, "VimEnter", &[]);
}
```

In `handle_key()`, when mode changes, emit `ModeChanged`:
```rust
let old_mode = format!("{:?}", self.vim.mode);
// ... existing key handling that may change mode ...
let new_mode = format!("{:?}", self.vim.mode);
if old_mode != new_mode {
    self.lua.events.borrow().emit(&self.lua.lua, "ModeChanged", &[
        mlua::Value::String(self.lua.lua.create_string(&old_mode)?),
        mlua::Value::String(self.lua.lua.create_string(&new_mode)?),
    ]);
}
```

In `save_file()`, emit `BufWritePre` and `BufWritePost`.

In `render()` or after cursor movement, emit `CursorMoved` if cursor changed.

- [ ] **Step 5: Update LuaRuntime::new() to accept event bus initialization**

In `runtime.rs`, update `new()`:
```rust
pub fn new() -> mlua::Result<Self> {
    let lua = Lua::new();
    let pending = RefCell::new(Vec::new());
    let events = RefCell::new(EventBus::new());
    let mut runtime = LuaRuntime {
        lua,
        keymaps: Vec::new(),
        pending,
        events,
        get_lines: RefCell::new(None),
        set_lines: RefCell::new(None),
        get_cursor: RefCell::new(None),
        set_cursor: RefCell::new(None),
    };

    let ruster = self::api::create_table(&runtime)?;
    runtime.lua.globals().set("ruster", ruster)?;
    Ok(runtime)
}
```

- [ ] **Step 6: Build and test**

```bash
cargo build --workspace 2>&1
```
Expected: clean build

```bash
cargo test --workspace 2>&1 | grep -E "^(test result:)"
```
Expected: 90+ tests pass (existing 85 + 5 ruster-lua tests)

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "feat: Lua buffer/cursor API and event system (ruster.api.*, ruster.on)"
```
