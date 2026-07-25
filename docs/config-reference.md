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

`:settings` (or `:config`) opens an interactive, grouped editor. `j`/`k` move, `Tab`
switches group, `Space`/`Enter` toggles or cycles, `h`/`l` adjust numbers/enums, `Enter`
edits text fields, and **`:w` saves** to `config.lua`. The **theme**, **font**, and **shell**
rows are pickers: they cycle through the themes in `themes/`, the fonts installed on your
system, and the shells found on `$PATH` (bash, zsh, ksh, tcsh, fish, … / PowerShell, cmd).
See [keybindings.md](keybindings.md#settings-page).

Launching `ruster <directory>` opens the file explorer (dired) at that location.

## Themes

Colors come from a **theme**, not individual settings. Built-in themes (`default`,
`gruvbox`, `tokyonight`, `nord`, `catppuccin-mocha`) are written to
`~/.config/ruster/themes/<name>.lua` on first run; pick one with `general.theme = "gruvbox"`. Each theme file is a Lua chunk
returning a palette you can edit or copy:

```lua
-- themes/mytheme.lua
return { bg = "#1e1e1e", fg = "#cdd6f4", gutter = "#6c7086",
         selection = "#585b70", cursor = "#f5e0dc", divider = "#45475a", accent = "#f38ba8" }
```

Theme changes apply to the GUI on restart.

### Recoloring individual elements

Pick a theme, then recolor individual UI elements **from that theme's palette**. The
**Colors** group in the Settings page has a row per element (background, foreground,
gutter, selection, cursor, **bars/divider**, accent); each is a picker that cycles the
**selected theme's named palette** (e.g. Catppuccin Mocha's `mauve`, `blue`, `surface0`,
…), shown by name with a swatch. `theme` means "leave at the theme's default". Changing
the Theme row updates the color pickers live. These map to `ruster.config.colors.*`
(stored as hex). GUI color changes apply on restart.

Each `themes/<name>.lua` file carries both the 7 UI `roles` and a `palette` of named
colors to choose from.

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
| `lsp.format_on_save` | boolean | false | Format via LSP before writing on `:w` |
| `lsp.diagnostics` / `hover` / `autostart` | boolean | true | LSP feature toggles |
| `terminal.shell` | string | _(platform)_ | `:term` program. Unset → `$SHELL` / `%COMSPEC%` (→ `/bin/sh` / `cmd.exe`) |
| `terminal.scrollback` | integer | 10000 | Terminal scrollback lines |
| `terminal.default_mode` | enum | "insert" | New terminal starts in `insert` or `normal` |
| `dired.show_hidden` | boolean | false | Show dotfiles in the file explorer |

> Colors are **not** listed here — they live in [themes](#themes).

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
