//! The settings schema — a single declarative registry of every configurable
//! option, used to (1) generate the default `config.lua`, (2) validate a loaded
//! config, and (3) drive the in-app Settings page. Adding an option here makes
//! it appear in all three places. This module is pure data + string generation
//! (no `mlua`), so it is trivially testable.

use std::fmt;

/// The type of a setting, which determines how it is validated and which
/// control the Settings page renders for it.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingKind {
    Bool,
    Int { min: i64, max: i64 },
    Float { min: f64, max: f64 },
    Text,
    /// One of a fixed set of string values (rendered as a radio/combobox).
    Enum(&'static [&'static str]),
    /// A `#RRGGBB` hex color.
    Color,
}

/// A concrete setting value.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    Enum(String),
    Color(String),
}

impl SettingValue {
    /// Render as the Lua literal used in the generated `config.lua`.
    pub fn to_lua(&self) -> String {
        match self {
            SettingValue::Bool(b) => b.to_string(),
            SettingValue::Int(i) => i.to_string(),
            SettingValue::Float(f) => {
                // Always include a decimal point so it stays a Lua number.
                if f.fract() == 0.0 {
                    format!("{f:.1}")
                } else {
                    f.to_string()
                }
            }
            SettingValue::Text(s) | SettingValue::Enum(s) | SettingValue::Color(s) => {
                format!("{:?}", s) // quoted + escaped
            }
        }
    }

    /// The value as shown in the Settings page control.
    pub fn display(&self) -> String {
        match self {
            SettingValue::Bool(b) => if *b { "on" } else { "off" }.to_string(),
            SettingValue::Int(i) => i.to_string(),
            SettingValue::Float(f) => format!("{f}"),
            SettingValue::Text(s) | SettingValue::Enum(s) | SettingValue::Color(s) => s.clone(),
        }
    }
}

/// One configurable option.
#[derive(Debug, Clone)]
pub struct SettingSpec {
    pub group: &'static str,
    pub key: &'static str,
    pub label: &'static str,
    pub kind: SettingKind,
    pub default: SettingValue,
    pub help: &'static str,
}

/// A validation failure for one setting.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigError {
    pub group: String,
    pub key: String,
    /// What was expected (kind/range/options).
    pub expected: String,
    /// The offending value, as seen.
    pub got: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}.{}: expected {}, got {} → using default",
            self.group, self.key, self.expected, self.got
        )
    }
}

impl SettingKind {
    /// A human-readable description of what this kind accepts (for errors/help).
    pub fn expected(&self) -> String {
        match self {
            SettingKind::Bool => "true or false".to_string(),
            SettingKind::Int { min, max } => format!("an integer {min}..{max}"),
            SettingKind::Float { min, max } => format!("a number {min}..{max}"),
            SettingKind::Text => "a string".to_string(),
            SettingKind::Enum(opts) => format!("one of {}", opts.join(", ")),
            SettingKind::Color => "a #RRGGBB color".to_string(),
        }
    }

    /// Check a value against this kind. `Ok` = valid; `Err(msg)` describes why
    /// not. A `SettingValue` variant that doesn't match the kind also fails.
    pub fn check(&self, value: &SettingValue) -> Result<(), String> {
        match (self, value) {
            (SettingKind::Bool, SettingValue::Bool(_)) => Ok(()),
            (SettingKind::Int { min, max }, SettingValue::Int(i)) => {
                if i >= min && i <= max {
                    Ok(())
                } else {
                    Err(format!("{i} is out of range {min}..{max}"))
                }
            }
            (SettingKind::Float { min, max }, SettingValue::Float(f)) => {
                if f >= min && f <= max {
                    Ok(())
                } else {
                    Err(format!("{f} is out of range {min}..{max}"))
                }
            }
            (SettingKind::Text, SettingValue::Text(_)) => Ok(()),
            (SettingKind::Enum(opts), SettingValue::Enum(s)) => {
                if opts.contains(&s.as_str()) {
                    Ok(())
                } else {
                    Err(format!("{s:?} is not one of {}", opts.join(", ")))
                }
            }
            (SettingKind::Color, SettingValue::Color(s)) => {
                if is_hex_color(s) {
                    Ok(())
                } else {
                    Err(format!("{s:?} is not a #RRGGBB color"))
                }
            }
            _ => Err(format!("expected {}", self.expected())),
        }
    }
}

/// Whether `s` is a `#RRGGBB` hex color.
pub fn is_hex_color(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 7 && bytes[0] == b'#' && bytes[1..].iter().all(|b| b.is_ascii_hexdigit())
}

/// Parse a `#RRGGBB` color into RGB bytes, if valid.
pub fn parse_hex_color(s: &str) -> Option<(u8, u8, u8)> {
    if !is_hex_color(s) {
        return None;
    }
    let r = u8::from_str_radix(&s[1..3], 16).ok()?;
    let g = u8::from_str_radix(&s[3..5], 16).ok()?;
    let b = u8::from_str_radix(&s[5..7], 16).ok()?;
    Some((r, g, b))
}

// Small constructors to keep the catalog readable.
fn b(v: bool) -> SettingValue {
    SettingValue::Bool(v)
}
fn i(v: i64) -> SettingValue {
    SettingValue::Int(v)
}
fn f(v: f64) -> SettingValue {
    SettingValue::Float(v)
}
fn t(v: &str) -> SettingValue {
    SettingValue::Text(v.to_string())
}
fn e(v: &str) -> SettingValue {
    SettingValue::Enum(v.to_string())
}

/// The groups, in display order.
pub const GROUPS: &[(&str, &str)] = &[
    ("general", "Editing, indentation, and paradigm"),
    ("gui", "GUI font, size, colors, and window"),
    ("gutter", "Line-number gutter"),
    ("whichkey", "Which-key hint panel"),
    ("lsp", "Language server features"),
    ("terminal", "Embedded terminal"),
    ("dired", "File explorer"),
    ("colors", "Per-element color overrides (empty = theme)"),
];

/// The full option catalog — the single source of truth.
pub fn schema() -> Vec<SettingSpec> {
    use SettingKind::*;
    let mut s = Vec::new();
    let mut add = |group, key, label, kind, default, help| {
        s.push(SettingSpec { group, key, label, kind, default, help });
    };

    // --- general ---
    add("general", "tabstop", "Tab width", Int { min: 1, max: 16 }, i(4), "Spaces a tab represents");
    add("general", "softtabstop", "Soft tab stop", Int { min: 0, max: 16 }, i(4), "Spaces inserted on Tab");
    add("general", "expandtab", "Expand tabs", Bool, b(true), "Insert spaces instead of tabs");
    add("general", "shiftwidth", "Shift width", Int { min: 1, max: 16 }, i(4), "Spaces per indent step");
    add("general", "editmode", "Editing paradigm", Enum(&["neovim", "emacs"]), e("neovim"), "Modal (neovim) or modeless (emacs)");
    add("general", "editorconfig", "Honor .editorconfig", Bool, b(true), "Apply project .editorconfig files");
    add("general", "line_ending", "Default line ending", Enum(&["lf", "crlf"]), e("lf"), "Line ending for new files");
    add("general", "theme", "Theme name", Text, t("default"), "Named color theme");

    // --- gui ---
    add("gui", "font", "Font", Text, t(""), "Font file/path; empty = auto-detect a Nerd font");
    add("gui", "font_size", "Font size", Int { min: 8, max: 48 }, i(20), "GUI glyph size in px");
    add("gui", "line_height", "Line height", Int { min: 10, max: 64 }, i(24), "Row height in px");
    add("gui", "padding_x", "Horizontal padding", Int { min: 0, max: 64 }, i(8), "Left padding in px");
    add("gui", "padding_y", "Vertical padding", Int { min: 0, max: 64 }, i(4), "Top padding in px");
    add("gui", "window_width", "Window width", Int { min: 320, max: 7680 }, i(800), "Initial window width");
    add("gui", "window_height", "Window height", Int { min: 240, max: 4320 }, i(600), "Initial window height");
    add("gui", "target_fps", "Target FPS", Int { min: 30, max: 240 }, i(60), "Render loop frame cap");
    add("gui", "cursor_kind", "Cursor shape", Enum(&["block", "bar"]), e("block"), "Block or bar cursor");
    add("gui", "cursor_anim", "Smooth cursor", Bool, b(true), "Animate cursor movement");
    add("gui", "cursor_anim_speed", "Cursor speed", Float { min: 1.0, max: 60.0 }, f(12.0), "Smooth-cursor easing speed");
    // Colors are theme-driven — see general.theme + the themes/ directory.

    // --- gutter ---
    add("gutter", "number", "Line numbers", Bool, b(false), "Show absolute line numbers");
    add("gutter", "relativenumber", "Relative numbers", Bool, b(false), "Show relative line numbers");

    // --- whichkey ---
    add("whichkey", "enabled", "Enabled", Bool, b(true), "Show the which-key hint panel");
    add("whichkey", "timeoutlen", "Timeout (ms)", Int { min: 0, max: 5000 }, i(300), "Delay before the panel appears");

    // --- lsp ---
    add("lsp", "format_on_save", "Format on save", Bool, b(false), "Run LSP formatting on :w");
    add("lsp", "diagnostics", "Diagnostics", Bool, b(true), "Show LSP diagnostics");
    add("lsp", "hover", "Hover", Bool, b(true), "Enable hover popups");
    add("lsp", "autostart", "Auto-start servers", Bool, b(true), "Launch a server when a file opens");

    // --- terminal ---
    add("terminal", "shell", "Shell", Text, t(""), "Program for :term; empty = platform default");
    add("terminal", "scrollback", "Scrollback", Int { min: 0, max: 1_000_000 }, i(10000), "Lines of history retained");
    add("terminal", "default_mode", "Start mode", Enum(&["insert", "normal"]), e("insert"), "Initial mode for a new terminal");

    // --- dired ---
    add("dired", "show_hidden", "Show hidden files", Bool, b(false), "Show dotfiles in the file explorer");

    // --- colors (overrides; empty = use the theme's color) ---
    add("colors", "bg", "Background", Text, t(""), "Override editor background");
    add("colors", "fg", "Foreground", Text, t(""), "Override default text color");
    add("colors", "gutter", "Gutter", Text, t(""), "Override line-number color");
    add("colors", "gutter_bg", "Gutter background", Text, t(""), "Override the gutter background");
    add("colors", "selection", "Selection", Text, t(""), "Override selection highlight");
    add("colors", "cursor", "Cursor", Text, t(""), "Override cursor color");
    add("colors", "divider", "Bars / divider", Text, t(""), "Override statusline bar + window divider");
    add("colors", "accent", "Accent", Text, t(""), "Override accent (titles, prompts)");

    s
}

/// Look up a spec by group + key.
pub fn spec_for(group: &str, key: &str) -> Option<SettingSpec> {
    schema().into_iter().find(|s| s.group == group && s.key == key)
}

/// Generate the default `config.lua` text from the schema.
pub fn generate_default_config() -> String {
    generate_config(&default_values())
}

/// The default value for every spec, keyed by `(group, key)`.
pub fn default_values() -> Vec<((&'static str, &'static str), SettingValue)> {
    schema().into_iter().map(|s| ((s.group, s.key), s.default)).collect()
}

/// Render a `config.lua` from a set of values, grouped and commented. Values are
/// looked up by `(group, key)`; any missing one uses the schema default.
pub fn generate_config(values: &[((&'static str, &'static str), SettingValue)]) -> String {
    let get = |group: &str, key: &str| -> SettingValue {
        values
            .iter()
            .find(|((g, k), _)| *g == group && *k == key)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| spec_for(group, key).map(|s| s.default).unwrap())
    };

    let mut out = String::new();
    out.push_str("-- ruster config — managed by the Settings page (:settings, save with :w).\n");
    out.push_str("-- Safe to hand-edit; comments and layout are regenerated on save.\n");
    out.push_str("-- Advanced scripting (keymaps, plugins) goes in init.lua, loaded after this.\n\n");

    let all = schema();
    for (group, group_help) in GROUPS {
        out.push_str(&format!("-- {group_help}\n"));
        out.push_str(&format!("ruster.config.{group} = {{\n"));
        for spec in all.iter().filter(|s| s.group == *group) {
            let val = get(group, spec.key);
            out.push_str(&format!("  {} = {},", spec.key, val.to_lua()));
            out.push_str(&format!("  -- {}\n", spec.help));
        }
        out.push_str("}\n\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_passes_its_own_kind() {
        for spec in schema() {
            assert!(
                spec.kind.check(&spec.default).is_ok(),
                "{}.{} default {:?} fails its kind {:?}",
                spec.group,
                spec.key,
                spec.default,
                spec.kind
            );
        }
    }

    #[test]
    fn kind_check_catches_bad_values() {
        let int = SettingKind::Int { min: 1, max: 10 };
        assert!(int.check(&SettingValue::Int(5)).is_ok());
        assert!(int.check(&SettingValue::Int(0)).is_err()); // below range
        assert!(int.check(&SettingValue::Int(11)).is_err()); // above range
        assert!(int.check(&SettingValue::Bool(true)).is_err()); // wrong type

        let en = SettingKind::Enum(&["a", "b"]);
        assert!(en.check(&SettingValue::Enum("a".into())).is_ok());
        assert!(en.check(&SettingValue::Enum("c".into())).is_err());

        assert!(SettingKind::Color.check(&SettingValue::Color("#aabbcc".into())).is_ok());
        assert!(SettingKind::Color.check(&SettingValue::Color("blue".into())).is_err());
    }

    #[test]
    fn hex_color_parsing() {
        assert_eq!(parse_hex_color("#1e1e1e"), Some((30, 30, 30)));
        assert_eq!(parse_hex_color("#FFFFFF"), Some((255, 255, 255)));
        assert_eq!(parse_hex_color("#fff"), None);
        assert_eq!(parse_hex_color("1e1e1e"), None);
        assert_eq!(parse_hex_color("#gggggg"), None);
    }

    #[test]
    fn generated_config_has_every_group_and_key() {
        let lua = generate_default_config();
        for (group, _) in GROUPS {
            assert!(lua.contains(&format!("ruster.config.{group} = {{")), "missing group {group}");
        }
        for spec in schema() {
            assert!(lua.contains(&format!("{} =", spec.key)), "missing key {}", spec.key);
        }
    }

    #[test]
    fn float_lua_literal_keeps_decimal() {
        assert_eq!(SettingValue::Float(12.0).to_lua(), "12.0");
        assert_eq!(SettingValue::Bool(true).to_lua(), "true");
        assert_eq!(SettingValue::Text("hi".into()).to_lua(), "\"hi\"");
    }
}
