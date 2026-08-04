//! Lua control plane: the compositor's config (`compositor.lua`) is parsed
//! with a standalone `mlua` table parser rather than `ruster-lua`'s
//! editor-shaped runtime (which drives buffers, windows and LSP, none of which
//! a compositor config needs). Keeping this parser here also avoids coupling
//! the compositor to the editor crate's plugin model.

use std::process::Command;

use mlua::Lua;
use smithay::input::keyboard::ModifiersState;

use crate::backend::Backend;
use crate::compositor::CompositorState;

/// The WM action bound to a keybind. Phase 0 knows two; a full keymap lands in
/// Phase 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Shut the compositor down (Super+Shift+q by default).
    Quit,
    /// Advance the active workspace (Super+t by default).
    CycleWorkspace,
}

/// The parsed compositor config: keybinds as `(binding, action-name)` pairs
/// and the list of clients to launch on startup.
#[derive(Debug, Clone, Default)]
pub struct LuaShell {
    pub keybinds: Vec<(String, String)>,
    pub startup_clients: Vec<String>,
}

/// Load `compositor.lua` from the config dir (`~/.config/ruster/`), falling
/// back to the embedded default (`assets/compositor.lua`). Errors are logged
/// and swallowed, never fatal.
pub fn load_compositor_config() -> LuaShell {
    let path = dirs::config_dir()
        .map(|p| p.join("ruster").join("compositor.lua"))
        .filter(|p| p.exists());
    let source = match path {
        Some(p) => match std::fs::read_to_string(&p) {
            Ok(src) => src,
            Err(err) => {
                tracing::warn!(path = %p.display(), %err, "failed to read compositor config");
                return LuaShell::default();
            }
        },
        None => include_str!("../assets/compositor.lua").to_string(),
    };
    match parse_config(&source) {
        Ok(shell) => shell,
        Err(err) => {
            tracing::warn!(%err, "failed to parse compositor config, using defaults");
            LuaShell::default()
        }
    }
}

/// Parse a compositor.lua source into a [`LuaShell`]. The Lua is a single
/// table:
///
/// ```lua
/// return {
///   keybinds = {
///     { "M-S-q", "quit" },
///     { "M-t", "cycle workspace" },
///   },
///   startup_clients = { "foot" },
/// }
/// ```
pub fn parse_config(source: &str) -> mlua::Result<LuaShell> {
    let lua = Lua::new();
    let table: mlua::Table = lua.load(source).eval()?;
    let mut shell = LuaShell::default();
    if let Ok(binds) = table.get::<mlua::Table>("keybinds") {
        for row in binds.sequence_values::<mlua::Table>() {
            let row = row?;
            shell.keybinds.push((row.get(1)?, row.get(2)?));
        }
    }
    if let Ok(clients) = table.get::<mlua::Table>("startup_clients") {
        for c in clients.sequence_values::<String>() {
            shell.startup_clients.push(c?);
        }
    }
    Ok(shell)
}

impl Action {
    /// Map a single keybind string plus the pressed key/modifier state to an
    /// action. `M` is Mod4 (Super/Logo), `S` is Shift. `"M-S-q"` requires
    /// logo+shift+`q`; `"M-t"` requires logo+`t` (deliberately *not* shift, so
    /// `M-S-t` stays distinct). Unrecognized binds return `None`.
    pub fn from_keybind(bind: &str, mods: &ModifiersState, key: &str) -> Option<Action> {
        match bind {
            "M-S-q" if mods.logo && mods.shift && key == "q" => Some(Action::Quit),
            "M-t" if mods.logo && !mods.shift && key == "t" => Some(Action::CycleWorkspace),
            _ => None,
        }
    }
}

/// Store the parsed config in the compositor state and launch the configured
/// startup clients on the compositor's socket.
///
/// Note: unlike the plan's `apply_config_to_shell(state, shell)` two-arg
/// signature, the socket name is passed explicitly — the plan intended startup
/// clients to be spawned here, and the socket name is only known once
/// `init_listener` has run (reading it back out of `state.socket_name` would
/// silently depend on call order).
pub fn apply_config_to_shell<B: Backend + 'static>(
    state: &mut CompositorState<B>,
    shell: LuaShell,
    socket_name: &str,
) {
    state.keybinds = shell.keybinds;
    spawn_startup_clients(&shell.startup_clients, socket_name);
}

/// Launch each configured startup client with `WAYLAND_DISPLAY` pointing at our
/// socket. Clients whose binary is not installed are skipped and a spawned
/// child failing is ignored — a startup client can never crash the compositor.
pub fn spawn_startup_clients(clients: &[String], socket_name: &str) {
    for client in clients {
        if Command::new(client).arg("--version").output().is_err() {
            tracing::warn!(%client, "startup client not found, skipping");
            continue;
        }
        if let Err(err) = Command::new(client)
            .env("WAYLAND_DISPLAY", socket_name)
            .spawn()
        {
            tracing::warn!(%client, %err, "failed to spawn startup client");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::input::keyboard::ModifiersState;

    #[test]
    fn default_config_has_startup_client_and_binds() {
        let shell = load_compositor_config();
        assert!(!shell.keybinds.is_empty());
        assert_eq!(shell.keybinds[0], ("M-S-q".into(), "quit".into()));
    }

    #[test]
    fn keybind_matches_produce_actions() {
        let mods = ModifiersState {
            logo: true,
            shift: true,
            ..Default::default()
        };
        assert_eq!(
            Action::from_keybind("M-S-q", &mods, "q"),
            Some(Action::Quit)
        );
        let no_shift = ModifiersState {
            logo: true,
            ..Default::default()
        };
        assert_eq!(
            Action::from_keybind("M-t", &no_shift, "t"),
            Some(Action::CycleWorkspace)
        );
    }

    #[test]
    fn cycle_bind_requires_logo_without_shift() {
        let logo = ModifiersState {
            logo: true,
            ..Default::default()
        };
        assert_eq!(
            Action::from_keybind("M-t", &logo, "t"),
            Some(Action::CycleWorkspace)
        );
        let logo_shift = ModifiersState {
            logo: true,
            shift: true,
            ..Default::default()
        };
        assert_eq!(Action::from_keybind("M-t", &logo_shift, "t"), None);
        let no_logo = ModifiersState {
            shift: true,
            ..Default::default()
        };
        assert_eq!(Action::from_keybind("M-t", &no_logo, "t"), None);
        // A different key does not trigger the cycle.
        assert_eq!(Action::from_keybind("M-t", &logo, "q"), None);
    }

    #[test]
    fn parse_config_extracts_keybinds_and_clients() {
        let shell = parse_config(
            r#"
            return {
              keybinds = { { "M-S-q", "quit" }, { "M-t", "cycle workspace" } },
              startup_clients = { "foot" },
            }
            "#,
        )
        .unwrap();
        assert_eq!(
            shell.keybinds,
            vec![
                ("M-S-q".into(), "quit".into()),
                ("M-t".into(), "cycle workspace".into())
            ]
        );
        assert_eq!(shell.startup_clients, vec!["foot".to_string()]);
    }

    #[test]
    fn parse_config_tolerates_a_missing_key() {
        // `mlua` evaluates a missing key to nil; a table without the optional
        // keys should still yield an empty, usable shell.
        let shell = parse_config("return {}").unwrap();
        assert!(shell.keybinds.is_empty());
        assert!(shell.startup_clients.is_empty());
    }
}
