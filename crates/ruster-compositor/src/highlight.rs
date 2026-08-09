//! Syntax highlighting for editor panes.
//!
//! One [`SyntaxEngine`] per *document*, not per pane: two panes on one file
//! should share a parse rather than run two of them, and the parse depends on
//! the text, which the panes share. Keyed by `BufferId` in a side map here
//! rather than held on the `Document`, because `Document` lives in `ruster-core`
//! and `SyntaxEngine` in `ruster-syntax`, which `ruster-core` does not depend on.
//! The same shape as `panes` beside `toplevels`, for the same reason.
//!
//! Nothing here decides what a colour *is*. `SyntaxEngine::highlight_line`
//! returns spans already carrying a [`SyntaxStyle`] with a resolved
//! `ruster_render::Color`, which `Chrome` already knows how to draw with — so
//! there is no token-to-theme mapping in the compositor to drift from the
//! editor's.

use std::collections::HashMap;

use ruster_core::buffer::Buffer;
use ruster_core::document::BufferId;
use ruster_render::{StyledLine, SyntaxStyle};
use ruster_syntax::SyntaxEngine;

/// A parse per document, and the buffer revision it was made from.
#[derive(Default)]
pub struct Highlights {
    engines: HashMap<BufferId, Parsed>,
}

struct Parsed {
    engine: SyntaxEngine,
    /// The `Buffer::revision` this parse reflects.
    ///
    /// Re-parsing every frame cost 107ms on a 10k-line file, which is why
    /// `revision` exists at all. Comparing it is what keeps a still buffer free.
    revision: u64,
}

impl Highlights {
    /// The visible lines of `buffer`, styled.
    ///
    /// Falls back to unstyled text whenever there is no parse to be had — an
    /// extension tree-sitter has no grammar for, a file that failed to parse.
    /// Returning plain lines rather than nothing is the difference between a
    /// file that looks unhighlighted and a pane that looks broken.
    pub fn styled_lines(
        &mut self,
        doc: BufferId,
        extension: &str,
        buffer: &Buffer,
        first_line: usize,
        lines: &[String],
    ) -> Vec<StyledLine> {
        let Some(parsed) = self.parsed_for(doc, extension, buffer, first_line, lines.len()) else {
            return lines.iter().map(|l| plain(l)).collect();
        };
        lines
            .iter()
            .enumerate()
            .map(|(row, text)| StyledLine {
                highlights: clip_spans(&parsed.engine.highlight_line(first_line + row), text),
                text: text.clone(),
            })
            .collect()
    }

    /// Forget a document's parse. Called when a buffer is closed, so a long
    /// session does not accumulate syntax trees for files nobody has open.
    pub fn forget(&mut self, doc: BufferId) {
        self.engines.remove(&doc);
    }

    /// The parse for `doc`, made or refreshed if the buffer has moved on.
    fn parsed_for(
        &mut self,
        doc: BufferId,
        extension: &str,
        buffer: &Buffer,
        first_line: usize,
        shown: usize,
    ) -> Option<&mut Parsed> {
        let revision = buffer.revision();
        match self.engines.entry(doc) {
            std::collections::hash_map::Entry::Occupied(entry) => {
                let parsed = entry.into_mut();
                if parsed.revision != revision {
                    // Whole-text reparse. `reparse_with_edits` is the cheaper
                    // path, but it needs the edits *since this parse*, and
                    // `Buffer::take_edits` drains — so a second reader (the
                    // editor, a future second pane) would find them gone.
                    // Correct and slower beats fast and occasionally wrong;
                    // the revision check already keeps a still buffer free.
                    parsed.engine.reparse(&buffer.to_string());
                    parsed.revision = revision;
                }
                // Only the lines on screen are coloured. A tile showing 40 lines
                // of a 10k-line file has no use for the other 9,960.
                parsed
                    .engine
                    .set_viewport(first_line, first_line + shown.max(1));
                Some(parsed)
            }
            std::collections::hash_map::Entry::Vacant(slot) => {
                let engine = SyntaxEngine::new(&buffer.to_string(), extension).ok()?;
                let parsed = slot.insert(Parsed { engine, revision });
                parsed
                    .engine
                    .set_viewport(first_line, first_line + shown.max(1));
                Some(parsed)
            }
        }
    }
}

/// A line with no highlighting.
fn plain(text: &str) -> StyledLine {
    StyledLine {
        text: text.to_string(),
        highlights: Vec::new(),
    }
}

/// Spans clipped to what `text` actually contains.
///
/// The engine indexes the buffer's line, which still has its terminator, while
/// a pane's line has been trimmed of it — and a span running one character past
/// the end would slice outside the string when it is drawn.
fn clip_spans(
    spans: &[(usize, usize, SyntaxStyle)],
    text: &str,
) -> Vec<(usize, usize, SyntaxStyle)> {
    let len = text.chars().count();
    spans
        .iter()
        .filter_map(|(start, end, style)| {
            let start = (*start).min(len);
            let end = (*end).min(len);
            (start < end).then_some((start, end, *style))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> SyntaxStyle {
        SyntaxStyle {
            fg: ruster_render::Color::Rgb(1, 2, 3),
            ..SyntaxStyle::default()
        }
    }

    #[test]
    fn a_span_past_the_end_of_a_line_is_cut_to_fit() {
        // The engine indexes the buffer's line, terminator and all; a pane's
        // line has been trimmed. A span left one character long would slice
        // outside the string when it is drawn.
        let spans = vec![(0, 99, style())];
        assert_eq!(clip_spans(&spans, "abc"), vec![(0, 3, style())]);
    }

    #[test]
    fn a_span_entirely_past_the_end_is_dropped_rather_than_emptied() {
        // An empty span is not a span; keeping it would draw a zero-width run
        // and confuse anything counting them.
        assert!(clip_spans(&[(10, 20, style())], "abc").is_empty());
    }

    #[test]
    fn spans_inside_the_line_are_left_alone() {
        let spans = vec![(0, 2, style()), (4, 7, style())];
        assert_eq!(clip_spans(&spans, "abcdefgh"), spans);
    }

    #[test]
    fn a_language_with_no_grammar_still_produces_lines() {
        // Highlighting is the enhancement; the text is the point. A file
        // tree-sitter cannot parse must look unhighlighted, not empty.
        let mut hl = Highlights::default();
        let buffer = Buffer::from_str("some text\nmore text\n");
        let lines = vec!["some text".to_string(), "more text".to_string()];
        let out = hl.styled_lines(BufferId(1), "no-such-extension", &buffer, 0, &lines);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].text, "some text");
        assert!(out[0].highlights.is_empty());
    }

    #[test]
    fn rust_source_comes_back_with_spans() {
        let mut hl = Highlights::default();
        let text = "fn main() {\n    let x = 1;\n}\n";
        let buffer = Buffer::from_str(text);
        let lines = vec![
            "fn main() {".to_string(),
            "    let x = 1;".to_string(),
            "}".to_string(),
        ];
        let out = hl.styled_lines(BufferId(1), "rs", &buffer, 0, &lines);
        assert_eq!(out.len(), 3);
        assert!(
            out.iter().any(|l| !l.highlights.is_empty()),
            "rust should highlight something: {out:?}"
        );
        // And every span must lie inside the line it belongs to.
        for line in &out {
            let len = line.text.chars().count();
            for (start, end, _) in &line.highlights {
                assert!(*start < *end && *end <= len, "span {start}..{end} in {len}");
            }
        }
    }

    #[test]
    fn a_still_buffer_is_not_reparsed() {
        // The revision check is the whole reason this is affordable: reparsing
        // every frame cost 107ms on a 10k-line file.
        let mut hl = Highlights::default();
        let buffer = Buffer::from_str("fn main() {}\n");
        let lines = vec!["fn main() {}".to_string()];
        hl.styled_lines(BufferId(1), "rs", &buffer, 0, &lines);
        let first = hl.engines[&BufferId(1)].revision;
        hl.styled_lines(BufferId(1), "rs", &buffer, 0, &lines);
        assert_eq!(
            hl.engines[&BufferId(1)].revision,
            first,
            "an unchanged buffer should not move the parse on"
        );
    }

    #[test]
    fn an_edited_buffer_is_reparsed() {
        let mut hl = Highlights::default();
        let mut buffer = Buffer::from_str("fn main() {}\n");
        let lines = vec!["fn main() {}".to_string()];
        hl.styled_lines(BufferId(1), "rs", &buffer, 0, &lines);
        let before = hl.engines[&BufferId(1)].revision;

        buffer.insert(0, "// a comment\n");
        let lines = vec!["// a comment".to_string()];
        hl.styled_lines(BufferId(1), "rs", &buffer, 0, &lines);
        assert_ne!(
            hl.engines[&BufferId(1)].revision,
            before,
            "an edited buffer must be reparsed"
        );
    }

    #[test]
    fn closing_a_document_drops_its_parse() {
        // Otherwise a long session accumulates syntax trees for files nobody
        // has open, which on a 10k-line file is not a small thing to keep.
        let mut hl = Highlights::default();
        let buffer = Buffer::from_str("fn main() {}\n");
        hl.styled_lines(BufferId(1), "rs", &buffer, 0, &["fn main() {}".to_string()]);
        assert!(hl.engines.contains_key(&BufferId(1)));
        hl.forget(BufferId(1));
        assert!(hl.engines.is_empty());
    }
}
