#[cfg(feature = "udev")]
pub mod drm;
pub mod winit;

use smithay::output::Output;
use smithay::utils::{Logical, Physical, Size};

/// Minimal backend contract. Phase 0 only needs a seat name, a buffer
/// reset hook and access to the compositor's primary output; DRM's Backend
/// impl is Task 11.
pub trait Backend {
    fn seat_name(&self) -> String;
    fn reset_buffers(&mut self, output: &Output);
    /// The compositor's primary output — the one the fullscreen toplevel
    /// spans — used e.g. to size the initial `xdg_toplevel` configure.
    fn output(&self) -> &Output;
}

/// The logical size of an output: its current physical mode size divided by
/// the current output scale. `None` while the output has no mode (e.g. before
/// the backend pushes its first mode). The fullscreen toplevel spans exactly
/// this, so the initial `xdg_toplevel` configure is sized to it. Backend
/// agnostic — both the winit and DRM backends drive an [`Output`].
pub fn logical_output_size(output: &Output) -> Option<Size<i32, Logical>> {
    let scale = output.current_scale().fractional_scale();
    output
        .current_mode()
        .map(|mode| logical_size_from(mode.size, scale))
}

/// Convert a physical size to a logical one at the given fractional scale,
/// rounding down to integer logical pixels. Pure, so it is unit-testable
/// without an [`Output`] (which needs a live display handle).
pub fn logical_size_from(size: Size<i32, Physical>, scale: f64) -> Size<i32, Logical> {
    Size::from((
        (size.w as f64 / scale) as i32,
        (size.h as f64 / scale) as i32,
    ))
}
