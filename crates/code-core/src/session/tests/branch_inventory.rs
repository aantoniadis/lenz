use super::*;
use lenz_content::branches::Refs;
use lenz_content::git::testing::{commit_all, git, repo, write as put};

fn load_branches(s: &mut Session) {
    s.dispatch(Command::OpenBranches);
    settle(s, |s| s.state().branches().is_some_and(|v| !v.loading));
}

fn has_branch(s: &Session, name: &str) -> bool {
    s.state().branch_rows("").iter().any(|row| {
        matches!(&row.target, crate::branches::BranchTarget::Branch(branch) if branch == name)
    })
}

fn obsolete_refs() -> Refs {
    Refs { local: vec!["obsolete-result".into()], ..Refs::default() }
}

#[test]
fn branch_inventory_does_not_wait_for_worktree_changed_path_counts() {
    let (r, wt, mut s) = git_branches::repo_session();
    put(r.path(), "untracked/main.txt", "main change\n");
    put(&wt.path().join("second"), "untracked/other.txt", "other change\n");
    load_branches(&mut s);
    let view = s.state().branches().unwrap();
    assert!(view.has_snapshot && view.error.is_none());
    assert_eq!(view.worktrees.len(), 2);
    assert!(view.worktrees.iter().all(|worktree| worktree.dirty.is_none()),
        "choosing a branch must not enumerate changed/untracked files in every worktree");
    assert!(has_branch(&s, "main") && has_branch(&s, "feature/x"));
    assert!(s.state().branch_rows("").iter().any(|row| row.section == "Remote"));
    assert!(s.state().branch_rows("").iter().any(|row| row.section == "Worktrees"));
}

#[test]
fn branch_inventory_reopens_immediately_then_refreshes_external_refs() {
    let (r, _wt, mut s) = git_branches::repo_session();
    load_branches(&mut s);
    s.dispatch(Command::CloseBranches);
    git(r.path(), &["branch", "created-elsewhere"]);

    s.dispatch(Command::OpenBranches);
    let view = s.state().branches().unwrap();
    assert!(view.loading && view.has_snapshot);
    assert_eq!(view.state_text(), None, "cached choices stay usable without a loading placeholder");
    assert!(has_branch(&s, "feature/x"));
    assert!(!has_branch(&s, "created-elsewhere"), "the old snapshot is visible before the fresh completion");

    settle(&mut s, |s| s.state().branches().is_some_and(|v| !v.loading));
    assert!(has_branch(&s, "created-elsewhere"));
    s.dispatch(Command::CloseBranches);
    s.dispatch(Command::OpenBranches);
    assert!(has_branch(&s, "created-elsewhere"), "the successful refresh replaces the retained snapshot");
}

#[test]
fn branch_inventory_close_and_reopen_rejects_obsolete_completions() {
    let (_r, _wt, mut s) = git_branches::repo_session();
    load_branches(&mut s);
    let root = s.state().repo().unwrap().root.clone();
    let old_request = s.branches_requests.load(Ordering::Relaxed);
    let cache = s.branches_cache.clone();

    s.dispatch(Command::CloseBranches);
    s.dispatch(Completion::Branches {
        request: old_request, root: root.clone(), result: Ok((obsolete_refs(), Vec::new())),
    });
    assert!(s.state().branches().is_none(), "dismissal cannot be undone by a late answer");
    assert_eq!(s.branches_cache, cache);

    s.dispatch(Command::OpenBranches);
    assert_ne!(s.branches_requests.load(Ordering::Relaxed), old_request);
    s.dispatch(Completion::Branches {
        request: old_request, root: root.clone(), result: Ok((obsolete_refs(), Vec::new())),
    });
    s.dispatch(Completion::Branches { request: old_request, root, result: Err("obsolete failure".into()) });
    assert!(s.state().branches().unwrap().loading);
    assert!(s.state().branches().unwrap().error.is_none());
    assert!(has_branch(&s, "feature/x") && !has_branch(&s, "obsolete-result"));
    assert_eq!(s.branches_cache, cache, "obsolete answers cannot poison the next reopen");
    settle(&mut s, |s| s.state().branches().is_some_and(|v| !v.loading));
    assert!(s.state().branches().unwrap().error.is_none());
}

#[test]
fn branch_inventory_root_change_drops_cached_rows_and_old_repository_results() {
    let (_r, _wt, mut s) = git_branches::repo_session();
    load_branches(&mut s);
    let old_root = s.state().repo().unwrap().root.clone();
    let old_request = s.branches_requests.load(Ordering::Relaxed);
    let other = repo();
    put(other.path(), "new.txt", "new repository\n");
    commit_all(other.path(), "new repository");
    git(other.path(), &["branch", "new-repository-only"]);

    s.dispatch(Command::OpenRoot(other.path().to_path_buf()));
    assert!(s.state().branches().is_none() && s.branches_cache.is_none());
    s.dispatch(Completion::Branches {
        request: old_request, root: old_root, result: Ok((obsolete_refs(), Vec::new())),
    });
    assert!(s.state().branches().is_none() && s.branches_cache.is_none());
    settle(&mut s, |s| s.state().repo().is_some_and(|r| r.root == other.path()));

    s.dispatch(Command::OpenBranches);
    assert!(!s.state().branches().unwrap().has_snapshot);
    assert!(!has_branch(&s, "feature/x"));
    settle(&mut s, |s| s.state().branches().is_some_and(|v| !v.loading));
    assert!(has_branch(&s, "new-repository-only") && !has_branch(&s, "obsolete-result"));
    assert_eq!(s.branches_cache.as_ref().unwrap().0, other.path());
}

#[test]
fn branch_inventory_comparison_modes_do_not_replace_the_full_cached_snapshot() {
    let (_r, _wt, mut s) = git_branches::repo_session();
    load_branches(&mut s);
    let cache = s.branches_cache.clone();
    for command in [Command::OpenWorktrees, Command::OpenRevisions] {
        s.dispatch(Command::CloseBranches);
        s.dispatch(command);
        assert!(s.state().branches().unwrap().has_snapshot);
        settle(&mut s, |s| s.state().branches().is_some_and(|v| !v.loading));
        assert_eq!(s.branches_cache, cache);
        let view = s.state().branches().unwrap();
        let rows = s.state().branch_rows("");
        if view.worktrees_only {
            assert!(!rows.is_empty() && rows.iter().all(|row| row.section == "Worktrees" && !row.current));
        } else {
            assert!(!rows.is_empty() && rows.iter().all(|row| matches!(row.section, "Local" | "Remote" | "Tags")));
        }
    }
    s.dispatch(Command::CloseBranches);
    s.dispatch(Command::OpenBranches);
    assert!(has_branch(&s, "feature/x"));
    assert!(s.state().branch_rows("").iter().any(|row| row.section == "Worktrees"));
}

#[test]
fn branch_inventory_refresh_failure_keeps_cached_choices_and_reports_the_error() {
    let (_r, _wt, mut s) = git_branches::repo_session();
    load_branches(&mut s);
    let cache = s.branches_cache.clone();
    s.dispatch(Command::CloseBranches);
    s.dispatch(Command::OpenBranches);
    let request = s.branches_requests.load(Ordering::Relaxed);
    let root = s.state().repo().unwrap().root.clone();
    s.dispatch(Completion::Branches { request, root, result: Err("refs cannot be read".into()) });
    let view = s.state().branches().unwrap();
    assert!(!view.loading && view.has_snapshot);
    assert_eq!(view.state_text(), Some("refs cannot be read"));
    assert!(has_branch(&s, "feature/x"));
    assert_eq!(s.branches_cache, cache);
}
