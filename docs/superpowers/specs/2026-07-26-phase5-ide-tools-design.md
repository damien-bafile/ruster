# Phase 5: IDE & Debugging Tools — Design Spec

**Goal:** The "VS Code / IntelliJ" power tier — run builds/tests/tasks and surface their
results in the editor, browse the project in a sidebar, switch between projects, and debug
via DAP. Built on what already exists: the embedded terminal (`ruster-terminal`), the
floating picker + jump-to-location (`open_diagnostics_picker`), the gutter
(`ruster_render::GutterView`), dired (`ruster_core::dired`), the window tree
(`ruster_core::windows`), and the LSP client's **std threads + channels JSON-RPC** pattern
(`ruster-lsp`) — no tokio.

## Components

| Area | Approach | Reuses |
| :--- | :--- | :--- |
| **Project / workspace** | Detect the project root from markers (`.git`, `ruster.toml`, `Cargo.toml`, `package.json`). Load optional `ruster.toml` (tasks, build/test overrides). Track recent projects for quick switch. | `ruster_config_dir`, `dirs`, a `toml` dep |
| **Build system** | Run the project's build (`cargo build`, `make`, `npm run build`) on a background thread, stream output, and parse compiler diagnostics into a **quickfix list**. | `ruster-terminal` reader-thread pattern, `mpsc`, the picker |
| **Quickfix list** | A reusable list of `(path, line, col, message, severity)` with next/prev/jump. Rendered as a picker; `:copen`/`]q`/`[q` navigate. | `PickerState` + `PickerAction::OpenLocation` |
| **Test runner** | Discover + run tests (`cargo test`, `cargo nextest`), parse pass/fail per test, and show **gutter signs** on test lines plus a results picker. | Gutter (extended with a sign column), background thread, quickfix |
| **Task runner** | User tasks in `ruster.toml` (`[tasks.build] cmd = "…"`). Run in the **embedded terminal** (`:term`) or a background thread; list via a picker. | `ruster-terminal`, picker |
| **File-explorer sidebar** | A persistent side window backed by a **tree** dired variant (expand/collapse dirs, create/delete/rename). Toggle with a keybinding; focus like any window. | `ruster_core::dired`, `WindowTree`, `SpecialKind` |
| **DAP debugger** | New `ruster-dap` crate: a Debug Adapter Protocol client over stdio (JSON-RPC), managing `lldb-dap`/`gdb`/`debugpy`. Breakpoints (gutter signs), stepping, stack frames, variable/watch panels. | `ruster-lsp` architecture (transport, manager, results), gutter signs |
| **Multi-cursor** | Already native (`CursorSet`, `C-n` add-cursor-at-next-match). Add `Ctrl-D` as an alias and ensure Alt+click in the GUI adds a caret. | `ruster_core::cursor` |

## Architecture notes

- **New crates:** `ruster-project` (root detection + `ruster.toml` + recent projects),
  `ruster-dap` (DAP client). Build/test/task **runners** live in `ruster-tui` (they drive
  terminal/threads + UI), with pure parsing helpers where testable.
- **Runner concurrency** mirrors LSP/`:Rg`: spawn the tool on a background thread, stream
  lines over `mpsc`, drain per frame — never block the render loop.
- **Quickfix + gutter signs** are the shared surfaces. Add a **sign column** to
  `GutterView` (or a parallel `SignsView`) carrying `(line, glyph, color)` so diagnostics,
  test results, and DAP breakpoints all render through one path in both frontends.
- **Config:** extend the Phase-5 settings groups via the existing schema
  (`ruster-lua/schema.rs`) — e.g. `build.command`, `test.command`, `dap.adapter`.
- **DAP** is the largest, most isolated piece; it comes last and can ship independently.

## Constraints

- `ruster-core` stays UI/OS-free; runners and DAP live above it.
- Every runner degrades gracefully when its tool is missing (message, never a crash/hang).
- Parsing (compiler errors, test output, `ruster.toml`) is unit-tested with inline fixtures;
  no runner test may require a real toolchain in CI (feed captured output to the parser).
- Keep `docs/{config-reference,keybindings}.md` in sync.
- Cross-platform: prefer the terminal/thread abstraction so ConPTY/forkpty differences stay
  inside `ruster-terminal`.
