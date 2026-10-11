//! The Markdown Preview's reading style: Obsidian's layout and element styling, coloured from the Lenz theme.
use crate::markdown::{BlockKind, CalloutKind};
use crate::theme::{self, Palette};
use gpui::Font;

/// Proportional body fonts, first installed wins.
pub(super) const SANS_CANDIDATES: &[&str] = &["Inter", "Noto Sans", "DejaVu Sans", "Cantarell", "Liberation Sans"];

/// The configured reading font when installed, else the first installed candidate, else gpui's `sans-serif`.
pub(super) fn sans_family<'a>(configured: Option<&'a str>, installed: &'a [String]) -> &'a str {
    let has = |name: &str| installed.iter().any(|f| f == name);
    configured
        .filter(|n| has(n))
        .or_else(|| SANS_CANDIDATES.iter().copied().find(|c| has(c)))
        .unwrap_or("sans-serif")
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ReadingStyle {
    pub body: Font,
    pub mono: Font,
    /// Body text size and line height in pixels.
    pub size: f32,
    pub line: f32,
    /// The widest reading column.
    pub column: f32,
    /// Heading sizes, `#` to `######`.
    pub heading: [f32; 6],
    pub text: u32,
    pub heading_color: u32,
    pub link: u32,
    pub external: u32,
    pub code_bg: u32,
    pub code_fg: u32,
    pub muted: u32,
    pub rule: u32,
    pub tag_bg: u32,
    pub tag_fg: u32,
    pub highlight: u32,
    pub quote_bar: u32,
    pub check: u32,
    pub added: u32,
    pub modified: u32,
    pub removed: u32,
}

impl ReadingStyle {
    /// `base` is the configured code font size; body text is two pixels larger.
    pub fn new(p: &Palette, mono: Font, body: Font, base: f32) -> Self {
        let size = base + 2.;
        let dark = theme::luminance(p.bg) < 0.5;
        ReadingStyle {
            body,
            mono,
            size,
            line: size * 1.6,
            // about 70 characters of proportional text
            column: size * 40.,
            heading: [1.8, 1.55, 1.3, 1.12, 1.0, 1.0].map(|k| k * size),
            text: p.fg,
            heading_color: theme::mix(p.fg, if dark { 0xffffff } else { 0x000000 }, 0.3),
            link: p.accent,
            external: p.syntax.function,
            code_bg: theme::mix(p.bg, p.fg, 0.07),
            code_fg: p.syntax.string,
            muted: p.dim,
            rule: p.border,
            tag_bg: theme::mix(p.bg, p.syntax.keyword, 0.18),
            tag_fg: p.syntax.keyword,
            highlight: theme::highlight_bg(p),
            quote_bar: p.accent,
            check: p.accent,
            added: p.status[0],
            removed: theme::removed_color(p),
            modified: p.status[2],
        }
    }

    /// A callout's background tint, title colour and icon.
    pub fn callout(&self, p: &Palette, kind: CalloutKind) -> (u32, u32, &'static str) {
        let color = theme::callout_color(p, kind);
        (theme::mix(p.bg, color, 0.12), color, theme::callout_icon(kind))
    }

    /// Space above a block given the block before it on the same side (`None` at the top).
    pub fn space_before(&self, kind: &BlockKind, previous: Option<&BlockKind>) -> f32 {
        let Some(previous) = previous else { return 0. };
        let em = match (previous, kind) {
            (_, BlockKind::Heading(1)) => 1.5,
            (_, BlockKind::Heading(2)) => 2.2,
            (_, BlockKind::Heading(3)) => 1.7,
            (_, BlockKind::Heading(_)) => 1.35,
            (BlockKind::Heading(_), _) => 0.5,
            (BlockKind::ListItem { .. }, BlockKind::ListItem { .. }) => 0.2,
            (BlockKind::TableRow { .. }, BlockKind::TableRow { .. }) => 0.,
            (BlockKind::Footnote { .. }, BlockKind::Footnote { .. }) => 0.3,
            (_, BlockKind::Footnote { .. }) => 2.,
            (_, BlockKind::Rule) | (BlockKind::Rule, _) => 1.4,
            _ => 1.,
        };
        em * self.size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_style_follows_the_theme_and_scale() {
        for p in [&theme::DARCULA, &theme::LIGHT] {
            let s = ReadingStyle::new(p, gpui::font("monospace"), gpui::font("sans-serif"), 13.);
            assert_eq!(s.size, 15.);
            assert!(s.heading[0] > s.heading[1] && s.heading[1] > s.heading[2] && s.heading[3] > s.size);
            assert!((s.line / s.size - 1.6).abs() < 0.01);
            assert!(s.column >= 35. * s.size && s.column <= 45. * s.size);
            assert_ne!(s.link, s.external);
            assert_ne!(s.code_bg, p.bg);
            let (tint, title, icon) = s.callout(p, CalloutKind::Warning);
            assert_ne!(tint, p.bg);
            assert_ne!(title, p.fg);
            assert_eq!(icon, "⚠");
            assert_ne!(s.callout(p, CalloutKind::Note).1, title, "notes and warnings differ");
            assert_ne!(s.added, s.removed);
            let lighter = theme::luminance(s.heading_color) > theme::luminance(p.fg);
            assert_eq!(lighter, theme::luminance(p.bg) < 0.5, "headings stand out from body text");
        }
    }

    #[test]
    fn headings_get_more_space_above_than_below() {
        let s = ReadingStyle::new(&theme::DARCULA, gpui::font("monospace"), gpui::font("sans-serif"), 13.);
        let p = BlockKind::Paragraph;
        assert_eq!(s.space_before(&p, None), 0.);
        assert!(s.space_before(&BlockKind::Heading(2), Some(&p)) > s.space_before(&p, Some(&BlockKind::Heading(2))));
        assert!(s.space_before(&p, Some(&p)) >= s.size);
    }

    #[test]
    fn the_reading_font_is_configured_installed_or_a_fallback() {
        let installed = ["DejaVu Sans".to_string(), "Inter".to_string()];
        assert_eq!(sans_family(Some("DejaVu Sans"), &installed), "DejaVu Sans");
        assert_eq!(sans_family(Some("Missing"), &installed), "Inter");
        assert_eq!(sans_family(None, &[]), "sans-serif");
    }
}
