//! Shared, compile-time application information. Opening About performs no I/O.
pub const TITLE: &str = "About Lenz";
pub const DESCRIPTION: &str = "Side-by-side diff viewer and merge tool";
pub const ROWS: &[(&str, &str)] = &[
    ("Version", env!("CARGO_PKG_VERSION")),
    ("Platform", std::env::consts::OS),
    ("Architecture", std::env::consts::ARCH),
    ("Licence", "MIT; engine Apache-2.0"),
    ("Project", "https://github.com/aantoniadis/lenz"),
];
