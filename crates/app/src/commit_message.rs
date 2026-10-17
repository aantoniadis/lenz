//! The commit message panel in Focus: one commit's full message, or a range's commits that open one at a time.
//! Both frontends draw [`Messages::lines`]; the repository session loads and owns the state.
use crate::changeset::Spec;
use crate::selection_title::EMPTY_TREE;
use std::path::Path;

/// Most commits a range lists.
// ponytail: a long range shows its newest commits only; page through `git log --skip` if anyone needs the rest.
pub const RANGE_LIMIT: usize = 200;
/// Most bytes of one commit message the panel keeps: both frontends wrap the panel while drawing, so a pasted
/// log in a message must not cost its full size on every frame.
const MESSAGE_BYTES: usize = 16 * 1024;

/// What the panel reads for a comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Query {
    One(String),
    /// A `git log` range, `a..b`.
    Range(String),
}

/// The commits a comparison shows, or `None` when it has no message (working tree, index, plain files).
pub fn query(spec: &Spec) -> Option<Query> {
    let Spec::Git { cached: false, revs, .. } = spec else { return None };
    match revs.as_slice() {
        // the diff runs from the merge base, so the log lists the right side's own commits
        [only] => only.split_once("...").map(|(a, b)| Query::Range(format!("{a}..{b}"))),
        [left, right] => Some(if left == EMPTY_TREE || left.strip_prefix(right.as_str()).is_some_and(|p| p.starts_with('^')) {
            Query::One(right.clone())
        } else {
            Query::Range(format!("{left}..{right}"))
        }),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit { pub sha: String, pub author: String, pub date: String, pub message: String }

impl Commit {
    pub fn subject(&self) -> &str { self.message.lines().next().unwrap_or("") }
    fn meta(&self) -> String { format!("{} · {} · {}", self.author, self.date, self.sha.get(..7).unwrap_or(&self.sha)) }
    /// The message after the subject, without the blank line between them.
    fn body(&self) -> impl Iterator<Item = &str> {
        let mut lines = self.message.trim_end().lines().skip(1).skip_while(|l| l.trim().is_empty()).peekable();
        std::iter::from_fn(move || lines.next())
    }
}

/// The panel's content: one commit, or a range whose commits open one at a time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Messages { pub query: Query, pub commits: Vec<Commit>, pub open: Vec<bool>, pub more: bool }

pub fn read(root: &Path, query: &Query) -> Result<Messages, String> {
    let limit = format!("-n{}", RANGE_LIMIT + 1);
    let rev = match query { Query::One(sha) => sha, Query::Range(range) => range };
    let out = crate::git::run(root, &["log", "--no-show-signature", "--no-notes", "--date=format:%Y-%m-%d %H:%M",
        "--format=%H%x00%aN%x00%ad%x00%B%x1e", if matches!(query, Query::One(_)) { "-n1" } else { &limit }, "--end-of-options", rev, "--"])?;
    let mut commits: Vec<Commit> = String::from_utf8_lossy(&out).split('\x1e').filter_map(|record| {
        let mut f = record.trim_start_matches('\n').splitn(4, '\0');
        Some(Commit { sha: f.next()?.into(), author: f.next()?.into(), date: f.next()?.into(), message: f.next()?.into() })
    }).collect();
    let more = commits.len() > RANGE_LIMIT;
    commits.truncate(RANGE_LIMIT);
    for c in &mut commits {
        if c.message.len() > MESSAGE_BYTES {
            let mut end = MESSAGE_BYTES;
            while !c.message.is_char_boundary(end) { end -= 1; }
            c.message.truncate(end);
            c.message.push_str("\n…");
        }
    }
    Ok(Messages { query: query.clone(), open: vec![false; commits.len()], commits, more })
}

/// What a panel line is for: clicking a `Toggle` collapses the panel, a `Commit` opens or closes that commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind { Toggle, Commit(usize), Meta, Body }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line { pub text: String, pub kind: LineKind }

impl Messages {
    /// The panel's lines wrapped to `columns`; `collapsed` leaves the first line only.
    pub fn lines(&self, collapsed: bool, columns: usize) -> Vec<Line> {
        let mut out = Vec::new();
        let mut push = |text: &str, indent: usize, kind: LineKind| wrap(&mut out, text, indent, columns, kind);
        let arrow = if collapsed { "▸" } else { "▾" };
        if let (Query::One(_), Some(c)) = (&self.query, self.commits.first()) {
            push(&format!("{arrow} {}", c.subject()), 2, LineKind::Toggle);
            if collapsed { return out; }
            push(&c.meta(), 2, LineKind::Meta);
            for line in c.body() { push(line, 0, LineKind::Body); }
            return out;
        }
        let count = match self.commits.len() { 1 => "1 commit".into(), n if self.more => format!("Newest {n} commits"), n => format!("{n} commits") };
        push(&format!("{arrow} {count}"), 2, LineKind::Toggle);
        if collapsed { return out; }
        for (i, c) in self.commits.iter().enumerate() {
            let open = self.open.get(i).copied().unwrap_or(false);
            push(&format!("  {} {}", if open { "▾" } else { "▸" }, c.subject()), 4, LineKind::Commit(i));
            if open {
                push(&format!("    {}", c.meta()), 4, LineKind::Meta);
                for line in c.body() { push(&format!("    {line}"), 4, LineKind::Body); }
            }
        }
        out
    }
}

/// `text` in rows of at most `columns` characters, continuation rows indented by `indent`; blank lines stay.
fn wrap(out: &mut Vec<Line>, text: &str, indent: usize, columns: usize, kind: LineKind) {
    let columns = columns.max(indent + 8);
    let chars: Vec<char> = text.chars().collect();
    let mut at = 0;
    loop {
        let room = if at == 0 { columns } else { columns - indent };
        let mut end = (at + room).min(chars.len());
        // break at the last space unless the row ends at a word's end
        if end < chars.len() && chars[end] != ' ' && let Some(space) = chars[at..end].iter().rposition(|c| *c == ' ').filter(|s| *s > 0) { end = at + space; }
        let piece: String = chars[at..end].iter().collect();
        let piece = piece.trim_end();
        out.push(Line { text: if at == 0 { piece.to_string() } else { format!("{}{piece}", " ".repeat(indent)) }, kind });
        at = end;
        while chars.get(at) == Some(&' ') { at += 1; }
        if at >= chars.len() { break; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testing::{commit_all, git, repo, write};
    use crate::repository_session::branch_comparison;

    fn head(root: &Path, rev: &str) -> String { git(root, &["rev-parse", rev]).trim().to_string() }

    #[test]
    fn a_huge_message_is_cut_for_the_panel() {
        let dir = repo();
        write(dir.path(), "a", "a");
        commit_all(dir.path(), &format!("subject\n\n{}", "é".repeat(MESSAGE_BYTES)));
        let m = read(dir.path(), &Query::One(head(dir.path(), "HEAD"))).unwrap();
        assert!(m.commits[0].message.len() <= MESSAGE_BYTES + 4 && m.commits[0].message.ends_with('…'));
    }

    #[test]
    fn queries_follow_the_comparison() {
        let one = branch_comparison("abc1234^1", "abc1234", false);
        assert_eq!(query(&one), Some(Query::One("abc1234".into())));
        assert_eq!(query(&branch_comparison("abc1234^2", "abc1234", false)), Some(Query::One("abc1234".into())));
        assert_eq!(query(&branch_comparison(EMPTY_TREE, "abc1234", false)), Some(Query::One("abc1234".into())));
        assert_eq!(query(&branch_comparison("old^1", "new", false)), Some(Query::Range("old^1..new".into())));
        assert_eq!(query(&branch_comparison("main", "topic", true)), Some(Query::Range("main..topic".into())), "commits since the merge base");
        assert_eq!(query(&crate::repository_session::local_spec()), None);
        assert_eq!(query(&crate::repository_session::index_spec()), None);
        assert_eq!(query(&Spec::Git { cached: false, revs: vec!["abc1234".into()], paths: vec![], untracked: false }), None, "a commit against the working tree");
    }

    #[test]
    fn reads_a_full_message_and_a_range() {
        let root = repo();
        write(root.path(), "f", "a\n"); commit_all(root.path(), "first");
        write(root.path(), "f", "b\n"); git(root.path(), &["commit", "-qam", "second subject\n\nbody line one\n\nbody line three"]);
        write(root.path(), "f", "c\n"); commit_all(root.path(), "third");
        let second = head(root.path(), "HEAD~1");
        let one = read(root.path(), &Query::One(second.clone())).unwrap();
        assert_eq!(one.commits.len(), 1);
        let lines: Vec<String> = one.lines(false, 80).into_iter().map(|l| l.text).collect();
        assert_eq!(lines[0], "▾ second subject");
        assert!(lines[1].contains(&second[..7]), "author · date · sha: {lines:?}");
        assert_eq!(&lines[2..], ["body line one", "", "body line three"]);
        assert_eq!(one.lines(true, 80).len(), 1, "collapsed keeps the subject");

        let mut range = read(root.path(), &Query::Range("HEAD~2..HEAD".into())).unwrap();
        let subjects: Vec<&str> = range.commits.iter().map(Commit::subject).collect();
        assert_eq!(subjects, ["third", "second subject"], "newest first");
        assert_eq!(range.lines(false, 80).iter().map(|l| l.kind).collect::<Vec<_>>(), [LineKind::Toggle, LineKind::Commit(0), LineKind::Commit(1)]);
        range.open[1] = true;
        let texts: Vec<String> = range.lines(false, 80).into_iter().map(|l| l.text).collect();
        assert_eq!(texts[3..].iter().map(|t| t.trim()).collect::<Vec<_>>()[1..], ["body line one", "", "body line three"]);
    }

    #[test]
    fn long_lines_wrap_at_spaces_with_an_indent() {
        let mut out = Vec::new();
        wrap(&mut out, "▾ alpha beta gamma delta", 2, 12, LineKind::Toggle);
        let texts: Vec<&str> = out.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["▾ alpha beta", "  gamma", "  delta"]);
        assert!(out.iter().all(|l| l.text.chars().count() <= 12));
    }

    fn settle(r: &mut crate::repository_session::RepositorySession) {
        let started = std::time::Instant::now();
        while r.busy() { r.poll(); assert!(started.elapsed().as_secs() < 10); std::thread::sleep(std::time::Duration::from_millis(2)); }
    }

    #[test]
    fn the_session_reads_the_message_for_the_comparison_on_screen() {
        use crate::repository_session::{MessageKey, MessagePanel, RepositoryCommand as R, RepositorySession};
        let root = repo();
        for (n, text) in ["one", "two", "three"].iter().enumerate() { write(root.path(), "f", format!("{n}\n")); commit_all(root.path(), text); }
        let request = crate::cli::parse(&["log".into()], root.path()).unwrap();
        let mut r = RepositorySession::open(&request, root.path().into());
        settle(&mut r);
        let show = |r: &mut RepositorySession, spec: Spec| {
            let set = crate::changeset::ChangeSet::load(&spec, root.path()).unwrap();
            r.showing(std::sync::Arc::new(set), spec);
        };
        let (two, three) = (head(root.path(), "HEAD~1"), head(root.path(), "HEAD"));
        // a read for an earlier comparison never replaces the later one
        show(&mut r, branch_comparison(&format!("{two}^1"), &two, false));
        show(&mut r, branch_comparison(&format!("{two}^1"), &three, false));
        assert!(matches!(r.message_panel(), Some(MessagePanel::Loading)));
        settle(&mut r);
        let Some(MessagePanel::Ready(m)) = r.message_panel() else { panic!("range read") };
        assert_eq!(m.commits.iter().map(Commit::subject).collect::<Vec<_>>(), ["three", "two"]);

        // the keyboard walks the range and opens the commit it is on
        assert_eq!(r.message_key(MessageKey::Up), None, "the first commit");
        r.execute(r.message_key(MessageKey::Down).unwrap());
        assert_eq!(r.message_cursor(), 1);
        assert_eq!(r.message_key(MessageKey::Down), None, "past the last: back to Files");
        assert_eq!(r.message_key(MessageKey::Enter), Some(R::ToggleMessage(Some(1))));
        r.execute(R::ToggleMessage(Some(1)));
        assert!(r.message_lines(80).unwrap().iter().any(|l| l.kind == LineKind::Meta), "the commit opened");
        r.execute(R::ToggleMessage(Some(9)));
        assert_eq!(r.notice.as_deref(), Some("No such commit"));
        r.execute(R::ToggleMessage(None));
        assert!(r.message_collapsed());
        assert_eq!(r.message_key(MessageKey::Enter), Some(R::ToggleMessage(None)), "collapsed: Enter expands");
        assert!(r.message_lines(80).is_none(), "hidden: no heading or body consumes space");
        assert!(r.disabled(&R::ToggleMessage(None)).is_none(), "hidden panel remains available to show");
        r.execute(R::ToggleMessage(None));

        show(&mut r, branch_comparison(&format!("{three}^1"), &three, false));
        settle(&mut r);
        assert_eq!(r.message_lines(80).unwrap()[0].text, "▾ three");

        show(&mut r, crate::repository_session::local_spec());
        assert!(r.message_panel().is_none(), "the working tree has no message");
        assert_eq!(r.disabled(&R::ToggleMessage(None)), Some("This comparison has no commit message"));
    }
}
