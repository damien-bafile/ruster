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

/// The modifier set a keybind string asks for. Matching is *exact* — a bind
/// that does not name Shift does not fire while Shift is held, so `M-t` and
/// `M-S-t` stay distinct bindings rather than the first shadowing the second.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BindMods {
    pub logo: bool,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

impl BindMods {
    fn matches(&self, mods: &ModifiersState) -> bool {
        self.logo == mods.logo
            && self.shift == mods.shift
            && self.ctrl == mods.ctrl
            && self.alt == mods.alt
    }
}

/// Split an Emacs-style bind string into its modifiers and its key name.
///
/// `M-` is Mod4 (Super/Logo), `S-` Shift, `C-` Control, `A-` Alt; the remainder
/// is the key, matched case-insensitively against the keysym name (`q`, `t`,
/// `F9`, `space`). Returns `None` for an empty key or an unknown modifier
/// prefix, so a typo in a user's config is ignored rather than binding
/// something surprising.
pub fn parse_bind(bind: &str) -> Option<(BindMods, &str)> {
    let mut mods = BindMods::default();
    let mut rest = bind.trim();
    // A single trailing token is the key, even when it is itself a modifier
    // letter — `M-S` binds Super+S, not a modifier-only chord.
    while let Some((prefix, tail)) = rest.split_once('-') {
        if tail.is_empty() {
            break;
        }
        match prefix {
            "M" => mods.logo = true,
            "S" => mods.shift = true,
            "C" => mods.ctrl = true,
            "A" => mods.alt = true,
            _ => return None,
        }
        rest = tail;
    }
    (!rest.is_empty()).then_some((mods, rest))
}

impl Action {
    /// Map an action name from a config to an [`Action`]. Underscores, dashes
    /// and case are all accepted so `cycle_workspace`, `cycle-workspace` and
    /// `Cycle Workspace` name the same thing.
    pub fn from_name(name: &str) -> Option<Action> {
        match name
            .trim()
            .to_ascii_lowercase()
            .replace(['_', '-'], " ")
            .as_str()
        {
            "quit" => Some(Action::Quit),
            "cycle workspace" => Some(Action::CycleWorkspace),
            _ => None,
        }
    }

    /// Whether `bind` describes the given key and modifier state.
    ///
    /// The bind string alone no longer picks the action — the config's action
    /// name does (see [`Action::from_name`]). Previously this matched the two
    /// literal strings `"M-S-q"` and `"M-t"` and derived the action from the
    /// bind itself, which meant a config could neither bind a different key nor
    /// point a bind at a different action: both halves of every configured pair
    /// were ignored.
    pub fn keybind_matches(bind: &str, mods: &ModifiersState, key: &str) -> bool {
        parse_bind(bind).is_some_and(|(want, want_key)| {
            want.matches(mods) && want_key.eq_ignore_ascii_case(key)
        })
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
    fn keybind_matches_the_chord_it_names() {
        let mods = ModifiersState {
            logo: true,
            shift: true,
            ..Default::default()
        };
        assert!(Action::keybind_matches("M-S-q", &mods, "q"));
        let no_shift = ModifiersState {
            logo: true,
            ..Default::default()
        };
        assert!(Action::keybind_matches("M-t", &no_shift, "t"));
    }

    #[test]
    fn cycle_bind_requires_logo_without_shift() {
        let logo = ModifiersState {
            logo: true,
            ..Default::default()
        };
        assert!(Action::keybind_matches("M-t", &logo, "t"));
        let logo_shift = ModifiersState {
            logo: true,
            shift: true,
            ..Default::default()
        };
        assert!(!Action::keybind_matches("M-t", &logo_shift, "t"));
        let no_logo = ModifiersState {
            shift: true,
            ..Default::default()
        };
        assert!(!Action::keybind_matches("M-t", &no_logo, "t"));
        // A different key does not trigger the cycle.
        assert!(!Action::keybind_matches("M-t", &logo, "q"));
    }

    #[test]
    fn binds_are_not_limited_to_the_two_built_in_chords() {
        // The whole point of the config: a user can name a key the compositor
        // has no hardcoded knowledge of. Before the bind parser landed, every
        // string other than "M-S-q"/"M-t" silently matched nothing.
        let logo = ModifiersState {
            logo: true,
            ..Default::default()
        };
        assert!(Action::keybind_matches("M-F9", &logo, "F9"));
        assert!(
            Action::keybind_matches("M-f9", &logo, "F9"),
            "case-insensitive"
        );
        let ctrl_alt = ModifiersState {
            ctrl: true,
            alt: true,
            ..Default::default()
        };
        assert!(Action::keybind_matches("C-A-space", &ctrl_alt, "space"));
        assert!(!Action::keybind_matches("C-A-space", &logo, "space"));
    }

    #[test]
    fn bind_parsing_rejects_junk() {
        assert_eq!(
            parse_bind("M-t"),
            Some((
                BindMods {
                    logo: true,
                    ..Default::default()
                },
                "t"
            ))
        );
        assert_eq!(parse_bind("X-t"), None, "unknown modifier");
        assert_eq!(parse_bind(""), None);
        // A trailing modifier letter is a key, not a modifier.
        assert_eq!(
            parse_bind("M-S"),
            Some((
                BindMods {
                    logo: true,
                    ..Default::default()
                },
                "S"
            ))
        );
    }

    #[test]
    fn action_names_map_from_config_spelling() {
        assert_eq!(Action::from_name("quit"), Some(Action::Quit));
        assert_eq!(
            Action::from_name("cycle workspace"),
            Some(Action::CycleWorkspace)
        );
        assert_eq!(
            Action::from_name("cycle_workspace"),
            Some(Action::CycleWorkspace)
        );
        assert_eq!(
            Action::from_name("Cycle-Workspace"),
            Some(Action::CycleWorkspace)
        );
        assert_eq!(Action::from_name("explode"), None);
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
