# Cmdline & Which-Key UX Redesign

## Overview

Four interrelated UX improvements to the editor's command-line and which-key
hint panel:

1.  **Cmdline tab-completions panel** — Tab in `:` cmdline mode opens a
    which-key-style bottom panel with filtered command completions, instead of
    the current floating command-palette overlay.
2.  **Which-key back navigation** — `Backspace` (del) in a which-key group pops
    the leader sequence by one level; when the sequence is empty, cancel leader
    mode entirely. Same for the `g` menu.
3.  **Which-key key accent** — The binding letter in each which-key row is
    rendered in an accent color (`whichkey_key` → fallback `accent`), distinct
    from the description text.
4.  **Uniform display labels** — Clean up the Settings UI labels for the 5
    whichkey/cmdline color fields.

---

## 1. Cmdline Tab-Completions Panel

### Current behavior

- Tab in `:` cmdline mode closes the cmdline and opens a **floating
  command-palette** (centered overlay with title bar, query line, selectable
  list — `PickerState` / `PickerWidget`).
- `M-x` (Alt+x) also opens the same floating palette.

### New behavior

- **`M-x` binding is removed.** Alt+x is a no-op / passthrough.
- **Tab** in `:` cmdline mode opens a **which-key-style bottom panel** that
  slides up directly above the cmdline input row, showing filtered command
  completions. The cmdline input stays active.

### Visual layout

```
┌─────────────────────────────────────────┐
│                                         │
│           (buffer content)              │
│                                         │
├─────────────────────────────────────────┤ ← completions panel slides up
│   w    write file                       │     from here
│   q    quit / close window              │
│   sp   split horizontal                 │     bg = whichkey_bg
│   wq   write & quit                     │     fg = whichkey_fg
│   ...                                   │     key letter = whichkey_key
├─────────────────────────────────────────┤
│ :w                                      │ ← cmdline input row (existing)
│_                                        │     bg = cmdline_bg
└─────────────────────────────────────────┘     fg = cmdline_fg
```

### Interaction

| Key | Action |
|-----|--------|
| `Tab` | Toggle completions panel open/closed (if open, hide panel) |
| `Esc` | Hide completions panel (stay in cmdline mode) |
| `Up` / `Down` | Navigate selection in the completions list |
| `Enter` | Accept highlighted completion, close panel, close cmdline, run command |
| Type chars | Filter completions list (fuzzy match via `nucleo_matcher`) |

### Data model

```rust
/// State for the cmdline completions panel, stored in FrameState
/// (or a new field on the App).
struct CmdlineCompletions {
    /// Currently matching completions (fuzzy-filtered).
    rows: Vec<CmdlineCompletionItem>,
    /// Index of the highlighted row.
    selected: usize,
    /// Whether the panel is currently shown.
    visible: bool,
}

struct CmdlineCompletionItem {
    key: String,     // command name
    desc: String,    // description text for the entry
}
```

Completion items are built from `PALETTE_COMMANDS` (the same 30ish entries).

### Rendering

- **Widget**: New `CmdlineCompletionsWidget` — visually identical to
  `WhichKeyWidget` but with selection highlight and key accent.
- **Shared style**: Uses `whichkey_bg`, `whichkey_fg`, `whichkey_key` for
  consistency (the completions panel and which-key panel look the same).
- **Animation**: Slides up with same `anim`-based clip as which-key
  (instant visibility, no timer-based fade — just appears on Tab).
- **Seat**: Rendered in the area between the buffer content and the cmdline
  input row. The renderer draws it above the cmdline (at
  `area.height - 1 - panel_height`), same layout as which-key lifts above
  the statusline.

### Changes needed

**ruster-render/src/lib.rs:**
- No changes needed (reuses existing theme colors and WhichKeyEntry).

**ruster-tui/src/app.rs:**
- Remove `M-x` keybinding (Alt+x) from the Emacs mode handler.
- Change `Tab` in cmdline mode: instead of opening the floating picker,
  toggle `CmdlineCompletions.visible` and populate the filtered list.
- Handle `Esc`, `Up`, `Down`, `Enter` key events when completions panel is
  visible (while in cmdline mode).
- Accept logic: run the selected command by feeding it through the existing
  cmdline execution path.

**ruster-tui/src/widgets.rs:**
- New `CmdlineCompletionsWidget` — struct + Widget impl.
- Or extend `WhichKeyWidget` with optional selection state if the overlap
  is large enough.

**ruster-tui/src/renderer.rs:**
- Draw the completions panel above the cmdline when `visible` is true.
- Lift the cmdline row to accommodate the panel (just like statusline lift
  for which-key).

---

## 2. Which-Key Back Navigation

### Current behavior

- `handle_leader_key` accepts only `KeyCode::Char(c)` — Backspace, Delete,
  and all other non-char keys **cancel** the entire leader sequence.
- The `g` menu handler similarly ignores Backspace (falls through to passthrough).

### New behavior

- **Backspace** in leader mode (`leader_pending` active) pops the last
  character from `leader_pending`. Re-resolve the shortened sequence.
  - If the new sequence resolves to a **Group** → update the which-key panel
    to show that group's entries.
  - If the vector is **empty** → cancel leader mode entirely (same as Esc).
- **Same behavior for the `g` menu**: Backspace pops the `g` state and either
  returns to the root `g` wait state or cancels.

### Changes needed

**ruster-tui/src/app.rs — `handle_leader_key`:**
- Add `KeyCode::Backspace` handling:
  ```rust
  KeyCode::Backspace => {
      let seq = self.leader_pending.as_mut()?;
      seq.pop();
      if seq.is_empty() {
          self.leader_pending = None;
      } else {
          let snapshot = seq.clone();
          if matches!(leader_resolve(&snapshot), LeaderResolve::Group) {
              // which-key panel will update on next render
          } else {
              self.leader_pending = None;
          }
      }
  }
  ```

**ruster-tui/src/app.rs — `handle_g_key`:**
- If `g` has a pending sub-sequence, Backspace pops it; cancel when empty.

---

## 3. Which-Key Key Accent

### Current behavior

- `WhichKeyView::rows` is `Vec<String>` — each entry is a flat string like
  `"w  window management"`.
- Rendered in a single color (`whichkey_fg`).

### New behavior

- Each entry is a structured `{key: String, desc: String}`.
- The key letter (e.g., `w`) is rendered in `whichkey_key` (new color field,
  fallback to `accent`).
- The description (e.g., `window management`) is rendered in `whichkey_fg`.

### Data model

```rust
// In ruster-render/src/lib.rs

pub struct WhichKeyEntry {
    pub key: String,
    pub desc: String,
}

pub struct WhichKeyView {
    pub title: String,
    pub rows: Vec<WhichKeyEntry>,
    pub anim: f32,
}
```

### Color field

Add to `Theme`:

| Field | Type | Default | Purpose |
|-------|------|---------|---------|
| `whichkey_key` | `Color` | `accent` | Key-letter highlight in which-key / completions |

Full fallback chain:
- `WhichKey::default()` → uses `self.accent` for `whichkey_key`
- `load_theme()` in `runtime.rs`:
  ```rust
  whichkey_key: get("whichkey_key", d.accent),
  ```
- `ColorOverrides` in `config.rs`:
  ```rust
  whichkey_key: Option<String>,
  ```

### Renderer changes

**ruster-tui/src/widgets.rs — WhichKeyWidget:**
- Iterate `WhichKeyEntry` items:
  - Draw `entry.key` in `whichkey_key` color
  - Draw `"  "` separator
  - Draw `entry.desc` in `whichkey_fg`

**ruster-render-raylib/src/lib.rs:**
- Same split rendering for whichkey entries.

**ruster-tui/src/app.rs — `leader_whichkey()`:**
- Build `WhichKeyEntry` objects instead of format strings:
  ```rust
  let rows = children
      .iter()
      .map(|(k, node)| {
          let desc = match node {
              LeaderNode::Group(d, _) => format!("+{}", d),
              LeaderNode::Action(d, _) => d.to_string(),
          };
          WhichKeyEntry { key: k.to_string(), desc }
      })
      .collect();
  ```

---

## 4. Display Labels

### Current schema labels

| Key | Current label | Issue |
|-----|--------------|-------|
| `whichkey_bg` | `WhichKey background` | `WhichKey` not hyphenated |
| `whichkey_fg` | `WhichKey foreground` | `WhichKey` not hyphenated |
| `cmdline_bg` | `Cmdline background` | Inconsistent casing |
| `cmdline_fg` | `Cmdline foreground` | Inconsistent casing |
| `cmdline_accent` | `Cmdline accent` | Inconsistent casing |

### New labels

| Key | New label |
|-----|-----------|
| `whichkey_bg` | `Which-key bg` |
| `whichkey_fg` | `Which-key fg` |
| `whichkey_key` | `Which-key key` |
| `cmdline_bg` | `Cmdline bg` |
| `cmdline_fg` | `Cmdline fg` |
| `cmdline_accent` | `Cmdline accent` |

The short `bg`/`fg` suffix matches the existing base fields (`bg`, `fg`,
`gutter_bg`, `gutter_fg`, etc.).

---

## Files affected

| File | Change |
|------|--------|
| `ruster-render/src/lib.rs` | Add `WhichKeyEntry` struct, add `whichkey_key` to Theme |
| `ruster-lua/src/schema.rs` | Add `colors.whichkey_key` schema entry, update display labels |
| `ruster-lua/src/config.rs` | Add `whichkey_key` to ThemeColors, ColorOverrides, to_settings, builtin themes |
| `ruster-lua/src/runtime.rs` | Parse `whichkey_key` in `load_theme()` |
| `ruster-tui/src/app.rs` | Remove M-x binding. Tab opens completions panel. Leader/g backspace handling. `leader_whichkey()` returns WhichKeyEntry. |
| `ruster-tui/src/widgets.rs` | New `CmdlineCompletionsWidget`. Update WhichKeyWidget for WhichKeyEntry |
| `ruster-tui/src/renderer.rs` | Draw completions panel above cmdline. Lift cmdline. |
| `ruster-render-raylib/src/lib.rs` | Update whichkey rendering for accent-key, add completions panel |
| `docs/config-reference.md` | Update theme example, add `whichkey_key` |

## Order of implementation

1. **WhichKeyEntry + key accent** — simplest structural change, unlocks the new
   rendering for both which-key and completions.
2. **Back navigation** — self-contained leader/g handler changes.
3. **Display labels** — trivial schema string changes.
4. **Cmdline completions panel** — most involved, depends on WhichKeyEntry
   existing.
