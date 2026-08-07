# Project Workspaces UI — Implementation Plan

> **Status:** delivered; boxes resolved 2026-08-07.
>
> This plan was executed but never ticked as it went, leaving 12 boxes
> that read as outstanding work. They are **plain bullets now, not back-ticked**:
> a box ticked long after the fact asserts a verification that did not happen,
> and this tree has already been bitten by exactly that. The bullets stand as a
> record of what was built.
>
> Evidence it shipped: all 8 identifiers this plan names in backticks exist in
> the tree, and `docs/verification/projects-{tui.txt,gui.png}` shows the recent-project picker.


> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Wire a `:projects` picker for recent projects, auto-record projects on file opens, and restore the last project on startup.

**Architecture:** The data layer (`ruster_project`) already provides `project_root()`, `recent_projects()`, `record_recent()`. Wire the picker UI (following the existing `open_files_picker` pattern), add auto-record calls in project-anchored actions, and save/restore the last project root.

**Tech Stack:** Rust, ruster-tui (app.rs), ruster-project, ruster-lua (config)

## Global Constraints

- No new crate dependencies
- Follow existing picker patterns
- Auto-record on `:e`, `:Files`, sidebar open, `:term`

---

### Task 1: Implement the :projects picker

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` (open_projects method)

**Interfaces:**
- Consumes: `ruster_project::recent_projects(dir)`, `PickerState`, `PickerAction::RunCmd(String)` for `:cd <path>`
- Produces: `App::open_projects_picker()` wired to `CmdAction::Projects`

- **Step 1: Check open_projects exists**

Search for `fn open_projects` in app.rs. If it already exists (the exploration found it at line 4487), verify it's complete. If not, implement it:

```rust
fn open_projects_picker(&mut self) {
    let state_dir = dirs::state_dir()
        .or_else(|| Some(PathBuf::from(".")))
        .unwrap();
    let projects = ruster_project::recent_projects(&state_dir);
    let items: Vec<PickerItem> = projects.into_iter().map(|p| {
        let label = p.to_string_lossy().to_string();
        PickerItem {
            label,
            action: PickerAction::RunCmd(format!(":cd {}", p.display())),
        }
    }).collect();
    if items.is_empty() {
        self.message = Some("No recent projects".to_string());
        return;
    }
    self.picker = Some(PickerState::new("Projects", items));
}
```

- **Step 2: Verify CmdAction::Projects already calls open_projects_picker**

Search for `CmdAction::Projects` in the command dispatch in `execute_cmd()` or `parse_cmdline()`. If it calls `open_projects()` already, ensure the method body from Step 1 is filled in. If not, add the match arm:

```rust
CmdAction::Projects => {
    self.open_projects_picker();
    return true;
}
```

- **Step 3: Build to verify**

```
cargo build -p ruster-tui 2>&1 | tail -5
```

- **Step 4: Commit**

```
git add crates/ruster-tui/src/app.rs
git commit -m "feat(workspaces): implement :projects picker"
```

---

### Task 2: Auto-record project on file operations

**Files:**
- Modify: `crates/ruster-tui/src/app.rs`

**Interfaces:**
- Consumes: `ruster_project::record_recent(state_dir, root, max)`
- Produces: `App::record_current_project()` helper

- **Step 1: Add record_current_project helper**

```rust
fn record_current_project(&self) {
    let root = match &self.project_root {
        Some(r) => r.clone(),
        None => return,
    };
    let state_dir = dirs::state_dir()
        .or_else(|| Some(PathBuf::from(".")))
        .unwrap();
    if !state_dir.exists() {
        std::fs::create_dir_all(&state_dir).ok();
    }
    ruster_project::record_recent(&state_dir, &root, 20);
}
```

- **Step 2: Call it at project-anchored moments**

Add `self.record_current_project()` calls after:
- `set_active_buffer` switches to a buffer whose path is under a project root
- `open_files_picker()` (when a file is selected)
- `toggle_sidebar()` (when opening a sidebar)
- `open_terminal()` (when opening a terminal)

For each, just add the one-liner call after the relevant logic.

- **Step 3: Build to verify**

```
cargo build -p ruster-tui 2>&1 | tail -5
```

- **Step 4: Commit**

```
git add crates/ruster-tui/src/app.rs
git commit -m "feat(workspaces): auto-record project on file ops"
```

---

### Task 3: Restore last project on startup

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` (App constructor / init)

**Interfaces:**
- Consumes: `ruster_project::recent_projects(dir)`
- Modifies: `App::new()` or startup init sequence

- **Step 1: Add project restore at startup**

In the `App::new()` or `init()` method, after setting the initial project_root (or as a replacement for it), read recent projects and restore the most recent one if it still exists:

```rust
// Restore last project.
if self.project_root.is_none() {
    let state_dir = dirs::state_dir()
        .or_else(|| Some(PathBuf::from(".")))
        .unwrap();
    let recent = ruster_project::recent_projects(&state_dir);
    if let Some(last) = recent.first() {
        if last.exists() {
            self.project_root = Some(last.clone());
        }
    }
}
```

Place this after the initial `project_root` detection (where it checks for root markers from the cwd).

- **Step 2: Build to verify**

```
cargo build -p ruster-tui 2>&1 | tail -5
```

- **Step 3: Run tests**

```
cargo test -p ruster-tui 2>&1 | tail -5
```

All existing tests should pass.

- **Step 4: Commit**

```
git add crates/ruster-tui/src/app.rs
git commit -m "feat(workspaces): restore last project on startup"
```
