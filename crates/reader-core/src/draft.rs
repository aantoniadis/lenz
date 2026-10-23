//! Session-owned compact edit history. Snapshots are current text or the confirmed baseline only.
use std::sync::Arc;
use crate::text::{EditError, TextEdit, TextSnapshot};

pub(crate) struct Draft {
    pub(crate) baseline: Arc<TextSnapshot>,
    dirty: bool,
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
}
struct Transaction { forward: Vec<TextEdit>, inverse: Vec<TextEdit> }
impl Draft {
    pub(crate) fn new(baseline: Arc<TextSnapshot>) -> Self { Self { baseline, dirty: false, undo: Vec::new(), redo: Vec::new() } }
    pub(crate) fn dirty(&self) -> bool { self.dirty }
    pub(crate) fn can_undo(&self) -> bool { !self.undo.is_empty() }
    pub(crate) fn can_redo(&self) -> bool { !self.redo.is_empty() }
    pub(crate) fn edit(&mut self, current: &TextSnapshot, version: u64, edits: Vec<TextEdit>) -> Result<Option<TextSnapshot>, EditError> {
        let next = current.apply_edits(version, &edits)?;
        if current.content_eq(&next) { return Ok(None); }
        let mut offset: isize = 0;
        let inverse = edits.iter().map(|edit| {
            let start = edit.range.start.checked_add_signed(offset).ok_or(EditError::InvalidRange)?;
            offset += edit.text.len() as isize - (edit.range.end - edit.range.start) as isize;
            Ok(TextEdit { range: start..start + edit.text.len(), text: current.copy_range(edit.range.clone()).ok_or(EditError::InvalidRange)?.into() })
        }).collect::<Result<Vec<_>, EditError>>()?;
        self.dirty = !self.baseline.content_eq(&next);
        self.undo.push(Transaction { forward: edits, inverse });
        self.redo.clear();
        Ok(Some(next))
    }
    pub(crate) fn undo(&mut self, current: &TextSnapshot, version: u64, redo: bool) -> Result<Option<(TextSnapshot, Vec<TextEdit>)>, EditError> {
        let from = if redo { &mut self.redo } else { &mut self.undo };
        let Some(transaction) = from.last() else { return Ok(None) };
        let edits = if redo { &transaction.forward } else { &transaction.inverse };
        let next = current.apply_edits(version, edits)?;
        self.dirty = !self.baseline.content_eq(&next);
        let edits = edits.clone();
        let Some(transaction) = from.pop() else { return Ok(None) };
        if redo { self.undo.push(transaction); } else { self.redo.push(transaction); }
        Ok(Some((next, edits)))
    }
}
