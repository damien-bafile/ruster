//! Lua control plane: the compositor's config (`compositor.lua`) is parsed
//! with a standalone `mlua` table parser rather than `ruster-lua`'s
//! editor-shaped runtime (which drives buffers, windows and LSP, none of which
//! a compositor config needs). Keeping this parser here also avoids coupling
//! the compositor to the editor crate's plugin model.

use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;

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

/// The parsed compositor config: keybinds as `(binding, action-name)` pairs,
/// the clients to launch on startup, and the workspace to start on.
#[derive(Debug, Clone, Default)]
pub struct LuaShell {
    pub keybinds: Vec<(String, String)>,
    pub startup_clients: Vec<String>,
    /// Workspace to start on, when the config asked for one.
    pub initial_workspace: Option<u32>,
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

/// Parse a compositor.lua source into a [`LuaShell`].
///
/// A config can declare itself as a table:
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
///
/// or call the `ruster.wm` API, which is what makes conditional configuration
/// possible — a table cannot branch on the machine it is being read on:
///
/// ```lua
/// ruster.wm.set_keybind("M-S-q", "quit")
/// ruster.wm.launch_client("foot")
/// ruster.wm.switch_workspace(2)
/// ```
///
/// Both may be used together; calls and table entries accumulate, in that
/// order. Nothing is applied while the config runs — the compositor does not
/// exist yet — so the calls record intent that [`apply_config_to_shell`] acts
/// on at startup.
pub fn parse_config(source: &str) -> mlua::Result<LuaShell> {
    let lua = Lua::new();
    let recorded = Rc::new(RefCell::new(LuaShell::default()));
    install_wm_api(&lua, &recorded)?;

    // Evaluate as a value, not a table: a config that only calls the API
    // returns nothing, and demanding a table would make it a parse error.
    let returned: mlua::Value = lua.load(source).eval()?;
    let mut shell = recorded.borrow().clone();
    if let mlua::Value::Table(table) = returned {
        merge_config_table(&table, &mut shell)?;
    }
    Ok(shell)
}

/// Fold a declarative config table into `shell`, after any `ruster.wm` calls.
fn merge_config_table(table: &mlua::Table, shell: &mut LuaShell) -> mlua::Result<()> {
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
    if let Ok(ws) = table.get::<u32>("workspace") {
        shell.initial_workspace = valid_workspace(ws);
    }
    Ok(())
}

/// A workspace number the shell will accept, or `None` for one it will not.
/// Out-of-range values are dropped with a warning rather than clamped: a config
/// asking for workspace 20 has a bug in it, and silently landing on 9 would
/// hide that.
fn valid_workspace(ws: u32) -> Option<u32> {
    if (1..=ruster_shell::WORKSPACE_COUNT).contains(&ws) {
        Some(ws)
    } else {
        tracing::warn!(
            workspace = ws,
            max = ruster_shell::WORKSPACE_COUNT,
            "config asked for a workspace that does not exist; ignoring"
        );
        None
    }
}

/// Install the `ruster.wm` table into the Lua globals.
///
/// Every function records into `shell` rather than acting: the config is read
/// before the compositor state exists, and even if it did, a config is a
/// declaration of what the session should look like rather than a script driving
/// a live session. Runtime control lands with the full keymap in Phase 1.
fn install_wm_api(lua: &Lua, shell: &Rc<RefCell<LuaShell>>) -> mlua::Result<()> {
    let wm = lua.create_table()?;

    let recorder = shell.clone();
    wm.set(
        "set_keybind",
        lua.create_function(move |_, (bind, action): (String, String)| {
            recorder.borrow_mut().keybinds.push((bind, action));
            Ok(())
        })?,
    )?;

    let recorder = shell.clone();
    wm.set(
        "launch_client",
        lua.create_function(move |_, command: String| {
            recorder.borrow_mut().startup_clients.push(command);
            Ok(())
        })?,
    )?;

    let recorder = shell.clone();
    wm.set(
        "switch_workspace",
        lua.create_function(move |_, workspace: u32| {
            if let Some(ws) = valid_workspace(workspace) {
                recorder.borrow_mut().initial_workspace = Some(ws);
            }
            Ok(())
        })?,
    )?;

    // `focus` is deliberately inert. Focus is a runtime operation against
    // windows that do not exist while the config is being read, and Phase 0 has
    // no addressing scheme for them — no directions, because there is no layout
    // until Phase 1, and no stable ids a user could write down. It exists so a
    // config calling it does not blow up, and says so.
    wm.set(
        "focus",
        lua.create_function(move |_, target: mlua::Value| {
            tracing::warn!(
                ?target,
                "ruster.wm.focus is not implemented in Phase 0 (no windows exist at config time); ignoring"
            );
            Ok(())
        })?,
    )?;

    let ruster = lua.create_table()?;
    ruster.set("wm", wm)?;
    lua.globals().set("ruster", ruster)?;
    Ok(())
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
    if let Some(workspace) = shell.initial_workspace {
        state.shell.workspace = workspace;
    }
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
    fn wm_api_records_binds_clients_and_workspace() {
        let shell = parse_config(
            r#"
            ruster.wm.set_keybind("M-S-q", "quit")
            ruster.wm.set_keybind("M-F9", "cycle workspace")
            ruster.wm.launch_client("foot")
            ruster.wm.switch_workspace(3)
            "#,
        )
        .unwrap();
        assert_eq!(
            shell.keybinds,
            vec![
                ("M-S-q".into(), "quit".into()),
                ("M-F9".into(), "cycle workspace".into())
            ]
        );
        assert_eq!(shell.startup_clients, vec!["foot".to_string()]);
        assert_eq!(shell.initial_workspace, Some(3));
    }

    #[test]
    fn a_config_that_only_calls_the_api_returns_nothing_and_still_parses() {
        // The declarative form returns a table; the imperative form returns
        // nil. Requiring a table would make every API-only config a parse error
        // and silently fall back to defaults.
        let shell = parse_config(r#"ruster.wm.launch_client("foot")"#).unwrap();
        assert_eq!(shell.startup_clients, vec!["foot".to_string()]);
    }

    #[test]
    fn api_calls_and_a_returned_table_accumulate() {
        let shell = parse_config(
            r#"
            ruster.wm.launch_client("foot")
            return { startup_clients = { "weston-terminal" } }
            "#,
        )
        .unwrap();
        assert_eq!(
            shell.startup_clients,
            vec!["foot".to_string(), "weston-terminal".to_string()]
        );
    }

    #[test]
    fn the_config_can_branch() {
        // The point of the API over a table: a table cannot decide anything.
        let shell = parse_config(
            r#"
            if os.getenv("DEFINITELY_NOT_SET_XYZ") then
              ruster.wm.launch_client("alacritty")
            else
              ruster.wm.launch_client("foot")
            end
            "#,
        )
        .unwrap();
        assert_eq!(shell.startup_clients, vec!["foot".to_string()]);
    }

    #[test]
    fn an_out_of_range_workspace_is_ignored_not_clamped() {
        let shell = parse_config("ruster.wm.switch_workspace(20)").unwrap();
        assert_eq!(shell.initial_workspace, None);
        let shell = parse_config("ruster.wm.switch_workspace(0)").unwrap();
        assert_eq!(shell.initial_workspace, None);
        let shell = parse_config("ruster.wm.switch_workspace(9)").unwrap();
        assert_eq!(shell.initial_workspace, Some(9));
    }

    #[test]
    fn focus_is_callable_but_inert() {
        // Phase 0 has no way to name a window, so `focus` records nothing. It
        // must still not raise, or a forward-looking config breaks the session.
        let shell = parse_config(r#"ruster.wm.focus("left")"#).unwrap();
        assert!(shell.keybinds.is_empty());
        assert!(shell.startup_clients.is_empty());
        assert_eq!(shell.initial_workspace, None);
    }

    #[test]
    fn the_shipped_default_config_parses() {
        // The embedded default is what every machine without a user config
        // runs, so a syntax error in it would ship as "no keybinds at all".
        let shell = parse_config(include_str!("../assets/compositor.lua")).unwrap();
        assert!(!shell.keybinds.is_empty(), "default config binds keys");
        assert!(
            !shell.startup_clients.is_empty(),
            "default config launches a client"
        );
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
