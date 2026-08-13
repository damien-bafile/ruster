//! A headless renderer that plays a scripted key sequence into the real frame
//! loop and records what each frame contained.
//!
//! [`Renderer`] already has the two seams this needs: `poll_input` is where the
//! frame loop asks for a key, and `should_close` is how it is told to stop. A
//! renderer that answers both from a script drives `App::run_gui` end to end —
//! real key dispatch, real Lua drain, real event firing, real `render()` — with
//! no window and no PTY. That is the only way to reach the key-driven surfaces
//! (which-key, flash jump, multi-cursor, pickers, cmdline completion), because
//! an `init.lua` queue can only send `:` commands.
//!
//! Recording the frames is the other half. `render_frame` receives the
//! [`FrameState`] that *both* backends draw from, so a digest of it asserts on
//! what a frame contains rather than merely that building it did not panic —
//! which, until now, was the whole of this project's render coverage.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use crossterm::event::KeyEvent;

use crate::{
    DebugOverlayView, DialogView, FloatView, FrameState, PickerView, Rect, Renderer, SelectionView,
    SettingsView, StatuslineView, StyledLine, TermGridView, WelcomeView, WhichKeyView,
};

/// One window as it was handed to the renderer.
///
/// [`crate::WindowView`] derives only `Default` (it is built field-by-field at
/// several sites), so this owns a clone of the parts a check can be written
/// against rather than borrowing the frame.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowDigest {
    pub rect: Rect,
    pub header: String,
    /// Buffer text with its highlight spans intact, so a check can assert that
    /// syntax highlighting produced something and not just that text arrived.
    pub lines: Vec<StyledLine>,
    pub gutter: Vec<String>,
    /// `(buffer_line, glyph)` — the colour is a theme concern the PNG shows
    /// better than a text digest can.
    pub signs: Vec<(u16, char)>,
    pub statusline: StatuslineView,
    pub cursor: (u16, u16),
    pub extra_cursors: Vec<(u16, u16)>,
    pub scroll_offset: u16,
    pub active: bool,
    pub selection: Option<SelectionView>,
    pub flash_labels: Vec<(u16, u16, String)>,
    pub terminal: Option<TermGridView>,
}

/// Everything one frame put on screen.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FrameDigest {
    pub windows: Vec<WindowDigest>,
    pub cmdline: Option<String>,
    pub noice_mini: Vec<String>,
    pub noice_notify: Option<Vec<String>>,
    pub picker: Option<PickerView>,
    pub whichkey: Option<WhichKeyView>,
    pub settings: Option<SettingsView>,
    pub welcome: Option<WelcomeView>,
    pub debug_overlay: Option<DebugOverlayView>,
    pub floats: Vec<FloatView>,
    pub dialog: Option<DialogView>,
}

impl FrameDigest {
    fn capture(state: &FrameState) -> Self {
        FrameDigest {
            windows: state
                .windows
                .iter()
                .map(|w| WindowDigest {
                    rect: w.rect,
                    header: w.header.clone(),
                    lines: w.lines.clone(),
                    gutter: w.gutter.rows.clone(),
                    signs: w.signs.signs.iter().map(|(l, g, _)| (*l, *g)).collect(),
                    statusline: w.statusline.clone(),
                    cursor: w.cursor,
                    extra_cursors: w.extra_cursors.clone(),
                    scroll_offset: w.scroll_offset,
                    active: w.active,
                    selection: w.selection,
                    flash_labels: w
                        .flash_labels
                        .iter()
                        .map(|f| (f.row, f.col, f.text.clone()))
                        .collect(),
                    terminal: w.terminal.clone(),
                })
                .collect(),
            cmdline: state.cmdline.map(str::to_string),
            noice_mini: state.noice_mini.clone(),
            noice_notify: state
                .noice_notify
                .as_ref()
                .map(|ls| ls.iter().map(|l| l.text.clone()).collect()),
            picker: state.picker.clone(),
            whichkey: state.whichkey.clone(),
            settings: state.settings.clone(),
            welcome: state.welcome.clone(),
            debug_overlay: state.debug_overlay.clone(),
            floats: state.floats.clone(),
            dialog: state.dialog.clone(),
        }
    }

    /// Every piece of text this frame would show, one per line, in draw order.
    ///
    /// Deliberately line-oriented and free of geometry: it is meant to be
    /// diffed between runs and searched by a check, so a one-cell layout shift
    /// must not rewrite the whole file. Geometry belongs in the PNG.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for (i, w) in self.windows.iter().enumerate() {
            out.push_str(&format!(
                "window {i}{}{}\n",
                if w.active { " active" } else { "" },
                if w.header.is_empty() {
                    String::new()
                } else {
                    format!(" header={:?}", w.header)
                },
            ));
            for (row, line) in w.lines.iter().enumerate() {
                let gutter = w.gutter.get(row).map(String::as_str).unwrap_or("");
                out.push_str(&format!("  {gutter}|{}\n", line.text));
            }
            if !w.signs.is_empty() {
                out.push_str(&format!("  signs {:?}\n", w.signs));
            }
            let s = &w.statusline;
            if !(s.left.is_empty() && s.center.is_empty() && s.right.is_empty()) {
                out.push_str(&format!(
                    "  status [{}] [{}] [{}]\n",
                    s.left, s.center, s.right
                ));
            }
            for (row, col, text) in &w.flash_labels {
                out.push_str(&format!("  flash {row},{col} {text}\n"));
            }
            if let Some(t) = &w.terminal {
                out.push_str(&format!("  terminal {}x{}\n", t.cols, t.rows));
            }
        }
        if let Some(c) = &self.cmdline {
            out.push_str(&format!("cmdline {c}\n"));
        }
        for line in &self.noice_mini {
            out.push_str(&format!("toast {line}\n"));
        }
        if let Some(lines) = &self.noice_notify {
            out.push_str("notify\n");
            for l in lines {
                out.push_str(&format!("  {l}\n"));
            }
        }
        if let Some(w) = &self.welcome {
            if w.visible {
                out.push_str(&format!(
                    "welcome version={} lsp={}\n",
                    w.version, w.lsp_status
                ));
                for p in &w.recent_projects {
                    out.push_str(&format!("  recent {p}\n"));
                }
            }
        }
        if let Some(p) = &self.picker {
            out.push_str(&format!(
                "picker {:?} query={:?} {:?}\n",
                p.title, p.query, p.placement
            ));
            for r in &p.rows {
                out.push_str(&format!(
                    "  {} {}\n",
                    if r.selected { ">" } else { " " },
                    r.label
                ));
            }
        }
        if let Some(k) = &self.whichkey {
            out.push_str(&format!("whichkey {:?}\n", k.title));
            for r in &k.rows {
                out.push_str(&format!("  {} {}\n", r.key, r.desc));
            }
        }
        if let Some(s) = &self.settings {
            out.push_str(&format!("settings dirty={}\n", s.dirty));
            for g in &s.groups {
                out.push_str(&format!("  [{}]\n", g.name));
                for r in &g.rows {
                    out.push_str(&format!(
                        "    {} {} = {}\n",
                        if r.selected { ">" } else { " " },
                        r.label,
                        r.value
                    ));
                }
            }
        }
        if let Some(d) = &self.debug_overlay {
            out.push_str(&format!("debug {}\n", d.toolbar));
            for row in d.rows() {
                out.push_str(&format!("  {row}\n"));
            }
        }
        for f in &self.floats {
            out.push_str(&format!(
                "float z={}{}\n",
                f.z,
                f.title
                    .as_ref()
                    .map(|t| format!(" {t:?}"))
                    .unwrap_or_default()
            ));
            for l in &f.lines {
                out.push_str(&format!("  {}\n", l.text));
            }
        }
        if let Some(d) = &self.dialog {
            out.push_str(&format!("dialog {:?}\n", d.title));
            for r in &d.rows {
                out.push_str(&format!(
                    "  {} {} = {}\n",
                    if r.selected { ">" } else { " " },
                    r.label,
                    r.value
                ));
            }
        }
        out
    }

    /// Whether any text anywhere in the frame contains `needle`. The blunt
    /// instrument most checks want: "did the hover float actually say this".
    pub fn contains(&self, needle: &str) -> bool {
        self.to_text().contains(needle)
    }
}

/// A shared handle on the frames a [`ScriptedRenderer`] recorded.
///
/// The app takes its renderer as `Box<dyn Renderer>` and owns it for the whole
/// run, so there is no way to reach back into it afterwards and no `Any` bound
/// to downcast through. Take one of these before boxing the renderer and the
/// frames are still readable when the loop exits.
#[derive(Clone, Default)]
pub struct FrameLog(Rc<RefCell<Vec<FrameDigest>>>);

impl FrameLog {
    pub fn len(&self) -> usize {
        self.0.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.borrow().is_empty()
    }

    /// The last frame drawn — what the screen settled on.
    pub fn last(&self) -> FrameDigest {
        self.0
            .borrow()
            .last()
            .cloned()
            .expect("the loop draws at least one frame")
    }

    pub fn all(&self) -> Vec<FrameDigest> {
        self.0.borrow().clone()
    }

    /// Whether any recorded frame contains `needle`.
    ///
    /// Transient surfaces are why this exists: a which-key panel or a flash
    /// label is gone by the time the run ends, so asking [`Self::last`] would
    /// miss it.
    pub fn any_contains(&self, needle: &str) -> bool {
        self.0.borrow().iter().any(|f| f.contains(needle))
    }

    /// The first frame containing `needle`, for asserting on the rest of the
    /// surface that was on screen at the same moment.
    pub fn first_containing(&self, needle: &str) -> Option<FrameDigest> {
        self.0.borrow().iter().find(|f| f.contains(needle)).cloned()
    }

    /// Every frame concatenated under a numbered header — the artifact worth
    /// printing when a check fails.
    pub fn transcript(&self) -> String {
        self.0
            .borrow()
            .iter()
            .enumerate()
            .map(|(i, f)| format!("=== frame {i} ===\n{}", f.to_text()))
            .collect()
    }
}

/// A [`Renderer`] that feeds a fixed key script into the frame loop and keeps a
/// digest of every frame it was asked to draw.
pub struct ScriptedRenderer {
    keys: VecDeque<KeyEvent>,
    /// Frames still to draw once the keys run out, so that work scheduled by
    /// the last key (a Lua drain, a queued command, a repaint) lands in a
    /// recorded frame instead of being cut off.
    settle: usize,
    /// Hard cap on total frames. `run_gui` has no sleep, so a renderer that
    /// never closes spins a core forever; this turns that into a failed check.
    budget: usize,
    /// One key per frame. `run_gui` polls in a `while let`, so answering every
    /// call would hand over the whole script before the first frame is drawn —
    /// and every intermediate state (the which-key panel between `SPC` and `w`,
    /// the picker between keystrokes) would never be recorded.
    served_this_frame: bool,
    /// Scripted mouse events, drained one per frame like `keys` and for the
    /// same reason: a click and the frame that reacts to it have to be separate
    /// frames or the intermediate state is never recorded.
    mouse: VecDeque<crate::mouse::MouseEvent>,
    /// Whether a mouse event was already served this frame.
    served_mouse_this_frame: bool,
    log: FrameLog,
    viewport: (u16, u16),
}

impl ScriptedRenderer {
    /// A renderer that plays `keys`, then draws 4 more frames before closing.
    pub fn new(keys: Vec<KeyEvent>) -> Self {
        ScriptedRenderer {
            keys: keys.into(),
            settle: 4,
            budget: 5_000,
            served_this_frame: false,
            mouse: VecDeque::new(),
            served_mouse_this_frame: false,
            log: FrameLog::default(),
            viewport: (120, 40),
        }
    }

    /// A handle on the frames this renderer records. Take it before boxing the
    /// renderer into the app, which then owns it for the rest of the run.
    pub fn log(&self) -> FrameLog {
        self.log.clone()
    }

    /// Draw `n` frames after the last key instead of the default 4.
    ///
    /// Lua timers advance on elapsed wall time, not frame count, so a setup
    /// using `ruster.defer` needs a settle long enough to actually reach the
    /// deadline — and headless frames are fast, so that is many more than four.
    pub fn settle(mut self, n: usize) -> Self {
        self.settle = n;
        self
    }

    /// Override the reported viewport. The default is 120x40, roomy enough for
    /// a picker with a preview pane and a which-key panel to appear unclipped.
    pub fn viewport(mut self, cols: u16, rows: u16) -> Self {
        self.viewport = (cols, rows);
        self
    }

    /// Queue one raw mouse event.
    pub fn push_mouse(mut self, ev: crate::mouse::MouseEvent) -> Self {
        self.mouse.push_back(ev);
        self
    }

    /// Queue a press and release at one cell — what a click actually is.
    pub fn simulate_mouse_click(
        &mut self,
        col: u16,
        row: u16,
        button: crate::mouse::MouseButton,
        mods: crossterm::event::KeyModifiers,
    ) {
        use crate::mouse::{MouseEvent, MouseKind};
        self.mouse
            .push_back(MouseEvent::new(col, row, MouseKind::Down, button, mods));
        self.mouse
            .push_back(MouseEvent::new(col, row, MouseKind::Up, button, mods));
    }

    /// Queue a press, intermediate drags, and a release.
    ///
    /// The intermediates matter: a handler that only looks at the release would
    /// pass a test that scripted just the endpoints, while doing nothing at all
    /// during the drag the user can see.
    pub fn simulate_mouse_drag(
        &mut self,
        from: (u16, u16),
        to: (u16, u16),
        button: crate::mouse::MouseButton,
        mods: crossterm::event::KeyModifiers,
    ) {
        use crate::mouse::{MouseEvent, MouseKind};
        const STEPS: u16 = 4;

        self.mouse.push_back(MouseEvent::new(
            from.0,
            from.1,
            MouseKind::Down,
            button,
            mods,
        ));
        // Walk the straight line between the endpoints, exclusive of the start.
        for step in 1..=STEPS {
            let lerp = |a: u16, b: u16| {
                let a = a as i32;
                (a + (b as i32 - a) * step as i32 / STEPS as i32) as u16
            };
            self.mouse.push_back(MouseEvent::new(
                lerp(from.0, to.0),
                lerp(from.1, to.1),
                MouseKind::Drag,
                button,
                mods,
            ));
        }
        self.mouse
            .push_back(MouseEvent::new(to.0, to.1, MouseKind::Up, button, mods));
    }

    /// Queue `notches` wheel events in one direction.
    pub fn simulate_mouse_wheel(
        &mut self,
        col: u16,
        row: u16,
        kind: crate::mouse::MouseKind,
        notches: u16,
        mods: crossterm::event::KeyModifiers,
    ) {
        use crate::mouse::{MouseButton, MouseEvent};
        for _ in 0..notches {
            self.mouse
                .push_back(MouseEvent::new(col, row, kind, MouseButton::None, mods));
        }
    }

    /// How many scripted mouse events are still queued.
    pub fn pending_mouse(&self) -> usize {
        self.mouse.len()
    }
}

impl Renderer for ScriptedRenderer {
    fn render_frame(&mut self, state: &FrameState) {
        self.log.0.borrow_mut().push(FrameDigest::capture(state));
        // The frame that consumed the last key does not count against the
        // settle budget — `settle(n)` means n frames *after* the script, and
        // the whole reason to draw them is to see what that key produced.
        if self.keys.is_empty() && self.mouse.is_empty() && !self.served_this_frame {
            self.settle = self.settle.saturating_sub(1);
        }
        self.served_this_frame = false;
        self.served_mouse_this_frame = false;
        self.budget = self.budget.saturating_sub(1);
    }

    fn viewport_cells(&self) -> (u16, u16) {
        self.viewport
    }

    fn poll_input(&mut self) -> Option<KeyEvent> {
        if self.served_this_frame {
            return None;
        }
        let key = self.keys.pop_front()?;
        self.served_this_frame = true;
        Some(key)
    }

    fn poll_mouse(&mut self) -> Option<crate::mouse::MouseEvent> {
        if self.served_mouse_this_frame {
            return None;
        }
        let ev = self.mouse.pop_front()?;
        self.served_mouse_this_frame = true;
        Some(ev)
    }

    fn should_close(&self) -> bool {
        self.budget == 0 || (self.keys.is_empty() && self.mouse.is_empty() && self.settle == 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GutterView, SignsView, WindowView};
    use crossterm::event::{KeyCode, KeyModifiers};

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    /// One mouse event per frame, for the same reason as one key per frame:
    /// serving the whole script at once would mean no intermediate state is
    /// ever drawn.
    #[test]
    fn one_mouse_event_per_frame_just_like_keys() {
        use crate::mouse::MouseButton;
        let mut r = ScriptedRenderer::new(vec![]);
        r.simulate_mouse_click(3, 4, MouseButton::Left, KeyModifiers::NONE);
        assert_eq!(r.pending_mouse(), 2);

        assert!(r.poll_mouse().is_some(), "first event of the frame");
        assert!(
            r.poll_mouse().is_none(),
            "second event waits for next frame"
        );

        r.render_frame(&FrameState::default());
        assert!(r.poll_mouse().is_some(), "next frame serves the release");
        assert_eq!(r.pending_mouse(), 0);
    }

    #[test]
    fn simulate_mouse_click_emits_down_then_up() {
        use crate::mouse::{MouseButton, MouseKind};
        let mut r = ScriptedRenderer::new(vec![]);
        r.simulate_mouse_click(7, 2, MouseButton::Right, KeyModifiers::NONE);

        let first = r.poll_mouse().expect("a press");
        r.render_frame(&FrameState::default());
        let second = r.poll_mouse().expect("a release");

        assert_eq!(first.kind, MouseKind::Down);
        assert_eq!(second.kind, MouseKind::Up);
        for ev in [first, second] {
            assert_eq!((ev.col, ev.row), (7, 2));
            assert_eq!(ev.button, MouseButton::Right);
        }
    }

    #[test]
    fn simulate_mouse_drag_emits_intermediate_drags() {
        use crate::mouse::{MouseButton, MouseKind};
        let mut r = ScriptedRenderer::new(vec![]);
        r.simulate_mouse_drag((0, 0), (8, 4), MouseButton::Left, KeyModifiers::NONE);

        let mut events = Vec::new();
        while r.pending_mouse() > 0 {
            if let Some(ev) = r.poll_mouse() {
                events.push(ev);
            }
            r.render_frame(&FrameState::default());
        }

        assert_eq!(events.first().map(|e| e.kind), Some(MouseKind::Down));
        assert_eq!(events.last().map(|e| e.kind), Some(MouseKind::Up));
        let drags: Vec<_> = events
            .iter()
            .filter(|e| e.kind == MouseKind::Drag)
            .collect();
        assert!(drags.len() > 1, "a drag is more than its endpoints");
        // The path runs from the start to the end without overshooting.
        assert_eq!(
            (events.last().unwrap().col, events.last().unwrap().row),
            (8, 4)
        );
        assert!(drags.iter().all(|d| d.col <= 8 && d.row <= 4));
    }

    #[test]
    fn simulate_mouse_wheel_emits_one_event_per_notch() {
        use crate::mouse::MouseKind;
        let mut r = ScriptedRenderer::new(vec![]);
        r.simulate_mouse_wheel(1, 1, MouseKind::ScrollDown, 3, KeyModifiers::NONE);
        assert_eq!(r.pending_mouse(), 3);
    }

    /// A script with only mouse events still terminates.
    #[test]
    fn a_mouse_only_script_closes_once_drained() {
        use crate::mouse::MouseButton;
        let mut r = ScriptedRenderer::new(vec![]).settle(1);
        r.simulate_mouse_click(1, 1, MouseButton::Left, KeyModifiers::NONE);
        assert!(!r.should_close(), "events are still pending");

        for _ in 0..10 {
            r.poll_mouse();
            r.render_frame(&FrameState::default());
        }
        assert!(r.should_close());
    }

    /// The whole point of the one-key-per-frame rule: a burst would hand the
    /// script over before the first frame is drawn, so the states *between*
    /// keystrokes — which is where which-key and pickers live — would never be
    /// recorded.
    #[test]
    fn only_one_key_is_served_per_frame() {
        let mut r = ScriptedRenderer::new(vec![key('a'), key('b')]);
        assert_eq!(r.poll_input(), Some(key('a')));
        assert_eq!(r.poll_input(), None, "the frame loop polls in a while-let");
        r.render_frame(&FrameState::default());
        assert_eq!(r.poll_input(), Some(key('b')));
    }

    #[test]
    fn it_closes_once_the_script_and_the_settle_budget_are_spent() {
        let mut r = ScriptedRenderer::new(vec![key('a')]).settle(2);
        assert!(!r.should_close(), "a key is still pending");
        r.poll_input();
        r.render_frame(&FrameState::default());
        assert!(
            !r.should_close(),
            "the key's own frame is not a settle frame"
        );
        r.render_frame(&FrameState::default());
        assert!(!r.should_close(), "one settle frame still owed");
        r.render_frame(&FrameState::default());
        assert!(r.should_close());
    }

    /// A renderer that never closes spins `run_gui` forever, because the loop
    /// has no sleep. The cap turns a hang into a bounded, inspectable failure.
    #[test]
    fn the_frame_budget_closes_a_run_that_would_otherwise_spin() {
        let mut r = ScriptedRenderer::new(vec![]).settle(usize::MAX);
        r.budget = 3;
        for _ in 0..3 {
            assert!(!r.should_close());
            r.render_frame(&FrameState::default());
        }
        assert!(
            r.should_close(),
            "the budget must win over an infinite settle"
        );
    }

    #[test]
    fn a_digest_reports_the_text_of_every_surface_in_the_frame() {
        let mut r = ScriptedRenderer::new(vec![]);
        let state = FrameState {
            windows: vec![WindowView {
                rect: Rect::new(0, 0, 40, 10),
                header: "demo.rs".into(),
                lines: vec![StyledLine {
                    text: "fn main() {}".into(),
                    highlights: vec![],
                }],
                gutter: GutterView {
                    width: 3,
                    rows: vec!["  1".into()],
                },
                signs: SignsView {
                    width: 1,
                    signs: vec![(0, 'E', crate::Color::Default)],
                },
                statusline: StatuslineView {
                    left: "NORMAL".into(),
                    right: "1:1".into(),
                    ..Default::default()
                },
                active: true,
                ..Default::default()
            }],
            cmdline: Some(":w"),
            noice_mini: vec!["saved".into()],
            ..Default::default()
        };
        r.render_frame(&state);

        let text = r.log().last().to_text();
        assert!(
            text.contains("window 0 active header=\"demo.rs\""),
            "{text}"
        );
        assert!(
            text.contains("  1|fn main() {}"),
            "gutter and text on one row: {text}"
        );
        assert!(text.contains("signs [(0, 'E')]"), "{text}");
        assert!(text.contains("status [NORMAL] [] [1:1]"), "{text}");
        assert!(text.contains("cmdline :w"), "{text}");
        assert!(text.contains("toast saved"), "{text}");
        assert!(r.log().any_contains("fn main"));
    }

    /// The digest keeps highlight spans, so a check can tell "the text arrived"
    /// apart from "the text arrived *and* tree-sitter styled it".
    #[test]
    fn the_digest_keeps_highlight_spans_rather_than_flattening_to_text() {
        let mut r = ScriptedRenderer::new(vec![]);
        r.render_frame(&FrameState {
            windows: vec![WindowView {
                lines: vec![StyledLine {
                    text: "fn".into(),
                    highlights: vec![(0, 2, crate::SyntaxStyle::default())],
                }],
                ..Default::default()
            }],
            ..Default::default()
        });
        assert_eq!(r.log().last().windows[0].lines[0].highlights.len(), 1);
    }
}
