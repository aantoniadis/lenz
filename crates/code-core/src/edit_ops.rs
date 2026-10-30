//! Structural edits planned over one draft snapshot: indentation, comments, moving and duplicating lines, and
//! paired delimiters. Each planner returns the exact transaction (and, where the default anchor transform cannot
//! express it, the selections after it); the session applies it once, so every action is one undo step.
use std::ops::Range;

use lenz_content::highlight::Language;

use crate::session::Selection;
use crate::text::{TextEdit, TextSnapshot};

/// One indentation step, from the file's resolved preferences.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IndentUnit {
    /// What one step inserts: a tab, or that many spaces.
    pub(crate) text: String,
    /// What one step removes at most: a tab counts `tab_width` columns, spaces one each.
    pub(crate) width: usize,
    /// The style and size themselves, for text that carries its own indentation (a snippet's inserted lines).
    pub(crate) style: crate::preferences::Style,
    pub(crate) size: usize,
}

impl IndentUnit {
    pub(crate) fn new(style: crate::preferences::Style, size: u8, tab_width: u8) -> Self {
        let size = usize::from(size.max(1));
        match style {
            crate::preferences::Style::Tab => Self { text: "\t".into(), width: usize::from(tab_width.max(1)), style, size },
            crate::preferences::Style::Space => Self { text: " ".repeat(size), width: size, style, size },
        }
    }
}

/// The lines (0-based, end-exclusive) a selection touches. A selection the user ended on a line's first byte does
/// not take that line; a caret takes the line it rests on.
pub(crate) fn lines_of(doc: &TextSnapshot, selection: Selection) -> Range<usize> {
    let range = selection.range();
    let first = doc.line_of(range.start);
    let last = if range.end > range.start { doc.line_of(range.end - 1) } else { first };
    first..last + 1
}

/// The lines the selections touch, ascending, with touching or overlapping runs merged: each line is edited once.
pub(crate) fn touched_lines(doc: &TextSnapshot, selections: &[Selection]) -> Vec<Range<usize>> {
    let mut lines: Vec<Range<usize>> = selections.iter().map(|selection| lines_of(doc, *selection)).collect();
    lines.sort_by_key(|range| (range.start, range.end));
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(lines.len());
    for range in lines {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

/// One indentation step at every touched line; carets insert the step itself (`outdent` removes it from their
/// line instead). Edits at the same byte are made once, and the list is ascending and disjoint as a transaction
/// needs.
pub(crate) fn indent(doc: &TextSnapshot, selections: &[Selection], unit: &IndentUnit, outdent: bool) -> Vec<TextEdit> {
    let mut starts: Vec<usize> = Vec::new();
    if outdent {
        // Every selection, a caret included, unindents the line it touches.
        for line in touched_lines(doc, selections).into_iter().flatten() {
            starts.push(doc.reader_line_range(line).start);
        }
    } else {
        // A caret inserts at itself; only a selection indents the lines it touches.
        let selected: Vec<Selection> = selections.iter().copied().filter(|selection| !selection.range().is_empty()).collect();
        starts.extend(selections.iter().filter(|selection| selection.range().is_empty()).map(|selection| selection.head));
        for line in touched_lines(doc, &selected).into_iter().flatten() {
            starts.push(doc.reader_line_range(line).start);
        }
    }
    starts.sort_unstable();
    starts.dedup();
    starts.into_iter().filter_map(|start| {
        if !outdent {
            return Some(TextEdit { range: start..start, text: unit.text.as_str().into() });
        }
        let removed = removable(doc, start, unit);
        (removed > 0).then(|| TextEdit { range: start..start + removed, text: String::new().into() })
    }).collect()
}

/// How a language writes a comment: a prefix on each line, or one block pair around a span, or none at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Comments {
    Line(&'static str),
    Block(&'static str, &'static str),
    Xml,
    None,
}

/// The comment syntax of a language's source. It is the language's, never the file's text: a `.json` file has no
/// comments, and markup uses a block pair because a line prefix would be visible text.
pub(crate) fn comments(language: Option<Language>) -> Comments {
    match language {
        Some(Language::Rust | Language::C | Language::Cpp | Language::Go | Language::Zig | Language::Java | Language::Kotlin
            | Language::CSharp | Language::JavaScript | Language::TypeScript | Language::Tsx | Language::Php) => Comments::Line("//"),
        Some(Language::Python | Language::Ruby | Language::Bash | Language::Yaml | Language::Toml | Language::Dockerfile | Language::Makefile
            | Language::Env) => Comments::Line("#"),
        Some(Language::Lua | Language::Sql) => Comments::Line("--"),
        Some(Language::Css) => Comments::Block("/*", "*/"),
        Some(Language::Xml) => Comments::Xml,
        Some(Language::Html | Language::Markdown) => Comments::Block("<!--", "-->"),
        Some(Language::Json) | None => Comments::None,
    }
}

/// Comment or uncomment every line the selections touch (`None`: nothing to change). Lines are commented after
/// their own indentation; a blank line is left alone, and a partly commented run is completed rather than removed.
/// A block language wraps each selection's span once instead.
pub(crate) fn toggle_comments(doc: &TextSnapshot, selections: &[Selection], comments: Comments) -> Option<Vec<TextEdit>> {
    match comments {
        Comments::None => None,
        Comments::Line(token) => toggle_lines(doc, selections, token),
        Comments::Block(open, close) => toggle_blocks(doc, selections, open, close, false),
        Comments::Xml => toggle_blocks(doc, selections, "<!--", "-->", true),
    }
}

/// Whether the touched lines hold anything to comment: a blank or empty document has nothing.
pub(crate) fn has_text_to_comment(doc: &TextSnapshot, selections: &[Selection], comments: Comments) -> bool {
    if matches!(comments, Comments::None) { return false; }
    touched_lines(doc, selections).iter().any(|range| range.clone().any(|line| {
        doc.copy_range(doc.reader_line_range(line)).is_some_and(|text| !text.trim().is_empty())
    }))
}

fn toggle_lines(doc: &TextSnapshot, selections: &[Selection], token: &str) -> Option<Vec<TextEdit>> {
    let mut lines: Vec<usize> = touched_lines(doc, selections).into_iter().flatten().collect();
    lines.sort_unstable();
    lines.dedup();
    // Each non-blank line with where its content starts.
    let content: Vec<(usize, usize)> = lines.into_iter().filter_map(|line| {
        let range = doc.reader_line_range(line);
        let text = doc.copy_range(range.clone())?;
        let offset = text.len() - text.trim_start().len();
        (!text.trim().is_empty()).then_some((range.start, range.start + offset))
    }).collect();
    let commented = |at: usize| doc.copy_range(at..at.saturating_add(token.len())).is_some_and(|text| text == token);
    if content.is_empty() { return None; }
    let all = content.iter().all(|(_, at)| commented(*at));
    let mut edits = Vec::with_capacity(content.len());
    for (_, at) in content {
        if !all {
            if !commented(at) {
                edits.push(TextEdit { range: at..at, text: format!("{token} ").into() });
            }
            continue;
        }
        // One space belongs to the token we wrote; a hand-written `//text` keeps its own spacing.
        let space = doc.copy_range(at + token.len()..at + token.len() + 1).as_deref() == Some(" ");
        edits.push(TextEdit { range: at..at + token.len() + usize::from(space), text: String::new().into() });
    }
    (!edits.is_empty()).then_some(edits)
}

/// Each selection's span (a caret takes its line) is wrapped once, or unwrapped when every one of them is wrapped
/// already. Openers and closers keep one space from the text they hold.
fn toggle_blocks(doc: &TextSnapshot, selections: &[Selection], open: &str, close: &str, xml: bool) -> Option<Vec<TextEdit>> {
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for selection in selections {
        let lines = lines_of(doc, *selection);
        let first = doc.reader_line_range(lines.start);
        let last = doc.reader_line_range(lines.end.saturating_sub(1).max(lines.start));
        let whole = doc.copy_range(first.start..last.end).unwrap_or_default();
        let lead = whole.len() - whole.trim_start().len();
        if whole.trim().is_empty() { continue; }
        spans.push((first.start + lead, last.end - (whole.len() - whole.trim_end().len())));
    }
    spans.sort_unstable();
    spans.dedup();
    if spans.is_empty() { return None; }
    let wrapped = |(start, end): (usize, usize)| {
        let text = doc.copy_range(start..end).unwrap_or_default();
        text.starts_with(open) && text.ends_with(close) && text.len() >= open.len() + close.len()
    };
    let all = spans.iter().all(|span| wrapped(*span));
    let mut edits: Vec<TextEdit> = Vec::new();
    for (start, end) in spans {
        if all {
            // `<!-- text -->` loses the pair and the single space each kept from the text.
            let text = doc.copy_range(start..end).unwrap_or_default();
            let after_open = text.strip_prefix(open).unwrap_or_default();
            let inner_start = start + open.len() + usize::from(after_open.starts_with(' '));
            let before_close = end.saturating_sub(close.len());
            let inner_end = before_close.saturating_sub(usize::from(doc.copy_range(before_close.saturating_sub(1)..before_close).as_deref() == Some(" ")));
            edits.push(TextEdit { range: start..inner_start, text: String::new().into() });
            edits.push(TextEdit { range: inner_end..end, text: String::new().into() });
        } else if !wrapped((start, end)) {
            // XML forbids `--` anywhere inside a comment, including an existing nested comment.
            if xml && doc.copy_range(start..end).is_some_and(|text| text.contains("--")) { return None; }
            edits.push(TextEdit { range: start..start, text: format!("{open} ").into() });
            edits.push(TextEdit { range: end..end, text: format!(" {close}").into() });
        }
    }
    (!edits.is_empty()).then_some(edits)
}

/// What typing one delimiter does to the selections: nothing special, stepping over the closer already there, or a
/// transaction that keeps the plan's own selections.
pub(crate) enum Pairing {
    Plain,
    Step(usize),
    Plan(Vec<TextEdit>, Vec<Selection>),
}

/// The closing delimiter a typed opening one pairs with: `(`/`)`, `[`/`]`, `{`/`}` and each quote with itself. A
/// closing delimiter opens nothing: typing one either steps over the one already there or is plain text.
fn pair_of(typed: &str) -> Option<char> {
    Some(match typed {
        "(" => ')',
        "[" => ']',
        "{" => '}',
        "\"" | "'" | "`" => typed.chars().next()?,
        _ => return None,
    })
}

/// Enter after one complete XML opening tag starts a child line; markup, closing and self-closing tags
/// preserve their line's indentation. Quotes keep `>` in an attribute from ending a tag.
pub(crate) fn xml_line_indent(before: &str, step: &str) -> String {
    let body = before.trim_start_matches([' ', '\t']);
    let mut indent = before.get(..before.len() - body.len()).unwrap_or_default().to_owned();
    let tag = body.trim_end();
    if !tag.starts_with('<') || ["</", "<!", "<?"].iter().any(|prefix| tag.starts_with(prefix)) || !tag.ends_with('>') || tag.ends_with("/>") { return indent; }
    let mut quote = None;
    for (i, c) in tag.char_indices().skip(1) {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '\'' | '"') => quote = Some(c),
            (None, '<') => return indent,
            (None, '>') if i + 1 != tag.len() => return indent,
            _ => {},
        }
    }
    if quote.is_none() && tag.chars().nth(1).is_some_and(|c| c.is_alphabetic() || matches!(c, '_' | ':')) { indent.push_str(step); }
    indent
}

/// The indentation a new YAML line starts with when Enter is pressed after `before`, its line's text up to the
/// caret: the line's own indentation; under a sequence item's `- `, the item's content; and one `step` deeper after a
/// key that opens a block (`key:`, a trailing comment aside) or a block scalar (`key: |`, `>-`). Only that one line
/// is read, and `step` is asked for only when a block opens.
pub(crate) fn yaml_line_indent(before: &str, step: impl FnOnce() -> usize) -> String {
    let rest = before.trim_start_matches([' ', '\t']);
    let mut indent = before.strip_suffix(rest).unwrap_or_default().to_owned();
    let mut rest = rest;
    // `- `, `- - `: the item's content is where its next line continues.
    while let Some(after) = rest.strip_prefix('-').filter(|a| a.is_empty() || a.starts_with(' ')) {
        let content = after.trim_start_matches(' ');
        let pad = if content.is_empty() { 1 } else { after.len() - content.len() };
        indent.push_str(&" ".repeat(1 + pad));
        rest = content;
    }
    let content = yaml_without_comment(rest).trim_end();
    let token = content.rsplit(' ').next().unwrap_or_default();
    let block_scalar = token.starts_with(['|', '>']) && token.chars().skip(1).all(|c| matches!(c, '-' | '+' | '1'..='9'))
        && content.strip_suffix(token).is_some_and(|key| key.is_empty() || key.ends_with(": "));
    if content.ends_with(':') || block_scalar { indent.push_str(&" ".repeat(step())); }
    indent
}

/// A YAML line's text before its comment: a `#` at its start or after a space, outside quoted scalars.
fn yaml_without_comment(line: &str) -> &str {
    let mut quote = None;
    let mut previous = ' ';
    for (at, c) in line.char_indices() {
        match (quote, c) {
            (None, '#') if previous == ' ' || previous == '\t' => return line.get(..at).unwrap_or(line),
            // A quote opens a quoted scalar only where one can start (an apostrophe inside a word is text).
            (None, '"' | '\'') if matches!(previous, ' ' | '\t' | ':' | '-' | '[' | '{' | ',') => quote = Some(c),
            (Some(q), _) if c == q => quote = None,
            _ => {}
        }
        previous = c;
    }
    line
}

/// The unfinished opening tag on this line, with its quote state. Attribute punctuation is not markup.
fn xml_attribute_context(before: &str) -> Option<(&str, Option<char>)> {
    let mut start = None;
    let mut quote = None;
    for (at, c) in before.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {},
            (None, '<') => start = Some(at + 1),
            (None, '>') => start = None,
            (None, '\'' | '"') if start.is_some() => quote = Some(c),
            _ => {},
        }
    }
    let tag = before.get(start?..)?;
    (!tag.starts_with(['/', '!', '?'])).then_some((tag, quote))
}

/// Whether this language's source takes delimiter pairs at all. Prose and data files (Markdown, YAML, TOML, plain
/// text) do not: there a quote or a bracket is just a character the author typed.
fn pairs(language: Option<Language>) -> bool {
    !matches!(language, None | Some(Language::Markdown | Language::Yaml | Language::Toml))
}

/// What typing `typed` does at the selections of a draft of this language. An opening delimiter wraps a selection,
/// a caret gets its pair with the caret between when the text there has room for one, and the closer already in
/// front of every caret is stepped over rather than doubled.
pub(crate) fn typed(typed: &str, doc: &TextSnapshot, selections: &[Selection], language: Option<Language>) -> Pairing {
    if language == Some(Language::Env) { return typed_env(typed, doc, selections) }
    let xml = language == Some(Language::Xml);
    if xml && !matches!(typed, "\"" | "'") { return Pairing::Plain; }
    if typed.chars().count() != 1 || !pairs(language) || selections.is_empty() { return Pairing::Plain }
    let next_char = |at: usize| doc.copy_range(at..doc.next_char_boundary(at));
    let caret_only = selections.iter().all(|selection| selection.range().is_empty());
    // Only a closing delimiter or a quote steps over its twin: any other character (Enter before a line break, a
    // letter before the same letter) is typed.
    let closer = matches!(typed, ")" | "]" | "}" | "\"" | "'" | "`");
    let xml_context = |at: usize, opens: bool| doc.line_range(doc.line_of(at)).filter(|line| at - line.start <= 65_536)
        .and_then(|line| doc.copy_range(line.start..at)).is_some_and(|before| {
            xml_attribute_context(&before).is_some_and(|(tag, quote)| if opens { quote.is_none() && tag.trim_end().ends_with('=') } else { quote == typed.chars().next() })
        });
    if closer && caret_only && (!xml || selections.iter().all(|s| xml_context(s.head, false))) && selections.iter().all(|selection| next_char(selection.head).as_deref() == Some(typed)) {
        return Pairing::Step(typed.len());
    }
    let Some(close) = pair_of(typed) else { return Pairing::Plain };
    let Some(open) = typed.chars().next() else { return Pairing::Plain };
    let word_before = |at: usize| doc.copy_range(doc.previous_char_boundary(at)..at)
        .is_some_and(|text| text.chars().all(|c| c.is_alphanumeric() || c == '_'));
    let mut edits: Vec<TextEdit> = Vec::new();
    let mut after: Vec<Selection> = Vec::with_capacity(selections.len());
    let mut inserted = 0usize;
    for selection in selections {
        let range = selection.range();
        let next = next_char(range.start);
        // `foo(` pairs, `a'` does not: a quote after a word is an apostrophe, and a pair in front of a word or of
        // the same closer would only be in the way.
        // Quotes pair only at an XML attribute value; punctuation in element text is literal.
        let xml_value = xml && xml_context(range.start, true);
        let room = (xml_value && matches!(next.as_deref(), None | Some(">" | "/"))) || next.as_deref().is_none_or(|next| next.chars().all(char::is_whitespace)
            || matches!(next, ")" | "]" | "}" | ";" | "," | "." | ":" | "'" | "\""));
        let apostrophe = open == close && word_before(range.start);
        if range.is_empty() && ((xml && !xml_value) || apostrophe || next.as_deref() == Some(typed) || !room) {
            edits.push(TextEdit { range: range.start..range.start, text: typed.to_string().into() });
            after.push(Selection { anchor: range.start + inserted + typed.len(), head: range.start + inserted + typed.len() });
            inserted += typed.len();
            continue;
        }
        if range.is_empty() {
            edits.push(TextEdit { range: range.start..range.start, text: format!("{open}{close}").into() });
        } else {
            edits.push(TextEdit { range: range.start..range.start, text: open.to_string().into() });
            edits.push(TextEdit { range: range.end..range.end, text: close.to_string().into() });
        }
        // A wrapped selection keeps its text and the caret of a pair sits between the two delimiters.
        let anchor = range.start + inserted + open.len_utf8();
        let head = if range.is_empty() { anchor } else { range.end + inserted + open.len_utf8() };
        after.push(Selection { anchor, head });
        inserted += open.len_utf8() + close.len_utf8();
    }
    Pairing::Plan(edits, after)
}

/// What typing `typed` does in an environment file, where a value is data: only `"` and `'` ever pair, and only
/// where a value starts (a caret right after an entry's `=`, with nothing after it on its line); a selection is
/// wrapped in the quote typed; a quote steps over its twin only when that twin ends the line (the pair just
/// typed) and is not escaped. Brackets, braces and backticks are always typed as they are, so `${VAR}` and `$(…)`
/// stay exactly what was typed.
fn typed_env(typed: &str, doc: &TextSnapshot, selections: &[Selection]) -> Pairing {
    if !matches!(typed, "\"" | "'") || selections.is_empty() { return Pairing::Plain }
    let line_around = |at: usize| -> Option<(String, String)> {
        let line = doc.line_range(doc.line_of(at))?;
        Some((doc.copy_range(line.start..at.max(line.start))?, doc.copy_range(at.min(line.end)..line.end)?))
    };
    let caret_only = selections.iter().all(|selection| selection.range().is_empty());
    let closes_line = |selection: &Selection| line_around(selection.head).is_some_and(|(before, after)| {
        after.strip_prefix(typed).is_some_and(|rest| rest.trim().is_empty()) && !before.ends_with('\\')
    });
    if caret_only && selections.iter().all(closes_line) {
        return Pairing::Step(typed.len());
    }
    let mut edits: Vec<TextEdit> = Vec::new();
    let mut after: Vec<Selection> = Vec::with_capacity(selections.len());
    let mut inserted = 0usize;
    for selection in selections {
        let range = selection.range();
        let opens = range.is_empty() && line_around(range.start).is_some_and(|(before, after)| env_value_starts(&before) && after.trim().is_empty());
        if range.is_empty() && !opens {
            edits.push(TextEdit { range: range.start..range.start, text: typed.into() });
            after.push(Selection { anchor: range.start + inserted + typed.len(), head: range.start + inserted + typed.len() });
            inserted += typed.len();
            continue;
        }
        if range.is_empty() {
            edits.push(TextEdit { range: range.start..range.start, text: format!("{typed}{typed}").into() });
        } else {
            edits.push(TextEdit { range: range.start..range.start, text: typed.into() });
            edits.push(TextEdit { range: range.end..range.end, text: typed.into() });
        }
        let anchor = range.start + inserted + typed.len();
        let head = if range.is_empty() { anchor } else { range.end + inserted + typed.len() };
        after.push(Selection { anchor, head });
        inserted += 2 * typed.len();
    }
    Pairing::Plan(edits, after)
}

/// Whether an environment file's line `before` the caret ends where a value starts: blanks, an optional `export`,
/// a key, optional blanks, `=` and optional blanks (the entry rule of the environment-file lexer).
fn env_value_starts(before: &str) -> bool {
    let rest = before.trim_start_matches([' ', '\t']);
    let rest = match rest.strip_prefix("export") {
        Some(after) if after.starts_with([' ', '\t']) && after.trim_start_matches([' ', '\t']).starts_with(|c: char| !matches!(c, '=' | '#')) => after.trim_start_matches([' ', '\t']),
        _ => rest,
    };
    let key = rest.find([' ', '\t', '=', '#', '"', '\'']).unwrap_or(rest.len());
    if key == 0 { return false }
    let rest = rest.get(key..).unwrap_or_default().trim_start_matches([' ', '\t']);
    rest.strip_prefix('=').is_some_and(|value| value.trim_start_matches([' ', '\t']).is_empty())
}

/// Whether the block of lines can move down (up), over a line of its own: a line's own ending stays at its
/// position, so the file's final newline (or lack of one) never changes.
fn can_move(doc: &TextSnapshot, block: &Range<usize>, down: bool) -> bool {
    if down {
        // The empty line after a final newline is the file's end, not a line to move past.
        doc.line_with_ending(block.end).is_some_and(|range| !range.is_empty())
    } else {
        block.start > 0
    }
}

/// Whether any block the selections touch has a line to move over.
pub(crate) fn can_move_lines(doc: &TextSnapshot, selections: &[Selection], down: bool) -> bool {
    touched_lines(doc, selections).iter().any(|block| can_move(doc, block, down))
}

/// Move every block of touched lines one line down (up) as one transaction, with the selections following their
/// lines: each line keeps its own newline where it stands, so CRLF, LF and mixed endings, the final newline and a
/// missing one are preserved byte for byte.
pub(crate) fn move_lines(doc: &TextSnapshot, selections: &[Selection], down: bool) -> Option<(Vec<TextEdit>, Vec<Selection>)> {
    let blocks: Vec<Range<usize>> = touched_lines(doc, selections).into_iter().filter(|block| can_move(doc, block, down)).collect();
    if blocks.is_empty() { return None; }
    let mut edits = Vec::with_capacity(blocks.len());
    // For each swapped span: where it is, where each old line's text went (new offset in the span, old content start,
    // content length, content plus ending length), where the block's lines start in the new text, and the line the
    // selection boundary just after the block follows (its start, or the span's end).
    struct Swapped { span: Range<usize>, placed: Vec<(usize, usize, usize, usize)>, end: usize, after: usize }
    let mut spans: Vec<Swapped> = Vec::with_capacity(blocks.len());
    for block in blocks {
        let swap = if down { block.start..block.end + 1 } else { block.start - 1..block.end };
        let lines: Vec<(usize, usize, usize)> = swap.clone().map(|line| {
            let (content, ending) = line_parts(doc, line);
            (content.start, content.len(), ending.len())
        }).collect();
        // New line `t` holds the content of old line `order[t]`; the endings stay where they are, so each line of
        // the span keeps the terminator written at its position.
        let mut order: Vec<usize> = (0..lines.len()).collect();
        if down { order.rotate_right(1); } else { order.rotate_left(1); }
        let mut text = String::new();
        let mut placed: Vec<(usize, usize, usize, usize)> = vec![(0, 0, 0, 0); lines.len()];
        // Where each new line of the span starts, relative to the span.
        let mut starts: Vec<usize> = Vec::with_capacity(lines.len());
        for (t, &sourced) in order.iter().enumerate() {
            let (start, len, own_ending) = lines[sourced];
            starts.push(text.len());
            placed[sourced] = (text.len(), start, len, len + own_ending);
            text.push_str(&doc.copy_range(start..start + len).unwrap_or_default());
            // The terminator written at this position stays where it is: a CRLF line keeps CRLF, a last line without
            // a newline keeps none, and no ending is invented, converted or lost.
            let (here_start, here_len, here) = lines[t];
            text.push_str(&doc.copy_range(here_start + here_len..here_start + here_len + here).unwrap_or_default());
        }
        let span_start = lines.first().map_or(0, |(start, _, _)| *start);
        let span_end = lines.last().map_or(0, |(start, len, ending)| start + len + ending);
        edits.push(TextEdit { range: span_start..span_end, text: text.into() });
        // The block's lines are the last ones of the span when moving up, the first when moving down; the boundary
        // after them is the next new line, or the span's own end.
        let block_len = block.end - block.start;
        let first = if down { 1 } else { 0 };
        let after = starts.get(first + block_len).map_or(span_end, |offset| span_start + offset);
        spans.push(Swapped { span: span_start..span_end, placed, end: block.end, after });
    }
    let map = |byte: usize| {
        for swapped in &spans {
            if !swapped.span.contains(&byte) { continue; }
            for (offset, old_start, len, whole) in &swapped.placed {
                if (*old_start..*old_start + *whole).contains(&byte) {
                    return swapped.span.start + offset + (byte - old_start).min(*len);
                }
            }
            return swapped.span.start;
        }
        byte
    };
    let selections = selections.iter().map(|selection| {
        let mut head = map(selection.head);
        // A selection that ended at the first byte of the line after a moved block still ends after the block: a
        // line boundary follows the lines it borders rather than the line that moved past them.
        let line = doc.line_of(selection.head);
        let at_line_start = doc.reader_line_range(line).start == selection.head;
        if selection.head != selection.anchor && at_line_start
            && let Some(swapped) = spans.iter().find(|swapped| swapped.end == line)
        {
            head = swapped.after;
        }
        Selection { anchor: map(selection.anchor), head }
    }).collect();
    Some((edits, selections))
}

/// Copy every block of touched lines directly below (above) itself, as one transaction. A copy repeats the block's
/// own terminators; a block that ends the file without a newline takes the previous line's terminator as its
/// separator, so the file's final newline state (and every other ending) stays as it was.
pub(crate) fn duplicate_lines(doc: &TextSnapshot, selections: &[Selection], down: bool) -> Option<Vec<TextEdit>> {
    if doc.len_bytes() == 0 { return None; }
    let blocks = touched_lines(doc, selections);
    let mut edits = Vec::with_capacity(blocks.len());
    for block in blocks {
        let first = doc.line_with_ending(block.start).map_or(0, |range| range.start);
        let last = doc.line_with_ending(block.end.saturating_sub(1)).map_or(first, |range| range.end);
        let text = doc.copy_range(first..last).unwrap_or_default();
        if text.is_empty() { continue; }
        if text.ends_with('\n') {
            let at = if down { last } else { first };
            edits.push(TextEdit { range: at..at, text: text.as_str().into() });
            continue;
        }
        // The block ends the file without a newline: the copy needs a terminator to stand beside the original.
        let separator = (0..block.start).rev().find_map(|line| {
            let (content, ending) = line_parts(doc, line);
            doc.copy_range(ending.start..ending.end.max(content.end)).filter(|ending| !ending.is_empty())
        }).unwrap_or_else(|| "\n".into());
        let (at, copy) = if down { (last, format!("{separator}{text}")) } else { (first, format!("{text}{separator}")) };
        edits.push(TextEdit { range: at..at, text: copy.as_str().into() });
    }
    (!edits.is_empty()).then_some(edits)
}

/// A line's content (without its terminator) and its terminator (`""` for the last line of a file that has none).
fn line_parts(doc: &TextSnapshot, line: usize) -> (Range<usize>, Range<usize>) {
    let content = doc.line_range(line).unwrap_or(0..0);
    let end = doc.line_with_ending(line).map_or(content.end, |whole| whole.end.max(content.end));
    (content.clone(), content.end..end)
}

/// How many bytes one step removes from the line starting at `line_start`: one tab, or up to `width` spaces.
fn removable(doc: &TextSnapshot, line_start: usize, unit: &IndentUnit) -> usize {
    let end = doc.floor_char_boundary(line_start.saturating_add(unit.width).min(doc.len_bytes()));
    let Some(text) = doc.copy_range(line_start..end) else { return 0 };
    let mut bytes = 0;
    let mut columns = 0;
    for c in text.chars() {
        if columns >= unit.width { break; }
        match c {
            '\t' => return bytes + 1,
            ' ' => { bytes += 1; columns += 1; }
            _ => break,
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_new_yaml_line_follows_its_lines_structure() {
        let indent = |before: &str| super::yaml_line_indent(before, || 2);
        for (before, expected) in [
            ("", ""), ("a: 1", ""), ("a:", "  "), ("  a:   ", "    "), ("  a: # 𝄞 note: x", "    "), ("a: 'b # c':", "  "),
            ("a: \"x\" # it's:", ""), ("don't: x # y:", ""), ("- a", "  "), ("  - a: 1", "    "), ("- - a:", "      "),
            ("-", "  "), ("- ", "  "), ("-   x", "    "), ("-x:", "  "), ("key: |", "  "), ("key: >-", "  "), ("- |2+", "    "),
            ("key: a|b", ""), ("  # only a comment", "  "), ("\t", "\t"),
        ] {
            assert_eq!(indent(before), expected, "{before:?}");
        }
        assert_eq!(super::yaml_line_indent("a: 1", || panic!("asked without a block")), "");
    }

    #[test]
    fn an_environment_value_starts_right_after_an_entrys_equals_sign() {
        for (before, starts) in [
            ("KEY=", true), ("KEY =  ", true), ("export KEY =", true), ("  export KEY=", true), ("export\tKEY = ", true), ("é😀=", true), ("export=", true),
            ("KEY=a", false), ("=", false), ("KEY", false), ("# KEY=", false), ("KEY#x=", false), ("two words=", false),
            ("export =", true), ("KEY='", false), ("", false),
        ] {
            assert_eq!(super::env_value_starts(before), starts, "{before:?}");
        }
    }
}
