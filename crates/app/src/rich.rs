//! Styled text for the PR reading tabs: Markdown into blocks, blocks into rows wrapped to a column count.
//! The terminal draws these rows; the GUI reflows their shared Markdown documents with the native reading style.
use lenz_content::markdown::{BlockKind, MarkdownDocument};
use std::{ops::Range, sync::Arc};
use unicode_width::UnicodeWidthChar;

/// A colour role; each frontend maps it to its palette.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone { #[default] Normal, Dim, Accent, Added, Removed, Good, Bad, Warn }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style { pub bold: bool, pub italic: bool, pub code: bool, pub strike: bool, pub highlight: bool, pub tone: Tone }

/// What a click on a span does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Url(String),
    /// A line of a file in Files changed (1-based, on the head side unless `left`).
    Line { path: String, line: u32, left: bool },
    /// Expand or collapse a thread or a commit, by its key in the page.
    Toggle(String),
    /// Show Files changed for one commit (`None`: the whole PR).
    Commit(Option<String>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Span { pub text: String, pub style: Style, pub target: Option<Target> }

impl Span {
    /// Text that would act on a terminal (escapes, bidi controls) shows as `\u{fffd}`.
    pub fn new(text: impl Into<String>, style: Style) -> Self {
        let text: String = text.into();
        let text = if text.chars().any(crate::document::hides_text) { text.chars().map(|c| if crate::document::hides_text(c) && !matches!(c, '\t' | '\n') { '\u{fffd}' } else { c }).collect() } else { text };
        Self { text, style, target: None }
    }
    pub fn plain(text: impl Into<String>) -> Self { Self::new(text, Style::default()) }
    pub fn tone(text: impl Into<String>, tone: Tone) -> Self { Self::new(text, Style { tone, ..Style::default() }) }
    pub fn bold(text: impl Into<String>) -> Self { Self::new(text, Style { bold: true, ..Style::default() }) }
    pub fn to(mut self, target: Target) -> Self { self.target = Some(target); self }
}

/// A row's background: code blocks and quotes are shaded, a rule is a line across.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fill { #[default] None, Code, Quote, Rule }

/// A paragraph-like unit before wrapping.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Block {
    pub spans: Vec<Span>,
    /// Columns before the text; `marker` (a list bullet) sits in them on the first row.
    pub indent: usize,
    pub marker: Option<Span>,
    pub fill: Fill,
    /// Code: one row per source line, cut rather than wrapped.
    pub verbatim: bool,
}

/// One drawn row: spans placed from column `indent`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Row { pub spans: Vec<Span>, pub indent: usize, pub fill: Fill }

impl Row {
    pub fn text(&self) -> String { self.spans.iter().map(|s| s.text.as_str()).collect() }
    /// The span under column `column` (counted from the row's start, indent included).
    pub fn span_at(&self, column: usize) -> Option<&Span> {
        let mut at = self.indent;
        self.spans.iter().find(|s| { let start = at; at += width(&s.text); (start..at).contains(&column) })
    }
}

pub fn width(text: &str) -> usize { text.chars().map(|c| if c == '\t' { 4 } else { c.width().unwrap_or(0) }).sum() }

/// The same bounded document parser used by Markdown file previews, plus terminal-ready blocks.
pub struct Markdown { pub document: Option<Arc<MarkdownDocument>>, blocks: Vec<Block> }
impl std::ops::Deref for Markdown {
    type Target = [Block];
    fn deref(&self) -> &[Block] { &self.blocks }
}

#[cold]
pub fn markdown(text: &str) -> Markdown {
    let limits = lenz_content::markdown::Limits::DEFAULT;
    let mut remaining = limits.segments_per_pair;
    let document = lenz_content::markdown::parse(Arc::from(text), limits, &mut remaining, &|| true).ok().map(Arc::new);
    let Some(doc) = &document else {
        return Markdown { document: None, blocks: vec![Block { spans: vec![Span::plain(text)], ..Block::default() }] };
    };
    let mut blocks = Vec::new();
    for block in &doc.blocks {
        let indent = usize::from(block.quote_depth) * 2 + usize::from(block.list_depth.saturating_sub(1)) * 2;
        let fill = if matches!(block.kind, BlockKind::Code { .. } | BlockKind::Literal | BlockKind::Properties) { Fill::Code }
            else if matches!(block.kind, BlockKind::Rule) { Fill::Rule }
            else if block.quote_depth > 0 { Fill::Quote } else { Fill::None };
        let verbatim = fill == Fill::Code;
        let mut at = block.text.start;
        for line in doc.text.get(block.text.clone()).unwrap_or_default().trim_end_matches('\n').split('\n') {
            let marker = if at == block.text.start { if let BlockKind::ListItem { marker, .. } = &block.kind { Some(Span::tone(format!("{marker} "), Tone::Dim)) } else { None } } else { None };
            let range = at + marker.as_ref().map_or(0, |m| m.text.len())..at + line.len();
            let mut spans = markdown_spans(doc, block, range);
            for span in &mut spans { span.text = span.text.replace('\t', if matches!(block.kind, BlockKind::TableRow { .. }) { " │ " } else { "    " }); }
            if matches!(&block.kind, BlockKind::Code { language: Some(l) } if l == "diff") {
                let tone = code_line(line, 0, true).spans[0].style.tone;
                for span in &mut spans { span.style.tone = tone; }
            }
            blocks.push(Block { spans, indent, fill, verbatim, marker });
            at += line.len() + 1;
        }
    }
    Markdown { document, blocks }
}

/// Styled spans of one rendered document range; links use the same safe schemes in both adapters.
pub fn markdown_spans(doc: &MarkdownDocument, block: &lenz_content::markdown::Block, range: Range<usize>) -> Vec<Span> {
    let mut cuts = vec![range.start, range.end];
    for run in &block.runs {
        if run.text.start < range.end && run.text.end > range.start { cuts.extend([run.text.start.max(range.start), run.text.end.min(range.end)]); }
    }
    for link in &doc.links {
        if link.text.start < range.end && link.text.end > range.start { cuts.extend([link.text.start.max(range.start), link.text.end.min(range.end)]); }
    }
    cuts.sort_unstable();
    cuts.dedup();
    cuts.windows(2).map(|r| {
        let run = block.runs.get(block.runs.partition_point(|s| s.text.end <= r[0])).filter(|s| s.text.start <= r[0]).map(|r| r.style).unwrap_or_default();
        let heading = matches!(block.kind, BlockKind::Heading(_) | BlockKind::TableRow { header: true, .. });
        let code = run.code || matches!(block.kind, BlockKind::Code { .. } | BlockKind::Literal | BlockKind::Properties);
        let target = doc.links.iter().find(|l| l.text.start <= r[0] && l.text.end >= r[1]).map(|l| l.destination.as_ref())
            .or(if let BlockKind::Image { destination, .. } = &block.kind { Some(destination.as_str()) } else { None })
            .filter(|url| crate::open::is_web_link(url)).map(|url| Target::Url(url.to_string()));
        let tone = if heading || target.is_some() { Tone::Accent } else { Tone::Normal };
        Span { target, ..Span::new(doc.text.get(r[0]..r[1]).unwrap_or_default(), Style { bold: heading || run.strong, italic: run.emphasis, code, strike: run.strike, highlight: run.highlight, tone }) }
    }).collect()
}

/// One line of code; in a diff, `+` and `-` lines are coloured.
pub fn code_line(line: &str, indent: usize, diff: bool) -> Block {
    let tone = match line.chars().next() {
        Some('+') if diff => Tone::Added,
        Some('-') if diff => Tone::Removed,
        Some('@') if diff => Tone::Dim,
        _ => Tone::Normal,
    };
    let text = line.replace('\t', "    ");
    Block { spans: vec![Span::new(text, Style { code: true, tone, ..Style::default() })], indent, fill: Fill::Code, verbatim: true, marker: None }
}

/// `blocks` as rows at most `columns` wide, appended to `out` with `extra` columns of indent (card nesting).
pub fn wrap(blocks: &[Block], columns: usize, extra: usize, out: &mut Vec<Row>) {
    for block in blocks {
        let indent = extra + block.indent + block.marker.as_ref().map_or(0, |m| width(&m.text));
        let room = columns.saturating_sub(indent).max(8);
        if block.fill == Fill::Rule { out.push(Row { spans: Vec::new(), indent: extra + block.indent, fill: Fill::Rule }); continue; }
        let mut rows: Vec<Vec<Span>> = vec![Vec::new()];
        let mut used = 0;
        for span in &block.spans {
            let mut piece = String::new();
            let flush = |rows: &mut Vec<Vec<Span>>, piece: &mut String| {
                if !piece.is_empty() { rows.last_mut().unwrap().push(Span { text: std::mem::take(piece), ..span.clone() }); }
            };
            // words, kept with their trailing space, move to the next row when they do not fit
            for word in span.text.split_inclusive(' ') {
                let w = width(word);
                if used + w.min(room) > room && used > 0 && !block.verbatim {
                    flush(&mut rows, &mut piece);
                    rows.push(Vec::new());
                    used = 0;
                    if word == " " { continue; }
                }
                for c in word.chars() {
                    let cw = c.width().unwrap_or(0);
                    if used + cw > room && used > 0 {
                        if block.verbatim { break; }
                        flush(&mut rows, &mut piece);
                        rows.push(Vec::new());
                        used = 0;
                    }
                    piece.push(c);
                    used += cw;
                }
            }
            flush(&mut rows, &mut piece);
        }
        for (i, mut spans) in rows.into_iter().enumerate() {
            if i == 0 && let Some(marker) = &block.marker { spans.insert(0, marker.clone()); }
            let start = if i == 0 && block.marker.is_some() { extra + block.indent } else { indent };
            out.push(Row { spans, indent: start, fill: block.fill });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(md: &str, columns: usize) -> Vec<String> {
        let mut rows = Vec::new();
        wrap(&markdown(md), columns, 0, &mut rows);
        rows.iter().map(|r| format!("{}{}", " ".repeat(r.indent), r.text())).collect()
    }

    #[test]
    fn markdown_becomes_styled_wrapped_rows() {
        assert_eq!(texts("# Title\n\nSome *words* here that wrap", 14), ["Title", "Some words ", "here that wrap"]);
        assert_eq!(texts("- one\n- two\n  1. deep", 40), ["• one", "• two", "  1. deep"]);
        assert_eq!(texts("```diff\n-a\n+b\n```", 40), ["-a", "+b"]);
        assert_eq!(texts("> quoted", 40), ["  quoted"], "the quote bar sits in the indent");
        assert_eq!(texts("| a | b |\n|---|---|\n| 1 | 2 |", 40), ["a │ b", "1 │ 2"]);
        let blocks = markdown("```diff\n-a\n+b\n```\n\n[site](https://x) and `code`");
        assert_eq!(blocks[0].spans[0].style.tone, Tone::Removed);
        assert_eq!(blocks[1].spans[0].style.tone, Tone::Added);
        assert_eq!(blocks[2].spans[0].target, Some(Target::Url("https://x".into())));
        assert_eq!(markdown("[x](file:///etc/passwd) [y](vscode://z)")[0].spans.iter().find_map(|s| s.target.clone()), None, "only web links are links");
        assert!(blocks[2].spans[2].style.code);
        assert_eq!(markdown("> q")[0].fill, Fill::Quote);
    }

    #[test]
    fn pr_markdown_uses_the_document_reading_parser() {
        assert_eq!(texts("A ==highlight== and [[guide|the guide]].", 80), ["A highlight and the guide."]);
    }

    #[test]
    fn code_tabs_stay_indentation_and_only_tables_have_cell_separators() {
        assert_eq!(texts("```rust\n\tfn main() {}\n```", 80), ["    fn main() {}"]);
        assert_eq!(texts("| a | b |\n|---|---|\n| x | y |", 80), ["a │ b", "x │ y"]);
    }

    #[test]
    fn continuation_rows_hang_under_the_text_and_spans_are_found_by_column() {
        let mut rows = Vec::new();
        wrap(&markdown("- alpha beta gamma"), 14, 2, &mut rows);
        assert_eq!(rows.iter().map(|r| (r.indent, r.text())).collect::<Vec<_>>(), [(2, "• alpha ".into()), (4, "beta gamma".into())]);
        assert_eq!(rows[1].span_at(4).map(|s| s.text.as_str()), Some("beta gamma"));
        assert_eq!(rows[1].span_at(3), None, "the indent holds no text");
    }
}
