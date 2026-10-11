//! Decoded text and comparison preparation shared by GUI and terminal adapters.
use std::{ops::Range, path::Path, sync::Arc};
use lenz_engine::{Budget, LineEnding, Options, Refinement};
use crate::{changeset::{ChangeSet, Content as Source, SideBytes}, encoding, highlight::{self, Language}, moves,
    view::{Side, View}, write::Snapshot};

pub struct TextDocument {
    pub(crate) text_left: Arc<str>,
    pub(crate) text_right: Arc<str>,
    pub(crate) lang_left: Option<Language>,
    pub(crate) lang_right: Option<Language>,
    pub(crate) raw_left: Vec<String>,
    pub(crate) raw_right: Vec<String>,
    /// Shown as a file-level notice when the Sides' line endings differ.
    pub(crate) line_endings: Option<(LineEnding, LineEnding)>,
    /// Each Side's encoding, when the header should name it (anything but plain UTF-8).
    pub(crate) encodings: [Option<String>; 2],
    /// File-level notices from loading: differing encodings, undecodable bytes.
    pub(crate) notes: Vec<String>,
    /// Binary or oversized Sides: shown instead of the text.
    pub(crate) placeholder: Option<Placeholder>,
    /// Longest line, in characters (as painted, so capped), for horizontal scrolling.
    pub(crate) widest: usize,
    /// Each Side as read for writing back (a move onto it), or why it is read-only.
    pub(crate) writable: [Result<Snapshot, &'static str>; 2],
}

#[derive(Debug, Clone, PartialEq)]
pub enum Placeholder {
    Binary { sizes: [u64; 2], differ: bool },
    /// Over `LARGE` bytes on a Side; diffed only on request.
    Large { bytes: u64 },
    Unreadable(String),
}

/// A comparison read once, with the same image detection and notices for every adapter.
/// Display resources are built only by the adapter after this worker result arrives.
pub struct PreparedDocument {
    pub text: TextDocument,
    pub image: Option<Result<crate::image_session::ImageDoc, String>>,
}
impl PreparedDocument {
    pub fn load(set: &ChangeSet, index: usize, left: &Path, right: &Path, force: bool) -> Result<Self, String> {
        let sides = match read_sides(set, index, force)? {
            ReadSides::Large(bytes) => return Ok(Self {
                text: TextDocument::placeholder(left, right, Placeholder::Large { bytes }),
                image: None,
            }),
            ReadSides::Bytes(sides) => sides,
        };
        let [l, r] = &sides;
        // A display name can supply the extension for difftool temporary files,
        // but must not hide the real SVG extension when it is merely a label.
        let comparison = &set.comparisons[index];
        fn image_name<'a>(actual: &'a str, label: &'a Path) -> &'a Path {
            if Path::new(actual).extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("svg")) {
                Path::new(actual)
            } else { label }
        }
        let (image_left, image_right) = (image_name(&comparison.old_path, left), image_name(&comparison.path, right));
        match crate::image_session::ImageDoc::read((&l.bytes, image_left, l.binary), (&r.bytes, image_right, r.binary)) {
            Some(Ok(image)) => {
                let mut text = TextDocument::new("", "", left, right);
                text.encodings = [image.info(Side::Left), image.info(Side::Right)];
                text.notes = image.notes();
                Ok(Self { text, image: Some(Ok(image)) })
            }
            Some(Err(error)) => {
                let sizes = [l.bytes.len() as u64, r.bytes.len() as u64];
                let mut text = TextDocument::placeholder(left, right, Placeholder::Binary { sizes, differ: l.bytes != r.bytes });
                text.notes.push(format!("cannot show as an image: {error}"));
                Ok(Self { text, image: Some(Err(error)) })
            }
            None => Ok(Self { text: TextDocument::from_sides(set, index, left, right, sides), image: None }),
        }
    }
}

/// Sides bigger than this ask before they're read and diffed.
pub const LARGE: u64 = 20 << 20;
/// Lines longer than this are cut short on screen.
pub const LONG_LINE: usize = 10_000;

/// A character that would hide or reorder what a reviewer sees: a C0/C1 control, or one of the Unicode
/// bidirectional embedding, override and isolate controls of "Trojan Source" (CVE-2021-42574), which make
/// a terminal or text shaper display source in an order the compiler never sees. Both frontends show U+FFFD
/// instead. Joiners, variation selectors and marks are not included: they are part of emoji and scripts.
pub fn hides_text(c: char) -> bool {
    c.is_control() || is_bidi_control(c)
}

fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Zero-width characters (combining marks, joiners) drawn on one character: Unicode calls a run longer than 30
/// abnormal (UAX #15, stream-safe text). Thousands stacked on one letter cost the GUI's shaper ~7 ms a line,
/// 381 ms a screenful, and draw over neighbouring rows; past this many, the rest are not drawn. Copy is unchanged.
pub const MAX_MARKS: usize = 30;

/// Whether `c` is drawn, given `run`: the zero-width characters drawn since the last visible one (updated).
pub fn shows_mark(c: char, run: &mut usize) -> bool {
    if unicode_width::UnicodeWidthChar::width(c) != Some(0) {
        *run = 0;
        return true;
    }
    *run += 1;
    *run <= MAX_MARKS
}

/// `text` without the zero-width characters past [`MAX_MARKS`] on one character.
pub fn cap_marks(text: &str) -> std::borrow::Cow<'_, str> {
    let mut run = 0;
    if text.chars().all(|c| shows_mark(c, &mut run)) {
        return text.into();
    }
    let mut run = 0;
    text.chars().filter(|&c| shows_mark(c, &mut run)).collect::<String>().into()
}

/// `text` with its bidi controls shown as U+FFFD. Both are three UTF-8 bytes, so every byte offset into `text`
/// (styles, selections, glyph maps) stays valid: for text shaped as it is, where the offsets are already fixed.
pub fn show_bidi(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains(is_bidi_control) {
        return text.into();
    }
    text.chars().map(|c| if is_bidi_control(c) { '\u{FFFD}' } else { c }).collect::<String>().into()
}
pub(crate) enum ReadSides { Large(u64), Bytes([SideBytes; 2]) }

pub(crate) fn read_sides(set: &ChangeSet, index: usize, force: bool) -> Result<ReadSides, String> {
    let sizes = set.sizes(index)?;
    if !force && sizes.iter().any(|&s| s > LARGE) {
        return Ok(ReadSides::Large(sizes[0].max(sizes[1])));
    }
    set.sides(index).map(ReadSides::Bytes)
}


impl TextDocument {
    pub fn new(left: &str, right: &str, left_path: &Path, right_path: &Path) -> Self {
        let raw = |t: &str| lenz_engine::split_lines(t).into_iter().map(String::from).collect::<Vec<_>>();
        let (raw_left, raw_right) = (raw(left), raw(right));
        let (el, er) = (lenz_engine::line_ending(left), lenz_engine::line_ending(right));
        let differ = el != er && el != LineEnding::None && er != LineEnding::None;
        let lang = |path: &Path, lines: &[String]| highlight::detect(path, lines.first().map_or("", String::as_str));
        TextDocument {
            text_left: left.into(),
            text_right: right.into(),
            lang_left: lang(left_path, &raw_left),
            lang_right: lang(right_path, &raw_right),
            widest: raw_left.iter().chain(&raw_right).map(|l| l.len().min(LONG_LINE + 40)).max().unwrap_or(0),
            raw_left,
            raw_right,
            line_endings: differ.then_some((el, er)),
            encodings: [None, None],
            notes: Vec::new(),
            placeholder: None,
            writable: [Err("not a file"), Err("not a file")],
        }
    }

    /// Whether this side has a writable file snapshot; command checks still govern each edit.
    pub fn is_writable(&self, side: Side) -> bool { self.writable[side as usize].is_ok() }

    /// The same Comparison with `side`'s text replaced by what was just written through `snapshot`.
    pub(crate) fn edited(&self, side: Side, text: &str, snapshot: Snapshot, left_path: &Path, right_path: &Path) -> TextDocument {
        let normalized = snapshot.crlf.then(|| crate::write::with_crlf(text));
        let text = normalized.as_deref().unwrap_or(text);
        let (l, r) = if side == Side::Left { (text, &*self.text_right) } else { (&*self.text_left, text) };
        let mut doc = TextDocument::new(l, r, left_path, right_path);
        doc.encodings = self.encodings.clone();
        doc.notes = self.notes.clone();
        doc.writable = self.writable.clone();
        doc.writable[side as usize] = Ok(snapshot);
        doc
    }

    /// Read and decode a comparison; oversized inputs require an explicit request.
    pub fn load(set: &ChangeSet, index: usize, left: &Path, right: &Path, force: bool) -> Result<Self, String> {
        match read_sides(set, index, force)? {
            ReadSides::Large(bytes) => Ok(Self::placeholder(left, right, Placeholder::Large { bytes })),
            ReadSides::Bytes(sides) => Ok(Self::from_sides(set, index, left, right, sides)),
        }
    }

    pub(crate) fn placeholder(left: &Path, right: &Path, placeholder: Placeholder) -> Self {
        Self { placeholder: Some(placeholder), ..Self::new("", "", left, right) }
    }

    pub(crate) fn from_sides(set: &ChangeSet, index: usize, left_path: &Path, right_path: &Path, [l, r]: [SideBytes; 2]) -> Self {
        let empty = |p| Self::placeholder(left_path, right_path, p);
        if l.binary || r.binary {
            let sizes = [l.bytes.len() as u64, r.bytes.len() as u64];
            return empty(Placeholder::Binary { sizes, differ: l.bytes != r.bytes });
        }
        // a Side that may take a move is read through the writer, so its text is exactly what the hash check covers
        let c = &set.comparisons[index];
        let temp = std::env::temp_dir();
        let read = |side: &SideBytes, content: &Source| -> (encoding::Decoded, Result<Snapshot, &'static str>) {
            let snapshot = match (moves::editable(content, &temp), content) {
                (Err(why), _) => Err(why),
                _ if side.textconv => Err("shown through a textconv driver"),
                (Ok(()), Source::File(path)) => match Snapshot::read(path, side.encoding.as_deref()) {
                    Ok((snapshot, decoded)) if !decoded.lossy => return (decoded, Ok(snapshot)),
                    Ok(_) => Err("not decodable without loss"),
                    Err(_) => Err("unreadable"),
                },
                _ => Err("not a file"),
            };
            (encoding::decode(&side.bytes, side.encoding.as_deref()), snapshot)
        };
        let ((dl, wl), (dr, wr)) = (read(&l, &c.left), read(&r, &c.right));
        let mut doc = TextDocument::new(&dl.text, &dr.text, left_path, right_path);
        doc.writable = [wl, wr];
        let shown = |d: &encoding::Decoded| d.shown_in_header().then(|| d.label());
        doc.encodings = [shown(&dl), shown(&dr)];
        doc.notes.extend(encoding::describe(&dl, &dr));
        doc.notes.extend(set.skipped_driver(index).map(str::to_owned));
        if dl.lossy || dr.lossy {
            doc.notes.push("undecodable bytes shown as �".into());
        }
        if l.textconv || r.textconv {
            doc.notes.push("shown through a textconv driver".into());
        }
        doc
    }

    fn lines(&self) -> (Vec<&str>, Vec<&str>) {
        (self.raw_left.iter().map(String::as_str).collect(), self.raw_right.iter().map(String::as_str).collect())
    }

    /// Both Sides' text of a block's line ranges — what identifies a Folded Change Block across re-diffs.
    pub(crate) fn block_text(&self, (l, r): &(Range<u32>, Range<u32>)) -> (String, String) {
        (self.raw_left[l.start as usize..l.end as usize].join("\n"), self.raw_right[r.start as usize..r.end as usize].join("\n"))
    }

    /// Fold the blocks of `view` whose text matches one of `texts`.
    pub(crate) fn refold(&self, view: &mut View, texts: &[(String, String)]) {
        if texts.is_empty() {
            return;
        }
        let blocks: Vec<usize> = view.block_ranges().iter().enumerate().filter(|(_, b)| texts.contains(&self.block_text(b))).map(|(i, _)| i).collect();
        view.fold_blocks(&blocks);
    }

    /// Change Blocks without Inline Changes; those come lazily, per block, from [`TextDocument::refine`].
    pub fn line_diff(&self, options: &Options) -> Vec<lenz_engine::ChangeBlock> {
        if self.placeholder.is_some() {
            return Vec::new();
        }
        let (l, r) = self.lines();
        lenz_engine::compare_lines(&l, &r, options)
    }

    pub fn view(&self, blocks: &[lenz_engine::ChangeBlock], viewport_rows: f32) -> View {
        let mut view = View::new(self.raw_left.len() as u32, self.raw_right.len() as u32, blocks, viewport_rows);
        let lens = |lines: &[String]| lines.iter().map(String::len).collect::<Vec<_>>();
        view.attach_line_lengths(&lens(&self.raw_left), &lens(&self.raw_right));
        view
    }

    pub fn refine(&self, blocks: Vec<(usize, lenz_engine::ChangeBlock)>, options: &Options, budget: &mut Budget) -> Vec<(usize, Refinement)> {
        let (l, r) = self.lines();
        blocks.into_iter().map(|(i, b)| (i, lenz_engine::refine(&l, &r, &b, options, budget))).collect()
    }

    /// Refine the blocks the view shows, in place (the first paint of a freshly diffed view).
    pub fn refine_visible(&self, view: &mut View, options: &Options, budget: &mut Budget) {
        let todo = view.blocks_to_refine();
        view.apply_refinements(self.refine(todo, options, budget));
    }
    pub fn text_arc(&self, side: Side) -> Arc<str> { if side == Side::Left { self.text_left.clone() } else { self.text_right.clone() } }
    pub fn text(&self, side: Side) -> &str { if side == Side::Left { &self.text_left } else { &self.text_right } }
    pub fn lines_on(&self, side: Side) -> &[String] {
        match side { Side::Left => &self.raw_left, Side::Right => &self.raw_right }
    }

    pub fn placeholder_kind(&self) -> Option<&Placeholder> { self.placeholder.as_ref() }
    pub fn language(&self, side: Side) -> Option<Language> { if side == Side::Left { self.lang_left } else { self.lang_right } }
    pub fn line_endings(&self) -> Option<(LineEnding, LineEnding)> { self.line_endings }
    pub fn widest(&self) -> usize { self.widest }
    pub fn notes(&self) -> &[String] { &self.notes }
    pub fn encoding(&self, side: Side) -> Option<&str> { self.encodings[side as usize].as_deref() }
}

#[cfg(test)]
mod prepared_tests {
    use super::*;
    #[test]
    fn hidden_controls_include_bidi_overrides_but_not_joiners_or_marks() {
        for c in ['\x1b', '\x7f', '\u{85}', '\u{202A}', '\u{202E}', '\u{2066}', '\u{2069}', '\t'] {
            assert!(hides_text(c), "{c:?}");
        }
        for c in ['a', 'é', '\u{200D}', '\u{FE0F}', '\u{300}', '😀', ' ', '\u{200E}'] {
            assert!(!hides_text(c), "{c:?}");
        }
        // same byte length, so offsets into the original stay valid
        let shown = show_bidi("a\u{202E}b\u{2066}c\td");
        assert_eq!(shown, "a\u{FFFD}b\u{FFFD}c\td");
        assert_eq!(shown.len(), "a\u{202E}b\u{2066}c\td".len());
        assert!(matches!(show_bidi("plain"), std::borrow::Cow::Borrowed(_)));
        // combining marks: 30 on one character are drawn, the rest are not; the next character starts afresh
        let zalgo = format!("a{}b\u{301}", "\u{300}".repeat(100));
        assert_eq!(cap_marks(&zalgo), format!("a{}b\u{301}", "\u{300}".repeat(MAX_MARKS)));
        assert!(matches!(cap_marks("e\u{301}👩\u{200D}💻"), std::borrow::Cow::Borrowed(_)));
    }
    #[test]
    fn named_svg_and_difftool_temporary_files_keep_image_detection() {
        let dir = tempfile::tempdir().unwrap();
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#f00"/></svg>"##;
        for (suffix, label) in [(".SVG", "Review λ"), ("", "original.svg")] {
            let left = dir.path().join(format!("left{suffix}"));
            let right = dir.path().join(format!("right{suffix}"));
            std::fs::write(&left, svg).unwrap();
            std::fs::write(&right, std::str::from_utf8(svg).unwrap().replace("#f00", "#00f")).unwrap();
            let spec = crate::changeset::Spec::Plain { left, right };
            let set = ChangeSet::load(&spec, dir.path()).unwrap();
            let prepared = PreparedDocument::load(&set, 0, Path::new(label), Path::new(label), false).unwrap();
            let image = prepared.image.unwrap().unwrap();
            assert_eq!(image.size, (8, 8));
            assert_eq!(image.sides[0].as_ref().unwrap().pixels.get_pixel(0, 0).0, [255, 0, 0, 255]);
            assert_eq!(image.sides[1].as_ref().unwrap().pixels.get_pixel(0, 0).0, [0, 0, 255, 255]);
            assert!(prepared.text.placeholder_kind().is_none());
        }
    }
}
