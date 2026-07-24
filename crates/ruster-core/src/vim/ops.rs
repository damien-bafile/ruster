use crate::buffer::Buffer;
use crate::editor::EditorView;
use crate::vim::motions::{next_word_start, prev_word_start, word_end, last_printable_in_line, char_to_line};

/// Compute the (start, end) char range for an operator (`d`/`y`/`c`) applied `count` times
/// to the named `motion`. Motions supported in the slice: `w`, `b`, `e`, `$`, `d` (line).
/// Returns `None` for any other motion (text objects come in Task 9).
pub fn range_for_motion(editor: &dyn EditorView, motion: char, count: u32) -> Option<(usize, usize)> {
    let head = editor.primary_head();
    let buf: &Buffer = editor.buffer();
    let total = buf.len_chars();
    match motion {
        'w' => {
            let mut end = head;
            for _ in 0..count { end = next_word_start(buf, end); }
            Some((head, end.min(total)))
        }
        'e' => {
            let mut end = head;
            for _ in 0..count { end = word_end(buf, end); }
            Some((head, (end + 1).min(total)))
        }
        'b' => {
            let mut start = head;
            for _ in 0..count { start = prev_word_start(buf, start); }
            Some((start, head))
        }
        '$' => {
            let last = last_printable_in_line(editor);
            Some((head, (last + 1).min(total)))
        }
        '0' => {
            let line = char_to_line(editor, head);
            let start = buf.line_start_char(line);
            // operator on `0` deletes/yanks the range from line start to current head (backward)
            Some((start, head + 1))
        }
        'G' => {
            // operator on `G` extends from head to the END of the last line (whole tail of buffer)
            let last_line = buf.line_count().saturating_sub(1);
            let start = head;
            let end = buf.line_end_char(last_line);
            Some((start, end))
        }
        'd' | 'y' | 'c' => {
            // dd/yy/cc and {count}dd etc.: operate on whole lines starting at current line,
            // INCLUDING the trailing newline (ropey's line_end_char points past the newline
            // when not the last line). Doubled-operator convention.
            let line = char_to_line(editor, head);
            let start = buf.line_start_char(line);
            let end_line = (line + (count as usize).saturating_sub(1)).min(buf.line_count().saturating_sub(1));
            let end = buf.line_end_char(end_line);
            Some((start, end))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::editor::Editor;
    use crate::key::KeyEvent;
    use crate::vim::{VimMode, VimState};

    fn to_start(e: &mut Editor, v: &mut VimState) {
        for a in v.handle(KeyEvent::Char('g'), e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('g'), e) { e.execute(a); }
    }

    #[test]
    fn dw_deletes_to_next_word_start() {
        let mut e = Editor::from_str("hello world");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('d'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('w'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "world");
        assert_eq!(e.primary_head(), 0);
    }

    #[test]
    fn d_dollar_deletes_to_end_of_line() {
        let mut e = Editor::from_str("hello world");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('d'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('$'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "");
        assert_eq!(e.primary_head(), 0);
    }

    #[test]
    fn dd_deletes_whole_line() {
        let mut e = Editor::from_str("abc\ndef\nghi");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('d'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('d'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "def\nghi");
        assert_eq!(e.primary_head(), 0);
    }

    #[test]
    fn x_deletes_char_under_cursor() {
        let mut e = Editor::from_str("ab");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('x'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "b");
        assert_eq!(e.primary_head(), 0);
    }

    #[test]
    fn yy_then_p_pastes_the_line_below() {
        let mut e = Editor::from_str("hello");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('y'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('y'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "hello");
        // `yy` is line-wise, so `p` puts the copy on the following line.
        for a in v.handle(KeyEvent::Char('p'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "hello\nhello");
    }

    #[test]
    fn insert_entry_keys() {
        // `a` appends after the cursor.
        let mut e = Editor::from_str("ab");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('a'), &e) { e.execute(a); }
        assert_eq!(e.primary_head(), 1, "a moves one right");

        // `A` appends at end of line.
        let mut e = Editor::from_str("ab\ncd");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('A'), &e) { e.execute(a); }
        assert_eq!(e.primary_head(), 2, "A goes to end of the first line");

        // `I` goes to the first non-blank.
        let mut e = Editor::from_str("   xy");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('I'), &e) { e.execute(a); }
        assert_eq!(e.primary_head(), 3, "I skips leading blanks");
    }

    #[test]
    fn open_line_below_and_above() {
        let mut e = Editor::from_str("one\ntwo");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('o'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "one\n\ntwo");

        let mut e = Editor::from_str("one\ntwo");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('O'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "\none\ntwo");
        assert_eq!(e.primary_head(), 0, "cursor sits on the new empty line");
    }

    #[test]
    fn replace_char_and_toggle_case() {
        let mut e = Editor::from_str("cat");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('r'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('b'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "bat");

        let mut e = Editor::from_str("cat");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('~'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "Cat");
    }

    #[test]
    fn capital_d_deletes_to_end_of_line() {
        let mut e = Editor::from_str("hello world\nnext");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('l'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('l'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('D'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "he\nnext");
    }

    #[test]
    fn yy_then_capital_p_pastes_the_line_above() {
        let mut e = Editor::from_str("hello");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('y'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('y'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('P'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "hello\nhello");
    }

    #[test]
    fn d0_deletes_from_line_start_to_cursor() {
        let mut e = Editor::from_str("hello world");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('l'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('l'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('d'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('0'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "lo world");
        assert_eq!(e.primary_head(), 0);
    }

    #[test]
    fn d_capital_g_deletes_from_cursor_to_buffer_end() {
        let mut e = Editor::from_str("foo\nbar\nbaz");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('j'), &e) { e.execute(a); } // cursor -> line 1
        for a in v.handle(KeyEvent::Char('d'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('G'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "foo\n");
    }

    #[test]
    fn y_capital_g_yanks_from_cursor_to_buffer_end() {
        let mut e = Editor::from_str("foo\nbar\nbaz");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('y'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('G'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "foo\nbar\nbaz");
        // `yG` is line-wise: `p` puts the three lines after the current one.
        for a in v.handle(KeyEvent::Char('p'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "foo\nfoo\nbar\nbaz\nbar\nbaz");
    }

    #[test]
    fn c0_deletes_to_line_start_and_enters_insert() {
        let mut e = Editor::from_str("hello world");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('l'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('l'), &e) { e.execute(a); }
        // cursor at 2; c0 deletes 0..3 → "lo world", enters Insert
        for a in v.handle(KeyEvent::Char('c'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('0'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "lo world");
        assert_eq!(v.mode, VimMode::Insert);
        for a in v.handle(KeyEvent::Char('H'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "Hlo world");
    }

    #[test]
    fn double_angle_indent_line() {
        let mut e = Editor::from_str("hello");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('>'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('>'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "    hello");
    }

    #[test]
    fn cw_changes_word_and_enters_insert() {
        let mut e = Editor::from_str("hello world");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('c'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('w'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "world");
        assert_eq!(v.mode, VimMode::Insert);
        for a in v.handle(KeyEvent::Char('H'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "Hworld");
    }
}