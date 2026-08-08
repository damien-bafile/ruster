//! Editor panes: tree leaves that are not Wayland clients.
//!
//! A pane takes an ordinary [`WindowId`] from the same allocator a client does
//! and goes into the same container tree as an ordinary leaf. The tree never
//! asks what a leaf *is* — it needs `Copy + PartialEq` and nothing else — so a
//! pane already lays out, already gets a focus border, already hit-tests, and is
//! already skipped by every path that looks a leaf up in `toplevels` and
//! `continue`s when it is absent.
//!
//! That is why there is no `Node::Leaf(Client | Buffer)` enum. The alternative
//! was to teach the tree the difference, which would have meant touching
//! `geometry`, `tile_under`, focus, the renderer and 94 tests in `ruster-shell`
//! — and, worse, would have re-opened the question of whether `tile_under` and
//! `geometry` agree at every one of those sites. They are the same list today,
//! and a side table keeps them that way by construction. The compositor has had
//! the pointer land somewhere other than where it looked exactly once, and it
//! was because two functions computed one rectangle two ways.
//!
//! Phase 3 Stage 1: a pane exists, holds focus, moves, splits, resizes and
//! survives a restart. It draws an empty titled frame. Buffers arrive in Stage 2.

use ruster_shell::WindowId;

/// One editor pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorPane {
    /// What the frame is titled. Becomes the buffer name in Stage 2.
    pub title: String,
    /// The character grid the pane's rectangle works out to, from
    /// [`cell_metrics`](ruster_render_gles::atlas::cell_metrics).
    ///
    /// Stored rather than derived at draw time because Stage 2 needs it to
    /// decide how much of a buffer is visible, and a scroll clamp that
    /// disagreed with what was drawn would put the cursor off screen.
    pub cols: u16,
    pub rows: u16,
}

impl EditorPane {
    pub fn new(title: impl Into<String>) -> Self {
        EditorPane {
            title: title.into(),
            cols: 0,
            rows: 0,
        }
    }

    /// The grid a rectangle of `width`x`height` logical pixels works out to.
    ///
    /// Saturating rather than panicking on a zero or negative rectangle: a tile
    /// can legitimately be measured before the output is configured, and a pane
    /// that took the session down for being briefly 0x0 would be a poor trade
    /// for a grid nobody was looking at yet.
    pub fn grid_for(width: i32, height: i32, cell_w: f32, cell_h: f32) -> (u16, u16) {
        if cell_w <= 0.0 || cell_h <= 0.0 {
            return (0, 0);
        }
        let cols = (width.max(0) as f32 / cell_w).floor().max(0.0);
        let rows = (height.max(0) as f32 / cell_h).floor().max(0.0);
        (
            cols.min(u16::MAX as f32) as u16,
            rows.min(u16::MAX as f32) as u16,
        )
    }
}

/// The panes a compositor holds, keyed the same way `toplevels` is.
///
/// A leaf is a client if `toplevels` has it and a pane if this does. Exactly one
/// of the two, which [`Panes::debug_assert_disjoint`] states.
pub type Panes = std::collections::HashMap<WindowId, EditorPane>;

/// Panics in debug builds if any id is both a client and a pane.
///
/// The invariant the whole side-table design rests on. Both maps are keyed from
/// the same id allocator, so a collision means someone inserted a pane over a
/// live window — which would show up as a window that renders as an empty frame
/// rather than as anything obviously wrong.
pub fn debug_assert_disjoint<T>(panes: &Panes, toplevels: &std::collections::HashMap<WindowId, T>) {
    debug_assert!(
        !panes.keys().any(|id| toplevels.contains_key(id)),
        "a leaf is a client or a pane, never both"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rectangle_becomes_the_grid_that_fits_inside_it() {
        // Floor, not round: a partial column has nowhere to put its right-hand
        // half, and rounding up would draw text past the tile's edge.
        assert_eq!(EditorPane::grid_for(800, 600, 10.0, 20.0), (80, 30));
        assert_eq!(EditorPane::grid_for(805, 609, 10.0, 20.0), (80, 30));
    }

    #[test]
    fn a_tile_too_small_for_one_cell_is_an_empty_grid() {
        assert_eq!(EditorPane::grid_for(5, 5, 10.0, 20.0), (0, 0));
    }

    #[test]
    fn a_degenerate_rectangle_does_not_take_the_session_down() {
        // `output_rect()` is 0x0 until the first output is configured, and a
        // pane can be measured in that window.
        assert_eq!(EditorPane::grid_for(0, 0, 10.0, 20.0), (0, 0));
        assert_eq!(EditorPane::grid_for(-100, -100, 10.0, 20.0), (0, 0));
        assert_eq!(EditorPane::grid_for(800, 600, 0.0, 0.0), (0, 0));
    }

    #[test]
    fn a_pane_starts_with_no_grid_until_it_is_laid_out() {
        // The layout is what decides the size; a pane that guessed one would
        // disagree with its own rectangle for the first frame.
        let pane = EditorPane::new("scratch");
        assert_eq!((pane.cols, pane.rows), (0, 0));
        assert_eq!(pane.title, "scratch");
    }
}
