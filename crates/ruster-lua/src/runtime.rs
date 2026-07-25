use std::cell::RefCell;
use std::path::Path;
use mlua::{Function, Lua, RegistryKey};
use crate::config::Config;
use crate::event::EventBus;
use crate::keymap::LuaKeymap;

#[derive(Debug)]
pub enum LuaAction {
    Cmd(String),
    Print(String),
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

/// Buffer/cursor bridge callbacks the app installs so Lua can read and edit the
/// active buffer. Boxed because their concrete closures live in the frontend.
type GetLinesFn = Box<dyn FnMut(i32, Option<i32>) -> Vec<String>>;
type SetLinesFn = Box<dyn FnMut(i32, i32, Vec<String>)>;
type GetCursorFn = Box<dyn FnMut() -> (i32, i32)>;
type SetCursorFn = Box<dyn FnMut(i32, i32)>;

pub struct LuaRuntime {
    pub lua: Lua,
    pub(crate) keymaps: RefCell<Vec<LuaKeymap>>,
    pub(crate) pending: RefCell<Vec<LuaAction>>,
    pub events: RefCell<EventBus>,
    pub current_dt: RefCell<f64>,
    pub(crate) get_lines: RefCell<Option<GetLinesFn>>,
    pub(crate) set_lines: RefCell<Option<SetLinesFn>>,
    pub(crate) get_cursor: RefCell<Option<GetCursorFn>>,
    pub(crate) set_cursor: RefCell<Option<SetCursorFn>>,
    /// Lua-registered statusline sections: (position, callback registry key).
    pub(crate) statusline: RefCell<Vec<(String, RegistryKey)>>,
    /// Window/buffer manipulation callbacks installed by the app.
    pub(crate) window_cb: RefCell<Option<WindowCallbacks>>,
}

impl LuaRuntime {
    pub fn new() -> mlua::Result<Self> {
        let lua = Lua::new();
        let pending = RefCell::new(Vec::new());
        let events = RefCell::new(EventBus::new());
        let runtime = LuaRuntime {
            lua,
            keymaps: RefCell::new(Vec::new()),
            pending,
            events,
            current_dt: RefCell::new(0.0),
            get_lines: RefCell::new(None),
            set_lines: RefCell::new(None),
            get_cursor: RefCell::new(None),
            set_cursor: RefCell::new(None),
            statusline: RefCell::new(Vec::new()),
            window_cb: RefCell::new(None),
        };

        let ruster = crate::api::create_table(&runtime)?;
        runtime.lua.globals().set("ruster", ruster)?;
        Ok(runtime)
    }

    pub fn set_buffer_callbacks(
        &self,
        get_lines: Box<dyn FnMut(i32, Option<i32>) -> Vec<String>>,
        set_lines: Box<dyn FnMut(i32, i32, Vec<String>)>,
        get_cursor: Box<dyn FnMut() -> (i32, i32)>,
        set_cursor: Box<dyn FnMut(i32, i32)>,
    ) {
        self.get_lines.replace(Some(get_lines));
        self.set_lines.replace(Some(set_lines));
        self.get_cursor.replace(Some(get_cursor));
        self.set_cursor.replace(Some(set_cursor));
    }

    /// Install the window/buffer manipulation callbacks.
    pub fn set_window_callbacks(&self, cb: WindowCallbacks) {
        self.window_cb.replace(Some(cb));
    }

    /// Evaluate all Lua statusline sections registered for `pos`
    /// ("left" | "center" | "right"), returning each one's string result.
    pub fn statusline_sections(&self, pos: &str) -> Vec<String> {
        let sections = self.statusline.borrow();
        let mut out = Vec::new();
        for (p, key) in sections.iter() {
            if p != pos {
                continue;
            }
            if let Ok(func) = self.lua.registry_value::<Function>(key) {
                if let Ok(s) = func.call::<String>(()) {
                    if !s.is_empty() {
                        out.push(s);
                    }
                }
            }
        }
        out
    }

    pub fn fire_event(&self, name: &str, args: &[mlua::Value]) {
        self.events.borrow().emit(&self.lua, name, args);
    }

    pub fn set_frame_dt(&self, dt: f64) {
        *self.current_dt.borrow_mut() = dt;
        let val = mlua::Value::Number(dt);
        self.fire_event("Frame", &[val]);
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

    pub fn fire_event_str(&self, name: &str, string_args: &[&str]) {
        let vals: Vec<mlua::Value> = string_args.iter()
            .map(|s| mlua::Value::String(self.lua.create_string(s).unwrap()))
            .collect();
        self.fire_event(name, &vals);
    }

    /// The `ruster.config` table, if present.
    fn config_table(&self) -> Option<mlua::Table> {
        self.lua.globals().get::<mlua::Table>("ruster").ok()?.get::<mlua::Table>("config").ok()
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
        let grouped = crate::schema::GROUPS
            .iter()
            .any(|(g, _)| cfg.get::<Option<mlua::Table>>(*g).ok().flatten().is_some());
        if !grouped {
            return (config_flat(&cfg, &defaults), Vec::new());
        }

        // Validated grouped read: every schema value defaults, then override with
        // valid entries; type/range failures are collected, not fatal.
        let mut vals: std::collections::HashMap<(&'static str, &'static str), crate::schema::SettingValue> =
            crate::schema::schema().iter().map(|s| ((s.group, s.key), s.default.clone())).collect();
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
        (config_from_grouped(&vals, &defaults), errors)
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

    pub fn load_init(&mut self, path: &Path) -> Result<(), String> {
        let code = std::fs::read_to_string(path).map_err(|e| format!("Cannot read {}: {}", path.display(), e))?;
        self.lua.load(&code).exec().map_err(|e| format!("Lua error in {}: {}", path.display(), e))
    }

    pub fn drain_actions(&self) -> Vec<LuaAction> {
        self.pending.borrow_mut().drain(..).collect()
    }

    /// Check if a Lua keymap matches for the given mode and key.
    /// Returns true if matched (consumed the key).
    pub fn handle_key(&self, mode: &str, ck: &crossterm::event::KeyEvent) -> bool {
        for km in self.keymaps.borrow().iter() {
            if km.mode != mode { continue; }
            if km.keys.len() != 1 { continue; } // multi-keys in future
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
    c.gui_font = cfg.get("gui_font").unwrap_or_else(|_| defaults.gui_font.clone());
    c.cursor_anim_enabled = cfg.get("cursor_anim_enabled").unwrap_or(defaults.cursor_anim_enabled);
    c.cursor_anim_speed = cfg.get("cursor_anim_speed").unwrap_or(defaults.cursor_anim_speed);
    c.timeoutlen = cfg.get("timeoutlen").unwrap_or(defaults.timeoutlen);
    c.format_on_save = cfg.get("format_on_save").unwrap_or(defaults.format_on_save);
    c.terminal_shell = cfg.get("terminal_shell").unwrap_or_else(|_| defaults.terminal_shell.clone());
    c.terminal_scrollback = cfg.get("terminal_scrollback").unwrap_or(defaults.terminal_scrollback);
    c
}

/// Map validated grouped values onto the typed `Config` (only the keys the app
/// consumes today; other schema keys are validated but wired up separately).
fn config_from_grouped(
    vals: &std::collections::HashMap<(&'static str, &'static str), crate::schema::SettingValue>,
    defaults: &Config,
) -> Config {
    use crate::config::{Rgb, ThemeColors};
    use crate::schema::SettingValue as V;
    let asb = |g, k, d: bool| match vals.get(&(g, k)) {
        Some(V::Bool(b)) => *b,
        _ => d,
    };
    let asu = |g, k, d: u32| match vals.get(&(g, k)) {
        Some(V::Int(i)) => *i as u32,
        _ => d,
    };
    let asf = |g, k, d: f32| match vals.get(&(g, k)) {
        Some(V::Float(f)) => *f as f32,
        _ => d,
    };
    let ass = |g, k| match vals.get(&(g, k)) {
        Some(V::Text(s)) | Some(V::Enum(s)) | Some(V::Color(s)) => Some(s.clone()),
        _ => None,
    };
    let opt_str = |g, k| ass(g, k).filter(|s| !s.is_empty());
    let asc = |g, k, d: Rgb| match vals.get(&(g, k)) {
        Some(V::Color(s)) => crate::schema::parse_hex_color(s)
            .map(|(r, gg, b)| Rgb::new(r, gg, b))
            .unwrap_or(d),
        _ => d,
    };
    let dc = defaults.colors;
    Config {
        tabstop: asu("general", "tabstop", defaults.tabstop),
        softtabstop: asu("general", "softtabstop", defaults.softtabstop),
        expandtab: asb("general", "expandtab", defaults.expandtab),
        shiftwidth: asu("general", "shiftwidth", defaults.shiftwidth),
        editmode: ass("general", "editmode").unwrap_or_else(|| defaults.editmode.clone()),
        editorconfig: asb("general", "editorconfig", defaults.editorconfig),
        line_ending: ass("general", "line_ending").unwrap_or_else(|| defaults.line_ending.clone()),
        number: asb("gutter", "number", defaults.number),
        relativenumber: asb("gutter", "relativenumber", defaults.relativenumber),
        theme: ass("general", "theme").unwrap_or_else(|| defaults.theme.clone()),
        gui_font: opt_str("gui", "font"),
        font_size: asu("gui", "font_size", defaults.font_size),
        line_height: asu("gui", "line_height", defaults.line_height),
        padding_x: asu("gui", "padding_x", defaults.padding_x),
        padding_y: asu("gui", "padding_y", defaults.padding_y),
        window_width: asu("gui", "window_width", defaults.window_width),
        window_height: asu("gui", "window_height", defaults.window_height),
        target_fps: asu("gui", "target_fps", defaults.target_fps),
        cursor_kind: ass("gui", "cursor_kind").unwrap_or_else(|| defaults.cursor_kind.clone()),
        cursor_anim_enabled: asb("gui", "cursor_anim", defaults.cursor_anim_enabled),
        cursor_anim_speed: asf("gui", "cursor_anim_speed", defaults.cursor_anim_speed),
        colors: ThemeColors {
            bg: asc("gui", "color_bg", dc.bg),
            fg: asc("gui", "color_fg", dc.fg),
            gutter: asc("gui", "color_gutter", dc.gutter),
            selection: asc("gui", "color_selection", dc.selection),
            cursor: asc("gui", "color_cursor", dc.cursor),
            divider: asc("gui", "color_divider", dc.divider),
            accent: asc("gui", "color_accent", dc.accent),
        },
        timeoutlen: asu("whichkey", "timeoutlen", defaults.timeoutlen),
        whichkey_enabled: asb("whichkey", "enabled", defaults.whichkey_enabled),
        format_on_save: asb("lsp", "format_on_save", defaults.format_on_save),
        lsp_diagnostics: asb("lsp", "diagnostics", defaults.lsp_diagnostics),
        lsp_hover: asb("lsp", "hover", defaults.lsp_hover),
        lsp_autostart: asb("lsp", "autostart", defaults.lsp_autostart),
        terminal_shell: opt_str("terminal", "shell"),
        terminal_scrollback: asu("terminal", "scrollback", defaults.terminal_scrollback),
        terminal_default_mode: ass("terminal", "default_mode")
            .unwrap_or_else(|| defaults.terminal_default_mode.clone()),
        dired_show_hidden: asb("dired", "show_hidden", defaults.dired_show_hidden),
    }
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
        Ok(mlua::Value::String(s)) => format!("{:?}", s.to_str().map(|x| x.to_string()).unwrap_or_default()),
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
        assert!(errors.iter().any(|e| e.key == "font_size" && e.group == "gui"));
        assert!(errors.iter().any(|e| e.key == "tabstop" && e.group == "general"));
    }

    #[test]
    fn legacy_flat_config_still_works() {
        let rt = rt_with("ruster.config = { tabstop = 3, number = true }");
        let (cfg, errors) = rt.config_validated();
        assert!(errors.is_empty());
        assert_eq!(cfg.tabstop, 3);
        assert!(cfg.number);
    }
}
