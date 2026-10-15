#![deny(unsafe_code)] // audited FFI only: `dock_icon.rs` and `large_blocks.rs` (lenz-content audits its own)
#[cfg(test)]
#[path = "../../../benches/support.rs"]
pub mod perf;
#[cfg(all(test, feature = "gui"))]
mod perf_tests;
#[cfg(any(test, feature = "perf-probe"))]
pub use lenz_content::memory_probe;
pub mod blame;
pub mod changeset;
pub mod cli;
pub mod conflicts;
pub use lenz_content::{encoding, highlight};
pub mod document;
pub mod find;
/// Go-to-line input, from the shared content crate.
pub use lenz_content::goto;
pub mod git;
#[cfg(all(target_os = "macos", feature = "gui"))]
pub mod dock_icon;
#[cfg(target_os = "macos")]
pub mod large_blocks;
/// The core tests run on the binaries' allocator.
#[cfg(all(test, target_os = "macos"))]
#[global_allocator]
static ALLOCATOR: large_blocks::LargeBlocks = large_blocks::LargeBlocks;
pub mod history;
pub mod images;
pub mod markdown;
pub mod merge;
pub mod moves;
pub mod navigator;
pub mod open;
pub mod place;
pub mod pr;
pub mod repo_picker;
pub mod repo;
pub mod selection;
pub mod selection_title;
pub mod commit_message;
pub mod rich;
pub mod pr_conversation;
pub mod settings;
pub mod sticky;
pub mod theme;
#[cfg(feature = "gui")]
mod ui;
pub mod view;
pub mod write;
pub mod work;

#[cfg(feature = "gui")]
pub mod gui;
pub mod commands;
pub mod diagnostics;
pub mod about;
pub mod session;
pub mod comparison_session;
pub mod image_session;
pub mod merge_session;
pub mod version_session;
pub mod conflict_session;
pub mod repository_session;
pub mod pr_session;
pub mod edit_session;
pub mod file_actions;
pub mod annotation_session;

pub mod search_session;

pub mod pane_layout;
pub mod pane_search;
pub mod layout;
pub mod catalog;
pub mod palette;
pub mod navigation;
