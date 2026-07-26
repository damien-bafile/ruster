# Phase 5: IDE & Debugging Tools — Implementation Plan

> **For agentic workers:** implement task-by-task; steps use checkbox (`- [ ]`) syntax.
> Mark a step `- [x]` only once its code compiles and its tests pass.

**Design spec:** [2026-07-26-phase5-ide-tools-design.md](../specs/2026-07-26-phase5-ide-tools-design.md)

**Goal:** Run builds/tests/tasks and surface results (quickfix + gutter signs), a file
sidebar, project/workspace switching, and a DAP debugger — reusing the terminal, picker,
gutter, dired, and LSP-style threads+channels. No tokio.

## Global constraints

- `ruster-core` stays UI/OS-free; runners/DAP live above it.
- Runners are non-blocking (background thread → `mpsc` → per-frame drain) and degrade
  gracefully when a tool is missing.
- Parsers are unit-tested with captured output; no CI test needs a real toolchain.
- Keep `docs/{config-reference,keybindings}.md` in sync.

## Suggested order

Runners first (fast, visible, build on the Phase-4 terminal), then sidebar/workspaces,
then DAP (largest, isolated). Ordered so each task is independently shippable.

### Task 1: Project root + `ruster.toml` — new `ruster-project` crate

- [ ] Detect the project root by walking up for markers (`.git`, `ruster.toml`,
  `Cargo.toml`, `package.json`); expose `project_root(from)`.
- [ ] Parse optional `ruster.toml` (`[tasks.<name>] cmd/cwd/use_terminal`, `[build] command`,
  `[test] command`) with the `toml` crate into typed structs.
- [ ] Track recent projects (persist under the config dir).
- [ ] Tests: marker walk on a temp dir; `ruster.toml` parse of a fixture string.

### Task 2: Shared quickfix list + gutter sign column ✅

- [x] `QuickfixList { items: Vec<(PathBuf, line, col, msg, severity)>, sel }` with
  next/prev/jump; render via `PickerState` (reuse `PickerAction::OpenLocation`).
- [x] Add a **sign column** to rendering: `ruster_render::SignsView { width, signs: Vec<(u16 line,
  char glyph, Color)> }` on `WindowView`, drawn left of the gutter in both TUI and GUI.
- [x] Route existing LSP diagnostics through the sign column too.
- [x] Commands: `:copen`/`:cnext`/`:cprev` (and `]q`/`[q`); tests for navigation + parsing glue.

### Task 3: Build system runner

- [ ] `:build` (or `SPC c b`) runs `[build].command` (default per project type:
  `cargo build`, `make`, `npm run build`) on a background thread, streaming to a results
  buffer/terminal.
- [ ] Parse `rustc`/`cargo` diagnostics (and a generic `file:line:col: msg`) into the
  quickfix list; jump on select.
- [ ] Tests: parse captured `cargo build` JSON/textual output → quickfix items.

### Task 4: Test runner

- [ ] `:test` (`SPC c t`) runs `[test].command` (default `cargo test`); parse per-test
  pass/fail and file/line.
- [ ] Show **gutter signs** (✓/✗) on the relevant lines and a results picker; failures feed
  the quickfix list.
- [ ] Tests: parse `cargo test`/`libtest` output → per-test results.

### Task 5: Task runner

- [ ] `:task` (`SPC o r`) lists `ruster.toml` tasks in a picker; running one opens the
  embedded terminal on its `cmd` (or a background thread if `use_terminal = false`).
- [ ] Tests: task discovery from a fixture `ruster.toml`.

### Task 6: File-explorer sidebar

- [ ] A persistent side window (`SpecialKind::Sidebar`) with a **tree** dired: expand/
  collapse dirs, open files, create/delete/rename (reuse dired ops).
- [ ] Toggle with `SPC e` / `:Sidebar`; focus/resize like a normal window; follows the
  active file.
- [ ] Tests: tree expand/collapse state over a temp dir.

### Task 7: Project workspaces

- [ ] Set the working root from the detected project; `:projects` picker of recent projects
  switches root (reload sidebar, re-detect build/test).
- [ ] Load project `ruster.toml` settings over the user config.
- [ ] Tests: recent-projects persistence + switching.

### Task 8: Multi-cursor polish

- [ ] Alias `Ctrl-D` to the existing add-cursor-at-next-match (`C-n`); ensure Alt+click in
  the GUI adds a caret. (Engine already supports multi-cursor via `CursorSet`.)
- [ ] Tests: `Ctrl-D` adds a caret at the next match.

### Task 9: DAP debugger — new `ruster-dap` crate

- [ ] Mirror `ruster-lsp`: transport (stdio JSON-RPC), manager (one adapter per language:
  `lldb-dap`/`debugpy`/`gdb`), and result parsers (stopped, stackTrace, scopes, variables).
- [ ] Breakpoints as gutter signs (toggle with `SPC d b`); launch/continue/step
  (`SPC d c` / `SPC d o` / `SPC d i`); a variables/watch panel window.
- [ ] Non-blocking poll each frame like LSP; graceful when the adapter is missing.
- [ ] Tests: parse captured DAP messages (initialize, stopped, stackTrace, variables).

### Task 10: Config, docs, CI

- [ ] Schema settings: `build.command`, `test.command`, `dap.adapter`, sidebar defaults.
- [ ] Document commands/keys in `docs/{config-reference,keybindings}.md`.
- [ ] `cargo test`/`clippy` green across the workspace; open the Phase 5 PR (likely split:
  runners+quickfix, sidebar+workspaces, DAP).
