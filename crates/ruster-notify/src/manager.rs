use crate::backend::BackendKind;
use crate::notification::{Notification, Timeout};
use ruster_core::message::MessageLevel;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// Runtime settings for the notification pipeline, mirroring the `noice` config
/// group. A plain value type so `ruster-notify` keeps its single dependency on
/// `ruster-core` and stays independent of the Lua config layer.
#[derive(Debug, Clone)]
pub struct NoiceSettings {
    pub mini_enabled: bool,
    pub notify_enabled: bool,
    pub split_enabled: bool,
    pub info_timeout: Duration,
    pub success_timeout: Duration,
    pub warning_timeout: Duration,
    pub max_history: usize,
}

impl Default for NoiceSettings {
    fn default() -> Self {
        Self {
            mini_enabled: true,
            notify_enabled: true,
            split_enabled: true,
            info_timeout: Duration::from_millis(2000),
            success_timeout: Duration::from_millis(2000),
            warning_timeout: Duration::from_millis(5000),
            max_history: 1000,
        }
    }
}

#[derive(Debug)]
struct ActiveEntry {
    notif: Notification,
    pushed_at: Instant,
    /// The notification's [`Timeout`] resolved against [`NoiceSettings`] at push
    /// time. `None` means persistent.
    timeout: Option<Duration>,
}

#[derive(Debug)]
pub struct NotificationManager {
    history: Vec<Notification>,
    active: BTreeMap<BackendKind, Vec<ActiveEntry>>,
    next_id: u64,
    settings: NoiceSettings,
}

impl NotificationManager {
    pub fn new(settings: NoiceSettings) -> Self {
        let mut active = BTreeMap::new();
        for kind in BackendKind::all() {
            active.insert(*kind, Vec::new());
        }
        Self {
            history: Vec::with_capacity(settings.max_history),
            active,
            next_id: 1,
            settings,
        }
    }

    /// Whether `:Noice split` may open the `*noice*` history buffer.
    pub fn split_enabled(&self) -> bool {
        self.settings.split_enabled
    }

    fn backend_enabled(&self, kind: BackendKind) -> bool {
        match kind {
            BackendKind::Mini => self.settings.mini_enabled,
            BackendKind::Notify => self.settings.notify_enabled,
            BackendKind::Split => self.settings.split_enabled,
        }
    }

    /// Resolve an authored [`Timeout`] to a concrete duration. `Default` picks
    /// the configured timeout for the notification's level; errors are
    /// persistent so they can't scroll away before being read.
    fn resolve_timeout(&self, notif: &Notification) -> Option<Duration> {
        match notif.timeout {
            Timeout::After(d) => Some(d),
            Timeout::Persistent => None,
            Timeout::Default => match notif.level {
                MessageLevel::Error => None,
                MessageLevel::Warning => Some(self.settings.warning_timeout),
                MessageLevel::Success => Some(self.settings.success_timeout),
                MessageLevel::Info => Some(self.settings.info_timeout),
            },
        }
    }

    pub fn push(&mut self, mut notif: Notification) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        notif.id = id;

        let timeout = self.resolve_timeout(&notif);

        // Errors and warnings go to the stacking panel; everything else to the
        // mini toast. Warnings additionally mirror into the toast so they're
        // seen without opening the panel.
        let primary = match notif.level {
            MessageLevel::Error | MessageLevel::Warning => BackendKind::Notify,
            _ => BackendKind::Mini,
        };
        let mut targets = vec![primary];
        if notif.level == MessageLevel::Warning {
            targets.push(BackendKind::Mini);
        }

        let now = Instant::now();
        for kind in targets {
            if !self.backend_enabled(kind) {
                continue;
            }
            if let Some(list) = self.active.get_mut(&kind) {
                list.push(ActiveEntry { notif: notif.clone(), pushed_at: now, timeout });
            }
        }

        // History is kept regardless of which backends are enabled — `:messages`
        // and `:Noice split` read it, not the active queues.
        self.history.push(notif);

        while self.history.len() > self.settings.max_history {
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
                let should_dismiss = e.timeout.is_some_and(|t| now - e.pushed_at >= t);
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

    fn mgr() -> NotificationManager {
        NotificationManager::new(NoiceSettings::default())
    }

    /// A manager whose every level times out instantly, so `tick()` expires it.
    fn instant_mgr() -> NotificationManager {
        NotificationManager::new(NoiceSettings {
            info_timeout: Duration::ZERO,
            success_timeout: Duration::ZERO,
            warning_timeout: Duration::ZERO,
            ..Default::default()
        })
    }

    #[test]
    fn test_push_adds_to_history() {
        let mut m = mgr();
        m.push(info_notif("hello"));
        assert_eq!(m.history().len(), 1);
    }

    #[test]
    fn test_push_adds_to_mini_active() {
        let mut m = mgr();
        m.push(info_notif("hello"));
        assert_eq!(m.active(BackendKind::Mini).len(), 1);
    }

    #[test]
    fn test_dismiss_removes_from_active() {
        let mut m = mgr();
        let id = m.push(info_notif("hello"));
        assert_eq!(m.active(BackendKind::Mini).len(), 1);
        m.dismiss(id);
        assert_eq!(m.active(BackendKind::Mini).len(), 0);
    }

    #[test]
    fn test_dismiss_all_clears_all_active() {
        let mut m = mgr();
        m.push(info_notif("a"));
        m.push(info_notif("b"));
        m.dismiss_all();
        assert_eq!(m.active(BackendKind::Mini).len(), 0);
    }

    #[test]
    fn test_tick_dismisses_expired() {
        let mut m = instant_mgr();
        m.push(info_notif("x"));
        assert_eq!(m.active(BackendKind::Mini).len(), 1);
        m.tick();
        assert_eq!(m.active(BackendKind::Mini).len(), 0);
        assert!(m.history().iter().any(|n| n.dismissed));
    }

    /// The default-timeout path is what every caller uses: `Notification::new`
    /// leaves `Timeout::Default`, which must resolve to the level's configured
    /// duration rather than staying up forever.
    #[test]
    fn default_timeout_resolves_per_level() {
        let mut m = NotificationManager::new(NoiceSettings {
            info_timeout: Duration::ZERO,
            warning_timeout: Duration::from_secs(60),
            ..Default::default()
        });
        m.push(info_notif("info"));
        m.push(Notification::new(MessageLevel::Warning, MessageSource::Echo, "warn"));
        m.tick();
        // Info used the zero timeout and expired; the warning's 60s has not.
        assert_eq!(m.active(BackendKind::Mini).len(), 1);
        assert_eq!(m.active(BackendKind::Notify).len(), 1);
    }

    #[test]
    fn explicit_timeout_overrides_the_level_default() {
        let mut m = NotificationManager::new(NoiceSettings {
            info_timeout: Duration::from_secs(60),
            ..Default::default()
        });
        m.push(info_notif("quick").with_timeout(Duration::ZERO));
        m.tick();
        assert_eq!(m.active(BackendKind::Mini).len(), 0);
    }

    #[test]
    fn errors_are_persistent_by_default() {
        let mut m = instant_mgr();
        m.push(Notification::new(MessageLevel::Error, MessageSource::System, "boom"));
        m.tick();
        // Error routes to Notify and has no default timeout, so it survives.
        assert_eq!(m.active(BackendKind::Notify).len(), 1);
    }

    #[test]
    fn explicit_persistent_survives_a_zero_level_timeout() {
        let mut m = instant_mgr();
        m.push(info_notif("sticky").with_persistent());
        m.tick();
        assert_eq!(m.active(BackendKind::Mini).len(), 1);
    }

    #[test]
    fn disabled_backend_drops_the_active_entry_but_keeps_history() {
        let mut m = NotificationManager::new(NoiceSettings {
            mini_enabled: false,
            ..Default::default()
        });
        m.push(info_notif("unseen"));
        assert_eq!(m.active(BackendKind::Mini).len(), 0);
        assert_eq!(m.history().len(), 1, ":messages still records it");
    }

    /// A warning fans out to both backends, so disabling one must not suppress
    /// the other.
    fn warn(text: &str) -> Notification {
        Notification::new(MessageLevel::Warning, MessageSource::Echo, text)
    }

    #[test]
    fn disabling_notify_leaves_the_warning_mirror_in_mini() {
        let mut m = NotificationManager::new(NoiceSettings {
            notify_enabled: false,
            ..Default::default()
        });
        m.push(warn("careful"));
        assert_eq!(m.active(BackendKind::Notify).len(), 0);
        assert_eq!(m.active(BackendKind::Mini).len(), 1);
    }

    #[test]
    fn test_history_respects_max() {
        let mut m = NotificationManager::new(NoiceSettings { max_history: 3, ..Default::default() });
        m.push(info_notif("1"));
        m.push(info_notif("2"));
        m.push(info_notif("3"));
        m.push(info_notif("4"));
        assert_eq!(m.history().len(), 3);
    }
}
