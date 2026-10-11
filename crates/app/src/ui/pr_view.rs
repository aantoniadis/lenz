//! Pull request review: a GitHub PR's Change Set (merge base ⟷ head), its commits in the log, personal
//! Viewed Marks, existing review threads (read-only) and refreshing. Nothing is written to GitHub.

use super::*;
use crate::pr::{Fetched, PullRequest, Thread, ViewStatus};

pub use crate::pr_session::open;
impl Viewer {
    /// Review a PR opened before the window (its Change Set already shown), through the shared session.
    pub fn adopt_pr(&mut self, found: PullRequest, fetched: Fetched, cx: &mut Context<Self>) {
        let Some(r) = self.repo.as_mut() else { return };
        self.spec = fetched.spec();
        r.showing(self.set.clone(), self.spec.clone());
        r.adopt_pr(found, fetched);
        self.present_repository(cx);
        self.poll_repository(cx);
    }

    /// A file's Viewed Mark.
    pub(super) fn view_status(&self, index: usize) -> Option<ViewStatus> {
        let s = self.repo.as_ref().and_then(|r| r.pr.as_ref())?;
        let c = self.set.comparisons.get(index)?;
        s.view_status(c)
    }

    /// `v`: mark the shown file viewed (or unmark it) and go to the next unviewed file.
    pub(super) fn toggle_viewed(&mut self, index: usize, jump: bool, cx: &mut Context<Self>) {
        self.repository_command(crate::repository_session::RepositoryCommand::Viewed { index, jump }, cx);
    }

    /// "k/N viewed".
    fn viewed_count(&self) -> (usize, usize) {
        let n = self.set.comparisons.len();
        ((0..n).filter(|&i| self.view_status(i) == Some(ViewStatus::Viewed)).count(), n,
        )
    }

    /// The review threads placed on `line` (0-based) of `side` in the shown file.
    pub(super) fn threads_at(&self, side: Side, line: u32) -> Option<Vec<Thread>> {
        let s = self.repo.as_ref().and_then(|r| r.pr.as_ref())?;
        let path = &self.set.comparisons.get(self.comparison.current)?.path;
        s.review.as_ref()?.placed.get(&(path.clone(), side, line + 1)).cloned()
    }

    /// Lines of the shown file with threads, per Side (0-based).
    pub(super) fn thread_lines(&self) -> [Vec<u32>; 2] {
        let mut out = [Vec::new(), Vec::new()];
        let (Some(s), Some(c)) = (self.repo.as_ref().and_then(|r| r.pr.as_ref()), self.set.comparisons.get(self.comparison.current),
        ) else {
            return out;
        };
        let Some(review) = s.review.as_ref() else {
            return out;
        };
        for (path, side, line) in review.placed.keys() {
            if *path == c.path {
                out[if *side == Side::Left { 0 } else { 1 }].push(line.saturating_sub(1));
            }
        }
        out
    }

    /// PR keys: `v` viewed, `r` refresh, `d` description, `t` this file's threads, `T` outdated threads; false when not one.
    pub(super) fn pr_key(&mut self, key: &str, shift: bool, cx: &mut Context<Self>) -> bool {
        let Some(s) = self.repo.as_ref().and_then(|r| r.pr.as_ref()) else { return false;
        };
        match (key, shift) {
            ("v", false) => self.toggle_viewed(self.current, true, cx),
            ("r", false) => self.repository_command(crate::repository_session::RepositoryCommand::RefreshPr, cx),
            // like any tab, from the Overview it enters Focus
            ("d", false) => self.pr_command(crate::pr_session::PrCommand::Show(crate::pr_session::DetailsTab::Conversation), cx),
            ("t", false) => {
                let path = self.set.comparisons.get(self.comparison.current).map(|c| c.path.clone()).unwrap_or_default();
                self.repository_command(crate::repository_session::RepositoryCommand::Threads { path, outdated: false }, cx);
            }
            ("t", true) => {
                let path = self.set.comparisons.get(self.comparison.current).map(|c| c.path.clone()).unwrap_or_default();
                if s.file_threads(&path, true).is_empty() {
                    self.toast("No outdated or file-level threads here".into(), cx);
                } else {
                    self.repository_command(crate::repository_session::RepositoryCommand::Threads { path, outdated: true }, cx);
                }
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    /// When the window comes back: one cheap check whether the PR moved on.
    pub fn check_pr_moved(&mut self, cx: &mut Context<Self>) {
        if self.repo.as_ref().is_some_and(|r| r.pr.is_some()) {
            self.repository_command(crate::repository_session::RepositoryCommand::CheckPrHead, cx);
        }
    }

    /// The PR line for the log header.
    pub(super) fn pr_status(&self) -> Option<String> {
        let s = self.repo.as_ref().and_then(|r| r.pr.as_ref())?;
        let (viewed, n) = self.viewed_count();
        Some(format!("{} · {viewed}/{n} viewed   (v viewed · d PR details · r refresh · T outdated threads)", s.summary()))
    }

}
