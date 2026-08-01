# Config Reference

ruster uses two files in its config directory — `~/.config/ruster/` on Linux **and
macOS** (or `$XDG_CONFIG_HOME/ruster/` if set), `%APPDATA%\ruster\` on Windows:

- **`config.lua`** — declarative settings, **generated on first run** and managed by the
  in-app Settings page (`:settings`, save with `:w`). Safe to hand-edit; regenerated on
  save. Settings are grouped tables under `ruster.config`.
- **`init.lua`** — *optional* advanced scripting (keymaps, plugins, arbitrary Lua). Never
  rewritten by ruster; loaded **after** `config.lua`, so it can override any setting.

```lua
-- config.lua (generated; grouped by area)
ruster.config.general  = { tabstop = 4, editmode = "neovim", theme = "default" }
ruster.config.gui      = { font_size = 20, cursor_kind = "block" }
ruster.config.gutter   = { number = false, relativenumber = false }
ruster.config.whichkey = { enabled = true, timeoutlen = 300 }
ruster.config.lsp      = { format_on_save = false }
ruster.config.terminal = { shell = "", scrollback = 10000 }
ruster.config.dired    = { show_hidden = false }
```

Invalid values are reported (`:config-errors`) and fall back to their defaults — ruster
never refuses to start over a bad config. A legacy flat `ruster.config = { … }` table is
still read for backward compatibility.

## Settings page

`:settings` (or `:config`) opens an interactive, grouped editor. `j`/`k` move, `gg`/`G`
jump to the top/bottom, `Tab` switches group, `Space`/`Enter` toggles or cycles, `h`/`l`
adjust numbers/enums, `Enter` edits text fields, `dd`/`Delete` resets a row to its
default, and **`:w` saves** to `config.lua`. The **theme**, **font**, and **shell**
rows are pickers: they cycle through the themes in `themes/`, the fonts installed on your
system, and the shells found on `$PATH` (bash, zsh, ksh, tcsh, fish, … / PowerShell, cmd).
See [keybindings.md](keybindings.md#settings-page).

Launching `ruster <directory>` opens the file explorer (dired) at that location.

## Themes

Colors come from a **theme**, not individual settings. Built-in themes (`default`,
`gruvbox`, `tokyonight`, `nord`, `catppuccin-mocha`, `starship`) are written to
`~/.config/ruster/themes/<name>.lua` on first run; pick one with `general.theme = "gruvbox"`. Each theme file is a Lua chunk
returning a palette you can edit or copy:

```lua
-- themes/mytheme.lua
return { bg = "#1e1e1e", fg = "#cdd6f4", gutter = "#6c7086", gutter_bg = "#1e1e1e",
         selection = "#585b70", selection_fg = "#cdd6f4",
         cursor = "#f5e0dc", cursor_fg = "#1e1e1e",
         divider = "#45475a", statusline_fg = "#cdd6f4",
         accent = "#f38ba8", accent_fg = "#1e1e1e" }
```

The **starship** theme uses a CRT‑black background (`#0a0e0a`) with green‑phosphor
foreground (`#33ff66`) and amber accent (`#ff8800`) — a crew‑terminal / industrial
sci‑fi palette. It is the default visual direction for the Starship UI.

GUI theme, colors, font, and size changes apply **live** when you save the Settings page
with `:w` — no restart needed.

### Starship UI — panel chrome & welcome screen

When no file is open, ruster shows the **"Ready Room"** welcome screen — a centered
panel listing recent projects, quick actions, system status (LSP), and keybinds. It
renders in both GUI and TUI modes using the active theme's colors.

Every window is framed with a **panel header** (a ruled line at the top showing the
filename as a stencil label) and, in the GUI, a thin vertical seam between adjacent
windows — giving the editor a crew‑terminal / control‑panel feel.

### Recoloring individual elements

Pick a theme, then recolor individual UI elements **from that theme's palette**. The
**Colors** group in the Settings page has a row per element — background, foreground,
gutter, **gutter background**, selection + **selection text**, cursor + **cursor text**,
**bars/divider** + **bar/divider text**, and accent + **accent text** (the `*_fg`
companions color the glyphs drawn *over* each element: text on the selection, the glyph
under the block cursor, the statusline text, and text on accent bars). Each is a picker
that cycles the **selected theme's named palette** (e.g. Catppuccin Mocha's `mauve`,
`blue`, `surface0`, …), shown by name with a swatch. `theme` means "leave at the theme's
default". Changing the Theme row updates the color pickers live. These map to
`ruster.config.colors.*` (stored as hex), and apply to the GUI live.

Press **`dd`** (or **`Delete`**) on any Settings row to reset it to its default; for color
rows that means "use the theme's color".

Each `themes/<name>.lua` file carries both the 12 UI `roles` and a `palette` of named
colors to choose from.

### Syntax colors (per language)

Below the **Colors** group, the Settings page has a **Syntax** section listing every
language with syntax highlighting (rust, python, c, lua, json, toml, yaml, scheme, just,
markdown, org). Press **Enter** on a language to expand it into a color picker per syntax
group — code languages expose `keyword, string, comment, function, type, variable,
constant, number, operator, builtin`; markup (markdown/org) exposes `heading, strong,
emphasis, code, link, url, marker, quote, keyword, block, todo, done`. Each group row
cycles the **selected theme's palette** (same picker as the Colors group) and shows
`default` when left at the built-in color; `dd`/`Delete` resets a group. Overrides are
**per language** and apply to open buffers on `:w` (no restart).

They persist to a `ruster.config.syntax` table:

```lua
ruster.config.syntax = {
  rust   = { keyword = "#cba6f7", string = "#a6e3a1" },
  python = { comment = "#6c7086" },
}
```

## Settings

Keys are addressed as `group.key`.

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `general.tabstop` | integer | 4 | Spaces a tab represents |
| `general.softtabstop` | integer | 4 | Spaces inserted when pressing Tab |
| `general.expandtab` | boolean | true | Insert spaces instead of tab characters |
| `general.shiftwidth` | integer | 4 | Spaces per indent step |
| `general.editmode` | enum | "neovim" | Editing paradigm: `neovim` (modal) or `emacs` (modeless) |
| `general.editorconfig` | boolean | true | Honor project `.editorconfig` files |
| `general.line_ending` | enum | "lf" | Default line ending for new files: `lf` or `crlf` |
| `general.theme` | string | "default" | Theme name (see [Themes](#themes)) |
| `gui.font` | string | _(auto)_ | GUI font: absolute path or a font-dir filename. Unset tries common Nerd/mono fonts. **A Nerd Font is required for icon glyphs** — see [GUI font & icons](#gui-font--icons) |
| `gui.font_size` | integer | 20 | Glyph size in px |
| `gui.line_height` | integer | 24 | Row height in px |
| `gui.padding_x` / `padding_y` | integer | 8 / 4 | Window padding in px |
| `gui.window_width` / `window_height` | integer | 800 / 600 | Initial window size |
| `gui.target_fps` | integer | 60 | Render loop frame cap |
| `gui.cursor_kind` | enum | "block" | Cursor shape: `block` or `bar` |
| `gui.cursor_anim` | boolean | true | Smooth cursor animation |
| `gui.cursor_anim_speed` | float | 12.0 | Smooth-cursor easing speed |
| `gutter.number` | boolean | false | Show absolute line numbers. Toggle live with `:set number` / `:set nonumber` / `:set number!` (abbrev. `nu`) |
| `gutter.relativenumber` | boolean | false | Show relative line numbers (hybrid with `number`). Toggle live with `:set relativenumber` / `:set norelativenumber` / `:set relativenumber!` (abbrev. `rnu`) |
| `whichkey.enabled` | boolean | true | Show the which-key hint panel |
| `whichkey.timeoutlen` | integer | 300 | Milliseconds before the panel appears |
| `whichkey.command_palette` | enum | "center" | Where the `:`-Tab command palette appears: `center` (floating box) or `bottom` (docked in the which-key area) |
| `lsp.format_on_save` | boolean | false | Format via LSP before writing on `:w` |
| `lsp.diagnostics` / `hover` / `autostart` | boolean | true | LSP feature toggles |
| `terminal.shell` | string | _(platform)_ | `:term` program. Unset → `$SHELL` / `%COMSPEC%` (→ `/bin/sh` / `cmd.exe`) |
| `terminal.scrollback` | integer | 10000 | Terminal scrollback lines |
| `terminal.default_mode` | enum | "insert" | New terminal starts in `insert` or `normal` |
| `dired.show_hidden` | boolean | false | Show dotfiles in the file explorer |
| `sidebar.auto_open` | boolean | false | Open the sidebar automatically on startup |
| `noice.mini` | boolean | true | Show transient toasts in the cmdline row |
| `noice.notify` | boolean | true | Show the stacking panel for warnings and errors |
| `noice.split` | boolean | true | Allow `:Noice split` to open the `*noice*` history buffer |
| `noice.info_timeout` | integer | 2000 | Milliseconds an info toast stays up |
| `noice.success_timeout` | integer | 2000 | Milliseconds a success toast stays up |
| `noice.warning_timeout` | integer | 5000 | Milliseconds a warning stays up |
| `noice.max_history` | integer | 1000 | Messages retained for `:messages` and `:Noice split` |
| `build.command` | string | _(detect)_ | Command for `:build`; empty detects from the project type |
| `test.command` | string | _(detect)_ | Command for `:test`; empty detects from the project type |
| `dap.adapter` | string | _(detect)_ | Debug adapter program for `:debug`; empty detects from the file's language |

> Colors are **not** listed here — they live in [themes](#themes).

### Build, test & debug

`:build` and `:test` resolve their command **most-specific first**:

1. the project's `ruster.toml` (`[build] command` / `[test] command`),
2. your `build.command` / `test.command` setting,
3. a built-in default for the project type — `cargo build` / `cargo test` for a
   `Cargo.toml`, `npm run build` / `npm test` for a `package.json`, `go build ./...`
   / `go test ./...` for a `go.mod`, and `make` / `make test` for a `Makefile`.

An empty setting means "unset", so it falls through to the next level rather than
running an empty command.

`ruster.toml` also defines named tasks, which `:task` lists:

```toml
[build]
command = "cargo build --release"

[tasks.serve]
cmd = "python -m http.server"
use_terminal = true
```

`dap.adapter` overrides the debug adapter program. Left empty, `:debug` picks one
from the current file's language (`lldb-vscode`, `debugpy`, `dlv dap`). See
[Commands & Keybindings](keybindings.md) for the debugger keys.

### Notifications (noice)

Messages are routed by level: info and success go to the **mini** toast in the cmdline
row, warnings go to the **notify** panel *and* mirror into the mini toast, and errors go
to the notify panel only.

Errors are **persistent** — they stay until dismissed, and no `noice.*_timeout` applies
to them. Disabling a backend only suppresses its on-screen display; every message is
still recorded in history, so `:messages` and `:Noice split` remain complete.

## GUI font & icons

The GUI (raylib) needs a **Nerd Font** to show icon glyphs — file-type icons in `ls`/
`eza`, Powerline segments, etc. Without one they render as `?`.

1. Install a Nerd Font (e.g. `brew install --cask font-jetbrains-mono-nerd-font`, or any
   from [nerdfonts.com](https://www.nerdfonts.com)).
2. ruster auto-detects common ones (JetBrains Mono, FiraCode, Cascadia, Hack, Meslo —
   preferring the `…Mono` variants for grid alignment). To force a specific font:

   ```lua
   ruster.config.gui.font = "FiraCodeNerdFontMono-Regular.ttf"  -- filename in the font dir
   -- or an absolute path:
   -- ruster.config.gui.font = "/Users/me/Library/Fonts/HackNerdFontMono-Regular.ttf"
   ```

The atlas bakes the standard Nerd Font icon ranges (Seti-UI, Devicons, Font Awesome,
Octicons, Codicons, Powerline, box-drawing). In `--tui` mode, icons come from your
terminal emulator's font instead, not this setting.

## Embedded terminal

`:term` (or `:terminal`) opens a shell in the current window. It has two modes like
Neovim's terminal: **Terminal-Insert** (keys go to the shell) and **Terminal-Normal**
(`Ctrl-\`), where the output is mirrored into a read-only buffer so vim motions, visual
selection and yank work over it; `i` / `a` / `Enter` resume the shell. The terminal
resizes to its window automatically and is torn down on quit. See
[keybindings.md](keybindings.md) for the full key list and [windows.md](windows.md) for
the Windows/ConPTY requirements.

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
