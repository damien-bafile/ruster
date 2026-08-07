# Sidebar Completion Implementation Plan

> **Status:** delivered; boxes resolved 2026-08-07.
>
> This plan was executed but never ticked as it went, leaving 31 boxes
> that read as outstanding work. They are **plain bullets now, not back-ticked**:
> a box ticked long after the fact asserts a verification that did not happen,
> and this tree has already been bitten by exactly that. The bullets stand as a
> record of what was built.
>
> Evidence it shipped: all 30 identifiers this plan names in backticks exist in
> the tree, and `docs/verification/sidebar-{tui.txt,gui.png}`.


> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish the File Explorer Sidebar — hidden-files toggle, refresh, jump-to-first/last, `:Sidebar resize N` command, auto-open config, and auto-refresh after file operations.

**Architecture:** All additions are straight-forward wire-ups of existing infrastructure. The `SidebarTree` data model gets two new methods; the app layer wires keys, a command variant, and a config option. No new files.

**Tech Stack:** `ruster-core::sidebar`, `ruster-lua::schema`/`config`, `ruster-tui::app`, `docs/keybindings.md`

## Global Constraints

- Sidebar width clamps to 16–60 columns.
- File operations reuse `DiredPrompt`/`DiredPromptKind` infrastructure.
- Config option `sidebar.auto_open` defaults to `false`.
- `show_hidden` toggle on `SidebarTree` is independent of `dired_show_hidden`.
- Update `docs/keybindings.md` with all missing keys.

---

### Task 1: Add `set_show_hidden` and `refresh` to `SidebarTree`

**Files:**
- Modify: `crates/ruster-core/src/sidebar.rs`

**Interfaces:**
- Produces: `pub fn set_show_hidden(&mut self, v: bool)` on `SidebarTree`
- Produces: `pub fn refresh(&mut self)` on `SidebarTree`

- Add `set_show_hidden`, `show_hidden` getter, and `refresh` methods after line 74 (`reveal`):

```rust
pub fn set_show_hidden(&mut self, v: bool) {
    self.show_hidden = v;
}

pub fn show_hidden(&self) -> bool {
    self.show_hidden
}

/// Discard expanded-state cache so the tree re-reads from disk on the
/// next [`rows()`](Self::rows) call. Call after file-system mutations.
pub fn refresh(&mut self) {
    self.expanded.clear();
    // Re-expand root so the top level is visible.
    self.expand(&self.root);
}
```

- Build and test:

```bash
cargo test -p ruster-core -- sidebar 2>&1 | tee
```

Expected: all sidebar tests pass.

- Commit:

```bash
git add crates/ruster-core/src/sidebar.rs
git commit -m "feat(sidebar): add set_show_hidden() and refresh() methods"
```

---

### Task 2: Add `sidebar_auto_open` Lua config option

**Files:**
- Modify: `crates/ruster-lua/src/schema.rs`
- Modify: `crates/ruster-lua/src/config.rs`

- In `crates/ruster-lua/src/schema.rs`, add `sidebar` group to `GROUPS` (line 192, after `dired`):

```rust
("sidebar", "File-explorer sidebar"),
```

- In `crates/ruster-lua/src/schema.rs`, add the setting in the `schema()` function (after the `dired` block, before `colors`):

```rust
// --- sidebar ---
add("sidebar", "auto_open", "Auto-open sidebar", Bool, b(false), "Open sidebar at startup when a project root is detected");
```

- In `crates/ruster-lua/src/config.rs`, add field to the `Config` struct (after `dired_show_hidden`):

```rust
pub sidebar_auto_open: bool,
```

- In `crates/ruster-lua/src/config.rs`, add default in `Default` impl (after `dired_show_hidden`):

```rust
sidebar_auto_open: false,
```

- In `crates/ruster-lua/src/config.rs`, add entry in `to_settings()` (after the `dired` entry):

```rust
(("sidebar", "auto_open"), Bool(self.sidebar_auto_open)),
```

- In `crates/ruster-lua/src/config.rs`, add entry in `from_settings()` (after `dired_show_hidden`):

```rust
sidebar_auto_open: bl("sidebar", "auto_open", d.sidebar_auto_open),
```

- Build:

```bash
cargo build -p ruster-lua 2>&1 | tail -10
```

- Commit:

```bash
git add crates/ruster-lua/src/schema.rs crates/ruster-lua/src/config.rs
git commit -m "feat(sidebar): add sidebar.auto_open Lua config option"
```

---

### Task 3: Wire `gg`, `G`, `.`, `R` keys in sidebar

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` (`handle_sidebar_key`, around lines 4639-4666)

- Add a `pending_g` field to the `App` struct (near `sidebar_prompt_dir`, line 1407):

```rust
sidebar_pending_g: bool,
```

Initialize to `false` in the constructor (line 1406):

```rust
sidebar_pending_g: false,
```

- In `handle_sidebar_key` after the Delete block (line 4664), add key cases before `_ => false`:

```rust
KeyCode::Char('g') if ck.modifiers.is_empty() => {
    if self.sidebar_pending_g {
        self.sidebar_selected = 0;
        self.sidebar_pending_g = false;
    } else {
        self.sidebar_pending_g = true;
    }
    true
}
KeyCode::Char('G') if ck.modifiers.is_empty() => {
    self.sidebar_selected = rows.len().saturating_sub(1);
    self.sidebar_pending_g = false;
    true
}
KeyCode::Char('.') if ck.modifiers.is_empty() => {
    tree.set_show_hidden(!tree.show_hidden());
    true
}
KeyCode::Char('R') if ck.modifiers.is_empty() => {
    tree.refresh();
    true
}
```

- Add reset of `sidebar_pending_g` on any non-g key — insert after the `match` block ends (after `handled` variable assignment) and before the clamp:

```rust
KeyCode::Char('.') if ck.modifiers.is_empty() => {
    tree.set_show_hidden(!tree.show_hidden());
    true
}
```

- Add `R` refresh:

```rust
KeyCode::Char('R') if ck.modifiers.is_empty() => {
    tree.refresh();
    true
}
```

- Clear `sidebar_pending_g` on any other key (add after `handled` block, before clamping):

In the `_ => false` arm is fine — that's the fallthrough. But `sidebar_pending_g` should reset on any non-g key motion. Add right before the clamp:

```rust
// Reset gg-pending state on any non-g key.
if !matches!(ck.code, KeyCode::Char('g')) {
    self.sidebar_pending_g = false;
}
```

- Build:

```bash
cargo build -p ruster-tui 2>&1 | tail -10
```

- Commit:

```bash
git add crates/ruster-tui/src/app.rs
git commit -m "feat(sidebar): gg/G jump, . toggle hidden, R refresh keys"
```

---

### Task 4: Add `:Sidebar resize N` command

**Files:**
- Modify: `crates/ruster-tui/src/app.rs`

- Add `SidebarResize(u16)` variant to `CmdAction` enum (find it near the top of `app.rs`, after existing sidebar entries):

Search for `CmdAction::Sidebar` to find the enum. Add:

```rust
SidebarResize(u16),
```

- Add parser in `parse_cmdline` (find the `sidebar` match and add after it):

```rust
_ if let Some(n) = trimmed.strip_prefix("sidebar resize ").and_then(|s| s.trim().parse::<u16>().ok()) => Ok(CmdAction::SidebarResize(n)),
```

- Add match arm in `apply_cmd` (find `CmdAction::Sidebar` match):

```rust
CmdAction::SidebarResize(n) => {
    self.sidebar_width = n.max(16).min(60);
}
```

- Build:

```bash
cargo build -p ruster-tui 2>&1 | tail -10
```

- Commit:

```bash
git add crates/ruster-tui/src/app.rs
git commit -m "feat(sidebar): add :Sidebar resize N command"
```

---

### Task 5: Auto-refresh sidebar after file operations

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` (`dired_execute_prompt`, line 4262)

The sidebar tree currently clamps selection after a create/rename/delete but does NOT call `refresh()`. This means new files don't appear and deleted ones still show until the sidebar is toggled off/on.

- In `dired_execute_prompt` at line 4262, replace the `if is_sidebar` block:

Old code:
```rust
if is_sidebar {
    if self.sidebar.is_some() {
        let rows = self.sidebar.as_ref().unwrap().rows();
        self.sidebar_selected = self.sidebar_selected.min(rows.len().saturating_sub(1));
    }
}
```

New code:
```rust
if is_sidebar {
    if let Some(ref mut tree) = self.sidebar {
        tree.refresh();
        let rows = tree.rows();
        self.sidebar_selected = self.sidebar_selected.min(rows.len().saturating_sub(1));
    }
}
```

- Build:

```bash
cargo build -p ruster-tui 2>&1 | tail -10
```

- Commit:

```bash
git add crates/ruster-tui/src/app.rs
git commit -m "fix(sidebar): auto-refresh tree after file create/rename/delete"
```

---

### Task 6: Auto-open sidebar at startup

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` (App constructor)

- In the App constructor, after `ensure_dashboard_buffer()` and `ensure_messages_buffer()` (line 1414), add:

```rust
// Auto-open sidebar if configured and a project root is detected.
if app.config.sidebar_auto_open && app.project_root.is_some() {
    app.toggle_sidebar();
}
```

- Build:

```bash
cargo build -p ruster-tui 2>&1 | tail -10
```

- Commit:

```bash
git add crates/ruster-tui/src/app.rs
git commit -m "feat(sidebar): auto-open at startup when sidebar.auto_open is true"
```

---

### Task 7: Update keybindings docs

**Files:**
- Modify: `docs/keybindings.md`

- Read existing sidebar section in `docs/keybindings.md` and add the missing key entries (`gg`, `G`, `.`, `R`).

```markdown
| `gg` / `G` | Jump to first / last row |
| `.` | Toggle hidden files |
| `R` | Refresh the tree from disk |
```

- Commit:

```bash
git add docs/keybindings.md
git commit -m "docs: document sidebar gg/G/./R keys"
```
