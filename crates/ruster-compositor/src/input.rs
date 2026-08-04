//! Pure keyboard/modifier bindings for the compositor.
//!
//! These live apart from the seat wiring so they can be unit tested without a
//! live display. Phase 0 exposes a quit binding and a workspace-cycle binding
//! whose defaults are `M-S-q` (quit) and `M-t` (cycle workspace); the full
//! ruster keymap lands in Phase 1.

use smithay::input::keyboard::xkb::keysym_get_name;
use smithay::input::keyboard::{Keysym, ModifiersState};

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

#[cfg(test)]
mod tests {
    use super::*;

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
