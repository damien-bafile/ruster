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
//! Every binding comes from `compositor.lua`, resolved through
//! [`Keymap`](crate::keymap::Keymap) into an [`Action`] that
//! [`CompositorState::dispatch`](crate::compositor::CompositorState::dispatch)
//! carries out. The one exception is `M-S-q`, which quits regardless of the
//! config — see [`is_quit_keysym`] for why that is worth the inconsistency.

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisSource, Event, InputBackend, InputEvent, KeyState,
    KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
};
use smithay::input::keyboard::xkb::keysym_get_name;
use smithay::input::keyboard::xkb::keysyms as xkb_keysyms;
use smithay::input::keyboard::{FilterResult, Keysym, ModifiersState};
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::reexports::wayland_server::protocol::{wl_pointer, wl_surface::WlSurface};
use smithay::utils::{Logical, Physical, Point, Size, SERIAL_COUNTER as SCOUNTER};
use std::time::Instant;

use tracing::{debug, trace};

use crate::backend::Backend;
use crate::compositor::CompositorState;
use smithay::desktop::PopupManager;

use ruster_core::key::{Arrow, KeyEvent};

use crate::keymap::{ChordState, Resolved};
use crate::lua::Action;
use crate::repeat::RepeatTarget;
use ruster_shell::{Rect, WindowId};

/// True when the given keysym+modifier state should quit the compositor:
/// Super+Shift+q.
///
/// This is a deliberate escape hatch, not a default: it fires whatever the
/// config says, because on DRM the compositor *is* the display server and a
/// config that binds no working quit leaves no way out short of another machine
/// or a power button. It cost a real session once already.
pub fn is_quit_keysym(keysym: Keysym, mods: &ModifiersState) -> bool {
    keysym == Keysym::q && mods.logo && mods.shift
}

/// The VT `Ctrl+Alt+F<n>` asks for, or `None` for any other key.
///
/// Must be given the **modified** keysym. xkb maps `Ctrl+Alt+F2` to
/// `XF86Switch_VT_2` only after the modifiers are applied; the raw sym stays
/// `F2` and matches nothing here. Passing the raw one compiles, type-checks,
/// and silently disables VT switching entirely — which is what it did.
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

/// An xkb keypress as the editor's [`KeyEvent`], or `None` for keys the editor
/// has no vocabulary for.
///
/// A third sibling of the editor's crossterm and raylib key tables. The source
/// vocabularies genuinely differ — xkb keysyms, crossterm codes, raylib enums —
/// so what is shared is the target, which already is.
///
/// `modified` carries the shifted symbol, which is the character actually typed;
/// reading text off the raw keysym would make every capital lowercase. The raw
/// one identifies the editing keys, which do not shift.
pub fn editor_key(raw: Keysym, modified: Keysym, mods: &ModifiersState) -> Option<KeyEvent> {
    use smithay::input::keyboard::keysyms as ks;
    let arrow = match raw.raw() {
        ks::KEY_Left => Some(Arrow::Left),
        ks::KEY_Right => Some(Arrow::Right),
        ks::KEY_Up => Some(Arrow::Up),
        ks::KEY_Down => Some(Arrow::Down),
        _ => None,
    };
    if let Some(arrow) = arrow {
        return Some(KeyEvent::Arrow(arrow));
    }
    match raw.raw() {
        ks::KEY_Escape => return Some(KeyEvent::Esc),
        ks::KEY_Return | ks::KEY_KP_Enter => return Some(KeyEvent::Enter),
        ks::KEY_BackSpace => return Some(KeyEvent::Backspace),
        ks::KEY_Delete => return Some(KeyEvent::Delete),
        ks::KEY_Tab => {
            return Some(if mods.shift {
                KeyEvent::BackTab
            } else {
                KeyEvent::Tab
            })
        }
        ks::KEY_ISO_Left_Tab => return Some(KeyEvent::BackTab),
        _ => {}
    }
    // Ctrl before the character: `Ctrl('d')` is a half-page scroll, while the
    // character it would otherwise produce is a control code nobody types.
    if mods.ctrl {
        return raw.key_char().map(KeyEvent::Ctrl);
    }
    if mods.alt {
        return raw.key_char().map(KeyEvent::Alt);
    }
    modified
        .key_char()
        .filter(|c| !c.is_control())
        .map(KeyEvent::Char)
}

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

/// How many buffer lines one scroll event moves a pane, signed the way the
/// wheel is: positive is towards the end of the buffer.
///
/// The amount arrives in the smooth units libinput reports, where one wheel
/// detent is 15.0; three lines a detent is what every editor does. Any nonzero
/// amount moves at least one line, because a touchpad reports fractions of a
/// detent and rounding those to zero would leave a pane that simply refuses to
/// scroll under a finger.
pub fn scroll_lines(amount: f64) -> i64 {
    const DETENT: f64 = 15.0;
    const LINES_PER_DETENT: f64 = 3.0;
    let lines = amount / DETENT * LINES_PER_DETENT;
    lines.abs().ceil().copysign(lines) as i64
}

impl<B: Backend + 'static> CompositorState<B> {
    /// The primary output's logical size: its physical mode size divided by the
    /// current output scale. The fullscreen toplevel spans exactly this.
    fn logical_output_size(&self) -> Option<Size<i32, Logical>> {
        crate::backend::logical_output_size(self.backend_data.output())
    }

    /// The leaf the pointer is over and the origin of its tile, or `None`.
    /// [`tile_under`] supplies the containment test against the laid-out tree,
    /// so a location over no leaf matches nothing — and since the layout covers
    /// only the active workspace, a click can never reach a window that is not
    /// on screen. Answering from the geometry rather than from `shell.focus` is
    /// what lets a click recover a visible window when focus has gone stale.
    ///
    /// A leaf is a client or a pane; an id that is neither is a toplevel with
    /// nothing on screen yet, and clicking one must not move focus onto it. The
    /// origin comes back too because a click into a pane has to know where the
    /// pane starts, and hit-testing a second time for it is exactly how two
    /// answers about one rectangle start to disagree.
    fn leaf_under(&self, location: Point<f64, Logical>) -> Option<(WindowId, Point<f64, Logical>)> {
        tile_under(&self.geometry(), location)
            .filter(|(id, _)| self.mapped.contains(id) || self.panes.contains_key(id))
    }

    /// Physical pixels from the top-left of the tile at `origin` — the
    /// coordinate space a pane's frame is drawn in. Chrome is measured in
    /// physical pixels and the layout in logical ones; this is the conversion
    /// `draw_window_borders` and the pane renderer both do.
    fn frame_local(
        &self,
        location: Point<f64, Logical>,
        origin: Point<f64, Logical>,
    ) -> Point<f64, Physical> {
        let scale = self
            .backend_data
            .output()
            .current_scale()
            .fractional_scale();
        (location - origin).to_physical(scale)
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
        let parent = toplevel.wl_surface().clone();

        // A popup over this window takes the pointer before the window does.
        // Without this a menu is visible but unclickable — every click falls
        // through to the toplevel behind it, which is what a client reads as
        // "the user dismissed the menu and clicked over there".
        for (popup, offset) in PopupManager::popups_for_surface(&parent) {
            let geo = popup.geometry();
            let popup_origin = origin + (offset - geo.loc).to_f64();
            let rect = Rect::new(
                popup_origin.x.round() as i32,
                popup_origin.y.round() as i32,
                geo.size.w,
                geo.size.h,
            );
            if rect.contains(location.x.floor() as i32, location.y.floor() as i32) {
                return Some((popup.wl_surface().clone(), popup_origin));
            }
        }
        Some((parent, origin))
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

                // Repeat is settled before anything decides what the key means,
                // because every route out of this closure has to agree on it: a
                // press supersedes whatever was repeating, and a release stops
                // the key it let go of. A repeat that outlives its keypress
                // types forever, and on DRM there is no second window to kill
                // it from. See [`crate::repeat`].
                if key_state == KeyState::Pressed {
                    compositor.cancel_repeat();
                } else {
                    compositor.release_key(keycode.raw());
                }

                // VT switching comes first and is never configurable: it is the
                // escape hatch, and a user who has bound themselves into a
                // corner still has to be able to get out.
                if key_state == KeyState::Pressed {
                    // The *modified* sym: xkb only produces `XF86Switch_VT_n`
                    // once Ctrl+Alt are applied, so the raw sym is plain `F2`
                    // and never matches. Testing the raw one is why this never
                    // fired on hardware.
                    if let Some(vt) = vt_switch_target(handle.modified_sym()) {
                        compositor.backend_data.change_vt(vt);
                        return FilterResult::Intercept(());
                    }
                }

                // An open prompt takes every key, before the keymap and before
                // the quit hatch. Letting anything through would type the
                // command into the focused terminal at the same time — and a
                // keybind firing mid-line would act on a half-typed thought.
                if compositor
                    .minibuffer
                    .as_ref()
                    .is_some_and(|mb| mb.is_open())
                {
                    if key_state == KeyState::Pressed {
                        compositor.hold_key(
                            keycode.raw(),
                            RepeatTarget::Prompt {
                                raw: keysym,
                                modified: handle.modified_sym(),
                            },
                        );
                    }
                    compositor.intercepted.insert(keycode.raw());
                    return FilterResult::Intercept(());
                }

                // Releases follow whatever their press did. Resolution is a
                // press-time decision and the pending sequence has moved on by
                // now, so asking again would answer differently.
                if key_state != KeyState::Pressed {
                    return if compositor.intercepted.remove(&keycode.raw()) {
                        FilterResult::Intercept(())
                    } else {
                        FilterResult::Forward
                    };
                }

                // An abandoned prefix stops eating keys after a second.
                compositor.chord.expire(Instant::now());

                let key = keysym_get_name(keysym);
                let candidates = compositor.keymap.continuations(compositor.chord.pending());
                let outcome =
                    compositor
                        .keymap
                        .resolve(compositor.chord.pending(), modifiers, &key);

                // The quit escape hatch is checked against a *fresh* key rather
                // than mid-sequence: `M-S-q` must work when nothing is pending,
                // whatever the config says, and must not hijack a sequence that
                // legitimately contains it.
                if outcome == Resolved::None
                    && !compositor.chord.is_active()
                    && is_quit_keysym(keysym, modifiers)
                {
                    compositor.dispatch(Action::Quit);
                    compositor.intercepted.insert(keycode.raw());
                    return FilterResult::Intercept(());
                }

                match outcome {
                    Resolved::Action(action) => {
                        compositor.chord.clear();
                        compositor.dispatch(action);
                        compositor.intercepted.insert(keycode.raw());
                        FilterResult::Intercept(())
                    }
                    Resolved::Pending => {
                        if let Some(spec) = ChordState::matching_spec(&candidates, modifiers, &key)
                        {
                            compositor.chord.push(spec);
                        }
                        compositor.intercepted.insert(keycode.raw());
                        FilterResult::Intercept(())
                    }
                    // A key that continues nothing abandons the sequence. It is
                    // swallowed rather than forwarded, because a client
                    // receiving the tail of a mistyped chord is worse than one
                    // missing a keystroke the user did not mean for it.
                    Resolved::None if compositor.chord.is_active() => {
                        compositor.chord.clear();
                        compositor.intercepted.insert(keycode.raw());
                        FilterResult::Intercept(())
                    }
                    // A focused editor pane takes the key — but only here, after
                    // VT switching, the mini-buffer, chord resolution and the
                    // quit hatch. An editor that swallowed `M-S-q` on a DRM
                    // boot would be the worst bug in this project.
                    Resolved::None if compositor.pane_has_focus() => {
                        if let Some((window, key)) = compositor.shell.focus.zip(editor_key(
                            keysym,
                            handle.modified_sym(),
                            modifiers,
                        )) {
                            // Named rather than left to `pane_key`'s own lookup
                            // of the focused leaf, so a repeat still going when
                            // focus moves is aimed at the pane that was typed
                            // into and not at whatever replaced it.
                            compositor.hold_key(keycode.raw(), RepeatTarget::Pane { window, key });
                        }
                        compositor.intercepted.insert(keycode.raw());
                        FilterResult::Intercept(())
                    }
                    Resolved::None => FilterResult::Forward,
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
        let delta = event.delta();
        // Relative motion is the DRM-only path — winit only ever sends absolute
        // — and nothing logged it, so a hardware boot could not report whether
        // the mouse had done anything at all. Trace level, because a moving
        // pointer would drown a debug log the way the paused render loop did.
        trace!(dx = delta.x, dy = delta.y, "pointer motion");
        let mut pos = self.pointer.current_location() + delta;
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

    /// Pointer button press/release, with click-to-focus on press — and, over
    /// an editor pane, click-to-position.
    pub fn on_pointer_button<I: InputBackend>(&mut self, event: I::PointerButtonEvent) {
        debug!(button = event.button_code(), "pointer button");
        let serial = SCOUNTER.next_serial();
        let time = Event::time_msec(&event);
        let state = wl_pointer::ButtonState::from(event.state());
        if state == wl_pointer::ButtonState::Pressed {
            // Click-to-focus: focus the leaf under the pointer. Button events
            // carry no position, so the seat pointer's tracked location is used
            // (anvil does the same via `pointer.current_location()`). Focusing
            // the window under the cursor — rather than re-asserting a possibly
            // stale `shell.focus` — lets a click always recover a visible
            // window when the previous focus unmapped.
            let location = self.pointer.current_location();
            if let Some((id, origin)) = self.leaf_under(location) {
                if self.shell.focus != Some(id) {
                    self.shell.set_focus(id);
                    self.update_keyboard_focus(serial);
                }
                // A pane has no client to forward the press to, so the click
                // means what it means in an editor: put the caret on the
                // character it landed on. One hit-test does both — the tile it
                // focused is the tile whose origin the caret is measured from.
                let local = self.frame_local(location, origin);
                // The text comes from the store rather than the pane, which now
                // holds only a handle to it. `panes` and `buffers` are distinct
                // fields, so the mutable and immutable borrows are disjoint.
                if let Some(pane) = self.panes.get_mut(&id) {
                    if let Some(doc) = self.buffers.get(pane.doc) {
                        pane.click_at(&doc.buffer, local.x as f32, local.y as f32);
                    }
                }
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

        // A pane scrolls itself. It is not a client, so there is no surface for
        // the axis frame to reach and the wheel would otherwise do nothing at
        // all over one. Horizontal scroll is dropped with it: a pane does not
        // scroll sideways, and passing it on would send it to whichever client
        // last held the pointer.
        if let Some((id, _)) = self.leaf_under(self.pointer.current_location()) {
            if let Some(pane) = self.panes.get_mut(&id) {
                if let Some(doc) = self.buffers.get(pane.doc) {
                    pane.scroll_by(&doc.buffer, scroll_lines(vertical_amount));
                }
                return;
            }
        }

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
    use std::sync::atomic::Ordering;

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
        /// VTs the compositor asked to switch to, in order.
        vt_switches: Vec<i32>,
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

        fn change_vt(&mut self, vt: i32) {
            self.vt_switches.push(vt);
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
    const KEY_LEFTCTRL: u32 = 29 + 8;
    const KEY_LEFTALT: u32 = 56 + 8;
    const KEY_F2: u32 = 60 + 8;

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
            TestBackend {
                output,
                resets: 0,
                vt_switches: Vec::new(),
            },
        );
        state.keymap = crate::keymap::Keymap::new(&[
            ("M-S-q".into(), "quit".into()),
            ("M-t".into(), "cycle workspace".into()),
        ]);
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
    fn activation_brings_a_window_back_from_a_hidden_workspace() {
        // What `foot`'s `bell.urgent` and a second browser invocation both ask
        // for. Honouring it only for windows already on screen would make the
        // request a no-op in the one case it exists for.
        let (_loop, mut state) = test_state();
        let here = add_visible_window(&mut state, "here");
        state.switch_workspace(4);
        // Two of them, and the activated one is deliberately *not* the one a
        // bare workspace switch would land on — otherwise the switch alone
        // would make the assertion below pass and the focus never be tested.
        let wanted = add_visible_window(&mut state, "wanted");
        let neighbour = add_visible_window(&mut state, "neighbour");
        state.switch_workspace(1);
        assert_eq!(state.shell.focus, Some(here));
        // Prove the discrimination: arriving on workspace 4 any other way puts
        // the keyboard on the neighbour, so an activation that only switched
        // and never focused would leave it there.
        state.switch_workspace(4);
        assert_eq!(state.shell.focus, Some(neighbour));
        state.switch_workspace(1);

        state.activate_window(wanted);

        assert_eq!(
            state.workspaces.active(),
            4,
            "the screen followed the window"
        );
        assert_eq!(state.shell.focus, Some(wanted), "and it took the keyboard");
        assert_ne!(state.shell.focus, Some(neighbour));
    }

    #[test]
    fn activation_of_a_window_with_no_buffer_leaves_the_screen_alone() {
        // A toplevel that has never committed cannot hold the keyboard, so
        // switching to its workspace would strand the user looking at nothing.
        let (_loop, mut state) = test_state();
        let here = add_visible_window(&mut state, "here");
        let ghost = state.shell.add_window("ghost".into(), 100, 100);
        state.workspaces.insert(ghost, None, Layout::Horizontal);
        state.move_to_workspace(ghost, 4);

        state.activate_window(ghost);

        assert_eq!(state.workspaces.active(), 1, "still on the same workspace");
        assert_eq!(state.shell.focus, Some(here));
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

    /// Resolve a single chord the way the key handler does, so these tests
    /// exercise the real keymap rather than a lookalike beside it.
    ///
    /// `resolve_wm_action` used to be that lookalike: once the chord machine
    /// took over the handler, it was reachable only from here — the same
    /// position `apply_action` was deleted from, and for the same reason.
    fn resolve_one(
        keybinds: &[(String, String)],
        mods: &ModifiersState,
        keysym: Keysym,
    ) -> Option<Action> {
        match crate::keymap::Keymap::new(keybinds).resolve(&[], mods, &keysym_get_name(keysym)) {
            Resolved::Action(action) => Some(action),
            _ => None,
        }
    }

    #[test]
    fn ctrl_alt_f2_through_the_seat_asks_for_the_vt() {
        // The escape hatch, driven the way a keyboard drives it. The old test
        // called `vt_switch_target(Keysym::XF86Switch_VT_2)` directly — an
        // input the real code never produced, because it passed the *raw* sym
        // (`F2`). It passed for months while VT switching did not work at all,
        // on the one path where being unable to switch away means a black
        // screen and no way back.
        let (_loop, mut state) = test_state();
        // Ctrl and Alt held, so xkb resolves F2 to the VT sym.
        press(&mut state, KEY_LEFTCTRL);
        press(&mut state, KEY_LEFTALT);
        press(&mut state, KEY_F2);
        assert_eq!(
            state.backend_data.vt_switches,
            vec![2],
            "Ctrl+Alt+F2 must ask the session for VT 2"
        );
    }

    #[test]
    fn f2_on_its_own_is_not_a_vt_switch() {
        // Without the modifiers xkb yields plain `F2` and nothing should
        // happen — otherwise a function key would throw you off the session.
        let (_loop, mut state) = test_state();
        press(&mut state, KEY_F2);
        assert!(state.backend_data.vt_switches.is_empty());
    }

    #[test]
    fn releasing_a_key_stops_it_repeating() {
        // The dangerous one. A repeat that outlives its key press types
        // forever, and on a DRM boot that is unrecoverable short of the quit
        // hatch.
        let (_loop, mut state) = test_state();
        state.open_pane();
        let id = state.shell.focus.unwrap();
        state.panes.get_mut(&id).unwrap().rows = 10;

        press(&mut state, KEY_A); // append: enters insert mode
        press(&mut state, KEY_Q);
        assert!(state.is_repeating(), "a held key should be repeating");
        release(&mut state, KEY_Q);
        assert!(!state.is_repeating(), "releasing it must stop the repeat");
    }

    #[test]
    fn a_repeat_does_not_outlive_the_pane_it_was_aimed_at() {
        // A pane that loses focus is no longer listening, and delivering to it
        // anyway would type into whatever replaced it.
        let (_loop, mut state) = test_state();
        state.open_pane();
        let pane = state.shell.focus.unwrap();
        state.panes.get_mut(&pane).unwrap().rows = 10;
        press(&mut state, KEY_A);
        press(&mut state, KEY_Q);
        assert!(state.is_repeating());

        // Focus moves elsewhere; the repeat has nothing left to deliver to.
        state.shell.focus = None;
        assert!(
            !state.deliver_repeat_for_test(),
            "delivery must refuse once the target is gone"
        );
    }

    #[test]
    fn the_quit_hatch_still_fires_while_an_editor_pane_has_focus() {
        // The ordering the Phase 3 plan calls non-negotiable. A pane takes
        // every key the keymap does not claim, so if its arm sat above the
        // escape hatch an editor would swallow `M-S-q` — on a DRM boot that is
        // a black screen with no way out.
        let (_loop, mut state) = test_state();
        // A keymap that does *not* bind quit, so what is being tested is the
        // unconditional hatch rather than a configured binding. With `M-S-q`
        // bound, this test passes even when the hatch is disabled entirely —
        // which it did, until the mutation showed it.
        state.keymap = crate::keymap::Keymap::new(&[("M-t".into(), "cycle workspace".into())]);
        state.open_pane();
        assert!(state.pane_has_focus(), "the new pane should hold focus");
        assert!(state.running.load(Ordering::SeqCst));

        press(&mut state, KEY_LEFTMETA);
        press(&mut state, KEY_LEFTSHIFT);
        press(&mut state, KEY_Q);
        assert!(
            !state.running.load(Ordering::SeqCst),
            "Super+Shift+q must quit even with an editor pane focused"
        );
    }

    /// A file whose lines are distinguishable by length, so an offset computed
    /// from the wrong line is not accidentally the right number.
    fn definition_fixture(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ruster-goto-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, "fn a() {}\nfn bb() {}\nfn ccc() {}\n").unwrap();
        path
    }

    #[test]
    fn a_definition_moves_the_cursor_to_the_line_the_server_named() {
        let (_loop, mut state) = test_state();
        state.open_pane();
        let id = state.shell.focus.expect("a pane was just focused");
        state.panes.get_mut(&id).unwrap().rows = 10;
        let path = definition_fixture("target.rs");

        state.goto_definition(&[ruster_lsp::Location {
            uri: path.to_string_lossy().into_owned(),
            start: ruster_lsp::results::LspPositionEq {
                line: 2,
                character: 3,
            },
        }]);

        // Line 2 starts after "fn a() {}\n" (10) + "fn bb() {}\n" (11) = 21,
        // and character 3 is three further in. Spelled out rather than computed
        // from the same conversion under test, which would pass either way.
        let head = state.panes[&id].cursors.primary().head;
        assert_eq!(head, 24, "cursor should be at line 2, character 3");
        let doc = state.panes[&id].doc;
        let opened = state.buffers.get(doc).unwrap().file_path.clone();
        assert_eq!(
            opened.as_deref(),
            Some(path.as_path()),
            "the jump should open the file the location named"
        );
    }

    #[test]
    fn a_definition_reply_with_no_locations_leaves_the_pane_alone() {
        // rust-analyzer answers `null` for a cursor on whitespace, and the empty
        // list has to be a no-op rather than a jump to the top of the file.
        //
        // The pane is moved somewhere first on purpose. Asserting against a
        // fresh pane proves nothing: a cursor that got reset to 0 and a cursor
        // that was never touched are the same reading, and the first version of
        // this test passed against a mutation that jumped to the top of the file
        // on every empty reply.
        let (_loop, mut state) = test_state();
        state.open_pane();
        let id = state.shell.focus.expect("a pane was just focused");
        state.panes.get_mut(&id).unwrap().rows = 10;
        let path = definition_fixture("stay-put.rs");
        state.goto_definition(&[ruster_lsp::Location {
            uri: path.to_string_lossy().into_owned(),
            start: ruster_lsp::results::LspPositionEq {
                line: 2,
                character: 3,
            },
        }]);
        let (doc, head) = (
            state.panes[&id].doc,
            state.panes[&id].cursors.primary().head,
        );
        assert_eq!(
            head, 24,
            "the setup jump must have landed somewhere visible"
        );

        state.goto_definition(&[]);

        assert_eq!(
            state.panes[&id].doc, doc,
            "an empty reply must not change the document"
        );
        assert_eq!(
            state.panes[&id].cursors.primary().head,
            head,
            "an empty reply must leave the cursor where it was"
        );
    }

    #[test]
    fn a_definition_jump_scrolls_the_pane_to_what_it_jumped_to() {
        // The failure this prevents is silent: the cursor moves correctly, the
        // target is below the fold, and the pane looks like it did nothing.
        let (_loop, mut state) = test_state();
        state.open_pane();
        let id = state.shell.focus.expect("a pane was just focused");
        state.panes.get_mut(&id).unwrap().rows = 3;
        let dir = std::env::temp_dir().join(format!("ruster-goto-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("long.rs");
        std::fs::write(&path, "x\n".repeat(200)).unwrap();

        state.goto_definition(&[ruster_lsp::Location {
            uri: path.to_string_lossy().into_owned(),
            start: ruster_lsp::results::LspPositionEq {
                line: 150,
                character: 0,
            },
        }]);

        let pane = &state.panes[&id];
        assert!(
            pane.scroll_top > 0,
            "a jump past the bottom of a 3-row pane must scroll: top={}",
            pane.scroll_top
        );
        assert!(
            pane.scroll_top <= 150,
            "and must not scroll past the target: top={}",
            pane.scroll_top
        );
    }

    #[test]
    fn a_screenshot_that_never_gets_a_frame_says_so() {
        // The silence this replaces: a verification run dispatched four actions
        // on time, wrote no PNG, and logged nothing to explain the gap, because
        // rendering waits for the host to invite a frame and a window the host
        // is not presenting is never invited.
        let (_loop, mut state) = test_state();
        let asked = std::time::Instant::now();
        state.dispatch(Action::Screenshot);
        assert!(state.screenshot_pending.is_some(), "the request is queued");

        assert!(
            !state.screenshot_overdue(asked + std::time::Duration::from_millis(500)),
            "half a second is a slow host, not a broken one"
        );
        assert!(
            state.screenshot_pending.is_some(),
            "and the request must survive to be served by the next frame"
        );

        assert!(
            state.screenshot_overdue(asked + std::time::Duration::from_secs(3)),
            "three seconds with no frame is a capture that is not coming"
        );
        assert!(
            state.screenshot_pending.is_none(),
            "giving up must clear it, or the capture lands later as a PNG of \
             a moment nobody asked about"
        );
    }

    #[test]
    fn a_hover_panel_keeps_the_code_and_drops_the_fence() {
        let (_loop, mut state) = test_state();
        state.open_pane();
        let id = state.shell.focus.expect("a pane was just focused");

        state.show_hover(
            id,
            4,
            2,
            "```rust\nfn main()\n```\n\n---\nthe entry point\n",
        );

        let hover = state.hover.as_ref().expect("a panel should be showing");
        assert_eq!(
            hover.lines,
            vec![
                "fn main()".to_string(),
                String::new(),
                "the entry point".to_string()
            ],
            "fences and separators go, the code stays"
        );
        assert_eq!(
            (hover.row, hover.col),
            (4, 2),
            "anchored where it was asked"
        );
    }

    #[test]
    fn a_hover_reply_of_pure_formatting_shows_no_panel() {
        // rust-analyzer answers with an empty markup block often enough, and an
        // empty box beside the caret reads as "this symbol is not documented"
        // rather than "the server said nothing".
        let (_loop, mut state) = test_state();
        state.open_pane();
        let id = state.shell.focus.expect("a pane was just focused");

        state.show_hover(id, 0, 0, "```rust\n```\n---\n\n");

        assert!(
            state.hover.is_none(),
            "a panel with nothing in it should not be shown at all"
        );
    }

    #[test]
    fn a_key_dismisses_the_hover_panel() {
        // It describes one caret on one character. The moment either can have
        // moved it is at best stale, and pointing at the wrong symbol confidently
        // is worse than not answering.
        let (_loop, mut state) = test_state();
        state.open_pane();
        let id = state.shell.focus.expect("a pane was just focused");
        state.panes.get_mut(&id).unwrap().rows = 10;
        state.show_hover(id, 0, 0, "fn main()");
        assert!(state.hover.is_some(), "the panel is up before the key");

        press(&mut state, KEY_A);

        assert!(state.hover.is_none(), "any key should dismiss it");
    }

    #[test]
    fn a_pane_receives_the_keys_the_keymap_does_not_claim() {
        // The other half: everything the WM has no use for reaches the editor,
        // rather than being forwarded to a client that is not focused.
        let (_loop, mut state) = test_state();
        state.open_pane();
        let id = state.shell.focus.expect("a pane was just focused");
        state.panes.get_mut(&id).unwrap().rows = 10;

        press(&mut state, KEY_A);
        let doc = state.panes[&id].doc;
        let text = state.buffers.get(doc).unwrap().buffer.line_to_string(0);
        assert!(
            !text.starts_with('a'),
            "`a` is an append command, not a literal: {text:?}"
        );
        // Now in insert mode, so the next key is text.
        press(&mut state, KEY_Q);
        assert!(
            state
                .buffers
                .get(state.panes[&id].doc)
                .unwrap()
                .buffer
                .line_to_string(0)
                .contains('q'),
            "typing after `a` should insert"
        );
    }

    #[test]
    fn quit_is_recognised_whatever_the_config_says() {
        // The escape hatch is unconditional on purpose: a config that binds
        // quit to something broken must not be able to trap a DRM session. It
        // lives in the key handler now rather than the resolver, so this tests
        // the predicate the handler consults.
        let mods = ModifiersState {
            logo: true,
            shift: true,
            ..Default::default()
        };
        assert!(is_quit_keysym(Keysym::q, &mods));
        // And a config binding quit elsewhere does not disable it.
        let keybinds = vec![("M-S-x".into(), "quit".into())];
        assert_eq!(resolve_one(&keybinds, &mods, Keysym::q), None);
    }

    #[test]
    fn a_rebound_key_no_longer_does_what_it_used_to() {
        // `M-t` cycled workspaces via a hardcoded fallback that fired even when
        // the config said otherwise, which made the config a liar about every
        // key it did not mention. Quit is the only survivor of that fallback.
        let keybinds = vec![("M-t".into(), "screenshot".into())];
        let mods = ModifiersState {
            logo: true,
            ..Default::default()
        };
        assert_eq!(
            resolve_one(&keybinds, &mods, Keysym::t),
            Some(Action::Screenshot)
        );
        // And with nothing bound to it at all, it does nothing.
        assert_eq!(resolve_one(&[], &mods, Keysym::t), None);
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
        assert_eq!(resolve_one(&keybinds, &mods, Keysym::q), Some(Action::Quit));
        let cycle_mods = ModifiersState {
            logo: true,
            ..Default::default()
        };
        assert_eq!(
            resolve_one(&keybinds, &cycle_mods, Keysym::t),
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
            resolve_one(&keybinds, &logo, Keysym::F9),
            Some(Action::CycleWorkspace),
            "a key the compositor has no built-in knowledge of"
        );
        let ctrl_alt = ModifiersState {
            ctrl: true,
            alt: true,
            ..Default::default()
        };
        assert_eq!(
            resolve_one(&keybinds, &ctrl_alt, Keysym::F10),
            Some(Action::Quit)
        );
        // An action name the compositor does not know binds nothing, rather
        // than falling through to some default.
        let unknown = vec![("M-F9".into(), "launch nukes".into())];
        assert_eq!(resolve_one(&unknown, &logo, Keysym::F9), None);
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
        assert_eq!(resolve_one(&keybinds, &tab_mods, Keysym::Tab), None);
        assert_eq!(resolve_one(&keybinds, &tab_mods, Keysym::NoSymbol), None);
    }
}
