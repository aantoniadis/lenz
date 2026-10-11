//! Styled text for the PR reading tabs: Markdown into blocks, blocks into rows wrapped to a column count.
//! Both frontends draw the same rows (the GUI in its monospace font), so wrapping and click targets agree.
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use unicode_width::UnicodeWidthChar;

/// A colour role; each frontend maps it to its palette.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone { #[default] Normal, Dim, Accent, Added, Removed, Good, Bad, Warn }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style { pub bold: bool, pub italic: bool, pub code: bool, pub strike: bool, pub tone: Tone }

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

pub fn width(text: &str) -> usize { text.chars().map(|c| c.width().unwrap_or(0)).sum() }

/// Markdown (GitHub flavour) as blocks. Images become a link to the image; raw HTML shows as dim text.
pub fn markdown(text: &str) -> Vec<Block> {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut out: Vec<Block> = Vec::new();
    let mut current = Block::default();
    let mut style = Style::default();
    let mut link: Option<String> = None;
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut quote = 0usize;
    let mut table_row: Vec<Vec<Span>> = Vec::new();
    let mut in_code = false;
    let indent = |lists: &Vec<Option<u64>>, quote: usize| quote * 2 + lists.len().saturating_sub(1) * 2;
    let flush = |out: &mut Vec<Block>, current: &mut Block| {
        if !current.spans.is_empty() || current.fill == Fill::Rule { out.push(std::mem::take(current)); } else { *current = Block::default(); }
    };
    for event in Parser::new_ext(text, options) {
        match event {
            Event::Start(Tag::Heading { .. }) => { flush(&mut out, &mut current); style.bold = true; style.tone = Tone::Accent; current.indent = indent(&lists, quote); }
            Event::End(TagEnd::Heading(_)) => { flush(&mut out, &mut current); style = Style::default(); }
            Event::Start(Tag::Paragraph) => {
                // a list item's first paragraph continues its marker
                if current.marker.is_none() { flush(&mut out, &mut current); current.indent = indent(&lists, quote); }
                if quote > 0 { current.fill = Fill::Quote; }
            }
            Event::End(TagEnd::Paragraph) => flush(&mut out, &mut current),
            Event::Start(Tag::BlockQuote(_)) => { flush(&mut out, &mut current); quote += 1; }
            Event::End(TagEnd::BlockQuote(_)) => { flush(&mut out, &mut current); quote -= 1; }
            Event::Start(Tag::List(start)) => { flush(&mut out, &mut current); lists.push(start); }
            Event::End(TagEnd::List(_)) => { flush(&mut out, &mut current); lists.pop(); }
            Event::Start(Tag::Item) => {
                flush(&mut out, &mut current);
                let marker = match lists.last_mut() { Some(Some(n)) => { let m = format!("{n}. "); *n += 1; m } _ => "• ".into() };
                current.indent = indent(&lists, quote);
                current.marker = Some(Span::tone(marker, Tone::Dim));
                if quote > 0 { current.fill = Fill::Quote; }
            }
            Event::End(TagEnd::Item) => flush(&mut out, &mut current),
            Event::TaskListMarker(done) => current.spans.push(Span::tone(if done { "☑ " } else { "☐ " }, Tone::Dim)),
            Event::Start(Tag::CodeBlock(kind)) => {
                flush(&mut out, &mut current);
                in_code = true;
                let diff = matches!(&kind, CodeBlockKind::Fenced(lang) if lang.as_ref() == "diff");
                current = Block { indent: indent(&lists, quote), fill: Fill::Code, verbatim: true, ..Block::default() };
                current.marker = diff.then(|| Span::plain(""));
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code = false;
                let diff = current.marker.take().is_some();
                let text: String = current.spans.drain(..).map(|s| s.text).collect();
                for line in text.trim_end_matches('\n').split('\n') {
                    out.push(code_line(line, current.indent, diff));
                }
                current = Block::default();
            }
            Event::Start(Tag::Emphasis) => style.italic = true,
            Event::End(TagEnd::Emphasis) => style.italic = false,
            Event::Start(Tag::Strong) => style.bold = true,
            Event::End(TagEnd::Strong) => style.bold = false,
            Event::Start(Tag::Strikethrough) => style.strike = true,
            Event::End(TagEnd::Strikethrough) => style.strike = false,
            Event::Start(Tag::Link { dest_url, .. }) => link = Some(dest_url.to_string()),
            Event::End(TagEnd::Link) => link = None,
            Event::Start(Tag::Image { dest_url, .. }) => {
                current.spans.push(Span::tone("🖼 ", Tone::Dim));
                link = Some(dest_url.to_string());
            }
            Event::End(TagEnd::Image) => link = None,
            Event::Start(Tag::Table(_)) => flush(&mut out, &mut current),
            Event::Start(Tag::TableHead | Tag::TableRow) => table_row.clear(),
            Event::Start(Tag::TableCell) => table_row.push(Vec::new()),
            Event::End(TagEnd::TableCell) => if let Some(cell) = table_row.last_mut() { cell.append(&mut current.spans); },
            Event::End(TagEnd::TableHead | TagEnd::TableRow) => {
                let head = matches!(event, Event::End(TagEnd::TableHead));
                let mut spans = Vec::new();
                for (i, cell) in table_row.drain(..).enumerate() {
                    if i > 0 { spans.push(Span::tone(" │ ", Tone::Dim)); }
                    spans.extend(cell.into_iter().map(|mut s| { s.style.bold |= head; s }));
                }
                out.push(Block { spans, indent: indent(&lists, quote), ..Block::default() });
            }
            Event::Rule => { flush(&mut out, &mut current); out.push(Block { fill: Fill::Rule, ..Block::default() }); }
            Event::Text(t) if in_code => current.spans.push(Span::plain(t.to_string())),
            Event::Text(t) => push(&mut current, &t, style, &link),
            Event::Code(t) => push(&mut current, &t, Style { code: true, ..style }, &link),
            Event::InlineMath(t) | Event::DisplayMath(t) => push(&mut current, &t, Style { code: true, ..style }, &link),
            Event::Html(t) | Event::InlineHtml(t) => push(&mut current, t.trim_end_matches('\n'), Style { tone: Tone::Dim, ..style }, &link),
            Event::SoftBreak => push(&mut current, " ", style, &link),
            Event::HardBreak => { let (indent, fill) = (current.indent, current.fill); flush(&mut out, &mut current); current.indent = indent; current.fill = fill; }
            Event::FootnoteReference(t) => push(&mut current, &format!("[{t}]"), style, &link),
            _ => {}
        }
    }
    flush(&mut out, &mut current);
    out
}

fn push(block: &mut Block, text: &str, style: Style, link: &Option<String>) {
    let style = if link.is_some() { Style { tone: Tone::Accent, ..style } } else { style };
    let mut span = Span::new(text, style);
    // other schemes stay plain text: a link that cannot open must not look clickable
    span.target = link.clone().filter(|url| crate::open::is_web_link(url)).map(Target::Url);
    block.spans.push(span);
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
    fn continuation_rows_hang_under_the_text_and_spans_are_found_by_column() {
        let mut rows = Vec::new();
        wrap(&markdown("- alpha beta gamma"), 14, 2, &mut rows);
        assert_eq!(rows.iter().map(|r| (r.indent, r.text())).collect::<Vec<_>>(), [(2, "• alpha ".into()), (4, "beta gamma".into())]);
        assert_eq!(rows[1].span_at(4).map(|s| s.text.as_str()), Some("beta gamma"));
        assert_eq!(rows[1].span_at(3), None, "the indent holds no text");
    }
}
