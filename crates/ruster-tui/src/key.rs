use crossterm::event::{KeyCode, KeyEvent as CKEvent, KeyModifiers};
use ruster_core::key::KeyEvent;

pub fn crossterm_to_ruster_key(ck: CKEvent) -> KeyEvent {
    match ck.code {
        KeyCode::Esc => KeyEvent::Esc,
        KeyCode::Enter => KeyEvent::Enter,
        KeyCode::Backspace => KeyEvent::Backspace,
        KeyCode::Delete => KeyEvent::Delete,
        KeyCode::Char(c) if ck.modifiers == KeyModifiers::CONTROL => {
            KeyEvent::Ctrl(c)
        }
        KeyCode::Char(c) => KeyEvent::Char(c),
        _ => KeyEvent::Char(' '),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn char_keys_roundtrip() {
        let ck = CKEvent::new(KeyCode::Char('w'), KeyModifiers::NONE);
        assert_eq!(crossterm_to_ruster_key(ck), KeyEvent::Char('w'));
    }

    #[test]
    fn ctrl_keys_roundtrip() {
        let ck = CKEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(crossterm_to_ruster_key(ck), KeyEvent::Ctrl('c'));
    }

    #[test]
    fn special_keys() {
        fn check(cc: KeyCode, expected: KeyEvent) {
            let ck = CKEvent::new(cc, KeyModifiers::NONE);
            assert_eq!(crossterm_to_ruster_key(ck), expected);
        }
        check(KeyCode::Esc, KeyEvent::Esc);
        check(KeyCode::Enter, KeyEvent::Enter);
        check(KeyCode::Backspace, KeyEvent::Backspace);
        check(KeyCode::Delete, KeyEvent::Delete);
    }
}
