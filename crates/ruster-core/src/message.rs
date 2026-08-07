/// Severity level for a message log entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageLevel {
    Info,
    Success,
    Warning,
    Error,
}

impl MessageLevel {
    pub fn label(&self) -> &'static str {
        match self {
            MessageLevel::Info => "INFO",
            MessageLevel::Success => " OK ",
            MessageLevel::Warning => "WARN",
            MessageLevel::Error => "ERR ",
        }
    }
}

/// Source of a message log entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageSource {
    Build,
    Test,
    Task,
    Lsp,
    Echo,
    System,
}

impl MessageSource {
    pub fn label(&self) -> &'static str {
        match self {
            MessageSource::Build => "build",
            MessageSource::Test => "test",
            MessageSource::Task => "task",
            MessageSource::Lsp => "lsp",
            MessageSource::Echo => "echo",
            MessageSource::System => "system",
        }
    }
}

/// A single entry in the message log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageEntry {
    pub level: MessageLevel,
    pub source: MessageSource,
    pub text: String,
}

/// A time-ordered log of editor/plugin messages.
///
/// Entries are deduplicated: if the same (level, source, text) is pushed
/// consecutively, the duplicate is silently dropped.
#[derive(Debug, Clone)]
pub struct MessageLog {
    pub entries: Vec<MessageEntry>,
    max_entries: usize,
}

impl MessageLog {
    pub fn new() -> Self {
        MessageLog {
            entries: Vec::new(),
            max_entries: 1000,
        }
    }

    /// Push a new entry. Consecutive duplicates are silently dropped.
    pub fn push(&mut self, level: MessageLevel, source: MessageSource, text: String) {
        if let Some(last) = self.entries.last() {
            if last.level == level && last.source == source && last.text == text {
                return;
            }
        }
        if self.entries.len() >= self.max_entries {
            self.entries.remove(0);
        }
        self.entries.push(MessageEntry {
            level,
            source,
            text,
        });
    }

    /// Clear all entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Return entries matching the given filters. `None` means "all".
    pub fn filtered(
        &self,
        source: Option<MessageSource>,
        level: Option<MessageLevel>,
    ) -> Vec<&MessageEntry> {
        self.entries
            .iter()
            .filter(|e| source.is_none_or(|s| e.source == s))
            .filter(|e| level.is_none_or(|l| e.level == l))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for MessageLog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_appends_entry() {
        let mut log = MessageLog::new();
        log.push(MessageLevel::Info, MessageSource::System, "hello".into());
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn consecutive_duplicates_are_deduplicated() {
        let mut log = MessageLog::new();
        log.push(MessageLevel::Info, MessageSource::System, "dup".into());
        log.push(MessageLevel::Info, MessageSource::System, "dup".into());
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn non_consecutive_duplicates_are_both_kept() {
        let mut log = MessageLog::new();
        log.push(MessageLevel::Info, MessageSource::System, "a".into());
        log.push(MessageLevel::Info, MessageSource::System, "b".into());
        log.push(MessageLevel::Info, MessageSource::System, "a".into());
        assert_eq!(log.len(), 3);
    }

    #[test]
    fn clear_removes_all() {
        let mut log = MessageLog::new();
        log.push(MessageLevel::Error, MessageSource::Build, "fail".into());
        log.clear();
        assert!(log.is_empty());
    }

    #[test]
    fn filtered_by_source() {
        let mut log = MessageLog::new();
        log.push(MessageLevel::Info, MessageSource::Build, "built".into());
        log.push(MessageLevel::Info, MessageSource::Lsp, "lsp".into());
        let f = log.filtered(Some(MessageSource::Build), None);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].text, "built");
    }

    #[test]
    fn filtered_by_level() {
        let mut log = MessageLog::new();
        log.push(MessageLevel::Info, MessageSource::System, "info".into());
        log.push(MessageLevel::Error, MessageSource::System, "err".into());
        let f = log.filtered(None, Some(MessageLevel::Error));
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].text, "err");
    }
}
