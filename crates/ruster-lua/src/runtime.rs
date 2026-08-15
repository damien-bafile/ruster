use crate::config::Config;
use crate::event::EventBus;
use crate::keymap::LuaKeymap;
use mlua::{Function, Lua, RegistryKey};
use std::cell::RefCell;
use std::path::Path;

#[derive(Debug)]
pub enum LuaAction {
    Cmd(String),
    Print(String),
    Notify(u8, String), // 0=Info, 1=Success, 2=Warning, 3=Error
    /// Show a modal form. Fields are `(label, kind, value, options)` where kind
    /// is one of `toggle`/`text`/`number`/`select`; the app owns the widgets, so
    /// Lua describes the form rather than drawing it.
    Dialog {
        title: String,
        fields: Vec<(String, String, String, Vec<String>)>,
    },
}

/// Callbacks the app installs so Lua can query and manipulate buffers and
/// windows. Ids are the raw `u32` values of `BufferId`/`WindowId` as `i32`.
pub struct WindowCallbacks {
    pub list_bufs: Box<dyn FnMut() -> Vec<i32>>,
    pub list_wins: Box<dyn FnMut() -> Vec<i32>>,
    pub current_win: Box<dyn FnMut() -> i32>,
    pub set_current_win: Box<dyn FnMut(i32)>,
    pub win_get_buf: Box<dyn FnMut(i32) -> i32>,
    pub win_set_buf: Box<dyn FnMut(i32, i32)>,
    /// Split the active window; `true` = vertical. Returns the new window id.
    pub open_win: Box<dyn FnMut(bool) -> i32>,
    pub close_win: Box<dyn FnMut(i32)>,
}

/// A diagnostic as Lua sees it.
#[derive(Clone)]
pub struct LuaDiagnostic {
    pub line: i64,
    pub col: i64,
    /// 1=error, 2=warning, 3=info, 4=hint — the LSP numbering, unchanged.
    pub severity: u8,
    pub message: String,
}

/// Read-only queries the app installs so a plugin can *ask* rather than only
/// act.
///
/// Deliberately small. Every getter here is API surface that has to keep
/// working, so this is what a statusline or a lightweight plugin actually
/// needs and nothing beyond it — anything else can be added when something
/// concrete wants it.
pub struct QueryCallbacks {
    /// Absolute path of the active buffer, empty for a scratch buffer.
    pub buf_path: Box<dyn FnMut() -> String>,
    /// Language key of the active buffer (`rust`, `lua`, …), empty if unknown.
    pub filetype: Box<dyn FnMut() -> String>,
    /// Diagnostics for the active buffer.
    pub diagnostics: Box<dyn FnMut() -> Vec<LuaDiagnostic>>,
    /// `(branch, staged, unstaged)`. Branch is empty outside a repository.
    pub git_status: Box<dyn FnMut() -> (String, usize, usize)>,
}

/// Buffer/cursor bridge callbacks the app installs so Lua can read and edit the
/// active buffer. Boxed because their concrete closures live in the frontend.
type GetLinesFn = Box<dyn FnMut(i32, Option<i32>) -> Vec<String>>;
type SetLinesFn = Box<dyn FnMut(i32, i32, Vec<String>)>;
type GetCursorFn = Box<dyn FnMut() -> (i32, i32)>;
type SetCursorFn = Box<dyn FnMut(i32, i32)>;

/// State shared between the runtime and the Lua closures it installs.
///
/// Held behind an `Rc` by both sides. This exists because the previous design
/// handed each closure a `*const LuaRuntime` taken from a local in `new()` and
/// then *moved* the runtime out on return — leaving every installed `ruster.*`
/// function dereferencing freed memory. Sharing by `Rc` means the closures keep
/// the state alive rather than pointing at where it used to be.
///
/// `Lua` deliberately stays outside: closures already receive `&Lua` as their
/// first argument, and putting it here would make the runtime own a `Lua` that
/// owns closures that own the runtime.
pub(crate) struct Shared {
    pub(crate) keymaps: RefCell<Vec<LuaKeymap>>,
    pub(crate) pending: RefCell<Vec<LuaAction>>,
    pub(crate) events: RefCell<EventBus>,
    pub(crate) current_dt: RefCell<f64>,
    pub(crate) get_lines: RefCell<Option<GetLinesFn>>,
    pub(crate) set_lines: RefCell<Option<SetLinesFn>>,
    pub(crate) get_cursor: RefCell<Option<GetCursorFn>>,
    pub(crate) set_cursor: RefCell<Option<SetCursorFn>>,
    /// Lua-registered statusline sections, in registration order.
    pub(crate) statusline: RefCell<Vec<StatusSectionReg>>,
    /// Window/buffer manipulation callbacks installed by the app.
    pub(crate) window_cb: RefCell<Option<WindowCallbacks>>,
    /// Read-only queries; see [`QueryCallbacks`].
    pub(crate) query_cb: RefCell<Option<QueryCallbacks>>,
    /// `on_submit` for the dialog currently being shown. Held here rather than
    /// travelling in `LuaAction` so the registry key never leaves this crate.
    pub(crate) dialog_cb: RefCell<Option<RegistryKey>>,
    /// `ruster.defer` / `ruster.timer` callbacks, drained on the frame tick.
    pub(crate) timers: RefCell<crate::timer::Timers>,
    /// Context-menu items plugins added, as `(zone, label, command)`. Drained
    /// by the app into its own registry — this crate knows nothing about zones
    /// beyond their names.
    pub(crate) context_menu: RefCell<Vec<(String, String, String)>>,
}

/// One statusline section a plugin registered.
///
/// The name is what a click is reported under, so it has to survive from
/// registration to dispatch; the app never sees the registry keys, only the
/// name and the text.
pub(crate) struct StatusSectionReg {
    /// "left" | "center" | "right".
    pub(crate) pos: String,
    pub(crate) name: String,
    /// Called each frame for the section's text.
    pub(crate) render: RegistryKey,
    /// Called when the section is clicked, if the plugin asked for clicks.
    pub(crate) on_click: Option<RegistryKey>,
}

/// What a Lua mouse or hover handler receives.
///
/// A table, not positional arguments, so fields can be added later without
/// breaking handlers — and a plain Rust struct here so callers never touch
/// `mlua`.
#[derive(Debug, Clone, Default)]
pub struct MousePayload {
    /// One of `down`, `up`, `drag`, `move`, `wheel`.
    pub kind: String,
    pub col: u16,
    pub row: u16,
    /// One of `left`, `right`, `middle`, `none`.
    pub button: String,
    /// One of `buffer`, `gutter`, `chrome`, `float`, `outside`.
    pub zone: String,
    pub alt: bool,
    pub ctrl: bool,
    pub shift: bool,
    /// Set only over buffer text.
    pub offset: Option<usize>,
    pub window: Option<u32>,
    /// 0-indexed line, and column within it.
    pub line: Option<usize>,
    pub col_in_line: Option<usize>,
    /// Name of the statusline section under the pointer, when there is one.
    pub section: Option<String>,
}

impl MousePayload {
    /// The `ruster.on` event name this payload is delivered under.
    fn event_name(&self) -> &'static str {
        match self.kind.as_str() {
            "down" => "mouse_down",
            "up" => "mouse_up",
            "drag" => "mouse_drag",
            "move" => "mouse_move",
            _ => "mouse_wheel",
        }
    }

    fn to_table(&self, lua: &Lua) -> Option<mlua::Table> {
        let t = lua.create_table().ok()?;
        t.set("kind", self.kind.clone()).ok()?;
        t.set("col", self.col).ok()?;
        t.set("row", self.row).ok()?;
        t.set("button", self.button.clone()).ok()?;
        t.set("zone", self.zone.clone()).ok()?;
        t.set("alt", self.alt).ok()?;
        t.set("ctrl", self.ctrl).ok()?;
        t.set("shift", self.shift).ok()?;
        // Absent fields stay nil rather than becoming a misleading zero.
        if let Some(v) = self.offset {
            t.set("offset", v).ok()?;
        }
        if let Some(v) = self.window {
            t.set("window", v).ok()?;
        }
        if let Some(v) = self.line {
            t.set("line", v).ok()?;
        }
        if let Some(v) = self.col_in_line {
            t.set("col_in_line", v).ok()?;
        }
        if let Some(v) = &self.section {
            t.set("section", v.as_str()).ok()?;
        }
        Some(t)
    }
}

pub struct LuaRuntime {
    pub lua: Lua,
    pub(crate) shared: std::rc::Rc<Shared>,
}

impl LuaRuntime {
    pub fn new() -> mlua::Result<Self> {
        let lua = Lua::new();
        let shared = std::rc::Rc::new(Shared {
            keymaps: RefCell::new(Vec::new()),
            pending: RefCell::new(Vec::new()),
            events: RefCell::new(EventBus::new()),
            current_dt: RefCell::new(0.0),
            get_lines: RefCell::new(None),
            set_lines: RefCell::new(None),
            get_cursor: RefCell::new(None),
            set_cursor: RefCell::new(None),
            statusline: RefCell::new(Vec::new()),
            window_cb: RefCell::new(None),
            query_cb: RefCell::new(None),
            dialog_cb: RefCell::new(None),
            timers: RefCell::new(crate::timer::Timers::new()),
            context_menu: RefCell::new(Vec::new()),
        });

        // The closures capture a clone of `shared`, so moving the runtime out of
        // this function on the next line is now safe.
        let ruster = crate::api::create_table(&lua, &shared)?;
        lua.globals().set("ruster", ruster)?;
        Ok(LuaRuntime { lua, shared })
    }

    pub fn set_buffer_callbacks(
        &self,
        get_lines: Box<dyn FnMut(i32, Option<i32>) -> Vec<String>>,
        set_lines: Box<dyn FnMut(i32, i32, Vec<String>)>,
        get_cursor: Box<dyn FnMut() -> (i32, i32)>,
        set_cursor: Box<dyn FnMut(i32, i32)>,
    ) {
        self.shared.get_lines.replace(Some(get_lines));
        self.shared.set_lines.replace(Some(set_lines));
        self.shared.get_cursor.replace(Some(get_cursor));
        self.shared.set_cursor.replace(Some(set_cursor));
    }

    /// Install the window/buffer manipulation callbacks.
    pub fn set_window_callbacks(&self, cb: WindowCallbacks) {
        self.shared.window_cb.replace(Some(cb));
    }

    pub fn set_query_callbacks(&self, cb: QueryCallbacks) {
        *self.shared.query_cb.borrow_mut() = Some(cb);
    }

    /// Evaluate all Lua statusline sections registered for `pos`
    /// ("left" | "center" | "right"), returning each one's `(name, text)`.
    ///
    /// The name travels with the text because the bar is clickable: what lands
    /// on screen has to stay attributable to the section that produced it, or a
    /// click has nothing to route to.
    pub fn statusline_sections(&self, pos: &str) -> Vec<(String, String)> {
        let sections = self.shared.statusline.borrow();
        let mut out = Vec::new();
        for sec in sections.iter() {
            if sec.pos != pos {
                continue;
            }
            if let Ok(func) = self.lua.registry_value::<Function>(&sec.render) {
                if let Ok(s) = func.call::<String>(()) {
                    if !s.is_empty() {
                        out.push((sec.name.clone(), s));
                    }
                }
            }
        }
        out
    }

    /// Deliver a click on the statusline section called `name`.
    ///
    /// Returns whether a handler ran, so the caller can fall back to the
    /// built-in behaviour for a section that has none — an inert section must
    /// still let the click do what a click on the bar has always done.
    pub fn dispatch_status_click(&self, name: &str, payload: &MousePayload) -> bool {
        let key = {
            let sections = self.shared.statusline.borrow();
            match sections
                .iter()
                .find(|s| s.name == name && s.on_click.is_some())
            {
                // Cloned out of the borrow: the handler may register another
                // section, and holding the borrow across the call would panic.
                Some(sec) => match self
                    .lua
                    .registry_value::<Function>(sec.on_click.as_ref().expect("filtered on Some"))
                {
                    Ok(f) => f,
                    Err(_) => return false,
                },
                None => return false,
            }
        };
        let Some(table) = payload.to_table(&self.lua) else {
            return false;
        };
        match key.call::<()>(mlua::Value::Table(table)) {
            Ok(()) => true,
            Err(e) => {
                // A broken handler must not swallow the click silently — the
                // plugin author gets an error and the editor keeps working.
                self.shared.pending.borrow_mut().push(LuaAction::Notify(
                    3,
                    format!("statusline section {name:?} on_click failed: {e}"),
                ));
                true
            }
        }
    }

    /// Hand a submitted dialog's values to its `on_submit`, if it had one.
    ///
    /// `button` is the label of the button pressed, or `None` when the form was
    /// submitted with Enter. The callback is consumed either way — a dialog is
    /// shown once.
    pub fn fire_dialog_submit(&self, values: &[(String, String)], button: Option<&str>) {
        let Some(key) = self.shared.dialog_cb.borrow_mut().take() else {
            return;
        };
        if let Ok(func) = self.lua.registry_value::<Function>(&key) {
            let table = match self.lua.create_table() {
                Ok(t) => t,
                Err(_) => return,
            };
            for (k, v) in values {
                let _ = table.set(k.as_str(), v.as_str());
            }
            let btn = button.map(|b| b.to_string());
            let _ = func.call::<()>((table, btn));
        }
        let _ = self.lua.remove_registry_value(key);
    }

    /// Drop a dialog's callback without calling it (the user cancelled).
    pub fn discard_dialog_callback(&self) {
        if let Some(key) = self.shared.dialog_cb.borrow_mut().take() {
            let _ = self.lua.remove_registry_value(key);
        }
    }

    pub fn fire_event(&self, name: &str, args: &[mlua::Value]) {
        self.shared.events.borrow().emit(&self.lua, name, args);
    }

    /// Fire an event whose handlers may consume it by returning `true`.
    /// Returns whether any did, i.e. whether the built-in behaviour is
    /// cancelled.
    pub fn fire_event_consuming(&self, name: &str, args: &[mlua::Value]) -> bool {
        self.shared
            .events
            .borrow()
            .emit_consuming(&self.lua, name, args)
    }

    /// Hand a mouse event to Lua, returning whether a handler consumed it.
    ///
    /// Takes a plain Rust payload rather than Lua values so that callers never
    /// need to depend on `mlua` — the whole editor talks to Lua through this
    /// crate and nothing else.
    pub fn dispatch_mouse(&self, payload: &MousePayload) -> bool {
        let Some(table) = payload.to_table(&self.lua) else {
            return false;
        };
        self.fire_event_consuming(payload.event_name(), &[mlua::Value::Table(table)])
    }

    /// Take the context-menu items registered since the last call.
    pub fn take_context_menu_items(&self) -> Vec<(String, String, String)> {
        std::mem::take(&mut *self.shared.context_menu.borrow_mut())
    }

    /// Announce that the pointer has come to rest over the buffer.
    pub fn dispatch_hover(&self, payload: &MousePayload) {
        if let Some(table) = payload.to_table(&self.lua) {
            self.fire_event("hover", &[mlua::Value::Table(table)]);
        }
    }

    pub fn set_frame_dt(&self, dt: f64) {
        *self.shared.current_dt.borrow_mut() = dt;
        let val = mlua::Value::Number(dt);
        self.fire_event("Frame", &[val]);
        self.run_due_timers(dt * 1000.0);
    }

    /// Fire every `ruster.defer` / `ruster.timer` callback that has come due.
    ///
    /// The borrow of `timers` is dropped before anything is called, so a
    /// callback may schedule another timer — or cancel itself — without a
    /// `BorrowMutError`. That is not hypothetical: rescheduling from inside the
    /// callback is how you write a backoff.
    pub fn run_due_timers(&self, dt_ms: f64) {
        let due = { self.shared.timers.borrow_mut().take_due(&self.lua, dt_ms) };
        for func in due {
            if let Err(e) = func.call::<()>(()) {
                // A broken timer must not take the editor down, and must not
                // fail silently either — a plugin author with neither an effect
                // nor an error has nothing to go on.
                self.shared
                    .pending
                    .borrow_mut()
                    .push(LuaAction::Notify(3, format!("timer callback failed: {e}")));
            }
        }
    }

    /// How many timers are outstanding. For tests and `:messages` diagnostics.
    pub fn timer_count(&self) -> usize {
        self.shared.timers.borrow().len()
    }

    pub fn set_mode(&self, mode: &str) {
        if let Ok(ruster) = self.lua.globals().get::<mlua::Table>("ruster") {
            let _ = ruster.set("mode", mode);
        }
    }

    /// Expose the active editing paradigm to Lua as `ruster.editmode`
    /// (`"neovim"` or `"emacs"`), so plugins can support both.
    pub fn set_editmode(&self, editmode: &str) {
        if let Ok(ruster) = self.lua.globals().get::<mlua::Table>("ruster") {
            let _ = ruster.set("editmode", editmode);
        }
    }

    /// Fire with numeric arguments — `CursorMoved(line, col)` and anything else
    /// where handing Lua a string would make every handler call `tonumber`.
    pub fn fire_event_nums(&self, name: &str, nums: &[i64]) {
        let args: Vec<mlua::Value> = nums.iter().map(|n| mlua::Value::Integer(*n)).collect();
        self.fire_event(name, &args);
    }

    pub fn fire_event_str(&self, name: &str, string_args: &[&str]) {
        let vals: Vec<mlua::Value> = string_args
            .iter()
            .map(|s| mlua::Value::String(self.lua.create_string(s).unwrap()))
            .collect();
        self.fire_event(name, &vals);
    }

    /// The `ruster.config` table, if present.
    fn config_table(&self) -> Option<mlua::Table> {
        self.lua
            .globals()
            .get::<mlua::Table>("ruster")
            .ok()?
            .get::<mlua::Table>("config")
            .ok()
    }

    /// The typed config (validation errors discarded — see `config_validated`).
    pub fn config(&self) -> Config {
        self.config_validated().0
    }

    /// Read the typed config plus any validation errors. Grouped tables
    /// (`ruster.config.general = {…}`) are validated against the schema; an old
    /// flat `ruster.config = {…}` is read as before (no validation), so existing
    /// configs keep working.
    pub fn config_validated(&self) -> (Config, Vec<crate::schema::ConfigError>) {
        let defaults = Config::default();
        let cfg = match self.config_table() {
            Some(t) => t,
            None => return (defaults, Vec::new()),
        };
        let syntax = read_syntax_overrides(&cfg);
        let grouped = crate::schema::GROUPS
            .iter()
            .any(|(g, _)| cfg.get::<Option<mlua::Table>>(*g).ok().flatten().is_some());
        if !grouped {
            let mut c = config_flat(&cfg, &defaults);
            c.syntax_overrides = syntax;
            return (c, Vec::new());
        }

        // Validated grouped read: every schema value defaults, then override with
        // valid entries; type/range failures are collected, not fatal.
        let mut vals: std::collections::HashMap<
            (&'static str, &'static str),
            crate::schema::SettingValue,
        > = crate::schema::schema()
            .iter()
            .map(|s| ((s.group, s.key), s.default.clone()))
            .collect();
        let mut errors = Vec::new();
        for spec in crate::schema::schema() {
            let gt = match cfg.get::<Option<mlua::Table>>(spec.group).ok().flatten() {
                Some(t) => t,
                None => continue,
            };
            match read_setting(&gt, &spec) {
                Ok(None) => {} // absent → keep default
                Ok(Some(v)) => match spec.kind.check(&v) {
                    Ok(()) => {
                        vals.insert((spec.group, spec.key), v);
                    }
                    Err(_) => errors.push(crate::schema::ConfigError {
                        group: spec.group.into(),
                        key: spec.key.into(),
                        expected: spec.kind.expected(),
                        got: v.display(),
                    }),
                },
                Err(got) => errors.push(crate::schema::ConfigError {
                    group: spec.group.into(),
                    key: spec.key.into(),
                    expected: spec.kind.expected(),
                    got,
                }),
            }
        }
        let mut c = config_from_grouped(&vals, &defaults);
        c.syntax_overrides = syntax;
        (c, errors)
    }

    /// LSP server overrides from `ruster.lsp.servers[filetype] = { cmd, args }`.
    pub fn lsp_servers(&self) -> Vec<(String, String, Vec<String>)> {
        let mut out = Vec::new();
        let servers: mlua::Table = match self
            .lua
            .globals()
            .get::<mlua::Table>("ruster")
            .and_then(|r| r.get::<mlua::Table>("lsp"))
            .and_then(|l| l.get::<mlua::Table>("servers"))
        {
            Ok(t) => t,
            Err(_) => return out,
        };
        for pair in servers.pairs::<String, mlua::Table>().flatten() {
            let (lang, cfg) = pair;
            let cmd: String = cfg.get("cmd").unwrap_or_default();
            let args: Vec<String> = cfg.get("args").unwrap_or_default();
            if !cmd.is_empty() {
                out.push((lang, cmd, args));
            }
        }
        out
    }

    /// Evaluate a theme file (a Lua chunk returning `{ bg = "#…", … }`) into a
    /// color palette. Missing/invalid entries fall back to the default palette.
    pub fn load_theme(&self, code: &str) -> Option<crate::config::Theme> {
        use crate::config::{Rgb, Theme, ThemeColors};
        let t: mlua::Table = self.lua.load(code).eval().ok()?;
        let d = ThemeColors::default();
        let get = |k: &str, def: Rgb| -> Rgb {
            t.get::<Option<String>>(k)
                .ok()
                .flatten()
                .and_then(|s| crate::schema::parse_hex_color(&s))
                .map(|(r, g, b)| Rgb::new(r, g, b))
                .unwrap_or(def)
        };
        let bg = get("bg", d.bg);
        let fg = get("fg", d.fg);
        let accent = get("accent", d.accent);
        let roles = ThemeColors {
            bg,
            fg,
            gutter: get("gutter", d.gutter),
            // Older theme files omit these; default fg-companions to fg/bg.
            gutter_bg: get("gutter_bg", bg),
            cursor_bg: get("cursor_bg", d.cursor_bg),
            selection_bg: get("selection_bg", d.selection_bg),
            selection_fg: get("selection_fg", fg),
            cursor_fg: get("cursor_fg", bg),
            divider: get("divider", d.divider),
            statusline_fg: get("statusline_fg", fg),
            statusline_bg: get("statusline_bg", d.statusline_bg),
            accent,
            accent_fg: get("accent_fg", bg),
            whichkey_bg: get("whichkey_bg", d.whichkey_bg),
            whichkey_fg: get("whichkey_fg", fg),
            whichkey_key: get("whichkey_key", accent),
            cmdline_bg: get("cmdline_bg", d.cmdline_bg),
            cmdline_fg: get("cmdline_fg", fg),
            cmdline_accent: get("cmdline_accent", accent),
            // Focused falls back to the accent because that is already the
            // "this is the active thing" colour everywhere else in the UI;
            // unfocused to the divider, which is what separates panes.
            border_focused: get("border_focused", accent),
            border_unfocused: get("border_unfocused", d.divider),
            mode_normal_bg: get("mode_normal_bg", d.mode_normal_bg),
            mode_normal_fg: get("mode_normal_fg", fg),
            mode_insert_bg: get("mode_insert_bg", d.mode_insert_bg),
            mode_insert_fg: get("mode_insert_fg", fg),
            mode_visual_bg: get("mode_visual_bg", d.mode_visual_bg),
            mode_visual_fg: get("mode_visual_fg", fg),
            mode_cmdline_bg: get("mode_cmdline_bg", d.mode_cmdline_bg),
            mode_cmdline_fg: get("mode_cmdline_fg", fg),
            mode_emacs_bg: get("mode_emacs_bg", d.mode_emacs_bg),
            mode_emacs_fg: get("mode_emacs_fg", fg),
        };
        // A `palette` sub-table of named colors, or (for older files) the roles.
        let palette = match t.get::<Option<mlua::Table>>("palette").ok().flatten() {
            Some(pt) => {
                let mut v = Vec::new();
                for (name, hex) in pt.pairs::<String, String>().flatten() {
                    if let Some((r, g, b)) = crate::schema::parse_hex_color(&hex) {
                        v.push((name, Rgb::new(r, g, b)));
                    }
                }
                v.sort_by(|a, b| a.0.cmp(&b.0)); // Lua pairs() order is unspecified
                v
            }
            None => vec![
                ("bg".into(), roles.bg),
                ("fg".into(), roles.fg),
                ("gutter".into(), roles.gutter),
                ("cursor_bg".into(), roles.cursor_bg),
                ("selection_bg".into(), roles.selection_bg),
                ("divider".into(), roles.divider),
                ("accent".into(), roles.accent),
                ("whichkey_bg".into(), roles.whichkey_bg),
                ("whichkey_fg".into(), roles.whichkey_fg),
                ("cmdline_bg".into(), roles.cmdline_bg),
                ("cmdline_fg".into(), roles.cmdline_fg),
                ("cmdline_accent".into(), roles.cmdline_accent),
                ("border_focused".into(), roles.border_focused),
                ("border_unfocused".into(), roles.border_unfocused),
            ],
        };
        Some(Theme { palette, roles })
    }

    pub fn load_init(&mut self, path: &Path) -> Result<(), String> {
        let code = std::fs::read_to_string(path)
            .map_err(|e| format!("Cannot read {}: {}", path.display(), e))?;
        self.lua
            .load(&code)
            .exec()
            .map_err(|e| format!("Lua error in {}: {}", path.display(), e))
    }

    pub fn drain_actions(&self) -> Vec<LuaAction> {
        self.shared.pending.borrow_mut().drain(..).collect()
    }

    /// Check if a Lua keymap matches for the given mode and key.
    /// Returns true if matched (consumed the key).
    pub fn handle_key(&self, mode: &str, ck: &crossterm::event::KeyEvent) -> bool {
        for km in self.shared.keymaps.borrow().iter() {
            if km.mode != mode {
                continue;
            }
            if km.keys.len() != 1 {
                continue;
            } // multi-keys in future
            let expected = crate::keymap::lua_key_to_crossterm(&km.keys[0]);
            if expected == *ck {
                if let Ok(func) = self.lua.registry_value::<Function>(&km.callback) {
                    let _ = func.call::<()>(());
                    return true;
                }
            }
        }
        false
    }
}

// --- config reading helpers ---

/// The legacy flat read: `ruster.config = { number = …, timeoutlen = … }`.
/// Only the historically-flat keys are read; newer options keep their defaults.
fn config_flat(cfg: &mlua::Table, defaults: &Config) -> Config {
    let mut c = defaults.clone();
    c.tabstop = cfg.get("tabstop").unwrap_or(defaults.tabstop);
    c.softtabstop = cfg.get("softtabstop").unwrap_or(defaults.softtabstop);
    c.expandtab = cfg.get("expandtab").unwrap_or(defaults.expandtab);
    c.shiftwidth = cfg.get("shiftwidth").unwrap_or(defaults.shiftwidth);
    c.number = cfg.get("number").unwrap_or(defaults.number);
    c.relativenumber = cfg.get("relativenumber").unwrap_or(defaults.relativenumber);
    c.theme = cfg.get("theme").unwrap_or_else(|_| defaults.theme.clone());
    c.gui_font = cfg
        .get("gui_font")
        .unwrap_or_else(|_| defaults.gui_font.clone());
    c.cursor_anim_enabled = cfg
        .get("cursor_anim_enabled")
        .unwrap_or(defaults.cursor_anim_enabled);
    c.cursor_anim_speed = cfg
        .get("cursor_anim_speed")
        .unwrap_or(defaults.cursor_anim_speed);
    c.timeoutlen = cfg.get("timeoutlen").unwrap_or(defaults.timeoutlen);
    c.format_on_save = cfg.get("format_on_save").unwrap_or(defaults.format_on_save);
    c.terminal_shell = cfg
        .get("terminal_shell")
        .unwrap_or_else(|_| defaults.terminal_shell.clone());
    c.terminal_scrollback = cfg
        .get("terminal_scrollback")
        .unwrap_or(defaults.terminal_scrollback);
    c
}

/// Map validated grouped values onto the typed `Config` (only the keys the app
/// consumes today; other schema keys are validated but wired up separately).
fn config_from_grouped(
    vals: &std::collections::HashMap<(&'static str, &'static str), crate::schema::SettingValue>,
    _defaults: &Config,
) -> Config {
    let slice: Vec<_> = vals
        .iter()
        .map(|((g, k), v)| ((*g, *k), v.clone()))
        .collect();
    Config::from_settings(&slice)
}

/// Read `ruster.config.syntax = { lang = { group = "#hex", … }, … }` into the
/// `lang -> group -> hex` map. Only `#RRGGBB` string values are kept.
fn read_syntax_overrides(
    cfg: &mlua::Table,
) -> std::collections::HashMap<String, std::collections::HashMap<String, String>> {
    let mut out = std::collections::HashMap::new();
    let Some(syntax) = cfg.get::<Option<mlua::Table>>("syntax").ok().flatten() else {
        return out;
    };
    for (lang, groups) in syntax.pairs::<String, mlua::Table>().flatten() {
        let mut m = std::collections::HashMap::new();
        for (group, hex) in groups.pairs::<String, String>().flatten() {
            if crate::schema::parse_hex_color(&hex).is_some() {
                m.insert(group, hex);
            }
        }
        if !m.is_empty() {
            out.insert(lang, m);
        }
    }
    out
}

/// Read one setting from a group table by its kind. `Ok(None)` = absent (use
/// default); `Err(got)` = present but wrong type, with a display of the value.
fn read_setting(
    tbl: &mlua::Table,
    spec: &crate::schema::SettingSpec,
) -> Result<Option<crate::schema::SettingValue>, String> {
    use crate::schema::{SettingKind, SettingValue as V};
    match &spec.kind {
        SettingKind::Bool => Ok(get_opt::<bool>(tbl, spec.key)?.map(V::Bool)),
        SettingKind::Int { .. } => Ok(get_opt::<i64>(tbl, spec.key)?.map(V::Int)),
        SettingKind::Float { .. } => Ok(get_opt::<f64>(tbl, spec.key)?.map(V::Float)),
        SettingKind::Text => Ok(get_opt::<String>(tbl, spec.key)?.map(V::Text)),
        SettingKind::Enum(_) => Ok(get_opt::<String>(tbl, spec.key)?.map(V::Enum)),
        SettingKind::Color => Ok(get_opt::<String>(tbl, spec.key)?.map(V::Color)),
    }
}

fn get_opt<T: mlua::FromLua>(tbl: &mlua::Table, key: &str) -> Result<Option<T>, String> {
    tbl.get::<Option<T>>(key).map_err(|_| raw_display(tbl, key))
}

fn raw_display(tbl: &mlua::Table, key: &str) -> String {
    match tbl.get::<mlua::Value>(key) {
        Ok(mlua::Value::Nil) => "nil".into(),
        Ok(mlua::Value::String(s)) => format!(
            "{:?}",
            s.to_str().map(|x| x.to_string()).unwrap_or_default()
        ),
        Ok(mlua::Value::Integer(i)) => i.to_string(),
        Ok(mlua::Value::Number(n)) => n.to_string(),
        Ok(mlua::Value::Boolean(b)) => b.to_string(),
        Ok(other) => other.type_name().to_string(),
        Err(_) => "?".into(),
    }
}

#[cfg(test)]
mod config_tests {
    use super::*;

    fn rt_with(src: &str) -> LuaRuntime {
        let rt = LuaRuntime::new().unwrap();
        rt.lua.load(src).exec().unwrap();
        rt
    }

    #[test]
    fn grouped_config_reads_typed_values() {
        let rt = rt_with(
            r#"
            ruster.config.general = { tabstop = 2, expandtab = false }
            ruster.config.gutter = { number = true }
            ruster.config.whichkey = { timeoutlen = 500 }
        "#,
        );
        let (cfg, errors) = rt.config_validated();
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(cfg.tabstop, 2);
        assert!(!cfg.expandtab);
        assert!(cfg.number);
        assert_eq!(cfg.timeoutlen, 500);
    }

    #[test]
    fn grouped_config_reports_bad_values_and_uses_default() {
        let rt = rt_with(
            r#"
            ruster.config.gui = { font_size = "big" }
            ruster.config.general = { tabstop = 999 }
        "#,
        );
        let (cfg, errors) = rt.config_validated();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert_eq!(cfg.tabstop, 4, "invalid tabstop falls back to default");
        assert!(errors
            .iter()
            .any(|e| e.key == "font_size" && e.group == "gui"));
        assert!(errors
            .iter()
            .any(|e| e.key == "tabstop" && e.group == "general"));
    }

    #[test]
    fn legacy_flat_config_still_works() {
        let rt = rt_with("ruster.config = { tabstop = 3, number = true }");
        let (cfg, errors) = rt.config_validated();
        assert!(errors.is_empty());
        assert_eq!(cfg.tabstop, 3);
        assert!(cfg.number);
    }

    #[test]
    fn syntax_overrides_are_parsed() {
        let rt = rt_with(
            r##"
            ruster.config.general = { tabstop = 4 }
            ruster.config.syntax = {
                rust = { keyword = "#ff00ff", string = "not-a-color" },
                python = { comment = "#123456" },
            }
        "##,
        );
        let (cfg, _errors) = rt.config_validated();
        assert_eq!(
            cfg.syntax_overrides["rust"]
                .get("keyword")
                .map(String::as_str),
            Some("#ff00ff")
        );
        // Non-hex values are dropped.
        assert!(!cfg.syntax_overrides["rust"].contains_key("string"));
        assert_eq!(
            cfg.syntax_overrides["python"]
                .get("comment")
                .map(String::as_str),
            Some("#123456")
        );
    }

    #[test]
    fn syntax_to_lua_round_trips_through_the_parser() {
        use std::collections::HashMap;
        let mut map: HashMap<String, HashMap<String, String>> = HashMap::new();
        map.insert(
            "rust".into(),
            HashMap::from([("keyword".to_string(), "#abcdef".to_string())]),
        );
        let lua = crate::config::syntax_to_lua(&map);
        let rt = rt_with(&lua);
        let (cfg, _) = rt.config_validated();
        assert_eq!(
            cfg.syntax_overrides["rust"]
                .get("keyword")
                .map(String::as_str),
            Some("#abcdef")
        );
    }
}
