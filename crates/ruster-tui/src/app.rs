use crate::key::crossterm_to_ruster_key;
use crate::renderer::TuiRenderer;
use ruster_core::action::{Action, EditOp, Motion};
use ruster_core::editor::Editor;
use ruster_core::key::KeyEvent;
use ruster_core::vim::VimMode;
use ruster_core::vim::VimState;
use ruster_lua::{config::Config, LuaAction, LuaRuntime};
use ruster_render::{CursorKind, EditorState, Renderer, StyledLine};
use ruster_syntax::SyntaxEngine;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

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
}

enum AppEvent {
    Input(crossterm::event::Event),
}

pub struct App {
    pub editor: Rc<RefCell<Editor>>,
    pub vim: VimState,
    pub renderer: Box<dyn Renderer>,
    file_path: PathBuf,
    pub should_quit: bool,
    message: Option<String>,
    syntax: Option<SyntaxEngine>,
    lua: LuaRuntime,
    config: Config,
    timer: FrameTimer,
    pub has_smooth_cursor: bool,
    cursor_anim: CursorAnim,
}

impl App {
    pub fn new(content: String, file_path: PathBuf) -> Self {
        let editor = Rc::new(RefCell::new(Editor::from_str(&content)));
        editor.borrow_mut().execute(Action::Move(Motion::To(0)));
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

        // Wire buffer callbacks
        let ed_get = editor.clone();
        let ed_set = editor.clone();
        let ed_get_cursor = editor.clone();
        let ed_set_cursor = editor.clone();
        lua.set_buffer_callbacks(
            Box::new(move |start, end_opt| {
                let b = ed_get.borrow();
                let buf = b.buffer();
                let count = buf.line_count() as i32;
                let end = end_opt.unwrap_or_else(|| start + 1);
                let end = if end == -1 { count } else { end.min(count) };
                (start..end).map(|i| buf.line_to_string(i as usize)).collect()
            }),
            Box::new(move |start, end, lines_vec| {
                let line_count = {
                    let b = ed_set.borrow();
                    b.buffer().line_count()
                };
                let end = (end as usize).min(line_count.saturating_sub(1));
                let (char_start, char_end) = {
                    let b = ed_set.borrow();
                    let buf = b.buffer();
                    let cs = buf.line_start_char(start as usize);
                    let ce = if end + 1 >= line_count { buf.len_chars() }
                             else { buf.line_start_char(end + 1) };
                    (cs, ce)
                };
                let mut b = ed_set.borrow_mut();
                b.execute(Action::BeginBatch);
                b.execute(Action::Edit(EditOp::DeleteRange(char_start, char_end)));
                let text = lines_vec.join("\n");
                if !text.is_empty() {
                    b.execute(Action::Edit(EditOp::InsertString(text)));
                }
                b.execute(Action::EndBatch);
            }),
            Box::new(move || {
                let b = ed_get_cursor.borrow();
                let head = b.primary_head();
                let row = b.char_to_line(head);
                let col = head - b.buffer().line_start_char(row);
                (row as i32, col as i32)
            }),
            Box::new(move |row, col| {
                let mut b = ed_set_cursor.borrow_mut();
                let pos = b.buffer().line_start_char(row as usize) + col as usize;
                b.execute(Action::Move(Motion::To(pos)));
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
        editor.borrow_mut().set_config_indent(config.tabstop);
        let timer = FrameTimer::new();
        let cursor_anim = CursorAnim::new();
        App {
            editor, vim, renderer, file_path,
            should_quit: false, message: None, syntax, lua, config, timer,
            has_smooth_cursor: false, cursor_anim
        }
    }

    pub fn handle_key(&mut self, ck: crossterm::event::KeyEvent) {
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

        if self.vim.mode == VimMode::Insert && key == KeyEvent::Tab {
            if self.config.expandtab {
                let spaces = " ".repeat(self.config.tabstop as usize);
                self.editor.borrow_mut().execute(Action::BeginBatch);
                self.editor.borrow_mut().execute(Action::Edit(EditOp::InsertString(spaces)));
                self.editor.borrow_mut().execute(Action::EndBatch);
            }
            return;
        }

        let actions = self.vim.handle(key, &*self.editor.borrow());
        for action in actions {
            match action {
                Action::Textobject { op, kind, target, count: _ } => {
                    let cursor = self.editor.borrow().primary_head();
                    if let Some((start, end)) = self.syntax.as_ref()
                        .and_then(|s| s.ts_textobject(kind, target, cursor))
                    {
                        self.exec_operator(op, start, end);
                    }
                }
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
                other => self.editor.borrow_mut().execute(other),
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
        let content = self.editor.borrow().buffer().to_string();
        if let Some(syn) = &mut self.syntax {
            syn.reparse(&content);
        }
        let styled_lines: Vec<StyledLine> = match &self.syntax {
            Some(syn) => syn.styled_lines().to_vec(),
            None => content.split('\n').map(|s| StyledLine { text: s.to_string(), highlights: vec![] }).collect(),
        };

        let (line, col) = self.cursor_line_col();

        let cursor_smooth = if self.has_smooth_cursor {
            Some((self.cursor_anim.cell_x - col as f32, self.cursor_anim.cell_y - line as f32))
        } else {
            None
        };

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
            cursor_visible: true,
            cursor_smooth,
            mode_label,
            file_path: &file_path,
            modified: false,
            cmdline: cmdline.as_deref(),
            message: None,
            scroll_offset: 0,
        };
        self.renderer.render_frame(&state);
    }

    fn cursor_line_col(&self) -> (u16, u16) {
        let editor = self.editor.borrow();
        let head = editor.primary_head();
        let buf = editor.buffer();
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
            _ if trimmed.starts_with("w ") || trimmed.starts_with("write ") => {
                let path = trimmed.splitn(2, ' ').nth(1).unwrap_or("").trim().to_string();
                if path.is_empty() {
                    Err("No path given".to_string())
                } else {
                    Ok(CmdAction::SaveAs(path))
                }
            }
            _ => Err(format!("Unknown command: {}", cmdline)),
        }
    }

    fn save_file(&mut self, force: bool) {
        self.lua.fire_event_str("BufWritePre", &[self.file_path.to_str().unwrap_or("")]);
        let content = self.editor.borrow().buffer().to_string();
        match std::fs::write(&self.file_path, &content) {
            Ok(()) => self.message = Some(format!("Saved: {}", self.file_path.display())),
            Err(_e) if force => {
                let _ = std::fs::write(&self.file_path, &content);
                self.message = Some(format!("Saved (forced): {}", self.file_path.display()));
            }
            Err(e) => self.message = Some(format!("Error: {}", e)),
        }
        self.lua.fire_event_str("BufWritePost", &[self.file_path.to_str().unwrap_or("")]);
    }

    fn exec_operator(&mut self, op: char, start: usize, end: usize) {
        let safe_end = end.min({
            let b = self.editor.borrow();
            b.buffer().len_chars()
        });
        match op {
            'd' => {
                let mut b = self.editor.borrow_mut();
                b.execute(Action::BeginBatch);
                b.execute(Action::Edit(EditOp::DeleteRange(start, safe_end)));
                b.execute(Action::EndBatch);
            }
            'c' => {
                {
                    let mut b = self.editor.borrow_mut();
                    b.execute(Action::BeginBatch);
                    b.execute(Action::Edit(EditOp::DeleteRange(start, safe_end)));
                }
                self.vim.mode = VimMode::Insert;
            }
            'y' => {
                let text = self.editor.borrow().buffer().slice_string(start, safe_end);
                self.vim.set_register(text);
            }
            _ => {}
        }
    }

    fn save_as(&mut self, path: &str) {
        let content = self.editor.borrow().buffer().to_string();
        match std::fs::write(path, &content) {
            Ok(()) => {
                self.file_path = PathBuf::from(path);
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
