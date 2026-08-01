pub mod backend;
pub mod manager;
pub mod notification;

pub use backend::BackendKind;
pub use manager::{NoiceSettings, NotificationManager};
pub use notification::{Notification, Timeout};

#[cfg(test)]
mod tests {
    use super::*;
    use ruster_core::message::{MessageLevel, MessageSource};

    #[test]
    fn test_notification_creation() {
        let n = Notification::new(MessageLevel::Info, MessageSource::Echo, "hello");
        assert_eq!(n.level, MessageLevel::Info);
        assert_eq!(n.source, MessageSource::Echo);
        assert_eq!(n.text, "hello");
        assert!(n.title.is_none());
    }

    #[test]
    fn test_backend_kind_all_contains_all() {
        let kinds = BackendKind::all();
        assert!(kinds.contains(&BackendKind::Mini));
        assert!(kinds.contains(&BackendKind::Notify));
        assert!(kinds.contains(&BackendKind::Split));
    }
}
