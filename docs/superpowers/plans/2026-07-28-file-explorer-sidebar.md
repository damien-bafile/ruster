# File-Explorer Sidebar — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A persistent tree-view sidebar (Neo-tree / VS Code style) for browsing, opening, creating, renaming, and deleting files without switching buffers.

**Architecture:** The sidebar is a fixed-width column carved from the render area before the window tree gets the rest — not a window in the split tree. The data model (`ruster_core::sidebar::SidebarTree`) and app fields (`sidebar`, `sidebar_selected`, etc.) already exist but are unused.

**Tech Stack:** `ruster-core::sidebar`, `ruster-core::dired`, `ruster-tui::app`, `ruster-render`.

## Global Constraints

- Sidebar is left-aligned, 30 columns wide, resizable via `:Sidebar resize N` (min 16, max 60).
- Reuses `DiredPrompt`/`DiredPromptKind` for rename/delete/create operations.
- Both TUI and GUI backends render the sidebar automatically — only `FrameState::windows` is modified.
- Follows the existing `ensure_*_buffer` pattern for lifecycle management.
- Tests use temp dir fixtures.
- Update `docs/keybindings.md`.

---

### Task 1: Render sidebar column

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` (render method, ~lines 2611-2680)

**Interfaces:**
- Consumes: `self.sidebar: Option<SidebarTree>`, `SIDEBAR_WIDTH: u16 = 30`
- Produces: sidebar rendered as a `WindowView` in `FrameState::windows`

**Goal:** When `self.sidebar.is_some()`, carve `self.sidebar_width` columns from the left of `buf_area`, build a `WindowView` for the sidebar with rows from `SidebarTree::rows()`, and add it to the frame. When hidden, nothing changes.

- [ ] **Step 1: Locate the render method**

The render method starts around line 2611 (`fn render(&mut self)`). The key section is where `buf_area` is defined and `compute_rects` is called. Find the exact line numbers.

- [ ] **Step 2: Add `sidebar_width` field to App**

Replace the `SIDEBAR_WIDTH` const with a field on `App`:

```rust
// Remove: const SIDEBAR_WIDTH: u16 = 30;
// Add to App struct:
sidebar_width: u16,
```

Initialize to `30` in the constructor.

- [ ] **Step 3: Carve sidebar area**

After computing `buf_area` from the viewport, if `self.sidebar.is_some()`, split `buf_area` into `sidebar_area` (self.sidebar_width wide on the left) and `content_area` (the remainder). Pass `content_area` to `compute_rects` instead of `buf_area`.

```rust
let sidebar_rect = if self.sidebar.is_some() {
    let w = self.sidebar_width.min(buf_area.width.saturating_sub(4));
    let sidebar = RRect { x: buf_area.x, y: buf_area.y, width: w, height: buf_area.height };
    buf_area = RRect { x: buf_area.x + w, y: buf_area.y, width: buf_area.width.saturating_sub(w), height: buf_area.height };
    Some(sidebar)
} else {
    None
};
```

- [ ] **Step 4: Build sidebar WindowView**

After the existing window loop, if `sidebar_rect.is_some()`, build a `WindowView`:

```rust
if let Some(srect) = sidebar_rect {
    let tree = self.sidebar.as_ref().unwrap();
    let rows = tree.rows();
    let selected = self.sidebar_selected.min(rows.len().saturating_sub(1));
    let scroll = self.sidebar_scroll.min(selected.saturating_sub(srect.height as usize / 2));
    let lines: Vec<StyledLine> = rows.iter().enumerate().skip(scroll).take(srect.height as usize).map(|(i, r)| {
        let indent = "  ".repeat(r.depth);
        let marker = if r.is_dir { if r.expanded { "▾ " } else { "▸ " } } else { "  " };
        let text = format!("{}{}{}", indent, marker, r.name);
        let style = if i == selected { /* highlight style */ } else { /* normal style */ };
        StyledLine { text, highlights: vec![] }
    }).collect();
    let view = WindowView {
        rect: srect,
        lines,
        cursor: (0, 0),
        extra_cursors: vec![],
        cursor_kind: CursorKind::Block,
        cursor_visible: false,
        cursor_smooth: None,
        scroll_offset: 0,
        gutter: GutterView { width: 0, rows: vec![] },
        signs: SignsView { width: 0, signs: vec![] },
        statusline: StatuslineView { left: "Sidebar".into(), center: String::new(), right: format!("{} items", rows.len()), active: self.sidebar_focused },
        active: self.sidebar_focused,
        selection: None,
        terminal: None,
        header: String::new(),
    };
    views.insert(0, view);
}
```

Use `cursor_visible: false` to hide the cursor, no selection, no terminal.

- [ ] **Step 4: Build and test**

```bash
cargo build -p ruster-tui 2>&1 | tail -5
cargo test -p ruster-tui 2>&1 | grep "test result"
```

- [ ] **Step 5: Commit**

```bash
git add crates/ruster-tui/src/app.rs
git commit -m "feat: render sidebar column carved from window area"
```

---

### Task 2: Toggle sidebar command + binding + auto-open

**Files:**
- Modify: `crates/ruster-tui/src/app.rs`

**Interfaces:**
- Consumes: `self.project_root: Option<PathBuf>`, `CmdAction`, `LeaderAction`, `OPEN_GROUP`
- Produces: `CmdAction::Sidebar`, `LeaderAction::Sidebar`, `:Sidebar` parser, `SPC e` binding (replaces old explorer binding), auto-open at startup

- [ ] **Step 1: Add `Sidebar` to `CmdAction` enum**

Find the `CmdAction` enum definition. It's near the top of `app.rs`. Add:

```rust
Sidebar,
SidebarResize(u16),
```

- [ ] **Step 2: Add `Sidebar` to `LeaderAction` enum**

Find `LeaderAction` and add after the existing entries:

```rust
Sidebar,
```

- [ ] **Step 3: Parse `:sidebar` command**

Find the `parse_cmdline` function and add:

```rust
_ if trimmed == "sidebar" => Ok(CmdAction::Sidebar),
_ if let Some(n) = trimmed.strip_prefix("sidebar resize ").and_then(|s| s.trim().parse::<u16>().ok()) => Ok(CmdAction::SidebarResize(n)),
```

- [ ] **Step 4: Wire `SPC e` to toggle sidebar**

In `OPEN_GROUP`, change the existing `'e'` entry from explorer to sidebar:

```rust
('e', LeaderNode::Action("sidebar", LeaderAction::Sidebar)),
```

Keep dired accessible via `:Dired` / `:Explore`.

In `FIND_GROUP`, also change `'e'`:

```rust
('e', LeaderNode::Action("sidebar", LeaderAction::Sidebar)),
```

- [ ] **Step 5: Wire match arms in `apply_cmd` and `apply_leader_action`**

In `apply_cmd`:

```rust
CmdAction::Sidebar => self.toggle_sidebar(),
CmdAction::SidebarResize(n) => {
    self.sidebar_width = n.max(16).min(60);
    // Re-render will pick it up next frame.
}
```

In `apply_leader_action`:

```rust
LeaderAction::Sidebar => self.toggle_sidebar(),
```

- [ ] **Step 6: Implement `toggle_sidebar` method**

```rust
fn toggle_sidebar(&mut self) {
    if self.sidebar.is_some() {
        self.sidebar = None;
        self.sidebar_focused = false;
    } else {
        let root = self.project_root.clone().unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        self.sidebar = Some(ruster_core::sidebar::SidebarTree::new(root, false));
        self.sidebar_selected = 0;
        self.sidebar_scroll = 0;
        self.sidebar_focused = true;
    }
}
```

- [ ] **Step 7: Auto-open sidebar at startup**

In the startup initialization (where dashboard is created), after a project root is detected:

```rust
if self.project_root.is_some() {
    self.toggle_sidebar();
}
```

This should be after the Dashboard setup.

- [ ] **Step 8: Build and test**

```bash
cargo build -p ruster-tui 2>&1 | tail -5
cargo test -p ruster-tui 2>&1 | grep "test result"
```

- [ ] **Step 9: Write tests for sidebar toggle**

Add tests in the `app.rs` test module:

```rust
#[test]
fn sidebar_toggle_creates_tree() {
    let mut a = test_app();
    assert!(a.sidebar.is_none());
    a.toggle_sidebar();
    assert!(a.sidebar.is_some());
    a.toggle_sidebar();
    assert!(a.sidebar.is_none());
}
```

```bash
cargo test -p ruster-tui -- sidebar_toggle --nocapture 2>&1
```

- [ ] **Step 10: Commit**

```bash
git add crates/ruster-tui/src/app.rs
git commit -m "feat: :Sidebar command, SPC e toggle, and auto-open at startup"
```

---

### Task 3: Keyboard navigation + focus

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` (handle_key method)

**Interfaces:**
- Consumes: `self.sidebar.is_some() && self.sidebar_focused`
- Produces: sidebar navigation (j/k/h/l/Enter/gg/G/Esc/Tab)

- [ ] **Step 1: Add sidebar key route guard**

In `handle_key`, before the main editor key handler, add:

```rust
if self.sidebar.is_some() && self.sidebar_focused {
    self.handle_sidebar_key(ck);
    return;
}
```

Place this near the existing guards (dired_prompt, picker, cmdline, etc.).

- [ ] **Step 2: Implement `handle_sidebar_key`**

```rust
fn handle_sidebar_key(&mut self, ck: crossterm::event::KeyEvent) {
    use crossterm::event::{KeyCode, KeyModifiers};
    let tree = match self.sidebar.as_mut() {
        Some(t) => t,
        None => return,
    };
    let rows = tree.rows();
    if rows.is_empty() { return; }
    let max = rows.len().saturating_sub(1);
    match ck.code {
        KeyCode::Char('j') | KeyCode::Down => {
            self.sidebar_selected = self.sidebar_selected.saturating_add(1).min(max);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            self.sidebar_selected = self.sidebar_selected.saturating_sub(1);
        }
        KeyCode::Char('h') | KeyCode::Left => {
            if let Some(row) = rows.get(self.sidebar_selected) {
                if row.is_dir { tree.collapse(&row.path); }
            }
        }
        KeyCode::Char('l') | KeyCode::Right | KeyCode::Enter => {
            if let Some(row) = rows.get(self.sidebar_selected) {
                if row.is_dir {
                    tree.toggle(&row.path);
                } else {
                    // Open the file
                    let path = row.path.clone();
                    self.sidebar_focused = false;
                    self.open_path(&path, None);
                }
            }
        }
        KeyCode::Char('g') => {
            // gg — jump to first
            self.sidebar_selected = 0;
        }
        KeyCode::Char('G') => {
            // G — jump to last
            self.sidebar_selected = max;
        }
        KeyCode::Char('.') => {
            // Toggle hidden files
            // SidebarTree needs show_hidden exposed — see side quest below
        }
        KeyCode::Tab | KeyCode::Esc => {
            self.sidebar_focused = false;
        }
        _ => {}
    }
    // Clamp scroll to keep selection visible
    let rows_len = rows.len();
    let visible = 20usize; // estimated visible rows — refined in render
    if self.sidebar_selected < self.sidebar_scroll {
        self.sidebar_scroll = self.sidebar_selected;
    } else if self.sidebar_selected >= self.sidebar_scroll + visible {
        self.sidebar_scroll = self.sidebar_selected.saturating_sub(visible.saturating_sub(1));
    }
}
```

Note: `SidebarTree` needs `show_hidden` to be togglable. Check if it already is — if not, add a `set_show_hidden(&mut self, v: bool)` method in sidebar.rs.

- [ ] **Step 3: Side quest — make `show_hidden` togglable on SidebarTree**

In `crates/ruster-core/src/sidebar.rs`, add:

```rust
pub fn set_show_hidden(&mut self, v: bool) {
    self.show_hidden = v;
}
```

- [ ] **Step 4: Build and test**

```bash
cargo build 2>&1 | tail -5
cargo test -p ruster-core 2>&1 | grep "test result"
cargo test -p ruster-tui 2>&1 | grep "test result"
```

- [ ] **Step 5: Write tests for sidebar navigation**

```rust
#[test]
fn sidebar_navigation_moves_selection() {
    let mut a = test_app_with_sidebar();
    assert_eq!(a.sidebar_selected, 0);
    a.handle_sidebar_key(CtKey::new(KeyCode::Char('j'), KeyModifiers::NONE));
    assert_eq!(a.sidebar_selected, 1);
    a.handle_sidebar_key(CtKey::new(KeyCode::Char('k'), KeyModifiers::NONE));
    assert_eq!(a.sidebar_selected, 0);
}
```

```bash
cargo test -p ruster-tui -- sidebar_navigation --nocapture 2>&1
```

- [ ] **Step 6: Commit**

```bash
git add crates/ruster-tui/src/app.rs crates/ruster-core/src/sidebar.rs
git commit -m "feat: sidebar keyboard navigation (j/k/h/l/Enter/gg/G/Esc)"
```

---

### Task 4: Follow active file

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` (the active-buffer-change path)

**Interfaces:**
- Consumes: `self.sidebar.as_mut().map(|t| t.reveal(path))`
- Produces: sidebar auto-reveals the current file

- [ ] **Step 1: Find where active buffer changes**

The method `set_active_buffer` in `app.rs` is where the active buffer is set. Also check `open_path` and `open_dired` — but those are entry points. The simplest approach: intercept in `open_path` after the file is opened, or in `handle_key` after `CmdAction::OpenPath` resolves.

Actually the cleanest hook is in `open_path` itself — it's the single entry point for opening files:

```rust
// After the file is successfully opened:
if let Some(ref mut tree) = self.sidebar {
    if path.starts_with(&tree.root) {
        tree.reveal(&path);
        // Find the row index for this path and select it
        let rows = tree.rows();
        if let Some(idx) = rows.iter().position(|r| r.path == path) {
            self.sidebar_selected = idx;
        }
    }
}
```

- [ ] **Step 2: Build and test**

```bash
cargo build -p ruster-tui 2>&1 | tail -5
cargo test -p ruster-tui 2>&1 | grep "test result"
```

- [ ] **Step 3: Commit**

```bash
git add crates/ruster-tui/src/app.rs
git commit -m "feat: sidebar follows active file"
```

---

### Task 5: File operations (rename, delete, create)

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` (handle_sidebar_key)

**Interfaces:**
- Consumes: `DiredPrompt`/`DiredPromptKind`, `handle_dired_prompt_key`, existing dired file ops
- Produces: `r`, `d`, `a` keys in sidebar trigger dired prompts

- [ ] **Step 1: Add r/d/a key handling in sidebar**

In `handle_sidebar_key`, add cases:

```rust
KeyCode::Char('r') => {
    if let Some(row) = rows.get(self.sidebar_selected) {
        let name = row.path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        self.dired_prompt = Some(DiredPrompt { kind: DiredPromptKind::Rename(name), input: name.clone() });
    }
}
KeyCode::Char('d') => {
    if let Some(row) = rows.get(self.sidebar_selected) {
        self.dired_prompt = Some(DiredPrompt {
            kind: DiredPromptKind::Delete(row.path.clone()),
            input: String::new(),
        });
    }
}
KeyCode::Char('a') => {
    self.dired_prompt = Some(DiredPrompt { kind: DiredPromptKind::Create, input: String::new() });
}
KeyCode::Char('R') => {
    // Refresh: re-read directories from disk
    if let Some(tree) = self.sidebar.as_mut() {
        tree.refresh();
    }
}
```

- [ ] **Step 2: Handle prompt completion in sidebar context**

When a dired prompt completes (Enter is pressed), the existing code in `handle_dired_prompt_key` executes the operation. After it succeeds, refresh the sidebar tree to reflect file system changes.

First, add a `refresh()` method to `SidebarTree` (it preserves expanded state since it only re-reads directory listings):

In `crates/ruster-core/src/sidebar.rs`, add:
```rust
/// Re-read all expanded directories from disk (after file ops).
pub fn refresh(&mut self) {
    // No state to rebuild — rows() re-reads from disk lazily.
}
```

Then in `handle_dired_prompt_key`, after the operation completes:
```rust
if let Some(ref mut tree) = self.sidebar {
    tree.refresh();
}
```

- [ ] **Step 3: Build and test**

```bash
cargo build -p ruster-tui 2>&1 | tail -5
cargo test -p ruster-tui 2>&1 | grep "test result"
```

- [ ] **Step 4: Commit**

```bash
git add crates/ruster-tui/src/app.rs
git commit -m "feat: sidebar file operations (r/d/a/R keys)"
```

---

### Task 6: Update docs

**Files:**
- Modify: `docs/keybindings.md`

- [ ] **Step 1: Update keybindings.md**

Add sidebar entries:

```markdown
### Sidebar

| Key | Action |
|-----|--------|
| `SPC e` | Toggle sidebar |
| `:Sidebar` | Toggle sidebar |
| `:Sidebar resize N` | Set sidebar width to N columns |
| `j` / `k` | Move selection |
| `Enter` / `l` | Open file or expand directory |
| `h` | Collapse directory |
| `gg` / `G` | First / last row |
| `r` | Rename |
| `d` | Delete (confirm with y/n) |
| `a` | Create file or directory (trailing `/` = dir) |
| `.` | Toggle hidden files |
| `R` | Refresh tree |
| `Esc` / `Tab` | Focus the main window area |

Update the Space leader section to show `SPC e` → sidebar instead of explorer. Add `SPC e` to the OPEN_GROUP table.

- [ ] **Step 2: Commit**

```bash
git add docs/keybindings.md
git commit -m "docs: sidebar keybindings"
```
