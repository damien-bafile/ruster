//! Enforces the doc-sync rule in `AGENTS.md`: every `:` command the parser
//! accepts must appear in `docs/keybindings.md`, and every setting the schema
//! declares must appear in `docs/config-reference.md`.
//!
//! This exists because the rule kept being broken. The Phase 5 debugger shipped
//! undocumented for a week, and a Phase 6 audit found 13 more commands with no
//! mention anywhere — `:Noice`, the whole `:echo`/`:echom`/`:echoe` family,
//! `:sidebar resize`. A convention that is only checked by remembering is not
//! checked. This turns it into a failing test.
//!
//! The command list is scraped from the source of `parse_cmdline` rather than
//! kept as a second list here, because a hand-maintained list would drift in
//! exactly the same way and hide the same gap.

/// The body of `parse_cmdline`, where every accepted command literal lives.
fn parse_cmdline_body() -> &'static str {
    const SRC: &str = include_str!("../src/app.rs");
    let start = SRC.find("fn parse_cmdline").expect("parse_cmdline exists");
    let rest = &SRC[start..];
    // Ends at the next method on the same impl.
    let end = rest[200..].find("\n    fn ").map(|i| i + 200).unwrap_or(rest.len());
    &rest[..end]
}

/// Command literals the parser matches, e.g. `"Diffview" | "diff" =>` and
/// `strip_prefix("sidebar resize ")`.
fn command_literals(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (i, _) in body.match_indices('"') {
        let rest = &body[i + 1..];
        let Some(close) = rest.find('"') else { continue };
        let lit = &rest[..close];
        // A command literal is alphanumeric plus the few separators commands
        // use; anything else is a message, a format string or a path.
        if lit.len() < 2
            || !lit.starts_with(|c: char| c.is_ascii_alphabetic())
            || !lit.chars().all(|c| c.is_ascii_alphanumeric() || " /!-_".contains(c))
        {
            continue;
        }
        // Only count it if it is being matched on, not merely mentioned.
        let after = rest[close + 1..].trim_start();
        let before = body[..i].trim_end();
        let matched = after.starts_with("|") || after.starts_with("=>");
        let prefixed = before.ends_with("strip_prefix(") || before.ends_with("starts_with(");
        if matched || prefixed {
            out.push(lit.trim().to_string());
        }
    }
    out.sort();
    out.dedup();
    out
}

fn doc(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs").join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .to_lowercase()
}

#[test]
fn every_command_the_parser_accepts_is_documented() {
    let body = parse_cmdline_body();
    let literals = command_literals(body);
    assert!(literals.len() > 50, "the scrape found only {} — it has broken", literals.len());

    let docs = doc("keybindings.md");
    let missing: Vec<&String> = literals.iter().filter(|l| !docs.contains(&l.to_lowercase())).collect();
    assert!(
        missing.is_empty(),
        "{} command(s) are accepted by :parse_cmdline but absent from docs/keybindings.md:\n{}\n\
         Add a row for each, or drop the command. See the doc-sync rule in AGENTS.md.",
        missing.len(),
        missing.iter().map(|m| format!("  :{m}")).collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn every_setting_in_the_schema_is_documented() {
    let docs = doc("config-reference.md");
    let missing: Vec<String> = ruster_lua::schema::schema()
        .iter()
        .map(|s| format!("{}.{}", s.group, s.key))
        .filter(|key| {
            // Colour settings are documented as a group rather than one row per
            // element — there is a row per *palette role*, not per key.
            !key.starts_with("colors.")
                && !docs.contains(&key.to_lowercase())
                // Several rows document a family together, e.g.
                // "`gui.padding_x` / `padding_y`".
                && !docs.contains(&key.rsplit('.').next().unwrap().to_lowercase())
        })
        .collect();
    assert!(
        missing.is_empty(),
        "{} setting(s) in the schema are absent from docs/config-reference.md:\n{}",
        missing.len(),
        missing.iter().map(|m| format!("  {m}")).collect::<Vec<_>>().join("\n")
    );
}

/// The other direction: the schema and the config reader must agree, or a
/// setting is either unreachable from `config.lua` or invisible in Settings.
#[test]
fn the_schema_and_the_config_reader_declare_the_same_keys() {
    const CFG: &str = include_str!("../../ruster-lua/src/config.rs");
    let schema: std::collections::BTreeSet<String> = ruster_lua::schema::schema()
        .iter()
        .map(|s| format!("{}.{}", s.group, s.key))
        .collect();
    let unread: Vec<&String> = schema
        .iter()
        .filter(|key| {
            let (g, k) = key.split_once('.').unwrap();
            !CFG.contains(&format!("\"{g}\", \"{k}\""))
        })
        .collect();
    assert!(
        unread.is_empty(),
        "{} setting(s) are declared in the schema but never read by config.rs, so \
         setting them in config.lua does nothing:\n{}",
        unread.len(),
        unread.iter().map(|m| format!("  {m}")).collect::<Vec<_>>().join("\n")
    );
}
