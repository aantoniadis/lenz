//! One opened source as an immutable snapshot: decoded text shared by reference, its line offsets, how it was
//! decoded and whether it is large. Coordinates are UTF-8 byte offsets into the preserved text.

use std::{ops::Range, path::PathBuf, sync::Arc};

use lenz_content::{encoding, highlight::{self, Language}, image as img, lines};

/// Above this many bytes a document is large: it is read in full, but syntax work is reduced.
pub const LARGE_BYTES: usize = 5 << 20;
/// Above this many lines a document is large.
pub const LARGE_LINES: usize = 100_000;
/// Image files above this many bytes are not decoded (their size is shown instead).
pub const IMAGE_BYTES: usize = 64 << 20;

/// An image a document shows: its format and size, and its pixels (capped at [`img::MAX_SIDE`] a side), or why
/// they cannot be shown.
#[derive(Debug)]
pub struct Image {
    pub format: img::Format,
    /// As displayed (EXIF orientation applied), before any downscale; 0 when not decoded.
    pub width: u32,
    pub height: u32,
    pub byte_size: usize,
    pub downscaled: bool,
    pub pixels: Result<Arc<img::Pixels>, String>,
}

/// Immutable source/decode facts shared by draft snapshots and worker Documents.
#[derive(Debug, Clone)]
pub struct SourceMeta {
    source: PathBuf,
    encoding: String,
    original_encoding: &'static str,
    had_bom: bool,
    shown_encoding: bool,
    lossy: bool,
    binary: bool,
    language: Option<Language>,
    image: Option<Arc<Image>>,
    fingerprint: u64,
    /// Reader restrictions inherited from the original encoded load.
    large: bool,
}

impl SourceMeta {
    pub(crate) fn is_large(&self) -> bool { self.large }
    pub(crate) fn renamed(&self, source: PathBuf) -> Self { Self { source, ..self.clone() } }
    pub(crate) fn bytes(&self) -> usize {
        size_of::<Self>() + self.source.as_os_str().len() + self.encoding.len()
            + self.image.as_ref().map_or(0, |i| size_of::<Image>() + i.pixels.as_ref().map_or(0, |p| p.as_raw().len()))
    }
    pub fn source(&self) -> &std::path::Path { &self.source }
    pub fn encoding(&self) -> &str { &self.encoding }
    pub fn original_encoding(&self) -> &'static str { self.original_encoding }
    pub fn had_bom(&self) -> bool { self.had_bom }
    pub fn is_lossy(&self) -> bool { self.lossy }
    pub fn is_binary(&self) -> bool { self.binary }
    pub fn language(&self) -> Option<Language> { self.language }
    pub fn image(&self) -> Option<&Arc<Image>> { self.image.as_ref() }
    pub(crate) fn notices(&self, large: bool) -> Vec<String> {
        let mut notices = Vec::new();
        if self.shown_encoding {
            notices.push(format!("Encoding: {}", self.encoding));
        }
        if self.lossy {
            notices.push(encoding::LOSSY_NOTICE.into());
        }
        if large {
            notices.push("Large file: syntax highlighting off".into());
        }
        if let Some(i) = &self.image {
            notices.push(match &i.pixels {
                Ok(_) => {
                    let scaled = if i.downscaled { format!(", shown downscaled to fit {} px", img::MAX_SIDE) } else { String::new() };
                    format!("{} image, {}×{} px, {}{scaled}", i.format.name(), i.width, i.height, size_label(i.byte_size))
                }
                Err(e) => format!("{} image, {}: cannot be shown ({e})", i.format.name(), size_label(i.byte_size)),
            });
        }
        notices
    }

}

#[derive(Debug)]
pub struct Document {
    source: Arc<SourceMeta>,
    version: u64,
    text: Arc<str>,
    line_starts: Arc<[usize]>,
    large: bool,
    /// Where language servers' lines start, when they differ from the reader's: a CR not followed by LF ends a line
    /// for them, not for the reader. Found when a language server first needs it, not at every open.
    protocol_lines: std::sync::OnceLock<Option<Arc<[usize]>>>,
}

impl Document {
    /// Decode `bytes` read from `source` as its `version`th snapshot.
    pub fn decode(source: PathBuf, version: u64, bytes: &[u8]) -> Self {
        // An image: decoded here (off the UI thread, like every load), bounded; a raster image has no text. A raster
        // format only for binary content (git's rule, as Lenz decides), so text that happens to start like an
        // image stays text; an SVG by its extension (Lenz's rule, shared).
        let binary = encoding::looks_binary(bytes);
        let format = img::detect_file(&source.to_string_lossy(), bytes, binary);
        let image = format.map(|format| {
            let pixels = if bytes.len() > IMAGE_BYTES {
                Err(format!("{} MiB is too large to show (the limit is {} MiB)", bytes.len() >> 20, IMAGE_BYTES >> 20))
            } else {
                img::decode(bytes, format)
            };
            Arc::new(match pixels {
                Ok(d) => Image { format, width: d.width, height: d.height, byte_size: d.byte_size, downscaled: d.downscaled, pixels: Ok(Arc::new(d.pixels)) },
                Err(e) => Image { format, width: 0, height: 0, byte_size: bytes.len(), downscaled: false, pixels: Err(e) },
            })
        });
        let fingerprint = if image.is_some() { Self::fingerprint(bytes) } else { 0 };
        let raster = format.is_some_and(|f| f != img::Format::Svg);
        let bytes = if raster { &[][..] } else { bytes };
        let decoded = encoding::decode(bytes, None);
        let line_starts: Arc<[usize]> = lines::line_starts(&decoded.text).into();
        // A final line ending ends the last line rather than starting another.
        let lines = line_starts.len() - usize::from(decoded.text.ends_with('\n'));
        let large = bytes.len() > LARGE_BYTES || lines > LARGE_LINES;
        let first = &decoded.text[lines::line_range(&decoded.text, &line_starts, 0)];
        let language = highlight::detect(&source, first);
        Self {
            source: Arc::new(SourceMeta {
                encoding: decoded.label(),
                original_encoding: decoded.encoding,
                had_bom: decoded.had_bom,
                shown_encoding: decoded.shown_in_header(),
                lossy: decoded.lossy,
                binary,
                language,
                source,
                image,
                fingerprint,
                large,
            }),
            text: decoded.text.into(),
            line_starts,
            large,
            version,
            protocol_lines: std::sync::OnceLock::new(),
        }
    }

    /// Text decoded by the file loader with its declared encoding, without guessing again.
    /// Image/binary sources must use `decode` so their original bytes remain available.
    pub fn from_decoded(source: PathBuf, version: u64, decoded: encoding::Decoded) -> Self {
        let metadata = SourceMeta {
            encoding: decoded.label(),
            original_encoding: decoded.encoding,
            had_bom: decoded.had_bom,
            shown_encoding: decoded.shown_in_header(),
            lossy: decoded.lossy,
            binary: false,
            language: None,
            source,
            image: None,
            fingerprint: 0,
            large: false,
        };
        let mut document = Self::from_snapshot_parts(Arc::new(metadata), version, decoded.text.into());
        Arc::make_mut(&mut document.source).language = highlight::detect(document.source(), document.line(0));
        document
    }

    /// Construct an explicitly materialized worker document without decoding draft UTF-8 again.
    pub fn from_snapshot_parts(source: Arc<SourceMeta>, version: u64, text: Arc<str>) -> Self {
        let line_starts: Arc<[usize]> = lines::line_starts(&text).into();
        Self::from_indexed_snapshot_parts(source, version, text, line_starts)
    }

    /// The snapshot assembler supplies its already indexed newline positions; callers cannot provide
    /// an unrelated index through the public document boundary.
    pub(crate) fn from_indexed_snapshot_parts(source: Arc<SourceMeta>, version: u64, text: Arc<str>, line_starts: Arc<[usize]>) -> Self {
        let lines = line_starts.len() - usize::from(text.ends_with('\n'));
        let large = source.large || text.len() > LARGE_BYTES || lines > LARGE_LINES;
        Self { source, version, text, line_starts, large, protocol_lines: std::sync::OnceLock::new() }
    }

    pub fn source_meta(&self) -> &Arc<SourceMeta> { &self.source }

    /// FNV-1a of a file's bytes: whether an image file changed, without decoding it.
    pub fn fingerprint(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
    }

    /// The fingerprint of the bytes an image was decoded from (0 for text).
    pub fn content_fingerprint(&self) -> u64 {
        self.source.fingerprint
    }

    /// The image this document shows, if it is an image file.
    pub fn image(&self) -> Option<&Arc<Image>> {
        self.source.image.as_ref()
    }

    /// Whether it shows the same content as `other` (text, and for images the bytes they were decoded from).
    pub fn same_content(&self, other: &Document) -> bool {
        self.text == other.text && self.source.fingerprint == other.source.fingerprint
    }

    /// Text a language server returned for virtual source `source` (an opaque URI): its language is detected from
    /// `name` (the URI's file name).
    pub fn retrieved(source: PathBuf, name: &std::path::Path, version: u64, text: &str) -> Self {
        let mut doc = Self::decode(source, version, text.as_bytes());
        Arc::make_mut(&mut doc.source).language = highlight::detect(name, doc.line(0));
        doc
    }

    /// The same snapshot under a new path (a confirmed move): text and offsets are shared, not copied.
    pub fn renamed(&self, source: PathBuf) -> Self {
        let metadata = self.source.renamed(source);
        Self {
            source: Arc::new(metadata),
            version: self.version,
            text: self.text.clone(),
            line_starts: self.line_starts.clone(),
            large: self.large,
            protocol_lines: self.protocol_lines.clone(),
        }
    }

    pub fn source(&self) -> &std::path::Path {
        &self.source.source
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    /// The decoded text, shared: clones are reference counts.
    pub fn text(&self) -> &Arc<str> {
        &self.text
    }

    pub(crate) fn line_starts(&self) -> &Arc<[usize]> { &self.line_starts }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Line `l`'s byte range in [`Document::text`], without its line ending.
    pub fn line_range(&self, l: usize) -> Range<usize> {
        lines::line_range(&self.text, &self.line_starts, l)
    }

    /// Line `l`'s byte range with its line ending (the last line has none).
    pub fn line_with_ending(&self, l: usize) -> Range<usize> {
        self.line_range(l).start..self.line_starts.get(l + 1).copied().unwrap_or(self.text.len())
    }

    /// Line `l`'s text, without its line ending.
    pub fn line(&self, l: usize) -> &str {
        &self.text[self.line_range(l)]
    }

    /// The line holding byte `offset` (clamped to the text).
    pub fn line_of(&self, offset: usize) -> usize {
        self.line_starts.partition_point(|&s| s <= offset.min(self.text.len())) - 1
    }

    /// The starts of language servers' lines (also ended by a CR not followed by LF), sorted; `None` when they are the
    /// reader's lines (no such CR).
    pub fn protocol_line_starts(&self) -> Option<&[usize]> {
        self.protocol_lines.get_or_init(|| {
            let bytes = self.text.as_bytes();
            let mut after_cr = self.text.match_indices('\r').map(|(i, _)| i + 1).filter(|&i| bytes.get(i) != Some(&b'\n')).peekable();
            after_cr.peek()?;
            // Both are sorted, and a lone CR never ends a reader's line: merge.
            let mut starts = Vec::with_capacity(self.line_starts.len());
            let mut lines = self.line_starts.iter().copied().peekable();
            while let (l, c) = (lines.peek().copied(), after_cr.peek().copied())
                && (l.is_some() || c.is_some())
            {
                if c.is_none_or(|c| l.is_some_and(|l| l < c)) {
                    starts.push(lines.next().expect("peeked"));
                } else {
                    starts.push(after_cr.next().expect("peeked"));
                }
            }
            Some(starts.into())
        }).as_deref()
    }

    pub fn is_large(&self) -> bool {
        self.large
    }

    pub fn language(&self) -> Option<Language> {
        self.source.language
    }

    /// The encoding as shown to the user, e.g. "UTF-8", "windows-1252", "UTF-16LE BOM".
    pub fn encoding(&self) -> &str {
        &self.source.encoding
    }

    /// What the reader should say about this snapshot: a notable encoding, lost bytes, large-file mode.
    pub fn notices(&self) -> Vec<String> {
        self.source.notices(self.large)
    }

    /// Memory this snapshot holds, for cache accounting.
    pub fn bytes(&self) -> usize {
        let pixels = self.source.image.as_ref().and_then(|i| i.pixels.as_ref().ok()).map_or(0, |p| p.as_raw().len());
        self.text.len() + self.line_starts.len() * size_of::<usize>() + self.source.source.as_os_str().len() + pixels
    }
}

fn size_label(bytes: usize) -> String {
    match bytes {
        b if b < 1 << 10 => format!("{b} B"),
        b if b < 1 << 20 => format!("{} KiB", b >> 10),
        b => format!("{:.1} MiB", b as f64 / (1 << 20) as f64),
    }
}

/// The identifier (letters, digits and `_`, not starting with a digit) holding the character at byte `at` of
/// `line`.
pub fn identifier(line: &str, at: usize) -> Option<std::ops::Range<usize>> {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    if !line.is_char_boundary(at) || !line[at..].chars().next().is_some_and(word) {
        return None;
    }
    let start = line[..at].char_indices().rev().take_while(|&(_, c)| word(c)).last().map_or(at, |(i, _)| i);
    let end = line[at..].char_indices().find(|&(_, c)| !word(c)).map_or(line.len(), |(i, _)| at + i);
    (!line[start..].starts_with(|c: char| c.is_ascii_digit())).then_some(start..end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(name: &str, bytes: &[u8]) -> Document {
        Document::decode(PathBuf::from(name), 1, bytes)
    }

    #[test]
    fn utf8_crlf_and_unicode_keep_source_coordinates() {
        let d = doc("a.rs", "fn café() {}\r\n// 日本\n".as_bytes());
        assert_eq!(d.line_count(), 3);
        assert_eq!(d.line(0), "fn café() {}");
        assert_eq!(d.line(1), "// 日本");
        assert_eq!(d.line(2), "");
        assert_eq!(&d.text()[d.line_range(1).end..d.line_range(2).start], "\n");
        assert_eq!(&d.text()[d.line_range(0).end..d.line_range(1).start], "\r\n", "line endings are preserved");
        assert_eq!(d.line_of(d.line_range(1).start + 3), 1);
        assert_eq!(&d.text()[d.line_with_ending(0)], "fn café() {}\r\n");
        assert_eq!(&d.text()[d.line_with_ending(2)], "");
        assert_eq!(d.language(), Some(Language::Rust));
        assert!(d.notices().is_empty());
    }

    #[test]
    fn images_decode_bounded_with_their_metadata_and_never_as_text() {
        let png = encode_png(&img::Pixels::from_pixel(3, 2, image::Rgba([10, 20, 30, 255])));
        let d = doc("a.png", &png);
        let i = d.image().unwrap();
        assert_eq!((i.format, i.width, i.height, i.pixels.as_ref().unwrap().dimensions()), (img::Format::Png, 3, 2, (3, 2)));
        assert_eq!(d.text().len(), 0, "a raster image has no text");
        assert_eq!(d.notices(), [format!("PNG image, 3×2 px, {} B", png.len())]);
        assert!(d.bytes() >= 3 * 2 * 4);
        // An SVG keeps its source as text, and its rendering.
        let svg = doc("a.svg", br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="5"/>"#);
        assert_eq!((svg.image().unwrap().width, svg.image().unwrap().height), (4, 5));
        assert!(svg.text().starts_with("<svg"));
        // Corrupt: said, not shown as text.
        let bad = doc("b.png", &png[..20]);
        assert!(bad.image().unwrap().pixels.is_err() && bad.text().is_empty());
        assert!(bad.notices()[0].starts_with("PNG image, 20 B: cannot be shown ("), "{:?}", bad.notices());
        // Text that looks like an image is text: "BM…" (a BMP's magic) and an HTML or JSX file with an inline <svg>.
        let bm = doc("notes.txt", b"BM is a fine name for a file\n");
        assert!(bm.image().is_none() && bm.line(0).starts_with("BM is"));
        let html = doc("page.html", b"<!DOCTYPE html><svg width=\"2\" height=\"2\"></svg>\n");
        assert!(html.image().is_none() && html.text().starts_with("<!DOCTYPE"));
        let jsx = doc("Icon.jsx", b"export const I = () => <svg/>;\n");
        assert!(jsx.image().is_none());
        let bare = doc("icon", br#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" width="3" height="3"/>"#);
        assert!(bare.image().is_none(), "SVG markup without the extension is text");
        // Same text (none), different pixels: a change.
        let other = encode_png(&img::Pixels::from_pixel(3, 2, image::Rgba([0, 0, 0, 255])));
        assert!(!d.same_content(&doc("a.png", &other)) && d.same_content(&doc("a.png", &png)));
    }

    fn encode_png(pixels: &img::Pixels) -> Vec<u8> {
        use image::ImageEncoder;
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out).write_image(pixels.as_raw(), pixels.width(), pixels.height(), image::ExtendedColorType::Rgba8).unwrap();
        out
    }

    #[test]
    fn empty_file_is_one_empty_line() {
        let d = doc("empty.txt", b"");
        assert_eq!((d.line_count(), d.line(0), d.is_large()), (1, "", false));
    }

    #[test]
    fn legacy_and_lossy_encodings_are_noticed() {
        let latin = doc("a.txt", b"caf\xe9 au lait, cr\xe8me br\xfbl\xe9e\n");
        assert_eq!(latin.encoding(), "windows-1252");
        assert_eq!(latin.notices(), ["Encoding: windows-1252"]);
        assert!(latin.line(0).starts_with("café"));
        let bom = doc("a.txt", b"\xEF\xBB\xBFx\n");
        assert_eq!(bom.notices(), ["Encoding: UTF-8 BOM"]);
        let mut odd = vec![0xFF, 0xFE];
        odd.extend("a".encode_utf16().flat_map(u16::to_le_bytes));
        odd.push(b'x');
        assert!(doc("a.txt", &odd).notices().contains(&encoding::LOSSY_NOTICE.to_string()));
    }

    #[test]
    fn large_by_bytes_or_lines_and_never_truncated() {
        let lines = "x\n".repeat(LARGE_LINES + 1);
        let by_lines = doc("a.rs", lines.as_bytes());
        assert!(by_lines.is_large(), "{} lines", by_lines.line_count());
        assert_eq!(by_lines.text().len(), lines.len());
        assert!(!doc("a.rs", "x\n".repeat(LARGE_LINES).as_bytes()).is_large(), "exactly the limit is not large");
        assert!(doc("a.rs", format!("{}x", "x\n".repeat(LARGE_LINES)).as_bytes()).is_large(), "an unterminated extra line counts");
        let wide = "y".repeat(LARGE_BYTES + 1);
        let by_bytes = doc("a.rs", wide.as_bytes());
        assert!(by_bytes.is_large());
        assert_eq!(by_bytes.line(0).len(), LARGE_BYTES + 1, "no silent truncation");
        assert!(by_bytes.notices().iter().any(|n| n.starts_with("Large file")));
    }

    #[test]
    fn control_characters_are_content() {
        let d = doc("a.txt", b"a\x1bb\tc\n");
        assert_eq!(d.line(0), "a\x1bb\tc");
    }
}
