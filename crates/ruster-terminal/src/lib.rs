//! Embedded terminal core for ruster (Phase 4).
//!
//! Wraps [`portable-pty`] (the PTY: ConPTY on Windows, `forkpty` on Unix) and
//! [`alacritty_terminal`] (the VT100/ANSI state machine) behind a small,
//! render-neutral API. A [`TerminalSession`] spawns a child process in a PTY,
//! runs a blocking reader thread that pumps the process's output into an
//! `alacritty_terminal::Term` grid, and hands the UI a cheap [`TermGrid`]
//! snapshot each frame. Nothing here depends on ruster's UI or core crates, so
//! the ConPTY/forkpty split lives entirely inside `portable-pty`.

use std::io::Read;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor, Processor};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

mod keys;
pub use keys::{encode_key, Key, Mods};

/// Result type for terminal operations; errors are human-readable strings.
pub type Result<T> = std::result::Result<T, String>;

/// A resolved terminal color. `Default` means "use the renderer's theme
/// default" (i.e. the ANSI foreground/background), so the frontend decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermColor {
    Default,
    Rgb(u8, u8, u8),
}

/// Rendering attributes for a cell, distilled from alacritty's cell flags.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TermAttrs {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
}

/// One rendered grid cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermCell {
    pub c: char,
    pub fg: TermColor,
    pub bg: TermColor,
    pub attrs: TermAttrs,
}

impl Default for TermCell {
    fn default() -> Self {
        TermCell { c: ' ', fg: TermColor::Default, bg: TermColor::Default, attrs: TermAttrs::default() }
    }
}

/// An immutable snapshot of the visible terminal grid, cheap to build each
/// frame. Cells are stored row-major (`rows * cols`).
#[derive(Debug, Clone)]
pub struct TermGrid {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<TermCell>,
    /// Cursor position as `(row, col)` within the visible grid.
    pub cursor: (usize, usize),
}

impl TermGrid {
    /// The plain text of one row (trailing blanks included). Handy in tests.
    pub fn row_text(&self, row: usize) -> String {
        if row >= self.rows {
            return String::new();
        }
        (0..self.cols).map(|c| self.cells[row * self.cols + c].c).collect()
    }
}

/// Terminal dimensions handed to `alacritty_terminal`. History is managed by
/// the grid up to `Config::scrolling_history`, so `total_lines == screen_lines`.
#[derive(Clone, Copy)]
struct TermSize {
    cols: usize,
    screen_lines: usize,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }
    fn screen_lines(&self) -> usize {
        self.screen_lines
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// A no-op event proxy — we don't yet surface bell/title/clipboard events.
#[derive(Clone)]
struct EventProxy;
impl EventListener for EventProxy {}

/// A running terminal: a PTY + child process + the parsed grid.
pub struct TerminalSession {
    term: Arc<Mutex<Term<EventProxy>>>,
    master: Box<dyn MasterPty + Send>,
    writer: Mutex<Box<dyn std::io::Write + Send>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
    reader: Option<JoinHandle<()>>,
    cols: u16,
    rows: u16,
}

impl TerminalSession {
    /// Spawn `program` with `args` in a fresh PTY of `cols`×`rows`, retaining
    /// `scrollback` lines of history, and start pumping its output into the grid
    /// on a background thread.
    pub fn spawn(
        program: &str,
        args: &[String],
        cols: u16,
        rows: u16,
        scrollback: usize,
    ) -> Result<Self> {
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| format!("openpty failed: {e}"))?;

        let mut cmd = CommandBuilder::new(program);
        for a in args {
            cmd.arg(a);
        }
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("failed to spawn {program}: {e}"))?;
        // The slave handle is no longer needed once the child owns it; dropping
        // it lets the PTY report EOF cleanly when the child exits.
        drop(pair.slave);

        let size = TermSize { cols: cols as usize, screen_lines: rows as usize };
        let config = Config { scrolling_history: scrollback, ..Config::default() };
        let term = Arc::new(Mutex::new(Term::new(config, &size, EventProxy)));

        let mut read_src = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("failed to clone PTY reader: {e}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("failed to take PTY writer: {e}"))?;

        let reader_term = Arc::clone(&term);
        let reader = std::thread::spawn(move || {
            let mut parser: Processor = Processor::new();
            let mut buf = [0u8; 8192];
            loop {
                match read_src.read(&mut buf) {
                    Ok(0) | Err(_) => break, // EOF or error → child gone
                    Ok(n) => {
                        if let Ok(mut t) = reader_term.lock() {
                            parser.advance(&mut *t, &buf[..n]);
                        }
                    }
                }
            }
        });

        Ok(TerminalSession {
            term,
            master: pair.master,
            writer: Mutex::new(writer),
            child: Mutex::new(child),
            reader: Some(reader),
            cols,
            rows,
        })
    }

    /// Write raw bytes (already-encoded input) to the child.
    pub fn write_input(&self, bytes: &[u8]) -> Result<()> {
        use std::io::Write;
        let mut w = self.writer.lock().map_err(|_| "writer poisoned".to_string())?;
        w.write_all(bytes).map_err(|e| e.to_string())?;
        w.flush().map_err(|e| e.to_string())
    }

    /// Resize the PTY and the grid to `cols`×`rows`.
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        if cols == 0 || rows == 0 || (cols == self.cols && rows == self.rows) {
            return Ok(());
        }
        self.master
            .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| format!("PTY resize failed: {e}"))?;
        if let Ok(mut t) = self.term.lock() {
            t.resize(TermSize { cols: cols as usize, screen_lines: rows as usize });
        }
        self.cols = cols;
        self.rows = rows;
        Ok(())
    }

    /// Whether the child process is still alive.
    pub fn is_running(&self) -> bool {
        match self.child.lock() {
            Ok(mut c) => matches!(c.try_wait(), Ok(None)),
            Err(_) => false,
        }
    }

    /// Build an immutable snapshot of the visible grid for rendering.
    pub fn snapshot(&self) -> TermGrid {
        let term = match self.term.lock() {
            Ok(t) => t,
            Err(_) => {
                return TermGrid {
                    cols: self.cols as usize,
                    rows: self.rows as usize,
                    cells: vec![TermCell::default(); self.cols as usize * self.rows as usize],
                    cursor: (0, 0),
                }
            }
        };
        let grid = term.grid();
        let cols = grid.columns();
        let rows = grid.screen_lines();
        let mut cells = Vec::with_capacity(cols * rows);
        for l in 0..rows {
            let row = &grid[Line(l as i32)];
            for c in 0..cols {
                cells.push(convert_cell(&row[Column(c)]));
            }
        }
        let point = grid.cursor.point;
        let cursor = (point.line.0.max(0) as usize, point.column.0.min(cols.saturating_sub(1)));
        TermGrid { cols, rows, cells, cursor }
    }

    /// Every retained line — scrollback history followed by the visible screen —
    /// as plain text, with the row the cursor sits on.
    ///
    /// [`Self::snapshot`] deliberately returns only the viewport, because that
    /// is what the renderer draws. Terminal-Normal wants the other thing: the
    /// grid keeps `terminal.scrollback` lines of history and nothing could
    /// reach them, so the setting promised a scrollback the editor could not
    /// show and output that scrolled off was gone for good.
    ///
    /// History lines are addressed with negative indices, `topmost_line()`
    /// being `-history_size`.
    pub fn scrollback_text(&self) -> (Vec<String>, usize) {
        let Ok(term) = self.term.lock() else {
            return (Vec::new(), 0);
        };
        let grid = term.grid();
        let cols = grid.columns();
        let top = grid.topmost_line().0;
        let bottom = grid.bottommost_line().0;

        let mut lines = Vec::with_capacity((bottom - top + 1).max(0) as usize);
        for l in top..=bottom {
            let row = &grid[Line(l)];
            let text: String = (0..cols).map(|c| row[Column(c)].c).collect();
            lines.push(text.trim_end().to_string());
        }
        // Rebase the cursor from grid coordinates (where 0 is the first visible
        // row) onto the returned vector (where 0 is the oldest history line).
        let cursor = (grid.cursor.point.line.0 - top).max(0) as usize;
        (lines, cursor)
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        // Kill the child so the PTY reader hits EOF on its next read.
        if let Ok(mut c) = self.child.lock() {
            let _ = c.kill();
        }
        // Detach — do NOT join — the reader thread. On Windows (ConPTY) `read()`
        // does not always return promptly after the child is killed, so joining
        // could block indefinitely (it hung CI for hours). Dropping the handle
        // detaches the thread; it exits on its own when the PTY closes, and a
        // detached thread never blocks process exit.
        self.reader.take();
    }
}

/// The default login shell and its args for the current platform.
pub fn default_shell() -> (String, Vec<String>) {
    #[cfg(windows)]
    {
        let shell = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string());
        (shell, Vec::new())
    }
    #[cfg(not(windows))]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        (shell, Vec::new())
    }
}

fn convert_cell(cell: &Cell) -> TermCell {
    TermCell {
        c: if cell.c == '\0' { ' ' } else { cell.c },
        fg: ansi_to_term_color(cell.fg),
        bg: ansi_to_term_color(cell.bg),
        attrs: TermAttrs {
            bold: cell.flags.contains(Flags::BOLD),
            italic: cell.flags.contains(Flags::ITALIC),
            underline: cell.flags.contains(Flags::UNDERLINE),
            inverse: cell.flags.contains(Flags::INVERSE),
        },
    }
}

/// Standard xterm 16-color palette.
const ANSI16: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (205, 0, 0),
    (0, 205, 0),
    (205, 205, 0),
    (0, 0, 238),
    (205, 0, 205),
    (0, 205, 205),
    (229, 229, 229),
    (127, 127, 127),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (92, 92, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

fn ansi_to_term_color(c: AnsiColor) -> TermColor {
    match c {
        AnsiColor::Spec(rgb) => TermColor::Rgb(rgb.r, rgb.g, rgb.b),
        AnsiColor::Indexed(i) => indexed_to_rgb(i),
        AnsiColor::Named(n) => {
            let idx = n as usize;
            if idx < 16 {
                let (r, g, b) = ANSI16[idx];
                TermColor::Rgb(r, g, b)
            } else {
                // Foreground/Background/Cursor and dim/bright fg → theme default.
                let _ = NamedColor::Foreground;
                TermColor::Default
            }
        }
    }
}

/// Map an xterm 256-color index to RGB.
fn indexed_to_rgb(i: u8) -> TermColor {
    let i = i as usize;
    if i < 16 {
        let (r, g, b) = ANSI16[i];
        TermColor::Rgb(r, g, b)
    } else if i < 232 {
        let n = i - 16;
        let step = |x: usize| -> u8 {
            if x == 0 {
                0
            } else {
                (55 + 40 * x) as u8
            }
        };
        TermColor::Rgb(step((n / 36) % 6), step((n / 6) % 6), step(n % 6))
    } else {
        let gray = (8 + (i - 232) * 10) as u8;
        TermColor::Rgb(gray, gray, gray)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spawn a deterministic command and wait until its output lands in the grid.
    #[cfg(not(windows))]
    fn spawn_and_wait(program: &str, args: &[String], needle: &str) -> TermGrid {
        let session = TerminalSession::spawn(program, args, 40, 6, 1000).expect("spawn");
        let mut grid = session.snapshot();
        for _ in 0..200 {
            grid = session.snapshot();
            if (0..grid.rows).any(|r| grid.row_text(r).contains(needle)) {
                return grid;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        grid
    }

    /// Output that has scrolled past the top of the screen must still be
    /// reachable. The grid keeps `scrollback` lines of it; before
    /// `scrollback_text` nothing could read them, so the setting promised a
    /// history the editor had no way to show.
    #[cfg(not(windows))]
    #[test]
    fn scrollback_reaches_output_that_has_scrolled_off_the_screen() {
        // 200 lines through a 6-row window: all but the last few have scrolled.
        let session = TerminalSession::spawn(
            "sh",
            &["-c".to_string(), "seq 1 200".to_string()],
            40,
            6,
            1000,
        )
        .expect("spawn");

        let mut lines = Vec::new();
        for _ in 0..300 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            lines = session.scrollback_text().0;
            if lines.iter().any(|l| l.trim() == "200") {
                break;
            }
        }

        let visible = session.snapshot();
        assert!(
            (0..visible.rows).all(|r| visible.row_text(r).trim() != "1"),
            "line 1 should have scrolled off the 6-row screen"
        );
        assert!(
            lines.iter().any(|l| l.trim() == "1"),
            "but scrollback should still hold it; got {} lines",
            lines.len()
        );
        assert!(lines.iter().any(|l| l.trim() == "200"), "and the newest line too");
        assert!(
            lines.len() > visible.rows,
            "scrollback ({}) must exceed the {} visible rows",
            lines.len(),
            visible.rows
        );
    }

    /// The cursor is reported in the returned vector's coordinates, not the
    /// grid's — history lines are addressed negatively, so a raw grid row index
    /// would point at the wrong line as soon as anything scrolled.
    #[cfg(not(windows))]
    #[test]
    fn the_scrollback_cursor_is_rebased_onto_the_returned_lines() {
        let session =
            TerminalSession::spawn("sh", &["-c".to_string(), "seq 1 50".to_string()], 40, 6, 1000)
                .expect("spawn");
        for _ in 0..300 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            if session.scrollback_text().0.iter().any(|l| l.trim() == "50") {
                break;
            }
        }
        let (lines, cursor) = session.scrollback_text();
        assert!(cursor < lines.len(), "cursor {cursor} is inside {} lines", lines.len());
        assert!(
            cursor + 6 >= lines.len(),
            "the cursor belongs on the visible screen, near the end of the history"
        );
    }

    #[test]
    // Unix only: headless-CI ConPTY doesn't reliably emit output to the reader,
    // so the output-capture path is exercised here (and by the app's `cat` test).
    // Windows PTY creation is covered by `grid_has_requested_dimensions`.
    #[cfg(not(windows))]
    fn shell_output_reaches_the_grid() {
        let grid = spawn_and_wait(
            "/bin/sh",
            &["-c".into(), "printf hello_ruster".into()],
            "hello_ruster",
        );
        assert!(
            (0..grid.rows).any(|r| grid.row_text(r).contains("hello_ruster")),
            "grid did not contain the shell output; row0 = {:?}",
            grid.row_text(0)
        );
    }

    #[test]
    fn grid_has_requested_dimensions() {
        let (sh, args) = default_shell();
        let session = TerminalSession::spawn(&sh, &args, 40, 6, 1000).expect("spawn");
        let grid = session.snapshot();
        assert_eq!(grid.cols, 40);
        assert_eq!(grid.rows, 6);
        assert_eq!(grid.cells.len(), 40 * 6);
    }

    #[test]
    fn indexed_palette_maps_cube_and_grayscale() {
        assert_eq!(indexed_to_rgb(0), TermColor::Rgb(0, 0, 0));
        assert_eq!(indexed_to_rgb(196), TermColor::Rgb(255, 0, 0)); // cube red
        assert_eq!(indexed_to_rgb(231), TermColor::Rgb(255, 255, 255)); // cube white
        assert_eq!(indexed_to_rgb(232), TermColor::Rgb(8, 8, 8)); // grayscale start
    }
}
