//! IntelliJ's word, char and splitter golden tests (see `intellij_inner/cases.rs` for provenance),
//! checked the way IntelliJ's ComparisonUtilTestBase does: changed-range lists and/or matching bitsets, in UTF-16 units.

mod intellij_inner {
    pub mod cases;
}

use intellij_inner::cases::{CASES, Case, Kind};
use lenz_engine::{ChangeBlock, HighlightLevel, InlineChange, Options, WhitespaceMode, compare, golden, split_lines};

fn utf16(text: &str, byte: usize) -> u32 {
    text[..byte].encode_utf16().count() as u32
}

/// IntelliJ's checkDiffChanges / checkDiffMatching on Inline Changes (byte offsets converted to UTF-16 units).
/// `l`/`r` is the text the offsets refer to: a Change Block's text lacks the newline after its last line,
/// so expectations that cover the text's final `\n` are clipped to it.
fn check_inline(case: &Case, label: &str, l: &str, r: &str, changes: &[InlineChange]) -> Result<(), String> {
    let (n1, n2) = (utf16(l, l.len()), utf16(r, r.len()));
    let clipped: Option<Vec<[u32; 4]>> = case.changes.map(|cs| {
        cs.iter()
            .map(|c| [c[0].min(n1), c[1].min(n1), c[2].min(n2), c[3].min(n2)])
            .filter(|c| c[0] != c[1] || c[2] != c[3])
            .collect()
    });
    let matching = case.matching.map(|(ml, mr)| (&ml[..n1 as usize], &mr[..n2 as usize]));
    let got: Vec<[u32; 4]> = changes
        .iter()
        .map(|c| [utf16(l, c.left.start), utf16(l, c.left.end), utf16(r, c.right.start), utf16(r, c.right.end)])
        .collect();
    if got.iter().any(|g| g[0] == g[1] && g[2] == g[3]) {
        return Err(format!("{label}: empty fragment in {got:?}"));
    }
    if let Some(expected) = clipped
        && got != expected
    {
        return Err(format!("{label}: expected changes {expected:?}, got {got:?}"));
    }
    if let Some((ml, mr)) = matching {
        let bits = |text: &str, pick: fn(&[u32; 4]) -> (u32, u32)| {
            let mut set = vec![false; text.encode_utf16().count()];
            for g in &got {
                let (s, e) = pick(g);
                set[s as usize..e as usize].iter_mut().for_each(|b| *b = true);
            }
            set
        };
        let parse = |m: &str| m.chars().map(|c| c != ' ').collect::<Vec<bool>>();
        let render = |s: &[bool]| s.iter().map(|&b| if b { '-' } else { ' ' }).collect::<String>();
        let (gl, gr) = (bits(l, |g| (g[0], g[1])), bits(r, |g| (g[2], g[3])));
        if gl != parse(ml) || gr != parse(mr) {
            return Err(format!("{label}: expected {ml:?} - {mr:?}, got {:?} - {:?}", render(&gl), render(&gr)));
        }
    }
    Ok(())
}

fn options(mode: WhitespaceMode) -> Options {
    Options { whitespace: mode, ignore_case: false, highlight: HighlightLevel::Word }
}

fn line_ranges(blocks: &[ChangeBlock]) -> Vec<[u32; 4]> {
    blocks.iter().map(|b| [b.left.start, b.left.end, b.right.start, b.right.end]).collect()
}

fn run(case: &Case) -> Result<(), String> {
    let (l, r, opts) = (case.left, case.right, options(case.mode));
    match case.kind {
        Kind::LinesInner => {
            // compareLinesInner + squash over the whole texts must give one block spanning all text offsets
            // (it may stop before an empty last line) and holding the Inline Changes
            let (ll, rl) = (split_lines(l), split_lines(r));
            let blocks = compare(&ll, &rl, &opts);
            let spans_all = |range: &std::ops::Range<u32>, lines: &[&str]| {
                range.start == 0 && (range.end as usize == lines.len() || range.end as usize == lines.len() - 1 && lines[lines.len() - 1].is_empty())
            };
            match blocks.as_slice() {
                [b] if spans_all(&b.left, &ll) && spans_all(&b.right, &rl) && b.inline.is_some() => {
                    let text = |lines: &[&str], range: &std::ops::Range<u32>| lines[..range.end as usize].join("\n");
                    let (bl, br) = (text(&ll, &b.left), text(&rl, &b.right));
                    check_inline(case, "lines_inner", &bl, &br, b.inline.as_ref().unwrap())?
                }
                _ => return Err(format!("lines_inner: expected one refined block over the whole texts, got {blocks:?}")),
            }
            check_inline(case, "words", l, r, &golden::words(l, r, &opts))
        }
        Kind::Words => check_inline(case, "words", l, r, &golden::words(l, r, &opts)),
        Kind::Chars | Kind::CharsSmart | Kind::CharsRaw => {
            if case.kind != Kind::CharsRaw {
                check_inline(case, "chars", l, r, &golden::chars(l, r, &opts))?;
            }
            if case.kind != Kind::CharsSmart && case.mode == WhitespaceMode::Default {
                check_inline(case, "chars_raw", l, r, &golden::chars_raw(l, r))?;
            }
            Ok(())
        }
        Kind::Splitter => {
            let (ll, rl) = (split_lines(l), split_lines(r));
            let blocks = if case.squash { compare(&ll, &rl, &opts) } else { golden::split(&ll, &rl, &opts) };
            let expected = case.changes.unwrap();
            let got = line_ranges(&blocks);
            if got != expected {
                return Err(format!("splitter: expected {expected:?}, got {got:?}"));
            }
            Ok(())
        }
    }
}

#[test]
fn intellij_inline_comparison_golden_tests() {
    let mut failures = Vec::new();
    let mut summary = Vec::new();
    for suite in ["Word", "Char", "Split"] {
        let cases: Vec<&Case> = CASES.iter().filter(|c| c.suite == suite).collect();
        let before = failures.len();
        for case in &cases {
            let result = std::panic::catch_unwind(|| run(case)).unwrap_or_else(|_| Err("panicked".into()));
            if let Err(e) = result {
                failures.push(format!("{} ({:?} / {:?}): {e}", case.name, case.left, case.right));
            }
        }
        summary.push(format!("{suite}: {}/{} passed", cases.len() - (failures.len() - before), cases.len()));
    }
    println!("{}", summary.join(", "));
    assert!(failures.is_empty(), "{}\n{}", summary.join(", "), failures.join("\n"));
}
