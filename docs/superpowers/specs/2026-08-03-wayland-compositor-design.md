# Ruster Wayland Compositor — EXWM-style Desktop Shell

**Date:** 2026-08-03
**Phase:** 0 (Compositor Foundation) — part of a new product roadmap
**Status:** Design

## Overview

Ruster becomes a Wayland compositor that is simultaneously the editor and the desktop, in the spirit of EXWM (Emacs X Window Manager). One binary boots directly on the GPU (DRM/KMS) as a Wayland compositor. Client applications connect over Wayland and their surfaces are composited into a single GL scene alongside Ruster's own chrome. Each workspace holds an i3-style container-tree whose leaves are either external client windows or Ruster editor buffers. Every window renders as an "editor-frame": the client surface plus a mode-line titlebar in the Starship palette. All control flows through Ruster — Lua config, keymaps, `:` commands, which-key, mini-buffer, statusline.

This spec covers the product definition and the overall architecture/roadmap. The first implementation plan (Phase 0) targets the compositor foundation: boot, map, composite, and bare chrome.

## Product Definition

- **Platform:** native Linux desktop; boot on DRM/KMS via the `udev` backend, with a `winit` nested backend for development (anvil's model).
- **Rendering:** one GL context for the whole desktop via Smithay's `glow` renderer. Ruster's chrome (statusline, which-key, mini-buffer, editor-frames, text) is drawn as textured quads and glyph-atlas text in the same GL scene as client surface textures. raylib is **not** used by the compositor; it remains only in the standalone editor builds.
- **Editor-in-desktop:** editor buffers are synthetic in-tree surfaces owned by the compositor (not real Wayland clients). They render directly through `ruster-render-gles`.
- **Control plane:** Lua (`mlua`) drives everything: keymaps, commands, startup clients, workspace management. Reuses `ruster-core`'s command/keymap engines, buffer/document model, and theme tokens.
- **Product name:** ruster (same brand).

### Locked visual decisions

- **Window chrome:** editor-frames with mode-line titlebars. Active frame gets the amber mode bar; inactive frames dim to `panel_gray`. Uses the existing Starship Crew Terminal palette.
- **Tiling:** full i3 container-tree model per workspace — binary split h/v, tabbed/stacked containers, resize, move-focus `hjkl`, swap, and a floating-window escape hatch.
- **Tree contents:** workspace trees hold **both** external client windows and Ruster editor buffers as leaves.
- **Chrome layout:** statusline bar at the bottom (mode, workspace tabs, focused window title, clock); which-key overlays on command prefixes; mini-buffer above the statusline.

## Architecture

### New crates (in the ruster workspace)

| Crate | Role | Notes |
| :--- | :--- | :--- |
| `ruster-shell` | Tiling/layout logic — container tree, workspaces, focus, floating, split/resize/swap. Pure, no GPU. | The "i3 brain"; unit-testable. |
| `ruster-render-gles` | GL compositor renderer (Smithay `glow`): composites client surface textures and draws Ruster chrome (statusline, which-key, editor-frames, text) via glyph-atlas text + quad primitives. | The raylib→GL port lives here. |
| `ruster-compositor` | The compositor binary + Smithay wiring: backend (udev/DRM + winit nested), seat/input, `xdg_shell`, `layer_shell`, clipboard, session. | Composition root. |

### Reused from ruster-core

- Lua runtime (`mlua`), command framework, keymap engine.
- Buffer/document model for the embedded editor.
- Theme tokens / palette.

### Dependencies

- `smithay` (path to `~/Dev/smithay` clone or crates.io `0.7`) with features: `backend_udev`, `backend_drm`, `backend_gbm`, `backend_session_libseat`, `backend_libinput`, `backend_winit`, `renderer_gl` (provides the `GlesRenderer`), `desktop`, `wayland_frontend`.
- Text rendering: `cosmic-text` (or `fontdue` + `fontdb`).
- Logging: `tracing`/`env_logger`.

### Data flow (Phase 0)

```
client (xdg-shell toplevel)
   │ wayland surface commits
   ▼
ruster-compositor: XdgShellHandler → ShellState (map/configure/commit/unmap)
   ▼
ruster-render-gles: GlowRenderer composites [client textures] + [chrome quads/text]
   ▼
udev/DRM or winit output ← vsync frame
```

### Key decisions

1. **Not winit+raylib for rendering.** A compositor must composite arbitrary client GL textures; that is Smithay's `egl`/`gles` domain. raylib stays only in standalone editor builds.
2. **Editor buffers are synthetic in-tree surfaces** owned by the compositor, not real Wayland clients — they render via `ruster-render-gles` into the same GL scene as client textures.
3. **Smithay dependency:** path-depend on the local `~/Dev/smithay` clone, or crates.io `0.7`. (Decision: path dep for development; revisit at release.)
4. **Entry point:** a dedicated `ruster-compositor` binary crate keeps `ruster-bin`'s editor path clean.

## Roadmap (phased plans)

- **Phase 0 (this plan): Compositor foundation.** Boot on DRM + winit-nested dev; map `xdg-shell` toplevels; composite client textures with glow; keyboard/pointer seat + basic focus; bare Ruster chrome (statusline + one editor frame + which-key skeleton); Lua config binds keys and launches clients.
- **Phase 1: Shell layout.** The i3 container-tree with editor-frames; workspaces; split/focus/resize/swap/floating; editor buffers as leaves; statusline reflects tree state.
- **Phase 2: Control plane.** Full `ruster.wm.*` Lua API, WM commands, workspace persistence, which-key/mini-buffer parity, frame theming.
- **Phase 3: Editor-in-desktop.** Multi-buffer editing, terminal leaf, LSP inside a tile, xdg-desktop-portal integration.
- **Phase 4: Polish.** Animations, layer-shell bars, screenshots, xwayland, session restore.

## Phase 0 — Definition of Done

`just compositor` boots a Wayland compositor on the local DRM device; `just compositor-nested` boots a winit dev-mode compositor. An `xdg-shell` client (e.g. `foot`, `weston-terminal`) launches from Lua config, maps, and composites behind a bare Ruster chrome: bottom statusline + one editor frame showing a buffer + a which-key overlay skeleton. Keyboard/pointer seat with working focus and a clean quit key. Lua config binds keys and launches clients.

### Phase 0 tasks

1. **Scaffold** — add `ruster-shell`, `ruster-render-gles`, `ruster-compositor` crates; smithay dependency and features; `just compositor` / `just compositor-nested` recipes. `ruster-shell` is scaffolded with a minimal surface/container shape only — the full i3 container-tree is Phase 1.
2. **Boot a minimal Smithay compositor** — `CompositorState` (compositor handle, backends, seat); udev/DRM backend via session auto-login + winit fallback; output add/resize/vsync/frame handling; SIGINT shutdown.
3. **GL compositing loop** — Smithay `GlesRenderer`; draw client surface textures into the output; frame clock + damage tracking.
4. **Seat & input** — keyboard + pointer handles, focus registration, key/motion events, quit key (`Mod4+Shift+q`).
5. **xdg-shell toplevels** — `XdgShellHandler`: map/configure/commit/unmap/close; title captured from `set_title`.
6. **Ruster chrome on GL (`ruster-render-gles`)** — glyph-atlas text (`cosmic-text`), rounded-rect/line primitives, ported theme tokens; draw statusline (mode, workspace, focused title), a real editor frame rendering a `ruster-core` buffer, and the which-key overlay.
7. **Minimal Lua control plane** — load `~/.config/ruster/compositor.lua`; bind keys; `client.launch("foot")`; `ruster.wm.*` stubs (`set_keybind`, `launch_client`, `focus`, `switch_workspace`).
8. **Input routing to clients** — surface-local pointer coords + keyboard focus forwarding, software cursor.
9. **Errors & robustness** — DRM permission guidance (seatd/logind/wheel), `RUST_LOG` tracing, clean teardown.
10. **Verification** — build/clippy; unit tests on theme tokens and the minimal `ruster-shell` surface/geometry helpers; manual matrix in nested mode (boot, map, title, focus, statusline, Lua binds, quit) + one DRM smoke test.

### Phase 0 risks

- Smithay 0.7 API learning curve (feature naming/architecture).
- GPU/DRM access on the target machine — nested mode is the safe dev path; document seatd/logind/wheel setup.
- GLES2 `glow` renderer vs client EGL textures — standard compositor path, low risk.
- Text rendering at 60fps — needs glyph atlas + dirty-cell redraw (ruster's existing pattern).
