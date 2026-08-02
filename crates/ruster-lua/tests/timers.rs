//! `ruster.defer` and `ruster.timer`, driven the way the editor drives them.
//!
//! Every test advances time by calling `set_frame_dt`, which is what the frame
//! loop does — so these exercise the real drain rather than a test-only entry
//! point.

use ruster_lua::runtime::LuaRuntime;

/// Advance the clock by `ms`, as one frame would.
fn tick(rt: &LuaRuntime, ms: f64) {
    rt.set_frame_dt(ms / 1000.0);
}

fn runtime_with(src: &str) -> LuaRuntime {
    let rt = LuaRuntime::new().expect("runtime");
    rt.lua.load(src).exec().expect("lua loaded");
    rt
}

fn count(rt: &LuaRuntime) -> i64 {
    rt.lua.globals().get::<i64>("n").unwrap_or(-1)
}

#[test]
fn a_deferred_callback_runs_once_when_it_comes_due() {
    let rt = runtime_with("n = 0; ruster.defer(50, function() n = n + 1 end)");
    assert_eq!(rt.timer_count(), 1);

    tick(&rt, 20.0);
    assert_eq!(count(&rt), 0, "not due yet");
    tick(&rt, 20.0);
    assert_eq!(count(&rt), 0, "still not due");
    tick(&rt, 20.0);
    assert_eq!(count(&rt), 1, "due at 60ms");

    // And never again.
    tick(&rt, 1000.0);
    assert_eq!(count(&rt), 1, "a defer is one-shot");
    assert_eq!(rt.timer_count(), 0, "it was removed, not left to accumulate");
}

#[test]
fn a_repeating_timer_keeps_firing() {
    let rt = runtime_with("n = 0; ruster.timer(10, function() n = n + 1 end)");
    for _ in 0..5 {
        tick(&rt, 10.0);
    }
    assert_eq!(count(&rt), 5);
    assert_eq!(rt.timer_count(), 1, "it stays registered");
}

#[test]
fn a_repeating_timer_fires_at_most_once_per_frame() {
    // A frame that ran long must not cause a catch-up burst. Firing a debounce
    // five times because one frame took 50ms is never what was wanted, and it
    // lets a single slow frame cascade.
    let rt = runtime_with("n = 0; ruster.timer(10, function() n = n + 1 end)");
    tick(&rt, 500.0);
    assert_eq!(count(&rt), 1, "one slow frame is still one firing");

    // And the debt must not be carried: three short frames after the long one
    // are well inside the 10ms period, so nothing more should fire. Reloading
    // the period by adding rather than assigning leaves it deeply negative and
    // produces exactly that burst, one firing per frame until it catches up.
    for _ in 0..3 {
        tick(&rt, 1.0);
    }
    assert_eq!(count(&rt), 1, "the missed time was carried over into a burst");
}

#[test]
fn a_timer_can_be_cancelled() {
    let rt = runtime_with(
        "n = 0; id = ruster.timer(10, function() n = n + 1 end)",
    );
    tick(&rt, 10.0);
    assert_eq!(count(&rt), 1);

    rt.lua.load("stopped = ruster.timer_stop(id)").exec().unwrap();
    assert!(rt.lua.globals().get::<bool>("stopped").unwrap(), "cancel reported success");
    assert_eq!(rt.timer_count(), 0);

    tick(&rt, 100.0);
    assert_eq!(count(&rt), 1, "no further firings after cancel");
}

#[test]
fn cancelling_an_already_fired_defer_is_a_no_op() {
    // Ids are never reused, so this must not cancel some unrelated timer that
    // happened to be allocated later.
    let rt = runtime_with(
        "n = 0
         id = ruster.defer(10, function() n = n + 1 end)",
    );
    tick(&rt, 20.0);
    assert_eq!(count(&rt), 1);

    rt.lua
        .load(
            "other = ruster.timer(10, function() n = n + 100 end)
             stopped = ruster.timer_stop(id)",
        )
        .exec()
        .unwrap();
    assert!(!rt.lua.globals().get::<bool>("stopped").unwrap(), "nothing to cancel");
    tick(&rt, 20.0);
    assert_eq!(count(&rt), 101, "the unrelated timer still ran");
}

#[test]
fn a_callback_may_schedule_another_from_inside_itself() {
    // The reentrancy case. `take_due` holds a `RefCell` borrow while it walks
    // the list; calling the callback inside that borrow would panic with
    // `BorrowMutError` the moment a plugin rescheduled itself — which is how
    // anyone writes a backoff.
    let rt = runtime_with(
        "n = 0
         function step()
           n = n + 1
           if n < 3 then ruster.defer(10, step) end
         end
         ruster.defer(10, step)",
    );
    tick(&rt, 10.0);
    assert_eq!(count(&rt), 1);
    tick(&rt, 10.0);
    assert_eq!(count(&rt), 2);
    tick(&rt, 10.0);
    assert_eq!(count(&rt), 3);
    tick(&rt, 10.0);
    assert_eq!(count(&rt), 3, "it stopped rescheduling itself");
}

#[test]
fn a_callback_may_cancel_itself() {
    let rt = runtime_with(
        "n = 0
         id = ruster.timer(10, function()
           n = n + 1
           ruster.timer_stop(id)
         end)",
    );
    tick(&rt, 10.0);
    assert_eq!(count(&rt), 1);
    tick(&rt, 100.0);
    assert_eq!(count(&rt), 1, "it removed itself");
    assert_eq!(rt.timer_count(), 0);
}

#[test]
fn a_failing_callback_does_not_take_the_editor_down() {
    let rt = runtime_with(
        "n = 0
         ruster.defer(10, function() error('boom') end)
         ruster.defer(10, function() n = n + 1 end)",
    );
    tick(&rt, 10.0);
    assert_eq!(count(&rt), 1, "the second callback still ran");

    // And it is reported rather than swallowed: a plugin author with neither an
    // effect nor an error message has nothing to go on.
    let actions = rt.drain_actions();
    let reported = actions.iter().any(|a| {
        matches!(a, ruster_lua::runtime::LuaAction::Notify(3, msg) if msg.contains("boom"))
    });
    assert!(reported, "the failure was swallowed: {actions:?}");
}

#[test]
fn defer_zero_runs_on_the_next_frame_not_never() {
    // `ruster.defer(0, f)` is a legitimate "after this frame settles".
    let rt = runtime_with("n = 0; ruster.defer(0, function() n = n + 1 end)");
    assert_eq!(count(&rt), 0, "not during registration");
    tick(&rt, 0.0);
    assert_eq!(count(&rt), 1);
}

#[test]
fn a_nonsense_delay_does_not_wedge_the_timer() {
    // NaN and negatives come from arithmetic on a config value, and should mean
    // "as soon as possible" rather than "never" or "every frame forever".
    let rt = runtime_with(
        "n = 0
         ruster.defer(-100, function() n = n + 1 end)
         ruster.defer(0/0, function() n = n + 1 end)",
    );
    tick(&rt, 1.0);
    assert_eq!(count(&rt), 2);
    assert_eq!(rt.timer_count(), 0);
}

#[test]
fn timers_are_independent() {
    let rt = runtime_with(
        "a = 0; b = 0
         ruster.timer(10, function() a = a + 1 end)
         ruster.timer(30, function() b = b + 1 end)",
    );
    for _ in 0..3 {
        tick(&rt, 10.0);
    }
    assert_eq!(rt.lua.globals().get::<i64>("a").unwrap(), 3);
    assert_eq!(rt.lua.globals().get::<i64>("b").unwrap(), 1);
}
