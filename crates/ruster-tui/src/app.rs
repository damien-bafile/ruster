use crate::key::crossterm_to_ruster_key;
use crate::picker::{PickerAction, PickerItem, PickerState};
use crate::renderer::TuiRenderer;
use ruster_core::action::{Action, EditOp, Motion};
use ruster_core::buffer::Buffer;
use ruster_core::cursor::CursorSet;
use ruster_core::document::{BufferId, DocKind, SpecialKind};
use ruster_core::editor::EditorView;
use ruster_core::key::KeyEvent;
use ruster_core::vim::VimMode;
use ruster_core::vim::VimState;
use ruster_core::windows::{FocusDir, Rect as CoreRect, SplitDir};
use ruster_core::workspace::Workspace;
use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use ruster_lua::{config::Config, LuaAction, LuaRuntime};
use ruster_render::{
    CursorKind, FrameState, Rect as RRect, Renderer, SelectionView, StatuslineView, StyledLine,
    WhichKeyView, WindowView,
};
use ruster_syntax::SyntaxEngine;
use ruster_lsp::{LspManager, LspPosition, ServerMessage};
use std::cell::RefCell;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

/// The TUI needs a real terminal on stdin (event source) and stdout (rendering).
/// On Unix `enable_raw_mode` already fails without a tty, but on Windows it can
/// succeed against piped stdio and then the event loop blocks forever, so guard
/// explicitly and error out uniformly across platforms.
fn require_terminal() -> Result<(), Box<dyn std::error::Error>> {
    if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        Ok(())
    } else {
        Err("ruster --tui requires an interactive terminal".into())
    }
}

/// Map a markdown code-fence language name to a file extension for the syntax
/// highlighter (rust-analyzer uses "rust", etc.).
fn fence_ext(lang: &str) -> &str {
    match lang {
        "rust" => "rs",
        "python" => "py",
        "javascript" => "js",
        "typescript" => "ts",
        "c" => "c",
        "lua" => "lua",
        "json" => "json",
        "toml" => "toml",
        "yaml" => "yaml",
        "scheme" => "scm",
        other => other,
    }
}

/// Highlight a code block; falls back to plain lines for unknown languages.
fn highlight_code_block(code: &str, lang: &str) -> Vec<StyledLine> {
    match SyntaxEngine::new(code, fence_ext(lang)) {
        Ok(engine) => engine.styled_lines().to_vec(),
        Err(_) => plain_lines(code),
    }
}

/// Render LSP hover markdown into styled lines: fenced code blocks are
/// tree-sitter highlighted, prose is shown plain (with fences/separators removed).
fn build_hover_lines(markdown: &str) -> Vec<StyledLine> {
    let mut out: Vec<StyledLine> = Vec::new();
    let mut in_code = false;
    let mut code_lang = String::new();
    let mut code_buf: Vec<String> = Vec::new();
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            if in_code {
                out.extend(highlight_code_block(&code_buf.join("\n"), &code_lang));
                code_buf.clear();
                in_code = false;
            } else {
                in_code = true;
                code_lang = line.trim_start().trim_start_matches("```").trim().to_string();
            }
            continue;
        }
        if in_code {
            code_buf.push(line.to_string());
        } else if line.trim() == "---" {
            continue; // drop markdown separators
        } else {
            out.push(StyledLine { text: line.to_string(), highlights: vec![] });
        }
    }
    if in_code && !code_buf.is_empty() {
        out.extend(highlight_code_block(&code_buf.join("\n"), &code_lang));
    }
    // Trim trailing blank lines and cap the height.
    while out.last().map(|l| l.text.trim().is_empty()).unwrap_or(false) {
        out.pop();
    }
    out.truncate(24);
    out
}

/// Recursively copy a directory tree.
fn copy_dir_recursive(src: &std::path::Path, dest: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Color a directory listing by entry type: directories blue, executables
/// green, symlinks teal, regular files default.
fn dired_styled_lines(entries: &[ruster_core::dired::DirEntry]) -> Vec<StyledLine> {
    use ruster_render::{Color, SyntaxStyle};
    let style = |fg: Color, bold: bool| SyntaxStyle { fg, bg: Color::Default, bold, italic: false };
    entries
        .iter()
        .map(|e| {
            let text = if e.is_dir {
                format!("{}/", e.name)
            } else {
                e.name.clone()
            };
            let s = if e.is_symlink {
                style(Color::Rgb(137, 220, 235), false) // teal
            } else if e.is_dir {
                style(Color::Rgb(137, 180, 250), true) // blue, bold
            } else if e.is_exec {
                style(Color::Rgb(166, 227, 161), false) // green
            } else {
                style(Color::Default, false)
            };
            let len = text.chars().count();
            let highlights = if matches!(s.fg, Color::Default) {
                Vec::new()
            } else {
                vec![(0, len, s)]
            };
            StyledLine { text, highlights }
        })
        .collect()
}

/// The dired keymap, shown as a popup by `?`.
fn dired_help_lines() -> Vec<StyledLine> {
    let entries = [
        "Enter / l    open file or enter directory",
        "h / -        parent directory",
        "j / k        move cursor",
        "C-n / C-p    move cursor down / up",
        "yy           copy entry",
        "dd           cut entry",
        "p            paste into this directory",
        "R            rename entry",
        "D            delete entry (confirm)",
        "+            new file, or dir if name ends with /",
        ".            toggle hidden files",
        "/ ? n N      search the listing (as in a normal buffer)",
        ": commands   run any :command",
        "g?           this help",
    ];
    std::iter::once(StyledLine { text: " dired keys".to_string(), highlights: vec![] })
        .chain(entries.iter().map(|e| StyledLine { text: format!("  {}", e), highlights: vec![] }))
        .collect()
}

/// The identifier word immediately before char offset `head`.
fn word_before(content: &str, head: usize) -> String {
    let chars: Vec<char> = content.chars().collect();
    let head = head.min(chars.len());
    let mut i = head;
    while i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') {
        i -= 1;
    }
    chars[i..head].iter().collect()
}

fn plain_lines(content: &str) -> Vec<StyledLine> {
    content
        .split('\n')
        .map(|s| StyledLine { text: s.to_string(), highlights: vec![] })
        .collect()
}

struct FrameTimer {
    last: std::time::Instant,
}

impl FrameTimer {
    fn new() -> Self {
        Self { last: std::time::Instant::now() }
    }

    fn tick(&mut self) -> Duration {
        let now = std::time::Instant::now();
        let dt = now.saturating_duration_since(self.last);
        self.last = now;
        dt
    }
}

struct CursorAnim {
    cell_x: f32,
    cell_y: f32,
}

impl CursorAnim {
    fn new() -> Self {
        Self { cell_x: 0.0, cell_y: 0.0 }
    }

    fn update(&mut self, dt: Duration, target_col: u16, target_line: u16, enabled: bool, speed: f32) {
        let dt = dt.as_secs_f32();
        let tx = target_col as f32;
        let ty = target_line as f32;

        if !enabled {
            self.cell_x = tx;
            self.cell_y = ty;
            return;
        }

        let dx = tx - self.cell_x;
        let dy = ty - self.cell_y;
        let dist = (dx * dx + dy * dy).sqrt();
        let s = speed / (1.0 + dist * 0.1);
        self.cell_x += dx * (1.0 - (-s * dt).exp());
        self.cell_y += dy * (1.0 - (-s * dt).exp());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CmdAction {
    Save(bool),
    SaveAs(String),
    Quit,
    ForceQuit,
    SaveAndQuit,
    Split(SplitDir),
    CloseWindow,
    Only,
    Fullscreen,
    Ibuffer,
    BufferDelete,
    Dired(Option<String>),
    Files,
    Rg(String),
    Rename(String),
    Format,
    WorkspaceSymbol(String),
    /// Call hierarchy: `true` = incoming/callers, `false` = outgoing/callees.
    CallHierarchy(bool),
    /// Switch editing paradigm (`:set editmode neovim|emacs`).
    SetEditMode(EditMode),
    /// `:s/pat/rep/[g]` — replace on the current line, or the whole buffer
    /// when `whole_buffer` (`:%s/...`). `all` is the `g` flag.
    Substitute {
        pattern: String,
        replacement: String,
        all: bool,
        whole_buffer: bool,
    },
}

/// Parse `s/pat/rep/flags` or `%s/pat/rep/flags` into a substitute action.
fn parse_substitute(trimmed: &str) -> Option<CmdAction> {
    let (whole_buffer, rest) = match trimmed.strip_prefix('%') {
        Some(r) => (true, r),
        None => (false, trimmed),
    };
    let rest = rest.strip_prefix('s').or_else(|| rest.strip_prefix("substitute"))?;
    let delim = rest.chars().next()?;
    if delim.is_alphanumeric() {
        return None; // e.g. ":set" must not parse as a substitution
    }
    let parts: Vec<&str> = rest[delim.len_utf8()..].split(delim).collect();
    let pattern = (*parts.first()?).to_string();
    if pattern.is_empty() {
        return None;
    }
    let replacement = parts.get(1).copied().unwrap_or("").to_string();
    let flags = parts.get(2).copied().unwrap_or("");
    Some(CmdAction::Substitute {
        pattern,
        replacement,
        all: flags.contains('g'),
        whole_buffer,
    })
}

/// The commands offered by the `:`-Tab command palette: (name, description).
const PALETTE_COMMANDS: &[(&str, &str)] = &[
    ("w", "write file"),
    ("q", "quit / close window"),
    ("wq", "write & quit"),
    ("sp", "split horizontal"),
    ("vsplit", "split vertical"),
    ("only", "close other windows"),
    ("close", "close window"),
    ("fullscreen", "toggle fullscreen"),
    ("ls", "buffer list"),
    ("bd", "delete buffer"),
    ("Dired", "file explorer"),
    ("Files", "find files"),
    ("fmt", "format buffer"),
    ("callers", "incoming calls (call hierarchy)"),
    ("callees", "outgoing calls (call hierarchy)"),
    ("set editmode emacs", "switch to Emacs (modeless) editing"),
    ("set editmode neovim", "switch to Neovim (modal) editing"),
];

/// The which-key continuations shown after a `Ctrl-w` prefix.
// --- Space-leader key tree (LazyVim style) ---

#[derive(Clone, Copy)]
enum LeaderAction {
    Focus(FocusDir),
    Split(SplitDir),
    CloseWindow,
    Only,
    Fullscreen,
    Files,
    Buffers,
    Explorer,
    Quit,
    SaveAndQuit,
    Hover,
    Definition,
    References,
    Format,
    Rename,
    DocumentSymbol,
    Diagnostics,
    IncomingCalls,
    OutgoingCalls,
}

enum LeaderNode {
    Group(&'static str, &'static [(char, LeaderNode)]),
    Action(&'static str, LeaderAction),
}

static WINDOW_GROUP: &[(char, LeaderNode)] = &[
    ('h', LeaderNode::Action("focus left", LeaderAction::Focus(FocusDir::Left))),
    ('j', LeaderNode::Action("focus down", LeaderAction::Focus(FocusDir::Down))),
    ('k', LeaderNode::Action("focus up", LeaderAction::Focus(FocusDir::Up))),
    ('l', LeaderNode::Action("focus right", LeaderAction::Focus(FocusDir::Right))),
    ('s', LeaderNode::Action("split below", LeaderAction::Split(SplitDir::Horizontal))),
    ('v', LeaderNode::Action("split right", LeaderAction::Split(SplitDir::Vertical))),
    ('c', LeaderNode::Action("close window", LeaderAction::CloseWindow)),
    ('q', LeaderNode::Action("close window", LeaderAction::CloseWindow)),
    ('o', LeaderNode::Action("only (close others)", LeaderAction::Only)),
    ('z', LeaderNode::Action("fullscreen", LeaderAction::Fullscreen)),
];

static FIND_GROUP: &[(char, LeaderNode)] = &[
    ('f', LeaderNode::Action("files", LeaderAction::Files)),
    ('b', LeaderNode::Action("buffers", LeaderAction::Buffers)),
    ('e', LeaderNode::Action("explorer (dired)", LeaderAction::Explorer)),
];

static QUIT_GROUP: &[(char, LeaderNode)] = &[
    ('q', LeaderNode::Action("quit", LeaderAction::Quit)),
    ('w', LeaderNode::Action("save and quit", LeaderAction::SaveAndQuit)),
];

static CODE_GROUP: &[(char, LeaderNode)] = &[
    ('k', LeaderNode::Action("hover", LeaderAction::Hover)),
    ('g', LeaderNode::Action("go to definition", LeaderAction::Definition)),
    ('r', LeaderNode::Action("references", LeaderAction::References)),
    ('f', LeaderNode::Action("format", LeaderAction::Format)),
    ('n', LeaderNode::Action("rename", LeaderAction::Rename)),
    ('o', LeaderNode::Action("document symbols", LeaderAction::DocumentSymbol)),
    ('d', LeaderNode::Action("diagnostics", LeaderAction::Diagnostics)),
    ('i', LeaderNode::Action("incoming calls", LeaderAction::IncomingCalls)),
    ('y', LeaderNode::Action("outgoing calls", LeaderAction::OutgoingCalls)),
];

static LEADER_ROOT: &[(char, LeaderNode)] = &[
    ('w', LeaderNode::Group("windows", WINDOW_GROUP)),
    ('f', LeaderNode::Group("find", FIND_GROUP)),
    ('c', LeaderNode::Group("code", CODE_GROUP)),
    ('q', LeaderNode::Group("quit", QUIT_GROUP)),
];

enum LeaderResolve {
    Group,
    Action(LeaderAction),
    Unknown,
}

/// What to do with the response to an LSP request we sent.
#[derive(Clone, Copy)]
enum LspAction {
    Hover,
    Definition,
    References,
    Format,
    Rename,
    DocumentSymbol,
    WorkspaceSymbol,
    /// Step 1 of call hierarchy resolved an item; fire the calls request in the
    /// given direction (`true` = incoming/callers, `false` = outgoing/callees).
    PrepareCallHierarchy(bool),
    /// Step 2 returned the calls; `true` = incoming.
    CallHierarchy(bool),
}

/// Per-buffer LSP document state (registered with `didOpen`).
struct LspDoc {
    uri: String,
    lang: String,
    version: i64,
    /// Last text synced to the server, to detect changes for `didChange`.
    synced: String,
}

/// An in-progress dired file operation awaiting input in the mini-buffer.
enum DiredPromptKind {
    /// Unified create: a trailing `/` makes a directory, otherwise a file.
    Create,
    Rename(String),
    Delete(PathBuf),
}

struct DiredPrompt {
    kind: DiredPromptKind,
    input: String,
}

fn dired_prompt_display(p: &DiredPrompt) -> String {
    match &p.kind {
        DiredPromptKind::Create => format!("Create (end with / for dir): {}", p.input),
        DiredPromptKind::Rename(old) => format!("Rename '{}' to: {}", old, p.input),
        DiredPromptKind::Delete(path) => format!(
            "Delete '{}'? (y/n)",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("")
        ),
    }
}

/// Resolve a full leader sequence: is it a group prefix, a complete action, or
/// an unknown/invalid path?
fn leader_resolve(seq: &[char]) -> LeaderResolve {
    let mut children = LEADER_ROOT;
    for (i, &c) in seq.iter().enumerate() {
        match children.iter().find(|(k, _)| *k == c).map(|(_, n)| n) {
            None => return LeaderResolve::Unknown,
            Some(LeaderNode::Group(_, ch)) => {
                if i + 1 == seq.len() {
                    return LeaderResolve::Group;
                }
                children = ch;
            }
            Some(LeaderNode::Action(_, a)) => {
                if i + 1 == seq.len() {
                    return LeaderResolve::Action(*a);
                }
                return LeaderResolve::Unknown;
            }
        }
    }
    LeaderResolve::Group // empty sequence = the root group
}

/// The children available at the node reached by `seq` (for which-key display).
fn leader_children(seq: &[char]) -> Option<&'static [(char, LeaderNode)]> {
    let mut children = LEADER_ROOT;
    for &c in seq {
        match children.iter().find(|(k, _)| *k == c).map(|(_, n)| n)? {
            LeaderNode::Group(_, ch) => children = ch,
            LeaderNode::Action(..) => return None,
        }
    }
    Some(children)
}

/// Build the which-key content (title, formatted rows) for the current pending
/// leader sequence, for the bottom sliding panel.
fn leader_whichkey(seq: &[char]) -> Option<(String, Vec<String>)> {
    let children = leader_children(seq)?;
    let mut title = String::from("SPC");
    for c in seq {
        title.push(' ');
        title.push(*c);
    }
    let rows = children
        .iter()
        .map(|(k, node)| {
            let desc = match node {
                LeaderNode::Group(d, _) => format!("+{}", d),
                LeaderNode::Action(d, _) => d.to_string(),
            };
            format!("{}  {}", k, desc)
        })
        .collect();
    Some((title, rows))
}

/// Parse one `rg --vimgrep` line (`file:line:col:text`) into its parts.
///
/// On Windows, `file` can carry a `C:\...` drive prefix whose colon would
/// otherwise be mistaken for the `line` separator, so peel that prefix off
/// before splitting and re-attach it to the parsed file path.
fn parse_rg_line(line: &str) -> Option<(PathBuf, usize, usize, String)> {
    let (drive, rest) = split_drive_prefix(line);
    let mut parts = rest.splitn(4, ':');
    let file = parts.next()?;
    let ln: usize = parts.next()?.parse().ok()?;
    let col: usize = parts.next()?.parse().ok()?;
    let text = parts.next().unwrap_or("").to_string();
    if file.is_empty() {
        return None;
    }
    let full_file = format!("{drive}{file}");
    Some((PathBuf::from(full_file), ln, col, text))
}

/// Split a leading Windows drive prefix (e.g. `C:\` or `C:/`) off the front of a
/// `rg` line. Returns the prefix (`""` when absent) and the remainder.
fn split_drive_prefix(line: &str) -> (&str, &str) {
    let bytes = line.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        line.split_at(2)
    } else {
        ("", line)
    }
}

enum AppEvent {
    Input(crossterm::event::Event),
}

pub struct App {
    pub ws: Rc<RefCell<Workspace>>,
    pub vim: VimState,
    pub renderer: Box<dyn Renderer>,
    pub should_quit: bool,
    message: Option<String>,
    /// Per-buffer tree-sitter syntax engines, created lazily the first time a
    /// buffer with a supported filetype is rendered. Buffers without a supported
    /// language (or without a file path) simply have no entry and render plain.
    syntax: std::collections::HashMap<BufferId, SyntaxEngine>,
    /// Buffers we've already attempted to build a syntax engine for, so an
    /// unsupported filetype isn't retried every frame.
    syntax_tried: std::collections::HashSet<BufferId>,
    lua: LuaRuntime,
    config: Config,
    timer: FrameTimer,
    pub has_smooth_cursor: bool,
    cursor_anim: CursorAnim,
    /// True after a `Ctrl-w` prefix, awaiting a window command key.
    pending_ctrl_w: bool,
    /// The keys typed after the `Space` leader so far (None = not in a leader
    /// sequence). Drives the which-key panel and multi-level dispatch.
    leader_pending: Option<Vec<char>>,
    /// Slide progress (0..1) of the bottom which-key panel, and the last content
    /// shown (kept while it slides back down). `anim_clock` measures frame dt.
    whichkey_anim: f32,
    whichkey_cache: Option<(String, Vec<String>)>,
    anim_clock: std::time::Instant,
    /// When the current leader sequence started, so the which-key panel only
    /// pops after `Config.timeoutlen` (unless already visible).
    leader_since: Option<std::time::Instant>,
    /// Active floating picker (buffer list, file finder, ...), if any.
    picker: Option<PickerState>,
    /// Streaming results for the active picker (`:Files` walk, `:Rg` output),
    /// drained into the picker each frame. Backend-agnostic (polled in render).
    pending_results: Option<std::sync::mpsc::Receiver<PickerItem>>,
    /// Cached highlighted contents of the file currently shown in the picker
    /// preview, so it isn't re-read and re-parsed every frame.
    preview_cache: Option<(PathBuf, Vec<StyledLine>)>,
    /// Current directory for each open dired (file-explorer) buffer.
    dired_dirs: std::collections::HashMap<BufferId, PathBuf>,
    /// Colored listing lines for each dired buffer, rebuilt on refresh.
    dired_styled: std::collections::HashMap<BufferId, Vec<StyledLine>>,
    /// The entries backing each dired buffer, so the listing text, colors and
    /// cursor→entry lookup always agree.
    dired_entries: std::collections::HashMap<BufferId, Vec<ruster_core::dired::DirEntry>>,
    /// Whether dired shows dot-files (toggled with `.`).
    dired_show_hidden: bool,
    /// An in-progress dired file operation awaiting mini-buffer input.
    dired_prompt: Option<DiredPrompt>,
    /// Path awaiting paste in dired, and whether it's a cut (`true` = move).
    dired_clipboard: Option<(PathBuf, bool)>,
    /// True after the first `y` (`yy` copy) or `d` (`dd` cut).
    dired_pending_y: bool,
    dired_pending_d: bool,
    /// True after `g` in dired, awaiting `g` (top) or `?` (help).
    dired_pending_g: bool,
    /// Language server manager (one server per language).
    lsp: LspManager,
    /// Per-buffer LSP document registration state.
    lsp_docs: std::collections::HashMap<BufferId, LspDoc>,
    /// Diagnostics per buffer, from `publishDiagnostics`.
    diagnostics: std::collections::HashMap<BufferId, Vec<ruster_lsp::Diagnostic>>,
    /// Outstanding LSP requests: (lang, request id) -> what to do with the reply.
    lsp_pending: std::collections::HashMap<(String, i64), LspAction>,
    /// Hover popup contents (syntax-highlighted lines), shown until the next key.
    hover: Option<Vec<StyledLine>>,
    /// Loaded snippet definitions (built-in + `~/.config/ruster/snippets/`).
    snippets: ruster_core::snippets::SnippetSet,
    /// Remaining tabstop offsets to visit in the active snippet, via Tab.
    snippet_stops: Vec<usize>,
    /// True while a `:w` is waiting on a format response before writing.
    pending_format_save: bool,
    /// Recorded macros by register, the in-progress recording, and the pending
    /// `q`/`@` awaiting its register letter.
    macros: std::collections::HashMap<char, Vec<crossterm::event::KeyEvent>>,
    macro_recording: Option<(char, Vec<crossterm::event::KeyEvent>)>,
    pending_macro: Option<char>,
    /// Guard so a macro can't recursively replay itself.
    replaying: bool,
    /// Which editing paradigm is active (`:set editmode neovim|emacs`).
    editmode: EditMode,
    /// Emacs-mode editing state (mark, kill-ring, prefix arg).
    emacs: ruster_core::emacs::EmacsState,
    /// True after `C-x`, awaiting the second key of the prefix.
    emacs_ctrl_x: bool,
    /// Active incremental search: (query, forward). Emacs `C-s`/`C-r`.
    emacs_isearch: Option<(String, bool)>,
}

/// The active editing paradigm. Neovim is modal; Emacs is modeless.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditMode {
    Neovim,
    Emacs,
}

impl App {
    pub fn new(content: String, file_path: PathBuf) -> Self {
        let ws = Rc::new(RefCell::new(Workspace::from_file(file_path.clone(), content.clone())));
        ws.borrow_mut().execute(Action::Move(Motion::To(0)));
        let initial_buffer = ws.borrow().active_buffer();
        let vim = VimState::new();
        let renderer = Box::new(TuiRenderer::dummy());
        let ext = ruster_syntax::lang_ext_for_path(&file_path);
        let ext = ext.as_str();
        let mut syntax: std::collections::HashMap<BufferId, SyntaxEngine> =
            std::collections::HashMap::new();
        // Highlight the LF-normalized buffer text, not the raw file bytes, so
        // CRLF files don't desync syntax spans from what's rendered.
        let normalized = ws.borrow().buffer().to_string();
        if let Ok(engine) = SyntaxEngine::new(&normalized, ext) {
            syntax.insert(initial_buffer, engine);
        }
        let mut syntax_tried = std::collections::HashSet::new();
        syntax_tried.insert(initial_buffer);
        let mut lua = LuaRuntime::new().unwrap_or_else(|e| {
            eprintln!("Lua init failed: {}", e);
            panic!("Lua init required");
        });
        // `dirs::config_dir()` resolves %APPDATA% on Windows and ~/.config on
        // Unix; if it can't be determined, just skip loading user config.
        if let Some(config_dir) = dirs::config_dir() {
            let config_path = config_dir.join("ruster").join("init.lua");
            if config_path.exists() {
                if let Err(e) = lua.load_init(&config_path) {
                    eprintln!("Lua config: {}", e);
                }
            }
        }

        // Wire buffer callbacks to the active window/document.
        let ws_get = ws.clone();
        let ws_set = ws.clone();
        let ws_get_cursor = ws.clone();
        let ws_set_cursor = ws.clone();
        lua.set_buffer_callbacks(
            Box::new(move |start, end_opt| {
                let w = ws_get.borrow();
                let buf = w.buffer();
                let count = buf.line_count() as i32;
                let end = end_opt.unwrap_or_else(|| start + 1);
                let end = if end == -1 { count } else { end.min(count) };
                (start..end).map(|i| buf.line_to_string(i as usize)).collect()
            }),
            Box::new(move |start, end, lines_vec| {
                let line_count = {
                    let w = ws_set.borrow();
                    w.buffer().line_count()
                };
                let end = (end as usize).min(line_count.saturating_sub(1));
                let (char_start, char_end) = {
                    let w = ws_set.borrow();
                    let buf = w.buffer();
                    let cs = buf.line_start_char(start as usize);
                    let ce = if end + 1 >= line_count { buf.len_chars() }
                             else { buf.line_start_char(end + 1) };
                    (cs, ce)
                };
                let mut w = ws_set.borrow_mut();
                w.execute(Action::BeginBatch);
                w.execute(Action::Edit(EditOp::DeleteRange(char_start, char_end)));
                let text = lines_vec.join("\n");
                if !text.is_empty() {
                    w.execute(Action::Edit(EditOp::InsertString(text)));
                }
                w.execute(Action::EndBatch);
            }),
            Box::new(move || {
                let w = ws_get_cursor.borrow();
                let head = w.primary_head();
                let buf = w.buffer();
                let row = buf.char_to_line(head);
                let col = head - buf.line_start_char(row);
                (row as i32, col as i32)
            }),
            Box::new(move |row, col| {
                let pos = {
                    let w = ws_set_cursor.borrow();
                    w.buffer().line_start_char(row as usize) + col as usize
                };
                ws_set_cursor.borrow_mut().execute(Action::Move(Motion::To(pos)));
            }),
        );

        // Window/buffer manipulation callbacks for the Lua API.
        {
            let ws_lb = ws.clone();
            let ws_lw = ws.clone();
            let ws_cw = ws.clone();
            let ws_scw = ws.clone();
            let ws_wgb = ws.clone();
            let ws_wsb = ws.clone();
            let ws_ow = ws.clone();
            let ws_cl = ws.clone();
            lua.set_window_callbacks(ruster_lua::WindowCallbacks {
                list_bufs: Box::new(move || {
                    ws_lb.borrow().buffers.ids().iter().map(|id| id.0 as i32).collect()
                }),
                list_wins: Box::new(move || {
                    let w = ws_lw.borrow();
                    w.windows
                        .compute_rects(CoreRect::new(0, 0, 1000, 1000))
                        .into_iter()
                        .map(|(id, _)| id.0 as i32)
                        .collect()
                }),
                current_win: Box::new(move || ws_cw.borrow().windows.active().0 as i32),
                set_current_win: Box::new(move |_id| {
                    // Focus-by-id is not exposed on WindowTree yet; no-op for now.
                    let _ = &ws_scw;
                }),
                win_get_buf: Box::new(move |win| {
                    let w = ws_wgb.borrow();
                    w.windows
                        .window(ruster_core::windows::WindowId(win as u32))
                        .map(|win| win.buffer.0 as i32)
                        .unwrap_or(0)
                }),
                win_set_buf: Box::new(move |win, buf| {
                    let mut w = ws_wsb.borrow_mut();
                    if w.buffers.get(ruster_core::document::BufferId(buf as u32)).is_some() {
                        if let Some(win) = w.windows.window_mut(ruster_core::windows::WindowId(win as u32)) {
                            win.buffer = ruster_core::document::BufferId(buf as u32);
                        }
                    }
                }),
                open_win: Box::new(move |vertical| {
                    let dir = if vertical { SplitDir::Vertical } else { SplitDir::Horizontal };
                    ws_ow.borrow_mut().windows.split(dir).0 as i32
                }),
                close_win: Box::new(move |_id| {
                    // Close the active window (id-targeted close is a follow-up).
                    ws_cl.borrow_mut().windows.close_active();
                }),
            });
        }

        lua.fire_event("VimEnter", &[]);
        // Apply Lua LSP server overrides (ruster.lsp.servers).
        let mut lsp = LspManager::new();
        for (lang, cmd, args) in lua.lsp_servers() {
            lsp.set_server(&lang, ruster_lsp::ServerConfig { cmd, args });
        }
        let mut config = lua.config();
        // Apply EditorConfig overrides
        let ec_props = ruster_core::editorconfig::parse(&file_path);
        if let Some(val) = ec_props.get("indent_style") {
            config.expandtab = *val != "tab";
        }
        if let Some(val) = ec_props.get("indent_size") {
            if let Ok(n) = val.parse::<u32>() {
                config.tabstop = n;
            }
        }
        if let Some(val) = ec_props.get("tab_width") {
            if let Ok(n) = val.parse::<u32>() {
                config.tabstop = n;
            }
        }
        ws.borrow_mut().set_active_indent_width(config.tabstop);
        let timer = FrameTimer::new();
        let cursor_anim = CursorAnim::new();
        App {
            ws, vim, renderer,
            should_quit: false, message: None, syntax, syntax_tried, lua, config, timer,
            has_smooth_cursor: false, cursor_anim, pending_ctrl_w: false, picker: None,
            leader_pending: None,
            pending_results: None,
            preview_cache: None,
            whichkey_anim: 0.0,
            whichkey_cache: None,
            anim_clock: std::time::Instant::now(),
            leader_since: None,
            dired_dirs: std::collections::HashMap::new(),
            dired_styled: std::collections::HashMap::new(),
            dired_entries: std::collections::HashMap::new(),
            dired_show_hidden: false,
            dired_prompt: None,
            dired_clipboard: None,
            dired_pending_y: false,
            dired_pending_d: false,
            dired_pending_g: false,
            lsp,
            lsp_docs: std::collections::HashMap::new(),
            diagnostics: std::collections::HashMap::new(),
            lsp_pending: std::collections::HashMap::new(),
            hover: None,
            snippets: {
                let mut s = ruster_core::snippets::SnippetSet::builtin();
                if let Some(dir) = dirs::config_dir() {
                    s.load_dir(&dir.join("ruster").join("snippets"));
                }
                s
            },
            snippet_stops: Vec::new(),
            pending_format_save: false,
            macros: std::collections::HashMap::new(),
            macro_recording: None,
            pending_macro: None,
            replaying: false,
            editmode: EditMode::Neovim,
            emacs: ruster_core::emacs::EmacsState::new(),
            emacs_ctrl_x: false,
            emacs_isearch: None,
        }
    }

    pub fn handle_key(&mut self, ck: crossterm::event::KeyEvent) {
        // Windows consoles report a Release (and Repeat) event for every key,
        // where Unix only reports Press. Acting on Release double-processes each
        // keystroke, so ignore anything that isn't a press/repeat.
        if ck.kind == KeyEventKind::Release {
            return;
        }
        // An open picker captures all input until it is accepted or cancelled.
        if self.picker.is_some() {
            self.handle_picker_key(ck);
            return;
        }

        // Any key dismisses an open hover popup (and still acts).
        self.hover = None;

        // A dired file-operation prompt captures input until confirmed/cancelled.
        if self.dired_prompt.is_some() {
            self.handle_dired_prompt_key(ck);
            return;
        }

        // A pending Space-leader sequence captures the next key.
        if self.leader_pending.is_some() {
            self.handle_leader_key(ck);
            return;
        }

        // Dired claims its action keys, but only while at rest — never while a
        // command-line/search prompt (vim Cmdline or an Emacs isearch) is open,
        // or a search term containing a dired key (e.g. `d` in "docs") would be
        // hijacked. Unclaimed keys fall through to normal handling.
        if self.active_is_dired()
            && self.vim.mode == VimMode::Normal
            && self.emacs_isearch.is_none()
            && self.handle_dired_key(ck)
        {
            return;
        }

        // Emacs is modeless: everything else routes to its own handler.
        if self.editmode == EditMode::Emacs {
            self.handle_key_emacs(ck);
            return;
        }

        // Window-command prefix (Ctrl-w) state machine takes priority.
        if self.pending_ctrl_w {
            self.pending_ctrl_w = false;
            self.handle_window_command(ck);
            return;
        }
        if self.vim.mode == VimMode::Normal
            && ck.code == KeyCode::Char('w')
            && ck.modifiers.contains(KeyModifiers::CONTROL)
        {
            self.pending_ctrl_w = true;
            return;
        }
        // Space starts the leader sequence (LazyVim-style), showing which-key.
        if self.vim.mode == VimMode::Normal
            && ck.code == KeyCode::Char(' ')
            && !ck.modifiers.contains(KeyModifiers::CONTROL)
        {
            self.leader_pending = Some(Vec::new());
            self.leader_since = Some(std::time::Instant::now());
            return;
        }
        // Direct Ctrl+h/j/k/l focus movement between splits (no Ctrl-w prefix).
        if self.vim.mode == VimMode::Normal && ck.modifiers.contains(KeyModifiers::CONTROL) {
            let dir = match ck.code {
                KeyCode::Char('h') => Some(FocusDir::Left),
                KeyCode::Char('j') => Some(FocusDir::Down),
                KeyCode::Char('k') => Some(FocusDir::Up),
                KeyCode::Char('l') => Some(FocusDir::Right),
                _ => None,
            };
            if let Some(dir) = dir {
                self.ws.borrow_mut().windows.focus(dir);
                return;
            }
        }
        // K → LSP hover (like vim's keyword lookup).
        if self.vim.mode == VimMode::Normal
            && ck.code == KeyCode::Char('K')
            && !ck.modifiers.contains(KeyModifiers::CONTROL)
        {
            self.lsp_hover();
            return;
        }

        // Macro recording (`q{reg}` … `q`) and playback (`@{reg}`).
        if self.vim.mode == VimMode::Normal && !ck.modifiers.contains(KeyModifiers::CONTROL) {
            if let Some(kind) = self.pending_macro.take() {
                if let KeyCode::Char(reg) = ck.code {
                    if kind == 'q' {
                        self.macro_recording = Some((reg, Vec::new()));
                        self.message = Some(format!("Recording @{}", reg));
                    } else {
                        self.replay_macro(reg);
                    }
                }
                return;
            }
            match ck.code {
                KeyCode::Char('q') => {
                    if let Some((reg, keys)) = self.macro_recording.take() {
                        let n = keys.len();
                        self.macros.insert(reg, keys);
                        self.message = Some(format!("Recorded @{} ({} keys)", reg, n));
                    } else {
                        self.pending_macro = Some('q');
                    }
                    return;
                }
                KeyCode::Char('@') => {
                    self.pending_macro = Some('@');
                    return;
                }
                _ => {}
            }
        }
        // Everything past this point is part of a recording.
        if let Some((_, keys)) = self.macro_recording.as_mut() {
            keys.push(ck);
        }

        let prev_mode = self.vim.mode;
        let mode = match prev_mode {
            VimMode::Normal => "n",
            VimMode::Insert => "i",
            VimMode::VisualChar | VimMode::VisualLine | VimMode::VisualBlock => "v",
            VimMode::Cmdline => "x",
        };
        if self.lua.handle_key(mode, &ck) {
            return;
        }
        let key = crossterm_to_ruster_key(ck);

        // Tab in the cmdline opens the command palette, seeded with the partial.
        if self.vim.mode == VimMode::Cmdline && key == KeyEvent::Tab {
            let seed = self
                .vim
                .cmdline_buffer()
                .trim_start_matches(':')
                .trim()
                .to_string();
            self.vim.mode = VimMode::Normal;
            self.open_command_picker(&seed);
            return;
        }

        if self.vim.mode == VimMode::Insert && key == KeyEvent::Tab {
            // 1) Cycle to the next tabstop of an active snippet.
            if !self.snippet_stops.is_empty() {
                let next = self.snippet_stops.remove(0);
                self.ws.borrow_mut().execute(Action::Move(Motion::To(next)));
                return;
            }
            // 2) Expand a snippet trigger before the cursor.
            if self.try_snippet_expand() {
                return;
            }
            // 3) Otherwise insert indentation.
            if self.config.expandtab {
                let spaces = " ".repeat(self.config.tabstop as usize);
                let mut w = self.ws.borrow_mut();
                w.execute(Action::BeginBatch);
                w.execute(Action::Edit(EditOp::InsertString(spaces)));
                w.execute(Action::EndBatch);
            }
            return;
        }
        // Any other key ends snippet-stop cycling (offsets would go stale).
        self.snippet_stops.clear();

        let actions = self.vim.handle(key, &*self.ws.borrow());
        for action in actions {
            match action {
                Action::Textobject { op, kind, target, count: _ } => {
                    let (cursor, active) = {
                        let w = self.ws.borrow();
                        (w.primary_head(), w.active_buffer())
                    };
                    if let Some((start, end)) = self.syntax.get(&active)
                        .and_then(|s| s.ts_textobject(kind, target, cursor))
                    {
                        self.exec_operator(op, start, end);
                    }
                }
                Action::CmdlineResult(cmd) => {
                    self.message = None;
                    match self.parse_cmdline(&cmd) {
                        Ok(a) => self.apply_cmd(a),
                        Err(e) => self.message = Some(e),
                    }
                }
                other => self.ws.borrow_mut().execute(other),
            }
        }
        if self.vim.mode != prev_mode {
            let mode_str = format!("{:?}", self.vim.mode);
            self.lua.set_mode(&mode_str);
            self.lua.fire_event_str("ModeChanged", &[&mode_str]);
        }
    }

    /// Switch editing paradigm, resetting per-mode state and notifying Lua.
    fn set_editmode(&mut self, mode: EditMode) {
        self.editmode = mode;
        self.emacs_ctrl_x = false;
        self.emacs_isearch = None;
        self.emacs.cancel();
        // Leave the vim layer in a clean Normal state so a later switch back
        // doesn't resume a half-finished operator or visual selection.
        self.vim = VimState::new();
        let name = match mode {
            EditMode::Neovim => "neovim",
            EditMode::Emacs => "emacs",
        };
        self.lua.set_editmode(name);
        self.message = Some(format!("editmode: {}", name));
    }

    /// Handle a key in Emacs (modeless) mode. App-level chords — the `C-x`
    /// prefix, `M-x`, isearch, `C-g` — are dealt with here; pure editing is
    /// delegated to [`ruster_core::emacs::EmacsState`], mirroring how the vim
    /// path intercepts the leader/`Ctrl-w` prefixes before delegating.
    fn handle_key_emacs(&mut self, ck: crossterm::event::KeyEvent) {
        use crossterm::event::KeyModifiers as KM;

        // An active incremental search captures keys until it ends.
        if self.emacs_isearch.is_some() {
            self.handle_isearch_key(ck);
            return;
        }

        let ctrl = ck.modifiers.contains(KM::CONTROL);

        // Second key of a `C-x` prefix.
        if self.emacs_ctrl_x {
            self.emacs_ctrl_x = false;
            match ck.code {
                KeyCode::Char('s') if ctrl => {
                    self.write_active_file(false);
                }
                KeyCode::Char('c') if ctrl => self.should_quit = true,
                KeyCode::Char('f') if ctrl => self.open_files_picker(),
                KeyCode::Char('b') if ctrl => self.open_ibuffer(),
                KeyCode::Char('u') => self.ws.borrow_mut().execute(Action::Undo),
                KeyCode::Char('0') => {
                    self.ws.borrow_mut().windows.close_active();
                }
                KeyCode::Char('1') => self.ws.borrow_mut().windows.only(),
                KeyCode::Char('2') => self.ws.borrow_mut().split(SplitDir::Horizontal),
                KeyCode::Char('3') => self.ws.borrow_mut().split(SplitDir::Vertical),
                _ => self.message = Some("C-x undefined".to_string()),
            }
            return;
        }

        let key = crossterm_to_ruster_key(ck);
        match key {
            KeyEvent::Ctrl('x') => {
                self.emacs_ctrl_x = true;
                return;
            }
            KeyEvent::Ctrl('g') => {
                self.emacs.cancel();
                self.message = Some("Quit".to_string());
                return;
            }
            KeyEvent::Alt('x') => {
                // M-x: run a command via the palette.
                self.open_command_picker("");
                return;
            }
            KeyEvent::Ctrl('s') => {
                self.start_isearch(true);
                return;
            }
            KeyEvent::Ctrl('r') => {
                self.start_isearch(false);
                return;
            }
            _ => {}
        }

        let actions = self.emacs.handle(key, &*self.ws.borrow());
        for action in actions {
            self.ws.borrow_mut().execute(action);
        }
        self.message = None;
    }

    /// Begin an Emacs incremental search in the given direction.
    fn start_isearch(&mut self, forward: bool) {
        self.emacs_isearch = Some((String::new(), forward));
        self.message = Some(if forward { "I-search: ".into() } else { "I-search backward: ".into() });
    }

    /// Drive an active incremental search: printable keys extend the query and
    /// jump to the next match; `C-s`/`C-r` repeat; `Enter`/`C-g`/`Esc` end it.
    fn handle_isearch_key(&mut self, ck: crossterm::event::KeyEvent) {
        let (mut query, mut forward) = self.emacs_isearch.take().unwrap();
        match ck.code {
            KeyCode::Enter | KeyCode::Esc => {
                self.message = None;
                return;
            }
            KeyCode::Backspace => {
                query.pop();
            }
            KeyCode::Char('s') if ck.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                forward = true;
                self.isearch_step(&query, true, true);
                self.emacs_isearch = Some((query.clone(), forward));
                self.set_isearch_message(&query, forward);
                return;
            }
            KeyCode::Char('r') if ck.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                forward = false;
                self.isearch_step(&query, false, true);
                self.emacs_isearch = Some((query.clone(), forward));
                self.set_isearch_message(&query, forward);
                return;
            }
            KeyCode::Char('g') if ck.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                self.message = None;
                return;
            }
            KeyCode::Char(c) => {
                query.push(c);
            }
            _ => {}
        }
        // Search from the current point for the (possibly extended) query.
        self.isearch_step(&query, forward, false);
        self.set_isearch_message(&query, forward);
        self.emacs_isearch = Some((query, forward));
    }

    fn set_isearch_message(&mut self, query: &str, forward: bool) {
        let label = if forward { "I-search" } else { "I-search backward" };
        self.message = Some(format!("{}: {}", label, query));
    }

    /// Move the cursor to the next/previous occurrence of `query`. `advance`
    /// starts the scan one char past point so repeated `C-s` walks matches.
    fn isearch_step(&mut self, query: &str, forward: bool, advance: bool) {
        if query.is_empty() {
            return;
        }
        let (text, head) = {
            let w = self.ws.borrow();
            (w.buffer().to_string(), w.primary_head())
        };
        // Work in char offsets to match the rest of the editor.
        let chars: Vec<char> = text.chars().collect();
        let pat: Vec<char> = query.chars().collect();
        let found = if forward {
            let from = if advance { head + 1 } else { head };
            (from..=chars.len().saturating_sub(pat.len()))
                .find(|&i| chars[i..].starts_with(&pat))
        } else {
            let start = if advance { head.saturating_sub(1) } else { head };
            (0..=start.min(chars.len().saturating_sub(pat.len())))
                .rev()
                .find(|&i| chars[i..].starts_with(&pat))
        };
        // On a miss, keep point; the message still shows the query.
        if let Some(i) = found {
            self.ws.borrow_mut().execute(Action::Move(Motion::To(i)));
        }
    }

    pub fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        require_terminal()?;
        crossterm::terminal::enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen)?;
        self.renderer = Box::new(TuiRenderer::new()?);

        loop {
            let dt = self.timer.tick();
            let secs = dt.as_secs_f64();
            self.lua.set_frame_dt(secs);
            if self.has_smooth_cursor {
                let (line, col) = self.cursor_line_col();
                self.cursor_anim.update(dt, col, line, self.config.cursor_anim_enabled, self.config.cursor_anim_speed);
            }
            self.render();
            if self.should_quit { break; }
            let ev = crossterm::event::read()?;
            let ck = match ev {
                crossterm::event::Event::Key(k) => k,
                _ => continue,
            };
            self.handle_key(ck);
        }

        self.lsp.shutdown_all();
        crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen)?;
        crossterm::terminal::disable_raw_mode()?;
        Ok(())
    }

    pub fn run_async(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        require_terminal()?;
        crossterm::terminal::enable_raw_mode()?;
        crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen)?;
        self.renderer = Box::new(TuiRenderer::new()?);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()?;

        let result = rt.block_on(self.async_run());

        // Kill language servers, and detach the runtime without waiting for the
        // blocking stdin reader (which is parked in event::read()) — otherwise
        // dropping the runtime hangs on exit.
        self.lsp.shutdown_all();
        rt.shutdown_background();

        crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen)?;
        crossterm::terminal::disable_raw_mode()?;
        result
    }

    async fn async_run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        // Spawn blocking reader
        let tx_reader = tx.clone();
        tokio::task::spawn_blocking(move || {
            while let Ok(ev) = crossterm::event::read() {
                if tx_reader.send(AppEvent::Input(ev)).is_err() {
                    break;
                }
            }
        });

        let mut interval = tokio::time::interval(Duration::from_secs_f64(1.0 / 60.0));
        interval.tick().await; // discard first immediate tick

        loop {
            tokio::select! {
                event = rx.recv() => {
                    match event {
                        Some(AppEvent::Input(ev)) => {
                            if let crossterm::event::Event::Key(k) = ev { self.handle_key(k) }
                        }
                        None => break,
                    }
                }
                _ = interval.tick() => {}
            }

            // Process queued Lua actions
            for action in self.lua.drain_actions() {
                match action {
                    LuaAction::Cmd(cmd) => {
                        match self.parse_cmdline(&cmd) {
                            Ok(a) => self.apply_cmd(a),
                            Err(e) => self.message = Some(e),
                        }
                    }
                    LuaAction::Print(msg) => {
                        self.message = Some(msg);
                    }
                }
            }

            let dt = self.timer.tick();
            let secs = dt.as_secs_f64();
            self.lua.set_frame_dt(secs);

            let (line, col) = self.cursor_line_col();
            self.cursor_anim.update(dt, col, line, self.config.cursor_anim_enabled, self.config.cursor_anim_speed);

            self.render();
            if self.should_quit { break; }
        }

        Ok(())
    }

    pub fn run_gui(&mut self) {
        loop {
            let dt = self.timer.tick();
            while let Some(key) = self.renderer.poll_input() {
                self.handle_key(key);
            }
            let secs = dt.as_secs_f64();
            self.lua.set_frame_dt(secs);

            let (line, col) = self.cursor_line_col();
            self.cursor_anim.update(dt, col, line, self.config.cursor_anim_enabled, self.config.cursor_anim_speed);
            self.render();
            if self.renderer.should_close() || self.should_quit { break; }
            std::thread::sleep(Duration::from_millis(16));
        }
        self.lsp.shutdown_all();
    }

    /// Build the preview pane for the picker's selected entry: the file's
    /// highlighted contents, windowed around the target line when there is one.
    fn picker_preview(&mut self, height: usize) -> Vec<StyledLine> {
        const PREVIEW_MAX_BYTES: usize = 512 * 1024;
        let action = match self.picker.as_mut().and_then(|p| p.selected_action()) {
            Some(a) => a,
            None => return Vec::new(),
        };
        // Resolve the selection to a file (and optional 1-indexed line).
        let (path, line): (PathBuf, Option<usize>) = match action {
            PickerAction::OpenPath(p) => (p, None),
            PickerAction::OpenLocation(p, l, _) => (p, Some(l)),
            PickerAction::OpenBuffer(id) => {
                let w = self.ws.borrow();
                match w.buffers.get(id).and_then(|d| d.file_path.clone()) {
                    Some(p) => (p, None),
                    None => return Vec::new(),
                }
            }
            PickerAction::RunCmd(_) => return Vec::new(),
        };

        // Load + highlight the file, reusing the cache when the path is unchanged.
        let cached = match &self.preview_cache {
            Some((p, lines)) if *p == path => Some(lines),
            _ => None,
        };
        if cached.is_none() {
            let text = match std::fs::metadata(&path) {
                Ok(m) if m.len() as usize <= PREVIEW_MAX_BYTES => {
                    // Normalize CRLF like Document::from_file does, so a stray
                    // `\r` doesn't render as a tofu glyph in the GUI backend.
                    std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n")
                }
                _ => String::new(),
            };
            let lines = match SyntaxEngine::new(&text, &ruster_syntax::lang_ext_for_path(&path)) {
                Ok(engine) => engine.styled_lines().to_vec(),
                Err(_) => plain_lines(&text),
            };
            self.preview_cache = Some((path.clone(), lines));
        }
        let lines = match &self.preview_cache {
            Some((_, l)) => l,
            None => return Vec::new(),
        };

        // Window the content: centered on the target line, else from the top.
        let start = match line {
            Some(l) => l.saturating_sub(1).saturating_sub(height / 3),
            None => 0,
        };
        lines.iter().skip(start).take(height).cloned().collect()
    }

    /// Drain any streamed picker results (`:Files`/`:Rg`) into the open picker.
    fn drain_pending_results(&mut self) {
        if let Some(rx) = self.pending_results.take() {
            let mut still_active = true;
            match self.picker.as_mut() {
                Some(picker) => loop {
                    match rx.try_recv() {
                        Ok(item) => picker.push_item(item),
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            still_active = false;
                            break;
                        }
                    }
                },
                // The picker was closed; stop draining.
                None => still_active = false,
            }
            if still_active {
                self.pending_results = Some(rx);
            }
        }
    }

    /// Ensure every visible buffer has a syntax engine (built lazily from its
    /// file extension), then reparse the active buffer's engine.
    fn update_syntax(&mut self) {
        let (visible, active) = {
            let w = self.ws.borrow();
            let active = w.active_buffer();
            let visible: Vec<BufferId> = w
                .windows
                .compute_rects(CoreRect::new(0, 0, 1000, 1000))
                .into_iter()
                .filter_map(|(id, _)| w.windows.window(id).map(|win| win.buffer))
                .collect();
            (visible, active)
        };
        for buf in visible {
            if self.syntax.contains_key(&buf) || self.syntax_tried.contains(&buf) {
                continue;
            }
            self.syntax_tried.insert(buf);
            let (content, ext) = {
                let w = self.ws.borrow();
                match w.buffers.get(buf) {
                    Some(d) => {
                        let ext = d
                            .file_path
                            .as_ref()
                            .map(|p| ruster_syntax::lang_ext_for_path(p))
                            .unwrap_or_default();
                        (d.buffer.to_string(), ext)
                    }
                    None => continue,
                }
            };
            if let Ok(engine) = SyntaxEngine::new(&content, &ext) {
                self.syntax.insert(buf, engine);
            }
        }
        let active_content = self.ws.borrow().buffers.get(active).map(|d| d.buffer.to_string());
        if let (Some(c), Some(engine)) = (active_content.as_ref(), self.syntax.get_mut(&active)) {
            engine.reparse(c);
        }
    }

    /// Sync the active buffer to its language server (didOpen/didChange) and
    /// drain incoming LSP messages, dispatching diagnostics and responses.
    fn update_lsp(&mut self) {
        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        // Register / update the active buffer if it's a supported file.
        let active = self.ws.borrow().active_buffer();
        let info = {
            let w = self.ws.borrow();
            w.buffers.get(active).and_then(|d| {
                let path = d.file_path.clone()?;
                let ext = ruster_syntax::lang_ext_for_path(&path);
                let lang = ruster_syntax::lang_key(&ext);
                if lang.is_empty() {
                    return None;
                }
                Some((path, lang.to_string(), d.buffer.to_string()))
            })
        };
        if let Some((path, lang, text)) = info {
            if self.lsp.ensure(&lang, &root) {
                // The server needs an absolute file URI to match its index.
                let abs = std::fs::canonicalize(&path).unwrap_or_else(|_| {
                    if path.is_absolute() { path.clone() } else { root.join(&path) }
                });
                let uri = ruster_lsp::protocol::uri_from_path(&abs);
                match self.lsp_docs.get_mut(&active) {
                    None => {
                        let language_id = ruster_lsp::registry::language_id(&lang).to_string();
                        self.lsp.did_open(&lang, &uri, &language_id, 0, &text);
                        self.lsp_docs.insert(active, LspDoc { uri, lang, version: 0, synced: text });
                    }
                    Some(doc) if doc.synced != text => {
                        doc.version += 1;
                        let version = doc.version;
                        let uri = doc.uri.clone();
                        let lang = doc.lang.clone();
                        doc.synced = text.clone();
                        self.lsp.did_change(&lang, &uri, version, &text);
                    }
                    Some(_) => {}
                }
            }
        }
        // Drain and dispatch server messages.
        for routed in self.lsp.poll() {
            match routed.message {
                ServerMessage::Notification { method, params }
                    if method == "textDocument/publishDiagnostics" =>
                {
                    let (path, diags) = ruster_lsp::parse_diagnostics(&params);
                    if let Some(buf) = self.buffer_for_path(&path) {
                        self.diagnostics.insert(buf, diags);
                    }
                }
                ServerMessage::Response { id, result, .. } => {
                    if let Some(action) = self.lsp_pending.remove(&(routed.lang.clone(), id)) {
                        self.handle_lsp_response(action, result);
                    }
                }
                _ => {}
            }
        }
    }

    /// Expand a snippet whose trigger word precedes the cursor (insert mode).
    /// Returns whether an expansion happened.
    fn try_snippet_expand(&mut self) -> bool {
        let active = self.ws.borrow().active_buffer();
        let filetype = {
            let w = self.ws.borrow();
            w.buffers
                .get(active)
                .and_then(|d| d.file_path.as_ref())
                .map(|p| ruster_syntax::lang_key(&ruster_syntax::lang_ext_for_path(p)).to_string())
                .unwrap_or_default()
        };
        if filetype.is_empty() {
            return false;
        }
        let (content, head) = {
            let w = self.ws.borrow();
            (w.buffer().to_string(), w.primary_head())
        };
        let trigger = word_before(&content, head);
        if trigger.is_empty() {
            return false;
        }
        let body = match self.snippets.get(&filetype, &trigger) {
            Some(b) => b.to_string(),
            None => return false,
        };
        let exp = ruster_core::snippets::expand(&body);
        let start = head - trigger.chars().count();
        {
            let mut w = self.ws.borrow_mut();
            w.execute(Action::BeginBatch);
            w.execute(Action::Edit(EditOp::DeleteRange(start, head)));
            w.execute(Action::Edit(EditOp::InsertString(exp.text.clone())));
            w.execute(Action::EndBatch);
        }
        // Absolute tabstop offsets; visit the first now, keep the rest for Tab.
        let mut stops: Vec<usize> = exp.stops.iter().map(|s| start + s.start).collect();
        if !stops.is_empty() {
            let first = stops.remove(0);
            self.ws.borrow_mut().execute(Action::Move(Motion::To(first)));
            self.snippet_stops = stops;
        }
        true
    }

    /// A one-line summary of a diagnostic on the cursor's line, if any.
    fn current_line_diagnostic(&self) -> Option<String> {
        let active = self.ws.borrow().active_buffer();
        let diags = self.diagnostics.get(&active)?;
        let line = {
            let w = self.ws.borrow();
            w.buffer().char_to_line(w.primary_head()) as u32
        };
        diags.iter().find(|d| d.start.line == line).map(|d| {
            let sev = match d.severity {
                1 => "E",
                2 => "W",
                3 => "I",
                _ => "H",
            };
            format!("[{}] {}", sev, d.message.replace('\n', " "))
        })
    }

    /// The open buffer whose file path matches `path`, if any.
    fn buffer_for_path(&self, path: &str) -> Option<BufferId> {
        let target = std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
        let w = self.ws.borrow();
        w.buffers.ids().iter().copied().find(|&id| {
            w.buffers
                .get(id)
                .and_then(|d| d.file_path.as_ref())
                .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()) == target)
                .unwrap_or(false)
        })
    }

    /// The active buffer's (lang, uri, cursor position) for an LSP request.
    fn active_lsp_target(&self) -> Option<(String, String, LspPosition)> {
        let active = self.ws.borrow().active_buffer();
        let doc = self.lsp_docs.get(&active)?;
        let (content, head) = {
            let w = self.ws.borrow();
            let d = w.buffers.get(active)?;
            (d.buffer.to_string(), w.primary_head())
        };
        let pos = ruster_lsp::offset_to_position(&content, head);
        Some((doc.lang.clone(), doc.uri.clone(), pos))
    }

    /// Send an LSP request built from the active position and record its action.
    /// Returns whether the request was actually sent.
    fn lsp_request(&mut self, method: &str, params: serde_json::Value, action: LspAction) -> bool {
        let lang = match self.active_lsp_target() {
            Some((lang, _, _)) => lang,
            None => {
                self.message = Some("No language server for this buffer".to_string());
                return false;
            }
        };
        if let Some(id) = self.lsp.request(&lang, method, params) {
            self.lsp_pending.insert((lang, id), action);
            true
        } else {
            self.message = Some("Language server still starting…".to_string());
            false
        }
    }

    fn lsp_hover(&mut self) {
        if let Some((_, uri, pos)) = self.active_lsp_target() {
            let params = ruster_lsp::protocol::text_document_position(&uri, pos);
            self.lsp_request("textDocument/hover", params, LspAction::Hover);
        }
    }

    fn lsp_definition(&mut self) {
        if let Some((_, uri, pos)) = self.active_lsp_target() {
            let params = ruster_lsp::protocol::text_document_position(&uri, pos);
            self.lsp_request("textDocument/definition", params, LspAction::Definition);
        }
    }

    fn lsp_references(&mut self) {
        if let Some((_, uri, pos)) = self.active_lsp_target() {
            let params = ruster_lsp::protocol::references_params(&uri, pos);
            self.lsp_request("textDocument/references", params, LspAction::References);
        }
    }

    fn lsp_format(&mut self) -> bool {
        if let Some((_, uri, _)) = self.active_lsp_target() {
            let params = ruster_lsp::protocol::formatting_params(
                &uri,
                self.config.tabstop,
                self.config.expandtab,
            );
            self.lsp_request("textDocument/formatting", params, LspAction::Format)
        } else {
            false
        }
    }

    fn lsp_document_symbols(&mut self) {
        if let Some((_, uri, _)) = self.active_lsp_target() {
            let params = ruster_lsp::protocol::document_symbol_params(&uri);
            self.lsp_request("textDocument/documentSymbol", params, LspAction::DocumentSymbol);
        }
    }

    fn lsp_workspace_symbols(&mut self, query: &str) {
        if self.active_lsp_target().is_some() {
            let params = ruster_lsp::protocol::workspace_symbol_params(query);
            self.lsp_request("workspace/symbol", params, LspAction::WorkspaceSymbol);
        }
    }

    /// Kick off call hierarchy: resolve the symbol under the cursor, then (in
    /// the response handler) request its callers or callees.
    fn lsp_call_hierarchy(&mut self, incoming: bool) {
        if let Some((_, uri, pos)) = self.active_lsp_target() {
            let params = ruster_lsp::protocol::prepare_call_hierarchy_params(&uri, pos);
            self.lsp_request(
                "textDocument/prepareCallHierarchy",
                params,
                LspAction::PrepareCallHierarchy(incoming),
            );
        }
    }

    fn lsp_rename(&mut self, new_name: &str) {
        if let Some((_, uri, pos)) = self.active_lsp_target() {
            let params = ruster_lsp::protocol::rename_params(&uri, pos, new_name);
            self.lsp_request("textDocument/rename", params, LspAction::Rename);
        }
    }

    fn handle_lsp_response(&mut self, action: LspAction, result: serde_json::Value) {
        match action {
            LspAction::Hover => {
                if let Some(text) = ruster_lsp::parse_hover(&result) {
                    self.hover = Some(build_hover_lines(&text));
                } else {
                    self.message = Some("No hover info".to_string());
                }
            }
            LspAction::Definition => {
                let locs = ruster_lsp::parse_locations(&result);
                if let Some(loc) = locs.first() {
                    self.open_path(
                        std::path::Path::new(&loc.uri),
                        Some((loc.start.line as usize + 1, loc.start.character as usize + 1)),
                    );
                } else {
                    self.message = Some("No definition found".to_string());
                }
            }
            LspAction::References => {
                let locs = ruster_lsp::parse_locations(&result);
                if locs.is_empty() {
                    self.message = Some("No references".to_string());
                    return;
                }
                let items = locs
                    .into_iter()
                    .map(|loc| {
                        let line = loc.start.line as usize + 1;
                        let col = loc.start.character as usize + 1;
                        PickerItem::new(
                            format!("{}:{}:{}", loc.uri, line, col),
                            PickerAction::OpenLocation(PathBuf::from(loc.uri), line, col),
                        )
                    })
                    .collect();
                self.picker = Some(PickerState::new("References", items));
            }
            LspAction::Format => {
                let edits = ruster_lsp::parse_text_edits(&result);
                self.apply_lsp_edits_to_active(&edits);
                if self.pending_format_save {
                    self.pending_format_save = false;
                    self.write_active_file(false);
                }
            }
            LspAction::Rename => {
                let per_file = ruster_lsp::parse_workspace_edit(&result);
                for (path, edits) in per_file {
                    self.apply_lsp_edits_to_path(&path, &edits);
                }
            }
            LspAction::DocumentSymbol => {
                let syms = ruster_lsp::parse_document_symbols(&result);
                let active_uri = self
                    .active_lsp_target()
                    .map(|(_, uri, _)| uri)
                    .unwrap_or_default();
                let items = syms
                    .into_iter()
                    .map(|s| {
                        let indent = "  ".repeat(s.depth as usize);
                        let path = s
                            .uri
                            .clone()
                            .unwrap_or_else(|| active_uri.trim_start_matches("file://").to_string());
                        PickerItem::new(
                            format!("{}{}", indent, s.name),
                            PickerAction::OpenLocation(
                                PathBuf::from(path),
                                s.start.line as usize + 1,
                                s.start.character as usize + 1,
                            ),
                        )
                    })
                    .collect();
                self.picker = Some(PickerState::new("Symbols", items));
            }
            LspAction::WorkspaceSymbol => {
                let syms = ruster_lsp::parse_workspace_symbols(&result);
                let items = syms
                    .into_iter()
                    .filter_map(|s| {
                        let uri = s.uri?;
                        Some(PickerItem::new(
                            format!("{}  {}", s.name, uri),
                            PickerAction::OpenLocation(
                                PathBuf::from(uri),
                                s.start.line as usize + 1,
                                s.start.character as usize + 1,
                            ),
                        ))
                    })
                    .collect();
                self.picker = Some(PickerState::new("Workspace symbols", items));
            }
            LspAction::PrepareCallHierarchy(incoming) => {
                let items = ruster_lsp::parse_call_hierarchy_prepare(&result);
                match items.into_iter().next() {
                    Some(item) => {
                        // Step 2: request the calls for the resolved item.
                        let method = if incoming {
                            "callHierarchy/incomingCalls"
                        } else {
                            "callHierarchy/outgoingCalls"
                        };
                        let params = ruster_lsp::protocol::call_hierarchy_calls_params(&item);
                        self.lsp_request(method, params, LspAction::CallHierarchy(incoming));
                    }
                    None => self.message = Some("No call hierarchy for symbol".to_string()),
                }
            }
            LspAction::CallHierarchy(incoming) => {
                let calls = ruster_lsp::parse_call_hierarchy_calls(&result, incoming);
                if calls.is_empty() {
                    self.message = Some(
                        if incoming { "No callers" } else { "No callees" }.to_string(),
                    );
                    return;
                }
                let title = if incoming { "Callers" } else { "Callees" };
                let items = calls
                    .into_iter()
                    .map(|c| {
                        let line = c.start.line as usize + 1;
                        let col = c.start.character as usize + 1;
                        let label = match &c.detail {
                            Some(d) => format!("{}  {}", c.name, d),
                            None => c.name.clone(),
                        };
                        PickerItem::new(
                            format!("{}  {}:{}", label, c.uri, line),
                            PickerAction::OpenLocation(PathBuf::from(c.uri), line, col),
                        )
                    })
                    .collect();
                self.picker = Some(PickerState::new(title, items));
            }
        }
    }

    /// Apply LSP text edits to the active buffer, replacing its whole content.
    fn apply_lsp_edits_to_active(&mut self, edits: &[ruster_lsp::TextEdit]) {
        if edits.is_empty() {
            return;
        }
        let content = self.ws.borrow().active_doc().buffer.to_string();
        let new = ruster_lsp::apply_edits(&content, edits);
        if new != content {
            self.replace_active_content(&new);
        }
    }

    fn apply_lsp_edits_to_path(&mut self, path: &str, edits: &[ruster_lsp::TextEdit]) {
        if edits.is_empty() {
            return;
        }
        // Only apply to the active buffer for now (multi-file rename opens each
        // affected file is a follow-up); apply if the path matches the active doc.
        let active_path = self
            .ws
            .borrow()
            .active_doc()
            .file_path
            .clone();
        let matches = active_path
            .map(|p| std::fs::canonicalize(&p).ok() == std::fs::canonicalize(path).ok())
            .unwrap_or(false);
        if matches {
            self.apply_lsp_edits_to_active(edits);
        }
    }

    /// `:s/pat/rep/[g]` — substitute on the cursor's line, or the whole buffer
    /// for `:%s`. Reports how many replacements were made.
    fn substitute(&mut self, pattern: &str, replacement: &str, all: bool, whole_buffer: bool) {
        if pattern.is_empty() {
            return;
        }
        let (content, cursor_line) = {
            let w = self.ws.borrow();
            let head = w.primary_head();
            (w.buffer().to_string(), w.buffer().char_to_line(head))
        };
        let replace_in = |line: &str, count: &mut usize| -> String {
            if all {
                *count += line.matches(pattern).count();
                line.replace(pattern, replacement)
            } else if let Some(idx) = line.find(pattern) {
                *count += 1;
                let mut s = String::with_capacity(line.len());
                s.push_str(&line[..idx]);
                s.push_str(replacement);
                s.push_str(&line[idx + pattern.len()..]);
                s
            } else {
                line.to_string()
            }
        };

        let mut count = 0usize;
        let had_trailing_newline = content.ends_with('\n');
        let new: Vec<String> = content
            .split('\n')
            .enumerate()
            .map(|(i, line)| {
                if whole_buffer || i == cursor_line {
                    replace_in(line, &mut count)
                } else {
                    line.to_string()
                }
            })
            .collect();
        if count == 0 {
            self.message = Some(format!("Pattern not found: {}", pattern));
            return;
        }
        let mut text = new.join("\n");
        if had_trailing_newline && !text.ends_with('\n') {
            text.push('\n');
        }
        self.replace_active_content(&text);
        self.message = Some(format!(
            "{} substitution{}",
            count,
            if count == 1 { "" } else { "s" }
        ));
    }

    /// Replace the active buffer's entire content via a single undo batch.
    fn replace_active_content(&mut self, new: &str) {
        let len = self.ws.borrow().active_doc().buffer.len_chars();
        let mut w = self.ws.borrow_mut();
        w.execute(Action::BeginBatch);
        if len > 0 {
            w.execute(Action::Edit(EditOp::DeleteRange(0, len)));
        }
        if !new.is_empty() {
            w.execute(Action::Edit(EditOp::InsertString(new.to_string())));
        }
        w.execute(Action::EndBatch);
        w.execute(Action::Move(Motion::To(0)));
    }

    fn render(&mut self) {
        self.drain_pending_results();
        self.update_lsp();
        let (cols, rows) = self.renderer.viewport_cells();
        // Reserve a bottom row for the cmdline/message only while one is shown,
        // so the statusline sits flush at the very bottom otherwise.
        let has_cmdline =
            self.vim.mode == VimMode::Cmdline || self.message.is_some() || self.dired_prompt.is_some();
        let reserved = if has_cmdline { 1 } else { 0 };
        let buf_area = CoreRect::new(0, 0, cols, rows.saturating_sub(reserved));

        // Ensure a syntax engine exists for every visible buffer, then reparse the
        // active buffer (the only one whose text can have changed this frame).
        self.update_syntax();

        let mode = self.vim.mode;
        // In Emacs mode the statusline shows an Emacs indicator instead of the
        // vim mode label, and the cursor is always a bar (modeless insert).
        let (mode_lbl, emacs) = match self.editmode {
            EditMode::Emacs => ("-- EMACS --".to_string(), true),
            EditMode::Neovim => (crate::widgets::mode_label(&mode).to_string(), false),
        };
        let cursor_kind = if emacs {
            CursorKind::Bar
        } else {
            match mode {
                VimMode::Insert | VimMode::Cmdline => CursorKind::Bar,
                _ => CursorKind::Block,
            }
        };
        let smooth = self.has_smooth_cursor;
        let (anim_x, anim_y) = (self.cursor_anim.cell_x, self.cursor_anim.cell_y);

        // Lua-registered statusline sections (global; shown on the active window).
        let lua_left = self.lua.statusline_sections("left").join("  ");
        let lua_center = self.lua.statusline_sections("center").join("  ");
        let lua_right = self.lua.statusline_sections("right").join("  ");

        // The Emacs region (mark..point) is highlighted like a char selection.
        let emacs_mark = if self.editmode == EditMode::Emacs {
            self.emacs.mark()
        } else {
            None
        };

        let mut views: Vec<WindowView> = Vec::new();
        {
            let mut w = self.ws.borrow_mut();
            let active_id = w.windows.active();
            let rects = w.windows.compute_rects(buf_area);
            for (wid, rect) in rects {
                let is_active = wid == active_id;
                let (buf_id, head, anchor, mut scroll, extra_heads) = {
                    let win = w.windows.window(wid).expect("window exists");
                    let primary = win.cursors.primary();
                    // Heads of every cursor except the primary, for multi-cursor
                    // rendering (the active window only).
                    let extra_heads: Vec<usize> = if is_active && win.cursors.count() > 1 {
                        let p = primary.head;
                        win.cursors
                            .iter_heads()
                            .filter(|&h| h != p)
                            .collect()
                    } else {
                        Vec::new()
                    };
                    (win.buffer, primary.head, primary.anchor, win.scroll_top, extra_heads)
                };
                let (content, cline, ccol, name, line_count, selection, extra_cursors) = {
                    let doc = w.buffers.get(buf_id).expect("buffer exists");
                    let cline = doc.buffer.char_to_line(head);
                    let ccol = head - doc.buffer.line_start_char(cline);
                    // Visual-mode selection spans anchor..head (inclusive).
                    let selection = if is_active
                        && matches!(
                            mode,
                            VimMode::VisualChar | VimMode::VisualLine | VimMode::VisualBlock
                        )
                    {
                        let (lo, hi) = (anchor.min(head), anchor.max(head));
                        let sl = doc.buffer.char_to_line(lo);
                        let el = doc.buffer.char_to_line(hi);
                        Some(SelectionView {
                            start: (sl as u16, (lo - doc.buffer.line_start_char(sl)) as u16),
                            end: (el as u16, (hi - doc.buffer.line_start_char(el)) as u16),
                            kind: match mode {
                                VimMode::VisualLine => ruster_render::SelectionKind::Line,
                                VimMode::VisualBlock => ruster_render::SelectionKind::Block,
                                _ => ruster_render::SelectionKind::Char,
                            },
                        })
                    } else if is_active {
                        // Emacs region: mark..point, shown like a char selection.
                        emacs_mark.filter(|&m| m != head).map(|m| {
                            let (lo, hi) = (m.min(head), m.max(head));
                            // The point itself is exclusive, so the last covered
                            // char is hi - 1.
                            let last = hi - 1;
                            let sl = doc.buffer.char_to_line(lo);
                            let el = doc.buffer.char_to_line(last);
                            SelectionView {
                                start: (sl as u16, (lo - doc.buffer.line_start_char(sl)) as u16),
                                end: (el as u16, (last - doc.buffer.line_start_char(el)) as u16),
                                kind: ruster_render::SelectionKind::Char,
                            }
                        })
                    } else {
                        None
                    };
                    let extra_cursors: Vec<(u16, u16)> = extra_heads
                        .iter()
                        .map(|&h| {
                            let l = doc.buffer.char_to_line(h);
                            let c = h - doc.buffer.line_start_char(l);
                            (l as u16, c as u16)
                        })
                        .collect();
                    (
                        doc.buffer.to_string(),
                        cline,
                        ccol,
                        doc.name.clone(),
                        doc.buffer.line_count(),
                        selection,
                        extra_cursors,
                    )
                };
                // Keep the cursor visible within this window's text area.
                let buf_h = rect.height.saturating_sub(1) as usize;
                if buf_h > 0 {
                    if cline < scroll {
                        scroll = cline;
                    } else if cline >= scroll + buf_h {
                        scroll = cline - buf_h + 1;
                    }
                }
                if let Some(win) = w.windows.window_mut(wid) {
                    win.scroll_top = scroll;
                    // Record the geometry so half-page scrolling can use it.
                    win.height = buf_h;
                }

                let lines: Vec<StyledLine> = match self.dired_styled.get(&buf_id) {
                    // Dired listings are colored by entry type.
                    Some(styled) => styled.clone(),
                    None => match self.syntax.get(&buf_id) {
                        Some(engine) => engine.styled_lines().to_vec(),
                        None => plain_lines(&content),
                    },
                };
                let pct = ((cline + 1) * 100).checked_div(line_count).unwrap_or(100);
                let mut left = if is_active { mode_lbl.clone() } else { String::new() };
                let mut center = name;
                let mut right = format!("{}%  {},{}", pct, cline + 1, ccol + 1);
                if is_active {
                    if !lua_left.is_empty() {
                        left = if left.is_empty() { lua_left.clone() } else { format!("{}  {}", left, lua_left) };
                    }
                    if !lua_center.is_empty() {
                        center = format!("{}  {}", center, lua_center);
                    }
                    if !lua_right.is_empty() {
                        right = format!("{}  {}", lua_right, right);
                    }
                }
                let statusline = StatuslineView { left, center, right, active: is_active };
                let cursor_smooth = if is_active && smooth {
                    Some((anim_x - ccol as f32, anim_y - cline as f32))
                } else {
                    None
                };
                let gutter = ruster_render::gutter_view(
                    scroll,
                    line_count,
                    cline,
                    self.config.number,
                    self.config.relativenumber,
                    buf_h,
                );
                views.push(WindowView {
                    rect: RRect::new(rect.x, rect.y, rect.width, rect.height),
                    lines,
                    cursor: (cline as u16, ccol as u16),
                    extra_cursors,
                    cursor_kind,
                    cursor_visible: is_active,
                    cursor_smooth,
                    scroll_offset: scroll as u16,
                    gutter,
                    statusline,
                    active: is_active,
                    selection,
                });
            }
        }

        let cmdline = if let Some(p) = &self.dired_prompt {
            Some(dired_prompt_display(p))
        } else {
            match mode {
                VimMode::Cmdline => Some(crate::widgets::cmdline_label(self.vim.cmdline_buffer())),
                _ => self.message.clone().or_else(|| self.current_line_diagnostic()),
            }
        };
        let picker_view = self.picker.as_mut().map(|p| p.view()).map(|mut v| {
            // Attach a preview of the selected entry (height ≈ the picker's rows).
            let preview_height = v.rows.len().clamp(8, 24);
            v.preview = self.picker_preview(preview_height);
            v
        });

        // Animate the bottom which-key panel sliding up while a leader sequence
        // is pending, and back down after it resolves/cancels.
        let now = std::time::Instant::now();
        let dt = (now - self.anim_clock).as_secs_f32().min(0.1);
        self.anim_clock = now;
        if let Some(seq) = self.leader_pending.as_deref() {
            if let Some(content) = leader_whichkey(seq) {
                self.whichkey_cache = Some(content);
            }
        }
        // Show the panel only after timeoutlen from the leader start — but once
        // it has begun appearing, keep it up until the sequence ends.
        let past_timeout = self
            .leader_since
            .is_some_and(|t| now.duration_since(t).as_millis() as u32 >= self.config.timeoutlen);
        let show = self.leader_pending.is_some() && (self.whichkey_anim > 0.01 || past_timeout);
        let target = if show { 1.0 } else { 0.0 };
        self.whichkey_anim += (target - self.whichkey_anim) * (1.0 - (-18.0 * dt).exp());
        if self.whichkey_anim < 0.002 {
            self.whichkey_anim = 0.0;
        }
        let whichkey = if self.whichkey_anim > 0.0 {
            self.whichkey_cache.as_ref().map(|(title, rows)| WhichKeyView {
                title: title.clone(),
                rows: rows.clone(),
                anim: self.whichkey_anim,
            })
        } else {
            None
        };

        let state = FrameState {
            windows: views,
            cmdline: cmdline.as_deref(),
            message: None,
            picker: picker_view,
            whichkey,
            hover: self.hover.clone(),
        };
        self.renderer.render_frame(&state);
    }

    fn cursor_line_col(&self) -> (u16, u16) {
        let w = self.ws.borrow();
        let head = w.primary_head();
        let buf = w.buffer();
        let line = buf.char_to_line(head);
        let col = head - buf.line_start_char(line);
        (line as u16, col as u16)
    }

    fn parse_cmdline(&self, cmdline: &str) -> Result<CmdAction, String> {
        let trimmed = cmdline.trim_start_matches(':').trim();
        if trimmed.is_empty() {
            return Err("Empty command".to_string());
        }
        match trimmed {
            "q" | "quit" => Ok(CmdAction::Quit),
            "q!" => Ok(CmdAction::ForceQuit),
            "w" | "write" => Ok(CmdAction::Save(false)),
            "w!" => Ok(CmdAction::Save(true)),
            "wq" | "x" => Ok(CmdAction::SaveAndQuit),
            "sp" | "split" => Ok(CmdAction::Split(SplitDir::Horizontal)),
            "vs" | "vsp" | "vsplit" => Ok(CmdAction::Split(SplitDir::Vertical)),
            "clo" | "close" => Ok(CmdAction::CloseWindow),
            "on" | "only" => Ok(CmdAction::Only),
            "fs" | "fullscreen" => Ok(CmdAction::Fullscreen),
            "ls" | "buffers" | "ibuffer" => Ok(CmdAction::Ibuffer),
            "bd" | "bdelete" => Ok(CmdAction::BufferDelete),
            "Dired" | "dired" | "Explore" | "Ex" => Ok(CmdAction::Dired(None)),
            "Files" | "files" => Ok(CmdAction::Files),
            "fmt" | "format" => Ok(CmdAction::Format),
            "callers" | "incomingcalls" => Ok(CmdAction::CallHierarchy(true)),
            "callees" | "outgoingcalls" => Ok(CmdAction::CallHierarchy(false)),
            _ if trimmed.starts_with("w ") || trimmed.starts_with("write ") => {
                let path = trimmed.split_once(' ').map(|x| x.1).unwrap_or("").trim().to_string();
                if path.is_empty() {
                    Err("No path given".to_string())
                } else {
                    Ok(CmdAction::SaveAs(path))
                }
            }
            _ if trimmed.starts_with("Dired ") || trimmed.starts_with("dired ") => {
                let path = trimmed.split_once(' ').map(|x| x.1).unwrap_or("").trim().to_string();
                Ok(CmdAction::Dired(Some(path)))
            }
            _ if trimmed.starts_with("Rg ") || trimmed.starts_with("rg ") => {
                let pat = trimmed.split_once(' ').map(|x| x.1).unwrap_or("").trim().to_string();
                if pat.is_empty() {
                    Err("No pattern given".to_string())
                } else {
                    Ok(CmdAction::Rg(pat))
                }
            }
            _ if trimmed.starts_with("rename ") => {
                let name = trimmed.split_once(' ').map(|x| x.1).unwrap_or("").trim().to_string();
                if name.is_empty() {
                    Err("No name given".to_string())
                } else {
                    Ok(CmdAction::Rename(name))
                }
            }
            _ if trimmed.starts_with("sym ") => {
                let q = trimmed.split_once(' ').map(|x| x.1).unwrap_or("").trim().to_string();
                Ok(CmdAction::WorkspaceSymbol(q))
            }
            _ if trimmed.starts_with("set editmode") || trimmed == "set editmode" => {
                match trimmed.rsplit(' ').next().unwrap_or("") {
                    "emacs" => Ok(CmdAction::SetEditMode(EditMode::Emacs)),
                    "neovim" | "vim" | "nvim" => Ok(CmdAction::SetEditMode(EditMode::Neovim)),
                    _ => Err("Usage: :set editmode neovim|emacs".to_string()),
                }
            }
            _ if parse_substitute(trimmed).is_some() => {
                Ok(parse_substitute(trimmed).expect("checked above"))
            }
            _ => Err(format!("Unknown command: {}", cmdline)),
        }
    }

    /// Apply a parsed cmdline action. `:q` closes the active window and only
    /// quits the app when it is the last window.
    fn apply_cmd(&mut self, action: CmdAction) {
        match action {
            CmdAction::Save(force) => self.save_file(force),
            CmdAction::SaveAs(p) => self.save_as(&p),
            CmdAction::Quit => {
                let closed = {
                    let mut w = self.ws.borrow_mut();
                    if w.windows.len() > 1 {
                        w.windows.close_active()
                    } else {
                        false
                    }
                };
                if !closed {
                    self.should_quit = true;
                }
            }
            CmdAction::ForceQuit => self.should_quit = true,
            CmdAction::SaveAndQuit => {
                self.save_file(false);
                self.should_quit = true;
            }
            CmdAction::Split(dir) => {
                self.ws.borrow_mut().windows.split(dir);
            }
            CmdAction::CloseWindow => {
                let closed = self.ws.borrow_mut().windows.close_active();
                if !closed {
                    self.message = Some("E444: Cannot close last window".to_string());
                }
            }
            CmdAction::Only => self.ws.borrow_mut().windows.only(),
            CmdAction::Fullscreen => self.ws.borrow_mut().windows.toggle_fullscreen(),
            CmdAction::Ibuffer => self.open_ibuffer(),
            CmdAction::BufferDelete => self.delete_active_buffer(),
            CmdAction::Dired(arg) => self.open_dired(arg),
            CmdAction::Files => self.open_files_picker(),
            CmdAction::Rg(pattern) => self.run_rg(&pattern),
            CmdAction::Rename(name) => self.lsp_rename(&name),
            CmdAction::Format => {
                self.lsp_format();
            }
            CmdAction::WorkspaceSymbol(q) => self.lsp_workspace_symbols(&q),
            CmdAction::CallHierarchy(incoming) => self.lsp_call_hierarchy(incoming),
            CmdAction::SetEditMode(mode) => self.set_editmode(mode),
            CmdAction::Substitute { pattern, replacement, all, whole_buffer } => {
                self.substitute(&pattern, &replacement, all, whole_buffer)
            }
        }
    }

    /// Open a fuzzy file picker over the project (gitignore-aware walk). The walk
    /// runs on a background thread, streaming paths into the picker each frame so
    /// the render loop never blocks on a large repo.
    fn open_files_picker(&mut self) {
        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let (tx, rx) = std::sync::mpsc::channel();
        let walk_root = root.clone();
        std::thread::spawn(move || {
            for result in ignore::WalkBuilder::new(&walk_root).build() {
                let entry = match result {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                    let path = entry.path().to_path_buf();
                    let label = path
                        .strip_prefix(&walk_root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .into_owned();
                    if tx.send(PickerItem::new(label, PickerAction::OpenPath(path))).is_err() {
                        break; // picker closed
                    }
                }
            }
        });
        self.picker = Some(PickerState::new("Files", Vec::new()));
        self.pending_results = Some(rx);
    }

    /// Run `rg --vimgrep <pattern>`, streaming matches into a picker from a
    /// background thread. Reports a clear message when ripgrep is not installed.
    fn run_rg(&mut self, pattern: &str) {
        let mut child = match std::process::Command::new("rg")
            .arg("--vimgrep")
            .arg(pattern)
            .stdout(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => {
                self.message = Some("ripgrep (rg) not found in PATH".to_string());
                return;
            }
        };
        let stdout = match child.stdout.take() {
            Some(s) => s,
            None => {
                self.message = Some("failed to capture rg output".to_string());
                return;
            }
        };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                if let Some((path, l, c, body)) = parse_rg_line(&line) {
                    let item = PickerItem::new(
                        format!("{}:{}:{}: {}", path.display(), l, c, body),
                        PickerAction::OpenLocation(path, l, c),
                    );
                    if tx.send(item).is_err() {
                        break;
                    }
                }
            }
            let _ = child.wait();
        });
        self.picker = Some(PickerState::new(format!("Rg: {}", pattern), Vec::new()));
        self.pending_results = Some(rx);
    }

    /// Open (or switch to) a dired file-explorer buffer for `arg` (defaulting
    /// to the current working directory).
    fn open_dired(&mut self, arg: Option<String>) {
        let path = arg
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let path = path.canonicalize().unwrap_or(path);
        let id = self
            .ws
            .borrow_mut()
            .buffers
            .create_special(SpecialKind::Dired, &path.to_string_lossy());
        self.ws.borrow_mut().set_active_buffer(id);
        self.refresh_dired(id, path);
    }

    /// Reload a dired buffer's listing for `path` and reset its window cursor.
    fn refresh_dired(&mut self, id: BufferId, path: PathBuf) {
        // List once, then derive the text, colors and lookup table from it.
        let entries = ruster_core::dired::list(&path, self.dired_show_hidden);
        let text = ruster_core::dired::render_entries(&entries);
        self.dired_styled.insert(id, dired_styled_lines(&entries));
        self.dired_entries.insert(id, entries);
        {
            let mut w = self.ws.borrow_mut();
            if let Some(doc) = w.buffers.get_mut(id) {
                doc.buffer = Buffer::from_str(&text);
                doc.name = if ruster_core::dired::is_drives_view(&path) {
                    "Drives".to_string()
                } else {
                    path.to_string_lossy().into_owned()
                };
                doc.modified = false;
            }
            if w.active_buffer() == id {
                w.windows.active_window_mut().cursors = CursorSet::single(0);
                w.windows.active_window_mut().scroll_top = 0;
            }
        }
        self.dired_dirs.insert(id, path);
    }

    fn active_is_dired(&self) -> bool {
        let w = self.ws.borrow();
        matches!(w.active_doc().kind, DocKind::Special(SpecialKind::Dired))
    }

    /// Handle a key in a dired buffer. Returns true if the key was consumed
    /// (movement keys fall through to vim so j/k/gg/G still work).
    fn handle_dired_key(&mut self, ck: crossterm::event::KeyEvent) -> bool {
        let ctrl = ck.modifiers.contains(KeyModifiers::CONTROL);
        // Pending `yy` copy / `dd` cut: a matching second key completes it.
        if self.dired_pending_y {
            self.dired_pending_y = false;
            if ck.code == KeyCode::Char('y') {
                self.dired_yank_under_cursor(false);
                return true;
            }
        }
        if self.dired_pending_d {
            self.dired_pending_d = false;
            if ck.code == KeyCode::Char('d') {
                self.dired_yank_under_cursor(true);
                return true;
            }
        }
        // Pending `g`: `gg` jumps to the top, `g?` shows dired help. Handled
        // locally (rather than falling through to vim) so `?` stays free for
        // reverse-search. Any other key is a no-op that ends the prefix.
        if self.dired_pending_g {
            self.dired_pending_g = false;
            match ck.code {
                KeyCode::Char('g') => self.ws.borrow_mut().execute(Action::Move(Motion::To(0))),
                KeyCode::Char('?') => self.hover = Some(dired_help_lines()),
                _ => {}
            }
            return true;
        }
        if ctrl {
            match ck.code {
                KeyCode::Char('n') => {
                    self.ws.borrow_mut().execute(Action::Move(Motion::Line(1)));
                    return true;
                }
                KeyCode::Char('p') => {
                    self.ws.borrow_mut().execute(Action::Move(Motion::Line(-1)));
                    return true;
                }
                _ => {}
            }
        }
        match ck.code {
            KeyCode::Enter | KeyCode::Char('l') => {
                self.dired_open_at_cursor();
                true
            }
            KeyCode::Char('h') | KeyCode::Char('-') | KeyCode::Char('^') => {
                self.dired_go_up();
                true
            }
            KeyCode::Char('+') => {
                self.dired_prompt = Some(DiredPrompt { kind: DiredPromptKind::Create, input: String::new() });
                true
            }
            KeyCode::Char('R') => {
                if let Some((_, name)) = self.dired_current_target() {
                    self.dired_prompt = Some(DiredPrompt {
                        kind: DiredPromptKind::Rename(name.clone()),
                        input: name,
                    });
                }
                true
            }
            KeyCode::Char('D') => {
                if let Some((path, _)) = self.dired_current_target() {
                    self.dired_prompt = Some(DiredPrompt { kind: DiredPromptKind::Delete(path), input: String::new() });
                }
                true
            }
            KeyCode::Char('y') => {
                self.dired_pending_y = true;
                true
            }
            KeyCode::Char('d') => {
                self.dired_pending_d = true;
                true
            }
            KeyCode::Char('p') => {
                self.dired_paste();
                true
            }
            KeyCode::Char('.') => {
                self.dired_show_hidden = !self.dired_show_hidden;
                self.dired_refresh_current();
                self.message = Some(format!(
                    "Hidden files {}",
                    if self.dired_show_hidden { "shown" } else { "hidden" }
                ));
                true
            }
            // `g` starts the dired prefix (`gg` top, `g?` help).
            KeyCode::Char('g') => {
                self.dired_pending_g = true;
                true
            }
            // Everything else falls through to normal handling. The buffer is
            // read-only (edits are no-ops), so this safely enables `:` commands,
            // `/`/`?`/`n`/`N` search, motions, the Space leader, and — in Emacs
            // mode — `C-s`/`M-x`, all operating over the listing.
            _ => false,
        }
    }

    /// Record the entry under the cursor for a later paste. `cut` moves on paste.
    fn dired_yank_under_cursor(&mut self, cut: bool) {
        match self.dired_current_target() {
            Some((path, name)) => {
                self.dired_clipboard = Some((path, cut));
                self.message = Some(format!(
                    "{} '{}'",
                    if cut { "Cut" } else { "Copied" },
                    name
                ));
            }
            None => self.message = Some("Nothing selected".to_string()),
        }
    }

    /// Paste the dired clipboard into the current directory (copy, or move for a cut).
    fn dired_paste(&mut self) {
        let (src, cut) = match self.dired_clipboard.clone() {
            Some(s) => s,
            None => {
                self.message = Some("Clipboard empty".to_string());
                return;
            }
        };
        let id = self.ws.borrow().active_buffer();
        let dir = match self.dired_dirs.get(&id) {
            Some(d) => d.clone(),
            None => return,
        };
        let name = match src.file_name() {
            Some(n) => n.to_os_string(),
            None => return,
        };
        let dest = dir.join(&name);
        if dest.exists() {
            self.message = Some(format!("'{}' already exists", name.to_string_lossy()));
            return;
        }
        let result = if cut {
            // Try a rename first; fall back to copy+remove across filesystems.
            std::fs::rename(&src, &dest).or_else(|_| {
                let copied = if src.is_dir() {
                    copy_dir_recursive(&src, &dest)
                } else {
                    std::fs::copy(&src, &dest).map(|_| ())
                };
                copied.and_then(|()| {
                    if src.is_dir() {
                        std::fs::remove_dir_all(&src)
                    } else {
                        std::fs::remove_file(&src)
                    }
                })
            })
        } else if src.is_dir() {
            copy_dir_recursive(&src, &dest)
        } else {
            std::fs::copy(&src, &dest).map(|_| ())
        };
        match result {
            Ok(()) => {
                self.message = Some(format!(
                    "{} '{}'",
                    if cut { "Moved" } else { "Pasted" },
                    name.to_string_lossy()
                ));
                if cut {
                    self.dired_clipboard = None; // a cut is consumed by the paste
                }
            }
            Err(e) => self.message = Some(format!("Paste failed: {}", e)),
        }
        self.dired_refresh_current();
    }

    /// The (path, name) of the entry under the cursor in the active dired buffer,
    /// or None for `..` / an empty listing.
    fn dired_current_target(&self) -> Option<(PathBuf, String)> {
        let id = self.ws.borrow().active_buffer();
        let dir = self.dired_dirs.get(&id)?.clone();
        let line = {
            let w = self.ws.borrow();
            w.buffer().char_to_line(w.primary_head())
        };
        let entries = self.dired_entries.get(&id)?;
        let entry = entries.get(line)?;
        if entry.name == ".." {
            return None;
        }
        Some((dir.join(&entry.name), entry.name.clone()))
    }

    /// Handle a key while a dired file-operation prompt is active.
    fn handle_dired_prompt_key(&mut self, ck: crossterm::event::KeyEvent) {
        let is_delete = matches!(
            self.dired_prompt.as_ref().map(|p| &p.kind),
            Some(DiredPromptKind::Delete(_))
        );
        if is_delete {
            match ck.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    if let Some(DiredPrompt { kind: DiredPromptKind::Delete(path), .. }) =
                        self.dired_prompt.take()
                    {
                        let result = if path.is_dir() {
                            std::fs::remove_dir_all(&path)
                        } else {
                            std::fs::remove_file(&path)
                        };
                        let name = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("")
                            .to_string();
                        self.message = Some(match result {
                            Ok(()) => format!("Deleted '{}'", name),
                            Err(e) => format!("Delete failed for '{}': {}", name, e),
                        });
                    }
                    self.dired_refresh_current();
                }
                _ => self.dired_prompt = None,
            }
            return;
        }
        match ck.code {
            KeyCode::Char(c) => {
                if let Some(p) = self.dired_prompt.as_mut() {
                    p.input.push(c);
                }
            }
            KeyCode::Backspace => {
                if let Some(p) = self.dired_prompt.as_mut() {
                    p.input.pop();
                }
            }
            KeyCode::Esc => self.dired_prompt = None,
            KeyCode::Enter => {
                if let Some(prompt) = self.dired_prompt.take() {
                    self.dired_execute_prompt(prompt);
                }
            }
            _ => {}
        }
    }

    fn dired_execute_prompt(&mut self, prompt: DiredPrompt) {
        let id = self.ws.borrow().active_buffer();
        let dir = match self.dired_dirs.get(&id) {
            Some(d) => d.clone(),
            None => return,
        };
        let input = prompt.input.trim().to_string();
        match prompt.kind {
            // A trailing '/' creates a directory, otherwise a file.
            DiredPromptKind::Create if !input.is_empty() => {
                let is_dir = input.ends_with('/');
                let name = input.trim_end_matches('/').to_string();
                if name.is_empty() {
                    self.message = Some("No name given".to_string());
                } else {
                    let target = dir.join(&name);
                    if target.exists() {
                        self.message = Some(format!("'{}' already exists", name));
                    } else {
                        let result = if is_dir {
                            std::fs::create_dir_all(&target)
                        } else {
                            if let Some(parent) = target.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            std::fs::File::create(&target).map(|_| ())
                        };
                        self.message = Some(match result {
                            Ok(()) => format!(
                                "Created {} '{}'",
                                if is_dir { "directory" } else { "file" },
                                name
                            ),
                            Err(e) => format!("Create failed: {}", e),
                        });
                    }
                }
            }
            DiredPromptKind::Rename(old) if !input.is_empty() => {
                let target = dir.join(&input);
                if target.exists() {
                    self.message = Some(format!("'{}' already exists", input));
                } else if let Err(e) = std::fs::rename(dir.join(&old), &target) {
                    self.message = Some(format!("Rename failed: {}", e));
                }
            }
            _ => {}
        }
        self.dired_refresh_current();
    }

    /// Reload the active dired buffer's listing (after a mutation).
    fn dired_refresh_current(&mut self) {
        let id = self.ws.borrow().active_buffer();
        if let Some(dir) = self.dired_dirs.get(&id).cloned() {
            self.refresh_dired(id, dir);
        }
    }

    fn dired_open_at_cursor(&mut self) {
        let id = self.ws.borrow().active_buffer();
        let dir = match self.dired_dirs.get(&id) {
            Some(p) => p.clone(),
            None => return,
        };
        let line = {
            let w = self.ws.borrow();
            w.buffer().char_to_line(w.primary_head())
        };
        let entry = match self.dired_entries.get(&id).and_then(|e| e.get(line)) {
            Some(e) => e.clone(),
            None => return,
        };
        // `..` ascends (and, at a drive root, reaches the drive picker).
        if entry.name == ".." {
            self.dired_go_up();
            return;
        }
        let target = dir.join(&entry.name);
        let target = target.canonicalize().unwrap_or(target);
        if entry.is_dir {
            self.refresh_dired(id, target);
        } else {
            self.open_path(&target, None);
        }
    }

    fn dired_go_up(&mut self) {
        let id = self.ws.borrow().active_buffer();
        let dir = match self.dired_dirs.get(&id) {
            Some(d) => d.clone(),
            None => return,
        };
        if ruster_core::dired::is_drives_view(&dir) {
            return; // already at the top
        }
        let target = match dir.parent() {
            Some(parent) => parent.to_path_buf(),
            // At a drive root (e.g. C:\, whose parent is None) on Windows,
            // ascend to the drive picker instead of staying put.
            None if cfg!(windows) => ruster_core::dired::drives_view(),
            None => return,
        };
        self.refresh_dired(id, target);
    }

    /// Open the `:`-Tab command palette, pre-filtered by `seed`.
    fn open_command_picker(&mut self, seed: &str) {
        let items: Vec<PickerItem> = PALETTE_COMMANDS
            .iter()
            .map(|(name, desc)| {
                PickerItem::new(
                    format!("{:<12} {}", name, desc),
                    PickerAction::RunCmd(name.to_string()),
                )
            })
            .collect();
        let mut p = PickerState::new("Commands", items);
        for c in seed.chars() {
            p.push_char(c);
        }
        self.picker = Some(p);
    }

    /// Open the buffer-list picker over every open buffer.
    fn open_ibuffer(&mut self) {
        let items: Vec<PickerItem> = {
            let w = self.ws.borrow();
            w.buffers
                .ids()
                .iter()
                .map(|&id| {
                    let d = w.buffers.get(id).expect("buffer exists");
                    let flag = if d.modified { "[+]" } else { "   " };
                    PickerItem::new(
                        format!("{:>3} {} {}", id.0, flag, d.name),
                        PickerAction::OpenBuffer(id),
                    )
                })
                .collect()
        };
        self.picker = Some(PickerState::new("Buffers", items));
    }

    /// Delete the active buffer, switching the active window to another open
    /// buffer first. Refuses when it is the only buffer, or when it is modified.
    fn delete_active_buffer(&mut self) {
        let mut w = self.ws.borrow_mut();
        let cur = w.active_buffer();
        let other = w.buffers.ids().iter().copied().find(|&id| id != cur);
        match other {
            Some(o) => {
                if w.buffers.get(cur).map(|d| d.modified).unwrap_or(false) {
                    drop(w);
                    self.message = Some("E89: buffer modified (add ! to override)".to_string());
                    return;
                }
                w.set_active_buffer(o);
                w.buffers.close(cur);
            }
            None => {
                drop(w);
                self.message = Some("E514: cannot close last buffer".to_string());
            }
        }
    }

    /// Replay a recorded macro by feeding its keys back through `handle_key`,
    /// so each one sees the state left by the previous.
    fn replay_macro(&mut self, reg: char) {
        if self.replaying {
            return; // don't let a macro invoke itself
        }
        let keys = match self.macros.get(&reg) {
            Some(k) => k.clone(),
            None => {
                self.message = Some(format!("No macro in @{}", reg));
                return;
            }
        };
        self.replaying = true;
        for k in keys {
            self.handle_key(k);
        }
        self.replaying = false;
    }

    /// Route a key to the open picker: type to filter, arrows/Ctrl-n/p to move,
    /// Enter to accept, Esc to cancel.
    fn handle_picker_key(&mut self, ck: crossterm::event::KeyEvent) {
        let ctrl = ck.modifiers.contains(KeyModifiers::CONTROL);
        let action = {
            let picker = match self.picker.as_mut() {
                Some(p) => p,
                None => return,
            };
            match ck.code {
                KeyCode::Esc => {
                    self.picker = None;
                    return;
                }
                KeyCode::Enter => picker.accept(),
                KeyCode::Up => {
                    picker.move_selection(-1);
                    return;
                }
                KeyCode::Down => {
                    picker.move_selection(1);
                    return;
                }
                KeyCode::Char('p') if ctrl => {
                    picker.move_selection(-1);
                    return;
                }
                KeyCode::Char('n') if ctrl => {
                    picker.move_selection(1);
                    return;
                }
                KeyCode::Backspace => {
                    picker.pop_char();
                    return;
                }
                KeyCode::Char(c) if !ctrl => {
                    picker.push_char(c);
                    return;
                }
                _ => return,
            }
        };
        // Enter was pressed: close the picker and dispatch the chosen action.
        self.picker = None;
        if let Some(action) = action {
            self.dispatch_picker_action(action);
        }
    }

    fn dispatch_picker_action(&mut self, action: PickerAction) {
        match action {
            PickerAction::OpenBuffer(id) => {
                self.ws.borrow_mut().set_active_buffer(id);
            }
            PickerAction::OpenPath(path) => self.open_path(&path, None),
            PickerAction::OpenLocation(path, line, col) => {
                self.open_path(&path, Some((line, col)));
            }
            PickerAction::RunCmd(cmd) => match self.parse_cmdline(&cmd) {
                Ok(a) => self.apply_cmd(a),
                Err(e) => self.message = Some(e),
            },
        }
    }

    /// Open `path` into a buffer shown in the active window. When `at` is given,
    /// move the cursor to that 1-indexed (line, col).
    fn open_path(&mut self, path: &std::path::Path, at: Option<(usize, usize)>) {
        let content = std::fs::read_to_string(path).unwrap_or_default();
        let id = self.ws.borrow_mut().buffers.open_file(path.to_path_buf(), content);
        self.ws.borrow_mut().set_active_buffer(id);
        if let Some((line, col)) = at {
            let pos = {
                let w = self.ws.borrow();
                let buf = w.buffer();
                let l = line.saturating_sub(1).min(buf.line_count().saturating_sub(1));
                buf.line_start_char(l) + col.saturating_sub(1)
            };
            self.ws.borrow_mut().execute(Action::Move(Motion::To(pos)));
        }
    }

    /// Advance the pending Space-leader sequence with the next key.
    fn handle_leader_key(&mut self, ck: crossterm::event::KeyEvent) {
        let c = match ck.code {
            KeyCode::Char(c) => c,
            // Esc or anything else cancels the leader sequence.
            _ => {
                self.leader_pending = None;
                return;
            }
        };
        let seq = self.leader_pending.get_or_insert_with(Vec::new);
        seq.push(c);
        let snapshot = seq.clone();
        match leader_resolve(&snapshot) {
            LeaderResolve::Group => { /* keep pending; which-key shows the group */ }
            LeaderResolve::Action(action) => {
                self.leader_pending = None;
                self.apply_leader_action(action);
            }
            LeaderResolve::Unknown => {
                self.leader_pending = None;
            }
        }
    }

    fn apply_leader_action(&mut self, action: LeaderAction) {
        match action {
            LeaderAction::Focus(dir) => self.ws.borrow_mut().windows.focus(dir),
            LeaderAction::Split(dir) => {
                self.ws.borrow_mut().windows.split(dir);
            }
            LeaderAction::CloseWindow => {
                self.ws.borrow_mut().windows.close_active();
            }
            LeaderAction::Only => self.ws.borrow_mut().windows.only(),
            LeaderAction::Fullscreen => self.ws.borrow_mut().windows.toggle_fullscreen(),
            LeaderAction::Files => self.open_files_picker(),
            LeaderAction::Buffers => self.open_ibuffer(),
            LeaderAction::Explorer => self.open_dired(None),
            LeaderAction::Quit => self.should_quit = true,
            LeaderAction::SaveAndQuit => {
                self.save_file(false);
                self.should_quit = true;
            }
            LeaderAction::Hover => self.lsp_hover(),
            LeaderAction::Definition => self.lsp_definition(),
            LeaderAction::References => self.lsp_references(),
            LeaderAction::Format => {
                self.lsp_format();
            }
            LeaderAction::Rename => {
                // Seed the cmdline with :rename for the new name.
                self.message = Some("Use :rename <new-name>".to_string());
            }
            LeaderAction::DocumentSymbol => self.lsp_document_symbols(),
            LeaderAction::Diagnostics => self.open_diagnostics_picker(),
            LeaderAction::IncomingCalls => self.lsp_call_hierarchy(true),
            LeaderAction::OutgoingCalls => self.lsp_call_hierarchy(false),
        }
    }

    /// Show the active buffer's diagnostics in a picker; Enter jumps to one.
    fn open_diagnostics_picker(&mut self) {
        let active = self.ws.borrow().active_buffer();
        let path = self.ws.borrow().active_doc().file_path.clone();
        let diags = self.diagnostics.get(&active).cloned().unwrap_or_default();
        if diags.is_empty() {
            self.message = Some("No diagnostics".to_string());
            return;
        }
        let path = match path {
            Some(p) => p,
            None => return,
        };
        let items = diags
            .into_iter()
            .map(|d| {
                let sev = match d.severity {
                    1 => "E",
                    2 => "W",
                    3 => "I",
                    _ => "H",
                };
                let line = d.start.line as usize + 1;
                let col = d.start.character as usize + 1;
                PickerItem::new(
                    format!("{} {}:{}  {}", sev, line, col, d.message.replace('\n', " ")),
                    PickerAction::OpenLocation(path.clone(), line, col),
                )
            })
            .collect();
        self.picker = Some(PickerState::new("Diagnostics", items));
    }

    /// Interpret the key following a `Ctrl-w` prefix.
    fn handle_window_command(&mut self, ck: crossterm::event::KeyEvent) {
        let mut w = self.ws.borrow_mut();
        match ck.code {
            KeyCode::Char('s') => { w.windows.split(SplitDir::Horizontal); }
            KeyCode::Char('v') => { w.windows.split(SplitDir::Vertical); }
            KeyCode::Char('c') => { w.windows.close_active(); }
            KeyCode::Char('o') => w.windows.only(),
            KeyCode::Char('h') => w.windows.focus(FocusDir::Left),
            KeyCode::Char('j') => w.windows.focus(FocusDir::Down),
            KeyCode::Char('k') => w.windows.focus(FocusDir::Up),
            KeyCode::Char('l') => w.windows.focus(FocusDir::Right),
            KeyCode::Char('z') => w.windows.toggle_fullscreen(),
            _ => {}
        }
    }

    fn save_file(&mut self, force: bool) {
        // Format-on-save: format via LSP first, then write when the edits arrive.
        if self.config.format_on_save && !self.pending_format_save {
            let active = self.ws.borrow().active_buffer();
            if self.lsp_docs.contains_key(&active) {
                self.pending_format_save = true;
                if self.lsp_format() {
                    return; // write deferred until the format response
                }
                self.pending_format_save = false; // couldn't format; save now
            }
        }
        self.write_active_file(force);
    }

    fn write_active_file(&mut self, force: bool) {
        let (path, content) = {
            let w = self.ws.borrow();
            let doc = w.active_doc();
            // Preserve the file's original line ending (LF/CRLF) on write.
            (doc.file_path.clone(), doc.encode_content())
        };
        let path = match path {
            Some(p) => p,
            None => {
                self.message = Some("E32: No file name".to_string());
                return;
            }
        };
        self.lua.fire_event_str("BufWritePre", &[path.to_str().unwrap_or("")]);
        match std::fs::write(&path, &content) {
            Ok(()) => {
                self.ws.borrow_mut().active_doc_mut().modified = false;
                self.message = Some(format!("Saved: {}", path.display()));
            }
            Err(_e) if force => {
                let _ = std::fs::write(&path, &content);
                self.ws.borrow_mut().active_doc_mut().modified = false;
                self.message = Some(format!("Saved (forced): {}", path.display()));
            }
            Err(e) => self.message = Some(format!("Error: {}", e)),
        }
        self.lua.fire_event_str("BufWritePost", &[path.to_str().unwrap_or("")]);
    }

    fn exec_operator(&mut self, op: char, start: usize, end: usize) {
        let safe_end = end.min(self.ws.borrow().buffer().len_chars());
        match op {
            'd' => {
                let mut w = self.ws.borrow_mut();
                w.execute(Action::BeginBatch);
                w.execute(Action::Edit(EditOp::DeleteRange(start, safe_end)));
                w.execute(Action::EndBatch);
            }
            'c' => {
                {
                    let mut w = self.ws.borrow_mut();
                    w.execute(Action::BeginBatch);
                    w.execute(Action::Edit(EditOp::DeleteRange(start, safe_end)));
                }
                self.vim.mode = VimMode::Insert;
            }
            'y' => {
                let text = self.ws.borrow().buffer().slice_string(start, safe_end);
                self.vim.set_register(text);
            }
            _ => {}
        }
    }

    fn save_as(&mut self, path: &str) {
        let content = self.ws.borrow().active_doc().encode_content();
        match std::fs::write(path, &content) {
            Ok(()) => {
                {
                    let mut w = self.ws.borrow_mut();
                    let doc = w.active_doc_mut();
                    doc.file_path = Some(PathBuf::from(path));
                    doc.modified = false;
                }
                self.message = Some(format!("Saved: {}", path));
            }
            Err(e) => self.message = Some(format!("Error: {}", e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_w_saves() {
        let a = App::new("content".into(), PathBuf::from("f.txt"));
        assert_eq!(a.parse_cmdline(":w"), Ok(CmdAction::Save(false)));
    }

    #[test]
    fn cmd_q_quits() {
        let a = App::new("content".into(), PathBuf::from("f.txt"));
        assert_eq!(a.parse_cmdline(":q"), Ok(CmdAction::Quit));
    }

    #[test]
    fn cmd_wq_saves_and_quits() {
        let a = App::new("content".into(), PathBuf::from("f.txt"));
        assert_eq!(a.parse_cmdline(":wq"), Ok(CmdAction::SaveAndQuit));
    }

    #[test]
    fn cmd_q_force_quits() {
        let a = App::new("content".into(), PathBuf::from("f.txt"));
        assert_eq!(a.parse_cmdline(":q!"), Ok(CmdAction::ForceQuit));
    }

    #[test]
    fn cmd_w_path_saves_as() {
        let a = App::new("content".into(), PathBuf::from("f.txt"));
        assert_eq!(a.parse_cmdline(":w /tmp/out.txt"), Ok(CmdAction::SaveAs("/tmp/out.txt".into())));
    }

    #[test]
    fn cmd_unknown_errors() {
        let a = App::new("content".into(), PathBuf::from("f.txt"));
        assert!(a.parse_cmdline(":xyz").is_err());
    }

    #[test]
    fn release_key_events_are_ignored() {
        use crossterm::event::{KeyCode as CtCode, KeyEvent as CtKey, KeyEventKind, KeyModifiers as CtMods};
        let mut a = App::new("ab".into(), PathBuf::from("f.txt"));
        // Windows emits a press *and* a release for each keystroke; only the
        // press should act, otherwise every key is processed twice.
        a.handle_key(CtKey::new_with_kind(CtCode::Char('x'), CtMods::NONE, KeyEventKind::Release));
        assert_eq!(a.ws.borrow().buffer().to_string(), "ab", "release must be a no-op");
        a.handle_key(CtKey::new_with_kind(CtCode::Char('x'), CtMods::NONE, KeyEventKind::Press));
        assert_eq!(a.ws.borrow().buffer().to_string(), "b", "press deletes one char");
    }

    #[test]
    fn save_preserves_crlf_line_endings() {
        let mut path = std::env::temp_dir();
        path.push(format!("ruster_crlf_{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut a = App::new("a\r\nb\r\n".into(), path.clone());
        a.save_file(false);
        let written = std::fs::read(&path).unwrap();
        assert_eq!(written, b"a\r\nb\r\n");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_keeps_lf_line_endings() {
        let mut path = std::env::temp_dir();
        path.push(format!("ruster_lf_{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut a = App::new("a\nb\n".into(), path.clone());
        a.save_file(false);
        let written = std::fs::read(&path).unwrap();
        assert_eq!(written, b"a\nb\n");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn parse_rg_line_posix() {
        let (file, ln, col, text) = parse_rg_line("src/x.rs:1:1:hi").unwrap();
        assert_eq!(file, PathBuf::from("src/x.rs"));
        assert_eq!(ln, 1);
        assert_eq!(col, 1);
        assert_eq!(text, "hi");
    }

    #[test]
    fn parse_rg_line_windows_drive() {
        let (file, ln, col, text) = parse_rg_line(r"C:\a\b.rs:2:3:hi").unwrap();
        assert_eq!(file, PathBuf::from(r"C:\a\b.rs"));
        assert_eq!(ln, 2);
        assert_eq!(col, 3);
        assert_eq!(text, "hi");
    }

    #[test]
    fn parse_rg_line_windows_drive_forward_slash() {
        let (file, ln, col, text) = parse_rg_line("C:/a/b.rs:4:5:text:with:colons").unwrap();
        assert_eq!(file, PathBuf::from("C:/a/b.rs"));
        assert_eq!(ln, 4);
        assert_eq!(col, 5);
        assert_eq!(text, "text:with:colons");
    }

    #[test]
    fn split_yields_two_windows_sharing_buffer() {
        use ruster_core::windows::{Rect, SplitDir};
        let a = App::new("hello".into(), PathBuf::from("f.txt"));
        let buf = a.ws.borrow().active_buffer();
        a.ws.borrow_mut().split(SplitDir::Vertical);
        let w = a.ws.borrow();
        assert_eq!(w.windows.len(), 2);
        let rects = w.windows.compute_rects(Rect::new(0, 0, 80, 24));
        assert_eq!(rects.len(), 2, "two windows produce two rects");
        // Both windows view the same buffer right after the split.
        for (id, _) in rects {
            assert_eq!(w.windows.window(id).unwrap().buffer, buf);
        }
    }

    #[test]
    fn per_buffer_syntax_engines_created_lazily() {
        let mut a = App::new("fn main() {}".into(), PathBuf::from("main.rs"));
        let rust_buf = a.ws.borrow().active_buffer();
        assert!(a.syntax.contains_key(&rust_buf), "initial rust buffer has an engine");

        // Open a Python file into a new active buffer.
        let tmp = std::env::temp_dir().join("ruster_syn_test.py");
        std::fs::write(&tmp, "def f():\n    return 1\n").unwrap();
        a.open_path(&tmp, None);
        let py_buf = a.ws.borrow().active_buffer();
        assert_ne!(rust_buf, py_buf);
        a.update_syntax();
        assert!(a.syntax.contains_key(&py_buf), "python buffer gets its own engine");
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn substitute_current_line_and_whole_buffer() {
        // `:s/a/X/` replaces the first match on the cursor's line only.
        let mut a = App::new("aaa\naaa\n".into(), PathBuf::from("f.txt"));
        a.apply_cmd(a.parse_cmdline(":s/a/X/").unwrap());
        assert_eq!(a.ws.borrow().buffer().to_string(), "Xaa\naaa\n");

        // `:s/a/X/g` replaces every match on that line.
        let mut a = App::new("aaa\naaa\n".into(), PathBuf::from("f.txt"));
        a.apply_cmd(a.parse_cmdline(":s/a/X/g").unwrap());
        assert_eq!(a.ws.borrow().buffer().to_string(), "XXX\naaa\n");

        // `:%s/a/X/g` replaces throughout the buffer.
        let mut a = App::new("aaa\naaa\n".into(), PathBuf::from("f.txt"));
        a.apply_cmd(a.parse_cmdline(":%s/a/X/g").unwrap());
        assert_eq!(a.ws.borrow().buffer().to_string(), "XXX\nXXX\n");
    }

    #[test]
    fn substitute_reports_when_pattern_is_missing() {
        let mut a = App::new("hello\n".into(), PathBuf::from("f.txt"));
        a.apply_cmd(a.parse_cmdline(":s/zzz/x/").unwrap());
        assert!(a.message.as_deref().unwrap_or("").contains("not found"));
        assert_eq!(a.ws.borrow().buffer().to_string(), "hello\n");
    }

    #[test]
    fn substitute_parsing_does_not_swallow_other_commands() {
        let a = App::new("x".into(), PathBuf::from("f.txt"));
        // ":sp" is a split, not a substitution.
        assert_eq!(a.parse_cmdline(":sp"), Ok(CmdAction::Split(SplitDir::Horizontal)));
        assert_eq!(a.parse_cmdline(":sym foo"), Ok(CmdAction::WorkspaceSymbol("foo".into())));
    }

    #[test]
    fn call_hierarchy_commands_parse() {
        let a = App::new("x".into(), PathBuf::from("f.txt"));
        assert_eq!(a.parse_cmdline(":callers"), Ok(CmdAction::CallHierarchy(true)));
        assert_eq!(a.parse_cmdline(":callees"), Ok(CmdAction::CallHierarchy(false)));
    }

    #[test]
    fn macro_record_and_replay() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let none = KeyModifiers::NONE;
        let mut a = App::new("aaa\nbbb\nccc\n".into(), PathBuf::from("f.txt"));

        // qq  x  q   — record "delete a char" into register q.
        a.handle_key(CtKey::new(KeyCode::Char('q'), none));
        a.handle_key(CtKey::new(KeyCode::Char('q'), none));
        assert!(a.macro_recording.is_some(), "recording started");
        a.handle_key(CtKey::new(KeyCode::Char('x'), none));
        a.handle_key(CtKey::new(KeyCode::Char('q'), none));
        assert!(a.macro_recording.is_none(), "recording stopped");
        assert_eq!(a.macros.get(&'q').map(|k| k.len()), Some(1));
        assert_eq!(a.ws.borrow().buffer().to_string(), "aa\nbbb\nccc\n");

        // @q replays it.
        a.handle_key(CtKey::new(KeyCode::Char('@'), none));
        a.handle_key(CtKey::new(KeyCode::Char('q'), none));
        assert_eq!(a.ws.borrow().buffer().to_string(), "a\nbbb\nccc\n");
    }

    #[test]
    fn set_editmode_switches_paradigm_and_routes_to_emacs() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let ctrl = KeyModifiers::CONTROL;
        let none = KeyModifiers::NONE;
        let mut a = App::new("hello".into(), PathBuf::from("f.txt"));
        assert_eq!(a.editmode, EditMode::Neovim);

        a.apply_cmd(a.parse_cmdline(":set editmode emacs").unwrap());
        assert_eq!(a.editmode, EditMode::Emacs);

        // Modeless: a plain key self-inserts (no Normal mode). Cursor is at 0.
        a.handle_key(CtKey::new(KeyCode::Char('X'), none));
        assert_eq!(a.ws.borrow().buffer().to_string(), "Xhello");

        // C-a jumps to line start, then C-e to line end.
        a.handle_key(CtKey::new(KeyCode::Char('e'), ctrl));
        a.handle_key(CtKey::new(KeyCode::Char('!'), none));
        assert_eq!(a.ws.borrow().buffer().to_string(), "Xhello!");

        // Switching back restores modal editing.
        a.apply_cmd(a.parse_cmdline(":set editmode neovim").unwrap());
        assert_eq!(a.editmode, EditMode::Neovim);
    }

    #[test]
    fn emacs_region_renders_a_selection() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let ctrl = KeyModifiers::CONTROL;
        let mut a = App::new("hello world".into(), PathBuf::from("f.txt"));
        a.set_editmode(EditMode::Emacs);
        a.ws.borrow_mut().execute(Action::Move(Motion::To(0)));
        // Set the mark, then move right five chars to select "hello".
        a.handle_key(CtKey::new(KeyCode::Char(' '), ctrl));
        assert_eq!(a.emacs.mark(), Some(0));
        for _ in 0..5 {
            a.handle_key(CtKey::new(KeyCode::Char('f'), ctrl));
        }
        // Mark at 0, point at 5 — the region spans "hello".
        assert_eq!(a.emacs.mark(), Some(0));
        assert_eq!(a.ws.borrow().primary_head(), 5);
        // The render path builds a SelectionView for the region without panic.
        a.render();
    }

    #[test]
    fn emacs_ctrl_x_ctrl_c_quits() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let ctrl = KeyModifiers::CONTROL;
        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        a.set_editmode(EditMode::Emacs);
        a.handle_key(CtKey::new(KeyCode::Char('x'), ctrl));
        assert!(a.emacs_ctrl_x, "C-x armed the prefix");
        a.handle_key(CtKey::new(KeyCode::Char('c'), ctrl));
        assert!(a.should_quit);
    }

    #[test]
    fn emacs_isearch_jumps_to_match() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let ctrl = KeyModifiers::CONTROL;
        let none = KeyModifiers::NONE;
        let mut a = App::new("foo bar foo".into(), PathBuf::from("f.txt"));
        a.set_editmode(EditMode::Emacs);
        a.ws.borrow_mut().execute(Action::Move(Motion::To(0)));
        // C-s starts isearch; typing "bar" moves point to offset 4.
        a.handle_key(CtKey::new(KeyCode::Char('s'), ctrl));
        for c in "bar".chars() {
            a.handle_key(CtKey::new(KeyCode::Char(c), none));
        }
        assert_eq!(a.ws.borrow().primary_head(), 4);
        // Enter ends the search.
        a.handle_key(CtKey::new(KeyCode::Enter, none));
        assert!(a.emacs_isearch.is_none());
    }

    #[test]
    fn visual_mode_produces_a_selection_for_the_active_window() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let none = KeyModifiers::NONE;
        let mut a = App::new("hello world\nsecond line\n".into(), PathBuf::from("f.txt"));
        // Enter visual mode and extend right a few characters.
        a.handle_key(CtKey::new(KeyCode::Char('v'), none));
        assert_eq!(a.vim.mode, VimMode::VisualChar);
        for _ in 0..4 {
            a.handle_key(CtKey::new(KeyCode::Char('l'), none));
        }
        // The active window's view carries the selection.
        let w = a.ws.borrow();
        let win = w.windows.active_window();
        let primary = win.cursors.primary();
        assert_ne!(primary.anchor, primary.head, "selection spans a range");
        drop(w);

        // Line-wise visual selects whole lines.
        a.handle_key(CtKey::new(KeyCode::Esc, none));
        a.handle_key(CtKey::new(KeyCode::Char('V'), none));
        assert_eq!(a.vim.mode, VimMode::VisualLine);
    }

    #[test]
    fn render_with_dummy_renderer_is_noop_and_safe() {
        // Ensures the multi-window render path builds a FrameState without panicking.
        let mut a = App::new("line1\nline2\nline3".into(), PathBuf::from("f.txt"));
        a.render();
    }

    #[test]
    fn parse_split_commands() {
        let a = App::new("x".into(), PathBuf::from("f.txt"));
        assert_eq!(a.parse_cmdline(":vsplit"), Ok(CmdAction::Split(SplitDir::Vertical)));
        assert_eq!(a.parse_cmdline(":sp"), Ok(CmdAction::Split(SplitDir::Horizontal)));
        assert_eq!(a.parse_cmdline(":only"), Ok(CmdAction::Only));
        assert_eq!(a.parse_cmdline(":close"), Ok(CmdAction::CloseWindow));
        assert_eq!(a.parse_cmdline(":fullscreen"), Ok(CmdAction::Fullscreen));
    }

    #[test]
    fn vsplit_produces_two_side_by_side_windows() {
        use ruster_core::windows::Rect;
        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        a.apply_cmd(CmdAction::Split(SplitDir::Vertical));
        let w = a.ws.borrow();
        let rects = w.windows.compute_rects(Rect::new(0, 0, 80, 24));
        assert_eq!(rects.len(), 2);
        // side by side: same y/height, different x
        assert_eq!(rects[0].1.y, rects[1].1.y);
        assert_ne!(rects[0].1.x, rects[1].1.x);
    }

    #[test]
    fn q_closes_window_when_multiple_else_quits() {
        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        a.apply_cmd(CmdAction::Split(SplitDir::Horizontal));
        assert_eq!(a.ws.borrow().windows.len(), 2);
        // First :q closes a window, does not quit.
        a.apply_cmd(CmdAction::Quit);
        assert_eq!(a.ws.borrow().windows.len(), 1);
        assert!(!a.should_quit);
        // Second :q on the last window quits.
        a.apply_cmd(CmdAction::Quit);
        assert!(a.should_quit);
    }

    #[test]
    fn ibuffer_lists_all_buffers_and_switches() {
        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        let scratch = a.ws.borrow_mut().buffers.create_scratch("scratch");
        a.apply_cmd(CmdAction::Ibuffer);
        {
            let p = a.picker.as_mut().expect("picker open");
            assert_eq!(p.view().rows.len(), 2);
        }
        a.dispatch_picker_action(PickerAction::OpenBuffer(scratch));
        assert_eq!(a.ws.borrow().active_buffer(), scratch);
    }

    #[test]
    fn bdelete_removes_buffer_when_another_exists() {
        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        let orig = a.ws.borrow().active_buffer();
        a.ws.borrow_mut().buffers.create_scratch("scratch");
        a.apply_cmd(CmdAction::BufferDelete);
        assert_eq!(a.ws.borrow().buffers.len(), 1);
        assert!(a.ws.borrow().buffers.get(orig).is_none());
    }

    #[test]
    fn bdelete_refuses_last_buffer() {
        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        a.apply_cmd(CmdAction::BufferDelete);
        assert_eq!(a.ws.borrow().buffers.len(), 1);
    }

    /// Open dired on a fresh temp dir containing the given subdirectories.
    fn dired_on_temp(name: &str, subdirs: &[&str]) -> (App, PathBuf) {
        let tmp = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&tmp);
        for s in subdirs {
            std::fs::create_dir_all(tmp.join(s)).unwrap();
        }
        if subdirs.is_empty() {
            std::fs::create_dir_all(&tmp).unwrap();
        }
        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        a.apply_cmd(CmdAction::Dired(Some(tmp.to_string_lossy().into_owned())));
        (a, tmp)
    }

    #[test]
    fn dired_colon_falls_through_to_cmdline() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let (mut a, tmp) = dired_on_temp("ruster_dired_colon", &[]);
        a.handle_key(CtKey::new(KeyCode::Char(':'), KeyModifiers::NONE));
        assert_eq!(a.vim.mode, VimMode::Cmdline, ": reaches the command line in dired");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn dired_slash_falls_through_to_search() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let (mut a, tmp) = dired_on_temp("ruster_dired_slash", &[]);
        a.handle_key(CtKey::new(KeyCode::Char('/'), KeyModifiers::NONE));
        assert_eq!(a.vim.mode, VimMode::Cmdline, "/ opens a search prompt in dired");
        assert!(a.vim.cmdline_buffer().starts_with('/'));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn dired_search_moves_cursor_and_enter_opens_entry() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let none = KeyModifiers::NONE;
        // Listing is "../", "adir/", "zebra/" (dirs sorted). Search for "zeb".
        let (mut a, tmp) = dired_on_temp("ruster_dired_search", &["adir", "zebra"]);
        for c in "/zeb".chars() {
            a.handle_key(CtKey::new(KeyCode::Char(c), none));
        }
        a.handle_key(CtKey::new(KeyCode::Enter, none)); // run the search → Move
        // The cursor is now on the "zebra/" line; a search key like 'd' in the
        // term must not have been hijacked by dired.
        assert!(a.dired_prompt.is_none(), "search term did not trigger dired keys");
        a.handle_key(CtKey::new(KeyCode::Enter, none)); // dired open at cursor
        let name = a.ws.borrow().active_doc().name.clone();
        assert!(name.ends_with("zebra"), "search then Enter opened zebra, got {name}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn dired_edit_keys_are_noops() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let none = KeyModifiers::NONE;
        let (mut a, tmp) = dired_on_temp("ruster_dired_ro", &["adir"]);
        let before = a.ws.borrow().buffer().to_string();
        // `x` deletes a char in vim; `i` then text would insert — both no-op here.
        a.handle_key(CtKey::new(KeyCode::Char('x'), none));
        a.handle_key(CtKey::new(KeyCode::Char('i'), none));
        a.handle_key(CtKey::new(KeyCode::Char('Z'), none));
        a.handle_key(CtKey::new(KeyCode::Esc, none));
        assert_eq!(a.ws.borrow().buffer().to_string(), before, "listing is read-only");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn dired_n_repeats_search_plus_creates() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let none = KeyModifiers::NONE;
        let (mut a, tmp) = dired_on_temp("ruster_dired_n", &["adir"]);
        // `n` is search-repeat now, not create: it must not open a Create prompt.
        a.handle_key(CtKey::new(KeyCode::Char('n'), none));
        assert!(a.dired_prompt.is_none(), "n no longer creates");
        // `+` still opens the Create prompt.
        a.handle_key(CtKey::new(KeyCode::Char('+'), none));
        assert!(matches!(
            a.dired_prompt,
            Some(DiredPrompt { kind: DiredPromptKind::Create, .. })
        ));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn dired_g_prefix_top_and_help() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let none = KeyModifiers::NONE;
        let (mut a, tmp) = dired_on_temp("ruster_dired_g", &["adir", "zebra"]);
        // Move down, then `gg` returns to the top.
        a.handle_key(CtKey::new(KeyCode::Char('j'), none));
        a.handle_key(CtKey::new(KeyCode::Char('j'), none));
        a.handle_key(CtKey::new(KeyCode::Char('g'), none));
        a.handle_key(CtKey::new(KeyCode::Char('g'), none));
        assert_eq!(a.ws.borrow().primary_head(), 0, "gg jumps to the top");
        // `g?` shows the dired help popup.
        a.handle_key(CtKey::new(KeyCode::Char('g'), none));
        a.handle_key(CtKey::new(KeyCode::Char('?'), none));
        assert!(a.hover.is_some(), "g? shows dired help");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn dired_opens_and_descends_into_subdir() {
        use ruster_core::document::{DocKind, SpecialKind};
        let tmp = std::env::temp_dir().join("ruster_app_dired");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("sub")).unwrap();
        std::fs::write(tmp.join("sub").join("inner.txt"), "hi").unwrap();

        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        a.apply_cmd(CmdAction::Dired(Some(tmp.to_string_lossy().into_owned())));
        assert!(matches!(
            a.ws.borrow().active_doc().kind,
            DocKind::Special(SpecialKind::Dired)
        ));
        // Listing: "../", "sub/". Move cursor to line 1 (sub) and Enter.
        let sub_line_start = {
            let w = a.ws.borrow();
            w.buffer().line_start_char(1)
        };
        a.ws.borrow_mut().execute(Action::Move(Motion::To(sub_line_start)));
        a.dired_open_at_cursor();
        let name = a.ws.borrow().active_doc().name.clone();
        assert!(name.ends_with("sub"), "descended into sub, got {name}");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(windows)]
    #[test]
    fn dired_ascends_from_drive_root_to_drive_picker() {
        let mut a = App::new(String::new(), PathBuf::from("f.txt"));
        a.apply_cmd(CmdAction::Dired(Some("C:\\".into())));
        // Ascend above the drive root: land in the drives view.
        a.dired_go_up();
        let id = a.ws.borrow().active_buffer();
        assert!(ruster_core::dired::is_drives_view(a.dired_dirs.get(&id).unwrap()));
        assert_eq!(a.ws.borrow().active_doc().name, "Drives");
        let content = a.ws.borrow().buffer().to_string();
        assert!(content.contains("C:"), "drives view lists C:, got {content:?}");

        // Selecting the C: entry descends back into that drive.
        let c_line = content.lines().position(|l| l.starts_with("C:")).unwrap();
        let start = a.ws.borrow().buffer().line_start_char(c_line);
        a.ws.borrow_mut().execute(Action::Move(Motion::To(start)));
        a.dired_open_at_cursor();
        let id2 = a.ws.borrow().active_buffer();
        assert!(!ruster_core::dired::is_drives_view(a.dired_dirs.get(&id2).unwrap()));
    }

    #[test]
    fn picker_preview_shows_selected_file_contents() {
        let tmp = std::env::temp_dir().join("ruster_preview_test.rs");
        std::fs::write(&tmp, "fn preview_me() {\n    let x = 1;\n}\n").unwrap();

        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        a.picker = Some(PickerState::new(
            "Files",
            vec![PickerItem::new(
                "preview_test.rs",
                PickerAction::OpenPath(tmp.clone()),
            )],
        ));
        let preview = a.picker_preview(10);
        assert!(
            preview.iter().any(|l| l.text.contains("preview_me")),
            "preview shows the file contents"
        );
        // Rust source is syntax-highlighted in the preview.
        assert!(preview.iter().any(|l| !l.highlights.is_empty()));

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn picker_preview_windows_around_a_location() {
        let tmp = std::env::temp_dir().join("ruster_preview_loc.rs");
        let body: String = (1..=60).map(|i| format!("// line {}\n", i)).collect();
        std::fs::write(&tmp, &body).unwrap();

        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        a.picker = Some(PickerState::new(
            "References",
            vec![PickerItem::new(
                "loc",
                PickerAction::OpenLocation(tmp.clone(), 40, 1),
            )],
        ));
        let preview = a.picker_preview(9);
        // The window is centred near line 40, not the top of the file.
        assert!(preview.iter().any(|l| l.text.contains("line 40")));
        assert!(!preview.iter().any(|l| l.text.contains("line 1\n")));

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn streamed_results_drain_into_picker() {
        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        let (tx, rx) = std::sync::mpsc::channel();
        a.picker = Some(PickerState::new("Files", Vec::new()));
        a.pending_results = Some(rx);
        tx.send(PickerItem::new("a.rs", PickerAction::OpenPath(PathBuf::from("a.rs")))).unwrap();
        tx.send(PickerItem::new("b.rs", PickerAction::OpenPath(PathBuf::from("b.rs")))).unwrap();
        a.drain_pending_results();
        assert_eq!(a.picker.as_ref().unwrap().len(), 2);
        assert!(a.pending_results.is_some(), "still streaming while sender is alive");
        drop(tx);
        a.drain_pending_results();
        assert!(a.pending_results.is_none(), "cleared once the sender disconnects");
    }

    #[test]
    fn dired_create_and_delete_file() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let none = KeyModifiers::NONE;
        let tmp = std::env::temp_dir().join("ruster_dired_mut");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        a.apply_cmd(CmdAction::Dired(Some(tmp.to_string_lossy().into_owned())));

        // '+' then type a name then Enter creates the file.
        a.handle_key(CtKey::new(KeyCode::Char('+'), none));
        assert!(a.dired_prompt.is_some());
        for c in "new.txt".chars() {
            a.handle_key(CtKey::new(KeyCode::Char(c), none));
        }
        a.handle_key(CtKey::new(KeyCode::Enter, none));
        assert!(a.dired_prompt.is_none());
        assert!(tmp.join("new.txt").exists(), "file created");

        // Move cursor onto new.txt (listing: "..", "new.txt") and delete it.
        let line1 = a.ws.borrow().buffer().line_start_char(1);
        a.ws.borrow_mut().execute(Action::Move(Motion::To(line1)));
        a.handle_key(CtKey::new(KeyCode::Char('D'), none));
        assert!(matches!(
            a.dired_prompt.as_ref().map(|p| &p.kind),
            Some(DiredPromptKind::Delete(_))
        ));
        a.handle_key(CtKey::new(KeyCode::Char('y'), none));
        assert!(!tmp.join("new.txt").exists(), "file deleted");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn dired_create_dir_with_trailing_slash_and_rejects_duplicates() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let none = KeyModifiers::NONE;
        let tmp = std::env::temp_dir().join("ruster_dired_create");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        a.apply_cmd(CmdAction::Dired(Some(tmp.to_string_lossy().into_owned())));

        // "sub/" creates a directory.
        a.handle_key(CtKey::new(KeyCode::Char('+'), none));
        for c in "sub/".chars() {
            a.handle_key(CtKey::new(KeyCode::Char(c), none));
        }
        a.handle_key(CtKey::new(KeyCode::Enter, none));
        assert!(tmp.join("sub").is_dir(), "trailing slash creates a directory");

        // Creating it again reports that it exists.
        a.handle_key(CtKey::new(KeyCode::Char('+'), none));
        for c in "sub/".chars() {
            a.handle_key(CtKey::new(KeyCode::Char(c), none));
        }
        a.handle_key(CtKey::new(KeyCode::Enter, none));
        assert!(a.message.as_deref().unwrap_or("").contains("already exists"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn dired_colors_dirs_execs_and_files_differently() {
        use ruster_render::Color;
        let tmp = std::env::temp_dir().join("ruster_dired_colors");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("adir")).unwrap();
        std::fs::write(tmp.join("plain.txt"), "x").unwrap();
        std::fs::write(tmp.join("runme.sh"), "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let p = tmp.join("runme.sh");
            let mut perms = std::fs::metadata(&p).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&p, perms).unwrap();
        }

        let entries = ruster_core::dired::list(&tmp, true);
        let styled = dired_styled_lines(&entries);
        let fg_of = |name: &str| -> Option<Color> {
            styled
                .iter()
                .find(|l| l.text.trim_end_matches('/') == name)
                .and_then(|l| l.highlights.first().map(|(_, _, s)| s.fg))
        };

        // Directories are blue, and ".." counts as one.
        assert_eq!(fg_of("adir"), Some(Color::Rgb(137, 180, 250)));
        assert_eq!(fg_of(".."), Some(Color::Rgb(137, 180, 250)));
        // A plain file has no color override.
        assert_eq!(fg_of("plain.txt"), None);
        #[cfg(unix)]
        {
            // An executable is green.
            assert_eq!(fg_of("runme.sh"), Some(Color::Rgb(166, 227, 161)));
        }

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn dired_dot_toggles_hidden_files() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let none = KeyModifiers::NONE;
        let tmp = std::env::temp_dir().join("ruster_dired_dot");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join(".hidden"), "x").unwrap();
        std::fs::write(tmp.join("shown.txt"), "y").unwrap();

        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        a.apply_cmd(CmdAction::Dired(Some(tmp.to_string_lossy().into_owned())));

        // Hidden by default.
        let text = a.ws.borrow().buffer().to_string();
        assert!(text.contains("shown.txt"));
        assert!(!text.contains(".hidden"), "dot-files hidden by default");

        // '.' reveals them.
        a.handle_key(CtKey::new(KeyCode::Char('.'), none));
        assert!(a.dired_show_hidden);
        let text = a.ws.borrow().buffer().to_string();
        assert!(text.contains(".hidden"), "dot-files shown after toggle");

        // '.' again hides them.
        a.handle_key(CtKey::new(KeyCode::Char('.'), none));
        let text = a.ws.borrow().buffer().to_string();
        assert!(!text.contains(".hidden"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn dired_yy_copies_and_p_pastes() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let none = KeyModifiers::NONE;
        let tmp = std::env::temp_dir().join("ruster_dired_copy");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("sub")).unwrap();
        std::fs::write(tmp.join("a.txt"), "hello").unwrap();

        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        a.apply_cmd(CmdAction::Dired(Some(tmp.to_string_lossy().into_owned())));

        // Listing: "..", "sub/", "a.txt" — put the cursor on a.txt (line 2).
        let line2 = a.ws.borrow().buffer().line_start_char(2);
        a.ws.borrow_mut().execute(Action::Move(Motion::To(line2)));
        a.handle_key(CtKey::new(KeyCode::Char('y'), none));
        assert!(a.dired_pending_y, "first y is pending");
        a.handle_key(CtKey::new(KeyCode::Char('y'), none));
        assert_eq!(a.dired_clipboard.as_ref().map(|(_, cut)| *cut), Some(false));

        // Descend into sub/ and paste.
        let dired_buf = a.ws.borrow().active_buffer();
        a.refresh_dired(dired_buf, tmp.join("sub"));
        a.handle_key(CtKey::new(KeyCode::Char('p'), none));
        assert!(tmp.join("sub").join("a.txt").exists(), "file pasted into sub/");
        assert!(tmp.join("a.txt").exists(), "copy leaves the original");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn dired_dd_cuts_and_p_moves() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let none = KeyModifiers::NONE;
        let tmp = std::env::temp_dir().join("ruster_dired_cut");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("sub")).unwrap();
        std::fs::write(tmp.join("b.txt"), "hi").unwrap();

        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        a.apply_cmd(CmdAction::Dired(Some(tmp.to_string_lossy().into_owned())));
        let line2 = a.ws.borrow().buffer().line_start_char(2);
        a.ws.borrow_mut().execute(Action::Move(Motion::To(line2)));
        a.handle_key(CtKey::new(KeyCode::Char('d'), none));
        a.handle_key(CtKey::new(KeyCode::Char('d'), none));
        assert_eq!(a.dired_clipboard.as_ref().map(|(_, cut)| *cut), Some(true));

        let dired_buf = a.ws.borrow().active_buffer();
        a.refresh_dired(dired_buf, tmp.join("sub"));
        a.handle_key(CtKey::new(KeyCode::Char('p'), none));
        assert!(tmp.join("sub").join("b.txt").exists(), "moved into sub/");
        assert!(!tmp.join("b.txt").exists(), "cut removes the original");
        assert!(a.dired_clipboard.is_none(), "cut is consumed by the paste");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn parse_rg_vimgrep_line() {
        let (path, line, col, body) =
            parse_rg_line("src/main.rs:12:5:let x = 1").expect("parses");
        assert_eq!(path, PathBuf::from("src/main.rs"));
        assert_eq!(line, 12);
        assert_eq!(col, 5);
        assert_eq!(body, "let x = 1");
    }

    #[test]
    fn parse_rg_line_keeps_colons_in_body() {
        let (_p, l, c, body) = parse_rg_line("a.rs:3:1:foo: bar: baz").expect("parses");
        assert_eq!((l, c), (3, 1));
        assert_eq!(body, "foo: bar: baz");
    }

    #[test]
    fn parse_rg_line_rejects_malformed() {
        assert!(parse_rg_line("not a grep line").is_none());
        assert!(parse_rg_line("a.rs:notanumber:1:x").is_none());
    }

    #[test]
    fn parse_rg_and_files_commands() {
        let a = App::new("x".into(), PathBuf::from("f.txt"));
        assert_eq!(a.parse_cmdline(":Files"), Ok(CmdAction::Files));
        assert_eq!(a.parse_cmdline(":Rg todo"), Ok(CmdAction::Rg("todo".into())));
        assert!(a.parse_cmdline(":Rg").is_err());
    }

    #[test]
    fn snippet_expands_on_tab_in_insert() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        // Type the trigger "fn" in a .rs buffer, then Tab in insert mode.
        let mut a = App::new(String::new(), PathBuf::from("f.rs"));
        a.vim.mode = VimMode::Insert;
        for c in "fn".chars() {
            a.handle_key(CtKey::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        a.handle_key(CtKey::new(KeyCode::Tab, KeyModifiers::NONE));
        let text = a.ws.borrow().active_doc().buffer.to_string();
        assert!(text.contains("fn name("), "snippet expanded: {text:?}");
        // Two more tabstops ($2 args, $0 body) remain after the first jump.
        assert_eq!(a.snippet_stops.len(), 2);
    }

    #[test]
    fn hover_markdown_highlights_code_and_strips_fences() {
        let md = "```rust\npub struct Range {}\n```\n\n---\n\nsize = 16 (0x10)";
        let lines = build_hover_lines(md);
        // Fence markers and separators are removed.
        assert!(lines.iter().all(|l| !l.text.contains("```")));
        assert!(lines.iter().all(|l| l.text.trim() != "---"));
        // The code line is syntax-highlighted.
        assert!(lines
            .iter()
            .any(|l| l.text.contains("struct") && !l.highlights.is_empty()));
        // Prose survives as plain text.
        assert!(lines.iter().any(|l| l.text.contains("size = 16")));
    }

    #[test]
    fn word_before_extracts_identifier() {
        assert_eq!(word_before("  foo", 5), "foo");
        assert_eq!(word_before("a.bar", 5), "bar");
        assert_eq!(word_before("x = ", 4), "");
    }

    #[test]
    fn parse_lsp_commands() {
        let a = App::new("x".into(), PathBuf::from("f.rs"));
        assert_eq!(a.parse_cmdline(":fmt"), Ok(CmdAction::Format));
        assert_eq!(a.parse_cmdline(":rename Foo"), Ok(CmdAction::Rename("Foo".into())));
        assert!(a.parse_cmdline(":rename").is_err());
    }

    #[test]
    fn leader_code_group_resolves() {
        assert!(matches!(
            leader_resolve(&['c', 'k']),
            LeaderResolve::Action(LeaderAction::Hover)
        ));
        assert!(matches!(
            leader_resolve(&['c', 'g']),
            LeaderResolve::Action(LeaderAction::Definition)
        ));
        let (title, rows) = leader_whichkey(&['c']).expect("code panel");
        assert_eq!(title, "SPC c");
        assert!(rows.iter().any(|r| r.contains("hover")));
        assert!(rows.iter().any(|r| r.contains("references")));
    }

    #[test]
    fn diagnostics_stored_and_surfaced_on_line() {
        let mut a = App::new("let x = 1;\n".into(), PathBuf::from("f.rs"));
        let buf = a.ws.borrow().active_buffer();
        a.diagnostics.insert(
            buf,
            vec![ruster_lsp::Diagnostic {
                start: ruster_lsp::results::LspPositionEq { line: 0, character: 4 },
                end: ruster_lsp::results::LspPositionEq { line: 0, character: 5 },
                severity: 1,
                message: "unused".into(),
            }],
        );
        // Cursor is on line 0, so the diagnostic surfaces.
        let msg = a.current_line_diagnostic().expect("diagnostic on line");
        assert!(msg.contains("unused"));
        assert!(msg.starts_with("[E]"));
    }

    #[test]
    fn command_palette_opens_with_seed_and_filters() {
        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        a.open_command_picker("wq");
        let p = a.picker.as_mut().expect("palette open");
        assert_eq!(p.filter, "wq");
        assert!(!p.filtered().is_empty(), "seed matches at least one command");
    }

    #[test]
    fn leader_resolves_groups_and_actions() {
        assert!(matches!(leader_resolve(&[]), LeaderResolve::Group));
        assert!(matches!(leader_resolve(&['w']), LeaderResolve::Group));
        assert!(matches!(
            leader_resolve(&['w', 'h']),
            LeaderResolve::Action(LeaderAction::Focus(FocusDir::Left))
        ));
        assert!(matches!(leader_resolve(&['z']), LeaderResolve::Unknown));
        assert!(matches!(leader_resolve(&['w', 'x']), LeaderResolve::Unknown));
    }

    #[test]
    fn leader_whichkey_shows_groups() {
        let (title, rows) = leader_whichkey(&[]).expect("root panel");
        assert_eq!(title, "SPC");
        assert!(rows.iter().any(|r| r.contains("+windows")));
        assert!(rows.iter().any(|r| r.contains("+quit")));

        let (wtitle, wrows) = leader_whichkey(&['w']).expect("window panel");
        assert_eq!(wtitle, "SPC w");
        assert!(wrows.iter().any(|r| r.starts_with("h ")));
        assert!(wrows.iter().any(|r| r.contains("focus left")));
    }

    #[test]
    fn which_key_is_delayed_by_timeoutlen() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        a.handle_key(CtKey::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert!(a.leader_pending.is_some());
        assert!(a.leader_since.is_some());
        // First frame is within timeoutlen (default 300ms), so the panel stays hidden.
        a.render();
        assert_eq!(a.whichkey_anim, 0.0, "panel should not appear before timeoutlen");
    }

    #[test]
    fn leader_quit_group_resolves() {
        assert!(matches!(
            leader_resolve(&['q', 'q']),
            LeaderResolve::Action(LeaderAction::Quit)
        ));
        assert!(matches!(
            leader_resolve(&['q', 'w']),
            LeaderResolve::Action(LeaderAction::SaveAndQuit)
        ));
    }

    #[test]
    fn space_q_q_quits() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        a.handle_key(CtKey::new(KeyCode::Char(' '), KeyModifiers::NONE));
        a.handle_key(CtKey::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(!a.should_quit, "q group is a prefix, not yet quit");
        a.handle_key(CtKey::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(a.should_quit);
    }

    #[test]
    fn space_w_l_focuses_right_split() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        let left = a.ws.borrow().windows.active();
        a.apply_cmd(CmdAction::Split(SplitDir::Vertical)); // new right window active
        let right = a.ws.borrow().windows.active();
        assert_ne!(left, right);
        // Move focus back to the left window, then via SPC w l to the right.
        a.ws.borrow_mut().windows.focus(FocusDir::Left);
        assert_eq!(a.ws.borrow().windows.active(), left);
        a.handle_key(CtKey::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert!(a.leader_pending.is_some());
        a.handle_key(CtKey::new(KeyCode::Char('w'), KeyModifiers::NONE));
        assert_eq!(a.leader_pending.as_deref(), Some(&['w'][..]));
        a.handle_key(CtKey::new(KeyCode::Char('l'), KeyModifiers::NONE));
        assert!(a.leader_pending.is_none());
        assert_eq!(a.ws.borrow().windows.active(), right);
    }

    #[test]
    fn ctrl_w_z_toggles_fullscreen() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        a.apply_cmd(CmdAction::Split(SplitDir::Vertical));
        // Ctrl-w then z
        a.handle_key(CtKey::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert!(a.pending_ctrl_w);
        a.handle_key(CtKey::new(KeyCode::Char('z'), KeyModifiers::NONE));
        assert!(a.ws.borrow().windows.is_fullscreen());
    }

    #[test]
    fn cursor_anim_converges_to_target() {
        let mut anim = CursorAnim::new();
        let dt = std::time::Duration::from_secs_f64(1.0/60.0);
        // After many frames at 60fps, should be very close to target
        for _ in 0..60 {
            anim.update(dt, 10, 5, true, 12.0);
        }
        let dx = (anim.cell_x - 10.0).abs();
        let dy = (anim.cell_y - 5.0).abs();
        assert!(dx < 0.01, "cell_x should converge to target: {dx}");
        assert!(dy < 0.01, "cell_y should converge to target: {dy}");
    }

    #[test]
    fn cursor_anim_disabled_snaps() {
        let mut anim = CursorAnim::new();
        anim.cell_x = 100.0;
        anim.cell_y = 200.0;
        anim.update(std::time::Duration::from_secs_f64(0.5), 5, 3, false, 12.0);
        assert_eq!(anim.cell_x, 5.0);
        assert_eq!(anim.cell_y, 3.0);
    }
}
