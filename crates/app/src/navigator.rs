//! File Navigator model (gpui-free): directory-grouped rows, fuzzy file matching, `name:line` queries.

use crate::changeset::{Comparison, Status};

/// Logical input panes; adapters supply only the panes their current layout exposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputPane {
    Sources, Log, Files, Left, Middle, Right, Results,
}

/// Cycle in layout order without allocating or stopping at a hidden pane.
pub fn cycle_pane(current: InputPane, panes: &[Option<InputPane>], backwards: bool) -> Option<InputPane> {
    let count = panes.len();
    let from = panes.iter().position(|pane| *pane == Some(current))
        .unwrap_or(if backwards { 0 } else { count.saturating_sub(1) });
    for step in 1..=count {
        let index = if backwards { (from + count - step) % count } else { (from + step) % count };
        if let Some(pane) = panes[index] { return Some(pane); }
    }
    None
}

/// A row of the File Navigator tree.
#[derive(Debug, Clone, PartialEq)]
pub enum NavRow {
    /// A group heading (Staged, Unstaged, …) and its file count.
    Group(String, usize),
    /// A directory heading (`"."` for the root).
    Dir(String),
    /// Comparison index, its file name, and the old path when a rename changed directory or name.
    File { index: usize, name: String, from: Option<String> },
}

fn split(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

/// Rows grouped by directory; Comparisons arrive sorted by directory, then file name.
pub fn rows(comparisons: &[Comparison]) -> Vec<NavRow> {
    rows_of(comparisons, 0..comparisons.len())
}

/// Rows under group headings (a Change Set's `groups`), directories restarting in each; plain rows when ungrouped.
pub fn grouped_rows(comparisons: &[Comparison], groups: &[(String, std::ops::Range<usize>)]) -> Vec<NavRow> {
    if groups.is_empty() {
        return rows(comparisons);
    }
    let mut out = Vec::new();
    for (name, range) in groups {
        out.push(NavRow::Group(name.clone(), range.len()));
        out.extend(rows_of(comparisons, range.clone()));
    }
    out
}

fn rows_of(comparisons: &[Comparison], range: std::ops::Range<usize>) -> Vec<NavRow> {
    let mut out = Vec::new();
    let mut last_dir: Option<&str> = None;
    for index in range {
        let c = &comparisons[index];
        let (dir, name) = split(&c.path);
        if last_dir != Some(dir) {
            out.push(NavRow::Dir(if dir.is_empty() { ".".into() } else { dir.into() }));
            last_dir = Some(dir);
        }
        let from = matches!(c.status, Status::Renamed { .. }).then(|| c.old_path.clone());
        out.push(NavRow::File { index, name: name.into(), from });
    }
    out
}

/// One-letter status badge.
pub fn badge(status: &Status) -> &'static str {
    match status {
        Status::Added | Status::Untracked => "A",
        Status::Deleted => "D",
        Status::Modified | Status::ModeChanged { .. } => "M",
        Status::Renamed { .. } => "R",
        Status::Conflicted => "C",
    }
}

pub use lenz_content::fuzzy::{rank, score};

/// Split a switcher query like `driver:120` into the file part and a 1-based line.
pub fn query_line(query: &str) -> (&str, Option<u32>) {
    match query.rsplit_once(':') {
        Some((file, line)) if !line.is_empty() && line.chars().all(|c| c.is_ascii_digit()) => (file, line.parse().ok()),
        _ => (query, None),
    }
}

/// IntelliJ's "press again to go to the next file" at the edge of a file's Change Blocks.
#[derive(Debug, Default)]
pub struct PressAgain {
    pending: Option<bool>,
}

impl PressAgain {
    /// Next/previous change hit the edge of the file: the first press arms, a second press in
    /// the same direction crosses into the next/previous file.
    pub fn at_edge(&mut self, forward: bool) -> bool {
        if self.pending == Some(forward) {
            self.pending = None;
            true
        } else {
            self.pending = Some(forward);
            false
        }
    }

    /// Any other action disarms it.
    pub fn reset(&mut self) {
        self.pending = None;
    }
}

/// Notice for Comparisons with nothing to diff.
pub fn notice(status: &Status, identical: bool) -> Option<String> {
    match status {
        Status::Renamed { .. } if identical => Some("Renamed — content identical".into()),
        Status::ModeChanged { old, new } => Some(format!("File mode {:o} → {:o}", old & 0o777, new & 0o777)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changeset::Content;

    #[test]
    fn pane_cycle_wraps_skips_hidden_panes_and_recovers_stale_focus() {
        use InputPane::*;
        let panes = [None, None, Some(Files), Some(Left), None, Some(Right), None];
        for (current, backwards, expected) in [(Left, true, Files), (Files, false, Left),
            (Left, false, Right), (Right, false, Files), (Files, true, Right),
            (Log, false, Files), (Sources, true, Right)] {
            assert_eq!(cycle_pane(current, &panes, backwards), Some(expected));
        }
        assert_eq!(cycle_pane(Left, &[], false), None);
        assert_eq!(cycle_pane(Left, &[None, None], true), None);
        assert_eq!(cycle_pane(Left, &[Some(Left)], true), Some(Left));
    }

    fn cmp(old: &str, path: &str, status: Status) -> Comparison {
        Comparison { old_path: old.into(), path: path.into(), status, left: Content::Empty, right: Content::Empty }
    }

    #[test]
    fn files_are_grouped_under_their_directory() {
        let set = [
            cmp("README.md", "README.md", Status::Modified),
            cmp("src/a.rs", "src/a.rs", Status::Added),
            cmp("old/b.rs", "src/b.rs", Status::Renamed { similarity: 90 }),
            cmp("src/io/c.rs", "src/io/c.rs", Status::Deleted),
        ];
        assert_eq!(
            rows(&set),
            vec![
                NavRow::Dir(".".into()),
                NavRow::File { index: 0, name: "README.md".into(), from: None },
                NavRow::Dir("src".into()),
                NavRow::File { index: 1, name: "a.rs".into(), from: None },
                NavRow::File { index: 2, name: "b.rs".into(), from: Some("old/b.rs".into()) },
                NavRow::Dir("src/io".into()),
                NavRow::File { index: 3, name: "c.rs".into(), from: None },
            ]
        );
    }

    #[test]
    fn groups_head_their_files_and_directories_restart_in_each() {
        let set = [
            cmp("src/a.rs", "src/a.rs", Status::Modified),
            cmp("src/a.rs", "src/a.rs", Status::Modified),
            cmp("new.txt", "new.txt", Status::Untracked),
        ];
        let groups = [("Staged".to_string(), 0..1), ("Unstaged".to_string(), 1..2), ("Untracked".to_string(), 2..3)];
        assert_eq!(
            grouped_rows(&set, &groups),
            vec![
                NavRow::Group("Staged".into(), 1),
                NavRow::Dir("src".into()),
                NavRow::File { index: 0, name: "a.rs".into(), from: None },
                NavRow::Group("Unstaged".into(), 1),
                NavRow::Dir("src".into()),
                NavRow::File { index: 1, name: "a.rs".into(), from: None },
                NavRow::Group("Untracked".into(), 1),
                NavRow::Dir(".".into()),
                NavRow::File { index: 2, name: "new.txt".into(), from: None },
            ]
        );
        assert_eq!(grouped_rows(&set, &[]), rows(&set));
    }

    #[test]
    fn file_name_matches_rank_above_path_matches_and_scattered_letters() {
        let paths = ["src/fs/mocks.rs", "src/net/tcp/socket.rs", "socks/readme.md", "src/sync/rwlock.rs"];
        // "rwlock" holds s-o-c-k too, as scattered letters, so it ranks last
        assert_eq!(rank("sock", &paths), vec![1, 2, 0, 3]);
        assert!(score("xyz", "src/lib.rs").is_none());
        assert_eq!(rank("", &paths), vec![0, 1, 2, 3]);
    }

    #[test]
    fn a_file_name_starting_with_the_query_beats_a_shorter_path_containing_it() {
        let paths = ["tests/io_driver.rs", "src/runtime/io/driver.rs"];
        assert_eq!(rank("driver", &paths), vec![1, 0]);
    }

    #[test]
    fn a_trailing_line_number_is_split_off_the_query() {
        assert_eq!(query_line("driver:120"), ("driver", Some(120)));
        assert_eq!(query_line("driver"), ("driver", None));
        assert_eq!(query_line("c:/odd"), ("c:/odd", None));
        assert_eq!(query_line("driver:"), ("driver:", None));
    }

    #[test]
    fn crossing_into_the_next_file_takes_a_second_press_in_the_same_direction() {
        let mut p = PressAgain::default();
        assert!(!p.at_edge(true));
        assert!(p.at_edge(true));
        assert!(!p.at_edge(true)); // crossing disarms it again
        assert!(!p.at_edge(false)); // a press the other way re-arms for that direction
        p.reset();
        assert!(!p.at_edge(false));
        assert!(p.at_edge(false));
    }

    #[test]
    fn identical_renames_and_mode_changes_get_a_notice() {
        assert_eq!(notice(&Status::Renamed { similarity: 100 }, true).as_deref(), Some("Renamed — content identical"));
        assert_eq!(notice(&Status::Renamed { similarity: 80 }, false), None);
        assert_eq!(
            notice(&Status::ModeChanged { old: 0o100644, new: 0o100755 }, true).as_deref(),
            Some("File mode 644 → 755")
        );
        assert_eq!(notice(&Status::Modified, false), None);
    }
}
