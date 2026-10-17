//! Single-pane text presentation shared by Lenz and its code reader: shaping a line with syntax and background
//! spans, painting numbered rows with selection and caret, hit-testing a pointer to a source byte, picking the
//! monospace font, and converting theme colours for GPUI. It takes content, viewport geometry and decorations
//! and returns hit locations; each product computes its own rows, decorations and commands.
#![deny(unsafe_code)]

pub mod find;
pub mod image;
pub mod markdown;

use std::ops::Range;

use gpui::{
    App, Bounds, ContentMask, Font, Hsla, Pixels, ShapedLine, SharedString, TextAlign, TextRun, Window, font, point, px,
    quad, rgb, size,
};
use lenz_content::{display, theme::Palette};

/// A theme colour (`0xRRGGBB`) for GPUI.
pub fn color(c: u32) -> Hsla {
    Hsla::from(rgb(c))
}

/// Text geometry for one pane: the monospace font, its size, the row height and the line-number column width.
#[derive(Clone, Copy)]
pub struct Metrics<'a> {
    pub font: &'a Font,
    pub font_size: f32,
    pub line_h: f32,
    pub num_w: f32,
}

/// One numbered source row, already in display coordinates ([`display::display_line`]): `fg` colours text, `bg`
/// fills behind it (later entries in `bg` lose to earlier ones), `selected` and `caret` are display offsets.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextRow {
    /// Row offset from the pane's top, in rows (fractional while scrolling).
    pub y: f32,
    /// 1-based; 0 draws no number (a soft-wrapped line's continuation row).
    pub number: u32,
    pub text: String,
    pub fg: Vec<(Range<usize>, u32)>,
    pub bg: Vec<(Range<usize>, u32)>,
    pub selected: Option<Range<usize>>,
    pub caret: Option<usize>,
}

/// Families tried in order when none is configured or the configured one is missing.
pub const MONO_CANDIDATES: &[&str] = &[
    "JetBrains Mono",
    "JetBrainsMono Nerd Font",
    "Menlo",
    "SF Mono",
    "DejaVu Sans Mono",
    "Noto Sans Mono",
    "Liberation Mono",
    "Ubuntu Mono",
];

/// The configured family when installed, else the first installed of [`MONO_CANDIDATES`] (gpui's own fallback
/// stack when none is); with a notice when the configured one is missing.
pub fn mono_family<'a>(configured: Option<&'a str>, installed: &'a [String]) -> (&'a str, Option<String>) {
    let has = |name: &str| installed.iter().any(|f| f == name);
    if let Some(name) = configured.filter(|n| has(n)) {
        return (name, None);
    }
    let fallback = MONO_CANDIDATES.iter().copied().find(|c| has(c)).unwrap_or("monospace");
    (fallback, configured.map(|n| format!("Font “{n}” is not installed; using {fallback}")))
}

/// The monospace font for `configured` among the installed fonts, and a notice when it is missing.
pub fn pick_mono(cx: &App, configured: Option<&str>) -> (Font, Option<String>) {
    let installed = cx.text_system().all_font_names();
    let (family, notice) = mono_family(configured, &installed);
    (font(family.to_string()), notice)
}

pub fn fill(window: &mut Window, x: f32, y: f32, w: f32, h: f32, c: u32) {
    window.paint_quad(quad(
        Bounds { origin: point(px(x), px(y)), size: size(px(w.max(0.0)), px(h.max(0.0))) },
        px(0.),
        color(c),
        px(0.),
        gpui::transparent_black(),
        Default::default(),
    ));
}

/// `bounds` narrowed horizontally to `x..x + w`.
pub fn clip(bounds: Bounds<Pixels>, x: f32, w: f32) -> ContentMask<Pixels> {
    ContentMask { bounds: Bounds { origin: point(px(x), bounds.origin.y), size: size(px(w.max(0.0)), bounds.size.height) } }
}

/// One line of plain text in `c`, with marks capped and bidi controls shown.
pub fn text(window: &mut Window, cx: &mut App, m: Metrics, x: f32, y: f32, s: &str, c: u32) {
    if s.is_empty() {
        return;
    }
    let s = display::cap_marks(s);
    let run = TextRun { len: s.len(), font: m.font.clone(), color: color(c), background_color: None, underline: None, strikethrough: None };
    let line = window.text_system().shape_line(SharedString::from(display::show_bidi(&s).into_owned()), px(m.font_size), &[run], None);
    let _ = line.paint(point(px(x), px(y)), px(m.line_h), TextAlign::Left, None, window, cx);
}

/// One line with `fg` text colours (else `default_fg`) and `bg` fills; returns the shaped line for caret placement.
/// Malformed spans fall back to plain text rather than panic.
#[allow(clippy::too_many_arguments)]
pub fn colored_text(
    window: &mut Window,
    cx: &mut App,
    m: Metrics,
    default_fg: u32,
    x: f32,
    y: f32,
    s: &str,
    fg: &[(Range<usize>, u32)],
    bg: &[(Range<usize>, u32)],
) -> Option<ShapedLine> {
    if s.is_empty() {
        return None;
    }
    let mut cuts = vec![0, s.len()];
    for (r, _) in fg.iter().chain(bg) {
        cuts.extend([r.start.min(s.len()), r.end.min(s.len())]);
    }
    cuts.sort_unstable();
    cuts.dedup();
    let covering = |spans: &[(Range<usize>, u32)], a: usize, b: usize| spans.iter().find(|(r, _)| r.start <= a && b <= r.end).map(|(_, c)| *c);
    let runs: Vec<TextRun> = cuts
        .windows(2)
        .filter(|w| w[0] < w[1] && s.is_char_boundary(w[0]) && s.is_char_boundary(w[1]))
        .map(|w| TextRun {
            len: w[1] - w[0],
            font: m.font.clone(),
            color: color(covering(fg, w[0], w[1]).unwrap_or(default_fg)),
            background_color: covering(bg, w[0], w[1]).map(color),
            underline: None,
            strikethrough: None,
        })
        .collect();
    if runs.iter().map(|r| r.len).sum::<usize>() != s.len() {
        text(window, cx, m, x, y, s, default_fg);
        return None;
    }
    let line = window.text_system().shape_line(SharedString::from(display::show_bidi(s).into_owned()), px(m.font_size), &runs, None);
    let o = point(px(x), px(y));
    let _ = line.paint_background(o, px(m.line_h), TextAlign::Left, None, window, cx);
    let _ = line.paint(o, px(m.line_h), TextAlign::Left, None, window, cx);
    Some(line)
}

/// A numbered row at `x..x + w` of a pane whose rows start at `bounds`' top: the number, then the text scrolled
/// left by `hscroll` under it, with the selection over the row's own backgrounds and the caret on top.
#[allow(clippy::too_many_arguments)]
pub fn paint_row(window: &mut Window, cx: &mut App, m: Metrics, palette: &Palette, bounds: Bounds<Pixels>, x: f32, w: f32, hscroll: f32, row: &TextRow) {
    let y = f32::from(bounds.origin.y) + row.y * m.line_h;
    if row.number > 0 {
        text(window, cx, m, x + 6.0, y, &format!("{:>6}", row.number), palette.num_fg); // 0: a wrapped line's next row
    }
    window.with_content_mask(Some(clip(bounds, x + m.num_w + 4.0, w - m.num_w - 4.0)), |window| {
        let tx = text_x(m, x, hscroll);
        let bg: Vec<_> = row.selected.iter().map(|r| (r.clone(), palette.sel_bg)).chain(row.bg.iter().cloned()).collect();
        let shaped = colored_text(window, cx, m, palette.fg, tx, y, &row.text, &row.fg, &bg);
        if let Some(at) = row.caret {
            let cx_ = shaped.map_or(0.0, |l| f32::from(l.x_for_index(at)));
            fill(window, tx + cx_, y + 1.0, 2.0, m.line_h - 2.0, palette.caret);
        }
    });
}

/// Where a pane's text starts at horizontal scroll `hscroll`, for a pane at `x`: the inverse of a pointer's `x`
/// in [`column_at`].
pub fn text_x(m: Metrics, x: f32, hscroll: f32) -> f32 {
    x + m.num_w + 8.0 - hscroll
}

/// The source byte (a character boundary of `line`) nearest `x`, measured from the start of the drawn text.
pub fn column_at(window: &Window, m: Metrics, line: &str, x: f32) -> usize {
    let line = &line[..display::displayed_bytes(line)];
    let (shown, map) = display::expand_tabs(line);
    let run = TextRun { len: shown.len(), font: m.font.clone(), color: Hsla::default(), background_color: None, underline: None, strikethrough: None };
    let shaped = window.text_system().shape_line(SharedString::from(shown), px(m.font_size), &[run], None);
    let d = shaped.closest_index_for_x(px(x.max(0.0)));
    line.char_indices().map(|(i, _)| i).chain([line.len()]).rfind(|&i| map[i] <= d).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_configured_font_family_is_used_only_when_installed() {
        let installed = ["DejaVu Sans Mono".to_string(), "Iosevka".to_string()];
        assert_eq!(mono_family(Some("Iosevka"), &installed), ("Iosevka", None));
        let (family, notice) = mono_family(Some("Nope Mono"), &installed);
        assert_eq!(family, "DejaVu Sans Mono");
        assert!(notice.unwrap().contains("Nope Mono"));
        assert_eq!(mono_family(None, &installed), ("DejaVu Sans Mono", None));
    }
}
