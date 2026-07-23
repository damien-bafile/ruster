# Lua Config System Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace TOML config with pure-Lua `ruster.config` table loaded from `init.lua`.

**Architecture:** `LuaRuntime::new()` creates a default `ruster.config` table. User `init.lua` overrides values. Rust reads back the final values into a `Config` struct via `LuaRuntime::config()`.

**Tech Stack:** Rust (mlua), Lua 5.4, existing `ruster-lua` crate.

## Global Constraints

- No new crate, no new dependencies (serde, toml crate prohibited)
- Config struct lives in `ruster-lua/src/config.rs`
- All settings have Rust-side defaults matching the Lua table defaults
- Docs and implementation in any order (they are independent)

---

### Task 1: Config struct + loading

**Files:**
- Create: `crates/ruster-lua/src/config.rs`
- Modify: `crates/ruster-lua/src/runtime.rs` (add `config()` method)
- Modify: `crates/ruster-lua/src/lib.rs` (add `mod config;`)

**Interfaces:**
- Consumes: `LuaRuntime` with its `lua: Lua` field and `ruster.config` table
- Produces: `Config { tabstop, softtabstop, expandtab, number, relativenumber, theme }`

- [ ] **Step 1: Create config.rs**

```rust
pub struct Config {
    pub tabstop: u32,
    pub softtabstop: u32,
    pub expandtab: bool,
    pub number: bool,
    pub relativenumber: bool,
    pub theme: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            tabstop: 4,
            softtabstop: 4,
            expandtab: true,
            number: false,
            relativenumber: false,
            theme: "default".into(),
        }
    }
}
```

- [ ] **Step 2: Add `config()` method to `LuaRuntime`**

In `crates/ruster-lua/src/runtime.rs`, add:

```rust
pub fn config(&self) -> Config {
    use mlua::Value;
    let defaults = Config::default();
    let ruster = match self.lua.globals().get::<mlua::Table>("ruster") {
        Ok(t) => t,
        Err(_) => return defaults,
    };
    let cfg = match ruster.get::<mlua::Table>("config") {
        Ok(t) => t,
        Err(_) => return defaults,
    };
    Config {
        tabstop: cfg.get("tabstop").unwrap_or(defaults.tabstop),
        softtabstop: cfg.get("softtabstop").unwrap_or(defaults.softtabstop),
        expandtab: cfg.get("expandtab").unwrap_or(defaults.expandtab),
        number: cfg.get("number").unwrap_or(defaults.number),
        relativenumber: cfg.get("relativenumber").unwrap_or(defaults.relativenumber),
        theme: cfg.get("theme").unwrap_or(defaults.theme),
    }
}
```

- [ ] **Step 3: Register module in lib.rs**

Add `pub mod config;` to `crates/ruster-lua/src/lib.rs`.

- [ ] **Step 4: Wire config loading in App::new()**

In `crates/ruster-tui/src/app.rs`, after `lua.fire_event("VimEnter", &[])`, verify config is accessible.
Read config via `lua.config()` and make it available (store as field or pass values through).

Add `config: Config` field to App struct, populate in constructor:

```rust
let config = lua.config();
```

- [ ] **Step 5: Build and test**

```bash
cargo build --workspace && cargo test --workspace
```

Expect: all 100 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/ruster-lua/src/config.rs crates/ruster-lua/src/runtime.rs crates/ruster-lua/src/lib.rs crates/ruster-tui/src/app.rs
git commit -m "feat: Lua-based config system (ruster.config table)"
```

---

### Task 2: Documentation files

**Files:**
- Create: `docs/config-reference.md`
- Create: `docs/lua-api.md`
- Modify: `AGENTS.md`

**Interfaces:**
- Consumes: Config struct + Lua API surface (existing code)
- Produces: Reference docs for developers and users

- [ ] **Step 1: Create config-reference.md**

```markdown
# Config Reference

All configuration is done in `~/.config/ruster/init.lua` via the `ruster.config` table.
Defaults are set by the editor; override only what you need.

```lua
ruster.config = {
  tabstop = 4,
  softtabstop = 4,
  expandtab = true,
  number = false,
  relativenumber = false,
  theme = "default",
}
```

## Settings

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `tabstop` | integer | 4 | Number of spaces a tab character represents |
| `softtabstop` | integer | 4 | Number of spaces inserted when pressing Tab |
| `expandtab` | boolean | true | Use spaces instead of tab characters |
| `number` | boolean | false | Show absolute line numbers in the gutter |
| `relativenumber` | boolean | false | Show relative line numbers (distance from cursor) |
| `theme` | string | "default" | Color theme name |
```

- [ ] **Step 2: Create lua-api.md**

```markdown
# Lua Scripting API

ruster provides a `ruster.*` namespace in Lua scripts loaded from
`~/.config/ruster/init.lua` and `~/.config/ruster/plugins/*.lua`.

## Namespace

### `ruster.print(...)`

Print one or more values to the message area (bottom of the editor).

```lua
ruster.print("hello", "world")  -- prints "hello\tworld"
```

### `ruster.cmd(command)`

Execute an editor command (cmdline-mode command).

```lua
ruster.cmd(":w")   -- save file
ruster.cmd(":q")   -- quit
```

### `ruster.keymap.set(mode, key_sequence, callback)`

Register a keymap that calls a Lua function when the key sequence is pressed.

Modes: `"n"` (Normal), `"i"` (Insert), `"v"` (Visual), `"x"` (Cmdline)

Key sequences use angle-bracket notation:
- `<C-s>` — Ctrl+S
- `<Esc>` — Escape
- `<CR>` — Enter
- `<Tab>` — Tab

```lua
ruster.keymap.set("n", "<C-s>", function()
  ruster.cmd(":w")
end)
```

### `ruster.on(event, callback)`

Register a callback for editor lifecycle events.

```lua
ruster.on("VimEnter", function()
  ruster.print("Editor ready!")
end)

ruster.on("BufWritePre", function()
  ruster.print("About to save...")
end)

ruster.on("BufWritePost", function(path)
  ruster.print("Saved: " .. path)
end)

ruster.on("ModeChanged", function(new_mode)
  ruster.print("Mode: " .. new_mode)
end)
```

### `ruster.mode`

Read-only string indicating the current editing mode (`"Normal"`, `"Insert"`,
`"VisualChar"`, `"VisualLine"`, `"Cmdline"`).

### `ruster.g`

A global variable table for sharing state between scripts and plugins.

```lua
ruster.g.my_plugin_state = { count = 0 }
```

### `ruster.api`

Editor API compatible with Neovim's `vim.api` naming conventions.

```lua
-- Get lines from the current buffer
local lines = ruster.api.nvim_buf_get_lines(0, 0, -1)

-- Replace lines in the buffer
ruster.api.nvim_buf_set_lines(0, 0, -1, {"new content"})

-- Get cursor position: { row, col }
local pos = ruster.api.nvim_win_get_cursor(0)

-- Set cursor position
ruster.api.nvim_win_set_cursor(0, { row = 5, col = 10 })
```

### `ruster.config`

Configuration table. See [Config Reference](config-reference.md).

```lua
ruster.config.tabstop = 2
ruster.config.number = true
```
```

- [ ] **Step 3: Update AGENTS.md**

Append a section at the end of AGENTS.md:

```markdown
## Documentation Maintenance

**All documentation in `docs/` must be kept in sync with the codebase.**
When you implement a new feature, change an existing setting, or modify the Lua API:
1. Update `docs/config-reference.md` if settings change
2. Update `docs/lua-api.md` if the Lua surface changes
3. If you created a new doc, add a reference in the relevant phase section above

**This includes:**
- Adding new `ruster.config` settings
- Adding new `ruster.api.*` functions
- Adding new events for `ruster.on()`
- Changing default values or behavior
```

- [ ] **Step 4: Commit**

```bash
git add docs/config-reference.md docs/lua-api.md AGENTS.md
git commit -m "docs: config reference, Lua API reference, AGENTS note"
```
