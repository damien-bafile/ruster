//! Keyboard/pointer handling for the compositor.
//!
//! Two layers live here. The lower one is pure — keysym/modifier matching and
//! the geometry of pointer focus — so it is unit testable without a live
//! display. The upper one is the seat wiring: `on_keyboard_key` and friends
//! are generic over [`InputBackend`], not over a concrete backend, so the
//! nested winit window and libinput on DRM feed the exact same code. That
//! matters beyond tidiness: before the split the only copy of this logic sat
//! inside the winit backend, which meant the DRM path had no input at all and
//! the keybinding path could not be exercised by a test.
//!
//! Phase 0 exposes a quit binding and a workspace-cycle binding whose defaults
//! are `M-S-q` (quit) and `M-t` (cycle workspace); the full ruster keymap lands
//! in Phase 1.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisSource, Event, InputBackend, InputEvent, KeyState,
    KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
};
use smithay::input::keyboard::xkb::keysym_get_name;
use smithay::input::keyboard::xkb::keysyms as xkb_keysyms;
use smithay::input::keyboard::{FilterResult, Keysym, ModifiersState};
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::reexports::wayland_server::protocol::{wl_pointer, wl_surface::WlSurface};
use smithay::utils::{Logical, Point, Size, SERIAL_COUNTER as SCOUNTER};
use tracing::{debug, info};

use crate::backend::Backend;
use crate::compositor::CompositorState;
use crate::lua::Action;
use ruster_shell::{Rect, WindowId};

/// Global mod for WM binds: Mod4 (Super/Logo).
pub const WM_MOD: u32 = 4;

/// True when the given keysym+modifier state should quit the compositor:
/// Super+Shift+q.
pub fn is_quit_keysym(keysym: Keysym, mods: &ModifiersState) -> bool {
    keysym == Keysym::q && mods.logo && mods.shift
}

/// The VT `Ctrl+Alt+F<n>` asks for, or `None` for any other key.
///
/// xkb turns `Ctrl+Alt+F1`..`F12` into the `XF86Switch_VT_1`..`_12` keysyms,
/// which occupy a contiguous range — matching the range keeps all twelve in one
/// line and does not depend on twelve constants being named as expected.
pub fn vt_switch_target(keysym: Keysym) -> Option<i32> {
    const FIRST: u32 = xkb_keysyms::KEY_XF86Switch_VT_1;
    const LAST: u32 = xkb_keysyms::KEY_XF86Switch_VT_12;
    let raw = keysym.raw();
    (FIRST..=LAST)
        .contains(&raw)
        .then(|| (raw - FIRST) as i32 + 1)
}

/// True when the given keysym+modifier should cycle active workspaces:
/// Super+t.
pub fn is_cycle_workspace(keysym: Keysym, mods: &ModifiersState) -> bool {
    keysym == Keysym::t && mods.logo && !mods.shift
}

/// Resolve the WM action bound to a key press: stringify the (raw, unshifted)
/// keysym, match it against the compositor's configured keybinds, and fall
/// back to the hardcoded Phase 0 defaults if nothing configured matched.
/// Pure, so it stays unit-testable without a live display.
///
/// The `keysym` is expected to be the *unmodified* level-0 keysym: modifier
/// state arrives separately in `mods`, so `Super+Shift+q` yields the raw `q`
/// keysym plus `shift` in `mods` rather than an uppercased `Q`.
pub fn resolve_wm_action(
    keybinds: &[(String, String)],
    mods: &ModifiersState,
    keysym: Keysym,
) -> Option<Action> {
    let key = keysym_get_name(keysym);
    keybinds
        .iter()
        .find_map(|(bind, action)| {
            Action::keybind_matches(bind, mods, &key)
                .then(|| Action::from_name(action))
                .flatten()
        })
        .or_else(|| {
            if is_quit_keysym(keysym, mods) {
                Some(Action::Quit)
            } else if is_cycle_workspace(keysym, mods) {
                Some(Action::CycleWorkspace)
            } else {
                None
            }
        })
}

/// Global position of a toplevel's frame. Phase 0 is fullscreen: every
/// toplevel covers the whole output from the origin, so no frame offset.
pub const TOPLEVEL_OFFSET: Point<f64, Logical> = Point::new(0.0, 0.0);

/// The window under `pointer`, and the origin of its tile, from a laid-out tree.
///
/// The origin is the half that is easy to get wrong. smithay derives the
/// client-visible position as `pointer - origin`, so a window tiled at x=937
/// that reports an origin of `(0, 0)` tells its client the pointer is 937px
/// further right than it is — every hover lands in the wrong place, and only for
/// the windows that are not at the origin. Phase 0 could return a constant
/// because there was only ever one window and it filled the output.
pub fn tile_under(
    geometry: &[(WindowId, Rect)],
    pointer: Point<f64, Logical>,
) -> Option<(WindowId, Point<f64, Logical>)> {
    // Searched from the back, because `geometry` is ordered bottom to top and a
    // click belongs to whatever is nearest the front. Read the other way a
    // floating window would be clicked *through* — the tiled window beneath it
    // comes first in the list and would match the same point.
    let (px, py) = (pointer.x.floor() as i32, pointer.y.floor() as i32);
    geometry
        .iter()
        .rev()
        .find(|(_, r)| r.contains(px, py))
        .map(|(id, r)| (*id, Point::from((r.x as f64, r.y as f64))))
}

/// Apply a resolved WM [`Action`] to the compositor lifecycle it owns. Only
/// the `running` flag lives here: `Quit` stops the compositor, while
/// `CycleWorkspace` needs the shell and renderer, so the caller dispatches it.
pub fn apply_action(action: &Action, running: &Arc<AtomicBool>) {
    match action {
        Action::Quit => running.store(false, Ordering::SeqCst),
        Action::CycleWorkspace => {}
    }
}

impl<B: Backend + 'static> CompositorState<B> {
    /// The primary output's logical size: its physical mode size divided by the
    /// current output scale. The fullscreen toplevel spans exactly this.
    fn logical_output_size(&self) -> Option<Size<i32, Logical>> {
        crate::backend::logical_output_size(self.backend_data.output())
    }

    /// The toplevel window the pointer is over, or `None`. [`tile_under`]
    /// supplies the containment test against the laid-out tree, so a location
    /// over no window matches nothing — and since the layout covers only the
    /// active workspace, a click can never reach a window that is not on
    /// screen. Answering from the geometry rather than from `shell.focus` is
    /// what lets a click recover a visible window when focus has gone stale.
    fn toplevel_under(&self, location: Point<f64, Logical>) -> Option<WindowId> {
        tile_under(&self.geometry(), location)
            .map(|(id, _)| id)
            .filter(|id| self.mapped.contains(id))
    }

    /// The surface under the pointer with its origin in global coordinates, or
    /// `None` when the pointer is over no mapped window of the active
    /// workspace.
    ///
    /// The focus tuple's second element is the surface's global origin: smithay
    /// subtracts it from the pointer location to derive the client-visible
    /// surface-local position, so the *local* position must not be returned
    /// here or every enter/motion would land at `(0,0)`.
    fn surface_under(
        &self,
        location: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        let (id, origin) = tile_under(&self.geometry(), location)?;
        if !self.mapped.contains(&id) {
            return None;
        }
        let toplevel = self.toplevels.get(&id)?;
        Some((toplevel.wl_surface().clone(), origin))
    }

    /// Route one backend input event to its handler. Generic over the input
    /// backend so `WinitInput` and `LibinputInputBackend` events both land
    /// here; the variants Phase 0 does not consume (touch, tablet, gestures,
    /// device add/remove) are deliberately dropped.
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event } => self.on_keyboard_key::<I>(event),
            InputEvent::PointerMotion { event } => self.on_pointer_motion::<I>(event),
            InputEvent::PointerMotionAbsolute { event } => {
                self.on_pointer_motion_absolute::<I>(event)
            }
            InputEvent::PointerButton { event } => self.on_pointer_button::<I>(event),
            InputEvent::PointerAxis { event } => self.on_pointer_axis::<I>(event),
            _ => {}
        }
    }

    /// Run a key through the seat keyboard, intercepting the keys the WM has
    /// bound and forwarding everything else to the focused client.
    ///
    /// Only `Action::Quit` on a press stops the compositor — cycling a
    /// workspace must not shut it down. Both press and release of a bound key
    /// are intercepted, so the client never sees half a chord.
    pub fn on_keyboard_key<I: InputBackend>(&mut self, event: I::KeyboardKeyEvent) {
        let keycode = event.key_code();
        let key_state = event.state();
        let serial = SCOUNTER.next_serial();
        let time = Event::time_msec(&event);
        let keyboard = self.keyboard.clone();
        let _ = keyboard.input::<(), _>(
            self,
            keycode,
            key_state,
            serial,
            time,
            |compositor, modifiers, handle| {
                // Use the raw (unshifted) keysym and the separate modifier
                // state so Super+Shift+q resolves to the `q` binding rather
                // than an uppercased `Q`.
                let keysym = handle
                    .raw_latin_sym_or_raw_current_sym()
                    .unwrap_or(Keysym::NoSymbol);
                // Every key, at debug level: when a binding does not fire on
                // real hardware this is the only way to see whether the keysym
                // or the modifier state is the one that disagreed.
                debug!(
                    key = %keysym_get_name(keysym),
                    modified = %keysym_get_name(handle.modified_sym()),
                    logo = modifiers.logo,
                    shift = modifiers.shift,
                    ctrl = modifiers.ctrl,
                    alt = modifiers.alt,
                    pressed = key_state == KeyState::Pressed,
                    "key",
                );

                // VT switching comes first and is never configurable: it is the
                // escape hatch, and a user who has bound themselves into a
                // corner still has to be able to get out.
                if key_state == KeyState::Pressed {
                    if let Some(vt) = vt_switch_target(keysym) {
                        compositor.backend_data.change_vt(vt);
                        return FilterResult::Intercept(());
                    }
                }

                match resolve_wm_action(&compositor.keybinds, modifiers, keysym) {
                    Some(Action::Quit) => {
                        if key_state == KeyState::Pressed {
                            info!("quit keybinding pressed, shutting down");
                            apply_action(&Action::Quit, &compositor.running);
                        }
                        FilterResult::Intercept(())
                    }
                    Some(Action::CycleWorkspace) => {
                        if key_state == KeyState::Pressed {
                            info!("workspace cycle keybinding");
                            compositor.cycle_workspace();
                        }
                        FilterResult::Intercept(())
                    }
                    None => FilterResult::Forward,
                }
            },
        );
    }

    /// Relative pointer motion, as emitted by a mouse under libinput. There is
    /// no absolute position in the event, so the seat's tracked location is
    /// advanced by the delta and clamped to the output — without the clamp the
    /// pointer would walk off the only output and never come back, since
    /// nothing else bounds it. winit never emits this variant (it reports
    /// absolute window coordinates), so this is the DRM pointer path.
    pub fn on_pointer_motion<I: InputBackend>(&mut self, event: I::PointerMotionEvent) {
        let size = self.logical_output_size().unwrap_or_default();
        let mut pos = self.pointer.current_location() + event.delta();
        pos.x = pos.x.clamp(0.0, (size.w as f64 - 1.0).max(0.0));
        pos.y = pos.y.clamp(0.0, (size.h as f64 - 1.0).max(0.0));
        let serial = SCOUNTER.next_serial();
        let time = Event::time_msec(&event);
        let pointer = self.pointer.clone();
        let under = self.surface_under(pos);
        pointer.motion(
            self,
            under,
            &MotionEvent {
                location: pos,
                serial,
                time,
            },
        );
        pointer.frame(self);
    }

    /// Absolute pointer motion: winit reports window coordinates this way, as
    /// do touchscreens and tablets under libinput. The event carries a
    /// normalized position, so it is scaled onto the output's logical size.
    pub fn on_pointer_motion_absolute<I: InputBackend>(
        &mut self,
        event: I::PointerMotionAbsoluteEvent,
    ) {
        let size = self.logical_output_size().unwrap_or_default();
        let pos = event.position_transformed(size);
        let serial = SCOUNTER.next_serial();
        let time = Event::time_msec(&event);
        let pointer = self.pointer.clone();
        let under = self.surface_under(pos);
        pointer.motion(
            self,
            under,
            &MotionEvent {
                location: pos,
                serial,
                time,
            },
        );
        pointer.frame(self);
    }

    /// Pointer button press/release, with click-to-focus on press.
    pub fn on_pointer_button<I: InputBackend>(&mut self, event: I::PointerButtonEvent) {
        debug!(button = event.button_code(), "pointer button");
        let serial = SCOUNTER.next_serial();
        let time = Event::time_msec(&event);
        let state = wl_pointer::ButtonState::from(event.state());
        if state == wl_pointer::ButtonState::Pressed {
            // Click-to-focus: focus the toplevel under the pointer. Button
            // events carry no position, so the seat pointer's tracked location
            // is used (anvil does the same via `pointer.current_location()`).
            // Focusing the window under the cursor — rather than re-asserting a
            // possibly stale `shell.focus` — lets a click always recover a
            // visible window when the previous focus unmapped.
            if let Some(id) = self
                .toplevel_under(self.pointer.current_location())
                .filter(|id| self.shell.focus != Some(*id))
            {
                self.shell.set_focus(id);
                self.update_keyboard_focus(serial);
            }
        }
        let pointer = self.pointer.clone();
        pointer.button(
            self,
            &ButtonEvent {
                serial,
                time,
                button: event.button_code(),
                state: state.try_into().unwrap(),
            },
        );
        pointer.frame(self);
    }

    /// Scroll: build an `AxisFrame` and hand it to the focused surface
    /// (anvil's `on_pointer_axis`). Sources that report only v120 discrete
    /// steps are converted to the smooth value smithay expects.
    pub fn on_pointer_axis<I: InputBackend>(&mut self, event: I::PointerAxisEvent) {
        let horizontal_amount = event
            .amount(Axis::Horizontal)
            .unwrap_or_else(|| event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.);
        let vertical_amount = event
            .amount(Axis::Vertical)
            .unwrap_or_else(|| event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.);

        let mut frame = AxisFrame::new(Event::time_msec(&event)).source(event.source());
        if horizontal_amount != 0.0 {
            frame = frame
                .relative_direction(Axis::Horizontal, event.relative_direction(Axis::Horizontal));
            frame = frame.value(Axis::Horizontal, horizontal_amount);
            if let Some(discrete) = event.amount_v120(Axis::Horizontal) {
                frame = frame.v120(Axis::Horizontal, discrete as i32);
            }
        }
        if vertical_amount != 0.0 {
            frame =
                frame.relative_direction(Axis::Vertical, event.relative_direction(Axis::Vertical));
            frame = frame.value(Axis::Vertical, vertical_amount);
            if let Some(discrete) = event.amount_v120(Axis::Vertical) {
                frame = frame.v120(Axis::Vertical, discrete as i32);
            }
        }
        if event.source() == AxisSource::Finger {
            if event.amount(Axis::Horizontal) == Some(0.0) {
                frame = frame.stop(Axis::Horizontal);
            }
            if event.amount(Axis::Vertical) == Some(0.0) {
                frame = frame.stop(Axis::Vertical);
            }
        }

        let pointer = self.pointer.clone();
        pointer.axis(self, frame);
        pointer.frame(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use smithay::backend::input::{Device, DeviceCapability, Keycode, UnusedEvent};
    use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
    use smithay::reexports::calloop::EventLoop;
    use smithay::reexports::wayland_server::Display;

    use crate::compositor::create_state;
    use ruster_shell::Layout;

    /// A [`Backend`] with no graphics at all: just the `Output` the input path
    /// needs for its logical-size lookups, plus a counter so a test can tell
    /// that a handler asked for a full redraw. An [`Output`] can be built
    /// without a display — only `create_global` needs one — so the whole
    /// compositor state comes up headless.
    struct TestBackend {
        output: Output,
        resets: u32,
    }

    impl Backend for TestBackend {
        fn seat_name(&self) -> String {
            String::from("ruster-test")
        }

        fn reset_buffers(&mut self, _output: &Output) {
            self.resets += 1;
        }

        fn output(&self) -> &Output {
            &self.output
        }
    }

    /// A synthetic [`InputBackend`]. Only keyboard events are constructible;
    /// every other event type is the uninhabited [`UnusedEvent`], which smithay
    /// already implements every event trait for. This is what lets a test drive
    /// `on_keyboard_key` — the same generic method libinput and winit call —
    /// rather than re-implementing its logic.
    #[derive(Debug)]
    struct TestInput;

    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    struct TestDevice;

    impl Device for TestDevice {
        fn id(&self) -> String {
            String::from("test-keyboard")
        }

        fn name(&self) -> String {
            String::from("test-keyboard")
        }

        fn has_capability(&self, capability: DeviceCapability) -> bool {
            capability == DeviceCapability::Keyboard
        }

        fn usb_id(&self) -> Option<(u32, u32)> {
            None
        }

        fn syspath(&self) -> Option<PathBuf> {
            None
        }
    }

    struct TestKeyEvent {
        keycode: u32,
        state: KeyState,
    }

    impl Event<TestInput> for TestKeyEvent {
        fn time(&self) -> u64 {
            0
        }

        fn device(&self) -> TestDevice {
            TestDevice
        }
    }

    impl KeyboardKeyEvent<TestInput> for TestKeyEvent {
        fn key_code(&self) -> Keycode {
            Keycode::new(self.keycode)
        }

        fn state(&self) -> KeyState {
            self.state
        }

        fn count(&self) -> u32 {
            1
        }
    }

    impl InputBackend for TestInput {
        type Device = TestDevice;
        type KeyboardKeyEvent = TestKeyEvent;
        type PointerAxisEvent = UnusedEvent;
        type PointerButtonEvent = UnusedEvent;
        type PointerMotionEvent = UnusedEvent;
        type PointerMotionAbsoluteEvent = UnusedEvent;
        type GestureSwipeBeginEvent = UnusedEvent;
        type GestureSwipeUpdateEvent = UnusedEvent;
        type GestureSwipeEndEvent = UnusedEvent;
        type GesturePinchBeginEvent = UnusedEvent;
        type GesturePinchUpdateEvent = UnusedEvent;
        type GesturePinchEndEvent = UnusedEvent;
        type GestureHoldBeginEvent = UnusedEvent;
        type GestureHoldEndEvent = UnusedEvent;
        type TouchDownEvent = UnusedEvent;
        type TouchUpEvent = UnusedEvent;
        type TouchMotionEvent = UnusedEvent;
        type TouchCancelEvent = UnusedEvent;
        type TouchFrameEvent = UnusedEvent;
        type TabletToolAxisEvent = UnusedEvent;
        type TabletToolProximityEvent = UnusedEvent;
        type TabletToolTipEvent = UnusedEvent;
        type TabletToolButtonEvent = UnusedEvent;
        type SwitchToggleEvent = UnusedEvent;
        type SpecialEvent = UnusedEvent;
    }

    // XKB keycodes are evdev codes offset by 8, the same translation
    // `LibinputInputBackend::key_code` applies, so these are exactly what real
    // hardware would deliver on a US layout.
    const KEY_Q: u32 = 16 + 8;
    const KEY_T: u32 = 20 + 8;
    const KEY_A: u32 = 30 + 8;
    const KEY_LEFTSHIFT: u32 = 42 + 8;
    const KEY_LEFTMETA: u32 = 125 + 8;

    /// A headless compositor state with the Phase 0 default keybinds installed.
    /// The [`EventLoop`] is returned alongside because `create_state` registers
    /// the display as one of its sources; dropping it would tear that down.
    fn test_state() -> (
        EventLoop<'static, CompositorState<TestBackend>>,
        CompositorState<TestBackend>,
    ) {
        let event_loop: EventLoop<'static, CompositorState<TestBackend>> =
            EventLoop::try_new().expect("failed to create test event loop");
        let display: Display<CompositorState<TestBackend>> =
            Display::new().expect("failed to create test display");
        let output = Output::new(
            "test".to_string(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "Ruster".into(),
                model: "Test".into(),
            },
        );
        output.change_current_state(
            Some(Mode {
                size: (800, 600).into(),
                refresh: 60_000,
            }),
            None,
            None,
            Some((0, 0).into()),
        );
        let mut state = create_state(
            display,
            event_loop.handle(),
            TestBackend { output, resets: 0 },
        );
        state.keybinds = vec![
            ("M-S-q".into(), "quit".into()),
            ("M-t".into(), "cycle workspace".into()),
        ];
        (event_loop, state)
    }

    /// Press and release a key through the real seat keyboard. Modifiers are
    /// held down by the caller rather than synthesized, because the filter
    /// reads xkb's live modifier state, not a value we could pass in.
    fn press(state: &mut CompositorState<TestBackend>, keycode: u32) {
        state.on_keyboard_key::<TestInput>(TestKeyEvent {
            keycode,
            state: KeyState::Pressed,
        });
    }

    fn release(state: &mut CompositorState<TestBackend>, keycode: u32) {
        state.on_keyboard_key::<TestInput>(TestKeyEvent {
            keycode,
            state: KeyState::Released,
        });
    }

    #[test]
    fn super_shift_q_through_the_seat_stops_the_compositor() {
        // The end-to-end keybinding path: a keycode goes into
        // `KeyboardHandle::input`, xkb resolves it to a keysym plus modifier
        // state, the filter resolves that to `Action::Quit` and applies it.
        let (_loop, mut state) = test_state();
        assert!(state.running.load(Ordering::SeqCst));

        press(&mut state, KEY_LEFTMETA);
        press(&mut state, KEY_LEFTSHIFT);
        press(&mut state, KEY_Q);

        assert!(
            !state.running.load(Ordering::SeqCst),
            "Super+Shift+q should stop the compositor"
        );
    }

    #[test]
    fn super_t_through_the_seat_cycles_the_workspace_without_quitting() {
        let (_loop, mut state) = test_state();
        let before = state.workspaces.active();

        press(&mut state, KEY_LEFTMETA);
        press(&mut state, KEY_T);
        release(&mut state, KEY_T);

        assert_eq!(
            state.workspaces.active(),
            before + 1,
            "Super+t should advance the workspace"
        );
        assert!(
            state.running.load(Ordering::SeqCst),
            "cycling a workspace must not stop the compositor"
        );
        // The press asks for a full redraw so the new workspace is drawn; the
        // release is intercepted too, and must not ask again.
        assert_eq!(state.backend_data.resets, 1);
    }

    /// Give the compositor a window on the active workspace, mapped and
    /// focused, without a client to map it. Everything the visibility paths
    /// read — the tree, `mapped`, `shell.focus` — is reachable from here; only
    /// the toplevel surface is missing, and no assertion below needs one.
    fn add_visible_window(state: &mut CompositorState<TestBackend>, title: &str) -> WindowId {
        let id = state.shell.add_window(title.into(), 100, 100);
        let near = state.shell.focus;
        state.workspaces.insert(id, near, Layout::Horizontal);
        state.mapped.insert(id);
        state.shell.set_focus(id);
        id
    }

    #[test]
    fn cycling_the_workspace_takes_the_windows_off_the_screen_with_it() {
        // What Phase 0's counter never did: after M-t the windows of the
        // workspace left behind have no rectangle, so nothing draws them and a
        // click lands on nothing. They are hidden, not closed — cycling all the
        // way round brings them back.
        let (_loop, mut state) = test_state();
        let a = add_visible_window(&mut state, "a");
        assert_eq!(state.geometry().len(), 1);

        press(&mut state, KEY_LEFTMETA);
        press(&mut state, KEY_T);
        release(&mut state, KEY_T);

        assert_eq!(state.workspaces.active(), 2);
        assert!(state.geometry().is_empty(), "workspace 2 holds no windows");
        assert_eq!(
            tile_under(&state.geometry(), Point::from((10.0, 10.0))),
            None,
            "a click cannot reach a window that is not on screen"
        );
        assert_eq!(
            state.shell.focus, None,
            "focus does not stay on a window that left the screen"
        );

        for _ in 0..(ruster_shell::WORKSPACE_COUNT - 1) {
            press(&mut state, KEY_T);
            release(&mut state, KEY_T);
        }
        assert_eq!(state.workspaces.active(), 1);
        assert_eq!(state.geometry().len(), 1, "the window is still there");
        assert_eq!(state.shell.focus, Some(a), "and takes focus back");
    }

    #[test]
    fn moving_a_window_away_leaves_focus_on_one_that_is_still_shown() {
        let (_loop, mut state) = test_state();
        let a = add_visible_window(&mut state, "a");
        let b = add_visible_window(&mut state, "b");

        state.move_to_workspace(b, 3);

        let ids: Vec<_> = state.geometry().into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, vec![a], "only the window still here is laid out");
        assert_eq!(state.shell.focus, Some(a));
        assert_eq!(state.workspaces.workspace_of(b), Some(3));

        state.switch_workspace(3);
        let ids: Vec<_> = state.geometry().into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, vec![b]);
        assert_eq!(state.shell.focus, Some(b));
    }

    #[test]
    fn unbound_key_is_forwarded_and_changes_nothing() {
        // Super+a matches no bind, so the filter returns `Forward`: the key
        // goes to the focused client (there is none here) and no compositor
        // state moves.
        let (_loop, mut state) = test_state();
        let workspace = state.workspaces.active();

        press(&mut state, KEY_LEFTMETA);
        press(&mut state, KEY_A);
        release(&mut state, KEY_A);

        assert!(state.running.load(Ordering::SeqCst));
        assert_eq!(state.workspaces.active(), workspace);
        assert_eq!(
            state.backend_data.resets, 0,
            "an unbound key must not trigger a redraw"
        );
    }

    #[test]
    fn q_without_the_modifiers_is_not_the_quit_binding() {
        // Guards the modifier half of the chord: the same keycode that quits
        // with Super+Shift held must be forwarded on its own.
        let (_loop, mut state) = test_state();

        press(&mut state, KEY_Q);
        release(&mut state, KEY_Q);

        assert!(state.running.load(Ordering::SeqCst));
    }

    #[test]
    fn wm_key_quit_sets_running_false() {
        // cannot construct CompositorState without a display; test the pure
        // decision instead: Action::Quit flips the running flag off.
        let running = Arc::new(AtomicBool::new(true));
        apply_action(&Action::Quit, &running);
        assert!(!running.load(Ordering::Relaxed));
    }

    #[test]
    fn wm_key_cycle_workspace_keeps_running() {
        // Cycling a workspace must not shut the compositor down.
        let running = Arc::new(AtomicBool::new(true));
        apply_action(&Action::CycleWorkspace, &running);
        assert!(running.load(Ordering::Relaxed));
    }

    #[test]
    fn keysym_quit_binding_recognized() {
        // Mod4+Shift+q → quit
        let keysym = Keysym::q;
        let mods = ModifiersState {
            alt: false,
            ctrl: false,
            logo: true,
            shift: true,
            ..Default::default()
        };
        assert!(is_quit_keysym(keysym, &mods));
    }

    #[test]
    fn workspace_cycle_binding_recognized() {
        // Mod4+t → cycle workspace (no shift)
        let keysym = Keysym::t;
        let mods = ModifiersState {
            alt: false,
            ctrl: false,
            logo: true,
            shift: false,
            ..Default::default()
        };
        assert!(is_cycle_workspace(keysym, &mods));
    }

    #[test]
    fn workspace_cycle_binding_ignores_shift_and_non_logo() {
        assert!(!is_cycle_workspace(
            Keysym::t,
            &ModifiersState {
                logo: true,
                shift: true,
                ..Default::default()
            }
        ));
        assert!(!is_cycle_workspace(
            Keysym::t,
            &ModifiersState {
                logo: false,
                shift: false,
                ..Default::default()
            }
        ));
        assert!(!is_cycle_workspace(
            Keysym::q,
            &ModifiersState {
                logo: true,
                ..Default::default()
            }
        ));
    }

    #[test]
    fn resolve_quit_and_cycle_from_configured_keybinds() {
        let keybinds = vec![
            ("M-S-q".into(), "quit".into()),
            ("M-t".into(), "cycle workspace".into()),
        ];
        let mods = ModifiersState {
            logo: true,
            shift: true,
            ..Default::default()
        };
        assert_eq!(
            resolve_wm_action(&keybinds, &mods, Keysym::q),
            Some(Action::Quit)
        );
        let cycle_mods = ModifiersState {
            logo: true,
            ..Default::default()
        };
        assert_eq!(
            resolve_wm_action(&keybinds, &cycle_mods, Keysym::t),
            Some(Action::CycleWorkspace)
        );
    }

    fn geom() -> Vec<(WindowId, Rect)> {
        // Two tiles side by side across a 1000x800 output.
        vec![
            (WindowId(0), Rect::new(0, 0, 500, 800)),
            (WindowId(1), Rect::new(500, 0, 500, 800)),
        ]
    }

    #[test]
    fn the_pointer_picks_the_tile_it_is_over() {
        assert_eq!(
            tile_under(&geom(), Point::from((10.0, 10.0))).map(|(id, _)| id),
            Some(WindowId(0))
        );
        assert_eq!(
            tile_under(&geom(), Point::from((600.0, 10.0))).map(|(id, _)| id),
            Some(WindowId(1))
        );
        assert_eq!(tile_under(&geom(), Point::from((1200.0, 10.0))), None);
        assert_eq!(tile_under(&geom(), Point::from((-1.0, 10.0))), None);
    }

    #[test]
    fn the_origin_is_the_tile_corner_not_the_output_corner() {
        // The bug this exists to prevent: smithay derives the client-visible
        // position as `pointer - origin`, so a window tiled at x=500 that
        // reports an origin of (0, 0) tells its client the pointer is 500px
        // further right than it is. It only goes wrong for windows away from
        // the origin, which is why one fullscreen window never showed it.
        let (_, origin) = tile_under(&geom(), Point::from((600.0, 40.0))).unwrap();
        assert_eq!(origin, Point::from((500.0, 0.0)));

        let local = Point::from((600.0, 40.0)) - origin;
        assert_eq!(
            local,
            Point::from((100.0, 40.0)),
            "100px into the right tile"
        );
    }

    #[test]
    fn a_shared_tile_edge_belongs_to_one_window() {
        // Tiles touch. If both claimed x=500 the window a click focused would
        // depend on iteration order.
        assert_eq!(
            tile_under(&geom(), Point::from((499.9, 400.0))).map(|(id, _)| id),
            Some(WindowId(0))
        );
        assert_eq!(
            tile_under(&geom(), Point::from((500.0, 400.0))).map(|(id, _)| id),
            Some(WindowId(1))
        );
    }

    #[test]
    fn a_click_lands_on_the_float_not_through_it() {
        // A floating window overlaps the tiled one beneath it. The geometry
        // slice is ordered bottom to top, so reading it forwards finds the
        // tiled window first and the click goes straight through the float —
        // which is what this pins.
        let geometry = vec![
            (WindowId(0), Rect::new(0, 0, 1000, 800)),
            (WindowId(1), Rect::new(100, 100, 200, 200)),
        ];
        assert_eq!(
            tile_under(&geometry, Point::from((150.0, 150.0))).map(|(id, _)| id),
            Some(WindowId(1)),
            "the float owns the overlap"
        );
        assert_eq!(
            tile_under(&geometry, Point::from((600.0, 400.0))).map(|(id, _)| id),
            Some(WindowId(0)),
            "and the tiled window owns everywhere else"
        );
        // The float's origin, not the output's, or its client is told the
        // pointer is 100px up and left of where it is.
        let (_, origin) = tile_under(&geometry, Point::from((150.0, 150.0))).unwrap();
        assert_eq!(origin, Point::from((100.0, 100.0)));
    }

    #[test]
    fn an_empty_layout_is_over_nothing() {
        assert_eq!(tile_under(&[], Point::from((10.0, 10.0))), None);
    }

    #[test]
    fn vt_switch_keysyms_map_to_their_terminal() {
        assert_eq!(
            vt_switch_target(Keysym::new(xkb_keysyms::KEY_XF86Switch_VT_1)),
            Some(1)
        );
        assert_eq!(
            vt_switch_target(Keysym::new(xkb_keysyms::KEY_XF86Switch_VT_3)),
            Some(3)
        );
        assert_eq!(
            vt_switch_target(Keysym::new(xkb_keysyms::KEY_XF86Switch_VT_12)),
            Some(12)
        );
        // Ordinary keys are not VT switches — in particular the WM binds, which
        // are resolved after this check and must still reach it.
        assert_eq!(vt_switch_target(Keysym::q), None);
        assert_eq!(vt_switch_target(Keysym::F1), None);
        assert_eq!(vt_switch_target(Keysym::NoSymbol), None);
    }

    #[test]
    fn a_config_can_bind_its_own_key_to_its_own_action() {
        // Both halves of a configured pair have to matter. Until the bind
        // parser landed, `resolve_wm_action` matched two hardcoded strings and
        // threw the action name away, so a config could express nothing the
        // compositor did not already know.
        let keybinds = vec![
            ("M-F9".into(), "cycle workspace".into()),
            ("C-A-F10".into(), "quit".into()),
        ];
        let logo = ModifiersState {
            logo: true,
            ..Default::default()
        };
        assert_eq!(
            resolve_wm_action(&keybinds, &logo, Keysym::F9),
            Some(Action::CycleWorkspace),
            "a key the compositor has no built-in knowledge of"
        );
        let ctrl_alt = ModifiersState {
            ctrl: true,
            alt: true,
            ..Default::default()
        };
        assert_eq!(
            resolve_wm_action(&keybinds, &ctrl_alt, Keysym::F10),
            Some(Action::Quit)
        );
        // An action name the compositor does not know binds nothing, rather
        // than falling through to some default.
        let unknown = vec![("M-F9".into(), "launch nukes".into())];
        assert_eq!(resolve_wm_action(&unknown, &logo, Keysym::F9), None);
    }

    #[test]
    fn resolve_falls_back_to_defaults_when_unbound() {
        let keybinds = vec![("M-S-q".into(), "quit".into())];
        // Super+Tab is no longer a WM bind (the cycle key is M-t now), so an
        // unconfigured press resolves to nothing.
        let tab_mods = ModifiersState {
            logo: true,
            ..Default::default()
        };
        assert_eq!(resolve_wm_action(&keybinds, &tab_mods, Keysym::Tab), None);
        assert_eq!(
            resolve_wm_action(&keybinds, &tab_mods, Keysym::NoSymbol),
            None
        );
    }
}
