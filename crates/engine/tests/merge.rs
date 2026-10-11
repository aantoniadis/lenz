//! Merge blocks, the simple resolver and Inline Changes in merge blocks through the public API: block kinds as in
//! IntelliJ's MergeTest (testChangeTypes, testLastLine, testIgnored), ComparisonUtilAutoTest.testMerge as a seeded
//! property test, and the resolver's cost cap.

use std::time::Instant;

use lenz_engine::{
    MAX_REFINE_TOKENS, MergeBlock, MergeInline, MergeKind, MergeOptions, WhitespaceMode, golden, merge_inline, merge_lines,
    resolve_simple, split_lines,
};

/// `_` is a newline, as in IntelliJ's tests.
fn merge(left: &str, base: &str, right: &str, whitespace: WhitespaceMode) -> Vec<MergeBlock> {
    let (l, b, r) = (left.replace('_', "\n"), base.replace('_', "\n"), right.replace('_', "\n"));
    merge_lines(&split_lines(&l), &split_lines(&b), &split_lines(&r), &MergeOptions { whitespace })
}

/// Kind and changed Sides: "left", "right" or "both".
fn types(left: &str, base: &str, right: &str, whitespace: WhitespaceMode) -> Vec<(MergeKind, &'static str)> {
    let side = |b: &MergeBlock| match (b.left_changed, b.right_changed) {
        (true, true) => "both",
        (true, false) => "left",
        (false, true) => "right",
        _ => "none",
    };
    merge(left, base, right, whitespace).iter().map(|b| (b.kind, side(b))).collect()
}

#[test]
fn block_kinds_and_changed_sides_follow_intellij() {
    use MergeKind::*;
    let d = WhitespaceMode::Default;
    // MergeTest.testChangeTypes: (left, base, right)
    assert_eq!(types("", "", "", d), []);
    assert_eq!(types("x", "x", "x", d), []);
    assert_eq!(types("x_y_z a x", "x_y_z a x", "x_y_z a x", d), []);
    assert_eq!(types("x", "x", "y", d), [(Modified, "right")]);
    assert_eq!(types("x", "y", "x", d), [(Modified, "both")]);
    assert_eq!(types("x_Y", "Y", "Y_z", d), [(Inserted, "left"), (Inserted, "right")]);
    assert_eq!(types("Y_z", "x_Y_z", "x_Y", d), [(Deleted, "left"), (Deleted, "right")]);
    assert_eq!(types("X_Z", "X_y_Z", "X_Z", d), [(Deleted, "both")]);
    assert_eq!(types("X_y_Z", "X_Z", "X_y_Z", d), [(Inserted, "both")]);
    for (l, b, r) in [("x", "y", "z"), ("z_Y", "x_Y", "Y"), ("z_Y", "x_Y", "k_x_Y"), ("x_Y", "Y", "z_Y"), ("x_Y", "Y", "z_x_Y"), ("x_Y", "x_z_Y", "z_Y")] {
        assert_eq!(types(l, b, r, d), [(Conflict, "both")], "{l:?} {b:?} {r:?}");
    }
    // testLastLine
    assert_eq!(types("x", "x_", "x", d), [(Deleted, "both")]);
    assert_eq!(types("x_", "x_", "x", d), [(Deleted, "right")]);
    assert_eq!(types("x_", "x_", "x_y", d), [(Modified, "right")]);
    assert_eq!(types("x", "x_", "x_y", d), [(Conflict, "both")]);
    assert_eq!(types("x_", "x", "x_y", d), [(Conflict, "both")]);
    // testIgnored: whitespace-only runs are blocks of their own, their changed Sides judged exactly
    let i = WhitespaceMode::Ignore;
    let got = types(" x_new_y_z", "x_ y _z", "x_y _new_ z", i);
    assert_eq!(got, [(Modified, "left"), (Inserted, "left"), (Modified, "both"), (Inserted, "right"), (Modified, "right")]);
    let got = types(" x_new_y1_z", "x_ y _z", "x_y2 _new_ z", i);
    assert_eq!(got, [(Modified, "left"), (Conflict, "both"), (Modified, "right")]);
}

#[test]
fn a_conflict_is_resolvable_when_its_word_edits_do_not_overlap_and_no_side_is_empty() {
    let blocks = merge("a b c", "a b c d", "a B c d", WhitespaceMode::Default);
    assert_eq!((blocks[0].kind, blocks[0].resolvable), (MergeKind::Conflict, true));
    assert_eq!(resolve_simple("a b c", "a b c d", "a B c d").as_deref(), Some("a B c"));
    // the same word edited on both Sides
    let blocks = merge("a X c", "a b c", "a Y c", WhitespaceMode::Default);
    assert_eq!((blocks[0].kind, blocks[0].resolvable), (MergeKind::Conflict, false));
    // delete/modify and insert/insert are never resolvable, even when the words would merge
    assert!(!merge("z_Y", "x_Y", "Y", WhitespaceMode::Default)[0].resolvable);
    assert!(!merge("a_Y", "Y", "b_Y", WhitespaceMode::Default)[0].resolvable);
    assert!(merge("x", "x", "y", WhitespaceMode::Default).iter().all(|b| !b.resolvable));
}

#[test]
fn whitespace_only_blocks_are_marked_under_trim_and_ignore() {
    let ws_only = |l: &str, b: &str, r: &str, mode| merge(l, b, r, mode).iter().map(MergeBlock::whitespace_only).collect::<Vec<_>>();
    assert_eq!(ws_only("x ", "x", "x", WhitespaceMode::Default), [false]);
    assert_eq!(ws_only("x ", "x", "x", WhitespaceMode::Trim), [true]);
    assert_eq!(ws_only("x ", "x", "y", WhitespaceMode::Trim), [false]);
    // a blank line inserted: ignored by Ignore only, as for an Ignored Change
    assert_eq!(ws_only("a_ _b", "a_b", "a_b", WhitespaceMode::Ignore), [true]);
    assert_eq!(ws_only("a_ _b", "a_b", "a_b", WhitespaceMode::Trim), [false]);
    assert_eq!(ws_only("a_ _b", "a_b", "a_b", WhitespaceMode::IgnoreTrimChunks), [true]);
}

#[test]
#[allow(clippy::single_range_in_vec_init)] // byte ranges, not ranges to collect
fn inline_changes_in_a_merge_block_are_two_word_compares_against_base() {
    let options = MergeOptions::default();
    let got = merge_inline(Some("a X c"), Some("a b c"), Some("a b Y"), &options).unwrap();
    assert_eq!(got, MergeInline { left: vec![2..3], base: vec![2..3, 4..5], right: vec![4..5] });
    // a missing Side: the other two compared directly
    let got = merge_inline(Some("a X"), None, Some("a Y"), &options).unwrap();
    assert_eq!(got, MergeInline { left: vec![2..3], base: vec![], right: vec![2..3] });
    let got = merge_inline(None, Some("a b"), Some("a Y"), &options).unwrap();
    assert_eq!(got, MergeInline { left: vec![], base: vec![2..3], right: vec![2..3] });
    assert_eq!(merge_inline(Some("a"), None, None, &options), None);
    assert_eq!(merge_inline(None, None, None, &options), None);
    // under Ignore, Sides equal ignoring whitespace have nothing to show
    let ignore = MergeOptions { whitespace: WhitespaceMode::Ignore };
    assert_eq!(merge_inline(Some(" "), None, None, &ignore), Some(MergeInline::default()));
    assert_eq!(merge_inline(Some("a  b"), Some("a b"), Some("ab"), &ignore), Some(MergeInline::default()));
}

#[test]
fn the_resolver_gives_up_on_texts_over_the_token_cap() {
    // non-overlapping edits at both ends of a long text: resolvable below the cap, not above it
    let resolve = |words: usize| {
        let base: Vec<String> = (0..words).map(|i| format!("w{i}")).collect();
        let (mut left, mut right) = (base.clone(), base.clone());
        left[0] = "first".into();
        right[words - 1] = "last".into();
        resolve_simple(&left.join(" "), &base.join(" "), &right.join(" "))
    };
    let under = resolve(MAX_REFINE_TOKENS / 3);
    assert!(under.as_deref().is_some_and(|t| t.starts_with("first ") && t.ends_with(" last")));
    assert_eq!(resolve(MAX_REFINE_TOKENS / 3 + 1), None);
    // punctuation counts too: it is compared mark by mark
    let marks = ";".repeat(MAX_REFINE_TOKENS / 2);
    assert_eq!(resolve_simple(&format!("a{marks}"), &marks, &format!("{marks}b")), None);

    // a pathological Conflict is typed at once, not resolved
    let words = |tag: &str| (0..MAX_REFINE_TOKENS / 2).map(|i| format!("{tag}{}", i % 97)).collect::<Vec<_>>().join(" ");
    let (l, b, r) = (words("l"), words("b"), words("r"));
    let start = Instant::now();
    let blocks = merge_lines(&[l.as_str()], &[b.as_str()], &[r.as_str()], &MergeOptions::default());
    assert_eq!((blocks[0].kind, blocks[0].resolvable), (MergeKind::Conflict, false));
    assert!(start.elapsed().as_secs() < 2, "took {:?}", start.elapsed());
}

// ---------------------------------------------------------------- ComparisonUtilAutoTest.testMerge

/// DiffTestCase.generateText: newlines, tabs, spaces, punctuation, four letters, and with `wide` four
/// characters outside the BMP (IntelliJ's surrogate pairs; here multi-byte UTF-8).
fn generate(rnd: &mut impl FnMut(u64) -> u64, max_length: u64, wide: bool) -> String {
    const TABLE: [&str; 16] = ["\n", "\n", "\t", " ", " ", ".", "<", "!", "i", "j", "k", "l", "\u{1F602}", "\u{1F92B}", "\u{1F52B}", "\u{1F9D2}"];
    let n = if wide { 16 } else { 12 };
    (0..rnd(max_length + 1)).map(|_| TABLE[rnd(n) as usize]).collect()
}

fn equal(a: &str, b: &str, ignore_spaces: bool) -> bool {
    let squeeze = |s: &str| s.chars().filter(|c| !matches!(c, ' ' | '\t' | '\n')).collect::<String>();
    if ignore_spaces { squeeze(a) == squeeze(b) } else { a == b }
}

fn lines_content(lines: &[&str], start: usize, end: usize) -> String {
    lines[start..end].join("\n")
}

/// checkResultMerge: the line blocks, then a three-way word compare of each.
fn check_merge(texts: [&str; 3], mode: WhitespaceMode) -> Result<(), String> {
    let [t1, t2, t3] = texts.map(split_lines);
    let options = MergeOptions { whitespace: mode };
    let blocks = merge_lines(&t1, &t2, &t3, &options);
    let ignore_spaces = mode != WhitespaceMode::Default;
    let r = |b: &MergeBlock| [b.left.clone(), b.base.clone(), b.right.clone()].map(|r| r.start as usize..r.end as usize);

    // checkLineConsistency3 + checkValidRanges3 (lines)
    let mut last = [0; 3];
    let mut last_end: Option<[usize; 3]> = None;
    for b in &blocks {
        let [l, m, rr] = r(b);
        if l.start > l.end || m.start > m.end || rr.start > rr.end || l.end > t1.len() || m.end > t2.len() || rr.end > t3.len() {
            return Err(format!("bad block {b:?}"));
        }
        if l.is_empty() && m.is_empty() && rr.is_empty() {
            return Err(format!("empty block {b:?}"));
        }
        if !ignore_spaces && last_end == Some([l.start, m.start, rr.start]) {
            return Err(format!("squashed block {b:?}"));
        }
        let (c1, c2, c3) = (lines_content(&t1, last[0], l.start), lines_content(&t2, last[1], m.start), lines_content(&t3, last[2], rr.start));
        if !equal(&c2, &c1, ignore_spaces) || !equal(&c2, &c3, ignore_spaces) {
            return Err(format!("unequal lines before {b:?}"));
        }
        // checkCantTrimLines3 (Default only): a block's first and last lines are not equal on all three Sides
        if !ignore_spaces && !l.is_empty() && !m.is_empty() && !rr.is_empty() {
            for (i1, i2, i3) in [(l.start, m.start, rr.start), (l.end - 1, m.end - 1, rr.end - 1)] {
                if t2[i2] == t1[i1] && t2[i2] == t3[i3] {
                    return Err(format!("trimmable block {b:?}"));
                }
            }
        }
        // our own: the flags are consistent with the kind
        if b.resolvable && b.kind != MergeKind::Conflict || b.whitespace_only() && (b.kind == MergeKind::Conflict || !ignore_spaces) {
            return Err(format!("inconsistent flags {b:?}"));
        }
        last = [l.end, m.end, rr.end];
        last_end = Some(last);

        check_words([&lines_content(&t1, l.start, l.end), &lines_content(&t2, m.start, m.end), &lines_content(&t3, rr.start, rr.end)], mode)?;
    }
    let (c1, c2, c3) = (lines_content(&t1, last[0], t1.len()), lines_content(&t2, last[1], t2.len()), lines_content(&t3, last[2], t3.len()));
    if !equal(&c2, &c1, ignore_spaces) || !equal(&c2, &c3, ignore_spaces) {
        return Err("unequal lines after the last block".into());
    }
    Ok(())
}

/// checkDiffConsistency3 + checkValidRanges3 (words) for one block; char boundaries stand in for checkCodePoints.
fn check_words(chunks: [&str; 3], mode: WhitespaceMode) -> Result<(), String> {
    let ranges = golden::merge_words(chunks[0], chunks[1], chunks[2], &MergeOptions { whitespace: mode });
    let (unchanged_spaces, changed_spaces) = (mode != WhitespaceMode::Default, mode == WhitespaceMode::Ignore);
    let mut last = [0; 3];
    let mut last_end: Option<[usize; 3]> = None;
    for w in &ranges {
        let (s, e) = ([w[0], w[2], w[4]], [w[1], w[3], w[5]]);
        if (0..3).any(|i| s[i] > e[i] || s[i] < last[i] || !chunks[i].is_char_boundary(s[i]) || !chunks[i].is_char_boundary(e[i])) {
            return Err(format!("bad word range {w:?} in {chunks:?}"));
        }
        if s == e || last_end == Some(s) {
            return Err(format!("empty or squashed word range {w:?} in {chunks:?}"));
        }
        let gap = |i: usize| &chunks[i][last[i]..s[i]];
        if !equal(gap(1), gap(0), unchanged_spaces) || !equal(gap(1), gap(2), unchanged_spaces) {
            return Err(format!("unequal words before {w:?} in {chunks:?}"));
        }
        let part = |i: usize| &chunks[i][s[i]..e[i]];
        if equal(part(1), part(0), changed_spaces) && equal(part(1), part(2), changed_spaces) {
            return Err(format!("equal word range {w:?} in {chunks:?}"));
        }
        last = e;
        last_end = Some(e);
    }
    let tail = |i: usize| &chunks[i][last[i]..];
    if !equal(tail(1), tail(0), unchanged_spaces) || !equal(tail(1), tail(2), unchanged_spaces) {
        return Err(format!("unequal words after the last range in {chunks:?}"));
    }
    // and the resolver and Inline Changes cope with whatever the block holds
    let _ = resolve_simple(chunks[0], chunks[1], chunks[2]);
    let inline = merge_inline(Some(chunks[0]), Some(chunks[1]), Some(chunks[2]), &MergeOptions { whitespace: mode });
    for (i, side) in inline.iter().flat_map(|m| [&m.left, &m.base, &m.right]).enumerate() {
        if side.iter().any(|r| !chunks[i].is_char_boundary(r.start) || !chunks[i].is_char_boundary(r.end)) {
            return Err(format!("inline change off a char boundary in {chunks:?}"));
        }
    }
    Ok(())
}

#[test]
fn merge_holds_intellijs_invariants_on_random_texts() {
    let mut seed: u64 = 0x5eed;
    let mut rnd = move |m: u64| {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) % m
    };
    let modes = [WhitespaceMode::Default, WhitespaceMode::Trim, WhitespaceMode::Ignore];
    let mut failures = Vec::new();
    for run in 0..100 {
        for wide in [true, false] {
            let texts = [generate(&mut rnd, 300, wide), generate(&mut rnd, 300, wide), generate(&mut rnd, 300, wide)];
            // IntelliJ's doTest3 passes base as right too; both ways are checked here
            for texts in [[&*texts[0], &*texts[1], &*texts[2]], [&*texts[0], &*texts[1], &*texts[1]]] {
                for mode in modes {
                    if let Err(e) = check_merge(texts, mode) {
                        failures.push(format!("run {run} {mode:?} {texts:?}: {e}"));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{} failed:\n{}", failures.len(), failures[..failures.len().min(10)].join("\n"));
}
