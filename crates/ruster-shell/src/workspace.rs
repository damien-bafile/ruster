//! Workspaces: nine container trees, one of them on screen.
//!
//! Phase 0 had a counter and nothing else, so "switching workspaces" moved a
//! label in the statusline and left the same windows where they were. Here a
//! workspace *is* a tree: every window belongs to exactly one, and
//! [`Workspaces::layout`] answers for the active one alone. Everything
//! downstream reads that — what is drawn, what a click can land on, what size
//! each client is configured to — so a window on another workspace is
//! untouched, mapped and alive, and simply not laid out.
//!
//! Focus is the part that does not follow from the trees. It is one handle
//! shared across all nine, so a switch can leave it pointing at a window
//! nothing draws; [`Workspaces::focus_for_active`] is what the compositor asks
//! to repair it.
//!
//! Workspace numbers are 1-based, because that is what the user types and what
//! the statusline shows. The conversion to an arena index lives in one place.

use crate::tree::{Layout, Rect, Tree};
use crate::window::WindowId;

/// Number of workspaces; [`next_workspace`] cycles 1..=9.
pub const WORKSPACE_COUNT: u32 = 9;

/// The workspace after `ws`, wrapping the last one back to the first.
pub fn next_workspace(ws: u32) -> u32 {
    let next = ws + 1;
    if next > WORKSPACE_COUNT {
        1
    } else {
        next
    }
}

/// The arena slot for a workspace number, or `None` for one that does not
/// exist. Every indexing path goes through this, so an out-of-range number is
/// refused rather than clamped or panicked on.
fn index(workspace: u32) -> Option<usize> {
    (1..=WORKSPACE_COUNT)
        .contains(&workspace)
        .then(|| (workspace - 1) as usize)
}

/// Nine container trees and the number of the one on screen.
#[derive(Debug, Clone)]
pub struct Workspaces {
    trees: [Tree; WORKSPACE_COUNT as usize],
    active: u32,
}

impl Default for Workspaces {
    fn default() -> Self {
        Workspaces::new()
    }
}

impl Workspaces {
    pub fn new() -> Self {
        Workspaces {
            trees: std::array::from_fn(|_| Tree::new()),
            active: 1,
        }
    }

    /// The number of the workspace on screen.
    pub fn active(&self) -> u32 {
        self.active
    }

    /// The tree on screen.
    pub fn tree(&self) -> &Tree {
        &self.trees[(self.active - 1) as usize]
    }

    pub fn tree_mut(&mut self) -> &mut Tree {
        &mut self.trees[(self.active - 1) as usize]
    }

    /// The tree of `workspace`, or `None` for a number outside 1..=9.
    pub fn tree_on(&self, workspace: u32) -> Option<&Tree> {
        index(workspace).map(|i| &self.trees[i])
    }

    /// Show `workspace`, hiding whatever was on screen.
    ///
    /// Reports whether the active workspace actually changed: a number that
    /// does not exist, or the one already showing, costs the caller a round of
    /// configures and a full redraw for nothing.
    pub fn switch_to(&mut self, workspace: u32) -> bool {
        if index(workspace).is_none() || workspace == self.active {
            return false;
        }
        self.active = workspace;
        true
    }

    /// Switch to the next workspace, wrapping. Always changes the active one,
    /// so the return is `true` — it is reported anyway to match [`switch_to`].
    ///
    /// [`switch_to`]: Workspaces::switch_to
    pub fn cycle(&mut self) -> bool {
        self.switch_to(next_workspace(self.active))
    }

    /// Where each window on the active workspace sits, given the whole output.
    /// Windows on the other eight get no rectangle at all, which is what hides
    /// them: the renderer and the pointer both work from this list.
    pub fn layout(&self, area: Rect) -> Vec<(WindowId, Rect)> {
        self.tree().layout(area)
    }

    /// Insert `window` into the active workspace. A new window belongs on the
    /// screen the user is looking at, never on a hidden one.
    pub fn insert(&mut self, window: WindowId, near: Option<WindowId>, layout: Layout) {
        self.tree_mut().insert(window, near, layout);
    }

    /// Remove `window` from whichever workspace holds it.
    ///
    /// A client closes on its own schedule, including while its workspace is
    /// hidden, so this cannot look only at the active tree — the leaf would
    /// survive its window and the workspace would tile a gap around a window
    /// that no longer exists.
    pub fn remove(&mut self, window: WindowId) {
        let Some(workspace) = self.workspace_of(window) else {
            return;
        };
        self.trees[(workspace - 1) as usize].remove(window);
    }

    /// The workspace holding `window`, if any.
    pub fn workspace_of(&self, window: WindowId) -> Option<u32> {
        self.trees
            .iter()
            .position(|tree| tree.find(window).is_some())
            .map(|i| i as u32 + 1)
    }

    /// Whether `window` is on the workspace currently on screen.
    ///
    /// The test to make before letting focus or an input event reach a window:
    /// a hidden window has no rectangle, so there is nothing to click and no
    /// reason for it to hold the keyboard.
    pub fn is_visible(&self, window: WindowId) -> bool {
        self.tree().find(window).is_some()
    }

    /// Move `window` to `workspace`, reporting whether anything moved.
    ///
    /// It joins the target tree at the root rather than beside a neighbour:
    /// focus follows the screen, not the workspace, so a hidden workspace has
    /// nothing to insert "next to".
    pub fn move_to_workspace(&mut self, window: WindowId, workspace: u32) -> bool {
        let (Some(to), Some(from)) = (index(workspace), self.workspace_of(window)) else {
            return false;
        };
        if from == workspace {
            return false;
        }
        self.trees[(from - 1) as usize].remove(window);
        self.trees[to].insert(window, None, Layout::Horizontal);
        true
    }

    /// Focus that is still legitimate on the active workspace: `current` when
    /// it is on screen, otherwise the last window of the active tree — the same
    /// fall back `ShellState::remove_window` makes — or `None` on an empty
    /// workspace.
    ///
    /// Without this a switch leaves the keyboard pointed at a window nothing
    /// draws, and every keystroke goes to a client the user cannot see.
    pub fn focus_for_active(&self, current: Option<WindowId>) -> Option<WindowId> {
        match current {
            Some(window) if self.is_visible(window) => Some(window),
            _ => self.tree().windows().last().copied(),
        }
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

    /// Insert `window` on `workspace` without disturbing which one is active.
    fn insert_on(ws: &mut Workspaces, workspace: u32, window: WindowId) {
        let was = ws.active();
        ws.switch_to(workspace);
        ws.insert(window, None, Layout::Horizontal);
        ws.switch_to(was);
    }

    #[test]
    fn a_new_workspace_set_starts_empty_on_the_first() {
        let ws = Workspaces::new();
        assert_eq!(ws.active(), 1);
        assert!(ws.layout(OUTPUT).is_empty());
    }

    #[test]
    fn a_window_on_one_workspace_does_not_lay_out_on_another() {
        // The whole point of the task: workspace 2 draws nothing just because
        // workspace 1 has a window, and the window is still there on the way
        // back — hidden, not closed.
        let mut ws = Workspaces::new();
        ws.insert(w(1), None, Layout::Horizontal);
        assert_eq!(ws.layout(OUTPUT), vec![(w(1), OUTPUT)]);

        ws.switch_to(2);
        assert!(ws.layout(OUTPUT).is_empty(), "workspace 2 holds nothing");
        assert_eq!(ws.workspace_of(w(1)), Some(1), "the window still exists");
        assert!(!ws.is_visible(w(1)));

        ws.switch_to(1);
        assert_eq!(ws.layout(OUTPUT), vec![(w(1), OUTPUT)]);
    }

    #[test]
    fn switching_changes_which_windows_lay_out() {
        let mut ws = Workspaces::new();
        ws.insert(w(1), None, Layout::Horizontal);
        ws.switch_to(2);
        ws.insert(w(2), None, Layout::Horizontal);
        ws.insert(w(3), Some(w(2)), Layout::Horizontal);

        let ids: Vec<_> = ws.layout(OUTPUT).into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, vec![w(2), w(3)]);

        ws.switch_to(1);
        let ids: Vec<_> = ws.layout(OUTPUT).into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, vec![w(1)]);
        // Each workspace tiles its own output from scratch, so the single
        // window on workspace 1 is fullscreen even though workspace 2 splits.
        assert_eq!(ws.layout(OUTPUT), vec![(w(1), OUTPUT)]);
    }

    #[test]
    fn cycling_walks_the_workspaces_and_wraps() {
        assert_eq!(next_workspace(1), 2);
        assert_eq!(next_workspace(WORKSPACE_COUNT), 1);
        let mut ws = Workspaces::new();
        for expected in 2..=WORKSPACE_COUNT {
            assert!(ws.cycle());
            assert_eq!(ws.active(), expected);
        }
        assert!(ws.cycle());
        assert_eq!(ws.active(), 1);
    }

    #[test]
    fn switching_to_a_workspace_that_does_not_exist_changes_nothing() {
        // The caller uses the report to decide whether to reconfigure clients
        // and force a redraw; a refused switch must not claim either.
        let mut ws = Workspaces::new();
        assert!(!ws.switch_to(0));
        assert!(!ws.switch_to(WORKSPACE_COUNT + 1));
        assert!(!ws.switch_to(1), "already showing workspace 1");
        assert_eq!(ws.active(), 1);
    }

    #[test]
    fn moving_a_window_takes_it_out_of_one_and_puts_it_in_the_other() {
        let mut ws = Workspaces::new();
        ws.insert(w(1), None, Layout::Horizontal);
        ws.insert(w(2), Some(w(1)), Layout::Horizontal);

        assert!(ws.move_to_workspace(w(2), 3));
        assert_eq!(ws.workspace_of(w(2)), Some(3));
        assert!(!ws.is_visible(w(2)));
        // The survivor grows back into the whole output — the same collapse
        // that a close performs, because a move is a remove on this side.
        assert_eq!(ws.layout(OUTPUT), vec![(w(1), OUTPUT)]);

        ws.switch_to(3);
        assert_eq!(ws.layout(OUTPUT), vec![(w(2), OUTPUT)]);
    }

    #[test]
    fn a_moved_window_shares_the_workspace_it_lands_on() {
        let mut ws = Workspaces::new();
        insert_on(&mut ws, 2, w(1));
        ws.insert(w(2), None, Layout::Horizontal);
        assert!(ws.move_to_workspace(w(2), 2));

        ws.switch_to(2);
        let rects = ws.layout(OUTPUT);
        assert_eq!(rects.len(), 2, "both windows are on workspace 2");
        let covered: i32 = rects.iter().map(|(_, r)| r.w * r.h).sum();
        assert_eq!(covered, OUTPUT.w * OUTPUT.h, "and they tile it exactly");
    }

    #[test]
    fn a_move_that_cannot_happen_is_refused() {
        let mut ws = Workspaces::new();
        ws.insert(w(1), None, Layout::Horizontal);
        assert!(!ws.move_to_workspace(w(1), 1), "already on workspace 1");
        assert!(!ws.move_to_workspace(w(1), 0), "no workspace 0");
        assert!(
            !ws.move_to_workspace(w(1), WORKSPACE_COUNT + 1),
            "no workspace past the last"
        );
        assert!(!ws.move_to_workspace(w(99), 2), "no such window");
        assert_eq!(ws.layout(OUTPUT), vec![(w(1), OUTPUT)]);
        assert_eq!(ws.workspace_of(w(99)), None);
    }

    #[test]
    fn removing_a_window_finds_it_on_a_hidden_workspace() {
        // A client can close while its workspace is off screen. Searching only
        // the active tree would leave the leaf behind, and the workspace would
        // tile a share of itself to a window that no longer exists.
        let mut ws = Workspaces::new();
        insert_on(&mut ws, 4, w(1));
        insert_on(&mut ws, 4, w(2));
        assert_eq!(ws.active(), 1);

        ws.remove(w(2));
        assert_eq!(ws.workspace_of(w(2)), None);

        ws.switch_to(4);
        assert_eq!(ws.layout(OUTPUT), vec![(w(1), OUTPUT)]);
    }

    #[test]
    fn removing_a_window_no_workspace_holds_changes_nothing() {
        let mut ws = Workspaces::new();
        ws.insert(w(1), None, Layout::Horizontal);
        ws.remove(w(99));
        assert_eq!(ws.layout(OUTPUT), vec![(w(1), OUTPUT)]);
    }

    #[test]
    fn focus_does_not_survive_on_a_window_that_is_no_longer_visible() {
        // Focus is one handle for all nine workspaces, so a switch has to
        // re-home it or the keyboard keeps talking to a window nothing draws.
        let mut ws = Workspaces::new();
        ws.insert(w(1), None, Layout::Horizontal);
        insert_on(&mut ws, 2, w(2));

        assert_eq!(
            ws.focus_for_active(Some(w(1))),
            Some(w(1)),
            "still on screen"
        );

        ws.switch_to(2);
        assert_eq!(
            ws.focus_for_active(Some(w(1))),
            Some(w(2)),
            "the window that left the screen does not keep focus"
        );

        ws.switch_to(3);
        assert_eq!(
            ws.focus_for_active(Some(w(1))),
            None,
            "an empty workspace has nothing to focus"
        );
    }

    #[test]
    fn focus_lands_on_the_last_window_of_the_workspace_switched_to() {
        // The same fall back `ShellState::remove_window` makes, so arriving on
        // a workspace and closing a window put focus in the same place.
        let mut ws = Workspaces::new();
        insert_on(&mut ws, 2, w(1));
        ws.switch_to(2);
        ws.insert(w(2), Some(w(1)), Layout::Horizontal);
        ws.switch_to(1);
        ws.switch_to(2);
        assert_eq!(ws.focus_for_active(None), Some(w(2)));
    }

    #[test]
    fn moving_the_focused_window_away_leaves_focus_on_screen() {
        let mut ws = Workspaces::new();
        ws.insert(w(1), None, Layout::Horizontal);
        ws.insert(w(2), Some(w(1)), Layout::Horizontal);
        ws.move_to_workspace(w(2), 5);
        assert_eq!(ws.focus_for_active(Some(w(2))), Some(w(1)));
    }

    #[test]
    fn each_workspace_keeps_its_own_tree() {
        // Nine independent trees: inserting on one must not disturb the shape
        // of another, which a single tree with a per-window workspace tag
        // would get wrong the moment two workspaces both had splits.
        let mut ws = Workspaces::new();
        ws.insert(w(1), None, Layout::Horizontal);
        ws.insert(w(2), Some(w(1)), Layout::Vertical);
        let before = ws.layout(OUTPUT);

        for workspace in 2..=WORKSPACE_COUNT {
            insert_on(&mut ws, workspace, w(10 + workspace));
        }
        assert_eq!(ws.layout(OUTPUT), before);

        for workspace in 2..=WORKSPACE_COUNT {
            let tree = ws.tree_on(workspace).expect("workspace exists");
            assert_eq!(tree.windows(), vec![w(10 + workspace)]);
        }
        assert!(ws.tree_on(0).is_none());
        assert!(ws.tree_on(WORKSPACE_COUNT + 1).is_none());
    }
}
