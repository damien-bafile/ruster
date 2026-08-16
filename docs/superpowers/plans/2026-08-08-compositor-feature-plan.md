# Compositor feature plan — after Phase 2's first half

> **Stale in its particulars, kept for its ordering.** Written 2026-08-08, when
> Tier 3 was untouched and the recommendation below was "1.1, then XKB, then
> 1.2/1.3". All of Tier 1 is done, all of Tier 2's small handlers are done, and
> Tier 3 — the project's actual thesis — is complete and verified, including a
> live `rust-analyzer` round trip. Updated 2026-08-14.
>
> **What is left of this plan (2026-08-16):** two Tier 2 items — fractional
> scaling (M) and `text-input`/IME (L) — and three of Tier 4: layer-shell,
> XWayland and animations. `wlr-screencopy` and session restore are done and
> verified. Everything finished has a row in `docs/compositor.md`, which now has
> no ⛔ rows and one ⚠️: every capture loses the EGL context, and that turns out
> to break the *next* capture rather than costing a frame.
>
> The judgement below is what survives, and it held up: XKB really was a
> correctness bug wearing a feature's clothes, the small protocol handlers really
> were a good batch for a session without hardware — wiring them found that the
> clipboard had been dead since the compositor was written — and Tier 3 really
> was better started once the control plane stopped moving.

## Tier 0 — Debt that blocks a hardware session

These are not features. They are the reasons a DRM boot is still an expedition.

### 0.1 The VT escape hatch, proven

Still zero VT switches in any log. `vt_switch_target` is implemented and
unit-tested and has never fired. Until `Ctrl+Alt+F2` is pressed *while the
compositor runs*, the session has no proven way out except a keybind that
assumes the compositor is healthy. Needs a person at the keyboard; everything
else in Tier 0 can ride along on the same boot.

### 0.2 DRM teardown leaves the display wrong

`Failed to restore previous state. Error: Invalid argument (os error 22)` from
smithay's atomic backend on every exit. The display has come back each time so
far, which makes this easy to keep ignoring — until the once it does not.

### 0.3 Pointer on DRM

Relative-motion handling has never executed on hardware; winit only ever emits
absolute. Nothing logs pointer events either, so a boot cannot even report
whether the mouse did anything — worth a `debug!` on motion before the trip, for
the same reason `dispatch` got one.

### 0.4 The screenshot's first real run

Written, compiles, unverified. Two specific things to check: that the PNG is not
black (the blit is asynchronous and the wait is untested), and that it is the
right way up (`Transform::Normal` is passed on the assumption that the DRM
output carries no transform, unlike winit).

**Update 2026-08-16.** Both capture paths are now heavily exercised *nested*, and
the orientation question turned out to be sharper than this predicted. The two
paths disagree about the flip and both are right: a screencopy client receives raw
pixels and applies the output transform itself, while the keybind path encodes the
PNG and must apply that transform on the way out. The winit output carries
`Transform::Flipped180`; the assumption above is that a DRM output carries none.
If that assumption is wrong, the keybind screenshot comes out inverted on hardware
and `grim` does not — which is the specific thing to look for, since a capture that
is upside down in only one of two paths is easy to write off as a fluke of the
other. Also unexplained: an `eglQuerySurface BAD_SURFACE` follows every capture
nested, survived but not understood, and hardware is a second data point on it.

---

## Tier 1 — Finish the control plane (Phase 2's back half)

### 1.1 A live `ruster.wm.*` API

The `Lua` VM is created inside `parse_config` and dropped before the compositor
exists, so every API call can only *record intent* — `ruster.wm.focus` is a stub
that warns and does nothing. This is the largest remaining gap between the spec's
Phase 2 and the tree.

`CompositorState<B>` is generic, so a Lua closure cannot capture it. The workable
design is a command queue: `Rc<RefCell<VecDeque<Action>>>` that the closures push
onto and the event loop drains once per iteration. It reuses the `Action` enum
and the `dispatch` that already exists, so the API surface is a thin translation
layer rather than a second implementation of every operation — which is the trap
`apply_action` fell into before it was deleted.

Then the query side: `tree_status`, `geometry` and the focused window become
readable from config, which is what makes conditional keybinds and status
scripting possible at all.

### 1.2 Chord prefixes and a which-key that waits

`resolve_wm_action` is stateless: there is no prefix, no pending chord, no
timeout. The overlay is permanently on screen because it has no notion of being
*triggered*. The editor has all of this — `LEADER_ROOT`, `leader_resolve`,
`leader_whichkey`, the animation and the timeout gate — but it is private inside
a 13k-line file and `ruster-compositor` does not depend on `ruster-tui`.

The reusable part is a ~50-line generic prefix-tree walker. Lift `LeaderNode<A>`
+ `resolve` + `children` + `whichkey_rows` into `ruster-render` and let each side
supply its own action enum and root table. `WhichKeyEntry`/`WhichKeyView` already
live there, and switching `draw_whichkey` to take a `&WhichKeyView` also buys the
title, the animation, and the `whichkey_key` accent — the compositor currently
draws key and description in one colour because it concatenates them into one
string.

### 1.3 A mini-buffer

The compositor has no `:` line at all — `Chrome` draws a statusline, an editor
frame and which-key, and nothing else. With 1.1 in place this becomes the natural
front end for it: type a command, resolve it through `Action::from_name`, hand it
to `dispatch`. `cmdline_bg`/`cmdline_fg`/`cmdline_accent` are already themed and
already reach the compositor now that it reads the real theme.

### 1.4 Workspace persistence

`crates/ruster-core/src/session.rs` is the pattern to follow, not the type to
reuse: `MAGIC`+`VERSION` header, preorder tree serialisation, all-or-nothing
parsing, FNV-1a-hashed filename. Two real differences. Its `LayoutSnapshot` is a
*binary* tree carrying one ratio, while `ruster_shell::tree::Node::Split` is
n-ary with a ratio vector. And a compositor leaf is a live client, so restoring
it means recording something re-launchable — a command line, which `Action::Spawn`
now has a parser for — rather than a `WindowId` that means nothing next boot.

---

## Tier 2 — The protocols real clients ask for

Every line here is something `foot` logged as missing on the hardware boot. They
are small individually and the difference between a demo and a desktop
collectively.

**Most of this tier is done.** The table below is kept with its outcomes rather
than deleted, because the recommended order at the bottom turned out to be right
and the reasoning is worth keeping. Verified against the globals the compositor
actually creates (`compositor.rs`), not against memory.

| Gap | What breaks without it | Size | State |
| :--- | :--- | :--- | :--- |
| **XKB layout from config** | The keymap is hardcoded. Any non-US layout is simply wrong, and there is a `TODO(next phase)` at `compositor.rs:442` saying so. This is the one that makes the compositor unusable for a whole class of user | S | ✅ done — and it was worse than described: the matcher recognised two hardcoded strings, so the Lua config could not bind *anything* |
| **`xdg_popup` positioning** | Client menus and tooltips have no parent-relative placement — `shell.rs:53` tracks no popups at all. Right-click menus land wherever | M | ✅ done, with real grabs — the first version was invisible, because `PopupManager::commit` does not send the initial configure |
| **Primary selection** | Middle-click paste does not work anywhere | S | ✅ done |
| **Decoration manager** | Clients use CSD unconditionally, so every window draws its own titlebar *inside* a tile that already has a border. Announcing server-side decorations is what makes the tiling look deliberate | S | ✅ done — note that tiled states alone do not stop a toolkit drawing a titlebar, and a client that keeps CSD keeps its shadow, which is what `surface_origin` exists for |
| **`xdg-activation`** | No focus-stealing protocol, so `bell.urgent` falls back to colouring margins red, and a client asking for attention cannot get it | S | ✅ done |
| **`cursor-shape-v1`** | Clients ship their own cursor bitmaps instead of naming a shape; the compositor already draws a software cursor and could serve them all | S | ✅ done |
| **Fractional scaling** | Clients cannot render at the real scale, so text is resampled on any non-integer output | M | ⬜ **open.** The scale *arithmetic* is there and tested; the `wp_fractional_scale_v1` global is not, so no client is ever told. Wants `wp_viewporter` alongside it, which is also absent |
| **`text-input` / IME** | No input method at all. Blocks CJK entry outright | L | ⬜ open, untouched |

Recommended order: XKB first (it is a correctness bug wearing a feature's
clothes), then decorations + primary selection + activation together (three small
handler impls), then popups, then scaling. IME last — it is a project.

Also absent, and never listed here because `foot` does not ask for them:
`wp_viewporter`, `presentation-time`, `relative-pointer` and `pointer-constraints`
(games and remote desktop need the pair), `virtual-keyboard`, `input-method`.

---

## Tier 3 — Editor-in-desktop (spec Phase 3) — **done**

Complete as of 2026-08-14; see `2026-08-08-compositor-phase3.md` for the stage
table and `docs/compositor.md` for the row per claim. The sections below are the
original scoping, left as written. Two of its predictions are worth keeping: the
leaf-type change was avoided entirely and never became necessary, and Stage 7 was
called "where Phase 3 will slip" — it did not, because by the time it arrived
`lsp_state` had already been extracted and carried the request/response
machinery with it.

### 3.1 A real buffer in a tile

Replace `welcome_buffer` with a `ruster-core` buffer rendered through the glyph
atlas. This is the first time a tree leaf is something other than a Wayland
client, so `Node::Leaf(WindowId)` has to become a leaf that is *either* a client
or a buffer — the single change with the widest blast radius in the plan, since
`geometry`, `tile_under`, focus and the renderer all case on it.

### 3.2 Terminal leaf

`ruster-tui` already has a terminal. In the compositor the honest version is a
client (`foot`) that the compositor spawns and treats specially, rather than a
second terminal emulator implementation.

### 3.3 LSP inside a tile

Follows 3.1 for free if the buffer leaf reuses the editor's document model.

---

## Tier 4 — Polish (spec Phase 4)

- ~~**`wlr-screencopy`**~~ — **done 2026-08-15.** It predicted its own value
  correctly: verification is now `grim` against the compositor's socket rather
  than a noise-masked screen diff driven by injected input. Three bugs, none of
  them the one that had been blamed for a week — a flush-after-sleep deadlock, a
  missing `zxdg_output_manager_v1`, and a flip that the client already applies.
- ~~**Layer-shell**~~ — **done 2026-08-16.** Bars, notification daemons and
  wallpapers can map. It was written, correct and invisible twice over: killed by
  `ensure_configured`, which raises the protocol error it sounds like it prevents,
  and then drawn *behind* ruster's own statusline, because the element list is
  front-to-back and the layers were emitted after the chrome. Nothing on this
  machine speaks the protocol, so `crates/ruster-bar` is a real client that does.
- **XWayland** — until then, no Electron app, no Steam, no legacy GTK2. The
  largest remaining unlock by some distance.
- ~~**Session restore**~~ — done, and confirmed on hardware: two windows put
  back at their positions in the tree, not merely respawned.
- **Animations** — last, deliberately.

---

## What I would actually do next

*Original recommendation, all of it now done: 1.1 (live Lua API), then 2's XKB
item, then 1.2/1.3 together. Tier 3 followed and did not slip. Kept below for the
reasoning, which held up.*

> The Lua API is the biggest gap between the spec and the tree, and everything in
> Tier 1 gets easier once a command queue exists — the mini-buffer becomes a text
> box that pushes onto it, and the chord machine becomes a resolver that feeds it.
> XKB jumps the queue because a hardcoded keymap is a bug, not a missing feature,
> and it is cheap.
>
> Tier 2's small handlers are a good batch for a session where the hardware is
> unavailable: they are pure protocol wiring, they are individually testable
> nested, and each one removes a line from `foot`'s complaint list — which is a
> verification signal that costs nothing to read.
>
> Tier 3 is where the project's actual thesis lives, and I would not start it
> until Tier 1 is finished, because the leaf-type change in 3.1 touches every
> consumer of the tree and is much less pleasant to do while the control plane is
> still moving.

### As of 2026-08-16

**A hardware pass first, then XWayland.**

Roughly forty rows of `docs/compositor.md` have only ever been proven nested.
Everything from Phase 3 onward — the editor in a tile, LSP, the launcher,
layer-shell, and the window-geometry fix — has never run on DRM. This project's
record is unambiguous about what that is worth: the screencopy path was believed
working for a week, the layer-shell bar logged success while being invisible, and
the nautilus offset was found by *looking at it*, after every headless test was
green. A VT session is the highest-yield hour available.

Then **XWayland**, which unlocks more third-party software than everything else
remaining put together. Fractional scaling after it, with `wp_viewporter`, since
the arithmetic is already in place and tested — only the globals are missing.
IME last, as it always was.

One thing genuinely unexplained: an `eglQuerySurface BAD_SURFACE` follows every
screencopy capture. It is survived rather than understood, and it is recorded that
way in the matrix rather than being written off.

## Verification standard

Unchanged, and it has earned its place three times this week:

- `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D
  warnings`, and the same with `--features ruster-compositor/udev`.
- Every tree/action operation unit-tested without a display.
- **Guards get mutation-tested.** A green test that cannot fail is worse than
  none.
- **Get a number out of the program.** The `swap`/`resize` "failures" were
  correct behaviour at a layout edge, and the focus border was confirmed by
  reading `(203,166,247)` out of a pixel rather than by looking at it. Both
  answers were unavailable by eye.
- A ⛔ in `docs/compositor.md` means untested, never fine.
