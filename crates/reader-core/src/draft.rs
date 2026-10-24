//! Session-owned compact edit history. Snapshots are current text or the confirmed baseline only.
use std::sync::Arc;
use crate::{tabs::TabId, text::{EditError, EditKind, TextEdit, TextSnapshot}};
use std::time::{Duration, Instant};

pub(crate) struct Draft {
    pub(crate) baseline: Arc<TextSnapshot>,
    dirty: bool,
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
    join: Option<Join>,
}
struct Join { tab: TabId, kind: EditKind, at: Instant, direction: i8 }
struct Transaction { forward: Vec<TextEdit>, inverse: Vec<TextEdit> }
impl Draft {
    pub(crate) fn new(baseline: Arc<TextSnapshot>) -> Self { Self { baseline, dirty: false, undo: Vec::new(), redo: Vec::new(), join: None } }
    pub(crate) fn dirty(&self) -> bool { self.dirty }
    pub(crate) fn can_undo(&self) -> bool { !self.undo.is_empty() }
    pub(crate) fn can_redo(&self) -> bool { !self.redo.is_empty() }
    pub(crate) fn end_group(&mut self) { self.join = None; }
    /// Saving establishes a new confirmed baseline while retaining every undo/redo transaction.
    pub(crate) fn confirm_baseline(&mut self, captured: Arc<TextSnapshot>, current: &TextSnapshot) {
        self.end_group();
        self.dirty = !captured.content_eq(current);
        self.baseline = captured;
    }
    pub(crate) fn edit(&mut self, current: &TextSnapshot, version: u64, edits: Vec<TextEdit>, tab: TabId, kind: EditKind, now: Instant) -> Result<Option<TextSnapshot>, EditError> {
        let next = current.apply_edits(version, &edits)?;
        if current.content_eq(&next) { return Ok(None); }
        let mut offset: isize = 0;
        let inverse = edits.iter().map(|edit| {
            let start = edit.range.start.checked_add_signed(offset).ok_or(EditError::InvalidRange)?;
            offset += edit.text.len() as isize - (edit.range.end - edit.range.start) as isize;
            Ok(TextEdit { range: start..start + edit.text.len(), text: current.copy_range(edit.range.clone()).ok_or(EditError::InvalidRange)?.into() })
        }).collect::<Result<Vec<_>, EditError>>()?;
        // Adjacent removals can become insertions at the same output offset.
        let mut combined: Vec<TextEdit> = Vec::with_capacity(inverse.len());
        for edit in inverse {
            if let Some(previous) = combined.last_mut().filter(|previous| previous.range.start == edit.range.start) {
                previous.range.end = edit.range.end;
                previous.text = format!("{}{}", previous.text, edit.text).into();
            } else { combined.push(edit); }
        }
        let inverse = combined;
        self.dirty = !self.baseline.content_eq(&next);
        let joined = self.merge(&edits, &inverse, tab, kind, now);
        if !joined { self.undo.push(Transaction { forward: edits, inverse }); }
        if !joined { self.join = matches!(kind, EditKind::Typing | EditKind::Delete)
            .then_some(Join { tab, kind, at: now, direction: 0 }); }
        self.redo.clear();
        Ok(Some(next))
    }
    fn merge(&mut self, edits: &[TextEdit], inverse: &[TextEdit], tab: TabId, kind: EditKind, now: Instant) -> bool {
        let joins = self.join.as_ref().is_some_and(|join| join.tab == tab && join.kind == kind
            && now.saturating_duration_since(join.at) <= Duration::from_millis(500));
        if !joins { return false; }
        if kind == EditKind::Typing && self.merge_carets(edits) {
            if let Some(join) = self.join.as_mut() { join.at = now; }
            return true;
        }
        let Some(join) = self.join.as_mut() else { return false };
        let (Some(previous), [edit], [removed]) = (self.undo.last_mut(), edits, inverse) else { return false };
        let ([forward], [backward]) = (previous.forward.as_mut_slice(), previous.inverse.as_mut_slice()) else { return false };
        match kind {
            EditKind::Typing if forward.range.is_empty() && edit.range.is_empty()
                && forward.range.start + forward.text.len() == edit.range.start => {
                forward.text = format!("{}{}", forward.text, edit.text).into();
                backward.range.end += edit.text.len();
            }
            EditKind::Delete if forward.text.is_empty() && edit.text.is_empty() && !edit.range.is_empty() => {
                if edit.range.end == forward.range.start && join.direction <= 0 {
                    forward.range.start = edit.range.start;
                    backward.range = edit.range.start..edit.range.start;
                    backward.text = format!("{}{}", removed.text, backward.text).into();
                    join.direction = -1;
                } else if edit.range.start == forward.range.start && join.direction >= 0 {
                    forward.range.end += edit.range.len();
                    backward.text = format!("{}{}", backward.text, removed.text).into();
                    join.direction = 1;
                } else { return false; }
            }
            EditKind::Typing | EditKind::Delete | EditKind::Paste | EditKind::Replace => return false,
        }
        join.at = now;
        true
    }
    /// Several carets typing: the last transaction inserted at each of them, and this keystroke inserts again at each
    /// caret left behind. Every insertion grows in place and every inverse range shifts by the text added before it,
    /// so one Undo takes the whole typing group, as one caret's typing does.
    fn merge_carets(&mut self, edits: &[TextEdit]) -> bool {
        let Some(previous) = self.undo.last_mut() else { return false };
        let carets = edits.len();
        let same_carets = carets > 0 && previous.forward.len() == carets && previous.inverse.len() == carets;
        let typing = edits.iter().all(|edit| edit.range.is_empty())
            && previous.forward.iter().all(|forward| forward.range.is_empty());
        // Each insertion continues the caret the last one left: the end of that caret's inserted text.
        let at_carets = edits.iter().zip(&previous.inverse).all(|(edit, backward)| edit.range.start == backward.range.end);
        let ordered = edits.windows(2).all(|pair| pair[0].range.start < pair[1].range.start)
            && previous.inverse.windows(2).all(|pair| pair[0].range.end <= pair[1].range.start);
        if !(same_carets && typing && at_carets && ordered) { return false; }
        let mut shifted = 0;
        for ((forward, backward), edit) in previous.forward.iter_mut().zip(previous.inverse.iter_mut()).zip(edits) {
            let added = edit.text.len();
            backward.range.start += shifted;
            backward.range.end += shifted + added;
            forward.text = format!("{}{}", forward.text, edit.text).into();
            shifted += added;
        }
        true
    }

    pub(crate) fn undo(&mut self, current: &TextSnapshot, version: u64, redo: bool) -> Result<Option<(TextSnapshot, Vec<TextEdit>)>, EditError> {
        self.end_group();
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
