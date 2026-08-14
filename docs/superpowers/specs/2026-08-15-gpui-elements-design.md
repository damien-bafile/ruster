# GPUI Elements — Design

**Date:** 2026-08-15
**Status:** Draft (post-brainstorming, pre-plan)
**Owners:** unassigned — open a tracking issue before implementation starts

## Purpose

Bring the three architectural ideas that make GPUI productive — a declarative
element tree with a Tailwind-style styling builder, a battle-tested flexbox/grid
layout engine, and stable element IDs for retained state — into `ruster`'s
compositor chrome, replacing the hand-built quad geometry in
`crates/ruster-compositor/src/chrome.rs` (~1,600 lines) with a small, portable
element layer that compiles down to the same `ChromeBatch` output the renderer
already emits.

This is **sub-project 1 of 4** in the "bring GPUI" programme:

1. **Declarative element + styling layer** (this spec) — `ruster-render-elements`.
2. Entity/View context + notify/observe — later cycle.
3. Async spawn API for Lua — later cycle.
4. Focus model / accessibility — later cycle.

Sub-projects 2–4 are out of scope here; the element-ID machinery is shaped so
they can hang off it later, but none of their behaviour ships in this spec.

## Goals

1. A new portable crate `ruster-render-elements` with a `div()`/`text()` builder
   API in GPUI's Tailwind idiom, laying out through **taffy** — the same flexbox
   engine GPUI, Bevy, Dioxus and Lapce render through (MIT, no GPL exposure).
2. One declarative scene per frame: `chrome_scene(frame) -> Elem`, laid out and
   tessellated once into the existing `ChromeBatch`, deleting the per-widget
   `draw_*` geometry methods, `mark()`/`translate_since`, and the chrome's
   separate physical/logical coordinate juggling.
3. Stable element IDs (`GlobalElementId`-style path from the root) driving
   damage tracking, replacing position-keyed `glyph_ids` so chrome reordering no
   longer remaps render-element ids and damages the wrong regions.
4. Byte-identical chrome pixels after the swap (statusline, which-key, hover,
   window borders, editor panes) — verified against the existing
   `docs/compositor.md` visual rows and a parity test.
5. No regression on the render budget: `RUSTER_BENCH_GLYPHS` frame time must not
   exceed today's ~4.7ms at 10,000 extra glyphs.

## Non-goals

- Sub-projects 2–4 (entity/context, spawn API, focus/a11y) — their own later
  cycles; this spec only guarantees the ID/store hooks they will need.
- Hover/focus semantics in v1. `.id()` plumbs identity and a retained state
  store; nothing reads the store yet.
- A raylib consumer of the element layer. The crate is shaped for it (portable,
  injected text measurement), but the first and only consumer is the compositor.
- CSS Grid usage in v1. taffy provides it; nothing needs it yet.
- Font loading or glyph rasterization changes — the atlas, shaping and the
  `FontFamily` grid are untouched (except a relocation, see below).
- Any change to `FrameBody` hit-testing semantics, the syntax-highlighting
  `runs()` logic, or the diagnostics gutter math.

## Background — what GPUI actually is, and what we take

GPUI does **not** implement flexbox itself. It delegates layout to
[taffy](https://github.com/DioxusLabs/taffy) and layers on top of it:

- a **`Styled` builder** — Tailwind-inspired method names (`.flex_col()`,
  `.gap_3()`, `.bg()`, `.border_1()`) over a `Style` struct, with a `.style(fn)`
  escape hatch down to the raw `taffy::Style`;
- a **retained/immediate hybrid** — the element tree is rebuilt every frame
  (immediate), but `div().id("x")` keeps focus/scroll/hover state across frames
  (retained); the stable identity is the path of ids from the root
  (`GlobalElementId`);
- a **paint pass** that walks the laid-out tree and emits primitives.

What we take: taffy as a dependency (the engine itself), the `Styled` builder
idiom, the element-ID model, and the `TextMeasurer`-injected text separation.
What we do not take: GPUI's entity system, executor, windowing or renderer —
those are sub-projects 2–3 or deliberately out of scope.

## Architecture

### Layering

```
ruster-render-elements            NEW crate; deps: ruster-render, taffy
  ├─ Styled builder  (div()/text(), .flex_col().gap_3().bg().border_1().id())
  ├─ Elem tree       (Container | Text), rebuilt every frame
  ├─ TextMeasurer    trait (injected — crate stays free of cosmic-text/GL)
  └─ layout(area, root, measure) -> LayoutScene     pure, f32 px, painter's order
        │
        ▼
ruster-render-gles                gets scene_to_chrome_batch(scene, atlas)
  └─ boxes  → rect_verts / rounded_rect_verts
     texts  → atlas layout_text → GlyphQuad           (unchanged shaping)
        │
        ▼
ruster-compositor
  └─ chrome_scene(frame) builds ONE Elem tree per frame
     render.rs: build → layout → scene_to_chrome_batch → existing emission
```

- `ruster-render-elements` depends only on `ruster-render` (for `Theme`,
  `StyledLine`, `Color`) and `taffy`. No smithay, no GL, no cosmic-text — the
  crate compiles on every platform, exactly like `ruster-render` does today.
- The tessellator lives in `ruster-render-gles` because the atlas (cosmic-text)
  is already Linux-gated there and the glyph/vertex geometry types live there.

### Type moves required

Two small relocations so the portable crate can name the types:

- **`FontFamily`** (`Ui`/`Mono`): move from `ruster-render-gles/src/atlas.rs`
  into `ruster-render` (it is a plain enum; both backends and the elements crate
  need it). Re-export from render-gles for source compatibility.
- **`ChromeBatch`** (+ its `Vertex`/`GlyphQuad`/`BatchMark` companions): move
  from `ruster-compositor/src/chrome.rs` into `ruster-render-gles`
  (`geometry.rs` already owns `Vertex` and `GlyphQuad`). The compositor imports
  it; nothing else changes.

### The element model

```rust
pub struct Elem {
    style: Style,               // our visual + font fields, plus taffy Style
    id: Option<String>,         // stable identity (see ElementKey)
    kind: ElemKind,
}

pub enum ElemKind {
    Container { children: Vec<Elem> },
    Text { line: StyledLine },  // one run-per-span text line, like today
}
```

Builders: `div()` (container), `text(s: impl Into<StyledLine>)` (text leaf).

`Styled` trait (method names follow GPUI's idiom; values are `f32` pixels or
`ruster_render::Color`):

```rust
// layout
.flex()  .flex_col()  .flex_row()  .flex_wrap()
.flex_grow(f32)  .flex_shrink(f32)  .flex_basis(Length)
.size(w, h)  .w(Length)  .h(Length)  .min_w_0()      // truncation escape hatch
.gap(f32)  .padding(f32)  .padding_x(f32)  .padding_y(f32)
.justify_center()  .items_center()  .absolute()  .position(x, y)
// appearance
.bg(Color)  .fg(Color)  .radius(f32)  .border_1()  .border_color(Color)
// text
.font_size(f32)  .font_family(FontFamily)  .bold()
// identity + escape hatch
.id("name")  .style(fn(&mut taffy::Style))
```

`Style` holds: the flexbox/sizing fields (mapped onto `taffy::Style`), plus the
chrome visuals taffy does not model — `bg`, `fg`, `radius`, `border_width`,
`border_color`, `font_size`, `font_family`, `bold`. `fg`/`bold` belong to the
element and are consumed when its `Text`/children tessellate; a container's
`fg` is inherited by descendant text with no explicit `fg`, matching how the
pane frame colours its rows today.

### Text measurement

`TextMeasurer` is the one injected dependency that keeps the crate portable:

```rust
pub trait TextMeasurer {
    /// Width and height in physical pixels, matching how chrome is measured
    /// and drawn today.
    fn measure(&mut self, line: &StyledLine, font_size: f32, family: FontFamily) -> (f32, f32);
}
```

- `ruster-render-elements::layout` calls it only for `Text` leaves, feeding the
  result to taffy as the leaf's intrinsic size.
- render-gles implements it over the existing cosmic-text `FontSystem` (the same
  shaping stack the atlas uses), as `atlas.measure(...)` or a `GlesTextMeasurer`
  adapter — the plan pins the exact home; the point is one `FontSystem`, not two.
- A future raylib backend supplies its own measurer. This mirrors GPUI's
  `TextSystem` separation.

### Layout

`layout(area: Rect, root: &Elem, measure: &mut impl TextMeasurer) -> LayoutScene`

1. Walk the tree, building a `taffy::TaffyTree` in lockstep: containers become
   flex/grid nodes, `Text` leaves become measured leaves. `.absolute()` +
   `.position(x, y)` map to taffy's `Position::Absolute`, which is how panes,
   the hover panel and which-key sit at their geometry rects on top of the
   tiling scene.
2. `compute_layout` against the output's pixel area.
3. Read back each node's computed rect, walk the tree in **painter's order**
   (container background → children → the container's own text) and emit:

```rust
pub struct LayoutScene {
    pub boxes: Vec<BoxNode>,   // rect, radius, fill, border, key
    pub texts: Vec<TextNode>,  // rect, StyledLine, font_size, family, fg, bold, key
}
pub struct ElementKey(pub Vec<String>);   // the id path from the root
```

- Emission order **is** painter's order (back to front). The existing
  `solid_elements_from_verts(...).rev()` in render.rs keeps its meaning; the
  scene simply declares the order instead of the batch structure implying it.
- `LayoutScene` is pure data — unit-testable with no GPU, matching the
  compositor's existing test discipline.

### Retained IDs and damage (v1 use only)

- `.id("name")` pushes onto the running path; the full path from the root is the
  `ElementKey`. Duplicate ids under one parent are a debug assertion, exactly
  GPUI's contract — and the same footgun: **reordering the tree remaps keys**
  (their `GlobalElementId` warning). A test pins that reordering remaps state.
- v1 consumer: damage. `Chrome` keeps `HashMap<ElementKey, Vec<Id>>` instead of
  the position-keyed `glyph_ids: Vec<Id>`; the tessellator asks for the element's
  id vec, allocating on first sight, so a Text that does not move keeps its
  render-element ids and reports no damage. Boxes already use stable
  `Id::new()` per element via smithay's element list.
- The `ElementState` store (per-`ElementKey`) is created empty in v1 — the hook
  sub-project 4 (hover/focus) will populate.

### One scene per frame (compositor)

New `chrome_scene(frame: &FrameInput) -> Elem` in the compositor builds one tree:

- **root** sized to the output area;
- **statusline** — a flex row of segments (mode, title, git, position), the
  segments that today come from `draw_statusline`;
- **window borders** — per-tile border boxes at each `geometry` rect, with the
  focused tile's border in `border_focused`;
- **which-key** — `.absolute()` panel of `div().id("whichkey")` rows, only when
  `scene.whichkey` is `Some`;
- **minibuffer** — the `:`/message line above the statusline when open;
- **hover** — `.absolute()` panel anchored through the existing
  `FrameBody::cell_origin` math (the anchor logic is preserved; only the drawing
  changes), emitted after the panes so it paints above them;
- **editor panes** — one `.absolute()` frame per `(id, rect)` in `geometry`:
  title bar, sign column, gutter, and one `Text` element per visible line (the
  `runs()`/`syntax_color()`/`severity_sign()` logic and `FrameBody` are
  untouched; they now produce the `StyledLine`s the elements carry).

`render.rs::collect_render_elements` shrinks to:

```
root = chrome_scene(scene)
scene = layout(area, &root, &mut measurer)
batch = scene_to_chrome_batch(&scene, &mut chrome.atlas)
elements = glyphs(batch) then reversed solids(batch)   // as today
```

The `mark()`/`translate_since` machinery, the physical-vs-logical scale juggling
per pane, and every `Chrome::draw_*` geometry method are deleted. `FrameBody`,
`runs`, `syntax_color`, `severity_sign`, `gutter_width`, `SIGN_COLS` stay.

**Ordering invariant** (was implicit in the batch, becomes declared): hover above
panes, statusline above panes, borders behind pane text. The scene's emission
order encodes it and a test asserts it.

### Tessellation (render-gles)

`scene_to_chrome_batch(scene, atlas) -> ChromeBatch` (measurement already
happened during `layout`; the atlas supplies both shaping and rasterization):

- `BoxNode` → `rect_verts` or `rounded_rect_verts` (radius), border as
  stroked-edge quads (4 thin rects) when `border_width > 0`;
- `TextNode` → `atlas.layout_text(...)` → `GlyphQuad`s, exactly the path
  `draw_editor_frame`/`draw_statusline` use today, including the existing
  glyph-atlas `dropped_glyphs`/`fill_fraction` behaviour;
- per-element stable ids from the keyed id map (above).

## Data flow

```
each frame (Redraw invitation):
  render.rs::collect_render_elements(scene, chrome, renderer)
    chrome_scene(frame)          → Elem tree (declarative)
    layout(area, root, measurer) → LayoutScene (rects, painter's order)
    scene_to_chrome_batch        → ChromeBatch (verts + glyph quads)
    glyph/solid element emission → smithay elements  (unchanged shape)
    damage_tracker.render_output → swap               (unchanged)
```

State enters via `FrameInput` exactly as today; nothing above it changes.

## File layout

| Path | Change |
|---|---|
| `crates/ruster-render-elements/Cargo.toml` | **NEW** — `ruster-render`, `taffy` |
| `crates/ruster-render-elements/src/{lib,element,style,layout,id}.rs` | **NEW** — builder, `Elem`/`Styled`, `TextMeasurer`, `layout`, `ElementKey` |
| `crates/ruster-render/src/lib.rs` | + `FontFamily` (moved from render-gles); re-export |
| `crates/ruster-render-gles/src/{lib,atlas}.rs` | + `scene_to_chrome_batch`, `measure`; re-export `FontFamily` |
| `crates/ruster-render-gles/src/geometry.rs` | + `ChromeBatch`, `BatchMark` (moved from compositor) |
| `crates/ruster-compositor/src/chrome.rs` | rebuild `Chrome` around the scene path; delete `draw_*` geometry, `mark`/`translate_since`; keep `FrameBody`, `runs`, gutter/sign helpers; `glyph_ids` → keyed id map |
| `crates/ruster-compositor/src/render.rs` | + `chrome_scene`; `collect_render_elements` uses layout + tessellation |
| `Cargo.toml` | workspace member `ruster-render-elements` |
| `docs/superpowers/plans/2026-08-15-gpui-elements.md` | **NEW** — implementation plan (via writing-plans skill) |

No config, no Lua API, no keybinding changes ship with this spec.

## Error handling

- **Atlas exhaustion** — unchanged: `dropped_glyphs`/`fill_fraction` counters,
  rate-limited warning; text simply stops appearing, no crash.
- **taffy measurement of an empty line** — a zero-size leaf (a space-only
  `StyledLine`) still lays out at zero advance, as `layout_text` does today.
- **Text measurer cost** — measurement runs every frame for visible lines only;
  taffy caching is not wired up in v1 (the tree is rebuilt each frame). Gate:
  `RUSTER_BENCH_GLYPHS` budget (Goal 5).
- **Id/key collision** — duplicate `.id()` under one parent is a debug assert;
  in release the later element wins the key, matching GPUI's documented
  behaviour.
- **Pane with no measurable cell** — `FrameBody` keeps its "(0,0)" guard;
  element text for an empty pane is simply absent, as today.

## Testing strategy

1. **Elements crate (pure)** — builder/Style→taffy mapping tests; layout golden
   tests reproducing each current widget's hardcoded numbers: statusline segment
   positions, which-key row geometry, hover panel size, dialog rows, a pane
   frame's gutter/text origin vs `FrameBody`'s answer. Reordering-the-tree-remaps
   keys test (the GPUI footgun, pinned).
2. **render-gles** — `scene_to_chrome_batch`: box→vertex counts, radius
   clamping, border quads, text→glyph quads + atlas UVs, per-element id
   stability across two frames with unchanged geometry.
3. **Parity test (the migration gate)** — for a representative set of states
   (statusline with a long title, which-key pending, hover up, one pane with
   diagnostics), assert the new scene's rects equal the old `draw_*` outputs.
   Green before `render.rs` flips; deleted after.
4. **Compositor** — existing unit tests stay green; new ordering test (hover
   above panes, statusline above panes, borders behind pane text) asserts scene
   emission order.
5. **Visual acceptance** — the `docs/compositor.md` rows for statusline,
   which-key, hover, borders, pane glyphs re-verified by screenshot after the
   flip; `RUSTER_BENCH_GLYPHS` re-measured and recorded against the 10k-glyph
   budget.

## Migration & rollout

1. Add the workspace member and crate; land the pure builder/layout with tests.
2. Move `FontFamily` and `ChromeBatch`; update the few import sites.
3. Implement the measurer + `scene_to_chrome_batch` in render-gles (tests green
   without the compositor touching it).
4. Build `chrome_scene` and the parity test against the *old* draw path; iterate
   until parity passes — chrome pixels are still produced by the old code, so
   nothing renders differently while this lands.
5. Flip `collect_render_elements` to the scene path; delete the dead `draw_*`
   geometry, `mark`/`translate_since`, position-keyed `glyph_ids`.
6. Re-verify the `docs/compositor.md` visual rows + glyph budget; record results.

## Risks & mitigations

| Risk | Mitigation |
|---|---|
| Rewriting the compositor's verified render path regresses pixels | Parity test against the old geometry before the flip; screenshot rows re-verified after; incremental staged rollout |
| taffy tree rebuilt every frame adds measurable cost | taffy lays out ~thousands of nodes in ~330µs; `RUSTER_BENCH_GLYPHS` gates the budget; no caching in v1 |
| `FontFamily`/`ChromeBatch` moves churn import sites | Mechanical, compiler-verified; re-exports keep old paths working |
| Text measurement doubles font work (two `FontSystem`s) | One `FontSystem`, shared by the atlas and the measurer (design point, pinned in the plan) |
| Flex text truncation surprises (GPUI: `min-width: auto`) | `.min_w_0()` escape hatch shipped from day one, mirroring GPUI's own gotcha |
| Duplicate/missing ids remap retained state | Debug assert on siblings sharing an id; reorder-remaps-keys test |
| Grid tempts scope creep | Non-goal: flex only in v1 |

## Open questions

1. **taffy version** — pin the latest published `0.7.x`. GPUI may track an
   internal fork; we use stock taffy and don't need to match.
2. **Measurer home** — `Atlas::measure(...)` method vs a standalone
   `GlesTextMeasurer` in render-gles. Both work; the plan picks one.
3. **Id-key damage map growth** — `HashMap<ElementKey, Vec<Id>>` grows with
   distinct text elements over a session. Bounded by chrome size today; revisit
   if the editor pane ever drives it unbounded (it cannot — ids are per element,
   glyphs per element reuse the vec).

## Definition of done

- [ ] `ruster-render-elements` crate exists (workspace member); builder +
      `TextMeasurer` trait + `layout` → `LayoutScene` are pure and tested.
- [ ] `FontFamily` and `ChromeBatch` relocated; import sites updated; old paths
      re-exported.
- [ ] `scene_to_chrome_batch` + measurer implemented in render-gles; tested.
- [ ] Parity test (new scene vs old geometry) passes for the representative
      states before the flip.
- [ ] `chrome_scene` in render.rs; `collect_render_elements` uses
      layout + tessellation; `draw_*`/`mark`/`translate_since`/position-keyed
      `glyph_ids` deleted.
- [ ] `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`
      clean.
- [ ] `docs/compositor.md` visual rows (statusline, which-key, hover, borders,
      pane glyphs) re-verified; `RUSTER_BENCH_GLYPHS` frame time within budget.
- [ ] Docs updated: workspace crate list in AGENTS.md, plan written to
      `docs/superpowers/plans/`.