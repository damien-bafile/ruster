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
