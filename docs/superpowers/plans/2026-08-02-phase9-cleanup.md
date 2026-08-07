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

- [x] **Step 1: Add `WhichKeyEntry` and change `WhichKeyView.rows`**

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

- [x] **Step 2: Add `colors.whichkey_key` to the theme and schema**

Follow the exact `whichkey_bg`/`whichkey_fg` pattern already in
`crates/ruster-render/src/lib.rs:63-99` and
`crates/ruster-lua/src/schema.rs:343-344`. Add the schema row, the
`Colors` field, and the default. Fix `app.rs:224-225` (`set(...)` for the new
field).

- [x] **Step 3: Change the construction sites to build `WhichKeyEntry`**

`leader_whichkey` builds each row as `format!("{}  {}", k, desc)` — split into
`WhichKeyEntry { key: k, desc }`. Do the same in `g_whichkey` and the picker
that opens the which-key panel.

- [x] **Step 4: Draw the accent in the TUI**

In `WhichKeyWidget::render`, draw the key letter in `whichkey_key` and the
description in `whichkey_fg`.

- [x] **Step 5: Draw the accent in raylib**

In `crates/ruster-render-raylib/src/lib.rs:1036-1050`, draw each key letter in
`whichkey_key`. Use `MeasureTextEx` to advance past the key column, matching
the TUI's column layout.

- [x] **Step 6: Tests**

Add to `colors_are_themeable.rs`: with `whichkey_key = "#ff0000"` set, the TUI
renderer styles the key span red and the description the default fg; the
raylib backend resolves the accent and doesn't render `?` for the key glyphs.
Both backends must agree the accent is distinct from `whichkey_fg`.

- [x] **Step 7: Verify in the running editor**

`just run` a file, press `SPC`, confirm the key letters take the accent colour
and the descriptions stay `whichkey_fg`. Then `just gui`, same check.

- [x] **Step 8: Commit**

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

- [x] **Step 1: Re-add the three variants to `BackendKind`**

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

- [x] **Step 2: Decide and implement the float mapping**

`CmdlinePopup` and `Popup` both become floats (the difference is duration —
confirm-dialog semantics and a persistent popup). `Confirm` becomes a
`ruster.ui.dialog`-style modal with an OK/Cancel. Implement the mapping in the
notification → float bridge in `app.rs`, following how `HoverWidget` builds a
float.

- [x] **Step 3: Tests**

Unit-test that a `CmdlinePopup`/`Popup` notification produces a float in the
next `FrameState`; a `Confirm` produces a modal. Reuse the render harness
pattern from `tests/draw_order_parity.rs` so both backends are covered.

- [x] **Step 4: Wire a command to exercise one backend**

Add `:Noice popup` (or reuse an existing notify command) that queues a
`Popup`-kind notification, and document it in `docs/keybindings.md` (the
docs-sync test will fail otherwise).

- [x] **Step 5: Verify + commit**

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

- [x] **Step 1: Drive a deferred GUI hover capture**

Use the gui-check recipe, but queue the screenshot inside a `ruster.defer` so it
fires after the LSP round-trip:

```lua
ruster.cmd(":hover")
ruster.defer(1500, function() ruster.cmd(":screenshot /tmp/hover-gui.png") end)
```

Run with the screen unlocked; read the PNG and confirm the hover float renders
with its border and the doc text.

- [x] **Step 2: Commit the artifact**

Move the PNG to `docs/verification/hover-gui.png` and reference it from the
Phase 10 matrix (below). Commit.

**Verified 2026-08-03.** `docs/verification/hover-gui.png` — the hover float
draws in raylib with its border, the syntax-highlighted signature
(`let greeting: String`) and the doc body. The `ruster.defer` recipe works: the
capture fires after the LSP round-trip instead of racing it.

Two defects surfaced while getting there, neither of them a rendering bug:

- **The LSP root was the process cwd, not the file's project — fixed.**
  `LspState::root()` returned `current_dir()`, so opening a file outside that
  directory initialised rust-analyzer against the wrong workspace and every
  request answered `null`, surfacing only as "No hover info". The wire log was
  unambiguous: `rootUri` was the ruster repo while the `didOpen` named a file
  under `/private/tmp`. Now `root_for(path)` derives the root from the file via
  `ruster_project::project_root`. Servers are still keyed by language alone, so
  the first project opened in a session owns its language's server.
- **A long hover is unbounded — still open.** Hovering `String` fills the
  entire window and long doc lines run off the right edge unwrapped. Needs a
  height/width clamp and wrapping.

---

## Task 4: Verify the raylib GUI surfaces

Phase 6 Task 4 was the one claim resting on reasoning rather than observation:
the sidebar reaches the GUI as an ordinary `WindowView`. The `gui-check` skill
now makes this repeatable rather than manual.

**Files:**
- Artifacts under `docs/verification/`
- If the skill proves insufficient, a repeatable script (see the Phase 10
  capture-harness section)

- [x] **Step 1: Sidebar**

Drive `:sidebar` in the GUI, screenshot, confirm the panel draws and the tree
is navigable (▸/▾ glyphs render).

**Verified 2026-08-03.** `docs/verification/sidebar-gui.png` — the panel draws
at the left, the `▸` glyphs resolve in the font atlas (no `?`), and the tree
lists the crate. Phase 6 Task 4's claim now rests on observation. One cosmetic
defect: the sidebar's own status segment and the window statusline overlap at
the bottom of the frame, so `[ruster-tui]` and `app.rs` overprint.

- [x] **Step 2: Debugger overlay**

Drive the debugger overlay (breakpoint set + `:DebugStart`), screenshot,
confirm the docked panel draws over the stopped line.

**Verified 2026-08-03.** `docs/verification/debugger-gui.png` — `:debug` stops
at the breakpoint and the docked panel draws `[Debug: PAUSED]`, a 16-frame call
stack with `hoverdemo::main` at frame 0, and a Locals section showing
`greeting`. The red breakpoint dot renders in the gutter on the right line.
The TUI shows the same session — PAUSED, the same stack, the same locals, plus
Registers — so the parity constraint holds.

Getting there took six fixes. `:debug` could not previously launch anything at
all — the RUNNING/"(no frames)" panel in the first capture was every one of
these failing in sequence:

1. **No `type` discriminator on outgoing messages.** The `dap` crate omits it;
   `read_message` requires it on the way in but `write_message` never wrote it.
   lldb-dap silently dropped every frame — no response, no error on any
   stream. Identical bytes with and without the field: 1506 vs 0.
2. **Absent optionals serialized as explicit `null`.** lldb-dap rejected
   `initialize` outright with "expected bool at
   `arguments.supportsMemoryReferences`". Nulls are now stripped recursively.
3. **The launch config was never sent.** `debug_start` built
   `cfg.launch_config` — the object carrying `program`/`cwd` — and then sent
   `send_launch(json!({}))`.
4. **The program was a placeholder.** `detect_config` defaulted to the literal
   `target/debug/<binary>`. `ruster_project::debug_binary` now reads the
   package name from `Cargo.toml`; a missing build says so instead.
5. **`configurationDone` was never sent.** The adapter holds the `launch` reply
   until it arrives, so the target never ran. Sent eagerly after the
   breakpoints go in — lldb-dap does not emit the `initialized` event until
   *after* `configurationDone`, so waiting on that event deadlocks.
6. **Every response was discarded.** `handle_response` was an empty function,
   so the stack trace the editor asked for and received was thrown away. It now
   files stack frames, scopes, variables and threads; `variable_cache` became
   `HashMap<u64, Vec<Variable>>`, since a `variablesReference` names a whole
   list rather than one variable.

Two smaller ones alongside: breakpoints were sent 0-based while `initialize`
advertises `linesStartAt1`, so every breakpoint bound one line above the one
the user set (`to_dap_line` now converts at the protocol boundary); and the
Rust adapter was looked up only as `lldb-vscode`, renamed `lldb-dap` in
LLVM 18, so `detect_config` now prefers whichever is installed.

- [x] **Step 3: Noice toast**

Queue `:echo text`, screenshot, confirm the mini toast renders.

**Verified 2026-08-03.** `docs/verification/noice-toast-gui.png` — the mini
toast renders top-right.

- [x] **Step 4: Commit the three artifacts**

---

## Task 4b: Defects the Phase 10 sweep found

Phase 10 built `scripts/verify-capture.sh` and captured all 32 surfaces in both
backends. Phase 10's rule is that it records defects and Phase 9 fixes them, so
they land here. Each has a committed artifact under `docs/verification/` and a
one-line repro.

**Blocking — the embedded terminal is a one-way door**

- [x] **`Ctrl-\` cannot be produced by either backend, so nothing exits
      Terminal-Insert.** *(fixed)* `handle_terminal_key` (`app.rs:5182`) forwards every key
      to the PTY and returns, with one escape: `KeyCode::Char('\\')` +
      `CONTROL`. Neither backend can generate that event.
      - **TUI:** `Ctrl-\` sends byte `0x1C`, and crossterm 0.28 decodes
        `0x1C..=0x1F` as `Char('4'..='7')` + `CONTROL`
        (`crossterm-0.28.1/src/event/sys/unix/parse.rs:110`). The app asks for
        `Char('\\')`, which that path never yields. Nothing requests the kitty
        keyboard protocol (no `PushKeyboardEnhancementFlags` anywhere), so the
        legacy encoding is the only one in play.
      - **GUI:** `modified_char_for_key` (`ruster-render-raylib/src/key.rs:41`)
        has no `KEY_BACKSLASH` arm and `map_raylib_key` does not map it either,
        so with Ctrl held the backslash key produces no event at all.

      Consequence: after `:term` focuses the terminal, every keystroke goes to
      the shell. No `:` commands, no `Ctrl-w` window nav, no way back to the
      file. The only exits are `exit` in the shell or quitting ruster.

      Two tests assert this works —
      `ctrl_backslash_enters_terminal_normal_and_mirrors_output` and
      `ctrl_backslash_defocuses_the_terminal` — by synthesising a `KeyEvent`
      neither backend can produce. They are the reason this survived.

      **Fixed.** `is_terminal_escape` accepts what each backend actually sends,
      `modified_char_for_key` maps the bracket family, and `terminal.escape`
      makes the key configurable (`<Esc>` for evil/vterm-style controls).
      `I`/`A` join `i`/`a`/`Enter` on the way back in. Verified live: in the
      TUI, `Ctrl-\` round-trips TERMINAL → NORMAL → `gg` moves → `i` →
      TERMINAL; in both backends `terminal.escape = "<Esc>"` does the same.

      **`Ctrl-\` in the GUI is untested by hand.** System Events cannot
      deliver Ctrl chords to a GLFW window, so the harness cannot reach it;
      a real `Ctrl-w v` was confirmed working, which establishes that Ctrl
      chords themselves are fine there. The `KEY_BACKSLASH` mapping the escape
      needed was unambiguously absent and is now present with tests, so the
      remaining risk is small — but it is one keypress from certainty.

- [x] **Typing a `:` command in Terminal-Normal leaked into the shell.**
      *(fixed)* The `i`/`a`/`I`/`A` re-focus check runs before the cmdline is
      handled, so `:echo hi` re-focused the terminal on the `i` of "hi" and sent
      `-from-cmdline` to zsh, which answered `command not found`. Now gated on
      `vim.is_normal_idle()` and no pending flash jump — the same hazard covers
      `r`, a pending operator, and a flash label, all of which leave the mode
      `Normal` while waiting for a character that may be `i`. Found by driving
      the terminal through a PTY, not by reading the code.

- [x] **Ctrl chords in the GUI: not a defect.** *(closed)* Recorded here
      briefly because the evidence pointed the wrong way and the next session
      should not re-run it. `Ctrl-w v` sent to the raylib window by osascript
      leaves the editor in VISUAL mode — the `v` lands, the `C-w` does not —
      and that held across `keystroke ... using control down`, `key code ...
      using control down`, and `key down control` / `key code` / `key up
      control` with 150ms holds. A real keypress splits the window correctly.
      So System Events cannot deliver Ctrl chords to a GLFW window, and no
      capture from `scripts/gui-keys.sh` is evidence about a `C-` binding in
      either direction. Documented in that script and in
      `docs/verification/README.md`.

- [ ] **Terminal scrollback is retained and unreachable.** `terminal.scrollback`
      defaults to 10000 lines and alacritty_terminal keeps them, but
      `TerminalSession::snapshot` (`ruster-terminal/src/lib.rs:219`) reads only
      `grid.screen_lines()` — the visible viewport — so Terminal-Normal mirrors
      one screen. `PageUp`/`PageDown` are forwarded to the shell rather than
      scrolling the viewport. There is no way to see output that has scrolled
      off, which makes the setting a promise the editor cannot keep.

- [ ] **Terminal-Normal is a frozen copy, not a live view.**
      `enter_terminal_normal` snapshots the grid into a buffer once; the shell
      keeps running behind it and the text goes stale with no indication.

**Retracted — three defects I reported that were not real**

Recorded rather than deleted, because each was published as blocking and the
way each was mis-diagnosed is the reusable lesson.

- [x] **"The settings page draws nothing in the GUI."** *Wrong — it renders
      correctly.* `:screenshot` closed the page before the shot fired: every
      command other than a few closes the settings page, deliberately, and the
      capture recipe queues `:screenshot`. Ordering the shot *before*
      `:settings` shows the overlay in full. **Real bug found underneath, and
      fixed:** photographing a page should not dismiss it, so
      `CmdAction::Screenshot` is now exempt from the close rule.

- [x] **"`:Noice popup` produces no float."** *Wrong — it renders correctly,
      with border, title and text, centred.* I checked the capture with `head
      -8`; the float sits at lines 19–21 of a 40-line pane. The committed
      artifact had it all along.

- [x] **"`:hover` shows no float against a live rust-analyzer."** *Wrong — it
      works.* Verified three times by hand against the fixture project: the
      float shows `p: Point` and persists indefinitely. The sweep fired
      `:hover` two thirds of the way into the wait, before a cold
      rust-analyzer had indexed a freshly copied project. `DEFER` now fires
      2.5s before the capture instead. **A harness limitation remains:** the
      scripted hover capture is still unreliable against a throwaway project
      path for reasons I could not pin down (a doubled slash in the path was
      ruled out). Reproduce by hand; see `docs/verification/README.md`.

**The lesson, since it caused all three:** reading a capture with `head`, or
photographing a surface with a command that dismisses it, produces exactly the
same artifact as a backend that cannot draw the surface. Read captures whole,
and prefer `drive.rs` — which asserts on `FrameState` — for "is it there at
all", before believing a picture.

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
