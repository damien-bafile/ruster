//! A colour written at its draw site is a colour no theme can reach.
//!
//! Stage 1 of the finetuning plan removed eight of them, routing each through a
//! *pseudo-language* in `ruster-syntax` so it inherits the same per-language
//! override machinery as syntax groups — `signs`, `dired`, `flash` alongside
//! the `diff` groups that established the pattern.
//!
//! Nothing stops the next one being added the easy way, which is what this is
//! for. It is deliberately a source scrape rather than a render assertion: the
//! failure it guards against is a literal appearing in the code, and that is
//! the thing to look at.
//!
//! **A literal is not automatically wrong.** Most of the ones here are
//! fallbacks in `theme.map(..).unwrap_or(LITERAL)`, where the theme wins
//! whenever there is one — those are defaults, not hardcoding. The allow-list
//! records which is which, and why, so that adding a colour forces the same
//! deliberate choice `commands_discoverable.rs` forces for keybindings.

/// Files whose literals are all theme fallbacks, with the shape that makes them
/// so. Listed per file rather than per line: line numbers rot, and every
/// literal in each of these follows the same pattern.
const FALLBACK_ONLY: &[(&str, &str)] = &[
    (
        "widgets/mod.rs",
        "every literal is the `unwrap_or` arm of `c(fallback, |t| t.field)` or is \
         overwritten wholesale by `with_theme` before the widget draws",
    ),
    (
        "widgets/debug_overlay.rs",
        "same `c(|t| t.field, fallback)` shape — the theme wins when there is one",
    ),
    (
        "raylib/lib.rs",
        "every literal is the second argument of `to_raylib(theme.field, fallback)`, \
         so the theme wins whenever it carries that role",
    ),
];

/// Individual exceptions, none needed so far.
///
/// `app.rs` and `renderer.rs` were listed here at first, for their colour
/// parsing and conversion — `Color::Rgb(r, g, b)` forwarding a value from Lua
/// config or the terminal's ANSI palette. The matcher already excludes those:
/// it requires three *numeric* arguments, so a forwarded value is not a literal
/// and never needed an exemption. The stale-entry check below is what said so.
const ALLOWED: &[(&str, &str)] = &[];

/// Every `Rgb(1, 2, 3)` literal outside `#[cfg(test)]`, as (file, line, text).
fn literals() -> Vec<(String, usize, String)> {
    let mut out = Vec::new();
    for (name, src) in sources() {
        let body = &src[..test_module_start(&src)];
        for (n, line) in body.lines().enumerate() {
            let t = line.trim();
            if t.starts_with("//") || t.starts_with("///") {
                continue;
            }
            if has_rgb_literal(line) || has_rgba_literal(line) {
                out.push((name.clone(), n + 1, t.to_string()));
            }
        }
    }
    out
}

/// Where the trailing `#[cfg(test)] mod tests` begins, or the end of the file.
///
/// Test modules are allowed literals — asserting on a specific colour is how
/// several of these guards work — so the scrape stops there.
///
/// It must be the test *module*, not the first `#[cfg(test)]`. `dired.rs` has a
/// test-only accessor at line 156 and its test module at 470, so cutting at the
/// first attribute skipped 300 lines including every colour this test was
/// written to watch. It passed with a hardcoded colour put back by hand, which
/// is how that was found.
fn test_module_start(src: &str) -> usize {
    let mut from = 0;
    while let Some(rel) = src[from..].find("#[cfg(test)]") {
        let at = from + rel;
        let after = src[at + "#[cfg(test)]".len()..].trim_start();
        if after.starts_with("mod tests") {
            return at;
        }
        from = at + 1;
    }
    src.len()
}

/// `Rgb(` followed by three numeric arguments. Deliberately narrow: `Rgb(r, g,
/// b)` forwarding a parsed value is not a hardcoded colour.
fn has_rgb_literal(line: &str) -> bool {
    let Some(at) = line.find("Rgb(") else {
        return false;
    };
    let rest = &line[at + 4..];
    let Some(close) = rest.find(')') else {
        return false;
    };
    let args: Vec<&str> = rest[..close].split(',').map(str::trim).collect();
    args.len() == 3
        && args
            .iter()
            .all(|a| !a.is_empty() && a.chars().all(|c| c.is_ascii_digit()))
}

/// Every drawing source in both backends, found by walking rather than listed.
///
/// It was a hand-maintained list of nine `include_str!`s, which meant a new file
/// was invisible to this guard until somebody remembered to add it — and
/// remembering is exactly the thing the guard exists to not rely on. It also
/// covered only `ruster-tui`, so the raylib backend, the one people mean when
/// they say the GUI, was never checked at all.
///
/// Read at runtime from `CARGO_MANIFEST_DIR` because `include_str!` needs paths
/// known at compile time, which is the whole problem.
fn sources() -> Vec<(String, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    collect(&root.join("src"), "", &mut out);
    collect(
        &root.join("../ruster-render-raylib/src"),
        "raylib/",
        &mut out,
    );
    out.sort();
    assert!(
        out.len() >= 9,
        "the walk found {} sources; it used to be a list of 9, so something is wrong \
         with the traversal rather than the tree",
        out.len()
    );
    out
}

/// Recursively read every `.rs` under `dir`, naming each relative to it.
fn collect(dir: &std::path::Path, prefix: &str, out: &mut Vec<(String, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        panic!("cannot read {} — the crate layout moved", dir.display());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            collect(&path, &format!("{prefix}{name}/"), out);
        } else if name.ends_with(".rs") {
            match std::fs::read_to_string(&path) {
                Ok(src) => out.push((format!("{prefix}{name}"), src)),
                Err(e) => panic!("cannot read {}: {e}", path.display()),
            }
        }
    }
}

/// `Color::new(` followed by four numeric arguments — raylib's colour form.
/// Same reasoning as [`has_rgb_literal`]: four *numbers*, so a forwarded value
/// is not a literal.
fn has_rgba_literal(line: &str) -> bool {
    let Some(at) = line.find("Color::new(") else {
        return false;
    };
    let rest = &line[at + "Color::new(".len()..];
    let Some(close) = rest.find(')') else {
        return false;
    };
    let args: Vec<&str> = rest[..close].split(',').map(str::trim).collect();
    args.len() == 4
        && args
            .iter()
            .all(|a| !a.is_empty() && a.chars().all(|c| c.is_ascii_digit()))
}

#[test]
fn no_drawing_code_picks_a_colour_a_theme_cannot_reach() {
    let allowed: Vec<&str> = FALLBACK_ONLY
        .iter()
        .chain(ALLOWED.iter())
        .map(|(f, _)| *f)
        .collect();
    let offending: Vec<String> = literals()
        .into_iter()
        .filter(|(file, _, _)| !allowed.contains(&file.as_str()))
        .map(|(file, line, text)| format!("  {file}:{line}  {text}"))
        .collect();

    assert!(
        offending.is_empty(),
        "{} hardcoded colour(s) that no theme or `ruster.config.syntax.*` can reach:\n{}\n\n\
         Route it through a pseudo-language in `ruster-syntax` — `signs`, `dired`, \
         `flash` or `diff` — so it appears in the Settings syntax editor like every \
         other group. If it is genuinely a theme fallback or a colour being parsed \
         rather than chosen, add the file to ALLOWED in this test with the reason.",
        offending.len(),
        offending.join("\n")
    );
}

/// The scrape must be able to see something, or the test above passes because
/// it is looking at nothing.
#[test]
fn the_scrape_still_finds_literals() {
    let found = literals();
    assert!(
        found.len() > 20,
        "the scrape found only {} literals across {} files — it has broken, and the \
         guard above is now vacuous",
        found.len(),
        sources().len()
    );
}

/// An allow-list entry for a file with no literals left is stale: it hides the
/// fact that nothing is being checked there any more.
#[test]
fn the_allow_list_has_no_stale_entries() {
    let found = literals();
    let stale: Vec<&str> = FALLBACK_ONLY
        .iter()
        .chain(ALLOWED.iter())
        .map(|(f, _)| *f)
        .filter(|f| !found.iter().any(|(file, _, _)| file == f))
        .collect();
    assert!(
        stale.is_empty(),
        "these files are allow-listed but no longer contain any colour literal, so \
         the entry is protecting nothing: {stale:?}"
    );
}

#[test]
fn every_allow_list_entry_explains_itself() {
    for (file, why) in FALLBACK_ONLY.iter().chain(ALLOWED.iter()) {
        assert!(
            why.len() > 20,
            "{file} is allow-listed without a real reason"
        );
    }
}

/// The narrow matcher is the whole basis of the guard: too loose and every
/// `Rgb(r, g, b)` conversion trips it, too tight and it sees nothing.
#[test]
fn the_scrape_reaches_past_a_test_only_item() {
    // The failure that made this guard vacuous once: a `#[cfg(test)]` item in
    // the middle of a file truncating the scrape before the drawing code.
    let src =
        "#[cfg(test)]\npub fn probe() {}\nlet c = Rgb(1, 2, 3);\n#[cfg(test)]\nmod tests {}\n";
    let body = &src[..test_module_start(src)];
    assert!(
        body.contains("Rgb(1, 2, 3)"),
        "the scrape stopped at the wrong #[cfg(test)]"
    );
    assert!(
        !body.contains("mod tests"),
        "the scrape did not stop at the test module"
    );
}

#[test]
fn the_matcher_tells_a_literal_from_a_forwarded_value() {
    assert!(
        has_rgb_literal("let x = Color::Rgb(30, 30, 46);"),
        "a plain literal"
    );
    assert!(has_rgb_literal("Rgb(1,2,3)"), "no spaces");
    assert!(
        !has_rgb_literal("Color::Rgb(r, g, b) => ..."),
        "forwarding a parsed value"
    );
    assert!(
        !has_rgb_literal("Color::Rgb(rgb.r, rgb.g, rgb.b)"),
        "forwarding fields"
    );
    assert!(!has_rgb_literal("let x = 5;"), "no colour at all");
}
