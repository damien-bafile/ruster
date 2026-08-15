//! The Super+Space launcher: a query engine with pluggable providers.
//!
//! Applications are the first provider, not the point of it. The request was for
//! something expandable — maths was the named example — so the shape is a query
//! fanned out to providers that score on one scale, with the results grouped
//! under their headings.
//!
//! Every row resolves to an [`Action`](crate::lua::Action), the same funnel a
//! keybind, the `:` prompt and `ruster.wm.*` go through. `minibuffer.rs` states
//! the rule this inherits: no route into the WM can do something the others
//! cannot.

pub mod math;
pub mod provider;

pub use provider::{
    order_groups, Activation, Candidate, Group, Provider, ProviderCtx, ProviderSet,
};
