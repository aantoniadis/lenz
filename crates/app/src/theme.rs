//! Lenz's themes: the shared implementation in [`lenz_content::theme`], canonical paths and environment overrides,
//! and diff colour mapping. The Markdown preview's colours are shared too.

pub use lenz_content::theme::*;

/// Lenz's theme files: `$XDG_CONFIG_HOME/lenz/themes`, `$XDG_CONFIG_HOME/lenz/appearance.json`, and Omarchy's
/// current theme.
pub fn paths_from_env() -> ThemePaths {
    ThemePaths::shared()
}

/// Shared overrides: LENZ_THEME > LENZ_APPEARANCE > LENZ_CODE_THEME > LENZ_CODE_APPEARANCE > LENZ_READER_*.
pub fn overrides_from_env() -> ThemeOverrides {
    ThemeOverrides::shared_from_env()
}

/// The shared change colors, fading ignored changes halfway towards the background.
pub fn change_color(palette: &Palette, kind: crate::view::Kind, ignored: bool, colors: &[u32; 4]) -> u32 {
    use crate::view::Kind;
    let color = colors[match kind { Kind::Inserted => 0, Kind::Deleted => 1, Kind::Modified => 2, Kind::Conflict => 3 }];
    if ignored { blend(color, palette.bg, 50) } else { color }
}

/// A Diff scrollbar mark's colour, shared by the window and the terminal app: a Change Block's (or merge block's) edge
/// colour by kind, the current block the accent, find hits the find highlight lifted toward the text (the current
/// one more: bare `found_bg` is under 1.5:1 on the thumb), so both stand out on the track and the thumb. Kinds Diff does not produce map as Lenz Code's do.
pub fn scrollbar_mark_color(kind: crate::scrollbar::MarkKind, p: &Palette) -> u32 {
    use crate::scrollbar::MarkKind;
    match kind {
        MarkKind::Added => p.edge[0],
        MarkKind::Deleted => p.edge[1],
        MarkKind::Modified => p.edge[2],
        MarkKind::Conflict => p.edge[3],
        MarkKind::Current | MarkKind::Warning | MarkKind::Bookmark => p.accent,
        MarkKind::FindCurrent => mix(p.found_bg, p.fg, 0.6),
        MarkKind::Find => mix(p.found_bg, p.fg, 0.4),
        MarkKind::Error => p.error,
        MarkKind::Caret => p.dim,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every mark a click can reach stands out from the track and from the thumb it may sit on, in both built-in palettes.
    #[test]
    fn scrollbar_marks_contrast_with_the_track_and_the_thumb() {
        use crate::scrollbar::{thumb_color, track_color, MarkKind::*};
        for (name, p) in [("Darcula", &DARCULA), ("Light", &LIGHT)] {
            for kind in [Error, Conflict, Warning, Current, FindCurrent, Find, Added, Deleted, Modified, Bookmark] {
                let c = scrollbar_mark_color(kind, p);
                for (under, bg) in [("track", track_color(p)), ("thumb", thumb_color(p))] {
                    assert!(contrast(c, bg) >= 1.8, "{name} {kind:?} on the {under}: {:.2}", contrast(c, bg));
                }
            }
            assert!(contrast(scrollbar_mark_color(FindCurrent, p), track_color(p)) > contrast(scrollbar_mark_color(Find, p), track_color(p)), "{name}: the current match is stronger");
        }
    }

    #[test]
    fn ignored_changes_fade_halfway_to_the_background() {
        use crate::view::Kind;
        let p = &DARCULA;
        assert_eq!(change_color(p, Kind::Deleted, false, &p.block), p.block[1]);
        assert_eq!(change_color(p, Kind::Conflict, true, &p.inline), blend(p.inline[3], p.bg, 50));
    }
}
