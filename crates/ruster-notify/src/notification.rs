use ruster_core::message::{MessageLevel, MessageSource};
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone)]
pub struct Notification {
    pub id: u64,
    pub level: MessageLevel,
    pub source: MessageSource,
    pub title: Option<String>,
    pub text: String,
    pub created_at: SystemTime,
    pub timeout: Option<Duration>,
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
            timeout: None,
            dismissed: false,
        }
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn with_persistent(mut self) -> Self {
        self.timeout = Some(Duration::from_secs(0));
        self
    }
}
