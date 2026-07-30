# Flash Jump Mode — Design Spec

**Date:** 2026-07-29
**Status:** Approved
**Implements:** Phase 6 — Flash (Jump Mode)

## Overview

Replace Vim's `f`/`F` inline character find with a **flash jump mode**: press `f` in Normal mode to overlay adaptive labels on every visible word. Type one or two characters to jump directly to that word. Adaptive labels use 1-char labels (`a`–`z`) when ≤26 visible words, 2-char labels (`aa`, `ab`…) when more.

## State

New field on `App`:

```rust
flash: Option<FlashState>
```

Where:

```rust
struct FlashState {
    labels: Vec<FlashLabel>,   // all visible labels
    pending: Option<char>,     // first char typed (for 2-char filtering)
}

struct FlashLabel {
    label: String,    // "a", "b", …, "aa", "ab", …
    offset: usize,    // buffer offset of the word's start
}
```

## Trigger

- **Key:** `f` in Normal mode, when `VimState::is_normal_idle()` returns true (no pending operator, count, or leader sequence).
- **Effect:** Compute labels for the visible range of the active window → set `flash = Some(FlashState { … })` → render overlay.
- `f` **completely replaces** Vim's inline character find (`f{char}`). There is no fallback or alternate key for the old behavior.

## Label Generation

1. Walk every visible line in the active window.
2. For each line, find word boundaries: sequences matching `[a-zA-Z0-9_]` of length ≥ 1.
3. Assign labels from a sequential pool:
   - First 26 words → `"a"`, `"b"`, …, `"z"`
   - Next 26 → `"aa"`, `"ab"`, …, `"az"`
   - Next 26 → `"ba"`, `"bb"`, …, `"bz"`
   - And so on.
4. Each label maps to the buffer offset of that word's first character.
5. Labels are tagged with a screen position (row, col) for overlay rendering.

## Interaction

### First keystroke (after `f`)
- User types `c` → filter labels to those starting with `c`.
- Overlay updates to show only the **second character** of filtered labels. For 1-char labels, the label itself remains visible.
- If exactly one match remains → jump immediately without waiting for a second char.

### Second keystroke
- User types `o` → find label `"co"` in filtered set → `Action::Move(Motion::To(offset))` → clear flash state.
- If no match → ignore keystroke, stay in flash mode (don't cancel).

### Cancel
- `Esc` → clear flash state, no jump.
- Any key that doesn't match a label prefix → clear flash state, no jump.
- Mouse click → clear flash state, no jump.
- Mode switch (entering Insert/Cmdline/Visual) → clear flash state.

## Rendering

### Data flow
`FlashState` is rendered into `WindowView` during `App::render()`:

```rust
pub struct WindowView {
    // … existing fields …
    pub flash_labels: Vec<FlashLabelRender>,
}

pub struct FlashLabelRender {
    pub row: u16,       // screen row (relative to window)
    pub col: u16,       // screen col
    pub text: String,   // the label characters to show
    pub color: Color,   // label foreground color
}
```

### TUI renderer
- In the TUI renderer's per-window rendering pass, after drawing all `StyledLine`s, iterate `window.flash_labels`.
- For each label, draw the label text at `(window_y + row, window_x + col)` using the flash label color (cyan/yellow) and dimmed background. The label **replaces** the underlying character(s) visually — the original word text is hidden behind the overlay.
- **First keystroke:** labels dim to a muted color except for matching candidates, which stay bright.
- **Second keystroke:** all labels disappear (transition state is cleared before the next frame).

### GUI renderer (Raylib)
- Same data flow: `WindowView.flash_labels` is read by the Raylib renderer and drawn as colored text quads at the appropriate screen position.

## Integration with Vim State

- Flash mode is **orthogonal to Vim mode**. It is not a new `VimMode` variant.
- The check is purely: `flash.is_some() && is_normal_idle()`.
- When flash is active, keystrokes are intercepted **before** the Vim state machine:
  1. Label char → process first/second keystroke.
  2. `Esc` → cancel.
  3. Any other key → cancel and **replay** the key into the normal dispatch chain (so `j`, `k`, etc. still work after a cancelled flash).
- `f` replaces inline find **everywhere**. In Visual, Insert, or other non-Normal modes, `f` does nothing (the old inline-find code path is removed). Only Normal mode gets flash.

## Noice Integration

When Flash is complete, Noice will be designed and implemented in a separate cycle. The two features share no state or data flow.

## Testing

- **Unit tests:**
  - `flash_state_initialized_on_f()` — pressing `f` in Normal sets `flash.is_some()`.
  - `flash_label_generation_wraps_alphabet` — verify labels are `a-z`, `aa-az`, `ba-bz`.
  - `flash_first_char_filters_and_updates_pending` — typing `c` sets `pending = Some('c')` and filters labels.
  - `flash_second_char_jumps` — typing `co` after `c` moves cursor to the matching offset.
  - `flash_single_match_jumps_immediately` — if only one label starts with `c`, jump on first char.
  - `flash_cancel_on_esc` — `Esc` clears flash state.
  - `flash_cancel_replays_key` — typing `j` during flash cancels and replays `j`.
- **Existing tests:** all 117 existing tests must pass.
