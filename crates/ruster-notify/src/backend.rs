/// Where an active notification is queued for display.
///
/// [`Mini`](Self::Mini) is the transient toast in the cmdline row and
/// [`Notify`](Self::Notify) the stacking panel; both are drawn from
/// `FrameState`. [`Split`](Self::Split) is the `*noice*` history buffer opened
/// by `:Noice split`, which reads
/// [`history()`](crate::NotificationManager::history) rather than an active
/// queue — it exists here so `noice.split_enabled` has something to name.
///
/// [`CmdlinePopup`](Self::CmdlinePopup) and [`Popup`](Self::Popup) are
/// floating boxes drawn above the window views — the difference is duration,
/// the same queueing machinery that expires `Mini`/`Notify`. They exist
/// because a notification that must be read without opening the stack panel
/// needs a real surface, and floats now render. [`Confirm`](Self::Confirm) is
/// the modal: it opens a dialog with OK/Cancel rather than drawing a float.
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
