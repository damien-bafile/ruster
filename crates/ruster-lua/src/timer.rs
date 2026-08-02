//! Deferred and repeating callbacks, drained on the frame tick.
//!
//! Not threads. The Lua runtime is `!Send` on purpose — every callback runs on
//! the same frame drain as `ruster.cmd` and keymaps, so a plugin can touch the
//! editor from a timer without any locking, and a slow callback shows up as a
//! slow frame rather than as a race.
//!
//! The cost of that choice is honest: resolution is one frame, and a callback
//! that blocks blocks the editor. Both are the same deal keymaps already have.

use mlua::{Function, Lua, RegistryKey};

/// A handle a plugin can cancel. Ids are never reused within a session, so
/// cancelling an already-fired one-shot is a no-op rather than a mis-cancel.
pub type TimerId = u64;

struct Entry {
    id: TimerId,
    /// Milliseconds until this fires next.
    remaining_ms: f64,
    /// `Some` for a repeating timer: the period to reload after firing.
    interval_ms: Option<f64>,
    func: RegistryKey,
}

#[derive(Default)]
pub struct Timers {
    next_id: TimerId,
    entries: Vec<Entry>,
}

impl Timers {
    pub fn new() -> Self {
        Timers { next_id: 1, entries: Vec::new() }
    }

    /// Register a callback. `interval_ms` set makes it repeat.
    pub fn add(
        &mut self,
        lua: &Lua,
        delay_ms: f64,
        func: Function,
        repeat: bool,
    ) -> mlua::Result<TimerId> {
        let key = lua.create_registry_value(func)?;
        let id = self.next_id;
        self.next_id += 1;
        // A negative or NaN delay means "as soon as possible", not "never" —
        // `ruster.defer(0, f)` is a legitimate way to run something after the
        // current frame settles.
        let delay = if delay_ms.is_finite() { delay_ms.max(0.0) } else { 0.0 };
        self.entries.push(Entry {
            id,
            remaining_ms: delay,
            interval_ms: repeat.then_some(delay.max(1.0)),
            func: key,
        });
        Ok(id)
    }

    /// Cancel by id. Returns whether anything was cancelled.
    pub fn cancel(&mut self, id: TimerId) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        self.entries.len() != before
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Advance every timer by `dt_ms` and return the callbacks now due.
    ///
    /// Returns resolved `Function`s rather than registry keys, for two reasons.
    /// A repeating timer keeps its key registered, so it has nothing to hand
    /// over — only a fresh reference. And the caller must be able to invoke
    /// without holding a borrow of this, because a callback is free to call
    /// `ruster.defer` again from inside itself.
    ///
    /// One-shots are removed and unregistered here; repeating timers reload.
    /// A repeating timer fires at most once per drain however far behind it is:
    /// catching up by firing five times in one frame is never what a debounce
    /// wanted, and it would let one slow frame cascade into several.
    pub fn take_due(&mut self, lua: &Lua, dt_ms: f64) -> Vec<Function> {
        let dt = if dt_ms.is_finite() { dt_ms.max(0.0) } else { 0.0 };
        let mut due = Vec::new();
        let mut i = 0;
        while i < self.entries.len() {
            self.entries[i].remaining_ms -= dt;
            if self.entries[i].remaining_ms > 0.0 {
                i += 1;
                continue;
            }
            let func: Option<Function> = lua.registry_value(&self.entries[i].func).ok();
            match self.entries[i].interval_ms {
                Some(period) => {
                    self.entries[i].remaining_ms = period;
                    i += 1;
                }
                None => {
                    let e = self.entries.remove(i);
                    // Registry values are not collected until removed, so a
                    // long session of `defer` calls would otherwise leak one
                    // Lua reference each.
                    let _ = lua.remove_registry_value(e.func);
                }
            }
            if let Some(f) = func {
                due.push(f);
            }
        }
        due
    }
}
