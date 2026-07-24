# Commands & Keybindings

The complete reference of keybindings and `:` commands ruster implements today.
All keys and commands work identically in the TUI and GUI backends.

> Notation: `C-x` = Ctrl+x, `SPC` = the Space leader, `<CR>` = Enter.

## Modes

| Key | From | Action |
|-----|------|--------|
| `i` | Normal | Insert mode (before cursor) |
| `v` | Normal | Visual (character-wise) |
| `V` | Normal | Visual (line-wise) |
| `:` | Normal | Command-line |
| `Esc` | Insert / Visual / Cmdline | Back to Normal (also clears extra cursors) |

## Normal mode — motions

Most motions accept a count prefix (e.g. `5j`, `3w`).

| Key | Motion |
|-----|--------|
| `h` `l` | Left / right one character |
| `j` `k` | Down / up one line |
| `0` | Start of line |
| `$` | End of line |
| `w` `b` `e` | Next word start / previous word start / word end |
| `gg` | Top of buffer |
| `G` | Bottom of buffer |

## Normal mode — editing

| Key | Action |
|-----|--------|
| `x` | Delete character under cursor |
| `p` | Paste (system clipboard, falls back to register) |
| `u` | Undo |
| `C-r` | Redo |
| `.` | Repeat last change |
| `C-d` | Add a cursor at the next occurrence of the word under the cursor (multi-cursor) |
| `Esc` | Clear extra cursors |

### Operators

An operator followed by a motion or text object acts on that range. Doubling the
operator (`dd`, `yy`, `cc`) acts on the current line.

| Operator | Meaning |
|----------|---------|
| `d` | Delete |
| `c` | Change (delete then insert) |
| `y` | Yank (copy) |
| `>>` | Indent line |
| `<<` | De-indent line |

Motions usable after `d`/`c`/`y`: `w` `b` `e` `0` `$` `G` (and the doubled form).

### Text objects

Used after an operator: `i` = inner, `a` = around.

| Text object | Selects |
|-------------|---------|
| `iw` / `aw` | Word |
| `i"` `a"` / `i'` `a'` | Quoted string |
| `i(` `a(` / `i{` `a{` | Bracketed block |
| `if` / `af` | Function (tree-sitter) |
| `ic` / `ac` | Class (tree-sitter) |
| `il` / `al` | Loop (tree-sitter) |
| `ia` / `aa` | Argument / parameter (tree-sitter) |

Examples: `daf` delete around function, `ciw` change inner word, `yi(` yank inside parens.

## Visual mode

Extend the selection with motions `h` `j` `k` `l` `w` `b` `e` `0`, then act:

| Key | Action |
|-----|--------|
| `d` / `x` | Delete selection |
| `c` | Change selection |
| `y` | Yank selection |
| `>` / `<` | Indent / de-indent |
| `Esc` | Leave visual mode |

## Insert mode

| Key | Action |
|-----|--------|
| `Esc` | Return to Normal |
| `Tab` | Expand a snippet trigger, or cycle to the next tabstop, else insert indentation |
| `<CR>` | New line |
| `Backspace` | Delete back |

## Windows

| Key | Command | Action |
|-----|---------|--------|
| `C-w s` | `:split` / `:sp` | Split horizontally |
| `C-w v` | `:vsplit` / `:vs` | Split vertically |
| `C-w c` | `:close` / `:clo` | Close window |
| `C-w o` | `:only` / `:on` | Close other windows |
| `C-w h/j/k/l` | | Focus window left/down/up/right |
| `C-h/j/k/l` | | Focus window (GUI-reliable; terminal-dependent in TUI) |
| `C-w z` | `:fullscreen` / `:fs` | Toggle fullscreen for the active window |

## Space leader (which-key)

Press `SPC` in Normal mode; the which-key panel lists continuations.

| Sequence | Action |
|----------|--------|
| `SPC w h/j/k/l` | Focus window left/down/up/right |
| `SPC w s` / `SPC w v` | Split below / right |
| `SPC w c` / `SPC w q` | Close window |
| `SPC w o` | Close other windows |
| `SPC w z` | Toggle fullscreen |
| `SPC f f` | Find files (`:Files`) |
| `SPC f b` | Buffer list (`:ls`) |
| `SPC f e` | File explorer (`:Dired`) |
| `SPC c k` (or `K`) | LSP hover |
| `SPC c g` | Go to definition |
| `SPC c r` | Find references |
| `SPC c n` | Rename (see `:rename`) |
| `SPC c f` | Format (`:fmt`) |
| `SPC c o` | Document symbols (outline) |
| `SPC c d` | Diagnostics list |
| `SPC q q` | Quit |
| `SPC q w` | Save and quit |

## `:` commands

### Files & quitting
| Command | Action |
|---------|--------|
| `:w` / `:write` | Write file |
| `:w <path>` | Write to a path |
| `:w!` | Force write |
| `:q` / `:quit` | Close window (quit when it's the last) |
| `:q!` | Force quit |
| `:wq` / `:x` | Write and quit |

### Windows & buffers
| Command | Action |
|---------|--------|
| `:split` `:sp` / `:vsplit` `:vs` | Split horizontally / vertically |
| `:close` `:clo` / `:only` `:on` | Close window / close others |
| `:fullscreen` `:fs` | Toggle fullscreen |
| `:ls` `:buffers` `:ibuffer` | Buffer list picker |
| `:bd` `:bdelete` | Delete the active buffer |

### Navigation
| Command | Action |
|---------|--------|
| `:Dired [path]` `:Explore` `:Ex` | File explorer |
| `:Files` | Fuzzy file finder |
| `:Rg <pattern>` | Live grep (ripgrep) |

### Code (LSP)
| Command | Action |
|---------|--------|
| `:fmt` / `:format` | Format the buffer |
| `:rename <name>` | Rename the symbol under the cursor |
| `:sym <query>` | Workspace symbol search |

Then `:` + `Tab` opens a fuzzy **command palette** of these commands.

## Pickers (ibuffer, files, references, symbols, command palette)

| Key | Action |
|-----|--------|
| *type* | Filter (fuzzy) |
| `C-n` / `C-p` (or `↓`/`↑`) | Move selection |
| `<CR>` | Accept |
| `Esc` | Cancel |

Pickers that select a file (files, buffers, references, symbols, diagnostics)
show a **syntax-highlighted preview** of the selected entry beside the list,
scrolled to the target line for location results.

## Dired (file explorer)

| Key | Action |
|-----|--------|
| `<CR>` / `l` | Open file / descend into directory |
| `h` / `-` / `^` | Go to parent directory |
| `j` / `k`, `C-n` / `C-p` | Move cursor |
| `yy` | Copy entry |
| `dd` | Cut entry |
| `p` | Paste into this directory |
| `R` | Rename entry |
| `D` | Delete entry (with `y`/`n` confirm) |
| `+` / `n` | New file — or directory if the name ends with `/` |
| `?` | Show this keymap in a popup |

Copy/cut/paste refuse to overwrite an existing name, and a cut is consumed by
its paste (the original is removed only after a successful move).

Entries are colored by type: **directories** blue (bold, with a trailing `/`),
**executables** green, **symlinks** teal, and regular files in the default
foreground.

## Snippets

In Insert mode, type a trigger word and press `Tab` to expand it, then `Tab` to
cycle through the tabstops. Built-in triggers include `fn`/`pfn`/`impl`/`test`
(Rust), `def`/`class` (Python), `fn` (Lua); add your own in
`~/.config/ruster/snippets/<filetype>.snippets`.
