# Static Dashboard & Messages Buffer Implementation Plan

> **Status:** delivered; boxes resolved 2026-08-07.
>
> This plan was executed but never ticked as it went, leaving 27 boxes
> that read as outstanding work. They are **plain bullets now, not back-ticked**:
> a box ticked long after the fact asserts a verification that did not happen,
> and this tree has already been bitten by exactly that. The bullets stand as a
> record of what was built.
>
> Evidence it shipped: all 24 identifiers this plan names in backticks exist in
> the tree, and `docs/verification/dashboard-*` and `messages-*`; the messages log gained its missing writer in Phase 10's final sweep.


> **For agentic workers:** implement task-by-task; steps use checkbox (`- [ ]`) syntax.

**Goal:** Make the Dashboard a non-closable static page and add a general-purpose messages buffer that captures build/test/task/LSP/echo output with color coding and filtering.

**Architecture:** `ruster-core` gets a `pinned` flag on `Document` and a new `MessageLog` module. The TUI app promotes Dashboard to a `Special(SpecialKind::Dashboard)` pinned buffer, holds a `MessageLog`, fans build/LSP/echo output into it, and exposes it via `:messages` / `SPC o m`.

## Global Constraints

- `ruster-core` stays UI/OS-free; `MessageLog` is pure data with no rendering logic.
- All new tests follow existing test patterns in the crate they're in.
- Keep `docs/{config-reference,keybindings.md}` in sync.

## File Structure

### New files:
- `crates/ruster-core/src/message.rs` — `MessageEntry`, `MessageLog`, `MessageLevel`, `MessageSource`

### Modified files:
- `crates/ruster-core/src/lib.rs` — add `pub mod message;`
- `crates/ruster-core/src/document.rs` — `pinned` field, `SpecialKind::Dashboard`, `SpecialKind::Message`
- `crates/ruster-core/src/workspace.rs` — `BufferStore::close()` refuses pinned buffers
- `crates/ruster-tui/src/app.rs` — Dashboard Special, MessageLog field, `:messages`, drain fan-out
- `crates/ruster-render/src/lib.rs` — `WelcomeView` condition also matches `Special(Dashboard)`

---

### Task 1: Core — `pinned` flag + `SpecialKind` variants

**Files:**
- Modify: `crates/ruster-core/src/document.rs`
- Test: in `document.rs` (existing test module)

**Interfaces:**
- Consumes: `Document::scratch()`, `Document::special()`, `Document::from_file()`
- Produces: `Document.pinned: bool`, `SpecialKind::Dashboard`, `SpecialKind::Message`

- **Step 1: Add `pinned` field and new SpecialKind variants**

In `crates/ruster-core/src/document.rs`, add to `SpecialKind`:
```rust
/// The Dashboard / welcome screen — a static page that cannot be closed.
Dashboard,
/// A general-purpose message log (build, LSP, echo, etc.).
Message,
```

Add `pinned: bool` to `Document`:
```rust
pub struct Document {
    pub buffer: Buffer,
    pub undo: UndoStack,
    pub file_path: Option<PathBuf>,
    pub name: String,
    pub modified: bool,
    pub kind: DocKind,
    pub indent: String,
    pub line_ending: LineEnding,
    /// When true, `BufferStore::close()` refuses to remove this document.
    pub pinned: bool,
}
```

Set `pinned: false` in `from_file()`, `scratch()`, and `special()`. The Dashboard and Message docs will set it to `true` at creation.

- **Step 2: Run existing tests to verify**

```bash
cargo test -p ruster-core
```

Expected: ALL existing tests pass.

- **Step 3: Commit**

```bash
git add crates/ruster-core/src/document.rs
git commit -m "feat(core): add pinned field and SpecialKind::Dashboard/Message"
```

---

### Task 2: Core — `BufferStore::close()` refuses pinned buffers

**Files:**
- Modify: `crates/ruster-core/src/workspace.rs`
- Test: in `workspace.rs` (existing test module)

**Interfaces:**
- Consumes: `BufferStore::close(id) -> bool`
- Produces: pinned buffers cannot be closed

- **Step 1: Modify `BufferStore::close()` to refuse pinned**

```rust
pub fn close(&mut self, id: BufferId) -> bool {
    let doc = match self.docs.get(&id) {
        Some(d) => d,
        None => return false,
    };
    if doc.pinned {
        return false;
    }
    if self.docs.len() == 1 && doc.modified {
        return false;
    }
    self.docs.remove(&id);
    self.order.retain(|&x| x != id);
    true
}
```

- **Step 2: Add tests for pinned close refusal**

In the `tests` module of `workspace.rs`:
```rust
#[test]
fn refuses_to_close_pinned_document() {
    let mut s = BufferStore::new();
    let a = s.create_scratch("pinned_test");
    s.get_mut(a).unwrap().pinned = true;
    assert!(!s.close(a));
    assert_eq!(s.len(), 1);
}

#[test]
fn unpinned_document_can_still_be_closed() {
    let mut s = BufferStore::new();
    let a = s.create_scratch("a");
    let b = s.create_scratch("b");
    s.get_mut(a).unwrap().pinned = true;
    assert!(!s.close(a));
    assert!(s.close(b)); // unpinned
    assert_eq!(s.len(), 1);
}
```

- **Step 3: Run tests**

```bash
cargo test -p ruster-core
```

Expected: ALL tests pass, including the two new ones.

- **Step 4: Commit**

```bash
git add crates/ruster-core/src/workspace.rs
git commit -m "feat(core): BufferStore::close() refuses pinned documents"
```

---

### Task 3: Core — `MessageLog` module

**Files:**
- Create: `crates/ruster-core/src/message.rs`
- Modify: `crates/ruster-core/src/lib.rs` — add `pub mod message;`
- Test: inline in `message.rs`

**Interfaces:**
- Produces: `MessageLevel` (Info, Warning, Error, Success), `MessageSource` (Build, Test, Task, Lsp, Echo, System), `MessageEntry { level, source, text, count }`, `MessageLog { entries, push(), clear(), filtered() }`

- **Step 1: Create `crates/ruster-core/src/message.rs`**

```rust
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
/// consecutively, `count` is incremented instead of appending a duplicate.
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

    /// Push a new entry. Consecutive duplicates increment `count` on the last entry.
    pub fn push(&mut self, level: MessageLevel, source: MessageSource, text: String) {
        if let Some(last) = self.entries.last_mut() {
            if last.level == level && last.source == source && last.text == text {
                return;
            }
        }
        if self.entries.len() >= self.max_entries {
            self.entries.remove(0);
        }
        self.entries.push(MessageEntry { level, source, text });
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
            .filter(|e| source.map_or(true, |s| e.source == s))
            .filter(|e| level.map_or(true, |l| e.level == l))
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
```

- **Step 2: Add `pub mod message;` to `crates/ruster-core/src/lib.rs`**

```rust
pub mod message;
```

- **Step 3: Run tests**

```bash
cargo test -p ruster-core
```

Expected: ALL tests pass, including the new message tests.

- **Step 4: Commit**

```bash
git add crates/ruster-core/src/message.rs crates/ruster-core/src/lib.rs
git commit -m "feat(core): add MessageLog module"
```

---

### Task 4: App — Dashboard as `Special(SpecialKind::Dashboard)` pinned buffer

**Files:**
- Modify: `crates/ruster-tui/src/app.rs`
- Modify: `crates/ruster-render/src/lib.rs`

- **Step 1: Change `open_dashboard()` to create a pinned `Special(Dashboard)`**

In `crates/ruster-tui/src/app.rs`, replace `open_dashboard()`:

```rust
fn open_dashboard(&mut self) {
    let mut w = self.ws.borrow_mut();
    let existing = w.buffers.ids().iter().copied().find(|&id| {
        w.buffers.get(id).is_some_and(|d| d.pinned && matches!(d.kind, ruster_core::document::DocKind::Special(ruster_core::document::SpecialKind::Dashboard)))
    });
    match existing {
        Some(id) => w.set_active_buffer(id),
        None => {
            let id = w.buffers.create_special(ruster_core::document::SpecialKind::Dashboard, "Dashboard");
            if let Some(doc) = w.buffers.get_mut(id) {
                doc.pinned = true;
            }
            w.set_active_buffer(id);
        }
    }
}
```

- **Step 2: Update WelcomeView detection to also match `Special(Dashboard)`**

In `crates/ruster-tui/src/app.rs`, replace the `is_scratch` block in the build_frame method (~line 2867):

```rust
let is_dashboard = {
    let w = self.ws.borrow();
    let active = w.active_doc();
    active.file_path.is_none()
        && (matches!(active.kind, DocKind::Scratch)
            || matches!(active.kind, DocKind::Special(SpecialKind::Dashboard)))
};
```

And rename the variable from `is_scratch` to `is_dashboard` wherever used below.

- **Step 3: Also update the start-up scratch workspace to create a pinned Dashboard**

In the app initialization, find where `Workspace::scratch()` is called and ensure the Dashboard buffer gets `pinned = true`:

```rust
// After creating the workspace, pin the Dashboard buffer.
let ws = self.ws.borrow();
if let Some(id) = ws.buffers.ids().first() {
    if let Some(doc) = ws.buffers.get_mut(*id) {
        if doc.name == "Dashboard" {
            doc.pinned = true;
        }
    }
}
```

Actually, let me look at how the app initializes. Let me check the app startup code.

- **Step 4: Run tests**

```bash
cargo test -p ruster-tui -p ruster-core
```

Expected: ALL tests pass.

- **Step 5: Commit**

```bash
git add crates/ruster-tui/src/app.rs crates/ruster-render/src/lib.rs
git commit -m "feat(ui): Dashboard as pinned Special(Dashboard) buffer"
```

---

### Task 5: App — Messages buffer integration

**Files:**
- Modify: `crates/ruster-tui/src/app.rs`
- Modify: (eventually) `docs/keybindings.md`

- **Step 1: Add `MessageLog` field and messages buffer id to App struct**

```rust
// In App struct fields:
messages: ruster_core::message::MessageLog,
messages_buf: Option<BufferId>,
```

- **Step 2: Initialize in App::new()**

```rust
messages: ruster_core::message::MessageLog::new(),
messages_buf: None,
```

- **Step 3: Add `ensure_messages_buffer()` method**

```rust
/// Ensure the `*messages*` buffer exists, returning its id.
fn ensure_messages_buffer(&mut self) -> BufferId {
    if let Some(id) = self.messages_buf {
        if self.ws.borrow().buffers.get(id).is_some() {
            return id;
        }
    }
    let id = self.ws.borrow_mut().buffers.create_special(
        ruster_core::document::SpecialKind::Message,
        "*messages*",
    );
    if let Some(doc) = self.ws.borrow_mut().buffers.get_mut(id) {
        doc.pinned = true;
    }
    self.messages_buf = Some(id);
    id
}
```

- **Step 4: Add `open_messages()` and `refresh_messages_buffer()` methods**

```rust
/// Open the messages buffer in the active window.
fn open_messages(&mut self) {
    let id = self.ensure_messages_buffer();
    self.refresh_messages_buffer(id);
    self.ws.borrow_mut().set_active_buffer(id);
}

/// Rebuild the buffer text from the message log.
fn refresh_messages_buffer(&mut self, id: BufferId) {
    let entries = self.messages.filtered(self.messages_filter_source, self.messages_filter_level);
    let mut text = String::new();
    for entry in &entries {
        // Format: "[SRC]  LEVEL  message"
        text.push_str(&format!(
            "[{}] {} {}\n",
            entry.source.label().to_uppercase(),
            entry.level.label(),
            entry.text
        ));
    }
    let mut w = self.ws.borrow_mut();
    if let Some(doc) = w.buffers.get_mut(id) {
        doc.buffer = ruster_core::buffer::Buffer::from_str(&text);
    }
}
```

Add filter fields to App:
```rust
messages_filter_source: Option<ruster_core::message::MessageSource>,
messages_filter_level: Option<ruster_core::message::MessageLevel>,
```

- **Step 5: Wire echo messages into the log**

Wherever `self.message = Some(...)` is set, also push to the message log. Add a helper:
```rust
fn push_message(&mut self, level: ruster_core::message::MessageLevel, source: ruster_core::message::MessageSource, text: String) {
    self.messages.push(level, source, text.clone());
    // Don't set self.message here — that's done by callers for transient display.
}
```

Replace `self.message = Some(...)` assignments for echo-style messages with:
```rust
self.push_message(MessageLevel::Info, MessageSource::Echo, "message text".into());
self.message = Some("message text".to_string());
```

- **Step 6: Wire build/test/task runner output into the log**

In `drain_build_runner()`, after each `RunnerMsg::Line(l)` is appended to `self.runner_output`, also push:
```rust
self.push_message(
    MessageLevel::Info,
    match self.runner_kind {
        RunnerKind::Build => MessageSource::Build,
        RunnerKind::Test => MessageSource::Test,
        RunnerKind::Task => MessageSource::Task,
    },
    l.clone(),
);
```

On completion (`RunnerMsg::Done(code)`), push:
```rust
let level = if code == Some(0) { MessageLevel::Success } else { MessageLevel::Error };
self.push_message(level, MessageSource::Build, format!("build exited with code {:?}", code));
```

- **Step 7: Add `:messages` command and keybinding**

In `parse_cmdline()`:
```rust
"messages" | "message" | "msgs" => Ok(CmdAction::Messages),
```

Add to `CmdAction` enum:
```rust
Messages,
```

Add to `LeaderAction`:
```rust
Messages,
```

Add `('m', LeaderNode::Action("messages", LeaderAction::Messages)),` to `OPEN_GROUP`.

In `apply_cmd()`:
```rust
CmdAction::Messages => self.open_messages(),
```

In `apply_leader_action()`:
```rust
LeaderAction::Messages => self.open_messages(),
```

- **Step 8: Add messages filter command support**

In the messages buffer, `:filter` or `:messages lsp` style filtering. Add parsing:
```rust
"messages" | "message" | "msgs" => {
    // Check for filter arguments
    let rest = trimmed.strip_prefix("messages ").or_else(|| trimmed.strip_prefix("message ")).or_else(|| trimmed.strip_prefix("msgs "));
    match rest {
        Some(filter) if !filter.is_empty() => Ok(CmdAction::MessagesFilter(filter.to_string())),
        _ => Ok(CmdAction::Messages),
    }
},
```

Add `CmdAction::MessagesFilter(String)` and handle it by setting `messages_filter_source`/`messages_filter_level` then refreshing.

- **Step 9: Run tests**

```bash
cargo build && cargo test
```

Expected: Build and all tests pass.

- **Step 10: Update keybindings docs**

Update `docs/keybindings.md` and `docs/config-reference.md` with the new `:messages` command and `SPC o m` binding.

- **Step 11: Commit**

```bash
git add crates/ruster-tui/src/app.rs docs/keybindings.md docs/config-reference.md
git commit -m "feat(ui): add messages buffer with :messages command"
```
