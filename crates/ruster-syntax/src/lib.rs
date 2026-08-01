pub mod highlighter;
pub mod markup;
pub mod theme;

use streaming_iterator::StreamingIterator;
use highlighter::Highlighter;
use markup::MarkupLang;
use ruster_render::{StyledLine, SyntaxStyle};

pub use theme::{
    base_group, default_fg_for, groups_for_lang, set_syntax_overrides, SyntaxOverrides,
};

/// Canonical keys of the languages that have real syntax-group highlighting
/// (a tree-sitter highlight query or the markup rules), in display order — the
/// list shown in the Settings syntax editor.
///
/// Every language `language_for_ext` accepts belongs here; a grammar without a
/// bundled query parses into a tree and then highlights nothing, which is the
/// gap `qcheck::every_parseable_language_has_a_highlight_query` now guards.
pub fn highlighted_languages() -> &'static [&'static str] {
    &[
        "rust", "python", "javascript", "typescript", "c", "lua", "json", "toml",
        "yaml", "scheme", "just", "markdown", "org",
    ]
}

#[derive(Debug)]
pub enum SyntaxError {
    UnsupportedLanguage,
    QueryError(String),
}

/// Tree-sitter-backed highlighting state.
struct TreeBackend {
    language: tree_sitter::Language,
    tree: tree_sitter::Tree,
    highlighter: Highlighter,
    source: String,
    bracket_depths: Vec<Option<usize>>,
    textobject_scm: &'static str,
    /// Kept so `todo_markers` can re-query the tree for `@comment` captures.
    highlight_scm: &'static str,
}

/// The highlighting strategy for a buffer: a tree-sitter grammar, or the
/// line-based markup rules for formats without a compatible grammar.
enum Backend {
    // Boxed: `TreeBackend` is far larger than the markup variant.
    Tree(Box<TreeBackend>),
    Markup(MarkupLang),
}

pub struct SyntaxEngine {
    backend: Backend,
    cached: Vec<StyledLine>,
}

impl SyntaxEngine {
    pub fn new(text: &str, file_ext: &str) -> Result<Self, SyntaxError> {
        let key = lang_key(file_ext);
        // Prefer a tree-sitter grammar; fall back to line-based markup.
        if let Some(language) = language_for_ext(file_ext) {
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(&language).map_err(|_| SyntaxError::QueryError("set_language".into()))?;
            let tree = parser.parse(text, None).ok_or(SyntaxError::QueryError("parse".into()))?;

            let (highlight_scm, textobject_scm) = query_files_for_lang(key);
            let mut highlighter = Highlighter::new(language.clone(), highlight_scm, key)
                .map_err(SyntaxError::QueryError)?;

            let bracket_depths = compute_bracket_depths(text);
            let cached = highlighter.highlight_lines(&tree, text, &bracket_depths);
            Ok(SyntaxEngine {
                backend: Backend::Tree(Box::new(TreeBackend {
                    language,
                    tree,
                    highlighter,
                    source: text.to_string(),
                    bracket_depths,
                    textobject_scm,
                    highlight_scm,
                })),
                cached,
            })
        } else if let Some(mlang) = markup::markup_lang(key) {
            let cached = markup::highlight_markup(mlang, text);
            Ok(SyntaxEngine { backend: Backend::Markup(mlang), cached })
        } else {
            Err(SyntaxError::UnsupportedLanguage)
        }
    }

    pub fn reparse(&mut self, text: &str) {
        match &mut self.backend {
            Backend::Tree(tb) => {
                let mut parser = tree_sitter::Parser::new();
                let _ = parser.set_language(&tb.language);
                if let Some(tree) = parser.parse(text, None) {
                    tb.tree = tree;
                    tb.source = text.to_string();
                    tb.bracket_depths = compute_bracket_depths(text);
                    self.cached =
                        tb.highlighter.highlight_lines(&tb.tree, text, &tb.bracket_depths);
                }
            }
            Backend::Markup(mlang) => {
                self.cached = markup::highlight_markup(*mlang, text);
            }
        }
    }

    /// Recompute the cached highlights with the currently-installed
    /// [`set_syntax_overrides`](crate::theme::set_syntax_overrides) — no reparse
    /// for tree-sitter buffers (reuses the existing tree). Call after the syntax
    /// colors change. `text` is only needed for the line-based markup backend.
    pub fn recolor(&mut self, text: &str) {
        match &mut self.backend {
            Backend::Tree(tb) => {
                self.cached =
                    tb.highlighter.highlight_lines(&tb.tree, &tb.source, &tb.bracket_depths);
            }
            Backend::Markup(mlang) => {
                self.cached = markup::highlight_markup(*mlang, text);
            }
        }
    }

    pub fn highlight_line(&self, line_idx: usize) -> Vec<(usize, usize, SyntaxStyle)> {
        self.cached
            .get(line_idx)
            .map(|sl| sl.highlights.clone())
            .unwrap_or_default()
    }

    pub fn num_lines(&self) -> usize {
        self.cached.len()
    }

    pub fn styled_lines(&self) -> &[StyledLine] {
        &self.cached
    }

    pub fn ts_textobject(&self, kind: char, target: char, cursor: usize) -> Option<(usize, usize)> {
        // Only tree-sitter buffers have structural text objects.
        let tb = match &self.backend {
            Backend::Tree(tb) => tb,
            Backend::Markup(_) => return None,
        };
        if tb.textobject_scm.is_empty() { return None; }
        let query_name = match (kind, target) {
            ('i', 'f') => "function.inner",
            ('a', 'f') => "function.outer",
            ('i', 'c') => "class.inner",
            ('a', 'c') => "class.outer",
            ('i', 'l') => "loop.inner",
            ('a', 'l') => "loop.outer",
            ('i', 'a') => "parameter.inner",
            ('a', 'a') => "parameter.outer",
            _ => return None,
        };

        let query = tree_sitter::Query::new(&tb.language, tb.textobject_scm).ok()?;
        let mut cursor_q = tree_sitter::QueryCursor::new();
        let mut matches = cursor_q.matches(&query, tb.tree.root_node(), tb.source.as_bytes());

        while let Some(m) = matches.next() {
            for cap in m.captures {
                let name = query.capture_names()[cap.index as usize];
                if name == query_name {
                    let start = byte_to_char_pos(&tb.source, cap.node.byte_range().start);
                    let end = byte_to_char_pos(&tb.source, cap.node.byte_range().end);
                    if start <= cursor && cursor <= end {
                        return Some((start, end));
                    }
                }
            }
        }
        None
    }

    /// Colour every `TODO`-class keyword found in a comment.
    ///
    /// Call after [`reparse`](Self::reparse) or [`recolor`](Self::recolor), which
    /// rebuild the cached lines and drop the overlay. Applied here rather than in
    /// the highlight query so the keyword set stays configurable at runtime —
    /// a query would bake it in per language.
    pub fn overlay_todo_highlights(&mut self, keywords: &[String], style: SyntaxStyle) {
        if keywords.is_empty() {
            return;
        }
        let markers = self.todo_markers(keywords);
        for m in markers {
            let Some(line) = self.cached.get_mut(m.line) else { continue };
            // Push last so it wins over the comment colour underneath.
            line.highlights.push((m.col, m.keyword.chars().count(), style));
        }
    }

    /// `TODO`-class markers, taken from the syntax tree's `@comment` captures.
    ///
    /// Sourcing the ranges from the tree rather than scanning text is what keeps
    /// `"TODO: not a real todo"` in a string literal from matching. Buffers with
    /// no grammar return nothing rather than guessing.
    pub fn todo_markers(&self, keywords: &[String]) -> Vec<TodoMarker> {
        let Backend::Tree(tb) = &self.backend else { return Vec::new() };
        let Ok(query) = tree_sitter::Query::new(&tb.language, tb.highlight_scm) else {
            return Vec::new();
        };
        let comment_idx: Vec<u32> = query
            .capture_names()
            .iter()
            .enumerate()
            .filter(|(_, n)| **n == "comment")
            .map(|(i, _)| i as u32)
            .collect();
        if comment_idx.is_empty() {
            return Vec::new();
        }

        // Line starts, so a byte offset can become (line, col) without rescanning.
        let mut line_start: Vec<usize> = vec![0];
        for (i, b) in tb.source.bytes().enumerate() {
            if b == b'\n' {
                line_start.push(i + 1);
            }
        }

        let mut out = Vec::new();
        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&query, tb.tree.root_node(), tb.source.as_bytes());
        while let Some(m) = matches.next() {
            for cap in m.captures {
                if !comment_idx.contains(&cap.index) {
                    continue;
                }
                let range = cap.node.byte_range();
                let Some(text) = tb.source.get(range.clone()) else { continue };
                for kw in keywords {
                    let mut from = 0;
                    while let Some(rel) = text[from..].find(kw.as_str()) {
                        let at = from + rel;
                        from = at + kw.len();
                        // Whole word only: `TODOS` and `XTODO` are not markers.
                        let before_ok = at == 0
                            || !text[..at].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_');
                        let after = &text[at + kw.len()..];
                        let after_ok = !after.chars().next().is_some_and(|c| c.is_alphanumeric() || c == '_');
                        if !(before_ok && after_ok) {
                            continue;
                        }
                        let abs = range.start + at;
                        let line = line_start.partition_point(|&s| s <= abs) - 1;
                        let col = tb.source[line_start[line]..abs].chars().count();
                        let rest = after.trim_start_matches([':', ' ', '\t']);
                        let rest = rest.lines().next().unwrap_or("").trim_end();
                        out.push(TodoMarker {
                            keyword: kw.clone(),
                            line,
                            col,
                            text: rest.to_string(),
                        });
                    }
                }
            }
        }
        out.sort_by_key(|m| (m.line, m.col));
        out
    }
}

fn byte_to_char_pos(source: &str, byte: usize) -> usize {
    source.char_indices().position(|(i, _)| i >= byte).unwrap_or(source.chars().count())
}

/// A `TODO`-class marker found inside a comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoMarker {
    pub keyword: String,
    /// 0-based line in the file.
    pub line: usize,
    /// 0-based character column of the keyword.
    pub col: usize,
    /// The comment text following the keyword, trimmed.
    pub text: String,
}

/// The keywords recognised when none are configured.
pub const DEFAULT_TODO_KEYWORDS: &[&str] = &["TODO", "FIXME", "HACK", "NOTE", "XXX"];

pub fn language_for_ext(ext: &str) -> Option<tree_sitter::Language> {
    match ext {
        "rs"              => Some(tree_sitter_rust::LANGUAGE.into()),
        "py"              => Some(tree_sitter_python::LANGUAGE.into()),
        "js" | "mjs" | "cjs" => Some(tree_sitter_javascript::LANGUAGE.into()),
        "ts" | "tsx"      => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
        "c" | "h"         => Some(tree_sitter_c::LANGUAGE.into()),
        "json"            => Some(tree_sitter_json::LANGUAGE.into()),
        "toml"            => Some(tree_sitter_toml_ng::LANGUAGE.into()),
        "yaml" | "yml"    => Some(tree_sitter_yaml::LANGUAGE.into()),
        "lua"             => Some(tree_sitter_lua::LANGUAGE.into()),
        "scm" | "ss" | "sld" | "sls" | "sch" | "scheme" => Some(tree_sitter_scheme::LANGUAGE.into()),
        "just" | "justfile" => Some(tree_sitter_just::LANGUAGE.into()),
        _ => None,
    }
}

/// Canonical language key for a file extension, or "" if unsupported.
pub fn lang_key(ext: &str) -> &'static str {
    match ext {
        "rs" => "rust",
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "tsx" => "typescript",
        "c" | "h" => "c",
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "lua" => "lua",
        "scm" | "ss" | "sld" | "sls" | "sch" | "scheme" => "scheme",
        "just" | "justfile" => "just",
        "md" | "markdown" | "mdown" | "mkd" => "markdown",
        "org" => "org",
        _ => "",
    }
}

/// The lookup key to use for a path: its extension when that maps to a known
/// language, otherwise the (lowercased, dot-stripped) file name — so
/// extensionless files like `justfile` / `.justfile` / `Justfile` are recognised.
pub fn lang_ext_for_path(path: &std::path::Path) -> String {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let lower = ext.to_lowercase();
        if !lang_key(&lower).is_empty() {
            return lower;
        }
    }
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.to_lowercase().trim_start_matches('.').to_string())
        .unwrap_or_default()
}

/// Highlight and textobject query sources for a language key. Languages without
/// bundled queries return empty strings — no (rather than wrong) highlighting;
/// rainbow brackets still apply since they are computed separately.
fn query_files_for_lang(key: &str) -> (&'static str, &'static str) {
    match key {
        "rust" => (
            include_str!("../queries/rust/highlights.scm"),
            include_str!("../queries/rust/textobjects.scm"),
        ),
        "python" => (include_str!("../queries/python/highlights.scm"), ""),
        "javascript" => (include_str!("../queries/javascript/highlights.scm"), ""),
        "typescript" => (include_str!("../queries/typescript/highlights.scm"), ""),
        "json" => (include_str!("../queries/json/highlights.scm"), ""),
        "lua" => (include_str!("../queries/lua/highlights.scm"), ""),
        "toml" => (include_str!("../queries/toml/highlights.scm"), ""),
        "yaml" => (include_str!("../queries/yaml/highlights.scm"), ""),
        "c" => (include_str!("../queries/c/highlights.scm"), ""),
        "scheme" => (include_str!("../queries/scheme/highlights.scm"), ""),
        "just" => (include_str!("../queries/just/highlights.scm"), ""),
        _ => ("", ""),
    }
}

fn compute_bracket_depths(source: &str) -> Vec<Option<usize>> {
    let len = source.len();
    let mut depths: Vec<Option<usize>> = vec![None; len];
    let mut d = 0usize;
    for (i, ch) in source.char_indices() {
        match ch {
            '(' | '{' | '[' => {
                depths[i] = Some(d);
                d += 1;
            }
            ')' | '}' | ']' => {
                d = d.saturating_sub(1);
                depths[i] = Some(d);
            }
            _ => {}
        }
    }
    depths
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kws() -> Vec<String> {
        super::DEFAULT_TODO_KEYWORDS.iter().map(|s| s.to_string()).collect()
    }

    /// The whole point of sourcing ranges from the tree: a keyword inside a
    /// string literal is not a marker, however much it looks like one.
    #[test]
    fn todo_markers_come_from_comments_not_string_literals() {
        let src = "// TODO: real one\nfn main() {\n    let s = \"TODO: not a marker\";\n    let t = \"FIXME also not\";\n}\n// FIXME: second real one\n";
        let e = SyntaxEngine::new(src, "rs").expect("rust grammar");
        let m = e.todo_markers(&kws());
        assert_eq!(m.len(), 2, "only the two comments count, got {m:?}");
        assert_eq!(m[0].keyword, "TODO");
        assert_eq!(m[0].line, 0);
        assert_eq!(m[0].text, "real one");
        assert_eq!(m[1].keyword, "FIXME");
        assert_eq!(m[1].line, 5);
        assert_eq!(m[1].text, "second real one");
    }

    #[test]
    fn todo_markers_match_whole_words_only() {
        let src = "// TODOS are not TODO markers, and XTODO is not either\n";
        let e = SyntaxEngine::new(src, "rs").expect("rust grammar");
        let m = e.todo_markers(&kws());
        assert_eq!(m.len(), 1, "only the standalone TODO, got {m:?}");
    }

    #[test]
    fn todo_markers_report_line_and_column() {
        let src = "fn a() {}\n    // HACK: indented\n";
        let e = SyntaxEngine::new(src, "rs").expect("rust grammar");
        let m = e.todo_markers(&kws());
        assert_eq!(m.len(), 1);
        assert_eq!((m[0].line, m[0].col), (1, 7), "0-based line and char column");
        assert_eq!(m[0].text, "indented");
    }

    #[test]
    fn todo_markers_handle_block_comments_and_multiple_per_comment() {
        let src = "/* TODO: one\n   FIXME: two */\n";
        let e = SyntaxEngine::new(src, "rs").expect("rust grammar");
        let m = e.todo_markers(&kws());
        assert_eq!(m.len(), 2, "both keywords inside one block comment: {m:?}");
        assert_eq!(m[0].line, 0);
        assert_eq!(m[1].line, 1);
    }

    #[test]
    fn todo_markers_respect_the_configured_keyword_set() {
        let src = "// TODO: ignored\n// REVIEW: wanted\n";
        let e = SyntaxEngine::new(src, "rs").expect("rust grammar");
        let m = e.todo_markers(&["REVIEW".to_string()]);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].keyword, "REVIEW");
    }

    /// A buffer with no grammar returns nothing rather than falling back to a
    /// text scan, which would reintroduce the string-literal false positives.
    #[test]
    fn todo_markers_are_empty_without_a_grammar() {
        let e = SyntaxEngine::new("# TODO: markdown has no comment capture\n", "md")
            .expect("markup backend");
        assert!(e.todo_markers(&kws()).is_empty());
    }

    #[test]
    fn supports_rust_source() {
        let engine = SyntaxEngine::new("fn main() {}", "rs");
        assert!(engine.is_ok());
    }

    #[test]
    fn unsupported_extension_returns_err() {
        let engine = SyntaxEngine::new("hello", "xyz");
        assert!(matches!(engine, Err(SyntaxError::UnsupportedLanguage)));
    }

    #[test]
    fn python_uses_python_queries_and_highlights() {
        // Previously this failed because Rust queries were applied to Python.
        let engine = SyntaxEngine::new("def foo():\n    return 1\n", "py").unwrap();
        let styled = engine.styled_lines();
        let has_highlight = styled.iter().any(|l| !l.highlights.is_empty());
        assert!(has_highlight, "python source should produce highlights");
    }

    #[test]
    fn json_highlights_without_error() {
        let engine = SyntaxEngine::new("{\"a\": 1, \"b\": true}", "json").unwrap();
        assert!(engine.styled_lines().iter().any(|l| !l.highlights.is_empty()));
    }

    #[test]
    fn override_recolors_a_group_and_recolor_reapplies() {
        use ruster_render::Color;
        use std::collections::HashMap;

        let magenta = Color::Rgb(255, 0, 255);
        let mut map: SyntaxOverrides = HashMap::new();
        let mut rust = HashMap::new();
        rust.insert("keyword".to_string(), magenta);
        map.insert("rust".to_string(), rust);
        set_syntax_overrides(map);

        // A freshly-built engine picks up the override…
        let src = "fn main() {}";
        let engine = SyntaxEngine::new(src, "rs").unwrap();
        let has_magenta = |e: &SyntaxEngine| {
            e.styled_lines().iter().any(|l| l.highlights.iter().any(|(_, _, s)| s.fg == magenta))
        };
        assert!(has_magenta(&engine), "override not applied to `fn` keyword");

        // …and clearing + recolor() drops it without a reparse.
        set_syntax_overrides(SyntaxOverrides::new());
        let mut engine = engine;
        engine.recolor(src);
        assert!(!has_magenta(&engine), "recolor did not drop the override");
    }

    #[test]
    fn groups_for_markup_vs_code() {
        assert!(groups_for_lang("markdown").contains(&"heading"));
        assert!(groups_for_lang("rust").contains(&"keyword"));
        assert!(!groups_for_lang("rust").contains(&"heading"));
    }

    #[test]
    fn all_bundled_queries_compile_and_highlight() {
        // (extension, sample source) — each must build (query compiles) and
        // produce at least one highlight, proving the query matches the grammar.
        let cases = [
            ("rs", "fn main() { let x = 1; }"),
            ("py", "def f(x):\n    return x + 1\n"),
            ("json", "{\"a\": 1, \"b\": true, \"c\": null}"),
            ("lua", "local function f(x)\n  return x + 1\nend\n"),
            ("c", "int main(void) {\n  int x = 1;\n  return x;\n}\n"),
            ("toml", "# c\n[table]\nkey = \"value\"\nn = 42\nb = true\n"),
            ("yaml", "# c\nname: value\ncount: 3\nflag: true\n"),
            ("scm", "; comment\n(define (square x)\n  (* x x))\n"),
            ("justfile", "# comment\nbuild:\n    cargo build\n"),
        ];
        for (ext, src) in cases {
            let engine = SyntaxEngine::new(src, ext)
                .unwrap_or_else(|e| panic!("{ext} query failed to build: {e:?}"));
            let has = engine.styled_lines().iter().any(|l| !l.highlights.is_empty());
            assert!(has, "{ext} produced no highlights");
        }
    }

    #[test]
    fn extensionless_justfiles_are_detected_by_name() {
        use std::path::Path;
        for name in ["justfile", "Justfile", ".justfile", "/proj/justfile"] {
            assert_eq!(
                lang_key(&lang_ext_for_path(Path::new(name))),
                "just",
                "{name} should resolve to just"
            );
        }
        // A normal extension still wins.
        assert_eq!(lang_key(&lang_ext_for_path(Path::new("src/main.rs"))), "rust");
        // *.just files work via the extension.
        assert_eq!(lang_key(&lang_ext_for_path(Path::new("tasks.just"))), "just");
        // Markdown and Org resolve to their markup keys.
        assert_eq!(lang_key(&lang_ext_for_path(Path::new("README.md"))), "markdown");
        assert_eq!(lang_key(&lang_ext_for_path(Path::new("notes.org"))), "org");
        // Truly unknown files resolve to nothing.
        assert_eq!(lang_key(&lang_ext_for_path(Path::new("photo.xyz"))), "");
    }

    #[test]
    fn markdown_and_org_highlight_via_the_markup_backend() {
        for (ext, src) in [("md", "# Title\n**bold**\n"), ("org", "* Head\n/em/\n")] {
            let engine = SyntaxEngine::new(src, ext).unwrap();
            let styled = engine.styled_lines();
            assert!(
                styled.iter().any(|l| !l.highlights.is_empty()),
                "{ext} should produce highlights"
            );
            // Markup buffers expose no structural text objects.
            assert!(engine.ts_textobject('i', 'f', 0).is_none());
        }
    }

    #[test]
    fn markup_reparse_updates_highlights() {
        let mut engine = SyntaxEngine::new("plain\n", "md").unwrap();
        assert!(engine.styled_lines()[0].highlights.is_empty());
        engine.reparse("# now a heading\n");
        assert!(!engine.styled_lines()[0].highlights.is_empty());
    }

    #[test]
    fn language_without_queries_still_builds() {
        // yaml has no bundled query yet — engine builds with an empty query
        // (no syntax highlights, but no error either).
        let engine = SyntaxEngine::new("a: 1\n", "yaml");
        assert!(engine.is_ok());
    }

    #[test]
    fn reparse_does_not_panic() {
        let mut engine = SyntaxEngine::new("fn main() {}", "rs").unwrap();
        engine.reparse("fn main() { let x = 1; }");
    }

    #[test]
    fn bracket_depths_basic() {
        let depths = compute_bracket_depths("(a(b))");
        assert_eq!(depths[0], Some(0));
        assert_eq!(depths[2], Some(1));
        assert_eq!(depths[4], Some(1));
        assert_eq!(depths[5], Some(0));
    }

    #[test]
    fn styled_lines_returns_correct_count() {
        let engine = SyntaxEngine::new("fn main() {\n  let x = 1;\n}", "rs").unwrap();
        assert_eq!(engine.styled_lines().len(), 3);
    }

    #[test]
    fn empty_file_produces_one_empty_line() {
        let engine = SyntaxEngine::new("", "rs").unwrap();
        assert_eq!(engine.styled_lines().len(), 1);
        assert_eq!(engine.styled_lines()[0].text, "");
    }

    #[test]
    fn ts_textobject_inner_function() {
        let engine = SyntaxEngine::new("fn foo() { let x = 1; }", "rs").unwrap();
        let result = engine.ts_textobject('i', 'f', 15);
        assert!(result.is_some());
        let (start, end) = result.unwrap();
        assert!(start < end);
    }

    #[test]
    fn styled_lines_strip_trailing_newlines() {
        let engine = SyntaxEngine::new("hello\nworld", "rs").unwrap();
        let lines = engine.styled_lines();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text.chars().count(), 5);
        assert_eq!(lines[1].text.chars().count(), 5);
    }

    #[test]
    fn styled_lines_with_trailing_newline_has_empty_last_line() {
        let engine = SyntaxEngine::new("hello\nworld\n", "rs").unwrap();
        let lines = engine.styled_lines();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].text, "hello");
        assert_eq!(lines[1].text, "world");
        assert_eq!(lines[2].text, "");
    }

    #[test]
    fn rainbow_bracket_colors_applied() {
        let engine = SyntaxEngine::new("(a(b)c)", "rs").unwrap();
        let styled = engine.styled_lines();
        assert_eq!(styled.len(), 1);
        let line = &styled[0];
        let bracket_highlights: Vec<_> = line.highlights.iter()
            .filter(|(s, _, _)| *s == 0 || *s == 6)
            .collect();
        assert!(!bracket_highlights.is_empty(), "expected bracket highlights");
    }
}

#[cfg(test)]
mod qcheck;
