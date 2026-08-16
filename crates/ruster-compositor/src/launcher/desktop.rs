//! Reading XDG desktop entries, so the launcher knows what "Firefox" means.
//!
//! The parser takes *text*, not paths, and the PATH probe is injected — the same
//! shape `resolve_terminal` already uses — so the whole of it is testable
//! against literals without a filesystem or an installed application.
//!
//! The spec is large and mostly irrelevant here. What is implemented is what it
//! takes to launch the main entry correctly, and the rules below are not
//! stylistic: each one is a way this quietly produces a row that looks right and
//! runs the wrong thing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{Activation, Candidate, Provider, ProviderCtx};
use crate::lua::Action;
use ruster_picker::Fuzzy;

/// One installed application.
#[derive(Debug, Clone, PartialEq)]
pub struct DesktopEntry {
    pub name: String,
    /// `Exec` with field codes removed, ready to hand to `Action::Spawn`.
    pub exec: String,
    /// `GenericName` or `Comment` — what the row shows on the right.
    pub comment: String,
    /// Whether it needs a terminal wrapped around it.
    pub terminal: bool,
    /// The desktop file id (`firefox.desktop`), which is what de-duplication is
    /// keyed on: the same id in a higher-precedence directory replaces it.
    pub id: String,
}

/// Strip the field codes an `Exec` line may carry.
///
/// `%f %F %u %U` are the file and URL placeholders a launcher passes arguments
/// through; `%i %c %k` are the icon, translated name and file path. Launching
/// with them left in passes the literal text `%u` to the program as an argument.
/// `%%` is an escaped percent and survives as one.
pub fn strip_field_codes(exec: &str) -> String {
    let mut out = String::with_capacity(exec.len());
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => out.push('%'),
            // Any other code is dropped along with its `%`.
            Some(_) => {}
            None => out.push('%'),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Parse one desktop file's contents.
///
/// `None` for anything that is not a visible, launchable application.
pub fn parse_entry(id: &str, text: &str, installed: impl Fn(&str) -> bool) -> Option<DesktopEntry> {
    let mut fields: HashMap<&str, &str> = HashMap::new();
    let mut in_entry = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            // Only the `[Desktop Entry]` group. `[Desktop Action new-window]`
            // carries its own `Name=` and `Exec=`, and a parser that reads
            // straight through produces an entry named "New Window" that
            // launches the wrong command — a row that looks entirely right.
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        // Keys are matched exactly, which is what keeps a translation out of
        // the name: `Name[de]` is simply a different key from `Name`, so it is
        // never consulted. An explicit filter for `[` was here and mutation
        // testing removed it without a single test noticing — because it could
        // not change the outcome.
        //
        // First wins within the group, so a repeated key keeps its first value.
        fields.entry(key).or_insert(value);
    }

    if fields.get("Type") != Some(&"Application") {
        return None;
    }
    if fields.get("NoDisplay") == Some(&"true") || fields.get("Hidden") == Some(&"true") {
        return None;
    }
    // `TryExec` names the binary that must exist for the entry to be valid — a
    // package can leave its .desktop behind after an uninstall.
    if let Some(try_exec) = fields.get("TryExec") {
        let binary = try_exec.split_whitespace().next().unwrap_or(try_exec);
        let binary = Path::new(binary)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(binary);
        if !installed(binary) {
            return None;
        }
    }
    let name = fields.get("Name")?.to_string();
    let exec = strip_field_codes(fields.get("Exec")?);
    if name.is_empty() || exec.is_empty() {
        return None;
    }
    Some(DesktopEntry {
        name,
        exec,
        comment: fields
            .get("GenericName")
            .or_else(|| fields.get("Comment"))
            .unwrap_or(&"")
            .to_string(),
        terminal: fields.get("Terminal") == Some(&"true"),
        id: id.to_string(),
    })
}

/// The directories to search, in precedence order — highest first.
///
/// Per the XDG basedir spec: `$XDG_DATA_HOME` (or `~/.local/share`) beats
/// `$XDG_DATA_DIRS`, and earlier entries in that list beat later ones.
pub fn search_dirs(home_data: Option<PathBuf>, xdg_data_dirs: Option<&str>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = home_data {
        dirs.push(home.join("applications"));
    }
    let listed = xdg_data_dirs.unwrap_or("/usr/local/share:/usr/share");
    for dir in listed.split(':').filter(|d| !d.is_empty()) {
        dirs.push(Path::new(dir).join("applications"));
    }
    dirs
}

/// Read every desktop entry under `dirs`, first-wins by id.
pub fn scan(dirs: &[PathBuf], installed: impl Fn(&str) -> bool) -> Vec<DesktopEntry> {
    let mut seen: HashMap<String, DesktopEntry> = HashMap::new();
    for dir in dirs {
        let Ok(read) = std::fs::read_dir(dir) else {
            continue;
        };
        for file in read.flatten() {
            let path = file.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            let Some(id) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            // First wins: `dirs` is in precedence order, so an entry already
            // recorded came from a directory that outranks this one.
            if seen.contains_key(id) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Some(entry) = parse_entry(id, &text, &installed) {
                seen.insert(id.to_string(), entry);
            }
        }
    }
    let mut entries: Vec<DesktopEntry> = seen.into_values().collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

/// The command that actually launches an entry.
///
/// A `Terminal=true` entry is a program with no window of its own, so it needs
/// one wrapped around it. The terminal comes from `terminal_command`, which is
/// already the compositor's single answer to "which terminal" — config, then
/// `$TERMINAL`, then the first installed of a known list. Inventing a second
/// rule here is how the launcher and `M-Return` would come to disagree.
pub fn launch_command(entry: &DesktopEntry, terminal: Option<&str>) -> String {
    if !entry.terminal {
        return entry.exec.clone();
    }
    match terminal {
        Some(term) => format!("{term} -e {}", entry.exec),
        None => entry.exec.clone(),
    }
}

/// Applications, as a launcher provider.
pub struct AppsProvider {
    entries: Vec<DesktopEntry>,
    fuzzy: Fuzzy,
    loaded: bool,
    /// The configured terminal, resolved once per open. See `prepare`.
    terminal: Option<String>,
}

impl Default for AppsProvider {
    fn default() -> Self {
        AppsProvider {
            entries: Vec::new(),
            fuzzy: Fuzzy::new(),
            loaded: false,
            terminal: None,
        }
    }
}

impl AppsProvider {
    /// Build one over a known list, for tests.
    pub fn with_entries(entries: Vec<DesktopEntry>) -> Self {
        AppsProvider {
            entries,
            fuzzy: Fuzzy::new(),
            loaded: true,
            terminal: None,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Provider for AppsProvider {
    fn name(&self) -> &str {
        "apps"
    }

    fn prepare(&mut self) {
        // Resolved per open rather than per keystroke: `terminal_command` walks
        // `PATH` looking for a binary, and `query` runs on every character typed
        // into the launcher. Not hoisted all the way to construction, because a
        // config reload can change which terminal is configured and an open is
        // the last moment that answer can still be picked up.
        self.terminal = crate::lua::terminal_command(None).map(|(cmd, _)| cmd);
        if self.loaded {
            return;
        }
        // Synchronously, once, on the first open. Measured on this machine: 100
        // files, 564 KB, read in about a millisecond — so a thread and a channel
        // would be machinery in front of something already faster than the frame
        // it would be hiding behind. The timing is logged so that claim keeps
        // being checked rather than remembered.
        let started = std::time::Instant::now();
        let dirs = search_dirs(
            dirs::data_dir(),
            std::env::var("XDG_DATA_DIRS").ok().as_deref(),
        );
        self.entries = scan(&dirs, |binary| {
            std::env::var_os("PATH")
                .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(binary).is_file()))
                .unwrap_or(false)
        });
        self.loaded = true;
        tracing::info!(
            entries = self.entries.len(),
            dirs = dirs.len(),
            ms = started.elapsed().as_millis() as u64,
            "scanned desktop entries"
        );
    }

    fn query(&mut self, query: &str, _ctx: &ProviderCtx<'_>, limit: usize) -> Vec<Candidate> {
        let terminal = self.terminal.clone();
        let mut hits: Vec<Candidate> = Vec::new();
        for entry in &self.entries {
            // The name at full weight; the description at three quarters, so a
            // word appearing in a comment can surface an app without outranking
            // the app actually called that.
            let by_name = self.fuzzy.score(query, &entry.name);
            let by_comment = if entry.comment.is_empty() {
                None
            } else {
                self.fuzzy.score(query, &entry.comment).map(|s| s * 3 / 4)
            };
            let Some(score) = by_name.into_iter().chain(by_comment).max() else {
                continue;
            };
            hits.push(Candidate {
                label: entry.name.clone(),
                detail: entry.comment.clone(),
                score,
                activation: Activation::Action(Action::Spawn(launch_command(
                    entry,
                    terminal.as_deref(),
                ))),
            });
        }
        hits.sort_by_key(|c| std::cmp::Reverse(c.score));
        hits.truncate(limit);
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn always(_: &str) -> bool {
        true
    }

    #[test]
    fn an_action_group_does_not_become_the_entry() {
        // The failure that produces a plausible wrong row: `[Desktop Action ...]`
        // sections carry their own Name and Exec, and several entries on this
        // machine have them.
        //
        // `Terminal` and `Comment` are deliberately absent from the entry group
        // and present in the action group: asserting only on Name and Exec
        // proves nothing, because first-wins already keeps those. A key the
        // entry group never defines is the one that leaks.
        let text = "[Desktop Entry]\n\
                    Type=Application\n\
                    Name=Files\n\
                    Exec=nautilus --new-window %U\n\
                    Actions=new-window;\n\
                    \n\
                    [Desktop Action new-window]\n\
                    Name=New Window\n\
                    Comment=Leaked from an action\n\
                    Terminal=true\n\
                    Exec=nautilus --new-window-please\n";
        let entry = parse_entry("org.gnome.Nautilus.desktop", text, always).unwrap();
        assert_eq!(entry.name, "Files");
        assert_eq!(entry.exec, "nautilus --new-window");
        assert_eq!(entry.comment, "", "an action's Comment is not the entry's");
        assert!(!entry.terminal, "an action's Terminal is not the entry's");
    }

    #[test]
    fn a_translated_name_is_not_the_name() {
        // Firefox ships over a hundred `Name[xx]` lines. Reading them would take
        // whichever sorted last.
        let text = "[Desktop Entry]\nType=Application\nName=Firefox\nName[de]=Dateien\nName[ach]=Nope\nExec=firefox %u\n";
        let entry = parse_entry("firefox.desktop", text, always).unwrap();
        assert_eq!(entry.name, "Firefox");
        assert_eq!(entry.exec, "firefox");
    }

    #[test]
    fn entries_that_are_not_meant_to_be_shown_are_not() {
        let base = "[Desktop Entry]\nType=Application\nName=X\nExec=x\n";
        assert!(parse_entry("a.desktop", &format!("{base}NoDisplay=true\n"), always).is_none());
        assert!(parse_entry("a.desktop", &format!("{base}Hidden=true\n"), always).is_none());
        assert!(parse_entry(
            "a.desktop",
            "[Desktop Entry]\nType=Link\nName=X\nURL=http://x\n",
            always
        )
        .is_none());
        assert!(parse_entry("a.desktop", base, always).is_some());
    }

    #[test]
    fn an_entry_whose_binary_is_gone_is_skipped() {
        // A package can leave its .desktop behind after an uninstall, and a row
        // that launches nothing is worse than no row.
        let text =
            "[Desktop Entry]\nType=Application\nName=Ghost\nTryExec=/usr/bin/ghost\nExec=ghost\n";
        assert!(parse_entry("g.desktop", text, |b| b == "ghost").is_some());
        assert!(parse_entry("g.desktop", text, |_| false).is_none());
    }

    #[test]
    fn field_codes_go_and_escaped_percents_stay() {
        assert_eq!(strip_field_codes("firefox %u"), "firefox");
        assert_eq!(strip_field_codes("gimp %U %i %c"), "gimp");
        assert_eq!(strip_field_codes("foo %% bar"), "foo % bar");
        assert_eq!(
            strip_field_codes("steam steam://open/games"),
            "steam steam://open/games"
        );
    }

    #[test]
    fn a_terminal_program_is_given_a_terminal() {
        let entry = DesktopEntry {
            name: "htop".into(),
            exec: "htop".into(),
            comment: String::new(),
            terminal: true,
            id: "htop.desktop".into(),
        };
        assert_eq!(launch_command(&entry, Some("foot")), "foot -e htop");
        // With no terminal on the machine, launching it raw at least tries.
        assert_eq!(launch_command(&entry, None), "htop");
        let gui = DesktopEntry {
            terminal: false,
            ..entry
        };
        assert_eq!(launch_command(&gui, Some("foot")), "htop");
    }

    #[test]
    fn the_higher_precedence_directory_wins() {
        let dir = std::env::temp_dir().join(format!("ruster-desktop-{}", std::process::id()));
        let (home, system) = (dir.join("home"), dir.join("system"));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&system).unwrap();
        let entry =
            |name: &str| format!("[Desktop Entry]\nType=Application\nName={name}\nExec={name}\n");
        std::fs::write(home.join("thing.desktop"), entry("Mine")).unwrap();
        std::fs::write(system.join("thing.desktop"), entry("Theirs")).unwrap();
        std::fs::write(system.join("other.desktop"), entry("Other")).unwrap();

        let found = scan(&[home.clone(), system.clone()], always);
        let names: Vec<&str> = found.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["Mine", "Other"],
            "the user's own entry replaces the system one, and both files are read"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_name_the_user_typed_the_start_of_comes_first() {
        let e = |name: &str, comment: &str| DesktopEntry {
            name: name.into(),
            exec: name.to_ascii_lowercase(),
            comment: comment.into(),
            terminal: false,
            id: format!("{name}.desktop"),
        };
        let mut p = AppsProvider::with_entries(vec![
            e("Thunderbird", "Mail Client"),
            e("Firefox", "Web Browser"),
            e("Files", "Browse the file system"),
        ]);
        let hits = p.query("fire", &ProviderCtx::default(), 10);
        assert_eq!(hits[0].label, "Firefox");
        assert!(
            hits[0].score >= 900,
            "a prefix belongs in the prefix band, got {}",
            hits[0].score
        );
        assert_eq!(
            hits[0].activation,
            Activation::Action(Action::Spawn("firefox".into()))
        );
    }

    #[test]
    fn the_terminal_resolved_on_open_reaches_the_row_that_needs_one() {
        // `launch_command` is tested directly above; what this covers is the
        // wiring between them, which is the part that just changed. The lookup
        // walks `PATH`, so it moved out of `query` — which runs per keystroke —
        // and into `prepare`, which runs per open. Cached in the wrong place it
        // goes stale; not cached at all it is a `PATH` walk per character; and
        // dropped on the floor it silently launches a terminal program with no
        // terminal, which looks like the program failing to start.
        let htop = DesktopEntry {
            name: "htop".into(),
            exec: "htop".into(),
            comment: String::new(),
            terminal: true,
            id: "htop.desktop".into(),
        };
        let mut p = AppsProvider::with_entries(vec![htop.clone()]);
        p.terminal = Some("foot".into());
        let hits = p.query("htop", &ProviderCtx::default(), 10);
        assert_eq!(
            hits[0].activation,
            Activation::Action(Action::Spawn("foot -e htop".into())),
            "the row must carry the terminal, not just know one exists"
        );

        // The half above sets the field by hand, so on its own it leaves
        // `prepare` free to resolve nothing at all — a mutation that survived
        // until this was added. Guarding it needs the real lookup, so it is
        // skipped where there is no terminal to find rather than asserting
        // something about the machine it happens to run on.
        if crate::lua::terminal_command(None).is_some() {
            let mut opened = AppsProvider::with_entries(Vec::new());
            opened.prepare();
            assert!(
                opened.terminal.is_some(),
                "opening the launcher must resolve the terminal, not just make room for one"
            );
        } else {
            eprintln!("skipping the resolve half: no terminal is installed here");
        }

        // And with no terminal found, the raw command — which at least tries.
        let mut bare = AppsProvider::with_entries(vec![htop]);
        bare.terminal = None;
        let hits = bare.query("htop", &ProviderCtx::default(), 10);
        assert_eq!(
            hits[0].activation,
            Activation::Action(Action::Spawn("htop".into()))
        );
    }

    #[test]
    fn a_word_in_the_description_finds_an_app_without_outranking_its_name() {
        let e = |name: &str, comment: &str| DesktopEntry {
            name: name.into(),
            exec: name.to_ascii_lowercase(),
            comment: comment.into(),
            terminal: false,
            id: format!("{name}.desktop"),
        };
        let mut p = AppsProvider::with_entries(vec![
            e("Thunderbird", "Mail Client"),
            e("Mail", "Send messages"),
        ]);
        let hits = p.query("mail", &ProviderCtx::default(), 10);
        assert_eq!(
            hits[0].label, "Mail",
            "the app actually called Mail leads the one merely described as mail"
        );
        assert!(hits.len() == 2, "but the other is still offered");
    }
}
