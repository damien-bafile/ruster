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
        for k in [
            BackendKind::Mini,
            BackendKind::Notify,
            BackendKind::Split,
            BackendKind::CmdlinePopup,
            BackendKind::Popup,
            BackendKind::Confirm,
        ] {
            assert!(kinds.contains(&k), "{k:?} missing from all()");
        }
    }
}
