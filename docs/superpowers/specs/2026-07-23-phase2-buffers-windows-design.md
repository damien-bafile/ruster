# Phase 2: Buffer, Window & File Management — Design

## Overview

Phase 2 moves ruster from a single-file editor to a **workspace**: many buffers open at
once, viewed through a tree of split windows, with the file-management and navigation UI
(buffer list, file explorer, fuzzy finder) needed to move between them. It also lands the
two per-window chrome features that Phase 1 left stubbed: the **gutter** (line numbers) and
an extensible **statusline**.

The single biggest change is architectural: today `App` owns exactly one
`Rc<RefCell<Editor>>` and one `file_path` (`crates/ruster-tui/src/app.rs:76-90`). Everything
in this phase depends on replacing that with a **buffer registry** and a **window tree**.
Get that foundation right and the remaining features are largely UI on top of it.

Scope is the Phase 2 row of `AGENTS.md`: Window Splits, Mini-buffer/which-key, Toggle
Fullscreen, Ibuffer, Dired, FZF/Ripgrep, Gutter, Statusline.

## Core Data Model

### Document (buffer) — `ruster-core/src/document.rs` (new)

A *document* is what vim calls a "buffer": the text plus its file identity and undo history.
It does **not** own a cursor or scroll position — those are per-window.

```rust
pub struct BufferId(pub u32);

pub struct Document {
    pub buffer: Buffer,          // rope (existing ruster-core::Buffer)
    pub undo: UndoStack,         // moved off Editor; undo is buffer-global
    pub file_path: Option<PathBuf>,
    pub name: String,            // display name (file name, "[No Name]", "*ibuffer*"…)
    pub modified: bool,
    pub kind: DocKind,           // File | Scratch | Special(SpecialKind)
    pub indent: String,          // buffer-local (from EditorConfig / config)
}

pub enum DocKind { File, Scratch, Special(SpecialKind) }
pub enum SpecialKind { Ibuffer, Dired, Picker }
```

### BufferStore — `ruster-core/src/workspace.rs` (new)

```rust
pub struct BufferStore {
    docs: HashMap<BufferId, Document>,
    order: Vec<BufferId>,   // MRU / creation order for :bnext, ibuffer
    next: u32,
}
impl BufferStore {
    pub fn open_file(&mut self, path: PathBuf) -> BufferId;   // reuse if already open
    pub fn create_scratch(&mut self, name: &str) -> BufferId;
    pub fn get(&self, id: BufferId) -> Option<&Document>;
    pub fn get_mut(&mut self, id: BufferId) -> Option<&mut Document>;
    pub fn close(&mut self, id: BufferId) -> bool;           // refuse if last & modified
    pub fn ids(&self) -> &[BufferId];
}
```

### Editing session — refactor of `Editor`

`Editor` currently bundles `buffer + cursors + undo + indent`
(`crates/ruster-core/src/editor.rs:6-11`). We keep `Editor` as the **editing session** but
narrow it: it operates on a `Document`'s buffer using a `CursorSet` that belongs to the
active window. Concretely, `Editor::execute` takes the buffer and cursor set it should act
on rather than owning both. The public `execute(Action)` surface is preserved; internals are
rewired so the same actions can target `(&mut Buffer, &mut CursorSet, &mut UndoStack)`.

**Design decision — cursor ownership:** the cursor and scroll live on the *window*, not the
document. Two windows showing the same file therefore have independent cursors and scroll,
which is the correct vim behavior. Undo history is shared per-document.

### Window tree — `ruster-core/src/windows.rs` (new)

A binary split tree; leaves are windows, internal nodes are splits.

```rust
pub struct WindowId(pub u32);

pub struct Window {
    pub buffer: BufferId,
    pub cursors: CursorSet,   // per-window
    pub scroll_top: usize,    // first visible line
}

pub enum Layout {
    Leaf(WindowId),
    Split { dir: SplitDir, ratio: f32, first: Box<Layout>, second: Box<Layout> },
}
pub enum SplitDir { Horizontal, Vertical }  // Horizontal = stacked, Vertical = side-by-side

pub struct WindowTree {
    root: Layout,
    windows: HashMap<WindowId, Window>,
    active: WindowId,
    next: u32,
    fullscreen: Option<WindowId>,   // when Some, render only this window
}
impl WindowTree {
    pub fn single(buffer: BufferId) -> Self;
    pub fn split(&mut self, dir: SplitDir) -> WindowId;   // split active, focus new
    pub fn close_active(&mut self) -> bool;               // false if it is the last window
    pub fn focus(&mut self, dir: FocusDir);               // Ctrl-w h/j/k/l
    pub fn active(&self) -> WindowId;
    pub fn active_window(&self) -> &Window;
    pub fn active_window_mut(&mut self) -> &mut Window;
    pub fn toggle_fullscreen(&mut self);
    /// Compute pixel/cell rectangles for every visible leaf given a total area.
    pub fn compute_rects(&self, area: Rect) -> Vec<(WindowId, Rect)>;
}
```

`compute_rects` is pure geometry (no rendering), so it is unit-testable without a terminal.

## App integration — `ruster-tui/src/app.rs`

`App` loses `editor` and `file_path`, gains:

```rust
pub struct App {
    pub buffers: BufferStore,
    pub windows: WindowTree,
    pub vim: VimState,
    pub renderer: Box<dyn Renderer>,
    // syntax cache keyed by BufferId, lua, config, timer, cursor_anim … as before
    syntax: HashMap<BufferId, SyntaxEngine>,
    picker: Option<PickerState>,   // active floating list (ibuffer/dired/fzf/which-key)
    ...
}
```

Key routing: `handle_key` dispatches to the **active window**'s buffer via `Editor` acting on
`(document.buffer, window.cursors)`. The existing Lua buffer callbacks
(`app.rs:117-171`) are re-pointed at the active window/document instead of the single editor.

## Rendering — `ruster-render` + backends

`EditorState` today describes one window (`crates/ruster-render/src/lib.rs:30-42`). Generalize
to a **frame of windows**:

```rust
pub struct WindowView<'a> {
    pub rect: Rect,               // where to draw
    pub lines: Vec<StyledLine>,
    pub cursor: (u16, u16),
    pub scroll_offset: u16,
    pub gutter: GutterView,       // rendered line-number column
    pub statusline: StatuslineView,
    pub active: bool,
}
pub struct FrameState<'a> {
    pub windows: Vec<WindowView<'a>>,
    pub cmdline: Option<&'a str>,
    pub message: Option<&'a str>,
    pub picker: Option<PickerView<'a>>,   // floating overlay
}
```

Both the ratatui renderer (`ruster-tui/src/renderer.rs`) and the raylib renderer
(`ruster-render-raylib/src/lib.rs:93`) render each `WindowView` into its `rect`, then draw the
cmdline and any floating picker on top. The old single-window `EditorState` is replaced;
`render_frame` takes `FrameState`.

### Gutter (line numbers)

`Config.number` / `Config.relativenumber` already exist (`ruster-lua/src/config.rs:6-7`) but
are not rendered. A `GutterView` is computed per window:

- `number && !relativenumber` → absolute line numbers, right-aligned
- `relativenumber && !number` → distance from the window's cursor line
- both → **hybrid**: absolute on the cursor line, relative elsewhere
- neither → no gutter column
- width = `max(3, digits(line_count)) + 1` padding; signs column reserved for Phase 6 gitsigns

### Statusline (lualine-style)

Each window renders its own statusline; the active window's is highlighted. Content is
assembled from **components** so Lua can extend it. Built-in components: mode, file name +
modified flag, filetype, cursor `line:col`, `%` through file. Lua registers components via
`ruster.statusline.section(pos, fn)` returning a string; the Rust side lays out
left/center/right groups. Ships a default configuration equivalent to a minimal lualine.

## UI Primitive: Picker (shared floating list)

Ibuffer, Dired, FZF/Rg, and which-key are all "a floating list you filter and select from."
Build one reusable primitive in `ruster-tui`:

```rust
pub struct PickerState {
    pub title: String,
    pub items: Vec<PickerItem>,   // { label, detail, payload }
    pub filter: String,           // typed query (fuzzy-matched)
    pub selected: usize,
    pub on_accept: PickerAction,  // OpenBuffer(BufferId) | OpenPath(PathBuf) | RunCmd(String)
}
```

Rendered as a centered floating box (`PickerView`) over the window frame. Fuzzy matching uses
the `nucleo-matcher` crate. `Esc` cancels, `Enter` accepts, `Ctrl-n/p` or arrows move.

## Feature specifics

### Window Splits
- Commands: `:split`/`:sp` (horizontal), `:vsplit`/`:vs` (vertical), `:close`/`:q` (close active
  window; quits app only when it is the last window), `:only`/`:on` (close others).
- Keys: `Ctrl-w s`, `Ctrl-w v`, `Ctrl-w c`, `Ctrl-w h/j/k/l` (focus), `Ctrl-w o` (only).
- Lua: `ruster.api.nvim_open_win`, `nvim_win_close`, `nvim_set_current_win`,
  `nvim_list_wins`, `nvim_win_get_buf`/`nvim_win_set_buf`.

### Toggle Fullscreen
- `Ctrl-w z` / `:fullscreen` toggles `WindowTree.fullscreen`. When set, only the active window
  is rendered full-area; layout is preserved and restored on toggle-off. No buffer/window state
  is lost.

### Ibuffer (buffer list)
- `:ls`/`:buffers` or `Space b` opens a Picker over `BufferStore::ids()`.
- Columns: id, modified flag, name, filetype. Filter by typed substring/fuzzy.
- `Enter` switches the active window to that buffer; `d` marks for delete, `x` executes
  deletes (refuses modified buffers without `!`).

### Dired (file explorer)
- `:Dired [path]` or `-` opens a Special buffer listing a directory.
- Lines are entries (dirs first, sorted). `Enter` opens file / descends dir; `^` goes up;
  `R` rename, `D` delete (confirm), `+` create file, `%` create dir. Backed by `std::fs`.
- Rendered through the normal window path (it is a buffer), not the Picker.

### FZF / Ripgrep
- `:Files` (or `Space f`) — spawn a directory walk (`ignore` crate, respects `.gitignore`) →
  Picker of paths → `Enter` opens the file in the active window.
- `:Rg <pattern>` (or `Space g`) — spawn `rg --vimgrep <pattern>` via `std::process::Command`
  in a background thread, stream results into a Picker (`file:line:col: text`) → `Enter` opens
  at that location. Falls back with a clear message if `rg` is not on `PATH`.
- Uses the async event loop already in `app.rs:297` (`AppEvent` channel) to receive results
  without blocking the 60fps render.

### Mini-buffer & which-key
- The cmdline becomes a floating mini-buffer showing the typed command; command completion
  (`:` + Tab) lists matching commands in the Picker.
- **which-key**: when a key sequence is pending (e.g. `Ctrl-w`, `Space`, `g`) and no full
  binding has resolved within a short timeout (~`timeoutlen`, default 300ms), pop a
  non-modal panel listing the continuations available from the current keymap node and their
  descriptions. Driven by the existing keymap tree in `ruster-lua/src/keymap.rs`.

## New dependencies

| Crate | Used for |
|-------|----------|
| `nucleo-matcher` | fuzzy matching in the Picker (ibuffer, files) |
| `ignore` | gitignore-aware file walk for `:Files` |
| `ripgrep` (external binary, not a crate) | `:Rg` live grep via `std::process::Command` |

No new workspace crates; all changes land in existing crates.

## Crate Changes Summary

| Crate | Changes |
|-------|---------|
| `ruster-core` | New `document.rs`, `workspace.rs`, `windows.rs`; refactor `Editor` to act on borrowed `(Buffer, CursorSet, UndoStack)`; undo moves onto `Document`. |
| `ruster-render` | Replace single-window `EditorState` with `FrameState`/`WindowView`; add `GutterView`, `StatuslineView`, `PickerView`. |
| `ruster-tui` | Multi-window `App`; per-window render; gutter, statusline, Picker primitive; ibuffer, dired, fzf/rg; which-key; `Ctrl-w` bindings; split/close/only commands; fullscreen. |
| `ruster-render-raylib` | Render window rects, gutter, statusline, floating picker; `Ctrl-w`/`Space` leader routing. |
| `ruster-lua` | Window/buffer API (`nvim_open_win`, `nvim_list_bufs`, …); `ruster.statusline.section`; gutter/leader config; which-key descriptions on keymaps. |

## Non-goals (deferred)

- Emacs-style window rotation/resize commands beyond basic split/close/only/focus.
- Persisting layout across sessions (Phase 7 Session Management).
- Neo-tree sidebar file explorer with git status (Phase 5); Dired here is the minimal buffer-based explorer.
- Tabs/tabpages (multiple layouts) — windows only this phase.
