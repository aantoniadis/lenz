//! Shared, compile-time application information. Opening About performs no I/O.
pub const TITLE: &str = "About Lenz";
pub const DESCRIPTION: &str = "Side-by-side diff viewer and merge tool";
pub use lenz_content::about::{version_line, wants_version, LOGO_PNG, LOGO_TERMINAL, ROWS};
