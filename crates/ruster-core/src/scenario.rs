use crate::editor::Editor;
use crate::key::KeyEvent;
use crate::vim::VimState;

/// Drive a headless Editor+VimState through a script of KeyEvents, asserting the
/// final buffer text (and optionally the cursor head). This is Plan A's regression backbone.
pub fn scenario(src: &str, keys: &[KeyEvent], expect_text: &str, expect_head: Option<usize>) {
    let mut e = Editor::from_str(src);
    let mut v = VimState::new();
    for k in keys {
        for a in v.handle(*k, &e) { e.execute(a); }
    }
    assert_eq!(e.buffer().to_string(), expect_text,
        "scenario src={:?} keys={:?}", src, keys);
    if let Some(h) = expect_head { assert_eq!(e.primary_head(), h); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::KeyEvent;

    #[test]
    fn edit_word_then_undo() {
        // ciw changes inner word under cursor (cursor ends up at "hello" via gg-from-end then w to "hello")
        // Plan A VimState cursor starts at end-of-buffer; gg jumps to 0; w moves to next word start
        // For "hello world" the first word is "hello" at offset 0, so after gg cursor is at 0; ciw deletes "hello" and enters insert; x types "x"; Esc exits insert
        scenario(
            "hello world",
            &[
                KeyEvent::Char('g'), KeyEvent::Char('g'),
                KeyEvent::Char('c'), KeyEvent::Char('i'), KeyEvent::Char('w'),
                KeyEvent::Char('x'),
                KeyEvent::Esc,
            ],
            "x world", None,
        );
        // After Esc, cursor is left of the inserted "x" -> offset 0
        // Undo restores "hello world"
        scenario(
            "x world",
            &[KeyEvent::Char('u')],
            "hello world", None,
        );
    }

    #[test]
    fn full_vim_pipeline_delete_word_dot_repeat() {
        // dw on "foo " then w to "bar " then . deletes "bar " -> "baz"
        // cursor starts at end-of-buffer; gg jumps to 0; dw deletes "foo " -> "bar baz"
        // w moves cursor to start of "bar"? No: after dw cursor is at 0 ('b' of "bar"); w moves to next word start = offset 4 ('b' of "baz").
        // . repeats dw at cursor 4: deletes "baz" -> "bar " (with trailing space)
        scenario(
            "foo bar baz",
            &[
                KeyEvent::Char('g'), KeyEvent::Char('g'),
                KeyEvent::Char('d'), KeyEvent::Char('w'),
                KeyEvent::Char('w'),
                KeyEvent::Char('.'),
            ],
            "bar ", None,
        );
    }
}
