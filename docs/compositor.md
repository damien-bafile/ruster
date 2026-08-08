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
| Screenshot works on DRM | press the screenshot bind on a hardware boot | ⛔ **written, unproven.** It did nothing at all before: `backend/drm.rs` never read `screenshot_pending`, so the action fired, logged nothing, wrote nothing — found by mining a hardware log for a PNG that was never written. Now implemented via `blit_frame_result` into an offscreen texture. It compiles and clippy is clean, which is exactly the standard that let the original gap through, so this row stays ⛔ until a boot produces a file. Two things to check when it does: that the image is not black (the blit is async and is waited on, but that wait is untested) and that it is the right way up (`Transform::Normal` is passed because the DRM output carries no transform, unlike winit) |
| A window can be opened from inside the session | press the spawn bind | ✅ nested via config; on DRM this is the only way to get a second window, since there is no outer compositor to launch one from and the startup clients are all you otherwise get |
| Focus is visible on screen | tile two windows, move focus, sample the pixels at the seam | ✅ nothing said which window took the next keystroke before this. Captured from the host with `grim` and read by value, not by eye: across the tile boundary the two 2px bands are `(49,50,68)` then `(203,166,247)`, and after `focus left` they swap to `(203,166,247)` then `(49,50,68)` — so the accent follows focus rather than window order. `docs/verification/compositor-focus-border.png` |
| Compositor uses the configured theme | boot it under a config with a theme and sample a chrome pixel | ✅ the borders came back `(203,166,247)` and `(49,50,68)` — Catppuccin Mocha's `accent` and `divider` from `config.lua`. `Theme::default()`'s divider is `(69,71,90)`, so the values alone prove which palette was used. Before this it drew with `Theme::default()` while `ruster-lua` sat unimported in its own Cargo.toml, and no theme or colour override reached compositor chrome at all |
| The keyboard layout is configurable | boot under `keyboard = { layout = "de" }` and press the physical `z` key | ✅ reports `key=y`, and `key=z` with no keyboard table — the same physical key, so the keymap really changed rather than merely parsing. It was `XkbConfig::default()` with a `TODO` beside it before, which made every non-US layout wrong with nowhere to say so |
| A bad layout cannot strand the session | boot under a layout xkb rejects | ✅ `the configured keymap was rejected; keeping the current one`, and the compositor carries on. It would otherwise have hit the `expect` at seat creation — on DRM that is a black screen and no keyboard with which to fix the file that caused it |
| Lua can drive the WM at runtime | a config calling `ruster.wm.*` | ✅ `action("workspace 3")`, `focus("left")` and `spawn("foot")` each reached `dispatch`, and the spawn produced a mapped toplevel. Every call queues an `Action` the event loop hands to the same `dispatch` a keybind uses, so the two cannot drift |
| Chord sequences resolve | bind `M-F1 h` and press both | ✅ `M-F1` alone leaves the sequence pending; `h` after it dispatches `Focus(Left)`. Bindings were one chord with nowhere to keep a half-typed sequence before |
| Which-key appears only when it has something to say | press a prefix, then let it lapse | ✅ the panel pixel is `(30,30,46)` — `whichkey_bg` — while `M-F1` is pending, and `(36,36,36)` (the client behind it) when nothing is. It used to be drawn every frame from a hardcoded pair, permanently on screen and never about anything |
| Which-key distinguishes key from description | look at a pending overlay | ✅ the key is drawn in `whichkey_key` and the description in `whichkey_fg`. They were concatenated into one string and therefore one colour, so the key you had to press did not stand out |
| Screenshot costs a frame | capture nested, watch the log | ⚠️ every capture is followed by one `Failed to submit buffer: The context has been lost`; the next frame renders normally and the PNG is correct. A one-frame stutter per screenshot, not a lost session |
| DRM fails gracefully without a seat | `--drm` inside a Wayland session | ✅ exits 1 with `failed to initialize libseat session` plus the seatd/logind hint; display untouched |
| DRM boots (hardware) | `bash scripts/drm-test.sh` on a free VT | ✅ booted on seat0: `/dev/dri/card1`, connector DP-3, mode `3440x1440@60`, GLES on the RTX 4090, socket `wayland-1` |
| DRM launches its startup client | the Lua config's client appears on a DRM boot | ✅ `new toplevel` → `toplevel mapped`, and the client was usable |
| DRM keyboard (libinput) | type, and use the quit binding | ✅ 95 key events routed through the seat; `Super+Shift+q` quit the compositor |
| DRM pointer (libinput) | move a mouse | ⛔ not confirmed on hardware — relative-motion handling is still unexecuted there, and nothing logs pointer events |
| Cursor is drawn | move the pointer over chrome, then over a client | ✅ nested: the built-in arrow renders over chrome, and a client's own cursor surface (foot's I-beam) renders with its hotspot applied once the pointer enters it |
| `Ctrl+Alt+F<n>` switches VT | press it on a DRM boot | ⛔ **not exercised** — the boot was ended with the quit binding instead, so no `XF86Switch_VT` keysym ever reached the handler. This is the escape hatch; it is unproven |
| VT suspend/resume | switch away from a DRM boot and back | ⛔ not run — no `pausing session`/`resuming session` in the log |
| DRM restores the previous state on exit | quit and check the log | ⛔ **fails**: `Failed to restore previous state. Error: Invalid argument (os error 22)` from smithay's atomic teardown |

## Running the real thing

```bash
# Winit (nested, dev): boots a window on a running display server.
just compositor

# DRM (hardware): needs a free VT and seatd/logind access.
just compositor-drm
```

The `just compositor` / `just compositor-drm` recipes are defined in the
root `justfile` and map to `cargo run -p ruster-compositor` and
`cargo run -p ruster-compositor --features ruster-compositor/udev -- --drm`
respectively.
