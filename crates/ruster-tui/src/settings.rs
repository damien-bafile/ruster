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
    pub dirty: bool,
}

impl SettingsState {
    pub fn new(config: &Config) -> Self {
        let specs = schema::schema();
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
        SettingsState { specs, values, selected: 0, editing: None, dirty: false }
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

    /// Space/Enter: toggle a bool, cycle an enum, else begin editing.
    pub fn activate(&mut self) {
        match &self.specs[self.selected].kind {
            SettingKind::Bool => self.flip(),
            SettingKind::Enum(_) => self.cycle(1),
            // Text/Number fields edit from an empty buffer (type the new value).
            _ => self.editing = Some(String::new()),
        }
    }

    /// h/l or Left/Right: cycle enum, step number, or flip bool.
    pub fn adjust(&mut self, delta: i64) {
        let spec = self.specs[self.selected].clone();
        match spec.kind {
            SettingKind::Enum(_) => self.cycle(delta),
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
        if let SettingKind::Enum(opts) = self.specs[self.selected].kind {
            let cur = self.values[self.selected].display();
            let idx = opts.iter().position(|o| *o == cur).unwrap_or(0) as i64;
            let n = opts.len() as i64;
            let next = ((idx + delta) % n + n) % n;
            self.set(SettingValue::Enum(opts[next as usize].to_string()));
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
                SettingKind::Text | SettingKind::Color => ControlKind::Text,
            };
            let editing = self.selected == i && self.editing.is_some();
            let value = if editing {
                self.editing.clone().unwrap_or_default()
            } else {
                self.values[i].display()
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
        let mut s = SettingsState::new(&Config::default());
        assert!(!s.dirty);
        // First row is general.tabstop (Int 1..16); +2 → 6.
        s.adjust(2);
        assert!(s.dirty);
        let vals = s.values();
        assert_eq!(vals[0].1, SettingValue::Int(6));
    }

    #[test]
    fn enum_cycles_through_options() {
        let mut s = SettingsState::new(&Config::default());
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
    fn text_edit_commit_validates() {
        let mut s = SettingsState::new(&Config::default());
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
