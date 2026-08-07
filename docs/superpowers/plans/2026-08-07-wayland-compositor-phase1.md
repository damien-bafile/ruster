# Phase 1 — Shell layout

**Status:** tasks 1-7 complete, 2026-08-07. Not verified on hardware — see below.

**Goal (from the design spec):** the i3 container-tree with editor-frames;
workspaces; split/focus/resize/swap/floating; editor buffers as leaves;
statusline reflects tree state.

## Where Phase 0 left it

`ruster-shell` is a flat `Vec<ClientWindow>` ordered by map time, with a
`workspace: u32` counter that nothing reads but the statusline label. The
compositor renders exactly one window — `shell.focus` — fullscreen from the
origin, and `crates/ruster-compositor/src/input.rs` assumes that when it maps a
pointer position onto a surface (`pointer_focus` tests containment against the
whole output).

So Phase 1 is not an addition to the Phase 0 model; it replaces it. Three
things currently assume "one fullscreen window" and each has to learn otherwise:
rendering, pointer hit-testing, and the initial `xdg_toplevel` configure, which
sizes every client to the whole output.

## Tasks

### 1. The container tree and its geometry

`ruster-shell` gains an arena-backed tree: `Node::Split { layout, children,
ratios }` and `Node::Leaf(WindowId)`, indices rather than `Rc<RefCell<_>>`. One
root per workspace.

The load-bearing part is `layout(root, rect) -> Vec<(WindowId, Rect)>`: pure,
total, and unit-testable without a display. Everything downstream consumes its
output, so it is where the tests go.

Insert and remove carry the fiddly invariants — a removed leaf whose parent is
left with one child collapses that parent — and those are the ones that rot
silently, so they get tests naming the invariant.

### 2. Render every leaf, not just the focused one

`collect_render_elements` walks the layout instead of taking `shell.focus`.
Each toplevel is configured to its leaf rect rather than the output size, which
means the initial-configure path in `compositor.rs` stops being a special case.

### 3. Pointer hit-testing against the tree

`pointer_focus` currently answers "is the pointer on the output". It becomes
"which leaf is the pointer over", returning that leaf's origin so surface-local
coordinates stay correct. This is also what makes click-to-focus meaningful
with more than one window on screen.

### 4. Directional focus, split, swap, resize

`focus(Direction)`, `split(Layout)`, `swap(Direction)`, `resize(Direction, f32)`.
Pure tree operations; keybindings arrive with the Phase 2 control plane, so
Phase 1 wires them to defaults only.

### 5. Workspaces that hold windows

A window belongs to a workspace. Switching shows that workspace's tree and hides
the rest — which is the point Phase 0's counter never reached. `move to
workspace N` comes with it.

### 6. Floating

A per-workspace list drawn above the tiled tree, with its own geometry. Kept
last because everything else has to be right first.

### 7. Statusline reflects the tree

Workspace number, the focused window's title, and the focused container's layout
direction — the three things you cannot infer from the screen once there is more
than one window.

## What landed

All seven tasks. The container tree and its geometry (task 1), rendering every
leaf (2), pointer hit-testing against it (3), directional focus/split/swap/resize
(4), workspaces that hold windows (5), floating (6), and a statusline that
reports the tree (7).

Two things are deliberately not done, both deferred to Phase 2 with the control
plane: **none of the tree operations are bound to a key** — `focus`, `split`,
`swap`, `resize`, `toggle_floating` and `move_to_workspace` are public methods
with tests and no keybinding — and **there is no server-side decoration**, so
tiled windows are told `Tiled{Left,Right,Top,Bottom}` and draw their own borders
if they insist.

## Constraints

- `cargo clippy --workspace --all-targets -- -D warnings` clean, and the same
  with `--features ruster-compositor/udev`.
- Every tree operation is unit-tested without a display. The Phase 0 lesson is
  that a display-dependent check marked "not run" hides defects for weeks, so
  the tree is built to be testable in isolation and the render path is verified
  nested afterwards.
- The nested winit path must keep working at every commit; it is the only way to
  see any of this before a VT trip.

## Hardware verification, deferred

Phase 0 left three rows unproven on real hardware: `Ctrl+Alt+F<n>` VT switching
(the escape hatch), the session suspend/resume cycle, and the pointer under
libinput. Phase 1's pointer work touches the third. All three are batched for a
single DRM trip after this phase, at the user's preference.
