// Ported from IntelliJ IDEA Community Edition (commit bc34875):
//   com.intellij.diff.comparison.ByLineRt, ChangeCorrector.SmartLineChangeCorrector, ChunkOptimizer,
//   ChunkOptimizer.LineChunkOptimizer, iterables.DiffIterableUtil (ChangeBuilder,
//   ExpandChangeBuilder), TrimUtil (expand*).
// Copyright 2000-2024 JetBrains s.r.o. and contributors. Licensed under the Apache License, Version 2.0.
// Modified: translated from Kotlin to Rust; lines are compared through interned policy keys;
// the LCS step uses gix-imara-diff's Myers instead of IntelliJ's MyersLCS/PatienceIntLCS;
// cancellation is not ported.

use gix_imara_diff::{Algorithm, Diff, Token};

/// Lines with 3 or fewer non-whitespace characters can't anchor the first matching pass.
const UNIMPORTANT_LINE_CHAR_COUNT: usize = 3;

/// A change between two sequences: `[s1, e1)` on the left replaced by `[s2, e2)` on the right.
/// Used for unchanged runs too, where both lengths are equal ("fair").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct R {
    pub s1: usize,
    pub e1: usize,
    pub s2: usize,
    pub e2: usize,
}

impl R {
    pub fn is_empty(&self) -> bool {
        self.s1 == self.e1 && self.s2 == self.e2
    }
}

/// One Side's lines as interned keys under one policy.
pub struct Keys<'a> {
    pub ids: &'a [u32],
    pub non_space: &'a [usize],
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    Default,
    Trim,
    Ignore,
}

/// Interned per-policy line keys for both Sides.
pub struct Prepared {
    pub policy_left: Vec<u32>,
    pub policy_right: Vec<u32>,
    pub iw_left: Vec<u32>,
    pub iw_right: Vec<u32>,
    pub non_space_left: Vec<usize>,
    pub non_space_right: Vec<usize>,
    pub tokens: u32,
}

pub(crate) fn is_ws(c: char) -> bool {
    c == ' ' || c == '\t' || c == '\n'
}

fn key(line: &str, policy: Policy, fold_case: bool) -> String {
    let folded;
    let line = if fold_case {
        folded = crate::chars::fold_str(line);
        folded.as_str()
    } else {
        line
    };
    match policy {
        Policy::Default => line.to_string(),
        Policy::Trim => line.trim_matches(is_ws).to_string(),
        Policy::Ignore => line.chars().filter(|&c| !is_ws(c)).collect(),
    }
}

pub fn prepare(left: &[&str], right: &[&str], policy: Policy, fold_case: bool) -> Prepared {
    let mut table: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut intern = |s: String| {
        let n = table.len() as u32;
        *table.entry(s).or_insert(n)
    };
    let mut ids = |lines: &[&str], p: Policy| -> Vec<u32> { lines.iter().map(|l| intern(key(l, p, fold_case))).collect() };
    let policy_left = ids(left, policy);
    let policy_right = ids(right, policy);
    let iw_left = ids(left, Policy::Ignore);
    let iw_right = ids(right, Policy::Ignore);
    let non_space = |lines: &[&str]| lines.iter().map(|l| l.chars().filter(|&c| !is_ws(c)).count()).collect();
    Prepared {
        non_space_left: non_space(left),
        non_space_right: non_space(right),
        policy_left,
        policy_right,
        iw_left,
        iw_right,
        tokens: table.len() as u32,
    }
}

/// ByLineRt.doCompare: line-level changes under `policy`.
pub fn compare(p: &Prepared, policy: Policy) -> Vec<R> {
    let iw1 = Keys { ids: &p.iw_left, non_space: &p.non_space_left };
    let iw2 = Keys { ids: &p.iw_right, non_space: &p.non_space_right };
    let (len1, len2) = (p.iw_left.len(), p.iw_right.len());
    if policy == Policy::Ignore {
        let changes = compare_smart(&iw1, &iw2, p.tokens);
        let changes = optimize_line_chunks(&iw1, &iw2, &changes);
        return expand_ranges(&iw1, &iw2, &changes, len1, len2);
    }
    compare_fair(p)
}

/// ByLineRt.doCompare's non-Ignore branch: match ignoring whitespace, then re-pair lines under the prepared policy.
/// The three-way comparison takes this branch under every policy, Ignore included.
pub fn compare_fair(p: &Prepared) -> Vec<R> {
    let iw1 = Keys { ids: &p.iw_left, non_space: &p.non_space_left };
    let iw2 = Keys { ids: &p.iw_right, non_space: &p.non_space_right };
    let p1 = Keys { ids: &p.policy_left, non_space: &p.non_space_left };
    let p2 = Keys { ids: &p.policy_right, non_space: &p.non_space_right };
    let iw_changes = compare_smart(&iw1, &iw2, p.tokens);
    let iw_changes = optimize_line_chunks(&p1, &p2, &iw_changes);
    correct_changes_second_step(&p1, &p2, &iw1, &iw2, &iw_changes)
}

// ---------------------------------------------------------------- LCS + iterables

/// Plain LCS over token ids (DiffIterableUtil.diff).
pub(crate) fn diff(a: &[u32], b: &[u32], tokens: u32) -> Vec<R> {
    // One side empty: the one hunk, without gix's tables.
    if a.is_empty() || b.is_empty() {
        return if a.is_empty() && b.is_empty() { Vec::new() } else { vec![R { s1: 0, e1: a.len(), s2: 0, e2: b.len() }] };
    }
    // gix sizes its occurrence tables by the largest id, O(tokens) per call. A gap of a big file (compare_smart
    // diffs every gap) gets dense local ids, so each gap costs what the gap is, not what the file is.
    let mut local: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    let mut id = |t: u32| -> u32 { let next = local.len() as u32; *local.entry(t).or_insert(next) };
    let (a, b, tokens): (Vec<Token>, Vec<Token>, u32) = if tokens as usize > 4 * (a.len() + b.len()) {
        let a = a.iter().map(|&t| Token(id(t))).collect();
        let b = b.iter().map(|&t| Token(id(t))).collect();
        (a, b, local.len() as u32)
    } else {
        (a.iter().map(|&t| Token(t)).collect(), b.iter().map(|&t| Token(t)).collect(), tokens)
    };
    let mut d = Diff::default();
    d.compute_with(Algorithm::Myers, &a, &b, tokens);
    d.hunks()
        .map(|h| R { s1: h.before.start as usize, e1: h.before.end as usize, s2: h.after.start as usize, e2: h.after.end as usize })
        .collect()
}

/// Unchanged runs between the given changes.
pub(crate) fn unchanged(changes: &[R], len1: usize, len2: usize) -> Vec<R> {
    let mut out = Vec::new();
    let (mut l1, mut l2) = (0, 0);
    for c in changes {
        if c.s1 > l1 || c.s2 > l2 {
            out.push(R { s1: l1, e1: c.s1, s2: l2, e2: c.s2 });
        }
        l1 = c.e1;
        l2 = c.e2;
    }
    if len1 > l1 || len2 > l2 {
        out.push(R { s1: l1, e1: len1, s2: l2, e2: len2 });
    }
    out
}

/// Changes between the given unchanged runs (DiffIterableUtil.createUnchanged).
pub(crate) fn changes_from_unchanged(unchanged: &[R], len1: usize, len2: usize) -> Vec<R> {
    let mut b = ChangeBuilder::new(len1, len2);
    for u in unchanged {
        b.mark_equal(u.s1, u.s2, u.e1, u.e2, |_, _| false);
    }
    b.finish(|_, _| false)
}

/// DiffIterableUtil.ChangeBuilder; `expand` is ExpandChangeBuilder's equality (or always-false).
pub(crate) struct ChangeBuilder {
    len1: usize,
    len2: usize,
    index1: usize,
    index2: usize,
    changes: Vec<R>,
}

impl ChangeBuilder {
    pub(crate) fn new(len1: usize, len2: usize) -> Self {
        ChangeBuilder { len1, len2, index1: 0, index2: 0, changes: Vec::new() }
    }

    pub(crate) fn mark_equal(&mut self, i1: usize, i2: usize, e1: usize, e2: usize, eq: impl Fn(usize, usize) -> bool) {
        if i1 == e1 && i2 == e2 {
            return;
        }
        debug_assert!(self.index1 <= i1 && self.index2 <= i2 && i1 <= e1 && i2 <= e2);
        if self.index1 != i1 || self.index2 != i2 {
            self.add_change(self.index1, self.index2, i1, i2, &eq);
        }
        self.index1 = e1;
        self.index2 = e2;
    }

    fn add_change(&mut self, s1: usize, s2: usize, e1: usize, e2: usize, eq: &impl Fn(usize, usize) -> bool) {
        let r = expand(eq, s1, s2, e1, e2);
        if !r.is_empty() {
            self.changes.push(r);
        }
    }

    pub(crate) fn finish(mut self, eq: impl Fn(usize, usize) -> bool) -> Vec<R> {
        if self.len1 != self.index1 || self.len2 != self.index2 {
            let (i1, i2, l1, l2) = (self.index1, self.index2, self.len1, self.len2);
            self.add_change(i1, i2, l1, l2, &eq);
        }
        self.changes
    }
}

pub(crate) fn expand(eq: &impl Fn(usize, usize) -> bool, s1: usize, s2: usize, e1: usize, e2: usize) -> R {
    let f = expand_forward(eq, s1, s2, e1, e2);
    let (s1, s2) = (s1 + f, s2 + f);
    let b = expand_backward(eq, s1, s2, e1, e2);
    R { s1, e1: e1 - b, s2, e2: e2 - b }
}

pub(crate) fn expand_forward(eq: &impl Fn(usize, usize) -> bool, mut s1: usize, mut s2: usize, e1: usize, e2: usize) -> usize {
    let start = s1;
    while s1 < e1 && s2 < e2 && eq(s1, s2) {
        s1 += 1;
        s2 += 1;
    }
    s1 - start
}

pub(crate) fn expand_backward(eq: &impl Fn(usize, usize) -> bool, s1: usize, s2: usize, mut e1: usize, mut e2: usize) -> usize {
    let end = e1;
    while s1 < e1 && s2 < e2 && eq(e1 - 1, e2 - 1) {
        e1 -= 1;
        e2 -= 1;
    }
    end - e1
}

fn eq_of<'k>(a: &'k Keys, b: &'k Keys) -> impl Fn(usize, usize) -> bool + 'k {
    move |i, j| a.ids[i] == b.ids[j]
}

// ---------------------------------------------------------------- compareSmart

/// Compare ignoring "unimportant" lines, then match the gaps between matched big lines.
fn compare_smart(l1: &Keys, l2: &Keys, tokens: u32) -> Vec<R> {
    let big = |k: &Keys| -> (Vec<u32>, Vec<usize>) {
        let idx: Vec<usize> = (0..k.ids.len()).filter(|&i| k.non_space[i] > UNIMPORTANT_LINE_CHAR_COUNT).collect();
        (idx.iter().map(|&i| k.ids[i]).collect(), idx)
    };
    let (big1, idx1) = big(l1);
    let (big2, idx2) = big(l2);
    let changes = diff(&big1, &big2, tokens);

    // SmartLineChangeCorrector
    let (len1, len2) = (l1.ids.len(), l2.ids.len());
    let eq = eq_of(l1, l2);
    let mut b = ChangeBuilder::new(len1, len2);
    let match_gap = |b: &mut ChangeBuilder, s1: usize, e1: usize, s2: usize, e2: usize| {
        let ex = expand(&eq, s1, s2, e1, e2);
        let inner = diff(&l1.ids[ex.s1..ex.e1], &l2.ids[ex.s2..ex.e2], tokens);
        b.mark_equal(s1, s2, ex.s1, ex.s2, |_, _| false);
        for u in unchanged(&inner, ex.e1 - ex.s1, ex.e2 - ex.s2) {
            b.mark_equal(ex.s1 + u.s1, ex.s2 + u.s2, ex.s1 + u.e1, ex.s2 + u.e2, |_, _| false);
        }
        b.mark_equal(ex.e1, ex.e2, e1, e2, |_, _| false);
    };
    let (mut last1, mut last2) = (0, 0);
    for u in unchanged(&changes, big1.len(), big2.len()) {
        for i in 0..(u.e1 - u.s1) {
            let (start1, start2) = (idx1[u.s1 + i], idx2[u.s2 + i]);
            match_gap(&mut b, last1, start1, last2, start2);
            b.mark_equal(start1, start2, start1 + 1, start2 + 1, |_, _| false);
            last1 = start1 + 1;
            last2 = start2 + 1;
        }
    }
    match_gap(&mut b, last1, len1, last2, len2);
    b.finish(|_, _| false)
}

// ---------------------------------------------------------------- ChunkOptimizer

/// Merge touching chunks and slide one-sided chunks towards empty/unimportant lines (LineChunkOptimizer).
fn optimize_line_chunks(l1: &Keys, l2: &Keys, changes: &[R]) -> Vec<R> {
    let shift = |touch_left: bool, fwd: usize, bwd: usize, r1: &R, r2: &R| line_shift(touch_left, fwd, bwd, r1, r2, l1, l2);
    optimize_chunks(changes, l1.ids.len(), l2.ids.len(), &eq_of(l1, l2), &shift)
}

/// ChunkOptimizer.build: merge touching chunks, else slide them by `shift(touch_left, equal_forward, equal_backward, r1, r2)`.
pub(crate) fn optimize_chunks(
    changes: &[R],
    len1: usize,
    len2: usize,
    eq: &impl Fn(usize, usize) -> bool,
    shift: &impl Fn(bool, usize, usize, &R, &R) -> isize,
) -> Vec<R> {
    let mut ranges: Vec<R> = Vec::new();
    for u in unchanged(changes, len1, len2) {
        ranges.push(u);
        process_last_ranges(&mut ranges, eq, shift);
    }
    changes_from_unchanged(&ranges, len1, len2)
}

fn process_last_ranges(ranges: &mut Vec<R>, eq: &impl Fn(usize, usize) -> bool, shift: &impl Fn(bool, usize, usize, &R, &R) -> isize) {
    if ranges.len() < 2 {
        return;
    }
    let r1 = ranges[ranges.len() - 2];
    let r2 = ranges[ranges.len() - 1];
    if r1.e1 != r2.s1 && r1.e2 != r2.s2 {
        return;
    }
    let count1 = r1.e1 - r1.s1;
    let count2 = r2.e1 - r2.s1;
    let equal_forward = expand_forward(eq, r1.e1, r1.e2, r1.e1 + count2, r1.e2 + count2);
    let equal_backward = expand_backward(eq, r2.s1 - count1, r2.s2 - count1, r2.s1, r2.s2);
    if equal_forward == 0 && equal_backward == 0 {
        return;
    }
    // [A]B[B] -> [AB]B
    if equal_forward == count2 {
        ranges.truncate(ranges.len() - 2);
        ranges.push(R { s1: r1.s1, e1: r1.e1 + count2, s2: r1.s2, e2: r1.e2 + count2 });
        process_last_ranges(ranges, eq, shift);
        return;
    }
    // [A]A[B] -> A[AB]
    if equal_backward == count1 {
        ranges.truncate(ranges.len() - 2);
        ranges.push(R { s1: r2.s1 - count1, e1: r2.e1, s2: r2.s2 - count1, e2: r2.e2 });
        process_last_ranges(ranges, eq, shift);
        return;
    }
    let shift = shift(r1.e1 == r2.s1, equal_forward, equal_backward, &r1, &r2);
    if shift != 0 {
        ranges.truncate(ranges.len() - 2);
        let s = |x: usize| (x as isize + shift) as usize;
        ranges.push(R { s1: r1.s1, e1: s(r1.e1), s2: r1.s2, e2: s(r1.e2) });
        ranges.push(R { s1: s(r2.s1), e1: r2.e1, s2: s(r2.s2), e2: r2.e2 });
    }
}

fn line_shift(touch_left: bool, fwd: usize, bwd: usize, r1: &R, r2: &R, l1: &Keys, l2: &Keys) -> isize {
    let (touch, other) = if touch_left { (l1, l2) } else { (l2, l1) };
    let touch_start = if touch_left { r2.s1 } else { r2.s2 };
    let change_start = if touch_left { r1.e2 } else { r1.e1 };
    let change_end = if touch_left { r2.s2 } else { r2.s1 };
    for threshold in [0, UNIMPORTANT_LINE_CHAR_COUNT] {
        // empty line boundary in unchanged lines: insertion goes right before/after it
        let f = next_unimportant(touch, touch_start, fwd + 1, threshold);
        let b = prev_unimportant(touch, touch_start as isize - 1, bwd + 1, threshold);
        if let Some(s) = pick_shift(f, b) {
            return s;
        }
        // empty line boundary in changed lines: insertion starts/ends with it
        let f = next_unimportant(other, change_start, fwd + 1, threshold);
        let b = prev_unimportant(other, change_end as isize - 1, bwd + 1, threshold);
        if let Some(s) = pick_shift(f, b) {
            return s;
        }
    }
    0
}

fn next_unimportant(lines: &Keys, offset: usize, count: usize, threshold: usize) -> Option<usize> {
    (0..count).find(|&i| lines.non_space[offset + i] <= threshold)
}

fn prev_unimportant(lines: &Keys, offset: isize, count: usize, threshold: usize) -> Option<usize> {
    (0..count).find(|&i| lines.non_space[(offset - i as isize) as usize] <= threshold)
}

fn pick_shift(forward: Option<usize>, backward: Option<usize>) -> Option<isize> {
    match (forward, backward) {
        (None, None) => None,
        (Some(0), _) | (_, Some(0)) => Some(0),
        (Some(f), _) => Some(f as isize),
        (None, Some(b)) => Some(-(b as isize)),
    }
}

// ---------------------------------------------------------------- policy correction

fn expand_ranges(l1: &Keys, l2: &Keys, changes: &[R], _len1: usize, _len2: usize) -> Vec<R> {
    let eq = eq_of(l1, l2);
    changes.iter().map(|c| expand(&eq, c.s1, c.s2, c.e1, c.e2)).filter(|r| !r.is_empty()).collect()
}

/// Re-apply the user's policy inside runs matched while ignoring whitespace,
/// pairing lines to maximise exactly-equal pairs (ByLineRt.correctChangesSecondStep).
fn correct_changes_second_step(p1: &Keys, p2: &Keys, iw1: &Keys, iw2: &Keys, iw_changes: &[R]) -> Vec<R> {
    let (len1, len2) = (p1.ids.len(), p2.ids.len());
    let eq = eq_of(p1, p2);
    let mut b = ChangeBuilder::new(len1, len2);
    let mut sample: Option<u32> = None; // IW key of the current sample line
    let (mut last1, mut last2) = (0usize, 0usize);

    let flush = |b: &mut ChangeBuilder, sample: &mut Option<u32>, last1: &mut usize, last2: &mut usize, line1: usize, line2: usize| {
        let Some(s) = sample.take() else { return };
        let start1 = (*last1).max(b.index1);
        let start2 = (*last2).max(b.index2);
        let mut sub1 = Vec::new();
        let mut sub2 = Vec::new();
        for i in start1..line1 {
            if iw1.ids[i] == s {
                sub1.push(i);
                *last1 = i + 1;
            }
        }
        for i in start2..line2 {
            if iw2.ids[i] == s {
                sub2.push(i);
                *last2 = i + 1;
            }
        }
        debug_assert!(!sub1.is_empty() && !sub2.is_empty());
        align_exact_matching(b, &sub1, &sub2, p1, p2, &eq);
    };

    for u in unchanged(iw_changes, len1, len2) {
        for i in 0..(u.e1 - u.s1) {
            let (i1, i2) = (u.s1 + i, u.s2 + i);
            if sample != Some(iw1.ids[i1]) {
                flush(&mut b, &mut sample, &mut last1, &mut last2, i1, i2);
                if p1.ids[i1] == p2.ids[i2] {
                    b.mark_equal(i1, i2, i1 + 1, i2 + 1, &eq);
                } else {
                    sample = Some(iw1.ids[i1]);
                }
            }
        }
    }
    flush(&mut b, &mut sample, &mut last1, &mut last2, len1, len2);
    b.finish(&eq)
}

fn align_exact_matching(b: &mut ChangeBuilder, sub1: &[usize], sub2: &[usize], p1: &Keys, p2: &Keys, eq: &impl Fn(usize, usize) -> bool) {
    let n = sub1.len().max(sub2.len());
    // brute force over C(n, k) is limited to n <= 10
    if n > 10 || sub1.len() == sub2.len() {
        for i in 0..sub1.len().min(sub2.len()) {
            if p1.ids[sub1[i]] == p2.ids[sub2[i]] {
                b.mark_equal(sub1[i], sub2[i], sub1[i] + 1, sub2[i] + 1, eq);
            }
        }
        return;
    }
    if sub1.len() < sub2.len() {
        let m = best_matching(sub1, sub2, |a, c| p1.ids[a] == p2.ids[c]);
        for (i, &j) in m.iter().enumerate() {
            let (i1, i2) = (sub1[i], sub2[j]);
            if p1.ids[i1] == p2.ids[i2] {
                b.mark_equal(i1, i2, i1 + 1, i2 + 1, eq);
            }
        }
    } else {
        let m = best_matching(sub2, sub1, |a, c| p2.ids[a] == p1.ids[c]);
        for (i, &j) in m.iter().enumerate() {
            let (i1, i2) = (sub1[j], sub2[i]);
            if p1.ids[i1] == p2.ids[i2] {
                b.mark_equal(i1, i2, i1 + 1, i2 + 1, eq);
            }
        }
    }
}

/// For each element of the shorter list, an increasing position in the longer one,
/// maximising the number of equal pairs (first maximum wins).
fn best_matching(short: &[usize], long: &[usize], equal: impl Fn(usize, usize) -> bool) -> Vec<usize> {
    let k = short.len();
    let mut best: Vec<usize> = (0..k).collect();
    let mut best_weight = 0;
    let mut comb = vec![0usize; k];
    fn rec(start: usize, n: usize, depth: usize, comb: &mut Vec<usize>, visit: &mut dyn FnMut(&[usize])) {
        if depth == comb.len() {
            visit(comb);
            return;
        }
        for i in start..n {
            comb[depth] = i;
            rec(i + 1, n, depth + 1, comb, visit);
        }
    }
    rec(0, long.len(), 0, &mut comb, &mut |c: &[usize]| {
        let w = (0..k).filter(|&i| equal(short[i], long[c[i]])).count();
        if w > best_weight {
            best_weight = w;
            best.copy_from_slice(c);
        }
    });
    best
}
