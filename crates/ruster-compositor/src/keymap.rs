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

    /// Every binding in force, full sequence and all, in config order.
    ///
    /// [`continuations`](Self::continuations) answers "what can I press next",
    /// which is the right question mid-chord and the wrong one for a reference:
    /// it collapses `M-w h` and `M-w l` into a single `M-w  +h` row, so a
    /// two-key binding can never be read off it. This lists what a user would
    /// actually have to type.
    pub fn all_bindings(&self) -> Vec<WhichKeyEntry> {
        self.binds
            .iter()
            .map(|(chords, action)| WhichKeyEntry {
                key: chords.join(" "),
                desc: action.clone(),
            })
            .collect()
    }

    /// Add a binding while the compositor is running.
    ///
    /// `ruster.wm.set_keybind` only recorded into the config until now, so a
    /// keybind set at runtime silently did nothing — which stopped being
    /// defensible once the Lua API became live.
    pub fn bind(&mut self, binding: &str, action: &str) {
        let chords: Vec<String> = binding.split_whitespace().map(str::to_string).collect();
        if chords.is_empty() {
            return;
        }
        // Replace rather than append: first match wins, so a second binding for
        // the same chord would be dead the moment it was added.
        self.binds.retain(|(existing, _)| *existing != chords);
        self.binds.push((chords, action.to_string()));
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

/// Whether the shortcut helper is pinned open.
///
/// It used to carry a row budget too, back when a list too long for the panel
/// was truncated. The panel lays itself out in columns now, so everything fits
/// and there is nothing to budget.
#[derive(Debug, Clone, Copy)]
pub struct HelpState {
    pub pinned: bool,
}

pub fn whichkey_view(keymap: &Keymap, chord: &ChordState, help: HelpState) -> Option<WhichKeyView> {
    let (title, rows) = if chord.is_active() {
        (
            chord.pending().join(" "),
            keymap.continuations(chord.pending()),
        )
    } else if help.pinned {
        // Every binding, not the root continuations: a reference that collapses
        // `M-w h` and `M-w l` into one `+h` row cannot be read off.
        ("keys".to_string(), keymap.all_bindings())
    } else {
        return None;
    };
    if rows.is_empty() {
        return None;
    }
    Some(WhichKeyView {
        title,
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
        let off = HelpState { pinned: false };
        assert!(whichkey_view(&km, &chord, off).is_none());
        chord.push("M-w".into());
        let view = whichkey_view(&km, &chord, off).expect("a prefix should raise the overlay");
        assert_eq!(view.title, "M-w");
        assert_eq!(view.rows.len(), 1);
    }

    #[test]
    fn pinning_the_helper_shows_the_whole_keymap_with_nothing_pending() {
        // With the overlay only appearing mid-chord there is otherwise no way
        // to simply look up what is bound.
        let km = Keymap::new(&[
            ("M-h".into(), "focus left".into()),
            ("M-l".into(), "focus right".into()),
        ]);
        let chord = ChordState::default();
        let pinned = HelpState { pinned: true };
        let view = whichkey_view(&km, &chord, pinned).expect("pinned should show");
        assert_eq!(view.title, "keys");
        assert_eq!(view.rows.len(), 2);
    }

    #[test]
    fn a_half_typed_chord_wins_over_the_pinned_helper() {
        // What you are in the middle of matters more than a reference you left
        // open, and showing the root list mid-sequence would be wrong anyway.
        let km = Keymap::new(&[
            ("M-w h".into(), "focus left".into()),
            ("M-q".into(), "quit".into()),
        ]);
        let mut chord = ChordState::default();
        chord.push("M-w".into());
        let view = whichkey_view(&km, &chord, HelpState { pinned: true }).unwrap();
        assert_eq!(view.title, "M-w");
        assert_eq!(view.rows.len(), 1, "the continuations, not the root list");
    }

    #[test]
    fn the_pinned_helper_lists_multi_key_bindings_in_full() {
        // `continuations` collapses `M-w h` and `M-w l` into one `M-w  +h` row,
        // so a reference built from it could not tell you what to actually
        // type. Every binding is listed whole instead.
        let km = Keymap::new(&[
            ("M-w h".into(), "focus left".into()),
            ("M-w l".into(), "focus right".into()),
            ("M-q".into(), "quit".into()),
        ]);
        let view = whichkey_view(&km, &ChordState::default(), HelpState { pinned: true }).unwrap();
        let keys: Vec<&str> = view.rows.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, ["M-w h", "M-w l", "M-q"]);
        assert_eq!(view.rows[0].desc, "focus left");
    }

    #[test]
    fn a_binding_added_at_runtime_shows_up_immediately() {
        // `ruster.wm.set_keybind` recorded into the config and nothing else, so
        // a keybind set at runtime silently did nothing — and could not appear
        // in a helper that reads the live keymap.
        let mut km = Keymap::new(&[("M-q".into(), "quit".into())]);
        km.bind("M-x", "screenshot");
        let view = whichkey_view(&km, &ChordState::default(), HelpState { pinned: true }).unwrap();
        assert_eq!(view.rows.len(), 2);
        assert!(view.rows.iter().any(|r| r.key == "M-x"));
        assert_eq!(
            km.resolve(&[], &logo(), "x"),
            Resolved::Action(Action::Screenshot)
        );
    }

    #[test]
    fn rebinding_a_chord_replaces_it_rather_than_shadowing_it() {
        // First match wins, so an appended duplicate would be dead on arrival
        // while the config said it was bound.
        let mut km = Keymap::new(&[("M-q".into(), "quit".into())]);
        km.bind("M-q", "screenshot");
        assert_eq!(
            km.resolve(&[], &logo(), "q"),
            Resolved::Action(Action::Screenshot)
        );
        assert_eq!(km.all_bindings().len(), 1);
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
