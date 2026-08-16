//! Ruster as a Wayland compositor: boots on DRM (udev) or a nested winit
//! window, composites xdg-shell clients with a GLES renderer, and draws
//! ruster's chrome around them.
//!
//! Wayland is Linux-only, so on other platforms this crate compiles to an empty
//! library rather than failing the workspace build. See `main.rs`.
#![cfg(target_os = "linux")]

pub mod backend;
pub mod chrome;
pub mod client;
pub mod clipboard;
pub mod compositor;
pub mod focus;
pub mod highlight;
pub mod input;
pub mod keymap;
pub mod launcher;
pub mod lua;
pub mod minibuffer;
pub mod pane;
pub mod persist;
pub mod render;
pub mod repeat;
pub mod scene;
pub mod screencopy;
pub mod screenshot;
pub mod shell;
pub mod xwayland;
