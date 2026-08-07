---
name: gui-check
description: Launch the raylib GUI, drive it to a given surface, and capture a PNG. Use when verifying how something looks in the GUI backend, or when a change touches ruster-render-raylib.
---

# Checking the GUI

The TUI is easy to verify — `tmux capture-pane` gives you the screen as text.
The GUI is not, and rediscovering how to drive it has cost multiple sessions.
This is the working recipe.

**For anything on the standard surface list, use the script instead:**
`just verify <surface>` (or `scripts/verify-capture.sh --list`) captures both
backends from one drive definition and writes the artifacts to
`docs/verification/`. The rest of this page is what that script does, and what
you need when the surface you want is not on the list.

## First: is the screen unlocked?

```bash
ioreg -n Root -d1 -a 2>/dev/null | grep -A1 CGSSessionScreenIsLocked
```

`<true/>` means **stop** — macOS refuses to create a window for a locked
session, GLFW enumerates zero monitors, and raylib panics with
`Attempting to create window failed!`. That panic says nothing about the real
cause. Ask the user to unlock; nothing else will work.

No output at all means unlocked.

The screen re-locks on an idle timer, so capture what you need promptly rather
than interleaving long analysis between shots.

## Driving it

`init.lua` is the control surface: `ruster.cmd()` queues an ex command, and
every queued action is applied before the first frame renders.

```bash
CFG=/tmp/guicheck; mkdir -p "$CFG/ruster"
cat > "$CFG/ruster/init.lua" <<'EOF'
ruster.cmd(":sidebar")
ruster.cmd(":Git")
ruster.defer(1200, function() ruster.cmd(":screenshot /tmp/shot.png") end)
ruster.defer(2100, function() ruster.cmd(":q!") end)
EOF

rm -f /tmp/shot.png
XDG_CONFIG_HOME="$CFG" timeout 15 ./target/debug/ruster path/to/file.rs >/dev/null 2>&1
```

Then look at it — `Read /tmp/shot.png` renders the image.

**The GUI can quit itself.** `:q!` sets `should_quit` unconditionally and
`run_gui` breaks on it, so a deferred quit ends the run with exit code 0. Keep
`timeout` as a backstop, but treat 124 as a *failure* — it means the deferred
quit never fired. (Use `:q!`, not `:q`: a plain `:q` closes a window first when
a surface opened a second one, so it would not exit with the sidebar or a split
on screen.)

Put the shot on a `ruster.defer` rather than in the queue whenever the surface
needs a round-trip — an LSP hover, a debugger launch, a terminal spawn. The
queue is applied before the first frame, which is too early for any of those.

For a modal dialog, `ruster.ui.dialog{...}` queues one the same way:

```lua
ruster.ui.dialog({ title = "Install?", fields = {
  { label = "Runs", kind = "text", value = "cargo install thing" },
  { label = "Install", kind = "button" },
}})
```

## Sending real keystrokes

Which-key, flash jump, multi-cursor and cmdline completion only exist *between*
keystrokes, and no `init.lua` can produce them. Two ways in:

- **`ScriptedRenderer`** (`crates/ruster-render/src/script.rs`) feeds a key
  script through the real `run_gui` loop headlessly and records every frame.
  This is what `crates/ruster-tui/tests/drive.rs` uses, and it is the reliable
  option — prefer it whenever you are checking *behaviour*.
- **`scripts/gui-keys.sh`** sends real keystrokes to the real window via macOS
  System Events, for when the pixels are the point. It needs Accessibility
  permission for whatever runs it; without that, System Events discards the keys
  silently and you get a screenshot of an undriven surface, so the script
  refuses up front rather than letting that happen.

## Two traps

**A command after `:settings` used to be swallowed.** Fixed, but if a capture
comes back without the surface you asked for, check whether something earlier in
the `init.lua` opened a modal.

**Order matters within one `init.lua`.** Everything queued is applied before the
first render, so `:screenshot` last captures all of it. Requesting the shot
first works equally well — the capture is deferred by a couple of frames.

## If the image is black or missing a surface

Both were real bugs in `:screenshot` and are fixed, but the symptoms are worth
recognising because they look like renderer faults and are not:

- **Entirely black** — the capture ran after `EndDrawing` swapped the buffers,
  or on a frame before the GL surface was ready.
- **Missing whatever was drawn last** — raylib batches draw calls and only
  submits them in `EndDrawing`. Reading pixels syncs GL but not that queue, so
  the newest draws are absent. `rlDrawRenderBatchActive()` before the read is
  the fix.

Both live in `capture_screen` in `crates/ruster-render-raylib/src/lib.rs`.

## What is worth capturing

Surfaces that only reach the screen through `ruster-render-raylib`, so a TUI
check says nothing about them:

| Command | What it exercises |
| --- | --- |
| `:sidebar` | the side panel and its `▸`/`▾` glyphs |
| `:Git`, `:GitStaged` | sectioned lists, diff colouring |
| `:Diffview` | two aligned panes and the `│` separator |
| `:Mason`, `:help`, `:Trouble` | long buffers, `✓`/`·` glyphs |
| `:settings` | the large centred overlay |
| `ruster.ui.dialog` | `draw_titled_box` — the shared border helper |
| `:echo text` | a mini toast |

Any glyph that appears must be in the font atlas, or raylib draws `?`. The guard
test `every_glyph_the_editor_draws_is_in_the_font_atlas` covers the known set —
add to it when introducing a new one.
