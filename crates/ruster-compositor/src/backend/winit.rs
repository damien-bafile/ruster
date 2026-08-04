use std::sync::atomic::Ordering;

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisSource, Event, InputEvent, KeyState, KeyboardKeyEvent,
    PointerAxisEvent, PointerButtonEvent,
};
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::{WinitEvent, WinitGraphicsBackend};
use smithay::input::keyboard::{FilterResult, Keysym};
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::wayland_server::protocol::{wl_pointer, wl_surface::WlSurface};
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::utils::{Logical, Point, Size, SERIAL_COUNTER as SCOUNTER};
use tracing::{debug, info};

use crate::compositor::CompositorState;
use crate::input::{apply_action, pointer_focus, resolve_wm_action};
use crate::lua::Action;

use super::Backend;

pub const OUTPUT_NAME: &str = "winit";

/// Winit backend state: owns the GLES winit surface, its damage tracker and the
/// single `wl_output` global backed by the winit window.
pub struct RusterWinitData {
    pub backend: WinitGraphicsBackend<GlesRenderer>,
    pub damage_tracker: OutputDamageTracker,
    pub output: Output,
    full_redraw: u8,
}

impl Backend for RusterWinitData {
    fn seat_name(&self) -> String {
        String::from("ruster-winit")
    }

    fn reset_buffers(&mut self, _output: &Output) {
        self.full_redraw = 4;
    }
}

impl RusterWinitData {
    pub fn new(
        backend: WinitGraphicsBackend<GlesRenderer>,
        damage_tracker: OutputDamageTracker,
        output: Output,
    ) -> Self {
        RusterWinitData {
            backend,
            damage_tracker,
            output,
            full_redraw: 4,
        }
    }

    /// Read and decrement the full-redraw counter. The first frames after
    /// startup or a resize are forced to age 0 (full damage); once it hits 0
    /// the renderer falls back to buffer-age based damage tracking.
    pub fn full_redraw(&mut self) -> u8 {
        let remaining = self.full_redraw;
        self.full_redraw = self.full_redraw.saturating_sub(1);
        remaining
    }

    /// Build the `wl_output` for the winit window, advertise it as a global and
    /// push the current window size as its only mode.
    pub fn build_output(
        backend: &WinitGraphicsBackend<GlesRenderer>,
        dh: &DisplayHandle,
    ) -> Output {
        let size = backend.window_size();
        let mode = Mode {
            size,
            refresh: 60_000,
        };
        let output = Output::new(
            OUTPUT_NAME.to_string(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "Smithay".into(),
                model: "Winit".into(),
                serial_number: "Unknown".into(),
            },
        );
        output.create_global::<CompositorState<RusterWinitData>>(dh);
        output.change_current_state(Some(mode), None, None, Some((0, 0).into()));
        output.set_preferred(mode);
        output
    }
}

impl CompositorState<RusterWinitData> {
    /// The output's logical size: its physical mode size divided by the
    /// current output scale. The fullscreen toplevel spans exactly this.
    fn logical_output_size(&self) -> Option<Size<i32, Logical>> {
        let output = &self.backend_data.output;
        let scale = output.current_scale().fractional_scale();
        output.current_mode().map(|mode| {
            Size::from((
                (mode.size.w as f64 / scale) as i32,
                (mode.size.h as f64 / scale) as i32,
            ))
        })
    }

    /// The surface under the pointer with its surface-local position, or `None`
    /// when the pointer is not over the focused mapped toplevel. Phase 0 is
    /// fullscreen: the focused toplevel covers the whole output from the origin,
    /// so the surface under the pointer is the focused one and surface-local
    /// coordinates equal the global position.
    fn surface_under(
        &self,
        location: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        let focus = self.shell.focused()?;
        let toplevel = self.toplevels.get(&focus.id)?;
        if !self.mapped.contains(&focus.id) {
            return None;
        }
        let local = pointer_focus(self.logical_output_size()?, location)?;
        Some((toplevel.wl_surface().clone(), local))
    }

    /// Route a `WinitEvent` into the compositor: resize updates the output mode,
    /// keyboard events run the WM keybindings, pointer events feed the seat, and
    /// closing the window flips `running` off.
    pub fn handle_event(&mut self, event: WinitEvent) {
        match event {
            WinitEvent::Resized { size, .. } => {
                info!(?size, "winit window resized");
                let mode = Mode {
                    size,
                    refresh: 60_000,
                };
                let output = self.backend_data.output.clone();
                output.change_current_state(Some(mode), None, None, None);
                output.set_preferred(mode);
                self.backend_data.reset_buffers(&output);
            }
            WinitEvent::Input(InputEvent::Keyboard { event }) => {
                let keycode = event.key_code();
                let key_state = event.state();
                let serial = SCOUNTER.next_serial();
                let time = Event::time_msec(&event);
                let keyboard = self.seat.get_keyboard().unwrap();
                // Intercept keys the WM has bound; everything else is Forwarded
                // to the focused client. Only `Action::Quit` on a press stops
                // the compositor — cycling a workspace must not shut it down.
                let _ = keyboard.input::<(), _>(
                    self,
                    keycode,
                    key_state,
                    serial,
                    time,
                    |compositor, modifiers, handle| {
                        // Use the raw (unshifted) keysym and the separate
                        // modifier state so Super+Shift+q resolves to the `q`
                        // binding rather than an uppercased `Q`.
                        let keysym = handle
                            .raw_latin_sym_or_raw_current_sym()
                            .unwrap_or(Keysym::NoSymbol);
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
                                    compositor.shell.cycle_workspace();
                                    let output = compositor.backend_data.output.clone();
                                    compositor.backend_data.reset_buffers(&output);
                                }
                                FilterResult::Intercept(())
                            }
                            None => FilterResult::Forward,
                        }
                    },
                );
            }
            WinitEvent::Input(InputEvent::PointerMotionAbsolute { event }) => {
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
            WinitEvent::Input(InputEvent::PointerButton { event }) => {
                debug!(button = event.button_code(), "pointer button");
                let serial = SCOUNTER.next_serial();
                let time = Event::time_msec(&event);
                let state = wl_pointer::ButtonState::from(event.state());
                if state == wl_pointer::ButtonState::Pressed {
                    // Click-to-focus: a press over the focused toplevel hands it
                    // the keyboard focus (anvil's `update_keyboard_focus`).
                    self.update_keyboard_focus(serial);
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
            WinitEvent::Input(InputEvent::PointerAxis { event }) => {
                // Build an `AxisFrame` and scroll the focused surface (anvil's
                // `on_pointer_axis`).
                let horizontal_amount = event.amount(Axis::Horizontal).unwrap_or_else(|| {
                    event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.
                });
                let vertical_amount = event.amount(Axis::Vertical).unwrap_or_else(|| {
                    event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.
                });

                let mut frame = AxisFrame::new(Event::time_msec(&event)).source(event.source());
                if horizontal_amount != 0.0 {
                    frame = frame.relative_direction(
                        Axis::Horizontal,
                        event.relative_direction(Axis::Horizontal),
                    );
                    frame = frame.value(Axis::Horizontal, horizontal_amount);
                    if let Some(discrete) = event.amount_v120(Axis::Horizontal) {
                        frame = frame.v120(Axis::Horizontal, discrete as i32);
                    }
                }
                if vertical_amount != 0.0 {
                    frame = frame.relative_direction(
                        Axis::Vertical,
                        event.relative_direction(Axis::Vertical),
                    );
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
            WinitEvent::CloseRequested => {
                info!("close requested, shutting down");
                self.running.store(false, Ordering::SeqCst);
            }
            WinitEvent::Focus(_) | WinitEvent::Redraw => {}
            WinitEvent::Input(_) => {}
        }
    }
}
