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
| `C-v` | Normal | Visual (block-wise / column) |
| `:` | Normal | Command-line |
| `/` `?` | Normal | Search prompt (forwards / backwards) |
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
| `f{char}` / `F{char}` | Jump to next / previous occurrence of `{char}` on the line |
| `t{char}` / `T{char}` | Jump just before / after `{char}` on the line |
| `;` / `,` | Repeat the last find forwards / backwards |
| `%` | Jump to the matching bracket |
| `/{pat}` / `?{pat}` | Search forwards / backwards (wraps around) |
| `n` / `N` | Repeat the search in the same / opposite direction |
| `*` | Search forwards for the word under the cursor |
| `gg` | Top of buffer |
| `G` | Bottom of buffer |

## Normal mode — editing

| Key | Action |
|-----|--------|
| `i` `a` | Insert before / append after the cursor |
| `I` `A` | Insert at first non-blank / append at end of line |
| `o` `O` | Open a new line below / above |
| `x` / `X` | Delete character under / before the cursor |
| `r{char}` | Replace the character under the cursor |
| `~` | Toggle the case of the character under the cursor |
| `D` / `C` | Delete / change to end of line |
| `Y` / `S` | Yank / change the whole line |
| `p` / `P` | Paste after / before the cursor (line-wise registers paste below / above) |
| `u` | Undo |
| `C-r` | Redo |
| `.` | Repeat last change |
| `C-d` / `C-u` | Scroll a half page down / up (cursor keeps its screen row) |
| `C-n` | Add a cursor at the next occurrence of the word under the cursor (multi-cursor) |
| `q{reg}` … `q` | Record a macro into `{reg}` / stop recording |
| `@{reg}` | Replay the macro in `{reg}` |
| `g-` / `g+` | Step backward/forward through *every* undo state in time order, including branches abandoned by editing after an undo |
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

`v` selects character-wise, `V` line-wise, and `C-v` **block-wise** (a column
rectangle). Extend the selection with motions `h` `j` `k` `l` `w` `b` `e` `0`,
then act:

| Key | Action |
|-----|--------|
| `d` / `x` | Delete selection |
| `c` | Change selection |
| `y` | Yank selection |
| `>` / `<` | Indent / de-indent |
| `Esc` | Leave visual mode |

In block mode `d`/`x`/`y` operate on the rectangle: every line's selected
columns are removed or copied (rows joined by newlines), and lines shorter than
the block are clipped rather than padded.

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
| `SPC c b` | Build the project (`:build`) |
| `SPC c t` | Run tests (`:test`) |
| `SPC c k` (or `K`) | LSP hover |
| `SPC c g` | Go to definition |
| `SPC c r` | Find references |
| `SPC c n` | Rename (see `:rename`) |
| `SPC c f` | Format (`:fmt`) |
| `SPC c o` | Document symbols (outline) |
| `SPC c d` | Diagnostics list |
| `SPC c i` | Incoming calls (`:callers`) |
| `SPC c y` | Outgoing calls (`:callees`) |
| `SPC b b` / `SPC b d` | Buffer list / delete buffer |
| `SPC s f` / `SPC s g` | Search files / grep (ripgrep) |
| `SPC s s` / `SPC s d` | Document symbols / diagnostics |
| `SPC o t` / `SPC o s` / `SPC o e` | Open terminal / settings / explorer |
| `SPC u n` / `SPC u r` | Toggle line / relative numbers |
| `SPC q q` | Quit |
| `SPC q w` | Save and quit |

## `g` menu (goto)

Press `g` in Normal mode; the which-key panel lists the goto commands (LazyVim-style).

| Sequence | Action |
|----------|--------|
| `g d` | Go to definition (LSP) |
| `g r` | References (LSP) |
| `g h` | Hover (LSP) |
| `g g` | Top of buffer |
| `g -` / `g +` | Older / newer change (undo-tree, by time) |

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
| `:term` `:terminal` | Open an embedded terminal |
| `:settings` `:config` | Open the settings page |
| `:config-errors` | Show config load/validation errors |

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
| `:callers` / `:incomingcalls` | Call hierarchy: who calls the symbol under the cursor |
| `:callees` / `:outgoingcalls` | Call hierarchy: what the symbol under the cursor calls |

### Build & quickfix
Diagnostics render as a **sign column** left of the line-number gutter (`E`/`W`/`I`/`H`,
colored by severity). The **quickfix list** collects them for navigation.

| Command | Action |
|---------|--------|
| `:build` / `:make` (or `SPC c b`) | Run the project's build command in the background; output streams to `*build*`, diagnostics feed the quickfix list |
| `:test` (or `SPC c t`) | Run the project's test command; output streams to `*test*`, failures get ✗ gutter signs and feed the quickfix list |
| `:copen` (`:cwindow`) | Open the quickfix list as a picker; Enter jumps to an entry |
| `:cnext` / `:cn` (or `]q`) | Jump to the next quickfix entry |
| `:cprev` / `:cp` (or `[q`) | Jump to the previous quickfix entry |

### Editing
| Command | Action |
|---------|--------|
| `:s/pat/rep/` | Replace the first match on the current line |
| `:s/pat/rep/g` | Replace every match on the current line |
| `:%s/pat/rep/g` | Replace throughout the buffer |

Then `:` + `Tab` opens a fuzzy **command palette** of these commands.

## Emacs mode

ruster ships two editing paradigms. Switch at any time:

| Command | Action |
|---------|--------|
| `:set editmode emacs` | Modeless Emacs editing |
| `:set editmode neovim` | Modal vim editing (the default) |

Lua plugins can read the active paradigm from `ruster.editmode` (`"neovim"` /
`"emacs"`). The statusline shows `-- EMACS --` while it is active.

In Emacs mode every printable key self-inserts; `Ctrl`/`Alt` chords are commands:

| Key | Action |
|-----|--------|
| `C-f` / `C-b` | Forward / backward char |
| `C-n` / `C-p` | Next / previous line |
| `C-a` / `C-e` | Beginning / end of line |
| `M-f` / `M-b` | Forward / backward word |
| `M-<` / `M->` | Beginning / end of buffer |
| `C-v` / `M-v` | Scroll down / up a page |
| `C-SPC` | Set the mark (start a region) |
| `C-w` / `M-w` | Kill (cut) / copy the region |
| `C-k` | Kill to end of line (or the newline) |
| `M-d` | Kill word forward |
| `C-y` | Yank (paste) the last kill |
| `M-y` | Yank-pop (cycle the kill ring) — right after a yank |
| `C-d` / `Del` | Delete the character after the cursor |
| `C-u <n>` | Universal argument: repeat the next command `n` times |
| `C-/` or `C-_` | Undo |
| `C-s` / `C-r` | Incremental search forward / backward (repeat to jump) |
| `M-x` | Run a command (opens the command palette) |
| `C-g` | Cancel — clear the mark / prefix |
| `C-x C-s` | Save the file |
| `C-x C-c` | Quit |
| `C-x C-f` | Find file · `C-x C-b` buffer list |
| `C-x u` | Undo |
| `C-x 0` / `1` / `2` / `3` | Close / only / split-below / split-right window |

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
| `+` | New file — or directory if the name ends with `/` |
| `.` | Toggle hidden (dot-)files |
| `g?` | Show this keymap in a popup |

A dired buffer is a **read-only buffer**, so the normal editor keys work over
the listing while its own keys above take priority:

| Key | Action |
|-----|--------|
| `/` `?` `n` `N` | Search the listing and jump between matches |
| `:` … | Any `:` command (`:q`, `:Files`, …) |
| `gg` / `G` | Top / bottom of the listing |
| `SPC` … | The Space leader / which-key menu |
| (Emacs mode) `C-s` `C-r`, `M-x` | Incremental search, run a command |

Editing keys (`i`, `x`, `dd`-as-delete-line, …) are inert — the listing can't be
typed into. Copy/cut/paste refuse to overwrite an existing name, and a cut is
consumed by its paste (the original is removed only after a successful move).

Entries are colored by type: **directories** blue (bold, with a trailing `/`),
**executables** green, **symlinks** teal, and regular files in the default
foreground.

## Snippets

In Insert mode, type a trigger word and press `Tab` to expand it, then `Tab` to
cycle through the tabstops. Built-in triggers include `fn`/`pfn`/`impl`/`test`
(Rust), `def`/`class` (Python), `fn` (Lua); add your own in
`~/.config/ruster/snippets/<filetype>.snippets`.

## Settings page

`:settings` opens a grouped, interactive editor for `config.lua`.

| Key | Action |
|-----|--------|
| `j` `k` / `↓` `↑` | Move between settings |
| `Tab` / `[` `]` | Jump to the next / previous group |
| `Space` `Enter` | Toggle a boolean, cycle an enum, or start editing a text/number field |
| `h` `l` / `←` `→` | Adjust a number or cycle an enum |
| _type_ then `Enter` | Commit a text/number edit (`Esc` cancels) |
| `:w` | Save to `config.lua` |
| `q` `Esc` | Close the page |

## Embedded terminal

`:term` opens a shell in the current window (`terminal.shell` /
`terminal.scrollback` config it — see [config-reference.md](config-reference.md)).

It has two modes, like Neovim's terminal:

**Terminal-Insert** (`-- TERMINAL --`) — keys go to the shell.

| Key | Action |
|-----|--------|
| _any key_ | Forwarded to the shell (`Ctrl-C`, arrows, Tab-completion, …) |
| `Ctrl-\` | Switch to Terminal-Normal |

**Terminal-Normal** — the visible output is mirrored into a read-only buffer, so the
normal editor keys work over it:

| Key | Action |
|-----|--------|
| `h j k l` `w` `b` `gg` `G` … | Move over the terminal output |
| `v` / `V` then `y` | Visually select and yank terminal output |
| `:` commands, `Ctrl-w` nav | Run commands / switch windows |
| `i` `a` `Enter` | Resume Terminal-Insert (back to the shell) |

The terminal resizes to its window automatically and is closed when ruster quits.
On Windows it needs ConPTY (Windows 10 1809+); see [windows.md](windows.md).
