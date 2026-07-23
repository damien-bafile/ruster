# Phase 2: Buffer, Window & File Management — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Design spec:** [2026-07-23-phase2-buffers-windows-design.md](../specs/2026-07-23-phase2-buffers-windows-design.md)

**Goal:** Turn ruster from a single-file editor into a workspace: multiple buffers, split
windows, and the file-management / navigation UI (buffer list, file explorer, fuzzy finder),
plus the per-window gutter and extensible statusline.

**Architecture:** A buffer registry (`Document` + `BufferStore`) and a window tree
(`WindowTree`) land in `ruster-core`. `ruster-render` grows from a single-window `EditorState`
to a multi-window `FrameState`. `ruster-tui` and `ruster-render-raylib` render each window into
its own rectangle and draw a shared floating `Picker` overlay reused by ibuffer, dired, fzf/rg,
and which-key. New deps: `nucleo-matcher`, `ignore` (crates); `ripgrep` (external binary).

**Tech Stack:** Rust, ropey (buffer), tree-sitter (syntax), ratatui/crossterm (TUI), raylib
(GUI), mlua (Lua), nucleo-matcher (fuzzy), ignore (file walk).

## Global Constraints

- Cursor and scroll are **per-window**; buffer text and undo history are **per-document**. Two
  windows on the same file have independent cursors.
- `:q`/`Ctrl-w c` closes the active *window*; the app quits only when the last window closes.
- Every list UI (ibuffer, dired file list, `:Files`, `:Rg`, which-key, `:` completion) is built
  on one shared `Picker` primitive — do not fork it per feature.
- Line-number gutter honors existing `Config.number` / `Config.relativenumber`; hybrid =
  absolute on cursor line, relative elsewhere.
- Long-running work (`:Rg`, `:Files` walk) runs off the render thread via the existing
  `AppEvent` channel so the 60fps loop never blocks.
- Keep `docs/config-reference.md` and `docs/lua-api.md` in sync (AGENTS.md mandate).

---

### Task 1: Document + BufferStore (buffer registry)

**Files:**
- Create: `crates/ruster-core/src/document.rs`
- Create: `crates/ruster-core/src/workspace.rs`
- Modify: `crates/ruster-core/src/lib.rs` (module exports)
- Modify: `crates/ruster-core/src/editor.rs` (move `undo`/`indent`/metadata out)

**Interfaces:**
- Produces: `BufferId`, `Document`, `DocKind`, `SpecialKind`, `BufferStore`
- Consumes: existing `Buffer`, `UndoStack`

- [ ] **Step 1:** Add `document.rs` with `BufferId(u32)`, `DocKind`, `SpecialKind`, and
  `Document { buffer, undo, file_path, name, modified, kind, indent }`. Constructors
  `Document::from_file(path, content)`, `Document::scratch(name)`, `Document::special(kind, name)`.
- [ ] **Step 2:** Add `workspace.rs` with `BufferStore` (`open_file`, `create_scratch`,
  `create_special`, `get`/`get_mut`, `close`, `ids`, MRU `order`). `open_file` reuses an existing
  `BufferId` when the path is already open (canonicalize before compare). `close` refuses the
  last remaining modified buffer.
- [ ] **Step 3:** Export both modules from `lib.rs`.
- [ ] **Step 4: Tests** in `workspace.rs`: open two files → two ids; re-open same path → same id;
  create scratch has `DocKind::Scratch`; close nonexistent → false; modified-flag round-trips.
- [ ] **Step 5:** `cargo test -p ruster-core` — all pass.
- [ ] **Step 6:** Commit: `feat: Document + BufferStore buffer registry`

---

### Task 2: WindowTree (splits, focus, fullscreen, geometry)

**Files:**
- Create: `crates/ruster-core/src/windows.rs`
- Modify: `crates/ruster-core/src/lib.rs`
- Modify: `crates/ruster-core/src/editor.rs` — narrow `Editor` to act on borrowed
  `(&mut Buffer, &mut CursorSet, &mut UndoStack)` while preserving the `execute(Action)` surface.

**Interfaces:**
- Produces: `WindowId`, `Window`, `Layout`, `SplitDir`, `FocusDir`, `WindowTree`,
  `WindowTree::compute_rects(area) -> Vec<(WindowId, Rect)>`
- Consumes: `BufferId`, `CursorSet`

- [ ] **Step 1:** Refactor `Editor`: keep `pub fn execute(&mut self, Action)` but split state so
  edits target a document's `Buffer` + a window's `CursorSet` + the document's `UndoStack`.
  Simplest path: make `Editor<'a>` a transient borrow bundle constructed per keystroke from the
  active `(Document, Window)`, or add `Editor::execute_on(buf, cursors, undo, action)` and keep the
  owned form as a thin wrapper for existing single-buffer tests. Preserve all current `editor.rs`
  tests.
- [ ] **Step 2:** Add `windows.rs`: `Window { buffer, cursors, scroll_top }`, `Layout` enum,
  `WindowTree { root, windows, active, next, fullscreen }`. Implement `single`, `split`,
  `close_active`, `focus`, `active`/`active_window[_mut]`, `toggle_fullscreen`.
- [ ] **Step 3:** Implement `compute_rects(area)`: recurse `Layout`, dividing by `ratio` and
  `dir`; when `fullscreen` is `Some`, return just that window at full `area`.
- [ ] **Step 4: Tests:** `single` → one rect == area; one horizontal split → two stacked rects
  covering area, no overlap; vertical split → side-by-side; `focus(Right)` moves active; `close_active`
  on last window → false and tree unchanged; fullscreen returns one full rect and restores exactly.
- [ ] **Step 5:** `cargo test -p ruster-core`.
- [ ] **Step 6:** Commit: `feat: WindowTree with splits, focus, fullscreen geometry`

---

### Task 3: Multi-window App + multi-window render pipeline

**Files:**
- Modify: `crates/ruster-render/src/lib.rs` — replace `EditorState` with `FrameState` /
  `WindowView`; add `GutterView`, `StatuslineView`, `PickerView` (empty placeholders wired in
  later tasks).
- Modify: `crates/ruster-tui/src/app.rs` — replace `editor`/`file_path` with `buffers` +
  `windows`; re-point Lua buffer callbacks at the active window/document; build `FrameState` in
  `render()`.
- Modify: `crates/ruster-tui/src/renderer.rs` — render each `WindowView` into its rect.
- Modify: `crates/ruster-render-raylib/src/lib.rs` — same, for GUI.
- Modify: `crates/ruster-tui/src/widgets.rs` — `BufferWidget`/`StatuslineWidget` take a rect.

**Interfaces:**
- Produces: `FrameState`, `WindowView`
- Consumes: `BufferStore`, `WindowTree`, `SyntaxEngine`

- [ ] **Step 1:** In `ruster-render/src/lib.rs`, add `WindowView` and `FrameState` per the design
  spec; change `Renderer::render_frame(&mut self, &FrameState)`. Keep `StyledLine`, `Color`,
  `CursorKind` as-is.
- [ ] **Step 2:** In `app.rs`, swap fields to `buffers: BufferStore`, `windows: WindowTree`,
  `syntax: HashMap<BufferId, SyntaxEngine>`. Update `App::new` to open the initial file into a
  buffer and a single window. Route `handle_key` edits through the active window/document.
- [ ] **Step 3:** Re-point the four Lua buffer callbacks (`app.rs:117-171`) at the active
  window/document instead of a single `Rc<RefCell<Editor>>`.
- [ ] **Step 4:** Rebuild `render()` to produce a `FrameState` with one `WindowView` per rect from
  `windows.compute_rects(area)`; each view carries that window's styled lines, cursor, and scroll.
- [ ] **Step 5:** Update both renderers to draw each `WindowView` at `view.rect`, then cmdline.
  Draw a 1-column separator between side-by-side windows.
- [ ] **Step 6: Tests:** existing `app.rs` cmd tests still pass (single window). Add: opening a
  second buffer and splitting yields two `WindowView`s; edits in the active window don't affect the
  other buffer.
- [ ] **Step 7:** `cargo test -p ruster-core -p ruster-render -p ruster-tui` and
  `cargo check -p ruster-bin -p ruster-render-raylib`.
- [ ] **Step 8:** Commit: `feat: multi-window app and render pipeline`

---

### Task 4: Split commands & Ctrl-w keybindings

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` — `parse_cmdline` + `handle_key`
- Modify: `crates/ruster-render-raylib/src/lib.rs` — `Ctrl-w` chord routing (GUI)

**Interfaces:**
- Consumes: `WindowTree::{split, close_active, focus, toggle_fullscreen}`

- [ ] **Step 1:** Add cmdline commands: `:split`/`:sp`, `:vsplit`/`:vs`, `:close`/`:clo`,
  `:only`/`:on`, `:fullscreen`. Extend the `CmdAction` enum and `parse_cmdline`
  (`app.rs:442`, `app.rs:64`). `:q` closes the active window; quits only when it was the last.
- [ ] **Step 2:** Add the `Ctrl-w` prefix state machine to `handle_key`: `s`, `v`, `c`, `o`,
  `h/j/k/l`, `z` (fullscreen). Feed the same actions in the raylib backend.
- [ ] **Step 3: Tests:** `:vsplit` → `windows.compute_rects` returns two side-by-side rects; `:q`
  with two windows closes one and does not set `should_quit`; `:q` with one window sets
  `should_quit`; `Ctrl-w z` toggles fullscreen.
- [ ] **Step 4:** `cargo test -p ruster-tui`.
- [ ] **Step 5:** Commit: `feat: window split/close/only commands and Ctrl-w bindings`

---

### Task 5: Gutter (line numbers: absolute / relative / hybrid)

**Files:**
- Modify: `crates/ruster-render/src/lib.rs` — `GutterView { rows: Vec<String>, width: u16 }`
- Modify: `crates/ruster-tui/src/app.rs` — compute gutter per window from config + cursor line
- Modify: `crates/ruster-tui/src/widgets.rs` — draw the gutter column left of buffer text
- Modify: `crates/ruster-render-raylib/src/lib.rs` — draw gutter (GUI)

**Interfaces:**
- Consumes: `Config.number`, `Config.relativenumber`, window cursor line, buffer line count

- [ ] **Step 1:** Add a pure helper `gutter_rows(first_line, line_count, cursor_line, number,
  relativenumber, height) -> GutterView`. Rules: number-only = absolute; relative-only = distance
  from cursor line; both = absolute on cursor line + relative elsewhere; neither = width 0.
  Right-align; width = `max(3, digits(line_count)) + 1`.
- [ ] **Step 2:** Populate `WindowView.gutter` in `render()` for each window (each window uses its
  own cursor line and scroll_top).
- [ ] **Step 3:** Render the gutter column in both backends; buffer text starts after `gutter.width`.
- [ ] **Step 4: Tests** (pure helper): absolute rows `["  1"," 2"…]`; hybrid puts absolute at cursor
  row and `1`,`2` above/below; width scales with line count; disabled → width 0.
- [ ] **Step 5:** `cargo test -p ruster-render -p ruster-tui`.
- [ ] **Step 6:** Commit: `feat: line-number gutter (absolute/relative/hybrid)`

---

### Task 6: Extensible statusline (lualine-style)

**Files:**
- Modify: `crates/ruster-render/src/lib.rs` — `StatuslineView { left, center, right, active }`
- Modify: `crates/ruster-tui/src/widgets.rs` — lay out left/center/right in `StatuslineWidget`
- Modify: `crates/ruster-lua/src/api.rs` — `ruster.statusline.section(pos, fn)`
- Modify: `crates/ruster-tui/src/app.rs` — assemble sections per window (active highlighted)
- Modify: `crates/ruster-render-raylib/src/lib.rs` — draw per-window statusline

**Interfaces:**
- Produces: `ruster.statusline.section`
- Consumes: window state, `Config`, Lua-registered components

- [ ] **Step 1:** Define built-in components: mode, name+modified, filetype, `line:col`, `%`.
  Compose a default left/center/right layout matching a minimal lualine.
- [ ] **Step 2:** Add `ruster.statusline.section(pos, fn)` to the Lua API; store registered
  callbacks in `LuaRuntime` and invoke them when building each window's `StatuslineView`.
- [ ] **Step 3:** Render active window's statusline highlighted, inactive dimmed, in both backends.
- [ ] **Step 4: Tests:** default statusline shows mode + filename + `line:col`; a Lua-registered
  right section string appears in `StatuslineView.right`; active flag set only for active window.
- [ ] **Step 5:** `cargo test -p ruster-tui -p ruster-lua`.
- [ ] **Step 6:** Update `docs/lua-api.md` (`ruster.statusline.section`). Commit:
  `feat: extensible per-window statusline`

---

### Task 7: Picker primitive (shared floating list + fuzzy match)

**Files:**
- Create: `crates/ruster-tui/src/picker.rs` — `PickerState`, `PickerItem`, `PickerAction`
- Modify: `crates/ruster-render/src/lib.rs` — `PickerView` (overlay geometry + rows + selection)
- Modify: `crates/ruster-tui/src/app.rs` — hold `picker: Option<PickerState>`; route keys to it
  when open
- Modify: both renderers — draw the floating picker centered over the frame
- Modify: `crates/ruster-tui/Cargo.toml` — add `nucleo-matcher`

**Interfaces:**
- Produces: `PickerState`, `PickerView`, `PickerAction { OpenBuffer | OpenPath | RunCmd }`

- [ ] **Step 1:** Add `nucleo-matcher` to `ruster-tui`. Implement `PickerState` with `items`,
  `filter`, `selected`, `on_accept`, and `filtered()` (fuzzy-ranked visible items).
- [ ] **Step 2:** When `picker.is_some()`, `handle_key` routes typing → filter, `Ctrl-n/p`/arrows →
  move, `Enter` → dispatch `on_accept`, `Esc` → close.
- [ ] **Step 3:** Add `PickerView` to `FrameState`; render a centered bordered box with the query
  line, filtered rows, and a highlighted selection, in both backends.
- [ ] **Step 4: Tests:** filtering narrows items and re-ranks; `Ctrl-n` wraps selection; accept
  dispatches the right `PickerAction`; empty filter shows all.
- [ ] **Step 5:** `cargo test -p ruster-tui`.
- [ ] **Step 6:** Commit: `feat: shared floating Picker primitive with fuzzy matching`

---

### Task 8: Ibuffer (buffer list)

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` — `:ls`/`:buffers` and `Space b` open a Picker over buffers
- Modify: `crates/ruster-tui/src/picker.rs` — buffer-row formatting

**Interfaces:**
- Consumes: `BufferStore::ids`, `Picker`, `PickerAction::OpenBuffer`

- [ ] **Step 1:** Build a Picker from `BufferStore` (`id  [+] name  filetype`). `Enter` →
  `OpenBuffer(id)` sets the active window's buffer. Bind `:ls`/`:buffers` and leader `Space b`.
- [ ] **Step 2:** Add mark-and-delete: `d` marks, `x` closes marked buffers (refuse modified without
  `!`), respecting `BufferStore::close`.
- [ ] **Step 3: Tests:** opening ibuffer with 3 buffers yields 3 items; accepting switches the active
  window's `buffer`; deleting a clean buffer removes it; a modified buffer is refused.
- [ ] **Step 4:** `cargo test -p ruster-tui`.
- [ ] **Step 5:** Commit: `feat: ibuffer buffer-list picker`

---

### Task 9: Dired (file explorer buffer)

**Files:**
- Create: `crates/ruster-core/src/dired.rs` — directory listing model over `std::fs`
- Modify: `crates/ruster-tui/src/app.rs` — `:Dired [path]` / `-` opens a Special buffer; key actions

**Interfaces:**
- Produces: `dired::list(path) -> Vec<DirEntry>` (dirs first, sorted; `..` first)
- Consumes: `BufferStore::create_special(SpecialKind::Dired)`

- [ ] **Step 1:** `dired::list` returns entries (name, is_dir, size) — dirs first, then files,
  each alphabetical; prepend `..` unless at filesystem root. Render into a Special buffer's text.
- [ ] **Step 2:** In dired buffers, map keys: `Enter` opens file / descends dir; `^` or `-` goes up;
  `R` rename, `D` delete (confirm via cmdline `y/n`), `+` create file, `%` create dir; each mutates
  the fs then re-lists.
- [ ] **Step 3: Tests** (core): `list` on a temp dir with 2 files + 1 subdir orders subdir first and
  includes `..`; `list` at root omits `..`. (App-level: `Enter` on a dir re-lists into the subdir.)
- [ ] **Step 4:** `cargo test -p ruster-core -p ruster-tui`.
- [ ] **Step 5:** Commit: `feat: dired file explorer buffer`

---

### Task 10: FZF (`:Files`) & Ripgrep (`:Rg`)

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` — `:Files`/`Space f`, `:Rg`/`Space g`; consume results via
  `AppEvent`
- Modify: `crates/ruster-tui/Cargo.toml` — add `ignore`

**Interfaces:**
- Consumes: `ignore::WalkBuilder` (files), `std::process::Command` (`rg --vimgrep`), `AppEvent`
  channel (`app.rs:72`, `app.rs:297`), `Picker`

- [ ] **Step 1:** `:Files`/`Space f` — spawn a background thread doing a gitignore-aware
  `ignore::WalkBuilder` walk from project root; send collected paths over the `AppEvent` channel;
  populate a Picker. `Enter` → `OpenPath` opens the file in the active window.
- [ ] **Step 2:** `:Rg <pattern>`/`Space g` — spawn `rg --vimgrep <pattern>` via `Command` in a
  thread; stream lines to the channel into a Picker (`file:line:col: text`). `Enter` opens the file
  and moves the cursor to that line:col. If `rg` is missing, show a clear cmdline error.
- [ ] **Step 3: Tests:** parse an `rg --vimgrep` line into `(path, line, col, text)`; missing-`rg`
  path yields the error message (test the parser and the not-found branch, not a live `rg` run).
- [ ] **Step 4:** `cargo test -p ruster-tui`.
- [ ] **Step 5:** Commit: `feat: :Files fuzzy finder and :Rg live grep`

---

### Task 11: Mini-buffer completion & which-key

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` — `:`+Tab command completion via Picker; pending-key timer
- Modify: `crates/ruster-lua/src/keymap.rs` — expose pending continuations + descriptions
- Modify: `crates/ruster-tui/src/widgets.rs`/renderers — which-key panel

**Interfaces:**
- Consumes: keymap tree (`ruster-lua/src/keymap.rs`), `Config.timeoutlen` (new), `Picker`/panel

- [ ] **Step 1:** Add `Config.timeoutlen` (default 300). Add a keymap query returning the
  continuations available from the current pending prefix node with their descriptions.
- [ ] **Step 2:** When a prefix (`Ctrl-w`, `Space`, `g`) is pending longer than `timeoutlen`, show a
  non-modal which-key panel listing `key → description`. Any matching key dismisses it and proceeds.
- [ ] **Step 3:** `:` + Tab opens a Picker of command names matching the current token; `Enter`
  fills the cmdline.
- [ ] **Step 4: Tests:** keymap query returns the right continuations for a known prefix; `timeoutlen`
  read from config; command-completion filters the command list by prefix.
- [ ] **Step 5:** `cargo test -p ruster-tui -p ruster-lua`.
- [ ] **Step 6:** Commit: `feat: mini-buffer command completion and which-key popup`

---

### Task 12: Lua window/buffer API + docs sync

**Files:**
- Modify: `crates/ruster-lua/src/api.rs` — window/buffer functions
- Modify: `crates/ruster-tui/src/app.rs` — wire callbacks to `BufferStore`/`WindowTree`
- Modify: `docs/lua-api.md`, `docs/config-reference.md`

**Interfaces:**
- Produces: `nvim_list_bufs`, `nvim_open_win`, `nvim_win_close`, `nvim_set_current_win`,
  `nvim_list_wins`, `nvim_win_get_buf`, `nvim_win_set_buf`

- [ ] **Step 1:** Add the window/buffer functions to `ruster.api`, backed by callbacks into
  `BufferStore`/`WindowTree` (mirroring the existing buffer-callback wiring in `app.rs:117-171`).
- [ ] **Step 2: Tests** (api.rs, no-callback safe defaults like the existing `nvim_buf_*` tests):
  `nvim_list_bufs` returns registered ids; `nvim_open_win` splits and returns a win id.
- [ ] **Step 3:** Update `docs/lua-api.md` (all new `ruster.api.*` + `ruster.statusline.section`) and
  `docs/config-reference.md` (`number`, `relativenumber` gutter behavior, `timeoutlen`, leader).
- [ ] **Step 4:** `cargo test -p ruster-lua`.
- [ ] **Step 5:** Commit: `feat: Lua window/buffer API and Phase 2 docs`

---

### Final Verification

- [ ] Full test suite: `cargo test -p ruster-core -p ruster-render -p ruster-syntax -p ruster-tui -p ruster-lua -p ruster-render-raylib`
- [ ] Build all: `cargo check -p ruster-bin -p ruster-tui -p ruster-render-raylib`
- [ ] Manual smoke (TUI): open a file, `:vsplit`, `Ctrl-w l`, edit one pane, confirm the other is
  unaffected; `Space b` switches buffers; `:Dired` browses; `:Files`/`:Rg` open results; gutter shows
  hybrid numbers; statusline highlights the active window; `Ctrl-w z` toggles fullscreen.
- [ ] `docs/config-reference.md` and `docs/lua-api.md` reflect every new setting and API.
- [ ] Expected: all tests pass, no new warnings.
