# Noice — Modern Notification System

**Date:** 2026-07-29
**Phase:** 6 (Advanced UX & Ecosystem)
**Status:** Design

## Overview

Noice replaces the ad-hoc `self.message: Option<String>` system with a proper notification pipeline. Non-blocking messages with levels, sources, timestamps, routing, and auto-dismiss. Ports the concepts from `folke/noice.nvim` to ruster's Rust+Lua architecture.

## Architecture

### Crate: `ruster-notify`

A new leaf crate depending only on `ruster-core` (for `MessageLevel`/`MessageSource`). Consumed by `ruster-tui` (rendering) and `ruster-lua` (config/API).

```
ruster-tui ──> ruster-notify <── ruster-lua
                    │
              ruster-core
```

### Notification Model

```rust
pub struct Notification {
    pub id: u64,
    pub level: MessageLevel,       // Info, Success, Warning, Error
    pub source: MessageSource,     // Echo, Lsp, Build, Test, Task, System
    pub title: Option<String>,
    pub text: String,
    pub created_at: std::time::SystemTime,  // for history display
    pub timeout: Option<Duration>, // None = persistent | Some(0) = use manager default
}

pub struct BackendConfig {
    pub kind: BackendKind,
    pub enabled: bool,
    pub default_timeout: Option<Duration>,
}

pub enum BackendKind {
    Mini,
    Notify,
    Split,
    CmdlinePopup,  // stub — needs floating windows
    Popup,         // stub — needs floating windows
    Confirm,       // stub — needs floating windows
}
```

### NotificationManager

```rust
pub struct NotificationManager {
    history: Vec<Notification>,
    active: BTreeMap<BackendKind, Vec<Notification>>,
    next_id: u64,
    default_timeout: Duration,  // applied when notif.timeout == Some(0)
    max_history: usize,         // default 1000
}
```

Methods:
- `push(notif)` — routes notification to backends, logs to history
- `dismiss(id)` — dismiss single notification
- `dismiss_all()` — dismiss all active
- `history()` → `&[Notification]`
- `active(kind)` → `&[Notification]`
- `tick(now)` — auto-dismiss expired notifications

### Default Routing

| Level | Backends | Timeout |
|-------|----------|---------|
| Info | Mini | 2s |
| Success | Mini | 2s |
| Warning | Notify + Mini | 5s |
| Error | Notify | persistent |

### View Backends

| Backend | Status | Description |
|---------|--------|-------------|
| Mini | Real | 1-line toast overlay near top-right. Auto-dismiss. Uses same overlay rendering as Flash labels. |
| Notify | Real | Stacking notification panel. Slides in from right edge. Scrollable list grouped by source. Toggle via `:Noice`. |
| Split | Real | History panel — pinned `*noice*` buffer with timestamps, levels, source icons. Opens via `:Noice split` or `:Noice history`. |
| CmdlinePopup | Stub | Placeholder. No-op push. Ready for floating windows. |
| Popup | Stub | Placeholder. No-op push. Ready for floating windows. |
| Confirm | Stub | Placeholder. No-op push. Ready for floating windows. |

### Lua API

```lua
ruster.api.notify("text")                           -- Info, Echo
ruster.api.notify.warn("text")                      -- Warning
ruster.api.notify.error("text")                     -- Error
ruster.api.notify.with({ title = "Build",           -- explicit
    text = "done",
    level = "success",
    timeout = 0 })                                   -- 0 = persistent
```

### `:echo` Commands

```
:echo Hello world                → Mini (Info, 2s)
:echom "Warning"                 → Notify + Mini (Warning, 5s)
:echoe "Error"                   → Notify (Error, persistent)
```

### App Integration

`App` gains `notify: NotificationManager`.

**FrameState:** Gains two new fields replacing `message`:
- `notify_mini: Option<Vec<&Notification>>` — active Mini toasts
- `notify_stack: Option<Vec<&Notification>>` — active Notify stack

**Migration:** All `self.message = Some(...)` sites are converted to `self.notify.push(...)`. The old `self.message` field is removed. `state.message` in FrameState is removed.

**Per-frame:** `self.notify.tick(now)` runs at the start of `render()` to dismiss expired notifications. Active notifications are queried and passed to `FrameState`.

### Rendering

**Mini:** Overlay rendering in `renderer.rs` — positioned at row 1 (below any statusline), right-aligned at `area.width - 2`. Each toast is a single line with styled background. Multiple toasts stack downward: first at row 1, second at row 2, etc.

**Notify:** Overlay panel rendered when the Notify stack is visible. Slides in from the right edge (or opens inline). Each entry shows level icon, timestamp, title, and text. Scrollable.

**Split:** Uses the existing buffer/pinned mechanism. `:Noice split` creates a `*noice*` buffer and populates it from `manager.history()`. Kept in sync on notification push.

### Config (Lua overrides)

```lua
ruster.config.noice = {
    backends = { mini = true, notify = true, split = true },
    timeout = { info = 2000, success = 2000, warning = 5000 },
    max_history = 1000,
    -- Optional route overrides:
    -- filters = { { source = "lsp", level = "error", backend = "notify" } }
}
```

Defaults are hardcoded in ruster-notify; Lua overrides merge on top.
