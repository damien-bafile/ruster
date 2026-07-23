# Phase 3: Syntax & Code Intelligence — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Design spec:** [2026-07-23-phase3-code-intelligence-design.md](../specs/2026-07-23-phase3-code-intelligence-design.md)

**Goal:** Finish the tree-sitter layer and add an LSP client with the code-intelligence
features on top: diagnostics, hover, go-to-definition, references, rename, formatting,
and a symbol outline — plus a snippet engine.

**Architecture:** A new `ruster-lsp` crate speaks JSON-RPC over a language server's stdio
using **std threads + channels** (no tokio), so the app polls LSP messages each frame like
the Phase 2 `:Rg` streaming. `lsp-types` provides the protocol types. Syntax becomes
per-buffer. New deps: `lsp-types`, `serde`, `serde_json`.

## Global Constraints

- LSP is fully **non-blocking**: requests fire from handlers, responses arrive later via a
  per-frame poll in `render` and update UI. Never block the render loop on a server.
- One `LspClient` per (server, workspace-root); missing servers degrade gracefully with a
  message, never a crash.
- Positions convert through one shared helper (ruster char-offset ↔ LSP UTF-16 line/char).
- Per-language tree-sitter queries; a missing query means no-highlight, never wrong-highlight.
- Keep `docs/config-reference.md` and `docs/lua-api.md` in sync.

---

### Task 1: Per-language tree-sitter queries + per-buffer syntax

**Files:**
- Modify: `crates/ruster-syntax/src/lib.rs` (`query_files_for_lang` selects by language)
- Create: `crates/ruster-syntax/queries/<lang>/highlights.scm` for bundled languages
- Modify: `crates/ruster-tui/src/app.rs` (`syntax: HashMap<BufferId, SyntaxEngine>`)

- [ ] **Step 1:** Add a `Lang` discriminant (or reuse the extension) so `SyntaxEngine`
  knows its language; `query_files_for_lang(lang)` returns that language's queries,
  falling back to empty (no-highlight) when absent.
- [ ] **Step 2:** Add `highlights.scm` for python, javascript, typescript, json, toml,
  yaml, c (vendored from nvim-treesitter or minimal hand-written); keep rust's textobjects.
- [ ] **Step 3:** In `app.rs`, replace `syntax`/`syntax_buffer` with
  `HashMap<BufferId, SyntaxEngine>`; build a buffer's engine on open from its extension,
  reparse the active buffer each frame, and use each window's buffer engine for highlights.
- [ ] **Step 4: Tests:** a Python source highlights with Python queries (not Rust); opening
  a second file of a different language highlights correctly; unsupported ext → plain text.
- [ ] **Step 5:** `cargo test -p ruster-syntax -p ruster-tui`.
- [ ] **Step 6:** Commit: `fix: per-language tree-sitter queries and per-buffer syntax`

---

### Task 2: `ruster-lsp` crate — JSON-RPC transport & client

**Files:**
- Create: `crates/ruster-lsp/{Cargo.toml, src/lib.rs, src/transport.rs, src/client.rs}`
- Modify: root `Cargo.toml` (workspace member)

- [ ] **Step 1:** New crate with deps `lsp-types`, `serde`, `serde_json`. Add to workspace.
- [ ] **Step 2:** `transport.rs`: read/write LSP frames (`Content-Length` headers). A reader
  thread parses frames off a `Read` and sends `ServerMessage { Response | Notification }`
  over an `mpsc::Sender`.
- [ ] **Step 3:** `client.rs`: `LspClient::spawn(cmd, args, root)` starts the child with
  piped stdio and the reader thread. `request(method, params) -> id`, `notify(method,
  params)`, `poll() -> Vec<ServerMessage>`, `shutdown()`.
- [ ] **Step 4: Tests:** frame round-trip (encode then decode yields the same JSON); a
  fake in-memory server (two pipes) exercises request → response id correlation and a
  notification, with no real process.
- [ ] **Step 5:** `cargo test -p ruster-lsp`.
- [ ] **Step 6:** Commit: `feat: ruster-lsp JSON-RPC transport and client`

---

### Task 3: Server registry, lifecycle & document sync

**Files:**
- Create: `crates/ruster-lsp/src/registry.rs`
- Modify: `crates/ruster-tui/src/app.rs` (LSP manager, per-buffer doc state, per-frame poll)

- [ ] **Step 1:** `registry.rs`: filetype → (command, args) defaults (rust-analyzer,
  pyright, typescript-language-server). `LspManager` holds `HashMap<Lang, LspClient>`,
  lazily spawning on first use and running `initialize`/`initialized`.
- [ ] **Step 2:** On buffer open of a supported filetype: `didOpen` (uri, language, version 0,
  text). On edit: bump version, `didChange` (full text). On close/quit: `didClose` /
  `shutdown` + `exit`.
- [ ] **Step 3:** In `App::render` (or a per-frame update), `poll()` every client and route
  messages: notifications to a handler, responses to pending-request dispatch (Task 4+).
- [ ] **Step 4: Tests:** registry returns the right default command per filetype; version
  counter increments on edits; poll routes a fake notification. (No live server.)
- [ ] **Step 5:** `cargo test -p ruster-lsp -p ruster-tui`.
- [ ] **Step 6:** Commit: `feat: LSP server registry, lifecycle, and document sync`

---

### Task 4: Diagnostics

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` (store diagnostics per buffer uri)
- Modify: `crates/ruster-render/src/lib.rs` + backends (gutter signs; GUI underlines)

- [ ] **Step 1:** Handle `textDocument/publishDiagnostics`: store `Vec<Diagnostic>` keyed by
  buffer. Map LSP ranges to ruster line/col.
- [ ] **Step 2:** Render: diagnostic sign in the gutter (`E`/`W`, colored) per affected line;
  in the GUI, a colored underline under the range. Add the active line's diagnostic message
  to the statusline or a virtual-text slot.
- [ ] **Step 3:** `:diagnostics` / `Space c d` opens diagnostics in a Picker; Enter jumps.
- [ ] **Step 4: Tests:** parse a `publishDiagnostics` payload into the store; gutter-sign
  computation marks the right lines; picker lists them.
- [ ] **Step 5:** `cargo test -p ruster-tui`.
- [ ] **Step 6:** Commit: `feat: LSP diagnostics with gutter signs and list`

---

### Task 5: Hover (`K`)

**Files:** Modify `crates/ruster-tui/src/app.rs`; `ruster-render` + backends (hover popup).

- [ ] **Step 1:** `K` / `Space c k` sends `textDocument/hover` at the cursor; on response,
  store the markdown/plaintext contents and show a floating bordered popup near the cursor.
- [ ] **Step 2:** Render the popup in both backends (reuse the picker-style box); dismiss on
  the next key.
- [ ] **Step 3: Tests:** hover response contents parse into popup lines; popup wraps/truncates.
- [ ] **Step 4:** `cargo test -p ruster-tui`.
- [ ] **Step 5:** Commit: `feat: LSP hover popup`

---

### Task 6: Go-to-definition (`gd`)

**Files:** Modify `crates/ruster-tui/src/app.rs`.

- [ ] **Step 1:** `gd` / `Space c g` sends `textDocument/definition`; on response, open the
  target (reuse `open_path(path, Some((line, col)))`), handling `Location` / `LocationLink`
  and single-vs-array results.
- [ ] **Step 2: Tests:** parse definition result variants into a (path, line, col).
- [ ] **Step 3:** Commit: `feat: LSP go-to-definition`

---

### Task 7: Find references (`gr`)

**Files:** Modify `crates/ruster-tui/src/app.rs`.

- [ ] **Step 1:** `gr` / `Space c r` sends `textDocument/references`; on response, populate a
  Picker of `file:line:col` locations (reuse the picker + `OpenLocation`).
- [ ] **Step 2: Tests:** references array parses into picker items.
- [ ] **Step 3:** Commit: `feat: LSP find references`

---

### Task 8: Rename & formatting

**Files:** Modify `crates/ruster-tui/src/app.rs`.

- [ ] **Step 1:** `Space c n` prompts (mini-buffer) for a new name → `textDocument/rename` →
  apply the `WorkspaceEdit` (text edits across affected buffers, opening them as needed).
- [ ] **Step 2:** `Space c f` / `:fmt` sends `textDocument/formatting`; apply returned edits
  to the buffer. Optional format-on-save behind a config flag.
- [ ] **Step 3: Tests:** apply a `WorkspaceEdit`/edit list to a buffer produces expected text
  (pure edit-application helper, no server).
- [ ] **Step 4:** Commit: `feat: LSP rename and formatting`

---

### Task 9: Document symbols → outline + workspace symbol search

**Files:** Modify `crates/ruster-tui/src/app.rs`; a symbols panel (special buffer or picker).

- [ ] **Step 1:** `Space c o` sends `textDocument/documentSymbol`; render a hierarchical
  outline (a Special buffer like dired, or a side split); Enter jumps to the symbol.
- [ ] **Step 2:** `Space c s` sends `workspace/symbol` with the typed query → Picker → jump.
- [ ] **Step 3: Tests:** documentSymbol (both `DocumentSymbol` tree and `SymbolInformation`
  flat) parse into outline entries.
- [ ] **Step 4:** Commit: `feat: LSP document outline and workspace symbol search`

---

### Task 10: Snippet engine

**Files:** Create `crates/ruster-core/src/snippets.rs`; modify `crates/ruster-tui/src/app.rs`.

- [ ] **Step 1:** `snippets.rs`: parse snippet bodies with tabstops (`$1`,`$2`,`$0`,
  `${1:default}`); an expansion produces the inserted text and an ordered list of tabstop
  ranges. Load definitions per filetype from `~/.config/ruster/snippets/`.
- [ ] **Step 2:** In `app.rs`, expand the trigger word before the cursor on a key (e.g. Tab
  in insert when a trigger matches); place the cursor at `$1`, cycle stops with Tab/Shift-Tab.
- [ ] **Step 3: Tests:** parsing tabstops; expansion text + stop ranges; cycling order.
- [ ] **Step 4:** Commit: `feat: snippet engine with tabstops`

---

### Task 11: Lua API, config & docs

**Files:** Modify `crates/ruster-lua/*`; `docs/lua-api.md`, `docs/config-reference.md`.

- [ ] **Step 1:** Lua LSP config: `ruster.lsp.servers[filetype] = { cmd, args }` to override
  the registry; `format_on_save` flag; snippet registration hook.
- [ ] **Step 2:** Expose events/handlers (`ruster.on("LspAttach", …)`, diagnostics count for
  the statusline).
- [ ] **Step 3:** Update `docs/lua-api.md` and `docs/config-reference.md` for every new
  setting, command, keybinding, and API.
- [ ] **Step 4:** Commit: `feat: Lua LSP config and Phase 3 docs`

---

### Final Verification

- [ ] Full test suite across all crates including `ruster-lsp`.
- [ ] Build all: `cargo check -p ruster-bin -p ruster-tui -p ruster-render-raylib`.
- [ ] Manual smoke with `rust-analyzer` installed: open a `.rs` file, see diagnostics; `K`
  hover; `gd` jump; `gr` references; rename; `Space c f` format; `Space c o` outline.
- [ ] Docs reflect every new setting, command, and API.
- [ ] Expected: all tests pass, no new warnings.

## Notes on verification limits

The JSON-RPC transport, position mapping, message routing, edit application, and snippet
engine are all unit-testable with fake in-memory servers / pure helpers. **End-to-end LSP
behavior needs a real language server** (e.g. `rust-analyzer`) and can only be confirmed by
a manual run — the plan isolates protocol logic from I/O so the bulk is covered by tests.
