//! XML formatting re-indents element-only content and changes nothing else. Tags (attributes and their own line breaks
//! included), text, comments, CDATA, processing instructions and the doctype stay source bytes. An element holding
//! text or CDATA (mixed or text-only content), or marked `xml:space="preserve"`, keeps everything between its tags as
//! written, so whitespace that may be data stays under the author's control. Malformed XML is refused; a document the
//! bounded validator cannot check (internal entity declarations) is left alone.
use super::format_work::ToolResult;
use crate::{document::Document, edit_ops::IndentUnit};
use std::{ops::Range, sync::atomic::{AtomicBool, Ordering}};

/// Why the text was not formatted.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    /// It is not well-formed: formatting it would hide where the problem is.
    Invalid(String),
    /// It cannot be checked or formatted here (too large, entity declarations, cancelled): a save goes ahead as it is.
    Unsupported(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self { Refusal::Invalid(why) | Refusal::Unsupported(why) => f.write_str(why) }
    }
}

pub(super) fn format(doc: &Document, unit: &IndentUnit, cancelled: &AtomicBool) -> ToolResult {
    let preferred = first_ending(doc.text()).unwrap_or("\n");
    match whitespace(doc.text(), &unit.text, preferred, cancelled) {
        Ok(text) => ToolResult::Formatted { tool: "Lenz XML formatter", edit: super::format_work::difference(doc.text(), text) },
        Err(Refusal::Invalid(why)) => ToolResult::Failed(format!("XML formatting: {why}")),
        Err(Refusal::Unsupported(why)) => ToolResult::Unavailable(format!("XML formatting: {why}")),
    }
}

/// The first line ending `text` uses.
pub(super) fn first_ending(text: &str) -> Option<&'static str> {
    let bytes = text.as_bytes();
    let at = bytes.iter().position(|b| matches!(b, b'\r' | b'\n'))?;
    Some(match (bytes[at], bytes.get(at + 1)) {
        (b'\r', Some(b'\n')) => "\r\n",
        (b'\r', _) => "\r",
        _ => "\n",
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// `<name …>`, and whether it asks for its whitespace to be preserved.
    Open { preserve: bool },
    Close,
    /// `<name …/>`.
    Empty,
    /// A comment, processing instruction or doctype: placed like an element, written as it is.
    Markup,
    Cdata,
    Text,
}

#[derive(Clone, Debug)]
struct Token { kind: Kind, range: Range<usize> }

/// The end of a construct starting at `from` and closed by `end` (past it); the text's end when unclosed.
fn past(text: &str, from: usize, end: &str) -> usize {
    text.get(from..).and_then(|rest| rest.find(end)).map_or(text.len(), |i| from + i + end.len())
}

/// The end of a tag or doctype starting at `from`: past its `>`, skipping quoted values (and a doctype's `[…]`).
fn tag_end(bytes: &[u8], from: usize, doctype: bool) -> usize {
    let (mut at, mut quote, mut subset) = (from, None, 0usize);
    while at < bytes.len() {
        let b = bytes[at];
        at += 1;
        match (quote, b) {
            (Some(q), _) if b == q => quote = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') => quote = Some(b),
            (None, b'[') if doctype => subset += 1,
            (None, b']') if doctype => subset = subset.saturating_sub(1),
            (None, b'>') if subset == 0 => return at,
            _ => {}
        }
    }
    bytes.len()
}

/// `text`'s bytes in `range`: every range here starts and ends at an ASCII delimiter or the text's end.
fn slice(text: &str, range: Range<usize>) -> &str { text.get(range).unwrap_or_default() }

fn tokens(text: &str, check: &dyn Fn() -> Result<(), Refusal>) -> Result<Vec<Token>, Refusal> {
    let bytes = text.as_bytes();
    let (mut out, mut at) = (Vec::new(), 0);
    while at < bytes.len() {
        if out.len().is_multiple_of(1024) { check()?; }
        let start = at;
        let rest = slice(text, at..text.len());
        let kind = if !rest.starts_with('<') {
            at = rest.find('<').map_or(text.len(), |i| at + i);
            Kind::Text
        } else if rest.starts_with("<!--") {
            at = past(text, at + 4, "-->");
            Kind::Markup
        } else if rest.starts_with("<![CDATA[") {
            at = past(text, at + 9, "]]>");
            Kind::Cdata
        } else if rest.starts_with("<?") {
            at = past(text, at + 2, "?>");
            Kind::Markup
        } else if rest.starts_with("<!") {
            at = tag_end(bytes, at + 2, true);
            Kind::Markup
        } else if rest.starts_with("</") {
            at = tag_end(bytes, at + 2, false);
            Kind::Close
        } else {
            at = tag_end(bytes, at + 1, false);
            let tag = slice(text, start..at);
            if tag.ends_with("/>") { Kind::Empty } else {
                let preserve = ["xml:space=\"preserve\"", "xml:space='preserve'"].iter().any(|p| tag.contains(p));
                Kind::Open { preserve }
            }
        };
        out.push(Token { kind, range: start..at });
    }
    Ok(out)
}

/// The formatted text, or why it was not formatted. `preferred` is the line ending used when `text` has none.
pub(super) fn whitespace(text: &str, indent: &str, preferred: &str, cancelled: &AtomicBool) -> Result<String, Refusal> {
    let check = || if cancelled.load(Ordering::Relaxed) { Err(Refusal::Unsupported("cancelled".into())) } else { Ok(()) };
    check()?;
    if text.len() > crate::document::LARGE_BYTES { return Err(Refusal::Unsupported("large file: not formatted".into())); }
    match lenz_content::highlight::check_xml(text, &|| !cancelled.load(Ordering::Relaxed)) {
        None => return Err(Refusal::Unsupported("cancelled".into())),
        Some(Err(problem)) if problem.limited => return Err(Refusal::Unsupported(format!("not formatted ({})", problem.message))),
        Some(Err(problem)) => return Err(Refusal::Invalid(format!("invalid XML: {}", problem.message.strip_prefix("XML: ").unwrap_or(&problem.message)))),
        Some(Ok(())) => {}
    }
    let tokens = tokens(text, &check)?;
    // For each opening tag: its closing token, and whether its content is element-only (re-indented).
    let mut close = vec![usize::MAX; tokens.len()];
    let mut structural = vec![false; tokens.len()];
    let mut textual = vec![false; tokens.len()];
    let mut stack: Vec<usize> = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        let parent = stack.last().copied();
        match token.kind {
            Kind::Open { .. } => { if let Some(p) = parent { structural[p] = true; } stack.push(i); }
            Kind::Close => {
                let Some(open) = stack.pop() else { return Err(Refusal::Invalid("invalid XML: unexpected closing tag".into())) };
                close[open] = i;
            }
            Kind::Empty | Kind::Markup => if let Some(p) = parent { structural[p] = true; },
            Kind::Cdata => if let Some(p) = parent { textual[p] = true; },
            Kind::Text if !slice(text, token.range.clone()).trim().is_empty() => if let Some(p) = parent { textual[p] = true; },
            Kind::Text => {}
        }
    }
    if !stack.is_empty() { return Err(Refusal::Invalid("invalid XML: unclosed element".into())); }
    let mut endings = {
        let bytes = text.as_bytes();
        bytes.iter().enumerate().filter_map(|(i, b)| match b {
            b'\r' if bytes.get(i + 1) != Some(&b'\n') => Some("\r"),
            b'\n' => Some(if i > 0 && bytes[i - 1] == b'\r' { "\r\n" } else { "\n" }),
            _ => None,
        }).collect::<Vec<_>>().into_iter()
    };
    let fallback = first_ending(text).unwrap_or(preferred);
    let limit = |out: &String, more: usize| -> Result<(), Refusal> {
        if out.len().saturating_add(more) > crate::document::LARGE_BYTES { Err(Refusal::Unsupported("formatted output would exceed the editor's large-file limit".into())) } else { Ok(()) }
    };
    let mut out = String::with_capacity(text.len() + text.len() / 4);
    let mut lines = 1usize;
    let mut place = |out: &mut String, depth: usize, slice: &str| -> Result<(), Refusal> {
        if !out.is_empty() {
            let ending = endings.next().unwrap_or(fallback);
            limit(out, ending.len() + depth.saturating_mul(indent.len()) + slice.len())?;
            if lines >= crate::document::LARGE_LINES { return Err(Refusal::Unsupported("formatted output would exceed the editor's large-file limit".into())); }
            out.push_str(ending);
            for _ in 0..depth { out.push_str(indent); }
            lines += 1;
        }
        out.push_str(slice);
        Ok(())
    };
    let (mut i, mut depth) = (0, 0usize);
    while i < tokens.len() {
        if i.is_multiple_of(1024) { check()?; }
        let token = &tokens[i];
        let source = slice(text, token.range.clone());
        match token.kind {
            Kind::Open { preserve } if preserve || textual[i] || !structural[i] => {
                // Written whole, from its opening to its closing tag.
                let end = tokens[close[i]].range.end;
                place(&mut out, depth, slice(text, token.range.start..end))?;
                i = close[i];
            }
            Kind::Open { .. } => { place(&mut out, depth, source)?; depth += 1; }
            Kind::Close => { depth = depth.saturating_sub(1); place(&mut out, depth, source)?; }
            Kind::Text => {}
            Kind::Empty | Kind::Markup | Kind::Cdata => place(&mut out, depth, source)?,
        }
        i += 1;
    }
    let final_ending = if text.ends_with("\r\n") { "\r\n" } else if text.ends_with('\n') { "\n" } else if text.ends_with('\r') { "\r" } else { "" };
    limit(&out, final_ending.len())?;
    out.push_str(final_ending);
    check()?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn formatted(text: &str) -> Result<String, Refusal> { whitespace(text, "  ", "\n", &AtomicBool::new(false)) }

    #[test]
    fn xml_format_indents_element_only_content_and_keeps_every_construct_as_written() {
        let source = "<?xml version=\"1.0\"?><!-- top --><root a=\"x > y\"><item id='1'/><item><name>Ada</name><note>  spaced  </note></item><code><![CDATA[<raw>]]></code></root>\n";
        let expected = "<?xml version=\"1.0\"?>\n<!-- top -->\n<root a=\"x > y\">\n  <item id='1'/>\n  <item>\n    <name>Ada</name>\n    <note>  spaced  </note>\n  </item>\n  <code><![CDATA[<raw>]]></code>\n</root>\n";
        assert_eq!(formatted(source).unwrap(), expected);
        assert_eq!(formatted(expected).unwrap(), expected, "formatting is idempotent");
    }

    #[test]
    fn xml_format_leaves_mixed_preserved_and_empty_content_exactly_as_written() {
        let mixed = "<p>Hello <b>bold</b>  world</p>";
        assert_eq!(formatted(mixed).unwrap(), mixed);
        let preserved = "<r><pre xml:space=\"preserve\"><a/>   <b/></pre><c/></r>";
        assert_eq!(formatted(preserved).unwrap(), "<r>\n  <pre xml:space=\"preserve\"><a/>   <b/></pre>\n  <c/>\n</r>");
        assert_eq!(formatted("<a>   </a>").unwrap(), "<a>   </a>", "whitespace-only content may be data");
        assert_eq!(formatted("<a\n   b=\"1\"><c/></a>").unwrap(), "<a\n   b=\"1\">\n  <c/>\n</a>", "a tag's own line breaks stay");
    }

    #[test]
    fn xml_format_keeps_crlf_and_the_final_newline_choice() {
        assert_eq!(formatted("<a>\r\n<b/><c/></a>\r\n").unwrap(), "<a>\r\n  <b/>\r\n  <c/>\r\n</a>\r\n");
        assert_eq!(whitespace("<a><b/></a>", "\t", "\r\n", &AtomicBool::new(false)).unwrap(), "<a>\r\n\t<b/>\r\n</a>");
    }

    #[test]
    fn xml_format_refuses_malformed_entities_and_cancelled_work() {
        for bad in ["<a><b></a>", "<a>", "text", "<a/><b/>", ""] {
            assert!(matches!(formatted(bad), Err(Refusal::Invalid(_))), "{bad:?}");
        }
        assert!(matches!(formatted("<!DOCTYPE a [<!ENTITY e \"x\">]><a>&e;</a>"), Err(Refusal::Unsupported(_))));
        assert!(matches!(whitespace("<a/>", "  ", "\n", &AtomicBool::new(true)), Err(Refusal::Unsupported(_))));
    }
}
