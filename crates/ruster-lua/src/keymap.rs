#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LuaKey {
    Char(char),
    Ctrl(char),
    Esc,
    Enter,
    Tab,
    BackTab,
    Backspace,
    Delete,
    Home,
    End,
    Left,
    Right,
    Up,
    Down,
    F(u8),
}

#[derive(Debug)]
pub struct LuaKeymap {
    pub mode: String,
    pub keys: Vec<LuaKey>,
    pub callback: mlua::RegistryKey,
}

/// Parse an angle-bracket key string like "<C-s>" or "j".
/// Returns None for unrecognized sequences.
pub fn parse_lua_key(s: &str) -> Option<LuaKey> {
    if s.len() == 1 {
        return Some(LuaKey::Char(s.chars().next().unwrap()));
    }
    if !s.starts_with('<') || !s.ends_with('>') {
        return None;
    }
    let inner = &s[1..s.len() - 1];
    match inner {
        "Esc" => Some(LuaKey::Esc),
        "CR" | "Enter" => Some(LuaKey::Enter),
        "Tab" => Some(LuaKey::Tab),
        "S-Tab" => Some(LuaKey::BackTab),
        "BS" | "Backspace" => Some(LuaKey::Backspace),
        "Del" | "Delete" => Some(LuaKey::Delete),
        "Home" => Some(LuaKey::Home),
        "End" => Some(LuaKey::End),
        "Left" => Some(LuaKey::Left),
        "Right" => Some(LuaKey::Right),
        "Up" => Some(LuaKey::Up),
        "Down" => Some(LuaKey::Down),
        _ if inner.len() == 3 && inner.starts_with('C') && inner.as_bytes()[1] == b'-' => {
            let c = inner.as_bytes()[2] as char;
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                Some(LuaKey::Ctrl(c))
            } else {
                None
            }
        }
        _ if inner.len() >= 2 && inner.starts_with('F') => inner[1..]
            .parse::<u8>()
            .ok()
            .filter(|&n| (1..=12).contains(&n))
            .map(LuaKey::F),
        _ => None,
    }
}

/// Convert a LuaKey to a single crossterm event for matching.
/// Returns None for multi-key sequences (handled at the LuaKeymap level).
pub fn lua_key_to_crossterm(key: &LuaKey) -> crossterm::event::KeyEvent {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    match key {
        LuaKey::Char(c) => KeyEvent::new(KeyCode::Char(*c), KeyModifiers::NONE),
        LuaKey::Ctrl(c) => KeyEvent::new(KeyCode::Char(*c), KeyModifiers::CONTROL),
        LuaKey::Esc => KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        LuaKey::Enter => KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        LuaKey::Tab => KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
        LuaKey::BackTab => KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE),
        LuaKey::Backspace => KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        LuaKey::Delete => KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE),
        LuaKey::Home => KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
        LuaKey::End => KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
        LuaKey::Left => KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
        LuaKey::Right => KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
        LuaKey::Up => KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
        LuaKey::Down => KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        LuaKey::F(n) => KeyEvent::new(KeyCode::F(*n), KeyModifiers::NONE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_char() {
        assert_eq!(parse_lua_key("j"), Some(LuaKey::Char('j')));
        assert_eq!(parse_lua_key(":"), Some(LuaKey::Char(':')));
    }

    #[test]
    fn parse_ctrl_key() {
        assert_eq!(parse_lua_key("<C-s>"), Some(LuaKey::Ctrl('s')));
        assert_eq!(parse_lua_key("<C-a>"), Some(LuaKey::Ctrl('a')));
    }

    #[test]
    fn parse_special_keys() {
        assert_eq!(parse_lua_key("<Esc>"), Some(LuaKey::Esc));
        assert_eq!(parse_lua_key("<CR>"), Some(LuaKey::Enter));
        assert_eq!(parse_lua_key("<Tab>"), Some(LuaKey::Tab));
        assert_eq!(parse_lua_key("<S-Tab>"), Some(LuaKey::BackTab));
        assert_eq!(parse_lua_key("<BS>"), Some(LuaKey::Backspace));
        assert_eq!(parse_lua_key("<Del>"), Some(LuaKey::Delete));
    }

    #[test]
    fn parse_function_keys() {
        assert_eq!(parse_lua_key("<F1>"), Some(LuaKey::F(1)));
        assert_eq!(parse_lua_key("<F12>"), Some(LuaKey::F(12)));
    }

    #[test]
    fn parse_invalid_returns_none() {
        assert_eq!(parse_lua_key("<invalid>"), None);
        assert_eq!(parse_lua_key(""), None);
    }
}
