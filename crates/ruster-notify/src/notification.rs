use ruster_core::message::{MessageLevel, MessageSource};
use std::time::{Duration, SystemTime};

/// How long an active notification stays up.
///
/// [`Default`](Self::Default) is the common case — the manager resolves it from
/// the `noice` per-level timeouts when the notification is pushed, so callers
/// don't have to know them. The other two override that per notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timeout {
    Default,
    After(Duration),
    /// Never auto-dismissed; stays until dismissed explicitly.
    Persistent,
}

#[derive(Debug, Clone)]
pub struct Notification {
    pub id: u64,
    pub level: MessageLevel,
    pub source: MessageSource,
    pub title: Option<String>,
    pub text: String,
    pub created_at: SystemTime,
    pub timeout: Timeout,
    pub dismissed: bool,
}

impl Notification {
    pub fn new(level: MessageLevel, source: MessageSource, text: impl Into<String>) -> Self {
        Self {
            id: 0,
            level,
            source,
            title: None,
            text: text.into(),
            created_at: SystemTime::now(),
            timeout: Timeout::Default,
            dismissed: false,
        }
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Timeout::After(timeout);
        self
    }

    /// Marks the notification as persistent (never auto-dismissed).
    pub fn with_persistent(mut self) -> Self {
        self.timeout = Timeout::Persistent;
        self
    }
}
