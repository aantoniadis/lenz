// Ported from IntelliJ IDEA Community Edition (commit bc34875):
//   com.intellij.diff.comparison.ByCharRt, ChangeCorrector.DefaultCharChangeCorrector.
// Copyright 2000-2024 JetBrains s.r.o. and contributors. Licensed under the Apache License, Version 2.0.
// Modified: translated from Kotlin to Rust; offsets are UTF-8 byte offsets instead of UTF-16 indices (every
// offset produced lies on a char boundary); code points are interned before the LCS step, which uses
// gix-imara-diff's Myers; punctuation matching returns matched offset pairs instead of a DiffIterable.

use std::collections::HashMap;

use crate::by_line::{ChangeBuilder, R, is_ws, unchanged};
use crate::lcs::diff;
use crate::by_word::{expand_ws_forward, trim_spaces};
use crate::chars::is_punctuation;

/// Selected code points of a text: interned ids and their byte offsets.
struct CodePoints {
    ids: Vec<u32>,
    offsets: Vec<usize>,
    lens: Vec<usize>,
}

impl CodePoints {
    fn new(text: &str, keep: impl Fn(char) -> bool, table: &mut HashMap<char, u32>) -> Self {
        let mut cp = CodePoints { ids: Vec::new(), offsets: Vec::new(), lens: Vec::new() };
        for (offset, c) in text.char_indices().filter(|&(_, c)| keep(c)) {
            let n = table.len() as u32;
            cp.ids.push(*table.entry(c).or_insert(n));
            cp.offsets.push(offset);
            cp.lens.push(c.len_utf8());
        }
        cp
    }

    fn after(&self, index: usize) -> usize {
        self.offsets[index] + self.lens[index]
    }
}

/// Plain LCS over all code points (ByCharRt.compare).
pub fn compare(text1: &str, text2: &str) -> Vec<R> {
    let mut table = HashMap::new();
    let c1 = CodePoints::new(text1, |_| true, &mut table);
    let c2 = CodePoints::new(text2, |_| true, &mut table);
    let at = |c: &CodePoints, len: usize, i: usize| if i == c.offsets.len() { len } else { c.offsets[i] };
    diff(&c1.ids, &c2.ids)
        .iter()
        .map(|ch| R {
            s1: at(&c1, text1.len(), ch.s1),
            e1: at(&c1, text1.len(), ch.e1),
            s2: at(&c2, text2.len(), ch.s2),
            e2: at(&c2, text2.len(), ch.e2),
        })
        .collect()
}

/// Match non-space code points first, then the gaps between them (ByCharRt.compareTwoStep).
pub fn compare_two_step(text1: &str, text2: &str) -> Vec<R> {
    let mut table = HashMap::new();
    let c1 = CodePoints::new(text1, |c| !is_ws(c), &mut table);
    let c2 = CodePoints::new(text2, |c| !is_ws(c), &mut table);
    let changes = diff(&c1.ids, &c2.ids);

    // DefaultCharChangeCorrector
    let mut b = ChangeBuilder::new(text1.len(), text2.len());
    let match_gap = |b: &mut ChangeBuilder, s1: usize, e1: usize, s2: usize, e2: usize| {
        let inner = compare(&text1[s1..e1], &text2[s2..e2]);
        for u in unchanged(&inner, e1 - s1, e2 - s2) {
            b.mark_equal(s1 + u.s1, s2 + u.s2, s1 + u.e1, s2 + u.e2, |_, _| false);
        }
    };
    let (mut last1, mut last2) = (0, 0);
    for u in unchanged(&changes, c1.ids.len(), c2.ids.len()) {
        for i in 0..u.e1 - u.s1 {
            let (i1, i2) = (u.s1 + i, u.s2 + i);
            match_gap(&mut b, last1, c1.offsets[i1], last2, c2.offsets[i2]);
            b.mark_equal(c1.offsets[i1], c2.offsets[i2], c1.after(i1), c2.after(i2), |_, _| false);
            last1 = c1.after(i1);
            last2 = c2.after(i2);
        }
    }
    match_gap(&mut b, last1, text1.len(), last2, text2.len());
    b.finish(|_, _| false)
}

/// ByCharRt.compareTrimWhitespaces.
pub fn compare_trim_whitespaces(text1: &str, text2: &str) -> Vec<R> {
    trim_spaces(&compare_two_step(text1, text2), text1, text2)
}

/// Whitespace is ignored while matching; adjoining whitespace counts as matched (ByCharRt.compareIgnoreWhitespaces).
pub fn compare_ignore_whitespaces(text1: &str, text2: &str) -> Vec<R> {
    let mut table = HashMap::new();
    let c1 = CodePoints::new(text1, |c| !is_ws(c), &mut table);
    let c2 = CodePoints::new(text2, |c| !is_ws(c), &mut table);
    let (t1, t2) = (text1.as_bytes(), text2.as_bytes());
    // match whitespace at an insertion point if we can: "x y" -> "x zy"
    let expand_forward = |ch: &R, left: bool| {
        let o1 = if ch.s1 == 0 { 0 } else { c1.after(ch.s1 - 1) };
        let o2 = if ch.s2 == 0 { 0 } else { c2.after(ch.s2 - 1) };
        (if left { o1 } else { o2 }) + expand_ws_forward(t1, t2, o1, o2, t1.len(), t2.len())
    };
    diff(&c1.ids, &c2.ids)
        .iter()
        .map(|ch| {
            let (s1, e1) = if ch.s1 == ch.e1 {
                let o = expand_forward(ch, true);
                (o, o)
            } else {
                (c1.offsets[ch.s1], c1.after(ch.e1 - 1))
            };
            let (s2, e2) = if ch.s2 == ch.e2 {
                let o = expand_forward(ch, false);
                (o, o)
            } else {
                (c2.offsets[ch.s2], c2.after(ch.e2 - 1))
            };
            R { s1, e1, s2, e2 }
        })
        .collect()
}

/// Punctuation chars of the given texts taken as one sequence, with byte offsets into that sequence.
pub fn punctuation(parts: &[&str]) -> Vec<(u8, usize)> {
    let mut out = Vec::new();
    let mut shift = 0;
    for part in parts {
        out.extend(part.bytes().enumerate().filter(|&(_, b)| is_punctuation(b as char)).map(|(i, b)| (b, shift + i)));
        shift += part.len();
    }
    out
}

/// Offsets of matched punctuation chars; everything else stays unmatched (ByCharRt.comparePunctuation).
pub fn match_punctuation(p1: &[(u8, usize)], p2: &[(u8, usize)]) -> Vec<(usize, usize)> {
    let ids = |p: &[(u8, usize)]| p.iter().map(|&(b, _)| b as u32).collect::<Vec<u32>>();
    let changes = diff(&ids(p1), &ids(p2));
    unchanged(&changes, p1.len(), p2.len())
        .iter()
        .flat_map(|u| (0..u.e1 - u.s1).map(move |i| (p1[u.s1 + i].1, p2[u.s2 + i].1)))
        .collect()
}
