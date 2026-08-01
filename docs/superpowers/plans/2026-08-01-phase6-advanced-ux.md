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

The theme **live-preview picker** described in AGENTS.md is still outstanding — see Task 6.

## Global constraints

- `ruster-core` stays UI/OS-free; git, tool management and widgets live above it.
- Anything shelling out (git, installers) is non-blocking: background thread → `mpsc` →
  per-frame drain, degrading gracefully when the tool is missing. No tokio.
- Parsers are unit-tested with captured output; no CI test needs a real git repo or network.
- New rendering goes through `FrameState`, and **both** backends must draw it — a TUI-only
  view is a GUI parity regression.
- Keep `docs/{config-reference,keybindings}.md` in sync.

## Suggested order

Salvage first (small, already-written, unblocks deleting a stale branch), then floating
windows (several features are queued behind it), then git, then the panels, then Mason.

---

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
      backends. Treat it as a deliberate visual change, folded into Task 6 if wanted.
- [ ] Delete `feat/starship-ui` (local and origin) once the above has landed.
- [ ] Tests: leader Backspace pops one key, then cancels when the sequence empties.

### Task 2: Floating windows

Unblocks the three notification backends removed in PR #24 as uncompiled stubs
(`CmdlinePopup`, `Popup`, `Confirm`), plus modal dialogs for Tasks 3 and 7.

- [ ] A `FloatView { rect, lines, border, title, z }` on `FrameState`, drawn **above** the
      window views. Positioning helpers: cursor-relative, centred, and edge-anchored.
- [ ] Draw it in both `ruster-tui/src/renderer.rs` and `ruster-render-raylib`. The hover
      popup (`HoverWidget`) and which-key panel are the closest existing precedents; fold
      them onto the new primitive if it doesn't complicate them.
- [ ] Re-introduce `BackendKind::{CmdlinePopup, Popup, Confirm}` in `ruster-notify` **only
      once they render** — they were removed precisely because they were unreachable.
- [ ] Tests: z-ordering over window views; clamping at each screen edge.

### Task 3: Git signs (gitsigns)

- [ ] New `ruster-git` crate: run `git diff --no-color -U0` for a file, parse the hunk
      headers into `(added, modified, removed)` line ranges. Pure parsing, unit-tested from
      captured output — no test may require a real repository.
- [ ] Feed the existing sign column (`SignsView`) alongside diagnostics and test results.
      That merge already happens in `render`; this is a third source, not a new column.
- [ ] `]h` / `[h` to jump between hunks; `:Gitsigns toggle`.
- [ ] Non-blocking: refresh on save and on buffer switch via a background thread.
- [ ] Tests: hunk parsing (added/modified/removed/mixed); sign merge precedence with
      diagnostics on the same line.

### Task 4: Trouble-style problem list

- [ ] A pinned `*trouble*` buffer aggregating diagnostics, quickfix entries and TODOs,
      grouped by file with fold/unfold. Reuse the picker primitive rather than adding a
      list widget.
- [ ] `:Trouble` / `SPC x x`; Enter jumps, `q` closes.
- [ ] Tests: grouping and jump-target resolution.

### Task 5: Todo comments

- [ ] Highlight `TODO` / `FIXME` / `HACK` / `NOTE` / `XXX` in comments. `ruster-syntax`
      already colours org-mode `TODO` keywords (`markup.rs:113-120`) — extend that rather
      than adding a parallel scanner.
- [ ] `:TodoList` feeds the Task 4 panel.
- [ ] Config: `todo.keywords` (list) and per-keyword colours in the theme.
- [ ] Tests: keyword detection inside comments only, not in string literals.

### Task 6: Theme live-preview picker

Completes the one partially-delivered Phase 6 item.

- [ ] Extend the theme picker so moving the selection applies the theme live, and Esc
      restores the previous one. `resolve_theme_colors` already exists; this is picker
      wiring plus a restore path.
- [ ] Ship the four Catppuccin variants (latte, frappe, macchiato, mocha) as `themes/*.lua`.
      Mocha is already the built-in default palette.
- [ ] Optionally fold in `WhichKeyEntry` / `whichkey_key` from Task 1 here.
- [ ] Tests: preview applies and Esc restores; discovery finds user themes.

### Task 7: TUI widget layer

- [ ] Evaluate `ratatui-widgets` / `ratada` against writing the handful actually needed
      (button, checkbox, select, text field). **Prefer writing them** unless the crate earns
      its dependency — the settings page already hand-rolls its controls and works.
- [ ] Expose the chosen set through the Lua API so plugins can build dialogs.
- [ ] Whatever is chosen must render in both backends (see Global constraints).

### Task 8: Mason-style tool installer

- [ ] `:Mason` lists known LSP servers, DAP adapters and formatters with an installed/missing
      state, resolved by probing `PATH`.
- [ ] Install by shelling out to the tool's own documented method, streamed through the
      Task 2 floating window. Never bundle binaries; never install without confirmation.
- [ ] Tests: registry parsing and `PATH` probing with a stubbed lookup.

### Task 9: Diff viewer

- [ ] `:Diffview` — side-by-side working-tree diff in a vertical split, reusing the Task 3
      hunk parser and the existing window tree.
- [ ] Synchronised scrolling between the two panes.
- [ ] Tests: pane alignment across an unbalanced hunk.

### Task 10: Config, docs, CI

- [ ] Schema settings for every option added above (`git.*`, `todo.*`, `trouble.*`).
- [ ] Document commands and keys in `docs/{config-reference,keybindings}.md` **as each task
      lands**, not at the end — the Phase 5 debugger shipped undocumented for a week because
      this was deferred.
- [ ] `cargo test` + `cargo clippy --workspace --all-targets -- -D warnings` green.

## Notes carried forward from Phase 5

- **`App` is still a god object.** PR #24 took it from 78 fields to 57 and extracted
  `file_prompt`, `sidebar` and `dired`. Terminal and picker remain: terminal is small but
  `enter_terminal_normal` writes `self.vim`, and picker's reverse coupling is worse than its
  forward coupling — ten sites construct `PickerState` directly. Extract them opportunistically
  when a task touches that code, not as a standalone refactor.
- **`DiredState::forget` exists but is unwired.** `delete_active_buffer` never clears the
  per-buffer dired caches. One-line fix, no ticket.
- **Graphify corpus hygiene.** Exclude `.impeccable/` and `.opencode/` from the next run;
  the Starship landing-page mocks generate three spurious "communities".
