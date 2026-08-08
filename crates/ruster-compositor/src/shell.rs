//! xdg-shell handling: maps toplevel surfaces into the shell's window list,
//! propagates title changes, and tears windows down on unmap/destroy.
//!
//! Phase 0 treats this handler as the event sink: `new_toplevel` records the
//! surface and adds a `ClientWindow`, and map/unmap is tracked from
//! `CompositorHandler::commit` (see [`CommitBuffer`]). Rendering, focus, and
//! keyboard forwarding are Tasks 7 and 10.

use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::utils::{Serial, SERIAL_COUNTER as SCOUNTER};
use smithay::wayland::compositor::{self, BufferAssignment};
use smithay::wayland::shell::xdg::{
    decoration::XdgDecorationHandler, PopupSurface, PositionerState, ToplevelSurface,
    XdgShellHandler, XdgShellState, XdgToplevelSurfaceData,
};

use crate::backend::Backend;
use crate::compositor::CompositorState;
use ruster_shell::Layout;

impl<B: Backend + 'static> XdgShellHandler for CompositorState<B> {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        // The client usually sends `set_title` only right after creating the
        // toplevel, so the title is typically not set yet here; it lands in
        // `title_changed`. Start the window record with whatever we have.
        let title = toplevel_title(&surface).unwrap_or_default();
        let id = self.shell.add_window(title, 800, 600);
        // Insert beside whatever has focus, on the workspace being shown, so a
        // new window splits the one you were looking at rather than appearing
        // somewhere arbitrary — or on a workspace you are not watching.
        let near = self.shell.focus;
        self.workspaces.insert(id, near, Layout::Horizontal);
        self.shell.set_focus(id);
        self.toplevels.insert(id, surface);
        self.pending_focus = Some(id);
        // Every existing window just got smaller; tell them before the new one
        // draws, or the first frame overlaps its neighbour.
        self.reconfigure_tiles();
        tracing::info!(?id, "new toplevel");
        // No configure is sent here: per the xdg protocol the initial
        // configure is sent on the first commit (anvil does the same).
        // `CompositorHandler::commit` sends it there, sized fullscreen to the
        // output's logical size. Keyboard focus is applied on the first
        // commit's map transition (`CompositorHandler::commit`), which
        // consumes `pending_focus`.
    }

    fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {
        // TODO(next phase): track popups relative to their parent toplevel.
        tracing::debug!("new popup (untracked in Phase 0)");
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: Serial) {}

    fn reposition_request(
        &mut self,
        _surface: PopupSurface,
        _positioner: PositionerState,
        _token: u32,
    ) {
    }

    fn title_changed(&mut self, surface: ToplevelSurface) {
        let Some(title) = toplevel_title(&surface) else {
            return;
        };
        let Some(id) = self.window_for_surface(surface.wl_surface()) else {
            return;
        };
        if let Some(window) = self.shell.window(id) {
            window.set_title(title);
        }
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        let Some(id) = self.window_for_surface(surface.wl_surface()) else {
            return;
        };
        self.toplevels.remove(&id);
        self.mapped.remove(&id);
        // Wherever it was: a client can close while its workspace is hidden.
        self.workspaces.remove(id);
        // `remove_window` refocuses the shell onto the most recent window (or
        // clears focus), but it knows nothing of workspaces and will happily
        // name one that is off screen; the workspaces have the last word.
        self.shell.remove_window(id);
        self.shell.focus = self.workspaces.focus_for_active(self.shell.focus);
        // The survivors grow into the space; they have to be told.
        self.reconfigure_tiles();
        self.update_keyboard_focus(SCOUNTER.next_serial());
        tracing::info!(?id, "toplevel destroyed");
    }
}

/// Decoration: ruster answers every client the same way, because the answer is
/// a fact about the compositor rather than a negotiation. `Chrome` already draws
/// a border around each tile and the statusline already names the focused
/// window, so a client titlebar is the second copy of both — drawn *inside* the
/// border, where it reads as a mistake.
impl<B: Backend + 'static> XdgDecorationHandler for CompositorState<B> {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        answer_decoration(&toplevel, None);
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: Mode) {
        answer_decoration(&toplevel, Some(mode));
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        answer_decoration(&toplevel, None);
    }
}

/// The mode ruster replies with, whatever the client asked for.
///
/// A separate function from the three handler methods above so the policy — no
/// client-side decorations, ever — is one statement that a test can pin, rather
/// than three arms that could drift apart.
fn decoration_mode(_requested: Option<Mode>) -> Mode {
    Mode::ServerSide
}

/// Reply to a decoration request on `toplevel`, `requested` being the mode the
/// client asked for if it named one.
///
/// Nothing is sent before the initial configure: `send_configure` marks the
/// initial configure as sent, and `CompositorHandler::commit` uses exactly that
/// flag to decide when to send the *sized* first configure. Answering the
/// decoration request immediately would therefore consume the flag and leave
/// the client sized to its own guess forever. The mode is instead left pending
/// and rides out on that first configure — smithay emits the decoration
/// configure ahead of the xdg one, which is the order the protocol requires.
fn answer_decoration(toplevel: &ToplevelSurface, requested: Option<Mode>) {
    let mode = decoration_mode(requested);
    toplevel.with_pending_state(|state| state.decoration_mode = Some(mode));
    if toplevel.is_initial_configure_sent() {
        toplevel.send_pending_configure();
    }
}

/// The client-provided title of a toplevel, read from its role attributes.
///
/// `ToplevelSurface::with_cached_state` does not expose the title — the cached
/// xdg state only tracks the last-acked configure — so it is read from
/// `XdgToplevelSurfaceData` (the non-double-buffered role attributes) instead.
fn toplevel_title(surface: &ToplevelSurface) -> Option<String> {
    compositor::with_states(surface.wl_surface(), |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .and_then(|data| data.lock().unwrap().title.clone())
    })
}

/// How a `wl_surface` commit affects a toplevel's mapped state, mirroring the
/// xdg-shell protocol (and smithay's xdg pre-commit hook): attaching a buffer
/// maps the toplevel, attaching a null buffer unmaps it, and a commit that
/// attaches nothing leaves the current state untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommitBuffer {
    /// A buffer was attached this commit.
    Attached,
    /// A null buffer was attached, unmapping the surface.
    Removed,
    /// This commit attached nothing.
    Unchanged,
}

impl From<&Option<BufferAssignment>> for CommitBuffer {
    fn from(buffer: &Option<BufferAssignment>) -> Self {
        match buffer {
            Some(BufferAssignment::NewBuffer(_)) => CommitBuffer::Attached,
            Some(BufferAssignment::Removed) => CommitBuffer::Removed,
            None => CommitBuffer::Unchanged,
        }
    }
}

impl CommitBuffer {
    /// The mapped state of the toplevel after this commit.
    pub(crate) fn is_mapped(self, currently_mapped: bool) -> bool {
        match self {
            CommitBuffer::Attached => true,
            CommitBuffer::Removed => false,
            CommitBuffer::Unchanged => currently_mapped,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruster_shell::ShellState;

    #[test]
    fn title_updates_flow_into_shell_state() {
        let mut shell = ShellState::new();
        let id = shell.add_window("init".into(), 100, 100);
        shell.window(id).unwrap().set_title("foot".into());
        assert_eq!(shell.window(id).unwrap().title, "foot");
    }

    #[test]
    fn unmap_of_nonfocused_window_keeps_focus() {
        let mut shell = ShellState::new();
        let a = shell.add_window("a".into(), 100, 100);
        let b = shell.add_window("b".into(), 100, 100);
        shell.set_focus(a);
        shell.remove_window(b);
        assert_eq!(shell.focused().unwrap().id, a);
    }

    #[test]
    fn committing_a_buffer_maps_an_unmapped_toplevel() {
        let mut mapped = false;
        mapped = CommitBuffer::Attached.is_mapped(mapped);
        assert!(mapped);
    }

    #[test]
    fn committing_a_null_buffer_unmaps_a_mapped_toplevel() {
        let mut mapped = true;
        mapped = CommitBuffer::Removed.is_mapped(mapped);
        assert!(!mapped);
    }

    #[test]
    fn commit_without_buffer_keeps_current_mapped_state() {
        assert!(CommitBuffer::Unchanged.is_mapped(true));
        assert!(!CommitBuffer::Unchanged.is_mapped(false));
    }

    #[test]
    fn every_decoration_request_is_answered_server_side() {
        // The compositor draws the border and names the window in the
        // statusline; conceding to a client that asks for CSD would put a
        // second titlebar inside a tile that already has chrome.
        assert_eq!(decoration_mode(None), Mode::ServerSide);
        assert_eq!(decoration_mode(Some(Mode::ClientSide)), Mode::ServerSide);
        assert_eq!(decoration_mode(Some(Mode::ServerSide)), Mode::ServerSide);
    }

    #[test]
    fn buffer_assignment_maps_to_commit_buffer() {
        assert_eq!(CommitBuffer::from(&None), CommitBuffer::Unchanged);
        assert_eq!(
            CommitBuffer::from(&Some(
                smithay::wayland::compositor::BufferAssignment::Removed
            )),
            CommitBuffer::Removed
        );
    }
}
