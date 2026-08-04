use std::sync::atomic::Ordering;
use std::time::Duration;

use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit;
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;
use smithay::reexports::winit::event_loop::pump_events::PumpStatus;
use tracing::info;

use ruster_compositor::backend::winit::RusterWinitData;
use ruster_compositor::compositor::{create_state, init_listener, CompositorState};

use tracing_subscriber::EnvFilter;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("ruster-compositor: Phase 0 scaffold");
        println!("Usage: ruster-compositor [--drm]");
        return Ok(());
    }
    if args.iter().any(|a| a == "--drm") {
        // TODO(Task 11): boot the udev/DRM backend on the primary GPU.
        anyhow::bail!("--drm backend is not implemented until Task 11");
    }
    run_winit()
}

fn run_winit() -> anyhow::Result<()> {
    let mut event_loop: EventLoop<'static, CompositorState<RusterWinitData>> =
        EventLoop::try_new()?;
    let display: Display<CompositorState<RusterWinitData>> = Display::new()?;

    let (backend, winit) = winit::init::<GlesRenderer>()
        .map_err(|err| anyhow::anyhow!("failed to initialize winit backend: {err}"))?;
    let output = RusterWinitData::build_output(&backend, &display.handle());
    let damage_tracker = OutputDamageTracker::from_output(&output);
    let data = RusterWinitData::new(backend, damage_tracker, output);

    let mut state = create_state(display, event_loop.handle(), data);
    let socket_name = init_listener(&mut state);
    info!(?socket_name, "wayland socket ready");
    spawn_test_client(&socket_name);

    let running = state.running.clone();
    ctrlc::set_handler(move || running.store(false, Ordering::SeqCst))?;

    let mut winit = winit;
    while state.running.load(Ordering::SeqCst) {
        let status = winit.dispatch_new_events(|event| state.handle_event(event));
        if let PumpStatus::Exit(_) = status {
            state.running.store(false, Ordering::SeqCst);
            break;
        }

        // TODO(Task 7): render a frame here (bind renderer, composite, submit).

        let result = event_loop.dispatch(Some(Duration::from_millis(1)), &mut state);
        if result.is_err() {
            state.running.store(false, Ordering::SeqCst);
            break;
        }
        state.display_handle.flush_clients().unwrap();
    }
    tracing::info!("shutting down");
    Ok(())
}

/// Experimental: launch a Wayland client on our socket so a toplevel is mapped
/// without manual setup. No-op if no known client is installed, and it can
/// never crash the compositor (a spawned child failing is ignored).
fn spawn_test_client(socket_name: &str) {
    use std::process::Command;

    let client = if Command::new("foot").arg("--version").output().is_ok() {
        "foot"
    } else if Command::new("weston-terminal")
        .arg("--help")
        .output()
        .is_ok()
    {
        "weston-terminal"
    } else {
        return;
    };
    let _ = Command::new(client)
        .env("WAYLAND_DISPLAY", socket_name)
        .spawn();
}
