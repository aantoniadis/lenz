//! Several selections in one view: one primary selection and secondaries kept disjoint and in source order.
//!
//! Adapters translate input into [`crate::session::Command`]s; this module holds the placement and merge rules
//! those commands share, so both frontends present the same set.
use crate::{session::Selection, text::TextSnapshot};

/// The caret on the next (or previous) shown line at the caret's column, `None` past the document's end or start.
/// A selection's head names the line and column, so its anchor stays where it was.
pub(crate) fn caret_on_adjacent_line(doc: &TextSnapshot, visible: &crate::folds::Visible, selection: Selection, below: bool) -> Option<Selection> {
    let line = doc.line_of(selection.head);
    let target = visible.step(line, if below { 1 } else { -1 });
    if target == line {
        return None;
    }
    let at = doc.byte_at_column(target, doc.column_of(selection.head));
    Some(Selection { anchor: at, head: at })
}

/// One primary selection and its secondaries, made consistent: ascending by position, repeats dropped, overlaps
/// merged into their union (keeping the earlier selection's direction), and the caret inside a selection absorbed
/// by it. Adjacent selections stay separate, so editing at two neighbouring carets remains two edits.
pub(crate) fn normalize(primary: Selection, secondaries: impl IntoIterator<Item = Selection>) -> (Selection, Vec<Selection>) {
    let mut all: Vec<(Selection, bool)> = std::iter::once((primary, true)).chain(secondaries.into_iter().map(|s| (s, false))).collect();
    all.sort_by_key(|(s, _)| (s.range().start, s.range().end));
    let mut merged: Vec<(Selection, bool)> = Vec::with_capacity(all.len());
    for (candidate, is_primary) in all {
        let Some((last, last_primary)) = merged.last_mut() else {
            merged.push((candidate, is_primary));
            continue;
        };
        let (last_range, candidate_range) = (last.range(), candidate.range());
        if absorbed(&last_range, &candidate_range) {
            let forward = last.head >= last.anchor;
            let start = last_range.start.min(candidate_range.start);
            let end = last_range.end.max(candidate_range.end);
            *last = if forward { Selection { anchor: start, head: end } } else { Selection { anchor: end, head: start } };
            *last_primary |= is_primary;
        } else {
            merged.push((candidate, is_primary));
        }
    }
    let at = merged.iter().position(|(_, is_primary)| *is_primary).unwrap_or(merged.len().saturating_sub(1));
    let primary = merged.get(at).map_or(primary, |(selection, _)| *selection);
    (primary, merged.into_iter().enumerate().filter(|(i, _)| *i != at).map(|(_, (selection, _))| selection).collect())
}

/// Whether a selection repeats, overlaps or covers the one before it: a caret inside a selection is part of it, and
/// two selections that only touch stay separate (typing at neighbouring carets remains two edits).
fn absorbed(last: &std::ops::Range<usize>, candidate: &std::ops::Range<usize>) -> bool {
    if last == candidate { return true; }
    if last.is_empty() { return candidate.start <= last.start && last.start <= candidate.end; }
    if candidate.is_empty() { return last.start <= candidate.start && candidate.start <= last.end; }
    last.start < candidate.end && candidate.start < last.end
}

/// The next literal occurrence of `needle` at or after `from`, then from the document's start, that overlaps no
/// selection already `taken`. Comparisons are case-sensitive and the scan copies one bounded window at a time, so a
/// long document is never assembled whole.
pub(crate) fn next_occurrence(doc: &TextSnapshot, needle: &str, from: usize, taken: &[Selection]) -> Option<Selection> {
    if needle.is_empty() || needle.len() > doc.len_bytes() {
        return None;
    }
    let mut at = doc.floor_char_boundary(from);
    let mut wrapped = false;
    loop {
        match find_from(doc, needle, at) {
            Some(found) => {
                let candidate = found..found + needle.len();
                if !taken.iter().any(|selection| overlaps(&selection.range(), &candidate)) {
                    return Some(Selection { anchor: candidate.start, head: candidate.end });
                }
                at = doc.next_char_boundary(found);
            }
            None if wrapped => return None,
            None => {
                wrapped = true;
                at = 0;
            }
        }
    }
}

fn overlaps(a: &std::ops::Range<usize>, b: &std::ops::Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

/// The first literal match at or after `from`, found one bounded window at a time.
fn find_from(doc: &TextSnapshot, needle: &str, from: usize) -> Option<usize> {
    const WINDOW: usize = 64 << 10;
    let end = doc.len_bytes();
    let mut at = doc.floor_char_boundary(from);
    while at < end {
        let stop = doc.floor_char_boundary(at.saturating_add(WINDOW).min(end));
        if let Some(i) = doc.range(at..stop)?.find(needle) {
            return Some(at + i);
        }
        // A match may begin in the window's last characters and end past it: scan on from just before that.
        let next = stop.saturating_sub(needle.len().saturating_sub(1)).max(doc.next_char_boundary(at));
        if next >= end {
            return None;
        }
        at = doc.floor_char_boundary(next);
    }
    None
}

/// The same text at every selection, in one ascending, non-overlapping transaction.
pub fn edits_at(selections: impl IntoIterator<Item = Selection>, text: &str) -> Vec<crate::text::TextEdit> {
    let mut ranges: Vec<_> = selections.into_iter().map(|selection| selection.range()).collect();
    ranges.sort_by_key(|range| (range.start, range.end));
    ranges.dedup();
    ranges.into_iter().map(|range| crate::text::TextEdit { range, text: text.into() }).collect()
}
