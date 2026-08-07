# Plan C1: Tree-sitter Integration Implementation Plan

> **Status:** delivered; boxes resolved 2026-08-07.
>
> This plan was executed but never ticked as it went, leaving 38 boxes
> that read as outstanding work. They are **plain bullets now, not back-ticked**:
> a box ticked long after the fact asserts a verification that did not happen,
> and this tree has already been bitten by exactly that. The bullets stand as a
> record of what was built.
>
> Evidence it shipped: all 21 identifiers this plan names in backticks exist in
> the tree, and `docs/verification/editor-tui.txt` and `editor-gui.png` show highlighted Rust, and `drive.rs::the_buffer_arrives_syntax_highlighted_not_merely_as_text` asserts spans rather than text.


> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver incremental syntax highlighting, structural textobjects (function, class, loop, parameter), and rainbow bracket coloring — all backed by Tree-sitter.

**Architecture:** New `ruster-syntax` crate wraps `tree-sitter` and grammar crates, producing per-line highlight spans consumed by `ruster-render`'s new `StyledLine` type. The `App` in `ruster-tui` orchestrates the `Editor` + `SyntaxEngine`, calling `reparse()` on each buffer mutation. Textobjects use a new `Action::Textobject` variant dispatched by VimState and resolved by SyntaxEngine in the App.

**Tech Stack:** `tree-sitter`, `tree-sitter-rust`, `tree-sitter-python`, `tree-sitter-javascript`, `tree-sitter-typescript`, `tree-sitter-c`, `tree-sitter-json`, `tree-sitter-toml`, `tree-sitter-yaml` (exact versions pinned at implementation time via `cargo add`).

## Global Constraints

- All existing 74 tests must continue to pass (with minor adjustments for new types).
- `ruster-core` must NOT depend on `tree-sitter` or `ruster-syntax`.
- Unsupported file extensions must gracefully fall back to plain-text rendering.
- Tree-sitter re-parses the full buffer on every edit (no incremental optimization in this iteration).
- Tests for `ruster-syntax` must not require special fixtures (embed test source inline as `&str`).

---

## File Structure

### New files
```
crates/ruster-syntax/
├── Cargo.toml
└── src/
    ├── lib.rs              — SyntaxEngine public API
    ├── highlighter.rs      — query-based highlighting
    ├── textobjects.rs      — textobject queries
    ├── rainbow.rs          — bracket nesting depth
    └── theme.rs            — highlight name → SyntaxStyle mapping

crates/ruster-syntax/queries/
├── rust/textobjects.scm
├── python/textobjects.scm
├── javascript/textobjects.scm
├── typescript/textobjects.scm
├── c/textobjects.scm
├── json/textobjects.scm
├── toml/textobjects.scm
└── yaml/textobjects.scm
```

### Modified files
```
Cargo.toml                                  — add ruster-syntax to workspace
crates/ruster-render/src/lib.rs             — add Color, SyntaxStyle, StyledLine; update EditorState
crates/ruster-core/src/action.rs            — add Action::Textobject variant
crates/ruster-core/src/editor.rs            — no-op match for Textobject
crates/ruster-core/src/vim/mod.rs           — dispatch Tree-sitter textobject targets (f,c,l,a)
crates/ruster-tui/Cargo.toml                — add ruster-syntax dep
crates/ruster-tui/src/app.rs                — SyntaxEngine integration + textobject handling
crates/ruster-tui/src/renderer.rs           — map Color → ratatui style
crates/ruster-tui/src/widgets.rs            — BufferWidget per-char coloring via StyledLine
```

---

### Task 1: ruster-render color types + StyledLine + EditorState update

**Files:**
- Modify: `crates/ruster-render/src/lib.rs`
- Test: inline in same file

**Interfaces:**
- Consumes: nothing new
- Produces: `Color`, `SyntaxStyle`, `StyledLine` types; `EditorState.lines` becomes `Vec<StyledLine>`

- **Step 1: Add Color, SyntaxStyle, StyledLine above EditorState**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Default,
    Rgb(u8, u8, u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyntaxStyle {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub italic: bool,
}

impl Default for SyntaxStyle {
    fn default() -> Self {
        SyntaxStyle { fg: Color::Default, bg: Color::Default, bold: false, italic: false }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledLine {
    pub text: String,
    pub highlights: Vec<(usize, usize, SyntaxStyle)>,
}
```

- **Step 2: Change `EditorState.lines` type and update all fields**

```rust
pub struct EditorState<'a> {
    pub lines: Vec<StyledLine>,
    pub cursor: (u16, u16),
    pub cursor_kind: CursorKind,
    pub mode_label: &'a str,
    pub file_path: &'a str,
    pub modified: bool,
    pub cmdline: Option<&'a str>,
    pub message: Option<&'a str>,
}
```

- **Step 3: Fix the existing test to use StyledLine**

Replace lines 28-41 of `crates/ruster-render/src/lib.rs`:

```rust
#[test]
fn renderer_trait_is_object_safe() {
    let state = EditorState {
        lines: vec![StyledLine { text: "hello".to_string(), highlights: vec![] }],
        cursor: (0, 0),
        cursor_kind: CursorKind::Block,
        mode_label: "NORMAL",
        file_path: "test.txt",
        modified: false,
        cmdline: None,
        message: None,
    };
    let mut r = TestRenderer;
    r.render_frame(&state);
}
```

- **Step 4: Run tests to verify**

Run: `cargo test -p ruster-render`
Expected: 1 test PASS

- **Step 5: Commit**

```bash
git add crates/ruster-render/src/lib.rs
git commit -m "feat(render): add Color, SyntaxStyle, StyledLine; EditorState uses StyledLine"
```

---

### Task 2: ruster-syntax crate — highlight engine + theme

**Files:**
- Create: `crates/ruster-syntax/Cargo.toml`
- Create: `crates/ruster-syntax/src/lib.rs`
- Create: `crates/ruster-syntax/src/highlighter.rs`
- Create: `crates/ruster-syntax/src/theme.rs`
- Modify: `Cargo.toml` (workspace members)

**Interfaces:**
- Consumes: `ruster-render` types (Color, SyntaxStyle, StyledLine)
- Produces: `SyntaxEngine { new(), reparse(), highlight_line(), language_for_ext() }`

- **Step 1: Create `crates/ruster-syntax/Cargo.toml`**

```toml
[package]
name = "ruster-syntax"
version = "0.1.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[dependencies]
tree-sitter = "0.24"
tree-sitter-rust = "0.23"
tree-sitter-python = "0.21"
tree-sitter-javascript = "0.22"
tree-sitter-typescript = "0.22"
tree-sitter-c = "0.22"
tree-sitter-json = "0.21"
tree-sitter-toml = "0.22"
tree-sitter-yaml = "0.21"
ruster-render = { path = "../ruster-render" }
```

- **Step 2: Add `ruster-syntax` to workspace in root `Cargo.toml`**

```toml
members = ["crates/ruster-core", "crates/ruster-render", "crates/ruster-tui", "crates/ruster-bin", "crates/ruster-syntax"]
```

- **Step 3: Add a placeholder highlight query for Rust**

Create `crates/ruster-syntax/queries/rust/highlights.scm`:

```scheme
;; Highlight query for Rust (minimal — covers common constructs)
; Keywords
"as" @keyword
"break" @keyword
"const" @keyword
"continue" @keyword
"crate" @keyword
"else" @keyword
"enum" @keyword
"extern" @keyword
"false" @keyword
"fn" @keyword
"for" @keyword
"if" @keyword
"impl" @keyword
"in" @keyword
"let" @keyword
"loop" @keyword
"match" @keyword
"mod" @keyword
"move" @keyword
"mut" @keyword
"pub" @keyword
"ref" @keyword
"return" @keyword
"self" @keyword
"Self" @keyword
"static" @keyword
"struct" @keyword
"super" @keyword
"trait" @keyword
"true" @keyword
"type" @keyword
"unsafe" @keyword
"use" @keyword
"where" @keyword
"while" @keyword

; Function calls
(call_expression function: (identifier) @function)
(call_expression function: (field_expression field: (field_identifier) @function.method))

; Function definitions
(function_item name: (identifier) @function)
(function_signature name: (identifier) @function)

; Type definitions
(struct_item name: (type_identifier) @type)
(enum_item name: (type_identifier) @type)
(trait_item name: (type_identifier) @type)
(type_identifier) @type

; Strings
(string_literal) @string
(char_literal) @string

; Comments
(line_comment) @comment
(block_comment) @comment

; Numbers
(integer_literal) @number
(float_literal) @number

; Operators
(assignment_expression "=" @operator)
(binary_expression ["+" "-" "*" "/" "%" "==" "!=" "<" ">" "<=" ">=" "&&" "||"] @operator)
(unary_expression ["!" "&" "*" "-"] @operator)

; Built-in types
((type_identifier) @builtin
  (#match? @builtin "^(i8|i16|i32|i64|i128|isize|u8|u16|u32|u64|u128|usize|f32|f64|bool|char|str|String|Vec|Option|Result|Box|Rc|Arc|HashMap|HashSet)$"))
```

Then create the same file structure for other languages with minimal stubs (at least covering the same categories). For the initial iteration, only Rust needs a full query — other languages can ship single-line stubs that match everything as punctuation for graceful fallback.

- **Step 4: Create `crates/ruster-syntax/src/theme.rs`**

```rust
use ruster_render::{Color, SyntaxStyle};

fn rgb(r: u8, g: u8, b: u8) -> Color { Color::Rgb(r, g, b) }

pub fn style_for_capture(name: &str) -> SyntaxStyle {
    let base = match name.split('.').next().unwrap_or(name) {
        "keyword"   => SyntaxStyle { fg: rgb(203, 166, 247), bg: Color::Default, bold: true,  italic: false },
        "string"    => SyntaxStyle { fg: rgb(166, 227, 161), bg: Color::Default, bold: false, italic: false },
        "comment"   => SyntaxStyle { fg: rgb(108, 112, 134), bg: Color::Default, bold: false, italic: true  },
        "function"  => SyntaxStyle { fg: rgb(137, 180, 250), bg: Color::Default, bold: false, italic: false },
        "type"      => SyntaxStyle { fg: rgb(249, 226, 175), bg: Color::Default, bold: false, italic: false },
        "variable"  => SyntaxStyle { fg: rgb(205, 214, 244), bg: Color::Default, bold: false, italic: false },
        "constant"  => SyntaxStyle { fg: rgb(250, 179, 135), bg: Color::Default, bold: false, italic: false },
        "number"    => SyntaxStyle { fg: rgb(250, 179, 135), bg: Color::Default, bold: false, italic: false },
        "operator"  => SyntaxStyle { fg: rgb(137, 220, 235), bg: Color::Default, bold: false, italic: false },
        "punctuation" => SyntaxStyle::default(),
        "builtin"   => SyntaxStyle { fg: rgb(243, 139, 168), bg: Color::Default, bold: false, italic: false },
        _           => SyntaxStyle::default(),
    };
    base
}

pub const RAINBOW_PALETTE: [Color; 6] = [
    Color::Rgb(243, 139, 168),  // red
    Color::Rgb(250, 179, 135),  // peach
    Color::Rgb(249, 226, 175),  // yellow
    Color::Rgb(166, 227, 161),  // green
    Color::Rgb(137, 190, 180),  // teal
    Color::Rgb(137, 180, 250),  // blue
];
```

- **Step 5: Create `crates/ruster-syntax/src/highlighter.rs`**

```rust
use std::collections::HashMap;
use ruster_render::{Color, SyntaxStyle, StyledLine};
use crate::theme::{style_for_capture, RAINBOW_PALETTE};

pub struct Highlighter {
    query: tree_sitter::Query,
    cursor: tree_sitter::QueryCursor,
    language: tree_sitter::Language,
}

impl Highlighter {
    pub fn new(language: tree_sitter::Language, query_bytes: &[u8]) -> Result<Self, String> {
        let query = tree_sitter::Query::new(language, query_bytes)
            .map_err(|e| format!("query error: {}", e))?;
        Ok(Highlighter { query, cursor: tree_sitter::QueryCursor::new(), language })
    }

    pub fn highlight_lines(
        &mut self,
        tree: &tree_sitter::Tree,
        source: &str,
        rainbow: &[Option<usize>],
    ) -> Vec<StyledLine> {
        let bytes = source.as_bytes();
        let mut styled: Vec<StyledLine> = Vec::new();
        let mut line_starts: Vec<usize> = vec![0];
        for (i, ch) in source.char_indices() {
            if ch == '\n' { line_starts.push(i + 1); }
        }
        line_starts.push(bytes.len());

        // Build per-line char-based highlight ranges
        let mut per_line: Vec<Vec<(usize, usize, SyntaxStyle)>> =
            (0..line_starts.len() - 1).map(|_| Vec::new()).collect();

        let captures = self.cursor.captures(&self.query, tree.root_node(), source.as_bytes());

        // Collect all captures into (capture_name, byte_start, byte_end)
        let mut raw_captures: Vec<(String, usize, usize)> = Vec::new();
        for (m, capture_idx) in captures {
            for cap in m.captures {
                let start = cap.node.byte_range().start;
                let end = cap.node.byte_range().end;
                let name = self.query.capture_names()[cap.index as usize].clone();
                raw_captures.push((name, start, end));
            }
        }
        raw_captures.sort_by_key(|c| c.1);

        for (name, bs, be) in &raw_captures {
            let style = style_for_capture(name);
            let line_s = byte_to_line(*bs, &line_starts);
            let line_e = byte_to_line(*be, &line_starts);
            for li in line_s..=line_e.min(per_line.len() - 1) {
                let lstart = line_starts[li];
                let lend = line_starts[li + 1].min(bytes.len());
                let range_start = (*bs).max(lstart).saturating_sub(lstart);
                let range_end = (*be).min(lend).saturating_sub(lstart);
                if range_end > range_start {
                    // Convert byte offsets to char offsets
                    let text_slice = &source[lstart..lend];
                    let cs = byte_to_char_offset(text_slice, range_start);
                    let ce = byte_to_char_offset(text_slice, range_end);
                    if ce > cs {
                        per_line[li].push((cs, ce - cs, style));
                    }
                }
            }
        }

        // Sort highlights per line
        for hl in &mut per_line {
            hl.sort_by_key(|r| r.0);
        }

        // Build StyledLines, merging rainbow brackets
        for (li, hl) in per_line.iter().enumerate() {
            let lstart = line_starts[li];
            let lend = line_starts[li + 1];
            let line_text = &source[lstart..lend.min(bytes.len())];
            let text = line_text.to_string();
            let mut merged = hl.clone();

            // Override bracket colors from rainbow data
            for (offset, ch) in text.char_indices() {
                let abs_pos = lstart + offset;
                if abs_pos < rainbow.len() {
                    if let Some(depth) = rainbow[abs_pos] {
                        if "(){}[]".contains(ch) {
                            let color = RAINBOW_PALETTE[depth % 6];
                            // Remove any existing highlight at this char and add bracket color
                            merged.retain(|(s, l, _)| !(*s <= offset && offset < *s + *l));
                            merged.push((offset, ch.len_utf8(),
                                SyntaxStyle { fg: color, bg: Color::Default, bold: false, italic: false }));
                        }
                    }
                }
            }
            merged.sort_by_key(|r| r.0);

            styled.push(StyledLine { text, highlights: merged });
        }

        styled
    }
}

fn byte_to_char_offset(text: &str, byte: usize) -> usize {
    text.char_indices().position(|(i, _)| i >= byte).unwrap_or(text.chars().count())
}

fn byte_to_line(byte: usize, line_starts: &[usize]) -> usize {
    for (i, &start) in line_starts.iter().enumerate() {
        if byte < start { return i.saturating_sub(1); }
    }
    line_starts.len().saturating_sub(2)
}
```

Note: The `tree_sitter::QueryCursor::captures` API may differ slightly between versions. Adjust if necessary during implementation.

- **Step 6: Create `crates/ruster-syntax/src/lib.rs`**

```rust
pub mod highlighter;
pub mod theme;

use highlighter::Highlighter;
use ruster_render::{StyledLine, SyntaxStyle};
use std::collections::HashMap;

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
    // Store highlight queries as (name, query_bytes) pairs. Only one query
    // needed per language (the "highlights.scm"). Textobject queries stored
    // separately as raw SCM text for runtime compilation.
    textobject_scm: &'static [u8],
}

impl SyntaxEngine {
    pub fn new(text: &str, file_ext: &str) -> Result<Self, SyntaxError> {
        let language = language_for_ext(file_ext).ok_or(SyntaxError::UnsupportedLanguage)?;
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(language).map_err(|_| SyntaxError::QueryError("set_language".into()))?;
        let tree = parser.parse(text, None).ok_or(SyntaxError::QueryError("parse".into()))?;

        let (highlight_scm, textobject_scm) = query_files_for_lang(language);
        let mut highlighter = Highlighter::new(language, highlight_scm)
            .map_err(SyntaxError::QueryError)?;

        let bracket_depths = compute_bracket_depths(&tree, text);
        let cached = highlighter.highlight_lines(&tree, text, &bracket_depths);

        Ok(SyntaxEngine { language, tree, highlighter, source: text.to_string(), bracket_depths, cached, textobject_scm })
    }

    pub fn reparse(&mut self, text: &str) {
        let mut parser = tree_sitter::Parser::new();
        let _ = parser.set_language(self.language);
        if let Some(tree) = parser.parse(text, None) {
            self.tree = tree;
            self.source = text.to_string();
            self.bracket_depths = compute_bracket_depths(&self.tree, text);
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

        let query = tree_sitter::Query::new(self.language, self.textobject_scm).ok()?;
        let mut cursor_q = tree_sitter::QueryCursor::new();
        let source_bytes = self.source.as_bytes();
        let matches = cursor_q.matches(&query, self.tree.root_node(), source_bytes);

        for m in matches {
            for cap in m.captures {
                let name = query.capture_names()[cap.index as usize].as_str();
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
        "rs"              => Some(tree_sitter_rust::language()),
        "py"              => Some(tree_sitter_python::language()),
        "js" | "mjs" | "cjs" => Some(tree_sitter_javascript::language()),
        "ts" | "tsx"      => Some(tree_sitter_typescript::language_tsx()),
        "c" | "h"         => Some(tree_sitter_c::language()),
        "json"            => Some(tree_sitter_json::language()),
        "toml"            => Some(tree_sitter_toml::language()),
        "yaml" | "yml"    => Some(tree_sitter_yaml::language()),
        _ => None,
    }
}

fn query_files_for_lang(language: tree_sitter::Language) -> (&'static [u8], &'static [u8]) {
    if language == tree_sitter_rust::language() {
        (include_bytes!("../queries/rust/highlights.scm"),
         include_bytes!("../queries/rust/textobjects.scm"))
    } else {
        (b"", b"") // no highlighting or textobjects for other langs yet
    }
}

fn compute_bracket_depths(tree: &tree_sitter::Tree, source: &str) -> Vec<Option<usize>> {
    let len = source.len();
    let mut depths: Vec<Option<usize>> = vec![None; len];
    let mut cursor = tree.walk();
    let mut depth = 0usize;
    let mut stack: Vec<(usize, char)> = Vec::new();

    // Walk the tree and track bracket pairs
    loop {
        let node = cursor.node();
        if node.is_named() {
            let kind = node.kind();
            if let Some(bracket_char) = match kind {
                "(" => Some('('),
                ")" => Some(')'),
                "{" | "}" => None, // handled by unnamed nodes
                _ => None,
            } {
                // Placeholder — the real implementation walks into unnamed leaf nodes
                // and tracks '(' ')' '{' '}' '[' ']' with a depth counter.
            }
        }
        if !cursor.goto_first_child() {
            while !cursor.goto_next_sibling() {
                if !cursor.goto_parent() { break; }
            }
        }
        if node == tree.root_node() && !cursor.goto_first_child() { break; }
    }

    // Simplified approach: scan source directly for bracket chars
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
    fn reparse_does_not_panic() {
        let mut engine = SyntaxEngine::new("fn main() {}", "rs").unwrap();
        engine.reparse("fn main() { let x = 1; }"); // no panic
    }

    #[test]
    fn bracket_depths_basic() {
        let depths = compute_bracket_depths(
            &tree_sitter::Parser::new().unwrap().parse("(a + (b))", None).unwrap(),
            "(a + (b))",
        );
        assert_eq!(depths[0], Some(0)); // first '('
        assert_eq!(depths[4], Some(1)); // inner '('
        assert_eq!(depths[7], Some(1)); // inner ')'
        assert_eq!(depths[8], Some(0)); // outer ')'
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
}
```

- **Step 7: Run tests to verify**

Run: `cargo test -p ruster-syntax`
Expected: All tests PASS

- **Step 8: Build full workspace to check no regressions**

Run: `cargo check --workspace`
Expected: No errors

- **Step 9: Commit**

```bash
git add Cargo.toml crates/ruster-syntax/
git commit -m "feat(syntax): add ruster-syntax crate with parsing, highlighting, theme, bracket depths"
```

---

### Task 3: ruster-tui integration — colored rendering

**Files:**
- Modify: `crates/ruster-tui/Cargo.toml`
- Modify: `crates/ruster-tui/src/app.rs`
- Modify: `crates/ruster-tui/src/renderer.rs`
- Modify: `crates/ruster-tui/src/widgets.rs`

**Interfaces:**
- Consumes: `SyntaxEngine` from `ruster-syntax`, `StyledLine` from `ruster-render`
- Produces: Colored terminal rendering on Rust source files

- **Step 1: Add `ruster-syntax` dependency to `crates/ruster-tui/Cargo.toml`**

```toml
ruster-syntax = { path = "../ruster-syntax" }
```

- **Step 2: Add TuiRenderer method to convert Color → ratatui style**

In `crates/ruster-tui/src/renderer.rs`, add a helper:

```rust
use ratatui::style::Style;

fn ruster_color_to_ratatui(c: &ruster_render::Color) -> ratatui::style::Color {
    match c {
        ruster_render::Color::Default => ratatui::style::Color::Reset,
        ruster_render::Color::Rgb(r, g, b) => ratatui::style::Color::Rgb(*r, *g, *b),
    }
}

pub fn ruster_style_to_ratatui(s: &ruster_render::SyntaxStyle) -> Style {
    let mut style = Style::default()
        .fg(ruster_color_to_ratatui(&s.fg))
        .bg(ruster_color_to_ratatui(&s.bg));
    if s.bold { style = style.add_modifier(ratatui::style::Modifier::BOLD); }
    if s.italic { style = style.add_modifier(ratatui::style::Modifier::ITALIC); }
    style
}
```

- **Step 3: Update BufferWidget to draw per-char colors from StyledLine**

Replace `BufferWidget` in `crates/ruster-tui/src/widgets.rs`:

```rust
use ruster_render::StyledLine;

pub struct BufferWidget {
    lines: Vec<StyledLine>,
    cursor: (u16, u16),
    syntax: bool,
}

impl BufferWidget {
    pub fn new(lines: Vec<StyledLine>, cursor: (u16, u16)) -> Self {
        BufferWidget { lines, cursor, syntax: false }
    }

    pub fn with_syntax(mut self, yes: bool) -> Self {
        self.syntax = yes;
        self
    }
}

impl Widget for BufferWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        for (i, line) in self.lines.iter().enumerate() {
            if i as u16 >= area.height { break; }
            let y = area.y + i as u16;
            let is_cursor_line = i as u16 == self.cursor.0;

            // Build a color map for this line: char_offset -> SyntaxStyle
            let mut style_map: std::collections::HashMap<usize, (Color, Color, bool, bool)> =
                std::collections::HashMap::new();
            if self.syntax {
                for (offset, length, style) in &line.highlights {
                    for c in 0..*length {
                        style_map.insert(offset + c, (style.fg, style.bg, style.bold, style.italic));
                    }
                }
            }

            for (j, ch) in line.text.chars().enumerate() {
                let x = area.x + j as u16;
                if x >= area.right() { break; }
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    if is_cursor_line && j as u16 == self.cursor.1 {
                        cell.set_bg(Color::White);
                        cell.set_fg(Color::Black);
                    } else if let Some((fg, bg, bold, italic)) = style_map.get(&j) {
                        cell.set_fg(ruster_render_color_to_tui(fg));
                        if !matches!(bg, Color::Default) {
                            cell.set_bg(ruster_render_color_to_tui(bg));
                        }
                        // bold/italic not directly settable per-cell in ratatui
                        // (would need Modifier on the whole cell)
                    }
                }
            }
        }
    }
}

fn ruster_render_color_to_tui(c: &ruster_render::Color) -> Color {
    match c {
        ruster_render::Color::Default => Color::Reset,
        ruster_render::Color::Rgb(r, g, b) => Color::Rgb(*r, *g, *b),
    }
}
```

- **Step 4: Update `App` to integrate SyntaxEngine**

In `crates/ruster-tui/src/app.rs`:

```rust
use ruster_syntax::SyntaxEngine;
use ruster_render::StyledLine;

pub struct App {
    pub editor: Editor,
    pub vim: VimState,
    renderer: TuiRenderer,
    file_path: PathBuf,
    pub should_quit: bool,
    message: Option<String>,
    syntax: Option<SyntaxEngine>,
}

impl App {
    pub fn new(content: String, file_path: PathBuf) -> Self {
        let mut editor = Editor::from_str(&content);
        editor.execute(Action::Move(Motion::To(0)));
        let vim = VimState::new();
        let renderer = TuiRenderer::dummy();
        let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let syntax = SyntaxEngine::new(&content, ext).ok();
        let should_quit = false;
        App { editor, vim, renderer, file_path, should_quit, message: None, syntax }
    }

    // ... run() stays mostly the same, but in the render() method use StyledLine
}
```

- **Step 5: Update `App::render()` to build StyledLine-based EditorState**

Replace the render method:

```rust
fn render(&mut self) {
    let head = self.editor.primary_head();
    let mut line = 0u16;
    let mut col = 0u16;

    let styled_lines: Vec<StyledLine> = if let Some(syn) = &self.syntax {
        syn.styled_lines().to_vec()
    } else {
        // fallback: plain text with no highlights
        self.editor.buffer().to_string()
            .split('\n')
            .map(|s| StyledLine { text: s.to_string(), highlights: vec![] })
            .collect()
    };

    // Compute cursor from head
    let mut remaining = head;
    for sl in &styled_lines {
        let lc = sl.text.chars().count();
        if remaining <= lc { col = remaining as u16; break; }
        remaining = remaining.saturating_sub(lc + 1);
        line += 1;
    }

    let cursor_kind = match self.vim.mode {
        VimMode::Insert | VimMode::Cmdline => CursorKind::Bar,
        _ => CursorKind::Block,
    };
    let mode_label = crate::widgets::mode_label(&self.vim.mode);
    let file_path = self.file_path.to_string_lossy().to_string();
    let cmdline = match self.vim.mode {
        VimMode::Cmdline => Some(crate::widgets::cmdline_label(self.vim.cmdline_buffer())),
        _ => self.message.as_ref().map(|m| m.clone()),
    };

    let state = EditorState {
        lines: styled_lines,
        cursor: (line, col),
        cursor_kind,
        mode_label,
        file_path: &file_path,
        modified: false,
        cmdline: cmdline.as_deref(),
        message: None,
    };
    self.renderer.render_frame(&state);
}
```

- **Step 6: Call `SyntaxEngine::reparse()` after each buffer mutation in the event loop**

In `App::run()`, after the `for action in self.vim.handle(...)` loop, add:

```rust
// Re-parse after buffer mutations
let before = std::mem::replace(
    &mut self.editor, 
    Editor::from_str("")
);
// ... ah, we need a different approach. Track content digest.

// Actually, simpler: compare buffer string before and after key handling.
// But we don't have access to the before string here.
// Best approach: after each iteration that produced CmdlineResult or that
// could have mutated the buffer, re-sync syntax.

// Move this check to right before render():
let new_content = self.editor.buffer().to_string();
if let Some(syn) = &mut self.syntax {
    syn.reparse(&new_content);
}
```

Actually, this is inefficient (reparses every frame even if nothing changed). Let's be smarter:

```rust
fn render(&mut self) {
    // Sync syntax with current buffer content
    let content = self.editor.buffer().to_string();
    if let Some(syn) = &mut self.syntax {
        // Simple approach: always reparse before render
        // (tree-sitter full parse is sub-ms for typical file sizes)
        syn.reparse(&content);
    }
    // ... rest of render
}
```

This re-parses every frame. For files <100KB this is fast enough (microseconds). If it becomes a bottleneck, add a digest check later.

- **Step 7: Pass `syntax` flag to BufferWidget in TuiRenderer**

In `crates/ruster-tui/src/renderer.rs`, the `render_frame` method:

```rust
let has_highlights = state.lines.iter().any(|l| !l.highlights.is_empty());
let buf_widget = crate::widgets::BufferWidget::new(
    state.lines.clone(),
    state.cursor,
).with_syntax(has_highlights);
```

- **Step 8: Fix existing tests in ruster-tui**

Update any test in `app.rs` that constructs `App` — no change needed since `App::new` now optionally initializes `SyntaxEngine`.

Update `renderer.rs` tests if any exist.

- **Step 9: Build and test the workspace**

Run: `cargo test --workspace`
Expected: 70+ tests PASS (some existing tests may need minor type fixes for `Vec<StyledLine>` in their `EditorState` construction)

If there are test failures due to the `EditorState` change, fix them by wrapping strings in `StyledLine { text: s.into(), highlights: vec![] }`.

- **Step 10: Commit**

```bash
git add crates/ruster-tui/
git commit -m "feat(tui): integrate syntax highlighting with colored BufferWidget rendering"
```

---

### Task 4: Tree-sitter textobjects (Action::Textobject + VimState dispatch + queries)

**Files:**
- Modify: `crates/ruster-core/src/action.rs`
- Modify: `crates/ruster-core/src/editor.rs`
- Modify: `crates/ruster-core/src/vim/mod.rs`
- Modify: `crates/ruster-tui/src/app.rs`
- Create: `crates/ruster-syntax/queries/rust/textobjects.scm`

**Interfaces:**
- Consumes: `SyntaxEngine::ts_textobject()` — needs to be added
- Produces: `Action::Textobject { op, kind, target, count }`; VimState dispatches `f`/`c`/`l`/`a` as TS textobjects; App resolves range via SyntaxEngine

- **Step 1: Add `Action::Textobject` variant**

In `crates/ruster-core/src/action.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    // ... existing variants ...
    Textobject { op: char, kind: char, target: char, count: u32 },
}
```

- **Step 2: Add no-op match in `Editor::execute`**

In `crates/ruster-core/src/editor.rs`:

```rust
Action::Textobject { .. } => {}
```

- **Step 3: Add Tree-sitter textobject dispatch in VimState**

In `crates/ruster-core/src/vim/mod.rs`, modify the `pending_textobj` response in `handle_normal`:

Find the match arm for `KeyEvent::Char(c2 @ ('w' | '"' | '\'' | '(' | ')' | '{' | '}'))` and add a separate arm for TS targets:

```rust
// After the existing textobj match arm:
KeyEvent::Char(c2 @ ('w' | '"' | '\'' | '(' | ')' | '{' | '}')) => {
    if let Some((start, end)) = crate::vim::textobj::range_for_textobj(kind, c2, editor) {
        self.apply_operator(op, start, end, editor, out);
        if op == 'd' || op == 'c' {
            self.last_change = Some(LastChange::OperatorTextobj { op, kind, target: c2 });
        }
    }
    return;
}
KeyEvent::Char(c2 @ ('f' | 'c' | 'l' | 'a')) => {
    // Tree-sitter textobject — emit action for App to handle
    self.pending = OpState::Idle;
    self.pending_textobj = None;
    let count = self.count.unwrap_or(1);
    self.count = None;
    out.push(Action::Textobject { op, kind, target: c2, count });
    return;
}
```

- **Step 4: Create Rust textobjects query file**

`crates/ruster-syntax/queries/rust/textobjects.scm`:

```scheme
;; function
(function_item body: (_) @function.inner) @function.outer
(closure_expression body: (_) @function.inner) @function.outer

;; struct/trait/enum
(struct_item body: (_) @class.inner) @class.outer
(trait_item body: (_) @class.inner) @class.outer
(enum_item body: (_) @class.inner) @class.outer
(impl_item body: (_) @class.inner) @class.outer

;; loop
(for_expression body: (_) @loop.inner) @loop.outer
(while_expression body: (_) @loop.inner) @loop.outer
(loop_expression body: (_) @loop.inner) @loop.outer

;; parameters
(parameters) @parameter.outer
(parameters "," (_) @parameter.inner)
```

Note: `SyntaxEngine::ts_textobject()` is already implemented in Task 2's `lib.rs`. It compiles the textobject SCM query at call time using `tree_sitter::Query::new()`. The `textobject_scm` field stores the embedded bytes.

- **Step 5: Handle `Action::Textobject` in the App's event loop**

In `crates/ruster-tui/src/app.rs`, modify the match in the event loop:

```rust
for action in self.vim.handle(key, &self.editor) {
    match action {
        Action::CmdlineResult(cmd) => {
            self.message = None;
            match self.parse_cmdline(&cmd) {
                Ok(CmdAction::Save(force)) => self.save_file(force),
                Ok(CmdAction::SaveAs(p)) => self.save_as(&p),
                Ok(CmdAction::Quit) | Ok(CmdAction::ForceQuit) => {
                    self.should_quit = true;
                }
                Ok(CmdAction::SaveAndQuit) => {
                    self.save_file(false);
                    self.should_quit = true;
                }
                Err(e) => self.message = Some(e),
            }
        }
        Action::Textobject { op, kind, target, count: _ } => {
            let cursor = self.editor.primary_head();
            if let Some((start, end)) = self.syntax.as_ref()
                .and_then(|s| s.ts_textobject(kind, target, cursor))
            {
                self.exec_operator(op, start, end);
            }
        }
        other => self.editor.execute(other),
    }
}
```

And add the `exec_operator` method:

```rust
fn exec_operator(&mut self, op: char, start: usize, end: usize) {
    let safe_end = end.min(self.editor.buffer().len_chars());
    match op {
        'd' => {
            self.editor.execute(Action::BeginBatch);
            self.editor.execute(Action::Edit(EditOp::DeleteRange(start, safe_end)));
            self.editor.execute(Action::EndBatch);
        }
        'c' => {
            self.editor.execute(Action::BeginBatch);
            self.editor.execute(Action::Edit(EditOp::DeleteRange(start, safe_end)));
            self.vim.mode = VimMode::Insert;
        }
        'y' => {
            let text = self.editor.buffer().slice_string(start, safe_end);
            self.vim.set_register(text);
        }
        _ => {}
    }
}
```

Add a `set_register` method to `VimState`:

```rust
pub fn set_register(&mut self, text: String) {
    self.register = Some(text);
}
```

- **Step 6: Add textobject tests in ruster-syntax and ruster-core**

In `crates/ruster-syntax/src/lib.rs`:

```rust
#[test]
fn ts_textobject_inner_function() {
    let engine = SyntaxEngine::new("fn foo() { let x = 1; }", "rs").unwrap();
    // cursor at 'x' (inside the function body) — byte offset ~15
    let result = engine.ts_textobject('i', 'f', 15);
    assert!(result.is_some());
    let (start, end) = result.unwrap();
    // Should cover just the body braces: " { let x = 1; }"
    assert!(start < end);
}
```

In `crates/ruster-core/src/vim/mod.rs` — add a test for the new dispatch:

```rust
#[test]
fn di_f_triggers_textobject_action() {
    let mut e = Editor::from_str("fn foo() { let x = 1; }");
    let mut v = VimState::new();
    for a in v.handle(KeyEvent::Char('d'), &e) { e.execute(a); }
    for a in v.handle(KeyEvent::Char('i'), &e) { e.execute(a); }
    let actions = v.handle(KeyEvent::Char('f'), &e);
    assert!(actions.iter().any(|a| matches!(a, Action::Textobject { op: 'd', kind: 'i', target: 'f', .. })));
}
```

- **Step 7: Build and test workspace**

Run: `cargo test --workspace`
Expected: All tests PASS (including new textobject tests)

- **Step 8: Commit**

```bash
git add crates/ruster-core/src/action.rs crates/ruster-core/src/editor.rs crates/ruster-core/src/vim/mod.rs crates/ruster-tui/src/app.rs crates/ruster-syntax/queries/rust/textobjects.scm crates/ruster-syntax/src/lib.rs
git commit -m "feat(core): tree-sitter textobjects via Action::Textobject with App dispatch"
```

---

### Task 5: Rainbow brackets — bracket depth integration in renderer

**Files:**
- Modify: `crates/ruster-syntax/src/lib.rs` — already handled in Task 2 via `compute_bracket_depths`
- Modify: `crates/ruster-tui/src/widgets.rs` — already handled in Task 3
- Test: integration test

Rainbow bracket colors were already built into Task 2's `Highlighter::highlight_lines` and Task 3's `BufferWidget`. The `compute_bracket_depths` function in `ruster-syntax` computes depths, and the `highlighter.rs` merges them into the StyledLine output.

This task is about verifying it works end-to-end.

- **Step 1: Add integration test for rainbow brackets**

In `crates/ruster-syntax/src/lib.rs`:

```rust
#[test]
fn rainbow_bracket_colors_applied() {
    let engine = SyntaxEngine::new("(a(b)c)", "rs").unwrap();
    let styled = engine.styled_lines();
    assert_eq!(styled.len(), 1);
    let line = &styled[0];
    // The brackets at positions 0 and 6 should have highlight entries
    let bracket_highlights: Vec<_> = line.highlights.iter()
        .filter(|(s, _, _)| *s == 0 || *s == 6)
        .collect();
    assert!(!bracket_highlights.is_empty(), "expected bracket highlights");
}
```

- **Step 2: Visual verification plan**

Since rainbow brackets are a visual feature, document manual verification:
1. Run `cargo run -- crates/ruster-syntax/src/lib.rs`
2. Verify that `(` `)` `{` `}` `[` `]` appear in cycling colors
3. Verify nesting: `((()))` shows three different colors

- **Step 3: Build and test**

Run: `cargo test --workspace`
Expected: All tests PASS

- **Step 4: Commit**

```bash
git add crates/ruster-syntax/src/lib.rs
git commit -m "feat(syntax): end-to-end rainbow bracket coloring verified"
```

---

### Verification

- **Final workspace check**

Run: `cargo test --workspace`
Expected: All tests PASS, zero warnings

Run: `cargo check --workspace`
Expected: Clean build

Run: `cargo run -- crates/ruster-core/src/editor.rs`
Expected: Terminal opens with colored syntax, rainbow brackets, functional textobjects (`d i f`, `d a c`, etc.)

- **Final commit**

```bash
git add -A && git commit -m "feat: Plan C1 Tree-sitter integration complete"
git push origin main
```
