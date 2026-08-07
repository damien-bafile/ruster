use crate::buffer::Buffer;
use crate::editor::EditorView;

/// Compute the (start, end) char range for a text object of `kind` ('i' inner / 'a' around)
/// for the named `target` ('w', '"', '\'', '(', ')', '{', '}').
pub fn range_for_textobj(
    kind: char,
    target: char,
    editor: &dyn EditorView,
) -> Option<(usize, usize)> {
    let head = editor.primary_head();
    let buf = editor.buffer();
    match target {
        'w' => match kind {
            'i' => inner_word(buf, head),
            'a' => around_word(buf, head),
            _ => None,
        },
        '"' => match kind {
            'i' => inner_pair(buf, head, '"', '"'),
            'a' => around_pair(buf, head, '"', '"'),
            _ => None,
        },
        '\'' => match kind {
            'i' => inner_pair(buf, head, '\'', '\''),
            'a' => around_pair(buf, head, '\'', '\''),
            _ => None,
        },
        '(' | ')' => match kind {
            'i' => inner_pair(buf, head, '(', ')'),
            'a' => around_pair(buf, head, '(', ')'),
            _ => None,
        },
        '{' | '}' => match kind {
            'i' => inner_pair(buf, head, '{', '}'),
            'a' => around_pair(buf, head, '{', '}'),
            _ => None,
        },
        _ => None,
    }
}

pub fn inner_word(buffer: &Buffer, head: usize) -> Option<(usize, usize)> {
    let total = buffer.len_chars();
    if head >= total {
        return None;
    }
    let start_char = buffer.char_at(head);
    let is_ws = start_char.is_whitespace();
    let mut s = head;
    let mut e = head;
    if is_ws {
        while s > 0 && buffer.char_at(s - 1).is_whitespace() {
            s -= 1;
        }
        while e < total && buffer.char_at(e).is_whitespace() {
            e += 1;
        }
    } else {
        while s > 0 && !buffer.char_at(s - 1).is_whitespace() {
            s -= 1;
        }
        while e < total && !buffer.char_at(e).is_whitespace() {
            e += 1;
        }
    }
    Some((s, e))
}

pub fn around_word(buffer: &Buffer, head: usize) -> Option<(usize, usize)> {
    let (s, e) = inner_word(buffer, head)?;
    let total = buffer.len_chars();
    let mut s2 = s;
    let mut e2 = e;
    if e2 < total && buffer.char_at(e2).is_whitespace() {
        e2 += 1;
    } else if s2 > 0 && buffer.char_at(s2 - 1).is_whitespace() {
        s2 -= 1;
    }
    Some((s2, e2))
}

fn find_enclosing_open(buffer: &Buffer, head: usize, open: char, close: char) -> Option<usize> {
    if head < buffer.len_chars() && buffer.char_at(head) == open {
        return Some(head);
    }
    if open == close {
        let mut i = head;
        while i > 0 {
            i -= 1;
            if buffer.char_at(i) == open {
                return Some(i);
            }
        }
        return None;
    }
    let mut i = head;
    let mut depth = 0i32;
    while i > 0 {
        i -= 1;
        let c = buffer.char_at(i);
        if c == close {
            depth += 1;
        } else if c == open {
            if depth == 0 {
                return Some(i);
            }
            depth -= 1;
        }
    }
    None
}

fn find_matching_close(buffer: &Buffer, open_idx: usize, open: char, close: char) -> Option<usize> {
    let total = buffer.len_chars();
    if open == close {
        let mut i = open_idx + 1;
        while i < total {
            if buffer.char_at(i) == close {
                return Some(i);
            }
            i += 1;
        }
        return None;
    }
    let mut depth = 0i32;
    let mut i = open_idx;
    while i < total {
        let c = buffer.char_at(i);
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

pub fn inner_pair(buffer: &Buffer, head: usize, open: char, close: char) -> Option<(usize, usize)> {
    let open_idx = find_enclosing_open(buffer, head, open, close)?;
    let close_idx = find_matching_close(buffer, open_idx, open, close)?;
    Some((open_idx + 1, close_idx))
}

pub fn around_pair(
    buffer: &Buffer,
    head: usize,
    open: char,
    close: char,
) -> Option<(usize, usize)> {
    let open_idx = find_enclosing_open(buffer, head, open, close)?;
    let close_idx = find_matching_close(buffer, open_idx, open, close)?;
    Some((open_idx, close_idx + 1))
}

#[cfg(test)]
mod tests {
    use crate::editor::Editor;
    use crate::key::KeyEvent;
    use crate::vim::VimState;

    fn to_start(e: &mut Editor, v: &mut VimState) {
        for a in v.handle(KeyEvent::Char('g'), e) {
            e.execute(a);
        }
        for a in v.handle(KeyEvent::Char('g'), e) {
            e.execute(a);
        }
    }

    fn l(e: &mut Editor, v: &mut VimState, n: usize) {
        for _ in 0..n {
            for a in v.handle(KeyEvent::Char('l'), e) {
                e.execute(a);
            }
        }
    }

    #[test]
    fn diw_deletes_inner_word_at_cursor() {
        let mut e = Editor::from_str("hello world");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('w'), &e) {
            e.execute(a);
        }
        for a in v.handle(KeyEvent::Char('d'), &e) {
            e.execute(a);
        }
        for a in v.handle(KeyEvent::Char('i'), &e) {
            e.execute(a);
        }
        for a in v.handle(KeyEvent::Char('w'), &e) {
            e.execute(a);
        }
        assert_eq!(e.buffer().to_string(), "hello ");
    }

    #[test]
    fn daw_deletes_around_word_with_leading_space() {
        let mut e = Editor::from_str("hello world");
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        for a in v.handle(KeyEvent::Char('w'), &e) {
            e.execute(a);
        }
        for a in v.handle(KeyEvent::Char('d'), &e) {
            e.execute(a);
        }
        for a in v.handle(KeyEvent::Char('a'), &e) {
            e.execute(a);
        }
        for a in v.handle(KeyEvent::Char('w'), &e) {
            e.execute(a);
        }
        assert_eq!(e.buffer().to_string(), "hello");
    }

    #[test]
    fn di_quote_deletes_inner_quotes() {
        let src = "say \"hi\" loudly";
        let mut e = Editor::from_str(src);
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        l(&mut e, &mut v, 5);
        for a in v.handle(KeyEvent::Char('d'), &e) {
            e.execute(a);
        }
        for a in v.handle(KeyEvent::Char('i'), &e) {
            e.execute(a);
        }
        for a in v.handle(KeyEvent::Char('"'), &e) {
            e.execute(a);
        }
        assert_eq!(e.buffer().to_string(), "say \"\" loudly");
    }

    #[test]
    fn da_paren_deletes_around_parens() {
        let src = "f(x) -> y";
        let mut e = Editor::from_str(src);
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        l(&mut e, &mut v, 1);
        for a in v.handle(KeyEvent::Char('d'), &e) {
            e.execute(a);
        }
        for a in v.handle(KeyEvent::Char('a'), &e) {
            e.execute(a);
        }
        for a in v.handle(KeyEvent::Char('('), &e) {
            e.execute(a);
        }
        assert_eq!(e.buffer().to_string(), "f -> y");
    }

    #[test]
    fn ci_quote_changes_inner_text_to_insert() {
        let src = "say \"hi\" loudly";
        let mut e = Editor::from_str(src);
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        l(&mut e, &mut v, 5);
        for a in v.handle(KeyEvent::Char('c'), &e) {
            e.execute(a);
        }
        for a in v.handle(KeyEvent::Char('i'), &e) {
            e.execute(a);
        }
        for a in v.handle(KeyEvent::Char('"'), &e) {
            e.execute(a);
        }
        assert_eq!(e.buffer().to_string(), "say \"\" loudly");
        assert_eq!(v.mode, crate::vim::VimMode::Insert);
        for a in v.handle(KeyEvent::Char('X'), &e) {
            e.execute(a);
        }
        assert_eq!(e.buffer().to_string(), "say \"X\" loudly");
    }

    #[test]
    fn nested_parens_around_inner() {
        let src = "(a(b)c)";
        let mut e = Editor::from_str(src);
        let mut v = VimState::new();
        to_start(&mut e, &mut v);
        l(&mut e, &mut v, 3);
        for a in v.handle(KeyEvent::Char('d'), &e) {
            e.execute(a);
        }
        for a in v.handle(KeyEvent::Char('a'), &e) {
            e.execute(a);
        }
        for a in v.handle(KeyEvent::Char('('), &e) {
            e.execute(a);
        }
        assert_eq!(e.buffer().to_string(), "(ac)");
    }
}
