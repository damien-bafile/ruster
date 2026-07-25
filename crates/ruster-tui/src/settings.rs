//! State for the in-app Settings page. Built from the schema + current config;
//! edits mutate working values that `:w` serializes back to `config.lua`.

use ruster_lua::config::{Addr, Config};
use ruster_lua::schema::{self, SettingKind, SettingSpec, SettingValue};
use ruster_render::{ControlKind, SettingRowView, SettingsGroup, SettingsView};

pub struct SettingsState {
    specs: Vec<SettingSpec>,
    /// Working values, parallel to `specs`.
    values: Vec<SettingValue>,
    /// Selected row (index into `specs`).
    selected: usize,
    /// In-edit text buffer for Text/Number fields (`None` = not editing).
    editing: Option<String>,
    /// Runtime-discovered cyclable options for specific rows (theme names, font
    /// filenames) keyed by row index. Turns those Text fields into pickers.
    dyn_opts: std::collections::HashMap<usize, Vec<String>>,
    pub dirty: bool,
}

impl SettingsState {
    /// `dynamic` supplies runtime option lists for specific rows as
    /// `(group, key, options)` — e.g. discovered themes, fonts, and shells —
    /// turning those Text fields into pickers.
    pub fn new(config: &Config, dynamic: Vec<(&'static str, &'static str, Vec<String>)>) -> Self {
        let specs = schema::schema();
        let mut dyn_opts = std::collections::HashMap::new();
        for (group, key, opts) in dynamic {
            if opts.is_empty() {
                continue;
            }
            if let Some(i) = specs.iter().position(|s| s.group == group && s.key == key) {
                dyn_opts.insert(i, opts);
            }
        }
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
        SettingsState { specs, values, selected: 0, editing: None, dyn_opts, dirty: false }
    }

    /// The selectable options for a row: an enum's variants, or the discovered
    /// theme/font list. `None` for free text/number/toggle rows.
    fn options_for(&self, idx: usize) -> Option<Vec<String>> {
        if let Some(opts) = self.dyn_opts.get(&idx) {
            return Some(opts.clone());
        }
        if let SettingKind::Enum(opts) = self.specs[idx].kind {
            return Some(opts.iter().map(|s| s.to_string()).collect());
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

    // --- navigation ---

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.specs.len() {
            self.selected += 1;
        }
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn next_group(&mut self) {
        let cur = self.specs[self.selected].group;
        if let Some(i) = self.specs.iter().enumerate().skip(self.selected).find(|(_, s)| s.group != cur) {
            self.selected = i.0;
        }
    }

    pub fn prev_group(&mut self) {
        let cur = self.specs[self.selected].group;
        // First row of the previous group: step back to a different group, then
        // back to that group's first row.
        if let Some(prev_group) = self.specs[..self.selected].iter().rev().find(|s| s.group != cur).map(|s| s.group) {
            if let Some(i) = self.specs.iter().position(|s| s.group == prev_group) {
                self.selected = i;
            }
        } else {
            self.selected = 0;
        }
    }

    // --- activation / adjustment ---

    /// Space/Enter: toggle a bool, cycle an enum/theme, else begin editing.
    pub fn activate(&mut self) {
        if matches!(self.specs[self.selected].kind, SettingKind::Bool) {
            self.flip();
        } else if self.options_for(self.selected).is_some() {
            self.cycle(1);
        } else {
            // Text/Number fields edit from an empty buffer (type the new value).
            self.editing = Some(String::new());
        }
    }

    /// h/l or Left/Right: cycle enum/theme, step number, or flip bool.
    pub fn adjust(&mut self, delta: i64) {
        if self.options_for(self.selected).is_some() {
            self.cycle(delta);
            return;
        }
        let spec = self.specs[self.selected].clone();
        match spec.kind {
            SettingKind::Bool => self.flip(),
            SettingKind::Int { min, max } => {
                if let SettingValue::Int(i) = self.values[self.selected] {
                    let n = (i + delta).clamp(min, max);
                    self.set(SettingValue::Int(n));
                }
            }
            SettingKind::Float { min, max } => {
                if let SettingValue::Float(f) = self.values[self.selected] {
                    let n = (f + delta as f64).clamp(min, max);
                    self.set(SettingValue::Float(n));
                }
            }
            _ => {}
        }
    }

    fn flip(&mut self) {
        if let SettingValue::Bool(b) = self.values[self.selected] {
            self.set(SettingValue::Bool(!b));
        }
    }

    fn cycle(&mut self, delta: i64) {
        let Some(opts) = self.options_for(self.selected) else { return };
        if opts.is_empty() {
            return;
        }
        let sentinel = self.unset_sentinel(self.selected);
        let cur_display = self.values[self.selected].display();
        // Empty text (unset font/shell/color) matches the sentinel option.
        let cur = if cur_display.is_empty() { sentinel.to_string() } else { cur_display };
        let idx = opts.iter().position(|o| *o == cur).unwrap_or(0) as i64;
        let n = opts.len() as i64;
        let next = (((idx + delta) % n) + n) % n;
        let chosen = opts[next as usize].clone();
        // Enum rows carry an Enum value; dynamic Text rows (theme/font/color) stay
        // Text, and the "unset" sentinel maps back to an empty value.
        let v = if matches!(self.specs[self.selected].kind, SettingKind::Enum(_)) {
            SettingValue::Enum(chosen)
        } else if chosen == sentinel {
            SettingValue::Text(String::new())
        } else {
            SettingValue::Text(chosen)
        };
        self.set(v);
    }

    /// The word a dynamic Text row shows/uses for an empty (default) value:
    /// "theme" for color overrides, "auto" otherwise.
    fn unset_sentinel(&self, idx: usize) -> &'static str {
        if self.specs[idx].group == "colors" {
            "theme"
        } else {
            "auto"
        }
    }

    fn set(&mut self, v: SettingValue) {
        if self.values[self.selected] != v {
            self.values[self.selected] = v;
            self.dirty = true;
        }
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
        let spec = &self.specs[self.selected];
        if let Some(v) = parse_value(&spec.kind, buf.trim()) {
            if spec.kind.check(&v).is_ok() {
                self.set(v);
            }
        }
    }

    // --- view ---

    pub fn view(&self) -> SettingsView {
        let mut groups: Vec<SettingsGroup> = Vec::new();
        for (i, spec) in self.specs.iter().enumerate() {
            let kind = match &spec.kind {
                SettingKind::Bool => ControlKind::Toggle,
                SettingKind::Enum(_) => ControlKind::Enum,
                SettingKind::Int { .. } | SettingKind::Float { .. } => ControlKind::Number,
                // The theme field is a cyclable list; render it like an enum.
                SettingKind::Text | SettingKind::Color => {
                    if self.options_for(i).is_some() {
                        ControlKind::Enum
                    } else {
                        ControlKind::Text
                    }
                }
            };
            let editing = self.selected == i && self.editing.is_some();
            let value = if editing {
                self.editing.clone().unwrap_or_default()
            } else {
                let d = self.values[i].display();
                // Dynamic Text rows show their unset sentinel ("auto"/"theme").
                if d.is_empty() && self.dyn_opts.contains_key(&i) {
                    self.unset_sentinel(i).to_string()
                } else {
                    decorate_value(spec.group, spec.key, d)
                }
            };
            let row = SettingRowView {
                label: spec.label.to_string(),
                kind,
                value,
                editing,
                selected: self.selected == i,
                help: spec.help.to_string(),
            };
            if groups.last().map(|g| g.name != group_title(spec.group)).unwrap_or(true) {
                groups.push(SettingsGroup { name: group_title(spec.group), rows: Vec::new() });
            }
            groups.last_mut().expect("group pushed").rows.push(row);
        }
        let footer = if self.editing.is_some() {
            "type value · Enter commit · Esc cancel".to_string()
        } else {
            "j/k move · Tab group · Space toggle/cycle · h/l adjust · Enter edit · :w save · q close".to_string()
        };
        SettingsView { groups, dirty: self.dirty, footer }
    }
}

/// Add a friendly suffix to certain enum values for display only (the stored
/// value stays the raw token, so cycling and saving are unaffected).
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
        other => other,
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_mutate_working_values_and_mark_dirty() {
        let mut s = SettingsState::new(&Config::default(), vec![("general", "theme", vec!["default".into(), "gruvbox".into()])]);
        assert!(!s.dirty);
        // First row is general.tabstop (Int 1..16); +2 → 6.
        s.adjust(2);
        assert!(s.dirty);
        let vals = s.values();
        assert_eq!(vals[0].1, SettingValue::Int(6));
    }

    #[test]
    fn enum_cycles_through_options() {
        let mut s = SettingsState::new(&Config::default(), vec![("general", "theme", vec!["default".into(), "gruvbox".into()])]);
        let idx = schema::schema().iter().position(|sp| sp.key == "editmode").unwrap();
        for _ in 0..idx {
            s.move_down();
        }
        s.cycle(1);
        assert_eq!(s.values()[idx].1, SettingValue::Enum("emacs".into()));
        s.cycle(1);
        assert_eq!(s.values()[idx].1, SettingValue::Enum("neovim".into()));
    }

    #[test]
    fn theme_row_cycles_through_discovered_themes() {
        let mut s = SettingsState::new(
            &Config::default(),
            vec![("general", "theme", vec!["default".into(), "gruvbox".into(), "nord".into()])],
        );
        let idx = schema::schema().iter().position(|sp| sp.key == "theme").unwrap();
        for _ in 0..idx {
            s.move_down();
        }
        // Starts at "default"; cycling advances through the discovered list.
        s.activate();
        assert_eq!(s.values()[idx].1, SettingValue::Text("gruvbox".into()));
        s.adjust(1);
        assert_eq!(s.values()[idx].1, SettingValue::Text("nord".into()));
        s.adjust(-1);
        assert_eq!(s.values()[idx].1, SettingValue::Text("gruvbox".into()));
    }

    #[test]
    fn text_edit_commit_validates() {
        let mut s = SettingsState::new(&Config::default(), vec![("general", "theme", vec!["default".into(), "gruvbox".into()])]);
        let idx = schema::schema().iter().position(|sp| sp.key == "font_size").unwrap();
        for _ in 0..idx {
            s.move_down();
        }
        s.activate(); // begin editing (Number)
        assert!(s.is_editing());
        for c in "28".chars() {
            s.edit_push(c);
        }
        s.edit_commit();
        assert_eq!(s.values()[idx].1, SettingValue::Int(28));
        // An out-of-range value is rejected (kept previous).
        s.activate();
        for c in "999".chars() {
            s.edit_push(c);
        }
        s.edit_commit();
        assert_eq!(s.values()[idx].1, SettingValue::Int(28));
    }
}
