# Phase 4: The Embedded Terminal — Implementation Plan

> **For agentic workers:** implement task-by-task; steps use checkbox (`- [ ]`) syntax.
> Mark a step `- [x]` only once its code compiles and its tests pass.

**Design spec:** [2026-07-25-phase4-embedded-terminal-design.md](../specs/2026-07-25-phase4-embedded-terminal-design.md)

**Goal:** A cross-platform embedded terminal — spawn a shell in a PTY, parse VT output
into a grid, render it in TUI + GUI, forward input, and resize. `portable-pty` for the
PTY (ConPTY/forkpty), `alacritty_terminal` for the VT state machine, std threads +
channels for concurrency (no tokio).

## Global constraints

- Non-blocking: reader thread pumps PTY → parser; UI reads a snapshot per frame.
- `ruster-core` must not depend on `ruster-terminal`.
- A dead/exited shell degrades gracefully (message, no crash/hang); sessions torn down on quit.
- New terminal tests use a deterministic command, never an interactive TTY.
- Keep `docs/{config-reference,lua-api,keybindings,windows}.md` in sync.

---

### Task 1: `ruster-terminal` crate — PTY + VT grid core — DONE

**Files:** Created `crates/ruster-terminal/{Cargo.toml,src/lib.rs,src/keys.rs}`; added to
workspace members. Deps: `portable-pty 0.9`, `alacritty_terminal 0.26` (pulls `vte 0.15`).

- [x] **Step 1:** Scaffold the crate; `cargo add portable-pty alacritty_terminal`;
  register in the workspace `Cargo.toml`.
- [x] **Step 2:** `TerminalSession::spawn(program, args, cols, rows)` opens a PTY, spawns
  the child, keeps the master + a writer, and starts a reader thread that feeds bytes into
  an `alacritty_terminal::Term` behind `Arc<Mutex<…>>` (the `vte::ansi::Processor` lives on
  the reader thread; only the grid is shared).
- [x] **Step 3:** `write_input(&[u8])`, `resize(cols, rows)`, `is_running()`, and
  `snapshot() -> TermGrid` (`TermCell { c, fg, bg, attrs }`), converting alacritty
  `Color`/`Flags` into render-neutral `TermColor`/`TermAttrs` (16-color + 256-cube +
  grayscale palette). `Drop` kills the child and joins the reader. `default_shell()` picks
  `$SHELL`/`/bin/sh` or `%COMSPEC%`/`cmd.exe`.
- [x] **Step 4:** Key encoder in `keys.rs`: `encode_key(Key, Mods) -> Vec<u8>` (Enter/Tab/
  Backspace/Esc, Ctrl-letters → C0, Alt → ESC prefix, arrows, Home/End/PgUp/PgDn/Del/Ins,
  UTF-8 printables).
- [x] **Step 5:** Tests (7 passing) — spawn `sh -c 'printf hello_ruster'` (Windows
  `cmd /c echo …`), poll the grid until the text appears; dimensions; palette mapping;
  `encode_key` for plain/ctrl/alt/special keys. Clippy clean; workspace builds.

### Task 2: Render contract — `TermGridView` — DONE

**Files:** Modified `crates/ruster-render/src/lib.rs`.

- [x] **Step 1:** Added `TermCellView { c, fg, bg, bold, italic, underline, inverse }` and
  `TermGridView { cols, rows, cells, cursor }` (reusing `Color` for fg + bg).
- [x] **Step 2:** Added `WindowView.terminal: Option<TermGridView>`; when `Some`, renderers
  draw the grid and ignore `lines`/`gutter`/`selection`. Updated all construction sites.
- [x] **Step 3:** Unit test constructing a `WindowView` with a terminal grid (9 tests pass).

### Task 3: App integration — open, poll, focus — DONE

**Files:** Modified `crates/ruster-tui/src/app.rs`; `crates/ruster-core/src/document.rs`
(`SpecialKind::Terminal`, read-only).

- [x] **Step 1:** `terminals: HashMap<BufferId, TerminalSession>` + `terminal_focused` on
  `App`; `:term`/`:terminal` open a terminal buffer via `open_terminal()` (spawns
  `default_shell()`), added to command completion. (Leader binding deferred to Task 6.)
- [x] **Step 2:** Each frame the render loop populates `WindowView.terminal` from
  `session.snapshot()` (via `to_term_grid_view`) for terminal buffers.
- [x] **Step 3:** Focused terminals route keys through `term_key_from_crossterm` →
  `encode_key` → `write_input`; `Ctrl-\` defocuses, `i`/`a`/Enter re-focus.
- [x] **Step 4:** The render loop calls `session.resize(cols, rows)` to the window's text
  area each frame (no-op when unchanged).
- [x] **Step 5:** `self.terminals.clear()` on every run-loop exit (kills children + joins
  reader threads via `Drop`), beside the existing `lsp.shutdown_all()`.
- [x] **Step 6:** Tests (3, 87 total pass) — `:term` parses + opens a terminal; typing
  `ping⏎` into a `cat` PTY is echoed into the grid; `Ctrl-\`/`i` toggle focus. Clippy clean.

> **Note:** the grid data path is complete but the TUI/GUI widgets don't yet *draw*
> `WindowView.terminal` (Tasks 4–5), so a terminal buffer currently shows placeholder text.

### Task 4: TUI rendering (ratatui) — DONE

**Files:** Modified `crates/ruster-tui/src/{widgets.rs,renderer.rs}`.

- [x] **Step 1:** `TerminalWidget` draws `TermGridView` cells with per-cell fg/bg + bold/
  italic/underline (inverse swaps fg/bg) and a block cursor at the grid cursor.
- [x] **Step 2:** The renderer draws `TerminalWidget` in the buffer area when
  `WindowView.terminal` is set, else the usual `BufferWidget`.
- [x] **Step 3:** Widget test asserts a cell's char + RGB fg map through and the cursor
  cell is painted (88 tests pass, clippy clean).

### Task 5: GUI rendering (raylib) — DONE

**Files:** Modified `crates/ruster-render-raylib/src/lib.rs`.

- [x] **Step 1:** When `view.terminal` is set, draw the grid inside the window's scissor
  rect: a background quad per cell (inverse swaps fg/bg), the glyph in fg (falling back to
  the theme color for `Color::Default`), and a block cursor on the active window. The buffer
  path (gutter/scroll/selection/multi-cursor) is bypassed via an `if/else`.
- [x] **Step 2:** `cargo check`/`clippy` clean; full workspace (incl. `ruster-bin`) builds
  and links. Live visual check deferred to a display (CI gates on build only).

### Task 6: Config, input polish, docs — DONE

**Files:** `crates/ruster-lua/src/{config.rs,runtime.rs}`; `crates/ruster-terminal/src/lib.rs`;
`crates/ruster-tui/src/app.rs`; `docs/{config-reference,keybindings,windows}.md`.

- [x] **Step 1:** `terminal_shell` (flat key; default `$SHELL`/`/bin/sh`, `%COMSPEC%`/
  `cmd.exe`) and `terminal_scrollback` (default 10 000) config; `spawn` takes `scrollback`;
  `open_terminal` honors both.
- [~] **Step 2:** Mouse forwarding / scrollback-scroll keys / leader binding — deferred as
  optional polish (not required for a working terminal).
- [x] **Step 3:** Documented `:term`, the config keys, the terminal keybindings, and the
  Windows/ConPTY constraints (Win10 1809+ floor, default shell) across
  `config-reference.md`, `keybindings.md`, `windows.md`.

### Task 7: CI + release — DONE

- [x] **Step 1:** CI (`rust.yml`) runs `cargo build`/`cargo test` for the whole workspace on
  ubuntu/windows/macos, so `ruster-terminal` + its tests are covered automatically (terminal
  tests OS-gated: `cmd /c echo` on Windows, `cat`/`sh` elsewhere). No workflow change needed.
- [x] **Step 2:** `cargo clippy --workspace` clean; Phase 4 PR opened.
