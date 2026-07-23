# Lua Config System

## Purpose

Replace the planned TOML config file (`ruster.toml`) with a pure-Lua configuration
model. All settings are declared in `~/.config/ruster/init.lua` via the `ruster.config`
table. Rust reads the final values back after loading user scripts.

## Design

### 1. `ruster.config` table

Created in `LuaRuntime::new()` with default values. User overrides in `init.lua`.

```lua
-- Defaults (set by Rust before loading user config)
ruster.config = {
  tabstop = 4,
  softtabstop = 4,
  expandtab = true,
  number = false,
  relativenumber = false,
  theme = "default",
}
```

### 2. Rust `Config` struct

A plain struct that reflects the Lua table. After loading `init.lua`,
`LuaRuntime` reads `ruster.config` and populates the struct.

```rust
pub struct Config {
    pub tabstop: u32,
    pub softtabstop: u32,
    pub expandtab: bool,
    pub number: bool,
    pub relativenumber: bool,
    pub theme: String,
}
```

### 3. Integration

- `LuaRuntime::new()` creates the table, loads `init.lua`
- `App::new()` calls `lua.config()` → `Config` after loading
- `App` uses `Config` values for behavior (tab width, line numbers in render, etc.)
- Plugins can read/write `ruster.config` at runtime for live setting changes

### 4. No new crate, no new dependencies

The `Config` struct lives in `ruster-lua` (or `ruster-core` if other crates need it).
No `serde`, no `toml` crate.

## Deliverables

1. **Config struct + loading** — `ruster-lua/src/config.rs`, wiring in runtime.rs
2. **`docs/config-reference.md`** — documents every configurable setting
3. **`docs/lua-api.md`** — documents the full Lua API surface
4. **AGENTS.md note** — reminder to update docs as development progresses
