# Phase 1 Feature Completion: Multi-Cursor, Clipboard, Tabs, EditorConfig

## Overview

Complete the four remaining/incomplete Phase 1 features in ruster's text editing primitives. Emacs dual-paradigm mode is deferred to a separate project.

## 1. Multi-Cursor

### Data Model
Extend the existing `CursorSet` in `ruster-core/src/cursor.rs`:
- `add_cursor(byte_offset: usize)` — insert a new cursor at the given position, set it as the primary for subsequent atomic edits
- `clear_extra()` — remove all non-primary cursors
- `all()` — iterate all cursor positions for bulk operations

### Editing with Multiple Cursors
All edit operations (`InsertChar`, `InsertString`, `Backspace`, `DeleteRange`) iterate cursors in **reverse order** (bottom-to-top in the buffer) so earlier inserts don't invalidate positions of later cursors. Each edit produces its own `Change` in the undo batch.

Motion operations apply to the primary cursor only (other cursors maintain their position).

### Keybindings
- **Ctrl+D** (Normal mode): Find next occurrence of word under cursor. If found, call `add_cursor()` at that position. If not found, do nothing. (Only works when ≥1 extra cursor doesn't already exist on that occurrence.)
- **Esc** (Normal mode, when `cursors.len() > 1`): call `clear_extra()`. Otherwise, normal Esc behavior.

### Edge Cases
- Ctrl+D with no word under cursor (cursor on whitespace): do nothing
- Ctrl+D when no more occurrences exist: do nothing
- Insert mode with multiple cursors: each keystroke inserts at every cursor position
- `dd`, `yy`, `>>`, `<<` with multiple cursors: apply to each cursor's line independently
- Visual mode: not multi-cursor aware initially (visual selection only uses primary cursor)

## 2. System Clipboard

### Dependency
Add `arboard` to `ruster-core/Cargo.toml`. `arboard` is cross-platform and returns `Err` on headless environments (graceful fallback).

### Integration
- `VimState` gains a `Clipboard` abstraction (wrapping `arboard::Clipboard`)
- **Unnamed register** (`""`) aliases to system clipboard. On yank, write to both the in-memory register AND `arboard`. On paste, read from `arboard` (fallback to in-memory on error).
- `VimState.unnamed_register: Option<String>` still exists as the in-memory fallback
- Yank operations (`y`, `d`, `x`, `c`, `s`) all write to system clipboard
- Paste operations (`p`, `P`) all read from system clipboard

### Keybindings (GUI mode)
- **Ctrl+C** → yank selection (Visual mode only, like `y`)
- **Ctrl+V** → paste after cursor (Insert mode: insert at cursor, Normal mode: like `p`)

### Edge Cases
- Clipboard unavailable (SSH, TUI, headless): fallback to in-memory register silently
- Large clipboard contents: no special handling — arboard handles it
- Non-text clipboard content: arboard returns `Err`, fallback to register

## 3. Tabs & Indentation

### Tab Key in Insert Mode
Tab key inserts `EditOp::InsertString(" ".repeat(config.tabstop))` — expands to spaces per `expandtab`. Shift+Tab inserts back when there are leading spaces to remove.

### Indent Operators
New `Action` variants:
- `Action::IndentLine` — insert `shiftwidth` spaces at line start
- `Action::DeindentLine` — remove up to `shiftwidth` leading spaces from line start
- `Action::IndentLines(usize, usize)` — indent range of lines
- `Action::DeindentLines(usize, usize)` — deindent range of lines

Vim operator bindings:
- `>>` → `IndentLine` (current line in Normal mode, selection in Visual mode)
- `<<` → `DeindentLine` (current line in Normal mode, selection in Visual mode)
- `==` → auto-indent (placeholder for now — just `IndentLine`)

### Configuration
Respects existing Config fields:
- `tabstop` (default 4): width of a tab character / indent level
- `shiftwidth` (default = tabstop): width for `>>`/`<<` operators
- `expandtab` (default true): whether Tab inserts spaces

### Edge Cases
- Deindent on a line with fewer spaces than `shiftwidth`: remove all leading spaces
- Deindent on an empty line: do nothing
- Visual mode `>>` on multiple lines: indent each line once
- `2>>` (count prefix): indent current line `count` times

## 4. EditorConfig

### Parser
A small module `ruster-core/src/editorconfig.rs` that parses `.editorconfig` files. The format is INI-like:
```ini
root = true

[*]
indent_style = space
indent_size = 4

[*.rs]
indent_size = 2
```

The parser:
1. Starts at the file's directory, walks up until `root = true` is found or filesystem root
2. Reads each `.editorconfig` along the path
3. Matches file path against `[glob]` sections (most specific pattern wins)
4. Returns a `HashMap<String, String>` of properties

### Supported Properties
- `indent_style` → `tab` sets `expandtab = false`, `space` sets `expandtab = true`
- `indent_size` → `tabstop` (and `shiftwidth` if no `tab_width` set)
- `tab_width` → `tabstop` (and `shiftwidth`)
- `end_of_line` → stored for future use (line ending handling)
- `charset` → stored for future use (encoding handling)
- `trim_trailing_whitespace` → stored for future use
- `insert_final_newline` → stored for future use

### Integration
- On file open: call `editorconfig::parse(file_path)`, merge returned properties into `Config`
- Config merge: EditorConfig values override `ruster.lua` config values (standard EditorConfig behavior)

### Edge Cases
- No `.editorconfig` found: no change to Config
- Invalid syntax in `.editorconfig`: skip that file, log warning, continue
- `root = true` marker: stops parent directory search
- Pattern matching: supports `*`, `**`, `?`, `[seq]`, `{a,b}` globs (simple implementation)
- Symlinks in path: resolved before starting the walk

## Crate Changes Summary

| Crate | Changes |
|-------|---------|
| `ruster-core` | Multi-cursor ops, arboard clipboard, indent/deindent actions, editorconfig parser |
| `ruster-tui` | Ctrl+D binding, Esc multi-cursor clear, Tab/Shift+Tab, `>>`/`<<` operators, EditorConfig call on file open |
| `ruster-render-raylib` | Ctrl+C/Ctrl+V bindings |

No new crates. `arboard` added to `ruster-core`.
