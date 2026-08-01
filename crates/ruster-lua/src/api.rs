use mlua::{Function, Table, Value};
use crate::runtime::{self, LuaRuntime};
use crate::keymap::{parse_lua_key, LuaKeymap};

pub fn create_table(runtime: &LuaRuntime) -> mlua::Result<Table> {
    let t = runtime.lua.create_table()?;

    // ruster.print(...)
    let rt = runtime as *const LuaRuntime;
    let print_fn = runtime.lua.create_function(move |_, args: mlua::MultiValue| {
        let parts: Vec<String> = args.iter().map(format_value).collect();
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
        let keys: Vec<_> = lhs.split_inclusive('>')
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

    // ruster.config - pre-created so grouped `ruster.config.general = {…}`
    // assignments in config.lua work without the user creating it first.
    let config = runtime.lua.create_table()?;
    t.set("config", config)?;

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

    // ruster.ui.dialog{ title = "...", fields = { {label=, kind=, value=, options=} } }
    //
    // NOTE: unusable from a real init.lua until the runtime's dangling-pointer
    // bug is fixed — `LuaRuntime::new` hands `&runtime` to these closures as a
    // raw pointer and then moves the struct out, so every `(*rt)` here reads
    // freed memory. The same flaw affects `print`, `cmd` and `keymap`; this
    // follows the existing idiom rather than inventing a second one, and both
    // are fixed together.
    let ui = runtime.lua.create_table()?;
    let rt = runtime as *const LuaRuntime;
    let dialog_fn = runtime.lua.create_function(move |_, spec: mlua::Table| {
        let title: String = spec.get::<Option<String>>("title")?.unwrap_or_default();
        let mut fields = Vec::new();
        if let Ok(list) = spec.get::<mlua::Table>("fields") {
            for f in list.sequence_values::<mlua::Table>().flatten() {
                let label: String = f.get::<Option<String>>("label")?.unwrap_or_default();
                let kind: String =
                    f.get::<Option<String>>("kind")?.unwrap_or_else(|| "text".into());
                let value: String = f.get::<Option<String>>("value")?.unwrap_or_default();
                let mut options = Vec::new();
                if let Ok(opts) = f.get::<mlua::Table>("options") {
                    options.extend(opts.sequence_values::<String>().flatten());
                }
                fields.push((label, kind, value, options));
            }
        }
        unsafe {
            (*rt).pending.borrow_mut().push(runtime::LuaAction::Dialog { title, fields });
        }
        Ok(())
    })?;
    ui.set("dialog", dialog_fn)?;
    t.set("ui", ui)?;

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

    // nvim_list_bufs() -> { buf_id, ... }
    let rt = runtime as *const LuaRuntime;
    let list_bufs = runtime.lua.create_function(move |lua, ()| {
        let ids = unsafe {
            let mut cb = (*rt).window_cb.borrow_mut();
            cb.as_mut().map(|c| (c.list_bufs)()).unwrap_or_default()
        };
        let t = lua.create_table()?;
        for (i, id) in ids.iter().enumerate() {
            t.set(i as i32 + 1, *id)?;
        }
        Ok(t)
    })?;
    api.set("nvim_list_bufs", list_bufs)?;

    // nvim_list_wins() -> { win_id, ... }
    let rt = runtime as *const LuaRuntime;
    let list_wins = runtime.lua.create_function(move |lua, ()| {
        let ids = unsafe {
            let mut cb = (*rt).window_cb.borrow_mut();
            cb.as_mut().map(|c| (c.list_wins)()).unwrap_or_default()
        };
        let t = lua.create_table()?;
        for (i, id) in ids.iter().enumerate() {
            t.set(i as i32 + 1, *id)?;
        }
        Ok(t)
    })?;
    api.set("nvim_list_wins", list_wins)?;

    // nvim_get_current_win() -> win_id (0 when unavailable)
    let rt = runtime as *const LuaRuntime;
    let get_current_win = runtime.lua.create_function(move |_, ()| {
        let id = unsafe {
            let mut cb = (*rt).window_cb.borrow_mut();
            cb.as_mut().map(|c| (c.current_win)()).unwrap_or(0)
        };
        Ok(id)
    })?;
    api.set("nvim_get_current_win", get_current_win)?;

    // nvim_set_current_win(win_id)
    let rt = runtime as *const LuaRuntime;
    let set_current_win = runtime.lua.create_function(move |_, win: i32| {
        unsafe {
            let mut cb = (*rt).window_cb.borrow_mut();
            if let Some(c) = cb.as_mut() {
                (c.set_current_win)(win);
            }
        }
        Ok(())
    })?;
    api.set("nvim_set_current_win", set_current_win)?;

    // nvim_win_get_buf(win_id) -> buf_id (0 when unavailable)
    let rt = runtime as *const LuaRuntime;
    let win_get_buf = runtime.lua.create_function(move |_, win: i32| {
        let id = unsafe {
            let mut cb = (*rt).window_cb.borrow_mut();
            cb.as_mut().map(|c| (c.win_get_buf)(win)).unwrap_or(0)
        };
        Ok(id)
    })?;
    api.set("nvim_win_get_buf", win_get_buf)?;

    // nvim_win_set_buf(win_id, buf_id)
    let rt = runtime as *const LuaRuntime;
    let win_set_buf = runtime.lua.create_function(move |_, (win, buf): (i32, i32)| {
        unsafe {
            let mut cb = (*rt).window_cb.borrow_mut();
            if let Some(c) = cb.as_mut() {
                (c.win_set_buf)(win, buf);
            }
        }
        Ok(())
    })?;
    api.set("nvim_win_set_buf", win_set_buf)?;

    // nvim_open_win(vertical) -> new win_id (0 when unavailable)
    let rt = runtime as *const LuaRuntime;
    let open_win = runtime.lua.create_function(move |_, vertical: Option<bool>| {
        let id = unsafe {
            let mut cb = (*rt).window_cb.borrow_mut();
            cb.as_mut().map(|c| (c.open_win)(vertical.unwrap_or(false))).unwrap_or(0)
        };
        Ok(id)
    })?;
    api.set("nvim_open_win", open_win)?;

    // nvim_win_close(win_id)
    let rt = runtime as *const LuaRuntime;
    let win_close = runtime.lua.create_function(move |_, win: i32| {
        unsafe {
            let mut cb = (*rt).window_cb.borrow_mut();
            if let Some(c) = cb.as_mut() {
                (c.close_win)(win);
            }
        }
        Ok(())
    })?;
    api.set("nvim_win_close", win_close)?;

    // ruster.api.get_frame_delta()
    let rt = runtime as *const LuaRuntime;
    let get_frame_delta = runtime.lua.create_function(move |_, ()| {
        unsafe {
            let dt = (*rt).current_dt.borrow();
            Ok(*dt)
        }
    })?;
    api.set("get_frame_delta", get_frame_delta)?;

    // ruster.api.notify(text) — Info level
    let rt = runtime as *const LuaRuntime;
    let notify_fn = runtime.lua.create_function(move |_, text: String| {
        unsafe { (*rt).pending.borrow_mut().push(runtime::LuaAction::Notify(0, text)); }
        Ok(())
    })?;
    api.set("notify", notify_fn)?;

    // ruster.api.notify_success(text)
    let rt = runtime as *const LuaRuntime;
    let notify_success = runtime.lua.create_function(move |_, text: String| {
        unsafe { (*rt).pending.borrow_mut().push(runtime::LuaAction::Notify(1, text)); }
        Ok(())
    })?;
    api.set("notify_success", notify_success)?;

    // ruster.api.notify_warn(text)
    let rt = runtime as *const LuaRuntime;
    let notify_warn = runtime.lua.create_function(move |_, text: String| {
        unsafe { (*rt).pending.borrow_mut().push(runtime::LuaAction::Notify(2, text)); }
        Ok(())
    })?;
    api.set("notify_warn", notify_warn)?;

    // ruster.api.notify_error(text)
    let rt = runtime as *const LuaRuntime;
    let notify_error = runtime.lua.create_function(move |_, text: String| {
        unsafe { (*rt).pending.borrow_mut().push(runtime::LuaAction::Notify(3, text)); }
        Ok(())
    })?;
    api.set("notify_error", notify_error)?;

    // ruster.api.notify_with({ text, level, timeout })
    let rt = runtime as *const LuaRuntime;
    let notify_with = runtime.lua.create_function(move |_, opts: mlua::Table| {
        let text: String = opts.get("text").unwrap_or_default();
        let level_str: String = opts.get("level").unwrap_or_else(|_| "info".to_string());
        let level = match level_str.as_str() {
            "success" => 1, "warning" => 2, "error" => 3,
            _ => 0,
        };
        unsafe { (*rt).pending.borrow_mut().push(runtime::LuaAction::Notify(level, text)); }
        Ok(())
    })?;
    api.set("notify_with", notify_with)?;

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
    fn nvim_list_bufs_no_callback_returns_empty() {
        let rt = make_runtime();
        let t = create_table(&rt).unwrap();
        let api: Table = t.get("api").unwrap();
        let list_bufs: Function = api.get("nvim_list_bufs").unwrap();
        let result: Table = list_bufs.call(()).unwrap();
        assert_eq!(result.len().unwrap(), 0);
    }

    #[test]
    fn nvim_open_win_no_callback_returns_zero() {
        let rt = make_runtime();
        let t = create_table(&rt).unwrap();
        let api: Table = t.get("api").unwrap();
        let open_win: Function = api.get("nvim_open_win").unwrap();
        let id: i32 = open_win.call(true).unwrap();
        assert_eq!(id, 0);
    }

    #[test]
    fn window_callbacks_drive_list_bufs_and_open_win() {
        use crate::runtime::WindowCallbacks;
        let rt = make_runtime();
        rt.set_window_callbacks(WindowCallbacks {
            list_bufs: Box::new(|| vec![1, 2, 3]),
            list_wins: Box::new(|| vec![1]),
            current_win: Box::new(|| 1),
            set_current_win: Box::new(|_| {}),
            win_get_buf: Box::new(|_| 7),
            win_set_buf: Box::new(|_, _| {}),
            open_win: Box::new(|_vertical| 42),
            close_win: Box::new(|_| {}),
        });
        let t = create_table(&rt).unwrap();
        let api: Table = t.get("api").unwrap();
        let list_bufs: Function = api.get("nvim_list_bufs").unwrap();
        let bufs: Table = list_bufs.call(()).unwrap();
        assert_eq!(bufs.len().unwrap(), 3);
        let open_win: Function = api.get("nvim_open_win").unwrap();
        let id: i32 = open_win.call(true).unwrap();
        assert_eq!(id, 42);
    }

    #[test]
    fn config_has_default_timeoutlen() {
        let rt = make_runtime();
        assert_eq!(rt.config().timeoutlen, 300);
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
