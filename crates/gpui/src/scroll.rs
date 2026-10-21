//! Wheel conversion and fractional steps shared by the GUI adapters.
use gpui::{Point, ScrollDelta, point};

/// Positive pixels move right/down; a wheel notch moves three rows or columns.
pub fn pixels(delta: ScrollDelta, line_h: f32, char_w: f32) -> Point<f32> {
    match delta {
        ScrollDelta::Pixels(p) => point(-f32::from(p.x), -f32::from(p.y)),
        ScrollDelta::Lines(l) => point(-l.x * 3.0 * char_w, -l.y * 3.0 * line_h),
    }
}

/// Shift redirects a vertical wheel to horizontal movement at the same pixel speed.
pub fn shifted(delta: Point<f32>, shift: bool) -> Point<f32> {
    if shift && delta.x == 0.0 { point(delta.y, 0.0) } else { delta }
}

/// A discrete viewport consumes whole steps and keeps the fraction for its next event.
pub fn steps(fraction: &mut f32, delta: f32) -> isize {
    *fraction += delta;
    let steps = fraction.trunc() as isize;
    *fraction -= steps as f32;
    steps
}

/// Discard movement beyond an edge so reversing direction responds immediately.
pub fn clamp(fraction: &mut f32, at_start: bool, at_end: bool) {
    if at_start && *fraction < 0.0 || at_end && *fraction > 0.0 { *fraction = 0.0; }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    #[test]
    fn pixels_keep_both_axes_and_wheel_units() {
        assert_eq!(pixels(ScrollDelta::Pixels(point(px(-0.5), px(-0.25))), 20.0, 8.0), point(0.5, 0.25));
        assert_eq!(pixels(ScrollDelta::Lines(point(-1.0, 2.0)), 20.0, 8.0), point(24.0, -120.0));
        assert_eq!(shifted(point(0.0, 0.25), true), point(0.25, 0.0));
        assert_eq!(shifted(point(0.5, 0.25), true), point(0.5, 0.25));
    }

    #[test]
    fn fractions_accumulate_and_clamp_at_both_edges() {
        let mut fraction = 0.0;
        for expected in [0, 0, 0, 1] { assert_eq!(steps(&mut fraction, 0.25), expected); }
        for (delta, start, end) in [(-0.25, true, false), (0.25, false, true)] {
            for _ in 0..3 { assert_eq!(steps(&mut fraction, delta), 0); clamp(&mut fraction, start, end); }
            for expected in [0, 0, 0, if delta > 0.0 { -1 } else { 1 }] {
                assert_eq!(steps(&mut fraction, -delta), expected);
            }
        }
    }
}
