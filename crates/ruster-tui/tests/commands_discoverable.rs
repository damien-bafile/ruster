//! Every `:` command must be *findable*, not just documented.
//!
//! `docs_in_sync.rs` already fails the build when a command has no row in
//! `keybindings.md`. That catches undocumented commands but not undiscoverable
//! ones: a user who does not read the docs meets ruster through `SPC`, and a
//! feature absent from the which-key tree may as well not exist.
//!
//! It had lapsed badly. The leader tree was built for Phase 5 and never grew,
//! so the whole Phase 7 git porcelain — status, commit, push, pull, staged
//! diff, hunk staging, diffview — was reachable only by typing its commands,
//! along with Mason, help, themes, sessions and the notification panel.
//!
//! This does not demand that *every* command have a binding. Plenty are typed
//! by nature: `:w`, `:q`, anything taking an argument. Those go on the
//! allow-list below, which is the point — adding a command now forces a
//! deliberate choice between binding it and declaring it typed-only, rather
//! than silently doing neither.

/// Commands that are typed, not bound, and why.
///
/// Keep the reason with the entry. "It has no binding" is not a reason; "it
/// takes an argument, so a keypress cannot express it" is.
const TYPED_ONLY: &[(&str, &str)] = &[
    // Muscle memory — every vi user types these, and a leader route would be
    // slower than the thing it replaced.
    ("Save", "`:w`"),
    ("SaveAs", "`:w <path>`, takes an argument"),
    ("SaveAndQuit", "`:wq`"),
    ("Quit", "`:q`"),
    ("ForceQuit", "`:q!`"),
    ("OpenFile", "takes a path"),
    ("Substitute", "`:s/a/b/`, takes a pattern"),
    ("Echo", "takes the text to show"),
    // Argument-taking: a keypress cannot supply the value.
    ("SetNamed", "`:set <opt>=<val>`"),
    ("ShowSetting", "`:set <opt>?`"),
    ("ResetSetting", "takes the setting name"),
    ("SetEditMode", "`:set editmode <paradigm>`"),
    ("MessagesFilter", "takes a filter"),
    ("SidebarResize", "takes a width"),
    ("Rg", "takes a pattern"),
    ("Screenshot", "takes an optional path"),
    ("Help", "bound as `SPC o h`; the argument form is typed"),
    // Reached by a dedicated key rather than the leader tree.
    ("QuickfixNext", "`]q`"),
    ("QuickfixPrev", "`[q`"),
    ("QuickfixOpen", "opened by the runner that fills it"),
    ("Sidebar", "`SPC e`"),
    ("Settings", "`SPC ,` and `SPC o s`"),
    // Maintenance commands, run deliberately and rarely.
    ("ConfigErrors", "diagnostic, run when something is wrong"),
    ("SyntaxReload", "run after editing a query by hand"),
    ("NoiceSplit", "the panel is `SPC o n`; the split form is typed"),
    ("Dired", "`SPC o e`; the path form is typed"),
    ("Ibuffer", "`SPC b b`"),
    ("CallHierarchy", "`SPC c i` / `SPC c y`"),
    ("WorkspaceSymbol", "`SPC s s` covers symbol search"),
    ("DebugNext", "`SPC d n`"),
];

/// Commands whose leader binding is named differently, and deliberately.
///
/// The convention is that a `LeaderAction` shares the `CmdAction`'s name. Where
/// it does not, say so here rather than adding the command to `TYPED_ONLY` and
/// claiming it is unbound — it is bound, just not by that name.
const ALIASES: &[(&str, &str)] = &[
    // `SPC o r` opens the task picker.
    ("TaskPicker", "Tasks"),
];

/// Every `CmdAction` variant, scraped from the enum.
fn command_actions() -> Vec<String> {
    const SRC: &str = include_str!("../src/app.rs");
    let start = SRC.find("enum CmdAction {").expect("CmdAction exists");
    let body = &SRC[start..];
    let end = body.find("\n}").expect("enum closes");
    body[..end]
        .lines()
        .filter_map(|l| {
            let t = l.trim();
            // `Variant,` or `Variant(..)` or `Variant {`, skipping doc comments.
            (!t.starts_with("//") && t.starts_with(|c: char| c.is_ascii_uppercase()))
                .then(|| t.trim_end_matches(',').split(['(', ' ', '{']).next().unwrap_or(t))
                .map(str::to_string)
        })
        .collect()
}

/// Every `LeaderAction` the which-key tree can actually reach.
///
/// Scraped from `LeaderNode::Action(...)` entries specifically, **not** from
/// every `LeaderAction::` mention. The enum declaration and the dispatch `match`
/// name every variant too, so a looser scrape proves only that the action
/// exists — it would pass with the tree entry deleted, which is exactly the
/// state this test is meant to catch. Verified by deleting a binding and
/// watching it fail.
fn leader_actions() -> Vec<String> {
    const SRC: &str = include_str!("../src/app.rs");
    // Line-based: each tree entry is one line, and a label may itself contain
    // parentheses ("mason (tools)"), so scanning to the first `)` truncates it.
    SRC.lines()
        .filter(|l| l.contains("LeaderNode::Action("))
        .filter_map(|l| {
            let at = l.find("LeaderAction::")? + "LeaderAction::".len();
            Some(l[at..].chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect())
        })
        .collect()
}

#[test]
fn every_command_is_bound_or_declared_typed_only() {
    let commands = command_actions();
    assert!(commands.len() > 40, "the scrape found only {} — it has broken", commands.len());
    let leader = leader_actions();
    assert!(leader.len() > 20, "the leader scrape found only {} — it has broken", leader.len());

    // A command counts as reachable when a LeaderAction shares its name, which
    // is the convention every existing binding follows.
    let typed: Vec<&str> = TYPED_ONLY.iter().map(|(n, _)| *n).collect();
    let aliased = |c: &str| {
        ALIASES.iter().any(|(cmd, leader_name)| *cmd == c && leader.iter().any(|l| l == leader_name))
    };
    let missing: Vec<&String> = commands
        .iter()
        .filter(|c| !leader.contains(c) && !typed.contains(&c.as_str()) && !aliased(c))
        .collect();

    assert!(
        missing.is_empty(),
        "{} command(s) have no which-key route and are not declared typed-only:\n{}\n\n\
         Either add a leader binding, or add it to TYPED_ONLY in this file with \
         the reason it is typed. A feature nobody can find may as well not exist.",
        missing.len(),
        missing.iter().map(|m| format!("  CmdAction::{m}")).collect::<Vec<_>>().join("\n")
    );
}

/// The allow-list must not rot: an entry for a command that no longer exists
/// hides the fact that nothing checks it any more.
#[test]
fn the_typed_only_list_has_no_stale_entries() {
    let commands = command_actions();
    let stale: Vec<&str> = TYPED_ONLY
        .iter()
        .map(|(n, _)| *n)
        .chain(ALIASES.iter().map(|(c, _)| *c))
        .filter(|n| !commands.iter().any(|c| c == n))
        .collect();
    assert!(
        stale.is_empty(),
        "TYPED_ONLY or ALIASES name commands that no longer exist: {stale:?}"
    );
}

/// Every entry carries a reason. "No binding" is not one.
#[test]
fn every_typed_only_entry_explains_itself() {
    for (name, why) in TYPED_ONLY {
        assert!(why.len() > 3, "{name} has no reason recorded");
    }
}
