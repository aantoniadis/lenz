//! Image Comparisons, without gpui: the pixel difference and its **Changed Regions**, and the onion-skin
//! blend. Single images (format detection, bounded decoding, the size cap, the pixel helpers) are shared with the
//! code reader in [`lenz_content::image`] and re-exported here; the UI turns RGBA into gpui images with
//! `lenz_gpui::image`.

use image::{Rgba, RgbaImage};

pub use lenz_content::image::*;

/// Changed Regions group changed pixels on a grid of this many pixels per cell.
pub const REGION_CELL: u32 = 24;
/// A pixel counts as changed when the sum of its four channel differences exceeds this (encoder noise).
pub const NOISE: u32 = 12;
const CHANGED: Rgba<u8> = Rgba([255, 40, 200, 255]);

/// Onion skin: `new` over `old` at opacity `alpha` (0 = old only, 1 = new only), on the union of their sizes.
pub fn blend(old: &RgbaImage, new: &RgbaImage, alpha: f32) -> RgbaImage {
    if old.dimensions() == new.dimensions() && old.as_raw().len() >= 64 << 10 {
        // At one opacity there are only 65,536 pairs of channel values. Preserve the exact
        // floating-point rounding, but calculate it once per pair rather than per image byte.
        let mut values = [0u8; 65_536];
        for (i, value) in values.iter_mut().enumerate() {
            *value = ((i >> 8) as f32 * (1.0 - alpha) + (i & 255) as f32 * alpha).round() as u8;
        }
        let mut bytes = vec![0; old.as_raw().len()];
        for ((out, &a), &b) in bytes.iter_mut().zip(old.as_raw()).zip(new.as_raw()) {
            *out = values[usize::from(a) * 256 + usize::from(b)];
        }
        return RgbaImage::from_raw(old.width(), old.height(), bytes).unwrap();
    }
    let (w, h) = (old.width().max(new.width()), old.height().max(new.height()));
    let at = |img: &RgbaImage, x: u32, y: u32| if x < img.width() && y < img.height() { *img.get_pixel(x, y) } else { Rgba([0, 0, 0, 0]) };
    RgbaImage::from_fn(w, h, |x, y| {
        let (a, b) = (at(old, x, y), at(new, x, y));
        Rgba(std::array::from_fn(|i| (a[i] as f32 * (1.0 - alpha) + b[i] as f32 * alpha).round() as u8))
    })
}

/// The pixel difference of two Sides. The Difference image itself is drawn on demand by [`difference`].
pub struct Diff {
    pub changed: u64,
    pub total: u64,
    /// Changed Regions, top-to-bottom then left-to-right.
    pub regions: Vec<Rect>,
}

/// Compares on the union of both sizes, top-left aligned; a pixel only one Side has counts as changed.
/// An absent Side is an empty image.
pub fn diff(old: &RgbaImage, new: &RgbaImage) -> Diff {
    let w = old.width().max(new.width());
    let h = old.height().max(new.height());
    let (gw, gh) = (w.div_ceil(REGION_CELL), h.div_ceil(REGION_CELL));
    let mut hot = vec![false; (gw * gh) as usize];
    let mut changed = 0;
    let mut mark = |x: u32, y: u32| {
        changed += 1;
        hot[((y / REGION_CELL) * gw + x / REGION_CELL) as usize] = true;
    };
    if old.dimensions() == new.dimensions() {
        // the common case, row by row without per-pixel bounds checks
        for (y, (a, b)) in rows(old).zip(rows(new)).enumerate() {
            for (x, (a, b)) in a.iter().zip(b).enumerate() {
                if differs(*a, *b) {
                    mark(x as u32, y as u32);
                }
            }
        }
    } else {
        for y in 0..h {
            for x in 0..w {
                if differs_at(old.get_pixel_checked(x, y), new.get_pixel_checked(x, y)) {
                    mark(x, y);
                }
            }
        }
    }
    Diff {
        changed,
        total: u64::from(w) * u64::from(h),
        regions: regions(&hot, gw, gh, w, h),
    }
}

/// The Difference image's pixel: changed pixels in magenta over a dimmed grey copy of the new image.
/// `None` is a pixel outside that Side.
pub fn difference_pixel(old: Option<&Rgba<u8>>, new: Option<&Rgba<u8>>) -> Rgba<u8> {
    Rgba(match (old, new) {
        (Some(a), Some(b)) => difference_of(a.0, b.0),
        _ => CHANGED.0,
    })
}

/// The Difference image of `rect`, in the union of both sizes. Built only when shown: kept for the whole
/// comparison it would cost another full-size buffer.
pub fn difference(old: &RgbaImage, new: &RgbaImage, rect: Rect) -> RgbaImage {
    let inside = rect.x.checked_add(rect.w).is_some_and(|r| r <= old.width())
        && rect.y.checked_add(rect.h).is_some_and(|b| b <= old.height());
    if old.dimensions() != new.dimensions() || !inside {
        return RgbaImage::from_fn(rect.w, rect.h, |x, y| {
            let (x, y) = (rect.x + x, rect.y + y);
            difference_pixel(old.get_pixel_checked(x, y), new.get_pixel_checked(x, y))
        });
    }
    let (w, x, y, rw) = (old.width() as usize, rect.x as usize, rect.y as usize, rect.w as usize);
    let mut out = vec![0; rw * rect.h as usize * 4];
    if out.is_empty() {
        return RgbaImage::new(rect.w, rect.h);
    }
    let (a, b) = (old.as_raw().as_chunks::<4>().0, new.as_raw().as_chunks::<4>().0);
    // Bands of rows in parallel: a 4K image is built in a few milliseconds when Difference is first shown.
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let band = (rect.h as usize).div_ceil(threads).max(64);
    std::thread::scope(|scope| {
        for (i, chunk) in out.chunks_mut(band * rw * 4).enumerate() {
            scope.spawn(move || {
                for (j, line) in chunk.as_chunks_mut::<4>().0.chunks_exact_mut(rw).enumerate() {
                    let start = (y + i * band + j) * w + x;
                    for ((out, a), b) in line.iter_mut().zip(&a[start..start + rw]).zip(&b[start..start + rw]) {
                        *out = difference_of(*a, *b);
                    }
                }
            });
        }
    });
    RgbaImage::from_raw(rect.w, rect.h, out).expect("one pixel per rect cell")
}

/// An image's rows as pixels.
fn rows(img: &RgbaImage) -> impl Iterator<Item = &[[u8; 4]]> {
    let width = img.width().max(1) as usize;
    img.as_raw().as_chunks::<4>().0.chunks_exact(width)
}

fn difference_of(a: [u8; 4], b: [u8; 4]) -> [u8; 4] {
    if differs(a, b) {
        return CHANGED.0;
    }
    let luma = 0.3 * f32::from(b[0]) + 0.59 * f32::from(b[1]) + 0.11 * f32::from(b[2]);
    let grey = (luma * 0.35) as u8;
    [grey, grey, grey, b[3]]
}

fn differs_at(a: Option<&Rgba<u8>>, b: Option<&Rgba<u8>>) -> bool {
    !matches!((a, b), (Some(a), Some(b)) if !differs(a.0, b.0))
}

fn differs(a: [u8; 4], b: [u8; 4]) -> bool {
    if a[3] == 0 && b[3] == 0 {
        return false; // colour under full transparency is invisible
    }
    let sum: u32 = a.iter().zip(b).map(|(&p, q)| u32::from(p.abs_diff(q))).sum();
    sum > NOISE
}

/// 8-connected groups of hot grid cells as pixel rectangles, clipped to the image.
fn regions(hot: &[bool], gw: u32, gh: u32, w: u32, h: u32) -> Vec<Rect> {
    let mut seen = vec![false; hot.len()];
    let mut out = Vec::new();
    for start in 0..hot.len() {
        if !hot[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        let mut stack = vec![start];
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
        while let Some(c) = stack.pop() {
            let (cx, cy) = (c as u32 % gw, c as u32 / gw);
            (x0, y0, x1, y1) = (x0.min(cx), y0.min(cy), x1.max(cx), y1.max(cy));
            for ny in cy.saturating_sub(1)..=(cy + 1).min(gh - 1) {
                for nx in cx.saturating_sub(1)..=(cx + 1).min(gw - 1) {
                    let n = (ny * gw + nx) as usize;
                    if hot[n] && !seen[n] {
                        seen[n] = true;
                        stack.push(n);
                    }
                }
            }
        }
        let (x, y) = (x0 * REGION_CELL, y0 * REGION_CELL);
        out.push(Rect {
            x,
            y,
            w: ((x1 + 1) * REGION_CELL).min(w) - x,
            h: ((y1 + 1) * REGION_CELL).min(h) - y,
        });
    }
    out.sort_by_key(|r| (r.y, r.x));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    const RED: Rgba<u8> = Rgba([255, 0, 0, 255]);
    const BLUE: Rgba<u8> = Rgba([0, 0, 255, 255]);
    #[test]
    fn onion_skin_blends_the_two_sides() {
        let old = RgbaImage::from_pixel(2, 1, RED);
        let new = RgbaImage::from_pixel(3, 1, BLUE);
        let half = blend(&old, &new, 0.5);
        assert_eq!(half.dimensions(), (3, 1));
        assert_eq!(*half.get_pixel(0, 0), Rgba([128, 0, 128, 255]));
        assert_eq!(*blend(&old, &new, 0.0).get_pixel(0, 0), RED);
        assert_eq!(*blend(&old, &new, 1.0).get_pixel(0, 0), BLUE);
        // where only one side has pixels, that side fades with its weight
        assert_eq!(blend(&old, &new, 0.5).get_pixel(2, 0)[3], 128);
    }

    /// Embedded raster data in an SVG is drawn in GUI builds; the
    /// shared decoder's bounds are tested in `lenz_content::image`.
    #[cfg(feature = "gui")]
    #[test]
    fn embedded_png_data_is_drawn_in_an_svg() {
        use image::ImageEncoder;
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png).write_image(&RED.0, 1, 1, image::ExtendedColorType::Rgba8).unwrap();
        let data_uri = png.iter().map(|b| format!("%{b:02X}")).collect::<String>();
        let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="1" height="1"><image width="1" height="1" xlink:href="data:image/png,{data_uri}"/></svg>"#);
        assert_eq!(*decode(svg.as_bytes(), Format::Svg).unwrap().pixels.get_pixel(0, 0), RED);
    }

    fn grey(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba([100, 100, 100, 255]))
    }

    #[test]
    fn identical_images_have_no_changes() {
        let d = diff(&grey(30, 20), &grey(30, 20));
        assert_eq!((d.changed, d.total, d.regions.len()), (0, 600, 0));
        let p = difference(&grey(30, 20), &grey(30, 20), Rect { x: 0, y: 0, w: 30, h: 20 }).get_pixel(0, 0).to_owned();
        assert!(
            p[0] < 100 && p[0] == p[1] && p[3] == 255,
            "dimmed grey, alpha kept: {p:?}"
        );
    }

    #[test]
    fn small_noise_is_ignored() {
        let mut new = grey(10, 10);
        new.put_pixel(0, 0, Rgba([104, 104, 104, 255])); // sum 12
        new.put_pixel(1, 0, Rgba([105, 104, 104, 255])); // sum 13
        let d = diff(&grey(10, 10), &new);
        assert_eq!(d.changed, 1);
        let image = difference(&grey(10, 10), &new, Rect { x: 0, y: 0, w: 10, h: 10 });
        assert_eq!(*image.get_pixel(1, 0), CHANGED);
        assert_ne!(*image.get_pixel(0, 0), CHANGED);
        // a part of the image is the same pixels as the whole
        assert_eq!(*difference(&grey(10, 10), &new, Rect { x: 1, y: 0, w: 2, h: 1 }).get_pixel(0, 0), CHANGED);
    }

    #[test]
    fn the_difference_image_matches_its_pixels_in_any_rect() {
        let px = |x: u32, y: u32, k: u32| Rgba([(x * k) as u8, (y * 3) as u8, (x ^ y) as u8, if (x + y).is_multiple_of(7) { 0 } else { 255 }]);
        let old = RgbaImage::from_fn(130, 200, |x, y| px(x, y, 5));
        let new = RgbaImage::from_fn(130, 200, |x, y| px(x, y, if y.is_multiple_of(3) { 6 } else { 5 }));
        let smaller = RgbaImage::from_fn(90, 150, |x, y| px(x, y, 5));
        for (old, new) in [(&old, &new), (&old, &smaller)] {
            let d = diff(old, new);
            let whole = Rect { x: 0, y: 0, w: 130, h: 200 };
            let mut changed = 0;
            for rect in [whole, Rect { x: 3, y: 5, w: 100, h: 190 }, Rect { x: 129, y: 199, w: 1, h: 1 }] {
                let image = difference(old, new, rect);
                for (x, y, p) in image.enumerate_pixels() {
                    let (x, y) = (rect.x + x, rect.y + y);
                    assert_eq!(*p, difference_pixel(old.get_pixel_checked(x, y), new.get_pixel_checked(x, y)), "({x}, {y})");
                    changed += u64::from(rect == whole && *p == CHANGED);
                }
            }
            assert_eq!(d.changed, changed);
        }
    }

    #[test]
    fn fully_transparent_pixels_are_equal_whatever_their_colour() {
        let d = diff(
            &RgbaImage::from_pixel(2, 2, Rgba([255, 0, 0, 0])),
            &RgbaImage::from_pixel(2, 2, Rgba([0; 4])),
        );
        assert_eq!(d.changed, 0);
    }

    fn with_blobs(w: u32, h: u32, blobs: &[(u32, u32)]) -> RgbaImage {
        let mut img = grey(w, h);
        for &(bx, by) in blobs {
            for y in by..by + 3 {
                for x in bx..bx + 3 {
                    img.put_pixel(x, y, RED);
                }
            }
        }
        img
    }

    #[test]
    fn separate_blobs_are_separate_regions_in_reading_order() {
        let d = diff(&grey(200, 200), &with_blobs(200, 200, &[(150, 100), (5, 5)]));
        assert_eq!(d.changed, 18);
        assert_eq!(
            d.regions,
            [
                Rect {
                    x: 0,
                    y: 0,
                    w: 24,
                    h: 24
                },
                Rect {
                    x: 144,
                    y: 96,
                    w: 24,
                    h: 24
                }
            ]
        );
    }

    #[test]
    fn diagonally_touching_cells_are_one_region() {
        let d = diff(&grey(100, 100), &with_blobs(100, 100, &[(10, 10), (30, 30)]));
        assert_eq!(
            d.regions,
            [Rect {
                x: 0,
                y: 0,
                w: 48,
                h: 48
            }]
        );
    }

    #[test]
    fn regions_are_clipped_to_the_image() {
        let d = diff(&grey(50, 30), &with_blobs(50, 30, &[(47, 27)]));
        assert_eq!(
            d.regions,
            [Rect {
                x: 24,
                y: 24,
                w: 26,
                h: 6
            }]
        ); // cells end at 72 × 48
    }

    #[test]
    fn different_sizes_compare_on_the_union() {
        let d = diff(&grey(10, 10), &grey(20, 10));
        assert_eq!((d.changed, d.total), (100, 200));
        assert_eq!(
            d.regions,
            [Rect {
                x: 0,
                y: 0,
                w: 20,
                h: 10
            }]
        );
        // An absent Side: everything changed.
        let d = diff(&RgbaImage::new(0, 0), &grey(5, 5));
        assert_eq!((d.changed, d.total), (25, 25));
    }

    #[test]
    #[ignore = "timing: run in release with --ignored --nocapture"]
    fn image_perf() {
        use std::{hint::black_box, time::Instant};
        let old = RgbaImage::from_pixel(4096, 4096, Rgba([24, 32, 48, 255]));
        let new = RgbaImage::from_pixel(4096, 4096, Rgba([200, 80, 60, 255]));
        let mut times = Vec::new();
        for _ in 0..5 {
            let start = Instant::now();
            let out = black_box(blend(&old, &new, black_box(0.5)));
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(*out.get_pixel(2048, 2048), Rgba([112, 56, 54, 255]));
        }
        crate::perf::record("image/4k-blend", "ms", &times);
        times.sort_by(f64::total_cmp);
        eprintln!("4K onion blend: median {:.3} ms", times[2]);
        let start = Instant::now();
        let d = diff(&old, &new);
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        crate::perf::record("image/4k-diff", "ms", &[ms]);
        eprintln!("4K image diff: {ms:.3} ms");
        assert_eq!(d.changed, 4096 * 4096);
        // built when Difference mode is first shown
        let start = Instant::now();
        let image = black_box(difference(&old, &new, Rect { x: 0, y: 0, w: 4096, h: 4096 }));
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        crate::perf::record("image/4k-difference-image", "ms", &[ms]);
        eprintln!("4K difference image: {ms:.3} ms");
        assert_eq!(*image.get_pixel(0, 0), CHANGED);
        crate::perf::check(times[2] < 80.0, "4K blend exceeds 80 ms");
    }

    #[test]
    fn large_blends_round_every_channel_pair_including_transparency() {
        let old = RgbaImage::from_raw(16_384, 1, (0..65_536).map(|i| (i >> 8) as u8).collect()).unwrap();
        let new = RgbaImage::from_raw(16_384, 1, (0..65_536).map(|i| i as u8).collect()).unwrap();
        // These binary-exact opacities have an independent integer rounding oracle.
        for quarter in 0..=4u32 {
            let out = blend(&old, &new, quarter as f32 / 4.0);
            for (i, &value) in out.as_raw().iter().enumerate() {
                let (a, b) = ((i >> 8) as u32, (i & 255) as u32);
                assert_eq!(u32::from(value), ((4 - quarter) * a + quarter * b + 2) / 4);
            }
        }
    }
}
