//! Key repeat for the keys the compositor keeps for itself.
//!
//! Repeat is normally a client's job: the compositor announces a delay and a
//! rate over `wl_keyboard.repeat_info` and the toolkit at the other end turns
//! one held key into a stream. That arrangement breaks down for every key
//! [`on_keyboard_key`](crate::input::CompositorState::on_keyboard_key)
//! intercepts, because there is no toolkit behind it — an editor pane and the
//! mini-buffer are drawn by the compositor itself. Holding `j` in a pane moved
//! the cursor exactly one line, and holding backspace at the `:` prompt deleted
//! exactly one character.
//!
//! So the compositor repeats those keys itself, off a calloop timer, using the
//! same `repeat_delay`/`repeat_rate` the seat announces to clients — a user who
//! turned repeat off, or slowed it down, means it everywhere and not just in
//! other people's windows.
//!
//! One key repeats at a time and it is always the last one pressed: every press
//! cancels whatever was repeating before it, and the release of the repeating
//! key stops it. That is not tidiness. A repeat that outlives its keypress types
//! forever, and on a DRM boot the compositor *is* the display server, so there
//! is no other window to go and kill it from.

use std::time::Duration;

use smithay::input::keyboard::Keysym;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use ruster_core::key::KeyEvent;
use ruster_shell::WindowId;

use crate::backend::Backend;
use crate::compositor::CompositorState;

/// Where a repeating key's repeats go, and what to send.
///
/// Each variant carries what its sink was called with at press time rather than
/// the keysym it came from, so a repeat cannot be translated differently to the
/// press that armed it.
#[derive(Debug, Clone, PartialEq)]
pub enum RepeatTarget {
    /// The editor pane in `window`, as the editor key it takes.
    Pane { window: WindowId, key: KeyEvent },
    /// The open `:` prompt, as the raw/modified keysym pair it reads.
    /// A key going to whichever modal overlay is open — the `:` prompt, or the
    /// launcher.
    ///
    /// Modifiers are carried because the launcher navigates with `C-n`/`C-p`,
    /// and a control chord produces a control character that `key_char()`
    /// filters out. Adding them later would mean editing the interception
    /// branch a second time, and that branch is the one not to revisit.
    Overlay {
        raw: Keysym,
        modified: Keysym,
        mods: smithay::input::keyboard::ModifiersState,
    },
}

impl RepeatTarget {
    /// Whether holding this key down should repeat it.
    ///
    /// Modifiers never do. They are held for as long as whatever they modify,
    /// so a repeating Shift would do nothing over and over while displacing the
    /// repeat of the key it was pressed with — the last press wins, and Shift
    /// is often the last press.
    ///
    /// A pane target cannot be a modifier: [`editor_key`](crate::input::editor_key)
    /// yields `None` for every one of them, since none produces a character.
    fn repeats(&self) -> bool {
        match self {
            RepeatTarget::Pane { .. } => true,
            RepeatTarget::Overlay { raw, .. } => !raw.is_modifier_key(),
        }
    }
}

/// The key currently held down, and what its repeats do.
#[derive(Debug, Clone)]
pub struct KeyRepeat {
    /// The keycode whose release stops this. Matched rather than assumed: a
    /// modifier going up while a letter is still held must not stop the letter.
    keycode: u32,
    target: RepeatTarget,
    /// Distinguishes this arming from every other one.
    ///
    /// A cancelled timer cannot be taken back out of the loop from inside its
    /// own callback, so it is left to fire once and recognise that it is stale.
    /// Without the id a second press of the *same* keycode would be driven by
    /// two live timers at once, i.e. at twice the configured rate.
    id: u64,
    /// Gap between repeats once they start.
    interval: Duration,
}

/// How long between repeats at `rate` repeats per second, or `None` when the
/// configuration asks for no repeat at all.
///
/// Zero is how `wl_keyboard.repeat_info` says "disabled", and the compositor has
/// to honour that for its own panes too — otherwise turning repeat off would
/// work everywhere except the one place the compositor is the toolkit.
pub fn repeat_interval(rate: i32) -> Option<Duration> {
    (rate > 0).then(|| Duration::from_secs_f64(1.0 / f64::from(rate)))
}

impl<B: Backend + 'static> CompositorState<B> {
    /// Take one intercepted key: deliver it, then start it repeating until it
    /// comes back up.
    ///
    /// The single entry point for both sinks. Delivering here rather than at the
    /// call site is what stops a press and its repeats meaning different things.
    pub fn hold_key(&mut self, keycode: u32, target: RepeatTarget) {
        self.deliver_repeat(&target);
        // Whatever was repeating is superseded, whether or not this key goes on
        // to repeat itself: holding `j` and then pressing `k` repeats `k`.
        self.repeat = None;
        let Some(interval) =
            repeat_interval(self.keyboard_config.repeat_rate).filter(|_| target.repeats())
        else {
            return;
        };
        self.repeat_generation += 1;
        let id = self.repeat_generation;
        self.repeat = Some(KeyRepeat {
            keycode,
            target,
            id,
            interval,
        });
        let delay = Duration::from_millis(self.keyboard_config.repeat_delay.max(0) as u64);
        self.schedule_repeat(id, delay);
    }

    /// Stop repeating if `keycode` is the key that was repeating.
    ///
    /// Every release goes through here, including the ones the mini-buffer
    /// swallows whole, because the alternative is a key that repeats after it
    /// has been let go.
    pub fn release_key(&mut self, keycode: u32) {
        if self.repeat.as_ref().is_some_and(|r| r.keycode == keycode) {
            self.repeat = None;
        }
    }

    /// Stop repeating whatever was.
    pub fn cancel_repeat(&mut self) {
        self.repeat = None;
    }

    /// Whether a key is repeating right now.
    /// Deliver one repeat to the current target, reporting whether it landed.
    /// Test-only: the timer path is what drives this in a real session.
    #[cfg(test)]
    pub fn deliver_repeat_for_test(&mut self) -> bool {
        let Some(target) = self.repeat.as_ref().map(|r| r.target.clone()) else {
            return false;
        };
        self.deliver_repeat(&target)
    }

    pub fn is_repeating(&self) -> bool {
        self.repeat.is_some()
    }

    /// Put the timer for arming `id` on the event loop.
    ///
    /// A one-shot [`Timer`] that re-arms itself with the repeat interval, the
    /// same shape the DRM backend schedules its repaints with. It is a timer
    /// source rather than anything the backends do per iteration because the
    /// winit loop already dispatches with a 1ms timeout, and giving it work to
    /// poll for would turn that into a busy-wait.
    fn schedule_repeat(&mut self, id: u64, delay: Duration) {
        let scheduled = self.handle.insert_source(
            Timer::from_duration(delay),
            move |_, _, state: &mut CompositorState<B>| state.fire_repeat(id),
        );
        if let Err(err) = scheduled {
            // Losing the timer means the key simply does not repeat. Forgetting
            // the arming as well keeps that honest: `is_repeating` would
            // otherwise claim a repeat that nothing will ever deliver.
            tracing::warn!(%err, "could not schedule the key repeat timer");
            self.repeat = None;
        }
    }

    /// One tick of the repeat timer armed as `id`.
    fn fire_repeat(&mut self, id: u64) -> TimeoutAction {
        // A press since this timer was armed has moved the repeat on, so this
        // one is the tail of a key that is no longer held.
        let Some(repeat) = self.repeat.as_ref().filter(|r| r.id == id) else {
            return TimeoutAction::Drop;
        };
        let (target, interval) = (repeat.target.clone(), repeat.interval);
        if !self.deliver_repeat(&target) {
            self.repeat = None;
            return TimeoutAction::Drop;
        }
        TimeoutAction::ToDuration(interval)
    }

    /// Send one key to the sink `target` names, reporting whether the sink was
    /// still there to take it.
    ///
    /// The same two calls the press path made before this existed —
    /// [`pane_key`](CompositorState::pane_key) and
    /// [`minibuffer_key`](CompositorState::minibuffer_key) — so nothing here
    /// decides what a key *means*.
    ///
    /// `false` is what stops a repeat outliving what it was aimed at: a pane
    /// that lost focus, or a prompt that has closed, is no longer listening, and
    /// delivering to it anyway would type into whatever took its place.
    fn deliver_repeat(&mut self, target: &RepeatTarget) -> bool {
        match target {
            RepeatTarget::Pane { window, key } => {
                if self.shell.focus != Some(*window) || !self.panes.contains_key(window) {
                    return false;
                }
                self.pane_key(*key);
                true
            }
            RepeatTarget::Overlay {
                raw,
                modified,
                mods,
            } => {
                if !self.overlay_is_open() {
                    return false;
                }
                self.overlay_key(*raw, *modified, *mods);
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_interval_is_the_configured_rate_and_zero_turns_repeat_off() {
        // The default seat rate, as a gap between repeats.
        assert_eq!(repeat_interval(25), Some(Duration::from_millis(40)));
        assert_eq!(repeat_interval(100), Some(Duration::from_millis(10)));
        // `wl_keyboard.repeat_info` spells "no repeat" as a rate of zero, and a
        // negative one would otherwise become a negative duration and panic.
        assert_eq!(repeat_interval(0), None);
        assert_eq!(repeat_interval(-5), None);
    }

    #[test]
    fn a_modifier_is_never_a_repeating_key() {
        // A held Shift would repeat nothing while displacing the repeat of the
        // key it was pressed with.
        for raw in [Keysym::Shift_L, Keysym::Control_L, Keysym::Super_L] {
            assert!(
                !RepeatTarget::Overlay {
                    raw,
                    modified: raw,
                    mods: Default::default()
                }
                .repeats(),
                "{raw:?} must not repeat"
            );
        }
        assert!(RepeatTarget::Overlay {
            raw: Keysym::a,
            modified: Keysym::a,
            mods: Default::default(),
        }
        .repeats());
    }
}
