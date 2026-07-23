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

    // ruster.g - global variable table
    let g = runtime.lua.create_table()?;
    t.set("g", g)?;

    // ruster.mode - read-only, set by App
    t.set("mode", "normal")?;

    // ruster.on(event, callback) — event registration
    let rt = runtime as *const LuaRuntime;
    let on_fn = runtime.lua.create_function(move |_, (event, func): (String, Function)| {
        unsafe {
            let mut events = (*rt).events.borrow_mut();
            events.on(&(*rt).lua, &event, func)
        }
    })?;
    t.set("on", on_fn)?;

    // ruster.statusline.section(pos, fn) — register a statusline component.
    // `pos` is "left" | "center" | "right"; `fn` returns a string each frame.
    let statusline = runtime.lua.create_table()?;
    let rt_sl = runtime as *const LuaRuntime;
    let section_fn = runtime.lua.create_function(move |_, (pos, func): (String, Function)| {
        unsafe {
            let key = (*rt_sl).lua.create_registry_value(func)?;
            (*rt_sl).statusline.borrow_mut().push((pos, key));
        }
        Ok(())
    })?;
    statusline.set("section", section_fn)?;
    t.set("statusline", statusline)?;

    // ruster.api table
    let api = runtime.lua.create_table()?;

    // nvim_buf_get_lines(buf, start, end_opt)
    let rt = runtime as *const LuaRuntime;
    let get_lines = runtime.lua.create_function(move |lua, (_buf, start, end_opt): (i32, i32, Option<i32>)| {
        let mut cb = unsafe { (*rt).get_lines.borrow_mut() };
        let lines = match &mut *cb {
            Some(f) => f(start, end_opt),
            None => Vec::new(),
        };
        let t = lua.create_table()?;
        for (i, line) in lines.iter().enumerate() {
            t.set(i as i32 + 1, line.as_str())?;
        }
        Ok(mlua::Value::Table(t))
    })?;
    api.set("nvim_buf_get_lines", get_lines)?;

    // nvim_buf_set_lines(buf, start, end, lines)
    let rt = runtime as *const LuaRuntime;
    let set_lines = runtime.lua.create_function(move |_, (_buf, start, end, lines): (i32, i32, i32, mlua::Value)| {
        let lines_vec: Vec<String> = match lines {
            mlua::Value::String(s) => vec![s.to_str().map(|s| s.to_string()).unwrap_or_default()],
            mlua::Value::Table(t) => {
                let mut v = Vec::new();
                for i in 1..=t.len()? {
                    if let Ok(s) = t.get::<String>(i) { v.push(s); }
                }
                v
            }
            _ => return Err(mlua::Error::external("set_lines expects string or table")),
        };
        let mut cb = unsafe { (*rt).set_lines.borrow_mut() };
        if let Some(f) = cb.as_mut() {
            f(start, end, lines_vec);
        }
        Ok(())
    })?;
    api.set("nvim_buf_set_lines", set_lines)?;

    // nvim_win_get_cursor(win)
    let rt = runtime as *const LuaRuntime;
    let get_cursor = runtime.lua.create_function(move |lua, _win: i32| {
        let mut cb = unsafe { (*rt).get_cursor.borrow_mut() };
        match &mut *cb {
            Some(f) => {
                let (row, col) = f();
                let t = lua.create_table()?;
                t.set("row", row)?;
                t.set("col", col)?;
                Ok(mlua::Value::Table(t))
            }
            None => Ok(mlua::Value::Nil),
        }
    })?;
    api.set("nvim_win_get_cursor", get_cursor)?;

    // nvim_win_set_cursor(win, {row, col})
    let rt = runtime as *const LuaRuntime;
    let set_cursor = runtime.lua.create_function(move |_, (_win, pos): (i32, mlua::Table)| {
        let row: i32 = pos.get("row").unwrap_or(0);
        let col: i32 = pos.get("col").unwrap_or(0);
        let mut cb = unsafe { (*rt).set_cursor.borrow_mut() };
        if let Some(f) = cb.as_mut() {
            f(row, col);
        }
        Ok(())
    })?;
    api.set("nvim_win_set_cursor", set_cursor)?;

    // ruster.api.get_frame_delta()
    let rt = runtime as *const LuaRuntime;
    let get_frame_delta = runtime.lua.create_function(move |_, ()| {
        unsafe {
            let dt = (*rt).current_dt.borrow();
            Ok(*dt)
        }
    })?;
    api.set("get_frame_delta", get_frame_delta)?;

    t.set("api", api)?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::LuaRuntime;

    fn make_runtime() -> LuaRuntime {
        LuaRuntime::new().expect("LuaRuntime init")
    }

    #[test]
    fn print_queues_action() {
        let rt = make_runtime();
        let t = create_table(&rt).unwrap();
        let print_fn: Function = t.get("print").unwrap();
        print_fn.call::<()>("hello").unwrap();
        let actions = rt.drain_actions();
        assert!(matches!(actions.as_slice(), [runtime::LuaAction::Print(m)] if m == "hello"));
    }

    #[test]
    fn cmd_queues_action() {
        let rt = make_runtime();
        let t = create_table(&rt).unwrap();
        let cmd_fn: Function = t.get("cmd").unwrap();
        cmd_fn.call::<()>(":w").unwrap();
        let actions = rt.drain_actions();
        assert!(matches!(actions.as_slice(), [runtime::LuaAction::Cmd(m)] if m == ":w"));
    }

    #[test]
    fn statusline_section_registers_and_evaluates() {
        let rt = make_runtime();
        let t = create_table(&rt).unwrap();
        let statusline: Table = t.get("statusline").unwrap();
        let section: Function = statusline.get("section").unwrap();
        let f = rt.lua.create_function(|_, ()| Ok("git:main".to_string())).unwrap();
        section.call::<()>(("right", f)).unwrap();
        assert_eq!(rt.statusline_sections("right"), vec!["git:main".to_string()]);
        assert!(rt.statusline_sections("left").is_empty());
    }

    #[test]
    fn api_get_lines_no_callback_returns_empty() {
        let rt = make_runtime();
        let t = create_table(&rt).unwrap();
        let api: Table = t.get("api").unwrap();
        let get_lines: Function = api.get("nvim_buf_get_lines").unwrap();
        let result: Value = get_lines.call((0, 0, Option::<i32>::None)).unwrap();
        assert!(matches!(result, Value::Table(_)));
        let table = match result {
            Value::Table(t) => t,
            _ => panic!("expected table"),
        };
        assert_eq!(table.len().unwrap(), 0);
    }

    #[test]
    fn api_get_cursor_no_callback_returns_nil() {
        let rt = make_runtime();
        let t = create_table(&rt).unwrap();
        let api: Table = t.get("api").unwrap();
        let get_cursor: Function = api.get("nvim_win_get_cursor").unwrap();
        let result: Value = get_cursor.call(0).unwrap();
        assert!(matches!(result, Value::Nil));
    }

    #[test]
    fn get_frame_delta_returns_initial_zero() {
        let rt = make_runtime();
        let t = create_table(&rt).unwrap();
        let api: Table = t.get("api").unwrap();
        let get_frame_delta: Function = api.get("get_frame_delta").unwrap();
        let result: f64 = get_frame_delta.call(()).unwrap();
        assert_eq!(result, 0.0);
    }

    #[test]
    fn get_frame_delta_returns_set_value() {
        let rt = make_runtime();
        let t = create_table(&rt).unwrap();
        let api: Table = t.get("api").unwrap();
        let get_frame_delta: Function = api.get("get_frame_delta").unwrap();
        rt.set_frame_dt(16.5);
        let result: f64 = get_frame_delta.call(()).unwrap();
        assert!((result - 16.5).abs() < 1e-9);
    }

    #[test]
    fn set_frame_dt_fires_frame_event() {
        let rt = make_runtime();
        let t = create_table(&rt).unwrap();
        let on_fn: Function = t.get("on").unwrap();
        let received = std::rc::Rc::new(std::cell::RefCell::new(None::<f64>));
        let received_clone = received.clone();
        let func = rt.lua.create_function(move |_, dt: f64| {
            *received_clone.borrow_mut() = Some(dt);
            Ok(())
        }).unwrap();
        on_fn.call::<()>(("Frame", func)).unwrap();
        rt.set_frame_dt(33.3);
        let val = received.borrow();
        assert!((val.unwrap() - 33.3).abs() < 1e-9);
    }

    #[test]
    fn on_registers_event_listener() {
        let rt = make_runtime();
        let t = create_table(&rt).unwrap();
        let on_fn: Function = t.get("on").unwrap();
        let func = rt.lua.create_function(|_, ()| Ok(())).unwrap();
        assert!(on_fn.call::<()>(("TestEvent", func)).is_ok());
    }
}
