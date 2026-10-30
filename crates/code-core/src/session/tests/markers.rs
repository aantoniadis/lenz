use super::*;
use crate::markers::{BaselineKind, LineChange};

/// The markers as the adapters draw them: each line's own change letter, then the deletion above it.
fn gutter(s: &Session, tab: TabId, lines: usize) -> Vec<String> {
    let m = s.state().markers(tab).expect("the tab tracks markers");
    (0..lines).map(|line| {
        let change = m.change(line).map_or(" ", LineChange::letter);
        let deleted = if m.deleted_above(line) > 0 { "D" } else { " " };
        format!("{change}{deleted}")
    }).collect()
}

fn baseline(s: &Session, tab: TabId) -> Option<BaselineKind> {
    s.state().markers(tab).and_then(|m| m.baseline()).map(crate::markers::Baseline::kind)
}

/// Drives the session's markers to their settled result, as their debounce wake does.
pub(super) fn settle_markers(s: &mut Session, done: impl Fn(&Session) -> bool) {
    let until = std::time::Instant::now() + Duration::from_secs(10);
    while !done(s) {
        let tab = s.state().active().map(|t| t.id());
        assert!(std::time::Instant::now() < until, "markers did not settle: {:?} {:?}", tab.and_then(|t| s.state().markers(t).map(|m| (m.pending(), m.failure().map(str::to_owned)))), s.status_message());
        s.dispatch(Completion::MarkersDue);
        while let Ok(c) = s.completions().try_recv() { s.dispatch(c); }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn edit_at(s: &mut Session, range: std::ops::Range<usize>, text: &str) {
    let tab = s.state().active().unwrap().id();
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range, text: text.into() }], kind: crate::text::EditKind::Replace });
}

#[test]
fn inactive_markers_cannot_retain_a_clean_snapshot_outside_the_cache_budget() {
    let dir = tempfile::tempdir().unwrap();
    let first = write(dir.path(), "first.txt", b"one\ntwo\n");
    let second = write(dir.path(), "second.txt", b"other\n");
    let mut s = session(&dir.path().join("state"));
    s.cache = crate::cache::InactiveCache::new(0);
    open(&mut s, Command::OpenRetained(first.clone()), "first.txt");
    let tab = s.state().active().unwrap().id();
    settle_markers(&mut s, |s| s.state().markers(tab).is_some_and(|m| !m.pending()));
    let observed = Arc::downgrade(s.state().current_source().unwrap());
    open(&mut s, Command::OpenRetained(second), "second.txt");
    assert!(s.state().tabs().iter().any(|t| t.id() == tab), "the inactive tab remains available");
    settle(&mut s, |_| observed.upgrade().is_none());
    assert!(observed.upgrade().is_none(), "published marker baselines must not defeat clean snapshot eviction");
    assert!(s.state().markers(tab).is_none(), "inactive markers are rebuilt when shown again");
    open(&mut s, Command::ActivateTab(tab), "first.txt");
    settle_markers(&mut s, |s| s.state().markers(tab).is_some_and(|m| !m.pending()));
    assert_eq!(gutter(&s, tab, 2), ["  ", "  "]);
    assert_eq!(std::fs::read(first).unwrap(), b"one\ntwo\n");
}

#[test]
fn a_file_outside_git_marks_changed_lines_against_its_last_saved_version() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "note.txt", b"one\ntwo\nthree\n");
    let mut s = session(&d.path().join("state"));
    open(&mut s, Command::OpenRetained(path.clone()), "note.txt");
    let tab = s.state().active().unwrap().id();
    settle_markers(&mut s, |s| s.state().markers(tab).is_some_and(|m| !m.pending()));
    assert_eq!(baseline(&s, tab), Some(BaselineKind::Saved), "outside Git the last save is the baseline");
    assert_eq!(gutter(&s, tab, 3), ["  ", "  ", "  "], "a saved file changes nothing");

    // A line inserted above a rewritten one: unsaved, so both are marked.
    edit_at(&mut s, 0..0, "one and a half\n");
    edit_at(&mut s, 19..22, "two point five");
    settle_markers(&mut s, |s| s.state().markers(tab).is_some_and(|m| !m.pending()));
    assert_eq!(gutter(&s, tab, 4), ["A ", "  ", "M ", "  "]);
    assert_eq!(s.state().markers(tab).unwrap().summary().as_deref(), Some("vs last saved: 1 added, 1 modified"));
    s.dispatch(Command::Select(Selection { anchor: 19, head: 19 }));
    assert_eq!(s.status_message(), "Modified from last saved", "the caret line's marker in words");
    assert!(s.status_details().contains("vs last saved: 1 added, 1 modified"), "{}", s.status_details());

    // Undo takes them back to the saved text: nothing is changed.
    s.dispatch(Command::Undo { tab });
    s.dispatch(Command::Undo { tab });
    settle_markers(&mut s, |s| s.state().markers(tab).is_some_and(|m| !m.pending()));
    assert_eq!(gutter(&s, tab, 3), ["  ", "  ", "  "]);
    assert_eq!(s.state().markers(tab).unwrap().summary().as_deref(), Some("vs last saved: no changes"));
    assert_eq!(std::fs::read(&path).unwrap(), b"one\ntwo\nthree\n", "markers never write");
}

/// Settles markers while ticking the repository, so HEAD and status changes are noticed as they are in use.
fn settle_repo(s: &mut Session, done: impl Fn(&Session) -> bool) {
    let until = std::time::Instant::now() + Duration::from_secs(10);
    while !done(s) {
        assert!(std::time::Instant::now() < until, "repository markers did not settle");
        s.dispatch(Completion::Semantic(semantics::Event::Tick));
        while let Ok(c) = s.completions().try_recv() { s.dispatch(c); }
        s.dispatch(Completion::MarkersDue);
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn a_tracked_draft_marks_lines_against_head_and_saving_keeps_them() {
    use lenz_content::git::testing::{commit_all, git, repo, write as put};
    let d = tempfile::tempdir().unwrap();
    let r = repo();
    put(r.path(), "a.txt", "one\ntwo\nthree\n");
    commit_all(r.path(), "base");
    let mut s = session(&d.path().join("state"));
    s.dispatch(Command::OpenRoot(r.path().to_path_buf()));
    open(&mut s, Command::OpenRetained(r.path().join("a.txt")), "a.txt");
    let tab = s.state().active().unwrap().id();
    settle_repo(&mut s, |s| s.state().git_status.is_some() && s.state().markers(tab).is_some_and(|m| !m.pending()));
    assert_eq!(baseline(&s, tab), Some(BaselineKind::Head));
    assert_eq!(gutter(&s, tab, 3), ["  ", "  ", "  "], "a committed, unedited file changes nothing");
    assert!(s.state().markers(tab).unwrap().baseline().unwrap().label().starts_with("HEAD (main · "), "{:?}", s.state().markers(tab).unwrap().baseline());

    edit_at(&mut s, 0..3, "ONE");
    settle_repo(&mut s, |s| s.state().markers(tab).is_some_and(|m| !m.pending() && m.change(0).is_some()));
    assert_eq!(gutter(&s, tab, 3), ["M ", "  ", "  "], "the unsaved edit is marked against HEAD");
    assert!(s.state().tab_dirty(tab) && s.state().tab_status(tab).is_none(), "dirty and repository-clean are separate");

    // Save: the draft is clean, the change against HEAD stays.
    s.dispatch(Command::Save { tab });
    settle_repo(&mut s, |s| !s.state().tab_dirty(tab) && s.state().tab_status(tab) == Some(lenz_content::branches::FileStatus::Modified));
    assert_eq!(gutter(&s, tab, 3), ["M ", "  ", "  "], "saving a tracked file does not erase its repository markers");
    assert_eq!(std::fs::read_to_string(r.path().join("a.txt")).unwrap(), "ONE\ntwo\nthree\n");

    // A commit made elsewhere moves HEAD: the same text is now the committed version.
    git(r.path(), &["commit", "-q", "--no-verify", "-am", "one"]);
    settle_repo(&mut s, |s| s.state().markers(tab).is_some_and(|m| !m.pending() && m.change(0).is_none()));
    assert_eq!(gutter(&s, tab, 3), ["  ", "  ", "  "], "a new HEAD takes the committed text as its baseline");
    assert!(s.state().markers(tab).unwrap().baseline().unwrap().label().starts_with("HEAD (main · "));

    // Undo after the commit: the old text differs from the new HEAD.
    s.dispatch(Command::Undo { tab });
    settle_repo(&mut s, |s| s.state().markers(tab).is_some_and(|m| !m.pending() && m.change(0).is_some()));
    assert_eq!(gutter(&s, tab, 3), ["M ", "  ", "  "]);
    assert_eq!(source_text(s.state().current_source().unwrap()), "one\ntwo\nthree\n");
    assert_eq!(std::fs::read_to_string(r.path().join("a.txt")).unwrap(), "ONE\ntwo\nthree\n", "undo changes no disk bytes");
}

#[test]
fn new_and_untracked_files_name_their_own_baseline_instead_of_a_commit() {
    use lenz_content::git::testing::{commit_all, git, repo, write as put};
    let d = tempfile::tempdir().unwrap();
    let r = repo();
    put(r.path(), "tracked.txt", "base\n");
    commit_all(r.path(), "base");
    put(r.path(), "new.txt", "one\ntwo\n");
    put(r.path(), "staged.txt", "staged\n");
    git(r.path(), &["add", "staged.txt"]);
    let mut s = session(&d.path().join("state"));
    s.dispatch(Command::OpenRoot(r.path().to_path_buf()));

    // An untracked file: every line is new to Git, and the display says so.
    open(&mut s, Command::OpenRetained(r.path().join("new.txt")), "new.txt");
    let untracked = s.state().active().unwrap().id();
    settle_repo(&mut s, |s| s.state().markers(untracked).is_some_and(|m| !m.pending()));
    assert_eq!(baseline(&s, untracked), Some(BaselineKind::New));
    assert_eq!(s.state().markers(untracked).unwrap().baseline().unwrap().label(), "new file (untracked)");
    assert_eq!(gutter(&s, untracked, 2), ["A ", "A "]);
    assert_eq!(s.state().tab_status(untracked), Some(lenz_content::branches::FileStatus::Untracked));
    // Saving a new file does not invent a committed baseline: Git still has no version at this path.
    edit_at(&mut s, 0..0, "zero\n");
    s.dispatch(Command::Save { tab: untracked });
    settle_repo(&mut s, |s| !s.state().tab_dirty(untracked) && s.state().markers(untracked).is_some_and(|m| !m.pending()));
    assert_eq!(s.state().markers(untracked).unwrap().baseline().unwrap().label(), "new file (untracked)");
    assert_eq!(gutter(&s, untracked, 3), ["A ", "A ", "A "]);

    // A file added to the index: new to Git too, named as added rather than as a modification.
    open(&mut s, Command::OpenRetained(r.path().join("staged.txt")), "staged.txt");
    let staged = s.state().active().unwrap().id();
    settle_repo(&mut s, |s| s.state().markers(staged).is_some_and(|m| !m.pending()));
    assert_eq!(s.state().markers(staged).unwrap().baseline().unwrap().label(), "new file (added to Git)");
    assert_eq!(gutter(&s, staged, 1), ["A "]);

    // A tracked file is HEAD's, changed or not.
    open(&mut s, Command::OpenRetained(r.path().join("tracked.txt")), "tracked.txt");
    let tracked = s.state().active().unwrap().id();
    settle_repo(&mut s, |s| s.state().markers(tracked).is_some_and(|m| !m.pending()));
    assert_eq!(baseline(&s, tracked), Some(BaselineKind::Head));
    assert_eq!(gutter(&s, tracked, 1), ["  "]);
}

#[test]
fn a_draft_that_keeps_changing_is_compared_once_it_rests_not_once_a_keystroke() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "note.txt", b"one\n");
    let mut s = session(&d.path().join("state"));
    open(&mut s, Command::OpenRetained(path.clone()), "note.txt");
    let tab = s.state().active().unwrap().id();
    settle_markers(&mut s, |s| s.state().markers(tab).is_some_and(|m| !m.pending()));
    let before = s.marker_requests();
    for _ in 0..5 {
        edit_at(&mut s, 0..0, "x");
        while let Ok(c) = s.completions().try_recv() { s.dispatch(c); }
    }
    assert_eq!(s.marker_requests(), before, "typing starts no comparison of its own");
    assert!(s.state().markers(tab).unwrap().pending(), "the markers name themselves as catching up");
    s.dispatch(Completion::MarkersDue);
    while let Ok(c) = s.completions().try_recv() { s.dispatch(c); }
    settle_markers(&mut s, |s| s.state().markers(tab).is_some_and(|m| !m.pending()));
    assert_eq!(s.marker_requests(), before + 1, "one comparison for the settled text");
    assert_eq!(gutter(&s, tab, 1), ["M "]);
    assert_eq!(source_text(s.state().current_source().unwrap()), "xxxxxone\n");
    assert_eq!(std::fs::read(&path).unwrap(), b"one\n");
}

#[test]
fn inspecting_and_reverting_insert_delete_replace_and_final_endings_preserves_exact_bytes_and_undo() {
    for (before,after,line) in [
        ("one\r\nλ two\r\nthree\r\n","one\r\nλ TWO\r\nthree\r\n",1),
        ("one\ntwo\n","new\none\ntwo\n",0),
        ("one\ntwo\nthree\n","one\nthree\n",1),
        ("one\ntwo\n","one\n",1),
        ("one\ntwo","one",0),
        ("one","one\ntwo",1),
        ("","λ\r\n",0),
    ] {
        let dir=tempfile::tempdir().unwrap();let path=write(dir.path(),"note.txt",before.as_bytes());let mut s=session(&dir.path().join("state"));
        open(&mut s,Command::OpenRetained(path.clone()),"note.txt");s.enable_text_input_for_test();let tab=s.state().active().unwrap().id();
        edit_at(&mut s,0..before.len(),after);
        settle_markers(&mut s,|s|s.state().markers(tab).is_some_and(|m|!m.pending()));
        s.dispatch(Command::InspectChangeAt {tab,line});
        assert!(s.inspected_change_text().unwrap_or_else(||panic!("No inspection: {before:?} -> {after:?}; line {line}; {}",s.status_details())).contains("last saved"),"{before:?} -> {after:?}");
        s.dispatch(Command::RevertChange);
        assert_eq!(source_text(s.state().current_source().unwrap()),before,"{after:?}");
        assert_eq!(std::fs::read(&path).unwrap(),before.as_bytes());
        s.dispatch(Command::Undo {tab});assert_eq!(source_text(s.state().current_source().unwrap()),after);
        s.dispatch(Command::Redo {tab});assert_eq!(source_text(s.state().current_source().unwrap()),before);
    }
}

#[test]
fn change_inspection_rejects_edits_and_baseline_refresh_before_revert() {
    let d=tempfile::tempdir().unwrap();let path=write(d.path(),"note.txt",b"one\ntwo\n");let mut s=session(&d.path().join("state"));
    open(&mut s,Command::OpenRetained(path.clone()),"note.txt");s.enable_text_input_for_test();let tab=s.state().active().unwrap().id();
    edit_at(&mut s,0..3,"ONE");settle_markers(&mut s,|s|s.state().markers(tab).is_some_and(|m|!m.pending()));
    s.dispatch(Command::InspectChangeAt{tab,line:0});assert!(s.inspected_change_text().is_some());
    edit_at(&mut s,4..7,"TWO");s.dispatch(Command::RevertChange);
    assert_eq!(source_text(s.state().current_source().unwrap()),"ONE\nTWO\n");assert!(s.inspected_change_text().is_none());
    settle_markers(&mut s,|s|s.state().markers(tab).is_some_and(|m|!m.pending()));
    s.dispatch(Command::InspectChangeAt{tab,line:0});s.markers_moved();s.dispatch(Command::RevertChange);
    assert_eq!(source_text(s.state().current_source().unwrap()),"ONE\nTWO\n");assert!(s.inspected_change_text().is_none());
    assert_eq!(std::fs::read(path).unwrap(),b"one\ntwo\n");
}

#[test]
fn tracked_change_inspector_uses_head_and_comparison_shares_draft_history_without_touching_the_index() {
    use lenz_content::git::testing::{commit_all,git,repo,write as put};
    let r=repo();put(r.path(),"a.txt","one\ntwo\n");commit_all(r.path(),"base");
    put(r.path(),"a.txt","staged\ntwo\n");git(r.path(),&["add","a.txt"]);
    let index=std::fs::read(r.path().join(".git/index")).unwrap();
    let d=tempfile::tempdir().unwrap();let mut s=session(d.path());s.dispatch(Command::OpenRoot(r.path().to_path_buf()));
    open(&mut s,Command::OpenRetained(r.path().join("a.txt")),"a.txt");s.enable_text_input_for_test();let tab=s.state().active().unwrap().id();
    edit_at(&mut s,0..6,"draft");settle_repo(&mut s,|s|s.state().markers(tab).is_some_and(|m|!m.pending()));
    let buffer=s.state().current_source().unwrap().key().buffer;
    s.dispatch(Command::InspectChangeAt{tab,line:0});let text=s.inspected_change_text().unwrap();assert!(text.contains("HEAD (")&&text.contains("one\n"));
    s.dispatch(Command::ShowChangeDiff);
    let c=s.state().saved_comparison().unwrap();assert_eq!(c.kind(),ComparisonKind::Marked);assert!(c.baseline_label().starts_with("HEAD ("));let current=c.current_tab();let saved=c.saved_tab();
    assert_eq!(s.state().source(current).unwrap().key().buffer,buffer);assert_eq!(source_text(s.state().source(saved).unwrap()),"one\ntwo\n");
    edit_at(&mut s,0..5,"comparison");s.dispatch(Command::Undo{tab:current});assert_eq!(source_text(s.state().source(current).unwrap()),"draft\ntwo\n");
    s.dispatch(Command::CloseSavedComparison);assert_eq!(s.state().active().unwrap().id(),tab);assert_eq!(source_text(s.state().current_source().unwrap()),"draft\ntwo\n");
    settle_repo(&mut s,|s|s.state().markers(tab).is_some_and(|m|!m.pending()));s.dispatch(Command::InspectChangeAt{tab,line:0});s.dispatch(Command::RevertChange);settle(&mut s,|s|s.reverting_change.is_none());
    assert_eq!(source_text(s.state().current_source().unwrap()),"one\ntwo\n");assert_eq!(std::fs::read(r.path().join("a.txt")).unwrap(),b"staged\ntwo\n");
    assert_eq!(std::fs::read(r.path().join(".git/index")).unwrap(),index);
    s.dispatch(Command::Undo{tab});assert_eq!(source_text(s.state().current_source().unwrap()),"draft\ntwo\n");s.dispatch(Command::Save{tab});
    settle(&mut s,|s|!s.state().tab_dirty(tab));
    assert_eq!(std::fs::read(r.path().join("a.txt")).unwrap(),b"draft\ntwo\n");assert_eq!(std::fs::read(r.path().join(".git/index")).unwrap(),index);
}

#[test]
fn an_untracked_change_inspects_an_empty_baseline_and_reverts_without_removing_its_disk_file() {
    use lenz_content::git::testing::{repo,commit_all,write as put};
    let r=repo();put(r.path(),"keep.txt","keep\n");commit_all(r.path(),"base");put(r.path(),"new.txt","new\r\nλ\r\n");
    let d=tempfile::tempdir().unwrap();let mut s=session(d.path());s.dispatch(Command::OpenRoot(r.path().to_path_buf()));open(&mut s,Command::OpenRetained(r.path().join("new.txt")),"new.txt");s.enable_text_input_for_test();let tab=s.state().active().unwrap().id();
    settle_repo(&mut s,|s|s.state().markers(tab).is_some_and(|m|!m.pending()));
    s.dispatch(Command::InspectChangeAt{tab,line:0});let text=s.inspected_change_text().unwrap();assert!(text.contains("new file (untracked)")&&text.contains("[empty]"));
    s.dispatch(Command::RevertChange);settle(&mut s,|s|s.reverting_change.is_none());assert_eq!(source_text(s.state().current_source().unwrap()),"");assert_eq!(std::fs::read(r.path().join("new.txt")).unwrap(),"new\r\nλ\r\n".as_bytes());
    s.dispatch(Command::Undo{tab});assert_eq!(source_text(s.state().current_source().unwrap()),"new\r\nλ\r\n");
}

#[test]
fn an_external_head_change_is_rejected_before_repository_polling_observes_it() {
    use lenz_content::git::testing::{repo,commit_all,write as put};
    let r=repo();put(r.path(),"a.txt","one\ntwo\n");commit_all(r.path(),"base");let d=tempfile::tempdir().unwrap();let mut s=session(d.path());s.dispatch(Command::OpenRoot(r.path().to_path_buf()));open(&mut s,Command::OpenRetained(r.path().join("a.txt")),"a.txt");s.enable_text_input_for_test();let tab=s.state().active().unwrap().id();
    edit_at(&mut s,0..3,"draft");settle_repo(&mut s,|s|s.state().markers(tab).is_some_and(|m|!m.pending()));s.dispatch(Command::InspectChangeAt{tab,line:0});assert!(s.inspected_change_text().is_some());
    put(r.path(),"other.txt","new HEAD\n");commit_all(r.path(),"external commit");
    s.dispatch(Command::RevertChange);settle(&mut s,|s|s.reverting_change.is_none());assert_eq!(source_text(s.state().current_source().unwrap()),"draft\ntwo\n");assert!(s.state().notices().iter().any(|n|n.contains("baseline moved")));
}
