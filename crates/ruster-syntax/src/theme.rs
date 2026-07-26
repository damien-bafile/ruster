use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use ruster_render::{Color, SyntaxStyle};

fn rgb(r: u8, g: u8, b: u8) -> Color { Color::Rgb(r, g, b) }

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

/// Base (dotless) group for a capture/markup name, e.g. `function.method` →
/// `function` — the key used for overrides and shown in the Settings editor.
pub fn base_group(name: &str) -> &str {
    name.split('.').next().unwrap_or(name)
}

/// The syntax groups a language exposes in the Settings editor, in display order.
pub fn groups_for_lang(key: &str) -> &'static [&'static str] {
    const CODE: &[&str] = &[
        "keyword", "string", "comment", "function", "type", "variable", "constant",
        "number", "operator", "builtin",
    ];
    const MARKUP: &[&str] = &[
        "heading", "strong", "emphasis", "code", "link", "url", "marker", "quote",
        "keyword", "block", "todo", "done",
    ];
    match key {
        "markdown" | "org" => MARKUP,
        _ => CODE,
    }
}

/// The built-in default style for a base code group (no overrides applied).
pub fn default_code_style(group: &str) -> SyntaxStyle {
    match group {
        "keyword"   => SyntaxStyle { fg: rgb(203, 166, 247), bg: Color::Default, bold: true,  italic: false },
        "string"    => SyntaxStyle { fg: rgb(166, 227, 161), bg: Color::Default, bold: false, italic: false },
        "comment"   => SyntaxStyle { fg: rgb(108, 112, 134), bg: Color::Default, bold: false, italic: true  },
        "function"  => SyntaxStyle { fg: rgb(137, 180, 250), bg: Color::Default, bold: false, italic: false },
        "type"      => SyntaxStyle { fg: rgb(249, 226, 175), bg: Color::Default, bold: false, italic: false },
        "variable"  => SyntaxStyle { fg: rgb(205, 214, 244), bg: Color::Default, bold: false, italic: false },
        "constant"  => SyntaxStyle { fg: rgb(250, 179, 135), bg: Color::Default, bold: false, italic: false },
        "number"    => SyntaxStyle { fg: rgb(250, 179, 135), bg: Color::Default, bold: false, italic: false },
        "operator"  => SyntaxStyle { fg: rgb(137, 220, 235), bg: Color::Default, bold: false, italic: false },
        "builtin"   => SyntaxStyle { fg: rgb(243, 139, 168), bg: Color::Default, bold: false, italic: false },
        _           => SyntaxStyle::default(),
    }
}

/// The default fg for a group in `lang`, used to seed the Settings editor's
/// swatch/value before any override is set.
pub fn default_fg_for(lang: &str, group: &str) -> Color {
    if matches!(lang, "markdown" | "org") {
        default_markup_style(group).fg
    } else {
        default_code_style(group).fg
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
        "heading"  => SyntaxStyle { fg: rgb(137, 180, 250), bg: Color::Default, bold: true,  italic: false },
        "strong"   => SyntaxStyle { fg: rgb(250, 179, 135), bg: Color::Default, bold: true,  italic: false },
        "emphasis" => SyntaxStyle { fg: rgb(203, 166, 247), bg: Color::Default, bold: false, italic: true  },
        "code"     => SyntaxStyle { fg: rgb(166, 227, 161), bg: Color::Default, bold: false, italic: false },
        "link"     => SyntaxStyle { fg: rgb(137, 220, 235), bg: Color::Default, bold: false, italic: false },
        "url"      => SyntaxStyle { fg: rgb(108, 112, 134), bg: Color::Default, bold: false, italic: true  },
        "marker"   => SyntaxStyle { fg: rgb(243, 139, 168), bg: Color::Default, bold: false, italic: false },
        "quote"    => SyntaxStyle { fg: rgb(108, 112, 134), bg: Color::Default, bold: false, italic: true  },
        "keyword"  => SyntaxStyle { fg: rgb(203, 166, 247), bg: Color::Default, bold: true,  italic: false },
        "block"    => SyntaxStyle { fg: rgb(166, 227, 161), bg: Color::Default, bold: false, italic: false },
        "todo"     => SyntaxStyle { fg: rgb(243, 139, 168), bg: Color::Default, bold: true,  italic: false },
        "done"     => SyntaxStyle { fg: rgb(166, 227, 161), bg: Color::Default, bold: true,  italic: false },
        _          => SyntaxStyle::default(),
    }
}

pub const RAINBOW_PALETTE: [Color; 6] = [
    Color::Rgb(243, 139, 168),  // red
    Color::Rgb(250, 179, 135),  // peach
    Color::Rgb(249, 226, 175),  // yellow
    Color::Rgb(166, 227, 161),  // green
    Color::Rgb(137, 190, 180),  // teal
    Color::Rgb(137, 180, 250),  // blue
];
