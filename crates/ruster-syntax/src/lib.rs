pub mod highlighter;
pub mod theme;

use streaming_iterator::StreamingIterator;
use highlighter::Highlighter;
use ruster_render::{StyledLine, SyntaxStyle};

#[derive(Debug)]
pub enum SyntaxError {
    UnsupportedLanguage,
    QueryError(String),
}

pub struct SyntaxEngine {
    language: tree_sitter::Language,
    tree: tree_sitter::Tree,
    highlighter: Highlighter,
    source: String,
    bracket_depths: Vec<Option<usize>>,
    cached: Vec<StyledLine>,
    textobject_scm: &'static str,
}

impl SyntaxEngine {
    pub fn new(text: &str, file_ext: &str) -> Result<Self, SyntaxError> {
        let language = language_for_ext(file_ext).ok_or(SyntaxError::UnsupportedLanguage)?;
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).map_err(|_| SyntaxError::QueryError("set_language".into()))?;
        let tree = parser.parse(text, None).ok_or(SyntaxError::QueryError("parse".into()))?;

        let (highlight_scm, textobject_scm) = query_files_for_lang(lang_key(file_ext));
        let mut highlighter = Highlighter::new(language.clone(), highlight_scm)
            .map_err(SyntaxError::QueryError)?;

        let bracket_depths = compute_bracket_depths(text);
        let cached = highlighter.highlight_lines(&tree, text, &bracket_depths);

        Ok(SyntaxEngine { language, tree, highlighter, source: text.to_string(), bracket_depths, cached, textobject_scm })
    }

    pub fn reparse(&mut self, text: &str) {
        let mut parser = tree_sitter::Parser::new();
        let _ = parser.set_language(&self.language);
        if let Some(tree) = parser.parse(text, None) {
            self.tree = tree;
            self.source = text.to_string();
            self.bracket_depths = compute_bracket_depths(text);
            self.cached = self.highlighter.highlight_lines(&self.tree, text, &self.bracket_depths);
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
        if self.textobject_scm.is_empty() { return None; }
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

        let query = tree_sitter::Query::new(&self.language, self.textobject_scm).ok()?;
        let mut cursor_q = tree_sitter::QueryCursor::new();
        let mut matches = cursor_q.matches(&query, self.tree.root_node(), self.source.as_bytes());

        while let Some(m) = matches.next() {
            for cap in m.captures {
                let name = query.capture_names()[cap.index as usize];
                if name == query_name {
                    let start = byte_to_char_pos(&self.source, cap.node.byte_range().start);
                    let end = byte_to_char_pos(&self.source, cap.node.byte_range().end);
                    if start <= cursor && cursor <= end {
                        return Some((start, end));
                    }
                }
            }
        }
        None
    }
}

fn byte_to_char_pos(source: &str, byte: usize) -> usize {
    source.char_indices().position(|(i, _)| i >= byte).unwrap_or(source.chars().count())
}

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
        _ => "",
    }
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
        "json" => (include_str!("../queries/json/highlights.scm"), ""),
        "lua" => (include_str!("../queries/lua/highlights.scm"), ""),
        "toml" => (include_str!("../queries/toml/highlights.scm"), ""),
        "yaml" => (include_str!("../queries/yaml/highlights.scm"), ""),
        "c" => (include_str!("../queries/c/highlights.scm"), ""),
        "scheme" => (include_str!("../queries/scheme/highlights.scm"), ""),
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
        ];
        for (ext, src) in cases {
            let engine = SyntaxEngine::new(src, ext)
                .unwrap_or_else(|e| panic!("{ext} query failed to build: {e:?}"));
            let has = engine.styled_lines().iter().any(|l| !l.highlights.is_empty());
            assert!(has, "{ext} produced no highlights");
        }
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
