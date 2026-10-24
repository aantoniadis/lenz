//! Workspace replacements reuse the same source-fenced decisions as closure.
use super::*;
use crate::{close::Intent, text::BufferId};

pub(super) enum Transition {
    Reload(TabId),
    Root(PathBuf),
    Checkout(crate::branches::Retry, lenz_content::branches::Mode),
}
impl Session {
    pub(super) fn prepare_transition(&mut self, mut next: Transition) -> Effect {
        if self.closed || self.has_close_intent() {
            return Effect {
                repaint: true,
                copy: None,
            };
        }
        if self.state.comparison.is_some() {
            let reload_buffer = match &next {
                Transition::Reload(tab) => self
                    .state
                    .comparison
                    .as_ref()
                    .filter(|c| c.saved_tab() == *tab || c.current_tab() == *tab)
                    .and_then(|c| self.state.source(c.current_tab()))
                    .map(|s| s.key().buffer),
                Transition::Root(_) | Transition::Checkout(..) => None,
            };
            self.close_saved_comparison();
            if let Some(buffer) = reload_buffer {
                let ordinary = self
                    .state
                    .active()
                    .filter(|t| t.buffer == Some(buffer))
                    .or_else(|| self.state.tabs.iter().find(|t| t.buffer == Some(buffer)));
                let Some(tab) = ordinary else {
                    return Effect {
                        repaint: true,
                        copy: None,
                    };
                };
                next = Transition::Reload(tab.id);
            }
        }
        let intent = match &next {
            Transition::Reload(tab) => Intent::Reload(*tab),
            Transition::Root(_) | Transition::Checkout(..) => Intent::Workspace,
        };
        self.prepare_close_with(intent, None, Some(next))
    }
    pub(super) fn commit_transition(
        &mut self,
        next: Transition,
        sources: Vec<(BufferId, SemanticSource)>,
    ) -> Effect {
        match next {
            Transition::Reload(tab) => {
                if let Some(blame) = self.tab_mut(tab).and_then(|t| t.blame.as_mut()) {
                    blame.stamp = None;
                }
                // The read may fail or an edit may supersede it. Consent is consumed only by a matched successful reply.
                self.load(tab, true);
            }
            Transition::Root(root) => {
                self.state.tree_marks.clear();
                self.open_root(root);
                for (buffer, _) in sources {
                    if !self.state.tabs.iter().any(|t| t.buffer == Some(buffer)) {
                        if let Some(entry) = self.state.sources.buffers.remove(&buffer)
                            && let Some(source) = entry.snapshot
                        {
                            self.cache.take_any(source.source());
                        }
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
        Effect {
            repaint: true,
            copy: None,
        }
    }
    pub(super) fn replacement_current(&self, buffer: BufferId) -> bool {
        self.replacements.get(&buffer).is_some_and(|source| {
            self.state
                .sources
                .buffers
                .get(&buffer)
                .is_some_and(|entry| {
                    !entry.save.status.busy()
                        && entry
                            .snapshot
                            .as_ref()
                            .is_some_and(|now| source.matches(now))
                })
        })
    }
    pub(super) fn finish_checkout_sources(&mut self, here: bool) {
        for (buffer, source) in std::mem::take(&mut self.checkout_sources) {
            if here
                && self
                    .state
                    .sources
                    .buffers
                    .get(&buffer)
                    .is_some_and(|entry| {
                        entry
                            .snapshot
                            .as_ref()
                            .is_some_and(|now| source.matches(now))
                    })
            {
                self.replacements.insert(buffer, source);
            }
        }
    }
}
