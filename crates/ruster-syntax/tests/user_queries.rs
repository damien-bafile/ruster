//! End-to-end cover for user-supplied highlight queries.
//!
//! The unit tests hand `load_queries` a directory directly, which leaves the
//! part that resolves `~/.config/ruster/queries` from the environment
//! unexercised — the part that decides whether any of this works when the
//! editor actually runs.
//!
//! This lives in `tests/` because it sets `XDG_CONFIG_HOME`, and an integration
//! test gets its own process. Everything runs in one `#[test]` for the same
//! reason: the variable is process-global, so two tests setting it would race.

use ruster_syntax::SyntaxEngine;

/// Deliberately bracket-free: rainbow brackets are computed separately from the
/// query, so a source containing any would keep producing spans even when the
/// query matches nothing, and hide whether the override took effect.
const SRC: &str = "const X: i32 = 1;\n";

fn write_query(root: &std::path::Path, body: &str) {
    let dir = root.join("ruster").join("queries").join("rust");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("highlights.scm"), body).unwrap();
}

/// The number of highlight spans across the buffer — how much of the query
/// actually matched.
fn span_count(engine: &SyntaxEngine) -> usize {
    engine.styled_lines().iter().map(|l| l.highlights.len()).sum()
}

#[test]
fn user_queries_are_discovered_read_and_degraded_from_the_config_dir() {
    let root = std::env::temp_dir().join("ruster_user_queries_e2e");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    // Baseline: no user directory, so the built-in query highlights the source.
    std::env::set_var("XDG_CONFIG_HOME", &root);
    let builtin = SyntaxEngine::new(SRC, "rs").expect("rust is supported");
    let builtin_spans = span_count(&builtin);
    assert!(builtin_spans > 0, "the built-in query highlights something");
    assert!(builtin.warnings().is_empty(), "the normal path is silent");

    // An empty user query is valid and matches nothing. Highlighting collapses,
    // which proves the file was found and used rather than silently ignored.
    write_query(&root, "");
    let empty = SyntaxEngine::new(SRC, "rs").expect("an empty query still builds");
    assert_eq!(span_count(&empty), 0, "the user query replaced the built-in");
    assert!(empty.warnings().is_empty(), "an empty query is legal, not an error");

    // A query tree-sitter rejects must not take the editor down: it falls back
    // to the built-in and says so.
    write_query(&root, "(((( not a query @@@");
    let broken = SyntaxEngine::new(SRC, "rs").expect("a malformed query is survivable");
    assert_eq!(
        span_count(&broken),
        builtin_spans,
        "fell back to the built-in, so highlighting is unchanged"
    );
    let warnings = broken.warnings();
    assert_eq!(warnings.len(), 1, "exactly one complaint: {warnings:?}");
    assert!(warnings[0].contains("built-in"), "says what it did: {:?}", warnings[0]);

    // The engine keeps the query it is *using* for later re-queries, not the
    // text it was handed. TODO markers re-run it against the tree, so storing
    // the rejected query would break them on exactly the buffers that already
    // warned — silently, since highlighting itself looks fine.
    let kws = vec!["TODO".to_string()];
    let with_todo = SyntaxEngine::new("// TODO: still found\nconst X: i32 = 1;\n", "rs")
        .expect("builds despite the broken user query");
    assert_eq!(
        with_todo.todo_markers(&kws).len(),
        1,
        "the fallback query still resolves comment captures"
    );

    // A working custom query takes effect and stays quiet.
    write_query(&root, "(integer_literal) @number");
    let custom = SyntaxEngine::new(SRC, "rs").expect("a valid query builds");
    assert_eq!(span_count(&custom), 1, "only the `1` is highlighted now");
    assert!(custom.warnings().is_empty());

    let _ = std::fs::remove_dir_all(&root);
}
