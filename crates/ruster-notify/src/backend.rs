#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BackendKind {
    Mini,
    Notify,
    Split,
    CmdlinePopup,
    Popup,
    Confirm,
}

impl BackendKind {
    pub fn all() -> &'static [BackendKind] {
        &[
            BackendKind::Mini,
            BackendKind::Notify,
            BackendKind::Split,
            BackendKind::CmdlinePopup,
            BackendKind::Popup,
            BackendKind::Confirm,
        ]
    }
}

#[derive(Debug, Clone)]
pub struct BackendConfig {
    pub kind: BackendKind,
    pub enabled: bool,
    pub default_timeout: Option<std::time::Duration>,
}

impl BackendConfig {
    pub fn new(kind: BackendKind) -> Self {
        Self { kind, enabled: true, default_timeout: None }
    }
}
