# Flash Jump Mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace Vim's `f`/`F` inline character find with a flash jump mode — press `f` in Normal mode to overlay adaptive labels on every visible word, type 1-2 characters to jump.

**Architecture:** New `FlashState` on `App` (orthogonal to Vim state machine) with overlay labels in `WindowView`. Label computation walks visible buffer lines and assigns adaptive labels. TUI renderer draws labels over text. First char filters, second char jumps. Tests use `App::new("content".into(), path)` pattern.

**Tech Stack:** Rust, ruster-tui (app.rs, renderer.rs), ruster-render (lib.rs, Color::Rgb), ruster-render-raylib (lib.rs)

## Global Constraints

- `f` replaces inline find everywhere — no fallback
- Flash mode only triggers in Normal mode (`is_normal_idle()`)
- Esc or non-label key cancels and replays into normal dispatch
- Adaptive labels: `a`–`z` for ≤26 visible words, then `aa`, `ab`… for more
- Labels overlay the original characters visually (don't mutate the buffer)

---

### Task 1: Core FlashState struct, App field, trigger + cancel

**Files:**
- Modify: `crates/ruster-tui/src/app.rs`

**Interfaces:**
- Produces: `FlashLabel { label: String, offset: usize }`, `FlashState { labels: Vec<FlashLabel>, pending: Option<char> }`
- Produces: `App.flash: Option<FlashState>` field
- Produces: `f` key intercept in Normal mode that sets flash state
- Produces: Esc and non-label cancel with key replay

- [x] **Step 1: Add FlashState and FlashLabel structs**

Add at the top of `app.rs` near other type definitions (around line 25):

```rust
/// A single flash jump label.
#[derive(Debug, Clone)]
pub struct FlashLabel {
    pub label: String,
    pub offset: usize,
}

/// Active flash jump mode state.
#[derive(Debug)]
pub struct FlashState {
    pub labels: Vec<FlashLabel>,
    pub pending: Option<char>,
}
```

- [x] **Step 2: Add flash field to App struct**

In the `App` struct, add after `leader_since` (around line 1370):

```rust
    pub flash: Option<FlashState>,
```

Initialize to `None` in the constructor where other Option fields are set (around line 1368):

```rust
    flash: None,
```

- [x] **Step 3: Add f key intercept in handle_key**

In `App::handle_key()`, find the section where Vim Normal-mode bare key dispatch happens (after which-key/leader checks, before Vim state machine dispatch). Check the actual vim mode variable name — it may be `self.vim.mode`, `self.vim_state`, or similar. Search for `is_normal_idle()`. Add before the Vim state machine call:

```rust
// Flash jump mode (f replaces inline find).
if ck.code == KeyCode::Char('f') && ck.modifiers.is_empty() && self.vim.is_normal_idle() {
    // Label generation will be added in Task 2.
    // For now, set a placeholder state to verify intercept works.
    self.message = Some("flash: f pressed".to_string());
    return true;
}
```

- [x] **Step 4: Add cancel logic (outside flash mode check)**

When flash is active (`self.flash.is_some()`), intercept keystrokes before the Vim state machine. Add this block right before the Vim dispatch, after the `f` check:

```rust
// Flash mode active — intercept or cancel.
if let Some(ref flash_state) = self.flash {
    match ck.code {
        KeyCode::Esc => {
            self.flash = None;
            self.message = None;
            return true;
        }
        KeyCode::Char(c) if c.is_ascii_lowercase() => {
            // Will handle labels in Task 3
            self.message = Some(format!("flash: key {c}"));
            return true;
        }
        _ => {
            // Cancel and replay the key.
            let ev = ck.clone();
            self.flash = None;
            // fall through to normal dispatch (don't return)
        }
    }
}
```

Note: The `_` arm does NOT return — it cancels flash and falls through to normal key handling so the key is replayed.

- [x] **Step 5: Build to verify**

Run: `cargo build -p ruster-tui 2>&1 | tail -5`
Expected: clean build

- [x] **Step 6: Run tests**

Run: `cargo test -p ruster-tui 2>&1 | tail -5`
Expected: 117 tests pass

- [x] **Step 7: Commit**

```bash
git add crates/ruster-tui/src/app.rs
git commit -m "feat(flash): scaffold FlashState, f trigger, and cancel/replay"
```

---

### Task 2: Label generation for visible range

**Files:**
- Modify: `crates/ruster-tui/src/app.rs`
- Test: `crates/ruster-tui/src/app.rs` (test module)

**Interfaces:**
- Produces: `App::compute_flash_labels(&self) -> Vec<FlashLabel>` — scans visible window lines, finds words, assigns adaptive labels
- Consumes: `self.ws.window(wid)`, `.scroll`, `.cursors`, `Buffer`

- [x] **Step 1: Add compute_flash_labels method**

Add this method to `App` (place near the flash-related code, after the `handle_key` method):

```rust
/// Generate flash jump labels for the visible range of the active window.
fn compute_flash_labels(&self) -> Vec<FlashLabel> {
    let Some(win) = self.active_window() else { return vec![] };
    let Some(buf) = self.active_buffer() else { return vec![] };
    let rect = win.rect;
    let scroll = win.scroll as usize;
    let visible_lines = rect.h as usize;
    let mut labels = Vec::new();
    let mut label_pool = label_pool_iter();

    for line_idx in 0..visible_lines {
        let buf_line = scroll + line_idx;
        if buf_line >= buf.line_count() { break; }
        let line_start = buf.line_to_offset(buf_line).unwrap_or(0);
        let line_len = buf.line_length(buf_line).unwrap_or(0);
        let text = match buf.slice(line_start..line_start + line_len) {
            Some(s) => s.to_string(),
            None => continue,
        };
        // Scan for word boundaries.
        let bytes = text.as_bytes();
        let mut pos = 0;
        while pos < bytes.len() {
            if bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_' {
                let word_start = pos;
                while pos < bytes.len() && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') {
                    pos += 1;
                }
                if let Some(label) = label_pool.next() {
                    labels.push(FlashLabel {
                        label,
                        offset: line_start + word_start,
                    });
                }
            } else {
                pos += 1;
            }
        }
    }
    labels
}

/// Infinite iterator over adaptive labels: a-z, aa-az, ba-bz, …
fn label_pool_iter() -> impl Iterator<Item = String> {
    let single = ('a'..='z').map(|c| c.to_string());
    let multi = ('a'..='z').flat_map(|first| {
        ('a'..='z').map(move |second| format!("{}{}", first, second))
    });
    single.chain(multi)
}
```

- [x] **Step 2: Wire label generation into the f trigger**

Update the `f` key handling in `handle_key` (from Task 1 Step 3). Replace the placeholder message with real label generation:

```rust
// Flash jump mode (f replaces inline find).
if ck.code == KeyCode::Char('f') && ck.modifiers.is_empty() && self.vim.is_normal_idle() {
    let labels = self.compute_flash_labels();
    if labels.is_empty() {
        return true;
    }
    self.flash = Some(FlashState {
        labels,
        pending: None,
    });
    return true;
}
```

- [x] **Step 3: Build to verify**

Run: `cargo build -p ruster-tui 2>&1 | tail -5`
Expected: clean build

- [x] **Step 4: Add unit tests for label generation**

Add tests in the `app::tests` module:

```rust
#[test]
fn flash_label_pool_starts_with_a() {
    let mut pool = super::label_pool_iter();
    assert_eq!(pool.next(), Some("a".to_string()));
    assert_eq!(pool.next(), Some("b".to_string()));
}

#[test]
fn flash_label_pool_wraps_to_aa_after_z() {
    let mut pool = super::label_pool_iter();
    // Skip a-z
    for _ in 0..26 { pool.next(); }
    assert_eq!(pool.next(), Some("aa".to_string()));
    assert_eq!(pool.next(), Some("ab".to_string()));
}

#[test]
fn flash_label_pool_ba_follows_az() {
    let mut pool = super::label_pool_iter();
    // Skip a-z, aa-az (26 + 26 = 52)
    for _ in 0..52 { pool.next(); }
    assert_eq!(pool.next(), Some("ba".to_string()));
}
```

- [x] **Step 5: Run tests**

Run: `cargo test -p ruster-tui 2>&1 | tail -5`
Expected: all tests pass (121+)

- [x] **Step 6: Commit**

```bash
git add crates/ruster-tui/src/app.rs
git commit -m "feat(flash): add compute_flash_labels with adaptive label pool"
```

---

### Task 3: First/second keystroke interaction

**Files:**
- Modify: `crates/ruster-tui/src/app.rs`
- Test: `crates/ruster-tui/src/app.rs`

**Interfaces:**
- Consumes: `FlashState.pending`, `FlashState.labels`
- Produces: filtered label display, jump on second char or single match

- [x] **Step 1: Implement first/second char handling**

Replace the placeholder `KeyCode::Char(c)` arm in the flash mode dispatch (from Task 1 Step 4) with real logic:

```rust
KeyCode::Char(c) if c.is_ascii_lowercase() => {
    // Take ownership temporarily.
    let mut fs = self.flash.take().unwrap();
    let result = match fs.pending {
        None => {
            // First keystroke — filter by first char.
            let matching: Vec<&FlashLabel> = fs.labels.iter()
                .filter(|l| l.label.starts_with(c))
                .collect();
            if matching.is_empty() {
                // No match, cancel.
                self.flash = None;
                return true;
            }
            if matching.len() == 1 {
                // Single match — jump immediately.
                self.ws.borrow_mut().execute(Action::Move(Motion::To(matching[0].offset)));
                self.flash = None;
                // Update the cursor drawn position
                return true;
            }
            // Multiple matches — filter and wait for second char.
            fs.labels = matching.into_iter().cloned().collect();
            fs.pending = Some(c);
            self.flash = Some(fs);
            true
        }
        Some(first) => {
            // Second keystroke — find exact label match.
            let target = format!("{}{}", first, c);
            if let Some(label) = fs.labels.iter().find(|l| l.label == target) {
                self.ws.borrow_mut().execute(Action::Move(Motion::To(label.offset)));
            }
            self.flash = None;
            true
        }
    };
    return result;
}
```

- [x] **Step 2: Build to verify**

Run: `cargo build -p ruster-tui 2>&1 | tail -5`
Expected: clean build

- [x] **Step 3: Run tests**

Run: `cargo test -p ruster-tui 2>&1 | tail -5`
Expected: all tests pass

- [x] **Step 4: Commit**

```bash
git add crates/ruster-tui/src/app.rs
git commit -m "feat(flash): implement first/second keystroke with immediate jump on single match"
```

---

### Task 4: Overlay rendering — WindowView field + TUI renderer

**Files:**
- Modify: `crates/ruster-render/src/lib.rs` (WindowView, FlashLabelRender)
- Modify: `crates/ruster-tui/src/app.rs` (render method — populate flash_labels)
- Modify: `crates/ruster-tui/src/renderer.rs` (draw flash labels)

**Interfaces:**
- Consumes: `FlashState.labels`, `FlashState.pending`
- Produces: `WindowView.flash_labels: Vec<FlashLabelRender>`

- [x] **Step 1: Add FlashLabelRender to ruster-render**

In `crates/ruster-render/src/lib.rs`, add after the `WindowView` struct fields (~line 380):

```rust
/// A single flash label to render at a screen position.
#[derive(Debug, Clone)]
pub struct FlashLabelRender {
    pub row: u16,
    pub col: u16,
    pub text: String,
    pub color: Color,
}
```

Add the field to `WindowView`:

```rust
pub flash_labels: Vec<FlashLabelRender>,
```

Initialize as `flash_labels: Vec::new()` wherever `WindowView` is constructed (search for `WindowView {` in the file).

The color type in `ruster-render` is `Color::Rgb(u8, u8, u8)` (defined at lib.rs:13). Use it directly:

```rust
pub color: Color,
```

- [x] **Step 2: Populate flash_labels in App::render()**

In `App::render()`, after all windows are processed (look for the section that builds `WindowView` for each window — around line 3100-3200), add after the `window_view` is fully built:

```rust
// Flash overlay labels.
if let Some(ref flash) = self.flash {
    let Some(win) = self.active_window() else { continue };
    let wid = self.active_window_id();
    let is_active = window.id == wid;
    if is_active {
        let rect = win.rect;
        let scroll = win.scroll as usize;
        let visible_lines = rect.h as usize;
        // Build flash_labels from computed labels and current window.
        for fl in &flash.labels {
            // Find the line and column for this offset.
            let Some(buf) = self.active_buffer() else { continue };
            let line_no = buf.offset_to_line(fl.offset).unwrap_or(0);
            let line_start = buf.line_to_offset(line_no).unwrap_or(0);
            let col = fl.offset.saturating_sub(line_start);
            let screen_row = line_no.saturating_sub(scroll);
            if screen_row >= visible_lines { continue; }
            let (display_text, color) = if let Some(first) = flash.pending {
                // Show only second char for 2-char labels, full label for 1-char.
                let sub = if fl.label.len() > 1 {
                    fl.label[1..].to_string()
                } else {
                    fl.label.clone()
                };
                (sub, Color::Rgb(255, 255, 0)) // bright yellow for candidates
            } else {
                (fl.label.clone(), Color::Rgb(0, 200, 255)) // cyan for initial
            };
            window_view.flash_labels.push(FlashLabelRender {
                row: screen_row as u16,
                col: col as u16,
                text: display_text,
                color,
            });
        }
    }
}
```

- [x] **Step 3: Render flash labels in the TUI renderer**

In `crates/ruster-tui/src/renderer.rs`, in the per-window rendering function (around where `WindowView` content is drawn), add after drawing `StyledLine`s and highlights:

```rust
// Flash jump labels.
use ruster_render::Color as RenderColor;

for fl in &window.flash_labels {
    let y = win_y + fl.row;
    let x = win_x + fl.col;
    let fg_color = match fl.color {
        RenderColor::Rgb(r, g, b) => ratatui::style::Color::Rgb(r, g, b),
        RenderColor::Default => ratatui::style::Color::Reset,
    };
    let style = Style::default().fg(fg_color);
    let line = Line::from(Span::styled(&fl.text, style));
    let area = Rect::new(x, y, fl.text.len() as u16, 1);
    f.render_widget(Paragraph::new(line), area);
}
```

- [x] **Step 4: Build to verify**

Run: `cargo build -p ruster-tui -p ruster-render 2>&1 | tail -5`
Expected: clean build

- [x] **Step 5: Run all tests**

Run: `cargo test -p ruster-tui 2>&1 | tail -5`
Expected: 117+ tests pass

- [x] **Step 6: Commit**

```bash
git add crates/ruster-render/src/lib.rs crates/ruster-tui/src/app.rs crates/ruster-tui/src/renderer.rs
git commit -m "feat(flash): render flash jump labels as overlay in TUI"
```

---

### Task 5: (Optional) Raylib renderer support — SKIPPED

**Status:** Not implemented. The TUI is the actively used renderer; flash labels
are TUI-only for now. `WindowView.flash_labels` is populated regardless, so the
Raylib side is a drop-in addition whenever that renderer becomes primary.

**Files:**
- Modify: `crates/ruster-render-raylib/src/lib.rs`

Skip this task if the Raylib renderer is not actively being used. If it is, add equivalent flash label rendering using Raylib's drawing primitives, reading `WindowView.flash_labels` the same way.

- [ ] **Step 1: Add flash label drawing in raylib renderer**

Search for where `WindowView` lines are drawn in the raylib renderer. After rendering styled lines, add:

```rust
// Flash jump labels.
use ruster_render::Color as RenderColor;

for fl in &window.flash_labels {
    let x = win_x + fl.col as i32 * char_width;
    let y = win_y + fl.row as i32 * line_height;
    let color = match fl.color {
        RenderColor::Rgb(r, g, b) => rraylib::Color::new(r, g, b, 255),
        RenderColor::Default => rraylib::Color::new(200, 200, 200, 255),
    };
    rraylib::draw_text(&fl.text, x, y, font_size, color);
}
```

Adjust types to match the actual raylib renderer's API and available functions.

- [ ] **Step 2: Build to verify**

Run: `cargo build -p ruster-render-raylib 2>&1 | tail -5`
Expected: clean build

- [ ] **Step 3: Commit**

```bash
git add crates/ruster-render-raylib/src/lib.rs
git commit -m "feat(flash): render flash labels in Raylib GUI"
```

---

## Post-implementation review (2026-07-30)

Tasks 1–4 were verified against the running code. Four defects were found and fixed:

- **Label keys leaked into the Vim state machine.** The ambiguous-match and
  second-char paths in the flash dispatch fell through to `self.vim.handle()`
  instead of returning, so a two-char jump like `f` `a` `b` opened Insert mode on
  `a` and typed `b` into the buffer. Every label path now returns.
- **Label offsets mixed byte and char indices.** `compute_flash_labels` scanned
  `text.as_bytes()` while adding the result to a char offset (`line_start_char`),
  skewing every label after a multi-byte char. The scan now walks chars and uses
  Unicode-aware `is_alphanumeric()`, so non-ASCII words are labelable too.
- **Overlay ignored the sign column and line-number gutter.** The renderer drew
  labels at `view.rect.x + col`, but `BufferWidget` puts text at
  `x + signs.width + gutter.width` — labels landed on the line numbers. The
  overlay now mirrors the widget's layout and clips labels that would spill into
  a neighbouring split.
- Dead `let ev = ck.clone();` removed from the cancel arm.

Behaviour tests added (`app::tests::flash_*`): label order/offsets, char-offset
regression, immediate jump on a unique first char, two-char narrow-then-jump plus
a buffer-unchanged assertion, Esc cancel, and non-label cancel-and-replay.

**Known scope limits (intentional):** flash is Neovim-editmode only (`handle_key`
routes Emacs earlier); operator-pending `df`/`cf` still use Vim's inline find
since the trigger requires `is_normal_idle()`; label colors are hardcoded rather
than theme-driven.
