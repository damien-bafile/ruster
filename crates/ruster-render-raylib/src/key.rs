use crossterm::event::{KeyEvent, KeyCode, KeyModifiers};

pub fn map_raylib_key(key: raylib::consts::KeyboardKey) -> Option<KeyEvent> {
    use raylib::consts::KeyboardKey::*;
    let code = match key {
        KEY_ENTER => KeyCode::Enter,
        KEY_BACKSPACE => KeyCode::Backspace,
        KEY_TAB => KeyCode::Tab,
        KEY_ESCAPE => KeyCode::Esc,
        KEY_LEFT => KeyCode::Left,
        KEY_RIGHT => KeyCode::Right,
        KEY_UP => KeyCode::Up,
        KEY_DOWN => KeyCode::Down,
        KEY_HOME => KeyCode::Home,
        KEY_END => KeyCode::End,
        KEY_PAGE_UP => KeyCode::PageUp,
        KEY_PAGE_DOWN => KeyCode::PageDown,
        KEY_DELETE => KeyCode::Delete,
        KEY_BACK => KeyCode::Esc,
        _ => return None,
    };
    Some(KeyEvent::new(code, KeyModifiers::empty()))
}

/// The base character a physical key stands for when a `Ctrl`/`Alt` chord is
/// being formed. The OS text-input layer suppresses or mangles modified keys
/// (Alt+f yields nothing on X11/Windows, a composed glyph on macOS, and
/// Ctrl+Space yields NUL), so Emacs chords like `C-f`, `M-f`, and `C-SPC` must
/// be reconstructed from the physical key instead. Emacs chords are
/// case-insensitive, so letters map to lowercase.
pub fn modified_char_for_key(key: raylib::consts::KeyboardKey, shift: bool) -> Option<char> {
    use raylib::consts::KeyboardKey::*;
    let n = key as u32;
    // raylib letter/digit key codes equal their ASCII uppercase / digit values.
    if (KEY_A as u32..=KEY_Z as u32).contains(&n) {
        return char::from_u32(n + 32); // 'A' -> 'a'
    }
    if (KEY_ZERO as u32..=KEY_NINE as u32).contains(&n) {
        return char::from_u32(n);
    }
    let c = match key {
        KEY_SPACE => ' ',
        KEY_COMMA => if shift { '<' } else { ',' },
        KEY_PERIOD => if shift { '>' } else { '.' },
        KEY_SLASH => '/',
        KEY_SEMICOLON => ';',
        KEY_MINUS => if shift { '_' } else { '-' },
        KEY_APOSTROPHE => '\'',
        // The bracket family. Without these, a chord over any of them produced
        // no event at all in the GUI — `Ctrl-\` is the embedded terminal's
        // escape key, so leaving it unmapped meant the terminal could not be
        // exited with a mouse or a keyboard.
        KEY_BACKSLASH => if shift { '|' } else { '\\' },
        KEY_LEFT_BRACKET => if shift { '{' } else { '[' },
        KEY_RIGHT_BRACKET => if shift { '}' } else { ']' },
        KEY_GRAVE => if shift { '~' } else { '`' },
        KEY_EQUAL => if shift { '+' } else { '=' },
        _ => return None,
    };
    Some(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_enter() {
        let e = map_raylib_key(raylib::consts::KeyboardKey::KEY_ENTER).unwrap();
        assert_eq!(e.code, KeyCode::Enter);
    }

    #[test]
    fn maps_backspace() {
        let e = map_raylib_key(raylib::consts::KeyboardKey::KEY_BACKSPACE).unwrap();
        assert_eq!(e.code, KeyCode::Backspace);
    }

    #[test]
    fn maps_tab() {
        let e = map_raylib_key(raylib::consts::KeyboardKey::KEY_TAB).unwrap();
        assert_eq!(e.code, KeyCode::Tab);
    }

    #[test]
    fn maps_arrows() {
        for (k, expected) in &[
            (raylib::consts::KeyboardKey::KEY_LEFT, KeyCode::Left),
            (raylib::consts::KeyboardKey::KEY_RIGHT, KeyCode::Right),
            (raylib::consts::KeyboardKey::KEY_UP, KeyCode::Up),
            (raylib::consts::KeyboardKey::KEY_DOWN, KeyCode::Down),
        ] {
            let e = map_raylib_key(*k).unwrap();
            assert_eq!(e.code, *expected);
        }
    }

    #[test]
    fn maps_navigation_keys() {
        for (k, expected) in &[
            (raylib::consts::KeyboardKey::KEY_HOME, KeyCode::Home),
            (raylib::consts::KeyboardKey::KEY_END, KeyCode::End),
            (raylib::consts::KeyboardKey::KEY_PAGE_UP, KeyCode::PageUp),
            (raylib::consts::KeyboardKey::KEY_PAGE_DOWN, KeyCode::PageDown),
            (raylib::consts::KeyboardKey::KEY_DELETE, KeyCode::Delete),
            (raylib::consts::KeyboardKey::KEY_BACK, KeyCode::Esc),
        ] {
            let e = map_raylib_key(*k).unwrap();
            assert_eq!(e.code, *expected);
        }
    }

    #[test]
    fn returns_none_for_unknown_key() {
        assert!(map_raylib_key(raylib::consts::KeyboardKey::KEY_A).is_none());
        assert!(map_raylib_key(raylib::consts::KeyboardKey::KEY_F1).is_none());
    }

    #[test]
    fn modified_letters_are_lowercase() {
        use raylib::consts::KeyboardKey::*;
        // C-f / M-f resolve to 'f' regardless of shift.
        assert_eq!(modified_char_for_key(KEY_F, false), Some('f'));
        assert_eq!(modified_char_for_key(KEY_F, true), Some('f'));
        assert_eq!(modified_char_for_key(KEY_A, false), Some('a'));
        assert_eq!(modified_char_for_key(KEY_Z, false), Some('z'));
    }

    #[test]
    fn modified_space_and_digits() {
        use raylib::consts::KeyboardKey::*;
        // C-SPC (set mark) and C-u digit arguments.
        assert_eq!(modified_char_for_key(KEY_SPACE, false), Some(' '));
        assert_eq!(modified_char_for_key(KEY_ZERO, false), Some('0'));
        assert_eq!(modified_char_for_key(KEY_NINE, false), Some('9'));
    }

    #[test]
    fn modified_shifted_punctuation_for_meta_angles() {
        use raylib::consts::KeyboardKey::*;
        // M-< / M-> (beginning/end of buffer) need the shifted forms.
        assert_eq!(modified_char_for_key(KEY_COMMA, true), Some('<'));
        assert_eq!(modified_char_for_key(KEY_PERIOD, true), Some('>'));
        assert_eq!(modified_char_for_key(KEY_COMMA, false), Some(','));
        // C-/ (undo).
        assert_eq!(modified_char_for_key(KEY_SLASH, false), Some('/'));
        // `Ctrl-\` leaves the embedded terminal. While this returned None the
        // chord produced no event at all and the terminal was a one-way door.
        assert_eq!(modified_char_for_key(KEY_BACKSLASH, false), Some('\\'));
        assert_eq!(modified_char_for_key(KEY_BACKSLASH, true), Some('|'));
        assert_eq!(modified_char_for_key(KEY_LEFT_BRACKET, false), Some('['));
        assert_eq!(modified_char_for_key(KEY_RIGHT_BRACKET, false), Some(']'));
        assert_eq!(modified_char_for_key(KEY_GRAVE, false), Some('`'));
        assert_eq!(modified_char_for_key(KEY_EQUAL, true), Some('+'));
    }

    #[test]
    fn modified_returns_none_for_special_keys() {
        // Enter/arrows have no base character; the caller falls back to
        // map_raylib_key for those.
        assert_eq!(modified_char_for_key(raylib::consts::KeyboardKey::KEY_ENTER, false), None);
        assert_eq!(modified_char_for_key(raylib::consts::KeyboardKey::KEY_LEFT, false), None);
    }
}
