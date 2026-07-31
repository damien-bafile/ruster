/// Where an active notification is queued for display.
///
/// [`Mini`](Self::Mini) is the transient toast in the cmdline row and
/// [`Notify`](Self::Notify) the stacking panel; both are drawn from
/// `FrameState`. [`Split`](Self::Split) is the `*noice*` history buffer opened
/// by `:Noice split`, which reads
/// [`history()`](crate::NotificationManager::history) rather than an active
/// queue — it exists here so `noice.split_enabled` has something to name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BackendKind {
    Mini,
    Notify,
    Split,
}

impl BackendKind {
    pub fn all() -> &'static [BackendKind] {
        &[BackendKind::Mini, BackendKind::Notify, BackendKind::Split]
    }
}
