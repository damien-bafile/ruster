# Lua Scripting API

ruster provides a `ruster.*` namespace in Lua scripts loaded from
`~/.config/ruster/init.lua` and `~/.config/ruster/plugins/*.lua`.

> See also: [Config Reference](config-reference.md) for settings and
> [Commands & Keybindings](keybindings.md) for keys and `:` commands.

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

### Mouse events

`ruster.on` also takes the mouse events: `"mouse_down"`, `"mouse_up"`,
`"mouse_drag"`, `"mouse_move"`, `"mouse_wheel"`, and `"hover"`.

```lua
ruster.on("mouse_down", function(ev)
  ruster.print(("clicked %s at %d,%d"):format(ev.button, ev.col, ev.row))
end)
```

**Returning `true` consumes the event**, cancelling ruster's own handling of it.
Other handlers still run — subscribers are independent, and a plugin that
registered second is not silenced by one that registered first. A handler that
throws is skipped: it neither consumes the event nor stops the handlers after
it, so a broken plugin cannot leave the mouse dead.

```lua
-- Middle-click pastes nothing; claim it for something else.
ruster.on("mouse_down", function(ev)
  if ev.button == "middle" then
    ruster.cmd("Files")
    return true
  end
end)
```

The payload is a table:

| Field | Type | Notes |
|-------|------|-------|
| `kind` | string | `down`, `up`, `drag`, `move`, `wheel_up`, `wheel_down`, `wheel_left`, `wheel_right` |
| `col`, `row` | integer | Cell coordinates, origin top-left |
| `button` | string | `left`, `right`, `middle`, `none` |
| `zone` | string | `buffer`, `gutter`, `chrome`, `float`, `outside` |
| `alt`, `ctrl`, `shift` | boolean | Modifier keys held |
| `offset` | integer? | Character offset into the buffer |
| `window` | integer? | Window id |
| `line` | integer? | 0-indexed buffer line |
| `col_in_line` | integer? | 0-indexed column within that line |

The last four are present only when the pointer is over buffer text
(`zone == "buffer"`); elsewhere they are `nil` rather than a misleading zero.

`"hover"` fires with the same payload once the pointer has been still over
buffer text for `mouse.hover_delay_ms` (300 by default). It fires once per
resting place, not once per frame, and re-arms when the pointer moves.

```lua
ruster.on("hover", function(ev)
  ruster.print(("resting on line %d"):format(ev.line))
end)
```

### `ruster.context_menu.add(zone, item)`

Add a row to the right-click menu. `zone` is `"buffer"`, `"gutter"`, `"chrome"`
or `"tab"`; `item.action` is a cmdline command — the same string `:` would take.

```lua
ruster.context_menu.add("buffer", { label = "Stage hunk", action = "GitStageHunk" })
```

Items append after the built-in ones for that zone. The menu is a picker, so it
filters as you type. Set `mouse.right_click_menu = false` to suppress the
built-in menu entirely and handle right-click yourself via `"mouse_down"`.

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

#### Windows & buffers

```lua
-- List open buffer ids and window ids
local bufs = ruster.api.nvim_list_bufs()   -- { 1, 2, ... }
local wins = ruster.api.nvim_list_wins()   -- { 1, 2, ... }

-- Active window, and the buffer a window shows
local win = ruster.api.nvim_get_current_win()
local buf = ruster.api.nvim_win_get_buf(win)

-- Point a window at a buffer
ruster.api.nvim_win_set_buf(win, buf)

-- Split the active window (true = vertical); returns the new window id
local new_win = ruster.api.nvim_open_win(true)

-- Close a window
ruster.api.nvim_win_close(new_win)
```

Buffer and window ids are stable integers. `nvim_open_win` splits the active
window; `nvim_win_close` closes the active window (id-targeted close is a
follow-up).

### `ruster.statusline.section(pos, fn)`

Register a statusline component. `pos` is `"left"`, `"center"`, or `"right"`.
`fn` is called each frame and must return a string; empty strings are skipped.
Sections are shown on the active window's statusline, appended to the built-in
components (mode, file name, percentage, and `line,col`). The statusline uses
the active theme's `statusline_fg` color and `divider` background; the first
segment (mode) is highlighted on the `accent` background for the active window.

```lua
-- Show the current git branch on the right side of the statusline.
ruster.statusline.section("right", function()
  return "⎇ main"
end)
```

### `ruster.lsp`

Language-server configuration. `ruster.lsp.servers[filetype]` overrides or adds
the server command for a filetype (keys match ruster's language names: `rust`,
`python`, `javascript`, `typescript`, `c`, `lua`, `scheme`).

```lua
ruster.lsp = {
  servers = {
    scheme = { cmd = "scheme-lsp-server", args = { "--stdio" } },
  },
}
```

### `ruster.config`

Configuration table. See [Config Reference](config-reference.md).

Built-in themes: `default`, `gruvbox`, `tokyonight`, `nord`, `catppuccin-mocha`
(the default), `starship`. Set with `ruster.config.general.theme = "starship"`.

```lua
ruster.config.tabstop = 2
ruster.config.number = true
ruster.config.format_on_save = true
```

## `ruster.ui.dialog(spec)`

Show a modal form. The editor owns the widgets; Lua describes the fields and
receives the answers.

```lua
ruster.ui.dialog{
  title = "Deploy",
  fields = {
    { label = "Dry run", kind = "toggle", value = "on" },
    { label = "Target",  kind = "select", value = "staging", options = { "staging", "prod" } },
    { label = "Message", kind = "text",   value = "ship it" },
    { label = "Retries", kind = "number", value = "3" },
    { label = "OK",      kind = "button" },
    { label = "Cancel",  kind = "button" },
  },
  on_submit = function(values, button)
    -- values is keyed by label; button is the one pressed, or nil for Enter.
    ruster.print(values.Target .. " " .. tostring(button))
  end,
}
```

| `kind` | Behaviour |
|--------|-----------|
| `toggle` | `Space` flips it; value is `"on"` / `"off"` |
| `select` | `Space`/`h`/`l` cycle `options`, wrapping |
| `text` | `Enter` edits in place, `Enter` again commits, `Esc` abandons the edit |
| `number` | as `text` |
| `button` | `Space`/`Enter` submits, reporting this label |

Anything else is treated as `text`, so a typo still shows the dialog.

`j`/`k` move, `Enter` submits from a non-text field, `Esc` cancels. **A button
always submits** — including one labelled "Cancel"; the plugin decides what each
label means. `on_submit` is not called when the dialog is cancelled with `Esc`.

`values` excludes buttons, since they are actions rather than fields.
