# Verification captures

Every user-visible surface, photographed in both backends.

Phases 0–9 shipped ~880 tests that prove the code does what it claims. None of
them proved it *looks* like anything: there is no `ratatui::TestBackend` in the
tree, `render()` was only ever checked for not panicking, and TUI/GUI parity was
guarded by comparing byte offsets of marker strings in two source files. These
artifacts are the other half — the things a test cannot assert: legibility,
glyph fallback, theme colour, pane alignment, and whether the two backends
actually agree.

## Running it

```
just verify                 # every surface, both backends
just verify sidebar         # one surface
just verify "--tui hover"   # one backend
just verify --list          # the surface names
```

`scripts/verify-capture.sh` holds one spec per surface — what to open, which ex
commands to queue, which keys to send, which service it needs — and both halves
read it, so a TUI/GUI difference is evidence rather than an artefact of the two
having been driven differently.

**The GUI half needs an unlocked screen.** macOS will not create a window for a
locked session; the script says so instead of letting raylib panic with
"Attempting to create window failed!", which names neither the cause nor the
fix. The screen re-locks on an idle timer, so a full run can hit this partway
through.

## How each half works

**TUI** — a fresh `tmux` session at 120×40 with `-f /dev/null` (the user's
`tmux.conf` must not decide what the artifact looks like), then
`capture-pane -p`.

**GUI** — a throwaway `XDG_CONFIG_HOME` whose `init.lua` queues the drive
commands, a deferred `:screenshot`, and a deferred `:q!`. `:q!` sets
`should_quit` unconditionally, so the run ends cleanly and a `timeout` firing is
a real failure rather than the expected outcome. (A plain `:q` closes a window
first when a surface opened a second one, so it would not exit with the sidebar
or a split on screen.)

**Keys** — which-key, flash, multi-cursor and cmdline completion only exist
*between* keystrokes, and no `init.lua` can produce them. tmux `send-keys`
drives the TUI; `scripts/gui-keys.sh` drives the GUI through macOS System
Events, which needs Accessibility permission for whatever runs it.

**Ctrl chords do not reach the GUI through System Events.** Sending `C-w v`
leaves the editor in VISUAL mode — the `v` lands, the `C-w` does not. A `C-`
capture from `gui-keys.sh` is therefore evidence of nothing in either
direction. Verify Ctrl chords in the TUI, where tmux puts real bytes through a
PTY, or by hand.

For *behaviour* rather than pixels, prefer the headless layer:
`ScriptedRenderer` (`crates/ruster-render/src/script.rs`) plays a key script
through the real `run_gui` loop and records every frame, and
`crates/ruster-tui/tests/drive.rs` asserts on them. That runs in CI; this does
not.

## Three things the fixtures encode

**`fixtures/demo.rs`** is opened by most surfaces, so two runs are comparable.
It carries a `TODO`, a `FIXME`, a repeated identifier for multi-cursor and
enough lines for the gutter to be non-trivial.

**`fixtures/demo-project/`** is a real, buildable cargo project for the
surfaces that need a live service. rust-analyzer roots itself at a project and
answers `null` for anything outside it, and lldb-dap needs a binary to launch —
a loose file gets an empty capture that looks like a broken feature. Its line
numbers are load-bearing; the file says which.

**Settings a capture merely needs *on* go in `config.lua`, not `:set`.** `:set`
echoes a confirmation toast, which then sits in the artifact pretending to be
part of the surface.

**Anything depending on a live service goes in `DEFER`, not `LUA`.** The `LUA`
queue is applied before the first frame, so a `:hover` there asks a language
server that has not finished indexing and gets an honest `null` back — which
renders as "No hover info" and reads exactly like the feature being broken.

## The surfaces

Status is what the artifacts currently show, not what is intended.

| Surface | Driven by | What to look for | Status |
|---|---|---|---|
| `dashboard` | bare launch | recent projects, quick actions, LSP status | ok (`:FuzzySearch` corrected to `:Files`) |
| `editor` | fixture + gutter on | syntax colour, gutter alignment | ok |
| `statusline` | `:16` | mode, project, file, percent, line:col | ok (collision fixed) |
| `gutter` | scratch repo, `:Gitsigns`, `:TodoList` | git signs, TODO signs, relative numbers | ok |
| `sidebar` | `:sidebar` | tree glyphs `▸`/`▾`, second window | ok |
| `dired` | `:Dired` | directory listing, trailing `/` on dirs | ok |
| `ibuffer` | `:e`, `:ls` | buffer list picker | ok |
| `whichkey` | `Space` | leader groups, key letters in the accent colour | ok |
| `whichkey-accent` | `Space` | `whichkey_key` distinct from `whichkey_fg` (Phase 9 T1) | ok |
| `cmdline` | `:e /tmp/` + `Tab` | cmdline row, completed path, CMDLINE mode | ok |
| `flash` | `f` | two-character jump labels over the buffer | ok |
| `multicursor` | `:52`, `w`, `C-n` ×2 | additional carets on the repeated identifier | ok |
| `git-status` | scratch repo, `:Git` | branch, Staged/Unstaged sections | ok |
| `git-staged` | scratch repo, `:GitStaged` | unified diff of the index | ok |
| `diffview` | scratch repo, `:Diffview` | two aligned panes, separator | ok |
| `trouble` | fixture project, deferred `:Trouble` | grouped diagnostics and markers | ok |
| `todos` | `:TodoList` | picker with preview | ok (positions were one low; fixed) |
| `settings` | `:settings` | grouped rows, controls, values | ok — see note below |
| `themes` | `:Themes` | theme list, live preview on move | ok |
| `help` | `:help` | long markup buffer | ok |
| `messages` | `:echo` ×2, `:messages` | the two messages in the log | ok (`:echo` now reaches the log) |
| `mason` | `:Mason` | `✓`/`·` glyphs, install commands | ok |
| `projects` | `:projects` | recent project list | needs a persistent config dir; warns otherwise |
| `noice-toast` | `:echo` | mini toast, top right | ok |
| `noice-panel` | `:echo` ×2, `:Noice` | stacking panel | shows only `Notify`-routed messages; `:echo` routes to `Mini` |
| `noice-popup` | `:Noice popup` | centred popup float | ok |
| `dialog` | `ruster.ui.dialog` | modal above every float | ok |
| `hover` | fixture project, deferred `:hover` | float with rustdoc, wrapped and clamped | works by hand; scripted capture unreliable — see note |
| `debugger` | fixture project, breakpoint, deferred `:debug` | `[Debug: PAUSED]`, call stack, scopes | ok (runtime frames now folded) |
| `terminal` | `:term` | shell prompt, TERMINAL mode | ok |
| `sessions` | `:SessionSave` | confirmation in the log | needs a project root |
| `gotoline` | `:16` | cursor and statusline agree | ok |

## Two ways a capture lies

Both of these produced artifacts indistinguishable from a broken backend, and
both fooled me into filing defects that were not real.

**A capture read with `head` is not a capture read.** The `:Noice popup` float
sits at lines 19–21 of a 40-line pane, because floats are centred. Read the
whole artifact, and prefer `crates/ruster-tui/tests/drive.rs` — which asserts on
`FrameState` — to answer "is the surface there at all".

**A command can dismiss the surface you are photographing.** Every command bar a
few closes the settings page, and the GUI recipe queues `:screenshot`, so the
page was gone before the shot fired. `:screenshot` is now exempt, but the class
remains: if a surface is missing from a GUI capture and present in the TUI, check
what the recipe issued before believing the backend.

**Known limit — the `hover` row.** `:hover` works: driven by hand against
`fixtures/demo-project/` the float shows `p: Point` and persists. The scripted
capture against a throwaway project path is unreliable anyway, and I could not
pin down why (a doubled slash in the temp path was ruled out). Verify it by
hand; do not read an empty hover capture as a broken feature.

`:Browse` and `:Music` (Phase 9 tasks 6 and 5) have no rows: neither is built.
