//! Translate key presses into the byte/escape sequences a PTY expects. Shared
//! by both frontends so terminal input behaves identically in the TUI and GUI.

/// A frontend-neutral key. The TUI (crossterm) and GUI (raylib) each map their
/// own key events onto this before encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Tab,
    Backspace,
    Esc,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Delete,
    Insert,
}

/// Active modifier keys.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Mods {
    pub const NONE: Mods = Mods {
        ctrl: false,
        alt: false,
        shift: false,
    };
    pub fn ctrl() -> Mods {
        Mods {
            ctrl: true,
            ..Mods::NONE
        }
    }
}

/// Encode a key press into the bytes to write to the PTY. Returns an empty
/// vec for keys with no terminal representation.
pub fn encode_key(key: Key, mods: Mods) -> Vec<u8> {
    // Alt/Meta prefixes the sequence with ESC.
    let esc = |bytes: &[u8]| -> Vec<u8> {
        if mods.alt {
            let mut v = vec![0x1b];
            v.extend_from_slice(bytes);
            v
        } else {
            bytes.to_vec()
        }
    };

    match key {
        Key::Char(c) => {
            if mods.ctrl {
                // Control chords: map A–Z (and a few symbols) to their C0 code.
                let upper = c.to_ascii_uppercase();
                if upper.is_ascii_alphabetic() {
                    return esc(&[(upper as u8) & 0x1f]);
                }
                match c {
                    ' ' | '@' => return esc(&[0x00]),
                    '[' => return esc(&[0x1b]),
                    '\\' => return esc(&[0x1c]),
                    ']' => return esc(&[0x1d]),
                    '^' => return esc(&[0x1e]),
                    '_' => return esc(&[0x1f]),
                    _ => {}
                }
            }
            let mut buf = [0u8; 4];
            esc(c.encode_utf8(&mut buf).as_bytes())
        }
        Key::Enter => esc(b"\r"),
        Key::Tab => esc(b"\t"),
        Key::Backspace => esc(&[0x7f]),
        Key::Esc => vec![0x1b],
        Key::Up => esc(b"\x1b[A"),
        Key::Down => esc(b"\x1b[B"),
        Key::Right => esc(b"\x1b[C"),
        Key::Left => esc(b"\x1b[D"),
        Key::Home => esc(b"\x1b[H"),
        Key::End => esc(b"\x1b[F"),
        Key::PageUp => esc(b"\x1b[5~"),
        Key::PageDown => esc(b"\x1b[6~"),
        Key::Delete => esc(b"\x1b[3~"),
        Key::Insert => esc(b"\x1b[2~"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_char_is_utf8() {
        assert_eq!(encode_key(Key::Char('a'), Mods::NONE), b"a");
        assert_eq!(encode_key(Key::Char('é'), Mods::NONE), "é".as_bytes());
    }

    #[test]
    fn ctrl_letter_is_c0_control() {
        assert_eq!(encode_key(Key::Char('c'), Mods::ctrl()), vec![0x03]); // Ctrl-C
        assert_eq!(encode_key(Key::Char('d'), Mods::ctrl()), vec![0x04]); // Ctrl-D
    }

    #[test]
    fn enter_and_special_keys() {
        assert_eq!(encode_key(Key::Enter, Mods::NONE), b"\r");
        assert_eq!(encode_key(Key::Backspace, Mods::NONE), vec![0x7f]);
        assert_eq!(encode_key(Key::Up, Mods::NONE), b"\x1b[A");
    }

    #[test]
    fn alt_prefixes_escape() {
        let m = Mods {
            alt: true,
            ..Mods::NONE
        };
        assert_eq!(encode_key(Key::Char('b'), m), vec![0x1b, b'b']);
    }
}
