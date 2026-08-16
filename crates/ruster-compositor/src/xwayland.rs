//! XWayland: X11 clients as ordinary tree leaves.
//!
//! Without this there is no Electron app, no Steam, and no legacy GTK2 — a large
//! fraction of what people actually run. XWayland is an X server that renders
//! into Wayland surfaces; smithay starts it and hands us an X11 connection, and
//! everything after that is a window manager's job. That job is this file.
//!
//! The compositor already knows how to tile, focus and draw a window. What it
//! did not know is that a window can arrive over a second protocol whose rules
//! are almost, but not quite, the same:
//!
//! - **X11 windows have absolute positions.** A Wayland toplevel is told a size
//!   and placed by the compositor. An X client places *itself*, so it must be
//!   told where its tile is or it draws at whatever origin it opened at. That is
//!   handled in [`Client::configure`](crate::client::Client::configure).
//! - **Mapping is a request, not a fact.** An X client asks the window manager
//!   for permission to appear, and stays invisible until granted. Forgetting to
//!   answer is a window that exists, has a buffer, and is never seen.
//! - **Override-redirect windows are not ours.** Menus, tooltips and drag icons
//!   set a flag meaning "the window manager keeps its hands off". They must be
//!   drawn, never tiled, and never configured.
//! - **Focus must be told, not inferred.** X11 has no `wl_keyboard.enter`; a
//!   window that is never activated draws itself greyed-out while receiving
//!   every keystroke.

use smithay::utils::{Logical, Rectangle, Size};
use smithay::wayland::xwayland_shell::{XWaylandShellHandler, XWaylandShellState};
use smithay::xwayland::xwm::{Reorder, XwmId};
use smithay::xwayland::{X11Surface, X11Wm, XwmHandler};

use crate::backend::Backend;
use crate::compositor::CompositorState;

impl<B: Backend + 'static> XWaylandShellHandler for CompositorState<B> {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }
}

impl<B: Backend + 'static> XwmHandler for CompositorState<B> {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        // Only ever one X server, so the id is not consulted. `expect` rather
        // than a silent default because every call here is made *by* the window
        // manager: if it is running, this is Some, and if it is not, none of
        // these callbacks can fire.
        self.xwm.as_mut().expect("an xwm callback without an xwm")
    }

    /// A window exists. It is not on screen and may never be — that is
    /// `map_window_request`'s decision.
    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    /// The client is asking to be shown.
    ///
    /// Granting it has to happen in this order: `set_mapped(true)` first, so the
    /// surface exists to be adopted, and only then insert it into the tree. The
    /// other order inserts a leaf whose surface is still `None`, which lays out
    /// a window that cannot be drawn.
    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Err(err) = window.set_mapped(true) {
            tracing::warn!(?err, "could not map an X11 window");
            return;
        }
        let title = window.title();
        let id = self.insert_client(window.into());
        tracing::info!(?id, title, "X11 window mapped");
    }

    /// A menu, tooltip or drag icon that has opted out of management.
    ///
    /// Deliberately *not* inserted into the tree: tiling a menu moves it away
    /// from the thing it belongs to. It is drawn where the client put it — see
    /// `render::override_redirect_elements`.
    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.x11_unmanaged.push(window);
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.x11_unmanaged.retain(|w| w != &window);
        if let Some(id) = self.window_for_x11(&window) {
            self.remove_client(id);
            tracing::info!(?id, "X11 window unmapped");
        }
    }

    fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.x11_unmanaged.retain(|w| w != &window);
        if let Some(id) = self.window_for_x11(&window) {
            self.remove_client(id);
        }
    }

    /// The client would like a different geometry.
    ///
    /// Answered with the tile it has, not the size it asked for — this is a
    /// tiling compositor, and a client that could resize itself would be able to
    /// overlap its neighbours. An unmanaged window gets what it asked for,
    /// because it is not in the layout to conflict with anything.
    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        if let Some(rect) = self
            .window_for_x11(&window)
            .and_then(|id| self.window_rect(id))
        {
            crate::client::Client::X11(window).configure(rect);
            return;
        }
        let current = window.geometry();
        let rect = Rectangle::new(
            (x.unwrap_or(current.loc.x), y.unwrap_or(current.loc.y)).into(),
            Size::<i32, Logical>::from((
                w.map(|v| v as i32).unwrap_or(current.size.w),
                h.map(|v| v as i32).unwrap_or(current.size.h),
            )),
        );
        if let Err(err) = window.configure(rect) {
            tracing::warn!(?err, "could not answer an X11 configure request");
        }
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        _window: X11Surface,
        _geometry: Rectangle<i32, Logical>,
        _above: Option<u32>,
    ) {
    }

    /// Refused, along with fullscreen below.
    ///
    /// A tiled window is already the size the layout says, and letting a client
    /// take the whole output on its own initiative is how a video player ends up
    /// covering a desktop the user did not ask it to. Silence would leave the
    /// client waiting; the reply says no.
    fn maximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        let _ = window.set_maximized(false);
    }

    fn unmaximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        let _ = window.set_maximized(false);
    }

    fn fullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        let _ = window.set_fullscreen(false);
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        let _ = window.set_fullscreen(false);
    }

    /// Interactive move and resize, refused by omission.
    ///
    /// The tree owns geometry; `resize`/`swap` keybinds are how it changes. A
    /// window that could drag its own edge would leave the layout and the screen
    /// disagreeing, which is the bug tiling exists to prevent.
    fn resize_request(
        &mut self,
        _xwm: XwmId,
        _window: X11Surface,
        _button: u32,
        _edges: smithay::xwayland::xwm::ResizeEdge,
    ) {
    }

    fn move_request(&mut self, _xwm: XwmId, _window: X11Surface, _button: u32) {}
}

/// Start XWayland and, when it says it is ready, take over as its window
/// manager.
///
/// Called from both backends' entry points rather than from `create_state`,
/// because `create_state` is also what the unit tests build a compositor with
/// and none of them want an X server. That means two call sites, which is the
/// shape that has already caused three bugs in this tree — so if you are adding
/// a third backend, this is the line you are looking for.
///
/// A machine with no `Xwayland` binary gets a warning and a compositor that
/// works in every other respect. That is the honest failure: X11 support is a
/// bonus on a Wayland compositor, not a precondition for booting one.
pub fn start<B: Backend + 'static>(
    dh: &smithay::reexports::wayland_server::DisplayHandle,
    handle: &smithay::reexports::calloop::LoopHandle<'static, CompositorState<B>>,
) {
    use smithay::xwayland::{XWayland, XWaylandEvent};

    let (xwayland, client) = match XWayland::spawn(
        dh,
        None,
        std::iter::empty::<(String, String)>(),
        true,
        std::process::Stdio::null(),
        std::process::Stdio::null(),
        |_| {},
    ) {
        Ok(pair) => pair,
        Err(err) => {
            tracing::warn!(?err, "no XWayland; X11 clients will not be able to connect");
            return;
        }
    };

    // Set now, not when XWayland reports ready. The display number is reserved
    // by `spawn` — the socket is created there — and readiness arrives several
    // hundred milliseconds later, by which time the configured startup clients
    // have already been launched. Setting it in the `Ready` arm meant an X11
    // program in `startup_clients` found no `DISPLAY` and exited, which on a DRM
    // boot is the only way to start anything.
    //
    // On the compositor's own environment so every child inherits it:
    // `spawn_command` builds a `Command`, which copies the parent's environment,
    // so this reaches startup clients and the launcher without either needing to
    // know X11 exists.
    std::env::set_var("DISPLAY", format!(":{}", xwayland.display_number()));

    let wm_handle = handle.clone();
    let inserted = handle.insert_source(xwayland, move |event, _, state| match event {
        XWaylandEvent::Ready {
            x11_socket,
            display_number,
        } => match X11Wm::start_wm(wm_handle.clone(), x11_socket, client.clone()) {
            Ok(wm) => {
                state.xwm = Some(wm);
                tracing::info!(display = display_number, "XWayland ready");
            }
            Err(err) => tracing::warn!(?err, "could not start the X11 window manager"),
        },
        XWaylandEvent::Error => {
            tracing::warn!("XWayland exited during startup; X11 clients cannot connect")
        }
    });
    if let Err(err) = inserted {
        tracing::warn!(?err, "could not register XWayland with the event loop");
    }
}
