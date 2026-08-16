//! What sits in a tree leaf when the leaf is somebody else's window.
//!
//! Two things can be: a Wayland `xdg_toplevel`, and — once XWayland is running —
//! an X11 window adopted through smithay's window manager. They answer the same
//! four questions the compositor ever asks (where is your surface, here is your
//! rectangle, you do/don't have the keyboard, are you still alive), and they
//! answer them completely differently underneath.
//!
//! An enum rather than a second `HashMap` beside `toplevels`, because the second
//! map is the version that compiles while being wrong: every site that resolves
//! an id would have to remember to consult both, and the failure when one forgets
//! is an X11 window that tiles but cannot be focused, or draws but cannot be
//! clicked. With one map of one enum the compiler finds the sites instead.

use smithay::reexports::wayland_server::protocol::wl_surface;
use smithay::wayland::shell::xdg::ToplevelSurface;
use smithay::xwayland::X11Surface;
use smithay::{
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel,
    utils::{Logical, Rectangle},
};

use ruster_shell::Rect;

/// A client window in a tree leaf.
#[derive(Debug, Clone, PartialEq)]
pub enum Client {
    /// A native Wayland `xdg_toplevel`.
    Wayland(ToplevelSurface),
    /// An X11 window, reached through XWayland's window manager.
    X11(X11Surface),
}

impl Client {
    /// The surface to draw and to send input to.
    ///
    /// `None` for an X11 window that has been created but not yet paired with
    /// its Wayland surface. That gap is real and it is not an error: X11 window
    /// creation and the `wl_surface` that carries its pixels arrive over two
    /// different sockets, and for a moment the compositor knows about the window
    /// and has nothing to draw.
    pub fn wl_surface(&self) -> Option<wl_surface::WlSurface> {
        match self {
            Client::Wayland(toplevel) => Some(toplevel.wl_surface().clone()),
            Client::X11(surface) => surface.wl_surface(),
        }
    }

    /// Give the window its rectangle.
    ///
    /// The Wayland half sends a size and lets the client place itself, because a
    /// toplevel has no say in where it is. The X11 half must send a *position*
    /// too: X11 windows carry absolute coordinates and an X client that is only
    /// told its size keeps drawing at whatever origin it last chose, which on a
    /// tiled desktop is wherever it happened to open.
    pub fn configure(&self, rect: Rect) {
        match self {
            Client::Wayland(toplevel) => {
                toplevel.with_pending_state(|state| {
                    state.size = Some((rect.w, rect.h).into());
                    // Tiled on every edge: the honest way to tell a client it
                    // does not own its borders, so it drops rounded corners and
                    // shadows.
                    state.states.set(xdg_toplevel::State::TiledLeft);
                    state.states.set(xdg_toplevel::State::TiledRight);
                    state.states.set(xdg_toplevel::State::TiledTop);
                    state.states.set(xdg_toplevel::State::TiledBottom);
                });
                toplevel.send_pending_configure();
            }
            Client::X11(surface) => {
                // Override-redirect windows are menus and tooltips that have
                // told the window manager to keep its hands off. Configuring one
                // is the WM overruling a client about its own popup, and the
                // usual result is a menu that snaps to the wrong corner.
                if surface.is_override_redirect() {
                    return;
                }
                let rect: Rectangle<i32, Logical> =
                    Rectangle::new((rect.x, rect.y).into(), (rect.w, rect.h).into());
                if let Err(err) = surface.configure(rect) {
                    tracing::warn!(?err, "could not configure an X11 window");
                }
            }
        }
    }

    /// Tell the window whether it holds the keyboard.
    ///
    /// A no-op on the Wayland side, where focus is the seat's business and the
    /// toplevel learns about it from `wl_keyboard.enter`. X11 has no such event:
    /// a window that is never told it is active draws itself greyed-out and
    /// unfocused while receiving every keystroke, which reads as a compositor
    /// sending input to the wrong place.
    pub fn set_activated(&self, active: bool) {
        if let Client::X11(surface) = self {
            if let Err(err) = surface.set_activated(active) {
                tracing::warn!(?err, "could not change an X11 window's activation");
            }
        }
    }

    /// Whether the window still exists.
    pub fn alive(&self) -> bool {
        match self {
            Client::Wayland(toplevel) => toplevel.alive(),
            Client::X11(surface) => surface.alive(),
        }
    }

    /// The Wayland toplevel, for the handful of places that are `xdg_shell`'s
    /// business alone — sending an initial configure, answering a decoration
    /// request. An X11 window has no `xdg_toplevel` and never will.
    pub fn toplevel(&self) -> Option<&ToplevelSurface> {
        match self {
            Client::Wayland(toplevel) => Some(toplevel),
            Client::X11(_) => None,
        }
    }
}

impl From<ToplevelSurface> for Client {
    fn from(toplevel: ToplevelSurface) -> Self {
        Client::Wayland(toplevel)
    }
}

impl From<X11Surface> for Client {
    fn from(surface: X11Surface) -> Self {
        Client::X11(surface)
    }
}
