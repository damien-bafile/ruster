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
| `relativenumber` | boolean | false | Show relative line numbers (distance from cursor). With `number` also on, the gutter is hybrid: the cursor line shows its absolute number, other lines show the relative distance |
| `theme` | string | "default" | Color theme name |
| `cursor_anim_enabled` | boolean | true | Enable smooth cursor animation |
| `cursor_anim_speed` | float | 12.0 | Smooth-cursor easing speed |
| `timeoutlen` | integer | 300 | Milliseconds before the which-key panel appears for a pending key prefix |
| `format_on_save` | boolean | false | Format the buffer via LSP before writing on `:w` |

## Windows & buffers

ruster is multi-buffer and multi-window. Relevant commands:

| Command | Action |
|---------|--------|
| `:split` / `:sp` | Split the active window horizontally |
| `:vsplit` / `:vs` | Split the active window vertically |
| `:close` / `:clo` | Close the active window (`Ctrl-w c`) |
| `:only` / `:on` | Close all other windows (`Ctrl-w o`) |
| `:fullscreen` | Toggle fullscreen for the active window (`Ctrl-w z`) |
| `:ls` / `:buffers` | Open the buffer-list picker |
| `:bd` / `:bdelete` | Delete the active buffer |
| `:Dired [path]` | Open the file explorer |
| `:Files` | Fuzzy file finder (gitignore-aware, streamed) |
| `:Rg <pattern>` | Live grep via ripgrep (streamed) |

In a dired buffer: `Enter` open/descend, `-`/`^` up, `+` create file, `%` create
directory, `R` rename, `D` delete (with `y`/`n` confirmation).

Window keys: `Ctrl-w s/v` split, `Ctrl-w c` close, `Ctrl-w o` only, `Ctrl-w h/j/k/l` focus, `Ctrl-w z` fullscreen. `Ctrl-h/j/k/l` also move focus directly. Press `:` then `Tab` for the command palette.

### Space leader (which-key)

Press `Space` in Normal mode to open the which-key panel, then a group and key:

| Sequence | Action |
|----------|--------|
| `Space w h/j/k/l` | Focus the window left/down/up/right |
| `Space w s` / `Space w v` | Split below / right |
| `Space w c` (or `q`) | Close window |
| `Space w o` | Close other windows |
| `Space w z` | Toggle fullscreen |
| `Space f f` | Find files |
| `Space f b` | Buffer list |
| `Space f e` | File explorer (dired) |
| `Space q q` | Quit |
| `Space q w` | Save and quit |

Each level shows its continuations in the which-key panel, which slides up from
the bottom mini-buffer and back down when the sequence resolves or is cancelled
with `Esc`.

## Language servers (LSP)

Supported filetypes spawn a language server automatically (if installed) and
send document changes. Diagnostics appear on the cursor line; code actions are
under the `Space c` leader:

| Key / command | Action |
|---------------|--------|
| `K` / `Space c k` | Hover documentation |
| `Space c g` | Go to definition |
| `Space c r` | Find references |
| `Space c n` (or `:rename <name>`) | Rename symbol |
| `Space c f` (or `:fmt`) | Format buffer |
| `Space c o` | Document symbols (outline) |
| `Space c d` | Diagnostics list |
| `:sym <query>` | Workspace symbol search |

Default servers: `rust-analyzer`, `pyright-langserver`, `typescript-language-server`,
`clangd`, `lua-language-server`, `scheme-lsp-server`. Override or add servers in
`init.lua`:

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
