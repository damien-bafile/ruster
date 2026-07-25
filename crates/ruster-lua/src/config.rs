pub struct Config {
    pub tabstop: u32,
    pub softtabstop: u32,
    pub expandtab: bool,
    pub shiftwidth: u32,
    pub number: bool,
    pub relativenumber: bool,
    pub theme: String,
    pub cursor_anim_enabled: bool,
    pub cursor_anim_speed: f32,
    /// Milliseconds to wait for a mapped key sequence before showing which-key.
    pub timeoutlen: u32,
    /// Format the buffer via LSP before writing it on `:w`.
    pub format_on_save: bool,
    /// Shell program for `:term` (`None` = platform default: `$SHELL`/`/bin/sh`
    /// on Unix, `%COMSPEC%`/`cmd.exe` on Windows).
    pub terminal_shell: Option<String>,
    /// Lines of scrollback history an embedded terminal retains.
    pub terminal_scrollback: u32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            tabstop: 4,
            softtabstop: 4,
            expandtab: true,
            shiftwidth: 4,
            number: false,
            relativenumber: false,
            theme: "default".into(),
            cursor_anim_enabled: true,
            cursor_anim_speed: 12.0,
            timeoutlen: 300,
            format_on_save: false,
            terminal_shell: None,
            terminal_scrollback: 10000,
        }
    }
}
