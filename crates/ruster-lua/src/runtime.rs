use std::cell::RefCell;
use std::path::Path;
use mlua::{Function, Lua};
use crate::keymap::LuaKeymap;

#[derive(Debug)]
pub enum LuaAction {
    Cmd(String),
    Print(String),
}

pub struct LuaRuntime {
    pub(crate) lua: Lua,
    pub(crate) keymaps: RefCell<Vec<LuaKeymap>>,
    pub(crate) pending: RefCell<Vec<LuaAction>>,
}

impl LuaRuntime {
    pub fn new() -> mlua::Result<Self> {
        let lua = Lua::new();
        let pending = RefCell::new(Vec::new());
        let runtime = LuaRuntime { lua, keymaps: RefCell::new(Vec::new()), pending };

        let ruster = crate::api::create_table(&runtime)?;
        runtime.lua.globals().set("ruster", ruster)?;
        Ok(runtime)
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
