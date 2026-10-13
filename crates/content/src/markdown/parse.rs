//! Event parsing only: geometry, comparison projection and workers live elsewhere.
use pulldown_cmark::{Alignment, CodeBlockKind, Event, LinkType, Options, Parser, Tag, TagEnd};
use std::collections::HashMap;

use super::*;

struct Item {
    source: Range<usize>,
    marker: String,
    checked: Option<bool>,
    emitted: bool,
}
struct Image {
    source: Range<usize>,
    destination: Arc<str>,
    alt: String,
}
type Target = (Arc<str>, Arc<str>);

fn contains(ranges: &[Range<usize>], byte: usize) -> bool {
    let index = ranges.partition_point(|range| range.start <= byte);
    index > 0 && byte < ranges[index - 1].end
}

fn whitespace_end(line: &str, start: usize) -> usize {
    start
        + line.as_bytes()[start..]
            .iter()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count()
}

fn columns(text: &str) -> usize {
    text.bytes().fold(0, |column, byte| {
        if byte == b'\t' {
            (column / 4 + 1) * 4
        } else {
            column + 1
        }
    })
}

fn quote_prefix(line: &str) -> (usize, usize) {
    let mut start = 0;
    let mut depth = 0;
    loop {
        let next = whitespace_end(line, start);
        if columns(&line[start..next]) > 3 || line.as_bytes().get(next) != Some(&b'>') {
            break;
        }
        start = next + 1;
        if matches!(line.as_bytes().get(start), Some(b' ' | b'\t')) {
            start += 1;
        }
        depth += 1;
    }
    (start, depth)
}

/// Recognize disabled footnotes independently of CommonMark link definitions.
/// Keep the first marker and all indented continuation bytes in one raw span.
fn footnote_spans(
    source: &str,
    current: &dyn Fn() -> bool,
) -> Result<Vec<Range<usize>>, MarkdownError> {
    let mut lines = source
        .split_inclusive('\n')
        .scan(0, |start, line| {
            let range = *start..*start + line.len();
            *start = range.end;
            Some((range, line))
        })
        .peekable();
    let mut spans = Vec::new();
    while let Some((line_range, line)) = lines.next() {
        if !current() {
            return Err(MarkdownError::Cancelled);
        }
        let (mut quote_end, mut quote_depth) = quote_prefix(line);
        let mut marker = whitespace_end(line, quote_end);
        loop {
            // Quotes can follow list markers as well as precede them. The
            // final quote prefix determines the continuation's inner indent.
            let (next_quote_end, next_depth) = quote_prefix(&line[marker..]);
            if next_depth > 0 {
                quote_end = marker + next_quote_end;
                quote_depth += next_depth;
                marker = whitespace_end(line, quote_end);
            }
            let tail = &line.as_bytes()[marker..];
            let digits = tail.iter().take_while(|byte| byte.is_ascii_digit()).count();
            let list_end = if matches!(tail.first(), Some(b'-' | b'+' | b'*')) {
                Some(marker + 1)
            } else if (1..=9).contains(&digits) && matches!(tail.get(digits), Some(b'.' | b')')) {
                Some(marker + digits + 1)
            } else {
                None
            };
            if let Some(end) =
                list_end.filter(|end| matches!(line.as_bytes().get(*end), Some(b' ' | b'\t')))
            {
                marker = whitespace_end(line, end);
            } else {
                break;
            }
        }
        if !line[marker..].starts_with("[^")
            || !line[marker + 2..].find("]:").is_some_and(|end| end > 0)
        {
            continue;
        }
        let required_indent = columns(&line[quote_end..marker]) + 4;
        let mut end = line_range.end;
        while let Some((next_range, next)) = lines.peek() {
            if !current() {
                return Err(MarkdownError::Cancelled);
            }
            let (next_quote_end, next_depth) = quote_prefix(next);
            if next_depth != quote_depth {
                break;
            }
            if next[next_quote_end..].trim().is_empty() {
                lines.next();
                continue;
            }
            let indent_end = whitespace_end(next, next_quote_end);
            if columns(&next[next_quote_end..indent_end]) < required_indent {
                break;
            }
            end = next_range.end;
            lines.next();
        }
        spans.push(line_range.start + marker..end);
    }
    Ok(spans)
}

struct Builder<'a> {
    doc: MarkdownDocument,
    remaining: &'a mut usize,
    leaf: Option<(BlockKind, Range<usize>)>,
    block: Option<Block>,
    quote_depth: u16,
    lists: Vec<Option<u64>>,
    items: Vec<Item>,
    style: InlineStyle,
    styles: Vec<InlineStyle>,
    alignment: Vec<TableAlignment>,
    cell_start: Option<usize>,
    image: Option<Image>,
    image_depth: usize,
    comment_start: Option<usize>,
    skip: usize,
    rendered_limit: usize,
    interned_bytes: usize,
    targets: HashMap<Arc<str>, Arc<str>>,
    references: HashMap<String, Target>,
    links: Vec<Option<Link>>,
    mapped_end: usize,
    reference_remaining: usize,
    /// Open callouts with the quote depth that opened them.
    callouts: Vec<(u16, Callout)>,
    next_callout: u32,
    /// The pending callout title line: its marker ends at `.0`, its title at `.1` (source offsets).
    title: Option<(usize, usize)>,
    /// Footnote numbers by lowercase label, in order of first use.
    footnotes: HashMap<String, usize>,
    /// Labels of the footnote definitions, lowercase.
    footnote_labels: std::collections::HashSet<String>,
    /// Source bytes before this offset were already rendered (a footnote reference split across text events).
    hidden_until: usize,
}

/// Parse one document within `limits`, spending at most `remaining` source segments (a budget its caller may
/// share across documents); `Cancelled` once `current` says it is superseded.
pub fn parse(
    source: Arc<str>,
    limits: Limits,
    remaining: &mut usize,
    current: &dyn Fn() -> bool,
) -> Result<MarkdownDocument, MarkdownError> {
    if !current() {
        return Err(MarkdownError::Cancelled);
    }
    if source.len() > limits.bytes_per_side {
        return Err(MarkdownError::Limit("source bytes"));
    }
    let options =
        Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_MATH
        | Options::ENABLE_WIKILINKS;
    // Pulldown also accepts `---` blocks later in a file, where they are usually a rule and a setext heading.
    let options = if source.starts_with("---\n") || source.starts_with("---\r\n") {
        options | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
    } else {
        options
    };
    let parser = Parser::new_ext(source.as_ref(), options);
    let footnotes = footnote_spans(&source, current)?;
    let footnote_labels = footnotes
        .iter()
        .filter_map(|r| {
            let text = &source[r.clone()];
            text.find("]:").map(|end| text[2..end].to_lowercase())
        })
        .collect();
    // Reference definitions are intentionally absent from the event stream.
    // Footnote spans are recognized from raw source instead, since ordinary
    // multiword definitions/continuations are not CommonMark link definitions.
    let mut definitions: Vec<_> = parser
        .reference_definitions()
        .iter()
        .filter(|(label, definition)| {
            !label.starts_with('^') && !contains(&footnotes, definition.span.start)
        })
        .map(|(_, definition)| (definition.span.clone(), false))
        .collect();
    definitions.extend(footnotes.into_iter().map(|range| (range, true)));
    definitions.sort_unstable_by_key(|(range, _)| range.start);
    let mut definitions = definitions.into_iter().peekable();
    let line_starts = crate::lines::line_starts(&source);
    let mut builder = Builder {
        doc: MarkdownDocument {
            source: source.clone(),
            text: String::new(),
            blocks: Vec::new(),
            segments: Vec::new(),
            line_starts,
            links: Vec::new(),
        },
        remaining,
        leaf: None,
        block: None,
        quote_depth: 0,
        lists: Vec::new(),
        items: Vec::new(),
        style: InlineStyle::default(),
        styles: Vec::new(),
        alignment: Vec::new(),
        cell_start: None,
        image: None,
        callouts: Vec::new(),
        next_callout: 0,
        title: None,
        footnotes: HashMap::new(),
        footnote_labels,
        hidden_until: 0,
        image_depth: 0,
        comment_start: None,
        skip: 0,
        rendered_limit: limits.rendered_bytes_per_side,
        interned_bytes: 0,
        targets: HashMap::new(),
        references: HashMap::new(),
        links: Vec::new(),
        mapped_end: 0,
        reference_remaining: source.len().max(100_000),
    };
    let mut cursor = 0;
    let mut literal_spans = Vec::new();
    for (ordinal, (event, source_range)) in parser.into_offset_iter().enumerate() {
        if !current() {
            return Err(MarkdownError::Cancelled);
        }
        if ordinal >= limits.events_per_side {
            return Err(MarkdownError::Limit("parser events"));
        }
        builder.validate(&source_range)?;
        let target = if matches!(event, Event::End(_)) {
            source_range.end
        } else {
            source_range.start
        };
        while definitions.peek().is_some_and(|(r, _)| r.start <= target) {
            let (range, footnote) = definitions.next().unwrap();
            if footnote
                && (matches!(&event, Event::Start(Tag::CodeBlock(_) | Tag::HtmlBlock))
                    || matches!(
                        builder.leaf.as_ref().map(|leaf| &leaf.0),
                        Some(BlockKind::Code { .. } | BlockKind::Literal | BlockKind::Properties)
                    ))
            {
                continue;
            }
            if range.start > cursor {
                builder.gap(cursor..range.start)?;
            }
            cursor = cursor.max(range.end);
            if footnote {
                literal_spans.push(range.clone());
            }
            builder.definition(range, footnote)?;
        }
        if target > cursor {
            builder.gap(cursor..target)?;
            cursor = target;
        }
        // Container ranges encompass their children; consume actual bytes only
        // at leaf events or closing boundaries, so skipped definitions in a
        // list/quote are processed while their context is still active.
        if !matches!(event, Event::Start(_)) {
            cursor = cursor.max(source_range.end);
        }
        if contains(&literal_spans, source_range.start) {
            // Maintain the dependency's expansion accounting even for syntax
            // inside a literal footnote, but do not render those events twice.
            if let Event::Start(
                Tag::Link {
                    link_type,
                    dest_url,
                    title,
                    ..
                }
                | Tag::Image {
                    link_type,
                    dest_url,
                    title,
                    ..
                },
            ) = &event
            {
                builder.reference(*link_type, dest_url, title)?;
            }
        } else {
            builder.event(event, source_range)?;
        }
    }
    for (range, footnote) in definitions {
        if !current() {
            return Err(MarkdownError::Cancelled);
        }
        if range.start > cursor {
            builder.gap(cursor..range.start)?;
        }
        cursor = cursor.max(range.end);
        builder.definition(range, footnote)?;
    }
    if cursor < source.len() {
        builder.gap(cursor..source.len())?;
    }
    if let Some(start) = builder.comment_start.take() {
        builder.hidden(start..source.len())?;
    }
    builder.finish();
    Ok(builder.doc)
}

impl Builder<'_> {
    fn check_output(&self, extra: usize) -> Result<(), MarkdownError> {
        if self
            .doc
            .text
            .len()
            .checked_add(extra)
            .is_none_or(|n| n > self.rendered_limit)
        {
            return Err(MarkdownError::Limit("rendered text bytes"));
        }
        Ok(())
    }

    fn reference(
        &mut self,
        kind: LinkType,
        destination: &str,
        title: &str,
    ) -> Result<(), MarkdownError> {
        match kind {
            LinkType::Reference | LinkType::Collapsed | LinkType::Shortcut => {
                let bytes = destination
                    .len()
                    .checked_add(title.len())
                    .ok_or(MarkdownError::Limit("reference expansion"))?;
                // The pinned parser privately subtracts these same bytes from
                // max(source.len(), 100_000), then silently stops resolving
                // references at zero. Equality is conservatively rejected too:
                // no later reference can become a partial literal comparison.
                self.reference_remaining = self
                    .reference_remaining
                    .checked_sub(bytes)
                    .filter(|n| *n > 0)
                    .ok_or(MarkdownError::Limit("reference expansion"))?;
            }
            LinkType::Inline
            | LinkType::ReferenceUnknown
            | LinkType::CollapsedUnknown
            | LinkType::ShortcutUnknown
            | LinkType::Autolink
            | LinkType::Email
            | LinkType::WikiLink { .. } => {}
        }
        Ok(())
    }

    fn intern(&mut self, text: &str) -> Result<Arc<str>, MarkdownError> {
        if let Some(shared) = self.targets.get(text) {
            return Ok(shared.clone());
        }
        let bytes = self
            .interned_bytes
            .checked_add(text.len())
            .ok_or(MarkdownError::Limit("link metadata bytes"))?;
        if bytes > self.rendered_limit {
            return Err(MarkdownError::Limit("link metadata bytes"));
        }
        let shared: Arc<str> = text.into();
        self.targets.insert(shared.clone(), shared.clone());
        self.interned_bytes = bytes;
        Ok(shared)
    }

    fn target(
        &mut self,
        id: &str,
        destination: &str,
        title: &str,
    ) -> Result<Target, MarkdownError> {
        // Cache by reference identity first: a long resolved URL is hashed only
        // once, rather than again for every short reference label occurrence.
        if !id.is_empty()
            && let Some(target) = self.references.get(id)
        {
            return Ok(target.clone());
        }
        let target = (self.intern(destination)?, self.intern(title)?);
        if !id.is_empty() {
            if id.len() > self.rendered_limit {
                return Err(MarkdownError::Limit("link metadata bytes"));
            }
            self.references.insert(id.to_string(), target.clone());
        }
        Ok(target)
    }

    fn validate(&self, range: &Range<usize>) -> Result<(), MarkdownError> {
        self.doc
            .source
            .get(range.clone())
            .ok_or(MarkdownError::InvalidMap)
            .map(|_| ())
    }

    fn append(
        &mut self,
        text: &str,
        origin: Origin,
        style: InlineStyle,
    ) -> Result<(), MarkdownError> {
        self.check_output(text.len())?;
        match &origin {
            Origin::Direct(r) | Origin::Normalized(r) => {
                self.validate(r)?;
                if !text.is_empty() {
                    self.mapped_end = self.mapped_end.max(r.end);
                }
            }
            Origin::Synthetic => {}
        }
        *self.remaining = self
            .remaining
            .checked_sub(1)
            .ok_or(MarkdownError::Limit("mapped text segments"))?;
        let start = self.doc.text.len();
        self.doc.text.push_str(text);
        let range = start..self.doc.text.len();
        self.doc.segments.push(MapSegment {
            text: range.clone(),
            origin,
        });
        if !range.is_empty()
            && let Some(block) = self.block.as_mut()
        {
            block.text.end = range.end;
            if let Some(last) = block
                .runs
                .last_mut()
                .filter(|r| r.style == style && r.text.end == range.start)
            {
                last.text.end = range.end;
            } else {
                block.runs.push(Run { text: range, style });
            }
        }
        Ok(())
    }

    fn hidden(&mut self, range: Range<usize>) -> Result<(), MarkdownError> {
        self.append("", Origin::Normalized(range), self.style)
    }

    fn mapped(
        &mut self,
        text: &str,
        range: Range<usize>,
        style: InlineStyle,
    ) -> Result<(), MarkdownError> {
        // Pulldown can omit an escape's backslash from a borrowed Text range.
        // Map that character to both original bytes rather than fabricating a
        // direct one-byte origin that loses the spelling of the escape.
        if self.doc.source.get(range.clone()) == Some(text)
            && range.start > self.mapped_end
            && self.doc.source.as_bytes().get(range.start - 1) == Some(&b'\\')
            && text
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_punctuation)
        {
            self.append(
                &text[..1],
                Origin::Normalized(range.start - 1..range.start + 1),
                style,
            )?;
            if text.len() > 1 {
                self.append(
                    &text[1..],
                    Origin::Direct(range.start + 1..range.end),
                    style,
                )?;
            }
            return Ok(());
        }
        let origin = if self.doc.source.get(range.clone()) == Some(text) {
            Origin::Direct(range)
        } else {
            Origin::Normalized(range)
        };
        self.append(text, origin, style)
    }

    fn open(&mut self, kind: BlockKind, source: Range<usize>) -> Result<(), MarkdownError> {
        if !matches!(kind, BlockKind::Paragraph | BlockKind::ListItem { .. })
            && let Some(item) = self.items.last_mut().filter(|i| !i.emitted)
        {
            item.emitted = true;
            let marker = item.marker.clone();
            let checked = item.checked;
            let range = item.source.clone();
            self.check_output(marker.len() + if checked.is_some() { 5 } else { 1 })?;
            let prefix = match checked {
                Some(true) => format!("{marker} [x] "),
                Some(false) => format!("{marker} [ ] "),
                None => format!("{marker} "),
            };
            self.open(BlockKind::ListItem { marker, checked }, range)?;
            self.append(&prefix, Origin::Synthetic, InlineStyle::default())?;
        }
        self.finish();
        if !self.doc.blocks.is_empty() {
            self.append("\n", Origin::Synthetic, InlineStyle::default())?;
        }
        let start = self.doc.text.len();
        self.block = Some(Block {
            kind,
            source,
            text: start..start,
            runs: Vec::new(),
            quote_depth: self.quote_depth,
            callout: self.callouts.last().map(|(_, c)| *c),
            list_depth: u16::try_from(self.lists.len())
                .map_err(|_| MarkdownError::Limit("list depth"))?,
        });
        Ok(())
    }

    fn ensure(&mut self, source: Range<usize>) -> Result<(), MarkdownError> {
        if self.block.is_some() {
            return Ok(());
        }
        let (mut kind, mut range) = self.leaf.clone().unwrap_or((BlockKind::Paragraph, source));
        let mut prefix = None;
        if matches!(kind, BlockKind::Paragraph)
            && let Some(item) = self.items.last_mut().filter(|i| !i.emitted)
        {
            kind = BlockKind::ListItem {
                marker: item.marker.clone(),
                checked: item.checked,
            };
            range = item.source.clone();
            prefix = Some(match item.checked {
                Some(true) => format!("{} [x] ", item.marker),
                Some(false) => format!("{} [ ] ", item.marker),
                None => format!("{} ", item.marker),
            });
            item.emitted = true;
        }
        self.open(kind, range)?;
        if let Some(prefix) = prefix {
            self.append(&prefix, Origin::Synthetic, InlineStyle::default())?;
        }
        Ok(())
    }

    fn finish(&mut self) {
        if let Some(block) = self.block.take()
            && !(matches!(block.kind, BlockKind::Paragraph) && block.text.is_empty()) {
            self.doc.blocks.push(block);
        }
    }

    fn definition(&mut self, range: Range<usize>, footnote: bool) -> Result<(), MarkdownError> {
        self.validate(&range)?;
        if footnote {
            self.check_output(range.len())?;
            let text = self.doc.source[range.clone()].to_string();
            // `[^label]:` and the spaces after it are hidden; footnote_spans guarantees the marker.
            let close = text.find("]:").ok_or(MarkdownError::InvalidMap)?;
            let body = close + 2 + (text[close + 2..].len() - text[close + 2..].trim_start_matches([' ', '\t']).len());
            let label = self.footnote(&text[2..close]).to_string();
            self.open(BlockKind::Footnote { label }, range.clone())?;
            self.append("", Origin::Normalized(range.start..range.start + body), InlineStyle::default())?;
            if body < text.len() {
                self.mapped(&text[body..], range.start + body..range.end, InlineStyle::default())?;
            }
            self.finish();
        } else {
            self.hidden(range)?;
        }
        Ok(())
    }

    /// The number of footnote `label`, assigning the next one on first use.
    fn footnote(&mut self, label: &str) -> usize {
        let next = self.footnotes.len() + 1;
        *self.footnotes.entry(label.to_lowercase()).or_insert(next)
    }

    fn gap(&mut self, range: Range<usize>) -> Result<(), MarkdownError> {
        self.validate(&range)?;
        // Duplicate reference definitions are omitted from both events and the
        // definition map. Preserve their source bytes; raw footnote spans were
        // already consumed by the independent source scanner.
        let mut start = range.start;
        while start < range.end {
            let end = self.doc.source[start..range.end]
                .find('\n')
                .map_or(range.end, |n| start + n + 1);
            let line = self.doc.source[start..end].trim_start();
            if !line.trim().is_empty() {
                self.hidden(start..end)?;
            }
            start = end;
        }
        Ok(())
    }

    fn text(&mut self, text: &str, range: Range<usize>, code: bool) -> Result<(), MarkdownError> {
        if let Some(image) = &self.image {
            let bytes = image
                .alt
                .len()
                .checked_add(text.len())
                .and_then(|n| n.checked_add(image.destination.len()))
                .and_then(|n| n.checked_add(12))
                .ok_or(MarkdownError::Limit("rendered text bytes"))?;
            self.check_output(bytes)?;
            self.image.as_mut().unwrap().alt.push_str(text);
            return Ok(());
        }
        self.ensure(range.clone())?;
        self.inline_extras(
            text,
            range,
            InlineStyle {
                code: code || self.style.code,
                ..self.style
            },
        )
    }

    /// Plain text with Obsidian's inline extras: `==highlight==` (markers hidden, still mapped) and `#tag`. Only for
    /// text that is the source verbatim, never inside code.
    ///
    /// Slicing: every offset comes from `find` of an ASCII delimiter (`==`, `[`, `]`, `\n`) in the same string, or
    /// from `title`/`hidden_until`, which were built that way on this source, so all are char boundaries.
    fn inline_extras(
        &mut self,
        text: &str,
        range: Range<usize>,
        mut style: InlineStyle,
    ) -> Result<(), MarkdownError> {
        if let Some((hide, end)) = self.title {
            if range.start < hide && self.doc.source.get(range.clone()) == Some(text) {
                // The `[!type]` marker: hidden but mapped.
                let cut = hide.min(range.end) - range.start;
                self.append("", Origin::Normalized(range.start..range.start + cut), style)?;
                return if cut < text.len() {
                    self.inline_extras(&text[cut..], range.start + cut..range.end, style)
                } else {
                    Ok(())
                };
            }
            if range.start >= hide && range.start < end {
                style.callout_title = true;
                if range.end > end && self.doc.source.get(range.clone()) == Some(text) {
                    let cut = end - range.start;
                    self.inline_extras(&text[..cut], range.start..end, style)?;
                    style.callout_title = false;
                    return self.inline_extras(&text[cut..], end..range.end, style);
                }
            }
        }
        if style.code || self.doc.source.get(range.clone()) != Some(text) {
            return self.mapped(text, range, style);
        }
        if range.start < self.hidden_until {
            let cut = self.hidden_until.min(range.end) - range.start;
            self.append("", Origin::Normalized(range.start..range.start + cut), style)?;
            return if cut < text.len() {
                self.inline_extras(&text[cut..], range.start + cut..range.end, style)
            } else {
                Ok(())
            };
        }
        // `[^label]` of a definition that is not also a CommonMark link definition arrives as plain text.
        if let Some((i, len, label)) = text.match_indices('[').find_map(|(i, _)| {
            let rest = self.doc.source[range.start + i..].strip_prefix("[^")?;
            let end = rest.find([']', '\n'])?;
            (rest[end..].starts_with(']') && self.footnote_labels.contains(&rest[..end].to_lowercase()))
                .then(|| (i, end + 3, rest[..end].to_string()))
        }) {
            let s = range.start;
            self.inline_extras(&text[..i], s..s + i, style)?;
            let number = self.footnote(&label).to_string();
            self.check_output(number.len())?;
            self.append(&number, Origin::Normalized(s + i..s + i + len), InlineStyle { footnote: true, ..style })?;
            self.hidden_until = s + i + len;
            return if i + len < text.len() {
                self.inline_extras(&text[i + len..], s + i + len..range.end, style)
            } else {
                Ok(())
            };
        }
        let mut at = 0;
        while at < text.len() {
            // `==` opens after whitespace or the start and before a non-space; it closes before whitespace,
            // punctuation or the end.
            let open = text[at..]
                .find("==")
                .map(|i| at + i)
                .filter(|&i| i == 0 || text[..i].ends_with(char::is_whitespace))
                .filter(|&i| text[i + 2..].starts_with(|c: char| !c.is_whitespace() && c != '='));
            let close = open
                .and_then(|i| text[i + 2..].find("==").map(|j| i + 2 + j))
                .filter(|&j| {
                    text[j + 2..]
                        .chars()
                        .next()
                        .is_none_or(|c| c.is_whitespace() || c.is_ascii_punctuation())
                });
            match open.zip(close) {
                Some((i, j)) => {
                    let s = range.start;
                    self.tags(&text[at..i], s + at, style)?;
                    self.append("", Origin::Normalized(s + i..s + i + 2), style)?;
                    self.tags(
                        &text[i + 2..j],
                        s + i + 2,
                        InlineStyle {
                            highlight: true,
                            ..style
                        },
                    )?;
                    self.append("", Origin::Normalized(s + j..s + j + 2), style)?;
                    at = j + 2;
                }
                None => {
                    self.tags(&text[at..], range.start + at, style)?;
                    at = text.len();
                }
            }
        }
        Ok(())
    }

    /// `text` (the source at `start`) with each `#tag` styled, never inside link text.
    fn tags(&mut self, text: &str, start: usize, style: InlineStyle) -> Result<(), MarkdownError> {
        if text.is_empty() {
            return Ok(());
        }
        if style.link {
            return self.mapped(text, start..start + text.len(), style);
        }
        let mut at = 0;
        while let Some((s, e)) = next_tag(text, at) {
            if s > at {
                self.mapped(&text[at..s], start + at..start + s, style)?;
            }
            self.mapped(
                &text[s..e],
                start + s..start + e,
                InlineStyle { tag: true, ..style },
            )?;
            at = e;
        }
        if at < text.len() {
            self.mapped(&text[at..], start + at..start + text.len(), style)?;
        }
        Ok(())
    }

    fn html(&mut self, text: &str, range: Range<usize>) -> Result<(), MarkdownError> {
        // HTML events can split a multiline comment into several fragments.
        let mut cursor = 0;
        while cursor < text.len() {
            if self.comment_start.is_some() {
                if let Some(end) = text[cursor..].find("-->") {
                    cursor += end + 3;
                    let start = self.comment_start.take().unwrap();
                    self.hidden(start..range.start + cursor)?;
                } else {
                    return Ok(());
                }
            } else if let Some(start) = text[cursor..].find("<!--") {
                let start = cursor + start;
                if start > cursor {
                    self.text(
                        &text[cursor..start],
                        range.start + cursor..range.start + start,
                        false,
                    )?;
                }
                self.comment_start = Some(range.start + start);
                cursor = start + 4;
            } else {
                if text[cursor..].trim().is_empty()
                    && self.block.is_none()
                    && matches!(self.leaf.as_ref().map(|l| &l.0), Some(BlockKind::Literal))
                {
                    break;
                }
                self.text(&text[cursor..], range.start + cursor..range.end, false)?;
                break;
            }
        }
        Ok(())
    }

    fn event(&mut self, event: Event<'_>, range: Range<usize>) -> Result<(), MarkdownError> {
        if let Event::Start(
            Tag::Link {
                link_type,
                dest_url,
                title,
                ..
            }
            | Tag::Image {
                link_type,
                dest_url,
                title,
                ..
            },
        ) = &event
        {
            self.reference(*link_type, dest_url, title)?;
        }
        if self.skip > 0 {
            match event {
                Event::Start(_) => self.skip += 1,
                Event::End(_) => self.skip -= 1,
                Event::Text(_)
                | Event::Code(_)
                | Event::InlineMath(_)
                | Event::DisplayMath(_)
                | Event::Html(_)
                | Event::InlineHtml(_)
                | Event::FootnoteReference(_)
                | Event::SoftBreak
                | Event::HardBreak
                | Event::Rule
                | Event::TaskListMarker(_) => {}
            }
            return Ok(());
        }
        match event {
            Event::Start(tag) => self.start(tag, range)?,
            Event::End(tag) => self.end(tag, range)?,
            Event::Text(text) => self.text(&text, range, false)?,
            Event::Code(text) => self.text(&text, range, true)?,
            Event::Html(text) | Event::InlineHtml(text) => self.html(&text, range)?,
            Event::SoftBreak | Event::HardBreak if self.title.is_some() => {
                // The callout title is a line of its own; without a title the break disappears.
                let titled = self.title.take().is_some_and(|(hide, end)| hide < end);
                self.ensure(range.clone())?;
                self.append(if titled { "\n" } else { "" }, Origin::Normalized(range), self.style)?;
            }
            Event::SoftBreak => self.text(" ", range, false)?,
            Event::HardBreak => self.text("\n", range, false)?,
            Event::Rule => {
                self.open(BlockKind::Rule, range)?;
                self.append("───", Origin::Synthetic, InlineStyle::default())?;
                self.finish();
            }
            Event::TaskListMarker(checked) => {
                if let Some(item) = self.items.last_mut() {
                    item.checked = Some(checked);
                }
            }
            Event::InlineMath(_) | Event::DisplayMath(_) | Event::FootnoteReference(_) => {
                self.check_output(range.len())?;
                let text = self.doc.source[range.clone()].to_string();
                self.text(&text, range, false)?;
            }
        }
        Ok(())
    }

    fn start(&mut self, tag: Tag<'_>, range: Range<usize>) -> Result<(), MarkdownError> {
        match tag {
            Tag::Paragraph => {
                self.finish();
                self.leaf = Some((BlockKind::Paragraph, range));
            }
            Tag::Heading { level, .. } => {
                self.finish();
                self.leaf = Some((BlockKind::Heading(level as u8), range));
            }
            Tag::CodeBlock(kind) => {
                let language = match kind {
                    CodeBlockKind::Indented => None,
                    CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().map(str::to_string)
                    }
                };
                self.leaf = Some((BlockKind::Code { language }, range.clone()));
                self.style.code = true;
                self.ensure(range)?;
            }
            Tag::HtmlBlock => {
                self.finish();
                self.leaf = Some((BlockKind::Literal, range));
            }
            Tag::BlockQuote(_) => {
                self.quote_depth = self
                    .quote_depth
                    .checked_add(1)
                    .ok_or(MarkdownError::Limit("quote depth"))?;
                if let Some((kind, folded, hide, end)) = callout_marker(&self.doc.source, range.start) {
                    self.callouts.push((
                        self.quote_depth,
                        Callout {
                            id: self.next_callout,
                            kind,
                            folded,
                        },
                    ));
                    self.next_callout = self.next_callout.wrapping_add(1);
                    self.title = Some((hide, end));
                }
            }
            Tag::List(number) => {
                // A parent item may contain only a nested list. Its marker must
                // appear before the children, even without a paragraph event.
                if self.items.last().is_some_and(|i| !i.emitted) {
                    self.ensure(range.clone())?;
                }
                self.finish();
                self.lists.push(number);
            }
            Tag::Item => {
                self.finish();
                let marker = match self.lists.last_mut() {
                    Some(Some(number)) => {
                        let marker = format!("{number}.");
                        *number = number.saturating_add(1);
                        marker
                    }
                    Some(None) | None => "•".into(),
                };
                self.items.push(Item {
                    source: range,
                    marker,
                    checked: None,
                    emitted: false,
                });
            }
            Tag::Table(alignment) => {
                self.finish();
                self.alignment = alignment
                    .into_iter()
                    .map(|a| match a {
                        Alignment::None => TableAlignment::None,
                        Alignment::Left => TableAlignment::Left,
                        Alignment::Center => TableAlignment::Center,
                        Alignment::Right => TableAlignment::Right,
                    })
                    .collect();
            }
            Tag::TableHead | Tag::TableRow => {
                let header = matches!(tag, Tag::TableHead);
                self.open(
                    BlockKind::TableRow {
                        header,
                        cells: Vec::new(),
                        alignment: self.alignment.clone(),
                    },
                    range,
                )?;
            }
            Tag::TableCell => {
                if let Some(Block {
                    kind: BlockKind::TableRow { cells, .. },
                    ..
                }) = &self.block
                    && !cells.is_empty()
                {
                    self.append("\t", Origin::Synthetic, InlineStyle::default())?;
                }
                self.cell_start = Some(self.doc.text.len());
            }
            Tag::Emphasis => {
                self.styles.push(self.style);
                self.style.emphasis = true;
            }
            Tag::Strong => {
                self.styles.push(self.style);
                self.style.strong = true;
            }
            Tag::Strikethrough => {
                self.styles.push(self.style);
                self.style.strike = true;
            }
            Tag::Link {
                link_type,
                id,
                dest_url,
                title,
            } => {
                if let Some(label) = id.strip_prefix('^') {
                    let number = self.footnote(label).to_string();
                    self.check_output(number.len())?;
                    self.ensure(range.clone())?;
                    let style = InlineStyle {
                        footnote: true,
                        ..self.style
                    };
                    self.append(&number, Origin::Normalized(range), style)?;
                    self.skip = 1;
                } else {
                    if self.image.is_some() {
                        self.links.push(None);
                        self.styles.push(self.style);
                        self.style.link = true;
                        return Ok(());
                    }
                    self.ensure(range.clone())?;
                    let (destination, title) = self.target(&id, &dest_url, &title)?;
                    let external =
                        destination.contains("://") || destination.starts_with("mailto:");
                    let start = self.doc.text.len();
                    self.links.push(Some(Link {
                        text: start..start,
                        source: range,
                        destination,
                        title,
                    }));
                    self.styles.push(self.style);
                    self.style.link = true;
                    self.style.external = external;
                    self.style.wikilink = matches!(link_type, LinkType::WikiLink { .. });
                }
            }
            Tag::Image {
                dest_url,
                title,
                id,
                ..
            } => {
                if self.image.is_some() {
                    self.image_depth += 1;
                    return Ok(());
                }
                if !matches!(
                    self.block.as_ref().map(|b| &b.kind),
                    Some(BlockKind::TableRow { .. })
                ) {
                    self.finish();
                }
                self.check_output(
                    dest_url
                        .len()
                        .checked_add(12)
                        .ok_or(MarkdownError::Limit("rendered text bytes"))?,
                )?;
                let (destination, _) = self.target(&id, &dest_url, &title)?;
                self.image = Some(Image {
                    source: range,
                    destination,
                    alt: String::new(),
                });
                self.image_depth = 1;
            }
            Tag::MetadataBlock(_) if range.start > 0 => {
                // A later `---` block in a file with front matter: literal.
                self.definition(range, true)?;
                self.skip = 1;
            }
            Tag::MetadataBlock(_) => {
                // Front matter: raw text, no inline extras; the adapters show rows from `properties`.
                self.finish();
                self.leaf = Some((BlockKind::Properties, range.clone()));
                self.style.code = true;
                self.ensure(range)?;
            }
            Tag::FootnoteDefinition(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::Superscript
            | Tag::Subscript => {
                self.definition(range, true)?;
                self.skip = 1;
            }
        }
        Ok(())
    }

    fn end(&mut self, tag: TagEnd, range: Range<usize>) -> Result<(), MarkdownError> {
        match tag {
            TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::HtmlBlock => {
                self.title = None;
                self.finish();
                self.leaf = None;
            }
            TagEnd::CodeBlock => {
                self.finish();
                self.leaf = None;
                self.style.code = false;
            }
            TagEnd::BlockQuote(_) => {
                if self.callouts.last().is_some_and(|(d, _)| *d == self.quote_depth) {
                    self.callouts.pop();
                    self.title = None;
                }
                self.quote_depth = self.quote_depth.saturating_sub(1);
            }
            TagEnd::List(_) => {
                self.finish();
                self.lists.pop();
            }
            TagEnd::Item => {
                if self.items.last().is_some_and(|i| !i.emitted) {
                    self.ensure(range)?;
                }
                self.finish();
                self.items.pop();
            }
            TagEnd::Table => {
                self.alignment.clear();
            }
            TagEnd::TableHead | TagEnd::TableRow => {
                self.finish();
            }
            TagEnd::TableCell => {
                if let Some(start) = self.cell_start.take()
                    && let Some(Block {
                        kind: BlockKind::TableRow { cells, .. },
                        ..
                    }) = &mut self.block
                {
                    cells.push(start..self.doc.text.len());
                }
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.style = self.styles.pop().ok_or(MarkdownError::InvalidMap)?;
            }
            TagEnd::Link => {
                if let Some(mut link) = self.links.pop().ok_or(MarkdownError::InvalidMap)? {
                    link.text.end = self.doc.text.len();
                    self.doc.links.push(link);
                }
                self.style = self.styles.pop().ok_or(MarkdownError::InvalidMap)?;
            }
            TagEnd::Image => {
                if self.image_depth > 1 {
                    self.image_depth -= 1;
                    return Ok(());
                }
                self.image_depth = 0;
                let image = self.image.take().ok_or(MarkdownError::InvalidMap)?;
                let table = matches!(
                    self.block.as_ref().map(|b| &b.kind),
                    Some(BlockKind::TableRow { .. })
                );
                let bytes = image
                    .alt
                    .len()
                    .checked_add(image.destination.len())
                    .and_then(|n| n.checked_add(12))
                    .ok_or(MarkdownError::Limit("rendered text bytes"))?;
                self.check_output(bytes + usize::from(!table && !self.doc.blocks.is_empty()))?;
                let text = format!("[Image: {} ({})]", image.alt, image.destination);
                if !table {
                    self.open(
                        BlockKind::Image {
                            alt: image.alt,
                            destination: image.destination.to_string(),
                        },
                        self.links.iter().flatten().next().map_or_else(|| image.source.clone(), |link| link.source.clone()),
                    )?;
                }
                self.mapped(&text, image.source, self.style)?;
                if !table {
                    self.finish();
                }
            }
            TagEnd::FootnoteDefinition
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
            | TagEnd::Superscript
            | TagEnd::Subscript => {}
            TagEnd::MetadataBlock(_) => {
                self.finish();
                self.leaf = None;
                self.style.code = false;
            }
        }
        Ok(())
    }
}

/// The next `#tag` at or after `from`: `#` at a word start, then a letter, then letters, digits, `_`, `-` or `/`.
/// Slicing: offsets are `find` results for `#` or a char predicate, so char boundaries.
fn next_tag(text: &str, from: usize) -> Option<(usize, usize)> {
    let mut search = from;
    while let Some(i) = text[search..].find('#').map(|i| search + i) {
        let starts_word = i == 0 || text[..i].ends_with(char::is_whitespace);
        let body = &text[i + 1..];
        if starts_word && body.starts_with(char::is_alphabetic) {
            let len = body
                .find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '/')))
                .unwrap_or(body.len());
            return Some((i, i + 1 + len));
        }
        search = i + 1;
    }
    None
}

/// A callout marker `[!type]` (then `+`/`-`) at the start of the quote whose `>` is at or after `start`: the kind, fold
/// state, where the marker and following spaces end, and where the title ends (source offsets).
/// Slicing: offsets advance over ASCII bytes (`>`, `[!`, the name, `]`, `+`/`-`, blanks), so char boundaries.
fn callout_marker(source: &str, start: usize) -> Option<(CalloutKind, Option<bool>, usize, usize)> {
    let line_end = source[start..].find('\n').map_or(source.len(), |n| start + n);
    let line = &source[start..line_end];
    let quote = line.find('>')?;
    if !line[..quote].trim().is_empty() {
        return None;
    }
    let mut at = quote + 1;
    if matches!(line.as_bytes().get(at), Some(b' ' | b'\t')) {
        at += 1;
    }
    let rest = line[at..].strip_prefix("[!")?;
    let name_len = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-')).unwrap_or(rest.len());
    if name_len == 0 || !rest[name_len..].starts_with(']') {
        return None;
    }
    let kind = CalloutKind::parse(&rest[..name_len]);
    let mut hide = at + 2 + name_len + 1;
    let folded = match line.as_bytes().get(hide) {
        Some(b'-') => Some(true),
        Some(b'+') => Some(false),
        _ => None,
    };
    if folded.is_some() {
        hide += 1;
    }
    hide += line[hide..].len() - line[hide..].trim_start().len();
    let end = at.max(line.trim_end().len()).max(hide);
    Some((kind, folded, start + hide, start + end))
}
