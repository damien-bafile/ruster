use std::sync::atomic::Ordering;

use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::{WinitEvent, WinitGraphicsBackend};
use smithay::output::{Mode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::utils::Transform;
use tracing::info;

use crate::compositor::CompositorState;

use super::Backend;

pub const OUTPUT_NAME: &str = "winit";

/// Winit backend state: owns the GLES winit surface, its damage tracker and the
/// single `wl_output` global backed by the winit window.
pub struct RusterWinitData {
    pub backend: WinitGraphicsBackend<GlesRenderer>,
    pub damage_tracker: OutputDamageTracker,
    pub output: Output,
    full_redraw: u8,
    pub redraw: RedrawGate,
}

/// Whether the host has invited us to draw a frame.
///
/// Rendering used to run on every pass of the event loop. On a nested Wayland
/// session `eglSwapBuffers` blocks until the host releases a buffer, and a host
/// that has stopped presenting the window — occluded, minimised, on another
/// workspace — never does. That parked the whole compositor inside the swap:
/// LSP polling, key repeat, chord expiry and queued Lua commands all sit behind
/// it on the render thread, so an unpresented window froze every one of them.
/// Measured at zero CPU across 12 seconds, stuck in `ppoll`.
///
/// So the swap is now only entered when the host asks for a frame, and the rest
/// of the loop runs regardless of whether it ever does.
#[derive(Debug, Default)]
pub struct RedrawGate {
    pending: bool,
}

impl RedrawGate {
    /// The host asked for a frame.
    pub fn request(&mut self) {
        self.pending = true;
    }

    /// Consume an invitation. False means "do not render this pass" — which is
    /// the whole point, and the reason this is a gate rather than a bool that
    /// something could read twice.
    pub fn take(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }
}

impl Backend for RusterWinitData {
    fn seat_name(&self) -> String {
        String::from("ruster-winit")
    }

    fn reset_buffers(&mut self, _output: &Output) {
        self.full_redraw = 4;
    }

    fn output(&self) -> &Output {
        &self.output
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
            // Closed. The first frame is armed by `request_redraw` before the
            // loop starts, so every frame without exception is one the host
            // asked for.
            redraw: RedrawGate::default(),
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
        let scale = backend.scale_factor();
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
            },
        );
        output.create_global::<CompositorState<RusterWinitData>>(dh);
        // `Flipped180` is not cosmetic: GL's origin is bottom-left while the
        // compositor's coordinates are top-left, so without it every frame —
        // client surfaces and chrome alike — renders upside down. anvil sets the
        // same transform on its winit output.
        output.change_current_state(
            Some(mode),
            Some(Transform::Flipped180),
            Some(Scale::Fractional(scale)),
            Some((0, 0).into()),
        );
        output.set_preferred(mode);
        output
    }
}

impl CompositorState<RusterWinitData> {
    /// Route a `WinitEvent` into the compositor: resize updates the output
    /// mode, input is handed to the backend-agnostic handlers in
    /// [`crate::input`], and closing the window flips `running` off.
    pub fn handle_event(&mut self, event: WinitEvent) {
        match event {
            WinitEvent::Resized { size, .. } => {
                info!(?size, "winit window resized");
                let scale = self.backend_data.backend.scale_factor();
                let mode = Mode {
                    size,
                    refresh: 60_000,
                };
                let output = self.backend_data.output.clone();
                output.change_current_state(Some(mode), None, Some(Scale::Fractional(scale)), None);
                output.set_preferred(mode);
                self.backend_data.reset_buffers(&output);
                // Ask for a frame rather than opening the gate directly. Forcing
                // one here is what defeated the first version of this fix: a
                // resize arrives at startup, so the compositor rendered a second
                // frame the host had not invited and blocked in its swap.
                self.backend_data.backend.window().request_redraw();
            }
            WinitEvent::Input(event) => self.process_input_event(event),
            WinitEvent::CloseRequested => {
                info!("close requested, shutting down");
                self.running.store(false, Ordering::SeqCst);
            }
            // The host inviting us to draw. Everything else in the loop runs
            // whether or not this ever arrives.
            WinitEvent::Redraw => self.backend_data.redraw.request(),
            WinitEvent::Focus(_) => {}
        }
    }
}

#[cfg(test)]
mod redraw_gate_tests {
    use super::RedrawGate;

    #[test]
    fn an_invitation_is_consumed_by_taking_it() {
        let mut gate = RedrawGate::default();
        gate.request();
        assert!(gate.take(), "the invitation should let one frame through");
        assert!(
            !gate.take(),
            "a second frame without a second invitation is the stall: it enters \
             eglSwapBuffers with no buffer released and parks the event loop"
        );
    }

    #[test]
    fn a_gate_that_was_never_invited_stays_shut() {
        assert!(!RedrawGate::default().take());
    }

    /// Repeated invitations before a frame is drawn must not queue up into
    /// several frames, or a burst of them would render faster than the host
    /// presents and reach the blocking swap anyway.
    #[test]
    fn invitations_do_not_accumulate() {
        let mut gate = RedrawGate::default();
        gate.request();
        gate.request();
        gate.request();
        assert!(gate.take());
        assert!(!gate.take());
    }
}
