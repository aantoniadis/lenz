//! Markdown comparisons: two documents parsed under one budget, their source diff and projection, the Preview's
//! state and commands. Single documents (parsing, source origins, local images) are shared through
//! [`lenz_content::markdown`] and re-exported here.

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

pub use lenz_content::markdown::*;

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

/// The source view's position of a document's decoded byte.
pub trait SourcePos {
    fn source_pos(&self, byte: usize) -> Option<crate::selection::Pos>;
}

impl SourcePos for MarkdownDocument {
    /// Translate decoded bytes (including CRLF) to the source view's raw position.
    fn source_pos(&self, byte: usize) -> Option<crate::selection::Pos> {
        self.line_column(byte).map(|(line, column)| crate::selection::Pos::new(line, column))
    }
}
