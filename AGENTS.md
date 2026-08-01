# `ruster` – A Modern Hybrid Editor (Neovim + Emacs)

**Core Philosophy:** Written in Rust for raw performance (60fps TUI/GUI), functionally extended entirely via Lua scripting for plugins and configuration. Aims to be a fully self-contained IDE and application platform.

---

## Phase 0: Foundation & Core Engine
*(The bare metal – get the app running, looping, and scriptable)*

| Feature | Implementation / Tech |
| :--- | :--- |
| **Cross-platform Base** | `winit` + `tokio` (async runtime). Targets: Windows, macOS, Linux, FreeBSD, BeOS (via `rustix`). |
| **GUI Backend** | `raylib` (primary) for immediate-mode GUI rendering. |
| **TUI Backend** | `ratatui` + `crossterm` (fallback/SSH mode). Will be extended with third‑party widget libraries (see Phase 6). |
| **Animation System** | `tachyonfx` – drives the 60fps refresh loop for animations, cursor blinking, and live updates. |
| **Lua Scripting Engine** | `mlua` (Lua 5.4 / Luau). Provides the backbone for ALL configuration and plugin logic. |
| **Text Buffer** | `ropey` – fast, thread-safe CRDT-like rope data structure for massive files. |
| **Event Loop** | Unified channel-based system (`tokio::sync::mpsc`). UI events, LSP messages, and timers are all multiplexed here. |

---

## Phase 1: Text Editing Primitives
*(The "Notepad" stage – moving cursors and changing text)*

| Feature | Implementation / Tech |
| :--- | :--- |
| **Cursor Management** | Multiple cursors supported natively in the buffer model. |
| **Insert / Delete** | Basic character insertion, newlines, backspace, delete. |
| **Undo / Redo** | `ropey`'s undo engine integrated with a custom history tree (undo-tree). |
| **Selection Modes** | Character-wise, line-wise, and block-wise (rectangle) visual selections. |
| **Clipboard (Copy/Paste)** | `arboard` crate – system clipboard integration across all OSes. |
| **Dual Editing Paradigm (Neovim ↔ Emacs Toggle)** | Global toggle (`:set editmode neovim|emacs`). **Neovim mode:** Full modal editing with Normal/Insert/Visual/Command-line states; operators (`d`, `y`, `c`, `>`), text objects (`iw`, `ap`, `it`, etc.), dot-repeat (`.`), macro recording/playback (`q`), and `:substitute`. **Emacs mode:** Modeless editing with standard `Ctrl`/`Alt` chords (`C-s`, `C-k`, `C-y`, `M-f`, `M-b`), kill-ring (clipboard history), prefix arguments (`C-u`), and incremental search (`C-s`/`C-r`). Switching modes rebinds keymaps dynamically, updates the statusline indicator, and alters mini-buffer prompt behavior. Lua plugins can query `editor.editmode` to support both paradigms. |
| **Tabs & Indentation** | Tab key inserts soft spaces. Number of spaces configurable via Lua (`vim.opt.tabstop`). |
| **EditorConfig support** | Native parser for `.editorconfig` to override indentation, charset, and line endings per project. |

---

## Phase 2: Buffer, Window & File Management
*(The layout – moving from a single file to a workspace)*

| Feature | Implementation / Tech |
| :--- | :--- |
| **Window Splits** | Vertical and horizontal splits (Emacs-style window tree). Lua API to manipulate layouts. |
| **Mini-buffer (Cmdline)** | Floating command bar with `which-key` style popups showing available keymaps. |
| **Toggle Fullscreen** | Keyboard shortcut to expand the mini-buffer/terminal into a full-screen pane and back. |
| **Ibuffer (Buffer List)** | Interactive buffer switching UI (filter by name, mode, modified status). |
| **Dired (File Explorer)** | Built-in file manager inside a buffer – create, delete, rename, open files. |
| **FZF / Ripgrep** | External fuzzy-finder integration for opening files (`:Files`) and live grep (`:Rg`). |
| **Gutter (Line Numbers)** | Configurable: sequential, relative, or hybrid (current line absolute, others relative). |
| **Statusline (Lualine)** | Extensible statusline written in Lua, showing mode, LSP status, git branch, cursor position. |

---

## Phase 3: Syntax & Code Intelligence
*(Make it "Smart" – Tree-sitter and LSP)*

| Feature | Implementation / Tech |
| :--- | :--- |
| **Tree-sitter Parser** | Incremental, error-tolerant syntax highlighting and code parsing. |
| **Textobjects** | `nvim-treesitter-textobjects` style – select functions, classes, loops (e.g., `daf` delete around function). |
| **Rainbow Brackets / Delimiters** | Use Tree-sitter's syntax tree to colorize matching brackets (`()`, `[]`, `{}`) with distinct, cycling colors based on nesting depth. Fully themeable via Lua (users can define their own color palettes). |
| **LSP Client** | Built-in Language Server Protocol client. Supports hover, go-to-definition, find-references, rename. |
| **Symbol Search** | Project-wide search for functions, variables, types using LSP or Tree-sitter tags. |
| **Code Outline** | Sidebar or floating panel listing hierarchical symbols in the current buffer. |
| **Call Hierarchy** | View incoming/outgoing calls for a function (via LSP). |
| **Code Formatting** | On-save formatting via LSP `textDocument/formatting`. Falls back to external formatters. |
| **Code Snippets** | `LuaSnip` style engine. Load snippets from `~/.ruster/snippets/`. |
| **Hover/Type Preview** | Show documentation and inferred types in a popup (akin to `K` in Neovim). |

---

## Phase 4: The Embedded Terminal
*(Cross-platform PTY layer – full shell inside your editor)*

| Component | Implementation / Tech |
| :--- | :--- |
| **PTY Backend** | [`portable-pty`](https://crates.io/crates/portable-pty) – Unified API. On Unix uses `forkpty`; on Windows uses Microsoft's **ConPTY** (Win10 1809+). |
| **Terminal State Machine** | [`alacritty_terminal`](https://crates.io/crates/alacritty_terminal) – Parses raw VT100/ANSI escape sequences into an in-memory grid buffer (cells with fg/bg/attributes). |
| **Rendering (GUI)** | Raylib renders the `Term` grid by drawing colored quads for background and `fontdue`-rasterized glyphs. Only "dirty" cells are updated each 60fps frame. |
| **Rendering (TUI)** | Ratatui translates the `Term` grid into `Span`/`Line` widgets. Relies on host terminal's fixed monospace fonts. |
| **Input Forwarding** | Keypresses are converted to ASCII/escape sequences and written to the PTY master. Mouse events forwarded via `\x1b[<...` sequences. |
| **Threading** | PTY reader runs in a dedicated blocking thread. Sends byte diffs via `mpsc` channel to the main UI thread to maintain 60fps. |
| **Scrollback** | Unlimited configurable scrollback history stored in the `alacritty_terminal` grid. |

---

## Phase 5: IDE & Debugging Tools
*(The "VS Code / IntelliJ" power features)*

| Feature | Implementation / Tech |
| :--- | :--- |
| **DAP (Debugger)** | Debug Adapter Protocol client. Connect to `lldb-vscode`, `gdb`, or `msvsmon`. Supports breakpoints, stack traces, variable watches. |
| **Build System** | Parse `Cargo.toml`, `Makefile`, `package.json`. Run builds and parse compiler errors into quickfix list. |
| **Test Runner** | Discover and run tests (e.g., `cargo test`). Show passing/failing inline in the buffer (gutter icons). |
| **Task Runner** | User-defined tasks in `ruster.toml` (build, deploy, lint). Run them in the embedded terminal or a background thread. |
| **File Explorer Sidebar** | A `neo-tree` style tree view in the side window. Create/delete/rename files and folders. |
| **Project Workspaces** | Project-specific configuration (root markers like `.git`, `ruster.toml`). Quick switching between recent projects. |
| **Multi-cursor Editing** | Fully integrated with the editing engine – add cursors via `Ctrl+D` or mouse + `Alt`. |

---

## Phase 6: Advanced UX & Ecosystem
*(Polish, plugins, and "magic" features)*

| Feature | Implementation / Tech |
| :--- | :--- |
| **Theme System & Picker UI** | Live theme switcher UI. Ships with **Catppuccin** (latte, frappe, macchiato, mocha). **Dynamic loading:** On startup, the editor scans `~/.config/ruster/themes/*.toml` and a system‑wide fallback directory (e.g., `/usr/share/ruster/themes/`) for theme definitions. The Theme Picker UI lists all discovered themes, shows a live preview, and applies the selected theme instantly. Users can add, modify, or remove themes without restarting – the picker re‑scans the folder each time it opens. |
| **Configuration Browser (`:RusterConfig`)** | Introspective buffer that lists *every* available option (editor, LSP, UI, terminal, plugins) by scanning the TOML schema and Lua runtime. Features live search (`/`), inline value editing (`<CR>`), reset to default (`d`), grouping by category, and instant application of changes. Eliminates the need to manually edit the config file for simple tweaks. |
| **TUI Widget Libraries** | Extend Ratatui with ready‑made interactive controls for building complex dialogs, settings panels, and plugin UIs. Recommended crates: <br> • **`ratatui-widgets`** – Buttons, radio buttons, checkboxes, toggles, sliders, dropdowns, menus.<br> • **`ratada`** – Full‑featured toolkit with modal dialogs, forms, pickers (color, date, path), tables, trees, fuzzy finders, and Markdown renderers.<br> • **`textual-rs`** – Reactive framework with 25+ widgets including RadioSet, Select, and more.<br> Integration: all widgets are exposed via Lua so plugins can create interactive UI elements. |
| **Git Signs (gitsigns)** | Gutter indicators for added, modified, and removed lines. Hunk staging via Lua. |
| **Todo Comments** | Highlight `TODO`, `FIXME`, `HACK` in the buffer. Allow listing them in a `trouble.nvim` style panel. |
| **Trouble.nvim** | A list-style panel for diagnostics, TODOs, LSP references, or quickfix items. |
| **Noice (Notifications)** | Non-blocking message system with a history viewer. Replaces default `:echo` with a modern UI. |
| **Mason (Tool Installer)** | UI to download and manage LSP servers, DAP servers, linters, and formatters. |
| **Flash (Jump Mode)** | Highlight labels for every visible word. Type two letters to jump anywhere (`s` + `<char><char>`). |
| **Diff Viewer** | Side-by-side or inline diff for Git working tree changes (`:Diffview`). |
| **Markdown / Org-mode** | Live preview rendering for Markdown and basic Org-mode (export to HTML/PDF). |
| **Plugin Manager** | Built-in UI to install/update Lua plugins from Git repos. Sandboxed Lua environments. |
| **Client/Server Model** | Daemonize the editor (like Emacs `--daemon`). Clients connect via Unix sockets or TCP to open new frames. |

---

## Phase 7: The "Emacs Extras" (Application Platform)
*(Everything else – turning the editor into an OS)*

| Feature | Implementation / Tech |
| :--- | :--- |
| **Magit Clone** | A full-featured Git porcelain UI built in Lua on top of `git2-rs`. |
| **Music Player** | `listen.el` style – control `mpd` or local player via UI. |
| **Email Client** | Gmail support via IMAP/SMTP. Render emails as HTML/Markdown in a buffer. |
| **Web Browser** | Embedded browser using `webkit2gtk` (Linux/macOS) / `WebView2` (Windows). Or text-mode browser for TUI. |
| **Help Menu** | Interactive help system (`:help`). Shows keymaps, function docs, and plugin APIs in a formatted buffer. |
| **Session Management** | Save/restore open buffers, window layouts, cursor positions, and terminal history. |

---

## Phase 8: Finetuning
*(No new features – paying down what building the others left behind)*

| Area | What |
| :--- | :--- |
| **Theming** | Eight sites draw fixed RGB that no theme can reach: git gutter signs, diagnostic/debug signs, dired entry types, flash labels, and a TUI-only toast background. Route them through the `diff` pseudo-language pattern already proven for `:GitStaged`. |
| **Missing basics** | `:16` goto-line, `:hover` — the latter also being the only deliberate way to put a float on screen. |
| **Stale render state** | `App::floats` has no writer; the raylib float/dialog draw order contradicts its own comment. |
| **Performance** | `SyntaxEngine::reparse` allocates a `Parser` and does a *full* reparse every frame. Incremental parsing is switched off, and it is the largest win available. |
| **`App` growth** | 127 → 151 → **189** methods across PR #24, Phase 6 and Phase 7; `app.rs` now 7,583 non-test lines. Track method and line count, **not** graphify betweenness — that measures being the composition root, which extraction cannot change. |
| **Lua extensibility** | `ruster.cmd` already makes every `:` command a Lua API, so commands are not the gap. Plugins can be *invoked* but barely *react*: four events exist where Neovim has sixty, there is no timer, and no read-only introspection. |
| **Application icon** | No `.icns`, `.ico` or `.desktop` anywhere, and the raylib window shows the default. Runtime window icon first; macOS needs an `.app` bundle before a Dock icon is possible at all. |

Plan: [docs/superpowers/plans/2026-08-02-phase8-finetuning.md](docs/superpowers/plans/2026-08-02-phase8-finetuning.md)

---

## Cross-Cutting Technical Decisions

- **Configuration**: Stored in `~/.config/ruster/ruster.toml` (TOML) with Lua overrides for dynamic logic.
- **Theme Loading**: Themes are defined as standalone TOML files. **Schema:**
  ```toml
  [metadata]
  name = "My Custom Theme"
  author = "Your Name"
  description = "A warm dark theme"
  
  [colors]
  background = "#1e1e2e"
  foreground = "#cdd6f4"
  cursor = "#f5e0dc"
  selection = "#585b70"
  comment = "#6c7086"
  # ... plus syntax‑highlighting groups (function, keyword, string, etc.)

## Documentation Maintenance

**All documentation in `docs/` must be kept in sync with the codebase.**
When you implement a new feature, change an existing setting, or modify the Lua API:
1. Update `docs/config-reference.md` if settings change
2. Update `docs/lua-api.md` if the Lua surface changes
3. Update `docs/keybindings.md` if a keybinding or `:` command changes
3. If you created a new doc, add a reference in the relevant phase section above

**This includes:**
- Adding new `ruster.config` settings
- Adding new `ruster.api.*` functions
- Adding new events for `ruster.on()`
- Changing default values or behavior
