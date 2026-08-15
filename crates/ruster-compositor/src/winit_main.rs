use std::sync::atomic::Ordering;
use std::time::Duration;

use smithay::backend::renderer::damage::{Error as OutputDamageTrackerError, OutputDamageTracker};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::{winit, SwapBuffersError};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;

use ruster_compositor::backend::winit::{poll_timeout, RusterWinitData, Servicing};
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

/// Dispatch the event loop and flush clients. False means the loop should end.
///
/// Shared by both paths through an iteration — the one that rendered and the one
/// that skipped because the host had not asked for a frame. Duplicating it once
/// meant a client-visible flush could be added to one path and silently not the
/// other.
///
/// The dispatch blocks. Winit and the Wayland clients are both calloop sources,
/// so a keystroke, a redraw invitation or a client request wakes it at once;
/// the timeout covers only what has no fd of its own.
fn pump(
    event_loop: &mut EventLoop<'static, CompositorState<RusterWinitData>>,
    state: &mut CompositorState<RusterWinitData>,
) -> bool {
    // Never sleep when we have already been told to stop. The `while` above
    // only reaches its condition again after this returns, so blocking here
    // with `running` already false waits for an event that may never come:
    // a deferred `quit` fired on time against a host that had stopped
    // presenting, and the process then sat alive for another thirteen seconds
    // until an unrelated event happened to wake it. Signals are fine — those
    // come through calloop's own source and wake the dispatch — but anything
    // the loop decides for itself has to be noticed here.
    if !state.running.load(Ordering::SeqCst) {
        return false;
    }
    let now = std::time::Instant::now();
    let timeout = poll_timeout(Servicing {
        lsp: state.lsp.has_servers(),
        chord: state.chord.is_active(),
        next_deferred: state.wm.as_ref().and_then(|wm| wm.next_due(now)),
    });
    if event_loop.dispatch(timeout, state).is_err() {
        return false;
    }
    state.display_handle.flush_clients().unwrap();
    true
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

    // Winit as a calloop source rather than something pumped by hand once per
    // pass. This is what lets the dispatch below actually block: input and
    // redraw invitations arrive on winit's own fd and wake the loop, so the
    // timeout stops being the worst case for noticing a keystroke — and stops
    // having to be traded off against frame rate. Smithay's `WinitEventLoop`
    // registers the fd and drains anything already queued in `before_sleep`,
    // so nothing is stranded by going to sleep. niri and Hyprland are both
    // shaped this way: everything that can wake the compositor is a source.
    event_loop
        .handle()
        .insert_source(winit, |event, _, state| state.handle_event(event))
        .map_err(|err| anyhow::anyhow!("failed to register the winit backend: {err}"))?;

    // Arm the first frame. Every frame after it is armed by the one before, so
    // the compositor only ever enters the blocking swap on the host's
    // invitation.
    state.backend_data.backend.window().request_redraw();

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
        // Above the redraw gate deliberately: a capture that is waiting for a
        // frame the host will never invite has to be given up on *here*, where
        // the loop still runs, rather than inside the rendering it is waiting
        // for.
        state.screenshot_overdue(std::time::Instant::now());
        // Same reason, same place: a screencopy client is waiting on a frame
        // that a non-presenting host will never invite, and `grim` blocks on
        // that promise forever rather than giving up.
        state.screencopy.expire(std::time::Instant::now());

        // Only when the host has asked for a frame. Rendering unconditionally is
        // what stalled the compositor: the swap at the end of it blocks until
        // the host releases a buffer, and a host that is not presenting the
        // window never does, so everything above — LSP, key repeat, chords, Lua
        // — stopped with it. Skipping the render leaves all of that running.
        if !state.backend_data.redraw.take() {
            if !pump(&mut event_loop, &mut state) {
                state.running.store(false, Ordering::SeqCst);
                break;
            }
            continue;
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
        let shot = state.screenshot_pending.take().map(|_| {
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
                let output_size = state
                    .backend_data
                    .output
                    .current_mode()
                    .map(|m| m.size)
                    .unwrap_or_default();
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
                    hover: state.hover.as_ref(),
                    launcher: state.launcher.as_mut().map(|l| {
                        // One source for the viewport, so the scroll window and
                        // the drawing cannot disagree about how much fits.
                        let rows = ruster_compositor::chrome::launcher_layout(
                            output_size.w,
                            output_size.h,
                            l.row_count(),
                        )
                        .visible_rows;
                        l.view(rows)
                    }),
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
                    let waiting = std::mem::take(&mut state.screencopy.pending);
                    if let Some(size) = state.backend_data.output.current_mode().map(|m| m.size) {
                        ruster_compositor::screencopy::serve(waiting, renderer, &fb, size);
                    }
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

        // Ask for the next frame. The host answers on its own presentation
        // cadence, which is what throttles this loop to the display rather than
        // to a spin — and if it stops answering, the loop keeps running without
        // us, which is the whole point of the gate.
        state.backend_data.backend.window().request_redraw();

        if !pump(&mut event_loop, &mut state) {
            state.running.store(false, Ordering::SeqCst);
            break;
        }
    }
    // After the loop rather than in the quit action: SIGTERM and a closed winit
    // window end the session too, and a layout that only survives one of the
    // three ways out is worse than one that does not survive at all.
    state.save_session();
    tracing::info!("shutting down");
    Ok(())
}
