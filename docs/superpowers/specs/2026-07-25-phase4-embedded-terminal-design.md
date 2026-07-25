# Phase 4: The Embedded Terminal — Design Spec

**Goal:** A full cross-platform terminal running inside a ruster window — spawn a
shell in a PTY, parse its VT100/ANSI output into a grid, render that grid in both
the TUI and GUI frontends, and forward keystrokes back to the shell. Unix uses
`forkpty`; Windows uses ConPTY. Both go through one dependency API so ruster's own
code stays platform-neutral.

## Tech

| Concern | Choice | Notes |
| :--- | :--- | :--- |
| PTY backend | [`portable-pty`](https://crates.io/crates/portable-pty) | One API; ConPTY on Windows (≥ Win10 1809), `forkpty` on Unix. |
| VT state machine | [`alacritty_terminal`](https://crates.io/crates/alacritty_terminal) | Parses raw bytes → an in-memory `Term` grid of cells (fg/bg/flags). Cross-platform. |
| Threading | std thread + `mpsc` | A blocking reader thread pumps PTY bytes into the parser; the UI polls a grid snapshot each frame. Matches the Phase 2 `:Rg` / Phase 3 LSP non-blocking model — no tokio. |

## Architecture

```
                    ┌──────────────────────── ruster-terminal (new crate) ───────────┐
                    │                                                                 │
  shell (pwsh/sh) ──┤  PtySession ── reader thread ──▶ Term<VT parser>  (Mutex)       │
        ▲           │      │ writer                         │ snapshot()              │
        │ bytes     │      │                                ▼                         │
        └───────────┤  write_input(bytes)            TermGrid { rows: Vec<TermRow> }  │
                    └───────────────────────────────────────┬─────────────────────────┘
                                                            │ each frame
   App (ruster-tui) ── HashMap<BufferId, TerminalSession> ──┤ poll + build view
                                                            ▼
                              ruster-render::TermGridView  (cells: fg/bg/attrs)
                                       │                         │
                             TuiRenderer (ratatui)      RaylibRenderer (quads + glyphs)
```

- **`ruster-terminal` crate.** Owns `TerminalSession`: the PTY master + writer, the
  child handle, a reader thread, and a `Mutex<Term>` (the alacritty grid). Public API:
  `spawn(shell, size)`, `write_input(&[u8])`, `resize(cols, rows)`, `snapshot() -> TermGrid`,
  `is_running()`. No UI or app types leak in — it depends only on `portable-pty` and
  `alacritty_terminal`.
- **Terminal buffers.** A terminal occupies a window like any buffer, via a new
  `DocKind` / `SpecialKind::Terminal`. The `Document` is a placeholder (no rope text);
  the live content is the `TerminalSession` keyed by `BufferId` on the `App`.
- **Render contract.** `ruster-render` gains a `TermGridView` (rows of cells, each with
  fg, bg, and attribute flags) and `WindowView.terminal: Option<TermGridView>`. When set,
  a renderer draws the grid instead of styled text lines. This keeps the frontends dumb —
  they render whatever the `FrameState` describes.
- **Input.** In a focused terminal window, keys are translated to their byte/escape
  sequences (`Enter`→`\r`, `Ctrl-C`→`0x03`, arrows→`\x1b[A`…) and written to the PTY.
  A small key-encoder lives in `ruster-terminal` so both frontends share it.
- **Resize.** When a terminal window's geometry changes, the App calls
  `session.resize(cols, rows)` (→ `portable-pty` `resize()` → `ResizePseudoConsole` on
  Windows / `TIOCSWINSZ` on Unix) and the parser is told the new dimensions. No signals.

## Cross-platform / Windows

- ConPTY requires **Windows 10 1809+**; older Windows runs the editor but not the embedded
  terminal (documented boundary). CI `windows-latest` (Server 2022) has ConPTY.
- **Default shell** is the one new user-facing platform choice: `terminal.shell` config
  (Lua). Default: `$SHELL` → `/bin/sh` on Unix; `%COMSPEC%` → `cmd.exe` on Windows.
- Program output is CRLF on Windows — that is *terminal grid* content handled by the VT
  parser and never touches ruster's buffer LF-normalization. The two are unrelated.
- Effectively **zero new `#[cfg]` branching in ruster's own code**: `portable-pty` owns the
  ConPTY/forkpty split.

## Constraints

- The terminal is **non-blocking**: the reader thread never blocks the render loop; the UI
  reads a cheap grid snapshot each frame. A dead shell degrades to a static grid + message,
  never a crash or hang (see the Phase 3 "TUI hangs on exit" fix — sessions must be torn
  down cleanly on quit).
- `ruster-core` must **not** depend on `ruster-terminal` (the engine stays UI/OS-free).
- Scrollback is bounded and configurable (`terminal.scrollback`, default 10 000 lines).
- Keep `docs/config-reference.md`, `docs/lua-api.md`, `docs/keybindings.md`, and
  `docs/windows.md` in sync.
- All existing tests keep passing; new terminal tests must not require an interactive TTY —
  spawn a deterministic command (`printf`/`echo`, or `cmd /c echo` on Windows) and assert
  the grid contents.
