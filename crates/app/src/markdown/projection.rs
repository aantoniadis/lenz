//! A compact source snapshot and cancellable projection; source View geometry is untouched.

use super::*;
use crate::view::View;

/// Snapshot once when the comparison/refinement revision changes. Engine inline
/// bytes address LF-joined lines, whereas decoded source can retain CRLF.
pub fn source_diff(doc: &TextDocument, view: &View, revision: u64) -> SourceDiff {
    let starts: [Vec<usize>; 2] = std::array::from_fn(|i| {
        std::iter::once(0)
            .chain(
                doc.text(side(i))
                    .bytes()
                    .enumerate()
                    .filter_map(|(n, b)| (b == b'\n').then_some(n + 1)),
            )
            .collect()
    });
    let hunks = view
        .source_changes()
        .iter()
        .enumerate()
        .map(|(id, block)| {
            let lines = [block.left.clone(), block.right.clone()];
            let bytes = std::array::from_fn(|i| {
                let start = starts[i]
                    .get(lines[i].start as usize)
                    .copied()
                    .unwrap_or(doc.text(side(i)).len());
                let end = starts[i]
                    .get(lines[i].end as usize)
                    .copied()
                    .unwrap_or(doc.text(side(i)).len());
                start..end
            });
            let inline = std::array::from_fn(|i| {
                let Some(changes) = &block.inline else {
                    return Vec::new();
                };
                let mut local = Vec::new();
                let mut offset = 0;
                for line in lines[i].clone() {
                    let length = doc
                        .lines_on(side(i))
                        .get(line as usize)
                        .map_or(0, String::len);
                    local.push((
                        offset,
                        starts[i].get(line as usize).copied().unwrap_or(usize::MAX),
                        length,
                    ));
                    offset += length + 1;
                }
                let translate = |byte: usize| {
                    let index = local
                        .partition_point(|&(at, _, _)| at <= byte)
                        .saturating_sub(1);
                    local
                        .get(index)
                        .and_then(|&(at, start, len)| {
                            (byte - at <= len)
                                .then(|| start.checked_add(byte - at))
                                .flatten()
                        })
                        .unwrap_or(usize::MAX)
                };
                changes
                    .iter()
                    .map(|change| {
                        let span = if i == 0 { &change.left } else { &change.right };
                        if lines[i].is_empty() && span.is_empty() {
                            bytes[i].start..bytes[i].start
                        } else {
                            translate(span.start)..translate(span.end)
                        }
                    })
                    .collect()
            });
            SourceHunk {
                id,
                lines,
                bytes,
                inline,
                ignored: block.ignored,
            }
        })
        .collect();
    SourceDiff { revision, hunks }
}

fn side(i: usize) -> Side {
    if i == 0 { Side::Left } else { Side::Right }
}

fn check(current: &dyn Fn() -> bool) -> Result<(), MarkdownError> {
    if current() {
        Ok(())
    } else {
        Err(MarkdownError::Cancelled)
    }
}
fn valid(text: &str, range: &Range<usize>) -> bool {
    text.get(range.clone()).is_some()
}
fn intersects(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

/// Source intervals can overlap (list markers/image placeholders) and hidden
/// origins are not necessarily emitted in source order. A sorted index with
/// subtree maxima prunes unrelated leaves even under a long enclosing wrapper.
struct Intervals {
    entries: Vec<(Range<usize>, usize)>,
    ends: Vec<usize>,
}
impl Intervals {
    fn new(
        mut entries: Vec<(Range<usize>, usize)>,
        current: &dyn Fn() -> bool,
    ) -> Result<Self, MarkdownError> {
        check(current)?;
        entries.sort_unstable_by_key(|(r, id)| (r.start, *id));
        fn build(
            entries: &[(Range<usize>, usize)],
            ends: &mut [usize],
            current: &dyn Fn() -> bool,
        ) -> Result<usize, MarkdownError> {
            if entries.is_empty() {
                return Ok(0);
            }
            let mid = entries.len() / 2;
            if mid.is_multiple_of(128) {
                check(current)?;
            }
            let left = build(&entries[..mid], &mut ends[..mid], current)?;
            let right = build(&entries[mid + 1..], &mut ends[mid + 1..], current)?;
            let end = entries[mid].0.end.max(left).max(right);
            ends[mid] = end;
            Ok(end)
        }
        let mut ends = vec![0; entries.len()];
        build(&entries, &mut ends, current)?;
        Ok(Self { entries, ends })
    }
    fn candidates<'a>(&'a self, range: &'a Range<usize>) -> Candidates<'a> {
        Candidates {
            index: self,
            range,
            // The traversal stack stores intervals, not their individual indices.
            stack: std::iter::once(0..self.entries.len()).collect(),
        }
    }
}

struct Candidates<'a> {
    index: &'a Intervals,
    range: &'a Range<usize>,
    // Balanced traversal retains at most logarithmically many pending subtrees.
    stack: Vec<Range<usize>>,
}
impl<'a> Iterator for Candidates<'a> {
    type Item = &'a (Range<usize>, usize);
    fn next(&mut self) -> Option<Self::Item> {
        while let Some(tree) = self.stack.pop() {
            if tree.is_empty() {
                continue;
            }
            let mid = tree.start + tree.len() / 2;
            if self.index.ends[mid] <= self.range.start {
                continue;
            }
            let entry = &self.index.entries[mid];
            if entry.0.start < self.range.end {
                self.stack.push(mid + 1..tree.end);
            }
            self.stack.push(tree.start..mid);
            if intersects(&entry.0, self.range) {
                return Some(entry);
            }
        }
        None
    }
}

struct Index {
    blocks: Intervals,
    // Prefix maximum source starts in rendered block order. Only queried when
    // no source interval intersects the hunk: wrapper fragments cannot move an
    // insertion boundary backwards after an image advanced the source frontier.
    block_starts: Vec<usize>,
    segments: Intervals,
}
impl Index {
    fn new(doc: &MarkdownDocument, current: &dyn Fn() -> bool) -> Result<Self, MarkdownError> {
        if doc.source.len() > Limits::DEFAULT.bytes_per_side
            || doc.text.len() > Limits::DEFAULT.rendered_bytes_per_side
        {
            return Err(MarkdownError::Limit("projection bytes"));
        }
        if doc.blocks.len() > Limits::DEFAULT.events_per_side {
            return Err(MarkdownError::Limit("projection blocks"));
        }
        let expected = std::iter::once(0).chain(
            doc.source
                .bytes()
                .enumerate()
                .filter_map(|(n, b)| (b == b'\n').then_some(n + 1)),
        );
        let mut starts = doc.line_starts.iter().copied();
        for (n, byte) in expected.enumerate() {
            if n % 128 == 0 {
                check(current)?;
            }
            if starts.next() != Some(byte) {
                return Err(MarkdownError::InvalidMap);
            }
        }
        if starts.next().is_some() {
            return Err(MarkdownError::InvalidMap);
        }
        let mut text_end = 0;
        let mut source_start = 0;
        let mut block_starts = Vec::with_capacity(doc.blocks.len());
        for (n, block) in doc.blocks.iter().enumerate() {
            if n % 128 == 0 {
                check(current)?;
            }
            if !valid(&doc.source, &block.source)
                || !valid(&doc.text, &block.text)
                || block.text.start < text_end
            {
                return Err(MarkdownError::InvalidMap);
            }
            text_end = block.text.end;
            // Split paragraph fragments retain the enclosing source wrapper,
            // so starts can decrease after an inline image. Keep a monotone
            // frontier in rendered order for insertion/gap boundary lookup.
            source_start = source_start.max(block.source.start);
            block_starts.push(source_start);
        }
        let mut origins = Vec::new();
        text_end = 0;
        for (n, segment) in doc.segments.iter().enumerate() {
            if n % 128 == 0 {
                check(current)?;
            }
            if !valid(&doc.text, &segment.text) || segment.text.start != text_end {
                return Err(MarkdownError::InvalidMap);
            }
            text_end = segment.text.end;
            match &segment.origin {
                Origin::Direct(source) | Origin::Normalized(source) => {
                    if !valid(&doc.source, source) {
                        return Err(MarkdownError::InvalidMap);
                    }
                    if matches!(segment.origin, Origin::Direct(_))
                        && doc.source[source.clone()] != doc.text[segment.text.clone()]
                    {
                        return Err(MarkdownError::InvalidMap);
                    }
                    origins.push((source.clone(), n));
                }
                Origin::Synthetic => {}
            }
        }
        if text_end != doc.text.len() {
            return Err(MarkdownError::InvalidMap);
        }
        Ok(Self {
            block_starts,
            blocks: Intervals::new(
                doc.blocks
                    .iter()
                    .enumerate()
                    .map(|(n, b)| (b.source.clone(), n))
                    .collect(),
                current,
            )?,
            segments: Intervals::new(origins, current)?,
        })
    }
    fn blocks(
        &self,
        bytes: &Range<usize>,
        current: &dyn Fn() -> bool,
    ) -> Result<Range<usize>, MarkdownError> {
        // A point strictly inside a leaf belongs to it; an exact block boundary
        // stays empty so an insertion cannot consume the following block.
        let query = if bytes.is_empty() {
            bytes.start..bytes.end.saturating_add(1)
        } else {
            bytes.clone()
        };
        let mut first = usize::MAX;
        let mut end = 0;
        for (n, (source, id)) in self.blocks.candidates(&query).enumerate() {
            if n % 128 == 0 {
                check(current)?;
            }
            let hit = if bytes.is_empty() {
                source.start < bytes.start && bytes.start < source.end
            } else {
                intersects(source, bytes)
            };
            if hit {
                first = first.min(*id);
                end = end.max(id + 1);
            }
        }
        if first != usize::MAX {
            return Ok(first..end);
        }
        let at = self
            .block_starts
            .partition_point(|&start| start < bytes.start);
        Ok(at..at)
    }
    fn mapped(
        &self,
        doc: &MarkdownDocument,
        ranges: &[Range<usize>],
        current: &dyn Fn() -> bool,
    ) -> Result<Vec<Range<usize>>, MarkdownError> {
        let mut mapped = Vec::new();
        for range in ranges {
            for (n, (source, id)) in self.segments.candidates(range).enumerate() {
                if n % 128 == 0 {
                    check(current)?;
                }
                let segment = &doc.segments[*id];
                if segment.text.is_empty() || !intersects(source, range) {
                    continue;
                }
                let text = match segment.origin {
                    Origin::Direct(_) => {
                        segment.text.start + range.start.max(source.start) - source.start
                            ..segment.text.start + range.end.min(source.end) - source.start
                    }
                    Origin::Normalized(_) => segment.text.clone(),
                    Origin::Synthetic => unreachable!(),
                };
                mapped.push(text);
                if mapped.len() > Limits::DEFAULT.segments_per_pair {
                    return Err(MarkdownError::Limit("projection decorations"));
                }
            }
        }
        mapped.sort_unstable_by_key(|r| (r.start, r.end));
        let mut merged: Vec<Range<usize>> = Vec::new();
        for range in mapped {
            if let Some(last) = merged.last_mut().filter(|last| range.start <= last.end) {
                last.end = last.end.max(range.end);
            } else {
                merged.push(range);
            }
        }
        Ok(merged)
    }
}

fn same_text(
    pair: &ParsedPair,
    mapped: &[Vec<Range<usize>>; 2],
    current: &dyn Fn() -> bool,
) -> Result<bool, MarkdownError> {
    // Compare borrowed spans without constructing copies of the rendered words.
    let chars = |i: usize| {
        mapped[i]
            .iter()
            .flat_map(move |r| pair.sides[i].text[r.clone()].chars())
    };
    let mut left = chars(0);
    let mut right = chars(1);
    let mut n = 0;
    loop {
        if n % 128 == 0 {
            check(current)?;
        }
        n += 1;
        match (left.next(), right.next()) {
            (None, None) => return Ok(true),
            (a, b) if a != b => return Ok(false),
            _ => {}
        }
    }
}

/// Reproject the existing parsed Arc after option/refinement changes. Every hunk
/// produces decorations on both sides, including navigable hidden-source markers.
pub fn project(
    pair: Arc<ParsedPair>,
    diff: SourceDiff,
    current: &dyn Fn() -> bool,
) -> Result<Projection, MarkdownError> {
    check(current)?;
    if pair.sides.iter().map(|d| d.segments.len()).sum::<usize>()
        > Limits::DEFAULT.segments_per_pair
        || diff.hunks.len() > Limits::DEFAULT.segments_per_pair
    {
        return Err(MarkdownError::Limit("projection segments"));
    }
    let indices = [
        Index::new(&pair.sides[0], current)?,
        Index::new(&pair.sides[1], current)?,
    ];
    let mut changed: Vec<AlignmentGroup> = Vec::new();
    let mut decorations = Vec::new();
    let mut previous = [0, 0];
    // Hunks inside one multi-line placeholder (an image's alt text) all map to the whole segment: compare each
    // distinct pair of mapped spans once, not once per hunk.
    let mut compared: std::collections::HashMap<[Vec<std::ops::Range<usize>>; 2], bool> = std::collections::HashMap::new();
    for (id, hunk) in diff.hunks.iter().enumerate() {
        check(current)?;
        let mut blocks = [0..0, 0..0];
        let mut mapped = [Vec::new(), Vec::new()];
        for i in 0..2 {
            let doc = &pair.sides[i];
            let bytes = &hunk.bytes[i];
            let lines = &hunk.lines[i];
            if hunk.id != id
                || !valid(&doc.source, bytes)
                || bytes.start < previous[i]
                || lines.start > lines.end
                || lines.end as usize > doc.line_starts.len()
                || doc
                    .line_starts
                    .get(lines.start as usize)
                    .copied()
                    .unwrap_or(doc.source.len())
                    != bytes.start
                || doc
                    .line_starts
                    .get(lines.end as usize)
                    .copied()
                    .unwrap_or(doc.source.len())
                    != bytes.end
            {
                return Err(MarkdownError::InvalidMap);
            }
            previous[i] = bytes.end;
            if hunk.inline[i].len() > Limits::DEFAULT.segments_per_pair {
                return Err(MarkdownError::Limit("projection inline spans"));
            }
            if hunk.inline[i]
                .iter()
                .any(|r| !valid(&doc.source, r) || r.start < bytes.start || r.end > bytes.end)
            {
                return Err(MarkdownError::InvalidMap);
            }
            blocks[i] = indices[i].blocks(bytes, current)?;
            let ranges = if hunk.inline[i].is_empty() {
                std::slice::from_ref(bytes)
            } else {
                &hunk.inline[i]
            };
            mapped[i] = indices[i].mapped(doc, ranges, current)?;
        }
        let markup = match compared.get(&mapped) {
            Some(&markup) => markup,
            None => {
                let markup = same_text(&pair, &mapped, current)?;
                compared.insert(mapped.clone(), markup);
                markup
            }
        };
        let kind = match (hunk.lines[0].is_empty(), hunk.lines[1].is_empty()) {
            (true, _) => Kind::Inserted,
            (_, true) => Kind::Deleted,
            _ => Kind::Modified,
        };
        for i in 0..2 {
            let doc = &pair.sides[i];
            if markup || mapped[i].is_empty() {
                let text = if blocks[i].is_empty() {
                    let at = doc
                        .blocks
                        .get(blocks[i].start)
                        .map_or(doc.text.len(), |b| b.text.start);
                    at..at
                } else {
                    doc.blocks[blocks[i].start].text.start..doc.blocks[blocks[i].end - 1].text.end
                };
                decorations.push(Decoration {
                    hunk: id,
                    side: side(i),
                    text,
                    source: hunk.bytes[i].clone(),
                    kind,
                    markup: true,
                    ignored: hunk.ignored,
                });
            } else {
                for text in mapped[i].drain(..) {
                    decorations.push(Decoration {
                        hunk: id,
                        side: side(i),
                        text,
                        source: hunk.bytes[i].clone(),
                        kind,
                        markup: false,
                        ignored: hunk.ignored,
                    });
                }
            }
        }
        if decorations.len() > Limits::DEFAULT.segments_per_pair {
            return Err(MarkdownError::Limit("projection decorations"));
        }
        if let Some(last) = changed
            .last_mut()
            .filter(|last| (0..2).any(|i| blocks[i].start < last.blocks[i].end))
        {
            for (last, block) in last.blocks.iter_mut().zip(&blocks) {
                last.start = last.start.min(block.start);
                last.end = last.end.max(block.end);
            }
            last.hunks.push(id);
        } else {
            changed.push(AlignmentGroup {
                blocks,
                hunks: vec![id],
            });
        }
    }
    let mut groups = Vec::new();
    let mut at = [0, 0];
    for group in changed {
        check(current)?;
        if (0..2).any(|i| group.blocks[i].start < at[i]) {
            return Err(MarkdownError::InvalidMap);
        }
        if (0..2).any(|i| at[i] < group.blocks[i].start) {
            groups.push(AlignmentGroup {
                blocks: std::array::from_fn(|i| at[i]..group.blocks[i].start),
                hunks: Vec::new(),
            });
        }
        at = std::array::from_fn(|i| group.blocks[i].end);
        groups.push(group);
    }
    if (0..2).any(|i| at[i] < pair.sides[i].blocks.len()) {
        groups.push(AlignmentGroup {
            blocks: std::array::from_fn(|i| at[i]..pair.sides[i].blocks.len()),
            hunks: Vec::new(),
        });
    }
    // Validate the representation invariant in linear work before publication.
    let mut represented = vec![false; diff.hunks.len()];
    for (n, decoration) in decorations.iter().enumerate() {
        if n % 128 == 0 {
            check(current)?;
        }
        represented[decoration.hunk] = true;
    }
    if represented.iter().any(|represented| !represented) {
        return Err(MarkdownError::InvalidMap);
    }
    check(current)?;
    let inline = super::inline_blocks(&pair, &groups);
    let mut whole: [Vec<usize>; 2] = Default::default();
    for b in &inline {
        if matches!((b.side, b.change), (Side::Right, Some(Kind::Inserted)) | (Side::Left, Some(Kind::Deleted))) {
            whole[b.side as usize].push(b.block);
        }
    }
    for side in &mut whole {
        side.sort_unstable();
    }
    Ok(Projection {
        pair,
        groups,
        decorations,
        revision: diff.revision,
        inline,
        whole,
    })
}

impl Projection {
    /// `Inserted` or `Deleted` when `block` on `side` was added or removed as a whole.
    pub fn whole_change(&self, side: Side, block: usize) -> Option<Kind> {
        self.whole[side as usize].binary_search(&block).ok().map(|_| if side == Side::Right { Kind::Inserted } else { Kind::Deleted })
    }

    pub fn represents_hunk(&self, id: usize) -> bool {
        self.decorations
            .get(self.decorations.partition_point(|d| d.hunk < id))
            .is_some_and(|d| d.hunk == id)
    }
    pub fn location(&self, hunk: usize, side: Side) -> Option<SourceAnchor> {
        // Published decorations are ordered by original hunk ID, then side.
        self.decorations[self.decorations.partition_point(|d| d.hunk < hunk)..]
            .iter()
            .take_while(|d| d.hunk == hunk)
            .find(|d| d.hunk == hunk && d.side == side)
            .map(|d| SourceAnchor {
                side,
                byte: d.source.start,
            })
    }
}

impl Projection {
    pub fn valid_selection(&self, selection: super::RenderSelection) -> bool {
        let text = &self.pair.sides[selection.side as usize].text;
        text.is_char_boundary(selection.anchor) && text.is_char_boundary(selection.head)
    }
    pub fn copy_visible(&self, selection: super::RenderSelection) -> Result<String, MarkdownError> {
        if !self.valid_selection(selection) {
            return Err(MarkdownError::InvalidMap);
        }
        Ok(self.pair.sides[selection.side as usize].text
            [selection.anchor.min(selection.head)..selection.anchor.max(selection.head)]
            .into())
    }
    /// Mapped source span only; synthetic placeholders never invent source bytes.
    pub fn source_selection(
        &self,
        selection: super::RenderSelection,
    ) -> Option<crate::selection::Selection> {
        if !self.valid_selection(selection) {
            return None;
        }
        let doc = &self.pair.sides[selection.side as usize];
        let selected = selection.anchor.min(selection.head)..selection.anchor.max(selection.head);
        let mut source: Option<Range<usize>> = None;
        for segment in &doc.segments {
            if !intersects(&selected, &segment.text) {
                continue;
            }
            let mapped = match &segment.origin {
                Origin::Direct(r) => {
                    r.start + selected.start.max(segment.text.start) - segment.text.start
                        ..r.start + selected.end.min(segment.text.end) - segment.text.start
                }
                Origin::Normalized(r) => r.clone(),
                Origin::Synthetic => continue,
            };
            if let Some(range) = &mut source {
                range.start = range.start.min(mapped.start);
                range.end = range.end.max(mapped.end);
            } else {
                source = Some(mapped);
            }
        }
        let source = source?;
        let start = doc.source_pos(source.start)?;
        let end = doc.source_pos(source.end)?;
        let (anchor, head) = if selection.anchor > selection.head {
            (end, start)
        } else {
            (start, end)
        };
        Some(crate::selection::Selection {
            side: selection.side,
            anchor,
            head,
        })
    }
    /// Copy enclosing original blocks, merging overlapping wrappers (for example
    /// paragraphs split by inline images). This is deliberately block-scoped.
    pub fn copy_source(&self, selection: super::RenderSelection) -> Result<String, MarkdownError> {
        if !self.valid_selection(selection) {
            return Err(MarkdownError::InvalidMap);
        }
        if selection.anchor == selection.head { return Ok(String::new()); }
        let doc = &self.pair.sides[selection.side as usize];
        let selected = selection.anchor.min(selection.head)..selection.anchor.max(selection.head);
        let mut ranges = Vec::new();
        for block in &doc.blocks {
            if !intersects(&block.text, &selected) {
                continue;
            }
            let selected_block =
                block.text.start.max(selected.start)..block.text.end.min(selected.end);
            let start = doc
                .segments
                .partition_point(|s| s.text.end <= selected_block.start);
            if doc.segments[start..]
                .iter()
                .take_while(|s| s.text.start < selected_block.end)
                .any(|s| {
                    intersects(&s.text, &selected_block) && !matches!(s.origin, Origin::Synthetic)
                })
            {
                ranges.push(block.source.clone());
            }
        }
        ranges.sort_unstable_by_key(|r| (r.start, r.end));
        let mut merged: Vec<Range<usize>> = Vec::new();
        for range in ranges {
            if let Some(last) = merged.last_mut().filter(|r| range.start <= r.end) {
                last.end = last.end.max(range.end);
            } else {
                merged.push(range);
            }
        }
        let mut copied = String::new();
        for range in merged {
            copied.push_str(&doc.source[range]);
        }
        Ok(copied)
    }
    /// Navigation-only source point mapping. Prefer the following segment at a
    /// boundary; hidden gaps use the nearest endpoint (ties go forward). A
    /// normalized interior maps to its rendered start, never an exact source caret.
    pub fn source_point(&self, anchor: SourceAnchor) -> Option<usize> {
        let doc = &self.pair.sides[anchor.side as usize];
        if !doc.source.is_char_boundary(anchor.byte) { return None; }
        let mut nearest: Option<(usize, bool, usize)> = None;
        for segment in doc.segments.iter().filter(|s| !s.text.is_empty()) {
            let origin = match &segment.origin {
                Origin::Direct(r) | Origin::Normalized(r) => r,
                Origin::Synthetic => continue,
            };
            if origin.contains(&anchor.byte) {
                return Some(match &segment.origin {
                    Origin::Direct(_) => segment.text.start + anchor.byte - origin.start,
                    _ => segment.text.start,
                });
            }
            let forward = anchor.byte <= origin.start;
            let (distance, byte) = if forward {
                (origin.start - anchor.byte, segment.text.start)
            } else { (anchor.byte - origin.end, segment.text.end) };
            let candidate = (distance, !forward, byte);
            if nearest.is_none_or(|old| candidate < old) { nearest = Some(candidate); }
        }
        nearest.map(|(_, _, byte)| byte)
    }
    /// Rendered spans in display order. Empty spans mark hidden markup; the caller
    /// retains the original source hit for Show Source and source search ordering.
    pub fn find_source(&self, side: Side, source: Range<usize>) -> Vec<Range<usize>> {
        let doc = &self.pair.sides[side as usize];
        if !valid(&doc.source, &source) {
            return Vec::new();
        }
        let mut found = Vec::new();
        for segment in &doc.segments {
            let origin = match &segment.origin {
                Origin::Direct(r) | Origin::Normalized(r) => r,
                Origin::Synthetic => continue,
            };
            if !intersects(origin, &source) {
                continue;
            }
            let mapped = match &segment.origin {
                Origin::Direct(_) => {
                    segment.text.start + source.start.max(origin.start) - origin.start
                        ..segment.text.start + source.end.min(origin.end) - origin.start
                }
                _ => segment.text.clone(),
            };
            if found.last() != Some(&mapped) {
                found.push(mapped);
            }
        }
        found
    }
}
