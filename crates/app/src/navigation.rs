//! Back and Forward: the places a repository view has been, so a step (another commit, file history, a branch) can be
//! undone and redone. Both frontends.
use crate::history::Branches;

/// What the log selected.
#[derive(Clone, Debug, PartialEq)]
pub enum Picked {
    /// Uncommitted changes.
    Local,
    /// A commit, or two (`second`), or the range between them.
    Commits { anchor: String, second: Option<String>, range: bool },
}

/// Where the view is: what the log shows, what it selected, the file and line on screen.
#[derive(Clone, Debug, PartialEq)]
pub struct Spot {
    pub branches: Branches,
    pub filter: String,
    pub file_history: Option<String>,
    pub picked: Option<Picked>,
    pub file: Option<String>,
    pub line: Option<u32>,
}

impl Spot {
    /// The same place for Back/Forward: the file and line move freely within it.
    pub fn same_place(&self, other: &Spot) -> bool {
        (&self.branches, &self.filter, &self.file_history, &self.picked) == (&other.branches, &other.filter, &other.file_history, &other.picked)
    }
}

const LIMIT: usize = 100;

#[derive(Debug, Default)]
pub struct History { back: Vec<Spot>, forward: Vec<Spot> }

impl History {
    /// `from` is being left by a step: Back returns to it, and Forward is forgotten.
    pub fn record(&mut self, from: Spot) {
        if !self.back.last().is_some_and(|top| top.same_place(&from)) {
            self.back.push(from);
            if self.back.len() > LIMIT { self.back.remove(0); }
        }
        self.forward.clear();
    }
    pub fn can_back(&self) -> bool { !self.back.is_empty() }
    pub fn can_forward(&self) -> bool { !self.forward.is_empty() }
    /// The place before `here`, which Forward then returns to.
    pub fn back(&mut self, here: Spot) -> Option<Spot> { Self::step(&mut self.back, &mut self.forward, here) }
    /// The place Back left, which Back then returns from.
    pub fn forward(&mut self, here: Spot) -> Option<Spot> { Self::step(&mut self.forward, &mut self.back, here) }

    fn step(from: &mut Vec<Spot>, to: &mut Vec<Spot>, here: Spot) -> Option<Spot> {
        let target = from.pop()?;
        to.push(here);
        if to.len() > LIMIT { to.remove(0); }
        Some(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(commit: &str) -> Spot {
        Spot { branches: Branches::All, filter: String::new(), file_history: None, picked: Some(Picked::Commits { anchor: commit.into(), second: None, range: false }), file: None, line: None }
    }

    #[test]
    fn back_and_forward_walk_the_recorded_places() {
        let mut h = History::default();
        assert!(!h.can_back() && !h.can_forward());
        h.record(at("a"));
        h.record(at("b"));
        assert_eq!(h.back(at("c")), Some(at("b")));
        assert_eq!(h.back(at("b")), Some(at("a")));
        assert_eq!(h.back(at("a")), None, "nothing before the first place");
        assert_eq!(h.forward(at("a")), Some(at("b")));
        assert_eq!(h.forward(at("b")), Some(at("c")));
        assert!(!h.can_forward());
    }

    #[test]
    fn a_new_step_forgets_forward_and_repeats_are_kept_once() {
        let mut h = History::default();
        h.record(at("a"));
        h.record(Spot { line: Some(9), ..at("a") });
        assert_eq!(h.back(at("b")), Some(at("a")), "the same place at another line is one step");
        assert!(h.can_forward());
        h.record(at("a"));
        assert!(!h.can_forward(), "a new step after Back forgets Forward");
    }

    #[test]
    fn the_history_keeps_the_last_hundred_places() {
        let mut h = History::default();
        for n in 0..150 { h.record(at(&n.to_string())); }
        let mut count = 0;
        let mut here = at("now");
        while let Some(previous) = h.back(here.clone()) { count += 1; here = previous; }
        assert_eq!(count, LIMIT);
        assert_eq!(here, at("50"), "the oldest kept is the 51st recorded");
    }
}
