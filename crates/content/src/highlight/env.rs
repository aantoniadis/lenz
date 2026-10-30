//! Environment files (`.env`, `.env.local`, `app.env`…): dotenv-style `KEY=value` lines, read as data.
//!
//! No grammar: a small line lexer, so the rules are exactly these and nothing in a value is ever interpreted.
//! - A line whose first non-blank character is `#` is a comment.
//! - An entry is an optional `export` and blanks, a key (every character up to blanks, `=`, `#` or a quote),
//!   optional blanks, `=`, optional blanks and a value; a key alone (optionally followed by blanks and a comment)
//!   is an entry without a value. Any other line is left unpainted.
//! - A value in double quotes runs to the next unescaped `"`, across lines; a backslash escapes the character after
//!   it (painted as an escape, never decoded). A value in single quotes runs to the next `'`, across lines, with no
//!   escapes. A quote that is never closed in the rest of the text is painted to its line's end only, so typing one
//!   does not repaint the lines below.
//! - An unquoted value runs to the line's end or to a `#` after a blank (a comment); trailing blanks are not part of
//!   it, and a backslash in it is an ordinary character.
//! - `$VAR`, `${VAR}`, `$(…)` and backticks are value text like any other: Lenz never expands, substitutes or runs
//!   them, so they are not painted as references either.
//!
//! Spans use the same per-line, line-relative byte ranges as the grammars' (line endings excluded).
use std::ops::Range;

use super::{Capture, Fold, FoldKind, Folds, Parsed, Span};

/// One entry's byte ranges in the text: the whole entry (from `export` or the key to its value's end), its value
/// (quotes included) and, for a quoted value, the text between the quotes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Entry {
    pub whole: Range<usize>,
    pub value: Range<usize>,
    pub inner: Option<Range<usize>>,
}

/// A text's highlight spans, fold regions (runs of comment lines, multi-line values) and entries.
pub(super) struct Lexed {
    pub parsed: Parsed,
    pub folds: Folds,
    pub entries: Vec<Entry>,
}

/// A quoted value left open at a line's end: its closing quote's byte and where the entry and value began.
struct Open {
    quote: u8,
    close: usize,
    entry: usize,
    value: usize,
    first_line: u32,
}

fn blank(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

fn skip_blanks(b: &[u8], mut i: usize) -> usize {
    while b.get(i).copied().is_some_and(blank) {
        i += 1;
    }
    i
}

/// The byte of the quote closing a value opened before `from`, searching `bytes[from..to]`: for `"`, a backslash
/// takes the byte after it (a multi-byte character's later bytes are never ASCII, so skipping one is safe).
fn closing(bytes: &[u8], from: usize, to: usize, quote: u8) -> Option<usize> {
    let mut i = from;
    while i < to {
        match bytes.get(i).copied() {
            Some(b'\\') if quote == b'"' => i += 2,
            Some(b) if b == quote => return Some(i),
            Some(_) => i += 1,
            None => return None,
        }
    }
    None
}

/// The spans of one line being built: line-relative, sorted, not overlapping.
struct Line<'a> {
    text: &'a str,
    spans: Vec<(Range<usize>, Capture)>,
}

impl Line<'_> {
    fn push(&mut self, range: Range<usize>, capture: Capture) {
        if range.start < range.end {
            self.spans.push((range, capture));
        }
    }

    /// Quoted text `range`: a string, with each backslash and the character after it an escape in double quotes.
    fn quoted(&mut self, range: Range<usize>, quote: u8) {
        if quote != b'"' {
            return self.push(range, Capture::String);
        }
        let bytes = self.text.as_bytes();
        let (mut from, mut i) = (range.start, range.start);
        while i < range.end {
            if bytes.get(i) == Some(&b'\\') {
                let next = self.text.get(i + 1..).and_then(|rest| rest.chars().next()).map_or(0, char::len_utf8);
                let end = (i + 1 + next).min(range.end);
                self.push(from..i, Capture::String);
                self.push(i..end, Capture::Escape);
                (from, i) = (end, end);
            } else {
                i += 1;
            }
        }
        self.push(from..range.end, Capture::String);
    }

    /// After a closing quote at `at`: blanks, then a comment if one starts there; anything else stays unpainted.
    fn after_value(&mut self, at: usize) {
        let bytes = self.text.as_bytes();
        let i = skip_blanks(bytes, at);
        if i > at && bytes.get(i) == Some(&b'#') {
            self.push(i..bytes.len(), Capture::Comment);
        }
    }
}

/// Lex `text`; `None` once `current` says the work is obsolete (checked every 256 lines).
pub(super) fn lex(text: &str, current: &dyn Fn() -> bool) -> Option<Lexed> {
    let starts = crate::lines::line_starts(text);
    let bytes = text.as_bytes();
    let mut parsed = Parsed { offsets: Vec::with_capacity(starts.len() + 1), spans: Vec::new() };
    let mut folds: Vec<Fold> = Vec::new();
    let mut entries: Vec<Entry> = Vec::new();
    let mut open: Option<Open> = None;
    // A quote kind whose search for a closer already reached the text's end: no later value of it closes either.
    let mut unclosed = [false; 2];
    // The first line of the run of comment lines that ends at the current line.
    let mut comments: Option<u32> = None;
    for l in 0..starts.len() {
        if l % 256 == 0 && !current() {
            return None;
        }
        let range = crate::lines::line_range(text, &starts, l);
        let mut line = Line { text: text.get(range.clone()).unwrap_or_default(), spans: Vec::new() };
        let lb = line.text.as_bytes();
        let row = l as u32;
        let mut comment_line = false;
        if let Some(o) = open.take() {
            if o.close >= range.end {
                line.quoted(0..lb.len(), o.quote);
                open = Some(o);
            } else {
                let c = o.close - range.start;
                line.quoted(0..c + 1, o.quote);
                line.after_value(c + 1);
                folds.push(Fold { first: o.first_line, last: row, kind: FoldKind::Block, closes: false });
                entries.push(Entry { whole: o.entry..o.close + 1, value: o.value..o.close + 1, inner: Some(o.value + 1..o.close) });
            }
        } else {
            let i = skip_blanks(lb, 0);
            if lb.get(i) == Some(&b'#') {
                line.push(i..lb.len(), Capture::Comment);
                comment_line = true;
            } else if i < lb.len() {
                open = entry(&mut line, i, range.start, row, bytes, &mut unclosed, &mut entries);
            }
        }
        // Runs of whole-line comments fold as one region.
        match (comment_line, comments) {
            (true, None) => comments = Some(row),
            (false, Some(first)) => {
                if row > first + 1 {
                    folds.push(Fold { first, last: row - 1, kind: FoldKind::Comment, closes: false });
                }
                comments = None;
            }
            _ => {}
        }
        parsed.offsets.push(parsed.spans.len());
        parsed.spans.extend(line.spans.into_iter().map(|(r, capture)| Span { start: r.start as u32, end: r.end as u32, capture }));
    }
    if let Some(first) = comments {
        let last = starts.len().saturating_sub(1) as u32;
        if last > first {
            folds.push(Fold { first, last, kind: FoldKind::Comment, closes: false });
        }
    }
    parsed.offsets.push(parsed.spans.len());
    parsed.spans.shrink_to_fit();
    folds.sort_unstable_by_key(|f| (f.first, std::cmp::Reverse(f.last)));
    folds.dedup_by_key(|f| f.first);
    Some(Lexed { parsed, folds: Folds { regions: folds }, entries })
}

/// An entry starting at line byte `i` (its first non-blank, not a comment) of the line starting at text byte
/// `base`; the quoted value it leaves open, if any.
fn entry(line: &mut Line<'_>, mut i: usize, base: usize, row: u32, text: &[u8], unclosed: &mut [bool; 2], entries: &mut Vec<Entry>) -> Option<Open> {
    let lb = line.text.as_bytes();
    let len = lb.len();
    let start = i;
    let mut spans: Vec<(Range<usize>, Capture)> = Vec::new();
    if lb.get(i..i + 6) == Some(b"export") && lb.get(i + 6).copied().is_some_and(blank) {
        let key = skip_blanks(lb, i + 6);
        // `export` alone (or `export = 1`) is a key named export.
        if lb.get(key).copied().is_some_and(|b| !matches!(b, b'#' | b'=' | b'"' | b'\'')) {
            spans.push((i..i + 6, Capture::Keyword));
            i = key;
        }
    }
    let key = i;
    while lb.get(i).copied().is_some_and(|b| !blank(b) && !matches!(b, b'=' | b'#' | b'"' | b'\'')) {
        i += 1;
    }
    if i == key {
        return None;
    }
    spans.push((key..i, Capture::Property));
    let key_end = i;
    let j = skip_blanks(lb, i);
    match lb.get(j) {
        // A key alone, maybe followed by a comment after a blank.
        None => {}
        Some(b'#') if j > key_end => spans.push((j..len, Capture::Comment)),
        Some(b'=') => {}
        // `KEY#x`, `two words`: not an entry.
        Some(_) => return None,
    }
    if lb.get(j) != Some(&b'=') {
        line.spans.extend(spans);
        entries.push(Entry { whole: base + start..base + key_end, value: base + key_end..base + key_end, inner: None });
        return None;
    }
    spans.push((j..j + 1, Capture::Operator));
    line.spans.extend(spans);
    let v = skip_blanks(lb, j + 1);
    let whole = |end: usize| base + start..base + end;
    match lb.get(v).copied() {
        None => {
            entries.push(Entry { whole: whole(j + 1), value: base + v..base + v, inner: None });
            None
        }
        Some(b'#') if v > j + 1 => {
            line.push(v..len, Capture::Comment);
            entries.push(Entry { whole: whole(j + 1), value: base + j + 1..base + j + 1, inner: None });
            None
        }
        Some(quote @ (b'"' | b'\'')) => {
            if let Some(c) = closing(lb, v + 1, len, quote) {
                line.quoted(v..c + 1, quote);
                line.after_value(c + 1);
                entries.push(Entry { whole: whole(c + 1), value: base + v..base + c + 1, inner: Some(base + v + 1..base + c) });
                return None;
            }
            line.quoted(v..len, quote);
            let kind = usize::from(quote == b'\'');
            let close = if unclosed[kind] { None } else { closing(text, base + len, text.len(), quote) };
            match close {
                Some(close) => Some(Open { quote, close, entry: base + start, value: base + v, first_line: row }),
                None => {
                    unclosed[kind] = true;
                    entries.push(Entry { whole: whole(len), value: base + v..base + len, inner: Some(base + v + 1..base + len) });
                    None
                }
            }
        }
        Some(_) => {
            // To a `#` after a blank, then without trailing blanks.
            let mut end = len;
            let mut k = v;
            while k < len {
                if lb.get(k) == Some(&b'#') && lb.get(k.wrapping_sub(1)).copied().is_some_and(blank) {
                    end = k;
                    break;
                }
                k += 1;
            }
            let comment = end;
            while end > v && lb.get(end - 1).copied().is_some_and(blank) {
                end -= 1;
            }
            line.push(v..end, Capture::String);
            line.push(comment..len, Capture::Comment);
            entries.push(Entry { whole: whole(end), value: base + v..base + end, inner: None });
            None
        }
    }
}

/// The ranges a selection grows through inside `text`'s entries from `range` (smallest first, each strictly
/// holding the one before): a quoted value's inside, the value, the whole entry. The caller adds the word and the
/// whole text.
pub(super) fn steps(text: &str, range: &Range<usize>, current: &dyn Fn() -> bool, push: &mut dyn FnMut(Range<usize>)) -> Option<()> {
    let lexed = lex(text, current)?;
    let holds = |r: &Range<usize>| r.start <= range.start && range.end <= r.end;
    if let Some(entry) = lexed.entries.iter().find(|e| holds(&e.whole)) {
        if let Some(inner) = entry.inner.clone().filter(|r| r.start < r.end && holds(r)) {
            push(inner);
        }
        if holds(&entry.value) {
            push(entry.value.clone());
        }
        push(entry.whole.clone());
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use Capture::{Comment, Escape, Keyword, Operator, Property, String as Str};

    fn painted(text: &str) -> Vec<Vec<(&str, Capture)>> {
        let lexed = lex(text, &|| true).unwrap();
        let starts = crate::lines::line_starts(text);
        lexed.parsed.spans(0..starts.len()).into_iter().enumerate().map(|(l, spans)| {
            let line = &text[crate::lines::line_range(text, &starts, l)];
            spans.into_iter().map(|(r, c)| (&line[r], c)).collect()
        }).collect()
    }

    #[test]
    fn keys_operators_values_comments_and_export() {
        let text = "# Database\nexport DB_USER=admin # who\nDB_PASS = 's3cr$t'  # quoted\nEMPTY=\nBARE\n  INDENTED=1\n";
        assert_eq!(painted(text), vec![
            vec![("# Database", Comment)],
            vec![("export", Keyword), ("DB_USER", Property), ("=", Operator), ("admin", Str), ("# who", Comment)],
            vec![("DB_PASS", Property), ("=", Operator), ("'s3cr$t'", Str), ("# quoted", Comment)],
            vec![("EMPTY", Property), ("=", Operator)],
            vec![("BARE", Property)],
            vec![("INDENTED", Property), ("=", Operator), ("1", Str)],
            vec![],
        ]);
    }

    #[test]
    fn references_commands_and_backticks_stay_value_text() {
        let text = "A=$HOME/${USER}:$(rm -rf ~)`id`\nB=\"$(whoami) ${X:-y}\"\nC=a#not-a-comment\n";
        assert_eq!(painted(text), vec![
            vec![("A", Property), ("=", Operator), ("$HOME/${USER}:$(rm -rf ~)`id`", Str)],
            vec![("B", Property), ("=", Operator), ("\"$(whoami) ${X:-y}\"", Str)],
            vec![("C", Property), ("=", Operator), ("a#not-a-comment", Str)],
            vec![],
        ]);
    }

    #[test]
    fn double_quotes_escape_single_quotes_do_not() {
        let text = "A=\"line\\n \\\"q\\\" \\é\"\nB='no \\escape'\nC=un\\quoted\n";
        assert_eq!(painted(text), vec![
            vec![("A", Property), ("=", Operator), ("\"line", Str), ("\\n", Escape), (" ", Str), ("\\\"", Escape), ("q", Str), ("\\\"", Escape), (" ", Str), ("\\é", Escape), ("\"", Str)],
            vec![("B", Property), ("=", Operator), ("'no \\escape'", Str)],
            vec![("C", Property), ("=", Operator), ("un\\quoted", Str)],
            vec![],
        ]);
    }

    #[test]
    fn quoted_values_continue_across_crlf_and_lf_lines_and_fold() {
        let text = "KEY=\"-----BEGIN\r\nMIIB==\nend\" # pem\r\nNEXT='a\nb'\nZ=1\n";
        assert_eq!(painted(text), vec![
            vec![("KEY", Property), ("=", Operator), ("\"-----BEGIN", Str)],
            vec![("MIIB==", Str)],
            vec![("end\"", Str), ("# pem", Comment)],
            vec![("NEXT", Property), ("=", Operator), ("'a", Str)],
            vec![("b'", Str)],
            vec![("Z", Property), ("=", Operator), ("1", Str)],
            vec![],
        ]);
        let lexed = lex(text, &|| true).unwrap();
        let regions: Vec<_> = lexed.folds.regions.iter().map(|f| (f.first, f.last, f.kind)).collect();
        assert_eq!(regions, vec![(0, 2, FoldKind::Block), (3, 4, FoldKind::Block)]);
    }

    #[test]
    fn an_unclosed_quote_paints_only_its_own_line() {
        let text = "A=\"open\nB=2\nC='also\nD=4\n";
        assert_eq!(painted(text), vec![
            vec![("A", Property), ("=", Operator), ("\"open", Str)],
            vec![("B", Property), ("=", Operator), ("2", Str)],
            vec![("C", Property), ("=", Operator), ("'also", Str)],
            vec![("D", Property), ("=", Operator), ("4", Str)],
            vec![],
        ]);
    }

    #[test]
    fn lines_that_are_not_entries_stay_unpainted() {
        let text = "two words\nKEY#x\n=value\nexport\nexport  # c\nexport = 1\n\u{1F600}=\u{00E9}\n";
        assert_eq!(painted(text), vec![
            vec![],
            vec![],
            vec![],
            vec![("export", Property)],
            vec![("export", Property), ("# c", Comment)],
            vec![("export", Property), ("=", Operator), ("1", Str)],
            vec![("\u{1F600}", Property), ("=", Operator), ("\u{00E9}", Str)],
            vec![],
        ]);
    }

    #[test]
    fn comment_runs_fold() {
        let text = "# a\n# b\n  # c\nA=1\n# alone\nB=2\n# x\n# y";
        let regions: Vec<_> = lex(text, &|| true).unwrap().folds.regions.iter().map(|f| (f.first, f.last, f.kind)).collect();
        assert_eq!(regions, vec![(0, 2, FoldKind::Comment), (6, 7, FoldKind::Comment)]);
    }

    #[test]
    fn selection_steps_grow_from_the_inside_of_a_value_to_its_entry() {
        let text = "export A=\"one two\"\nB=x\n";
        let at = text.find("two").unwrap();
        let mut out = Vec::new();
        steps(text, &(at..at), &|| true, &mut |r| out.push(r)).unwrap();
        let shown: Vec<_> = out.iter().map(|r| &text[r.clone()]).collect();
        assert_eq!(shown, ["one two", "\"one two\"", "export A=\"one two\""]);
    }

    #[test]
    fn many_open_quotes_stay_linear() {
        // Each line opens a quote: the next line's quote closes it (a two-line value), and a search that reaches the
        // text's end is not repeated for the quotes after it.
        let text: std::string::String = (0..20_000).map(|i| format!("K{i}=\"v\n")).chain((0..20_000).map(|i| format!("S{i}='v\nT{i}=\"w\n"))).collect();
        let started = std::time::Instant::now();
        let lexed = lex(&text, &|| true).unwrap();
        assert_eq!(lexed.entries.first().map(|e| &text[e.whole.clone()]), Some("K0=\"v\nK1=\""));
        assert_eq!(lexed.parsed.offsets.len(), 60_002, "one span list per line");
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "{:?}", started.elapsed());
    }

    #[test]
    fn arbitrary_bytes_never_panic_and_spans_stay_on_char_boundaries() {
        let pieces = ["\"", "'", "\\", "#", "=", " ", "\t", "\r\n", "\n", "\r", "export ", "K", "é", "\u{1F600}", "\u{FFFD}", "`", "$"];
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..2_000 {
            let mut text = std::string::String::new();
            for _ in 0..24 {
                seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17;
                text.push_str(pieces[(seed % pieces.len() as u64) as usize]);
            }
            let lexed = lex(&text, &|| true).unwrap();
            let starts = crate::lines::line_starts(&text);
            for (l, spans) in lexed.parsed.spans(0..starts.len()).into_iter().enumerate() {
                let line = &text[crate::lines::line_range(&text, &starts, l)];
                let mut last = 0;
                for (r, _) in spans {
                    assert!(r.start >= last && r.start < r.end && r.end <= line.len() && line.is_char_boundary(r.start) && line.is_char_boundary(r.end), "{text:?} line {l}: {r:?}");
                    last = r.end;
                }
            }
            for e in &lexed.entries {
                assert!(e.whole.end <= text.len() && text.is_char_boundary(e.whole.start) && text.is_char_boundary(e.whole.end), "{text:?}: {e:?}");
            }
        }
    }
}
