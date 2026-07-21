pub struct Config {
    pub tabstop: u32,
    pub softtabstop: u32,
    pub expandtab: bool,
    pub number: bool,
    pub relativenumber: bool,
    pub theme: String,
    pub cursor_anim_enabled: bool,
    pub cursor_anim_speed: f32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            tabstop: 4,
            softtabstop: 4,
            expandtab: true,
            number: false,
            relativenumber: false,
            theme: "default".into(),
            cursor_anim_enabled: true,
            cursor_anim_speed: 12.0,
        }
    }
}
