//! Mouse dispatch.
//!
//! Both frontends funnel their native mouse input through
//! [`ruster_render::mouse::MouseEvent`] and land here, so hit-testing and the
//! per-zone handlers stay free of backend conditionals. Keeping this out of
//! `app.rs` also keeps that file from absorbing the whole mouse surface.

use std::collections::HashMap;
use std::time::Instant;

use crossterm::event::KeyModifiers;
use ruster_core::action::{Action, Motion};
use ruster_core::cursor::Range;
use ruster_core::windows::{Rect, WindowId};
use ruster_render::mouse::{MouseButton, MouseEvent, MouseKind};

use crate::app::App;

/// Everything the mouse remembers between events.
///
/// A click is only a double-click relative to the last one, a drag only means
/// something relative to where it started, and hover only fires once the
/// pointer stops — none of which a single [`MouseEvent`] can tell you.
#[derive(Debug, Default)]
pub struct MouseState {
    pub click: ClickTracker,
    pub hover: HoverState,
    pub drag: DragState,
    pub menu: MenuRegistry,
    /// Set while a split edge is being dragged.
    pub resize: Option<ResizeState>,
}

/// The last button press, for deciding whether the next one is a double or
/// triple click.
#[derive(Debug, Default)]
pub struct ClickTracker {
    pub last_down: Option<(Instant, u16, u16, MouseButton)>,
    /// How many clicks the current streak is up to: 1 after a single click, 2
    /// after a double, 3 after a triple (which then resets).
    pub streak: u8,
}

/// Where the pointer is and how long it has been there.
#[derive(Debug)]
pub struct HoverState {
    pub last_pos: (u16, u16),
    pub last_move: Instant,
    /// The position hover already fired for, so stillness fires once, not once
    /// per frame.
    pub emitted_for: Option<(u16, u16)>,
}

impl Default for HoverState {
    fn default() -> Self {
        HoverState {
            last_pos: (0, 0),
            last_move: Instant::now(),
            emitted_for: None,
        }
    }
}

/// The in-progress drag, if the button is down and has moved.
#[derive(Debug, Default)]
pub struct DragState {
    /// Buffer offset the drag started from. `None` when no drag is in flight.
    pub anchor: Option<usize>,
    pub kind: DragKind,
    /// The window the drag started in. A drag stays in its originating window
    /// even when the pointer wanders out of it.
    pub wid: Option<WindowId>,
}

/// The shape of the selection a drag builds.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DragKind {
    #[default]
    Char,
    Line,
    Block,
}

/// A split edge being dragged, holding enough to compute a delta and to undo
/// the drag if it is abandoned.
#[derive(Debug, Clone, Copy)]
pub struct ResizeState {
    pub wid: WindowId,
    pub start_col: u16,
    pub start_row: u16,
    pub original: Rect,
}

/// Context-menu items, keyed by the zone that was right-clicked. Plugins add to
/// this through `ruster.context_menu.add`.
#[derive(Debug, Default)]
pub struct MenuRegistry {
    items: HashMap<Zone, Vec<MenuItem>>,
}

impl MenuRegistry {
    pub fn items_for(&self, zone: Zone) -> &[MenuItem] {
        self.items.get(&zone).map(Vec::as_slice).unwrap_or_default()
    }

    pub fn add(&mut self, zone: Zone, item: MenuItem) {
        self.items.entry(zone).or_default().push(item);
    }
}

/// The coarse region a context menu belongs to. Coarser than
/// [`HitZone`](self) deliberately: a menu is registered for "the gutter", not
/// for one particular gutter cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Zone {
    Buffer,
    Gutter,
    Chrome,
    Tab,
}

/// One row of a context menu. `cmd` is a cmdline string so that menu items and
/// typed commands go down the same path.
#[derive(Debug, Clone)]
pub struct MenuItem {
    pub label: String,
    pub cmd: String,
    pub submenu: Vec<MenuItem>,
}

impl MenuItem {
    pub fn new(label: impl Into<String>, cmd: impl Into<String>) -> Self {
        MenuItem {
            label: label.into(),
            cmd: cmd.into(),
            submenu: Vec::new(),
        }
    }
}

/// What sits under a screen cell.
///
/// Resolved in priority order by [`hit_test`]: a float covers whatever is
/// beneath it, chrome and gutter are not buffer text, and everything left over
/// is outside any window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitZone {
    /// A floating box, by index into the last frame's floats (topmost wins).
    Float(usize),
    Chrome(ChromeKind),
    /// The sign/number gutter of a window, and the buffer line beside it.
    Gutter(WindowId, usize),
    /// Buffer text, and the offset it resolves to.
    Buffer(WindowId, usize),
    Outside,
}

/// The parts of a window that are not buffer text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromeKind {
    /// The window's title row.
    Header(WindowId),
    /// The window's statusline row.
    StatusLine(WindowId),
    /// A boundary shared with an adjacent window, draggable to resize.
    /// `vertical` means the edge itself runs vertically — a left/right split.
    SplitEdge { wid: WindowId, vertical: bool },
}

/// Resolve a screen cell to the thing under it.
///
/// Everything is answered from the last rendered frame rather than recomputed,
/// so the hit-test cannot disagree with what is on screen.
pub fn hit_test(app: &App, col: u16, row: u16) -> HitZone {
    // Floats are drawn above everything; the topmost one wins.
    if let Some(idx) = app.last_floats.iter().rposition(|r| contains(*r, col, row)) {
        return HitZone::Float(idx);
    }

    // Buffer text before chrome: it is the largest zone and the common case.
    if let Some((wid, offset)) = app.buffer_offset_at(col, row) {
        return HitZone::Buffer(wid, offset);
    }

    for l in &app.last_layout {
        if !contains(l.rect, col, row) {
            continue;
        }
        let wid = l.window;
        let last_row = l.rect.y + l.rect.height.saturating_sub(1);
        let last_col = l.rect.x + l.rect.width.saturating_sub(1);

        // A shared boundary is a resize handle, so it outranks the chrome row
        // or column it happens to be drawn on.
        if col == last_col && has_neighbour_right(app, l.rect) {
            return HitZone::Chrome(ChromeKind::SplitEdge {
                wid,
                vertical: true,
            });
        }
        if row == last_row && has_neighbour_below(app, l.rect) {
            return HitZone::Chrome(ChromeKind::SplitEdge {
                wid,
                vertical: false,
            });
        }
        if row == l.rect.y {
            return HitZone::Chrome(ChromeKind::Header(wid));
        }
        if row == last_row {
            return HitZone::Chrome(ChromeKind::StatusLine(wid));
        }
        // Inside the window, on a text row, left of the text: the gutter.
        if col < l.text.x && row >= l.text.y && row < l.text.y + l.text.height {
            let line = l.scroll_top + (row - l.text.y) as usize;
            return HitZone::Gutter(wid, line);
        }
        return HitZone::Outside;
    }

    HitZone::Outside
}

fn contains(r: ruster_render::Rect, col: u16, row: u16) -> bool {
    col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height
}

/// Whether another window starts exactly where this one ends, horizontally.
fn has_neighbour_right(app: &App, rect: ruster_render::Rect) -> bool {
    let edge = rect.x + rect.width;
    app.last_layout.iter().any(|o| {
        o.rect.x == edge && o.rect.y < rect.y + rect.height && rect.y < o.rect.y + o.rect.height
    })
}

/// Whether another window starts exactly where this one ends, vertically.
fn has_neighbour_below(app: &App, rect: ruster_render::Rect) -> bool {
    let edge = rect.y + rect.height;
    app.last_layout.iter().any(|o| {
        o.rect.y == edge && o.rect.x < rect.x + rect.width && rect.x < o.rect.x + o.rect.width
    })
}

/// How long after a click a second one still counts as a double-click.
/// Replaced by `config.mouse.double_click_ms` in Task 17.
const DOUBLE_CLICK_MS: u64 = 400;

/// How far a second click may land from the first and still be a double-click.
/// Without slack a shaky hand turns a double-click into two single ones.
const DOUBLE_CLICK_SLACK_CELLS: u16 = 2;

/// Route one mouse event to its handler.
pub fn handle_mouse_event(app: &mut App, ev: MouseEvent) {
    let zone = hit_test(app, ev.col, ev.row);
    if ev.kind == MouseKind::Down {
        on_mouse_down(app, ev, zone);
    }
}

fn on_mouse_down(app: &mut App, ev: MouseEvent, zone: HitZone) {
    let clicks = track_click(app, &ev);
    match zone {
        HitZone::Buffer(wid, offset) => on_buffer_down(app, ev, wid, offset, clicks),
        // Clicking off every window drops into the cmdline, which is the only
        // thing down there to aim at.
        HitZone::Outside => app.vim.set_cmdline(":"),
        // Chrome, gutter and float handling arrive with Tasks 13 and 14.
        HitZone::Chrome(_) | HitZone::Gutter(..) | HitZone::Float(_) => {}
    }
}

/// Fold this press into the click streak and return how many clicks it makes:
/// 1 for a single, 2 for a double, 3 for a triple (after which it restarts).
fn track_click(app: &mut App, ev: &MouseEvent) -> u8 {
    let now = Instant::now();
    let near = |a: u16, b: u16| a.abs_diff(b) <= DOUBLE_CLICK_SLACK_CELLS;

    let continues = match app.mouse.click.last_down {
        Some((at, col, row, button)) => {
            button == ev.button
                && near(col, ev.col)
                && near(row, ev.row)
                && now.duration_since(at).as_millis() as u64 <= DOUBLE_CLICK_MS
        }
        None => false,
    };
    // A triple click ends the streak: the fourth starts a fresh single.
    app.mouse.click.streak = if continues && app.mouse.click.streak < 3 {
        app.mouse.click.streak + 1
    } else {
        1
    };
    app.mouse.click.last_down = Some((now, ev.col, ev.row, ev.button));
    app.mouse.click.streak
}

fn on_buffer_down(app: &mut App, ev: MouseEvent, wid: WindowId, offset: usize, clicks: u8) {
    if ev.button != MouseButton::Left {
        return;
    }

    // Alt adds a caret instead of moving the one that's there.
    if ev.modifiers.contains(KeyModifiers::ALT) {
        if let Some(win) = app.ws.borrow_mut().windows.window_mut(wid) {
            win.cursors.add_cursor(offset);
        }
        return;
    }

    // Ctrl jumps to the start of the word under the pointer.
    if ev.modifiers.contains(KeyModifiers::CONTROL) {
        if let Some(word) = word_range(app, wid, offset) {
            app.ws
                .borrow_mut()
                .execute(Action::Move(Motion::To(word.start())));
        }
        return;
    }

    match clicks {
        2 => {
            if let Some(word) = word_range(app, wid, offset) {
                app.ws.borrow_mut().execute(Action::SelectWord {
                    anchor: word.anchor,
                    head: word.head,
                });
            }
        }
        3 => {
            if let Some(line) = line_range(app, wid, offset) {
                app.ws.borrow_mut().execute(Action::SelectLine {
                    anchor: line.anchor,
                    head: line.head,
                });
            }
        }
        _ => app
            .ws
            .borrow_mut()
            .execute(Action::Move(Motion::To(offset))),
    }
}

/// The word around `offset` in `wid`'s buffer, or `None` if the window is gone.
fn word_range(app: &App, wid: WindowId, offset: usize) -> Option<Range> {
    with_window_buffer(app, wid, |cursors, buffer| {
        cursors.select_word(buffer, offset)
    })
}

/// The line around `offset` in `wid`'s buffer.
fn line_range(app: &App, wid: WindowId, offset: usize) -> Option<Range> {
    with_window_buffer(app, wid, |cursors, buffer| {
        cursors.select_line(buffer, offset)
    })
}

fn with_window_buffer<T>(
    app: &App,
    wid: WindowId,
    f: impl FnOnce(&ruster_core::cursor::CursorSet, &ruster_core::buffer::Buffer) -> T,
) -> Option<T> {
    let ws = app.ws.borrow();
    let win = ws.windows.window(wid)?;
    let doc = ws.buffers.get(win.buffer)?;
    Some(f(&win.cursors, &doc.buffer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Render once so the hit-test has a frame to resolve against, and hand
    /// back the first window's layout.
    fn laid_out(a: &mut App) -> crate::app::WindowLayout {
        a.render();
        *a.last_layout.first().expect("one window was laid out")
    }

    #[test]
    fn hit_test_buffer_for_text_cell() {
        let mut a = App::new("alpha\nbravo\ncharlie\n".into(), PathBuf::from("f.txt"));
        a.config.number = true;
        let l = laid_out(&mut a);
        assert_eq!(
            hit_test(&a, l.text.x, l.text.y),
            HitZone::Buffer(l.window, 0)
        );
    }

    #[test]
    fn hit_test_gutter_for_left_margin() {
        let mut a = App::new("alpha\nbravo\ncharlie\n".into(), PathBuf::from("f.txt"));
        a.config.number = true;
        let l = laid_out(&mut a);
        assert!(l.text.x > 0, "the number gutter reserves columns");
        // Second text row's gutter is the second visible line.
        assert_eq!(
            hit_test(&a, l.text.x - 1, l.text.y + 1),
            HitZone::Gutter(l.window, l.scroll_top + 1)
        );
    }

    #[test]
    fn hit_test_chrome_for_header_row() {
        let mut a = App::new("alpha\n".into(), PathBuf::from("f.txt"));
        let l = laid_out(&mut a);
        assert_eq!(
            hit_test(&a, l.text.x, l.rect.y),
            HitZone::Chrome(ChromeKind::Header(l.window))
        );
    }

    #[test]
    fn hit_test_chrome_for_statusline_row() {
        let mut a = App::new("alpha\n".into(), PathBuf::from("f.txt"));
        let l = laid_out(&mut a);
        let last_row = l.rect.y + l.rect.height - 1;
        assert_eq!(
            hit_test(&a, l.text.x, last_row),
            HitZone::Chrome(ChromeKind::StatusLine(l.window))
        );
    }

    /// Below the last line of a short buffer is still inside the window, but it
    /// is not text — and it is not chrome either.
    #[test]
    fn hit_test_outside_below_a_short_buffer() {
        let mut a = App::new("alpha\n".into(), PathBuf::from("f.txt"));
        let l = laid_out(&mut a);
        assert!(l.text.height > 2, "need blank rows under the text");
        let blank = l.text.y + l.text.height - 1;
        assert_eq!(hit_test(&a, l.text.x, blank), HitZone::Outside);
    }

    #[test]
    fn hit_test_float_wins_over_buffer() {
        let mut a = App::new("alpha\nbravo\n".into(), PathBuf::from("f.txt"));
        a.config.number = true;
        let l = laid_out(&mut a);
        // A cell that resolves to buffer text with no float over it...
        assert_eq!(
            hit_test(&a, l.text.x, l.text.y),
            HitZone::Buffer(l.window, 0)
        );

        // ...is claimed by the hover popup once one is drawn there.
        a.hover = Some(vec![ruster_render::StyledLine {
            text: "hovering".into(),
            highlights: Vec::new(),
        }]);
        a.render();
        let f = *a.last_floats.first().expect("hover raised a float");
        assert!(contains(f, f.x, f.y), "float has a non-empty rect");
        assert_eq!(hit_test(&a, f.x, f.y), HitZone::Float(0));
    }

    /// A cell in no window at all — past the right edge of a narrow layout.
    #[test]
    fn hit_test_outside_beyond_every_window() {
        let mut a = App::new("alpha\n".into(), PathBuf::from("f.txt"));
        laid_out(&mut a);
        assert_eq!(hit_test(&a, u16::MAX, u16::MAX), HitZone::Outside);
    }

    fn down(col: u16, row: u16, modifiers: KeyModifiers) -> MouseEvent {
        MouseEvent::new(col, row, MouseKind::Down, MouseButton::Left, modifiers)
    }

    /// The primary cursor's selection, as text.
    fn selected(a: &App) -> String {
        let ws = a.ws.borrow();
        let win = ws.windows.active_window();
        let r = win.cursors.primary();
        ws.buffers
            .get(win.buffer)
            .map(|d| d.buffer.slice_string(r.start(), r.end()))
            .unwrap_or_default()
    }

    #[test]
    fn left_click_in_buffer_moves_cursor() {
        let mut a = App::new("alpha\nbravo\n".into(), PathBuf::from("f.txt"));
        a.config.number = true;
        let l = laid_out(&mut a);
        handle_mouse_event(
            &mut a,
            down(l.text.x + 2, l.text.y + 1, KeyModifiers::empty()),
        );
        // Third column of the second line: 'a' of "bravo", at offset 8.
        assert_eq!(a.ws.borrow().windows.active_window().cursors.head(), 8);
    }

    #[test]
    fn alt_left_click_adds_cursor() {
        let mut a = App::new("alpha\nbravo\n".into(), PathBuf::from("f.txt"));
        a.config.number = true;
        let l = laid_out(&mut a);
        handle_mouse_event(&mut a, down(l.text.x + 2, l.text.y + 1, KeyModifiers::ALT));
        let ws = a.ws.borrow();
        let cursors = &ws.windows.active_window().cursors;
        assert_eq!(cursors.count(), 2);
        assert!(cursors.iter_heads().any(|h| h == 8));
    }

    #[test]
    fn ctrl_left_click_jumps_to_word_start() {
        let mut a = App::new("alpha bravo\n".into(), PathBuf::from("f.txt"));
        a.config.number = true;
        let l = laid_out(&mut a);
        // Land inside "bravo", which starts at offset 6.
        handle_mouse_event(&mut a, down(l.text.x + 8, l.text.y, KeyModifiers::CONTROL));
        assert_eq!(a.ws.borrow().windows.active_window().cursors.head(), 6);
    }

    #[test]
    fn double_click_selects_word() {
        let mut a = App::new("alpha bravo\n".into(), PathBuf::from("f.txt"));
        a.config.number = true;
        let l = laid_out(&mut a);
        let at = down(l.text.x + 8, l.text.y, KeyModifiers::empty());
        handle_mouse_event(&mut a, at);
        handle_mouse_event(&mut a, at);
        assert_eq!(selected(&a), "bravo");
    }

    #[test]
    fn triple_click_selects_line() {
        let mut a = App::new("alpha bravo\ncharlie\n".into(), PathBuf::from("f.txt"));
        a.config.number = true;
        let l = laid_out(&mut a);
        let at = down(l.text.x + 2, l.text.y, KeyModifiers::empty());
        handle_mouse_event(&mut a, at);
        handle_mouse_event(&mut a, at);
        handle_mouse_event(&mut a, at);
        assert_eq!(selected(&a), "alpha bravo\n");
    }

    /// A fourth click starts a fresh streak rather than staying on the line.
    #[test]
    fn fourth_click_restarts_the_streak() {
        let mut a = App::new("alpha bravo\n".into(), PathBuf::from("f.txt"));
        a.config.number = true;
        let l = laid_out(&mut a);
        let at = down(l.text.x + 8, l.text.y, KeyModifiers::empty());
        for _ in 0..4 {
            handle_mouse_event(&mut a, at);
        }
        assert_eq!(selected(&a), "", "back to a bare caret");
        assert_eq!(a.mouse.click.streak, 1);
    }

    /// Two clicks far apart are two single clicks, not a double.
    #[test]
    fn clicks_far_apart_do_not_pair_into_a_double() {
        let mut a = App::new("alpha bravo\n".into(), PathBuf::from("f.txt"));
        a.config.number = true;
        let l = laid_out(&mut a);
        handle_mouse_event(&mut a, down(l.text.x, l.text.y, KeyModifiers::empty()));
        handle_mouse_event(&mut a, down(l.text.x + 8, l.text.y, KeyModifiers::empty()));
        assert_eq!(a.mouse.click.streak, 1);
        assert_eq!(selected(&a), "");
    }

    #[test]
    fn click_in_outside_zone_focuses_cmdline() {
        let mut a = App::new("alpha\n".into(), PathBuf::from("f.txt"));
        let l = laid_out(&mut a);
        let blank = l.text.y + l.text.height - 1;
        assert_eq!(hit_test(&a, l.text.x, blank), HitZone::Outside);
        handle_mouse_event(&mut a, down(l.text.x, blank, KeyModifiers::empty()));
        assert_eq!(a.vim.mode, ruster_core::vim::VimMode::Cmdline);
    }

    #[test]
    fn hit_test_split_edge_between_side_by_side_windows() {
        use ruster_core::windows::SplitDir;

        let mut a = App::new("alpha\nbravo\n".into(), PathBuf::from("f.txt"));
        a.ws.borrow_mut().windows.split(SplitDir::Vertical);
        a.render();
        assert_eq!(a.last_layout.len(), 2, "two windows are laid out");

        let left = a
            .last_layout
            .iter()
            .min_by_key(|l| l.rect.x)
            .copied()
            .expect("a leftmost window");
        let edge_col = left.rect.x + left.rect.width - 1;
        let mid_row = left.rect.y + left.rect.height / 2;

        assert_eq!(
            hit_test(&a, edge_col, mid_row),
            HitZone::Chrome(ChromeKind::SplitEdge {
                wid: left.window,
                vertical: true,
            })
        );
    }
}
