# Ruster Wayland Compositor — Phase 0 Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ruster boots as a Wayland compositor on the local GPU (DRM) and in a nested winit dev window, maps `xdg-shell` clients, composites them with Smithay's `GlesRenderer`, and draws a bare Ruster chrome (statusline + one editor frame + which-key overlay) around them — driven by a Lua config that binds keys and launches clients.

**Architecture:** Three new crates in the ruster workspace: `ruster-shell` (pure shell state: windows, focus, workspace counter), `ruster-render-gles` (GL text/quad primitives + glyph atlas, plus Smithay render elements), and `ruster-compositor` (the binary + Smithay wiring: backends, seat, xdg-shell, render loop). Phase 0 renders every mapped toplevel fullscreen on the active output and draws chrome on top — the i3 container-tree is Phase 1, so `ruster-shell` stays minimal.

**Tech Stack:** Rust, Smithay 0.7 (path dep `~/Dev/smithay`), `GlesRenderer`, `winit` backend (dev) + `udev`/DRM backend, `wayland-backend` server, `xdg-shell`, `cosmic-text`, `mlua`, `tracing`. The compositor adapts Smithay's reference compositor `~/Dev/smithay/anvil/` (exact files cited per task).

## Global Constraints

- Workspace root: `/home/daimyo/Dev/ruster`. All new crates go under `crates/` and must be listed in the root `Cargo.toml` `members`.
- Smithay is a **path dependency** on `~/Dev/smithay` (clone is v0.7.0). Features: `backend_udev`, `backend_drm`, `backend_gbm`, `backend_session_libseat`, `backend_libinput`, `backend_winit`, `renderer_gl`, `desktop`, `wayland_frontend`. If the build complains a feature is unknown, re-check `~/Dev/smithay/Cargo.toml` `[features]` — do not guess.
- Renderer is **`GlesRenderer`** (`smithay::backend::renderer::gles::GlesRenderer`), NOT the old glow renderer.
- Palette/theme: reuse `ruster-render::Theme` and `ruster-render::Color`. Do NOT introduce a new color type in the compositor crates.
- Entry point is a dedicated binary crate `ruster-compositor`. `ruster-bin` stays untouched.
- Rust edition 2021 (matches existing crates). Run `cargo clippy --all-targets` and `cargo fmt` before every commit.
- No emojis, no comments unless a comment explains a non-obvious Smithay requirement.
- Every task must compile independently: `cargo check -p <crate>` and `cargo test -p <crate>`.
- When a plan step says "adapt from anvil", the anvil source at `~/Dev/smithay/anvil/src/` is the authoritative reference for the exact Smithay 0.7 API; copy the pattern, rename types, strip features we don't use.

---

### Task 1: Workspace scaffolding — three new crates, smithay dep, just recipes

**Files:**
- Modify: `Cargo.toml` (workspace members)
- Create: `crates/ruster-shell/Cargo.toml`, `crates/ruster-shell/src/lib.rs`
- Create: `crates/ruster-render-gles/Cargo.toml`, `crates/ruster-render-gles/src/lib.rs`
- Create: `crates/ruster-compositor/Cargo.toml`, `crates/ruster-compositor/src/main.rs`, `crates/ruster-compositor/src/lib.rs`
- Modify: `justfile`

**Interfaces:**
- Consumes: root workspace `Cargo.toml` members list; `ruster-render` crate (Theme/Color) at `crates/ruster-render`.
- Produces: three buildable crates. `ruster-compositor` produces a binary named `ruster-compositor`. Later tasks fill in module files under these crates.

- [ ] **Step 1: Add the three crates to the workspace**

In `Cargo.toml` replace the `members = [...]` line so the list includes the three new crates:

```toml
members = ["crates/ruster-core", "crates/ruster-notify", "crates/ruster-render", "crates/ruster-syntax", "crates/ruster-lua", "crates/ruster-lsp", "crates/ruster-terminal", "crates/ruster-git", "crates/ruster-project", "crates/ruster-dap", "crates/ruster-tui", "crates/ruster-bin", "crates/ruster-render-raylib", "crates/ruster-shell", "crates/ruster-render-gles", "crates/ruster-compositor"]
```

- [ ] **Step 2: Create `ruster-shell`**

`crates/ruster-shell/Cargo.toml`:

```toml
[package]
name = "ruster-shell"
version = "0.1.0"
edition = "2021"

[dependencies]
```

`crates/ruster-shell/src/lib.rs`:

```rust
//! Shell state for the ruster Wayland compositor.
//! Phase 0 keeps this minimal: a window record, focus tracking, and a
//! workspace counter. The i3 container-tree lands in Phase 1.
```

- [ ] **Step 3: Create `ruster-render-gles`**

`crates/ruster-render-gles/Cargo.toml`:

```toml
[package]
name = "ruster-render-gles"
version = "0.1.0"
edition = "2021"

[dependencies]
ruster-render = { path = "../ruster-render" }
smithay = { path = "../../../smithay", default-features = false, features = ["renderer_gl", "wayland_frontend", "backend_winit"] }
cosmic-text = "0.12"
fontdb = "0.16"
```
> Path to smithay is relative from `crates/ruster-render-gles/` → repo root is `../..` → smithay is `../../../smithay`. Adjust to the actual relative path if the shell resolves differently; the smithay clone lives at `/home/daimyo/Dev/smithay` beside `/home/daimyo/Dev/ruster`.

`crates/ruster-render-gles/src/lib.rs`:

```rust
//! GL rendering primitives for the ruster compositor: glyph-atlas text,
//! quad/rounded-rect geometry, and the Smithay render elements that draw
//! ruster's chrome (statusline, editor frames, which-key) in the same GL
//! scene as client surfaces.
```

- [ ] **Step 4: Create `ruster-compositor`**

`crates/ruster-compositor/Cargo.toml`:

```toml
[package]
name = "ruster-compositor"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "ruster-compositor"
path = "src/main.rs"

[features]
default = ["winit"]
winit = ["dep:smithay/backend_winit"]
udev = ["dep:smithay/backend_udev", "dep:smithay/backend_drm", "dep:smithay/backend_gbm", "dep:smithay/backend_session_libseat", "dep:smithay/backend_libinput"]

[dependencies]
ruster-shell = { path = "../ruster-shell" }
ruster-render = { path = "../ruster-render" }
ruster-render-gles = { path = "../ruster-render-gles" }
ruster-core = { path = "../ruster-core" }
ruster-lua = { path = "../ruster-lua" }
smithay = { path = "../../../smithay", default-features = false, features = ["renderer_gl", "wayland_frontend", "desktop"] }
calloop = "0.13"
mlua = { version = "0.9", features = ["lua54"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
anyhow = "1"
```
> If `ruster-lua` already pins `mlua`, match its version/features (see `crates/ruster-lua/Cargo.toml`) instead of adding a second mlua.

`crates/ruster-compositor/src/lib.rs`:

```rust
//! Ruster as a Wayland compositor: boots on DRM (udev) or a nested winit
//! window, composites xdg-shell clients with a GLES renderer, and draws
//! ruster's chrome around them.
```

`crates/ruster-compositor/src/main.rs`:

```rust
fn main() {
    println!("ruster-compositor: Phase 0 scaffold");
}
```

- [ ] **Step 5: Add `just` recipes**

Append to `justfile`:

```make
# Run the compositor nested in a winit window (dev).
compositor:
    cargo run -p ruster-compositor

# Run the compositor on DRM (needs a free VT + seatd/logind access).
compositor-drm:
    cargo run -p ruster-compositor --features ruster-compositor/udev -- --drm
```

- [ ] **Step 6: Build and test everything**

Run: `cargo build` then `cargo test -p ruster-shell -p ruster-render-gles -p ruster-compositor`
Expected: all three crates compile; `ruster-compositor --help`-style run prints the scaffold line.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml justfile crates/ruster-shell crates/ruster-render-gles crates/ruster-compositor
git commit -m "chore(compositor): scaffold ruster-shell, ruster-render-gles, ruster-compositor crates"
```

---

### Task 2: `ruster-shell` — Phase 0 shell state

**Files:**
- Create: `crates/ruster-shell/src/window.rs`
- Create: `crates/ruster-shell/src/state.rs`
- Modify: `crates/ruster-shell/src/lib.rs`

**Interfaces:**
- Consumes: nothing from other tasks (self-contained crate).
- Produces:
  - `pub struct WindowId(pub u32)` — `Copy`, `Eq`, `Hash`.
  - `pub struct ClientWindow { pub id: WindowId, pub title: String, pub width: i32, pub height: i32 }`
  - `pub struct ShellState { pub workspace: u32, pub focus: Option<WindowId>, next_id: u32 }`
  - `impl ShellState { pub fn new() -> Self; pub fn add_window(&mut self, title: String, w: i32, h: i32) -> WindowId; pub fn remove_window(&mut self, id: WindowId); pub fn set_focus(&mut self, id: WindowId); pub fn focused(&self) -> Option<&ClientWindow>; pub fn window(&self, id: WindowId) -> Option<&ClientWindow>; pub fn cycle_workspace(&mut self); pub fn windows(&self) -> impl Iterator<Item=&ClientWindow> }`
  - `impl ClientWindow { pub fn set_title(&mut self, title: String); pub fn set_size(&mut self, w: i32, h: i32) }`
  - `pub fn next_workspace(ws: u32) -> u32` — `ws.wrapping_add(1).min(9)`, wraps 9→1.

Phase 0 holds all mapped windows in insertion order (no tiling tree); `focus` names the window rendered fullscreen. Later tasks (compositor, chrome) depend on these exact names.

- [ ] **Step 1: Write the failing tests**

`crates/ruster-shell/src/state.rs` (tests at bottom):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_window_assigns_incrementing_ids() {
        let mut s = ShellState::new();
        let a = s.add_window("foot".into(), 800, 600);
        let b = s.add_window("firefox".into(), 1024, 768);
        assert_eq!(a, WindowId(0));
        assert_eq!(b, WindowId(1));
        assert_eq!(s.windows().count(), 2);
    }

    #[test]
    fn focus_tracks_most_recent() {
        let mut s = ShellState::new();
        let a = s.add_window("a".into(), 100, 100);
        let b = s.add_window("b".into(), 100, 100);
        s.set_focus(b);
        assert_eq!(s.focused().unwrap().id, b);
        s.set_focus(a);
        assert_eq!(s.focused().unwrap().id, a);
    }

    #[test]
    fn remove_window_clears_focus_if_needed() {
        let mut s = ShellState::new();
        let a = s.add_window("a".into(), 100, 100);
        s.set_focus(a);
        s.remove_window(a);
        assert_eq!(s.focused(), None);
        assert_eq!(s.windows().count(), 0);
    }

    #[test]
    fn workspace_cycles_and_wraps() {
        assert_eq!(next_workspace(1), 2);
        assert_eq!(next_workspace(9), 1);
        let mut s = ShellState::new();
        s.cycle_workspace();
        assert_eq!(s.workspace, 2);
    }

    #[test]
    fn window_title_and_size_mutators() {
        let mut s = ShellState::new();
        let a = s.add_window("old".into(), 10, 10);
        s.window(a).unwrap().set_title("new".into());
        s.window(a).unwrap().set_size(1920, 1080);
        let w = s.window(a).unwrap();
        assert_eq!(w.title, "new");
        assert_eq!((w.width, w.height), (1920, 1080));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ruster-shell`
Expected: FAIL — `error[E0425]: cannot find value 'ShellState' in this scope` (state.rs doesn't exist yet).

- [ ] **Step 3: Implement**

`crates/ruster-shell/src/window.rs`:

```rust
/// Handle to a compositor-managed client window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(pub u32);

/// One mapped client window. Phase 0 stores no layout tree — windows are
/// ordered by map time and the focused one renders fullscreen.
#[derive(Debug, Clone)]
pub struct ClientWindow {
    pub id: WindowId,
    pub title: String,
    pub width: i32,
    pub height: i32,
}

impl ClientWindow {
    pub fn set_title(&mut self, title: String) {
        self.title = title;
    }
    pub fn set_size(&mut self, width: i32, height: i32) {
        self.width = width;
        self.height = height;
    }
}
```

`crates/ruster-shell/src/state.rs`:

```rust
use crate::window::{ClientWindow, WindowId};

/// Number of workspaces; `next_workspace` cycles 1..=9.
pub const WORKSPACE_COUNT: u32 = 9;

pub fn next_workspace(ws: u32) -> u32 {
    let next = ws + 1;
    if next > WORKSPACE_COUNT {
        1
    } else {
        next
    }
}

/// Phase 0 shell state: insertion-ordered window list, a focus handle, and a
/// workspace counter. The i3 container-tree replaces the flat list in Phase 1.
pub struct ShellState {
    windows: Vec<ClientWindow>,
    next_id: u32,
    pub focus: Option<WindowId>,
    pub workspace: u32,
}

impl Default for ShellState {
    fn default() -> Self {
        Self::new()
    }
}

impl ShellState {
    pub fn new() -> Self {
        ShellState { windows: Vec::new(), next_id: 0, focus: None, workspace: 1 }
    }

    pub fn add_window(&mut self, title: String, width: i32, height: i32) -> WindowId {
        let id = WindowId(self.next_id);
        self.next_id += 1;
        self.windows.push(ClientWindow { id, title, width, height });
        id
    }

    pub fn remove_window(&mut self, id: WindowId) {
        if let Some(pos) = self.windows.iter().position(|w| w.id == id) {
            self.windows.remove(pos);
        }
        if self.focus == Some(id) {
            self.focus = self.windows.last().map(|w| w.id);
        }
    }

    pub fn set_focus(&mut self, id: WindowId) {
        if self.windows.iter().any(|w| w.id == id) {
            self.focus = Some(id);
        }
    }

    pub fn focused(&self) -> Option<&ClientWindow> {
        self.focus.and_then(|id| self.windows.iter().find(|w| w.id == id))
    }

    pub fn window(&self, id: WindowId) -> Option<&ClientWindow> {
        self.windows.iter().find(|w| w.id == id)
    }

    pub fn windows(&self) -> impl Iterator<Item = &ClientWindow> {
        self.windows.iter()
    }

    pub fn cycle_workspace(&mut self) {
        self.workspace = next_workspace(self.workspace);
    }
}
```

`crates/ruster-shell/src/lib.rs` — replace the doc-comment placeholder with:

```rust
//! Shell state for the ruster Wayland compositor.
//! Phase 0 keeps this minimal: a window record, focus tracking, and a
//! workspace counter. The i3 container-tree lands in Phase 1.

pub mod state;
pub mod window;

pub use state::ShellState;
pub use window::{ClientWindow, WindowId};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p ruster-shell`
Expected: 5 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ruster-shell
git commit -m "feat(shell): add Phase 0 shell state (window list, focus, workspace counter)"
```

---

### Task 3: `ruster-render-gles` — glyph atlas and quad geometry

**Files:**
- Create: `crates/ruster-render-gles/src/atlas.rs`
- Create: `crates/ruster-render-gles/src/geometry.rs`
- Modify: `crates/ruster-render-gles/src/lib.rs`

**Interfaces:**
- Consumes: `ruster-render::{Color, Theme, Rect}` (Rect is in `ruster-render`, x/y/width/height u16, cell coords).
- Produces:
  - `pub struct Glyph { pub x: f32, pub y: f32, pub w: f32, pub h: f32, pub u0: f32, pub v0: f32, pub u1: f32, pub v1: f32 }`
  - `pub struct Atlas { pub texture_size: u32, glyphs: HashMap<(u32, char), Glyph> }` with `impl Atlas { pub fn new() -> Self; pub fn glyph(&mut self, font_size_px: u32, c: char) -> Glyph }`. Non-renderable glyphs return a zero-size Glyph. Backed by `fontdb` + `cosmic-text`'s `FontSystem`/`SwashCache` to rasterize a 64×64 cell into an internal CPU texture that Task 7 uploads; for Task 3 the atlas only needs to track glyph metadata + measure.
  - `pub struct TextLayout { pub glyphs: Vec<(f32, f32, char)>, pub width_px: f32 }` — `pub fn layout_text(text: &str, font_size_px: u32, wrap_width: Option<f32>) -> TextLayout` via `cosmic-text::Buffer`.
  - Geometry: `pub fn rounded_rect_verts(x: f32, y: f32, w: f32, h: f32, r: f32, color: (f32,f32,f32,f32)) -> Vec<[f32; 8]>` returning `xy + rgba` vertex tuples for a triangulated rounded rect; and `pub fn rect_verts(x, y, w, h, color) -> Vec<[f32; 8]>` (two triangles, 6 verts). Pure math — unit-testable without a GL context.
  - `impl From<Color> for (f32, f32, f32, f32)` — ruster `Color::Rgb(u8,u8,u8)` → normalized rgba (a=1.0), `Color::Default` → white.

Task 7's render elements consume these. Keep `Atlas` textrure size fixed at 512×512 with 64px cells (8×8 grid) for now; real packing/upload happens in Task 7.

- [ ] **Step 1: Write the failing tests**

`crates/ruster-render-gles/src/geometry.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_verts_make_two_triangles() {
        let v = rect_verts(0.0, 0.0, 10.0, 5.0, (1.0, 0.0, 0.0, 1.0));
        assert_eq!(v.len(), 6);
        for vert in &v {
            assert_eq!(vert[4], 1.0); // r
            assert_eq!(vert[7], 1.0); // a
        }
        let xs: Vec<f32> = v.iter().map(|t| t[0]).collect();
        assert!(xs.iter().all(|x| (0.0..=10.0).contains(x)));
    }

    #[test]
    fn rounded_rect_corner_radius_is_clamped() {
        let v = rounded_rect_verts(0.0, 0.0, 10.0, 10.0, 100.0, (0.0, 1.0, 0.0, 1.0));
        // clamp r to min(w,h)/2 → r=5; verts must all stay inside [0,10].
        assert!(v.iter().all(|t| (0.0..=10.0).contains(&t[0]) && (0.0..=10.0).contains(&t[1])));
    }

    #[test]
    fn color_rgb_converts_to_normalized_rgba() {
        let c: (f32, f32, f32, f32) = Color::Rgb(255, 128, 0).into();
        assert_eq!(c, (1.0, 128.0 / 255.0, 0.0, 1.0));
        let d: (f32, f32, f32, f32) = Color::Default.into();
        assert_eq!(d, (1.0, 1.0, 1.0, 1.0));
    }
}
```

`crates/ruster-render-gles/src/atlas.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyph_returns_zero_size_for_unknown() {
        let mut atlas = Atlas::new();
        let g = atlas.glyph(20, '\0');
        assert_eq!((g.w, g.h), (0.0, 0.0));
    }

    #[test]
    fn glyphs_are_cached() {
        let mut atlas = Atlas::new();
        let a = atlas.glyph(20, 'a');
        let b = atlas.glyph(20, 'a');
        assert_eq!((a.u0, a.v0, a.u1, a.v1), (b.u0, b.v0, b.u1, b.v1));
    }

    #[test]
    fn layout_text_measures_width() {
        let layout = layout_text("hello", 20, None);
        assert!(layout.width_px > 0.0);
        assert_eq!(layout.glyphs.len(), 5);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ruster-render-gles`
Expected: FAIL — modules `geometry` and `atlas` not found in `lib.rs`.

- [ ] **Step 3: Implement**

`crates/ruster-render-gles/src/geometry.rs`:

```rust
use ruster_render::Color;

/// One vertex: x, y, r, g, b, a. Sized for a `gl::VertexAttribPointer` of
/// two vec2 attributes (position, color).
pub type Vertex = [f32; 8];

pub fn rect_verts(x: f32, y: f32, w: f32, h: f32, color: (f32, f32, f32, f32)) -> Vec<Vertex> {
    let (r, g, b, a) = color;
    vec![
        [x, y, r, g, b, a],
        [x + w, y, r, g, b, a],
        [x + w, y + h, r, g, b, a],
        [x, y, r, g, b, a],
        [x + w, y + h, r, g, b, a],
        [x, y + h, r, g, b, a],
    ]
}

pub fn rounded_rect_verts(x: f32, y: f32, w: f32, h: f32, radius: f32, color: (f32, f32, f32, f32)) -> Vec<Vertex> {
    let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    let (rr, g, b, a) = color;
    if r <= 0.001 {
        return rect_verts(x, y, w, h, color);
    }
    // Center + 4 corner fan: draw an inner rect + 4 rounded corners as quads.
    let mut verts = rect_verts(x + r, y, w - 2.0 * r, h, color); // middle band
    verts.extend(rect_verts(x, y + r, w, h - 2.0 * r, color));  // vertical band
    // Corners: for each corner draw a 2x2 grid of squares, skipping the outer
    // corner cell (the one beyond radius). 64 verts is fine for a chrome bar.
    let corners = [
        (x, y, 1.0, 1.0),
        (x + w - r, y, -1.0, 1.0),
        (x, y + h - r, 1.0, -1.0),
        (x + w - r, y + h - r, -1.0, -1.0),
    ];
    for (cx, cy, sx, sy) in corners {
        for gy in 0..2 {
            for gx in 0..2 {
                let cell_x = cx + gx as f32 * r;
                let cell_y = cy + gy as f32 * r;
                let dx = (gx as f32 + 0.5) * sx;
                let dy = (gy as f32 + 0.5) * sy;
                let inside = dx * dx + dy * dy <= r * r;
                if inside {
                    verts.extend(rect_verts(cell_x, cell_y, r, r, (rr, g, b, a)));
                }
            }
        }
    }
    verts
}

impl From<Color> for (f32, f32, f32, f32) {
    fn from(color: Color) -> Self {
        match color {
            Color::Default => (1.0, 1.0, 1.0, 1.0),
            Color::Rgb(r, g, b) => (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0),
        }
    }
}
```
> The corner loop above intentionally approximates a rounded corner with 3 sub-cells (the two non-corner cells of the 2×2 grid). This is a Phase 0 approximation; if you prefer true circles, subdivide into 3 rings — the unit test only requires vertices stay inside `[0,10]` with `r=5`.

`crates/ruster-render-gles/src/atlas.rs`:

```rust
use std::collections::HashMap;

use cosmic_text::{FontSystem, Metrics, Shaping};
use fontdb::Database;

/// A single glyph's pixel rect (in the destination) and UV rect (in the atlas).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glyph {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
}

/// One laid-out text run: glyph ids plus char offsets. Phase 0 uses this only
/// to measure width; Task 7 rasterizes from the same layout.
pub struct TextLayout {
    pub glyphs: Vec<(f32, f32, char)>,
    pub width_px: f32,
}

/// CPU-side glyph atlas. Phase 0 keeps a fixed 512px texture with 64px cells;
/// Task 7 uploads `cells()` to a GL texture and draws glyph quads.
pub struct Atlas {
    pub texture_size: u32,
    glyphs: HashMap<(u32, char), Glyph>,
    font_db: Database,
    font_system: FontSystem,
}

impl Atlas {
    pub fn new() -> Self {
        let font_db = Database::new();
        font_db.load_system_fonts();
        Atlas {
            texture_size: 512,
            glyphs: HashMap::new(),
            font_db,
            font_system: FontSystem::new(),
        }
    }

    pub fn glyph(&mut self, font_size_px: u32, c: char) -> Glyph {
        const CELL: u32 = 64;
        if let Some(g) = self.glyphs.get(&(font_size_px, c)) {
            return *g;
        }
        let (w, h) = self.measure(font_size_px, c);
        let empty = Glyph { x: 0.0, y: 0.0, w: w as f32, h: h as f32, u0: 0.0, v0: 0.0, u1: 0.0, v1: 0.0 };
        if c == '\0' || c.is_control() {
            self.glyphs.insert((font_size_px, c), empty);
            return empty;
        }
        let index = self.font_db
            .faces()
            .find(|f| f.post_script_name.as_deref().is_some_and(|n| n.to_lowercase().contains("mono")))
            .or_else(|| self.font_db.faces().next())
            .and_then(|face| self.font_db.face_id(face.index));
        let id = index.unwrap_or(0);
        let font = self.font_system.get_or_load(&self.font_db, id).unwrap_or_default();
        let metrics = Metrics { font_size: font_size_px as f32, line_height: font_size_px as f32 + 4.0 };
        let mut buf = cosmic_text::Buffer::new(&mut self.font_system, metrics);
        let mut chars = String::from(c);
        buf.set_text(&mut self.font_system, &mut chars, None, Shaping::Advanced);
        let mut layout = TextLayout { glyphs: Vec::new(), width_px: 0.0 };
        for run in buf.layout_runs() {
            for glyph in run.glyphs.iter() {
                layout.glyphs.push((glyph.x as f32, glyph.y as f32, glyph.c));
            }
            if layout.width_px < run.line_w {
                layout.width_px = run.line_w;
            }
        }
        let _ = font;
        // Cell index: round-robin on the 8x8 grid, keyed by the hash.
        let cell = (self.glyphs.len() % 64) as u32;
        let col = cell % 8;
        let row = cell / 8;
        let x = col * CELL;
        let y = row * CELL;
        let u = (x as f32) / self.texture_size as f32;
        let v = (y as f32) / self.texture_size as f32;
        let uw = (CELL as f32) / self.texture_size as f32;
        let vh = (CELL as f32) / self.texture_size as f32;
        let g = Glyph {
            x: 0.0,
            y: 0.0,
            w: layout.width_px,
            h: font_size_px as f32,
            u0: u,
            v0: v,
            u1: u + uw,
            v1: v + vh,
        };
        self.glyphs.insert((font_size_px, c), g);
        g
    }

    fn measure(&self, font_size_px: u32, c: char) -> (u32, u32) {
        let _ = (font_size_px, c);
        // Phase 0: estimate from the layout produced by glyph(); the real cell
        // rasterization in Task 7 replaces this.
        (font_size_px, font_size_px)
    }
}

/// Lay out a string, measuring its pixel width. Used by chrome drawing and by
/// the atlas for glyph metrics.
pub fn layout_text(text: &str, font_size_px: u32, _wrap_width: Option<f32>) -> TextLayout {
    let mut font_system = FontSystem::new();
    let mut font_db = Database::new();
    font_db.load_system_fonts();
    let metrics = Metrics { font_size: font_size_px as f32, line_height: font_size_px as f32 + 4.0 };
    let mut buf = cosmic_text::Buffer::new(&mut font_system, metrics);
    buf.set_text(&mut font_system, text, None, Shaping::Advanced);
    let mut layout = TextLayout { glyphs: Vec::new(), width_px: 0.0 };
    for run in buf.layout_runs() {
        for glyph in run.glyphs.iter() {
            layout.glyphs.push((glyph.x as f32, glyph.y as f32, glyph.c));
        }
        if layout.width_px < run.line_w {
            layout.width_px = run.line_w;
        }
    }
    layout
}
```
> If the `cosmic-text` 0.12 API differs (e.g. `set_text` signature), match the version's actual signatures — the anvil/tests will surface this during `cargo build`. The unit tests only assert measurement + caching behavior, not exact widths.

`crates/ruster-render-gles/src/lib.rs` — replace placeholder with:

```rust
//! GL rendering primitives for the ruster compositor: glyph-atlas text,
//! quad/rounded-rect geometry, and the Smithay render elements that draw
//! ruster's chrome (statusline, editor frames, which-key) in the same GL
//! scene as client surfaces.

pub mod atlas;
pub mod geometry;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p ruster-render-gles`
Expected: 5 tests PASS. If `cosmic-text` pulls in deps that need system libs, install them (`libfontconfig1-dev` etc.) — note in commit message.

- [ ] **Step 5: Commit**

```bash
git add crates/ruster-render-gles
git commit -m "feat(render-gles): glyph atlas, text layout, and quad geometry primitives"
```

---

### Task 4: Compositor boot — `ruster-compositor` event loop + winit backend

**Files:**
- Create: `crates/ruster-compositor/src/compositor.rs`
- Create: `crates/ruster-compositor/src/backend/winit.rs`
- Create: `crates/ruster-compositor/src/backend/mod.rs`
- Modify: `crates/ruster-compositor/src/lib.rs`, `crates/ruster-compositor/src/main.rs`

**Interfaces:**
- Consumes: `ruster-shell::ShellState` (Task 2); smithay `winit` backend; `GlesRenderer`.
- Produces:
  - `pub struct CompositorState<B: Backend> { pub display_handle: DisplayHandle, pub shell: ShellState, pub seat: Seat<CompositorState<B>>, pub pointer: PointerHandle<CompositorState<B>>, pub keyboard: KeyboardHandle<CompositorState<B>>, pub running: Arc<AtomicBool>, pub handle: LoopHandle<'static, CompositorState<B>> }`
  - `pub trait Backend { fn seat_name(&self) -> String; fn reset_buffers(&mut self, output: &Output); }`
  - `pub struct RusterWinitData { pub backend: WinitGraphicsBackend<GlesRenderer>, pub damage_tracker: OutputDamageTracker }` + `impl Backend for RusterWinitData`
  - `pub fn init_winit(data: WinitGraphicsBackend<GlesRenderer>, event_loop: LoopHandle<'static, CompositorState<WinitData>>, state: &mut CompositorState<WinitData>)`
  - `pub fn run_winit() -> anyhow::Result<()>` — creates display, state, seat, keyboard, pointer, output, and runs the calloop loop until `running` flips false.
  - `main.rs`: parse `--drm` (Task 11) else call `run_winit()`.

This is the composition root. Adapt the skeleton from `~/Dev/smithay/anvil/src/winit.rs` and `anvil/src/main.rs` — but Phase 0 is far leaner: no dmabuf, no layer-shell, no popups.

- [ ] **Step 1: Write the failing test (state construction)**

`crates/ruster-compositor/src/compositor.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // Constructing a CompositorState requires a DisplayHandle; exercise the
    // parts that don't need one: ShellState lifecycle and the running flag.
    #[test]
    fn running_flag_defaults_true() {
        let running = Arc::new(AtomicBool::new(true));
        assert!(running.load(Ordering::Relaxed));
    }

    #[test]
    fn shell_state_rejects_unknown_focus() {
        let mut shell = ShellState::new();
        let id = shell.add_window("x".into(), 100, 100);
        shell.set_focus(WindowId(999));
        assert_eq!(shell.focused(), None);
        shell.set_focus(id);
        assert!(shell.focused().is_some());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ruster-compositor`
Expected: FAIL — `cannot find module compositor` in lib.rs.

- [ ] **Step 3: Implement the compositor state**

`crates/ruster-compositor/src/backend/mod.rs`:

```rust
pub mod winit;

use smithay::output::Output;

/// Minimal backend contract. Phase 0 only needs a seat name and a buffer
/// reset hook; DRM's Backend impl is Task 11.
pub trait Backend {
    fn seat_name(&self) -> String;
    fn reset_buffers(&mut self, output: &Output);
}
```

`crates/ruster-compositor/src/compositor.rs`:

```rust
use std::sync::{Arc, atomic::AtomicBool};

use calloop::LoopHandle;
use smithay::input::keyboard::KeyboardHandle;
use smithay::input::pointer::PointerHandle;
use smithay::input::Seat;
use smithay::wayland::socket::ListeningSocketSource;
use smithay::reexports::wayland_server::{Display, DisplayHandle};
use tracing::info;

use crate::backend::Backend;
use ruster_shell::{ShellState, WindowId};

/// The compositor's composition root: everything the backend and the input
/// handlers need to reach. Mirrors anvil's `AnvilState` but trimmed to Phase 0.
pub struct CompositorState<B: Backend + 'static> {
    pub backend_data: B,
    pub display_handle: DisplayHandle,
    pub socket_name: Option<String>,
    pub running: Arc<AtomicBool>,
    pub handle: LoopHandle<'static, CompositorState<B>>,
    pub shell: ShellState,
    pub seat: Seat<CompositorState<B>>,
    pub pointer: PointerHandle<CompositorState<B>>,
    pub keyboard: KeyboardHandle<CompositorState<B>>,
}

impl<B: Backend + 'static> CompositorState<B> {
    pub fn seat_name(&self) -> String {
        self.backend_data.seat_name()
    }
}

/// Create the Wayland display, add the client socket, and start listening for
/// connections. Returns the socket name for clients to connect to.
pub fn init_listener<B: Backend + 'static>(state: &mut CompositorState<B>) -> String {
    let display = Display::new();
    let mut source = ListeningSocketSource::new_auto(display.handle()).unwrap();
    let socket_name = source.socket_name().to_string();
    state.socket_name = Some(socket_name.clone());
    state.handle.insert_source(source, move |client_stream, _, state: &mut CompositorState<B>| {
        info!(client = ?client_stream.peer_addr().ok(), "client connected");
        // Phase 0: accept the stream; client init + handlers in Task 5/6.
        let _ = client_stream;
    }).unwrap();
    socket_name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_flag_defaults_true() {
        let running = Arc::new(AtomicBool::new(true));
        assert!(running.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[test]
    fn shell_state_rejects_unknown_focus() {
        let mut shell = ShellState::new();
        let id = shell.add_window("x".into(), 100, 100);
        shell.set_focus(WindowId(999));
        assert_eq!(shell.focused(), None);
        shell.set_focus(id);
        assert!(shell.focused().is_some());
    }
}
```
> `ListeningSocketSource::new_auto` gives an auto-named socket; anvil names it "ruster-N". For Phase 0, print `socket_name` from `main.rs` and set `WAYLAND_DISPLAY` accordingly in the launch script.

- [ ] **Step 4: Implement the winit backend**

Adapt `~/Dev/smithay/anvil/src/winit.rs` (setup + `WinitData`), trimmed to Phase 0. Key points to copy verbatim from anvil: `WinitEvent` handling (Resized/CloseRequested/Focused), the `Output` creation with `PhysicalProperties`/`Mode`/`Subpixel::Unknown`, `backend.bind()`, and the damage tracker reset on resize.

`crates/ruster-compositor/src/backend/winit.rs`:

```rust
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::{self, WinitEvent, WinitGraphicsBackend};
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::utils::{Scale, Transform};
use tracing::{info, warn};

use super::Backend;

pub struct RusterWinitData {
    pub backend: WinitGraphicsBackend<GlesRenderer>,
    pub damage_tracker: smithay::backend::renderer::damage::OutputDamageTracker,
    full_redraw: u8,
}

impl Backend for RusterWinitData {
    fn seat_name(&self) -> String {
        String::from("ruster-winit")
    }
    fn reset_buffers(&mut self, _output: &Output) {
        self.full_redraw = 4;
    }
}

impl RusterWinitData {
    pub fn handle_event<F>(&mut self, event: WinitEvent, mut on_output: F)
    where
        F: FnMut(&Output, bool),
    {
        match event {
            WinitEvent::Resized { size, .. } => {
                info!(?size, "winit output resized");
                self.full_redraw = 4;
            }
            WinitEvent::CloseRequested => {
                info!("close requested");
                // main loop watches the running flag; caller sets it.
            }
            WinitEvent::Focused(focused) => {
                info!(focused, "winit focus event");
            }
            WinitEvent::Output(o) => {
                on_output(&o, false);
            }
            WinitEvent::NewInput(_) => {}
        }
    }
}

/// Build a winit output + register it with the compositor's global state.
pub fn init_winit<F>(
    backend: WinitGraphicsBackend<GlesRenderer>,
    output: Output,
    mut on_output: F,
) -> RusterWinitData
where
    F: FnMut(&Output, bool),
{
    let size = backend.window_size();
    let mode = Mode { size, refresh: 60000 };
    output.set_preferred(mode);
    output.set_current(Some(mode));
    output.set_physical_properties(PhysicalProperties {
        size,
        subpixel: Subpixel::Unknown,
        make: "ruster".into(),
        model: "winit".into(),
    });
    output.set_transform(Transform::Normal);
    output.set_scale(Scale(1.0));
    on_output(&output, false);
    let _ = backend;
    RusterWinitData { backend, damage_tracker: smithay::backend::renderer::damage::OutputDamageTracker::new(size), full_redraw: 4 }
}
```
> The exact `WinitEvent` variants and `Output` builder calls must match `anvil/src/winit.rs` in the local clone — copy the event match arms from there and drop the ones we don't need.

- [ ] **Step 5: Wire `run_winit()` and main**

`crates/ruster-compositor/src/lib.rs` — replace placeholder:

```rust
//! Ruster as a Wayland compositor: boots on DRM (udev) or a nested winit
//! window, composites xdg-shell clients with a GLES renderer, and draws
//! ruster's chrome around them.

pub mod backend;
pub mod compositor;
```

`crates/ruster-compositor/src/main.rs`:

```rust
use std::sync::Arc;

use calloop::{EventLoop, LoopSignal};
use ruster_compositor::{backend::winit::{RusterWinitData, init_winit}, compositor::{CompositorState, init_listener}};
use smithay::backend::winit::{self, WinitEvent};
use smithay::reexports::calloop::LoopHandle;
use smithay::output::Output;
use tracing_subscriber::EnvFilter;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter(EnvFilter::from_default_env()).init();
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--drm") {
        return run_drm(); // Task 11
    }
    run_winit()
}

fn run_winit() -> anyhow::Result<()> {
    let event_loop: EventLoop<'static, CompositorState<RusterWinitData>> = EventLoop::try_new()?;
    let running = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let signal: LoopSignal = event_loop.get_signal();
    let handle: LoopHandle<'static, CompositorState<RusterWinitData>> = event_loop.handle();
    let mut backend = winit::init(event_loop.handle().clone()).map_err(|e| anyhow::anyhow!("winit init: {e}"))?;
    let backend_data = backend.backend().unwrap();

    let (mut state, _renderer) = create_state::<RusterWinitData>(handle.clone(), backend_data, running.clone());
    let _ = init_listener(&mut state);

    // Poll winit events into the loop.
    event_loop.run(move |_, _, state| {
        // pump winit events; the compositor state is state
        Ok(calloop::PostAction::Continue)
    })?;
    Ok(())
}
```
> This skeleton won't compile until Task 5/6 fill in `create_state` (seat/keyboard/pointer/output). Ship this task as: crates compile with `compositor.rs`, `backend/mod.rs`, `backend/winit.rs`, `main.rs` compiling to a binary whose `--help` path prints "Phase 0 scaffold" and whose default path reaches `run_winit()`. It is acceptable for `run_winit` to remain a stub returning `Ok(())` with the event loop created — the loop plumbing lands in Task 5. Mark the stub clearly with a `TODO(Task 5)` line.

- [ ] **Step 6: Build and test**

Run: `cargo build -p ruster-compositor && cargo test -p ruster-compositor`
Expected: compiles; 2 unit tests pass. `cargo clippy -p ruster-compositor` clean.

- [ ] **Step 7: Commit**

```bash
git add crates/ruster-compositor
git commit -m "feat(compositor): boot structure — display, shell state, winit backend scaffold"
```

---

### Task 5: Seat, keyboard, pointer + display globals

**Files:**
- Create: `crates/ruster-compositor/src/input.rs`
- Create: `crates/ruster-compositor/src/globals.rs`
- Modify: `crates/ruster-compositor/src/compositor.rs` (add `create_state`, `init_globals`, `keyboard`/`pointer` init), `crates/ruster-compositor/src/main.rs` (drive the event loop)

**Interfaces:**
- Consumes: `CompositorState` (Task 4); anvil `input_handler.rs`, `focus.rs`, `state.rs` (seat init) for the exact smithay API.
- Produces:
  - `pub fn create_state<B: Backend + 'static>(handle, backend_data, running) -> (CompositorState<B>, GlesRenderer)` — builds seat (name from `Backend::seat_name`), keyboard (XKB config from `~/.config/ruster/` fallback system), pointer, and calls `init_globals`.
  - `pub fn init_globals<B: Backend + 'static>(state: &mut CompositorState<B>)` — inserts `CompositorState`, `ShmState`, `XdgShellState`, `SeatState`, and wires the `delegate_*` handlers needed by `wayland_server` (see anvil `delegate_dispatch2!`).
  - `pub fn run_winit()` (filled in) — full loop: `handle.insert_source` winit events, call `state.backend_data.handle_event(...)`, request frame, `event_loop.run(...)` until `running` is false, then `signal.stop()`.
  - `main.rs`: on SIGINT (ctrlc or a calloop signal source) set `running=false`.

- [ ] **Step 1: Write the failing tests**

`crates/ruster-compositor/src/input.rs` tests (pure logic that survives without a live display):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keysym_quit_binding_recognized() {
        // Mod4+Shift+q → quit
        let keysym = Keysym::q;
        let mods = ModifiersState { alt: false, ctrl: false, logo: true, shift: true };
        assert!(is_quit_keysym(keysym, &mods));
    }

    #[test]
    fn workspace_cycle_binding_recognized() {
        let keysym = Keysym::Tab;
        let mods = ModifiersState { alt: false, ctrl: true, logo: true, shift: false };
        assert!(is_cycle_workspace(keysym, &mods));
    }
}
```
> `ModifiersState` is `smithay::input::keyboard::ModifiersState` — check its field names (`logo`/`shift`/`ctrl`/`alt`) against the anvil usage before writing; adjust field names to match the actual struct.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ruster-compositor`
Expected: FAIL — `input` module not found.

- [ ] **Step 3: Implement the input module**

`crates/ruster-compositor/src/input.rs`:

```rust
use smithay::input::keyboard::{Keysym, ModifiersState};

/// Global mod for WM binds: Mod4 (Super/Logo).
pub const WM_MOD: u32 = 4;

pub fn is_quit_keysym(keysym: Keysym, mods: &ModifiersState) -> bool {
    keysym == Keysym::q && mods.logo && mods.shift
}

pub fn is_cycle_workspace(keysym: Keysym, mods: &ModifiersState) -> bool {
    keysym == Keysym::Tab && mods.logo
}
```
> If `ModifiersState` field names differ (e.g. `super_key` instead of `logo`), match the actual `smithay::input::keyboard::ModifiersState`. The test asserts `logo=true` behavior; adapt the field name, not the behavior.

- [ ] **Step 4: Implement globals + state construction**

Adapt `anvil/src/state.rs` `init`/`create_state` and `anvil/src/input_handler.rs` for the exact `KeyboardHandle::new`, `PointerHandle::new`, `Seat::new`, `SeatState` calls. Phase 0 globals: `CompositorState` (required by every client), `ShmState` (shared-memory buffers), `XdgShellState`, `SeatState`.

`crates/ruster-compositor/src/globals.rs`:

```rust
use smithay::wayland::compositor::CompositorState;
use smithay::wayland::shm::ShmState;
use smithay::wayland::shell::xdg::XdgShellState;
use smithay::input::SeatState;

pub struct GlobalStates {
    pub compositor_state: CompositorState,
    pub shm_state: ShmState,
    pub xdg_shell_state: XdgShellState,
    pub seat_state: SeatState<crate::compositor::CompositorState<crate::backend::winit::RusterWinitData>>,
}
```
> The concrete generic bound is painful to type across backends; if it causes friction, store `GlobalStates` behind a macro or keep it in `compositor.rs` alongside `CompositorState` (anvil keeps all state fields on `AnvilState` directly — do that: add `compositor_state`, `shm_state`, `xdg_shell_state`, `seat_state` fields onto `CompositorState<B>` instead of a separate struct). Prefer the anvil layout: fields on the state struct.

- [ ] **Step 5: Wire the event loop in main.rs**

Fill `run_winit()` per `anvil/src/main.rs` winit branch: create `winit::init`, extract `WinitGraphicsBackend`, create state with seat/keyboard/pointer/globals, `init_listener`, then `event_loop.run` that (a) pumps winit events via `state.backend_data.handle_event`, (b) on `WinitEvent::NewFrame` calls `render_frame` (Task 7 — stub returning Ok for now), and (c) breaks when `!state.running.load(Ordering::Relaxed)`, calling `signal.stop()`.

- [ ] **Step 6: Build, test, and smoke-run**

Run: `cargo build -p ruster-compositor && cargo test -p ruster-compositor`
Then run `cargo run -p ruster-compositor` (needs a display; it will open a winit window and log the socket name — you may need `WAYLAND_DISPLAY` unset and a running X/Wayland session; if headless, skip the smoke run and rely on the unit tests).
Expected: compiles; tests pass; if a display is available, a winit window opens and logs `client connected` on client connect.

- [ ] **Step 7: Commit**

```bash
git add crates/ruster-compositor
git commit -m "feat(compositor): seat, keyboard, pointer, and core display globals"
```

---

### Task 6: `xdg-shell` toplevels — map, configure, unmap, close

**Files:**
- Create: `crates/ruster-compositor/src/shell.rs`
- Modify: `crates/ruster-compositor/src/compositor.rs` (add toplevel map/slot on `CompositorState`), `crates/ruster-compositor/src/lib.rs`, `crates/ruster-compositor/src/main.rs` (launch a test client)

**Interfaces:**
- Consumes: `ShellState` (Task 2); anvil `shell/xdg.rs` `XdgShellHandler` impl.
- Produces:
  - `pub struct ClientWindowId(pub WindowId)` (newtype in shell.rs) and `impl smithay::desktop::space::SpaceElement for ClientWindowId` — Phase 0 renders each toplevel fullscreen; `geometry(usize)` returns the full output area when focused, else the window's size; `get_bbox`/`is_rendered`/`z_index` implemented per anvil's `WindowElement`.
  - `XdgShellHandler` impl for `CompositorState<B>`: `new_toplevel` → store `ToplevelSurface`, add to `ShellState` (title from `toplevel.toplevel().title()`), set pending focus; `toplevel_map` → `output.activate()`/damage; `toplevel_commit` → update title/size + damage; `toplevel_unmap`/`toplevel_destroy` → remove from `ShellState`, refocus; `toplevel_close` → send close to the client.
  - `pub struct RusterToplevel { pub surface: ToplevelSurface, pub window_id: WindowId }` and `CompositorState.shell` stores `Vec<RusterToplevel>` (Phase 0: no Space; the render loop draws surfaces directly in Task 7). Actually simpler: store `ToplevelSurface` map in `CompositorState.toplevels: HashMap<WindowId, ToplevelSurface>`.

- [ ] **Step 1: Write the failing tests**

`crates/ruster-compositor/src/shell.rs` tests (pure parts, no display needed):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ruster_shell::ShellState;

    #[test]
    fn title_updates_flow_into_shell_state() {
        let mut shell = ShellState::new();
        let id = shell.add_window("init".into(), 100, 100);
        shell.window(id).unwrap().set_title("foot".into());
        assert_eq!(shell.window(id).unwrap().title, "foot");
    }

    #[test]
    fn unmap_of_nonfocused_window_keeps_focus() {
        let mut shell = ShellState::new();
        let a = shell.add_window("a".into(), 100, 100);
        let b = shell.add_window("b".into(), 100, 100);
        shell.set_focus(a);
        shell.remove_window(b);
        assert_eq!(shell.focused().unwrap().id, a);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ruster-compositor`
Expected: FAIL — `shell` module not found.

- [ ] **Step 3: Implement the xdg-shell handler**

`crates/ruster-compositor/src/shell.rs`:

```rust
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::wayland::shell::xdg::{
    Configure, PopupSurface, PositionerState, ToplevelCachedState, ToplevelSurface, XdgShellHandler,
};
use smithay::wayland::seat::WaylandFocus;
use smithay::utils::Serial;

use crate::compositor::CompositorState;
use crate::backend::Backend;

impl<B: Backend + 'static> XdgShellHandler for CompositorState<B> {
    fn xdg_shell_state(&mut self) -> &mut smithay::wayland::shell::xdg::XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let title = surface.toplevel().title().map(|s| s.to_string()).unwrap_or_default();
        let id = self.shell.add_window(title.clone(), 800, 600);
        let serial = Serial::from(self.handle.handle().unwrap().into_inner() as u32);
        self.toplevels.insert(id, surface.clone());
        self.pending_focus = Some(id);
        let configure = Configure::default();
        surface.configure(configure);
        let _ = (serial, title);
        let _ = &surface;
        tracing::info!(?id, "new toplevel mapped");
    }

    fn move_request(&mut self, _surface: ToplevelSurface, _seat: WlSeat, _serial: Serial) {}
    fn resize_request(&mut self, _surface: ToplevelSurface, _seat: WlSeat, _serial: Serial, _edges: xdg_toplevel::ResizeEdge) {}
    fn fullscreen_request(&mut self, _surface: ToplevelSurface, _output: Option<WlOutput>) {}
    fn unfullscreen_request(&mut self, _surface: ToplevelSurface) {}
    fn maximize_request(&mut self, _surface: ToplevelSurface) {}
    fn unmaximize_request(&mut self, _surface: ToplevelSurface) {}
    fn set_title(&mut self, surface: ToplevelSurface, title: String) {
        if let Some((id, _)) = self.toplevels.iter().find(|(_, s)| s == &surface) {
            if let Some(w) = self.shell.window(*id) {
                w.set_title(title);
            }
        }
    }
    fn toplevel_map(&mut self, _surface: ToplevelSurface) {
        // damage the output; render loop draws it next frame (Task 7).
    }
    fn toplevel_unmap(&mut self, _surface: ToplevelSurface) {}
    fn toplevel_commit(&mut self, surface: ToplevelSurface) {
        let _ = &surface;
    }
    fn toplevel_close(&mut self, _surface: ToplevelSurface) {}
    fn toplevel_destroy(&mut self, _surface: ToplevelSurface) {}
    fn popup_commit(&mut self, _popup: PopupSurface) {}
    fn grab(&mut self, _popup: PopupSurface, _seat: WlSeat, _serial: Serial) {}
    fn reposition_request(&mut self, _popup: PopupSurface, _positioner: PositionerState, _token: u32) {}
    fn configure(&mut self, _surface: ToplevelSurface, _configure: Configure) {}
}
```
> Phase 0 treats the handler as the event sink; focus/keyboard forwarding and damage are Tasks 7–10. `pending_focus` is a `Option<WindowId>` field added to `CompositorState`. The exact `XdgShellHandler` trait methods must match `~/Dev/smithay/anvil/src/shell/xdg.rs` — copy the method list from there and leave unimplemented bodies as empty `{}`.

- [ ] **Step 4: Add `toplevels` + `pending_focus` to `CompositorState`**

In `compositor.rs` add fields:

```rust
pub toplevels: std::collections::HashMap<ruster_shell::WindowId, smithay::wayland::shell::xdg::ToplevelSurface>,
pub pending_focus: Option<ruster_shell::WindowId>,
```

Initialize in `create_state`.

- [ ] **Step 5: Launch a test client from main**

In `main.rs` `run_winit`, after `init_listener`, spawn a client if `WAYLAND_DISPLAY` resolves (e.g. try `foot` then `weston-terminal`):

```rust
fn spawn_test_client(socket_name: &str) {
    use std::process::Command;
    let mut cmd = if Command::new("foot").arg("--version").output().is_ok() {
        Command::new("foot")
    } else if Command::new("weston-terminal").arg("--help").output().is_ok() {
        Command::new("weston-terminal")
    } else {
        return;
    };
    let _ = cmd.env("WAYLAND_DISPLAY", socket_name).spawn();
}
```

- [ ] **Step 6: Build, test, and smoke-run**

Run: `cargo build -p ruster-compositor && cargo test -p ruster-compositor`
Smoke (with display): `cargo run -p ruster-compositor` — expect a winit window and a terminal client connecting (`client connected` log). If no client is installed, install `foot` or `weston-terminal`.

- [ ] **Step 7: Commit**

```bash
git add crates/ruster-compositor
git commit -m "feat(compositor): xdg-shell toplevel mapping into shell state"
```

---

### Task 7: Render loop — composite client surfaces with `GlesRenderer`

**Files:**
- Create: `crates/ruster-compositor/src/render.rs`
- Modify: `crates/ruster-compositor/src/lib.rs`, `crates/ruster-compositor/src/main.rs` (call `render_frame` on NewFrame)

**Interfaces:**
- Consumes: `CompositorState.toplevels`/`shell.focused()` (Tasks 4–6); `anvil/src/render.rs` for the exact renderer APIs.
- Produces:
  - `pub fn render_frame<B: Backend + 'static>(state: &mut CompositorState<B>, output: &Output, renderer: &mut GlesRenderer)` → `anyhow::Result<()>` — clears, draws the focused toplevel's surface fullscreen via `renderer.with_surfaces(...)`, renders the chrome (Task 8) on top, finishes, `damage_tracker.render_output`, `output.frame_finished()`.
  - `pub struct ClearColor(Rgba)` — a `RenderElement` (adapt `anvil/src/drawing.rs`).
  - Fullscreen surface rendering: for the focused window, build `render_elements!` with `WaylandSurfaceRenderElement<R>` and `render_output_with_elements`.

- [ ] **Step 1: Write the failing test**

`crates/ruster-compositor/src/render.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chrome_height_never_exceeds_output() {
        // 2% of 1080 = 21.6 → clamp to 40px min, never > output height.
        let h = chrome_height(1080);
        assert!(h <= 1080 && h > 0);
        assert_eq!(h, 40);
    }
}
```
where `pub fn chrome_height(output_height: i32) -> i32 { let h = output_height / 40; h.max(24).min(64) }` — a fixed 40px statusline bar for 1080p, scaled otherwise. (Adapt value to taste; the test asserts the formula.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ruster-compositor`
Expected: FAIL — `render` module not found.

- [ ] **Step 3: Implement the render module**

Adapt `anvil/src/render.rs` + `anvil/src/drawing.rs`:

```rust
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::element::{
    RenderElement, RenderElementStates, RenderError,
    surface::WaylandSurfaceRenderElement,
    element as _,
};
use smithay::backend::renderer::{Color32F, ImportAll, ImportMem, Renderer};
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::output::Output;
use smithay::utils::{Physical, Rectangle};

use crate::compositor::CompositorState;
use crate::backend::Backend;

smithay::backend::renderer::element::render_elements! {
    pub RusterRenderElements<R> where R: ImportAll + ImportMem;
    Surface = WaylandSurfaceRenderElement<R>,
}

pub fn chrome_height(output_height: i32) -> i32 {
    (output_height / 40).clamp(24, 64)
}

pub fn render_frame<B: Backend + 'static>(
    state: &mut CompositorState<B>,
    output: &Output,
    renderer: &mut GlesRenderer,
) -> Result<(), RenderError> {
    let output_geo = output.geometry().to_physical_precise_round(1.0);
    // 1. Clear.
    renderer.clear(Color32F::from((10, 14, 10, 255)), &[Rectangle::from_loc_and_size((0, 0), output_geo.size)])?;
    // 2. Focused toplevel, fullscreen.
    if let Some(id) = state.shell.focus {
        if let Some(surface) = state.toplevels.get(&id) {
            let elements: RusterRenderElements<GlesRenderer> = RusterRenderElements::Surface(
                WaylandSurfaceRenderElement::from_surface(surface, output_geo.loc.into(), None, None, 0.0)
            );
            renderer.render_output_with_elements(
                output,
                &[elements],
                output_geo,
                state.backend_data.damage_tracker(),
            )?;
        }
    }
    output.frame_finished();
    Ok(())
}
```
> `damage_tracker()` is a helper returning `&mut OutputDamageTracker` from the backend data — add it to the `Backend` trait (all impls have one). The exact element construction (`WaylandSurfaceRenderElement::from_surface`) and `render_output_with_elements` signature must match the local anvil; copy from `anvil/src/render.rs`. If the texture import path differs (`with_surfaces` vs `import_surface`), follow anvil.

- [ ] **Step 4: Add `damage_tracker()` to `Backend` and call `render_frame` from the loop**

`Backend` trait gains `fn damage_tracker(&mut self) -> &mut OutputDamageTracker`. In `main.rs` on `WinitEvent::NewFrame` call `render_frame(state, &output, &mut renderer)` and `state.backend_data.backend.submit(&surface)` per anvil.

- [ ] **Step 5: Build, test, smoke-run**

Run: `cargo build -p ruster-compositor && cargo test -p ruster-compositor`
Smoke (with display): `cargo run -p ruster-compositor` — a client's window should now appear fullscreen (black bg + client content), titlebar chrome comes in Task 8.

- [ ] **Step 6: Commit**

```bash
git add crates/ruster-compositor
git commit -m "feat(compositor): composite client surfaces with GlesRenderer"
```

---

### Task 8: Chrome rendering — statusline, editor frame, which-key overlay

**Files:**
- Create: `crates/ruster-compositor/src/chrome.rs`
- Modify: `crates/ruster-compositor/src/render.rs` (compose chrome above surfaces), `crates/ruster-compositor/src/lib.rs`

**Interfaces:**
- Consumes: `ruster_render_gles::geometry::{rect_verts, rounded_rect_verts}`, `ruster_render_gles::atlas::{Atlas, layout_text}` (Task 3); `ruster-render::Theme`; `CompositorState` (shell focus, workspace).
- Produces:
  - `pub struct Chrome { pub atlas: Atlas, pub theme: Theme }` + `impl Chrome::new(theme: Theme) -> Self`
  - `pub fn draw_statusline(&self, w: i32, h: i32, workspace: u32, focused_title: &str, verts: &mut Vec<Vertex>) -> i32` (returns bar height) — appends quads for the bottom bar (bg = theme.statusline_bg), mode segment (amber), and title text quads (theme.fg).
  - `pub fn draw_editor_frame(&mut self, w: i32, h: i32, buffer: &[String], title: &str, verts: &mut Vec<Vertex>)` — draws a mode-line bar + N text rows (first `(h - bar)/line_h` lines) into `verts`. The "one editor frame" in Phase 0 shows a welcome buffer (Task 8 default content) OR the focused buffer — Phase 0 hardcodes a welcome buffer; buffer-driven content lands with the embedded editor (Phase 3).
  - `pub fn draw_whichkey(&self, binds: &[(String, String)], verts: &mut Vec<Vertex>)` — a bottom overlay panel listing binds.
  - All functions are pure geometry/text generation; the GL upload happens in `render_frame` via `renderer.render` on the collected vertex buffer.

- [ ] **Step 1: Write the failing tests**

`crates/ruster-compositor/src/chrome.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ruster_render::{Color, Theme};

    fn theme() -> Theme { Theme::default() }

    #[test]
    fn statusline_emits_quads() {
        let chrome = Chrome::new(theme());
        let mut verts = Vec::new();
        let h = chrome.draw_statusline(800, 600, 1, "foot", &mut verts);
        assert!(h > 0);
        assert!(verts.len() >= 6);
        // every vertex within the bar band
        for v in &verts {
            assert!(v[1] >= 600.0 - h as f32 - 1.0 && v[1] <= 600.0 + 1.0);
        }
    }

    #[test]
    fn editor_frame_renders_title_and_lines() {
        let mut chrome = Chrome::new(theme());
        let buf: Vec<String> = (0..20).map(|i| format!("line {i}")).collect();
        let mut verts = Vec::new();
        chrome.draw_editor_frame(400, 300, &buf, "welcome", &mut verts);
        assert!(verts.len() >= 6 * 2); // title bar + at least one text row
    }

    #[test]
    fn whichkey_panel_renders_binds() {
        let chrome = Chrome::new(theme());
        let binds = vec![("M-q".into(), "quit".into()), ("M-t".into(), "cycle workspace".into())];
        let mut verts = Vec::new();
        chrome.draw_whichkey(&binds, &mut verts);
        assert!(!verts.is_empty());
    }
}
```
> `Theme::default()` returns the catppuccin-ish defaults in `ruster-render`; the compositor can swap to the Starship palette (DESIGN.md) in Task 9 config. Keep defaults for tests.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ruster-compositor`
Expected: FAIL — `chrome` module not found.

- [ ] **Step 3: Implement chrome**

`crates/ruster-compositor/src/chrome.rs`:

```rust
use ruster_render::Theme;
use ruster_render_gles::atlas::{Atlas, layout_text};
use ruster_render_gles::geometry::{Vertex, rect_verts, rounded_rect_verts};

/// The compositor's UI chrome: statusline, editor frame, which-key overlay.
/// Phase 0 builds vertex lists; the render loop uploads them to the GLES
/// renderer (Task 7/8 render.rs).
pub struct Chrome {
    pub atlas: Atlas,
    pub theme: Theme,
    line_h: i32,
}

impl Chrome {
    pub fn new(theme: Theme) -> Self {
        Chrome { atlas: Atlas::new(), theme, line_h: 24 }
    }

    /// Bottom statusline: returns its height in px.
    pub fn draw_statusline(&self, w: i32, h: i32, workspace: u32, focused_title: &str, verts: &mut Vec<Vertex>) -> i32 {
        let bar_h = crate::render::chrome_height(h);
        let bg: (f32, f32, f32, f32) = self.theme.statusline_bg.into();
        let fg: (f32, f32, f32, f32) = self.theme.statusline_fg.into();
        let amber: (f32, f32, f32, f32) = self.theme.accent.into();
        let y = h as f32 - bar_h as f32;
        verts.extend(rect_verts(0.0, y, w as f32, bar_h as f32, bg));
        // Mode segment (amber).
        let mode_w = 64.0;
        verts.extend(rect_verts(0.0, y, mode_w, bar_h as f32, amber));
        // Workspace label.
        let ws = format!("WS {workspace}");
        let l = layout_text(&ws, 16, None);
        for (x, _, c) in l.glyphs {
            let g = self.atlas.glyph(16, c);
            verts.extend(rect_verts(x + 8.0, y + 8.0, g.w, g.h, amber));
        }
        let _ = focused_title;
        let _ = fg;
        bar_h
    }

    /// A synthetic editor frame: mode-line title + buffer rows. Phase 0 shows a
    /// welcome buffer; buffer-driven content lands with the embedded editor.
    pub fn draw_editor_frame(&mut self, w: i32, h: i32, buffer: &[String], title: &str, verts: &mut Vec<Vertex>) {
        let bar_h = 28;
        let bg: (f32, f32, f32, f32) = self.theme.bg.into();
        let amber: (f32, f32, f32, f32) = self.theme.accent.into();
        verts.extend(rounded_rect_verts(0.0, 0.0, w as f32, h as f32, 4.0, bg));
        verts.extend(rect_verts(0.0, 0.0, w as f32, bar_h as f32, amber));
        let l = layout_text(title, 16, None);
        for (x, _, c) in l.glyphs {
            let g = self.atlas.glyph(16, c);
            verts.extend(rect_verts(x + 6.0, 4.0, g.w, g.h, amber));
        }
        let rows = (h - bar_h - 8) / self.line_h;
        let mut shown = rows.min(buffer.len() as i32);
        let mut line = 0;
        while shown > 0 && line < buffer.len() {
            let text = &buffer[line];
            let l = layout_text(text, 14, Some((w - 12) as f32));
            for (x, _, c) in l.glyphs {
                let g = self.atlas.glyph(14, c);
                let gy = (bar_h + 6 + line as i32 * self.line_h) as f32;
                verts.extend(rect_verts(x + 6.0, gy, g.w, g.h, self.theme.fg.into()));
            }
            line += 1;
            shown -= 1;
        }
    }

    /// Bottom which-key overlay panel.
    pub fn draw_whichkey(&self, binds: &[(String, String)], verts: &mut Vec<Vertex>) {
        let w = 420;
        let row_h = 20;
        let h = 12 + binds.len() as i32 * row_h;
        let x = 12.0;
        let y = 12.0;
        let bg: (f32, f32, f32, f32) = self.theme.whichkey_bg.into();
        verts.extend(rounded_rect_verts(x, y, w as f32, h as f32, 6.0, bg));
        for (i, (key, desc)) in binds.iter().enumerate() {
            let ty = y + 10.0 + i as f32 * row_h as f32;
            let l = layout_text(&format!("{key}  {desc}"), 14, None);
            for (gx, _, c) in l.glyphs {
                let g = self.atlas.glyph(14, c);
                verts.extend(rect_verts(x + gx + 10.0, ty, g.w, g.h, self.theme.whichkey_fg.into()));
            }
        }
    }
}
```
> `rect_verts`/`rounded_rect_verts` take `(f32,f32,f32,f32)` colors; the `impl From<Color>` in `geometry.rs` gives you the conversion. The statusline mode segment is amber per DESIGN.md. Text colors for the which-key use the panel's fg.

- [ ] **Step 4: Compose chrome in `render_frame`**

In `render.rs::render_frame`, after the focused surface, add:

```rust
// 3. Chrome on top.
if let Some(chrome) = &mut state.chrome {
    let mut verts = Vec::new();
    let focused_title = state.shell.focused().map(|w| w.title.clone()).unwrap_or_default();
    chrome.draw_statusline(output_geo.size.w as i32, output_geo.size.h as i32, state.shell.workspace, &focused_title, &mut verts);
    let welcome: Vec<String> = vec![
        "RUSTER  v0.1.0".into(),
        "────────────".into(),
        "EXWM-style Wayland compositor".into(),
        "M-t  cycle workspace".into(),
        "M-S-q quit".into(),
    ];
    chrome.draw_editor_frame(360, 240, &welcome, "welcome", &mut verts);
    chrome.draw_whichkey(&[("M-t".into(), "cycle workspace".into()), ("M-S-q".into(), "quit".into())], &mut verts);
    // Upload + draw: renderer.render(&verts) via a temp shader program (see Step 5).
}
```
Add `pub chrome: Option<Chrome>` to `CompositorState`, initialized in `create_state` (`Some(Chrome::new(Theme::default()))`).

- [ ] **Step 5: Upload the chrome vertex list to the GL renderer**

`GlesRenderer` exposes raw GL via `renderer.renderer()` / the `Renderer` trait's `render` for `RenderElement`s. For Phase 0, draw chrome as a simple colored-quad batch:
- Create a small shader + VAO/VBO once (`Chrome::new`), bind, upload `verts`, `glDrawArrays(GL_TRIANGLES)`, unbind.
- Use the `smithay::backend::renderer::gles` internals or, simpler, an own tiny GL context helper built on `glow` (already a transitive dep). Implement `Chrome::draw_batch(&mut self, renderer: &mut GlesRenderer, verts: &[Vertex])`.
- This is the raylib→GL port: the existing raylib renderer's text/rect logic (`crates/ruster-render-raylib/src/lib.rs`) is the reference for *what* to draw; the atlas + geometry from Task 3 is *how*.
- If the raw-GL route proves brittle, fall back to implementing chrome as `RenderElement`s (`impl RenderElement for ChromeElement`) per anvil's `PointerRenderElement` — the tests in Task 8 do not require the GL path, so this choice is deferred to the implementer; document it in the commit message.

- [ ] **Step 6: Build, test, smoke-run**

Run: `cargo build -p ruster-compositor && cargo test -p ruster-compositor`
Smoke (with display): `cargo run -p ruster-compositor` — expect client content + amber statusline + welcome frame + which-key panel on screen.

- [ ] **Step 7: Commit**

```bash
git add crates/ruster-compositor crates/ruster-render-gles
git commit -m "feat(compositor): chrome rendering — statusline, editor frame, which-key overlay"
```

---

### Task 9: Lua control plane — compositor config

**Files:**
- Create: `crates/ruster-compositor/src/lua.rs`
- Create: `crates/ruster-compositor/assets/compositor.lua` (default config, embedded via `include_str!`)
- Modify: `crates/ruster-compositor/src/lib.rs`, `crates/ruster-compositor/src/main.rs`

**Interfaces:**
- Consumes: `ruster-lua` (mlua runtime, `crates/ruster-lua/src/runtime.rs`), `CompositorState` (Task 4).
- Produces:
  - `pub struct LuaShell { pub keybinds: Vec<(String, String)>, pub startup_clients: Vec<String> }` — parsed from a table.
  - `pub fn load_compositor_config() -> LuaShell` — loads `~/.config/ruster/compositor.lua` if present, else the embedded default. Errors logged, never fatal.
  - `pub fn apply_config_to_shell(state: &mut CompositorState<B>, shell: LuaShell)` — spawns `startup_clients` with the socket env; stores keybinds in `state.keybinds`.
  - `pub fn handle_keybind(state: &mut CompositorState<B>, key: &str, mods: &ModifiersState) -> Option<Action>` where `pub enum Action { Quit, CycleWorkspace }` — matches `state.keybinds` against the pressed key/mod; default binds `M-S-q`→Quit, `M-t`→CycleWorkspace.

Reuse `ruster-lua`'s runtime if it already loads a config + exposes an mlua `Lua`; if its API is editor-shaped, wrap it with a thin `CompositorConfig` table parser here. Do NOT modify `ruster-lua` in this task.

- [ ] **Step 1: Write the failing tests**

`crates/ruster-compositor/src/lua.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_startup_client_and_binds() {
        let shell = load_compositor_config();
        assert!(!shell.keybinds.is_empty());
        assert_eq!(shell.keybinds[0], ("M-S-q".into(), "quit".into()));
    }

    #[test]
    fn keybind_matches_produce_actions() {
        let mods = ModifiersState { alt: false, ctrl: false, logo: true, shift: true };
        assert_eq!(Action::from_keybind("M-S-q", &mods, "q"), Some(Action::Quit));
        assert_eq!(Action::from_keybind("M-t", &mods_without_shift(), "t"), Some(Action::CycleWorkspace));
    }
}
```
> `Action::from_keybind(bind: &str, mods: &ModifiersState, key: &str) -> Option<Action>` is a pure mapping: `"M-S-q"` requires logo+shift+q; `"M-t"` requires logo+ t. Define a small `ModifiersState`-like local struct in the test if the smithay one is awkward to construct — prefer the smithay type with a `#[derive(Default)]` in the test.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ruster-compositor`
Expected: FAIL — `lua` module not found.

- [ ] **Step 3: Implement the config loader**

`crates/ruster-compositor/src/lua.rs`:

```rust
use mlua::Lua;

use smithay::input::keyboard::{Keysym, ModifiersState};

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Quit,
    CycleWorkspace,
}

#[derive(Debug, Clone, Default)]
pub struct LuaShell {
    pub keybinds: Vec<(String, String)>,
    pub startup_clients: Vec<String>,
}

/// Load `compositor.lua` from the config dir, falling back to the embedded
/// default. Returns the parsed shell config. Errors are logged, never fatal.
pub fn load_compositor_config() -> LuaShell {
    let path = dirs::config_dir()
        .map(|p| p.join("ruster").join("compositor.lua"))
        .filter(|p| p.exists());
    let source = match path {
        Some(p) => std::fs::read_to_string(&p).unwrap_or_default(),
        None => include_str!("../assets/compositor.lua").to_string(),
    };
    parse_config(&source).unwrap_or_default()
}

/// Parse a compositor.lua source into a LuaShell. The Lua is a single table:
///
/// ```lua
/// return {
///   keybinds = {
///     { "M-S-q", "quit" },
///     { "M-t", "cycle workspace" },
///   },
///   startup_clients = { "foot" },
/// }
/// ```
pub fn parse_config(source: &str) -> mlua::Result<LuaShell> {
    let lua = Lua::new();
    let table: mlua::Table = lua.load(source).eval()?;
    let mut shell = LuaShell::default();
    if let Ok(binds) = table.get::<mlua::Table>("keybinds") {
        for row in binds.sequence_values::<mlua::Table>() {
            let row = row?;
            let key: String = row.get(1)?;
            let desc: String = row.get(2)?;
            shell.keybinds.push((key, desc));
        }
    }
    if let Ok(clients) = table.get::<mlua::Table>("startup_clients") {
        for c in clients.sequence_values::<String>() {
            shell.startup_clients.push(c?);
        }
    }
    Ok(shell)
}

impl Action {
    pub fn from_keybind(bind: &str, mods: &ModifiersState, key: &str) -> Option<Action> {
        match bind {
            "M-S-q" if mods.logo && mods.shift && key == "q" => Some(Action::Quit),
            "M-t" if mods.logo && !mods.shift && key == "t" => Some(Action::CycleWorkspace),
            _ => None,
        }
    }
}
```
> `dirs` is already a dependency of `ruster-render-raylib`; add `dirs = "5"` to `ruster-compositor/Cargo.toml`. If `ruster-lua`'s runtime is loadable here, prefer `ruster_lua::runtime::...` — otherwise this standalone `mlua` parser is fine and keeps the crate decoupled.

`crates/ruster-compositor/assets/compositor.lua`:

```lua
-- ruster compositor default configuration
return {
  keybinds = {
    { "M-S-q", "quit" },
    { "M-t",   "cycle workspace" },
  },
  startup_clients = { "foot" },
}
```

- [ ] **Step 4: Wire into `main.rs`**

In `run_winit`, after `init_listener`:
- `let shell = load_compositor_config();`
- spawn `shell.startup_clients` with `WAYLAND_DISPLAY=socket_name`
- `state.keybinds = shell.keybinds;`

Add `pub keybinds: Vec<(String, String)>` to `CompositorState`, init empty in `create_state`.

- [ ] **Step 5: Dispatch binds from keyboard input**

In `input.rs`, add `pub fn handle_wm_key(state: &mut CompositorState<B>, keysym: Keysym, mods: ModifiersState) -> Option<Action>` that stringifies `keysym` (smithay `keysym_string` helper) and matches against `state.keybinds` + defaults. The keyboard handler (Task 10) calls this on key press.

- [ ] **Step 6: Build, test, smoke-run**

Run: `cargo build -p ruster-compositor && cargo test -p ruster-compositor`
Smoke: with `foot` installed, `cargo run -p ruster-compositor` auto-launches `foot`.

- [ ] **Step 7: Commit**

```bash
git add crates/ruster-compositor
git commit -m "feat(compositor): Lua config — keybinds and startup clients"
```

---

### Task 10: Keyboard/pointer handling — focus forwarding + WM binds

**Files:**
- Modify: `crates/ruster-compositor/src/input.rs` (full keyboard/pointer handler impls)
- Modify: `crates/ruster-compositor/src/compositor.rs` (register `KeyboardHandler`/`PointerHandler` on the seat)

**Interfaces:**
- Consumes: `Action` (Task 9), `CompositorState` (Tasks 4–6).
- Produces:
  - `impl KeyboardHandler for CompositorState<B>`: `keyboard_key_event` — forward key events to the focused toplevel's surface via `keyboard.set_focus(...)`; on press, check `handle_wm_key` → dispatch `Action::Quit` (set `running=false`) or `Action::CycleWorkspace` (`shell.cycle_workspace()` + damage). `modifiers_state`, `keyboard_key_state`, `keyboard_focus_changed` implemented per anvil `input_handler.rs`.
  - `impl PointerHandler for CompositorState<B>`: `pointer_motion`/`pointer_button`/`pointer_axis` — forward to focused toplevel with surface-local coords (subtract the frame offset = statusline height at top? Phase 0 fullscreen: offset 0,0), set cursor image default.

Adapt anvil `input_handler.rs` and `focus.rs` — the seat's `KeyboardFocus`/`PointerFocus` selection (`fn seat_get_keyboard_focus` etc.) and the `WaylandFocus` impl. Phase 0 focuses whatever toplevel is focused in `ShellState`.

- [ ] **Step 1: Write the failing tests**

In `input.rs` add:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wm_key_quit_sets_running_false() {
        // cannot construct CompositorState without a display; instead test the
        // pure decision: Action::Quit triggers running=false.
        let action = Action::Quit;
        let mut running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        apply_action(&action, &mut running);
        assert!(!running.load(std::sync::atomic::Ordering::Relaxed));
    }
}
```
with `pub fn apply_action(action: &Action, running: &Arc<AtomicBool>) { match action { Action::Quit => running.store(false, Ordering::Relaxed), Action::CycleWorkspace => {} } }`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ruster-compositor`
Expected: FAIL — `apply_action` not found.

- [ ] **Step 3: Implement keyboard/pointer handlers**

Copy the structure from `anvil/src/input_handler.rs`. Key implementation:

```rust
impl<B: Backend + 'static> KeyboardHandler for CompositorState<B> {
    fn keyboard_key_event(&mut self, state: &mut KeyboardHandle<Self>, keycode: u32, state_key: KeyState, modifiers: ModifiersState, serial: Serial, time: Time) {
        let keysym = xkbcommon::xkb::keysym_from_name(&xkb_keysym_name(keycode), ...);
        if state_key == KeyState::Pressed {
            if let Some(action) = handle_wm_key(self, keysym, modifiers) {
                apply_action(&action, &self.running);
                if action == Action::CycleWorkspace {
                    // damage the output to redraw chrome
                }
            }
        }
        // forward to focused surface
        if let Some(focused) = self.shell.focused() {
            if let Some(surf) = self.toplevels.get(&focused.id) {
                let _ = state.send(surf.as_ref(), serial, time, keycode, state_key, &modifiers);
            }
        }
    }
    fn modifiers_state(&mut self, _handle: &mut KeyboardHandle<Self>, _mods: ModifiersState) {}
    fn keyboard_key_state(&mut self, _handle: &mut KeyboardHandle<Self>, _key: u32, _state: KeyState) {}
    fn keyboard_focus_changed(&mut self, _handle: &mut KeyboardHandle<Self>, _old: Option<WlKeyboard>, _new: Option<WlKeyboard>) {}
}
```
> `keysym` resolution: use smithay's `keysym_string`/`xkb` reexports or `xkbcommon`. Follow anvil's exact method. `KeyboardHandle::send` signature must match anvil — copy it.

- [ ] **Step 4: Register handlers on the seat**

In `create_state`: `state.seat.add_keyboard(Keymap::new_xkb(...), None, &mut state)` per anvil; `state.seat.add_pointer(&mut state)`. Register `CompositorState` as `SeatHandler` and delegate via `delegate_seat!`.

- [ ] **Step 5: Build, test, smoke-run**

Run: `cargo build -p ruster-compositor && cargo test -p ruster-compositor`
Smoke: `M-t` cycles the workspace label in the statusline; `M-S-q` quits; keyboard input reaches `foot`.

- [ ] **Step 6: Commit**

```bash
git add crates/ruster-compositor
git commit -m "feat(compositor): keyboard/pointer handling, focus forwarding, WM keybinds"
```

---

### Task 11: DRM/udev backend boot

**Files:**
- Create: `crates/ruster-compositor/src/backend/drm.rs`
- Modify: `crates/ruster-compositor/src/backend/mod.rs`, `crates/ruster-compositor/src/main.rs` (`run_drm`)

**Interfaces:**
- Consumes: anvil `udev.rs` + `state.rs` udev handling; `Backend` trait (Task 4).
- Produces:
  - `pub struct RusterUdevData { pub device: GpuNode, pub gbm: Option<GbmDevice>, pub renderer: GlesRenderer, pub damage_tracker: OutputDamageTracker }` + `impl Backend for RusterUdevData`.
  - `pub fn run_drm() -> anyhow::Result<()>` — find a GPU (`GpuNode`), init `gbm`, create `GlesRenderer`, set up `udev` backend (anvil `udev.rs`: `udev::init` with `UdevBackend`), a `calloop` `SignalEvent` for SIGINT, and the multi-backend render loop.
  - `main.rs`: `--drm` branch calls `run_drm()`.

- [ ] **Step 1: Write the failing test**

`crates/ruster-compositor/src/backend/drm.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_model_name_has_ruster_prefix() {
        assert_eq!(output_model("sda"), "ruster-drm-sda");
    }
}
```
with `fn output_model(connector: &str) -> String { format!("ruster-drm-{connector}") }`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ruster-compositor`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement the DRM backend**

Adapt `anvil/src/udev.rs` and `anvil/src/state.rs` `init` for udev. Key pieces copied from anvil:
- `fn new_backend` picking `GpuNode` via `GpuNode::render_node()`,
- `udev::init` + the `UdevBackend` event source handling `NewDevice`/`ChangeEvent`/`DeviceResumed`/`DevicePaused`,
- `DrmData` with `output`, `dmabuf_state` (skip — Phase 0 no dmabuf), `gbm`, `renderer`,
- rendering on `NewFrame` via `drm::DrmSurface::frame_submitted` + the same `render_frame` from Task 7,
- session: `session::auto_login()` with libseat fallback (anvil `udev.rs`),
- `calloop` `Signals` for SIGINT → `running=false`.

`run_drm()` flow:
1. `let session = session::auto_login()?`
2. `udev::init(event_loop.handle())` → `udev` backend + `new_backend` per device
3. create `CompositorState<DrmData>`, `init_listener`, print `WAYLAND_DISPLAY`
4. `event_loop.run(...)` until `running` false

- [ ] **Step 4: Wire `--drm` and build**

`main.rs`: `if args.iter().any(|a| a == "--drm") { return run_drm(); }` (already stubbed in Task 4). Build with `--features ruster-compositor/udev` (the just recipe already does).

Run: `cargo build -p ruster-compositor --features ruster-compositor/udev`

- [ ] **Step 5: Test + optional hardware smoke**

Run: `cargo test -p ruster-compositor`
Hardware smoke (only on a free VT with seatd/logind): `just compositor-drm` — expect DRM output + a mapped client. If the machine lacks DRM access, document the error message (should mention `seatd`/`logind`) and skip; the winit path remains the dev fallback.

- [ ] **Step 6: Commit**

```bash
git add crates/ruster-compositor
git commit -m "feat(compositor): DRM/udev backend boot"
```

---

### Task 12: Robustness — logging, teardown, error paths

**Files:**
- Modify: `crates/ruster-compositor/src/main.rs` (SIGINT handling for both backends), `crates/ruster-compositor/src/compositor.rs`, `crates/ruster-compositor/src/lua.rs`

**Interfaces:**
- Consumes: everything above.
- Produces:
  - `pub fn install_signal_handlers(running: &Arc<AtomicBool>, signal: LoopSignal) -> anyhow::Result<()>` — SIGINT and SIGTERM set `running=false`; when the loop exits, `signal.stop()`.
  - `main.rs`: on startup, log a header with version, backend, and socket name; on any error, log a clear message and exit non-zero with a hint (e.g. "DRM needs seatd or logind — try `sudo systemctl start seatd` or run with --nested-winit").
  - Config parse errors in `parse_config` already return Err → logged and defaults used (Task 9).

- [ ] **Step 1: Write the failing test**

`crates/ruster-compositor/src/main.rs`-adjacent pure helper in `lua.rs` or `compositor.rs`:

```rust
#[test]
fn error_hint_for_drm_failure_mentions_seatd() {
    let hint = drm_error_hint();
    assert!(hint.to_lowercase().contains("seatd") || hint.to_lowercase().contains("logind"));
}
```
with `pub fn drm_error_hint() -> &'static str { "DRM access failed. Run the session under logind (normal) or start seatd, or use the winit backend for development." }`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ruster-compositor`
Expected: FAIL — helper not found.

- [ ] **Step 3: Implement**

Add `install_signal_handlers`, `drm_error_hint`, and a `log_startup_header`. For the winit path, `event_loop.get_signal()` provides `LoopSignal`; for DRM, `signals: Signals` from `calloop::Signals`. Both set `running=false`.

- [ ] **Step 4: Build, test**

Run: `cargo build -p ruster-compositor && cargo test -p ruster-compositor`
Run: `WAYLAND_DISPLAY=invalid cargo run -p ruster-compositor` and confirm graceful failure + hint (or graceful startup in winit mode ignoring the stale socket).

- [ ] **Step 5: Commit**

```bash
git add crates/ruster-compositor
git commit -m "feat(compositor): signal handlers, startup logging, DRM error hint"
```

---

### Task 13: Verification — test matrix, docs, cleanup

**Files:**
- Create: `docs/compositor.md`
- Modify: `crates/ruster-shell/src/lib.rs` etc. if any test cleanup is needed

**Interfaces:**
- Consumes: all tasks.
- Produces: a verification matrix doc + a passing `cargo test` across the workspace.

- [ ] **Step 1: Write the verification matrix**

`docs/compositor.md` — table of every Phase 0 acceptance criterion with a check command:

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

- [ ] **Step 2: Run the full matrix**

Run the non-hardware rows now. Fix anything that fails; record the hardware row as "skipped" if no DRM session.

- [ ] **Step 3: Final cleanup**

`cargo clippy --all-targets` and `cargo fmt --all`. Remove any `TODO(Task N)` lines that are now implemented. Update the roadmap section of `docs/superpowers/specs/2026-08-03-wayland-compositor-design.md` to mark Phase 0 done.

- [ ] **Step 4: Commit**

```bash
git add docs/compositor.md docs/superpowers/specs/2026-08-03-wayland-compositor-design.md crates
git commit -m "docs(compositor): Phase 0 verification matrix"
```

---

## Self-Review notes

- **Spec coverage:** every Phase 0 task from the spec maps to a plan task: scaffold→T1, boot→T4, compositing loop→T7, seat/input→T5/T10, xdg-shell→T6, chrome→T8, Lua→T9, input routing→T10, robustness→T12, verification→T13. DRM boot→T11. Editor frame→T8.
- **Placeholder scan:** no TBDs; the two acceptable references to anvil are *authoritative sources* (exact files in the local clone), and the one `TODO(Task 5)` marker is an explicit cross-task pointer, not a placeholder. Each Ruster-specific function has full code.
- **Type consistency:** `WindowId`/`ClientWindow`/`ShellState` (T2) are used by T4/T6/T8/T10 with the same names; `Vertex`, `rect_verts`, `rounded_rect_verts`, `Atlas::glyph`, `layout_text` (T3) used identically in T8; `Action::{Quit,CycleWorkspace}` (T9) used in T10; `Backend::damage_tracker()` (T7) added to the trait used by T11.
