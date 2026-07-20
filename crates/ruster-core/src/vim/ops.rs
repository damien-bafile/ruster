use crate::buffer::Buffer;
use crate::editor::Editor;
use crate::vim::motions::{next_word_start, prev_word_start, word_end, last_printable_in_line, char_to_line};

/// Compute the (start, end) char range for an operator (`d`/`y`/`c`) applied `count` times
/// to the named `motion`. Motions supported in the slice: `w`, `b`, `e`, `$`, `d` (line).
/// Returns `None` for any other motion (text objects come in Task 9).
pub fn range_for_motion(editor: &Editor, motion: char, count: u32) -> Option<(usize, usize)> {
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
    fn yy_then_p_yanks_and_pastes_at_cursor() {
        let mut e = Editor::from_str("hello");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('y'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('y'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "hello");
        for a in v.handle(KeyEvent::Char('p'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "hellohello");
        assert_eq!(e.primary_head(), 5);
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