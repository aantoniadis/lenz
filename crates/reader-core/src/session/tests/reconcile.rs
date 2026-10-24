//! Reconciling an external disk change with a dirty draft: the three sides, the decisions and their guards.
use super::*;
use crate::reconcile::{self, Choice, NO_CONFLICT};

/// The draft `draft λ\r\n` edited over the confirmed baseline `saved\r\n`, then the file changed on disk.
fn conflict(dir: &Path, name: &str) -> (Session, PathBuf, TabId) {
    let (mut s, path, tab) = close_fixture(dir, name);
    std::fs::write(&path, b"external replacement\r\n").unwrap();
    s.dispatch(Command::Recheck);
    settle(&mut s, |s| s.state().active().unwrap().stale_on_disk());
    (s, path, tab)
}

fn opened_prompt(s: &mut Session) -> reconcile::Prompt {
    s.dispatch(Command::ResolveExternalChange);
    settle(s, |s| s.reconcile_prompt().is_some_and(|prompt| prompt.disk.is_some() || prompt.read_failure.is_some()));
    s.reconcile_prompt().unwrap()
}

#[test]
fn the_conflict_prompt_identifies_the_disk_draft_and_confirmed_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, path, _) = conflict(dir.path(), "prompt.txt");
    assert_eq!(s.disabled(CommandId::ResolveExternalChange), None);
    let prompt = opened_prompt(&mut s);
    assert_eq!(prompt.path, path);
    let disk = prompt.disk.as_ref().unwrap();
    assert_eq!((disk.lines, disk.bytes), (1, 22), "the file on disk is read as it is now");
    assert_eq!((prompt.draft.lines, prompt.draft.bytes), (1, 10));
    assert_eq!((prompt.baseline.lines, prompt.baseline.bytes), (1, 7));
    let text = prompt.text();
    for word in ["Disk:", "Current draft:", "Confirmed baseline"] {
        assert!(text.contains(word), "{word} is missing from the dialog: {text}");
    }
    assert!(text.contains("differs from the confirmed baseline"), "{text}");
    assert_eq!(Choice::ALL.map(Choice::label), ["Compare Disk, Draft and Baseline…", "Take Disk Version", "Save Draft over Disk", "Keep Draft"]);
    assert_eq!(Choice::ALL.map(Choice::key), ["c", "d", "s", "Esc"]);
    for choice in Choice::ALL {
        assert_eq!(prompt.disabled(choice), None, "{}", choice.label());
    }
    // Looking at the conflict decides nothing.
    assert_eq!(source_text(s.state().current_source().unwrap()), "draft λ\r\n");
    assert_eq!(std::fs::read(&path).unwrap(), b"external replacement\r\n");
    assert!(active_view(&s).dirty() && active_view(&s).can_undo());
}

/// The prompt's answer, applied through the same command an adapter sends.
fn choose(s: &mut Session, prompt: &reconcile::Prompt, choice: Choice) {
    s.dispatch(Command::ChooseReconcile { ticket: prompt.ticket, choice });
}

#[test]
fn taking_the_disk_version_is_one_undoable_edit_that_confirms_it() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, path, _) = conflict(dir.path(), "take.txt");
    let prompt = opened_prompt(&mut s);
    choose(&mut s, &prompt, Choice::TakeDisk);
    settle(&mut s, |s| source_text(s.state().current_source().unwrap()) == "external replacement\r\n");
    assert_eq!(source_text(s.state().current_source().unwrap()), "external replacement\r\n");
    assert!(!active_view(&s).dirty() && !active_view(&s).can_redo());
    assert!(active_view(&s).can_undo(), "the take is one undoable edit");
    assert_eq!(s.disabled(CommandId::ResolveExternalChange), Some(NO_CONFLICT), "nothing is left to reconcile");
    assert_eq!(std::fs::read(&path).unwrap(), b"external replacement\r\n", "taking disk never writes");

    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "draft λ\r\n");
    assert!(active_view(&s).dirty(), "the old draft differs from the confirmed disk version");
    assert!(active_view(&s).can_redo());
    s.dispatch(Command::Redo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "external replacement\r\n");
    assert!(!active_view(&s).dirty());
    // The confirmed baseline is the taken disk version, not the one the draft was edited from.
    s.dispatch(Command::CompareSaved);
    let saved = s.state().saved_comparison().unwrap().saved_tab();
    assert_eq!(source_text(s.state().view(saved).unwrap().snapshot()), "external replacement\r\n");
    s.dispatch(Command::CloseSavedComparison);
    assert_eq!(std::fs::read(&path).unwrap(), b"external replacement\r\n");
}

#[test]
fn keeping_the_draft_does_not_adopt_the_new_disk_version_as_its_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, path, _) = conflict(dir.path(), "keep.txt");
    let prompt = opened_prompt(&mut s);
    choose(&mut s, &prompt, Choice::KeepDraft);
    assert!(s.reconcile_prompt().is_none(), "the dialog is closed by its own answer");
    assert_eq!(source_text(s.state().current_source().unwrap()), "draft λ\r\n");
    assert!(active_view(&s).dirty() && active_view(&s).can_undo());
    // The confirmed baseline is still the version the draft was edited from.
    s.dispatch(Command::CompareSaved);
    let saved = s.state().saved_comparison().unwrap().saved_tab();
    assert_eq!(source_text(s.state().view(saved).unwrap().snapshot()), "saved\r\n");
    s.dispatch(Command::CloseSavedComparison);
    // Keeping the draft decides nothing about a write: its save state is unchanged, and so is the file.
    assert!(matches!(active_view(&s).save_status(), crate::save::Status::Ready));
    assert_eq!(s.disabled(CommandId::ResolveExternalChange), None, "the conflict can be reviewed again");
    assert_eq!(std::fs::read(&path).unwrap(), b"external replacement\r\n");
    let again = opened_prompt(&mut s);
    choose(&mut s, &again, Choice::KeepDraft);
    assert_eq!(source_text(s.state().current_source().unwrap()), "draft λ\r\n");
}

#[test]
fn saving_the_draft_over_disk_writes_after_one_more_check_and_keeps_undo() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, path, _) = conflict(dir.path(), "over.txt");
    let prompt = opened_prompt(&mut s);
    choose(&mut s, &prompt, Choice::SaveDraft);
    settle(&mut s, |s| !active_view(s).dirty() && !matches!(active_view(s).save_status(), crate::save::Status::Saving | crate::save::Status::Rechecking));
    assert_eq!(std::fs::read(&path).unwrap(), "draft λ\r\n".as_bytes());
    assert!(!active_view(&s).dirty() && active_view(&s).can_undo());
    assert_eq!(s.disabled(CommandId::ResolveExternalChange), Some(NO_CONFLICT));
    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "saved\r\n");
    assert!(active_view(&s).dirty());
    assert_eq!(std::fs::read(&path).unwrap(), "draft λ\r\n".as_bytes(), "undo changes the draft, not the file");
}

#[test]
fn a_second_external_change_while_deciding_retains_the_draft_and_refreshes_the_sides() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, path, _) = conflict(dir.path(), "second.txt");
    let prompt = opened_prompt(&mut s);
    std::fs::write(&path, b"someone else wrote this\r\n").unwrap();
    choose(&mut s, &prompt, Choice::TakeDisk);
    settle(&mut s, |s| s.reconcile_prompt().is_some_and(|prompt| prompt.disk.as_ref().is_some_and(|disk| disk.bytes == 25)));
    assert_eq!(source_text(s.state().current_source().unwrap()), "draft λ\r\n", "nothing is taken from a version the user did not see");
    assert!(active_view(&s).dirty() && active_view(&s).can_undo());
    assert_eq!(std::fs::read(&path).unwrap(), b"someone else wrote this\r\n");
    let prompt = s.reconcile_prompt().unwrap();
    assert_eq!(prompt.disk_change, Some(reconcile::DiskChange::Other));
    assert!(prompt.text().contains("25 bytes"), "{}", prompt.text());
    assert!(s.state().messages.iter().any(|m| m.contains("changed again")), "{:?}", s.state().messages);
    // The same decision then applies the version the user has now seen.
    choose(&mut s, &prompt, Choice::TakeDisk);
    settle(&mut s, |s| source_text(s.state().current_source().unwrap()) == "someone else wrote this\r\n");
    assert!(!active_view(&s).dirty());
    assert_eq!(std::fs::read(&path).unwrap(), b"someone else wrote this\r\n");
}

#[test]
fn an_uncertain_write_after_saving_over_disk_retains_the_draft_and_the_confirmed_baseline() {
    use lenz_content::write::readiness_fault::{at_path, Fault};
    let dir = tempfile::tempdir().unwrap();
    let (mut s, path, _) = conflict(dir.path(), "uncertain.txt");
    let prompt = opened_prompt(&mut s);
    let guard = at_path(&path, Fault::AfterReplace);
    choose(&mut s, &prompt, Choice::SaveDraft);
    settle(&mut s, |s| matches!(active_view(s).save_status(), crate::save::Status::Uncertain(_)));
    drop(guard);
    assert_eq!(std::fs::read(&path).unwrap(), "draft λ\r\n".as_bytes(), "the write happened, its outcome is uncertain");
    assert_eq!(source_text(s.state().current_source().unwrap()), "draft λ\r\n");
    assert!(active_view(&s).dirty() && active_view(&s).can_undo());
    assert!(s.disabled(CommandId::Save).is_some(), "an uncertain outcome needs a recheck, not another write");
    s.dispatch(Command::RecheckSave { tab: s.state().active().unwrap().id() });
    settle(&mut s, |s| matches!(active_view(s).save_status(), crate::save::Status::Checked(_)));
    assert_eq!(std::fs::read(&path).unwrap(), "draft λ\r\n".as_bytes());
}

#[test]
fn resolving_needs_a_recorded_conflict_and_an_older_ticket_decides_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _, _) = close_fixture(dir.path(), "noconflict.txt");
    assert_eq!(s.disabled(CommandId::ResolveExternalChange), Some(NO_CONFLICT));
    s.dispatch(Command::ResolveExternalChange);
    assert!(s.reconcile_prompt().is_none(), "nothing was read for a file that matches its confirmed baseline");

    let (mut s, path, _) = conflict(dir.path(), "stale.txt");
    let older = opened_prompt(&mut s);
    let current = opened_prompt(&mut s);
    assert!(current.ticket > older.ticket);
    choose(&mut s, &older, Choice::TakeDisk);
    assert_eq!(source_text(s.state().current_source().unwrap()), "draft λ\r\n", "an older dialog cannot decide");
    assert_eq!(s.reconcile_prompt().map(|prompt| prompt.ticket), Some(current.ticket), "the current dialog is still open");
    assert_eq!(std::fs::read(&path).unwrap(), b"external replacement\r\n");
    assert!(active_view(&s).dirty() && active_view(&s).can_undo());
}

#[test]
fn the_conflict_comparison_shows_the_disk_the_draft_and_the_confirmed_baseline() {
    use crate::session::ComparisonKind;
    let dir = tempfile::tempdir().unwrap();
    let (mut s, path, _) = conflict(dir.path(), "compare.txt");
    let prompt = opened_prompt(&mut s);
    choose(&mut s, &prompt, Choice::Compare);
    assert!(s.reconcile_prompt().is_none(), "the comparison replaces the dialog while it is open");
    let comparison = s.state().saved_comparison().expect("the conflict comparison is open");
    assert_eq!(comparison.kind(), ComparisonKind::Conflict);
    let (disk, current, baseline) = (comparison.saved_tab(), comparison.current_tab(), comparison.context_tab().unwrap());
    assert_eq!(s.state().view(disk).unwrap().tab().title(), "Disk (read-only)");
    assert_eq!(s.state().view(current).unwrap().tab().title(), "Current draft");
    assert_eq!(s.state().view(baseline).unwrap().tab().title(), "Confirmed baseline (read-only)");
    assert_eq!(source_text(s.state().view(disk).unwrap().snapshot()), "external replacement\r\n");
    assert_eq!(source_text(s.state().view(current).unwrap().snapshot()), "draft λ\r\n");
    assert_eq!(source_text(s.state().view(baseline).unwrap().snapshot()), "saved\r\n");
    assert_eq!(s.state().view(disk).unwrap().edit_disabled(), Some("The disk version is read-only"));
    assert_eq!(s.state().view(baseline).unwrap().edit_disabled(), Some("The confirmed baseline is read-only"));
    assert!(s.state().view(current).unwrap().edit_disabled().is_none());
    assert_eq!(s.state().active().unwrap().id(), current);
    // The disk version is what the draft is compared with.
    settle(&mut s, |s| s.state().saved_comparison().is_some_and(|comparison| !comparison.pending()));
    assert_eq!(s.state().saved_comparison().unwrap().changes().len(), 1);
    // Resolving edits the ordinary draft through the comparison; the file on disk stays as it is.
    let expected = s.state().view(current).unwrap().snapshot().key();
    s.dispatch(Command::Edit { tab: current, expected, edits: vec![crate::text::TextEdit { range: 0..5, text: "resolved".into() }], kind: crate::text::EditKind::Replace });
    assert_eq!(source_text(s.state().current_source().unwrap()), "resolved λ\r\n");
    assert_eq!(std::fs::read(&path).unwrap(), b"external replacement\r\n");
    // The read-only sides refuse edits.
    let stale = s.state().view(disk).unwrap().snapshot().key();
    s.dispatch(Command::Edit { tab: disk, expected: stale, edits: vec![crate::text::TextEdit { range: 0..0, text: "x".into() }], kind: crate::text::EditKind::Paste });
    assert_eq!(source_text(s.state().view(disk).unwrap().snapshot()), "external replacement\r\n");
    // Closing the comparison brings the dialog back, still undecided, for the draft as it is now.
    s.dispatch(Command::CloseSavedComparison);
    assert!(s.state().saved_comparison().is_none());
    let prompt = s.reconcile_prompt().expect("the dialog returns after the comparison closes");
    assert_eq!((prompt.draft.lines, prompt.draft.bytes), (1, 13), "the sides describe the draft as it is now");
    choose(&mut s, &prompt, Choice::SaveDraft);
    settle(&mut s, |s| matches!(active_view(s).save_status(), crate::save::Status::Ready) && !active_view(s).dirty());
    assert_eq!(std::fs::read(&path).unwrap(), "resolved λ\r\n".as_bytes());
}

#[test]
fn a_take_that_cannot_be_applied_confirms_no_new_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, path, _) = conflict(dir.path(), "readonly.txt");
    let prompt = opened_prompt(&mut s);
    // The decision's own read is applied only when its reply is dispatched: read-only mode arrives first.
    choose(&mut s, &prompt, Choice::TakeDisk);
    s.dispatch(Command::SetReadOnly(true));
    settle(&mut s, |s| s.state().messages.iter().any(|message| message.contains("could not be put into the draft")));
    assert_eq!(source_text(s.state().current_source().unwrap()), "draft λ\r\n");
    assert!(active_view(&s).dirty() && active_view(&s).can_undo());
    assert_eq!(std::fs::read(&path).unwrap(), b"external replacement\r\n");
    s.dispatch(Command::SetReadOnly(false));
    s.dispatch(Command::CompareSaved);
    let saved = s.state().saved_comparison().unwrap().saved_tab();
    assert_eq!(source_text(s.state().view(saved).unwrap().snapshot()), "saved\r\n", "a failed take adopts nothing");
    s.dispatch(Command::CloseSavedComparison);
}
