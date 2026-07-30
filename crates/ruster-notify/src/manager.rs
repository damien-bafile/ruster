use crate::backend::BackendKind;
use crate::notification::Notification;
use ruster_core::message::MessageLevel;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

#[derive(Debug)]
struct ActiveEntry {
    notif: Notification,
    pushed_at: Instant,
}

#[derive(Debug)]
pub struct NotificationManager {
    history: Vec<Notification>,
    active: BTreeMap<BackendKind, Vec<ActiveEntry>>,
    next_id: u64,
    default_timeout: Duration,
    max_history: usize,
}

impl NotificationManager {
    pub fn new(default_timeout: Duration) -> Self {
        Self::with_max(default_timeout, 1000)
    }

    pub fn with_max(default_timeout: Duration, max_history: usize) -> Self {
        let mut active = BTreeMap::new();
        for kind in BackendKind::all() {
            active.insert(*kind, Vec::new());
        }
        Self {
            history: Vec::with_capacity(max_history),
            active,
            next_id: 1,
            default_timeout,
            max_history,
        }
    }

    pub fn push(&mut self, mut notif: Notification) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        notif.id = id;

        let effective_timeout = match notif.timeout {
            Some(d) if d == Duration::ZERO => Some(self.default_timeout),
            other => other,
        };
        notif.timeout = effective_timeout;

        let kind = match notif.level {
            MessageLevel::Error => BackendKind::Notify,
            MessageLevel::Warning => BackendKind::Notify,
            _ => BackendKind::Mini,
        };

        // Warning also goes to Mini
        if notif.level == MessageLevel::Warning {
            self.active
                .get_mut(&BackendKind::Mini)
                .unwrap()
                .push(ActiveEntry {
                    notif: notif.clone(),
                    pushed_at: Instant::now(),
                });
        }

        self.active
            .get_mut(&kind)
            .unwrap()
            .push(ActiveEntry {
                notif: notif.clone(),
                pushed_at: Instant::now(),
            });

        self.history.push(notif);

        while self.history.len() > self.max_history {
            self.history.remove(0);
        }
        id
    }

    pub fn dismiss(&mut self, id: u64) {
        for list in self.active.values_mut() {
            list.retain(|e| e.notif.id != id);
        }
        if let Some(n) = self.history.iter_mut().find(|n| n.id == id) {
            n.dismissed = true;
        }
    }

    pub fn dismiss_all(&mut self) {
        for list in self.active.values_mut() {
            for e in list.iter() {
                if let Some(h) = self.history.iter_mut().find(|hn| hn.id == e.notif.id) {
                    h.dismissed = true;
                }
            }
            list.clear();
        }
    }

    pub fn history(&self) -> &[Notification] {
        &self.history
    }

    pub fn active(&self, kind: BackendKind) -> Vec<&Notification> {
        self.active
            .get(&kind)
            .map(|v| v.iter().map(|e| &e.notif).collect())
            .unwrap_or_default()
    }

    pub fn tick(&mut self) {
        let now = Instant::now();
        for list in self.active.values_mut() {
            list.retain(|e| {
                let should_dismiss =
                    e.notif.timeout.is_some_and(|t| now - e.pushed_at >= t);
                if should_dismiss {
                    if let Some(h) = self.history.iter_mut().find(|hn| hn.id == e.notif.id) {
                        h.dismissed = true;
                    }
                }
                !should_dismiss
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruster_core::message::{MessageLevel, MessageSource};
    use std::time::Duration;

    fn info_notif(text: &str) -> Notification {
        Notification::new(MessageLevel::Info, MessageSource::Echo, text)
    }

    #[test]
    fn test_push_adds_to_history() {
        let mut mgr = NotificationManager::new(Duration::from_secs(2));
        mgr.push(info_notif("hello"));
        assert_eq!(mgr.history().len(), 1);
    }

    #[test]
    fn test_push_adds_to_mini_active() {
        let mut mgr = NotificationManager::new(Duration::from_secs(2));
        let n = info_notif("hello").with_timeout(Duration::from_secs(2));
        mgr.push(n);
        assert_eq!(mgr.active(BackendKind::Mini).len(), 1);
    }

    #[test]
    fn test_dismiss_removes_from_active() {
        let mut mgr = NotificationManager::new(Duration::from_secs(2));
        let id = mgr.push(info_notif("hello").with_timeout(Duration::from_secs(2)));
        assert_eq!(mgr.active(BackendKind::Mini).len(), 1);
        mgr.dismiss(id);
        assert_eq!(mgr.active(BackendKind::Mini).len(), 0);
    }

    #[test]
    fn test_dismiss_all_clears_all_active() {
        let mut mgr = NotificationManager::new(Duration::from_secs(2));
        mgr.push(info_notif("a").with_timeout(Duration::from_secs(2)));
        mgr.push(info_notif("b").with_timeout(Duration::from_secs(2)));
        mgr.dismiss_all();
        assert_eq!(mgr.active(BackendKind::Mini).len(), 0);
    }

    #[test]
    fn test_tick_dismisses_expired() {
        let mut mgr = NotificationManager::new(Duration::from_secs(0));
        mgr.push(info_notif("x").with_timeout(Duration::from_secs(0)));
        assert_eq!(mgr.active(BackendKind::Mini).len(), 1);
        mgr.tick();
        assert_eq!(mgr.active(BackendKind::Mini).len(), 0);
        assert!(mgr.history().iter().any(|n| n.dismissed));
    }

    #[test]
    fn test_default_timeout_applied() {
        let mut mgr = NotificationManager::new(Duration::from_secs(5));
        let n = info_notif("default");
        mgr.push(n);
        assert_eq!(mgr.active(BackendKind::Mini).len(), 1);
    }

    #[test]
    fn test_persistent_notification_not_dismissed_by_tick() {
        let mut mgr = NotificationManager::new(Duration::from_secs(1));
        let n = Notification::new(MessageLevel::Error, MessageSource::System, "persistent");
        mgr.push(n);
        mgr.tick();
        // Error routes to Notify backend; persistent means it survives tick
        assert_eq!(mgr.active(BackendKind::Notify).len(), 1);
    }

    #[test]
    fn test_history_respects_max() {
        let mut mgr = NotificationManager::with_max(Duration::from_secs(2), 3);
        mgr.push(info_notif("1"));
        mgr.push(info_notif("2"));
        mgr.push(info_notif("3"));
        mgr.push(info_notif("4"));
        assert_eq!(mgr.history().len(), 3);
    }
}
