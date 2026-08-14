use crate::keymap::{parse_lua_key, LuaKeymap};
use crate::runtime::{self, Shared};
use mlua::{Function, Table, Value};
use std::rc::Rc;

/// Install the `ruster` table.
///
/// Takes the shared state by `Rc` rather than a reference to the runtime: the
/// closures below outlive this call, and the runtime is moved immediately after
/// it returns.
pub fn create_table(lua: &mlua::Lua, shared: &Rc<Shared>) -> mlua::Result<Table> {
    let t = lua.create_table()?;

    // ruster.print(...)
    let sh = shared.clone();
    let print_fn = lua.create_function(move |_, args: mlua::MultiValue| {
        let parts: Vec<String> = args.iter().map(format_value).collect();
        let msg = parts.join("\t");
        {
            sh.pending.borrow_mut().push(runtime::LuaAction::Print(msg));
        }
        Ok(())
    })?;
    t.set("print", print_fn)?;

    // ruster.defer(ms, fn) -> id     — run once, after ms
    // ruster.timer(ms, fn) -> id     — run every ms until cancelled
    // ruster.timer_stop(id) -> bool  — cancel either
    //
    // Callbacks run on the frame drain, not a thread: the runtime is `!Send`
    // and deliberately so, which means a timer can touch the editor with no
    // locking at all. Resolution is therefore one frame.
    for (name, repeats) in [("defer", false), ("timer", true)] {
        let sh = shared.clone();
        let f = lua.create_function(move |lua, (ms, func): (f64, mlua::Function)| {
            sh.timers.borrow_mut().add(lua, ms, func, repeats)
        })?;
        t.set(name, f)?;
    }
    let sh = shared.clone();
    let stop_fn = lua.create_function(move |_, id: u64| Ok(sh.timers.borrow_mut().cancel(id)))?;
    t.set("timer_stop", stop_fn)?;

    // ruster.cmd(str)
    let sh = shared.clone();
    let cmd_fn = lua.create_function(move |_, cmd: String| {
        {
            sh.pending.borrow_mut().push(runtime::LuaAction::Cmd(cmd));
        }
        Ok(())
    })?;
    t.set("cmd", cmd_fn)?;

    // ruster.keymap.set(mode, lhs, callback)
    let sh = shared.clone();
    let keymap_set =
        lua.create_function(move |lua, (mode, lhs, func): (String, String, Function)| {
            let keys: Vec<_> = lhs
                .split_inclusive('>')
                .filter(|s| !s.is_empty())
                .filter_map(|s| {
                    if s.ends_with('>') {
                        parse_lua_key(s.trim())
                    } else {
                        s.chars()
                            .map(|c| parse_lua_key(&c.to_string()))
                            .collect::<Option<Vec<_>>>()?
                            .into_iter()
                            .next()
                    }
                })
                .collect();
            if keys.is_empty() {
                return Err(mlua::Error::external("Cannot parse key sequence"));
            }
            let reg = lua.create_registry_value(func);
            match reg {
                Ok(r) => sh.keymaps.borrow_mut().push(LuaKeymap {
                    mode,
                    keys,
                    callback: r,
                }),
                Err(e) => return Err(e),
            }
            Ok(())
        })?;
    let keymap = lua.create_table()?;
    keymap.set("set", keymap_set)?;
    t.set("keymap", keymap)?;

    // ruster.g - global variable table
    let g = lua.create_table()?;
    t.set("g", g)?;

    // ruster.config - pre-created so grouped `ruster.config.general = {…}`
    // assignments in config.lua work without the user creating it first.
    let config = lua.create_table()?;
    t.set("config", config)?;

    // ruster.mode - read-only, set by App
    t.set("mode", "normal")?;

    // ruster.on(event, callback) — event registration
    let sh = shared.clone();
    let on_fn = lua.create_function(move |lua, (event, func): (String, Function)| {
        let mut events = sh.events.borrow_mut();
        events.on(lua, &event, func)
    })?;
    t.set("on", on_fn)?;

    // ruster.context_menu.add(zone, { label = ..., action = ... }) — add a row
    // to the right-click menu for a zone ("buffer", "gutter", "chrome", "tab").
    // `action` is a cmdline command, the same string `:` would take.
    let context_menu = lua.create_table()?;
    let sh = shared.clone();
    let add_fn = lua.create_function(move |_, (zone, item): (String, mlua::Table)| {
        let label: String = item.get("label")?;
        let action: String = item.get("action")?;
        if label.is_empty() || action.is_empty() {
            return Err(mlua::Error::RuntimeError(
                "context_menu.add needs a non-empty label and action".to_string(),
            ));
        }
        sh.context_menu.borrow_mut().push((zone, label, action));
        Ok(())
    })?;
    context_menu.set("add", add_fn)?;
    t.set("context_menu", context_menu)?;

    // ruster.statusline.section(pos, fn) — register a statusline component.
    // `pos` is "left" | "center" | "right"; `fn` returns a string each frame.
    let statusline = lua.create_table()?;
    let sh = shared.clone();
    let section_fn = lua.create_function(move |lua, (pos, func): (String, Function)| {
        {
            let key = lua.create_registry_value(func)?;
            sh.statusline.borrow_mut().push((pos, key));
        }
        Ok(())
    })?;
    statusline.set("section", section_fn)?;
    t.set("statusline", statusline)?;

    // ruster.ui.dialog{ title = "...", fields = { {label=, kind=, value=, options=} } }
    let ui = lua.create_table()?;
    let sh = shared.clone();
    let dialog_fn = lua.create_function(move |lua, spec: mlua::Table| {
        let title: String = spec.get::<Option<String>>("title")?.unwrap_or_default();
        let mut fields = Vec::new();
        if let Ok(list) = spec.get::<mlua::Table>("fields") {
            for f in list.sequence_values::<mlua::Table>().flatten() {
                let label: String = f.get::<Option<String>>("label")?.unwrap_or_default();
                let kind: String = f
                    .get::<Option<String>>("kind")?
                    .unwrap_or_else(|| "text".into());
                let value: String = f.get::<Option<String>>("value")?.unwrap_or_default();
                let mut options = Vec::new();
                if let Ok(opts) = f.get::<mlua::Table>("options") {
                    options.extend(opts.sequence_values::<String>().flatten());
                }
                fields.push((label, kind, value, options));
            }
        }
        // Stash on_submit here; the registry key never travels in LuaAction.
        if let Ok(cb) = spec.get::<Function>("on_submit") {
            if let Ok(key) = lua.create_registry_value(cb) {
                *sh.dialog_cb.borrow_mut() = Some(key);
            }
        }
        {
            sh.pending
                .borrow_mut()
                .push(runtime::LuaAction::Dialog { title, fields });
        }
        Ok(())
    })?;
    ui.set("dialog", dialog_fn)?;
    t.set("ui", ui)?;

    // ruster.api table
    let api = lua.create_table()?;

    // nvim_buf_get_lines(buf, start, end_opt)
    let sh = shared.clone();
    let get_lines = lua.create_function(
        move |lua, (_buf, start, end_opt): (i32, i32, Option<i32>)| {
            let mut cb = { sh.get_lines.borrow_mut() };
            let lines = match &mut *cb {
                Some(f) => f(start, end_opt),
                None => Vec::new(),
            };
            let t = lua.create_table()?;
            for (i, line) in lines.iter().enumerate() {
                t.set(i as i32 + 1, line.as_str())?;
            }
            Ok(mlua::Value::Table(t))
        },
    )?;
    api.set("nvim_buf_get_lines", get_lines)?;

    // nvim_buf_set_lines(buf, start, end, lines)
    let sh = shared.clone();
    let set_lines = lua.create_function(
        move |_, (_buf, start, end, lines): (i32, i32, i32, mlua::Value)| {
            let lines_vec: Vec<String> = match lines {
                mlua::Value::String(s) => {
                    vec![s.to_str().map(|s| s.to_string()).unwrap_or_default()]
                }
                mlua::Value::Table(t) => {
                    let mut v = Vec::new();
                    for i in 1..=t.len()? {
                        if let Ok(s) = t.get::<String>(i) {
                            v.push(s);
                        }
                    }
                    v
                }
                _ => return Err(mlua::Error::external("set_lines expects string or table")),
            };
            let mut cb = { sh.set_lines.borrow_mut() };
            if let Some(f) = cb.as_mut() {
                f(start, end, lines_vec);
            }
            Ok(())
        },
    )?;
    api.set("nvim_buf_set_lines", set_lines)?;

    // nvim_win_get_cursor(win)
    let sh = shared.clone();
    let get_cursor = lua.create_function(move |lua, _win: i32| {
        let mut cb = { sh.get_cursor.borrow_mut() };
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
    let sh = shared.clone();
    let set_cursor = lua.create_function(move |_, (_win, pos): (i32, mlua::Table)| {
        let row: i32 = pos.get("row").unwrap_or(0);
        let col: i32 = pos.get("col").unwrap_or(0);
        let mut cb = { sh.set_cursor.borrow_mut() };
        if let Some(f) = cb.as_mut() {
            f(row, col);
        }
        Ok(())
    })?;
    api.set("nvim_win_set_cursor", set_cursor)?;

    // nvim_list_bufs() -> { buf_id, ... }
    let sh = shared.clone();
    let list_bufs = lua.create_function(move |lua, ()| {
        let ids = {
            let mut cb = sh.window_cb.borrow_mut();
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
    let sh = shared.clone();
    let list_wins = lua.create_function(move |lua, ()| {
        let ids = {
            let mut cb = sh.window_cb.borrow_mut();
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
    let sh = shared.clone();
    let get_current_win = lua.create_function(move |_, ()| {
        let id = {
            let mut cb = sh.window_cb.borrow_mut();
            cb.as_mut().map(|c| (c.current_win)()).unwrap_or(0)
        };
        Ok(id)
    })?;
    api.set("nvim_get_current_win", get_current_win)?;

    // nvim_set_current_win(win_id)
    let sh = shared.clone();
    let set_current_win = lua.create_function(move |_, win: i32| {
        {
            let mut cb = sh.window_cb.borrow_mut();
            if let Some(c) = cb.as_mut() {
                (c.set_current_win)(win);
            }
        }
        Ok(())
    })?;
    api.set("nvim_set_current_win", set_current_win)?;

    // nvim_win_get_buf(win_id) -> buf_id (0 when unavailable)
    let sh = shared.clone();
    let win_get_buf = lua.create_function(move |_, win: i32| {
        let id = {
            let mut cb = sh.window_cb.borrow_mut();
            cb.as_mut().map(|c| (c.win_get_buf)(win)).unwrap_or(0)
        };
        Ok(id)
    })?;
    api.set("nvim_win_get_buf", win_get_buf)?;

    // nvim_win_set_buf(win_id, buf_id)
    let sh = shared.clone();
    let win_set_buf = lua.create_function(move |_, (win, buf): (i32, i32)| {
        {
            let mut cb = sh.window_cb.borrow_mut();
            if let Some(c) = cb.as_mut() {
                (c.win_set_buf)(win, buf);
            }
        }
        Ok(())
    })?;
    api.set("nvim_win_set_buf", win_set_buf)?;

    // nvim_open_win(vertical) -> new win_id (0 when unavailable)
    let sh = shared.clone();
    let open_win = lua.create_function(move |_, vertical: Option<bool>| {
        let id = {
            let mut cb = sh.window_cb.borrow_mut();
            cb.as_mut()
                .map(|c| (c.open_win)(vertical.unwrap_or(false)))
                .unwrap_or(0)
        };
        Ok(id)
    })?;
    api.set("nvim_open_win", open_win)?;

    // nvim_win_close(win_id)
    let sh = shared.clone();
    let win_close = lua.create_function(move |_, win: i32| {
        {
            let mut cb = sh.window_cb.borrow_mut();
            if let Some(c) = cb.as_mut() {
                (c.close_win)(win);
            }
        }
        Ok(())
    })?;
    api.set("nvim_win_close", win_close)?;

    // ruster.api.get_frame_delta()
    let sh = shared.clone();
    let get_frame_delta = lua.create_function(move |_, ()| {
        let dt = sh.current_dt.borrow();
        Ok(*dt)
    })?;
    api.set("get_frame_delta", get_frame_delta)?;

    // ruster.api.notify(text) — Info level
    let sh = shared.clone();
    let notify_fn = lua.create_function(move |_, text: String| {
        {
            sh.pending
                .borrow_mut()
                .push(runtime::LuaAction::Notify(0, text));
        }
        Ok(())
    })?;
    api.set("notify", notify_fn)?;

    // ruster.api.notify_success(text)
    let sh = shared.clone();
    let notify_success = lua.create_function(move |_, text: String| {
        {
            sh.pending
                .borrow_mut()
                .push(runtime::LuaAction::Notify(1, text));
        }
        Ok(())
    })?;
    api.set("notify_success", notify_success)?;

    // ruster.api.notify_warn(text)
    let sh = shared.clone();
    let notify_warn = lua.create_function(move |_, text: String| {
        {
            sh.pending
                .borrow_mut()
                .push(runtime::LuaAction::Notify(2, text));
        }
        Ok(())
    })?;
    api.set("notify_warn", notify_warn)?;

    // ruster.api.notify_error(text)
    let sh = shared.clone();
    let notify_error = lua.create_function(move |_, text: String| {
        {
            sh.pending
                .borrow_mut()
                .push(runtime::LuaAction::Notify(3, text));
        }
        Ok(())
    })?;
    api.set("notify_error", notify_error)?;

    // ruster.api.notify_with({ text, level, timeout })
    let sh = shared.clone();
    let notify_with = lua.create_function(move |_, opts: mlua::Table| {
        let text: String = opts.get("text").unwrap_or_default();
        let level_str: String = opts.get("level").unwrap_or_else(|_| "info".to_string());
        let level = match level_str.as_str() {
            "success" => 1,
            "warning" => 2,
            "error" => 3,
            _ => 0,
        };
        {
            sh.pending
                .borrow_mut()
                .push(runtime::LuaAction::Notify(level, text));
        }
        Ok(())
    })?;
    api.set("notify_with", notify_with)?;

    // Read-only introspection. Small on purpose: a statusline needs the path,
    // the filetype, the diagnostics and the git state, and every getter beyond
    // that is surface which has to keep working forever.
    //
    // Each returns an empty/zero value rather than erroring when the app has
    // not installed the callbacks — a plugin loaded before the editor finished
    // starting should degrade, not blow up.
    let sh = shared.clone();
    let buf_path = lua.create_function(move |_, ()| {
        let mut cb = sh.query_cb.borrow_mut();
        Ok(cb.as_mut().map(|c| (c.buf_path)()).unwrap_or_default())
    })?;
    api.set("buf_path", buf_path)?;

    let sh = shared.clone();
    let filetype = lua.create_function(move |_, ()| {
        let mut cb = sh.query_cb.borrow_mut();
        Ok(cb.as_mut().map(|c| (c.filetype)()).unwrap_or_default())
    })?;
    api.set("filetype", filetype)?;

    let sh = shared.clone();
    let diagnostics = lua.create_function(move |lua, ()| {
        let items = {
            let mut cb = sh.query_cb.borrow_mut();
            cb.as_mut().map(|c| (c.diagnostics)()).unwrap_or_default()
        };
        let out = lua.create_table()?;
        for (i, d) in items.iter().enumerate() {
            let row = lua.create_table()?;
            row.set("line", d.line)?;
            row.set("col", d.col)?;
            row.set("severity", d.severity)?;
            row.set("message", d.message.clone())?;
            out.set(i + 1, row)?;
        }
        Ok(out)
    })?;
    api.set("diagnostics", diagnostics)?;

    let sh = shared.clone();
    let git_status = lua.create_function(move |lua, ()| {
        let (branch, staged, unstaged) = {
            let mut cb = sh.query_cb.borrow_mut();
            cb.as_mut().map(|c| (c.git_status)()).unwrap_or_default()
        };
        let out = lua.create_table()?;
        out.set("branch", branch)?;
        out.set("staged", staged)?;
        out.set("unstaged", unstaged)?;
        Ok(out)
    })?;
    api.set("git_status", git_status)?;

    t.set("api", api)?;

    Ok(t)
}

fn format_value(v: &Value) -> String {
    match v {
        Value::Nil => "nil".into(),
        Value::String(s) => s
            .to_str()
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "?".to_string()),
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

    /// The regression test for the dangling-pointer bug.
    ///
    /// The existing tests below build their own table with `create_table`, on a
    /// runtime that is never moved afterwards — so they exercise a *different*
    /// table from the `ruster` global that `init.lua` actually calls. This one
    /// uses the global, on a runtime that has been returned from `new()` (and
    /// therefore moved) and then moved again, which is exactly what broke:
    /// every closure held a `*const LuaRuntime` into a dead stack slot.
    #[test]
    fn the_installed_global_survives_the_runtime_being_moved() {
        let rt = make_runtime();
        // Move it again for good measure — a pointer to the original slot would
        // now be doubly wrong.
        let rt = Box::new(rt);

        rt.lua
            .load(r#"ruster.print("from init.lua")"#)
            .exec()
            .expect("print must not crash");
        rt.lua
            .load(r#"ruster.cmd(":w")"#)
            .exec()
            .expect("cmd must not crash");
        rt.lua
            .load(r#"ruster.keymap.set("n", "<F8>", function() end)"#)
            .exec()
            .expect("keymap.set must not crash");
        rt.lua
            .load(r#"ruster.ui.dialog{ title = "T", fields = { { label = "A", kind = "toggle", value = "on" } } }"#)
            .exec()
            .expect("ui.dialog must not crash");

        let actions = rt.drain_actions();
        assert_eq!(
            actions.len(),
            3,
            "print, cmd and dialog all queued: {actions:?}"
        );
        assert!(matches!(&actions[0], runtime::LuaAction::Print(m) if m == "from init.lua"));
        assert!(matches!(&actions[1], runtime::LuaAction::Cmd(m) if m == ":w"));
        assert!(matches!(
            &actions[2],
            runtime::LuaAction::Dialog { title, fields } if title == "T" && fields.len() == 1
        ));
    }

    /// A dialog a plugin cannot get answers from is a display, not an API.
    #[test]
    fn dialog_on_submit_receives_the_values_and_the_button() {
        let rt = make_runtime();
        rt.lua
            .load(
                r#"
                _G.got = nil
                ruster.ui.dialog{
                  title = "T",
                  fields = { { label = "Force", kind = "toggle", value = "on" } },
                  on_submit = function(values, button)
                    _G.got = { force = values.Force, button = button }
                  end,
                }
                "#,
            )
            .exec()
            .unwrap();

        rt.fire_dialog_submit(&[("Force".into(), "off".into())], Some("OK"));

        let got: Table = rt.lua.globals().get("got").unwrap();
        assert_eq!(got.get::<String>("force").unwrap(), "off");
        assert_eq!(got.get::<String>("button").unwrap(), "OK");
    }

    /// Cancelling must not call it — and must not leave it armed for the next
    /// dialog either.
    #[test]
    fn a_cancelled_dialog_never_calls_on_submit() {
        let rt = make_runtime();
        rt.lua
            .load(
                r#"
                _G.calls = 0
                ruster.ui.dialog{ title = "T", fields = {}, on_submit = function() _G.calls = _G.calls + 1 end }
                "#,
            )
            .exec()
            .unwrap();

        rt.discard_dialog_callback();
        rt.fire_dialog_submit(&[], None);
        let calls: i64 = rt.lua.globals().get("calls").unwrap();
        assert_eq!(calls, 0, "the discarded callback must not fire");
    }

    #[test]
    fn a_dialog_without_on_submit_is_fine() {
        let rt = make_runtime();
        rt.lua
            .load(r#"ruster.ui.dialog{ title = "T", fields = {} }"#)
            .exec()
            .unwrap();
        rt.fire_dialog_submit(&[], None); // must not panic
    }

    #[test]
    fn print_queues_action() {
        let rt = make_runtime();
        let t: Table = rt.lua.globals().get("ruster").unwrap();
        let print_fn: Function = t.get("print").unwrap();
        print_fn.call::<()>("hello").unwrap();
        let actions = rt.drain_actions();
        assert!(matches!(actions.as_slice(), [runtime::LuaAction::Print(m)] if m == "hello"));
    }

    #[test]
    fn cmd_queues_action() {
        let rt = make_runtime();
        let t: Table = rt.lua.globals().get("ruster").unwrap();
        let cmd_fn: Function = t.get("cmd").unwrap();
        cmd_fn.call::<()>(":w").unwrap();
        let actions = rt.drain_actions();
        assert!(matches!(actions.as_slice(), [runtime::LuaAction::Cmd(m)] if m == ":w"));
    }

    #[test]
    fn nvim_list_bufs_no_callback_returns_empty() {
        let rt = make_runtime();
        let t: Table = rt.lua.globals().get("ruster").unwrap();
        let api: Table = t.get("api").unwrap();
        let list_bufs: Function = api.get("nvim_list_bufs").unwrap();
        let result: Table = list_bufs.call(()).unwrap();
        assert_eq!(result.len().unwrap(), 0);
    }

    #[test]
    fn nvim_open_win_no_callback_returns_zero() {
        let rt = make_runtime();
        let t: Table = rt.lua.globals().get("ruster").unwrap();
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
        let t: Table = rt.lua.globals().get("ruster").unwrap();
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
        let t: Table = rt.lua.globals().get("ruster").unwrap();
        let statusline: Table = t.get("statusline").unwrap();
        let section: Function = statusline.get("section").unwrap();
        let f = rt
            .lua
            .create_function(|_, ()| Ok("git:main".to_string()))
            .unwrap();
        section.call::<()>(("right", f)).unwrap();
        assert_eq!(
            rt.statusline_sections("right"),
            vec!["git:main".to_string()]
        );
        assert!(rt.statusline_sections("left").is_empty());
    }

    #[test]
    fn api_get_lines_no_callback_returns_empty() {
        let rt = make_runtime();
        let t: Table = rt.lua.globals().get("ruster").unwrap();
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
        let t: Table = rt.lua.globals().get("ruster").unwrap();
        let api: Table = t.get("api").unwrap();
        let get_cursor: Function = api.get("nvim_win_get_cursor").unwrap();
        let result: Value = get_cursor.call(0).unwrap();
        assert!(matches!(result, Value::Nil));
    }

    #[test]
    fn get_frame_delta_returns_initial_zero() {
        let rt = make_runtime();
        let t: Table = rt.lua.globals().get("ruster").unwrap();
        let api: Table = t.get("api").unwrap();
        let get_frame_delta: Function = api.get("get_frame_delta").unwrap();
        let result: f64 = get_frame_delta.call(()).unwrap();
        assert_eq!(result, 0.0);
    }

    #[test]
    fn get_frame_delta_returns_set_value() {
        let rt = make_runtime();
        let t: Table = rt.lua.globals().get("ruster").unwrap();
        let api: Table = t.get("api").unwrap();
        let get_frame_delta: Function = api.get("get_frame_delta").unwrap();
        rt.set_frame_dt(16.5);
        let result: f64 = get_frame_delta.call(()).unwrap();
        assert!((result - 16.5).abs() < 1e-9);
    }

    #[test]
    fn set_frame_dt_fires_frame_event() {
        let rt = make_runtime();
        let t: Table = rt.lua.globals().get("ruster").unwrap();
        let on_fn: Function = t.get("on").unwrap();
        let received = std::rc::Rc::new(std::cell::RefCell::new(None::<f64>));
        let received_clone = received.clone();
        let func = rt
            .lua
            .create_function(move |_, dt: f64| {
                *received_clone.borrow_mut() = Some(dt);
                Ok(())
            })
            .unwrap();
        on_fn.call::<()>(("Frame", func)).unwrap();
        rt.set_frame_dt(33.3);
        let val = received.borrow();
        assert!((val.unwrap() - 33.3).abs() < 1e-9);
    }

    #[test]
    fn on_registers_event_listener() {
        let rt = make_runtime();
        let t: Table = rt.lua.globals().get("ruster").unwrap();
        let on_fn: Function = t.get("on").unwrap();
        let func = rt.lua.create_function(|_, ()| Ok(())).unwrap();
        assert!(on_fn.call::<()>(("TestEvent", func)).is_ok());
    }
}
