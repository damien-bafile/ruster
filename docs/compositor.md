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

Run on a headless box (no usable display server, no seatd/logind, no DRM
session), `rustc 1.97.1`, at commit `a5f8d85`. The non-hardware rows were
executed; the display/DRM rows could not be.

| Criterion | Check | Result |
| :--- | :--- | :--- |
| Workspace builds | `cargo build` | ✅ passes |
| All crates test clean | `cargo test` | ✅ passes |
| Clippy clean | `cargo clippy --all-targets -- -D warnings` | ✅ passes |
| Shell state unit tests | `cargo test -p ruster-shell` | ✅ 5 passed |
| Render-gles unit tests | `cargo test -p ruster-render-gles` | ✅ 6 passed |
| Compositor unit tests | `cargo test -p ruster-compositor` | ✅ 30 passed |
| Compositor unit tests (udev) | `cargo test -p ruster-compositor --features ruster-compositor/udev` | ✅ 34 passed |
| Winit compositor boots | `WAYLAND_DISPLAY=invalid cargo run -p ruster-compositor` | ⚠️ graceful non-zero exit (`failed to initialize winit backend: Failed to initialize an event loop`, exit 1) — no display in this environment |
| Client maps & composites | `just compositor` + auto-launched `foot` | ⛔ not run — requires a display |
| Titlebar chrome updates on focus | focus `foot`, title shows in statusline | ⛔ not run — requires a display |
| Lua keybinds work | `M-t` cycles WS label; `M-S-q` quits | ⛔ not run — requires a display (keybind config parsing is covered by `cargo test -p ruster-compositor`) |
| Editor frame + which-key visible | visual check at 1080p | ⛔ not run — requires a display |
| DRM boots (hardware) | `just compositor-drm` on a free VT | ⛔ not run — requires hardware + seatd/logind |
| SIGINT quits cleanly | `Ctrl-C`, process exits 0 | ⛔ not run — requires a booted compositor |

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
