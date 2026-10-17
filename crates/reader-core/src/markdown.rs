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
    Ok(Preview { doc: Arc::new(doc), syntax: Arc::new(syntax), images: None })
}

/// The document's local images, decoded while `current` holds: only files inside `root` (resolved), at most
/// [`markdown::MAX_IMAGES`], each within [`markdown::IMAGE_BYTES`].
pub(crate) fn images(doc: &MarkdownDocument, file: &Path, root: &Path, current: impl Fn() -> bool) -> Images {
    let mut images = HashMap::new();
    for (b, block) in doc.blocks.iter().enumerate() {
        let BlockKind::Image { destination, .. } = &block.kind else { continue };
        if !current() {
            break;
        }
        let image = if images.len() >= markdown::MAX_IMAGES {
            Err("more images than the preview shows")
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
