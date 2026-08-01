//! A modal form: a title, a list of fields, and a submit/cancel outcome.
//!
//! The fields reuse [`ControlKind`] and [`SettingRowView`] — the same vocabulary
//! the settings page is built from, which **both** backends already render. That
//! is the whole reason this exists rather than a widget crate: `ratatui-widgets`
//! and friends only draw into ratatui, and half of this editor is raylib.
//!
//! Pure: navigation, editing and value extraction need no workspace, so they are
//! all tested directly.

use ruster_render::{ControlKind, DialogView, SettingRowView};

/// One field in a dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub label: String,
    pub kind: ControlKind,
    /// Current value, as displayed and as handed back on submit.
    pub value: String,
    /// Choices for [`ControlKind::Enum`]; ignored otherwise.
    pub options: Vec<String>,
}

impl Field {
    pub fn toggle(label: &str, on: bool) -> Self {
        Self {
            label: label.into(),
            kind: ControlKind::Toggle,
            value: if on { "on".into() } else { "off".into() },
            options: Vec::new(),
        }
    }

    pub fn text(label: &str, value: &str) -> Self {
        Self {
            label: label.into(),
            kind: ControlKind::Text,
            value: value.into(),
            options: Vec::new(),
        }
    }

    pub fn number(label: &str, value: i64) -> Self {
        Self {
            label: label.into(),
            kind: ControlKind::Number,
            value: value.to_string(),
            options: Vec::new(),
        }
    }

    pub fn select(label: &str, options: &[&str], value: &str) -> Self {
        Self {
            label: label.into(),
            kind: ControlKind::Enum,
            value: value.into(),
            options: options.iter().map(|s| s.to_string()).collect(),
        }
    }
}

/// What the caller should do after feeding a key to a dialog.
#[derive(Debug, PartialEq, Eq)]
pub enum DialogResponse {
    /// Still open.
    Pending,
    /// Dismissed without a result.
    Cancelled,
    /// Accepted; read the values with [`DialogState::values`].
    Submitted,
}

pub struct DialogState {
    title: String,
    fields: Vec<Field>,
    selected: usize,
    /// In-place edit buffer for a Text/Number field, `None` when not editing.
    editing: Option<String>,
}

impl DialogState {
    pub fn new(title: impl Into<String>, fields: Vec<Field>) -> Self {
        Self { title: title.into(), fields, selected: 0, editing: None }
    }

    pub fn is_editing(&self) -> bool {
        self.editing.is_some()
    }

    /// `(label, value)` for every field, in declaration order.
    pub fn values(&self) -> Vec<(String, String)> {
        self.fields.iter().map(|f| (f.label.clone(), f.value.clone())).collect()
    }

    pub fn handle_key(&mut self, ck: crossterm::event::KeyEvent) -> DialogResponse {
        use crossterm::event::KeyCode;

        // While editing, keys go into the buffer — otherwise typing a name that
        // contains 'j' would move the selection out from under it.
        if let Some(buf) = self.editing.as_mut() {
            match ck.code {
                KeyCode::Char(c) => buf.push(c),
                KeyCode::Backspace => {
                    buf.pop();
                }
                KeyCode::Enter => {
                    let v = self.editing.take().unwrap_or_default();
                    if let Some(f) = self.fields.get_mut(self.selected) {
                        f.value = v;
                    }
                }
                KeyCode::Esc => {
                    self.editing = None;
                }
                _ => {}
            }
            return DialogResponse::Pending;
        }

        match ck.code {
            KeyCode::Esc => return DialogResponse::Cancelled,
            KeyCode::Char('j') | KeyCode::Down => {
                if self.selected + 1 < self.fields.len() {
                    self.selected += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
            }
            KeyCode::Char(' ') => self.activate(),
            KeyCode::Char('h') | KeyCode::Left => self.cycle(-1),
            KeyCode::Char('l') | KeyCode::Right => self.cycle(1),
            KeyCode::Enter => {
                // Enter edits a text field, and submits anywhere else. A text
                // field still needs a way out: Enter again commits, then Enter
                // submits.
                match self.fields.get(self.selected).map(|f| f.kind) {
                    Some(ControlKind::Text) | Some(ControlKind::Number) => self.activate(),
                    _ => return DialogResponse::Submitted,
                }
            }
            _ => {}
        }
        DialogResponse::Pending
    }

    /// Space/Enter on the selected field: toggle, cycle, or start editing.
    fn activate(&mut self) {
        let Some(f) = self.fields.get_mut(self.selected) else { return };
        match f.kind {
            ControlKind::Toggle => {
                f.value = if f.value == "on" { "off".into() } else { "on".into() };
            }
            ControlKind::Enum => self.cycle(1),
            ControlKind::Text | ControlKind::Number => {
                self.editing = Some(f.value.clone());
            }
        }
    }

    /// Step an enum field through its options, wrapping.
    fn cycle(&mut self, delta: isize) {
        let Some(f) = self.fields.get_mut(self.selected) else { return };
        if f.kind != ControlKind::Enum || f.options.is_empty() {
            return;
        }
        let n = f.options.len() as isize;
        let cur = f.options.iter().position(|o| *o == f.value).unwrap_or(0) as isize;
        let next = ((cur + delta) % n + n) % n;
        f.value = f.options[next as usize].clone();
    }

    pub fn view(&self) -> DialogView {
        let rows = self
            .fields
            .iter()
            .enumerate()
            .map(|(i, f)| SettingRowView {
                label: f.label.clone(),
                kind: f.kind,
                value: if self.editing.is_some() && i == self.selected {
                    self.editing.clone().unwrap_or_default()
                } else {
                    f.value.clone()
                },
                selected: i == self.selected,
                editing: self.editing.is_some() && i == self.selected,
                help: String::new(),
                swatch: None,
            })
            .collect();
        DialogView {
            title: self.title.clone(),
            rows,
            footer: if self.editing.is_some() {
                "Enter commit · Esc cancel edit".into()
            } else {
                "j/k move · Space toggle/cycle · Enter submit · Esc cancel".into()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent};

    fn key(c: char) -> KeyEvent {
        KeyEvent::from(KeyCode::Char(c))
    }

    fn dialog() -> DialogState {
        DialogState::new(
            "Test",
            vec![
                Field::toggle("Enabled", true),
                Field::select("Mode", &["fast", "slow"], "fast"),
                Field::text("Name", "hello"),
            ],
        )
    }

    #[test]
    fn space_toggles_a_boolean() {
        let mut d = dialog();
        assert_eq!(d.values()[0].1, "on");
        d.handle_key(key(' '));
        assert_eq!(d.values()[0].1, "off");
        d.handle_key(key(' '));
        assert_eq!(d.values()[0].1, "on");
    }

    #[test]
    fn enum_fields_cycle_and_wrap_both_ways() {
        let mut d = dialog();
        d.handle_key(key('j'));
        d.handle_key(key('l'));
        assert_eq!(d.values()[1].1, "slow");
        d.handle_key(key('l'));
        assert_eq!(d.values()[1].1, "fast", "wraps forward");
        d.handle_key(key('h'));
        assert_eq!(d.values()[1].1, "slow", "wraps backward");
    }

    #[test]
    fn navigation_clamps_at_both_ends() {
        let mut d = dialog();
        d.handle_key(key('k'));
        assert!(d.view().rows[0].selected, "clamped at the top");
        for _ in 0..10 {
            d.handle_key(key('j'));
        }
        assert!(d.view().rows[2].selected, "clamped at the bottom");
    }

    /// While a text field is being edited, `j` is a character — not a motion.
    #[test]
    fn typing_into_a_text_field_does_not_move_the_selection() {
        let mut d = dialog();
        d.handle_key(key('j'));
        d.handle_key(key('j')); // onto "Name"
        d.handle_key(KeyEvent::from(KeyCode::Enter)); // start editing
        assert!(d.is_editing());
        for c in "jjkk".chars() {
            d.handle_key(key(c));
        }
        assert!(d.view().rows[2].selected, "still on the text field");
        d.handle_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(d.values()[2].1, "hellojjkk");
    }

    #[test]
    fn esc_while_editing_abandons_the_edit_but_keeps_the_dialog() {
        let mut d = dialog();
        d.handle_key(key('j'));
        d.handle_key(key('j'));
        d.handle_key(KeyEvent::from(KeyCode::Enter));
        d.handle_key(key('X'));
        let r = d.handle_key(KeyEvent::from(KeyCode::Esc));
        assert_eq!(r, DialogResponse::Pending, "dialog stays open");
        assert!(!d.is_editing());
        assert_eq!(d.values()[2].1, "hello", "original value kept");
    }

    #[test]
    fn enter_submits_from_a_non_text_field() {
        let mut d = dialog();
        assert_eq!(d.handle_key(KeyEvent::from(KeyCode::Enter)), DialogResponse::Submitted);
    }

    #[test]
    fn esc_cancels() {
        let mut d = dialog();
        assert_eq!(d.handle_key(KeyEvent::from(KeyCode::Esc)), DialogResponse::Cancelled);
    }

    #[test]
    fn values_come_back_in_declaration_order() {
        let d = dialog();
        let v = d.values();
        assert_eq!(v.len(), 3);
        assert_eq!(v[0].0, "Enabled");
        assert_eq!(v[1].0, "Mode");
        assert_eq!(v[2].0, "Name");
    }

    #[test]
    fn the_view_marks_the_selected_row_and_shows_the_edit_buffer() {
        let mut d = dialog();
        d.handle_key(key('j'));
        let v = d.view();
        assert!(v.rows[1].selected);
        assert!(!v.rows[1].editing);
        d.handle_key(key('j'));
        d.handle_key(KeyEvent::from(KeyCode::Enter));
        d.handle_key(key('!'));
        let v = d.view();
        assert!(v.rows[2].editing);
        assert_eq!(v.rows[2].value, "hello!", "shows the in-progress edit");
    }

    #[test]
    fn an_empty_dialog_does_not_panic() {
        let mut d = DialogState::new("Empty", vec![]);
        d.handle_key(key('j'));
        d.handle_key(key(' '));
        assert!(d.values().is_empty());
        assert_eq!(d.handle_key(KeyEvent::from(KeyCode::Enter)), DialogResponse::Submitted);
    }
}
