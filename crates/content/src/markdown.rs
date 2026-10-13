//! Single Markdown documents: bounded parsing into blocks with inline styles and links, the map from rendered
//! text back to source bytes, front matter, and the local images a document may show (paths validated against a
//! root). Shared by Lenz (which pairs two documents for its comparisons) and the code reader.

use std::{collections::HashMap, ops::Range, path::Path, sync::Arc};

use crate::highlight::{self, Capture};

mod parse;
pub use parse::parse;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub bytes_per_side: usize,
    pub events_per_side: usize,
    pub segments_per_pair: usize,
    pub rendered_bytes_per_side: usize,
}
impl Limits {
    pub const DEFAULT: Self = Self {
        bytes_per_side: 4 << 20,
        events_per_side: 100_000,
        segments_per_pair: 200_000,
        rendered_bytes_per_side: 8 << 20,
    };
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkdownError {
    Cancelled,
    Limit(&'static str),
    InvalidMap,
    Interrupted,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    Direct(Range<usize>),
    Normalized(Range<usize>),
    Synthetic,
}
#[derive(Clone, Debug)]
pub struct MapSegment {
    pub text: Range<usize>,
    pub origin: Origin,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InlineStyle {
    pub strong: bool,
    pub emphasis: bool,
    pub strike: bool,
    pub code: bool,
    pub link: bool,
    pub highlight: bool,
    pub tag: bool,
    pub wikilink: bool,
    pub external: bool,
    pub footnote: bool,
    pub callout_title: bool,
}
#[derive(Clone, Debug)]
pub struct Run {
    pub text: Range<usize>,
    pub style: InlineStyle,
}
#[derive(Clone, Debug)]
pub struct Link {
    pub text: Range<usize>,
    pub source: Range<usize>,
    pub destination: Arc<str>,
    pub title: Arc<str>,
}
#[derive(Clone, Debug)]
pub enum BlockKind {
    Paragraph,
    /// Front matter; `properties` splits simple front matter into rows.
    Properties,
    /// A footnote definition, labelled with its reference number; its body stays literal.
    Footnote {
        label: String,
    },
    Heading(u8),
    Code {
        language: Option<String>,
    },
    ListItem {
        marker: String,
        checked: Option<bool>,
    },
    TableRow {
        header: bool,
        cells: Vec<Range<usize>>,
        alignment: Vec<TableAlignment>,
    },
    Rule,
    Literal,
    Image {
        alt: String,
        destination: String,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableAlignment {
    None,
    Left,
    Center,
    Right,
}
#[derive(Clone, Debug)]
pub struct Block {
    pub kind: BlockKind,
    pub quote_depth: u16,
    pub list_depth: u16,
    pub source: Range<usize>,
    pub text: Range<usize>,
    pub runs: Vec<Run>,
    /// The innermost Obsidian callout (`> [!type]`) this block is inside.
    pub callout: Option<Callout>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Callout {
    /// Distinct per callout in one document.
    pub id: u32,
    pub kind: CalloutKind,
    /// `Some(true)` for `[!type]-` (folded), `Some(false)` for `[!type]+`, `None` when not foldable.
    pub folded: Option<bool>,
}
impl Callout {
    /// Whether the callout's body is hidden: `-` starts folded unless it holds changes (no change is hidden), and
    /// each toggle (a click on its title) inverts that. Both frontends.
    pub fn shut(&self, changed: bool, toggled: bool) -> bool {
        self.folded.is_some() && ((self.folded == Some(true) && !changed) != toggled)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalloutKind {
    Note,
    Abstract,
    Info,
    Todo,
    Tip,
    Success,
    Question,
    Warning,
    Failure,
    Danger,
    Bug,
    Example,
    Quote,
}
impl CalloutKind {
    /// Obsidian's type names and aliases, any case; unknown types display as notes.
    pub fn parse(name: &str) -> CalloutKind {
        match name.to_ascii_lowercase().as_str() {
            "abstract" | "summary" | "tldr" => Self::Abstract,
            "info" => Self::Info,
            "todo" => Self::Todo,
            "tip" | "hint" | "important" => Self::Tip,
            "success" | "check" | "done" => Self::Success,
            "question" | "help" | "faq" => Self::Question,
            "warning" | "caution" | "attention" => Self::Warning,
            "failure" | "fail" | "missing" => Self::Failure,
            "danger" | "error" => Self::Danger,
            "bug" => Self::Bug,
            "example" => Self::Example,
            "quote" | "cite" => Self::Quote,
            _ => Self::Note,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Note => "Note",
            Self::Abstract => "Abstract",
            Self::Info => "Info",
            Self::Todo => "Todo",
            Self::Tip => "Tip",
            Self::Success => "Success",
            Self::Question => "Question",
            Self::Warning => "Warning",
            Self::Failure => "Failure",
            Self::Danger => "Danger",
            Self::Bug => "Bug",
            Self::Example => "Example",
            Self::Quote => "Quote",
        }
    }
}
#[derive(Debug)]
pub struct MarkdownDocument {
    pub source: Arc<str>,
    pub text: String,
    pub blocks: Vec<Block>,
    pub segments: Vec<MapSegment>,
    pub line_starts: Vec<usize>,
    pub links: Vec<Link>,
}

impl MarkdownDocument {
    /// The source line and byte column of decoded byte `byte` (CRLF excluded from the line's length).
    pub fn line_column(&self, byte: usize) -> Option<(u32, usize)> {
        if !self.source.is_char_boundary(byte) {
            return None;
        }
        let line = self.line_starts.partition_point(|&start| start <= byte).saturating_sub(1);
        let start = *self.line_starts.get(line)?;
        let end = self.line_starts.get(line + 1).copied().unwrap_or(self.source.len());
        let len = self.source[start..end].trim_end_matches(['\r', '\n']).len();
        Some((line as u32, (byte - start).min(len)))
    }
}

/// Front matter rows: `key: value`, `key: [a, b]`, or `key:` followed by `- item` lines. `None` for anything else
/// (nested maps, block strings), which the adapters show as literal text.
pub fn properties(text: &str) -> Option<Vec<(String, Vec<String>)>> {
    let unquote = |v: &str| v.trim().trim_matches(['"', '\'']).to_string();
    let mut rows: Vec<(String, Vec<String>)> = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#')) {
        // `- item` belongs to the key above it, indented or not
        if let Some(item) = line.trim_start().strip_prefix("- ") {
            rows.last_mut()?.1.push(unquote(item));
            continue;
        }
        if line.starts_with(char::is_whitespace) {
            return None;
        }
        let (key, value) = line.split_once(':')?;
        let value = value.trim();
        if matches!(value, "|" | ">" | "|-" | ">-") {
            return None;
        }
        let values = if value.is_empty() {
            Vec::new()
        } else if let Some(list) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
            list.split(',').map(unquote).filter(|v| !v.is_empty()).collect()
        } else {
            vec![unquote(value)]
        };
        rows.push((key.trim().to_string(), values));
    }
    Some(rows)
}

/// The largest image file the preview decodes, and how many images one side shows at most.
pub const IMAGE_BYTES: usize = 64 << 20;
pub const MAX_IMAGES: usize = 64;

/// A local image file's RGBA pixels; `None` when it is larger than [`IMAGE_BYTES`] or not a readable image.
pub fn decode_image(path: &std::path::Path) -> Option<crate::image::Pixels> {
    if std::fs::metadata(path).ok()?.len() > IMAGE_BYTES as u64 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let format = crate::image::detect(&path.to_string_lossy(), &bytes)?;
    crate::image::decode(&bytes, format).ok().map(|d| d.pixels)
}

/// An image destination the preview may load: relative to the Markdown file, a supported image type, and inside
/// `root` once symlinks are resolved. URLs, absolute paths and anything leaving `root` are refused.
pub fn local_image(
    root: &std::path::Path,
    markdown_file: &std::path::Path,
    destination: &str,
) -> Option<std::path::PathBuf> {
    let destination = destination.split(['#', '?']).next()?.replace("%20", " ");
    let relative = std::path::Path::new(&destination);
    if destination.is_empty() || destination.contains(':') || relative.has_root() {
        return None;
    }
    let ext = relative.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp") {
        return None;
    }
    let path = markdown_file.parent()?.join(relative).canonicalize().ok()?;
    (path.starts_with(root.canonicalize().ok()?) && path.is_file()).then_some(path)
}

/// Syntax spans of each code block with a known language, as byte ranges of the document's rendered text, computed
/// while `current` holds: call it off the UI thread.
pub fn code_syntax(doc: &MarkdownDocument, current: impl Fn() -> bool + Send + Clone + 'static) -> HashMap<usize, Vec<(Range<usize>, Capture)>> {
    let mut blocks = HashMap::new();
    for (index, block) in doc.blocks.iter().enumerate() {
        if !current() {
            break;
        }
        let BlockKind::Code { language: Some(language) } = &block.kind else { continue };
        let extension = match language.as_str() {
            "rust" => "rs",
            "python" => "py",
            "javascript" => "js",
            "typescript" => "ts",
            "bash" => "sh",
            other => other,
        };
        let Some(language) = highlight::detect(Path::new(&format!("code.{extension}")), "") else { continue };
        let text = &doc.text[block.text.clone()];
        let Some(parsed) = highlight::parse_while(language, Arc::from(text), current.clone()) else { continue };
        let starts: Vec<_> = std::iter::once(block.text.start).chain(text.match_indices('\n').map(|(b, _)| block.text.start + b + 1)).collect();
        let mut spans = Vec::new();
        for (line, items) in parsed.spans(0..starts.len()).into_iter().enumerate() {
            for (r, capture) in items {
                spans.push((starts[line] + r.start..starts[line] + r.end, capture));
            }
        }
        blocks.insert(index, spans);
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_document_parses_within_its_budget_and_maps_back_to_source() {
        let source: Arc<str> = "# Títle\r\n\r\nSome *text* and `code`.\r\n\r\n```rust\r\nfn x() {}\r\n```\r\n\r\n![logo](img/a.png)\r\n".into();
        let mut remaining = Limits::DEFAULT.segments_per_pair;
        let doc = parse(source.clone(), Limits::DEFAULT, &mut remaining, &|| true).unwrap();
        assert!(remaining < Limits::DEFAULT.segments_per_pair, "the budget is spent");
        let kinds: Vec<_> = doc
            .blocks
            .iter()
            .map(|b| match &b.kind {
                BlockKind::Heading(n) => format!("h{n}"),
                BlockKind::Paragraph => "p".into(),
                BlockKind::Code { language } => format!("code {language:?}"),
                BlockKind::Image { destination, .. } => format!("image {destination}"),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(kinds, ["h1", "p", "code Some(\"rust\")", "image img/a.png"]);
        // A block's source range, as a line and column of the CRLF source.
        let paragraph = &doc.blocks[1];
        assert_eq!(doc.line_column(paragraph.source.start), Some((2, 0)));
        assert_eq!(&source[paragraph.source.clone()], "Some *text* and `code`.\r\n", "with its line ending");
        // The code block's `fn` keyword, highlighted.
        let syntax = code_syntax(&doc, || true);
        let (range, _) = &syntax[&2][0];
        assert_eq!(&doc.text[range.clone()], "fn");
        assert!(code_syntax(&doc, || false).is_empty(), "cancelled");
        assert_eq!(parse(source, Limits::DEFAULT, &mut 1, &|| true).err(), Some(MarkdownError::Limit("mapped text segments")));
        assert_eq!(parse("x".into(), Limits::DEFAULT, &mut 100, &|| false).err(), Some(MarkdownError::Cancelled));
        let small = Limits { bytes_per_side: 4, ..Limits::DEFAULT };
        assert_eq!(parse("12345".into(), small, &mut 100, &|| true).err(), Some(MarkdownError::Limit("source bytes")));
    }

    #[test]
    fn local_images_stay_inside_their_root() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("repo");
        std::fs::create_dir_all(root.join("docs/img")).unwrap();
        std::fs::write(root.join("docs/img/a.png"), b"x").unwrap();
        std::fs::write(d.path().join("outside.png"), b"x").unwrap();
        let md = root.join("docs/readme.md");
        assert_eq!(local_image(&root, &md, "img/a.png"), Some(root.join("docs/img/a.png").canonicalize().unwrap()));
        assert_eq!(local_image(&root, &md, "img/a%20b.png"), None, "missing");
        assert_eq!(local_image(&root, &md, "../../outside.png"), None, "outside the root");
        assert_eq!(local_image(&root, &md, "/etc/passwd.png"), None, "absolute");
        assert_eq!(local_image(&root, &md, "https://x/a.png"), None, "a URL");
        assert_eq!(local_image(&root, &md, "img/a.svg"), None, "not a raster type");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(d.path().join("outside.png"), root.join("docs/img/link.png")).unwrap();
            assert_eq!(local_image(&root, &md, "img/link.png"), None, "a symlink leaving the root");
        }
    }
}
