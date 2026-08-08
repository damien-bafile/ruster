# Compositor Verification Matrix

The Phase 0 Wayland compositor's acceptance criteria and how to check each one.
This page is the manual test plan for the `ruster-compositor` crate — see
[windows.md](windows.md) and [keybindings.md](keybindings.md) for the rest of
the platform and key documentation.

## The matrix

| Criterion | Check |
| :--- | :--- |
| Workspace builds | `cargo build` |
| All crates test clean | `cargo test` |
| Clippy clean | `cargo clippy --all-targets` |
| Shell state unit tests | `cargo test -p ruster-shell` |
| Render-gles unit tests | `cargo test -p ruster-render-gles` |
| Winit compositor boots | `just compositor` |
| Client maps & composites | `just compositor` + auto-launched `foot` |
| Titlebar chrome updates on focus | focus `foot`, title shows in statusline |
| Lua keybinds work | `M-t` cycles WS label; `M-S-q` quits |
| Editor frame + which-key visible | visual check at 1080p |
| DRM boots (hardware) | `just compositor-drm` on a free VT |
| SIGINT quits cleanly | `Ctrl-C`, process exits 0 |

## Results on this machine

Run nested under a Wayland session (`mango`, NVIDIA RTX 4090, GLES 3.2), on a
`1873x1334` winit window. The DRM rows still need a free VT and are not covered
here.

An earlier pass of this table was recorded on a headless box and marked every
display row ⛔ "not run". Those rows were the ones hiding the bugs: the first
time the compositor was actually put on a screen it rendered upside down, drew
no client at all, and killed its own client on startup. Later, the first time a
key was actually pressed, it turned out the Lua config could not bind anything —
the matcher recognised two hardcoded strings and discarded the configured action
name. Treat a ⛔ here as an untested claim, not a passing one.

Note on driving the nested compositor: `wtype` cannot do it. It uploads its own
xkb keymap to the *host*, and a nested compositor receives raw keycodes which it
resolves with its own keymap, so keysyms arrive scrambled — typing `echo hello`
into the nested client produces `12342555553678396`. Injecting on a virtual
evdev device instead makes the host see an ordinary keyboard and forward
ordinary keycodes.

| Criterion | Check | Result |
| :--- | :--- | :--- |
| Workspace builds | `cargo build` | ✅ passes |
| All crates test clean | `cargo test` | ✅ passes |
| Clippy clean | `cargo clippy --all-targets -- -D warnings` | ✅ passes |
| Shell state unit tests | `cargo test -p ruster-shell` | ✅ passes |
| Render-gles unit tests | `cargo test -p ruster-render-gles` | ✅ passes |
| Compositor unit tests | `cargo test -p ruster-compositor` | ✅ 41 passed |
| Compositor unit tests (udev) | `cargo test -p ruster-compositor --features ruster-compositor/udev` | ✅ 46 passed |
| Winit compositor boots | `just compositor` | ✅ boots, GLES renderer up, socket `wayland-1` |
| Client maps & composites | `just compositor` + auto-launched `foot` | ✅ foot maps and composites fullscreen (needed `wl_data_device_manager`, `on_commit_buffer_handler`) |
| Frame is the right way up | prompt reads top-left, not mirrored | ✅ fixed by `Transform::Flipped180` on the winit output |
| Chrome contents visible | accent segment + glyphs over their backgrounds | ✅ fixed by reversing the chrome element order |
| Editor frame + which-key visible | visual check | ✅ which-key top-left, editor frame centred with accent titlebar |
| Chrome text legible | statusline reads `N  WS 1  <title>` | ✅ real glyphs, rasterized through `cosmic-text` into the atlas |
| Titlebar chrome updates on focus | launch a second client, its title takes the statusline | ✅ statusline went from the shell's title to `RUSTER-FOCUS-TEST` when that client mapped and took focus |
| Keyboard reaches the client | type into the nested client | ✅ keys are forwarded to the focused toplevel |
| Lua user config is loaded | `~/.config/ruster/compositor.lua` binds a key | ✅ a config binding `M-F9`/`M-F10` was loaded and both binds took effect |
| `ruster.wm.*` API works | a config that only calls the API | ✅ `set_keybind` + `switch_workspace(4)` + a branched `launch_client` all took effect: statusline showed `WS 4`, the client launched, and the API-registered `M-F9` moved it to `WS 5`. `focus` warns and does nothing by design |
| Lua keybinds work | cycle binding changes the WS label; quit binding exits 0 | ✅ injected `Super+F9` at the evdev level → statusline went `WS 1` → `WS 2`; `Super+F10` → process exits 0 |
| SIGINT quits cleanly | `Ctrl-C`, process exits 0 | ✅ exits 0, logs `shutting down` |
| Layout actions are reachable from a keyboard | drive each bound action nested, and read the geometry the compositor logs | ✅ all six, by the numbers: `focus left` moved focus `1`→`0`; `swap right` exchanged their rects; `resize right` stepped the boundary `937`→`1030`→`1124`→`1217`; `split vertical` restacked the pair; `toggle floating`, `workspace N` and `move to workspace N` each changed the visible set. Screenshots agree with the log |
| Actions no-op only where designed | `resize`/`swap` against the outer edge | ✅ both do nothing on the window with no neighbour that way, which is what `Tree::resize` documents. Indistinguishable from a dead keybind on screen, which is why `dispatch` now logs the resulting geometry |
| Which-key tells the truth | run a config that binds nothing it used to advertise | ✅ the overlay listed all 11 binds of a config that bound neither `M-t` nor `M-S-q`, and the welcome frame named that config's real quit (`M-F12`). Both were hardcoded to `M-t`/`M-S-q` before, and both were simply wrong on any custom config |
| Chrome renders every character | draw a word the font would ligate | ✅ `toggle floating` reached the screen as `toggle f oating` — advanced shaping collapses `fl` to one ligature glyph, and the atlas is keyed by `char`, so the `l` was dropped while the pen advanced the full width. Fixed by shaping `Basic`; guarded by a test over `floating`/`file`/`office`/`flat` |
| Screenshot works on DRM | press the screenshot bind on a hardware boot | ✅ `screenshot path=/run/user/1000/ruster-shot-1.png`, 3440x1440, 100% non-black. It did nothing at all before — `backend/drm.rs` never read `screenshot_pending` — which was found by mining a hardware log for a PNG that was never written |
| The DRM capture is the right way up | look at one | ✅ the first capture ever taken came out vertically mirrored — `capture` flipped rows unconditionally because a direct GL framebuffer read is bottom-left first, but the DRM path reads back a texture `blit_frame_result` already wrote top-left. Flip is now per-backend, and the next boot's capture reads statusline-at-the-bottom, welcome frame upright |
| A window can be opened from inside the session | press the spawn bind | ✅ nested via config; on DRM this is the only way to get a second window, since there is no outer compositor to launch one from and the startup clients are all you otherwise get |
| Focus is visible on screen | tile two windows, move focus, sample the pixels at the seam | ✅ nothing said which window took the next keystroke before this. Captured from the host with `grim` and read by value, not by eye: across the tile boundary the two 2px bands are `(49,50,68)` then `(203,166,247)`, and after `focus left` they swap to `(203,166,247)` then `(49,50,68)` — so the accent follows focus rather than window order. `docs/verification/compositor-focus-border.png` |
| Compositor uses the configured theme | boot it under a config with a theme and sample a chrome pixel | ✅ the borders came back `(203,166,247)` and `(49,50,68)` — Catppuccin Mocha's `accent` and `divider` from `config.lua`. `Theme::default()`'s divider is `(69,71,90)`, so the values alone prove which palette was used. Before this it drew with `Theme::default()` while `ruster-lua` sat unimported in its own Cargo.toml, and no theme or colour override reached compositor chrome at all |
| The keyboard layout is configurable | boot under `keyboard = { layout = "de" }` and press the physical `z` key | ✅ reports `key=y`, and `key=z` with no keyboard table — the same physical key, so the keymap really changed rather than merely parsing. It was `XkbConfig::default()` with a `TODO` beside it before, which made every non-US layout wrong with nowhere to say so |
| A bad layout cannot strand the session | boot under a layout xkb rejects | ✅ `the configured keymap was rejected; keeping the current one`, and the compositor carries on. It would otherwise have hit the `expect` at seat creation — on DRM that is a black screen and no keyboard with which to fix the file that caused it |
| Lua can drive the WM at runtime | a config calling `ruster.wm.*` | ✅ `action("workspace 3")`, `focus("left")` and `spawn("foot")` each reached `dispatch`, and the spawn produced a mapped toplevel. Every call queues an `Action` the event loop hands to the same `dispatch` a keybind uses, so the two cannot drift |
| Chord sequences resolve | bind `M-F1 h` and press both | ✅ `M-F1` alone leaves the sequence pending; `h` after it dispatches `Focus(Left)`. Bindings were one chord with nowhere to keep a half-typed sequence before |
| Which-key appears only when it has something to say | press a prefix, then let it lapse | ✅ the panel pixel is `(30,30,46)` — `whichkey_bg` — while `M-F1` is pending, and `(36,36,36)` (the client behind it) when nothing is. It used to be drawn every frame from a hardcoded pair, permanently on screen and never about anything |
| Which-key distinguishes key from description | look at a pending overlay | ✅ the key is drawn in `whichkey_key` and the description in `whichkey_fg`. They were concatenated into one string and therefore one colour, so the key you had to press did not stand out |
| The `:` prompt runs commands | bind `command`, type `focus left`, press Enter | ✅ `dispatch action=Prompt(Command)` on open, then `dispatch action=Focus(Left)` on submit. The line resolves through `Action::from_name`, the same function a keybind uses, so the prompt cannot grow a vocabulary the keymap lacks |
| The prompt's sigil is accented | look at an open prompt | ✅ the two dots of the `:` are the only accent-hued pixels in the bar — `(179,147,220)`, `cmdline_accent` blended over `cmdline_bg` — against 292 text-hued pixels for `focus left`. Judged by hue rather than by eye: a two-dot glyph is too thin to read, and an exact-colour match misses anti-aliased blends |
| The helper shows every shortcut | pin it with the full shipped config | ✅ all 43 bindings listed, full sequences and all, nothing truncated. It reads `Keymap::all_bindings` rather than the root continuations, which collapse `M-w h` and `M-w l` into one `+h` row a user cannot type from. The panel wraps into columns when a keymap is taller than the output, so growing the keymap cannot push entries off. `docs/verification/compositor-help-all-keys.png` |
| A shortcut added at runtime appears | call `ruster.wm.set_keybind` from a live session | ✅ `M-F9 screenshot` is in the pinned list on the first frame after the call. `set_keybind` recorded into a struct read once at startup before this, so setting a keybind from a live session silently did nothing |
| The shortcut helper toggles | bind `toggle help` and press it twice | ✅ the panel pixel is `(30,30,46)` — `whichkey_bg` — after the first press and `(36,36,36)` (the client behind it) after the second, with two `ToggleHelp` dispatches in the log. Pinned, it lists the whole keymap under the title `keys`; a half-typed chord still takes the panel over while it lasts. `docs/verification/compositor-help-pinned.png` |
| Primary selection, xdg-activation, cursor-shape and server-side decorations | check the globals are created and delegated | ✅ all four constructed in `init_globals` and delegated. They come off `foot`'s startup complaint list from a hardware boot, which is a free inventory of what a real client misses. Wiring them also found that the **clipboard was silently dead**: `set_data_device_focus` was never called, so `wl_data_device.selection` had never fired for any client since the compositor was written |
| Workspace layouts survive a restart | quit and reboot the compositor | ✅ confirmed on hardware, a full round trip: `session saved windows=2 relaunchable=2`, then on the next boot `restoring session windows=2 relaunched=2` and both windows *put back at their positions in the tree*, not merely respawned into a default layout. The file is readable text — `ruster-workspaces 1`, the app and title per window, then `split h 2 0.5 0.5` / `leaf 0` / `leaf 1` — so a session that comes back wrong can be diagnosed by looking at it |
| Client menus and tooltips appear | right-click in a client that has a menu | ⛔ **written, unproven.** `new_popup` did nothing but log — popups were untracked, so every client menu existed as far as the client was concerned and was never drawn. Now tracked, configured on commit, drawn in front of their parent, hit-tested for the pointer, and unconstrained against the output so a menu near an edge flips or slides back on screen instead of running off it. Needs a client with a real menu to confirm |
| Unfocused clients keep redrawing | run something animated in an unfocused tile, diff two captures 1.5s apart | ✅ **0 changed pixels before this, 30,742 after.** `send_frame_callbacks` served the focused window alone, so an unfocused client was not slow — it was frozen, with only the 1s backstop, and nothing said so. It also had a worse failure queued: once focus can be something that is not a client the lookup yields `None` and *nothing on screen* gets a callback, which is the whole desktop stopping. Now every visible window is served |
| Render-element budget is known | `RUSTER_BENCH_GLYPHS=n`, read the logged frame time | ✅ measured, and it settles a design question. Release: flat at ~4.7ms from 0 to 10,000 extra glyph quads, 16ms at 50,000, 74ms at 200,000. An 80x40 editor pane is 3,200, so per-glyph quads are the right choice for Phase 3 and the per-row texture path planned as a fallback is not needed. Debug is ~5x worse and misses the 60Hz budget at 5,000 — a sluggish pane under `cargo run` should be re-checked in release before anyone optimises |
| An editor pane is a peer of a client | open one beside a window | ✅ Phase 3 Stage 1. A pane takes an ordinary `WindowId` and goes into the same tree as an ordinary leaf, so it lays out, gets a focus border and hit-tests with no change to `ruster-shell` at all — the `Node::Leaf` enum the plan expected to need was avoided, which keeps `tile_under` and `geometry` the same list by construction rather than by discipline. It draws an empty titled frame; buffers are Stage 2 |
| A pane survives a restart | quit with a pane on screen, boot again | ✅ saved as its own shape (`pane` / `title scratch`, beside `app foot`) and restored to its position — from a config that creates neither. Saving it as a window with no command would have been indistinguishable from one whose program could not be identified, and `Tree::rebuild` drops those, so every pane would have vanished silently. `docs/verification/compositor-pane-restored.png` |
| A pane shows a real buffer | `edit <path>`, then screenshot | ✅ Phase 3 Stage 2. `pane.rs` rendered inside the compositor in a tile — monospace, right-aligned line numbers, every character on the cell grid. Captured by the compositor's own screenshot rather than from the host, which is the only way to photograph a tile without guessing where the nested window is |
| A pane's text is on a fixed grid | look at any two lines | ✅ the body draws in `FontFamily::Mono` and lays out on `cell_metrics`, not on a chrome line height — the two are different numbers and mixing them drifts a row out of alignment every few lines. The chrome's own `line_h` field is gone; nothing read it once the frame moved to cell metrics |
| Screenshot costs a frame | capture nested, watch the log | ⚠️ every capture is followed by one `Failed to submit buffer: The context has been lost`; the next frame renders normally and the PNG is correct. A one-frame stutter per screenshot, not a lost session |
| DRM fails gracefully without a seat | `--drm` inside a Wayland session | ✅ exits 1 with `failed to initialize libseat session` plus the seatd/logind hint; display untouched |
| DRM boots (hardware) | `just compositor-drm` on a free VT | ✅ booted on seat0: `/dev/dri/card1`, connector DP-3, mode `3440x1440@60`, GLES on the RTX 4090, socket `wayland-1` |
| DRM launches its startup client | the Lua config's client appears on a DRM boot | ✅ `new toplevel` → `toplevel mapped`, and the client was usable |
| DRM keyboard (libinput) | type, and use the quit binding | ✅ 95 key events routed through the seat; `Super+Shift+q` quit the compositor |
| DRM pointer (libinput) | move a mouse | ✅ confirmed on hardware: the cursor is visible and follows the mouse, and `pointer button` events reach the seat. Relative motion had never executed anywhere — winit only sends absolute — and nothing logged it, so a boot could not report whether the mouse did anything; it now traces `dx`/`dy` at trace level, quiet enough not to drown the log the way the paused render loop did |
| Cursor is drawn | move the pointer over chrome, then over a client | ✅ nested: the built-in arrow renders over chrome, and a client's own cursor surface (foot's I-beam) renders with its hotspot applied once the pointer enters it |
| `Ctrl+Alt+F<n>` switches VT | press it on a DRM boot | ✅ **fixed, then confirmed on hardware.** Pressed on hardware, it did nothing: the log shows `key=F2 modified=XF86Switch_VT_2`, and the handler was testing the *raw* sym, which is plain `F2` and never in the `XF86Switch_VT_*` range. So VT switching had been dead since it was written. The old test called `vt_switch_target(Keysym::XF86Switch_VT_2)` directly — an input the real code never produced — and passed throughout. Now reads the modified sym, guarded by a test that drives Ctrl+Alt+F2 through the real seat and asserts the session was asked for VT 2, and confirmed on hardware: `switching virtual terminal vt=2` with the compositor still alive afterwards |
| VT suspend/resume | switch away from a DRM boot and back | ✅ six pauses and five resumes across repeated switches, libinput and DRM recovering each time. It also exposed a spin: while paused the render loop kept rendering at full rate, logging `PrepareFrame(DeviceInactive)` — 11,746 warnings in five minutes, 96% of the log. On a VT the log is the only diagnostic channel there is, so drowning it costs more than the wasted frames. `render_surface` now returns early while the session is inactive |
| DRM hands the device back cleanly on exit | quit and check the log | ✅ confirmed on hardware: `released the drm device`, zero `Failed to restore previous state`, clean teardown. Every exit used to log `ERROR Failed to restore previous state. Error: Invalid argument`. smithay's atomic `Drop` replays every property captured at startup in one commit — connectors, CRTCs, framebuffers, planes — and the kernel rejects it, almost certainly because the capture includes immutable properties (a connector's EDID and PATH among them) and an atomic commit that sets one is invalid by definition. The compositor now calls `DrmOutputManager::pause()` before anything drops, which is smithay's documented way to say it is finished with the fd and releases the DRM master lock. Nothing is lost: the restore never once succeeded, so the behaviour observed all along *was* the no-restore behaviour, and dropping master is what the console driver actually waits for. The error is gone because the commit is no longer attempted |

## Running the real thing

```bash
# Winit (nested, dev): boots a window on a running display server.
just compositor

# DRM (hardware): needs a free VT and seatd/logind access. Builds, logs to
# /tmp/ruster-drm.log, and reports what happened once it exits.
just compositor-drm
```

The `just compositor` / `just compositor-drm` recipes are defined in the root
`justfile`. The first is `cargo run -p ruster-compositor`; the second runs
`scripts/drm-test.sh`, which builds with the `udev` feature, launches on DRM,
and afterwards reports the exit status, any screenshots and whether a VT switch
was seen — on a VT that summary is the whole diagnosis, since the screen is gone
by the time you can read anything.
