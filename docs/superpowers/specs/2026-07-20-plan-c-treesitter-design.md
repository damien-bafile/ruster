# Ruster Tree-sitter Integration Design (Plan C1)

> **Goal:** Deliver incremental syntax highlighting, structural textobjects (function, class, loop, parameter), and rainbow bracket coloring — all backed by Tree-sitter — as an integrated feature of the `ruster` terminal editor.
>
> **Depends on:** `ruster-core` (Plan A) + `ruster-tui`/`ruster-render`/`ruster-bin` (Plan B).
>
> **Exclusions:** LSP client, code formatting, snippets, symbol search, call hierarchy, code outline, hover/type preview. These are all later plans (C2+).

---

## Architecture

### New crate: `ruster-syntax`

A new workspace crate that owns the Tree-sitter parser tree and produces highlight spans, textobject ranges, and rainbow bracket colors.

```
crates/ruster-syntax/
├── Cargo.toml
└── src/
    ├── lib.rs              — SyntaxEngine public API
    ├── highlighter.rs      — wraps tree-sitter highlight query, produces per-line ranges
    ├── textobjects.rs      — language-specific .scm queries → textobject ranges
    ├── rainbow.rs          — bracket nesting-depth → color assignment
    └── theme.rs            — highlight name → Color mapping (Catppuccin mocha)
```

**Dependencies** (exact version pinned at implementation time — check crates.io):
```toml
[dependencies]
tree-sitter = "0.24"
tree-sitter-rust = "0.22"
tree-sitter-python = "0.21"
tree-sitter-javascript = "0.21"
tree-sitter-typescript = "0.21"
tree-sitter-c = "0.21"
tree-sitter-json = "0.21"
tree-sitter-toml = "0.21"
tree-sitter-yaml = "0.21"
ruster-render = { path = "../ruster-render" }
```

### Data flow

```
User types → VimState → Action → Editor::execute → buffer mutates
                                                          ↓
App loop detects change → SyntaxEngine::edit() → incremental re-parse + re-highlight
                                                          ↓
App builds EditorState → SyntaxEngine::highlight_line(i) → StyledLine
                                                          ↓
TuiRenderer::render_frame draws each StyledLine with colors (fg, bg, bold, italic)
```

SyntaxEngine does NOT live inside `ruster-core`. It lives in `ruster-tui`'s `App` (which orchestrates `Editor` + `VimState` + `SyntaxEngine`). This keeps `ruster-core` free of the C build dependency on `tree-sitter`.

---

## Types in `ruster-render`

### Color

```rust
pub enum Color {
    Default,
    Rgb(u8, u8, u8),
}
```

### SyntaxStyle

```rust
pub struct SyntaxStyle {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub italic: bool,
}
```

### StyledLine (replaces plain Vec<String> for lines)

```rust
pub struct StyledLine {
    pub text: String,
    /// (char_offset, length, style) — sorted, non-overlapping, ascending
    pub highlights: Vec<(usize, usize, SyntaxStyle)>,
}
```

### EditorState change

```rust
pub struct EditorState<'a> {
    pub lines: Vec<StyledLine>,       // was Vec<String>
    pub cursor: (u16, u16),
    pub cursor_kind: CursorKind,
    pub mode_label: &'a str,
    pub file_path: &'a str,
    pub modified: bool,
    pub cmdline: Option<&'a str>,
    pub message: Option<&'a str>,
}
```

Empty-file and error cases: `StyledLine { text: "", highlights: vec![] }`.

---

## SyntaxEngine

### Lifecycle

```rust
impl SyntaxEngine {
    /// Create from initial buffer text. Detects language from file extension.
    pub fn new(text: &str, file_ext: &str) -> Result<Self, SyntaxError>;

    /// Re-parse from scratch with new buffer content. Used after every
    /// buffer mutation in the first iteration.
    pub fn reparse(&mut self, text: &str);

    /// Future: incremental re-parse after an edit (reserved for perf optimization).
    pub fn edit(
        &mut self,
        old_start_char: usize,
        old_end_char: usize,
        new_start_char: usize,
        new_end_char: usize,
        new_text: &str,
    );

    /// Get highlights for a specific line. Returns cached results.
    pub fn highlight_line(&self, line_idx: usize) -> Vec<(usize, usize, SyntaxStyle)>;

    /// Textobject query: returns (start_char, end_char) for a structural
    /// textobject target (f=function, c=class, l=loop, a=parameters).
    pub fn ts_textobject(&self, kind: char, target: char, cursor: usize, buffer: &Buffer)
        -> Option<(usize, usize)>;

    /// Rainbow bracket info: for a given char position, returns the nesting
    /// depth (for bracket characters) or None.
    pub fn bracket_depth_at(&self, pos: usize) -> Option<usize>;
}
```

### Re-parse approach

The initial implementation re-parses the full buffer on every edit via `SyntaxEngine::new()`, which calls `tree_sitter::Parser::parse()`. Full parses are fast for files under ~100KB (sub-millisecond). This avoids the complexity of tracking edit ranges and incremental re-highlighting during the first iteration.

After each buffer mutation, the App calls `SyntaxEngine::reparse(buffer_text)` — a thin wrapper that re-parses from scratch, re-runs all queries, and invalidates the highlight cache. `SyntaxEngine::edit()` and incremental re-highlighting are reserved as a future optimization if profiling shows the need.

### Highlighting

Uses `tree-sitter`'s raw highlighting approach (not `tree-sitter-highlight` crate, for more control):

1. Load highlight query from embedded `.scm` file for the language
2. Walk the tree cursor, matching query patterns against syntax nodes
3. For each match, map the capture name to a `SyntaxStyle` via `theme.rs`
4. Produce sorted, non-overlapping ranges per line

The highlight query files are embedded at compile time via `include_str!()` from the grammar crates (most grammar crates ship a `queries/highlights.scm`). Fallback: if no highlight query is available for a language, all chars get `SyntaxStyle::default()` (plain text).

---

## Language Detection

```rust
pub fn language_for_ext(ext: &str) -> Option<Language> {
    match ext {
        "rs" => Some(tree_sitter_rust::language()),
        "py" => Some(tree_sitter_python::language()),
        "js" | "mjs" | "cjs" => Some(tree_sitter_javascript::language()),
        "ts" | "tsx" => Some(tree_sitter_typescript::language_tsx()),
        "c" | "h" => Some(tree_sitter_c::language()),
        "json" => Some(tree_sitter_json::language()),
        "toml" => Some(tree_sitter_toml::language()),
        "yaml" | "yml" => Some(tree_sitter_yaml::language()),
        _ => None,
    }
}
```

When `language_for_ext` returns `None`, `SyntaxEngine::new` returns an error, and the App falls back to plain-text rendering (no syntax highlighting, no tree-sitter textobjects, no rainbow brackets for that file).

File extension is taken from the path argument passed to `ruster-bin`.

---

## Theme

Hardcoded Catppuccin mocha palette for the first iteration. Later plans will load from `.toml` theme files (as described in AGENTS.md Phase 6).

```rust
/// Map a tree-sitter highlight capture name to a SyntaxStyle.
pub fn style_for_capture(name: &str) -> SyntaxStyle {
    match name {
        "keyword"         => rgb(203, 166, 247),  // mauve
        "string"          => rgb(166, 227, 161),  // green
        "comment"         => rgb(108, 112, 134),  // overlay0, italic
        "function"        => rgb(137, 180, 250),  // blue
        "type"            => rgb(249, 226, 175),  // yellow
        "variable"        => rgb(205, 214, 244),  // text
        "constant"        => rgb(250, 179, 135),  // peach
        "number"          => rgb(250, 179, 135),  // peach
        "operator"        => rgb(137, 220, 235),  // sky
        "punctuation"     => default,             // inherit
        "tag"             => rgb(203, 166, 247),  // mauve
        "attribute"       => rgb(166, 227, 161),  // green
        "embedded"        => rgb(243, 139, 168),  // red
        "builtin"         => rgb(243, 139, 168),  // red
        _                 => default,             // plain text
    }
}
```

Bold: `"keyword"` (bold variant). Italic: `"comment"` (italic).

For captures whose names include a `.` suffix (e.g. `function.method`, `variable.parameter`), the engine strips the suffix and matches against the base name.

---

## Textobjects

### Targets

Tree-sitter textobjects add 4 new targets to the existing `VimState` textobject handler:

| Key | target | `i` (inner) | `a` (around) |
|:---:|:------:|:-----------:|:------------:|
| `f` | function | body only (between braces) | signature + body |
| `c` | class/struct/trait | body only | declaration + body |
| `l` | loop (for/while/loop) | body only | keyword + body |
| `a` | parameter list | params only | parens + params |

### Query files

Each language ships a `textobjects.scm` query file, embedded at compile time. Examples:

**Rust textobjects.scm:**
```scheme
;; function
(function_item body: (_) @function.inner) @function.outer
(closure_expression body: (_) @function.inner) @function.outer

;; struct/trait
(struct_item body: (_) @class.inner) @class.outer
(trait_item body: (_) @class.inner) @class.outer
(impl_item body: (_) @class.inner) @class.outer

;; loop
(for_expression body: (_) @loop.inner) @loop.outer
(while_expression body: (_) @loop.inner) @loop.outer
(loop_expression body: (_) @loop.inner) @loop.outer

;; parameters
(parameters) @parameter.outer
(parameters "," (_) @parameter.inner)
```

### Integration

`range_for_textobj` stays in `ruster-core` for existing targets (`w`, `"`, `'`, `(`, `)`, `{`, `}`). Tree-sitter textobjects use a new `Action::Textobject` variant that carries operator info:

```rust
pub enum Action {
    // ... existing variants ...
    /// Tree-sitter textobject: operator 'd'/'y'/'c', kind 'i'/'a', target 'f'/'c'/'l'/'a'
    Textobject { op: char, kind: char, target: char, count: u32 },
}
```

In `VimState::handle_normal`, when a tree-sitter target (`f`, `c`, `l`, `a`) is hit after `i` or `a`, VimState emits `Action::Textobject` and clears the pending state (no operator application). The `Editor::execute` treats `Textobject` as a no-op (same pattern as `CmdlineResult`).

The App intercepts `Textobject` in its event loop before calling `editor.execute`:
```rust
for action in self.vim.handle(key, &self.editor) {
    match action {
        Action::CmdlineResult(cmd) => { ... }
        Action::Textobject { op, kind, target, count } => {
            if let Some((start, end)) = self.syntax.as_ref()
                .and_then(|s| s.ts_textobject(kind, target, editor.primary_head(), editor.buffer()))
            {
                self.exec_operator(op, start, end);
            }
        }
        other => self.editor.execute(other),
    }
}
```

`exec_operator` applies the operator (DeleteRange for `d`, Yank for `y`, DeleteRange + Insert for `c`) to the editor, mirroring `VimState::apply_operator`.

### Per-language query files

For each supported language, two `.scm` query files are embedded:
- `highlights.scm` — from the grammar crate's `queries/highlights.scm`
- `textobjects.scm` — custom, written for this project

If a language's grammar crate does not ship a `highlights.scm`, the engine skips highlighting (plain text) but still provides structural features from the parser tree.

---

## Rainbow Brackets

### Approach

After parsing, walk the tree-sitter tree to identify bracket nodes (`(`, `)`, `{`, `}`, `[`, `]`). Compute nesting depth for each bracket by maintaining a counter as we traverse in source order.

```rust
pub struct BracketInfo {
    /// Character position of each bracket pair with its nesting depth
    pub brackets: Vec<(usize, char, usize)>, // (pos, bracket_char, depth)
}
```

Called right after highlighting. The depth is stored per bracket position. During rendering, if a character is a bracket, the `BufferWidget` looks up its depth and applies the cycling color.

### Cycling palette

Nesting depth `d` maps to `palette[d % 6]`:
1. `#f38ba8` (red)
2. `#fab387` (peach)
3. `#f9e2af` (yellow)
4. `#a6e3a1` (green)
5. `#89beb4` (teal)
6. `#89b4fa` (blue)

Rainbow colors override the syntax-highlight color for bracket characters only.

### Integration

`SyntaxEngine` stores a `Vec<Option<usize>>` (one entry per char position in the buffer, or `None` if not a bracket). On edit + re-parse, this vector is recomputed for the affected range. The `highlight_line()` method merges rainbow bracket info with syntax highlights: if a char position has a bracket depth, its style's `fg` is overridden with the cycling color.

---

## Changes to Existing Crates

### `ruster-render`

- Add `Color`, `SyntaxStyle`, `StyledLine` types
- Change `EditorState.lines: Vec<String>` → `Vec<StyledLine>`
- Test adjustments for new types

### `ruster-core`

- Add `Action::Textobject { op: char, kind: char, target: char, count: u32 }` variant
- `Editor::execute`: no-op match for `Textobject`

### `ruster-tui`

- `App` gains `syntax: Option<SyntaxEngine>` field
- SyntaxEngine initialized on `App::new()` if language is detected
- `App::render()` calls `SyntaxEngine::highlight_line()` per line to build `StyledLine`s
- `App::run()` calls `SyntaxEngine::reparse()` on each buffer mutation
- `BufferWidget::render` draws per-char fg from `StyledLine.highlights`
- `TuiRenderer` maps `Color` → ratatui colors

### `ruster-bin`

- No change (already passes file path to App)

### `ruster-syntax` (new crate)

- ~500 lines of Rust total across 5 modules
- 9 highlight `.scm` files embedded from grammar crates
- 9 custom `textobjects.scm` query files (one per language)

---

## Error Handling

### Parse errors

Tree-sitter always produces a tree, even for incomplete/corrupt source (incremental parsing). Syntax highlighting works on partial trees. No crash path.

### Missing grammar

If `language_for_ext` returns `None`:
- `SyntaxEngine::new` returns `Err(SyntaxError::UnsupportedLanguage)`
- `App` stores `syntax: None`
- All rendering falls through to plain text (no colors, no TS textobjects, no rainbow brackets)
- No crash — feature degrades gracefully

### Missing highlight query

If a grammar crate doesn't include a `highlights.scm`:
- The engine still parses (textobjects work)
- Highlighting produces empty ranges (plain text)
- Rainbow brackets still work (derived from the tree)

---

## Testing

### Unit tests in `ruster-syntax`

- `SyntaxEngine::new` with Rust source → tree is non-null
- `highlight_line` returns expected highlights for a simple Rust snippet
- `ts_textobject` returns correct ranges for function/class/loop
- `bracket_depth_at` returns correct depths for nested brackets
- `reparse` after buffer mutation re-parses correctly
- Unsupported extension → `Err(SyntaxError::UnsupportedLanguage)`
- Empty file → empty `StyledLine`

### Integration tests in `ruster-tui`

- App with syntax enabled renders without error
- Typing in Insert mode triggers `SyntaxEngine::edit()` — no crash

### File-based test

Open `ruster-core/src/vim/mod.rs` in the editor; syntax highlights appear. Manually verify colored keywords, strings, comments.

---

## Migration Path

The change from `Vec<String>` to `Vec<StyledLine>` in `EditorState` is the only breaking API change in `ruster-render`. All existing tests (74 across the workspace) will need minor adjustments:
- Tests that construct `EditorState` manually need to wrap strings in `StyledLine`
- `StyledLine { text: s, highlights: vec![] }` — easy find-and-replace pattern

---

## Delivery Order

The implementation plan will deliver in this order:

1. **ruster-syntax crate scaffold** — `SyntaxEngine` with parsing, highlighting, theme
2. **ruster-render types** — `Color`, `SyntaxStyle`, `StyledLine`, update `EditorState`
3. **ruster-tui integration** — `App` gains `SyntaxEngine`, renderer draws colors
4. **Textobjects** — `Action::Textobject`, query files, VimState dispatch
5. **Rainbow brackets** — bracket depth computation, render-time color override
6. **Verification** — `cargo test --workspace` passes, visual check on Rust source
