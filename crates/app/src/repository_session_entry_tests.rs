//! Entry paths retain Git's launch-directory semantics after the inventory resolves the repository root.
use super::*;
use crate::git::testing::{commit_all, git, repo, write};

fn settle(state: &mut RepositorySession) {
    let start = std::time::Instant::now();
    while state.busy() {
        state.poll();
        assert!(start.elapsed().as_secs() < 10, "entry work did not settle");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}
fn paths(set: &ChangeSet) -> Vec<&str> { set.comparisons.iter().map(|c| c.path.as_str()).collect() }

#[test]
fn entry_show_older_commit_places_history_without_changing_the_requested_comparison() {
    let root = repo();
    let mut commits = Vec::new();
    for n in 0..3 {
        write(root.path(), "f.txt", format!("commit {n}\n"));
        commit_all(root.path(), &format!("Commit {n}"));
        commits.push(git(root.path(), &["rev-parse", "HEAD"]).trim().to_owned());
    }
    let request = crate::cli::parse_with(&["show".into(), commits[1].clone()], root.path(), &Default::default()).unwrap();
    let mut state = RepositorySession::open(&request, root.path().into());
    settle(&mut state);
    state.execute(RepositoryCommand::SetPanels { branches: true, history: true });
    settle(&mut state);
    assert_eq!(state.picker.log.id(state.picker.log_row(state.picker.anchor).unwrap()), commits[1]);
    assert_eq!(state.current_spec.as_ref(), Some(&request.spec));
    assert!(state.take_change_set().is_none(), "revealing the log must not replace the CLI comparison");
    assert_eq!(state.picker.clipboard_text(&RepositoryCommand::CopyHash, state.picker.anchor), Some(commits[1].clone()));
}

#[test]
fn entry_follow_resolves_relative_absolute_and_literal_filenames_from_the_launch_directory() {
    let root = repo();
    write(root.path(), "f[one].txt", "wrong root file\n");
    commit_all(root.path(), "Root decoy");
    write(root.path(), "nested/f[one].txt", "expected nested file\n");
    commit_all(root.path(), "Nested file");
    let cwd = root.path().join("nested");
    let expected = git(root.path(), &["rev-parse", "HEAD"]).trim().to_owned();
    for name in ["f[one].txt".into(), cwd.join("f[one].txt").to_string_lossy().into_owned(), "../nested/f[one].txt".into()] {
        let request = crate::cli::parse_with(&["log".into(), "--follow".into(), name], &cwd, &Default::default()).unwrap();
        let mut state = RepositorySession::open(&request, cwd.clone());
        settle(&mut state);
        let (set, _) = state.take_change_set().unwrap();
        assert_eq!(paths(&set), ["nested/f[one].txt"]);
        assert_eq!(set.read(&set.comparisons[0].right).unwrap(), b"expected nested file\n");
        assert_eq!(state.picker.log.id(state.picker.log_row(state.picker.anchor).unwrap()), expected);
        state.execute(RepositoryCommand::RefreshRepository);
        settle(&mut state);
        assert_eq!(paths(&state.take_change_set().unwrap().0), ["nested/f[one].txt"]);
    }
}

#[test]
fn entry_show_refresh_keeps_relative_pathspec_scope_after_resolving_the_root() {
    let root = repo();
    for name in ["f.txt", "nested/f.txt"] { write(root.path(), name, "before\n"); }
    commit_all(root.path(), "Base");
    write(root.path(), "f.txt", "wrong root change\n");
    write(root.path(), "nested/f.txt", "expected nested change\n");
    commit_all(root.path(), "Both changed");
    let cwd = root.path().join("nested");
    let request = crate::cli::parse_with(&["show".into(), "HEAD".into(), "--".into(), "f.txt".into()], &cwd, &Default::default()).unwrap();
    let mut state = RepositorySession::open(&request, cwd);
    state.execute(RepositoryCommand::SetPanels { branches: true, history: true });
    settle(&mut state);
    for _ in 0..2 {
        state.execute(RepositoryCommand::RefreshRepository);
        settle(&mut state);
        let (set, spec) = state.take_change_set().unwrap();
        assert_eq!(spec, request.spec);
        assert_eq!(paths(&set), ["nested/f.txt"]);
        assert_eq!(set.read(&set.comparisons[0].right).unwrap(), b"expected nested change\n");
    }
}

#[test]
fn entry_log_scope_preserves_git_pathspec_magic_and_literal_launch_prefix_on_both_constructors() {
    let root = repo();
    for name in ["f.txt", "scope[one]/f.txt", "scope[one]/F.TXT", "scope[one]/f[one].txt", "scope[one]/deep/f.txt", "scopeo/f.txt"] {
        write(root.path(), name, "before\n");
    }
    commit_all(root.path(), "Base");
    for name in ["f.txt", "scope[one]/f.txt", "scope[one]/F.TXT", "scope[one]/f[one].txt", "scope[one]/deep/f.txt", "scopeo/f.txt"] {
        write(root.path(), name, "after\n");
    }
    commit_all(root.path(), "Changed");
    let cwd = root.path().join("scope[one]");
    let absolute = cwd.join("f.txt").to_string_lossy().into_owned();
    for scope in [vec!["f.txt"], vec!["../f.txt"], vec!["."], vec![":(glob)*.txt"], vec![":(icase)f.txt"], vec![":(literal)f[one].txt"], vec![":/f.txt"], vec![":(top)f.txt"], vec![".", ":(exclude)f.txt"], vec![absolute.as_str()]] {
        let mut args = vec!["log".into(), "--".into()];
        args.extend(scope.iter().map(|s| (*s).to_owned()));
        let request = crate::cli::parse_with(&args, &cwd, &Default::default()).unwrap();
        let mut expected_spec = request.spec.clone();
        if let Spec::Git { revs, .. } = &mut expected_spec { *revs = vec!["HEAD^".into(), "HEAD".into()]; }
        let expected = ChangeSet::load(&expected_spec, &cwd).unwrap();
        for loaded in [false, true] {
            let mut state = if loaded {
                let (set, spec, _) = initial_log_comparison(&cwd, &request.spec).unwrap();
                RepositorySession::open_loaded_with_paths(cwd.clone(), true, Some(initial_log_branches(&request.spec)), None, log_paths(&request.spec), std::sync::Arc::new(set), spec)
            } else { RepositorySession::open(&request, cwd.clone()) };
            settle(&mut state);
            if let Some((set, _)) = state.take_change_set() { assert_eq!(paths(&set), paths(&expected), "scope={scope:?}, loaded={loaded}"); }
            else { assert_eq!(paths(state.set.as_ref().unwrap()), paths(&expected)); }
            state.execute(RepositoryCommand::RefreshRepository);
            settle(&mut state);
            assert_eq!(paths(&state.take_change_set().unwrap().0), paths(&expected), "refresh scope={scope:?}, loaded={loaded}");
        }
    }
    // A grouped working-tree view uses the same cwd, including new file matches on a later refresh.
    write(root.path(), "f.txt", "wrong root draft\n");
    write(root.path(), "scope[one]/f.txt", "nested draft\n");
    let grouped = repo::local_changes_paths(&cwd, false, &["f.txt".into()]).unwrap();
    assert_eq!(paths(&grouped), ["scope[one]/f.txt"]);
}

#[test]
fn entry_navigation_restores_exact_launch_scoped_comparison_and_cancels_stale_restores() {
    let root = repo();
    for n in 0..3 {
        write(root.path(), "f.txt", format!("root {n}\n"));
        write(root.path(), "nested/f.txt", format!("nested {n}\n"));
        commit_all(root.path(), &format!("Commit {n}"));
    }
    let cwd = root.path().join("nested");
    let request = crate::cli::parse_with(&["show".into(), "HEAD".into(), "--".into(), "f.txt".into()], &cwd, &Default::default()).unwrap();
    let mut state = RepositorySession::open(&request, cwd.clone());
    state.execute(RepositoryCommand::SetPanels { branches: true, history: true });
    settle(&mut state);
    let previous = git(root.path(), &["rev-parse", "HEAD~1"]).trim().to_owned();
    let row = state.picker.display_row(state.picker.log.row_of(&previous).unwrap()).unwrap();
    state.execute(RepositoryCommand::SelectRow { row, extend: false, two: false });
    settle(&mut state);
    let (away, _) = state.take_change_set().unwrap();
    assert_eq!(paths(&away), ["f.txt", "nested/f.txt"]);
    assert_eq!(state.spec_cwd, state.picker.root);
    for _ in 0..2 {
        state.go_back();
        settle(&mut state);
        let (back, spec) = state.take_change_set().unwrap();
        assert_eq!(spec, request.spec);
        assert_eq!(state.spec_cwd, cwd);
        assert_eq!(paths(&back), ["nested/f.txt"]);
        assert_eq!(back.read(&back.comparisons[0].right).unwrap(), b"nested 2\n");
        state.execute(RepositoryCommand::RefreshRepository);
        settle(&mut state);
        assert_eq!(paths(&state.take_change_set().unwrap().0), ["nested/f.txt"]);
        state.go_forward();
        settle(&mut state);
        assert_eq!(paths(&state.take_change_set().unwrap().0), ["f.txt", "nested/f.txt"]);
    }
    // A later explicit row selection supersedes Back even if its read has not completed.
    state.go_back();
    state.execute(RepositoryCommand::SelectRow { row, extend: false, two: false });
    settle(&mut state);
    assert!(state.take_change_set().is_none(), "the still-displayed requested comparison is reused");
    assert_eq!(paths(state.set.as_ref().unwrap()), ["f.txt", "nested/f.txt"]);
    assert_eq!(state.spec_cwd, state.picker.root);
    assert!(state.navigation_comparison.is_none() && state.pending.is_none(), "the earlier Back request is canceled");
}

#[test]
fn entry_ui_file_history_during_inventory_keeps_root_relative_paths() {
    let root = repo();
    write(root.path(), "f.txt", "root\n");
    write(root.path(), "nested/f.txt", "nested\n");
    commit_all(root.path(), "Files");
    let cwd = root.path().join("nested");
    let request = crate::cli::parse_with(&["show".into(), "HEAD".into(), "--".into(), "f.txt".into()], &cwd, &Default::default()).unwrap();
    let mut state = RepositorySession::open(&request, cwd);
    // The view's file paths already come from Git's root-relative inventory.
    state.execute(RepositoryCommand::FileHistory("nested/f.txt".into()));
    settle(&mut state);
    assert_eq!(paths(&state.take_change_set().unwrap().0), ["nested/f.txt"]);
}
