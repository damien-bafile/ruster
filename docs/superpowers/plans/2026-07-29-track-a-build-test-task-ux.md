# Build/Test/Task UX — Implementation Plan

> **Status:** delivered; boxes resolved 2026-08-07.
>
> This plan was executed but never ticked as it went, leaving 13 boxes
> that read as outstanding work. They are **plain bullets now, not back-ticked**:
> a box ticked long after the fact asserts a verification that did not happen,
> and this tree has already been bitten by exactly that. The bullets stand as a
> record of what was built.
>
> Evidence it shipped: all 12 identifiers this plan names in backticks exist in
> the tree, and `:build`/`:test`/`:task` parse and route to the quickfix list, which `:Trouble` renders (`trouble-tui.txt`).


> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Wire keybindings, statusline integration, and auto-quickfix for the existing build/test/task runner.

**Architecture:** Pure additions to `ruster-tui` — the runner infrastructure already exists in `runner.rs` and `App`. Add F-key dispatch to `handle_key()`, a statusline hook for active runner text, and auto-open quickfix on build errors.

**Tech Stack:** Rust, ruster-tui (app.rs, runner.rs, renderer.rs, widgets.rs), ruster-render (FrameState)

## Global Constraints

- Follow existing App dispatch patterns (sequential `if let` / `else if` chain in `handle_key`)
- Use the existing `LuaKey::F(u8)` type for F-key definitions
- Do NOT add new crate dependencies
- All new Lua config is out of scope (no config needed)

---

### Task 1: Wire F7/F6/F9 key dispatch

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` (handle_key section, before Vim state dispatch)

**Interfaces:**
- Consumes: `self.run_build()`, `self.run_test()`, `self.open_task_picker()` (all exist already)
- Produces: None (adds side effects in key dispatch)

- **Step 1: Add F-key dispatch to handle_key**

In `app.rs`, in the `handle_key` method, before the "macro recording" / "Lua keymap hook" / "Vim state" dispatchers, add an F-key check block:

```rust
// F-key dispatch for build/test/task.
match ck.code {
    KeyCode::F(7) => {
        self.run_build();
        return true;
    }
    KeyCode::F(6) => {
        self.run_test();
        return true;
    }
    KeyCode::F(9) => {
        self.open_task_picker();
        return true;
    }
    _ => {}
}
```

Note: match on `crossterm::event::KeyEvent`'s `code` field. The parameter type is `crossterm::event::KeyEvent` (passed as `ck`). Place this after the `Ctrl-w prefix` and Ctrl+h/j/k/l blocks, before the "K LSP hover" check.

- **Step 2: Build to verify**

```
cargo build -p ruster-tui 2>&1 | tail -5
```
Expected: clean build

- **Step 3: Commit**

```
git add crates/ruster-tui/src/app.rs
git commit -m "feat(build): wire F7/F6/F9 for build/test/task"
```

---

### Task 2: Add runner status text to App

**Files:**
- Modify: `crates/ruster-tui/src/app.rs`

**Interfaces:**
- Produces: `App::runner_status_text() -> Option<&'static str>`

- **Step 1: Add method to App**

Add after `run_build()` / `run_test()` region:

```rust
/// Returns a status message when a build/test/task runner is active, or None.
pub fn runner_status_text(&self) -> Option<&'static str> {
    self.runner_kind.as_ref().map(|kind| match kind {
        Build => "Building...",
        Test => "Testing...",
        Task => "Running Task...",
    })
}
```

Where `Build`, `Test`, `Task` are the variants of `RunnerKind` (check exact type name in runner.rs — use the existing enum).

- **Step 2: Build to verify**

```
cargo build -p ruster-tui 2>&1 | tail -5
```

- **Step 3: Commit**

```
git add crates/ruster-tui/src/app.rs
git commit -m "feat(build): add runner_status_text to App"
```

---

### Task 3: Show runner status in statusline

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` (render method, statusline section)

**Interfaces:**
- Consumes: `app.runner_status_text()`
- Modifies: `FrameState.windows[].statusline.center` or `.left`

- **Step 1: Locate the statusline text construction in `App::render()`**

Find the section that builds `StatuslineView`. In the `render()` method (~line 3027), the statusline content is built per window. The center section shows the buffer name. When a runner is active, prepend the runner status.

Look for code like:
```rust
statusline_center = format!(" {} ", win_name);
```

Replace with:
```rust
let runner_msg = self.runner_status_text();
let statusline_center = if let Some(msg) = runner_msg {
    format!(" {} {} ", msg, win_name)
} else {
    format!(" {} ", win_name)
};
```

- **Step 2: Build to verify**

```
cargo build -p ruster-tui 2>&1 | tail -5
```

- **Step 3: Run existing tests**

```
cargo test -p ruster-tui 2>&1 | tail -5
```
Expected: all passing

- **Step 4: Commit**

```
git add crates/ruster-tui/src/app.rs
git commit -m "feat(build): show runner status in statusline"
```

---

### Task 4: Auto-open quickfix on build errors

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` (in `finish_build` or equivalent)

**Interfaces:**
- Consumes: `self.quickfix.is_empty()`, `self.open_quickfix()`
- Modifies: `finish_build` callback

- **Step 1: Find where finish_build is defined**

Search for `fn finish_build` or `finish_test` in app.rs. The runner drain/completion logic calls a finish method after the process exits.

In the build completion path (where `QuickfixList::new(items)` is set), add after the quickfix assignment:

```rust
if !self.quickfix.is_empty() {
    self.open_quickfix();
}
```

- **Step 2: Build to verify**

```
cargo build -p ruster-tui 2>&1 | tail -5
```

- **Step 3: Commit**

```
git add crates/ruster-tui/src/app.rs
git commit -m "feat(build): auto-open quickfix on build errors"
```
