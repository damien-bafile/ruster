//! Multi-key bindings, and the which-key overlay that explains them.
//!
//! [`resolve_wm_action`](crate::input::resolve_wm_action) matched one chord
//! against one binding and had nowhere to keep a half-typed sequence, so there
//! was no `M-w h` and no way for the overlay to be *triggered* — it sat on
//! screen permanently because there was no state that could turn it off.
//!
//! A binding is now a whitespace-separated sequence of chords: `"M-w h"` means
//! Super+w, then h. A single chord is a sequence of length one, so every
//! existing config keeps working unchanged.
//!
//! The editor has a leader implementation (`LEADER_ROOT`, `leader_resolve`) and
//! this is deliberately not it. That one walks a `&'static` tree written out in
//! source; a compositor's keymap is `Vec<(String, String)>` read from a file at
//! startup, and the shapes are different enough that one type serving both would
//! fit neither. What *is* shared is [`WhichKeyView`], which both sides render.

use std::time::{Duration, Instant};

use ruster_render::{WhichKeyEntry, WhichKeyView};
use smithay::input::keyboard::ModifiersState;

use crate::lua::Action;

/// How long a half-typed sequence waits for its next key.
///
/// Without a timeout a mistyped prefix swallows keystrokes forever: the next
/// thing you type goes to the keymap instead of the window, and nothing on
/// screen ever says why. Matches the editor's leader timeout.
pub const CHORD_TIMEOUT: Duration = Duration::from_millis(1000);

/// What a keypress meant, given what had already been typed.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolved {
    /// Not bound here. The key belongs to the focused client.
    None,
    /// A prefix of at least one binding — wait for the next key.
    Pending,
    /// A complete binding.
    Action(Action),
}

/// The bindings in force, as chord sequences.
#[derive(Debug, Clone, Default)]
pub struct Keymap {
    /// `(chord specs, action name)`, in config order — first match wins, the
    /// same rule the single-chord matcher used.
    binds: Vec<(Vec<String>, String)>,
}

impl Keymap {
    pub fn new(binds: &[(String, String)]) -> Self {
        Keymap {
            binds: binds
                .iter()
                .map(|(bind, action)| {
                    (
                        bind.split_whitespace().map(str::to_string).collect(),
                        action.clone(),
                    )
                })
                .filter(|(chords, _): &(Vec<String>, String)| !chords.is_empty())
                .collect(),
        }
    }

    /// Resolve `(mods, key)` arriving after the chords already in `pending`.
    ///
    /// `pending` holds the *specs* that matched, not the raw keys, so the
    /// comparison here is spec-to-spec and cannot disagree with the match that
    /// put them there.
    pub fn resolve(&self, pending: &[String], mods: &ModifiersState, key: &str) -> Resolved {
        let mut saw_longer = false;
        for (chords, action) in &self.binds {
            if chords.len() <= pending.len() || !starts_with(chords, pending) {
                continue;
            }
            if !Action::keybind_matches(&chords[pending.len()], mods, key) {
                continue;
            }
            if chords.len() == pending.len() + 1 {
                // A complete binding wins immediately. A longer binding sharing
                // this prefix would otherwise make the shorter one unreachable,
                // and the config that wrote both should get the one it named.
                if let Some(action) = Action::from_name(action) {
                    return Resolved::Action(action);
                }
            } else {
                saw_longer = true;
            }
        }
        if saw_longer {
            Resolved::Pending
        } else {
            Resolved::None
        }
    }

    /// The binding that runs `action`, if the config named one.
    ///
    /// The welcome frame needs this to say how to quit. It reads from the same
    /// structure the key handler resolves against, so it cannot advertise a
    /// binding the keyboard does not have — which is exactly what the hardcoded
    /// `M-S-q` did before.
    pub fn binding_for(&self, action: &str) -> Option<String> {
        self.binds
            .iter()
            .find(|(_, name)| name == action)
            .map(|(chords, _)| chords.join(" "))
    }

    /// The keys that could come next after `pending`, and what they do.
    ///
    /// Empty when nothing continues from here, which is what tells the caller
    /// there is no overlay to show.
    pub fn continuations(&self, pending: &[String]) -> Vec<WhichKeyEntry> {
        let mut rows: Vec<WhichKeyEntry> = Vec::new();
        for (chords, action) in &self.binds {
            if chords.len() <= pending.len() || !starts_with(chords, pending) {
                continue;
            }
            let key = chords[pending.len()].clone();
            if rows.iter().any(|r| r.key == key) {
                continue;
            }
            // A key that continues into a longer sequence is a group, and
            // naming it after one of its leaves would be a lie about what
            // pressing it does.
            let desc = if chords.len() == pending.len() + 1 {
                action.clone()
            } else {
                format!("+{}", chords[pending.len() + 1..].join(" "))
            };
            rows.push(WhichKeyEntry { key, desc });
        }
        rows
    }
}

fn starts_with(chords: &[String], prefix: &[String]) -> bool {
    chords.len() >= prefix.len() && chords[..prefix.len()] == *prefix
}

/// The half-typed sequence, and when it was last added to.
#[derive(Debug, Clone, Default)]
pub struct ChordState {
    pending: Vec<String>,
    last: Option<Instant>,
}

impl ChordState {
    /// The chords typed so far.
    pub fn pending(&self) -> &[String] {
        &self.pending
    }

    pub fn is_active(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Record that `chord` continued the sequence.
    pub fn push(&mut self, chord: String) {
        self.pending.push(chord);
        self.last = Some(Instant::now());
    }

    pub fn clear(&mut self) {
        self.pending.clear();
        self.last = None;
    }

    /// Drop the sequence if it has been waiting too long, reporting whether it
    /// did. Called on each key and each event-loop pass, so a prefix left
    /// hanging clears itself rather than eating the next thing typed.
    pub fn expire(&mut self, now: Instant) -> bool {
        match self.last {
            Some(at) if now.duration_since(at) >= CHORD_TIMEOUT => {
                self.clear();
                true
            }
            _ => false,
        }
    }

    /// The chord spec that matched `(mods, key)` out of `candidates`, if any.
    ///
    /// The spec is what gets pushed, so `Keymap::resolve` can compare strings
    /// rather than re-deriving which binding matched.
    pub fn matching_spec(
        candidates: &[WhichKeyEntry],
        mods: &ModifiersState,
        key: &str,
    ) -> Option<String> {
        candidates
            .iter()
            .find(|row| Action::keybind_matches(&row.key, mods, key))
            .map(|row| row.key.clone())
    }
}

/// The overlay to draw for a half-typed sequence, or `None` when nothing is
/// pending.
///
/// The overlay used to be drawn unconditionally from a hardcoded pair, so it was
/// always on screen and never about anything. It now appears only once a prefix
/// has been typed, which is what "which key" means.
pub fn whichkey_view(keymap: &Keymap, chord: &ChordState) -> Option<WhichKeyView> {
    if !chord.is_active() {
        return None;
    }
    let rows = keymap.continuations(chord.pending());
    if rows.is_empty() {
        return None;
    }
    Some(WhichKeyView {
        title: chord.pending().join(" "),
        rows,
        anim: 1.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruster_shell::Direction;

    fn logo() -> ModifiersState {
        ModifiersState {
            logo: true,
            ..Default::default()
        }
    }

    fn plain() -> ModifiersState {
        ModifiersState::default()
    }

    #[test]
    fn a_single_chord_binding_still_fires_directly() {
        // Every existing config is a sequence of length one, and must keep
        // working exactly as it did.
        let km = Keymap::new(&[("M-h".into(), "focus left".into())]);
        assert_eq!(
            km.resolve(&[], &logo(), "h"),
            Resolved::Action(Action::Focus(Direction::Left))
        );
    }

    #[test]
    fn a_sequence_waits_for_its_second_key() {
        let km = Keymap::new(&[("M-w h".into(), "focus left".into())]);
        assert_eq!(km.resolve(&[], &logo(), "w"), Resolved::Pending);
        assert_eq!(
            km.resolve(&["M-w".into()], &plain(), "h"),
            Resolved::Action(Action::Focus(Direction::Left))
        );
    }

    #[test]
    fn a_key_that_continues_nothing_belongs_to_the_client() {
        // The compositor must not swallow keystrokes it has no use for; that
        // is how a terminal stops receiving letters.
        let km = Keymap::new(&[("M-w h".into(), "focus left".into())]);
        assert_eq!(km.resolve(&[], &plain(), "x"), Resolved::None);
        assert_eq!(km.resolve(&["M-w".into()], &plain(), "x"), Resolved::None);
    }

    #[test]
    fn a_complete_binding_beats_a_longer_one_sharing_its_prefix() {
        // Otherwise binding both `M-w` and `M-w h` would make `M-w`
        // unreachable, silently, with the config saying it is bound.
        let km = Keymap::new(&[
            ("M-w".into(), "quit".into()),
            ("M-w h".into(), "focus left".into()),
        ]);
        assert_eq!(
            km.resolve(&[], &logo(), "w"),
            Resolved::Action(Action::Quit)
        );
    }

    #[test]
    fn continuations_describe_groups_as_groups() {
        // A key that leads somewhere is not the same as a key that does
        // something, and labelling a prefix with one of its leaves would claim
        // pressing it performs that leaf.
        let km = Keymap::new(&[
            ("M-w h".into(), "focus left".into()),
            ("M-w l".into(), "focus right".into()),
            ("M-q".into(), "quit".into()),
        ]);
        let root = km.continuations(&[]);
        let group = root.iter().find(|r| r.key == "M-w").unwrap();
        assert_eq!(group.desc, "+h");
        let leaf = root.iter().find(|r| r.key == "M-q").unwrap();
        assert_eq!(leaf.desc, "quit");

        let inner = km.continuations(&["M-w".into()]);
        assert_eq!(inner.len(), 2);
        assert_eq!(inner[0].desc, "focus left");
    }

    #[test]
    fn the_overlay_only_appears_once_something_is_pending() {
        // It used to be drawn every frame from a hardcoded pair, so it was
        // permanently on screen and never about anything in particular.
        let km = Keymap::new(&[("M-w h".into(), "focus left".into())]);
        let mut chord = ChordState::default();
        assert!(whichkey_view(&km, &chord).is_none());
        chord.push("M-w".into());
        let view = whichkey_view(&km, &chord).expect("a prefix should raise the overlay");
        assert_eq!(view.title, "M-w");
        assert_eq!(view.rows.len(), 1);
    }

    #[test]
    fn a_pending_sequence_times_out_rather_than_eating_the_next_key() {
        // A half-typed prefix left alone would otherwise capture whatever is
        // typed next, with nothing on screen explaining where it went.
        let mut chord = ChordState::default();
        chord.push("M-w".into());
        assert!(!chord.expire(Instant::now()), "not yet");
        assert!(chord.is_active());
        assert!(chord.expire(Instant::now() + CHORD_TIMEOUT));
        assert!(!chord.is_active());
        // And an idle state has nothing to expire.
        assert!(!chord.expire(Instant::now() + CHORD_TIMEOUT * 10));
    }

    #[test]
    fn an_empty_binding_string_is_dropped_rather_than_matching_everything() {
        // A zero-length sequence is a prefix of every sequence, so it would
        // otherwise claim every key pressed.
        let km = Keymap::new(&[("".into(), "quit".into()), ("   ".into(), "quit".into())]);
        assert_eq!(km.resolve(&[], &logo(), "q"), Resolved::None);
        assert!(km.continuations(&[]).is_empty());
    }
}
