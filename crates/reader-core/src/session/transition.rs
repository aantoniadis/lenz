//! Workspace replacements reuse the same source-fenced decisions as closure.
use super::*;
use crate::{close::Intent, text::BufferId};

pub(super) enum Transition {
    Reload(TabId),
    Root(PathBuf),
    Checkout(crate::branches::Retry, lenz_content::branches::Mode),
}
impl Session {
    pub(super) fn prepare_transition(&mut self, next: Transition) -> Effect {
        let intent = match &next {
            Transition::Reload(tab) => Intent::Reload(*tab),
            Transition::Root(_) | Transition::Checkout(..) => Intent::Workspace,
        };
        self.prepare_close_with(intent, None, Some(next))
    }
    pub(super) fn commit_transition(&mut self, next: Transition, sources: Vec<(BufferId, SemanticSource)>) -> Effect {
        match next {
            Transition::Reload(tab) => {
                if let Some(blame) = self.tab_mut(tab).and_then(|t| t.blame.as_mut()) { blame.stamp = None; }
                // The read may fail or an edit may supersede it. Consent is consumed only by a matched successful reply.
                self.load(tab, true);
            }
            Transition::Root(root) => {
                self.state.tree_marks.clear();
                self.open_root(root);
                for (buffer, _) in sources {
                    if !self.state.tabs.iter().any(|t| t.buffer == Some(buffer)) {
                        if let Some(entry) = self.state.sources.buffers.remove(&buffer) && let Some(source) = entry.snapshot { self.cache.take_any(source.source()); }
                        self.state.sources.aliases.retain(|_, id| *id != buffer);
                    }
                }
                self.release_current();
            }
            Transition::Checkout(retry, mode) => {
                self.checkout_sources = sources;
                return self.retry(retry, mode);
            }
        }
        Effect { repaint: true, copy: None }
    }
    pub(super) fn finish_checkout_sources(&mut self, here: bool) {
        for (buffer, source) in std::mem::take(&mut self.checkout_sources) {
            if !here { continue; }
            if let Some(entry) = self.state.sources.buffers.get_mut(&buffer)
                && entry.snapshot.as_ref().is_some_and(|now| source.matches(now)) {
                entry.draft = None;
            }
        }
    }
}
