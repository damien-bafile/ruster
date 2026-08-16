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
    output::{OutputHandler, OutputManagerState},
    selection::data_device::{
        request_data_device_client_selection, set_data_device_focus, set_data_device_selection,
        ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
    },
    selection::primary_selection::{
        set_primary_focus, PrimarySelectionHandler, PrimarySelectionState,
    },
    selection::{SelectionHandler, SelectionSource, SelectionTarget},
    shell::wlr_layer::{Layer as WlrLayer, LayerSurface, WlrLayerShellHandler, WlrLayerShellState},
    shell::xdg::{decoration::XdgDecorationState, XdgShellState},
    shm::{ShmHandler, ShmState},
    tablet_manager::TabletSeatHandler,
    xdg_activation::{
        XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
    },
};
use tracing::{debug, info, warn};

use crate::backend::{logical_output_size, Backend};
use crate::chrome::Chrome;
use crate::lua::Action;
use crate::shell::CommitBuffer;
use ruster_render::Theme;
use ruster_shell::{Rect, ShellState, WindowId, Workspaces};

/// The area windows are tiled into: the output, less whatever the bars reserved.
///
/// A free function because this is geometry, and geometry is where the bug will
/// be — the same reason `launcher_layout` and `gutter_cols` are free functions.
/// `non_exclusive_zone` is smithay's answer; the part worth testing is what
/// happens when that answer is unusable.
pub fn tiling_area(
    output: smithay::utils::Size<i32, smithay::utils::Logical>,
    zone: smithay::utils::Rectangle<i32, smithay::utils::Logical>,
) -> Rect {
    // A bar claiming the whole screen leaves nothing to tile into, and every
    // window would then be laid out at zero size — which on screen is a
    // compositor that appears to have lost its windows. Ignoring a zone that
    // cannot be used is the better failure: the bar still draws, and the windows
    // are merely underneath it.
    if zone.size.w <= 0 || zone.size.h <= 0 {
        return Rect::new(0, 0, output.w, output.h);
    }
    Rect::new(zone.loc.x, zone.loc.y, zone.size.w, zone.size.h)
}

/// A toplevel's window geometry: the part of its surface that is the window
/// proper, with the client-side drop shadow outside it.
///
/// `None` from a client that has not set one, which the protocol says means the
/// whole surface is the window.
pub fn window_geometry(
    surface: &wl_surface::WlSurface,
) -> Option<smithay::utils::Rectangle<i32, smithay::utils::Logical>> {
    with_states(surface, |states| {
        states
            .cached_state
            .get::<smithay::wayland::shell::xdg::SurfaceCachedState>()
            .current()
            .geometry
    })
}

/// Where a toplevel's *surface* goes, given the tile its *window* has to fill.
///
/// These are not the same point, and assuming they were is a real bug this
/// found: a GTK4 client hands over a buffer with an invisible shadow margin
/// around it and says so via `set_window_geometry` — nautilus reports `(20,20)`.
/// Drawn at the tile origin, the shadow is on screen in the top-left, every
/// pixel of the window is 20px down and right of where it belongs, and the last
/// 20px — the window controls, on a right-hand edge — is pushed off the output
/// and clipped.
///
/// A free function because the renderer and hit-testing both need the answer,
/// and the failure when they disagree is silent: the window draws correctly and
/// every click lands 20px away from what it looks like it hit. The popup path
/// has always done this (`render.rs`, subtracting `popup.geometry().loc`);
/// toplevels are the case that was missed.
pub fn surface_origin(
    tile_loc: smithay::utils::Point<i32, smithay::utils::Logical>,
    geometry: Option<smithay::utils::Rectangle<i32, smithay::utils::Logical>>,
) -> smithay::utils::Point<i32, smithay::utils::Logical> {
    let inset = geometry.map(|g| g.loc).unwrap_or_default();
    tile_loc - inset
}

/// What a reply from a language server is for.
///
/// A response carries only the id of the request it answers, so the intent has
/// to be recorded when the question is asked; this is the type parameter
/// `LspState` is generic over. It was `()` for as long as the compositor only
/// consumed diagnostics — which arrive unbidden and answer nothing — and that
/// `()` was the shape of the gap: there was no way to say what a reply meant,
/// because no request had ever been sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LspPending {
    /// Move the focused pane to wherever the symbol is defined.
    Definition,
    /// Show what the server knows about the symbol, in a panel by the caret.
    ///
    /// Carries where to put it, because by the time the reply lands the caret
    /// may have moved and a panel that followed it would point at a symbol the
    /// text is no longer about.
    Hover {
        pane: WindowId,
        row: usize,
        col: usize,
    },
}

/// A hover panel waiting to be drawn: what the server said, and where to say it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoverPanel {
    /// The pane whose caret this describes. The panel goes away with it: a
    /// panel outliving its pane would be an explanation of nothing, floating
    /// over whatever took the tile.
    pub pane: WindowId,
    pub row: usize,
    pub col: usize,
    pub lines: Vec<String>,
}

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
    pub clients: HashMap<WindowId, crate::client::Client>,
    /// Window that should take focus once the seat is set up (Task 10).
    pub pending_focus: Option<WindowId>,
    /// Toplevels that have committed a buffer and are thus rendered (Task 7).
    pub mapped: HashSet<WindowId>,
    /// The compositor's UI chrome (statusline, editor frame, which-key), drawn
    /// above the client surfaces (Task 8).
    pub chrome: Option<Chrome>,
    /// When a screenshot was asked for, cleared by the render loop once it has
    /// captured. The capture needs the renderer and the finished framebuffer,
    /// neither of which the key handler has, so the request has to wait for the
    /// frame rather than be served where it is made.
    ///
    /// The *time* rather than a bare flag because waiting for a frame is not the
    /// same as getting one. Rendering is gated on the host inviting a frame, and
    /// a nested window the host is not presenting — occluded, on another
    /// workspace, or simply not mapped where anyone can see it — is never
    /// invited. The request then sits here forever: the action dispatches, logs
    /// that it dispatched, writes nothing, and says nothing about it. That is
    /// how a verification run came back with four actions on time, no PNG, and
    /// no error to explain the gap. [`screenshot_overdue`] turns that into a
    /// warning.
    pub screenshot_pending: Option<std::time::Instant>,
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
    /// The intercepted key currently held down, if it is one that repeats.
    ///
    /// A key the compositor keeps for itself never reaches a toolkit, and the
    /// toolkit is what repeats a held key — so the compositor has to. See
    /// [`crate::repeat`].
    pub repeat: Option<crate::repeat::KeyRepeat>,
    /// Armings so far, so a timer can tell whether it is still the live one.
    pub repeat_generation: u64,
    /// The keyboard configuration the seat was given.
    ///
    /// Held because `repeat_delay`/`repeat_rate` have to drive the compositor's
    /// own repeat timer as well as the `wl_keyboard.repeat_info` the seat
    /// announces, and smithay keeps its copy private. Written only where the
    /// seat is told — [`crate::lua::apply_keyboard_config`] and the seat
    /// construction below — so the two cannot drift.
    pub keyboard_config: crate::lua::KeyboardConfig,
    /// The terminal the config named, if it named one. Kept unresolved: the
    /// rest of the answer is `$TERMINAL` and `PATH`, which are read when the
    /// key is pressed rather than frozen at startup.
    pub terminal: Option<String>,
    /// Language servers, one per language, and the diagnostics they publish.
    ///
    /// `LspState` is keyed by `BufferId` — the same id a pane names its document
    /// by — which is why it moved out of `ruster-tui` rather than being rewritten
    /// here. The type parameter is what a reply should be *used for*.
    pub lsp: ruster_lsp::state::LspState<LspPending>,
    /// The hover panel on screen, if a reply has arrived and nothing has
    /// dismissed it yet.
    pub hover: Option<HoverPanel>,
    /// Screen captures a client has asked for and a frame has not yet served.
    pub screencopy: crate::screencopy::ScreencopyState,
    /// Bars, notification daemons and wallpapers: surfaces that sit outside the
    /// tiling rather than in it.
    pub layer_shell_state: WlrLayerShellState,
    /// The `xwayland_shell_v1` global, which is how XWayland tells the
    /// compositor that a given `wl_surface` belongs to a given X11 window.
    pub xwayland_shell_state: smithay::wayland::xwayland_shell::XWaylandShellState,
    /// The X11 window manager, once XWayland has started and said it is ready.
    /// `None` before that, and for the whole session on a machine with no
    /// `Xwayland` binary — which is a warning, not a failure.
    pub xwm: Option<smithay::xwayland::X11Wm>,
    /// X11 windows the window manager was told to keep its hands off: menus,
    /// tooltips, drag icons. Drawn where the client put them, never tiled.
    pub x11_unmanaged: Vec<smithay::xwayland::X11Surface>,
    /// The launcher overlay, when open.
    pub launcher: Option<crate::launcher::Launcher>,
    /// What answers a launcher query. Built once at startup.
    pub providers: crate::launcher::ProviderSet,
    /// Syntax parses, one per document, refreshed when a buffer changes.
    pub highlights: std::cell::RefCell<crate::highlight::Highlights>,
    /// The seat's selection text, shared between editor panes and clients.
    pub clipboard: crate::clipboard::Clipboard,
    /// Editor panes: tree leaves that are not Wayland clients. A leaf is a
    /// client if `toplevels` has it and a pane if this does — see `pane.rs` for
    /// why that is a side table rather than a variant on `Node::Leaf`.
    pub panes: crate::pane::Panes,
    /// Every open document, which panes name by id.
    ///
    /// `ruster_core::workspace::BufferStore` and nothing else from that module:
    /// `Workspace` brings its own `WindowTree`, and a second tiling tree inside
    /// `ruster_shell::Tree` is exactly what the spec's "buffers and clients are
    /// peers in one tree" rules out. The store is the half with no opinion about
    /// layout, which is the half a compositor needs.
    pub buffers: ruster_core::workspace::BufferStore,
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
        let focused_id = self
            .pending_focus
            .take()
            .filter(|id| self.focusable(*id))
            .or_else(|| self.shell.focus.filter(|id| self.focusable(*id)));
        // X11 has no `wl_keyboard.enter`, so an X window is only "active"
        // because the window manager said so. Told nothing, it draws itself
        // greyed-out while receiving every keystroke — which reads as the
        // compositor delivering input to the wrong window. Every window is told,
        // including the ones losing focus, or the previous holder stays lit.
        for (id, client) in &self.clients {
            client.set_activated(Some(*id) == focused_id);
        }
        let focus = focused_id
            .and_then(|id| self.clients.get(&id))
            .and_then(|client| client.wl_surface());
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
        keyboard.set_focus(self, focus.map(crate::focus::FocusTarget::from), serial);
    }

    /// The window id owning `surface`, if any toplevel does.
    ///
    /// `toplevels` is keyed the other way round because every other caller has
    /// the id and wants the surface; the protocol handlers arrive with only a
    /// surface, and each of them used to open-code this same linear scan.
    pub fn window_for_surface(&self, surface: &wl_surface::WlSurface) -> Option<WindowId> {
        self.clients
            .iter()
            .find(|(_, client)| client.wl_surface().as_ref() == Some(surface))
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
        let output = self.backend_data.output();
        let size = logical_output_size(output).unwrap_or_default();
        // What is left after the bars. A layer surface with an exclusive zone —
        // which is what a bar is — takes its strip out of the area windows are
        // tiled into, so a window laid out against the whole output would sit
        // underneath it. `arrange` is what turns the anchors and margins the
        // client asked for into that number, and it has to run before the answer
        // is read.
        let mut map = smithay::desktop::layer_map_for_output(output);
        map.arrange();
        let zone = map.non_exclusive_zone();
        drop(map);
        tiling_area(size, zone)
    }

    /// The output's size in real pixels, which is what a framebuffer read
    /// returns and therefore what a screencopy client must size its buffer to.
    ///
    /// Distinct from [`output_rect`](Self::output_rect), which is logical: on a
    /// scaled output the two differ, and handing a client the logical size would
    /// have it allocate a buffer the capture overruns.
    pub fn output_size_physical(
        &self,
    ) -> Option<smithay::utils::Size<i32, smithay::utils::Physical>> {
        self.backend_data.output().current_mode().map(|m| m.size)
    }

    /// One key into the launcher.
    ///
    /// Text comes off the *modified* keysym and editing keys off the raw one,
    /// the same split the mini-buffer makes: reading text off the raw sym makes
    /// every capital lowercase.
    pub fn launcher_key(
        &mut self,
        raw: Keysym,
        modified: Keysym,
        mods: smithay::input::keyboard::ModifiersState,
    ) {
        use smithay::input::keyboard::keysyms as ks;
        let mut requery = false;
        match raw.raw() {
            ks::KEY_Escape => {
                self.launcher = None;
                return;
            }
            ks::KEY_Return | ks::KEY_KP_Enter => return self.launcher_accept(),
            ks::KEY_BackSpace => {
                let empty = self
                    .launcher
                    .as_mut()
                    .map(|l| l.backspace())
                    .unwrap_or(true);
                if empty {
                    self.launcher = None;
                    return;
                }
                requery = true;
            }
            ks::KEY_Down => self.launcher_move(1),
            ks::KEY_Up => self.launcher_move(-1),
            ks::KEY_Tab => self.launcher_move(1),
            ks::KEY_ISO_Left_Tab => self.launcher_move(-1),
            // `C-n`/`C-p`, which is why the repeat target carries modifiers: a
            // control chord produces a control character that `key_char()`
            // filters out, so the text path below never sees these.
            ks::KEY_n if mods.ctrl => self.launcher_move(1),
            ks::KEY_p if mods.ctrl => self.launcher_move(-1),
            _ => {
                if mods.ctrl || mods.logo || mods.alt {
                    return;
                }
                let Some(c) = modified.key_char().filter(|c| !c.is_control()) else {
                    return;
                };
                if let Some(l) = self.launcher.as_mut() {
                    l.push(c);
                    requery = true;
                }
            }
        }
        if requery {
            self.launcher_refresh();
        }
    }

    fn launcher_move(&mut self, delta: i32) {
        if let Some(l) = self.launcher.as_mut() {
            l.move_selection(delta);
        }
    }

    /// Ask every provider about the current query and keep what they say.
    pub fn launcher_refresh(&mut self) {
        /// Rows one provider may contribute. Enough that a real answer is not
        /// cut off, few enough that one chatty provider cannot bury the others.
        const PER_PROVIDER: usize = 8;
        let Some(query) = self.launcher.as_ref().map(|l| l.query.clone()) else {
            return;
        };
        // `providers` and `launcher` are distinct fields, so these borrows are
        // disjoint. Calling `self.report(..)` in here would not be — which is
        // why nothing in this scope reports.
        let ctx = crate::launcher::ProviderCtx {
            wm: self.wm.as_ref(),
        };
        let groups = self.providers.query(&query, &ctx, PER_PROVIDER);
        if let Some(l) = self.launcher.as_mut() {
            l.set_groups(groups);
        }
    }

    /// Run the selected row and close.
    fn launcher_accept(&mut self) {
        let Some(activation) = self.launcher.as_mut().and_then(|l| l.accept()) else {
            self.launcher = None;
            return;
        };
        self.launcher = None;
        match activation {
            crate::launcher::Activation::Action(action) => self.dispatch(action),
            crate::launcher::Activation::Copy(text) => self.copy_text(text),
            crate::launcher::Activation::Report(message) => self.report(message),
        }
    }

    /// Put text on the seat selection, as yanking from a pane does.
    pub fn copy_text(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.clipboard.set_from_pane(text.clone());
        set_data_device_selection(
            &self.display_handle,
            &self.seat.clone(),
            crate::clipboard::mime_types(),
            (),
        );
        self.report(format!("copied: {text}"));
    }

    /// Whether a modal overlay is taking every key right now.
    ///
    /// One question with one answer, asked by the key interception branch and by
    /// key repeat. The launcher and the `:` prompt cannot both be open —
    /// opening the launcher closes the prompt — so this is an either, not a
    /// precedence.
    pub fn overlay_is_open(&self) -> bool {
        self.launcher.is_some() || self.minibuffer.as_ref().is_some_and(|mb| mb.is_open())
    }

    /// Feed one key to whichever overlay is open.
    pub fn overlay_key(
        &mut self,
        raw: Keysym,
        modified: Keysym,
        mods: smithay::input::keyboard::ModifiersState,
    ) {
        if self.launcher.is_some() {
            self.launcher_key(raw, modified, mods);
            return;
        }
        self.minibuffer_key(raw, modified);
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
        // A scratch document each, not one shared one: two `new pane`s are two
        // empty buffers in every editor, and sharing would make typing in one
        // appear in the other.
        let doc = self.buffers.create_scratch("scratch");
        self.open_pane_with(doc);
    }

    /// Insert a pane showing `doc` beside the focused leaf and focus it.
    ///
    /// The one place a pane enters the tree, so an empty scratch pane and a
    /// pane opened on a file cannot end up in different shapes.
    pub fn open_pane_with(&mut self, doc: ruster_core::document::BufferId) {
        let area = self.output_rect();
        let title = self.document_name(doc);
        let id = self.shell.add_window(title, area.w, area.h);
        self.workspaces
            .insert(id, self.shell.focus, ruster_shell::Layout::Horizontal);
        self.panes.insert(id, crate::pane::EditorPane::new(doc));
        crate::pane::debug_assert_disjoint(&self.panes, &self.clients);
        self.shell.set_focus(id);
        self.reconfigure_tiles();
        // The seat keyboard has no surface to hold now, which is the correct
        // answer for a pane and is what `update_keyboard_focus` already does
        // with an id no toplevel resolves.
        self.update_keyboard_focus(SCOUNTER.next_serial());
        tracing::info!(?id, "new pane");
    }

    /// The focused leaf, when it is an editor pane.
    pub fn focused_pane(&self) -> Option<WindowId> {
        self.shell.focus.filter(|id| self.panes.contains_key(id))
    }

    /// Whether the focused leaf is an editor pane.
    pub fn pane_has_focus(&self) -> bool {
        self.focused_pane().is_some()
    }

    /// The pane at `id` and the document it is showing.
    ///
    /// The pair is what every pane operation needs and what neither half holds
    /// alone: the text is in the store, the cursor and scroll are on the pane.
    /// One accessor so no caller open-codes the second lookup and quietly
    /// disagrees about what a missing document means.
    pub fn pane_document(
        &self,
        id: WindowId,
    ) -> Option<(&crate::pane::EditorPane, &ruster_core::document::Document)> {
        let pane = self.panes.get(&id)?;
        let doc = self.buffers.get(pane.doc)?;
        Some((pane, doc))
    }

    pub fn pane_document_mut(
        &mut self,
        id: WindowId,
    ) -> Option<(
        &mut crate::pane::EditorPane,
        &mut ruster_core::document::Document,
    )> {
        let pane = self.panes.get_mut(&id)?;
        let doc = self.buffers.get_mut(pane.doc)?;
        Some((pane, doc))
    }

    /// What a document calls itself, for a window title or a message.
    pub(crate) fn document_name(&self, doc: ruster_core::document::BufferId) -> String {
        self.buffers
            .get(doc)
            .map(|d| d.name.clone())
            .unwrap_or_default()
    }

    /// Say something in the mini-buffer.
    ///
    /// Every pane command reports this way, because the mini-buffer is the only
    /// place the compositor can say anything at all: a pane has no status line
    /// of its own and a DRM boot has no terminal to print to.
    pub fn report(&mut self, message: impl Into<String>) {
        self.minibuffer = Some(crate::minibuffer::MiniBuffer::message(message));
    }

    /// Feed a key to the focused pane, keeping its clipboard and the seat's in
    /// step.
    ///
    /// Around the key rather than inside `VimState`: the editor's clipboard is
    /// an `arboard` handle plus an in-process buffer, and inside a compositor
    /// `arboard` has no display to reach — this process is the display. Seeding
    /// before and publishing after leaves the editor's own logic untouched and
    /// makes the compositor's selection the one that counts.
    pub fn pane_key(&mut self, key: ruster_core::key::KeyEvent) {
        let before = self.seed_pane_clipboard();
        self.pane_key_inner(key);
        self.publish_pane_clipboard(before);
    }

    /// Put the seat's selection where the pane's paste will look, returning what
    /// the pane's clipboard held so a yank can be told apart afterwards.
    fn seed_pane_clipboard(&mut self) -> Option<String> {
        let text = self.clipboard.text().to_string();
        let id = self.shell.focus?;
        let pane = self.panes.get(&id)?;
        if !text.is_empty() {
            pane.vim.clipboard_set(&text);
        }
        pane.vim.clipboard_get()
    }

    /// If the key yanked something, make it the seat's selection.
    fn publish_pane_clipboard(&mut self, before: Option<String>) {
        let Some(id) = self.shell.focus else {
            return;
        };
        let Some(after) = self.panes.get(&id).and_then(|p| p.vim.clipboard_get()) else {
            return;
        };
        if Some(&after) == before.as_ref() || after.is_empty() {
            return;
        }
        self.clipboard.set_from_pane(after);
        set_data_device_selection(
            &self.display_handle,
            &self.seat.clone(),
            crate::clipboard::mime_types(),
            (),
        );
    }

    fn pane_key_inner(&mut self, key: ruster_core::key::KeyEvent) {
        let Some(id) = self.shell.focus else {
            return;
        };
        // Any key dismisses a hover panel. It describes one caret on one
        // character, so the moment either can have moved it is at best stale and
        // at worst pointing at something it is not about — and a panel that has
        // to be dismissed deliberately is one more thing to know about an
        // explanation that was meant to be free.
        self.hover = None;
        // The indent is the document's, not a constant here: it is buffer-local
        // in the editor, seeded from config and EditorConfig, and one file
        // indented two ways depending on which program had it open is exactly
        // the drift a shared `Document` exists to prevent.
        if let Some((pane, doc)) = self.pane_document_mut(id) {
            pane.handle_key(key, doc);
        }
    }

    /// The document for `path`, reading it from disk the first time.
    ///
    /// The read happens here rather than in the pane so a file that cannot be
    /// read produces a message instead of an empty pane titled after it — an
    /// empty buffer and a missing file look identical once drawn.
    ///
    /// The file is read even when it is already open, because `BufferStore`
    /// answers the "is it open?" question by canonical path and its answer is
    /// the one that decides. The content is then ignored, so a second pane on a
    /// file being edited shows the edits rather than what is on disk — the
    /// whole point of asking the store rather than opening a document per pane.
    fn document_for(&mut self, path: &str) -> Result<ruster_core::document::BufferId, String> {
        let expanded = expand_home(path);
        match std::fs::read_to_string(&expanded) {
            Ok(text) => Ok(self
                .buffers
                .open_file(std::path::PathBuf::from(expanded), text)),
            Err(err) => {
                tracing::warn!(%path, %err, "could not open the file");
                Err(format!("{path}: {err}"))
            }
        }
    }

    /// Open `path` — in the focused pane if there is one, otherwise in a new
    /// pane — or report why not.
    ///
    /// Replacing the focused pane's document is what `:e` means in vim, and it
    /// is the only way to walk a project without accumulating a tile per file.
    /// When focus is a client there is nothing to replace: a Wayland window
    /// cannot show a file, so the file needs a leaf of its own.
    pub fn open_file(&mut self, path: &str) {
        let doc = match self.document_for(path) {
            Ok(doc) => doc,
            Err(message) => return self.report(message),
        };
        match self.focused_pane() {
            Some(pane) => self.show_document(pane, doc),
            None => self.open_pane_with(doc),
        }
        // After the pane exists, so a server that answers instantly has
        // somewhere to put its diagnostics.
        self.lsp_open(doc);
    }

    /// Point the pane at `id` at `doc`, and re-title its leaf to match.
    ///
    /// The title is carried rather than stored twice: `ShellState` is what the
    /// statusline and the session file read, and a pane that changed document
    /// without changing it would report the file it used to be showing.
    pub fn show_document(&mut self, id: WindowId, doc: ruster_core::document::BufferId) {
        let Some(target) = self.buffers.get(doc) else {
            return;
        };
        let name = target.name.clone();
        let Some(pane) = self.panes.get_mut(&id) else {
            return;
        };
        pane.show(doc, &target.buffer);
        if let Some(window) = self.shell.window(id) {
            window.set_title(name);
        }
    }

    /// Show the open document `name` picks out in the focused pane.
    pub fn show_named_document(&mut self, name: &str) {
        let Some(id) = self.focused_pane() else {
            return self.report(format!("{name}: no editor pane has focus"));
        };
        match self.find_document(name) {
            Ok(doc) => self.show_document(id, doc),
            Err(message) => self.report(message),
        }
    }

    /// The open document `name` picks out: the one called exactly that, or the
    /// single one whose name or path contains it.
    ///
    /// Ambiguity is refused rather than resolved by order. Two documents can
    /// easily share a name — `mod.rs`, `main.rs` — and switching to whichever
    /// happened to be opened first is a pane showing the wrong file with nothing
    /// to say it did.
    fn find_document(&self, name: &str) -> Result<ruster_core::document::BufferId, String> {
        let mut partial = Vec::new();
        for id in self.buffers.ids() {
            let Some(doc) = self.buffers.get(*id) else {
                continue;
            };
            if doc.name == name {
                return Ok(*id);
            }
            let path = doc
                .file_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            if doc.name.contains(name) || path.contains(name) {
                partial.push(*id);
            }
        }
        match partial.len() {
            0 => Err(format!("no open buffer matches: {name}")),
            1 => Ok(partial[0]),
            n => Err(format!("{n} open buffers match: {name}")),
        }
    }

    /// Where the focused pane's caret is, as the protocol counts it.
    ///
    /// Returns the pane, the caret as the protocol counts it, and the server and
    /// URI to ask. The server comes from the document rather than being
    /// re-derived from the path: a request routed to a second server for the
    /// same file would be answered against a document that server has never been
    /// told about.
    ///
    /// Hands back the position rather than finished params so that building them
    /// stays at the call site, which is the only thing keeping `serde_json` out
    /// of this crate's dependencies.
    fn lsp_target(
        &mut self,
        verb: &str,
    ) -> Option<(
        WindowId,
        ruster_lsp::LspPosition,
        ruster_lsp::ServerKey,
        String,
    )> {
        let Some(id) = self.focused_pane() else {
            self.report(format!("{verb}: no editor pane has focus"));
            return None;
        };
        let pane = self.panes.get(&id)?;
        let (doc_id, offset) = (pane.doc, pane.cursors.primary().head);
        let document = self.buffers.get(doc_id)?;
        let text = document.buffer.to_string();
        let Some(lsp_doc) = self.lsp.doc(doc_id) else {
            self.report(format!("{verb}: no language server for this file"));
            return None;
        };
        let uri = lsp_doc.uri.clone();
        let key = lsp_doc.key.clone();
        // Character offsets are not LSP positions: the protocol counts UTF-16
        // code units, so anything past an astral character on the line would ask
        // about the wrong column.
        let pos = ruster_lsp::position::offset_to_position(&text, offset);
        Some((id, pos, key, uri))
    }

    /// How long a capture may wait for a frame before it is called a failure.
    ///
    /// Generous on purpose: a request made while the host happens not to be
    /// presenting should still be served when it next does, and a host under
    /// load can be a good few frames late. This is only meant to catch the case
    /// where no frame is coming at all.
    const SCREENSHOT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

    /// How long until a waiting capture — of either kind — must be given up on.
    ///
    /// Both paths queue work that only a rendered frame can finish, and both
    /// give up after a timeout that is checked once per loop pass. The loop
    /// blocks until something happens, so a deadline nothing wakes it for is a
    /// deadline that never arrives: the screencopy half of this was found when a
    /// client hung, and the keybind half when the compositor slept through its
    /// own quit and had to be forced out by the second signal.
    pub fn next_capture_deadline(&self, now: std::time::Instant) -> Option<std::time::Duration> {
        let screenshot = self.screenshot_pending.map(|asked| {
            Self::SCREENSHOT_TIMEOUT.saturating_sub(now.saturating_duration_since(asked))
        });
        let screencopy = self.screencopy.next_deadline(now);
        [screenshot, screencopy].into_iter().flatten().min()
    }

    /// Say so when a screenshot has been waiting for a frame that is not coming.
    ///
    /// Called once per event-loop pass. Returns whether it gave up, which is
    /// what the tests assert on — the warning itself is the point of the
    /// function, but a log line is not something a test can hold.
    pub fn screenshot_overdue(&mut self, now: std::time::Instant) -> bool {
        let Some(asked) = self.screenshot_pending else {
            return false;
        };
        if now.duration_since(asked) < Self::SCREENSHOT_TIMEOUT {
            return false;
        }
        // Cleared rather than left pending. A capture that lands thirty seconds
        // late, when the host finally presents, is a PNG of a moment nobody
        // asked about — and on a verification run it would be filed as evidence
        // of the frame that was meant to be captured.
        self.screenshot_pending = None;
        tracing::warn!(
            waited_ms = now.duration_since(asked).as_millis() as u64,
            "screenshot not taken: no frame has been rendered since it was asked for. \
             Rendering waits for the host to invite a frame, and a nested window that is \
             occluded or on another workspace is never invited"
        );
        self.report("screenshot: no frame to capture".to_string());
        true
    }

    /// Ask where the symbol under the cursor is defined.
    ///
    /// Best effort like everything else here: a file with no server, a cursor on
    /// nothing, or a server that answers with no location all leave the pane
    /// exactly as it was, and say why in the minibuffer rather than in silence.
    pub fn lsp_definition(&mut self) {
        let Some((_, pos, key, uri)) = self.lsp_target("definition") else {
            return;
        };
        let params = ruster_lsp::protocol::text_document_position(&uri, pos);
        if !self.lsp.request(
            &key,
            "textDocument/definition",
            params,
            LspPending::Definition,
        ) {
            self.report("definition: the language server is not running".to_string());
        }
    }

    /// Ask what the symbol under the cursor is, and put the answer beside it.
    pub fn lsp_hover(&mut self) {
        let Some((pane, pos, key, uri)) = self.lsp_target("hover") else {
            return;
        };
        let (row, col) = (pos.line as usize, pos.character as usize);
        let params = ruster_lsp::protocol::text_document_position(&uri, pos);
        // Any previous panel goes now rather than when the reply lands. Leaving
        // it up would mean a stale explanation sitting beside a new caret for as
        // long as the server takes to answer, which is exactly the window in
        // which it is most likely to be believed.
        self.hover = None;
        if !self.lsp.request(
            &key,
            "textDocument/hover",
            params,
            LspPending::Hover { pane, row, col },
        ) {
            self.report("hover: the language server is not running".to_string());
        }
    }

    /// Put the server's answer beside the caret it was asked about.
    pub(crate) fn show_hover(&mut self, pane: WindowId, row: usize, col: usize, markup: &str) {
        // Markdown, minus the parts that mean nothing without a renderer. The
        // editor highlights fenced code; this keeps the code and drops the
        // fence, which is the difference between a panel and a parser.
        let mut lines: Vec<String> = markup
            .lines()
            .filter(|l| !l.trim_start().starts_with("```") && l.trim() != "---")
            .map(|l| l.trim_end().to_string())
            .skip_while(|l| l.is_empty())
            .collect();
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        // A server that answers with nothing but formatting has said nothing,
        // and an empty panel beside the caret is worse than no panel: it reads
        // as "this symbol is not documented" rather than "nobody was asked".
        if lines.is_empty() {
            self.hover = None;
            return self.report("hover: nothing to show here".to_string());
        }
        self.hover = Some(HoverPanel {
            pane,
            row,
            col,
            lines,
        });
    }

    /// Show whatever the server said `textDocument/definition` resolves to.
    ///
    /// Takes the locations already parsed rather than the raw reply, which keeps
    /// `serde_json` out of this crate's dependencies for the sake of one type in
    /// one signature.
    pub(crate) fn goto_definition(&mut self, locations: &[ruster_lsp::Location]) {
        let Some(location) = locations.first() else {
            return self.report("definition: no definition found".to_string());
        };
        // `parse_locations` has already turned a `file://` URI into a path, and
        // handles both `Location` and `LocationLink`, which are different shapes
        // for the same answer that different servers choose between.
        self.open_file(&location.uri);
        let Some(id) = self.focused_pane() else {
            return;
        };
        // `panes` and `buffers` are distinct fields, so the mutable and
        // immutable borrows below are disjoint.
        let Some(pane) = self.panes.get_mut(&id) else {
            return;
        };
        let Some(document) = self.buffers.get(pane.doc) else {
            return;
        };
        // Back through the same conversion, the other way: the server answers in
        // UTF-16 columns and the buffer is indexed by character.
        let text = document.buffer.to_string();
        let offset = ruster_lsp::position_to_offset(
            &text,
            ruster_lsp::LspPosition {
                line: location.start.line,
                character: location.start.character,
            },
        );
        pane.cursors = ruster_core::cursor::CursorSet::single(offset);
        // Without this the jump lands off screen whenever the definition is
        // further down the file than the pane is tall, which is most of the time
        // and looks exactly like nothing happening.
        pane.follow_cursor(&document.buffer);
    }

    /// Tell a language server about a document a pane has opened.
    ///
    /// Best effort by design: a language with no server configured, or a server
    /// that is not installed, must leave the pane working. An editor that
    /// refused to open a file because `rust-analyzer` was missing would be worse
    /// than one without diagnostics.
    pub fn lsp_open(&mut self, doc: ruster_core::document::BufferId) {
        let Some(document) = self.buffers.get(doc) else {
            return;
        };
        let Some(path) = document.file_path.clone() else {
            return;
        };
        let Some(lang) = language_of(&path) else {
            return;
        };
        let root = ruster_lsp::state::LspState::<LspPending>::root_for(&path);
        let text = document.buffer.to_string();
        let sync = self.lsp.sync(doc, &path, lang, &text, &root);
        // Said out loud because everything about a language server is invisible
        // otherwise: it runs in another process, and when it fails to start it
        // does so silently. A row of "no diagnostics" is indistinguishable from
        // a server that never launched.
        tracing::info!(lang, path = %path.display(), root = %root.display(), ?sync, "lsp open");
    }

    /// Drain whatever the servers have said and file the diagnostics.
    ///
    /// Called once per event-loop pass, beside the Lua queue: `LspClient` reads
    /// its server on a thread into an mpsc channel, so polling is a channel
    /// drain and never blocks the compositor on a process that has stopped
    /// answering.
    pub fn poll_lsp(&mut self) {
        for routed in self.lsp.poll() {
            // Replies to our own requests. Everything that is not a
            // notification used to be dropped here with a trace line, which is
            // why no request had ever been worth sending: `hover` and
            // `definition` are questions, and nothing was listening for answers.
            if let ruster_lsp::ServerMessage::Response { id, result, error } = &routed.message {
                let Some(pending) = self.lsp.take_pending(&routed.key, *id) else {
                    // A reply to a request this compositor did not send, or one
                    // whose answer has already been taken.
                    tracing::debug!(id, "lsp reply with nothing waiting on it");
                    continue;
                };
                if let Some(error) = error {
                    tracing::warn!(?pending, %error, "lsp request failed");
                    self.report(format!("{pending:?}: the language server refused"));
                    continue;
                }
                match pending {
                    LspPending::Definition => {
                        let locations = ruster_lsp::parse_locations(result);
                        self.goto_definition(&locations);
                    }
                    LspPending::Hover { pane, row, col } => match ruster_lsp::parse_hover(result) {
                        Some(markup) => self.show_hover(pane, row, col, &markup),
                        None => {
                            self.hover = None;
                            self.report("hover: nothing to show here".to_string());
                        }
                    },
                }
                continue;
            }
            let ruster_lsp::ServerMessage::Notification { method, params } = &routed.message else {
                tracing::trace!("lsp non-notification");
                continue;
            };
            tracing::trace!(method, "lsp notification");
            if method != "textDocument/publishDiagnostics" {
                continue;
            }
            let (path, diags) = ruster_lsp::parse_diagnostics(params);
            // Matched by path, because that is what the server names. The pane
            // knows its `BufferId` and the document knows its path, so this is
            // the one place the two are joined.
            let doc = self.document_at_path(&path);
            // Logged before the match rather than after, so a server reporting a
            // path no pane holds — a symlinked root, a file opened by a
            // different name — reads as a routing failure rather than as
            // silence indistinguishable from a clean file.
            tracing::debug!(%path, count = diags.len(), matched = doc.is_some(), "diagnostics");
            let Some(doc) = doc else {
                continue;
            };
            self.lsp.set_diagnostics(doc, diags);
        }
    }

    /// The document opened from `path`, if a pane has one.
    fn document_at_path(&self, path: &str) -> Option<ruster_core::document::BufferId> {
        let wanted = std::path::Path::new(path);
        self.buffers.ids().iter().copied().find(|id| {
            match self.buffers.get(*id).and_then(|d| d.file_path.as_deref()) {
                Some(opened) => same_file(opened, wanted),
                None => false,
            }
        })
    }

    /// Write the focused pane's document back to the file it came from.
    ///
    /// A pane with no path says so: it is a scratch buffer, there is nowhere to
    /// put it, and a `:w` that quietly did nothing would be indistinguishable
    /// from one that worked until the next boot.
    pub fn write_pane(&mut self) {
        let Some(id) = self.focused_pane() else {
            return self.report("no editor pane has focus");
        };
        let Some((_, doc)) = self.pane_document(id) else {
            return;
        };
        let Some(path) = doc.file_path.clone() else {
            return self.report(format!("{}: no file name", doc.name));
        };
        // `encode_content`, not `buffer.to_string()`: the document remembers the
        // line ending it was read with, and writing a CRLF file back as LF turns
        // a one-character edit into a whole-file diff.
        let content = doc.encode_content();
        let lines = doc.buffer.line_count();
        match std::fs::write(&path, content) {
            Ok(()) => {
                if let Some((_, doc)) = self.pane_document_mut(id) {
                    doc.modified = false;
                }
                tracing::info!(path = %path.display(), lines, "wrote a pane's document");
                self.report(format!("wrote {} ({lines} lines)", path.display()));
            }
            Err(err) => {
                tracing::warn!(path = %path.display(), %err, "could not write the file");
                self.report(format!("{}: {err}", path.display()));
            }
        }
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
            Some(wm) => {
                let mut queued = wm.take_actions();
                // Deferred actions after the immediate ones, so that within a
                // single pass "do this now" still precedes "do this later" even
                // when the later one has come due on the same iteration.
                queued.extend(wm.take_due(std::time::Instant::now()));
                queued
            }
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
            Action::Screenshot => {
                self.screenshot_pending = Some(std::time::Instant::now());
                // Ask for the frame it needs, as a screencopy capture does.
                // Without this the keybind waits for a frame that may never be
                // invited and then reports a failure that is really the
                // compositor's for never asking — the two capture paths had
                // different answers to the same problem.
                self.backend_data.request_redraw();
            }
            Action::ToggleHelp => self.help_pinned = !self.help_pinned,
            Action::NewPane => self.open_pane(),
            Action::Edit(path) => self.open_file(&path),
            Action::Definition => self.lsp_definition(),
            Action::Hover => self.lsp_hover(),
            Action::Launcher => {
                // The overlay owns the screen while it is up: a which-key panel
                // or a hover float beside it would be drawn by a different
                // batch and read as part of it.
                self.minibuffer = None;
                self.hover = None;
                self.providers.prepare();
                self.launcher = Some(crate::launcher::Launcher::new());
                self.launcher_refresh();
            }
            Action::Write => self.write_pane(),
            Action::ShowBuffer(name) => self.show_named_document(&name),
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
            Action::Terminal => self.spawn_terminal(),
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

    /// Launch the user's terminal, naming the one chosen and where the choice
    /// came from.
    ///
    /// The naming is the point. A terminal keybind that resolves three ways can
    /// fail three ways, and on a DRM boot — where this bind is the only route to
    /// a second window — an unexplained nothing is indistinguishable from a
    /// broken keymap, a missing binary and a compositor that never got the key.
    fn spawn_terminal(&mut self) {
        match crate::lua::terminal_command(self.terminal.as_deref()) {
            // Down the same path as `Action::Spawn`, so a terminal opened this
            // way is recorded for the session file like any other client.
            Some((command, source)) => {
                info!(%command, ?source, "terminal");
                self.persist.spawn(&command, self.socket_name.as_deref());
            }
            None => warn!(
                candidates = ?crate::lua::KNOWN_TERMINALS,
                "no terminal found: set `terminal` in compositor.lua, or $TERMINAL, \
                 or install one of these"
            ),
        }
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
            let Some(client) = self.clients.get(&id) else {
                continue;
            };
            // The tiled states and the X11 position both live in `configure`,
            // because the two protocols need different things said to make the
            // same thing true. The titlebar goes away separately, via
            // xdg-decoration (see `shell::answer_decoration`) — tiled states
            // alone do not stop a toolkit drawing one.
            client.configure(rect);
        }
    }

    /// Put a newly created window into the tree and give it the keyboard.
    ///
    /// Shared by both protocols on purpose. `minibuffer.rs` states the rule this
    /// follows — "no route into the WM can do something the others cannot" — and
    /// the failure mode when an X11 window takes its own path is not a crash but
    /// a divergence: it tiles but does not restore, or focuses but does not
    /// reconfigure its neighbours, and every symptom looks like a different bug.
    pub fn place_new_client(
        &mut self,
        id: WindowId,
        client: crate::client::Client,
        pid: Option<u32>,
    ) {
        // Insert beside whatever has focus, on the workspace being shown, so a
        // new window splits the one you were looking at rather than appearing
        // somewhere arbitrary — unless the saved session was waiting for this
        // client, in which case it goes back where it was.
        let near = self.shell.focus;
        if !self.place_restored_window(id, pid) {
            self.workspaces
                .insert(id, near, ruster_shell::Layout::Horizontal);
        }
        // A restored window can land on a workspace that is not on screen, and
        // it must not take the keyboard there: every keystroke would go to a
        // client the user cannot see.
        if self.workspaces.is_visible(id) {
            self.shell.set_focus(id);
            self.pending_focus = Some(id);
        } else {
            self.shell.focus = self.workspaces.focus_for_active(self.shell.focus);
        }
        self.clients.insert(id, client);
        // Every existing window just got smaller; tell them before the new one
        // draws, or the first frame overlaps its neighbour.
        self.reconfigure_tiles();
    }

    /// Take a window out of the tree, wherever it was, and let the survivors
    /// grow into the space.
    pub fn remove_client(&mut self, id: WindowId) {
        self.clients.remove(&id);
        self.mapped.remove(&id);
        // Wherever it was: a client can close while its workspace is hidden.
        self.workspaces.remove(id);
        // `remove_window` refocuses the shell onto the most recent window (or
        // clears focus), but it knows nothing of workspaces and will happily
        // name one that is off screen; the workspaces have the last word.
        self.shell.remove_window(id);
        self.shell.focus = self.workspaces.focus_for_active(self.shell.focus);
        self.reconfigure_tiles();
        self.update_keyboard_focus(SCOUNTER.next_serial());
    }

    /// Adopt an X11 window that XWayland has asked us to map.
    pub fn insert_client(&mut self, client: crate::client::Client) -> WindowId {
        let title = match &client {
            crate::client::Client::X11(surface) => surface.title(),
            crate::client::Client::Wayland(_) => String::new(),
        };
        let id = self.shell.add_window(title, 800, 600);
        let pid = match &client {
            crate::client::Client::X11(surface) => surface.pid(),
            crate::client::Client::Wayland(_) => None,
        };
        self.place_new_client(id, client, pid);
        id
    }

    /// The window id holding this X11 surface, if the tree has adopted it.
    pub fn window_for_x11(&self, window: &smithay::xwayland::X11Surface) -> Option<WindowId> {
        self.clients
            .iter()
            .find(|(_, client)| matches!(client, crate::client::Client::X11(s) if s == window))
            .map(|(id, _)| *id)
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

/// The language id for a path, or `None` when nothing here speaks it.
///
/// Deliberately short. A language with no entry simply gets no server, which is
/// the same outcome as a server that is not installed — and both leave the pane
/// working.
/// Whether a buffer opened as `opened` is the file a language server named as
/// `reported`.
///
/// Not `==`. The server is told a canonicalised URI, so a buffer opened through
/// a symlink, a relative path or a `..` comes back under a name that never
/// matches the one it was opened with, and its diagnostics are dropped in
/// silence. The same mismatch is what `fix(lsp): resolve symlinks when building
/// file URIs` fixed on the editor side.
///
/// Literal equality stays as the fallback so a buffer whose file has been
/// deleted or moved — where `canonicalize` fails — still matches itself.
pub fn same_file(opened: &std::path::Path, reported: &std::path::Path) -> bool {
    if opened == reported {
        return true;
    }
    match (
        std::fs::canonicalize(opened),
        std::fs::canonicalize(reported),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn language_of(path: &std::path::Path) -> Option<&'static str> {
    match path.extension()?.to_str()? {
        "rs" => Some("rust"),
        "py" => Some("python"),
        "ts" | "tsx" => Some("typescript"),
        "js" | "jsx" => Some("javascript"),
        "go" => Some("go"),
        "c" | "h" => Some("c"),
        "cpp" | "hpp" | "cc" => Some("cpp"),
        "lua" => Some("lua"),
        _ => None,
    }
}

/// `~` expanded to the home directory.
///
/// Only the leading `~`, and only when it starts a path — a file legitimately
/// named `a~b` is not a home directory reference, and expanding it would open
/// the wrong thing while looking like it worked.
fn expand_home(path: &str) -> String {
    match path.strip_prefix("~/") {
        Some(rest) => match dirs::home_dir() {
            Some(home) => home.join(rest).to_string_lossy().into_owned(),
            None => path.to_string(),
        },
        None => path.to_string(),
    }
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
    layer_shell_state: WlrLayerShellState,
    xwayland_shell_state: smithay::wayland::xwayland_shell::XWaylandShellState,
    seat_state: SeatState<CompositorState<B>>,
    seat: Seat<CompositorState<B>>,
    keyboard: KeyboardHandle<CompositorState<B>>,
    pointer: PointerHandle<CompositorState<B>>,
    /// The configuration the keyboard above was built with, so the state can
    /// record what the seat was actually told.
    keyboard_config: crate::lua::KeyboardConfig,
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
        clients: HashMap::new(),
        pending_focus: None,
        mapped: HashSet::new(),
        // The user's theme, not the built-in one: the compositor drew with
        // `Theme::default()` while `ruster-lua` sat unimported in its
        // Cargo.toml, so every colour the editor lets you configure was ignored
        // here.
        chrome: Some(Chrome::new(user_theme())),
        screenshot_pending: None,
        screenshot_count: 0,
        keymap: crate::keymap::Keymap::default(),
        chord: crate::keymap::ChordState::default(),
        intercepted: HashSet::new(),
        repeat: None,
        repeat_generation: 0,
        keyboard_config: globals.keyboard_config,
        terminal: None,
        lsp: ruster_lsp::state::LspState::new(),
        hover: None,
        screencopy: crate::screencopy::ScreencopyState::default(),
        layer_shell_state: globals.layer_shell_state,
        xwayland_shell_state: globals.xwayland_shell_state,
        xwm: None,
        x11_unmanaged: Vec::new(),
        launcher: None,
        providers: {
            // Registration order is the tie-break when two providers are
            // equally confident, so it is the order they appear in.
            let mut set = crate::launcher::ProviderSet::default();
            set.push(Box::new(crate::launcher::math::MathProvider));
            set.push(Box::new(crate::launcher::desktop::AppsProvider::default()));
            set
        },
        highlights: std::cell::RefCell::new(crate::highlight::Highlights::default()),
        clipboard: crate::clipboard::Clipboard::default(),
        panes: crate::pane::Panes::new(),
        buffers: ruster_core::workspace::BufferStore::new(),
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
    // `Xbgr8888` on top of the two formats wl_shm always has. Screencopy offers
    // it — it is the byte order `copy_framebuffer` already returns — and a
    // format offered by one global and refused by another is not a capture
    // anyone can take: `grim` asked for exactly what it was told, and wl_shm
    // answered `format Xbgr8888 not supported`.
    let shm_state = ShmState::new::<CompositorState<B>>(
        dh,
        vec![smithay::reexports::wayland_server::protocol::wl_shm::Format::Xbgr8888],
    );
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
    // `zxdg_output_manager_v1`, alongside the `wl_output` each backend creates.
    //
    // It reports an output's position and size in *logical* coordinates, which
    // is what a client compositing several outputs needs and what `wl_output`
    // alone cannot express. Without it `grim` says "guessing the output layout"
    // and guesses 0x0, so a capture that worked in every other respect was
    // written as a zero-by-zero PNG. The delegate for this was already in place;
    // only the global was missing.
    let _output_manager_state = OutputManagerState::new_with_xdg_output::<CompositorState<B>>(dh);
    // `zwlr_layer_shell_v1`: the protocol bars, notification daemons and
    // wallpaper setters speak. Without it none of them can map a surface at
    // all, which is why an external launcher was never an option here either.
    let layer_shell_state = WlrLayerShellState::new::<CompositorState<B>>(dh);
    // Advertised unconditionally. Only XWayland ever binds it, and it is how
    // XWayland pairs an X11 window with the `wl_surface` carrying its pixels.
    let xwayland_shell_state =
        smithay::wayland::xwayland_shell::XWaylandShellState::new::<CompositorState<B>>(dh);
    // wlr-screencopy. Not a smithay state object — the protocol is not
    // implemented there, so this is a bare global whose `Dispatch` impls live in
    // `crate::screencopy`. Version 3 for `buffer_done`; only shm buffers are
    // offered, and a client that wants dmabuf will take the shm path it is also
    // required to support.
    dh.create_global::<CompositorState<B>, wayland_protocols_wlr::screencopy::v1::server::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1, _>(3, ());
    let mut seat_state = SeatState::new();
    let mut seat = seat_state.new_wl_seat(dh, seat_name);
    let pointer = seat.add_pointer();
    // The system keymap to start with — `XkbConfig::default()` compiles the
    // libxkbcommon default, which honours `XKB_DEFAULT_LAYOUT` and friends, so
    // an unconfigured compositor already matches the rest of the session. A
    // config that names its own is applied afterwards by
    // `lua::apply_keyboard_config`, which can fail safely; this one cannot fail
    // at all without the machine being broken.
    let keyboard_config = crate::lua::KeyboardConfig::default();
    let keyboard = seat
        .add_keyboard(
            XkbConfig::default(),
            keyboard_config.repeat_delay,
            keyboard_config.repeat_rate,
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
        layer_shell_state,
        xwayland_shell_state,
        seat_state,
        seat,
        keyboard,
        pointer,
        keyboard_config,
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
        // Two kinds of client now, and only one of them is ours. XWayland
        // connects to the compositor as an ordinary Wayland client, but smithay
        // spawns it and attaches its own `XWaylandClientData` — so the `expect`
        // here, which had been true for every client since Phase 0, became a
        // panic the moment X11 support was switched on. It fired on the first
        // run: `client has no ClientState`, taking the whole compositor down
        // before XWayland had finished starting.
        if let Some(state) = client.get_data::<ClientState>() {
            return &state.compositor_state;
        }
        &client
            .get_data::<smithay::xwayland::XWaylandClientData>()
            .expect("a client that is neither ruster's nor XWayland's")
            .compositor_state
    }

    fn commit(&mut self, surface: &wl_surface::WlSurface) {
        // The popup manager needs every commit: it is what advances a popup
        // from "created" to "mapped".
        self.popups.commit(surface);
        // A layer surface has to be told its size before it can draw, the same
        // way a popup does — and the map has to be re-arranged when one changes,
        // or a bar that grew keeps its old strip and the windows keep the old
        // gap.
        //
        // The configure is sent with `send_pending_configure`, and emphatically
        // *not* with `ensure_configured`, whose name reads like a request and is
        // an assertion: it raises a protocol error when the surface has not been
        // configured, which on a first commit it never has. It killed the client
        // it was meant to be serving — `Protocol error 2 on
        // zwlr_layer_surface_v1: layer_surface has never been configured`, from
        // the compositor, to a bar doing exactly the right thing.
        {
            let output = self.backend_data.output().clone();
            let mut map = smithay::desktop::layer_map_for_output(&output);
            let found = map
                .layer_for_surface(surface, smithay::desktop::WindowSurfaceType::ALL)
                .cloned();
            if let Some(layer) = found {
                // Arranged before the configure is sent, because the configure
                // carries the size and the arrangement is what decides it.
                let rearranged = map.arrange();
                drop(map);
                layer.layer_surface().send_pending_configure();
                if rearranged {
                    // One line per bar that maps, resizes or goes away — not per
                    // frame. It is the only place the exclusive zone becomes
                    // visible from outside: a bar that maps but is ignored, and
                    // a bar that is honoured, look identical until this prints
                    // an area shorter than the output.
                    let area = self.output_rect();
                    tracing::info!(
                        x = area.x,
                        y = area.y,
                        w = area.w,
                        h = area.h,
                        "layer surfaces rearranged; tiling into"
                    );
                    self.reconfigure_tiles();
                }
            }
        }
        // Sending the initial configure is *not* part of that, though this used
        // to claim it was. `track_popup` only records the popup and
        // `PopupManager::commit` only moves it between two lists — neither
        // touches the protocol. So the client was told its menu was tracked and
        // never told how big it was, and a client with no size attaches no
        // buffer: the popup was positioned, unconstrained, hit-tested, drawn in
        // the right order and empty, which on screen is indistinguishable from
        // the untracked popups this was written to fix. Found by right-clicking
        // in weston-terminal and reading the log against the pixels: `popup
        // tracked x=56 y=66 w=276 h=156` with nothing on screen at all.
        if let Some(smithay::desktop::PopupKind::Xdg(popup)) = self.popups.find_popup(surface) {
            if !popup.is_initial_configure_sent() {
                if let Err(err) = popup.send_configure() {
                    tracing::warn!(%err, "could not send a popup's initial configure");
                }
            }
        }
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
        // `xdg_shell`'s handshake alone. An X11 window has no `xdg_surface` and
        // therefore no initial configure to withhold — XWayland maps it when the
        // window manager says so, which happens in `map_window_request`.
        if let Some(toplevel) = self.clients.get(&id).and_then(|c| c.toplevel()) {
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

impl<B: Backend + 'static> WlrLayerShellHandler for CompositorState<B> {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    /// A bar, a notification daemon or a wallpaper has appeared.
    ///
    /// It goes into the output's `LayerMap` rather than into the container tree.
    /// That is the whole distinction: a layer surface is not a window the user
    /// tiles, it is chrome the compositor arranges from the anchors and margins
    /// the client asked for — and smithay's `arrange` is what turns those into a
    /// rectangle.
    fn new_layer_surface(
        &mut self,
        surface: LayerSurface,
        _output: Option<smithay::reexports::wayland_server::protocol::wl_output::WlOutput>,
        layer: WlrLayer,
        namespace: String,
    ) {
        // The client may name an output; with one output there is nothing to
        // choose between, and honouring a name we cannot satisfy would be worse
        // than putting it on the only screen there is.
        let output = self.backend_data.output().clone();
        let mut map = smithay::desktop::layer_map_for_output(&output);
        // `LayerMap` works in `desktop::LayerSurface`, which pairs the shell
        // surface with the namespace it announced — that name is what a config
        // would key rules on, and what a warning has to be able to say.
        let desktop_surface = smithay::desktop::LayerSurface::new(surface, namespace.clone());
        if let Err(err) = map.map_layer(&desktop_surface) {
            tracing::warn!(%namespace, ?layer, %err, "could not map a layer surface");
            return;
        }
        tracing::info!(%namespace, ?layer, "layer surface mapped");
        drop(map);
        // The tiling area may have just shrunk — a bar takes its space out of
        // it — so every window needs its rectangle again.
        self.reconfigure_tiles();
    }

    fn layer_destroyed(&mut self, surface: LayerSurface) {
        let output = self.backend_data.output().clone();
        let mut map = smithay::desktop::layer_map_for_output(&output);
        // Found by its `wl_surface` rather than kept in a side table: the map
        // already owns the pairing, and a second record of which layer is which
        // is a second thing to keep in step.
        let found = map
            .layer_for_surface(
                surface.wl_surface(),
                smithay::desktop::WindowSurfaceType::TOPLEVEL,
            )
            .cloned();
        if let Some(layer) = found {
            map.unmap_layer(&layer);
        }
        drop(map);
        // And the space it reserved comes back.
        self.reconfigure_tiles();
    }
}

impl<B: Backend + 'static> OutputHandler for CompositorState<B> {
    fn output_bound(&mut self, _output: Output, _wl_output: wl_output::WlOutput) {}
}

impl<B: Backend + 'static> SeatHandler for CompositorState<B> {
    // One type for all three, and a newtype over `WlSurface` rather than the
    // surface itself: `PopupManager::grab_popup` needs
    // `KeyboardFocus: From<PopupKind>`, and neither of those is local, so with a
    // bare `WlSurface` the orphan rule made a real popup grab impossible to
    // write. See [`crate::focus`].
    type KeyboardFocus = crate::focus::FocusTarget;
    type PointerFocus = crate::focus::FocusTarget;
    // Touch stays a bare surface. `grab_popup` constrains only the keyboard and
    // pointer focus, `process_input_event` drops touch events, and a
    // `TouchTarget` impl here would be delegation nothing calls — which is the
    // kind of code that is wrong for months without anyone finding out.
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

    /// A client is reading the selection a pane put there.
    fn send_selection(
        &mut self,
        _ty: SelectionTarget,
        mime_type: String,
        fd: std::os::unix::io::OwnedFd,
        _seat: Seat<Self>,
        _user_data: &(),
    ) {
        if !crate::clipboard::is_text_mime(&mime_type) {
            return;
        }
        // Written here rather than on the event loop: the far end is a pipe the
        // client just created and is waiting on, and a selection is small. The
        // fd is dropped either way, which is what tells the client the transfer
        // ended — leaking it would hang whatever asked.
        use std::io::Write;
        let mut file = std::fs::File::from(fd);
        if let Err(err) = file.write_all(self.clipboard.text().as_bytes()) {
            tracing::warn!(%err, "could not hand over the selection");
        }
    }

    /// Someone has taken the selection.
    ///
    /// `source` is `Some` when a *client* set it, and the text is fetched here
    /// so a pane can paste it later. Not fetched at paste time: reading means
    /// asking the owning client for a pipe and waiting on it, and a keystroke
    /// cannot wait — a client that never answered would hang the display server.
    fn new_selection(
        &mut self,
        ty: SelectionTarget,
        source: Option<SelectionSource>,
        seat: Seat<Self>,
    ) {
        if ty != SelectionTarget::Clipboard {
            return;
        }
        let Some(source) = source else {
            return;
        };
        self.clipboard.released();
        let Some(mime) = crate::clipboard::preferred_mime(&source.mime_types()) else {
            return;
        };
        match fetch_client_selection(&seat, mime) {
            Ok(text) => self.clipboard.set_from_client(text),
            Err(err) => tracing::warn!(%err, "could not read the client selection"),
        }
    }
}

/// Read the current client selection for `seat` as text.
///
/// A pipe, handed to the client to write into, read back here. Bounded by
/// [`SELECTION_LIMIT`](crate::clipboard::SELECTION_LIMIT) because the far end is
/// another process and this one is the display server.
fn fetch_client_selection<B: Backend + 'static>(
    seat: &Seat<CompositorState<B>>,
    mime: String,
) -> Result<String, String> {
    let (rx, tx) = std::os::unix::net::UnixStream::pair().map_err(|err| err.to_string())?;
    request_data_device_client_selection(seat, mime, tx.into())
        .map_err(|err| format!("{err:?}"))?;
    crate::clipboard::read_selection(rx.into(), crate::clipboard::SELECTION_LIMIT)
        .map_err(|err| err.to_string())
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
smithay::delegate_layer_shell!(@<B: Backend + 'static> CompositorState<B>);
smithay::delegate_xwayland_shell!(@<B: Backend + 'static> CompositorState<B>);
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
    let asked = Arc::new(AtomicBool::new(false));
    ctrlc::set_handler(
        move || match signal_action(asked.swap(true, Ordering::SeqCst)) {
            SignalAction::AskToStop => {
                flag.store(false, Ordering::SeqCst);
                stop.stop();
            }
            SignalAction::ForceExit => {
                // Deliberately not a clean shutdown: the loop has not come back to
                // read the flag the first signal set, so there is nothing left to
                // ask. Writing the session from a signal handler while the main
                // thread is stopped inside a blocking call is not safe, so the
                // session is the price of getting out.
                eprintln!("ruster-compositor: not responding, forcing exit");
                // `abort`, not `exit`. `std::process::exit` runs atexit handlers
                // and static destructors, and those can want a lock the stalled
                // main thread is holding — which is exactly what happened the
                // first time this was written: the message printed and the
                // process stayed up. Termination here has to be the one thing
                // that cannot block.
                std::process::abort();
            }
        },
    )?;
    Ok(())
}

/// What a shutdown signal should do, given whether one has already been asked
/// for.
///
/// Split out so the decision is testable: a handler that only ever asks nicely
/// leaves no way out of a stalled loop, and a compositor you cannot signal is a
/// compositor you have to find another machine to kill.
///
/// The stall is real and has been observed. `WinitGraphicsBackend::submit` ends
/// in `eglSwapBuffers`, which on Wayland blocks until the host releases a
/// buffer; when the host stops presenting to the window, the main thread stops
/// there and never reaches the `running` check. The DRM backend does not share
/// that call — it presents through page flips on vblank, and already declines to
/// render while the session is inactive — but a second signal costs nothing and
/// covers whatever else might one day block.
fn signal_action(already_asked: bool) -> SignalAction {
    if already_asked {
        SignalAction::ForceExit
    } else {
        SignalAction::AskToStop
    }
}

/// The two things a shutdown signal can mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SignalAction {
    /// Set the flag and let the loop wind down: the normal path, which writes
    /// the session on the way out.
    AskToStop,
    /// Leave now. The loop is not listening.
    ForceExit,
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
    #[test]
    fn a_second_signal_forces_the_exit_the_first_asked_for() {
        // One Ctrl-C asks; a second one leaves. Without the second, a loop that
        // has stopped reading the flag cannot be signalled out of at all — and
        // `submit` really can block there, inside `eglSwapBuffers`, when the
        // host stops presenting to the window.
        assert_eq!(signal_action(false), SignalAction::AskToStop);
        assert_eq!(signal_action(true), SignalAction::ForceExit);
    }

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
#[cfg(test)]
mod diagnostic_routing_tests {
    use super::same_file;
    use std::path::Path;

    #[test]
    fn a_file_matches_itself() {
        assert!(same_file(
            Path::new("/tmp/x/main.rs"),
            Path::new("/tmp/x/main.rs")
        ));
    }

    #[test]
    fn different_files_do_not_match() {
        assert!(!same_file(
            Path::new("/tmp/x/a.rs"),
            Path::new("/tmp/x/b.rs")
        ));
    }

    /// The case that loses diagnostics: opened through a symlink, reported
    /// canonicalised. Byte equality says no; the file is the same file.
    #[test]
    fn a_symlinked_path_matches_its_target() {
        let dir = std::env::temp_dir().join(format!("ruster-samefile-{}", std::process::id()));
        let real = dir.join("real");
        std::fs::create_dir_all(&real).unwrap();
        let file = real.join("main.rs");
        std::fs::write(&file, "fn main() {}").unwrap();
        let link = dir.join("link");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let opened = link.join("main.rs");
        assert_ne!(
            opened, file,
            "the two spellings must differ, or this proves nothing"
        );
        assert!(same_file(&opened, &file));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// `canonicalize` fails on a path that does not exist, and the fallback has
    /// to keep an unsaved or deleted buffer matching itself rather than
    /// dropping every diagnostic for it.
    #[test]
    fn a_nonexistent_path_still_matches_itself() {
        let p = Path::new("/tmp/does-not-exist-ruster/main.rs");
        assert!(same_file(p, p));
    }
}
