// Ported from IntelliJ IDEA Community Edition (commit bc34875):
//   com.intellij.diff.comparison.ByWordRt (compare for two and three texts, compareAndSplit, getInlineChunks,
//   AdjustmentPunctuationMatcher, DefaultCorrector, TrimSpacesCorrector, IgnoreSpacesCorrector, MergeDefaultCorrector,
//   MergeTrimSpacesCorrector, MergeIgnoreSpacesCorrector), LineFragmentSplitter, ChunkOptimizer.WordChunkOptimizer,
//   iterables.SubiterableDiffIterable, TrimUtil (trim*, expandWhitespaces*, isEquals*).
// Copyright 2000-2024 JetBrains s.r.o. and contributors. Licensed under the Apache License, Version 2.0.
// Modified: translated from Kotlin to Rust; offsets are UTF-8 byte offsets instead of UTF-16 indices (every
// offset produced lies on a char boundary); words are interned by content before the LCS step, which uses
// gix-imara-diff's Myers; the three-way comparison interns base with each other text separately; "words first" mode
// and cancellation are not ported.

use std::collections::HashMap;

use crate::by_char::{match_punctuation, punctuation};
use crate::by_line::{ChangeBuilder, Policy, R, is_ws, optimize_chunks, unchanged};
use crate::lcs::diff;
use crate::chars::{equals_ignore_ws, is_alpha, is_continuous_script};
use crate::merge::{R3, equal_texts, zip};

const NEWLINE: u32 = 0;

/// A word, a single continuous-script code point, or a newline (ByWordRt.InlineChunk).
#[derive(Clone, Copy, Debug)]
pub struct Chunk {
    start: usize,
    end: usize,
    id: u32,
}

impl Chunk {
    fn is_newline(&self) -> bool {
        self.id == NEWLINE
    }
}

/// One line-aligned part of a modified block (ByWordRt.LineBlock): `fragments` are relative to `offsets`.
pub struct LineBlock {
    pub fragments: Vec<R>,
    pub offsets: R,
    pub newlines1: usize,
    pub newlines2: usize,
}

/// Word-level changes over the whole texts (ByWordRt.compare).
pub fn compare(text1: &str, text2: &str, policy: Policy) -> Vec<R> {
    let (w1, w2) = inline_chunks(text1, text2);
    let changes = diff(&ids(&w1), &ids(&w2));
    let changes = optimize_word_chunks(text1, text2, &w1, &w2, &changes);
    let changes = match_adjustment_delimiters(text1, text2, &w1, &w2, &changes, 0, 0);
    match_adjustment_whitespaces(text1, text2, &changes, policy)
}

/// Word-level merge ranges between left, base and right (ByWordRt.compare for three texts): base→left and base→right
/// word diffs zipped on base, then whitespace corrected per `policy`.
pub fn compare3(left: &str, base: &str, right: &str, policy: Policy) -> Vec<R3> {
    let from_base = |text: &str| {
        let (wb, wt) = inline_chunks(base, text);
        let changes = diff(&ids(&wb), &ids(&wt));
        let changes = optimize_word_chunks(base, text, &wb, &wt, &changes);
        let changes = match_adjustment_delimiters(base, text, &wb, &wt, &changes, 0, 0);
        unchanged(&changes, base.len(), text.len())
    };
    let ranges = zip(&from_base(left), &from_base(right), [left.len(), base.len(), right.len()], None);
    let texts = [left.as_bytes(), base.as_bytes(), right.as_bytes()];
    match policy {
        Policy::Default => merge_default_spaces(&ranges, texts),
        Policy::Trim => merge_trim_spaces(&merge_default_spaces(&ranges, texts), [left, base, right]),
        Policy::Ignore => merge_ignore_spaces(&ranges, [left, base, right]),
    }
}

/// Word-level changes, split into line-aligned blocks (ByWordRt.compareAndSplit).
pub fn compare_and_split(text1: &str, text2: &str, policy: Policy) -> Vec<LineBlock> {
    let (w1, w2) = inline_chunks(text1, text2);
    let changes = diff(&ids(&w1), &ids(&w2));
    let changes = optimize_word_chunks(text1, text2, &w1, &w2, &changes);
    let word_blocks = split_by_lines(text1, text2, &w1, &w2, &changes);

    let mut index = 0;
    word_blocks
        .iter()
        .map(|block| {
            let (words, o) = (block.words, block.offsets);
            // collectWordBlockSubIterables
            while index < changes.len() && (changes[index].e1 < words.s1 || changes[index].e2 < words.s2) {
                index += 1;
            }
            let sub = sub_changes(&changes, words, index);
            let (sub1, sub2) = (&text1[o.s1..o.e1], &text2[o.s2..o.e2]);
            let (subw1, subw2) = (&w1[words.s1..words.e1], &w2[words.s2..words.e2]);
            let delimiters = match_adjustment_delimiters(sub1, sub2, subw1, subw2, &sub, o.s1, o.s2);
            let newlines = |w: &[Chunk]| w.iter().filter(|c| c.is_newline()).count();
            LineBlock {
                fragments: match_adjustment_whitespaces(sub1, sub2, &delimiters, policy),
                offsets: o,
                newlines1: newlines(subw1),
                newlines2: newlines(subw2),
            }
        })
        .collect()
}

// ---------------------------------------------------------------- tokens

/// ByWordRt.getInlineChunks for both texts, words interned by content (newline is id 0).
fn inline_chunks<'a>(text1: &'a str, text2: &'a str) -> (Vec<Chunk>, Vec<Chunk>) {
    let mut table: HashMap<&'a str, u32> = HashMap::new();
    table.insert("\n", NEWLINE);
    let mut chunks = |text: &'a str| {
        let mut out = Vec::new();
        chunk_bounds(text, |start, end| {
            let n = table.len() as u32;
            out.push(Chunk { start, end, id: *table.entry(&text[start..end]).or_insert(n) });
        });
        out
    };
    let w1 = chunks(text1);
    let w2 = chunks(text2);
    (w1, w2)
}

/// How many chunks [`inline_chunks`] makes of a text, without making them.
pub fn count(text: &str) -> usize {
    let mut n = 0;
    chunk_bounds(text, |_, _| n += 1);
    n
}

/// Each word, continuous-script code point and newline of a text, in order.
fn chunk_bounds(text: &str, mut chunk: impl FnMut(usize, usize)) {
    let mut word_start = None;
    for (offset, c) in text.char_indices() {
        let alpha = is_alpha(c);
        if alpha && !is_continuous_script(c) {
            word_start.get_or_insert(offset);
            continue;
        }
        if let Some(start) = word_start.take() {
            chunk(start, offset);
        }
        if alpha || c == '\n' {
            chunk(offset, offset + c.len_utf8());
        }
    }
    if let Some(start) = word_start {
        chunk(start, text.len());
    }
}

fn ids(words: &[Chunk]) -> Vec<u32> {
    words.iter().map(|w| w.id).collect()
}

// ---------------------------------------------------------------- WordChunkOptimizer

/// Minimise the number of chunks, then the number of modified "sentences" (words not separated by whitespace):
/// "[AX] [AZ]" - "[AX] AY [AZ]" rather than "[AX A][Z]" - "[AX A]Y A[Z]".
fn optimize_word_chunks(text1: &str, text2: &str, w1: &[Chunk], w2: &[Chunk], changes: &[R]) -> Vec<R> {
    let eq = |i: usize, j: usize| w1[i].id == w2[j].id;
    let shift = |touch_left: bool, forward: usize, backward: usize, _: &R, r2: &R| -> isize {
        let (text, words, start) = if touch_left { (text1, w1, r2.s1) } else { (text2, w2, r2.s2) };
        if separated(text, &words[start - 1], &words[start]) {
            return 0;
        }
        if let Some(s) = sequence_edge_shift(text, words, start, forward, true) {
            return s as isize;
        }
        if let Some(s) = sequence_edge_shift(text, words, start - 1, backward, false) {
            return -(s as isize);
        }
        0
    };
    optimize_chunks(changes, w1.len(), w2.len(), &eq, &shift)
}

fn sequence_edge_shift(text: &str, words: &[Chunk], offset: usize, count: usize, left_to_right: bool) -> Option<usize> {
    (0..count).find(|&i| {
        let (a, b) = if left_to_right { (offset + i, offset + i + 1) } else { (offset - i - 1, offset - i) };
        separated(text, &words[a], &words[b])
    }).map(|i| i + 1)
}

fn separated(text: &str, a: &Chunk, b: &Chunk) -> bool {
    a.is_newline() || b.is_newline() || text.as_bytes()[a.end..b.start].iter().any(|&c| is_ws(c as char))
}

// ---------------------------------------------------------------- LineFragmentSplitter

#[derive(Clone, Copy)]
struct WordBlock {
    words: R,
    offsets: R,
}

struct PendingChunk {
    block: WordBlock,
    has_equal_words: bool,
    has_words_inside: bool,
    equal_ignore_ws: bool,
}

/// Given matched words, split a line block into "logically different" line blocks (LineFragmentSplitter).
/// Word indices are offset by one here: 0 is the start of text, `len + 1` its end.
fn split_by_lines(text1: &str, text2: &str, w1: &[Chunk], w2: &[Chunk], changes: &[R]) -> Vec<WordBlock> {
    let mut result = Vec::new();
    let mut pending: Option<PendingChunk> = None;
    let (mut last1, mut last2) = (0usize, 0usize);

    let offset = |words: &[Chunk], text: &str, index: usize| match index {
        0 => 0,
        i if i == words.len() + 1 => text.len(),
        i => {
            debug_assert!(words[i - 1].is_newline());
            words[i - 1].end
        }
    };
    let mut add = |end1: usize, end2: usize, has_equal_words: bool| {
        if last1 > end1 || last2 > end2 {
            return;
        }
        let offsets = R {
            s1: offset(w1, text1, last1),
            e1: offset(w1, text1, end1),
            s2: offset(w2, text2, last2),
            e2: offset(w2, text2, end2),
        };
        if offsets.is_empty() {
            return;
        }
        let words = R { s1: last1, e1: end1.min(w1.len()), s2: last2, e2: end2.min(w2.len()) };
        let has_words_inside =
            w1[words.s1..words.e1].iter().chain(&w2[words.s2..words.e2]).any(|w| !w.is_newline());
        let equal_ignore_ws = equals_ignore_ws(&text1[offsets.s1..offsets.e1], &text2[offsets.s2..offsets.e2]);
        let chunk = PendingChunk { block: WordBlock { words, offsets }, has_equal_words, has_words_inside, equal_ignore_ws };
        pending = Some(match pending.take() {
            Some(p) if should_merge(&p, &chunk) => merge(p, chunk),
            Some(p) => {
                result.push(p.block);
                chunk
            }
            None => chunk,
        });
        last1 = end1;
        last2 = end2;
    };

    let first_in_line = |words: &[Chunk], i: usize| i == 0 || words[i - 1].is_newline();
    let mut has_equal_words = false;
    for u in unchanged(changes, w1.len(), w2.len()) {
        for i in 0..u.e1 - u.s1 {
            let (i1, i2) = (u.s1 + i, u.s2 + i);
            if w1[i1].is_newline() && w2[i2].is_newline() {
                // split by matched newlines
                add(i1 + 1, i2 + 1, has_equal_words);
                has_equal_words = false;
            } else {
                // split by matched first word in line
                if first_in_line(w1, i1) && first_in_line(w2, i2) {
                    add(i1, i2, has_equal_words);
                }
                has_equal_words = true;
            }
        }
    }
    add(w1.len() + 1, w2.len() + 1, has_equal_words);
    if let Some(p) = pending {
        result.push(p.block);
    }
    result
}

fn should_merge(a: &PendingChunk, b: &PendingChunk) -> bool {
    // lines matched only by '\n', whitespace-only changed lines, a block without words
    (!a.has_equal_words && !b.has_equal_words)
        || (a.equal_ignore_ws && b.equal_ignore_ws)
        || !a.has_words_inside
        || !b.has_words_inside
}

fn merge(a: PendingChunk, b: PendingChunk) -> PendingChunk {
    let (x, y) = (a.block, b.block);
    PendingChunk {
        block: WordBlock {
            words: R { s1: x.words.s1, e1: y.words.e1, s2: x.words.s2, e2: y.words.e2 },
            offsets: R { s1: x.offsets.s1, e1: y.offsets.e1, s2: x.offsets.s2, e2: y.offsets.e2 },
        },
        has_equal_words: a.has_equal_words || b.has_equal_words,
        has_words_inside: a.has_words_inside || b.has_words_inside,
        equal_ignore_ws: a.equal_ignore_ws && b.equal_ignore_ws,
    }
}

/// `changes` clipped to the word range `w` and shifted to start at it (SubiterableDiffIterable).
fn sub_changes(changes: &[R], w: R, first: usize) -> Vec<R> {
    let mut out = Vec::new();
    for r in &changes[first..] {
        if r.e1 < w.s1 || r.e2 < w.s2 {
            continue;
        }
        if r.s1 > w.e1 || r.s2 > w.e2 {
            break;
        }
        let n = R {
            s1: r.s1.max(w.s1) - w.s1,
            e1: r.e1.min(w.e1) - w.s1,
            s2: r.s2.max(w.s2) - w.s2,
            e2: r.e2.min(w.e2) - w.s2,
        };
        if !n.is_empty() {
            out.push(n);
        }
    }
    out
}

// ---------------------------------------------------------------- AdjustmentPunctuationMatcher

/// Match punctuation between matched words: "[ X { A ! B } Y ]" - "( X ... Y )" compares "[" - "(",
/// "{" + "}" - "...", "]" - ")". Words are given with offsets `shift1`/`shift2` ahead of `text1`/`text2`.
fn match_adjustment_delimiters(
    text1: &str,
    text2: &str,
    w1: &[Chunk],
    w2: &[Chunk],
    changes: &[R],
    shift1: usize,
    shift2: usize,
) -> Vec<R> {
    let mut m = PunctuationMatcher { text1, text2, w1, w2, shift1, shift2, builder: ChangeBuilder::new(text1.len(), text2.len()), last: None };
    m.match_forward(None, None);
    for u in unchanged(changes, w1.len(), w2.len()) {
        for i in 0..u.e1 - u.s1 {
            let (i1, i2) = (u.s1 + i, u.s2 + i);
            m.match_backward(i1, i2);
            let (s1, e1, s2, e2) = (m.start1(i1), m.end1(i1), m.start2(i2), m.end2(i2));
            m.builder.mark_equal(s1, s2, e1, e2, |_, _| false);
            m.match_forward(Some(i1), Some(i2));
        }
    }
    m.match_backward(w1.len(), w2.len());
    m.builder.finish(|_, _| false)
}

struct PunctuationMatcher<'a> {
    text1: &'a str,
    text2: &'a str,
    w1: &'a [Chunk],
    w2: &'a [Chunk],
    shift1: usize,
    shift2: usize,
    builder: ChangeBuilder,
    /// The gap after the last matched words: (start1, start2, end1, end2).
    last: Option<(usize, usize, usize, usize)>,
}

impl PunctuationMatcher<'_> {
    fn start1(&self, i: usize) -> usize {
        self.w1[i].start - self.shift1
    }
    fn start2(&self, i: usize) -> usize {
        self.w2[i].start - self.shift2
    }
    fn end1(&self, i: usize) -> usize {
        self.w1[i].end - self.shift1
    }
    fn end2(&self, i: usize) -> usize {
        self.w2[i].end - self.shift2
    }

    fn match_forward(&mut self, i1: Option<usize>, i2: Option<usize>) {
        let s1 = i1.map_or(0, |i| self.end1(i));
        let s2 = i2.map_or(0, |i| self.end2(i));
        let n1 = i1.map_or(0, |i| i + 1);
        let n2 = i2.map_or(0, |i| i + 1);
        let e1 = if n1 == self.w1.len() { self.text1.len() } else { self.start1(n1) };
        let e2 = if n2 == self.w2.len() { self.text2.len() } else { self.start2(n2) };
        debug_assert!(self.last.is_none());
        self.last = Some((s1, s2, e1, e2));
    }

    fn match_backward(&mut self, i1: usize, i2: usize) {
        let s1 = if i1 == 0 { 0 } else { self.end1(i1 - 1) };
        let s2 = if i2 == 0 { 0 } else { self.end2(i2 - 1) };
        let e1 = if i1 == self.w1.len() { self.text1.len() } else { self.start1(i1) };
        let e2 = if i2 == self.w2.len() { self.text2.len() } else { self.start2(i2) };
        let (ls1, ls2, le1, le2) = self.last.take().expect("match_forward before match_backward");
        if ls1 == s1 && ls2 == s2 {
            // adjoining matched words, match the gap between: "A B" - "A B"
            self.match_range(s1, s2, e1, e2);
        } else if ls1 < s1 && ls2 < s2 {
            // matched words with unmatched ones between: "A X B" - "A Y B"
            self.match_range(ls1, ls2, le1, le2);
            self.match_range(s1, s2, e1, e2);
        } else if ls1 == s1 && le1 == e1 {
            // one side adjoining, the other has unmatched words between: "A B" - "A Y B"
            self.match_two_gaps(true, s1, e1, (ls2, le2), (s2, e2));
        } else if ls2 == s2 && le2 == e2 {
            self.match_two_gaps(false, s2, e2, (ls1, le1), (s1, e1));
        } else {
            unreachable!("punctuation gaps out of order");
        }
    }

    fn match_range(&mut self, s1: usize, s2: usize, e1: usize, e2: usize) {
        if s1 == e1 && s2 == e2 {
            return;
        }
        let p1 = punctuation(&[&self.text1[s1..e1]]);
        let p2 = punctuation(&[&self.text2[s2..e2]]);
        for (o1, o2) in match_punctuation(&p1, &p2) {
            self.builder.mark_equal(s1 + o1, s2 + o2, s1 + o1 + 1, s2 + o2 + 1, |_, _| false);
        }
    }

    /// One gap on side 1 (`left`) or side 2 against two gaps on the other, compared as if they were one
    /// (comparePunctuation2Side via matchComplexRangeLeft / matchComplexRangeRight).
    fn match_two_gaps(&mut self, left: bool, s: usize, e: usize, gap_a: (usize, usize), gap_b: (usize, usize)) {
        let (single, double) = if left { (self.text1, self.text2) } else { (self.text2, self.text1) };
        let len_a = gap_a.1 - gap_a.0;
        let p = punctuation(&[&single[s..e]]);
        let pp = punctuation(&[&double[gap_a.0..gap_a.1], &double[gap_b.0..gap_b.1]]);
        for (o, oo) in match_punctuation(&p, &pp) {
            let other = if oo < len_a { gap_a.0 + oo } else { gap_b.0 + oo - len_a };
            let (x1, x2) = if left { (s + o, other) } else { (other, s + o) };
            self.builder.mark_equal(x1, x2, x1 + 1, x2 + 1, |_, _| false);
        }
    }
}

// ---------------------------------------------------------------- whitespace correctors

fn match_adjustment_whitespaces(text1: &str, text2: &str, changes: &[R], policy: Policy) -> Vec<R> {
    match policy {
        Policy::Default => default_spaces(changes, text1, text2),
        Policy::Trim => trim_spaces(&default_spaces(changes, text1, text2), text1, text2),
        Policy::Ignore => ignore_spaces(changes, text1, text2),
    }
}

/// Shrink changes by whitespace that matches on both sides (DefaultCorrector).
fn default_spaces(changes: &[R], text1: &str, text2: &str) -> Vec<R> {
    let (t1, t2) = (text1.as_bytes(), text2.as_bytes());
    changes
        .iter()
        .filter_map(|r| {
            let end = expand_ws_backward(t1, t2, r.s1, r.s2, r.e1, r.e2);
            let start = expand_ws_forward(t1, t2, r.s1, r.s2, r.e1 - end, r.e2 - end);
            let x = R { s1: r.s1 + start, e1: r.e1 - end, s2: r.s2 + start, e2: r.e2 - end };
            (!x.is_empty()).then_some(x)
        })
        .collect()
}

/// Drop leading/trailing whitespace of lines from changes (TrimSpacesCorrector).
pub fn trim_spaces(changes: &[R], text1: &str, text2: &str) -> Vec<R> {
    let (t1, t2) = (text1.as_bytes(), text2.as_bytes());
    changes
        .iter()
        .filter_map(|r| {
            let (mut s1, mut e1, mut s2, mut e2) = (r.s1, r.e1, r.s2, r.e2);
            if leading_trailing_space(t1, s1 as isize) {
                s1 = trim_start(t1, s1, e1);
            }
            if leading_trailing_space(t1, e1 as isize - 1) {
                e1 = trim_end(t1, s1, e1);
            }
            if leading_trailing_space(t2, s2 as isize) {
                s2 = trim_start(t2, s2, e2);
            }
            if leading_trailing_space(t2, e2 as isize - 1) {
                e2 = trim_end(t2, s2, e2);
            }
            let x = R { s1, e1, s2, e2 };
            (!x.is_empty() && text1[s1..e1] != text2[s2..e2]).then_some(x)
        })
        .collect()
}

/// Match whitespace where possible, ignore it where not (IgnoreSpacesCorrector).
fn ignore_spaces(changes: &[R], text1: &str, text2: &str) -> Vec<R> {
    let (t1, t2) = (text1.as_bytes(), text2.as_bytes());
    changes
        .iter()
        .filter_map(|r| {
            let start = expand_ws_forward(t1, t2, r.s1, r.s2, r.e1, r.e2);
            let (s1, s2) = (r.s1 + start, r.s2 + start);
            let end = expand_ws_backward(t1, t2, s1, s2, r.e1, r.e2);
            let (e1, e2) = (r.e1 - end, r.e2 - end);
            let (s1, e1) = (trim_start(t1, s1, e1), trim_end(t1, trim_start(t1, s1, e1), e1));
            let (s2, e2) = (trim_start(t2, s2, e2), trim_end(t2, trim_start(t2, s2, e2), e2));
            let x = R { s1, e1, s2, e2 };
            (!x.is_empty() && !equals_ignore_ws(&text1[s1..e1], &text2[s2..e2])).then_some(x)
        })
        .collect()
}

/// Shrink ranges by whitespace that matches on all three sides (MergeDefaultCorrector).
fn merge_default_spaces(ranges: &[R3], t: [&[u8]; 3]) -> Vec<R3> {
    ranges
        .iter()
        .filter_map(|r| {
            let end = expand_ws3_backward(t, [r.s1, r.s2, r.s3], [r.e1, r.e2, r.e3]);
            let start = expand_ws3_forward(t, [r.s1, r.s2, r.s3], [r.e1 - end, r.e2 - end, r.e3 - end]);
            let x = R3 { s1: r.s1 + start, e1: r.e1 - end, s2: r.s2 + start, e2: r.e2 - end, s3: r.s3 + start, e3: r.e3 - end };
            (!x.is_empty()).then_some(x)
        })
        .collect()
}

/// Drop leading/trailing whitespace of lines from ranges (MergeTrimSpacesCorrector).
fn merge_trim_spaces(ranges: &[R3], t: [&str; 3]) -> Vec<R3> {
    let trim = |text: &str, s: usize, e: usize| {
        let b = text.as_bytes();
        let s = if leading_trailing_space(b, s as isize) { trim_start(b, s, e) } else { s };
        let e = if leading_trailing_space(b, e as isize - 1) { trim_end(b, s, e) } else { e };
        (s, e)
    };
    ranges
        .iter()
        .filter_map(|r| {
            let ((s1, e1), (s2, e2), (s3, e3)) = (trim(t[0], r.s1, r.e1), trim(t[1], r.s2, r.e2), trim(t[2], r.s3, r.e3));
            let x = R3 { s1, e1, s2, e2, s3, e3 };
            (!x.is_empty() && !equal3(&x, t, Policy::Default)).then_some(x)
        })
        .collect()
}

/// Match whitespace where possible, ignore it where not (MergeIgnoreSpacesCorrector).
fn merge_ignore_spaces(ranges: &[R3], t: [&str; 3]) -> Vec<R3> {
    let b = [t[0].as_bytes(), t[1].as_bytes(), t[2].as_bytes()];
    ranges
        .iter()
        .filter_map(|r| {
            let start = expand_ws3_forward(b, [r.s1, r.s2, r.s3], [r.e1, r.e2, r.e3]);
            let (s1, s2, s3) = (r.s1 + start, r.s2 + start, r.s3 + start);
            let end = expand_ws3_backward(b, [s1, s2, s3], [r.e1, r.e2, r.e3]);
            let trim = |t: &[u8], s: usize, e: usize| (trim_start(t, s, e), trim_end(t, trim_start(t, s, e), e));
            let ((s1, e1), (s2, e2), (s3, e3)) = (trim(b[0], s1, r.e1 - end), trim(b[1], s2, r.e2 - end), trim(b[2], s3, r.e3 - end));
            let x = R3 { s1, e1, s2, e2, s3, e3 };
            (!x.is_empty() && !equal3(&x, t, Policy::Ignore)).then_some(x)
        })
        .collect()
}

/// Identical whitespace at the start of all three ranges (TrimUtil.expandWhitespacesForward for three texts).
fn expand_ws3_forward(t: [&[u8]; 3], s: [usize; 3], e: [usize; 3]) -> usize {
    let n = (0..3).map(|i| e[i] - s[i]).min().unwrap();
    (0..n).find(|&k| t[0][s[0] + k] != t[1][s[1] + k] || t[0][s[0] + k] != t[2][s[2] + k] || !ws(t[0][s[0] + k])).unwrap_or(n)
}

/// Identical whitespace at the end of all three ranges (TrimUtil.expandWhitespacesBackward for three texts).
fn expand_ws3_backward(t: [&[u8]; 3], s: [usize; 3], e: [usize; 3]) -> usize {
    let n = (0..3).map(|i| e[i] - s[i]).min().unwrap();
    let at = |i: usize, k: usize| t[i][e[i] - 1 - k];
    (0..n).find(|&k| at(0, k) != at(1, k) || at(0, k) != at(2, k) || !ws(at(0, k))).unwrap_or(n)
}

/// Base equals left and right under `policy` within the range (TrimUtil.isEquals / isEqualsIgnoreWhitespaces).
fn equal3(r: &R3, t: [&str; 3], policy: Policy) -> bool {
    let base = &t[1][r.s2..r.e2];
    equal_texts(base, &t[0][r.s1..r.e1], policy) && equal_texts(base, &t[2][r.s3..r.e3], policy)
}

fn ws(b: u8) -> bool {
    is_ws(b as char)
}

pub fn expand_ws_forward(t1: &[u8], t2: &[u8], s1: usize, s2: usize, e1: usize, e2: usize) -> usize {
    (0..(e1 - s1).min(e2 - s2)).find(|&i| t1[s1 + i] != t2[s2 + i] || !ws(t1[s1 + i])).unwrap_or((e1 - s1).min(e2 - s2))
}

fn expand_ws_backward(t1: &[u8], t2: &[u8], s1: usize, s2: usize, e1: usize, e2: usize) -> usize {
    let n = (e1 - s1).min(e2 - s2);
    (0..n).find(|&i| t1[e1 - 1 - i] != t2[e2 - 1 - i] || !ws(t1[e1 - 1 - i])).unwrap_or(n)
}

fn trim_start(t: &[u8], mut start: usize, end: usize) -> usize {
    while start < end && ws(t[start]) {
        start += 1;
    }
    start
}

fn trim_end(t: &[u8], start: usize, mut end: usize) -> usize {
    while start < end && ws(t[end - 1]) {
        end -= 1;
    }
    end
}

/// Whitespace at `i` that only has whitespace between it and the line start or end.
fn leading_trailing_space(t: &[u8], i: isize) -> bool {
    if i < 0 || i as usize >= t.len() || !ws(t[i as usize]) {
        return false;
    }
    let i = i as usize;
    // Each scan stops at the first non-space; the second runs only when the first says no.
    t[..i].iter().rev().find(|&&c| c == b'\n' || !ws(c)).is_none_or(|&c| c == b'\n')
        || t[i..].iter().find(|&&c| c == b'\n' || !ws(c)).is_none_or(|&c| c == b'\n')
}
