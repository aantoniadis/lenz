//! Lenz's themes: the shared implementation in [`lenz_content::theme`], Lenz's paths and environment overrides,
//! and diff colour mapping. The Markdown preview's colours are shared too.

pub use lenz_content::theme::*;

/// Lenz's theme files: `$XDG_CONFIG_HOME/lenz/themes`, `$XDG_CONFIG_HOME/lenz/appearance.json`, and Omarchy's
/// current theme.
pub fn paths_from_env() -> ThemePaths {
    ThemePaths::xdg("lenz")
}

/// `LENZ_THEME` (a theme ID or `system`) wins over `LENZ_APPEARANCE` (`light` or `dark`).
pub fn overrides_from_env() -> ThemeOverrides {
    ThemeOverrides::from_env("LENZ_THEME", "LENZ_APPEARANCE")
}

/// The shared change colors, fading ignored changes halfway towards the background.
pub fn change_color(palette: &Palette, kind: crate::view::Kind, ignored: bool, colors: &[u32; 4]) -> u32 {
    use crate::view::Kind;
    let color = colors[match kind { Kind::Inserted => 0, Kind::Deleted => 1, Kind::Modified => 2, Kind::Conflict => 3 }];
    if ignored { blend(color, palette.bg, 50) } else { color }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignored_changes_fade_halfway_to_the_background() {
        use crate::view::Kind;
        let p = &DARCULA;
        assert_eq!(change_color(p, Kind::Deleted, false, &p.block), p.block[1]);
        assert_eq!(change_color(p, Kind::Conflict, true, &p.inline), blend(p.inline[3], p.bg, 50));
    }
}
