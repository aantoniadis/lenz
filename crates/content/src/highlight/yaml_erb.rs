//! YAML that Rails reads through ERB (`config/database.yml` and the like): `<% if … %>`, `<%= ENV["X"] %>` and
//! `<% end %>` are not YAML, so the YAML grammar (and a YAML language server) would report them as errors and lose
//! the structure after them. Such a document is parsed as a *masked* copy of the same length, line for line and
//! character for character: a tag alone on its line becomes a YAML comment, an output tag inside a line a plain
//! scalar of underscores, any other tag blanks. Every byte keeps its offset and every non-ASCII character stays, so
//! byte, UTF-8 and UTF-16 positions are the original's. The tags' Ruby is highlighted as Ruby (all tags' code read
//! together, so an `if` in one tag and its `end` in another are one statement), their delimiters as tags.
use std::ops::Range;

use super::{Capture, Parsed, Span, Syntax, Want};

/// The most tags read before the document is left as plain YAML.
const MAX_TAGS: usize = 20_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// `<% … %>`, `<%- … -%>`.
    Code,
    /// `<%= … %>`, `<%== … %>`.
    Output,
    /// `<%# … %>`.
    Comment,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Tag {
    pub range: Range<usize>,
    pub code: Range<usize>,
    pub kind: Kind,
}

/// The ERB tags in `text`, in order. `<%%` is ERB's literal `<%` and opens nothing; an unclosed tag is left as text,
/// so the YAML error stays where it is.
pub(super) fn tags(text: &str) -> Vec<Tag> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = find(b, at, b"<%") {
        let after = |k: usize| b.get(i + k).copied();
        if after(2) == Some(b'%') { at = i + 3; continue; }
        let (open, kind) = match after(2) {
            Some(b'=') if after(3) == Some(b'=') => (4, Kind::Output),
            Some(b'=') => (3, Kind::Output),
            Some(b'#') => (3, Kind::Comment),
            Some(b'-') => (3, Kind::Code),
            _ => (2, Kind::Code),
        };
        let Some(j) = find(b, i + open, b"%>") else { break };
        let close = if j > i + open && b[j - 1] == b'-' { j - 1 } else { j };
        out.push(Tag { range: i..j + 2, code: i + open..close, kind });
        if out.len() > MAX_TAGS { return Vec::new(); }
        at = j + 2;
    }
    out
}

fn find(b: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    b.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

fn line_start(b: &[u8], at: usize) -> usize { b[..at].iter().rposition(|&c| c == b'\n').map_or(0, |i| i + 1) }
fn line_end(b: &[u8], at: usize) -> usize { b[at..].iter().position(|&c| c == b'\n' || c == b'\r').map_or(b.len(), |i| at + i) }
fn blank(b: &[u8]) -> bool { b.iter().all(|&c| c == b' ' || c == b'\t') }

/// The masked copy the YAML grammar and a YAML language server read (see the module's description).
pub(super) fn mask(text: &str, tags: &[Tag]) -> String {
    let mut out = text.as_bytes().to_vec();
    let b = text.as_bytes();
    for tag in tags {
        let alone = blank(&b[line_start(b, tag.range.start)..tag.range.start]) && blank(&b[tag.range.end..line_end(b, tag.range.end)]);
        let mut first_on_line = true;
        for i in tag.range.clone() {
            match b[i] {
                b'\n' | b'\r' => { first_on_line = true; continue; }
                c if c >= 0x80 => { first_on_line = false; continue; } // non-ASCII stays: positions in every encoding hold
                b' ' | b'\t' if alone && first_on_line => continue,     // a comment line keeps its indentation
                _ => {}
            }
            out[i] = if alone {
                if std::mem::replace(&mut first_on_line, false) { b'#' } else { b' ' }
            } else if tag.kind == Kind::Output { b'_' } else { b' ' };
        }
    }
    // Only ASCII bytes were replaced, by ASCII: the result is still UTF-8.
    String::from_utf8(out).unwrap_or_else(|_| text.to_owned())
}

/// The tags' code where it stands, everything else blank: what the Ruby grammar reads to highlight it.
fn ruby_view(text: &str, tags: &[Tag]) -> String {
    let b = text.as_bytes();
    let mut out: Vec<u8> = b.iter().map(|&c| if c == b'\n' || c == b'\r' { c } else { b' ' }).collect();
    for tag in tags.iter().filter(|t| t.kind != Kind::Comment) {
        out[tag.code.clone()].copy_from_slice(&b[tag.code.clone()]);
    }
    String::from_utf8(out).unwrap_or_default()
}

/// `text`'s masked copy when it holds ERB tags, for a YAML language server to read.
pub fn masked(text: &str) -> Option<String> {
    if !text.contains("<%") { return None; }
    let tags = tags(text);
    (!tags.is_empty()).then(|| mask(text, &tags))
}

/// Parse YAML holding ERB tags (see the module's description); `None` when it holds none, or was superseded.
pub(super) fn parse(text: &str, current: &dyn Fn() -> bool, want: Want, grammar: impl Fn(super::Language, &str, Want) -> Option<Syntax>) -> Option<Option<Syntax>> {
    if !text.contains("<%") { return None; }
    let tags = tags(text);
    if tags.is_empty() { return None; }
    let mut syntax = match grammar(super::Language::Yaml, &mask(text, &tags), want) { Some(s) => s, None => return Some(None) };
    if !current() { return Some(None); }
    let ruby = grammar(super::Language::Ruby, &ruby_view(text, &tags), Want::default()).map(|s| s.parsed);
    syntax.parsed = merge(text, &syntax.parsed, ruby.as_ref(), &tags);
    Some(Some(syntax))
}

/// Each line's YAML spans with the tags taken out, and the tags painted in: delimiters as tags, a comment's text as
/// a comment, code as the Ruby grammar painted it.
fn merge(text: &str, yaml: &Parsed, ruby: Option<&Parsed>, tags: &[Tag]) -> Parsed {
    let starts = crate::lines::line_starts(text);
    let n = yaml.offsets.len().saturating_sub(1);
    let mut out = Parsed { offsets: Vec::with_capacity(n + 1), spans: Vec::new(), limited: yaml.limited };
    let mut first = 0;
    for line in 0..n {
        out.offsets.push(out.spans.len());
        let base = starts.get(line).copied().unwrap_or(text.len());
        let end = base + crate::lines::line_range(text, &starts, line).len();
        while first < tags.len() && tags[first].range.end <= base { first += 1; }
        let here: Vec<&Tag> = tags[first..].iter().take_while(|t| t.range.start < end).filter(|t| t.range.end > base).collect();
        let yaml_line = &yaml.spans[yaml.offsets[line]..yaml.offsets[line + 1]];
        if here.is_empty() {
            out.spans.extend(yaml_line.iter().map(|s| Span { start: s.start, end: s.end, capture: s.capture }));
            continue;
        }
        let rel = |r: Range<usize>| r.start.max(base) - base..r.end.min(end).max(base) - base;
        let pieces: Vec<Range<usize>> = here.iter().map(|t| rel(t.range.clone())).filter(|r| !r.is_empty()).collect();
        let mut line_spans: Vec<(Range<usize>, Capture)> = Vec::new();
        // The YAML spans, cut around the tags.
        for s in yaml_line {
            let mut parts: Vec<Range<usize>> = std::iter::once(s.start as usize..s.end as usize).collect();
            for p in &pieces {
                parts = parts.into_iter().flat_map(|r| [r.start..r.end.min(p.start), r.start.max(p.end)..r.end]).filter(|r| !r.is_empty()).collect();
            }
            line_spans.extend(parts.into_iter().map(|r| (r, s.capture)));
        }
        let ruby_line = ruby.and_then(|r| r.offsets.get(line).zip(r.offsets.get(line + 1)).map(|(a, b)| &r.spans[*a..*b])).unwrap_or_default();
        for tag in here {
            let code = rel(tag.code.clone());
            let whole = rel(tag.range.clone());
            for delimiter in [whole.start..code.start.max(whole.start), code.end.min(whole.end)..whole.end] {
                if !delimiter.is_empty() { line_spans.push((delimiter, Capture::Tag)); }
            }
            if code.is_empty() { continue; }
            match tag.kind {
                Kind::Comment => line_spans.push((code, Capture::Comment)),
                Kind::Code | Kind::Output => line_spans.extend(ruby_line.iter()
                    .map(|s| s.start as usize..s.end as usize)
                    .zip(ruby_line.iter().map(|s| s.capture))
                    .map(|(r, c)| (r.start.max(code.start)..r.end.min(code.end), c))
                    .filter(|(r, _)| !r.is_empty())),
            }
        }
        line_spans.sort_by_key(|(r, _)| (r.start, r.end));
        out.spans.extend(line_spans.into_iter().map(|(r, capture)| Span { start: r.start as u32, end: r.end as u32, capture }));
    }
    out.offsets.push(out.spans.len());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATABASE: &str = "default: &default\n  adapter: postgresql\n  pool: <%= ENV.fetch(\"RAILS_MAX_THREADS\") { 5 } %>\n\ndevelopment:\n  <<: *default\n  database: app_dev\n<% if ENV[\"CI\"] %>\n  host: <%= ENV[\"DB_HOST\"] %>\n<% end %>\n";

    #[test]
    fn masking_keeps_every_offset_and_makes_tags_comments_or_scalars() {
        let tags = tags(DATABASE);
        assert_eq!(tags.iter().map(|t| t.kind).collect::<Vec<_>>(), [Kind::Output, Kind::Code, Kind::Output, Kind::Code]);
        let masked = mask(DATABASE, &tags);
        assert_eq!(masked.len(), DATABASE.len());
        assert_eq!(masked.lines().count(), DATABASE.lines().count());
        let pool = "<%= ENV.fetch(\"RAILS_MAX_THREADS\") { 5 } %>";
        assert!(masked.contains(&format!("  pool: {}\n", "_".repeat(pool.len()))), "an output tag in a value is a scalar: {masked}");
        for tag in ["<% if ENV[\"CI\"] %>", "<% end %>"] {
            assert!(masked.contains(&format!("\n#{}\n", " ".repeat(tag.len() - 1))), "a control tag alone is a comment line: {masked:?}");
        }
        assert!(!masked.contains("<%"));
        // Non-ASCII stays where it was: UTF-16 positions after it hold.
        let text = "a: <%= t(\"héllo\") %> # é\n";
        let masked = mask(text, &super::tags(text));
        assert_eq!(masked.encode_utf16().count(), text.encode_utf16().count());
        assert!(masked.contains('é') && masked.starts_with(&format!("a: {}é", "_".repeat("<%= t(\"h".len()))), "{masked}");
    }

    #[test]
    fn literals_unclosed_tags_and_comment_tags() {
        assert!(tags("a: <%% literal").is_empty());
        assert!(tags("a: <%= open").is_empty(), "unclosed: left as text");
        let t = tags("<%# note %>\n<%- x -%>");
        assert_eq!(t.iter().map(|t| (t.kind, t.code.clone())).collect::<Vec<_>>(), [(Kind::Comment, 3..9), (Kind::Code, 15..18)]);
        assert_eq!(masked("plain: yes\n"), None);
    }

    #[test]
    fn yaml_with_erb_parses_without_errors_and_paints_the_tags() {
        let syntax = super::super::parse_with(super::super::Language::Yaml, DATABASE.into(), Want { folds: true, declarations: false }, || true).unwrap();
        assert_eq!(syntax.errors.as_deref(), Some(&[][..]), "the tags are not YAML errors");
        let lines = syntax.parsed.spans(0..12);
        let line = |n: usize| DATABASE.lines().nth(n).unwrap();
        let painted = |n: usize, cap: Capture| lines[n].iter().filter(|(_, c)| *c == cap).map(|(r, _)| &line(n)[r.clone()]).collect::<Vec<_>>();
        assert_eq!(painted(2, Capture::Tag), ["<%=", "%>"]);
        assert!(painted(2, Capture::Property).contains(&"pool"), "YAML keys around tags stay keys: {:?}", lines[2]);
        assert!(painted(2, Capture::String).iter().any(|s| s.contains("RAILS_MAX_THREADS")), "the tag's Ruby string: {:?}", lines[2]);
        assert_eq!(painted(7, Capture::Tag), ["<%", "%>"]);
        assert!(painted(7, Capture::Keyword).contains(&"if"));
        assert!(painted(9, Capture::Keyword).contains(&"end"), "`end` closes the `if` across tags");
        assert!(painted(8, Capture::Property).contains(&"host"));
        // Without the masking, the same text has errors.
        let raw = super::super::parse_grammar(super::super::Language::Yaml, DATABASE, &|| true, Want::default()).unwrap();
        assert!(raw.errors.is_some_and(|e| !e.is_empty()), "the grammar alone rejects ERB");
    }
}
