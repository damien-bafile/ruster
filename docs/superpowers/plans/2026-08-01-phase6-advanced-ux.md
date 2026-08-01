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
| **0 — Loose ends** | 1–6 | **1–3 done (PR #26), 5 done (PR #27)**; 4 needs a human; 6 deferred past Stage 1. |
| **1 — Foundations** | 7–8 | Floating windows, then git plumbing. Everything later builds on these. |
| **2 — Surfaces** | 9–12 | Trouble, todos, theme preview, widgets — all consume Stage 1. |
| **3 — Ecosystem** | 13–15 | Mason, diff viewer, then the config/docs/CI sweep. |

Task 6 has since been deferred past Stage 1 outright — see the task for why. Task 4 needs
a human at a GUI. Everything else in Stage 0 has landed, so Stage 1 is clear to start.

---

## Stage 0 — Loose ends

### Task 1: Salvage `feat/starship-ui` ✅ (PR #26)

> **Executed 2026-08-01. Two claims in the original plan were wrong; corrected below.**

The branch is 7 commits and 5 days stale, diverged before 92 commits of `main`. Most of it
is **superseded**: the test runner shipped as PR #19, four of its five theme colour fields
are already in `main` (more thoroughly — 15–30 references vs 12), and its headline commit
*removes* `whichkey.command_palette`, which `main` deliberately kept and made configurable
in PR #18. A dry-run merge conflicts in **10 files**, including `app.rs` (restructured in
PR #24) and `widgets.rs` (now a `widgets/` directory).

- [x] Cherry-pick `ad96b45` — Backspace pops the leader sequence, and cancels the `g` menu.
      Applies cleanly.
      **Correction:** this was planned as "a real UX win". It is not — it is behaviourally a
      **no-op** on `main`. The leader tree is one level deep, so popping the single group key
      empties the sequence and cancels, which is exactly what Esc already did; and the
      `g`-menu replay path was harmless too, because `VimState` clears `pending_g` on the
      very next key (`vim/mod.rs:334`). Both were confirmed by reverting each half and
      watching the tests still pass. Kept because the code now states its intent and the pop
      starts to matter once a group nests — but its tests are labelled characterization
      tests, not bug guards.
- [x] **Do not salvage the design docs.** `.impeccable/surfaces/editor-chrome.md`,
      `DESIGN.md` and `PRODUCT.md` turned out to be **byte-identical** between the branch and
      `main`, so there was nothing to take. The two genuinely unique docs
      (`2026-07-28-cmdline-whichkey-ux.md` + its spec) describe **the road not taken** —
      "the M-x keybinding is removed entirely", `CmdlineCompletions` replacing `PickerState`
      — the opposite of what shipped in PR #18. Landing them in `docs/superpowers/plans/`
      would leave a contradictory plan for a future agent, and that file opens with
      *"REQUIRED SUB-SKILL: use subagent-driven-development to implement this plan
      task-by-task."* They stay on the branch.
- [ ] **Still open — decide, don't merge:** `WhichKeyEntry` + the `whichkey_key` accent
      colour (55 lines, 7 files) is the one thing genuinely absent from `main`. It touches
      `ruster-render`, so it needs both backends. Deferred to Task 11 as a deliberate visual
      change; cherry-pick it there rather than merging the branch.
- [x] **Archived, not deleted.** The branch was local-only, so deleting it would have
      destroyed the only copy. Pushed to `origin/feat/starship-ui` as a reference. It is not
      a merge candidate — leave it indefinitely; it costs nothing.

### Task 2: Drop per-buffer caches on close ✅ (PR #26)

> **Scope grew during execution: five maps leaked, not one — and one leaked a process.**

`delete_active_buffer` closed the buffer and left every per-buffer cache behind, so they
grew for the life of the session.

- [x] Call `self.dired.forget(id)` from `delete_active_buffer`.
- [x] **`syntax`, `lsp_docs`, `diagnostics` and `terminals` leaked identically** — all are
      keyed by `BufferId` and none was ever cleaned up. Swept behind one `forget_buffer(id)`
      on the single close path.
- [x] **`terminals` was the one that mattered beyond memory:** `TerminalSession` kills its
      child in `Drop`, so a leaked session left a shell process running until the editor
      exited.
- [x] Tests: close a dired buffer, assert every cache dropped its id. Confirmed to fail with
      the cleanup removed.

### Task 3: Collapse the verbose notification call sites ✅ (PR #26)

`app.rs` has **72** `Notification::new(...)` calls, most spelling out the full
`ruster_core::message::MessageLevel::…` / `MessageSource::Echo` path across 130 columns.
`App::echo` was added in PR #24 and covers the Info case.

- [x] Added `echo_success` / `echo_warn` / `echo_error` beside `echo`, all routing through
      one `echo_at`. Converted the **40** `Echo`-source sites; the 32 remaining calls carry a
      non-`Echo` source (System, Lsp, Task, Build, Test) or a computed level and stay explicit.
- [x] **Level preservation verified mechanically, not by eye.** A script extracts every
      `(level, source, message)` triple emitted from `app.rs` before and after — expanding
      each helper call back to the level it implies — and compares the multisets. 57
      notifications, identical. (The first run showed a spurious 2-entry gap; that was an
      asymmetric filter in the script, which was fixed rather than waved through.)
- [x] One site needed restructuring rather than substitution: `debug_toggle_breakpoint`
      reported inside a live `Ref` on `self.ws`, which the old single-field
      `self.notify.push` tolerated but a `&mut self` helper does not.

### Task 4: Verify the raylib GUI, and record how

The one claim in PR #24 that rests on reasoning rather than observation: the sidebar
reaches the GUI as an ordinary `WindowView`, and neither render crate was touched — but
nobody has *looked* at it. macOS blocked synthetic keystrokes and screen capture in the
agent environment.

- [ ] Run `just gui`, open `:sidebar`, confirm the panel draws and the tree is navigable.
- [ ] Do the same for the debugger overlay and a noice toast, which also changed in Phase 5.
- [ ] If this keeps recurring, write a project skill under `.claude/skills/` that launches
      the GUI and drives it, so the check is repeatable rather than manual.

### Task 5: Refresh the stale process docs ✅

- [x] `.superpowers/sdd/task-11-report.md` marked **RESOLVED**, with the original text kept
      as the historical record. Both concerns verified cleared on `main` at `824bfbe`:
      `set_isearch_message` no longer exists and clippy is clean and CI-enforced; the four
      crates it called testless now hold 177 / 33 / 14 / 16 tests.
- [x] `.superpowers/sdd/progress.md` closed out, pointing here for ongoing work.
- [x] **Decision: leave the `:n` / `:s` debug aliases as they are.**

      While a debug session is active, `:n` and `:s` alias step-over and step-into
      (`app.rs`, guarded on `debug_session.is_some()`). Bare `:s` — repeat-last-substitute —
      is therefore shadowed mid-session, because the alias is matched before
      `parse_substitute`. `:s/pat/rep/` is unaffected: it carries a `/` and never
      exact-matches `"s"`.

      Left alone because the collision is narrow (bare `:s`, only while stopped in the
      debugger), the `:db_*` names are unambiguous, always available and documented, and the
      short forms match what gdb/lldb users expect. Removing them would cost more in muscle
      memory than the shadowing costs. Documented in `docs/keybindings.md` under the
      debugger section. **Do not re-litigate without a user report.**

### Task 6: Re-run graphify on a clean corpus — **deferred to after Stage 1**

**Decision (2026-08-01): do not run this yet.** The last run cost **474,462 input
tokens**, and the graph's value is navigating unfamiliar structure — which Stage 1 is about
to change again by adding floating windows and the `ruster-git` crate. Running now and
again after Stage 1 pays twice for the same insight. Run it **once**, after Task 8 lands,
so a single pass covers both the PR #24 extraction and the Stage 1 additions.

The current `graphify-out/` is from 2026-07-30 and is stale by the whole `app.rs`
extraction, so **treat its `App` metrics as historical**, not as a description of the tree.

When it is run:

- [ ] **There is no `--exclude` flag** — graphify narrows by the paths it is given. Pass an
      include-list rather than the repo root:
      `crates docs .superpowers AGENTS.md DESIGN.md PRODUCT.md`.
      That drops `.impeccable/` and `.opencode/`, which contributed 5 files and generated
      three whole communities ("Starship Design Board", "Starship Hero Mock", "OpenCode
      Plugin Deps") unrelated to the editor.
- [ ] Compare `App`'s betweenness against the previous **0.369** (next-highest was 0.127),
      and its edge count against **215**. PR #24 took `App` from 78 fields to 57 and moved
      ~640 non-test lines out of `app.rs`; this is the check on whether that actually
      reduced its centrality or merely relocated code.
- [ ] Also worth comparing: 2,415 nodes / 5,439 edges / 96 communities, and whether the
      **245 isolated nodes** shrank.
- [ ] **Ignore the "Import Cycles" section.** All 20 entries are files cycling to themselves
      (`buffer.rs -> buffer.rs`) — a graphify artifact, not real cycles. Recorded here so it
      is not investigated a third time.

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
- [ ] Optionally fold in `WhichKeyEntry` / `whichkey_key` — the one piece of
      `feat/starship-ui` worth keeping (55 lines across 7 files, archived at
      `origin/feat/starship-ui`, commit `b369449`). Cherry-pick it deliberately and teach
      **both** backends to draw the accent; do not merge the branch to get it.
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
