//! Every `:` command the UI offers the user must be one the parser accepts.
//!
//! `docs_in_sync.rs` checks the other direction — commands the parser accepts
//! must be documented — and `commands_discoverable.rs` checks they are bound or
//! declared typed-only. Neither noticed that the dashboard's Quick Actions
//! panel had been telling every new user to type `:FuzzySearch`, which no
//! branch of `parse_cmdline` has ever accepted; the real command is `:Files`.
//! It sat there in both backends, on the first screen anyone sees, answering
//! "Unknown command" to the one instruction the editor volunteered.
//!
//! Scraped from the widget source rather than kept as a list here, for the same
//! reason as the sibling tests: a hand-maintained copy drifts the same way and
//! hides the same gap.

/// The body of `parse_cmdline`, where every accepted command literal lives.
fn parse_cmdline_body() -> &'static str {
    const SRC: &str = include_str!("../src/app.rs");
    let start = SRC.find("fn parse_cmdline").expect("parse_cmdline exists");
    let rest = &SRC[start..];
    let end = rest[200..]
        .find("\n    fn ")
        .map(|i| i + 200)
        .unwrap_or(rest.len());
    &rest[..end]
}

/// `:`-prefixed literals a source file offers to the user, e.g. `":Dired"`.
///
/// The leading colon is what distinguishes an instruction to the user from an
/// ordinary string, which is why the panels write them that way.
fn advertised(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (i, _) in src.match_indices("\":") {
        let rest = &src[i + 2..];
        let Some(close) = rest.find('"') else {
            continue;
        };
        let lit = &rest[..close];
        // Take the command word only: `:e <path>` advertises `e`.
        let word = lit.split_whitespace().next().unwrap_or("");
        if word.is_empty() || !word.starts_with(|c: char| c.is_ascii_alphabetic()) {
            continue;
        }
        if !word.chars().all(|c| c.is_ascii_alphanumeric()) {
            continue;
        }
        out.push(word.to_string());
    }
    out.sort();
    out.dedup();
    out
}

#[test]
fn every_command_the_dashboard_advertises_is_one_the_parser_accepts() {
    let body = parse_cmdline_body();

    let panels: [(&str, &str); 2] = [
        (
            "crates/ruster-tui/src/widgets/mod.rs",
            include_str!("../src/widgets/mod.rs"),
        ),
        (
            "crates/ruster-render-raylib/src/lib.rs",
            include_str!("../../ruster-render-raylib/src/lib.rs"),
        ),
    ];

    let mut missing: Vec<String> = Vec::new();
    for (name, src) in panels {
        // Only the Quick Actions table, not every `":..."` in the file.
        let Some(start) = src.find("Find Files") else {
            panic!("{name}: the Quick Actions panel moved — this scrape needs updating");
        };
        let from = src[..start].rfind("Open file").unwrap_or(0);
        let to = (start + 400).min(src.len());
        let window = &src[from..to];

        for cmd in advertised(window) {
            // The parser matches on the bare word, without the colon.
            let quoted = format!("\"{cmd}\"");
            if !body.contains(&quoted) {
                missing.push(format!("  {name}: :{cmd}"));
            }
        }
    }

    assert!(
        missing.is_empty(),
        "{} command(s) are advertised to the user but rejected by parse_cmdline:\n{}\n\
         The dashboard is the first screen anyone sees; a command it names must work.",
        missing.len(),
        missing.join("\n")
    );
}

/// The commands the `:`-Tab / `M-x` palette offers, as written there.
///
/// Kept as whole strings rather than first words: several are multi-word
/// (`set number`), and truncating them to `set` would make the check pass for a
/// palette entry of `set nonsense`.
fn palette_commands() -> Vec<String> {
    const SRC: &str = include_str!("../src/app.rs");
    let start = SRC
        .find("const PALETTE_COMMANDS")
        .expect("PALETTE_COMMANDS exists");
    let body = &SRC[start..];
    let end = body.find("\n];").unwrap_or(body.len());
    body[..end]
        .lines()
        .filter_map(|l| l.trim().strip_prefix("(\""))
        .filter_map(|l| l.split_once('"'))
        .map(|(cmd, _)| cmd.to_string())
        .collect()
}

/// Whether `parse_cmdline` accepts `cmd`, including by prefix.
///
/// `:set number` is not a literal in the parser — it is reached through
/// `starts_with("set ")`. So every prefix of the command is a candidate, longest
/// first, which is also how the parser itself disambiguates
/// `set editmode ` from the general `set `.
///
/// A prefix match alone is too weak for `set`: it would accept `set nonsense`,
/// because the parser does route that to `parse_set_general` — which then
/// rejects it at runtime against the settings schema. So `set <option>` is
/// checked the way `parse_set_general` checks it, and the palette cannot
/// advertise an option that does not exist.
fn parser_accepts(body: &str, cmd: &str) -> bool {
    let words: Vec<&str> = cmd.split_whitespace().collect();
    let matched = (1..=words.len()).rev().find(|n| {
        let prefix = words[..*n].join(" ");
        body.contains(&format!("\"{prefix}\"")) || body.contains(&format!("\"{prefix} \""))
    });
    let Some(n) = matched else { return false };

    // Landed on the generic `set ` branch: validate the option name itself.
    if words[..n] == ["set"] && words.len() > 1 {
        return option_exists(words[1]);
    }
    true
}

/// Whether `:set <tok>` names a real setting, mirroring `parse_set_general`:
/// a `no` prefix negates a boolean, and `?`/`&`/`=value` suffixes query, reset
/// or assign.
fn option_exists(tok: &str) -> bool {
    let k = tok
        .split('=')
        .next()
        .unwrap_or(tok)
        .trim_end_matches(['?', '&', '!']);
    let k = k.strip_prefix("no").filter(|s| !s.is_empty()).unwrap_or(k);
    ruster_lua::schema::spec_by_key(k).is_some() || ruster_lua::schema::spec_by_key(tok).is_some()
}

#[test]
fn every_command_the_palette_offers_is_one_the_parser_accepts() {
    // The palette is the discovery surface for anyone who does not already know
    // a command's name — it is the only place the editor volunteers a command
    // *with a description*. An entry the parser rejects answers "Unknown
    // command" to a suggestion the editor itself made, which is exactly what
    // :FuzzySearch did on the dashboard.
    let body = parse_cmdline_body();
    let commands = palette_commands();

    assert!(
        commands.len() > 10,
        "palette scrape found {} commands — the scrape has broken, not the code",
        commands.len()
    );

    let missing: Vec<String> = commands
        .iter()
        .filter(|c| !parser_accepts(body, c))
        .map(|c| format!("  :{c}"))
        .collect();

    assert!(
        missing.is_empty(),
        "{} palette command(s) are offered but rejected by parse_cmdline:\n{}",
        missing.len(),
        missing.join("\n")
    );
}
