//! Content shared by Lenz and its code reader: decoding, line offsets, display transforms, syntax highlighting,
//! themes, fuzzy file-path ranking, in-document find, go to line and soft wrap. Free of either product's sessions,
//! workflow commands and GPUI; declares no global allocator.
#![deny(unsafe_code)] // audited FFI only: `highlight.rs`, `syntax_arena.rs` and the test-only `memory_probe.rs`
#[cfg(test)]
#[path = "../../../benches/support.rs"]
pub mod perf;
#[cfg(any(test, feature = "perf-probe"))]
pub mod memory_probe;
pub mod about;
pub mod appearance;
pub mod display;
pub mod encoding;
pub mod find;
pub mod file_icons;
pub mod fuzzy;
pub mod goto;
pub mod highlight;
pub mod image;
pub mod icons;
pub mod lines;
pub mod markdown;
pub mod open;
mod syntax_arena;
pub mod theme;
pub mod wrap;
