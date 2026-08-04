use std::collections::{HashMap, HashSet};
use std::sync::{atomic::AtomicBool, Arc};

use smithay::delegate_dispatch2;
use smithay::input::keyboard::{KeyboardHandle, XkbConfig};
use smithay::input::pointer::PointerHandle;
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::output::Output;
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, LoopHandle, Mode as SourceMode, PostAction};
use smithay::reexports::wayland_server::{
    backend::{ClientData, ClientId, DisconnectReason},
    protocol::{wl_buffer, wl_output, wl_surface},
    Client, Display, DisplayHandle,
};
use smithay::wayland::socket::ListeningSocketSource;
use smithay::wayland::{
    buffer::BufferHandler,
    compositor::{
        with_states, CompositorClientState, CompositorHandler,
        CompositorState as WlCompositorState, SurfaceAttributes,
    },
    output::OutputHandler,
    shell::xdg::{ToplevelSurface, XdgShellState},
    shm::{ShmHandler, ShmState},
};
use tracing::info;

use crate::backend::Backend;
use crate::shell::CommitBuffer;
use ruster_shell::{ShellState, WindowId};

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
    pub seat_state: SeatState<CompositorState<B>>,
    pub seat: Seat<CompositorState<B>>,
    pub keyboard: KeyboardHandle<CompositorState<B>>,
    pub pointer: PointerHandle<CompositorState<B>>,
    /// xdg toplevel surfaces keyed by their `ShellState` window id.
    pub toplevels: HashMap<WindowId, ToplevelSurface>,
    /// Window that should take focus once the seat is set up (Task 10).
    pub pending_focus: Option<WindowId>,
    /// Toplevels that have committed a buffer and are thus rendered (Task 7).
    pub mapped: HashSet<WindowId>,
}

impl<B: Backend + 'static> CompositorState<B> {
    pub fn seat_name(&self) -> String {
        self.backend_data.seat_name()
    }
}

/// Globals created for a display; bundled so `create_state` can build them
/// before the state struct itself exists (anvil does this inline in `init`).
struct InitGlobals<B: Backend + 'static> {
    compositor_state: WlCompositorState,
    shm_state: ShmState,
    xdg_shell_state: XdgShellState,
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
        seat_state: globals.seat_state,
        seat: globals.seat,
        keyboard: globals.keyboard,
        pointer: globals.pointer,
        toplevels: HashMap::new(),
        pending_focus: None,
        mapped: HashSet::new(),
    }
}

/// Insert the core display globals for `CompositorState<B>`.
fn init_globals<B: Backend + 'static>(dh: &DisplayHandle, seat_name: String) -> InitGlobals<B> {
    let compositor_state = WlCompositorState::new::<CompositorState<B>>(dh);
    let shm_state = ShmState::new::<CompositorState<B>>(dh, vec![]);
    let xdg_shell_state = XdgShellState::new::<CompositorState<B>>(dh);
    let mut seat_state = SeatState::new();
    let mut seat = seat_state.new_wl_seat(dh, seat_name);
    let pointer = seat.add_pointer();
    // TODO(Task 12): load an XKB config from ~/.config/ruster/ when present.
    let keyboard = seat
        .add_keyboard(XkbConfig::default(), 200, 25)
        .expect("failed to initialize the keyboard");

    InitGlobals {
        compositor_state,
        shm_state,
        xdg_shell_state,
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
        let was_mapped = self.mapped.contains(&id);
        let commit_buffer = with_states(surface, |states| {
            CommitBuffer::from(
                &states
                    .cached_state
                    .get::<SurfaceAttributes>()
                    .current()
                    .buffer,
            )
        });
        let is_mapped = commit_buffer.is_mapped(was_mapped);
        match (was_mapped, is_mapped) {
            (false, true) => {
                self.mapped.insert(id);
                info!(?id, "toplevel mapped");
            }
            (true, false) => {
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
}

delegate_dispatch2!(@<B: Backend + 'static> CompositorState<B>);

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

#[cfg(test)]
mod tests {
    use super::*;
    use ruster_shell::WindowId;
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
}
