//! State for the in-app Settings page. Built from the schema + current config;
//! edits mutate working values that `:w` serializes back to `config.lua`.

use std::collections::HashMap;

use ruster_lua::config::{Addr, Config};
use ruster_lua::schema::{self, SettingKind, SettingSpec, SettingValue};
use ruster_render::{ControlKind, SettingRowView, SettingsGroup, SettingsView};

/// A picker option as `(display label, stored value)`. For theme/font/shell the
/// two match (with a sentinel whose value is empty); color rows show a palette
/// color name and store its `#hex`.
type Opt = (String, String);

pub struct SettingsState {
    specs: Vec<SettingSpec>,
    /// Working values, parallel to `specs`.
    values: Vec<SettingValue>,
    /// Selected row (index into `specs`).
    selected: usize,
    /// In-edit text buffer for Text/Number fields (`None` = not editing).
    editing: Option<String>,
    /// Runtime picker options per row (theme/font/shell/color).
    dyn_opts: HashMap<usize, Vec<Opt>>,
    /// Each available theme's palette (name → `(color_name, hex)` pairs), used to
    /// (re)build the color-row options for the selected theme.
    theme_palettes: HashMap<String, Vec<Opt>>,
    theme_idx: Option<usize>,
    color_rows: Vec<usize>,
    pub dirty: bool,
}

impl SettingsState {
    /// `dynamic` supplies `(group, key, options)` for theme/font/shell rows;
    /// `theme_palettes` supplies each theme's palette for the color rows.
    pub fn new(
        config: &Config,
        dynamic: Vec<(&'static str, &'static str, Vec<Opt>)>,
        theme_palettes: Vec<(String, Vec<Opt>)>,
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
        let mut st = SettingsState {
            specs,
            values,
            selected: 0,
            editing: None,
            dyn_opts,
            theme_palettes: theme_palettes.into_iter().collect(),
            theme_idx,
            color_rows,
            dirty: false,
        };
        st.rebuild_color_opts();
        st
    }

    /// Rebuild the color rows' options from the currently-selected theme's
    /// palette. Called on open and whenever the theme row changes.
    fn rebuild_color_opts(&mut self) {
        let theme = self.theme_idx.map(|i| self.values[i].display()).unwrap_or_default();
        let palette = self.theme_palettes.get(&theme).cloned().unwrap_or_default();
        // "theme" (unset → empty value) plus each palette color (name → hex).
        let mut opts: Vec<Opt> = vec![("theme".to_string(), String::new())];
        opts.extend(palette);
        for i in self.color_rows.clone() {
            self.dyn_opts.insert(i, opts.clone());
        }
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
        if let Some(prev_group) = self.specs[..self.selected].iter().rev().find(|s| s.group != cur).map(|s| s.group) {
            if let Some(i) = self.specs.iter().position(|s| s.group == prev_group) {
                self.selected = i;
            }
        } else {
            self.selected = 0;
        }
    }

    // --- activation / adjustment ---

    /// Space/Enter: toggle a bool, cycle a picker, else begin editing.
    pub fn activate(&mut self) {
        if matches!(self.specs[self.selected].kind, SettingKind::Bool) {
            self.flip();
        } else if self.options_for(self.selected).is_some() {
            self.cycle(1);
        } else {
            self.editing = Some(String::new());
        }
    }

    /// h/l or Left/Right: cycle a picker, step a number, or flip a bool.
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
                    self.set(SettingValue::Int((i + delta).clamp(min, max)));
                }
            }
            SettingKind::Float { min, max } => {
                if let SettingValue::Float(f) = self.values[self.selected] {
                    self.set(SettingValue::Float((f + delta as f64).clamp(min, max)));
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
        let cur = self.values[self.selected].display();
        let idx = opts.iter().position(|(_, v)| *v == cur).unwrap_or(0) as i64;
        let n = opts.len() as i64;
        let next = (((idx + delta) % n) + n) % n;
        let value = opts[next as usize].1.clone();
        let v = if matches!(self.specs[self.selected].kind, SettingKind::Enum(_)) {
            SettingValue::Enum(value)
        } else {
            SettingValue::Text(value)
        };
        self.set(v);
    }

    fn set(&mut self, v: SettingValue) {
        if self.values[self.selected] != v {
            self.values[self.selected] = v;
            self.dirty = true;
            // Switching theme repopulates the color pickers with its palette.
            if Some(self.selected) == self.theme_idx {
                self.rebuild_color_opts();
            }
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
            let dyn_row = self.dyn_opts.contains_key(&i);
            let kind = match &spec.kind {
                SettingKind::Bool => ControlKind::Toggle,
                SettingKind::Enum(_) => ControlKind::Enum,
                SettingKind::Int { .. } | SettingKind::Float { .. } => ControlKind::Number,
                SettingKind::Text | SettingKind::Color => {
                    if dyn_row { ControlKind::Enum } else { ControlKind::Text }
                }
            };
            let editing = self.selected == i && self.editing.is_some();
            let stored = self.values[i].display();
            let value = if editing {
                self.editing.clone().unwrap_or_default()
            } else if let Some(opts) = self.dyn_opts.get(&i) {
                // Show the option label for the current stored value.
                opts.iter()
                    .find(|(_, v)| *v == stored)
                    .map(|(l, _)| l.clone())
                    .unwrap_or_else(|| stored.clone())
            } else {
                decorate_value(spec.group, spec.key, stored.clone())
            };
            // A swatch for color rows: the stored hex, if any.
            let swatch = if spec.group == "colors" && is_hex(&stored) {
                Some(stored.clone())
            } else {
                None
            };
            let row = SettingRowView {
                label: spec.label.to_string(),
                kind,
                value,
                editing,
                selected: self.selected == i,
                help: spec.help.to_string(),
                swatch,
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
        let palettes = vec![
            ("default".to_string(), vec![("mauve".to_string(), "#cba6f7".to_string())]),
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
            vec![("default".to_string(), "default".to_string()), ("gruvbox".to_string(), "gruvbox".to_string())],
        )];
        SettingsState::new(&Config::default(), dynamic, palettes)
    }

    fn goto(s: &mut SettingsState, group: &str, key: &str) {
        let idx = schema::schema().iter().position(|sp| sp.group == group && sp.key == key).unwrap();
        s.selected = 0;
        for _ in 0..idx {
            s.move_down();
        }
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
        // Select gruvbox.
        goto(&mut s, "general", "theme");
        s.cycle(1);
        let theme_val =
            s.values().into_iter().find(|((g, k), _)| *g == "general" && *k == "theme").unwrap().1;
        assert_eq!(theme_val, SettingValue::Text("gruvbox".into()));
        // Now the accent picker offers gruvbox colors.
        goto(&mut s, "colors", "accent");
        s.activate(); // theme → first palette color = orange
        assert_eq!(
            s.values().iter().find(|((g, k), _)| *g == "colors" && *k == "accent").unwrap().1,
            SettingValue::Text("#fe8019".into())
        );
    }

    #[test]
    fn stored_hex_displays_palette_name_with_swatch() {
        let mut s = state();
        goto(&mut s, "colors", "cursor");
        s.activate(); // pick mauve
        let view = s.view();
        let row = view
            .groups
            .iter()
            .flat_map(|g| &g.rows)
            .find(|r| r.label == "Cursor")
            .unwrap();
        assert_eq!(row.value, "mauve");
        assert_eq!(row.swatch.as_deref(), Some("#cba6f7"));
    }
}
