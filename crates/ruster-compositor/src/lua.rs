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
use ruster_shell::{Direction, Layout};

/// The WM action bound to a keybind.
///
/// Several carry an argument, which is what lets one action name cover a family
/// — `focus left` and `focus right` are the same operation pointed differently,
/// and spelling them as separate variants would mean four of everything.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Shut the compositor down (Super+Shift+q by default).
    Quit,
    /// Advance the active workspace (Super+t by default).
    CycleWorkspace,
    /// Move keyboard focus to the window drawn in that direction.
    Focus(Direction),
    /// Exchange the focused window with its neighbour that way.
    Swap(Direction),
    /// Move the boundary between the focused window and its neighbour.
    Resize(Direction),
    /// Re-divide the container holding the focused window along this axis,
    /// restacking what is already in it — and, as a consequence, deciding the
    /// axis the next window inserted here arrives on.
    Split(Layout),
    /// Float the focused window, or return it to the tiling.
    ToggleFloating,
    /// Show a numbered workspace.
    Workspace(u32),
    /// Send the focused window to a numbered workspace.
    MoveToWorkspace(u32),
    /// Launch a program on the compositor's own Wayland socket.
    ///
    /// Without this there is no way to open a window from inside the session at
    /// all: nested you can launch one from the host, but on DRM the compositor
    /// *is* the display server, so the only windows that ever exist are the
    /// startup clients — and the only way to get a second one is to type into
    /// the first, which assumes the first is a terminal.
    Spawn(String),
    /// Write the composited output to a PNG.
    ///
    /// The compositor implements no screencopy protocol, so on a real boot
    /// nothing outside it can see the screen. This is how a DRM session
    /// produces evidence instead of a description.
    Screenshot,
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

/// `text` with a leading `verb` removed, if it starts with that word.
///
/// Matched case-insensitively on the verb alone so `Spawn foot` works, while
/// everything after it is returned exactly as written.
fn strip_verb<'a>(text: &'a str, verb: &str) -> Option<&'a str> {
    let rest = text
        .get(..verb.len())?
        .eq_ignore_ascii_case(verb)
        .then(|| &text[verb.len()..])?;
    match rest.strip_prefix(|c: char| c.is_ascii_whitespace()) {
        Some(rest) => Some(rest.trim()),
        None => None,
    }
}

/// A compass word as a [`Direction`], or `None` for anything else.
fn direction(word: &str) -> Option<Direction> {
    match word {
        "left" | "l" => Some(Direction::Left),
        "right" | "r" => Some(Direction::Right),
        "up" | "u" => Some(Direction::Up),
        "down" | "d" => Some(Direction::Down),
        _ => None,
    }
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
        // Before normalising: a command line is not a keyword, and lowercasing
        // it or turning its dashes into spaces would break `foot -e htop` and
        // every program with a `-` or `_` in its name.
        let trimmed = name.trim();
        if let Some(rest) = strip_verb(trimmed, "spawn") {
            return (!rest.is_empty()).then(|| Action::Spawn(rest.to_string()));
        }
        let name = trimmed.to_ascii_lowercase().replace(['_', '-'], " ");
        // Several actions take an argument, and it is always the last word, so
        // the verb is everything before it. Keeping the whole action in one
        // string is what lets the config stay `(binding, action)` pairs rather
        // than growing a third column.
        let (verb, arg) = match name.rsplit_once(' ') {
            Some((verb, arg)) => (verb, Some(arg)),
            None => (name.as_str(), None),
        };
        match (name.as_str(), verb, arg) {
            ("quit", _, _) => Some(Action::Quit),
            ("cycle workspace", _, _) => Some(Action::CycleWorkspace),
            ("screenshot", _, _) => Some(Action::Screenshot),
            ("toggle floating" | "float", _, _) => Some(Action::ToggleFloating),
            (_, "focus", Some(d)) => direction(d).map(Action::Focus),
            (_, "swap", Some(d)) => direction(d).map(Action::Swap),
            (_, "resize", Some(d)) => direction(d).map(Action::Resize),
            (_, "split", Some("horizontal" | "h")) => Some(Action::Split(Layout::Horizontal)),
            (_, "split", Some("vertical" | "v")) => Some(Action::Split(Layout::Vertical)),
            (_, "workspace", Some(n)) => n
                .parse()
                .ok()
                .and_then(valid_workspace)
                .map(Action::Workspace),
            (_, "move to workspace", Some(n)) => n
                .parse()
                .ok()
                .and_then(valid_workspace)
                .map(Action::MoveToWorkspace),
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
        state.switch_workspace(workspace);
    }
    spawn_startup_clients(&shell.startup_clients, socket_name);
}

/// Launch each configured startup client with `WAYLAND_DISPLAY` pointing at our
/// socket. Clients whose binary is not installed are skipped and a spawned
/// child failing is ignored — a startup client can never crash the compositor.
/// Launch `command` on the compositor's own Wayland socket.
///
/// Split on whitespace, so `foot -e htop` works but quoting does not — a config
/// needing a shell can spawn one (`sh -c ...`) rather than have this grow a
/// parser it would get subtly wrong.
///
/// Children are not reaped, so a spawned program that exits leaves a zombie
/// until the compositor does. Startup clients have always behaved this way; a
/// keybind makes it reachable more often, but the fix is a SIGCHLD handler on
/// the event loop rather than anything here.
pub fn spawn_command(command: &str, socket_name: Option<&str>) {
    let Some(mut cmd) = build_command(command, socket_name) else {
        return;
    };
    match cmd.spawn() {
        Ok(child) => tracing::info!(%command, pid = child.id(), "spawned"),
        Err(err) => tracing::warn!(%command, %err, "failed to spawn"),
    }
}

/// The [`Command`] a spawn action describes, or `None` if it names no program.
///
/// Split out from the spawning so the part that can be silently wrong — which
/// program, which arguments, and above all which display — is testable without
/// running anything.
fn build_command(command: &str, socket_name: Option<&str>) -> Option<Command> {
    let mut parts = command.split_whitespace();
    let program = parts.next()?;
    let mut cmd = Command::new(program);
    cmd.args(parts);
    // Without this the child inherits the *host* socket when nested, and
    // connects to the wrong compositor — its window opens outside the session
    // that spawned it, which looks exactly like the spawn silently failing.
    if let Some(socket) = socket_name {
        cmd.env("WAYLAND_DISPLAY", socket);
    }
    Some(cmd)
}

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
    fn every_binding_the_shipped_config_declares_actually_works() {
        // A binding whose action name does not parse, or whose chord does not,
        // is silently dead: the key does nothing and nothing is logged. That is
        // the same defect `advertised_commands_exist` exists to catch on the
        // editor side, and with ~40 binds now shipped a typo is likely.
        let shell = parse_config(include_str!("../assets/compositor.lua"))
            .expect("the shipped config must parse");
        assert!(!shell.keybinds.is_empty());
        for (bind, action) in &shell.keybinds {
            assert!(
                Action::from_name(action).is_some(),
                "config binds {bind:?} to {action:?}, which is not an action"
            );
            assert!(
                parse_bind(bind).is_some(),
                "config binding {bind:?} is not a parseable chord"
            );
        }
    }

    #[test]
    fn no_two_shipped_bindings_claim_the_same_chord() {
        // First match wins in `resolve_wm_action`, so a duplicate chord makes
        // the later binding unreachable without saying so.
        let shell = parse_config(include_str!("../assets/compositor.lua")).unwrap();
        let mut seen = std::collections::HashSet::new();
        for (bind, _) in &shell.keybinds {
            let chord = bind.to_ascii_lowercase();
            assert!(seen.insert(chord), "{bind:?} is bound twice");
        }
    }

    #[test]
    fn a_spawned_program_is_pointed_at_this_compositor() {
        // The failure this prevents is quiet and confusing: with no
        // WAYLAND_DISPLAY the child inherits the host's, connects to the outer
        // compositor, and its window opens *outside* the session that spawned
        // it — which looks exactly like the keybind doing nothing.
        let cmd = build_command("foot -e htop", Some("wayland-7")).unwrap();
        assert_eq!(cmd.get_program(), "foot");
        let args: Vec<_> = cmd.get_args().collect();
        assert_eq!(args, ["-e", "htop"]);
        let display = cmd
            .get_envs()
            .find(|(k, _)| *k == "WAYLAND_DISPLAY")
            .and_then(|(_, v)| v);
        assert_eq!(display, Some("wayland-7".as_ref()));
    }

    #[test]
    fn a_spawn_with_no_socket_yet_still_runs() {
        // The socket name is an Option on the compositor, and refusing to spawn
        // without one would make the keybind dead rather than merely unpointed.
        let cmd = build_command("foot", None).unwrap();
        assert_eq!(cmd.get_program(), "foot");
        assert!(cmd.get_envs().all(|(k, _)| k != "WAYLAND_DISPLAY"));
        assert!(build_command("   ", Some("wayland-1")).is_none());
    }

    #[test]
    fn a_spawn_keeps_its_command_line_exactly_as_written() {
        // Everything else is normalised — lowercased, `_` and `-` turned into
        // spaces — which would turn `foot -e htop` into `foot  e htop` and
        // `Discord-canary` into something that is not a program.
        assert_eq!(
            Action::from_name("spawn foot"),
            Some(Action::Spawn("foot".into()))
        );
        assert_eq!(
            Action::from_name("spawn foot -e htop"),
            Some(Action::Spawn("foot -e htop".into()))
        );
        assert_eq!(
            Action::from_name("spawn Discord-canary"),
            Some(Action::Spawn("Discord-canary".into()))
        );
        assert_eq!(
            Action::from_name("  spawn   my_app  "),
            Some(Action::Spawn("my_app".into()))
        );
    }

    #[test]
    fn spawn_needs_something_to_spawn() {
        // Binding a key to a spawn with no command should bind nothing, rather
        // than a key that runs the empty string every time it is pressed.
        assert_eq!(Action::from_name("spawn"), None);
        assert_eq!(Action::from_name("spawn   "), None);
        // And a word that merely starts with "spawn" is not the spawn verb.
        assert_eq!(Action::from_name("spawnfoot"), None);
    }

    #[test]
    fn actions_take_their_argument_from_the_last_word() {
        assert_eq!(
            Action::from_name("focus left"),
            Some(Action::Focus(Direction::Left))
        );
        assert_eq!(
            Action::from_name("swap down"),
            Some(Action::Swap(Direction::Down))
        );
        assert_eq!(
            Action::from_name("resize right"),
            Some(Action::Resize(Direction::Right))
        );
        assert_eq!(
            Action::from_name("split vertical"),
            Some(Action::Split(Layout::Vertical))
        );
        assert_eq!(Action::from_name("workspace 3"), Some(Action::Workspace(3)));
        assert_eq!(
            Action::from_name("move to workspace 7"),
            Some(Action::MoveToWorkspace(7))
        );
        assert_eq!(
            Action::from_name("toggle floating"),
            Some(Action::ToggleFloating)
        );
    }

    #[test]
    fn argument_spellings_a_user_would_reach_for_all_work() {
        // Underscores and dashes normalise to spaces already, and the short
        // forms are what anyone who has used i3 will type first.
        assert_eq!(
            Action::from_name("move_to_workspace_2"),
            Some(Action::MoveToWorkspace(2))
        );
        assert_eq!(
            Action::from_name("Focus-Left"),
            Some(Action::Focus(Direction::Left))
        );
        assert_eq!(
            Action::from_name("focus l"),
            Some(Action::Focus(Direction::Left))
        );
        assert_eq!(
            Action::from_name("split h"),
            Some(Action::Split(Layout::Horizontal))
        );
        assert_eq!(Action::from_name("float"), Some(Action::ToggleFloating));
    }

    #[test]
    fn a_nonsense_argument_binds_nothing_rather_than_guessing() {
        // Binding `focus sideways` to *something* would be worse than binding
        // it to nothing: the key would work, just not as written.
        assert_eq!(Action::from_name("focus sideways"), None);
        assert_eq!(Action::from_name("focus"), None);
        assert_eq!(Action::from_name("split diagonal"), None);
        assert_eq!(Action::from_name("workspace twelve"), None);
    }

    #[test]
    fn a_workspace_argument_out_of_range_is_refused() {
        // Same reasoning as the config's `switch_workspace`: asking for
        // workspace 20 is a bug, and quietly landing on 9 hides it.
        assert_eq!(Action::from_name("workspace 0"), None);
        assert_eq!(Action::from_name("workspace 10"), None);
        assert_eq!(Action::from_name("move to workspace 99"), None);
        assert_eq!(Action::from_name("workspace 9"), Some(Action::Workspace(9)));
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
