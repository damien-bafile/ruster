/// An RGB color, parsed from a `#RRGGBB` config value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Rgb { r, g, b }
    }
    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

/// The configurable color palette (GUI). Defaults mirror the previously
/// hardcoded raylib constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeColors {
    pub bg: Rgb,
    pub fg: Rgb,
    pub gutter: Rgb,
    /// Gutter background. Defaults to `bg` when a theme doesn't set it.
    pub gutter_bg: Rgb,
    pub selection: Rgb,
    /// Text drawn over the selection highlight. Defaults to `fg`.
    pub selection_fg: Rgb,
    pub cursor: Rgb,
    /// Glyph under the block cursor. Defaults to `bg` (a solid block).
    pub cursor_fg: Rgb,
    pub divider: Rgb,
    /// Statusline / bar text. Defaults to `fg`.
    pub statusline_fg: Rgb,
    pub accent: Rgb,
    /// Text drawn on accent-colored bars. Defaults to `bg`.
    pub accent_fg: Rgb,
}

impl Default for ThemeColors {
    fn default() -> Self {
        ThemeColors {
            bg: Rgb::new(30, 30, 30),
            fg: Rgb::new(205, 214, 244),
            gutter: Rgb::new(108, 112, 134),
            gutter_bg: Rgb::new(30, 30, 30),
            selection: Rgb::new(88, 91, 112),
            selection_fg: Rgb::new(205, 214, 244),
            cursor: Rgb::new(245, 224, 220),
            cursor_fg: Rgb::new(30, 30, 30),
            divider: Rgb::new(69, 71, 90),
            statusline_fg: Rgb::new(205, 214, 244),
            accent: Rgb::new(243, 139, 168),
            accent_fg: Rgb::new(30, 30, 30),
        }
    }
}

/// A theme: an ordered named palette plus the 8 UI role colors (the defaults
/// applied to the editor). The Settings page lets the user assign any palette
/// color to each UI element.
#[derive(Debug, Clone)]
pub struct Theme {
    pub palette: Vec<(String, Rgb)>,
    pub roles: ThemeColors,
}

impl Theme {
    /// Serialize as a theme file: a Lua chunk returning the roles + palette.
    pub fn to_lua(&self) -> String {
        let r = &self.roles;
        let mut s = String::from(
            "-- ruster theme. `roles` colour the UI; `palette` are the named colours\n\
             -- the Settings page assigns to each element. Edit or copy freely.\n\
             return {\n",
        );
        s.push_str(&format!(
            "  bg = {:?}, fg = {:?}, gutter = {:?}, gutter_bg = {:?},\n  \
             selection = {:?}, selection_fg = {:?}, cursor = {:?}, cursor_fg = {:?},\n  \
             divider = {:?}, statusline_fg = {:?}, accent = {:?}, accent_fg = {:?},\n",
            r.bg.to_hex(), r.fg.to_hex(), r.gutter.to_hex(), r.gutter_bg.to_hex(),
            r.selection.to_hex(), r.selection_fg.to_hex(), r.cursor.to_hex(), r.cursor_fg.to_hex(),
            r.divider.to_hex(), r.statusline_fg.to_hex(), r.accent.to_hex(), r.accent_fg.to_hex(),
        ));
        s.push_str("  palette = {\n");
        for (name, c) in &self.palette {
            s.push_str(&format!("    {} = {:?},\n", name, c.to_hex()));
        }
        s.push_str("  },\n}\n");
        s
    }
}

fn hex_to_rgb(hex: &str) -> Rgb {
    if hex.len() == 7 && hex.as_bytes()[0] == b'#' {
        if let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&hex[1..3], 16),
            u8::from_str_radix(&hex[3..5], 16),
            u8::from_str_radix(&hex[5..7], 16),
        ) {
            return Rgb::new(r, g, b);
        }
    }
    Rgb::new(0, 0, 0)
}

/// Build a named palette from `(name, "#hex")` entries.
pub fn palette(entries: &[(&str, &str)]) -> Vec<(String, Rgb)> {
    entries.iter().map(|(n, h)| (n.to_string(), hex_to_rgb(h))).collect()
}

/// The Catppuccin Mocha palette (also used as the `default` theme's palette).
const MOCHA: &[(&str, &str)] = &[
    ("rosewater", "#f5e0dc"), ("flamingo", "#f2cdcd"), ("pink", "#f5c2e7"), ("mauve", "#cba6f7"),
    ("red", "#f38ba8"), ("maroon", "#eba0ac"), ("peach", "#fab387"), ("yellow", "#f9e2af"),
    ("green", "#a6e3a1"), ("teal", "#94e2d5"), ("sky", "#89dceb"), ("sapphire", "#74c7ec"),
    ("blue", "#89b4fa"), ("lavender", "#b4befe"), ("text", "#cdd6f4"), ("subtext1", "#bac2de"),
    ("subtext0", "#a6adc8"), ("overlay2", "#9399b2"), ("overlay1", "#7f849c"), ("overlay0", "#6c7086"),
    ("surface2", "#585b70"), ("surface1", "#45475a"), ("surface0", "#313244"), ("base", "#1e1e2e"),
    ("mantle", "#181825"), ("crust", "#11111b"),
];

/// Built-in themes written to `themes/` on first run and selectable via
/// `general.theme`.
pub fn builtin_themes() -> Vec<(&'static str, Theme)> {
    vec![
        (
            "starship",
            Theme {
                palette: palette(&[
                    ("crt_black", "#0a0e0a"), ("phosphor_green", "#33ff66"),
                    ("phosphor_dim", "#1a6633"), ("phosphor_bright", "#66ff99"),
                    ("amber", "#ff8800"), ("amber_dim", "#664400"),
                    ("panel_offwhite", "#ccbbaa"), ("panel_gray", "#222a22"),
                    ("stencil", "#88aa88"), ("hazard_red", "#ff3333"),
                ]),
                roles: ThemeColors {
                    bg: Rgb::new(10, 14, 10), fg: Rgb::new(51, 255, 102),
                    gutter: Rgb::new(26, 102, 51), gutter_bg: Rgb::new(10, 14, 10),
                    selection: Rgb::new(13, 51, 26), selection_fg: Rgb::new(51, 255, 102),
                    cursor: Rgb::new(102, 255, 153), cursor_fg: Rgb::new(10, 14, 10),
                    divider: Rgb::new(17, 26, 17), statusline_fg: Rgb::new(51, 255, 102),
                    accent: Rgb::new(255, 136, 0), accent_fg: Rgb::new(10, 14, 10),
                },
            },
        ),
        (
            "default",
            Theme { palette: palette(MOCHA), roles: ThemeColors::default() },
        ),
        (
            "gruvbox",
            Theme {
                palette: palette(&[
                    ("bg", "#282828"), ("bg1", "#3c3836"), ("bg2", "#504945"), ("bg3", "#665c54"),
                    ("bg4", "#7c6f64"), ("fg", "#ebdbb2"), ("fg2", "#d5c4a1"), ("gray", "#928374"),
                    ("red", "#fb4934"), ("green", "#b8bb26"), ("yellow", "#fabd2f"), ("blue", "#83a598"),
                    ("purple", "#d3869b"), ("aqua", "#8ec07c"), ("orange", "#fe8019"),
                ]),
                roles: ThemeColors {
                    bg: Rgb::new(40, 40, 40), fg: Rgb::new(235, 219, 178),
                    gutter: Rgb::new(124, 111, 100), gutter_bg: Rgb::new(40, 40, 40),
                    selection: Rgb::new(80, 73, 69), selection_fg: Rgb::new(235, 219, 178),
                    cursor: Rgb::new(254, 128, 25), cursor_fg: Rgb::new(40, 40, 40),
                    divider: Rgb::new(60, 56, 54), statusline_fg: Rgb::new(235, 219, 178),
                    accent: Rgb::new(250, 189, 47), accent_fg: Rgb::new(40, 40, 40),
                },
            },
        ),
        (
            "tokyonight",
            Theme {
                palette: palette(&[
                    ("bg", "#1a1b26"), ("bg_dark", "#16161e"), ("bg_highlight", "#292e42"),
                    ("terminal_black", "#414868"), ("fg", "#c0caf5"), ("fg_dark", "#a9b1d6"),
                    ("comment", "#565f89"), ("blue", "#7aa2f7"), ("cyan", "#7dcfff"), ("blue1", "#2ac3de"),
                    ("green", "#9ece6a"), ("teal", "#1abc9c"), ("red", "#f7768e"), ("orange", "#ff9e64"),
                    ("yellow", "#e0af68"), ("magenta", "#bb9af7"), ("purple", "#9d7cd8"),
                ]),
                roles: ThemeColors {
                    bg: Rgb::new(26, 27, 38), fg: Rgb::new(192, 202, 245),
                    gutter: Rgb::new(86, 95, 137), gutter_bg: Rgb::new(26, 27, 38),
                    selection: Rgb::new(40, 52, 87), selection_fg: Rgb::new(192, 202, 245),
                    cursor: Rgb::new(192, 202, 245), cursor_fg: Rgb::new(26, 27, 38),
                    divider: Rgb::new(65, 72, 104), statusline_fg: Rgb::new(192, 202, 245),
                    accent: Rgb::new(122, 162, 247), accent_fg: Rgb::new(26, 27, 38),
                },
            },
        ),
        (
            "nord",
            Theme {
                palette: palette(&[
                    ("nord0", "#2e3440"), ("nord1", "#3b4252"), ("nord2", "#434c5e"), ("nord3", "#4c566a"),
                    ("nord4", "#d8dee9"), ("nord5", "#e5e9f0"), ("nord6", "#eceff4"), ("nord7", "#8fbcbb"),
                    ("nord8", "#88c0d0"), ("nord9", "#81a1c1"), ("nord10", "#5e81ac"), ("nord11", "#bf616a"),
                    ("nord12", "#d08770"), ("nord13", "#ebcb8b"), ("nord14", "#a3be8c"), ("nord15", "#b48ead"),
                ]),
                roles: ThemeColors {
                    bg: Rgb::new(46, 52, 64), fg: Rgb::new(216, 222, 233),
                    gutter: Rgb::new(76, 86, 106), gutter_bg: Rgb::new(46, 52, 64),
                    selection: Rgb::new(67, 76, 94), selection_fg: Rgb::new(216, 222, 233),
                    cursor: Rgb::new(136, 192, 208), cursor_fg: Rgb::new(46, 52, 64),
                    divider: Rgb::new(59, 66, 82), statusline_fg: Rgb::new(216, 222, 233),
                    accent: Rgb::new(136, 192, 208), accent_fg: Rgb::new(46, 52, 64),
                },
            },
        ),
        (
            "catppuccin-mocha",
            Theme {
                palette: palette(MOCHA),
                roles: ThemeColors {
                    bg: Rgb::new(30, 30, 46), fg: Rgb::new(205, 214, 244),
                    gutter: Rgb::new(108, 112, 134), gutter_bg: Rgb::new(30, 30, 46),
                    selection: Rgb::new(88, 91, 112), selection_fg: Rgb::new(205, 214, 244),
                    cursor: Rgb::new(245, 224, 220), cursor_fg: Rgb::new(30, 30, 46),
                    divider: Rgb::new(49, 50, 68), statusline_fg: Rgb::new(205, 214, 244),
                    accent: Rgb::new(203, 166, 247), accent_fg: Rgb::new(30, 30, 46),
                },
            },
        ),
    ]
}

/// Per-element color overrides (hex, or empty = use the theme's color). Applied
/// on top of the selected theme. Lets the user recolor individual UI elements by
/// cycling the theme palette in the Settings page.
#[derive(Debug, Clone, Default)]
pub struct ColorOverrides {
    pub bg: String,
    pub fg: String,
    pub gutter: String,
    pub gutter_bg: String,
    pub selection: String,
    pub selection_fg: String,
    pub cursor: String,
    pub cursor_fg: String,
    pub divider: String,
    pub statusline_fg: String,
    pub accent: String,
    pub accent_fg: String,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub tabstop: u32,
    pub softtabstop: u32,
    pub expandtab: bool,
    pub shiftwidth: u32,
    /// Editing paradigm at startup: "neovim" (modal) or "emacs" (modeless).
    pub editmode: String,
    /// Honor project `.editorconfig` files.
    pub editorconfig: bool,
    /// Default line ending for new files: "lf" or "crlf".
    pub line_ending: String,
    pub number: bool,
    pub relativenumber: bool,
    pub theme: String,
    /// Font for the GUI: an absolute path, or a filename looked up in the user
    /// font dir. `None` tries a list of common Nerd/mono fonts. A Nerd Font is
    /// needed for icon glyphs (otherwise they render as `?`).
    pub gui_font: Option<String>,
    pub font_size: u32,
    pub line_height: u32,
    pub padding_x: u32,
    pub padding_y: u32,
    pub window_width: u32,
    pub window_height: u32,
    pub target_fps: u32,
    /// GUI cursor shape: "block" or "bar".
    pub cursor_kind: String,
    pub cursor_anim_enabled: bool,
    pub cursor_anim_speed: f32,
    pub colors: ThemeColors,
    /// Per-element color overrides layered over `colors` (the theme palette).
    pub color_overrides: ColorOverrides,
    /// Milliseconds to wait for a mapped key sequence before showing which-key.
    pub timeoutlen: u32,
    pub whichkey_enabled: bool,
    /// Format the buffer via LSP before writing it on `:w`.
    pub format_on_save: bool,
    pub lsp_diagnostics: bool,
    pub lsp_hover: bool,
    pub lsp_autostart: bool,
    /// Shell program for `:term` (`None` = platform default: `$SHELL`/`/bin/sh`
    /// on Unix, `%COMSPEC%`/`cmd.exe` on Windows).
    pub terminal_shell: Option<String>,
    /// Lines of scrollback history an embedded terminal retains.
    pub terminal_scrollback: u32,
    /// Initial mode for a new terminal: "insert" or "normal".
    pub terminal_default_mode: String,
    /// Show dotfiles in dired by default.
    pub dired_show_hidden: bool,
    /// Per-language syntax color overrides: `lang key -> (group -> hex)`. Carried
    /// separately from the flat schema (edited via the Settings syntax editor).
    pub syntax_overrides: std::collections::HashMap<String, std::collections::HashMap<String, String>>,
}

/// The `(group, key)` address of a setting.
pub type Addr = (&'static str, &'static str);

impl Config {
    /// The current value of every schema setting, for the Settings page and for
    /// writing `config.lua`. Colors are excluded (they live in themes/).
    pub fn to_settings(&self) -> Vec<(Addr, crate::schema::SettingValue)> {
        use crate::schema::SettingValue::*;
        vec![
            (("general", "tabstop"), Int(self.tabstop as i64)),
            (("general", "softtabstop"), Int(self.softtabstop as i64)),
            (("general", "expandtab"), Bool(self.expandtab)),
            (("general", "shiftwidth"), Int(self.shiftwidth as i64)),
            (("general", "editmode"), Enum(self.editmode.clone())),
            (("general", "editorconfig"), Bool(self.editorconfig)),
            (("general", "line_ending"), Enum(self.line_ending.clone())),
            (("general", "theme"), Text(self.theme.clone())),
            (("gui", "font"), Text(self.gui_font.clone().unwrap_or_default())),
            (("gui", "font_size"), Int(self.font_size as i64)),
            (("gui", "line_height"), Int(self.line_height as i64)),
            (("gui", "padding_x"), Int(self.padding_x as i64)),
            (("gui", "padding_y"), Int(self.padding_y as i64)),
            (("gui", "window_width"), Int(self.window_width as i64)),
            (("gui", "window_height"), Int(self.window_height as i64)),
            (("gui", "target_fps"), Int(self.target_fps as i64)),
            (("gui", "cursor_kind"), Enum(self.cursor_kind.clone())),
            (("gui", "cursor_anim"), Bool(self.cursor_anim_enabled)),
            (("gui", "cursor_anim_speed"), Float(self.cursor_anim_speed as f64)),
            (("gutter", "number"), Bool(self.number)),
            (("gutter", "relativenumber"), Bool(self.relativenumber)),
            (("whichkey", "enabled"), Bool(self.whichkey_enabled)),
            (("whichkey", "timeoutlen"), Int(self.timeoutlen as i64)),
            (("lsp", "format_on_save"), Bool(self.format_on_save)),
            (("lsp", "diagnostics"), Bool(self.lsp_diagnostics)),
            (("lsp", "hover"), Bool(self.lsp_hover)),
            (("lsp", "autostart"), Bool(self.lsp_autostart)),
            (("terminal", "shell"), Text(self.terminal_shell.clone().unwrap_or_default())),
            (("terminal", "scrollback"), Int(self.terminal_scrollback as i64)),
            (("terminal", "default_mode"), Enum(self.terminal_default_mode.clone())),
            (("dired", "show_hidden"), Bool(self.dired_show_hidden)),
            (("colors", "bg"), Text(self.color_overrides.bg.clone())),
            (("colors", "fg"), Text(self.color_overrides.fg.clone())),
            (("colors", "gutter"), Text(self.color_overrides.gutter.clone())),
            (("colors", "gutter_bg"), Text(self.color_overrides.gutter_bg.clone())),
            (("colors", "selection"), Text(self.color_overrides.selection.clone())),
            (("colors", "selection_fg"), Text(self.color_overrides.selection_fg.clone())),
            (("colors", "cursor"), Text(self.color_overrides.cursor.clone())),
            (("colors", "cursor_fg"), Text(self.color_overrides.cursor_fg.clone())),
            (("colors", "divider"), Text(self.color_overrides.divider.clone())),
            (("colors", "statusline_fg"), Text(self.color_overrides.statusline_fg.clone())),
            (("colors", "accent"), Text(self.color_overrides.accent.clone())),
            (("colors", "accent_fg"), Text(self.color_overrides.accent_fg.clone())),
        ]
    }

    /// Build a `Config` from settings values (defaults for anything absent).
    /// Colors are left at their default and applied separately from the theme.
    pub fn from_settings(vals: &[(Addr, crate::schema::SettingValue)]) -> Config {
        use crate::schema::SettingValue as V;
        let find = |g: &str, k: &str| vals.iter().find(|((a, b), _)| *a == g && *b == k).map(|(_, v)| v);
        let d = Config::default();
        let bl = |g, k, dv: bool| match find(g, k) {
            Some(V::Bool(b)) => *b,
            _ => dv,
        };
        let u = |g, k, dv: u32| match find(g, k) {
            Some(V::Int(i)) => *i as u32,
            _ => dv,
        };
        let fl = |g, k, dv: f32| match find(g, k) {
            Some(V::Float(f)) => *f as f32,
            _ => dv,
        };
        let st = |g, k| match find(g, k) {
            Some(V::Text(s)) | Some(V::Enum(s)) | Some(V::Color(s)) => Some(s.clone()),
            _ => None,
        };
        let ostr = |g, k| st(g, k).filter(|s| !s.is_empty());
        Config {
            tabstop: u("general", "tabstop", d.tabstop),
            softtabstop: u("general", "softtabstop", d.softtabstop),
            expandtab: bl("general", "expandtab", d.expandtab),
            shiftwidth: u("general", "shiftwidth", d.shiftwidth),
            editmode: st("general", "editmode").unwrap_or(d.editmode),
            editorconfig: bl("general", "editorconfig", d.editorconfig),
            line_ending: st("general", "line_ending").unwrap_or(d.line_ending),
            number: bl("gutter", "number", d.number),
            relativenumber: bl("gutter", "relativenumber", d.relativenumber),
            theme: st("general", "theme").unwrap_or(d.theme),
            gui_font: ostr("gui", "font"),
            font_size: u("gui", "font_size", d.font_size),
            line_height: u("gui", "line_height", d.line_height),
            padding_x: u("gui", "padding_x", d.padding_x),
            padding_y: u("gui", "padding_y", d.padding_y),
            window_width: u("gui", "window_width", d.window_width),
            window_height: u("gui", "window_height", d.window_height),
            target_fps: u("gui", "target_fps", d.target_fps),
            cursor_kind: st("gui", "cursor_kind").unwrap_or(d.cursor_kind),
            cursor_anim_enabled: bl("gui", "cursor_anim", d.cursor_anim_enabled),
            cursor_anim_speed: fl("gui", "cursor_anim_speed", d.cursor_anim_speed),
            colors: d.colors,
            timeoutlen: u("whichkey", "timeoutlen", d.timeoutlen),
            whichkey_enabled: bl("whichkey", "enabled", d.whichkey_enabled),
            format_on_save: bl("lsp", "format_on_save", d.format_on_save),
            lsp_diagnostics: bl("lsp", "diagnostics", d.lsp_diagnostics),
            lsp_hover: bl("lsp", "hover", d.lsp_hover),
            lsp_autostart: bl("lsp", "autostart", d.lsp_autostart),
            terminal_shell: ostr("terminal", "shell"),
            terminal_scrollback: u("terminal", "scrollback", d.terminal_scrollback),
            terminal_default_mode: st("terminal", "default_mode").unwrap_or(d.terminal_default_mode),
            dired_show_hidden: bl("dired", "show_hidden", d.dired_show_hidden),
            // Not part of the flat schema; carried separately and merged by the
            // caller (runtime parse / Settings save).
            syntax_overrides: std::collections::HashMap::new(),
            color_overrides: ColorOverrides {
                bg: st("colors", "bg").unwrap_or_default(),
                fg: st("colors", "fg").unwrap_or_default(),
                gutter: st("colors", "gutter").unwrap_or_default(),
                gutter_bg: st("colors", "gutter_bg").unwrap_or_default(),
                selection: st("colors", "selection").unwrap_or_default(),
                selection_fg: st("colors", "selection_fg").unwrap_or_default(),
                cursor: st("colors", "cursor").unwrap_or_default(),
                cursor_fg: st("colors", "cursor_fg").unwrap_or_default(),
                divider: st("colors", "divider").unwrap_or_default(),
                statusline_fg: st("colors", "statusline_fg").unwrap_or_default(),
                accent: st("colors", "accent").unwrap_or_default(),
                accent_fg: st("colors", "accent_fg").unwrap_or_default(),
            },
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            tabstop: 4,
            softtabstop: 4,
            expandtab: true,
            shiftwidth: 4,
            editmode: "neovim".into(),
            editorconfig: true,
            line_ending: "lf".into(),
            number: false,
            relativenumber: false,
            theme: "default".into(),
            gui_font: None,
            font_size: 20,
            line_height: 24,
            padding_x: 8,
            padding_y: 4,
            window_width: 800,
            window_height: 600,
            target_fps: 60,
            cursor_kind: "block".into(),
            cursor_anim_enabled: true,
            cursor_anim_speed: 12.0,
            colors: ThemeColors::default(),
            color_overrides: ColorOverrides::default(),
            timeoutlen: 300,
            whichkey_enabled: true,
            format_on_save: false,
            lsp_diagnostics: true,
            lsp_hover: true,
            lsp_autostart: true,
            terminal_shell: None,
            terminal_scrollback: 10000,
            terminal_default_mode: "insert".into(),
            dired_show_hidden: false,
            syntax_overrides: std::collections::HashMap::new(),
        }
    }
}

/// Serialize the per-language syntax overrides as a `ruster.config.syntax` Lua
/// table (sorted for stable output; only non-empty languages emitted). Appended
/// to the generated `config.lua` beside the grouped tables.
pub fn syntax_to_lua(
    map: &std::collections::HashMap<String, std::collections::HashMap<String, String>>,
) -> String {
    let mut langs: Vec<_> = map.iter().filter(|(_, groups)| !groups.is_empty()).collect();
    if langs.is_empty() {
        return String::new();
    }
    langs.sort_by(|a, b| a.0.cmp(b.0));
    let mut s = String::from("\n-- Per-language syntax highlight colours (Settings ▸ Syntax).\nruster.config.syntax = {\n");
    for (lang, groups) in langs {
        let mut items: Vec<_> = groups.iter().filter(|(_, hex)| !hex.is_empty()).collect();
        items.sort_by(|a, b| a.0.cmp(b.0));
        if items.is_empty() {
            continue;
        }
        s.push_str(&format!("  {lang} = {{ "));
        for (group, hex) in items {
            s.push_str(&format!("{group} = {hex:?}, "));
        }
        s.push_str("},\n");
    }
    s.push_str("}\n");
    s
}
