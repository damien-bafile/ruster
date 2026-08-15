//! The fuzzy-filtered selectable list, and the one scoring scale.
//!
//! This was `ruster-tui`'s picker, used by the buffer list, file finder, live
//! grep and context menus. It moved here so the compositor's launcher can use
//! the same state machine rather than growing a second one: two implementations
//! of "which row is selected" are two answers that can disagree, and this
//! codebase has paid for that shape before (`FrameBody`, `popup_rect`).
//!
//! [`PickerState`] is generic over the action a row carries, because that is the
//! only part that was ever editor-specific — `ruster-tui` keeps its
//! `PickerAction` and aliases this.
//!
//! Rendering is not here. A picker produces a [`ruster_render::PickerView`] and
//! something else draws it: ratatui in the TUI, raylib in the GUI, the GL chrome
//! in the compositor.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use ruster_render::{PickerRow, PickerView};

/// The top of the shared confidence scale. A provider that is *certain* — a
/// parsed equation, an exact name match — returns this.
pub const CONFIDENCE_MAX: u32 = 1000;

/// Where a fuzzy match can reach. Below the prefix band on purpose: no amount of
/// scattered-letter matching should outrank something the user actually typed
/// the start of.
pub const CONFIDENCE_FUZZY_CEILING: u32 = 800;

/// Map a raw matcher score onto `0..=CONFIDENCE_MAX`.
///
/// This exists because nucleo's scores are unbounded and grow with the length of
/// the pattern. Compared directly across providers they are nonsense that looks
/// plausible: a long query into a large app list out-scores a parsed equation
/// for no reason but having more characters in it. Bounding them is what makes
/// "best result first" mean anything when the results came from different
/// places.
///
/// The constants are arbitrary. The properties are not, and are what the tests
/// assert: equality beats a prefix, a prefix beats any fuzzy match, and fuzzy is
/// monotone in the raw score.
pub fn confidence(raw: u32, query: &str, haystack: &str) -> u32 {
    if query.eq_ignore_ascii_case(haystack) {
        return CONFIDENCE_MAX;
    }
    let (q, h) = (query.to_ascii_lowercase(), haystack.to_ascii_lowercase());
    if !q.is_empty() && h.starts_with(&q) {
        // The more of the haystack the prefix covers, the better: "fire" against
        // "Firefox" beats "fire" against "Firewall Configuration Tool".
        let share = (q.len() * 99 / h.len().max(1)) as u32;
        return 900 + share.min(99);
    }
    // Asymptotic to the fuzzy ceiling, so this band can never reach the prefix
    // band however large the raw score gets.
    let raw = raw as u64;
    (CONFIDENCE_FUZZY_CEILING as u64 * raw / (raw + 200)) as u32
}

/// The matcher, owned because nucleo needs `&mut` to score.
pub struct Fuzzy {
    matcher: Matcher,
    buf: Vec<char>,
}

impl Default for Fuzzy {
    fn default() -> Self {
        Self::new()
    }
}

impl Fuzzy {
    pub fn new() -> Self {
        Fuzzy {
            matcher: Matcher::new(Config::DEFAULT),
            buf: Vec::new(),
        }
    }

    /// nucleo's own score, unbounded. Use [`Fuzzy::score`] unless you are
    /// ranking within one list — across providers this is not comparable.
    pub fn raw_score(&mut self, query: &str, haystack: &str) -> Option<u32> {
        if query.is_empty() {
            return Some(0);
        }
        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
        let hay = Utf32Str::new(haystack, &mut self.buf);
        pattern.score(hay, &mut self.matcher)
    }

    /// The score on the shared scale, which is what a [`Provider`]-style caller
    /// wants.
    ///
    /// [`Provider`]: https://docs.rs/ruster-compositor
    pub fn score(&mut self, query: &str, haystack: &str) -> Option<u32> {
        let raw = self.raw_score(query, haystack)?;
        Some(confidence(raw, query, haystack))
    }
}

/// One selectable entry, carrying whatever the caller wants to do with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerItem<A> {
    pub label: String,
    pub action: A,
}

impl<A> PickerItem<A> {
    pub fn new(label: impl Into<String>, action: A) -> Self {
        PickerItem {
            label: label.into(),
            action,
        }
    }
}

/// A live picker: a title, the full item list, the typed filter, and the
/// current selection (an index into the *filtered* view).
pub struct PickerState<A> {
    pub title: String,
    items: Vec<PickerItem<A>>,
    pub filter: String,
    pub selected: usize,
    /// Center (floating) or bottom (docked) placement.
    pub placement: ruster_render::PickerPlacement,
    fuzzy: Fuzzy,
    /// The last filter the cache was computed for, and the result.
    ///
    /// `filtered()` is called by `view`, `accept`, `selected_action` and
    /// `move_selection` — several times a frame — and each call scored every
    /// item afresh. The cache makes that once per change instead of once per
    /// question.
    cache: Option<(String, usize, Vec<usize>)>,
}

impl<A: Clone> PickerState<A> {
    pub fn new(title: impl Into<String>, items: Vec<PickerItem<A>>) -> Self {
        PickerState {
            title: title.into(),
            items,
            filter: String::new(),
            selected: 0,
            placement: ruster_render::PickerPlacement::Center,
            fuzzy: Fuzzy::new(),
            cache: None,
        }
    }

    /// Indices into `items` matching the current filter, best match first.
    /// An empty filter keeps the original order.
    pub fn filtered(&mut self) -> Vec<usize> {
        if let Some((filter, len, hits)) = &self.cache {
            if filter == &self.filter && *len == self.items.len() {
                return hits.clone();
            }
        }
        let hits = self.compute_filtered();
        self.cache = Some((self.filter.clone(), self.items.len(), hits.clone()));
        hits
    }

    fn compute_filtered(&mut self) -> Vec<usize> {
        if self.filter.is_empty() {
            return (0..self.items.len()).collect();
        }
        let mut scored: Vec<(usize, u32)> = Vec::new();
        for i in 0..self.items.len() {
            let label = self.items[i].label.clone();
            if let Some(score) = self.fuzzy.raw_score(&self.filter, &label) {
                scored.push((i, score));
            }
        }
        // Higher score first; stable on ties by original index.
        scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        scored.into_iter().map(|(i, _)| i).collect()
    }

    fn filtered_len(&mut self) -> usize {
        self.filtered().len()
    }

    /// Move the selection by `delta`, wrapping within the filtered list.
    pub fn move_selection(&mut self, delta: i32) {
        let len = self.filtered_len();
        if len == 0 {
            self.selected = 0;
            return;
        }
        let cur = self.selected.min(len - 1) as i32;
        let next = (cur + delta).rem_euclid(len as i32);
        self.selected = next as usize;
    }

    /// Append an item (used to stream results into an open picker).
    pub fn push_item(&mut self, item: PickerItem<A>) {
        self.items.push(item);
    }

    /// Replace every item, resetting the selection. The launcher rebuilds its
    /// list on each keystroke rather than streaming into it.
    pub fn set_items(&mut self, items: Vec<PickerItem<A>>) {
        self.items = items;
        self.selected = 0;
        self.cache = None;
    }

    /// Number of items currently loaded.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn push_char(&mut self, c: char) {
        self.filter.push(c);
        self.selected = 0;
    }

    pub fn pop_char(&mut self) {
        self.filter.pop();
        self.selected = 0;
    }

    /// The action of the currently-selected item without consuming it (used to
    /// build the preview pane).
    pub fn selected_action(&mut self) -> Option<A> {
        let filtered = self.filtered();
        let idx = *filtered.get(self.selected)?;
        Some(self.items[idx].action.clone())
    }

    /// The action of the currently-selected item, if any.
    pub fn accept(&mut self) -> Option<A> {
        let filtered = self.filtered();
        let idx = *filtered.get(self.selected)?;
        Some(self.items[idx].action.clone())
    }

    /// Build the render view for the current state.
    pub fn view(&mut self) -> PickerView {
        let filtered = self.filtered();
        let selected = self.selected.min(filtered.len().saturating_sub(1));
        let rows = filtered
            .iter()
            .enumerate()
            .map(|(row, &item_idx)| PickerRow {
                label: self.items[item_idx].label.clone(),
                selected: row == selected,
            })
            .collect();
        PickerView {
            title: self.title.clone(),
            query: self.filter.clone(),
            rows,
            preview: Vec::new(), // filled in by the app
            placement: self.placement,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_exact_name_is_certain_and_a_prefix_is_nearly_so() {
        assert_eq!(confidence(50, "firefox", "Firefox"), CONFIDENCE_MAX);
        let prefix = confidence(50, "fire", "Firefox");
        assert!(
            (900..CONFIDENCE_MAX).contains(&prefix),
            "a prefix belongs in its own band: {prefix}"
        );
    }

    #[test]
    fn no_fuzzy_match_can_outrank_a_prefix() {
        // The property the whole scale exists for. nucleo's raw scores are
        // unbounded and grow with pattern length, so without the mapping a long
        // scattered match beats a short prefix — and worse, beats a provider
        // that was certain.
        let huge = confidence(u32::MAX, "xyz", "a haystack with x y and z in it");
        assert!(
            huge < 900,
            "a fuzzy match reached the prefix band with score {huge}"
        );
        assert!(huge <= CONFIDENCE_FUZZY_CEILING);
        assert!(confidence(u32::MAX, "q", "q") == CONFIDENCE_MAX);
    }

    #[test]
    fn fuzzy_confidence_rises_with_the_raw_score() {
        let (low, high) = (
            confidence(10, "abc", "a b c zzz"),
            confidence(400, "abc", "a b c zzz"),
        );
        assert!(low < high, "{low} should be below {high}");
    }

    #[test]
    fn a_longer_prefix_of_a_shorter_name_ranks_higher() {
        // "fire" is most of "Firefox" and very little of the other, so the first
        // should win — which is the behaviour a launcher lives or dies on.
        let focused = confidence(50, "fire", "Firefox");
        let diluted = confidence(50, "fire", "Firewall Configuration Tool");
        assert!(focused > diluted, "{focused} should beat {diluted}");
    }

    #[test]
    fn the_filter_is_scored_once_per_change_not_once_per_question() {
        // `view`, `accept` and `move_selection` all ask for the filtered list.
        // Before the cache each of those re-scored every item, several times a
        // frame.
        let mut p = PickerState::new(
            "t",
            vec![PickerItem::new("alpha", 1u8), PickerItem::new("beta", 2u8)],
        );
        p.push_char('a');
        let first = p.filtered();
        assert_eq!(p.filtered(), first, "a repeat question gives one answer");
        assert!(p.cache.is_some(), "and it was cached");

        // Changing the item count must invalidate it, or a streamed-in result
        // never appears — which is how the file finder works.
        //
        // Asserted as an exact count, not `>=`: a stale cache returns the
        // previous list, which satisfies `>=` perfectly well. The first version
        // of this assertion did that and survived a cache that ignored the item
        // count entirely.
        assert_eq!(first.len(), 2, "alpha and beta contain an 'a'");
        p.push_item(PickerItem::new("gamma", 3u8));
        assert_eq!(
            p.filtered().len(),
            3,
            "a streamed-in item must be scored, not served from the cache"
        );
    }

    #[test]
    fn set_items_replaces_and_resets() {
        let mut p = PickerState::new("t", vec![PickerItem::new("old", 1u8)]);
        p.move_selection(1);
        p.set_items(vec![
            PickerItem::new("new", 2u8),
            PickerItem::new("er", 3u8),
        ]);
        assert_eq!(p.len(), 2);
        assert_eq!(p.selected, 0);
        assert_eq!(p.accept(), Some(2u8));
    }
}
