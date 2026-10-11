//! Shared, bounded Markdown parsing and source origins.

use crate::{
    document::TextDocument,
    highlight::Language,
    view::{Kind, Side},
};
use std::{ops::Range, sync::Arc};

mod state;
pub use state::MarkdownState;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PresentationMode {
    #[default]
    Source,
    Rendered,
}
#[derive(Clone, Copy, Debug)]
pub enum MarkdownCommand {
    SetMode(PresentationMode),
    Select(RenderSelection),
    SetAnchor(SourceAnchor),
    ShowSource(SourceAnchor),
    ClearSelection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderSelection {
    pub side: Side,
    pub anchor: usize,
    pub head: usize,
}

mod parse;
pub use parse::parse_pair;
mod projection;
mod inline;
pub use inline::{InlineBlock, InlineSpan, inline_blocks};
pub use projection::{project, source_diff};

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
#[derive(Debug)]
pub struct ParsedPair {
    pub sides: [MarkdownDocument; 2],
}

#[derive(Clone, Debug)]
pub struct SourceHunk {
    pub id: usize,
    pub lines: [Range<u32>; 2],
    pub bytes: [Range<usize>; 2],
    pub inline: [Vec<Range<usize>>; 2],
    pub ignored: bool,
}
#[derive(Clone, Debug)]
pub struct SourceDiff {
    pub revision: u64,
    pub hunks: Vec<SourceHunk>,
}
#[derive(Debug)]
pub struct AlignmentGroup {
    pub blocks: [Range<usize>; 2],
    pub hunks: Vec<usize>,
}
#[derive(Clone, Debug)]
pub struct Decoration {
    pub hunk: usize,
    pub side: Side,
    pub text: Range<usize>,
    /// Full source hunk, retained even for a zero-width markup marker.
    pub source: Range<usize>,
    pub kind: Kind,
    pub markup: bool,
    pub ignored: bool,
}
#[derive(Debug)]
pub struct Projection {
    pub pair: Arc<ParsedPair>,
    pub groups: Vec<AlignmentGroup>,
    pub decorations: Vec<Decoration>,
    pub revision: u64,
    /// The Inline view's arrangement, in reading order.
    pub inline: Vec<InlineBlock>,
    /// Per side, the blocks added (new side) or removed (old side) as a whole, ascending: the Inline view's
    /// pairing, so side by side marks them with their bar alone rather than with whatever words matched.
    pub whole: [Vec<usize>; 2],
}
/// How the Preview shows two documents: one column with changes in place, or both side by side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MarkdownLayout {
    #[default]
    Inline,
    SideBySide,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceAnchor {
    pub side: Side,
    pub byte: usize,
}

/// The decoded document language also includes difftool `--name` labels.
pub fn eligible(doc: &TextDocument) -> bool {
    doc.placeholder_kind().is_none()
        && [Side::Left, Side::Right]
            .into_iter()
            .any(|side| doc.language(side) == Some(Language::Markdown))
}

#[cfg(test)]
mod tests;

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

/// The local image files the image blocks of comparison `current` show, per side: (block, file). A revision's
/// images come from the working tree, where the Markdown file would be; files outside the repository (or the
/// compared folders, or the Markdown file's folder) are refused. Both frontends.
pub fn image_sources(pair: &ParsedPair, set: &crate::changeset::ChangeSet, current: usize, spec: &crate::changeset::Spec, cwd: &std::path::Path) -> [Vec<(usize, std::path::PathBuf)>; 2] {
    let mut out: [Vec<(usize, std::path::PathBuf)>; 2] = Default::default();
    let Some(c) = set.comparisons.get(current) else { return out };
    for (i, sources) in out.iter_mut().enumerate() {
        let (content, path) = if i == 0 { (&c.left, &c.old_path) } else { (&c.right, &c.path) };
        let file = match content {
            crate::changeset::Content::File(p) | crate::changeset::Content::Symlink(p) => cwd.join(p),
            _ => match set.root() {
                Some(root) => root.join(path),
                None => continue,
            },
        };
        let folder = match spec {
            crate::changeset::Spec::Plain { left, right } => Some(cwd.join(if i == 0 { left } else { right })).filter(|d| d.is_dir()),
            _ => None,
        };
        let Some(root) = set.root().map(std::path::Path::to_path_buf).or(folder).or_else(|| file.parent().map(std::path::Path::to_path_buf)) else {
            continue;
        };
        for (b, block) in pair.sides[i].blocks.iter().enumerate() {
            if let BlockKind::Image { destination, .. } = &block.kind
                && sources.len() < MAX_IMAGES
                && let Some(image) = local_image(&root, &file, destination)
            {
                sources.push((b, image));
            }
        }
    }
    out
}

/// A local image file's RGBA pixels; `None` when it is larger than [`IMAGE_BYTES`] or not a readable image.
pub fn decode_image(path: &std::path::Path) -> Option<crate::image_session::Pixels> {
    if std::fs::metadata(path).ok()?.len() > IMAGE_BYTES as u64 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let format = crate::images::detect(&path.to_string_lossy(), &bytes)?;
    crate::images::decode(&bytes, format).ok().map(|d| d.pixels)
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
