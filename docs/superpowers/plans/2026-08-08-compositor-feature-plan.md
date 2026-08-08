# Compositor feature plan — after Phase 2's first half

## Where this actually stands

Phase 0 and Phase 1 are done and verified nested. Phase 2 is half done: actions
carry arguments and run through one `dispatch`, which-key and the welcome frame
report the binds really in force, windows have focus borders, and the compositor
finally reads the user's theme. Everything below is what is left, ordered by
what stops the thing being usable rather than by what is interesting.

Two sources ground this list. The first is `docs/compositor.md`, whose ⛔ rows
are the claims nobody has tested. The second is the hardware log: `foot` prints
a diagnostic for every protocol it wanted and did not find, and that list is a
free, honest inventory of what a real client misses. Nothing here is speculative
— each item is either a matrix row, a client complaint, or a `TODO` in the tree.

---

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

| Gap | What breaks without it | Size |
| :--- | :--- | :--- |
| **XKB layout from config** | The keymap is hardcoded. Any non-US layout is simply wrong, and there is a `TODO(next phase)` at `compositor.rs:442` saying so. This is the one that makes the compositor unusable for a whole class of user | S |
| **`xdg_popup` positioning** | Client menus and tooltips have no parent-relative placement — `shell.rs:53` tracks no popups at all. Right-click menus land wherever | M |
| **Primary selection** | Middle-click paste does not work anywhere | S |
| **Decoration manager** | Clients use CSD unconditionally, so every window draws its own titlebar *inside* a tile that already has a border. Announcing server-side decorations is what makes the tiling look deliberate | S |
| **`xdg-activation`** | No focus-stealing protocol, so `bell.urgent` falls back to colouring margins red, and a client asking for attention cannot get it | S |
| **`cursor-shape-v1`** | Clients ship their own cursor bitmaps instead of naming a shape; the compositor already draws a software cursor and could serve them all | S |
| **Fractional scaling** | Clients cannot render at the real scale, so text is resampled on any non-integer output | M |
| **`text-input` / IME** | No input method at all. Blocks CJK entry outright | L |

Recommended order: XKB first (it is a correctness bug wearing a feature's
clothes), then decorations + primary selection + activation together (three small
handler impls), then popups, then scaling. IME last — it is a project.

---

## Tier 3 — Editor-in-desktop (spec Phase 3)

The premise of the whole thing, and untouched: `ruster-compositor` imports
nothing from `ruster-core` today, and the editor frame draws a hardcoded welcome
buffer.

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

- **`wlr-screencopy`** — would have made this entire session's verification a
  `grim` call instead of a keybind, a blit and a PNG encoder. Worth doing for
  testing alone, and it is what lets OBS and every screenshot tool work.
- **Layer-shell** — bars, notification daemons, launchers. The single protocol
  that unlocks the most third-party software.
- **XWayland** — until then, no Electron app, no Steam, no legacy GTK2.
- **Session restore** — Tier 1.4 plus relaunching what was there.
- **Animations** — last, deliberately.

---

## What I would actually do next

**1.1 (live Lua API), then 2's XKB item, then 1.2/1.3 together.**

The Lua API is the biggest gap between the spec and the tree, and everything in
Tier 1 gets easier once a command queue exists — the mini-buffer becomes a text
box that pushes onto it, and the chord machine becomes a resolver that feeds it.
XKB jumps the queue because a hardcoded keymap is a bug, not a missing feature,
and it is cheap.

Tier 2's small handlers are a good batch for a session where the hardware is
unavailable: they are pure protocol wiring, they are individually testable
nested, and each one removes a line from `foot`'s complaint list — which is a
verification signal that costs nothing to read.

Tier 3 is where the project's actual thesis lives, and I would not start it until
Tier 1 is finished, because the leaf-type change in 3.1 touches every consumer of
the tree and is much less pleasant to do while the control plane is still moving.

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
