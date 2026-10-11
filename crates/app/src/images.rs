//! Images in a Comparison, without gpui: format detection, decoding to RGBA, the pixel difference and its
//! **Changed Regions**, and the scaling helpers the panes need. The UI turns the RGBA buffers into gpui
//! `RenderImage`s with [`swap_rb`] (gpui wants BGRA).
//!
//! SVG is rasterised in this shared decoder. External image references are disabled:
//! repository content must not read arbitrary local files or block on special files.


use std::io::Cursor;

use image::codecs::webp::WebPDecoder;
use image::codecs::{bmp::BmpDecoder, gif::GifDecoder, ico::IcoDecoder, jpeg::JpegDecoder, png::PngDecoder};
use image::imageops::{self, FilterType};
use image::{DynamicImage, ImageDecoder, ImageResult, Limits, Rgba, RgbaImage};

/// Largest side drawn: bigger textures are silently left blank by gpui's atlas, so we downscale.
pub const MAX_SIDE: u32 = 8192;
/// Changed Regions group changed pixels on a grid of this many pixels per cell.
pub const REGION_CELL: u32 = 24;
/// A pixel counts as changed when the sum of its four channel differences exceeds this (encoder noise).
pub const NOISE: u32 = 12;
const CHANGED: Rgba<u8> = Rgba([255, 40, 200, 255]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
    Gif,
    WebP,
    Bmp,
    Ico,
    Svg,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::Png => "PNG",
            Format::Jpeg => "JPEG",
            Format::Gif => "GIF",
            Format::WebP => "WebP",
            Format::Bmp => "BMP",
            Format::Ico => "ICO",
            Format::Svg => "SVG",
        }
    }
}

/// The image format by magic bytes, or SVG by extension or a `<svg` in the first KB. `None`: not an image
/// we show (other binaries keep the placeholder).
pub fn detect(path: &str, bytes: &[u8]) -> Option<Format> {
    use image::ImageFormat as F;
    match image::guess_format(bytes) {
        Ok(F::Png) => return Some(Format::Png),
        Ok(F::Jpeg) => return Some(Format::Jpeg),
        Ok(F::Gif) => return Some(Format::Gif),
        Ok(F::WebP) => return Some(Format::WebP),
        Ok(F::Bmp) => return Some(Format::Bmp),
        Ok(F::Ico) => return Some(Format::Ico),
        _ => {}
    }
    let svg_ext = std::path::Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"));
    let head = &bytes[..bytes.len().min(1024)];
    (svg_ext || head.windows(4).any(|w| w == b"<svg")).then_some(Format::Svg)
}

/// One Side's image, ready to draw.
pub struct Decoded {
    pub format: Format,
    /// As displayed (EXIF orientation applied), before any downscale.
    pub width: u32,
    pub height: u32,
    pub byte_size: usize,
    /// Downscaled to fit [`MAX_SIDE`]: the header shows a notice.
    pub downscaled: bool,
    pub pixels: RgbaImage,
}

/// Decodes the first frame to RGBA with EXIF orientation applied (SVG rasterised), capped at [`MAX_SIDE`].
pub fn decode(bytes: &[u8], format: Format) -> Result<Decoded, String> {
    let c = Cursor::new(bytes);
    let decoded = match format {
        // Naming the six decoders (not `ImageReader`) keeps exr/tiff/… out of the binary (~360 KB).
        Format::Png => first_frame(PngDecoder::with_limits(c, Limits::default())),
        Format::Jpeg => first_frame(JpegDecoder::new(c)),
        // `from_decoder` reads only frame 0 of an animation.
        Format::Gif => first_frame(GifDecoder::new(c)),
        Format::WebP => first_frame(WebPDecoder::new(c)),
        Format::Bmp => first_frame(BmpDecoder::new(c)),
        Format::Ico => first_frame(IcoDecoder::new(c)),
        Format::Svg => svg(bytes),
    };
    let pixels = decoded.map_err(|e| format!("{} decode failed: {e}", format.name()))?;
    let (width, height) = pixels.dimensions();
    let (pixels, downscaled) = cap(pixels);
    Ok(Decoded {
        format,
        width,
        height,
        byte_size: bytes.len(),
        downscaled,
        pixels,
    })
}

/// An SVG drawn at its own size (capped at [`MAX_SIDE`]).
fn svg(bytes: &[u8]) -> ImageResult<RgbaImage> {
    use resvg::{tiny_skia, usvg};
    let fail = |e: String| image::ImageError::Decoding(image::error::DecodingError::new(image::error::ImageFormatHint::Name("SVG".into()), e));
    // ponytail: no system fonts are loaded, so text inside an SVG isn't drawn; load fontdb if that matters
    let options = usvg::Options {
        image_href_resolver: svg_image_resolver(),
        ..Default::default()
    };
    // usvg inflates gzip (`.svgz`) whole and unbounded: a small file could become gigabytes of XML.
    if bytes.starts_with(&[0x1f, 0x8b]) {
        return Err(fail("compressed SVG (svgz) is not shown".into()));
    }
    let tree = usvg::Tree::from_data(bytes, &options).map_err(|e| fail(e.to_string()))?;
    let size = tree.size().to_int_size();
    let (w, h) = (size.width().min(MAX_SIDE), size.height().min(MAX_SIDE));
    let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or_else(|| fail("empty SVG".into()))?;
    let scale = (w as f32 / size.width() as f32).min(h as f32 / size.height() as f32);
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    // tiny-skia is premultiplied; images here are straight alpha
    let mut out = RgbaImage::new(w, h);
    for (px, c) in out.pixels_mut().zip(pixmap.pixels()) {
        let c = c.demultiply();
        *px = Rgba([c.red(), c.green(), c.blue(), c.alpha()]);
    }
    Ok(out)
}

/// Embedded raster data is decoded by resvg, bypassing `first_frame`. Apply the
/// same header-only RGBA bound before allowing it into the tree (including nested SVGs).
fn svg_image_resolver() -> resvg::usvg::ImageHrefResolver<'static> {
    use resvg::usvg::{ImageHrefResolver, ImageKind};
    let resolve_data = ImageHrefResolver::default_data_resolver();
    ImageHrefResolver {
        resolve_string: Box::new(|_, _| None),
        resolve_data: Box::new(move |mime, data, options| {
            let image = resolve_data(mime, data, options)?;
            let allowed = match &image {
                // PNG metadata is decompressed by the constructor, before the pixel bounds below.
                ImageKind::PNG(data) => bounded_decoder(PngDecoder::with_limits(Cursor::new(data.as_slice()), Limits::default())).is_ok(),
                ImageKind::JPEG(data) => bounded_decoder(JpegDecoder::new(Cursor::new(data.as_slice()))).is_ok(),
                ImageKind::GIF(data) => bounded_decoder(GifDecoder::new(Cursor::new(data.as_slice()))).is_ok(),
                ImageKind::WEBP(data) => bounded_decoder(WebPDecoder::new(Cursor::new(data.as_slice()))).is_ok(),
                ImageKind::SVG(_) => true,
            };
            allowed.then_some(image)
        }),
    }
}

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

fn bounded_decoder<D: ImageDecoder>(decoder: ImageResult<D>) -> ImageResult<D> {
    let d = decoder?;
    let (width, height) = d.dimensions();
    // Decoder limits cover its native color format, not the later RGBA expansion.
    Limits::default().reserve_buffer(width, height, image::ColorType::Rgba8)?;
    Ok(d)
}

fn first_frame<D: ImageDecoder>(decoder: ImageResult<D>) -> ImageResult<RgbaImage> {
    let mut d = bounded_decoder(decoder)?;
    d.set_limits(Limits::default())?; // the bare constructors have none: keep the 512 MiB bomb guard
    let orientation = d.orientation()?;
    let mut img = DynamicImage::from_decoder(d)?;
    img.apply_orientation(orientation);
    Ok(img.into_rgba8())
}

/// Downscales to fit [`MAX_SIDE`] a side, keeping the aspect ratio; `true` when it did.
pub fn cap(img: RgbaImage) -> (RgbaImage, bool) {
    let (w, h) = img.dimensions();
    let side = w.max(h);
    if side <= MAX_SIDE {
        return (img, false);
    }
    let scale = |n: u32| ((u64::from(n) * u64::from(MAX_SIDE) / u64::from(side)) as u32).max(1);
    (imageops::resize(&img, scale(w), scale(h), FilterType::Triangle), true)
}

/// RGBA ↔ BGRA in place (gpui's `RenderImage` is BGRA); the same swap goes both ways.
pub fn swap_rb(buf: &mut [u8]) {
    for px in buf.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
}

/// A rectangle in image pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Nearest-neighbour upscale of `rect` (clipped to the image) by `factor`: sharp pixels above 300 % zoom,
/// where gpui only samples bilinearly.
pub fn upscale_nearest<C: std::ops::Deref<Target = [u8]>>(src: &image::ImageBuffer<Rgba<u8>, C>, rect: Rect, factor: u32) -> RgbaImage {
    let x = rect.x.min(src.width());
    let y = rect.y.min(src.height());
    let w = rect.w.min(src.width() - x);
    let h = rect.h.min(src.height() - y);
    RgbaImage::from_fn(w * factor, h * factor, |px, py| {
        *src.get_pixel(x + px / factor, y + py / factor)
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
    use image::codecs::gif::GifEncoder;
    use image::codecs::jpeg::JpegEncoder;
    use image::{ExtendedColorType, Frame, ImageEncoder, ImageFormat};

    const RED: Rgba<u8> = Rgba([255, 0, 0, 255]);
    const BLUE: Rgba<u8> = Rgba([0, 0, 255, 255]);

    #[test]
    fn svg_is_rasterised_at_its_own_size() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect x="0" y="0" width="10" height="10" fill="#ff0000"/></svg>"##;
        let d = decode(svg, Format::Svg).unwrap();
        assert_eq!((d.width, d.height, d.format), (20, 10, Format::Svg));
        assert_eq!(*d.pixels.get_pixel(2, 2), RED);
        assert_eq!(d.pixels.get_pixel(15, 5)[3], 0, "transparent where nothing is drawn");
        assert!(decode(b"<svg nonsense", Format::Svg).is_err());
    }

    #[test]
    fn svg_does_not_read_external_images_but_keeps_embedded_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("private.png");
        RgbaImage::from_pixel(1, 1, RED).save(&path).unwrap();
        let external = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="1" height="1"><image width="1" height="1" xlink:href="{}"/></svg>"#, path.display());
        assert_eq!(decode(external.as_bytes(), Format::Svg).unwrap().pixels.get_pixel(0, 0)[3], 0);
        let embedded = br#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="1" height="1"><image width="1" height="1" xlink:href="data:image/svg+xml,%3Csvg%20xmlns='http://www.w3.org/2000/svg'%20width='1'%20height='1'%3E%3Crect%20width='1'%20height='1'%20fill='red'/%3E%3C/svg%3E"/></svg>"#;
        assert_eq!(*decode(embedded, Format::Svg).unwrap().pixels.get_pixel(0, 0), RED);
    }

    #[test]
    fn embedded_raster_dimensions_are_checked_without_decoding_pixels() {
        use std::sync::Arc;
        use resvg::usvg::{ImageHrefResolver, ImageKind, Options};
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&RED.0, 1, 1, ExtendedColorType::Rgba8).unwrap();
        let options = Options { image_href_resolver: svg_image_resolver(), ..Default::default() };
        let resolve = |data| (options.image_href_resolver.resolve_data)("image/png", Arc::new(data), &options);
        assert!(matches!(resolve(png.clone()), Some(ImageKind::PNG(_))));
        #[cfg(feature = "gui")]
        {
            let data_uri = png.iter().map(|b| format!("%{b:02X}")).collect::<String>();
            let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="1" height="1"><image width="1" height="1" xlink:href="data:image/png,{data_uri}"/></svg>"#);
            assert_eq!(*decode(svg.as_bytes(), Format::Svg).unwrap().pixels.get_pixel(0, 0), RED);
        }
        // Only change IHDR and its checksum: this fixture stays tiny and no pixel
        // decoding is attempted. The default resolver accepts the oversized header.
        png[16..20].copy_from_slice(&12_000u32.to_be_bytes());
        png[20..24].copy_from_slice(&12_000u32.to_be_bytes());
        let mut crc = !0u32;
        for &byte in &png[12..29] {
            crc ^= u32::from(byte);
            for _ in 0..8 { crc = (crc >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(crc & 1)); }
        }
        png[29..33].copy_from_slice(&(!crc).to_be_bytes());
        assert_eq!(PngDecoder::new(Cursor::new(&png)).unwrap().dimensions(), (12_000, 12_000));
        assert!(ImageHrefResolver::default_data_resolver()("image/png", Arc::new(png.clone()), &Options::default()).is_some());
        assert!(resolve(png).is_none());
    }

    #[test]
    fn rgba_expansion_is_limited_before_decoding() {
        struct LargeGray;
        impl ImageDecoder for LargeGray {
            fn dimensions(&self) -> (u32, u32) { (20_000, 20_000) }
            fn color_type(&self) -> image::ColorType { image::ColorType::L8 }
            fn read_image(self, _: &mut [u8]) -> ImageResult<()> { panic!("must reject before allocating or reading pixels") }
            fn read_image_boxed(self: Box<Self>, buf: &mut [u8]) -> ImageResult<()> { self.read_image(buf) }
        }
        assert!(matches!(first_frame(Ok(LargeGray)), Err(image::ImageError::Limits(_))));
    }

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

    /// 4×2: left half red, right half blue.
    fn pattern() -> RgbaImage {
        RgbaImage::from_fn(4, 2, |x, _| if x < 2 { RED } else { BLUE })
    }

    fn encode(img: DynamicImage, format: ImageFormat) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, format).unwrap();
        out.into_inner()
    }

    #[test]
    fn lossless_formats_round_trip() {
        for (format, want) in [
            (ImageFormat::Png, Format::Png),
            (ImageFormat::WebP, Format::WebP),
            (ImageFormat::Bmp, Format::Bmp),
            (ImageFormat::Ico, Format::Ico),
            (ImageFormat::Gif, Format::Gif),
        ] {
            let bytes = encode(pattern().into(), format);
            assert_eq!(detect("a.bin", &bytes), Some(want), "{format:?}");
            let d = decode(&bytes, want).unwrap();
            assert_eq!((d.format, d.width, d.height, d.byte_size), (want, 4, 2, bytes.len()));
            assert!(!d.downscaled);
            assert_eq!(d.pixels, pattern(), "{format:?}");
        }
    }

    fn close(a: &Rgba<u8>, b: Rgba<u8>) -> bool {
        a.0.iter().zip(b.0).all(|(&p, q)| p.abs_diff(q) < 40)
    }

    #[test]
    fn jpeg_decodes() {
        let img = RgbaImage::from_fn(16, 16, |x, _| if x < 8 { RED } else { BLUE });
        let bytes = encode(DynamicImage::from(img).to_rgb8().into(), ImageFormat::Jpeg);
        assert_eq!(detect("photo", &bytes), Some(Format::Jpeg));
        let d = decode(&bytes, Format::Jpeg).unwrap();
        assert_eq!((d.width, d.height), (16, 16));
        assert!(close(d.pixels.get_pixel(1, 8), RED) && close(d.pixels.get_pixel(14, 8), BLUE));
    }

    #[test]
    fn jpeg_exif_orientation_is_applied() {
        // A little-endian TIFF chunk with one IFD entry: Orientation (0x0112) = 6, "rotate 90° clockwise".
        let exif = [
            b'I', b'I', 42, 0, 8, 0, 0, 0, // header, IFD at 8
            1, 0, // one entry
            0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, // tag, SHORT, count 1, value 6
            0, 0, 0, 0, // no next IFD
        ];
        let img = DynamicImage::from(RgbaImage::from_fn(32, 16, |x, _| if x < 16 { RED } else { BLUE })).to_rgb8();
        let mut bytes = Vec::new();
        let mut enc = JpegEncoder::new(&mut bytes);
        enc.set_exif_metadata(exif.to_vec()).unwrap();
        enc.write_image(&img, 32, 16, ExtendedColorType::Rgb8).unwrap();

        let d = decode(&bytes, Format::Jpeg).unwrap();
        assert_eq!((d.width, d.height), (16, 32));
        // Clockwise: the left half becomes the top.
        assert!(close(d.pixels.get_pixel(8, 4), RED) && close(d.pixels.get_pixel(8, 28), BLUE));
    }

    #[test]
    fn gif_decodes_the_first_frame_only() {
        let first = pattern();
        let second = RgbaImage::from_pixel(4, 2, Rgba([0, 255, 0, 255]));
        let mut bytes = Vec::new();
        GifEncoder::new(&mut bytes)
            .encode_frames([Frame::new(first.clone()), Frame::new(second)])
            .unwrap();
        let d = decode(&bytes, Format::Gif).unwrap();
        assert_eq!(d.pixels, first);
    }

    #[test]
    fn svg_is_detected_by_extension_or_tag() {
        let svg = br#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" width="4" height="2"/>"#;
        assert_eq!(detect("icon.svg", svg), Some(Format::Svg));
        assert_eq!(detect("icon.xml", svg), Some(Format::Svg));
        assert_eq!(detect("ICON.SVG", b"<?xml?>"), Some(Format::Svg));
        assert_eq!(detect("notes.txt", b"hello"), None);
        assert_eq!(detect("data.bin", &[0, 1, 2, 3]), None);
    }

    #[test]
    fn corrupt_bytes_are_an_error() {
        let mut bytes = encode(pattern().into(), ImageFormat::Png);
        bytes.truncate(20);
        assert!(decode(&bytes, Format::Png).is_err());
    }

    #[test]
    fn over_the_cap_is_downscaled() {
        let bytes = encode(RgbaImage::from_pixel(MAX_SIDE * 2 + 2, 3, RED).into(), ImageFormat::Png);
        let d = decode(&bytes, Format::Png).unwrap();
        assert_eq!((d.width, d.height), (MAX_SIDE * 2 + 2, 3));
        assert!(d.downscaled);
        assert_eq!(d.pixels.dimensions(), (MAX_SIDE, 1));
        assert!(!cap(RgbaImage::new(MAX_SIDE, 1)).1);
    }

    #[test]
    fn swap_rb_converts_rgba_to_bgra_and_back() {
        let mut buf = vec![1, 2, 3, 4, 5, 6, 7, 8];
        swap_rb(&mut buf);
        assert_eq!(buf, [3, 2, 1, 4, 7, 6, 5, 8]);
        swap_rb(&mut buf);
        assert_eq!(buf, [1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn upscale_is_nearest_neighbour_and_clipped() {
        let src = pattern();
        let up = upscale_nearest(&src, Rect { x: 1, y: 0, w: 2, h: 1 }, 3);
        assert_eq!(up.dimensions(), (6, 3));
        assert_eq!(*up.get_pixel(2, 2), RED);
        assert_eq!(*up.get_pixel(3, 0), BLUE);
        assert_eq!(
            upscale_nearest(
                &src,
                Rect {
                    x: 3,
                    y: 1,
                    w: 10,
                    h: 10
                },
                2
            )
            .dimensions(),
            (2, 2)
        );
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
