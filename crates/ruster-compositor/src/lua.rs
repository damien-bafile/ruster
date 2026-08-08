//! Lua control plane: the compositor's config (`compositor.lua`) is parsed
//! with a standalone `mlua` table parser rather than `ruster-lua`'s
//! editor-shaped runtime (which drives buffers, windows and LSP, none of which
//! a compositor config needs). Keeping this parser here also avoids coupling
//! the compositor to the editor crate's plugin model.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::process::Command;
use std::rc::Rc;

use mlua::Lua;
use smithay::input::keyboard::{ModifiersState, XkbConfig};

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
    /// Pin the shortcut helper open, or unpin it.
    ToggleHelp,
    /// Open the `:` prompt (or `=` for Lua).
    Prompt(crate::minibuffer::Prompt),
    /// Write the composited output to a PNG.
    ///
    /// The compositor implements no screencopy protocol, so on a real boot
    /// nothing outside it can see the screen. This is how a DRM session
    /// produces evidence instead of a description.
    Screenshot,
}

/// The keyboard layout and repeat behaviour a config asked for.
///
/// Empty strings mean "whatever the system says", which is what libxkbcommon
/// falls back to — and it honours `XKB_DEFAULT_LAYOUT` and friends, so an
/// unconfigured compositor already matches the rest of the session. The point of
/// this struct is to let a config *override* that, which until now was
/// impossible: the keymap was `XkbConfig::default()` with a `TODO` beside it,
/// so every non-US layout was simply wrong and there was nowhere to say so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardConfig {
    pub layout: String,
    pub variant: String,
    pub model: String,
    pub rules: String,
    /// xkb options such as `ctrl:nocaps`, comma separated.
    pub options: Option<String>,
    /// Milliseconds held before a key starts repeating.
    pub repeat_delay: i32,
    /// Repeats per second once it starts.
    pub repeat_rate: i32,
}

impl Default for KeyboardConfig {
    fn default() -> Self {
        // The delay/rate pair the seat was built with before this was
        // configurable, so an existing session feels identical.
        KeyboardConfig {
            layout: String::new(),
            variant: String::new(),
            model: String::new(),
            rules: String::new(),
            options: None,
            repeat_delay: 200,
            repeat_rate: 25,
        }
    }
}

/// The parsed compositor config: keybinds as `(binding, action-name)` pairs,
/// the clients to launch on startup, and the workspace to start on.
#[derive(Debug, Clone, Default)]
pub struct LuaShell {
    pub keybinds: Vec<(String, String)>,
    pub startup_clients: Vec<String>,
    /// Workspace to start on, when the config asked for one.
    pub initial_workspace: Option<u32>,
    pub keyboard: KeyboardConfig,
}

/// Load `compositor.lua` from the config dir (`~/.config/ruster/`), falling
/// back to the embedded default (`assets/compositor.lua`). Errors are logged
/// and swallowed, never fatal.
pub fn load_compositor_config() -> (Option<WmControl>, LuaShell) {
    let path = dirs::config_dir()
        .map(|p| p.join("ruster").join("compositor.lua"))
        .filter(|p| p.exists());
    let source = match path {
        Some(p) => match std::fs::read_to_string(&p) {
            Ok(src) => src,
            Err(err) => {
                tracing::warn!(path = %p.display(), %err, "failed to read compositor config");
                return (None, LuaShell::default());
            }
        },
        None => include_str!("../assets/compositor.lua").to_string(),
    };
    match WmControl::from_source(&source) {
        Ok((control, shell)) => (Some(control), shell),
        Err(err) => {
            // No control plane rather than a half-built one: a config that
            // failed to run may have registered some of its calls and not
            // others, and a WM obeying half a config is worse than one obeying
            // none of it.
            tracing::warn!(%err, "failed to parse compositor config, using defaults");
            (None, LuaShell::default())
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
    WmControl::from_source(source).map(|(_, shell)| shell)
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
    if let Ok(kb) = table.get::<mlua::Table>("keyboard") {
        merge_keyboard_table(&kb, &mut shell.keyboard);
    }
    Ok(())
}

/// Fold a `keyboard = { ... }` table into `keyboard`, leaving anything the
/// config did not mention alone.
fn merge_keyboard_table(table: &mlua::Table, keyboard: &mut KeyboardConfig) {
    if let Ok(v) = table.get::<String>("layout") {
        keyboard.layout = v;
    }
    if let Ok(v) = table.get::<String>("variant") {
        keyboard.variant = v;
    }
    if let Ok(v) = table.get::<String>("model") {
        keyboard.model = v;
    }
    if let Ok(v) = table.get::<String>("rules") {
        keyboard.rules = v;
    }
    if let Ok(v) = table.get::<String>("options") {
        // An empty string means "no options", not "an option named nothing" —
        // xkb rejects the latter and would take the whole keymap down with it.
        keyboard.options = (!v.is_empty()).then_some(v);
    }
    if let Ok(v) = table.get::<i32>("repeat_delay") {
        keyboard.repeat_delay = v;
    }
    if let Ok(v) = table.get::<i32>("repeat_rate") {
        keyboard.repeat_rate = v;
    }
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
/// What the compositor last published about itself, for Lua to read.
///
/// A snapshot rather than a live borrow: [`CompositorState`] is generic over its
/// backend, so a Lua closure cannot hold one — and even if it could, handing the
/// live state to a script that runs mid-frame invites a `RefCell` panic at the
/// worst possible moment. The compositor refreshes this once per event-loop
/// iteration, so a query answers with the previous frame at the oldest.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WmStatus {
    pub workspace: u32,
    /// Windows on the active workspace.
    pub windows: usize,
    pub focused_title: String,
    /// Whether the focused window is floating.
    pub floating: bool,
    /// The split axis of the container holding focus, if any.
    pub layout: Option<String>,
}

/// The live control plane: a Lua VM that outlives config parsing, the queue its
/// calls push onto, and the status it can read back.
///
/// Before this, the VM was created inside the config parse and dropped when it
/// returned, so `ruster.wm.*` could only *record intent* — `focus` was a stub
/// that logged a warning and did nothing, because there was nothing for it to
/// act on. Keeping the VM alive is what makes the API an API.
///
/// Calls do not act directly: they push an [`Action`] onto a queue the event
/// loop drains into [`CompositorState::dispatch`]. That indirection buys two
/// things — the closures need no access to the generic compositor state, and
/// every route into the WM (keybind, Lua call, mini-buffer) ends up in the same
/// `dispatch`, so none of them can drift from the others.
pub struct WmControl {
    lua: Lua,
    queue: Rc<RefCell<VecDeque<Action>>>,
    status: Rc<RefCell<WmStatus>>,
}

impl WmControl {
    /// Build a control plane from config source, returning it and what the
    /// config declared.
    pub fn from_source(source: &str) -> mlua::Result<(Self, LuaShell)> {
        let lua = Lua::new();
        let recorded = Rc::new(RefCell::new(LuaShell::default()));
        let queue: Rc<RefCell<VecDeque<Action>>> = Rc::new(RefCell::new(VecDeque::new()));
        let status = Rc::new(RefCell::new(WmStatus::default()));
        install_wm_api(&lua, &recorded, &queue, &status)?;

        // Evaluate as a value, not a table: a config that only calls the API
        // returns nothing, and demanding a table would make it a parse error.
        let returned: mlua::Value = lua.load(source).eval()?;
        let mut shell = recorded.borrow().clone();
        if let mlua::Value::Table(table) = returned {
            merge_config_table(&table, &mut shell)?;
        }
        Ok((WmControl { lua, queue, status }, shell))
    }

    /// Everything queued since the last call, in the order it was queued.
    ///
    /// Returns owned actions rather than lending the queue, so the caller can
    /// take `&mut self` on the compositor to run them — which it must, since
    /// dispatching is the entire point.
    pub fn take_actions(&self) -> Vec<Action> {
        self.queue.borrow_mut().drain(..).collect()
    }

    /// Publish what the compositor currently looks like, for `ruster.wm.status`.
    pub fn publish(&self, status: WmStatus) {
        *self.status.borrow_mut() = status;
    }

    /// Run a chunk of Lua against the live VM.
    ///
    /// This is what makes a mini-buffer possible: the same VM the config ran in,
    /// so a command can call anything the config could.
    pub fn eval(&self, code: &str) -> Result<(), String> {
        self.lua.load(code).exec().map_err(|err| err.to_string())
    }
}

/// Install the `ruster.wm` table into the Lua globals.
///
/// Two kinds of function live here. The declarative ones (`set_keybind`,
/// `launch_client`) record into `shell` because they describe the session rather
/// than change it, and are meaningless once it is running. The rest queue an
/// [`Action`], which works identically whether the config is being read or a
/// keybind fired ten minutes later.
fn install_wm_api(
    lua: &Lua,
    shell: &Rc<RefCell<LuaShell>>,
    queue: &Rc<RefCell<VecDeque<Action>>>,
    status: &Rc<RefCell<WmStatus>>,
) -> mlua::Result<()> {
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

    // Records *and* queues. At startup the recorded value is applied and then
    // the queued action repeats it, which is idempotent; called at runtime the
    // recording is ignored and the queued action is the whole effect. One
    // implementation that means the same thing in both places.
    let recorder = shell.clone();
    let q = queue.clone();
    wm.set(
        "switch_workspace",
        lua.create_function(move |_, workspace: u32| {
            if let Some(ws) = valid_workspace(workspace) {
                recorder.borrow_mut().initial_workspace = Some(ws);
                q.borrow_mut().push_back(Action::Workspace(ws));
            }
            Ok(())
        })?,
    )?;

    // Every action by name, which is the whole `Action` vocabulary in one
    // function and stays correct as variants are added.
    let q = queue.clone();
    wm.set(
        "action",
        lua.create_function(move |_, name: String| match Action::from_name(&name) {
            Some(action) => {
                q.borrow_mut().push_back(action);
                Ok(true)
            }
            None => {
                tracing::warn!(%name, "ruster.wm.action: not an action");
                Ok(false)
            }
        })?,
    )?;

    // `focus` used to warn and do nothing, because the config ran before any
    // window existed and there was no way to name one. With a queue there is:
    // the call is carried out whenever the compositor next drains, by which time
    // there is a layout to move around in.
    let q = queue.clone();
    wm.set(
        "focus",
        lua.create_function(move |_, direction: String| {
            match direction_word(&direction).map(Action::Focus) {
                Some(action) => {
                    q.borrow_mut().push_back(action);
                    Ok(true)
                }
                None => {
                    tracing::warn!(%direction, "ruster.wm.focus: not a direction");
                    Ok(false)
                }
            }
        })?,
    )?;

    let q = queue.clone();
    wm.set(
        "spawn",
        lua.create_function(move |_, command: String| {
            let command = command.trim().to_string();
            if command.is_empty() {
                return Ok(false);
            }
            q.borrow_mut().push_back(Action::Spawn(command));
            Ok(true)
        })?,
    )?;

    let q = queue.clone();
    wm.set(
        "quit",
        lua.create_function(move |_, ()| {
            q.borrow_mut().push_back(Action::Quit);
            Ok(())
        })?,
    )?;

    let snapshot = status.clone();
    wm.set(
        "status",
        lua.create_function(move |lua, ()| {
            let s = snapshot.borrow().clone();
            let t = lua.create_table()?;
            t.set("workspace", s.workspace)?;
            t.set("windows", s.windows)?;
            t.set("title", s.focused_title)?;
            t.set("floating", s.floating)?;
            t.set("layout", s.layout)?;
            Ok(t)
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
fn direction_word(word: &str) -> Option<Direction> {
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
            ("toggle help" | "help" | "toggle whichkey", _, _) => Some(Action::ToggleHelp),
            ("command" | "prompt", _, _) => {
                Some(Action::Prompt(crate::minibuffer::Prompt::Command))
            }
            ("lua" | "eval", _, _) => Some(Action::Prompt(crate::minibuffer::Prompt::Lua)),
            ("toggle floating" | "float", _, _) => Some(Action::ToggleFloating),
            (_, "focus", Some(d)) => direction_word(d).map(Action::Focus),
            (_, "swap", Some(d)) => direction_word(d).map(Action::Swap),
            (_, "resize", Some(d)) => direction_word(d).map(Action::Resize),
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
    control: Option<WmControl>,
    shell: LuaShell,
    socket_name: &str,
) {
    state.wm = control;
    apply_keyboard_config(state, &shell.keyboard);
    state.keymap = crate::keymap::Keymap::new(&shell.keybinds);
    if let Some(workspace) = shell.initial_workspace {
        state.switch_workspace(workspace);
    }
    spawn_startup_clients(&shell.startup_clients, socket_name);
}

/// Launch each configured startup client with `WAYLAND_DISPLAY` pointing at our
/// socket. Clients whose binary is not installed are skipped and a spawned
/// child failing is ignored — a startup client can never crash the compositor.
/// Load the configured keymap onto the seat, keeping the old one if xkb will
/// not have it.
///
/// The fallback is the whole point. `add_keyboard` is called with
/// `XkbConfig::default()` before any config is read, and a config naming a
/// layout that does not exist would otherwise take the session down — on DRM,
/// where the compositor is the display server, that means a black screen and no
/// keyboard with which to fix the file that caused it. A warning and the
/// previous keymap is always the better trade.
pub fn apply_keyboard_config<B: Backend + 'static>(
    state: &mut CompositorState<B>,
    keyboard: &KeyboardConfig,
) {
    let keymap = XkbConfig {
        rules: &keyboard.rules,
        model: &keyboard.model,
        layout: &keyboard.layout,
        variant: &keyboard.variant,
        options: keyboard.options.clone(),
    };
    let handle = state.keyboard.clone();
    if *keyboard != KeyboardConfig::default() {
        match handle.set_xkb_config(state, keymap) {
            Ok(()) => tracing::info!(
                layout = %keyboard.layout,
                variant = %keyboard.variant,
                options = ?keyboard.options,
                "keymap loaded"
            ),
            Err(err) => tracing::warn!(
                layout = %keyboard.layout,
                variant = %keyboard.variant,
                ?err,
                "the configured keymap was rejected; keeping the current one"
            ),
        }
    }
    handle.change_repeat_info(keyboard.repeat_rate, keyboard.repeat_delay);
}

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
        let (_, shell) = load_compositor_config();
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
    fn a_config_can_name_its_keyboard_layout() {
        // The keymap was `XkbConfig::default()` with a TODO beside it, so every
        // non-US layout was simply wrong and there was nowhere to say otherwise.
        let shell = parse_config(
            r#"return { keyboard = {
                 layout = "gb", variant = "colemak", options = "ctrl:nocaps",
                 repeat_delay = 300, repeat_rate = 40,
               } }"#,
        )
        .unwrap();
        assert_eq!(shell.keyboard.layout, "gb");
        assert_eq!(shell.keyboard.variant, "colemak");
        assert_eq!(shell.keyboard.options.as_deref(), Some("ctrl:nocaps"));
        assert_eq!(shell.keyboard.repeat_delay, 300);
        assert_eq!(shell.keyboard.repeat_rate, 40);
    }

    #[test]
    fn an_unmentioned_keyboard_field_keeps_its_default() {
        // A config setting only the layout must not silently reset the repeat
        // rate to zero, which would be a keyboard that never repeats.
        let shell = parse_config(r#"return { keyboard = { layout = "de" } }"#).unwrap();
        let default = KeyboardConfig::default();
        assert_eq!(shell.keyboard.layout, "de");
        assert_eq!(shell.keyboard.repeat_delay, default.repeat_delay);
        assert_eq!(shell.keyboard.repeat_rate, default.repeat_rate);
        assert_eq!(shell.keyboard.variant, "");
    }

    #[test]
    fn no_keyboard_table_means_the_system_keymap() {
        // Empty strings are what libxkbcommon reads as "use the system
        // default", which honours XKB_DEFAULT_LAYOUT — so an unconfigured
        // compositor matches the rest of the session rather than forcing US.
        let shell = parse_config("return {}").unwrap();
        assert_eq!(shell.keyboard, KeyboardConfig::default());
        assert!(shell.keyboard.layout.is_empty());
        assert!(shell.keyboard.options.is_none());
    }

    #[test]
    fn empty_options_are_dropped_rather_than_passed_to_xkb() {
        // `options = ""` is an empty option list, not an option named nothing;
        // xkb rejects the latter and takes the whole keymap with it.
        let shell = parse_config(r#"return { keyboard = { options = "" } }"#).unwrap();
        assert!(shell.keyboard.options.is_none());
    }

    #[test]
    fn focus_is_a_real_call_now_rather_than_a_warning() {
        // It used to log "not implemented" and drop the call, because the VM
        // died with the config parse and there was nothing to act on.
        let (wm, _) = WmControl::from_source(r#"ruster.wm.focus("left")"#).unwrap();
        assert_eq!(wm.take_actions(), vec![Action::Focus(Direction::Left)]);
    }

    #[test]
    fn queued_actions_come_back_in_the_order_they_were_made() {
        // Order is the whole contract: `focus left` then `swap right` is not
        // the same session as the reverse.
        let (wm, _) = WmControl::from_source(
            r#"
            ruster.wm.focus("right")
            ruster.wm.action("swap left")
            ruster.wm.spawn("foot -e htop")
            ruster.wm.quit()
            "#,
        )
        .unwrap();
        assert_eq!(
            wm.take_actions(),
            vec![
                Action::Focus(Direction::Right),
                Action::Swap(Direction::Left),
                Action::Spawn("foot -e htop".into()),
                Action::Quit,
            ]
        );
    }

    #[test]
    fn draining_the_queue_empties_it() {
        // The event loop drains every iteration; a queue that kept its contents
        // would replay the whole session's actions on every frame.
        let (wm, _) = WmControl::from_source(r#"ruster.wm.quit()"#).unwrap();
        assert_eq!(wm.take_actions().len(), 1);
        assert!(wm.take_actions().is_empty());
    }

    #[test]
    fn a_bad_call_is_reported_to_the_script_and_queues_nothing() {
        // Returning false rather than raising: a config that mistypes one
        // direction should not abort the rest of the config, but it must not
        // silently look like it worked either.
        let (wm, _) = WmControl::from_source(
            r#"
            ok_dir = ruster.wm.focus("sideways")
            ok_act = ruster.wm.action("frobnicate")
            ok_spawn = ruster.wm.spawn("   ")
            "#,
        )
        .unwrap();
        assert!(wm.take_actions().is_empty());
        for name in ["ok_dir", "ok_act", "ok_spawn"] {
            let v: bool = wm.lua.globals().get(name).unwrap();
            assert!(!v, "{name} should have reported failure");
        }
    }

    #[test]
    fn status_reports_what_the_compositor_last_published() {
        // The query side cannot borrow the compositor — it is generic over its
        // backend — so it reads a snapshot the event loop refreshes.
        let (wm, _) = WmControl::from_source("").unwrap();
        wm.publish(WmStatus {
            workspace: 4,
            windows: 3,
            focused_title: "foot".into(),
            floating: true,
            layout: Some("vertical".into()),
        });
        wm.eval(
            r#"
            local s = ruster.wm.status()
            assert(s.workspace == 4, "workspace")
            assert(s.windows == 3, "windows")
            assert(s.title == "foot", "title")
            assert(s.floating == true, "floating")
            assert(s.layout == "vertical", "layout")
            "#,
        )
        .unwrap();
    }

    #[test]
    fn switching_workspace_works_from_a_config_and_at_runtime() {
        // One function for both: the recorded value is what startup applies,
        // and the queued action is what a runtime call does. Applying both at
        // boot lands on the same workspace twice, which is the same workspace.
        let (wm, shell) = WmControl::from_source("ruster.wm.switch_workspace(4)").unwrap();
        assert_eq!(shell.initial_workspace, Some(4));
        assert_eq!(wm.take_actions(), vec![Action::Workspace(4)]);
    }

    #[test]
    fn the_minibuffer_can_run_anything_the_config_could() {
        // `eval` is the same VM the config ran in, which is what makes a `:`
        // line worth having rather than a second, smaller command language.
        let (wm, _) = WmControl::from_source("").unwrap();
        wm.eval(r#"ruster.wm.action("workspace 7")"#).unwrap();
        assert_eq!(wm.take_actions(), vec![Action::Workspace(7)]);
        assert!(wm.eval("this is not lua").is_err());
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
