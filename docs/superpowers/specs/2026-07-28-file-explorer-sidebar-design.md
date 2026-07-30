# File-Explorer Sidebar — Design Spec

**Goal:** A persistent tree-view sidebar (Neo-tree / VS Code style) that lets users
browse, open, create, rename, and delete files without switching buffers.

**Status:** Approved; ready for implementation planning.

---

## Architecture

The sidebar is **not** a buffer inside `WindowTree` — it is a fixed column carved from
the render area before the window tree computes its layout. This keeps the sidebar
independent of the user's split layout: splits, closing windows, and fullscreen toggles
do not affect the sidebar.

The data model (`ruster_core::sidebar::SidebarTree`) already exists. The `App` struct
already carries the sidebar fields (`sidebar`, `sidebar_selected`, `sidebar_scroll`,
`sidebar_focused`) and a `SIDEBAR_WIDTH` constant. None are wired to rendering or
keyboard input.

### Data model (existing, reused)

```
SidebarTree {
    root: PathBuf,
    expanded: BTreeSet<PathBuf>,
    show_hidden: bool,
}
```

- Lazily-expanded: children of a collapsed dir are not included in `rows()`.
- `expand(path)` / `collapse(path)` / `toggle(path)` manage the `expanded` set.
- `reveal(path)` expands every ancestor of `path` so it becomes visible.
- `rows() -> Vec<SidebarRow>` returns a depth-first flat list with `depth`, `is_dir`,
  and `expanded` flag on each row.
- No `..` entry (unlike dired — the tree has an explicit root).

### State in App

```rust
sidebar: Option<SidebarTree>,
sidebar_selected: usize,       // index into sidebar.rows()
sidebar_scroll: usize,         // scroll offset for the sidebar WindowView
sidebar_focused: bool,         // true when keyboard input targets the sidebar
```

---

## Window layout

```
+--------+--------------------------------------------+
|        |                                            |
|        |    Window tree (splits, etc.)              |
|   30   |                                            |
|  cols  |                                            |
|        |                                            |
|        |                                            |
+--------+--------------------------------------------+
```

In `App::render()`:

1. If `sidebar.is_some()`, steal `SIDEBAR_WIDTH` columns from the left of `buf_area`.
2. Build a `WindowView` for the sidebar — normal statusline at top (`"Sidebar"` label),
   gutter, `StyledLine` rows from `SidebarTree::rows()`, with the current selection
   highlighted.
3. Pass the remaining area to `w.windows.compute_rects()`. The sidebar's `WindowView`
   is prepended to `FrameState::windows`.
4. Both backends render it automatically — no backend-specific code needed.

When the sidebar is closed (hidden), the window tree gets the full area.

---

## Lua Config

```lua
ruster.config.sidebar = {
  auto_open = false,   -- Open sidebar automatically at startup when a project root is detected
}
```

The `sidebar` group is registered in the schema with a `Bool` setting `auto_open` (default `false`).

---

## Toggle & commands

| Trigger | Action |
|---------|--------|
| `SPC e` (or `:Sidebar`) | Toggle sidebar open/closed. If opening: create tree at project root (or cwd), focus sidebar. If closing: focus the window tree. |
| `Esc` / `Tab` (sidebar focused) | Focus the active window in the window tree. |
| `:Sidebar` | Toggle. |
| `:Sidebar resize N` | Set `sidebar_width` to N columns (min 16, max 60). |

When `sidebar.auto_open = true`, the sidebar opens at startup after project root
detection (alongside the Dashboard).

---

## Keyboard navigation (sidebar focused)

| Key | Action |
|-----|--------|
| `j` / `↓` | Move selection down one row |
| `k` / `↑` | Move selection up one row |
| `Enter` | If directory: toggle expand/collapse. If file: open in the active window, then focus the window tree. |
| `l` / `→` | Expand a collapsed directory (same as Enter on a dir) |
| `h` / `←` | Collapse an expanded directory |
| `gg` | Jump to first row |
| `G` | Jump to last row |
| `r` | Rename the selected entry (reuse dired prompt + same confirmation flow) |
| `d` | Delete (reuse dired prompt: confirm with `y`/`n`) |
| `a` | Create new file or directory (reuse dired `+` prompt: name ending in `/` = directory) |
| `.` | Toggle hidden files |
| `R` | Refresh (re-read the tree from disk) |
| `Esc` / `Tab` | Focus the main window tree |

All operations reuse existing dired prompt infrastructure (`DiredPrompt`, `DiredPromptKind`
in app.rs). After a create/rename/delete prompt completes, the sidebar automatically
refreshes to reflect the new filesystem state.

---

## Follow active file

Whenever the active buffer changes to a file-backed document whose path is inside the
sidebar's root, call `sidebar.reveal(path)`, select the file's row, and scroll to keep
it visible. This is triggered in `App::set_active_buffer`: after the buffer switches,
if the sidebar is open and the new buffer has a file path under the sidebar root,
reveal the path. Tab-completing a path argument in `:e` also counts.

---

## Tests

- Expand/collapse a known directory tree over a temp dir fixture.
- Render rows produce correct depth/indentation.
- Reveal expands ancestor chain.
- Toggling hidden files includes/excludes dotfiles.
- `:Sidebar` command parsing and dispatch.
- Focus routing: sidebar-focused vs. window-tree-focused key dispatch.
- Follow-active-file selects the right row after opening a file.
