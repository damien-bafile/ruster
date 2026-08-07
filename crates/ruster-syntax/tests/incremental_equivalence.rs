//! An incremental reparse must produce exactly what a full parse would.
//!
//! This is the only property that matters for incremental parsing, and the one
//! that fails silently: a wrong `InputEdit` leaves tree-sitter reusing nodes
//! that have moved, so the tree is subtly wrong, the highlighting is subtly
//! wrong, and nothing errors. Comparing against a full parse of the same text
//! is the only way to know.

use ruster_core::buffer::Buffer;
use ruster_syntax::SyntaxEngine;

/// Apply `edits` to a buffer incrementally and compare the highlighting against
/// a fresh engine built from the final text.
fn assert_matches_full_parse(initial: &str, edits: &[(usize, &str, usize)]) {
    let mut buf = Buffer::from_str(initial);
    let mut engine = SyntaxEngine::new(initial, "rs").expect("rust");

    for (at, insert, delete_len) in edits {
        if *delete_len > 0 {
            buf.delete(*at..*at + *delete_len);
        }
        if !insert.is_empty() {
            buf.insert(*at, insert);
        }
        let text = buf.to_string();
        let recorded = buf.take_edits();
        engine.reparse_with_edits(&text, &recorded);

        let fresh = SyntaxEngine::new(&text, "rs").expect("rust");
        assert_eq!(
            engine.styled_lines(),
            fresh.styled_lines(),
            "incremental differs from a full parse after {:?}\n--- text ---\n{text}",
            (at, insert, delete_len)
        );
    }
}

#[test]
fn a_single_insert_matches() {
    assert_matches_full_parse("fn main() {\n    let x = 1;\n}\n", &[(16, "y", 0)]);
}

#[test]
fn inserting_a_whole_line_matches() {
    assert_matches_full_parse(
        "fn main() {\n    let x = 1;\n}\n",
        &[(12, "    const N: i32 = 9;\n", 0)],
    );
}

#[test]
fn a_delete_matches() {
    // Remove `let ` from the middle line.
    assert_matches_full_parse("fn main() {\n    let x = 1;\n}\n", &[(16, "", 4)]);
}

#[test]
fn a_replacement_matches() {
    assert_matches_full_parse("fn main() {\n    let x = 1;\n}\n", &[(16, "const", 3)]);
}

/// Several edits between parses is the paste / macro case, and the one where
/// applying them in the wrong order or skipping one shows up.
#[test]
fn many_edits_before_one_reparse_match() {
    let mut buf = Buffer::from_str("fn a() {}\n");
    let mut engine = SyntaxEngine::new("fn a() {}\n", "rs").unwrap();

    buf.insert(9, "\nfn b() {}");
    buf.insert(0, "// top\n");
    buf.delete(3..6);

    let text = buf.to_string();
    engine.reparse_with_edits(&text, &buf.take_edits());
    let fresh = SyntaxEngine::new(&text, "rs").unwrap();
    assert_eq!(engine.styled_lines(), fresh.styled_lines(), "text:\n{text}");
}

/// Editing across a line boundary is where the row/column points matter most.
#[test]
fn edits_spanning_newlines_match() {
    assert_matches_full_parse(
        "fn a() {\n    one();\n    two();\n}\n",
        &[(18, "\n    inserted();", 0), (9, "", 12)],
    );
}

/// Multi-byte characters make byte offsets and char offsets disagree, which is
/// exactly where an offset bug hides.
#[test]
fn unicode_in_the_buffer_matches() {
    assert_matches_full_parse(
        "fn main() {\n    let s = \"café ☕\";\n}\n",
        &[(28, "x", 0), (12, "    // ünïcödé\n", 0)],
    );
}

/// A long run of single-character inserts, the way typing actually happens.
#[test]
fn typing_character_by_character_matches() {
    let src = "fn main() {\n    \n}\n";
    let mut buf = Buffer::from_str(src);
    let mut engine = SyntaxEngine::new(src, "rs").unwrap();

    for (i, ch) in "let total = 42;".chars().enumerate() {
        buf.insert(16 + i, &ch.to_string());
        let text = buf.to_string();
        engine.reparse_with_edits(&text, &buf.take_edits());
        let fresh = SyntaxEngine::new(&text, "rs").unwrap();
        assert_eq!(engine.styled_lines(), fresh.styled_lines(), "after {ch:?}:\n{text}");
    }
}

/// Losing the edits must cost speed, not correctness.
#[test]
fn an_empty_edit_list_still_produces_the_right_result() {
    let mut buf = Buffer::from_str("fn a() {}\n");
    let mut engine = SyntaxEngine::new("fn a() {}\n", "rs").unwrap();
    buf.insert(0, "const X: i32 = 1;\n");
    let text = buf.to_string();
    let _ = buf.take_edits(); // thrown away on purpose

    engine.reparse_with_edits(&text, &[]);
    let fresh = SyntaxEngine::new(&text, "rs").unwrap();
    assert_eq!(engine.styled_lines(), fresh.styled_lines());
}

/// The small cases above cannot detect a bad `InputEdit`: on a five-line file
/// tree-sitter re-parses nearly everything regardless, so wrong coordinates
/// still yield the right tree. Verified by mutation — corrupting the edits left
/// all of them passing.
///
/// Reuse only matters at scale, so this repeats the check on a real source file
/// with edits far apart. Mutation testing against *this* one shows it has
/// exactly the sensitivity that matters:
///
/// - `new_end_byte + 1` — **fails**. Under-invalidation leaves tree-sitter
///   reusing nodes that moved, and the tree is wrong.
/// - `start_byte: 0` — **passes**. Over-invalidation makes it re-parse from the
///   top: slower, never incorrect.
///
/// That asymmetry is the safety property worth having. A caller who loses track
/// of the edits pays in time, not correctness — which is also why
/// `reparse_with_edits(text, &[])` is defined as a full parse.
#[test]
fn incremental_matches_a_full_parse_on_a_large_file() {
    const BIG: &str = include_str!("../src/lib.rs");
    let mut buf = Buffer::from_str(BIG);
    let mut engine = SyntaxEngine::new(BIG, "rs").expect("rust");

    // Edits scattered through the file, including one near the end where a
    // stale prefix would go unnoticed by a check that only looked at the top.
    let len = buf.to_string().chars().count();
    let spots = [len / 10, len / 2, len - (len / 8), len / 4];

    for (i, at) in spots.into_iter().enumerate() {
        buf.insert(at, "\n// inserted marker\n");
        let text = buf.to_string();
        engine.reparse_with_edits(&text, &buf.take_edits());

        let fresh = SyntaxEngine::new(&text, "rs").expect("rust");
        let (inc, full) = (engine.styled_lines(), fresh.styled_lines());
        assert_eq!(inc.len(), full.len(), "line count differs after edit {i}");
        for (n, (a, b)) in inc.iter().zip(full).enumerate() {
            assert_eq!(a, b, "line {n} differs after edit {i} at char {at}");
        }
    }
}
