//! Raised surfaces shared by both windows: menus, the command palette, pickers, popups and dialogs. One shadow,
//! corner radius, border and highlighted-row shape, so every floating box reads as the same kind of thing.

use gpui::{BoxShadow, Bounds, Corners, Pixels, Window, point, px, quad};
use lenz_content::theme::{Palette, luminance};

use crate::color;

/// Corner radius of a floating box.
pub const RADIUS: f32 = 6.0;
/// Corner radius of a highlighted row, and its inset from the box's sides.
pub const ROW_RADIUS: f32 = 4.0;
pub const ROW_INSET: f32 = 4.0;

/// The background and border of a floating box ([`Palette::raised`]).
pub fn colors(palette: &Palette) -> (u32, u32) { palette.raised() }

/// The fill of a menu's highlighted row on a floating box.
pub fn selected(palette: &Palette) -> u32 { palette.menu_selected(palette.raised().0) }

/// The drop shadow: a wide soft one for depth and a tight one that seats the edge. Dark themes need more of both
/// to show against a dark window.
pub fn shadows(palette: &Palette) -> [BoxShadow; 2] {
    let (wide, tight) = if luminance(palette.bg) < 0.4 { (0.75, 0.50) } else { (0.32, 0.18) };
    [
        BoxShadow::new(px(0.), px(8.), gpui::hsla(0., 0., 0., wide)).blur_radius(px(20.)),
        BoxShadow::new(px(0.), px(2.), gpui::hsla(0., 0., 0., tight)).blur_radius(px(6.)),
    ]
}

/// Paint a floating box over `x, y, w, h` (its inside; the 1px border sits just outside): shadow, border and the
/// lifted background.
pub fn surface(window: &mut Window, x: f32, y: f32, w: f32, h: f32, palette: &Palette) {
    surface_on(window, x, y, w, h, palette, colors(palette).0);
}

/// [`surface`] with its own background: documentation keeps the sidebar's, which its Markdown styling is made for.
pub fn surface_on(window: &mut Window, x: f32, y: f32, w: f32, h: f32, palette: &Palette, bg: u32) {
    let border = colors(palette).1;
    let outer = Bounds { origin: point(px(x - 1.), px(y - 1.)), size: gpui::size(px((w + 2.).max(0.)), px((h + 2.).max(0.))) };
    window.paint_drop_shadows(outer, Corners::all(px(RADIUS)), &shadows(palette));
    window.paint_quad(quad(outer, px(RADIUS), color(bg), px(1.), color(border), Default::default()));
}

/// Paint the highlighted row of a menu across `x, w` (the box's inside) at `y, h`: inset from both sides, rounded.
pub fn row(window: &mut Window, x: f32, y: f32, w: f32, h: f32, fill: u32) {
    let bounds: Bounds<Pixels> =
        Bounds { origin: point(px(x + ROW_INSET), px(y)), size: gpui::size(px((w - 2. * ROW_INSET).max(0.)), px(h.max(0.))) };
    window.paint_quad(quad(bounds, px(ROW_RADIUS), color(fill), px(0.), gpui::transparent_black(), Default::default()));
}

/// Paint a text field's box inside a floating box: `edge` is its 1px outline.
pub fn field(window: &mut Window, x: f32, y: f32, w: f32, h: f32, edge: u32, fill: u32) {
    let bounds: Bounds<Pixels> = Bounds { origin: point(px(x), px(y)), size: gpui::size(px(w.max(0.)), px(h.max(0.))) };
    window.paint_quad(quad(bounds, px(ROW_RADIUS), color(fill), px(1.), color(edge), Default::default()));
}

/// Style a dialog or tooltip built from elements as a floating box: background, border, radius and shadow.
pub fn dialog<E: gpui::Styled>(element: E, palette: &Palette) -> E {
    let (bg, border) = colors(palette);
    element.bg(color(bg)).border_1().border_color(color(border)).rounded(px(RADIUS)).shadow(shadows(palette).to_vec())
}
