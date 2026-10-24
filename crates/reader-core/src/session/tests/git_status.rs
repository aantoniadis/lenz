use super::*;
use lenz_content::branches::FileStatus;
use lenz_content::git::testing::{commit_all, git, repo, try_git, write as put};

fn tick(s: &mut Session) { s.dispatch(Completion::Semantic(semantics::Event::Tick)); }

/// Settles until `done`, ticking so repository signals are checked without waiting for the tick thread.
fn settle_ticking(s: &mut Session, done: impl Fn(&Session) -> bool) {
    let until = std::time::Instant::now() + Duration::from_secs(10);
    while !done(s) {
        assert!(std::time::Instant::now() < until, "timed out: {:?} {:?}", s.state().tabs().iter().map(|t| (t.title(), s.state().tab_dirty(t.id()), s.state().tab_status(t.id()))).collect::<Vec<_>>(), s.status_message());
        tick(s);
        while let Ok(c) = s.completions().try_recv() { s.dispatch(c); }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn row<'a>(s: &'a Session, path: &str) -> &'a crate::project::TreeRow {
    s.state().tree_rows().find(|r| &*r.path == path).unwrap_or_else(|| panic!("no row {path}"))
}
fn row_status(s: &Session, path: &str) -> Option<FileStatus> { s.state().row_status(row(s, path)) }
fn edit(s: &mut Session, text: &str) {
    let tab = s.state().active().unwrap().id();
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..0, text: text.into() }], kind: crate::text::EditKind::Replace });
}

#[test]
fn explorer_and_tabs_show_repository_status_apart_from_drafts_and_save_keeps_it() {
    let state = tempfile::tempdir().unwrap();
    let r = repo();
    for f in ["clean.txt", "edited.txt", "staged.txt", "sub/deep.txt"] { put(r.path(), f, format!("{f}\n")); }
    commit_all(r.path(), "base");
    put(r.path(), "edited.txt", "edited on disk\n");
    put(r.path(), "staged.txt", "staged\n");
    git(r.path(), &["add", "staged.txt"]);
    put(r.path(), "added.txt", "added\n");
    git(r.path(), &["add", "added.txt"]);
    put(r.path(), "loose/new.txt", "untracked\n");
    let mut s = session(state.path());
    s.dispatch(Command::OpenRoot(r.path().to_path_buf()));
    settle(&mut s, |s| scanned(s) && s.state().tree_rows().any(|r| &*r.path == "edited.txt") && row_status(s, "edited.txt").is_some());
    let rows: Vec<_> = ["added.txt", "clean.txt", "edited.txt", "loose", "staged.txt", "sub"].iter().map(|p| row_status(&s, p)).collect();
    assert_eq!(rows, [Some(FileStatus::Added), None, Some(FileStatus::Modified), Some(FileStatus::Untracked), Some(FileStatus::Modified), None]);

    // A dirty draft over a Git-clean file: unsaved, with no repository status.
    open(&mut s, Command::OpenRetained(r.path().join("clean.txt")), "clean.txt");
    let clean = s.state().active().unwrap().id();
    edit(&mut s, "draft ");
    assert!(s.state().tab_dirty(clean));
    assert_eq!(s.state().tab_status(clean), None);
    assert!(s.state().row_dirty(row(&s, "clean.txt")) && row_status(&s, "clean.txt").is_none());
    assert!(!s.state().row_dirty(row(&s, "edited.txt")));
    // A file inside an untracked folder is untracked; a tracked one inside a clean folder has no badge.
    open(&mut s, Command::OpenRetained(r.path().join("loose/new.txt")), "new.txt");
    assert_eq!(s.state().tab_status(s.state().active().unwrap().id()), Some(FileStatus::Untracked));
    open(&mut s, Command::OpenRetained(r.path().join("sub/deep.txt")), "deep.txt");
    assert_eq!(s.state().tab_status(s.state().active().unwrap().id()), None);
    // An untitled document is unsaved with no repository status until Save As gives it a file.
    s.dispatch(Command::NewDocument);
    let untitled = s.state().active().unwrap().id();
    edit(&mut s, "new ");
    assert!(s.state().tab_dirty(untitled) && s.state().tab_status(untitled).is_none());
    s.dispatch(Command::SaveAs { tab: untitled, path: r.path().join("saved-as.txt") });
    settle_ticking(&mut s, |s| !s.state().tab_dirty(untitled) && s.state().tab_status(untitled) == Some(FileStatus::Untracked));

    // Save writes the draft: the draft is clean and the file is now modified against HEAD.
    s.dispatch(Command::ActivateTab(clean));
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source().ends_with("clean.txt")));
    s.dispatch(Command::Save { tab: clean });
    settle_ticking(&mut s, |s| !s.state().tab_dirty(clean) && s.state().tab_status(clean) == Some(FileStatus::Modified));
    assert_eq!(row_status(&s, "clean.txt"), Some(FileStatus::Modified));
    assert_eq!(std::fs::read_to_string(r.path().join("clean.txt")).unwrap(), "draft clean.txt\n");
    assert_eq!(git(r.path(), &["diff", "--cached", "--name-only"]), "added.txt\nstaged.txt\n", "the index is untouched");
}

#[test]
fn head_and_index_changes_refresh_status_without_touching_drafts_or_history() {
    let state = tempfile::tempdir().unwrap();
    let r = repo();
    put(r.path(), "a.txt", "one\n");
    put(r.path(), "gone.txt", "tracked\n");
    commit_all(r.path(), "base");
    put(r.path(), "a.txt", "two\n");
    let mut s = session(state.path());
    s.dispatch(Command::OpenRoot(r.path().to_path_buf()));
    open(&mut s, Command::OpenRetained(r.path().join("gone.txt")), "gone.txt");
    let gone = s.state().active().unwrap().id();
    open(&mut s, Command::OpenRetained(r.path().join("a.txt")), "a.txt");
    let a = s.state().active().unwrap().id();
    settle(&mut s, |s| s.state().tab_status(a) == Some(FileStatus::Modified));
    edit(&mut s, "draft ");
    // Staged from a terminal: still modified against HEAD.
    git(r.path(), &["add", "a.txt"]);
    settle_ticking(&mut s, |s| s.state().tab_status(a) == Some(FileStatus::Modified));
    // Committed from a terminal: HEAD now matches the disk, the draft and its undo stay.
    git(r.path(), &["commit", "-q", "--no-verify", "-m", "two"]);
    settle_ticking(&mut s, |s| s.state().tab_status(a).is_none());
    assert_eq!(source_text(s.state().current_source().unwrap()), "draft two\n");
    assert!(s.state().tab_dirty(a) && active_view(&s).can_undo());
    // A tracked file removed from disk is deleted; its tab stays.
    std::fs::remove_file(r.path().join("gone.txt")).unwrap();
    settle_ticking(&mut s, |s| s.state().tab_status(gone) == Some(FileStatus::Deleted));
    assert!(s.state().tabs().iter().any(|t| t.id() == gone));
    s.dispatch(Command::Undo { tab: a });
    assert_eq!(source_text(s.state().current_source().unwrap()), "two\n");
}

#[test]
fn conflicts_renames_and_folders_outside_git() {
    let state = tempfile::tempdir().unwrap();
    let r = repo();
    put(r.path(), "a.txt", "base\n");
    put(r.path(), "old.txt", "moving\n");
    commit_all(r.path(), "base");
    git(r.path(), &["switch", "-q", "-c", "other"]);
    put(r.path(), "a.txt", "other\n");
    commit_all(r.path(), "other");
    git(r.path(), &["switch", "-q", "main"]);
    put(r.path(), "a.txt", "main\n");
    commit_all(r.path(), "main");
    assert!(!try_git(r.path(), &["merge", "-q", "other"]));
    git(r.path(), &["mv", "old.txt", "new.txt"]);
    let mut s = session(state.path());
    s.dispatch(Command::OpenRoot(r.path().to_path_buf()));
    settle(&mut s, |s| scanned(s) && s.state().tree_rows().any(|r| &*r.path == "new.txt") && row_status(s, "a.txt").is_some());
    assert_eq!((row_status(&s, "a.txt"), row_status(&s, "new.txt")), (Some(FileStatus::Conflicted), Some(FileStatus::Renamed)));
    // Another folder, not a repository: the previous repository's statuses go and none are invented.
    let plain = tempfile::tempdir().unwrap();
    put(plain.path(), "a.txt", "plain\n");
    s.dispatch(Command::OpenRoot(plain.path().to_path_buf()));
    settle(&mut s, |s| scanned(s) && s.state().repo_checked() && s.state().tree_rows().any(|r| &*r.path == "a.txt"));
    for _ in 0..3 { tick(&mut s); while let Ok(c) = s.completions().try_recv() { s.dispatch(c); } }
    assert_eq!(row_status(&s, "a.txt"), None);
}
