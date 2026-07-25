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
    pub selection: Rgb,
    pub cursor: Rgb,
    pub divider: Rgb,
    pub accent: Rgb,
}

impl Default for ThemeColors {
    fn default() -> Self {
        ThemeColors {
            bg: Rgb::new(30, 30, 30),
            fg: Rgb::new(205, 214, 244),
            gutter: Rgb::new(108, 112, 134),
            selection: Rgb::new(88, 91, 112),
            cursor: Rgb::new(245, 224, 220),
            divider: Rgb::new(69, 71, 90),
            accent: Rgb::new(243, 139, 168),
        }
    }
}

impl ThemeColors {
    /// Serialize as a theme file: a Lua chunk returning a `{ bg = "#…", … }` table.
    pub fn to_lua(&self) -> String {
        format!(
            "-- ruster theme. Edit the hex colors, or copy this file to make your own.\n\
             return {{\n  \
             bg = {:?},\n  fg = {:?},\n  gutter = {:?},\n  selection = {:?},\n  \
             cursor = {:?},\n  divider = {:?},\n  accent = {:?},\n}}\n",
            self.bg.to_hex(),
            self.fg.to_hex(),
            self.gutter.to_hex(),
            self.selection.to_hex(),
            self.cursor.to_hex(),
            self.divider.to_hex(),
            self.accent.to_hex(),
        )
    }
}

/// Built-in themes written to `themes/` on first run and selectable via
/// `general.theme`.
pub fn builtin_themes() -> Vec<(&'static str, ThemeColors)> {
    vec![
        ("default", ThemeColors::default()),
        (
            "gruvbox",
            ThemeColors {
                bg: Rgb::new(40, 40, 40),
                fg: Rgb::new(235, 219, 178),
                gutter: Rgb::new(124, 111, 100),
                selection: Rgb::new(80, 73, 69),
                cursor: Rgb::new(254, 128, 25),
                divider: Rgb::new(60, 56, 54),
                accent: Rgb::new(250, 189, 47),
            },
        ),
        (
            "tokyonight",
            ThemeColors {
                bg: Rgb::new(26, 27, 38),
                fg: Rgb::new(192, 202, 245),
                gutter: Rgb::new(86, 95, 137),
                selection: Rgb::new(40, 52, 87),
                cursor: Rgb::new(192, 202, 245),
                divider: Rgb::new(65, 72, 104),
                accent: Rgb::new(122, 162, 247),
            },
        ),
        (
            "nord",
            ThemeColors {
                bg: Rgb::new(46, 52, 64),
                fg: Rgb::new(216, 222, 233),
                gutter: Rgb::new(76, 86, 106),
                selection: Rgb::new(67, 76, 94),
                cursor: Rgb::new(136, 192, 208),
                divider: Rgb::new(59, 66, 82),
                accent: Rgb::new(136, 192, 208),
            },
        ),
        (
            "catppuccin-mocha",
            ThemeColors {
                bg: Rgb::new(30, 30, 46),      // base   #1e1e2e
                fg: Rgb::new(205, 214, 244),   // text   #cdd6f4
                gutter: Rgb::new(108, 112, 134), // overlay0 #6c7086
                selection: Rgb::new(88, 91, 112), // surface2 #585b70
                cursor: Rgb::new(245, 224, 220), // rosewater #f5e0dc
                divider: Rgb::new(49, 50, 68),  // surface0 #313244
                accent: Rgb::new(203, 166, 247), // mauve  #cba6f7
            },
        ),
    ]
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
        }
    }
}
