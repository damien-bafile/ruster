use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

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
    last_invite: Option<Instant>,
}

impl RedrawGate {
    /// The host asked for a frame.
    pub fn request(&mut self) {
        self.pending = true;
        self.last_invite = Some(Instant::now());
    }

    /// Consume an invitation. False means "do not render this pass" — which is
    /// the whole point, and the reason this is a gate rather than a bool that
    /// something could read twice.
    pub fn take(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }

    /// How long since the host last asked for a frame. `None` means it never
    /// has. Feeds [`poll_timeout`], which is where that gets interpreted.
    pub fn since_invite(&self, now: Instant) -> Option<Duration> {
        self.last_invite
            .map(|then| now.saturating_duration_since(then))
    }
}

/// How long the event loop may block waiting for something to happen.
///
/// Gating the render on an invitation stopped the compositor freezing behind
/// `eglSwapBuffers`, but it left the loop turning over a fixed 1ms timeout —
/// about 9,400 passes and ~1% of a core every 10 seconds, forever, for a window
/// nobody is looking at. This is the other half of that fix.
///
/// The timeout cannot simply be raised, because winit's fd is *not* a calloop
/// source: it is pumped by hand once per pass, so this value is also the worst
/// case for noticing a keystroke. Hence the split — stay at 1ms while the host
/// is presenting (which is when someone is plausibly typing, and when redraw
/// invitations arrive every ~16ms anyway so the loop is paced by the display
/// rather than by this), and back off only once the invitations have stopped.
///
/// A pending deferred action overrides both: sleeping past its deadline would
/// run it late, and `ruster.wm.defer` is what the compositor uses to photograph
/// itself, so lateness there is measurement error.
pub fn poll_timeout(since_invite: Option<Duration>, next_deferred: Option<Duration>) -> Duration {
    /// One frame at 60Hz.
    const PRESENTING: Duration = Duration::from_millis(16);
    const ACTIVE: Duration = Duration::from_millis(1);
    const IDLE: Duration = Duration::from_millis(32);

    let base = match since_invite {
        Some(gap) if gap <= PRESENTING => ACTIVE,
        // Never invited, or invited so long ago the host has clearly stopped.
        _ => IDLE,
    };
    match next_deferred {
        Some(until) => base.min(until),
        None => base,
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
mod poll_timeout_tests {
    use super::poll_timeout;
    use std::time::Duration;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    #[test]
    fn a_presenting_host_keeps_the_loop_responsive() {
        // One frame ago at 60Hz: someone may be typing, and the pump timeout is
        // the worst case for noticing it, because winit's fd is not a calloop
        // source and is polled by hand once per pass.
        assert_eq!(poll_timeout(Some(MS(16)), None), MS(1));
    }

    #[test]
    fn a_host_that_has_stopped_presenting_lets_the_loop_sleep() {
        assert_eq!(
            poll_timeout(Some(MS(5_000)), None),
            MS(32),
            "spinning at 1ms for a window nobody is presenting is the ~1% CPU \
             idle burn this exists to stop"
        );
    }

    #[test]
    fn a_host_that_has_never_presented_is_idle_not_active() {
        assert_eq!(poll_timeout(None, None), MS(32));
    }

    /// The boundary is one frame at 60Hz, and it is inclusive: a gap of exactly
    /// a frame is the steady state of a presenting host, not evidence it has
    /// stopped.
    #[test]
    fn the_boundary_sits_at_one_frame_and_includes_it() {
        assert_eq!(poll_timeout(Some(MS(15)), None), MS(1));
        assert_eq!(poll_timeout(Some(MS(16)), None), MS(1));
        assert_eq!(poll_timeout(Some(MS(17)), None), MS(32));
    }

    #[test]
    fn a_deferred_action_is_never_slept_past() {
        assert_eq!(
            poll_timeout(Some(MS(5_000)), Some(MS(5))),
            MS(5),
            "idling 32ms would run a 5ms defer 27ms late, and defer is how the \
             compositor times its own screenshots"
        );
    }

    #[test]
    fn a_distant_deferred_action_does_not_hold_the_loop_awake() {
        assert_eq!(
            poll_timeout(Some(MS(5_000)), Some(MS(900))),
            MS(32),
            "clamping is a ceiling, not a target"
        );
        assert_eq!(poll_timeout(Some(MS(16)), Some(MS(900))), MS(1));
    }

    /// An already-overdue action asks for no wait at all: come round now, run
    /// it, and the queue empties rather than spinning.
    #[test]
    fn an_overdue_deferred_action_yields_immediately() {
        assert_eq!(poll_timeout(Some(MS(5_000)), Some(Duration::ZERO)), MS(0));
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
