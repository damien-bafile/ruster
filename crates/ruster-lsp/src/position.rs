//! Conversions between ruster char offsets and LSP positions.
//!
//! ruster addresses text by **char offset** (Unicode scalar values from the
//! buffer start). LSP addresses text by `(line, character)` where `character`
//! counts **UTF-16 code units** within the line. These helpers convert both
//! ways so requests and results line up on multi-byte / astral-plane text.

/// LSP position: zero-based line, zero-based UTF-16 character within the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LspPosition {
    pub line: u32,
    pub character: u32,
}

/// Convert a char offset in `text` to an LSP position.
pub fn offset_to_position(text: &str, char_offset: usize) -> LspPosition {
    let mut line = 0u32;
    let mut utf16 = 0u32;
    for (i, ch) in text.chars().enumerate() {
        if i == char_offset {
            return LspPosition { line, character: utf16 };
        }
        if ch == '\n' {
            line += 1;
            utf16 = 0;
        } else {
            utf16 += ch.len_utf16() as u32;
        }
    }
    // At or past end of text.
    LspPosition { line, character: utf16 }
}

/// Convert an LSP position in `text` back to a char offset.
pub fn position_to_offset(text: &str, pos: LspPosition) -> usize {
    let mut cur_line = 0u32;
    let mut utf16 = 0u32;
    for (i, ch) in text.chars().enumerate() {
        if cur_line == pos.line && utf16 >= pos.character {
            return i;
        }
        if ch == '\n' {
            if cur_line == pos.line {
                // Target character is past this line's end; clamp to line end.
                return i;
            }
            cur_line += 1;
            utf16 = 0;
        } else {
            utf16 += ch.len_utf16() as u32;
        }
    }
    text.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(line: u32, character: u32) -> LspPosition {
        LspPosition { line, character }
    }

    #[test]
    fn ascii_round_trips() {
        let text = "fn main() {\n    let x = 1;\n}";
        // offset of 'x' on line 1
        let off = text.chars().position(|c| c == 'x').unwrap();
        let p = offset_to_position(text, off);
        assert_eq!(p, pos(1, 8));
        assert_eq!(position_to_offset(text, p), off);
    }

    #[test]
    fn start_and_line_breaks() {
        let text = "a\nb\nc";
        assert_eq!(offset_to_position(text, 0), pos(0, 0));
        assert_eq!(offset_to_position(text, 2), pos(1, 0)); // 'b'
        assert_eq!(offset_to_position(text, 4), pos(2, 0)); // 'c'
    }

    #[test]
    fn astral_char_counts_two_utf16_units() {
        // "a😀b" — the emoji is 2 UTF-16 code units.
        let text = "a😀b";
        // char offset 2 is 'b'; after 'a'(1) + emoji(2) => character 3
        assert_eq!(offset_to_position(text, 2), pos(0, 3));
        assert_eq!(position_to_offset(text, pos(0, 3)), 2);
    }

    #[test]
    fn position_past_line_end_clamps() {
        let text = "ab\ncd";
        // asking for character 10 on line 0 clamps to the newline (offset 2)
        assert_eq!(position_to_offset(text, pos(0, 10)), 2);
    }
}
