//! Guards on the bundled highlight queries.
//!
//! A query that fails to compile degrades to *no* highlighting rather than
//! erroring visibly, and a query that compiles can still match nothing — so
//! both properties are asserted here.

use crate::SyntaxEngine;

#[test]
fn every_bundled_query_compiles_against_its_grammar() {
    let cases: &[(&str, &str, tree_sitter::Language)] = &[
        ("rust",       include_str!("../queries/rust/highlights.scm"),       tree_sitter_rust::LANGUAGE.into()),
        ("python",     include_str!("../queries/python/highlights.scm"),     tree_sitter_python::LANGUAGE.into()),
        ("javascript", include_str!("../queries/javascript/highlights.scm"), tree_sitter_javascript::LANGUAGE.into()),
        ("typescript", include_str!("../queries/typescript/highlights.scm"), tree_sitter_typescript::LANGUAGE_TSX.into()),
        ("c",          include_str!("../queries/c/highlights.scm"),          tree_sitter_c::LANGUAGE.into()),
        ("json",       include_str!("../queries/json/highlights.scm"),       tree_sitter_json::LANGUAGE.into()),
        ("toml",       include_str!("../queries/toml/highlights.scm"),       tree_sitter_toml_ng::LANGUAGE.into()),
        ("yaml",       include_str!("../queries/yaml/highlights.scm"),       tree_sitter_yaml::LANGUAGE.into()),
        ("lua",        include_str!("../queries/lua/highlights.scm"),        tree_sitter_lua::LANGUAGE.into()),
        ("scheme",     include_str!("../queries/scheme/highlights.scm"),     tree_sitter_scheme::LANGUAGE.into()),
        ("just",       include_str!("../queries/just/highlights.scm"),       tree_sitter_just::LANGUAGE.into()),
    ];
    for (name, scm, lang) in cases {
        if let Err(e) = tree_sitter::Query::new(lang, scm) {
            panic!("{name} query failed to compile: {e:?}");
        }
    }
}

/// Distinct foreground colors on a line — a "did more than one group match"
/// signal that doesn't depend on the active theme's exact values.
fn distinct_colors(engine: &SyntaxEngine, line: usize) -> usize {
    let mut seen: Vec<ruster_render::Color> = Vec::new();
    for (_, _, style) in engine.highlight_line(line) {
        if !seen.contains(&style.fg) {
            seen.push(style.fg);
        }
    }
    seen.len()
}

#[test]
fn javascript_highlights_keywords_strings_and_functions() {
    let src = "// a comment\nconst greet = (name) => {\n  return `hi ${name}`;\n};\nclass Foo extends Bar {}\n";
    let e = SyntaxEngine::new(src, "js").expect("js engine builds");
    assert!(!e.highlight_line(0).is_empty(), "comment line is highlighted");
    assert!(!e.highlight_line(1).is_empty(), "const/arrow line is highlighted");
    assert!(!e.highlight_line(4).is_empty(), "class line is highlighted");
    assert!(distinct_colors(&e, 1) > 1, "more than one syntax group on the const line");
}

#[test]
fn typescript_highlights_types_and_interfaces() {
    let src = "interface User {\n  name: string;\n}\nfunction load(id: number): User {\n  return null;\n}\n";
    let e = SyntaxEngine::new(src, "ts").expect("ts engine builds");
    assert!(!e.highlight_line(0).is_empty(), "interface declaration is highlighted");
    assert!(!e.highlight_line(1).is_empty(), "property signature is highlighted");
    assert!(!e.highlight_line(3).is_empty(), "typed function signature is highlighted");
    assert!(distinct_colors(&e, 3) > 1, "more than one syntax group on the signature line");
}

/// `.tsx` shares the TSX grammar with `.ts`, so JSX must not break the query.
#[test]
fn tsx_files_highlight_without_error() {
    let src = "const App = () => <div className=\"x\">hi</div>;\n";
    let e = SyntaxEngine::new(src, "tsx").expect("tsx engine builds");
    assert!(!e.highlight_line(0).is_empty(), "jsx line is highlighted");
}

/// Regression for the gap these queries closed: a language the parser accepts
/// but that ships no query parses into a tree and then highlights nothing.
#[test]
fn every_parseable_language_has_a_highlight_query() {
    for ext in ["rs", "py", "js", "ts", "c", "json", "toml", "yaml", "lua", "scm", "just"] {
        assert!(crate::language_for_ext(ext).is_some(), "{ext} has a grammar");
        let (scm, _) = crate::query_files_for_lang(crate::lang_key(ext));
        assert!(!scm.is_empty(), "{ext} parses but ships no highlight query");
    }
}
