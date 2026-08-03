pub mod winit;

use smithay::output::Output;

/// Minimal backend contract. Phase 0 only needs a seat name and a buffer
/// reset hook; DRM's Backend impl is Task 11.
pub trait Backend {
    fn seat_name(&self) -> String;
    fn reset_buffers(&mut self, output: &Output);
}
