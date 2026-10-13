//! Single images as GPUI textures, shared by Lenz and its code reader: RGBA pixels to a texture, the
//! checkerboard behind transparent pixels, and the uploads a view still draws (GPUI's atlas keeps a texture until
//! it is removed explicitly; dropping the last `Arc` frees only the CPU pixels).

use std::sync::Arc;

use gpui::{Bounds, Pixels, RenderImage, Window};
use lenz_content::image::{Frame, Pixels as Rgba, swap_rb};

use crate::fill;

/// RGBA pixels as a GPUI texture (GPUI wants BGRA: converted in place, no copy).
pub fn texture(mut pixels: Rgba) -> Arc<RenderImage> {
    swap_rb(&mut pixels);
    bgra_texture(pixels)
}

/// Pixels already in GPUI's BGRA order as a texture.
pub fn bgra_texture(pixels: Rgba) -> Arc<RenderImage> {
    Arc::new(RenderImage::new(vec![Frame::new(pixels)]))
}

/// Cell size of [`checker`].
pub const CHECKER_CELL: f32 = 12.0;

/// A checkerboard behind transparent pixels: `area` (x, y, w, h) clipped to `clip`.
pub fn checker(window: &mut Window, clip: Bounds<Pixels>, area: (f32, f32, f32, f32)) {
    let (ax, ay, aw, ah) = area;
    let (cx0, cy0) = (f32::from(clip.origin.x), f32::from(clip.origin.y));
    let (cx1, cy1) = (cx0 + f32::from(clip.size.width), cy0 + f32::from(clip.size.height));
    let (x0, y0, x1, y1) = (ax.max(cx0), ay.max(cy0), (ax + aw).min(cx1), (ay + ah).min(cy1));
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    fill(window, x0, y0, x1 - x0, y1 - y0, 0x3A3A3A);
    let mut y = (y0 / CHECKER_CELL).floor() * CHECKER_CELL;
    while y < y1 {
        let mut x = (x0 / CHECKER_CELL).floor() * CHECKER_CELL + if ((y / CHECKER_CELL) as i64) % 2 == 0 { 0.0 } else { CHECKER_CELL };
        while x < x1 {
            let (l, t) = (x.max(x0), y.max(y0));
            let (r, b) = ((x + CHECKER_CELL).min(x1), (y + CHECKER_CELL).min(y1));
            if r > l && b > t {
                fill(window, l, t, r - l, b - t, 0x4A4A4A);
            }
            x += 2.0 * CHECKER_CELL;
        }
        y += CHECKER_CELL;
    }
}

/// The textures a view uploaded and still draws.
#[derive(Default)]
pub struct Uploads(Vec<Arc<RenderImage>>);

impl Uploads {
    /// From now on the view draws `keep`: every other texture it uploaded leaves GPUI's atlas.
    pub fn keep(&mut self, keep: Vec<Arc<RenderImage>>, window: &mut Window) {
        for old in std::mem::replace(&mut self.0, keep) {
            if !self.0.iter().any(|r| r.id == old.id) {
                let _ = window.drop_image(old);
            }
        }
    }

    /// How many textures it holds (for tests and diagnostics).
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
