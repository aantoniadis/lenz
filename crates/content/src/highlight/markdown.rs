//! Parse inline constructs per block paragraph, avoiding a second full-document syntax tree.
use std::ops::Range;


use super::Capture;

pub(super) struct Inline {
    pub range: Range<usize>,
    pub capture: Capture,
    /// Prefix maximum end: a viewport can skip every range that finishes before it, including nested ranges.
    pub end_before: usize,
}

pub(super) fn inline(text: &str, line_starts: &[usize], blocks: &tree_sitter::Tree, current: &dyn Fn() -> bool) -> Option<Vec<Inline>> {
    let mut spans = Vec::new();
    let mut blocks = blocks.walk();
    loop {
        if !current() { return None; }
        let node = blocks.node();
        if matches!(node.kind(), "inline" | "pipe_table_cell") && text.get(node.byte_range())?.bytes().any(|b| matches!(b, b'*' | b'_' | b'`' | b'[' | b'<')) {
            let mut range = node.byte_range();
            let mut ranges = Vec::new();
            let mut children = node.walk();
            for child in node.named_children(&mut children) {
                let excluded = child.byte_range();
                if range.start < excluded.start { ranges.push(range.start..excluded.start); }
                range.start = excluded.end;
            }
            if range.start < range.end { ranges.push(range); }
            // Block-continuation prefixes are not inline text; retain an offset map to the original source.
            // Keep inline text from becoming a heading, list or fence when its block context is removed.
            let mut inline = String::from("x ");
            let mut pieces = Vec::new();
            for range in ranges {
                let start = inline.len();
                inline.push_str(text.get(range.clone())?);
                pieces.push((start..inline.len(), range.start));
            }
            // Reference brackets are source syntax even when their definition is in another paragraph.
            let mut reference = |_: pulldown_cmark::BrokenLink<'_>| Some(("".into(), "".into()));
            let parser = pulldown_cmark::Parser::new_with_broken_link_callback(&inline, pulldown_cmark::Options::empty(), Some(&mut reference));
            for (event, range) in parser.into_offset_iter() {
                if !current() { return None; }
                let capture = match event {
                    pulldown_cmark::Event::Start(pulldown_cmark::Tag::Emphasis) => Capture::Emphasis,
                    pulldown_cmark::Event::Start(pulldown_cmark::Tag::Strong) => Capture::Strong,
                    pulldown_cmark::Event::Start(pulldown_cmark::Tag::Link { .. } | pulldown_cmark::Tag::Image { .. }) => Capture::Link,
                    pulldown_cmark::Event::Code(_) => Capture::String,
                    _ => continue,
                };
                let first = pieces.partition_point(|(piece, _)| piece.end <= range.start);
                for (piece, offset) in pieces[first..].iter().take_while(|(piece, _)| piece.start < range.end) {
                    let start = offset + range.start.max(piece.start) - piece.start;
                    let end = offset + range.end.min(piece.end) - piece.start;
                    let line = line_starts.partition_point(|&s| s <= start).saturating_sub(1);
                    let mut boundary = (line / super::HIGHLIGHT_CHUNK + 1) * super::HIGHLIGHT_CHUNK;
                    let mut start = start;
                    while let Some(&split) = line_starts.get(boundary).filter(|&&split| split < end) {
                        if !current() { return None; }
                        spans.push(Inline { range: start..split, capture, end_before: 0 });
                        start = split;
                        boundary += super::HIGHLIGHT_CHUNK;
                    }
                    spans.push(Inline { range: start..end, capture, end_before: 0 });
                }
            }
        }
        if !advance(&mut blocks) { break; }
    }
    spans.sort_by_key(|span| (span.range.start, std::cmp::Reverse(span.range.end)));
    let mut end = 0;
    for span in &mut spans {
        end = end.max(span.range.end);
        span.end_before = end;
    }
    Some(spans)
}

fn advance(cursor: &mut tree_sitter::TreeCursor<'_>) -> bool {
    if cursor.goto_first_child() { return true; }
    loop {
        if cursor.goto_next_sibling() { return true; }
        if !cursor.goto_parent() { return false; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enclosing_emphasis_keeps_window_lookup_bounded_and_obsolete_work_stops() {
        let source = format!("*{}end*", "line with **strong** and `code`\n".repeat(2000));
        let starts = crate::lines::line_starts(&source);
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_md::LANGUAGE.into()).unwrap();
        let tree = parser.parse(&source, None).unwrap();
        let spans = inline(&source, &starts, &tree, &|| true).unwrap();
        let window = starts[1792]..starts[2000];
        let first = spans.partition_point(|span| span.end_before <= window.start);
        assert!(first > 3500, "earlier windows are skipped even inside document-wide emphasis");
        let visible: Vec<_> = spans[first..].iter().take_while(|s| s.range.start < window.end).collect();
        assert!(visible.len() < 500);
        assert!(visible.iter().any(|s| s.capture == Capture::Emphasis && s.range.start == window.start));
        assert!(inline(&source, &starts, &tree, &|| false).is_none());
    }
}
