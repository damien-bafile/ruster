# Lua Plugin API

**Date:** 2026-07-20
**Status:** Draft
**Phase:** Phase 0 (remainder)

## Goal

Integrate a Lua scripting engine (`mlua`) into ruster, providing a full plugin API under the `ruster.*` namespace for configuration, keymaps, commands, and event hooks.

## Architecture

New crate `crates/ruster-lua` with `mlua` (Lua 5.4). `ruster-tui` depends on it.

```
ruster-lua ──→ ruster-core
ruster-tui ──→ ruster-lua
```

`LuaRuntime` struct owns the mlua `Lua` instance, loaded keymaps, and event handlers. `App` creates it in `new()`, dispatches events to it, and checks Lua-registered keymaps before built-in key handling.

## API Surface

All functions are exposed as members of the `ruster` table:

| API | Signature | Purpose |
|-----|-----------|---------|
| `ruster.print(...)` | `any...` | Show message in status bar |
| `ruster.cmd(str)` | string | Execute an ex command (e.g. `":w"`, `":q!"`) |
| `ruster.keymap.set(mode, lhs, fn)` | string, string, function | Register a keymap. Mode: `"n"`, `"i"`, `"v"`, `"x"`. lhs: e.g. `"<C-s>"`, `"jj"` |
| `ruster.on(event, fn)` | string, function | Subscribe to an editor event |
| `ruster.api.nvim_buf_get_lines(buf, start, end)` | int, int, int | Get buffer lines between indices |
| `ruster.api.nvim_buf_set_lines(buf, start, end, lines)` | int, int, int, string or table | Replace lines in buffer |
| `ruster.api.nvim_win_get_cursor(win)` | int | Get cursor `{row, col}` (0-indexed) |
| `ruster.api.nvim_win_set_cursor(win, {row, col})` | int, table | Set cursor position |
| `ruster.g` | table | A global table for plugins to share state |
| `ruster.mode` | string | Read-only: current mode (`"normal"`, `"insert"`, `"visual"`, `"cmdline"`) |

### Keymap DSL

`ruster.keymap.set(mode, lhs, callback)` registers a key sequence to a Lua function. Mode strings:
- `"n"` — Normal mode
- `"i"` — Insert mode
- `"v"` — Visual mode (char)
- `"x"` — Visual mode (line)

lhs uses angle-bracket notation for special keys: `<CR>`, `<Esc>`, `<Tab>`, `<S-Tab>`, `<C-a>` through `<C-z>`, `<F1>` through `<F12>`, `<Left>`, `<Right>`, `<Up>`, `<Down>`, `<Home>`, `<End>`, `<BS>`, `<Del>`.

Examples:
```lua
ruster.keymap.set("n", "<C-s>", function()
  ruster.cmd(":w")
  ruster.print("Saved!")
end)

ruster.keymap.set("n", "jj", function()
  ruster.api.nvim_win_set_cursor(0, { 0, 0 })
end)
```

### Keymap Priority

On each keystroke in the event loop:
1. Check Lua-registered keymaps for the current mode
2. If a Lua keymap matches the sequence, call the callback and skip built-in handling
3. If no Lua keymap matches, fall through to `self.vim.handle()` as before

Longest prefix wins for multi-key sequences like `jj` — the Lua keymap buffer accumulates until it either matches, fails to match (fallback to built-in), or times out.

## Events

`ruster.on("EventName", callback)` — the callback receives event-specific arguments:

| Event | Arguments | When |
|-------|-----------|------|
| `VimEnter` | none | Startup complete, plugins loaded |
| `ModeChanged` | `old_mode, new_mode` (strings) | Editor mode changed |
| `BufWritePre` | none | Before saving a file |
| `BufWritePost` | none | After saving a file |
| `CursorMoved` | none | Cursor position changed |

## Plugin Loading

At startup, `App::new()` creates the `LuaRuntime` and loads scripts in order:

1. `~/.config/ruster/init.lua` (user config)
2. `~/.config/ruster/plugins/*.lua` (sorted by filename)

Each script is loaded into the same Lua state. Errors are caught (`pcall`) and reported as editor messages without crashing.

## Implementation Plan

The implementation is split into two tasks:

### Task 1: Crate setup, LuaRuntime, and basic API

- Create `crates/ruster-lua/` with `Cargo.toml` (depends on `mlua`, `ruster-core`)
- `LuaRuntime` struct: owns `mlua::Lua`, stores registered keymaps and event handlers
- Expose `ruster.print()`, `ruster.cmd()`, `ruster.keymap.set()`, `ruster.g`, `ruster.mode`
- Keymap matching: `check_keymaps(mode, key) -> Option<Action>` that the event loop queries
- Plugin loading from config directories
- Wire into `App`: create LuaRuntime, pass on keystrokes, check Lua keymaps first

### Task 2: Buffer/cursor API and events

- `ruster.api.*` buffer and cursor functions
- `ruster.on()` event subscription system
- `VimEnter`, `ModeChanged`, `BufWritePre`, `BufWritePost`, `CursorMoved` events
- Wire event dispatch from `App` to `LuaRuntime`

## File Structure

```
crates/ruster-lua/
├── Cargo.toml
└── src/
    ├── lib.rs         — crate exports (LuaRuntime, LuaKeymap, LuaEvent)
    ├── runtime.rs     — LuaRuntime struct: init, load, keymap check
    ├── api.rs         — ruster.* API functions
    ├── keymap.rs      — key sequence parsing and matching
    └── event.rs       — event subscription and dispatch
```

## Test Strategy

- Unit tests for keymap parsing and matching (tests parse `"<C-s>"` to crossterm key representations)
- Unit tests for API functions using a mock `Editor` and `LuaRuntime`
- Integration test: `ruster.cmd(":w")` triggers save action
- All existing 85 tests must pass unchanged
