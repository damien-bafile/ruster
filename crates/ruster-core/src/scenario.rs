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
        // A fresh Editor starts with an empty UndoStack — undo MUST be in the same session as the
        // change to do anything. Straight-line script: gg ciw x Esc creates a change; u reverses it.
        scenario(
            "hello world",
            &[
                KeyEvent::Char('g'), KeyEvent::Char('g'),
                KeyEvent::Char('c'), KeyEvent::Char('i'), KeyEvent::Char('w'),
                KeyEvent::Char('x'),
                KeyEvent::Esc,
                KeyEvent::Char('u'),
            ],
            "hello world", None,
        );
    }

    #[test]
    fn g_minus_recovers_a_branch_that_redo_cannot() {
        // Delete a word, undo it, then delete a different one — that abandons
        // the first deletion onto a side branch. `g-` walks back to it in time
        // order, where `C-r` would only ever see the newest branch.
        scenario(
            "foo bar baz",
            &[
                KeyEvent::Char('g'), KeyEvent::Char('g'),
                KeyEvent::Char('d'), KeyEvent::Char('w'), // -> "bar baz"
                KeyEvent::Char('u'),                      // -> "foo bar baz"
                KeyEvent::Char('w'),
                KeyEvent::Char('d'), KeyEvent::Char('w'), // -> "foo baz"
                KeyEvent::Char('g'), KeyEvent::Char('-'), // back to "bar baz"
            ],
            "bar baz", None,
        );
    }

    #[test]
    fn g_plus_returns_along_the_newer_branch() {
        scenario(
            "foo bar baz",
            &[
                KeyEvent::Char('g'), KeyEvent::Char('g'),
                KeyEvent::Char('d'), KeyEvent::Char('w'),
                KeyEvent::Char('u'),
                KeyEvent::Char('w'),
                KeyEvent::Char('d'), KeyEvent::Char('w'), // -> "foo baz"
                KeyEvent::Char('g'), KeyEvent::Char('-'), // -> "bar baz"
                KeyEvent::Char('g'), KeyEvent::Char('+'), // -> "foo baz" again
            ],
            "foo baz", None,
        );
    }

    #[test]
    fn undo_lands_cursor_at_change_position_not_zero() {
        // After deleting text in the middle of the buffer, `u` should land
        // cursor at the position of the change, not hardcoded to offset 0.
        scenario(
            "foo bar baz",
            &[
                KeyEvent::Char('g'), KeyEvent::Char('g'),
                KeyEvent::Char('w'), // cursor -> 4 (start of "bar")
                KeyEvent::Char('d'), KeyEvent::Char('w'), // delete "bar " -> "foo baz"
                KeyEvent::Char('u'), // undo: restores "bar " at offset 4
            ],
            "foo bar baz", Some(4),
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
