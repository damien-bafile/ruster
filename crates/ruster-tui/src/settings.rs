//! State for the in-app Settings page. Built from the schema + current config;
//! edits mutate working values that `:w` serializes back to `config.lua`.
//!
//! The page shows the flat schema rows first, then an expandable **Syntax**
//! section — one row per highlighted language which unfolds into a colour picker
//! per syntax group. A single `selected` cursor indexes a computed list of
//! *visible rows* ([`Row`]) so both kinds share navigation, scrolling and reset.

use std::collections::HashMap;

use ruster_lua::config::{Addr, Config};
use ruster_lua::schema::{self, SettingKind, SettingSpec, SettingValue};
use ruster_render::{ControlKind, SettingRowView, SettingsGroup, SettingsView};

/// A picker option as `(display label, stored value)`. For theme/font/shell the
/// two match (with a sentinel whose value is empty); color rows show a palette
/// color name and store its `#hex`.
type Opt = (String, String);

/// Seed data for the Syntax section: `(language, [(group, default_hex, current_hex)])`.
pub type SyntaxSeed = Vec<(String, Vec<(String, String, String)>)>;

/// One syntax group's colour within a language: `value` is the override hex
/// (`""` = use the built-in `default_hex`).
struct SyntaxGroupRow {
    group: String,
    value: String,
    default_hex: String,
}

/// A highlighted language in the Syntax section, collapsible to hide its groups.
struct SyntaxLang {
    key: String,
    expanded: bool,
    groups: Vec<SyntaxGroupRow>,
}

/// A visible, selectable row: a flat schema setting, a syntax language header,
/// or a syntax group colour under a language.
#[derive(Clone, Copy)]
enum Row {
    Spec(usize),
    SyntaxLang(usize),
    SyntaxGroup(usize, usize),
}

pub struct SettingsState {
    specs: Vec<SettingSpec>,
    /// Working values, parallel to `specs`.
    values: Vec<SettingValue>,
    /// Per-language syntax colour rows (the Syntax section).
    syntax: Vec<SyntaxLang>,
    /// The currently-visible rows; rebuilt when a language folds/unfolds.
    rows: Vec<Row>,
    /// Selected row (index into `rows`).
    selected: usize,
    /// In-edit text buffer for Text/Number fields (`None` = not editing).
    editing: Option<String>,
    /// Live search filter string. Non-`None` means filter mode is active.
    pub filter: Option<String>,
    /// Runtime picker options per spec row (theme/font/shell/color).
    dyn_opts: HashMap<usize, Vec<Opt>>,
    /// Each available theme's palette (name → `(color_name, hex)` pairs), used to
    /// (re)build the color-row options for the selected theme.
    theme_palettes: HashMap<String, Vec<Opt>>,
    theme_idx: Option<usize>,
    color_rows: Vec<usize>,
    /// True after a lone `d`, so the next `d` completes a `dd` reset.
    d_pending: bool,
    /// True after a lone `g`, so the next `g` completes a `gg` jump-to-top.
    g_pending: bool,
    pub dirty: bool,
}

impl SettingsState {
    /// `dynamic` supplies `(group, key, options)` for theme/font/shell rows;
    /// `theme_palettes` supplies each theme's palette for the color rows;
    /// `syntax` supplies `(lang, [(group, default_hex, current_hex)])` for the
    /// Syntax section.
    pub fn new(
        config: &Config,
        dynamic: Vec<(&'static str, &'static str, Vec<Opt>)>,
        theme_palettes: Vec<(String, Vec<Opt>)>,
        syntax: SyntaxSeed,
    ) -> Self {
        let specs = schema::schema();
        let mut dyn_opts: HashMap<usize, Vec<Opt>> = HashMap::new();
        for (group, key, opts) in dynamic {
            if opts.is_empty() {
                continue;
            }
            if let Some(i) = specs.iter().position(|s| s.group == group && s.key == key) {
                dyn_opts.insert(i, opts);
            }
        }
        let theme_idx = specs.iter().position(|s| s.group == "general" && s.key == "theme");
        let color_rows: Vec<usize> =
            specs.iter().enumerate().filter(|(_, s)| s.group == "colors").map(|(i, _)| i).collect();
        let current = config.to_settings();
        let values = specs
            .iter()
            .map(|s| {
                current
                    .iter()
                    .find(|((g, k), _)| *g == s.group && *k == s.key)
                    .map(|(_, v)| v.clone())
                    .unwrap_or_else(|| s.default.clone())
            })
            .collect();
        let syntax = syntax
            .into_iter()
            .map(|(key, groups)| SyntaxLang {
                key,
                expanded: false,
                groups: groups
                    .into_iter()
                    .map(|(group, default_hex, value)| SyntaxGroupRow { group, value, default_hex })
                    .collect(),
            })
            .collect();
        let mut st = SettingsState {
            specs,
            values,
            syntax,
            rows: Vec::new(),
            selected: 0,
            editing: None,
            filter: None,
            dyn_opts,
            theme_palettes: theme_palettes.into_iter().collect(),
            theme_idx,
            color_rows,
            d_pending: false,
            g_pending: false,
            dirty: false,
        };
        st.rebuild_color_opts();
        st.rebuild_rows();
        st
    }

    /// Recompute the visible-row list from the schema + (un)folded languages.
    pub fn rebuild_rows(&mut self) {
        let filter = self.filter.as_deref().unwrap_or("");
        let matches = |label: &str, key: &str| {
            filter.is_empty()
                || label.to_lowercase().contains(filter)
                || key.to_lowercase().contains(filter)
        };
        let mut rows: Vec<Row> = (0..self.specs.len())
            .filter(|&i| matches(self.specs[i].label, self.specs[i].key))
            .map(Row::Spec)
            .collect();
        for (li, lang) in self.syntax.iter().enumerate() {
            if rows.is_empty() && !filter.is_empty() { break; }
            if matches(&lang.key, &lang.key) {
                rows.push(Row::SyntaxLang(li));
                if lang.expanded {
                    rows.extend((0..lang.groups.len()).map(|gi| Row::SyntaxGroup(li, gi)));
                }
            }
        }
        self.rows = rows;
        if self.selected >= self.rows.len() {
            self.selected = self.rows.len().saturating_sub(1);
        }
    }

    /// Rebuild the color rows' options from the currently-selected theme's
    /// palette. Called on open and whenever the theme row changes.
    fn rebuild_color_opts(&mut self) {
        let opts = self.palette_opts("theme");
        for i in self.color_rows.clone() {
            self.dyn_opts.insert(i, opts.clone());
        }
    }

    /// The current theme's palette as picker options, prefixed with an
    /// "unset → use default" sentinel labelled `unset_label` (empty value).
    fn palette_opts(&self, unset_label: &str) -> Vec<Opt> {
        let theme = self.theme_idx.map(|i| self.values[i].display()).unwrap_or_default();
        let palette = self.theme_palettes.get(&theme).cloned().unwrap_or_default();
        let mut opts: Vec<Opt> = vec![(unset_label.to_string(), String::new())];
        opts.extend(palette);
        opts
    }

    fn options_for(&self, idx: usize) -> Option<Vec<Opt>> {
        if let Some(opts) = self.dyn_opts.get(&idx) {
            return Some(opts.clone());
        }
        if let SettingKind::Enum(opts) = self.specs[idx].kind {
            return Some(opts.iter().map(|s| (s.to_string(), s.to_string())).collect());
        }
        None
    }

    pub fn is_editing(&self) -> bool {
        self.editing.is_some()
    }

    /// The current working values, addressed by `(group, key)`.
    pub fn values(&self) -> Vec<(Addr, SettingValue)> {
        self.specs.iter().zip(&self.values).map(|(s, v)| ((s.group, s.key), v.clone())).collect()
    }

    /// The edited per-language syntax overrides (`lang -> group -> hex`), only
    /// including groups the user actually set.
    pub fn syntax_overrides(&self) -> HashMap<String, HashMap<String, String>> {
        let mut out = HashMap::new();
        for lang in &self.syntax {
            let mut m = HashMap::new();
            for g in &lang.groups {
                if !g.value.is_empty() {
                    m.insert(g.group.clone(), g.value.clone());
                }
            }
            if !m.is_empty() {
                out.insert(lang.key.clone(), m);
            }
        }
        out
    }

    // --- current row ---

    fn cur(&self) -> Row {
        self.rows.get(self.selected).copied().unwrap_or(Row::Spec(0))
    }

    /// The spec index if the selected row is a flat schema setting.
    fn cur_spec(&self) -> Option<usize> {
        match self.cur() {
            Row::Spec(i) => Some(i),
            _ => None,
        }
    }

    // --- navigation ---

    /// The section a row belongs to, for `Tab`/`[`/`]` group jumps.
    fn section_of(&self, row: Row) -> &str {
        match row {
            Row::Spec(i) => self.specs[i].group,
            Row::SyntaxLang(_) | Row::SyntaxGroup(_, _) => "syntax",
        }
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.rows.len() {
            self.selected += 1;
        }
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// `G` — jump to the last row.
    pub fn move_to_bottom(&mut self) {
        self.selected = self.rows.len().saturating_sub(1);
    }

    /// `gg` (two presses) jumps to the first row; the first `g` arms it.
    /// Returns true when the jump actually fired.
    pub fn press_g(&mut self) -> bool {
        if self.g_pending {
            self.g_pending = false;
            self.selected = 0;
            true
        } else {
            self.g_pending = true;
            false
        }
    }

    /// Clear a half-typed `gg` when any other key is pressed.
    pub fn cancel_g(&mut self) {
        self.g_pending = false;
    }

    pub fn next_group(&mut self) {
        let cur = self.section_of(self.cur()).to_string();
        if let Some(i) =
            (self.selected + 1..self.rows.len()).find(|&i| self.section_of(self.rows[i]) != cur)
        {
            self.selected = i;
        }
    }

    pub fn prev_group(&mut self) {
        let cur = self.section_of(self.cur()).to_string();
        // Walk back to a different section, then to that section's first row.
        let prev = (0..self.selected).rev().find(|&i| self.section_of(self.rows[i]) != cur);
        match prev {
            Some(j) => {
                let target = self.section_of(self.rows[j]).to_string();
                self.selected =
                    (0..=j).find(|&i| self.section_of(self.rows[i]) == target).unwrap_or(j);
            }
            None => self.selected = 0,
        }
    }

    // --- activation / adjustment ---

    /// Space/Enter: toggle a bool, cycle a picker, unfold a language, else edit.
    pub fn activate(&mut self) {
        match self.cur() {
            Row::Spec(i) => {
                if matches!(self.specs[i].kind, SettingKind::Bool) {
                    self.flip(i);
                } else if self.options_for(i).is_some() {
                    self.cycle_spec(i, 1);
                } else {
                    self.editing = Some(self.values[i].display());
                }
            }
            Row::SyntaxLang(li) => {
                self.syntax[li].expanded = !self.syntax[li].expanded;
                self.rebuild_rows();
            }
            Row::SyntaxGroup(li, gi) => self.cycle_syntax(li, gi, 1),
        }
    }

    /// h/l or Left/Right: cycle a picker, step a number, fold/unfold a language.
    pub fn adjust(&mut self, delta: i64) {
        match self.cur() {
            Row::Spec(i) => self.adjust_spec(i, delta),
            Row::SyntaxLang(li) => {
                let expand = delta > 0;
                if self.syntax[li].expanded != expand {
                    self.syntax[li].expanded = expand;
                    self.rebuild_rows();
                }
            }
            Row::SyntaxGroup(li, gi) => self.cycle_syntax(li, gi, delta),
        }
    }

    fn adjust_spec(&mut self, i: usize, delta: i64) {
        if self.options_for(i).is_some() {
            self.cycle_spec(i, delta);
            return;
        }
        match self.specs[i].kind {
            SettingKind::Bool => self.flip(i),
            SettingKind::Int { min, max } => {
                if let SettingValue::Int(v) = self.values[i] {
                    self.set(i, SettingValue::Int((v + delta).clamp(min, max)));
                }
            }
            SettingKind::Float { min, max } => {
                if let SettingValue::Float(f) = self.values[i] {
                    self.set(i, SettingValue::Float((f + delta as f64).clamp(min, max)));
                }
            }
            _ => {}
        }
    }

    fn flip(&mut self, i: usize) {
        if let SettingValue::Bool(b) = self.values[i] {
            self.set(i, SettingValue::Bool(!b));
        }
    }

    fn cycle_spec(&mut self, i: usize, delta: i64) {
        let Some(opts) = self.options_for(i) else { return };
        if opts.is_empty() {
            return;
        }
        let cur = self.values[i].display();
        let idx = opts.iter().position(|(_, v)| *v == cur).unwrap_or(0) as i64;
        let n = opts.len() as i64;
        let next = (((idx + delta) % n) + n) % n;
        let value = opts[next as usize].1.clone();
        let v = if matches!(self.specs[i].kind, SettingKind::Enum(_)) {
            SettingValue::Enum(value)
        } else {
            SettingValue::Text(value)
        };
        self.set(i, v);
    }

    /// Cycle a syntax group's colour through the selected theme's palette
    /// (with a leading "default" that clears the override).
    fn cycle_syntax(&mut self, li: usize, gi: usize, delta: i64) {
        let opts = self.palette_opts("default");
        if opts.is_empty() {
            return;
        }
        let cur = self.syntax[li].groups[gi].value.clone();
        let idx = opts.iter().position(|(_, v)| *v == cur).unwrap_or(0) as i64;
        let n = opts.len() as i64;
        let next = (((idx + delta) % n) + n) % n;
        let value = opts[next as usize].1.clone();
        if self.syntax[li].groups[gi].value != value {
            self.syntax[li].groups[gi].value = value;
            self.dirty = true;
        }
    }

    fn set(&mut self, i: usize, v: SettingValue) {
        if self.values[i] != v {
            self.values[i] = v;
            self.dirty = true;
            // Switching theme repopulates the color pickers with its palette.
            if Some(i) == self.theme_idx {
                self.rebuild_color_opts();
            }
        }
    }

    /// Reset the selected row to its default. Flat rows use the schema default
    /// (for colours, `""` = use the theme); syntax group rows clear the override.
    /// Returns true if anything changed.
    pub fn reset_selected(&mut self) -> bool {
        match self.cur() {
            Row::Spec(i) => {
                let def = self.specs[i].default.clone();
                let changed = self.values[i] != def;
                self.set(i, def);
                changed
            }
            Row::SyntaxGroup(li, gi) => {
                let changed = !self.syntax[li].groups[gi].value.is_empty();
                if changed {
                    self.syntax[li].groups[gi].value.clear();
                    self.dirty = true;
                }
                changed
            }
            Row::SyntaxLang(_) => false,
        }
    }

    /// `d` in the (non-editing) list: the first arms a `dd`, the second resets.
    /// Returns true when a reset actually fired.
    pub fn press_d(&mut self) -> bool {
        if self.d_pending {
            self.d_pending = false;
            self.reset_selected()
        } else {
            self.d_pending = true;
            false
        }
    }

    /// Clear a half-typed `dd` when any other key is pressed.
    pub fn cancel_d(&mut self) {
        self.d_pending = false;
    }

    // --- text editing ---

    pub fn edit_push(&mut self, c: char) {
        if let Some(buf) = &mut self.editing {
            buf.push(c);
        }
    }

    pub fn edit_backspace(&mut self) {
        if let Some(buf) = &mut self.editing {
            buf.pop();
        }
    }

    pub fn edit_cancel(&mut self) {
        self.editing = None;
    }

    /// Commit the edit buffer if it parses and validates; otherwise discard.
    pub fn edit_commit(&mut self) {
        let Some(buf) = self.editing.take() else { return };
        let Some(i) = self.cur_spec() else { return };
        let spec = &self.specs[i];
        if let Some(v) = parse_value(&spec.kind, buf.trim()) {
            if spec.kind.check(&v).is_ok() {
                self.set(i, v);
            }
        }
    }

    // --- view ---

    /// The palette colour name for a stored hex, or the hex itself if it isn't a
    /// named palette colour.
    fn color_label(&self, hex: &str) -> String {
        let theme = self.theme_idx.map(|i| self.values[i].display()).unwrap_or_default();
        self.theme_palettes
            .get(&theme)
            .and_then(|p| p.iter().find(|(_, v)| v == hex).map(|(l, _)| l.clone()))
            .unwrap_or_else(|| hex.to_string())
    }

    pub fn view(&self) -> SettingsView {
        let mut groups: Vec<SettingsGroup> = Vec::new();
        let push_row = |groups: &mut Vec<SettingsGroup>, section: String, row: SettingRowView| {
            if groups.last().map(|g| g.name != section).unwrap_or(true) {
                groups.push(SettingsGroup { name: section.clone(), rows: Vec::new() });
            }
            groups.last_mut().expect("group pushed").rows.push(row);
        };

        for (ri, row) in self.rows.iter().enumerate() {
            let selected = self.selected == ri;
            match *row {
                Row::Spec(i) => {
                    let spec = &self.specs[i];
                    let dyn_row = self.dyn_opts.contains_key(&i);
                    let kind = match &spec.kind {
                        SettingKind::Bool => ControlKind::Toggle,
                        SettingKind::Enum(_) => ControlKind::Enum,
                        SettingKind::Int { .. } | SettingKind::Float { .. } => ControlKind::Number,
                        SettingKind::Text | SettingKind::Color => {
                            if dyn_row { ControlKind::Enum } else { ControlKind::Text }
                        }
                    };
                    let editing = selected && self.editing.is_some();
                    let stored = self.values[i].display();
                    let value = if editing {
                        self.editing.clone().unwrap_or_default()
                    } else if let Some(opts) = self.dyn_opts.get(&i) {
                        opts.iter()
                            .find(|(_, v)| *v == stored)
                            .map(|(l, _)| l.clone())
                            .unwrap_or_else(|| stored.clone())
                    } else {
                        decorate_value(spec.group, spec.key, stored.clone())
                    };
                    let swatch = if spec.group == "colors" && is_hex(&stored) {
                        Some(stored.clone())
                    } else {
                        None
                    };
                    push_row(&mut groups, group_title(spec.group), SettingRowView {
                        label: spec.label.to_string(),
                        kind,
                        value,
                        editing,
                        selected,
                        help: spec.help.to_string(),
                        swatch,
                    });
                }
                Row::SyntaxLang(li) => {
                    let lang = &self.syntax[li];
                    let caret = if lang.expanded { "▾" } else { "▸" };
                    push_row(&mut groups, "Syntax".to_string(), SettingRowView {
                        label: format!("{caret} {}", lang.key),
                        kind: ControlKind::Text,
                        value: String::new(),
                        editing: false,
                        selected,
                        help: format!("Enter: expand/collapse · syntax colours for {}", lang.key),
                        swatch: None,
                    });
                }
                Row::SyntaxGroup(li, gi) => {
                    let g = &self.syntax[li].groups[gi];
                    let (value, hex) = if g.value.is_empty() {
                        ("default".to_string(), g.default_hex.clone())
                    } else {
                        (self.color_label(&g.value), g.value.clone())
                    };
                    push_row(&mut groups, "Syntax".to_string(), SettingRowView {
                        label: format!("    {}", g.group),
                        kind: ControlKind::Enum,
                        value,
                        editing: false,
                        selected,
                        help: format!("{} colour ({})", g.group, self.syntax[li].key),
                        swatch: is_hex(&hex).then_some(hex),
                    });
                }
            }
        }

        let footer = if self.editing.is_some() {
            "type value · Enter commit · Esc cancel".to_string()
        } else if let Some(ref f) = self.filter {
            let num = self.rows.len();
            format!("/{f} · {num} rows · Esc clear")
        } else {
            "j/k move · gg/G top/bottom · Tab group · / filter · Space toggle/cycle · h/l adjust · Enter edit/expand · dd reset · :w save · q close".to_string()
        };
        SettingsView { groups, dirty: self.dirty, footer }
    }
}

fn is_hex(s: &str) -> bool {
    s.len() == 7 && s.as_bytes()[0] == b'#' && s[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

fn parse_value(kind: &SettingKind, s: &str) -> Option<SettingValue> {
    match kind {
        SettingKind::Int { .. } => s.parse::<i64>().ok().map(SettingValue::Int),
        SettingKind::Float { .. } => s.parse::<f64>().ok().map(SettingValue::Float),
        SettingKind::Text => Some(SettingValue::Text(s.to_string())),
        SettingKind::Color => Some(SettingValue::Color(s.to_string())),
        SettingKind::Bool => match s {
            "on" | "true" => Some(SettingValue::Bool(true)),
            "off" | "false" => Some(SettingValue::Bool(false)),
            _ => None,
        },
        SettingKind::Enum(opts) => opts.contains(&s).then(|| SettingValue::Enum(s.to_string())),
    }
}

/// Add a friendly suffix to certain enum values for display only.
fn decorate_value(group: &str, key: &str, value: String) -> String {
    if group == "general" && key == "line_ending" {
        return match value.as_str() {
            "lf" => "lf (unix)".to_string(),
            "crlf" => "crlf (windows)".to_string(),
            _ => value,
        };
    }
    value
}

/// A display title for a group id.
fn group_title(group: &str) -> String {
    match group {
        "general" => "General",
        "gui" => "GUI",
        "gutter" => "Gutter",
        "whichkey" => "Which-key",
        "lsp" => "LSP",
        "terminal" => "Terminal",
        "dired" => "Dired",
        "colors" => "Colors",
        other => other,
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> SettingsState {
        state_with_syntax(Vec::new())
    }

    fn state_with_syntax(syntax: SyntaxSeed) -> SettingsState {
        // Named after whatever `Config::default()` actually selects: the colour
        // rows are built from the *selected* theme's palette, so a fixture that
        // hard-codes a theme name goes quietly empty when the default changes.
        let selected = Config::default().theme;
        let palettes = vec![
            (selected.clone(), vec![("mauve".to_string(), "#cba6f7".to_string())]),
            (
                "gruvbox".to_string(),
                vec![
                    ("orange".to_string(), "#fe8019".to_string()),
                    ("yellow".to_string(), "#fabd2f".to_string()),
                ],
            ),
        ];
        let dynamic = vec![(
            "general",
            "theme",
            vec![(selected.clone(), selected), ("gruvbox".to_string(), "gruvbox".to_string())],
        )];
        SettingsState::new(&Config::default(), dynamic, palettes, syntax)
    }

    fn goto(s: &mut SettingsState, group: &str, key: &str) {
        let idx = schema::schema().iter().position(|sp| sp.group == group && sp.key == key).unwrap();
        s.selected = 0;
        for _ in 0..idx {
            s.move_down();
        }
    }

    /// A one-language syntax fixture: rust with keyword/string.
    fn rust_syntax() -> SyntaxSeed {
        vec![(
            "rust".to_string(),
            vec![
                ("keyword".to_string(), "#cba6f7".to_string(), String::new()),
                ("string".to_string(), "#a6e3a1".to_string(), String::new()),
            ],
        )]
    }

    #[test]
    fn color_row_cycles_selected_theme_palette() {
        let mut s = state();
        goto(&mut s, "colors", "accent");
        // Default theme palette: [theme, mauve]. Cycle → mauve (#cba6f7).
        s.activate();
        assert_eq!(s.values().iter().find(|((g, k), _)| *g == "colors" && *k == "accent").unwrap().1,
                   SettingValue::Text("#cba6f7".into()));
    }

    #[test]
    fn changing_theme_rebuilds_color_options_live() {
        let mut s = state();
        goto(&mut s, "general", "theme");
        s.adjust(1); // → gruvbox
        let theme_val =
            s.values().into_iter().find(|((g, k), _)| *g == "general" && *k == "theme").unwrap().1;
        assert_eq!(theme_val, SettingValue::Text("gruvbox".into()));
        goto(&mut s, "colors", "accent");
        s.activate(); // theme → first palette color = orange
        assert_eq!(
            s.values().iter().find(|((g, k), _)| *g == "colors" && *k == "accent").unwrap().1,
            SettingValue::Text("#fe8019".into())
        );
    }

    #[test]
    fn dd_resets_a_color_row_to_theme_default() {
        let mut s = state();
        goto(&mut s, "colors", "accent");
        s.activate(); // pick a palette color (non-default)
        let val = |s: &SettingsState| {
            s.values().into_iter().find(|((g, k), _)| *g == "colors" && *k == "accent").unwrap().1
        };
        assert_ne!(val(&s), SettingValue::Text(String::new()));
        assert!(!s.press_d());
        assert!(s.press_d());
        assert_eq!(val(&s), SettingValue::Text(String::new()));
    }

    #[test]
    fn other_key_cancels_pending_dd() {
        let mut s = state();
        goto(&mut s, "general", "tabstop");
        let before = s.values();
        s.press_d();
        s.cancel_d();
        assert!(!s.press_d());
        assert_eq!(s.values(), before);
    }

    #[test]
    fn reset_selected_restores_int_default() {
        let mut s = state();
        goto(&mut s, "general", "tabstop");
        let def = s.values().into_iter().find(|((g, k), _)| *g == "general" && *k == "tabstop").unwrap().1;
        s.adjust(3);
        assert!(s.reset_selected());
        let now = s.values().into_iter().find(|((g, k), _)| *g == "general" && *k == "tabstop").unwrap().1;
        assert_eq!(now, def);
    }

    #[test]
    fn stored_hex_displays_palette_name_with_swatch() {
        let mut s = state();
        goto(&mut s, "colors", "cursor_bg");
        s.activate(); // pick mauve
        let view = s.view();
        let row = view
            .groups
            .iter()
            .flat_map(|g| &g.rows)
            .find(|r| r.label == "Cursor background")
            .unwrap();
        assert_eq!(row.value, "mauve");
        assert_eq!(row.swatch.as_deref(), Some("#cba6f7"));
    }

    #[test]
    fn gg_and_shift_g_jump_to_top_and_bottom() {
        let mut s = state_with_syntax(rust_syntax());
        let last = s.rows.len() - 1;
        // G jumps to the last row.
        s.move_to_bottom();
        assert_eq!(s.selected, last);
        // A single g only arms; a second g completes the jump to the top.
        assert!(!s.press_g());
        assert!(s.press_g());
        assert_eq!(s.selected, 0);
        // A non-g key cancels a half-typed gg.
        s.move_to_bottom();
        s.press_g(); // arm
        s.cancel_g();
        assert!(!s.press_g(), "fresh first-g must not jump");
        assert_eq!(s.selected, last, "selection unchanged after cancelled gg");
    }

    #[test]
    fn expanding_a_language_reveals_its_group_rows() {
        let mut s = state_with_syntax(rust_syntax());
        // The lang row is the last visible row while collapsed.
        let lang_row = s.rows.len() - 1;
        s.selected = lang_row;
        let collapsed = s.rows.len();
        s.activate(); // expand
        assert_eq!(s.rows.len(), collapsed + 2, "keyword+string rows should appear");
        // The Syntax group is present with a caret + indented group rows.
        let view = s.view();
        let syntax = view.groups.iter().find(|g| g.name == "Syntax").unwrap();
        assert!(syntax.rows.iter().any(|r| r.label.contains("rust")));
        assert!(syntax.rows.iter().any(|r| r.label.trim() == "keyword"));
    }

    #[test]
    fn syntax_group_cycles_palette_and_exports_override() {
        let mut s = state_with_syntax(rust_syntax());
        s.selected = s.rows.len() - 1; // rust lang row
        s.activate(); // expand
        s.move_down(); // first group: keyword
        s.activate(); // default → first palette colour (#cba6f7)
        let ov = s.syntax_overrides();
        assert_eq!(ov.get("rust").and_then(|m| m.get("keyword")).map(String::as_str), Some("#cba6f7"));
        // dd clears it back to default (dropped from the export).
        assert!(!s.press_d());
        assert!(s.press_d());
        assert!(s.syntax_overrides().is_empty());
    }
}
