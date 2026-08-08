use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use smithay::backend::renderer::utils::on_commit_buffer_handler;
use smithay::input::keyboard::{KeyboardHandle, Keysym, XkbConfig};
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
    Client, Display, DisplayHandle, Resource,
};
use smithay::utils::{Serial, SERIAL_COUNTER as SCOUNTER};
use smithay::wayland::socket::ListeningSocketSource;
use smithay::wayland::{
    buffer::BufferHandler,
    compositor::{
        with_states, CompositorClientState, CompositorHandler,
        CompositorState as WlCompositorState, SurfaceAttributes,
    },
    cursor_shape::CursorShapeManagerState,
    output::OutputHandler,
    selection::data_device::{
        set_data_device_focus, ClientDndGrabHandler, DataDeviceHandler, DataDeviceState,
        ServerDndGrabHandler,
    },
    selection::primary_selection::{
        set_primary_focus, PrimarySelectionHandler, PrimarySelectionState,
    },
    selection::SelectionHandler,
    shell::xdg::{decoration::XdgDecorationState, ToplevelSurface, XdgShellState},
    shm::{ShmHandler, ShmState},
    tablet_manager::TabletSeatHandler,
    xdg_activation::{
        XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
    },
};
use tracing::{debug, info};

use crate::backend::{logical_output_size, Backend};
use crate::chrome::Chrome;
use crate::lua::Action;
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
    /// The middle-click clipboard. Separate state from `data_device_state`
    /// because it is a separate protocol, but the same seat and the same
    /// [`SelectionHandler`] serve both.
    pub primary_selection_state: PrimarySelectionState,
    /// Tokens handed out by `xdg_activation_v1`, which a client redeems to ask
    /// that one of its windows be focused.
    pub xdg_activation_state: XdgActivationState,
    /// `zxdg_decoration_manager_v1`. Held only so the global can be taken back
    /// down; every decoration decision is made in the handler.
    pub xdg_decoration_state: XdgDecorationState,
    /// `wp_cursor_shape_manager_v1`. Held for the same reason — smithay turns a
    /// client's named shape straight into a [`SeatHandler::cursor_image`] call,
    /// so there is no per-request state of ours to keep.
    pub cursor_shape_state: CursorShapeManagerState,
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
    /// The configured bindings, as chord sequences.
    ///
    /// The only copy. This was briefly a `Vec<(String, String)>` *and* a
    /// `Keymap` built from it, which meant two sources of truth for the same
    /// question — and two tests promptly set one and read the other.
    pub keymap: crate::keymap::Keymap,
    /// The half-typed sequence, if any.
    pub chord: crate::keymap::ChordState,
    /// Keycodes whose press was intercepted, so their release can be too.
    ///
    /// Resolution happens on press and the pending sequence has moved on by the
    /// time the release arrives, so re-resolving would answer differently and
    /// leak a stray release to the client — which is how a terminal ends up
    /// with a key it thinks is still held.
    pub intercepted: HashSet<u32>,
    /// Editor panes: tree leaves that are not Wayland clients. A leaf is a
    /// client if `toplevels` has it and a pane if this does — see `pane.rs` for
    /// why that is a side table rather than a variant on `Node::Leaf`.
    pub panes: crate::pane::Panes,
    /// Popups (client menus, tooltips), tracked so they can be drawn at the
    /// position their positioner asked for rather than not at all.
    pub popups: smithay::desktop::PopupManager,
    /// Whether the shortcut helper is pinned open.
    ///
    /// The overlay appears on its own while a chord is half-typed; pinning is
    /// what makes the keymap browsable when nothing is pending, which the panel
    /// used to do by accident by never going away.
    pub help_pinned: bool,
    /// The `:` prompt, when open or showing a result.
    pub minibuffer: Option<crate::minibuffer::MiniBuffer>,
    /// The live Lua control plane, when a config produced one. `None` means the
    /// config failed to run, so there is nothing to call into — keybinds still
    /// work, since those are resolved from `keybinds` above.
    pub wm: Option<crate::lua::WmControl>,
    /// Where the windows came from, and what the saved session is still waiting
    /// for. See [`crate::persist`].
    pub persist: crate::persist::Persistence,
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
        // Both clipboards follow the keyboard. smithay only offers a selection
        // to the client the *seat* considers focused, and it does not derive
        // that from the keyboard focus — so until this call existed neither
        // `wl_data_device.selection` nor its primary twin ever fired and paste
        // was silently dead. Done here rather than in `SeatHandler::focus_changed`
        // because that callback is not invoked when focus is cleared, which
        // would leave the last client still holding the offer after its window
        // went away.
        let client = focus.as_ref().and_then(|surface| surface.client());
        set_data_device_focus(&self.display_handle, &self.seat, client.clone());
        set_primary_focus(&self.display_handle, &self.seat, client);
        let keyboard = self.keyboard.clone();
        keyboard.set_focus(self, focus, serial);
    }

    /// The window id owning `surface`, if any toplevel does.
    ///
    /// `toplevels` is keyed the other way round because every other caller has
    /// the id and wants the surface; the protocol handlers arrive with only a
    /// surface, and each of them used to open-code this same linear scan.
    pub fn window_for_surface(&self, surface: &wl_surface::WlSurface) -> Option<WindowId> {
        self.toplevels
            .iter()
            .find(|(_, toplevel)| toplevel.wl_surface() == surface)
            .map(|(id, _)| *id)
    }

    /// Whether `window` may hold the keyboard: it has committed a buffer, and
    /// it is on the workspace currently on screen. Both halves are about the
    /// same failure — keystrokes disappearing into a window the user is not
    /// looking at, with no way to tell where they went.
    fn focusable(&self, window: WindowId) -> bool {
        // A pane is focusable as soon as it is on screen: it has no buffer to
        // commit and no client to wait for, so the `mapped` gate — which exists
        // to stop the keyboard reaching a window that has never drawn — has
        // nothing to say about it.
        let exists = self.mapped.contains(&window) || self.panes.contains_key(&window);
        exists && self.workspaces.is_visible(window)
    }
}

impl<B: Backend + 'static> CompositorState<B> {
    /// The output's whole area, in logical pixels, as the tree's root rectangle.
    pub fn output_rect(&self) -> Rect {
        let size = logical_output_size(self.backend_data.output()).unwrap_or_default();
        Rect::new(0, 0, size.w, size.h)
    }

    /// Feed one keypress to the open prompt.
    ///
    /// `modified` is the shifted keysym — the character actually typed — while
    /// the raw one identifies the editing keys. Reading text off the raw keysym
    /// would make every capital letter lowercase.
    pub fn minibuffer_key(&mut self, raw: Keysym, modified: Keysym) {
        use smithay::input::keyboard::keysyms as ks;
        match raw.raw() {
            ks::KEY_Escape => self.minibuffer = None,
            ks::KEY_Return | ks::KEY_KP_Enter => self.submit_minibuffer(),
            ks::KEY_BackSpace => {
                // Deleting back past the sigil closes the prompt, as in vim.
                if let Some(mb) = &mut self.minibuffer {
                    if mb.backspace() {
                        self.minibuffer = None;
                    }
                }
            }
            _ => {
                if let Some(c) = modified.key_char().filter(|c| !c.is_control()) {
                    if let Some(mb) = &mut self.minibuffer {
                        mb.push(c);
                    }
                }
            }
        }
    }

    /// Carry out a submitted mini-buffer line.
    ///
    /// Actions go to the same `dispatch` a keybind uses and Lua goes to the same
    /// VM the config ran in, so the prompt cannot do anything the rest of the
    /// control plane cannot — nor grow a vocabulary of its own that drifts.
    pub fn submit_minibuffer(&mut self) {
        use crate::minibuffer::{MiniBuffer, Submission};
        let submission = match &self.minibuffer {
            Some(mb) if mb.is_open() => mb.submit(),
            _ => return,
        };
        self.minibuffer = None;
        match submission {
            Submission::Action(action) => self.dispatch(action),
            Submission::Lua(code) => {
                let result = match &self.wm {
                    Some(wm) => wm.eval(&code),
                    // A config that failed to parse leaves no VM, and silently
                    // ignoring the line would look like the prompt is broken.
                    None => Err("no lua runtime (the config failed to load)".to_string()),
                };
                if let Err(err) = result {
                    self.minibuffer = Some(MiniBuffer::message(err));
                }
            }
            Submission::Nothing(msg) if !msg.is_empty() => {
                self.minibuffer = Some(MiniBuffer::message(msg));
            }
            Submission::Nothing(_) => {}
        }
    }

    /// Insert a new editor pane beside the focused leaf and focus it.
    ///
    /// The same two calls `new_toplevel` makes — an id from `ShellState`, then
    /// `Workspaces::insert` next to whatever has focus — because a pane is an
    /// ordinary leaf and taking a different route would be the first step
    /// towards the two diverging.
    pub fn open_pane(&mut self) {
        let area = self.output_rect();
        let id = self.shell.add_window("scratch".to_string(), area.w, area.h);
        self.workspaces
            .insert(id, self.shell.focus, ruster_shell::Layout::Horizontal);
        self.panes
            .insert(id, crate::pane::EditorPane::new("scratch"));
        crate::pane::debug_assert_disjoint(&self.panes, &self.toplevels);
        self.shell.set_focus(id);
        self.reconfigure_tiles();
        // The seat keyboard has no surface to hold now, which is the correct
        // answer for a pane and is what `update_keyboard_focus` already does
        // with an id no toplevel resolves.
        self.update_keyboard_focus(SCOUNTER.next_serial());
        tracing::info!(?id, "new pane");
    }

    /// Run everything Lua has queued since last time, then publish what the
    /// compositor now looks like.
    ///
    /// Called once per event-loop iteration by both backends. The actions are
    /// taken out of the queue first so the borrow on `wm` ends before
    /// [`dispatch`](Self::dispatch) needs `&mut self` — and so a script that
    /// queues while being drained cannot spin the loop forever, since anything
    /// it adds waits for the next pass.
    pub fn drain_wm_commands(&mut self) {
        let queued = match &self.wm {
            Some(wm) => wm.take_actions(),
            None => return,
        };
        for action in queued {
            self.dispatch(action);
        }
        let status = self.wm_status();
        if let Some(wm) = &self.wm {
            wm.publish(status);
        }
    }

    /// What `ruster.wm.status()` should report about the compositor right now.
    fn wm_status(&self) -> crate::lua::WmStatus {
        let tree = self.tree_status();
        crate::lua::WmStatus {
            workspace: self.workspaces.active(),
            windows: tree.windows,
            focused_title: self
                .shell
                .focused()
                .map(|w| w.title.clone())
                .unwrap_or_default(),
            floating: tree.floating,
            layout: tree.layout.map(|l| match l {
                ruster_shell::Layout::Horizontal => "horizontal".to_string(),
                ruster_shell::Layout::Vertical => "vertical".to_string(),
            }),
        }
    }

    /// Carry out a bound action.
    ///
    /// One place, so adding an action means adding one arm rather than another
    /// branch in the keyboard filter — and so the Lua control plane in the rest
    /// of Phase 2 has something to call that is not the key handler.
    ///
    /// Every layout operation needs a focused window to act on and does nothing
    /// without one, which is the honest behaviour: on an empty workspace there
    /// is no "window to the left".
    pub fn dispatch(&mut self, action: Action) {
        let area = self.output_rect();
        let focus = self.shell.focus;
        // Say what was asked for and what the tree looked like when it was
        // asked. Several actions are legitimately no-ops — `swap right` on the
        // rightmost window has nothing to swap with — and from a screenshot
        // that is indistinguishable from a broken keybind. On DRM there is not
        // even a screenshot unless someone presses the key for one.
        debug!(?action, ?focus, "dispatch");
        match action {
            Action::Quit => {
                info!("quit keybinding pressed, shutting down");
                self.running.store(false, Ordering::SeqCst);
            }
            Action::CycleWorkspace => self.cycle_workspace(),
            Action::Workspace(n) => {
                self.switch_workspace(n);
            }
            Action::Screenshot => self.screenshot_pending = true,
            Action::ToggleHelp => self.help_pinned = !self.help_pinned,
            Action::NewPane => self.open_pane(),
            Action::Bind(binding, action) => self.keymap.bind(&binding, &action),
            Action::Prompt(prompt) => {
                // Opening clears any message from last time; a stale result
                // sitting behind a fresh prompt reads as a reply to it.
                self.minibuffer = Some(crate::minibuffer::MiniBuffer::new(prompt));
            }
            Action::Spawn(command) => {
                // Through `persist` rather than straight to `spawn_command`, so
                // the window this produces can be saved as something the next
                // boot knows how to launch again.
                self.persist.spawn(&command, self.socket_name.as_deref());
            }
            Action::Focus(dir) => {
                if let Some(next) = focus
                    .and_then(|id| self.workspaces.tree().focus_target(id, dir, area))
                    .filter(|id| self.focusable(*id))
                {
                    self.shell.set_focus(next);
                    self.workspaces.raise_floating(next);
                    self.update_keyboard_focus(SCOUNTER.next_serial());
                }
            }
            Action::Swap(dir) => {
                if let (Some(from), Some(to)) = (
                    focus,
                    focus.and_then(|id| self.workspaces.tree().focus_target(id, dir, area)),
                ) {
                    self.workspaces.tree_mut().swap(from, to);
                    self.reconfigure_tiles();
                }
            }
            Action::Resize(dir) => {
                if let Some(id) = focus {
                    self.workspaces.tree_mut().resize(id, dir, RESIZE_STEP);
                    self.reconfigure_tiles();
                }
            }
            Action::Split(layout) => {
                if let Some(id) = focus {
                    self.workspaces.tree_mut().split(id, layout);
                }
            }
            Action::ToggleFloating => {
                if let Some(id) = focus {
                    self.workspaces.toggle_floating(id, area);
                    self.reconfigure_tiles();
                }
            }
            Action::MoveToWorkspace(n) => {
                if let Some(id) = focus {
                    self.move_to_workspace(id, n);
                }
            }
        }
        // The layout afterwards, which is the only evidence that separates
        // "the action ran and changed nothing" from "the key never arrived".
        debug!(geometry = ?self.geometry(), focus = ?self.shell.focus, "dispatched");
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
        let (cell_w, cell_h) = ruster_render_gles::atlas::cell_metrics(PANE_FONT_PX);
        for (id, rect) in self.geometry() {
            // A pane is sized in cells rather than pixels. Same layout pass and
            // the same rectangle, so a pane and a client can never disagree
            // about where the tile boundary is.
            if let Some(pane) = self.panes.get_mut(&id) {
                let (cols, rows) =
                    crate::pane::EditorPane::grid_for(rect.w, rect.h, cell_w, cell_h);
                pane.cols = cols;
                pane.rows = rows;
                continue;
            }
            let Some(toplevel) = self.toplevels.get(&id) else {
                continue;
            };
            toplevel.with_pending_state(|state| {
                state.size = Some((rect.w, rect.h).into());
                // Tiled on every edge: the honest way to tell a client it does
                // not own its borders, so it drops rounded corners and shadows.
                // The titlebar goes away separately, via xdg-decoration (see
                // `shell::answer_decoration`) — tiled states alone do not
                // stop a toolkit drawing one.
                state.states.set(xdg_toplevel::State::TiledLeft);
                state.states.set(xdg_toplevel::State::TiledRight);
                state.states.set(xdg_toplevel::State::TiledTop);
                state.states.set(xdg_toplevel::State::TiledBottom);
            });
            toplevel.send_pending_configure();
        }
    }

    /// Put `window` in front of the user and give it the keyboard, switching
    /// workspaces if that is where it lives. Does nothing for a window that
    /// cannot be shown — see [`activation_workspace`].
    ///
    /// Focusing goes through the same three calls a `focus` keybind makes, so
    /// an activation and a keypress leave the compositor in the same state.
    pub fn activate_window(&mut self, window: WindowId) {
        let Some(workspace) = activation_workspace(&self.workspaces, &self.mapped, window) else {
            return;
        };
        debug!(?window, workspace, "activating window");
        self.switch_workspace(workspace);
        self.shell.set_focus(window);
        self.workspaces.raise_floating(window);
        self.update_keyboard_focus(SCOUNTER.next_serial());
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

/// How far one `resize` keypress moves a boundary. A whole tenth is coarse
/// enough to be worth pressing and fine enough to land where you meant after a
/// couple of taps.
const RESIZE_STEP: f32 = 0.05;

/// The window that should take keyboard focus after `unmapped` hid itself:
/// the most recently mapped window still visible, or `None` when nothing else
/// is mapped. Mirrors `ShellState::remove_window`'s fall back to the last
/// remaining window, and is pure so the compositor's unmap path stays
/// unit-testable without a live display.
fn next_focus_after_unmap(mapped: &HashSet<WindowId>, unmapped: WindowId) -> Option<WindowId> {
    mapped.iter().filter(|id| **id != unmapped).max().copied()
}

/// Font size a pane's character grid is measured at.
///
/// One constant so the grid `reconfigure_tiles` computes and the text Stage 2
/// draws cannot be measured at different sizes — which would put the cursor in
/// the wrong cell without anything looking obviously wrong.
pub const PANE_FONT_PX: u32 = 14;

/// Which workspace has to be on screen before `window` can take focus, or
/// `None` when an activation request naming it should be dropped.
///
/// A window with no buffer yet has nothing to show and cannot hold the keyboard
/// (see [`CompositorState::focusable`]), and one the trees do not hold is
/// already gone — honouring either would switch the user to a workspace to look
/// at nothing. Pure, so the activation policy is testable without a display.
fn activation_workspace(
    workspaces: &Workspaces,
    mapped: &HashSet<WindowId>,
    window: WindowId,
) -> Option<u32> {
    if !mapped.contains(&window) {
        return None;
    }
    workspaces.workspace_of(window)
}

/// Globals created for a display; bundled so `create_state` can build them
/// before the state struct itself exists (anvil does this inline in `init`).
struct InitGlobals<B: Backend + 'static> {
    compositor_state: WlCompositorState,
    shm_state: ShmState,
    xdg_shell_state: XdgShellState,
    data_device_state: DataDeviceState,
    primary_selection_state: PrimarySelectionState,
    xdg_activation_state: XdgActivationState,
    xdg_decoration_state: XdgDecorationState,
    cursor_shape_state: CursorShapeManagerState,
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
        primary_selection_state: globals.primary_selection_state,
        xdg_activation_state: globals.xdg_activation_state,
        xdg_decoration_state: globals.xdg_decoration_state,
        cursor_shape_state: globals.cursor_shape_state,
        seat_state: globals.seat_state,
        seat: globals.seat,
        keyboard: globals.keyboard,
        pointer: globals.pointer,
        cursor_status: CursorImageStatus::default_named(),
        workspaces: Workspaces::new(),
        toplevels: HashMap::new(),
        pending_focus: None,
        mapped: HashSet::new(),
        // The user's theme, not the built-in one: the compositor drew with
        // `Theme::default()` while `ruster-lua` sat unimported in its
        // Cargo.toml, so every colour the editor lets you configure was ignored
        // here.
        chrome: Some(Chrome::new(user_theme())),
        screenshot_pending: false,
        screenshot_count: 0,
        keymap: crate::keymap::Keymap::default(),
        chord: crate::keymap::ChordState::default(),
        intercepted: HashSet::new(),
        panes: crate::pane::Panes::new(),
        popups: smithay::desktop::PopupManager::default(),
        help_pinned: false,
        minibuffer: None,
        wm: None,
        persist: crate::persist::Persistence::default(),
    }
}

/// The theme from the user's `config.lua`, or the built-in one if there is none.
///
/// The compositor drew with `Theme::default()` while `ruster-lua` sat unimported
/// in its Cargo.toml, so none of the colours the editor lets you configure — and
/// none of the built-in themes — reached the compositor's chrome at all.
fn user_theme() -> Theme {
    // Never touch the real ~/.config/ruster from the test suite; the state
    // constructor below is built in unit tests.
    if cfg!(test) {
        return Theme::default();
    }
    let mut lua = match ruster_lua::runtime::LuaRuntime::new() {
        Ok(lua) => lua,
        Err(err) => {
            tracing::warn!(%err, "could not start the lua runtime; using the default theme");
            return Theme::default();
        }
    };
    if let Some(dir) = dirs::config_dir().map(|p| p.join("ruster")) {
        // `config.lua` then `init.lua`, the same order and the same meaning as
        // the editor: the declarative file first, user scripting on top. Unlike
        // the editor this never *writes* a default config — a compositor that
        // creates files in the config dir on a bare VT boot would be a surprise.
        for name in ["config.lua", "init.lua"] {
            let path = dir.join(name);
            if path.exists() {
                if let Err(err) = lua.load_init(&path) {
                    tracing::warn!(path = %path.display(), %err, "config failed to load");
                }
            }
        }
    }
    (&lua.config().colors).into()
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
    // The middle-click clipboard. Every terminal and editor on X11 had one and
    // toolkits still expect it; foot names its absence at startup.
    let primary_selection_state = PrimarySelectionState::new::<CompositorState<B>>(dh);
    let xdg_activation_state = XdgActivationState::new::<CompositorState<B>>(dh);
    // Announce that ruster decorates windows. It already draws a border per
    // tile, so a client drawing its own titlebar inside that border is chrome
    // twice over; without this global every toolkit assumes CSD unconditionally.
    let xdg_decoration_state = XdgDecorationState::new::<CompositorState<B>>(dh);
    // Named cursor shapes. The compositor already draws the pointer itself, so
    // this only gives clients a way to name the shape they want instead of each
    // one loading an XCursor theme and attaching its own surface.
    let cursor_shape_state = CursorShapeManagerState::new::<CompositorState<B>>(dh);
    let mut seat_state = SeatState::new();
    let mut seat = seat_state.new_wl_seat(dh, seat_name);
    let pointer = seat.add_pointer();
    // The system keymap to start with — `XkbConfig::default()` compiles the
    // libxkbcommon default, which honours `XKB_DEFAULT_LAYOUT` and friends, so
    // an unconfigured compositor already matches the rest of the session. A
    // config that names its own is applied afterwards by
    // `lua::apply_keyboard_config`, which can fail safely; this one cannot fail
    // at all without the machine being broken.
    let defaults = crate::lua::KeyboardConfig::default();
    let keyboard = seat
        .add_keyboard(
            XkbConfig::default(),
            defaults.repeat_delay,
            defaults.repeat_rate,
        )
        .expect("failed to initialize the keyboard");

    InitGlobals {
        compositor_state,
        shm_state,
        xdg_shell_state,
        data_device_state,
        primary_selection_state,
        xdg_activation_state,
        xdg_decoration_state,
        cursor_shape_state,
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
        // The popup manager needs every commit: it is what advances a popup
        // from "created" to "mapped" and sends the initial configure. Without
        // it a tracked popup never gets configured, so the client never draws
        // into it and the menu stays empty.
        self.popups.commit(surface);
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
        let Some(id) = self.window_for_surface(surface) else {
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
                    // Through the tree, not the `mapped` set: a client unmapping
                    // beside a pane would otherwise pick `None` and focus would
                    // vanish, because a pane is not in `mapped` and never will be.
                    let after_tree = self
                        .workspaces
                        .focus_for_active(Some(id))
                        .filter(|n| *n != id);
                    if let Some(next) = after_tree.or_else(|| next_focus_after_unmap(&visible, id))
                    {
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

impl<B: Backend + 'static> PrimarySelectionHandler for CompositorState<B> {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.primary_selection_state
    }
}

// Named cursor shapes arrive as `SeatHandler::cursor_image` calls, so nothing
// extra is needed for the pointer. The tablet half of the protocol shares the
// manager global and its handler has a do-nothing default: ruster does not
// speak `zwp_tablet_manager_v2` at all, so no tool can ever ask for a shape.
impl<B: Backend + 'static> TabletSeatHandler for CompositorState<B> {}

impl<B: Backend + 'static> XdgActivationHandler for CompositorState<B> {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.xdg_activation_state
    }

    /// A client redeemed a token asking that one of its windows be focused —
    /// what foot's `bell.urgent` does, and what a browser does when a second
    /// invocation hands the URL to the copy already running.
    ///
    /// The window is brought into view: if it sits on a workspace that is not
    /// on screen, that workspace is switched to. The alternative was to honour
    /// activation only for windows already visible, which is safer against a
    /// client yanking the screen around — but it makes activation a no-op in
    /// exactly the case it exists for, and a request that does nothing is
    /// indistinguishable from a compositor that never implemented it.
    fn request_activation(
        &mut self,
        _token: XdgActivationToken,
        _token_data: XdgActivationTokenData,
        surface: wl_surface::WlSurface,
    ) {
        if let Some(id) = self.window_for_surface(&surface) {
            self.activate_window(id);
        }
    }
}

// One delegate per protocol we speak. Each wires the `Dispatch`/`GlobalDispatch`
// impls for that protocol's objects through to the smithay state we hold above,
// so the handler traits implemented in this file (and `XdgShellHandler` /
// `XdgDecorationHandler` in `shell.rs`) are all the glue we write by hand.
smithay::delegate_compositor!(@<B: Backend + 'static> CompositorState<B>);
smithay::delegate_shm!(@<B: Backend + 'static> CompositorState<B>);
smithay::delegate_output!(@<B: Backend + 'static> CompositorState<B>);
smithay::delegate_seat!(@<B: Backend + 'static> CompositorState<B>);
smithay::delegate_xdg_shell!(@<B: Backend + 'static> CompositorState<B>);
smithay::delegate_xdg_decoration!(@<B: Backend + 'static> CompositorState<B>);
smithay::delegate_data_device!(@<B: Backend + 'static> CompositorState<B>);
smithay::delegate_primary_selection!(@<B: Backend + 'static> CompositorState<B>);
smithay::delegate_xdg_activation!(@<B: Backend + 'static> CompositorState<B>);
smithay::delegate_cursor_shape!(@<B: Backend + 'static> CompositorState<B>);

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

    #[test]
    fn activation_names_the_workspace_holding_the_window() {
        // The point of honouring xdg-activation across workspaces: a client on
        // a workspace nobody is looking at asks to be seen, and the answer is
        // the workspace to switch to rather than "no".
        let mut workspaces = Workspaces::new();
        let here = WindowId(1);
        let away = WindowId(2);
        workspaces.insert(here, None, ruster_shell::Layout::Horizontal);
        workspaces.switch_to(4);
        workspaces.insert(away, None, ruster_shell::Layout::Horizontal);
        workspaces.switch_to(1);

        let mapped = HashSet::from([here, away]);
        assert_eq!(
            activation_workspace(&workspaces, &mapped, here),
            Some(workspaces.active())
        );
        assert_eq!(activation_workspace(&workspaces, &mapped, away), Some(4));
    }

    #[test]
    fn activation_is_dropped_for_windows_that_cannot_be_shown() {
        // A window that has never committed a buffer has nothing to show and
        // cannot hold the keyboard, so switching the user to its workspace
        // would move the screen to look at nothing.
        let mut workspaces = Workspaces::new();
        let unmapped = WindowId(7);
        workspaces.insert(unmapped, None, ruster_shell::Layout::Horizontal);
        assert_eq!(
            activation_workspace(&workspaces, &HashSet::new(), unmapped),
            None
        );
        // And a window no tree holds is already gone, mapped set or not.
        assert_eq!(
            activation_workspace(&workspaces, &HashSet::from([WindowId(9)]), WindowId(9)),
            None
        );
    }
}
