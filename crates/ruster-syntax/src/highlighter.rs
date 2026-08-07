use crate::theme::{set_current_lang, style_for_capture, RAINBOW_PALETTE};
use ruster_render::{Color, StyledLine, SyntaxStyle};
use streaming_iterator::StreamingIterator;

pub struct Highlighter {
    query: tree_sitter::Query,
    cursor: tree_sitter::QueryCursor,
    /// Byte ranges of `@comment` captures from the last highlight pass.
    ///
    /// The TODO scan wants exactly these, and the highlight pass has already
    /// walked the tree to find them. Running the same query a second time to
    /// rediscover them was the bulk of that scan's cost.
    comments: Vec<(usize, usize)>,
    /// Canonical language key, so per-language overrides resolve.
    lang: String,
}

impl Highlighter {
    /// Comment ranges seen by the last [`highlight_lines`](Self::highlight_lines).
    pub fn comments(&self) -> &[(usize, usize)] {
        &self.comments
    }

    /// The compiled highlight query.
    ///
    /// Exposed so `todo_markers` can reuse it: it needs the same `@comment`
    /// captures, and compiling the query source again — which it used to do on
    /// every call — costs more than the scan it performs.
    pub fn query(&self) -> &tree_sitter::Query {
        &self.query
    }

    pub fn new(
        language: tree_sitter::Language,
        query_source: &str,
        lang: &str,
    ) -> Result<Self, String> {
        let query = tree_sitter::Query::new(&language, query_source)
            .map_err(|e| format!("query error: {}", e))?;
        Ok(Highlighter {
            query,
            cursor: tree_sitter::QueryCursor::new(),
            comments: Vec::new(),
            lang: lang.to_string(),
        })
    }

    /// Every `@comment` range in the tree, ignoring any viewport.
    ///
    /// [`comments`](Self::comments) holds only what the last highlight pass
    /// looked at, which is the visible rows. A whole-file list — the TODO
    /// panel — has to pay for the full query.
    pub fn comments_in(&mut self, tree: &tree_sitter::Tree, source: &str) -> Vec<(usize, usize)> {
        let ids: Vec<u32> = self
            .query
            .capture_names()
            .iter()
            .enumerate()
            .filter(|(_, n)| **n == "comment")
            .map(|(i, _)| i as u32)
            .collect();
        if ids.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        self.cursor.set_byte_range(0..usize::MAX);
        let mut caps = self
            .cursor
            .captures(&self.query, tree.root_node(), source.as_bytes());
        while let Some((m, _)) = caps.next() {
            for c in m.captures {
                if ids.contains(&c.index) {
                    let r = c.node.byte_range();
                    out.push((r.start, r.end));
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    pub fn highlight_lines(
        &mut self,
        tree: &tree_sitter::Tree,
        source: &str,
        rainbow: &[Option<usize>],
        lines: Option<std::ops::Range<usize>>,
    ) -> Vec<StyledLine> {
        // Resolve this pass's per-language color overrides.
        set_current_lang(&self.lang);
        let bytes = source.as_bytes();
        let mut line_starts: Vec<usize> = vec![0];
        for (i, ch) in source.char_indices() {
            if ch == '\n' {
                line_starts.push(i + 1);
            }
        }
        line_starts.push(bytes.len());

        let mut per_line: Vec<Vec<(usize, usize, SyntaxStyle)>> =
            (0..line_starts.len() - 1).map(|_| Vec::new()).collect();

        // Resolve each capture *name* to a style once, then index by capture id.
        //
        // This used to allocate a `String` per capture and call
        // `style_for_capture` per capture — which takes an `RwLock` read to
        // consult the override map. On a 10k-line file that is tens of
        // thousands of allocations and lock acquisitions to produce at most a
        // couple of dozen distinct styles.
        let styles: Vec<SyntaxStyle> = self
            .query
            .capture_names()
            .iter()
            .map(|n| style_for_capture(n))
            .collect();

        let comment_ids: Vec<u32> = self
            .query
            .capture_names()
            .iter()
            .enumerate()
            .filter(|(_, n)| **n == "comment")
            .map(|(i, _)| i as u32)
            .collect();
        self.comments.clear();

        // Running the query is ~90% of a highlight pass, and it is the only
        // part whose cost scales with the *file* rather than the screen. Bound
        // it to the lines someone can actually see.
        //
        // A node that merely overlaps the range still matches — a block comment
        // or a raw string opening above the viewport keeps its capture, and the
        // per-line clipping below trims it to the visible rows. Verified against
        // tree-sitter rather than assumed; without it, scrolling into the middle
        // of a long comment would render it as plain text.
        self.cursor.set_byte_range(match &lines {
            Some(r) => {
                let s = line_starts.get(r.start).copied().unwrap_or(bytes.len());
                let e = line_starts.get(r.end).copied().unwrap_or(bytes.len());
                s..e
            }
            None => 0..usize::MAX,
        });
        let mut raw_captures: Vec<(u32, usize, usize)> = Vec::new();
        {
            let mut captures =
                self.cursor
                    .captures(&self.query, tree.root_node(), source.as_bytes());
            while let Some(&(ref m, _capture_idx)) = captures.next() {
                for cap in m.captures {
                    let start = cap.node.byte_range().start;
                    let end = cap.node.byte_range().end;
                    if comment_ids.contains(&cap.index) {
                        self.comments.push((start, end));
                    }
                    raw_captures.push((cap.index, start, end));
                }
            }
        }
        // Leave the cursor unbounded: `comments_in` and any future caller that
        // forgets to set a range should see the whole tree, not the last
        // viewport this happened to be called with.
        self.cursor.set_byte_range(0..usize::MAX);
        raw_captures.sort_by_key(|c| c.1);

        for (idx, bs, be) in &raw_captures {
            let style = styles[*idx as usize];
            let line_s = byte_to_line(*bs, &line_starts);
            let line_e = byte_to_line(*be, &line_starts);
            for li in line_s..=line_e.min(per_line.len() - 1) {
                let lstart = line_starts[li];
                let lend = line_starts[li + 1].min(bytes.len());
                let range_start = (*bs).max(lstart).saturating_sub(lstart);
                let range_end = (*be).min(lend).saturating_sub(lstart);
                if range_end > range_start {
                    let text_slice = &source[lstart..lend];
                    let cs = byte_to_char_offset(text_slice, range_start);
                    let ce = byte_to_char_offset(text_slice, range_end);
                    if ce > cs {
                        per_line[li].push((cs, ce - cs, style));
                    }
                }
            }
        }

        for hl in &mut per_line {
            hl.sort_by_key(|r| r.0);
        }

        let mut styled: Vec<StyledLine> = Vec::new();

        for (li, hl) in per_line.iter().enumerate() {
            let lstart = line_starts[li];
            let lend = line_starts[li + 1];
            let raw = &source[lstart..lend.min(bytes.len())];
            let text = raw.strip_suffix('\n').unwrap_or(raw).to_string();
            // Off screen: keep the text, skip the styling. The rainbow-bracket
            // pass below walks every character of the line, so leaving it
            // unbounded would have kept a chunk of the per-file cost — and left
            // rows that are half-styled, brackets coloured and nothing else.
            if lines.as_ref().is_some_and(|r| !r.contains(&li)) {
                styled.push(StyledLine {
                    text,
                    highlights: Vec::new(),
                });
                continue;
            }
            let text_len = text.chars().count();
            let mut merged: Vec<(usize, usize, SyntaxStyle)> = hl
                .iter()
                .map(|&(s, l, style)| {
                    let clamped_l = (s + l).min(text_len).saturating_sub(s);
                    (s, clamped_l, style)
                })
                .filter(|(_, l, _)| *l > 0)
                .collect();

            for (offset, ch) in text.char_indices() {
                let abs_pos = lstart + offset;
                if abs_pos < rainbow.len() {
                    if let Some(depth) = rainbow[abs_pos] {
                        if "(){}[]".contains(ch) {
                            let color = RAINBOW_PALETTE[depth % 6];
                            merged.retain(|(s, l, _)| !(*s <= offset && offset < *s + *l));
                            merged.push((
                                offset,
                                ch.len_utf8(),
                                SyntaxStyle {
                                    fg: color,
                                    bg: Color::Default,
                                    bold: false,
                                    italic: false,
                                },
                            ));
                        }
                    }
                }
            }
            merged.sort_by_key(|r| r.0);

            styled.push(StyledLine {
                text,
                highlights: merged,
            });
        }

        styled
    }
}

/// Character offset of `byte` within `text`.
///
/// ASCII fast path first: for source code the two are almost always equal, and
/// walking `char_indices` to discover that is the common case made expensive.
fn byte_to_char_offset(text: &str, byte: usize) -> usize {
    if text.is_ascii() {
        return byte.min(text.len());
    }
    text.char_indices()
        .position(|(i, _)| i >= byte)
        .unwrap_or(text.chars().count())
}

/// The 0-based line containing `byte`.
///
/// Binary search, because `line_starts` is sorted and this runs once per
/// capture *per highlight pass*. Scanning it linearly made the pass
/// O(captures x lines) — on a 10k-line file with tens of thousands of
/// captures that was hundreds of millions of comparisons, and about 70 of the
/// 75 ms the pass cost.
fn byte_to_line(byte: usize, line_starts: &[usize]) -> usize {
    match line_starts.binary_search(&byte) {
        // Exactly at a line start: that line.
        Ok(i) => i.min(line_starts.len().saturating_sub(2)),
        // Otherwise the line whose start precedes it.
        Err(i) => i.saturating_sub(1).min(line_starts.len().saturating_sub(2)),
    }
}

#[cfg(test)]
mod lookup_tests {
    use super::*;

    /// The binary search replaced a linear scan. It must agree with it on
    /// *every* position, not just the ones a hand-written case picks — an
    /// off-by-one here shifts highlights by a line, silently.
    #[test]
    fn byte_to_line_matches_a_linear_scan_everywhere() {
        fn linear(byte: usize, starts: &[usize]) -> usize {
            for (i, &s) in starts.iter().enumerate() {
                if byte < s {
                    return i.saturating_sub(1);
                }
            }
            starts.len().saturating_sub(2)
        }

        for text in [
            "",
            "one line",
            "a\nb\nc\n",
            "\n\n\n",
            "trailing newline\n",
            "no trailing newline",
            "unicode \u{e9}\u{e8}\n second line\n",
        ] {
            let mut starts = vec![0usize];
            for (i, c) in text.char_indices() {
                if c == '\n' {
                    starts.push(i + 1);
                }
            }
            starts.push(text.len());
            for b in 0..=text.len() {
                assert_eq!(
                    byte_to_line(b, &starts),
                    linear(b, &starts),
                    "byte {b} of {text:?}"
                );
            }
        }
    }

    /// The ASCII fast path must give the same answer as the general walk.
    #[test]
    fn byte_to_char_offset_agrees_on_ascii_and_unicode() {
        for text in ["plain ascii", "caf\u{e9} au lait", "\u{1f600} emoji", ""] {
            for b in 0..=text.len() {
                let general = text
                    .char_indices()
                    .position(|(i, _)| i >= b)
                    .unwrap_or(text.chars().count());
                assert_eq!(
                    byte_to_char_offset(text, b),
                    general,
                    "byte {b} of {text:?}"
                );
            }
        }
    }
}
