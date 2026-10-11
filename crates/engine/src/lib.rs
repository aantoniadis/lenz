#![forbid(unsafe_code)]
use std::borrow::Cow;
use std::ops::Range;

mod by_char;
mod by_line;
mod by_word;
mod chars;
mod lcs;
mod merge;

use by_line::{Policy, R};

pub use merge::{MergeBlock, MergeInline, MergeKind, MergeOptions, merge_inline, merge_lines, resolve_simple};

/// How whitespace counts when comparing Sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WhitespaceMode {
    #[default]
    Default,
    Trim,
    Ignore,
    /// Ignore, and whitespace-only edges of a Change Block are dropped (a block that is nothing else vanishes).
    IgnoreTrimChunks,
}

/// How finely a modified Change Block is refined into Inline Changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HighlightLevel {
    #[default]
    Word,
    Char,
    /// No Inline Changes.
    Line,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Options {
    pub whitespace: WhitespaceMode,
    pub ignore_case: bool,
    pub highlight: HighlightLevel,
}

/// A contiguous run of lines that differ between the two Sides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeBlock {
    pub left: Range<u32>,
    pub right: Range<u32>,
    /// The Sides are equal under the current Whitespace Mode / Ignore Case: an Ignored Change, shown faded.
    /// Whenever `inline` is `Some`, this is exactly `inline == Some(vec![])`.
    pub ignored: bool,
    /// `None`: not refined (Highlight Level Line, or an insertion/deletion that is a real change).
    /// `Some(empty)`: refined, and the Sides are equal under the current modes (an Ignored Change).
    /// `Some(list)`: the Inline Changes, in order.
    pub inline: Option<Vec<InlineChange>>,
}

/// A differing span inside a Change Block, as byte ranges into each Side's *block text*: the block's lines
/// (as given to [`compare`], so without `\r`) joined with `"\n"`, without a trailing newline.
/// Ranges always lie on char boundaries; one of them may be empty (a pure insertion or deletion).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineChange {
    pub left: Range<usize>,
    pub right: Range<usize>,
}

/// A text's line-ending convention; when the Sides differ the viewer shows a file-level notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    None,
    Lf,
    CrLf,
    Mixed,
}

pub fn line_ending(text: &str) -> LineEnding {
    let newlines = text.matches('\n').count();
    let crlf = text.matches("\r\n").count();
    match (newlines, crlf) {
        (0, _) => LineEnding::None,
        (n, c) if c == n => LineEnding::CrLf,
        (_, 0) => LineEnding::Lf,
        _ => LineEnding::Mixed,
    }
}

/// Split text into lines on `\n` exactly as IntelliJ does: `"a\nb\n"` is three lines, the last empty.
/// A `\r` before each `\n` is dropped, so CRLF and LF texts compare by content.
pub fn split_lines(text: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
    // only a `\r` that ended a line: a lone one at the very end is content, and shows (git shows it too)
    if let Some(last) = lines.last_mut() {
        *last = &text[text.rfind('\n').map_or(0, |i| i + 1)..];
    }
    lines
}

/// Change Blocks between two Sides, each modified one refined into Inline Changes per `options.highlight`
/// (IntelliJ's compareLinesInner, then processBlocks squashing each block's parts back together, and trimming
/// whitespace-only parts off its edges under `IgnoreTrimChunks`). Never gives up: every block is refined.
/// The same as [`compare_lines`] followed by [`refine`] of each block with an unlimited [`Budget`].
pub fn compare(left: &[&str], right: &[&str], options: &Options) -> Vec<ChangeBlock> {
    blocks(left, right, options, true)
}

/// Exact line changes by plain Myers LCS, without [`compare_lines`]' pairing of distinctive lines first (which, among
/// repeated lines such as conflict markers, can pair the wrong ones): for lining up texts that are mostly the same,
/// such as a file against the merge output it was edited from. Not refined; `ignored` is false.
pub fn diff_lines<'a>(left: &[&'a str], right: &[&'a str]) -> Vec<ChangeBlock> {
    let mut ids = std::collections::HashMap::new();
    let mut intern = |lines: &[&'a str]| -> Vec<u32> {
        lines
            .iter()
            .map(|&l| {
                let next = ids.len() as u32;
                *ids.entry(l).or_insert(next)
            })
            .collect()
    };
    let (a, b) = (intern(left), intern(right));
    by_line::diff(&a, &b, ids.len() as u32)
        .into_iter()
        .map(|r| ChangeBlock { left: r.s1 as u32..r.e1 as u32, right: r.s2 as u32..r.e2 as u32, ignored: false, inline: None })
        .collect()
}

/// Change Blocks between two Sides without Inline Changes (`inline: None`), for [`refine`] to fill in lazily.
/// `ignored` is judged on whole lines, as at Highlight Level Line. Under `IgnoreTrimChunks`, insertions and deletions
/// of nothing but whitespace are already gone (at Highlight Level Line, any block equal under the mode is).
pub fn compare_lines(left: &[&str], right: &[&str], options: &Options) -> Vec<ChangeBlock> {
    let policy = policy(options.whitespace);
    let trim = options.whitespace == WhitespaceMode::IgnoreTrimChunks;
    let prepared = by_line::prepare(left, right, policy, options.ignore_case);
    let mut out = Vec::new();
    for r in by_line::compare(&prepared, policy) {
        let one_sided = r.s1 == r.e1 || r.s2 == r.e2;
        let ignored = if one_sided {
            // createInnerFragments: the inserted/deleted text, with the newline after it, against nothing
            let text = if r.s1 == r.e1 { block_text(right, r.s2, r.e2) } else { block_text(left, r.s1, r.e1) };
            match policy {
                Policy::Default => text.is_empty(),
                Policy::Trim => !text.contains('\n') && text.chars().all(by_line::is_ws),
                Policy::Ignore => text.chars().all(by_line::is_ws),
            }
        } else {
            equal_under(&left[r.s1..r.e1], &right[r.s2..r.e2], policy, options.ignore_case)
        };
        // trimming an all-whitespace block leaves nothing; a refined one is trimmed part by part
        if trim && ignored && (one_sided || options.highlight == HighlightLevel::Line) {
            continue;
        }
        out.push(ChangeBlock { left: r.s1 as u32..r.e1 as u32, right: r.s2 as u32..r.e2 as u32, ignored, inline: None });
    }
    out
}

/// DiffConfig.MAX_BAD_LINES: after this many too-costly blocks, the rest of a file isn't refined.
pub const MAX_BAD_LINES: u32 = 3;

/// A modified block with more tokens than this (both Sides together, newlines included; words at Highlight Level
/// Word, characters at Char) is too costly to refine. Lenz's stand-in for IntelliJ's DiffTooBigException, which
/// needs the patience fallback that isn't ported: here the LCS cost cap falls back to gix-imara-diff and never fails.
pub const MAX_REFINE_TOKENS: usize = 100_000;

/// Word-level changes between two texts: each item is the changed byte range in `text1` and in `text2`, in order,
/// on char boundaries.
pub fn word_changes(text1: &str, text2: &str) -> Vec<[std::ops::Range<usize>; 2]> {
    by_word::compare(text1, text2, Policy::Default).into_iter().map(|r| [r.s1..r.e1, r.s2..r.e2]).collect()
}

/// How much refinement one Comparison has left (ComparisonManagerImpl.createInnerFragments' `tooBigChunksCount`).
/// Keep one per Comparison and options; start afresh when they change.
#[derive(Debug, Clone, Default)]
pub struct Budget {
    too_big: u32,
}

/// What [`refine`] made of a block from [`compare_lines`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refinement {
    /// The block to show instead, `inline` and `ignored` as [`compare`] gives them. Under `IgnoreTrimChunks` its line
    /// ranges may have narrowed: whitespace-only lines at its edges are trimmed off.
    Refined(ChangeBlock),
    /// Under `IgnoreTrimChunks` nothing but whitespace was left: [`compare`] has no such block.
    Dropped,
    /// Too costly to refine, or the budget is spent: show the block line-only, as it is.
    TooBig,
}

/// Refine one block from [`compare_lines`] (same Sides, same `options`) as [`compare`] would have. Insertions and
/// deletions are never too big; a modified block is [`Refinement::TooBig`] when it has over [`MAX_REFINE_TOKENS`]
/// tokens, which costs `budget` a strike, and without any work once [`MAX_BAD_LINES`] strikes are spent.
pub fn refine(left: &[&str], right: &[&str], block: &ChangeBlock, options: &Options, budget: &mut Budget) -> Refinement {
    let two_sided = !block.left.is_empty() && !block.right.is_empty();
    // A block from another version of the Sides: line-only, never a slice past the end.
    if block.left.end as usize > left.len() || block.right.end as usize > right.len() {
        return Refinement::TooBig;
    }
    if options.highlight != HighlightLevel::Line && two_sided {
        // Do not try to build fine blocks after few fails
        if budget.too_big >= MAX_BAD_LINES {
            return Refinement::TooBig;
        }
        let tokens = |lines: &[&str], r: &Range<u32>| -> usize {
            let lines = &lines[r.start as usize..r.end as usize];
            // Punctuation marks are tokens too (the delimiter matcher diffs them), as `merge` counts them.
            let count = |l: &&str| if options.highlight == HighlightLevel::Char { l.chars().count() } else { by_word::count(l) + l.bytes().filter(|&b| chars::is_punctuation(b as char)).count() };
            lines.iter().map(count).sum::<usize>() + lines.len()
        };
        if tokens(left, &block.left) + tokens(right, &block.right) > MAX_REFINE_TOKENS {
            budget.too_big += 1;
            return Refinement::TooBig;
        }
    }
    match refine_block(left, right, block, options, true).pop() {
        Some(b) => Refinement::Refined(b),
        None => Refinement::Dropped,
    }
}

pub(crate) fn policy(mode: WhitespaceMode) -> Policy {
    match mode {
        WhitespaceMode::Default => Policy::Default,
        WhitespaceMode::Trim => Policy::Trim,
        WhitespaceMode::Ignore | WhitespaceMode::IgnoreTrimChunks => Policy::Ignore,
    }
}

/// A line-aligned part of a refined block: lines relative to the block, `offsets` and `inner` into its text.
struct Part {
    lines: R,
    offsets: R,
    inner: Vec<R>,
}

/// `squash: false` keeps the line-aligned parts of a refined block as blocks of their own (IntelliJ's "split").
fn blocks(left: &[&str], right: &[&str], options: &Options, squash: bool) -> Vec<ChangeBlock> {
    compare_lines(left, right, options).iter().flat_map(|b| refine_block(left, right, b, options, squash)).collect()
}

/// One block from [`compare_lines`] refined without limits (createInnerFragments): none, one, or unsquashed several.
fn refine_block(left: &[&str], right: &[&str], b: &ChangeBlock, options: &Options, squash: bool) -> Vec<ChangeBlock> {
    let r = R { s1: b.left.start as usize, e1: b.left.end as usize, s2: b.right.start as usize, e2: b.right.end as usize };
    if options.highlight == HighlightLevel::Line || r.s1 == r.e1 || r.s2 == r.e2 {
        // insertions and deletions are only checked for equality, which compare_lines did
        let inline = (options.highlight != HighlightLevel::Line && b.ignored).then(Vec::new);
        return vec![ChangeBlock { inline, ..b.clone() }];
    }
    let policy = policy(options.whitespace);
    let mut out = Vec::new();
    // IntelliJ refines the block's text including the newline after its last line
    let (t1, t2) = (block_text(left, r.s1, r.e1), block_text(right, r.s2, r.e2));
    let (f1, map1) = fold(&t1, options.ignore_case);
    let (f2, map2) = fold(&t2, options.ignore_case);
    let mut parts = refine_parts(&f1, &f2, r.e1 - r.s1, r.e2 - r.s2, policy, options.highlight);
    if options.whitespace == WhitespaceMode::IgnoreTrimChunks {
        let keep = |p: &Part| {
            !p.inner.is_empty() && !chars::equals_ignore_ws(&f1[p.offsets.s1..p.offsets.e1], &f2[p.offsets.s2..p.offsets.e2])
        };
        let Some(start) = parts.iter().position(keep) else { return Vec::new() };
        let end = parts.iter().rposition(keep).unwrap() + 1;
        parts.truncate(end);
        parts.drain(..start);
    }
    // Merge views also refine their aligned, already-equal ranges. A shared empty final line has
    // no text (and therefore no word parts), but its range must stay to keep merge block IDs aligned.
    if parts.is_empty() {
        return vec![ChangeBlock { inline: Some(Vec::new()), ignored: true, ..b.clone() }];
    }
    let at = |map: &Option<Vec<usize>>, x: usize| map.as_ref().map_or(x, |m| m[x]);
    let unfold = |x: R| R { s1: at(&map1, x.s1), e1: at(&map1, x.e1), s2: at(&map2, x.s2), e2: at(&map2, x.e2) };
    let groups: Vec<&[Part]> = if squash { vec![&parts] } else { parts.chunks(1).collect() };
    for group in groups {
        let (first, last) = (&group[0], &group[group.len() - 1]);
        let lines = R { s1: r.s1 + first.lines.s1, e1: r.s1 + last.lines.e1, s2: r.s2 + first.lines.s2, e2: r.s2 + last.lines.e2 };
        let base = unfold(first.offsets);
        // ComparisonManagerImpl.doSquash: Inline Changes adjoining across parts merge
        let mut inner: Vec<R> = Vec::new();
        for f in group.iter().flat_map(|p| &p.inner).map(|&f| unfold(f)) {
            match inner.last_mut() {
                Some(prev) if prev.e1 == f.s1 && prev.e2 == f.s2 => (prev.e1, prev.e2) = (f.e1, f.e2),
                _ => inner.push(f),
            }
        }
        // rebase onto the block text, which ends before the newline IntelliJ included
        let (len1, len2) = (joined_len(left, lines.s1, lines.e1), joined_len(right, lines.s2, lines.e2));
        let inline: Vec<InlineChange> = inner
            .iter()
            .map(|f| InlineChange {
                left: (f.s1 - base.s1).min(len1)..(f.e1 - base.s1).min(len1),
                right: (f.s2 - base.s2).min(len2)..(f.e2 - base.s2).min(len2),
            })
            .filter(|c| !c.left.is_empty() || !c.right.is_empty())
            .collect();
        out.push(ChangeBlock {
            left: lines.s1 as u32..lines.e1 as u32,
            right: lines.s2 as u32..lines.e2 as u32,
            ignored: inline.is_empty(),
            inline: Some(inline),
        });
    }
    out
}

/// IntelliJ's createInnerWordFragments / createInnerCharFragments for one modified block.
fn refine_parts(t1: &str, t2: &str, lines1: usize, lines2: usize, policy: Policy, level: HighlightLevel) -> Vec<Part> {
    if level == HighlightLevel::Char {
        let (lines, offsets) = (R { s1: 0, e1: lines1, s2: 0, e2: lines2 }, R { s1: 0, e1: t1.len(), s2: 0, e2: t2.len() });
        return vec![Part { lines, offsets, inner: chars_policy(t1, t2, policy) }];
    }
    let blocks = by_word::compare_and_split(t1, t2, policy);
    let (mut l1, mut l2) = (0, 0);
    let n = blocks.len();
    blocks
        .iter()
        .enumerate()
        .map(|(i, b)| {
            // the last part ends where the block does (its last line may have no newline)
            let (e1, e2) = if i + 1 == n { (lines1, lines2) } else { (l1 + b.newlines1, l2 + b.newlines2) };
            let o = b.offsets;
            let inner = b.fragments.iter().map(|f| R { s1: f.s1 + o.s1, e1: f.e1 + o.s1, s2: f.s2 + o.s2, e2: f.e2 + o.s2 });
            let part = Part { lines: R { s1: l1, e1, s2: l2, e2 }, offsets: o, inner: inner.collect() };
            (l1, l2) = (e1, e2);
            part
        })
        .collect()
}

/// ComparisonManagerImpl.doCompareChars.
fn chars_policy(t1: &str, t2: &str, policy: Policy) -> Vec<R> {
    match policy {
        Policy::Default => by_char::compare_two_step(t1, t2),
        Policy::Trim => by_char::compare_trim_whitespaces(t1, t2),
        Policy::Ignore => by_char::compare_ignore_whitespaces(t1, t2),
    }
}

/// Lines `[start, end)` joined with `\n`, plus the newline after the last one unless it ends the text.
fn block_text(lines: &[&str], start: usize, end: usize) -> String {
    let mut text = lines[start..end].join("\n");
    if end < lines.len() {
        text.push('\n');
    }
    text
}

fn joined_len(lines: &[&str], start: usize, end: usize) -> usize {
    lines[start..end].iter().map(|l| l.len() + 1).sum::<usize>().saturating_sub(1)
}

/// The text with Ignore Case applied, and a map from its byte offsets back to the original's
/// (folding may change a char's UTF-8 length, e.g. the Kelvin sign).
fn fold(text: &str, ignore_case: bool) -> (Cow<'_, str>, Option<Vec<usize>>) {
    if !ignore_case {
        return (Cow::Borrowed(text), None);
    }
    let folded = chars::fold_str(text);
    if text.chars().all(|c| chars::fold(c).len_utf8() == c.len_utf8()) {
        return (Cow::Owned(folded), None);
    }
    let mut map = vec![0; folded.len() + 1];
    let (mut f, mut o) = (0, 0);
    for c in text.chars() {
        map[f] = o;
        f += chars::fold(c).len_utf8();
        o += c.len_utf8();
    }
    map[f] = o;
    (Cow::Owned(folded), Some(map))
}

/// ComparisonUtil.isEqualTexts on a modified block's lines (joined with `\n`).
fn equal_under(left: &[&str], right: &[&str], policy: Policy, ignore_case: bool) -> bool {
    let fold = |l: &&str| if ignore_case { chars::fold_str(l) } else { l.to_string() };
    let ws = |c: char| c == ' ' || c == '\t' || c == '\n';
    match policy {
        Policy::Default => false,
        // inner lines are compared trimmed, so line counts must match
        Policy::Trim => {
            left.len() == right.len() && left.iter().zip(right).all(|(a, b)| fold(a).trim_matches(ws) == fold(b).trim_matches(ws))
        }
        Policy::Ignore => {
            let squeeze = |ls: &[&str]| ls.iter().map(fold).collect::<Vec<_>>().join("\n").replace(ws, "");
            squeeze(left) == squeeze(right)
        }
    }
}

/// Entry points for IntelliJ's golden tests; not a stable API.
#[doc(hidden)]
pub mod golden {
    use super::*;

    fn inline(changes: Vec<R>) -> Vec<InlineChange> {
        changes.into_iter().map(|r| InlineChange { left: r.s1..r.e1, right: r.s2..r.e2 }).collect()
    }

    /// ByWordRt.compare over whole texts (Ignore Case not applied).
    pub fn words(left: &str, right: &str, options: &Options) -> Vec<InlineChange> {
        inline(by_word::compare(left, right, policy(options.whitespace)))
    }

    /// ComparisonManagerImpl.compareChars (Ignore Case not applied).
    pub fn chars(left: &str, right: &str, options: &Options) -> Vec<InlineChange> {
        inline(chars_policy(left, right, policy(options.whitespace)))
    }

    /// ByCharRt.compare.
    pub fn chars_raw(left: &str, right: &str) -> Vec<InlineChange> {
        inline(by_char::compare(left, right))
    }

    /// Like [`compare`], but the line-aligned parts of refined blocks are not squashed back together.
    pub fn split(left: &[&str], right: &[&str], options: &Options) -> Vec<ChangeBlock> {
        blocks(left, right, options, false)
    }

    fn ranges3(ranges: Vec<merge::R3>) -> Vec<[usize; 6]> {
        ranges.iter().map(|m| [m.s1, m.e1, m.s2, m.e2, m.s3, m.e3]).collect()
    }

    /// Two ByCharRt.compare calls (base→left, base→right) zipped by ComparisonMergeUtil.buildSimple:
    /// [left start, left end, base start, base end, right start, right end].
    pub fn merge_chars(left: &str, base: &str, right: &str) -> Vec<[usize; 6]> {
        let from_base = |side: &str| by_line::unchanged(&by_char::compare(base, side), base.len(), side.len());
        ranges3(merge::zip(&from_base(left), &from_base(right), [left.len(), base.len(), right.len()], None))
    }

    /// ComparisonManager.mergeLines (`keep_ignored`) or compareLines for three sides, untyped.
    pub fn merge_line_ranges(left: &[&str], base: &[&str], right: &[&str], options: &MergeOptions, keep_ignored: bool) -> Vec<[usize; 6]> {
        ranges3(merge::line_ranges(left, base, right, policy(options.whitespace), keep_ignored))
    }

    /// ByWordRt.compare for three texts.
    pub fn merge_words(left: &str, base: &str, right: &str, options: &MergeOptions) -> Vec<[usize; 6]> {
        ranges3(by_word::compare3(left, base, right, policy(options.whitespace)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(left: &str, right: &str, whitespace: WhitespaceMode, ignore_case: bool) -> Vec<ChangeBlock> {
        compare(&split_lines(left), &split_lines(right), &Options { whitespace, ignore_case, ..Options::default() })
    }

    #[test]
    fn blank_lines_inserted_under_ignore_whitespace_are_an_ignored_change() {
        let blocks = run("a\nb", "a\n\n  \nb", WhitespaceMode::Ignore, false);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].ignored);
    }

    #[test]
    fn ignore_case_makes_case_only_edits_disappear_in_any_whitespace_mode() {
        assert_eq!(run("Foo\nbar", "foo\nBAR", WhitespaceMode::Default, true), vec![]);
        assert_eq!(run("Foo bar", "foo  BAR", WhitespaceMode::Ignore, true), vec![]);
        assert_eq!(run("Foo", "foo", WhitespaceMode::Default, false).len(), 1);
    }

    #[test]
    fn crlf_and_lf_texts_compare_equal_and_their_line_endings_are_reported() {
        assert_eq!(run("a\r\nb\r\n", "a\nb\n", WhitespaceMode::Default, false), vec![]);
        assert_eq!(line_ending("a\r\nb\r\n"), LineEnding::CrLf);
        assert_eq!(line_ending("a\nb\n"), LineEnding::Lf);
        assert_eq!(line_ending("a\r\nb\n"), LineEnding::Mixed);
        assert_eq!(line_ending("single line"), LineEnding::None);
    }

    fn inline(left: &str, right: &str, options: Options) -> Vec<Option<Vec<(String, String)>>> {
        let (l, r) = (split_lines(left), split_lines(right));
        compare(&l, &r, &options)
            .iter()
            .map(|b| {
                let (bl, br) = (l[b.left.start as usize..b.left.end as usize].join("\n"), r[b.right.start as usize..b.right.end as usize].join("\n"));
                if let Some(i) = &b.inline {
                    assert_eq!(b.ignored, i.is_empty());
                }
                b.inline.as_ref().map(|i| i.iter().map(|c| (bl[c.left.clone()].to_string(), br[c.right.clone()].to_string())).collect())
            })
            .collect()
    }

    fn pairs(p: &[(&str, &str)]) -> Option<Vec<(String, String)>> {
        Some(p.iter().map(|&(a, b)| (a.to_string(), b.to_string())).collect())
    }

    #[test]
    fn a_modified_block_is_refined_by_word_and_its_offsets_exclude_the_newline_after_it() {
        let got = inline("a\nlet x = 1;\nb", "a\nlet y = 1;\nb", Options::default());
        assert_eq!(got, vec![pairs(&[("x", "y")])]);
    }

    #[test]
    fn highlight_levels_refine_by_char_or_not_at_all() {
        let char_level = Options { highlight: HighlightLevel::Char, ..Options::default() };
        assert_eq!(inline("value", "valve", char_level), vec![pairs(&[("u", "v")])]);
        assert_eq!(inline("value", "valve", Options::default()), vec![pairs(&[("value", "valve")])]);
        let line_level = Options { highlight: HighlightLevel::Line, ..Options::default() };
        assert_eq!(inline("value", "valve", line_level), vec![None]);
    }

    #[test]
    fn a_pure_insertion_is_not_refined_unless_it_is_equal_under_the_whitespace_mode() {
        assert_eq!(inline("a\nb", "a\nx\nb", Options::default()), vec![None]);
        let ignore = Options { whitespace: WhitespaceMode::Ignore, ..Options::default() };
        assert_eq!(inline("a\nb", "a\n  \nb", ignore), vec![Some(vec![])]);
        // IntelliJ compares the inserted text with the newline after it: gaining a final newline adds an
        // empty last line, which is nothing at all; a blank last line is nothing once trimmed
        assert_eq!(inline("a", "a\n", Options::default()), vec![Some(vec![])]);
        let trim = Options { whitespace: WhitespaceMode::Trim, ..Options::default() };
        assert_eq!(inline("a", "a\n  ", trim), vec![Some(vec![])]);
        assert_eq!(inline("a\nb", "a\n  \nb", trim), vec![None]);
        assert_eq!(inline("a\nb", "a\n\nb", Options::default()), vec![None]);
    }

    #[test]
    fn ignore_case_applies_to_word_equality_with_offsets_in_the_original_text() {
        let ic = Options { ignore_case: true, ..Options::default() };
        assert_eq!(inline("Foo Bar x", "foo bar y", ic), vec![pairs(&[("x", "y")])]);
        // the Kelvin sign folds to a one-byte 'k'
        assert_eq!(inline("\u{212A}elvin x", "kelvin y", ic), vec![pairs(&[("x", "y")])]);
        let ic_char = Options { highlight: HighlightLevel::Char, ..ic };
        assert_eq!(inline("\u{212A}elvin x", "kelvin y", ic_char), vec![pairs(&[("x", "y")])]);
    }

    #[test]
    fn ignore_trim_chunks_drops_whitespace_only_blocks_and_block_edges() {
        let trim = Options { whitespace: WhitespaceMode::IgnoreTrimChunks, ..Options::default() };
        assert_eq!(compare(&split_lines("a\nb"), &split_lines("a\n\n  \nb"), &trim), vec![]);
        // "a b" re-wrapped is a whitespace-only part at the block's edge, and trimmed off
        let (l, r) = (split_lines("a b\nx"), split_lines("a\nb\ny"));
        let blocks = compare(&l, &r, &trim);
        assert_eq!((blocks[0].left.clone(), blocks[0].right.clone()), (1..2, 2..3));
        assert_eq!(blocks[0].inline, Some(vec![InlineChange { left: 0..1, right: 0..1 }]));
        let ignore = Options { whitespace: WhitespaceMode::Ignore, ..Options::default() };
        let blocks = compare(&l, &r, &ignore);
        assert_eq!((blocks[0].left.clone(), blocks[0].right.clone()), (0..2, 0..3));
        assert_eq!(blocks[0].inline, Some(vec![InlineChange { left: 4..5, right: 4..5 }]));
    }

    #[test]
    fn compare_lines_leaves_every_block_unrefined_and_judges_ignored_by_lines() {
        let ignore = Options { whitespace: WhitespaceMode::Ignore, ..Options::default() };
        let blocks = compare_lines(&split_lines("a\nb\nx y"), &split_lines("a\n\n  \nb\nx  z"), &ignore);
        let got: Vec<_> = blocks.iter().map(|b| (b.left.clone(), b.right.clone(), b.ignored, b.inline.clone())).collect();
        assert_eq!(got, vec![(1..1, 1..3, true, None), (2..3, 4..5, false, None)]);
        let trim = Options { whitespace: WhitespaceMode::IgnoreTrimChunks, ..Options::default() };
        assert_eq!(compare_lines(&split_lines("a\nb"), &split_lines("a\n\n  \nb"), &trim), vec![]);
    }

    #[test]
    fn under_ignore_trim_chunks_a_rewrapped_block_is_trimmed_by_its_parts_not_as_a_whole() {
        let words = Options { whitespace: WhitespaceMode::IgnoreTrimChunks, ..Options::default() };
        // equal once all whitespace is gone, yet the words that moved across the line break are a change
        let (l, r) = (["a b", "c"], ["a", "b c"]);
        let changes = vec![InlineChange { left: 2..3, right: 1..1 }, InlineChange { left: 4..4, right: 2..3 }];
        let refined = ChangeBlock { left: 0..2, right: 0..2, ignored: false, inline: Some(changes) };
        assert_eq!(compare(&l, &r, &words), vec![refined.clone()]);
        let blocks = compare_lines(&l, &r, &words);
        assert_eq!(refine(&l, &r, &blocks[0], &words, &mut Budget::default()), Refinement::Refined(refined));
        // here every part is whitespace-only: the block goes
        let (l, r) = (["ab", "c"], ["a", "bc"]);
        assert_eq!(compare(&l, &r, &words), vec![]);
        let blocks = compare_lines(&l, &r, &words);
        assert_eq!(refine(&l, &r, &blocks[0], &words, &mut Budget::default()), Refinement::Dropped);
        // by line, a block equal under the Whitespace Mode is gone at once
        let lines = Options { highlight: HighlightLevel::Line, ..words };
        assert_eq!(compare_lines(&l, &r, &lines), vec![]);
    }

    #[test]
    fn refining_under_ignore_trim_chunks_narrows_a_block_to_its_real_change() {
        let trim = Options { whitespace: WhitespaceMode::IgnoreTrimChunks, ..Options::default() };
        let (l, r) = (split_lines("a b\nx"), split_lines("a\nb\ny"));
        let blocks = compare_lines(&l, &r, &trim);
        assert_eq!((blocks[0].left.clone(), blocks[0].right.clone()), (0..2, 0..3));
        let refined = refine(&l, &r, &blocks[0], &trim, &mut Budget::default());
        assert_eq!(refined, Refinement::Refined(compare(&l, &r, &trim).remove(0)));
    }

    fn words(word: &str, n: usize) -> String {
        vec![word; n].join(" ")
    }

    #[test]
    fn three_blocks_over_the_token_cap_stop_refinement_of_two_sided_blocks() {
        let big = MAX_REFINE_TOKENS / 2 + 1; // per Side, so a block of them is over the cap
        let (a, b) = (words("a", big), words("b", big));
        let left = [a.as_str(), "keep 1", &a, "keep 2", "small one", "keep 3", &a, "keep 4", "small three", "keep 5"];
        let right = [b.as_str(), "keep 1", &b, "keep 2", "small two", "keep 3", &b, "keep 4", "small four", "keep 5", "added"];
        let options = Options::default();
        let blocks = compare_lines(&left, &right, &options);
        assert_eq!(blocks.len(), 6);
        let mut budget = Budget::default();
        let got: Vec<Refinement> = blocks.iter().map(|b| refine(&left, &right, b, &options, &mut budget)).collect();
        use Refinement::*;
        assert_eq!(got[..2], [TooBig, TooBig]);
        // two strikes: still refining
        assert!(matches!(&got[2], Refined(b) if b.inline == Some(vec![InlineChange { left: 6..9, right: 6..9 }])));
        assert_eq!(got[3], TooBig);
        // the third strike: a small block is no longer refined
        assert_eq!(got[4], TooBig);
        // insertions and deletions are never refined, so the budget doesn't apply to them
        assert_eq!(got[5], Refined(blocks[5].clone()));
        // `compare` has no budget: it refines every block
        assert!(compare(&left, &right, &options).iter().take(5).all(|b| b.inline.is_some()));
    }

    #[test]
    fn the_token_cap_counts_words_by_word_and_characters_by_character() {
        let (x, y) = ("x".repeat(MAX_REFINE_TOKENS), "y".repeat(MAX_REFINE_TOKENS));
        let (left, right) = ([x.as_str()], [y.as_str()]);
        let refine_at = |highlight| {
            let options = Options { highlight, ..Options::default() };
            refine(&left, &right, &compare_lines(&left, &right, &options)[0], &options, &mut Budget::default())
        };
        assert!(matches!(refine_at(HighlightLevel::Word), Refinement::Refined(_)));
        assert_eq!(refine_at(HighlightLevel::Char), Refinement::TooBig);
        // Line level has nothing to refine
        let line = Options { highlight: HighlightLevel::Line, ..Options::default() };
        let block = compare_lines(&left, &right, &line).remove(0);
        assert_eq!(refine(&left, &right, &block, &line, &mut Budget::default()), Refinement::Refined(block));
    }

    #[test]
    fn a_real_change_is_never_ignored() {
        let blocks = run("a\nb", "a\nc", WhitespaceMode::Ignore, false);
        assert!(!blocks[0].ignored);
        let blocks = run("a\nb", "a\n\nb", WhitespaceMode::Default, false);
        assert!(!blocks[0].ignored);
    }
}

