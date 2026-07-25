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
  -- terminal_shell = "/bin/bash",  -- default: $SHELL / %COMSPEC%
  -- terminal_scrollback = 10000,
}
```

## Settings

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `tabstop` | integer | 4 | Number of spaces a tab character represents |
| `softtabstop` | integer | 4 | Number of spaces inserted when pressing Tab |
| `expandtab` | boolean | true | Use spaces instead of tab characters |
| `number` | boolean | false | Show absolute line numbers in the gutter |
| `relativenumber` | boolean | false | Show relative line numbers (distance from cursor). With `number` also on, the gutter is hybrid: the cursor line shows its absolute number, other lines show the relative distance |
| `theme` | string | "default" | Color theme name |
| `cursor_anim_enabled` | boolean | true | Enable smooth cursor animation |
| `cursor_anim_speed` | float | 12.0 | Smooth-cursor easing speed |
| `timeoutlen` | integer | 300 | Milliseconds before the which-key panel appears for a pending key prefix |
| `format_on_save` | boolean | false | Format the buffer via LSP before writing on `:w` |
| `terminal_shell` | string | _(platform default)_ | Program `:term` launches. Unset uses `$SHELL` (Unix) / `%COMSPEC%` (Windows), falling back to `/bin/sh` / `cmd.exe`. Program only — no argument splitting |
| `terminal_scrollback` | integer | 10000 | Lines of scrollback history an embedded terminal retains |

## Embedded terminal

`:term` (or `:terminal`) opens a shell in the current window. Keys are forwarded to the
shell while the terminal is **focused**; press `Ctrl-\` to return to editor keys (window
navigation, `:` commands), and `i` / `a` / `Enter` to re-enter it. The terminal resizes
to its window automatically and is torn down on quit. See
[windows.md](windows.md) for the Windows/ConPTY requirements.

> **Keys & commands:** all keybindings and `:` commands live in the
> [Commands & Keybindings reference](keybindings.md). This page covers only the
> configurable settings.

## Language servers (LSP)

Supported filetypes spawn a language server automatically (if installed) and
send document changes. See [keybindings.md](keybindings.md) for the code-action
keys. Default servers: `rust-analyzer`, `pyright-langserver`,
`typescript-language-server`, `clangd`, `lua-language-server`, `scheme-lsp-server`.
Override or add servers in `init.lua`:

```lua
ruster.lsp = {
  servers = {
    scheme = { cmd = "my-scheme-lsp", args = { "--stdio" } },
    rust   = { cmd = "rust-analyzer", args = {} },
  },
}
ruster.config.format_on_save = true
```

## Snippets

Tab in insert mode expands a snippet whose trigger word precedes the cursor, then
jumps to `$1`; further Tabs cycle through the remaining tabstops. A small built-in
set ships for Rust/Python/Lua. Add your own in
`~/.config/ruster/snippets/<filetype>.snippets`, one per line as
`trigger<TAB>body`, where `\n` in the body is a newline and `$1`/`$2`/`$0`/`${1:default}`
are tabstops:

```
fn	fn ${1:name}(${2:args}) {\n    $0\n}
```
