//! Pure keyboard/modifier bindings for the compositor.
//!
//! These live apart from the seat wiring so they can be unit tested without a
//! live display. Phase 0 exposes a quit binding and a workspace-cycle binding
//! whose defaults are `M-S-q` (quit) and `M-t` (cycle workspace); the full
//! ruster keymap lands in Phase 1.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use smithay::input::keyboard::xkb::keysym_get_name;
use smithay::input::keyboard::{Keysym, ModifiersState};
use smithay::utils::{Logical, Point, Size};

use crate::lua::Action;

/// Global mod for WM binds: Mod4 (Super/Logo).
pub const WM_MOD: u32 = 4;

/// True when the given keysym+modifier state should quit the compositor:
/// Super+Shift+q.
pub fn is_quit_keysym(keysym: Keysym, mods: &ModifiersState) -> bool {
    keysym == Keysym::q && mods.logo && mods.shift
}

/// True when the given keysym+modifier should cycle active workspaces:
/// Super+t.
pub fn is_cycle_workspace(keysym: Keysym, mods: &ModifiersState) -> bool {
    keysym == Keysym::t && mods.logo && !mods.shift
}

/// Resolve the WM action bound to a key press: stringify the (raw, unshifted)
/// keysym, match it against the compositor's configured keybinds, and fall
/// back to the hardcoded Phase 0 defaults if nothing configured matched.
/// Pure, so it stays unit-testable without a live display.
///
/// The `keysym` is expected to be the *unmodified* level-0 keysym: modifier
/// state arrives separately in `mods`, so `Super+Shift+q` yields the raw `q`
/// keysym plus `shift` in `mods` rather than an uppercased `Q`.
pub fn resolve_wm_action(
    keybinds: &[(String, String)],
    mods: &ModifiersState,
    keysym: Keysym,
) -> Option<Action> {
    let key = keysym_get_name(keysym);
    keybinds
        .iter()
        .find_map(|(bind, _)| Action::from_keybind(bind, mods, &key))
        .or_else(|| {
            if is_quit_keysym(keysym, mods) {
                Some(Action::Quit)
            } else if is_cycle_workspace(keysym, mods) {
                Some(Action::CycleWorkspace)
            } else {
                None
            }
        })
}

/// Global position of a toplevel's frame. Phase 0 is fullscreen: every
/// toplevel covers the whole output from the origin, so no frame offset.
pub const TOPLEVEL_OFFSET: Point<f64, Logical> = Point::new(0.0, 0.0);

/// The pointer focus for the focused fullscreen toplevel: `Some(origin)`,
/// where `origin` is the toplevel's origin in *global* coordinates, when the
/// pointer lies within the toplevel's logical bounds, and `None` otherwise.
///
/// smithay's `PointerInnerHandle::motion` derives the client-visible position
/// as `event.location - origin`, so the focus tuple's second element must be
/// the surface origin — handing it the local pointer position would report
/// every enter/motion at `(0,0)`. Phase 0 draws the toplevel at the origin,
/// so the origin is [`TOPLEVEL_OFFSET`] regardless of where the pointer is.
pub fn pointer_focus(
    toplevel_size: Size<i32, Logical>,
    pointer: Point<f64, Logical>,
) -> Option<Point<f64, Logical>> {
    if pointer.x < 0.0
        || pointer.y < 0.0
        || pointer.x >= toplevel_size.w as f64
        || pointer.y >= toplevel_size.h as f64
    {
        None
    } else {
        Some(TOPLEVEL_OFFSET)
    }
}

/// Apply a resolved WM [`Action`] to the compositor lifecycle it owns. Only
/// the `running` flag lives here: `Quit` stops the compositor, while
/// `CycleWorkspace` needs the shell and renderer, so the caller dispatches it.
pub fn apply_action(action: &Action, running: &Arc<AtomicBool>) {
    match action {
        Action::Quit => running.store(false, Ordering::SeqCst),
        Action::CycleWorkspace => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wm_key_quit_sets_running_false() {
        // cannot construct CompositorState without a display; test the pure
        // decision instead: Action::Quit flips the running flag off.
        let running = Arc::new(AtomicBool::new(true));
        apply_action(&Action::Quit, &running);
        assert!(!running.load(Ordering::Relaxed));
    }

    #[test]
    fn wm_key_cycle_workspace_keeps_running() {
        // Cycling a workspace must not shut the compositor down.
        let running = Arc::new(AtomicBool::new(true));
        apply_action(&Action::CycleWorkspace, &running);
        assert!(running.load(Ordering::Relaxed));
    }

    #[test]
    fn pointer_focus_reports_the_surface_origin_not_the_local_position() {
        // smithay's PointerInnerHandle::motion sends `event.location - origin`
        // to the client, so the focus tuple's second element is the surface's
        // origin in *global* coordinates. A fullscreen toplevel at the origin
        // reports (0,0) no matter where the pointer is inside it.
        let size = Size::from((800, 600));
        assert_eq!(
            pointer_focus(size, Point::from((10.0, 20.0))),
            Some(TOPLEVEL_OFFSET)
        );
        assert_eq!(
            pointer_focus(size, Point::from((799.0, 599.0))),
            Some(TOPLEVEL_OFFSET)
        );
    }

    #[test]
    fn pointer_focus_outside_toplevel_is_none() {
        let size = Size::from((800, 600));
        // Outside on every edge.
        assert_eq!(pointer_focus(size, Point::from((-1.0, 0.0))), None);
        assert_eq!(pointer_focus(size, Point::from((800.0, 0.0))), None);
        assert_eq!(pointer_focus(size, Point::from((0.0, 600.0))), None);
        assert_eq!(pointer_focus(size, Point::from((0.0, 700.0))), None);
    }

    #[test]
    fn keysym_quit_binding_recognized() {
        // Mod4+Shift+q → quit
        let keysym = Keysym::q;
        let mods = ModifiersState {
            alt: false,
            ctrl: false,
            logo: true,
            shift: true,
            ..Default::default()
        };
        assert!(is_quit_keysym(keysym, &mods));
    }

    #[test]
    fn workspace_cycle_binding_recognized() {
        // Mod4+t → cycle workspace (no shift)
        let keysym = Keysym::t;
        let mods = ModifiersState {
            alt: false,
            ctrl: false,
            logo: true,
            shift: false,
            ..Default::default()
        };
        assert!(is_cycle_workspace(keysym, &mods));
    }

    #[test]
    fn workspace_cycle_binding_ignores_shift_and_non_logo() {
        assert!(!is_cycle_workspace(
            Keysym::t,
            &ModifiersState {
                logo: true,
                shift: true,
                ..Default::default()
            }
        ));
        assert!(!is_cycle_workspace(
            Keysym::t,
            &ModifiersState {
                logo: false,
                shift: false,
                ..Default::default()
            }
        ));
        assert!(!is_cycle_workspace(
            Keysym::q,
            &ModifiersState {
                logo: true,
                ..Default::default()
            }
        ));
    }

    #[test]
    fn resolve_quit_and_cycle_from_configured_keybinds() {
        let keybinds = vec![
            ("M-S-q".into(), "quit".into()),
            ("M-t".into(), "cycle workspace".into()),
        ];
        let mods = ModifiersState {
            logo: true,
            shift: true,
            ..Default::default()
        };
        assert_eq!(
            resolve_wm_action(&keybinds, &mods, Keysym::q),
            Some(Action::Quit)
        );
        let cycle_mods = ModifiersState {
            logo: true,
            ..Default::default()
        };
        assert_eq!(
            resolve_wm_action(&keybinds, &cycle_mods, Keysym::t),
            Some(Action::CycleWorkspace)
        );
    }

    #[test]
    fn resolve_falls_back_to_defaults_when_unbound() {
        let keybinds = vec![("M-S-q".into(), "quit".into())];
        // Super+Tab is no longer a WM bind (the cycle key is M-t now), so an
        // unconfigured press resolves to nothing.
        let tab_mods = ModifiersState {
            logo: true,
            ..Default::default()
        };
        assert_eq!(resolve_wm_action(&keybinds, &tab_mods, Keysym::Tab), None);
        assert_eq!(
            resolve_wm_action(&keybinds, &tab_mods, Keysym::NoSymbol),
            None
        );
    }
}
