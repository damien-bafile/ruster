//! `:help` — the manual, read out of the manual (Phase 7 Task 2).
//!
//! The text is `docs/keybindings.md` and `docs/config-reference.md`, embedded
//! at build time. Deliberately not a second copy: those two files are already
//! the reference, and since Phase 6 Task 17 they are *enforced* —
//! `tests/docs_in_sync.rs` fails CI if a command or setting exists without a
//! row in them. So help cannot drift from the code without breaking the build,
//! which is a stronger guarantee than any hand-written help text would have.
//!
//! Everything here is pure: [`document`] builds the text and [`resolve`] finds
//! the line a topic starts on, so both test without an editor.

/// The embedded manuals, in the order they appear in the help buffer.
const KEYBINDINGS: &str = include_str!("../../../docs/keybindings.md");
const CONFIG: &str = include_str!("../../../docs/config-reference.md");

/// The whole help text.
pub fn document() -> String {
    format!(
        "# ruster help\n\
         #\n\
         # `:help <topic>` jumps to a section, a `:command`, or a `setting.key`.\n\
         # `/` searches, `q` closes.\n\
         \n{KEYBINDINGS}\n\n{CONFIG}\n"
    )
}

/// Every heading in the document, as `(line, title)` — the topic list.
pub fn headings(doc: &str) -> Vec<(usize, String)> {
    doc.lines()
        .enumerate()
        .filter_map(|(i, l)| {
            let t = l.trim_start_matches('#').trim();
            (l.starts_with('#') && !t.is_empty()).then(|| (i, t.to_string()))
        })
        .collect()
}

/// The line `query` should jump to, or `None` when nothing matches.
///
/// Tried in order of how specific the match is, so `:help git` lands on the Git
/// *section* rather than the first row that happens to mention git:
///
/// 1. a heading, exactly then by prefix;
/// 2. a `:command` row — with or without the leading colon;
/// 3. a `setting.key` row;
/// 4. any line containing the text.
///
/// All comparisons are case-insensitive: `:help Mason` and `:help mason` are
/// the same question.
pub fn resolve(doc: &str, query: &str) -> Option<usize> {
    let q = query.trim().trim_start_matches(':').to_lowercase();
    if q.is_empty() {
        return Some(0);
    }
    let heads = headings(doc);

    if let Some((line, _)) = heads.iter().find(|(_, t)| t.to_lowercase() == q) {
        return Some(*line);
    }
    if let Some((line, _)) = heads.iter().find(|(_, t)| t.to_lowercase().starts_with(&q)) {
        return Some(*line);
    }

    // A command row writes the command in backticks with its colon, so look for
    // the whole token rather than a substring — `:w` must not match `:wq`.
    let cmd = format!("`:{q}`");
    if let Some(line) = doc.lines().position(|l| l.to_lowercase().contains(&cmd)) {
        return Some(line);
    }
    // A setting row is `group.key`, likewise in backticks.
    let setting = format!("`{q}`");
    if let Some(line) = doc.lines().position(|l| l.to_lowercase().contains(&setting)) {
        return Some(line);
    }
    doc.lines().position(|l| l.to_lowercase().contains(&q))
}

/// The topics `:help` can be asked about, for completion and for the
/// "no such topic" message. Headings only — the useful, browsable set.
pub fn topic_names(doc: &str) -> Vec<String> {
    let mut names: Vec<String> = headings(doc).into_iter().map(|(_, t)| t).collect();
    names.sort();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_document_embeds_both_manuals() {
        let d = document();
        assert!(d.contains("Commands & Keybindings"), "keybindings.md is in there");
        assert!(d.contains("Config Reference"), "config-reference.md too");
        assert!(d.starts_with("# ruster help"), "with a preamble saying how to use it");
    }

    /// The whole point: help is generated from the files CI already keeps in
    /// sync with the code, so a new command cannot be missing from help without
    /// also failing `docs_in_sync`.
    #[test]
    fn help_covers_commands_that_shipped_recently() {
        let d = document();
        for cmd in [":Mason", ":Diffview", ":SyntaxReload", ":Trouble", ":SessionSave"] {
            assert!(d.contains(cmd), "{cmd} is missing from help");
        }
    }

    #[test]
    fn an_empty_topic_goes_to_the_top() {
        let d = document();
        assert_eq!(resolve(&d, ""), Some(0));
        assert_eq!(resolve(&d, "   "), Some(0));
    }

    #[test]
    fn a_heading_resolves_to_its_own_line() {
        let d = "# One\nbody\n## Two Words\nmore\n";
        assert_eq!(resolve(d, "One"), Some(0));
        assert_eq!(resolve(d, "Two Words"), Some(2));
        // Case does not matter.
        assert_eq!(resolve(d, "two words"), Some(2));
        // A prefix is enough when nothing matches exactly.
        assert_eq!(resolve(d, "Two"), Some(2));
    }

    /// An exact heading beats a prefix, so `:help Windows` does not land on
    /// "Windows & buffers" when "Windows" exists.
    #[test]
    fn an_exact_heading_wins_over_a_prefix() {
        let d = "# Windows & buffers\nx\n# Windows\ny\n";
        assert_eq!(resolve(d, "Windows"), Some(2), "the exact one");
    }

    #[test]
    fn a_command_resolves_with_or_without_its_colon() {
        let d = document();
        let by_colon = resolve(&d, ":Mason").expect("found");
        assert_eq!(resolve(&d, "Mason"), Some(by_colon), "the colon is optional");
        assert!(d.lines().nth(by_colon).unwrap().contains("Mason"));
    }

    /// A command token must match whole: `:w` must not land on `:wq`.
    #[test]
    fn a_short_command_does_not_match_a_longer_one() {
        let d = "| `:wq` | write and quit |\n| `:w` | write |\n";
        assert_eq!(resolve(d, ":w"), Some(1), "the `:w` row, not the `:wq` row above it");
    }

    #[test]
    fn a_setting_resolves_to_its_row() {
        let d = document();
        let line = resolve(&d, "session.autoload").expect("documented");
        assert!(d.lines().nth(line).unwrap().contains("session.autoload"), "lands on the row");
    }

    #[test]
    fn an_unknown_topic_resolves_to_nothing() {
        let d = document();
        assert_eq!(resolve(&d, "definitely-not-a-topic-xyz"), None);
    }

    #[test]
    fn the_topic_list_is_sorted_and_free_of_duplicates() {
        let names = topic_names(&document());
        assert!(names.len() > 10, "the manual has plenty of sections");
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(names, sorted);
    }

    #[test]
    fn headings_ignore_empty_hashes_and_body_text() {
        let d = "#\n# Real\nnot a heading\n#   \n";
        assert_eq!(headings(d), vec![(1, "Real".to_string())]);
    }
}
