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

### `ruster.statusline.section(pos, fn)`

Register a statusline component. `pos` is `"left"`, `"center"`, or `"right"`.
`fn` is called each frame and must return a string; empty strings are skipped.
Sections are shown on the active window's statusline, appended to the built-in
components (mode, file name, percentage, and `line,col`).

```lua
-- Show the current git branch on the right side of the statusline.
ruster.statusline.section("right", function()
  return "⎇ main"
end)
```

### `ruster.config`

Configuration table. See [Config Reference](config-reference.md).

```lua
ruster.config.tabstop = 2
ruster.config.number = true
```
