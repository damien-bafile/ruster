//! The pseudo-languages — `diff`, `signs`, `dired`, `flash` — must behave like
//! real ones.
//!
//! Nothing parses them; they exist so that colours which used to be hardcoded
//! at their draw site go through the same per-language override machinery as
//! syntax groups, and so appear in the Settings editor and honour
//! `ruster.config.syntax.<lang>.*` without a second theming system.
//!
//! That only holds if three things stay true, and each is easy to break by
//! adding a group and forgetting a step:
//!
//! 1. Every group the Settings editor lists resolves to a real colour.
//! 2. An override actually reaches the style the drawing code asks for.
//! 3. The groups are distinguishable, or the colour is decorative.

use ruster_render::Color;

/// The function a pseudo-language's drawing code calls to resolve a group.
type StyleFn = fn(&str) -> ruster_render::SyntaxStyle;

/// The pseudo-languages and the accessor each one's drawing code calls.
fn accessors() -> Vec<(&'static str, StyleFn)> {
    vec![
        ("diff", ruster_syntax::diff_style as StyleFn),
        ("signs", ruster_syntax::sign_style),
        ("dired", ruster_syntax::dired_style),
        ("flash", ruster_syntax::flash_style),
    ]
}

/// Overrides live in one process-global map, so these tests cannot run in
/// parallel with each other — one clearing the map while another has just
/// written to it looks exactly like the bug they are meant to catch. Every test
/// below takes this first.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Take the lock and start from a clean map. Returns the guard, which must be
/// held for the body of the test.
fn exclusive() -> std::sync::MutexGuard<'static, ()> {
    // A panicking test poisons the lock; the state it left behind is cleared on
    // the next line anyway, so recover rather than cascading failures.
    let g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    clear_overrides();
    g
}

fn clear_overrides() {
    ruster_syntax::set_syntax_overrides(ruster_syntax::SyntaxOverrides::new());
}

#[test]
fn every_group_offered_in_settings_resolves_to_a_colour() {
    // A group listed in the editor whose style falls through to the `_` arm
    // renders as `Color::Default` — the knob is there, it just does nothing.
    let _g = exclusive();
    for lang in ruster_syntax::highlighted_languages() {
        for group in ruster_syntax::groups_for_lang(lang) {
            assert_ne!(
                ruster_syntax::default_fg_for(lang, group),
                Color::Default,
                "{lang}.{group} is listed in the Settings syntax editor but has no \
                 default colour, so the setting exists and changes nothing"
            );
        }
    }
}

#[test]
fn the_accessor_and_the_settings_list_agree() {
    // The other direction: the function the drawing code calls must know every
    // group the editor offers. A typo in either list shows up here rather than
    // as a silently uncoloured glyph.
    let _g = exclusive();
    for (lang, style_of) in accessors() {
        for group in ruster_syntax::groups_for_lang(lang) {
            assert_ne!(
                style_of(group).fg,
                Color::Default,
                "{lang}.{group} is offered in Settings but {lang}'s style function \
                 does not handle it"
            );
        }
    }
}

#[test]
fn an_override_reaches_every_group() {
    let _g = exclusive();
    for (lang, style_of) in accessors() {
        for group in ruster_syntax::groups_for_lang(lang) {
            clear_overrides();
            let mut ov = ruster_syntax::SyntaxOverrides::new();
            let mut groups = std::collections::HashMap::new();
            groups.insert((*group).to_string(), Color::Rgb(1, 2, 3));
            ov.insert(lang.to_string(), groups);
            ruster_syntax::set_syntax_overrides(ov);

            assert_eq!(
                style_of(group).fg,
                Color::Rgb(1, 2, 3),
                "ruster.config.syntax.{lang}.{group} did not reach the drawing code"
            );
        }
    }
    clear_overrides();
}

#[test]
fn an_override_on_one_language_does_not_leak_into_another() {
    // `signs.added`, `diff.added` and `dired.directory` share a namespace of
    // group names. Overrides are keyed by language, and this is what says so.
    let _g = exclusive();
    let mut ov = ruster_syntax::SyntaxOverrides::new();
    let mut groups = std::collections::HashMap::new();
    groups.insert("added".to_string(), Color::Rgb(1, 2, 3));
    ov.insert("diff".to_string(), groups);
    ruster_syntax::set_syntax_overrides(ov);

    assert_eq!(ruster_syntax::diff_style("added").fg, Color::Rgb(1, 2, 3));
    assert_ne!(
        ruster_syntax::sign_style("added").fg,
        Color::Rgb(1, 2, 3),
        "overriding diff.added also changed the git gutter's added sign"
    );
    clear_overrides();
}

#[test]
fn the_groups_within_a_language_are_distinguishable() {
    // Two groups sharing a colour is a legitimate choice — `signs.removed` and
    // `signs.error` are both the same red on purpose, since both mean
    // "something is wrong here". What is not legitimate is a whole language
    // collapsing to one colour, which is what a copy-paste slip in the match
    // arms produces.
    let _g = exclusive();
    for (lang, style_of) in accessors() {
        let groups = ruster_syntax::groups_for_lang(lang);
        let distinct: std::collections::BTreeSet<String> =
            groups.iter().map(|g| format!("{:?}", style_of(g).fg)).collect();
        assert!(
            distinct.len() > 1,
            "every group in {lang} draws the same colour ({} groups, {} distinct)",
            groups.len(),
            distinct.len()
        );
    }
}

#[test]
fn a_style_function_does_not_depend_on_set_current_lang() {
    // These are called a few glyphs at a time from the render loop, not from
    // the highlight pass that owns the thread-local. Naming the language at the
    // lookup is what stops "forgot the setter" from silently dropping
    // overrides — the bug the `diff` accessor was one call away from having.
    let _g = exclusive();
    let mut ov = ruster_syntax::SyntaxOverrides::new();
    let mut groups = std::collections::HashMap::new();
    groups.insert("directory".to_string(), Color::Rgb(9, 9, 9));
    ov.insert("dired".to_string(), groups);
    ruster_syntax::set_syntax_overrides(ov);

    // Point the thread-local somewhere else entirely.
    ruster_syntax::set_current_lang("rust");
    assert_eq!(
        ruster_syntax::dired_style("directory").fg,
        Color::Rgb(9, 9, 9),
        "the override was missed because the thread-local named another language"
    );
    clear_overrides();
}
