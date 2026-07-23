use crate::buffer::Buffer;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub anchor: usize,
    pub head: usize,
}

impl Range {
    pub fn caret(at: usize) -> Self { Range { anchor: at, head: at } }
    pub fn start(&self) -> usize { self.anchor.min(self.head) }
    pub fn end(&self) -> usize { self.anchor.max(self.head) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge { Start, End }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dir { Left, Right }

#[derive(Clone)]
pub struct CursorSet {
    pub(crate) cursors: Vec<Range>,
    pub(crate) primary: usize,
    pub(crate) desired_col: usize,
}

impl CursorSet {
    pub fn single(at: usize) -> Self {
        CursorSet { cursors: vec![Range::caret(at)], primary: 0, desired_col: usize::MAX }
    }

    pub fn primary(&self) -> Range { self.cursors[self.primary] }
    pub fn head(&self) -> usize { self.primary().head }

    pub fn set_head(&mut self, at: usize, buffer: &Buffer) {
        let anchor = self.cursors[self.primary].anchor;
        self.cursors[self.primary] = Range { anchor, head: at };
        let line = self.line_of(buffer, at);
        self.desired_col = at - buffer.line_start_char(line);
        self.collapse_at(at);
    }

    /// In visual mode: set the cursor's `anchor` to `anchor` while preserving the current `head`.
    /// This lets the head move freely (extending the selection) while the anchor stays fixed.
    pub fn set_visual_anchor(&mut self, anchor: usize) {
        let head = self.cursors[self.primary].head;
        self.cursors[self.primary] = Range { anchor, head };
    }

    fn collapse_at(&mut self, at: usize) {
        self.cursors[self.primary] = Range::caret(at);
    }

    fn line_of(&self, buffer: &Buffer, char_idx: usize) -> usize {
        let mut acc = 0usize;
        for line in 0..buffer.line_count() {
            let start = buffer.line_start_char(line);
            if start <= char_idx { acc = line; } else { break; }
        }
        acc
    }

    fn line_content_len(&self, buffer: &Buffer, line: usize) -> usize {
        let end = buffer.line_end_char(line);
        let start = buffer.line_start_char(line);
        if end > start && buffer.char_at(end - 1) == '\n' {
            end - start - 1
        } else {
            end - start
        }
    }

    pub fn add_cursor(&mut self, at: usize) {
        self.cursors.push(Range::caret(at));
        self.primary = self.cursors.len() - 1;
    }

    pub fn clear_extra(&mut self) {
        let original = self.cursors[0];
        self.cursors.truncate(0);
        self.cursors.push(original);
        self.primary = 0;
    }

    pub fn count(&self) -> usize {
        self.cursors.len()
    }

    fn grapheme_step(&self, buffer: &Buffer, from: usize, dir: Dir) -> usize {
        let text = buffer.to_string();
        let graphemes: Vec<&str> = UnicodeSegmentation::graphemes(&*text, true).collect();
        let mut char_pos = 0usize;
        let mut gidx = 0usize;
        for (i, g) in graphemes.iter().enumerate() {
            if char_pos == from { gidx = i; break; }
            char_pos += g.chars().count();
            gidx = i + 1;
        }
        match dir {
            Dir::Left => {
                if gidx == 0 { from } else {
                    let prev = graphemes[gidx - 1];
                    from - prev.chars().count()
                }
            }
            Dir::Right => {
                if gidx >= graphemes.len() { from } else {
                    let cur = graphemes[gidx];
                    from + cur.chars().count()
                }
            }
        }
    }

    pub fn move_grapheme(&mut self, buffer: &Buffer, dir: i32) {
        let d = if dir > 0 { Dir::Right } else { Dir::Left };
        let from = self.head();
        let to = self.grapheme_step(buffer, from, d);
        self.set_head(to, buffer);
    }

    pub fn move_line(&mut self, buffer: &Buffer, delta: i32) {
        let from = self.head();
        let line = self.line_of(buffer, from);
        if self.desired_col == usize::MAX {
            self.desired_col = from - buffer.line_start_char(line);
        }
        let target_line = (line as i32 + delta).max(0) as usize;
        let last = buffer.line_count().saturating_sub(1);
        let target_line = target_line.min(last);
        let start = buffer.line_start_char(target_line);
        let content_len = self.line_content_len(buffer, target_line);
        let col = self.desired_col.min(content_len);
        let new_head = start + col;
        let anchor = self.cursors[self.primary].anchor;
        self.cursors[self.primary] = Range { anchor, head: new_head };
        self.collapse_at(new_head);
    }

    pub fn move_line_edge(&mut self, buffer: &Buffer, edge: Edge) {
        let from = self.head();
        let line = self.line_of(buffer, from);
        let at = match edge {
            Edge::Start => buffer.line_start_char(line),
            Edge::End => buffer.line_start_char(line) + self.line_content_len(buffer, line),
        };
        self.set_head(at, buffer);
    }

    pub fn collapse(&mut self) {
        let h = self.head();
        self.cursors[self.primary] = Range::caret(h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_anchor_equals_head() {
        let c = CursorSet::single(3);
        assert_eq!(c.primary().anchor, 3);
        assert_eq!(c.head(), 3);
    }

    #[test]
    fn move_grapheme_right_skips_combining_mark() {
        let b = Buffer::from_str("e\u{0301}x"); // e + combining acute, then x; 3 chars total
        let mut c = CursorSet::single(0);
        c.move_grapheme(&b, 1);
        assert_eq!(c.head(), 2, "grapheme cluster boundary");
    }

    #[test]
    fn move_line_down_preserves_column_intent() {
        let b = Buffer::from_str("abc\ndefg\nhi");
        let mut c = CursorSet::single(1); // col 1 of line 0
        c.move_line(&b, 1);
        assert_eq!(c.head(), 5, "line 1 col 1 -> offset 5 ('e' in 'defg')");
    }

    #[test]
    fn move_line_down_clamps_short_line() {
        let b = Buffer::from_str("abcd\ne\nfg");
        let mut c = CursorSet::single(3); // col 3 of "abcd"
        c.move_line(&b, 1);
        assert_eq!(c.head(), 6, "line 'e' has only col 0 -> head at 6 (after 'e')");
        c.move_line(&b, 1);
        assert_eq!(c.head(), 9, "col 3 of 'fg' -> after 'g' (line is 2 chars)");
    }

    #[test]
    fn move_line_edge_to_end() {
        let b = Buffer::from_str("hello world");
        let mut c = CursorSet::single(0);
        c.move_line_edge(&b, Edge::End);
        assert_eq!(c.head(), 11);
    }

    #[test]
    fn add_and_clear_extra_cursors() {
        let mut c = CursorSet::single(5);
        assert_eq!(c.count(), 1);
        c.add_cursor(10);
        assert_eq!(c.count(), 2);
        c.clear_extra();
        assert_eq!(c.count(), 1);
        assert_eq!(c.head(), 5);
    }
}