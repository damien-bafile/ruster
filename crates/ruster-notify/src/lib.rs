pub mod backend;
pub mod notification;

pub use backend::{BackendConfig, BackendKind};
pub use notification::Notification;

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
        assert!(kinds.contains(&BackendKind::CmdlinePopup));
        assert!(kinds.contains(&BackendKind::Popup));
        assert!(kinds.contains(&BackendKind::Confirm));
    }
}
