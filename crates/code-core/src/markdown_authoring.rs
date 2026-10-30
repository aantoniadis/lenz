//! Markdown authoring help Lenz gives itself, with no language server: list continuation on Enter, the headings a
//! link fragment can name (with GitHub-style anchors) and the link destination being typed at the caret. Every
//! function reads only the lines it needs of an immutable snapshot (or one string), never the whole document as one
//! copy.

use std::{collections::HashMap, ops::Range};

use crate::text::{TextEdit, TextSnapshot};

/// How many lines above the caret Enter reads to tell whether it is inside a fenced code block; a fence further up
/// is not seen, and the line is taken as prose.
pub(crate) const FENCE_LOOKBACK: usize = 5_000;
/// How many following lines a continued numbered list is renumbered over.
pub(crate) const RENUMBER_LINES: usize = 1_000;
/// The largest file whose headings a link fragment is offered or followed to (the preview's own source limit).
pub(crate) const HEADING_BYTES: usize = 4 << 20;

/// A list item's line, as Enter reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Item {
    /// The blockquote markers before it (with the space after the last one).
    quote: usize,
    /// Where its marker starts (after the quote and the item's indentation).
    marker: usize,
    /// Its marker: a bullet, or a number with its delimiter.
    kind: Kind,
    /// Where its text starts: after the marker, its spacing and a task box.
    content: usize,
    /// The spacing written after the marker (kept for the next item).
    spacing: Range<usize>,
    task: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Bullet(char),
    Ordered { number: u64, digits: usize, delimiter: char },
}

/// The list item `line` starts, if it is one (not a thematic break such as `- - -` or `***`).
fn item(line: &str) -> Option<Item> {
    let bytes = line.as_bytes();
    let mut i = 0;
    let mut quote = 0;
    loop {
        let indent = bytes[i..].iter().take_while(|b| **b == b' ').count();
        if indent <= 3 && bytes.get(i + indent) == Some(&b'>') {
            i += indent + 1;
            if bytes.get(i) == Some(&b' ') { i += 1; }
            quote = i;
        } else {
            break;
        }
    }
    let marker = i + bytes[i..].iter().take_while(|b| matches!(b, b' ' | b'\t')).count();
    let after = |end: usize| bytes.get(end).is_none_or(|b| matches!(b, b' ' | b'\t'));
    let (kind, end) = match bytes.get(marker)? {
        c @ (b'-' | b'*' | b'+') if after(marker + 1) => (Kind::Bullet(*c as char), marker + 1),
        b'0'..=b'9' => {
            let digits = bytes[marker..].iter().take_while(|b| b.is_ascii_digit()).count();
            let delimiter = *bytes.get(marker + digits)?;
            if digits > 9 || !matches!(delimiter, b'.' | b')') || !after(marker + digits + 1) { return None; }
            let number = line.get(marker..marker + digits)?.parse().ok()?;
            (Kind::Ordered { number, digits, delimiter: delimiter as char }, marker + digits + 1)
        }
        _ => return None,
    };
    if let Kind::Bullet(c) = kind && thematic_break(line.get(marker..).unwrap_or_default(), c) { return None; }
    let spaces = bytes[end..].iter().take_while(|b| matches!(b, b' ' | b'\t')).count();
    // Five or more spaces start indented code inside the item: the next item gets one.
    let spacing = end..end + if spaces > 4 { 1 } else { spaces };
    let mut content = end + spaces;
    let rest = line.get(content..).unwrap_or_default();
    let task = (rest.starts_with("[ ]") || rest.starts_with("[x]") || rest.starts_with("[X]")) && after(content + 3) && spaces > 0;
    if task { content += 3 + bytes[content + 3..].iter().take_while(|b| matches!(b, b' ' | b'\t')).count(); }
    Some(Item { quote, marker, kind, content, spacing, task })
}

/// `-- -`, `* * *`: three or more of the bullet character and nothing else but blanks.
fn thematic_break(rest: &str, c: char) -> bool {
    rest.chars().all(|x| x == c || x == ' ' || x == '\t') && rest.chars().filter(|x| *x == c).count() >= 3
}

/// A code fence line (```` ``` ```` or `~~~`, at most three spaces in, inside blockquotes too).
fn fence(line: &str) -> bool {
    let mut rest = line;
    loop {
        let trimmed = rest.trim_start_matches(' ');
        if rest.len() - trimmed.len() > 3 { return false; }
        match trimmed.strip_prefix('>') {
            Some(inner) => rest = inner,
            None => return trimmed.starts_with("```") || trimmed.starts_with("~~~"),
        }
    }
}

/// Whether line `line` lies inside a fenced code block, judged from at most [`FENCE_LOOKBACK`] lines above it.
fn in_code(doc: &TextSnapshot, line: usize) -> bool {
    let from = line.saturating_sub(FENCE_LOOKBACK);
    let fences = (from..line).filter(|l| doc.line_range(*l).and_then(|r| doc.range(r)).is_some_and(|text| fence(&text))).count();
    fences % 2 == 1
}

/// The edits Enter makes at `caret` in a Markdown draft when its line is a list item: the next item's marker (a
/// bullet again, the next number, an open task box) after the newline, and with `renumber` the numbers of the same
/// list's following items moved up by one; on an item with no text yet, the marker removed instead, which ends the
/// list. `None`: not after an item's marker, inside a code block, or not a list at all (Enter types a plain newline).
pub(crate) fn list_enter(doc: &TextSnapshot, caret: usize, newline: &str, renumber: bool) -> Option<Vec<TextEdit>> {
    let line = doc.line_of(caret);
    let range = doc.line_range(line)?;
    let text = doc.range(range.clone())?;
    let item = item(&text)?;
    let column = caret.checked_sub(range.start)?;
    if column < item.content || in_code(doc, line) { return None; }
    if text.get(item.content..).unwrap_or_default().trim().is_empty() {
        // An empty item ends the list: its marker (and indentation) goes, any blockquote stays.
        return Some(vec![TextEdit { range: range.start + item.quote..range.end, text: "".into() }]);
    }
    let lead = text.get(..item.marker).unwrap_or_default();
    let spacing = match text.get(item.spacing.clone()).unwrap_or_default() { "" => " ", s => s };
    let (marker, mut edits) = match item.kind {
        Kind::Bullet(c) => (c.to_string(), Vec::new()),
        Kind::Ordered { number, delimiter, .. } => {
            let lazy = neighbour_number(doc, line, &item, lead, false) == Some(number) || neighbour_number(doc, line, &item, lead, true) == Some(number);
            let next = if lazy { number } else { number + 1 };
            let edits = if renumber && !lazy { renumbered(doc, line, &item, lead, next) } else { Vec::new() };
            (format!("{next}{delimiter}"), edits)
        }
    };
    let task = if item.task { "[ ] " } else { "" };
    edits.insert(0, TextEdit { range: caret..caret, text: format!("{newline}{lead}{marker}{spacing}{task}").into() });
    Some(edits)
}

/// The number of the same list's item just above (`up`) or below `line`, skipping blank and more deeply indented
/// lines; `None` when the list ends first.
fn neighbour_number(doc: &TextSnapshot, line: usize, item: &Item, lead: &str, up: bool) -> Option<u64> {
    let mut l = line;
    for _ in 0..RENUMBER_LINES {
        l = if up { l.checked_sub(1)? } else { l + 1 };
        let text = doc.range(doc.line_range(l)?)?;
        match sibling(&text, item, lead) {
            Sibling::Skip => continue,
            Sibling::Item(Kind::Ordered { number, .. }) => return Some(number),
            Sibling::Item(_) | Sibling::End => return None,
        }
    }
    None
}

enum Sibling { Skip, Item(Kind), End }

/// How a line after (or before) `item` relates to its list: another item at the same depth, a line inside the
/// list to skip (blank, or indented deeper), or the list's end.
fn sibling(text: &str, item: &Item, lead: &str) -> Sibling {
    if text.trim().is_empty() { return Sibling::Skip; }
    match self::item(text) {
        Some(other) if other.marker == item.marker && text.get(..other.marker) == Some(lead) => match (other.kind, item.kind) {
            (Kind::Ordered { delimiter: a, .. }, Kind::Ordered { delimiter: b, .. }) if a == b => Sibling::Item(other.kind),
            (Kind::Bullet(a), Kind::Bullet(b)) if a == b => Sibling::Item(other.kind),
            _ => Sibling::End,
        },
        Some(other) if other.marker > item.marker => Sibling::Skip,
        Some(_) => Sibling::End,
        // Continuation text indented to the item's content belongs to it.
        None => if text.len() - text.trim_start().len() >= item.content.min(item.marker + 2) { Sibling::Skip } else { Sibling::End },
    }
}

/// The following items of a numbered list that count on from the continued one, each moved up by one so the new
/// item `next` fits: they renumber only while they are in sequence (a list that skips numbers is left alone from
/// there).
fn renumbered(doc: &TextSnapshot, line: usize, item: &Item, lead: &str, next: u64) -> Vec<TextEdit> {
    let mut edits = Vec::new();
    let mut expected = next;
    for l in line + 1..(line + 1 + RENUMBER_LINES).min(doc.line_count()) {
        let Some(range) = doc.line_range(l) else { break };
        let Some(text) = doc.range(range.clone()) else { break };
        match sibling(&text, item, lead) {
            Sibling::Skip => continue,
            Sibling::Item(Kind::Ordered { number, digits, .. }) if number == expected => {
                let at = range.start + item.marker;
                edits.push(TextEdit { range: at..at + digits, text: (number + 1).to_string().into() });
                expected += 1;
            }
            Sibling::Item(_) | Sibling::End => break,
        }
    }
    edits
}

/// A heading of a Markdown document: its level, its text as written (without its `#` marks), its anchor and the
/// line it is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    pub text: String,
    pub anchor: String,
    pub line: usize,
}

/// The headings of a document given line by line (ATX `#` headings and setext underlines), outside front matter
/// and fenced code, with GitHub's anchors: a repeated anchor is numbered `-1`, `-2`… in order. `current` stops the
/// walk (`None` then).
pub(crate) fn headings<'a>(lines: impl Iterator<Item = std::borrow::Cow<'a, str>>, current: impl Fn() -> bool) -> Option<Vec<Heading>> {
    let mut found = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut add = |level: u8, text: &str, line: usize, found: &mut Vec<Heading>| {
        let base = anchor(text);
        let n = seen.entry(base.clone()).or_insert(0);
        let anchor = if *n == 0 { base } else { format!("{base}-{n}") };
        *n += 1;
        found.push(Heading { level, text: text.to_owned(), anchor, line });
    };
    let mut fenced: Option<(char, usize)> = None;
    let mut front = false;
    // The paragraph line a setext underline would make a heading.
    let mut paragraph: Option<(usize, String)> = None;
    for (i, line) in lines.enumerate() {
        if i % 4096 == 0 && !current() { return None; }
        let trimmed = line.trim_start_matches(' ');
        let indent = line.len() - trimmed.len();
        if i == 0 && line.trim_end() == "---" { front = true; continue; }
        if front {
            if matches!(line.trim_end(), "---" | "...") { front = false; }
            continue;
        }
        if let Some((c, n)) = fenced {
            let run = trimmed.chars().take_while(|x| *x == c).count();
            if indent <= 3 && run >= n && trimmed.get(run * c.len_utf8()..).unwrap_or_default().trim().is_empty() { fenced = None; }
            continue;
        }
        if indent <= 3 && (trimmed.starts_with("```") || trimmed.starts_with("~~~")) {
            let c = trimmed.chars().next().unwrap_or('`');
            fenced = Some((c, trimmed.chars().take_while(|x| *x == c).count()));
            paragraph = None;
            continue;
        }
        if indent <= 3 && let Some(level) = atx(trimmed) {
            let text = atx_text(trimmed.get(level as usize..).unwrap_or_default());
            add(level, text, i, &mut found);
            paragraph = None;
            continue;
        }
        if indent <= 3 && let Some(level) = setext(trimmed) && let Some((at, text)) = paragraph.take() {
            add(level, &text, at, &mut found);
            continue;
        }
        paragraph = (indent <= 3 && !trimmed.trim().is_empty() && item(&line).is_none() && !trimmed.starts_with('>') && !trimmed.starts_with('|'))
            .then(|| (i, trimmed.trim().to_owned()));
    }
    Some(found)
}

/// The level of the ATX heading a line (indentation removed) opens.
fn atx(trimmed: &str) -> Option<u8> {
    let marks = trimmed.bytes().take_while(|b| *b == b'#').count();
    ((1..=6).contains(&marks) && trimmed.as_bytes().get(marks).is_none_or(|b| matches!(b, b' ' | b'\t'))).then_some(marks as u8)
}

/// An ATX heading's text: trimmed, without its closing `#` run.
fn atx_text(rest: &str) -> &str {
    let text = rest.trim();
    let closed = text.trim_end_matches('#');
    if closed.len() == text.len() { return text; }
    if closed.is_empty() { return ""; }
    if closed.ends_with([' ', '\t']) { closed.trim_end() } else { text }
}

/// The setext level an underline line gives the paragraph line above it.
fn setext(trimmed: &str) -> Option<u8> {
    let line = trimmed.trim_end();
    if line.is_empty() { return None; }
    if line.bytes().all(|b| b == b'=') { return Some(1); }
    if line.bytes().all(|b| b == b'-') { return Some(2); }
    None
}

/// GitHub's anchor for a heading's text: its inline marks (`` ` ``, `*`, `~`, a link's destination) dropped,
/// lower-cased, letters, digits, `-` and `_` kept, spaces made `-`, everything else removed.
pub fn anchor(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        // `[label](destination)` reads as its label.
        if c == ']' && rest.get(1..).is_some_and(|r| r.starts_with('(')) && let Some(end) = rest.find(')') {
            rest = rest.get(end + 1..).unwrap_or_default();
            continue;
        }
        plain.push(c);
        rest = rest.get(c.len_utf8()..).unwrap_or_default();
    }
    let mut out = String::with_capacity(plain.len());
    for c in plain.chars() {
        if c.is_alphanumeric() || c == '-' || c == '_' { out.extend(c.to_lowercase()); }
        else if c == ' ' { out.push('-'); }
    }
    out
}

/// The link destination being typed at the end of `before` (a line's text up to the caret): `[text](` or
/// `![alt](` followed by the destination so far, with no blank, `)` or `<` in it, outside inline code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Destination<'a> {
    /// Where the destination starts in `before`.
    pub start: usize,
    /// The path typed so far (before any `#`).
    pub path: &'a str,
    /// The fragment typed after `#`, once one is.
    pub fragment: Option<&'a str>,
}

impl Destination<'_> {
    /// Where the part being completed starts in the line: after the fragment's `#`, after the path's last `/`, or
    /// at the destination's start.
    pub fn word(&self) -> usize {
        match self.fragment {
            Some(_) => self.start + self.path.len() + 1,
            None => self.start + self.path.rfind('/').map_or(0, |i| i + 1),
        }
    }
}

pub(crate) fn destination(before: &str) -> Option<Destination<'_>> {
    let open = before.rfind("](")?;
    let start = open + 2;
    let typed = before.get(start..)?;
    if typed.contains([' ', '\t', ')', '(', '<', '>', '[', ']', '"', '\'']) { return None; }
    // Inside inline code (an odd number of backticks before it), `](` is literal text.
    if before.get(..open)?.matches('`').count() % 2 == 1 { return None; }
    let (path, fragment) = match typed.split_once('#') {
        Some((path, fragment)) if !fragment.contains('#') => (path, Some(fragment)),
        Some(_) => return None,
        None => (typed, None),
    };
    // A scheme (`https:`, `mailto:`) or a protocol-relative URL is not a file.
    if path.starts_with("//") || path.split('/').next().is_some_and(|first| first.contains(':')) { return None; }
    Some(Destination { start, path, fragment })
}

/// `path` (a link's path part, `/`-separated, `%`-escaped) resolved against folder `dir` (relative to the root,
/// `""` for the root itself); a leading `/` starts at the root. `None` when it leaves the root or is malformed.
pub(crate) fn resolve(dir: &str, path: &str) -> Option<String> {
    let mut parts: Vec<String> = if path.starts_with('/') { Vec::new() } else { dir.split('/').filter(|p| !p.is_empty()).map(str::to_owned).collect() };
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => { parts.pop()?; }
            part => parts.push(decode(part)?),
        }
    }
    Some(parts.join("/"))
}

/// `%XX` escapes decoded (UTF-8); `None` when one is malformed.
pub(crate) fn decode(part: &str) -> Option<String> {
    if !part.contains('%') { return Some(part.to_owned()); }
    let bytes = part.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = part.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// A file or folder name as a link destination writes it: blanks, parentheses, `#`, `%` and `<>` escaped.
pub(crate) fn encode(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        match c {
            ' ' => out.push_str("%20"),
            '(' => out.push_str("%28"),
            ')' => out.push_str("%29"),
            '#' => out.push_str("%23"),
            '%' => out.push_str("%25"),
            '<' => out.push_str("%3C"),
            '>' => out.push_str("%3E"),
            c => out.push(c),
        }
    }
    out
}

/// A character that continues the destination part being completed (a name, or a fragment).
pub(crate) fn destination_char(c: char) -> bool {
    !c.is_whitespace() && !matches!(c, '/' | '#' | '(' | ')' | '[' | ']' | '<' | '>' | '"' | '\'')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use std::path::PathBuf;

    fn snapshot(text: &str) -> TextSnapshot {
        TextSnapshot::from_document(crate::text::BufferId(1), &Document::decode(PathBuf::from("/n.md"), 1, text.as_bytes()))
    }

    /// `text` with `|` as the caret, after Enter.
    fn enter(text: &str) -> Option<String> {
        let caret = text.find('|').unwrap();
        let source = text.replace('|', "");
        let doc = snapshot(&source);
        let edits = list_enter(&doc, caret, "\n", true)?;
        let next = doc.apply_edits(2, &edits).unwrap();
        let caret = crate::text::edited_byte(caret, &edits[..1]);
        let mut out = next.copy_range(0..next.len_bytes()).unwrap();
        out.insert(caret, '|');
        Some(out)
    }

    #[test]
    fn enter_continues_bullets_numbers_and_tasks() {
        assert_eq!(enter("- one|").as_deref(), Some("- one\n- |"));
        assert_eq!(enter("* one|\n").as_deref(), Some("* one\n* |\n"));
        assert_eq!(enter("+   wide|").as_deref(), Some("+   wide\n+   |"));
        assert_eq!(enter("  - nested|").as_deref(), Some("  - nested\n  - |"));
        assert_eq!(enter("> - quoted|").as_deref(), Some("> - quoted\n> - |"));
        assert_eq!(enter("1. one|").as_deref(), Some("1. one\n2. |"));
        assert_eq!(enter("9) nine|").as_deref(), Some("9) nine\n10) |"));
        assert_eq!(enter("- [x] done|").as_deref(), Some("- [x] done\n- [ ] |"));
        assert_eq!(enter("3. [ ] open|").as_deref(), Some("3. [ ] open\n4. [ ] |"));
        // Text after the caret moves into the new item.
        assert_eq!(enter("- split|here").as_deref(), Some("- split\n- |here"));
        assert_eq!(enter("- café ✓|").as_deref(), Some("- café ✓\n- |"));
    }

    #[test]
    fn enter_on_an_empty_item_ends_the_list() {
        assert_eq!(enter("- one\n- |").as_deref(), Some("- one\n|"));
        assert_eq!(enter("- one\n  - |").as_deref(), Some("- one\n|"));
        assert_eq!(enter("1. one\n2. |\n").as_deref(), Some("1. one\n|\n"));
        assert_eq!(enter("- [ ] |").as_deref(), Some("|"));
        assert_eq!(enter("> - |").as_deref(), Some("> |"));
    }

    #[test]
    fn enter_renumbers_the_items_after_a_new_one_while_they_count_on() {
        assert_eq!(enter("1. a|\n2. b\n3. c\n").as_deref(), Some("1. a\n2. |\n3. b\n4. c\n"));
        // Nested items, continuation lines and blank lines inside the list are passed over; the list ends at
        // text that is not part of it.
        assert_eq!(enter("1. a|\n   - inner\n   more\n\n2. b\n\nAfter\n3. c\n").as_deref(), Some("1. a\n2. |\n   - inner\n   more\n\n3. b\n\nAfter\n3. c\n"));
        // A list that skips numbers is renumbered only up to the gap.
        assert_eq!(enter("1. a|\n2. b\n7. c\n").as_deref(), Some("1. a\n2. |\n3. b\n7. c\n"));
        // Another delimiter or a bullet is another list.
        assert_eq!(enter("1. a|\n2) b\n").as_deref(), Some("1. a\n2. |\n2) b\n"));
        // A list numbered `1.` throughout stays so.
        assert_eq!(enter("1. a|\n1. b\n").as_deref(), Some("1. a\n1. |\n1. b\n"));
        assert_eq!(enter("1. a\n1. b|").as_deref(), Some("1. a\n1. b\n1. |"));
        // Nested numbered lists renumber at their own depth only.
        assert_eq!(enter("1. a\n   1. x|\n   2. y\n2. b\n").as_deref(), Some("1. a\n   1. x\n   2. |\n   3. y\n2. b\n"));
    }

    #[test]
    fn enter_is_plain_outside_items_before_their_text_and_in_code() {
        assert_eq!(enter("plain|"), None);
        assert_eq!(enter("|- one"), None);
        assert_eq!(enter("-| one"), None);
        assert_eq!(enter("---|"), None);
        assert_eq!(enter("* * *|"), None);
        assert_eq!(enter("-not a list|"), None);
        assert_eq!(enter("```yaml\n- key: 1|\n```\n"), None);
        assert_eq!(enter("```\ncode\n```\n- after|").as_deref(), Some("```\ncode\n```\n- after\n- |"));
        assert_eq!(enter("1234567890. too long|"), None);
    }

    #[test]
    fn enter_keeps_crlf_newlines() {
        let doc = snapshot("- a\r\n");
        let edits = list_enter(&doc, 3, "\r\n", true).unwrap();
        assert_eq!(edits, [TextEdit { range: 3..3, text: "\r\n- ".into() }]);
    }

    fn heads(text: &str) -> Vec<(u8, String, String, usize)> {
        headings(text.lines().map(std::borrow::Cow::Borrowed), || true).unwrap().into_iter().map(|h| (h.level, h.text, h.anchor, h.line)).collect()
    }

    #[test]
    fn headings_have_github_anchors_outside_code_and_front_matter() {
        let text = "---\ntitle: x\n# not\n---\n# Intro\n\n## Set up `lenz` *now*\n\n```\n# code\n```\nSetext Title\n===\n\nSecond\n---\n### Intro\n#hashtag\n####### seven\n## Closed ##\n## C# and F#\n## Café Ünïcode\n## [Link](x.md) here\n";
        assert_eq!(heads(text), [
            (1, "Intro".into(), "intro".into(), 4),
            (2, "Set up `lenz` *now*".into(), "set-up-lenz-now".into(), 6),
            (1, "Setext Title".into(), "setext-title".into(), 11),
            (2, "Second".into(), "second".into(), 14),
            (3, "Intro".into(), "intro-1".into(), 16),
            (2, "Closed".into(), "closed".into(), 19),
            (2, "C# and F#".into(), "c-and-f".into(), 20),
            (2, "Café Ünïcode".into(), "café-ünïcode".into(), 21),
            (2, "[Link](x.md) here".into(), "link-here".into(), 22),
        ]);
        assert_eq!(headings(["# a".into()].into_iter(), || false), None);
    }

    #[test]
    fn the_destination_at_the_caret_is_found_only_in_a_link() {
        let d = |s| destination(s).map(|d| (d.start, d.path, d.fragment, d.word()));
        assert_eq!(d("see [docs]("), Some((11, "", None, 11)));
        assert_eq!(d("see [docs](guide/ins"), Some((11, "guide/ins", None, 17)));
        assert_eq!(d("![logo](img/"), Some((8, "img/", None, 12)));
        assert_eq!(d("[a](other.md#set"), Some((4, "other.md", Some("set"), 13)));
        assert_eq!(d("[a](#"), Some((4, "", Some(""), 5)));
        assert_eq!(d("[a](b c"), None);
        assert_eq!(d("[a](b) more"), None);
        assert_eq!(d("[a](https://x"), None);
        assert_eq!(d("[a](mailto:x"), None);
        assert_eq!(d("[a](//host/"), None);
        assert_eq!(d("`[a](x"), None);
        assert_eq!(d("plain text"), None);
    }

    #[test]
    fn link_paths_resolve_inside_the_root_and_escape_names() {
        assert_eq!(resolve("docs", "guide/").as_deref(), Some("docs/guide"));
        assert_eq!(resolve("docs/a", "../b/./c").as_deref(), Some("docs/b/c"));
        assert_eq!(resolve("docs", "/top").as_deref(), Some("top"));
        assert_eq!(resolve("", "my%20notes.md").as_deref(), Some("my notes.md"));
        assert_eq!(resolve("docs", "../.."), None);
        assert_eq!(resolve("", "bad%zz"), None);
        assert_eq!(encode("my notes (draft)#1 100%.md"), "my%20notes%20%28draft%29%231%20100%25.md");
        assert_eq!(decode(&encode("a b(c)#%<>é")).as_deref(), Some("a b(c)#%<>é"));
    }
}
