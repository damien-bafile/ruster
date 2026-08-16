//! xdg-shell handling: maps toplevel surfaces into the shell's window list,
//! propagates title changes, and tears windows down on unmap/destroy.
//!
//! Phase 0 treats this handler as the event sink: `new_toplevel` records the
//! surface and adds a `ClientWindow`, and map/unmap is tracked from
//! `CompositorHandler::commit` (see [`CommitBuffer`]). Rendering, focus, and
//! keyboard forwarding are Tasks 7 and 10.

use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::desktop::{find_popup_root_surface, PopupKeyboardGrab, PopupKind, PopupPointerGrab,
    PopupUngrabStrategy};
use smithay::input::pointer::Focus;
use smithay::input::Seat;
use smithay::utils::{Logical, Rectangle, Serial, SERIAL_COUNTER as SCOUNTER};
use smithay::wayland::compositor::{self, BufferAssignment};
use smithay::wayland::shell::xdg::{
    decoration::XdgDecorationHandler, PopupSurface, PositionerState, ToplevelSurface,
    XdgShellHandler, XdgShellState, XdgToplevelSurfaceData,
};

use crate::backend::Backend;
use crate::compositor::CompositorState;
use crate::focus::FocusTarget;
use ruster_shell::Layout;

/// Where a popup should sit, kept inside the output.
///
/// A menu opened near an edge asks for a rectangle that runs off the screen and
/// the protocol expects the compositor to bring it back. smithay's positioner
/// knows how the client wants that resolved — flip to the other side of the
/// anchor, slide along, or resize — so this hands it the output rectangle and
/// takes its answer rather than clamping by hand, which would ignore the
/// client's stated preference and put submenus on the wrong side of their
/// parent.
fn unconstrain_popup(
    positioner: &PositionerState,
    output: ruster_shell::Rect,
) -> Rectangle<i32, Logical> {
    let area = Rectangle::new(
        (output.x, output.y).into(),
        (output.w.max(1), output.h.max(1)).into(),
    );
    positioner.get_unconstrained_geometry(area)
}

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
        // somewhere arbitrary — or on a workspace you are not watching. Unless
        // the saved session was waiting for this client, in which case it goes
        // back where it was, which may not be this workspace at all.
        let near = self.shell.focus;
        let pid = crate::persist::client_pid(&surface, &self.display_handle);
        if !self.place_restored_window(id, pid) {
            self.workspaces.insert(id, near, Layout::Horizontal);
        }
        // A restored window can land on a workspace that is not on screen, and
        // it must not take the keyboard there: every keystroke would go to a
        // client the user cannot see. Until now every new window was inserted on
        // the active workspace, so this could not arise.
        if self.workspaces.is_visible(id) {
            self.shell.set_focus(id);
            self.pending_focus = Some(id);
        } else {
            self.shell.focus = self.workspaces.focus_for_active(self.shell.focus);
        }
        self.clients.insert(id, surface.into());
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

    fn new_popup(&mut self, surface: PopupSurface, positioner: PositionerState) {
        // Untracked until now, which meant every client menu and tooltip was
        // simply not drawn — the surface existed and the client believed it was
        // on screen, so a right-click opened a menu nobody could see and the
        // next click went to whatever was behind it.
        //
        // The positioner is unconstrained against the output first: a menu
        // opened near an edge asks for a position that runs off the screen, and
        // the protocol expects the compositor to flip or slide it back. Without
        // this the bottom of every menu near the bottom of the screen is
        // unreachable.
        let output = self.output_rect();
        let geometry = unconstrain_popup(&positioner, output);
        surface.with_pending_state(|state| {
            state.geometry = geometry;
        });
        if let Err(err) = self.popups.track_popup(PopupKind::Xdg(surface)) {
            tracing::warn!(%err, "could not track popup");
        } else {
            // Only the failure was audible before, which is the wrong half: the
            // bug this replaced was a popup that never appeared, and a silent
            // success is indistinguishable from a client that never asked. The
            // geometry is the useful part — it is what the unconstrain decided,
            // so a menu that lands off-screen can be told apart from one the
            // client positioned badly.
            tracing::info!(
                x = geometry.loc.x,
                y = geometry.loc.y,
                w = geometry.size.w,
                h = geometry.size.h,
                "popup tracked"
            );
        }
    }

    /// A client asking for its popup to hold a grab — a menu, rather than a
    /// tooltip.
    ///
    /// This was a no-op for as long as the seat's focus type was `WlSurface`:
    /// `PopupManager::grab_popup` needs `KeyboardFocus: From<PopupKind>`, and
    /// two foreign types cannot be joined by a local impl. [`FocusTarget`] is
    /// the newtype that makes it expressible.
    ///
    /// Taking the grab is what makes a menu behave like one. smithay's
    /// `PopupGrab` redirects the keyboard to the popup — so arrow keys and
    /// Escape reach the menu instead of the window behind it — dismisses the
    /// whole chain on a click outside, and keeps submenus nested under their
    /// parent rather than replacing it.
    ///
    /// A refused grab is not an error worth failing on: the protocol allows the
    /// compositor to decline, and a client whose menu does not grab still has a
    /// menu. It is logged, because a menu that silently does not grab is the
    /// bug this replaced.
    fn grab(&mut self, surface: PopupSurface, seat: WlSeat, serial: Serial) {
        let Some(seat) = Seat::<CompositorState<B>>::from_resource(&seat) else {
            return;
        };
        let popup = PopupKind::Xdg(surface);
        let Ok(root) = find_popup_root_surface(&popup) else {
            tracing::warn!("a popup asked to grab with no root surface");
            return;
        };
        let grab = self
            .popups
            .grab_popup(FocusTarget::from(root), popup, &seat, serial);
        match grab {
            Ok(mut grab) => {
                // Both devices, and both are needed: the pointer grab is what
                // makes a click outside dismiss the menu, and the keyboard grab
                // is what lets the menu be driven without the mouse at all.
                if let Some(keyboard) = seat.get_keyboard() {
                    if keyboard.is_grabbed()
                        && !(keyboard.has_grab(serial)
                            || keyboard.has_grab(grab.previous_serial().unwrap_or(serial)))
                    {
                        grab.ungrab(PopupUngrabStrategy::All);
                        return;
                    }
                    keyboard.set_focus(self, grab.current_grab(), serial);
                    keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
                }
                if let Some(pointer) = seat.get_pointer() {
                    if pointer.is_grabbed()
                        && !(pointer.has_grab(serial)
                            || pointer.has_grab(grab.previous_serial().unwrap_or(grab.serial())))
                    {
                        grab.ungrab(PopupUngrabStrategy::All);
                        return;
                    }
                    pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
                }
                // Asked of the seat rather than assumed from having called
                // `set_grab`: the two are not the same claim, and this is the
                // only part of the keyboard grab that can be checked without a
                // key actually arriving — which, nested, it does not, because
                // the host keeps keyboard focus elsewhere.
                tracing::debug!(
                    keyboard = seat.get_keyboard().map(|k| k.is_grabbed()).unwrap_or(false),
                    pointer = seat.get_pointer().map(|p| p.is_grabbed()).unwrap_or(false),
                    "popup took a grab"
                );
            }
            Err(err) => tracing::debug!(%err, "popup grab refused"),
        }
    }

    /// A client asking to move a popup it has already mapped — a submenu
    /// following the cursor, say.
    ///
    /// The token has to be echoed back with `send_repositioned`, or the client
    /// waits forever for an acknowledgement it will never get.
    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        let output = self.output_rect();
        surface.with_pending_state(|state| {
            state.geometry = unconstrain_popup(&positioner, output);
            state.positioner = positioner;
        });
        surface.send_repositioned(token);
        if let Err(err) = surface.send_configure() {
            tracing::warn!(%err, "could not configure a repositioned popup");
        }
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
        self.clients.remove(&id);
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
    use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_positioner::{
        Anchor, ConstraintAdjustment, Gravity,
    };
    use smithay::utils::Size;

    /// A positioner for a menu anchored at `anchor` inside a parent, wanting to
    /// open down-right, the way a right-click menu does.
    fn menu_positioner(anchor_at: (i32, i32), size: (i32, i32)) -> PositionerState {
        PositionerState {
            rect_size: Size::from(size),
            // Zero-sized: the anchor *point* the pointer was at, so the
            // expected position is the point itself rather than a corner of a
            // rectangle.
            anchor_rect: Rectangle::new(anchor_at.into(), (0, 0).into()),
            anchor_edges: Anchor::BottomRight,
            gravity: Gravity::BottomRight,
            constraint_adjustment: ConstraintAdjustment::FlipY | ConstraintAdjustment::SlideX,
            ..PositionerState::default()
        }
    }

    #[test]
    fn a_menu_in_open_space_opens_exactly_where_it_asked() {
        // The unconstrain must not move a popup that already fits, or every
        // menu would drift from the point it was opened at.
        let output = ruster_shell::Rect::new(0, 0, 1920, 1080);
        let geo = unconstrain_popup(&menu_positioner((100, 100), (200, 300)), output);
        assert_eq!((geo.loc.x, geo.loc.y), (100, 100));
        assert_eq!((geo.size.w, geo.size.h), (200, 300));
    }

    #[test]
    fn a_menu_near_the_bottom_is_brought_back_onto_the_screen() {
        // Opened 60px from the bottom, a 300px menu would put most of itself
        // below the screen — and the part you cannot see is the part you were
        // reaching for.
        let output = ruster_shell::Rect::new(0, 0, 1920, 1080);
        let geo = unconstrain_popup(&menu_positioner((100, 1020), (200, 300)), output);
        assert!(
            geo.loc.y + geo.size.h <= 1080,
            "menu runs off the bottom: y={} h={}",
            geo.loc.y,
            geo.size.h
        );
    }

    #[test]
    fn a_menu_near_the_right_edge_is_brought_back_too() {
        let output = ruster_shell::Rect::new(0, 0, 1920, 1080);
        let geo = unconstrain_popup(&menu_positioner((1900, 100), (200, 300)), output);
        assert!(
            geo.loc.x + geo.size.w <= 1920,
            "menu runs off the right: x={} w={}",
            geo.loc.x,
            geo.size.w
        );
    }

    #[test]
    fn a_zero_sized_output_does_not_panic_the_unconstrain() {
        // `output_rect()` returns 0x0 before the first output is configured,
        // and a popup arriving in that window must not take the session down.
        let geo = unconstrain_popup(
            &menu_positioner((0, 0), (200, 300)),
            ruster_shell::Rect::new(0, 0, 0, 0),
        );
        assert_eq!((geo.size.w, geo.size.h), (200, 300));
    }

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
