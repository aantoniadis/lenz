//! XML is data: a cancellable streaming validator never resolves external entities. Lexical spans remain
//! useful in incomplete drafts; folds require matching, case-sensitive names, never HTML's void-tag rules.
use std::{io::{self, Read}, ops::Range};
use xml::{common::Position, reader::{ParserConfig, XmlEvent}};
use super::{Capture, Fold, FoldKind, Folds, Parsed, Span, Syntax, SyntaxError, Want};

struct Input<'a> { bytes: &'a [u8], current: &'a dyn Fn() -> bool, read: usize }
impl Read for Input<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.read.is_multiple_of(4096) && !(self.current)() { return Err(io::Error::other("Obsolete XML draft")); }
        let n = out.len().min(self.bytes.len()).min(4096 - self.read % 4096);
        out[..n].copy_from_slice(&self.bytes[..n]);
        self.bytes = &self.bytes[n..];
        self.read += n;
        Ok(n)
    }
}

fn validate(text: &str, starts: &[usize], current: &dyn Fn() -> bool) -> Option<Vec<SyntaxError>> {
    if text.contains("<!ENTITY") {
        // xml-rs 0.8 bypasses its expansion ceilings for attributes and parameter entities.
        let bytes = text.as_bytes();
        let mut at = 0;
        let mut next_check = 0;
        let mut wanted = |at: usize| {
            if at < next_check { return true; }
            next_check = at.saturating_add(4096);
            current()
        };
        while at < bytes.len() {
            if !wanted(at) { return None; }
            let rest = text.get(at..)?;
            let markup = if rest.starts_with("<!--") { Some((4, "-->")) }
                else if rest.starts_with("<![CDATA[") { Some((9, "]]>")) }
                else if rest.starts_with("<?") { Some((2, "?>")) } else { None };
            if let Some((prefix, end)) = markup {
                at += prefix;
                while at < bytes.len() && !text.get(at..)?.starts_with(end) {
                    if !wanted(at) { return None; }
                    at += text.get(at..)?.chars().next()?.len_utf8();
                }
                at = (at + end.len()).min(bytes.len());
            } else if rest.starts_with("<!ENTITY") && bytes.get(at + 8).is_some_and(u8::is_ascii_whitespace) {
                return Some(vec![SyntaxError { range: at..at + 8, limited: true,
                    message: "XML validation limit: internal entity declarations are unsupported by the bounded validator".into() }]);
            } else { at += rest.chars().next()?.len_utf8(); }
        }
    }
    // The document is already decoded. Respect its original encoding on Save, not a second time here.
    let budget = text.len().max(65_536);
    let config = ParserConfig::new().override_encoding(Some(xml::Encoding::Utf8))
        .ignore_invalid_encoding_declarations(true).allow_multiple_root_elements(false).max_entity_expansion_length(65_536).max_entity_expansion_depth(8)
        // Entity references can repeatedly expand into one text or attribute buffer.
        .max_data_length(budget).max_attribute_length(budget);
    let input = Input { bytes: text.as_bytes(), current, read: 0 };
    let mut reader = config.create_reader(input);
    loop {
        if !current() { return None; }
        match reader.next() {
            Ok(XmlEvent::EndDocument) => return Some(Vec::new()),
            Ok(_) => {},
            Err(error) => {
                if !current() { return None; }
                let pos = error.position();
                let line = (pos.row as usize).min(starts.len().saturating_sub(1));
                let base = starts.get(line).copied().unwrap_or(text.len());
                let body = text.get(crate::lines::line_range(text, starts, line)).unwrap_or_default();
                let byte = base + body.char_indices().nth(pos.column as usize).map_or(body.len(), |(b, _)| b);
                let end = byte + text.get(byte..).and_then(|s| s.chars().next()).map_or(0, char::len_utf8);
                let message = error.msg();
                let limit = message == "Entity too big" || message == "This document is larger/more complex than allowed by the parser's configuration";
                let label = if limit { "XML validation limit" } else { "XML" };
                return Some(vec![SyntaxError { range: byte..end, message: format!("{label}: {message}"), limited: limit }]);
            }
        }
    }
}

pub(super) fn parse(text: &str, current: &dyn Fn() -> bool, want: Want) -> Option<Syntax> {
    let starts = crate::lines::line_starts(text);
    let errors = validate(text, &starts, current)?;
    let bytes = text.as_bytes();
    let mut spans: Vec<(Range<usize>, Capture)> = Vec::new();
    let mut folds = Vec::new();
    let mut stack: Vec<(Range<usize>, usize)> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if i.is_multiple_of(4096) && !current() { return None; }
        let start = i;
        let rest = &bytes[i..];
        if rest.starts_with(b"<!--") || rest.starts_with(b"<![CDATA[") || rest.starts_with(b"<?") {
            let (prefix, end, capture) = if rest.starts_with(b"<!--") { (4, b"-->".as_slice(), Capture::Comment) }
                else if rest.starts_with(b"<![CDATA[") { (9, b"]]>".as_slice(), Capture::String) }
                else { (2, b"?>".as_slice(), Capture::Keyword) };
            i += prefix;
            while i < bytes.len() && !bytes[i..].starts_with(end) {
                if i.is_multiple_of(4096) && !current() { return None; }
                i += 1;
            }
            i = (i + end.len()).min(bytes.len());
            spans.push((start..i, capture));
            add_fold(&starts, start, i, if capture == Capture::Comment { FoldKind::Comment } else { FoldKind::Block }, false, &mut folds);
        } else if bytes[i] == b'<' {
            let declaration = rest.starts_with(b"<!");
            let closing = rest.starts_with(b"</");
            i += if declaration || closing { 2 } else { 1 };
            let name = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !matches!(bytes[i], b'>' | b'/') {
                if i.is_multiple_of(4096) && !current() { return None; }
                i += 1;
            }
            let name = name..i;
            spans.push((start..name.start, Capture::Punctuation));
            spans.push((name.clone(), if declaration { Capture::Keyword } else { Capture::Tag }));
            let mut depth = 0usize;
            while i < bytes.len() {
                if i.is_multiple_of(4096) && !current() { return None; }
                let at = i;
                match bytes[i] {
                    b'\'' | b'"' => {
                        let quote = bytes[i];
                        i += 1;
                        while i < bytes.len() && bytes[i] != quote {
                            if i.is_multiple_of(4096) && !current() { return None; }
                            i += 1;
                        }
                        if i < bytes.len() { i += 1; }
                        spans.push((at..i, Capture::String));
                    }
                    b'[' if declaration => { depth += 1; i += 1; }
                    b']' if declaration => { depth = depth.saturating_sub(1); i += 1; }
                    b'>' if depth == 0 => { i += 1; spans.push((at..i, Capture::Punctuation)); break; }
                    b'=' | b'/' => { i += 1; spans.push((at..i, Capture::Punctuation)); }
                    b if b.is_ascii_whitespace() => i += 1,
                    _ => {
                        i += 1;
                        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !matches!(bytes[i], b'=' | b'/' | b'>' | b'\'' | b'"' | b'[' | b']') {
                            if i.is_multiple_of(4096) && !current() { return None; }
                            i += 1;
                        }
                        spans.push((at..i, if declaration { Capture::Keyword } else { Capture::Attribute }));
                    }
                }
            }
            if declaration { add_fold(&starts, start, i, FoldKind::Block, false, &mut folds); }
            else if closing {
                if let Some((_, from)) = stack.pop().filter(|(open, _)| bytes.get(open.clone()) == bytes.get(name.clone())) {
                    let line = starts.partition_point(|&s| s <= start).saturating_sub(1);
                    let closes = text.get(crate::lines::line_range(text, &starts, line)).is_some_and(|line| Some(line.trim()) == text.get(start..i));
                    add_fold(&starts, from, i, FoldKind::Block, closes, &mut folds);
                }
            } else if !bytes.get(start..i).is_some_and(|tag| tag.ends_with(b"/>")) && bytes.get(i.wrapping_sub(1)) == Some(&b'>') {
                stack.push((name, start));
            }
        } else if bytes[i] == b'&' {
            i += 1;
            while i < bytes.len() && !matches!(bytes[i], b';' | b'<' | b'&') && !bytes[i].is_ascii_whitespace() {
                if i.is_multiple_of(4096) && !current() { return None; }
                i += 1;
            }
            if bytes.get(i) == Some(&b';') { i += 1; spans.push((start..i, Capture::Escape)); }
        } else { i += 1; }
    }
    let mut parsed = Parsed { offsets: Vec::with_capacity(starts.len() + 1), spans: Vec::new() };
    let mut index = 0;
    for line in 0..starts.len() {
        if line.is_multiple_of(256) && !current() { return None; }
        let range = crate::lines::line_range(text, &starts, line);
        parsed.offsets.push(parsed.spans.len());
        while spans.get(index).is_some_and(|(r, _)| r.end <= range.start) { index += 1; }
        let mut at = index;
        while let Some((r, capture)) = spans.get(at).filter(|(r, _)| r.start < range.end) {
            if at.is_multiple_of(256) && !current() { return None; }
            let start = r.start.max(range.start) - range.start;
            let end = r.end.min(range.end) - range.start;
            if start < end { parsed.spans.push(Span { start: start as u32, end: end as u32, capture: *capture }); }
            at += 1;
        }
    }
    parsed.offsets.push(parsed.spans.len());
    folds.sort_unstable_by_key(|f| (f.first, std::cmp::Reverse(f.last)));
    folds.dedup_by_key(|f| f.first);
    Some(Syntax { parsed, folds: want.folds.then_some(Folds { regions: folds }), declarations: None, errors: Some(errors) })
}

fn add_fold(starts: &[usize], start: usize, end: usize, kind: FoldKind, closes: bool, folds: &mut Vec<Fold>) {
    let first = starts.partition_point(|&s| s <= start).saturating_sub(1) as u32;
    let last = starts.partition_point(|&s| s < end).saturating_sub(1) as u32;
    if last > first { folds.push(Fold { first, last, kind, closes }); }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn checked(text: &str) -> Syntax { parse(text, &|| true, Want { folds: true, declarations: false }).unwrap() }
    #[test]
    fn xml_well_formedness_covers_namespaces_special_markup_and_entities() {
        for text in [
            "<Root/>", "<root><BR></BR></root>",
            "<?xml version='1.0'?><ns:根 xmlns:ns='urn:example' ns:属性='é'>&amp;&#x1f600;</ns:根>",
            "<?target a='b'?><!DOCTYPE root SYSTEM 'https://invalid.example/no-fetch.dtd'><root><!-- hi --><![CDATA[<not-a-tag> & raw]]></root>",
            "<?xml version='1.0' encoding='UTF-16'?><root>é</root>",
        ] { assert!(checked(text).errors.unwrap().is_empty(), "{text}"); }
        for text in ["", "<Root></root>", "<root>", "<root><br></root>", "<r/><other/>", "<r a='1' a='2'/>", "<x:r/>", "<r>&unknown;</r>", "<r><!-- a--b --></r>", "<r>\u{0}</r>"] {
            let errors = checked(text).errors.unwrap();
            assert_eq!(errors.len(), 1, "{text}");
            assert!(errors[0].range.end <= text.len() && text.is_char_boundary(errors[0].range.start));
        }
    }
    #[test]
    fn xml_special_markup_is_lexical_and_only_multiline_regions_fold() {
        let text = "<?proc yes?>\n<!DOCTYPE root [\n<!ELEMENT root ANY>\n]>\n<root>\n<!-- comment\n<ignored/> -->\n<![CDATA[<fake>\nstill raw]]>\n<ns:tag a='é > x'/>\n</root>";
        let s = checked(text);
        let regions = &s.folds.unwrap().regions;
        assert!(regions.iter().any(|f| f.first == 4 && f.last == 10));
        assert!(regions.iter().any(|f| f.first == 5 && f.last == 6 && f.kind == FoldKind::Comment));
        assert_eq!(s.parsed.capture_at(5, 5), Some(Capture::Comment));
        assert_eq!(s.parsed.capture_at(7, 12), Some(Capture::String));
        assert_eq!(s.parsed.capture_at(9, 1), Some(Capture::Tag));
    }
    #[test]
    fn xml_fold_closing_line_with_text_is_not_only_a_closer() {
        let syntax = checked("<root>\ntext</root>");
        assert!(!syntax.folds.unwrap().regions[0].closes);
    }
    #[test]
    fn xml_spans_remain_sorted_and_on_unicode_boundaries_in_incomplete_drafts() {
        let pieces = ["<", ">", "/", "é", "根", "\n", "\r\n", "'", "\"", "<!--", "-->", "<![CDATA[", "]]>", "<?", "?>", "&", ";", "=", " ", "[", "]"];
        let mut seed = 42u64;
        for _ in 0..1000 {
            let mut text = String::new();
            for _ in 0..30 { seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1); text.push_str(pieces[(seed >> 32) as usize % pieces.len()]); }
            let syntax = checked(&text);
            let starts = crate::lines::line_starts(&text);
            for (line, spans) in syntax.parsed.spans(0..usize::MAX).into_iter().enumerate() {
                let line = &text[crate::lines::line_range(&text, &starts, line)];
                let mut last = 0;
                for (r, _) in spans {
                    assert!(r.start >= last && r.start < r.end && r.end <= line.len() && line.is_char_boundary(r.start) && line.is_char_boundary(r.end), "{text:?}: {r:?}");
                    last = r.end;
                }
            }
        }
    }
    #[test]
    fn xml_repeated_entities_cannot_grow_text_or_attributes_beyond_source_budget() {
        let entity = "x".repeat(4096);
        let references = "&chunk;".repeat(32);
        for body in [format!("<r>{references}</r>"), format!("<r value='{references}'/>")] {
            let text = format!("<!DOCTYPE r [<!ENTITY chunk '{entity}'>]>{body}");
            assert!(text.len() < 65_536);
            let errors = checked(&text).errors.unwrap();
            assert_eq!(errors.len(), 1, "entity expansion must stop within a source-sized buffer: {}", body.starts_with("<r value="));
            assert!(errors[0].message.contains("validation limit"), "{:?}", errors[0]);
            assert!(errors[0].limited);
        }
        let text = format!("<r>{}</r>", "x".repeat(70_000));
        assert!(checked(&text).errors.unwrap().is_empty(), "ordinary source text stays supported beyond the minimum buffer budget");
        let text = "<!DOCTYPE r [<!ENTITY % a 'safe'><!ENTITY % b '%a;%a;%a;'>]><r/>";
        assert!(checked(text).errors.unwrap()[0].limited, "parameter declarations never reach the unbounded expander");
        let text = "<!DOCTYPE r [<!ENTITY label 'safe'>]><r>&label;</r>";
        let parsed = checked(text);
        assert!(parsed.errors.unwrap()[0].limited);
        assert_eq!(parsed.parsed.capture_at(0, text.find("<r>").unwrap() + 1), Some(Capture::Tag), "a validation limit retains highlighting");
        assert!(checked("<r><!-- <!ENTITY example --> <![CDATA[<!ENTITY example]]> <?hint <!ENTITY example ?></r>").errors.unwrap().is_empty(), "literal markup in comments, CDATA and processing instructions is not a declaration");
    }
    #[test]
    fn xml_large_documents_cancel_inside_a_single_token() {
        use std::cell::Cell;
        for text in [format!("<r>{}</r>", "x".repeat(200_000)), format!("<!--{}--><r/>", "x".repeat(200_000)), format!("<{}>", "é".repeat(100_000))] {
            let calls = Cell::new(0);
            let current = || { calls.set(calls.get() + 1); calls.get() < 6 };
            assert!(parse(&text, &current, Want::default()).is_none());
        }
        let text = format!("<r>\n{}</r>", "  <item attr='é'/>\n".repeat(20_000));
        let s = checked(&text);
        assert!(s.errors.unwrap().is_empty());
        assert_eq!(s.parsed.spans(19_999..20_001).len(), 2);
    }
    #[test]
    #[ignore = "release component probe; run alone to retain timing and RSS samples"]
    fn xml_parse_release_probe() {
        use std::{sync::Arc, time::{Duration, Instant}};
        for count in [32, 20_000] {
            let text: Arc<str> = format!("<root>\n{}</root>", "  <ns:item xmlns:ns='urn:test' a='é'/>\n".repeat(count)).into();
            for sample in 0..6 {
                let started = Instant::now();
                let parsed = super::super::parse_with(super::super::Language::Xml, text.clone(), Want { folds: true, declarations: false }, || true).unwrap();
                let ms = started.elapsed().as_secs_f64() * 1000.0;
                assert!(parsed.errors.unwrap().is_empty());
                assert_eq!(parsed.parsed.offsets.len(), count + 3);
                assert_eq!(parsed.folds.unwrap().regions.len(), 1);
                eprintln!("xml_parse rows={count} sample={sample} bytes={} completed_ms={ms:.3} syntax_bytes={} rss_mib={:?}", text.len(), parsed.parsed.bytes(), crate::memory_probe::footprint_mib());
            }
        }
        let text: Arc<str> = format!("<root>\n{}</root>", "  <item a='é'/>\n".repeat(20_000)).into();
        let started = Instant::now();
        let mut rounds = 0;
        while started.elapsed() < Duration::from_secs(5) {
            let parsed = super::super::parse_with(super::super::Language::Xml, text.clone(), Want { folds: true, declarations: false }, || true).unwrap();
            assert!(parsed.errors.unwrap().is_empty());
            rounds += 1;
        }
        eprintln!("xml_stress rounds={rounds} actual_seconds={:.3} settled_rss_mib={:?}", started.elapsed().as_secs_f64(), crate::memory_probe::footprint_mib());
    }

}
