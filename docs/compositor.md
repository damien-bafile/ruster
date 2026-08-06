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
no client at all, and killed its own client on startup. Treat a ⛔ here as an
untested claim, not a passing one.

| Criterion | Check | Result |
| :--- | :--- | :--- |
| Workspace builds | `cargo build` | ✅ passes |
| All crates test clean | `cargo test` | ✅ passes |
| Clippy clean | `cargo clippy --all-targets -- -D warnings` | ✅ passes |
| Shell state unit tests | `cargo test -p ruster-shell` | ✅ passes |
| Render-gles unit tests | `cargo test -p ruster-render-gles` | ✅ passes |
| Compositor unit tests | `cargo test -p ruster-compositor` | ✅ 34 passed |
| Compositor unit tests (udev) | `cargo test -p ruster-compositor --features ruster-compositor/udev` | ✅ 40 passed |
| Winit compositor boots | `just compositor` | ✅ boots, GLES renderer up, socket `wayland-1` |
| Client maps & composites | `just compositor` + auto-launched `foot` | ✅ foot maps and composites fullscreen (needed `wl_data_device_manager`, `on_commit_buffer_handler`) |
| Frame is the right way up | prompt reads top-left, not mirrored | ✅ fixed by `Transform::Flipped180` on the winit output |
| Chrome contents visible | accent segment + glyphs over their backgrounds | ✅ fixed by reversing the chrome element order |
| Editor frame + which-key visible | visual check | ✅ which-key top-left, editor frame centred with accent titlebar |
| Chrome text legible | statusline reads `N  WS 1  <title>` | ✅ real glyphs, rasterized through `cosmic-text` into the atlas |
| Titlebar chrome updates on focus | launch a second client, its title takes the statusline | ✅ statusline went from the shell's title to `RUSTER-FOCUS-TEST` when that client mapped and took focus |
| Lua keybinds work | `M-t` cycles WS label; `M-S-q` quits | ⛔ not run — no key-injection tool on this box. Bind parsing and action dispatch are covered by `cargo test -p ruster-compositor`; the keypress path itself is unverified |
| DRM boots (hardware) | `just compositor-drm` on a free VT | ⛔ not run — requires a free VT + seatd/logind |
| SIGINT quits cleanly | `Ctrl-C`, process exits 0 | ✅ exits 0, logs `shutting down` |

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
