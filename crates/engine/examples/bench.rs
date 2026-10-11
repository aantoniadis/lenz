//! `cargo run --release -p lenz-engine --example bench -- <left> <right>`: time `compare` per Whitespace Mode
//! and Highlight Level (Word/Char minus Line is the cost of refining blocks into Inline Changes).
use std::time::Instant;

use lenz_engine::{HighlightLevel, Options, WhitespaceMode, compare, split_lines};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (l, r) = (std::fs::read_to_string(&args[0]).unwrap(), std::fs::read_to_string(&args[1]).unwrap());
    let (l, r) = (split_lines(&l), split_lines(&r));
    for whitespace in [WhitespaceMode::Default, WhitespaceMode::Trim, WhitespaceMode::Ignore, WhitespaceMode::IgnoreTrimChunks] {
        for highlight in [HighlightLevel::Line, HighlightLevel::Word, HighlightLevel::Char] {
            let t = Instant::now();
            let blocks = compare(&l, &r, &Options { whitespace, highlight, ..Options::default() });
            let inline: usize = blocks.iter().filter_map(|b| b.inline.as_ref()).map(Vec::len).sum();
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            println!("{whitespace:?}/{highlight:?}: {} blocks, {inline} Inline Changes in {ms:.1} ms", blocks.len());
        }
    }
}
