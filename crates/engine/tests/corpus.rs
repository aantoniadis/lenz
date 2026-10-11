//! Snapshots of Change Blocks and Inline Changes over real-world file pairs (see `corpus/README.md`).
//! Each block shows its line ranges and both Sides' lines, Inline Changes marked `[-…-]` / `{+…+}`.

use std::ops::Range;
use std::path::PathBuf;

use lenz_engine::{Budget, ChangeBlock, HighlightLevel, Options, Refinement, WhitespaceMode, compare, compare_lines, refine, split_lines};

fn pairs() -> Vec<(String, String, String)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/corpus");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    dirs.retain(|p| p.join("left").exists());
    dirs.sort();
    dirs.iter()
        .map(|d| {
            let read = |f: &str| std::fs::read_to_string(d.join(f)).unwrap();
            (d.file_name().unwrap().to_string_lossy().into_owned(), read("left"), read("right"))
        })
        .collect()
}

fn block_text(lines: &[&str], range: &Range<u32>) -> String {
    lines[range.start as usize..range.end as usize].join("\n")
}

/// Panics unless the blocks are ordered, in bounds, and every Inline Change lies on char boundaries of its block text.
fn check(left: &[&str], right: &[&str], blocks: &[ChangeBlock]) {
    let (mut l, mut r) = (0, 0);
    for b in blocks {
        assert!(l <= b.left.start && b.left.start <= b.left.end && b.left.end as usize <= left.len(), "{b:?}");
        assert!(r <= b.right.start && b.right.start <= b.right.end && b.right.end as usize <= right.len(), "{b:?}");
        (l, r) = (b.left.end, b.right.end);
        if let Some(inline) = &b.inline {
            assert_eq!(b.ignored, inline.is_empty());
            let (bl, br) = (block_text(left, &b.left), block_text(right, &b.right));
            for c in inline {
                assert!(c.left.start <= c.left.end && c.right.start <= c.right.end, "{c:?}");
                assert!(bl.is_char_boundary(c.left.start) && bl.is_char_boundary(c.left.end), "{c:?} in {bl:?}");
                assert!(br.is_char_boundary(c.right.start) && br.is_char_boundary(c.right.end), "{c:?} in {br:?}");
            }
        }
    }
}

fn mark(text: &str, ranges: impl Iterator<Item = Range<usize>>, open: &str, close: &str) -> String {
    let mut out = String::new();
    let mut at = 0;
    for r in ranges.filter(|r| !r.is_empty()) {
        out += &text[at..r.start];
        out += open;
        out += &text[r.clone()];
        out += close;
        at = r.end;
    }
    out + &text[at..]
}

fn render(left: &[&str], right: &[&str], blocks: &[ChangeBlock]) -> String {
    let mut out = String::new();
    for b in blocks {
        let ignored = if b.ignored { " (ignored)" } else { "" };
        out += &format!("@@ L{}..{} R{}..{}{ignored}\n", b.left.start + 1, b.left.end + 1, b.right.start + 1, b.right.end + 1);
        let inline = b.inline.as_deref().unwrap_or_default();
        let (bl, br) = (block_text(left, &b.left), block_text(right, &b.right));
        if !b.left.is_empty() {
            mark(&bl, inline.iter().map(|c| c.left.clone()), "[-", "-]").lines().for_each(|l| out += &format!("-{l}\n"));
        }
        if !b.right.is_empty() {
            mark(&br, inline.iter().map(|c| c.right.clone()), "{+", "+}").lines().for_each(|l| out += &format!("+{l}\n"));
        }
    }
    out
}

#[test]
fn corpus_change_blocks_and_inline_changes() {
    for (name, l, r) in pairs() {
        let (left, right) = (split_lines(&l), split_lines(&r));
        let blocks = compare(&left, &right, &Options::default());
        check(&left, &right, &blocks);
        insta::assert_snapshot!(name, render(&left, &right, &blocks));
    }
}

#[test]
fn corpus_holds_its_invariants_in_every_mode() {
    let modes = [WhitespaceMode::Default, WhitespaceMode::Trim, WhitespaceMode::Ignore, WhitespaceMode::IgnoreTrimChunks];
    for (_, l, r) in pairs() {
        let (left, right) = (split_lines(&l), split_lines(&r));
        for whitespace in modes {
            for highlight in [HighlightLevel::Word, HighlightLevel::Char, HighlightLevel::Line] {
                for ignore_case in [false, true] {
                    check(&left, &right, &compare(&left, &right, &Options { whitespace, ignore_case, highlight }));
                }
            }
        }
    }
}

#[test]
fn generated_unicode_and_control_text_keeps_valid_ranges_and_exact_reconstruction() {
    let alphabet = ['a', 'A', 'ß', 'İ', '\u{301}', '漢', '👩', '\u{200d}', '\0', '\x1b', ' ', '\t', '\n', '\r'];
    let mut seed = 0x1e02_2026_u64;
    let mut next = || { seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1); (seed >> 32) as usize };
    for _ in 0..1_000 {
        let texts: [String; 2] = std::array::from_fn(|_| (0..next()%50).map(|_| alphabet[next()%alphabet.len()]).collect());
        let [left, right] = texts.each_ref().map(|s| split_lines(s));
        for whitespace in [WhitespaceMode::Default, WhitespaceMode::Trim, WhitespaceMode::Ignore, WhitespaceMode::IgnoreTrimChunks] {
            for highlight in [HighlightLevel::Word, HighlightLevel::Char, HighlightLevel::Line] {
                for ignore_case in [false, true] {
                    let blocks = compare(&left, &right, &Options { whitespace, highlight, ignore_case });
                    check(&left, &right, &blocks);
                    if whitespace == WhitespaceMode::Default && !ignore_case {
                        let mut rebuilt = Vec::new();
                        let mut at = 0;
                        for b in &blocks {
                            rebuilt.extend_from_slice(&left[at..b.left.start as usize]);
                            rebuilt.extend_from_slice(&right[b.right.start as usize..b.right.end as usize]);
                            at = b.left.end as usize;
                        }
                        rebuilt.extend_from_slice(&left[at..]);
                        assert_eq!(rebuilt, right, "{texts:?} {highlight:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn lines_first_then_refining_each_block_gives_what_compare_gives() {
    let modes = [WhitespaceMode::Default, WhitespaceMode::Trim, WhitespaceMode::Ignore, WhitespaceMode::IgnoreTrimChunks];
    for (name, l, r) in pairs() {
        let (left, right) = (split_lines(&l), split_lines(&r));
        for whitespace in modes {
            for highlight in [HighlightLevel::Word, HighlightLevel::Char, HighlightLevel::Line] {
                for ignore_case in [false, true] {
                    let options = Options { whitespace, ignore_case, highlight };
                    let mut budget = Budget::default();
                    let lazy: Vec<ChangeBlock> = compare_lines(&left, &right, &options)
                        .iter()
                        .filter_map(|b| match refine(&left, &right, b, &options, &mut budget) {
                            Refinement::Refined(b) => Some(b),
                            Refinement::Dropped => None,
                            Refinement::TooBig => panic!("{name}: no corpus block is too big"),
                        })
                        .collect();
                    assert_eq!(lazy, compare(&left, &right, &options), "{name} {options:?}");
                }
            }
        }
    }
}
