//! xdg-shell handling: maps toplevel surfaces into the shell's window list,
//! propagates title changes, and tears windows down on unmap/destroy.
//!
//! Phase 0 treats this handler as the event sink: `new_toplevel` records the
//! surface and adds a `ClientWindow`, and map/unmap is tracked from
//! `CompositorHandler::commit` (see [`CommitBuffer`]). Rendering, focus, and
//! keyboard forwarding are Tasks 7 and 10.

use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::utils::{Serial, SERIAL_COUNTER as SCOUNTER};
use smithay::wayland::compositor::{self, BufferAssignment};
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
    XdgToplevelSurfaceData,
};

use crate::backend::Backend;
use crate::compositor::CompositorState;

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
        self.shell.set_focus(id);
        self.toplevels.insert(id, surface);
        self.pending_focus = Some(id);
        tracing::info!(?id, "new toplevel");
        // No configure is sent here: per the xdg protocol the initial
        // configure is sent on the first commit (anvil does the same).
        // Keyboard focus is applied on the first commit's map transition
        // (`CompositorHandler::commit`), which consumes `pending_focus`.
    }

    fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {
        // TODO(Task 10+): track popups relative to their parent toplevel.
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
        let Some(id) = self
            .toplevels
            .iter()
            .find(|(_, t)| t.wl_surface() == surface.wl_surface())
            .map(|(id, _)| *id)
        else {
            return;
        };
        if let Some(window) = self.shell.window(id) {
            window.set_title(title);
        }
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        let Some(id) = self
            .toplevels
            .iter()
            .find(|(_, t)| t.wl_surface() == surface.wl_surface())
            .map(|(id, _)| *id)
        else {
            return;
        };
        self.toplevels.remove(&id);
        self.mapped.remove(&id);
        // `remove_window` refocuses the shell onto the most recent window (or
        // clears focus); re-apply that to the seat keyboard.
        self.shell.remove_window(id);
        self.update_keyboard_focus(SCOUNTER.next_serial());
        tracing::info!(?id, "toplevel destroyed");
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
