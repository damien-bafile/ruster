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

## Status (updated 2026-07-23)

**Tasks 1–12 implemented and committed** on branch `phase2-buffers-windows`
(commits `174dbc3` → `e9af8ae`). 175 tests pass; the workspace builds with no new
warnings. Checkbox legend below: `[x]` done, `[~]` partially done (see note),
`[ ]` not done.

**Follow-up work — all resolved:**
- **GUI (raylib) rendering** — ✅ **done in Task 13**: renders all split windows at
  their rects with per-window gutter, statusline, independent scroll, and overlays.
- **`:Rg` / `:Files`** — ✅ **done in Task 14**: streamed off a background thread.
- **Dired mutations** — ✅ **done in Task 15**: create/rename/delete.
- **which-key timing** — ✅ **done in Task 16**: gated by `timeoutlen`.
- Only the two **manual visual checks** remain (TUI smoke + GUI layout), which need a
  human at a display; the GUI has been spot-checked via screenshots.

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

- [x] **Step 1:** Add `document.rs` with `BufferId(u32)`, `DocKind`, `SpecialKind`, and
  `Document { buffer, undo, file_path, name, modified, kind, indent }`. Constructors
  `Document::from_file(path, content)`, `Document::scratch(name)`, `Document::special(kind, name)`.
- [x] **Step 2:** Add `workspace.rs` with `BufferStore` (`open_file`, `create_scratch`,
  `create_special`, `get`/`get_mut`, `close`, `ids`, MRU `order`). `open_file` reuses an existing
  `BufferId` when the path is already open (canonicalize before compare). `close` refuses the
  last remaining modified buffer.
- [x] **Step 3:** Export both modules from `lib.rs`.
- [x] **Step 4: Tests** in `workspace.rs`: open two files → two ids; re-open same path → same id;
  create scratch has `DocKind::Scratch`; close nonexistent → false; modified-flag round-trips.
- [x] **Step 5:** `cargo test -p ruster-core` — all pass.
- [x] **Step 6:** Commit: `feat: Document + BufferStore buffer registry`

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

- [x] **Step 1:** Refactor `Editor`: keep `pub fn execute(&mut self, Action)` but split state so
  edits target a document's `Buffer` + a window's `CursorSet` + the document's `UndoStack`.
  Simplest path: make `Editor<'a>` a transient borrow bundle constructed per keystroke from the
  active `(Document, Window)`, or add `Editor::execute_on(buf, cursors, undo, action)` and keep the
  owned form as a thin wrapper for existing single-buffer tests. Preserve all current `editor.rs`
  tests.
- [x] **Step 2:** Add `windows.rs`: `Window { buffer, cursors, scroll_top }`, `Layout` enum,
  `WindowTree { root, windows, active, next, fullscreen }`. Implement `single`, `split`,
  `close_active`, `focus`, `active`/`active_window[_mut]`, `toggle_fullscreen`.
- [x] **Step 3:** Implement `compute_rects(area)`: recurse `Layout`, dividing by `ratio` and
  `dir`; when `fullscreen` is `Some`, return just that window at full `area`.
- [x] **Step 4: Tests:** `single` → one rect == area; one horizontal split → two stacked rects
  covering area, no overlap; vertical split → side-by-side; `focus(Right)` moves active; `close_active`
  on last window → false and tree unchanged; fullscreen returns one full rect and restores exactly.
- [x] **Step 5:** `cargo test -p ruster-core`.
- [x] **Step 6:** Commit: `feat: WindowTree with splits, focus, fullscreen geometry`

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

- [x] **Step 1:** In `ruster-render/src/lib.rs`, add `WindowView` and `FrameState` per the design
  spec; change `Renderer::render_frame(&mut self, &FrameState)`. Keep `StyledLine`, `Color`,
  `CursorKind` as-is.
- [x] **Step 2:** In `app.rs`, swap fields to `buffers: BufferStore`, `windows: WindowTree`,
  `syntax: HashMap<BufferId, SyntaxEngine>`. Update `App::new` to open the initial file into a
  buffer and a single window. Route `handle_key` edits through the active window/document.
- [x] **Step 3:** Re-point the four Lua buffer callbacks (`app.rs:117-171`) at the active
  window/document instead of a single `Rc<RefCell<Editor>>`.
- [x] **Step 4:** Rebuild `render()` to produce a `FrameState` with one `WindowView` per rect from
  `windows.compute_rects(area)`; each view carries that window's styled lines, cursor, and scroll.
- [x] **Step 5:** Update both renderers to draw each `WindowView` at `view.rect`, then cmdline.
  **TUI + GUI done (GUI multi-window rendering completed in Task 13).**
  Draw a 1-column separator between side-by-side windows.
- [x] **Step 6: Tests:** existing `app.rs` cmd tests still pass (single window). Add: opening a
  second buffer and splitting yields two `WindowView`s; edits in the active window don't affect the
  other buffer.
- [x] **Step 7:** `cargo test -p ruster-core -p ruster-render -p ruster-tui` and
  `cargo check -p ruster-bin -p ruster-render-raylib`.
- [x] **Step 8:** Commit: `feat: multi-window app and render pipeline`

---

### Task 4: Split commands & Ctrl-w keybindings

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` — `parse_cmdline` + `handle_key`
- Modify: `crates/ruster-render-raylib/src/lib.rs` — `Ctrl-w` chord routing (GUI)

**Interfaces:**
- Consumes: `WindowTree::{split, close_active, focus, toggle_fullscreen}`

- [x] **Step 1:** Add cmdline commands: `:split`/`:sp`, `:vsplit`/`:vs`, `:close`/`:clo`,
  `:only`/`:on`, `:fullscreen`. Extend the `CmdAction` enum and `parse_cmdline`
  (`app.rs:442`, `app.rs:64`). `:q` closes the active window; quits only when it was the last.
- [x] **Step 2:** Add the `Ctrl-w` prefix state machine to `handle_key`: `s`, `v`, `c`, `o`,
  `h/j/k/l`, `z` (fullscreen). Feed the same actions in the raylib backend.
- [x] **Step 3: Tests:** `:vsplit` → `windows.compute_rects` returns two side-by-side rects; `:q`
  with two windows closes one and does not set `should_quit`; `:q` with one window sets
  `should_quit`; `Ctrl-w z` toggles fullscreen.
- [x] **Step 4:** `cargo test -p ruster-tui`.
- [x] **Step 5:** Commit: `feat: window split/close/only commands and Ctrl-w bindings`

---

### Task 5: Gutter (line numbers: absolute / relative / hybrid)

**Files:**
- Modify: `crates/ruster-render/src/lib.rs` — `GutterView { rows: Vec<String>, width: u16 }`
- Modify: `crates/ruster-tui/src/app.rs` — compute gutter per window from config + cursor line
- Modify: `crates/ruster-tui/src/widgets.rs` — draw the gutter column left of buffer text
- Modify: `crates/ruster-render-raylib/src/lib.rs` — draw gutter (GUI)

**Interfaces:**
- Consumes: `Config.number`, `Config.relativenumber`, window cursor line, buffer line count

- [x] **Step 1:** Add a pure helper `gutter_rows(first_line, line_count, cursor_line, number,
  relativenumber, height) -> GutterView`. Rules: number-only = absolute; relative-only = distance
  from cursor line; both = absolute on cursor line + relative elsewhere; neither = width 0.
  Right-align; width = `max(3, digits(line_count)) + 1`.
- [x] **Step 2:** Populate `WindowView.gutter` in `render()` for each window (each window uses its
  own cursor line and scroll_top).
- [x] **Step 3:** Render the gutter column in both backends; buffer text starts after `gutter.width`.
  **TUI + GUI done (GUI gutter completed in Task 13).**
- [x] **Step 4: Tests** (pure helper): absolute rows `["  1"," 2"…]`; hybrid puts absolute at cursor
  row and `1`,`2` above/below; width scales with line count; disabled → width 0.
- [x] **Step 5:** `cargo test -p ruster-render -p ruster-tui`.
- [x] **Step 6:** Commit: `feat: line-number gutter (absolute/relative/hybrid)`

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

- [x] **Step 1:** Define built-in components: mode, name+modified, filetype, `line:col`, `%`.
  Compose a default left/center/right layout matching a minimal lualine.
- [x] **Step 2:** Add `ruster.statusline.section(pos, fn)` to the Lua API; store registered
  callbacks in `LuaRuntime` and invoke them when building each window's `StatuslineView`.
- [x] **Step 3:** Render active window's statusline highlighted, inactive dimmed, in both backends.
  **TUI + GUI done (per-window GUI statuslines completed in Task 13).**
- [x] **Step 4: Tests:** default statusline shows mode + filename + `line:col`; a Lua-registered
  right section string appears in `StatuslineView.right`; active flag set only for active window.
- [x] **Step 5:** `cargo test -p ruster-tui -p ruster-lua`.
- [x] **Step 6:** Update `docs/lua-api.md` (`ruster.statusline.section`). Commit:
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

- [x] **Step 1:** Add `nucleo-matcher` to `ruster-tui`. Implement `PickerState` with `items`,
  `filter`, `selected`, `on_accept`, and `filtered()` (fuzzy-ranked visible items).
- [x] **Step 2:** When `picker.is_some()`, `handle_key` routes typing → filter, `Ctrl-n/p`/arrows →
  move, `Enter` → dispatch `on_accept`, `Esc` → close.
- [x] **Step 3:** Add `PickerView` to `FrameState`; render a centered bordered box with the query
  line, filtered rows, and a highlighted selection, in both backends.
  **TUI + GUI done (GUI picker overlay completed in Task 13).**
- [x] **Step 4: Tests:** filtering narrows items and re-ranks; `Ctrl-n` wraps selection; accept
  dispatches the right `PickerAction`; empty filter shows all.
- [x] **Step 5:** `cargo test -p ruster-tui`.
- [x] **Step 6:** Commit: `feat: shared floating Picker primitive with fuzzy matching`

---

### Task 8: Ibuffer (buffer list)

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` — `:ls`/`:buffers` and `Space b` open a Picker over buffers
- Modify: `crates/ruster-tui/src/picker.rs` — buffer-row formatting

**Interfaces:**
- Consumes: `BufferStore::ids`, `Picker`, `PickerAction::OpenBuffer`

- [x] **Step 1:** Build a Picker from `BufferStore` (`id  [+] name  filetype`). `Enter` →
  `OpenBuffer(id)` sets the active window's buffer. Bind `:ls`/`:buffers` and leader `Space b`.
- [x] **Step 2:** Add mark-and-delete: `d` marks, `x` closes marked buffers (refuse modified without
  `!`), respecting `BufferStore::close`.
- [x] **Step 3: Tests:** opening ibuffer with 3 buffers yields 3 items; accepting switches the active
  window's `buffer`; deleting a clean buffer removes it; a modified buffer is refused.
- [x] **Step 4:** `cargo test -p ruster-tui`.
- [x] **Step 5:** Commit: `feat: ibuffer buffer-list picker`

---

### Task 9: Dired (file explorer buffer)

**Files:**
- Create: `crates/ruster-core/src/dired.rs` — directory listing model over `std::fs`
- Modify: `crates/ruster-tui/src/app.rs` — `:Dired [path]` / `-` opens a Special buffer; key actions

**Interfaces:**
- Produces: `dired::list(path) -> Vec<DirEntry>` (dirs first, sorted; `..` first)
- Consumes: `BufferStore::create_special(SpecialKind::Dired)`

- [x] **Step 1:** `dired::list` returns entries (name, is_dir, size) — dirs first, then files,
  each alphabetical; prepend `..` unless at filesystem root. Render into a Special buffer's text.
- [x] **Step 2:** In dired buffers, map keys: `Enter` opens file / descends dir; `^` or `-` goes up;
  `R` rename, `D` delete (confirm via cmdline `y/n`), `+` create file, `%` create dir; each mutates
  the fs then re-lists.
- [x] **Step 3: Tests** (core): `list` on a temp dir with 2 files + 1 subdir orders subdir first and
  includes `..`; `list` at root omits `..`. (App-level: `Enter` on a dir re-lists into the subdir.)
- [x] **Step 4:** `cargo test -p ruster-core -p ruster-tui`.
- [x] **Step 5:** Commit: `feat: dired file explorer buffer`

---

### Task 10: FZF (`:Files`) & Ripgrep (`:Rg`)

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` — `:Files`/`Space f`, `:Rg`/`Space g`; consume results via
  `AppEvent`
- Modify: `crates/ruster-tui/Cargo.toml` — add `ignore`

**Interfaces:**
- Consumes: `ignore::WalkBuilder` (files), `std::process::Command` (`rg --vimgrep`), `AppEvent`
  channel (`app.rs:72`, `app.rs:297`), `Picker`

- [x] **Step 1:** `:Files`/`Space f` — spawn a background thread doing a gitignore-aware
  `ignore::WalkBuilder` walk from project root; send collected paths over the `AppEvent` channel;
  populate a Picker. `Enter` → `OpenPath` opens the file in the active window.
- [x] **Step 2:** `:Rg <pattern>`/`Space g` — spawn `rg --vimgrep <pattern>` via `Command` in a
  thread; stream lines to the channel into a Picker (`file:line:col: text`). `Enter` opens the file
  and moves the cursor to that line:col. If `rg` is missing, show a clear cmdline error.
- [x] **Step 3: Tests:** parse an `rg --vimgrep` line into `(path, line, col, text)`; missing-`rg`
  path yields the error message (test the parser and the not-found branch, not a live `rg` run).
- [x] **Step 4:** `cargo test -p ruster-tui`.
- [x] **Step 5:** Commit: `feat: :Files fuzzy finder and :Rg live grep`

---

### Task 11: Mini-buffer completion & which-key

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` — `:`+Tab command completion via Picker; pending-key timer
- Modify: `crates/ruster-lua/src/keymap.rs` — expose pending continuations + descriptions
- Modify: `crates/ruster-tui/src/widgets.rs`/renderers — which-key panel

**Interfaces:**
- Consumes: keymap tree (`ruster-lua/src/keymap.rs`), `Config.timeoutlen` (new), `Picker`/panel

- [x] **Step 1:** Add `Config.timeoutlen` (default 300). Add a keymap query returning the
  continuations available from the current pending prefix node with their descriptions.
- [x] **Step 2:** When a prefix (`Ctrl-w`, `Space`, `g`) is pending longer than `timeoutlen`, show a
  non-modal which-key panel listing `key → description`. Any matching key dismisses it and proceeds.
- [x] **Step 3:** `:` + Tab opens a Picker of command names matching the current token; `Enter`
  fills the cmdline.
- [x] **Step 4: Tests:** keymap query returns the right continuations for a known prefix; `timeoutlen`
  read from config; command-completion filters the command list by prefix.
- [x] **Step 5:** `cargo test -p ruster-tui -p ruster-lua`.
- [x] **Step 6:** Commit: `feat: mini-buffer command completion and which-key popup`

---

### Task 12: Lua window/buffer API + docs sync

**Files:**
- Modify: `crates/ruster-lua/src/api.rs` — window/buffer functions
- Modify: `crates/ruster-tui/src/app.rs` — wire callbacks to `BufferStore`/`WindowTree`
- Modify: `docs/lua-api.md`, `docs/config-reference.md`

**Interfaces:**
- Produces: `nvim_list_bufs`, `nvim_open_win`, `nvim_win_close`, `nvim_set_current_win`,
  `nvim_list_wins`, `nvim_win_get_buf`, `nvim_win_set_buf`

- [x] **Step 1:** Add the window/buffer functions to `ruster.api`, backed by callbacks into
  `BufferStore`/`WindowTree` (mirroring the existing buffer-callback wiring in `app.rs:117-171`).
- [x] **Step 2: Tests** (api.rs, no-callback safe defaults like the existing `nvim_buf_*` tests):
  `nvim_list_bufs` returns registered ids; `nvim_open_win` splits and returns a win id.
- [x] **Step 3:** Update `docs/lua-api.md` (all new `ruster.api.*` + `ruster.statusline.section`) and
  `docs/config-reference.md` (`number`, `relativenumber` gutter behavior, `timeoutlen`, leader).
- [x] **Step 4:** `cargo test -p ruster-lua`.
- [x] **Step 5:** Commit: `feat: Lua window/buffer API and Phase 2 docs`

---

### Final Verification

- [x] Full test suite: `cargo test -p ruster-core -p ruster-render -p ruster-syntax -p ruster-tui -p ruster-lua -p ruster-render-raylib`
- [x] Build all: `cargo check -p ruster-bin -p ruster-tui -p ruster-render-raylib`
- [x] Manual smoke (TUI). Deferred at the time; run 2026-08-07 with the Phase 10
  harness. `:vsplit`, `Ctrl-w l` and `Ctrl-w z` are covered by
  `drive.rs::splits_window_nav_and_fullscreen_round_trip` and
  `ctrl_w_v_splits_the_window` — driven through the real frame loop rather than
  eyeballed. `:Dired`, the gutter and the statusline are captured in
  `docs/verification/{dired,gutter,statusline}-tui.txt`; the buffer list in
  `ibuffer-tui.txt`. `:Rg` and per-pane edit isolation were not driven.
- [x] `docs/config-reference.md` and `docs/lua-api.md` reflect every new setting and API.
- [x] Expected: all tests pass, no new warnings.

---

## Follow-up tasks (deferrals from Tasks 1–12)

These capture work the original tasks specified but that was not completed in the
first pass. They are additive; the Phase 2 core (buffers, windows, splits, gutter,
statusline, pickers, dired, fzf/rg, which-key, Lua API) is functional without them.

---

### Task 13: GUI (raylib) multi-window, gutter & picker rendering — DONE (commit `3d41a31`)

**Why:** `ruster-render-raylib` previously rendered only the active `WindowView`
full-screen and ignored `view.rect`, `view.gutter`, and `state.picker`. The TUI
backend renders all of these; the GUI was effectively a single-window view, and an
open picker/which-key panel was invisible even though it captured input. All
required data was already in `FrameState` — no app or core changes were needed.

**Files:** Modified `crates/ruster-render-raylib/src/lib.rs`.

- [x] **Step 1:** Loop over `state.windows`, drawing each at its `view.rect`
  converted from cells to pixels (`x*char_w`, `y*LINE_H`).
- [x] **Step 2:** Use each window's own `view.scroll_offset` instead of recomputing
  a single global scroll, so split panes scroll independently.
- [x] **Step 3:** Draw `view.gutter` (the pre-formatted rows) in a left column and
  offset buffer text by `gutter.width`, mirroring `BufferWidget`.
- [x] **Step 4:** Draw each window's `view.statusline` at the bottom of its rect
  (active highlighted, inactive dimmed) and a divider between side-by-side panes.
- [x] **Step 5:** Draw `state.picker` as a centered overlay (title, query, rows,
  selected highlight), mirroring `PickerWidget`/`renderer.rs`.
- [x] **Step 6:** Manual visual check of the GUI pixel layout. Blocked on a
  display at the time; done 2026-08-07 — `docs/verification/*-gui.png` is 32
  screenshots of the real raylib window, one per surface, produced by
  `just verify` and reviewed.
- [x] **Step 7:** Commit: `feat: GUI multi-window, gutter, and picker rendering`

---

### Task 14: Async `:Rg` / `:Files` — DONE (commit `45843a4`)

**Why:** `run_rg`/`open_files_picker` (`crates/ruster-tui/src/app.rs`) ran
synchronously on the UI thread; a large repo blocked the render loop.

**Files:** Modified `crates/ruster-tui/src/app.rs`, `crates/ruster-tui/src/picker.rs`.

- [x] **Step 1:** Stream results over a `std::sync::mpsc` channel held on `App`
  (`pending_results`), drained into the open picker each frame by
  `drain_pending_results` — backend-agnostic since `render` runs every frame in
  both loops (chosen over `AppEvent`, which only the TUI async loop drains).
- [x] **Step 2:** Spawn the `ignore` walk / `rg` process on a background thread
  that sends `PickerItem`s; `PickerState::push_item` appends them live.
- [x] **Step 3:** Closing the picker drops the receiver, ending the worker on its
  next send.
- [x] **Step 4:** Commit: `feat: stream :Files/:Rg off-thread; delay which-key by timeoutlen`

---

### Task 15: Dired file mutations — DONE (commit `102763c`)

**Why:** Dired supported open/descend/up only; the design listed create/rename/delete.

**Files:** Modified `crates/ruster-tui/src/app.rs` (dired key handling + prompt).

- [x] **Step 1:** `+` create file, `%` create dir (name entered in a mini-buffer prompt).
- [x] **Step 2:** `R` rename, `D` delete the entry under the cursor, delete with a
  `y/n` confirmation; the listing reloads afterward.
- [x] **Step 3:** Test for create + delete against a temp dir.
- [x] **Step 4:** Commit: `feat: dired create/rename/delete`

---

### Task 16: `timeoutlen`-based which-key timing — DONE (commit `45843a4`)

**Why:** The which-key panel appeared immediately on a pending prefix;
`Config.timeoutlen` should gate it so it only pops after the delay.

**Files:** Modified `crates/ruster-tui/src/app.rs`.

- [x] **Step 1:** Record `leader_since` when the leader starts; in `render`, only
  target the panel visible once `timeoutlen` has elapsed (and keep it up once it
  has begun appearing). Fast sequences never flash the panel.
- [x] **Step 2:** Commit: `feat: delay which-key by timeoutlen`. Shipped — the
  panel is gated on `whichkey.timeoutlen` (`app.rs`, `show` requires
  `past_timeout`), with a unit test asserting it stays hidden before the
  timeout. The box was simply never ticked.
