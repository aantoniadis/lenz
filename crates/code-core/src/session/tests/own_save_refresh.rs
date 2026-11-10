use super::*;

fn insert(s: &mut Session, text: &str) {
    let tab = s.state().active().unwrap().id();
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit {
        tab, expected,
        edits: vec![crate::text::TextEdit { range: 0..0, text: text.into() }],
        kind: crate::text::EditKind::Paste,
    });
}

/// Drive the real writer while retaining filesystem checks for deliberate delivery ordering.
fn save_without_delivering_file_checks(s: &mut Session, tab: TabId) {
    let rx = s.completions();
    s.dispatch(Command::Save { tab });
    let until = std::time::Instant::now() + Duration::from_secs(10);
    while s.state().tab_dirty(tab) || s.state().view(tab).unwrap().save_status().busy() {
        assert!(std::time::Instant::now() < until, "{}", s.status_details());
        let completion = rx.recv_blocking_timeout().unwrap();
        if !matches!(completion, Completion::Changed(_) | Completion::Refreshed { .. }) {
            s.dispatch(completion);
        }
    }
}

fn reject_obsolete_own_save_observation(read_before_write: bool) {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "source.txt", b"saved\r\n");
    let mut s = session(&dir.path().join("session"));
    open(&mut s, Command::OpenRetained(path.clone()), "source.txt");
    // Registration notifications are irrelevant to this explicitly ordered disk-check result.
    s.watcher = Watcher::new(|_| true);
    insert(&mut s, "first ");
    let tab = s.state().active().unwrap().id();
    let buffer = s.state().current_source().unwrap().key().buffer;
    let old_stamp = s.state().active().unwrap().stamp;
    let old_baseline = s.state.sources.buffers.get(&buffer).unwrap().draft.as_ref().unwrap().baseline.clone();
    let checked_baseline = s.refresh_baseline(tab);
    // The state installed by check() before its worker reads the file.
    let token = s.next_refresh();
    s.tab_mut(tab).unwrap().refresh = token;
    let before_write = read_before_write.then(|| super::super::check(&path, old_stamp, Some(&old_baseline), &s.versions));

    save_without_delivering_file_checks(&mut s, tab);
    assert_eq!(std::fs::read(&path).unwrap(), b"first saved\r\n");
    assert!(!s.state().active().unwrap().stale_on_disk());
    let confirmed_stamp = s.state().active().unwrap().stamp;
    // Either the old check read before the write, or it reads our atomic write
    // against the baseline captured before Save; both answers arrive afterward.
    let late = before_write.unwrap_or_else(|| super::super::check(&path, old_stamp, Some(&old_baseline), &s.versions));
    assert!(matches!((&late, read_before_write), (Refresh::Same(_), true) | (Refresh::Changed(..), false)));
    s.dispatch(Completion::Refreshed { tab, token, baseline: checked_baseline, result: late });

    assert_eq!(s.state().active().unwrap().stamp, confirmed_stamp,
        "an obsolete Same result must not replace the confirmed Save's new disk stamp");
    assert!(!s.state().active().unwrap().stale_on_disk(),
        "an obsolete pre-Save check must not turn our confirmed Save into an external-change refusal");
    insert(&mut s, "next ");
    s.dispatch(Command::CloseTab(tab));
    let prompt = s.close_prompt().unwrap();
    assert_eq!(prompt.save_disabled, None, "Save-and-close remains available for the next draft");
    s.dispatch(Command::ChooseClose {
        ticket: prompt.ticket, buffer: prompt.buffer, choice: crate::close::Choice::Save,
    });
    settle(&mut s, |s| s.state().active().is_none());
    assert_eq!(std::fs::read(path).unwrap(), b"next first saved\r\n");
}

#[test]
fn a_pre_save_changed_result_cannot_mark_the_confirmed_file_stale() {
    reject_obsolete_own_save_observation(false);
}

#[test]
fn a_pre_save_same_result_cannot_restore_the_old_disk_stamp() {
    reject_obsolete_own_save_observation(true);
}

#[test]
fn a_current_disk_check_after_save_still_detects_real_external_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "source.txt", b"saved\r\n");
    let mut s = session(&dir.path().join("session"));
    open(&mut s, Command::OpenRetained(path.clone()), "source.txt");
    s.watcher = Watcher::new(|_| true);
    insert(&mut s, "first ");
    let tab = s.state().active().unwrap().id();
    save_without_delivering_file_checks(&mut s, tab);
    insert(&mut s, "next ");
    std::fs::write(&path, b"written elsewhere\r\n").unwrap();
    s.check(tab, None);
    let request = s.state().active().unwrap().refresh;
    let until = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        assert!(std::time::Instant::now() < until, "the current disk check must finish");
        let completion = s.completions().recv_blocking_timeout().unwrap();
        if matches!(&completion, Completion::Refreshed { tab: found, token, .. } if *found == tab && *token == request) {
            s.dispatch(completion);
            break;
        }
        if !matches!(completion, Completion::Changed(_) | Completion::Refreshed { .. }) {
            s.dispatch(completion);
        }
    }
    assert!(s.state().active().unwrap().stale_on_disk());
    assert_eq!(source_text(s.state().current_source().unwrap()), "next first saved\r\n");
    s.dispatch(Command::CloseTab(tab));
    assert!(s.close_prompt().unwrap().save_disabled.unwrap().contains("file changed on disk"));
    assert_eq!(std::fs::read(path).unwrap(), b"written elsewhere\r\n");
}

#[test]
fn an_external_write_after_save_is_detected_when_an_obsolete_check_is_retried() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "source.txt", b"saved\r\n");
    let mut s = session(&dir.path().join("session"));
    open(&mut s, Command::OpenRetained(path.clone()), "source.txt");
    s.watcher = Watcher::new(|_| true);
    insert(&mut s, "first ");
    let tab = s.state().active().unwrap().id();
    let buffer = s.state().current_source().unwrap().key().buffer;
    let old_stamp = s.state().active().unwrap().stamp;
    let old_baseline = s.state.sources.buffers.get(&buffer).unwrap().draft.as_ref().unwrap().baseline.clone();
    let checked_baseline = s.refresh_baseline(tab);
    let token = s.next_refresh();
    s.tab_mut(tab).unwrap().refresh = token;
    save_without_delivering_file_checks(&mut s, tab);
    let late = super::super::check(&path, old_stamp, Some(&old_baseline), &s.versions);
    assert!(matches!(late, Refresh::Changed(..)));
    insert(&mut s, "next ");
    std::fs::write(&path, b"written after our Save\r\n").unwrap();

    s.dispatch(Completion::Refreshed { tab, token, baseline: checked_baseline, result: late });
    settle(&mut s, |s| s.state().active().unwrap().stale_on_disk());

    assert_eq!(source_text(s.state().current_source().unwrap()), "next first saved\r\n");
    assert_eq!(std::fs::read(&path).unwrap(), b"written after our Save\r\n");
    s.dispatch(Command::CloseTab(tab));
    assert!(s.close_prompt().unwrap().save_disabled.unwrap().contains("file changed on disk"));
}
