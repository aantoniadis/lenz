//! In-document find and replace presentation. Applications supply core state and dispatch the returned option
//! or row.
use crate::{Metrics, fill, text};
use gpui::{App, Bounds, ContentMask, Pixels, Window, point, px, size};
use lenz_content::{find::FindOption, theme::Palette};

#[derive(Clone, Debug)]
pub struct FindBox {
    pub bounds: Bounds<Pixels>,
    pub toggles: [(FindOption, Bounds<Pixels>); 3],
    pub on: [bool; 3],
    pub query: String,
    pub query_cursor: lenz_content::field_edit::Cursor,
    pub replace_cursor: lenz_content::field_edit::Cursor,
    pub counter: String,
    pub error: bool,
    pub focused: bool,
    /// Whether the replacement row is offered at all (the reader has replace; Diff's find bar does not).
    pub show_replace: bool,
    /// What the replacement row does (the core's Replace label), shown before its text.
    pub replace_label: String,
    /// The text Replace and Replace All write, in the row below the query.
    pub replace: String,
    /// The replace row has the typed keys.
    pub replace_focused: bool,
    /// The replace row's bounds (empty when the pane leaves no room for it).
    pub replace_row: Bounds<Pixels>,
}

impl FindBox {
    /// Geometry stays within the pane, including when no room remains for option buttons or the replace row.
    pub fn layout(mut self, pane: Bounds<Pixels>, line_h: f32, char_w: f32) -> Self {
        let (x, y, w, h) = (
            f32::from(pane.origin.x),
            f32::from(pane.origin.y),
            f32::from(pane.size.width).max(0.0),
            f32::from(pane.size.height).max(0.0),
        );
        let margin = 8.0_f32.min(w / 2.0);
        let toggle_w = 3.0 * char_w + 4.0;
        let minimum = (3.0*toggle_w + self.counter.chars().count().min(12) as f32*char_w + 6.0*char_w + 32.0).max(240.0);
        let bw = ((self.query.chars().count() + self.counter.chars().count() + 18) as f32 * char_w)
            .max(minimum)
            .min((w - 2.0 * margin).max(0.0));
        let row_h = line_h + 4.0;
        // Both rows show only where the pane has room for them; the query row always does.
        let two_rows = self.show_replace && h >= row_h * 2.0;
        let bh = if two_rows { row_h * 2.0 } else { row_h.min(h) };
        let by = y + 4.0_f32.min((h - bh).max(0.0));
        self.bounds = Bounds {
            origin: point(px(x + w - margin - bw), px(by)),
            size: size(px(bw), px(bh)),
        };
        self.replace_row = if two_rows {
            Bounds {
                origin: point(px(x + w - margin - bw), px(by + row_h)),
                size: size(px(bw), px(row_h)),
            }
        } else {
            Bounds::default()
        };
        let counter_w = self.counter_width(char_w);
        let right = x + w - margin - 8.0 - counter_w;
        self.toggles = std::array::from_fn(|i| {
            let visible = bw >= (3.0 * toggle_w + counter_w + 6.0*char_w + 32.0).max(240.0) && row_h > 4.0;
            (
                [FindOption::Case, FindOption::Word, FindOption::Regex][i],
                Bounds {
                    origin: point(px(right - 8.0 - (3 - i) as f32 * toggle_w), px(by + 2.0)),
                    size: size(px(if visible { toggle_w - 2.0 } else { 0.0 }), px(if visible { row_h - 4.0 } else { 0.0 })),
                },
            )
        });
        self
    }

    fn counter_width(&self, char_w:f32)->f32 {
        let w=f32::from(self.bounds.size.width);
        if w<240.0 {0.0}else{(self.counter.chars().count() as f32*char_w).min(w*0.4)}
    }

    /// The query keeps priority over counters and option buttons in a compact pane.
    pub fn query_bounds(&self, offset:gpui::Point<Pixels>, char_w:f32)->Bounds<Pixels>{
        let x=f32::from(self.bounds.origin.x+offset.x);
        let y=f32::from(self.bounds.origin.y+offset.y);
        let w=f32::from(self.bounds.size.width);
        let row_h=f32::from(self.bounds.size.height)/if self.replace_row.size.height>px(0.0){2.0}else{1.0};
        let end=if self.toggles[0].1.size.width>px(0.0){f32::from(self.toggles[0].1.origin.x+offset.x)-4.0}else{x+w-12.0-self.counter_width(char_w)};
        Bounds{origin:point(px(x+8.0),px(y+2.0)),size:size(px((end-x-12.0).max(0.0)),px((row_h-4.0).max(0.0)))}
    }

    pub fn option_at(&self, p: gpui::Point<Pixels>) -> Option<FindOption> {
        self.toggles
            .iter()
            .find(|(_, b)| b.size.width > px(0.0) && b.contains(&p))
            .map(|(o, _)| *o)
    }

    /// Whether `p` is in the replace row, which then takes the typed keys.
    pub fn replace_at(&self, p: gpui::Point<Pixels>) -> bool {
        self.replace_row.size.height > px(0.0) && self.replace_row.contains(&p)
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
        let row_h = h / if self.replace_row.size.height > px(0.0) { 2.0 } else { 1.0 };
        let counter_w = self.counter_width(char_w);
        let counter_x = x + w - 8.0 - counter_w;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            fill(window, x, y, w, h, if self.focused { palette.accent } else { palette.border });
            fill(window, x + 1.0, y + 1.0, w - 2.0, h - 2.0, palette.header_bg);
            crate::field::paint(crate::field::View { text: &self.query, cursor: self.query_cursor,
                bounds: self.query_bounds(offset,char_w),char_w,active:self.focused&&!self.replace_focused },window,cx,m,palette);
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
                fill(window, tx, ty, tw, th, if on { palette.selected_bg } else { palette.header_bg });
                text(window, cx, m, tx + (tw - label.chars().count() as f32 * char_w) / 2.0, ty, label, if on { palette.fg } else { palette.dim });
            }
            let counter = Bounds {
                origin: point(px(counter_x), px(y)),
                size: size(px(counter_w), px(row_h)),
            };
            window.with_content_mask(Some(ContentMask { bounds: counter }), |window| {
                text(window, cx, m, counter_x, y + 2.0, &self.counter, if self.error { palette.fg } else { palette.dim });
            });
            if self.replace_row.size.height > px(0.0) {
                let ry = y + row_h;
                fill(window, x + 1.0, ry, w - 2.0, 1.0, palette.border);
                let label = if w<self.replace_label.chars().count() as f32*char_w + 6.0*char_w+20.0 {"↦"}else{self.replace_label.as_str()};
                let left = x + 6.0;
                let field_left = left + label.chars().count() as f32 * char_w + 6.0;
                text(window, cx, m, left, ry + 2.0, label, palette.dim);
                let row = Bounds {
                    origin: point(px(field_left), px(ry)),
                    size: size(px((x + w - 8.0 - field_left).max(0.0)), px(row_h)),
                };
                crate::field::paint(crate::field::View{text:&self.replace,cursor:self.replace_cursor,bounds:Bounds{origin:point(row.origin.x,px(ry+2.0)),size:row.size},char_w,active:self.focused&&self.replace_focused},window,cx,m,palette);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn boxed(query: &str, replace: &str) -> FindBox {
        FindBox {
            bounds: Bounds::default(),
            toggles: [(FindOption::Case, Bounds::default()); 3],
            on: [false; 3],
            query: query.into(),
            query_cursor:Default::default(),replace_cursor:Default::default(),
            counter: "invalid expression ".repeat(50),
            error: true,
            focused: true,
            show_replace: true,
            replace_label: "Replace".into(),
            replace: replace.into(),
            replace_focused: true,
            replace_row: Bounds::default(),
        }
    }
    #[test]
    fn tiny_and_narrow_find_stays_inside_pane() {
        for char_w in [5.0, 9.6, 19.0, 32.0] {
            for height in [0.0, 1.0, 20.0, 49.0, 100.0] {
                for width in [0.0, 1.0, 16.0, 80.0, 240.0, 600.0] {
                    let pane = Bounds {
                        origin: point(px(12.0), px(7.0)),
                        size: size(px(width), px(height)),
                    };
                    let b = boxed(&"long query ".repeat(50), &"replacement ".repeat(50)).layout(pane, 24.0, char_w);
                    assert!(b.bounds.origin.x >= pane.origin.x);
                    assert!(b.bounds.right() <= pane.right());
                    assert!(b.bounds.bottom() <= pane.bottom());
                    for (option, t) in b.toggles {
                        if t.size.width > px(0.0) {
                            assert!(t.origin.x >= b.bounds.origin.x && t.right() <= b.bounds.right());
                            assert_eq!(b.option_at(t.center()), Some(option));
                        }
                    }
                    // The replace row is inside the box when it shows, and absent when the pane is too short.
                    if b.replace_row.size.height > px(0.0) {
                        assert_eq!(b.replace_row.origin.x, b.bounds.origin.x);
                        assert!(b.replace_row.right() <= b.bounds.right());
                        assert!(b.replace_row.bottom() <= b.bounds.bottom());
                        if b.replace_row.size.width > px(0.0) {
                            assert!(b.replace_at(b.replace_row.center()));
                            assert!(!b.replace_at(b.bounds.origin + point(px(1.0), px(1.0))), "the query row is above it");
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn compact_find_preserves_a_visible_query_in_a_split_pane(){
        for width in [48.0,76.0,100.0,224.0]{
            let b=boxed("alpha beta ","replacement").layout(Bounds{origin:point(px(0.0),px(0.0)),size:size(px(width+16.0),px(100.0))},20.0,8.0);
            let field=b.query_bounds(point(px(0.0),px(0.0)),8.0);
            assert_eq!(f32::from(field.size.width),width-24.0);
            assert!(b.toggles.iter().all(|(_,b)|b.size.width==px(0.0)));
            assert_eq!(b.counter_width(8.0),0.0);
        }
    }

}
