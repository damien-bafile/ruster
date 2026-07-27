# Cmdline & Which-Key UX Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Revamp the cmdline tab-completion, which-key navigation, key-letter accenting, and color field display labels.

**Architecture:** A new `WhichKeyEntry` struct replaces flat strings in `WhichKeyView::rows`, enabling per-key accent rendering. A new `CmdlineCompletions` state + widget replaces the floating PickerState for Tab-in-cmdline. Leader/g-menu handlers gain Backspace-back behavior. Schema display labels are normalized.

**Tech Stack:** Rust, crossterm, ratatui, raylib, mlua

## Global Constraints

- All new color fields follow the existing fallback chain: Theme::default() → load_theme() → ColorOverrides
- The M-x (Alt+x) keybinding is removed entirely
- Backspace in which-key pops one level; empty sequence cancels leader mode
- Cmdline completions panel reuses whichkey_* theme colors
- Every theme/lua builtin theme gets the new `whichkey_key` field

---

### Task 1: WhichKeyEntry + key accent (data model + renderers)

**Files:**
- Modify: `crates/ruster-render/src/lib.rs` (WhichKeyView struct, Theme struct)
- Modify: `crates/ruster-tui/src/app.rs` (`leader_whichkey()` builder)
- Modify: `crates/ruster-tui/src/widgets.rs` (WhichKeyWidget render)
- Modify: `crates/ruster-render-raylib/src/lib.rs` (whichkey render)
- Modify: `crates/ruster-lua/src/schema.rs` (add `whichkey_key` SettingSpec)
- Modify: `crates/ruster-lua/src/config.rs` (add field to ThemeColors, ColorOverrides, to_settings, builtin themes)
- Modify: `crates/ruster-lua/src/runtime.rs` (parse new field)
- Modify: `crates/ruster-tui/src/app.rs` (resolve_theme_colors, gui_config)

- [ ] **Step 1: Add `WhichKeyEntry` struct and update `WhichKeyView`**

In `crates/ruster-render/src/lib.rs`, add the new struct and update `WhichKeyView`:

```rust
// Before the WhichKeyView struct
pub struct WhichKeyEntry {
    pub key: String,
    pub desc: String,
}

// Update WhichKeyView to use WhichKeyEntry
pub struct WhichKeyView {
    pub title: String,
    pub rows: Vec<WhichKeyEntry>,
    pub anim: f32,
}
```

- [ ] **Step 2: Add `whichkey_key` to Theme**

In `crates/ruster-render/src/lib.rs`, add the field to Theme:

```rust
pub struct Theme {
    // ...existing fields...
    pub whichkey_key: Color,
}
```

In `impl Theme::default()`:

```rust
whichkey_key: Color::Rgb(248, 189, 150),  // warm accent
```

- [ ] **Step 3: Add schema entry for `whichkey_key`**

In `crates/ruster-lua/src/schema.rs`, add after the `whichkey_fg` entry:

```rust
add("colors", "whichkey_key", "Which-key key", Text, t(""), "Key-letter highlight in which-key / completions");
```

- [ ] **Step 4: Add field to config types**

In `crates/ruster-lua/src/config.rs`:

In `ThemeColors`:
```rust
pub whichkey_key: String,
```

In `impl Default for ThemeColors`:
```rust
whichkey_key: String::new(),
```

In `ColorOverrides`:
```rust
pub whichkey_key: Option<String>,
```

In `to_settings()`:
```rust
whichkey_key: self.whichkey_key.clone(),
```

In `from_settings()`:
```rust
whichkey_key: s.get("whichkey_key").cloned().unwrap_or_default(),
```

In `to_lua()`:
```rust
if !self.whichkey_key.is_empty() {
    map.push(("whichkey_key".to_string(), Value::String(self.whichkey_key.clone())));
}
```

In each builtin theme (default, gruvbox, tokyonight, nord, catppuccin-mocha, starship), add:
```rust
whichkey_key: s.whichkey_key.clone().or_else(|| s.accent.clone()),
```

Actually, the builtin themes should just set it from the accent field. Let me think about this — `whichkey_key` gets an explicitly configured value or falls back to `accent`. In the builtin theme builders, we can set:
```rust
whichkey_key: "#f38ba8".to_string(),  // same as accent for catppuccin
```

For starship:
```rust
whichkey_key: "#ff8800".to_string(),  // same as accent
```

Actually, looking at the existing pattern more carefully — the builtin themes in `config.rs` return `BTreeMap<String, ThemeColors>`. The `ThemeColors` struct has all the fields. Let me just set `whichkey_key` to the same value as `accent` in each builtin theme builder, or default to `""` to let the runtime resolve it.

Looking at how the runtime resolves colors in `runtime.rs`:

```rust
whichkey_key: get("whichkey_key", d.accent),
```

So if the theme file doesn't set `whichkey_key`, it falls back to `accent`. The builtin themes just need to include it or not. Let me omit it from builtin themes (empty string) since they already have an `accent` set and the fallback will work.

In `to_lua()`, skip empty:
```rust
if !self.whichkey_key.is_empty() {
    map.push(("whichkey_key".to_string(), Value::String(self.whichkey_key.clone())));
}
```

This way builtin themes don't need to set it.

- [ ] **Step 5: Parse in runtime.rs**

In `crates/ruster-lua/src/runtime.rs`, in the `roles` function:

```rust
whichkey_key: get("whichkey_key", d.accent),
```

- [ ] **Step 6: Wire in app.rs resolution + gui_config**

In `crates/ruster-tui/src/app.rs`, in `resolve_theme_colors()`:

```rust
set(&ov.whichkey_key, &mut colors.whichkey_key);
```

In `gui_config()`:

```rust
whichkey_key: col(c.colors.whichkey_key),
```

- [ ] **Step 7: Update `leader_whichkey()` builder**

In `crates/ruster-tui/src/app.rs`, change `leader_whichkey()` to return `WhichKeyEntry`:

```rust
fn leader_whichkey(seq: &[char]) -> Option<(String, Vec<WhichKeyEntry>)> {
    let children = leader_children(seq)?;
    let mut title = String::from("SPC");
    for c in seq {
        title.push(' ');
        title.push(*c);
    }
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
    Some((title, rows))
}
```

Update the call site where `leader_whichkey()` result is assigned to `state.whichkey` — it needs a `ruster_render::WhichKeyEntry` import.

- [ ] **Step 8: Update WhichKeyWidget in widgets.rs**

```rust
pub struct WhichKeyWidget {
    view: ruster_render::WhichKeyView,
    fg: Color,
    bg: Color,
    key: Color,
}

impl WhichKeyWidget {
    pub fn new(view: ruster_render::WhichKeyView) -> Self {
        WhichKeyWidget {
            view,
            fg: Color::Rgb(205, 214, 244),
            bg: Color::Rgb(69, 71, 90),
            key: Color::Rgb(255, 136, 0),
        }
    }

    pub fn with_theme(mut self, theme: &ruster_render::Theme) -> Self {
        self.fg = ruster_render_color_to_tui(&theme.whichkey_fg);
        self.bg = ruster_render_color_to_tui(&theme.whichkey_bg);
        self.key = ruster_render_color_to_tui(&theme.whichkey_key);
        self
    }
}

impl Widget for WhichKeyWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Fill background (same as before)
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(' ');
                    cell.set_bg(self.bg);
                }
            }
        }
        let put = |buf: &mut Buffer, x: u16, y: u16, ch: char, color: Color, cb: Color| {
            if x < area.right() && y < area.bottom() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    cell.set_fg(color);
                    cell.set_bg(cb);
                }
            }
        };

        for (row, entry) in self.view.rows.iter().enumerate() {
            let y = area.y + row as u16;
            if y >= area.bottom() { break; }
            // Key letter in accent color
            for (i, ch) in entry.key.chars().enumerate() {
                put(buf, area.x + i as u16 + 2, y, ch, self.key, self.bg);
            }
            // Description in foreground color
            let desc_x = area.x + 2 + entry.key.len() as u16 + 2;
            for (i, ch) in entry.desc.chars().enumerate() {
                put(buf, desc_x + i as u16, y, ch, self.fg, self.bg);
            }
        }
    }
}
```

And update the `render` in `renderer.rs`:

In `renderer.rs`, the imports need to be updated. The whichkey rendering in renderer.rs pushes a `WhichKeyWidget::new(wk.clone())` — this should still work since the struct signature hasn't changed.

- [ ] **Step 9: Update raylib whichkey rendering**

In `crates/ruster-render-raylib/src/lib.rs`, update the whichkey panel draw:

```rust
if let Some(wk) = &state.whichkey {
    let row_h = (wk.rows.len() as i32).max(1) * line_h;
    let panel_top = screen_h - (row_h as f32 * wk.anim.clamp(0.0, 1.0)) as i32;
    let mut s = d.begin_scissor_mode(0, panel_top, screen_w, screen_h - panel_top);
    s.draw_rectangle(0, panel_top, screen_w, screen_h - panel_top, whichkey_bg);
    for (i, entry) in wk.rows.iter().enumerate() {
        let ry = panel_top + i as i32 * line_h;
        // Draw key letter in whichkey_key color
        s.draw_text_ex(font, &format!("   {}", entry.key), Vector2::new(pad_x as f32, ry as f32), font_size as f32, 1.0, whichkey_key);
        // Draw description in whichkey_fg color
        let key_w = measure(&format!("  {}", entry.key)) as i32;
        s.draw_text_ex(font, &entry.desc, Vector2::new((pad_x + key_w) as f32, ry as f32), font_size as f32, 1.0, whichkey_fg);
    }
}
```

Add `whichkey_key` to the resolved colors near the top:
```rust
let whichkey_key = to_raylib(theme.whichkey_key, accent);
```

- [ ] **Step 10: Update `measure()` to be accessible if not already**

The raylib renderer likely has a `measure()` closure. Check that it's accessible within the whichkey block. If it's defined outside, it'll be available. Let me check by reading the file. Actually I already read this — let me verify the `measure` closure is defined in scope.

The `measure` is defined in `begin_drawing` block? Let me check. From the earlier exploration, the raylib lib.rs has code that sets up `measure = |s: &str| -> f32 { ... }`. It should be available throughout the drawing block.

- [ ] **Step 11: Build and test**

```bash
cargo build 2>&1
cargo test 2>&1
```

- [ ] **Step 12: Commit**

```bash
git add -A && git commit -m "feat: WhichKeyEntry struct with key accent color (whichkey_key)"
```

---

### Task 2: Which-Key Back Navigation

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` (`handle_leader_key`, `handle_g_key`)

- [ ] **Step 1: Update `handle_leader_key` for Backspace**

In `crates/ruster-tui/src/app.rs`, add Backspace handling in `handle_leader_key`:

```rust
fn handle_leader_key(&mut self, ck: crossterm::event::KeyEvent) {
    let c = match ck.code {
        KeyCode::Char(c) => c,
        KeyCode::Backspace => {
            if let Some(seq) = &mut self.leader_pending {
                seq.pop();
                if seq.is_empty() {
                    self.leader_pending = None;
                }
                // Re-resolve happens on next render via `update_whichkey`
            }
            return;
        }
        _ => {
            self.leader_pending = None;
            return;
        }
    };
    // ...rest of existing code unchanged...
}
```

- [ ] **Step 2: Update `handle_g_key` for Backspace**

Find the `handle_g_key` method and add Backspace handling. The `g` menu currently uses a `g_pending` field. Let me check what field name it uses. From the exploration, it's in `app.rs` around line 4063-4074. Let me add Backspace handling there.

The g_menu system likely also has a `g_pending` or similar field. From the earlier code:

```rust
fn handle_g_key(&mut self, ck: crossterm::event::KeyEvent) {
    match ck.code {
        KeyCode::Char('d') => self.lsp_definition(),
        KeyCode::Char('r') => self.lsp_references(),
        KeyCode::Char('h') => self.lsp_hover(),
        KeyCode::Esc => {} // cancel
        other => {
            self.feed_key_to_vim(KeyCode::Char('g'));
            self.feed_key_to_vim(other);
        }
    }
}
```

This is a single-key dispatch (not multi-level like leader). For `g`, Backspace should cancel (same as Esc):
```rust
KeyCode::Backspace => {} // cancel
```

This is simpler since `g` has no multi-level sub-menus.

- [ ] **Step 3: Build and test**

```bash
cargo build 2>&1
cargo test 2>&1
```

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat: which-key backspace pops leader sequence; backspace in g menu cancels"
```

---

### Task 3: Display Labels

**Files:**
- Modify: `crates/ruster-lua/src/schema.rs` (update display labels)

- [ ] **Step 1: Update schema display labels**

In `crates/ruster-lua/src/schema.rs`, find and update these entries:

```rust
// Change:
add("colors", "whichkey_bg", "WhichKey background", Text, t(""), "...");
// To:
add("colors", "whichkey_bg", "Which-key bg", Text, t(""), "...");

// Change:
add("colors", "whichkey_fg", "WhichKey foreground", Text, t(""), "...");
// To:
add("colors", "whichkey_fg", "Which-key fg", Text, t(""), "...");

// Change:
add("colors", "cmdline_bg", "Cmdline background", Text, t(""), "...");
// To:
add("colors", "cmdline_bg", "Cmdline bg", Text, t(""), "...");

// Change:
add("colors", "cmdline_fg", "Cmdline foreground", Text, t(""), "...");
// To:
add("colors", "cmdline_fg", "Cmdline fg", Text, t(""), "...");

// Change:
add("colors", "cmdline_accent", "Cmdline accent", Text, t(""), "...");
// To:
add("colors", "cmdline_accent", "Cmdline accent", Text, t(""), "...");
// (No change — already correct)

// The new whichkey_key entry:
add("colors", "whichkey_key", "Which-key key", Text, t(""), "Key-letter highlight in which-key / completions");
```

- [ ] **Step 2: Build and test**

```bash
cargo build 2>&1
cargo test 2>&1
```

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "chore: normalize color display labels in schema"
```

---

### Task 4: Cmdline Completions Panel

**Files:**
- Modify: `crates/ruster-render/src/lib.rs` (CmdlineCompletions state)
- Modify: `crates/ruster-tui/src/app.rs` (remove M-x, Tab → completions panel, key handling)
- Modify: `crates/ruster-tui/src/widgets.rs` (new CmdlineCompletionsWidget)
- Modify: `crates/ruster-tui/src/renderer.rs` (draw completions panel above cmdline)
- Modify: `crates/ruster-render-raylib/src/lib.rs` (draw completions panel)
- Modify: `docs/config-reference.md` (document whichkey_key)

- [ ] **Step 1: Add CmdlineCompletions state + CmdlineCompletionItem**

In `crates/ruster-render/src/lib.rs`, add to FrameState:

```rust
#[derive(Clone)]
pub struct CmdlineCompletionItem {
    pub key: String,
    pub desc: String,
}

/// State for the cmdline completions panel.
#[derive(Clone)]
pub struct CmdlineCompletions {
    /// All available completions (pre-filtered pool).
    pub items: Vec<CmdlineCompletionItem>,
    /// Currently visible (filtered) rows.
    pub rows: Vec<usize>,
    /// Selected index within `rows`.
    pub selected: usize,
    /// Whether the panel is visible.
    pub visible: bool,
}

impl CmdlineCompletions {
    pub fn new(items: Vec<CmdlineCompletionItem>) -> Self {
        let count = items.len();
        CmdlineCompletions {
            items,
            rows: (0..count).collect(),
            selected: 0,
            visible: false,
        }
    }

    pub fn filter(&mut self, query: &str) {
        // TODO: fuzzy filter using the existing nucleo_matcher, or a simpler
        // prefix match for now. After discussion, we'll use the same
        // nucleo_matcher that PickerState uses.
        if query.is_empty() {
            self.rows = (0..self.items.len()).collect();
        } else {
            // Simple prefix/case-insensitive contains filter
            let q = query.to_lowercase();
            self.rows = self.items
                .iter()
                .enumerate()
                .filter(|(_, item)| item.key.to_lowercase().contains(&q) || item.desc.to_lowercase().contains(&q))
                .map(|(i, _)| i)
                .collect();
        }
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
    }
}
```

Add `pub cmdline_completions: Option<CmdlineCompletions>` to `FrameState`.

- [ ] **Step 2: Remove M-x binding**

In `crates/ruster-tui/src/app.rs`, find the Emacs `M-x` handler (around line 1689):

```rust
// Remove this block entirely:
// KeyEvent::Alt('x') => {
//     // M-x: run a command via the palette.
//     self.open_command_picker("");
//     return;
// }
```

- [ ] **Step 3: Build the completions pool from PALETTE_COMMANDS**

Add a method or constant to build `CmdlineCompletionItem` vec from `PALETTE_COMMANDS`. In `app.rs`:

```rust
fn build_cmdline_completions() -> Vec<ruster_render::CmdlineCompletionItem> {
    PALETTE_COMMANDS
        .iter()
        .map(|(name, desc)| ruster_render::CmdlineCompletionItem {
            key: name.to_string(),
            desc: desc.to_string(),
        })
        .collect()
}
```

- [ ] **Step 4: Change Tab handler in cmdline mode**

Find the Tab handler (~line 1536) and change it:

```rust
// Tab in the cmdline toggles the completions panel.
if self.vim.mode == VimMode::Cmdline && key == KeyEvent::Tab {
    if let Some(cc) = &mut self.state.cmdline_completions {
        cc.visible = !cc.visible;
        if cc.visible {
            let seed = self.vim.cmdline_buffer().trim_start_matches(':').trim().to_string();
            cc.filter(&seed);
        }
    } else {
        let items = build_cmdline_completions();
        let mut cc = ruster_render::CmdlineCompletions::new(items);
        let seed = self.vim.cmdline_buffer().trim_start_matches(':').trim().to_string();
        cc.filter(&seed);
        cc.visible = true;
        self.state.cmdline_completions = Some(cc);
    }
    return;
}
```

- [ ] **Step 5: Handle Esc, Up, Down, Enter with completions visible**

In the cmdline key handling, add cases after the Tab handler. These should fire only when `cmdline_completions` is `Some` and `visible`:

```rust
// In the cmdline input handling section:
if let Some(cc) = &mut self.state.cmdline_completions {
    if cc.visible {
        match key {
            KeyEvent::Esc => {
                cc.visible = false;
                return;
            }
            KeyEvent::Up => {
                cc.selected = cc.selected.saturating_sub(1);
                return;
            }
            KeyEvent::Down => {
                cc.selected = (cc.selected + 1).min(cc.rows.len().saturating_sub(1));
                return;
            }
            KeyEvent::Enter => {
                if let Some(&idx) = cc.rows.get(cc.selected) {
                    let cmd = cc.items[idx].key.clone();
                    cc.visible = false;
                    // Run the command
                    self.execute_command(&cmd);
                    return;
                }
            }
            _ => {}
        }
    }
}
```

Also, when any char is typed in cmdline mode and completions are visible, re-filter:

```rust
// After handling the char input in cmdline mode:
if let Some(cc) = &mut self.state.cmdline_completions {
    if cc.visible {
        let seed = self.vim.cmdline_buffer().trim_start_matches(':').trim().to_string();
        cc.filter(&seed);
    }
}
```

- [ ] **Step 6: Enter triggers command execution**

When Enter is pressed on a completion, feed the command through the same Vim key mechanism the command palette picker uses. The existing pattern for `PickerAction::RunCmd` feeds chars sequentially:

```rust
// Inside the cmdline_key handler, Enter with completions visible:
KeyEvent::Enter => {
    if let Some(&idx) = cc.rows.get(cc.selected) {
        let cmd = cc.items[idx].key.clone();
        cc.visible = false;
        self.state.cmdline_completions = None;
        // Feed the command as keystrokes through vim (same approach picker uses)
        self.feed_key_to_vim(KeyCode::Char(':'));
        for ch in cmd.chars() {
            self.feed_key_to_vim(KeyCode::Char(ch));
        }
        self.feed_key_to_vim(KeyCode::Enter);
        return;
    }
}
```

Find the existing `PickerAction::RunCmd` handler in app.rs (search for `RunCmd`) to see the exact pattern and replicate it.

- [ ] **Step 7: Create CmdlineCompletionsWidget**

In `crates/ruster-tui/src/widgets.rs`, add a new widget:

```rust
pub struct CmdlineCompletionsWidget {
    view: ruster_render::CmdlineCompletions,
    fg: Color,
    bg: Color,
    key: Color,
    sel_bg: Color,
    sel_fg: Color,
}

impl CmdlineCompletionsWidget {
    pub fn new(view: ruster_render::CmdlineCompletions) -> Self {
        CmdlineCompletionsWidget {
            view,
            fg: Color::Rgb(205, 214, 244),
            bg: Color::Rgb(69, 71, 90),
            key: Color::Rgb(255, 136, 0),
            sel_bg: Color::Rgb(69, 71, 90),
            sel_fg: Color::Rgb(205, 214, 244),
        }
    }

    pub fn with_theme(mut self, theme: &ruster_render::Theme) -> Self {
        self.fg = ruster_render_color_to_tui(&theme.whichkey_fg);
        self.bg = ruster_render_color_to_tui(&theme.whichkey_bg);
        self.key = ruster_render_color_to_tui(&theme.whichkey_key);
        self
    }
}

impl Widget for CmdlineCompletionsWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Fill background
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(' ');
                    cell.set_bg(self.bg);
                }
            }
        }
        let put = |buf: &mut Buffer, x: u16, y: u16, ch: char, color: Color, cb: Color| {
            if x < area.right() && y < area.bottom() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    cell.set_fg(color);
                    cell.set_bg(cb);
                }
            }
        };

        for (row_idx, &item_idx) in self.view.rows.iter().enumerate() {
            let y = area.y + row_idx as u16;
            if y >= area.bottom() { break; }
            let is_selected = row_idx == self.view.selected;
            let (row_bg, row_fg) = if is_selected {
                (self.sel_bg, self.sel_fg)
            } else {
                (self.bg, self.fg)
            };

            // Key in accent color
            let entry = &self.view.items[item_idx];
            for (i, ch) in entry.key.chars().enumerate() {
                put(buf, area.x + i as u16 + 2, y, ch, self.key, row_bg);
            }
            // Description
            let desc_x = area.x + 2 + entry.key.len() as u16 + 2;
            for (i, ch) in entry.desc.chars().enumerate() {
                put(buf, desc_x + i as u16, y, ch, row_fg, row_bg);
            }
        }
    }
}
```

- [ ] **Step 8: Update renderer to draw completions panel**

In `crates/ruster-tui/src/renderer.rs`, after drawing the cmdline, draw the completions panel:

```rust
// Draw cmdline completions panel if visible
if let Some(cc) = &state.cmdline_completions {
    if cc.visible && !cc.rows.is_empty() {
        let panel_h = cc.rows.len().min(10) as u16; // max 10 rows
        let cl_area = Rect::new(0, area.height.saturating_sub(1 + panel_h), area.width, panel_h);
        frame.render_widget(
            crate::widgets::CmdlineCompletionsWidget::new(cc.clone())
                .with_theme(&self.theme),
            cl_area,
        );
    }
}
```

- [ ] **Step 9: Update raylib renderer**

In `crates/ruster-render-raylib/src/lib.rs`, after drawing the cmdline:

```rust
// Cmdline completions panel
if let Some(cc) = &state.cmdline_completions {
    if cc.visible && !cc.rows.is_empty() {
        let max_rows = cc.rows.len().min(10) as i32;
        let panel_h = max_rows * line_h;
        let panel_top = cmd_y - panel_h; // cmd_y is where cmdline is drawn
        d.draw_rectangle(0, panel_top, screen_w, panel_h, whichkey_bg);
        for (row_idx, &item_idx) in cc.rows.iter().enumerate().take(10) {
            let ry = panel_top + row_idx as i32 * line_h;
            let entry = &cc.items[item_idx];
            // Key in whichkey_key
            d.draw_text_ex(font, &format!("   {}", entry.key), Vector2::new(pad_x as f32, ry as f32), font_size as f32, 1.0, whichkey_key);
            // Description in whichkey_fg
            let key_w = measure(&format!("  {}", entry.key)) as i32;
            d.draw_text_ex(font, &entry.desc, Vector2::new((pad_x + key_w) as f32, ry as f32), font_size as f32, 1.0, whichkey_fg);
        }
    }
}
```

Need to save `cmd_y` from the cmdline draw section. Currently the cmdline code computes `cmd_y` — wrap it in a binding that the completions code can reference:

```rust
let cmd_y = pad_y + (rows - 1) * line_h;
```

This variable is already computed. The completions panel should use the same `cmd_y` to determine its top edge.

- [ ] **Step 10: Update docs**

In `docs/config-reference.md`, update the theme example to include `whichkey_key`:

```lua
return { bg = "#1e1e1e", fg = "#cdd6f4", gutter = "#6c7086", gutter_bg = "#1e1e1e",
         selection = "#585b70", selection_fg = "#cdd6f4",
         cursor = "#f5e0dc", cursor_fg = "#1e1e1e",
         divider = "#45475a", statusline_fg = "#cdd6f4",
         accent = "#f38ba8", accent_fg = "#1e1e1e",
         whichkey_bg = "#45475a", whichkey_fg = "#cdd6f4",
         whichkey_key = "#f38ba8",
         cmdline_bg = "#45475a", cmdline_fg = "#cdd6f4", cmdline_accent = "#f38ba8" }
```

Also add a note about the M-x removal and Tab-completions panel.

- [ ] **Step 11: Build and test**

```bash
cargo build 2>&1
cargo test 2>&1
```

- [ ] **Step 12: Commit**

```bash
git add -A && git commit -m "feat: cmdline tab-completions panel (which-key style) replaces floating palette; M-x removed"
```
