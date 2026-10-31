//! Draft change markers: which lines of the shown draft differ from its baseline — HEAD's blob for a tracked file,
//! otherwise the confirmed last saved version — as sorted line ranges. A frame asks only for the lines it draws, and a
//! settled version is compared once off the UI thread; nothing here is recomputed per keystroke or per frame.
//!
//! Change markers are not repository state: they describe the edited text against a named baseline, so saving a
//! tracked file does not clear them, while saving outside Git does (the saved text is then the baseline).

use std::{
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    time::Duration,
};

use lenz_content::{branches::FileStatus, scrollbar::MarkKind, theme::Palette};

use crate::{
    document::Document,
    session::SemanticSource,
    text::{BufferId, SnapshotKey},
};

/// How long a draft rests before its markers are compared again: a typed burst is one comparison, not one a keystroke.
pub const REST: Duration = Duration::from_millis(300);

/// A line's change against its baseline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineChange {
    /// The line is not in the baseline (or the file has no committed version at all).
    Added,
    /// The line is in the baseline and its text differs (or it stands in place of lines that did).
    Modified,
}

impl LineChange {
    /// Its gutter letter: the marker identified without colour.
    pub fn letter(self) -> &'static str {
        match self {
            Self::Added => "A",
            Self::Modified => "M",
        }
    }
    /// Its name in words, as the status line tells it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Modified => "modified",
        }
    }
    /// The palette's file-status colour, as the explorer and tabs badge it: added green, modified blue.
    pub fn color(self, p: &Palette) -> u32 {
        match self {
            Self::Added => FileStatus::Added.color(p),
            Self::Modified => FileStatus::Modified.color(p),
        }
    }
}

/// The baseline a draft's markers are computed against, named so no marker leaves it implicit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Baseline {
    /// HEAD's blob at the file's path: the tracked file's committed version (`label` names the branch or commit).
    Head { label: String },
    /// Git has no committed version at this path: a new file, its every line added against nothing.
    New { label: &'static str },
    /// No repository (or no committed file at this path): the confirmed last saved version.
    Saved,
}

/// Which baseline a markers set compares against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaselineKind {
    Head,
    New,
    Saved,
}

impl Baseline {
    pub fn kind(&self) -> BaselineKind {
        match self {
            Self::Head { .. } => BaselineKind::Head,
            Self::New { .. } => BaselineKind::New,
            Self::Saved => BaselineKind::Saved,
        }
    }
    /// As the status bar and the details name it: `HEAD (main · 1a2b3c4)`, the new file's account, or `last saved`.
    pub fn label(&self) -> &str {
        match self {
            Self::Head { label } => label,
            Self::New { label } => label,
            Self::Saved => "last saved",
        }
    }
    /// A narrow status bar's name for it.
    pub fn short(&self) -> &'static str {
        match self.kind() {
            BaselineKind::Head => "HEAD",
            BaselineKind::New => "new",
            BaselineKind::Saved => "saved",
        }
    }
}

/// The gutter's letter for lines deleted just above a line (their own letter, so the marker needs no colour).
pub const DELETED: &str = "D";

/// The colour of a deletion boundary's letter: the palette's deleted-file grey, beside the added and modified colours.
pub fn deleted_color(p: &Palette) -> u32 {
    FileStatus::Deleted.color(p)
}

/// A find match's background and text, shared by both Code adapters: stronger than the theme's find colour, so it
/// stands out over syntax colours.
pub fn match_colors(p: &Palette) -> (u32, u32) {
    let bg = lenz_content::theme::mix(p.found_bg, p.accent, 0.45);
    // Black or white, whichever contrasts more (WCAG ratio: (L1 + 0.05) / (L2 + 0.05)).
    let l = lenz_content::theme::luminance(bg);
    (bg, if (l + 0.05) / 0.05 >= 1.05 / (l + 0.05) { 0x000000 } else { 0xFFFFFF })
}

/// A scrollbar's thumb: [`lenz_content::scrollbar::thumb_color`], shared with Lenz Diff.
pub fn scrollbar_thumb_color(p: &Palette) -> u32 { lenz_content::scrollbar::thumb_color(p) }

/// A scrollbar mark's colour: its kind's colour elsewhere (problems, change marks, bookmarks); find matches the text's
/// match background, lifted toward its text colour (the current one more); the caret's dim.
pub fn scrollbar_mark_color(kind: MarkKind, p: &Palette) -> u32 {
    match kind {
        MarkKind::Error | MarkKind::Conflict => p.error,
        MarkKind::Warning | MarkKind::Current | MarkKind::Bookmark => p.accent,
        MarkKind::FindCurrent => { let (bg, fg) = match_colors(p); lenz_content::theme::mix(bg, fg, 0.45) }
        MarkKind::Find => { let (bg, fg) = match_colors(p); lenz_content::theme::mix(bg, fg, 0.2) }
        MarkKind::Added => LineChange::Added.color(p),
        MarkKind::Modified => LineChange::Modified.color(p),
        MarkKind::Deleted => deleted_color(p),
        MarkKind::Caret => p.dim,
    }
}

/// A version's change blocks, sorted for a drawn line's lookup by binary search.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Blocks {
    /// Draft line ranges (`start..end`) and their change, in order, without overlaps.
    changed: Vec<(u32, u32, LineChange)>,
    /// The draft line each run of deleted baseline lines sits above, and how many lines it removed.
    deleted: Vec<(u32, u32)>,
}

impl Blocks {
    /// The change of `line` of the draft, if any.
    pub fn change(&self, line: usize) -> Option<LineChange> {
        let line = u32::try_from(line).ok()?;
        let at = self.changed.partition_point(|(start, ..)| *start <= line);
        let &(start, end, change) = self.changed.get(at.checked_sub(1)?)?;
        (start <= line && line < end).then_some(change)
    }
    /// How many baseline lines were deleted just above `line` of the draft (several runs at one place add up).
    pub fn deleted_above(&self, line: usize) -> u32 {
        let Ok(line) = u32::try_from(line) else { return 0 };
        let Ok(at) = self.deleted.binary_search_by_key(&line, |(at, _)| *at) else { return 0 };
        let from = self.deleted[..at].iter().rposition(|(other, _)| *other != line).map_or(0, |i| i + 1);
        self.deleted[from..=at].iter().map(|(_, n)| *n).sum()
    }
    /// `3 added, 1 modified, 2 deleted lines`, or `no changes`.
    pub fn summary(&self) -> String {
        let count = |change: LineChange| self.changed.iter().filter(|(.., c)| *c == change).map(|(start, end, _)| (end - start) as usize).sum::<usize>();
        let deleted: u32 = self.deleted.iter().map(|(_, n)| *n).sum();
        let parts = [(count(LineChange::Added), "added"), (count(LineChange::Modified), "modified"), (deleted as usize, "deleted")]
            .into_iter()
            .filter(|(n, _)| *n > 0)
            .map(|(n, what)| format!("{n} {what}"))
            .collect::<Vec<_>>();
        if parts.is_empty() { "no changes".into() } else { parts.join(", ") }
    }
}

/// `right`'s lines against `left`'s: a block with one side empty is an insertion or a deletion; a block of both sides
/// pairs its lines by place, the extras past the shorter side being added lines (or removed ones). Whitespace counts,
/// as Git's own line diff does.
#[cfg(test)]
pub(crate) fn blocks(left: &[&str], right: &[&str]) -> Blocks {
    let options = lenz_engine::Options { highlight: lenz_engine::HighlightLevel::Line, ..Default::default() };
    from_changes(&lenz_engine::compare_lines(left, right, &options))
}

pub(crate) fn from_changes(changes: &[lenz_engine::ChangeBlock]) -> Blocks {
    let mut out = Blocks::default();
    for block in changes {
        if block.ignored {
            continue;
        }
        if block.left.is_empty() || block.right.is_empty() {
            out.push(block.right.start, block.left.len(), block.right.len());
            continue;
        }
        let paired = block.left.len().min(block.right.len());
        if paired > 0 {
            out.push(block.right.start, paired, paired);
        }
        if block.right.len() > paired {
            out.push(block.right.start.saturating_add(u32::try_from(paired).unwrap_or(u32::MAX)), 0, block.right.len() - paired);
        }
        if block.left.len() > paired {
            out.push(block.right.end, block.left.len() - paired, 0);
        }
    }
    out
}

impl Blocks {
    pub(crate) fn clamp_deletions(&mut self, last: u32) {
        let mut deleted: Vec<(u32,u32)> = Vec::new();
        for &(at,count) in &self.deleted { let at=at.min(last); if let Some((previous,n))=deleted.last_mut().filter(|(previous,_)|*previous==at) { let _=previous; *n+=count; }else{deleted.push((at,count));} }
        self.deleted=deleted;
    }
    /// Adds one block: a run of deleted baseline lines goes above `line`, everything else marks `right` lines. Runs of
    /// one kind are kept merged, so a modified block is one range however many lines it holds.
    fn push(&mut self, line: u32, left: usize, right: usize) {
        let count = u32::try_from(right).unwrap_or(u32::MAX);
        let end = line.saturating_add(count);
        match (left, right) {
            (0, 0) => {}
            (_, 0) => {
                let count = u32::try_from(left).unwrap_or(u32::MAX);
                match self.deleted.last_mut() {
                    Some((at, previous)) if *at == line => *previous = previous.saturating_add(count),
                    _ => self.deleted.push((line, count)),
                }
            }
            _ => {
                let change = if left == 0 { LineChange::Added } else { LineChange::Modified };
                match self.changed.last_mut() {
                    Some((_, previous_end, previous)) if *previous_end == line && *previous == change => *previous_end = end,
                    _ => self.changed.push((line, end, change)),
                }
            }
        }
    }
}

/// What a markers set describes once its comparison is published.
pub(crate) struct Published {
    /// The draft version the blocks are for.
    pub(crate) source: SemanticSource,
    pub(crate) baseline: Baseline,
    pub(crate) blocks: Blocks,
    pub(crate) baseline_text: Arc<crate::text::TextSnapshot>,
    pub(crate) hunks: Vec<Hunk>,
}

/// Exact original UTF-8 byte ranges for one settled line change, including its line endings.
#[derive(Clone, Debug)]
pub(crate) struct Hunk { pub left: std::ops::Range<usize>, pub right: std::ops::Range<usize>, pub lines: std::ops::Range<u32> }

#[derive(Clone)]
pub(crate) enum Guard { Head {path:PathBuf,sha:String}, New {path:PathBuf} }

pub(crate) struct Inspection {
    pub tab: crate::tabs::TabId,
    pub source: SemanticSource,
    pub epoch: u64,
    pub baseline: Baseline,
    pub baseline_text: Arc<crate::text::TextSnapshot>,
    pub hunk: Hunk,
    pub guard: Option<Guard>,
}

/// How a buffer's markers stand.
pub(crate) enum MarkerState {
    /// A newer version or baseline is being compared: the last published blocks stay drawn, named as pending.
    Pending,
    Ready,
    /// There are no markers for this file: those lines cannot change.
    Failed(String),
}

/// HEAD's blob as last read, kept while the repository's read epoch does not move.
pub(crate) struct HeadBlob {
    pub(crate) sha: String,
    pub(crate) label: String,
    pub(crate) document: Arc<Document>,
    pub(crate) epoch: u64,
}

/// What a buffer's markers were computed for: compared on each refresh, so work follows only real changes.
#[derive(Clone, PartialEq)]
pub(crate) struct Tracked {
    pub(crate) key: SnapshotKey,
    pub(crate) path: PathBuf,
    /// Whether the file belongs to the session's repository, as its statuses say.
    pub(crate) in_repo: bool,
    /// Its status against HEAD, as last read (`None`: unchanged, or outside the repository).
    pub(crate) status: Option<FileStatus>,
    /// The repository read epoch: bumped whenever HEAD or the statuses are read again.
    pub(crate) epoch: u64,
    /// The confirmed last saved version: the baseline outside Git.
    pub(crate) saved: Option<SemanticSource>,
}

/// One buffer's draft change markers.
pub struct Markers {
    pub(crate) tracked: Tracked,
    /// The comparison being asked for (0: none).
    pub(crate) request: u64,
    pub(crate) state: MarkerState,
    pub(crate) published: Option<Published>,
    pub(crate) head: Option<HeadBlob>,
}

impl Markers {
    pub(crate) fn new(tracked: Tracked) -> Self {
        Self { tracked, request: 0, state: MarkerState::Pending, published: None, head: None }
    }
    /// The baseline the published blocks compare against (`None` until the first comparison is published).
    pub fn baseline(&self) -> Option<&Baseline> {
        self.published.as_ref().map(|p| &p.baseline)
    }
    /// `line`'s change against the baseline, as drawn now.
    pub fn change(&self, line: usize) -> Option<LineChange> {
        self.published.as_ref()?.blocks.change(line)
    }
    /// The published changed ranges (draft lines) in document order.
    pub fn blocks(&self) -> impl Iterator<Item = (std::ops::Range<usize>, LineChange)> + '_ {
        self.published.iter().flat_map(|p| p.blocks.changed.iter().map(|&(start, end, change)| (start as usize..end as usize, change)))
    }
    /// The draft line each published deletion sits above.
    pub fn deletions(&self) -> impl Iterator<Item = usize> + '_ {
        self.published.iter().flat_map(|p| p.blocks.deleted.iter().map(|&(at, _)| at as usize))
    }
    /// How many baseline lines were deleted just above `line`.
    pub fn deleted_above(&self, line: usize) -> u32 {
        self.published.as_ref().map_or(0, |p| p.blocks.deleted_above(line))
    }
    /// The draft version the drawn blocks describe (`None`: nothing is drawn yet).
    pub fn drawn_version(&self) -> Option<u64> {
        self.published.as_ref().map(|published| published.source.key.version)
    }
    /// A newer version or baseline is being compared; what is drawn is the last published comparison.
    pub fn pending(&self) -> bool {
        matches!(self.state, MarkerState::Pending)
    }
    /// Why this file has no markers, if it has none.
    pub fn failure(&self) -> Option<&str> {
        match &self.state {
            MarkerState::Failed(why) => Some(why),
            MarkerState::Pending | MarkerState::Ready => None,
        }
    }
    /// What is drawn, in words: `vs HEAD (main · 1a2b3c4): 1 added, 2 modified` (and that it is catching up).
    pub fn summary(&self) -> Option<String> {
        let published = self.published.as_ref()?;
        let baseline = match &published.baseline {
            Baseline::Saved => "vs last saved".to_owned(),
            other => format!("vs {}", other.label()),
        };
        let pending = if self.pending() { " (updating…)" } else { "" };
        Some(format!("{baseline}: {}{pending}", published.blocks.summary()))
    }
    /// The caret line's marker in words, for the status line: `Modified from HEAD (main · 1a2b3c4)`.
    pub fn line_label(&self, line: usize) -> Option<String> {
        let baseline = self.baseline()?;
        match self.change(line) {
            Some(change) => Some(format!("{} from {}", capitalize(change.label()), baseline.label())),
            None => match self.deleted_above(line) {
                0 => None,
                1 => Some(format!("1 line deleted from {}", baseline.label())),
                n => Some(format!("{n} lines deleted from {}", baseline.label())),
            },
        }
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().collect::<String>() + chars.as_str())
}

impl crate::session::State {
    /// Tab `id`'s draft change markers (none for a saved version, an untitled document or a source never read).
    pub fn markers(&self, id: crate::tabs::TabId) -> Option<&Markers> {
        let tab = self.tabs().iter().find(|tab| tab.id() == id)?;
        self.marker_sets.get(&tab.buffer?)
    }
}

/// Schedules one comparison per buffer once its draft has rested: a single thread waits out
/// [`REST`] after the newest request, so continuous typing makes no whole-document comparison.
pub(crate) struct Debounce {
    tx: Option<Sender<BufferId>>,
    /// No thread could start: wake at once rather than never (a comparison per change, as if there were no rest).
    fallback: Option<Box<dyn Fn() + Send>>,
}

impl Debounce {
    pub fn new(wake: impl Fn() + Send + 'static + Clone) -> Self {
        let (tx, rx) = mpsc::channel::<BufferId>();
        let thread = {
            let wake = wake.clone();
            std::thread::Builder::new().name("reader-markers".into()).spawn(move || rest(&rx, REST, &wake))
        };
        match thread {
            Ok(_) => Self { tx: Some(tx), fallback: None },
            Err(_) => Self { tx: None, fallback: Some(Box::new(wake)) },
        }
    }
    /// `buffer`'s draft changed: compare it once it rests.
    pub fn rest(&self, buffer: BufferId) {
        match (&self.tx, &self.fallback) {
            (Some(tx), _) => { let _ = tx.send(buffer); }
            (None, Some(wake)) => wake(),
            (None, None) => {}
        }
    }
}

/// Waits until no request arrived for `rest`, then wakes the session once; ends with the channel.
fn rest(rx: &Receiver<BufferId>, rest: Duration, wake: &(impl Fn() + Send + 'static)) {
    let mut waiting = false;
    loop {
        let next = if waiting { rx.recv_timeout(rest) } else { rx.recv().map_err(|_| RecvTimeoutError::Disconnected) };
        match next {
            Ok(_) => waiting = true,
            Err(RecvTimeoutError::Timeout) => {
                waiting = false;
                wake();
            }
            // A closed channel still flushes a request that was already resting.
            Err(RecvTimeoutError::Disconnected) => { if waiting { wake(); } return; }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// Every mark a click can reach stands out from the track and from the thumb it may sit on, in both built-in palettes.
    #[test]
    fn scrollbar_marks_contrast_with_the_track_and_the_thumb() {
        use lenz_content::theme::{contrast, DARCULA, LIGHT};
        use MarkKind::*;
        for (name, p) in [("Darcula", &DARCULA), ("Light", &LIGHT)] {
            for kind in [Error, Conflict, Warning, Current, FindCurrent, Find, Added, Deleted, Modified, Bookmark].into_iter().filter(|k| k.jumps()) {
                let c = scrollbar_mark_color(kind, p);
                for (under, bg) in [("track", lenz_content::scrollbar::track_color(p)), ("thumb", scrollbar_thumb_color(p))] {
                    assert!(contrast(c, bg) >= 1.8, "{name} {kind:?} on the {under}: {:.2}", contrast(c, bg));
                }
            }
            assert!(contrast(scrollbar_mark_color(FindCurrent, p), p.gutter_bg) > contrast(scrollbar_mark_color(Find, p), p.gutter_bg), "{name}: the current match is stronger");
        }
    }

    #[test]
    fn blocks_name_insertions_modifications_and_deletions_by_line() {
        let left = ["one", "two", "three", "four", "five"];
        let right = ["one", "TWO", "three and a half", "four", "FIVE", "six"];
        let b = blocks(&left, &right);
        assert_eq!(b.change(0), None);
        assert_eq!(b.change(1), Some(LineChange::Modified));
        assert_eq!(b.change(2), Some(LineChange::Modified), "a replaced line stays modified");
        assert_eq!(b.change(3), None);
        assert_eq!((b.change(4), b.change(5)), (Some(LineChange::Modified), Some(LineChange::Added)), "added after a replaced line");
        assert_eq!((b.deleted_above(0), b.deleted_above(5)), (0, 0));
        assert_eq!(b.summary(), "1 added, 3 modified");
        let b = blocks(&["a", "b", "c"], &["a", "c"]);
        assert_eq!(b.deleted_above(1), 1);
        assert_eq!(b.summary(), "1 deleted");
        // A file with no committed version: every line is new, nothing is deleted.
        let b = blocks(&[], &["a", "b"]);
        assert_eq!(b.change(1), Some(LineChange::Added));
        assert_eq!(b.summary(), "2 added");
        assert_eq!(blocks(&["a"], &["a"]).summary(), "no changes");
        // Lookups past the end are answered without indexing.
        assert_eq!((b.change(usize::MAX), b.deleted_above(usize::MAX)), (None, 0));
    }

    #[test]
    fn a_waiting_rest_wakes_once_after_the_last_request() {
        let (requests, rx) = mpsc::channel::<BufferId>();
        let (wakes, woke) = mpsc::channel::<Duration>();
        let started = Instant::now();
        std::thread::scope(|s| {
            s.spawn(move || rest(&rx, Duration::from_millis(40), &move || { let _ = wakes.send(started.elapsed()); }));
            for _ in 0..3 {
                requests.send(BufferId(1)).unwrap();
                std::thread::sleep(Duration::from_millis(10));
            }
            let at = woke.recv_timeout(Duration::from_secs(2)).expect("a wake after the last request rested");
            assert!(at >= Duration::from_millis(60), "not before the last request rested: {at:?}");
            assert!(woke.recv_timeout(Duration::from_millis(200)).is_err(), "one wake, not one a request");
            drop(requests);
        });
    }
}
