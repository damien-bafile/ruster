use streaming_iterator::StreamingIterator;
use ruster_render::{Color, SyntaxStyle, StyledLine};
use crate::theme::{style_for_capture, RAINBOW_PALETTE};

pub struct Highlighter {
    query: tree_sitter::Query,
    cursor: tree_sitter::QueryCursor,
}

impl Highlighter {
    pub fn new(language: tree_sitter::Language, query_source: &str) -> Result<Self, String> {
        let query = tree_sitter::Query::new(&language, query_source)
            .map_err(|e| format!("query error: {}", e))?;
        Ok(Highlighter { query, cursor: tree_sitter::QueryCursor::new() })
    }

    pub fn highlight_lines(
        &mut self,
        tree: &tree_sitter::Tree,
        source: &str,
        rainbow: &[Option<usize>],
    ) -> Vec<StyledLine> {
        let bytes = source.as_bytes();
        let mut line_starts: Vec<usize> = vec![0];
        for (i, ch) in source.char_indices() {
            if ch == '\n' { line_starts.push(i + 1); }
        }
        line_starts.push(bytes.len());

        let mut per_line: Vec<Vec<(usize, usize, SyntaxStyle)>> =
            (0..line_starts.len() - 1).map(|_| Vec::new()).collect();

        let mut raw_captures: Vec<(String, usize, usize)> = Vec::new();
        {
            let mut captures = self.cursor.captures(&self.query, tree.root_node(), source.as_bytes());
            while let Some(&(ref m, _capture_idx)) = captures.next() {
                for cap in m.captures {
                    let start = cap.node.byte_range().start;
                    let end = cap.node.byte_range().end;
                    let name = self.query.capture_names()[cap.index as usize].to_string();
                    raw_captures.push((name, start, end));
                }
            }
        }
        raw_captures.sort_by_key(|c| c.1);

        for (name, bs, be) in &raw_captures {
            let style = style_for_capture(name);
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
            let text_len = text.chars().count();
            let mut merged: Vec<(usize, usize, SyntaxStyle)> = hl.iter()
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
                            merged.push((offset, ch.len_utf8(),
                                SyntaxStyle { fg: color, bg: Color::Default, bold: false, italic: false }));
                        }
                    }
                }
            }
            merged.sort_by_key(|r| r.0);

            styled.push(StyledLine { text, highlights: merged });
        }

        styled
    }
}

fn byte_to_char_offset(text: &str, byte: usize) -> usize {
    text.char_indices().position(|(i, _)| i >= byte).unwrap_or(text.chars().count())
}

fn byte_to_line(byte: usize, line_starts: &[usize]) -> usize {
    for (i, &start) in line_starts.iter().enumerate() {
        if byte < start { return i.saturating_sub(1); }
    }
    line_starts.len().saturating_sub(2)
}
