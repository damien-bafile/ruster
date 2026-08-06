//! Entry point for the ruster Wayland compositor.
//!
//! Wayland — and therefore smithay — is Linux-only, but ruster's workspace also
//! builds on macOS and Windows for the raylib GUI. So the real entry point lives
//! in `winit_main`, behind a platform gate, and this file degrades to a binary
//! that explains itself and exits non-zero everywhere else. That keeps
//! `cargo build` at the workspace root working on every platform we ship.

#[cfg(target_os = "linux")]
mod winit_main;

#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    winit_main::run()
}

#[cfg(not(target_os = "linux"))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("ruster-compositor is a Wayland compositor and only runs on Linux")
}
