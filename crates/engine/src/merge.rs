// Ported from IntelliJ IDEA Community Edition (commit bc34875):
//   com.intellij.diff.comparison.ComparisonMergeUtil (FairMergeBuilder, ChangeBuilder, IgnoringChangeBuilder),
//   ByLineRt (merge: doCompare for three sides), MergeResolveUtil (tryResolve, SimpleHelper), ComparisonUtil
//   (isEqualTexts); com.intellij.diff.util.MergeRangeUtil (getMergeType, getLineMergeType, getWordMergeType,
//   canResolveLineConflict), MergeRange; DiffUtil.compareThreesideInner (platform/diff-impl).
// Copyright 2000-2026 JetBrains s.r.o. and contributors. Licensed under the Apache License, Version 2.0.
// Modified: translated from Kotlin/Java to Rust; a merge block carries its type as kind + changed Sides + resolvable;
// a too-costly conflict is unresolvable (MAX_REFINE_TOKENS stands in for DiffTooBigException); the greedy resolver,
// the SEMANTIC (PSI) strategy, import handling and cancellation are not ported.

use std::ops::Range;

use crate::by_line::{self, Policy, R, unchanged};
use crate::chars::{equals_ignore_ws, is_punctuation};
use crate::{MAX_REFINE_TOKENS, WhitespaceMode, by_word};

/// Whitespace handling for a merge. `IgnoreTrimChunks` is treated as `Ignore`; there is no ignore-case in merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MergeOptions {
    pub whitespace: WhitespaceMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeKind {
    Inserted,
    Deleted,
    Modified,
    Conflict,
}

/// A run of lines where left, base and right are not all equal (IntelliJ's MergeLineFragment + MergeConflictType).
/// Line ranges index the Sides' lines as given to [`merge_lines`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeBlock {
    pub left: Range<u32>,
    pub base: Range<u32>,
    pub right: Range<u32>,
    pub kind: MergeKind,
    /// The Side differs from base (exactly, for a whitespace-only block).
    pub left_changed: bool,
    pub right_changed: bool,
    /// A `Conflict` that [`resolve_simple`] resolves (IntelliJ's TEXT strategy): base, left and right all non-empty.
    /// Always false otherwise.
    pub resolvable: bool,
    ws_only: bool,
}

impl MergeBlock {
    /// A block that is not whitespace-only, for merges built outside [`merge_lines`] (no common base) and tests.
    pub fn new(left: Range<u32>, base: Range<u32>, right: Range<u32>, kind: MergeKind, left_changed: bool, right_changed: bool) -> Self {
        MergeBlock { left, base, right, kind, left_changed, right_changed, resolvable: false, ws_only: false }
    }

    /// Under Trim or Ignore, equal under the Whitespace Mode on all Sides: shown faded, like an Ignored Change.
    /// Such a block is never a Conflict.
    pub fn whitespace_only(&self) -> bool {
        self.ws_only
    }
}

/// Three ranges (IntelliJ's MergeRange): 1 = left, 2 = base, 3 = right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct R3 {
    pub s1: usize,
    pub e1: usize,
    pub s2: usize,
    pub e2: usize,
    pub s3: usize,
    pub e3: usize,
}

impl R3 {
    pub fn is_empty(&self) -> bool {
        self.s1 == self.e1 && self.s2 == self.e2 && self.s3 == self.e3
    }
}

/// ComparisonManager.mergeLines + MergeRangeUtil.getLineMergeType. Lines as from [`crate::split_lines`].
pub fn merge_lines(left: &[&str], base: &[&str], right: &[&str], options: &MergeOptions) -> Vec<MergeBlock> {
    let policy = crate::policy(options.whitespace);
    line_ranges(left, base, right, policy, true).iter().map(|&m| typed(left, base, right, m, policy)).collect()
}

/// ByLineRt.merge (`keep_ignored`) or ByLineRt.compare for three sides: base→left and base→right line diffs zipped.
pub(crate) fn line_ranges(left: &[&str], base: &[&str], right: &[&str], policy: Policy, keep_ignored: bool) -> Vec<R3> {
    let from_base = |side: &[&str]| unchanged(&by_line::compare_fair(&by_line::prepare(base, side, policy, false)), base.len(), side.len());
    let lens = [left.len(), base.len(), right.len()];
    // IgnoringChangeBuilder: runs equal under the policy but not exactly become blocks of their own
    let exact = |l: usize, b: usize, r: usize| base[b] == left[l] && base[b] == right[r];
    let true_eq: Option<&dyn Fn(usize, usize, usize) -> bool> = (keep_ignored && policy != Policy::Default).then_some(&exact);
    zip(&from_base(left), &from_base(right), lens, true_eq)
}

// ---------------------------------------------------------------- ComparisonMergeUtil

/// FairMergeBuilder: walk the unchanged runs of base→left and base→right (base is `s1..e1` in both) together;
/// every gap between runs unchanged on all three becomes a range. `lens` are left, base, right.
pub(crate) fn zip(left: &[R], right: &[R], lens: [usize; 3], true_eq: Option<&dyn Fn(usize, usize, usize) -> bool>) -> Vec<R3> {
    let mut b = Builder { changes: Vec::new(), index: [0; 3], true_eq };
    let (mut i, mut j) = (0, 0);
    while i < left.len() && j < right.len() {
        let (r1, r2) = (left[i], right[j]);
        if r1.e1 <= r2.s1 {
            i += 1;
            continue;
        }
        if r2.e1 <= r1.s1 {
            j += 1;
            continue;
        }
        let (start_base, end_base) = (r1.s1.max(r2.s1), r1.e1.min(r2.e1));
        let count = end_base - start_base;
        let start_left = r1.s2 + start_base - r1.s1;
        let start_right = r2.s2 + start_base - r2.s1;
        b.mark_equal([start_left, start_base, start_right], [start_left + count, end_base, start_right + count]);
        if r1.e1 <= r2.e1 {
            i += 1;
        } else {
            j += 1;
        }
    }
    b.process_change(b.index, lens);
    b.changes
}

/// ChangeBuilder, or IgnoringChangeBuilder with `true_eq`.
struct Builder<'a> {
    changes: Vec<R3>,
    index: [usize; 3],
    true_eq: Option<&'a dyn Fn(usize, usize, usize) -> bool>,
}

impl Builder<'_> {
    fn mark_equal(&mut self, start: [usize; 3], end: [usize; 3]) {
        debug_assert!((0..3).all(|i| self.index[i] <= start[i] && start[i] <= end[i]));
        self.process_change(self.index, start);
        self.index = end;
    }

    fn process_change(&mut self, start: [usize; 3], end: [usize; 3]) {
        if let Some(eq) = self.true_eq {
            // the equal run since the last change (which may itself be an ignored run)
            let from = self.changes.last().map_or([0; 3], |c| [c.e1, c.e2, c.e3]);
            let count = start[1] - from[1];
            debug_assert!(start[0] - from[0] == count && start[2] - from[2] == count);
            let mut first_ignored = None;
            for i in 0..=count {
                let ignored = i < count && !eq(from[0] + i, from[1] + i, from[2] + i);
                match (ignored, first_ignored) {
                    (true, None) => first_ignored = Some(i),
                    (false, Some(f)) => {
                        self.add([from[0] + f, from[1] + f, from[2] + f], [from[0] + i, from[1] + i, from[2] + i]);
                        first_ignored = None;
                    }
                    _ => {}
                }
            }
        }
        self.add(start, end);
    }

    fn add(&mut self, s: [usize; 3], e: [usize; 3]) {
        let r = R3 { s1: s[0], e1: e[0], s2: s[1], e2: e[1], s3: s[2], e3: e[2] };
        if !r.is_empty() {
            self.changes.push(r);
        }
    }
}

// ---------------------------------------------------------------- MergeRangeUtil

/// getMergeType: the kind and changed Sides of a range from its contents. `eq(a, b)` compares two Sides' contents
/// under the policy and `true_eq` exactly; a conflict is left for the caller to try resolving.
fn merge_type<T: ?Sized>(l: &T, b: &T, r: &T, empty: impl Fn(&T) -> bool, eq: impl Fn(&T, &T) -> bool, true_eq: impl Fn(&T, &T) -> bool) -> (MergeKind, bool, bool) {
    use MergeKind::*;
    let (le, be, re) = (empty(l), empty(b), empty(r));
    debug_assert!(!le || !be || !re);
    if be {
        return match (le, re) {
            (true, _) => (Inserted, false, true),
            (_, true) => (Inserted, true, false),
            // insert/insert: never resolvable, the order of the insertions is unknowable
            _ if eq(l, r) => (Inserted, true, true),
            _ => (Conflict, true, true),
        };
    }
    if le && re {
        return (Deleted, true, true);
    }
    let (unchanged_left, unchanged_right) = (eq(b, l), eq(b, r));
    if unchanged_left && unchanged_right {
        // equal under the policy, not exactly: the changed Sides are those that differ exactly
        let (true_left, true_right) = (true_eq(b, l), true_eq(b, r));
        debug_assert!(!true_left || !true_right);
        return (Modified, !true_left, !true_right);
    }
    if unchanged_left {
        return (if re { Deleted } else { Modified }, false, true);
    }
    if unchanged_right {
        return (if le { Deleted } else { Modified }, true, false);
    }
    if eq(l, r) { (Modified, true, true) } else { (Conflict, true, true) }
}

/// getLineMergeType + canResolveLineConflict for one line range.
fn typed(left: &[&str], base: &[&str], right: &[&str], m: R3, policy: Policy) -> MergeBlock {
    let (l, b, r) = (&left[m.s1..m.e1], &base[m.s2..m.e2], &right[m.s3..m.e3]);
    // compareLineMergeContents: equal line counts, then line by line
    let lines_eq = |p: Policy| move |x: &[&str], y: &[&str]| x.len() == y.len() && x.iter().zip(y).all(|(a, b)| equal_texts(a, b, p));
    let (kind, left_changed, right_changed) = merge_type(l, b, r, <[&str]>::is_empty, lines_eq(policy), lines_eq(Policy::Default));
    let resolvable = kind == MergeKind::Conflict
        && !l.is_empty()
        && !b.is_empty()
        && !r.is_empty()
        && resolve_simple(&l.join("\n"), &b.join("\n"), &r.join("\n")).is_some();
    let ws_only = kind != MergeKind::Conflict
        && match policy {
            Policy::Default => false,
            Policy::Trim => !b.is_empty() && lines_eq(policy)(b, l) && lines_eq(policy)(b, r),
            // DiffUtil.compareThreesideInner: blank lines inserted or deleted count too
            Policy::Ignore => {
                let base = b.join("\n");
                equals_ignore_ws(&base, &l.join("\n")) && equals_ignore_ws(&base, &r.join("\n"))
            }
        };
    let range = |s: usize, e: usize| s as u32..e as u32;
    MergeBlock {
        left: range(m.s1, m.e1),
        base: range(m.s2, m.e2),
        right: range(m.s3, m.e3),
        kind,
        left_changed,
        right_changed,
        resolvable,
        ws_only,
    }
}

/// ComparisonUtil.isEqualTexts: under Trim, each line (inner ones too) is compared trimmed.
pub(crate) fn equal_texts(a: &str, b: &str, policy: Policy) -> bool {
    match policy {
        Policy::Default => a == b,
        Policy::Trim => {
            let (mut x, mut y) = (a.split('\n'), b.split('\n'));
            loop {
                match (x.next(), y.next()) {
                    (None, None) => return true,
                    (Some(p), Some(q)) if p.trim_matches(by_line::is_ws) == q.trim_matches(by_line::is_ws) => {}
                    _ => return false,
                }
            }
        }
        Policy::Ignore => equals_ignore_ws(a, b),
    }
}

// ---------------------------------------------------------------- MergeResolveUtil

/// MergeResolveUtil.tryResolve on block texts (lines joined with "\n", no trailing newline): the Sides' word-level
/// edits applied to base, exactly or failing that ignoring whitespace. `None` when they overlap, or when the texts
/// hold more than [`MAX_REFINE_TOKENS`] words and punctuation marks together (IntelliJ's DiffTooBigException).
pub fn resolve_simple(left: &str, base: &str, right: &str) -> Option<String> {
    if cost(left) + cost(base) + cost(right) > MAX_REFINE_TOKENS {
        return None;
    }
    try_resolve(left, base, right, Policy::Default).or_else(|| try_resolve(left, base, right, Policy::Ignore))
}

/// Words and punctuation marks: the elements the word-level LCS steps compare.
fn cost(text: &str) -> usize {
    by_word::count(text) + text.bytes().filter(|&b| is_punctuation(b as char)).count()
}

/// SimpleHelper.execute.
fn try_resolve(left: &str, base: &str, right: &str, policy: Policy) -> Option<String> {
    let texts = [left, base, right];
    let mut out = String::new();
    let mut last = [0; 3];
    let slice = |m: &R3| [&left[m.s1..m.e1], &base[m.s2..m.e2], &right[m.s3..m.e3]];
    let word_type = |m: &R3, policy: Policy| {
        let [l, b, r] = slice(m);
        let eq = |x: &str, y: &str| equal_texts(x, y, policy);
        merge_type(l, b, r, str::is_empty, eq, |x: &str, y: &str| x == y)
    };
    let gap = |out: &mut String, last: &mut [usize; 3], end: [usize; 3]| {
        let m = R3 { s1: last[0], e1: end[0], s2: last[1], e2: end[1], s3: last[2], e3: end[2] };
        *last = end;
        if m.is_empty() {
            return;
        }
        // unchanged under the policy: keep a Side's whitespace edits
        let [l, b, r] = slice(&m);
        let side = if b == l && b == r {
            1
        } else {
            match word_type(&m, Policy::Default) {
                (_, true, _) => 0,
                (_, _, true) => 2,
                _ => 1,
            }
        };
        out.push_str(slice(&m)[side]);
    };
    for c in by_word::compare3(left, base, right, policy) {
        gap(&mut out, &mut last, [c.s1, c.s2, c.s3]);
        let m = R3 { s1: c.s1, e1: c.e1, s2: c.s2, e2: c.e2, s3: c.s3, e3: c.e3 };
        last = [c.e1, c.e2, c.e3];
        match word_type(&m, policy) {
            (MergeKind::Conflict, ..) => return None,
            (_, true, _) => out.push_str(slice(&m)[0]),
            _ => out.push_str(slice(&m)[2]),
        }
    }
    gap(&mut out, &mut last, texts.map(str::len));
    Some(out)
}

// ---------------------------------------------------------------- DiffUtil.compareThreesideInner

/// Inline Changes in a merge block, as byte ranges into each Side's block text.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MergeInline {
    pub left: Vec<Range<usize>>,
    /// Base's ranges against left, then against right: in that order, not sorted, possibly overlapping.
    pub base: Vec<Range<usize>>,
    pub right: Vec<Range<usize>>,
}

/// DiffUtil.compareThreesideInner: two 2-way word compares, base→left and base→right. A Side is `None` when it is
/// unchanged, resolved or empty. `None` when fewer than two Sides are given, or the texts are too costly to compare
/// (over [`MAX_REFINE_TOKENS`] words and punctuation marks together); under Ignore, empty when all are equal
/// ignoring whitespace (a missing Side counting as empty).
pub fn merge_inline(left: Option<&str>, base: Option<&str>, right: Option<&str>, options: &MergeOptions) -> Option<MergeInline> {
    let policy = crate::policy(options.whitespace);
    let present = [left, base, right].iter().flatten().count();
    if present == 0 {
        return None;
    }
    if policy == Policy::Ignore {
        let (l, b, r) = (left.unwrap_or(""), base.unwrap_or(""), right.unwrap_or(""));
        // whitespace-only changes, e.g. empty lines added or removed
        if equals_ignore_ws(l, b) && equals_ignore_ws(l, r) {
            return Some(MergeInline::default());
        }
    }
    if present == 1 || [left, base, right].iter().flatten().map(|t| cost(t)).sum::<usize>() > MAX_REFINE_TOKENS {
        return None;
    }
    let mut out = MergeInline::default();
    let first = |c: &R| c.s1..c.e1;
    let second = |c: &R| c.s2..c.e2;
    if let (Some(l), Some(b), Some(r)) = (left, base, right) {
        for c in by_word::compare(b, l, policy) {
            out.base.push(first(&c));
            out.left.push(second(&c));
        }
        for c in by_word::compare(b, r, policy) {
            out.base.push(first(&c));
            out.right.push(second(&c));
        }
        return Some(out);
    }
    // two Sides: left or else base against right or else base
    let (t1, t2) = (left.or(base).unwrap(), right.or(base).unwrap());
    let changes = by_word::compare(t1, t2, policy);
    let (ranges1, ranges2) = (changes.iter().map(first).collect(), changes.iter().map(second).collect());
    *(if left.is_some() { &mut out.left } else { &mut out.base }) = ranges1;
    *(if right.is_some() { &mut out.right } else { &mut out.base }) = ranges2;
    Some(out)
}
