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
use ruster_compositor::render::{render_frame, FrameInput};

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

/// Average frame time over the last second, at info level.
///
/// Exists for the `RUSTER_BENCH_GLYPHS` measurement: the question Stage 2 of the
/// Phase 3 plan has to answer is how frame time moves with render-element count,
/// and that is not answerable by watching. Averaged rather than logged per frame
/// because a line per frame is the flood that cost this project a 2MB DRM log.
fn report_frame_time(elapsed: Duration) {
    use std::cell::Cell;
    thread_local! {
        static FRAMES: Cell<u32> = const { Cell::new(0) };
        static TOTAL: Cell<Duration> = const { Cell::new(Duration::ZERO) };
        static SINCE: Cell<Option<std::time::Instant>> = const { Cell::new(None) };
    }
    if std::env::var_os("RUSTER_BENCH_GLYPHS").is_none() {
        return;
    }
    FRAMES.with(|f| f.set(f.get() + 1));
    TOTAL.with(|t| t.set(t.get() + elapsed));
    let start = SINCE.with(|s| {
        let v = s.get().unwrap_or_else(std::time::Instant::now);
        s.set(Some(v));
        v
    });
    if start.elapsed() < Duration::from_secs(1) {
        return;
    }
    let frames = FRAMES.with(|f| f.replace(0));
    let total = TOTAL.with(|t| t.replace(Duration::ZERO));
    SINCE.with(|s| s.set(Some(std::time::Instant::now())));
    if frames > 0 {
        tracing::info!(
            frames,
            avg_ms = total.as_secs_f64() * 1000.0 / frames as f64,
            "frame time"
        );
    }
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
    let (control, shell) = load_compositor_config();
    apply_config_to_shell(&mut state, control, shell, &socket_name);

    let running = state.running.clone();
    install_signal_handlers(&running, event_loop.get_signal())?;

    let mut winit = winit;
    while state.running.load(Ordering::SeqCst) {
        // Anything `ruster.wm.*` queued since the last pass, before rendering,
        // so a Lua-driven layout change shows up on this frame rather than the
        // next one.
        state.drain_wm_commands();
        // Whatever the language servers have said. A channel drain, so a server
        // that has stopped answering costs nothing here.
        state.poll_lsp();
        // A half-typed chord that is never finished has to clear itself, or the
        // overlay stays up and the next key is still being read as part of it.
        state.chord.expire(std::time::Instant::now());
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
        let tree_status = state.tree_status();
        let shot = state.screenshot_pending.then(|| {
            state.screenshot_pending = false;
            state.screenshot_count += 1;
            (
                ruster_compositor::screenshot::capture_path(state.screenshot_count),
                state.backend_data.output.current_mode().map(|m| m.size),
            )
        });
        let frame_started = std::time::Instant::now();
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
                let scene = FrameInput {
                    focus: state.shell.focus,
                    toplevels: &state.toplevels,
                    output: &state.backend_data.output,
                    workspace: state.workspaces.active(),
                    focused_title: &focused_title,
                    cursor_status: &cursor_status,
                    cursor_location,
                    geometry: &geometry,
                    tree_status,
                    panes: &state.panes,
                    buffers: &state.buffers,
                    highlights: &state.highlights,
                    lsp: &state.lsp,
                    keymap: &state.keymap,
                    minibuffer: state.minibuffer.as_ref(),
                    whichkey: ruster_compositor::keymap::whichkey_view(
                        &state.keymap,
                        &state.chord,
                        ruster_compositor::keymap::HelpState {
                            pinned: state.help_pinned,
                        },
                    ),
                };
                render_frame(
                    &scene,
                    &mut state.chrome,
                    &mut state.backend_data.damage_tracker,
                    renderer,
                    &mut fb,
                    age,
                )
                .map_err(|err| match err {
                    OutputDamageTrackerError::Rendering(err) => err.into(),
                    _ => unreachable!(),
                })
                .inspect(|_| {
                    // After the frame is drawn and before it is submitted: the
                    // contents are complete, and the copy is non-destructive so
                    // what reaches the screen is unchanged.
                    if let Some((path, Some(size))) = shot {
                        match ruster_compositor::screenshot::capture(
                            renderer,
                            &fb,
                            (size.w, size.h).into(),
                            // A direct GL framebuffer read is bottom-left first.
                            true,
                            &path,
                        ) {
                            Ok(path) => tracing::info!(path = %path.display(), "screenshot"),
                            Err(err) => tracing::warn!("screenshot failed: {err}"),
                        }
                    }
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

        report_frame_time(frame_started.elapsed());

        let result = event_loop.dispatch(Some(Duration::from_millis(1)), &mut state);
        if result.is_err() {
            state.running.store(false, Ordering::SeqCst);
            break;
        }
        state.display_handle.flush_clients().unwrap();
    }
    // After the loop rather than in the quit action: SIGTERM and a closed winit
    // window end the session too, and a layout that only survives one of the
    // three ways out is worse than one that does not survive at all.
    state.save_session();
    tracing::info!("shutting down");
    Ok(())
}
