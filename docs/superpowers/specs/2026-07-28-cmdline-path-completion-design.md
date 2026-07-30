# `:e` Command & Cmdline Path Completion

**Date:** 2026-07-28
**Status:** Approved
**Scope:** New command + path autocompletion in the cmdline

---

## Problem

The welcome screen advertises `:e path/to/file` but the command doesn't exist. Users must use `:Files` (fuzzy picker) to open files by path, which is heavier than a quick `:e src/foo.rs<Tab>`.

## Goal

Implement `:e <path>` / `:edit <path>` with Tab-based path autocompletion in the cmdline, giving users a vim-native file opening experience.

---

## Design

### 1. Command Parsing

**File:** `crates/ruster-tui/src/app.rs` — `parse_cmdline()` (~line 3141)

Add a new `CmdAction` variant:
```rust
CmdAction::OpenFile(String)
```

New match arms in `parse_cmdline()`:
- `":e"` or `":edit"` (bare, no path) → `CmdAction::RunCmd("Files")` (fallback to file picker)
- `:e <path>` or `:edit <path>` → `CmdAction::OpenFile(path)`

In `apply_cmd()`, handle `CmdAction::OpenFile` by resolving the path and calling `open_path()`.

### 2. Path Resolution

New helper function `resolve_path()`:
- Expand `~` via `home::home_dir()`
- Relative paths resolve against the active file's parent directory (falls back to cwd)
- Absolute paths pass through

```rust
fn resolve_path(partial: &str, active_file_dir: Option<&Path>) -> PathBuf {
    let expanded = if partial.starts_with("~/") {
        let home = home::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(&partial[2..])
    } else if partial == "~" {
        home::home_dir().unwrap_or_else(|| PathBuf::from("."))
    } else {
        PathBuf::from(partial)
    };
    if expanded.is_absolute() {
        expanded
    } else {
        let base = active_file_dir.unwrap_or_else(|| Path::new("."));
        base.join(expanded)
    }
}
```

### 3. Completion State Machine

New struct on `App`:
```rust
struct CmdlineCompletion {
    completions: Vec<String>,  // relative paths for display
    index: usize,
    original: String,          // cmdline text before completion started
}
```

Stored as `Option<CmdlineCompletion>` on `App`.

**State transitions:**

| Current State | Event | Next State | Action |
|---|---|---|---|
| Idle | Tab in cmdline (`:e`/`:edit` prefix) | Cycling | Generate candidates, show first |
| Cycling | Tab | Cycling | Advance index (wrapping) |
| Cycling | Shift-Tab | Idle | Open picker with candidates, close cmdline |
| Cycling | Enter | Idle | Accept current candidate, open file |
| Cycling | Esc | Idle | Clear completion, clear cmdline |
| Cycling | any other key | Idle | Clear completion, process key normally |
| Idle | Tab in cmdline (other prefix) | — | Open command palette (existing) |

### 4. Candidate Generation

When Tab is pressed and no completion state exists:

1. Extract partial path after `:e ` / `:edit `
2. Resolve `~`, resolve relative to active file dir
3. Split into `(parent_dir, filename_prefix)`
4. `std::fs::read_dir(parent_dir)` → filter entries starting with `prefix`
5. Sort: directories first (with `/`), then files, alphabetical within each
6. Store as relative paths (preserving the user's original prefix up to the last `/`)

When the current candidate ends with `/` (directory), next Tab press drills into that directory.

### 5. Key Handling

**File:** `crates/ruster-tui/src/app.rs` — `handle_key()` (~line 1685)

Default triggers (configurable via `CmdlineCompletionConfig` in Section 6):
- Cycling trigger: Tab
- Picker trigger: Shift-Tab

```
if [cycling_trigger] && cmdline mode:
    if completion state exists:
        → cycle to next candidate
    else if cmdline starts with ":e " or ":edit ":
        → generate candidates, enter Cycling state
    else:
        → open command palette (existing)

if [picker_trigger] && cmdline mode && completion state:
    → open picker with completion candidates
```

### 6. Settings

**File:** `crates/ruster-lua/src/config.rs`

```rust
pub struct CmdlineCompletionConfig {
    pub trigger: String,        // default: "tab"
    pub picker_trigger: String, // default: "shift-tab"
}
```

Add to `EditorConfig`, with schema entries in `schema.rs`. Settings control which keys trigger cycling vs picker.

### 7. Error Handling

- **No matches:** `set_message("No matches for '<partial>'")`, stay in cmdline
- **Single match:** auto-complete to it on first Tab, show "1 match"
- **Invalid path:** `open_path()` handles missing files with a message
- **`:e` with no file open:** falls back to cwd

---

## Files to Modify

| File | Changes |
|---|---|
| `crates/ruster-tui/src/app.rs` | `CmdAction::OpenFile` variant, `parse_cmdline()` arms, `apply_cmd()` handler, `CmdlineCompletion` struct, Tab/Shift-Tab logic, `resolve_path()` helper |
| `crates/ruster-lua/src/config.rs` | `CmdlineCompletionConfig` struct |
| `crates/ruster-lua/src/schema.rs` | Settings schema entries |
| `crates/ruster-tui/src/widgets.rs` | Update welcome screen hint (cosmetic) |
| `crates/ruster-render-raylib/src/lib.rs` | Update welcome screen hint (cosmetic) |

## Dependencies

- `home` crate — already a dependency (used elsewhere for `~` expansion)
- No new crates needed

## Testing

- Unit test `resolve_path()` with `~`, relative, absolute paths
- Unit test candidate generation with mock directory
- Integration test: `:e<Tab>` cycles, `:e<Tab><Tab>` advances, `:e<Tab><Enter>` opens
- Test bare `:e` falls back to picker
