//! Shared, compile-time application information for every About. Opening About performs no I/O.
pub const ROWS: &[(&str, &str)] = &[
    ("Version", env!("CARGO_PKG_VERSION")),
    ("Platform", std::env::consts::OS),
    ("Architecture", std::env::consts::ARCH),
    ("Licence", "MIT; engine Apache-2.0"),
    ("Project", "https://github.com/aantoniadis/lenz"),
];

/// The About logo for the GUIs: a 96×96 PNG, decoded only while About is open.
pub const LOGO_PNG: &[u8] = include_bytes!("../../../assets/lenz-about.png");
/// The About logo for terminals: 24×24 RGBA pixels, drawn as half-block cells.
pub const LOGO_TERMINAL: &[u8] = include_bytes!("../../../assets/lenz-about-terminal.rgba");
/// The terminal logo's width and height in pixels.
pub const LOGO_TERMINAL_SIZE: usize = 24;

#[cfg(test)]
mod tests {
    #[test]
    fn rows_name_the_workspace_version_and_the_logos_have_their_sizes() {
        assert_eq!(super::ROWS[0], ("Version", env!("CARGO_PKG_VERSION")));
        assert_eq!(super::LOGO_TERMINAL.len(), super::LOGO_TERMINAL_SIZE * super::LOGO_TERMINAL_SIZE * 4);
        assert!(super::LOGO_PNG.starts_with(b"\x89PNG"));
    }
}
