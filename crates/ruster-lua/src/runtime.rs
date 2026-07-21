use std::cell::RefCell;
use std::path::Path;
use mlua::{Function, Lua};
use crate::config::Config;
use crate::event::EventBus;
use crate::keymap::LuaKeymap;

#[derive(Debug)]
pub enum LuaAction {
    Cmd(String),
    Print(String),
}

pub struct LuaRuntime {
    pub lua: Lua,
    pub(crate) keymaps: RefCell<Vec<LuaKeymap>>,
    pub(crate) pending: RefCell<Vec<LuaAction>>,
    pub events: RefCell<EventBus>,
    pub current_dt: RefCell<f64>,
    pub(crate) get_lines: RefCell<Option<Box<dyn FnMut(i32, Option<i32>) -> Vec<String>>>>,
    pub(crate) set_lines: RefCell<Option<Box<dyn FnMut(i32, i32, Vec<String>)>>>,
    pub(crate) get_cursor: RefCell<Option<Box<dyn FnMut() -> (i32, i32)>>>,
    pub(crate) set_cursor: RefCell<Option<Box<dyn FnMut(i32, i32)>>>,
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

    pub fn fire_event_str(&self, name: &str, string_args: &[&str]) {
        let vals: Vec<mlua::Value> = string_args.iter()
            .map(|s| mlua::Value::String(self.lua.create_string(s).unwrap()))
            .collect();
        self.fire_event(name, &vals);
    }

    pub fn config(&self) -> Config {
        let defaults = Config::default();
        let ruster = match self.lua.globals().get::<mlua::Table>("ruster") {
            Ok(t) => t,
            Err(_) => return defaults,
        };
        let cfg = match ruster.get::<mlua::Table>("config") {
            Ok(t) => t,
            Err(_) => return defaults,
        };
        Config {
            tabstop: cfg.get("tabstop").unwrap_or(defaults.tabstop),
            softtabstop: cfg.get("softtabstop").unwrap_or(defaults.softtabstop),
            expandtab: cfg.get("expandtab").unwrap_or(defaults.expandtab),
            number: cfg.get("number").unwrap_or(defaults.number),
            relativenumber: cfg.get("relativenumber").unwrap_or(defaults.relativenumber),
            theme: cfg.get("theme").unwrap_or(defaults.theme),
            cursor_anim_enabled: cfg.get("cursor_anim_enabled").unwrap_or(defaults.cursor_anim_enabled),
            cursor_anim_speed: cfg.get("cursor_anim_speed").unwrap_or(defaults.cursor_anim_speed),
        }
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

pub(crate) fn queue_action(runtime: &LuaRuntime, action: LuaAction) {
    runtime.pending.borrow_mut().push(action);
}
