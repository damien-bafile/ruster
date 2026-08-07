//! Line-based highlighting for lightweight markup — Markdown and Org.
//!
//! Neither has a tree-sitter grammar compatible with our pinned tree-sitter
//! 0.25 (Org's crate is stuck on 0.20, and Markdown's is a split block/inline
//! parser that doesn't fit the single-tree engine), and both are line-oriented
//! by design, so per-line rules give good — including *inline* — highlighting
//! without a grammar. State that spans lines (fenced code blocks) is carried in
//! a fold across the document.

use ruster_render::{StyledLine, SyntaxStyle};

use crate::theme::{markup_style, set_current_lang};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkupLang {
    Markdown,
    Org,
}

/// Map a canonical language key to a markup language, if it is one.
pub fn markup_lang(key: &str) -> Option<MarkupLang> {
    match key {
        "markdown" => Some(MarkupLang::Markdown),
        "org" => Some(MarkupLang::Org),
        _ => None,
    }
}

/// A styled span within a single line, in character offsets.
type Span = (usize, usize, SyntaxStyle);

/// Highlight a whole document, one [`StyledLine`] per line (including a trailing
/// empty line when the text ends in a newline, matching the tree-sitter path).
pub fn highlight_markup(lang: MarkupLang, source: &str) -> Vec<StyledLine> {
    // Resolve this pass's per-language color overrides.
    set_current_lang(match lang {
        MarkupLang::Markdown => "markdown",
        MarkupLang::Org => "org",
    });
    let mut in_code = false;
    source
        .split('\n')
        .map(|line| {
            let (spans, still_in_code) = match lang {
                MarkupLang::Markdown => markdown_line(line, in_code),
                MarkupLang::Org => org_line(line, in_code),
            };
            in_code = still_in_code;
            StyledLine {
                text: line.to_string(),
                highlights: spans,
            }
        })
        .collect()
}

/// Highlight one Markdown line. `in_code` is whether we are inside a fenced code
/// block on entry; the returned bool is that state on exit.
fn markdown_line(line: &str, in_code: bool) -> (Vec<Span>, bool) {
    let trimmed = line.trim_start();
    let is_fence = trimmed.starts_with("```") || trimmed.starts_with("~~~");
    if in_code {
        // The closing fence leaves the block; everything else is block content.
        let spans = vec![(0, line.chars().count(), markup_style("block"))];
        return (spans, !is_fence);
    }
    if is_fence {
        return (
            vec![(0, line.chars().count(), markup_style("keyword"))],
            true,
        );
    }

    // ATX heading: 1–6 leading '#'s then a space.
    let indent = line.chars().take_while(|c| *c == ' ').count();
    let after = &line[indent..];
    let hashes = after.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && after.chars().nth(hashes) == Some(' ') {
        return (
            vec![(0, line.chars().count(), markup_style("heading"))],
            false,
        );
    }

    // Blockquote and thematic break span the whole line.
    if after.starts_with('>') {
        return (
            vec![(0, line.chars().count(), markup_style("quote"))],
            false,
        );
    }
    if is_thematic_break(after) {
        return (
            vec![(0, line.chars().count(), markup_style("marker"))],
            false,
        );
    }

    let mut spans = Vec::new();
    // A list marker (`-`/`*`/`+` or `N.`) at the start, before inline scanning.
    if let Some(len) = list_marker_len(after) {
        spans.push((indent, len, markup_style("marker")));
    }
    inline_markdown(line, &mut spans);
    spans.sort_by_key(|s| s.0);
    (spans, false)
}

/// Highlight one Org line.
fn org_line(line: &str, in_code: bool) -> (Vec<Span>, bool) {
    let lower = line.trim_start().to_ascii_lowercase();
    let is_end = lower.starts_with("#+end_");
    let is_begin = lower.starts_with("#+begin_");
    if in_code {
        let spans = vec![(0, line.chars().count(), markup_style("block"))];
        return (spans, !is_end);
    }

    // `#+KEYWORD:` metadata and block delimiters.
    if line.trim_start().starts_with("#+") {
        return (
            vec![(0, line.chars().count(), markup_style("keyword"))],
            is_begin,
        );
    }
    // A plain `# ` comment line.
    if line.trim_start().starts_with("# ") {
        return (
            vec![(0, line.chars().count(), markup_style("quote"))],
            false,
        );
    }

    // Headings: leading '*'s then a space. TODO/DONE keywords get their own color.
    let stars = line.chars().take_while(|c| *c == '*').count();
    if stars > 0 && line.chars().nth(stars) == Some(' ') {
        let mut spans = vec![(0, line.chars().count(), markup_style("heading"))];
        let rest = &line[stars + 1..];
        if let Some(kw) = rest.split_whitespace().next() {
            let style = match kw {
                "TODO" => Some(markup_style("todo")),
                "DONE" => Some(markup_style("done")),
                _ => None,
            };
            if let Some(style) = style {
                spans.push((stars + 1, kw.chars().count(), style));
            }
        }
        spans.sort_by_key(|s| s.0);
        return (spans, false);
    }

    // Table rows.
    if line.trim_start().starts_with('|') {
        return (
            vec![(0, line.chars().count(), markup_style("marker"))],
            false,
        );
    }

    let mut spans = Vec::new();
    let indent = line.chars().take_while(|c| *c == ' ').count();
    if let Some(len) = list_marker_len(&line[indent..]) {
        spans.push((indent, len, markup_style("marker")));
    }
    inline_org(line, &mut spans);
    spans.sort_by_key(|s| s.0);
    (spans, false)
}

/// `---`, `***`, or `___` (3+), optionally spaced, alone on the line.
fn is_thematic_break(s: &str) -> bool {
    let t: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    t.len() >= 3
        && (t.chars().all(|c| c == '-')
            || t.chars().all(|c| c == '*')
            || t.chars().all(|c| c == '_'))
}

/// Length (in chars) of a leading list marker like `- `, `* `, `+ `, or `12. `.
fn list_marker_len(s: &str) -> Option<usize> {
    let mut chars = s.chars();
    match chars.next()? {
        '-' | '+' | '*' if chars.next() == Some(' ') => Some(1),
        c if c.is_ascii_digit() => {
            let digits = s.chars().take_while(|c| c.is_ascii_digit()).count();
            let rest = &s[digits..];
            if (rest.starts_with(". ") || rest.starts_with(") ")) && digits <= 9 {
                Some(digits + 1)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Markdown inline spans: `` `code` ``, `**strong**`, `*em*`/`_em_`,
/// `[text](url)`. Code is scanned first and its interior is left untouched.
fn inline_markdown(line: &str, spans: &mut Vec<Span>) {
    let chars: Vec<char> = line.chars().collect();
    let mut covered = vec![false; chars.len()];

    scan_delimited(&chars, &mut covered, "`", "`", markup_style("code"), spans);
    scan_delimited(
        &chars,
        &mut covered,
        "**",
        "**",
        markup_style("strong"),
        spans,
    );
    scan_delimited(
        &chars,
        &mut covered,
        "__",
        "__",
        markup_style("strong"),
        spans,
    );
    scan_delimited(
        &chars,
        &mut covered,
        "*",
        "*",
        markup_style("emphasis"),
        spans,
    );
    scan_delimited(
        &chars,
        &mut covered,
        "_",
        "_",
        markup_style("emphasis"),
        spans,
    );
    scan_links(&chars, &mut covered, spans);
}

/// Org inline spans: `*strong*`, `/emphasis/`, `~code~`/`=verbatim=`,
/// `_underline_`. Org uses the same simple paired-delimiter scan.
fn inline_org(line: &str, spans: &mut Vec<Span>) {
    let chars: Vec<char> = line.chars().collect();
    let mut covered = vec![false; chars.len()];

    scan_delimited(&chars, &mut covered, "~", "~", markup_style("code"), spans);
    scan_delimited(&chars, &mut covered, "=", "=", markup_style("code"), spans);
    scan_delimited(
        &chars,
        &mut covered,
        "*",
        "*",
        markup_style("strong"),
        spans,
    );
    scan_delimited(
        &chars,
        &mut covered,
        "/",
        "/",
        markup_style("emphasis"),
        spans,
    );
    scan_delimited(
        &chars,
        &mut covered,
        "_",
        "_",
        markup_style("emphasis"),
        spans,
    );
}

/// Find `open … close` pairs and emit one span per pair (delimiters included).
/// Skips positions already `covered` by an earlier, higher-precedence scan, and
/// requires a non-empty interior so `**` alone isn't treated as a pair.
fn scan_delimited(
    chars: &[char],
    covered: &mut [bool],
    open: &str,
    close: &str,
    style: SyntaxStyle,
    spans: &mut Vec<Span>,
) {
    let open: Vec<char> = open.chars().collect();
    let close: Vec<char> = close.chars().collect();
    let n = chars.len();
    let mut i = 0;
    while i + open.len() <= n {
        if covered[i] || !matches_at(chars, i, &open) {
            i += 1;
            continue;
        }
        // Find the closing delimiter after a non-empty interior.
        let mut j = i + open.len();
        let mut found = None;
        while j + close.len() <= n {
            if !covered[j] && matches_at(chars, j, &close) && j > i + open.len() {
                found = Some(j);
                break;
            }
            j += 1;
        }
        match found {
            Some(end) => {
                let span_end = end + close.len();
                for c in covered.iter_mut().take(span_end).skip(i) {
                    *c = true;
                }
                spans.push((i, span_end - i, style));
                i = span_end;
            }
            None => i += 1,
        }
    }
}

/// `[text](url)` — the bracketed text as a link, the url dimmed.
fn scan_links(chars: &[char], covered: &mut [bool], spans: &mut Vec<Span>) {
    let n = chars.len();
    let mut i = 0;
    while i < n {
        if chars[i] == '[' && !covered[i] {
            if let Some(rb) = (i + 1..n).find(|&k| chars[k] == ']') {
                if rb + 1 < n && chars[rb + 1] == '(' {
                    if let Some(rp) = (rb + 2..n).find(|&k| chars[k] == ')') {
                        for c in covered.iter_mut().take(rp + 1).skip(i) {
                            *c = true;
                        }
                        spans.push((i, rb + 1 - i, markup_style("link")));
                        spans.push((rb + 1, rp + 1 - (rb + 1), markup_style("url")));
                        i = rp + 1;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
}

fn matches_at(chars: &[char], at: usize, pat: &[char]) -> bool {
    at + pat.len() <= chars.len() && chars[at..at + pat.len()] == *pat
}

#[cfg(test)]
mod tests {
    use super::*;

    fn styles_of(sl: &StyledLine) -> Vec<(usize, usize)> {
        sl.highlights.iter().map(|(o, l, _)| (*o, *l)).collect()
    }

    #[test]
    fn markdown_heading_and_inline() {
        let out = highlight_markup(MarkupLang::Markdown, "# Title\ntext **bold** and `code`\n");
        // Heading line: one whole-line span.
        assert_eq!(styles_of(&out[0]), vec![(0, 7)]);
        // Second line has a strong span over "**bold**" and a code span.
        let second = &out[1];
        assert!(second
            .highlights
            .iter()
            .any(|(o, l, s)| *o == 5 && *l == 8 && s.bold));
        assert!(second
            .highlights
            .iter()
            .any(|(_, _, s)| s.fg == markup_style("code").fg));
    }

    #[test]
    fn markdown_fenced_code_block_spans_lines() {
        let src = "before\n```rust\nfn x() {}\n```\nafter\n";
        let out = highlight_markup(MarkupLang::Markdown, src);
        // The opening fence, the code line, and the closing fence are all styled.
        assert!(!out[1].highlights.is_empty(), "opening fence styled");
        assert!(!out[2].highlights.is_empty(), "code content styled");
        assert!(!out[3].highlights.is_empty(), "closing fence styled");
        // Text after the block is back to normal (no whole-line block span).
        assert!(out[4].highlights.is_empty(), "after block is plain");
    }

    #[test]
    fn markdown_list_and_link() {
        let out = highlight_markup(MarkupLang::Markdown, "- see [docs](http://x)\n");
        let line = &out[0];
        // Marker at 0..1, link text and url spans present.
        assert!(line.highlights.iter().any(|(o, l, _)| *o == 0 && *l == 1));
        assert!(line
            .highlights
            .iter()
            .any(|(_, _, s)| s.fg == markup_style("link").fg));
        assert!(line
            .highlights
            .iter()
            .any(|(_, _, s)| s.fg == markup_style("url").fg));
    }

    #[test]
    fn org_heading_todo_and_keyword() {
        let out = highlight_markup(
            MarkupLang::Org,
            "#+TITLE: x\n* TODO task\n/em/ and ~code~\n",
        );
        assert_eq!(styles_of(&out[0]), vec![(0, 10)]); // whole #+ line
                                                       // Heading line has a whole-line heading span plus a TODO span at 2..6.
        assert!(out[1].highlights.iter().any(|(o, l, _)| *o == 2 && *l == 4));
        // Inline emphasis and code on the third line.
        assert!(out[2].highlights.iter().any(|(_, _, s)| s.italic));
        assert!(out[2]
            .highlights
            .iter()
            .any(|(_, _, s)| s.fg == markup_style("code").fg));
    }

    #[test]
    fn org_src_block_spans_lines() {
        let src = "#+BEGIN_SRC rust\nlet x = 1;\n#+END_SRC\ndone\n";
        let out = highlight_markup(MarkupLang::Org, src);
        assert!(!out[1].highlights.is_empty(), "src content styled");
        assert!(out[3].highlights.is_empty(), "after block is plain");
    }

    #[test]
    fn unpaired_delimiter_is_not_a_span() {
        // A lone '*' should not produce an emphasis span.
        let out = highlight_markup(MarkupLang::Markdown, "a * b\n");
        assert!(out[0].highlights.is_empty());
    }
}
