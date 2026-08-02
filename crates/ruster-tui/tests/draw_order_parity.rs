//! The two backends must paint overlapping surfaces in the same order.
//!
//! `ruster-render-raylib` does not depend on ratatui and shares no drawing code
//! with the TUI — only the geometry in `ruster-render`. Draw *order* is not
//! geometry, so nothing but agreement between two files keeps them in step, and
//! a divergence shows up as "it looks right in the terminal" and nowhere else.
//!
//! This is a source scrape because that is what the property is: which of two
//! blocks appears first in each file. There is no rendered artefact to assert
//! on without a GPU on one side and a PTY on the other.
//!
//! The specific thing being pinned: **a modal dialog draws last**, above any
//! float. Both backends used to do the opposite while both of their comments
//! claimed this order — inert, because the only float is the hover popup and it
//! never coexists with a dialog, but the code and the comments disagreed and
//! one of them had to be wrong. A modal is the surface with focus, so it wins.

const TUI: &str = include_str!("../src/renderer.rs");
const GUI: &str = include_str!("../../ruster-render-raylib/src/lib.rs");

/// Where a backend draws its floats, and where it draws its dialog.
fn draw_positions(src: &str, what: &str) -> (usize, usize) {
    let floats = src
        .find("floats_in_draw_order(&state.floats)")
        .unwrap_or_else(|| panic!("{what} no longer draws floats — the scrape has broken"));
    let dialog = src
        .find("if let Some(dlg) = &state.dialog")
        .unwrap_or_else(|| panic!("{what} no longer draws a dialog — the scrape has broken"));
    (floats, dialog)
}

#[test]
fn a_modal_dialog_draws_above_the_floats_in_both_backends() {
    for (src, what) in [(TUI, "the TUI"), (GUI, "the raylib GUI")] {
        let (floats, dialog) = draw_positions(src, what);
        assert!(
            dialog > floats,
            "{what} draws floats after the dialog, so a float would paint over a \
             modal. The dialog is the surface with focus and must draw last."
        );
    }
}

#[test]
fn both_backends_agree_on_the_order() {
    // Stated separately from the rule above: even if the intended order were
    // reversed, the two backends disagreeing is its own bug — that is the
    // parity break the whole render split exists to avoid.
    let (tf, td) = draw_positions(TUI, "the TUI");
    let (gf, gd) = draw_positions(GUI, "the raylib GUI");
    assert_eq!(
        td > tf,
        gd > gf,
        "the backends disagree on whether the dialog or the floats draw last: \
         TUI dialog-last={}, GUI dialog-last={}",
        td > tf,
        gd > gf
    );
}
