//! What answers a launcher query, and how the answers are ordered.
//!
//! The launcher is a query engine with pluggable providers, of which
//! "applications" is only the first. That is the whole point of the feature: the
//! request was for something expandable, and an abstraction with one
//! implementation has never been tested.
//!
//! Every provider scores on one bounded scale
//! ([`ruster_picker::confidence`]) so results from different providers can be
//! ranked against each other at all — raw matcher scores are unbounded and grow
//! with pattern length, which makes cross-provider ordering nonsense that looks
//! plausible.

use crate::lua::Action;

/// One result a provider offers.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub label: String,
    /// Right-hand detail: the command a row runs, an equation's value. May be
    /// empty.
    pub detail: String,
    /// Confidence on the shared `0..=CONFIDENCE_MAX` scale — *not* a raw matcher
    /// score, which is not comparable between providers.
    pub score: u32,
    pub activation: Activation,
}

/// What accepting a row does.
///
/// A closed set of effects the compositor already knows how to perform, rather
/// than a boxed closure. Every route into the WM ends at `dispatch`; a closure
/// would be a second one, and the first thing it would be used for is spawning a
/// process behind `persist`'s back, which is what makes a window unrelaunchable
/// on the next session restore.
#[derive(Debug, Clone, PartialEq)]
pub enum Activation {
    /// Any WM action — how an app launches (`Spawn`), how a file opens (`Edit`),
    /// and what a Lua row's `action = "..."` becomes.
    Action(Action),
    /// Put text on the seat selection. The maths provider's accept.
    Copy(String),
    /// Say something and close.
    Report(String),
}

/// What a provider may read while answering.
///
/// Deliberately tiny. A provider handed the whole `CompositorState` is a
/// provider that can re-enter `dispatch` in the middle of a keystroke; this
/// carries the one thing a provider legitimately needs and nothing else.
#[derive(Default)]
pub struct ProviderCtx<'a> {
    /// The live Lua VM, for the bridge. `None` when no config loaded.
    pub wm: Option<&'a crate::lua::WmControl>,
}

pub trait Provider {
    /// The group heading, and the name a warning about this provider uses.
    fn name(&self) -> &str;

    /// Called each time the launcher opens, before the first query. A cheap
    /// refresh belongs here; an expensive one belongs behind a staleness check.
    fn prepare(&mut self) {}

    /// Score `query`, returning at most `limit` candidates, best first.
    fn query(&mut self, query: &str, ctx: &ProviderCtx<'_>, limit: usize) -> Vec<Candidate>;
}

/// One provider's answers.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub name: String,
    pub rows: Vec<Candidate>,
}

/// The registered providers, in registration order.
#[derive(Default)]
pub struct ProviderSet {
    providers: Vec<Box<dyn Provider>>,
}

impl ProviderSet {
    pub fn push(&mut self, provider: Box<dyn Provider>) {
        self.providers.push(provider);
    }

    pub fn names(&self) -> Vec<&str> {
        self.providers.iter().map(|p| p.name()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    pub fn prepare(&mut self) {
        for p in &mut self.providers {
            p.prepare();
        }
    }

    /// Ask every provider, and order what comes back.
    pub fn query(&mut self, query: &str, ctx: &ProviderCtx<'_>, per_provider: usize) -> Vec<Group> {
        let groups = self
            .providers
            .iter_mut()
            .map(|p| Group {
                name: p.name().to_string(),
                rows: p.query(query, ctx, per_provider),
            })
            .collect();
        order_groups(groups)
    }
}

/// Order groups by their best candidate, descending; ties keep registration
/// order. Empty groups are dropped, and rows within a group are re-sorted.
///
/// The re-sort is not defensive tidying: a Lua provider is written by someone
/// else and may return anything in any order, and a group whose rows disagree
/// with their own scores would make the selection appear to jump.
pub fn order_groups(mut groups: Vec<Group>) -> Vec<Group> {
    groups.retain(|g| !g.rows.is_empty());
    for group in &mut groups {
        group.rows.sort_by_key(|r| std::cmp::Reverse(r.score));
    }
    // `sort_by_key` is stable, so equal bests keep the order they were
    // registered in — which is the only tie-break a user can predict.
    groups.sort_by_key(|g| std::cmp::Reverse(g.rows.first().map(|r| r.score).unwrap_or(0)));
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        name: &'static str,
        scores: Vec<u32>,
    }

    impl Provider for Fake {
        fn name(&self) -> &str {
            self.name
        }
        fn query(&mut self, _q: &str, _ctx: &ProviderCtx<'_>, _limit: usize) -> Vec<Candidate> {
            self.scores
                .iter()
                .map(|&score| Candidate {
                    label: format!("{}-{score}", self.name),
                    detail: String::new(),
                    score,
                    activation: Activation::Report(self.name.to_string()),
                })
                .collect()
        }
    }

    fn set(providers: Vec<Fake>) -> ProviderSet {
        let mut s = ProviderSet::default();
        for p in providers {
            s.push(Box::new(p));
        }
        s
    }

    #[test]
    fn the_best_answer_leads_whoever_produced_it() {
        // The property the whole scale exists for: a provider registered second
        // still leads when it is more certain. Asserted under both registration
        // orders, because a sort that happened to agree with registration order
        // would pass one of them by luck.
        let forward = set(vec![
            Fake {
                name: "apps",
                scores: vec![900],
            },
            Fake {
                name: "math",
                scores: vec![1000],
            },
        ])
        .query("q", &ProviderCtx::default(), 10);
        assert_eq!(forward[0].name, "math");

        let backward = set(vec![
            Fake {
                name: "math",
                scores: vec![1000],
            },
            Fake {
                name: "apps",
                scores: vec![900],
            },
        ])
        .query("q", &ProviderCtx::default(), 10);
        assert_eq!(backward[0].name, "math");
    }

    #[test]
    fn a_tie_keeps_registration_order() {
        // The only tie-break a user can predict. Without it the order depends on
        // the sort's internals and changes between runs of the same query.
        let groups = set(vec![
            Fake {
                name: "first",
                scores: vec![500],
            },
            Fake {
                name: "second",
                scores: vec![500],
            },
        ])
        .query("q", &ProviderCtx::default(), 10);
        assert_eq!(
            groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
            vec!["first", "second"]
        );
    }

    #[test]
    fn a_provider_with_nothing_to_say_is_not_shown() {
        // A heading over no rows is worse than no heading: it reads as "this
        // provider searched and found nothing", which is indistinguishable from
        // "this provider is broken".
        let groups = set(vec![
            Fake {
                name: "quiet",
                scores: vec![],
            },
            Fake {
                name: "loud",
                scores: vec![10],
            },
        ])
        .query("q", &ProviderCtx::default(), 10);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "loud");
    }

    #[test]
    fn rows_within_a_group_are_ordered_by_score() {
        let groups = set(vec![Fake {
            name: "apps",
            scores: vec![10, 900, 400],
        }])
        .query("q", &ProviderCtx::default(), 10);
        let scores: Vec<u32> = groups[0].rows.iter().map(|r| r.score).collect();
        assert_eq!(scores, vec![900, 400, 10]);
    }
}
