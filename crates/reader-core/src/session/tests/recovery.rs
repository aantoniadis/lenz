//! Recovered drafts through the Session command seam: private records, restart offers and explicit Discard.
use super::*;
use crate::close::Choice as CloseChoice;
use crate::recovery::{Choice, Disk};
use super::super::recovery_work::RESTORE_DIRTY;

fn edit(s: &mut Session, text: &str) {
    let tab = s.state().active().unwrap().id();
    let expected = s.state().current_source().unwrap().key();
    let len = s.state().current_source().unwrap().len_bytes();
    s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..len, text: text.into() }], kind: crate::text::EditKind::Replace });
}

fn text(s: &Session) -> String { source_text(s.state().current_source().unwrap()) }

#[test]
fn an_interrupted_draft_is_offered_at_the_next_start_and_restored_against_its_baseline() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "a.txt", b"saved\r\n");
    {
        let mut s = session(d.path());
        open(&mut s, Command::OpenRetained(path.clone()), "a.txt");
        edit(&mut s, "draft λ\r\n");
        s.flush_recovery_for_test();
        assert_eq!(std::fs::read(&path).unwrap(), b"saved\r\n", "recovery never writes the original file");
    }
    let mut s = session(d.path());
    let offers = s.recovered_drafts();
    assert_eq!(offers.len(), 1);
    assert_eq!((offers[0].record().title(), offers[0].disk()), ("a.txt", Disk::Matches));
    let prompt = s.recovery_prompt().unwrap();
    assert_eq!((prompt.label.as_str(), prompt.remaining, prompt.disabled(Choice::Restore)), ("a.txt", 1, None));
    assert!(prompt.text().contains("still matches this draft's confirmed baseline"), "{}", prompt.text());
    assert!(!prompt.text().contains("draft λ"), "the dialog never shows source text");
    let ticket = prompt.ticket;
    assert!(s.disabled(CommandId::RecoverDrafts).is_none());
    s.dispatch(Command::ChooseRecoveredDraft { ticket, choice: Choice::Restore });
    let tab = s.state().active().unwrap().id();
    assert!(s.state().tabs.iter().find(|t| t.id == tab).unwrap().recovered());
    assert_eq!(s.state().tabs.iter().find(|t| t.id == tab).unwrap().title(), "a.txt");
    assert_eq!(text(&s), "draft λ\r\n");
    assert_eq!(active_view(&s).save_status(), &crate::save::Status::Ready);
    assert!(s.state().tab_dirty(tab));
    assert!(s.recovered_drafts().is_empty() && s.recovery_prompt().is_none());
    assert_eq!(std::fs::read(&path).unwrap(), b"saved\r\n", "only Restore reopens the work; disk is unchanged");
    // The restored draft is the ordinary draft of the file: Save confirms it and it is no longer recovered.
    s.dispatch(Command::Save { tab });
    settle(&mut s, |s| !s.state().tab_dirty(tab));
    assert_eq!(std::fs::read(&path).unwrap(), "draft λ\r\n".as_bytes());
    assert!(!s.state().tabs.iter().find(|t| t.id == tab).unwrap().recovered());
    s.flush_recovery_for_test();
    drop(s);
    let restarted = session(d.path());
    assert!(restarted.recovered_drafts().is_empty(), "saved work has nothing to recover");
}

#[test]
fn a_new_file_draft_is_recovered_and_saved_as() {
    let d = tempfile::tempdir().unwrap();
    {
        let mut s = session(d.path());
        s.dispatch(Command::NewDocument);
        edit(&mut s, "new λ\n");
        s.flush_recovery_for_test();
    }
    let mut s = session(d.path());
    let offer = &s.recovered_drafts()[0];
    assert_eq!((offer.record().title(), offer.disk()), ("Untitled-1", Disk::Unsaved));
    let prompt = s.recovery_prompt().unwrap();
    assert!(prompt.text().contains("New File document was never saved"), "{}", prompt.text());
    s.dispatch(Command::ChooseRecoveredDraft { ticket: prompt.ticket, choice: Choice::Restore });
    let tab = s.state().active().unwrap().id();
    assert!(s.state().tabs.iter().find(|t| t.id == tab).unwrap().recovered());
    assert_eq!(text(&s), "new λ\n");
    assert!(s.state().active().unwrap().untitled());
    assert_eq!(s.disabled(CommandId::Save), None, "Save on a New File asks where to write it");
    assert_eq!(s.action(CommandId::Save), crate::actions::CatalogAction::Interface(crate::actions::InterfaceAction::SaveAs));
    let target = d.path().join("chosen.txt");
    s.dispatch(Command::SaveAs { tab, path: target.clone() });
    settle(&mut s, |s| s.state().current_source().is_some_and(|source| source.source() == target));
    assert_eq!(std::fs::read(&target).unwrap(), "new λ\n".as_bytes());
    s.flush_recovery_for_test();
    drop(s);
    let restarted = session(d.path());
    assert!(restarted.recovered_drafts().is_empty(), "the New File record is superseded by its saved file");
}

#[test]
fn explicit_discard_keeps_discarded_work_from_returning() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "a.txt", b"saved\n");
    {
        let mut s = session(d.path());
        open(&mut s, Command::OpenRetained(path.clone()), "a.txt");
        edit(&mut s, "discarded draft\n");
        s.flush_recovery_for_test();
        let tab = s.state().active().unwrap().id();
        s.dispatch(Command::CloseTab(tab));
        let prompt = s.close_prompt().unwrap();
        s.dispatch(Command::ChooseClose { ticket: prompt.ticket, buffer: prompt.buffer, choice: CloseChoice::Discard });
        assert!(s.state().active().is_none());
        s.flush_recovery_for_test();
        assert_eq!(std::fs::read(&path).unwrap(), b"saved\n");
    }
    let mut restarted = session(d.path());
    assert!(restarted.recovered_drafts().is_empty(), "discarded work is not offered again");
    assert!(restarted.recovery_prompt().is_none());
    assert_eq!(restarted.disabled(CommandId::RecoverDrafts), Some("No recovered drafts are offered"));
    restarted.dispatch(Command::ShowRecoveredDrafts);
    assert!(restarted.state().notices().iter().any(|m| m.contains("No recovered drafts are offered")));
}

#[test]
fn an_edit_discarded_by_the_next_edit_never_returns() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "a.txt", b"saved\n");
    {
        let mut s = session(d.path());
        open(&mut s, Command::OpenRetained(path.clone()), "a.txt");
        edit(&mut s, "first draft\n");
        // The save/discard race: a record is queued, then the draft is discarded before it is written.
        let tab = s.state().active().unwrap().id();
        s.dispatch(Command::CloseTab(tab));
        let prompt = s.close_prompt().unwrap();
        s.dispatch(Command::ChooseClose { ticket: prompt.ticket, buffer: prompt.buffer, choice: CloseChoice::Discard });
        s.flush_recovery_for_test();
    }
    assert!(session(d.path()).recovered_drafts().is_empty());
}

#[test]
fn undo_back_to_the_baseline_leaves_nothing_to_recover() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "a.txt", b"saved\n");
    {
        let mut s = session(d.path());
        open(&mut s, Command::OpenRetained(path.clone()), "a.txt");
        edit(&mut s, "draft\n");
        let tab = s.state().active().unwrap().id();
        s.dispatch(Command::Undo { tab });
        assert!(!s.state().tab_dirty(tab));
        s.flush_recovery_for_test();
    }
    assert!(session(d.path()).recovered_drafts().is_empty());
}

#[test]
fn a_changed_file_keeps_save_closed_until_the_current_file_is_rechecked() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "a.txt", b"saved\n");
    {
        let mut s = session(d.path());
        open(&mut s, Command::OpenRetained(path.clone()), "a.txt");
        edit(&mut s, "draft\n");
        s.flush_recovery_for_test();
    }
    std::fs::write(&path, b"other\n").unwrap();
    let mut s = session(d.path());
    let prompt = s.recovery_prompt().unwrap();
    assert_eq!(s.recovered_drafts()[0].disk(), Disk::Differs);
    assert!(prompt.text().contains("has changed since this draft's confirmed baseline"), "{}", prompt.text());
    s.dispatch(Command::ChooseRecoveredDraft { ticket: prompt.ticket, choice: Choice::Restore });
    let tab = s.state().active().unwrap().id();
    assert!(s.state().tabs.iter().find(|t| t.id == tab).unwrap().recovered());
    assert_eq!(text(&s), "draft\n");
    assert!(matches!(active_view(&s).save_status(), crate::save::Status::Rejected(why) if why.contains("differs from its confirmed baseline")), "{:?}", active_view(&s).save_status());
    assert_eq!(s.disabled(CommandId::Save), Some("This source has no confirmed write target; reopen the file before saving"));
    assert_eq!(s.disabled(CommandId::RecheckSave), None, "the current file can still be rechecked");
    assert_eq!(std::fs::read(&path).unwrap(), b"other\n");
}

#[test]
fn a_file_with_an_undecided_draft_gains_no_live_draft_that_restoring_would_conflict_with() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "a.txt", b"saved\n");
    {
        let mut s = session(d.path());
        open(&mut s, Command::OpenRetained(path.clone()), "a.txt");
        edit(&mut s, "crashed draft\n");
        s.flush_recovery_for_test();
    }
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path.clone()), "a.txt");
    edit(&mut s, "live draft\n");
    assert_eq!(text(&s), "saved\n", "the edit would have replaced the recovered draft");
    assert!(s.state().notices().is_empty() || !s.state().notices().iter().any(|m| m == RESTORE_DIRTY));
    let prompt = s.recovery_prompt().unwrap();
    assert_eq!(prompt.disabled(Choice::Restore), None);
    s.dispatch(Command::ChooseRecoveredDraft { ticket: prompt.ticket, choice: Choice::Restore });
    assert_eq!(text(&s), "crashed draft\n");
    assert!(s.recovered_drafts().is_empty());
}

#[test]
fn later_keeps_the_offer_and_the_command_reopens_the_dialog() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "a.txt", b"saved\n");
    {
        let mut s = session(d.path());
        open(&mut s, Command::OpenRetained(path.clone()), "a.txt");
        edit(&mut s, "draft\n");
        s.flush_recovery_for_test();
    }
    let mut s = session(d.path());
    let ticket = s.recovery_prompt().unwrap().ticket;
    // The presets give recovery no chord: the File menu is its route, and menus say so.
    for preset in crate::keymap::KeymapPreset::ALL {
        s.dispatch(Command::SetKeymapPreset(preset));
        assert_eq!(s.key_hint(CommandId::RecoverDrafts), None, "{preset:?}");
        assert_eq!(s.action(CommandId::RecoverDrafts), crate::actions::CatalogAction::Command(Command::ShowRecoveredDrafts), "{preset:?}");
        assert_eq!(s.recovery_prompt().map(|p| p.ticket), Some(ticket), "a preset change does not decide the offer");
    }
    s.dispatch(Command::ChooseRecoveredDraft { ticket, choice: Choice::Later });
    assert!(s.recovery_prompt().is_none() && s.recovered_drafts().len() == 1);
    assert_eq!(s.disabled(CommandId::RecoverDrafts), None);
    s.dispatch(Command::ShowRecoveredDrafts);
    assert_eq!(s.recovery_prompt().unwrap().ticket, ticket);
    // A stale ticket decides nothing.
    s.dispatch(Command::ChooseRecoveredDraft { ticket: ticket + 99, choice: Choice::Discard });
    assert_eq!(s.recovered_drafts().len(), 1);
}

#[test]
fn a_missing_file_restores_as_a_draft_to_save_elsewhere() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "gone.txt", b"saved\n");
    {
        let mut s = session(d.path());
        open(&mut s, Command::OpenRetained(path.clone()), "gone.txt");
        edit(&mut s, "draft\n");
        s.flush_recovery_for_test();
    }
    std::fs::remove_file(&path).unwrap();
    let mut s = session(d.path());
    let prompt = s.recovery_prompt().unwrap();
    assert_eq!(s.recovered_drafts()[0].disk(), Disk::Missing);
    assert!(prompt.text().contains("no longer on disk"), "{}", prompt.text());
    s.dispatch(Command::ChooseRecoveredDraft { ticket: prompt.ticket, choice: Choice::Restore });
    assert_eq!(text(&s), "draft\n");
    assert_eq!(s.disabled(CommandId::Save), Some("This source has no confirmed write target; reopen the file before saving"));
    assert_eq!(s.disabled(CommandId::SaveAs), None);
}

#[test]
fn a_non_utf8_document_keeps_its_encoding_through_recovery() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "legacy.txt", b"caf\xe9\n");
    {
        let mut s = session(d.path());
        open(&mut s, Command::OpenRetained(path.clone()), "legacy.txt");
        assert_eq!(s.state().current_source().unwrap().encoding(), "windows-1252");
        edit(&mut s, "draft crème\n");
        s.flush_recovery_for_test();
    }
    let mut s = session(d.path());
    let prompt = s.recovery_prompt().unwrap();
    s.dispatch(Command::ChooseRecoveredDraft { ticket: prompt.ticket, choice: Choice::Restore });
    assert_eq!(text(&s), "draft crème\n");
    assert_eq!(s.state().current_source().unwrap().encoding(), "windows-1252", "the document's encoding survives recovery");
}

#[test]
fn a_dirty_draft_follows_its_moved_file_and_the_old_path_is_not_offered() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "before.txt", b"saved\n");
    let moved = d.path().join("after.txt");
    {
        let mut s = session(d.path());
        open(&mut s, Command::OpenRetained(path.clone()), "before.txt");
        edit(&mut s, "moved draft\n");
        s.flush_recovery_for_test();
        let tab = s.state().active().unwrap().id();
        let token = s.state().tabs.iter().find(|t| t.id == tab).unwrap().refresh;
        std::fs::rename(&path, &moved).unwrap();
        s.dispatch(Completion::Moved { tab, token, to: moved.clone(), stamp: Stamp::of(&moved), canonical: Some(moved.clone()) });
        s.flush_recovery_for_test();
    }
    let mut restarted = session(d.path());
    let offers = restarted.recovered_drafts();
    assert_eq!(offers.len(), 1, "one record, for the file's new path");
    assert_eq!((offers[0].record().title(), offers[0].disk()), ("after.txt", Disk::Matches));
    restarted.dispatch(Command::ChooseRecoveredDraft { ticket: offers[0].id(), choice: Choice::Restore });
    assert_eq!(text(&restarted), "moved draft\n");
}

/// Each tab's file, pane, place and recovered mark: what a restart must bring back.
fn arrangement(s: &Session) -> Vec<(PathBuf, u8, Option<Selection>, usize, bool)> {
    s.state.tabs.iter().map(|t| (t.source.clone(), t.pane, t.selection(), t.top_line, t.recovered())).collect()
}

/// Opens the project's last tabs: the recovery dialog waits until they are back.
fn reopen(s: &mut Session, root: &Path) {
    s.dispatch(Command::OpenRoot(root.to_owned()));
    assert!(s.recovery_prompt().is_none(), "the dialog waits for the project's last tabs");
    if !s.recovered_drafts().is_empty() { assert_eq!(s.disabled(CommandId::RecoverDrafts), Some(super::super::recovery_work::RESTORING)); }
    settle(s, |s| !s.restore_workspace && scanned(s) && s.state.shown(0).is_some_and(|t| t.loaded) && s.state.loading().is_none());
}

#[test]
fn an_interrupted_project_reopens_its_tabs_split_and_places_around_the_recovered_draft() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project");
    tree(&root, &["a.txt", "src/b.txt"]);
    std::fs::write(root.join("a.txt"), "alpha\n".repeat(80)).unwrap();
    let saved = "beta λ\n".repeat(80);
    std::fs::write(root.join("src/b.txt"), &saved).unwrap();
    let mut one = session(d.path());
    open_project(&mut one, &root, "a.txt");
    one.dispatch(Command::Pin(one.state.active.unwrap()));
    one.dispatch(Command::Scroll(30));
    one.dispatch(Command::OpenRetained(root.join("src/b.txt")));
    settle(&mut one, opened);
    one.dispatch(Command::Split(Split::Columns));
    settle(&mut one, |s| s.state.shown(1).is_some_and(|t| t.loaded));
    // The draft is edited in one pane; both panes show the same buffer.
    let tab = one.state.active().unwrap().id();
    let expected = one.state.current_source().unwrap().key();
    one.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..0, text: "draft ✓\n".into() }], kind: crate::text::EditKind::Replace });
    one.dispatch(Command::Select(Selection { anchor: 40, head: 45 }));
    one.dispatch(Command::Scroll(20));
    one.dispatch(Command::FocusPane(0));
    let before = arrangement(&one);
    let draft = text(&one);
    assert_eq!(one.state.tabs.iter().filter(|t| t.source.ends_with("src/b.txt")).count(), 2, "split shows the file in both panes");
    one.interrupt_for_test();
    assert_eq!(std::fs::read_to_string(root.join("src/b.txt")).unwrap(), saved, "recovery never writes the file");

    let mut two = session(d.path());
    assert_eq!(two.recovered_drafts().len(), 1);
    reopen(&mut two, &root);
    assert_eq!(two.state.split(), Some(Split::Columns));
    assert_eq!(arrangement(&two), before, "tabs, panes and places are those of the interrupted session");
    let b: Vec<TabId> = two.state.tabs.iter().filter(|t| t.source.ends_with("src/b.txt")).map(|t| t.id).collect();
    assert!(b.iter().all(|id| two.state.view(*id).is_none_or(|v| v.edit_disabled() == Some(super::super::recovery_work::UNDECIDED))),
        "the disk version is not edited while its recovered draft is undecided");
    let prompt = two.recovery_prompt().expect("offered once the tabs are back");
    assert_eq!(prompt.disk, Disk::Matches);
    two.dispatch(Command::ChooseRecoveredDraft { ticket: prompt.ticket, choice: Choice::Restore });
    settle(&mut two, |s| s.state.shown(0).is_some_and(|t| t.loaded) && s.state.shown(1).is_some_and(|t| t.loaded));
    let restored = arrangement(&two);
    assert_eq!(restored.iter().map(|(path, pane, selection, top, _)| (path.clone(), *pane, *selection, *top)).collect::<Vec<_>>(),
        before.iter().map(|(path, pane, selection, top, _)| (path.clone(), *pane, *selection, *top)).collect::<Vec<_>>(), "restoring keeps every tab where it was");
    assert!(restored.iter().all(|(path, _, _, _, recovered)| *recovered == path.ends_with("src/b.txt")), "{restored:?}");
    let buffers: std::collections::HashSet<_> = b.iter().map(|id| two.state.tabs.iter().find(|t| t.id == *id).unwrap().buffer).collect();
    assert_eq!(buffers.len(), 1, "both panes share the restored draft's buffer");
    for id in &b {
        let view = two.state.view(*id).unwrap();
        assert_eq!(source_text(view.snapshot()), draft);
        assert!(view.dirty() && view.edit_disabled().is_none());
    }
    assert_eq!(std::fs::read_to_string(root.join("src/b.txt")).unwrap(), saved);
}

#[test]
fn a_clean_shutdown_reopens_the_tabs_and_offers_no_saved_or_discarded_work() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project");
    tree(&root, &["a.txt", "b.txt"]);
    let mut one = session(d.path());
    open_project(&mut one, &root, "a.txt");
    one.dispatch(Command::Pin(one.state.active.unwrap()));
    edit(&mut one, "saved draft\n");
    let tab = one.state.active().unwrap().id();
    one.dispatch(Command::Save { tab });
    settle(&mut one, |s| !s.state().tab_dirty(tab));
    one.dispatch(Command::OpenRetained(root.join("b.txt")));
    settle(&mut one, opened);
    edit(&mut one, "discarded draft\n");
    one.flush_recovery_for_test();
    let b = one.state.active().unwrap().id();
    one.dispatch(Command::CloseTab(b));
    let prompt = one.close_prompt().unwrap();
    one.dispatch(Command::ChooseClose { ticket: prompt.ticket, buffer: prompt.buffer, choice: CloseChoice::Discard });
    settle(&mut one, |s| s.state.tabs.len() == 1);
    let before = arrangement(&one);
    one.flush_recovery_for_test();
    drop(one);
    let mut two = session(d.path());
    assert!(two.recovered_drafts().is_empty(), "saved and discarded work is not recovered");
    reopen(&mut two, &root);
    assert_eq!(arrangement(&two), before);
    assert!(two.recovery_prompt().is_none());
    assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "saved draft\n");
    assert_eq!(std::fs::read_to_string(root.join("b.txt")).unwrap(), "b.txt");
}

#[test]
fn a_postponed_recovered_draft_survives_attempted_edits_reload_and_another_restart() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project");
    tree(&root, &["a.txt"]);
    let mut one = session(d.path());
    open_project(&mut one, &root, "a.txt");
    edit(&mut one, "recovered λ\n");
    one.interrupt_for_test();

    let mut two = session(d.path());
    reopen(&mut two, &root);
    let ticket = two.recovery_prompt().unwrap().ticket;
    two.dispatch(Command::ChooseRecoveredDraft { ticket, choice: Choice::Later });
    assert!(two.recovery_prompt().is_none(), "Later cancels the dialog");
    assert_eq!(text(&two), "a.txt", "the tab shows the file until the draft is restored");
    assert_eq!(active_view(&two).edit_disabled(), Some(super::super::recovery_work::UNDECIDED));
    assert_eq!(two.disabled(CommandId::Paste), Some(super::super::recovery_work::UNDECIDED));
    edit(&mut two, "replacement\n");
    two.dispatch(Command::InsertText { text: "typed".into(), kind: crate::text::EditKind::Typing });
    two.dispatch(Command::Paste("pasted".into()));
    assert_eq!(text(&two), "a.txt", "no edit replaces the undecided draft");
    two.dispatch(Command::Reload);
    settle(&mut two, opened);
    two.flush_recovery_for_test();
    drop(two);

    let mut three = session(d.path());
    let offers = three.recovered_drafts();
    assert_eq!(offers.len(), 1, "a postponed draft is offered again");
    assert_eq!(offers[0].record().draft(), "recovered λ\n");
    reopen(&mut three, &root);
    let ticket = three.recovery_prompt().unwrap().ticket;
    three.dispatch(Command::ChooseRecoveredDraft { ticket, choice: Choice::Discard });
    assert_eq!(active_view(&three).edit_disabled(), None, "a decided file is editable again");
    edit(&mut three, "new work\n");
    assert_eq!(text(&three), "new work\n");
    assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "a.txt");
}

#[test]
fn a_running_session_keeps_its_drafts_to_itself() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "a.txt", b"saved\n");
    let mut one = session(d.path());
    open(&mut one, Command::OpenRetained(path.clone()), "a.txt");
    edit(&mut one, "live draft\n");
    one.flush_recovery_for_test();
    let two = session(d.path());
    assert!(two.recovered_drafts().is_empty(), "another window's live draft is not recovered work");
    drop(two);
    one.interrupt_for_test();
    let three = session(d.path());
    assert_eq!(three.recovered_drafts().len(), 1, "an interrupted session's draft is offered");
    assert_eq!(std::fs::read(&path).unwrap(), b"saved\n");
}

#[test]
fn reopening_names_tabs_whose_files_are_gone_and_keeps_a_new_file_opened_first() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project");
    tree(&root, &["a.txt", "gone.txt"]);
    let mut one = session(d.path());
    open_project(&mut one, &root, "gone.txt");
    one.dispatch(Command::Pin(one.state.active.unwrap()));
    one.dispatch(Command::OpenRetained(root.join("a.txt")));
    settle(&mut one, opened);
    drop(one);
    std::fs::remove_file(root.join("gone.txt")).unwrap();
    let mut two = session(d.path());
    reopen(&mut two, &root);
    assert_eq!(two.state.tabs.len(), 1);
    assert!(two.state.notices().iter().any(|m| m.contains("Not reopened from the last session") && m.contains("gone.txt")), "{:?}", two.state.notices());
    drop(two);

    // A New File made before the last tabs arrive is the latest choice: it is not replaced by them.
    let mut three = session(d.path());
    three.dispatch(Command::OpenRoot(root.clone()));
    let completion = loop { let c = three.completions().recv_blocking_timeout().unwrap(); if matches!(c, Completion::PlacesRead { .. }) { break c; } three.dispatch(c); };
    three.dispatch(Command::NewDocument);
    three.dispatch(completion);
    assert_eq!(three.state.tabs.len(), 1);
    assert!(three.state.active().unwrap().untitled());
}
