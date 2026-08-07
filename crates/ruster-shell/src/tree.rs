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

/// A container tree with a single root.
#[derive(Debug, Clone, Default)]
pub struct Tree {
    nodes: Vec<Option<Node>>,
    parents: Vec<Option<NodeId>>,
    root: Option<NodeId>,
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

    // ---- internals -------------------------------------------------------

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
}
