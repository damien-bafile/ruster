# GPUI Elements Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the hand-built chrome geometry in `crates/ruster-compositor/src/chrome.rs` (~1,600 lines) with a new portable crate `ruster-render-elements` — a GPUI-style `div()`/`text()` builder, a taffy flexbox engine, and stable element IDs — that compiles down to the same `ChromeBatch` output the renderer already emits, with byte-identical chrome pixels and no regression on the glyph budget.

**Architecture:** A declarative one-scene-per-frame path: `chrome_scene(frame, theme, measurer) -> Elem` builds one element tree; the portable crate's `layout(area, root, measurer) -> LayoutScene` runs taffy and emits a pure `boxes + texts` list in painter's order; render-gles's `scene_to_chrome_batch(scene, atlas) -> ChromeBatch` tessellates it through the existing atlas into the existing `ChromeBatch` (which gains a parallel `glyph_keys` list). The compositor's `collect_render_elements` flips from calling `Chrome::draw_*` to `build → layout → scene_to_chrome_batch → existing emission`. Glyph ids become keyed (`HashMap<ElementKey, Vec<Id>>`) instead of position-keyed, so reordering chrome no longer remaps render-element ids. All layout/pixel math stays f32 (no rounding), the pane `FrameBody` grid and `runs()`/`severity_sign()` logic are untouched, and the hover panel keeps its own front-of-everything layer exactly as `OverlayBatch` does today.

**Tech Stack:** Rust, `taffy` (flexbox engine; **0.13.0**, the current latest), existing `ruster-render` (`Theme`, `StyledLine`, `Color`), existing render-gles `Atlas`/`layout_text_in`/`glyph_in`/`rect_verts`/`rounded_rect_verts`. No new deps besides taffy. The elements crate depends only on `ruster-render` + taffy and compiles on every platform.

## Deviations from this plan (recorded as they were found)

The spec (`docs/superpowers/specs/2026-08-15-gpui-elements-design.md`, commit `4307c3f`) described several things that are not true of the codebase, plus one open question this plan resolves. Each is implemented as close to the intent as reality allows.

| Spec says | Reality | Done instead |
|---|---|---|
| taffy "0.7.x" (Open question 1) | `cargo search taffy` → latest is **0.13.0**; not in Cargo.lock or the local registry cache, so the first build fetches it | Pin `taffy = "0.13"`. API: `TaffyTree::<Ctx>::new()`, `new_leaf_with_context`, `new_with_children`, `compute_layout_with_measure(root, space, measure)` with `measure: FnMut(Size<Option<f32>>, Size<AvailableSpace>, NodeId, Option<&mut T>, &Style) -> Size<f32>` capturing `&mut measurer`, `layout(node)` → `.location: Point<f32>` + `.size` |
| `layout(area: Rect, ...)` | ruster-render's `Rect` is a u16 cell grid, not physical px | The elements crate defines its own `PxRect { x, y, w, h: f32 }`; `layout(area: PxRect, ...)` |
| "one `FontSystem`, not two" (Risk row) | `layout_text`/`layout_text_in` already route through a thread-local `FONT_SYSTEM` (`atlas.rs:429`); the measurer can reuse the exact same functions | `GlesTextMeasurer` wraps `layout_text_in` (one shared `FontSystem`); measured width == tessellated width by construction |
| "Boxes already use stable `Id::new()` per element via smithay's element list" | `SolidColorRenderElement::from_buffer` takes **no** id; the id comes from a fresh `SolidColorBuffer::new()` per quad per frame (`solid.rs:81`, `from_buffer`), so boxes are unique-but-not-stable across frames | Box ids unchanged in v1 (not worse than today). **Only glyph/text elements get keyed stable ids** — the `glyph_keys` mechanism is the v1 deliverable |
| Ordering invariant "hover above panes, statusline above panes" | The real painter's order in `collect_render_elements` is **borders → statusline → minibuffer → whichkey → panes → hover** (`render.rs:192-301`) — panes draw **after** the statusline, and a comment at `render.rs:222-223` claiming panes sit "below the statusline" contradicts the code | The ordering test asserts the **actual** old order; `chrome_scene` composes borders, statusline, minibuffer, whichkey, then per-pane frames, then hover in exactly that order |
| `chrome_scene(frame) -> Elem` "in render.rs" | Needs the theme (from `Chrome`) and the measurer (hover pre-measures); also the ordering test needs a seam that does not build a full `FrameInput` | `chrome_scene(frame, theme, measurer)` in a new `crates/ruster-compositor/src/scene.rs` (`pub mod scene;` added to lib.rs). Per-widget `*_elem` builders return `Elem`; the ordering test drives a shared composition helper with synthetic pieces instead of a `FrameInput` |
| `collect_render_elements` emission "glyphs(batch) then reversed solids(batch)" | Hover is already a separate `OverlayBatch` emitted **ahead** of the base layer so its panel covers base *glyphs* (the hoist makes every base glyph beat every base panel — `render.rs:312-345`) | Hover keeps its own layer: `chrome_scene` returns `(Elem, Option<Elem>)` for (base, hover-overlay); `scene_to_chrome_batch` runs on each; emission order is preserved exactly as today |
| `whichkey_elem(output_w, output_h, view, theme, measurer)` measuring via measurer | The panel's width comes from taffy sizing the text leaves; but the **column chunking** (`per_col`, `columns`, `rows_drawn`, `w`/`h`) is pure arithmetic already done in `draw_whichkey` | `whichkey_elem` keeps that arithmetic, applies `.max_w(output_w - 24)`, and lets taffy size columns from measured text; parity test verifies equality |
| `hover_elem(..., measurer)` | Hover needs `text_w` **before** layout to decide above/below flip and clamp | `hover_elem` pre-measures its lines via the injected measurer, then builds an absolutely positioned panel; text re-measures during layout (same widths) |
| FontFamily's `attrs()` method | `attrs()` returns cosmic-text `Attrs<'static>`, which is Linux-only; the enum itself is portable | Enum moves to ruster-render; `family_attrs(FontFamily) -> Attrs<'static>` becomes a free fn in render-gles `atlas.rs`; `pub use` re-exports keep `ruster_render_gles::atlas::{... FontFamily}` and compositor imports compiling |
| taffy rounds to integers by default | Chrome geometry is fractional f32 today (e.g. `(mode_w-16)/2 = 24`, hover `max(4.0)`) | `taffy.config.use_rounding = false` (set on `TaffyTree` config) |
| `.min_w_0()` mirrors GPUI's `min-width: auto` | taffy 0.13 auto min size can grow a flex item past its content on overflow | Shipped; the pane's per-line text nodes are absolutely positioned so this only matters for which-key/statusline columns (parity-tested) |
| Spec's `Elem` has `id: Option<String>` on the struct | Id path must be derivable during the layout walk, including for **un-named** children | `Style` carries `id: Option<String>`; the build walk derives `ElementKey` per node (root `ElementKey(vec![])`, child = parent + `id` or index), so `layout` builds keys without a separate tree walk |
| — | `ruster-core/workspace.rs:132` has `impl Default for BufferStore`, but Panes, Highlights, Keymap, LspState have no confirmed `Default` | Ordering test drives a composition helper, **not** a `FrameInput` |

## Global Constraints

- **Spec source of truth:** `docs/superpowers/specs/2026-08-15-gpui-elements-design.md` (commit `4307c3f`); this plan's deviations above override it where reality differed.
- **No pixel regression:** the parity test (Task 5) must pass — new scene rects/glyphs equal old `draw_*` output — **before** `render.rs` flips (Task 6). `docs/compositor.md` visual rows re-verified after.
- **No render budget regression:** `RUSTER_BENCH_GLYPHS=10000` frame time (info log `"frame time"`, `winit_main.rs:83`) must stay at or below today's ~4.7ms.
- **No new deps beyond taffy** (in `ruster-render-elements` only). Everything else reuses existing crates (`ruster-render`, render-gles atlas/geometry).
- **No regression:** `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings` green at every commit. All existing tests stay green (chrome.rs tests `statusline_emits_quads`, `whichkey_panel_renders_its_view`, `a_cell_origin_is_where_a_click_on_that_cell_lands`, hover tests; geometry.rs tests; atlas tests).
- **Untouched:** `FrameBody` hit-testing, `runs()`/`syntax_color()`, `severity_sign()`, `gutter_width`, `SIGN_COLS`, the atlas's glyph rasterization, cursor elements, client-surface emission, the winit/DRM boot paths.
- **One scene per frame, pure data:** `LayoutScene { boxes, texts }` is `#[derive(Debug, Clone, PartialEq)]` plain data, unit-testable with no GPU.
- **f32 everywhere, no rounding:** element geometry stays f32 physical px; `use_rounding = false`; pane `w`/`h` truncated `as i32` exactly where `draw_editor_frame` truncates today.
- **Keyed ids, not positional:** `Chrome.glyph_ids: Vec<Id>` → `id_map: HashMap<ElementKey, Vec<Id>>`; `Chrome::element_ids(&key, len) -> Vec<Id>` allocates on first sight; `glyph_elements` groups consecutive equal keys.
- **Definition of done (all gates):** crate is a workspace member; FontFamily/ChromeBatch relocated with re-exports; measurer + tessellator tested; parity green before flip; flip deletes `draw_*`, `mark`/`translate_since`, `glyph_ids`, `OverlayBatch`; all three cargo commands clean; `docs/compositor.md` rows + budget re-verified and recorded; AGENTS.md workspace crate list + `docs/config-reference.md`/`docs/lua-api.md`/`docs/keybindings.md` untouched (no config/Lua/keybinding surface changes).

## File structure

| Path | Status | Responsibility |
|---|---|---|
| `Cargo.toml` | edit | workspace member `crates/ruster-render-elements` |
| `crates/ruster-render-elements/Cargo.toml` | NEW | deps `ruster-render` (path), `taffy = "0.13"` |
| `crates/ruster-render-elements/src/lib.rs` | NEW | `pub mod element; pub mod style; pub mod layout; pub mod id;` re-exports |
| `crates/ruster-render-elements/src/element.rs` | NEW | `Elem`, `ElemKind::{Container, Text}`, `div()`, `text(impl Into<StyledLine>)`, `children(Vec<Elem>)`, `Styled` trait |
| `crates/ruster-render-elements/src/style.rs` | NEW | `Style { taffy: taffy::Style, bg, fg, radius, border_width, border_color, font_size, font_family, bold, id }` + all Styled methods |
| `crates/ruster-render-elements/src/layout.rs` | NEW | `TextMeasurer` trait, `PxRect`, `BoxNode`, `TextNode`, `LayoutScene`, `layout(area, root, &mut dyn TextMeasurer)` |
| `crates/ruster-render-elements/src/id.rs` | NEW | `ElementKey(pub Vec<String>)` with `Default` (empty root) + `push`/`child` helpers |
| `crates/ruster-render/src/lib.rs` | edit | `pub enum FontFamily { Ui, Mono }` (moved, derives Debug/Clone/Copy/PartialEq/Eq/Hash/Default) |
| `crates/ruster-render-gles/src/atlas.rs` | edit | delete `FontFamily` + `FontFamily::attrs`; add `pub use ruster_render::FontFamily;`, `pub fn family_attrs(FontFamily) -> Attrs<'static>`; keep `layout_text`, `layout_text_in`, `cell_metrics`, `glyph_in` |
| `crates/ruster-render-gles/src/geometry.rs` | edit | + `ChromeBatch { verts, glyphs, glyph_keys }`, `BatchMark`, `mark()`, `translate_since()` (moved from compositor) |
| `crates/ruster-render-gles/src/tessellate.rs` | NEW | `GlesTextMeasurer`, `scene_to_chrome_batch(&LayoutScene, &mut Atlas) -> ChromeBatch` (boxes → rect/rounded/border quads; texts → `layout_text_in` → `atlas.glyph_in` → `GlyphQuad` + `glyph_keys`) |
| `crates/ruster-render-gles/src/lib.rs` | edit | `pub mod tessellate;` + dep on `ruster-render-elements` |
| `crates/ruster-render-gles/Cargo.toml` | edit | dep `ruster-render-elements` (path) |
| `crates/ruster-compositor/src/scene.rs` | NEW | `chrome_scene(frame, theme, measurer) -> (Elem, Option<Elem>)`; `*_elem` builders; `compose` helper for the ordering test; `GlesTextMeasurer` reused from render-gles |
| `crates/ruster-compositor/src/chrome.rs` | edit | Task 2: delete `ChromeBatch`/`BatchMark`/`mark`/`translate_since` (moved), imports from render-gles; Task 6: delete `draw_*`, `glyph_ids` → `id_map` + `element_ids`; keep `FrameBody`, `runs`, `severity_sign`, `gutter_width`, `SIGN_COLS`, `FRAME_*`/`SIGN_*` consts, `HoverAnchor` (moved to scene.rs or kept), `OverlayBatch` (deleted at flip), tests re-anchored |
| `crates/ruster-compositor/src/render.rs` | edit | Task 6: `collect_render_elements` = build scene → layout → tessellate → existing emission; delete `glyph_elements` position-keyed id call (now keyed); `bench_glyphs` pushes unique keys |
| `crates/ruster-compositor/src/lib.rs` | edit | `pub mod scene;` |
| `docs/AGENTS.md` | edit | workspace crate list gains `ruster-render-elements` |
| `docs/compositor.md` | edit | re-verify + record visual rows and `RUSTER_BENCH_GLYPHS` frame time |

## Tasks

### Task 1 — Move `FontFamily` and `ChromeBatch`

Relocate the two shared types the portable crate must name, with source-compatibility re-exports.

- [ ] **TDD fail:** add a compile-error test — `cargo build -p ruster-render` fails because `ruster_render::FontFamily` does not exist.
- [ ] Move `FontFamily` enum (derives Debug/Clone/Copy/PartialEq/Eq/Hash/Default) from `ruster-render-gles/src/atlas.rs:74` into `crates/ruster-render/src/lib.rs`.
- [ ] In render-gles `atlas.rs`: `use ruster_render::FontFamily; pub use ruster_render::FontFamily;` at the module top (re-export keeps `ruster_render_gles::atlas::FontFamily` paths compiling) and replace `FontFamily::attrs(self)` (atlas.rs:82) with `pub fn family_attrs(family: FontFamily) -> Attrs<'static>`; update `Buffer::set_text` call sites that used `.attrs()`.
- [ ] Move `ChromeBatch`/`BatchMark`/`mark`/`translate_since` from `chrome.rs:296-331` into `ruster-render-gles/src/geometry.rs` (after `GlyphQuad`), keeping `#[derive(Debug, Default)]` / `#[derive(Debug, Clone, Copy)]`.
- [ ] Delete them from chrome.rs; import `use ruster_render_gles::geometry::{rect_verts, rounded_rect_verts, BatchMark, ChromeBatch, GlyphQuad, Vertex};` (chrome.rs:16 already imports the first two + GlyphQuad + Vertex).
- [ ] **TDD pass:** `cargo build` + `cargo test -p ruster-render-gles -p ruster-compositor` green.
- Commit: `refactor: move FontFamily and ChromeBatch so the elements crate can name them`

### Task 2 — `ruster-render-elements`: builder, style, id

Land the crate skeleton: `Elem` tree, `div()`/`text()` builders, `Styled` builder, `Style`, `ElementKey`.

- [ ] Add workspace member `crates/ruster-render-elements`; create `Cargo.toml` (deps `ruster-render`, `taffy = "0.13"`).
- [ ] `id.rs`: `#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)] pub struct ElementKey(pub Vec<String>);` + `child(&self, seg: &str) -> ElementKey` (push clone), `last()`.
- [ ] `element.rs`: `pub struct Elem { pub style: Style, pub kind: ElemKind }`; `pub enum ElemKind { Container { children: Vec<Elem> }, Text { line: StyledLine } }`; `pub fn div() -> Elem`; `pub fn text(impl Into<StyledLine>) -> Elem`; `impl Elem { pub fn children(&mut self, children: Vec<Elem>) -> &mut Self }` (panics on `Text` leaf — a text cannot have children).
- [ ] `style.rs`: `pub struct Style { pub taffy: taffy::Style, pub bg: Option<Color>, pub fg: Option<Color>, pub radius: f32, pub border_width: f32, pub border_color: Color, pub font_size: f32, pub font_family: FontFamily, pub bold: bool, pub id: Option<String> }` + `impl Default`.
- [ ] `Styled` trait with `#[allow(clippy::should_implement_trait)]` on `fn style` (name clash with field `style`): `flex_col() flex_row() flex_wrap() flex_grow(f32) flex_shrink(f32) size(w,h) w() h() min_w_0() gap(f32) padding() padding_x() padding_y() padding_left/right/top/bottom(f32) justify_center() items_center() absolute() position(x,y) bg(Color) fg(Color) radius(f32) border_1() border_color(Color) font_size(f32) font_family(FontFamily) bold() id(&str) style(fn(&mut taffy::Style))`.
  - `size(w,h)`/`w`/`h`/`gap`/`padding*`/`min_w_0`/`position` take `f32` and map to `taffy::Style` via `taffy::prelude::{length, auto, zero, LengthPercentage, LengthPercentageAuto, Dimension}`; `absolute()` sets `taffy.position = Position::Absolute` and the `.position(x,y)` sets `inset.left/top`.
  - `flex_*`/`justify_center`/`items_center` map to the taffy enum fields; `flex_grow`/`flex_shrink` to the f32 fields.
  - Font/visual fields go on the Style struct's own fields, not taffy.
- [ ] `element.rs`: `impl Styled for Elem` delegating to `self.style`; make `Styled` callable in a chain ending before `children` (return `&mut Self`).
- [ ] `lib.rs`: `pub use element::{div, text, Elem, ElemKind, Styled}; pub use style::Style; pub use id::ElementKey; pub use layout::{layout, BoxNode, LayoutScene, PxRect, TextMeasurer, TextNode};`
- [ ] **Tests (TDD):** builder → taffy mapping unit tests: `size`/`position` land on `Style.taffy`; `id()` lands on `Style.id`; a `text` leaf cannot take `children` (panics); `ElementKey` root is empty and `child()` appends. Reordering-the-tree-remaps-keys test: two siblings with `.id("a")`/`.id("b")` produce different keys when swapped.
- [ ] **Verify:** `cargo test -p ruster-render-elements` green; `cargo clippy --all-targets -- -D warnings` clean.
- Commit: `feat: ruster-render-elements builder, style and element ids`

### Task 3 — `layout()`: taffy build + pure `LayoutScene`

The heart of the crate: walk the tree, build a `TaffyTree<NodeCtx>` in lockstep with a mirror `SceneNode` tree, measure text leaves through the injected `TextMeasurer`, run taffy, read back rects, emit painter's-order `LayoutScene`.

- [ ] **TDD fail:** a golden test (below) fails because `layout` does not exist.
- [ ] `layout.rs`:
  ```rust
  pub trait TextMeasurer {
      fn measure(&mut self, line: &StyledLine, font_size: f32, family: FontFamily) -> (f32, f32);
  }
  #[derive(Debug, Clone, Copy, PartialEq, Default)]
  pub struct PxRect { pub x: f32, pub y: f32, pub w: f32, pub h: f32 }
  #[derive(Debug, Clone, PartialEq)]
  pub struct BoxNode { pub rect: PxRect, pub radius: f32, pub fill: (f32,f32,f32,f32), pub border_width: f32, pub border_color: (f32,f32,f32,f32), pub key: ElementKey }
  #[derive(Debug, Clone, PartialEq)]
  pub struct TextNode { pub rect: PxRect, pub line: StyledLine, pub font_size: f32, pub family: FontFamily, pub fg: (f32,f32,f32,f32), pub bold: bool, pub key: ElementKey }
  #[derive(Debug, Clone, PartialEq, Default)]
  pub struct LayoutScene { pub boxes: Vec<BoxNode>, pub texts: Vec<TextNode> }
  ```
- [ ] Internal `NodeCtx { key: ElementKey, kind: NodeKind, parent_key: ElementKey }` and `SceneNode { node: NodeId, ctx: NodeCtx, style: Style }` mirror; build walks `Elem`:
  - `RootStyle`: `taffy::Style { size: area.size, position: Relative }`; the root gets `ElementKey(vec![])`.
  - Containers → `new_with_children`; Text leaves → `new_leaf_with_context` (context carries `StyledLine`, `font_size`, `family`).
  - Key derivation per child: `key = parent_key.child(child.id.unwrap_or(&index.to_string()))`; duplicate `.id()` under one parent → `debug_assert!` in debug, later wins in release (GPUI contract).
  - `.absolute()` nodes are plain children of the root/container (taffy `Position::Absolute`); their `position(x,y)` set `inset`.
- [ ] `compute_layout_with_measure` against `area.size` with a closure capturing `&mut measurer`: if `Option<&mut NodeCtx>` says the node is a text leaf, call `measurer.measure(line, font_size, family)` and return `Size { width, height }`, else `Size::ZERO`.
- [ ] Read-back walk (mirror tree, accumulate parent origin from `tree.layout(node)`):
  - Emit container `BoxNode` (bg present or border_width > 0) at its rect **before** children (painter's order), with `fill = bg.into()` (ruster-render `impl From<Color> for (f32,f32,f32,f32)` exists; `Color::Default` → white, matching current tessellation), `radius`, border.
  - Emit each `Text` leaf's `TextNode` at its computed rect with its effective `fg` (explicit, else inherited container fg, else `Color::Default`), `font_size`, `family`, `bold`, `key`.
  - `fg` inheritance is threaded as the build walk descends (a `parent_fg: Option<Color>`).
- [ ] **Tests (TDD):** a mock `TextMeasurer` (returns `(len * 8.0, font_size + 4.0)`) + golden layout tests reproducing each widget's hardcoded numbers:
  - statusline: bar at `(0, h-40, w, 40)`, mode box `(0, y, 64, 40)`, "N" at `(24, pad)`, ws at `(76, y+pad)`, indicator after `ws_w+20`, title after `ind_w+20` (`pad = (40-16)/2 = 12`);
  - which-key: panel at `(12,12)` with `w`/`h` from column chunking arithmetic; a column's width = max over rows of `key_w + 8 + desc_w`;
  - hover: pre-measured `w = (text_w + 16).min(output_w - 8)`, `h = 16 + lines*18`, below/above flip, x clamp;
  - a pane: `body.x = FRAME_PAD + gutter_cols*cell_w`, `body.y = FRAME_BAR_H + FRAME_PAD`, first run glyph origin;
  - absolute vs in-flow ordering: a pane drawn at an absolute `(x,y)` origin vs the same content in-flow yields identical rects.
  - Reorder-remaps-keys test (from Task 2) now at the layout level.
- [ ] **Verify:** `cargo test -p ruster-render-elements` green; clippy clean.
- Commit: `feat: taffy layout to a pure painter's-order LayoutScene`

### Task 4 — render-gles: `GlesTextMeasurer` + `scene_to_chrome_batch`

Tessellate `LayoutScene` into the existing `ChromeBatch` (now carrying `glyph_keys`), and provide the measurer.

- [ ] **TDD fail:** a tessellation test (below) fails because `scene_to_chrome_batch` does not exist.
- [ ] `tessellate.rs`:
  - `pub struct GlesTextMeasurer;` implementing `TextMeasurer` via `layout_text_in(&line.text, font_size as u32, None, family)` → `(width_px, font_size as f32 + 4.0)` (matching `cell_metrics`'s height rule; width equals what the atlas will tessellate).
  - `pub fn scene_to_chrome_batch(scene: &LayoutScene, atlas: &mut Atlas) -> ChromeBatch`:
    - `BoxNode`: `rect_verts` or `rounded_rect_verts` (radius); border as 4 thin rects (top/bottom/left/right, `border_width`) when `border_width > 0`; push verts in `BoxNode` emission order.
    - `TextNode`: `layout_text_in(&line.text, font_size as u32, None, family)`, then per glyph `atlas.glyph_in(font_size as u32, rgb8(fg), c, family)` (skip empty) pushing `GlyphQuad { x: rect.x + gx + g.x, y: rect.y + g.y, w, h, u0..v1 }` and one `ElementKey` clone into `glyph_keys` per glyph.
    - `rgb8` (chrome.rs:926) moves here (or into geometry.rs); chrome.rs's `text_in` is deleted at flip so no duplicate.
- [ ] Add `ChromeBatch.glyph_keys: Vec<ElementKey>` (Task 1 moved ChromeBatch — extend it here; `Default` still derives; `translate_since` also shifts keys? No — keys are per-glyph identity, not position, so `translate_since` leaves them; it is deleted at flip anyway).
- [ ] **Tests (TDD):** box→vertex counts (rect = 6 verts, rounded ≥ 6, radius clamped like `rounded_rect_verts`), border = 4 rects, text→glyph quads + atlas UVs (a known string at a known position yields the same quads as the old `text_in` did), `glyph_keys` length == glyph count, and **id stability**: two `scene_to_chrome_batch` calls with unchanged geometry produce equal `glyph_keys`.
- [ ] **Verify:** `cargo test -p ruster-render-gles` green (incl. existing geometry/atlas tests); clippy clean.
- Commit: `feat: tessellate LayoutScene to ChromeBatch with keyed glyph ids`

### Task 5 — compositor `scene.rs`: builders + parity test

Build the per-widget `*_elem` functions, `chrome_scene`, and the **parity gate** comparing the new scene against the old `draw_*` output. Nothing renders differently yet — the old path still runs.

- [ ] **TDD fail:** a parity test (below) fails because the builders do not exist.
- [ ] `scene.rs` (imports `ruster_render_elements::{div, text, Elem, Styled, FontFamily, PxRect}`, render-gles `GlesTextMeasurer`, chrome helpers):
  - `statusline_elem(w, h, workspace, title, tree: TreeStatus, theme) -> Elem`: bar absolute at `(0, h - bar_h)` sized `(w, bar_h)` bg `statusline_bg`; mode box absolute `(0, 0, 64, bar_h)` bg `accent` with "N" text at `(24, pad)` 16px `accent_fg`; ws/indicator/title as flex-row children with `gap(20)`, `padding_left(76)`, `padding_top(pad)` — **no** `items_center` (text top must equal `y + pad`); indicator = `tree.indicator()`, title fallback `"(no client)"`.
  - `window_borders_elem(windows, focus, scale, theme) -> Elem`: for each `(id, rect)`, 4 absolute boxes (top/bottom/left/right) at `(x,y) = (rect.x*s, rect.y*s)` sized `(w, width)` / `(width, h)` with `width = (BORDER_WIDTH * s).max(1.0)`; skip `w <= 0 || h <= 0`; focused color `border_focused` else `border_unfocused`.
  - `minibuffer_elem(output_w, output_h, line, sigil_len, theme) -> Elem`: bar_h = `chrome_height(output_h)`, y = `output_h - 2*bar_h`, bg `cmdline_bg`, font = `(bar_h * 0.5) as u32`, sigil in `cmdline_accent` at `(10, y + h*0.25)`, rest in `cmdline_fg` (x advanced by sigil width — use a flex-row with `gap(0)` `padding_left(10)` `padding_top(0.25*h)` or absolute x computed from measured sigil width).
  - `whichkey_elem(output_w, output_h, view: &WhichKeyView, theme, measurer) -> Elem`: keep the exact `per_col`/`columns`/`rows_drawn`/`w`/`h` arithmetic from `draw_whichkey`; panel absolute `(12,12)` `bg whichkey_bg` `radius 6` `.max_w(output_w - 24)`; title row (if any) in `whichkey_key`; columns flex-row `gap(COL_GAP)`; each column flex-col of `(key, desc)` rows flex-row `gap(GAP)`; key in `whichkey_key`, desc in `whichkey_fg`; rows `h(ROW_H)`.
  - `hover_elem(output_w, output_h, anchor: HoverAnchor, lines: &[String], theme, measurer) -> Elem`: pre-measure `text_w` via measurer, `w = (text_w + PAD*2).min(output_w - 8)`, `h = PAD*2 + lines.len()*ROW_H`, below/above flip, `x` clamp (exact `draw_hover` math); panel absolute `(x,y)` `radius 6` `bg whichkey_bg`; lines flex-col `gap(0)` each `h(ROW_H)` text in `whichkey_fg`.
  - `pane_elem(w, h, lines: &[StyledLine], first_line, severities, title, theme) -> Elem`: rounded bg `radius 4`; title bar `(0,0,w,FRAME_BAR_H)` bg `accent` with title text at `(FRAME_PAD, (28-16)/2)` 16px `accent_fg`; body from `FrameBody::new(first_line, lines.len())`: per visible row (`rows = ((h - 28 - 8)/cell_h).max(0) as usize`) a sign text (severity_sign, Mono, at `(FRAME_PAD, gy)`), line number (right-aligned Mono at `numbers_x = FRAME_PAD + SIGN_COLS*cell_w`), and one `text()` per `runs(line)` run at `(body.x + run.column*cell_w, gy)` Mono, colored by run or inherited `fg`. **Position per cell, never by chaining advances** (the grid is the authority — same rule as today).
  - `compose(pieces: Vec<Elem>, theme, w, h) -> Elem`: wraps the given pieces (already absolutely positioned) in a root `div()` sized `(w, h)`; the ordering test drives this directly.
  - `chrome_scene(frame, theme, measurer) -> (Elem, Option<Elem>)`: builds `window_borders_elem`, `statusline_elem`, `minibuffer_elem` (only when `frame.minibuffer`), `whichkey_elem` (only when `frame.whichkey`), then per-pane `pane_elem` at absolute `(rect.x*scale, rect.y*scale)` (pane w/h truncated `as i32` exactly like `draw_editor_frame` today), computing hover anchor via the preserved `FrameBody::cell_origin` + `chrome_scale` math from `render.rs:270-280`; returns `(compose(...), hover_elem(...))`.
  - Import `TreeStatus`, `WhichKeyView`, `HoverAnchor` (HoverAnchor moves here from chrome.rs or stays — the plan keeps it in chrome.rs and imports it).
- [ ] **Parity test (in `render.rs` tests module, the migration gate):** a shared `Chrome` instance (same atlas, `theme()` helper at chrome.rs:1153) drives both paths for a representative set of states, comparing rects and glyphs **exactly** (f32 equality; both paths share `layout_text_in` and the atlas, so UVs and glyph origins must be identical):
  - statusline with a long focused title + `TreeStatus { layout: Some(Horizontal), .. }`;
  - which-key pending (`view.rows` spanning two columns);
  - hover up (3 lines) below the caret;
  - one pane with diagnostics (`severities`), a syntax-highlighted `StyledLine` (multi-span), and a wide title;
  - window borders focused + unfocused at a fractional scale.
  - Old path: `chrome.draw_statusline(...)`, `draw_window_borders(...)`, `draw_whichkey(...)`, `draw_editor_frame(...)`, `draw_hover(...)` into `ChromeBatch`/`OverlayBatch`. New path: `statusline_elem`/etc → `layout(area, elem, measurer)` → `scene_to_chrome_batch(scene, atlas)`. Assert `batch.verts == new_verts` and `batch.glyphs == new_glyphs` (with `glyph_keys` compared separately where applicable). **This test is deleted at the flip (Task 6)**.
  - **Ordering test:** drives `compose` with synthetic pieces carrying `.id("borders")`, `.id("statusline")`, `.id("minibuffer")`, `.id("whichkey")`, `.id("pane")`, and asserts the laid-out scene's boxes appear in the old painter's order **borders → statusline → minibuffer → whichkey → panes**, with hover in the returned overlay element (separate `(Elem, Option<Elem>)`).
- [ ] **Verify:** `cargo test -p ruster-compositor` green (old + new paths coexist); clippy clean.
- Commit: `feat: declarative chrome scene with parity-tested geometry`

### Task 6 — Flip `render.rs` to the scene path; delete the old geometry

`collect_render_elements` switches to `build → layout → scene_to_chrome_batch`, and every superseded hand-built path is deleted.

- [ ] **TDD fail:** no test fails yet — this is a mechanical flip; the **parity test from Task 5 is deleted in this task** and the remaining tests must still pass.
- [ ] `render.rs::collect_render_elements` (lines 179-345): replace the `draw_*` block with:
  ```rust
  let (base, overlay) = scene::chrome_scene(scene, &chrome.theme, &mut GlesTextMeasurer);
  let area = PxRect { x: 0.0, y: 0.0, w: size.w as f32, h: size.h as f32 };
  let base_scene = layout(area, &base, &mut GlesTextMeasurer);
  let base_batch = scene_to_chrome_batch(&base_scene, &mut chrome.atlas);
  let overlay_batch = overlay.map(|o| { let s = layout(area, &o, &mut GlesTextMeasurer); scene_to_chrome_batch(&s, &mut chrome.atlas) });
  ```
  then emit exactly as today: overlay glyphs → overlay solids `.rev()` → base glyphs → base solids `.rev()` (render.rs:316-345). Add a `Chrome::theme()` accessor (`&self` — the field `theme` stays private).
- [ ] `glyph_elements` (render.rs:444-493): zip `glyphs` with `glyph_keys`; group consecutive equal keys; `chrome.element_ids(&key, n)` per group instead of `chrome.glyph_id(index)`.
- [ ] `Chrome`: `glyph_ids: Vec<Id>` (chrome.rs:352, `glyph_id` at 404-409) → `id_map: HashMap<ElementKey, Vec<Id>>` + `pub fn element_ids(&mut self, key: &ElementKey, len: usize) -> Vec<Id>` (grow with `Id::new()` on first sight, clone out `len`). Add `use ruster_render_elements::ElementKey;` (compositor gains the dep — it already will via render-gles re-export, but import directly).
- [ ] `bench_glyphs` (chrome.rs:696-785): keep, but push one unique `ElementKey(vec![format!("bench:{i}")])` per glyph.
- [ ] Delete from chrome.rs: `ChromeBatch`, `BatchMark`, `mark`, `translate_since`, `OverlayBatch`, all `draw_*` methods (`draw_statusline`, `draw_editor_frame`, `draw_window_borders`, `draw_minibuffer`, `draw_hover`, `draw_whichkey`), `text`/`text_in`, `rgb8`. Keep: `FrameBody`, `runs`, `severity_sign`, `gutter_width`, `gutter_cols`, `SIGN_COLS`, `FRAME_BAR_H`, `FRAME_PAD`, `PANE_FONT_PX`, `HoverAnchor`, `Chrome::new`, `atlas_texture`, `cursor_element`, `solid_elements_from_verts`, the `chrome_height` call in render.rs.
- [ ] Delete the Task 5 parity test (its purpose is served); keep the **ordering test** and the existing `statusline_emits_quads` / `whichkey_panel_renders_its_view` / hover / cell-origin tests — re-anchor any that asserted `ChromeBatch`/`mark` internals onto the scene path.
- [ ] **Verify:** `cargo build` (warnings clean), `cargo test -p ruster-compositor -p ruster-render-gles -p ruster-render-elements`, `cargo clippy --all-targets -- -D warnings` — all green; `rg "draw_statusline|draw_editor_frame|draw_whichkey|draw_hover|translate_since|OverlayBatch|glyph_ids|glyph_id"` in the compositor returns nothing.
- Commit: `feat: render chrome through the declarative scene; delete hand-built geometry`

### Task 7 — Verification & docs

Confirm the flip against the spec's Definition of Done gates and record results.

- [ ] `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings` all clean across the workspace.
- [ ] **Render budget:** run the compositor on the winit backend with `RUSTER_BENCH_GLYPHS=10000` (e.g. `cargo run -p ruster-bin -- compositor --winit` or the documented invocation in `docs/compositor.md`), capture the `"frame time"` info log (`winit_main.rs:83`), confirm ≤ today's ~4.7ms, record the number.
- [ ] **Visual rows:** capture the `docs/compositor.md` rows that render chrome — statusline, which-key, hover, window borders, editor panes — and confirm they match the recorded screenshots / prior verification.
- [ ] Update `docs/AGENTS.md` workspace crate list (+ `ruster-render-elements`).
- [ ] Update `docs/compositor.md` (new `frame time` number; note the declarative scene; the visual rows table stays).
- [ ] **Self-review:** re-read this plan's checklist; confirm every box is ticked; confirm no `draw_*` geometry remains; confirm `docs/config-reference.md`, `docs/lua-api.md`, `docs/keybindings.md` needed no edits (no config/Lua/keybinding surface changed — if one did, that's a spec violation to fix).
- Commit: `docs: verify chrome scene render path and record frame time`

## Self-review checklist

- **Spec fidelity:** every spec requirement is implemented, with each deviation above justified by a cited reality (line/commit) and no silent scope changes.
- **Parity before flip:** the migration gate (Task 5 parity test) was green with both paths emitting, then deleted only at the flip; the flip changed `collect_render_elements` and nothing the tests can't see.
- **No dead code:** after Task 6, `rg` shows zero `draw_*`, `mark`, `translate_since`, `OverlayBatch`, `glyph_ids`, `glyph_id` in the compositor.
- **Layering:** `ruster-render-elements` depends only on `ruster-render` + taffy; render-gles depends on elements; compositor depends on both — no cycle, no smithay/GL/cosmic-text in the portable crate.
- **Budget:** `RUSTER_BENCH_GLYPHS=10000` frame time recorded and within budget; no caching in v1 (taffy rebuilds each frame by design).
- **Keyed ids:** `element_ids(&key, n)` stable across frames with unchanged geometry (Task 4 test); reordering remaps keys (Task 2/3 test) exactly as GPUI's documented footgun.
- **Docs:** AGENTS.md + compositor.md updated; config/lua/keybinding docs untouched (no surface change).
