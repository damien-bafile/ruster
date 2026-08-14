//! What the seat can be focused on.
//!
//! This exists for one bound. `PopupManager::grab_popup` requires
//! `SeatHandler::KeyboardFocus: From<PopupKind>`, and the focus type used to be
//! `WlSurface` — both of them foreign, so that impl is forbidden by the orphan
//! rule and no popup could ever take a real grab. A local type can implement it.
//!
//! Deliberately a newtype over `WlSurface` rather than anvil's enum of window
//! kinds. Anvil's variants exist because a `Window`, a `LayerSurface` and an X11
//! surface answer input differently; here every focusable thing is a surface —
//! toplevels and popups alike — and an editor pane is not focusable by the seat
//! at all (`update_keyboard_focus` maps a pane to `None`, which clears it, and
//! the pane reads keys from the compositor instead). An enum whose variants all
//! did the same thing would be ceremony, and each variant is a place for the
//! delegation to drift.
//!
//! Everything below forwards to the `WlSurface` impls smithay already provides,
//! so this adds a conversion and no behaviour.

use std::borrow::Cow;

use smithay::backend::input::KeyState;
use smithay::desktop::PopupKind;
use smithay::input::keyboard::{KeyboardTarget, KeysymHandle, ModifiersState};
use smithay::input::pointer::{
    AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
    GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent,
    GestureSwipeUpdateEvent, MotionEvent, PointerTarget, RelativeMotionEvent,
};
use smithay::input::{Seat, SeatHandler};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{IsAlive, Serial};
use smithay::wayland::seat::WaylandFocus;

/// A surface the seat can hold focus on: a toplevel, or a popup that grabbed.
#[derive(Debug, Clone, PartialEq)]
pub struct FocusTarget(pub WlSurface);

impl From<WlSurface> for FocusTarget {
    fn from(surface: WlSurface) -> Self {
        FocusTarget(surface)
    }
}

/// The conversion this module exists for: the bound `grab_popup` needs.
impl From<PopupKind> for FocusTarget {
    fn from(popup: PopupKind) -> Self {
        FocusTarget(popup.wl_surface().clone())
    }
}

impl IsAlive for FocusTarget {
    fn alive(&self) -> bool {
        self.0.alive()
    }
}

impl WaylandFocus for FocusTarget {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        Some(Cow::Borrowed(&self.0))
    }
}

impl<D: SeatHandler + 'static> KeyboardTarget<D> for FocusTarget {
    fn enter(&self, seat: &Seat<D>, data: &mut D, keys: Vec<KeysymHandle<'_>>, serial: Serial) {
        KeyboardTarget::enter(&self.0, seat, data, keys, serial)
    }

    fn leave(&self, seat: &Seat<D>, data: &mut D, serial: Serial) {
        KeyboardTarget::leave(&self.0, seat, data, serial)
    }

    fn key(
        &self,
        seat: &Seat<D>,
        data: &mut D,
        key: KeysymHandle<'_>,
        state: KeyState,
        serial: Serial,
        time: u32,
    ) {
        KeyboardTarget::key(&self.0, seat, data, key, state, serial, time)
    }

    fn modifiers(&self, seat: &Seat<D>, data: &mut D, modifiers: ModifiersState, serial: Serial) {
        KeyboardTarget::modifiers(&self.0, seat, data, modifiers, serial)
    }
}

impl<D: SeatHandler + 'static> PointerTarget<D> for FocusTarget {
    fn enter(&self, seat: &Seat<D>, data: &mut D, event: &MotionEvent) {
        PointerTarget::enter(&self.0, seat, data, event)
    }
    fn motion(&self, seat: &Seat<D>, data: &mut D, event: &MotionEvent) {
        PointerTarget::motion(&self.0, seat, data, event)
    }
    fn relative_motion(&self, seat: &Seat<D>, data: &mut D, event: &RelativeMotionEvent) {
        PointerTarget::relative_motion(&self.0, seat, data, event)
    }
    fn button(&self, seat: &Seat<D>, data: &mut D, event: &ButtonEvent) {
        PointerTarget::button(&self.0, seat, data, event)
    }
    fn axis(&self, seat: &Seat<D>, data: &mut D, frame: AxisFrame) {
        PointerTarget::axis(&self.0, seat, data, frame)
    }
    fn frame(&self, seat: &Seat<D>, data: &mut D) {
        PointerTarget::frame(&self.0, seat, data)
    }
    fn gesture_swipe_begin(&self, seat: &Seat<D>, data: &mut D, event: &GestureSwipeBeginEvent) {
        PointerTarget::gesture_swipe_begin(&self.0, seat, data, event)
    }
    fn gesture_swipe_update(&self, seat: &Seat<D>, data: &mut D, event: &GestureSwipeUpdateEvent) {
        PointerTarget::gesture_swipe_update(&self.0, seat, data, event)
    }
    fn gesture_swipe_end(&self, seat: &Seat<D>, data: &mut D, event: &GestureSwipeEndEvent) {
        PointerTarget::gesture_swipe_end(&self.0, seat, data, event)
    }
    fn gesture_pinch_begin(&self, seat: &Seat<D>, data: &mut D, event: &GesturePinchBeginEvent) {
        PointerTarget::gesture_pinch_begin(&self.0, seat, data, event)
    }
    fn gesture_pinch_update(&self, seat: &Seat<D>, data: &mut D, event: &GesturePinchUpdateEvent) {
        PointerTarget::gesture_pinch_update(&self.0, seat, data, event)
    }
    fn gesture_pinch_end(&self, seat: &Seat<D>, data: &mut D, event: &GesturePinchEndEvent) {
        PointerTarget::gesture_pinch_end(&self.0, seat, data, event)
    }
    fn gesture_hold_begin(&self, seat: &Seat<D>, data: &mut D, event: &GestureHoldBeginEvent) {
        PointerTarget::gesture_hold_begin(&self.0, seat, data, event)
    }
    fn gesture_hold_end(&self, seat: &Seat<D>, data: &mut D, event: &GestureHoldEndEvent) {
        PointerTarget::gesture_hold_end(&self.0, seat, data, event)
    }
    fn leave(&self, seat: &Seat<D>, data: &mut D, serial: Serial, time: u32) {
        PointerTarget::leave(&self.0, seat, data, serial, time)
    }
}
