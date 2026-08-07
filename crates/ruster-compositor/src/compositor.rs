use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use smithay::backend::renderer::utils::on_commit_buffer_handler;
use smithay::input::keyboard::{KeyboardHandle, XkbConfig};
use smithay::input::pointer::{CursorImageStatus, PointerHandle};
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::output::Output;
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{
    Interest, LoopHandle, LoopSignal, Mode as SourceMode, PostAction,
};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::{
    backend::{ClientData, ClientId, DisconnectReason},
    protocol::{wl_buffer, wl_output, wl_surface},
    Client, Display, DisplayHandle,
};
use smithay::utils::{Serial, SERIAL_COUNTER as SCOUNTER};
use smithay::wayland::socket::ListeningSocketSource;
use smithay::wayland::{
    buffer::BufferHandler,
    compositor::{
        with_states, CompositorClientState, CompositorHandler,
        CompositorState as WlCompositorState, SurfaceAttributes,
    },
    output::OutputHandler,
    selection::data_device::{
        ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
    },
    selection::SelectionHandler,
    shell::xdg::{ToplevelSurface, XdgShellState},
    shm::{ShmHandler, ShmState},
};
use tracing::info;

use crate::backend::{logical_output_size, Backend};
use crate::chrome::Chrome;
use crate::shell::CommitBuffer;
use ruster_render::Theme;
use ruster_shell::{Rect, ShellState, WindowId, Workspaces};

/// The compositor's composition root: everything the backend and the input
/// handlers need to reach. Mirrors anvil's `AnvilState` but trimmed to Phase 0.
pub struct CompositorState<B: Backend + 'static> {
    pub backend_data: B,
    pub display_handle: DisplayHandle,
    pub socket_name: Option<String>,
    pub running: Arc<AtomicBool>,
    pub handle: LoopHandle<'static, CompositorState<B>>,
    pub shell: ShellState,
    pub compositor_state: WlCompositorState,
    pub shm_state: ShmState,
    pub xdg_shell_state: XdgShellState,
    pub data_device_state: DataDeviceState,
    pub seat_state: SeatState<CompositorState<B>>,
    pub seat: Seat<CompositorState<B>>,
    pub keyboard: KeyboardHandle<CompositorState<B>>,
    pub pointer: PointerHandle<CompositorState<B>>,
    /// What the pointer should currently look like. Starts as the default
    /// named cursor, which the compositor draws itself; a client focusing the
    /// pointer replaces it with its own surface.
    pub cursor_status: CursorImageStatus,
    /// How the mapped windows divide the output: one container tree per
    /// workspace, of which exactly one is on screen. Every question about where
    /// a window is — or whether it is anywhere at all — is answered by the
    /// active tree, so the eight hidden ones cost nothing but memory.
    pub workspaces: Workspaces,
    /// xdg toplevel surfaces keyed by their `ShellState` window id.
    pub toplevels: HashMap<WindowId, ToplevelSurface>,
    /// Window that should take focus once the seat is set up (Task 10).
    pub pending_focus: Option<WindowId>,
    /// Toplevels that have committed a buffer and are thus rendered (Task 7).
    pub mapped: HashSet<WindowId>,
    /// The compositor's UI chrome (statusline, editor frame, which-key), drawn
    /// above the client surfaces (Task 8).
    pub chrome: Option<Chrome>,
    /// Set by the screenshot keybind, cleared by the render loop once it has
    /// captured. The capture needs the renderer and the finished framebuffer,
    /// neither of which the key handler has, so the request has to wait for the
    /// frame rather than be served where it is made.
    pub screenshot_pending: bool,
    /// How many captures this session has taken, so they do not overwrite.
    pub screenshot_count: u32,
    /// Configured WM keybinds as `(binding, action)` pairs, loaded from
    /// `compositor.lua` (Task 9). Empty until the config is applied.
    pub keybinds: Vec<(String, String)>,
}

impl<B: Backend + 'static> CompositorState<B> {
    pub fn seat_name(&self) -> String {
        self.backend_data.seat_name()
    }

    /// Apply the shell's focus to the seat keyboard: the surface of the focused
    /// toplevel becomes the keyboard focus, or focus is cleared when there is
    /// none. Consumes `pending_focus` — the window that should take focus once
    /// the seat is up — falling back to the shell's tracked focus. Only
    /// [`focusable`](Self::focusable) windows are considered, so a click, a
    /// destroyed-but-not-yet-committed window or a workspace switch can never
    /// grab the keyboard for a surface that is not on screen.
    pub fn update_keyboard_focus(&mut self, serial: Serial) {
        let focus = self
            .pending_focus
            .take()
            .filter(|id| self.focusable(*id))
            .or_else(|| self.shell.focus.filter(|id| self.focusable(*id)))
            .and_then(|id| self.toplevels.get(&id))
            .map(|toplevel| toplevel.wl_surface().clone());
        let keyboard = self.keyboard.clone();
        keyboard.set_focus(self, focus, serial);
    }

    /// Whether `window` may hold the keyboard: it has committed a buffer, and
    /// it is on the workspace currently on screen. Both halves are about the
    /// same failure — keystrokes disappearing into a window the user is not
    /// looking at, with no way to tell where they went.
    fn focusable(&self, window: WindowId) -> bool {
        self.mapped.contains(&window) && self.workspaces.is_visible(window)
    }
}

impl<B: Backend + 'static> CompositorState<B> {
    /// The output's whole area, in logical pixels, as the tree's root rectangle.
    pub fn output_rect(&self) -> Rect {
        let size = logical_output_size(self.backend_data.output()).unwrap_or_default();
        Rect::new(0, 0, size.w, size.h)
    }

    /// What the statusline should say about the layout: the axis of the split
    /// holding the focused window, how many windows share the workspace, and
    /// whether the focused one floats.
    pub fn tree_status(&self) -> crate::chrome::TreeStatus {
        let focus = self.shell.focus;
        crate::chrome::TreeStatus {
            layout: focus.and_then(|id| self.workspaces.layout_at(id)),
            windows: self.workspaces.visible_count(),
            floating: focus.is_some_and(|id| self.workspaces.is_floating(id)),
        }
    }

    /// Where each window on the active workspace sits, per its tree.
    ///
    /// This is the whole of "switching workspaces hides the rest": the
    /// renderer, the pointer and `reconfigure_tiles` all read this list and
    /// nothing else, so a window on one of the other eight has no rectangle to
    /// be drawn at, clicked on, or resized to.
    pub fn geometry(&self) -> Vec<(WindowId, Rect)> {
        self.workspaces.layout(self.output_rect())
    }

    /// The rectangle assigned to one window, if the tree holds it.
    pub fn window_rect(&self, id: WindowId) -> Option<Rect> {
        self.geometry()
            .into_iter()
            .find(|(w, _)| *w == id)
            .map(|(_, r)| r)
    }

    /// Tell every mapped client the size its leaf now has.
    ///
    /// Called whenever the tree changes shape, because inserting or removing one
    /// window resizes its neighbours too — a client that is never told keeps
    /// drawing at its old size and either overlaps the window beside it or
    /// leaves a gap.
    pub fn reconfigure_tiles(&mut self) {
        for (id, rect) in self.geometry() {
            let Some(toplevel) = self.toplevels.get(&id) else {
                continue;
            };
            toplevel.with_pending_state(|state| {
                state.size = Some((rect.w, rect.h).into());
                // Tiled on every edge: the honest way to tell a client it does
                // not own its borders, so it drops rounded corners and shadows.
                // Server-side decoration is Phase 2's frame theming.
                state.states.set(xdg_toplevel::State::TiledLeft);
                state.states.set(xdg_toplevel::State::TiledRight);
                state.states.set(xdg_toplevel::State::TiledTop);
                state.states.set(xdg_toplevel::State::TiledBottom);
            });
            toplevel.send_pending_configure();
        }
    }

    /// Show `workspace` and hide whatever was on screen.
    pub fn switch_workspace(&mut self, workspace: u32) {
        if self.workspaces.switch_to(workspace) {
            self.visible_windows_changed();
        }
    }

    /// Show the next workspace, wrapping — the `M-t` binding.
    pub fn cycle_workspace(&mut self) {
        if self.workspaces.cycle() {
            self.visible_windows_changed();
        }
    }

    /// Send `window` to `workspace`. It keeps its client and its buffer; it
    /// only stops being laid out here and starts being laid out there.
    pub fn move_to_workspace(&mut self, window: WindowId, workspace: u32) {
        if self.workspaces.move_to_workspace(window, workspace) {
            self.visible_windows_changed();
        }
    }

    /// Re-establish everything that depended on the previous set of on-screen
    /// windows. Every path that changes which windows are visible ends here.
    fn visible_windows_changed(&mut self) {
        // Focus is one handle across all nine workspaces, so it can be left
        // pointing at a window that is no longer drawn — after which every
        // keystroke goes to a client the user cannot see.
        self.shell.focus = self.workspaces.focus_for_active(self.shell.focus);
        // A window that has been off screen was never told about the resizes
        // that happened while it was away, and one arriving from another
        // workspace has a rectangle it has never heard of. Either way it draws
        // at a stale size until it is configured.
        self.reconfigure_tiles();
        self.update_keyboard_focus(SCOUNTER.next_serial());
        // Ask for a full redraw rather than trusting incremental damage. The
        // whole screen changes at once here — every client surface plus the
        // workspace label in the statusline — and the backends swap between
        // several buffers, so the ones not drawn this frame would otherwise
        // keep showing the workspace we just left.
        let output = self.backend_data.output().clone();
        self.backend_data.reset_buffers(&output);
    }
}

/// The window that should take keyboard focus after `unmapped` hid itself:
/// the most recently mapped window still visible, or `None` when nothing else
/// is mapped. Mirrors `ShellState::remove_window`'s fall back to the last
/// remaining window, and is pure so the compositor's unmap path stays
/// unit-testable without a live display.
fn next_focus_after_unmap(mapped: &HashSet<WindowId>, unmapped: WindowId) -> Option<WindowId> {
    mapped.iter().filter(|id| **id != unmapped).max().copied()
}

/// Globals created for a display; bundled so `create_state` can build them
/// before the state struct itself exists (anvil does this inline in `init`).
struct InitGlobals<B: Backend + 'static> {
    compositor_state: WlCompositorState,
    shm_state: ShmState,
    xdg_shell_state: XdgShellState,
    data_device_state: DataDeviceState,
    seat_state: SeatState<CompositorState<B>>,
    seat: Seat<CompositorState<B>>,
    keyboard: KeyboardHandle<CompositorState<B>>,
    pointer: PointerHandle<CompositorState<B>>,
}

/// Create the compositor state for a freshly minted [`Display`]: registers the
/// client dispatch source on the event loop, then inserts the core globals
/// (`wl_compositor`, `wl_shm`, `xdg_wm_base`, `wl_seat` + pointer/keyboard).
pub fn create_state<B: Backend + 'static>(
    display: Display<CompositorState<B>>,
    handle: LoopHandle<'static, CompositorState<B>>,
    backend_data: B,
) -> CompositorState<B> {
    let dh = display.handle();

    handle
        .insert_source(
            Generic::new(display, Interest::READ, SourceMode::Level),
            |_, display, state| {
                // Safety: the display is owned by the event loop source and thus
                // outlives every dispatch call.
                unsafe {
                    display.get_mut().dispatch_clients(state).unwrap();
                }
                Ok(PostAction::Continue)
            },
        )
        .expect("failed to init wayland server source");

    let globals = init_globals(&dh, backend_data.seat_name());

    CompositorState {
        backend_data,
        display_handle: dh,
        socket_name: None,
        running: Arc::new(AtomicBool::new(true)),
        handle,
        shell: ShellState::new(),
        compositor_state: globals.compositor_state,
        shm_state: globals.shm_state,
        xdg_shell_state: globals.xdg_shell_state,
        data_device_state: globals.data_device_state,
        seat_state: globals.seat_state,
        seat: globals.seat,
        keyboard: globals.keyboard,
        pointer: globals.pointer,
        cursor_status: CursorImageStatus::default_named(),
        workspaces: Workspaces::new(),
        toplevels: HashMap::new(),
        pending_focus: None,
        mapped: HashSet::new(),
        chrome: Some(Chrome::new(Theme::default())),
        screenshot_pending: false,
        screenshot_count: 0,
        keybinds: Vec::new(),
    }
}

/// Insert the core display globals for `CompositorState<B>`.
fn init_globals<B: Backend + 'static>(dh: &DisplayHandle, seat_name: String) -> InitGlobals<B> {
    let compositor_state = WlCompositorState::new::<CompositorState<B>>(dh);
    let shm_state = ShmState::new::<CompositorState<B>>(dh, vec![]);
    let xdg_shell_state = XdgShellState::new::<CompositorState<B>>(dh);
    // `wl_data_device_manager` carries the clipboard. It is not optional in
    // practice: foot (and other toolkits) treat a missing manager as fatal and
    // exit before they ever map a surface, so without this global no client
    // reaches the compositor at all.
    let data_device_state = DataDeviceState::new::<CompositorState<B>>(dh);
    let mut seat_state = SeatState::new();
    let mut seat = seat_state.new_wl_seat(dh, seat_name);
    let pointer = seat.add_pointer();
    // TODO(next phase): load an XKB config from ~/.config/ruster/ when present.
    let keyboard = seat
        .add_keyboard(XkbConfig::default(), 200, 25)
        .expect("failed to initialize the keyboard");

    InitGlobals {
        compositor_state,
        shm_state,
        xdg_shell_state,
        data_device_state,
        seat_state,
        seat,
        keyboard,
        pointer,
    }
}

/// Per-client data. `ClientData` is the only hook wayland_server gives us to
/// attach state to a client; the compositor client state is what smithay's
/// `wl_compositor` handler reads back in `client_compositor_state`.
#[derive(Debug, Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

impl<B: Backend + 'static> CompositorHandler for CompositorState<B> {
    fn compositor_state(&mut self) -> &mut WlCompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client
            .get_data::<ClientState>()
            .expect("client has no ClientState")
            .compositor_state
    }

    fn commit(&mut self, surface: &wl_surface::WlSurface) {
        // Read the buffer assignment BEFORE importing it. This ordering is load
        // bearing: `on_commit_buffer_handler` calls `attrs.buffer.take()`, so
        // once it has run the surface reports no buffer this commit and the
        // map/unmap detection below sees `Unchanged` forever. A toplevel then
        // never enters `self.mapped` — it still renders, because that path keys
        // off `focus`, but `update_keyboard_focus` and `surface_under` both
        // filter on `mapped`, so the client silently receives no keyboard or
        // pointer input at all.
        let commit_buffer = with_states(surface, |states| {
            CommitBuffer::from(
                &states
                    .cached_state
                    .get::<SurfaceAttributes>()
                    .current()
                    .buffer,
            )
        });

        // Import the newly attached buffer into the surface's renderer state.
        // Nothing else does this, and without it `SurfaceTree` has no texture to
        // hand the renderer: the client maps, gets configures and frame
        // callbacks, and still draws as an empty region. Runs for every surface,
        // before the toplevel lookup below, because subsurfaces need it too.
        on_commit_buffer_handler::<Self>(surface);

        // A toplevel becomes "mapped" once its client commits a buffer, and
        // unmapped again on a null-buffer commit. Map/unmap is tracked here
        // (see `CommitBuffer`); the render loop draws mapped surfaces in
        // Task 7.
        let Some(id) = self
            .toplevels
            .iter()
            .find(|(_, t)| t.wl_surface() == surface)
            .map(|(id, _)| *id)
        else {
            return;
        };
        // Send the initial configure once, in response to the client's first
        // commit — which is usually buffer-less, so this runs before the
        // map/unmap match below. Spec-compliant clients wait for
        // `xdg_surface.configure` before mapping; `send_configure` flags
        // `initial_configure_sent` internally, so later commits don't
        // re-send. The size is the window's leaf rectangle in the tree, not the
        // whole output — with one window those are the same thing, which is why
        // Phase 0 could get away with the output size.
        let rect = self.window_rect(id);
        if let Some(toplevel) = self.toplevels.get(&id) {
            if !toplevel.is_initial_configure_sent() {
                if let Some(rect) = rect {
                    toplevel.with_pending_state(|state| {
                        state.size = Some((rect.w, rect.h).into());
                        state.states.set(xdg_toplevel::State::TiledLeft);
                        state.states.set(xdg_toplevel::State::TiledRight);
                        state.states.set(xdg_toplevel::State::TiledTop);
                        state.states.set(xdg_toplevel::State::TiledBottom);
                    });
                }
                toplevel.send_configure();
            }
        }
        let was_mapped = self.mapped.contains(&id);
        let is_mapped = commit_buffer.is_mapped(was_mapped);
        match (was_mapped, is_mapped) {
            (false, true) => {
                self.mapped.insert(id);
                info!(?id, "toplevel mapped");
                // The surface just became visible; hand the seat keyboard to it
                // (consuming `pending_focus` if this is a fresh toplevel).
                self.update_keyboard_focus(SCOUNTER.next_serial());
            }
            (true, false) => {
                if self.shell.focus == Some(id) {
                    // The focused toplevel hid itself; hand the keyboard to the
                    // most recently mapped window still visible (mirroring
                    // `remove_window`'s fall back to the last remaining
                    // window), or clear it when nothing is left. Only windows on
                    // the workspace being shown are candidates — a mapped window
                    // on a hidden one is no more use to the keyboard than the
                    // one that just disappeared.
                    let visible: HashSet<WindowId> = self
                        .mapped
                        .iter()
                        .copied()
                        .filter(|w| self.workspaces.is_visible(*w))
                        .collect();
                    if let Some(next) = next_focus_after_unmap(&visible, id) {
                        self.shell.set_focus(next);
                    }
                    self.update_keyboard_focus(SCOUNTER.next_serial());
                }
                self.mapped.remove(&id);
                info!(?id, "toplevel unmapped");
            }
            _ => {}
        }
    }
}

impl<B: Backend + 'static> ShmHandler for CompositorState<B> {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl<B: Backend + 'static> BufferHandler for CompositorState<B> {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

// The `XdgShellHandler` impl lives in `crate::shell`.

impl<B: Backend + 'static> OutputHandler for CompositorState<B> {
    fn output_bound(&mut self, _output: Output, _wl_output: wl_output::WlOutput) {}
}

impl<B: Backend + 'static> SeatHandler for CompositorState<B> {
    type KeyboardFocus = wl_surface::WlSurface;
    type PointerFocus = wl_surface::WlSurface;
    type TouchFocus = wl_surface::WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<CompositorState<B>> {
        &mut self.seat_state
    }

    /// A client asked for a different pointer image (or for none). Record it;
    /// the render loop draws whatever is current. Ignoring this callback is
    /// what left the pointer invisible: a client's `wl_pointer.set_cursor`
    /// went nowhere, and nothing else ever drew a cursor either.
    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        self.cursor_status = image;
    }
}

// Clipboard/DnD. Phase 0 takes smithay's default behaviour wholesale: the
// default `SelectionHandler` methods already route client-to-client copy/paste
// through the seat, and ruster has no server-side selection of its own yet, so
// the DnD grab handlers stay empty.
impl<B: Backend + 'static> SelectionHandler for CompositorState<B> {
    type SelectionUserData = ();
}

impl<B: Backend + 'static> ClientDndGrabHandler for CompositorState<B> {}

impl<B: Backend + 'static> ServerDndGrabHandler for CompositorState<B> {}

impl<B: Backend + 'static> DataDeviceHandler for CompositorState<B> {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

// One delegate per protocol we speak. Each wires the `Dispatch`/`GlobalDispatch`
// impls for that protocol's objects through to the smithay state we hold above,
// so the handler traits implemented in this file (and `XdgShellHandler` in
// `shell.rs`) are all the glue we write by hand.
smithay::delegate_compositor!(@<B: Backend + 'static> CompositorState<B>);
smithay::delegate_shm!(@<B: Backend + 'static> CompositorState<B>);
smithay::delegate_output!(@<B: Backend + 'static> CompositorState<B>);
smithay::delegate_seat!(@<B: Backend + 'static> CompositorState<B>);
smithay::delegate_xdg_shell!(@<B: Backend + 'static> CompositorState<B>);
smithay::delegate_data_device!(@<B: Backend + 'static> CompositorState<B>);

/// Register the auto-named Wayland client listening socket with the compositor's
/// event loop. Returns the socket name for clients to connect to (print it and
/// set `WAYLAND_DISPLAY` accordingly in the launch script).
pub fn init_listener<B: Backend + 'static>(state: &mut CompositorState<B>) -> String {
    let source = ListeningSocketSource::new_auto().unwrap();
    let socket_name = source.socket_name().to_string_lossy().into_owned();
    state.socket_name = Some(socket_name.clone());
    state
        .handle
        .insert_source(
            source,
            |client_stream, _, state: &mut CompositorState<B>| {
                info!(client = ?client_stream.peer_addr().ok(), "client connected");
                state
                    .display_handle
                    .insert_client(client_stream, Arc::new(ClientState::default()))
                    .unwrap();
            },
        )
        .expect("failed to register wayland listening socket");
    socket_name
}

/// Message printed when DRM hardware access fails, pointing the user at a
/// seat manager. Lives outside the udev-gated DRM backend so default builds
/// can print it too.
pub fn drm_error_hint() -> &'static str {
    "DRM access failed. Run the session under logind (normal) or start seatd, or use the winit backend for development."
}

/// Install SIGINT/SIGTERM handlers that flip `running` off and stop the
/// calloop loop (the loop's next dispatch returns immediately). The ctrlc
/// `termination` feature extends the default SIGINT-only handler to also
/// cover SIGTERM (and SIGHUP).
pub fn install_signal_handlers(
    running: &Arc<AtomicBool>,
    signal: LoopSignal,
) -> anyhow::Result<()> {
    let flag = running.clone();
    let stop = signal.clone();
    ctrlc::set_handler(move || {
        flag.store(false, Ordering::SeqCst);
        stop.stop();
    })?;
    Ok(())
}

/// Log a startup header naming the version, backend and Wayland socket.
pub fn log_startup_header(version: &str, backend: &str, socket_name: &str) {
    info!(
        ?version,
        ?backend,
        ?socket_name,
        "ruster-compositor starting"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::logical_size_from;
    use ruster_shell::WindowId;
    use smithay::utils::Size;
    use std::sync::atomic::Ordering;

    // Constructing a CompositorState requires a DisplayHandle; exercise the
    // parts that don't need one: ShellState lifecycle and the running flag.
    #[test]
    fn running_flag_defaults_true() {
        let running = Arc::new(AtomicBool::new(true));
        assert!(running.load(Ordering::Relaxed));
    }

    #[test]
    fn shell_state_rejects_unknown_focus() {
        let mut shell = ShellState::new();
        let id = shell.add_window("x".into(), 100, 100);
        shell.set_focus(WindowId(999));
        assert_eq!(shell.focused(), None);
        shell.set_focus(id);
        assert!(shell.focused().is_some());
    }

    #[test]
    fn error_hint_for_drm_failure_mentions_seatd() {
        let hint = drm_error_hint();
        assert!(hint.to_lowercase().contains("seatd") || hint.to_lowercase().contains("logind"));
    }

    #[test]
    fn logical_size_divides_physical_by_fractional_scale() {
        // The fullscreen toplevel is sized to the output's logical size, so
        // the initial configure must scale physical pixels down by the
        // current output scale (the winit helper's math, backend-agnostic).
        assert_eq!(
            logical_size_from(Size::from((1920, 1080)), 1.0),
            Size::from((1920, 1080))
        );
        assert_eq!(
            logical_size_from(Size::from((1920, 1080)), 2.0),
            Size::from((960, 540))
        );
        // Fractional scales round down to integer logical pixels.
        assert_eq!(
            logical_size_from(Size::from((1920, 1080)), 1.25),
            Size::from((1536, 864))
        );
    }

    #[test]
    fn unmapping_focused_toplevel_refocuses_last_mapped() {
        // The most recently mapped window takes over (mirroring
        // `ShellState::remove_window`'s fall back to the last remaining
        // window), so keyboard focus is never dropped onto an invisible
        // surface while another window is still visible.
        let mapped = HashSet::from([WindowId(0), WindowId(1), WindowId(3)]);
        assert_eq!(
            next_focus_after_unmap(&mapped, WindowId(0)),
            Some(WindowId(3))
        );
        assert_eq!(
            next_focus_after_unmap(&mapped, WindowId(3)),
            Some(WindowId(1))
        );
        // Unmapping the only mapped window leaves nothing to focus.
        assert_eq!(
            next_focus_after_unmap(&HashSet::from([WindowId(2)]), WindowId(2)),
            None
        );
    }
}
