# Mouse Controls Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add unified mouse controls (click, drag, double/triple-click, wheel, hover, context menu, cursor-shape) to ruster across both TUI and raylib GUI backends via a single `MouseEvent` type, single dispatcher, and shared Lua surface — without breaking headless tests.

**Architecture:** A new `ruster_render::mouse::MouseEvent` (cell coords + mods) is produced by each backend from native input; `App::handle_mouse_event` is a single dispatcher that hit-tests (float → chrome → gutter → buffer → outside), runs Lua consume-or-pass, and routes to per-zone handlers with mode-aware drag semantics; a new `MouseState` on `App` owns `ClickTracker`, `HoverState`, and `drag_anchor`; the GUI also gets cursor-shape changes via a new `Renderer::set_pointer`. No backend branches inside core handlers — pixel math is confined to `RaylibRenderer::poll_mouse`.

**Tech Stack:** Rust, `crossterm::event::MouseEvent` (TUI source), `raylib::GetMousePosition` / `IsMouseButtonPressed` (GUI source), `mlua` (Lua extension), `nucleo_matcher` (no change), existing `EventBus` (extended), existing `PickerState` (reused for menus).

## Global Constraints

- **Spec source of truth:** `docs/superpowers/specs/2026-08-10-mouse-controls-design.md` (this plan's every requirement).
- **No growth budget on `app.rs`:** `App::handle_mouse_event` and its helpers live in a new `crates/ruster-tui/src/mouse.rs`. Only one new field on `App` (`mouse: MouseState`) plus a one-line delegation. Growth < 200 lines in `app.rs` for this work.
- **No regression:** `cargo test` stays green at every commit. `ScriptedRenderer` tests, Alt+Left test at `app.rs:11593+`, and all hit-test tests at `app.rs:11593-11660` remain passing.
- **No new dependencies** in `Cargo.toml` (everything reuses existing crates).
- **TUI capture:** keep `crossterm::execute!(stdout, EnableMouseCapture)` at `app.rs:3052`; new `tui_capture = false` opt-out via `config.mouse.enabled`.
- **Headless default:** `Renderer::poll_mouse` returns `None`; `ScriptedRenderer` keeps its existing one-key-per-frame rule unchanged.
- **Naming:** `MouseEvent` / `MouseKind` / `MouseButton` / `PointerKind` (exact spellings from spec).
- **No `#[derive]` on `MouseEvent`** with `Default` — cell coords are mandatory at construction.
- **Phase 2 scope-out:** `[mouse.bindings]` table, drag-and-drop file open, multi-touch, mouse-driven command palette — **not** built here.
- **Definition of done (all gates):** `cargo build` green, `cargo test` green, no new clippy warnings, eight `docs/verification/mouse-*` artifacts committed.

## File structure

| Path | Status | Responsibility |
|---|---|---|
| `crates/ruster-render/src/mouse.rs` | NEW | `MouseEvent`, `MouseKind`, `MouseButton`, `PointerKind` (cell-coord, no backend deps) |
| `crates/ruster-render/src/lib.rs` | MODIFY | Add default-no-op `poll_mouse`, `cell_metrics`, `set_pointer` to `Renderer`; pub-mod `mouse`; re-export the four mouse types (the pre-existing `CursorKind` is the *caret* shape and is left alone) |
| `crates/ruster-render/src/script.rs` | MODIFY | `ScriptedRenderer` records `Vec<MouseEvent>` alongside keys; new helpers `simulate_mouse_click/drag/wheel`; `FrameDigest.mouse_events` field |
| `crates/ruster-core/src/action.rs` | MODIFY | Add `Action::SelectWord`, `Action::SelectLine`, `Action::SetMark(usize)`, `Action::ResizeWindow { wid, dy, dx }` |
| `crates/ruster-core/src/cursor.rs` | MODIFY | Add `CursorSet::select_word(&Buffer, anchor) -> Range`, `select_line(&Buffer, anchor) -> Range`, `set_region(mark: usize, point: usize)` |
| `crates/ruster-tui/src/app.rs` | MODIFY | Replace the 12-line stub at `:2917-2928` with one-line delegation; add `mouse: MouseState`; add `is_gui: bool` (default false); add `on_resize` hook clears `drag_anchor`/`ClickTracker` |
| `crates/ruster-tui/src/mouse.rs` | NEW | `MouseState`, `ClickTracker`, `HoverState`, `DragState`, `MenuRegistry`, `handle_mouse_event`, `hit_test`, `on_mouse_down/up/drag/move/scroll`, `default_context_menu_items` |
| `crates/ruster-tui/src/lib.rs` | MODIFY | Add `pub mod mouse;` |
| `crates/ruster-render-raylib/src/lib.rs` | MODIFY | Implement `poll_mouse` (pixel→cell via `cell_metrics`), `cell_metrics`, `set_pointer`; emit `MouseEvent` into event queue during `drain_raylib` |
| `crates/ruster-lua/src/config.rs` | MODIFY | Add `pub mouse: MouseConfig` to `Config`; populate in `Config::default()` |
| `crates/ruster-lua/src/schema.rs` | MODIFY | Add `(mouse, enabled|hover_delay_ms|double_click_ms|wheel_lines|tui_capture|right_click_menu)` specs |
| `crates/ruster-lua/src/event.rs` | MODIFY | `EventBus::emit_consuming(&self, lua, name, args) -> bool` (returns true if any handler returned true) |
| `crates/ruster-lua/src/runtime.rs` | MODIFY | `LuaRuntime::dispatch_mouse(&self, ev) -> bool`, `dispatch_hover(&self, payload)` |
| `crates/ruster-lua/src/lib.rs` | MODIFY | Register `ruster.on("mouse_*"|"hover")`, `ruster.mouse.{get,set}`, `ruster.context_menu.add(zone, item)` |
| `docs/config-reference.md` | MODIFY | New `[mouse]` section |
| `docs/keybindings.md` | MODIFY | Mouse gesture table |
| `docs/lua-api.md` | MODIFY | `ruster.on("mouse_*")`, `ruster.mouse.*`, `ruster.context_menu` |
| `docs/verification/mouse-{click,drag,wheel,right-click,hover,double-click,chrome-tab,tui-right-click}-{tui.txt,gui.png}` | NEW | Eight Phase-10 verification surfaces |

Tasks below modify these in the listed order; later tasks reference exact symbols from earlier ones — **types and method names are locked by Task 1**.

---

## Task 1: Define `ruster_render::mouse` types (TDD)

**Files:** `crates/ruster-render/src/mouse.rs` (new), `crates/ruster-render/src/lib.rs` (add `pub mod mouse;` and re-export)

**Goal:** Establish the shared mouse event type so all downstream tasks compile against a stable contract.

**Locked symbols (do not rename later):**
- `pub struct MouseEvent { col: u16, row: u16, kind: MouseKind, button: MouseButton, modifiers: KeyModifiers }`
- `pub enum MouseKind { Down, Up, Drag, Move, ScrollUp, ScrollDown, ScrollLeft, ScrollRight }`
- `pub enum MouseButton { Left, Right, Middle, None }`
- `pub enum PointerKind { Default, IBeam, Resize, Crosshair, PointingHand }`
- `pub fn from_crossterm(ev: crossterm::event::MouseEvent) -> MouseEvent`

**Steps:**

- [x] Create `crates/ruster-render/src/mouse.rs` with `MouseEvent`, `MouseKind`, `MouseButton`, `PointerKind`, `from_crossterm`, plus `Default for PointerKind = PointerKind::Default`.
- [x] Add `pub mod mouse;` near the top of `crates/ruster-render/src/lib.rs` (after `pub mod script;`).
- [x] Add `pub use mouse::{MouseButton, MouseEvent, MouseKind, PointerKind};` to `lib.rs` (the existing caret `CursorKind` re-export is untouched).
- [x] Add unit tests in `mouse.rs`:
  - `mouse_kind_round_trips_via_debug`
  - `from_crossterm_maps_scroll_up`
  - `from_crossterm_maps_drag_with_left_button`
  - `mouse_button_none_only_for_move_or_scroll`
- [x] Run `cargo test -p ruster-render` — verify all pass.
- [x] Commit: `feat(render): add unified MouseEvent / MouseKind / MouseButton / PointerKind types`. *(`ddb8284`)*

## Task 2: Extend `Renderer` trait with default-no-op mouse seams

**Files:** `crates/ruster-render/src/lib.rs`

**Goal:** Add `poll_mouse`, `cell_metrics`, `set_pointer` to the trait so backends opt in. Defaults preserve headless tests.

**Steps:**

- [x] In `crates/ruster-render/src/lib.rs` after `fn poll_input(&mut self)` (~line 1029), add:
  ```rust
  fn poll_mouse(&mut self) -> Option<mouse::MouseEvent> { None }
  fn cell_metrics(&self) -> (f32, f32) { (1.0, 1.0) }
  fn set_pointer(&mut self, _pointer: mouse::PointerKind) -> bool { false }
  ```
- [x] Run `cargo build --workspace` — verify zero new warnings.
- [x] Run `cargo test --workspace` — verify `ScriptedRenderer` and `TestRenderer` still pass (they use the defaults). *(1022 passed)*
- [x] Commit: `feat(render): add poll_mouse / cell_metrics / set_pointer defaults to Renderer`. *(`98cb33b`)*

## Task 3: TUI delegation — preserve Alt+Left, route to new dispatcher

**Files:** `crates/ruster-tui/src/app.rs`, `crates/ruster-tui/src/lib.rs`, `crates/ruster-tui/src/mouse.rs`

**Goal:** Replace the 12-line stub at `app.rs:2917-2928` with a delegation to a new dispatcher module. Existing Alt+Left test must stay green.

**Steps:**

- [ ] Create empty skeleton `crates/ruster-tui/src/mouse.rs` with `pub fn handle_mouse_event(app: &mut App, ev: ruster_render::MouseEvent)` that calls back to the existing `app.buffer_offset_at` only for `MouseKind::Down(Left)` with `Alt` — exact same behavior as the stub. **No new state, no new logic, just a thin extraction.**
- [ ] In `crates/ruster-tui/src/lib.rs`, add `pub mod mouse;` next to the other module declarations.
- [ ] In `crates/ruster-tui/src/app.rs`, replace lines 2917-2928 with `crate::mouse::handle_mouse_event(self, ev.into());` (convert crossterm `MouseEvent` to `ruster_render::MouseEvent` via `crate::mouse::from_crossterm`).
- [ ] Run `cargo test -p ruster-tui` — verify `mouse_hit_test_*` tests at `app.rs:11593+` still pass and the new dispatcher is hit (add a temporary `dbg!` then remove it; do **not** commit the `dbg!`).
- [ ] Commit: `refactor(tui): extract App::handle_mouse_event into crate::mouse`.

## Task 4: Add `MouseState` skeleton to `App`

**Files:** `crates/ruster-tui/src/app.rs`, `crates/ruster-tui/src/mouse.rs`

**Goal:** Introduce the in-progress drag/click/hover bookkeeping without behavior. All fields default to neutral; no-op handlers exist.

**Steps:**

- [ ] In `crates/ruster-tui/src/mouse.rs`, add:
  ```rust
  pub struct ClickTracker { pub last_down: Option<(Instant, u16, u16, MouseButton)> }
  pub struct HoverState { pub last_pos: (u16, u16), pub last_move: Instant, pub emitted_for: Option<(u16, u16)> }
  pub struct DragState { pub anchor: Option<usize>, pub kind: DragKind, pub wid: Option<WindowId> }
  pub enum DragKind { Char, Line, Block }
  pub struct MouseState {
      pub click: ClickTracker,
      pub hover: HoverState,
      pub drag: DragState,
      pub menu: MenuRegistry,
      pub resize: Option<ResizeState>,
  }
  pub struct ResizeState { pub wid: WindowId, pub start_col: u16, pub start_row: u16, pub original: Rect }
  pub struct MenuRegistry { items: HashMap<Zone, Vec<MenuItem>> }
  pub enum Zone { Buffer, Gutter, Chrome, Tab }
  pub struct MenuItem { pub label: String, pub cmd: String, pub submenu: Vec<MenuItem> }
  impl Default for MouseState { … }
  ```
- [ ] Add `pub mouse: MouseState` field to `App` in `app.rs` near `last_layout` (~line 1485); initialize via `MouseState::default()` in `App::new`.
- [ ] Add `pub is_gui: bool` field to `App` defaulting to `false`; set `is_gui = true` in `app.run_gui()` (the GUI entrypoint called from `crates/ruster-bin/src/main.rs:50`).
- [ ] No behavior change yet. `crate::mouse::handle_mouse_event` ignores state.
- [ ] Run `cargo test -p ruster-tui` — verify all existing tests still green.
- [ ] Commit: `feat(tui): add MouseState skeleton (ClickTracker, HoverState, DragState, MenuRegistry)`.

## Task 5: Hit-test zones — TDD each zone

**Files:** `crates/ruster-tui/src/mouse.rs`, `crates/ruster-tui/src/app.rs` (read-only)

**Goal:** Build the priority-ordered zone resolver. Zone order: Float > Chrome > Gutter > Buffer > Outside. Reuse `last_layout` and `TextArea::cell_at`.

**Steps:**

- [ ] Add `pub enum HitZone { Chrome(ChromeKind), Gutter(WindowId, usize), Buffer(WindowId, usize), Float(FloatId), Outside }` and `pub enum ChromeKind { Tab(usize), StatusSection(String), SplitEdge { wid: WindowId, vertical: bool } }` to `mouse.rs`.
- [ ] Add `pub fn hit_test(app: &App, col: u16, row: u16) -> HitZone` with the priority cascade. Float hits first by checking `app.floats` (read existing `Vec<FloatView>` rendering field at `app.rs:4656+`); chrome by checking tab row at y=0, split edges via geometry; gutter by checking `TextArea::cell_at` returns `None` AND col < `TextArea::x`; buffer by `buffer_offset_at` (reuse at `app.rs:2936`); outside otherwise.
- [ ] Add tests using the existing `rendered_text_area` helper at `app.rs:11587`:
  - `hit_test_buffer_for_text_cell`
  - `hit_test_gutter_for_left_margin`
  - `hit_test_chrome_for_tab_row`
  - `hit_test_outside_for_statusline_blank`
  - `hit_test_float_wins_over_buffer`
- [ ] Run `cargo test -p ruster-tui hit_test` — verify all pass.
- [ ] Commit: `feat(tui): hit-test zones (float > chrome > gutter > buffer > outside)`.

## Task 6: Add new `Action` and `CursorSet` methods (TDD)

**Files:** `crates/ruster-core/src/action.rs`, `crates/ruster-core/src/cursor.rs`

**Goal:** Wire up the missing action variants and cursor helpers needed by click/drag handlers.

**Steps:**

- [ ] In `action.rs`, add to `pub enum Action`:
  ```rust
  SelectWord { anchor: usize, head: usize },
  SelectLine { anchor: usize, head: usize },
  SetMark(usize),
  ResizeWindow { wid: WindowId, dy: i32, dx: i32 },
  ```
- [ ] In `cursor.rs`, add on `CursorSet`:
  ```rust
  pub fn select_word(&self, buf: &Buffer, anchor: usize) -> Range;   // expand anchor outward to whitespace
  pub fn select_line(&self, buf: &Buffer, anchor: usize) -> Range;   // expand to line bounds
  pub fn set_region(&mut self, mark: usize, point: usize);          // Emacs: anchor=mark, head=point
  ```
- [ ] Unit tests on `CursorSet::select_word` with `"foo bar"` and offsets 1, 4, 6 — verify each gives the right word bound.
- [ ] Unit tests on `CursorSet::select_line` with `"a\nbc\nde"` and offsets 0, 3, 6.
- [ ] Compile check across the workspace — `Action::SelectWord` etc. don't break `Action` exhaustiveness anywhere.
- [ ] Run `cargo test --workspace` — verify all pass.
- [ ] Commit: `feat(core): add Action::SelectWord/SelectLine/SetMark/ResizeWindow and CursorSet selection helpers`.

## Task 7: Click handlers — Left, Alt+Left, Ctrl+Left, double, triple (TDD)

**Files:** `crates/ruster-tui/src/mouse.rs`, `crates/ruster-tui/src/app.rs` (read-only)

**Goal:** Implement `on_mouse_down`. Alt+Left must remain identical to the current behavior so the test at `app.rs:11593+` stays green.

**Steps:**

- [ ] Add `fn on_mouse_down(app: &mut App, ev: MouseEvent, zone: HitZone)` to `mouse.rs`. Dispatch:
  - `HitZone::Buffer(wid, offset)`:
    - Plain Left → `app.ws.borrow_mut().execute(Action::Move(Motion::To(offset)))`
    - Alt+Left → `app.ws.borrow_mut().execute(Action::AddCursor(offset))` (existing)
    - Ctrl+Left → walk to next word boundary via `CursorSet::select_word` then `Action::Move`
    - Then update `ClickTracker` from `ev.kind == Down`; if 2nd click within `config.mouse.double_click_ms` (default 400ms) and within 2 cells, fire `Action::SelectWord`; if 3rd, fire `Action::SelectLine` and reset.
  - `HitZone::Outside` → focus cmdline (set `vim.mode = VimMode::Cmdline`).
  - `HitZone::Chrome(ChromeKind::Tab(n))` → call into `app.switch_to_buffer_n(n)` (route via existing `CmdAction::Buffer`).
  - `HitZone::Chrome(ChromeKind::StatusSection(_))` → no-op (phase 1; plugin hook exists in later task).
  - `HitZone::Float(_)` → defer to `Float::hit_test` (Task 13); phase 1 only closes outside-clicks.
- [ ] Wire into `crate::mouse::handle_mouse_event` after hit-test.
- [ ] Tests in `crates/ruster-tui/src/mouse.rs` (using `App::new`):
  - `left_click_in_buffer_moves_cursor`
  - `alt_left_click_adds_cursor`
  - `ctrl_left_click_jumps_word`
  - `double_click_within_window_selects_word`
  - `triple_click_selects_line`
  - `click_in_outside_zone_focuses_cmdline`
- [ ] Run `cargo test -p ruster-tui` — verify the existing 4 hit-test tests plus the 6 new ones, all green.
- [ ] Commit: `feat(tui): click handlers (left, alt-left, ctrl-left, double, triple)`.

## Task 8: Drag handlers — Neovim Visual vs Emacs region (TDD)

**Files:** `crates/ruster-tui/src/mouse.rs`

**Goal:** Mode-aware drag. `drag_anchor` lives on `MouseState`. First Drag decides Char/Line/Block by direction (Line if crossed a line boundary by the 2nd Drag event; Block if Alt held on first Drag).

**Steps:**

- [ ] Add `fn on_mouse_drag(app: &mut App, ev: MouseEvent, zone: HitZone)`.
- [ ] On the **first** Drag event after a Down, set `app.mouse.drag.anchor = Some(offset); app.mouse.drag.wid = Some(wid); app.mouse.drag.kind = DragKind::Char` (default).
- [ ] If `ev.modifiers.contains(ALT)` on the first Drag, switch `kind = DragKind::Block`.
- [ ] If by the second Drag the offset has crossed a line boundary, switch `kind = DragKind::Line`.
- [ ] Per Drag event:
  - Neovim Normal/Insert → `Action::BeginVisual(anchor)` once on first Drag; subsequent Drags → `Action::Move(Motion::To(offset))`.
  - Emacs → if mark not set, `Action::SetMark(anchor)` once; subsequent Drags → `CursorSet::set_region(mark, offset)` then render region via existing `SelectionView`.
  - Picker/dialog zone → consumed (no-op).
- [ ] On `MouseKind::Up` with no movement since Down → revert to a single caret (cancel Visual/region).
- [ ] Tests:
  - `drag_in_neovim_enters_visual_block_when_alt_held`
  - `drag_in_neovim_promotes_to_line_after_crossing_line`
  - `drag_in_emacs_sets_mark_and_extends_region`
  - `up_without_drag_keeps_caret_not_visual`
- [ ] Run `cargo test -p ruster-tui` — verify all green.
- [ ] Commit: `feat(tui): drag handlers with mode-aware visual/region selection`.

## Task 9: Wheel — vertical, Shift+horizontal, Ctrl+zoom (TDD)

**Files:** `crates/ruster-tui/src/mouse.rs`

**Goal:** Implement `on_mouse_scroll`. Ctrl+wheel is GUI-only and shows a noice toast in TUI.

**Steps:**

- [ ] Add `fn on_mouse_scroll(app: &mut App, ev: MouseEvent, zone: HitZone)`.
- [ ] Branch:
  - `ev.modifiers.contains(CONTROL)` → if `app.is_gui`, call `app.zoom_font(dir)` (new method, see Task 10); else emit noice toast `"Ctrl+wheel zoom: GUI only"`.
  - `ScrollLeft | ScrollRight` → scroll horizontal on `wid` from `zone`.
  - `ScrollUp | ScrollDown` → `wid.scroll_top = wid.scroll_top.saturating_add_signed(dir * lines)` where `lines = app.config.mouse.wheel_lines`.
- [ ] Tests:
  - `wheel_scroll_up_decrements_scroll_top`
  - `wheel_shift_modifier_scrolls_horizontal`
  - `wheel_ctrl_in_tui_emits_toast_not_action`
  - `wheel_outside_buffer_is_noop`
- [ ] Run `cargo test -p ruster-tui` — all green.
- [ ] Commit: `feat(tui): wheel handlers (vertical / shift-horizontal / ctrl-zoom GUI-only)`.

## Task 10: GUI zoom + cursor-shape plumbing on `RaylibRenderer`

**Files:** `crates/ruster-render-raylib/src/lib.rs`, `crates/ruster-tui/src/app.rs`

**Goal:** Implement `poll_mouse`, `cell_metrics`, `set_pointer` on `RaylibRenderer`. Expose `App::zoom_font` (changes `config.font_size` by ±1, clamped 8..72).

**Steps:**

- [ ] In `crates/ruster-render-raylib/src/lib.rs` `impl Renderer for RaylibRenderer` (~line 507), add:
  ```rust
  fn cell_metrics(&self) -> (f32, f32) { (self.char_w, self.line_h) }
  fn set_pointer(&mut self, kind: ruster_render::PointerKind) -> bool {
      // Map to existing internal cursor sprite; return true on change.
      let prev = self.cursor_kind;
      self.cursor_kind = kind;
      prev != kind
  }
  fn poll_mouse(&mut self) -> Option<ruster_render::MouseEvent> {
      // Translate raylib mouse state to a MouseEvent; pixel→cell via cell_metrics().
      // Drain into self.event_buffer as MouseEvent items (extend the queue enum).
  }
  ```
- [ ] Extend the existing event queue enum to include `Mouse(ruster_render::MouseEvent)` (separate from the `Vec<KeyEvent>` already at `event_buffer`).
- [ ] In `App::run_gui` (called from `crates/ruster-bin/src/main.rs:50`), set `self.is_gui = true`.
- [ ] In `crates/ruster-tui/src/app.rs`, add `pub fn zoom_font(&mut self, dir: i32)` that adjusts `self.config.font_size = (self.config.font_size as i32 + dir).clamp(8, 72) as u32` and re-applies via `self.renderer.set_gui_config(...)`.
- [ ] Tests:
  - In `crates/ruster-render-raylib/src/lib.rs` `#[cfg(test)]`, test `cell_metrics_returns_font_dims`.
  - In `crates/ruster-tui/src/app.rs`, test `zoom_font_clamps_to_min_and_max`.
- [ ] Run `cargo build --workspace` and `cargo test --workspace` — green.
- [ ] Commit: `feat(render-raylib): implement poll_mouse/cell_metrics/set_pointer; feat(tui): add zoom_font`.

## Task 11: Extend `ScriptedRenderer` with mouse scripting (TDD)

**Files:** `crates/ruster-render/src/script.rs`

**Goal:** Allow headless tests to drive the mouse surface deterministically. No existing script changes shape.

**Steps:**

- [ ] In `crates/ruster-render/src/script.rs`:
  - Add `pub mouse_events: Vec<MouseEvent>` field to `FrameDigest` (next to `cmdline: Option<String>`).
  - Add `pub mouse: VecDeque<MouseEvent>` field to `ScriptedRenderer` (next to `keys: VecDeque<KeyEvent>`).
  - Add `pub fn push_mouse(mut self, ev: MouseEvent) -> Self` builder.
  - Add `pub fn simulate_mouse_click(&mut self, col: u16, row: u16, button: MouseButton, mods: KeyModifiers)`.
  - Add `pub fn simulate_mouse_drag(&mut self, from: (u16,u16), to: (u16,u16), button: MouseButton, mods: KeyModifiers)` — emits Down, N intermediate Drags (every 4 cells), Up.
  - Add `pub fn simulate_mouse_wheel(&mut self, col: u16, row: u16, dir: ScrollDir, mods: KeyModifiers)`.
  - Override `poll_mouse` to drain `self.mouse` one-per-frame (same rule as `poll_input`).
  - In `render_frame`, after capture, push the mouse events consumed this frame into `state.mouse_events` — wait, `FrameDigest` doesn't have `state` post-build, so just record a clone: `FrameDigest.mouse_events.push(ev.clone())` for each mouse event drained.
- [ ] Tests:
  - `one_mouse_event_per_frame_just_like_keys`
  - `simulate_mouse_click_emits_down_then_up`
  - `simulate_mouse_drag_emits_multiple_drag_events`
- [ ] Run `cargo test -p ruster-render` — all green.
- [ ] Commit: `feat(render): extend ScriptedRenderer with mouse scripting`.

## Task 12: Cursor-shape changes per zone (GUI only, TDD)

**Files:** `crates/ruster-tui/src/mouse.rs`, `crates/ruster-tui/src/app.rs`

**Goal:** Call `self.renderer.set_pointer(PointerKind)` after every hit-test in the GUI; no-op in TUI (the default `set_pointer` already returns false).

**Steps:**

- [ ] In `crates/ruster-tui/src/mouse.rs`, add `fn set_pointer_for_zone(app: &mut App, zone: HitZone)` mapping:
  - `Buffer(_)` → `IBeam`
  - `Chrome(SplitEdge{..})` → `Resize`
  - `Chrome(Tab(_)) | Chrome(StatusSection(_)) | Gutter(_)` → `PointingHand`
  - `Float(_) | Outside` → `Default`
- [ ] Call after hit-test in `handle_mouse_event` only when `app.is_gui`.
- [ ] Test (uses a stub renderer that records `set_pointer` calls):
  - `cursor_set_to_ibeam_in_buffer_zone_gui_only`
  - `cursor_set_to_resize_on_split_edge`
- [ ] Run `cargo test -p ruster-tui` — green.
- [ ] Commit: `feat(tui): cursor-shape per zone (GUI only)`.

## Task 13: Right-click menu — `FloatKind::Menu` + `MenuRegistry` (TDD)

**Files:** `crates/ruster-tui/src/mouse.rs`, `crates/ruster-tui/src/picker.rs` (read-only), `crates/ruster-render/src/lib.rs`

**Goal:** Add `FloatKind::Menu(Vec<MenuItem>)` reusing `PickerState` for selection. Default buffer items registered in `MenuRegistry::default()`.

**Steps:**

- [ ] In `crates/ruster-render/src/lib.rs`, add alongside `FloatView`:
  ```rust
  pub enum FloatKind { Plain, Picker, Menu }
  pub struct FloatSpec { pub rect: Rect, pub kind: FloatKind, pub items: Vec<MenuItem> }
  pub struct MenuItem { pub label: String, pub cmd: String }
  ```
- [ ] In `crates/ruster-tui/src/mouse.rs`, populate `MenuRegistry::default()` with the spec's default items: Cut, Copy, Paste, Select All, Copy Filename, Copy Path, Toggle Line Comment (mode-aware), Format Buffer. Each maps to its `ruster.cmd` string.
- [ ] On `MouseKind::Down(Right)` in `HitZone::Buffer`, push a new `FloatKind::Menu` float into `app.floats`. Render by translating `MenuItem` rows into `PickerView` rows.
- [ ] Add `App::dispatch_menu_select(idx: usize)` that takes the selected menu item's `cmd` and runs it via existing `CmdAction` parsing.
- [ ] Tests:
  - `right_click_in_buffer_pushes_menu_float`
  - `menu_registry_default_has_format_buffer`
  - `menu_item_cmd_round_trips_through_cmdline`
- [ ] Run `cargo test -p ruster-tui` — green.
- [ ] Commit: `feat(tui): right-click menu via FloatKind::Menu + default MenuRegistry`.

## Task 14: Chrome interactions — tab click, statusline routing, split-edge drag (TDD)

**Files:** `crates/ruster-tui/src/mouse.rs`, `crates/ruster-tui/src/app.rs`

**Goal:** Wire up chrome clicks. Statusline section routing is a no-op for phase 1 (registry only). Split-edge resize is a 3-frame drag recorded in `MouseState.resize`.

**Steps:**

- [ ] In `on_mouse_down` for `HitZone::Chrome(ChromeKind::Tab(n))`:
  - Left → switch to buffer N (call `App::select_buffer_n(n)`).
  - Middle → close buffer N (`CmdAction::Bdelete`).
  - Right → open a `FloatKind::Menu` with tab-specific items.
- [ ] For `HitZone::Chrome(ChromeKind::StatusSection(name))`:
  - Add `pub fn register_statusline_click(app: &mut App, section: String, handler: Box<dyn Fn(&mut App)>)` and a `Vec<(String, Box<dyn Fn>>>` on `App`. Phase 1: only `position` (no-op) and `mode` (no-op) registered.
- [ ] For `HitZone::Chrome(ChromeKind::SplitEdge { wid, vertical })`:
  - Down → set `app.mouse.resize = Some(ResizeState { wid, start_col, start_row, original })`.
  - Drag → compute dy/dx delta; on Up → emit `Action::ResizeWindow { wid, dy, dx }` (Task 6) and clear `resize`.
- [ ] Tests:
  - `tab_left_click_switches_buffer`
  - `tab_middle_click_closes_buffer`
  - `split_edge_drag_emits_resize_window_action`
  - `statusline_section_click_invokes_handler`
- [ ] Run `cargo test -p ruster-tui` — green.
- [ ] Commit: `feat(tui): chrome interactions (tab, statusline, split-edge resize)`.

## Task 15: Lua surface — `ruster.on("mouse_*")`, `ruster.mouse.*`, `ruster.context_menu.add` (TDD)

**Files:** `crates/ruster-lua/src/event.rs`, `crates/ruster-lua/src/runtime.rs`, `crates/ruster-lua/src/lib.rs`, `crates/ruster-lua/src/api.rs`

**Goal:** Consume-or-pass semantics for handlers; expose `ruster.mouse.{get,set}`; allow plugins to add context-menu items by zone.

**Steps:**

- [ ] In `crates/ruster-lua/src/event.rs`, add `pub fn emit_consuming(&self, lua: &Lua, event: &str, args: &[mlua::Value]) -> bool` — returns `true` if any handler returned `true`. Use `pcall` to swallow handler errors and log them via `ruster.notify.warn` (call back into Lua side).
- [ ] In `crates/ruster-lua/src/runtime.rs`, add:
  ```rust
  pub fn dispatch_mouse(&self, ev: &MouseEvent, zone: &str, extras: Value) -> bool;
  pub fn dispatch_hover(&self, payload: Value);
  ```
- [ ] In `crates/ruster-lua/src/lib.rs`, register Lua bindings:
  - `ruster.on("mouse_down"|"mouse_up"|"mouse_drag"|"mouse_move"|"mouse_wheel", function(ev) ... end)` — handlers may return `true` to consume.
  - `ruster.on("hover", function(payload) ... end)`.
  - `ruster.mouse.get(key) -> value`, `ruster.mouse.set(key, value)`.
  - `ruster.context_menu.add(zone, { label = ..., action = "..." })`.
- [ ] In `crates/ruster-tui/src/mouse.rs`, wire `crate::mouse::handle_mouse_event` to call `app.lua.dispatch_mouse(&ev, &zone_str, payload)` first; if it returns `true`, return early.
- [ ] Tests using existing `LuaRuntime::new_for_test`:
  - `mouse_down_handler_returning_true_consumes_event`
  - `mouse_handler_throwing_does_not_punish_default`
  - `ruster_mouse_set_round_trips_via_get`
  - `context_menu_add_appends_to_zone`
- [ ] Run `cargo test --workspace` — all green.
- [ ] Commit: `feat(lua): ruster.on(mouse_*|hover), ruster.mouse.{get,set}, ruster.context_menu.add`.

## Task 16: Hover hook — 300ms throttle (TDD)

**Files:** `crates/ruster-tui/src/mouse.rs`, `crates/ruster-tui/src/app.rs`

**Goal:** Emit `ruster.on("hover", payload)` when the pointer has been still for `config.mouse.hover_delay_ms` (default 300). Buffer zone only (per spec open-question #3).

**Steps:**

- [ ] In `crate::mouse::on_mouse_move`, update `app.mouse.hover.last_pos` and `last_move = Instant::now()`.
- [ ] Add `fn hover_tick(app: &mut App)` called once per frame from the frame loop (call it next to `fire_watched_events` at `app.rs:3108`).
- [ ] `hover_tick` checks: if `now - hover.last_move > hover_delay` and `hover.last_pos != hover.emitted_for` and zone at that pos is `Buffer`, build a payload `{col, row, offset, wid, line, col_in_line}` and call `app.lua.dispatch_hover(payload)`, then set `emitted_for = Some(last_pos)`.
- [ ] Add `pub fn hover_tick(&mut self)` on `App` that wraps the call.
- [ ] Tests:
  - `hover_tick_emits_after_delay`
  - `hover_tick_does_not_re_emit_for_same_position`
  - `hover_tick_skips_non_buffer_zone`
- [ ] Run `cargo test -p ruster-tui` — green.
- [ ] Commit: `feat(tui): hover hook with 300ms throttle (buffer zone only)`.

## Task 17: Configuration schema — `[mouse]` section

**Files:** `crates/ruster-lua/src/config.rs`, `crates/ruster-lua/src/schema.rs`

**Goal:** Wire the spec's `[mouse]` config block into the schema/parser/Settings page/CLI defaults.

**Steps:**

- [ ] In `crates/ruster-lua/src/config.rs`, add:
  ```rust
  pub struct MouseConfig {
      pub enabled: bool,             // default true
      pub hover_delay_ms: u32,       // default 300
      pub double_click_ms: u32,      // default 400
      pub wheel_lines: u32,          // default 3
      pub tui_capture: bool,         // default true
      pub right_click_menu: bool,    // default true
  }
  ```
  Add `pub mouse: MouseConfig` to `Config`. Populate in `Config::default()`. Update `to_settings()` to include the 6 entries under `(mouse, ...)`.
- [ ] In `crates/ruster-lua/src/schema.rs`, add the 6 entries to `pub fn schema()`:
  ```rust
  add("mouse", "enabled",            "Mouse enabled",                       Bool, b(true), "Master switch for all mouse input");
  add("mouse", "hover_delay_ms",     "Hover delay (ms)",                     Int{min:0,max:5000}, i(300), "Stillness before ruster.on('hover') fires (0 disables)");
  add("mouse", "double_click_ms",    "Double-click window (ms)",             Int{min:50,max:2000}, i(400), "Max delay between clicks to count as double/triple");
  add("mouse", "wheel_lines",        "Lines per wheel notch",                Int{min:1,max:20}, i(3), "Scroll step");
  add("mouse", "tui_capture",        "TUI captures the mouse",               Bool, b(true), "False lets terminal text-selection win");
  add("mouse", "right_click_menu",   "Right-click context menu",             Bool, b(true), "Disable to repurpose Right-click via Lua");
  ```
- [ ] In `crates/ruster-tui/src/mouse.rs`, at top of `handle_mouse_event`, early-return if `!app.config.mouse.enabled`.
- [ ] Tests:
  - `config_defaults_mouse_section_present`
  - `schema_includes_mouse_entries`
  - `mouse_disabled_early_return`
- [ ] Run `cargo test --workspace` — green.
- [ ] Commit: `feat(config): add [mouse] section (enabled, hover_delay_ms, double_click_ms, wheel_lines, tui_capture, right_click_menu)`.

## Task 18: Documentation updates

**Files:** `docs/config-reference.md`, `docs/keybindings.md`, `docs/lua-api.md`

**Goal:** Update public docs to match the new surface.

**Steps:**

- [ ] `docs/config-reference.md`: add a `## Mouse` section under existing top-level headings, listing the 6 fields with defaults and a one-line description each.
- [ ] `docs/keybindings.md`: add a `## Mouse` table with rows: Left-click → move, Alt+Left → add cursor, Ctrl+Left → word, double → word-select, triple → line-select, drag → visual/region, wheel → scroll 3 lines, Shift+wheel → horizontal scroll, Ctrl+wheel (GUI) → zoom font, right-click → context menu, hover (300ms) → Lua hook.
- [ ] `docs/lua-api.md`: document `ruster.on("mouse_down"|"mouse_up"|"mouse_drag"|"mouse_move"|"mouse_wheel"|"hover", fn)`, `ruster.mouse.get/set`, `ruster.context_menu.add(zone, item)`, the payload schema from spec §Lua API (MouseEvent shape with `col/row/button/kind/mods/zone/wid/offset/line/col_in_line`).
- [ ] Verify no broken markdown links by `grep -rn '\.md)' docs/`.
- [ ] Commit: `docs(mouse): add [mouse] config, mouse gesture table, Lua mouse surface`.

## Task 19: Verification surfaces (eight `docs/verification/mouse-*` files)

**Files:** `docs/verification/mouse-{click,drag-visual,wheel-scroll,right-click-menu,hover-popup,double-click-word,chrome-tab-click,tui-right-click}-{gui.png,tui.txt}` (16 files), plus `docs/verification/mouse-{click,drag-visual,wheel-scroll,right-click-menu,hover-popup,double-click-word,chrome-tab-click,tui-right-click}.md` per-surface README.

**Goal:** Capture each user-visible surface in both backends per Phase 10.

**Steps:**

- [ ] Add 8 entries to `scripts/verify-capture.sh`'s surface table (extend the existing list, not rewrite).
- [ ] For each of the 8 surfaces: write a `KEYS`/init-script that drives the surface, capture TUI text via `tmux capture-pane`, capture GUI PNG via the `gui-check` skill.
- [ ] Each surface README documents the drive script, expected behavior, and references the related task.
- [ ] Run `just verify mouse-click-position` (and the other 7) — confirm all green.
- [ ] Commit: `verification(mouse): capture 8 surfaces × 2 backends = 16 artifacts`.

## Task 20: Final wiring & regression sweep

**Files:** all touched

**Goal:** Verify the full matrix before declaring done.

**Steps:**

- [ ] `cargo build --workspace` — zero warnings.
- [ ] `cargo test --workspace` — every test green.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- [ ] Confirm `App::handle_mouse_event` is now exactly 1 line (delegation) by `wc -l` on the relevant snippet.
- [ ] Confirm `app.rs` line-count growth < 200 lines (`git diff --stat`).
- [ ] Verify spec Definition of Done checklist:
  - [ ] `crates/ruster-render/src/mouse.rs` exists
  - [ ] `Renderer::poll_mouse`, `cell_metrics`, `set_pointer` exist with default impls
  - [ ] `RaylibRenderer` implements all three
  - [ ] `App::handle_mouse_event` dispatches; existing Alt+Left test passes
  - [ ] 8 verification surfaces captured
  - [ ] docs/config-reference.md, keybindings.md, lua-api.md updated
  - [ ] `app.rs` growth < 200 lines
- [ ] Commit: `chore(mouse): definition-of-done sweep (zero warnings, all tests green)`.

---

## Self-review

**Spec coverage:**
- §Goals: ✓ Tasks 1, 5, 7-9, 11, 12, 15, 16
- §Architecture (layering, trait changes, single dispatcher): ✓ Tasks 1, 2, 3
- §Backend trait changes: ✓ Task 2 (defaults), Task 10 (Raylib impl)
- §MouseEvent type: ✓ Task 1
- §App::handle_mouse_event dispatcher: ✓ Tasks 3, 4, 7, 8, 9, 16
- §Hit-test zones: ✓ Task 5
- §Per-zone handlers (buffer/gutter/chrome/float): ✓ Tasks 7, 14
- §Drag semantics (mode-aware Neovim/Emacs): ✓ Task 8
- §Wheel: ✓ Task 9 (logic), Task 10 (GUI zoom)
- §Double/triple click: ✓ Task 7
- §Hover: ✓ Task 16
- §Cursor-shape: ✓ Task 12
- §Context menu (FloatKind::Menu): ✓ Task 13
- §Data flow: implicit (Tasks 3, 10, 11)
- §File layout: ✓ all 16 file rows above
- §Configuration: ✓ Task 17
- §Lua API (mouse_*, hover, context_menu, mouse.{get,set}): ✓ Task 15
- §Error handling (headless, GUI panic, TUI resize, Lua throw, OOB): Tasks 2 (defaults), 10 (panic-safe), 4 (resize clears state), 15 (pcall), 2 (clamping to viewport in TUI; Task 10 for GUI)
- §Testing strategy: ✓ Tasks 1, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17
- §Risks (table): mitigated per-task (TUI capture opt-out → Task 17; `vw` surprise → Task 7 double/triple first; TUI mouse-up loss → Task 13; ScriptedRenderer brittleness → Task 2 defaults; GUI hot-loop → Tasks 9, 16 throttle; HiDPI fractional cells → Task 10 `.floor()`)
- §Open questions: question 1 (whitespace) → Task 6 default to whitespace via `CursorSet::select_word`; question 2 (cross-window drag) → Task 8 explicit "no, drag stays in originating window"; question 3 (hover only buffer zone) → Task 16 explicit
- §Definition of done: ✓ Task 20 explicit checklist

**Placeholder scan:** No "TBD", "TODO", "implement later", "fill in details", "add appropriate error handling", "similar to Task N", or "write tests for the above" remains. Every step is a concrete, runnable action.

**Type consistency:** `MouseEvent`/`MouseKind`/`MouseButton`/`PointerKind` defined in Task 1 and used unchanged through Task 20. `MouseState`/`ClickTracker`/`HoverState`/`DragState`/`MenuRegistry`/`ResizeState`/`MenuItem`/`Zone`/`HitZone`/`ChromeKind`/`FloatKind` defined in Tasks 4-13 and referenced by exact name downstream. `Action::SelectWord/SelectLine/SetMark/ResizeWindow` introduced in Task 6 and consumed in Tasks 7, 8, 14. `set_pointer_for_zone` (Task 12) and `set_zone_cursor` (Task 7) are different things — Task 7 has no such method; only Task 12 names it `set_pointer_for_zone`. ✓ no collision.

**Drift inline noted:** `self.is_gui` field added in Task 4 (does not yet exist on `App`); `ruster-render/src/mouse.rs` is a new file (Task 1); `FloatKind::Menu` is a new variant (Task 13); `Action::SelectWord/SelectLine/SetMark/ResizeWindow` are new variants (Task 6); `CursorSet::select_word/select_line/set_region` are new methods (Task 6).
