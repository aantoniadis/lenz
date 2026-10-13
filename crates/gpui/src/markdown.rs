//! Single Markdown documents presented with GPUI, shared by Lenz's Preview and the code reader: the reading
//! style (Obsidian's layout and element styling, coloured from the theme), a block's text shaped with its inline
//! styles and code blocks' syntax colours, and block indentation. Each product lays out rows, scrolls and
//! decorates on top (Lenz pairs two documents for its comparisons).

use std::ops::Range;

use gpui::{Font, ShapedLine, TextRun, px, rgb};
use lenz_content::{
    highlight::Capture,
    markdown::{Block, BlockKind, CalloutKind, MarkdownDocument},
    theme::{self, Palette},
};


/// Proportional body fonts, first installed wins.
pub const SANS_CANDIDATES: &[&str] = &["Inter", "Noto Sans", "DejaVu Sans", "Cantarell", "Liberation Sans"];

/// The configured reading font when installed, else the first installed candidate, else gpui's `sans-serif`.
pub fn sans_family<'a>(configured: Option<&'a str>, installed: &'a [String]) -> &'a str {
    let has = |name: &str| installed.iter().any(|f| f == name);
    configured
        .filter(|n| has(n))
        .or_else(|| SANS_CANDIDATES.iter().copied().find(|c| has(c)))
        .unwrap_or("sans-serif")
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReadingStyle {
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

/// Quotes that are not callouts.
pub fn quotes(block: &Block) -> u16 {
    block.quote_depth.saturating_sub(u16::from(block.callout.is_some()))
}
/// Where a block's text starts: inside callout padding, quote bars and list levels.
pub fn indent(style: &ReadingStyle, left: f32, block: &Block) -> f32 {
    let pad = if block.callout.is_some() { style.size } else { 0. };
    // a footnote's number hangs in front of its text
    let note = if matches!(block.kind, BlockKind::Footnote { .. }) { style.size * 1.6 } else { 0. };
    left + pad + note + quotes(block) as f32 * style.size + block.list_depth as f32 * LIST_STEP
}
/// Width of one list level.
pub const LIST_STEP: f32 = 22.;
#[allow(clippy::too_many_arguments)]
pub fn shape(
    doc: &MarkdownDocument,
    block: &Block,
    range: Range<usize>,
    font_size: f32,
    style: &ReadingStyle,
    p: &theme::Palette,
    text_system: &gpui::WindowTextSystem,
    syntax: &[(Range<usize>, Capture)],
    color: Option<u32>,
) -> ShapedLine {
    shape_with(doc, block, range, font_size, style, p, text_system, syntax, color, false)
}
/// [`shape`], struck through and muted when `struck` (Inline's removed text).
#[allow(clippy::too_many_arguments)]
pub fn shape_with(
    doc: &MarkdownDocument,
    block: &Block,
    range: Range<usize>,
    font_size: f32,
    style: &ReadingStyle,
    p: &theme::Palette,
    text_system: &gpui::WindowTextSystem,
    syntax: &[(Range<usize>, Capture)],
    color: Option<u32>,
    struck: bool,
) -> ShapedLine {
    // a list item's synthetic marker is muted; a task's `[ ] ` is drawn as a checkbox instead
    let (marker_end, task) = match &block.kind {
        BlockKind::ListItem { marker, checked } => {
            let m = block.text.start + marker.len() + 1;
            (m, checked.map(|_| m..m + 4))
        }
        _ => (block.text.start, None),
    };
    let mut cuts = vec![range.start, range.end];
    for at in [Some(marker_end), task.as_ref().map(|t| t.end)].into_iter().flatten() {
        if range.contains(&at) {
            cuts.push(at);
        }
    }
    let start = block.runs.partition_point(|r| r.text.end <= range.start);
    for run in block.runs[start..]
        .iter()
        .take_while(|r| r.text.start < range.end)
    {
        if run.text.start < range.end && run.text.end > range.start {
            cuts.push(run.text.start.max(range.start));
            cuts.push(run.text.end.min(range.end));
        }
    }
    for (r, _) in syntax
        .iter()
        .skip(syntax.partition_point(|(r, _)| r.end <= range.start))
        .take_while(|(r, _)| r.start < range.end)
    {
        cuts.push(r.start.max(range.start));
        cuts.push(r.end.min(range.end));
    }
    cuts.sort_unstable();
    cuts.dedup();
    let runs: Vec<_> = cuts
        .windows(2)
        .map(|r| {
            let run = block
                .runs
                .get(block.runs.partition_point(|s| s.text.end <= r[0]))
                .filter(|s| s.text.start <= r[0] && s.text.end >= r[1])
                .map(|s| s.style)
                .unwrap_or_default();
            let inline_code = run.code && !matches!(block.kind, BlockKind::Code { .. } | BlockKind::Literal | BlockKind::Properties);
            let mono = inline_code || matches!(block.kind, BlockKind::Code { .. } | BlockKind::Literal);
            let mut f = if mono { style.mono.clone() } else { style.body.clone() };
            if run.strong
                || run.callout_title
                || run.footnote
                || matches!(block.kind, BlockKind::Heading(_) | BlockKind::TableRow { header: true, .. })
            {
                f.weight = gpui::FontWeight::BOLD;
            }
            if run.emphasis {
                f.style = gpui::FontStyle::Italic;
            }
            let done = matches!(block.kind, BlockKind::ListItem { checked: Some(true), .. });
            let syntax_color = || {
                syntax
                    .get(syntax.partition_point(|(s, _)| s.end <= r[0]))
                    .filter(|(s, _)| s.start <= r[0])
                    .map(|(_, c)| p.syntax_color(*c))
            };
            let color = if let Some(c) = color {
                c
            } else if struck || r[0] < marker_end {
                style.muted
            } else if run.callout_title {
                block.callout.map_or(style.text, |c| style.callout(p, c.kind).1)
            } else if run.tag {
                style.tag_fg
            } else if run.link && run.external {
                style.external
            } else if run.link || run.footnote {
                style.link
            } else if inline_code {
                style.code_fg
            } else if let Some(c) = syntax_color() {
                c
            } else if done || matches!(block.kind, BlockKind::Footnote { .. }) {
                style.muted
            } else if matches!(block.kind, BlockKind::Heading(_)) {
                style.heading_color
            } else {
                style.text
            };
            TextRun {
                len: r[1] - r[0],
                font: f,
                color: if task.as_ref().is_some_and(|t| t.contains(&r[0])) || matches!(block.kind, BlockKind::Rule) {
                    gpui::transparent_black()
                } else {
                    rgb(color).into()
                },
                background_color: run.highlight.then(|| rgb(style.highlight).into()),
                underline: (run.link && run.external).then_some(gpui::UnderlineStyle {
                    thickness: px(1.),
                    color: None,
                    wavy: false,
                }),
                strikethrough: (struck || run.strike || (done && r[0] >= task.as_ref().map_or(0, |t| t.end))).then_some(gpui::StrikethroughStyle {
                    thickness: px(1.),
                    color: None,
                }),
            }
        })
        .collect();
    text_system.shape_line(
        lenz_content::display::show_bidi(&doc.text[range]).into_owned().into(),
        px(font_size),
        &runs,
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use lenz_content::theme;

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
