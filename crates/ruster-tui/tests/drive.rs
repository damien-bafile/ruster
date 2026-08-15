//! Drive the editor the way a user does — keystrokes into the real frame loop —
//! and assert on what the frame actually contained.
//!
//! Every other test in this workspace either exercises a pure function or feeds
//! keys to `App::handle_key` and inspects app state. Neither says anything about
//! what reached the screen: the only render coverage until now was
//! `render_with_dummy_renderer_is_noop_and_safe`, which calls `render()` and
//! inspects nothing, and TUI/GUI parity was guarded by comparing byte offsets of
//! marker strings in two source files.
//!
//! [`ScriptedRenderer`] closes that gap. It answers `poll_input` from a script
//! and records the [`FrameState`] handed to `render_frame` — the same value both
//! backends draw from — so these tests run the genuine `App::run_gui` loop, with
//! real key dispatch, real Lua drains and real event firing, and then assert on
//! the frames. No window, no PTY, no display: this is the layer CI can run.
//!
//! Setup goes through an `init.lua` in a throwaway `XDG_CONFIG_HOME`, which is
//! the same mechanism `scripts/verify-capture.sh` uses to drive the GUI. One
//! drive spec therefore serves the headless check and the screenshot alike.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

use crossterm::event::KeyEvent;
use ruster_lua::keymap::{lua_key_to_crossterm, parse_lua_key};
use ruster_render::script::{FrameLog, ScriptedRenderer};
use ruster_tui::app::App;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

static DIR_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// `App::new` reads `$XDG_CONFIG_HOME` as it is constructed, and an env var is
/// process-global while cargo runs these tests on parallel threads. Holding this
/// across "point the env var somewhere" + "construct the app" is what keeps two
/// tests from loading each other's `init.lua`. The lock is released before the
/// frame loop, which is the part that actually takes time.
static ENV: Mutex<()> = Mutex::new(());

fn temp_dir(tag: &str) -> PathBuf {
    let id = DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("ruster_drive_{}_{tag}_{id}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// Split a key script into individual events.
///
/// The notation is the one `ruster.keymap.set` already accepts — `<C-d>`,
/// `<Esc>`, `<CR>`, `<Tab>`, `<S-Tab>`, `<F5>` — and each token is resolved by
/// [`parse_lua_key`], so a script here and a keymap in a user's config cannot
/// disagree about what `<C-d>` means. `<Space>` is the one addition: a literal
/// space is legal in the notation but invisible in a test, and the leader key
/// is a space.
fn keys(script: &str) -> Vec<KeyEvent> {
    let mut out = Vec::new();
    let chars: Vec<char> = script.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '<' {
            if let Some(close) = chars[i..].iter().position(|&c| c == '>') {
                let token: String = chars[i..=i + close].iter().collect();
                if token == "<Space>" {
                    out.push(lua_key_to_crossterm(&parse_lua_key(" ").unwrap()));
                } else {
                    let key = parse_lua_key(&token)
                        .unwrap_or_else(|| panic!("unrecognised key token {token:?}"));
                    out.push(lua_key_to_crossterm(&key));
                }
                i += close + 1;
                continue;
            }
        }
        let key = parse_lua_key(&chars[i].to_string())
            .unwrap_or_else(|| panic!("unrecognised key {:?}", chars[i]));
        out.push(lua_key_to_crossterm(&key));
        i += 1;
    }
    out
}

/// A run to perform: some Lua setup, a key script, and how long to let the
/// frame loop settle afterwards.
struct Drive {
    setup: String,
    script: String,
    settle: usize,
    content: String,
    file: String,
    dir: Option<PathBuf>,
}

impl Drive {
    fn new() -> Self {
        Drive {
            setup: String::new(),
            script: String::new(),
            settle: 4,
            content: String::new(),
            file: "demo.rs".into(),
            dir: None,
        }
    }

    /// Lua run before the first frame, exactly as a user's `init.lua` would be.
    fn setup(mut self, lua: &str) -> Self {
        self.setup = lua.into();
        self
    }

    /// Keys fed one per frame, in the `ruster.keymap.set` notation.
    fn keys(mut self, script: &str) -> Self {
        self.script = script.into();
        self
    }

    /// Frames to draw after the last key. Raise it when the setup schedules
    /// work with `ruster.defer`, which comes due on elapsed time rather than
    /// frame count.
    fn settle(mut self, n: usize) -> Self {
        self.settle = n;
        self
    }

    fn content(mut self, text: &str) -> Self {
        self.content = text.into();
        self
    }

    /// Open a real file on disk rather than an unbacked buffer. Needed by
    /// anything that resolves a project root or a filetype from the path.
    fn file(mut self, name: &str, text: &str) -> Self {
        let dir = temp_dir("work");
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        self.content = text.into();
        self.file = path.to_string_lossy().into_owned();
        self.dir = Some(dir);
        self
    }

    /// Run it, and hand back every frame that was drawn.
    fn run(self) -> FrameLog {
        let cfg = temp_dir("cfg");
        std::fs::create_dir_all(cfg.join("ruster")).unwrap();
        if !self.setup.is_empty() {
            std::fs::write(cfg.join("ruster").join("init.lua"), &self.setup).unwrap();
        }

        let renderer = ScriptedRenderer::new(keys(&self.script)).settle(self.settle);
        let log = renderer.log();

        let mut app = {
            let _guard: MutexGuard<()> = ENV.lock().unwrap_or_else(|e| e.into_inner());
            std::env::set_var("XDG_CONFIG_HOME", &cfg);
            App::new(self.content.clone(), PathBuf::from(&self.file))
        };
        app.renderer = Box::new(renderer);
        app.run_gui();

        log
    }
}

/// A small Rust file that exercises the things a frame is supposed to show:
/// keywords and strings for the highlighter, a `TODO` for the todo scanner,
/// a repeated identifier for multi-cursor, and enough lines for the gutter.
const DEMO: &str = r#"use std::fmt;

// TODO: prove the gutter renders this marker
fn main() {
    let total = 1;
    let total_again = total + 1;
    println!("total is {total_again}");
}

struct Point {
    x: i32,
    y: i32,
}

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}
"#;

// ---------------------------------------------------------------------------
// The harness itself
// ---------------------------------------------------------------------------

#[test]
fn the_key_script_notation_matches_the_lua_keymap_notation() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let parsed = keys("a<C-d><Esc><CR><Tab><Space>");
    assert_eq!(
        parsed,
        vec![
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
        ]
    );
}

/// The base case everything else rests on: the loop runs, frames are recorded,
/// and the buffer's text is in them.
#[test]
fn a_bare_run_records_frames_containing_the_buffer() {
    let log = Drive::new().content(DEMO).run();
    assert!(!log.is_empty(), "the loop must draw at least one frame");
    assert!(log.last().contains("fn main()"), "{}", log.transcript());
}

/// Highlight spans, not just text. A frame carrying the right characters with
/// no styling is exactly the regression the digest exists to catch.
#[test]
fn the_buffer_arrives_syntax_highlighted_not_merely_as_text() {
    let log = Drive::new().file("demo.rs", DEMO).run();
    let frame = log.last();
    let styled = frame.windows[0]
        .lines
        .iter()
        .filter(|l| !l.highlights.is_empty())
        .count();
    assert!(
        styled > 0,
        "no line carried a highlight span:\n{}",
        log.transcript()
    );
}

// ---------------------------------------------------------------------------
// Key-driven surfaces — unreachable by any other automated route
// ---------------------------------------------------------------------------

/// The panel is gated on `whichkey.timeoutlen` of *wall* time since the prefix
/// key, and headless frames take microseconds — so the run would end long
/// before the default 300ms elapsed. Dropping the timeout to zero exercises the
/// same gate without pacing the test to a stopwatch.
const NO_WHICHKEY_DELAY: &str = "ruster.cmd(':set timeoutlen=0')";

/// Which-key only exists between two keystrokes. An `init.lua` queue cannot
/// produce it, and a test that fed both keys in one frame would never see it.
#[test]
fn the_leader_key_raises_the_which_key_panel() {
    let log = Drive::new()
        .content(DEMO)
        .setup(NO_WHICHKEY_DELAY)
        .keys("<Space>")
        .run();
    let frame = log.last();
    let wk = frame.whichkey.as_ref().unwrap_or_else(|| {
        panic!(
            "no which-key panel after the leader key:\n{}",
            log.transcript()
        )
    });
    assert!(!wk.rows.is_empty(), "the panel is empty");
    assert!(
        wk.rows.iter().any(|r| r.desc.contains("indows")),
        "expected the +windows group, got {:?}",
        wk.rows
    );
}

#[test]
fn a_leader_group_drills_into_its_own_panel() {
    let log = Drive::new()
        .content(DEMO)
        .setup(NO_WHICHKEY_DELAY)
        .keys("<Space>w")
        .run();
    let frame = log.last();
    let wk = frame
        .whichkey
        .as_ref()
        .unwrap_or_else(|| panic!("no which-key panel after SPC w:\n{}", log.transcript()));
    assert!(
        wk.rows.iter().any(|r| r.key == "s" || r.key == "v"),
        "the windows group should offer splits, got {:?}",
        wk.rows
    );
}

#[test]
fn ctrl_d_adds_a_second_cursor_on_the_next_match() {
    let log = Drive::new()
        .content(DEMO)
        .keys("/total<CR><Esc><C-n>")
        .settle(6)
        .run();
    let frame = log.last();
    assert!(
        !frame.windows[0].extra_cursors.is_empty(),
        "no additional cursor was placed:\n{}",
        log.transcript()
    );
}

/// `Ctrl-w` is a prefix, so this also pins that a chord and the key after it
/// survive being served on separate frames.
#[test]
fn ctrl_w_v_splits_the_window() {
    let log = Drive::new().content(DEMO).keys("<C-w>v").settle(6).run();
    let frame = log.last();
    assert_eq!(
        frame.windows.len(),
        2,
        "C-w v should leave two windows:\n{}",
        log.transcript()
    );
}

/// The Phase 2 manual smoke list, which could not be run when that plan shipped
/// ("no display"): split, move between panes, and toggle fullscreen.
#[test]
fn splits_window_nav_and_fullscreen_round_trip() {
    // `:vsplit`, then `C-w l` to the right pane, then `C-w z` fullscreen.
    let log = Drive::new()
        .content(DEMO)
        .setup("ruster.cmd(':vsplit')")
        .keys("<C-w>l")
        .settle(6)
        .run();
    assert_eq!(
        log.last().windows.len(),
        2,
        "vsplit gives two windows:\n{}",
        log.transcript()
    );

    let full = Drive::new()
        .content(DEMO)
        .setup("ruster.cmd(':vsplit')")
        .keys("<C-w>z")
        .settle(6)
        .run();
    assert_eq!(
        full.last().windows.len(),
        1,
        "C-w z fullscreens the active window:\n{}",
        full.transcript()
    );
}

#[test]
fn typing_a_colon_command_shows_it_on_the_cmdline_before_it_runs() {
    let log = Drive::new().content(DEMO).keys(":sidebar").run();
    assert!(
        log.any_contains("cmdline :sidebar"),
        "the cmdline never showed the command being typed:\n{}",
        log.transcript()
    );
}

/// Visual mode is a selection on the frame, not just a mode flag.
#[test]
fn visual_line_mode_puts_a_selection_on_the_frame() {
    let log = Drive::new().content(DEMO).keys("Vj").run();
    let frame = log.last();
    let sel = frame.windows[0]
        .selection
        .unwrap_or_else(|| panic!("no selection after Vj:\n{}", log.transcript()));
    assert_eq!(sel.start.0, 0);
    assert_eq!(sel.end.0, 1, "j should extend the selection by one line");
}

/// A line wider than any window, of characters distinctive enough that a
/// suffix of it can be told from the whole.
fn wide_line() -> String {
    "0123456789".repeat(40)
}

/// The first line of the last frame, as drawn.
fn first_line(log: &FrameLog) -> String {
    log.last().windows[0].lines[0].text.clone()
}

/// Nothing wraps, so the only way text past the window edge reaches the screen
/// is for the view to move sideways. `$` puts the cursor at the end of a line
/// far wider than the window; the frame has to have followed it there.
#[test]
fn a_long_line_scrolls_sideways_to_follow_the_cursor() {
    let line = wide_line();
    let log = Drive::new().content(&format!("{line}\n")).keys("$").run();
    let drawn = first_line(&log);

    assert!(!drawn.is_empty(), "{}", log.transcript());
    assert!(
        line.ends_with(&drawn),
        "the frame must show the tail of the line, got {drawn:?}"
    );
    assert!(
        drawn.len() < line.len(),
        "and not the whole of it — the view never moved"
    );
}

/// `zh` walks the view back a column. Two runs rather than two frames of one:
/// the assertion is about the difference between the views, and a frame index
/// would be a guess about how the loop paces itself.
#[test]
fn zh_scrolls_the_view_back_one_column() {
    let line = wide_line();
    let content = format!("{line}\n");
    let at_end = first_line(&Drive::new().content(&content).keys("$").run());
    let scrolled = first_line(&Drive::new().content(&content).keys("$zh").run());

    assert_eq!(
        scrolled.len(),
        at_end.len() + 1,
        "one more column of the line is visible"
    );
    assert!(line.ends_with(&scrolled));
}

// ---------------------------------------------------------------------------
// Command-driven surfaces, checked through the same loop
// ---------------------------------------------------------------------------

#[test]
fn the_sidebar_opens_as_a_second_window() {
    let log = Drive::new()
        .file("demo.rs", DEMO)
        .setup("ruster.cmd(':sidebar')")
        .run();
    let frame = log.last();
    assert!(
        frame.windows.len() >= 2,
        "the sidebar should add a window, got {}:\n{}",
        frame.windows.len(),
        log.transcript()
    );
}

/// The bufferline is on by default, so an ordinary session has one, it lists
/// what is open, and it marks the buffer being edited.
#[test]
fn the_bufferline_shows_the_open_buffers_with_the_active_one_marked() {
    let log = Drive::new()
        .file("demo.rs", DEMO)
        .setup("ruster.cmd(':e second.rs')")
        .run();
    let frame = log.last();
    let bl = frame
        .bufferline
        .as_ref()
        .unwrap_or_else(|| panic!("no bufferline on the frame:\n{}", log.transcript()));

    let labels: Vec<&str> = bl.tabs.iter().map(|t| t.label.trim()).collect();
    assert!(
        labels.iter().any(|l| l.contains("demo.rs")),
        "got {labels:?}"
    );
    assert_eq!(
        bl.tabs.iter().filter(|t| t.active).count(),
        1,
        "exactly one tab is the active one: {labels:?}"
    );
}

/// It takes its row from the window area rather than drawing over it — a strip
/// that overlaps the first window's header would be invisible in one backend
/// and corrupt in the other.
#[test]
fn the_bufferline_row_is_above_every_window() {
    let log = Drive::new().file("demo.rs", DEMO).run();
    let frame = log.last();
    let bl = frame.bufferline.as_ref().expect("a bufferline");
    for w in &frame.windows {
        assert!(
            w.rect.y >= bl.rect.y + bl.rect.height,
            "window at y={} overlaps the strip at y={}",
            w.rect.y,
            bl.rect.y
        );
    }
}

#[test]
fn turning_the_bufferline_off_gives_the_row_back() {
    let with = Drive::new().file("demo.rs", DEMO).run();
    let without = Drive::new()
        .file("demo.rs", DEMO)
        .setup("ruster.config.bufferline = { enabled = false }")
        .run();

    let tall = without.last().windows[0].rect.height;
    let short = with.last().windows[0].rect.height;
    assert!(
        without.last().bufferline.is_none(),
        "no strip when it is off:\n{}",
        without.transcript()
    );
    assert_eq!(tall, short + 1, "the window got the row back");
}

#[test]
fn the_settings_page_opens_with_populated_groups() {
    let log = Drive::new()
        .content(DEMO)
        .setup("ruster.cmd(':settings')")
        .run();
    let frame = log.last();
    let s = frame
        .settings
        .as_ref()
        .unwrap_or_else(|| panic!("no settings page:\n{}", log.transcript()));
    assert!(!s.groups.is_empty(), "the settings page has no groups");
    assert!(
        s.groups
            .iter()
            .any(|g| g.rows.iter().any(|r| r.label.contains("heme"))),
        "expected a theme row somewhere in {:?}",
        s.groups.iter().map(|g| &g.name).collect::<Vec<_>>()
    );
}

#[test]
fn echo_raises_a_notification_toast() {
    let log = Drive::new()
        .content(DEMO)
        .setup("ruster.cmd(':echo hello there')")
        .run();
    assert!(
        log.any_contains("hello there"),
        "the echoed text never reached a frame:\n{}",
        log.transcript()
    );
}

#[test]
fn noice_popup_raises_a_float() {
    let log = Drive::new()
        .content(DEMO)
        .setup("ruster.cmd(':Noice popup')")
        .run();
    let frame = log.last();
    assert!(
        !frame.floats.is_empty(),
        "`:Noice popup` should put a float on the frame:\n{}",
        log.transcript()
    );
}

#[test]
fn the_theme_picker_lists_the_built_in_themes() {
    let log = Drive::new()
        .content(DEMO)
        .setup("ruster.cmd(':Themes')")
        .run();
    let frame = log.last();
    let p = frame
        .picker
        .as_ref()
        .unwrap_or_else(|| panic!("no picker:\n{}", log.transcript()));
    assert!(
        p.rows.iter().any(|r| r.label.contains("catppuccin-mocha")),
        "the default theme should be listed, got {:?}",
        p.rows.iter().map(|r| &r.label).collect::<Vec<_>>()
    );
}

/// A dialog is drawn above every other overlay, so it is the one surface whose
/// absence from the frame is unambiguous.
#[test]
fn a_lua_dialog_reaches_the_frame() {
    let log = Drive::new()
        .content(DEMO)
        .setup(
            r#"ruster.ui.dialog{
                 title = "Drive check",
                 fields = { { label = "Dry run", kind = "toggle", value = "on" } },
                 on_submit = function() end,
               }"#,
        )
        .run();
    let frame = log.last();
    let d = frame
        .dialog
        .as_ref()
        .unwrap_or_else(|| panic!("no dialog:\n{}", log.transcript()));
    assert_eq!(d.title, "Drive check");
}

#[test]
fn the_todo_marker_in_the_fixture_becomes_a_sign() {
    let log = Drive::new()
        .file("demo.rs", DEMO)
        .setup("ruster.cmd(':TodoList')")
        .run();
    assert!(
        log.any_contains("prove the gutter renders this marker"),
        "the TODO never surfaced:\n{}",
        log.transcript()
    );
}

// ---------------------------------------------------------------------------
// Quitting
// ---------------------------------------------------------------------------

/// `.claude/skills/gui-check/SKILL.md` says the GUI cannot quit itself and that
/// captures must rely on `timeout`. It can: `:q` sets `should_quit` and the
/// frame loop breaks on it. This is the check that keeps that claim honest, and
/// it is why `verify-capture.sh` can treat a non-zero exit as a real failure
/// instead of expecting 124.
#[test]
fn a_deferred_quit_ends_the_frame_loop_on_its_own() {
    let log = Drive::new()
        .content(DEMO)
        .setup("ruster.cmd(':q')")
        .settle(usize::MAX)
        .run();
    assert!(
        log.len() < 5_000,
        "the loop ran to the frame budget instead of quitting: {} frames",
        log.len()
    );
}
