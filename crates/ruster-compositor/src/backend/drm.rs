//! DRM/udev backend: boots ruster on the primary GPU, driving a single
//! connector through libseat, udev, GBM, EGL and smithay's `DrmOutputManager`
//! (Task 11). Phase 0 renders one output on the primary GPU and ignores
//! hotplug/removal events; dmabuf feedback, multi-GPU and leasing are later
//! tasks.
//!
//! Hardware access requires a seat manager: run `seatd` (or launch ruster from
//! a logind session with `drm` in `systemd-analyze`'s session scope). Without
//! either, [`LibSeatSession::new`] fails and the compositor exits with an
//! error before touching any device.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::Fourcc;
use smithay::backend::drm::compositor::FrameFlags;
use smithay::backend::drm::exporter::gbm::GbmFramebufferExporter;
use smithay::backend::drm::output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements};
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmEvent, DrmNode};
use smithay::backend::egl::context::ContextPriority;
use smithay::backend::egl::{EGLContext, EGLDevice, EGLDisplay};
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::multigpu::gbm::GbmGlesBackend;
use smithay::backend::renderer::multigpu::{GpuManager, MultiRenderer};
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::{Event as SessionEvent, Session};
use smithay::backend::udev::{all_gpus, primary_gpu, UdevBackend, UdevEvent};
use smithay::output::{Mode as WlMode, Output, PhysicalProperties};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::drm::control::Device as _;
use smithay::reexports::drm::control::{self, connector, crtc, ModeTypeFlags};
use smithay::reexports::input::Libinput;
use smithay::reexports::rustix::fs::OFlags;
use smithay::reexports::wayland_server::Display;
use smithay::utils::DeviceFd;
use tracing::{error, info, warn};

use crate::backend::Backend;
use crate::compositor::{
    create_state, init_listener, install_signal_handlers, log_startup_header, CompositorState,
};
use crate::lua::{apply_config_to_shell, load_compositor_config};
use crate::render::{
    collect_render_elements, send_frame_callbacks, ChromeRenderElements, FrameInput, CLEAR_COLOR,
};

/// Pixel formats the compositor will try for the primary plane, in order of
/// preference. 10-bit formats first (wider color), then the 8-bit fallbacks.
const SUPPORTED_FORMATS: &[Fourcc] = &[
    Fourcc::Abgr2101010,
    Fourcc::Argb2101010,
    Fourcc::Abgr8888,
    Fourcc::Argb8888,
];

/// The `wl_output` model string for a connector, e.g. `ruster-drm-eDP-1`.
fn output_model(connector: &str) -> String {
    format!("ruster-drm-{connector}")
}

/// The renderer used to composite onto the DRM primary plane. `GbmGlesBackend`
/// instantiates a `GlesRenderer` per render node; `MultiRenderer` hands out
/// `&mut`-renderers for one device at a time.
type RusterUdevRenderer<'a> = MultiRenderer<
    'a,
    'a,
    GbmGlesBackend<GlesRenderer, DrmDeviceFd>,
    GbmGlesBackend<GlesRenderer, DrmDeviceFd>,
>;

type UdevAllocator = GbmAllocator<DrmDeviceFd>;
type UdevOutputManager =
    DrmOutputManager<UdevAllocator, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;
type UdevDrmOutput = DrmOutput<UdevAllocator, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;

/// Per-backend state for the DRM backend: the session, the GPU manager, the
/// output manager and the single output currently driven by it.
pub struct RusterUdevData {
    session: LibSeatSession,
    /// The EGL render node (`renderD*`) the compositor renders on. This is the
    /// node registered with [`GpuManager::add_node`], so it — not the seat's
    /// card node — must be handed to [`GpuManager::single_renderer`] every
    /// frame. The card node is only used during init for device discovery.
    render_node: DrmNode,
    gpus: GpuManager<GbmGlesBackend<GlesRenderer, DrmDeviceFd>>,
    drm_output_manager: UdevOutputManager,
    output: Output,
    drm_output: UdevDrmOutput,
    crtc: crtc::Handle,
}

impl Backend for RusterUdevData {
    fn seat_name(&self) -> String {
        self.session.seat()
    }

    fn reset_buffers(&mut self, _output: &Output) {
        self.drm_output.reset_buffers();
    }

    fn output(&self) -> &Output {
        &self.output
    }

    fn change_vt(&mut self, vt: i32) {
        info!(vt, "switching virtual terminal");
        if let Err(err) = self.session.change_vt(vt) {
            error!(vt, "failed to switch virtual terminal: {err}");
        }
    }
}

/// Boot the DRM/udev backend on the primary GPU: open a libseat session, find
/// the primary GPU, initialize EGL/GBM, scan for a connected connector, create
/// the output and run the calloop event loop until `running` flips off.
pub fn run_drm() -> anyhow::Result<()> {
    let mut event_loop: EventLoop<'static, CompositorState<RusterUdevData>> = EventLoop::try_new()?;
    let display: Display<CompositorState<RusterUdevData>> = Display::new()?;

    // Session: libseat gates device access; a running seatd or a logind
    // session is required (a bare VT without either fails right here).
    let (mut session, session_notifier) = LibSeatSession::new()
        .map_err(|err| anyhow::anyhow!("failed to initialize libseat session: {err}"))?;
    let seat_name = session.seat();
    info!(?seat_name, "libseat session active");

    // Find the primary GPU for the seat: honor the RUSTER_DRM_DEVICE override,
    // else the seat's primary GPU, else any GPU. This is the *card* node used
    // for device discovery; the render node is derived from EGL below.
    let primary_gpu = resolve_gpu_node(
        &seat_name,
        std::env::var("RUSTER_DRM_DEVICE").ok().as_deref(),
    )?;
    info!(?primary_gpu, "using primary gpu");

    // One GbmGlesBackend per render node; GpuManager turns it into renderers.
    let mut gpus: GpuManager<GbmGlesBackend<GlesRenderer, DrmDeviceFd>> =
        GpuManager::new(GbmGlesBackend::with_factory(|display| {
            let context = EGLContext::new_with_priority(display, ContextPriority::High)?;
            let capabilities = unsafe { GlesRenderer::supported_capabilities(&context)? };
            Ok(unsafe { GlesRenderer::with_capabilities(context, capabilities)? })
        }))
        .map_err(|err| anyhow::anyhow!("failed to initialize gpu manager: {err}"))?;

    // Resolve the device path backing the primary GPU from the udev device
    // list, then open it through the session (libseat grants the fd).
    let udev_backend = UdevBackend::new(&seat_name)
        .map_err(|err| anyhow::anyhow!("failed to initialize udev backend: {err}"))?;
    let device_path = udev_device_path(&udev_backend, &primary_gpu)
        .ok_or_else(|| anyhow::anyhow!("no drm device in udev list for seat {seat_name}"))?;
    info!(path = ?device_path, "opening drm device");

    let fd = session
        .open(
            &device_path,
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
        )
        .map_err(|err| anyhow::anyhow!("failed to open {}: {err}", device_path.display()))?;
    let drm_fd = DrmDeviceFd::new(DeviceFd::from(fd));

    // Set up DRM and GBM against the same device fd.
    let (drm, drm_notifier) = DrmDevice::new(drm_fd.clone(), true)
        .map_err(|err| anyhow::anyhow!("failed to initialize drm device: {err}"))?;
    let gbm =
        GbmDevice::new(drm_fd).map_err(|err| anyhow::anyhow!("failed to initialize gbm: {err}"))?;

    // EGL on the scanout device; the render node may differ from the scanout
    // node (hybrid graphics). A software device is rejected: ruster does not
    // (yet) fall back to rendering through a different GPU.
    let egl_display = unsafe { EGLDisplay::new(gbm.clone()) }
        .map_err(|err| anyhow::anyhow!("failed to initialize egl display: {err}"))?;
    let egl_device = EGLDevice::device_for_display(&egl_display)
        .map_err(|err| anyhow::anyhow!("failed to resolve egl device: {err}"))?;
    if egl_device.is_software() {
        anyhow::bail!("EGL reports a software device for {primary_gpu}; refusing to run");
    }
    let render_node = egl_device
        .try_get_render_node()
        .ok()
        .flatten()
        .unwrap_or(primary_gpu);
    info!(?render_node, "using render node");
    gpus.as_mut()
        .add_node(render_node, gbm.clone())
        .map_err(|err| anyhow::anyhow!("failed to add gpu node: {err}"))?;

    let allocator = GbmAllocator::new(
        gbm.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );
    let exporter = GbmFramebufferExporter::new(gbm.clone(), render_node.into());

    // Formats the GLES renderer can composite into, fed to the DrmCompositor
    // so it can pick scan-out-capable buffer formats.
    let render_formats = {
        let mut renderer = gpus
            .single_renderer(&render_node)
            .map_err(|err| anyhow::anyhow!("failed to get renderer for {render_node}: {err}"))?;
        renderer
            .as_mut()
            .egl_context()
            .dmabuf_render_formats()
            .iter()
            .copied()
            .collect::<FormatSet>()
    };

    let mut drm_output_manager = DrmOutputManager::new(
        drm,
        allocator,
        exporter,
        Some(gbm),
        SUPPORTED_FORMATS.iter().copied(),
        render_formats,
    );

    // Scan for a connected connector and the crtc it maps to.
    let (connector, crtc, drm_mode) = find_connected_connector(&drm_output_manager)?;
    let output_name = format!(
        "{}-{}",
        connector.interface().as_str(),
        connector.interface_id()
    );
    info!(?crtc, "setting up connector {output_name}");

    let wl_mode = WlMode::from(drm_mode);
    let (phys_w, phys_h) = connector.size().unwrap_or((0, 0));
    let output = Output::new(
        output_name.clone(),
        PhysicalProperties {
            size: (phys_w as i32, phys_h as i32).into(),
            subpixel: connector.subpixel().into(),
            make: "Smithay".into(),
            model: output_model(&output_name),
        },
    );
    output.create_global::<CompositorState<RusterUdevData>>(&display.handle());
    output.set_preferred(wl_mode);
    output.change_current_state(Some(wl_mode), None, None, Some((0, 0).into()));

    // Initialize the scan-out surface: a `DrmOutput` driving `crtc` with the
    // connector attached. `planes: None` lets the compositor use all planes.
    let mut renderer = gpus
        .single_renderer(&render_node)
        .map_err(|err| anyhow::anyhow!("failed to get renderer for {render_node}: {err}"))?;
    let drm_output = drm_output_manager
        .initialize_output::<_, ChromeRenderElements<RusterUdevRenderer<'_>>>(
            crtc,
            drm_mode,
            &[connector.handle()],
            &output,
            None,
            &mut renderer,
            &DrmOutputRenderElements::default(),
        )
        .map_err(|err| anyhow::anyhow!("failed to initialize drm output: {err}"))?;
    drop(renderer);

    let data = RusterUdevData {
        session,
        render_node,
        gpus,
        drm_output_manager,
        output,
        drm_output,
        crtc,
    };
    let mut state = create_state(display, event_loop.handle(), data);
    let socket_name = init_listener(&mut state);
    log_startup_header(env!("CARGO_PKG_VERSION"), "drm", &socket_name);
    // Same config as the winit path: install the WM keybinds and launch the
    // configured startup clients. On DRM this is the only way a client ever
    // appears — there is no outer compositor to launch one into our socket by
    // hand — so a boot without it renders an empty screen no keybinding can
    // escape.
    let (control, shell) = load_compositor_config();
    apply_config_to_shell(&mut state, control, shell, &socket_name);

    // Input: libinput seatted to the same libseat session, so device fds are
    // opened through the session and revoked on VT switch. Events go through
    // the same backend-agnostic handlers the winit path uses.
    let mut libinput_context = Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(
        state.backend_data.session.clone().into(),
    );
    libinput_context
        .udev_assign_seat(&seat_name)
        .map_err(|()| anyhow::anyhow!("failed to assign libinput to seat {seat_name}"))?;
    state
        .handle
        .insert_source(
            LibinputInputBackend::new(libinput_context.clone()),
            |event, _, data: &mut CompositorState<RusterUdevData>| {
                data.process_input_event(event);
            },
        )
        .map_err(|err| anyhow::anyhow!("failed to insert libinput source: {err}"))?;

    // DRM device events: VBlanks mark the queued frame as presented and
    // schedule the next repaint; errors are logged.
    let crtc_handle = state.backend_data.crtc;
    state
        .handle
        .insert_source(
            drm_notifier,
            move |event, _metadata, data: &mut CompositorState<RusterUdevData>| match event {
                DrmEvent::VBlank(crtc) if crtc == crtc_handle => data.frame_finish(),
                DrmEvent::VBlank(_) => {}
                DrmEvent::Error(err) => error!("drm device error: {err:?}"),
            },
        )
        .expect("failed to insert drm device source");

    // Session events: pause on VT switch away, resume the drm backend and
    // repaint when the session becomes active again. libinput must be
    // suspended alongside the drm device — its device fds are revoked by the
    // session on VT switch, and dispatching against revoked fds is what turns
    // a VT switch into a dead keyboard on the way back.
    state
        .handle
        .insert_source(
            session_notifier,
            move |event, &mut (), data: &mut CompositorState<RusterUdevData>| match event {
                SessionEvent::PauseSession => {
                    info!("pausing session");
                    libinput_context.suspend();
                    data.backend_data.drm_output_manager.pause();
                }
                SessionEvent::ActivateSession => {
                    info!("resuming session");
                    if let Err(err) = libinput_context.resume() {
                        error!("failed to resume libinput context: {err:?}");
                    }
                    if let Err(err) = data.backend_data.drm_output_manager.activate(false) {
                        error!("failed to activate drm backend: {err:?}");
                    }
                    data.handle.insert_idle(|data| data.render_surface());
                }
            },
        )
        .expect("failed to insert session source");

    // udev events: Phase 0 drives a single output on the primary GPU, so
    // additions/removals of other devices are only logged.
    state
        .handle
        .insert_source(
            udev_backend,
            |event, _, _data: &mut CompositorState<RusterUdevData>| match event {
                UdevEvent::Added { device_id, path } => {
                    info!(?device_id, ?path, "drm device added (ignored in Phase 0)");
                }
                UdevEvent::Changed { device_id } => {
                    info!(?device_id, "drm device changed");
                }
                UdevEvent::Removed { device_id } => {
                    info!(?device_id, "drm device removed");
                }
            },
        )
        .expect("failed to insert udev source");

    // SIGINT/SIGTERM flip `running` off and stop the event loop.
    let running = state.running.clone();
    install_signal_handlers(&running, event_loop.get_signal())?;

    // Kick off the first frame.
    state.handle.insert_idle(|data| data.render_surface());

    while state.running.load(Ordering::SeqCst) {
        // Anything `ruster.wm.*` queued since the last pass. The DRM path
        // repaints on vblank rather than every iteration, and `dispatch`
        // reconfigures tiles, so a queued layout change lands on the next frame.
        state.drain_wm_commands();
        // A half-typed chord that is never finished has to clear itself, or the
        // overlay stays up and the next key is still being read as part of it.
        state.chord.expire(std::time::Instant::now());
        // A dispatch failure used to drop out of the loop and return `Ok`,
        // so the compositor exited 0 with nothing on stdout and no line in the
        // log — indistinguishable from a clean quit, and impossible to
        // diagnose from a VT where the screen is already gone.
        if let Err(err) = event_loop.dispatch(Some(Duration::from_millis(16)), &mut state) {
            error!("event loop dispatch failed: {err}");
            return Err(anyhow::anyhow!("event loop dispatch failed: {err}"));
        }
        if let Err(err) = state.display_handle.flush_clients() {
            error!("failed to flush wayland clients: {err}");
        }
    }
    // Before the device goes back: this is the last point at which the layout
    // still exists, and it is after every way out of the loop rather than in the
    // quit action, so a SIGTERM'd session is saved too.
    state.save_session();
    // Hand the device back before anything drops.
    //
    // smithay's atomic `Drop` otherwise replays every property it captured at
    // startup — connectors, CRTCs, framebuffers and planes — in one commit, to
    // put the console back the way it found it. The kernel rejects that with
    // `Invalid argument`, every time, because the capture includes immutable
    // properties (a connector's EDID and PATH among them) and an atomic commit
    // that sets an immutable property is invalid by definition. The result was
    // an `ERROR Failed to restore previous state` on every single exit.
    //
    // `pause()` is smithay's documented way to say we are finished with the
    // file descriptor — "avoid making calls to the file descriptor e.g. on
    // drop" — and it releases the DRM master lock, which is the handover the
    // console driver actually waits for. Nothing is lost by skipping the
    // restore: it never once succeeded, and the display has always come back,
    // so dropping master was doing the work all along.
    state.backend_data.drm_output_manager.pause();
    info!("released the drm device");
    info!("shutting down drm backend");
    Ok(())
}

impl CompositorState<RusterUdevData> {
    /// Composite the focused toplevel and chrome onto the DRM output and queue
    /// the frame for scan-out. Called from the initial idle kick, the repaint
    /// timers and the session-activate handler. When nothing is rendered
    /// (no damage) or the frame fails to queue, a repaint is re-scheduled for
    /// the next frame to keep polling for damage.
    fn render_surface(&mut self) {
        // While the session is paused — the user is on another VT — the DRM
        // device is inactive and every frame fails. Rendering anyway spun at
        // full frame rate producing `PrepareFrame(DeviceInactive)`: 11,746
        // warnings in five minutes on the first hardware boot that switched
        // away, 96% of the log. On a VT the log is the only diagnostic channel
        // there is, so drowning it is worse than the wasted work.
        //
        // Nothing is rescheduled here: `ActivateSession` kicks a render when
        // the session comes back, which is the only moment there is anything
        // new to draw.
        if !self.backend_data.session.is_active() {
            return;
        }
        let output = self.backend_data.output.clone();
        // Read before the renderer is acquired: `single_renderer` borrows
        // `self` mutably for the rest of the frame.
        let geometry = self.geometry();
        let tree_status = self.tree_status();
        let focused_title = self
            .shell
            .focused()
            .map(|w| w.title.clone())
            .unwrap_or_default();

        let mut renderer = match self
            .backend_data
            .gpus
            .single_renderer(&self.backend_data.render_node)
        {
            Ok(renderer) => renderer,
            Err(err) => {
                warn!("failed to acquire drm renderer: {err:?}");
                self.schedule_render(frame_duration(&output));
                return;
            }
        };
        let cursor_status = self.cursor_status.clone();
        let cursor_location = self.pointer.current_location();
        let scene = FrameInput {
            focus: self.shell.focus,
            toplevels: &self.toplevels,
            output: &output,
            workspace: self.workspaces.active(),
            focused_title: &focused_title,
            cursor_status: &cursor_status,
            cursor_location,
            geometry: &geometry,
            tree_status,
            panes: &self.panes,
            buffers: &self.buffers,
            keymap: &self.keymap,
            minibuffer: self.minibuffer.as_ref(),
            whichkey: crate::keymap::whichkey_view(
                &self.keymap,
                &self.chord,
                crate::keymap::HelpState {
                    pinned: self.help_pinned,
                },
            ),
        };
        let elements = collect_render_elements(&scene, &mut self.chrome, &mut renderer);
        send_frame_callbacks(&geometry, &self.toplevels, &output);

        let reschedule = match self.backend_data.drm_output.render_frame(
            &mut renderer,
            &elements,
            CLEAR_COLOR,
            FrameFlags::DEFAULT,
        ) {
            Ok(result) => {
                // Before `queue_frame`, which takes the output mutably and so
                // ends the borrow this result holds on it.
                if self.screenshot_pending {
                    self.screenshot_pending = false;
                    self.screenshot_count += 1;
                    let path = crate::screenshot::capture_path(self.screenshot_count);
                    let size = output.current_mode().map(|m| m.size).unwrap_or_default();
                    // `Transform::Normal`: unlike winit, the DRM output carries
                    // no transform, so the composited frame is already the way
                    // up the display shows it.
                    match crate::screenshot::capture_drm_frame(
                        &result,
                        &mut renderer,
                        size,
                        smithay::utils::Transform::Normal,
                        &path,
                    ) {
                        Ok(path) => info!(path = %path.display(), "screenshot"),
                        Err(err) => warn!("screenshot failed: {err}"),
                    }
                }
                if result.is_empty {
                    true
                } else {
                    match self.backend_data.drm_output.queue_frame(()) {
                        Ok(_) => false,
                        Err(err) => {
                            warn!("failed to queue drm frame: {err:?}");
                            true
                        }
                    }
                }
            }
            Err(err) => {
                warn!("failed to render drm frame: {err:?}");
                true
            }
        };

        if reschedule {
            self.schedule_render(frame_duration(&output));
        }
    }

    /// Called from the DRM device notifier on every VBlank to mark the queued
    /// frame as presented and schedule the next repaint. The repaint is
    /// scheduled at ~60% of a frame duration (anvil's latency split: the client
    /// gets the head start, the compositor repaints into the tail before the
    /// next VBlank).
    fn frame_finish(&mut self) {
        let output = self.backend_data.output.clone();
        match self.backend_data.drm_output.frame_submitted() {
            Ok(_) => {
                let delay = frame_duration(&output).mul_f64(0.6);
                self.schedule_render(delay);
            }
            Err(err) => {
                warn!("drm frame submission failed: {err:?}");
                self.schedule_render(frame_duration(&output));
            }
        }
    }

    /// Schedule a repaint after `delay` via a one-shot calloop timer.
    fn schedule_render(&mut self, delay: Duration) {
        self.handle
            .insert_source(
                Timer::from_duration(delay),
                move |_, _, data: &mut CompositorState<RusterUdevData>| {
                    data.render_surface();
                    TimeoutAction::Drop
                },
            )
            .expect("failed to schedule drm render timer");
    }
}

/// Duration of one retrace of the output at its current mode.
fn frame_duration(output: &Output) -> Duration {
    output
        .current_mode()
        .map(|mode| Duration::from_secs_f64(1_000f64 / mode.refresh as f64))
        .unwrap_or_else(|| Duration::from_millis(16))
}

/// Resolve the GPU to drive for the seat. Honors the `RUSTER_DRM_DEVICE`
/// environment override (mirroring anvil's `ANVIL_DRM_DEVICE`), else the
/// seat's primary GPU, else the first GPU enumerated for the seat. The
/// returned node is the *card* node used for device discovery; the render node
/// is derived from EGL during init and is what all rendering uses.
fn resolve_gpu_node(seat_name: &str, env_device: Option<&str>) -> anyhow::Result<DrmNode> {
    if let Some(dev) = env_device.filter(|dev| !dev.is_empty()) {
        return DrmNode::from_path(dev)
            .map_err(|err| anyhow::anyhow!("invalid RUSTER_DRM_DEVICE {dev}: {err}"));
    }
    primary_gpu(seat_name)
        .map_err(|err| anyhow::anyhow!("failed to enumerate gpus for seat {seat_name}: {err}"))?
        .and_then(|path| DrmNode::from_path(path).ok())
        .or_else(|| {
            all_gpus(seat_name)
                .unwrap_or_default()
                .into_iter()
                .find_map(|path| DrmNode::from_path(path).ok())
        })
        .ok_or_else(|| anyhow::anyhow!("no drm device found on seat {seat_name}"))
}

/// Find the device path in the udev device list backing `primary_gpu`, falling
/// back to the first entry for the seat.
fn udev_device_path(udev_backend: &UdevBackend, primary_gpu: &DrmNode) -> Option<PathBuf> {
    udev_backend
        .device_list()
        .find(|(device_id, _)| *device_id == primary_gpu.dev_id())
        .map(|(_, path)| path.to_path_buf())
        .or_else(|| {
            udev_backend
                .device_list()
                .next()
                .map(|(_, path)| path.to_path_buf())
        })
}

/// Find the first connected connector on the device, its preferred mode and a
/// crtc it can be driven by. Returns the connector, crtc and DRM mode.
fn find_connected_connector(
    drm_output_manager: &UdevOutputManager,
) -> anyhow::Result<(connector::Info, crtc::Handle, control::Mode)> {
    let device = drm_output_manager.device();
    let resources = device.resource_handles()?;

    let (connector, handle) = resources
        .connectors()
        .iter()
        .copied()
        .find_map(|handle| {
            let info = device.get_connector(handle, true).ok()?;
            (info.state() == connector::State::Connected).then_some((info, handle))
        })
        .ok_or_else(|| anyhow::anyhow!("no connected connector found on the primary gpu"))?;

    let drm_mode = connector
        .modes()
        .iter()
        .find(|mode| mode.mode_type().contains(ModeTypeFlags::PREFERRED))
        .or_else(|| connector.modes().first())
        .copied()
        .ok_or_else(|| {
            anyhow::anyhow!("connector {} has no modes", connector.interface().as_str())
        })?;

    // Pick the first crtc the connector's encoders can drive.
    let crtc = connector
        .encoders()
        .iter()
        .copied()
        .find_map(|encoder| {
            let possible = device.get_encoder(encoder).ok().and_then(|info| {
                resources
                    .filter_crtcs(info.possible_crtcs())
                    .first()
                    .copied()
            });
            possible
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no free crtc for connector {}",
                connector.interface().as_str()
            )
        })?;

    info!(connector = ?handle, ?crtc, "found connected connector");
    Ok((connector, crtc, drm_mode))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_model_name_has_ruster_prefix() {
        assert_eq!(output_model("sda"), "ruster-drm-sda");
    }

    #[test]
    fn output_model_prefixes_connector() {
        assert_eq!(output_model("eDP-1"), "ruster-drm-eDP-1");
    }

    #[test]
    fn supported_formats_include_8bit_fallback() {
        assert!(SUPPORTED_FORMATS.contains(&Fourcc::Argb8888));
    }

    #[test]
    fn env_device_override_rejects_bad_path() {
        let err = resolve_gpu_node("seat0", Some("/nonexistent/drm/ruster-card99")).unwrap_err();
        assert!(
            err.to_string().contains("RUSTER_DRM_DEVICE"),
            "unexpected error: {err}"
        );
    }
}
