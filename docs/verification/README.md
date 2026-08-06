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
| `dashboard` | bare launch | recent projects, quick actions, LSP status | ⚠ advertises `:FuzzySearch`, which no parser branch accepts |
| `editor` | fixture + gutter on | syntax colour, gutter alignment | ok |
| `statusline` | `:16` | mode, project, file, percent, line:col | ok wide; ⚠ GUI groups collide when narrow |
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
| `diffview` | scratch repo, `:Diffview` | two aligned panes, separator | ok; ⚠ GUI statusline collision |
| `trouble` | fixture project, deferred `:Trouble` | grouped diagnostics and markers | ok |
| `todos` | `:TodoList` | picker with preview | ⚠ line numbers one low |
| `settings` | `:settings` | grouped rows, controls, values | ⚠ **TUI only — draws nothing in the GUI** |
| `themes` | `:Themes` | theme list, live preview on move | ok |
| `help` | `:help` | long markup buffer | ok |
| `messages` | `:echo` ×2, `:messages` | the two messages in the log | ⚠ **empty — `:echo` never reaches the log** |
| `mason` | `:Mason` | `✓`/`·` glyphs, install commands | ok |
| `projects` | `:projects` | recent project list | needs a persistent config dir; warns otherwise |
| `noice-toast` | `:echo` | mini toast, top right | ok |
| `noice-panel` | `:echo` ×2, `:Noice` | stacking panel | ⚠ nothing appears |
| `noice-popup` | `:Noice popup` | centred popup float | ⚠ **nothing appears** |
| `dialog` | `ruster.ui.dialog` | modal above every float | ok |
| `hover` | fixture project, deferred `:hover` | float with rustdoc, wrapped and clamped | ⚠ **no float** |
| `debugger` | fixture project, breakpoint, deferred `:debug` | `[Debug: PAUSED]`, call stack, scopes | ok; ⚠ stack is 30+ std frames deep |
| `terminal` | `:term` | shell prompt, TERMINAL mode | ok |
| `sessions` | `:SessionSave` | confirmation in the log | needs a project root |
| `gotoline` | `:16` | cursor and statusline agree | ok |

Rows marked ⚠ are recorded in the Phase 9 cleanup plan. Per Phase 10's own
rule, this phase records and routes defects; fixing them is Phase 9's job.

`:Browse` and `:Music` (Phase 9 tasks 6 and 5) have no rows: neither is built.
