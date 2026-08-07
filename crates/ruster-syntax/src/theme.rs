use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use ruster_render::{Color, SyntaxStyle};

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

/// Per-language syntax color overrides: `lang key -> (group -> fg)`. Only the
/// foreground is overridable; bold/italic stay at the group's default.
pub type SyntaxOverrides = HashMap<String, HashMap<String, Color>>;

static OVERRIDES: OnceLock<RwLock<SyntaxOverrides>> = OnceLock::new();

fn overrides() -> &'static RwLock<SyntaxOverrides> {
    OVERRIDES.get_or_init(|| RwLock::new(SyntaxOverrides::new()))
}

/// Install the active per-language overrides (from config). Applied by the next
/// highlight pass — call [`SyntaxEngine::recolor`](crate::SyntaxEngine::recolor)
/// on open buffers afterwards to refresh them.
pub fn set_syntax_overrides(map: SyntaxOverrides) {
    if let Ok(mut w) = overrides().write() {
        *w = map;
    }
}

thread_local! {
    /// The language of the highlight pass in progress, so `style_for_capture` /
    /// `markup_style` can resolve per-language overrides without threading the
    /// key through every call site. Set by the highlighter before it runs.
    static CURRENT_LANG: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Record the language whose highlight pass is running (see `CURRENT_LANG`).
pub fn set_current_lang(lang: &str) {
    CURRENT_LANG.with(|c| c.borrow_mut().replace_range(.., lang));
}

/// The override fg for `group` in the current language, if any.
fn override_fg(group: &str) -> Option<Color> {
    let lang = CURRENT_LANG.with(|c| c.borrow().clone());
    if lang.is_empty() {
        return None;
    }
    let ov = overrides().read().ok()?;
    ov.get(&lang)?.get(group).copied()
}

/// The override fg for `group` in `lang`, naming the language rather than
/// relying on whatever `set_current_lang` last set.
///
/// The thread-local exists for the highlight pass, which sets it once and then
/// resolves thousands of captures. Everything else — the pseudo-languages
/// below, drawn a handful of glyphs at a time — is better off saying which
/// language it means, so forgetting the setter cannot silently drop overrides.
fn override_fg_in(lang: &str, group: &str) -> Option<Color> {
    let ov = overrides().read().ok()?;
    ov.get(lang)?.get(group).copied()
}

/// Base (dotless) group for a capture/markup name, e.g. `function.method` →
/// `function` — the key used for overrides and shown in the Settings editor.
pub fn base_group(name: &str) -> &str {
    name.split('.').next().unwrap_or(name)
}

/// The syntax groups a language exposes in the Settings editor, in display order.
pub fn groups_for_lang(key: &str) -> &'static [&'static str] {
    const CODE: &[&str] = &[
        "keyword", "string", "comment", "function", "type", "variable", "constant", "number",
        "operator", "builtin",
    ];
    const MARKUP: &[&str] = &[
        "heading", "strong", "emphasis", "code", "link", "url", "marker", "quote", "keyword",
        "block", "todo", "done",
    ];
    const DIFF: &[&str] = &["added", "removed", "hunk", "header"];
    const SIGNS: &[&str] = &[
        "added",
        "modified",
        "removed",
        "breakpoint",
        "error",
        "warning",
        "info",
        "hint",
        "todo",
    ];
    const DIRED: &[&str] = &["directory", "executable", "symlink"];
    const FLASH: &[&str] = &["label", "pending"];
    match key {
        "markdown" | "org" => MARKUP,
        "diff" => DIFF,
        "signs" => SIGNS,
        "dired" => DIRED,
        "flash" => FLASH,
        _ => CODE,
    }
}

/// The built-in default style for a base code group (no overrides applied).
pub fn default_code_style(group: &str) -> SyntaxStyle {
    match group {
        "keyword" => SyntaxStyle {
            fg: rgb(203, 166, 247),
            bg: Color::Default,
            bold: true,
            italic: false,
        },
        "string" => SyntaxStyle {
            fg: rgb(166, 227, 161),
            bg: Color::Default,
            bold: false,
            italic: false,
        },
        "comment" => SyntaxStyle {
            fg: rgb(108, 112, 134),
            bg: Color::Default,
            bold: false,
            italic: true,
        },
        "function" => SyntaxStyle {
            fg: rgb(137, 180, 250),
            bg: Color::Default,
            bold: false,
            italic: false,
        },
        "type" => SyntaxStyle {
            fg: rgb(249, 226, 175),
            bg: Color::Default,
            bold: false,
            italic: false,
        },
        "variable" => SyntaxStyle {
            fg: rgb(205, 214, 244),
            bg: Color::Default,
            bold: false,
            italic: false,
        },
        "constant" => SyntaxStyle {
            fg: rgb(250, 179, 135),
            bg: Color::Default,
            bold: false,
            italic: false,
        },
        "number" => SyntaxStyle {
            fg: rgb(250, 179, 135),
            bg: Color::Default,
            bold: false,
            italic: false,
        },
        "operator" => SyntaxStyle {
            fg: rgb(137, 220, 235),
            bg: Color::Default,
            bold: false,
            italic: false,
        },
        "builtin" => SyntaxStyle {
            fg: rgb(243, 139, 168),
            bg: Color::Default,
            bold: false,
            italic: false,
        },
        _ => SyntaxStyle::default(),
    }
}

/// The default fg for a group in `lang`, used to seed the Settings editor's
/// swatch/value before any override is set.
pub fn default_fg_for(lang: &str, group: &str) -> Color {
    match lang {
        "markdown" | "org" => default_markup_style(group).fg,
        "diff" => default_diff_style(group).fg,
        "signs" => default_sign_style(group).fg,
        "dired" => default_dired_style(group).fg,
        "flash" => default_flash_style(group).fg,
        _ => default_code_style(group).fg,
    }
}

/// Styles for a unified diff, with the current language's overrides applied.
///
/// `diff` is a pseudo-language: nothing parses it, but routing it through the
/// same per-language override machinery as Markdown means it appears in the
/// Settings syntax editor and honours `ruster.config.syntax.diff.*` for free,
/// instead of needing a second theming system for four colours.
pub fn diff_style(kind: &str) -> SyntaxStyle {
    styled("diff", kind, default_diff_style(kind))
}

/// Apply `lang`'s override for `group` to `base`, if one is set.
fn styled(lang: &str, group: &str, base: SyntaxStyle) -> SyntaxStyle {
    let mut style = base;
    if let Some(fg) = override_fg_in(lang, group) {
        style.fg = fg;
    }
    style
}

/// Colours for the sign column, with `ruster.config.syntax.signs.*` applied.
///
/// One pseudo-language for every glyph in the gutter — git hunks, breakpoints,
/// failing tests, TODO markers — rather than a group per feature. They share a
/// column, so a theme wants to pick them together.
pub fn sign_style(kind: &str) -> SyntaxStyle {
    styled("signs", kind, default_sign_style(kind))
}

/// The built-in default sign style for `kind` (no overrides applied).
pub fn default_sign_style(kind: &str) -> SyntaxStyle {
    match kind {
        "added" => SyntaxStyle {
            fg: rgb(166, 227, 161),
            bg: Color::Default,
            bold: false,
            italic: false,
        },
        "modified" => SyntaxStyle {
            fg: rgb(249, 226, 175),
            bg: Color::Default,
            bold: false,
            italic: false,
        },
        "removed" => SyntaxStyle {
            fg: rgb(243, 139, 168),
            bg: Color::Default,
            bold: false,
            italic: false,
        },
        "breakpoint" => SyntaxStyle {
            fg: rgb(255, 50, 50),
            bg: Color::Default,
            bold: false,
            italic: false,
        },
        // Diagnostic severities 1-4. `error` doubles as the failing-test sign:
        // both mean "this line is broken", and a theme that wanted them apart
        // would be picking two reds.
        "error" => SyntaxStyle {
            fg: rgb(243, 139, 168),
            bg: Color::Default,
            bold: false,
            italic: false,
        },
        "warning" => SyntaxStyle {
            fg: rgb(249, 226, 175),
            bg: Color::Default,
            bold: false,
            italic: false,
        },
        "info" => SyntaxStyle {
            fg: rgb(137, 180, 250),
            bg: Color::Default,
            bold: false,
            italic: false,
        },
        "hint" => SyntaxStyle {
            fg: rgb(148, 226, 213),
            bg: Color::Default,
            bold: false,
            italic: false,
        },
        // Bold: a TODO marker is drawn over the comment colour and has to win.
        "todo" => SyntaxStyle {
            fg: rgb(249, 226, 175),
            bg: Color::Default,
            bold: true,
            italic: false,
        },
        _ => SyntaxStyle::default(),
    }
}

/// Colours for a dired listing, with `ruster.config.syntax.dired.*` applied.
pub fn dired_style(kind: &str) -> SyntaxStyle {
    styled("dired", kind, default_dired_style(kind))
}

/// The built-in default dired style for `kind` (no overrides applied).
pub fn default_dired_style(kind: &str) -> SyntaxStyle {
    match kind {
        "directory" => SyntaxStyle {
            fg: rgb(137, 180, 250),
            bg: Color::Default,
            bold: true,
            italic: false,
        },
        "executable" => SyntaxStyle {
            fg: rgb(166, 227, 161),
            bg: Color::Default,
            bold: false,
            italic: false,
        },
        "symlink" => SyntaxStyle {
            fg: rgb(137, 220, 235),
            bg: Color::Default,
            bold: false,
            italic: false,
        },
        _ => SyntaxStyle::default(),
    }
}

/// Colours for flash-jump labels, with `ruster.config.syntax.flash.*` applied.
///
/// Their own group rather than part of `signs`: they are transient overlays on
/// the text, not gutter glyphs, and a theme will want them loud in a way it
/// never wants a sign column to be.
pub fn flash_style(kind: &str) -> SyntaxStyle {
    styled("flash", kind, default_flash_style(kind))
}

/// The built-in default flash-label style for `kind` (no overrides applied).
pub fn default_flash_style(kind: &str) -> SyntaxStyle {
    match kind {
        // The first key has been typed; this is the remainder still to type.
        "pending" => SyntaxStyle {
            fg: rgb(255, 255, 0),
            bg: Color::Default,
            bold: false,
            italic: false,
        },
        "label" => SyntaxStyle {
            fg: rgb(0, 200, 255),
            bg: Color::Default,
            bold: false,
            italic: false,
        },
        _ => SyntaxStyle::default(),
    }
}

/// The built-in default diff style for `kind` (no overrides applied).
pub fn default_diff_style(kind: &str) -> SyntaxStyle {
    match kind {
        "added" => SyntaxStyle {
            fg: rgb(166, 227, 161),
            bg: Color::Default,
            bold: false,
            italic: false,
        },
        "removed" => SyntaxStyle {
            fg: rgb(243, 139, 168),
            bg: Color::Default,
            bold: false,
            italic: false,
        },
        "hunk" => SyntaxStyle {
            fg: rgb(137, 180, 250),
            bg: Color::Default,
            bold: true,
            italic: false,
        },
        // File headers and `index` lines: present but not the point.
        "header" => SyntaxStyle {
            fg: rgb(108, 112, 134),
            bg: Color::Default,
            bold: false,
            italic: false,
        },
        _ => SyntaxStyle {
            fg: Color::Default,
            bg: Color::Default,
            bold: false,
            italic: false,
        },
    }
}

pub fn style_for_capture(name: &str) -> SyntaxStyle {
    let group = base_group(name);
    let mut style = default_code_style(group);
    if let Some(fg) = override_fg(group) {
        style.fg = fg;
    }
    style
}

/// Styles for the line-based markup highlighter (Markdown / Org), with the
/// current language's overrides applied. Kept beside the tree-sitter theme so
/// both share the same Catppuccin palette.
pub fn markup_style(kind: &str) -> SyntaxStyle {
    let mut style = default_markup_style(kind);
    if let Some(fg) = override_fg(kind) {
        style.fg = fg;
    }
    style
}

/// The built-in default markup style for `kind` (no overrides applied).
pub fn default_markup_style(kind: &str) -> SyntaxStyle {
    match kind {
        "heading" => SyntaxStyle {
            fg: rgb(137, 180, 250),
            bg: Color::Default,
            bold: true,
            italic: false,
        },
        "strong" => SyntaxStyle {
            fg: rgb(250, 179, 135),
            bg: Color::Default,
            bold: true,
            italic: false,
        },
        "emphasis" => SyntaxStyle {
            fg: rgb(203, 166, 247),
            bg: Color::Default,
            bold: false,
            italic: true,
        },
        "code" => SyntaxStyle {
            fg: rgb(166, 227, 161),
            bg: Color::Default,
            bold: false,
            italic: false,
        },
        "link" => SyntaxStyle {
            fg: rgb(137, 220, 235),
            bg: Color::Default,
            bold: false,
            italic: false,
        },
        "url" => SyntaxStyle {
            fg: rgb(108, 112, 134),
            bg: Color::Default,
            bold: false,
            italic: true,
        },
        "marker" => SyntaxStyle {
            fg: rgb(243, 139, 168),
            bg: Color::Default,
            bold: false,
            italic: false,
        },
        "quote" => SyntaxStyle {
            fg: rgb(108, 112, 134),
            bg: Color::Default,
            bold: false,
            italic: true,
        },
        "keyword" => SyntaxStyle {
            fg: rgb(203, 166, 247),
            bg: Color::Default,
            bold: true,
            italic: false,
        },
        "block" => SyntaxStyle {
            fg: rgb(166, 227, 161),
            bg: Color::Default,
            bold: false,
            italic: false,
        },
        "todo" => SyntaxStyle {
            fg: rgb(243, 139, 168),
            bg: Color::Default,
            bold: true,
            italic: false,
        },
        "done" => SyntaxStyle {
            fg: rgb(166, 227, 161),
            bg: Color::Default,
            bold: true,
            italic: false,
        },
        _ => SyntaxStyle::default(),
    }
}

pub const RAINBOW_PALETTE: [Color; 6] = [
    Color::Rgb(243, 139, 168), // red
    Color::Rgb(250, 179, 135), // peach
    Color::Rgb(249, 226, 175), // yellow
    Color::Rgb(166, 227, 161), // green
    Color::Rgb(137, 190, 180), // teal
    Color::Rgb(137, 180, 250), // blue
];
