use crate::key::crossterm_to_ruster_key;
use crate::renderer::TuiRenderer;
use ruster_core::action::{Action, EditOp, Motion};
use ruster_core::editor::Editor;
use ruster_core::vim::VimMode;
use ruster_core::vim::VimState;
use ruster_render::{CursorKind, EditorState, Renderer, StyledLine};
use ruster_syntax::SyntaxEngine;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
enum CmdAction {
    Save(bool),
    SaveAs(String),
    Quit,
    ForceQuit,
    SaveAndQuit,
}

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
        let ext = file_path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        let syntax = SyntaxEngine::new(&content, ext).ok();
        App { editor, vim, renderer, file_path, should_quit: false, message: None, syntax }
    }

    pub fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        crossterm::terminal::enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen)?;

        self.renderer = TuiRenderer::new()?;

        loop {
            self.render();
            if self.should_quit { break; }

            let ev = crossterm::event::read()?;
            let ck = match ev {
                crossterm::event::Event::Key(k) => k,
                _ => continue,
            };
            let key = crossterm_to_ruster_key(ck);
            for action in self.vim.handle(key, &self.editor) {
                match action {
                    Action::Textobject { op, kind, target, count: _ } => {
                        let cursor = self.editor.primary_head();
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
                    other => self.editor.execute(other),
                }
            }
        }

        crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen)?;
        crossterm::terminal::disable_raw_mode()?;
        Ok(())
    }

    fn render(&mut self) {
        let content = self.editor.buffer().to_string();
        if let Some(syn) = &mut self.syntax {
            syn.reparse(&content);
        }
        let styled_lines: Vec<StyledLine> = match &self.syntax {
            Some(syn) => syn.styled_lines().to_vec(),
            None => content.split('\n').map(|s| StyledLine { text: s.to_string(), highlights: vec![] }).collect(),
        };

        let head = self.editor.primary_head();
        let mut line = 0u16;
        let mut col = 0u16;
        let mut remaining = head;
        for l in &styled_lines {
            let lc = l.text.chars().count();
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
            cursor_visible: true,
            mode_label,
            file_path: &file_path,
            modified: false,
            cmdline: cmdline.as_deref(),
            message: None,
        };
        self.renderer.render_frame(&state);
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
        let content = self.editor.buffer().to_string();
        match std::fs::write(&self.file_path, &content) {
            Ok(()) => self.message = Some(format!("Saved: {}", self.file_path.display())),
            Err(_e) if force => {
                let _ = std::fs::write(&self.file_path, &content);
                self.message = Some(format!("Saved (forced): {}", self.file_path.display()));
            }
            Err(e) => self.message = Some(format!("Error: {}", e)),
        }
    }

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

    fn save_as(&mut self, path: &str) {
        let content = self.editor.buffer().to_string();
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
}
