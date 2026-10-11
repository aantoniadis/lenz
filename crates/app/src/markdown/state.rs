//! Lazy ownership of one document's rendered presentation and cancellable work.
use super::{
    Limits, MarkdownCommand, MarkdownError, PresentationMode, Projection, RenderSelection,
    SourceAnchor, SourceDiff, eligible, parse_pair, project,
};
use crate::{document::TextDocument, view::Side};
use std::sync::{
    Arc,
    mpsc::{Receiver, TryRecvError},
};

type Inbox = Receiver<Option<Result<Projection, MarkdownError>>>;
struct Pending {
    token: Arc<()>,
    revision: u64,
    inbox: Inbox,
}

#[derive(Default)]
pub struct MarkdownState {
    mode: PresentationMode,
    preferred: PresentationMode,
    sources: Option<[Arc<str>; 2]>,
    ready: Option<Arc<Projection>>,
    pending: Option<Pending>,
    revision: Option<u64>,
    notice: Option<String>,
    anchors: [Option<SourceAnchor>; 2],
    selection: Option<RenderSelection>,
}
impl MarkdownState {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn execute(
        &mut self,
        command: MarkdownCommand,
        doc: Option<Arc<TextDocument>>,
        diff: Option<SourceDiff>,
    ) {
        self.execute_document(command, doc.as_deref(), diff);
    }
    pub(crate) fn execute_document(
        &mut self,
        command: MarkdownCommand,
        doc: Option<&TextDocument>,
        diff: Option<SourceDiff>,
    ) {
        match command {
            MarkdownCommand::SetMode(mode) => {
                self.preferred = mode;
                if mode == PresentationMode::Source {
                    self.cancel();
                    self.mode = mode;
                    self.notice = None;
                    return;
                }
                self.refresh(doc, diff);
            }
            MarkdownCommand::ClearSelection => self.selection = None,
            MarkdownCommand::Select(selection) => {
                self.selection = self
                    .ready
                    .as_ref()
                    .filter(|p| p.valid_selection(selection))
                    .map(|_| selection);
            }
            MarkdownCommand::SetAnchor(anchor) | MarkdownCommand::ShowSource(anchor) => {
                let index = if anchor.side == Side::Left { 0 } else { 1 };
                if self
                    .sources
                    .as_ref()
                    .is_some_and(|s| s[index].is_char_boundary(anchor.byte))
                {
                    self.anchors[index] = Some(anchor);
                    if matches!(command, MarkdownCommand::ShowSource(_)) {
                        self.cancel();
                        self.mode = PresentationMode::Source;
                        self.preferred = PresentationMode::Source;
                        self.notice = None;
                    }
                }
            }
        }
    }
    pub(crate) fn refresh(&mut self, doc: Option<&TextDocument>, diff: Option<SourceDiff>) {
        let Some(doc) = doc.filter(|doc| eligible(doc)) else {
            self.invalidate();
            return;
        };
        // The two source allocations are stable across options/refinement, but change on edits.
        let sources = [doc.text_arc(Side::Left), doc.text_arc(Side::Right)];
        if !self
            .sources
            .as_ref()
            .is_some_and(|old| old.iter().zip(&sources).all(|(a, b)| Arc::ptr_eq(a, b)))
        {
            self.invalidate();
            self.sources = Some(sources.clone());
        }
        if self.preferred == PresentationMode::Source {
            return;
        }
        let Some(diff) = diff else {
            return;
        };
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.revision == diff.revision)
        {
            return;
        }
        if self
            .ready
            .as_ref()
            .is_some_and(|p| p.revision == diff.revision)
        {
            self.cancel();
            self.mode = PresentationMode::Rendered;
            return;
        }
        self.cancel();
        self.notice = None;
        self.revision = Some(diff.revision);
        let pair = self.ready.as_ref().map(|p| p.pair.clone());
        let token = Arc::new(());
        let weak = Arc::downgrade(&token);
        let revision = diff.revision;
        let inbox = crate::work::submit(weak.clone(), move || {
            let current = || weak.strong_count() > 0;
            let pair = match pair {
                Some(pair) => pair,
                None => Arc::new(parse_pair(sources, Limits::DEFAULT, &current)?),
            };
            project(pair, diff, &current)
        });
        self.pending = Some(Pending {
            token,
            revision,
            inbox,
        });
    }
    pub fn poll(&mut self) -> bool {
        let Some(pending) = &self.pending else {
            return false;
        };
        let result = match pending.inbox.try_recv() {
            Ok(Some(result)) => result,
            Ok(None) | Err(TryRecvError::Disconnected) => Err(MarkdownError::Interrupted),
            Err(TryRecvError::Empty) => return false,
        };
        let pending = self.pending.take().expect("pending Markdown request");
        if self.revision != Some(pending.revision) || Arc::strong_count(&pending.token) != 1 {
            return false;
        }
        match result {
            Ok(projection)
                if projection.revision == pending.revision
                    && self.sources.as_ref().is_some_and(|sources| {
                        sources
                            .iter()
                            .zip(&projection.pair.sides)
                            .all(|(source, side)| Arc::ptr_eq(source, &side.source))
                    }) =>
            {
                self.ready = Some(Arc::new(projection));
                self.mode = PresentationMode::Rendered;
                self.notice = None;
            }
            Ok(_) => self.fail(MarkdownError::Interrupted),
            Err(error) => self.fail(error),
        }
        true
    }
    fn fail(&mut self, error: MarkdownError) {
        self.mode = PresentationMode::Source;
        self.ready = None;
        self.notice = Some(match error {
            MarkdownError::Limit(limit) => {
                format!("Markdown preview exceeds {limit} — showing Source")
            }
            MarkdownError::Cancelled | MarkdownError::Interrupted => {
                "Markdown preview preparation was interrupted — showing Source".into()
            }
            MarkdownError::InvalidMap => {
                "Markdown preview source mapping failed — showing Source".into()
            }
        });
    }
    pub(crate) fn cancel(&mut self) {
        self.pending = None;
        self.revision = None;
    }
    pub fn invalidate(&mut self) {
        self.cancel();
        self.sources = None;
        self.ready = None;
        self.mode = PresentationMode::Source;
        self.notice = None;
        self.anchors = [None; 2];
        self.selection = None;
    }
    pub fn mode(&self) -> PresentationMode {
        self.mode
    }
    pub fn preferred_mode(&self) -> PresentationMode {
        self.preferred
    }
    pub fn loading(&self) -> bool {
        self.pending.is_some()
    }
    pub fn projection(&self) -> Option<&Arc<Projection>> {
        self.ready.as_ref()
    }
    pub fn selection(&self) -> Option<RenderSelection> {
        self.selection
    }
    pub fn anchor(&self, side: Side) -> Option<SourceAnchor> {
        self.anchors[if side == Side::Left { 0 } else { 1 }]
    }
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }
    pub fn edit_disabled(&self) -> Option<&'static str> {
        (self.mode == PresentationMode::Rendered || self.loading())
            .then_some("Switch to Source to edit")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn markdown_state_disconnected_worker_stops_loading() {
        let mut state = MarkdownState::new();
        let (sender, inbox) = std::sync::mpsc::channel();
        drop(sender);
        state.revision = Some(1);
        state.pending = Some(Pending {
            token: Arc::new(()),
            revision: 1,
            inbox,
        });
        assert!(state.poll());
        assert!(!state.loading());
        assert_eq!(state.mode(), PresentationMode::Source);
        assert!(state.notice().unwrap().contains("interrupted"));
    }
    #[test]
    fn markdown_state_rejects_wrong_document_or_revision_and_releases_source_on_close() {
        for wrong_revision in [false, true] {
            let doc = TextDocument::new(
                "# a",
                "# b",
                std::path::Path::new("a.md"),
                std::path::Path::new("b.md"),
            );
            let sources = [doc.text_arc(Side::Left), doc.text_arc(Side::Right)];
            let pair = Arc::new(
                parse_pair(
                    if wrong_revision {
                        sources.clone()
                    } else {
                        ["# other".into(), "# other".into()]
                    },
                    Limits::DEFAULT,
                    &|| true,
                )
                .unwrap(),
            );
            let projection = project(
                pair,
                SourceDiff {
                    revision: if wrong_revision { 2 } else { 1 },
                    hunks: Vec::new(),
                },
                &|| true,
            )
            .unwrap();
            let (sender, inbox) = std::sync::mpsc::channel();
            sender.send(Some(Ok(projection))).unwrap();
            let mut state = MarkdownState {
                sources: Some(sources),
                revision: Some(1),
                pending: Some(Pending {
                    token: Arc::new(()),
                    revision: 1,
                    inbox,
                }),
                ..Default::default()
            };
            assert!(state.poll());
            assert!(state.projection().is_none());
            assert!(!state.loading());
            assert!(state.notice().is_some());
            let weak = Arc::downgrade(&state.sources.as_ref().unwrap()[0]);
            drop(doc);
            state.invalidate();
            assert!(weak.upgrade().is_none());
        }
    }
    #[test]
    fn markdown_state_source_cancels_pending_work() {
        let doc = Arc::new(TextDocument::new(
            "# a",
            "# b",
            std::path::Path::new("a.md"),
            std::path::Path::new("b.md"),
        ));
        let mut state = MarkdownState::new();
        let diff = super::super::source_diff(&doc, &doc.view(&[], 20.0), 1);
        state.execute(
            MarkdownCommand::SetMode(PresentationMode::Rendered),
            Some(doc.clone()),
            Some(diff),
        );
        let weak = Arc::downgrade(&state.pending.as_ref().unwrap().token);
        state.execute(
            MarkdownCommand::SetMode(PresentationMode::Source),
            Some(doc),
            None,
        );
        assert!(weak.upgrade().is_none());
        assert!(!state.loading());
        assert!(!state.poll());
        assert_eq!(state.mode(), PresentationMode::Source);
        assert_eq!(state.preferred_mode(), PresentationMode::Source);
    }
    #[test]
    fn markdown_state_duplicate_pending_request_keeps_identity() {
        let doc = Arc::new(TextDocument::new(
            "# a",
            "# b",
            std::path::Path::new("a.md"),
            std::path::Path::new("b.md"),
        ));
        let mut state = MarkdownState::new();
        let diff = || super::super::source_diff(&doc, &doc.view(&[], 20.0), 1);
        state.execute(
            MarkdownCommand::SetMode(PresentationMode::Rendered),
            Some(doc.clone()),
            Some(diff()),
        );
        let weak = Arc::downgrade(&state.pending.as_ref().unwrap().token);
        state.execute(
            MarkdownCommand::SetMode(PresentationMode::Rendered),
            Some(doc.clone()),
            Some(diff()),
        );
        assert!(weak.ptr_eq(&Arc::downgrade(&state.pending.as_ref().unwrap().token)));
        state.invalidate();
        assert!(weak.upgrade().is_none());
    }
}
