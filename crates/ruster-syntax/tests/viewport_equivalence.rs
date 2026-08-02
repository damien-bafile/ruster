//! Highlighting only the visible lines must not change what they look like.
//!
//! The optimisation bounds the tree-sitter query to a byte range. That is only
//! safe because the query matches nodes that *overlap* the range rather than
//! ones contained in it — otherwise a string or comment opening above the
//! viewport would lose its capture and render as plain text. These tests are
//! the guard on that, and on the smaller claim that nothing else in the pass
//! quietly depends on having seen the whole file.
//!
//! Every case compares against the unbounded result rather than against a
//! hand-written expectation, so it stays honest if the grammar or the queries
//! change.

use ruster_syntax::SyntaxEngine;

/// A file several margins long, with the constructs most likely to break: a
/// block comment and a raw string each spanning hundreds of lines, so a
/// viewport can sit well inside one with its opener far out of range.
///
/// Length matters. At 200 lines of margin either side, a fixture under ~400
/// lines is covered end to end by any viewport, and every assertion below
/// passes whether or not the query is bounded at all.
fn long_source() -> String {
    let mut s = String::new();
    for i in 0..300 {
        s.push_str(&format!("fn head_{i}() -> u32 {{ {i} }}\n"));
    }
    s.push_str("/* a block comment\n");
    for i in 0..300 {
        s.push_str(&format!("   still inside the comment, line {i}\n"));
    }
    s.push_str("   end of it */\n");
    // An ordinary string, not a raw one: the Rust highlight query captures
    // `raw_string_literal` bodies as nothing at all, so a raw string would make
    // the assertions below vacuous rather than testing overlap.
    s.push_str("const S: &str = \"line 0\n");
    for i in 1..300 {
        s.push_str(&format!("   still inside the string, line {i}\n"));
    }
    s.push_str("end\";\n");
    s.push_str("/// A doc comment with a TODO: buried in it\n");
    for i in 0..300 {
        s.push_str(&format!("fn tail_{i}() -> u32 {{ {i} }}\n"));
    }
    s
}

/// The index of the first line containing `needle`, so the tests name what
/// they mean instead of hard-coding offsets that drift when the fixture grows.
fn line_with(src: &str, needle: &str) -> usize {
    src.lines()
        .position(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("no line has {needle:?}"))
}

/// Styles for `lines`, highlighted with a viewport that covers exactly them.
fn viewport_styles(src: &str, first: usize, last: usize) -> Vec<ruster_render::StyledLine> {
    let mut e = SyntaxEngine::new(src, "rs").unwrap();
    e.reparse(src);
    assert!(
        e.set_viewport(first, last),
        "a fresh engine has no viewport to reuse"
    );
    e.styled_lines().to_vec()
}

fn full_styles(src: &str) -> Vec<ruster_render::StyledLine> {
    let mut e = SyntaxEngine::new(src, "rs").unwrap();
    e.reparse(src);
    e.styled_lines().to_vec()
}

#[test]
fn a_viewport_gives_the_same_styles_as_a_full_pass_for_the_lines_it_covers() {
    let src = long_source();
    let full = full_styles(&src);
    let total = full.len();

    // Walk a 20-line window across the file, including both edges.
    for top in (0..total.saturating_sub(20)).step_by(7) {
        let bottom = top + 19;
        let vp = viewport_styles(&src, top, bottom);
        assert_eq!(vp.len(), full.len(), "the cache stays one entry per line");
        for line in top..=bottom {
            assert_eq!(
                vp[line], full[line],
                "line {line} differs with a viewport of {top}..={bottom}\n\
                 viewport: {:?}\nfull:     {:?}",
                vp[line], full[line]
            );
        }
    }
}

#[test]
fn a_viewport_inside_a_block_comment_still_colours_it() {
    let src = long_source();
    let full = full_styles(&src);
    // Deep inside the block comment, ~150 lines past its opener and well
    // outside any margin around it.
    let inside = line_with(&src, "inside the comment, line 150");
    assert!(
        !full[inside].highlights.is_empty(),
        "precondition: line {inside} is highlighted in a full pass — {:?}",
        full[inside].text
    );
    let vp = viewport_styles(&src, inside, inside);
    assert_eq!(
        vp[inside], full[inside],
        "a comment opening above the viewport lost its capture"
    );
}

#[test]
fn a_viewport_inside_a_string_still_colours_it() {
    let src = long_source();
    let full = full_styles(&src);
    let inside = line_with(&src, "inside the string, line 150");
    assert!(
        full[inside].text.contains("inside the string"),
        "precondition: line {inside} is inside the string, got {:?}",
        full[inside].text
    );
    assert!(
        !full[inside].highlights.is_empty(),
        "precondition: it is highlighted"
    );
    let vp = viewport_styles(&src, inside, inside);
    assert_eq!(
        vp[inside], full[inside],
        "a string opening above the viewport lost its capture"
    );
}

#[test]
fn lines_outside_the_viewport_keep_their_text() {
    // They lose their colour — that is the point — but the text a caller reads
    // for line count, width or the minimap must still be right.
    let src = long_source();
    let full = full_styles(&src);
    let vp = viewport_styles(&src, 0, 10);
    for (i, (a, b)) in vp.iter().zip(full.iter()).enumerate() {
        assert_eq!(a.text, b.text, "line {i} text changed outside the viewport");
    }
}

#[test]
fn lines_outside_the_viewport_are_left_plain() {
    // The output half of the optimisation: the per-line build loop skips
    // styling for rows nobody can see, including the rainbow-bracket pass that
    // walks every character.
    //
    // This does *not* prove the query was bounded — with `set_byte_range`
    // removed the assertion below still holds, because this skip runs either
    // way. `the_query_is_bounded_not_just_its_output` is the guard on that,
    // and was the only test to fail when the bound was deleted.
    let src = long_source();
    let full = full_styles(&src);
    let vp = viewport_styles(&src, 0, 10);
    // `len() - 2`: the fixture ends in a newline, so the last entry is the
    // empty line after it and is never highlighted by anything.
    let far = full.len() - 2;
    assert!(
        !full[far].highlights.is_empty(),
        "precondition: line {far} is highlighted"
    );
    assert!(
        vp[far].highlights.is_empty(),
        "line {far} is far outside a 0..=10 viewport but was still highlighted — \
         the query is not being bounded, so the optimisation is not happening"
    );
}

#[test]
fn scrolling_within_the_margin_does_no_work() {
    let src = long_source();
    let mut e = SyntaxEngine::new(&src, "rs").unwrap();
    e.reparse(&src);
    assert!(e.set_viewport(100, 120), "the first call must highlight");
    assert!(
        !e.set_viewport(101, 121),
        "a one-line scroll is inside the margin"
    );
    assert!(
        !e.set_viewport(100, 120),
        "the same range twice does nothing"
    );
    // Far enough to leave the margin.
    assert!(
        e.set_viewport(500, 520),
        "leaving the margin must re-highlight"
    );
}

#[test]
fn the_viewport_survives_an_edit() {
    let src = long_source();
    let mut e = SyntaxEngine::new(&src, "rs").unwrap();
    e.reparse(&src);
    e.set_viewport(100, 120);
    let before = e.viewport();
    let mut b = ruster_core::buffer::Buffer::from_str(&src);
    b.insert(0, "x");
    e.reparse_with_edits(&b.to_string(), &b.take_edits());
    assert_eq!(
        e.viewport(),
        before,
        "a reparse must not widen the viewport back to the file"
    );
    // And the reparse must have honoured it rather than highlighting everything.
    let far = e.styled_lines().len() - 2;
    assert!(
        e.styled_lines()[far].highlights.is_empty(),
        "the reparse ignored the viewport"
    );
}

#[test]
fn the_query_is_bounded_not_just_its_output() {
    // The half that actually costs time. Comment ranges are recorded straight
    // from the query as it runs, so they are the one observable that says what
    // the query looked at rather than what survived into the output.
    let src = long_source();
    let kws = vec!["TODO".to_string()];
    let mut e = SyntaxEngine::new(&src, "rs").unwrap();
    e.reparse(&src);
    let marker = line_with(&src, "TODO: buried");
    e.set_viewport(0, 10);
    assert!(
        marker > 211,
        "precondition: the marker at line {marker} is outside a 0..=10 viewport plus margin"
    );
    assert!(
        e.todo_markers(&kws).is_empty(),
        "the query ran past the viewport — it found a comment {} lines beyond it, \
         so the pass is still doing whole-file work",
        marker - 211
    );
}

#[test]
fn the_todo_panel_sees_markers_outside_the_viewport() {
    // `todo_markers` reads the last pass's comments, which is right for drawing
    // and wrong for a list. The whole-file variant exists for the list, and
    // this is what stops the two being collapsed back together.
    let src = long_source();
    let kws = vec!["TODO".to_string()];
    let mut e = SyntaxEngine::new(&src, "rs").unwrap();
    e.reparse(&src);
    // The doc comment holding the TODO is near the end; look at the start.
    e.set_viewport(0, 10);
    assert!(
        e.todo_markers(&kws).is_empty(),
        "precondition: the marker is outside a 0..=10 viewport"
    );
    let all = e.all_todo_markers(&kws);
    assert_eq!(
        all.len(),
        1,
        "the whole-file scan must still find it: {all:?}"
    );
    assert_eq!(all[0].text, "buried in it");
}

#[test]
fn a_whole_file_scan_does_not_disturb_the_cached_viewport() {
    // `comments_in` reuses the same QueryCursor. If it left the cursor
    // unbounded — or bounded — the next highlight pass would inherit it.
    let src = long_source();
    let kws = vec!["TODO".to_string()];
    let mut e = SyntaxEngine::new(&src, "rs").unwrap();
    e.reparse(&src);
    e.set_viewport(0, 10);
    let before = e.styled_lines().to_vec();
    let _ = e.all_todo_markers(&kws);
    assert_eq!(
        e.styled_lines(),
        before.as_slice(),
        "the scan rewrote the cache"
    );
    assert_eq!(e.viewport(), Some(0..211), "the scan moved the viewport");
}

#[test]
fn markup_buffers_ignore_the_viewport() {
    // The markup backend highlights line-by-line and has no query to bound.
    let src = "# Title\n\nsome *text*\n";
    let mut e = SyntaxEngine::new(src, "md").unwrap();
    e.reparse(src);
    let before = e.styled_lines().to_vec();
    assert!(!e.set_viewport(0, 1), "there is no work for markup to do");
    assert_eq!(e.styled_lines(), before.as_slice());
    assert_eq!(e.viewport(), None);
}
