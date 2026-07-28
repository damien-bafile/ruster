# Phase 5: Workspace, Build, Debug — Design Spec

**Goal:** Complete the remaining Phase 5 IDE features: build/test/task runner UX, multi-cursor
keybindings, project workspace management, and a full DAP debugger.

**Status:** Approved; ready for implementation planning.

---

## Track A — Build / Test / Task Runner UX

### Status quo

`spawn_shell_command`, `parse_build_diagnostics`, `parse_test_results` exist in `runner.rs`.
`CmdAction` has `Build`, `Test`, `TaskPicker`, `QuickfixOpen`, `QuickfixNext`, `QuickfixPrev`.
`App` fields `runner_rx`, `runner_buf`, `runner_root`, `runner_output`, `runner_kind (Build | Test | Task)`.
`ruster-project` detects default build/test commands and parses `ruster.toml` tasks.
`result_signs` already renders `✓`/`✗` after test runs.

### Keybindings

| Key | Action |
|-----|--------|
| `F7` | Run build (`:make` / `:build`) |
| `F6` | Run tests (`:test`) |
| `F9` | Open task picker (`:task`) |

### Statusline integration

When `self.runner_kind.is_some()` and the runner is active, the statusline shows a spinner
(`"Building..."` / `"Testing..."` / `"Running...")` in place of the mode indicator.

Implementation: a new method `App::runner_status_text() -> Option<String>` that returns a
status message when a runner is active; the statusline renderer calls it.

### Quickfix integration

On build completion (`drain_build_runner` sees exit):
- If exit code != 0 and quickfix has items, auto-open quickfix picker.
- If exit code == 0, show `"Build successful"` in messages.

No changes to the runner internals — just wire the existing hooks.

---

## Track B — Multi-cursor Keybindings

### Status quo

`CursorSet` supports multiple cursors with `add_cursor()`, `clear_extra()`, `iter_heads()`.
`EditSession::apply_edit_multi` handles edits at multiple cursors with offset correction.
`Action::AddCursor(usize)` and `Action::ClearExtraCursors` exist.
What's missing: the keybindings to trigger them.

### Bindings

| Key | Action |
|-----|--------|
| `Ctrl+D` in Normal mode | Add cursor at next occurrence of the word under the primary cursor. If no more occurrences, beep/flash. |
| `Alt+click` (mouse event) | Add cursor at the clicked position. Falls back to normal click (move cursor) if click is in a non-editor area. |
| `Ctrl+D` on a selection | Add cursor at next occurrence of the selected text. |
| `Esc` when cursors > 1 | `Action::ClearExtraCursors` (already works via existing command dispatch) |

### Ctrl+D implementation

1. Get word at primary head from the buffer (grapheme cluster boundaries).
2. Search forward from primary head + 1 for next occurrence.
3. If found: `Action::AddCursor(position)`.
4. If not found, wrap to buffer start and search from 0 to primary head.
5. If still not found: no-op (or flash).

Mouse `Alt+click`: extract `(row, col)` from the ratatui `MouseEvent`, convert to buffer
offset via window's screen-to-buffer mapping, `Action::AddCursor(offset)`.

---

## Track C — Project Workspaces UI

### Status quo

`ruster_project::project_root()`, `record_recent()`, `recent_projects()` exist.
`CmdAction::Projects` exists but is not wired.

### :projects picker

When `:projects` (or `:workspaces`) is invoked:
1. Read `recent_projects(state_dir)`.
2. Show in a picker buffer (like `:Files`): one project per line, root path + marker name.
3. Selecting a project: sets `self.project_root`, reloads config, re-scans tree-sitter, updates LSP.
4. `d` on a project: removes it from the recent list.

### Auto-record

Record a project on every meaningful root-anchored action:
- `:e path`, `:Files` → file opened inside a project → record that project
- Sidebar open → record root
- `:term` → record cwd's project

### Startup restore

On startup, if the last project root still exists, re-open it as the active project root.

---

## Track D — DAP Debugger

### Crate

New workspace member `ruster-dap`. Depends on `dap` (sztomi/dap-rs) for protocol types.

```
crates/ruster-dap/src/
  lib.rs             — re-exports
  transport.rs       — JSON-RPC 2.0 framing over stdio (copy ruster-lsp pattern)
  client.rs          — spawn adapter process, send requests, receive events via mpsc
  session.rs         — breakpoints, stack frames, variable references, threads state
  config.rs          — adapter launcher per language
```

### transport.rs

JSON-RPC 2.0 over stdio with `Content-Length` headers (identical framing to LSP).
`read_message<R: Read>(reader) -> Result<ServerMessage>` / `write_message<W: Write>(writer, msg)`.
`ServerMessage` enum: `Response(Response)`, `Event(Event)`, `Request(Request)`.

Reuses the exact pattern from `ruster-lsp/src/transport.rs`.

### client.rs

`DapClient` struct:
- `spawn(config: &DebugAdapterConfig)` — launches the adapter process (`std::process::Command`
  with piped stdin/stdout), spawns a reader thread that calls `transport::read_message` in a loop
  and sends `ServerMessage` over an `mpsc::Sender<ServerMessage>`.
- `initialize()` / `configure()` / `launch()` / `attach()` — the init handshake sequence.
- `request(command, arguments)` — send a request, store pending response callback.
- `send_event(event)` — send a DAP event (debug adapter receives it as a request).
- `poll()` — drain the receiver, return buffered events/responses.
- `shutdown()` — send `disconnect` request, kill process.

Relies on `dap` crate's type enums (`Command`, `Request`, `Response`, `Event`) for the
wire format but does NOT use `dap::Server` (which is for writing adapters, not clients).

### session.rs

`DebugSession` struct — state machine for a single debug session:

```
DebugSession {
    client: DapClient,
    state: SessionState,           // Inactive | Initializing | Running | Paused | Terminated
    breakpoints: HashMap<PathBuf, Vec<Breakpoint>>,
    threads: HashMap<u64, Thread>,
    stack_frames: Vec<StackFrame>,  // current stopped-thread frames
    variable_cache: HashMap<u64, Variable>,  // variablesReference → Variable
}
```

Key methods:
- `start(config)` — spawn, initialize, launch/attach → Running.
- `pause()` / `continue_execution()` — pause/continue.
- `step_over()` / `step_into()` / `step_out()` — stepping.
- `set_breakpoint(path, line)` / `clear_breakpoint(path, line)` — manage source breakpoints.
- `get_stack_frames(thread_id)` — fetch current call stack.
- `get_variables(ref_id)` — fetch children of a variable/scope.
- `evaluate(expr, context)` — evaluate expression (hover/watch).
- `handle_event(event)` — process stopped/continued/exited/terminated events → update state.
- `poll_events()` — drain client's event queue, update state.

### config.rs

`DebugAdapterConfig` per language, detected at debug-start time:

```rust
pub struct DebugAdapterConfig {
    pub name: String,        // e.g. "lldb-vscode"
    pub command: String,     // e.g. "lldb-vscode"
    pub args: Vec<String>,   // e.g. ["--source-path", "."]
    pub init_commands: Vec<serde_json::Value>,  // extra init sequence
}
```

Defaults (same pattern as `ruster-lsp/src/registry.rs`):
- Rust/LLDB: `lldb-vscode` (from `rustup` or Homebrew)
- C++/GDB: `gdb` via a gdb-to-DAP bridge or `Debugger for GDB` adapter
- Python: `debugpy` (pip-installable)

### UI in ruster-tui

New module `crates/ruster-tui/src/debug.rs`. Rendered as overlays and gutter
modifications — does NOT create new windows or splits (keeps integration lightweight).

**Debug Toolbar (floating bar below the statusline):**

```
[ ▶ Continue] [ ⤵ Step Over] [ ↘ Step Into] [ ↖ Step Out] [ 🔄 Restart] [ ⏹ Stop]
```

Only visible when a debug session is active (`App.debug_session.is_some()`).
Buttons are keyboard-activatable with `Tab`/`Shift+Tab` focus or bound to F-keys.

**Breakpoint gutter:**

In the line-number gutter, a red `●` on lines with breakpoints, a dim `○` on disabled ones.
`Ctrl+F8` toggles a breakpoint on the current line.

**Stack panel (right-side overlay or pinned buffer):**

When the session is paused, shows the call stack:
```
▶ main() at src/main.rs:42
  handle_request() at src/server.rs:120
  parse_payload() at src/parser.rs:85
```

Selected frame is highlighted. Enter/click jumps to that source location.

**Variables panel (below stack panel or second overlay):**

Shows scopes and variables for the selected stack frame:
```
  Locals
    name: "Alice" (String)
    age: 42 (i32)
    items: Vec<Item> (length: 3)  → expandable
  Registers / Statics (if available)
```

Expandable variables send `variables` request on first expand, cache result.

**Hover evaluation:**

In Normal mode, pressing `K` while a debug session is paused evaluates the expression under
the cursor via DAP's `evaluate` request. Result shown in a floating popup.

### Event flow (stopped → pause)

1. DAP adapter sends `StoppedEvent { reason, thread_id }`.
2. `DebugSession::handle_event` → state = Paused, fetch stack frames for thread_id.
3. Stack panel updates.
4. If the stop reason is `breakpoint`, the editor scrolls to the breakpoint line.
5. User inspects stack/variables, steps, or continues.

### Keybindings

| Key | Action |
|-----|--------|
| `Ctrl+F8` | Toggle breakpoint on current line |
| `F5` | Start/continue debugging (if no session → prompt for config; if paused → continue) |
| `Shift+F5` | Stop debugging |
| `F10` | Step over |
| `F11` | Step into |
| `Shift+F11` | Step out |
| `K` (Normal) | Hover evaluate expression under cursor |

---

## Cross-cutting

### Statusline integration (build + debug)

The statusline already shows mode, filename, cursor position, LSP status. Two new sections:
- **Runner status**: "Building..." / "Testing..." / "Running Task X" with spinner.
- **Debug status**: "[Running]" / "[Paused at breakpoint]" / thread info.

### Config

New Lua options under `ruster.config.debug`:

```toml
[debug]
adapter_overrides = {}  # per-language adapter command/args overrides
```

No config needed for tracks A, B, C — they use existing options or sensible defaults.
