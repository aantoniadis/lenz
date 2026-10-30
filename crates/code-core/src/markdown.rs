//! A Markdown document's preview: parsed within Lenz's limits, its code blocks highlighted and its local images
//! decoded, all off the UI thread for one document version. Blocks keep their source ranges, so the reading
//! location (a source line) holds in both presentations.

use std::{collections::HashMap, ops::Range, path::Path, sync::Arc};

use lenz_content::{
    highlight::Capture,
    image::Pixels,
    markdown::{self, BlockKind, Limits, MarkdownDocument, MarkdownError},
};

/// Syntax spans per code block (by block index), in the rendered text.
pub type CodeSyntax = HashMap<usize, Vec<(Range<usize>, Capture)>>;

/// Image blocks' pixels or why they are not shown, by block index.
pub type Images = HashMap<usize, Result<Arc<Pixels>, &'static str>>;

/// One document version's preview.
#[derive(Debug)]
pub struct Preview {
    /// The document version it was made from (its source bytes are that version's).
    pub version: u64,
    pub doc: Arc<MarkdownDocument>,
    pub syntax: Arc<CodeSyntax>,
    /// Its local images, once decoded (`None` meanwhile).
    pub images: Option<Images>,
}

impl Preview {
    /// Memory its images hold.
    pub fn bytes(&self) -> usize {
        self.images.iter().flat_map(|i| i.values()).filter_map(|i| i.as_ref().ok()).map(|p| p.as_raw().len()).sum()
    }
}

/// Parse `source` and highlight its code blocks while `current` holds; `Err` says why it cannot be previewed.
pub(crate) fn render(source: Arc<str>, current: impl Fn() -> bool + Send + Clone + 'static) -> Result<Preview, String> {
    let mut remaining = Limits::DEFAULT.segments_per_pair;
    let doc = markdown::parse(source, Limits::DEFAULT, &mut remaining, &current.clone()).map_err(|e| match e {
        MarkdownError::Limit(what) => format!("its {what} exceed the preview's limit"),
        MarkdownError::Cancelled => "superseded".into(),
        MarkdownError::InvalidMap | MarkdownError::Interrupted => "it could not be mapped to its source".into(),
    })?;
    let syntax = markdown::code_syntax(&doc, current);
    Ok(Preview { version: 0, doc: Arc::new(doc), syntax: Arc::new(syntax), images: None })
}

/// Image results by destination, from an earlier preview of the same file: what an edited version reuses rather
/// than decoding again.
pub(crate) type Decoded = HashMap<String, Result<Arc<Pixels>, &'static str>>;

/// `preview`'s image results by destination (the first block's, for a destination shown twice).
pub(crate) fn decoded(preview: &Preview) -> Decoded {
    let Some(images) = &preview.images else { return Decoded::new() };
    let mut out = Decoded::new();
    for (b, block) in preview.doc.blocks.iter().enumerate() {
        if let (BlockKind::Image { destination, .. }, Some(image)) = (&block.kind, images.get(&b)) {
            out.entry(destination.clone()).or_insert_with(|| image.clone());
        }
    }
    out
}

/// The image results `reuse` already holds for every image block of `doc`, or `None` when one is missing.
pub(crate) fn reused(doc: &MarkdownDocument, reuse: &Decoded) -> Option<Images> {
    let mut images = HashMap::new();
    for (b, block) in doc.blocks.iter().enumerate() {
        let BlockKind::Image { destination, .. } = &block.kind else { continue };
        if images.len() >= markdown::MAX_IMAGES { return None; }
        images.insert(b, reuse.get(destination)?.clone());
    }
    Some(images)
}

/// The document's local images, decoded while `current` holds: only files inside `root` (resolved), at most
/// [`markdown::MAX_IMAGES`], each within [`markdown::IMAGE_BYTES`]. A destination `reuse` holds (an earlier
/// version's result) is taken from there instead of being read again.
pub(crate) fn images(doc: &MarkdownDocument, file: &Path, root: &Path, reuse: &Decoded, current: impl Fn() -> bool) -> Images {
    let mut images = HashMap::new();
    for (b, block) in doc.blocks.iter().enumerate() {
        let BlockKind::Image { destination, .. } = &block.kind else { continue };
        if !current() {
            break;
        }
        let image = if images.len() >= markdown::MAX_IMAGES {
            Err("more images than the preview shows")
        } else if let Some(image) = reuse.get(destination) {
            image.clone()
        } else {
            match markdown::local_image(root, file, destination) {
                None => Err("not shown: remote, missing, outside the folder or not a supported image"),
                Some(path) => markdown::decode_image(&path).map(Arc::new).ok_or("cannot be decoded, or over the preview's image size limit"),
            }
        };
        images.insert(b, image);
    }
    images
}
