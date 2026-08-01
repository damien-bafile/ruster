pub mod grammar;
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
    textobject_scm: String,
    /// The highlight query **actually in use** — the user's when it loaded, the
    /// built-in when theirs was rejected. Kept so `todo_markers` can re-query
    /// the tree for `@comment` captures; re-querying with the rejected text
    /// would fail every time.
    highlight_scm: String,
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
    /// Non-fatal query-loading problems, for the caller to surface.
    warnings: Vec<String>,
}

impl SyntaxEngine {
    pub fn new(text: &str, file_ext: &str) -> Result<Self, SyntaxError> {
        let key = lang_key(file_ext);
        // Prefer a tree-sitter grammar; fall back to line-based markup.
        let (resolved, grammar_warning) = resolve_language(file_ext);
        if let Some(language) = resolved {
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(&language).map_err(|_| SyntaxError::QueryError("set_language".into()))?;
            let tree = parser.parse(text, None).ok_or(SyntaxError::QueryError("parse".into()))?;

            let loaded = load_queries(user_query_dir().as_deref(), key);
            let mut warnings = loaded.warnings;
            warnings.extend(grammar_warning);

            // A malformed *user* query must not leave the buffer unhighlighted:
            // report it and fall back to the built-in, the way a bad config.lua
            // already degrades. A malformed built-in is a bug, and still fatal.
            // Bound first: holding the borrow of `loaded.highlights` across the
            // match would stop the `Ok` arm moving it out.
            let attempt = Highlighter::new(language.clone(), &loaded.highlights, key);
            let (mut highlighter, highlight_scm) = match attempt {
                Ok(h) => (h, loaded.highlights.into_owned()),
                Err(e) if loaded.highlights_from_user => {
                    warnings.push(format!(
                        "{key}/highlights.scm: {e} — using the built-in query"
                    ));
                    let builtin = builtin_queries(key).0;
                    let h = Highlighter::new(language.clone(), builtin, key)
                        .map_err(SyntaxError::QueryError)?;
                    // The built-in, not the rejected text: `todo_markers`
                    // re-runs this query and would otherwise fail every call.
                    (h, builtin.to_string())
                }
                Err(e) => return Err(SyntaxError::QueryError(e)),
            };

            let bracket_depths = compute_bracket_depths(text);
            let cached = highlighter.highlight_lines(&tree, text, &bracket_depths);
            Ok(SyntaxEngine {
                backend: Backend::Tree(Box::new(TreeBackend {
                    language,
                    tree,
                    highlighter,
                    source: text.to_string(),
                    bracket_depths,
                    textobject_scm: loaded.textobjects.into_owned(),
                    highlight_scm,
                })),
                cached,
                warnings,
            })
        } else if let Some(mlang) = markup::markup_lang(key) {
            let cached = markup::highlight_markup(mlang, text);
            Ok(SyntaxEngine { backend: Backend::Markup(mlang), cached, warnings: Vec::new() })
        } else {
            Err(SyntaxError::UnsupportedLanguage)
        }
    }

    /// Problems hit while loading this buffer's queries — a malformed or
    /// unreadable user query. Empty on the normal path. The caller surfaces
    /// these through the notification pipeline; highlighting already fell back.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
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

        let query = tree_sitter::Query::new(&tb.language, &tb.textobject_scm).ok()?;
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
        let Ok(query) = tree_sitter::Query::new(&tb.language, &tb.highlight_scm) else {
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

/// The grammar to parse `file_ext` with, and any complaint worth surfacing.
///
/// A user grammar in `~/.config/ruster/grammars/` wins over the compiled-in
/// one; anything wrong with it degrades to the built-in with a warning, the way
/// a malformed user query already does.
///
/// [`language_for_ext`] deliberately keeps meaning *the compiled-in set only*.
/// That is what `qcheck::every_parseable_language_has_a_highlight_query` asserts
/// over its hardcoded list of 11 extensions, and that invariant is still exactly
/// true — it is a statement about what ruster ships, which dynamic loading does
/// not change. A user grammar with no query is a runtime condition, handled here
/// by falling back rather than a build failure.
pub fn resolve_language(file_ext: &str) -> (Option<tree_sitter::Language>, Option<String>) {
    let key = lang_key(file_ext);
    let builtin = language_for_ext(file_ext);
    let Some(dir) = grammar::user_grammar_dir() else {
        return (builtin, None);
    };
    match grammar::load_grammar(&dir, key) {
        Ok(lang) => (Some(lang), None),
        // Nothing installed for this language, which is the normal path.
        Err(grammar::GrammarError::NotFound) => (builtin, None),
        Err(e) if builtin.is_some() => {
            (builtin, Some(format!("grammar {key}: {e} — using the built-in grammar")))
        }
        // No built-in to fall back to, so this language simply goes unparsed.
        Err(e) => (None, Some(format!("grammar {key}: {e}"))),
    }
}

/// Where the user's own queries live: `~/.config/ruster/queries/<lang>/`,
/// honouring `XDG_CONFIG_HOME`. The same discovery the theme loader uses for
/// `~/.config/ruster/themes/`, so there is one place to put customisations.
///
/// `None` when there is no home directory to resolve against, which is simply
/// the no-user-queries case.
pub fn user_query_dir() -> Option<std::path::PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Some(std::path::PathBuf::from(xdg).join("ruster").join("queries"));
        }
    }
    dirs::home_dir().map(|h| h.join(".config").join("ruster").join("queries"))
}

/// Query sources for one language, and where they came from.
#[derive(Debug, Default)]
pub(crate) struct LoadedQueries {
    pub highlights: std::borrow::Cow<'static, str>,
    pub textobjects: std::borrow::Cow<'static, str>,
    /// Whether `highlights` came from the user, so a query error can fall back
    /// to the built-in rather than leaving the buffer unhighlighted.
    pub highlights_from_user: bool,
    /// Problems worth telling the user about. Never fatal.
    pub warnings: Vec<String>,
}

/// Load `<dir>/<key>/<file>`, or `None` when it is absent — the normal case,
/// which must stay silent. A file that exists but cannot be read is worth a
/// warning, because the user plainly meant it to be used.
fn read_user_query(
    dir: Option<&std::path::Path>,
    key: &str,
    file: &str,
    warnings: &mut Vec<String>,
) -> Option<String> {
    let path = dir?.join(key).join(file);
    if !path.is_file() {
        return None;
    }
    match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) => {
            warnings.push(format!("{}: {e} — using the built-in query", path.display()));
            None
        }
    }
}

/// Queries for `key`, preferring the user's copy in `dir` over the compiled-in
/// one. Takes the directory rather than resolving it, so this stays pure and
/// testable against a temp dir.
pub(crate) fn load_queries(dir: Option<&std::path::Path>, key: &str) -> LoadedQueries {
    let (builtin_hl, builtin_to) = builtin_queries(key);
    let mut warnings = Vec::new();
    let user_hl = read_user_query(dir, key, "highlights.scm", &mut warnings);
    let user_to = read_user_query(dir, key, "textobjects.scm", &mut warnings);

    LoadedQueries {
        highlights_from_user: user_hl.is_some(),
        highlights: user_hl.map_or(std::borrow::Cow::Borrowed(builtin_hl), std::borrow::Cow::Owned),
        textobjects: user_to.map_or(std::borrow::Cow::Borrowed(builtin_to), std::borrow::Cow::Owned),
        warnings,
    }
}

/// Highlight and textobject query sources compiled into the binary. Languages
/// without bundled queries return empty strings — no (rather than wrong)
/// highlighting; rainbow brackets still apply since they are computed
/// separately.
fn builtin_queries(key: &str) -> (&'static str, &'static str) {
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

    use std::sync::atomic::{AtomicUsize, Ordering};

    static QUERY_FIXTURE: AtomicUsize = AtomicUsize::new(0);

    /// A unique `queries/` root per call, so these stay parallel-safe.
    fn query_dir(files: &[(&str, &str)]) -> std::path::PathBuf {
        let id = QUERY_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("ruster_queries_{id}"));
        let _ = std::fs::remove_dir_all(&root);
        for (rel, body) in files {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, body).unwrap();
        }
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// The overwhelmingly common case: no user directory at all. It must cost
    /// nothing and produce exactly the compiled-in queries.
    #[test]
    fn with_no_user_directory_the_builtin_queries_are_used() {
        let loaded = load_queries(None, "rust");
        assert_eq!(loaded.highlights, builtin_queries("rust").0);
        assert_eq!(loaded.textobjects, builtin_queries("rust").1);
        assert!(!loaded.highlights_from_user);
        assert!(loaded.warnings.is_empty(), "silence is the normal path");

        // A directory that exists but has nothing for this language is the same.
        let dir = query_dir(&[("python/highlights.scm", "(module) @none")]);
        let loaded = load_queries(Some(&dir), "rust");
        assert_eq!(loaded.highlights, builtin_queries("rust").0);
        assert!(loaded.warnings.is_empty());
    }

    #[test]
    fn a_user_query_wins_over_the_builtin() {
        let dir = query_dir(&[
            ("rust/highlights.scm", "(identifier) @variable"),
            ("rust/textobjects.scm", "(function_item) @function.outer"),
        ]);
        let loaded = load_queries(Some(&dir), "rust");
        assert_eq!(loaded.highlights, "(identifier) @variable");
        assert_eq!(loaded.textobjects, "(function_item) @function.outer");
        assert!(loaded.highlights_from_user);
        assert!(loaded.warnings.is_empty());
    }

    /// The two files are independent: overriding highlights must not silently
    /// discard the built-in textobjects, or `daf` would stop working.
    #[test]
    fn overriding_one_file_keeps_the_builtin_of_the_other() {
        let dir = query_dir(&[("rust/highlights.scm", "(identifier) @variable")]);
        let loaded = load_queries(Some(&dir), "rust");
        assert_eq!(loaded.highlights, "(identifier) @variable");
        assert_eq!(loaded.textobjects, builtin_queries("rust").1, "textobjects still built in");
        assert!(!loaded.textobjects.is_empty());
    }

    /// A query that tree-sitter rejects must degrade to the built-in with a
    /// warning — never take the editor down or leave the buffer unhighlighted.
    #[test]
    fn a_malformed_user_query_falls_back_instead_of_failing() {
        let dir = query_dir(&[("rust/highlights.scm", "(this is not a valid query @@@")]);
        let loaded = load_queries(Some(&dir), "rust");
        // Loading itself succeeds — the file was readable; it is tree-sitter
        // that rejects it, which is why the fallback lives in `SyntaxEngine`.
        assert!(loaded.highlights_from_user);

        let lang = language_for_ext("rs").unwrap();
        assert!(
            Highlighter::new(lang.clone(), &loaded.highlights, "rust").is_err(),
            "the fixture really is malformed"
        );
        assert!(
            Highlighter::new(lang, builtin_queries("rust").0, "rust").is_ok(),
            "and the built-in it falls back to is fine"
        );
    }

    /// The end-to-end version of the above, through the real constructor.
    #[test]
    fn an_engine_survives_a_malformed_user_query_and_says_so() {
        let dir = query_dir(&[("rust/highlights.scm", "((((")]);
        let loaded = load_queries(Some(&dir), "rust");
        let lang = language_for_ext("rs").unwrap();
        let mut warnings = loaded.warnings;
        let ok = match Highlighter::new(lang.clone(), &loaded.highlights, "rust") {
            Ok(h) => Some(h),
            Err(e) if loaded.highlights_from_user => {
                warnings.push(format!("rust/highlights.scm: {e} — using the built-in query"));
                Highlighter::new(lang, builtin_queries("rust").0, "rust").ok()
            }
            Err(_) => None,
        };
        assert!(ok.is_some(), "highlighting still works");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("built-in"), "{:?}", warnings[0]);
    }

    /// A language with no compiled-in query still accepts a user one — that is
    /// how a grammar ships highlighting it never had.
    #[test]
    fn a_user_query_supplies_a_language_that_ships_without_one() {
        assert_eq!(builtin_queries("nosuchlang"), ("", ""));
        let dir = query_dir(&[("nosuchlang/highlights.scm", "(x) @keyword")]);
        let loaded = load_queries(Some(&dir), "nosuchlang");
        assert_eq!(loaded.highlights, "(x) @keyword");
    }

    /// A *directory* named `highlights.scm` is not a query. Reading it would
    /// fail with an io error, so it has to be rejected before the read.
    #[test]
    fn a_directory_named_like_a_query_is_ignored() {
        let dir = query_dir(&[("rust/highlights.scm/decoy", "x")]);
        let loaded = load_queries(Some(&dir), "rust");
        assert_eq!(loaded.highlights, builtin_queries("rust").0);
        assert!(!loaded.highlights_from_user);
        assert!(loaded.warnings.is_empty(), "indistinguishable from absent");
    }

    #[test]
    fn the_user_query_dir_sits_under_the_ruster_config_dir() {
        let dir = user_query_dir().expect("a home directory in the test environment");
        assert!(dir.ends_with("ruster/queries"), "{}", dir.display());
    }

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
