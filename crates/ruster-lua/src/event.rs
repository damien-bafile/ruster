use std::collections::HashMap;
use mlua::{Function, Lua, MultiValue, RegistryKey};

pub struct EventBus {
    handlers: HashMap<String, Vec<RegistryKey>>,
}

impl EventBus {
    pub fn new() -> Self {
        EventBus { handlers: HashMap::new() }
    }

    pub fn on(&mut self, lua: &Lua, event: &str, func: Function) -> mlua::Result<()> {
        let key = lua.create_registry_value(func)?;
        self.handlers.entry(event.to_string()).or_default().push(key);
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

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
        let func = lua.create_function(move |_, ()| {
            *c.borrow_mut() = true;
            Ok(())
        }).unwrap();
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
        let f1 = lua.create_function(move |_, ()| {
            *c1.borrow_mut() += 1;
            Ok(())
        }).unwrap();
        let f2 = lua.create_function(move |_, ()| {
            *c2.borrow_mut() += 1;
            Ok(())
        }).unwrap();
        bus.on(&lua, "Multi", f1).unwrap();
        bus.on(&lua, "Multi", f2).unwrap();
        bus.emit(&lua, "Multi", &[]);
        assert_eq!(*count.borrow(), 2);
    }
}
