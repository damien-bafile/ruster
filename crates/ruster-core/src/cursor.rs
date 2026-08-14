use crate::buffer::Buffer;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub anchor: usize,
    pub head: usize,
}

impl Range {
    pub fn caret(at: usize) -> Self {
        Range {
            anchor: at,
            head: at,
        }
    }
    pub fn start(&self) -> usize {
        self.anchor.min(self.head)
    }
    pub fn end(&self) -> usize {
        self.anchor.max(self.head)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Start,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dir {
    Left,
    Right,
}

#[derive(Clone)]
pub struct CursorSet {
    pub(crate) cursors: Vec<Range>,
    pub(crate) primary: usize,
    pub(crate) desired_col: usize,
}

impl CursorSet {
    pub fn single(at: usize) -> Self {
        CursorSet {
            cursors: vec![Range::caret(at)],
            primary: 0,
            desired_col: usize::MAX,
        }
    }

    pub fn primary(&self) -> Range {
        self.cursors[self.primary]
    }
    pub fn head(&self) -> usize {
        self.primary().head
    }

    pub fn set_head(&mut self, at: usize, buffer: &Buffer) {
        let anchor = self.cursors[self.primary].anchor;
        self.cursors[self.primary] = Range { anchor, head: at };
        let line = buffer.char_to_line(at);
        self.desired_col = at - buffer.line_start_char(line);
        self.collapse_at(at);
    }

    /// In visual mode: set the cursor's `anchor` to `anchor` while preserving the current `head`.
    /// This lets the head move freely (extending the selection) while the anchor stays fixed.
    pub fn set_visual_anchor(&mut self, anchor: usize) {
        let head = self.cursors[self.primary].head;
        self.cursors[self.primary] = Range { anchor, head };
    }

    /// Set the primary cursor to span `[mark, point]` — Emacs' region, where
    /// the mark is the fixed end and point is the end that moves.
    pub fn set_region(&mut self, mark: usize, point: usize) {
        self.cursors[self.primary] = Range {
            anchor: mark,
            head: point,
        };
    }

    /// The word around `anchor`, expanded outward until whitespace.
    ///
    /// "Word" here is deliberately whitespace-delimited rather than
    /// punctuation-aware: a double-click on `foo.bar` selects the whole thing,
    /// which is what a path or a qualified name wants.
    ///
    /// An `anchor` sitting on whitespace selects the run of whitespace itself,
    /// so the result is never empty for a non-empty buffer.
    pub fn select_word(&self, buffer: &Buffer, anchor: usize) -> Range {
        let len = buffer.len_chars();
        if len == 0 {
            return Range::caret(0);
        }
        let at = anchor.min(len - 1);
        // Match like against like, so a click on a gap grabs the gap.
        let on_space = buffer.char_at(at).is_whitespace();
        let same = |i: usize| buffer.char_at(i).is_whitespace() == on_space;

        let mut start = at;
        while start > 0 && same(start - 1) {
            start -= 1;
        }
        let mut end = at;
        while end + 1 < len && same(end + 1) {
            end += 1;
        }
        Range {
            anchor: start,
            head: end + 1,
        }
    }

    /// The whole line around `anchor`, including its trailing newline so that
    /// a triple-click selects something that can be cut as a line.
    pub fn select_line(&self, buffer: &Buffer, anchor: usize) -> Range {
        let len = buffer.len_chars();
        let at = anchor.min(len);
        let line = buffer.char_to_line(at);
        let start = buffer.line_start_char(line);
        let end = if line + 1 < buffer.line_count() {
            buffer.line_start_char(line + 1)
        } else {
            len
        };
        Range {
            anchor: start,
            head: end,
        }
    }

    fn collapse_at(&mut self, at: usize) {
        self.cursors[self.primary] = Range::caret(at);
    }

    pub fn add_cursor(&mut self, at: usize) {
        self.cursors.push(Range::caret(at));
        self.primary = self.cursors.len() - 1;
    }

    /// Clamp every cursor so no anchor/head exceeds `max` (typically the new
    /// buffer's `len_chars`). Used when a window switches to a shorter buffer so
    /// stale positions can't index out of bounds. `max` itself is a valid index
    /// (one past the last char), so it is not decremented.
    pub fn clamp_to(&mut self, max: usize) {
        for c in &mut self.cursors {
            c.anchor = c.anchor.min(max);
            c.head = c.head.min(max);
        }
        self.desired_col = self.desired_col.min(max);
    }

    /// Collapse cursors that have landed on the same position into one, keeping
    /// whichever the primary pointed at. Editing can push two carets together
    /// (e.g. adjacent occurrences), and duplicates would then edit in lockstep.
    pub fn merge_overlaps(&mut self) {
        if self.cursors.len() <= 1 {
            return;
        }
        let primary_head = self.cursors[self.primary].head;
        let mut seen = std::collections::HashSet::new();
        self.cursors.retain(|r| seen.insert(r.head));
        self.primary = self
            .cursors
            .iter()
            .position(|r| r.head == primary_head)
            .unwrap_or(0);
    }

    pub fn clear_extra(&mut self) {
        let original = self.cursors[0];
        self.cursors.clear();
        self.cursors.push(original);
        self.primary = 0;
    }

    pub fn count(&self) -> usize {
        self.cursors.len()
    }

    /// Head offset of every cursor, primary included, in storage order.
    pub fn iter_heads(&self) -> impl Iterator<Item = usize> + '_ {
        self.cursors.iter().map(|r| r.head)
    }

    /// Step one grapheme cluster left or right of `from`, clamped at the ends of
    /// the buffer.
    ///
    /// Only the cursor's own line is segmented, which keeps the cost independent
    /// of buffer size. That is equivalent to segmenting the whole buffer because
    /// a cluster never spans a `\n`: the in-memory buffer is always
    /// LF-normalized (see [`crate::document::Document`]), so `\r\n` — the one
    /// cluster that contains a newline — cannot occur.
    fn grapheme_step(&self, buffer: &Buffer, from: usize, dir: Dir) -> usize {
        let from = from.min(buffer.len_chars());
        // Crossing a line boundary always steps over exactly one `\n`, so the
        // neighbouring line never has to be segmented.
        if dir == Dir::Left {
            if from == 0 {
                return 0;
            }
            if from == buffer.line_start_char(buffer.char_to_line(from)) {
                return from - 1;
            }
        }

        let line = buffer.char_to_line(from);
        let line_start = buffer.line_start_char(line);
        // Includes the line's trailing `\n`, so stepping right off the last
        // character lands on the next line's first one.
        let text = buffer.slice_string(line_start, buffer.line_end_char(line));

        let mut pos = line_start;
        let mut prev_start = None;
        for g in UnicodeSegmentation::graphemes(text.as_str(), true) {
            let end = pos + g.chars().count();
            if from < end {
                // `from` is at this cluster's start, or inside it — in which
                // case snap to the enclosing boundary rather than landing
                // mid-cluster again.
                return match dir {
                    Dir::Right => end,
                    Dir::Left if from == pos => prev_start.unwrap_or(pos),
                    Dir::Left => pos,
                };
            }
            prev_start = Some(pos);
            pos = end;
        }
        // `from` is past the last cluster — the end of a line with no trailing
        // newline, i.e. the end of the buffer. Right is a fixed point; Left
        // still has the final cluster to step back over.
        match dir {
            Dir::Right => from,
            Dir::Left => prev_start.unwrap_or(from),
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
        let line = buffer.char_to_line(from);
        if self.desired_col == usize::MAX {
            self.desired_col = from - buffer.line_start_char(line);
        }
        let target_line = (line as i32 + delta).max(0) as usize;
        let last = buffer.line_count().saturating_sub(1);
        let target_line = target_line.min(last);
        let start = buffer.line_start_char(target_line);
        let content_len = buffer.line_content_len(target_line);
        let col = self.desired_col.min(content_len);
        let new_head = start + col;
        let anchor = self.cursors[self.primary].anchor;
        self.cursors[self.primary] = Range {
            anchor,
            head: new_head,
        };
        self.collapse_at(new_head);
    }

    pub fn move_line_edge(&mut self, buffer: &Buffer, edge: Edge) {
        let from = self.head();
        let line = buffer.char_to_line(from);
        let at = match edge {
            Edge::Start => buffer.line_start_char(line),
            Edge::End => buffer.line_start_char(line) + buffer.line_content_len(line),
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
    fn select_word_expands_to_whitespace() {
        let b = Buffer::from_str("foo bar");
        let cs = CursorSet::single(0);
        // Inside the first word, from either end or the middle.
        for at in [0, 1, 2] {
            let r = cs.select_word(&b, at);
            assert_eq!((r.start(), r.end()), (0, 3), "offset {at}");
        }
        // Inside the second word.
        for at in [4, 5, 6] {
            let r = cs.select_word(&b, at);
            assert_eq!((r.start(), r.end()), (4, 7), "offset {at}");
        }
        // On the gap: the gap itself, so the selection is never empty.
        let r = cs.select_word(&b, 3);
        assert_eq!((r.start(), r.end()), (3, 4));
    }

    #[test]
    fn select_word_handles_an_empty_buffer() {
        let b = Buffer::from_str("");
        let cs = CursorSet::single(0);
        let r = cs.select_word(&b, 0);
        assert_eq!((r.start(), r.end()), (0, 0));
    }

    /// Punctuation stays inside the word, so a click on a path or a qualified
    /// name grabs all of it.
    #[test]
    fn select_word_keeps_punctuation_inside_the_word() {
        let b = Buffer::from_str("use foo.bar::baz end");
        let cs = CursorSet::single(0);
        let r = cs.select_word(&b, 8);
        assert_eq!(b.slice_string(r.start(), r.end()), "foo.bar::baz");
    }

    #[test]
    fn select_line_spans_the_line_and_its_newline() {
        let b = Buffer::from_str("a\nbc\nde");
        let cs = CursorSet::single(0);
        // "a\n"
        let r = cs.select_line(&b, 0);
        assert_eq!((r.start(), r.end()), (0, 2));
        // "bc\n" — offset 3 is the 'c'.
        let r = cs.select_line(&b, 3);
        assert_eq!((r.start(), r.end()), (2, 5));
        // "de", the last line, which has no trailing newline.
        let r = cs.select_line(&b, 6);
        assert_eq!((r.start(), r.end()), (5, 7));
    }

    #[test]
    fn set_region_fixes_the_mark_and_moves_point() {
        let b = Buffer::from_str("hello world");
        let mut cs = CursorSet::single(0);
        cs.set_region(2, 7);
        assert_eq!(cs.primary().anchor, 2);
        assert_eq!(cs.primary().head, 7);
        assert_eq!(
            b.slice_string(cs.primary().start(), cs.primary().end()),
            "llo w"
        );

        // Point can move behind the mark; the mark does not follow.
        cs.set_region(2, 0);
        assert_eq!(cs.primary().anchor, 2);
        assert_eq!((cs.primary().start(), cs.primary().end()), (0, 2));
    }

    /// Buffers that exercise the awkward cases for offset<->line mapping:
    /// empty, no trailing newline, bare and repeated newlines, multi-byte
    /// scalars, and grapheme clusters wider than one char.
    const EDGE_CASE_BUFFERS: &[&str] = &[
        "",
        "a",
        "a\n",
        "\n",
        "\n\n\n",
        "abc\ndef\n",
        "abc\ndef",
        "héllo\nwörld\n",
        "a\n\nb\n\n\nc",
        "tab\there\ntrailing spaces   \n\n",
        "e\u{0301}x\ncafe\u{0301}\n",
        "👨‍👩‍👧 family\n🎉\n",
    ];

    /// The linear scan `CursorSet::line_of` used to perform, kept as a reference
    /// so the rope-backed `Buffer::char_to_line` that replaced it stays honest.
    fn line_of_by_scan(buffer: &Buffer, char_idx: usize) -> usize {
        let mut acc = 0usize;
        for line in 0..buffer.line_count() {
            if buffer.line_start_char(line) <= char_idx {
                acc = line;
            } else {
                break;
            }
        }
        acc
    }

    /// The whole-buffer segmentation `grapheme_step` used to perform, kept as a
    /// reference for the per-line rewrite.
    fn grapheme_step_whole_buffer(buffer: &Buffer, from: usize, dir: Dir) -> usize {
        let text = buffer.to_string();
        let graphemes: Vec<&str> = UnicodeSegmentation::graphemes(&*text, true).collect();
        let mut char_pos = 0usize;
        let mut gidx = 0usize;
        for (i, g) in graphemes.iter().enumerate() {
            if char_pos == from {
                gidx = i;
                break;
            }
            char_pos += g.chars().count();
            gidx = i + 1;
        }
        match dir {
            Dir::Left => {
                if gidx == 0 {
                    from
                } else {
                    from - graphemes[gidx - 1].chars().count()
                }
            }
            Dir::Right => {
                if gidx >= graphemes.len() {
                    from
                } else {
                    from + graphemes[gidx].chars().count()
                }
            }
        }
    }

    /// Every offset that sits on a grapheme boundary — i.e. every position a
    /// cursor can actually hold.
    fn boundary_offsets(content: &str) -> Vec<usize> {
        let mut out = vec![0];
        let mut pos = 0;
        for g in UnicodeSegmentation::graphemes(content, true) {
            pos += g.chars().count();
            out.push(pos);
        }
        out
    }

    #[test]
    fn grapheme_step_matches_whole_buffer_segmentation() {
        let c = CursorSet::single(0);
        for content in EDGE_CASE_BUFFERS {
            let buf = Buffer::from_str(content);
            for from in boundary_offsets(content) {
                for dir in [Dir::Left, Dir::Right] {
                    assert_eq!(
                        c.grapheme_step(&buf, from, dir),
                        grapheme_step_whole_buffer(&buf, from, dir),
                        "{dir:?} from offset {from} of {content:?}"
                    );
                }
            }
        }
    }

    /// A head left mid-cluster (e.g. by a clamp or an external jump) snaps to the
    /// enclosing boundary. The old whole-buffer version instead subtracted the
    /// *buffer's* last cluster width, which was meaningless; this is deliberately
    /// the one place the rewrite does not reproduce it.
    #[test]
    fn grapheme_step_snaps_out_of_a_cluster() {
        let c = CursorSet::single(0);
        // "e" + combining acute is one cluster spanning offsets 0..2.
        let b = Buffer::from_str("e\u{0301}x");
        assert_eq!(
            c.grapheme_step(&b, 1, Dir::Left),
            0,
            "snap back to cluster start"
        );
        assert_eq!(
            c.grapheme_step(&b, 1, Dir::Right),
            2,
            "snap forward to cluster end"
        );
    }

    #[test]
    fn grapheme_step_crosses_line_boundaries() {
        let c = CursorSet::single(0);
        let b = Buffer::from_str("ab\ncd");
        // Right off the last char of line 0 lands on its newline, then line 1.
        assert_eq!(c.grapheme_step(&b, 1, Dir::Right), 2);
        assert_eq!(c.grapheme_step(&b, 2, Dir::Right), 3);
        // Left from the start of line 1 steps back onto the newline.
        assert_eq!(c.grapheme_step(&b, 3, Dir::Left), 2);
        // Both ends of the buffer are fixed points.
        assert_eq!(c.grapheme_step(&b, 0, Dir::Left), 0);
        assert_eq!(c.grapheme_step(&b, 5, Dir::Right), 5);
    }

    #[test]
    fn grapheme_step_treats_zwj_emoji_as_one_cluster() {
        let c = CursorSet::single(0);
        let b = Buffer::from_str("👨‍👩‍👧x");
        // Three emoji joined by two ZWJs: five chars, one cluster.
        assert_eq!(c.grapheme_step(&b, 0, Dir::Right), 5);
        assert_eq!(c.grapheme_step(&b, 5, Dir::Left), 0);
    }

    #[test]
    fn char_to_line_matches_linear_scan_at_every_offset() {
        for content in EDGE_CASE_BUFFERS {
            let buf = Buffer::from_str(content);
            for idx in 0..=buf.len_chars() {
                assert_eq!(
                    buf.char_to_line(idx),
                    line_of_by_scan(&buf, idx),
                    "offset {idx} of {content:?}"
                );
            }
        }
    }

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
        assert_eq!(
            c.head(),
            6,
            "line 'e' has only col 0 -> head at 6 (after 'e')"
        );
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
