//! Reading colours derived from a theme: Markdown callouts, highlights and removed text, picked by hue from the
//! palette so every theme gets fitting ones. Shared by Lenz's Markdown Preview and the code reader.

use super::{Palette, mix};

/// Hue in degrees and saturation (0–1) of `0xRRGGBB`.
fn hue_saturation(c: u32) -> (f32, f32) {
    let [r, g, b] = [16, 8, 0].map(|s| ((c >> s) & 0xff) as f32 / 255.);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    if d == 0. {
        return (0., 0.);
    }
    let h = if max == r { ((g - b) / d).rem_euclid(6.) } else if max == g { (b - r) / d + 2. } else { (r - g) / d + 4. };
    let l = (max + min) / 2.;
    (h * 60., d / (1. - (2. * l - 1.).abs()))
}

/// The most saturated-enough palette colour closest to `hue` (degrees).
pub fn nearest_hue(p: &Palette, hue: f32) -> u32 {
    let s = &p.syntax;
    let candidates = [s.function, s.string, s.number, s.keyword, s.constant, p.accent, p.error, p.status[0], p.status[1], p.status[2]];
    let distance = |c: u32| {
        let (h, sat) = hue_saturation(c);
        let d = (h - hue).abs() % 360.;
        let d = d.min(360. - d);
        if sat < 0.25 { d + 180. } else { d }
    };
    candidates.into_iter().min_by(|a, b| distance(*a).total_cmp(&distance(*b))).unwrap_or(p.accent)
}

/// A Markdown callout's colour: the theme colour nearest each kind's usual hue (blue notes, green tips, orange
/// warnings, red dangers, purple examples, muted quotes).
pub fn callout_color(p: &Palette, kind: crate::markdown::CalloutKind) -> u32 {
    use crate::markdown::CalloutKind::*;
    let hue = match kind {
        Note | Info | Todo => 215.,
        Abstract | Question => 190.,
        Tip | Success => 120.,
        Warning => 30.,
        Failure | Danger | Bug => 0.,
        Example => 275.,
        Quote => return p.dim,
    };
    nearest_hue(p, hue)
}

pub fn callout_icon(kind: crate::markdown::CalloutKind) -> &'static str {
    use crate::markdown::CalloutKind::*;
    match kind {
        Note | Info => "ⓘ",
        Abstract => "☰",
        Todo => "☑",
        Tip => "✦",
        Success => "✓",
        Question => "?",
        Warning => "⚠",
        Failure => "✗",
        Danger => "⚡",
        Bug => "✱",
        Example => "≡",
        Quote => "❝",
    }
}

/// Removed text in the Markdown Preview: the theme's deleted colour, or its nearest red where deletions are grey
/// (Darcula and Light grey them, as IntelliJ does).
pub fn removed_color(p: &Palette) -> u32 {
    if hue_saturation(p.status[1]).1 < 0.25 { nearest_hue(p, 0.) } else { p.status[1] }
}

/// The background of Markdown `==highlights==`: the theme's amber, or a standard amber when it has none (a red
/// highlight would read as a removal).
pub fn highlight_bg(p: &Palette) -> u32 {
    let near = nearest_hue(p, 45.);
    let (hue, saturation) = hue_saturation(near);
    let amber = if saturation >= 0.25 && (25. ..75.).contains(&hue) { near } else { 0xF2C94C };
    mix(p.bg, amber, 0.35)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{DARCULA, LIGHT};

    #[test]
    fn extras_colours_follow_the_hue_of_each_kind() {
        use crate::markdown::CalloutKind;
        for p in [&DARCULA, &LIGHT] {
            assert_ne!(callout_color(p, CalloutKind::Warning), callout_color(p, CalloutKind::Note));
            assert_eq!(callout_color(p, CalloutKind::Quote), p.dim);
            assert_ne!(highlight_bg(p), p.bg);
            let (hue, _) = hue_saturation(highlight_bg(p));
            assert!((25. ..75.).contains(&hue), "highlights are amber, never red like a removal: {:x}", highlight_bg(p));
            assert_eq!(callout_icon(CalloutKind::Warning), "⚠");
            let (hue, saturation) = hue_saturation(removed_color(p));
            assert!(saturation > 0.25 && !(30. ..330.).contains(&hue), "removed text is red even where the theme greys deletions: {:x}", removed_color(p));
        }
    }
}
