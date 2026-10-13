//! Single-document decoding of raw bytes to text.
//!
//! Pipeline: a BOM (UTF-8, UTF-16LE/BE) wins, then a declared label (git's
//! `working-tree-encoding`, supplied by the caller), then valid UTF-8, then a
//! `chardetng` guess on the first 64 KB. Undecodable bytes become U+FFFD.
//! UTF-16 with a BOM is decoded as text, where git would call it binary.
//!
//! Crates: `encoding_rs` ((Apache-2.0 OR MIT) AND BSD-3-Clause for the WHATWG
//! index data) and `chardetng` (Apache-2.0 OR MIT).

use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};
use encoding_rs::{Encoding, UTF_8};

/// One document's text and how it was decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    /// The text, BOM stripped.
    pub text: String,
    /// WHATWG encoding name, e.g. "UTF-8", "UTF-16LE", "windows-1252", "Shift_JIS".
    pub encoding: &'static str,
    pub had_bom: bool,
    /// Some bytes were undecodable and became U+FFFD.
    pub lossy: bool,
}

/// Decodes one document. `declared` is a git `working-tree-encoding` label; unknown labels are ignored.
pub fn decode(bytes: &[u8], declared: Option<&str>) -> Decoded {
    let bom = Encoding::for_bom(bytes);
    let declared = declared.and_then(|label| Encoding::for_label_no_replacement(label.as_bytes()));
    let encoding = match bom.map(|(encoding, _)| encoding).or(declared) {
        Some(encoding) => encoding,
        // fast path: one validation pass, one copy
        None => match UTF_8.decode_without_bom_handling_and_without_replacement(bytes) {
            Some(text) => return Decoded { text: text.into_owned(), encoding: "UTF-8", had_bom: false, lossy: false },
            None => guess(bytes),
        },
    };
    // `decode` honours and strips a BOM, so a BOM beats the declared label
    let (text, encoding, lossy) = encoding.decode(bytes);
    Decoded { text: text.into_owned(), encoding: encoding.name(), had_bom: bom.is_some(), lossy }
}

/// Guesses a legacy encoding from the first 64 KB (the whole file costs seconds).
fn guess(bytes: &[u8]) -> &'static Encoding {
    let head = &bytes[..bytes.len().min(64 << 10)];
    let mut detector = EncodingDetector::new(Iso2022JpDetection::Deny);
    detector.feed(head, head.len() == bytes.len());
    // UTF-8 was already ruled out
    detector.guess(None, Utf8Detection::Deny)
}

impl Decoded {
    /// The encoding as shown to the user: "windows-1252", "UTF-8 BOM".
    pub fn label(&self) -> String {
        if self.had_bom { format!("{} BOM", self.encoding) } else { self.encoding.to_string() }
    }

    /// Whether a header names this encoding: anything but UTF-8 without a BOM.
    pub fn shown_in_header(&self) -> bool {
        self.encoding != "UTF-8" || self.had_bom
    }
}

/// The notice for a [`Decoded::lossy`] document.
/// git's `buffer_is_binary`: a NUL in the first 8000 bytes. UTF-16 with a BOM is the exception: git calls it binary,
/// Lenz decodes it as text (a UTF-32 BOM stays binary).
pub fn looks_binary(bytes: &[u8]) -> bool {
    let utf16 = bytes.starts_with(&[0xFE, 0xFF]) || bytes.starts_with(&[0xFF, 0xFE]) && !bytes.starts_with(&[0xFF, 0xFE, 0, 0]);
    !utf16 && bytes[..bytes.len().min(8000)].contains(&0)
}

pub const LOSSY_NOTICE: &str = "undecodable bytes shown as �";

/// The file-level notice when two documents' encodings differ (a comparison's Sides).
pub fn describe(a: &Decoded, b: &Decoded) -> Option<String> {
    let (a, b) = (a.label(), b.label());
    (a != b).then(|| format!("encodings differ: {a} vs {b}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRENCH: &str = "Le cœur déçu mais l'âme plutôt naïve, Louÿs rêva de crapaüter en canoë au delà des îles, \
        près du mälström où brûlent les novæ. Où est la crème brûlée? À côté du château, élève!\n";
    const JAPANESE: &str = "吾輩は猫である。名前はまだ無い。どこで生れたかとんと見当がつかぬ。\
        何でも薄暗いじめじめした所でニャーニャー泣いていた事だけは記憶している。\
        吾輩はここで始めて人間というものを見た。\n";

    fn utf16(s: &str, le: bool) -> Vec<u8> {
        let mut out = if le { vec![0xFF, 0xFE] } else { vec![0xFE, 0xFF] };
        out.extend(s.encode_utf16().flat_map(|u| if le { u.to_le_bytes() } else { u.to_be_bytes() }));
        out
    }

    #[test]
    fn utf8_without_bom() {
        let d = decode("héllo\n".as_bytes(), None);
        assert_eq!(d, Decoded { text: "héllo\n".into(), encoding: "UTF-8", had_bom: false, lossy: false });
        assert!(!d.shown_in_header());
    }

    #[test]
    fn empty_is_utf8() {
        assert_eq!(decode(b"", None).encoding, "UTF-8");
    }

    #[test]
    fn utf8_bom_is_stripped() {
        let d = decode(b"\xEF\xBB\xBFabc", None);
        assert_eq!(d, Decoded { text: "abc".into(), encoding: "UTF-8", had_bom: true, lossy: false });
        assert_eq!(d.label(), "UTF-8 BOM");
        assert!(d.shown_in_header());
    }

    #[test]
    fn utf16_with_bom_decodes_as_text() {
        let le = decode(&utf16("añb\n", true), None);
        assert_eq!(le, Decoded { text: "añb\n".into(), encoding: "UTF-16LE", had_bom: true, lossy: false });
        let be = decode(&utf16("añb\n", false), None);
        assert_eq!(be, Decoded { text: "añb\n".into(), encoding: "UTF-16BE", had_bom: true, lossy: false });
    }

    #[test]
    fn windows_1252_is_detected() {
        let (bytes, _, _) = encoding_rs::WINDOWS_1252.encode(FRENCH);
        let d = decode(&bytes, None);
        assert_eq!((d.text.as_str(), d.encoding, d.lossy), (FRENCH, "windows-1252", false));
        assert!(d.shown_in_header());
    }

    #[test]
    fn shift_jis_is_detected() {
        let (bytes, _, _) = encoding_rs::SHIFT_JIS.encode(JAPANESE);
        let d = decode(&bytes, None);
        assert_eq!((d.text.as_str(), d.encoding, d.lossy), (JAPANESE, "Shift_JIS", false));
    }

    #[test]
    fn invalid_utf8_without_declaration_is_guessed_not_lossy() {
        // every byte decodes in the single-byte encoding chardetng picks
        let d = decode(b"ab\xFFcd\n", None);
        assert_ne!(d.encoding, "UTF-8");
        assert!(!d.lossy);
    }

    #[test]
    fn undecodable_bytes_become_replacement_chars() {
        let d = decode(b"ab\xFFcd", Some("utf-8"));
        assert_eq!((d.text.as_str(), d.encoding, d.lossy), ("ab\u{FFFD}cd", "UTF-8", true));
        let bom = decode(b"\xEF\xBB\xBFa\xFF", None);
        assert_eq!((bom.text.as_str(), bom.lossy), ("a\u{FFFD}", true));
        let mut odd = utf16("a", true);
        odd.push(b'x');
        let odd = decode(&odd, None);
        assert_eq!((odd.text.as_str(), odd.lossy), ("a\u{FFFD}", true));
    }

    #[test]
    fn declared_label_beats_detection() {
        let d = decode("café".as_bytes(), Some("latin1"));
        assert_eq!((d.text.as_str(), d.encoding), ("cafÃ©", "windows-1252"));
    }

    #[test]
    fn bom_beats_declared_label() {
        // git's `working-tree-encoding=UTF-16` is UTF-16LE in WHATWG; a big-endian BOM still wins
        let d = decode(&utf16("ab", false), Some("UTF-16"));
        assert_eq!((d.text.as_str(), d.encoding, d.had_bom), ("ab", "UTF-16BE", true));
    }

    #[test]
    fn unknown_declared_label_falls_back_to_detection() {
        assert_eq!(decode(&utf16("ab", true), Some("UTF-16LE-BOM")).encoding, "UTF-16LE");
        assert_eq!(decode(b"plain", Some("no-such-charset")).encoding, "UTF-8");
    }

    #[test]
    fn describe_names_differing_encodings() {
        let utf8 = decode(b"x", None);
        let bom = decode(b"\xEF\xBB\xBFx", None);
        let latin = decode(b"x", Some("windows-1252"));
        assert_eq!(describe(&latin, &utf8).as_deref(), Some("encodings differ: windows-1252 vs UTF-8"));
        assert_eq!(describe(&utf8, &bom).as_deref(), Some("encodings differ: UTF-8 vs UTF-8 BOM"));
        assert_eq!(describe(&utf8, &decode(b"y", None)), None);
    }

    #[test]
    fn timings() {
        let line = "the quick brown fox jumps over the lazy dog 0123456789\n";
        let ascii = line.repeat((50usize << 20).div_ceil(line.len()));
        let started = std::time::Instant::now();
        let d = decode(ascii.as_bytes(), None);
        let utf8 = started.elapsed();
        assert_eq!(d.text.len(), ascii.len());

        let french = FRENCH.repeat((64 << 10) / FRENCH.chars().count() + 1);
        let (latin, _, _) = encoding_rs::WINDOWS_1252.encode(&french);
        let latin = &latin[..64 << 10];
        let started = std::time::Instant::now();
        assert_eq!(decode(latin, None).encoding, "windows-1252");
        let cp1252 = started.elapsed();
        crate::perf::record("decode/ascii-50mib", "ms", &[utf8.as_secs_f64() * 1000.0]);
        crate::perf::record("decode/windows1252-64kib", "ms", &[cp1252.as_secs_f64() * 1000.0]);
        eprintln!("decode: {} MB ASCII {utf8:?}, 64 KB windows-1252 {cp1252:?}", ascii.len() >> 20);
        // loose ceiling: debug builds are slow; real numbers come from `--release`
        crate::perf::check(utf8.as_secs() < 5 && cp1252.as_secs() < 5, "decoding exceeds the five-second ceiling");
    }
}
