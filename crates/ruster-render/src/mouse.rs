//! Backend-independent mouse input.
//!
//! Both frontends speak this type: the TUI converts `crossterm`'s mouse events
//! via [`from_crossterm`], the raylib backend divides pixel coordinates by its
//! cell metrics. Everything downstream — hit-testing, drag state, the Lua
//! surface — sees cell coordinates only, so no core handler needs to know which
//! backend it is running under.

use crossterm::event::KeyModifiers;

/// A mouse event in cell coordinates, with the origin at the top-left cell.
///
/// Deliberately has no `Default`: an event without a position is meaningless,
/// so `col`/`row` are mandatory at construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseEvent {
    pub col: u16,
    pub row: u16,
    pub kind: MouseKind,
    pub button: MouseButton,
    pub modifiers: KeyModifiers,
}

impl MouseEvent {
    pub fn new(
        col: u16,
        row: u16,
        kind: MouseKind,
        button: MouseButton,
        modifiers: KeyModifiers,
    ) -> Self {
        Self {
            col,
            row,
            kind,
            button,
            modifiers,
        }
    }
}

/// What the mouse did. Scroll directions are separate variants rather than a
/// signed delta because terminals report notches, not pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseKind {
    Down,
    Up,
    Drag,
    Move,
    ScrollUp,
    ScrollDown,
    ScrollLeft,
    ScrollRight,
}

impl MouseKind {
    /// True for the four scroll variants.
    pub fn is_scroll(self) -> bool {
        matches!(
            self,
            MouseKind::ScrollUp
                | MouseKind::ScrollDown
                | MouseKind::ScrollLeft
                | MouseKind::ScrollRight
        )
    }

    /// True when no button can be attached to this event — motion without a
    /// held button, and every scroll notch.
    pub fn is_buttonless(self) -> bool {
        self == MouseKind::Move || self.is_scroll()
    }
}

/// The button carried by a press, release, or drag. `None` is the button of a
/// bare move or a scroll notch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    None,
}

/// The shape the GUI pointer takes over a given zone.
///
/// Distinct from [`crate::CursorKind`], which is the shape of the *text caret*
/// inside a buffer. This one is the mouse pointer; the TUI ignores it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PointerKind {
    #[default]
    Default,
    IBeam,
    Resize,
    Crosshair,
    PointingHand,
}

/// Convert a `crossterm` mouse event into the backend-independent form.
///
/// Crossterm already reports cell coordinates, so this is a pure re-shaping:
/// the button moves out of the kind and into its own field.
pub fn from_crossterm(ev: crossterm::event::MouseEvent) -> MouseEvent {
    use crossterm::event::{MouseButton as CtButton, MouseEventKind as CtKind};

    let button = |b: CtButton| match b {
        CtButton::Left => MouseButton::Left,
        CtButton::Right => MouseButton::Right,
        CtButton::Middle => MouseButton::Middle,
    };

    let (kind, button) = match ev.kind {
        CtKind::Down(b) => (MouseKind::Down, button(b)),
        CtKind::Up(b) => (MouseKind::Up, button(b)),
        CtKind::Drag(b) => (MouseKind::Drag, button(b)),
        CtKind::Moved => (MouseKind::Move, MouseButton::None),
        CtKind::ScrollUp => (MouseKind::ScrollUp, MouseButton::None),
        CtKind::ScrollDown => (MouseKind::ScrollDown, MouseButton::None),
        CtKind::ScrollLeft => (MouseKind::ScrollLeft, MouseButton::None),
        CtKind::ScrollRight => (MouseKind::ScrollRight, MouseButton::None),
    };

    MouseEvent::new(ev.column, ev.row, kind, button, ev.modifiers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{
        MouseButton as CtButton, MouseEvent as CtEvent, MouseEventKind as CtKind,
    };

    fn ct(kind: CtKind) -> CtEvent {
        CtEvent {
            kind,
            column: 7,
            row: 3,
            modifiers: KeyModifiers::empty(),
        }
    }

    #[test]
    fn mouse_kind_round_trips_via_debug() {
        let all = [
            MouseKind::Down,
            MouseKind::Up,
            MouseKind::Drag,
            MouseKind::Move,
            MouseKind::ScrollUp,
            MouseKind::ScrollDown,
            MouseKind::ScrollLeft,
            MouseKind::ScrollRight,
        ];
        let names: Vec<String> = all.iter().map(|k| format!("{k:?}")).collect();
        // Every variant prints distinctly — the Lua surface sends these strings
        // across, so two variants sharing a name would silently merge.
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), all.len());
        assert_eq!(names[0], "Down");
        assert_eq!(names[4], "ScrollUp");
    }

    #[test]
    fn from_crossterm_maps_scroll_up() {
        let ev = from_crossterm(ct(CtKind::ScrollUp));
        assert_eq!(ev.kind, MouseKind::ScrollUp);
        assert_eq!(ev.button, MouseButton::None);
        assert_eq!((ev.col, ev.row), (7, 3));
    }

    #[test]
    fn from_crossterm_maps_drag_with_left_button() {
        let ev = from_crossterm(ct(CtKind::Drag(CtButton::Left)));
        assert_eq!(ev.kind, MouseKind::Drag);
        assert_eq!(ev.button, MouseButton::Left);
    }

    #[test]
    fn mouse_button_none_only_for_move_or_scroll() {
        let buttoned = [
            CtKind::Down(CtButton::Left),
            CtKind::Up(CtButton::Right),
            CtKind::Drag(CtButton::Middle),
        ];
        for kind in buttoned {
            let ev = from_crossterm(ct(kind));
            assert_ne!(ev.button, MouseButton::None, "{kind:?} lost its button");
            assert!(!ev.kind.is_buttonless(), "{kind:?} should carry a button");
        }

        let buttonless = [
            CtKind::Moved,
            CtKind::ScrollUp,
            CtKind::ScrollDown,
            CtKind::ScrollLeft,
            CtKind::ScrollRight,
        ];
        for kind in buttonless {
            let ev = from_crossterm(ct(kind));
            assert_eq!(ev.button, MouseButton::None, "{kind:?} invented a button");
            assert!(ev.kind.is_buttonless(), "{kind:?} should be buttonless");
        }
    }

    #[test]
    fn pointer_kind_defaults_to_default() {
        assert_eq!(PointerKind::default(), PointerKind::Default);
    }

    #[test]
    fn modifiers_survive_conversion() {
        let mut ev = ct(CtKind::Down(CtButton::Left));
        ev.modifiers = KeyModifiers::ALT | KeyModifiers::CONTROL;
        let out = from_crossterm(ev);
        assert!(out.modifiers.contains(KeyModifiers::ALT));
        assert!(out.modifiers.contains(KeyModifiers::CONTROL));
    }
}
