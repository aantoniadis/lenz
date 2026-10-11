//! IntelliJ's three-way comparison and resolver golden tests (see `intellij_merge/cases.rs` for provenance),
//! checked the way ComparisonMergeUtilTestBase / ComparisonUtilTestBase do: range lists and/or matching bitsets.

mod intellij_merge {
    pub mod cases;
}

use intellij_merge::cases::{CASES, Case, Kind, RESOLVE};
use lenz_engine::{MergeOptions, golden, merge_lines, resolve_simple, split_lines};

/// IntelliJ's `parseLineMatching`: a line is changed if its segment holds anything but ' ' and '_'.
fn line_matching(pattern: &str) -> Vec<bool> {
    let mut out = Vec::new();
    let mut seg = String::new();
    for ch in pattern.chars() {
        seg.push(ch);
        if ch == '_' || ch == '*' {
            out.push(seg.chars().any(|c| c != ' ' && c != '_'));
            seg.clear();
        }
    }
    out.push(seg.chars().any(|c| c != ' ' && c != '_'));
    out
}

/// Units (lines or chars) touched by any range, per Side: left, base, right.
fn touched(ranges: &[[usize; 6]], lens: [usize; 3]) -> [Vec<bool>; 3] {
    let mut sets = lens.map(|n| vec![false; n]);
    for r in ranges {
        for (side, set) in sets.iter_mut().enumerate() {
            set[r[2 * side]..r[2 * side + 1]].iter_mut().for_each(|b| *b = true);
        }
    }
    sets
}

/// checkConsistency: ordered, none empty on all three Sides, and (`squash`) none starting where the previous one ended
/// on all three.
fn consistent(ranges: &[[usize; 6]], squash: bool) -> Result<(), String> {
    let mut last: Option<[usize; 6]> = None;
    for r in ranges {
        let starts = [r[0], r[2], r[4]];
        let ends = [r[1], r[3], r[5]];
        if (0..3).any(|i| starts[i] > ends[i] || last.is_some_and(|l| l[2 * i + 1] > starts[i])) {
            return Err(format!("out of order: {ranges:?}"));
        }
        if starts == ends {
            return Err(format!("empty range: {ranges:?}"));
        }
        if squash && last.is_some_and(|l| (0..3).all(|i| l[2 * i + 1] == starts[i])) {
            return Err(format!("squashed ranges: {ranges:?}"));
        }
        last = Some(*r);
    }
    Ok(())
}

fn run(case: &Case) -> Result<(), String> {
    let [l, b, r] = case.texts;
    let options = MergeOptions { whitespace: case.mode };
    let lines = || (split_lines(l), split_lines(b), split_lines(r));
    let (got, lens) = match case.kind {
        Kind::Chars => (golden::merge_chars(l, b, r), [l.len(), b.len(), r.len()]),
        Kind::Words => (golden::merge_words(l, b, r, &options), [l.len(), b.len(), r.len()]),
        Kind::LinesMerge | Kind::LinesDiff => {
            let (ll, bl, rl) = lines();
            let got = golden::merge_line_ranges(&ll, &bl, &rl, &options, case.kind == Kind::LinesMerge);
            if case.kind == Kind::LinesMerge {
                let blocks: Vec<[usize; 6]> = merge_lines(&ll, &bl, &rl, &options)
                    .iter()
                    .map(|m| [m.left.start, m.left.end, m.base.start, m.base.end, m.right.start, m.right.end].map(|x| x as usize))
                    .collect();
                if blocks != got {
                    return Err(format!("merge_lines blocks {blocks:?} differ from its ranges {got:?}"));
                }
            }
            (got, [ll.len(), bl.len(), rl.len()])
        }
    };
    // the line tests check no consistency: whitespace-only runs may adjoin a block
    if matches!(case.kind, Kind::Chars | Kind::Words) {
        consistent(&got, case.kind == Kind::Chars)?;
    }
    if let Some(expected) = case.changes
        && got != expected
    {
        return Err(format!("expected changes {expected:?}, got {got:?}"));
    }
    if let Some(matching) = case.matching {
        let want = if matches!(case.kind, Kind::LinesMerge | Kind::LinesDiff) {
            matching.map(line_matching)
        } else {
            matching.map(|m| m.chars().map(|c| c != ' ').collect())
        };
        let got = touched(&got, lens);
        if got != want {
            let render = |s: &[bool]| s.iter().map(|&b| if b { '-' } else { ' ' }).collect::<String>();
            return Err(format!("expected matching {matching:?}, got {:?}", got.each_ref().map(|s| render(s))));
        }
    }
    Ok(())
}

#[test]
fn intellij_merge_comparison_golden_tests() {
    let mut failures = Vec::new();
    for case in CASES {
        let result = std::panic::catch_unwind(|| run(case)).unwrap_or_else(|_| Err("panicked".into()));
        if let Err(e) = result {
            failures.push(format!("{} ({:?}): {e}", case.name, case.texts));
        }
    }
    assert!(failures.is_empty(), "{} of {} failed:\n{}", failures.len(), CASES.len(), failures.join("\n"));
}

#[test]
fn intellij_merge_resolve_golden_tests() {
    let mut failures = Vec::new();
    for case in RESOLVE {
        let got = resolve_simple(case.left, case.base, case.right);
        if got.as_deref() != case.expected {
            failures.push(format!("{}: expected {:?}, got {got:?}", case.name, case.expected));
        }
    }
    assert!(failures.is_empty(), "{} of {} failed:\n{}", failures.len(), RESOLVE.len(), failures.join("\n"));
}
