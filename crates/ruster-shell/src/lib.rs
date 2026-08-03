//! Shell state for the ruster Wayland compositor.
//! Phase 0 keeps this minimal: a window record, focus tracking, and a
//! workspace counter. The i3 container-tree lands in Phase 1.

pub mod state;
pub mod window;

pub use state::ShellState;
pub use window::{ClientWindow, WindowId};
