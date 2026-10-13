//! Single images, without gpui: format detection, bounded decoding to RGBA (the first frame, EXIF orientation
//! applied, SVG rasterised), the size cap and the pixel helpers presentation needs. Both Lenz (for its image
//! comparisons) and the code reader use it; comparisons themselves stay in Lenz.
//!
//! SVG is rasterised here. External image references are disabled: repository content must not read arbitrary
//! local files or block on special files.

use std::io::Cursor;

use image::codecs::webp::WebPDecoder;
use image::codecs::{bmp::BmpDecoder, gif::GifDecoder, ico::IcoDecoder, jpeg::JpegDecoder, png::PngDecoder};
use image::imageops::{self, FilterType};
use image::{DynamicImage, ImageDecoder, ImageResult, Limits, Rgba, RgbaImage};

/// Decoded pixels and an animation frame (what GPUI textures are made of), re-exported so consumers need no image
/// dependency of their own.
pub use image::{Frame, RgbaImage as Pixels};

/// Largest side drawn: bigger textures are silently left blank by gpui's atlas, so we downscale.
pub const MAX_SIDE: u32 = 8192;

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

/// The image format a file is shown as: raster formats only for binary content (so text that happens to begin
/// with an image's magic stays text), SVG only with the `.svg` extension or (in binary content) its tag.
/// `binary` is git's rule, [`crate::encoding::looks_binary`], or what attributes say.
pub fn detect_file(path: &str, bytes: &[u8], binary: bool) -> Option<Format> {
    let svg = path.to_ascii_lowercase().ends_with(".svg");
    (binary || svg).then(|| detect(path, bytes)).flatten()
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
        // That such data is drawn depends on resvg's raster features, which GPUI enables: Lenz's GUI tests it.
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
    fn only_binary_content_or_an_svg_file_is_an_image() {
        let bm = b"BM is a fine way to start a note\n";
        assert_eq!(detect("notes.txt", bm), Some(Format::Bmp), "the bare magic test matches text");
        assert_eq!(detect_file("notes.txt", bm, crate::encoding::looks_binary(bm)), None, "but text stays text");
        let html = b"<html><svg/></html>";
        assert_eq!(detect_file("page.html", html, false), None);
        assert_eq!(detect_file("icon.SVG", b"<svg/>", false), Some(Format::Svg));
        let png = encode(DynamicImage::ImageRgba8(pattern()), ImageFormat::Png);
        assert_eq!(detect_file("x", &png, crate::encoding::looks_binary(&png)), Some(Format::Png));
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
}
