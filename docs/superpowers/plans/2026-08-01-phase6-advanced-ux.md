# Phase 6: Advanced UX & Ecosystem — Implementation Plan

> **For agentic workers:** implement task-by-task; steps use checkbox (`- [ ]`) syntax.
> Mark a step `- [x]` only once its code compiles and its tests pass.

**Roadmap:** [AGENTS.md](../../../AGENTS.md) § Phase 6
**Predecessor:** [2026-07-26-phase5-ide-tools.md](2026-07-26-phase5-ide-tools.md) (complete)

**Goal:** Git awareness (signs, hunks, diff), aggregated problem lists (Trouble, TODOs),
tool management (Mason), and the UI polish deferred out of earlier phases — floating
windows and the interactive widget layer that several stubbed features are waiting on.

## Already delivered

Four Phase 6 items shipped early, during Phase 5. They are **not** in scope here:

| Item | Where |
|---|---|
| Noice (notifications) | `crates/ruster-notify` — manager, mini/notify backends, `:Noice split` |
| Flash (jump mode) | `FlashState` in `crates/ruster-tui/src/app.rs` |
| Configuration browser | `:settings` / `:config`, schema-driven from `ruster-lua/src/schema.rs` |
| Theme system (partial) | `themes/*.lua` discovery + theme names in the picker |

The theme **live-preview picker** described in AGENTS.md is still outstanding — see Task 11.

## Global constraints

- `ruster-core` stays UI/OS-free; git, tool management and widgets live above it.
- Anything shelling out (git, installers) is non-blocking: background thread → `mpsc` →
  per-frame drain, degrading gracefully when the tool is missing. No tokio.
- Parsers are unit-tested with captured output; no CI test needs a real git repo or network.
- New rendering goes through `FrameState`, and **both** backends must draw it — a TUI-only
  view is a GUI parity regression.
- Keep `docs/{config-reference,keybindings}.md` in sync.

## Order of work

Three stages. **Stage 0 clears the decks** — it is entirely small, known work carried out
of Phase 5, and none of it is blocked on anything. Doing it first means Phase 6 features
start from a repo with no stale branches, no stale process docs, and an accurate dependency
graph to navigate by.

| Stage | Tasks | Theme |
|---|---|---|
| **0 — Loose ends** | 1–6 | Carried out of Phase 5. Small, independent, mostly hygiene. |
| **1 — Foundations** | 7–8 | Floating windows, then git plumbing. Everything later builds on these. |
| **2 — Surfaces** | 9–12 | Trouble, todos, theme preview, widgets — all consume Stage 1. |
| **3 — Ecosystem** | 13–15 | Mason, diff viewer, then the config/docs/CI sweep. |

Within Stage 0, do Task 1 first (it deletes a branch, so it stops rotting) and Task 6 last
(the graph is most useful once the tree has settled).

---

## Stage 0 — Loose ends

### Task 1: Salvage `feat/starship-ui`, then delete the branch

The branch is 7 commits and 5 days stale, diverged before 92 commits of `main`. Most of it
is **superseded**: the test runner shipped as PR #19, four of its five theme colour fields
are already in `main` (more thoroughly — 15–30 references vs 12), and its headline commit
*removes* `whichkey.command_palette`, which `main` deliberately kept and made configurable
in PR #18. A dry-run merge conflicts in **10 files**, including `app.rs` (restructured in
PR #24) and `widgets.rs` (now a `widgets/` directory).

So: cherry-pick what is genuinely novel, take the docs, and drop the rest.

- [ ] Cherry-pick `ad96b45` — Backspace pops the leader sequence, and cancels the `g` menu.
      12 lines, one file; `leader_pending: Option<Vec<char>>` is unchanged in `main`, so it
      applies cleanly. Add a test alongside `leader_resolves_groups_and_actions`.
- [ ] Salvage the design docs, which merge cleanly (they were absent from the conflict set):
      `docs/superpowers/plans/2026-07-28-cmdline-whichkey-ux.md` (798 lines),
      `docs/superpowers/specs/2026-07-28-cmdline-whichkey-ux-design.md` (300 lines),
      `.impeccable/surfaces/editor-chrome.md`, and the `DESIGN.md` / `PRODUCT.md` additions.
      Reconcile them against what shipped — they predate the current cmdline design.
- [ ] **Decide, don't merge:** `WhichKeyEntry` + the `whichkey_key` accent colour (55 lines,
      7 files) is genuinely absent from `main` but touches `ruster-render`, so it needs both
      backends. Treat it as a deliberate visual change, folded into Task 11 if wanted.
- [ ] Delete `feat/starship-ui` once the above has landed. It is **local-only** — never
      pushed to origin — so this is a local `git branch -D`, not a remote deletion.
- [ ] Tests: leader Backspace pops one key, then cancels when the sequence empties.

### Task 2: Wire up `DiredState::forget`

`delete_active_buffer` closes a buffer without clearing the per-buffer dired caches
(`dirs`, `styled`, `entries`), so they grow for the life of the session. `forget(id)` was
written for this in PR #24 and has **zero call sites**.

- [ ] Call `self.dired.forget(id)` from `delete_active_buffer` in `crates/ruster-tui/src/app.rs`.
- [ ] Check the same leak for `terminals` and `syntax`, which are also keyed by `BufferId`.
- [ ] Tests: open a dired buffer, close it, assert the caches no longer hold its id.

### Task 3: Collapse the verbose notification call sites

`app.rs` has **72** `Notification::new(...)` calls, most spelling out the full
`ruster_core::message::MessageLevel::…` / `MessageSource::Echo` path across 130 columns.
`App::echo` was added in PR #24 and covers the Info case.

- [ ] Add `warn` / `error` siblings to `App::echo`, then convert the call sites.
- [ ] Purely mechanical — **the level of each message must not change**. PR #24 already
      fixed one bug caused by an argument-list `match` picking the wrong level; don't
      introduce another while tidying.
- [ ] Tests: the existing suite is the guard; no new tests needed.

### Task 4: Verify the raylib GUI, and record how

The one claim in PR #24 that rests on reasoning rather than observation: the sidebar
reaches the GUI as an ordinary `WindowView`, and neither render crate was touched — but
nobody has *looked* at it. macOS blocked synthetic keystrokes and screen capture in the
agent environment.

- [ ] Run `just gui`, open `:sidebar`, confirm the panel draws and the tree is navigable.
- [ ] Do the same for the debugger overlay and a noice toast, which also changed in Phase 5.
- [ ] If this keeps recurring, write a project skill under `.claude/skills/` that launches
      the GUI and drives it, so the check is repeatable rather than manual.

### Task 5: Refresh the stale process docs

- [ ] `.superpowers/sdd/task-11-report.md` is still marked `DONE_WITH_CONCERNS`. Both
      concerns — "3 build warnings" and "crates have zero tests" — were resolved before
      Phase 5 shipped. Mark it resolved with a pointer to what fixed it.
- [ ] `.superpowers/sdd/progress.md` ends at the noice ledger; either close it out or
      note that Phase 5 superseded it.
- [ ] Decide and record: `:n` and `:s` alias step-over/step-into while a debug session is
      active, shadowing bare `:s`. **Recommendation: leave it** — the `:db_*` names always
      work and are documented. Write the decision down so it isn't re-litigated.

### Task 6: Re-run graphify on a clean corpus

The current `graphify-out/` is from 2026-07-30 and is **17 commits stale** — it predates
the whole `app.rs` extraction, so its `App` god-node metrics describe a file that no longer
exists in that shape.

- [ ] Exclude `.impeccable/` and `.opencode/`. The Starship landing-page mocks generated
      three spurious communities ("Starship Design Board", "Starship Hero Mock", "OpenCode
      Plugin Deps") that have nothing to do with the editor.
- [ ] Re-run and compare the `App` betweenness against the previous 0.369, as a check on
      whether the extraction actually moved the needle.
- [ ] Ignore the "Import Cycles" section — all 20 entries are files cycling to themselves,
      a graphify artifact, not real cycles. Noted here so it isn't investigated again.

---

## Stage 1 — Foundations

### Task 7: Floating windows

Unblocks the three notification backends removed in PR #24 as uncompiled stubs
(`CmdlinePopup`, `Popup`, `Confirm`), plus modal dialogs for Tasks 9 and 12.

- [ ] A `FloatView { rect, lines, border, title, z }` on `FrameState`, drawn **above** the
      window views. Positioning helpers: cursor-relative, centred, and edge-anchored.
- [ ] Draw it in both `ruster-tui/src/renderer.rs` and `ruster-render-raylib`. The hover
      popup (`HoverWidget`) and which-key panel are the closest existing precedents; fold
      them onto the new primitive if it doesn't complicate them.
- [ ] Re-introduce `BackendKind::{CmdlinePopup, Popup, Confirm}` in `ruster-notify` **only
      once they render** — they were removed precisely because they were unreachable.
- [ ] Tests: z-ordering over window views; clamping at each screen edge.

### Task 8: Git signs (gitsigns)

- [ ] New `ruster-git` crate: run `git diff --no-color -U0` for a file, parse the hunk
      headers into `(added, modified, removed)` line ranges. Pure parsing, unit-tested from
      captured output — no test may require a real repository.
- [ ] Feed the existing sign column (`SignsView`) alongside diagnostics and test results.
      That merge already happens in `render`; this is a third source, not a new column.
- [ ] `]h` / `[h` to jump between hunks; `:Gitsigns toggle`.
- [ ] Non-blocking: refresh on save and on buffer switch via a background thread.
- [ ] Tests: hunk parsing (added/modified/removed/mixed); sign merge precedence with
      diagnostics on the same line.

## Stage 2 — Surfaces

### Task 9: Trouble-style problem list

- [ ] A pinned `*trouble*` buffer aggregating diagnostics, quickfix entries and TODOs,
      grouped by file with fold/unfold. Reuse the picker primitive rather than adding a
      list widget.
- [ ] `:Trouble` / `SPC x x`; Enter jumps, `q` closes.
- [ ] Tests: grouping and jump-target resolution.

### Task 10: Todo comments

- [ ] Highlight `TODO` / `FIXME` / `HACK` / `NOTE` / `XXX` in comments. `ruster-syntax`
      already colours org-mode `TODO` keywords (`markup.rs:113-120`) — extend that rather
      than adding a parallel scanner.
- [ ] `:TodoList` feeds the Task 9 panel.
- [ ] Config: `todo.keywords` (list) and per-keyword colours in the theme.
- [ ] Tests: keyword detection inside comments only, not in string literals.

### Task 11: Theme live-preview picker

Completes the one partially-delivered Phase 6 item.

- [ ] Extend the theme picker so moving the selection applies the theme live, and Esc
      restores the previous one. `resolve_theme_colors` already exists; this is picker
      wiring plus a restore path.
- [ ] Ship the four Catppuccin variants (latte, frappe, macchiato, mocha) as `themes/*.lua`.
      Mocha is already the built-in default palette.
- [ ] Optionally fold in `WhichKeyEntry` / `whichkey_key` from Task 1 here.
- [ ] Tests: preview applies and Esc restores; discovery finds user themes.

### Task 12: TUI widget layer

- [ ] Evaluate `ratatui-widgets` / `ratada` against writing the handful actually needed
      (button, checkbox, select, text field). **Prefer writing them** unless the crate earns
      its dependency — the settings page already hand-rolls its controls and works.
- [ ] Expose the chosen set through the Lua API so plugins can build dialogs.
- [ ] Whatever is chosen must render in both backends (see Global constraints).

## Stage 3 — Ecosystem

### Task 13: Mason-style tool installer

- [ ] `:Mason` lists known LSP servers, DAP adapters and formatters with an installed/missing
      state, resolved by probing `PATH`.
- [ ] Install by shelling out to the tool's own documented method, streamed through the
      Task 7 floating window. Never bundle binaries; never install without confirmation.
- [ ] Tests: registry parsing and `PATH` probing with a stubbed lookup.

### Task 14: Diff viewer

- [ ] `:Diffview` — side-by-side working-tree diff in a vertical split, reusing the Task 8
      hunk parser and the existing window tree.
- [ ] Synchronised scrolling between the two panes.
- [ ] Tests: pane alignment across an unbalanced hunk.

### Task 15: Config, docs, CI

- [ ] Schema settings for every option added above (`git.*`, `todo.*`, `trouble.*`).
- [ ] Document commands and keys in `docs/{config-reference,keybindings}.md` **as each task
      lands**, not at the end — the Phase 5 debugger shipped undocumented for a week because
      this was deferred.
- [ ] `cargo test` + `cargo clippy --workspace --all-targets -- -D warnings` green.

## Out of scope, deliberately

- **Extracting terminal and picker from `App`.** PR #24 took it from 78 fields to 57 and
  `app.rs` to 5,767 non-test lines. The two remaining clusters are harder: `enter_terminal_normal`
  writes `self.vim`, and picker's *reverse* coupling is worse than its forward coupling —
  ten sites construct `PickerState` directly. Extract them opportunistically when a Phase 6
  task already touches that code, not as a standalone refactor.
- **Re-introducing the `CmdlinePopup` / `Popup` / `Confirm` backends** before Task 7 lands.
  They were removed in PR #24 as unreachable; adding the variants back without a renderer
  recreates exactly the dead surface that was cleaned up.
