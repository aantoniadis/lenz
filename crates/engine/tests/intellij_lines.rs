//! IntelliJ's line-comparison golden tests (see `intellij_lines/cases.rs` for provenance).

mod intellij_lines {
    pub mod cases;
}

use intellij_lines::cases::{CASES, Expect, options};
use lenz_engine::{compare, split_lines};

/// Lines touched by any Change Block, per Side, as in IntelliJ's `checkLineMatching`.
fn touched(blocks: &[lenz_engine::ChangeBlock], left_len: usize, right_len: usize) -> (Vec<bool>, Vec<bool>) {
    let (mut l, mut r) = (vec![false; left_len], vec![false; right_len]);
    for b in blocks {
        b.left.clone().for_each(|i| l[i as usize] = true);
        b.right.clone().for_each(|i| r[i as usize] = true);
    }
    (l, r)
}

/// IntelliJ's `parseLineMatching`: a line is changed if its segment holds anything but ' ' and '_'.
fn parse_matching(pattern: &str) -> Vec<bool> {
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

#[test]
fn intellij_line_comparison_golden_tests() {
    let mut failures = Vec::new();
    for case in CASES {
        let (left, right) = (split_lines(case.left), split_lines(case.right));
        let blocks = compare(&left, &right, &options(case.mode));
        let ok = match &case.expect {
            Expect::Changes(expected) => {
                let got: Vec<[u32; 4]> =
                    blocks.iter().map(|b| [b.left.start, b.left.end, b.right.start, b.right.end]).collect();
                let pass = got == *expected;
                if !pass {
                    failures.push(format!("{}: expected {:?}, got {:?}", case.name, expected, got));
                }
                pass
            }
            Expect::Matching(l, r) => {
                let got = touched(&blocks, left.len(), right.len());
                let want = (parse_matching(l), parse_matching(r));
                let pass = got == want;
                if !pass {
                    failures.push(format!("{}: expected {:?}, got {:?}", case.name, want, got));
                }
                pass
            }
        };
        let _ = ok;
    }
    assert!(failures.is_empty(), "{} of {} failed:\n{}", failures.len(), CASES.len(), failures.join("\n"));
}
