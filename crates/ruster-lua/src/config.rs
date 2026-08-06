/// An RGB color, parsed from a `#RRGGBB` config value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Bump this when built-in theme defaults change to force regeneration of
/// cached theme files on disk.
pub const CURRENT_THEME_VERSION: u32 = 2;

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
    pub cursor_bg: Rgb,
    /// Selection highlight background.
    pub selection_bg: Rgb,
    /// Text over the selection / cursor highlight.
    pub selection_fg: Rgb,
    /// Glyph under the block cursor. Defaults to `bg` (a solid block).
    pub cursor_fg: Rgb,
    pub divider: Rgb,
    /// Statusline / bar text. Defaults to `fg`.
    pub statusline_fg: Rgb,
    /// Statusline / bar background.
    pub statusline_bg: Rgb,
    pub accent: Rgb,
    /// Text drawn on accent-colored bars. Defaults to `bg`.
    pub accent_fg: Rgb,
    /// Which-key panel background.
    pub whichkey_bg: Rgb,
    /// Which-key panel text.
    pub whichkey_fg: Rgb,
    /// Which-key key-letter accent.
    pub whichkey_key: Rgb,
    /// Cmdline / mini-buffer background.
    pub cmdline_bg: Rgb,
    /// Cmdline / mini-buffer text.
    pub cmdline_fg: Rgb,
    /// Statusline background in Normal mode.
    pub mode_normal_bg: Rgb,
    /// Statusline text in Normal mode.
    pub mode_normal_fg: Rgb,
    /// Statusline background in Insert mode.
    pub mode_insert_bg: Rgb,
    /// Statusline text in Insert mode.
    pub mode_insert_fg: Rgb,
    /// Statusline background in Visual mode.
    pub mode_visual_bg: Rgb,
    /// Statusline text in Visual mode.
    pub mode_visual_fg: Rgb,
    /// Statusline background in Cmdline mode.
    pub mode_cmdline_bg: Rgb,
    /// Statusline text in Cmdline mode.
    pub mode_cmdline_fg: Rgb,
    /// Statusline background in Emacs mode.
    pub mode_emacs_bg: Rgb,
    /// Statusline text in Emacs mode.
    pub mode_emacs_fg: Rgb,
}

impl Default for ThemeColors {
    fn default() -> Self {
        ThemeColors {
            bg: Rgb::new(30, 30, 30),
            fg: Rgb::new(205, 214, 244),
            gutter: Rgb::new(108, 112, 134),
            gutter_bg: Rgb::new(30, 30, 30),
            cursor_bg: Rgb::new(245, 224, 220),
            cursor_fg: Rgb::new(30, 30, 30),
            selection_bg: Rgb::new(88, 91, 112),
            selection_fg: Rgb::new(205, 214, 244),
            divider: Rgb::new(69, 71, 90),
            statusline_fg: Rgb::new(205, 214, 244),
            statusline_bg: Rgb::new(69, 71, 90),
            accent: Rgb::new(243, 139, 168),
            accent_fg: Rgb::new(30, 30, 30),
            whichkey_bg: Rgb::new(30, 30, 46),
            whichkey_fg: Rgb::new(205, 214, 244),
            whichkey_key: Rgb::new(243, 139, 168),
            cmdline_bg: Rgb::new(30, 30, 30),
            cmdline_fg: Rgb::new(205, 214, 244),
            mode_normal_bg: Rgb::new(69, 71, 90),
            mode_normal_fg: Rgb::new(205, 214, 244),
            mode_insert_bg: Rgb::new(40, 72, 50),
            mode_insert_fg: Rgb::new(205, 214, 244),
            mode_visual_bg: Rgb::new(72, 50, 80),
            mode_visual_fg: Rgb::new(205, 214, 244),
            mode_cmdline_bg: Rgb::new(60, 55, 40),
            mode_cmdline_fg: Rgb::new(205, 214, 244),
            mode_emacs_bg: Rgb::new(50, 50, 70),
            mode_emacs_fg: Rgb::new(205, 214, 244),
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
        let mut s = format!(
            "-- ruster theme. `roles` colour the UI; `palette` are the named colours\n\
             -- the Settings page assigns to each element. Edit or copy freely.\n\
             -- ruster-theme-version: {}\n\
             return {{\n",
            CURRENT_THEME_VERSION,
        );
        s.push_str(&format!(
            "  bg = {:?}, fg = {:?}, gutter = {:?}, gutter_bg = {:?},\n  \
             cursor_bg = {:?}, selection_bg = {:?}, selection_fg = {:?}, cursor_fg = {:?},\n  \
             divider = {:?}, statusline_fg = {:?}, statusline_bg = {:?}, accent = {:?}, accent_fg = {:?},\n  \
             whichkey_bg = {:?}, whichkey_fg = {:?}, whichkey_key = {:?}, cmdline_bg = {:?}, cmdline_fg = {:?},\n  \
             mode_normal_bg = {:?}, mode_normal_fg = {:?},\n  \
             mode_insert_bg = {:?}, mode_insert_fg = {:?},\n  \
             mode_visual_bg = {:?}, mode_visual_fg = {:?},\n  \
             mode_cmdline_bg = {:?}, mode_cmdline_fg = {:?},\n  \
             mode_emacs_bg = {:?}, mode_emacs_fg = {:?},\n",
            r.bg.to_hex(), r.fg.to_hex(), r.gutter.to_hex(), r.gutter_bg.to_hex(),
            r.cursor_bg.to_hex(), r.selection_bg.to_hex(), r.selection_fg.to_hex(), r.cursor_fg.to_hex(),
            r.divider.to_hex(), r.statusline_fg.to_hex(), r.statusline_bg.to_hex(), r.accent.to_hex(), r.accent_fg.to_hex(),
            r.whichkey_bg.to_hex(), r.whichkey_fg.to_hex(), r.whichkey_key.to_hex(),
            r.cmdline_bg.to_hex(), r.cmdline_fg.to_hex(),
            r.mode_normal_bg.to_hex(), r.mode_normal_fg.to_hex(),
            r.mode_insert_bg.to_hex(), r.mode_insert_fg.to_hex(),
            r.mode_visual_bg.to_hex(), r.mode_visual_fg.to_hex(),
            r.mode_cmdline_bg.to_hex(), r.mode_cmdline_fg.to_hex(),
            r.mode_emacs_bg.to_hex(), r.mode_emacs_fg.to_hex(),
        ));
        s.push_str("  palette = {\n");
        for (name, c) in &self.palette {
            s.push_str(&format!("    {} = {:?},\n", name, c.to_hex()));
        }
        s.push_str("  },\n}\n");
        s
    }
}

/// Extract the `ruster-theme-version: N` marker from a theme file's content.
/// Returns `None` if the marker is absent (old file without versioning).
pub fn theme_version(content: &str) -> Option<u32> {
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("-- ruster-theme-version: ") {
            if let Ok(v) = rest.trim().parse::<u32>() {
                return Some(v);
            }
        }
    }
    None
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

/// The Catppuccin Mocha UI roles — what ruster looks like out of the box.
///
/// Shared between the `catppuccin-mocha` built-in and [`Config::default`], so
/// the shipped default and the theme it names cannot drift apart.
pub fn mocha_roles() -> ThemeColors {
    ThemeColors {
        bg: Rgb::new(30, 30, 46), fg: Rgb::new(205, 214, 244),
        gutter: Rgb::new(108, 112, 134), gutter_bg: Rgb::new(30, 30, 46),
        cursor_bg: Rgb::new(245, 224, 220), cursor_fg: Rgb::new(30, 30, 46),
        selection_bg: Rgb::new(88, 91, 112), selection_fg: Rgb::new(205, 214, 244),
        divider: Rgb::new(49, 50, 68), statusline_fg: Rgb::new(205, 214, 244),
        statusline_bg: Rgb::new(49, 50, 68),
        accent: Rgb::new(203, 166, 247), accent_fg: Rgb::new(30, 30, 46),
        whichkey_bg: Rgb::new(30, 30, 46), whichkey_fg: Rgb::new(205, 214, 244),
        whichkey_key: Rgb::new(203, 166, 247),
        cmdline_bg: Rgb::new(30, 30, 46), cmdline_fg: Rgb::new(205, 214, 244),
        mode_normal_bg: Rgb::new(49, 50, 68), mode_normal_fg: Rgb::new(205, 214, 244),
        mode_insert_bg: Rgb::new(30, 60, 45), mode_insert_fg: Rgb::new(205, 214, 244),
        mode_visual_bg: Rgb::new(55, 35, 70), mode_visual_fg: Rgb::new(205, 214, 244),
        mode_cmdline_bg: Rgb::new(49, 50, 68), mode_cmdline_fg: Rgb::new(205, 214, 244),
        mode_emacs_bg: Rgb::new(40, 45, 60), mode_emacs_fg: Rgb::new(205, 214, 244),
    }
}

const LATTE: &[(&str, &str)] = &[
    ("rosewater", "#dc8a78"), ("flamingo", "#dd7878"), ("pink", "#ea76cb"), ("mauve", "#8839ef"),
    ("red", "#d20f39"), ("maroon", "#e64553"), ("peach", "#fe640b"), ("yellow", "#df8e1d"),
    ("green", "#40a02b"), ("teal", "#179299"), ("sky", "#04a5e5"), ("sapphire", "#209fb5"),
    ("blue", "#1e66f5"), ("lavender", "#7287fd"), ("text", "#4c4f69"), ("subtext1", "#5c5f77"),
    ("subtext0", "#6c6f85"), ("overlay2", "#7c7f93"), ("overlay1", "#8c8fa1"), ("overlay0", "#9ca0b0"),
    ("surface2", "#acb0be"), ("surface1", "#bcc0cc"), ("surface0", "#ccd0da"), ("base", "#eff1f5"),
    ("mantle", "#e6e9ef"), ("crust", "#dce0e8"),
];

const FRAPPE: &[(&str, &str)] = &[
    ("rosewater", "#f2d5cf"), ("flamingo", "#eebebe"), ("pink", "#f4b8e4"), ("mauve", "#ca9ee6"),
    ("red", "#e78284"), ("maroon", "#ea999c"), ("peach", "#ef9f76"), ("yellow", "#e5c890"),
    ("green", "#a6d189"), ("teal", "#81c8be"), ("sky", "#99d1db"), ("sapphire", "#85c1dc"),
    ("blue", "#8caaee"), ("lavender", "#babbf1"), ("text", "#c6d0f5"), ("subtext1", "#b5bfe2"),
    ("subtext0", "#a5adce"), ("overlay2", "#949cbb"), ("overlay1", "#838ba7"), ("overlay0", "#737994"),
    ("surface2", "#626880"), ("surface1", "#51576d"), ("surface0", "#414559"), ("base", "#303446"),
    ("mantle", "#292c3c"), ("crust", "#232634"),
];

const MACCHIATO: &[(&str, &str)] = &[
    ("rosewater", "#f4dbd6"), ("flamingo", "#f0c6c6"), ("pink", "#f5bde6"), ("mauve", "#c6a0f6"),
    ("red", "#ed8796"), ("maroon", "#ee99a0"), ("peach", "#f5a97f"), ("yellow", "#eed49f"),
    ("green", "#a6da95"), ("teal", "#8bd5ca"), ("sky", "#91d7e3"), ("sapphire", "#7dc4e4"),
    ("blue", "#8aadf4"), ("lavender", "#b7bdf8"), ("text", "#cad3f5"), ("subtext1", "#b8c0e0"),
    ("subtext0", "#a5adcb"), ("overlay2", "#939ab7"), ("overlay1", "#8087a2"), ("overlay0", "#6e738d"),
    ("surface2", "#5b6078"), ("surface1", "#494d64"), ("surface0", "#363a4f"), ("base", "#24273a"),
    ("mantle", "#1e2030"), ("crust", "#181926"),
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
                    cursor_bg: Rgb::new(102, 255, 153), cursor_fg: Rgb::new(10, 14, 10),
                    selection_bg: Rgb::new(88, 91, 112), selection_fg: Rgb::new(51, 255, 102),
                    divider: Rgb::new(17, 26, 17), statusline_fg: Rgb::new(51, 255, 102),
                    statusline_bg: Rgb::new(17, 26, 17),
                    accent: Rgb::new(255, 136, 0), accent_fg: Rgb::new(10, 14, 10),
                    whichkey_bg: Rgb::new(10, 14, 10), whichkey_fg: Rgb::new(51, 255, 102),
                    whichkey_key: Rgb::new(255, 136, 0),
                    cmdline_bg: Rgb::new(10, 14, 10), cmdline_fg: Rgb::new(51, 255, 102),
                    mode_normal_bg: Rgb::new(17, 26, 17), mode_normal_fg: Rgb::new(51, 255, 102),
                    mode_insert_bg: Rgb::new(17, 26, 51), mode_insert_fg: Rgb::new(51, 255, 102),
                    mode_visual_bg: Rgb::new(40, 20, 20), mode_visual_fg: Rgb::new(51, 255, 102),
                    mode_cmdline_bg: Rgb::new(17, 26, 17), mode_cmdline_fg: Rgb::new(51, 255, 102),
                    mode_emacs_bg: Rgb::new(17, 26, 17), mode_emacs_fg: Rgb::new(51, 255, 102),
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
                    cursor_bg: Rgb::new(254, 128, 25), cursor_fg: Rgb::new(40, 40, 40),
                    selection_bg: Rgb::new(88, 91, 112), selection_fg: Rgb::new(235, 219, 178),
                    divider: Rgb::new(60, 56, 54), statusline_fg: Rgb::new(235, 219, 178),
                    statusline_bg: Rgb::new(60, 56, 54),
                    accent: Rgb::new(250, 189, 47), accent_fg: Rgb::new(40, 40, 40),
                    whichkey_bg: Rgb::new(40, 40, 40), whichkey_fg: Rgb::new(235, 219, 178),
                    whichkey_key: Rgb::new(250, 189, 47),
                    cmdline_bg: Rgb::new(40, 40, 40), cmdline_fg: Rgb::new(235, 219, 178),
                    mode_normal_bg: Rgb::new(60, 56, 54), mode_normal_fg: Rgb::new(235, 219, 178),
                    mode_insert_bg: Rgb::new(40, 60, 40), mode_insert_fg: Rgb::new(235, 219, 178),
                    mode_visual_bg: Rgb::new(60, 40, 50), mode_visual_fg: Rgb::new(235, 219, 178),
                    mode_cmdline_bg: Rgb::new(60, 56, 54), mode_cmdline_fg: Rgb::new(235, 219, 178),
                    mode_emacs_bg: Rgb::new(50, 50, 65), mode_emacs_fg: Rgb::new(235, 219, 178),
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
                    cursor_bg: Rgb::new(192, 202, 245), cursor_fg: Rgb::new(26, 27, 38),
                    selection_bg: Rgb::new(88, 91, 112), selection_fg: Rgb::new(192, 202, 245),
                    divider: Rgb::new(65, 72, 104), statusline_fg: Rgb::new(192, 202, 245),
                    statusline_bg: Rgb::new(65, 72, 104),
                    accent: Rgb::new(122, 162, 247), accent_fg: Rgb::new(26, 27, 38),
                    whichkey_bg: Rgb::new(30, 30, 46), whichkey_fg: Rgb::new(192, 202, 245),
                    whichkey_key: Rgb::new(122, 162, 247),
                    cmdline_bg: Rgb::new(26, 27, 38), cmdline_fg: Rgb::new(192, 202, 245),
                    mode_normal_bg: Rgb::new(65, 72, 104), mode_normal_fg: Rgb::new(192, 202, 245),
                    mode_insert_bg: Rgb::new(40, 72, 50), mode_insert_fg: Rgb::new(192, 202, 245),
                    mode_visual_bg: Rgb::new(65, 50, 80), mode_visual_fg: Rgb::new(192, 202, 245),
                    mode_cmdline_bg: Rgb::new(65, 72, 104), mode_cmdline_fg: Rgb::new(192, 202, 245),
                    mode_emacs_bg: Rgb::new(50, 60, 90), mode_emacs_fg: Rgb::new(192, 202, 245),
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
                    cursor_bg: Rgb::new(136, 192, 208), cursor_fg: Rgb::new(46, 52, 64),
                    selection_bg: Rgb::new(88, 91, 112), selection_fg: Rgb::new(216, 222, 233),
                    divider: Rgb::new(59, 66, 82), statusline_fg: Rgb::new(216, 222, 233),
                    statusline_bg: Rgb::new(59, 66, 82),
                    accent: Rgb::new(136, 192, 208), accent_fg: Rgb::new(46, 52, 64),
                    whichkey_bg: Rgb::new(46, 52, 64), whichkey_fg: Rgb::new(216, 222, 233),
                    whichkey_key: Rgb::new(136, 192, 208),
                    cmdline_bg: Rgb::new(46, 52, 64), cmdline_fg: Rgb::new(216, 222, 233),
                    mode_normal_bg: Rgb::new(59, 66, 82), mode_normal_fg: Rgb::new(216, 222, 233),
                    mode_insert_bg: Rgb::new(40, 66, 50), mode_insert_fg: Rgb::new(216, 222, 233),
                    mode_visual_bg: Rgb::new(66, 45, 80), mode_visual_fg: Rgb::new(216, 222, 233),
                    mode_cmdline_bg: Rgb::new(59, 66, 82), mode_cmdline_fg: Rgb::new(216, 222, 233),
                    mode_emacs_bg: Rgb::new(50, 55, 75), mode_emacs_fg: Rgb::new(216, 222, 233),
                },
            },
        ),
        (
            "catppuccin-mocha",
            Theme { palette: palette(MOCHA), roles: mocha_roles() },
        ),
        (
            "catppuccin-latte",
            Theme {
                palette: palette(LATTE),
                roles: ThemeColors {
                    bg: Rgb::new(239, 241, 245), fg: Rgb::new(76, 79, 105),
                    gutter: Rgb::new(156, 160, 176), gutter_bg: Rgb::new(239, 241, 245),
                    cursor_bg: Rgb::new(220, 138, 120), cursor_fg: Rgb::new(239, 241, 245),
                    selection_bg: Rgb::new(172, 176, 190), selection_fg: Rgb::new(76, 79, 105),
                    divider: Rgb::new(204, 208, 218), statusline_fg: Rgb::new(76, 79, 105),
                    statusline_bg: Rgb::new(230, 233, 239),
                    accent: Rgb::new(136, 57, 239), accent_fg: Rgb::new(239, 241, 245),
                    whichkey_bg: Rgb::new(239, 241, 245), whichkey_fg: Rgb::new(76, 79, 105),
                    whichkey_key: Rgb::new(136, 57, 239),
                    cmdline_bg: Rgb::new(239, 241, 245), cmdline_fg: Rgb::new(76, 79, 105),
                    mode_normal_bg: Rgb::new(230, 233, 239), mode_normal_fg: Rgb::new(76, 79, 105),
                    mode_insert_bg: Rgb::new(215, 235, 215), mode_insert_fg: Rgb::new(76, 79, 105),
                    mode_visual_bg: Rgb::new(230, 215, 240), mode_visual_fg: Rgb::new(76, 79, 105),
                    mode_cmdline_bg: Rgb::new(230, 233, 239), mode_cmdline_fg: Rgb::new(76, 79, 105),
                    mode_emacs_bg: Rgb::new(215, 225, 240), mode_emacs_fg: Rgb::new(76, 79, 105),
                },
            },
        ),
        (
            "catppuccin-frappe",
            Theme {
                palette: palette(FRAPPE),
                roles: ThemeColors {
                    bg: Rgb::new(48, 52, 70), fg: Rgb::new(198, 208, 245),
                    gutter: Rgb::new(115, 121, 148), gutter_bg: Rgb::new(48, 52, 70),
                    cursor_bg: Rgb::new(242, 213, 207), cursor_fg: Rgb::new(48, 52, 70),
                    selection_bg: Rgb::new(98, 104, 128), selection_fg: Rgb::new(198, 208, 245),
                    divider: Rgb::new(65, 69, 89), statusline_fg: Rgb::new(198, 208, 245),
                    statusline_bg: Rgb::new(65, 69, 89),
                    accent: Rgb::new(202, 158, 230), accent_fg: Rgb::new(48, 52, 70),
                    whichkey_bg: Rgb::new(48, 52, 70), whichkey_fg: Rgb::new(198, 208, 245),
                    whichkey_key: Rgb::new(202, 158, 230),
                    cmdline_bg: Rgb::new(48, 52, 70), cmdline_fg: Rgb::new(198, 208, 245),
                    mode_normal_bg: Rgb::new(65, 69, 89), mode_normal_fg: Rgb::new(198, 208, 245),
                    mode_insert_bg: Rgb::new(48, 65, 55), mode_insert_fg: Rgb::new(198, 208, 245),
                    mode_visual_bg: Rgb::new(65, 45, 75), mode_visual_fg: Rgb::new(198, 208, 245),
                    mode_cmdline_bg: Rgb::new(65, 69, 89), mode_cmdline_fg: Rgb::new(198, 208, 245),
                    mode_emacs_bg: Rgb::new(55, 60, 75), mode_emacs_fg: Rgb::new(198, 208, 245),
                },
            },
        ),
        (
            "catppuccin-macchiato",
            Theme {
                palette: palette(MACCHIATO),
                roles: ThemeColors {
                    bg: Rgb::new(36, 39, 58), fg: Rgb::new(202, 211, 245),
                    gutter: Rgb::new(110, 115, 141), gutter_bg: Rgb::new(36, 39, 58),
                    cursor_bg: Rgb::new(244, 219, 214), cursor_fg: Rgb::new(36, 39, 58),
                    selection_bg: Rgb::new(91, 96, 120), selection_fg: Rgb::new(202, 211, 245),
                    divider: Rgb::new(54, 58, 79), statusline_fg: Rgb::new(202, 211, 245),
                    statusline_bg: Rgb::new(54, 58, 79),
                    accent: Rgb::new(198, 160, 246), accent_fg: Rgb::new(36, 39, 58),
                    whichkey_bg: Rgb::new(36, 39, 58), whichkey_fg: Rgb::new(202, 211, 245),
                    whichkey_key: Rgb::new(198, 160, 246),
                    cmdline_bg: Rgb::new(36, 39, 58), cmdline_fg: Rgb::new(202, 211, 245),
                    mode_normal_bg: Rgb::new(54, 58, 79), mode_normal_fg: Rgb::new(202, 211, 245),
                    mode_insert_bg: Rgb::new(36, 55, 50), mode_insert_fg: Rgb::new(202, 211, 245),
                    mode_visual_bg: Rgb::new(55, 40, 70), mode_visual_fg: Rgb::new(202, 211, 245),
                    mode_cmdline_bg: Rgb::new(54, 58, 79), mode_cmdline_fg: Rgb::new(202, 211, 245),
                    mode_emacs_bg: Rgb::new(45, 50, 65), mode_emacs_fg: Rgb::new(202, 211, 245),
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
    pub cursor_bg: String,
    pub selection_bg: String,
    pub selection_fg: String,
    pub cursor_fg: String,
    pub divider: String,
    pub statusline_fg: String,
    pub statusline_bg: String,
    pub accent: String,
    pub accent_fg: String,
    pub whichkey_bg: String,
    pub whichkey_fg: String,
    pub whichkey_key: String,
    pub cmdline_bg: String,
    pub cmdline_fg: String,
    pub mode_normal_bg: String,
    pub mode_normal_fg: String,
    pub mode_insert_bg: String,
    pub mode_insert_fg: String,
    pub mode_visual_bg: String,
    pub mode_visual_fg: String,
    pub mode_cmdline_bg: String,
    pub mode_cmdline_fg: String,
    pub mode_emacs_bg: String,
    pub mode_emacs_fg: String,
}

#[derive(Debug, Clone)]
pub struct NoiceConfig {
    pub mini_enabled: bool,
    pub notify_enabled: bool,
    pub split_enabled: bool,
    pub info_timeout_ms: u64,
    pub success_timeout_ms: u64,
    pub warning_timeout_ms: u64,
    pub max_history: usize,
}

impl Default for NoiceConfig {
    fn default() -> Self {
        Self {
            mini_enabled: true,
            notify_enabled: true,
            split_enabled: true,
            info_timeout_ms: 2000,
            success_timeout_ms: 2000,
            warning_timeout_ms: 5000,
            max_history: 1000,
        }
    }
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
    /// Where the `:`-Tab command palette appears: "center" or "bottom".
    pub command_palette: String,
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
    /// `git.signs` — show added/changed/removed markers in the gutter.
    pub git_signs: bool,
    /// `todo.keywords` — markers highlighted in comments and listed by `:TodoList`.
    pub todo_keywords: Vec<String>,
    /// `:build` command override; `None` = detect from the project type.
    /// A project's `ruster.toml` still takes precedence over this.
    pub build_command: Option<String>,
    /// `:test` command override; `None` = detect from the project type.
    pub test_command: Option<String>,
    /// Debug adapter program; `None` = detect from the file's language.
    pub dap_adapter: Option<String>,
    /// Open the sidebar automatically on startup.
    pub sidebar_auto_open: bool,
    /// Reopen the project's saved session on startup. Off by default:
    /// silently reopening files is surprising when you asked for one.
    pub session_autoload: bool,
    /// Write the session on exit, so `:SessionRestore` has something to read.
    pub session_autosave: bool,
    pub noice: NoiceConfig,
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
            (("whichkey", "command_palette"), Enum(self.command_palette.clone())),
            (("lsp", "format_on_save"), Bool(self.format_on_save)),
            (("lsp", "diagnostics"), Bool(self.lsp_diagnostics)),
            (("lsp", "hover"), Bool(self.lsp_hover)),
            (("lsp", "autostart"), Bool(self.lsp_autostart)),
            (("terminal", "shell"), Text(self.terminal_shell.clone().unwrap_or_default())),
            (("terminal", "scrollback"), Int(self.terminal_scrollback as i64)),
            (("terminal", "default_mode"), Enum(self.terminal_default_mode.clone())),
            (("dired", "show_hidden"), Bool(self.dired_show_hidden)),
            (("sidebar", "auto_open"), Bool(self.sidebar_auto_open)),
            (("session", "autoload"), Bool(self.session_autoload)),
            (("session", "autosave"), Bool(self.session_autosave)),
            (("colors", "bg"), Text(self.color_overrides.bg.clone())),
            (("colors", "fg"), Text(self.color_overrides.fg.clone())),
            (("colors", "gutter"), Text(self.color_overrides.gutter.clone())),
            (("colors", "gutter_bg"), Text(self.color_overrides.gutter_bg.clone())),
            (("colors", "cursor_bg"), Text(self.color_overrides.cursor_bg.clone())),
            (("colors", "selection_bg"), Text(self.color_overrides.selection_bg.clone())),
            (("colors", "selection_fg"), Text(self.color_overrides.selection_fg.clone())),
            (("colors", "cursor_fg"), Text(self.color_overrides.cursor_fg.clone())),
            (("colors", "divider"), Text(self.color_overrides.divider.clone())),
            (("colors", "statusline_fg"), Text(self.color_overrides.statusline_fg.clone())),
            (("colors", "statusline_bg"), Text(self.color_overrides.statusline_bg.clone())),
            (("colors", "accent"), Text(self.color_overrides.accent.clone())),
            (("colors", "accent_fg"), Text(self.color_overrides.accent_fg.clone())),
            (("colors", "whichkey_bg"), Text(self.color_overrides.whichkey_bg.clone())),
            (("colors", "whichkey_fg"), Text(self.color_overrides.whichkey_fg.clone())),
            (("colors", "whichkey_key"), Text(self.color_overrides.whichkey_key.clone())),
            (("colors", "cmdline_bg"), Text(self.color_overrides.cmdline_bg.clone())),
            (("colors", "cmdline_fg"), Text(self.color_overrides.cmdline_fg.clone())),
            (("colors", "mode_normal_bg"), Text(self.color_overrides.mode_normal_bg.clone())),
            (("colors", "mode_normal_fg"), Text(self.color_overrides.mode_normal_fg.clone())),
            (("colors", "mode_insert_bg"), Text(self.color_overrides.mode_insert_bg.clone())),
            (("colors", "mode_insert_fg"), Text(self.color_overrides.mode_insert_fg.clone())),
            (("colors", "mode_visual_bg"), Text(self.color_overrides.mode_visual_bg.clone())),
            (("colors", "mode_visual_fg"), Text(self.color_overrides.mode_visual_fg.clone())),
            (("colors", "mode_cmdline_bg"), Text(self.color_overrides.mode_cmdline_bg.clone())),
            (("colors", "mode_cmdline_fg"), Text(self.color_overrides.mode_cmdline_fg.clone())),
            (("colors", "mode_emacs_bg"), Text(self.color_overrides.mode_emacs_bg.clone())),
            (("colors", "mode_emacs_fg"), Text(self.color_overrides.mode_emacs_fg.clone())),
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
            command_palette: st("whichkey", "command_palette").unwrap_or(d.command_palette),
            format_on_save: bl("lsp", "format_on_save", d.format_on_save),
            lsp_diagnostics: bl("lsp", "diagnostics", d.lsp_diagnostics),
            lsp_hover: bl("lsp", "hover", d.lsp_hover),
            lsp_autostart: bl("lsp", "autostart", d.lsp_autostart),
            terminal_shell: ostr("terminal", "shell"),
            terminal_scrollback: u("terminal", "scrollback", d.terminal_scrollback),
            terminal_default_mode: st("terminal", "default_mode").unwrap_or(d.terminal_default_mode),
            dired_show_hidden: bl("dired", "show_hidden", d.dired_show_hidden),
            git_signs: bl("git", "signs", d.git_signs),
            todo_keywords: st("todo", "keywords")
                .map(|v| split_keywords(&v))
                .unwrap_or(d.todo_keywords),
            build_command: ostr("build", "command"),
            test_command: ostr("test", "command"),
            dap_adapter: ostr("dap", "adapter"),
            sidebar_auto_open: bl("sidebar", "auto_open", d.sidebar_auto_open),
            session_autoload: bl("session", "autoload", d.session_autoload),
            session_autosave: bl("session", "autosave", d.session_autosave),
            // Not part of the flat schema; carried separately and merged by the
            // caller (runtime parse / Settings save).
            syntax_overrides: std::collections::HashMap::new(),
            color_overrides: ColorOverrides {
                bg: st("colors", "bg").unwrap_or_default(),
                fg: st("colors", "fg").unwrap_or_default(),
                gutter: st("colors", "gutter").unwrap_or_default(),
                gutter_bg: st("colors", "gutter_bg").unwrap_or_default(),
                cursor_bg: st("colors", "cursor_bg").unwrap_or_default(),
                selection_bg: st("colors", "selection_bg").unwrap_or_default(),
                selection_fg: st("colors", "selection_fg").unwrap_or_default(),
                cursor_fg: st("colors", "cursor_fg").unwrap_or_default(),
                divider: st("colors", "divider").unwrap_or_default(),
                statusline_fg: st("colors", "statusline_fg").unwrap_or_default(),
                statusline_bg: st("colors", "statusline_bg").unwrap_or_default(),
                accent: st("colors", "accent").unwrap_or_default(),
                accent_fg: st("colors", "accent_fg").unwrap_or_default(),
                whichkey_bg: st("colors", "whichkey_bg").unwrap_or_default(),
                whichkey_fg: st("colors", "whichkey_fg").unwrap_or_default(),
                whichkey_key: st("colors", "whichkey_key").unwrap_or_default(),
                cmdline_bg: st("colors", "cmdline_bg").unwrap_or_default(),
                cmdline_fg: st("colors", "cmdline_fg").unwrap_or_default(),
                mode_normal_bg: st("colors", "mode_normal_bg").unwrap_or_default(),
                mode_normal_fg: st("colors", "mode_normal_fg").unwrap_or_default(),
                mode_insert_bg: st("colors", "mode_insert_bg").unwrap_or_default(),
                mode_insert_fg: st("colors", "mode_insert_fg").unwrap_or_default(),
                mode_visual_bg: st("colors", "mode_visual_bg").unwrap_or_default(),
                mode_visual_fg: st("colors", "mode_visual_fg").unwrap_or_default(),
                mode_cmdline_bg: st("colors", "mode_cmdline_bg").unwrap_or_default(),
                mode_cmdline_fg: st("colors", "mode_cmdline_fg").unwrap_or_default(),
                mode_emacs_bg: st("colors", "mode_emacs_bg").unwrap_or_default(),
                mode_emacs_fg: st("colors", "mode_emacs_fg").unwrap_or_default(),
            },
            noice: NoiceConfig {
                mini_enabled: bl("noice", "mini", d.noice.mini_enabled),
                notify_enabled: bl("noice", "notify", d.noice.notify_enabled),
                split_enabled: bl("noice", "split", d.noice.split_enabled),
                info_timeout_ms: u("noice", "info_timeout", d.noice.info_timeout_ms as u32) as u64,
                success_timeout_ms: u("noice", "success_timeout", d.noice.success_timeout_ms as u32)
                    as u64,
                warning_timeout_ms: u("noice", "warning_timeout", d.noice.warning_timeout_ms as u32)
                    as u64,
                max_history: u("noice", "max_history", d.noice.max_history as u32) as usize,
            },
        }
    }
}

/// Split a comma-separated keyword list, dropping blanks so a trailing comma or
/// an empty setting doesn't produce a marker that matches everywhere.
fn split_keywords(v: &str) -> Vec<String> {
    v.split(',').map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).collect()
}

/// The built-in marker set. Duplicated as strings rather than depending on
/// `ruster-syntax` — `ruster-lua` sits below it in the crate graph.
fn ruster_syntax_default_todo_keywords() -> Vec<String> {
    ["TODO", "FIXME", "HACK", "NOTE", "XXX"].iter().map(|s| s.to_string()).collect()
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
            theme: "catppuccin-mocha".into(),
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
            // Matches `theme` above: the fallback and the named default agree.
            colors: mocha_roles(),
            color_overrides: ColorOverrides::default(),
            timeoutlen: 300,
            whichkey_enabled: true,
            command_palette: "center".to_string(),
            format_on_save: false,
            lsp_diagnostics: true,
            lsp_hover: true,
            lsp_autostart: true,
            terminal_shell: None,
            terminal_scrollback: 10000,
            terminal_default_mode: "insert".into(),
            dired_show_hidden: false,
            git_signs: true,
            todo_keywords: ruster_syntax_default_todo_keywords(),
            build_command: None,
            test_command: None,
            dap_adapter: None,
            sidebar_auto_open: false,
            session_autoload: false,
            session_autosave: true,
            noice: NoiceConfig::default(),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped default, in three places that have to agree: the setting
    /// schema (what `:settings` and a generated `config.lua` show), `Config`
    /// itself, and a built-in theme actually named that.
    #[test]
    fn the_default_theme_is_catppuccin_mocha() {
        assert_eq!(Config::default().theme, "catppuccin-mocha");

        let schema_default = crate::schema::schema()
            .into_iter()
            .find(|s| s.group == "general" && s.key == "theme")
            .map(|s| s.default)
            .expect("general.theme is in the schema");
        assert_eq!(schema_default, crate::schema::SettingValue::Text("catppuccin-mocha".into()));

        assert!(
            builtin_themes().iter().any(|(n, _)| *n == "catppuccin-mocha"),
            "the default has to name a theme that exists"
        );
    }

    /// `resolve_theme_colors` only reaches `Config::default().colors` when no
    /// theme file and no built-in match. Those colours should still be the
    /// ones the default theme would have given, not a different palette.
    #[test]
    fn the_fallback_colours_match_the_default_theme() {
        let (_, mocha) = builtin_themes()
            .into_iter()
            .find(|(n, _)| *n == "catppuccin-mocha")
            .expect("built-in exists");
        assert_eq!(Config::default().colors, mocha.roles);
    }
}
