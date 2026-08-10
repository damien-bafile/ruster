# Mouse Controls — Design

**Date:** 2026-08-10
**Status:** Draft (post-brainstorming, pre-plan)
**Owners:** unassigned — open a tracking issue before implementation starts

## Purpose

Give `ruster` the mouse surface users expect from a modern editor — across both the raylib GUI and the ratatui/crossterm TUI — without breaking headless tests or TUI escape-hatch behavior. The TUI already enables mouse capture (`crates/ruster-tui/src/app.rs:3052`) and has a single Alt+Left handler; the GUI captures zero mouse state today. This plan closes that gap and unifies behavior so both backends share one hit-test path, one dispatcher, and one Lua extension surface.

## Goals

1. Single `MouseEvent` type used by both backends, with cell coordinates (not pixels) so the editor never branches on backend inside its core handlers.
2. Five hit-test zones — float first (overlays everything), then chrome, gutter, buffer, outside — with one handler per zone.
3. Gestures: click, drag-select (mode-aware), double-click (word), triple-click (line), wheel (scroll), Ctrl+wheel (zoom in GUI only), Shift+wheel (horizontal), Alt+click (add cursor), right-click (context menu), hover (Lua hook after 300ms stillness), cursor change on hover over interactive zones.
4. No regressions in headless tests (`ScriptedRenderer`, `TestRenderer`).
5. Configurable from `ruster.toml` and `ruster.mouse.*` from Lua.
6. Extensible: `ruster.on("mouse_*")` and `ruster.on("hover")` consume-or-pass hooks.

## Non-goals

- Touch / pinch / multi-touch gestures (only desktop mouse).
- Scriptable context-menu **actions** beyond what's already reachable via `ruster.cmd` — the menu is a host, items are registered commands.
- Drag-and-drop file open (out of scope; revisit after Phase 9 cleanup).
- A mouse-driven "command palette" (already covered by `:Fzf` / `:`).

## Architecture

### Layering

```
   ┌──────────────┐                            ┌──────────────┐
   │  ruster-tui  │     crossterm MouseEvent   │  raylib GUI  │
   │  (tokio mp)  │                            │ (poll_input) │
   └──────┬───────┘                            └──────┬───────┘
          │                                           │
          ▼                                           ▼
   ┌──────────────────────────────────────────────────────────┐
   │     ruster-render::MouseEvent  (cell coords + mods)     │
   └─────────────────────────┬────────────────────────────────┘
                             │
                             ▼
   ┌──────────────────────────────────────────────────────────┐
   │   ruster-tui::App::handle_mouse_event (single dispatcher)│
   │   ├─ hit_test(col, row) -> Zone                          │
   │   ├─ zone-specific handler (chrome|gutter|buffer|float)  │
   │   └─ ruster.on("mouse_*", ...) chain → default action    │
   └──────────────────────────────────────────────────────────┘
```

### Backend trait changes

Extend `Renderer` in `crates/ruster-render/src/lib.rs:1020` with two new methods, both default `None` / `(1.0, 1.0)`:

```rust
fn poll_mouse(&mut self) -> Option<crate::mouse::MouseEvent> { None }
fn cell_metrics(&self) -> (f32, f32) { (1.0, 1.0) }
```

`cell_metrics` is the pixels-per-cell of the rendered grid. TUI: `(1.0, 1.0)` (cells are 1 unit). Raylib: `(font_w, font_h)` from the loaded font (already known to the renderer).

The existing `poll_input` is **not changed** — keyboard and mouse remain orthogonal at the trait surface; orthogonality keeps `ScriptedRenderer`'s unit tests untouched.

### New type: `ruster_render::MouseEvent`

Defined in a new file `crates/ruster-render/src/mouse.rs`:

```rust
#[derive(Clone, Copy, Debug)]
pub struct MouseEvent {
    pub col: u16,           // cell column (0-indexed)
    pub row: u16,           // cell row (0-indexed)
    pub kind: MouseKind,    // see below
    pub button: MouseButton,
    pub modifiers: KeyModifiers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseKind {
    Down,
    Up,
    Drag,           // move with a button held
    Move,           // move with no button
    ScrollUp,
    ScrollDown,
    ScrollLeft,
    ScrollRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton { Left, Right, Middle, None }
```

Conversions:

- TUI path: `crossterm::event::MouseEvent` → `MouseEvent`. The crossterm `(column, row)` is 0-indexed cells; we map 1:1. `MouseKind` mapped: `Down(btn)`→`Down`, `Up(btn)`→`Up`, `Drag(btn)`→`Drag`, `Moved`→`Move`, `ScrollUp/Down/Left/Right`→`ScrollUp/Down/Left/Right`. `MouseButton::None` only valid for `Move`/scroll.
- GUI path: `raylib::get_mouse_position()` returns `(f32, f32)`; divide by `cell_metrics()` to get `(col, row)`. Convert at the call site — pixel math never leaks past `poll_mouse`.

### `App::handle_mouse_event` (single dispatcher)

Lives in `crates/ruster-tui/src/app.rs` (where `App` is today). Replaces the current 12-line stub at `app.rs:2917-2928`. Pipeline:

```rust
fn handle_mouse_event(&mut self, ev: ruster_render::MouseEvent) {
    if !self.config.mouse.enabled { return; }

    // Lua consume-or-pass: every mouse_* event hits the bus first.
    if self.lua.dispatch_mouse(ev) { return; }

    match ev.kind {
        MouseKind::Down   => self.on_mouse_down(ev),
        MouseKind::Up     => self.on_mouse_up(ev),
        MouseKind::Drag   => self.on_mouse_drag(ev),
        MouseKind::Move   => self.on_mouse_move(ev),
        MouseKind::ScrollUp | ScrollDown | ScrollLeft | ScrollRight =>
            self.on_mouse_scroll(ev),
    }
}
```

### Hit-test zones

Hit-test runs in `App`, using the existing `last_layout: Vec<WindowLayout>` cache (already at `app.rs:1484-1485`, populated at `app.rs:4465`). New helper:

```rust
enum HitZone { Chrome(ChromeKind), Gutter(wid), Buffer(wid, offset), Float(FloatId), Outside }

fn hit_test(&self, col: u16, row: u16) -> Option<HitZone> { /* see zone rules */ }
```

Zone rules, in priority order:

1. **Float** — overlay (which-key, search picker, hover popup). If a float is at this position, return it.
2. **Chrome** — split lines (─, ┬, ├), tab bar, statusline. Click target depends on chrome kind (`ChromeKind::SplitEdge`, `Tab(n)`, `StatusSection(name)`).
3. **Gutter** — col 0 … sign_width+1 of a window. Click target is the line under cursor.
4. **Buffer** — `TextArea::cell_at` (already at `crates/ruster-render/src/lib.rs:360-412`) returns a `(row, col)`; combined with `scroll_top` this is the existing `buffer_offset_at(col, row)` helper at `app.rs:2936-2959` — reused, not duplicated.
5. **Outside** — anywhere else (empty statusline area, etc.). Default: focus cmdline.

### Per-zone handlers

**Buffer click:**
- Left down: `Action::Move(Motion::To(offset))`.
- Left drag: enter mode-aware selection (see "Drag semantics" below).
- Alt+Left down (no drag): `Action::AddCursor(offset)` (existing behavior).
- Ctrl+Left down: word-wise `Motion::To` (matches `b`/`w` boundaries).
- Double-click (within 400ms, ≤2 cell drift): extend anchor to surrounding word boundary on both ends. `Action::SelectWord`.
- Triple-click: extend anchor to line bounds. `Action::SelectLine`.

**Buffer drag:**
- Neovim mode: on first Down, stash `drag_anchor: Option<usize>` and `drag_kind` (char/line/block based on first-frame motion direction). On Up, convert to Visual — single mouse click was just a move, no-op on Up.
- Emacs mode: first Down sets mark if no mark is set (`Action::SetMark(offset)`); subsequent drag extends region (`Action::Move(Motion::To(offset))` with mark active).

**Gutter click:**
- Click on a line: open `:GitSigns` preview if hunks exist; else `:Diagnostics` if errors. Long-term: per-gutter-sign action. Phase 1 ships the default "preview under cursor in a float."
- Drag in gutter: text-block selection (column 0 of each clicked line through line end). Lua hook: `ruster.on("gutter_click", { line, kind })`.

**Chrome click:**
- Tab: switch to buffer. Middle-click closes tab. Right-click opens tab context menu.
- Statusline section: route by section name (`branch`, `lsp`, `filetype`, `position`, `mode`). Configurable via `ruster.statusline.on_click(section, fn)`. Phase 1 ships only `position` (no-op) and `mode` (no-op) — registry exists, plugins wire behaviors.
- Split edge (─, ┬, ├): drag edge to resize window. Phase 1 ships the hit-test; resizing itself is implemented as a 3-frame drag (Down → Drag → Up) recorded in `App::resize_state`.

**Float click:**
- Click outside any float closes it. Phase 1 implements only that. Per-element clicks (items in picker, items in menu) are routed by the owning float via a new `Float::hit_test(col, row) -> Option<FloatAction>` callback.

**Right-click (any zone):**
- Buffer: open context menu at click position. Menu is a `Float` of kind `Menu`.
- Gutter: open gutter context menu (preview/diagnostics actions).
- Chrome: per-chrome menu (tab close, statusline actions).
- Float: defer to the float.

### Drag semantics (mode-aware)

The `drag_anchor` field on `App` (added in `crates/ruster-tui/src/app.rs` near `cursors`) is the single source of truth for an in-progress drag. Resolution at Down time:

| Editor mode | Anchor action | Drag action | Up action |
|---|---|---|---|
| Neovim Normal | enter Visual at offset | grow visual; line-wise if cross-line, block-wise if Alt held | exit Visual if no movement; else leave Visual active (matches `vw` semantics) |
| Neovim Insert | (treat as Normal; selection isn't idiomatic) | as Normal | as Normal |
| Emacs | `Action::SetMark(offset)` if no mark | `Action::Move(Motion::To(offset))` with mark active | leave region |
| Picker / dialog | consumed by picker | consumed | consumed |

The "alt held" block-wise detection is done at first `Drag` event, not at Down — a user might Alt-tap (cursor add) without intent to block-visual.

### Wheel

```rust
fn on_mouse_scroll(&mut self, ev: MouseEvent) {
    if self.lua.dispatch_mouse(ev) { return; }
    let dir = match ev.kind { ScrollUp => -1, ScrollDown => 1, ScrollLeft => -1, ScrollRight => 1, _ => return, _ };
    let lines = self.config.mouse.wheel_lines;
    let horizontal = matches!(ev.kind, ScrollLeft | ScrollRight);
    if ev.modifiers.contains(KeyModifiers::CONTROL) {
        // GUI only — TUI shows noice toast "Ctrl+wheel zoom: GUI only"
        if self.is_gui { self.zoom_font(dir); }
        return;
    }
    if horizontal {
        if let Some((wid, _)) = self.buffer_zone_under(ev) {
            self.windows.window_mut(wid).scroll_horizontal(dir * lines as i32);
        }
        return;
    }
    if let Some((wid, _)) = self.buffer_zone_under(ev) {
        let w = self.windows.window_mut(wid).unwrap();
        w.scroll_top = w.scroll_top.saturating_add_signed(dir * lines as i32);
    }
}
```

### Double / triple click

State in `App`:

```rust
struct ClickTracker {
    last_down: Option<(Instant, u16, u16, MouseButton)>,  // for double/triple
}
```

On Down:
1. If last_down was < 400ms ago and within 2 cells and same button:
   - 2nd click → fire `Action::SelectWord`.
   - 3rd click → fire `Action::SelectLine`. Reset to single.
2. Else replace `last_down`.

`400` is `config.mouse.double_click_ms` (configurable).

### Hover

The TUI's frame loop already runs at 60Hz. We piggyback:

```rust
struct HoverState {
    last_pos: (u16, u16),
    last_move: Instant,
    emitted_for: Option<(u16, u16)>,
}
```

On every `MouseKind::Move`, update `last_pos`/`last_move`. At every frame tick (existing `tachyonfx` driver), if `now - last_move > hover_delay` and `last_pos != emitted_for`: emit `ruster.on("hover", payload)` and set `emitted_for`. The `hover_delay` is `config.mouse.hover_delay_ms` (default 300).

Hover is a **Lua hook only**. The handler doesn't draw anything; consumers register `:lua require'ruster-hover'.attach()`-style plugins that pop a Float with K-style docs.

### Cursor-shape change

GUI only. The raylib renderer already draws an invisible cursor sprite (found via `ruster-render-gles`); we expose `Renderer::set_cursor(CursorKind)` returning `bool` (true if changed). `App` sets it after `hit_test`:

```rust
fn set_cursor_for_zone(zone: HitZone) {
    let kind = match zone {
        HitZone::Chrome(_) | HitZone::Gutter(_) | HitZone::Chrome::SplitEdge(_) => CursorKind::Resize,
        HitZone::Buffer(_, _) => CursorKind::IBeam,
        HitZone::Float(_) => CursorKind::Default,
        HitZone::Outside => CursorKind::Default,
    };
    self.renderer.set_cursor(kind);   // no-op for TUI
}
```

TUI sees the host terminal's default cursor (`_`/block); no work needed.

### Context menu

Built on the existing `Float` system. New `FloatKind::Menu(Vec<MenuItem>)`. A `MenuItem` is `{ label, keymap_or_cmd, submenu? }`. The picker already has pop-and-confirm logic; the menu reuses it (`MenuState` shares state with `PickerState`).

Registration surface:

```lua
ruster.context_menu.add("buffer", {
    label = "Go to Definition",
    action = function() vim.lsp.buf.definition() end,
})
```

Default buffer menu items: Cut, Copy, Paste, Select All, Copy Filename, Copy Path, Toggle Line Comment (mode-aware), Format Buffer.

Phase 1 right-click **only** opens the menu; menu item implementation reuses existing `ruster.cmd` dispatch.

## Data flow

```
each frame (60Hz):
  TUI:                                   GUI:
    crossterm::event::read() (blocking     raylib polls:
      thread → mpsc)                        GetMousePosition
    for each Event::Mouse:                  IsMouseButtonPressed/Released
      App::handle_mouse_event(ev)            GetMouseWheelMove
    drag/hover state ticks → might emit    drain into event_buffer (extend type)
      Lua hook                              for each mouse event:
                                              App::handle_mouse_event(ev)
  both paths converge on App::handle_mouse_event(ev)
```

## File layout

| Path | Change |
|---|---|
| `crates/ruster-render/src/mouse.rs` | **NEW** — `MouseEvent`, `MouseKind`, `MouseButton`, `CursorKind` |
| `crates/ruster-render/src/lib.rs` | + `poll_mouse`, `cell_metrics`, `set_cursor` (default no-ops); re-export from `mouse` mod |
| `crates/ruster-tui/src/app.rs` | replace the 12-line `handle_mouse_event` stub at `:2917-2928` with a one-line delegation to `crate::mouse::handle_mouse_event`; add `mouse: MouseState` field; nothing else |
| `crates/ruster-tui/src/mouse.rs` | **NEW** — houses the full dispatcher: `MouseState`, `handle_mouse_event`, `hit_test`, `ClickTracker`, `HoverState`, `drag_anchor`, `on_mouse_*`, `MenuRegistry` |
| `crates/ruster-render-raylib/src/lib.rs` | implement `poll_mouse`, `cell_metrics`, `set_cursor`; track click times; emit `MouseEvent` from `drain_raylib` |
| `crates/ruster-render-gles/src/` | (already has cursor sprite) — no change |
| `crates/ruster-core/src/action.rs` | + `Action::SelectWord`, `Action::SelectLine`, `Action::SetMark(offset)`, `Action::ResizeWindow { wid, dy, dx }` |
| `crates/ruster-core/src/cursor.rs` | + `CursorSet::select_word(buffer, anchor)`, `select_line(buffer, anchor)`, `set_region(mark, point)` |
| `crates/ruster-lua/src/lib.rs` | + `dispatch_mouse(ev)`, `dispatch_hover(payload)`, new events table; `ruster.mouse.*` registration; `ruster.context_menu.add(zone, item)` |
| `crates/ruster-lua/src/events.rs` | extend with mouse_* events; `ruster.on("hover", fn)` |
| `src/config/default.toml` | + `[mouse]` section with defaults |
| `docs/config-reference.md` | + `[mouse]` documentation |
| `docs/keybindings.md` | + mouse gesture table |
| `docs/lua-api.md` | + `ruster.on("mouse_*", …)`, `ruster.mouse.*`, `ruster.context_menu` |
| `docs/superpowers/plans/2026-08-10-mouse-controls.md` | **NEW** — implementation plan (via writing-plans skill) |

## Configuration

```toml
[mouse]
enabled = true
hover_delay_ms = 300      # 0 disables hover hooks
double_click_ms = 400     # window for 2nd/3rd click
wheel_lines = 3           # lines per wheel notch
tui_capture = true        # set false to let terminal text-selection win
tui_right_click = "menu"  # or "ignore" or map-to-keymap "<C-Right>"
right_click_menu = true   # false disables entirely

[mouse.bindings]   # optional rebinds (mode = "neovim"|"emacs")
# "<C-Left>" = { action = "MoveWord", mode = "all" }
```

Phase 1 ships `enabled`, `hover_delay_ms`, `double_click_ms`, `wheel_lines`, `tui_capture`, `right_click_menu`. The `[mouse.bindings]` table is phase 2.

## Lua API

```lua
-- Subscribe to mouse events. Returning true consumes the event (no default action).
ruster.on("mouse_down",   function(ev) end)   -- ev = {col, row, button, mods}
ruster.on("mouse_up",     function(ev) end)
ruster.on("mouse_drag",   function(ev) end)
ruster.on("mouse_move",   function(ev) end)
ruster.on("mouse_wheel",  function(ev) end)
ruster.on("hover",        function(payload) end)  -- {col, row, offset, wid, line}

-- Tweak behavior
ruster.mouse.set("hover_delay_ms", 250)
ruster.mouse.get("enabled")  -- boolean

-- Register context menu items by zone
ruster.context_menu.add("buffer", { label = "Format", action = ":Format" })
ruster.context_menu.add("tab",    { label = "Close",  action = ":close" })
ruster.context_menu.add("gutter", { label = "Run tests", action = ":TestNearest" })
```

Event payload schema (stable, versioned if changed):

```lua
MouseEvent {
  col: integer, row: integer,
  button: "left"|"right"|"middle"|"none",
  kind:   "down"|"up"|"drag"|"move"|"scroll_up"|"scroll_down"|"scroll_left"|"scroll_right",
  mods:   { ctrl: bool, alt: bool, shift: bool, meta: bool },
  zone:   "chrome"|"gutter"|"buffer"|"float"|"outside",
  -- present for buffer zone only:
  wid:    integer?, offset: integer?, line: integer?, col_in_line: integer?,
}
```

Zone is computed at the dispatcher and included in the payload so Lua handlers don't have to re-hit-test. `ruster.on("hover")` is a separate event with `{wid, line, col_in_line, offset}`.

## Error handling

- **Headless test path:** `poll_mouse` default `None`; `ScriptedRenderer` records scripts unchanged.
- **GUI panic / hang:** `drain_raylib` runs each frame; a malformed raylib call (won't happen) returns no event rather than crashing.
- **TUI capture loss:** if `crossterm::event::read()` returns `Event::Resize` mid-drag, the drag anchor is invalidated — `on_resize` clears `drag_anchor` and `ClickTracker`.
- **Lua handler throws:** caught in `dispatch_mouse`; logged via `ruster.notify.warn`; event still runs its default action (don't punish the user for a plugin crash).
- **Out-of-bounds cell coords:** `poll_mouse` clamps to `viewport_cells()`; the dispatcher early-returns if `(col, row) >= (W, H)` after a resize race.

## Testing strategy

Three layers, mirroring existing patterns:

1. **Pure hit-test tests** — unit-test `App::hit_test` with a known `last_layout`. (Reuses pattern at `app.rs:11593-11660`.) New tests for chrome/float/gutter zones; existing buffer tests stay green.
2. **`ScriptedRenderer` mouse scripts** — extend the test harness at `crates/ruster-render/src/script.rs:304+` to record `Vec<MouseEvent>` alongside `Vec<KeyEvent>`. New `simulate_mouse_click(...)`, `simulate_mouse_drag(...)`, `simulate_mouse_wheel(...)` helpers. Drives `App::handle_mouse_event` deterministically.
3. **End-to-end visuals** — re-use the `gui-check` skill (already at `.claude/skills/gui-check/SKILL.md`) for the GUI surface; new surface entries `docs/verification/<surface>-mouse-{gui,tui}.{png,txt}` per Phase 10. Surfaces to capture:
   - `mouse-click-position`: click moves cursor + visual caret follow.
   - `mouse-drag-visual`: drag enters Visual block.
   - `mouse-wheel-scroll`: wheel scrolls window under cursor.
   - `mouse-right-click-menu`: menu opens, item runs.
   - `mouse-hover-popup`: Lua handler pops a float.
   - `mouse-double-click-word`: word selected.
   - `mouse-chrome-tab-click`: tab switch via mouse.
   - `mouse-tui-right-click`: same surface in TUI.

The verification surface entries feed Phase 10's matrix; defects found in capture become Phase 9 bugs.

## Migration & rollout

- Default `[mouse] enabled = true` in TUI and GUI; respects existing Alt+Left behavior.
- `tui_capture = false` opt-out for TUI users who want terminal text-selection to keep working — the existing single Alt+Left handler still fires (still routed through crossterm mpsc) but all other gestures ignore.
- No database migration; no versioned config bumps.
- No keymap default changes; existing vim/emacs users see no difference until they pick up the mouse.

## Risks & mitigations

| Risk | Mitigation |
|---|---|
| TUI capture fights terminal text selection | `tui_capture = false` opt-out, documented in `docs/keybindings.md` |
| Drag in Neovim mode surprises users (`vw` semantics aren't intuitive) | Triple-click → line, double-click → word first; users learn mouse Visual from those |
| Right-click in TUI: terminal may not send mouse-up | Phase 1 only opens menu on `Down`; menu dismissal handles the no-up case via Esc / outside-click |
| `ScriptedRenderer` test scripts brittle to mouse additions | Default `poll_mouse = None`; no existing test changes |
| GUI hot-loop cost (60Hz hover ticks, scroll rate-limiting) | Throttle scroll events to ≤120Hz; hover ticks are constant-time, ≤5µs |
| Cell-coord precision: raylib HiDPI could give fractional cells | `cell_metrics()` returns floats; `poll_mouse` does `.floor()` to cells; documented |

## Open questions

1. Should double-click on a non-word buffer (e.g. inside a long URL) select to whitespace, to newline, or to punctuation? (Phasestretch — pick "whitespace" for v1, document it.)
2. Does drag-select cross split boundaries? Phase 1 says: **no** — drag in buffer stays in the originating window. Cross-window selection is a future surface.
3. Does the hover hook fire for **all** moves, or only buffer-zone moves? Phase 1: **buffer zone only** — hover over gutter/chrome/menu emits nothing. (Plugins can subscribe to `mouse_move` for that.)

## Definition of done

- [ ] `crates/ruster-render/src/mouse.rs` exists; `MouseEvent`/`MouseKind`/`MouseButton`/`CursorKind` are pub.
- [ ] `Renderer::poll_mouse`, `cell_metrics`, `set_cursor` exist; default impls are no-op.
- [ ] `RaylibRenderer` implements all three.
- [ ] `App::handle_mouse_event` dispatches the full gesture matrix; existing Alt+Left test still passes.
- [ ] All eight verification surfaces captured in `docs/verification/mouse-*`.
- [ ] `cargo build` and `cargo test` green; no new clippy warnings.
- [ ] `docs/config-reference.md`, `docs/keybindings.md`, `docs/lua-api.md` updated.
- [ ] No growth: `App::handle_mouse_event` and its helpers live in a new `crates/ruster-tui/src/mouse.rs` file (not `app.rs`); `app.rs` line count growth < 200 lines, restricted to a single `mouse: MouseState` field plus a delegation call.
