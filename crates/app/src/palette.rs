//! The command palette: a query over the catalog, the rows it keeps, the selection. Both frontends.
use std::borrow::Cow;
use crate::catalog::{Action, Availability, Entry};

pub struct Row { pub entry: Entry, pub disabled: Option<Cow<'static, str>> }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaletteCommand { Type(String), Backspace, Move(isize), Home, End, Select(usize) }

#[derive(Debug, PartialEq)]
pub enum Outcome { Nothing, Run(Action), Disabled(Cow<'static, str>) }

/// Every whitespace-separated word of `query` appears in `label` or `group`, ignoring case.
pub fn matches(label: &str, group: &str, query: &str) -> bool {
    let (label, group) = (label.to_lowercase(), group.to_lowercase());
    query.split_whitespace().all(|word| { let word = word.to_lowercase(); label.contains(&word) || group.contains(&word) })
}

pub struct Palette { all: Vec<Row>, query: String, hits: Vec<usize>, selected: usize, first: usize }

impl Palette {
    /// Rows for `entries`, each with its disabled reason as of now.
    pub fn new(entries: Vec<Entry>, availability: &dyn Availability) -> Self {
        let all: Vec<Row> = entries.into_iter().map(|entry| Row { disabled: availability.disabled(entry.action), entry }).collect();
        let hits = (0..all.len()).collect();
        Self { all, query: String::new(), hits, selected: 0, first: 0 }
    }
    pub fn query(&self) -> &str { &self.query }
    pub fn len(&self) -> usize { self.hits.len() }
    pub fn is_empty(&self) -> bool { self.hits.is_empty() }
    pub fn rows(&self) -> impl Iterator<Item = &Row> { self.hits.iter().map(|&i| &self.all[i]) }
    pub fn selected(&self) -> Option<usize> { (!self.hits.is_empty()).then_some(self.selected) }

    pub fn command(&mut self, command: PaletteCommand) {
        let last = self.hits.len().saturating_sub(1);
        match command {
            PaletteCommand::Type(text) => { self.query.push_str(&text); self.refilter(); }
            PaletteCommand::Backspace => { self.query.pop(); self.refilter(); }
            PaletteCommand::Move(delta) => self.selected = self.selected.saturating_add_signed(delta).min(last),
            PaletteCommand::Home => self.selected = 0,
            PaletteCommand::End => self.selected = last,
            PaletteCommand::Select(row) => if row < self.hits.len() { self.selected = row; },
        }
    }

    /// What Enter on the selection does.
    pub fn run(&self) -> Outcome {
        let Some(row) = self.hits.get(self.selected).map(|&i| &self.all[i]) else { return Outcome::Nothing };
        match &row.disabled { Some(reason) => Outcome::Disabled(reason.clone()), None => Outcome::Run(row.entry.action) }
    }

    /// The matching rows to draw in `visible` lines, scrolled so the selection shows.
    pub fn window(&mut self, visible: usize) -> std::ops::Range<usize> {
        if visible == 0 || self.hits.is_empty() { return 0..0; }
        if self.selected < self.first { self.first = self.selected; }
        if self.selected >= self.first + visible { self.first = self.selected + 1 - visible; }
        self.first = self.first.min(self.hits.len().saturating_sub(visible));
        self.first..(self.first + visible).min(self.hits.len())
    }

    fn refilter(&mut self) {
        let query = self.query.clone();
        self.hits = (0..self.all.len()).filter(|&i| matches(&self.all[i].entry.label, self.all[i].entry.group.label(), &query)).collect();
        (self.selected, self.first) = (0, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{self, Action, Scope};
    use crate::commands::{Command, Interaction};
    struct None_;
    impl Availability for None_ { fn disabled(&self, _: Action) -> Option<Cow<'static, str>> { None } }
    struct NoSelection;
    impl Availability for NoSelection {
        fn disabled(&self, a: Action) -> Option<Cow<'static, str>> { (a == Action::Interaction(Interaction::Copy)).then(|| "No selected text".into()) }
    }
    fn palette(a: &dyn Availability) -> Palette { Palette::new(catalog::entries(&Scope::comparison()), a) }

    #[test]
    fn words_match_label_or_group_in_any_case() {
        assert!(matches("Whitespace: Trim", "Diff", "white trim"));
        assert!(matches("Next change", "Navigate", "NAV next"));
        assert!(!matches("Next change", "Navigate", "trim"));
        assert!(matches("anything", "Group", ""));
    }

    #[test]
    fn typing_filters_and_selects_the_first_match() {
        let mut p = palette(&None_);
        p.command(PaletteCommand::Type("whitesp".into()));
        p.command(PaletteCommand::Type("ace: ig".into()));
        assert!(p.rows().all(|r| r.entry.label.to_lowercase().contains("whitespace: ig")));
        assert_eq!(p.selected(), Some(0));
        assert!(matches!(p.run(), Outcome::Run(Action::Comparison(Command::SetWhitespace(_)))));
        p.command(PaletteCommand::Backspace);
        assert_eq!(p.query(), "whitespace: i");
    }

    #[test]
    fn no_matches_runs_nothing() {
        let mut p = palette(&None_);
        p.command(PaletteCommand::Type("zzzz-no-such-command".into()));
        assert_eq!(p.len(), 0);
        assert_eq!(p.selected(), None);
        assert!(matches!(p.run(), Outcome::Nothing));
        p.command(PaletteCommand::Move(3));
        p.command(PaletteCommand::End);
        assert!(matches!(p.run(), Outcome::Nothing));
    }

    #[test]
    fn disabled_rows_stay_listed_and_report_their_reason() {
        let mut p = palette(&NoSelection);
        p.command(PaletteCommand::Type("copy selection".into()));
        assert_eq!(p.rows().next().unwrap().disabled.as_deref(), Some("No selected text"));
        assert!(matches!(p.run(), Outcome::Disabled(reason) if reason == "No selected text"));
    }

    #[test]
    fn selection_clamps_and_the_window_follows_it() {
        let mut p = palette(&None_);
        let n = p.len();
        p.command(PaletteCommand::End);
        assert_eq!(p.selected(), Some(n - 1));
        assert_eq!(p.window(5), n - 5..n);
        p.command(PaletteCommand::Move(-1));
        assert_eq!(p.window(5), n - 5..n, "Up from the end keeps the window");
        p.command(PaletteCommand::Home);
        assert_eq!(p.window(5), 0..5);
        p.command(PaletteCommand::Move(-4));
        assert_eq!(p.selected(), Some(0));
        p.command(PaletteCommand::Select(n + 10));
        assert_eq!(p.selected(), Some(0), "an out-of-range click changes nothing");
        assert_eq!(p.window(0), 0..0);
    }
}
