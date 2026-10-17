//! Soft wrap: a line's rows at a pane's width in columns, made for the lines shown or stepped over and kept for one
//! document version, width and set of inlay hints.

use std::{collections::HashMap, ops::Range, sync::Arc};

use lenz_content::wrap::{continuation_indent, wrap_starts};

/// One row of a (possibly wrapped) line: its part of the displayed text and of the source line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub line: usize,
    /// 0 for the line's first row.
    pub index: usize,
    pub display: Range<usize>,
    pub source: Range<usize>,
    /// Columns before the text (continuation rows only).
    pub indent: usize,
}

#[derive(Debug, Default)]
pub struct WrapCache {
    version: u64,
    cols: usize,
    hints: Option<Arc<crate::decor::Hints>>,
    lines: HashMap<usize, Arc<[Row]>>,
}

impl WrapCache {
    /// `line`'s rows at `cols` with `hints` drawn (one row when it fits).
    pub fn line_rows(&mut self, doc: &crate::document::Document, line: usize, cols: usize, hints: Option<&Arc<crate::decor::Hints>>) -> Arc<[Row]> {
        // The hints are held, so a new set cannot reuse their address.
        let same = self.hints.as_ref().map(Arc::as_ptr) == hints.map(Arc::as_ptr);
        if (self.version, self.cols) != (doc.version(), cols) || !same {
            *self = Self { version: doc.version(), cols, hints: hints.cloned(), lines: HashMap::new() };
        }
        self.lines.entry(line).or_insert_with(|| rows_of(doc.line(line), hints.map_or(&[][..], |h| h.line(line)), line, cols).into()).clone()
    }
}

/// The source byte shown at display offset `d` of a line whose display map is `map` (a source byte's display
/// offset, one past the end included).
pub fn source_of(map: &[usize], d: usize) -> usize {
    map.partition_point(|&x| x < d).min(map.len().saturating_sub(1))
}

fn rows_of(text: &str, hints: &[crate::decor::Hint], line: usize, cols: usize) -> Vec<Row> {
    let crate::shown::Shown { text: shown, map, .. } = crate::shown::shown_line(text, hints);
    let starts = wrap_starts(&shown, cols);
    let indent = continuation_indent(&shown, cols);
    let bounds: Vec<usize> = std::iter::once(0).chain(starts).chain(std::iter::once(shown.len())).collect();
    bounds
        .windows(2)
        .enumerate()
        .map(|(index, w)| Row { line, index, display: w[0]..w[1], source: source_of(&map, w[0])..source_of(&map, w[1]), indent: if index == 0 { 0 } else { indent } })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> crate::document::Document {
        crate::document::Document::decode("/t.txt".into(), 1, text.as_bytes())
    }

    #[test]
    fn a_line_splits_into_rows_with_source_and_display_ranges() {
        let d = doc("\tab cd ef\nx\n");
        let mut c = WrapCache::default();
        let rows = c.line_rows(&d, 0, 6, None);
        // Displayed "    ab cd ef" (a tab is four columns): "    ab ", "cd ", "ef"; continuation rows indented 3.
        assert_eq!(rows.iter().map(|r| r.display.clone()).collect::<Vec<_>>(), vec![0..7, 7..10, 10..12]);
        assert_eq!(rows.iter().map(|r| r.source.clone()).collect::<Vec<_>>(), vec![0..4, 4..7, 7..9]);
        assert_eq!((rows[0].indent, rows[1].indent, rows[1].index), (0, 3, 1));
        assert_eq!(c.line_rows(&d, 1, 6, None).len(), 1);
    }

    #[test]
    fn the_cache_is_per_version_and_width() {
        let d = doc("aaaa bbbb\n");
        let mut c = WrapCache::default();
        assert_eq!(c.line_rows(&d, 0, 5, None).len(), 2);
        assert_eq!(c.line_rows(&d, 0, 20, None).len(), 1, "a new width recomputes");
    }
    #[test]
    fn hints_take_columns_in_wrapped_rows_and_rewrap_when_they_change() {
        let d = doc("ab cd\n");
        let hints = Arc::new(crate::decor::Hints::new(vec![(0, crate::decor::Hint { byte: 2, label: ": xyz".into(), kind: crate::decor::HintKind::Type })]));
        let mut c = WrapCache::default();
        assert_eq!(c.line_rows(&d, 0, 6, None).len(), 1);
        let rows = c.line_rows(&d, 0, 6, Some(&hints));
        // Shown "ab: xyz cd" (10 columns): "ab: " then "xyz cd".
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].display.end, "ab: xyz cd".len());
        assert_eq!((rows[0].source.start, rows[0].source.end, rows[1].source.end), (0, rows[1].source.start, 5));
        assert_eq!(c.line_rows(&d, 0, 6, None).len(), 1, "other hints: made again");
    }
}
