use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::WinitGraphicsBackend;
use smithay::output::Output;

use super::Backend;

/// Winit backend state: owns the GLES winit surface and its damage tracker.
///
/// Output creation (`Output::new` + `PhysicalProperties`/`Mode`,
/// `change_current_state`, `set_preferred`), `winit::init::<GlesRenderer>()` and
/// the `WinitEvent` pump are wired up in `run_winit`/Task 5, mirroring
/// `anvil/src/winit.rs`.
pub struct RusterWinitData {
    pub backend: WinitGraphicsBackend<GlesRenderer>,
    pub damage_tracker: OutputDamageTracker,
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
    ) -> Self {
        RusterWinitData {
            backend,
            damage_tracker,
            full_redraw: 4,
        }
    }
}
