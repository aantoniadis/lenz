//! The Inline view's arrangement: unchanged blocks once, removed and inserted words in place, whole blocks where a
//! word-by-word mix would not read well. Spans point into each side's rendered text.
use super::{AlignmentGroup, BlockKind, ParsedPair};
use crate::view::{Kind, Side};
use std::ops::Range;

#[derive(Clone, Debug, PartialEq)]
pub struct InlineSpan {
    pub side: Side,
    pub text: Range<usize>,
    /// `Inserted` or `Deleted`; `None` for unchanged text.
    pub change: Option<Kind>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct InlineBlock {
    /// The block whose kind and styles the row uses.
    pub side: Side,
    pub block: usize,
    /// The alignment group it comes from.
    pub group: usize,
    pub spans: Vec<InlineSpan>,
    /// `Inserted` (added), `Deleted` (removed), `Modified` (words changed in place) or `None`.
    pub change: Option<Kind>,
}

/// Blocks whose words are mixed in place; others change as whole blocks.
fn interleaves(kind: &BlockKind) -> bool {
    matches!(kind, BlockKind::Paragraph | BlockKind::Heading(_) | BlockKind::ListItem { .. } | BlockKind::Footnote { .. })
}

pub fn inline_blocks(pair: &ParsedPair, groups: &[AlignmentGroup]) -> Vec<InlineBlock> {
    let mut out = Vec::new();
    let whole = |side: Side, block: usize, group: usize, change: Option<Kind>| {
        let text = pair.sides[side as usize].blocks[block].text.clone();
        InlineBlock { side, block, group, spans: vec![InlineSpan { side, text, change }], change }
    };
    for (g, group) in groups.iter().enumerate() {
        if group.hunks.is_empty() {
            out.extend(group.blocks[1].clone().map(|b| whole(Side::Right, b, g, None)));
            continue;
        }
        let (left, right) = (group.blocks[0].clone(), group.blocks[1].clone());
        for k in 0..left.len().max(right.len()) {
            let (l, r) = (left.clone().nth(k), right.clone().nth(k));
            match (l, r) {
                (Some(l), Some(r)) => out.extend(pair_blocks(pair, l, r, g)),
                (Some(l), None) => out.push(whole(Side::Left, l, g, Some(Kind::Deleted))),
                (None, Some(r)) => out.push(whole(Side::Right, r, g, Some(Kind::Inserted))),
                (None, None) => {}
            }
        }
    }
    out
}

/// One old and one new block at the same place in a changed group.
/// Slicing: block text ranges come from the parser and word-diff offsets are char boundaries.
fn pair_blocks(pair: &ParsedPair, l: usize, r: usize, group: usize) -> Vec<InlineBlock> {
    let (lb, rb) = (&pair.sides[0].blocks[l], &pair.sides[1].blocks[r]);
    let (lt, rt) = (&pair.sides[0].text[lb.text.clone()], &pair.sides[1].text[rb.text.clone()]);
    let span = |side: Side, text: Range<usize>, change| InlineSpan { side, text, change };
    let whole = |side: Side, block: usize, b: &super::Block, change| InlineBlock {
        side, block, group, spans: vec![span(side, b.text.clone(), Some(change))], change: Some(change),
    };
    if lt == rt {
        // markup only: the Markup change marker carries it
        return vec![InlineBlock { side: Side::Right, block: r, group, spans: vec![span(Side::Right, rb.text.clone(), None)], change: None }];
    }
    let same_kind = std::mem::discriminant(&lb.kind) == std::mem::discriminant(&rb.kind);
    let changes = if same_kind && interleaves(&rb.kind) { lenz_engine::word_changes(lt, rt) } else { Vec::new() };
    let inserted: usize = changes.iter().map(|c| c[1].len()).sum();
    if changes.is_empty() || (rt.len() - inserted) * 3 < lt.len().max(rt.len()) {
        return vec![whole(Side::Left, l, lb, Kind::Deleted), whole(Side::Right, r, rb, Kind::Inserted)];
    }
    let (ls, rs) = (lb.text.start, rb.text.start);
    let mut spans = Vec::new();
    let mut at = 0;
    for [old, new] in changes {
        if at < new.start {
            spans.push(span(Side::Right, rs + at..rs + new.start, None));
        }
        if !old.is_empty() {
            spans.push(span(Side::Left, ls + old.start..ls + old.end, Some(Kind::Deleted)));
        }
        if !new.is_empty() {
            spans.push(span(Side::Right, rs + new.start..rs + new.end, Some(Kind::Inserted)));
        }
        at = new.end;
    }
    if at < rt.len() {
        spans.push(span(Side::Right, rs + at..rs + rt.len(), None));
    }
    vec![InlineBlock { side: Side::Right, block: r, group, spans, change: Some(Kind::Modified) }]
}
