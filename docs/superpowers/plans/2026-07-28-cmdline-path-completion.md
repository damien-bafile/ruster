# `:e` Command & Cmdline Path Completion — Implementation Plan

> **Status:** delivered; boxes resolved 2026-08-07.
>
> This plan was executed but never ticked as it went, leaving 48 boxes
> that read as outstanding work. They are **plain bullets now, not back-ticked**:
> a box ticked long after the fact asserts a verification that did not happen,
> and this tree has already been bitten by exactly that. The bullets stand as a
> record of what was built.
>
> Evidence it shipped: all 22 identifiers this plan names in backticks exist in
> the tree, and `docs/verification/cmdline-{tui.txt,gui.png}` show `:e /tmp/` Tab-completing, driven by real keystrokes in both backends.


> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement `:e <path>` / `:edit <path>` with Tab-based path autocompletion and Shift-Tab picker fallback in the cmdline.

**Architecture:** Add a `CmdAction::OpenFile` variant, parse `:e`/`:edit` in `parse_cmdline()`, implement a stateful `CmdlineCompletion` struct for cycling through directory entries on Tab, and open a picker on Shift-Tab.

**Tech Stack:** Rust, std `fs::read_dir`, `home` crate for `~` expansion.

## Global Constraints

- Follow existing code patterns (match arms in `parse_cmdline`, `CmdAction` enum dispatch)
- No new crate dependencies
- All changes compile with `cargo build -p ruster-tui`

---

### Task 1: Add `CmdAction::OpenFile` variant and `:e`/`:edit` parsing

**Files:**
- Modify: `crates/ruster-tui/src/app.rs:452-509` (CmdAction enum)
- Modify: `crates/ruster-tui/src/app.rs:3141-3233` (parse_cmdline)
- Modify: `crates/ruster-tui/src/app.rs:563-586` (PALETTE_COMMANDS)

- **Step 1: Add `OpenFile` variant to `CmdAction`**

In `crates/ruster-tui/src/app.rs`, add after the `Sidebar` variant (line 508):

```rust
    /// Open a file by path (`:e path` / `:edit path`).
    OpenFile(String),
```

- **Step 2: Add `:e` and `:edit` to `parse_cmdline()`**

In the `parse_cmdline()` match block, add before the `_ => Err(...)` fallback (line 3231):

```rust
            "e" | "edit" => Ok(CmdAction::Files),
            _ if trimmed.starts_with("e ") || trimmed.starts_with("edit ") => {
                let path = trimmed
                    .split_once(' ')
                    .map(|x| x.1)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if path.is_empty() {
                    Ok(CmdAction::Files)
                } else {
                    Ok(CmdAction::OpenFile(path))
                }
            }
```

- **Step 3: Add `:e` and `:edit` to `PALETTE_COMMANDS`**

In `PALETTE_COMMANDS` (line 563), add:

```rust
    ("e", "open file by path"),
    ("edit", "open file by path (alias)"),
```

- **Step 4: Verify it compiles**

Run: `cargo build -p ruster-tui 2>&1 | tail -5`
Expected: compiles with "unused variant `OpenFile`" warning (handled in Task 2).

- **Step 5: Commit**

```bash
git add crates/ruster-tui/src/app.rs
git commit -m "feat: add :e/:edit command parsing with CmdAction::OpenFile"
```

---

### Task 2: Implement `CmdAction::OpenFile` in `apply_cmd()`

**Files:**
- Modify: `crates/ruster-tui/src/app.rs:3235-3314` (apply_cmd)
- Modify: `crates/ruster-tui/src/app.rs:4680-4704` (open_path area)

- **Step 1: Add `resolve_path()` helper**

In `crates/ruster-tui/src/app.rs`, add near the `open_path()` function (around line 4678):

```rust
    /// Expand `~` and resolve relative paths against the active file's directory.
    fn resolve_path(&self, partial: &str) -> std::path::PathBuf {
        let expanded = if partial.starts_with("~/") {
            let home = home::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
            home.join(&partial[2..])
        } else if partial == "~" {
            home::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."))
        } else {
            std::path::PathBuf::from(partial)
        };
        if expanded.is_absolute() {
            expanded
        } else {
            let base = self
                .ws
                .borrow()
                .buffer()
                .file_path
                .as_ref()
                .and_then(|p| p.parent())
                .unwrap_or_else(|| std::path::Path::new("."));
            base.join(expanded)
        }
    }
```

- **Step 2: Handle `CmdAction::OpenFile` in `apply_cmd()`**

In the `apply_cmd()` match block, add after the `Sidebar` arm (line 3312):

```rust
            CmdAction::OpenFile(path) => {
                let resolved = self.resolve_path(&path);
                self.open_path(&resolved, None);
            }
```

- **Step 3: Verify it compiles**

Run: `cargo build -p ruster-tui 2>&1 | tail -5`
Expected: clean compile, no warnings.

- **Step 4: Commit**

```bash
git add crates/ruster-tui/src/app.rs
git commit -m "feat: implement :e/:edit with resolve_path() helper"
```

---

### Task 3: Add `CmdlineCompletion` struct and state to `App`

**Files:**
- Modify: `crates/ruster-tui/src/app.rs:881-999` (App struct)
- Modify: `crates/ruster-tui/src/app.rs` (App::new or Default impl)

- **Step 1: Define `CmdlineCompletion` struct**

In `crates/ruster-tui/src/app.rs`, add before the `App` struct (around line 879):

```rust
/// State for cmdline path completion cycling.
struct CmdlineCompletion {
    /// Completion candidates (relative paths as typed, with dirs ending in `/`).
    completions: Vec<String>,
    /// Index of the currently displayed candidate.
    index: usize,
    /// The cmdline text before completion started (for restoring on cancel).
    original: String,
}
```

- **Step 2: Add field to `App` struct**

In the `App` struct, add after the `pending_macro` field (line 960):

```rust
    /// Active cmdline path completion state (Tab cycling).
    cmdline_completion: Option<CmdlineCompletion>,
```

- **Step 3: Initialize in `App::new()`**

Find the `App::new()` function and add the field initialization. Search for `pending_macro:` in the struct literal and add after it:

```rust
            cmdline_completion: None,
```

- **Step 4: Verify it compiles**

Run: `cargo build -p ruster-tui 2>&1 | tail -5`
Expected: clean compile.

- **Step 5: Commit**

```bash
git add crates/ruster-tui/src/app.rs
git commit -m "feat: add CmdlineCompletion state struct to App"
```

---

### Task 4: Implement candidate generation and Tab cycling

**Files:**
- Modify: `crates/ruster-tui/src/app.rs:1685-1696` (Tab handling in handle_key)

- **Step 1: Add `generate_path_completions()` method**

In `crates/ruster-tui/src/app.rs`, add near the other helper methods:

```rust
    /// Generate path completion candidates for `:e <partial>`.
    fn generate_path_completions(&self, partial: &str) -> Vec<String> {
        use std::path::Path;

        let resolved = self.resolve_path(partial);
        // Determine the directory to read and the filename prefix to filter.
        let (dir, prefix) = if resolved.is_dir() {
            (resolved, String::new())
        } else {
            let dir = resolved.parent().unwrap_or_else(|| Path::new("."));
            let prefix = resolved
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            (dir.to_path_buf(), prefix)
        };

        let mut entries: Vec<String> = Vec::new();
        if let Ok(read) = std::fs::read_dir(&dir) {
            for entry in read.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.starts_with(&prefix) {
                    continue;
                }
                let is_dir = entry
                    .file_type()
                    .map(|ft| ft.is_dir())
                    .unwrap_or(false);
                entries.push(if is_dir {
                    format!("{}/", name)
                } else {
                    name
                });
            }
        }

        // Sort: directories first, then files, alphabetical within each.
        entries.sort_by(|a, b| {
            let a_dir = a.ends_with('/');
            let b_dir = b.ends_with('/');
            a_dir.cmp(&b_dir).then_with(|| a.to_lowercase().cmp(&b.to_lowercase()))
        });

        entries
    }
```

- **Step 2: Add `apply_completion_candidate()` method**

```rust
    /// Replace the path portion of the cmdline with the selected completion.
    fn apply_completion_candidate(&mut self, candidate: &str) {
        let cmdline = self.vim.cmdline_buffer();
        // Find the start of the path argument (after ":e " or ":edit ").
        let prefix_len = if let Some(pos) = cmdline.find(' ') {
            pos + 1 // include the space
        } else {
            return;
        };
        let new_cmdline = format!("{}{}", &cmdline[..prefix_len], candidate);
        self.vim.set_cmdline(&new_cmdline);
    }
```

- **Step 3: Rework Tab handler in `handle_key()`**

Replace the existing Tab handler (lines 1685-1696) with:

```rust
        // Tab in the cmdline: path completion for :e/:edit, command palette otherwise.
        if self.vim.mode == VimMode::Cmdline && key == KeyEvent::Tab {
            let buf = self.vim.cmdline_buffer();
            let trimmed = buf.trim_start_matches(':').trim();
            if trimmed.starts_with("e ") || trimmed.starts_with("edit ") {
                // Path completion mode.
                let partial = trimmed
                    .split_once(' ')
                    .map(|x| x.1)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if self.cmdline_completion.is_none() {
                    // First Tab: generate candidates.
                    let candidates = self.generate_path_completions(&partial);
                    if candidates.is_empty() {
                        self.set_message(format!("No matches for '{}'", partial));
                        return;
                    }
                    self.cmdline_completion = Some(CmdlineCompletion {
                        completions: candidates,
                        index: 0,
                        original: buf.clone(),
                    });
                } else {
                    // Subsequent Tab: cycle to next candidate.
                    let comp = self.cmdline_completion.as_mut().unwrap();
                    comp.index = (comp.index + 1) % comp.completions.len();
                }
                let candidate = self.cmdline_completion.as_ref().unwrap().completions
                    [self.cmdline_completion.as_ref().unwrap().index]
                    .clone();
                self.apply_completion_candidate(&candidate);
                let total = self.cmdline_completion.as_ref().unwrap().completions.len();
                self.set_message(format!("{}/{}", self.cmdline_completion.as_ref().unwrap().index + 1, total));
                return;
            }
            // Fallback: open command palette.
            let seed = buf.trim_start_matches(':').trim().to_string();
            self.vim.mode = VimMode::Normal;
            self.open_command_picker(&seed);
            return;
        }

        // Shift-Tab in cmdline: open picker with completion candidates.
        if self.vim.mode == VimMode::Cmdline && key == KeyEvent::BackTab {
            if let Some(comp) = self.cmdline_completion.take() {
                self.vim.mode = VimMode::Normal;
                let items: Vec<PickerItem> = comp
                    .completions
                    .iter()
                    .map(|c| PickerItem {
                        label: c.clone(),
                        action: PickerAction::RunCmd(format!("e {}", c)),
                    })
                    .collect();
                self.picker = Some(PickerState::new("Path Completion", items));
                self.set_message(None);
                return;
            }
        }

        // Any non-Tab key in cmdline: clear completion state.
        if self.vim.mode == VimMode::Cmdline && self.cmdline_completion.is_some()
            && key != KeyEvent::Tab && key != KeyEvent::BackTab
        {
            self.cmdline_completion = None;
        }
```

- **Step 4: Verify it compiles**

Run: `cargo build -p ruster-tui 2>&1 | tail -5`
Expected: clean compile. Check that `BackTab` is a valid `KeyEvent` variant (it is in crossterm).

- **Step 5: Commit**

```bash
git add crates/ruster-tui/src/app.rs
git commit -m "feat: implement Tab cycling and Shift-Tab picker for :e path completion"
```

---

### Task 5: Add `BackTab` import if needed

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` (imports)

- **Step 1: Check if `BackTab` is imported**

Run: `grep -n 'BackTab\|use crossterm' crates/ruster-tui/src/app.rs | head -10`

If `BackTab` is not in scope, the crossterm `KeyCode::BackTab` needs to be mapped to a `ruster_key::KeyEvent`. Check how `crossterm_to_ruster_key` maps keys.

- **Step 2: Add mapping if needed**

Find the `crossterm_to_ruster_key` function and add:

```rust
            crossterm::event::KeyCode::BackTab => ruster_key::KeyEvent::BackTab,
```

Check `crates/ruster-core/src/key.rs` for the `KeyEvent` enum to see if `BackTab` exists, or if it needs to be added.

- **Step 3: Verify it compiles**

Run: `cargo build -p ruster-tui 2>&1 | tail -5`
Expected: clean compile.

- **Step 4: Commit**

```bash
git add crates/ruster-tui/src/app.rs crates/ruster-core/src/key.rs
git commit -m "feat: add BackTab key mapping for Shift-Tab path completion"
```

---

### Task 6: Handle Enter to accept completion and Esc to cancel

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` (handle_key, in Cmdline mode)

- **Step 1: Handle Enter in completion mode**

Find the existing Enter handler in `VimMode::Cmdline` (in the vim layer at `crates/ruster-core/src/vim/mod.rs` line 198). The `Action::CmdlineResult` is emitted and handled at app.rs line 1736. No changes needed for Enter — when the user presses Enter, `parse_cmdline` will parse the completed `:e <full-path>` and `apply_cmd` will open it. The `cmdline_completion` state will be cleared by the next keypress logic.

However, we should clear completion state when the cmdline result is processed. Add in the `Action::CmdlineResult` handler (line 1736):

```rust
        Action::CmdlineResult(cmd) => {
            self.cmdline_completion = None; // clear completion state
            self.message = None;
            self.message_time = None;
            match self.parse_cmdline(&cmd) {
                Ok(a) => self.apply_cmd(a),
                Err(e) => self.set_message(e),
            }
        }
```

- **Step 2: Handle Esc in completion mode**

The existing Esc handler in the vim layer (`crates/ruster-core/src/vim/mod.rs` line 193) clears the cmdline and returns to Normal. We need to clear `cmdline_completion` when Esc is processed. Add at the beginning of the `VimMode::Cmdline` Esc handling in vim/mod.rs — but since we don't have direct access there, handle it in `handle_key` instead. When `vim.mode` transitions from Cmdline to Normal, clear the state.

A simpler approach: add a guard at the top of `handle_key` after `self.vim.handle()`:

```rust
        // If we just left cmdline mode, clear completion state.
        if self.cmdline_completion.is_some() && self.vim.mode != VimMode::Cmdline {
            self.cmdline_completion = None;
            self.set_message(None);
        }
```

Place this right after the `self.vim.handle(key, ...)` call (around line 1575).

- **Step 3: Verify it compiles**

Run: `cargo build -p ruster-tui 2>&1 | tail -5`
Expected: clean compile.

- **Step 4: Commit**

```bash
git add crates/ruster-tui/src/app.rs
git commit -m "feat: clear cmdline completion state on Enter/Esc/leave"
```

---

### Task 7: Wire up welcome screen hints

**Files:**
- Modify: `crates/ruster-tui/src/widgets.rs:604`
- Modify: `crates/ruster-render-raylib/src/lib.rs:375`

- **Step 1: Update TUI welcome screen**

In `crates/ruster-tui/src/widgets.rs` line 604, change:

```rust
            (":e path/to/file", "Open File"),
```

to:

```rust
            (":e <path>", "Open file (Tab to complete)"),
```

- **Step 2: Update Raylib welcome screen**

In `crates/ruster-render-raylib/src/lib.rs` line 375, change:

```rust
(":e path/to/file", "Open File")
```

to:

```rust
(":e <path>", "Open file (Tab to complete)")
```

- **Step 3: Verify it compiles**

Run: `cargo build -p ruster-tui -p ruster-render-raylib 2>&1 | tail -5`
Expected: clean compile.

- **Step 4: Commit**

```bash
git add crates/ruster-tui/src/widgets.rs crates/ruster-render-raylib/src/lib.rs
git commit -m "chore: update welcome screen hints for :e completion"
```

---

### Task 8: Add `:e` and `:edit` to command palette

**Files:**
- Modify: `crates/ruster-tui/src/app.rs:563-586` (PALETTE_COMMANDS)

- **Step 1: Verify palette entries**

Already added in Task 1 Step 3. Verify they're present:

```rust
    ("e", "open file by path"),
    ("edit", "open file by path (alias)"),
```

- **Step 2: Commit (if not already done)**

```bash
git diff --cached crates/ruster-tui/src/app.rs
# If changes are already committed in Task 1, skip. Otherwise:
git add crates/ruster-tui/src/app.rs
git commit -m "chore: add :e/:edit to command palette"
```

---

### Task 9: Integration test

**Files:**
- Test: manual or integration test

- **Step 1: Build and run the editor**

```bash
cargo run -p ruster-tui
```

- **Step 2: Test `:e` with a relative path**

Type `:e cr<Tab>` — should auto-complete to `crates/` (directory with trailing `/`).
Type `:e crates/<Tab>` — should show contents of `crates/`.
Type `:e crates/ruster-core/src/<Tab>` — should show files in that directory.

- **Step 3: Test Shift-Tab**

Type `:e cr<Tab>`, then `<S-Tab>` — should open a picker with the completion candidates.

- **Step 4: Test bare `:e`**

Type `:e<Enter>` — should open the file picker.

- **Step 5: Test Esc cancels**

Type `:e cr<Tab>`, then `<Esc>` — should clear completion and return to Normal mode.

- **Step 6: Test Enter accepts**

Type `:e cr<Tab><Tab><Enter>` — should open the second matching file.

- **Step 7: Test `~` expansion**

Type `:e ~/D<Tab>` — should show files in `~/` starting with `D`.

- **Step 8: Run existing tests**

```bash
cargo test --workspace
```

Expected: all existing tests pass (no regressions).

- **Step 9: Commit final state**

```bash
git add -A
git commit -m "feat: :e/:edit command with Tab path completion"
```

---

### Task 10: Add cmdline completion settings (optional)

**Files:**
- Modify: `crates/ruster-lua/src/schema.rs:248-249` (add cmdline group)
- Modify: `crates/ruster-lua/src/config.rs:382-436` (Config struct)
- Modify: `crates/ruster-lua/src/config.rs:444-481` (to_settings)

This task adds configurable trigger keys for cmdline completion. Skip if you want to ship with hardcoded Tab/Shift-Tab.

- **Step 1: Add schema entries**

In `crates/ruster-lua/src/schema.rs`, add a new `cmdline` group after the `dired` section (line 248):

```rust
    // --- cmdline ---
    add("cmdline", "complete_trigger", "Complete key", Enum(&["tab"]), e("tab"), "Key to cycle path completions in :e");
    add("cmdline", "picker_trigger", "Picker key", Enum(&["shift-tab"]), e("shift-tab"), "Key to open completion picker in :e");
```

- **Step 2: Add Config fields**

In `crates/ruster-lua/src/config.rs`, add to the `Config` struct after `dired_show_hidden` (line 432):

```rust
    /// Key to cycle path completions in `:e` (currently only "tab").
    pub cmdline_complete_trigger: String,
    /// Key to open completion picker in `:e` (currently only "shift-tab").
    pub cmdline_picker_trigger: String,
```

- **Step 3: Add to `to_settings()`**

In `Config::to_settings()`, add after the `dired_show_hidden` entry (line 478):

```rust
            (("cmdline", "complete_trigger"), Enum(self.cmdline_complete_trigger.clone())),
            (("cmdline", "picker_trigger"), Enum(self.cmdline_picker_trigger.clone())),
```

- **Step 4: Set defaults in Config loading**

Find where `Config` is constructed from Lua (search for `dired_show_hidden` in config.rs) and add defaults:

```rust
            cmdline_complete_trigger: "tab".to_string(),
            cmdline_picker_trigger: "shift-tab".to_string(),
```

- **Step 5: Verify it compiles**

Run: `cargo build -p ruster-tui 2>&1 | tail -5`
Expected: clean compile.

- **Step 6: Commit**

```bash
git add crates/ruster-lua/src/schema.rs crates/ruster-lua/src/config.rs
git commit -m "feat: add cmdline completion trigger key settings"
```
