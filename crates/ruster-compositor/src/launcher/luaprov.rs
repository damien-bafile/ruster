//! Providers written in Lua.
//!
//! One [`LuaProvider`] per registered callback, so each gets its own group
//! heading and its own place in the ordering — a single provider forwarding to
//! all of them would merge unrelated results under one name.
//!
//! Rows name an *action*, never a callback. That is the same rule
//! `ruster.context_menu.add` follows, and it is what keeps a Lua provider able
//! to do everything a keybind can and nothing more.

use super::{Activation, Candidate, Provider, ProviderCtx};
pub struct LuaProvider {
    name: String,
    index: usize,
}

impl LuaProvider {
    pub fn new(name: String, index: usize) -> Self {
        LuaProvider { name, index }
    }
}

impl Provider for LuaProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn query(&mut self, query: &str, ctx: &ProviderCtx<'_>, limit: usize) -> Vec<Candidate> {
        let Some(wm) = ctx.wm else {
            return Vec::new();
        };
        match wm.provider_query(self.index, query) {
            Ok(rows) => rows
                .into_iter()
                .take(limit)
                .map(|row| Candidate {
                    label: row.label,
                    detail: row.detail,
                    score: row.score,
                    activation: Activation::Action(row.action),
                })
                .collect(),
            Err(err) => {
                // Named, because "the launcher shows nothing" is otherwise
                // indistinguishable from "this provider is broken".
                tracing::warn!(provider = %self.name, %err, "launcher provider failed");
                Vec::new()
            }
        }
    }
}
