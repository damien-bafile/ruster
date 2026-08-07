# Phase 10 — Verification

**Status:** complete, 2026-08-07. Harness built, matrix captured, findings
adjudicated, final sweep run. Seven defects were reported and routed to Phase 9 Task
4b — **three of them were mis-diagnosed and have been retracted**, five real
ones (plus one found underneath a false one) are fixed. See
`docs/verification/README.md` for the per-surface status table and for the two
ways a capture lies, which is what produced the false reports.

Phases 0–9 built an editor, and every phase ended with tests that prove the
code does what it claims. None of them proved it **looks** like anything. This
phase captures every user-visible surface in **both** backends — TUI as text,
GUI as PNG — and commits the artifacts so a later session can see what "done"
looked like and a human can check the things a unit test cannot: legibility,
glyph rendering, theme colours, and TUI/GUI agreement.

**This is a check, not a build.** Nothing here should require a code change
unless a capture reveals a defect — in which case that defect is a Phase 9
(cleanup) bug and gets fixed there, then recaptured.

## Global constraints

- **Both backends, every surface.** A surface with only one capture is an
  unfinished verification; the parity rule has been a standing constraint since
  Phase 6, and this is where it gets checked with eyes.
- **Deterministic where possible.** Drive surfaces with an `init.lua` queue
  applied before the first frame. For LSP/debugger-dependent surfaces, use
  `ruster.defer` to let a round-trip settle before the capture fires (unblocked
  by PR #59).
- **Artifacts live in the repo.** `docs/verification/<surface>-tui.txt` and
  `docs/verification/<surface>-gui.png`, committed. The matrix below links
  each.
- **No test may require a live LSP/debugger/network.** Where a surface needs
  one, capture the most static faithful approximation and mark the row with the
  manual step, or seed the fixture so it is deterministic.

## Capture harness

Two additions to what this plan assumed. **Key injection**, which the plan
worked around by listing key-driven rows as manual: `ScriptedRenderer`
(`crates/ruster-render/src/script.rs`) plays a key script through the real
`run_gui` loop headlessly and records every frame, and `scripts/gui-keys.sh`
sends real keystrokes to the raylib window via System Events. And a **frame
digest**, which is the first thing in this project that can assert on what a
frame contains rather than that building it did not panic.

**Files:**
- Create: `scripts/verify-capture.sh`
- Create: `docs/verification/README.md` — one paragraph per surface: how it was
  driven, what to look for, and any manual caveat
- Modify: `justfile` — a `verify` recipe calling the script

**Interfaces:**
- Produces: `scripts/verify-capture.sh <surface> [args]` which writes
  `docs/verification/<surface>-tui.txt` and `docs/verification/<surface>-gui.png`.
  TUI via tmux `capture-pane -p`; GUI via the gui-check recipe (XDG_CONFIG_HOME
  + init.lua queue + deferred `:screenshot`), gated on the screen being unlocked
  (`ioreg -n Root -d1 -a | grep CGSSessionScreenIsLocked` must be absent).

- [x] **Step 1: Write the TUI half.** Launch `ruster <file>` under a fresh
      tmux session with an `init.lua` in a temp `XDG_CONFIG_HOME` that queues
      the drive commands, wait ~1s for the first frame, `tmux capture-pane -p`,
      write the text file, kill the session.
- [x] **Step 2: Write the GUI half.** The gui-check recipe exactly, with the
      screen-unlock guard at the top and a clear "screen is locked — ask the
      user" message instead of the raylib panic.
- [x] **Step 3: Add `just verify <surface>`** wiring both halves and reporting
      which artifacts were written.
- [x] **Step 4: Smoke-test the harness** on the dashboard surface (bare launch,
      no drive commands) in both backends and confirm the artifacts render.

---

## Surface matrix

Every row is one capture pair. The `Drive` column is the `init.lua` queue (or
the manual step, marked `*`). All captures target a small fixture file
(`docs/verification/fixtures/demo.rs`) so syntax highlighting, the gutter and
the statusline are exercised identically every time.

| # | Surface | Drive | TUI | GUI | Phase first shipped |
|---|---|---|---|---|---|
| 1 | Dashboard / welcome | bare launch | ✅ | ✅ | 0 |
| 2 | Editor + syntax highlight | open `demo.rs` | ✅ | ✅ | 0 |
| 3 | Statusline | open `demo.rs` | ✅ | ✅ | 2 |
| 4 | Gutter (signs) | `:Gitsigns toggle`, `:TodoList` on a fixture with hunks + a TODO | ✅ | ✅ | 2 |
| 5 | Sidebar | `:sidebar` | ✅ | ✅ | 2 |
| 6 | Dired / file explorer | `:Files` picker, `:e` completion | ✅ | ✅ | 2 |
| 7 | Which-key | press `SPC` | ✅ | ✅ | 2 |
| 8 | Which-key key accent (P9 T1) | press `SPC` with `whichkey_key` set | ✅ | ✅ | 9 |
| 9 | Cmdline + completion | `:e ~/De` + Tab | ✅ | ✅ | 2 |
| 10 | Git status / staged | `:Git`, `:GitStaged` on a dirty fixture | ✅ | ✅ | 7 |
| 11 | Diffview | `:Diffview` on a dirty fixture | ✅ | ✅ | 6 |
| 12 | Trouble | `:Trouble` (needs diagnostics — seed a fixture with known LSP errors, or `*` manual) | ✅ | ✅ | 6 |
| 13 | Todo list | `:TodoList` on a fixture with `TODO`/`FIXME` | ✅ | ✅ | 6 |
| 14 | Notifications (noice) | `:echo hello`, `:Noice` | ✅ | ✅ | 6 |
| 15 | Notification popup (P9 T2) | `:Noice popup` | ✅ | ✅ | 9 |
| 16 | Dialog | `ruster.ui.dialog{...}` | ✅ | ✅ | 6 |
| 17 | Hover (LSP) | `:hover` + `ruster.defer(1500, screenshot)` `*` needs live rust-analyzer | ✅ | ✅ | 3/8 |
| 18 | Settings / config browser | `:settings` | ✅ | ✅ | 6 |
| 19 | Theme picker | `:Themes` | ✅ | ✅ | 6 |
| 20 | Help | `:help`, `:help :sidebar` | ✅ | ✅ | 7 |
| 21 | Sessions | `:SessionSave`, `:SessionRestore` | ✅ | ✅ | 7 |
| 22 | Messages panel | `:messages` | ✅ | ✅ | 7 |
| 23 | Mason | `:Mason` | ✅ | ✅ | 6 |
| 24 | Terminal | `:terminal` | ✅ | ✅ | 4 |
| 25 | Debugger overlay + breakpoints | set a breakpoint, `:DebugStart` `*` needs a real debug target | ✅ | ✅ | 5 |
| 26 | Flash jump mode | press `s` then two chars | ✅ | ✅ | 6 |
| 27 | Projects / workspaces | `:Projects` | ✅ | ✅ | 5 |
| 28 | Multi-cursor | `Ctrl+D` on a repeated token | ✅ | ✅ | 5 |
| 29 | Ibuffer | `:Ibuffer` | ✅ | ✅ | 2 |
| 30 | `:16` / goto line | `:16` | ✅ | ✅ | 8 |
| 31 | ~~`:Browse`~~ | **declined** (Phase 9) — no row | — | — | — |
| 32 | ~~`:Music`~~ | **declined** (Phase 9) — no row | — | — | — |

Rows 17, 12 and 25 depend on a live service (31 and 32 were declined). For each, attempt the
defer-driven capture first (guarded: skip silently if the service isn't
reachable, mark the row). If no service is available, capture the surrounding
surface (the float, the overlay, the error toast) and mark the row `manual` in
the README.

- [x] **Task: capture rows 1–32 in both backends.** One row per commit or in
      coherent groups (all of one phase's surfaces together), each with its
      `docs/verification/README.md` entry. A row is done when both artifacts
      exist, are legible, glyphs render (no `?`), and theme colours apply.
- [x] **Task: adjudicate defects.** Seven found and written up as Phase 9 Task
      4b with an artifact and a repro each. No glyph fell back to `?` and no
      colour was unthemed; the failures were a surface the GUI does not draw at
      all (settings), a notification backend that reaches no screen
      (`:Noice popup`), a hover that returns nothing against a live
      rust-analyzer, a quickfix line-numbering convention two producers
      disagree about, `:echo` never reaching the message log, GUI statusline
      groups overwriting each other in a narrow window, and a dashboard
      advertising a command that does not exist. The matrix stays open until
      those are fixed and recaptured.
- [x] **Task: final sweep**, 2026-08-07. `just verify` produced all 32 pairs —
      64 artifacts, none empty, all read rather than skimmed. Every surface
      shows its content, `hover` included — the row that had resisted capture
      for days turned out to be a real product bug, not a harness quirk (below).

      The sweep paid for itself again, finding three more defects:

      - **`echo_at` wrote only to the toast.** The choke point for ~90
        `echo`/`echo_warn`/`echo_error` sites — "Session saved", every internal
        warning — so all of it expired in seconds and `:messages` never saw it.
        Fixing the `:echo` *command* earlier had left the other ninety; the
        empty `sessions` capture was the tell, which I had previously written
        off in the README as "needs a project root". It needed one *and* was
        hiding this.
      - **`:q!` did not quit with the settings page open.** `ForceQuit` was
        grouped with `:q` and `:Settings`, all three closing the page and
        returning, so there was no way out while it was up. Found by the
        harness timing out on the one surface whose deferred `:q!` never took
        effect.
      - **The call-stack fold numbered its summary row by position**, putting a
        `3` under frames 0, 14 and 15. It now carries the depth of the first
        frame it stands for, and `/usr/lib/` (the dynamic loader) folds too.
      - **`uri_from_path` did not resolve symlinks**, so every LSP request for a
        project under a symlinked path was answered `null`. This is what had
        been dismissed as "the hover capture is flaky": on macOS `$TMPDIR` is
        `/var/folders/…` and `/var` is a symlink, so `rootUri` and the document
        URIs named different directories and rust-analyzer put every document
        outside the workspace. Also hits `/tmp` and any symlinked project or
        home directory — hover, goto, references and rename alike. `just verify all` produces every pair; confirm the
      tree is clean, docs/verification is committed, and the matrix has no
      empty cells.

---

## Out of scope, deliberately

- **Fixing what the captures reveal is Phase 9's job**, not this phase's — this
  phase records and routes defects.
- **Video / animation capture.** `:screenshot` is a still; slide animations,
  the cursor blink and the flash jump are captured as a settled frame, not a
  movie.
- **Every plan checkbox.** The matrix is the curated user-visible surface set;
  a screenshot of `:GotoLine` parsing internals proves nothing a test doesn't.
