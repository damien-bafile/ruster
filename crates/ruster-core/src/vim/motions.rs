use crate::buffer::Buffer;
use crate::editor::EditorView;

pub fn next_word_start(buffer: &Buffer, head: usize) -> usize {
    let total = buffer.len_chars();
    let mut i = head;
    while i < total {
        let c = buffer.char_at(i);
        if c.is_whitespace() { break; }
        i += 1;
    }
    while i < total {
        let c = buffer.char_at(i);
        if !c.is_whitespace() { break; }
        i += 1;
    }
    i
}

pub fn prev_word_start(buffer: &Buffer, head: usize) -> usize {
    let mut i = head.saturating_sub(1);
    while i > 0 {
        let c = buffer.char_at(i);
        if !c.is_whitespace() { break; }
        i -= 1;
    }
    while i > 0 {
        let c = buffer.char_at(i - 1);
        if c.is_whitespace() { break; }
        i -= 1;
    }
    i
}

pub fn word_end(buffer: &Buffer, head: usize) -> usize {
    let total = buffer.len_chars();
    let mut i = head + 1;
    while i < total {
        let c = buffer.char_at(i);
        if !c.is_whitespace() { break; }
        i += 1;
    }
    while i + 1 < total {
        let c = buffer.char_at(i + 1);
        if c.is_whitespace() { break; }
        i += 1;
    }
    i.min(total.saturating_sub(1))
}

pub fn last_printable_in_line(editor: &dyn EditorView) -> usize {
    let head = editor.primary_head();
    let line = char_to_line(editor, head);
    let start = editor.buffer().line_start_char(line);
    let content_len = editor.buffer().line_content_len(line);
    if content_len > 0 { start + content_len - 1 } else { start }
}

/// Position just past the last character on `line`, before any newline — where
/// `A` starts appending and where `$`-style operators stop.
pub fn line_content_end(editor: &dyn EditorView, line: usize) -> usize {
    let buf = editor.buffer();
    buf.line_start_char(line) + buf.line_content_len(line)
}

/// First non-blank character on `line` — where `I` starts inserting.
pub fn first_non_blank(editor: &dyn EditorView, line: usize) -> usize {
    let buf = editor.buffer();
    let start = buf.line_start_char(line);
    let end = line_content_end(editor, line);
    let mut i = start;
    while i < end && buf.char_at(i).is_whitespace() {
        i += 1;
    }
    i
}

/// Target position for `f`/`t`/`F`/`T` searching `target` on the cursor's line.
/// `f`/`t` search forward, `F`/`T` backward; `t`/`T` stop just short.
pub fn find_char_in_line(
    editor: &dyn EditorView,
    cmd: char,
    target: char,
    from: usize,
) -> Option<usize> {
    let buf = editor.buffer();
    let line = char_to_line(editor, from);
    let start = buf.line_start_char(line);
    let end = line_content_end(editor, line);
    match cmd {
        'f' | 't' => {
            let mut i = from + 1;
            while i < end {
                if buf.char_at(i) == target {
                    return Some(if cmd == 't' { i.saturating_sub(1) } else { i });
                }
                i += 1;
            }
            None
        }
        'F' | 'T' => {
            let mut i = from;
            while i > start {
                i -= 1;
                if buf.char_at(i) == target {
                    return Some(if cmd == 'T' { i + 1 } else { i });
                }
            }
            None
        }
        _ => None,
    }
}

/// The bracket matching the one at (or next on the line after) `from`.
pub fn matching_bracket(editor: &dyn EditorView, from: usize) -> Option<usize> {
    const PAIRS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];
    let buf = editor.buffer();
    let len = buf.len_chars();
    let line = char_to_line(editor, from);
    let line_end = line_content_end(editor, line);

    // Find the first bracket at or after the cursor, on this line.
    let mut at = None;
    let mut i = from;
    while i < line_end {
        let c = buf.char_at(i);
        if PAIRS.iter().any(|(o, cl)| *o == c || *cl == c) {
            at = Some(i);
            break;
        }
        i += 1;
    }
    let at = at?;
    let c = buf.char_at(at);

    if let Some((open, close)) = PAIRS.iter().find(|(o, _)| *o == c) {
        let mut depth = 0i32;
        let mut j = at;
        while j < len {
            let ch = buf.char_at(j);
            if ch == *open {
                depth += 1;
            } else if ch == *close {
                depth -= 1;
                if depth == 0 {
                    return Some(j);
                }
            }
            j += 1;
        }
        return None;
    }
    if let Some((open, close)) = PAIRS.iter().find(|(_, cl)| *cl == c) {
        let mut depth = 0i32;
        let mut j = at as i64;
        while j >= 0 {
            let ch = buf.char_at(j as usize);
            if ch == *close {
                depth += 1;
            } else if ch == *open {
                depth -= 1;
                if depth == 0 {
                    return Some(j as usize);
                }
            }
            j -= 1;
        }
    }
    None
}

pub fn char_to_line(editor: &dyn EditorView, char_idx: usize) -> usize {
    let mut acc = 0usize;
    for line in 0..editor.buffer().line_count() {
        if editor.buffer().line_start_char(line) <= char_idx { acc = line; } else { break; }
    }
    acc
}

#[cfg(test)]
mod tests {
    use crate::editor::Editor;
    use crate::key::KeyEvent;
    use crate::vim::VimMode;
    use crate::vim::VimState;

    fn run(src: &str, keys: &[KeyEvent], expect_head: usize) {
        let mut e = Editor::from_str(src);
        let mut v = VimState::new();
        for a in v.handle(KeyEvent::Char('g'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('g'), &e) { e.execute(a); }
        for k in keys {
            for action in v.handle(*k, &e) { e.execute(action); }
        }
        assert_eq!(e.primary_head(), expect_head, "after {:?} on {:?}", keys, src);
    }

    #[test]
    fn hljk_basic() {
        let s = "abc\ndef\nghi";
        run(s, &[KeyEvent::Char('l')], 1);
        run(s, &[KeyEvent::Char('l'), KeyEvent::Char('l')], 2);
        run(s, &[KeyEvent::Char('j')], 4);
        run(s, &[KeyEvent::Char('l'), KeyEvent::Char('j')], 5);
        run(s, &[KeyEvent::Char('l'), KeyEvent::Char('j'), KeyEvent::Char('h')], 4);
        run(s, &[KeyEvent::Char('l'), KeyEvent::Char('j'), KeyEvent::Char('k')], 1);
    }

    #[test]
    fn wbe_word_motions() {
        run("hello world", &[KeyEvent::Char('w')], 6);
        run("hello world", &[KeyEvent::Char('w'), KeyEvent::Char('b')], 0);
        run("hello world", &[KeyEvent::Char('e')], 4);
        run("hello world", &[KeyEvent::Char('w'), KeyEvent::Char('e')], 10);
    }

    #[test]
    fn zero_dollar() {
        run("abc def", &[KeyEvent::Char('$')], 6);
        run("abc def", &[KeyEvent::Char('$'), KeyEvent::Char('0')], 0);
    }

    #[test]
    fn gg_g() {
        run("abc\ndef\nghi", &[KeyEvent::Char('g'), KeyEvent::Char('g')], 0);
        let mut e = Editor::from_str("abc\ndef\nghi");
        let mut v = VimState::new();
        for a in v.handle(KeyEvent::Char('g'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('g'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('G'), &e) { e.execute(a); }
        assert_eq!(e.primary_head(), 8);
    }

    #[test]
    fn count_prefix() {
        run("hello world", &[KeyEvent::Char('3'), KeyEvent::Char('l')], 3);
        run("hello world", &[KeyEvent::Char('2'), KeyEvent::Char('w')], 11);
    }

    #[test]
    fn i_esc_insert() {
        let mut e = Editor::from_str("ab");
        let mut v = VimState::new();
        for a in v.handle(KeyEvent::Char('g'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('g'), &e) { e.execute(a); }
        for a in v.handle(KeyEvent::Char('i'), &e) { e.execute(a); }
        assert_eq!(v.mode, VimMode::Insert);
        for a in v.handle(KeyEvent::Char('x'), &e) { e.execute(a); }
        assert_eq!(e.buffer().to_string(), "xab");
        for a in v.handle(KeyEvent::Esc, &e) { e.execute(a); }
        assert_eq!(v.mode, VimMode::Normal);
        assert_eq!(e.primary_head(), 0);
    }
}