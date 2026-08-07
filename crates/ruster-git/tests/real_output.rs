//! Validates the parser's line mapping against output from a real `git diff`,
//! captured from a fixture whose contents are known.
use ruster_git::{parse_hunks, Hunk, HunkKind};

#[test]
fn maps_real_git_output_onto_the_right_working_file_lines() {
    // From a file whose 0-based working copy is:
    //   0 one / 1 TWO / 2 three / 3 four / 4 AAA / 5 BBB / 6 five
    // where TWO replaced "two", AAA+BBB were inserted, and "six" was deleted.
    let diff = "@@ -2 +2 @@ one\n@@ -4,0 +5,2 @@ four\n@@ -6 +7,0 @@ five\n";
    let h = parse_hunks(diff);
    assert_eq!(
        h,
        [
            // "TWO" is 0-based line 1.
            Hunk {
                kind: HunkKind::Modified,
                start: 1,
                count: 1
            },
            // "AAA"/"BBB" are 0-based lines 4 and 5.
            Hunk {
                kind: HunkKind::Added,
                start: 4,
                count: 2
            },
            // "six" was deleted at end-of-file; the sign sits on the last
            // remaining line, 0-based 6 ("five"), not one past it.
            Hunk {
                kind: HunkKind::Removed,
                start: 6,
                count: 0
            },
        ]
    );
    assert_eq!(
        h[1].lines().collect::<Vec<_>>(),
        vec![4, 5],
        "the inserted lines"
    );
}
