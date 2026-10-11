//! Bounded pane geometry in adapter units (pixels or terminal cells).
//! Preferred proportions are retained by adapters; clamping never overwrites them.

fn positive(value: f32) -> f32 {
    if value.is_finite() { value.max(0.0) } else { 0.0 }
}

/// Resolve a preferred size while leaving room for a neighbouring pane.
/// In tiny layouts the minimums shrink proportionally, keeping both panes usable.
pub fn clamp_size(available: f32, desired: f32, min: f32, remaining: f32) -> f32 {
    let available = positive(available);
    let (min, remaining) = (positive(min), positive(remaining));
    let scale = if min + remaining > available { available / (min + remaining) } else { 1.0 };
    let lower = min * scale;
    let upper = (available - remaining * scale).max(lower);
    let desired = if desired.is_finite() { desired } else { available * 0.5 };
    desired.clamp(lower, upper)
}

/// Width of the first of two panes, excluding the divider itself.
pub fn split(total: f32, ratio: f32, min: f32) -> f32 {
    let total = positive(total);
    let ratio = if ratio.is_finite() { ratio.clamp(0.0, 1.0) } else { 0.5 };
    clamp_size(total, total * ratio, min, min)
}

/// Widths of three panes, from preferred cumulative divider proportions.
pub fn three(total: f32, ratios: [f32; 2], min: f32) -> [f32; 3] {
    let total = positive(total);
    let min = positive(min).min(total / 3.0);
    let ratio = |value: f32, default: f32| if value.is_finite() { value.clamp(0.0, 1.0) } else { default };
    let first = clamp_size(total, total * ratio(ratios[0], 1.0 / 3.0), min, min * 2.0);
    let second = (total * ratio(ratios[1], 2.0 / 3.0)).clamp(first + min, (total - min).max(first + min));
    [first, second - first, (total - second).max(0.0)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_remain_bounded_at_extremes_and_tiny_geometry() {
        for total in [0.0, 1.0, 20.0, 1000.0, f32::NAN, f32::INFINITY] {
            for ratio in [-10.0, 0.2, 0.5, 0.8, 10.0, f32::NAN] {
                let left = split(total, ratio, 20.0);
                let available = positive(total);
                assert!(left.is_finite() && left >= 0.0 && left <= available);
                let widths = three(total, [ratio, ratio + 0.2], 20.0);
                assert!(widths.iter().all(|w| w.is_finite() && *w >= 0.0));
                assert!((widths.iter().sum::<f32>() - available).abs() < 0.001);
            }
        }
    }

    #[test]
    fn proportions_and_neighbour_minimums_are_preserved() {
        assert_eq!(split(1000.0, 0.3, 100.0), 300.0);
        assert_eq!(split(1000.0, 0.99, 100.0), 900.0);
        assert_eq!(three(900.0, [0.2, 0.7], 100.0), [180.0, 450.0, 270.0]);
        assert_eq!(clamp_size(100.0, 90.0, 20.0, 60.0), 40.0);
        assert_eq!(clamp_size(10.0, 90.0, 20.0, 20.0), 5.0);
    }
}
