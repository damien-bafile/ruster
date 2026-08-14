use std::collections::HashMap;

use crate::cursor::CursorSet;
use crate::document::BufferId;

/// Handle to a [`Window`] inside a [`WindowTree`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(pub u32);

/// A rectangle in cell coordinates (origin top-left).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    pub fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Rect {
            x,
            y,
            width,
            height,
        }
    }
    fn cx(&self) -> i32 {
        self.x as i32 + self.width as i32 / 2
    }
    fn cy(&self) -> i32 {
        self.y as i32 + self.height as i32 / 2
    }
}

/// A viewport onto a buffer. Cursor and scroll are per-window, so two windows
/// showing the same buffer scroll and move independently.
pub struct Window {
    pub buffer: BufferId,
    pub cursors: CursorSet,
    pub scroll_top: usize,
    /// First visible column. Nothing wraps, so without this everything past the
    /// window's width would be unreachable rather than merely off-screen.
    pub scroll_left: usize,
    /// Text rows this window last rendered with. Only the renderer knows the
    /// real geometry, so it records it here for half-page scrolling to use.
    pub height: usize,
    /// Text columns this window last rendered with, recorded for the same
    /// reason as `height`: horizontal scrolling has to know how wide the view is
    /// to tell whether the cursor still fits inside it.
    pub width: usize,
}

impl Window {
    fn new(buffer: BufferId) -> Self {
        Window {
            buffer,
            cursors: CursorSet::single(0),
            scroll_top: 0,
            scroll_left: 0,
            height: 0,
            width: 0,
        }
    }
}

/// Orientation of a split. `Vertical` places children side by side (a vertical
/// divider between them); `Horizontal` stacks children top/bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDir {
    Horizontal,
    Vertical,
}

/// Direction for spatial window focus (`Ctrl-w h/j/k/l`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusDir {
    Left,
    Right,
    Up,
    Down,
}

enum Layout {
    Leaf(WindowId),
    Split {
        dir: SplitDir,
        ratio: f32,
        first: Box<Layout>,
        second: Box<Layout>,
    },
}

/// A binary tree of split windows. Leaves are [`Window`]s; internal nodes are
/// splits. Exactly one window is active at a time.
pub struct WindowTree {
    root: Layout,
    windows: HashMap<WindowId, Window>,
    active: WindowId,
    next: u32,
    /// When `Some`, only this window is rendered full-area (layout preserved).
    fullscreen: Option<WindowId>,
}

/// Rebuild one node, allocating window ids in tree order.
fn build(
    snap: &LayoutSnapshot,
    resolve: impl Fn(usize) -> Option<BufferId> + Copy,
    windows: &mut HashMap<WindowId, Window>,
    next: &mut u32,
    active: &mut Option<WindowId>,
) -> Option<Layout> {
    match snap {
        LayoutSnapshot::Leaf {
            buffer,
            cursor,
            scroll,
            active: is_active,
        } => {
            let buf = resolve(*buffer)?;
            let id = WindowId(*next);
            *next += 1;
            windows.insert(
                id,
                Window {
                    buffer: buf,
                    cursors: CursorSet::single(*cursor),
                    scroll_top: *scroll,
                    // Not saved: the first render clamps it to whatever keeps
                    // the restored cursor visible, which is the only value that
                    // would have been correct anyway.
                    scroll_left: 0,
                    height: 0,
                    width: 0,
                },
            );
            if *is_active {
                *active = Some(id);
            }
            Some(Layout::Leaf(id))
        }
        LayoutSnapshot::Split {
            dir,
            ratio,
            first,
            second,
        } => {
            let a = build(first, resolve, windows, next, active);
            let b = build(second, resolve, windows, next, active);
            match (a, b) {
                (Some(a), Some(b)) => Some(Layout::Split {
                    dir: *dir,
                    ratio: *ratio,
                    first: Box::new(a),
                    second: Box::new(b),
                }),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            }
        }
    }
}

/// A serialisable picture of the window layout, for saving and restoring a
/// session.
///
/// Mirrors the private [`Layout`] deliberately rather than exposing it. A leaf
/// refers to a buffer by **index into the session's file list**, not by
/// [`BufferId`] — ids are allocation order and mean nothing across runs.
#[derive(Debug, Clone, PartialEq)]
pub enum LayoutSnapshot {
    Leaf {
        buffer: usize,
        cursor: usize,
        scroll: usize,
        /// Whether this was the focused window.
        active: bool,
    },
    Split {
        dir: SplitDir,
        ratio: f32,
        first: Box<LayoutSnapshot>,
        second: Box<LayoutSnapshot>,
    },
}

impl WindowTree {
    /// Capture the layout for a session.
    ///
    /// `index_of` maps a buffer to its position in the session's file list, and
    /// returns `None` for buffers that are not being saved — terminals, dired,
    /// and the other special buffers. A split with an unsaveable child collapses
    /// to its saveable side, so a layout that was half scratch still restores
    /// its real windows instead of being dropped whole.
    pub fn snapshot(
        &self,
        index_of: impl Fn(BufferId) -> Option<usize> + Copy,
    ) -> Option<LayoutSnapshot> {
        self.snapshot_at(&self.root, index_of)
    }

    fn snapshot_at(
        &self,
        node: &Layout,
        index_of: impl Fn(BufferId) -> Option<usize> + Copy,
    ) -> Option<LayoutSnapshot> {
        match node {
            Layout::Leaf(id) => {
                let w = self.windows.get(id)?;
                Some(LayoutSnapshot::Leaf {
                    buffer: index_of(w.buffer)?,
                    cursor: w.cursors.head(),
                    scroll: w.scroll_top,
                    active: *id == self.active,
                })
            }
            Layout::Split {
                dir,
                ratio,
                first,
                second,
            } => {
                match (
                    self.snapshot_at(first, index_of),
                    self.snapshot_at(second, index_of),
                ) {
                    (Some(a), Some(b)) => Some(LayoutSnapshot::Split {
                        dir: *dir,
                        ratio: *ratio,
                        first: Box::new(a),
                        second: Box::new(b),
                    }),
                    // One side had nothing worth saving: keep the other rather
                    // than losing both.
                    (Some(a), None) => Some(a),
                    (None, Some(b)) => Some(b),
                    (None, None) => None,
                }
            }
        }
    }

    /// Rebuild a tree from a snapshot.
    ///
    /// `resolve` turns a saved file index back into an open buffer, returning
    /// `None` when that file could not be reopened — those leaves are dropped
    /// the same way unsaveable ones were. `None` overall means nothing restored.
    pub fn restore(
        snap: &LayoutSnapshot,
        resolve: impl Fn(usize) -> Option<BufferId> + Copy,
    ) -> Option<WindowTree> {
        let mut windows = HashMap::new();
        let mut next = 1u32;
        let mut active = None;
        let root = build(snap, resolve, &mut windows, &mut next, &mut active)?;
        // Nothing was marked active (or the active leaf was dropped): focus the
        // first window rather than leaving the tree without a cursor.
        let active = active.or_else(|| windows.keys().copied().min())?;
        Some(WindowTree {
            root,
            windows,
            active,
            next,
            fullscreen: None,
        })
    }

    /// The layout ratios, for tests and for callers that need to compare shapes.
    #[cfg(test)]
    fn shape(&self) -> String {
        fn walk(l: &Layout, out: &mut String) {
            match l {
                Layout::Leaf(_) => out.push('L'),
                Layout::Split {
                    dir, first, second, ..
                } => {
                    out.push(if *dir == SplitDir::Vertical { 'V' } else { 'H' });
                    out.push('(');
                    walk(first, out);
                    out.push(' ');
                    walk(second, out);
                    out.push(')');
                }
            }
        }
        let mut s = String::new();
        walk(&self.root, &mut s);
        s
    }

    /// A single window viewing `buffer`, filling the whole area.
    pub fn single(buffer: BufferId) -> Self {
        let id = WindowId(1);
        let mut windows = HashMap::new();
        windows.insert(id, Window::new(buffer));
        WindowTree {
            root: Layout::Leaf(id),
            windows,
            active: id,
            next: 2,
            fullscreen: None,
        }
    }

    fn alloc_id(&mut self) -> WindowId {
        let id = WindowId(self.next);
        self.next += 1;
        id
    }

    pub fn active(&self) -> WindowId {
        self.active
    }

    pub fn active_window(&self) -> &Window {
        &self.windows[&self.active]
    }

    pub fn active_window_mut(&mut self) -> &mut Window {
        self.windows
            .get_mut(&self.active)
            .expect("active window exists")
    }

    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.get(&id)
    }

    pub fn window_mut(&mut self, id: WindowId) -> Option<&mut Window> {
        self.windows.get_mut(&id)
    }

    pub fn is_fullscreen(&self) -> bool {
        self.fullscreen.is_some()
    }

    /// Number of open windows.
    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// Split the active window in `dir`, creating a new window that views the
    /// same buffer (copying the active window's cursor and scroll). The new
    /// window becomes active. Returns its id.
    pub fn split(&mut self, dir: SplitDir) -> WindowId {
        let new_id = self.alloc_id();
        let (buffer, scroll, scroll_left, cursors) = {
            let a = self.active_window();
            (a.buffer, a.scroll_top, a.scroll_left, a.cursors.clone())
        };
        let mut win = Window::new(buffer);
        win.scroll_top = scroll;
        win.scroll_left = scroll_left;
        win.cursors = cursors;
        self.windows.insert(new_id, win);

        let target = self.active;
        split_leaf(&mut self.root, target, dir, new_id);
        // Splitting exits fullscreen (mirrors vim behavior).
        self.fullscreen = None;
        self.active = new_id;
        new_id
    }

    /// Close the active window, collapsing its parent split. Returns `false`
    /// (and changes nothing) if it is the only window.
    pub fn close_active(&mut self) -> bool {
        if self.windows.len() <= 1 {
            return false;
        }
        let target = self.active;
        if !remove_leaf(&mut self.root, target) {
            return false;
        }
        self.windows.remove(&target);
        if self.fullscreen == Some(target) {
            self.fullscreen = None;
        }
        self.active = first_leaf(&self.root);
        true
    }

    /// Close every window except the active one.
    pub fn only(&mut self) {
        let keep = self.active;
        self.windows.retain(|&id, _| id == keep);
        self.root = Layout::Leaf(keep);
        self.fullscreen = None;
    }

    /// Toggle fullscreen for the active window. The split layout is preserved
    /// and restored on toggle-off; no window state is lost.
    pub fn toggle_fullscreen(&mut self) {
        self.fullscreen = match self.fullscreen {
            Some(_) => None,
            None => Some(self.active),
        };
    }

    /// Move focus to the nearest window in `dir` (no-op if none exists).
    /// Focus a window by id, ignoring ids that are not in the tree. Returns
    /// whether focus moved.
    pub fn focus_id(&mut self, id: WindowId) -> bool {
        if self.active == id || !self.windows.contains_key(&id) {
            return false;
        }
        self.active = id;
        true
    }

    pub fn focus(&mut self, dir: FocusDir) {
        // Adjacency is scale-invariant, so compute over a large virtual area.
        let rects = self.compute_all_rects(Rect::new(0, 0, 10_000, 10_000));
        let active_rect = match rects.iter().find(|(id, _)| *id == self.active) {
            Some((_, r)) => *r,
            None => return,
        };
        let mut best: Option<(WindowId, i32)> = None;
        for (id, r) in &rects {
            if *id == self.active {
                continue;
            }
            let ok = match dir {
                FocusDir::Left => r.cx() < active_rect.cx() && overlaps_v(r, &active_rect),
                FocusDir::Right => r.cx() > active_rect.cx() && overlaps_v(r, &active_rect),
                FocusDir::Up => r.cy() < active_rect.cy() && overlaps_h(r, &active_rect),
                FocusDir::Down => r.cy() > active_rect.cy() && overlaps_h(r, &active_rect),
            };
            if !ok {
                continue;
            }
            let dist = match dir {
                FocusDir::Left | FocusDir::Right => (r.cx() - active_rect.cx()).abs(),
                FocusDir::Up | FocusDir::Down => (r.cy() - active_rect.cy()).abs(),
            };
            if best.is_none_or(|(_, d)| dist < d) {
                best = Some((*id, dist));
            }
        }
        if let Some((id, _)) = best {
            self.active = id;
        }
    }

    /// Rectangles for every visible window given the total `area`. When a
    /// window is fullscreen, returns only that window filling `area`. Pure
    /// geometry — no rendering.
    pub fn compute_rects(&self, area: Rect) -> Vec<(WindowId, Rect)> {
        if let Some(id) = self.fullscreen {
            return vec![(id, area)];
        }
        self.compute_all_rects(area)
    }

    /// Move the split edge on `wid`'s right or bottom by `dx`/`dy` cells.
    ///
    /// `area` is the frame the layout is computed against — the same one passed
    /// to [`compute_rects`](Self::compute_rects) — because a ratio only means
    /// something relative to the space being divided.
    ///
    /// Returns whether anything moved. The edge that moves is the one belonging
    /// to the innermost split that has `wid` on its *first* side: that is the
    /// edge drawn at `wid`'s right or bottom, which is the one under the
    /// pointer when a drag starts there. Both panes keep a minimum width, so an
    /// enthusiastic drag cannot collapse one to nothing.
    pub fn resize(&mut self, wid: WindowId, area: Rect, dx: i32, dy: i32) -> bool {
        /// Smallest a pane may be squeezed to, in cells.
        const MIN_CELLS: f32 = 4.0;

        fn walk(
            layout: &mut Layout,
            area: Rect,
            wid: WindowId,
            want: SplitDir,
            delta: i32,
        ) -> bool {
            let Layout::Split {
                dir,
                ratio,
                first,
                second,
            } = layout
            else {
                return false;
            };

            let (first_area, second_area) = split_areas(*dir, *ratio, area);
            // Innermost first: a nested split's edge is nearer the pointer than
            // its parent's.
            if walk(first, first_area, wid, want, delta)
                || walk(second, second_area, wid, want, delta)
            {
                return true;
            }

            if *dir != want || !contains_window(first, wid) {
                return false;
            }
            let span = match want {
                SplitDir::Vertical => area.width,
                SplitDir::Horizontal => area.height,
            } as f32;
            if span <= 0.0 {
                return false;
            }
            let lo = MIN_CELLS / span;
            let hi = 1.0 - lo;
            if lo >= hi {
                return false;
            }
            let next = (*ratio + delta as f32 / span).clamp(lo, hi);
            if (next - *ratio).abs() < f32::EPSILON {
                return false;
            }
            *ratio = next;
            true
        }

        let mut moved = false;
        if dx != 0 {
            moved |= walk(&mut self.root, area, wid, SplitDir::Vertical, dx);
        }
        if dy != 0 {
            moved |= walk(&mut self.root, area, wid, SplitDir::Horizontal, dy);
        }
        moved
    }

    /// Rectangles for all leaves ignoring fullscreen (used for focus geometry).
    fn compute_all_rects(&self, area: Rect) -> Vec<(WindowId, Rect)> {
        let mut out = Vec::new();
        layout_rects(&self.root, area, &mut out);
        out
    }
}

fn overlaps_v(a: &Rect, b: &Rect) -> bool {
    a.y < b.y + b.height && b.y < a.y + a.height
}

fn overlaps_h(a: &Rect, b: &Rect) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width
}

/// Whether `layout` contains `wid` anywhere beneath it.
fn contains_window(layout: &Layout, wid: WindowId) -> bool {
    match layout {
        Layout::Leaf(id) => *id == wid,
        Layout::Split { first, second, .. } => {
            contains_window(first, wid) || contains_window(second, wid)
        }
    }
}

/// The two halves `area` divides into at `ratio`. The single definition of
/// where a split edge falls, shared by layout and resize so the edge a drag
/// moves is the edge that was drawn.
fn split_areas(dir: SplitDir, ratio: f32, area: Rect) -> (Rect, Rect) {
    match dir {
        SplitDir::Vertical => {
            let w1 = ((area.width as f32) * ratio)
                .round()
                .clamp(0.0, area.width as f32) as u16;
            (
                Rect::new(area.x, area.y, w1, area.height),
                Rect::new(area.x + w1, area.y, area.width - w1, area.height),
            )
        }
        SplitDir::Horizontal => {
            let h1 = ((area.height as f32) * ratio)
                .round()
                .clamp(0.0, area.height as f32) as u16;
            (
                Rect::new(area.x, area.y, area.width, h1),
                Rect::new(area.x, area.y + h1, area.width, area.height - h1),
            )
        }
    }
}

fn layout_rects(layout: &Layout, area: Rect, out: &mut Vec<(WindowId, Rect)>) {
    match layout {
        Layout::Leaf(id) => out.push((*id, area)),
        Layout::Split {
            dir,
            ratio,
            first,
            second,
        } => {
            let (first_area, second_area) = split_areas(*dir, *ratio, area);
            layout_rects(first, first_area, out);
            layout_rects(second, second_area, out);
        }
    }
}

/// Replace `Leaf(target)` with a split of `[target, new]`. Returns whether the
/// target was found.
fn split_leaf(layout: &mut Layout, target: WindowId, dir: SplitDir, new: WindowId) -> bool {
    match layout {
        Layout::Leaf(id) if *id == target => {
            *layout = Layout::Split {
                dir,
                ratio: 0.5,
                first: Box::new(Layout::Leaf(target)),
                second: Box::new(Layout::Leaf(new)),
            };
            true
        }
        Layout::Leaf(_) => false,
        Layout::Split { first, second, .. } => {
            split_leaf(first, target, dir, new) || split_leaf(second, target, dir, new)
        }
    }
}

/// Remove `Leaf(target)`, replacing its parent split with the sibling subtree.
/// Returns whether the target was found (never removes the root leaf).
fn remove_leaf(layout: &mut Layout, target: WindowId) -> bool {
    let Layout::Split { first, second, .. } = layout else {
        return false;
    };
    if matches!(**first, Layout::Leaf(id) if id == target) {
        let sibling = std::mem::replace(second.as_mut(), Layout::Leaf(target));
        *layout = sibling;
        return true;
    }
    if matches!(**second, Layout::Leaf(id) if id == target) {
        let sibling = std::mem::replace(first.as_mut(), Layout::Leaf(target));
        *layout = sibling;
        return true;
    }
    remove_leaf(first, target) || remove_leaf(second, target)
}

/// The first (leftmost/topmost) leaf window id in the tree.
fn first_leaf(layout: &Layout) -> WindowId {
    match layout {
        Layout::Leaf(id) => *id,
        Layout::Split { first, .. } => first_leaf(first),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> WindowTree {
        WindowTree::single(BufferId(1))
    }

    #[test]
    fn single_window_fills_area() {
        let t = tree();
        let area = Rect::new(0, 0, 80, 24);
        let rects = t.compute_rects(area);
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0].1, area);
    }

    #[test]
    fn horizontal_split_stacks_and_covers_area() {
        let mut t = tree();
        let a = t.active();
        let b = t.split(SplitDir::Horizontal);
        assert_ne!(a, b);
        assert_eq!(t.active(), b, "new split is focused");
        let rects = t.compute_rects(Rect::new(0, 0, 80, 24));
        assert_eq!(rects.len(), 2);
        // Stacked: same width, split height, second starts below first, no overlap.
        let ra = rects.iter().find(|(id, _)| *id == a).unwrap().1;
        let rb = rects.iter().find(|(id, _)| *id == b).unwrap().1;
        assert_eq!(ra.width, 80);
        assert_eq!(rb.width, 80);
        assert_eq!(ra.height + rb.height, 24);
        assert_eq!(rb.y, ra.y + ra.height);
    }

    #[test]
    fn vertical_split_is_side_by_side() {
        let mut t = tree();
        let a = t.active();
        let b = t.split(SplitDir::Vertical);
        let rects = t.compute_rects(Rect::new(0, 0, 80, 24));
        let ra = rects.iter().find(|(id, _)| *id == a).unwrap().1;
        let rb = rects.iter().find(|(id, _)| *id == b).unwrap().1;
        assert_eq!(ra.height, 24);
        assert_eq!(rb.height, 24);
        assert_eq!(ra.width + rb.width, 80);
        assert_eq!(rb.x, ra.x + ra.width);
    }

    #[test]
    fn focus_moves_active_between_side_by_side() {
        let mut t = tree();
        let a = t.active();
        let b = t.split(SplitDir::Vertical); // b is on the right, now active
        assert_eq!(t.active(), b);
        t.focus(FocusDir::Left);
        assert_eq!(t.active(), a);
        t.focus(FocusDir::Right);
        assert_eq!(t.active(), b);
    }

    #[test]
    fn close_active_on_last_window_is_false() {
        let mut t = tree();
        assert!(!t.close_active());
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn close_active_collapses_split() {
        let mut t = tree();
        let a = t.active();
        let _b = t.split(SplitDir::Vertical);
        assert_eq!(t.len(), 2);
        assert!(t.close_active()); // closes b
        assert_eq!(t.len(), 1);
        assert_eq!(t.active(), a);
        let rects = t.compute_rects(Rect::new(0, 0, 80, 24));
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0].1, Rect::new(0, 0, 80, 24));
    }

    #[test]
    fn only_keeps_active_and_fills_area() {
        let mut t = tree();
        t.split(SplitDir::Vertical);
        t.split(SplitDir::Horizontal);
        assert_eq!(t.len(), 3);
        let keep = t.active();
        t.only();
        assert_eq!(t.len(), 1);
        assert_eq!(t.active(), keep);
        assert_eq!(t.compute_rects(Rect::new(0, 0, 80, 24)).len(), 1);
    }

    #[test]
    fn fullscreen_returns_single_full_rect_and_restores() {
        let mut t = tree();
        let a = t.active();
        let _b = t.split(SplitDir::Vertical);
        let before = t.compute_rects(Rect::new(0, 0, 80, 24));
        assert_eq!(before.len(), 2);
        // Focus back to a, then fullscreen it.
        t.focus(FocusDir::Left);
        assert_eq!(t.active(), a);
        t.toggle_fullscreen();
        let fs = t.compute_rects(Rect::new(0, 0, 80, 24));
        assert_eq!(fs.len(), 1);
        assert_eq!(fs[0], (a, Rect::new(0, 0, 80, 24)));
        // Toggle off restores both.
        t.toggle_fullscreen();
        assert_eq!(t.compute_rects(Rect::new(0, 0, 80, 24)).len(), 2);
    }

    #[test]
    fn split_copies_cursor_and_buffer() {
        let mut t = tree();
        t.active_window_mut().scroll_top = 7;
        t.active_window_mut().scroll_left = 12;
        t.active_window_mut()
            .cursors
            .set_head(0, &crate::buffer::Buffer::from_str(""));
        let buf = t.active_window().buffer;
        let new = t.split(SplitDir::Horizontal);
        assert_eq!(t.window(new).unwrap().buffer, buf);
        assert_eq!(t.window(new).unwrap().scroll_top, 7);
        assert_eq!(
            t.window(new).unwrap().scroll_left,
            12,
            "a split of a sideways-scrolled window looks at the same columns"
        );
    }

    /// The shape, the focus, and every cursor must survive a save/restore.
    #[test]
    fn a_split_layout_round_trips_through_a_snapshot() {
        let mut t = WindowTree::single(BufferId(1));
        t.active_window_mut().cursors = CursorSet::single(42);
        t.active_window_mut().scroll_top = 7;
        let right = t.split(SplitDir::Vertical);
        t.window_mut(right).unwrap().buffer = BufferId(2);
        t.window_mut(right).unwrap().cursors = CursorSet::single(9);
        let bottom = t.split(SplitDir::Horizontal);
        t.window_mut(bottom).unwrap().buffer = BufferId(3);
        let before = t.shape();
        let active_buf = t.active_window().buffer;

        // BufferId(n) <-> index n-1.
        let snap = t
            .snapshot(|b| Some(b.0 as usize - 1))
            .expect("all saveable");
        let restored =
            WindowTree::restore(&snap, |i| Some(BufferId(i as u32 + 1))).expect("rebuilt");

        assert_eq!(restored.shape(), before, "same tree shape");
        assert_eq!(restored.len(), 3);
        assert_eq!(
            restored.active_window().buffer,
            active_buf,
            "focus preserved"
        );
        let first = restored.window(WindowId(1)).unwrap();
        assert_eq!(
            (first.cursors.head(), first.scroll_top),
            (42, 7),
            "cursor and scroll kept"
        );
    }

    /// Terminals and dired are not saveable; their windows must drop out and
    /// leave the rest of the layout intact rather than losing everything.
    #[test]
    fn unsaveable_windows_collapse_out_of_the_layout() {
        let mut t = WindowTree::single(BufferId(1));
        let right = t.split(SplitDir::Vertical);
        t.window_mut(right).unwrap().buffer = BufferId(99); // a terminal, say

        let snap = t
            .snapshot(|b| (b.0 != 99).then_some(b.0 as usize - 1))
            .expect("one survives");
        assert!(
            matches!(snap, LayoutSnapshot::Leaf { buffer: 0, .. }),
            "{snap:?}"
        );

        let restored = WindowTree::restore(&snap, |i| Some(BufferId(i as u32 + 1))).unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored.active_window().buffer, BufferId(1));
    }

    #[test]
    fn a_layout_with_nothing_saveable_snapshots_to_nothing() {
        let t = WindowTree::single(BufferId(1));
        assert!(t.snapshot(|_| None).is_none());
    }

    /// A file deleted since the session was written must not take the rest of
    /// the layout with it.
    #[test]
    fn a_file_that_no_longer_opens_is_dropped_on_restore() {
        let snap = LayoutSnapshot::Split {
            dir: SplitDir::Vertical,
            ratio: 0.5,
            first: Box::new(LayoutSnapshot::Leaf {
                buffer: 0,
                cursor: 0,
                scroll: 0,
                active: true,
            }),
            second: Box::new(LayoutSnapshot::Leaf {
                buffer: 1,
                cursor: 0,
                scroll: 0,
                active: false,
            }),
        };
        // Index 0 is gone; only index 1 reopens.
        let t = WindowTree::restore(&snap, |i| (i == 1).then_some(BufferId(7))).expect("one left");
        assert_eq!(t.len(), 1);
        assert_eq!(
            t.active_window().buffer,
            BufferId(7),
            "focus falls to what remains"
        );

        // And if nothing reopens at all, there is no tree.
        assert!(WindowTree::restore(&snap, |_| None).is_none());
    }
}
