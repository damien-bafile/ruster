use std::sync::atomic::Ordering;
use std::time::Duration;

use smithay::backend::renderer::damage::{Error as OutputDamageTrackerError, OutputDamageTracker};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::{winit, SwapBuffersError};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;
use smithay::reexports::winit::platform::pump_events::PumpStatus;

use ruster_compositor::backend::winit::RusterWinitData;
#[cfg(feature = "udev")]
use ruster_compositor::compositor::drm_error_hint;
use ruster_compositor::compositor::{
    create_state, init_listener, install_signal_handlers, log_startup_header, CompositorState,
};
use ruster_compositor::lua::{apply_config_to_shell, load_compositor_config};
use ruster_compositor::render::render_frame;

use tracing_subscriber::EnvFilter;

pub fn run() -> anyhow::Result<()> {
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
        #[cfg(feature = "udev")]
        {
            if let Err(err) = ruster_compositor::backend::drm::run_drm() {
                eprintln!("DRM backend failed: {err}");
                eprintln!("{}", drm_error_hint());
                std::process::exit(1);
            }
            return Ok(());
        }
        #[cfg(not(feature = "udev"))]
        anyhow::bail!("--drm requires building ruster-compositor with the `udev` feature");
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
    log_startup_header(env!("CARGO_PKG_VERSION"), "winit", &socket_name);
    apply_config_to_shell(&mut state, load_compositor_config(), &socket_name);

    let running = state.running.clone();
    install_signal_handlers(&running, event_loop.get_signal())?;

    let mut winit = winit;
    while state.running.load(Ordering::SeqCst) {
        let status = winit.dispatch_new_events(|event| state.handle_event(event));
        if let PumpStatus::Exit(_) = status {
            state.running.store(false, Ordering::SeqCst);
            break;
        }

        // Composite the focused toplevel and present it to the winit window.
        let age = if state.backend_data.full_redraw() > 0 {
            0
        } else {
            state.backend_data.backend.buffer_age().unwrap_or(0)
        };
        // Read before `bind()` takes a mutable borrow of the backend for the
        // rest of the frame.
        let geometry = state.geometry();
        let render_res = state
            .backend_data
            .backend
            .bind()
            .and_then(|(renderer, mut fb)| {
                let focused_title = state
                    .shell
                    .focused()
                    .map(|w| w.title.clone())
                    .unwrap_or_default();
                let cursor_status = state.cursor_status.clone();
                let cursor_location = state.pointer.current_location();
                render_frame(
                    state.shell.focus,
                    &state.toplevels,
                    &mut state.backend_data.damage_tracker,
                    &state.backend_data.output,
                    &mut state.chrome,
                    state.shell.workspace,
                    &focused_title,
                    renderer,
                    &mut fb,
                    age,
                    &cursor_status,
                    cursor_location,
                    &geometry,
                )
                .map_err(|err| match err {
                    OutputDamageTrackerError::Rendering(err) => err.into(),
                    _ => unreachable!(),
                })
            });
        match render_res {
            Ok(Some(damage)) => {
                if let Err(err) = state.backend_data.backend.submit(Some(&damage)) {
                    tracing::warn!("Failed to submit buffer: {err}");
                }
            }
            Ok(None) => {}
            Err(SwapBuffersError::ContextLost(err)) => {
                tracing::error!("GL context lost, shutting down: {err}");
                state.running.store(false, Ordering::SeqCst);
            }
            Err(err) => tracing::error!("Rendering failed: {err}"),
        }

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
