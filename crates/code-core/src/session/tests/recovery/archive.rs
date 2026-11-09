//! Archive drafts survive restart independently of their enclosing binary container.
use super::*;
use crate::archive::Location;

fn location(path: &Path, members: &[&str]) -> Location {
    Location::new(path.to_path_buf(), members.iter().map(|s| (*s).to_owned()).collect()).unwrap()
}
fn member_open(s: &mut Session, location: &Location) {
    s.dispatch(Command::OpenArchiveEntry(location.clone()));
    settle(s, |s| s.state().current_source().is_some_and(|d| d.source_meta().archive() == Some(location)) && s.state().loading().is_none());
}
fn restore(s: &mut Session) {
    let prompt = s.recovery_prompt().unwrap();
    assert_eq!(prompt.disk, Disk::Unchecked);
    s.dispatch(Command::ChooseRecoveredDraft { ticket: prompt.ticket, choice: Choice::Restore });
    settle(s, |s| s.recovered_drafts().is_empty() && s.state().current_source().is_some());
}

#[test]
fn nested_archive_recovery_keeps_encoding_baseline_and_siblings_then_saves_normally() {
    let dir = tempfile::tempdir().unwrap();
    let inner = lenz_archive::testing::tar(lenz_archive::Compression::Gzip, &[("folder/café.txt", b"\xff\xfes\0a\0v\0e\0d\0\r\0\n\0"), ("untouched", b"inner sibling")]);
    let original = lenz_archive::testing::zip(&[("nested.tgz", &inner), ("outer", b"outer sibling")]);
    let path = write(dir.path(), "work.zip", &original);
    let loc = location(&path, &["nested.tgz", "folder/café.txt"]);
    {
        let mut s = session(dir.path());
        member_open(&mut s, &loc);
        edit(&mut s, "draft λ\r\n");
        s.flush_recovery_for_test();
    }
    assert_eq!(std::fs::read(&path).unwrap(), original);
    let mut s = session(dir.path());
    assert_eq!(s.recovered_drafts().len(), 1);
    assert!(matches!(s.recovered_drafts()[0].record().key(), crate::recovery::Key::Archive { location } if location == &loc));
    restore(&mut s);
    assert_eq!(text(&s), "draft λ\r\n");
    assert_eq!(active_view(&s).snapshot().source_meta().original_encoding(), "UTF-16LE");
    assert_eq!(s.disabled(CommandId::Save), None);
    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Save { tab });
    settle(&mut s, |s| !s.state().tab_dirty(tab));
    assert_eq!(&**crate::archive::read_location(&loc, 99).unwrap().text(), "draft λ\r\n");
    for (members, expected) in [(&["outer"][..], "outer sibling"), (&["nested.tgz", "untouched"][..], "inner sibling")] {
        assert_eq!(&**crate::archive::read_location(&location(&path, members), 100).unwrap().text(), expected);
    }
    s.flush_recovery_for_test();
    drop(s);
    assert!(session(dir.path()).recovered_drafts().is_empty());
}

#[test]
fn changed_or_missing_archive_member_restores_editable_draft_for_standalone_export() {
    for missing in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "work.zip", &lenz_archive::testing::zip(&[("a.txt", b"saved"), ("other", b"original sibling")]));
        let loc = location(&path, &["a.txt"]);
        {
            let mut s = session(dir.path());
            member_open(&mut s, &loc);
            edit(&mut s, "recovered work");
            s.flush_recovery_for_test();
        }
        if missing { std::fs::remove_file(&path).unwrap(); }
        else { std::fs::write(&path, lenz_archive::testing::zip(&[("a.txt", b"external"), ("other", b"external sibling")])).unwrap(); }
        let before = std::fs::read(&path).ok();
        let mut s = session(dir.path());
        restore(&mut s);
        assert_eq!(text(&s), "recovered work");
        assert!(s.disabled(CommandId::Save).is_some());
        edit(&mut s, "recovered and edited");
        assert_eq!(text(&s), "recovered and edited");
        let export = dir.path().join("export.txt");
        let tab = s.state().active().unwrap().id();
        s.dispatch(Command::SaveAs { tab, path: export.clone() });
        settle(&mut s, |s| !s.state().view(tab).unwrap().save_status().busy());
        assert_eq!(std::fs::read_to_string(export).unwrap(), "recovered and edited");
        assert_eq!(std::fs::read(&path).ok(), before);
    }
}

#[test]
fn discard_while_archive_restore_is_queued_drops_its_late_completion() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "work.zip", &lenz_archive::testing::zip(&[("a.txt", b"saved")]));
    let loc = location(&path, &["a.txt"]);
    {
        let mut s = session(dir.path());
        member_open(&mut s, &loc);
        edit(&mut s, "draft");
        s.flush_recovery_for_test();
    }
    let mut s = session(dir.path());
    let (started, entered) = std::sync::mpsc::channel();
    let gates: Vec<_> = (0..crate::jobs::MAX_JOBS).map(|_| {
        let (release, wait) = std::sync::mpsc::channel();
        let started = started.clone();
        s.jobs.submit(Priority::Visible, move || { started.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(10)); });
        release
    }).collect();
    for _ in &gates { entered.recv_timeout(Duration::from_secs(5)).unwrap(); }
    let ticket = s.recovery_prompt().unwrap().ticket;
    s.dispatch(Command::ChooseRecoveredDraft { ticket, choice: Choice::Restore });
    assert!(s.recovery_prompt().unwrap().restore_disabled.is_some());
    s.dispatch(Command::ChooseRecoveredDraft { ticket, choice: Choice::Discard });
    for gate in gates { gate.send(()).unwrap(); }
    let doc = crate::archive::read_location(&loc, 900).unwrap();
    s.complete(Completion::ArchiveRecovered { ticket, result: Ok(doc) });
    assert!(s.recovered_drafts().is_empty());
    assert!(s.state().current_source().is_none());
    assert_eq!(&**crate::archive::read_location(&loc, 901).unwrap().text(), "saved");
}

#[test]
fn project_restart_restores_archive_browser_with_or_without_an_open_member() {
    for open_member in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let path = write(&project, "work.zip", &lenz_archive::testing::zip(&[("folder/a.txt", b"saved")]));
        let root = location(&path, &[]);
        {
            let mut s = session(dir.path());
            s.dispatch(Command::OpenRoot(project.clone()));
            settle(&mut s, |s| scanned(s) && !s.restore_workspace);
            s.dispatch(Command::OpenRetained(path.clone()));
            settle(&mut s, |s| s.state().archive_tree().roots() == [root.clone()]);
            if open_member { member_open(&mut s, &location(&path, &["folder/a.txt"])); }
            s.dispatch(Command::SaveWorkspace);
            s.workspace_saved().recv_timeout(Duration::from_secs(5)).unwrap();
        }
        let mut s = session(dir.path());
        s.dispatch(Command::OpenRoot(project));
        settle(&mut s, |s| !s.restore_workspace && s.state().archive_tree().roots() == [root.clone()]);
        if open_member {
            settle(&mut s, |s| s.state().current_source().is_some());
            assert_eq!(text(&s), "saved");
        } else { assert!(s.state().tabs().is_empty()); }
    }
}
