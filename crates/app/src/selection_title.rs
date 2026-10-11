//! What a view compares, named for people: `main ~ feature/x`, `v0.1.0 (a1b2c3d)`, `#12 Title`. Both frontends.

use crate::changeset::Spec;
use crate::history::{RefKind, Refs};
use crate::repo::Stash;

/// git's empty tree (SHA-1): the left Side of a root commit.
// ponytail: SHA-256 repositories title a root commit as "<empty> ~ <commit>"; read history::empty_tree if that matters.
pub const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

#[derive(Debug, Clone, PartialEq)]
pub struct SelectionTitle {
    /// What the header and window title show.
    pub text: String,
    /// The full revisions, for a tooltip.
    pub detail: String,
}

/// The title of a comparison, or `None` for plain files (their file labels say it already).
pub fn title(spec: &Spec, refs: Option<&Refs>, stashes: &[Stash], pr: Option<(u64, &str)>) -> Option<SelectionTitle> {
    let Spec::Git { cached, revs, .. } = spec else { return None };
    let detail = revs.join(" ");
    let done = |text: String| Some(SelectionTitle { text, detail: detail.clone() });
    if let Some((number, name)) = pr { return done(format!("#{number} {name}")); }
    let name = |rev: &str| name(rev, refs);
    match revs.as_slice() {
        [] if *cached => done("HEAD ~ Index".into()),
        [only] if only == "HEAD" => done("HEAD ~ Working tree".into()),
        [only] => match (only.split_once("..."), only.strip_suffix("^1")) {
            (Some((a, b)), _) => done(format!("{} ... {}", name(a), name(b))),
            // a range from a commit up to the working tree
            (None, Some(older)) => done(format!("{} .. Working tree", name(older))),
            (None, None) => done(format!("{} ~ Working tree", name(only))),
        },
        [left, right] => {
            if let Some(s) = stashes.iter().find(|s| &s.sha == right && *left == format!("{right}^1")) {
                return done(format!("stash@{{{}}}", s.index));
            }
            if left == EMPTY_TREE { return done(one(right, refs)); }
            if let Some(parent) = left.strip_prefix(right.as_str()).and_then(|p| p.strip_prefix('^')) {
                return done(if parent == "1" { one(right, refs) } else { format!("{} (vs ^{parent})", one(right, refs)) });
            }
            match left.strip_suffix("^1") {
                Some(older) => done(format!("{} .. {}", name(older), name(right))),
                None => done(format!("{} ~ {}", name(left), name(right))),
            }
        }
        _ => None,
    }
}

fn is_id(rev: &str) -> bool { rev.len() >= 7 && rev.bytes().all(|b| b.is_ascii_hexdigit()) }
fn short(rev: &str) -> &str { rev.get(..7).unwrap_or(rev) }

/// A commit's best ref name: the current branch, a branch, a remote branch, then a tag.
fn ref_name<'a>(id: &str, refs: Option<&'a Refs>) -> Option<&'a str> {
    let rank = |kind: RefKind, current: bool| match (kind, current) {
        (RefKind::Branch, true) => 0, (RefKind::Branch, false) => 1, (RefKind::Remote, _) => 2, (RefKind::Tag, _) => 3, (RefKind::Head, _) => 9,
    };
    refs?.at(id).iter().filter(|r| r.kind != RefKind::Head).min_by_key(|r| rank(r.kind, r.current)).map(|r| r.name.as_str())
}

/// One side of a comparison: a ref name, or a commit's best name, or its short SHA.
fn name(rev: &str, refs: Option<&Refs>) -> String {
    // a compare against HEAD names the branch HEAD is on
    if rev == "HEAD" && let Some(head) = refs.and_then(Refs::head) { return ref_name(head, refs).unwrap_or(short(head)).to_string(); }
    if is_id(rev) { return ref_name(rev, refs).unwrap_or(short(rev)).to_string(); }
    ["refs/heads/", "refs/remotes/", "refs/tags/"].iter().find_map(|p| rev.strip_prefix(p)).unwrap_or(rev).to_string()
}

/// A single commit: `main (a1b2c3d)` when named, else `a1b2c3d`.
fn one(rev: &str, refs: Option<&Refs>) -> String {
    if !is_id(rev) { return name(rev, refs); }
    match ref_name(rev, refs) { Some(n) => format!("{n} ({})", short(rev)), None => short(rev).to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changeset::Spec;
    use crate::history::{Ref, RefKind, Refs};
    use crate::repo::Stash;

    const A: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678";
    const B: &str = "e4f5a6b7c8d90123456789abcdef0123456789ab";

    fn git(revs: &[&str]) -> Spec { Spec::Git { cached: false, revs: revs.iter().map(|r| r.to_string()).collect(), paths: Vec::new(), untracked: false } }
    fn refs() -> Refs {
        let r = |kind, name: &str, current| Ref { kind, name: name.into(), current };
        Refs::from_map([
            (A.to_string(), vec![r(RefKind::Head, "HEAD", false), r(RefKind::Remote, "origin/main", false), r(RefKind::Branch, "main", true), r(RefKind::Tag, "v0.1.0", false)]),
            (B.to_string(), vec![r(RefKind::Tag, "v0.2.0", false)]),
        ].into_iter().collect())
    }
    fn text(spec: &Spec) -> String { title(spec, Some(&refs()), &[], None).unwrap().text }

    #[test]
    fn one_commit_prefers_the_current_branch_then_shows_its_sha() {
        assert_eq!(text(&git(&[&format!("{A}^1"), A])), "main (a1b2c3d)");
        assert_eq!(text(&git(&[&format!("{B}^1"), B])), "v0.2.0 (e4f5a6b)");
    }

    #[test]
    fn unnamed_commit_is_its_short_sha_and_other_parent_is_marked() {
        let c = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(text(&git(&[&format!("{c}^1"), c])), "0123456");
        assert_eq!(text(&git(&[&format!("{c}^2"), c])), "0123456 (vs ^2)");
    }

    #[test]
    fn root_commit_against_the_empty_tree_is_one_commit() {
        assert_eq!(text(&git(&[EMPTY_TREE, B])), "v0.2.0 (e4f5a6b)");
    }

    #[test]
    fn two_commits_range_and_branch_compare() {
        assert_eq!(text(&git(&[A, B])), "main ~ v0.2.0");
        assert_eq!(text(&git(&[&format!("{A}^1"), B])), "main .. v0.2.0");
        assert_eq!(text(&git(&["main", "refs/heads/feature/x"])), "main ~ feature/x");
        assert_eq!(text(&git(&["main...refs/heads/feature/x"])), "main ... feature/x");
    }

    #[test]
    fn compare_with_head_names_its_branch() {
        assert_eq!(text(&git(&["HEAD", "refs/heads/feature/x"])), "main ~ feature/x");
        assert_eq!(text(&git(&["HEAD...refs/heads/feature/x"])), "main ... feature/x");
        assert_eq!(title(&git(&["HEAD", "refs/heads/feature/x"]), None, &[], None).unwrap().text, "HEAD ~ feature/x", "refs not loaded");
    }

    #[test]
    fn range_up_to_the_working_tree_names_its_oldest_commit() {
        assert_eq!(text(&git(&[&format!("{B}^1")])), "v0.2.0 .. Working tree");
    }

    #[test]
    fn local_changes_index_and_working_tree() {
        assert_eq!(text(&crate::repository_session::local_spec()), "HEAD ~ Working tree");
        assert_eq!(text(&crate::repository_session::index_spec()), "HEAD ~ Index");
        assert_eq!(text(&git(&[A])), "main ~ Working tree");
    }

    #[test]
    fn stash_and_pull_request() {
        let stash = Stash { index: 0, sha: B.into(), message: "WIP on main".into(), date: 0 };
        assert_eq!(title(&git(&[&format!("{B}^1"), B]), Some(&refs()), &[stash], None).unwrap().text, "stash@{0}");
        assert_eq!(title(&git(&[A, B]), Some(&refs()), &[], Some((12, "Limit request body size"))).unwrap().text, "#12 Limit request body size");
    }

    #[test]
    fn refs_not_loaded_falls_back_to_short_shas() {
        let t = title(&git(&[&format!("{A}^1"), A]), None, &[], None).unwrap();
        assert_eq!(t.text, "a1b2c3d");
        assert_eq!(t.detail, format!("{A}^1 {A}"));
    }

    #[test]
    fn plain_files_and_empty_have_no_title() {
        assert_eq!(title(&Spec::Plain { left: "a".into(), right: "b".into() }, None, &[], None), None);
        assert_eq!(title(&Spec::Empty, None, &[], None), None);
    }
}
