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
use crossterm::event::{KeyCode, KeyModifiers};
use ruster_lua::{config::Config, LuaAction, LuaRuntime};
use ruster_render::{
    CursorKind, FrameState, PickerRow, PickerView, Rect as RRect, Renderer, StatuslineView,
    StyledLine, WindowView,
};
use ruster_syntax::SyntaxEngine;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

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
];

/// The which-key continuations shown after a `Ctrl-w` prefix.
fn which_key_ctrl_w() -> PickerView {
    let entries = [
        ("s", "split horizontal"),
        ("v", "split vertical"),
        ("c", "close window"),
        ("o", "only (close others)"),
        ("h/j/k/l", "focus left/down/up/right"),
        ("z", "toggle fullscreen"),
    ];
    PickerView {
        title: "Ctrl-w".to_string(),
        query: String::new(),
        rows: entries
            .iter()
            .map(|(k, d)| PickerRow { label: format!("{:<8} {}", k, d), selected: false })
            .collect(),
    }
}

/// Parse one `rg --vimgrep` line (`file:line:col:text`) into its parts.
fn parse_rg_line(line: &str) -> Option<(PathBuf, usize, usize, String)> {
    let mut parts = line.splitn(4, ':');
    let file = parts.next()?;
    let ln: usize = parts.next()?.parse().ok()?;
    let col: usize = parts.next()?.parse().ok()?;
    let text = parts.next().unwrap_or("").to_string();
    if file.is_empty() {
        return None;
    }
    Some((PathBuf::from(file), ln, col, text))
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
    /// Syntax engine for `syntax_buffer` (the initially-opened file). Windows
    /// showing other buffers render as plain text for now.
    syntax: Option<SyntaxEngine>,
    syntax_buffer: BufferId,
    lua: LuaRuntime,
    config: Config,
    timer: FrameTimer,
    pub has_smooth_cursor: bool,
    cursor_anim: CursorAnim,
    /// True after a `Ctrl-w` prefix, awaiting a window command key.
    pending_ctrl_w: bool,
    /// Active floating picker (buffer list, file finder, ...), if any.
    picker: Option<PickerState>,
    /// Current directory for each open dired (file-explorer) buffer.
    dired_dirs: std::collections::HashMap<BufferId, PathBuf>,
}

impl App {
    pub fn new(content: String, file_path: PathBuf) -> Self {
        let ws = Rc::new(RefCell::new(Workspace::from_file(file_path.clone(), content.clone())));
        ws.borrow_mut().execute(Action::Move(Motion::To(0)));
        let syntax_buffer = ws.borrow().active_buffer();
        let vim = VimState::new();
        let renderer = Box::new(TuiRenderer::dummy());
        let ext = file_path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        let syntax = SyntaxEngine::new(&content, ext).ok();
        let mut lua = LuaRuntime::new().unwrap_or_else(|e| {
            eprintln!("Lua init failed: {}", e);
            panic!("Lua init required");
        });
        let config_path = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("ruster")
            .join("init.lua");
        if config_path.exists() {
            if let Err(e) = lua.load_init(&config_path) {
                eprintln!("Lua config: {}", e);
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

        lua.fire_event("VimEnter", &[]);
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
            should_quit: false, message: None, syntax, syntax_buffer, lua, config, timer,
            has_smooth_cursor: false, cursor_anim, pending_ctrl_w: false, picker: None,
            dired_dirs: std::collections::HashMap::new(),
        }
    }

    pub fn handle_key(&mut self, ck: crossterm::event::KeyEvent) {
        // An open picker captures all input until it is accepted or cancelled.
        if self.picker.is_some() {
            self.handle_picker_key(ck);
            return;
        }

        // Dired buffers intercept navigation keys (movement falls through).
        if self.active_is_dired() && self.handle_dired_key(ck) {
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

        let prev_mode = self.vim.mode;
        let mode = match prev_mode {
            VimMode::Normal => "n",
            VimMode::Insert => "i",
            VimMode::VisualChar | VimMode::VisualLine => "v",
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
            if self.config.expandtab {
                let spaces = " ".repeat(self.config.tabstop as usize);
                let mut w = self.ws.borrow_mut();
                w.execute(Action::BeginBatch);
                w.execute(Action::Edit(EditOp::InsertString(spaces)));
                w.execute(Action::EndBatch);
            }
            return;
        }

        let actions = self.vim.handle(key, &*self.ws.borrow());
        for action in actions {
            match action {
                Action::Textobject { op, kind, target, count: _ } => {
                    let cursor = self.ws.borrow().primary_head();
                    if let Some((start, end)) = self.syntax.as_ref()
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

    pub fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
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

        crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen)?;
        crossterm::terminal::disable_raw_mode()?;
        Ok(())
    }

    pub fn run_async(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        crossterm::terminal::enable_raw_mode()?;
        crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen)?;
        self.renderer = Box::new(TuiRenderer::new()?);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()?;

        let result = rt.block_on(self.async_run());

        crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen)?;
        crossterm::terminal::disable_raw_mode()?;
        result
    }

    async fn async_run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        // Spawn blocking reader
        let tx_reader = tx.clone();
        tokio::task::spawn_blocking(move || {
            loop {
                match crossterm::event::read() {
                    Ok(ev) => {
                        if tx_reader.send(AppEvent::Input(ev)).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
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
                            match ev {
                                crossterm::event::Event::Key(k) => self.handle_key(k),
                                _ => {}
                            }
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
    }

    fn render(&mut self) {
        let (cols, rows) = self.renderer.viewport_cells();
        // Reserve the bottom row for the shared cmdline/message line.
        let buf_area = CoreRect::new(0, 0, cols, rows.saturating_sub(1));

        // Reparse syntax for the tracked buffer, then snapshot its styled lines.
        let syntax_content = self.ws.borrow().buffers
            .get(self.syntax_buffer)
            .map(|d| d.buffer.to_string());
        if let (Some(c), Some(syn)) = (syntax_content.as_ref(), self.syntax.as_mut()) {
            syn.reparse(c);
        }
        let styled: Option<Vec<StyledLine>> = self.syntax.as_ref().map(|s| s.styled_lines().to_vec());

        let mode = self.vim.mode;
        let mode_lbl = crate::widgets::mode_label(&mode).to_string();
        let cursor_kind = match mode {
            VimMode::Insert | VimMode::Cmdline => CursorKind::Bar,
            _ => CursorKind::Block,
        };
        let smooth = self.has_smooth_cursor;
        let (anim_x, anim_y) = (self.cursor_anim.cell_x, self.cursor_anim.cell_y);

        // Lua-registered statusline sections (global; shown on the active window).
        let lua_left = self.lua.statusline_sections("left").join("  ");
        let lua_center = self.lua.statusline_sections("center").join("  ");
        let lua_right = self.lua.statusline_sections("right").join("  ");

        let mut views: Vec<WindowView> = Vec::new();
        {
            let mut w = self.ws.borrow_mut();
            let active_id = w.windows.active();
            let rects = w.windows.compute_rects(buf_area);
            for (wid, rect) in rects {
                let is_active = wid == active_id;
                let (buf_id, head, mut scroll) = {
                    let win = w.windows.window(wid).expect("window exists");
                    (win.buffer, win.cursors.head(), win.scroll_top)
                };
                let (content, cline, ccol, name, line_count) = {
                    let doc = w.buffers.get(buf_id).expect("buffer exists");
                    let cline = doc.buffer.char_to_line(head);
                    let ccol = head - doc.buffer.line_start_char(cline);
                    (doc.buffer.to_string(), cline, ccol, doc.name.clone(), doc.buffer.line_count())
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
                }

                let lines: Vec<StyledLine> = if buf_id == self.syntax_buffer {
                    styled.clone().unwrap_or_else(|| plain_lines(&content))
                } else {
                    plain_lines(&content)
                };
                let pct = if line_count > 0 {
                    (cline + 1) * 100 / line_count
                } else {
                    100
                };
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
                    cursor_kind,
                    cursor_visible: is_active,
                    cursor_smooth,
                    scroll_offset: scroll as u16,
                    gutter,
                    statusline,
                    active: is_active,
                });
            }
        }

        let cmdline = match mode {
            VimMode::Cmdline => Some(crate::widgets::cmdline_label(self.vim.cmdline_buffer())),
            _ => self.message.clone(),
        };
        // Which-key panel takes over the overlay while a Ctrl-w prefix is pending.
        let picker_view = if self.pending_ctrl_w {
            Some(which_key_ctrl_w())
        } else {
            self.picker.as_mut().map(|p| p.view())
        };
        let state = FrameState {
            windows: views,
            cmdline: cmdline.as_deref(),
            message: None,
            picker: picker_view,
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
            _ if trimmed.starts_with("w ") || trimmed.starts_with("write ") => {
                let path = trimmed.splitn(2, ' ').nth(1).unwrap_or("").trim().to_string();
                if path.is_empty() {
                    Err("No path given".to_string())
                } else {
                    Ok(CmdAction::SaveAs(path))
                }
            }
            _ if trimmed.starts_with("Dired ") || trimmed.starts_with("dired ") => {
                let path = trimmed.splitn(2, ' ').nth(1).unwrap_or("").trim().to_string();
                Ok(CmdAction::Dired(Some(path)))
            }
            _ if trimmed.starts_with("Rg ") || trimmed.starts_with("rg ") => {
                let pat = trimmed.splitn(2, ' ').nth(1).unwrap_or("").trim().to_string();
                if pat.is_empty() {
                    Err("No pattern given".to_string())
                } else {
                    Ok(CmdAction::Rg(pat))
                }
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
        }
    }

    /// Open a fuzzy file picker over the project (gitignore-aware walk).
    fn open_files_picker(&mut self) {
        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let mut items: Vec<PickerItem> = Vec::new();
        for result in ignore::WalkBuilder::new(&root).build() {
            let entry = match result {
                Ok(e) => e,
                Err(_) => continue,
            };
            if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                let path = entry.path().to_path_buf();
                let label = path
                    .strip_prefix(&root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                items.push(PickerItem::new(label, PickerAction::OpenPath(path)));
            }
        }
        self.picker = Some(PickerState::new("Files", items));
    }

    /// Run `rg --vimgrep <pattern>` and show matches in a picker. Reports a
    /// clear message when ripgrep is not installed.
    fn run_rg(&mut self, pattern: &str) {
        let output = std::process::Command::new("rg")
            .arg("--vimgrep")
            .arg(pattern)
            .output();
        match output {
            Ok(out) => {
                let text = String::from_utf8_lossy(&out.stdout);
                let items: Vec<PickerItem> = text
                    .lines()
                    .filter_map(parse_rg_line)
                    .map(|(path, line, col, body)| {
                        PickerItem::new(
                            format!("{}:{}:{}: {}", path.display(), line, col, body),
                            PickerAction::OpenLocation(path, line, col),
                        )
                    })
                    .collect();
                if items.is_empty() {
                    self.message = Some(format!("No matches for '{}'", pattern));
                } else {
                    self.picker = Some(PickerState::new(format!("Rg: {}", pattern), items));
                }
            }
            Err(_) => {
                self.message = Some("ripgrep (rg) not found in PATH".to_string());
            }
        }
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
        let text = ruster_core::dired::render(&path);
        {
            let mut w = self.ws.borrow_mut();
            if let Some(doc) = w.buffers.get_mut(id) {
                doc.buffer = Buffer::from_str(&text);
                doc.name = path.to_string_lossy().into_owned();
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
        match ck.code {
            KeyCode::Enter => {
                self.dired_open_at_cursor();
                true
            }
            KeyCode::Char('-') | KeyCode::Char('^') => {
                self.dired_go_up();
                true
            }
            // Movement keys pass through to vim for navigation.
            KeyCode::Char('j') | KeyCode::Char('k') | KeyCode::Char('g')
            | KeyCode::Char('G') | KeyCode::Up | KeyCode::Down => false,
            // Swallow everything else to keep the listing read-only.
            _ => true,
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
        let entries = ruster_core::dired::list(&dir);
        let entry = match entries.get(line) {
            Some(e) => e.clone(),
            None => return,
        };
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
        if let Some(dir) = self.dired_dirs.get(&id) {
            if let Some(parent) = dir.parent() {
                let parent = parent.to_path_buf();
                self.refresh_dired(id, parent);
            }
        }
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
        let (path, content) = {
            let w = self.ws.borrow();
            let doc = w.active_doc();
            (doc.file_path.clone(), doc.buffer.to_string())
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
        let content = self.ws.borrow().active_doc().buffer.to_string();
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
    fn command_palette_opens_with_seed_and_filters() {
        let mut a = App::new("x".into(), PathBuf::from("f.txt"));
        a.open_command_picker("wq");
        let p = a.picker.as_mut().expect("palette open");
        assert_eq!(p.filter, "wq");
        assert!(!p.filtered().is_empty(), "seed matches at least one command");
    }

    #[test]
    fn which_key_lists_ctrl_w_continuations() {
        let v = which_key_ctrl_w();
        assert_eq!(v.title, "Ctrl-w");
        assert!(v.rows.iter().any(|r| r.label.contains("split")));
        assert!(v.rows.iter().any(|r| r.label.contains("fullscreen")));
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
