//! The container tree: how windows divide an output between them.
//!
//! An i3-style tree of splits and leaves. A split divides its rectangle among
//! its children along one axis; a leaf is one client window. Phase 0 had a flat
//! list and drew whichever window had focus fullscreen, which is the degenerate
//! case of this — one leaf under one root.
//!
//! Nodes live in an arena and refer to each other by index. The alternative,
//! `Rc<RefCell<Node>>`, makes every read a runtime borrow and every parent link
//! a `Weak`, and the operations here — remove a leaf, collapse its parent,
//! reparent the survivor — are exactly the ones that turn into borrow panics.
//! Indices make them ordinary code, at the cost of the arena never shrinking
//! (see [`Tree::compact`]).
//!
//! [`Tree::layout`] is the load-bearing function: it turns the tree plus an
//! output rectangle into the list of window rectangles to draw. Everything
//! downstream — rendering, pointer hit-testing, the size each client is
//! configured to — reads its output and nothing else, so it is where the tests
//! are aimed.

use crate::window::WindowId;

/// A rectangle in output-local pixels, origin top-left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Rect { x, y, w, h }
    }

    /// Whether `(px, py)` falls inside this rectangle. Left and top edges are
    /// inside, right and bottom are not, so tiled neighbours that share an edge
    /// never both claim the same pixel.
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w && py < self.y + self.h
    }
}

/// The axis a split divides its children along.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// Children side by side, dividing the width.
    Horizontal,
    /// Children stacked, dividing the height.
    Vertical,
}

/// A direction to move focus, a window, or a split boundary in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    /// The split axis this direction travels along. A split on the other axis
    /// has no boundary to move in this direction, which is what makes an
    /// across-the-grain resize a no-op rather than a surprise.
    pub fn axis(self) -> Layout {
        match self {
            Direction::Left | Direction::Right => Layout::Horizontal,
            Direction::Up | Direction::Down => Layout::Vertical,
        }
    }
}

/// Index into [`Tree`]'s arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub usize);

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    /// Divides its rectangle among `children` along `layout`.
    ///
    /// `ratios` has one entry per child and sums to 1.0. Keeping it beside the
    /// children rather than on each child means a resize touches one node, and
    /// the "must sum to 1" invariant has one place to be maintained.
    Split {
        layout: Layout,
        children: Vec<NodeId>,
        ratios: Vec<f32>,
    },
    Leaf(WindowId),
}

/// The smallest share of a split any one child may be squeezed to. Below a few
/// percent a window is a sliver with no usable content and no edge wide enough
/// to grab back, so a resize that would go further stops here instead.
const MIN_RATIO: f32 = 0.05;

/// A container tree with a single root.
#[derive(Debug, Clone, Default)]
pub struct Tree {
    nodes: Vec<Option<Node>>,
    parents: Vec<Option<NodeId>>,
    root: Option<NodeId>,
    /// A [`Tree::split`] on a leaf that had no parent, waiting for the insert
    /// that gives it one. See that method for why it cannot act immediately.
    pending_split: Option<(WindowId, Layout)>,
}

impl Tree {
    pub fn new() -> Self {
        Tree::default()
    }

    pub fn root(&self) -> Option<NodeId> {
        self.root
    }

    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.0).and_then(|n| n.as_ref())
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.parents.get(id.0).copied().flatten()
    }

    fn alloc(&mut self, node: Node) -> NodeId {
        // Reuse a freed slot when there is one, so a session that opens and
        // closes windows all day does not grow the arena without bound.
        if let Some(i) = self.nodes.iter().position(|n| n.is_none()) {
            self.nodes[i] = Some(node);
            self.parents[i] = None;
            return NodeId(i);
        }
        self.nodes.push(Some(node));
        self.parents.push(None);
        NodeId(self.nodes.len() - 1)
    }

    fn free(&mut self, id: NodeId) {
        if let Some(slot) = self.nodes.get_mut(id.0) {
            *slot = None;
        }
        if let Some(slot) = self.parents.get_mut(id.0) {
            *slot = None;
        }
    }

    fn set_parent(&mut self, child: NodeId, parent: Option<NodeId>) {
        if let Some(slot) = self.parents.get_mut(child.0) {
            *slot = parent;
        }
    }

    /// The leaf holding `window`, if it is in this tree.
    pub fn find(&self, window: WindowId) -> Option<NodeId> {
        self.nodes.iter().enumerate().find_map(|(i, n)| match n {
            Some(Node::Leaf(w)) if *w == window => Some(NodeId(i)),
            _ => None,
        })
    }

    /// Every window in the tree, left to right, depth first.
    pub fn windows(&self) -> Vec<WindowId> {
        let mut out = Vec::new();
        if let Some(root) = self.root {
            self.walk(root, &mut out);
        }
        out
    }

    fn walk(&self, id: NodeId, out: &mut Vec<WindowId>) {
        match self.node(id) {
            Some(Node::Leaf(w)) => out.push(*w),
            Some(Node::Split { children, .. }) => {
                for child in children.clone() {
                    self.walk(child, out);
                }
            }
            None => {}
        }
    }

    /// Insert `window` as a sibling of `near`, splitting along `layout`.
    ///
    /// With no `near` — the first window, or an empty workspace — it becomes the
    /// root. When `near`'s parent already splits along `layout` the window joins
    /// it as another child rather than nesting a second identical split, which
    /// is what stops a row of four windows from being three nested pairs.
    pub fn insert(&mut self, window: WindowId, near: Option<WindowId>, layout: Layout) -> NodeId {
        let leaf = self.alloc(Node::Leaf(window));

        let Some(near_id) = near.and_then(|w| self.find(w)) else {
            if self.root.is_none() {
                self.root = Some(leaf);
                return leaf;
            }
            // A tree exists but the caller named no neighbour: split the root.
            let old = self.root.take().expect("checked above");
            let layout = self.take_pending_split(old, layout);
            let split = self.alloc(Node::Split {
                layout,
                children: vec![old, leaf],
                ratios: vec![0.5, 0.5],
            });
            self.set_parent(old, Some(split));
            self.set_parent(leaf, Some(split));
            self.root = Some(split);
            return leaf;
        };

        match self.parent(near_id) {
            // Same axis: join the existing split rather than nesting.
            Some(parent) if self.split_layout(parent) == Some(layout) => {
                let at = self.child_index(parent, near_id).map(|i| i + 1);
                self.attach(parent, leaf, at);
            }
            // Different axis, or no parent: wrap the neighbour in a new split.
            _ => {
                let parent = self.parent(near_id);
                let layout = self.take_pending_split(near_id, layout);
                let split = self.alloc(Node::Split {
                    layout,
                    children: vec![near_id, leaf],
                    ratios: vec![0.5, 0.5],
                });
                self.set_parent(near_id, Some(split));
                self.set_parent(leaf, Some(split));
                match parent {
                    Some(p) => self.replace_child(p, near_id, split),
                    None => self.root = Some(split),
                }
            }
        }
        leaf
    }

    /// Remove `window`, collapsing any split left with a single child.
    ///
    /// The collapse is the part that matters: without it, closing one of two
    /// windows leaves a split with one child, which still divides the rectangle
    /// and leaves the survivor occupying half the screen with nothing beside it.
    pub fn remove(&mut self, window: WindowId) {
        let Some(leaf) = self.find(window) else {
            return;
        };
        if matches!(self.pending_split, Some((w, _)) if w == window) {
            self.pending_split = None;
        }
        let parent = self.parent(leaf);
        self.free(leaf);

        let Some(parent) = parent else {
            // The root itself.
            self.root = None;
            return;
        };
        self.detach(parent, leaf);

        // A split with one child is not a split.
        if let Some(Node::Split { children, .. }) = self.node(parent) {
            if children.len() == 1 {
                let survivor = children[0];
                let grandparent = self.parent(parent);
                self.free(parent);
                match grandparent {
                    Some(gp) => {
                        self.replace_child(gp, parent, survivor);
                        self.set_parent(survivor, Some(gp));
                    }
                    None => {
                        self.root = Some(survivor);
                        self.set_parent(survivor, None);
                    }
                }
            }
        }
    }

    /// The rectangle of every window, given the whole output.
    ///
    /// Total by construction: a leaf gets the rectangle handed to it, a split
    /// divides that rectangle among its children, and the divisions are integer
    /// pixels with the remainder given to the last child so the children always
    /// tile their parent exactly. Rounding each child independently would leave
    /// a one-pixel seam at some window widths, which is the kind of thing that
    /// is invisible in a test and obvious on a screen.
    pub fn layout(&self, area: Rect) -> Vec<(WindowId, Rect)> {
        let mut out = Vec::new();
        if let Some(root) = self.root {
            self.layout_into(root, area, &mut out);
        }
        out
    }

    fn layout_into(&self, id: NodeId, area: Rect, out: &mut Vec<(WindowId, Rect)>) {
        match self.node(id) {
            Some(Node::Leaf(w)) => out.push((*w, area)),
            Some(Node::Split {
                layout,
                children,
                ratios,
            }) => {
                let total = match layout {
                    Layout::Horizontal => area.w,
                    Layout::Vertical => area.h,
                };
                let mut offset = 0;
                for (i, child) in children.iter().enumerate() {
                    let last = i + 1 == children.len();
                    let size = if last {
                        total - offset
                    } else {
                        (total as f32 * ratios.get(i).copied().unwrap_or(0.0)).round() as i32
                    };
                    let rect = match layout {
                        Layout::Horizontal => Rect::new(area.x + offset, area.y, size, area.h),
                        Layout::Vertical => Rect::new(area.x, area.y + offset, area.w, size),
                    };
                    self.layout_into(*child, rect, out);
                    offset += size;
                }
            }
            None => {}
        }
    }

    /// The window at `(px, py)`, if any.
    pub fn window_at(&self, area: Rect, px: i32, py: i32) -> Option<WindowId> {
        self.layout(area)
            .into_iter()
            .find(|(_, r)| r.contains(px, py))
            .map(|(w, _)| w)
    }

    /// Where a directional focus move from `window` should land, if anywhere.
    ///
    /// Answered from the laid-out rectangles rather than from tree structure.
    /// The tree's shape and the screen's do not always agree — a window's
    /// left-hand neighbour can be several levels up and across the tree — and
    /// what a user means by "focus left" is the thing that is drawn to the
    /// left, so that is what is measured.
    pub fn focus_target(&self, from: WindowId, dir: Direction, area: Rect) -> Option<WindowId> {
        let rects = self.layout(area);
        let (sx, sy) = centre(rects.iter().find(|(w, _)| *w == from)?.1);
        rects
            .iter()
            .filter(|(w, _)| *w != from)
            .filter_map(|(w, r)| {
                let (cx, cy) = centre(*r);
                let (along, across) = match dir {
                    Direction::Left => (sx - cx, (cy - sy).abs()),
                    Direction::Right => (cx - sx, (cy - sy).abs()),
                    Direction::Up => (sy - cy, (cx - sx).abs()),
                    Direction::Down => (cy - sy, (cx - sx).abs()),
                };
                (along > 0).then_some((along, across, *w))
            })
            // Nearest in the direction of travel wins; only an exact tie there
            // is settled by how far off the row or column the candidate sits,
            // so a grid steps to the window in line rather than the diagonal.
            .min_by_key(|(along, across, _)| (*along, *across))
            .map(|(_, _, w)| w)
    }

    /// Divide the split holding `window` along `layout` from now on.
    ///
    /// A leaf that is the whole tree has no split to set, and doing nothing
    /// would make "split vertical, then open a terminal" — the ordinary way to
    /// start a session — silently ignore the first half. So the intent is held
    /// and spent by the insert that creates the split it was meant for.
    pub fn split(&mut self, window: WindowId, layout: Layout) {
        let Some(leaf) = self.find(window) else {
            return;
        };
        self.pending_split = None;
        match self.parent(leaf) {
            Some(parent) => {
                if let Some(Node::Split { layout: axis, .. }) =
                    self.nodes.get_mut(parent.0).and_then(|n| n.as_mut())
                {
                    *axis = layout;
                }
            }
            None => self.pending_split = Some((window, layout)),
        }
    }

    /// Exchange the positions of two windows, leaving the tree's shape alone.
    ///
    /// Only the leaves' contents trade places, so both windows inherit the
    /// other's rectangle exactly and every ratio, split and parent link is
    /// untouched. Moving the nodes instead would have to reason about one
    /// being an ancestor of the other.
    pub fn swap(&mut self, a: WindowId, b: WindowId) {
        let (Some(leaf_a), Some(leaf_b)) = (self.find(a), self.find(b)) else {
            return;
        };
        if let Some(Some(Node::Leaf(w))) = self.nodes.get_mut(leaf_a.0) {
            *w = b;
        }
        if let Some(Some(Node::Leaf(w))) = self.nodes.get_mut(leaf_b.0) {
            *w = a;
        }
    }

    /// Move the boundary on `window`'s `dir` side by `delta`, as a fraction of
    /// the split that owns it.
    ///
    /// The boundary is rarely in the window's own parent: in a grid every leaf
    /// sits in a column, and the edge between the columns belongs to the split
    /// above. So this climbs until it finds an ancestor that has a neighbour on
    /// this axis, and moves the ratio between that whole subtree and the
    /// neighbour — which is what the user sees as dragging one edge.
    ///
    /// Whatever the subtree gains its neighbour loses, which is what keeps the
    /// ratios summing to 1.0 — the invariant [`Tree::layout`] relies on to tile
    /// without a seam. A window against the edge of the layout in `dir`, or on
    /// an axis nothing splits along, has no boundary to move: nothing happens.
    pub fn resize(&mut self, window: WindowId, dir: Direction, delta: f32) {
        let Some(mut node) = self.find(window) else {
            return;
        };
        while let Some(parent) = self.parent(node) {
            if self.split_layout(parent) == Some(dir.axis()) {
                if let Some(here) = self.child_index(parent, node) {
                    if self.move_boundary(parent, here, dir, delta) {
                        return;
                    }
                }
            }
            node = parent;
        }
    }

    // ---- internals -------------------------------------------------------

    /// Shift `delta` from the child on the `dir` side of child `here` onto
    /// `here`, reporting whether there was a child on that side at all.
    fn move_boundary(&mut self, split: NodeId, here: usize, dir: Direction, delta: f32) -> bool {
        let Some(Node::Split {
            children, ratios, ..
        }) = self.nodes.get_mut(split.0).and_then(|n| n.as_mut())
        else {
            return false;
        };
        let Some(neighbour) = (match dir {
            Direction::Left | Direction::Up => here.checked_sub(1),
            Direction::Right | Direction::Down => (here + 1 < children.len()).then_some(here + 1),
        }) else {
            return false;
        };
        let (Some(&mine), Some(&theirs)) = (ratios.get(here), ratios.get(neighbour)) else {
            return false;
        };
        // The pair's combined share is fixed, so there has to be room in it for
        // two of them before either can be clamped against the other.
        let total = mine + theirs;
        if total < 2.0 * MIN_RATIO {
            return true;
        }
        let grown = (mine + delta).clamp(MIN_RATIO, total - MIN_RATIO);
        ratios[here] = grown;
        ratios[neighbour] = total - grown;
        true
    }

    /// The axis a split newly wrapping `id` should use. A [`Tree::split`] on a
    /// parentless leaf had no split to record itself in and was held instead;
    /// this is where it is spent, and it beats the caller's default.
    fn take_pending_split(&mut self, id: NodeId, fallback: Layout) -> Layout {
        let Some((window, layout)) = self.pending_split else {
            return fallback;
        };
        if self.node(id) == Some(&Node::Leaf(window)) {
            self.pending_split = None;
            return layout;
        }
        fallback
    }

    fn split_layout(&self, id: NodeId) -> Option<Layout> {
        match self.node(id) {
            Some(Node::Split { layout, .. }) => Some(*layout),
            _ => None,
        }
    }

    fn child_index(&self, parent: NodeId, child: NodeId) -> Option<usize> {
        match self.node(parent) {
            Some(Node::Split { children, .. }) => children.iter().position(|c| *c == child),
            _ => None,
        }
    }

    /// Add `child` to `parent` at `at`, re-dividing the space evenly.
    fn attach(&mut self, parent: NodeId, child: NodeId, at: Option<usize>) {
        if let Some(Node::Split {
            children, ratios, ..
        }) = self.nodes.get_mut(parent.0).and_then(|n| n.as_mut())
        {
            let at = at.unwrap_or(children.len()).min(children.len());
            children.insert(at, child);
            let n = children.len();
            *ratios = vec![1.0 / n as f32; n];
        }
        self.set_parent(child, Some(parent));
    }

    fn detach(&mut self, parent: NodeId, child: NodeId) {
        if let Some(Node::Split {
            children, ratios, ..
        }) = self.nodes.get_mut(parent.0).and_then(|n| n.as_mut())
        {
            if let Some(i) = children.iter().position(|c| *c == child) {
                children.remove(i);
                let n = children.len();
                *ratios = if n == 0 {
                    Vec::new()
                } else {
                    vec![1.0 / n as f32; n]
                };
            }
        }
    }

    fn replace_child(&mut self, parent: NodeId, old: NodeId, new: NodeId) {
        if let Some(Node::Split { children, .. }) =
            self.nodes.get_mut(parent.0).and_then(|n| n.as_mut())
        {
            if let Some(i) = children.iter().position(|c| *c == old) {
                children[i] = new;
            }
        }
        self.set_parent(new, Some(parent));
    }
}

fn centre(r: Rect) -> (i32, i32) {
    (r.x + r.w / 2, r.y + r.h / 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: Rect = Rect {
        x: 0,
        y: 0,
        w: 1000,
        h: 800,
    };

    fn w(n: u32) -> WindowId {
        WindowId(n)
    }

    /// Root split horizontal, each half split vertical: w1 w2 across the top,
    /// w3 w4 below them. The smallest tree in which a directional move has a
    /// wrong answer available to pick.
    fn grid() -> Tree {
        let mut t = Tree::new();
        t.insert(w(1), None, Layout::Horizontal);
        t.insert(w(2), Some(w(1)), Layout::Horizontal);
        t.insert(w(3), Some(w(1)), Layout::Vertical);
        t.insert(w(4), Some(w(2)), Layout::Vertical);
        t
    }

    fn rect_of(t: &Tree, window: WindowId) -> Rect {
        t.layout(OUTPUT)
            .into_iter()
            .find(|(id, _)| *id == window)
            .map(|(_, r)| r)
            .unwrap_or_else(|| panic!("{window:?} has no rect"))
    }

    /// Every split divides all of itself among its children and no more.
    /// [`Tree::layout`] divides by these numbers without renormalising, so a
    /// sum that has drifted shows up on screen as a gap or an overlap.
    fn assert_ratios_sum_to_one(t: &Tree) {
        for node in t.nodes.iter().flatten() {
            if let Node::Split {
                children, ratios, ..
            } = node
            {
                assert_eq!(ratios.len(), children.len(), "one ratio per child");
                let sum: f32 = ratios.iter().sum();
                assert!((sum - 1.0).abs() < 1e-4, "ratios sum to {sum}, not 1.0");
            }
        }
    }

    /// The laid-out rectangles cover `area` once over: they fit inside it, they
    /// do not overlap, and their areas add up to all of it — so there is no
    /// gap. Checked from the output of `layout` rather than from the tree, so
    /// it does not restate how `layout` divides.
    fn assert_tiles_exactly(rects: &[(WindowId, Rect)], area: Rect) {
        let covered: i64 = rects.iter().map(|(_, r)| r.w as i64 * r.h as i64).sum();
        assert_eq!(
            covered,
            area.w as i64 * area.h as i64,
            "rects cover {covered} of {area:?}"
        );
        for (i, (id, a)) in rects.iter().enumerate() {
            assert!(
                a.x >= area.x
                    && a.y >= area.y
                    && a.x + a.w <= area.x + area.w
                    && a.y + a.h <= area.y + area.h,
                "{id:?} at {a:?} escapes {area:?}"
            );
            for (other, b) in rects.iter().skip(i + 1) {
                let overlaps =
                    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
                assert!(!overlaps, "{id:?} at {a:?} overlaps {other:?} at {b:?}");
            }
        }
    }

    fn assert_each_window_once(t: &Tree, expected: usize) {
        let rects = t.layout(OUTPUT);
        let mut ids: Vec<_> = rects.iter().map(|(id, _)| *id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), expected, "each window appears exactly once");
        assert_eq!(rects.len(), expected);
    }

    #[test]
    fn an_empty_tree_lays_out_nothing() {
        assert!(Tree::new().layout(OUTPUT).is_empty());
    }

    #[test]
    fn one_window_fills_the_output() {
        let mut t = Tree::new();
        t.insert(w(1), None, Layout::Horizontal);
        assert_eq!(t.layout(OUTPUT), vec![(w(1), OUTPUT)]);
    }

    #[test]
    fn two_windows_split_the_width_exactly() {
        let mut t = Tree::new();
        t.insert(w(1), None, Layout::Horizontal);
        t.insert(w(2), Some(w(1)), Layout::Horizontal);
        assert_eq!(
            t.layout(OUTPUT),
            vec![
                (w(1), Rect::new(0, 0, 500, 800)),
                (w(2), Rect::new(500, 0, 500, 800)),
            ]
        );
    }

    #[test]
    fn a_vertical_split_divides_the_height() {
        let mut t = Tree::new();
        t.insert(w(1), None, Layout::Vertical);
        t.insert(w(2), Some(w(1)), Layout::Vertical);
        assert_eq!(
            t.layout(OUTPUT),
            vec![
                (w(1), Rect::new(0, 0, 1000, 400)),
                (w(2), Rect::new(0, 400, 1000, 400)),
            ]
        );
    }

    #[test]
    fn windows_on_the_same_axis_join_one_split_rather_than_nesting() {
        // Three horizontal windows are one split of three, not a split holding
        // a split. Nesting would still tile correctly but each new window would
        // halve only its neighbour, so the row would go 1/2, 1/4, 1/4.
        let mut t = Tree::new();
        t.insert(w(1), None, Layout::Horizontal);
        t.insert(w(2), Some(w(1)), Layout::Horizontal);
        t.insert(w(3), Some(w(2)), Layout::Horizontal);
        let rects = t.layout(OUTPUT);
        assert_eq!(rects.len(), 3);
        for (_, r) in &rects {
            assert!(
                (r.w - 333).abs() <= 1,
                "each of three should get about a third, got {}",
                r.w
            );
        }
    }

    #[test]
    fn children_always_tile_their_parent_exactly() {
        // The remainder goes to the last child, so no width leaves a seam. 1001
        // is the case that catches naive rounding: three children of 333.67.
        for width in [999, 1000, 1001, 1003, 7] {
            let mut t = Tree::new();
            t.insert(w(1), None, Layout::Horizontal);
            t.insert(w(2), Some(w(1)), Layout::Horizontal);
            t.insert(w(3), Some(w(2)), Layout::Horizontal);
            let area = Rect::new(0, 0, width, 100);
            let rects = t.layout(area);
            let covered: i32 = rects.iter().map(|(_, r)| r.w).sum();
            assert_eq!(covered, width, "widths must sum to the parent at w={width}");
            // and be contiguous, with no gap or overlap
            let mut x = 0;
            for (_, r) in &rects {
                assert_eq!(r.x, x, "gap or overlap at w={width}");
                x += r.w;
            }
        }
    }

    #[test]
    fn a_mixed_tree_nests() {
        // Split horizontally, then split the right-hand window vertically.
        let mut t = Tree::new();
        t.insert(w(1), None, Layout::Horizontal);
        t.insert(w(2), Some(w(1)), Layout::Horizontal);
        t.insert(w(3), Some(w(2)), Layout::Vertical);
        assert_eq!(
            t.layout(OUTPUT),
            vec![
                (w(1), Rect::new(0, 0, 500, 800)),
                (w(2), Rect::new(500, 0, 500, 400)),
                (w(3), Rect::new(500, 400, 500, 400)),
            ]
        );
    }

    #[test]
    fn removing_one_of_two_gives_the_survivor_everything() {
        // The collapse invariant. Without it the survivor keeps half the screen
        // and the other half draws nothing.
        let mut t = Tree::new();
        t.insert(w(1), None, Layout::Horizontal);
        t.insert(w(2), Some(w(1)), Layout::Horizontal);
        t.remove(w(2));
        assert_eq!(t.layout(OUTPUT), vec![(w(1), OUTPUT)]);
    }

    #[test]
    fn removing_the_last_window_empties_the_tree() {
        let mut t = Tree::new();
        t.insert(w(1), None, Layout::Horizontal);
        t.remove(w(1));
        assert!(t.is_empty());
        assert!(t.layout(OUTPUT).is_empty());
    }

    #[test]
    fn a_collapse_reparents_into_the_grandparent() {
        // Three windows, the last two nested vertically inside the right half.
        // Removing one of the pair must hand the whole right half to the other,
        // not leave a one-child vertical split inside it.
        let mut t = Tree::new();
        t.insert(w(1), None, Layout::Horizontal);
        t.insert(w(2), Some(w(1)), Layout::Horizontal);
        t.insert(w(3), Some(w(2)), Layout::Vertical);
        t.remove(w(3));
        assert_eq!(
            t.layout(OUTPUT),
            vec![
                (w(1), Rect::new(0, 0, 500, 800)),
                (w(2), Rect::new(500, 0, 500, 800)),
            ]
        );
    }

    #[test]
    fn removing_an_unknown_window_changes_nothing() {
        let mut t = Tree::new();
        t.insert(w(1), None, Layout::Horizontal);
        let before = t.layout(OUTPUT);
        t.remove(w(99));
        assert_eq!(t.layout(OUTPUT), before);
    }

    #[test]
    fn the_arena_reuses_freed_slots() {
        // Open and close in a loop without the arena growing — a long session
        // does exactly this.
        let mut t = Tree::new();
        for i in 0..50 {
            t.insert(w(i), None, Layout::Horizontal);
            t.remove(w(i));
        }
        assert!(t.is_empty());
        assert!(
            t.nodes.len() <= 4,
            "arena grew to {} slots across 50 open/close cycles",
            t.nodes.len()
        );
    }

    #[test]
    fn window_at_finds_the_leaf_under_a_point() {
        let mut t = Tree::new();
        t.insert(w(1), None, Layout::Horizontal);
        t.insert(w(2), Some(w(1)), Layout::Horizontal);
        assert_eq!(t.window_at(OUTPUT, 10, 10), Some(w(1)));
        assert_eq!(t.window_at(OUTPUT, 600, 10), Some(w(2)));
        assert_eq!(t.window_at(OUTPUT, 2000, 10), None);
    }

    #[test]
    fn a_shared_edge_belongs_to_exactly_one_window() {
        // Tiled neighbours touch. If both claimed x=500 a click there would be
        // ambiguous, and which one won would depend on iteration order.
        let mut t = Tree::new();
        t.insert(w(1), None, Layout::Horizontal);
        t.insert(w(2), Some(w(1)), Layout::Horizontal);
        assert_eq!(t.window_at(OUTPUT, 499, 400), Some(w(1)));
        assert_eq!(t.window_at(OUTPUT, 500, 400), Some(w(2)));
    }

    #[test]
    fn windows_lists_every_leaf_left_to_right() {
        let mut t = Tree::new();
        t.insert(w(1), None, Layout::Horizontal);
        t.insert(w(2), Some(w(1)), Layout::Horizontal);
        t.insert(w(3), Some(w(2)), Layout::Vertical);
        assert_eq!(t.windows(), vec![w(1), w(2), w(3)]);
    }

    #[test]
    fn every_window_gets_exactly_one_rect() {
        // The property that makes layout() safe to consume: no window is
        // missing, none appears twice.
        let mut t = Tree::new();
        for i in 1..=6 {
            let near = (i > 1).then(|| w(i - 1));
            let layout = if i % 2 == 0 {
                Layout::Vertical
            } else {
                Layout::Horizontal
            };
            t.insert(w(i), near, layout);
        }
        let rects = t.layout(OUTPUT);
        let mut ids: Vec<_> = rects.iter().map(|(id, _)| *id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 6, "each window appears exactly once");
        assert_eq!(rects.len(), 6);
    }

    // ---- directional focus ----------------------------------------------

    #[test]
    fn focus_moves_to_the_window_drawn_on_that_side() {
        let mut t = Tree::new();
        t.insert(w(1), None, Layout::Horizontal);
        t.insert(w(2), Some(w(1)), Layout::Horizontal);
        assert_eq!(t.focus_target(w(1), Direction::Right, OUTPUT), Some(w(2)));
        assert_eq!(t.focus_target(w(2), Direction::Left, OUTPUT), Some(w(1)));
    }

    #[test]
    fn focus_at_the_edge_of_the_layout_has_nowhere_to_go() {
        let mut t = Tree::new();
        t.insert(w(1), None, Layout::Horizontal);
        t.insert(w(2), Some(w(1)), Layout::Horizontal);
        assert_eq!(t.focus_target(w(1), Direction::Left, OUTPUT), None);
        assert_eq!(t.focus_target(w(2), Direction::Right, OUTPUT), None);
        // Nothing is above or below either of them, so the other axis is edge
        // in both directions too.
        assert_eq!(t.focus_target(w(1), Direction::Up, OUTPUT), None);
        assert_eq!(t.focus_target(w(1), Direction::Down, OUTPUT), None);
    }

    #[test]
    fn focus_takes_the_neighbour_in_line_not_the_diagonal() {
        // In the grid, "right" from the top-left has two windows to its right.
        // Picking the nearer-in-line one is what makes h/j/k/l feel like moving
        // a cursor rather than cycling.
        let t = grid();
        assert_eq!(t.focus_target(w(1), Direction::Right, OUTPUT), Some(w(2)));
        assert_eq!(t.focus_target(w(1), Direction::Down, OUTPUT), Some(w(3)));
        assert_eq!(t.focus_target(w(4), Direction::Left, OUTPUT), Some(w(3)));
        assert_eq!(t.focus_target(w(4), Direction::Up, OUTPUT), Some(w(2)));
    }

    #[test]
    fn focus_crosses_the_tree_when_the_screen_says_to() {
        // w(3) and w(2) are in different branches — their nearest common
        // ancestor is the root — but on screen w(2) is directly above the
        // right-hand neighbour of w(3), and left/up must still find each other.
        let t = grid();
        assert_eq!(t.focus_target(w(3), Direction::Right, OUTPUT), Some(w(4)));
        assert_eq!(t.focus_target(w(2), Direction::Left, OUTPUT), Some(w(1)));
    }

    #[test]
    fn focus_from_an_unknown_window_is_none() {
        let t = grid();
        assert_eq!(t.focus_target(w(99), Direction::Left, OUTPUT), None);
        assert_eq!(
            Tree::new().focus_target(w(1), Direction::Left, OUTPUT),
            None
        );
    }

    // ---- split -----------------------------------------------------------

    #[test]
    fn splitting_changes_the_axis_of_the_containing_split() {
        let mut t = Tree::new();
        t.insert(w(1), None, Layout::Horizontal);
        t.insert(w(2), Some(w(1)), Layout::Horizontal);
        t.split(w(1), Layout::Vertical);
        assert_eq!(
            t.layout(OUTPUT),
            vec![
                (w(1), Rect::new(0, 0, 1000, 400)),
                (w(2), Rect::new(0, 400, 1000, 400)),
            ]
        );
        assert_ratios_sum_to_one(&t);
        assert_tiles_exactly(&t.layout(OUTPUT), OUTPUT);
    }

    #[test]
    fn splitting_the_only_window_steers_the_next_insert() {
        // There is no split yet to change, so the request has to survive until
        // the insert that creates one — otherwise the first window on a fresh
        // workspace can never be split before a second one exists.
        let mut t = Tree::new();
        t.insert(w(1), None, Layout::Horizontal);
        t.split(w(1), Layout::Vertical);
        t.insert(w(2), Some(w(1)), Layout::Horizontal);
        assert_eq!(
            t.layout(OUTPUT),
            vec![
                (w(1), Rect::new(0, 0, 1000, 400)),
                (w(2), Rect::new(0, 400, 1000, 400)),
            ]
        );
    }

    #[test]
    fn a_held_split_is_spent_once() {
        // The third window must land on the axis the caller asked for, not on
        // the one a `split` two inserts ago wanted.
        let mut t = Tree::new();
        t.insert(w(1), None, Layout::Horizontal);
        t.split(w(1), Layout::Vertical);
        t.insert(w(2), Some(w(1)), Layout::Horizontal);
        t.insert(w(3), Some(w(2)), Layout::Horizontal);
        assert_eq!(
            t.layout(OUTPUT),
            vec![
                (w(1), Rect::new(0, 0, 1000, 400)),
                (w(2), Rect::new(0, 400, 500, 400)),
                (w(3), Rect::new(500, 400, 500, 400)),
            ]
        );
    }

    #[test]
    fn splitting_an_unknown_window_changes_nothing() {
        let mut t = Tree::new();
        t.insert(w(1), None, Layout::Horizontal);
        t.split(w(99), Layout::Vertical);
        t.insert(w(2), Some(w(1)), Layout::Horizontal);
        assert_eq!(
            t.layout(OUTPUT),
            vec![
                (w(1), Rect::new(0, 0, 500, 800)),
                (w(2), Rect::new(500, 0, 500, 800)),
            ]
        );
    }

    // ---- swap ------------------------------------------------------------

    #[test]
    fn swapping_exchanges_two_windows_rectangles() {
        let t0 = grid();
        let (before1, before4) = (rect_of(&t0, w(1)), rect_of(&t0, w(4)));
        let mut t = grid();
        t.swap(w(1), w(4));
        assert_eq!(rect_of(&t, w(4)), before1);
        assert_eq!(rect_of(&t, w(1)), before4);
    }

    #[test]
    fn swapping_leaves_the_tree_tiling_exactly() {
        // Only the leaves' contents move, so the shape of the tree — and with
        // it every ratio and every rectangle — has to come out identical.
        let before = grid().layout(OUTPUT);
        let mut t = grid();
        t.swap(w(2), w(3));
        assert_ratios_sum_to_one(&t);
        assert_each_window_once(&t, 4);
        assert_tiles_exactly(&t.layout(OUTPUT), OUTPUT);
        let after = t.layout(OUTPUT);
        let rects_before: Vec<_> = before.iter().map(|(_, r)| *r).collect();
        let rects_after: Vec<_> = after.iter().map(|(_, r)| *r).collect();
        assert_eq!(rects_after, rects_before, "the rectangles themselves stay");
    }

    #[test]
    fn swapping_with_an_unknown_window_changes_nothing() {
        let mut t = grid();
        let before = t.layout(OUTPUT);
        t.swap(w(1), w(99));
        t.swap(w(99), w(1));
        assert_eq!(t.layout(OUTPUT), before);
    }

    #[test]
    fn swapping_a_window_with_itself_changes_nothing() {
        let mut t = grid();
        let before = t.layout(OUTPUT);
        t.swap(w(2), w(2));
        assert_eq!(t.layout(OUTPUT), before);
    }

    // ---- resize ----------------------------------------------------------

    #[test]
    fn resizing_moves_the_boundary_between_two_neighbours() {
        let mut t = Tree::new();
        t.insert(w(1), None, Layout::Horizontal);
        t.insert(w(2), Some(w(1)), Layout::Horizontal);
        t.resize(w(1), Direction::Right, 0.1);
        assert_eq!(
            t.layout(OUTPUT),
            vec![
                (w(1), Rect::new(0, 0, 600, 800)),
                (w(2), Rect::new(600, 0, 400, 800)),
            ]
        );
    }

    #[test]
    fn resizing_the_far_side_of_a_pair_moves_the_same_boundary() {
        // Growing w(2) leftwards and growing w(1) rightwards are the same edge
        // seen from either end.
        let mut t = Tree::new();
        t.insert(w(1), None, Layout::Horizontal);
        t.insert(w(2), Some(w(1)), Layout::Horizontal);
        t.resize(w(2), Direction::Left, 0.1);
        assert_eq!(
            t.layout(OUTPUT),
            vec![
                (w(1), Rect::new(0, 0, 400, 800)),
                (w(2), Rect::new(400, 0, 600, 800)),
            ]
        );
    }

    #[test]
    fn resizing_at_the_edge_of_the_layout_does_nothing() {
        let mut t = Tree::new();
        t.insert(w(1), None, Layout::Horizontal);
        t.insert(w(2), Some(w(1)), Layout::Horizontal);
        let before = t.layout(OUTPUT);
        t.resize(w(1), Direction::Left, 0.1);
        t.resize(w(2), Direction::Right, 0.1);
        assert_eq!(t.layout(OUTPUT), before);
    }

    #[test]
    fn resizing_across_the_axis_of_the_split_does_nothing() {
        // Nothing in this tree splits vertically, so there is no horizontal
        // boundary anywhere to move up or down.
        let mut t = Tree::new();
        t.insert(w(1), None, Layout::Horizontal);
        t.insert(w(2), Some(w(1)), Layout::Horizontal);
        let before = t.layout(OUTPUT);
        t.resize(w(1), Direction::Down, 0.2);
        t.resize(w(1), Direction::Up, 0.2);
        assert_eq!(t.layout(OUTPUT), before);
    }

    #[test]
    fn resizing_moves_the_column_edge_a_leaf_does_not_own() {
        // Every leaf in the grid sits in a vertical column, so the edge between
        // the columns belongs to the root, two levels up. Stopping at the leaf's
        // own parent would make a horizontal resize impossible in a grid — the
        // most ordinary layout there is.
        let mut t = grid();
        t.resize(w(1), Direction::Right, 0.1);
        assert_eq!(rect_of(&t, w(1)), Rect::new(0, 0, 600, 400));
        assert_eq!(rect_of(&t, w(3)), Rect::new(0, 400, 600, 400));
        assert_eq!(rect_of(&t, w(2)), Rect::new(600, 0, 400, 400));
        assert_eq!(rect_of(&t, w(4)), Rect::new(600, 400, 400, 400));
        assert_ratios_sum_to_one(&t);
        assert_tiles_exactly(&t.layout(OUTPUT), OUTPUT);
    }

    #[test]
    fn resizing_past_the_outermost_split_does_nothing() {
        // w(2) is in the right-hand column, so climbing for a right-hand
        // boundary runs out of ancestors: it is against the edge of the output.
        let mut t = grid();
        let before = t.layout(OUTPUT);
        t.resize(w(2), Direction::Right, 0.1);
        t.resize(w(1), Direction::Left, 0.1);
        assert_eq!(t.layout(OUTPUT), before);
    }

    #[test]
    fn a_resize_cannot_squeeze_a_neighbour_out_of_existence() {
        let mut t = Tree::new();
        t.insert(w(1), None, Layout::Horizontal);
        t.insert(w(2), Some(w(1)), Layout::Horizontal);
        t.resize(w(1), Direction::Right, 10.0);
        assert_eq!(rect_of(&t, w(2)), Rect::new(950, 0, 50, 800));
        t.resize(w(1), Direction::Right, -10.0);
        assert_eq!(rect_of(&t, w(1)), Rect::new(0, 0, 50, 800));
        assert_ratios_sum_to_one(&t);
    }

    #[test]
    fn a_resize_only_moves_the_one_boundary() {
        // Three in a row: growing the middle one rightwards must come out of
        // the right-hand window alone. Renormalising the whole split instead
        // would shuffle the left-hand window that nobody touched.
        let mut t = Tree::new();
        t.insert(w(1), None, Layout::Horizontal);
        t.insert(w(2), Some(w(1)), Layout::Horizontal);
        t.insert(w(3), Some(w(2)), Layout::Horizontal);
        let before = rect_of(&t, w(1));
        t.resize(w(2), Direction::Right, 0.1);
        assert_eq!(rect_of(&t, w(1)), before);
        assert!(rect_of(&t, w(2)).w > rect_of(&t, w(3)).w);
    }

    #[test]
    fn ratios_still_sum_to_one_after_a_resize() {
        let mut t = grid();
        t.resize(w(1), Direction::Right, 0.15);
        t.resize(w(1), Direction::Down, -0.2);
        t.resize(w(4), Direction::Up, 0.3);
        t.resize(w(3), Direction::Right, -0.4);
        assert_ratios_sum_to_one(&t);
    }

    #[test]
    fn children_still_tile_their_parent_exactly_after_a_resize() {
        // The property that survives arithmetic: whatever the ratios end up
        // being, the rectangles still cover the output once each.
        for width in [999, 1000, 1001, 1003, 7] {
            let area = Rect::new(0, 0, width, 801);
            let mut t = grid();
            t.resize(w(1), Direction::Right, 0.17);
            t.resize(w(3), Direction::Up, 0.23);
            t.resize(w(2), Direction::Down, -0.11);
            assert_each_window_once(&t, 4);
            assert_tiles_exactly(&t.layout(area), area);
        }
    }

    #[test]
    fn resizing_an_unknown_window_changes_nothing() {
        let mut t = grid();
        let before = t.layout(OUTPUT);
        t.resize(w(99), Direction::Right, 0.2);
        assert_eq!(t.layout(OUTPUT), before);
    }

    #[test]
    fn resizing_the_only_window_changes_nothing() {
        let mut t = Tree::new();
        t.insert(w(1), None, Layout::Horizontal);
        t.resize(w(1), Direction::Right, 0.2);
        assert_eq!(t.layout(OUTPUT), vec![(w(1), OUTPUT)]);
    }
}
