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
    let end = rest[200..].find("\n    fn ").map(|i| i + 200).unwrap_or(rest.len());
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
        let Some(close) = rest.find('"') else { continue };
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
        ("crates/ruster-tui/src/widgets/mod.rs", include_str!("../src/widgets/mod.rs")),
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
