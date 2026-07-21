pub struct Config {
    pub tabstop: u32,
    pub softtabstop: u32,
    pub expandtab: bool,
    pub number: bool,
    pub relativenumber: bool,
    pub theme: String,
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
        }
    }
}
