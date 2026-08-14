use mlua::{Function, Lua, MultiValue, RegistryKey};
use std::collections::HashMap;

pub struct EventBus {
    handlers: HashMap<String, Vec<RegistryKey>>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        EventBus {
            handlers: HashMap::new(),
        }
    }

    pub fn on(&mut self, lua: &Lua, event: &str, func: Function) -> mlua::Result<()> {
        let key = lua.create_registry_value(func)?;
        self.handlers
            .entry(event.to_string())
            .or_default()
            .push(key);
        Ok(())
    }

    pub fn emit(&self, lua: &Lua, event: &str, args: &[mlua::Value]) {
        if let Some(handlers) = self.handlers.get(event) {
            for key in handlers {
                if let Ok(func) = lua.registry_value::<Function>(key) {
                    let _ = func.call::<()>(MultiValue::from_vec(args.to_vec()));
                }
            }
        }
    }

    /// Emit, letting a handler consume the event by returning `true`.
    ///
    /// Every handler runs even after one consumes: they are independent
    /// subscribers, and a plugin that registered second should not be silenced
    /// by one that registered first. What consuming decides is only whether the
    /// *built-in* behaviour also runs.
    ///
    /// A handler that throws is skipped rather than being treated as consuming.
    /// A broken plugin should not be able to make the editor stop responding to
    /// the mouse.
    pub fn emit_consuming(&self, lua: &Lua, event: &str, args: &[mlua::Value]) -> bool {
        let mut consumed = false;
        if let Some(handlers) = self.handlers.get(event) {
            for key in handlers {
                if let Ok(func) = lua.registry_value::<Function>(key) {
                    match func.call::<mlua::Value>(MultiValue::from_vec(args.to_vec())) {
                        Ok(v) => consumed |= matches!(v, mlua::Value::Boolean(true)),
                        Err(_) => continue,
                    }
                }
            }
        }
        consumed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A handler returning true cancels the built-in behaviour.
    #[test]
    fn a_handler_returning_true_consumes_the_event() {
        let lua = Lua::new();
        let mut bus = EventBus::new();
        let f = lua
            .load("function() return true end")
            .eval::<Function>()
            .unwrap();
        bus.on(&lua, "mouse_down", f).unwrap();
        assert!(bus.emit_consuming(&lua, "mouse_down", &[]));
    }

    #[test]
    fn a_handler_returning_nothing_passes_the_event_through() {
        let lua = Lua::new();
        let mut bus = EventBus::new();
        let f = lua.load("function() end").eval::<Function>().unwrap();
        bus.on(&lua, "mouse_down", f).unwrap();
        assert!(!bus.emit_consuming(&lua, "mouse_down", &[]));
    }

    /// A broken plugin must not be able to stop the mouse from working, nor
    /// silence the handlers registered after it.
    #[test]
    fn a_throwing_handler_neither_consumes_nor_blocks_the_rest() {
        let lua = Lua::new();
        let mut bus = EventBus::new();
        let boom = lua
            .load("function() error('boom') end")
            .eval::<Function>()
            .unwrap();
        bus.on(&lua, "mouse_down", boom).unwrap();
        assert!(
            !bus.emit_consuming(&lua, "mouse_down", &[]),
            "a throw is not a consume"
        );

        lua.load("ran = false").exec().unwrap();
        let after = lua
            .load("function() ran = true end")
            .eval::<Function>()
            .unwrap();
        bus.on(&lua, "mouse_down", after).unwrap();
        bus.emit_consuming(&lua, "mouse_down", &[]);
        assert!(
            lua.globals().get::<bool>("ran").unwrap(),
            "the handler after the broken one still ran"
        );
    }

    /// Consuming does not silence the other subscribers — they are independent.
    #[test]
    fn every_handler_runs_even_after_one_consumes() {
        let lua = Lua::new();
        let mut bus = EventBus::new();
        lua.load("count = 0").exec().unwrap();
        for src in [
            "function() count = count + 1; return true end",
            "function() count = count + 1 end",
        ] {
            let f = lua.load(src).eval::<Function>().unwrap();
            bus.on(&lua, "mouse_down", f).unwrap();
        }
        assert!(bus.emit_consuming(&lua, "mouse_down", &[]));
        assert_eq!(lua.globals().get::<i64>("count").unwrap(), 2);
    }

    #[test]
    fn event_bus_new_is_empty() {
        let bus = EventBus::new();
        assert!(bus.handlers.is_empty());
    }

    #[test]
    fn event_bus_on_registers_listener() {
        let lua = Lua::new();
        let mut bus = EventBus::new();
        let func = lua.create_function(|_, ()| Ok(())).unwrap();
        assert!(bus.on(&lua, "TestEvent", func).is_ok());
        assert_eq!(bus.handlers.len(), 1);
        assert_eq!(bus.handlers.get("TestEvent").unwrap().len(), 1);
    }

    #[test]
    fn event_bus_emit_calls_listener() {
        let lua = Lua::new();
        let mut bus = EventBus::new();
        let called = Rc::new(RefCell::new(false));
        let c = called.clone();
        let func = lua
            .create_function(move |_, ()| {
                *c.borrow_mut() = true;
                Ok(())
            })
            .unwrap();
        bus.on(&lua, "TestEvent", func).unwrap();
        bus.emit(&lua, "TestEvent", &[]);
        assert!(*called.borrow());
    }

    #[test]
    fn event_bus_emit_unregistered_is_noop() {
        let lua = Lua::new();
        let bus = EventBus::new();
        bus.emit(&lua, "NonExistent", &[]);
    }

    #[test]
    fn event_bus_multiple_listeners_same_event() {
        let lua = Lua::new();
        let mut bus = EventBus::new();
        let count = Rc::new(RefCell::new(0));
        let c1 = count.clone();
        let c2 = count.clone();
        let f1 = lua
            .create_function(move |_, ()| {
                *c1.borrow_mut() += 1;
                Ok(())
            })
            .unwrap();
        let f2 = lua
            .create_function(move |_, ()| {
                *c2.borrow_mut() += 1;
                Ok(())
            })
            .unwrap();
        bus.on(&lua, "Multi", f1).unwrap();
        bus.on(&lua, "Multi", f2).unwrap();
        bus.emit(&lua, "Multi", &[]);
        assert_eq!(*count.borrow(), 2);
    }
}
