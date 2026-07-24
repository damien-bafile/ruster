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
        Rect { x, y, width, height }
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
    /// Text rows this window last rendered with. Only the renderer knows the
    /// real geometry, so it records it here for half-page scrolling to use.
    pub height: usize,
}

impl Window {
    fn new(buffer: BufferId) -> Self {
        Window { buffer, cursors: CursorSet::single(0), scroll_top: 0, height: 0 }
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
    Split { dir: SplitDir, ratio: f32, first: Box<Layout>, second: Box<Layout> },
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

impl WindowTree {
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
        self.windows.get_mut(&self.active).expect("active window exists")
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
        let (buffer, scroll, cursors) = {
            let a = self.active_window();
            (a.buffer, a.scroll_top, a.cursors.clone())
        };
        let mut win = Window::new(buffer);
        win.scroll_top = scroll;
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
            if best.map_or(true, |(_, d)| dist < d) {
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

fn layout_rects(layout: &Layout, area: Rect, out: &mut Vec<(WindowId, Rect)>) {
    match layout {
        Layout::Leaf(id) => out.push((*id, area)),
        Layout::Split { dir, ratio, first, second } => match dir {
            SplitDir::Vertical => {
                let w1 = ((area.width as f32) * ratio).round() as u16;
                let w1 = w1.clamp(0, area.width);
                let first_area = Rect::new(area.x, area.y, w1, area.height);
                let second_area =
                    Rect::new(area.x + w1, area.y, area.width - w1, area.height);
                layout_rects(first, first_area, out);
                layout_rects(second, second_area, out);
            }
            SplitDir::Horizontal => {
                let h1 = ((area.height as f32) * ratio).round() as u16;
                let h1 = h1.clamp(0, area.height);
                let first_area = Rect::new(area.x, area.y, area.width, h1);
                let second_area =
                    Rect::new(area.x, area.y + h1, area.width, area.height - h1);
                layout_rects(first, first_area, out);
                layout_rects(second, second_area, out);
            }
        },
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
        t.active_window_mut().cursors.set_head(0, &crate::buffer::Buffer::from_str(""));
        let buf = t.active_window().buffer;
        let new = t.split(SplitDir::Horizontal);
        assert_eq!(t.window(new).unwrap().buffer, buf);
        assert_eq!(t.window(new).unwrap().scroll_top, 7);
    }
}
