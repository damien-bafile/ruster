//! Ruster as a Wayland compositor: boots on DRM (udev) or a nested winit
//! window, composites xdg-shell clients with a GLES renderer, and draws
//! ruster's chrome around them.

pub mod backend;
pub mod compositor;
pub mod input;
