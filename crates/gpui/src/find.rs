//! In-document find presentation. Applications supply core state and dispatch the returned option.
use crate::{Metrics, fill, text};
use gpui::{App, Bounds, ContentMask, Pixels, Window, point, px, size};
use lenz_content::{find::FindOption, theme::Palette};

#[derive(Clone, Debug)]
pub struct FindBox {
    pub bounds: Bounds<Pixels>,
    pub toggles: [(FindOption, Bounds<Pixels>); 3],
    pub on: [bool; 3],
    pub query: String,
    pub counter: String,
    pub error: bool,
    pub focused: bool,
}

impl FindBox {
    /// Geometry stays within the pane, including when no room remains for option buttons.
    pub fn layout(mut self, pane: Bounds<Pixels>, line_h: f32, char_w: f32) -> Self {
        let (x, y, w, h) = (
            f32::from(pane.origin.x),
            f32::from(pane.origin.y),
            f32::from(pane.size.width).max(0.0),
            f32::from(pane.size.height).max(0.0),
        );
        let margin = 8.0_f32.min(w / 2.0);
        let bw = ((self.query.chars().count() + self.counter.chars().count() + 18) as f32 * char_w)
            .max(240.0)
            .min((w - 2.0 * margin).max(0.0));
        let bh = (line_h + 4.0).min(h);
        let by = y + 4.0_f32.min((h - bh).max(0.0));
        self.bounds = Bounds {
            origin: point(px(x + w - margin - bw), px(by)),
            size: size(px(bw), px(bh)),
        };
        let toggle_w = 3.0 * char_w + 4.0;
        let counter_w = (self.counter.chars().count() as f32 * char_w).min(bw * 0.4);
        let right = x + w - margin - 8.0 - counter_w;
        self.toggles = std::array::from_fn(|i| {
            let visible = bw >= (3.0 * toggle_w + counter_w + char_w + 32.0).max(240.0) && bh > 4.0;
            (
                [FindOption::Case, FindOption::Word, FindOption::Regex][i],
                Bounds {
                    origin: point(px(right - 8.0 - (3 - i) as f32 * toggle_w), px(by + 2.0)),
                    size: size(
                        px(if visible { toggle_w - 2.0 } else { 0.0 }),
                        px(if visible { bh - 4.0 } else { 0.0 }),
                    ),
                },
            )
        });
        self
    }

    pub fn option_at(&self, p: gpui::Point<Pixels>) -> Option<FindOption> {
        self.toggles
            .iter()
            .find(|(_, b)| b.size.width > px(0.0) && b.contains(&p))
            .map(|(o, _)| *o)
    }

    pub fn paint(
        &self,
        window: &mut Window,
        cx: &mut App,
        m: Metrics,
        palette: &Palette,
        offset: gpui::Point<Pixels>,
        char_w: f32,
    ) {
        let bounds = Bounds {
            origin: self.bounds.origin + offset,
            size: self.bounds.size,
        };
        let (x, y, w, h) = (
            f32::from(bounds.origin.x),
            f32::from(bounds.origin.y),
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
        );
        if w <= 2.0 || h <= 2.0 {
            return;
        }
        let counter_w = (self.counter.chars().count() as f32 * char_w).min(w * 0.4);
        let counter_x = x + w - 8.0 - counter_w;
        let query_end = if self.toggles[0].1.size.width > px(0.0) {
            f32::from(self.toggles[0].1.origin.x + offset.x) - 4.0
        } else {
            counter_x - 4.0
        };
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            fill(
                window,
                x,
                y,
                w,
                h,
                if self.focused {
                    palette.accent
                } else {
                    palette.border
                },
            );
            fill(
                window,
                x + 1.0,
                y + 1.0,
                w - 2.0,
                h - 2.0,
                palette.header_bg,
            );
            let field = Bounds {
                origin: point(px(x + 6.0), px(y)),
                size: size(px((query_end - x - 6.0).max(0.0)), px(h)),
            };
            window.with_content_mask(Some(ContentMask { bounds: field }), |window| {
                let query_w = self.query.chars().count() as f32 * char_w;
                let tx = (query_end - query_w - 4.0).min(x + 8.0);
                text(window, cx, m, tx, y + 2.0, &self.query, palette.fg);
                if self.focused {
                    fill(window, tx + query_w, y + 4.0, 2.0, h - 8.0, palette.caret);
                }
            });
            for ((option, b), on) in self.toggles.iter().zip(self.on) {
                if b.size.width <= px(0.0) {
                    continue;
                }
                let (tx, ty, tw, th) = (
                    f32::from(b.origin.x + offset.x),
                    f32::from(b.origin.y + offset.y),
                    f32::from(b.size.width),
                    f32::from(b.size.height),
                );
                let label = option.symbol();
                fill(
                    window,
                    tx,
                    ty,
                    tw,
                    th,
                    if on {
                        palette.selected_bg
                    } else {
                        palette.header_bg
                    },
                );
                text(
                    window,
                    cx,
                    m,
                    tx + (tw - label.chars().count() as f32 * char_w) / 2.0,
                    ty,
                    label,
                    if on { palette.fg } else { palette.dim },
                );
            }
            let counter = Bounds {
                origin: point(px(counter_x), px(y)),
                size: size(px(counter_w), px(h)),
            };
            window.with_content_mask(Some(ContentMask { bounds: counter }), |window| {
                text(
                    window,
                    cx,
                    m,
                    counter_x,
                    y + 2.0,
                    &self.counter,
                    if self.error { palette.fg } else { palette.dim },
                );
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tiny_and_narrow_find_stays_inside_pane() {
        for char_w in [5.0, 9.6, 19.0, 32.0] {
            for height in [0.0, 1.0, 20.0, 49.0, 100.0] {
                for width in [0.0, 1.0, 16.0, 80.0, 240.0, 600.0] {
                    let pane = Bounds {
                        origin: point(px(12.0), px(7.0)),
                        size: size(px(width), px(height)),
                    };
                    let b = FindBox {
                        bounds: Bounds::default(),
                        toggles: [(FindOption::Case, Bounds::default()); 3],
                        on: [false; 3],
                        query: "long query ".repeat(50),
                        counter: "invalid expression ".repeat(50),
                        error: true,
                        focused: true,
                    }
                    .layout(pane, 24.0, char_w);
                    assert!(b.bounds.origin.x >= pane.origin.x);
                    assert!(b.bounds.right() <= pane.right());
                    assert!(b.bounds.bottom() <= pane.bottom());
                    for (option, t) in b.toggles {
                        if t.size.width > px(0.0) {
                            assert!(
                                t.origin.x >= b.bounds.origin.x && t.right() <= b.bounds.right()
                            );
                            assert_eq!(b.option_at(t.center()), Some(option));
                        }
                    }
                }
            }
        }
    }
}
