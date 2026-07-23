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
}
