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
| `number` | boolean | false | Show absolute line numbers in the gutter. Toggle live with `:set number` / `:set nonumber` / `:set number!` (abbrev. `nu`) |
| `relativenumber` | boolean | false | Show relative line numbers (distance from cursor). With `number` also on, the gutter is hybrid: the cursor line shows its absolute number, other lines show the relative distance. Toggle live with `:set relativenumber` / `:set norelativenumber` / `:set relativenumber!` (abbrev. `rnu`) |
| `theme` | string | "default" | Color theme name |
| `cursor_anim_enabled` | boolean | true | Enable smooth cursor animation |
| `cursor_anim_speed` | float | 12.0 | Smooth-cursor easing speed |
| `timeoutlen` | integer | 300 | Milliseconds before the which-key panel appears for a pending key prefix |
| `format_on_save` | boolean | false | Format the buffer via LSP before writing on `:w` |

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
