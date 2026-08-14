# Ruster — Core Slice Design (Phase 0 + Phase 1)

**Date:** 2026-07-20
**Status:** Approved (brainstorming complete)
**Scope:** Sub-project 1 of the `AGENTS.md` vision — the bootable, usable editor core. Phases 2–7 (window management, tree-sitter/LSP, embedded terminal, IDE tools, ecosystem, application platform) are explicitly **out of scope** and get their own spec → plan → implementation cycles later.

---

## 1. Context & Acceptance Criteria

`ruster` is a hybrid Neovim/Emacs editor written in Rust, scripted in Lua. This slice delivers the smallest **daily-driver** editor:

> Open ruster (TUI or GUI), open a file, edit it in either Neovim or Emacs mode, toggle modes at runtime, save, quit — with Lua config loading tabstop/editmode/keybindings. Manually verified, on top of a green automated test suite.

### Decisions locked during brainstorming

| Decision | Choice |
| :--- | :--- |
| Scope | Phase 0 + Phase 1 core slice only |
| Renderers | Both ratatui (TUI) and raylib (GUI), behind a `Renderer` trait |
| Lua flavor | Lua 5.4 via `mlua` |
| Editing paradigms | Neovim modal + Emacs modeless, runtime `:set editmode` toggle |
| Undo | Linear undo/redo (undo-tree deferred) |
| Multiple cursors | Cursor-set data model now; multi-cursor commands in Phase 5 |
| Architecture | Cargo workspace, crate per layer |
| Acceptance | Usable daily-driver demo, manually verified |

### Spec corrections to `AGENTS.md` (apply to that doc when convenient)

1. `ropey` is a rope, not a CRDT. Fine choice; the description was wrong. (CRDTs would matter for Phase 7 client-server collaboration — revisit then.)
2. `tachyonfx` is a ratatui *effects* library, not a frame clock. Moved to Phase 6 polish. Each frontend runs its own 60fps tick feeding `Tick` events into the shared event channel.
3. `winit` is dropped for the GUI backend — raylib manages its own window and input.

---

## 2. Architecture: Workspace & Crate Boundaries

```
ruster/
├── Cargo.toml                 # workspace root
└── crates/
    ├── ruster-core/           # Buffer, CursorSet, UndoStack, KeymapEngine,
    │                          # mode state machines, Command enum
    │                          # deps: ropey, thiserror — NO UI, NO Lua
    ├── ruster-render/         # Renderer trait + Cell/Frame/Style types only
    │                          # deps: none (pure types — the seam)
    ├── ruster-tui/            # ratatui + crossterm backend
    ├── ruster-gui/            # raylib backend (fontdue glyph cache)
    ├── ruster-lua/            # mlua (Lua 5.4) bindings over ruster-core
    └── ruster-bin/            # binary: tokio event loop, config loading,
                               # CLI args, frontend selection & wiring
```

**One-way dependency rule:** `core` depends on nothing ruster-internal. `render` is pure types. `tui`/`gui` depend on `render`. `lua` depends on `core`. `bin` depends on all. The binary maps core state → `Frame` each tick; core never sees a renderer.

---

## 3. Core Engine (`ruster-core`)

- **`Buffer`** — wraps `ropey::Rope`. Line/char indexing, grapheme-safe edits (`insert`, `delete`, `replace`). Every op returns a `Change` record — the currency of undo.
- **`CursorSet`** — ordered set of `(anchor, head)` ranges with a designated primary cursor. Overlap-merge rules defined now. All slice commands operate on the primary only; Phase 5 multi-cursor becomes purely additive.
- **`UndoStack`** — linear, change-batched: consecutive inserts group into one undo unit; one operator+motion is one unit. `u` / `Ctrl-r`.
- **`Command` enum** — every editing action as data (`InsertChar`, `Newline`, `Backspace`, `DeleteOp(motion)`, `MoveLeft`, …). Commands are the single entry point: keymaps produce them, Lua can invoke them, tests drive them.
- **Mode state machines** — `VimState` (Normal/Insert/Visual/Command-line) and `EmacsState` (modeless + prefix-arg state) as explicit enums with pure transition functions. A `Paradigm` enum holds whichever is live; `:set editmode` swaps it plus keymap root and statusline indicator.

Everything in this crate is headless: tests feed `KeyEvent`s in and assert buffer text out, with zero terminal involvement.

---

## 4. Keymap Engine & Dual Paradigms

- **Unified `KeyEvent`** — core's own key type; crossterm and raylib events normalize into it at the frontend boundary.
- **Trie-based keymap engine** — key sequence → `Command` or Lua callback. Per-mode map layers (Normal, Insert, Visual, Command-line, Emacs). `timeoutlen` for multi-key sequences. Lua rebinds via `ruster.map(mode, lhs, rhs)`.
- **Neovim mode (slice scope):** Normal/Insert/Visual(char+line)/Command-line. Operators `d y c >` × motions `w b e 0 $ gg G` + counts. Text objects: minimal set `iw aw i" i' i( i{` (the full set needs tree-sitter — Phase 3). Dot-repeat records the last change `Command` and replays it. Command-line: `:w :q :wq :set`.
- **Emacs mode (slice scope):** `C-f C-b C-n C-p M-f M-b C-a C-e` movement, `C-k` kill-line, `C-y` yank, kill-ring as a rotating `Vec<String>`, `C-u` numeric prefix, `C-d` delete-char, minimal `M-x` (`set-editmode`, `write`, `quit`). `C-s`/`C-r` incremental search: minibuffer prompt, live highlight, next/prev.
- **The toggle** — `:set editmode neovim|emacs` (and `M-x set-editmode`) swaps the live `Paradigm`, rebinds the keymap root, updates statusline indicator and minibuffer prompt style. Buffer, undo history, and cursor position are untouched — paradigm is pure input interpretation.

---

## 5. Rendering & Event Loop

- **`Renderer` trait** (`ruster-render`) — `draw(&Frame)`, `size()`, `poll_events()`. A `Frame` is a cell grid (char + fg/bg/attrs) plus cursor position/style. Rendering is a pure function of core state, rebuilt every tick.
- **TUI backend** — ratatui + crossterm; `Frame` maps onto ratatui's buffer (its double-buffer provides dirty-cell diffing).
- **GUI backend** — raylib window; `fontdue` rasterization with a glyph cache; background quads then glyphs, dirty cells only.
- **Event loop (binary)** — one `tokio::select!` over mpsc channels: frontend input events, 60fps tick, Lua callbacks. **macOS constraint:** raylib/GLFW must live on the main thread — in GUI mode raylib owns the main thread and forwards events into the channel while core+Lua run on the tokio runtime; in TUI mode a crossterm reader thread feeds the same channel. Core never knows which frontend is attached.
- **Frontend selection** — `--tui` / `--gui` flag overrides `frontend` in `ruster.toml`; default: TUI when `$SSH_TTY` is set, GUI otherwise.

---

## 6. Lua API & Configuration

- **Config files:** `~/.config/ruster/ruster.toml` (static: `frontend`, `tabstop`, `editmode`, `theme` placeholder) loaded first, then `~/.config/ruster/init.lua` (dynamic), which may override anything.
- **Slice API surface** (deliberately small):
  - `ruster.opt.tabstop = 4` / `ruster.opt.editmode = "emacs"` — live option read/write
  - `ruster.map("normal", "lhs", "rhs"|function)` — keybinding per mode
  - `ruster.command("Name", function)` — callable via `:Name` / `M-x Name`
  - `ruster.buf.lines()` / `ruster.buf.line(n)` — read-only buffer access (write access post-slice; needs `Change`-record integration)
- **Error containment:** a Lua error renders as a message-line notification, never crashes the editor. Sandbox hardening is Phase 6; the slice trusts local config.

---

## 7. Error Handling & Testing

- **Errors:** `thiserror` typed errors in core/render/lua crates; `anyhow` only in the binary. Fatal errors (config unreadable, frontend init failure) print cleanly to stderr before exit; runtime errors go to the message line.
- **Testing (TDD — tests written first per task):**
  - **ruster-core:** unit tests for buffer ops, undo grouping, cursor-set merge rules, keymap trie, both paradigm state machines. Plus **scenario tests**: scripted key sequences → expected buffer text/cursor, headless (e.g. `ciwfoo<Esc>` on known input). These are the regression backbone.
  - **ruster-render:** Frame/dirty-cell unit tests.
  - **ruster-tui:** ratatui `TestBackend` snapshot tests.
  - **ruster-gui:** manually verified (no headless GL in the slice); logic kept thin so tests live in render/core.
  - **ruster-lua:** Lua snippets evaluated against a headless core (remaps, opt changes).
- **Acceptance:** the daily-driver demo, manually verified on macOS, plus `cargo test --workspace` green.

---

## 8. Explicitly Out of Scope (future spec cycles)

Window splits, tabs, Dired/Ibuffer, FZF/Rg, tree-sitter, LSP, embedded terminal, DAP, build/test runners, themes beyond a hardcoded default, plugin manager, Magit, daemon/client-server, undo-tree, multi-cursor commands, visual selections beyond char/line-wise, macros (`q`), `:substitute`, `.editorconfig`.
