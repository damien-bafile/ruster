# Phase 9 — Cleanup

**Status:** planning, 2026-08-02.

Phases 6–8 deferred or parked a small set of items. None is a new capability —
each is either work that could not be finished when the phase shipped (a
screenshot that needed a timer that did not exist, a GUI nobody could look at),
or a feature a phase plan deliberately left for "later" that later has now
arrived.

The ordering rule is the same as Phase 8: what a user can see first. Tasks 1–4
are visible the moment they land; 5–7 are whole small features; 8 is
housekeeping so the plan tree stops lying about what shipped.

**The one genuinely open decision:** Task 5 (`:Music`). Phase 7's own plan
calls it "the least defensible feature in the phase". Keep the *decide at
execution* gate: if it feels wrong when the time comes, skip it and record the
decision in the plan — a deliberate non-feature is a decision, an unchecked box
is an accident.

---

## Global constraints (unchanged from Phases 6–8)

- **GUI/TUI parity.** `ruster-render-raylib` does not depend on ratatui. Any
  change that reaches only one backend is a regression. New rendering goes
  through `FrameState` and **both** backends must draw it.
- **Non-blocking.** Background thread → `mpsc` → per-frame drain. No tokio in
  the editor loop. Anything shelling out degrades gracefully when the tool is
  missing.
- **Docs as each task lands.** `crates/ruster-tui/tests/docs_in_sync.rs` fails
  CI naming any `:` command absent from `docs/keybindings.md`. New commands and
  settings update `docs/{config-reference,keybindings}.md` in the same commit.
- **`App` stays the composition root, not the trash can.** New subsystems own
  their state in their own module and expose one field, the way
  `sidebar`/`dired`/`trouble`/`debug_state`/`git_gutter`/`lsp_state` do.

---

## Task 1: Which-key key accent (`whichkey_key`)

Carried over from Phase 6 Task 1 / Task 11. `WhichKeyView::rows` is
`Vec<String>`, so the key letter and its description share one colour. The
`feat/starship-ui` branch (archived at `origin/feat/starship-ui`) has a version
of this as commit `b369449`, but it was built on that branch's diverged tree
and no longer applies — **reimplement it, do not cherry-pick.**

**Files:**
- Modify: `crates/ruster-render/src/lib.rs:516-520` — `WhichKeyView.rows` type
- Modify: `crates/ruster-tui/src/app.rs:1077-1100` — `leader_whichkey`, and the
  equivalent `g_whichkey` / `whichkey` construction sites
- Modify: `crates/ruster-tui/src/widgets/mod.rs:1020-1080` — `WhichKeyWidget`
- Modify: `crates/ruster-tui/src/renderer.rs:185-197` — TUI draw path
- Modify: `crates/ruster-render-raylib/src/lib.rs:1036-1050` — raylib draw path
- Modify: `crates/ruster-syntax/src/theme.rs`, `crates/ruster-lua/src/schema.rs`
  — `colors.whichkey_key` setting
- Test: `crates/ruster-tui/tests/colors_are_themeable.rs`

**Interfaces:**
- Produces: `WhichKeyEntry { key: String, desc: String }` in `ruster-render`;
  `WhichKeyView.rows: Vec<WhichKeyEntry>`; `Colors::whichkey_key`.

- [ ] **Step 1: Add `WhichKeyEntry` and change `WhichKeyView.rows`**

```rust
// crates/ruster-render/src/lib.rs
#[derive(Debug, Clone, PartialEq)]
pub struct WhichKeyEntry {
    pub key: String,
    pub desc: String,
}

pub struct WhichKeyView {
    pub title: String,
    pub rows: Vec<WhichKeyEntry>,
    pub anim: f32,
}
```

- [ ] **Step 2: Add `colors.whichkey_key` to the theme and schema**

Follow the exact `whichkey_bg`/`whichkey_fg` pattern already in
`crates/ruster-render/src/lib.rs:63-99` and
`crates/ruster-lua/src/schema.rs:343-344`. Add the schema row, the
`Colors` field, and the default. Fix `app.rs:224-225` (`set(...)` for the new
field).

- [ ] **Step 3: Change the construction sites to build `WhichKeyEntry`**

`leader_whichkey` builds each row as `format!("{}  {}", k, desc)` — split into
`WhichKeyEntry { key: k, desc }`. Do the same in `g_whichkey` and the picker
that opens the which-key panel.

- [ ] **Step 4: Draw the accent in the TUI**

In `WhichKeyWidget::render`, draw the key letter in `whichkey_key` and the
description in `whichkey_fg`.

- [ ] **Step 5: Draw the accent in raylib**

In `crates/ruster-render-raylib/src/lib.rs:1036-1050`, draw each key letter in
`whichkey_key`. Use `MeasureTextEx` to advance past the key column, matching
the TUI's column layout.

- [ ] **Step 6: Tests**

Add to `colors_are_themeable.rs`: with `whichkey_key = "#ff0000"` set, the TUI
renderer styles the key span red and the description the default fg; the
raylib backend resolves the accent and doesn't render `?` for the key glyphs.
Both backends must agree the accent is distinct from `whichkey_fg`.

- [ ] **Step 7: Verify in the running editor**

`just run` a file, press `SPC`, confirm the key letters take the accent colour
and the descriptions stay `whichkey_fg`. Then `just gui`, same check.

- [ ] **Step 8: Commit**

```bash
git add crates/ruster-render crates/ruster-tui crates/ruster-render-raylib crates/ruster-syntax crates/ruster-lua
git commit -m "feat(theme): colour which-key key letters separately (whichkey_key)"
```

---

## Task 2: Re-introduce the notification popup backends

Phase 6 Task 7 removed `BackendKind::{CmdlinePopup, Popup, Confirm}` as
uncompiled stubs — nothing rendered them. Floats now render
(`FrameState.floats`, drawn above window views), so the backends can come back
as real surfaces instead of dead enum variants.

**Files:**
- Modify: `crates/ruster-notify/src/backend.rs` — enum + `all()`
- Modify: `crates/ruster-notify/src/lib.rs` — dispatch the new kinds to floats
- Modify: `crates/ruster-tui/src/app.rs` — turn a `BackendKind` into a float
- Test: `crates/ruster-notify` unit tests, `crates/ruster-tui` render test

**Interfaces:**
- Consumes: `FrameState.floats` (float rect, border, title, z).
- Produces: `BackendKind::{CmdlinePopup, Popup, Confirm}` in `all()`; a float
  is pushed for each when a notification of that kind is queued.

- [ ] **Step 1: Re-add the three variants to `BackendKind`**

```rust
pub enum BackendKind {
    Mini,
    Notify,
    Split,
    CmdlinePopup,
    Popup,
    Confirm,
}
```

Extend `all()` to include them.

- [ ] **Step 2: Decide and implement the float mapping**

`CmdlinePopup` and `Popup` both become floats (the difference is duration —
confirm-dialog semantics and a persistent popup). `Confirm` becomes a
`ruster.ui.dialog`-style modal with an OK/Cancel. Implement the mapping in the
notification → float bridge in `app.rs`, following how `HoverWidget` builds a
float.

- [ ] **Step 3: Tests**

Unit-test that a `CmdlinePopup`/`Popup` notification produces a float in the
next `FrameState`; a `Confirm` produces a modal. Reuse the render harness
pattern from `tests/draw_order_parity.rs` so both backends are covered.

- [ ] **Step 4: Wire a command to exercise one backend**

Add `:Noice popup` (or reuse an existing notify command) that queues a
`Popup`-kind notification, and document it in `docs/keybindings.md` (the
docs-sync test will fail otherwise).

- [ ] **Step 5: Verify + commit**

See a popup float in both backends. Commit.

---

## Task 3: `:hover` GUI capture

Phase 8 Task 7 landed `:hover` (dispatch to the same `lsp_hover` as `K`), but
the GUI screenshot was unverifiable: the shot fired a couple of frames in,
long before the LSP replied, and nothing could delay it. `ruster.defer` landed
in PR #59 and unblocks this.

**Files:**
- Modify: `~/.config/ruster/init.lua` (test config), scripts in `scripts/`
- Artifact: a committed hover screenshot under `docs/verification/`

- [ ] **Step 1: Drive a deferred GUI hover capture**

Use the gui-check recipe, but queue the screenshot inside a `ruster.defer` so it
fires after the LSP round-trip:

```lua
ruster.cmd(":hover")
ruster.defer(1500, function() ruster.cmd(":screenshot /tmp/hover-gui.png") end)
```

Run with the screen unlocked; read the PNG and confirm the hover float renders
with its border and the doc text.

- [ ] **Step 2: Commit the artifact**

Move the PNG to `docs/verification/hover-gui.png` and reference it from the
Phase 10 matrix (below). Commit.

---

## Task 4: Verify the raylib GUI surfaces

Phase 6 Task 4 was the one claim resting on reasoning rather than observation:
the sidebar reaches the GUI as an ordinary `WindowView`. The `gui-check` skill
now makes this repeatable rather than manual.

**Files:**
- Artifacts under `docs/verification/`
- If the skill proves insufficient, a repeatable script (see the Phase 10
  capture-harness section)

- [ ] **Step 1: Sidebar**

Drive `:sidebar` in the GUI, screenshot, confirm the panel draws and the tree
is navigable (▸/▾ glyphs render).

- [ ] **Step 2: Debugger overlay**

Drive the debugger overlay (breakpoint set + `:DebugStart`), screenshot,
confirm the docked panel draws over the stopped line.

- [ ] **Step 3: Noice toast**

Queue `:echo text`, screenshot, confirm the mini toast renders.

- [ ] **Step 4: Commit the three artifacts**

---

## Task 5: `:Music` (mpd) — **decide at execution**

Phase 7 Task 4. Control an already-running `mpd` on `localhost:6600` over its
plain-text protocol. No bundled player, no audio decoding. Degrade silently
when mpd is absent. Tests parse captured protocol responses — no test may
require a running daemon.

**Gate:** if this feels wrong when implementing — if it is pure busy-work that
nobody would use — skip it and record the decision in this plan. Do not
half-build it.

**Files (if built):**
- Create: `crates/ruster-mpd/` (or a module) for the protocol client
- Modify: `crates/ruster-tui/src/app.rs` — `:Music` command
- Test: protocol parsing from captured output

- [ ] **Step 1: Decide.** Re-read Phase 7's "Honest note" and this gate. Record
      the decision.
- [ ] **Step 2 (if building): protocol client + tests**
- [ ] **Step 3 (if building): `:Music` command, docs, both-backend verify**

---

## Task 6: `:Browse <url>`

Phase 7 Task 5's recommended alternative to an embedded browser: fetch a URL
over HTTP and render it as markup in a buffer, reusing the markdown path that
already serves `:help` and hover docs. Text-mode only, both backends, no engine.

**Files:**
- Create: `crates/ruster-browse/` (HTTP fetch, HTML→text/markdown)
- Modify: `crates/ruster-tui/src/app.rs` — `:Browse` command + buffer
- Modify: `docs/keybindings.md`
- Test: fetch + render from a stubbed HTTP response

- [ ] **Step 1: HTTP fetch with graceful failure** (no network in CI tests —
      stub the client; missing/refusing server degrades to a notification)
- [ ] **Step 2: Render fetched HTML as markup in a buffer** (reuse the
      `:help` markup path)
- [ ] **Step 3: `:Browse <url>` command, docs, tests, verify both backends**

---

## Task 7: Email — compose only

Phase 7 Task 6's defensible slice: open an editor buffer, hand the result to
the system's configured MUA (`mailto:`/`sendmail`). No credentials, no IMAP,
no inbox. Full IMAP stays a plugin concern.

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` — `:Email` / `:Mail` compose command
- Modify: `docs/keybindings.md`
- Test: compose buffer content → the correct `mailto:`/`sendmail` invocation

- [ ] **Step 1: Compose buffer + `:w` sends** (shell out to the MUA, degrade
      gracefully when none is configured)
- [ ] **Step 2: Docs, tests, verify**

---

## Task 8: Doc hygiene

The plan tree has checked-in work with unticked boxes, and stale status
headers. None of this changes code; all of it makes the plans honest.

- [ ] **Step 1: Phase 6 Task 10 (todo comments)** — confirm the work exists
      (`todo.keywords` in config, `:TodoList`, trouble panel), then tick the
      boxes and note the confirmation.
- [ ] **Step 2: Phase 6 Task 11 (theme live-preview)** — confirm
      `theme_before_preview` / `:Themes` exist, tick the boxes.
- [ ] **Step 3: Phase 6 Task 4** — mark verified once Task 4 above lands.
- [ ] **Step 4: Phase 8 Task 7** — mark the GUI bullet verified once Task 3
      above lands.
- [ ] **Step 5: Commit**

---

## Out of scope, deliberately

- **Extracting terminal and picker from `App`.** Still deferred from Phase 6;
  still the right call.
- **Full IMAP** (Phase 7 Task 6) — a plugin concern, not core.
- **Embedded browser** (Phase 7 Task 5) — parity constraint forbids it.
- **Threading `Rc<RefCell<Workspace>>`** — Phase 8's out-of-scope note stands.
