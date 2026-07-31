# Phase 3: Syntax & Code Intelligence — Design

## Overview

Phase 3 makes ruster "smart": it finishes the tree-sitter layer and adds a
**Language Server Protocol (LSP) client** — the centerpiece — plus the
code-intelligence features built on it: diagnostics, hover, go-to-definition,
find-references, rename, formatting, and a symbol outline. It also adds a
snippet engine.

Scope is the Phase 3 row of `AGENTS.md`.

## What already exists (from Phase 0–2)

`ruster-syntax` already has tree-sitter parsing for 8 languages, syntax
highlighting, **rainbow brackets** (`compute_bracket_depths` + `RAINBOW_PALETTE`),
and **`ts_textobject`** for function/class/loop/parameter inner/outer — wired into
vim via `Action::Textobject` (`daf`, `cif`, …). So a good chunk of the Phase 3
"tree-sitter" row is done.

**Known gaps to fix first:**
- ~~`query_files_for_lang()` **always returns the Rust queries** regardless of
  language — so highlighting/textobjects are wrong for Python/JS/etc. Only
  `queries/rust/` exists.~~ **Fixed during Phase 3** — it now matches on the
  language key and returns that language's queries.
  - Follow-up (2026-07-31): JavaScript and TypeScript had grammars registered
    and extensions mapped but no query files, so they parsed into a tree and
    highlighted nothing. `queries/javascript/` and `queries/typescript/` now
    exist, and `qcheck::every_parseable_language_has_a_highlight_query` fails
    the build if a grammar is ever added without one again.
- Syntax is bound to the **initially-opened file** (`App.syntax` + `syntax_buffer`);
  buffers opened later render as plain text (Phase 2 note). Needs to be per-buffer.

## Core: the LSP client (`ruster-lsp`, new crate)

A new `ruster-lsp` crate speaking JSON-RPC 2.0 over a language server's stdio.
Deliberately **std threads + channels**, not tokio — this matches the existing
background-work pattern (the Phase 2 `:Rg`/`:Files` streaming and `mpsc` drain in
`render`), so the app polls LSP messages each frame with no async runtime coupling.

### Transport & framing
- Spawn the server as a child process (`std::process::Command`, piped stdio).
- LSP framing: `Content-Length: N\r\n\r\n<json>`. A dedicated **reader thread**
  parses frames off the child's stdout and sends `ServerMessage`s over an `mpsc`
  channel. A **writer** serializes requests/notifications to the child's stdin.
- Types come from the **`lsp-types`** crate; payloads (de)serialized with `serde_json`.

### Client model — `ruster-lsp/src/client.rs`
```rust
pub struct LspClient {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<ServerMessage>,   // from the reader thread
    next_id: i64,
    pending: HashMap<i64, &'static str>, // id -> method, for routing responses
    initialized: bool,
}

pub enum ServerMessage {
    Response { id: i64, result: serde_json::Value, error: Option<ResponseError> },
    Notification { method: String, params: serde_json::Value },
}

impl LspClient {
    pub fn spawn(cmd: &str, args: &[String], root: &Path) -> io::Result<Self>;
    pub fn request(&mut self, method: &'static str, params: impl Serialize) -> i64; // returns request id
    pub fn notify(&mut self, method: &str, params: impl Serialize);
    pub fn poll(&self) -> Vec<ServerMessage>;  // drain the channel (non-blocking)
    pub fn shutdown(&mut self);                // shutdown + exit
}
```

### Lifecycle
`initialize` (with client capabilities) → `initialized` notification → per-document
`textDocument/didOpen`. On buffer edits: `didChange` (full-text sync to start; ranged
later). On close: `didClose`. On quit: `shutdown` + `exit`.

### Registry — `ruster-lsp/src/registry.rs`
Maps a filetype to a server command (default: `rust` → `rust-analyzer`, `python` →
`pyright-langserver --stdio`, `typescript`/`javascript` → `typescript-language-server
--stdio`). One `LspClient` per (server, workspace-root). Configurable from Lua later.

## App integration — `ruster-tui`

`App` gains an `Lsp` manager holding clients keyed by language, plus per-buffer
LSP document state (version counter, URI). Each frame (in `render`, like the picker
drain) the app **polls every client** and dispatches `ServerMessage`s:
- `textDocument/publishDiagnostics` → store diagnostics for that buffer's URI.
- responses → resolve the pending action (hover popup, jump, references picker, …).

Requests are fired from key/command handlers; the response arrives a frame or more
later and updates UI — fully non-blocking.

### Feature surface
- **Diagnostics**: stored per buffer; rendered as gutter signs (`E`/`W`) and, in the
  GUI, colored underlines; `:diagnostics` / `Space c d` opens them in a Picker.
- **Hover** (`K` / `Space c k`): `textDocument/hover` → floating popup (reuses a
  bordered box like the picker overlay).
- **Go-to-definition** (`gd` / `Space c g`): `textDocument/definition` → open the
  target file + jump (reuses `open_path(path, Some((line,col)))`).
- **Find references** (`gr` / `Space c r`): `textDocument/references` → Picker of
  locations → jump.
- **Rename** (`Space c n`): `textDocument/rename` → apply the returned
  `WorkspaceEdit` across buffers.
- **Formatting** (`Space c f` / on save if enabled): `textDocument/formatting` →
  apply text edits to the buffer.
- **Document symbols → outline** (`Space c o`): `textDocument/documentSymbol` →
  a sidebar/split listing symbols; Enter jumps. Workspace symbol search
  (`Space c s`) via `workspace/symbol` → Picker.

Positions convert between ruster's char offsets and LSP's UTF-16 line/character.
A shared `lsp_pos` helper handles the buffer↔LSP coordinate mapping.

## Tree-sitter polish

- **Per-language queries**: add `queries/<lang>/highlights.scm` (+ `textobjects.scm`
  where practical) for the bundled languages; `query_files_for_lang(lang)` selects by
  language instead of hardcoding Rust. Missing queries degrade to no-highlight, not wrong-highlight.
- **Per-buffer syntax**: move syntax from `App.syntax` to a `HashMap<BufferId,
  SyntaxEngine>`, created on buffer open from the file extension, reparsed on edit for
  the active buffer.

## Snippets — `ruster-core/src/snippets.rs` (new)

A minimal LuaSnip-style engine: load snippet definitions (JSON/Lua tables) keyed by
filetype and trigger word; on expand, insert the body with tabstops (`$1`, `$2`,
`$0`) and jump between them with Tab/Shift-Tab. Loaded from `~/.config/ruster/snippets/`.

## New dependencies

| Crate | Used for |
|-------|----------|
| `lsp-types` | LSP request/response/notification types |
| `serde` / `serde_json` | JSON-RPC (de)serialization |

`ruster-lsp` is a **new workspace crate**. No tokio for LSP (std threads + channels).

## Crate changes summary

| Crate | Changes |
|-------|---------|
| `ruster-lsp` (new) | JSON-RPC transport, `LspClient`, server registry, position mapping. |
| `ruster-syntax` | Per-language query selection; add language query files. |
| `ruster-core` | Snippet engine; per-buffer syntax support types if needed. |
| `ruster-tui` | Per-buffer `SyntaxEngine`; LSP manager + per-frame poll; diagnostics store & render; hover/goto/refs/rename/format/outline commands + keybindings; snippet expansion. |
| `ruster-render` + backends | Diagnostic gutter signs + underlines; hover popup; outline panel. |
| `ruster-lua` | LSP config (filetype→server), `ruster.lsp.*` hooks, snippet registration. |

## Non-goals (deferred)

- DAP / debugging (Phase 5).
- Semantic tokens highlighting (tree-sitter highlighting stays primary).
- Incremental/ranged `didChange` (full-text sync first; ranged is an optimization).
- Inlay hints, code lens, code actions beyond rename/format (later).
- Call hierarchy UI — stub the request; full UI is a follow-up.
