# Phase 3 — Editor-in-desktop

**Status:** not started. `ruster-compositor` declares `ruster-core` in its
`Cargo.toml` and imports nothing from it — `grep -rn ruster_core
crates/ruster-compositor/src` returns nothing at all. `Chrome::draw_editor_frame`
(`chrome.rs:293`) exists, is called from three tests and nowhere else, and lost
its last real caller when the hardcoded welcome buffer was deleted.

**Goal (from the spec):** multi-buffer editing, terminal leaf, LSP inside a
tile, xdg-desktop-portal integration. Or, in the spec's own words for the
project as a whole: "workspace trees hold **both** external client windows and
Ruster editor buffers as leaves."

> **On the evidence in this plan.** Three load-bearing claims were re-checked
> against the source before filing: `send_frame_callbacks` (`render.rs:339`)
> really does serve only the focused client, the compositor really imports
> nothing from `ruster_core`, and `layout_text` (`atlas.rs:287`) really does
> shape with cosmic-text's proportional default.
>
> A fourth was **wrong**. This plan originally called atlas exhaustion silent;
> `atlas.rs:186` actually does `tracing::warn!(?c, "glyph atlas is full; glyph
> will not be drawn")`. That inverts the risk rather than removing it — a
> warning per missing glyph per frame is a flood, and this project has already
> lost a log to exactly that failure (11,746 `DeviceInactive` lines in five
> minutes, 96% of a 2MB file, on the only diagnostic channel a VT boot has).
> Stage 5's mitigation is therefore a rate-limited counter, not a first warning.

---

## The central decision: do not change the leaf type

The obvious reading of Phase 3 is that `Node::Leaf(WindowId)` becomes
`Node::Leaf(Client(WindowId) | Buffer(PaneId))`, and that this is the change
with the widest blast radius in the project. The second half is true. The first
half is avoidable, and avoiding it is the single largest de-risking move
available.

Three facts, from the source rather than from taste.

**The tree does not care what a leaf is.** Every method in
`crates/ruster-shell/src/tree.rs` treats the payload as an opaque token: `find`
compares it, `walk` pushes it, `layout_into` pairs it with a rectangle, `swap`
exchanges two of them. The only requirements are `Copy`, `PartialEq` and — for
the tests — `Ord`. `Workspaces` is the same. Adding a variant would tell the
tree something it has no use for.

**Every compositor consumer that needs a client already handles a leaf that
isn't one.** `reconfigure_tiles` does `let Some(toplevel) = ... else { continue }`
(`compositor.rs:432`). `collect_render_elements` does the same (`render.rs:213`).
`surface_under` returns `None` when the leaf is not in `mapped` (`input.rs:126`).
These were written to be defensive about a window that is mid-teardown, and that
shape is exactly what a non-client leaf needs.

**`tile_under` and `geometry` are already the same list.** `tile_under` takes
`&[(WindowId, Rect)]` and its only caller passes `self.geometry()`, which is
`workspaces.layout(...)`. They cannot disagree unless someone introduces a
second source. A side table that never touches the layout keeps that guarantee
for free; a leaf-type change re-opens the question at every one of the twenty-odd
sites below. The compositor has had the pointer land somewhere other than where
it looked exactly once, and it was because two functions computed the same
rectangle two ways.

So: **an editor pane gets an ordinary `WindowId` from `ShellState::add_window`,
goes into the tree as `Node::Leaf`, and the compositor keeps a
`panes: HashMap<WindowId, EditorPane>` beside `toplevels`.** A leaf is a client
if `toplevels` has it and a pane if `panes` does; both maps are already keyed the
same way and the invariant ("exactly one of the two") is one assertion.

If, after Stage 4, the type-level distinction still looks worth having, do it
then as a mechanical rename of `WindowId` to `LeafId` with no variant added. The
cost is 74 `WindowId(` construction sites and 94 tests across `ruster-shell`, and
it should be paid only once the behaviour is settled.

---

## Every consumer, and the order to change them in

This is the blast radius. Nothing in `ruster-shell` appears in it before Stage 4,
which is the point.

| Site | Today | Under a pane leaf |
| :--- | :--- | :--- |
| `ShellState::set_focus` `state.rs:49` | refuses an id not in `windows` | **the silent gate.** A pane must have a record or focus is rejected with no diagnostic |
| `ShellState::add_window` `state.rs:28` | allocates the id | unchanged — panes take ids from the same allocator, so collision is impossible |
| `CompositorState::focusable` `compositor.rs:193` | `mapped && is_visible` | must accept a pane: `(mapped ∪ panes)` |
| `update_keyboard_focus` `compositor.rs:154` | id → toplevel → surface | already resolves to `None` for a pane, and clearing the seat is the *correct* answer |
| `next_focus_after_unmap` `compositor.rs:523` | `max()` over `mapped` | a client unmapping beside a pane loses focus entirely. Route through `Workspaces::focus_for_active`, which reads the tree and already sees panes |
| `reconfigure_tiles` `compositor.rs:430` | `continue`s past non-clients | must also hand the pane its new `(cols, rows)` |
| `dispatch` `compositor.rs:315` | acts on `shell.focus` | `Focus`/`Swap`/`Resize`/`Split`/`MoveToWorkspace` are pure tree ops, unchanged. `ToggleFloating` on a pane is refused with a mini-buffer message |
| `tile_under` `input.rs:76` | containment over `geometry()` | **unchanged.** This is the lockstep guarantee |
| `toplevel_under` `input.rs:105` | filters on `mapped` | click-to-focus silently refuses a pane. Becomes `leaf_under` |
| `surface_under` `input.rs:119` | `None` when not mapped | correct as written: the pointer *leaves* the client when it crosses into a pane |
| `on_keyboard_key` `input.rs:275` | `Resolved::None => Forward` | the one new arm: feed the key to the pane and `Intercept` |
| `collect_render_elements` `render.rs:211` | `continue`s past non-clients | draws the pane instead |
| `send_frame_callbacks` `render.rs:339` | focused surface only | **breaks catastrophically** — see Stage 0.1 |
| `Chrome::draw_window_borders` `chrome.rs:347` | outlines every rect | unchanged, and a focused pane already looks focused |
| `persist::Session::capture` `shell/persist.rs:118` | `app_of: Fn(WindowId) -> App` | a pane has no command line; see Stage 1.4 |
| `Tree::rebuild` `tree.rs:467` | drops unresolvable leaves | **silently drops panes** from a restored layout unless taught otherwise |

Order: `ShellState` first (id allocator and focus gate), then focus, render,
input, reconfigure, persistence. Each of the six leaves `cargo test --workspace`
green.

---

## Stage 0 — prerequisites, each worth doing on its own merits

### 0.1 Frame callbacks to every visible client, not just the focused one

`send_frame_callbacks` resolves `focus` to a surface and sends to that one. The
moment focus is a pane the resolution is `None` and **no client on screen
receives a frame callback at all** — every window freezes, with no error and
nothing in the log.

This is not a Phase 3 bug; it is a live one. An unfocused terminal running
anything that draws by itself gets only the 1s backstop today. Send to every
mapped, visible toplevel.

Check by running something animated in an unfocused tile and watching it keep
moving. Do this first: a pane that takes focus before this lands looks exactly
like the compositor having crashed.

### 0.2 Monospace metrics in the atlas

`layout_text` shapes with `Attrs::new()`, whose cosmic-text default family is
sans-serif. Chrome does not care; a text grid does — column alignment, cursor
placement, the gutter and click-to-position all assume a fixed advance.
`layout_text` needs a family parameter and the atlas needs
`cell_metrics(font_px) -> (advance_px, line_h_px)`.

Testable with no display: lay out `"iiii"` and `"WWWW"` at the same size in the
monospace family and assert equal widths. A guard that can actually fail.

### 0.3 One measurement, before any rendering decision — **done, and it decides Stage 2**

Measured with `RUSTER_BENCH_GLYPHS=n`, which makes `Chrome` emit *n* real atlas
glyph quads per frame in a pane-shaped grid. Nested, 1873x1334, average frame
time over one-second windows.

**Release build:**

| extra glyph quads | avg frame ms |
| ---: | ---: |
| 0 | 4.73 |
| 2,000 | 4.69 |
| 3,200 (an 80x40 pane) | 4.69 |
| 5,000 | 4.69 |
| 10,000 | 4.84 |
| 50,000 | 15.96 |
| 200,000 | 73.85 |

Flat to ten thousand, and an 80x40 pane is 3,200. Several panes at once would
still sit in the flat region. The cliff is somewhere between 10k and 50k, which
is an order of magnitude past anything Phase 3 will ask for.

The flatness was checked rather than believed: a harness that was not really
emitting the quads would look identical, so the load was pushed to 50k and 200k
until the cost appeared. It does, proportionally.

**So: implement option A, per-glyph quads with ids re-keyed to `(pane, row,
col)`, and do not build option B.** Per-row CPU rasterization was designed here
to solve a problem the numbers say does not exist. If a future pane somehow
needs 50,000 cells, this section is why B was written down.

**One caveat worth carrying.** The same measurement on a *debug* build:

| extra glyph quads | avg frame ms (debug) |
| ---: | ---: |
| 0 | 4.66 |
| 500 | 4.66 |
| 2,000 | 9.88 |
| 5,000 | 24.2 |

A debug build degrades from 2,000 quads and misses the 60Hz budget at 5,000. So
an editor pane will feel sluggish under `cargo run` while being perfectly fine
in release — and a performance complaint during Phase 3 development should be
re-checked in release before anyone optimises anything.

## Stage 1 — a pane that exists, takes focus, and draws nothing

The whole blast radius, paid before any text-rendering work exists to confuse it.

**1.1 The pane.** `EditorPane { title, cols, rows }` and `panes: HashMap<WindowId,
EditorPane>`. `Action::NewPane` inserts one with the same two calls
`new_toplevel` makes (`shell.rs:32-42`).

**1.2 Focus.** The `focusable` / `update_keyboard_focus` /
`next_focus_after_unmap` changes above. A focused pane means the seat keyboard
has no focus and neither clipboard has a client — both fall out of existing code,
and both are right.

**1.3 Drawing.** `Chrome::draw_editor_frame` at the pane's rect, translated with
`ChromeBatch::translate_since` — the mechanism written for exactly this and
unused since the welcome frame went. A titled empty frame with a focus border.
Usable as a spacer tile, arrangeable with every existing keybind, and it proves
the focus plumbing end to end.

**1.4 Persistence, or a loud refusal.** `Session::capture` calls `app_of` for
every leaf, and `Tree::rebuild` drops leaves it cannot resolve. Without this the
first restart after Stage 1 silently loses panes from a saved layout. Either
`App` grows a third shape now, or a workspace containing a pane refuses to be
captured and says so. The former is two lines in a hand-written format that
already has an `app`/`title` pair; take it.

**Done when:** a keybind makes a pane, focus moves between it and a client, the
border follows, splitting and resizing work, and the client beside it is still
redrawing.

---

## Stage 2 — a real buffer in the pane, read-only

**2.1 Documents.** `BufferStore` on `CompositorState`; `EditorPane` gains
`buffer`, `cursors`, `scroll_top` — which is `ruster_core::windows::Window`
field for field, minus the height the layout already knows.

**2.2 The view.** Build a `ruster_render::WindowView` compositor-side: `lines`
from `Buffer::line_to_string`, `gutter` from `gutter_view`, `cursor` from
`CursorSet`. `ruster-render` is already a compositor dependency, so this costs
nothing new — and it is the same view model the TUI and raylib backends consume,
so a bug in the compositor's pane is comparable against two working
implementations.

**2.3 Drawing it.** Three options; pick by the number from 0.3.

- **A — per-glyph quads, ids keyed by cell.** `Chrome::glyph_id(index)` hands out
  ids by position in the batch, so scrolling one line renumbers every glyph in
  the frame. Re-key to `(pane, row, col)`. Nearly free, because the path exists.
- **B — one CPU-rasterized texture per text row,** uploaded with `import_memory`,
  cached and invalidated per row — the mechanism `Chrome::atlas_texture` and
  `cursor_element` already use, including the `Box<dyn Any>` downcast that keeps
  them renderer-agnostic across `GlesRenderer` nested and `MultiRenderer` on DRM.
  ~40 elements per pane instead of ~3,200, and a row is the natural damage unit
  for an editor: a keystroke dirties one.
- **C — render to texture in GL.** Fewest uploads, but binds the pane path to a
  concrete renderer and re-opens the seam `Chrome` deliberately hides. Rejected.

**Recommendation: implement A, measure, move to B if the number says so.** B's
design is written down here so that move is a port and not a redesign.

**Done when:** a pane shows a file, scrolls, and re-flows when the tile resizes —
verified by a number, not by eye: log the first visible line and the cell count
and check they match what was asked for.

---

## Stage 3 — editing

**3.1 Keys.** In the filter closure, where the last arm returns
`FilterResult::Forward`, a pane-focused branch converts the key to
`ruster_core::key::KeyEvent` and intercepts — recording the keycode in
`intercepted` so the release is swallowed too, exactly as the mini-buffer arm
does.

Placement is not negotiable: **after** VT switching, the mini-buffer, chord
resolution and the quit hatch. A WM keybind must work while an editor pane has
focus for the same reason it works while a client does, and an editor that
swallowed `M-S-q` on a DRM boot would be the worst bug in the project.

**3.2 Editing.** `VimState::handle(key, &dyn EditorView) -> Vec<Action>` and
`EditSession::new(...).execute(action)`. The pane implements `EditorView` over
its own buffer and cursors. `EmacsState::handle` is the same shape later.

**3.3 Key repeat.** Holding `j` moves one line. Repeat is a client-toolkit
behaviour today; an intercepted key has none. Needs a calloop timer using the
delay/rate already parsed into `KeyboardConfig` (`lua.rs:84-87`). Easy to forget
until someone tries to use it.

**3.4 The clipboard.** `VimState::new` constructs `arboard::Clipboard::new().ok()`.
On a DRM boot there is no display for arboard to reach, so it is `None` and
yank/paste falls back to an in-process buffer — meaning the editor's clipboard
and the Wayland clipboard are two different things. Mirror into the compositor's
`DataDeviceState`, which is right there and now focus-tracked.

**3.5 Pointer.** `TextArea::cell_at` maps a cell to a buffer position; the
pixel→cell step is `cell_metrics`. `surface_under` already returns `None` over a
pane, so the client correctly sees a pointer leave.

**Done when:** a file opens in a tile, is edited, undone and written, with
`M-S-q` still quitting throughout.

---

## Stage 4 — multi-buffer, and persistence that means something

Several panes over one `BufferStore`; two panes on the same document scroll
independently because cursor and scroll live on the pane. `:e`, `:b`, `:w`
through the existing mini-buffer, resolving through `Action::from_name` so the
prompt cannot grow a vocabulary the keymap lacks.

Persistence becomes real: a pane leaf saves its path, cursor and scroll — which
`ruster_core::session` already does for the editor and is the pattern to copy,
not the type to reuse, for the same reason the workspace format was
hand-written.

This is the first stage that could justify the leaf-type change, and the first
where `ruster-shell` is touched at all.

---

## Stage 5 — syntax

`SyntaxEngine::new(text, ext)` per document, `reparse_with_edits` fed from
`Buffer::take_edits`, gated on `Buffer::revision` — the mechanism that exists
precisely because re-parsing every frame cost 107 ms on a 10k-line file.
`styled_lines()` drops straight into `WindowView::lines`.

**The atlas is the risk here, not the parser.** Glyphs are keyed by
`(size, colour, char)` and packed shelf-wise into 1024²; once full, new glyphs
return `Glyph::EMPTY` and are not drawn. A theme with twenty syntax colours
multiplies the cell count by twenty.

`atlas.rs:186` does warn — so the failure is not silent, it is *loud in the worst
way*: one warning per missing glyph per frame, which is a flood on the only
diagnostic channel a VT boot has. Before this stage: replace the per-glyph
warning with a rate-limited counter (fill percentage and a count of dropped
glyphs), test that it reports, and decide between growing the texture and
tinting pane text at draw time instead of baking colour. Tinting is what the
atlas comment already rules out for chrome, for a reason that still holds — so
growing is the likely answer.

---

## Stage 6 — the terminal leaf

Spawn `foot` and treat it as a client. `Action::Spawn` already exists, already
records provenance for the session file, and already works on hardware.

The alternative — `ruster-terminal` in a pane — means the display server hosts a
VT parser for a job it is already hosting a Wayland client to do. The only thing
it buys is a terminal the editor's own buffer commands can address, and that is
not worth a second terminal emulator inside the compositor process. **This is a
one-line stage, and the spec line it satisfies is satisfied honestly.**

---

## Stage 7 — LSP inside a tile

`ruster-lsp` has no async runtime: `Command::new` plus a `std::thread::spawn`
reader into an mpsc channel, which drains cleanly from a calloop iteration. And
`ruster_tui::lsp_state::LspState<A>` is *already* the extracted, generic surface
— it depends on `ruster-core` and `ruster-lsp` and nothing else. Moving it to a
shared crate is the cheapest extraction available in `ruster-tui`.

What follows is diagnostics as signs, hover in a float, go-to-definition opening
a document in a pane. **Be realistic: this stage is where Phase 3 will slip.**
Worth planning for, not worth promising.

---

## What `ruster-tui` actually gives you

Less than it looks, and the useful part is not in `ruster-tui`.

**Reusable as-is:** `ruster-core` entirely — `Buffer`, `Document`, `BufferStore`,
`CursorSet`, `UndoStack`, `EditSession`, `VimState`, `EmacsState`,
`key::KeyEvent`. Its dependencies are ropey, unicode-segmentation, thiserror and
arboard; nothing terminal-shaped. `ruster-render`'s view model — `WindowView`,
`StyledLine`, `TextArea`, `gutter_view` — which the compositor already depends
on. `ruster-syntax`. `ruster-lsp`.

**Reusable with a move:** `lsp_state`, `sidebar`, `dired`, `quickfix` — modules
whose own doc comments record that they were already extracted from `App` along
boundaries that existed.

**Not reusable:** `app.rs`. 13,061 lines, and the piece you want is the
window-view builder between `app.rs:4164` and `app.rs:4491` — about 350 lines
that read `self.syntax`, `self.git_gutter`, `self.lsp`, `self.flash`,
`self.dired`, `self.sidebar`, `self.terminals`, `self.config`, `self.lua` and
`self.renderer.viewport_cells()`. Extracting it means extracting all of those.

**Not reusable, and a trap if attempted:** `App` itself. It computes window
rectangles for the *whole screen* and divides them with its own `WindowTree` — so
an `App` in a tile gives you a second tiling tree inside the first, and the
spec's whole claim is that buffers and clients are peers in *one* tree. And
`App::new` starts a Lua runtime, background git threads, LSP servers and config
reads; running that inside the process that is the display server is a category
error.

The honest summary: **the editor that runs in a compositor tile is
`ruster-core` + `ruster-render` + `ruster-syntax`, driven by roughly a thousand
lines of new compositor code, and `ruster-tui` contributes almost nothing
directly.** That sounds worse than it is — those three crates *are* the editor;
`ruster-tui` is a terminal application built on them.

---

## Input routing when focus is a buffer

Keyboard focus means "a `wl_surface`" in exactly one place:
`update_keyboard_focus`, which maps `shell.focus` through `toplevels`. For a
pane that resolves to `None`, which clears the seat focus and hands both
clipboards to nobody — both correct, so the function needs no change beyond the
`focusable` gate.

The resulting precedence chain:

1. `Ctrl+Alt+F<n>` — the escape hatch, never configurable.
2. An open mini-buffer — takes every key.
3. A pending chord, or a complete WM binding.
4. `M-S-q` on a fresh key, whatever the config says.
5. **The focused editor pane, if there is one.** *(new)*
6. The focused client surface.

5 and 6 are the same slot seen from either side, which is what makes a pane a
peer of a client rather than a special case.

Three things do *not* fall out for free: frame callbacks (0.1), key repeat
(3.3), and click-to-focus (`toplevel_under` filters on `mapped`).

---

## xdg-desktop-portal

**It does not belong in this phase, and probably not in the next two.**

A portal is a D-Bus service that *clients* call. Nothing about editor-in-desktop
needs one — the editor pane has its own file picking and its own buffers. What
needs one is Firefox wanting to share a screen.

The price is not the D-Bus. The reference compositor-side backend needs
`wlr-screencopy-v1` (or `ext-image-copy-capture-v1`) plus PipeWire. This tree has
none of the three: `grep -rn "zbus\|dbus\|pipewire" crates/*/Cargo.toml` is
empty, and `screenshot.rs` says in its own module doc that the compositor
"implements no screencopy protocol". So "portal integration" is: implement
screencopy, add PipeWire, add an async D-Bus runtime to a compositor with no
async runtime, then write the portal. That is a phase, and a
*client-compatibility* one rather than an editor one.

One small piece is worth remembering: `org.freedesktop.impl.portal.Settings`
would let clients follow ruster's colour scheme, which the compositor already
reads from `config.lua`. Pure D-Bus, no PipeWire. Phase 4 polish, recorded as
such rather than smuggled in here.

**Recommendation: strike xdg-desktop-portal from Phase 3 and record it against
Phase 4, next to `wlr-screencopy` and layer-shell, where its dependencies live.**

---

## Risks

**The compositor works now. Almost everything here can break it.** That is why
the staging is shaped this way: no change to `ruster-shell` before Stage 4, and
the entire focus/input/render blast radius paid in Stage 1 while the pane still
draws nothing, so a regression has one obvious cause.

- **Frame callbacks (0.1) change how every client is paced.** A mistake is a
  stuttering or frozen desktop. Small change, observable check, must land first.
- **Frame time.** Measured (0.3): flat to 10,000 render elements in release, so
  element count is *not* a risk for a pane. Glyph-id churn under scrolling is
  still unmeasured, and CPU rasterization cost is now moot — option B is not
  being built.
- **Atlas exhaustion warns per glyph.** Not silent — flooding. See Stage 5.
- **`arboard::Clipboard::new()` runs inside the display server.** On a bare VT it
  is a library reaching for X11/Wayland from inside the thing that *is* Wayland.
  Construct once at startup, never on the render path.
- **Silent loss of restored layout.** `Tree::rebuild` drops leaves it cannot
  resolve, so a pane in a saved session vanishes without a word unless 1.4 lands
  with Stage 1.
- **Scope creep into `ruster-tui`.** The moment the compositor depends on it, it
  inherits ratatui, crossterm, tokio and a 13k-line god object into the display
  server. If a stage seems to need it, that is the signal to extract, not depend.
- **Two tiling trees.** Adopting `ruster_core::workspace::Workspace` brings
  `WindowTree` alongside `ruster_shell::Tree`. Use `BufferStore` and
  `EditSession` directly.

---

## What I would actually do first

**Stage 0 in full, then Stage 1, then stop and look at it.**

0.1 is a bug fix that happens to be a prerequisite. 0.2 is a bug fix wearing a
feature's clothes — code in a proportional font is not a rendering choice. 0.3 is
the measurement that decides Stage 2, and this project's own standard says to get
a number out of the program rather than argue about it.

Stage 1 is where the risk is, and it is worth landing *alone*: a pane that takes
focus, moves, splits, resizes, persists and draws an empty titled frame is a
complete, boring change that proves the hard part. If focus, input and
persistence survive a hardware boot with a pane in the tree, the rest of Phase 3
is ordinary work.

**Explicitly deferred:** xdg-desktop-portal (Phase 4). The `Node::Leaf` type
change (Stage 4 at the earliest, and only if the side table proves insufficient).
A compositor-hosted terminal emulator (never; spawn `foot`). Floating panes and
editor-side splits inside a pane — the compositor tree is the splitter, and
`ToggleFloating` on a pane is refused with a message. LSP (planned, not
promised). Adopting `ruster-tui::App` — not deferred, rejected.

---

## Definition of done

A tile holds a `ruster-core` buffer. It is opened from a keybind or the `:`
prompt, edited with the vim keymap, written to disk, syntax-highlighted, and
arranged with the same `focus`/`swap`/`resize`/`split` bindings a Wayland client
uses — on the same workspace, in the same tree, with the same focus border.
`M-S-q` still quits. A `foot` window beside it still redraws. The layout,
including which file was in which tile, survives a restart.

---

## Verification standard

Unchanged, and it has earned its place repeatedly:

- `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D
  warnings`, and the same with `--features ruster-compositor/udev`.
- Every pane operation — insert, focus, resize-to-cells, scroll clamp, key
  translation — unit-tested without a display.
- **Guards get mutation-tested.** The monospace-advance assertion, the atlas
  exhaustion counter and the "a pane can hold focus" test are all guards, and a
  green test that cannot fail is worse than none. The VT-switch test passed for
  months against an input the real code never produced.
- **Get a number out of the program.** Frame time at *n* glyph elements. The
  first visible buffer line against the cell count. Atlas fill percentage with a
  highlighted file open. None are answerable by eye.
- A ⛔ in `docs/compositor.md` means untested, never fine. Phase 3 adds rows;
  they start ⛔ and stay that way until someone presses a key.

---

## What this plan does not know

- Whether `arboard::Clipboard::new()` returns promptly or blocks when called from
  inside the compositor on a bare VT. Worth a timed log line before Stage 3.
- How large the atlas needs to be for a full screen of highlighted code under a
  real theme. Measurable once Stage 5 starts; guessing now would produce a number
  to defend.
