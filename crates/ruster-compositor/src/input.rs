//! Pure keyboard/modifier bindings for the compositor.
//!
//! These live apart from the seat wiring so they can be unit tested without a
//! live display. Phase 0 exposes only a quit binding and a workspace-cycle
//! binding; ruster's full keymap lands in Phase 1.

use smithay::input::keyboard::{Keysym, ModifiersState};

/// Global mod for WM binds: Mod4 (Super/Logo).
pub const WM_MOD: u32 = 4;

/// True when the given keysym+modifier state should quit the compositor:
/// Super+Shift+q.
pub fn is_quit_keysym(keysym: Keysym, mods: &ModifiersState) -> bool {
    keysym == Keysym::q && mods.logo && mods.shift
}

/// True when the given keysym+modifier should cycle active workspaces:
/// Super+Tab.
pub fn is_cycle_workspace(keysym: Keysym, mods: &ModifiersState) -> bool {
    keysym == Keysym::Tab && mods.logo
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
        let keysym = Keysym::Tab;
        let mods = ModifiersState {
            alt: false,
            ctrl: true,
            logo: true,
            shift: false,
            ..Default::default()
        };
        assert!(is_cycle_workspace(keysym, &mods));
    }
}
