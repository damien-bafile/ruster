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
