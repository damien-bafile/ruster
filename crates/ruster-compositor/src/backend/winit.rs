use std::sync::atomic::Ordering;

use smithay::backend::input::{
    AbsolutePositionEvent, Event, InputEvent, KeyState, KeyboardKeyEvent, PointerButtonEvent,
};
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::{WinitEvent, WinitGraphicsBackend};
use smithay::input::keyboard::FilterResult;
use smithay::input::pointer::{ButtonEvent, MotionEvent};
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::wayland_server::protocol::wl_pointer;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::utils::{Size, SERIAL_COUNTER as SCOUNTER};
use tracing::{debug, info};

use crate::compositor::CompositorState;
use crate::input::{is_cycle_workspace, is_quit_keysym};

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
                let state = event.state();
                let serial = SCOUNTER.next_serial();
                let time = Event::time_msec(&event);
                let keyboard = self.seat.get_keyboard().unwrap();
                let intercepted = keyboard.input::<(), _>(
                    self,
                    keycode,
                    state,
                    serial,
                    time,
                    |_, modifiers, handle| {
                        let keysym = handle.modified_sym();
                        if is_quit_keysym(keysym, modifiers) {
                            FilterResult::Intercept(())
                        } else if is_cycle_workspace(keysym, modifiers) {
                            // TODO(Task 8): cycle the focused workspace.
                            debug!("workspace cycle keybinding");
                            FilterResult::Intercept(())
                        } else {
                            FilterResult::Forward
                        }
                    },
                );
                if intercepted.is_some() && state == KeyState::Pressed {
                    info!("quit keybinding pressed, shutting down");
                    self.running.store(false, Ordering::SeqCst);
                }
            }
            WinitEvent::Input(InputEvent::PointerMotionAbsolute { event }) => {
                let output = &self.backend_data.output;
                let scale = output.current_scale().fractional_scale();
                let size: smithay::utils::Size<i32, smithay::utils::Logical> = output
                    .current_mode()
                    .map(|mode| {
                        Size::from((
                            (mode.size.w as f64 / scale) as i32,
                            (mode.size.h as f64 / scale) as i32,
                        ))
                    })
                    .unwrap_or_default();
                let pos = event.position_transformed(size);
                let serial = SCOUNTER.next_serial();
                let time = Event::time_msec(&event);
                let pointer = self.pointer.clone();
                pointer.motion(
                    self,
                    None,
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
                // TODO(Task 6): dispatch clicks once surfaces exist.
                let serial = SCOUNTER.next_serial();
                let time = Event::time_msec(&event);
                let state = wl_pointer::ButtonState::from(event.state());
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
                debug!(?event, "pointer axis");
                // TODO(Task 6): build an `AxisFrame` and scroll focused surfaces.
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
