use mlua::{Function, Table, Value};
use crate::runtime::{self, LuaRuntime};
use crate::keymap::{parse_lua_key, LuaKeymap};

pub fn create_table(runtime: &LuaRuntime) -> mlua::Result<Table> {
    let t = runtime.lua.create_table()?;

    // ruster.print(...)
    let rt = runtime as *const LuaRuntime;
    let print_fn = runtime.lua.create_function(move |_, args: mlua::MultiValue| {
        let parts: Vec<String> = args.iter().map(|v| format_value(v)).collect();
        let msg = parts.join("\t");
        unsafe { (*rt).pending.borrow_mut().push(runtime::LuaAction::Print(msg)); }
        Ok(())
    })?;
    t.set("print", print_fn)?;

    // ruster.cmd(str)
    let rt = runtime as *const LuaRuntime;
    let cmd_fn = runtime.lua.create_function(move |_, cmd: String| {
        unsafe { (*rt).pending.borrow_mut().push(runtime::LuaAction::Cmd(cmd)); }
        Ok(())
    })?;
    t.set("cmd", cmd_fn)?;

    // ruster.keymap.set(mode, lhs, callback)
    let rt = runtime as *const LuaRuntime;
    let keymap_set = runtime.lua.create_function(move |_, (mode, lhs, func): (String, String, Function)| {
        let keys: Vec<_> = lhs.split_inclusive(|c: char| c == '>')
            .filter(|s| !s.is_empty())
            .filter_map(|s| {
                if s.ends_with('>') { parse_lua_key(s.trim()) }
                else { s.chars().map(|c| parse_lua_key(&c.to_string())).collect::<Option<Vec<_>>>()?.into_iter().next() }
            })
            .collect();
        if keys.is_empty() { return Err(mlua::Error::external("Cannot parse key sequence")); }
        let reg = unsafe { (*rt).lua.create_registry_value(func) };
        match reg {
            Ok(r) => unsafe { (*rt).keymaps.borrow_mut().push(LuaKeymap { mode, keys, callback: r }) },
            Err(e) => return Err(e),
        }
        Ok(())
    })?;
    let keymap = runtime.lua.create_table()?;
    keymap.set("set", keymap_set)?;
    t.set("keymap", keymap)?;

    // ruster.g
    let g = runtime.lua.create_table()?;
    t.set("g", g)?;

    // ruster.mode - read-only, set by App
    t.set("mode", "normal")?;

    Ok(t)
}

fn format_value(v: &Value) -> String {
    match v {
        Value::Nil => "nil".into(),
        Value::String(s) => s.to_str().map(|s| s.to_string()).unwrap_or_else(|_| "?".to_string()),
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Boolean(b) => b.to_string(),
        _ => format!("{:?}", v),
    }
}
