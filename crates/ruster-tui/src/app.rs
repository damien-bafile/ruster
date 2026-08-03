use crate::dialog::{DialogResponse, DialogState};
use crate::dired::{DiredResponse, DiredState};
use crate::file_prompt::{self, FilePrompt, PromptOrigin, PromptStep};
use crate::key::crossterm_to_ruster_key;
use crate::picker::{PickerAction, PickerItem, PickerState};
use crate::quickfix::{QuickfixItem, QuickfixList};
use crate::renderer::TuiRenderer;
use crate::settings::{SettingsState, SyntaxSeed};
use crate::sidebar::{SidebarResponse, SidebarState};
use crate::trouble::{Source as TroubleSource, TroubleItem, TroubleState};
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
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEventKind, KeyModifiers, MouseButton,
    MouseEvent, MouseEventKind,
};
use ruster_lua::{config::Config, schema::{SettingKind, SettingValue}, LuaAction, LuaRuntime};
use ruster_notify::{BackendKind, Notification, NotificationManager};
use ruster_render::{
    CursorKind, FlashLabelRender, FrameState, Rect as RRect, Renderer, SelectionView,
    StatuslineView, StyledLine, SyntaxStyle, WelcomeView, WhichKeyView, WindowView,
};
use ruster_syntax::SyntaxEngine;
use ruster_lsp::{LspPosition, ServerMessage};
use ruster_terminal::{encode_key, Key as TKey, Mods as TMods, TerminalSession};
use std::cell::RefCell;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

/// A single flash jump label.
#[derive(Debug, Clone)]
pub struct FlashLabel {
    pub label: String,
    pub offset: usize,
}

/// Active flash jump mode state.
#[derive(Debug)]
pub struct FlashState {
    pub labels: Vec<FlashLabel>,
    pub pending: Option<char>,
}

/// Where one window's buffer text was drawn in the last rendered frame.
///
/// Mouse hit-testing resolves clicks against this rather than recomputing the
/// layout, so it cannot disagree with what the user is looking at — the sidebar
/// column, the conditionally reserved cmdline row, the window header and the
/// sign/number gutter are all already accounted for.
#[derive(Debug, Clone, Copy)]
pub struct WindowLayout {
    pub window: ruster_core::windows::WindowId,
    pub buffer: BufferId,
    pub text: ruster_render::TextArea,
    /// First visible buffer line, so a screen row maps back to a buffer line.
    pub scroll_top: usize,
}

/// Infinite iterator over adaptive labels: a-z, aa-az, ba-bz, …
fn label_pool_iter() -> impl Iterator<Item = String> {
    let single = ('a'..='z').map(|c| c.to_string());
    let multi = ('a'..='z').flat_map(|first| {
        ('a'..='z').map(move |second| format!("{}{}", first, second))
    });
    single.chain(multi)
}

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

/// The ruster config directory: `$XDG_CONFIG_HOME/ruster` when set, else
/// `~/.config/ruster` on Unix (incl. macOS, matching nvim/helix conventions) and
/// `%APPDATA%\ruster` on Windows. (`dirs::config_dir()` would give
/// `~/Library/Application Support` on macOS, which CLI-editor users don't expect.)
fn ruster_config_dir() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("ruster"));
        }
    }
    #[cfg(windows)]
    {
        dirs::config_dir().map(|d| d.join("ruster"))
    }
    #[cfg(not(windows))]
    {
        dirs::home_dir().map(|h| h.join(".config").join("ruster"))
    }
}


/// Resolve a theme's UI colors: the theme's role colors (user `themes/<name>.lua`
/// first, then a built-in, then default) with the per-element overrides layered on.
fn resolve_theme_colors(
    lua: &LuaRuntime,
    theme_name: &str,
    ov: &ruster_lua::config::ColorOverrides,
) -> ruster_lua::config::ThemeColors {
    use ruster_lua::config::Rgb;
    let mut colors = ruster_config_dir()
        .map(|d| d.join("themes").join(format!("{theme_name}.lua")))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|code| lua.load_theme(&code))
        .map(|t| t.roles)
        .or_else(|| {
            ruster_lua::config::builtin_themes()
                .into_iter()
                .find(|(n, _)| *n == theme_name)
                .map(|(_, t)| t.roles)
        })
        .unwrap_or_default();
    let set = |hex: &str, field: &mut Rgb| {
        if let Some((r, g, b)) = ruster_lua::schema::parse_hex_color(hex) {
            *field = Rgb::new(r, g, b);
        }
    };
    set(&ov.bg, &mut colors.bg);
    set(&ov.fg, &mut colors.fg);
    set(&ov.gutter, &mut colors.gutter);
    set(&ov.gutter_bg, &mut colors.gutter_bg);
    set(&ov.selection_bg, &mut colors.selection_bg);
    set(&ov.selection_fg, &mut colors.selection_fg);
    set(&ov.cursor_bg, &mut colors.cursor_bg);
    set(&ov.cursor_fg, &mut colors.cursor_fg);
    set(&ov.divider, &mut colors.divider);
    set(&ov.statusline_fg, &mut colors.statusline_fg);
    set(&ov.statusline_bg, &mut colors.statusline_bg);
    set(&ov.accent, &mut colors.accent);
    set(&ov.accent_fg, &mut colors.accent_fg);
    set(&ov.whichkey_bg, &mut colors.whichkey_bg);
    set(&ov.whichkey_fg, &mut colors.whichkey_fg);
    set(&ov.whichkey_key, &mut colors.whichkey_key);
    set(&ov.cmdline_bg, &mut colors.cmdline_bg);
    set(&ov.cmdline_fg, &mut colors.cmdline_fg);
    set(&ov.mode_normal_bg, &mut colors.mode_normal_bg);
    set(&ov.mode_normal_fg, &mut colors.mode_normal_fg);
    set(&ov.mode_insert_bg, &mut colors.mode_insert_bg);
    set(&ov.mode_insert_fg, &mut colors.mode_insert_fg);
    set(&ov.mode_visual_bg, &mut colors.mode_visual_bg);
    set(&ov.mode_visual_fg, &mut colors.mode_visual_fg);
    set(&ov.mode_cmdline_bg, &mut colors.mode_cmdline_bg);
    set(&ov.mode_cmdline_fg, &mut colors.mode_cmdline_fg);
    set(&ov.mode_emacs_bg, &mut colors.mode_emacs_bg);
    set(&ov.mode_emacs_fg, &mut colors.mode_emacs_fg);
    colors
}

/// Convert the config's `lang -> group -> hex` syntax overrides into the color
/// map the syntax highlighter consumes.
fn syntax_overrides_to_colors(
    map: &std::collections::HashMap<String, std::collections::HashMap<String, String>>,
) -> ruster_syntax::SyntaxOverrides {
    let mut out = ruster_syntax::SyntaxOverrides::new();
    for (lang, groups) in map {
        let mut m = std::collections::HashMap::new();
        for (group, hex) in groups {
            if let Some((r, g, b)) = ruster_lua::schema::parse_hex_color(hex) {
                m.insert(group.clone(), ruster_render::Color::Rgb(r, g, b));
            }
        }
        if !m.is_empty() {
            out.insert(lang.clone(), m);
        }
    }
    out
}

/// What `ruster.api.buf_path()` and friends read.
///
/// A snapshot the app refreshes each frame rather than callbacks reaching into
/// `App`. The Lua closures are installed before `App` exists — they capture the
/// workspace `Rc`, and there is no `&mut self` for them to hold — so anything
/// living on `App` (diagnostics, git state) has to be pushed here instead of
/// pulled from there.
#[derive(Default, Clone)]
struct QuerySnapshot {
    path: String,
    filetype: String,
    diagnostics: Vec<ruster_lua::runtime::LuaDiagnostic>,
    branch: String,
    staged: usize,
    unstaged: usize,
}

/// The editor state the Lua event layer watches, so a change becomes an event.
///
/// Diffed once per frame rather than firing from each mutation site. There are
/// far too many places that can change the active buffer — every open, close,
/// split, pick, jump and `:bd` — and an event that fires from most of them is
/// worse than one that fires from all of them, because a plugin cannot tell
/// which case it missed.
///
/// It also debounces `CursorMoved` for free: a held `j` moves the cursor many
/// times between frames and fires one event, which is the behaviour the plan
/// asked for and would otherwise need its own timer.
#[derive(Default, Clone, PartialEq, Eq)]
struct WatchedState {
    buffer: Option<BufferId>,
    /// Path of `buffer`, resolved when it changed so `BufLeave` can name the
    /// buffer being left after the switch has already happened.
    path: String,
    window: Option<ruster_core::windows::WindowId>,
    cursor: (usize, usize),
    filetype: String,
}

/// A diagnostic severity's sign glyph + color (1=error … 4=hint).
fn severity_sign(severity: u8) -> (char, ruster_render::Color) {
    let group = match severity {
        1 => "error",
        2 => "warning",
        3 => "info",
        _ => "hint",
    };
    let glyph = match severity {
        1 => 'E',
        2 => 'W',
        3 => 'I',
        _ => 'H',
    };
    (glyph, ruster_syntax::sign_style(group).fg)
}

fn vim_mode_to_ui_mode(mode: ruster_core::vim::VimMode) -> ruster_render::UIMode {
    use ruster_core::vim::VimMode;
    match mode {
        VimMode::Normal => ruster_render::UIMode::Normal,
        VimMode::Insert => ruster_render::UIMode::Insert,
        VimMode::VisualChar | VimMode::VisualLine | VimMode::VisualBlock => {
            ruster_render::UIMode::Visual
        }
        VimMode::Cmdline => ruster_render::UIMode::Cmdline,
    }
}

/// Expand leading `~` to the user's home directory and resolve relative paths
/// against a base directory. Normalizes the result.
fn resolve_path(raw: &str, base_dir: &std::path::Path) -> std::path::PathBuf {
    let expanded = if let Some(rest) = raw.strip_prefix("~/") {
        let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        home.join(rest)
    } else if raw == "~" {
        dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."))
    } else {
        std::path::PathBuf::from(raw)
    };
    if expanded.is_absolute() {
        expanded
    } else {
        base_dir.join(expanded)
    }
}

/// Render one pane of a side-by-side diff.
///
/// `rows` comes from [`ruster_git::align`]; `pick` selects which side of each
/// row this pane shows. The source line number is written into the text rather
/// than left to the gutter, because padding rows have no line of their own and
/// the gutter — which numbers display rows — would label every line after the
/// first hunk wrongly.
fn diff_pane_text(rows: &[(Option<u32>, Option<u32>)], lines: &[&str], right: bool) -> String {
    let width = lines.len().max(1).to_string().len();
    rows.iter()
        .map(|row| {
            match if right { row.1 } else { row.0 } {
                Some(n) => {
                    let text = lines.get(n as usize).copied().unwrap_or("");
                    format!("{:>width$} │ {}", n + 1, text, width = width)
                }
                // No counterpart on this side: filler, as vimdiff shows it.
                None => format!("{:>width$} │ ~", "", width = width),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Report a problem loading a user highlight query.
///
/// Warning rather than error: highlighting has already fallen back to the
/// built-in query, so the editor still works — the user just isn't getting the
/// customisation they asked for, and would otherwise have no way to know.
fn push_query_warning(notify: &mut NotificationManager, text: String) {
    notify.push(Notification::new(
        ruster_core::message::MessageLevel::Warning,
        ruster_core::message::MessageSource::System,
        text,
    ));
    }

/// The first `ruster-NNN.png` in `dir` that does not exist yet, so repeated
/// `:screenshot` calls accumulate instead of overwriting one file.
fn next_screenshot_path(dir: &std::path::Path) -> PathBuf {
    (1u32..)
        .map(|n| dir.join(format!("ruster-{n:03}.png")))
        .find(|p| !p.exists())
        // Unreachable short of four billion screenshots, but a fallback keeps
        // this total rather than panicking on an exhausted range.
        .unwrap_or_else(|| dir.join("ruster.png"))
}

/// Where `:screenshot [arg]` should write, resolved against `cwd`.
///
/// An absent argument, or one naming an existing directory, picks a fresh
/// numbered file there. The extension is forced to `.png` because the backend
/// chooses its encoder from it, and any other suffix would produce a file
/// nothing can open.
fn screenshot_path(arg: Option<&str>, cwd: &std::path::Path) -> PathBuf {
    let Some(arg) = arg.map(str::trim).filter(|s| !s.is_empty()) else {
        return next_screenshot_path(cwd);
    };
    let mut p = resolve_path(arg, cwd);
    if p.is_dir() {
        return next_screenshot_path(&p);
    }
    if p.extension().is_none_or(|e| !e.eq_ignore_ascii_case("png")) {
        let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
        p.set_file_name(format!("{name}.png"));
    }
    p
}

/// State for cmdline path completion (Tab/Shift-Tab cycling).
struct CmdlineCompletion {
    /// The original text before completion started.
    original: String,
    /// Completion candidates (file/dir paths).
    candidates: Vec<String>,
    /// Index of the currently selected candidate (0 = first candidate).
    index: usize,
    /// The prefix before the path portion (e.g., ":e ").
    prefix: String,
}

/// Build a sign column from a buffer's diagnostics: one glyph per line, the most
/// severe (lowest severity number) winning when several land on the same line.
/// The colour for a `TODO`-class keyword — the same amber the warning severity
/// uses, so the gutter and the comment agree on what "needs attention" looks like.
fn todo_style() -> ruster_render::SyntaxStyle {
    ruster_syntax::sign_style("todo")
}

fn diagnostics_to_signs(diags: &[ruster_lsp::Diagnostic]) -> ruster_render::SignsView {
    let mut best: std::collections::HashMap<u16, u8> = std::collections::HashMap::new();
    for d in diags {
        let line = d.start.line as u16;
        let e = best.entry(line).or_insert(u8::MAX);
        *e = (*e).min(d.severity);
    }
    let mut signs: Vec<(u16, char, ruster_render::Color)> = best
        .into_iter()
        .map(|(line, sev)| {
            let (g, c) = severity_sign(sev);
            (line, g, c)
        })
        .collect();
    signs.sort_by_key(|(l, _, _)| *l);
    let width = if signs.is_empty() { 0 } else { 1 };
    ruster_render::SignsView { width, signs }
}

/// Find an executable named `name` on `$PATH`, returning its full path.
fn find_in_path(name: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let full = dir.join(name);
        if full.is_file() {
            return Some(full.to_string_lossy().into_owned());
        }
    }
    None
}

fn plain_lines(content: &str) -> Vec<StyledLine> {
    content
        .split('\n')
        .map(|s| StyledLine { text: s.to_string(), highlights: vec![] })
        .collect()
}

/// Map a crossterm key event onto a terminal `Key`/`Mods` for PTY encoding.
/// Returns `None` for keys with no terminal representation.
fn term_key_from_crossterm(ck: crossterm::event::KeyEvent) -> Option<(TKey, TMods)> {
    let m = ck.modifiers;
    let mods = TMods {
        ctrl: m.contains(KeyModifiers::CONTROL),
        alt: m.contains(KeyModifiers::ALT),
        shift: m.contains(KeyModifiers::SHIFT),
    };
    let key = match ck.code {
        KeyCode::Char(c) => TKey::Char(c),
        KeyCode::Enter => TKey::Enter,
        KeyCode::Tab => TKey::Tab,
        KeyCode::Backspace => TKey::Backspace,
        KeyCode::Esc => TKey::Esc,
        KeyCode::Up => TKey::Up,
        KeyCode::Down => TKey::Down,
        KeyCode::Left => TKey::Left,
        KeyCode::Right => TKey::Right,
        KeyCode::Home => TKey::Home,
        KeyCode::End => TKey::End,
        KeyCode::PageUp => TKey::PageUp,
        KeyCode::PageDown => TKey::PageDown,
        KeyCode::Delete => TKey::Delete,
        KeyCode::Insert => TKey::Insert,
        _ => return None,
    };
    Some((key, mods))
}

/// Convert a terminal-core grid snapshot into a renderer-neutral grid view.
fn to_term_grid_view(grid: &ruster_terminal::TermGrid) -> ruster_render::TermGridView {
    let conv = |c: ruster_terminal::TermColor| match c {
        ruster_terminal::TermColor::Default => ruster_render::Color::Default,
        ruster_terminal::TermColor::Rgb(r, g, b) => ruster_render::Color::Rgb(r, g, b),
    };
    let cells = grid
        .cells
        .iter()
        .map(|c| ruster_render::TermCellView {
            c: c.c,
            fg: conv(c.fg),
            bg: conv(c.bg),
            bold: c.attrs.bold,
            italic: c.attrs.italic,
            underline: c.attrs.underline,
            inverse: c.attrs.inverse,
        })
        .collect();
    ruster_render::TermGridView { cols: grid.cols, rows: grid.rows, cells, cursor: grid.cursor }
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

/// The value part of a general `:set` command. `Toggle` flips the current value
/// of a boolean option; `Exact` sets any option to a specific parsed value.
#[derive(Debug, Clone, PartialEq)]
enum SetNamedVal {
    Exact(SettingValue),
    Toggle,
}

#[derive(Debug, Clone, PartialEq)]
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
    /// General schema-backed `:set key[=value]`, `:set nokey`, `:set key!`.
    SetNamed(String, SetNamedVal),
    /// Display current setting value (`:set key?` or bare `:set key` on non-bool).
    ShowSetting(String),
    /// Reset a setting to its schema default (`:set key&`).
    ResetSetting(String),
    /// Echo a message (`:echo text` / `:echom text` / `:echoe text`).
    Echo(String, ruster_core::message::MessageLevel),
    /// Open an embedded terminal (`:term` / `:terminal`).
    Terminal,
    /// Show config load/validation errors (`:config-errors`).
    ConfigErrors,
    /// Open the settings page (`:settings` / `:config`).
    Settings,
    /// Run the project's build command (`:build` / `:make`).
    Build,
    /// Run the project's test command (`:test`).
    Test,
    /// Pick a `ruster.toml` task to run (`:task`).
    TaskPicker,
    /// Open the quickfix list as a picker (`:copen`).
    QuickfixOpen,
    /// Step to the next/prev quickfix entry and jump (`:cnext`/`:cprev`).
    QuickfixNext,
    QuickfixPrev,
    /// `:s/pat/rep/[g]` — replace on the current line, or the whole buffer
    /// when `whole_buffer` (`:%s/...`). `all` is the `g` flag.
    Substitute {
        pattern: String,
        replacement: String,
        all: bool,
        whole_buffer: bool,
    },
    /// Open the messages buffer (`:messages` / `:msgs`).
    Messages,
    /// Filter the messages buffer by source or level.
    MessagesFilter(String),
    /// Project list / switch (`:projects`).
    Projects,
    /// Toggle the file-explorer sidebar (`:sidebar`).
    Sidebar,
    /// List external tools and their installed state (`:Mason`).
    Mason,
    /// Open the manual, optionally jumping to a topic (`:help [topic]`).
    Help(Option<String>),
    /// Open the git status view (`:Git`).
    GitStatus,
    /// Stage the hunk under the cursor (`:GitStageHunk`).
    GitStageHunk,
    /// Show the staged diff, where `u` unstages a hunk (`:GitStaged`).
    GitStaged,
    /// Compose a commit message (`:GitCommit`).
    GitCommit,
    /// Push to the remote, after confirmation (`:GitPush`).
    GitPush,
    /// Pull from the remote, after confirmation (`:GitPull`).
    GitPull,
    /// Save the open files and window layout for this project (`:SessionSave`).
    SessionSave,
    /// Reopen this project's saved session (`:SessionRestore`).
    SessionRestore,
    GitsignsToggle,
    TodoList,
    Trouble,
    Themes,
    /// Resize the sidebar to N columns (`:Sidebar resize N`).
    SidebarResize(u16),
    /// Re-read highlight queries from disk and rebuild every engine
    /// (`:SyntaxReload`), so editing a query does not need a restart.
    SyntaxReload,
    /// Side-by-side diff of the active file against HEAD (`:Diffview`).
    Diffview,
    /// Save an image of the screen (`:screenshot [path]`). `None` picks the
    /// next free `ruster-NNN.png` in the working directory.
    Screenshot(Option<String>),
    /// `:16` — jump to a line. `None` is `:$`, the last line.
    GotoLine(Option<usize>),
    Hover,
    /// Toggle the Noice notification-stack panel (`:Noice`).
    NoicePanel,
    /// Open the Noice split history buffer (`:Noice split` / `:Noice history`).
    NoiceSplit,
    /// Queue a popup notification (`:Noice popup`).
    NoicePopup,
    /// Open a file by path (`:e path` / `:edit path`).
    OpenFile(String),
    /// Debug actions.
    DebugStart,
    DebugContinue,
    DebugNext,
    DebugStepIn,
    DebugStepOut,
    DebugStop,
    DebugToggleBreakpoint,
}

/// Parse a general schema-backed `:set` command. Accepts:
/// - `key?`             — show current value (any type)
/// - `key&`             — reset to default
/// - `key=value`        — set any option to a parsed literal
/// - `nokey`            — set a boolean option to false
/// - `key!`             — toggle a boolean option
/// - `key` (bool)       — set true
/// - `key` (non-bool)   — show current value (same as `key?`)
fn parse_set_general(arg: &str) -> Result<CmdAction, String> {
    let tok = arg.trim();
    if tok.is_empty() {
        return Err("Usage: :set [no]key[!?&=value]".to_string());
    }

    if let Some(k) = tok.strip_suffix('?') {
        let k = k.trim();
        if k.is_empty() {
            return Err("Usage: :set key? — display a setting value".to_string());
        }
        let _ = ruster_lua::schema::spec_by_key(k)
            .ok_or_else(|| format!("Unknown option: {k}"))?;
        return Ok(CmdAction::ShowSetting(k.to_string()));
    }

    if let Some(k) = tok.strip_suffix('&') {
        let k = k.trim();
        if k.is_empty() {
            return Err("Usage: :set key& — reset a setting to default".to_string());
        }
        let _ = ruster_lua::schema::spec_by_key(k)
            .ok_or_else(|| format!("Unknown option: {k}"))?;
        return Ok(CmdAction::ResetSetting(k.to_string()));
    }

    let (key, named_val) = if let Some(rest) = tok.strip_suffix('!') {
        let k = rest.trim();
        let spec = ruster_lua::schema::spec_by_key(k)
            .ok_or_else(|| format!("Unknown option: {k}"))?;
        if !matches!(spec.kind, SettingKind::Bool) {
            return Err(format!("{k}: only boolean options support toggle (!)"));
        }
        (k.to_string(), SetNamedVal::Toggle)
    } else if let Some(rest) = tok.strip_prefix("no") {
        let k = rest.trim();
        let spec = ruster_lua::schema::spec_by_key(k)
            .ok_or_else(|| format!("Unknown option: {k}"))?;
        if !matches!(spec.kind, SettingKind::Bool) {
            return Err(format!("{k}: only boolean options support the 'no' prefix"));
        }
        (k.to_string(), SetNamedVal::Exact(SettingValue::Bool(false)))
    } else if let Some((k, v)) = tok.split_once('=') {
        let k = k.trim();
        let spec = ruster_lua::schema::spec_by_key(k)
            .ok_or_else(|| format!("Unknown option: {k}"))?;
        let parsed = spec.kind.parse_value(v)?;
        (k.to_string(), SetNamedVal::Exact(parsed))
    } else {
        let spec = ruster_lua::schema::spec_by_key(tok)
            .ok_or_else(|| format!("Unknown option: {tok}"))?;
        match spec.kind {
            SettingKind::Bool => {
                (tok.to_string(), SetNamedVal::Exact(SettingValue::Bool(true)))
            }
            _ => return Ok(CmdAction::ShowSetting(tok.to_string())),
        }
    };

    Ok(CmdAction::SetNamed(key, named_val))
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
    ("term", "open an embedded terminal"),
    ("config-errors", "show config load/validation errors"),
    ("settings", "open the settings page"),
    ("Dired", "file explorer"),
    ("Files", "find files"),
    ("fmt", "format buffer"),
    ("callers", "incoming calls (call hierarchy)"),
    ("callees", "outgoing calls (call hierarchy)"),
    ("set editmode emacs", "switch to Emacs (modeless) editing"),
    ("set editmode neovim", "switch to Neovim (modal) editing"),
    ("set number", "show absolute line numbers"),
    ("set relativenumber", "show relative line numbers"),
    ("e", "open file by path"),
    ("edit", "open file by path (alias)"),
];

/// The which-key continuations shown after a `Ctrl-w` prefix.
// --- Space-leader key tree (LazyVim style) ---

#[derive(Clone, Copy)]
enum LeaderAction {
    Trouble,
    // Git — the whole Phase 7 porcelain had no discoverable route.
    GitStatus,
    GitCommit,
    Diffview,
    GitStaged,
    GitStageHunk,
    GitPush,
    GitPull,
    GitsignsToggle,
    // Surfaces added in Phases 6-7 that were only reachable by typing.
    Mason,
    Help,
    Themes,
    TodoList,
    NoicePanel,
    SessionSave,
    SessionRestore,
    DebugStart,
    DebugToggleBreakpoint,
    DebugContinue,
    DebugStepOver,
    DebugStepIn,
    DebugStepOut,
    DebugStop,
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
    BufferDelete,
    Terminal,
    Settings,
    ToggleNumber,
    ToggleRelative,
    Grep,
    Build,
    Test,
    Tasks,
    Dashboard,
    Messages,
    Projects,
    Sidebar,
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
    ('b', LeaderNode::Action("build", LeaderAction::Build)),
    ('t', LeaderNode::Action("test", LeaderAction::Test)),
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

static BUFFER_GROUP: &[(char, LeaderNode)] = &[
    ('b', LeaderNode::Action("buffers", LeaderAction::Buffers)),
    ('d', LeaderNode::Action("delete buffer", LeaderAction::BufferDelete)),
];

static SEARCH_GROUP: &[(char, LeaderNode)] = &[
    ('f', LeaderNode::Action("files", LeaderAction::Files)),
    ('g', LeaderNode::Action("grep (ripgrep)", LeaderAction::Grep)),
    ('s', LeaderNode::Action("document symbols", LeaderAction::DocumentSymbol)),
    ('d', LeaderNode::Action("diagnostics", LeaderAction::Diagnostics)),
];

static OPEN_GROUP: &[(char, LeaderNode)] = &[
    ('d', LeaderNode::Action("dashboard", LeaderAction::Dashboard)),
    ('t', LeaderNode::Action("terminal", LeaderAction::Terminal)),
    ('s', LeaderNode::Action("settings", LeaderAction::Settings)),
    ('e', LeaderNode::Action("explorer (dired)", LeaderAction::Explorer)),
    ('m', LeaderNode::Action("messages", LeaderAction::Messages)),
    ('r', LeaderNode::Action("run task", LeaderAction::Tasks)),
    ('h', LeaderNode::Action("help", LeaderAction::Help)),
    ('M', LeaderNode::Action("mason (tools)", LeaderAction::Mason)),
    ('T', LeaderNode::Action("themes", LeaderAction::Themes)),
    ('n', LeaderNode::Action("notifications", LeaderAction::NoicePanel)),
];

/// Git. Phase 7 built a porcelain reachable only by typing its commands; this
/// is the route a user can find by pressing `SPC` and looking.
static GIT_GROUP: &[(char, LeaderNode)] = &[
    ('g', LeaderNode::Action("status", LeaderAction::GitStatus)),
    ('c', LeaderNode::Action("commit", LeaderAction::GitCommit)),
    ('d', LeaderNode::Action("diff vs HEAD", LeaderAction::Diffview)),
    ('S', LeaderNode::Action("staged diff", LeaderAction::GitStaged)),
    ('s', LeaderNode::Action("stage hunk", LeaderAction::GitStageHunk)),
    ('p', LeaderNode::Action("push", LeaderAction::GitPush)),
    ('F', LeaderNode::Action("pull", LeaderAction::GitPull)),
    ('t', LeaderNode::Action("toggle signs", LeaderAction::GitsignsToggle)),
];

/// Sessions: save and restore what was open.
static SESSION_GROUP: &[(char, LeaderNode)] = &[
    ('s', LeaderNode::Action("save session", LeaderAction::SessionSave)),
    ('r', LeaderNode::Action("restore session", LeaderAction::SessionRestore)),
];

static PROJECT_GROUP: &[(char, LeaderNode)] = &[
    ('p', LeaderNode::Action("switch project", LeaderAction::Projects)),
];

static TROUBLE_GROUP: &[(char, LeaderNode)] = &[
    ('x', LeaderNode::Action("problem list", LeaderAction::Trouble)),
    ('t', LeaderNode::Action("todo markers", LeaderAction::TodoList)),
];

static DEBUG_GROUP: &[(char, LeaderNode)] = &[
    ('d', LeaderNode::Action("start debugging", LeaderAction::DebugStart)),
    ('b', LeaderNode::Action("toggle breakpoint", LeaderAction::DebugToggleBreakpoint)),
    ('c', LeaderNode::Action("continue", LeaderAction::DebugContinue)),
    ('n', LeaderNode::Action("step over", LeaderAction::DebugStepOver)),
    ('i', LeaderNode::Action("step into", LeaderAction::DebugStepIn)),
    ('o', LeaderNode::Action("step out", LeaderAction::DebugStepOut)),
    ('q', LeaderNode::Action("stop debugging", LeaderAction::DebugStop)),
];

static UI_GROUP: &[(char, LeaderNode)] = &[
    ('n', LeaderNode::Action("toggle line numbers", LeaderAction::ToggleNumber)),
    ('r', LeaderNode::Action("toggle relative numbers", LeaderAction::ToggleRelative)),
];

static LEADER_ROOT: &[(char, LeaderNode)] = &[
    (',', LeaderNode::Action("settings", LeaderAction::Settings)),
    ('e', LeaderNode::Action("toggle sidebar", LeaderAction::Sidebar)),
    ('w', LeaderNode::Group("windows", WINDOW_GROUP)),
    ('f', LeaderNode::Group("find", FIND_GROUP)),
    ('b', LeaderNode::Group("buffers", BUFFER_GROUP)),
    ('s', LeaderNode::Group("search", SEARCH_GROUP)),
    ('c', LeaderNode::Group("code", CODE_GROUP)),
    ('o', LeaderNode::Group("open", OPEN_GROUP)),
    ('p', LeaderNode::Group("project", PROJECT_GROUP)),
    ('d', LeaderNode::Group("debug", DEBUG_GROUP)),
    ('x', LeaderNode::Group("diagnostics", TROUBLE_GROUP)),
    ('g', LeaderNode::Group("git", GIT_GROUP)),
    ('S', LeaderNode::Group("session", SESSION_GROUP)),
    ('u', LeaderNode::Group("ui / toggle", UI_GROUP)),
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
fn leader_whichkey(seq: &[char]) -> Option<(String, Vec<ruster_render::WhichKeyEntry>)> {
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
            ruster_render::WhichKeyEntry { key: k.to_string(), desc }
        })
        .collect();
    Some((title, rows))
}

/// The which-key content for the `g` menu (LazyVim-style goto prefix).
fn g_whichkey() -> (String, Vec<ruster_render::WhichKeyEntry>) {
    let e = |key: &str, desc: &str| ruster_render::WhichKeyEntry { key: key.to_string(), desc: desc.to_string() };
    (
        "g".to_string(),
        vec![
            e("d", "go to definition"),
            e("r", "references"),
            e("h", "hover"),
            e("g", "top of buffer"),
            e("-", "older change (undo-tree time)"),
            e("+", "newer change (undo-tree time)"),
        ],
    )
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

    /// Per-buffer tree-sitter syntax engines, created lazily the first time a
    /// buffer with a supported filetype is rendered. Buffers without a supported
    /// language (or without a file path) simply have no entry and render plain.
    syntax: std::collections::HashMap<BufferId, SyntaxEngine>,
    /// Buffers we've already attempted to build a syntax engine for, so an
    /// unsupported filetype isn't retried every frame.
    syntax_tried: std::collections::HashSet<BufferId>,
    /// The buffer revision each engine was last parsed at, so an unchanged
    /// buffer is not re-parsed every frame.
    syntax_revision: std::collections::HashMap<BufferId, u64>,
    /// How many reparses have actually run. The dirty check is a performance
    /// guard, and a guard whose effect nothing observes is one that can be
    /// removed without any test noticing — as a mutation of the condition
    /// demonstrated.
    syntax_reparses: u64,
    /// Previous frame's watched state; see [`WatchedState`].
    watched: WatchedState,
    /// What Lua's read-only queries see; see [`QuerySnapshot`].
    query_snapshot: Rc<RefCell<QuerySnapshot>>,
    /// Background `git status` results, for the branch and counts a statusline
    /// wants without the user having opened `:Git`.
    /// `None` means the worker ran and found nothing — outside a repository,
    /// or git missing. It still has to report back, or the in-flight guard
    /// below would latch on and stop polling for the rest of the session.
    git_status_rx: std::sync::mpsc::Receiver<Option<ruster_git::Status>>,
    git_status_tx: std::sync::mpsc::Sender<Option<ruster_git::Status>>,
    /// When the last background `git status` was started, and whether one is
    /// still running. Without the in-flight guard a slow repository would have
    /// a new process spawned every tick while the previous ones piled up.
    git_status_polled: Option<std::time::Instant>,
    git_status_in_flight: bool,
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
    whichkey_cache: Option<(String, Vec<ruster_render::WhichKeyEntry>)>,
    anim_clock: std::time::Instant,
    /// When the current leader sequence started, so the which-key panel only
    /// pops after `Config.timeoutlen` (unless already visible).
    leader_since: Option<std::time::Instant>,
    /// Active flash jump mode state, if any.
    pub flash: Option<FlashState>,
    /// Window geometry from the last rendered frame, for mouse hit-testing.
    last_layout: Vec<WindowLayout>,
    /// Noice notification manager.
    pub notify: NotificationManager,
    /// Active floating picker (buffer list, file finder, ...), if any.
    picker: Option<PickerState>,
    /// Streaming results for the active picker (`:Files` walk, `:Rg` output),
    /// drained into the picker each frame. Backend-agnostic (polled in render).
    pending_results: Option<std::sync::mpsc::Receiver<PickerItem>>,
    /// Cached highlighted contents of the file currently shown in the picker
    /// preview, so it isn't re-read and re-parsed every frame.
    preview_cache: Option<(PathBuf, Vec<StyledLine>)>,
    /// Per-buffer state for dired file-explorer buffers.
    dired: DiredState,
    /// The in-progress file operation (create/rename/delete), driven by both
    /// dired and the sidebar.
    file_prompt: Option<FilePrompt>,
    /// Language server manager (one server per language).
    /// Language servers, document sync, diagnostics and in-flight requests.
    lsp: crate::lsp_state::LspState<LspAction>,
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
    /// Embedded terminals, keyed by their buffer id (Phase 4).
    terminals: std::collections::HashMap<BufferId, TerminalSession>,
    /// When true, keystrokes are forwarded to the active terminal's PTY rather
    /// than the editing layer. `Ctrl-\` defocuses; `i`/`a`/Enter re-focuses.
    terminal_focused: bool,
    /// Config load/validation problems, shown to the user (non-fatal; invalid
    /// values fall back to their defaults). Viewable via `:config-errors`.
    config_errors: Vec<String>,
    /// The Settings page (`:settings`), when open. Captures input like a picker.
    settings: Option<SettingsState>,
    /// When a bare `g` was pressed in idle Normal mode: the moment it started, so
    /// the which-key `g` menu can appear after `timeoutlen`.
    g_pending: Option<std::time::Instant>,
    /// Guard so replaying `g`-motions into the vim layer doesn't re-trigger the
    /// `g` menu.
    g_replaying: bool,
    /// When `]` or `[` was pressed in idle Normal mode: the pending bracket, so
    /// `]q`/`[q` step the quickfix list (any other key replays the motion).
    bracket_pending: Option<char>,
    /// The project root (where the .git / ruster.toml / etc. lives), if detected.
    project_root: Option<PathBuf>,
    /// The shared quickfix list (`:copen`/`:cnext`/`:cprev`, `]q`/`[q`).
    quickfix: QuickfixList,
    /// A running build/test command's output stream, drained per frame.
    runner_rx: Option<std::sync::mpsc::Receiver<crate::runner::RunnerMsg>>,
    /// The results buffer the current run streams into.
    runner_buf: Option<BufferId>,
    /// The project root the run was launched from (to resolve diagnostic paths),
    /// the accumulated output for post-run parsing, and what kind of run it is.
    runner_root: PathBuf,
    runner_output: String,
    runner_kind: RunnerKind,
    /// Folds and the last parsed status for the `:Git` view.
    git_status: crate::git_status::GitStatusState,
    /// A shell command awaiting the user's confirmation. `Some` is what
    /// distinguishes ruster's own confirmation dialog from a plugin's.
    ///
    /// One slot for both `:Mason` installs and git push/pull: they differ only
    /// in wording and which results buffer they stream to, and a second copy of
    /// the mechanism would be a second place for it to go wrong.
    pending_confirm: Option<PendingConfirm>,
    /// Per-file gutter signs from the last test run (✓/✗), merged with diagnostics.
    result_signs: std::collections::HashMap<PathBuf, ruster_render::SignsView>,
    /// Per-buffer git hunks for the gutter, and their background workers.
    git: crate::git_gutter::GitGutter,
    /// An open modal form, if any.
    dialog: Option<DialogState>,
    /// The theme in force before a theme picker opened, restored on cancel.
    /// `Some` only while that picker is up, which is also how the picker knows
    /// to preview as the selection moves.
    theme_before_preview: Option<String>,
    /// The aggregated problem list, and its pinned buffer.
    trouble: TroubleState,
    trouble_buf: Option<BufferId>,
    /// The file-explorer side panel.
    sidebar: SidebarState,
    /// The message log for editor/plugin messages.
    messages: ruster_core::message::MessageLog,
    /// The pinned messages buffer, once created.
    messages_buf: Option<BufferId>,
    /// Active filters for the messages buffer display.
    messages_filter_source: Option<ruster_core::message::MessageSource>,
    messages_filter_level: Option<ruster_core::message::MessageLevel>,
    /// State for cmdline path completion (Tab/Shift-Tab cycling).
    cmdline_completion: Option<CmdlineCompletion>,
    /// The debug session and its breakpoints.
    debug: crate::debug_state::DebugState,
    /// When true, the Noice notification-stack panel is shown as a right-side bar.
    pub show_noice_panel: bool,
}

/// A command the user has been asked to confirm.
struct PendingConfirm {
    cmd: String,
    kind: RunnerKind,
    /// The confirming button's label, e.g. `Install` or `Push`.
    verb: String,
}

/// What a background run is, so its output is parsed appropriately on completion.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RunnerKind {
    Build,
    Test,
    Task,
    /// A `:Mason` install, streamed like any other command run.
    Install,
    /// A git command the user confirmed — push or pull.
    Git,
}

/// The active editing paradigm. Neovim is modal; Emacs is modeless.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditMode {
    Neovim,
    Emacs,
}

impl App {
    pub fn new(content: String, file_path: PathBuf) -> Self {
        let ws = if file_path.as_os_str().is_empty() {
            Rc::new(RefCell::new(Workspace::scratch()))
        } else {
            Rc::new(RefCell::new(Workspace::from_file(file_path.clone(), content.clone())))
        };
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
        // The notification manager does not exist yet, so hold any query
        // problems until it does rather than dropping them.
        let mut query_warnings: Vec<String> = Vec::new();
        if let Ok(engine) = SyntaxEngine::new(&normalized, ext) {
            query_warnings.extend(engine.warnings().iter().cloned());
            syntax.insert(initial_buffer, engine);
        }
        let mut syntax_tried = std::collections::HashSet::new();
        syntax_tried.insert(initial_buffer);
        let (git_status_tx_init, git_status_rx_init) = std::sync::mpsc::channel();
        let query_snapshot: Rc<RefCell<QuerySnapshot>> =
            Rc::new(RefCell::new(QuerySnapshot::default()));
        let mut lua = LuaRuntime::new().unwrap_or_else(|e| {
            eprintln!("Lua init failed: {}", e);
            panic!("Lua init required");
        });
        // On first run we generate a default `config.lua` (the declarative,
        // Settings-page-managed file); `init.lua` is optional user scripting
        // loaded *after* it, so it can override settings.
        let mut config_errors: Vec<String> = Vec::new();
        // Skip all config-dir file IO under test so the suite never touches the
        // user's real ~/.config/ruster.
        if !cfg!(test) {
        if let Some(dir) = ruster_config_dir() {
            let config_path = dir.join("config.lua");
            let init_path = dir.join("init.lua");
            if !config_path.exists() {
                let _ = std::fs::create_dir_all(&dir);
                if let Err(e) =
                    std::fs::write(&config_path, ruster_lua::schema::generate_default_config())
                {
                    eprintln!("ruster: could not write {}: {}", config_path.display(), e);
                }
            }
            if config_path.exists() {
                if let Err(e) = lua.load_init(&config_path) {
                    config_errors.push(e);
                }
            }
            if init_path.exists() {
                if let Err(e) = lua.load_init(&init_path) {
                    config_errors.push(e);
                }
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
            // Read-only queries, served from a snapshot the frame loop keeps
            // current. See `QuerySnapshot` for why this is a push rather than
            // the pull the other callbacks use.
            let snap = query_snapshot.clone();
            let s1 = snap.clone();
            let s2 = snap.clone();
            let s3 = snap.clone();
            lua.set_query_callbacks(ruster_lua::runtime::QueryCallbacks {
                buf_path: Box::new(move || s1.borrow().path.clone()),
                filetype: Box::new(move || s2.borrow().filetype.clone()),
                diagnostics: Box::new(move || s3.borrow().diagnostics.clone()),
                git_status: Box::new(move || {
                    let s = snap.borrow();
                    (s.branch.clone(), s.staged, s.unstaged)
                }),
            });
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
        let mut lsp: crate::lsp_state::LspState<LspAction> = crate::lsp_state::LspState::new();
        for (lang, cmd, args) in lua.lsp_servers() {
            lsp.set_server(&lang, ruster_lsp::ServerConfig { cmd, args });
        }
        let (mut config, verrs) = lua.config_validated();
        for e in verrs {
            config_errors.push(e.to_string());
        }
        // Generate built-in themes on first run, then apply the selected palette
        // (`general.theme`) from themes/<name>.lua, falling back to a built-in.
        if !cfg!(test) {
        if let Some(dir) = ruster_config_dir() {
            let themes_dir = dir.join("themes");
            let _ = std::fs::create_dir_all(&themes_dir);
            for (name, theme) in ruster_lua::config::builtin_themes() {
                let path = themes_dir.join(format!("{name}.lua"));
                if !path.exists() {
                    let _ = std::fs::write(&path, theme.to_lua());
                }
            }
            // Warn on an unknown theme name (resolve falls back to default).
            let known = ruster_lua::config::builtin_themes().iter().any(|(n, _)| *n == config.theme)
                || themes_dir.join(format!("{}.lua", config.theme)).exists();
            if !known && !config.theme.is_empty() && config.theme != "default" {
                config_errors.push(format!(
                    "general.theme: unknown theme {:?} → using default",
                    config.theme
                ));
            }
            // Resolve the theme roles and layer per-element color overrides.
            config.colors = resolve_theme_colors(&lua, &config.theme, &config.color_overrides);
        }
        }
        // Apply EditorConfig overrides (unless disabled via general.editorconfig).
        if config.editorconfig {
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
        }
        // Install per-language syntax colours from config, then recolour the
        // initial buffer's engine (built before the config was loaded).
        ruster_syntax::set_syntax_overrides(syntax_overrides_to_colors(&config.syntax_overrides));
        if !config.syntax_overrides.is_empty() {
            let text = ws.borrow().buffer().to_string();
            for engine in syntax.values_mut() {
                engine.recolor(&text);
            }
        }
        ws.borrow_mut().set_active_indent_width(config.tabstop);
        // A brand-new file adopts the configured default line ending.
        if config.line_ending == "crlf" && !file_path.exists() {
            let mut w = ws.borrow_mut();
            let id = w.active_buffer();
            if let Some(doc) = w.buffers.get_mut(id) {
                doc.line_ending = ruster_core::document::LineEnding::Crlf;
            }
        }
        // Startup editing paradigm + dired default from config.
        let editmode = if config.editmode == "emacs" {
            lua.set_editmode("emacs");
            EditMode::Emacs
        } else {
            EditMode::Neovim
        };
        let dired_show_hidden = config.dired_show_hidden;
        let timer = FrameTimer::new();
        let cursor_anim = CursorAnim::new();
        let project_root = ruster_project::project_root(&file_path);
        // Fall back to the most recent project if no root found.
        let project_root = project_root.or_else(|| {
            ruster_config_dir().and_then(|d| {
                ruster_project::recent_projects(&d).into_iter().next().filter(|p| p.exists())
            })
        });
        if let Some(ref state_dir) = ruster_config_dir() {
            if let Some(ref root) = project_root {
                ruster_project::record_recent(state_dir, root, 30);
            }
        }
        let git_signs_init = config.git_signs;
        let mut notify = NotificationManager::new(ruster_notify::NoiceSettings {
            mini_enabled: config.noice.mini_enabled,
            notify_enabled: config.noice.notify_enabled,
            split_enabled: config.noice.split_enabled,
            info_timeout: std::time::Duration::from_millis(config.noice.info_timeout_ms),
            success_timeout: std::time::Duration::from_millis(config.noice.success_timeout_ms),
            warning_timeout: std::time::Duration::from_millis(config.noice.warning_timeout_ms),
            max_history: config.noice.max_history,
        });
        if !config_errors.is_empty() {
            notify.push(Notification::new(
                ruster_core::message::MessageLevel::Warning,
                ruster_core::message::MessageSource::System,
                format!(
                    "config: {} problem(s) — {} (:config-errors for all)",
                    config_errors.len(),
                    config_errors[0]
                )
            ));
        }
        let mut app = App {
            ws, vim, renderer,
            should_quit: false, syntax, syntax_tried,
            syntax_revision: std::collections::HashMap::new(),
            syntax_reparses: 0,
            watched: WatchedState::default(),
            query_snapshot,
            git_status_rx: git_status_rx_init,
            git_status_tx: git_status_tx_init,
            git_status_polled: None,
            git_status_in_flight: false, lua, config, timer, notify,
            has_smooth_cursor: false, cursor_anim, pending_ctrl_w: false, picker: None,
            leader_pending: None,
            pending_results: None,
            preview_cache: None,
            whichkey_anim: 0.0,
            whichkey_cache: None,
            anim_clock: std::time::Instant::now(),
            leader_since: None,
            flash: None,
            last_layout: Vec::new(),
            dired: DiredState::new(dired_show_hidden),
            file_prompt: None,
            lsp,
            hover: None,
            snippets: {
                let mut s = ruster_core::snippets::SnippetSet::builtin();
                if let Some(dir) = ruster_config_dir() {
                    s.load_dir(&dir.join("snippets"));
                }
                s
            },
            snippet_stops: Vec::new(),
            pending_format_save: false,
            macros: std::collections::HashMap::new(),
            macro_recording: None,
            pending_macro: None,
            replaying: false,
            editmode,
            emacs: ruster_core::emacs::EmacsState::new(),
            emacs_ctrl_x: false,
            emacs_isearch: None,
            terminals: std::collections::HashMap::new(),
            terminal_focused: false,
            config_errors,
            settings: None,
            g_pending: None,
            g_replaying: false,
            bracket_pending: None,
            project_root,
            quickfix: QuickfixList::default(),
            runner_rx: None,
            runner_buf: None,
            runner_root: PathBuf::new(),
            runner_output: String::new(),
            runner_kind: RunnerKind::Build,
            git_status: crate::git_status::GitStatusState::new(),
            pending_confirm: None,
            result_signs: std::collections::HashMap::new(),
            git: crate::git_gutter::GitGutter::new(git_signs_init),
            dialog: None,
            theme_before_preview: None,
            trouble: TroubleState::new(),
            trouble_buf: None,
            sidebar: SidebarState::new(),
            messages: ruster_core::message::MessageLog::new(),
            messages_buf: None,
            messages_filter_source: None,
            messages_filter_level: None,
            cmdline_completion: None,
            debug: crate::debug_state::DebugState::new(),
            show_noice_panel: false,
        };
        // Create background buffers (pinned, not navigated to).
        let initial = app.ws.borrow().active_buffer();
        app.refresh_git_hunks(initial);
        app.ensure_dashboard_buffer();
        app.ensure_messages_buffer();
        for w in query_warnings {
            push_query_warning(&mut app.notify, w);
        }
        // Auto-open sidebar if configured and a project root is detected.
        if app.config.sidebar_auto_open && app.project_root.is_some() {
            app.toggle_sidebar();
        }
        // Restoring is quiet on startup: a warning about a session that was
        // never saved is noise on every first run in a project.
        if app.config.session_autoload {
            app.restore_session(true);
        }
        app
    }

    /// The configured GUI font (`gui_font`), for the renderer to load.
    pub fn gui_font(&self) -> Option<String> {
        self.config.gui_font.clone()
    }

    /// GUI metrics + theme built from config, for the raylib renderer.
    pub fn gui_config(&self) -> ruster_render::GuiConfig {
        let c = &self.config;
        let col = |rgb: ruster_lua::config::Rgb| ruster_render::Color::Rgb(rgb.r, rgb.g, rgb.b);
        ruster_render::GuiConfig {
            font_size: c.font_size as i32,
            line_height: c.line_height as i32,
            padding_x: c.padding_x as i32,
            padding_y: c.padding_y as i32,
            window_width: c.window_width as i32,
            window_height: c.window_height as i32,
            target_fps: c.target_fps as i32,
            cursor_kind: if c.cursor_kind == "bar" {
                ruster_render::CursorKind::Bar
            } else {
                ruster_render::CursorKind::Block
            },
            theme: ruster_render::Theme {
                bg: col(c.colors.bg),
                fg: col(c.colors.fg),
                gutter: col(c.colors.gutter),
                gutter_bg: col(c.colors.gutter_bg),
                cursor_bg: col(c.colors.cursor_bg),
                cursor_fg: col(c.colors.cursor_fg),
                selection_bg: col(c.colors.selection_bg),
                selection_fg: col(c.colors.selection_fg),
                divider: col(c.colors.divider),
                statusline_fg: col(c.colors.statusline_fg),
                statusline_bg: col(c.colors.statusline_bg),
                mode_normal_bg: col(c.colors.mode_normal_bg),
                mode_normal_fg: col(c.colors.mode_normal_fg),
                mode_insert_bg: col(c.colors.mode_insert_bg),
                mode_insert_fg: col(c.colors.mode_insert_fg),
                mode_visual_bg: col(c.colors.mode_visual_bg),
                mode_visual_fg: col(c.colors.mode_visual_fg),
                mode_cmdline_bg: col(c.colors.mode_cmdline_bg),
                mode_cmdline_fg: col(c.colors.mode_cmdline_fg),
                mode_emacs_bg: col(c.colors.mode_emacs_bg),
                mode_emacs_fg: col(c.colors.mode_emacs_fg),
                accent: col(c.colors.accent),
                accent_fg: col(c.colors.accent_fg),
                whichkey_bg: col(c.colors.whichkey_bg),
                whichkey_fg: col(c.colors.whichkey_fg),
                whichkey_key: col(c.colors.whichkey_key),
                cmdline_bg: col(c.colors.cmdline_bg),
                cmdline_fg: col(c.colors.cmdline_fg),
            },
        }
    }

    fn theme_palette(&self) -> ruster_render::Theme {
        let c = &self.config;
        let col = |rgb: ruster_lua::config::Rgb| ruster_render::Color::Rgb(rgb.r, rgb.g, rgb.b);
        ruster_render::Theme {
            bg: col(c.colors.bg),
            fg: col(c.colors.fg),
            gutter: col(c.colors.gutter),
            gutter_bg: col(c.colors.gutter_bg),
            cursor_bg: col(c.colors.cursor_bg),
            cursor_fg: col(c.colors.cursor_fg),
            selection_bg: col(c.colors.selection_bg),
            selection_fg: col(c.colors.selection_fg),
            divider: col(c.colors.divider),
            statusline_fg: col(c.colors.statusline_fg),
            statusline_bg: col(c.colors.statusline_bg),
            mode_normal_bg: col(c.colors.mode_normal_bg),
            mode_normal_fg: col(c.colors.mode_normal_fg),
            mode_insert_bg: col(c.colors.mode_insert_bg),
            mode_insert_fg: col(c.colors.mode_insert_fg),
            mode_visual_bg: col(c.colors.mode_visual_bg),
            mode_visual_fg: col(c.colors.mode_visual_fg),
            mode_cmdline_bg: col(c.colors.mode_cmdline_bg),
            mode_cmdline_fg: col(c.colors.mode_cmdline_fg),
            mode_emacs_bg: col(c.colors.mode_emacs_bg),
            mode_emacs_fg: col(c.colors.mode_emacs_fg),
            accent: col(c.colors.accent),
            accent_fg: col(c.colors.accent_fg),
            whichkey_bg: col(c.colors.whichkey_bg),
            whichkey_fg: col(c.colors.whichkey_fg),
            whichkey_key: col(c.colors.whichkey_key),
            cmdline_bg: col(c.colors.cmdline_bg),
            cmdline_fg: col(c.colors.cmdline_fg),
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

        // The Settings page captures input, except ':' (opens the cmdline for
        // :w/:q) when not mid-edit, and except while the cmdline is already open.
        if self.settings.is_some() && self.vim.mode != VimMode::Cmdline {
            let editing = self.settings.as_ref().is_some_and(|s| s.is_editing());
            // ':' (when not mid-edit) falls through to open the cmdline for :w/:q.
            let colon = matches!(ck.code, KeyCode::Char(':')) && !editing;
            if !colon {
                self.handle_settings_key(ck);
                return;
            }
        }

        // Any key dismisses an open hover popup (and still acts).
        self.hover = None;

        // A dired file-operation prompt captures input until confirmed/cancelled.
        // A modal dialog is modal: it takes every key until it closes.
        if self.dialog.is_some() {
            self.handle_dialog_key(ck);
            return;
        }
        if self.file_prompt.is_some() {
            self.handle_file_prompt_key(ck);
            return;
        }

        // A pending Space-leader sequence captures the next key.
        if self.leader_pending.is_some() {
            self.handle_leader_key(ck);
            return;
        }

        // LazyVim-style `g` menu: intercept a bare `g` in idle Normal mode so the
        // next key is a goto command (`gd`/`gr`/`gh`) or a replayed native
        // g-motion (`gg`/`g-`/`g+`). Skipped while replaying, in dired/terminal,
        // in Emacs mode, or mid vim sequence.
        if !self.g_replaying
            && self.editmode == EditMode::Neovim
            && self.vim.is_normal_idle()
            && !self.active_is_dired()
            && self.active_terminal_buffer().is_none()
        {
            if self.g_pending.take().is_some() {
                self.handle_g_key(ck);
                return;
            }
            if matches!(ck.code, KeyCode::Char('g')) {
                self.g_pending = Some(std::time::Instant::now());
                return;
            }
            // `]q`/`[q` step the quickfix list; any other key after `]`/`[`
            // replays the native bracket motion into the vim layer.
            if let Some(open) = self.bracket_pending.take() {
                if matches!(ck.code, KeyCode::Char('q')) {
                    if open == ']' { self.quickfix_next() } else { self.quickfix_prev() }
                } else if matches!(ck.code, KeyCode::Char('h')) {
                    self.jump_hunk(open == ']');
                } else {
                    self.feed_key_to_vim(KeyCode::Char(open));
                    self.feed_key_to_vim(ck.code);
                }
                return;
            }
            if matches!(ck.code, KeyCode::Char(']') | KeyCode::Char('[')) {
                if let KeyCode::Char(c) = ck.code {
                    self.bracket_pending = Some(c);
                }
                return;
            }
        }

        // A focused embedded terminal forwards keys to its PTY. When unfocused,
        // `i`/`a`/Enter re-enter it; anything else falls through to normal
        // handling so window nav and `:` commands still work.
        if let Some(bid) = self.active_terminal_buffer() {
            if self.terminal_focused {
                self.handle_terminal_key(ck, bid);
                return;
            } else if matches!(ck.code, KeyCode::Char('i') | KeyCode::Char('a') | KeyCode::Enter) {
                self.terminal_focused = true;
                return;
            }
        }

        // A focused sidebar captures navigation keys. Unhandled keys (e.g.
        // Space for the leader prefix) fall through to the main handler.
        if self.sidebar.is_open() && self.sidebar.is_focused() && self.handle_sidebar_key(ck) {
            return;
        }

        // Dired claims its action keys, but only while at rest — never while a
        // command-line/search prompt (vim Cmdline or an Emacs isearch) is open,
        // or a search term containing a dired key (e.g. `d` in "docs") would be
        // hijacked. Unclaimed keys fall through to normal handling.
        // The problem list claims Enter/Tab/r/q the same way dired claims its
        // keys; everything else falls through so `:`, `/` and motions still work.
        if self.active_is_git_staged()
            && self.vim.mode == VimMode::Normal
            && self.emacs_isearch.is_none()
            && self.handle_git_staged_key(ck)
        {
            return;
        }
        if self.active_is_git_status()
            && self.vim.mode == VimMode::Normal
            && self.emacs_isearch.is_none()
            && self.handle_git_status_key(ck)
        {
            return;
        }
        if self.active_is_help()
            && self.vim.mode == VimMode::Normal
            && self.emacs_isearch.is_none()
            && self.handle_help_key(ck)
        {
            return;
        }
        if self.active_is_mason()
            && self.vim.mode == VimMode::Normal
            && self.emacs_isearch.is_none()
            && self.handle_mason_key(ck)
        {
            return;
        }
        if self.active_is_trouble()
            && self.vim.mode == VimMode::Normal
            && self.emacs_isearch.is_none()
            && self.handle_trouble_key(ck)
        {
            return;
        }
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
                if dir == FocusDir::Left && self.sidebar.is_open() && !self.sidebar.is_focused() {
                    let before = self.ws.borrow().windows.active();
                    self.ws.borrow_mut().windows.focus(dir);
                    let after = self.ws.borrow().windows.active();
                    if before == after {
                        self.sidebar.set_focused(true);
                    }
                } else {
                    self.ws.borrow_mut().windows.focus(dir);
                }
                return;
            }
        }
        // Ctrl+D: add cursor at next word occurrence.
        if self.vim.mode == VimMode::Normal
            && ck.code == KeyCode::Char('d')
            && ck.modifiers.contains(KeyModifiers::CONTROL)
        {
            let win_id = self.ws.borrow().windows.active();
            let pos = self.ws.borrow().primary_head();
            let text = self.ws.borrow().buffer().to_string();
            let is_word = |c: char| c.is_alphanumeric() || c == '_';
            let chars: Vec<char> = text.chars().collect();
            if pos < chars.len() {
                let start = (0..pos).rev().find(|&i| is_word(chars[i])).unwrap_or(0);
                let end = (pos..chars.len()).find(|&i| !is_word(chars[i])).unwrap_or(chars.len());
                if start < end {
                    let word: String = chars[start..end].iter().collect();
                    let word_len = word.chars().count();
                    let search_from = pos + word_len;
                    if search_from < chars.len() {
                        let text_rest: String = chars[search_from..].iter().collect();
                        if let Some(found) = text_rest.find(&word) {
                            let offset = search_from + found;
                            self.ws.borrow_mut().windows.window_mut(win_id).unwrap().cursors.add_cursor(offset);
                        }
                    }
                }
            }
            return;
        }
        // F-key dispatch for build/test/task and debug.
        match ck.code {
            KeyCode::F(7) => {
                self.run_build();
                return;
            }
            KeyCode::F(6) => {
                self.run_test();
                return;
            }
            KeyCode::F(9) => {
                self.open_task_picker();
                return;
            }
            KeyCode::F(2) => {
                self.debug_toggle_breakpoint();
                return;
            }
            KeyCode::F(5) if !ck.modifiers.contains(crossterm::event::KeyModifiers::SHIFT) => {
                self.debug_continue();
                return;
            }
            KeyCode::F(5) => {
                self.debug_stop();
                return;
            }
            KeyCode::F(10) => {
                self.debug_step_over();
                return;
            }
            KeyCode::F(11) => {
                self.debug_step_in();
                return;
            }
            KeyCode::F(12) => {
                self.debug_step_out();
                return;
            }
            _ => {}
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
                        self.echo(format!("Recording @{}", reg));
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
                        self.echo(format!("Recorded @{} ({} keys)", reg, n));
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

        // Esc in cmdline cancels path completion and restores original input.
        if self.vim.mode == VimMode::Cmdline && key == KeyEvent::Esc {
            if let Some(comp) = self.cmdline_completion.take() {
                self.vim.set_cmdline(&format!("{}{}", comp.prefix, comp.original));
                return;
            }
        }

        // Tab in the cmdline opens the command palette, seeded with the partial.
        if self.vim.mode == VimMode::Cmdline && key == KeyEvent::Tab {
            let raw = self.vim.cmdline_buffer().to_string();
            let trimmed = raw.trim_start_matches(':');

            // If in an :e/:edit command, do path completion
            if trimmed.starts_with("e ") || trimmed.starts_with("edit ") {
                let path_part = trimmed
                    .split_once(' ')
                    .map(|x| x.1)
                    .unwrap_or("")
                    .trim()
                    .to_string();

                if self.cmdline_completion.is_none() {
                    // First Tab press: generate candidates
                    let candidates = self.generate_completion_candidates(&path_part);
                    if candidates.is_empty() {
                        self.echo_warn(format!("No matches for '{}'", path_part));
                        return;
                    }
                    let prefix = raw
                        .split_once(' ')
                        .map(|x| format!("{} ", x.0))
                        .unwrap_or_else(|| ":e ".to_string());
                    self.cmdline_completion = Some(CmdlineCompletion {
                        original: path_part,
                        candidates,
                        index: 0,
                        prefix: prefix.clone(),
                    });
                    if let Some(ref comp) = self.cmdline_completion {
                        let candidate = comp.candidates[0].clone();
                        self.vim.set_cmdline(&format!("{}{}", comp.prefix, candidate));
                    }
                } else if let Some(ref mut comp) = self.cmdline_completion {
                    // Subsequent Tab press: cycle to next candidate
                    comp.index = (comp.index + 1) % comp.candidates.len();
                    let candidate = comp.candidates[comp.index].clone();
                    self.vim.set_cmdline(&format!("{}{}", comp.prefix, candidate));
                }
                return;
            }

            // Otherwise, fall back to command palette
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

        if self.vim.mode == VimMode::Cmdline && key == KeyEvent::BackTab {
            if let Some(_comp) = self.cmdline_completion.take() {
                let path_part = self
                    .vim
                    .cmdline_buffer()
                    .trim_start_matches(':')
                    .split_once(' ')
                    .map(|x| x.1)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let candidates = self.generate_completion_candidates(&path_part);
                if !candidates.is_empty() {
                    let items: Vec<PickerItem> = candidates
                        .iter()
                        .map(|c| {
                            PickerItem::new(
                                c.clone(),
                                PickerAction::OpenPath(std::path::PathBuf::from(c)),
                            )
                        })
                        .collect();
                    self.picker =
                        Some(crate::picker::PickerState::new("path completion", items));
                }
            }
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

        // Digit keys on the dashboard open recent projects by index.
        if self.is_dashboard_active()
            && self.vim.is_normal_idle()
        {
            if let KeyEvent::Char(c) = key {
                if let Some(d) = c.to_digit(10) {
                    if (1..=9).contains(&d) {
                        let recent: Vec<PathBuf> = ruster_config_dir()
                            .map(|d| ruster_project::recent_projects(&d))
                            .unwrap_or_default();
                        if let Some(path) = recent.get(d as usize - 1) {
                            self.open_path(path, None);
                            return;
                        }
                    }
                }
            }
        }

        // Flash jump mode (f replaces inline find).
        if ck.code == KeyCode::Char('f') && ck.modifiers.is_empty() && self.vim.is_normal_idle() {
            let labels = self.compute_flash_labels();
            if labels.is_empty() {
                return;
            }
            self.flash = Some(FlashState {
                labels,
                pending: None,
            });
            return;
        }

        // Flash mode active — intercept or cancel.
        if self.flash.is_some() {
            match ck.code {
                KeyCode::Esc => {
                    self.flash = None;
                    return;
                }
                // A label key is consumed by flash mode — every path here must
                // return, or the key also reaches the Vim state machine below
                // (where `a` would open Insert mode and the next label key
                // would be typed into the buffer).
                KeyCode::Char(c) if c.is_ascii_lowercase() => {
                    let mut fs = self.flash.take().unwrap();
                    match fs.pending {
                        None => {
                            let matching: Vec<FlashLabel> = fs.labels.into_iter()
                                .filter(|l| l.label.starts_with(c))
                                .collect();
                            if matching.is_empty() {
                                return;
                            }
                            if matching.len() == 1 {
                                self.ws.borrow_mut().execute(Action::Move(Motion::To(matching[0].offset)));
                                return;
                            }
                            // Ambiguous — keep the candidates and wait for the
                            // disambiguating second char.
                            fs.labels = matching;
                            fs.pending = Some(c);
                            self.flash = Some(fs);
                        }
                        Some(first) => {
                            let target = format!("{}{}", first, c);
                            if let Some(label) = fs.labels.iter().find(|l| l.label == target) {
                                self.ws.borrow_mut().execute(Action::Move(Motion::To(label.offset)));
                            }
                        }
                    }
                    return;
                }
                _ => {
                    // Cancel and fall through to normal dispatch so the key is replayed.
                    self.flash = None;
                }
            }
        }

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

                    self.cmdline_completion = None;
                    match self.parse_cmdline(&cmd) {
                        Ok(a) => self.apply_cmd(a),
                        Err(e) => { self.echo(e); },
                    }
                }
                other => self.ws.borrow_mut().execute(other),
            }
        }
        if self.vim.mode != prev_mode {
            // Clear cmdline completion when leaving Cmdline mode
            if prev_mode == VimMode::Cmdline {
                self.cmdline_completion = None;
            }
            let mode_str = format!("{:?}", self.vim.mode);
            self.lua.set_mode(&mode_str);
            self.lua.fire_event_str("ModeChanged", &[&mode_str]);
            // Insert is the one mode plugins overwhelmingly care about, and
            // deriving it here rather than making every plugin parse
            // `ModeChanged` keeps that logic in one place.
            if prev_mode == VimMode::Insert {
                self.lua.fire_event_str("InsertLeave", &[&mode_str]);
            }
            if self.vim.mode == VimMode::Insert {
                self.lua.fire_event_str("InsertEnter", &[&mode_str]);
            }
        }
    }

    /// Generate flash jump labels for the visible range of the active window.
    fn compute_flash_labels(&self) -> Vec<FlashLabel> {
        let ws = match self.ws.try_borrow() {
            Ok(w) => w,
            Err(_) => return vec![],
        };
        let win = ws.active_window();
        let buf = ws.buffer();
        let scroll = win.scroll_top;
        let visible_lines = if win.height == 0 { 24 } else { win.height };
        let mut labels = Vec::new();
        let mut label_pool = label_pool_iter();

        for line_idx in 0..visible_lines {
            let buf_line = scroll + line_idx;
            if buf_line >= buf.line_count() { break; }
            let line_start = buf.line_start_char(buf_line);
            let line_end = buf.line_end_char(buf_line);
            let text = buf.slice_string(line_start, line_end);
            // Scan for word boundaries. Indices must be *char* offsets, since
            // line_start is a char offset and Motion::To takes one — walking
            // bytes would skew every label on a line containing non-ASCII text.
            let chars: Vec<char> = text.chars().collect();
            let is_word = |c: char| c.is_alphanumeric() || c == '_';
            let mut pos = 0;
            while pos < chars.len() {
                if is_word(chars[pos]) {
                    let word_start = pos;
                    while pos < chars.len() && is_word(chars[pos]) {
                        pos += 1;
                    }
                    if let Some(label) = label_pool.next() {
                        labels.push(FlashLabel {
                            label,
                            offset: line_start + word_start,
                        });
                    }
                } else {
                    pos += 1;
                }
            }
        }
        labels
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
        self.echo(format!("editmode: {}", name));
    }

    /// Apply a general schema-backed `:set` command. Looks up the option by key,
    /// resolves toggle from the current config value, validates, and rebuilds the
    /// whole Config from the updated settings so every field stays consistent.
    fn set_named_option(&mut self, key: &str, val: SetNamedVal) {
        let spec = match ruster_lua::schema::spec_by_key(key) {
            Some(s) => s,
            None => {
                self.notify.push(Notification::new(
                    ruster_core::message::MessageLevel::Error, ruster_core::message::MessageSource::Echo,
                    format!("E518: Unknown option: {key}"),
                ));
                return;
            }
        };

        // Resolve the value (Toggle needs the current config).
        let value = match val {
            SetNamedVal::Exact(v) => v,
            SetNamedVal::Toggle => {
                let cur = self.config.to_settings().into_iter()
                    .find(|((_g, k), _)| *k == key)
                    .map(|(_, v)| v)
                    .unwrap_or_else(|| spec.default.clone());
                match cur {
                    SettingValue::Bool(b) => SettingValue::Bool(!b),
                    _ => {
                        self.notify.push(Notification::new(
                            ruster_core::message::MessageLevel::Error, ruster_core::message::MessageSource::Echo,
                            format!("E548: {key} cannot be toggled"),
                        ));
                        return;
                    }
                }
            }
        };

        // Validate.
        if let Err(e) = spec.kind.check(&value) {
            self.notify.push(Notification::new(
                ruster_core::message::MessageLevel::Error, ruster_core::message::MessageSource::Echo,
                format!("E474: {key}: {e}"),
            ));
            return;
        }

        // Rebuild config with the new value.
        let mut vals = self.config.to_settings();
        if let Some(pos) = vals.iter_mut().find(|((_g, k), _)| *k == key) {
            pos.1 = value.clone();
        }
        let old_editmode = self.config.editmode.clone();
        self.config = Config::from_settings(&vals);
        self.config.colors = resolve_theme_colors(&self.lua, &self.config.theme, &self.config.color_overrides);

        // If editmode changed, apply the switch.
        if key == "editmode" && self.config.editmode != old_editmode {
            let mode = match self.config.editmode.as_str() {
                "emacs" => EditMode::Emacs,
                _ => EditMode::Neovim,
            };
            self.set_editmode(mode);
        }

        self.notify.push(Notification::new(
            ruster_core::message::MessageLevel::Info, ruster_core::message::MessageSource::Echo,
            format!("{} = {}", key, value.display()),
        ));
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
                _ => { self.echo_warn("C-x undefined".to_string()); },
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
                self.echo("Quit".to_string());
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

    }

    fn handle_mouse_event(&mut self, me: MouseEvent) {
        if me.kind != MouseEventKind::Down(MouseButton::Left)
            || !me.modifiers.contains(KeyModifiers::ALT)
        {
            return;
        }
        if let Some((wid, offset)) = self.buffer_offset_at(me.column, me.row) {
            if let Some(win) = self.ws.borrow_mut().windows.window_mut(wid) {
                win.cursors.add_cursor(offset);
            }
        }
    }

    /// Resolve a screen cell to a buffer offset, using the geometry of the last
    /// rendered frame (`last_layout`) rather than recomputing it.
    ///
    /// `None` when the cell is not over buffer text: window chrome, the sign or
    /// number gutter, the sidebar, or past the buffer's last line. That last case
    /// matters — indexing the rope beyond the final line panics.
    fn buffer_offset_at(
        &self,
        col: u16,
        row: u16,
    ) -> Option<(ruster_core::windows::WindowId, usize)> {
        let w = self.ws.borrow();
        for l in &self.last_layout {
            let Some((text_row, text_col)) = l.text.cell_at(col, row) else {
                continue;
            };
            let doc = w.buffers.get(l.buffer)?;
            let buf_line = l.scroll_top + text_row as usize;
            if buf_line >= doc.buffer.line_count() {
                return None;
            }
            // Clamp into the line's own text, as normal mode does: clicking past
            // the end of a line lands on its last character.
            let content_len = doc.buffer.line_content_len(buf_line);
            let offset = doc.buffer.line_start_char(buf_line)
                + (text_col as usize).min(content_len.saturating_sub(1));
            return Some((l.window, offset));
        }
        None
    }

    /// Begin an Emacs incremental search in the given direction.
    fn start_isearch(&mut self, forward: bool) {
        self.emacs_isearch = Some((String::new(), forward));
    }

    /// Drive an active incremental search: printable keys extend the query and
    /// jump to the next match; `C-s`/`C-r` repeat; `Enter`/`C-g`/`Esc` end it.
    fn handle_isearch_key(&mut self, ck: crossterm::event::KeyEvent) {
        let (mut query, mut forward) = self.emacs_isearch.take().unwrap();
        match ck.code {
            KeyCode::Enter | KeyCode::Esc => {

                return;
            }
            KeyCode::Backspace => {
                query.pop();
            }
            KeyCode::Char('s') if ck.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                forward = true;
                self.isearch_step(&query, true, true);
                self.emacs_isearch = Some((query, forward));
                return;
            }
            KeyCode::Char('r') if ck.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                forward = false;
                self.isearch_step(&query, false, true);
                self.emacs_isearch = Some((query, forward));
                return;
            }
            KeyCode::Char('g') if ck.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {

                return;
            }
            KeyCode::Char(c) => {
                query.push(c);
            }
            _ => {}
        }
        // Search from the current point for the (possibly extended) query.
        // The "I-search: <query>" prompt is derived from `emacs_isearch` when
        // the frame is built, so there is nothing to publish here.
        self.isearch_step(&query, forward, false);
        self.emacs_isearch = Some((query, forward));
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

    pub fn run_async(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        require_terminal()?;
        crossterm::terminal::enable_raw_mode()?;
        crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen)?;
        crossterm::execute!(std::io::stdout(), EnableMouseCapture)?;
        self.renderer = Box::new(TuiRenderer::new()?);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()?;

        let result = rt.block_on(self.async_run());

        // Kill language servers, and detach the runtime without waiting for the
        // blocking stdin reader (which is parked in event::read()) — otherwise
        // dropping the runtime hangs on exit.
        if self.config.session_autosave {
            self.save_session(true);
        }
        self.terminals.clear();
        self.lsp.shutdown_all();
        rt.shutdown_background();

        crossterm::execute!(std::io::stdout(), DisableMouseCapture)?;
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
                        Some(AppEvent::Input(ev)) => match ev {
                            crossterm::event::Event::Key(k) => self.handle_key(k),
                            crossterm::event::Event::Mouse(me) => self.handle_mouse_event(me),
                            _ => {}
                        },
                        None => break,
                    }
                }
                _ = interval.tick() => {}
            }

            self.fire_watched_events();
            self.drain_lua_actions();

            let dt = self.timer.tick();
            let secs = dt.as_secs_f64();
            self.lua.set_frame_dt(secs);

            let (line, col) = self.cursor_line_col();
            self.cursor_anim.update(dt, col, line, self.config.cursor_anim_enabled, self.config.cursor_anim_speed);

            self.sync_diff_scroll();
            self.render();
            if self.should_quit { break; }
        }

        Ok(())
    }

    /// Turn changes since the last frame into Lua events.
    ///
    /// Called once per frame, before the Lua drain, so a handler's `ruster.cmd`
    /// runs on the same frame the event fired.
    /// How often a background `git status` may run.
    ///
    /// It spawns a process, so this is a compromise rather than a right
    /// answer: fast enough that a statusline is not visibly stale after a
    /// commit, slow enough not to run `git` at frame rate on a large repo.
    const GIT_STATUS_POLL: std::time::Duration = std::time::Duration::from_secs(2);

    /// Keep `git_status` fresh in the background.
    ///
    /// Without this it was only ever populated by `:Git`, so
    /// `ruster.api.git_status()` returned an empty branch until the user
    /// happened to open the status view — which a statusline plugin, the main
    /// reason the query exists, never does.
    fn poll_git_status(&mut self) {
        while let Ok(result) = self.git_status_rx.try_recv() {
            self.git_status_in_flight = false;
            if let Some(status) = result {
                self.git_status.set_status(status);
            }
        }
        if self.git_status_in_flight {
            return;
        }
        let due = self
            .git_status_polled
            .is_none_or(|t| t.elapsed() >= Self::GIT_STATUS_POLL);
        if !due {
            return;
        }
        let Some(root) = self.git_root() else { return };
        self.git_status_polled = Some(std::time::Instant::now());
        self.git_status_in_flight = true;
        let tx = self.git_status_tx.clone();
        std::thread::spawn(move || {
            // Always sends, including on failure — see the field comment.
            let _ = tx.send(ruster_git::status(&root));
        });
    }

    fn fire_watched_events(&mut self) {
        // Before the snapshot below reads it.
        self.poll_git_status();
        let now = {
            let w = self.ws.borrow();
            let buffer = w.active_buffer();
            let doc = w.buffers.get(buffer);
            let path = doc
                .and_then(|d| d.file_path.as_ref())
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let filetype = doc
                .and_then(|d| d.file_path.as_ref())
                .map(|p| ruster_syntax::lang_ext_for_path(p))
                .unwrap_or_default();
            let head = w.primary_head();
            let buf = w.buffer();
            let line = buf.char_to_line(head);
            let col = head - buf.line_start_char(line);
            WatchedState {
                buffer: Some(buffer),
                path,
                window: Some(w.windows.active()),
                cursor: (line, col),
                filetype,
            }
        };
        // Refresh what Lua's read-only queries see, while the path and
        // filetype are already to hand.
        {
            let active = now.buffer;
            let mut snap = self.query_snapshot.borrow_mut();
            snap.path = now.path.clone();
            snap.filetype = now.filetype.clone();
            snap.diagnostics = active
                .map(|b| self.lsp.diagnostics(b))
                .filter(|ds| !ds.is_empty())
                .map(|ds| {
                    ds.iter()
                        .map(|d| ruster_lua::runtime::LuaDiagnostic {
                            // 1-based line to match `CursorMoved` and
                            // `nvim_win_get_cursor`; column stays 0-based.
                            line: d.start.line as i64 + 1,
                            col: d.start.character as i64,
                            severity: d.severity,
                            message: d.message.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            let status = self.git_status.status();
            snap.branch = status.branch.clone().unwrap_or_default();
            snap.staged = status.entries.iter().filter(|e| e.staged.is_some()).count();
            snap.unstaged = status.entries.iter().filter(|e| e.unstaged.is_some()).count();
        }

        let prev = std::mem::replace(&mut self.watched, now.clone());

        // First frame: record the state without firing. `VimEnter` already
        // covers startup, and a plugin does not want a BufEnter storm for a
        // buffer that was open before it loaded.
        if prev == WatchedState::default() {
            return;
        }
        if prev.buffer != now.buffer {
            // Leave before enter, and name the buffer being left rather than
            // the one arriving — a handler saving state needs the old path.
            self.lua.fire_event_str("BufLeave", &[&prev.path]);
            self.lua.fire_event_str("BufEnter", &[&now.path]);
        }
        if prev.window != now.window {
            self.lua.fire_event_str("WinEnter", &[&now.path]);
        }
        if prev.filetype != now.filetype && !now.filetype.is_empty() {
            self.lua.fire_event_str("FileType", &[&now.filetype]);
        }
        if prev.cursor != now.cursor {
            let (line, col) = now.cursor;
            // 1-based line, 0-based column: the same convention
            // `nvim_win_get_cursor` already uses, so a handler can pass one
            // straight to the other.
            self.lua.fire_event_nums("CursorMoved", &[line as i64 + 1, col as i64]);
        }
    }

    /// Run whatever Lua queued since the last frame: `vim.cmd()`, `print()` and
    /// `noice.notify()` all land here. Every event loop must call this, or those
    /// callbacks pile up and never execute.
    fn drain_lua_actions(&mut self) {
        use ruster_core::message::{MessageLevel, MessageSource};
        for action in self.lua.drain_actions() {
            match action {
                LuaAction::Cmd(cmd) => match self.parse_cmdline(&cmd) {
                    Ok(a) => self.apply_cmd(a),
                    Err(e) => {
                        self.notify.push(Notification::new(
                            MessageLevel::Info,
                            MessageSource::Echo,
                            e,
                        ));
                    }
                },
                LuaAction::Print(msg) => {
                    self.notify.push(Notification::new(
                        MessageLevel::Info,
                        MessageSource::Echo,
                        msg,
                    ));
                }
                LuaAction::Dialog { title, fields } => {
                    let fields = fields
                        .into_iter()
                        .map(|(label, kind, value, options)| {
                            let opts: Vec<&str> = options.iter().map(|s| s.as_str()).collect();
                            match kind.as_str() {
                                "toggle" => crate::dialog::Field::toggle(&label, value == "on"),
                                "number" => crate::dialog::Field {
                                    label,
                                    kind: ruster_render::ControlKind::Number,
                                    value,
                                    options: Vec::new(),
                                },
                                "select" => crate::dialog::Field::select(&label, &opts, &value),
                                "button" => crate::dialog::Field::button(&label),
                                // Anything unrecognised is a text field rather
                                // than an error — a plugin typo should not stop
                                // the dialog appearing.
                                _ => crate::dialog::Field::text(&label, &value),
                            }
                        })
                        .collect();
                    self.dialog = Some(DialogState::new(title, fields));
                }
                LuaAction::Notify(level, text) => {
                    let notif_level = match level {
                        1 => MessageLevel::Success,
                        2 => MessageLevel::Warning,
                        3 => MessageLevel::Error,
                        _ => MessageLevel::Info,
                    };
                    self.notify.push(Notification::new(notif_level, MessageSource::Echo, text));
                }
            }
        }
    }

    pub fn run_gui(&mut self) {
        loop {
            let dt = self.timer.tick();
            while let Some(key) = self.renderer.poll_input() {
                self.handle_key(key);
            }
            self.fire_watched_events();
            self.drain_lua_actions();
            let secs = dt.as_secs_f64();
            self.lua.set_frame_dt(secs);

            let (line, col) = self.cursor_line_col();
            self.cursor_anim.update(dt, col, line, self.config.cursor_anim_enabled, self.config.cursor_anim_speed);
            self.sync_diff_scroll();
            self.render();
            // Reported here rather than at the command, so the message reflects
            // the file that was actually written — and so the toast announcing
            // the screenshot never appears *in* it.
            if let Some(result) = self.renderer.poll_screenshot() {
                use ruster_core::message::MessageLevel;
                let (level, text) = match result {
                    Ok(p) => (MessageLevel::Success, format!("Screenshot saved to {}", p.display())),
                    Err(e) => (MessageLevel::Error, format!("Screenshot failed: {e}")),
                };
                self.notify.push(Notification::new(
                    level,
                    ruster_core::message::MessageSource::Echo,
                    text,
                ));
            }
            if self.renderer.should_close() || self.should_quit { break; }
            // No sleep here: raylib paces the loop from `gui.target_fps`
            // (see RaylibRenderer::set_gui_config). A fixed sleep on top of
            // that pinned the GUI to ~60fps whatever the setting said.
        }
        if self.config.session_autosave {
            self.save_session(true);
        }
        self.terminals.clear();
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
            PickerAction::RunCmd(_)
            | PickerAction::RunTask(_)
            | PickerAction::SetTheme(_) => return Vec::new(),
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
            if let Ok(mut engine) = SyntaxEngine::new(&content, &ext) {
                engine.overlay_todo_highlights(&self.config.todo_keywords, todo_style());
                for w in engine.warnings() {
                    push_query_warning(&mut self.notify, w.clone());
                }
                self.syntax.insert(buf, engine);
            }
        }
        // Only reparse when the text actually changed. This runs from `render`,
        // so without the guard a 10k-line file re-parsed every frame: 107 ms
        // against a 16.7 ms budget, or about 7 fps, for a buffer nobody had
        // touched.
        let revision = self.ws.borrow().buffers.get(active).map(|d| d.buffer.revision());
        let stale = revision.is_some_and(|r| self.syntax_revision.get(&active) != Some(&r));
        if stale {
            // Take the edits *with* the text, so the two describe the same
            // moment. Draining them here means a buffer whose engine does not
            // exist yet does not accumulate edits forever.
            let (content, edits) = {
                let mut w = self.ws.borrow_mut();
                match w.buffers.get_mut(active) {
                    Some(d) => (Some(d.buffer.to_string()), d.buffer.take_edits()),
                    None => (None, Vec::new()),
                }
            };
            if let (Some(c), Some(engine)) = (content.as_ref(), self.syntax.get_mut(&active)) {
                engine.reparse_with_edits(c, &edits);
                self.syntax_reparses += 1;
                // reparse rebuilds the cached lines, so the overlay has to be
                // reapplied — and it is not cheap either (22 ms on that file).
                engine.overlay_todo_highlights(&self.config.todo_keywords, todo_style());
                if let Some(r) = revision {
                    self.syntax_revision.insert(active, r);
                }
            }
        }
    }

    /// Re-read highlight queries *and grammars* from disk and rebuild every
    /// buffer's engine.
    ///
    /// Queries are read once when an engine is built, so without this an edit
    /// to `~/.config/ruster/queries/…` would need a restart to see — which
    /// defeats the point of making them editable. `syntax_tried` is cleared
    /// too, or buffers whose engine previously failed to build would never be
    /// retried against the corrected query.
    fn syntax_reload(&mut self) {
        let count = self.syntax.len();
        self.syntax.clear();
        self.syntax_tried.clear();
        self.update_syntax();
        self.notify.push(Notification::new(
            ruster_core::message::MessageLevel::Info,
            ruster_core::message::MessageSource::System,
            format!("Reloaded grammars and queries ({count} buffers)"),
        ));
    }

    /// Every `TODO`-class marker in the open buffers, newest file first.
    ///
    /// Only buffers that already have a syntax engine are scanned — markers come
    /// from the tree's comment captures, so a file with no grammar has none.
    fn todo_markers(&mut self) -> Vec<(PathBuf, ruster_syntax::TodoMarker)> {
        let mut out = Vec::new();
        let w = self.ws.borrow();
        // `all_todo_markers`, not `todo_markers`: the latter reads the comment
        // ranges left by the last highlight pass, which now only covers the
        // visible rows. Right for drawing the overlay, wrong for a panel that
        // claims to list every marker in the file.
        for (&buf, engine) in &mut self.syntax {
            let Some(path) = w.buffers.get(buf).and_then(|d| d.file_path.clone()) else {
                continue;
            };
            for m in engine.all_todo_markers(&self.config.todo_keywords) {
                out.push((path.clone(), m));
            }
        }
        out.sort_by(|a, b| (&a.0, a.1.line).cmp(&(&b.0, b.1.line)));
        out
    }

    /// Sync the active buffer to its language server (didOpen/didChange) and
    /// drain incoming LSP messages, dispatching diagnostics and responses.
    fn update_lsp(&mut self) {
        // `lsp.autostart = false` disables launching/using language servers.
        if !self.config.lsp_autostart {
            return;
        }
        let root = crate::lsp_state::LspState::<LspAction>::root();
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
            self.lsp.sync(active, &path, &lang, &text, &root);
        }
        // Drain and dispatch server messages.
        for routed in self.lsp.poll() {
            match routed.message {
                ServerMessage::Notification { method, params }
                    if method == "textDocument/publishDiagnostics" =>
                {
                    let (path, diags) = ruster_lsp::parse_diagnostics(&params);
                    let n_err = diags.iter().filter(|d| d.severity == 1).count();
                    let n_warn = diags.iter().filter(|d| d.severity == 2).count();
                    if let Some(buf) = self.buffer_for_path(&path) {
                        self.lsp.set_diagnostics(buf, diags);
                    }
                    if n_err > 0 || n_warn > 0 {
                        let file = std::path::Path::new(&path)
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or(path);
                        self.push_message(
                            ruster_core::message::MessageLevel::Warning,
                            ruster_core::message::MessageSource::Lsp,
                            format!("{}: {} err, {} warn", file, n_err, n_warn),
                        );
                    }
                }
                ServerMessage::Response { id, result, .. } => {
                    if let Some(action) = self.lsp.take_pending(&routed.lang, id) {
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
        if !self.config.lsp_diagnostics {
            return None;
        }
        let active = self.ws.borrow().active_buffer();
        let diags = Some(self.lsp.diagnostics(active)).filter(|d| !d.is_empty())?;
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
        let doc = self.lsp.doc(active)?;
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
                self.notify.push(Notification::new(ruster_core::message::MessageLevel::Warning, ruster_core::message::MessageSource::Lsp, "No language server for this buffer".to_string()));
                return false;
            }
        };
        if self.lsp.request(&lang, method, params, action) {
            true
        } else {
            self.notify.push(Notification::new(ruster_core::message::MessageLevel::Warning, ruster_core::message::MessageSource::Lsp, "Language server still starting…".to_string()));
            false
        }
    }

    fn lsp_hover(&mut self) {
        if !self.config.lsp_hover {
            return;
        }
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
                    self.notify.push(Notification::new(ruster_core::message::MessageLevel::Info, ruster_core::message::MessageSource::Lsp, "No hover info".to_string()));
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
                    self.notify.push(Notification::new(ruster_core::message::MessageLevel::Info, ruster_core::message::MessageSource::Lsp, "No definition found".to_string()));
                }
            }
            LspAction::References => {
                let locs = ruster_lsp::parse_locations(&result);
                if locs.is_empty() {
                    self.notify.push(Notification::new(ruster_core::message::MessageLevel::Info, ruster_core::message::MessageSource::Lsp, "No references".to_string()));
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
                    None => { self.notify.push(Notification::new(ruster_core::message::MessageLevel::Info, ruster_core::message::MessageSource::Lsp, "No call hierarchy for symbol".to_string())); },
                }
            }
            LspAction::CallHierarchy(incoming) => {
                let calls = ruster_lsp::parse_call_hierarchy_calls(&result, incoming);
                if calls.is_empty() {
                    self.notify.push(Notification::new(ruster_core::message::MessageLevel::Info, ruster_core::message::MessageSource::Lsp, if incoming { "No callers" } else { "No callees" }.to_string(),));
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
            self.echo_warn(format!("Pattern not found: {}", pattern));
            return;
        }
        let mut text = new.join("\n");
        if had_trailing_newline && !text.ends_with('\n') {
            text.push('\n');
        }
        self.replace_active_content(&text);
        self.echo(format!(
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
        self.notify.tick();
        self.drain_git_hunks();
        self.drain_pending_results();
        self.drain_build_runner();
        self.drain_debug_events();
        self.update_lsp();
        let (cols, rows) = self.renderer.viewport_cells();
        // Reserve a bottom row for the cmdline/message only while one is shown,
        // so the statusline sits flush at the very bottom otherwise.
        let has_cmdline =
            self.vim.mode == VimMode::Cmdline || self.emacs_isearch.is_some() || self.file_prompt.is_some();
        let reserved = if has_cmdline { 1 } else { 0 };
        let mut buf_area = CoreRect::new(0, 0, cols, rows.saturating_sub(reserved));

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
        // Non-insert cursor uses the configured shape (gui.cursor_kind).
        let rest_cursor = if self.config.cursor_kind == "bar" {
            CursorKind::Bar
        } else {
            CursorKind::Block
        };
        let cursor_kind = if emacs {
            CursorKind::Bar
        } else {
            match mode {
                VimMode::Insert | VimMode::Cmdline => CursorKind::Bar,
                _ => rest_cursor,
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
        // Rebuilt below from the geometry actually used to draw this frame; the
        // mouse hit-test reads it back.
        self.last_layout.clear();
        let sidebar_rect = self.sidebar.carve(&mut buf_area);
        let flash_info = self.flash.as_ref().map(|f| (f.labels.clone(), f.pending));
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
                let buf_h = rect.height.saturating_sub(2) as usize;
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

                let is_diff = w.buffers.get(buf_id).is_some_and(|d| {
                    matches!(
                        d.kind,
                        DocKind::Special(ruster_core::document::SpecialKind::GitStaged)
                    )
                });
                let lines: Vec<StyledLine> = match self.dired.styled_lines(buf_id) {
                    // Dired listings are colored by entry type.
                    Some(styled) => styled.to_vec(),
                    // A staged diff is coloured as a diff, not as source.
                    None if is_diff => crate::git_status::diff_styled_lines(&content),
                    None => match self.syntax.get_mut(&buf_id) {
                        Some(engine) => {
                            // Highlighting is bounded to what this window shows.
                            // Doing it here rather than in `update_syntax` is
                            // deliberate: this is the only place the scroll
                            // offset is settled, and reading a stale one would
                            // leave the top or bottom row unstyled for a frame.
                            //
                            // Almost always a range comparison; it re-highlights
                            // only when the scroll leaves the margin.
                            if engine.set_viewport(scroll, scroll + buf_h) {
                                // The rebuild dropped the TODO overlay with it.
                                engine.overlay_todo_highlights(
                                    &self.config.todo_keywords,
                                    todo_style(),
                                );
                            }
                            engine.styled_lines().to_vec()
                        }
                        None => plain_lines(&content),
                    },
                };
                let pct = ((cline + 1) * 100).checked_div(line_count).unwrap_or(100);
                // A focused terminal shows a dedicated mode label; in Terminal-
                // Normal the underlying vim mode (NORMAL/VISUAL) shows through.
                let focused_terminal =
                    is_active && self.terminal_focused && self.terminals.contains_key(&buf_id);
                let mut left = if focused_terminal {
                    "-- TERMINAL --".to_string()
                } else if is_active {
                    let mut lbl = mode_lbl.clone();
                    if let Some(ref root) = self.project_root {
                        if let Some(name) = root.file_name() {
                            lbl = format!("{}  [{}]", lbl, name.to_string_lossy());
                        }
                    }
                    lbl
                } else {
                    String::new()
                };
                let runner_msg = self.runner_status_text();
                let mut center = if let Some(msg) = runner_msg {
                    format!(" {} {} ", msg, name)
                } else {
                    name.clone()
                };
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
                let statusline = StatuslineView { left, center, right, active: is_active, mode: vim_mode_to_ui_mode(mode) };
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
                // If this window hosts a terminal, resize its PTY to the window's
                // text area and snapshot its grid for rendering — unless it's the
                // active terminal in Terminal-Normal mode, where the mirrored
                // buffer text is drawn instead so vim motions/visual show through.
                let in_terminal_normal = is_active && !self.terminal_focused;
                let terminal = if self.terminals.contains_key(&buf_id) && !in_terminal_normal {
                    let session = self.terminals.get_mut(&buf_id).expect("terminal exists");
                    let cols = rect.width.max(1);
                    let rows = rect.height.saturating_sub(1).max(1);
                    let _ = session.resize(cols, rows);
                    Some(to_term_grid_view(&session.snapshot()))
                } else {
                    None
                };
                // Diagnostics render as a sign column (not on terminals); test
                // results (✓/✗) from the last run are merged in by file path.
                let signs = if terminal.is_some() {
                    ruster_render::SignsView::default()
                } else {
                    // Git status is the weakest signal: a diagnostic or a
                    // breakpoint on the same line matters more, and later signs
                    // win, so these go in first.
                    let mut s = self.git_signs_for(buf_id);
                    if let Some(diag) =
                        Some(self.lsp.diagnostics(buf_id)).filter(|d| !d.is_empty()).map(diagnostics_to_signs)
                    {
                        s.width = s.width.max(diag.width);
                        s.signs.extend(diag.signs);
                    }
                    if !self.result_signs.is_empty() {
                        // `w`, not `self.ws.borrow()`: a mutable borrow is
                        // already live in this scope, and re-borrowing panics
                        // with `RefCell already mutably borrowed`. Both this
                        // branch and the breakpoint one below did exactly that
                        // — silently, because each is behind a guard that is
                        // false until a test has been run or a breakpoint set.
                        if let Some(p) =
                            w.buffers.get(buf_id).and_then(|d| d.file_path.clone())
                        {
                            let key = p.canonicalize().unwrap_or(p);
                            if let Some(rs) = self.result_signs.get(&key) {
                                s.width = s.width.max(rs.width);
                                s.signs.extend(rs.signs.iter().cloned());
                            }
                        }
                    }
                    if self.debug.any_breakpoints() {
                        if let Some(p) =
                            w.buffers.get(buf_id).and_then(|d| d.file_path.clone())
                        {
                            let key = p.canonicalize().unwrap_or(p);
                            {
                                let bps = self.debug.breakpoints_in(&key);
                                s.width = s.width.max(1);
                                let bp_signs: Vec<(u16, char, ruster_render::Color)> = bps
                                    .iter()
                                    .map(|&l| (l, '●', ruster_syntax::sign_style("breakpoint").fg))
                                    .collect();
                                s.signs.extend(bp_signs);
                            }
                        }
                    }
                    s
                };
                let flash_labels = if is_active {
                    if let Some((ref labels, pending)) = flash_info {
                        let buf_h = rect.height.saturating_sub(2) as usize;
                        let mut result = Vec::new();
                        if let Some(doc) = w.buffers.get(buf_id) {
                            for fl in labels {
                                let offset = fl.offset;
                                let line_no = doc.buffer.char_to_line(offset);
                                let line_start = doc.buffer.line_start_char(line_no);
                                let col = offset.saturating_sub(line_start);
                                let screen_row = line_no.saturating_sub(scroll);
                                if screen_row >= buf_h { continue; }
                                let (display_text, color) = if pending.is_some() {
                                    let sub = if fl.label.len() > 1 {
                                        fl.label[1..].to_string()
                                    } else {
                                        fl.label.clone()
                                    };
                                    (sub, ruster_syntax::flash_style("pending").fg)
                                } else {
                                    (fl.label.clone(), ruster_syntax::flash_style("label").fg)
                                };
                                result.push(FlashLabelRender {
                                    row: screen_row as u16,
                                    col: col as u16,
                                    text: display_text,
                                    color,
                                });
                            }
                        }
                        result
                    } else {
                        vec![]
                    }
                } else {
                    vec![]
                };
                let rrect = RRect::new(rect.x, rect.y, rect.width, rect.height);
                // A terminal window draws its own grid, so there is no buffer
                // text to click into.
                if terminal.is_none() {
                    self.last_layout.push(WindowLayout {
                        window: wid,
                        buffer: buf_id,
                        text: ruster_render::TextArea::of(rrect, signs.width, gutter.width),
                        scroll_top: scroll,
                    });
                }
                views.push(WindowView {
                    rect: rrect,
                    header: name.clone(),
                    lines,
                    cursor: (cline as u16, ccol as u16),
                    extra_cursors,
                    cursor_kind,
                    cursor_visible: true,
                    cursor_smooth,
                    scroll_offset: scroll as u16,
                    gutter,
                    signs,
                    statusline,
                    active: is_active,
                    selection,
                    terminal,
                    flash_labels,
                });
            }
        }
        if let Some(srect) = sidebar_rect {
            views.insert(
                0,
                self.sidebar.view(srect, vim_mode_to_ui_mode(self.vim.mode), &self.theme_palette()),
            );
        }

        let cmdline = if let Some(p) = &self.file_prompt {
            Some(p.display())
        } else {
            match mode {
                VimMode::Cmdline => Some(crate::widgets::cmdline_label(self.vim.cmdline_buffer())),
                _ => self.emacs_isearch.as_ref().map(|(q,f)| format!("{}: {}", if *f { "I-search" } else { "I-search backward" }, q)).or_else(|| self.current_line_diagnostic()),
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
        } else if self.g_pending.is_some() {
            self.whichkey_cache = Some(g_whichkey());
        }
        // Show the panel only after timeoutlen from the prefix start — but once
        // it has begun appearing, keep it up until the sequence ends. Both the
        // Space leader and the `g` menu drive it.
        let prefix_since = self.leader_since.or(self.g_pending);
        let past_timeout = prefix_since
            .is_some_and(|t| now.duration_since(t).as_millis() as u32 >= self.config.timeoutlen);
        let show = self.config.whichkey_enabled
            && (self.leader_pending.is_some() || self.g_pending.is_some())
            && (self.whichkey_anim > 0.01 || past_timeout);
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

        // Show the welcome / "Dashboard" screen when no named file is open and
        // the active buffer is a scratch document or the pinned Dashboard.
        let is_dashboard = {
            let w = self.ws.borrow();
            let active = w.active_doc();
            active.file_path.is_none()
                && (matches!(active.kind, DocKind::Scratch)
                    || matches!(active.kind, DocKind::Special(SpecialKind::Dashboard)))
        };
        let welcome_recent: Vec<String> = ruster_config_dir()
            .map(|d| {
                ruster_project::recent_projects(&d)
                    .iter()
                    .take(10)
                    .map(|p| p.display().to_string())
                    .collect()
            })
            .unwrap_or_default();
        let welcome_view = if is_dashboard {
            Some(WelcomeView {
                visible: true,
                recent_projects: welcome_recent,
                version: option_env!("CARGO_PKG_VERSION")
                    .unwrap_or("0.1.0")
                    .to_string(),
                lsp_status: "● Ready".into(),
                edit_mode: match mode {
                    VimMode::Insert => "Insert",
                    VimMode::VisualChar | VimMode::VisualLine | VimMode::VisualBlock => "Visual",
                    VimMode::Cmdline => "Cmdline",
                    _ => "Normal",
                }
                .into(),
            })
        } else {
            None
        };

        let level_icon = |level: ruster_core::message::MessageLevel| -> &'static str {
            match level {
                ruster_core::message::MessageLevel::Info => "",
                ruster_core::message::MessageLevel::Success => "✓",
                ruster_core::message::MessageLevel::Warning => "⚠",
                ruster_core::message::MessageLevel::Error => "✗",
            }
        };
        let noice_mini: Vec<String> = self.notify.active(BackendKind::Mini)
            .into_iter()
            .map(|n| format!("{} {}", level_icon(n.level), n.text))
            .collect();
        let noice_notify = if self.show_noice_panel {
            let stack = self.notify.active(BackendKind::Notify);
            if stack.is_empty() {
                None
            } else {
                Some(stack.into_iter().map(|n| {
                    let style = match n.level {
                        ruster_core::message::MessageLevel::Error => SyntaxStyle::error(),
                        ruster_core::message::MessageLevel::Warning => SyntaxStyle::warning(),
                        _ => SyntaxStyle::info(),
                    };
                    let text = format!("{} {}: {}", level_icon(n.level), n.source.label(), n.text);
                    let len = text.len();
                    StyledLine { text, highlights: vec![(0, len, style)] }
                }).collect())
            }
        } else {
            None
        };
        // The hover popup is an ordinary float, so it shares one clamping and
        // drawing path with every other floating surface.
        //
        // Built fresh each frame rather than kept on `App`. There was a
        // `floats` field for this, cloned into every `FrameState` and never
        // written to by anything — the hover popup was always pushed here, to
        // the local. Nothing can raise a float except hover, so the field was
        // an empty vector with a clone.
        let mut floats: Vec<ruster_render::FloatView> = Vec::new();
        if let Some(lines) = &self.hover {
            if !lines.is_empty() {
                floats.push(ruster_render::FloatView::anchored(
                    RRect::new(0, 0, cols, rows),
                    ruster_render::FloatAnchor::Edge(ruster_render::FloatEdge::Top),
                    lines.clone(),
                ));
            }
        }
        // Notification popups float above the window views, one box per active
        // notification. `CmdlinePopup` and `Popup` differ only in duration; a
        // `Confirm` raises the modal dialog instead of drawing a float.
        floats.extend(self.notification_floats(cols, rows));
        let state = FrameState {
            dialog: self.dialog.as_ref().map(|d| d.view()),
            floats,
            windows: views,
            cmdline: cmdline.as_deref(),
            noice_mini,
            noice_notify,
            picker: picker_view,
            whichkey,
            settings: self.settings.as_ref().map(|s| s.view()),
            welcome: welcome_view,
            theme: self.theme_palette(),
            debug_overlay: self.build_debug_overlay(),
        };
        self.renderer.render_frame(&state);
    }

    /// Build the floats raised by active popup notifications. `CmdlinePopup`
    /// and `Popup` differ only in duration, so both render the same way: a
    /// titled box centred on the window. A queued `Confirm` is handled here
    /// too — it becomes the modal dialog, the same surface a plugin's `:dialog`
    /// raises, so it draws last above every float.
    fn notification_floats(&mut self, cols: u16, rows: u16) -> Vec<ruster_render::FloatView> {
        let mut floats = Vec::new();
        for (i, n) in self
            .notify
            .active(BackendKind::CmdlinePopup)
            .into_iter()
            .chain(self.notify.active(BackendKind::Popup))
            .enumerate()
        {
            let style = match n.level {
                ruster_core::message::MessageLevel::Error => SyntaxStyle::error(),
                ruster_core::message::MessageLevel::Warning => SyntaxStyle::warning(),
                _ => SyntaxStyle::info(),
            };
            let text = n.text.clone();
            let len = text.len();
            floats.push(
                ruster_render::FloatView::anchored_titled(
                    RRect::new(0, 0, cols, rows),
                    ruster_render::FloatAnchor::Center,
                    vec![StyledLine { text, highlights: vec![(0, len, style)] }],
                    n.title.clone().or_else(|| Some("ruster".into())),
                )
                .with_z(5 + i as i32),
            );
        }
        // A queued Confirm notification becomes a modal dialog. Only raise it
        // when nothing else is already showing one — the dialog is exclusive.
        if self.dialog.is_none() {
            if let Some(n) = self.notify.active(BackendKind::Confirm).into_iter().next() {
                let title = n.title.clone().unwrap_or_else(|| "Confirm".into());
                let text = n.text.clone();
                self.dialog = Some(crate::dialog::DialogState::new(
                    title,
                    vec![
                        crate::dialog::Field::text("Message", &text),
                        crate::dialog::Field::button("OK"),
                        crate::dialog::Field::button("Cancel"),
                    ],
                ));
            }
        }
        floats
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
            "term" | "terminal" => Ok(CmdAction::Terminal),
            "config-errors" | "configerrors" => Ok(CmdAction::ConfigErrors),
            "settings" | "config" | "RusterConfig" | "rusterconfig" => Ok(CmdAction::Settings),
            "build" | "make" => Ok(CmdAction::Build),
            "test" => Ok(CmdAction::Test),
            "task" | "tasks" => Ok(CmdAction::TaskPicker),
            "copen" | "cope" | "cwindow" | "cw" => Ok(CmdAction::QuickfixOpen),
            "cnext" | "cn" => Ok(CmdAction::QuickfixNext),
            "cprev" | "cp" | "cN" | "cprevious" => Ok(CmdAction::QuickfixPrev),
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
            _ if trimmed == "messages" || trimmed == "message" || trimmed == "msgs" => {
                Ok(CmdAction::Messages)
            }
            _ if trimmed.starts_with("messages ") || trimmed.starts_with("msgs ") => {
                let filter = trimmed.split_once(' ').map(|x| x.1).unwrap_or("").trim().to_string();
                Ok(CmdAction::MessagesFilter(filter))
            }
            _ if trimmed.starts_with("messages/") || trimmed.starts_with("msgs/") => {
                let filter = trimmed.split_once('/').map(|x| x.1).unwrap_or("").trim().to_string();
                Ok(CmdAction::MessagesFilter(filter))
            }
            "e" | "edit" => Ok(CmdAction::Files),
            _ if trimmed.starts_with("e ") || trimmed.starts_with("edit ") => {
                let path = trimmed
                    .split_once(' ')
                    .map(|x| x.1)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if path.is_empty() {
                    Ok(CmdAction::Files)
                } else {
                    Ok(CmdAction::OpenFile(path))
                }
            }
            _ if trimmed == "projects" => Ok(CmdAction::Projects),
            "db" | "debug" => Ok(CmdAction::DebugStart),
            "db_continue" | "continue" if self.debug.is_running() => Ok(CmdAction::DebugContinue),
            "db_next" | "n" if self.debug.is_running() => Ok(CmdAction::DebugNext),
            "db_stepin" | "s" if self.debug.is_running() => Ok(CmdAction::DebugStepIn),
            "db_stepout" | "finish" if self.debug.is_running() => Ok(CmdAction::DebugStepOut),
            "db_stop" if self.debug.is_running() => Ok(CmdAction::DebugStop),
            "db_toggle" | "B" => Ok(CmdAction::DebugToggleBreakpoint),
            "Gitsigns" | "gitsigns" => Ok(CmdAction::GitsignsToggle),
            "TodoList" | "todolist" | "todo" => Ok(CmdAction::TodoList),
            "Trouble" | "trouble" => Ok(CmdAction::Trouble),
            "Themes" | "themes" | "theme" => Ok(CmdAction::Themes),
            _ if trimmed == "sidebar" => Ok(CmdAction::Sidebar),
            "Mason" | "mason" => Ok(CmdAction::Mason),
            "Git" | "git" | "G" => Ok(CmdAction::GitStatus),
            "GitStageHunk" | "gitstagehunk" | "stagehunk" => Ok(CmdAction::GitStageHunk),
            "GitStaged" | "gitstaged" | "staged" => Ok(CmdAction::GitStaged),
            "GitCommit" | "gitcommit" | "commit" => Ok(CmdAction::GitCommit),
            "GitPush" | "gitpush" | "push" => Ok(CmdAction::GitPush),
            "GitPull" | "gitpull" | "pull" => Ok(CmdAction::GitPull),
            "help" | "h" | "Help" => Ok(CmdAction::Help(None)),
            _ if let Some(t) = trimmed.strip_prefix("help ").or_else(|| trimmed.strip_prefix("h ")) => {
                Ok(CmdAction::Help(Some(t.trim().to_string())))
            }
            "SessionSave" | "sessionsave" | "mksession" => Ok(CmdAction::SessionSave),
            "SessionRestore" | "sessionrestore" | "loadsession" => Ok(CmdAction::SessionRestore),
            "SyntaxReload" | "syntaxreload" | "syntax reload" => Ok(CmdAction::SyntaxReload),
            "Diffview" | "diffview" | "Diff" | "diff" => Ok(CmdAction::Diffview),
            _ if let Some(n) = trimmed.strip_prefix("sidebar resize ").and_then(|s| s.trim().parse::<u16>().ok()) => Ok(CmdAction::SidebarResize(n)),
            "screenshot" | "Screenshot" => Ok(CmdAction::Screenshot(None)),
            _ if let Some(rest) = trimmed
                .strip_prefix("screenshot ")
                .or_else(|| trimmed.strip_prefix("Screenshot ")) =>
            {
                Ok(CmdAction::Screenshot(Some(rest.trim().to_string())))
            }
            "Noice" | "noice" => Ok(CmdAction::NoicePanel),
            _ if trimmed.starts_with("Noice ") || trimmed.starts_with("noice ") => {
                let sub = trimmed.split_once(' ').map(|x| x.1).unwrap_or("").trim().to_string();
                match sub.as_str() {
                    "split" | "history" => Ok(CmdAction::NoiceSplit),
                    "popup" => Ok(CmdAction::NoicePopup),
                    _ => Err(format!(":Noice subcommand '{}' unknown. Use :Noice (toggle panel), :Noice split|history, or :Noice popup", sub)),
                }
            }
            _ if trimmed.starts_with("set editmode ") => {
                match trimmed.rsplit(' ').next().unwrap_or("") {
                    "emacs" => Ok(CmdAction::SetEditMode(EditMode::Emacs)),
                    "neovim" | "vim" | "nvim" => Ok(CmdAction::SetEditMode(EditMode::Neovim)),
                    _ => Err("Usage: :set editmode neovim|emacs".to_string()),
                }
            }
            _ if trimmed.starts_with("set ") => {
                parse_set_general(trimmed.strip_prefix("set ").unwrap_or(""))
            }
            _ if parse_substitute(trimmed).is_some() => {
                Ok(parse_substitute(trimmed).expect("checked above"))
            }
            _ if trimmed.starts_with("echo ") => {
                let text = trimmed.strip_prefix("echo ").unwrap_or("").to_string();
                Ok(CmdAction::Echo(text, ruster_core::message::MessageLevel::Info))
            }
            _ if trimmed.starts_with("echom ") => {
                let text = trimmed.strip_prefix("echom ").unwrap_or("").to_string();
                Ok(CmdAction::Echo(text, ruster_core::message::MessageLevel::Warning))
            }
            _ if trimmed.starts_with("echoe ") => {
                let text = trimmed.strip_prefix("echoe ").unwrap_or("").to_string();
                Ok(CmdAction::Echo(text, ruster_core::message::MessageLevel::Error))
            }
            // `:16` and `:$`. Vim users type these constantly, and until now the
            // cmdline answered "Unknown command: 16" — which reads as a bug in
            // whatever you were doing, not a missing feature. I misread it as
            // one myself while verifying something else.
            "hover" | "Hover" => Ok(CmdAction::Hover),
            "$" => Ok(CmdAction::GotoLine(None)),
            _ if !trimmed.is_empty() && trimmed.bytes().all(|b| b.is_ascii_digit()) => {
                // Saturating: `:99999999999999999999` is a typo, not an error
                // worth a message. Clamped to the buffer when it is applied.
                Ok(CmdAction::GotoLine(Some(trimmed.parse().unwrap_or(usize::MAX))))
            }
            _ => Err(format!("Unknown command: {}", cmdline)),
        }
    }

    /// Apply a parsed cmdline action. `:q` closes the active window and only
    /// quits the app when it is the last window.
    fn apply_cmd(&mut self, action: CmdAction) {
        // While the settings page is open, `:w` saves it and `:q` closes it.
        //
        // Anything else closes the page and then runs normally. It used to be
        // swallowed — `_ => {}` and `return` — so with the settings page up,
        // `:Git`, `:help` and every other command did nothing at all and said
        // nothing about why. Asking for something else is a clear enough signal
        // that the page has served its purpose.
        if self.settings.is_some() {
            match action {
                CmdAction::Save(_) => {
                    self.save_settings();
                    return;
                }
                CmdAction::SaveAndQuit => {
                    self.save_settings();
                    self.settings = None;
                    return;
                }
                CmdAction::Quit | CmdAction::ForceQuit | CmdAction::Settings => {
                    self.settings = None;
                    return;
                }
                _ => self.settings = None,
            }
        }
        match action {
            CmdAction::Save(force) => {
                // `:w` on a commit message commits it rather than writing a
                // file — that buffer has no path to write to.
                if self.active_is_git_commit() {
                    self.commit_from_buffer();
                } else {
                    self.save_file(force);
                }
            }
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
                    self.echo_error("E444: Cannot close last window".to_string());
                }
            }
            CmdAction::Only => self.ws.borrow_mut().windows.only(),
            CmdAction::Fullscreen => self.ws.borrow_mut().windows.toggle_fullscreen(),
            CmdAction::Ibuffer => self.open_ibuffer(),
            CmdAction::Terminal => self.open_terminal(),
            CmdAction::ConfigErrors => self.open_config_errors(),
            CmdAction::Settings => self.open_settings(),
            CmdAction::Build => self.run_build(),
            CmdAction::Test => self.run_test(),
            CmdAction::TaskPicker => self.open_task_picker(),
            CmdAction::QuickfixOpen => self.open_quickfix(),
            CmdAction::QuickfixNext => self.quickfix_next(),
            CmdAction::QuickfixPrev => self.quickfix_prev(),
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
            CmdAction::SetNamed(key, named_val) => self.set_named_option(&key, named_val),
            CmdAction::Substitute { pattern, replacement, all, whole_buffer } => {
                self.substitute(&pattern, &replacement, all, whole_buffer)
            }
            CmdAction::Messages => self.open_messages(),
            CmdAction::MessagesFilter(filter) => self.apply_messages_filter(&filter),
            CmdAction::Projects => self.open_projects(),
            CmdAction::TodoList => self.open_todo_list(),
            CmdAction::Trouble => self.open_trouble(),
            CmdAction::Themes => self.open_themes_picker(),
            CmdAction::GitsignsToggle => {
                // `set_enabled` drops the cache when turning off, so the
                // "clear" half of this no longer has to be remembered here.
                if self.git.set_enabled(!self.git.enabled()) {
                    let id = self.ws.borrow().active_buffer();
                    self.refresh_git_hunks(id);
                    self.echo("Git signs on");
                } else {
                    self.echo("Git signs off");
                }
            }
            CmdAction::Sidebar => self.toggle_sidebar(),
            CmdAction::Mason => self.open_mason(),
            CmdAction::GitStatus => self.open_git_status(),
            CmdAction::GitStageHunk => self.git_stage_hunk(),
            CmdAction::GitStaged => self.open_git_staged(),
            CmdAction::GitCommit => self.open_git_commit(),
            CmdAction::GitPush => self.confirm_git_remote("Push"),
            CmdAction::GitPull => self.confirm_git_remote("Pull"),
            CmdAction::Help(topic) => self.open_help(topic.as_deref()),
            CmdAction::SessionSave => self.save_session(false),
            CmdAction::SessionRestore => self.restore_session(false),
            CmdAction::SyntaxReload => self.syntax_reload(),
            CmdAction::Diffview => self.open_diffview(),
            CmdAction::SidebarResize(n) => {
                self.sidebar.set_width(n);
            }
            CmdAction::Hover => self.lsp_hover(),
            CmdAction::GotoLine(target) => {
                // Clamped, not rejected: `:9999` in a short file goes to the
                // end, which is what vim does and what the typist meant.
                let pos = {
                    let w = self.ws.borrow();
                    let buf = w.buffer();
                    let last = buf.line_count().saturating_sub(1);
                    // 1-based on the way in; `:0` means the first line.
                    let line = match target {
                        Some(n) => n.saturating_sub(1).min(last),
                        None => last,
                    };
                    buf.line_start_char(line)
                };
                self.ws.borrow_mut().execute(Action::Move(Motion::To(pos)));
                // No explicit scroll: `render` already pulls the window to the
                // cursor, which is the same path `G` and a quickfix jump take.
            }
            CmdAction::Screenshot(arg) => {
                let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                let path = screenshot_path(arg.as_deref(), &cwd);
                if !self.renderer.request_screenshot(&path) {
                    self.notify.push(Notification::new(
                        ruster_core::message::MessageLevel::Warning,
                        ruster_core::message::MessageSource::Echo,
                        "Screenshots need the GUI backend — run `just gui`".to_string(),
                    ));
                }
                // On success there is nothing to say yet: the capture happens
                // after the next frame, and `run_gui` reports the outcome.
            }
            CmdAction::DebugStart => self.debug_start(),
            CmdAction::DebugContinue => self.debug_continue(),
            CmdAction::DebugNext => self.debug_step_over(),
            CmdAction::DebugStepIn => self.debug_step_in(),
            CmdAction::DebugStepOut => self.debug_step_out(),
            CmdAction::DebugStop => self.debug_stop(),
            CmdAction::DebugToggleBreakpoint => self.debug_toggle_breakpoint(),
            CmdAction::ShowSetting(key) => {
                let value = self.config.to_settings().into_iter()
                    .find(|((_g, k), _)| *k == key)
                    .map(|(_, v)| v.display())
                    .unwrap_or_default();
                let msg = match value.as_str() {
                    "on" => key.to_string(),
                    "off" => format!("no{}", key),
                    _ => format!("{}={}", key, value),
                };
                self.notify.push(Notification::new(
                    ruster_core::message::MessageLevel::Info,
                    ruster_core::message::MessageSource::Echo,
                    msg,
                ));
            }
            CmdAction::ResetSetting(key) => {
                let spec = match ruster_lua::schema::spec_by_key(&key) {
                    Some(s) => s,
                    None => {
                        self.notify.push(Notification::new(
                            ruster_core::message::MessageLevel::Error, ruster_core::message::MessageSource::Echo,
                            format!("E518: Unknown option: {key}"),
                        ));
                        return;
                    }
                };
                let default = spec.default;
                let mut vals = self.config.to_settings();
                if let Some(pos) = vals.iter_mut().find(|((_g, k), _)| *k == key) {
                    pos.1 = default.clone();
                }
                let old_editmode = self.config.editmode.clone();
                self.config = Config::from_settings(&vals);
                self.config.colors = resolve_theme_colors(&self.lua, &self.config.theme, &self.config.color_overrides);
                if key == "editmode" && self.config.editmode != old_editmode {
                    let mode = match self.config.editmode.as_str() {
                        "emacs" => EditMode::Emacs,
                        _ => EditMode::Neovim,
                    };
                    self.set_editmode(mode);
                }
                self.notify.push(Notification::new(
                    ruster_core::message::MessageLevel::Info,
                    ruster_core::message::MessageSource::Echo,
                    format!("{} = {} (default)", key, default.display()),
                ));
            }
            CmdAction::Echo(text, level) => {
                self.notify.push(Notification::new(level, ruster_core::message::MessageSource::Echo, text));
            }
            CmdAction::NoicePanel => self.show_noice_panel = !self.show_noice_panel,
            CmdAction::NoiceSplit => self.open_noice_split(),
            CmdAction::NoicePopup => {
                self.notify.push_to(
                    Notification::new(
                        ruster_core::message::MessageLevel::Info,
                        ruster_core::message::MessageSource::Echo,
                        ":Noice popup — a popup notification.".to_string(),
                    )
                    .with_persistent(),
                    BackendKind::Popup,
                );
            }
            CmdAction::OpenFile(path) => {
                let base = self.ws.borrow()
                    .active_doc()
                    .file_path
                    .as_ref()
                    .and_then(|p| std::path::Path::new(p).parent().map(|p| p.to_path_buf()))
                    .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
                let resolved = resolve_path(&path, &base);
                self.open_path(&resolved, None);
            }
        }
    }

    /// Open a fuzzy file picker over the project (gitignore-aware walk). The walk
    /// runs on a background thread, streaming paths into the picker each frame so
    /// the render loop never blocks on a large repo.
    fn open_files_picker(&mut self) {
        let root = self.project_root.clone()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
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
        let cwd = self.project_root.clone()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let mut child = match std::process::Command::new("rg")
            .arg("--vimgrep")
            .arg(pattern)
            .current_dir(&cwd)
            .stdout(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => {
                self.echo_error("ripgrep (rg) not found in PATH".to_string());
                return;
            }
        };
        let stdout = match child.stdout.take() {
            Some(s) => s,
            None => {
                self.echo_error("failed to capture rg output".to_string());
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
        self.dired.open(&mut self.ws.borrow_mut(), path);
    }

    fn active_is_dired(&self) -> bool {
        DiredState::active_is_dired(&self.ws.borrow())
    }

    /// The active buffer id if it is a terminal with a live session.
    fn active_terminal_buffer(&self) -> Option<BufferId> {
        let bid = self.ws.borrow().active_buffer();
        let is_term = matches!(
            self.ws.borrow().buffers.get(bid).map(|d| d.kind),
            Some(DocKind::Special(SpecialKind::Terminal))
        );
        if is_term && self.terminals.contains_key(&bid) {
            Some(bid)
        } else {
            None
        }
    }

    /// Handle a key while the Settings page is open.
    fn handle_settings_key(&mut self, ck: crossterm::event::KeyEvent) {
        let mut changed = false;
        let mut close = false;
        {
            let Some(s) = self.settings.as_mut() else { return };
            if s.is_editing() {
                match ck.code {
                    KeyCode::Enter => { s.edit_commit(); changed = true; }
                    KeyCode::Esc => s.edit_cancel(),
                    KeyCode::Backspace => s.edit_backspace(),
                    KeyCode::Char(c) => s.edit_push(c),
                    _ => {}
                }
            } else if s.filter.is_some() {
                match ck.code {
                    KeyCode::Esc | KeyCode::Enter => { s.filter = None; s.rebuild_rows(); }
                    KeyCode::Backspace => {
                        let f = s.filter.as_mut().unwrap();
                        f.pop();
                        s.rebuild_rows();
                    }
                    KeyCode::Char(c) => {
                        s.filter.as_mut().unwrap().push(c);
                        s.rebuild_rows();
                    }
                    _ => {}
                }
            } else {
                if !matches!(ck.code, KeyCode::Char('d')) { s.cancel_d(); }
                if !matches!(ck.code, KeyCode::Char('g')) { s.cancel_g(); }
                match ck.code {
                    KeyCode::Esc | KeyCode::Char('q') => close = true,
                    KeyCode::Char('j') | KeyCode::Down => s.move_down(),
                    KeyCode::Char('k') | KeyCode::Up => s.move_up(),
                    KeyCode::Char('g') => { s.press_g(); }
                    KeyCode::Char('G') => s.move_to_bottom(),
                    KeyCode::Tab | KeyCode::Char(']') => s.next_group(),
                    KeyCode::BackTab | KeyCode::Char('[') => s.prev_group(),
                    KeyCode::Char('/') => { s.filter = Some(String::new()); }
                    KeyCode::Char(' ') | KeyCode::Enter => { s.activate(); changed = true; }
                    KeyCode::Char('l') | KeyCode::Right => { s.adjust(1); changed = true; }
                    KeyCode::Char('h') | KeyCode::Left => { s.adjust(-1); changed = true; }
                    KeyCode::Char('d') => changed = s.press_d(),
                    KeyCode::Delete => changed = s.reset_selected(),
                    _ => {}
                }
            }
        }
        if close {
            self.settings = None;
        } else if changed {
            self.apply_settings_live();
        }
    }

    /// Serialize the settings page's values to `config.lua` and apply the ones
    /// that can take effect live (GUI font/size/colors still need a restart).
    fn save_settings(&mut self) {
        let Some(s) = self.settings.as_ref() else { return };
        let values = s.values();
        // The syntax editor's overrides are carried outside the flat schema.
        let syntax = s.syntax_overrides();
        let mut lua = ruster_lua::schema::generate_config(&values);
        lua.push_str(&ruster_lua::config::syntax_to_lua(&syntax));
        let mut wrote = false;
        if let Some(dir) = ruster_config_dir() {
            let _ = std::fs::create_dir_all(&dir);
            if std::fs::write(dir.join("config.lua"), &lua).is_ok() {
                wrote = true;
            }
        }
        // Apply the edited values (config + live GUI re-theme), then install the
        // syntax colours and recolour open buffers.
        self.apply_settings_live();
        self.config.syntax_overrides = syntax;
        self.install_and_recolor_syntax();
        if let Some(s) = self.settings.as_mut() {
            s.dirty = false;
        }
        self.echo_error(if wrote {
            "Saved config.lua".to_string()
        } else {
            "Could not write config.lua".to_string()
        });
    }

    /// Push the config's per-language syntax colours into the highlighter and
    /// recompute every open buffer's cached highlights (no reparse).
    fn install_and_recolor_syntax(&mut self) {
        ruster_syntax::set_syntax_overrides(syntax_overrides_to_colors(&self.config.syntax_overrides));
        let ws = self.ws.clone();
        for (id, engine) in self.syntax.iter_mut() {
            let text = ws.borrow().buffers.get(*id).map(|d| d.buffer.to_string());
            if let Some(text) = text {
                engine.recolor(&text);
            }
        }
    }

    /// Rebuild the config from the Settings page's current values, re-resolve the
    /// theme colors, and re-theme the GUI live — used for both on-change preview
    /// and `:w` save.
    fn apply_settings_live(&mut self) {
        let values = match self.settings.as_ref() {
            Some(s) => s.values(),
            None => return,
        };
        self.config = ruster_lua::config::Config::from_settings(&values);
        self.config.colors =
            resolve_theme_colors(&self.lua, &self.config.theme, &self.config.color_overrides);
        self.ws.borrow_mut().set_active_indent_width(self.config.tabstop);
        let gui = self.gui_config();
        let font = self.gui_font();
        self.renderer.set_gui_config(&gui, font.as_deref());
    }

    /// Theme names available in the picker: built-ins plus any `themes/*.lua`.
    fn available_themes(&self) -> Vec<String> {
        let mut names: Vec<String> =
            ruster_lua::config::builtin_themes().iter().map(|(n, _)| n.to_string()).collect();
        if let Some(dir) = ruster_config_dir() {
            if let Ok(rd) = std::fs::read_dir(dir.join("themes")) {
                for entry in rd.flatten() {
                    let path = entry.path();
                    if path.extension().is_some_and(|x| x == "lua") {
                        if let Some(stem) = path.file_stem() {
                            let s = stem.to_string_lossy().into_owned();
                            if !names.contains(&s) {
                                names.push(s);
                            }
                        }
                    }
                }
            }
        }
        names
    }

    /// A theme's named palette as `(color_name, "#hex")` pairs — built-in themes
    /// carry theirs directly; user themes are read from their `.lua` file.
    fn theme_palette_for(&self, name: &str) -> Vec<(String, String)> {
        if let Some((_, theme)) =
            ruster_lua::config::builtin_themes().into_iter().find(|(n, _)| *n == name)
        {
            return theme.palette.iter().map(|(n, c)| (n.clone(), c.to_hex())).collect();
        }
        if let Some(dir) = ruster_config_dir() {
            let path = dir.join("themes").join(format!("{name}.lua"));
            if let Ok(code) = std::fs::read_to_string(&path) {
                if let Some(theme) = self.lua.load_theme(&code) {
                    return theme.palette.iter().map(|(n, c)| (n.clone(), c.to_hex())).collect();
                }
            }
        }
        Vec::new()
    }

    /// Every available theme's palette, for the Settings color pickers.
    fn all_theme_palettes(&self) -> Vec<(String, Vec<(String, String)>)> {
        self.available_themes()
            .into_iter()
            .map(|name| {
                let pal = self.theme_palette_for(&name);
                (name, pal)
            })
            .collect()
    }

    /// Installed font filenames (`.ttf`/`.otf`) for the font picker.
    fn available_fonts(&self) -> Vec<String> {
        let mut dirs_list: Vec<PathBuf> = Vec::new();
        if let Some(d) = dirs::font_dir() {
            dirs_list.push(d);
        }
        #[cfg(target_os = "macos")]
        {
            dirs_list.push(PathBuf::from("/Library/Fonts"));
            dirs_list.push(PathBuf::from("/System/Library/Fonts"));
        }
        #[cfg(target_os = "linux")]
        {
            dirs_list.push(PathBuf::from("/usr/share/fonts"));
        }
        #[cfg(windows)]
        {
            dirs_list.push(PathBuf::from(r"C:\Windows\Fonts"));
        }
        let mut names: Vec<String> = Vec::new();
        for dir in dirs_list {
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for entry in rd.flatten() {
                    let p = entry.path();
                    let ext_ok = p.extension().is_some_and(|x| x == "ttf" || x == "otf");
                    if ext_ok {
                        if let Some(f) = p.file_name() {
                            let s = f.to_string_lossy().into_owned();
                            if !names.contains(&s) {
                                names.push(s);
                            }
                        }
                    }
                }
            }
        }
        names.sort();
        names
    }

    /// Installed shells for the terminal-shell picker. Looks up the common
    /// shells on `$PATH` (plus `/etc/shells` on Unix); full paths, deduped.
    fn available_shells(&self) -> Vec<String> {
        let mut found: Vec<String> = Vec::new();
        let push = |p: String, v: &mut Vec<String>| {
            if !v.contains(&p) {
                v.push(p);
            }
        };
        #[cfg(not(windows))]
        {
            for name in ["bash", "zsh", "ksh", "tcsh", "csh", "fish", "sh", "dash"] {
                if let Some(p) = find_in_path(name) {
                    push(p, &mut found);
                }
            }
            if let Ok(text) = std::fs::read_to_string("/etc/shells") {
                for line in text.lines() {
                    let line = line.trim();
                    if line.starts_with('/') && std::path::Path::new(line).is_file() {
                        push(line.to_string(), &mut found);
                    }
                }
            }
        }
        #[cfg(windows)]
        {
            for name in ["powershell.exe", "pwsh.exe", "cmd.exe"] {
                if let Some(p) = find_in_path(name) {
                    push(p, &mut found);
                }
            }
        }
        found
    }

    /// Open dired at `path` (used when ruster is launched with a directory).
    pub fn open_dir(&mut self, path: &std::path::Path) {
        self.open_dired(Some(path.to_string_lossy().into_owned()));
    }

    /// Open a read-only buffer listing config load/validation errors.
    fn open_config_errors(&mut self) {
        let text = if self.config_errors.is_empty() {
            "No config errors — everything loaded cleanly.".to_string()
        } else {
            let mut s = format!("{} config problem(s):\n\n", self.config_errors.len());
            for e in &self.config_errors {
                s.push_str("  • ");
                s.push_str(e);
                s.push('\n');
            }
            s.push_str("\nInvalid values fall back to their defaults; edit config.lua or use :settings.");
            s
        };
        let id = self
            .ws
            .borrow_mut()
            .buffers
            .create_special(SpecialKind::ConfigErrors, "*config-errors*");
        {
            let mut w = self.ws.borrow_mut();
            if let Some(doc) = w.buffers.get_mut(id) {
                doc.buffer = Buffer::from_str(&text);
            }
            w.set_active_buffer(id);
        }
    }

    /// Open a new embedded terminal in the active window and focus it.
    fn open_terminal(&mut self) {
        // Config `terminal_shell` overrides the platform default when set.
        let (shell, args) = match &self.config.terminal_shell {
            Some(s) if !s.is_empty() => (s.clone(), Vec::new()),
            _ => ruster_terminal::default_shell(),
        };
        let scrollback = self.config.terminal_scrollback as usize;
        // Spawn at a default size; the first render resizes it to the window.
        match TerminalSession::spawn(&shell, &args, 80, 24, scrollback) {
            Ok(session) => {
                let id = self
                    .ws
                    .borrow_mut()
                    .buffers
                    .create_special(SpecialKind::Terminal, "*terminal*");
                self.ws.borrow_mut().set_active_buffer(id);
                self.terminals.insert(id, session);
                // Honor terminal.default_mode ("insert" focuses the shell).
                self.terminal_focused = self.config.terminal_default_mode != "normal";
                self.notify.push(Notification::new(ruster_core::message::MessageLevel::Info, ruster_core::message::MessageSource::System, "terminal: Ctrl-\\ to leave, i to re-enter".to_string()));
            }
            Err(e) => { self.notify.push(Notification::new(ruster_core::message::MessageLevel::Info, ruster_core::message::MessageSource::System, format!("terminal: {e}"))); },
        }
    }

    /// Forward one key press to a focused terminal's PTY. `Ctrl-\` switches to
    /// Terminal-Normal mode (vim motions / visual / yank over the output).
    fn handle_terminal_key(&mut self, ck: crossterm::event::KeyEvent, bid: BufferId) {
        if ck.modifiers.contains(KeyModifiers::CONTROL) {
            if let KeyCode::Char('\\') = ck.code {
                self.enter_terminal_normal(bid);
                return;
            }
        }
        if let Some((key, mods)) = term_key_from_crossterm(ck) {
            let bytes = encode_key(key, mods);
            if let Some(session) = self.terminals.get(&bid) {
                let _ = session.write_input(&bytes);
            }
        }
    }

    /// Leave terminal-insert for Terminal-Normal: snapshot the visible grid into
    /// the (read-only) buffer so the vim layer's motions, visual selection and
    /// yank operate over the terminal's output. `i`/`a`/Enter resume insert.
    fn enter_terminal_normal(&mut self, bid: BufferId) {
        if let Some(session) = self.terminals.get(&bid) {
            let grid = session.snapshot();
            let mut lines: Vec<String> =
                (0..grid.rows).map(|r| grid.row_text(r).trim_end().to_string()).collect();
            while lines.last().map(|l| l.is_empty()).unwrap_or(false) {
                lines.pop();
            }
            let text = lines.join("\n");
            let cursor_line = grid.cursor.0.min(lines.len().saturating_sub(1));
            let mut w = self.ws.borrow_mut();
            if let Some(doc) = w.buffers.get_mut(bid) {
                doc.buffer = Buffer::from_str(&text);
                let pos = doc.buffer.line_start_char(cursor_line);
                if w.active_buffer() == bid {
                    w.windows.active_window_mut().cursors = CursorSet::single(pos);
                }
            }
        }
        self.terminal_focused = false;
        self.vim = VimState::new();
        self.notify.push(Notification::new(ruster_core::message::MessageLevel::Info, ruster_core::message::MessageSource::System, "terminal: NORMAL — motions/visual/y to yank, i to resume".to_string()));
    }

    /// Handle a key in a dired buffer. Returns true if the key was consumed
    /// (movement keys fall through to vim so j/k/gg/G still work).
    /// Feed a key to the active dired listing, performing whatever it asks for.
    /// Returns `false` for keys it did not claim, which must fall through to the
    /// main handler so `:`, `/`, `n` and the leader keep working in a listing.
    fn handle_dired_key(&mut self, ck: crossterm::event::KeyEvent) -> bool {
        // The workspace borrow is confined to this block: `open_path` and the
        // refresh helpers below re-borrow it internally.
        let resp = {
            let mut w = self.ws.borrow_mut();
            self.dired.handle_key(ck, &mut w, &mut self.notify)
        };
        match resp {
            DiredResponse::Ignored => false,
            DiredResponse::Handled => true,
            DiredResponse::ShowHelp => {
                self.hover = Some(crate::dired::help_lines());
                true
            }
            DiredResponse::Prompt(p) => {
                self.file_prompt = Some(p);
                true
            }
            DiredResponse::OpenFile(path) => {
                self.open_path(&path, None);
                true
            }
        }
    }

    /// Handle a key while a dired file-operation prompt is active.
    fn handle_dialog_key(&mut self, ck: crossterm::event::KeyEvent) {
        let Some(d) = self.dialog.as_mut() else { return };
        match d.handle_key(ck) {
            DialogResponse::Pending => {}
            DialogResponse::Cancelled => {
                self.dialog = None;
                // Drop the callback: a cancelled dialog reports nothing.
                self.lua.discard_dialog_callback();
            }
            DialogResponse::Submitted { button } => {
                let values = d.values();
                self.dialog = None;
                if self.pending_confirm.is_some() {
                    // ruster's own confirmation, not a plugin's dialog.
                    self.run_pending_confirm(button.as_deref());
                    return;
                }
                // Hand the values to the plugin that opened it. Without this the
                // dialog is a display, not an API.
                self.lua.fire_dialog_submit(&values, button.as_deref());
            }
        }
    }

    /// Feed a key to the active file prompt, then commit or drop it.
    ///
    /// The refresh is dispatched on the prompt's recorded origin, so the surface
    /// that opened it is the one updated — including for delete, which used to
    /// return early and refresh nothing.
    fn handle_file_prompt_key(&mut self, ck: crossterm::event::KeyEvent) {
        let Some(p) = self.file_prompt.as_mut() else { return };
        match p.press(ck) {
            PromptStep::Pending => {}
            PromptStep::Cancelled => self.file_prompt = None,
            PromptStep::Commit => {
                let prompt = match self.file_prompt.take() {
                    Some(p) => p,
                    None => return,
                };
                if let Some((level, msg)) = file_prompt::commit(&prompt) {
                    self.notify.push(Notification::new(
                        level,
                        ruster_core::message::MessageSource::Echo,
                        msg,
                    ));
                }
                match prompt.origin {
                    PromptOrigin::Dired => self.dired_refresh_current(),
                    PromptOrigin::Sidebar => self.sidebar.refresh(),
                }
            }
        }
    }

    /// Reload the active dired buffer's listing (after a mutation).
    fn dired_refresh_current(&mut self) {
        self.dired.refresh_current(&mut self.ws.borrow_mut());
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
        // The command palette can dock at the bottom (which-key area) or float
        // centered, per `whichkey.command_palette`.
        if self.config.command_palette == "bottom" {
            p.placement = ruster_render::PickerPlacement::Bottom;
        }
        for c in seed.chars() {
            p.push_char(c);
        }
        self.picker = Some(p);
    }

    /// Switch to the Dashboard buffer, or create a pinned one if none exists.
    fn ensure_dashboard_buffer(&mut self) {
        let mut w = self.ws.borrow_mut();
        let existing = w.buffers.ids().iter().copied().any(|id| {
            w.buffers.get(id).is_some_and(|d| d.pinned && matches!(d.kind, ruster_core::document::DocKind::Special(ruster_core::document::SpecialKind::Dashboard)))
        });
        if !existing {
            let id = w.buffers.create_special(ruster_core::document::SpecialKind::Dashboard, "Dashboard");
            if let Some(doc) = w.buffers.get_mut(id) {
                doc.pinned = true;
            }
        }
    }

    fn is_dashboard_active(&self) -> bool {
        let w = self.ws.borrow();
        let active = w.active_doc();
        active.file_path.is_none()
            && (matches!(active.kind, DocKind::Scratch)
                || matches!(active.kind, DocKind::Special(SpecialKind::Dashboard)))
    }

    fn open_dashboard(&mut self) {
        let mut w = self.ws.borrow_mut();
        let existing = w.buffers.ids().iter().copied().find(|&id| {
            w.buffers.get(id).is_some_and(|d| d.pinned && matches!(d.kind, ruster_core::document::DocKind::Special(ruster_core::document::SpecialKind::Dashboard)))
        });
        match existing {
            Some(id) => w.set_active_buffer(id),
            None => {
                let id = w.buffers.create_special(ruster_core::document::SpecialKind::Dashboard, "Dashboard");
                if let Some(doc) = w.buffers.get_mut(id) {
                    doc.pinned = true;
                }
                w.set_active_buffer(id);
            }
        }
    }

    /// Push a message to the log and optionally show it in the status line.
    fn push_message(&mut self, level: ruster_core::message::MessageLevel, source: ruster_core::message::MessageSource, text: String) {
        self.messages.push(level, source, text.clone());
    }

    /// Ensure the pinned `*messages*` buffer exists, returning its id.
    fn ensure_messages_buffer(&mut self) -> BufferId {
        if let Some(id) = self.messages_buf {
            if self.ws.borrow().buffers.get(id).is_some() {
                return id;
            }
        }
        let id = self.ws.borrow_mut().buffers.create_special(
            ruster_core::document::SpecialKind::Message,
            "*messages*",
        );
        if let Some(doc) = self.ws.borrow_mut().buffers.get_mut(id) {
            doc.pinned = true;
        }
        self.messages_buf = Some(id);
        id
    }

    /// Rebuild the messages buffer text from the message log with current filters.
    fn refresh_messages_buffer(&mut self, id: BufferId) {
        use ruster_core::message::MessageLevel;
        let entries = self.messages.filtered(self.messages_filter_source, self.messages_filter_level);
        let mut text = String::new();
        for entry in &entries {
            let level_tag = match entry.level {
                MessageLevel::Error => "ERR ",
                MessageLevel::Warning => "WARN",
                MessageLevel::Success => " OK ",
                MessageLevel::Info => "INFO",
            };
            text.push_str(&format!(
                "[{}] {} {}\n",
                entry.source.label().to_uppercase(),
                level_tag,
                entry.text
            ));
        }
        let mut w = self.ws.borrow_mut();
        if let Some(doc) = w.buffers.get_mut(id) {
            doc.buffer = ruster_core::buffer::Buffer::from_str(&text);
        }
    }

    /// Open the messages buffer in the active window.
    fn open_messages(&mut self) {
        let id = self.ensure_messages_buffer();
        self.refresh_messages_buffer(id);
        self.ws.borrow_mut().set_active_buffer(id);
    }

    /// Open or focus the pinned `*noice*` split buffer populated from history.
    fn open_noice_split(&mut self) {
        if !self.notify.split_enabled() {
            self.notify.push(Notification::new(
                ruster_core::message::MessageLevel::Warning,
                ruster_core::message::MessageSource::Echo,
                "noice.split is disabled".to_string(),
            ));
            return;
        }
        let buf_name = "*noice*";
        let existing = self.ws.borrow().buffers.ids().iter().copied().find(|&id| {
            self.ws.borrow().buffers.get(id).is_some_and(|d| d.name == buf_name)
        });
        if let Some(id) = existing {
            self.ws.borrow_mut().set_active_buffer(id);
            return;
        }
        let history = self.notify.history().to_vec();
        let level_icon = |level: ruster_core::message::MessageLevel| -> &'static str {
            match level {
                ruster_core::message::MessageLevel::Info => "",
                ruster_core::message::MessageLevel::Success => "✓",
                ruster_core::message::MessageLevel::Warning => "⚠",
                ruster_core::message::MessageLevel::Error => "✗",
            }
        };
        let text: String = history.iter()
            .map(|n| format!("[{}] {} {}", level_icon(n.level), n.source.label(), n.text))
            .collect::<Vec<_>>()
            .join("\n");
        let id = self.ws.borrow_mut().buffers.create_special(
            ruster_core::document::SpecialKind::Message,
            buf_name,
        );
        if let Some(doc) = self.ws.borrow_mut().buffers.get_mut(id) {
            doc.pinned = true;
            doc.buffer = ruster_core::buffer::Buffer::from_str(&text);
        }
        self.ws.borrow_mut().set_active_buffer(id);
    }

    /// Open a side-by-side diff of the active file against HEAD.
    ///
    /// Two read-only panes in a vertical split: HEAD on the left, the working
    /// tree on the right, aligned so the same code sits on the same screen row
    /// even where a hunk changes the line count.
    fn open_diffview(&mut self) {
        let Some(root) = self.project_root.clone() else {
            self.echo_warn("Not in a project".to_string());
            return;
        };
        // Resolve inside the narrowest scope, then act with the borrow dropped.
        let target = {
            let w = self.ws.borrow();
            w.buffers
                .get(w.active_buffer())
                .and_then(|d| d.file_path.clone().map(|p| (p, d.buffer.to_string())))
        };
        let Some((path, working)) = target else {
            self.echo_warn("No file in this window to diff".to_string());
            return;
        };
        if !ruster_git::is_repo(&root) {
            self.echo_warn("Not a git repository".to_string());
            return;
        }
        let Some(head) = ruster_git::file_at_head(&root, &path) else {
            self.echo_warn(format!(
                "{} is not tracked at HEAD",
                path.file_name().unwrap_or_default().to_string_lossy()
            ));
            return;
        };
        let hunks = ruster_git::diff_hunks_two_sided(&root, &path).unwrap_or_default();
        let (old_lines, new_lines): (Vec<&str>, Vec<&str>) =
            (head.lines().collect(), working.lines().collect());
        if hunks.is_empty() {
            self.echo(format!(
                "{} matches HEAD",
                path.file_name().unwrap_or_default().to_string_lossy()
            ));
            return;
        }
        let rows = ruster_git::align(&hunks, old_lines.len() as u32, new_lines.len() as u32);

        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        self.close_diffview();
        let left = self.make_diff_pane(
            &format!("*diff HEAD: {name}*"),
            &diff_pane_text(&rows, &old_lines, false),
        );
        let right = self.make_diff_pane(
            &format!("*diff working: {name}*"),
            &diff_pane_text(&rows, &new_lines, true),
        );

        // Split first, then fill: `split` copies the active window's buffer, so
        // the panes have to be assigned afterwards.
        let mut w = self.ws.borrow_mut();
        w.set_active_buffer(left);
        let new_win = w.windows.split(ruster_core::windows::SplitDir::Vertical);
        if let Some(win) = w.windows.window_mut(new_win) {
            win.buffer = right;
        }
        drop(w);
        let n = hunks.len();
        self.echo(format!("{n} hunk{} in {name}", if n == 1 { "" } else { "s" }));
    }

    /// Create one read-only pane buffer for [`open_diffview`](Self::open_diffview).
    fn make_diff_pane(&mut self, name: &str, text: &str) -> BufferId {
        let mut w = self.ws.borrow_mut();
        let id = w.buffers.create_special(ruster_core::document::SpecialKind::Diff, name);
        if let Some(doc) = w.buffers.get_mut(id) {
            doc.buffer = ruster_core::buffer::Buffer::from_str(text);
        }
        id
    }

    /// Drop any previous diff buffers, so re-running `:Diffview` replaces the
    /// old panes instead of accumulating them.
    fn close_diffview(&mut self) {
        let stale: Vec<BufferId> = {
            let w = self.ws.borrow();
            w.buffers
                .ids()
                .iter()
                .copied()
                .filter(|&id| self.is_diff_buffer(&w, id))
                .collect()
        };
        let mut w = self.ws.borrow_mut();
        for id in stale {
            w.buffers.close(id);
        }
    }

    fn is_diff_buffer(&self, w: &ruster_core::workspace::Workspace, id: BufferId) -> bool {
        w.buffers.get(id).is_some_and(|d| {
            matches!(
                d.kind,
                ruster_core::document::DocKind::Special(ruster_core::document::SpecialKind::Diff)
            )
        })
    }

    /// Keep the two diff panes in step.
    ///
    /// Syncs the *cursor line*, not `scroll_top`. Scroll is recomputed inside
    /// `render` from the cursor, so assigning it here would be overwritten on
    /// the very next frame — and the follower's own clamp would drag it back to
    /// wherever its cursor had been left. Because the panes are row-aligned by
    /// construction, putting both cursors on the same row makes that same clamp
    /// produce the same scroll for both, and the cursor ends up where the reader
    /// is looking rather than stranded at the top.
    ///
    /// Whichever pane holds the cursor leads. The pair is identified by buffer
    /// kind rather than stored window ids, so closing and reopening a pane
    /// cannot leave a stale pairing behind.
    fn sync_diff_scroll(&mut self) {
        let mut w = self.ws.borrow_mut();
        let active = w.windows.active();
        // `compute_rects` is the only enumeration the window tree offers; the
        // area is irrelevant here since only the ids are used.
        let diff_wins: Vec<_> = w
            .windows
            .compute_rects(CoreRect::new(0, 0, 1000, 1000))
            .into_iter()
            .map(|(id, _)| id)
            .filter(|&id| {
                w.windows.window(id).is_some_and(|win| {
                    w.buffers.get(win.buffer).is_some_and(|d| {
                        matches!(
                            d.kind,
                            ruster_core::document::DocKind::Special(
                                ruster_core::document::SpecialKind::Diff
                            )
                        )
                    })
                })
            })
            .collect();
        if diff_wins.len() != 2 || !diff_wins.contains(&active) {
            return;
        }
        // The line the leader's cursor sits on, in display rows.
        let Some(line) = w.windows.window(active).and_then(|win| {
            let head = win.cursors.primary().head;
            w.buffers.get(win.buffer).map(|d| d.buffer.char_to_line(head))
        }) else {
            return;
        };
        for id in diff_wins.into_iter().filter(|&id| id != active) {
            let Some(buf) = w.windows.window(id).map(|win| win.buffer) else { continue };
            // Clamp: the panes are the same height, but a buffer can be empty.
            let Some(off) = w.buffers.get(buf).map(|d| {
                let last = d.buffer.line_count().saturating_sub(1);
                d.buffer.line_start_char(line.min(last))
            }) else {
                continue;
            };
            if let Some(win) = w.windows.window_mut(id) {
                win.cursors = ruster_core::cursor::CursorSet::single(off);
            }
        }
    }

    /// Apply a filter string to the messages buffer (`:msgs build`, `:msgs/err`).
    fn apply_messages_filter(&mut self, filter: &str) {
        use ruster_core::message::{MessageLevel, MessageSource};
        let lower = filter.to_lowercase();
        self.messages_filter_source = match lower.as_str() {
            "build" => Some(MessageSource::Build),
            "test" => Some(MessageSource::Test),
            "task" => Some(MessageSource::Task),
            "lsp" => Some(MessageSource::Lsp),
            "echo" => Some(MessageSource::Echo),
            "system" => Some(MessageSource::System),
            "all" | "clear" => None,
            _ => self.messages_filter_source,
        };
        self.messages_filter_level = match lower.as_str() {
            "err" | "error" => Some(MessageLevel::Error),
            "warn" | "warning" => Some(MessageLevel::Warning),
            "ok" | "success" => Some(MessageLevel::Success),
            "info" => Some(MessageLevel::Info),
            "all" | "clear" => None,
            _ => self.messages_filter_level,
        };
        if let Some(id) = self.messages_buf {
            self.refresh_messages_buffer(id);
        }
    }

    /// Open a picker listing recent projects. Selecting one switches the working
    /// project root, sets `runner_root`, and opens dired at that root.
    fn open_projects(&mut self) {
        let recent: Vec<PathBuf> = ruster_config_dir()
            .map(|d| ruster_project::recent_projects(&d))
            .unwrap_or_default();
        if recent.is_empty() {
            self.echo_warn("No recent projects".to_string());
            return;
        }
        let items: Vec<PickerItem> = recent
            .iter()
            .map(|p| {
                let name = p.file_name().map(|n| n.to_string_lossy()).unwrap_or_default().to_string();
                PickerItem::new(name, PickerAction::OpenPath(p.clone()))
            })
            .collect();
        self.picker = Some(PickerState::new("Projects", items));
    }

    /// Record the current project root in the recent-projects list.
    fn record_current_project(&self) {
        if let (Some(ref state_dir), Some(root)) = (ruster_config_dir(), self.project_root.as_ref()) {
            ruster_project::record_recent(state_dir, root, 30);
        }
    }

    /// Push a plain informational message.
    fn echo(&mut self, msg: impl Into<String>) {
        self.echo_at(ruster_core::message::MessageLevel::Info, msg);
    }

    /// A successful outcome the user asked for (`Created …`, `Deleted …`).
    fn echo_success(&mut self, msg: impl Into<String>) {
        self.echo_at(ruster_core::message::MessageLevel::Success, msg);
    }

    /// Something the user should notice but that isn't a failure.
    fn echo_warn(&mut self, msg: impl Into<String>) {
        self.echo_at(ruster_core::message::MessageLevel::Warning, msg);
    }

    /// A failed operation. Errors route to the notify panel and are persistent.
    fn echo_error(&mut self, msg: impl Into<String>) {
        self.echo_at(ruster_core::message::MessageLevel::Error, msg);
    }

    fn echo_at(&mut self, level: ruster_core::message::MessageLevel, msg: impl Into<String>) {
        self.notify.push(Notification::new(
            level,
            ruster_core::message::MessageSource::Echo,
            msg.into(),
        ));
    }

    /// Toggle the file-explorer sidebar on/off, rooting it at the project root
    /// (or the current directory) on first open.
    fn toggle_sidebar(&mut self) {
        if self.sidebar.is_open() {
            self.close_sidebar();
        } else {
            let root = self
                .project_root
                .clone()
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_else(|| PathBuf::from("."));
            self.sidebar.open(root);
            self.record_current_project();
            self.echo("Sidebar opened");
        }
    }

    /// Close the sidebar and drop the tree.
    fn close_sidebar(&mut self) {
        self.sidebar.close();
        self.echo("Sidebar closed");
    }

    /// Feed a key to the focused sidebar, performing whatever it asks for.
    /// Returns `false` for keys it did not claim, which must fall through to the
    /// main handler (e.g. `SPC e` to close it).
    fn handle_sidebar_key(&mut self, ck: crossterm::event::KeyEvent) -> bool {
        match self.sidebar.handle_key(ck) {
            SidebarResponse::Ignored => false,
            SidebarResponse::Handled => true,
            SidebarResponse::Close => {
                self.close_sidebar();
                true
            }
            SidebarResponse::Prompt(p) => {
                self.file_prompt = Some(p);
                true
            }
            SidebarResponse::OpenFile(path) => {
                self.open_path(&path, None);
                true
            }
        }
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
    /// Signs for `buf_id`'s git hunks: `+` added, `~` modified, `_` removed.
    ///
    /// Empty when `git.signs` is off or the buffer has no hunks, which is the
    /// common case and must cost nothing.
    fn git_signs_for(&self, buf_id: BufferId) -> ruster_render::SignsView {
        let hunks = self.git.hunks(buf_id);
        if !self.git.enabled() || hunks.is_empty() {
            return ruster_render::SignsView::default();
        }
        let mut signs: Vec<(u16, char, ruster_render::Color)> = Vec::new();
        for h in hunks {
            let (glyph, color) = match h.kind {
                ruster_git::HunkKind::Added => ('+', ruster_syntax::sign_style("added").fg),
                ruster_git::HunkKind::Modified => ('~', ruster_syntax::sign_style("modified").fg),
                ruster_git::HunkKind::Removed => ('_', ruster_syntax::sign_style("removed").fg),
            };
            match h.kind {
                // A deletion has no lines of its own — mark the boundary.
                ruster_git::HunkKind::Removed => signs.push((h.start as u16, glyph, color)),
                _ => signs.extend(h.lines().map(|l| (l as u16, glyph, color))),
            }
        }
        ruster_render::SignsView { width: 1, signs }
    }

    /// Kick off a background `git diff` for `buf_id`.
    ///
    /// Non-blocking like the LSP and runner paths: a thread writes back through
    /// an mpsc channel that `render` drains. Silently does nothing when the file
    /// is untracked, outside a repo, or git is missing.
    fn refresh_git_hunks(&mut self, buf_id: BufferId) {
        if !self.git.enabled() {
            return;
        }
        let Some(path) = self.ws.borrow().buffers.get(buf_id).and_then(|d| d.file_path.clone())
        else {
            return;
        };
        let root = match self.project_root.clone() {
            Some(r) => r,
            None => match path.parent() {
                Some(p) => p.to_path_buf(),
                None => return,
            },
        };
        self.git.request(buf_id, path, root);
    }

    /// Take whatever the git workers finished since the last frame.
    fn drain_git_hunks(&mut self) {
        self.git.drain();
    }

    /// `]h` / `[h` — jump to the next/previous hunk, wrapping.
    fn jump_hunk(&mut self, forward: bool) {
        let buf_id = self.ws.borrow().active_buffer();
        let hunks = self.git.hunks(buf_id).to_vec();
        if hunks.is_empty() {
            self.echo("No git hunks");
            return;
        }
        let line = {
            let w = self.ws.borrow();
            w.buffer().char_to_line(w.primary_head()) as u32
        };
        let target = if forward {
            ruster_git::next_hunk(&hunks, line)
        } else {
            ruster_git::prev_hunk(&hunks, line)
        };
        if let Some(h) = target {
            let off = self.ws.borrow().buffer().line_start_char(h.start as usize);
            self.ws.borrow_mut().execute(Action::Move(Motion::To(off)));
        }
    }

    /// Drop every per-buffer cache keyed by `id`.
    ///
    /// Call this whenever a buffer is closed. Each of these maps is keyed by
    /// `BufferId` and none of them was cleaned up before, so they grew for the
    /// life of the session — and a leaked `TerminalSession` keeps its child
    /// process alive, since the kill happens in its `Drop`.
    fn forget_buffer(&mut self, id: BufferId) {
        self.dired.forget(id);
        self.syntax.remove(&id);
        self.lsp.forget(id);
        self.terminals.remove(&id);
        // Added with the extraction: this sweep cleared four caches and missed
        // the git hunks, so a long session of opening and closing files grew
        // that map without bound.
        self.git.forget(id);
    }

    fn delete_active_buffer(&mut self) {
        let mut w = self.ws.borrow_mut();
        let cur = w.active_buffer();
        let other = w.buffers.ids().iter().copied().find(|&id| id != cur);
        match other {
            Some(o) => {
                if w.buffers.get(cur).map(|d| d.modified).unwrap_or(false) {
                    drop(w);
                    self.echo_warn("E89: buffer modified (add ! to override)".to_string());
                    return;
                }
                w.set_active_buffer(o);
                w.buffers.close(cur);
                drop(w);
                self.forget_buffer(cur);
            }
            None => {
                drop(w);
                self.echo_warn("E514: cannot close last buffer".to_string());
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
                self.echo_warn(format!("No macro in @{}", reg));
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
        /// What the key did, decided while the picker is borrowed and acted on
        /// after — a theme preview needs `&mut self`, which that borrow forbids.
        enum Step {
            /// Picker stays open; the selection may have moved.
            Stay,
            Cancel,
            Accept(Option<PickerAction>),
        }

        let ctrl = ck.modifiers.contains(KeyModifiers::CONTROL);
        // A theme picker repaints the editor as the selection moves, so the
        // choice is judged against real content rather than a swatch.
        let previewing = self.theme_before_preview.is_some();

        let step = {
            let picker = match self.picker.as_mut() {
                Some(p) => p,
                None => return,
            };
            match ck.code {
                KeyCode::Esc => Step::Cancel,
                KeyCode::Enter => Step::Accept(picker.accept()),
                KeyCode::Up => {
                    picker.move_selection(-1);
                    Step::Stay
                }
                KeyCode::Down => {
                    picker.move_selection(1);
                    Step::Stay
                }
                KeyCode::Char('p') if ctrl => {
                    picker.move_selection(-1);
                    Step::Stay
                }
                KeyCode::Char('n') if ctrl => {
                    picker.move_selection(1);
                    Step::Stay
                }
                KeyCode::Backspace => {
                    picker.pop_char();
                    Step::Stay
                }
                KeyCode::Char(c) if !ctrl => {
                    picker.push_char(c);
                    Step::Stay
                }
                _ => Step::Stay,
            }
        };

        match step {
            Step::Stay => {
                if previewing {
                    self.preview_selected_theme();
                }
            }
            Step::Cancel => {
                self.picker = None;
                // Cancelling a preview puts the previous theme back.
                if let Some(prev) = self.theme_before_preview.take() {
                    self.apply_theme(&prev);
                }
            }
            Step::Accept(action) => {
                self.picker = None;
                // Accepting keeps whatever is on screen, so there is nothing to
                // restore.
                self.theme_before_preview = None;
                if let Some(action) = action {
                    self.dispatch_picker_action(action);
                }
            }
        }
    }

    /// `:Themes` — pick a theme, previewing each as the selection moves.
    fn open_themes_picker(&mut self) {
        let items: Vec<PickerItem> = self
            .available_themes()
            .into_iter()
            .map(|name| {
                PickerItem::new(name.clone(), PickerAction::SetTheme(name))
            })
            .collect();
        self.theme_before_preview = Some(self.config.theme.clone());
        self.picker = Some(PickerState::new("Themes", items));
        // Preview the row the picker opens on, so the list is live immediately.
        self.preview_selected_theme();
    }

    /// Apply the theme under the picker's cursor without committing it.
    fn preview_selected_theme(&mut self) {
        let name = self.picker.as_mut().and_then(|p| match p.selected_action() {
            Some(PickerAction::SetTheme(n)) => Some(n),
            _ => None,
        });
        if let Some(name) = name {
            self.apply_theme(&name);
        }
    }

    /// Resolve `name` and repaint with it. Does not persist — `:w` in the
    /// settings page or accepting the picker is what makes it stick.
    fn apply_theme(&mut self, name: &str) {
        self.config.theme = name.to_string();
        self.config.colors =
            resolve_theme_colors(&self.lua, &self.config.theme, &self.config.color_overrides);
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
            PickerAction::SetTheme(name) => {
                // Preview already applied it; this makes the choice the one that
                // `:w` in the settings page would persist.
                self.apply_theme(&name);
                self.echo(format!("Theme: {name}"));
            }
            PickerAction::RunCmd(cmd) => match self.parse_cmdline(&cmd) {
                Ok(a) => self.apply_cmd(a),
                Err(e) => { self.echo(e); },
            },
            PickerAction::RunTask(name) => self.run_task(&name),
        }
    }

    /// Generate path completion candidates for the given path prefix.
    fn generate_completion_candidates(&self, path_prefix: &str) -> Vec<String> {
        let (dir, file_prefix) = if path_prefix.contains('/') {
            let (dir_part, prefix_part) =
                path_prefix.rsplit_once('/').unwrap_or((path_prefix, ""));
            let base = resolve_path(dir_part, &std::env::current_dir().unwrap_or_default());
            (base, prefix_part.to_string())
        } else {
            let ws = self.ws.borrow();
            let base = ws
                .active_doc()
                .file_path
                .as_ref()
                .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            (base, path_prefix.to_string())
        };

        let mut candidates = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with(&file_prefix) || file_prefix.is_empty() {
                    let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                    let display = if is_dir {
                        format!("{}/", name)
                    } else {
                        name
                    };
                    if path_prefix.contains('/') {
                        let dir_part =
                            path_prefix.rsplit_once('/').map(|x| x.0).unwrap_or("");
                        candidates.push(format!("{}/{}", dir_part, display));
                    } else {
                        candidates.push(display);
                    }
                }
            }
        }
        // Sort: directories first, then alphabetically
        candidates.sort_by(|a, b| {
            let a_dir = a.ends_with('/');
            let b_dir = b.ends_with('/');
            b_dir.cmp(&a_dir).then_with(|| a.cmp(b))
        });
        candidates
    }

    /// Open `path` into a buffer shown in the active window. When `at` is given,
    /// move the cursor to that 1-indexed (line, col).
    fn open_path(&mut self, path: &std::path::Path, at: Option<(usize, usize)>) {
        let content = std::fs::read_to_string(path).unwrap_or_default();
        let id = self.ws.borrow_mut().buffers.open_file(path.to_path_buf(), content);
        self.ws.borrow_mut().set_active_buffer(id);
        self.refresh_git_hunks(id);
        // Detect the project root for this file and update the cached root.
        let new_root = ruster_project::project_root(path);
        if new_root.is_some() && new_root != self.project_root {
            self.project_root = new_root.clone();
            if let Some(ref state_dir) = ruster_config_dir() {
                if let Some(ref root) = self.project_root {
                    ruster_project::record_recent(state_dir, root, 30);
                }
            }
        }
        if let Some((line, col)) = at {
            let pos = {
                let w = self.ws.borrow();
                let buf = w.buffer();
                let l = line.saturating_sub(1).min(buf.line_count().saturating_sub(1));
                buf.line_start_char(l) + col.saturating_sub(1)
            };
            self.ws.borrow_mut().execute(Action::Move(Motion::To(pos)));
        }
        self.sidebar.reveal(path);
    }

    /// Advance the pending Space-leader sequence with the next key.
    /// Second key of a `g` sequence: LSP goto commands, or replay a native
    /// g-motion (`gg`/`g-`/`g+`/…) into the vim layer.
    fn handle_g_key(&mut self, ck: crossterm::event::KeyEvent) {
        match ck.code {
            KeyCode::Char('d') => self.lsp_definition(),
            KeyCode::Char('r') => self.lsp_references(),
            KeyCode::Char('h') => self.lsp_hover(),
            KeyCode::Esc | KeyCode::Backspace => {} // cancel
            other => {
                self.feed_key_to_vim(KeyCode::Char('g'));
                self.feed_key_to_vim(other);
            }
        }
    }

    /// Re-inject a key into the normal vim path (used to replay native g-motions
    /// after the `g` menu decided not to handle the sequence itself).
    fn feed_key_to_vim(&mut self, code: KeyCode) {
        self.g_replaying = true;
        self.handle_key(crossterm::event::KeyEvent::new(code, KeyModifiers::NONE));
        self.g_replaying = false;
    }

    fn handle_leader_key(&mut self, ck: crossterm::event::KeyEvent) {
        // Backspace pops the last key from the leader sequence. When the
        // sequence becomes empty, cancel leader mode entirely.
        if ck.code == KeyCode::Backspace {
            if let Some(seq) = &mut self.leader_pending {
                seq.pop();
                if seq.is_empty() {
                    self.leader_pending = None;
                }
            }
            return;
        }
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
                self.echo("Use :rename <new-name>".to_string());
            }
            LeaderAction::DocumentSymbol => self.lsp_document_symbols(),
            LeaderAction::Diagnostics => self.open_diagnostics_picker(),
            LeaderAction::IncomingCalls => self.lsp_call_hierarchy(true),
            LeaderAction::OutgoingCalls => self.lsp_call_hierarchy(false),
            LeaderAction::BufferDelete => self.delete_active_buffer(),
            LeaderAction::Terminal => self.open_terminal(),
            LeaderAction::Settings => self.open_settings(),
            LeaderAction::ToggleNumber => {
                self.config.number = !self.config.number;
                self.echo(format!("number: {}", self.config.number));
            }
            LeaderAction::ToggleRelative => {
                self.config.relativenumber = !self.config.relativenumber;
                self.echo(format!("relativenumber: {}", self.config.relativenumber));
            }
            LeaderAction::Grep => {
                // Seed the cmdline for a ripgrep pattern.
                self.vim.set_cmdline(":Rg ");
                self.echo("Type a pattern and press Enter".to_string());
            }
            LeaderAction::Build => self.run_build(),
            LeaderAction::Test => self.run_test(),
            LeaderAction::Tasks => self.open_task_picker(),
            LeaderAction::Dashboard => self.open_dashboard(),
            LeaderAction::Messages => self.open_messages(),
            LeaderAction::Projects => self.open_projects(),
            LeaderAction::Trouble => self.open_trouble(),
            LeaderAction::GitStatus => self.open_git_status(),
            LeaderAction::GitCommit => self.open_git_commit(),
            LeaderAction::Diffview => self.open_diffview(),
            LeaderAction::GitStaged => self.open_git_staged(),
            LeaderAction::GitStageHunk => self.git_stage_hunk(),
            LeaderAction::GitPush => self.confirm_git_remote("Push"),
            LeaderAction::GitPull => self.confirm_git_remote("Pull"),
            LeaderAction::GitsignsToggle => self.apply_cmd(CmdAction::GitsignsToggle),
            LeaderAction::Mason => self.open_mason(),
            LeaderAction::Help => self.open_help(None),
            LeaderAction::Themes => self.apply_cmd(CmdAction::Themes),
            LeaderAction::TodoList => self.apply_cmd(CmdAction::TodoList),
            LeaderAction::NoicePanel => self.apply_cmd(CmdAction::NoicePanel),
            LeaderAction::SessionSave => self.save_session(false),
            LeaderAction::SessionRestore => self.restore_session(false),
            LeaderAction::DebugStart => self.debug_start(),
            LeaderAction::DebugToggleBreakpoint => self.debug_toggle_breakpoint(),
            LeaderAction::DebugContinue => self.debug_continue(),
            LeaderAction::DebugStepOver => self.debug_step_over(),
            LeaderAction::DebugStepIn => self.debug_step_in(),
            LeaderAction::DebugStepOut => self.debug_step_out(),
            LeaderAction::DebugStop => self.debug_stop(),
            LeaderAction::Sidebar => self.toggle_sidebar(),
        }
    }

    /// Open the Settings page (shared by `:settings` and the leader binding).
    fn open_settings(&mut self) {
        // Picker options as (label, stored value) pairs. For theme/font/shell the
        // two are the same (plus a sentinel); color rows are built by SettingsState
        // from the selected theme's palette.
        let pairs = |vals: Vec<String>| -> Vec<(String, String)> {
            vals.into_iter().map(|v| (v.clone(), v)).collect()
        };
        let mut fonts = vec!["auto".to_string()];
        fonts.extend(self.available_fonts());
        // The shell picker's default sentinel (empty value) shows the user's
        // detected default shell, so it isn't a blank `< >`.
        let default_shell = ruster_terminal::default_shell().0;
        let default_name = std::path::Path::new(&default_shell)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or(default_shell);
        let mut shells: Vec<(String, String)> =
            vec![(format!("{default_name} (default)"), String::new())];
        shells.extend(pairs(self.available_shells()));
        let dynamic = vec![
            ("general", "theme", pairs(self.available_themes())),
            ("gui", "font", pairs(fonts)),
            ("terminal", "shell", shells),
        ];
        let palettes = self.all_theme_palettes();
        let syntax = self.syntax_editor_data();
        self.settings = Some(SettingsState::new(&self.config, dynamic, palettes, syntax));
    }

    /// The Syntax section's data: each highlighted language, its groups, each
    /// group's built-in default colour (hex) and the current override (or "").
    fn syntax_editor_data(&self) -> SyntaxSeed {
        let hex = |c: ruster_render::Color| match c {
            ruster_render::Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
            ruster_render::Color::Default => String::new(),
        };
        ruster_syntax::highlighted_languages()
            .iter()
            .map(|&lang| {
                let groups = ruster_syntax::groups_for_lang(lang)
                    .iter()
                    .map(|&g| {
                        let default_hex = hex(ruster_syntax::default_fg_for(lang, g));
                        let current = self
                            .config
                            .syntax_overrides
                            .get(lang)
                            .and_then(|m| m.get(g))
                            .cloned()
                            .unwrap_or_default();
                        (g.to_string(), default_hex, current)
                    })
                    .collect();
                (lang.to_string(), groups)
            })
            .collect()
    }

    /// Show the active buffer's diagnostics in a picker; Enter jumps to one.
    fn open_diagnostics_picker(&mut self) {
        let active = self.ws.borrow().active_buffer();
        let path = self.ws.borrow().active_doc().file_path.clone();
        let diags = self.lsp.diagnostics(active).to_vec();
        if diags.is_empty() {
            self.echo_warn("No diagnostics".to_string());
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

    /// Rebuild the quickfix list from all buffers' diagnostics (sorted by
    /// path/line/col). Feeds `:copen`/`:cnext`/`:cprev` until build/test runners
    /// (later tasks) populate it themselves.
    fn rebuild_quickfix_from_diagnostics(&mut self) {
        let mut items: Vec<QuickfixItem> = Vec::new();
        {
            let w = self.ws.borrow();
            for (id, diags) in self.lsp.all_diagnostics() {
                let path = match w.buffers.get(id).and_then(|d| d.file_path.clone()) {
                    Some(p) => p,
                    None => continue,
                };
                for d in diags {
                    items.push(QuickfixItem {
                        path: path.clone(),
                        line: d.start.line as usize + 1,
                        col: d.start.character as usize + 1,
                        message: d.message.replace('\n', " "),
                        severity: d.severity, // already u8
                    });
                }
            }
        }
        items.sort_by(|a, b| {
            a.path.cmp(&b.path).then(a.line.cmp(&b.line)).then(a.col.cmp(&b.col))
        });
        self.quickfix = QuickfixList::new(items);
    }

    /// `:copen` — refresh the quickfix list from diagnostics and show it as a
    /// picker; choosing an entry jumps to it.
    /// Gather every problem the editor knows about: LSP diagnostics, the
    /// quickfix list, and TODO markers.
    ///
    /// Deliberately re-read on each open rather than kept in sync — all three
    /// sources change underneath, and a stale panel is worse than a slow one.
    fn collect_trouble(&mut self) -> Vec<TroubleItem> {
        let mut out = Vec::new();
        {
            let w = self.ws.borrow();
            for (buf, diags) in self.lsp.all_diagnostics() {
                let Some(path) = w.buffers.get(buf).and_then(|d| d.file_path.clone()) else {
                    continue;
                };
                for d in diags {
                    out.push(TroubleItem {
                        path: path.clone(),
                        line: d.start.line as usize,
                        col: d.start.character as usize,
                        message: d.message.clone(),
                        severity: d.severity,
                        source: TroubleSource::Diagnostic,
                    });
                }
            }
        }
        for q in self.quickfix.items() {
            out.push(TroubleItem {
                path: q.path.clone(),
                line: q.line,
                col: q.col,
                message: q.message.clone(),
                severity: q.severity,
                source: TroubleSource::Quickfix,
            });
        }
        for (path, m) in self.todo_markers() {
            out.push(TroubleItem {
                path,
                line: m.line,
                col: m.col,
                message: if m.text.is_empty() {
                    m.keyword.clone()
                } else {
                    format!("{}: {}", m.keyword, m.text)
                },
                severity: 3,
                source: TroubleSource::Todo,
            });
        }
        out
    }

    /// `:Trouble` / `SPC x x` — open (or refresh) the pinned problem list.
    fn open_trouble(&mut self) {
        let items = self.collect_trouble();
        self.trouble.set_items(items);
        let id = self.ensure_trouble_buffer();
        self.refresh_trouble_buffer(id);
        self.ws.borrow_mut().set_active_buffer(id);
    }

    fn ensure_trouble_buffer(&mut self) -> BufferId {
        if let Some(id) = self.trouble_buf {
            if self.ws.borrow().buffers.get(id).is_some() {
                return id;
            }
        }
        let id = self
            .ws
            .borrow_mut()
            .buffers
            .create_special(ruster_core::document::SpecialKind::Trouble, "*trouble*");
        if let Some(doc) = self.ws.borrow_mut().buffers.get_mut(id) {
            doc.pinned = true;
        }
        self.trouble_buf = Some(id);
        id
    }

    fn refresh_trouble_buffer(&mut self, id: BufferId) {
        let text = self.trouble.render(self.project_root.as_deref());
        let mut w = self.ws.borrow_mut();
        if let Some(doc) = w.buffers.get_mut(id) {
            doc.buffer = Buffer::from_str(&text);
            doc.modified = false;
        }
    }

    /// Open (or refresh) the `:Mason` listing of external tools.
    fn open_mason(&mut self) {
        let tools = crate::mason::builtin_tools();
        let text = crate::mason::render(&tools, crate::mason::is_installed);
        let id = {
            let mut w = self.ws.borrow_mut();
            let existing = w
                .buffers
                .ids()
                .iter()
                .copied()
                .find(|&id| w.buffers.get(id).is_some_and(|d| d.name == "*mason*"));
            let id = existing.unwrap_or_else(|| {
                w.buffers.create_special(ruster_core::document::SpecialKind::Mason, "*mason*")
            });
            if let Some(doc) = w.buffers.get_mut(id) {
                doc.buffer = Buffer::from_str(&text);
            }
            id
        };
        self.ws.borrow_mut().set_active_buffer(id);
    }

    /// Open the manual, jumping to `topic` when one was given.
    ///
    /// An unknown topic still opens the manual — at the top, with a note. That
    /// is more useful than refusing: the reader is looking for something, and
    /// the text they need is now in front of them and searchable with `/`.
    fn open_help(&mut self, topic: Option<&str>) {
        let doc = crate::help::document();
        let line = topic.and_then(|t| crate::help::resolve(&doc, t));
        let id = {
            let mut w = self.ws.borrow_mut();
            let existing = w
                .buffers
                .ids()
                .iter()
                .copied()
                .find(|&id| w.buffers.get(id).is_some_and(|d| d.name == "*help*"));
            let id = existing.unwrap_or_else(|| {
                w.buffers.create_special(ruster_core::document::SpecialKind::Help, "*help*")
            });
            if let Some(d) = w.buffers.get_mut(id) {
                d.buffer = Buffer::from_str(&doc);
            }
            id
        };
        self.ws.borrow_mut().set_active_buffer(id);

        // Put the cursor on the topic's line and scroll it into view.
        let offset = {
            let w = self.ws.borrow();
            w.buffers.get(id).map(|d| d.buffer.line_start_char(line.unwrap_or(0)))
        };
        if let Some(off) = offset {
            let mut w = self.ws.borrow_mut();
            let win = w.windows.active_window_mut();
            win.cursors = ruster_core::cursor::CursorSet::single(off);
            win.scroll_top = line.unwrap_or(0);
        }

        if let (Some(t), None) = (topic, line) {
            self.echo_warn(format!("No help for '{t}' — showing the manual"));
        }
    }

    /// The git working tree to run git in.
    ///
    /// **Not** `project_root`: in a workspace that is the nearest `Cargo.toml`,
    /// i.e. a crate directory, and git reports paths relative to wherever it is
    /// invoked — so running from a crate makes a change elsewhere in the
    /// repository come back as `../other-crate/src/lib.rs`.
    ///
    /// Computed rather than cached: the two places `project_root` is assigned
    /// are not the only ones that matter, since tests set it directly, and a
    /// stale cache would be worse than the one `rev-parse` this costs.
    fn git_root(&self) -> Option<PathBuf> {
        ruster_git::repo_root(self.project_root.as_deref()?)
    }

    /// Open (or refresh) the `:Git` status view.
    fn open_git_status(&mut self) {
        let Some(root) = self.git_root() else {
            self.echo_warn("Not a git repository".to_string());
            return;
        };
        let Some(status) = ruster_git::status(&root) else {
            self.echo_error("Could not read git status".to_string());
            return;
        };
        self.git_status.set_status(status);
        let text = self.git_status.render(Some(&root));

        let id = {
            let mut w = self.ws.borrow_mut();
            let existing = w
                .buffers
                .ids()
                .iter()
                .copied()
                .find(|&id| w.buffers.get(id).is_some_and(|d| d.name == "*git*"));
            let id = existing.unwrap_or_else(|| {
                w.buffers.create_special(ruster_core::document::SpecialKind::Git, "*git*")
            });
            if let Some(doc) = w.buffers.get_mut(id) {
                doc.buffer = Buffer::from_str(&text);
            }
            id
        };
        self.ws.borrow_mut().set_active_buffer(id);
    }

    fn active_is_git_status(&self) -> bool {
        matches!(
            self.ws.borrow().active_doc().kind,
            DocKind::Special(ruster_core::document::SpecialKind::Git)
        )
    }

    /// The status row the cursor is on, asked of the view rather than computed
    /// by offsetting a constant — the layout puts a blank line before every
    /// section after the first, so the two drift apart.
    fn git_status_row(&self) -> Option<usize> {
        let line = {
            let w = self.ws.borrow();
            w.active_doc().buffer.char_to_line(w.primary_head())
        };
        self.git_status.row_at_line(line)
    }

    /// Keys while the status view is focused. Unclaimed keys fall through, so
    /// `:`, `/` and the motions still work.
    fn handle_git_status_key(&mut self, ck: crossterm::event::KeyEvent) -> bool {
        if !ck.modifiers.difference(crossterm::event::KeyModifiers::SHIFT).is_empty() {
            return false;
        }
        match ck.code {
            KeyCode::Enter => {
                let Some(path) = self.git_status_row().and_then(|r| self.git_status.path_at(r))
                else {
                    return true; // a heading: claimed, but nothing to open
                };
                let root = self.project_root.clone().unwrap_or_default();
                let full = if path.is_absolute() { path } else { root.join(path) };
                if full.is_file() {
                    self.open_path(&full, None);
                } else {
                    self.echo_warn(format!("{} is gone", full.display()));
                }
                true
            }
            // `z` folds here as it does in `:Trouble`, so the two sectioned
            // lists share one idiom.
            KeyCode::Tab | KeyCode::Char('z') => {
                if let Some(r) = self.git_status_row() {
                    self.git_status.toggle_at(r);
                    self.refresh_git_status_buffer();
                }
                true
            }
            KeyCode::Char('s') => {
                self.git_stage_at_cursor(true);
                true
            }
            KeyCode::Char('u') => {
                self.git_stage_at_cursor(false);
                true
            }
            KeyCode::Char('c') => {
                self.open_git_commit();
                true
            }
            KeyCode::Char('d') => {
                self.open_git_staged();
                true
            }
            KeyCode::Char('P') => {
                self.confirm_git_remote("Push");
                true
            }
            KeyCode::Char('F') => {
                self.confirm_git_remote("Pull");
                true
            }
            KeyCode::Char('r') | KeyCode::Char('g') => {
                self.open_git_status();
                true
            }
            KeyCode::Char('q') => {
                self.delete_active_buffer();
                true
            }
            _ => false,
        }
    }

    /// Open a buffer to compose a commit message. `:w` commits it.
    fn open_git_commit(&mut self) {
        let Some(root) = self.git_root() else {
            self.echo_warn("Not a git repository".to_string());
            return;
        };
        let Some(status) = ruster_git::status(&root) else {
            self.echo_warn("Not a git repository".to_string());
            return;
        };
        if status.staged().is_empty() {
            self.echo_warn("Nothing staged to commit".to_string());
            return;
        }

        // A template like git's own: the message on top, then a reminder of
        // what is about to be committed, as comments that get stripped.
        let mut text = String::from("\n");
        text.push_str("# Write a commit message above. Save with :w to commit,\n");
        text.push_str("# or close the buffer to abandon it. Lines starting with\n");
        text.push_str("# '#' are ignored, and an empty message aborts.\n#\n");
        text.push_str(&format!(
            "# On branch {}\n# Changes to be committed:\n",
            status.branch.as_deref().unwrap_or("(detached)")
        ));
        for e in status.staged() {
            let name = e.path.strip_prefix(&root).unwrap_or(&e.path).display();
            text.push_str(&format!(
                "#   {} {}\n",
                e.staged.map_or(' ', ruster_git::FileStatus::letter),
                name
            ));
        }

        let id = {
            let mut w = self.ws.borrow_mut();
            let existing = w
                .buffers
                .ids()
                .iter()
                .copied()
                .find(|&id| w.buffers.get(id).is_some_and(|d| d.name == "*git-commit*"));
            let id = existing.unwrap_or_else(|| {
                w.buffers
                    .create_special(ruster_core::document::SpecialKind::GitCommit, "*git-commit*")
            });
            if let Some(doc) = w.buffers.get_mut(id) {
                doc.buffer = Buffer::from_str(&text);
            }
            id
        };
        self.ws.borrow_mut().set_active_buffer(id);
        self.enter_insert_at_top();
    }

    /// Put the cursor on the first (empty) line ready to type the message.
    fn enter_insert_at_top(&mut self) {
        let mut w = self.ws.borrow_mut();
        w.windows.active_window_mut().cursors = ruster_core::cursor::CursorSet::single(0);
    }

    fn active_is_git_commit(&self) -> bool {
        matches!(
            self.ws.borrow().active_doc().kind,
            DocKind::Special(ruster_core::document::SpecialKind::GitCommit)
        )
    }

    /// Commit what the message buffer holds. Called instead of a file write.
    fn commit_from_buffer(&mut self) {
        let Some(root) = self.git_root() else { return };
        let raw = self.ws.borrow().active_doc().buffer.to_string();
        let message = ruster_git::clean_commit_message(&raw);
        if message.is_empty() {
            self.echo_warn("Empty commit message — nothing committed".to_string());
            return;
        }
        match ruster_git::commit(&root, &message) {
            Ok(out) => {
                // The message is committed, so the buffer is no longer unsaved
                // work — clear the flag or `delete_active_buffer` refuses it and
                // reports "buffer modified", which reads as if the commit failed.
                self.ws.borrow_mut().active_doc_mut().modified = false;
                self.delete_active_buffer();
                let summary = out.lines().next().unwrap_or("committed").to_string();
                self.echo(summary);
                // The committed lines are no longer changes, so the gutter must
                // stop marking them.
                let id = self.ws.borrow().active_buffer();
                self.refresh_git_hunks(id);
                if self.active_is_git_status() {
                    self.open_git_status();
                }
            }
            Err(e) => self.echo_error(format!("Commit failed: {e}")),
        }
    }

    /// Ask before pushing or pulling — both talk to a remote, and a push in
    /// particular is not something to trigger by a stray keypress.
    fn confirm_git_remote(&mut self, verb: &'static str) {
        if self.git_root().is_none() {
            self.echo_warn("Not a git repository".to_string());
            return;
        }
        let cmd = format!("git {}", verb.to_lowercase());
        self.confirm_command(format!("{verb}?"), verb, cmd, RunnerKind::Git);
    }

    /// Open the staged diff, where `u` unstages the hunk under the cursor.
    ///
    /// This is what makes hunk unstaging well defined. Doing it from a *file*
    /// buffer cannot work: it needs the HEAD→index diff, whose line numbers are
    /// the index's, and those stop matching the file the moment it also has
    /// unstaged edits — exactly when someone reaches for it. Here the buffer
    /// *is* that diff, so a cursor line resolves to a hunk with no translation
    /// at all.
    fn open_git_staged(&mut self) {
        let Some(root) = self.git_root() else {
            self.echo_warn("Not a git repository".to_string());
            return;
        };
        let Some(diff) = ruster_git::staged_diff(&root) else {
            self.echo_error("Could not read the staged diff".to_string());
            return;
        };
        // Unstaging the last hunk empties the diff. If the view is open it must
        // say so rather than keep showing what is no longer staged.
        let text = if diff.trim().is_empty() {
            self.echo("Nothing staged".to_string());
            if !self.active_is_git_staged() {
                return;
            }
            "Nothing staged.\n".to_string()
        } else {
            diff
        };
        let id = {
            let mut w = self.ws.borrow_mut();
            let existing = w
                .buffers
                .ids()
                .iter()
                .copied()
                .find(|&id| w.buffers.get(id).is_some_and(|d| d.name == "*git-staged*"));
            let id = existing.unwrap_or_else(|| {
                w.buffers
                    .create_special(ruster_core::document::SpecialKind::GitStaged, "*git-staged*")
            });
            if let Some(doc) = w.buffers.get_mut(id) {
                doc.buffer = Buffer::from_str(&text);
            }
            id
        };
        self.ws.borrow_mut().set_active_buffer(id);
    }

    fn active_is_git_staged(&self) -> bool {
        matches!(
            self.ws.borrow().active_doc().kind,
            DocKind::Special(ruster_core::document::SpecialKind::GitStaged)
        )
    }

    /// Keys in the staged diff. `u` unstages the hunk under the cursor.
    fn handle_git_staged_key(&mut self, ck: crossterm::event::KeyEvent) -> bool {
        if !ck.modifiers.is_empty() {
            return false;
        }
        match ck.code {
            KeyCode::Char('u') => {
                self.git_unstage_hunk_at_cursor();
                true
            }
            KeyCode::Char('r') | KeyCode::Char('g') => {
                self.open_git_staged();
                true
            }
            KeyCode::Char('q') => {
                self.delete_active_buffer();
                true
            }
            _ => false,
        }
    }

    /// Unstage the hunk the cursor is inside, in the staged diff buffer.
    ///
    /// Everything derives from the buffer text — it is the diff — so there is
    /// no cached patch list to fall out of step with what is on screen.
    fn git_unstage_hunk_at_cursor(&mut self) {
        let Some(root) = self.git_root() else { return };
        let (diff, line) = {
            let w = self.ws.borrow();
            (w.active_doc().buffer.to_string(), w.active_doc().buffer.char_to_line(w.primary_head()))
        };
        let Some(index) = ruster_git::hunk_of_line(&diff).get(line).copied().flatten() else {
            self.echo_warn("No hunk under the cursor".to_string());
            return;
        };
        let patches = ruster_git::split_hunks(&diff);
        let Some(patch) = patches.get(index) else {
            self.echo_error("Could not isolate that hunk".to_string());
            return;
        };

        match ruster_git::apply_to_index(&root, patch, true) {
            Ok(()) => {
                let total = patches.len();
                // Re-read: the diff just changed, and the buffer is the diff.
                self.open_git_staged();
                self.echo(format!("Unstaged hunk {} of {total}", index + 1));
            }
            Err(e) => self.echo_error(format!("Could not unstage that hunk: {e}")),
        }
    }

    /// Stage the hunk the cursor is inside, in the file being edited.
    ///
    /// Driven from the *file* buffer rather than the status view because that
    /// is where hunks are visible: the gutter already marks them and `]h`/`[h`
    /// already move between them. The cursor line is a working-file line, which
    /// is exactly the coordinate system of the index→worktree diff being
    /// staged.
    ///
    /// There is deliberately no cursor-driven *unstage*: that would need the
    /// HEAD→index diff, whose line numbers are the index's, and those do not
    /// match the buffer whenever the file also has unstaged edits — which is
    /// precisely when someone would reach for it. Unstaging stays at file
    /// granularity (`u` in `:Git`) until there is a view of the staged diff to
    /// point at.
    fn git_stage_hunk(&mut self) {
        let Some(root) = self.git_root() else {
            self.echo_warn("Not a git repository".to_string());
            return;
        };
        // Resolve inside the narrowest scope, then act with the borrow dropped.
        let target = {
            let w = self.ws.borrow();
            let id = w.active_buffer();
            w.buffers.get(id).and_then(|d| d.file_path.clone()).map(|p| {
                (p, w.active_doc().buffer.char_to_line(w.primary_head()) as u32)
            })
        };
        let Some((path, line)) = target else {
            self.echo_warn("No file in this window".to_string());
            return;
        };

        let Some(diff) = ruster_git::diff_text(&root, &path, false) else {
            self.echo_warn("Could not read the diff".to_string());
            return;
        };
        let hunks = ruster_git::parse_diff_hunks(&diff);
        let Some(index) = ruster_git::hunk_index_at(&hunks, line) else {
            self.echo_warn("No unstaged hunk under the cursor".to_string());
            return;
        };
        let patches = ruster_git::split_hunks(&diff);
        let Some(patch) = patches.get(index) else {
            self.echo_error("Could not isolate that hunk".to_string());
            return;
        };

        match ruster_git::apply_to_index(&root, patch, false) {
            Ok(()) => {
                let id = self.ws.borrow().active_buffer();
                self.refresh_git_hunks(id);
                self.echo(format!("Staged hunk {} of {}", index + 1, patches.len()));
            }
            Err(e) => self.echo_error(format!("Could not stage that hunk: {e}")),
        }
    }

    /// Stage or unstage the file under the cursor.
    ///
    /// Only the index is touched either way — `git add` and `git restore
    /// --staged` cannot alter the working tree — so the worst a bug here can do
    /// is stage the wrong file, never lose an edit.
    fn git_stage_at_cursor(&mut self, stage: bool) {
        let (Some(row), Some(root)) = (self.git_status_row(), self.git_root()) else {
            return;
        };
        let Some(path) = self.git_status.path_at(row) else {
            self.echo_warn("Put the cursor on a file".to_string());
            return;
        };
        let full = if path.is_absolute() { path.clone() } else { root.join(&path) };

        let result = if stage {
            ruster_git::stage(&root, &full)
        } else {
            ruster_git::unstage(&root, &full)
        };
        let name = path.display().to_string();
        match result {
            Ok(()) => {
                let line = self.cursor_line_in_active();
                self.open_git_status();
                // The list just changed shape, so hold the cursor near where it
                // was rather than flinging it to the top on every keypress.
                self.restore_git_cursor(line);
                self.echo(format!("{} {name}", if stage { "Staged" } else { "Unstaged" }));
            }
            Err(e) => self.echo_error(format!(
                "Could not {} {name}: {e}",
                if stage { "stage" } else { "unstage" }
            )),
        }
    }

    fn cursor_line_in_active(&self) -> usize {
        let w = self.ws.borrow();
        w.active_doc().buffer.char_to_line(w.primary_head())
    }

    /// Put the cursor back on `line`, clamped — the status list is usually
    /// shorter after staging.
    fn restore_git_cursor(&mut self, line: usize) {
        let mut w = self.ws.borrow_mut();
        let id = w.active_buffer();
        let Some(doc) = w.buffers.get(id) else { return };
        let last = doc.buffer.line_count().saturating_sub(1);
        let off = doc.buffer.line_start_char(line.min(last));
        w.windows.active_window_mut().cursors = ruster_core::cursor::CursorSet::single(off);
    }

    /// Re-render the status buffer from the state already parsed, without
    /// shelling out again — folding must not cost a `git status`.
    fn refresh_git_status_buffer(&mut self) {
        let root = self.project_root.clone();
        let text = self.git_status.render(root.as_deref());
        let mut w = self.ws.borrow_mut();
        let id = w.active_buffer();
        if let Some(doc) = w.buffers.get_mut(id) {
            doc.buffer = Buffer::from_str(&text);
        }
    }

    fn active_is_help(&self) -> bool {
        matches!(
            self.ws.borrow().active_doc().kind,
            DocKind::Special(ruster_core::document::SpecialKind::Help)
        )
    }

    /// `q` closes the manual. Everything else falls through, so `/`, `n` and the
    /// motions all work while reading.
    fn handle_help_key(&mut self, ck: crossterm::event::KeyEvent) -> bool {
        if ck.code == KeyCode::Char('q') && ck.modifiers.is_empty() {
            self.delete_active_buffer();
            return true;
        }
        false
    }

    fn active_is_mason(&self) -> bool {
        matches!(
            self.ws.borrow().active_doc().kind,
            DocKind::Special(ruster_core::document::SpecialKind::Mason)
        )
    }

    /// Keys while the Mason list is focused. Unclaimed keys fall through, so
    /// `:`, `/` and the leader still work here.
    fn handle_mason_key(&mut self, ck: crossterm::event::KeyEvent) -> bool {
        match ck.code {
            KeyCode::Enter => {
                self.confirm_install();
                true
            }
            KeyCode::Char('r') => {
                self.open_mason();
                true
            }
            KeyCode::Char('q') => {
                self.delete_active_buffer();
                true
            }
            _ => false,
        }
    }

    /// Ask before installing anything.
    ///
    /// The dialog shows the exact command that will run, verbatim. ruster never
    /// installs unprompted and never runs anything the user has not read — the
    /// registry is a convenience, not a licence to execute.
    fn confirm_install(&mut self) {
        let (line, text) = {
            let w = self.ws.borrow();
            let doc = w.active_doc();
            (doc.buffer.char_to_line(w.primary_head()), doc.buffer.to_string())
        };
        let tools = crate::mason::builtin_tools();
        let Some(tool) = crate::mason::tool_at_row(&tools, &text, line) else {
            self.echo("Not a tool — put the cursor on a listed tool".to_string());
            return;
        };
        if crate::mason::is_installed(&tool.binary) {
            self.echo(format!("{} is already installed", tool.name));
            return;
        }
        self.confirm_command(
            format!("Install {}?", tool.name),
            "Install",
            tool.install.clone(),
            RunnerKind::Install,
        );
    }

    /// Ask before running `cmd`, showing it verbatim.
    ///
    /// `verb` names the confirming button and the message if it is declined, so
    /// the dialog reads as the thing it will do rather than a generic "OK".
    fn confirm_command(
        &mut self,
        title: String,
        verb: &str,
        cmd: String,
        kind: RunnerKind,
    ) {
        self.dialog = Some(crate::dialog::DialogState::new(
            title,
            vec![
                crate::dialog::Field::text("Runs", &cmd),
                crate::dialog::Field::button(verb),
                crate::dialog::Field::button("Cancel"),
            ],
        ));
        self.pending_confirm =
            Some(PendingConfirm { cmd, kind, verb: verb.to_string() });
    }

    /// Run a confirmed command, streamed through the runner like a build.
    fn run_pending_confirm(&mut self, button: Option<&str>) {
        let Some(p) = self.pending_confirm.take() else { return };
        if button != Some(p.verb.as_str()) {
            self.echo(format!("{} cancelled", p.verb));
            return;
        }
        // A git command runs from the working tree, not from whichever crate
        // directory happens to be the project root.
        let root = match p.kind {
            RunnerKind::Git => self.git_root(),
            _ => self.project_root.clone(),
        }
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        self.start_run(p.kind, p.cmd, root);
    }

    /// Report a push/pull's outcome and refresh the status view if it is open,
    /// so the ahead/behind counts stop lying immediately.
    fn finish_git_command(&mut self, code: Option<i32>) {
        use ruster_core::message::{MessageLevel, MessageSource};
        let (level, text) = match code {
            Some(0) => (MessageLevel::Success, "git finished".to_string()),
            Some(c) => (MessageLevel::Error, format!("git failed (exit {c})")),
            None => (MessageLevel::Error, "git failed to run".to_string()),
        };
        self.push_message(level, MessageSource::System, text);
        let open = self
            .ws
            .borrow()
            .buffers
            .ids()
            .iter()
            .any(|&id| self.ws.borrow().buffers.get(id).is_some_and(|d| d.name == "*git*"));
        if open {
            self.open_git_status();
        }
    }

    /// Report an install's outcome and refresh the list, so a tool that just
    /// arrived flips to ✓ without the user re-running `:Mason`.
    fn finish_install(&mut self, code: Option<i32>) {
        use ruster_core::message::{MessageLevel, MessageSource};
        let (level, text) = match code {
            Some(0) => (MessageLevel::Success, "Install finished".to_string()),
            Some(c) => (MessageLevel::Error, format!("Install failed (exit {c})")),
            None => (MessageLevel::Error, "Install failed to run".to_string()),
        };
        self.push_message(level, MessageSource::System, text);
        // Only refresh the listing if it is still open.
        let open = self.ws.borrow().buffers.ids().iter().any(|&id| {
            self.ws.borrow().buffers.get(id).is_some_and(|d| d.name == "*mason*")
        });
        if open {
            self.open_mason();
        }
    }

    /// Capture the current session: which real files are open, and the layout.
    ///
    /// Only file-backed buffers are saved. Special buffers (dired, mason, diff,
    /// terminals, `*messages*`) have nothing durable to point at, and an unsaved
    /// scratch buffer has nowhere its contents could come back from.
    fn capture_session(&self) -> Option<ruster_core::session::Session> {
        let w = self.ws.borrow();
        let mut files: Vec<PathBuf> = Vec::new();
        let mut index: std::collections::HashMap<BufferId, usize> =
            std::collections::HashMap::new();
        for &id in w.buffers.ids().iter() {
            let Some(doc) = w.buffers.get(id) else { continue };
            if matches!(doc.kind, DocKind::Special(_)) {
                continue;
            }
            let Some(path) = doc.file_path.clone() else { continue };
            // Absolute, or the session only restores from the directory the
            // editor happened to be started in. Canonicalising also collapses
            // `..` so the same file saved two ways is one entry.
            let path = std::fs::canonicalize(&path).unwrap_or_else(|_| {
                std::env::current_dir().map(|d| d.join(&path)).unwrap_or(path)
            });
            index.insert(id, files.len());
            files.push(path);
        }
        if files.is_empty() {
            return None;
        }
        // Every visible window showed a special buffer (Mason, dired, a
        // terminal), so the layout has nothing to say — but files *are* open.
        // Fall back to a single window on the first of them rather than
        // discarding the session and losing them.
        let layout = w.windows.snapshot(|b| index.get(&b).copied()).unwrap_or(
            ruster_core::windows::LayoutSnapshot::Leaf {
                buffer: 0,
                cursor: 0,
                scroll: 0,
                active: true,
            },
        );
        Some(ruster_core::session::Session { files, layout })
    }

    /// Write the session for the current project.
    fn save_session(&mut self, quiet: bool) {
        let (Some(root), Some(dir)) = (self.project_root.clone(), ruster_config_dir()) else {
            if !quiet {
                self.echo_warn("No project — nothing to save a session for".to_string());
            }
            return;
        };
        let Some(session) = self.capture_session() else {
            if !quiet {
                self.echo_warn("No files open to save".to_string());
            }
            return;
        };
        match ruster_core::session::save(&dir, &root, &session) {
            Ok(()) if !quiet => self.echo(format!("Session saved ({} files)", session.files.len())),
            Ok(()) => {}
            Err(e) => self.echo_error(format!("Could not save session: {e}")),
        }
    }

    /// Reopen the saved session for the current project.
    ///
    /// Files that no longer exist are skipped and their windows collapse out of
    /// the layout, so a session written before a refactor still restores what
    /// survives instead of failing whole.
    fn restore_session(&mut self, quiet: bool) {
        let (Some(root), Some(dir)) = (self.project_root.clone(), ruster_config_dir()) else {
            if !quiet {
                self.echo_warn("No project — no session to restore".to_string());
            }
            return;
        };
        let Some(session) = ruster_core::session::load(&dir, &root) else {
            if !quiet {
                self.echo_warn("No saved session for this project".to_string());
            }
            return;
        };

        // Open every file that still exists, remembering which index it took.
        let mut opened: Vec<Option<BufferId>> = Vec::with_capacity(session.files.len());
        let mut missing = 0usize;
        for path in &session.files {
            if !path.is_file() {
                opened.push(None);
                missing += 1;
                continue;
            }
            let content = std::fs::read_to_string(path).unwrap_or_default();
            let id = self.ws.borrow_mut().buffers.open_file(path.clone(), content);
            opened.push(Some(id));
        }

        let Some(tree) =
            ruster_core::windows::WindowTree::restore(&session.layout, |i| {
                opened.get(i).copied().flatten()
            })
        else {
            self.echo_warn("Session restored no files (all missing)".to_string());
            return;
        };
        self.ws.borrow_mut().windows = tree;
        self.update_syntax();

        let n = opened.iter().filter(|o| o.is_some()).count();
        let note = if missing > 0 { format!(", {missing} missing") } else { String::new() };
        self.echo(format!("Session restored ({n} files{note})"));
    }

    fn active_is_trouble(&self) -> bool {
        matches!(
            self.ws.borrow().active_doc().kind,
            DocKind::Special(ruster_core::document::SpecialKind::Trouble)
        )
    }

    /// Keys while the problem list is focused. Unclaimed keys fall through, so
    /// `:`, `/` and motions keep working over the listing.
    fn handle_trouble_key(&mut self, ck: crossterm::event::KeyEvent) -> bool {
        let row = {
            let w = self.ws.borrow();
            w.buffer().char_to_line(w.primary_head())
        };
        match ck.code {
            KeyCode::Enter => {
                if let Some((path, line, col)) = self.trouble.target_at(row) {
                    self.open_path(&path, Some((line, col)));
                }
                true
            }
            KeyCode::Tab | KeyCode::Char('z') => {
                self.trouble.toggle_at(row);
                if let Some(id) = self.trouble_buf {
                    self.refresh_trouble_buffer(id);
                }
                true
            }
            KeyCode::Char('r') => {
                self.open_trouble();
                true
            }
            KeyCode::Char('q') => {
                self.delete_active_buffer();
                true
            }
            _ => false,
        }
    }

    /// `:TodoList` — collect TODO-class markers into the quickfix list and open
    /// it. Routing through quickfix means `]q`/`[q` and the Trouble panel get
    /// them for free rather than each growing its own list.
    fn open_todo_list(&mut self) {
        let markers = self.todo_markers();
        if markers.is_empty() {
            self.echo_warn("No TODO markers in open buffers".to_string());
            return;
        }
        let items: Vec<QuickfixItem> = markers
            .into_iter()
            .map(|(path, m)| QuickfixItem {
                path,
                line: m.line,
                col: m.col,
                message: if m.text.is_empty() {
                    m.keyword.clone()
                } else {
                    format!("{}: {}", m.keyword, m.text)
                },
                // Info: a marker is a note to self, not a compiler complaint.
                severity: 3,
            })
            .collect();
        self.quickfix = QuickfixList::new(items);
        self.open_quickfix_picker("TODO");
    }

    fn open_quickfix(&mut self) {
        self.rebuild_quickfix_from_diagnostics();
        self.open_quickfix_picker("Quickfix");
    }

    /// Show whatever is already in the quickfix list. Split out so callers that
    /// populate it themselves — `:TodoList` — aren't overwritten by the
    /// diagnostics rebuild.
    fn open_quickfix_picker(&mut self, title: &str) {
        if self.quickfix.is_empty() {
            self.echo_warn("Quickfix list is empty".to_string());
            return;
        }
        let items: Vec<PickerItem> = self
            .quickfix
            .items()
            .iter()
            .map(|q| {
                let sev = match q.severity {
                    1 => "E",
                    2 => "W",
                    3 => "I",
                    _ => "H",
                };
                let name = q
                    .path
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| q.path.display().to_string());
                PickerItem::new(
                    format!("{} {}:{}:{}  {}", sev, name, q.line, q.col, q.message),
                    PickerAction::OpenLocation(q.path.clone(), q.line, q.col),
                )
            })
            .collect();
        self.picker = Some(PickerState::new(title, items));
    }

    /// Jump to the current quickfix entry and report the position in the list.
    fn quickfix_jump_current(&mut self) {
        let (path, line, col, msg, pos, total) = match self.quickfix.current() {
            Some(q) => (
                q.path.clone(),
                q.line,
                q.col,
                q.message.clone(),
                self.quickfix.selected() + 1,
                self.quickfix.len(),
            ),
            None => {
                self.echo_warn("Quickfix list is empty".to_string());
                return;
            }
        };
        self.open_path(&path, Some((line, col)));
        self.echo(format!("({pos}/{total}) {msg}"));
    }

    /// `:cnext` / `]q` — advance the quickfix selection and jump.
    fn quickfix_next(&mut self) {
        if self.quickfix.is_empty() {
            self.rebuild_quickfix_from_diagnostics();
        }
        self.quickfix.next();
        self.quickfix_jump_current();
    }

    /// `:cprev` / `[q` — step back through the quickfix list and jump.
    fn quickfix_prev(&mut self) {
        if self.quickfix.is_empty() {
            self.rebuild_quickfix_from_diagnostics();
        }
        self.quickfix.prev();
        self.quickfix_jump_current();
    }

    /// Ensure the `*build*` results buffer exists, returning its id.
    fn ensure_runner_buffer(&mut self, name: &str) -> BufferId {
        if let Some(id) = self.runner_buf {
            if self.ws.borrow().buffers.get(id).is_some() {
                return id;
            }
        }
        let id = self.ws.borrow_mut().buffers.create_special(SpecialKind::Build, name);
        self.runner_buf = Some(id);
        id
    }

    /// `:build` / `SPC c b` — run the project's build command.
    fn run_build(&mut self) {
        let root = self.project_root_for_run();
        let cmd = ruster_project::ProjectConfig::load(&root)
            .build_command_with(&root, self.config.build_command.as_deref());
        self.start_run(RunnerKind::Build, cmd, root);
    }

    /// `:test` / `SPC c t` — run the project's test command.
    fn run_test(&mut self) {
        let root = self.project_root_for_run();
        let cmd = ruster_project::ProjectConfig::load(&root)
            .test_command_with(&root, self.config.test_command.as_deref());
        self.start_run(RunnerKind::Test, cmd, root);
    }

    /// `:task` / `SPC o r` — pick a `ruster.toml` task to run.
    fn open_task_picker(&mut self) {
        let root = self.project_root_for_run();
        let cfg = ruster_project::ProjectConfig::load(&root);
        if cfg.tasks.is_empty() {
            self.echo_warn("No tasks — add [tasks.<name>] to ruster.toml".to_string());
            return;
        }
        let items: Vec<PickerItem> = cfg
            .tasks
            .iter()
            .map(|(name, task)| {
                PickerItem::new(format!("{name:<16} {}", task.cmd), PickerAction::RunTask(name.clone()))
            })
            .collect();
        self.picker = Some(PickerState::new("Tasks", items));
    }

    /// Returns a status message when a build/test/task runner is active, or None.
    pub fn runner_status_text(&self) -> Option<&'static str> {
        if self.runner_rx.is_some() {
            Some(match self.runner_kind {
                RunnerKind::Install => "Installing...",
                RunnerKind::Git => "Running git...",
                RunnerKind::Build => "Building...",
                RunnerKind::Test => "Testing...",
                RunnerKind::Task => "Running Task...",
            })
        } else {
            None
        }
    }

    /// Run the named `ruster.toml` task — in the embedded terminal (default) or a
    /// background thread when `use_terminal = false`.
    fn run_task(&mut self, name: &str) {
        let root = self.project_root_for_run();
        let cfg = ruster_project::ProjectConfig::load(&root);
        let Some(task) = cfg.tasks.get(name) else {
            self.echo_warn(format!("No such task: {name}"));
            return;
        };
        let cwd = match &task.cwd {
            Some(c) => root.join(c),
            None => root.clone(),
        };
        if task.use_terminal {
            self.open_terminal_running(&task.cmd, &cwd, name);
        } else {
            self.start_run(RunnerKind::Task, task.cmd.clone(), cwd);
        }
    }

    /// Open an embedded terminal that runs `cmd` in `cwd`, then drops to a shell
    /// so its output stays visible.
    fn open_terminal_running(&mut self, cmd: &str, cwd: &std::path::Path, name: &str) {
        let cwd = cwd.to_string_lossy();
        let (shell, args) = if cfg!(windows) {
            ("cmd".to_string(), vec!["/K".to_string(), format!("cd /d {cwd} && {cmd}")])
        } else {
            ("sh".to_string(), vec!["-c".to_string(), format!("cd {cwd} && {cmd}; exec ${{SHELL:-sh}}")])
        };
        let scrollback = self.config.terminal_scrollback as usize;
        match TerminalSession::spawn(&shell, &args, 80, 24, scrollback) {
            Ok(session) => {
                let id = self
                    .ws
                    .borrow_mut()
                    .buffers
                    .create_special(SpecialKind::Terminal, &format!("*task:{name}*"));
                self.ws.borrow_mut().set_active_buffer(id);
                self.terminals.insert(id, session);
                self.terminal_focused = self.config.terminal_default_mode != "normal";
                self.notify.push(Notification::new(ruster_core::message::MessageLevel::Info, ruster_core::message::MessageSource::System, format!("task {name}: Ctrl-\\ to leave, i to re-enter")));
            }
            Err(e) => { self.notify.push(Notification::new(ruster_core::message::MessageLevel::Info, ruster_core::message::MessageSource::Task, format!("task {name}: {e}"))); },
        }
    }

    /// The project root for a run: the active file's root, else the cwd.
    fn project_root_for_run(&self) -> PathBuf {
        self.ws
            .borrow()
            .active_doc()
            .file_path
            .clone()
            .and_then(|p| ruster_project::project_root(&p))
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// Spawn a background run, streaming into a results buffer; the per-frame
    /// drain parses its output on completion.
    fn start_run(&mut self, kind: RunnerKind, cmd: String, root: PathBuf) {
        let (buf_name, label) = match kind {
            RunnerKind::Install => ("*install*", "install"),
            RunnerKind::Git => ("*git-output*", "git command"),
            RunnerKind::Build => ("*build*", "build"),
            RunnerKind::Test => ("*test*", "test"),
            RunnerKind::Task => ("*task*", "task"),
        };
        if self.runner_rx.is_some() {
            self.echo(format!("A {label} is already running"));
            return;
        }
        if cmd.is_empty() {
            self.echo(format!("No {label} command for this project (set it in ruster.toml)"));
            return;
        }
        self.runner_kind = kind;
        self.runner_output = format!("$ {cmd}\n");
        self.runner_root = root.clone();
        let id = self.ensure_runner_buffer(buf_name);
        {
            let mut w = self.ws.borrow_mut();
            if let Some(doc) = w.buffers.get_mut(id) {
                doc.buffer = Buffer::from_str(&self.runner_output);
            }
            w.set_active_buffer(id);
        }
        self.runner_rx = Some(crate::runner::spawn_shell_command(&cmd, &root));
        self.echo(format!("{label}: {cmd}"));
    }

    /// Poll the active debug session (if any) for DAP events, updating state.
    fn drain_debug_events(&mut self) {
        let Some(session) = self.debug.session_mut() else { return };
        for ev in session.poll_events() {
            match ev {
                ruster_dap::session::DapEvent::Stopped { reason: _, thread_id } => {
                    session.get_stack(thread_id).ok();
                    session.state = ruster_dap::session::SessionState::Paused;
                }
                ruster_dap::session::DapEvent::Terminated => {
                    self.debug.stop();
                    return;
                }
                _ => {}
            }
        }
        if session.stopped() && session.scopes.is_empty() && !session.stack_frames.is_empty() {
            if let Some(frame) = session.stack_frames.first() {
                session.get_scopes(frame.id as u64).ok();
            }
        }
        let scopes = session.scopes.clone();
        if !scopes.is_empty() && session.variables.is_empty() {
            for scope in &scopes {
                if scope.variables_reference > 0 {
                    session.get_variables(scope.variables_reference as u64).ok();
                }
            }
        }
        let vars: Vec<(String, Vec<(String, String)>)> = {
            let scopes = &session.scopes;
            let cache = &session.variable_cache;
            scopes.iter().filter(|s| s.variables_reference > 0).map(|scope| {
                let pairs = cache
                    .iter()
                    .filter(|(&k, _)| k == scope.variables_reference as u64)
                    .map(|(_, v)| (v.name.clone(), v.value.clone()))
                    .collect();
                (scope.name.clone(), pairs)
            }).collect()
        };
        session.variables = vars;
    }

    /// Drain the running command's output into its results buffer; on completion
    /// parse it (build → diagnostics; test → results + ✓/✗ signs) into the
    /// quickfix list. Called once per frame.
    fn drain_build_runner(&mut self) {
        use crate::runner::RunnerMsg;
        use ruster_core::message::{MessageLevel, MessageSource};
        use std::sync::mpsc::TryRecvError;
        let Some(rx) = self.runner_rx.take() else { return };
        let msg_source = match self.runner_kind {
            RunnerKind::Build => MessageSource::Build,
            RunnerKind::Test => MessageSource::Test,
            RunnerKind::Task => MessageSource::Task,
            // An install is a system action, not part of the project's build.
            RunnerKind::Install => MessageSource::System,
            RunnerKind::Git => MessageSource::System,
        };
        let mut appended = false;
        let mut done: Option<Option<i32>> = None;
        loop {
            match rx.try_recv() {
                Ok(RunnerMsg::Line(l)) => {
                    self.runner_output.push_str(&l);
                    self.runner_output.push('\n');
                    appended = true;
                    self.push_message(MessageLevel::Info, msg_source, l.clone());
                }
                Ok(RunnerMsg::Done(code)) => {
                    done = Some(code);
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    done = Some(None);
                    break;
                }
            }
        }
        if appended {
            if let Some(id) = self.runner_buf {
                let mut w = self.ws.borrow_mut();
                if let Some(doc) = w.buffers.get_mut(id) {
                    doc.buffer = Buffer::from_str(&self.runner_output);
                }
            }
        }
        if let Some(code) = done {
            match self.runner_kind {
                RunnerKind::Build => self.finish_build(code),
                RunnerKind::Test => self.finish_test(code),
                RunnerKind::Install => self.finish_install(code),
                RunnerKind::Git => self.finish_git_command(code),
                RunnerKind::Task => {
                    let status_text = match code {
                        Some(0) => "succeeded".to_string(),
                        Some(c) => format!("exit {c}"),
                        None => "failed to run".to_string(),
                    };
                    self.push_message(
                        if code == Some(0) { ruster_core::message::MessageLevel::Success } else { ruster_core::message::MessageLevel::Error },
                        ruster_core::message::MessageSource::Task,
                        status_text.clone(),
                    );
                    self.notify.push(Notification::new(ruster_core::message::MessageLevel::Info, ruster_core::message::MessageSource::Task, format!("task {status_text}")));
                }
            }
        }
    }

    fn finish_build(&mut self, code: Option<i32>) {
        let items = crate::runner::parse_build_diagnostics(&self.runner_output, &self.runner_root);
        let n = items.len();
        self.quickfix = QuickfixList::new(items);
        if !self.quickfix.is_empty() {
            self.open_quickfix();
        }
        let status = match code {
            Some(0) => "ok".to_string(),
            Some(c) => format!("exit {c}"),
            None => "failed to run".to_string(),
        };
        let hint = if n > 0 { "  (:copen)" } else { "" };
        let msg = format!("build {status} — {n} problem(s){hint}");
        self.push_message(
            if code == Some(0) { ruster_core::message::MessageLevel::Success } else { ruster_core::message::MessageLevel::Error },
            ruster_core::message::MessageSource::Build,
            msg.clone(),
        );
        self.echo(msg);
    }

    fn finish_test(&mut self, code: Option<i32>) {
        let run = crate::runner::parse_test_results(&self.runner_output, &self.runner_root);
        // Failures feed the quickfix list and place ✗ signs; passes tally only.
        let mut items: Vec<QuickfixItem> = Vec::new();
        self.result_signs.clear();
        for t in &run.results {
            if t.outcome != crate::runner::TestOutcome::Fail {
                continue;
            }
            let Some((path, line, col)) = t.location.clone() else { continue };
            items.push(QuickfixItem {
                path: path.clone(),
                line,
                col,
                message: format!("test failed: {}", t.name),
                severity: 1,
            });
            // Key signs by the canonical path so they match open buffers.
            let key = path.canonicalize().unwrap_or(path);
            let entry = self.result_signs.entry(key).or_default();
            entry.width = 1;
            entry.signs.push((
                line.saturating_sub(1) as u16,
                '✗',
                ruster_syntax::sign_style("error").fg,
            ));
        }
        self.quickfix = QuickfixList::new(items);
        let status = if run.failed == 0 && code == Some(0) { "ok" } else { "FAILED" };
        let hint = if run.failed > 0 { "  (:copen)" } else { "" };
        let msg = format!("test {status} — {} passed, {} failed{hint}", run.passed, run.failed);
        self.push_message(
            if run.failed == 0 && code == Some(0) { ruster_core::message::MessageLevel::Success } else { ruster_core::message::MessageLevel::Error },
            ruster_core::message::MessageSource::Test,
            msg.clone(),
        );
        self.echo(msg);
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

    /// Re-diff the active buffer. Called after a write, when the file on disk
    /// no longer matches what git last saw.
    fn refresh_git_hunks_active(&mut self) {
        let id = self.ws.borrow().active_buffer();
        self.refresh_git_hunks(id);
    }

    fn save_file(&mut self, force: bool) {
        // Format-on-save: format via LSP first, then write when the edits arrive.
        if self.config.format_on_save && !self.pending_format_save {
            let active = self.ws.borrow().active_buffer();
            if self.lsp.is_tracked(active) {
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
                self.echo_error("E32: No file name".to_string());
                return;
            }
        };
        self.lua.fire_event_str("BufWritePre", &[path.to_str().unwrap_or("")]);
        match std::fs::write(&path, &content) {
            Ok(()) => {
                self.ws.borrow_mut().active_doc_mut().modified = false;
                self.echo_success(format!("Saved: {}", path.display()));
            }
            Err(_e) if force => {
                let _ = std::fs::write(&path, &content);
                self.ws.borrow_mut().active_doc_mut().modified = false;
                self.echo_success(format!("Saved (forced): {}", path.display()));
            }
            Err(e) => { self.echo_error(format!("Error: {}", e)); },
        }
        // The file on disk changed, so the diff against the index has too.
        self.refresh_git_hunks_active();
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
                self.echo_success(format!("Saved: {}", path));
            }
            Err(e) => { self.echo_error(format!("Error: {}", e)); },
        }
    }

    fn build_debug_overlay(&self) -> Option<ruster_render::DebugOverlayView> {
        let session = self.debug.session()?;
        let status = if session.stopped() { "PAUSED" } else { "RUNNING" };
        let toolbar = format!("[Debug: {}] F5:Continue F10:Next F11:StepIn S-F5:Stop", status);
        let stack: Vec<(u16, String, String)> = session
            .stack_frames
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let loc = f.source.as_ref().and_then(|s| s.path.as_deref()).unwrap_or("?");
                (i as u16, f.name.clone(), format!("{}:{}", loc, f.line))
            })
            .collect();
        let scopes: Vec<(String, Vec<(String, String)>)> = session
            .variables
            .iter()
            .map(|(scope_name, vars)| {
                let vars: Vec<(String, String)> = vars.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                (scope_name.clone(), vars)
            })
            .collect();
        Some(ruster_render::DebugOverlayView { toolbar, stack, scopes })
    }

    fn debug_start(&mut self) {
        if self.debug.is_running() {
            self.notify.push(Notification::new(ruster_core::message::MessageLevel::Info, ruster_core::message::MessageSource::System, "Debug session already active"));
            return;
        }
        let root = self.project_root.as_deref().unwrap_or(std::path::Path::new("."));
        // Pick the adapter from the file being edited, not a fixed language.
        let lang = {
            let w = self.ws.borrow();
            match w.active_doc().file_path.as_deref() {
                Some(p) => {
                    // detect_config matches a language name or a bare extension.
                    // Prefer the canonical name where syntax knows one (`rs` →
                    // `rust`), and fall back to the extension where it doesn't
                    // (`go` has no syntax key but is a valid adapter language).
                    let ext = ruster_syntax::lang_ext_for_path(p);
                    let key = ruster_syntax::lang_key(&ext);
                    if key.is_empty() { ext } else { key.to_string() }
                }
                None => String::new(),
            }
        };
        let cfg = ruster_dap::config::detect_config(&lang, root, None);
        let cfg = match cfg {
            Some(c) => c,
            None => {
                self.notify.push(Notification::new(
                    ruster_core::message::MessageLevel::Warning,
                    ruster_core::message::MessageSource::System,
                    format!("No debug adapter for '{lang}' — set dap.adapter to override"),
                ));
                return;
            }
        };
        // A configured adapter overrides the detected program.
        let cfg = match self.config.dap_adapter.as_deref() {
            Some(prog) if !prog.is_empty() => {
                ruster_dap::config::AdapterConfig { command: prog.to_string(), ..cfg }
            }
            _ => cfg,
        };
        match ruster_dap::session::DebugSession::start(&cfg, root) {
            Ok(mut session) => {
                session.send_initialize().ok();
                session.send_launch(serde_json::json!({})).ok();
                // `start` pushes the breakpoint table. Before this, the
                // session was merely stored and `toggle_breakpoint` only
                // pushed when one was already running — so breakpoints placed
                // before `:DebugStart`, which is the normal order, were never
                // sent and the debugger ran straight past them.
                self.debug.start(session);
                self.notify.push(Notification::new(ruster_core::message::MessageLevel::Info, ruster_core::message::MessageSource::System, format!("Debug started: {}", cfg.name)));
            }
            Err(e) => { self.notify.push(Notification::new(ruster_core::message::MessageLevel::Info, ruster_core::message::MessageSource::System, format!("Debug start failed: {}", e))); },
        }
    }

    fn debug_stop(&mut self) {
        if let Some(mut session) = self.debug.stop() {
            session.disconnect().ok();
        }
    }

    fn debug_continue(&mut self) {
        if let Some(session) = self.debug.session_mut() {
            session.continue_exec().ok();
        }
    }

    fn debug_step_over(&mut self) {
        if let Some(session) = self.debug.session_mut() {
            session.step_over().ok();
        }
    }

    fn debug_step_in(&mut self) {
        if let Some(session) = self.debug.session_mut() {
            session.step_into().ok();
        }
    }

    fn debug_step_out(&mut self) {
        if let Some(session) = self.debug.session_mut() {
            session.step_out().ok();
        }
    }

    fn debug_toggle_breakpoint(&mut self) {
        // Resolve inside the borrow, report outside it — `echo_warn` takes
        // `&mut self`, which the live `Ref` on `self.ws` would forbid.
        let resolved = {
            let w = self.ws.borrow();
            w.active_doc().file_path.as_ref().map(|p| {
                let path = p.canonicalize().unwrap_or_else(|_| p.clone());
                let line = w.buffer().char_to_line(w.primary_head()) as u16;
                (path, line)
            })
        };
        let (path, line) = match resolved {
            Some(v) => v,
            None => {
                self.echo_warn("No file path for breakpoint");
                return;
            }
        };
        // The push to a running session lives in `DebugState::toggle_breakpoint`
        // now, so there is no longer a way to edit the table without the
        // debugger hearing about it.
        self.debug.toggle_breakpoint(&path, line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_prompt::FilePromptKind;

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
        assert!(a.notify.history().iter().any(|n| n.text.contains("not found")));
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
    fn parse_set_option_variants() {
        use ruster_lua::schema::SettingValue as SV;
        let a = App::new("x".into(), PathBuf::from("f.txt"));
        assert_eq!(
            a.parse_cmdline(":set number"),
            Ok(CmdAction::SetNamed("number".into(), SetNamedVal::Exact(SV::Bool(true))))
        );
        assert_eq!(
            a.parse_cmdline(":set nonumber"),
            Ok(CmdAction::SetNamed("number".into(), SetNamedVal::Exact(SV::Bool(false))))
        );
        assert_eq!(
            a.parse_cmdline(":set number!"),
            Ok(CmdAction::SetNamed("number".into(), SetNamedVal::Toggle))
        );
        assert_eq!(
            a.parse_cmdline(":set relativenumber"),
            Ok(CmdAction::SetNamed("relativenumber".into(), SetNamedVal::Exact(SV::Bool(true))))
        );
        assert_eq!(
            a.parse_cmdline(":set expandtab"),
            Ok(CmdAction::SetNamed("expandtab".into(), SetNamedVal::Exact(SV::Bool(true))))
        );
        assert_eq!(
            a.parse_cmdline(":set tabstop=8"),
            Ok(CmdAction::SetNamed("tabstop".into(), SetNamedVal::Exact(SV::Int(8))))
        );
        assert_eq!(
            a.parse_cmdline(":set editmode=emacs"),
            Ok(CmdAction::SetNamed("editmode".into(), SetNamedVal::Exact(SV::Enum("emacs".into()))))
        );
        // Bare non-bool key shows current value (like Vim).
        assert_eq!(
            a.parse_cmdline(":set tabstop"),
            Ok(CmdAction::ShowSetting("tabstop".into()))
        );
        // ? suffix shows value for any type.
        assert_eq!(
            a.parse_cmdline(":set tabstop?"),
            Ok(CmdAction::ShowSetting("tabstop".into()))
        );
        assert_eq!(
            a.parse_cmdline(":set number?"),
            Ok(CmdAction::ShowSetting("number".into()))
        );
        // & suffix resets to default.
        assert_eq!(
            a.parse_cmdline(":set tabstop&"),
            Ok(CmdAction::ResetSetting("tabstop".into()))
        );
        assert_eq!(
            a.parse_cmdline(":set number&"),
            Ok(CmdAction::ResetSetting("number".into()))
        );
        assert!(a.parse_cmdline(":set bogus").is_err());
        assert!(a.parse_cmdline(":set tabstop=bogus").is_err());
        assert!(a.parse_cmdline(":set no").is_err());  // "no" prefix with empty key
        assert!(a.parse_cmdline(":set ?").is_err());   // ? with empty key
        assert!(a.parse_cmdline(":set &").is_err());   // & with empty key
    }

    #[test]
    fn set_named_toggles_config_live() {
        use ruster_lua::schema::SettingValue as SV;
        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        assert!(!a.config.number);
        a.apply_cmd(CmdAction::SetNamed("number".into(), SetNamedVal::Exact(SV::Bool(true))));
        assert!(a.config.number);
        a.apply_cmd(CmdAction::SetNamed("number".into(), SetNamedVal::Toggle));
        assert!(!a.config.number);
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
            // file + scratch + dashboard + messages
            assert_eq!(p.view().rows.len(), 4);
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
        // scratch + dashboard + messages
        assert_eq!(a.ws.borrow().buffers.len(), 3);
        assert!(a.ws.borrow().buffers.get(orig).is_none());
    }

    /// Closing a buffer must drop its per-buffer caches. None of these maps was
    /// cleaned up before, so they grew for the life of the session.
    /// Git status is the weakest signal in the gutter: when a line is both
    /// changed and carries a diagnostic, the diagnostic must win. Signs are
    /// merged later-wins, so this pins the ordering.
    /// Setting a single breakpoint used to crash the editor on the next frame.
    ///
    /// The sign-column code re-borrowed the workspace inside a scope that
    /// already held a mutable borrow, panicking with `RefCell already mutably
    /// borrowed`. It survived because the branch is behind
    /// `any_breakpoints()` — false until someone actually sets one, which no
    /// test did and which is the first thing anyone using the debugger does.
    #[test]
    fn setting_a_breakpoint_does_not_panic_on_the_next_render() {
        let mut a = App::new("one\ntwo\nthree\n".into(), PathBuf::from("f.rs"));
        a.debug.toggle_breakpoint(std::path::Path::new("f.rs"), 1);
        assert!(a.debug.any_breakpoints(), "precondition: the branch is now live");
        a.render();
    }

    /// The same re-borrow, in the branch next to it. Guarded by
    /// `!result_signs.is_empty()`, so it needs a test run to fire rather than a
    /// breakpoint — equally reachable, equally fatal, and fixed together.
    #[test]
    fn a_test_result_sign_does_not_panic_on_the_next_render() {
        let mut a = App::new("one\ntwo\nthree\n".into(), PathBuf::from("f.rs"));
        a.result_signs.insert(
            PathBuf::from("f.rs"),
            ruster_render::SignsView {
                width: 1,
                signs: vec![(0, '\u{2717}', ruster_render::Color::Default)],
            },
        );
        assert!(!a.result_signs.is_empty(), "precondition: the branch is now live");
        a.render();
    }

    #[test]
    fn a_diagnostic_outranks_a_git_sign_on_the_same_line() {
        let mut a = App::new("one\ntwo\nthree\n".into(), PathBuf::from("f.txt"));
        let id = a.ws.borrow().active_buffer();
        a.git.set_enabled(true);
        a.git.set_hunks(
            id,
            vec![ruster_git::Hunk { kind: ruster_git::HunkKind::Modified, start: 1, count: 1 }],
        );

        // Line 1 is changed *and* has an error.
        let signs = a.git_signs_for(id);
        assert_eq!(signs.at(1).map(|(g, _)| g), Some('~'), "git sign alone shows");

        let mut merged = signs;
        merged.signs.push((1, 'E', ruster_render::Color::Rgb(243, 139, 168)));
        assert_eq!(
            merged.at(1).map(|(g, _)| g),
            Some('E'),
            "the diagnostic pushed later wins the line"
        );
    }

    #[test]
    fn git_signs_map_each_hunk_kind_to_its_glyph() {
        let mut a = App::new("a\nb\nc\nd\ne\n".into(), PathBuf::from("f.txt"));
        let id = a.ws.borrow().active_buffer();
        a.git.set_enabled(true);
        a.git.set_hunks(
            id,
            vec![
                ruster_git::Hunk { kind: ruster_git::HunkKind::Added, start: 0, count: 2 },
                ruster_git::Hunk { kind: ruster_git::HunkKind::Modified, start: 3, count: 1 },
                ruster_git::Hunk { kind: ruster_git::HunkKind::Removed, start: 4, count: 0 },
            ],
        );
        let s = a.git_signs_for(id);
        assert_eq!(s.at(0).map(|(g, _)| g), Some('+'), "added spans its lines");
        assert_eq!(s.at(1).map(|(g, _)| g), Some('+'));
        assert_eq!(s.at(2), None, "unchanged line has no sign");
        assert_eq!(s.at(3).map(|(g, _)| g), Some('~'));
        assert_eq!(s.at(4).map(|(g, _)| g), Some('_'), "a deletion marks its boundary line");
    }

    #[test]
    fn git_signs_disabled_produces_nothing() {
        let mut a = App::new("x\n".into(), PathBuf::from("f.txt"));
        let id = a.ws.borrow().active_buffer();
        a.git.set_hunks(
            id,
            vec![ruster_git::Hunk { kind: ruster_git::HunkKind::Added, start: 0, count: 1 }],
        );
        a.git.set_enabled(false);
        assert!(a.git_signs_for(id).signs.is_empty(), "git.signs = false draws nothing");
    }

    #[test]
    fn gitsigns_command_toggles_and_clears() {
        let mut a = App::new("x\n".into(), PathBuf::from("f.txt"));
        let id = a.ws.borrow().active_buffer();
        a.git.set_hunks(
            id,
            vec![ruster_git::Hunk { kind: ruster_git::HunkKind::Added, start: 0, count: 1 }],
        );
        assert_eq!(a.parse_cmdline(":Gitsigns"), Ok(CmdAction::GitsignsToggle));
        assert!(a.git.enabled(), "on by default");
        a.apply_cmd(CmdAction::GitsignsToggle);
        assert!(!a.git.enabled());
        assert_eq!(a.git.tracked(), 0, "toggling off drops the cached hunks");
    }

    /// `]h` / `[h` move the cursor between hunks and wrap.
    #[test]
    fn bracket_h_jumps_between_git_hunks() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let none = KeyModifiers::NONE;
        let mut a = App::new("l0\nl1\nl2\nl3\nl4\nl5\n".into(), PathBuf::from("f.txt"));
        let id = a.ws.borrow().active_buffer();
        a.git.set_enabled(true);
        a.git.set_hunks(
            id,
            vec![
                ruster_git::Hunk { kind: ruster_git::HunkKind::Modified, start: 1, count: 1 },
                ruster_git::Hunk { kind: ruster_git::HunkKind::Added, start: 4, count: 1 },
            ],
        );
        let line_of = |a: &App| {
            let w = a.ws.borrow();
            w.buffer().char_to_line(w.primary_head())
        };

        a.handle_key(CtKey::new(KeyCode::Char(']'), none));
        a.handle_key(CtKey::new(KeyCode::Char('h'), none));
        assert_eq!(line_of(&a), 1, "]h goes to the first hunk");
        a.handle_key(CtKey::new(KeyCode::Char(']'), none));
        a.handle_key(CtKey::new(KeyCode::Char('h'), none));
        assert_eq!(line_of(&a), 4, "]h advances");
        a.handle_key(CtKey::new(KeyCode::Char('['), none));
        a.handle_key(CtKey::new(KeyCode::Char('h'), none));
        assert_eq!(line_of(&a), 1, "[h goes back");
    }

    /// Moving the selection repaints immediately, and cancelling puts the
    /// previous theme back — the picker must not leave the editor recoloured.
    #[test]
    fn theme_picker_previews_on_move_and_restores_on_cancel() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let none = KeyModifiers::NONE;
        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        let before_name = a.config.theme.clone();
        let before_bg = a.config.colors.bg;

        a.apply_cmd(CmdAction::Themes);
        assert!(a.picker.is_some(), "picker opened");
        assert_eq!(
            a.theme_before_preview.as_deref(),
            Some(before_name.as_str()),
            "remembers what to restore"
        );

        // Step through until the palette actually differs from where we started.
        let mut changed = false;
        for _ in 0..6 {
            a.handle_key(CtKey::new(KeyCode::Down, none));
            if a.config.colors.bg != before_bg {
                changed = true;
                break;
            }
        }
        assert!(changed, "moving the selection repainted the editor");

        a.handle_key(CtKey::new(KeyCode::Esc, none));
        assert!(a.picker.is_none());
        assert_eq!(a.config.theme, before_name, "theme name restored");
        assert_eq!(a.config.colors.bg, before_bg, "palette restored");
        assert!(a.theme_before_preview.is_none(), "nothing left to restore");
    }

    #[test]
    fn accepting_a_theme_keeps_it() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let none = KeyModifiers::NONE;
        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        let before_name = a.config.theme.clone();

        a.apply_cmd(CmdAction::Themes);
        for _ in 0..3 {
            a.handle_key(CtKey::new(KeyCode::Down, none));
        }
        let previewed = a.config.theme.clone();
        a.handle_key(CtKey::new(KeyCode::Enter, none));

        assert!(a.picker.is_none());
        assert_ne!(previewed, before_name, "moved off the starting theme");
        assert_eq!(a.config.theme, previewed, "accept keeps what was previewed");
        assert!(a.theme_before_preview.is_none(), "nothing to restore after accept");
    }

    /// The picker lists the built-ins, including all four Catppuccin variants.
    #[test]
    fn theme_discovery_lists_the_builtins() {
        let a = App::new("x".into(), PathBuf::from("f.txt"));
        let names = a.available_themes();
        for want in [
            "catppuccin-mocha",
            "catppuccin-latte",
            "catppuccin-frappe",
            "catppuccin-macchiato",
        ] {
            assert!(names.iter().any(|n| n == want), "{want} missing from {names:?}");
        }
    }

    #[test]
    fn bdelete_forgets_the_buffers_caches() {
        let tmp = std::env::temp_dir().join("ruster_forget_caches");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("a.txt"), "x").unwrap();

        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        a.apply_cmd(CmdAction::Dired(Some(tmp.to_string_lossy().into_owned())));
        let dired_id = a.ws.borrow().active_buffer();
        assert!(a.dired.styled_lines(dired_id).is_some(), "dired cached its listing");

        // Give it a diagnostics entry too, so the sweep is covered beyond dired.
        // A *non-empty* one: the accessor reports absent and empty alike, so an
        // empty vec here would satisfy the assertion below whether or not the
        // entry was actually dropped.
        a.lsp.set_diagnostics(
            dired_id,
            vec![ruster_lsp::Diagnostic {
                start: ruster_lsp::results::LspPositionEq { line: 0, character: 0 },
                end: ruster_lsp::results::LspPositionEq { line: 0, character: 1 },
                severity: 1,
                message: "x".into(),
            }],
        );

        a.apply_cmd(CmdAction::BufferDelete);
        assert!(a.ws.borrow().buffers.get(dired_id).is_none(), "buffer closed");
        assert!(a.dired.styled_lines(dired_id).is_none(), "dired caches dropped");
        assert!(a.dired.dir_of(dired_id).is_none());
        assert!(a.lsp.diagnostics(dired_id).is_empty(), "diagnostics dropped");
        assert!(!a.syntax.contains_key(&dired_id));
        assert!(!a.lsp.is_tracked(dired_id));
        assert!(!a.terminals.contains_key(&dired_id));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn bdelete_refuses_last_buffer_when_only_pinned_remain() {
        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        let orig = a.ws.borrow().active_buffer();
        a.apply_cmd(CmdAction::BufferDelete);
        // pinned dashboard + messages survive, file buffer removed
        assert_eq!(a.ws.borrow().buffers.len(), 2);
        assert!(a.ws.borrow().buffers.get(orig).is_none());
        // active buffer switched to a pinned one (dashboard or messages)
        assert!(a.ws.borrow().buffers.get(a.ws.borrow().active_buffer()).is_some_and(|d| d.pinned));
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
        assert!(a.file_prompt.is_none(), "search term did not trigger dired keys");
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
        assert!(a.file_prompt.is_none(), "n no longer creates");
        // `+` still opens the Create prompt.
        a.handle_key(CtKey::new(KeyCode::Char('+'), none));
        assert!(matches!(
            a.file_prompt,
            Some(FilePrompt { kind: FilePromptKind::Create, .. })
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
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
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
        a.handle_key(CtKey::new(KeyCode::Enter, KeyModifiers::NONE));
        let name = a.ws.borrow().active_doc().name.clone();
        assert!(name.ends_with("sub"), "descended into sub, got {name}");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(windows)]
    #[test]
    fn dired_ascends_from_drive_root_to_drive_picker() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let mut a = App::new(String::new(), PathBuf::from("f.txt"));
        a.apply_cmd(CmdAction::Dired(Some("C:\\".into())));
        // Ascend above the drive root: land in the drives view.
        a.handle_key(CtKey::new(KeyCode::Char('h'), KeyModifiers::NONE));
        let id = a.ws.borrow().active_buffer();
        assert!(ruster_core::dired::is_drives_view(a.dired.dir_of(id).unwrap()));
        assert_eq!(a.ws.borrow().active_doc().name, "Drives");
        let content = a.ws.borrow().buffer().to_string();
        assert!(content.contains("C:"), "drives view lists C:, got {content:?}");

        // Selecting the C: entry descends back into that drive.
        let c_line = content.lines().position(|l| l.starts_with("C:")).unwrap();
        let start = a.ws.borrow().buffer().line_start_char(c_line);
        a.ws.borrow_mut().execute(Action::Move(Motion::To(start)));
        a.handle_key(CtKey::new(KeyCode::Enter, KeyModifiers::NONE));
        let id2 = a.ws.borrow().active_buffer();
        assert!(!ruster_core::dired::is_drives_view(a.dired.dir_of(id2).unwrap()));
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
        assert!(a.file_prompt.is_some());
        for c in "new.txt".chars() {
            a.handle_key(CtKey::new(KeyCode::Char(c), none));
        }
        a.handle_key(CtKey::new(KeyCode::Enter, none));
        assert!(a.file_prompt.is_none());
        assert!(tmp.join("new.txt").exists(), "file created");

        // Move cursor onto new.txt (listing: "..", "new.txt") and delete it.
        let line1 = a.ws.borrow().buffer().line_start_char(1);
        a.ws.borrow_mut().execute(Action::Move(Motion::To(line1)));
        a.handle_key(CtKey::new(KeyCode::Char('D'), none));
        assert!(matches!(
            a.file_prompt.as_ref().map(|p| &p.kind),
            Some(FilePromptKind::Delete(_))
        ));
        a.handle_key(CtKey::new(KeyCode::Char('y'), none));
        assert!(!tmp.join("new.txt").exists(), "file deleted");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Deleting used to run the removal and return without refreshing anything,
    /// so the entry stayed on screen until something else forced a reload.
    #[test]
    fn dired_delete_refreshes_the_listing() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let none = KeyModifiers::NONE;
        let (mut a, tmp) = dired_on_temp("ruster_dired_del_refresh", &["doomed.txt"]);
        assert!(a.ws.borrow().buffer().to_string().contains("doomed.txt"));

        // Move onto doomed.txt (listing is "..", "doomed.txt") and delete it.
        let line1 = a.ws.borrow().buffer().line_start_char(1);
        a.ws.borrow_mut().execute(Action::Move(Motion::To(line1)));
        a.handle_key(CtKey::new(KeyCode::Char('D'), none));
        a.handle_key(CtKey::new(KeyCode::Char('y'), none));

        assert!(!tmp.join("doomed.txt").exists(), "file removed from disk");
        assert!(
            !a.ws.borrow().buffer().to_string().contains("doomed.txt"),
            "listing refreshed after the delete"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The sidebar's target directory used to live in a separate field that only
    /// the create/rename commit path cleared. A sidebar delete (or a cancelled
    /// sidebar prompt) left it set, and the next dired `+` then resolved against
    /// that stale directory instead of the buffer's own.
    #[test]
    fn sidebar_prompt_does_not_leak_into_the_next_dired_prompt() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let none = KeyModifiers::NONE;

        // A sidebar rooted somewhere entirely separate from the dired buffer.
        let side = std::env::temp_dir().join("ruster_leak_side");
        let _ = std::fs::remove_dir_all(&side);
        std::fs::create_dir_all(&side).unwrap();
        std::fs::write(side.join("victim.txt"), "x").unwrap();

        let (mut a, dired_dir) = dired_on_temp("ruster_leak_dired", &["other.txt"]);

        // Open the sidebar on `side`, focus it, and arm a delete prompt.
        a.sidebar.open(side.clone());
        a.handle_key(CtKey::new(KeyCode::Char('d'), none));
        assert!(a.file_prompt.is_some(), "sidebar armed a delete prompt");
        // Cancel it — this is the path that used to leave the directory behind.
        a.handle_key(CtKey::new(KeyCode::Char('n'), none));
        assert!(a.file_prompt.is_none());
        a.sidebar.set_focused(false);

        // Now create a file from dired. It must land in the dired directory.
        a.handle_key(CtKey::new(KeyCode::Char('+'), none));
        for c in "fresh.txt".chars() {
            a.handle_key(CtKey::new(KeyCode::Char(c), none));
        }
        a.handle_key(CtKey::new(KeyCode::Enter, none));

        assert!(dired_dir.join("fresh.txt").exists(), "created in the dired directory");
        assert!(!side.join("fresh.txt").exists(), "not in the stale sidebar directory");

        let _ = std::fs::remove_dir_all(&side);
        let _ = std::fs::remove_dir_all(&dired_dir);
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
        assert!(a.notify.history().iter().any(|n| n.text.contains("already exists")));

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
        let styled = crate::dired::styled_lines(&entries);
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
        assert!(a.dired.show_hidden());
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
        assert!(a.dired.pending_y(), "first y is pending");
        a.handle_key(CtKey::new(KeyCode::Char('y'), none));
        assert_eq!(a.dired.clipboard().map(|(_, cut)| *cut), Some(false));

        // Descend into sub/ and paste.
        let dired_buf = a.ws.borrow().active_buffer();
        a.dired.refresh(&mut a.ws.borrow_mut(), dired_buf, tmp.join("sub"));
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
        assert_eq!(a.dired.clipboard().map(|(_, cut)| *cut), Some(true));

        let dired_buf = a.ws.borrow().active_buffer();
        a.dired.refresh(&mut a.ws.borrow_mut(), dired_buf, tmp.join("sub"));
        a.handle_key(CtKey::new(KeyCode::Char('p'), none));
        assert!(tmp.join("sub").join("b.txt").exists(), "moved into sub/");
        assert!(!tmp.join("b.txt").exists(), "cut removes the original");
        assert!(a.dired.clipboard().is_none(), "cut is consumed by the paste");

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
        assert!(rows.iter().any(|r| r.desc.contains("hover")));
        assert!(rows.iter().any(|r| r.desc.contains("references")));
    }

    #[test]
    fn diagnostics_stored_and_surfaced_on_line() {
        let mut a = App::new("let x = 1;\n".into(), PathBuf::from("f.rs"));
        let buf = a.ws.borrow().active_buffer();
        a.lsp.set_diagnostics(
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

        // The lsp.diagnostics toggle suppresses the inline message.
        a.config.lsp_diagnostics = false;
        assert!(a.current_line_diagnostic().is_none(), "diagnostics off → no message");
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
        // Expanded groups.
        assert!(matches!(
            leader_resolve(&['o', 't']),
            LeaderResolve::Action(LeaderAction::Terminal)
        ));
        assert!(matches!(
            leader_resolve(&['u', 'n']),
            LeaderResolve::Action(LeaderAction::ToggleNumber)
        ));
        assert!(matches!(
            leader_resolve(&['b', 'd']),
            LeaderResolve::Action(LeaderAction::BufferDelete)
        ));
        assert!(matches!(
            leader_resolve(&['s', 's']),
            LeaderResolve::Action(LeaderAction::DocumentSymbol)
        ));
        // The debug group: `SPC d` is a prefix, `SPC d b` toggles a breakpoint.
        assert!(matches!(leader_resolve(&['d']), LeaderResolve::Group));
        assert!(matches!(
            leader_resolve(&['d', 'b']),
            LeaderResolve::Action(LeaderAction::DebugToggleBreakpoint)
        ));
        assert!(matches!(
            leader_resolve(&['d', 'o']),
            LeaderResolve::Action(LeaderAction::DebugStepOut)
        ));
        // Settings: top-level `SPC ,` and the `SPC o s` group entry.
        assert!(matches!(leader_resolve(&[',']), LeaderResolve::Action(LeaderAction::Settings)));
        assert!(matches!(leader_resolve(&['o', 's']), LeaderResolve::Action(LeaderAction::Settings)));
    }

    #[test]
    fn command_palette_placement_follows_config() {
        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        // Default: centered.
        a.open_command_picker("");
        assert_eq!(
            a.picker.as_mut().unwrap().view().placement,
            ruster_render::PickerPlacement::Center
        );
        // With the bottom setting, the palette docks at the bottom.
        a.picker = None;
        a.config.command_palette = "bottom".to_string();
        a.open_command_picker("");
        assert_eq!(
            a.picker.as_mut().unwrap().view().placement,
            ruster_render::PickerPlacement::Bottom
        );
    }

    #[test]
    fn g_menu_starts_and_gg_goes_to_top() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let none = KeyModifiers::NONE;
        let mut a = App::new("aaa\nbbb\nccc\n".into(), PathBuf::from("f.txt"));
        // Move off the top.
        a.handle_key(CtKey::new(KeyCode::Char('G'), none));
        assert!(a.ws.borrow().primary_head() > 0);
        // `g` opens the menu (pending); a second `g` replays gg → top of buffer.
        a.handle_key(CtKey::new(KeyCode::Char('g'), none));
        assert!(a.g_pending.is_some(), "g starts the menu");
        a.handle_key(CtKey::new(KeyCode::Char('g'), none));
        assert!(a.g_pending.is_none(), "second key resolves the menu");
        assert_eq!(a.ws.borrow().primary_head(), 0, "gg went to the top");
    }

    /// Backspace closes the `g` menu explicitly rather than falling into the
    /// replay arm. Both paths happen to end up here — vim clears `pending_g` on
    /// the next key either way — so this is a characterization test pinning the
    /// contract, not a fix for an observable bug.
    #[test]
    fn backspace_cancels_the_g_menu_without_replaying() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let none = KeyModifiers::NONE;
        let mut a = App::new("aaa\nbbb\nccc\n".into(), PathBuf::from("f.txt"));
        a.handle_key(CtKey::new(KeyCode::Char('G'), none));
        let before = a.ws.borrow().primary_head();
        assert!(before > 0);

        a.handle_key(CtKey::new(KeyCode::Char('g'), none));
        assert!(a.g_pending.is_some(), "g starts the menu");
        a.handle_key(CtKey::new(KeyCode::Backspace, none));
        assert!(a.g_pending.is_none(), "Backspace closes the menu");
        assert_eq!(
            a.ws.borrow().primary_head(),
            before,
            "cancelling must not replay g into vim and move the cursor"
        );
    }

    /// Backspace steps back out of a pending leader sequence. The tree is only
    /// one level deep today, so popping the single group key empties the
    /// sequence and cancels — the same visible result as Esc. The pop only
    /// starts to matter once a group nests; this pins the behaviour until then.
    #[test]
    fn backspace_steps_back_out_of_the_leader_sequence() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let none = KeyModifiers::NONE;
        let mut a = App::new("x".into(), PathBuf::from("f.txt"));

        a.handle_key(CtKey::new(KeyCode::Char(' '), none));
        assert_eq!(a.leader_pending.as_deref(), Some(&[][..]), "SPC arms the leader");
        a.handle_key(CtKey::new(KeyCode::Char('c'), none));
        assert_eq!(a.leader_pending.as_deref(), Some(&['c'][..]), "c opens the code group");

        a.handle_key(CtKey::new(KeyCode::Backspace, none));
        assert!(a.leader_pending.is_none(), "popping the last key leaves the leader");

        // Backspace straight after SPC also leaves cleanly.
        a.handle_key(CtKey::new(KeyCode::Char(' '), none));
        a.handle_key(CtKey::new(KeyCode::Backspace, none));
        assert!(a.leader_pending.is_none());
    }

    #[test]
    fn leader_whichkey_shows_groups() {
        let (title, rows) = leader_whichkey(&[]).expect("root panel");
        assert_eq!(title, "SPC");
        assert!(rows.iter().any(|r| r.desc.contains("+windows")));
        assert!(rows.iter().any(|r| r.desc.contains("+quit")));

        let (wtitle, wrows) = leader_whichkey(&['w']).expect("window panel");
        assert_eq!(wtitle, "SPC w");
        assert!(wrows.iter().any(|r| r.key == "h"));
        assert!(wrows.iter().any(|r| r.desc.contains("focus left")));
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

    #[test]
    fn settings_page_opens_and_captures_keys() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let none = KeyModifiers::NONE;
        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        assert_eq!(a.parse_cmdline(":settings"), Ok(CmdAction::Settings));
        a.apply_cmd(CmdAction::Settings);
        assert!(a.settings.is_some(), "settings page opened");

        // Navigation is captured by the settings handler, not the buffer.
        let before = a.ws.borrow().buffer().to_string();
        a.handle_key(CtKey::new(KeyCode::Char('j'), none));
        a.handle_key(CtKey::new(KeyCode::Char('k'), none));
        assert_eq!(a.ws.borrow().buffer().to_string(), before, "keys don't reach the buffer");

        // q closes it.
        a.handle_key(CtKey::new(KeyCode::Char('q'), none));
        assert!(a.settings.is_none(), "q closes the settings page");
    }

    #[test]
    fn term_command_parses_and_opens_a_terminal() {
        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        assert_eq!(a.parse_cmdline(":term"), Ok(CmdAction::Terminal));
        assert_eq!(a.parse_cmdline(":terminal"), Ok(CmdAction::Terminal));
        a.apply_cmd(CmdAction::Terminal);
        assert!(a.active_terminal_buffer().is_some(), "active buffer is a terminal");
        assert!(a.terminal_focused, "a fresh terminal is focused");
    }

    // `cat` echoes stdin → deterministic; not available on Windows.
    #[cfg(not(windows))]
    #[test]
    fn keystrokes_reach_the_terminal_pty() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let none = KeyModifiers::NONE;
        let mut a = App::new("x".into(), PathBuf::from("f.txt"));

        // Open a terminal backed by `cat`, which echoes what we type.
        let id = a.ws.borrow_mut().buffers.create_special(SpecialKind::Terminal, "*terminal*");
        a.ws.borrow_mut().set_active_buffer(id);
        let session = TerminalSession::spawn("cat", &[], 40, 6, 1000).expect("spawn cat");
        a.terminals.insert(id, session);
        a.terminal_focused = true;

        for c in "ping".chars() {
            a.handle_key(CtKey::new(KeyCode::Char(c), none));
        }
        a.handle_key(CtKey::new(KeyCode::Enter, none));

        let mut found = false;
        for _ in 0..200 {
            let snap = a.terminals.get(&id).unwrap().snapshot();
            if (0..snap.rows).any(|r| snap.row_text(r).contains("ping")) {
                found = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(found, "typed text should be echoed into the terminal grid");
    }

    #[cfg(not(windows))]
    #[test]
    fn ctrl_backslash_enters_terminal_normal_and_mirrors_output() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let none = KeyModifiers::NONE;
        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        let id = a.ws.borrow_mut().buffers.create_special(SpecialKind::Terminal, "*terminal*");
        a.ws.borrow_mut().set_active_buffer(id);
        a.terminals.insert(id, TerminalSession::spawn("cat", &[], 40, 6, 1000).expect("spawn"));
        a.terminal_focused = true;

        for c in "hello".chars() {
            a.handle_key(CtKey::new(KeyCode::Char(c), none));
        }
        a.handle_key(CtKey::new(KeyCode::Enter, none));
        for _ in 0..200 {
            if a.terminals.get(&id).unwrap().snapshot().row_text(0).contains("hello") {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        // Ctrl-\ enters Terminal-Normal: the grid is mirrored into the buffer.
        a.handle_key(CtKey::new(KeyCode::Char('\\'), KeyModifiers::CONTROL));
        assert!(!a.terminal_focused, "Ctrl-\\ leaves insert");
        let buf = a.ws.borrow().buffers.get(id).unwrap().buffer.to_string();
        assert!(buf.contains("hello"), "buffer mirrors terminal output: {buf:?}");

        // Vim motions work over the mirror; `i` resumes insert.
        a.handle_key(CtKey::new(KeyCode::Char('G'), none));
        a.handle_key(CtKey::new(KeyCode::Char('i'), none));
        assert!(a.terminal_focused, "i resumes terminal-insert");
    }

    #[cfg(not(windows))]
    #[test]
    fn ctrl_backslash_defocuses_the_terminal() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        let id = a.ws.borrow_mut().buffers.create_special(SpecialKind::Terminal, "*terminal*");
        a.ws.borrow_mut().set_active_buffer(id);
        a.terminals.insert(id, TerminalSession::spawn("cat", &[], 40, 6, 1000).expect("spawn"));
        a.terminal_focused = true;

        a.handle_key(CtKey::new(KeyCode::Char('\\'), KeyModifiers::CONTROL));
        assert!(!a.terminal_focused, "Ctrl-\\ leaves terminal focus");
        // `i` re-enters.
        a.handle_key(CtKey::new(KeyCode::Char('i'), KeyModifiers::NONE));
        assert!(a.terminal_focused, "i re-focuses the terminal");
    }

    #[test]
    fn build_and_test_commands_parse() {
        let a = App::new("x".into(), PathBuf::from("f.txt"));
        assert!(matches!(a.parse_cmdline("build"), Ok(CmdAction::Build)));
        assert!(matches!(a.parse_cmdline("make"), Ok(CmdAction::Build)));
        assert!(matches!(a.parse_cmdline("test"), Ok(CmdAction::Test)));
        assert!(matches!(a.parse_cmdline("task"), Ok(CmdAction::TaskPicker)));
        assert!(matches!(
            leader_resolve(&['c', 't']),
            LeaderResolve::Action(LeaderAction::Test)
        ));
        assert!(matches!(
            leader_resolve(&['o', 'r']),
            LeaderResolve::Action(LeaderAction::Tasks)
        ));
    }

    #[test]
    fn quickfix_commands_parse() {
        let a = App::new("x".into(), PathBuf::from("f.txt"));
        assert!(matches!(a.parse_cmdline("copen"), Ok(CmdAction::QuickfixOpen)));
        assert!(matches!(a.parse_cmdline("cnext"), Ok(CmdAction::QuickfixNext)));
        assert!(matches!(a.parse_cmdline("cn"), Ok(CmdAction::QuickfixNext)));
        assert!(matches!(a.parse_cmdline("cprev"), Ok(CmdAction::QuickfixPrev)));
        assert!(matches!(a.parse_cmdline("cp"), Ok(CmdAction::QuickfixPrev)));
    }

    #[test]
    fn diagnostics_build_a_sign_column() {
        use ruster_lsp::results::{Diagnostic, LspPositionEq};
        let pos = |l: u32| LspPositionEq { line: l, character: 0 };
        let diags = vec![
            Diagnostic { start: pos(2), end: pos(2), severity: 2, message: "warn".into() },
            // A second, more severe diagnostic on the same line wins.
            Diagnostic { start: pos(2), end: pos(2), severity: 1, message: "err".into() },
            Diagnostic { start: pos(5), end: pos(5), severity: 3, message: "info".into() },
        ];
        let signs = diagnostics_to_signs(&diags);
        assert_eq!(signs.width, 1);
        assert_eq!(signs.at(2).map(|(g, _)| g), Some('E'), "error outranks warning on line 2");
        assert_eq!(signs.at(5).map(|(g, _)| g), Some('I'));
        assert_eq!(signs.at(9), None);
        // No diagnostics → no sign column.
        assert_eq!(diagnostics_to_signs(&[]).width, 0);
    }

    #[test]
    fn flash_label_pool_starts_with_a() {
        let mut pool = super::label_pool_iter();
        assert_eq!(pool.next(), Some("a".to_string()));
        assert_eq!(pool.next(), Some("b".to_string()));
    }

    #[test]
    fn flash_label_pool_wraps_to_aa_after_z() {
        let mut pool = super::label_pool_iter();
        // Skip a-z
        for _ in 0..26 { pool.next(); }
        assert_eq!(pool.next(), Some("aa".to_string()));
        assert_eq!(pool.next(), Some("ab".to_string()));
    }

    #[test]
    fn flash_label_pool_ba_follows_az() {
        let mut pool = super::label_pool_iter();
        // Skip a-z, aa-az (26 + 26 = 52)
        for _ in 0..52 { pool.next(); }
        assert_eq!(pool.next(), Some("ba".to_string()));
    }

    /// `f` in Normal mode labels every visible word, in reading order.
    #[test]
    fn flash_f_labels_visible_words_in_order() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let mut a = App::new("alpha beta\ngamma\n".into(), PathBuf::from("f.txt"));
        a.handle_key(CtKey::new(KeyCode::Char('f'), KeyModifiers::NONE));
        let fs = a.flash.as_ref().expect("flash mode active after f");
        assert_eq!(fs.pending, None);
        let labels: Vec<(&str, usize)> =
            fs.labels.iter().map(|l| (l.label.as_str(), l.offset)).collect();
        assert_eq!(labels, vec![("a", 0), ("b", 6), ("c", 11)]);
    }

    /// Label offsets are char offsets, so a multi-byte char earlier on the line
    /// must not skew the words after it.
    #[test]
    fn flash_label_offsets_are_char_offsets_not_bytes() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        // "é" is one char but two bytes; "beta" starts at char 2, byte 3.
        let mut a = App::new("é beta\n".into(), PathBuf::from("f.txt"));
        a.handle_key(CtKey::new(KeyCode::Char('f'), KeyModifiers::NONE));
        let fs = a.flash.as_ref().expect("flash mode active after f");
        let labels: Vec<(&str, usize)> =
            fs.labels.iter().map(|l| (l.label.as_str(), l.offset)).collect();
        assert_eq!(labels, vec![("a", 0), ("b", 2)]);
    }

    /// A first keystroke matching exactly one label jumps without waiting.
    #[test]
    fn flash_single_match_jumps_immediately() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let none = KeyModifiers::NONE;
        let mut a = App::new("alpha beta\n".into(), PathBuf::from("f.txt"));
        a.handle_key(CtKey::new(KeyCode::Char('f'), none));
        // Labels are a -> "alpha" (0) and b -> "beta" (6).
        a.handle_key(CtKey::new(KeyCode::Char('b'), none));
        assert!(a.flash.is_none(), "flash cleared after the jump");
        assert_eq!(a.ws.borrow().primary_head(), 6);
    }

    /// With more words than single-char labels, the first key narrows and the
    /// second key jumps.
    #[test]
    fn flash_two_char_label_jumps_on_second_key() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let none = KeyModifiers::NONE;
        // 30 two-char words on one line: word i starts at char 3*i.
        let content = vec!["zz"; 30].join(" ") + "\n";
        let mut a = App::new(content.clone(), PathBuf::from("f.txt"));
        a.handle_key(CtKey::new(KeyCode::Char('f'), none));
        assert_eq!(a.flash.as_ref().unwrap().labels.len(), 30);

        // Labels: a..z for words 0..25, then aa, ab, ac, ad for words 26..29.
        a.handle_key(CtKey::new(KeyCode::Char('a'), none));
        let fs = a.flash.as_ref().expect("still active, waiting for second char");
        assert_eq!(fs.pending, Some('a'));
        assert_eq!(fs.labels.len(), 5, "a, aa, ab, ac, ad");

        a.handle_key(CtKey::new(KeyCode::Char('b'), none));
        assert!(a.flash.is_none());
        assert_eq!(a.ws.borrow().primary_head(), 27 * 3, "label ab -> word 27");
        // Label keys must never reach the Vim layer: `a` would open Insert mode
        // and `b` would land in the buffer.
        assert_eq!(a.ws.borrow().buffer().to_string(), content);
        assert_eq!(a.vim.mode, VimMode::Normal);
    }

    /// Esc leaves flash mode without moving the cursor.
    #[test]
    fn flash_esc_cancels_without_moving() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let none = KeyModifiers::NONE;
        let mut a = App::new("alpha beta gamma\n".into(), PathBuf::from("f.txt"));
        let before = a.ws.borrow().primary_head();
        a.handle_key(CtKey::new(KeyCode::Char('f'), none));
        assert!(a.flash.is_some());
        a.handle_key(CtKey::new(KeyCode::Esc, none));
        assert!(a.flash.is_none());
        assert_eq!(a.ws.borrow().primary_head(), before);
    }

    /// A key that can't be a label cancels flash and is replayed as a normal
    /// key, rather than being swallowed.
    #[test]
    fn flash_non_label_key_cancels_and_replays() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let none = KeyModifiers::NONE;
        let mut a = App::new("alpha beta\n".into(), PathBuf::from("f.txt"));
        // Move off column 0 so the replayed `0` is observable.
        a.handle_key(CtKey::new(KeyCode::Char('w'), none));
        assert_eq!(a.ws.borrow().primary_head(), 6);

        a.handle_key(CtKey::new(KeyCode::Char('f'), none));
        assert!(a.flash.is_some());
        a.handle_key(CtKey::new(KeyCode::Char('0'), none));
        assert!(a.flash.is_none(), "non-label key cancels flash");
        assert_eq!(a.ws.borrow().primary_head(), 0, "`0` still ran as a motion");
    }

    /// Lay out a frame and hand back the active window's text area, so hit-test
    /// assertions are relative to the real geometry rather than to a guess about
    /// the viewport size.
    fn rendered_text_area(a: &mut App) -> ruster_render::TextArea {
        a.render();
        a.last_layout.first().expect("one window was laid out").text
    }

    #[test]
    fn mouse_hit_test_maps_clicks_to_buffer_offsets() {
        let mut a = App::new("alpha\nbravo\ncharlie\n".into(), PathBuf::from("f.txt"));
        a.config.number = true;
        let text = rendered_text_area(&mut a);
        // Top-left text cell is the first char of the buffer.
        assert_eq!(a.buffer_offset_at(text.x, text.y).map(|(_, o)| o), Some(0));
        // Second row, third column: 'a' of "bravo", which starts at offset 6.
        assert_eq!(a.buffer_offset_at(text.x + 2, text.y + 1).map(|(_, o)| o), Some(8));
    }

    /// The header row and the number gutter are not buffer text. Getting this
    /// wrong shifted every click by a row and by the gutter width.
    #[test]
    fn mouse_hit_test_rejects_window_chrome_and_gutter() {
        let mut a = App::new("alpha\nbravo\n".into(), PathBuf::from("f.txt"));
        a.config.number = true;
        let text = rendered_text_area(&mut a);
        assert!(text.x > 0, "the number gutter reserves columns");
        assert!(text.y > 0, "the header row sits above the text");
        assert_eq!(a.buffer_offset_at(text.x, text.y - 1), None, "header row");
        assert_eq!(a.buffer_offset_at(text.x - 1, text.y), None, "gutter column");
    }

    /// Clicking the blank space below a short buffer used to index the rope past
    /// its last line, which panics inside ropey and took the editor down.
    #[test]
    fn mouse_hit_test_below_last_line_is_none() {
        let mut a = App::new("alpha\nbravo\n".into(), PathBuf::from("f.txt"));
        let text = rendered_text_area(&mut a);
        assert!(text.height > 3, "need blank rows below the text");
        assert_eq!(a.buffer_offset_at(text.x, text.y + text.height - 1), None);
    }

    #[test]
    fn mouse_hit_test_clamps_past_end_of_line() {
        let mut a = App::new("ab\nlonger line\n".into(), PathBuf::from("f.txt"));
        let text = rendered_text_area(&mut a);
        // Column 9 is past the end of "ab", so it lands on 'b'.
        assert_eq!(a.buffer_offset_at(text.x + 9, text.y).map(|(_, o)| o), Some(1));
    }

    /// With the sidebar open every window shifts right; the hit-test reads the
    /// layout that was drawn, so it follows automatically.
    #[test]
    fn mouse_hit_test_follows_the_sidebar_offset() {
        let mut a = App::new("alpha\nbravo\n".into(), PathBuf::from("f.txt"));
        let before = rendered_text_area(&mut a).x;
        a.toggle_sidebar();
        let after = rendered_text_area(&mut a);
        assert!(after.x > before, "sidebar pushes the text area right");
        assert_eq!(a.buffer_offset_at(after.x, after.y).map(|(_, o)| o), Some(0));
        // A click in the sidebar column is not buffer text.
        assert_eq!(a.buffer_offset_at(before, after.y), None);
    }

    #[test]
    fn git_commit_and_remote_commands_parse() {
        let a = App::new("x".into(), PathBuf::from("f.rs"));
        assert_eq!(a.parse_cmdline(":GitCommit"), Ok(CmdAction::GitCommit));
        assert_eq!(a.parse_cmdline(":commit"), Ok(CmdAction::GitCommit));
        assert_eq!(a.parse_cmdline(":GitPush"), Ok(CmdAction::GitPush));
        assert_eq!(a.parse_cmdline(":pull"), Ok(CmdAction::GitPull));
    }

    /// Push and pull talk to a remote, so neither may run on a keypress alone.
    #[test]
    fn pushing_asks_before_running_anything() {
        let mut a = App::new("x".into(), PathBuf::from("f.rs"));
        a.apply_cmd(CmdAction::GitPush);
        assert!(a.runner_rx.is_none(), "nothing spawned by asking");
        let d = a.dialog.as_ref().expect("a confirmation dialog");
        assert!(d.view().title.contains("Push"), "{:?}", d.view().title);
        let p = a.pending_confirm.as_ref().expect("a command is pending");
        assert_eq!(p.cmd, "git push");
        assert_eq!(p.verb, "Push");
    }

    /// The shared confirmation slot must not confuse a push with an install:
    /// pressing the *other* verb declines.
    #[test]
    fn confirming_with_the_wrong_verb_declines() {
        let mut a = App::new("x".into(), PathBuf::from("f.rs"));
        a.pending_confirm = Some(PendingConfirm {
            cmd: "git push".to_string(),
            kind: RunnerKind::Git,
            verb: "Push".to_string(),
        });
        a.run_pending_confirm(Some("Install"));
        assert!(a.runner_rx.is_none(), "the wrong button ran nothing");
        assert!(a.pending_confirm.is_none(), "and the command is forgotten");
    }

    /// A commit message buffer must be editable — every other special buffer is
    /// read-only, and this one exists to be typed into.
    #[test]
    fn the_commit_buffer_is_editable() {
        let doc = ruster_core::document::Document::special(
            ruster_core::document::SpecialKind::GitCommit,
            "*git-commit*",
        );
        assert!(!doc.read_only());
    }

    #[test]
    fn committing_with_nothing_staged_declines() {
        let dir = shot_dir();
        let mut a = App::new("x".into(), dir.join("f.rs"));
        a.project_root = Some(dir);
        a.apply_cmd(CmdAction::GitCommit);
        assert!(!a.active_is_git_commit(), "no message buffer opened");
        let last = a.notify.history().last().expect("a message");
        assert!(last.text.contains("repository") || last.text.contains("staged"), "{:?}", last.text);
    }

    /// Regression: with the settings page open, every command that was not
    /// settings-specific was silently swallowed — `:Git`, `:help` and the rest
    /// did nothing and said nothing. Found in the GUI, and true in the TUI too.
    #[test]
    fn a_command_while_settings_is_open_closes_it_and_runs() {
        let mut a = App::new("x".into(), PathBuf::from("f.rs"));
        a.apply_cmd(CmdAction::Settings);
        assert!(a.settings.is_some(), "the page is open");

        a.apply_cmd(CmdAction::Help(None));
        assert!(a.settings.is_none(), "the page stepped aside");
        assert!(a.active_is_help(), "and the command actually ran");
    }

    /// The two that genuinely mean something different there still do.
    #[test]
    fn save_and_quit_still_belong_to_the_settings_page() {
        let mut a = App::new("x".into(), PathBuf::from("f.rs"));
        a.apply_cmd(CmdAction::Settings);
        a.apply_cmd(CmdAction::Quit);
        assert!(a.settings.is_none(), ":q closes the page");
        assert!(!a.should_quit, "and does not quit the editor");

        a.apply_cmd(CmdAction::Settings);
        assert!(a.settings.is_some());
        a.apply_cmd(CmdAction::Save(false));
        assert!(a.settings.is_some(), ":w saves the page and leaves it open");
    }

    #[test]
    fn git_staged_parses() {
        let a = App::new("x".into(), PathBuf::from("f.rs"));
        assert_eq!(a.parse_cmdline(":GitStaged"), Ok(CmdAction::GitStaged));
        assert_eq!(a.parse_cmdline(":staged"), Ok(CmdAction::GitStaged));
    }

    /// The cursor line in a diff buffer maps straight to a hunk — that is the
    /// whole reason unstaging happens here rather than in the file buffer.
    #[test]
    fn a_cursor_line_in_the_staged_diff_resolves_to_its_hunk() {
        let diff = "\
diff --git a/f.txt b/f.txt
index 1..2 100644
--- a/f.txt
+++ b/f.txt
@@ -1,3 +1,3 @@
 a
-b
+B
@@ -10,3 +10,3 @@
 c
-d
+D
";
        let map = ruster_git::hunk_of_line(diff);
        let lines: Vec<&str> = diff.lines().collect();
        let first_change = lines.iter().position(|l| *l == "+B").unwrap();
        let second_change = lines.iter().position(|l| *l == "+D").unwrap();
        assert_eq!(map[first_change], Some(0));
        assert_eq!(map[second_change], Some(1));
        // A file header belongs to no hunk, so `u` there declines.
        assert_eq!(map[0], None);

        // And those indices address the right patches.
        let patches = ruster_git::split_hunks(diff);
        assert!(patches[0].contains("+B") && !patches[0].contains("+D"));
        assert!(patches[1].contains("+D") && !patches[1].contains("+B"));
    }

    #[test]
    fn git_staged_outside_a_repository_warns() {
        let dir = shot_dir();
        let mut a = App::new("x".into(), dir.join("f.rs"));
        a.project_root = Some(dir);
        a.apply_cmd(CmdAction::GitStaged);
        assert!(!a.active_is_git_staged(), "no diff buffer opened");
        let last = a.notify.history().last().expect("a message");
        assert!(last.text.contains("git repository"), "{:?}", last.text);
    }

    #[test]
    fn git_stage_hunk_parses() {
        let a = App::new("x".into(), PathBuf::from("f.rs"));
        assert_eq!(a.parse_cmdline(":GitStageHunk"), Ok(CmdAction::GitStageHunk));
        assert_eq!(a.parse_cmdline(":stagehunk"), Ok(CmdAction::GitStageHunk));
    }

    /// Outside a repository the command must decline rather than shell out.
    #[test]
    fn git_stage_hunk_outside_a_project_warns() {
        let dir = shot_dir();
        let f = dir.join("loose.rs");
        std::fs::write(&f, "x\n").unwrap();
        let mut a = App::new("x".into(), f);
        a.project_root = None;
        a.apply_cmd(CmdAction::GitStageHunk);
        let last = a.notify.history().last().expect("a message");
        assert!(last.text.contains("git repository"), "{:?}", last.text);
    }

    /// A buffer with no file on disk has no hunks to stage.
    #[test]
    fn git_stage_hunk_needs_a_file() {
        // A real repository, so the run gets past the repo check to the one
        // being tested — the working directory during a test run is one.
        let mut a = App::new("scratch".into(), PathBuf::from(""));
        a.project_root = std::env::current_dir().ok();
        {
            let id = a.ws.borrow().active_buffer();
            a.ws.borrow_mut().buffers.get_mut(id).unwrap().file_path = None;
        }
        a.apply_cmd(CmdAction::GitStageHunk);
        let last = a.notify.history().last().expect("a message");
        assert!(last.text.contains("No file"), "{:?}", last.text);
    }

    #[test]
    fn git_status_parses() {
        let a = App::new("x".into(), PathBuf::from("f.rs"));
        assert_eq!(a.parse_cmdline(":Git"), Ok(CmdAction::GitStatus));
        assert_eq!(a.parse_cmdline(":git"), Ok(CmdAction::GitStatus));
    }

    /// A project root that is not a git repository must say so rather than
    /// opening an empty view.
    ///
    /// The root is set explicitly: `App` falls back to the working directory,
    /// which during a test run *is* a repository, so a temp file alone would
    /// not exercise this path.
    #[test]
    fn git_status_outside_a_repository_warns() {
        let dir = shot_dir();
        let mut a = App::new("x".into(), dir.join("loose.rs"));
        a.project_root = Some(dir);
        a.apply_cmd(CmdAction::GitStatus);
        assert!(!a.active_is_git_status(), "no status buffer opened");
        let last = a.notify.history().last().expect("a message");
        assert!(last.text.contains("git repository"), "{:?}", last.text);
    }

    /// Folding must not shell out again — a `git status` per keypress would be
    /// visible on any large repository.
    #[test]
    fn folding_re_renders_without_re_reading_git() {
        let mut a = App::new("x".into(), PathBuf::from("f.rs"));
        a.git_status.set_status(ruster_git::parse_status(
            "# branch.head main\n1 A. N... 0 0 0 a b one.txt\n? two.txt\n",
        ));
        let before = a.git_status.rows().len();
        a.git_status.toggle_at(0);
        assert!(a.git_status.rows().len() < before, "folded from state alone");
    }

    /// Regression: the view puts a blank line before every section after the
    /// first, so a constant header offset drifts by one per section and a file
    /// in the *second* section resolved to the wrong row — or to none at all.
    /// Found by pressing `G` on an untracked file and being told to put the
    /// cursor on a file.
    #[test]
    fn a_file_in_a_later_section_resolves_to_its_own_row() {
        let mut a = App::new("x".into(), PathBuf::from("f.rs"));
        a.git_status.set_status(ruster_git::parse_status(
            "# branch.head main\n1 .M N... 0 0 0 a b edited.txt\n? untracked.txt\n",
        ));
        let text = a.git_status.render(None);

        for name in ["edited.txt", "untracked.txt"] {
            let line = text.lines().position(|l| l.contains(name)).expect("listed");
            let row = a.git_status.row_at_line(line).expect("a row");
            assert_eq!(
                a.git_status.path_at(row),
                Some(PathBuf::from(name)),
                "{name} on line {line} resolved to the wrong row"
            );
        }

        // A heading resolves to its own row, so folding works with the cursor
        // on it — but that row is not a file, so `Enter` still does nothing.
        let heading = text.lines().position(|l| l.contains("Untracked")).unwrap();
        let hrow = a.git_status.row_at_line(heading).expect("a heading has a row");
        assert_eq!(a.git_status.path_at(hrow), None, "a heading is not a file");
        assert_eq!(a.git_status.row_at_line(0), None, "the branch header is not a row");
        assert_eq!(a.git_status.row_at_line(9999), None);
    }

    #[test]
    fn help_parses_with_and_without_a_topic() {
        let a = App::new("x".into(), PathBuf::from("f.rs"));
        assert_eq!(a.parse_cmdline(":help"), Ok(CmdAction::Help(None)));
        assert_eq!(a.parse_cmdline(":h"), Ok(CmdAction::Help(None)));
        assert_eq!(
            a.parse_cmdline(":help Mason"),
            Ok(CmdAction::Help(Some("Mason".to_string())))
        );
        assert_eq!(a.parse_cmdline(":h :w"), Ok(CmdAction::Help(Some(":w".to_string()))));
    }

    #[test]
    fn help_opens_the_manual_at_the_top() {
        let mut a = App::new("x".into(), PathBuf::from("f.rs"));
        a.apply_cmd(CmdAction::Help(None));
        assert!(a.active_is_help());
        let w = a.ws.borrow();
        let text = w.active_doc().buffer.to_string();
        assert!(text.starts_with("# ruster help"), "{}", &text[..40.min(text.len())]);
        assert_eq!(w.primary_head(), 0, "at the top");
    }

    /// The useful bit: a topic puts the cursor on the right line, not line 0.
    #[test]
    fn a_topic_jumps_to_its_line() {
        let mut a = App::new("x".into(), PathBuf::from("f.rs"));
        a.apply_cmd(CmdAction::Help(Some("Mason".into())));
        let w = a.ws.borrow();
        let doc = w.active_doc();
        let line = doc.buffer.char_to_line(w.primary_head());
        assert!(line > 0, "jumped somewhere");
        let text = doc.buffer.line_to_string(line);
        assert!(text.contains("Mason"), "landed on a Mason line: {text:?}");
        assert_eq!(
            w.windows.active_window().scroll_top,
            line,
            "and scrolled there, so it is actually on screen"
        );
    }

    /// An unknown topic must still show the manual — the reader is looking for
    /// something, and refusing leaves them with nothing.
    #[test]
    fn an_unknown_topic_still_opens_the_manual_with_a_warning() {
        let mut a = App::new("x".into(), PathBuf::from("f.rs"));
        a.apply_cmd(CmdAction::Help(Some("nonsense-topic-zzz".into())));
        assert!(a.active_is_help(), "the manual is open regardless");
        assert_eq!(a.ws.borrow().primary_head(), 0, "at the top");
        let last = a.notify.history().last().expect("a message");
        assert!(last.text.contains("No help for"), "{:?}", last.text);
    }

    /// Reopening must not stack up `*help*` buffers.
    #[test]
    fn reopening_help_reuses_the_same_buffer() {
        let mut a = App::new("x".into(), PathBuf::from("f.rs"));
        a.apply_cmd(CmdAction::Help(None));
        let n = a.ws.borrow().buffers.ids().len();
        a.apply_cmd(CmdAction::Help(Some("Windows".into())));
        assert_eq!(a.ws.borrow().buffers.ids().len(), n, "no second manual");
    }

    #[test]
    fn q_closes_the_manual_and_other_keys_fall_through() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let mut a = App::new("x".into(), PathBuf::from("f.rs"));
        a.apply_cmd(CmdAction::Help(None));
        // `j` is not claimed, so reading still works.
        assert!(!a.handle_help_key(CtKey::new(KeyCode::Char('j'), KeyModifiers::NONE)));
        assert!(a.handle_help_key(CtKey::new(KeyCode::Char('q'), KeyModifiers::NONE)));
        assert!(!a.active_is_help(), "closed");
    }

    #[test]
    fn session_commands_parse() {
        let a = App::new("x".into(), PathBuf::from("f.rs"));
        assert_eq!(a.parse_cmdline(":SessionSave"), Ok(CmdAction::SessionSave));
        assert_eq!(a.parse_cmdline(":mksession"), Ok(CmdAction::SessionSave));
        assert_eq!(a.parse_cmdline(":SessionRestore"), Ok(CmdAction::SessionRestore));
        assert_eq!(a.parse_cmdline(":loadsession"), Ok(CmdAction::SessionRestore));
    }

    /// Special buffers have nothing durable to point at, so a session must not
    /// try to save them — restoring a dead terminal is not restoration.
    #[test]
    fn a_session_saves_only_file_backed_buffers() {
        let dir = shot_dir();
        let real = dir.join("real.rs");
        std::fs::write(&real, "fn main() {}\n").unwrap();
        let mut a = App::new("fn main() {}".into(), real.clone());
        // Add a special buffer alongside the real one.
        a.apply_cmd(CmdAction::Mason);
        assert!(a.ws.borrow().buffers.ids().len() >= 2, "mason buffer exists");

        let s = a.capture_session().expect("something to save");
        // Canonicalised on capture, so compare canonically — on macOS /var is
        // a symlink to /private/var.
        assert_eq!(
            s.files,
            vec![std::fs::canonicalize(&real).unwrap()],
            "only the file-backed buffer"
        );
        assert!(
            matches!(s.layout, ruster_core::windows::LayoutSnapshot::Leaf { buffer: 0, .. }),
            "{:?}",
            s.layout
        );
    }

    /// A scratch buffer with no path cannot be restored from anywhere.
    #[test]
    fn a_session_with_nothing_on_disk_captures_nothing() {
        let a = App::new("scratch".into(), PathBuf::from(""));
        {
            let id = a.ws.borrow().active_buffer();
            a.ws.borrow_mut().buffers.get_mut(id).unwrap().file_path = None;
        }
        assert!(a.capture_session().is_none(), "nothing worth saving");
    }

    /// The round trip that matters: a split layout, saved and reopened, with the
    /// cursor where it was.
    #[test]
    fn a_session_round_trips_through_disk() {
        let dir = shot_dir();
        let (a_rs, b_rs) = (dir.join("a.rs"), dir.join("b.rs"));
        std::fs::write(&a_rs, "fn a() {}\nline two\n").unwrap();
        std::fs::write(&b_rs, "fn b() {}\n").unwrap();

        let mut app = App::new("fn a() {}\nline two\n".into(), a_rs.clone());
        app.open_path(&b_rs, None);
        app.ws.borrow_mut().windows.split(ruster_core::windows::SplitDir::Vertical);
        let saved = app.capture_session().expect("a session");
        assert_eq!(saved.files.len(), 2);

        // Write and read it back through the real file path.
        ruster_core::session::save(&dir, &dir, &saved).unwrap();
        let loaded = ruster_core::session::load(&dir, &dir).expect("reloaded");
        assert_eq!(loaded, saved, "survives the file format unchanged");
    }

    /// A file deleted since the session was written must not stop the rest
    /// reopening.
    #[test]
    fn restoring_skips_files_that_no_longer_exist() {
        let dir = shot_dir();
        let keep = dir.join("keep.rs");
        std::fs::write(&keep, "fn keep() {}\n").unwrap();
        let session = ruster_core::session::Session {
            files: vec![dir.join("gone.rs"), keep.clone()],
            layout: ruster_core::windows::LayoutSnapshot::Split {
                dir: ruster_core::windows::SplitDir::Vertical,
                ratio: 0.5,
                first: Box::new(ruster_core::windows::LayoutSnapshot::Leaf {
                    buffer: 0,
                    cursor: 0,
                    scroll: 0,
                    active: true,
                }),
                second: Box::new(ruster_core::windows::LayoutSnapshot::Leaf {
                    buffer: 1,
                    cursor: 0,
                    scroll: 0,
                    active: false,
                }),
            },
        };
        let restored = ruster_core::windows::WindowTree::restore(&session.layout, |i| {
            session.files.get(i).filter(|p| p.is_file()).map(|_| BufferId(i as u32 + 1))
        })
        .expect("the surviving file still opens");
        assert_eq!(restored.len(), 1, "the missing file's window collapsed out");
    }

    #[test]
    fn mason_parses_and_opens_a_listing() {
        let mut a = App::new("x".into(), PathBuf::from("f.rs"));
        assert_eq!(a.parse_cmdline(":Mason"), Ok(CmdAction::Mason));
        assert_eq!(a.parse_cmdline(":mason"), Ok(CmdAction::Mason));

        a.apply_cmd(CmdAction::Mason);
        assert!(a.active_is_mason(), "the listing is focused");
        let text = a.ws.borrow().active_doc().buffer.to_string();
        assert!(text.contains("Language servers:"), "{text}");
        assert!(text.contains("rust-analyzer"), "{text}");
        assert!(text.contains("installed."), "{text}");
    }

    /// Nothing may run without the user seeing and confirming the exact command.
    /// `Enter` opens a dialog; it must not start anything by itself.
    #[test]
    fn enter_on_a_tool_only_asks_and_never_installs() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let mut a = App::new("x".into(), PathBuf::from("f.rs"));
        a.apply_cmd(CmdAction::Mason);
        let text = a.ws.borrow().active_doc().buffer.to_string();

        // Pick a tool this machine does *not* have, so the result does not
        // depend on what happens to be installed on the test host.
        let Some(row) = text.lines().position(|l| l.starts_with("  ·")) else {
            return; // every known tool installed — nothing to confirm
        };
        let name = crate::mason::tool_at_row(&crate::mason::builtin_tools(), &text, row)
            .expect("a listed tool")
            .name;
        let off = a.ws.borrow().active_doc().buffer.line_start_char(row);
        a.ws.borrow_mut().windows.active_window_mut().cursors =
            ruster_core::cursor::CursorSet::single(off);

        a.handle_mason_key(CtKey::new(KeyCode::Enter, KeyModifiers::NONE));

        assert!(a.runner_rx.is_none(), "nothing was spawned merely by pressing Enter");
        let d = a.dialog.as_ref().expect("a confirmation dialog is open");
        assert!(d.view().title.contains(&name), "names the tool: {:?}", d.view().title);
        // The exact command is armed and on screen before anyone agrees to it.
        let pending = a.pending_confirm.as_ref().expect("a command is pending").cmd.clone();
        assert!(text.contains(&pending), "the listing already showed it: {pending}");
        assert!(format!("{:?}", d.view()).contains(&pending), "and so does the dialog");
    }

    /// An already-installed tool is a no-op, not a re-install.
    #[test]
    fn enter_on_an_installed_tool_does_nothing() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let mut a = App::new("x".into(), PathBuf::from("f.rs"));
        a.apply_cmd(CmdAction::Mason);
        let text = a.ws.borrow().active_doc().buffer.to_string();
        let Some(row) = text.lines().position(|l| l.starts_with("  ✓")) else {
            return; // nothing installed on this host
        };
        let off = a.ws.borrow().active_doc().buffer.line_start_char(row);
        a.ws.borrow_mut().windows.active_window_mut().cursors =
            ruster_core::cursor::CursorSet::single(off);

        a.handle_mason_key(CtKey::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(a.dialog.is_none(), "no dialog for something already present");
        assert!(a.pending_confirm.is_none());
        assert!(a.runner_rx.is_none());
    }

    /// Declining must discard the command, not merely close the dialog.
    #[test]
    fn cancelling_the_dialog_runs_nothing_and_forgets_the_command() {
        let mut a = App::new("x".into(), PathBuf::from("f.rs"));
        a.pending_confirm = Some(PendingConfirm {
            cmd: "rm -rf /".to_string(),
            kind: RunnerKind::Install,
            verb: "Install".to_string(),
        });
        a.run_pending_confirm(Some("Cancel"));
        assert!(a.runner_rx.is_none(), "nothing spawned");
        assert!(a.pending_confirm.is_none(), "the command is forgotten, not left armed");

        // Dismissing without a button (Esc) is also a refusal.
        a.pending_confirm = Some(PendingConfirm {
            cmd: "rm -rf /".to_string(),
            kind: RunnerKind::Install,
            verb: "Install".to_string(),
        });
        a.run_pending_confirm(None);
        assert!(a.runner_rx.is_none());
        assert!(a.pending_confirm.is_none());
    }

    /// The other half of the gate: confirming really does run it. Uses a
    /// harmless command rather than a registry entry, so the test installs
    /// nothing on the machine running it.
    #[test]
    fn confirming_the_dialog_runs_the_command() {
        let mut a = App::new("x".into(), PathBuf::from("f.rs"));
        a.pending_confirm = Some(PendingConfirm {
            cmd: "echo installed".to_string(),
            kind: RunnerKind::Install,
            verb: "Install".to_string(),
        });
        a.run_pending_confirm(Some("Install"));
        assert!(a.runner_rx.is_some(), "the confirmed command was spawned");
        assert!(a.pending_confirm.is_none(), "and consumed, so it cannot re-run");
        assert!(a.runner_output.starts_with("$ echo installed"), "{}", a.runner_output);
    }

    /// A plugin's dialog and ruster's confirmation share one widget, so the
    /// submit path must not confuse them — a Lua dialog must never be able to
    /// trigger an install.
    #[test]
    fn a_plugin_dialog_does_not_run_an_install() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let mut a = App::new("x".into(), PathBuf::from("f.rs"));
        assert!(a.pending_confirm.is_none());
        a.dialog = Some(crate::dialog::DialogState::new(
            "From Lua",
            vec![crate::dialog::Field::button("OK")],
        ));
        a.handle_dialog_key(CtKey::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(a.runner_rx.is_none(), "a plugin dialog installs nothing");
    }

    #[test]
    fn mason_reports_when_the_cursor_is_not_on_a_tool() {
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let mut a = App::new("x".into(), PathBuf::from("f.rs"));
        a.apply_cmd(CmdAction::Mason);
        a.ws.borrow_mut().windows.active_window_mut().cursors =
            ruster_core::cursor::CursorSet::single(0); // the first heading line
        a.handle_mason_key(CtKey::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(a.dialog.is_none(), "no dialog for a heading");
        assert!(a.runner_rx.is_none());
    }

    #[test]
    fn diffview_parses() {
        let a = App::new("x".into(), PathBuf::from("f.rs"));
        assert_eq!(a.parse_cmdline(":Diffview"), Ok(CmdAction::Diffview));
        assert_eq!(a.parse_cmdline(":diff"), Ok(CmdAction::Diffview));
    }

    /// The two panes must be the same height, or they stop lining up the moment
    /// either is scrolled.
    #[test]
    fn diff_panes_are_the_same_height() {
        let h = ruster_git::DiffHunk { old_start: 1, old_count: 1, new_start: 1, new_count: 4 };
        let rows = ruster_git::align(&[h], 3, 6);
        let old: Vec<&str> = vec!["a", "b", "c"];
        let new: Vec<&str> = vec!["a", "B1", "B2", "B3", "B4", "c"];
        let left = diff_pane_text(&rows, &old, false);
        let right = diff_pane_text(&rows, &new, true);
        assert_eq!(left.lines().count(), right.lines().count());
        assert_eq!(left.lines().count(), rows.len());
    }

    /// Line numbers come from the *file*, not the display row — the whole reason
    /// they are written into the text instead of left to the gutter. After a
    /// hunk that adds lines, the left pane's numbering must skip nothing.
    #[test]
    fn diff_pane_numbers_lines_from_the_file_not_the_screen() {
        let h = ruster_git::DiffHunk { old_start: 1, old_count: 1, new_start: 1, new_count: 4 };
        let rows = ruster_git::align(&[h], 3, 6);
        let old: Vec<&str> = vec!["a", "b", "c"];
        let left: Vec<String> = diff_pane_text(&rows, &old, false).lines().map(str::to_string).collect();

        assert!(left[0].ends_with("│ a"), "{:?}", left[0]);
        assert!(left[0].trim_start().starts_with('1'));
        assert!(left[1].ends_with("│ b"), "{:?}", left[1]);
        // Three filler rows where the new side added lines the old side lacks.
        assert!(left[2].ends_with('~') && left[3].ends_with('~') && left[4].ends_with('~'));
        // And `c` is still line 3 of the old file, not line 6 of the display.
        assert!(left[5].ends_with("│ c"), "{:?}", left[5]);
        assert_eq!(left[5].trim_start().split(' ').next(), Some("3"), "{:?}", left[5]);
    }

    #[test]
    fn diff_pane_right_side_shows_the_working_tree_lines() {
        let h = ruster_git::DiffHunk { old_start: 1, old_count: 1, new_start: 1, new_count: 2 };
        let rows = ruster_git::align(&[h], 2, 3);
        let new: Vec<&str> = vec!["a", "B1", "B2"];
        let right: Vec<String> =
            diff_pane_text(&rows, &new, true).lines().map(str::to_string).collect();
        assert!(right.iter().all(|l| !l.ends_with('~')), "the longer side never pads");
        assert!(right[1].ends_with("│ B1") && right[2].ends_with("│ B2"));
    }

    /// A file with no counterpart on one side still renders — an empty pane
    /// would look like a bug rather than a new file.
    #[test]
    fn a_brand_new_file_renders_filler_on_the_head_side() {
        let rows = ruster_git::align(&[], 0, 2);
        let left = diff_pane_text(&rows, &[], false);
        assert_eq!(left.lines().count(), 2);
        assert!(left.lines().all(|l| l.trim_start().starts_with("│ ~")), "{left:?}");
    }

    /// Regression: the first version synced `scroll_top`, which `render`
    /// recomputes from the cursor — so the follower snapped back every frame and
    /// the panes drifted apart the moment either was scrolled. Syncing the
    /// cursor is what makes the shared clamp land both on the same row.
    #[test]
    fn diff_panes_follow_each_other_by_cursor_line() {
        let mut a = App::new("x".into(), PathBuf::from("f.rs"));
        let text = (1..=20).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let left = a.make_diff_pane("*diff HEAD: t*", &text);
        let right = a.make_diff_pane("*diff working: t*", &text);
        {
            let mut w = a.ws.borrow_mut();
            w.set_active_buffer(left);
            let new_win = w.windows.split(ruster_core::windows::SplitDir::Vertical);
            if let Some(win) = w.windows.window_mut(new_win) {
                win.buffer = right;
            }
        }
        // Put the active pane's cursor on line 12.
        let target = {
            let w = a.ws.borrow();
            let buf = w.windows.active_window().buffer;
            w.buffers.get(buf).unwrap().buffer.line_start_char(12)
        };
        {
            let mut w = a.ws.borrow_mut();
            w.windows.active_window_mut().cursors = ruster_core::cursor::CursorSet::single(target);
        }

        a.sync_diff_scroll();

        let w = a.ws.borrow();
        let lines: Vec<usize> = w
            .windows
            .compute_rects(CoreRect::new(0, 0, 100, 40))
            .into_iter()
            .filter_map(|(id, _)| {
                let win = w.windows.window(id)?;
                let d = w.buffers.get(win.buffer)?;
                a.is_diff_buffer(&w, win.buffer)
                    .then(|| d.buffer.char_to_line(win.cursors.primary().head))
            })
            .collect();
        assert_eq!(lines.len(), 2, "two diff panes");
        assert_eq!(lines[0], lines[1], "both cursors on the same row");
        assert_eq!(lines[0], 12);
    }

    /// A single diff pane with no partner must be left alone rather than
    /// half-synced against whatever else is on screen.
    #[test]
    fn a_lone_diff_pane_is_not_synced() {
        let mut a = App::new("x".into(), PathBuf::from("f.rs"));
        let only = a.make_diff_pane("*diff HEAD: t*", "a\nb\nc");
        {
            let mut w = a.ws.borrow_mut();
            w.set_active_buffer(only);
        }
        a.sync_diff_scroll(); // must not panic, and must change nothing
        let w = a.ws.borrow();
        assert_eq!(w.windows.active_window().cursors.primary().head, 0);
    }

    #[test]
    fn diffview_outside_a_repository_warns_rather_than_opening_panes() {
        let dir = shot_dir();
        let file = dir.join("loose.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();
        let mut a = App::new("fn main() {}".into(), file);
        a.apply_cmd(CmdAction::Diffview);
        // Either "not in a project" or "not a git repository" — both are the
        // refusal path, and neither may leave a diff buffer behind.
        let w = a.ws.borrow();
        let diffs = w.buffers.ids().iter().filter(|&&id| a.is_diff_buffer(&w, id)).count();
        assert_eq!(diffs, 0, "no panes opened");
    }

    /// The guard that took a 10k-line file from ~7 fps to full speed: an
    /// untouched buffer must not be re-parsed, and a touched one must be.
    ///
    /// Counts reparses rather than inspecting the recorded revision — the
    /// record looks identical whether or not the work was skipped, so asserting
    /// on it passes even with the condition mutated to `true`.
    #[test]
    fn an_unchanged_buffer_is_not_reparsed() {
        let mut a = App::new("fn main() {}\n".into(), PathBuf::from("f.rs"));
        a.update_syntax();
        let after_first = a.syntax_reparses;
        assert!(after_first >= 1, "the first pass parses");

        // Nothing changed: no work.
        a.update_syntax();
        a.update_syntax();
        assert_eq!(a.syntax_reparses, after_first, "an untouched buffer is not reparsed");

        // An edit earns exactly one reparse, however many passes follow.
        let buf = a.ws.borrow().active_buffer();
        {
            let mut w = a.ws.borrow_mut();
            w.buffers.get_mut(buf).unwrap().buffer.insert(0, "// x\n");
        }
        a.update_syntax();
        a.update_syntax();
        assert_eq!(a.syntax_reparses, after_first + 1, "one edit, one reparse");
    }

    /// The point of the guard is that highlighting still tracks edits.
    #[test]
    fn highlighting_follows_an_edit() {
        let mut a = App::new("fn a() {}\n".into(), PathBuf::from("f.rs"));
        a.update_syntax();
        let buf = a.ws.borrow().active_buffer();
        let before = a.syntax.get(&buf).map(|e| e.styled_lines().len()).unwrap_or(0);

        {
            let mut w = a.ws.borrow_mut();
            w.buffers.get_mut(buf).unwrap().buffer.insert(0, "fn b() {}\n");
        }
        a.update_syntax();
        let after = a.syntax.get(&buf).map(|e| e.styled_lines().len()).unwrap_or(0);
        assert!(after > before, "the new line is highlighted too ({before} -> {after})");
    }

    #[test]
    fn syntax_reload_parses() {
        let a = App::new("fn main() {}".into(), PathBuf::from("f.rs"));
        assert_eq!(a.parse_cmdline(":SyntaxReload"), Ok(CmdAction::SyntaxReload));
        assert_eq!(a.parse_cmdline(":syntaxreload"), Ok(CmdAction::SyntaxReload));
    }

    /// Queries are read when an engine is built, so a reload has to discard the
    /// engines — keeping them would re-report success while still highlighting
    /// from the old query.
    #[test]
    fn syntax_reload_rebuilds_the_engines() {
        let mut a = App::new("fn main() {}".into(), PathBuf::from("f.rs"));
        assert_eq!(a.syntax.len(), 1, "the initial buffer is highlighted");

        a.apply_cmd(CmdAction::SyntaxReload);
        assert_eq!(a.syntax.len(), 1, "and is highlighted again afterwards");
        assert!(!a.syntax_tried.is_empty(), "the rebuild re-marked it as tried");

        let last = a.notify.history().last().expect("a message was pushed");
        assert!(last.text.contains("Reloaded grammars and queries"), "{:?}", last.text);
    }

    /// A buffer whose engine failed to build must be retried after a reload —
    /// fixing a broken query is the main reason to run one.
    #[test]
    fn syntax_reload_retries_buffers_that_previously_failed() {
        let mut a = App::new("plain text".into(), PathBuf::from("f.unknownext"));
        // No grammar for this extension, so no engine — but it was attempted.
        assert!(a.syntax.is_empty());
        a.update_syntax();
        assert!(!a.syntax_tried.is_empty(), "marked so it is not retried every frame");

        a.apply_cmd(CmdAction::SyntaxReload);
        // The retry happened (the set was cleared and repopulated) even though
        // it failed again, which is what lets a corrected query take effect.
        assert!(a.syntax.is_empty(), "still unsupported");
    }

    static SHOT_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    /// A unique empty temp dir per call, so these stay parallel-safe.
    fn shot_dir() -> PathBuf {
        let id = SHOT_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("ruster_shot_{id}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn a_bare_screenshot_numbers_from_one_in_the_working_directory() {
        let dir = shot_dir();
        assert_eq!(screenshot_path(None, &dir), dir.join("ruster-001.png"));
        // Blank and whitespace-only arguments mean the same as none at all.
        assert_eq!(screenshot_path(Some("   "), &dir), dir.join("ruster-001.png"));
    }

    /// Two screenshots in a row must not silently overwrite the first.
    #[test]
    fn numbering_skips_files_that_already_exist() {
        let dir = shot_dir();
        std::fs::write(dir.join("ruster-001.png"), "x").unwrap();
        std::fs::write(dir.join("ruster-002.png"), "x").unwrap();
        assert_eq!(screenshot_path(None, &dir), dir.join("ruster-003.png"));
    }

    #[test]
    fn a_relative_argument_resolves_against_the_working_directory() {
        let dir = shot_dir();
        assert_eq!(screenshot_path(Some("shot.png"), &dir), dir.join("shot.png"));
        assert_eq!(screenshot_path(Some("sub/shot.png"), &dir), dir.join("sub/shot.png"));
    }

    #[test]
    fn an_absolute_argument_is_left_alone() {
        let dir = shot_dir();
        let abs = dir.join("elsewhere.png");
        assert_eq!(
            screenshot_path(Some(abs.to_str().unwrap()), std::path::Path::new("/nowhere")),
            abs
        );
    }

    /// The backend picks its encoder from the extension, so anything else would
    /// write a file that no viewer can open.
    #[test]
    fn a_non_png_argument_gains_the_extension() {
        let dir = shot_dir();
        assert_eq!(screenshot_path(Some("shot"), &dir), dir.join("shot.png"));
        assert_eq!(screenshot_path(Some("shot.jpg"), &dir), dir.join("shot.jpg.png"));
        // Already a PNG, in any case: left exactly as typed.
        assert_eq!(screenshot_path(Some("shot.PNG"), &dir), dir.join("shot.PNG"));
    }

    /// `:screenshot ~/Pictures` names a folder, not a file to be clobbered.
    #[test]
    fn a_directory_argument_gets_a_numbered_file_inside_it() {
        let dir = shot_dir();
        let sub = dir.join("pics");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(screenshot_path(Some("pics"), &dir), sub.join("ruster-001.png"));
    }

    /// Load `src` into the app's Lua and run one frame of the event pass.
    fn lua_app(src: &str) -> App {
        let mut a = App::new("one\ntwo\nthree\n".into(), PathBuf::from("f.rs"));
        a.lua.lua.load(src).exec().expect("lua loaded");
        // First pass records the baseline without firing; see `fire_watched_events`.
        a.fire_watched_events();
        a
    }

    fn lua_int(a: &App, name: &str) -> i64 {
        a.lua.lua.globals().get::<i64>(name).unwrap_or(-1)
    }

    fn lua_str(a: &App, name: &str) -> String {
        a.lua.lua.globals().get::<String>(name).unwrap_or_default()
    }

    #[test]
    fn moving_the_cursor_fires_cursor_moved_once_per_frame() {
        let mut a = lua_app(
            "n = 0; last = ''
             ruster.on('CursorMoved', function(l, c) n = n + 1; last = l .. ',' .. c end)",
        );
        // Several moves within one frame must still be one event: this is the
        // debounce the plan asked for, and it falls out of diffing per frame
        // rather than firing per keystroke.
        for _ in 0..3 {
            a.ws.borrow_mut().execute(Action::Move(Motion::Line(1)));
        }
        a.fire_watched_events();
        assert_eq!(lua_int(&a, "n"), 1, "three moves in one frame is one event");
        assert_eq!(lua_str(&a, "last"), "4,0", "1-based line, 0-based column");

        // A frame with no movement fires nothing.
        a.fire_watched_events();
        assert_eq!(lua_int(&a, "n"), 1);
    }

    #[test]
    fn the_first_pass_does_not_fire_a_storm() {
        // A plugin loading into an editor that already has a buffer open should
        // not receive BufEnter/CursorMoved for state that predates it —
        // `VimEnter` is what covers startup.
        let mut a = App::new("one\ntwo\n".into(), PathBuf::from("f.rs"));
        a.lua
            .lua
            .load(
                "n = 0
                 ruster.on('CursorMoved', function() n = n + 1 end)
                 ruster.on('BufEnter', function() n = n + 1 end)",
            )
            .exec()
            .unwrap();
        a.fire_watched_events();
        assert_eq!(lua_int(&a, "n"), 0, "the baseline pass fired events");
    }

    #[test]
    fn switching_buffers_fires_leave_then_enter_with_the_right_paths() {
        let dir = std::env::temp_dir().join(format!("ruster_ev_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let other = dir.join("other.rs");
        std::fs::write(&other, "fn other() {}\n").unwrap();

        let mut a = lua_app(
            "log = {}
             ruster.on('BufLeave', function(p) log[#log+1] = 'leave:' .. p end)
             ruster.on('BufEnter', function(p) log[#log+1] = 'enter:' .. p end)",
        );
        a.open_path(&other, None);
        a.fire_watched_events();

        a.lua
            .lua
            .load("first = log[1] or ''; second = log[2] or ''; n = #log")
            .exec()
            .unwrap();
        assert_eq!(lua_int(&a, "n"), 2, "exactly one leave and one enter");
        // Leave must name the buffer being *left*. It fires after the switch has
        // already happened, so the obvious implementation reports the new path
        // for both — and a handler saving per-file state would write it against
        // the wrong file.
        assert!(
            lua_str(&a, "first").starts_with("leave:") && lua_str(&a, "first").ends_with("f.rs"),
            "first event was {:?}",
            lua_str(&a, "first")
        );
        assert!(
            lua_str(&a, "second").starts_with("enter:")
                && lua_str(&a, "second").ends_with("other.rs"),
            "second event was {:?}",
            lua_str(&a, "second")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn entering_and_leaving_insert_fires_both_events() {
        let mut a = lua_app(
            "enter = 0; leave = 0
             ruster.on('InsertEnter', function() enter = enter + 1 end)
             ruster.on('InsertLeave', function() leave = leave + 1 end)",
        );
        use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyModifiers};
        let none = KeyModifiers::NONE;
        a.handle_key(CtKey::new(KeyCode::Char('i'), none));
        assert_eq!(lua_int(&a, "enter"), 1, "InsertEnter");
        assert_eq!(lua_int(&a, "leave"), 0);
        a.handle_key(CtKey::new(KeyCode::Esc, none));
        assert_eq!(lua_int(&a, "enter"), 1);
        assert_eq!(lua_int(&a, "leave"), 1, "InsertLeave");
    }

    #[test]
    fn a_handler_that_errors_does_not_take_the_editor_down() {
        let mut a = lua_app(
            "ok = 0
             ruster.on('CursorMoved', function() error('boom') end)
             ruster.on('CursorMoved', function() ok = ok + 1 end)",
        );
        a.ws.borrow_mut().execute(Action::Move(Motion::Line(1)));
        a.fire_watched_events();
        assert_eq!(lua_int(&a, "ok"), 1, "the second handler still ran");
    }

    #[test]
    fn lua_can_read_the_path_and_filetype_of_the_active_buffer() {
        let a = lua_app("");
        a.lua
            .lua
            .load("p = ruster.api.buf_path(); ft = ruster.api.filetype()")
            .exec()
            .unwrap();
        assert!(lua_str(&a, "p").ends_with("f.rs"), "got {:?}", lua_str(&a, "p"));
        assert_eq!(lua_str(&a, "ft"), "rs");
    }

    #[test]
    fn lua_sees_diagnostics_for_the_active_buffer() {
        let mut a = lua_app("");
        let buf = a.ws.borrow().active_buffer();
        a.lsp.set_diagnostics(
            buf,
            vec![ruster_lsp::Diagnostic {
                start: ruster_lsp::results::LspPositionEq { line: 3, character: 7 },
                end: ruster_lsp::results::LspPositionEq { line: 3, character: 9 },
                severity: 1,
                message: "something is wrong".to_string(),
            }],
        );
        a.fire_watched_events();
        a.lua
            .lua
            .load(
                "d = ruster.api.diagnostics()
                 count = #d
                 line = d[1].line
                 col = d[1].col
                 sev = d[1].severity
                 msg = d[1].message",
            )
            .exec()
            .unwrap();
        assert_eq!(lua_int(&a, "count"), 1);
        assert_eq!(lua_int(&a, "line"), 4, "1-based, matching CursorMoved");
        assert_eq!(lua_int(&a, "col"), 7, "0-based column");
        assert_eq!(lua_int(&a, "sev"), 1);
        assert_eq!(lua_str(&a, "msg"), "something is wrong");
    }

    #[test]
    fn the_introspection_api_degrades_rather_than_erroring() {
        // A plugin that runs before the app finished wiring itself up should
        // get an empty answer, not a Lua error it cannot do anything about.
        let rt = ruster_lua::runtime::LuaRuntime::new().unwrap();
        rt.lua
            .load(
                "p = ruster.api.buf_path()
                 d = #ruster.api.diagnostics()
                 g = ruster.api.git_status().branch",
            )
            .exec()
            .expect("no callbacks installed, but the calls still work");
        assert_eq!(rt.lua.globals().get::<String>("p").unwrap(), "");
        assert_eq!(rt.lua.globals().get::<i64>("d").unwrap(), 0);
        assert_eq!(rt.lua.globals().get::<String>("g").unwrap(), "");
    }

    #[test]
    fn a_bare_line_number_parses_as_a_jump() {
        let a = App::new("content".into(), PathBuf::from("f.txt"));
        assert_eq!(a.parse_cmdline(":16"), Ok(CmdAction::GotoLine(Some(16))));
        assert_eq!(a.parse_cmdline(":1"), Ok(CmdAction::GotoLine(Some(1))));
        assert_eq!(a.parse_cmdline(":0"), Ok(CmdAction::GotoLine(Some(0))));
        assert_eq!(a.parse_cmdline(":$"), Ok(CmdAction::GotoLine(None)));
        // Whitespace around it is still a line number.
        assert_eq!(a.parse_cmdline(": 16 "), Ok(CmdAction::GotoLine(Some(16))));
    }

    #[test]
    fn a_number_does_not_swallow_commands_that_merely_contain_one() {
        // The arm is `all digits`, not `starts with a digit`, or `:2vsplit`
        // and `:w2` would become jumps.
        let a = App::new("content".into(), PathBuf::from("f.txt"));
        assert!(a.parse_cmdline(":16x").is_err(), ":16x is not a line number");
        assert!(a.parse_cmdline(":x16").is_err(), ":x16 is not a line number");
        assert!(a.parse_cmdline(":-4").is_err(), "negatives are not supported");
        assert_eq!(a.parse_cmdline(":w"), Ok(CmdAction::Save(false)), "still a save");
    }

    #[test]
    fn a_line_jump_moves_the_cursor_and_clamps_to_the_buffer() {
        let text = (1..=20).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let mut a = App::new(text, PathBuf::from("f.txt"));

        let line_of = |a: &App| {
            let w = a.ws.borrow();
            let head = w.cursors().primary().head;
            w.buffer().char_to_line(head)
        };

        a.apply_cmd(CmdAction::GotoLine(Some(16)));
        assert_eq!(line_of(&a), 15, ":16 is the 16th line, which is index 15");

        // `:0` and `:1` both mean the first line, as in vim.
        a.apply_cmd(CmdAction::GotoLine(Some(0)));
        assert_eq!(line_of(&a), 0);
        a.apply_cmd(CmdAction::GotoLine(Some(1)));
        assert_eq!(line_of(&a), 0);

        // Past the end clamps rather than erroring — the typist meant the end.
        a.apply_cmd(CmdAction::GotoLine(Some(9999)));
        assert_eq!(line_of(&a), 19, "clamped to the last line");

        a.apply_cmd(CmdAction::GotoLine(Some(5)));
        a.apply_cmd(CmdAction::GotoLine(None));
        assert_eq!(line_of(&a), 19, ":$ is the last line");
    }

    #[test]
    fn a_line_jump_lands_at_the_start_of_the_line() {
        // Not merely on the right line: `:16` in vim puts you at its first
        // character, and anything else makes the next `d`/`y` do the wrong thing.
        let mut a = App::new("aaa\nbbbbbbbb\nccc\n".into(), PathBuf::from("f.txt"));
        a.apply_cmd(CmdAction::GotoLine(Some(2)));
        let w = a.ws.borrow();
        let head = w.cursors().primary().head;
        assert_eq!(head, w.buffer().line_start_char(1), "cursor is at the line start");
    }

    #[test]
    fn screenshot_parses_with_and_without_a_path() {
        let a = App::new("content".into(), PathBuf::from("f.txt"));
        assert_eq!(a.parse_cmdline(":screenshot"), Ok(CmdAction::Screenshot(None)));
        assert_eq!(a.parse_cmdline(":Screenshot"), Ok(CmdAction::Screenshot(None)));
        assert_eq!(
            a.parse_cmdline(":screenshot ~/x.png"),
            Ok(CmdAction::Screenshot(Some("~/x.png".to_string())))
        );
    }

    /// The TUI cannot produce an image, and must say so rather than appear to
    /// have saved one.
    #[test]
    fn screenshot_on_a_backend_without_support_warns() {
        let target = shot_dir().join("unsupported.png");
        let mut a = App::new("content".into(), PathBuf::from("f.txt"));
        a.apply_cmd(CmdAction::Screenshot(Some(target.to_string_lossy().into_owned())));
        let last = a.notify.history().last().expect("a message was pushed");
        assert_eq!(last.level, ruster_core::message::MessageLevel::Warning);
        assert!(last.text.contains("GUI backend"), "{:?}", last.text);
        assert!(!target.exists(), "nothing was written");
    }

    /// A popup notification becomes a titled float in the next frame, so both
    /// backends — which draw whatever `FrameState` carries — render it.
    #[test]
    fn popup_notification_becomes_a_float() {
        let mut a = App::new("content".into(), PathBuf::from("f.txt"));
        a.notify.push_to(
            Notification::new(
                ruster_core::message::MessageLevel::Info,
                ruster_core::message::MessageSource::Echo,
                "pop",
            )
            .with_persistent(),
            BackendKind::Popup,
        );
        let floats = a.notification_floats(80, 24);
        assert_eq!(floats.len(), 1, "one popup → one float");
        assert_eq!(floats[0].title.as_deref(), Some("ruster"), "untitled popup takes the app name");
    }

    /// CmdlinePopup and Popup render identically (the difference is duration),
    /// so a CmdlinePopup notification also lands as a float.
    #[test]
    fn cmdline_popup_notification_becomes_a_float() {
        let mut a = App::new("d".into(), PathBuf::from("f.txt"));
        a.notify.push_to(
            Notification::new(
                ruster_core::message::MessageLevel::Warning,
                ruster_core::message::MessageSource::Echo,
                "bad",
            )
            .with_persistent(),
            BackendKind::CmdlinePopup,
        );
        let floats = a.notification_floats(80, 24);
        assert_eq!(floats.len(), 1);
    }

    /// A Confirm notification raises the modal dialog — the dedicated confirm
    /// surface — instead of a float.
    #[test]
    fn confirm_notification_raises_a_modal_not_a_float() {
        let mut a = App::new("d".into(), PathBuf::from("f.txt"));
        a.notify.push_to(
            Notification::new(
                ruster_core::message::MessageLevel::Info,
                ruster_core::message::MessageSource::Echo,
                "sure?",
            )
            .with_persistent(),
            BackendKind::Confirm,
        );
        assert!(a.dialog.is_none(), "dialog not raised until the floats are built");
        let floats = a.notification_floats(80, 24);
        assert!(floats.is_empty(), "no float for a Confirm");
        assert!(a.dialog.is_some(), "Confirm raises a modal dialog");
    }

    /// `:Noice popup` queues a Popup-kind notification rather than routing by
    /// level, so it shows as a float instead of the mini toast.
    #[test]
    fn noice_popup_queues_a_popup_notification() {
        let mut a = App::new("d".into(), PathBuf::from("f.txt"));
        a.apply_cmd(CmdAction::NoicePopup);
        assert_eq!(a.notify.active(BackendKind::Popup).len(), 1);
        assert_eq!(a.notify.active(BackendKind::Mini).len(), 0, "no level-based toast");
    }
}
