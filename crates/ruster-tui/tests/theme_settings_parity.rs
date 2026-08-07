//! Every colour the editor draws with must be reachable from the Settings page.
//!
//! Three layers have to agree, and nothing but this test makes them:
//!
//! - `ruster_render::Theme` — the fields the backends actually draw with.
//! - `ruster_lua::config::ColorOverrides` — what a `config.lua` can set.
//! - `ruster_lua::schema::schema()` — what the Settings page lists.
//!
//! A colour in the first but not the second is themeable but unconfigurable. In
//! the second but not the third it is settable by hand and invisible in the UI,
//! which is the same as not existing for anyone who has not read the source. The
//! only way to notice either is to go looking, so this goes looking.
//!
//! The two struct field lists are scraped rather than reflected — Rust has no
//! runtime field introspection, and this is the same approach `docs_in_sync.rs`
//! and `commands_discoverable.rs` already take. Whitespace is collapsed first so
//! that a rustfmt pass cannot make a field look deleted.

use std::collections::BTreeSet;

const THEME: &str = include_str!("../../ruster-render/src/lib.rs");
const CONFIG: &str = include_str!("../../ruster-lua/src/config.rs");

/// The `pub <name>: <ty>,` fields of a named struct, in declaration order.
///
/// Deliberately narrow: it reads to the first `}` at column 0, so a struct
/// whose body contains a nested block would need more care. None of the two
/// here do, and a looser parser would be harder to trust.
fn struct_fields(src: &str, decl: &str) -> BTreeSet<String> {
    let body = match src.split_once(decl) {
        Some((_, rest)) => rest.split_once("\n}").map(|(b, _)| b).unwrap_or(rest),
        None => panic!("{decl} not found — the scrape has broken, not the code"),
    };
    body.lines()
        .filter_map(|l| l.trim().strip_prefix("pub "))
        .filter_map(|l| l.split_once(':'))
        .map(|(name, _)| name.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect()
}

fn theme_fields() -> BTreeSet<String> {
    struct_fields(THEME, "pub struct Theme {")
}

fn override_fields() -> BTreeSet<String> {
    struct_fields(CONFIG, "pub struct ColorOverrides {")
}

fn schema_colors() -> BTreeSet<String> {
    ruster_lua::schema::schema()
        .iter()
        .filter(|s| s.group == "colors")
        .map(|s| s.key.to_string())
        .collect()
}

#[test]
fn every_theme_colour_can_be_overridden() {
    let missing: Vec<_> = theme_fields()
        .difference(&override_fields())
        .cloned()
        .collect();
    assert!(
        missing.is_empty(),
        "{} colour(s) are drawn but cannot be set from config.lua — add a field to \
         ColorOverrides in crates/ruster-lua/src/config.rs:\n{}",
        missing.len(),
        missing.join("\n  ")
    );
}

#[test]
fn every_overridable_colour_appears_in_settings() {
    let missing: Vec<_> = override_fields()
        .difference(&schema_colors())
        .cloned()
        .collect();
    assert!(
        missing.is_empty(),
        "{} colour(s) can be set from config.lua but are absent from the Settings \
         page — add `add(\"colors\", ...)` in crates/ruster-lua/src/schema.rs:\n{}",
        missing.len(),
        missing.join("\n  ")
    );
}

#[test]
fn settings_does_not_offer_colours_nothing_draws() {
    // The other direction. A Settings row for a colour no theme carries is a
    // control that silently does nothing, which is worse than a missing one:
    // the user has every reason to believe they changed something.
    let extra: Vec<_> = schema_colors()
        .difference(&theme_fields())
        .cloned()
        .collect();
    assert!(
        extra.is_empty(),
        "{} colour setting(s) are offered but no Theme field receives them:\n{}",
        extra.len(),
        extra.join("\n  ")
    );
}

#[test]
fn the_scrapes_found_something_to_check() {
    // Each assertion above passes trivially if its scrape returns nothing, so a
    // silent parser break would read as three green tests. This is the canary.
    assert!(
        theme_fields().len() > 20,
        "Theme scrape found {} fields — the parser has broken",
        theme_fields().len()
    );
    assert_eq!(
        theme_fields().len(),
        schema_colors().len(),
        "the layers are the same size when they agree; a mismatch here means one \
         of the three assertions above should have fired"
    );
}
