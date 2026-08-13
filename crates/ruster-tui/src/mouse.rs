//! Mouse dispatch.
//!
//! Both frontends funnel their native mouse input through
//! [`ruster_render::mouse::MouseEvent`] and land here, so hit-testing and the
//! per-zone handlers stay free of backend conditionals. Keeping this out of
//! `app.rs` also keeps that file from absorbing the whole mouse surface.

use std::collections::HashMap;
use std::time::Instant;

use crossterm::event::KeyModifiers;
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

/// Route one mouse event to its handler.
pub fn handle_mouse_event(app: &mut App, ev: MouseEvent) {
    if ev.kind != MouseKind::Down
        || ev.button != MouseButton::Left
        || !ev.modifiers.contains(KeyModifiers::ALT)
    {
        return;
    }
    if let Some((wid, offset)) = app.buffer_offset_at(ev.col, ev.row) {
        if let Some(win) = app.ws.borrow_mut().windows.window_mut(wid) {
            win.cursors.add_cursor(offset);
        }
    }
}
