//! Explorer speed search only visits the cached, visible rows; it never opens a candidate while typing.
use super::*;
use lenz_content::find::{Matcher, FindOptions, Query};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExplorerSearchInput {
    Text(String),
    Backspace,
    Step { forward: bool },
    Clear,
    Activate,
}

#[derive(Default)]
pub struct ExplorerSearch {
    query: String,
    matcher: Option<Matcher>,
    original: Option<Arc<str>>,
    matched: usize,
}

impl ExplorerSearch {
    pub fn query(&self) -> &str { &self.query }
    pub fn matched(&self) -> usize { self.matched }
    pub fn label(&self) -> Option<String> {
        (!self.query.is_empty()).then(|| format!("Search: {} — {} matches", self.query, self.matched))
    }
    pub fn ranges<'a>(&'a self, name: &'a str) -> impl Iterator<Item = std::ops::Range<usize>> + 'a {
        self.matcher.iter().flat_map(move |matcher| matcher.ranges(name))
    }
    fn matches(&self, row: &project::TreeRow) -> bool { self.ranges(&row.name).next().is_some() }
}

impl Session {
    pub(super) fn explorer_search(&mut self, input: ExplorerSearchInput) -> Effect {
        let repaint = Effect { repaint: true, ..Effect::default() };
        let selected = self.state.explorer.selected.as_deref().map(Arc::<str>::from);
        let step = match &input { ExplorerSearchInput::Step { forward } => Some(*forward), _ => None };
        match input {
            ExplorerSearchInput::Clear => {
                let original = self.state.explorer_search.original.take();
                self.state.explorer_search = Default::default();
                if let Some(path) = original.filter(|p| self.state.tree_rows().any(|r| r.path == *p)) { self.show_created(path); }
                return repaint;
            }
            ExplorerSearchInput::Activate => {
                let candidate = selected.filter(|p| self.state.tree_rows().any(|r| r.path == *p && self.state.explorer_search.matches(r)));
                self.state.explorer_search = Default::default();
                return candidate.map_or(repaint, |path| self.activate_tree(path, false));
            }
            ExplorerSearchInput::Text(text) => {
                if self.state.explorer_search.query.is_empty() { self.state.explorer_search.original = selected.clone(); }
                // A speed-search prompt stays small even if an input method commits a large string.
                for c in text.chars().filter(|c| !c.is_control()) {
                    if self.state.explorer_search.query.len() + c.len_utf8() > 256 { break; }
                    self.state.explorer_search.query.push(c);
                }
            }
            ExplorerSearchInput::Backspace => {
                self.state.explorer_search.query.pop();
                if self.state.explorer_search.query.is_empty() { return self.explorer_search(ExplorerSearchInput::Clear); }
            }
            ExplorerSearchInput::Step { .. } => {}
        }
        if self.state.explorer_search.query.is_empty() { return repaint; }
        if step.is_none() {
            self.state.explorer_search.matcher = Matcher::new(&Query { text: self.state.explorer_search.query.clone(), options: FindOptions::default() }).ok();
        }
        let search = &self.state.explorer_search;
        let rows = self.state.tree_rows();
        let len = rows.clone().count();
        let at = selected.as_ref().and_then(|p| rows.clone().position(|r| r.path == *p));
        let matched = rows.clone().filter(|r| search.matches(r)).count();
        let candidate = match step {
            Some(true) => {
                let start = at.map_or(0, |i| i + 1);
                rows.clone().skip(start).chain(rows.clone().take(start)).find(|r| search.matches(r))
            }
            Some(false) => {
                let start = at.unwrap_or(len);
                rows.clone().rev().skip(len - start).chain(rows.clone().rev().take(len - start)).find(|r| search.matches(r))
            }
            None => selected.as_ref().and_then(|p| rows.clone().find(|r| r.path == *p && search.matches(r))).or_else(|| rows.clone().find(|r| search.matches(r))),
        }.map(|r| r.path.clone());
        drop(rows);
        self.state.explorer_search.matched = matched;
        if let Some(path) = candidate { self.show_created(path); }
        repaint
    }
}
