use super::*;
mod external;
use crate::tabs::TabId;
use crate::text::TextSnapshot;
use std::time::Duration;
use lenz_content::find::FindOption;
use lenz_content::goto::LineColumn;

// Exact source assertions may copy a whole fixture; production reads remain windowed.
fn source_text(source: &TextSnapshot) -> String { source.copy_range(0..source.len_bytes()).unwrap() }
fn source_line(source: &TextSnapshot, line: usize) -> String { source.copy_range(source.reader_line_range(line)).unwrap() }

fn close_fixture(dir: &Path, name: &str) -> (Session, PathBuf, TabId) {
    let path = write(dir, name, b"saved\r\n");
    let mut s = session(&dir.join(format!("{name}-state")));
    open(&mut s, Command::OpenRetained(path.clone()), name);
    let tab = s.state().active().unwrap().id(); let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..5, text: "draft λ".into() }], kind: crate::text::EditKind::Replace });
    (s, path, tab)
}
#[test]
fn a_pending_reload_cannot_replace_the_ordinary_owner_with_a_temporary_comparison_tab() {
    let d = tempfile::tempdir().unwrap(); let path = write(d.path(), "reload.txt", b"saved\n");
    let mut s = session(d.path()); open(&mut s, Command::OpenRetained(path.clone()), "reload.txt");
    s.dispatch(Command::Split(Split::Columns)); let ordinary = s.state().active().unwrap().id();
    let (started, entered) = std::sync::mpsc::channel();
    let gates: Vec<_> = (0..crate::jobs::MAX_JOBS).map(|_| { let (release, wait) = std::sync::mpsc::channel(); let started = started.clone();
        s.jobs.submit(Priority::Visible, move || { started.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); }); release }).collect();
    for _ in &gates { entered.recv_timeout(Duration::from_secs(5)).unwrap(); }
    s.dispatch(Command::Reload); s.dispatch(Command::CompareSaved);
    for gate in gates { gate.send(()).unwrap(); }
    settle(&mut s, |s| s.state.loading.is_none() && s.state().saved_comparison().is_some_and(|c| !c.pending()));
    assert!(s.state().tabs().iter().any(|t| t.id() == ordinary));
    s.dispatch(Command::CloseSavedComparison);
    assert_eq!(s.state().active().unwrap().id(), ordinary);
    assert_eq!(source_text(s.state().current_source().unwrap()), "saved\n");
    assert_eq!(std::fs::read(path).unwrap(), b"saved\n");
}

#[test]
fn saved_comparison_reports_line_ending_only_changes() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap(); let path = write(d.path(), "endings.txt", b"same\r\n");
    let mut s = session(d.path()); open(&mut s, Command::OpenRetained(path.clone()), "endings.txt");
    let tab = s.state().active().unwrap().id(); let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..6, text: "same\n".into() }], kind: EditKind::Replace });
    s.dispatch(Command::CompareSaved); settle(&mut s, |s| !s.state().saved_comparison().unwrap().pending());
    assert!(s.state().saved_comparison().unwrap().changes().is_empty());
    assert!(s.state().saved_comparison().unwrap().line_endings_differ());
    assert!(s.status_details().contains("line endings differ"));
    assert_eq!(std::fs::read(path).unwrap(), b"same\r\n");
}

#[test]
fn closing_the_ordinary_owner_during_comparison_still_requires_its_dirty_decision() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap(); let path = write(d.path(), "owner.txt", b"saved");
    let mut s = session(d.path()); open(&mut s, Command::OpenRetained(path.clone()), "owner.txt");
    let ordinary = s.state().active().unwrap().id(); let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab: ordinary, expected, edits: vec![TextEdit { range: 0..5, text: "draft".into() }], kind: EditKind::Replace });
    s.dispatch(Command::CompareSaved);
    s.dispatch(Command::CloseTab(ordinary));
    assert!(s.state().saved_comparison().is_none());
    let prompt = s.close_prompt().unwrap();
    s.dispatch(Command::ChooseClose { ticket: prompt.ticket, buffer: prompt.buffer, choice: crate::close::Choice::Cancel });
    assert_eq!(s.state().active().unwrap().id(), ordinary);
    assert_eq!(source_text(s.state().current_source().unwrap()), "draft");
    assert_eq!(std::fs::read(path).unwrap(), b"saved");
}

#[test]
fn close_during_saved_backing_compaction_waits_without_demanding_another_save() {
    use crate::save::Status;
    let d=tempfile::tempdir().unwrap();let original="a".repeat(128*1024);let path=write(d.path(),"compact-close.txt",original.as_bytes());let mut s=session(d.path());open(&mut s,Command::OpenRetained(path.clone()),"compact-close.txt");let tab=s.state().active().unwrap().id();let expected=s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit {tab,expected,edits:vec![crate::text::TextEdit {range:0..original.len()-16384,text:"".into()}],kind:crate::text::EditKind::Replace});let _ready=analysis_document(&mut s);
    let (entered,wait_entered)=std::sync::mpsc::channel();let mut gates:Vec<_>=(0..crate::jobs::MAX_JOBS).map(|_|{let (release,wait)=std::sync::mpsc::channel();let entered=entered.clone();s.jobs.submit(Priority::Visible,move||{entered.send(()).unwrap();let _=wait.recv_timeout(Duration::from_secs(5));});release}).collect();for _ in &gates {wait_entered.recv_timeout(Duration::from_secs(5)).unwrap();}
    s.dispatch(Command::Save {tab});
    let (after_write,write_held)=std::sync::mpsc::channel();let (release,wait)=std::sync::mpsc::channel();s.jobs.submit(Priority::Visible,move||{after_write.send(()).unwrap();let _=wait.recv_timeout(Duration::from_secs(5));});gates.remove(0).send(()).unwrap();write_held.recv_timeout(Duration::from_secs(5)).unwrap();
    s.dispatch(Completion::AnalysisWake);assert!(!active_view(&s).dirty());assert_eq!(active_view(&s).save_status(),&Status::Saving,"real saved-baseline compaction is held in the existing worker pool");assert_eq!(std::fs::read(&path).unwrap(),vec![b'a';16384]);
    s.dispatch(Command::CloseTab(tab));assert!(s.close_prompt().unwrap().waiting);assert!(s.state().active().is_some());
    for gate in gates {gate.send(()).unwrap();}release.send(()).unwrap();settle(&mut s,|s|s.state().active().is_none() || *active_view(s).save_status()==Status::Ready);
    assert!(s.state().active().is_none(),"compaction completion is not an unconfirmed Save failure");assert_eq!(std::fs::read(path).unwrap(),vec![b'a';16384]);
}

#[test]
fn close_discard_recheck_receipt_cannot_authorize_a_later_draft_version() {
    use crate::close::Choice;use crate::save::Status;use lenz_content::write::readiness_fault::{at_path,Fault};
    let d=tempfile::tempdir().unwrap();let (mut s,path,tab)=close_fixture(d.path(),"a.txt");let guard=at_path(&path,Fault::AfterReplace);s.dispatch(Command::Save {tab});settle(&mut s,|s|matches!(active_view(s).save_status(),Status::Uncertain(_)));drop(guard);
    s.dispatch(Command::RecheckSave {tab});let expected=s.state().current_source().unwrap().key();s.dispatch(Command::Edit {tab,expected,edits:vec![crate::text::TextEdit {range:0..0,text:"newer ".into()}],kind:crate::text::EditKind::Paste});settle(&mut s,|s|matches!(active_view(s).save_status(),Status::Checked(_)));
    s.dispatch(Command::CloseTab(tab));let stale=s.close_prompt().unwrap();assert!(stale.discard_disabled.is_some(),"a recheck captured before the edit is not current consent");s.dispatch(Command::ChooseClose {ticket:stale.ticket,buffer:stale.buffer,choice:Choice::Discard});assert!(s.state().active().is_some());s.dispatch(Command::ChooseClose {ticket:stale.ticket,buffer:stale.buffer,choice:Choice::Cancel});
    s.dispatch(Command::RecheckSave {tab});settle(&mut s,|s|matches!(active_view(s).save_status(),Status::Checked(_)));s.dispatch(Command::CloseTab(tab));let fresh=s.close_prompt().unwrap();assert!(fresh.discard_disabled.is_none());assert!(fresh.ticket>stale.ticket);s.dispatch(Command::ChooseClose {ticket:fresh.ticket,buffer:fresh.buffer,choice:Choice::Discard});assert!(s.state().active().is_none());assert_eq!(std::fs::read(path).unwrap(),"draft λ\r\n".as_bytes());
}

#[test]
fn close_batch_rejects_new_draft_in_an_initially_evicted_clean_buffer() {
    use crate::close::{Choice,CloseCoordinator,Progress};
    let d=tempfile::tempdir().unwrap();let (mut a,ap,_)=close_fixture(d.path(),"a.txt");let mut b=session(&d.path().join("b-state"));
    let dormant=write(d.path(),"dormant.txt",b"dormant saved");let shown=write(d.path(),"shown.txt",b"shown saved");
    b.cache=InactiveCache::new(0);open(&mut b,Command::OpenRetained(dormant.clone()),"dormant.txt");let tab=b.state().active().unwrap().id();open(&mut b,Command::OpenRetained(shown.clone()),"shown.txt");assert!(b.cache.is_empty());
    b.cache=InactiveCache::new(crate::cache::INACTIVE_BYTES);
    let mut c=CloseCoordinator::default();c.register(a.id());c.register(b.id());c.begin(&mut [&mut a,&mut b]);let p=a.close_prompt().unwrap();a.dispatch(Command::ChooseClose {ticket:p.ticket,buffer:p.buffer,choice:Choice::Discard});
    b.dispatch(Command::ActivateTab(tab));settle(&mut b,|s|s.state().current_source().is_some_and(|s|s.source()==dormant));let expected=b.state().current_source().unwrap().key();
    b.dispatch(Command::Edit {tab,expected,edits:vec![crate::text::TextEdit {range:0..0,text:"new draft ".into()}],kind:crate::text::EditKind::Paste});
    assert_eq!(c.progress(&mut [&mut a,&mut b]),Progress::Cancelled,"new edits invalidate preparation even when the clean buffer had no retained snapshot at prepare time");
    assert!(!a.closed() && !b.closed());assert_eq!(source_text(b.state().current_source().unwrap()),"new draft dormant saved");assert_eq!(std::fs::read(ap).unwrap(),b"saved\r\n");assert_eq!(std::fs::read(dormant).unwrap(),b"dormant saved");assert_eq!(std::fs::read(shown).unwrap(),b"shown saved");
}

#[test]
fn close_after_uncertain_save_requires_recheck_then_fresh_discard_without_accepting_a_baseline() {
    use crate::close::Choice;
    use crate::save::{DiskMatch,Status};
    use lenz_content::write::readiness_fault::{at_path,Fault};
    for changed in [false,true] {
        let d=tempfile::tempdir().unwrap();let (mut s,path,tab)=close_fixture(d.path(),"a.txt");
        let guard=at_path(&path,Fault::AfterReplace);s.dispatch(Command::CloseTab(tab));let old=s.close_prompt().unwrap();
        s.dispatch(Command::ChooseClose {ticket:old.ticket,buffer:old.buffer,choice:Choice::Save});settle(&mut s,|s|matches!(active_view(s).save_status(),Status::Uncertain(_)));
        assert!(s.close_prompt().is_none(),"uncertain Save cancels the old close continuation");drop(guard);
        s.dispatch(Command::CloseTab(tab));let before=s.close_prompt().unwrap();assert!(before.save_disabled.is_some() && before.discard_disabled.is_some());
        s.dispatch(Command::ChooseClose {ticket:before.ticket,buffer:before.buffer,choice:Choice::Discard});assert_eq!(s.close_prompt().unwrap().ticket,before.ticket);
        s.dispatch(Command::ChooseClose {ticket:before.ticket,buffer:before.buffer,choice:Choice::Cancel});
        if changed {std::fs::write(&path,b"external disk content").unwrap();}
        let disk=std::fs::read(&path).unwrap();s.dispatch(Command::RecheckSave {tab});settle(&mut s,|s|matches!(active_view(s).save_status(),Status::Checked(_)));
        let status=Status::Checked(if changed {DiskMatch::Changed}else{DiskMatch::Attempted});assert_eq!(active_view(&s).save_status(),&status);assert!(active_view(&s).dirty());
        s.dispatch(Command::CloseTab(tab));let fresh=s.close_prompt().unwrap();assert!(fresh.ticket>old.ticket);assert!(fresh.save_disabled.is_some());assert!(fresh.discard_disabled.is_none(),"a successful current recheck permits a new explicit Discard, including Changed");
        s.dispatch(Command::ChooseClose {ticket:old.ticket,buffer:old.buffer,choice:Choice::Discard});assert_eq!(s.close_prompt().unwrap().ticket,fresh.ticket,"the failed close never resumes");
        assert_eq!(active_view(&s).save_status(),&status);assert!(active_view(&s).dirty());assert_eq!(source_text(s.state().current_source().unwrap()),"draft λ\r\n");
        s.dispatch(Command::ChooseClose {ticket:fresh.ticket,buffer:fresh.buffer,choice:Choice::Discard});assert!(s.state().active().is_none());assert!(s.close_prompt().is_none());assert_eq!(std::fs::read(path).unwrap(),disk);
    }
}

#[test]
fn closed_sessions_reject_new_user_mutations_and_old_close_permits() {
    use crate::close::{Choice,CloseCoordinator,Progress};
    let d=tempfile::tempdir().unwrap();let (mut s,path,tab)=close_fixture(d.path(),"a.txt");let mut c=CloseCoordinator::default();c.register(s.id());c.begin(&mut [&mut s]);let p=s.close_prompt().unwrap();
    s.dispatch(Command::ChooseClose {ticket:p.ticket,buffer:p.buffer,choice:Choice::Discard});let id=s.id();assert_eq!(c.progress(&mut [&mut s]),Progress::Committed(vec![id]));
    let source=s.state().current_source().unwrap().clone();
    assert_eq!(s.last_close().unwrap().discarded,vec![source.key().buffer]);
    s.dispatch(Command::Edit {tab,expected:source.key(),edits:vec![crate::text::TextEdit {range:0..0,text:"late ".into()}],kind:crate::text::EditKind::Paste});s.dispatch(Command::Undo {tab});s.dispatch(Command::OpenRetained(path.clone()));
    s.dispatch(Command::CommitClose(crate::close::Permit {session:id,ticket:p.ticket}));
    assert_eq!(s.state().current_source().unwrap().key(),source.key(),"closed Session is metadata for cleanup, never a mutable editor");assert!(s.closed());assert!(s.close_prompt().is_none());assert_eq!(std::fs::read(path).unwrap(),b"saved\r\n");
}

#[test]
fn close_consent_is_invalidated_by_a_new_explicit_save() {
    use crate::close::{Choice,CloseCoordinator,Progress};
    let d=tempfile::tempdir().unwrap();let (mut a,path,tab)=close_fixture(d.path(),"a.txt");
    let mut c=CloseCoordinator::default();c.register(a.id());c.begin(&mut [&mut a]);let p=a.close_prompt().unwrap();
    a.dispatch(Command::ChooseClose {ticket:p.ticket,buffer:p.buffer,choice:Choice::Discard});
    a.dispatch(Command::Save {tab});
    settle(&mut a,|s| !s.state().tab_dirty(tab));
    assert_eq!(c.progress(&mut [&mut a]),Progress::Cancelled,"an intervening explicit operation supersedes old destructive consent");assert!(!a.closed());
    assert_eq!(std::fs::read(path).unwrap(),"draft λ\r\n".as_bytes());
}

#[test]
fn close_twins_unsplit_and_final_discard_preserve_accessible_draft_owners() {
    use crate::close::Choice;
    let d=tempfile::tempdir().unwrap();let (mut s,path,first)=close_fixture(d.path(),"a.txt");
    s.dispatch(Command::Split(Split::Columns)); let twin=s.state().active().unwrap().id(); assert_ne!(first,twin);
    s.dispatch(Command::CloseTab(twin)); assert!(s.close_prompt().is_none());assert_eq!(s.state().active().unwrap().id(),first);
    assert_eq!(source_text(s.state().current_source().unwrap()),"draft λ\r\n");
    s.dispatch(Command::Split(Split::Columns));let other=write(d.path(),"b.txt",b"other");open(&mut s,Command::OpenRetained(other),"b.txt");
    s.dispatch(Command::Unsplit);assert!(s.close_prompt().is_none());assert_eq!(s.state().tabs().len(),2,"distinct tabs move; only the duplicate source view is removed");
    let first=s.state().tabs().iter().find(|t|t.source()==path).unwrap().id();
    s.dispatch(Command::CloseTab(first));let p=s.close_prompt().unwrap();s.dispatch(Command::ChooseClose {ticket:p.ticket,buffer:p.buffer,choice:Choice::Cancel});
    assert!(s.state().tabs().iter().any(|t|t.id()==first));
    s.dispatch(Command::CloseTab(first));let p=s.close_prompt().unwrap();
    s.dispatch(Command::ChooseClose {ticket:p.ticket-1,buffer:p.buffer,choice:Choice::Discard});assert_eq!(s.close_prompt().unwrap().ticket,p.ticket,"an obsolete response cannot cancel or approve the new decision");
    s.dispatch(Command::ChooseClose {ticket:p.ticket,buffer:p.buffer,choice:Choice::Discard});assert!(!s.state().tabs().iter().any(|t|t.id()==first));
    open(&mut s,Command::OpenRetained(path.clone()),"a.txt");assert_eq!(source_text(s.state().current_source().unwrap()),"saved\r\n");assert_eq!(std::fs::read(path).unwrap(),b"saved\r\n");
}
#[test]
fn close_batch_save_failure_cancels_every_prepared_discard() {
    use crate::close::{Choice,CloseCoordinator,Progress};
    use lenz_content::write::readiness_fault::{at_path,Fault};
    for fault in [Fault::BeforeReplace,Fault::AfterReplace] {
        let d=tempfile::tempdir().unwrap();let (mut a,ap,_)=close_fixture(d.path(),"a.txt");let (mut b,bp,_)=close_fixture(d.path(),"b.txt");
        let mut c=CloseCoordinator::default();c.register(a.id());c.register(b.id());c.begin(&mut [&mut a,&mut b]);
        let p=a.close_prompt().unwrap();a.dispatch(Command::ChooseClose {ticket:p.ticket,buffer:p.buffer,choice:Choice::Discard});
        let guard=at_path(&bp,fault);let p=b.close_prompt().unwrap();b.dispatch(Command::ChooseClose {ticket:p.ticket,buffer:p.buffer,choice:Choice::Save});
        settle(&mut b,|s|s.close_prompt().is_none());assert_eq!(c.progress(&mut [&mut a,&mut b]),Progress::Cancelled);
        assert!(!a.closed() && !b.closed());assert_eq!(source_text(a.state().current_source().unwrap()),"draft λ\r\n");assert_eq!(source_text(b.state().current_source().unwrap()),"draft λ\r\n");
        assert_eq!(std::fs::read(ap).unwrap(),b"saved\r\n");assert_eq!(std::fs::read(bp).unwrap(),if fault==Fault::AfterReplace {"draft λ\r\n".as_bytes()} else {b"saved\r\n"});drop(guard);
    }
}

#[test]
fn close_batch_cancel_preserves_every_draft_and_only_explicit_saves_persist() {
    use crate::close::{Choice, CloseCoordinator, Progress};
    for choice in [Choice::Discard, Choice::Save] {
        let d = tempfile::tempdir().unwrap(); let (mut a, ap, at) = close_fixture(d.path(), "a.txt"); let (mut b, bp, bt) = close_fixture(d.path(), "b.txt");
        let mut coordinator = CloseCoordinator::default(); coordinator.register(a.id()); coordinator.register(b.id());
        coordinator.begin(&mut [&mut a, &mut b]);
        assert_eq!(coordinator.progress(&mut [&mut a, &mut b]), Progress::Pending(a.id()));
        let prompt = a.close_prompt().unwrap(); a.dispatch(Command::ChooseClose { ticket: prompt.ticket, buffer: prompt.buffer, choice });
        if choice == Choice::Save { settle(&mut a, |s| s.close_prompt().is_none()); }
        assert!(!a.closed() && !b.closed(), "early consent must never remove a participant");
        let prompt = b.close_prompt().unwrap(); b.dispatch(Command::ChooseClose { ticket: prompt.ticket, buffer: prompt.buffer, choice: Choice::Cancel });
        assert_eq!(coordinator.progress(&mut [&mut a, &mut b]), Progress::Cancelled);
        assert_eq!(a.state().active().unwrap().id(), at); assert_eq!(b.state().active().unwrap().id(), bt);
        assert_eq!(source_text(a.state().current_source().unwrap()), "draft λ\r\n"); assert_eq!(source_text(b.state().current_source().unwrap()), "draft λ\r\n");
        assert_eq!(std::fs::read(ap).unwrap(), if choice == Choice::Save { "draft λ\r\n".as_bytes() } else { b"saved\r\n" });
        assert_eq!(std::fs::read(bp).unwrap(), b"saved\r\n");
    }
}
#[test]
fn close_batch_rejects_edited_or_replaced_membership_before_any_commit() {
    use crate::close::{Choice, CloseCoordinator, Progress};
    for new_member in [false, true] {
        let d = tempfile::tempdir().unwrap(); let (mut a, ap, _) = close_fixture(d.path(), "a.txt"); let (mut b, bp, _) = close_fixture(d.path(), "b.txt");
        let mut coordinator = CloseCoordinator::default(); coordinator.register(a.id()); coordinator.register(b.id()); coordinator.begin(&mut [&mut a, &mut b]);
        for s in [&mut a, &mut b] { let p=s.close_prompt().unwrap(); s.dispatch(Command::ChooseClose { ticket:p.ticket,buffer:p.buffer,choice:Choice::Discard }); }
        if new_member { let c=session(&d.path().join("c")); coordinator.register(c.id()); }
        else { let tab=a.state().active().unwrap().id(); let expected=a.state().current_source().unwrap().key(); a.dispatch(Command::Edit {tab,expected,edits:vec![crate::text::TextEdit {range:0..0,text:"new ".into()}],kind:crate::text::EditKind::Paste}); }
        assert_eq!(coordinator.progress(&mut [&mut a,&mut b]),Progress::Cancelled);
        assert!(!a.closed() && !b.closed()); assert_eq!(std::fs::read(ap).unwrap(),b"saved\r\n"); assert_eq!(std::fs::read(bp).unwrap(),b"saved\r\n");
    }
}
#[test]
fn close_save_resolves_inactive_dirty_buffer_without_activating_it() {
    use crate::close::{Choice, CloseCoordinator, Progress};
    let d=tempfile::tempdir().unwrap(); let (mut s,path,tab)=close_fixture(d.path(),"a.txt");
    let other=write(d.path(),"b.txt",b"other"); open(&mut s,Command::OpenRetained(other),"b.txt"); let active=s.state().active().unwrap().id();
    assert!(s.state().view(tab).is_none());
    let mut coordinator=CloseCoordinator::default();coordinator.register(s.id());coordinator.begin(&mut [&mut s]);let p=s.close_prompt().unwrap();
    s.dispatch(Command::ChooseClose {ticket:p.ticket,buffer:p.buffer,choice:Choice::Save});settle(&mut s,|s|s.close_prompt().is_none());
    assert_eq!(s.state().active().unwrap().id(),active);assert_eq!(std::fs::read(path).unwrap(),"draft λ\r\n".as_bytes());
    assert_eq!(coordinator.progress(&mut [&mut s]),Progress::Committed(vec![s.id()])); assert!(s.closed());
}

#[test]
fn dirty_last_tab_close_preserves_the_draft_until_a_close_decision() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "close.txt", b"saved\r\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path.clone()), "close.txt");
    let tab = s.state().active().unwrap().id();
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..5, text: "draft λ".into() }], kind: EditKind::Replace });
    s.dispatch(Command::CloseTab(tab));
    assert_eq!(s.state().active().map(|t| t.id()), Some(tab), "last dirty owner must wait for Save/Discard/Cancel");
    assert_eq!(source_text(s.state().current_source().unwrap()), "draft λ\r\n");
    assert_eq!(std::fs::read(path).unwrap(), b"saved\r\n");
}

#[test]
fn saved_comparison_restores_an_existing_split_and_never_persists_temporary_views() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap(); let text = "line\n".repeat(100); let path = write(d.path(), "split.txt", text.as_bytes());
    let mut s = session(d.path()); open(&mut s, Command::OpenRetained(path.clone()), "split.txt");
    let first = s.state().active().unwrap().id(); s.dispatch(Command::Scroll(20));
    s.dispatch(Command::Split(Split::Rows)); let second = s.state().active().unwrap().id(); s.dispatch(Command::Scroll(30));
    s.dispatch(Command::CompareSaved); let comparison = s.state().saved_comparison().unwrap(); let old = comparison.saved_tab(); let current = comparison.current_tab();
    let expected = s.state().view(first).unwrap().snapshot().key();
    s.dispatch(Command::Edit { tab: first, expected, edits: vec![TextEdit { range: 0..0, text: "new\n".into() }], kind: EditKind::Paste });
    assert_eq!(s.state().view(current).unwrap().snapshot().key(), s.state().view(second).unwrap().snapshot().key());
    let positions: Vec<_> = [first, second].iter().map(|id| { let tab = s.state.tabs.iter().find(|t| t.id == *id).unwrap(); (tab.top_line(), tab.selection()) }).collect();
    s.dispatch(Command::SetSoftWrap(true));
    // An incompatible view transition deliberately closes only the comparison before normal split handling.
    s.dispatch(Command::Unsplit);
    assert!(s.state().saved_comparison().is_none()); assert_eq!(s.state().split(), Some(Split::Rows));
    assert_eq!(s.state().focused_pane(), 1); assert_eq!(s.state().active().unwrap().id(), second); assert_eq!(s.state().shown(0).unwrap().id(), first);
    assert_eq!(s.state().tabs().len(), 2);
    for (id, position) in [first, second].iter().zip(positions) { let tab = s.state.tabs.iter().find(|t| t.id == *id).unwrap(); assert_eq!((tab.top_line(), tab.selection()), position); }
    assert!(s.state().view(old).is_none()); assert!(s.state().view(current).is_none());
    s.dispatch(Command::Undo { tab: second }); assert_eq!(source_text(s.state().current_source().unwrap()), text);
    assert_eq!(std::fs::read(path).unwrap(), text.as_bytes());
}

#[test]
fn saved_comparison_rebinds_compacted_baseline_backing_without_a_new_semantic_version() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap(); let original = "a".repeat(256 * 1024); let path = write(d.path(), "compact.txt", original.as_bytes());
    let mut s = session(d.path()); open(&mut s, Command::OpenRetained(path.clone()), "compact.txt"); s.dispatch(Command::CompareSaved);
    let c = s.state().saved_comparison().unwrap(); let (saved, current) = (c.saved_tab(), c.current_tab());
    let expected = s.state().view(current).unwrap().snapshot().key();
    s.dispatch(Command::Edit { tab: current, expected, edits: vec![TextEdit { range: 0..original.len() - 16384, text: "".into() }], kind: EditKind::Replace });
    s.dispatch(Command::Save { tab: current });
    settle(&mut s, |s| matches!(s.state().view(current).unwrap().save_status(), crate::save::Status::Ready) && !s.state().saved_comparison().unwrap().pending());
    assert!(s.state().view(saved).unwrap().snapshot().storage_bytes() < 64 * 1024);
    assert_eq!(s.state().view(saved).unwrap().snapshot().key().version, s.state().view(current).unwrap().snapshot().key().version);
    assert!(s.state().saved_comparison().unwrap().changes().is_empty());
    s.dispatch(Command::CloseSavedComparison); let ordinary = s.state().active().unwrap().id(); s.dispatch(Command::Undo { tab: ordinary });
    assert_eq!(source_text(s.state().current_source().unwrap()), original); assert_eq!(std::fs::read(path).unwrap(), vec![b'a'; 16384]);
}

#[test]
fn saved_comparison_advances_only_the_confirmed_captured_save_and_rejects_old_diffs() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap(); let path = write(d.path(), "captured.txt", b"v1\n");
    let mut s = session(d.path()); open(&mut s, Command::OpenRetained(path.clone()), "captured.txt");
    s.dispatch(Command::CompareSaved);
    let (left, right) = { let c = s.state().saved_comparison().unwrap(); (c.saved_tab(), c.current_tab()) };
    // Keep a real completed worker reply queued while the comparison's inputs change.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let old = loop {
        assert!(std::time::Instant::now() < deadline);
        if let Ok(reply) = s.completions().try_recv() {
            if matches!(&reply, Completion::Analyzed { result, .. } if matches!(result.as_ref(), Completion::SavedCompared { .. })) { break reply; }
            s.dispatch(reply);
        } else { std::thread::yield_now(); }
    };
    let expected = s.state().view(right).unwrap().snapshot().key();
    s.dispatch(Command::Edit { tab: right, expected, edits: vec![TextEdit { range: 0..3, text: "captured v2\n".into() }], kind: EditKind::Replace });
    s.dispatch(old);
    assert!(s.state().saved_comparison().unwrap().pending());
    assert!(s.state().saved_comparison().unwrap().changes().is_empty());
    s.dispatch(Command::Save { tab: right });
    let expected = s.state().view(right).unwrap().snapshot().key();
    s.dispatch(Command::Edit { tab: right, expected, edits: vec![TextEdit { range: 0..12, text: "newer v3\n".into() }], kind: EditKind::Replace });
    settle(&mut s, |s| matches!(s.state().view(right).unwrap().save_status(), crate::save::Status::Ready) && !s.state().saved_comparison().unwrap().pending());
    assert_eq!(source_text(s.state().view(left).unwrap().snapshot()), "captured v2\n");
    assert_eq!(source_text(s.state().view(right).unwrap().snapshot()), "newer v3\n");
    assert!(s.state().view(right).unwrap().dirty());
    assert_eq!(std::fs::read(path).unwrap(), b"captured v2\n");
    s.dispatch(Command::CloseTab(left));
    assert!(s.state().saved_comparison().is_none());
    s.dispatch(Command::Undo { tab: s.state().active().unwrap().id() });
    assert_eq!(source_text(s.state().current_source().unwrap()), "captured v2\n");
    assert!(!active_view(&s).dirty());
}

#[test]
fn saved_comparison_keeps_the_confirmed_side_after_rejected_or_uncertain_save() {
    use lenz_content::write::readiness_fault::{at_path, Fault};
    use crate::text::{EditKind, TextEdit};
    for fault in [Fault::BeforeReplace, Fault::AfterReplace] {
        let d = tempfile::tempdir().unwrap(); let path = write(d.path(), "fault.txt", b"confirmed\n");
        let mut s = session(d.path()); open(&mut s, Command::OpenRetained(path.clone()), "fault.txt");
        s.dispatch(Command::CompareSaved); let c = s.state().saved_comparison().unwrap(); let (left, right) = (c.saved_tab(), c.current_tab());
        let expected = s.state().view(right).unwrap().snapshot().key();
        s.dispatch(Command::Edit { tab: right, expected, edits: vec![TextEdit { range: 0..10, text: "attempted\n".into() }], kind: EditKind::Replace });
        let _fault = at_path(&path, fault); s.dispatch(Command::Save { tab: right });
        settle(&mut s, |s| matches!(s.state().view(right).unwrap().save_status(), crate::save::Status::Rejected(_) | crate::save::Status::Uncertain(_)));
        assert_eq!(source_text(s.state().view(left).unwrap().snapshot()), "confirmed\n");
        assert_eq!(source_text(s.state().view(right).unwrap().snapshot()), "attempted\n");
        assert!(s.state().view(right).unwrap().dirty());
        assert_eq!(std::fs::read(path).unwrap(), if fault == Fault::AfterReplace { b"attempted\n" } else { b"confirmed\n" });
    }
}

#[test]
fn saved_comparison_finds_both_versions_without_becoming_current_file_authority() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap(); let path = write(d.path(), "versions.txt", b"saved needle\n");
    let mut s = session(d.path()); open(&mut s, Command::OpenRetained(path.clone()), "versions.txt");
    let ordinary = s.state().active().unwrap().id(); let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab: ordinary, expected, edits: vec![TextEdit { range: 0..13, text: "draft needle\n".into() }], kind: EditKind::Replace });
    let (started, entered) = std::sync::mpsc::channel();
    let gates: Vec<_> = (0..crate::jobs::MAX_JOBS).map(|_| {
        let (release, wait) = std::sync::mpsc::channel(); let started = started.clone();
        s.jobs.submit(Priority::Visible, move || { started.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); }); release
    }).collect();
    for _ in &gates { entered.recv_timeout(Duration::from_secs(5)).unwrap(); }
    s.dispatch(Command::Find("draft".into()));
    s.dispatch(Command::CompareSaved);
    let comparison = s.state().saved_comparison().unwrap();
    let (saved, current) = (comparison.saved_tab(), comparison.current_tab());
    let synthetic = s.state().view(saved).unwrap().snapshot().key().buffer;
    let current_buffer = s.state().view(current).unwrap().snapshot().key().buffer;
    assert_ne!(synthetic, current_buffer);
    s.dispatch(Command::ActivateTab(saved));
    assert!(s.state().saved_comparison().is_some(), "selecting a comparison tab focuses its side");
    s.dispatch(Command::Find("saved".into()));
    for gate in gates { gate.send(()).unwrap(); }
    settle(&mut s, |s| [saved, ordinary].iter().all(|id| s.state.tabs.iter().find(|t| t.id == *id).unwrap().find().is_some_and(|find| !find.pending())));
    assert_eq!(s.state.tabs.iter().find(|t| t.id == saved).unwrap().find().unwrap().matches().len(), 1);
    assert_eq!(s.state.tabs.iter().find(|t| t.id == ordinary).unwrap().find().unwrap().matches().len(), 1);
    assert_eq!(s.state.sources.aliases.get(&path), Some(&current_buffer));
    assert_eq!(s.captured_project_sources(d.path())[&path].identity.key.buffer, current_buffer);
    assert!(s.state().provider().is_none(), "saved text never becomes provider authority");
    let ordinary_source = s.state.sources.buffers.remove(&current_buffer).unwrap();
    assert!(Session::target_source(&s.state, &path).is_none(), "the immutable saved side cannot become semantic target authority even without a current snapshot");
    s.state.sources.buffers.insert(current_buffer, ordinary_source);
    assert_eq!(Session::target_source(&s.state, &path).unwrap().key().buffer, current_buffer);
    s.dispatch(Command::CloseSavedComparison);
    assert!(!s.state.sources.buffers.contains_key(&synthetic));
    assert_eq!(source_text(s.state().current_source().unwrap()), "draft needle\n");
    assert_eq!(std::fs::read(path).unwrap(), b"saved needle\n");
}

#[test]
fn saved_comparison_edits_the_ordinary_buffer_and_restores_its_view() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap();
    let file = write(d.path(), "compare.txt", "α saved\r\nsecond\n".as_bytes());
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(file.clone()), "compare.txt");
    let ordinary = s.state().active().unwrap().id();
    s.dispatch(Command::Select(Selection { anchor: 3, head: 3 }));
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab: ordinary, expected, edits: vec![TextEdit { range: 3..8, text: "draft".into() }], kind: EditKind::Replace });
    let ordinary_position = s.state().selection();
    s.dispatch(Command::CompareSaved);
    let comparison = s.state().saved_comparison().unwrap();
    let (saved, current) = (comparison.saved_tab(), comparison.current_tab());
    assert_eq!(source_text(s.state().view(saved).unwrap().snapshot()), "α saved\r\nsecond\n");
    assert_eq!(s.state().view(saved).unwrap().edit_disabled(), Some("The last saved version is read-only"));
    assert_eq!(s.state().view(current).unwrap().snapshot().key().buffer, s.state().view(ordinary).unwrap().snapshot().key().buffer);
    let expected = s.state().view(current).unwrap().snapshot().key();
    s.dispatch(Command::Edit { tab: current, expected, edits: vec![TextEdit { range: 3..8, text: "current".into() }], kind: EditKind::Replace });
    assert_eq!(source_text(s.state().view(ordinary).unwrap().snapshot()), "α current\r\nsecond\n");
    let old = s.state().view(saved).unwrap().snapshot().key();
    s.dispatch(Command::Edit { tab: saved, expected: old, edits: vec![TextEdit { range: 0..0, text: "forbidden".into() }], kind: EditKind::Paste });
    s.dispatch(Command::Undo { tab: saved });
    assert_eq!(source_text(s.state().view(saved).unwrap().snapshot()), "α saved\r\nsecond\n");
    s.dispatch(Command::Undo { tab: current });
    assert_eq!(source_text(s.state().view(current).unwrap().snapshot()), "α draft\r\nsecond\n");
    settle(&mut s, |s| s.state().saved_comparison().is_some_and(|comparison| !comparison.pending()));
    assert_eq!(s.state().saved_comparison().unwrap().changes().len(), 1);
    s.dispatch(Command::CloseSavedComparison);
    assert_eq!(s.state().active().unwrap().id(), ordinary);
    assert_eq!(s.state().selection(), ordinary_position);
    assert!(s.state().view(ordinary).unwrap().can_redo());
    assert_eq!(std::fs::read(file).unwrap(), "α saved\r\nsecond\n".as_bytes());
}

#[test]
fn promoted_panel_and_symbol_targets_keep_exact_save_provenance() {
    use crate::navigation::{Chosen, Scope as NavigationScope};
    use crate::text::{EditKind, TextEdit};
    for navigation in [false, true] {
        let d = tempfile::tempdir().unwrap(); let root = d.path().join("project"); std::fs::create_dir(&root).unwrap();
        let original = b"\xef\xbb\xbffn UniquePromoted() {}\r\n";
        let file = write(&root, "a.rs", original); let other = write(&root, "other.txt", b"stay here\n");
        let mut s = session(d.path()); s.dispatch(Command::OpenRoot(root)); settle(&mut s, |s| scanned(s) && s.state().index_coverage().complete());
        open(&mut s, Command::OpenRetained(other), "other.txt"); s.cache = InactiveCache::new(0);
        if navigation {
            s.dispatch(Command::SearchEverywhere { scope: NavigationScope::Symbols, query: "UniquePromoted".into() }); settle(&mut s, |s| s.state().navigation().is_some_and(|n| !n.pending));
            let revision = s.state().navigation().unwrap().revision;
            let Some(Chosen::Command(command)) = s.navigation_choice(revision, 0) else { panic!("captured disk symbol") }; s.dispatch(command);
        } else {
            s.dispatch(Command::FindDeclarations("UniquePromoted".into())); settle(&mut s, searched); s.dispatch(Command::OpenResult(0));
        }
        settle(&mut s, |s| s.state().current_source().is_some_and(|source| source.source() == file) && s.state().loading().is_none());
        assert_eq!(active_view(&s).save_status(), &crate::save::Status::Ready, "checked payload must retain opened-file provenance");
        let tab = s.state().active().unwrap().id(); let expected = s.state().current_source().unwrap().key();
        s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..0, text: "// edited\r\n".into() }], kind: EditKind::Paste });
        s.dispatch(Command::Save { tab }); settle(&mut s, |s| !active_view(s).dirty() && *active_view(s).save_status() == crate::save::Status::Ready);
        assert_eq!(std::fs::read(file).unwrap(), b"\xef\xbb\xbf// edited\r\nfn UniquePromoted() {}\r\n");
    }
}

#[test]
fn save_keeps_unicode_history_across_real_cache_eviction_and_undo() {
    use crate::text::{EditKind, TextEdit};
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "save.txt", b"\xef\xbb\xbf\xce\xb1\xf0\x9f\x98\x80\r\nbeta\n");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o751)).unwrap();
    let other = write(d.path(), "other.txt", b"other\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path.clone()), "save.txt");
    let tab = s.state().active().unwrap().id();
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..2, text: "é".into() }], kind: EditKind::Replace });
    s.dispatch(Command::Save { tab });
    settle(&mut s, |s| !s.state().tab_dirty(tab));
    assert_eq!(std::fs::read(&path).unwrap(), "\u{feff}é😀\r\nbeta\n".as_bytes());
    assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o7777, 0o751);
    s.cache = InactiveCache::new(0);
    open(&mut s, Command::OpenRetained(other), "other.txt");
    assert!(s.cache.is_empty());
    open(&mut s, Command::OpenRetained(path.clone()), "save.txt");
    assert!(active_view(&s).can_undo());
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "α😀\r\nbeta\n");
    assert!(active_view(&s).dirty());
    assert_eq!(std::fs::read(&path).unwrap(), "\u{feff}é😀\r\nbeta\n".as_bytes());
    s.dispatch(Command::Redo { tab });
    assert!(!active_view(&s).dirty());
}
#[test]
fn delayed_save_confirms_only_captured_version_and_refreshes_the_next_target() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap(); let path = write(d.path(), "delayed.txt", b"original\n");
    let mut s = session(d.path()); open(&mut s, Command::OpenRetained(path.clone()), "delayed.txt");
    let tab = s.state().active().unwrap().id();
    let edit = |s: &mut Session, text: &str| {
        let current = s.state().current_source().unwrap();
        s.dispatch(Command::Edit { tab, expected: current.key(), edits: vec![TextEdit { range: 0..current.len_bytes(), text: text.into() }], kind: EditKind::Replace });
    };
    let (started, entered) = std::sync::mpsc::channel();
    let gates: Vec<_> = (0..crate::jobs::MAX_JOBS).map(|_| {
        let (release, wait) = std::sync::mpsc::channel(); let started = started.clone();
        s.jobs.submit(Priority::Visible, move || { started.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); }); release
    }).collect();
    for _ in &gates { entered.recv_timeout(Duration::from_secs(5)).unwrap(); }
    edit(&mut s, "saved v2\n");
    s.dispatch(Command::Save { tab });
    assert!(matches!(active_view(&s).save_status(), crate::save::Status::Saving));
    edit(&mut s, "newer v3\n");
    s.dispatch(Command::Save { tab });
    for gate in gates { gate.send(()).unwrap(); }
    settle(&mut s, |s| matches!(active_view(s).save_status(), crate::save::Status::Ready));
    assert_eq!(std::fs::read(&path).unwrap(), b"saved v2\n");
    assert_eq!(source_text(s.state().current_source().unwrap()), "newer v3\n");
    assert!(active_view(&s).dirty());
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "saved v2\n");
    assert!(!active_view(&s).dirty());
    s.dispatch(Command::Redo { tab });
    s.dispatch(Command::Save { tab });
    settle(&mut s, |s| !s.state().tab_dirty(tab));
    assert_eq!(std::fs::read(path).unwrap(), b"newer v3\n");
    assert!(active_view(&s).can_undo());
}

#[test]
fn save_faults_retain_draft_history_and_require_explicit_matched_recheck() {
    use crate::text::{EditKind, TextEdit};
    use lenz_content::write::readiness_fault::{at_path, Fault};
    use crate::save::{Status, DiskMatch};
    for (fault, linked, disk, matched) in [
        (Fault::BeforeReplace, false, b"abcdef".as_slice(), DiskMatch::Baseline),
        (Fault::Write, true, b"abcdef".as_slice(), DiskMatch::Baseline),
        (Fault::WriteAndRollback, true, b"aVWdef".as_slice(), DiskMatch::Changed),
        (Fault::AfterReplace, false, b"UVWXYZ".as_slice(), DiskMatch::Attempted),
    ] {
        let d = tempfile::tempdir().unwrap(); let path = write(d.path(), "fault.txt", b"abcdef");
        if linked { std::fs::hard_link(&path, d.path().join("alias")).unwrap(); }
        let mut s = session(d.path()); open(&mut s, Command::OpenRetained(path.clone()), "fault.txt");
        let tab = s.state().active().unwrap().id(); let expected = s.state().current_source().unwrap().key();
        s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..6, text: "UVWXYZ".into() }], kind: EditKind::Replace });
        let guard = at_path(&path, fault);
        s.dispatch(Command::Save { tab });
        settle(&mut s, |s| !matches!(active_view(s).save_status(), Status::Saving));
        assert_eq!(std::fs::read(&path).unwrap(), disk);
        assert!(active_view(&s).dirty() && active_view(&s).can_undo());
        assert_eq!(source_text(s.state().current_source().unwrap()), "UVWXYZ");
        if fault == Fault::BeforeReplace { assert!(matches!(active_view(&s).save_status(), Status::Rejected(_))); }
        else {
            assert!(matches!(active_view(&s).save_status(), Status::Uncertain(_)));
            assert!(s.disabled(CommandId::Save).is_some());
            s.dispatch(Command::Save { tab });
            assert_eq!(std::fs::read(&path).unwrap(), disk);
        }
        drop(guard);
        s.dispatch(Command::RecheckSave { tab });
        settle(&mut s, |s| matches!(active_view(s).save_status(), Status::Checked(_)));
        assert_eq!(active_view(&s).save_status(), &Status::Checked(matched));
        assert!(active_view(&s).dirty(), "an observation alone never accepts a new baseline");
        s.dispatch(Command::ConfirmRecheckedSave { tab });
        assert_eq!(std::fs::read(&path).unwrap(), disk);
        if matched == DiskMatch::Changed { assert!(s.disabled(CommandId::Save).is_some()); }
        else { assert_eq!(active_view(&s).dirty(), matched != DiskMatch::Attempted); }
        s.dispatch(Command::Undo { tab });
        assert_eq!(source_text(s.state().current_source().unwrap()), "abcdef");
        assert_eq!(active_view(&s).dirty(), matched == DiskMatch::Attempted);
    }
}

#[test]
fn save_rejects_same_content_substitution_without_learning_a_new_baseline() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap(); let path = write(d.path(), "target.txt", b"original");
    let substitute = write(d.path(), "substitute.txt", b"original");
    let mut s = session(d.path()); open(&mut s, Command::OpenRetained(path.clone()), "target.txt");
    let tab = s.state().active().unwrap().id(); let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..8, text: "draft".into() }], kind: EditKind::Replace });
    std::fs::remove_file(&path).unwrap(); std::fs::hard_link(&substitute, &path).unwrap();
    s.dispatch(Command::Save { tab });
    settle(&mut s, |s| matches!(active_view(s).save_status(), crate::save::Status::Rejected(_)));
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    assert_eq!(std::fs::read(substitute).unwrap(), b"original");
    assert!(active_view(&s).dirty() && active_view(&s).can_undo());
    assert_eq!(source_text(s.state().current_source().unwrap()), "draft");
}

#[test]
fn saving_a_small_surviving_slice_releases_retired_original_backing_without_losing_undo() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap(); let original = "a".repeat(256 * 1024);
    let path = write(d.path(), "reclaim.txt", original.as_bytes());
    let mut s = session(d.path()); open(&mut s, Command::OpenRetained(path.clone()), "reclaim.txt");
    let tab = s.state().active().unwrap().id(); let baseline = Arc::downgrade(s.state().current_source().unwrap());
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..original.len() - 16384, text: "".into() }], kind: EditKind::Replace });
    assert!(s.state().current_source().unwrap().storage_bytes() >= original.len());
    s.dispatch(Command::Save { tab });
    settle(&mut s, |s| matches!(active_view(s).save_status(), crate::save::Status::Ready));
    assert_eq!(std::fs::read(&path).unwrap(), vec![b'a'; 16384]);
    assert!(baseline.upgrade().is_none(), "the original confirmed snapshot retired");
    assert!(s.state().current_source().unwrap().storage_bytes() < 64 * 1024, "tiny surviving chunks must not pin the retired full text/index allocation");
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), original);
    assert!(active_view(&s).dirty());
    assert_eq!(std::fs::read(path).unwrap(), vec![b'a'; 16384]);
}

#[test]
fn reader_save_preserves_original_utf16_and_legacy_encoding_and_rejects_lossy_binary_sources() {
    use crate::text::{EditKind, TextEdit};
    for (name, old, new) in [
        ("little.txt", vec![0xff, 0xfe, b'a', 0, 13, 0, 10, 0], vec![0xff, 0xfe, 0xe9, 0, 13, 0, 10, 0]),
        ("big.txt", vec![0xfe, 0xff, 0, b'a', 0, 13, 0, 10], vec![0xfe, 0xff, 0, 0xe9, 0, 13, 0, 10]),
        ("legacy.txt", b"caf\xe9\r\n".to_vec(), b"x\xe9\r\n".to_vec()),
    ] {
        let d = tempfile::tempdir().unwrap(); let path = write(d.path(), name, &old);
        let mut s = session(d.path()); open(&mut s, Command::OpenRetained(path.clone()), name);
        let tab = s.state().active().unwrap().id(); let expected = s.state().current_source().unwrap().key();
        let (range, text) = if name == "legacy.txt" { (0..3, "x") } else { (0..1, "é") };
        s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range, text: text.into() }], kind: EditKind::Replace });
        s.dispatch(Command::Save { tab }); settle(&mut s, |s| !s.state().tab_dirty(tab));
        assert_eq!(std::fs::read(path).unwrap(), new);
    }
    for (name, bytes) in [("lossy.txt", vec![0xff, 0xfe, 0x00, 0xd8]), ("binary.txt", b"a\0b".to_vec())] {
        let d = tempfile::tempdir().unwrap(); let path = write(d.path(), name, &bytes);
        let mut s = session(d.path()); open(&mut s, Command::OpenRetained(path.clone()), name);
        assert!(active_view(&s).edit_disabled().is_some()); assert!(s.disabled(CommandId::Save).is_some());
        let tab = s.state().active().unwrap().id(); s.dispatch(Command::Save { tab });
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn saving_newline_dense_text_finishes_without_repacking_its_legitimate_index_forever() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap(); let path = write(d.path(), "dense.txt", "a\n".repeat(50000).as_bytes());
    let mut s = session(d.path()); open(&mut s, Command::OpenRetained(path.clone()), "dense.txt");
    let tab = s.state().active().unwrap().id(); let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..1, text: "b".into() }], kind: EditKind::Replace });
    s.dispatch(Command::Save { tab });
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while !matches!(active_view(&s).save_status(), crate::save::Status::Ready) {
        assert!(std::time::Instant::now() < deadline, "legitimate live newline indexes must not trigger endless compaction");
        if let Ok(c) = s.completions().try_recv() { s.dispatch(c); } else { std::thread::yield_now(); }
    }
    assert!(!active_view(&s).dirty()); assert_eq!(std::fs::read(path).unwrap().len(), 100000);
}

#[test]
fn save_refuses_a_provenance_less_source_without_reading_a_new_disk_baseline() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap(); let path = write(d.path(), "unproven.txt", b"disk");
    let mut s = session(d.path()); s.dispatch(Command::OpenRetained(path.clone()));
    let tab = s.state().active().unwrap().id();
    let version = s.versions.fetch_add(1, Ordering::Relaxed) + 1;
    s.dispatch(Completion::Loaded { request: s.request, tab, path: path.clone(), result: Ok((Stamp::of(&path), Arc::new(Document::decode(path.clone(), version, b"disk")), Some(path.clone()))) });
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..4, text: "draft".into() }], kind: EditKind::Replace });
    assert!(s.disabled(CommandId::Save).is_some_and(|why| why.contains("confirmed write target")));
    s.dispatch(Command::Save { tab });
    assert_eq!(std::fs::read(path).unwrap(), b"disk"); assert!(active_view(&s).dirty());
}

#[test]
fn save_never_confuses_a_renamed_displayed_path_with_its_old_confirmed_target() {
    use crate::text::{EditKind, TextEdit};
    use crate::save::Status;
    for pending in [false, true] {
        let d = tempfile::tempdir().unwrap(); let old = write(d.path(), "old.txt", b"original"); let new = d.path().join("new.txt");
        let mut s = session(d.path()); open(&mut s, Command::OpenRetained(old.clone()), "old.txt");
        let tab = s.state().active().unwrap().id(); let expected = s.state().current_source().unwrap().key();
        s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..8, text: "draft".into() }], kind: EditKind::Replace });
        let (started, entered) = std::sync::mpsc::channel();
        let gates: Vec<_> = if pending { (0..crate::jobs::MAX_JOBS).map(|_| {
            let (release, wait) = std::sync::mpsc::channel(); let started = started.clone();
            s.jobs.submit(Priority::Visible, move || { started.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); }); release
        }).collect() } else { Vec::new() };
        for _ in &gates { entered.recv_timeout(Duration::from_secs(5)).unwrap(); }
        if pending { s.dispatch(Command::Save { tab }); }
        std::fs::rename(&old, &new).unwrap();
        let token = s.state().active().unwrap().refresh;
        s.dispatch(Completion::Moved { tab, token, to: new.clone(), stamp: Stamp::of(&new), canonical: Some(new.clone()) });
        std::fs::hard_link(&new, &old).unwrap();
        let replacement = write(d.path(), "replacement", b"original"); std::fs::rename(replacement, &new).unwrap();
        if !pending { s.dispatch(Command::Save { tab }); }
        for gate in gates { gate.send(()).unwrap(); }
        settle(&mut s, |s| !matches!(active_view(s).save_status(), Status::Saving));
        assert_eq!(std::fs::read(&old).unwrap(), if pending { b"draft".as_slice() } else { b"original".as_slice() });
        assert_eq!(std::fs::read(&new).unwrap(), b"original");
        assert!(active_view(&s).dirty(), "a write to the captured old path cannot confirm the new named target");
        assert!(s.disabled(CommandId::Save).is_some()); assert!(s.disabled(CommandId::RecheckSave).is_none());
        s.dispatch(Command::RecheckSave { tab }); settle(&mut s, |s| matches!(active_view(s).save_status(), Status::Checked(_)));
        s.dispatch(Command::ConfirmRecheckedSave { tab });
        s.dispatch(Command::Save { tab }); settle(&mut s, |s| !s.state().tab_dirty(tab));
        assert_eq!(std::fs::read(new).unwrap(), b"draft");
        assert_eq!(std::fs::read(old).unwrap(), if pending { b"draft".as_slice() } else { b"original".as_slice() });
    }
}

fn active_view(s: &Session) -> View<'_> { s.state().view(s.state().active().unwrap().id()).unwrap() }
fn pane_view(s: &Session, pane: u8) -> View<'_> { s.state().view(s.state().shown(pane).unwrap().id()).unwrap() }
fn source_rows(view: View<'_>, count: usize, cols: Option<usize>) -> Vec<crate::layout::Row> {
    let rows = view.layout_rows(count, cols);
    assert!(rows.pending.is_none(), "fixture layout must be ready before asserting rows");
    rows.rows
}
fn analysis_document(s: &mut Session) -> Arc<Document> {
    let tab = s.state().active().unwrap().id();
    let until = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(doc) = s.admitted_document(tab, Priority::Visible) { return doc; }
        assert!(std::time::Instant::now() < until, "analysis representation did not become ready");
        s.dispatch(Completion::AnalysisWake);
        if let Ok(completion) = s.completions().try_recv() { s.dispatch(completion); }
        else { std::thread::yield_now(); }
    }
}


fn session(dir: &Path) -> Session { session_clock(dir, Arc::new(std::time::Instant::now)) }
fn session_clock(dir: &Path, clock: semantics::Clock) -> Session {
    // Never a real language server: the settings name one that does not exist.
    let providers = dir.join("config/lenz-reader/providers.json");
    std::fs::create_dir_all(providers.parent().unwrap()).unwrap();
    let none: serde_json::Map<String, serde_json::Value> = crate::semantics::provider::PROVIDERS.iter().map(|p| (p.id.to_string(), serde_json::json!({ "executable": format!("/nonexistent/{}", p.id) }))).collect();
    std::fs::write(&providers, serde_json::json!({ "providers": none }).to_string()).unwrap();
    Session::new(Config {
        themes: ThemePaths::under(&dir.join("config"), &dir.join("state"), "lenz-reader"),
        recent: dir.join("state/lenz-reader/recent-roots.json"),
        settings: dir.join("state/lenz-reader/settings.json"),
        providers: dir.join("config/lenz-reader/providers.json"),

        cache: dir.join("cache"),
        idle: crate::semantics::IDLE,
        clock,
        overrides: ThemeOverrides::default(),
        dark: true,
    })
}

/// Apply background results until `done` holds.
fn settle(s: &mut Session, done: impl Fn(&Session) -> bool) {
    let rx = s.completions();
    let until = std::time::Instant::now() + Duration::from_secs(10);
    while !done(s) {
        assert!(std::time::Instant::now() < until, "timed out");
        if let Ok(c) = rx.recv_blocking_timeout() {
            s.dispatch(c);
        }
    }
}

trait RecvTimeout {
    fn recv_blocking_timeout(&self) -> Result<Completion, ()>;
}

impl RecvTimeout for async_channel::Receiver<Completion> {
    /// Spins (yielding) rather than sleeping, so timings measure the work, not a poll interval.
    fn recv_blocking_timeout(&self) -> Result<Completion, ()> {
        let until = std::time::Instant::now() + Duration::from_secs(2);
        while std::time::Instant::now() < until {
            if let Ok(c) = self.try_recv() {
                return Ok(c);
            }
            std::thread::yield_now();
        }
        Err(())
    }
}

fn doc_bytes(s: &Session, name: &str) -> usize {
    s.cache_entries().iter().find(|d| d.source().ends_with(name)).map_or(0, |source| source.storage_bytes())
}

#[cfg(unix)]
#[test]
fn validated_result_promotes_its_checked_payload_when_the_inactive_cache_refuses_it() {
    let d = tempfile::tempdir().unwrap(); let root = d.path().join("project"); std::fs::create_dir(&root).unwrap();
    let file = write(&root, "a.txt", b"needle captured\n"); let other = write(&root, "other.txt", b"stay here\n");
    let mut s = session(d.path()); s.dispatch(Command::OpenRoot(root)); settle(&mut s, scanned);
    open(&mut s, Command::OpenRetained(other), "other.txt"); s.cache = InactiveCache::new(0);
    s.dispatch(Command::Search { query: "needle".into(), scope: SearchIn::Project }); settle(&mut s, searched);
    s.dispatch(Command::OpenResult(0));
    let completion = loop {
        let completion = s.completions().recv_blocking_timeout().unwrap();
        if matches!(&completion, Completion::Analyzed { result, .. } if matches!(result.as_ref(), Completion::Validated { .. })) { break completion; }
        s.dispatch(completion);
    };
    std::fs::write(&file, b"new unrelated\n").unwrap();
    s.dispatch(completion);
    let source = s.state().current_source().expect("activation must publish its checked source without another disk read");
    assert_eq!(source_text(source), "needle captured\n");
    assert_eq!(s.state().selected_text().as_deref(), Some("needle"));
    assert!(s.state().loading().is_none(), "the redundant load has been superseded");
    while let Ok(completion) = s.completions().try_recv() {
        if matches!(completion, Completion::Loaded { .. } | Completion::AnalysisWake) { s.dispatch(completion); }
    }
    assert_eq!(source_text(s.state().current_source().unwrap()), "needle captured\n");
    assert_eq!(std::fs::read(file).unwrap(), b"new unrelated\n");
}

#[test]
fn closing_symbol_navigation_preserves_panel_capture_and_releases_its_own_query() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap(); let root = d.path().join("project"); std::fs::create_dir(&root).unwrap();
    let file = write(&root, "a.rs", b"fn Saved() {}\n");
    let mut s = session(d.path()); s.dispatch(Command::OpenRoot(root)); settle(&mut s, |s| scanned(s) && s.state().index_coverage().complete());
    open(&mut s, Command::OpenRetained(file.clone()), "a.rs"); let tab = s.state().active().unwrap().id();
    let replace = |s: &mut Session, text: &str| {
        let source = s.state().current_source().unwrap(); let (expected, len) = (source.key(), source.len_bytes());
        s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..len, text: text.into() }], kind: EditKind::Replace });
    };
    replace(&mut s, "fn OldDraftOnly() {}\n"); let old = Arc::downgrade(s.state().current_source().unwrap());
    let (started, starts) = std::sync::mpsc::channel();
    let releases: Vec<_> = (0..crate::jobs::MAX_JOBS).map(|_| {
        let (release, wait) = std::sync::mpsc::channel(); let started = started.clone();
        s.jobs.submit(Priority::Visible, move || { started.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); }); release
    }).collect();
    for _ in &releases { starts.recv_timeout(Duration::from_secs(5)).unwrap(); }
    s.dispatch(Command::Search { query: "OldDraftOnly".into(), scope: SearchIn::Project });
    let panel = s.state().results().unwrap().id();
    s.dispatch(Command::SearchEverywhere { scope: crate::navigation::Scope::Symbols, query: "OldDraftOnly".into() });
    replace(&mut s, "fn NewerCurrent() {}\n");
    s.dispatch(Command::CloseSearchEverywhere);
    assert!(s.state().navigation().is_none()); assert_eq!(s.state().results().unwrap().id(), panel);
    assert!(old.upgrade().is_some(), "independent panel query still captures the old version");
    for release in releases { release.send(()).unwrap(); }
    settle(&mut s, searched);
    assert_eq!(s.state().results().unwrap().hits()[0].text.as_ref(), "fn OldDraftOnly() {}");
    assert!(s.state().navigation().is_none(), "cancelled modal completion cannot resurrect it");
    s.dispatch(Command::CloseResults); settle(&mut s, |_| old.upgrade().is_none());
    assert_eq!(source_text(s.state().current_source().unwrap()), "fn NewerCurrent() {}\n");
    assert_eq!(std::fs::read(file).unwrap(), b"fn Saved() {}\n");
}

#[test]
fn scoped_declaration_records_keep_disk_identity_after_a_clean_view_is_evicted() {
    use crate::navigation::{Chosen, Scope as NavigationScope};
    let d = tempfile::tempdir().unwrap(); let root = d.path().join("project"); std::fs::create_dir(&root).unwrap();
    let file = write(&root, "a.rs", b"// before\nfn UniqueEvictedSymbol() {}\n"); let other = write(&root, "other.txt", b"stay here\n");
    let mut s = session(d.path()); s.dispatch(Command::OpenRoot(root)); settle(&mut s, |s| scanned(s) && s.state().index_coverage().complete());
    open(&mut s, Command::OpenRetained(file.clone()), "a.rs"); s.cache = InactiveCache::new(0);
    s.dispatch(Command::SearchEverywhere { scope: NavigationScope::Symbols, query: "UniqueEvictedSymbol".into() }); settle(&mut s, |s| s.state().navigation().is_some_and(|n| !n.pending));
    s.dispatch(Command::CloseSearchEverywhere);
    open(&mut s, Command::OpenRetained(other.clone()), "other.txt");
    s.dispatch(Command::SearchEverywhere { scope: NavigationScope::Symbols, query: "UniqueEvictedSymbol".into() }); settle(&mut s, |s| s.state().navigation().is_some_and(|n| !n.pending));
    let revision = s.state().navigation().unwrap().revision;
    let Some(Chosen::Command(command)) = s.navigation_choice(revision, 0) else { panic!("captured disk symbol") };
    s.dispatch(command); settle(&mut s, |s| s.state().navigation().is_none() && s.state().current_source().is_some_and(|source| source.source() == file) || s.navigation_status().contains("search again"));
    assert!(s.state().navigation().is_none(), "unchanged evicted source remains valid");
    assert_eq!(s.state().current_source().unwrap().source(), file.as_path());
    open(&mut s, Command::OpenRetained(other.clone()), "other.txt");
    s.dispatch(Command::SearchEverywhere { scope: NavigationScope::Symbols, query: "UniqueEvictedSymbol".into() }); settle(&mut s, |s| s.state().navigation().is_some_and(|n| !n.pending));
    let revision = s.state().navigation().unwrap().revision;
    let Some(Chosen::Command(command)) = s.navigation_choice(revision, 0) else { panic!("second captured disk symbol") };
    let stamp = Stamp::of(&file).unwrap(); std::fs::write(&file, b"// change\nfn UniqueEvictedSymbol() {}\n").unwrap();
    std::fs::File::options().write(true).open(&file).unwrap().set_modified(stamp.modified.unwrap()).unwrap(); assert_eq!(Stamp::of(&file), Some(stamp));
    s.dispatch(command); settle(&mut s, |s| s.state().navigation().is_none() || s.navigation_status().contains("search again"));
    assert!(s.state().navigation().is_some()); assert!(s.navigation_status().contains("source changed"));
    assert_eq!(s.state().current_source().unwrap().source(), other.as_path());
}

#[test]
fn navigation_symbol_queries_overlay_drafts_and_keep_panel_search_independent() {
    use crate::text::{EditKind, TextEdit};
    use crate::navigation::{Scope as NavigationScope, Kind, Chosen};
    let d = tempfile::tempdir().unwrap(); let root = d.path().join("project"); std::fs::create_dir(&root).unwrap();
    let file = write(&root, "a.rs", b"fn RemovedZebraSymbol() {}\n");
    write(&root, "b.rs", b"fn DiskZebraSymbol() {}\n");
    let mut s = session(d.path()); s.dispatch(Command::OpenRoot(root)); settle(&mut s, |s| scanned(s) && s.state().index_coverage().complete());
    open(&mut s, Command::OpenRetained(file.clone()), "a.rs"); let tab = s.state().active().unwrap().id();
    let source = s.state().current_source().unwrap(); let (expected, len) = (source.key(), source.len_bytes());
    s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..len, text: "\nfn UnsavedZebraSymbol() {}\n".into() }], kind: EditKind::Replace });
    s.dispatch(Command::Search { query: "ZebraSymbol".into(), scope: SearchIn::Project }); settle(&mut s, searched);
    let panel = s.state().results().unwrap().id();
    for scope in [NavigationScope::All, NavigationScope::Symbols] {
        s.dispatch(Command::SearchEverywhere { scope, query: "ZebraSymbol".into() }); settle(&mut s, |s| s.state().navigation().is_some_and(|n| !n.pending));
        let mut symbols: Vec<_> = s.state().navigation().unwrap().rows.iter().filter(|row| row.kind == Kind::Symbol).map(|row| row.label.as_str()).collect(); symbols.sort();
        assert_eq!(symbols, ["DiskZebraSymbol", "UnsavedZebraSymbol"]);
        assert_eq!(s.state().results().unwrap().id(), panel);
        s.dispatch(Command::CloseSearchEverywhere);
        assert!(s.state().results().unwrap().done());
    }
    s.dispatch(Command::SearchEverywhere { scope: NavigationScope::Symbols, query: "RemovedZebraSymbol".into() }); settle(&mut s, |s| s.state().navigation().is_some_and(|n| !n.pending));
    assert!(s.state().navigation().unwrap().rows.is_empty());
    s.dispatch(Command::SearchEverywhere { scope: NavigationScope::Symbols, query: "UnsavedZebraSymbol".into() }); settle(&mut s, |s| s.state().navigation().is_some_and(|n| !n.pending));
    let revision = s.state().navigation().unwrap().revision;
    let Some(Chosen::Command(command)) = s.navigation_choice(revision, 0) else { panic!("symbol activation is a shared command") };
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..0, text: "// shifted\n".into() }], kind: EditKind::Typing });
    let selection = s.state().selection(); s.dispatch(command);
    assert_eq!(s.state().selection(), selection);
    assert!(s.state().navigation().is_some()); assert!(s.navigation_status().contains("source changed"));
    assert_eq!(s.state().results().unwrap().id(), panel);
    assert_eq!(std::fs::read(file).unwrap(), b"fn RemovedZebraSymbol() {}\n");
}

#[test]
fn project_overlay_follows_canonical_aliases_without_searching_the_saved_source() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap(); let root = d.path().join("project"); std::fs::create_dir(&root).unwrap();
    let real = write(&root, "real.txt", b"saved source\n"); let alias = root.join("alias.txt"); std::os::unix::fs::symlink(&real, &alias).unwrap();
    let mut s = session(d.path()); s.dispatch(Command::OpenRoot(root)); settle(&mut s, scanned);
    open(&mut s, Command::OpenRetained(alias), "alias.txt"); let tab = s.state().active().unwrap().id();
    let source = s.state().current_source().unwrap(); let (expected, len) = (source.key(), source.len_bytes());
    s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..len, text: "\nneedle alias draft\n".into() }], kind: EditKind::Replace });
    s.dispatch(Command::Search { query: "needle".into(), scope: SearchIn::Matching("real.txt".into()) }); settle(&mut s, searched);
    let r = s.state().results().unwrap(); assert_eq!(r.hits().len(), 1); assert_eq!((r.hits()[0].path.as_ref(), r.hits()[0].line), ("real.txt", 1));
    s.dispatch(Command::OpenResult(0)); settle(&mut s, |s| s.state().selected_text().as_deref() == Some("needle") || s.state().results().unwrap().is_stale(0));
    assert!(!s.state().results().unwrap().is_stale(0));
    assert_eq!(s.state().current_source().unwrap().key().buffer, expected.buffer);
    assert_eq!(source_text(s.state().current_source().unwrap()), "\nneedle alias draft\n");
    assert_eq!(std::fs::read(real).unwrap(), b"saved source\n");
}

#[test]
fn held_result_validation_rejects_version_preserving_metadata_rename() {
    let d = tempfile::tempdir().unwrap(); let root = d.path().join("project"); std::fs::create_dir(&root).unwrap();
    let a = write(&root, "a.txt", b"needle original\n"); let moved = root.join("renamed.txt");
    let mut s = session(d.path()); s.dispatch(Command::OpenRoot(root)); settle(&mut s, scanned);
    open(&mut s, Command::OpenRetained(a.clone()), "a.txt");
    s.dispatch(Command::Search { query: "needle".into(), scope: SearchIn::Project }); settle(&mut s, searched);
    let tab = s.state().active().unwrap().id(); let original = s.state().current_source().unwrap().key();
    s.dispatch(Command::Select(Selection { anchor: 5, head: 5 })); s.dispatch(Command::OpenResult(0));
    let completion = loop {
        let completion = s.completions().recv_blocking_timeout().unwrap();
        if matches!(&completion, Completion::Analyzed { result, .. } if matches!(result.as_ref(), Completion::Validated { .. })) { break completion; }
        s.dispatch(completion);
    };
    std::fs::rename(&a, &moved).unwrap();
    let token = s.state().active().unwrap().refresh;
    s.dispatch(Completion::Moved { tab, token, to: moved.clone(), stamp: Stamp::of(&moved), canonical: std::fs::canonicalize(&moved).ok() });
    assert_eq!(s.state().current_source().unwrap().key(), original, "rename preserves the text version");
    s.dispatch(completion);
    assert!(s.state().results().unwrap().is_stale(0)); assert_eq!(s.state().selection(), Some(Selection { anchor: 5, head: 5 }));
    assert_eq!(s.state().current_source().unwrap().source(), moved.as_path());
    assert_eq!(std::fs::read(moved).unwrap(), b"needle original\n");
}

#[test]
fn held_result_validation_rejects_a_new_version_even_when_undo_restores_identical_text() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap(); let root = d.path().join("project"); std::fs::create_dir(&root).unwrap();
    let file = write(&root, "a.txt", b"needle original\n");
    let mut s = session(d.path()); s.dispatch(Command::OpenRoot(root)); settle(&mut s, scanned);
    open(&mut s, Command::OpenRetained(file.clone()), "a.txt");
    s.dispatch(Command::Search { query: "needle".into(), scope: SearchIn::Project }); settle(&mut s, searched);
    let tab = s.state().active().unwrap().id(); let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Select(Selection { anchor: 5, head: 5 }));
    s.dispatch(Command::OpenResult(0));
    let completion = loop {
        let completion = s.completions().recv_blocking_timeout().unwrap();
        if matches!(&completion, Completion::Analyzed { result, .. } if matches!(result.as_ref(), Completion::Validated { .. })) { break completion; }
        s.dispatch(completion);
    };
    s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..0, text: "x".into() }], kind: EditKind::Typing });
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "needle original\n");
    let current = s.state().current_source().unwrap().key(); assert!(current.version > expected.version);
    s.dispatch(completion);
    assert!(s.state().results().unwrap().is_stale(0));
    assert_eq!(s.state().selection(), Some(Selection { anchor: 5, head: 5 }));
    assert_eq!(s.state().current_source().unwrap().key(), current);
    assert!(s.state().notices().iter().any(|message| message.starts_with("This result")));
    assert_eq!(std::fs::read(file).unwrap(), b"needle original\n");
}

#[test]
fn no_hit_draft_capture_releases_text_but_reports_later_draft_set_changes() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap(); let root = d.path().join("project"); std::fs::create_dir(&root).unwrap();
    let file = write(&root, "a.txt", b"saved\n");
    let mut s = session(d.path()); s.dispatch(Command::OpenRoot(root)); settle(&mut s, scanned);
    open(&mut s, Command::OpenRetained(file.clone()), "a.txt"); let tab = s.state().active().unwrap().id();
    let replace = |s: &mut Session, text: &str| {
        let source = s.state().current_source().unwrap(); let (expected, len) = (source.key(), source.len_bytes());
        s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..len, text: text.into() }], kind: EditKind::Replace });
    };
    replace(&mut s, "absent draft\n"); let captured = Arc::downgrade(s.state().current_source().unwrap());
    s.dispatch(Command::Search { query: "needle".into(), scope: SearchIn::Project }); settle(&mut s, searched);
    assert!(s.state().results().unwrap().hits().is_empty());
    replace(&mut s, "needle newer\n");
    settle(&mut s, |_| captured.upgrade().is_none());
    assert!(s.state().results().unwrap().error().is_some_and(|error| error.contains("drafts changed")));
    s.dispatch(Command::RerunSearch); settle(&mut s, searched);
    assert_eq!(s.state().results().unwrap().hits().len(), 1);
    assert!(s.state().results().unwrap().error().is_none());
    assert_eq!(std::fs::read(file).unwrap(), b"saved\n");
}

#[test]
fn closing_or_replacing_a_project_query_releases_queued_captured_versions() {
    use crate::text::{EditKind, TextEdit};
    for close in [true, false] {
        let d = tempfile::tempdir().unwrap(); let root = d.path().join("project"); std::fs::create_dir(&root).unwrap();
        let file = write(&root, "a.txt", b"saved\n");
        let mut s = session(d.path()); s.dispatch(Command::OpenRoot(root)); settle(&mut s, scanned);
        open(&mut s, Command::OpenRetained(file.clone()), "a.txt"); let tab = s.state().active().unwrap().id();
        let replace = |s: &mut Session, text: &str| {
            let source = s.state().current_source().unwrap(); let (expected, len) = (source.key(), source.len_bytes());
            s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..len, text: text.into() }], kind: EditKind::Replace });
        };
        replace(&mut s, "needle captured\n"); let captured = Arc::downgrade(s.state().current_source().unwrap());
        s.dispatch(Command::Search { query: "needle".into(), scope: SearchIn::Project });
        let listed = loop {
            let completion = s.completions().recv_blocking_timeout().unwrap();
            if matches!(&completion, Completion::Analyzed { result, .. } if matches!(result.as_ref(), Completion::ProjectListed { .. })) { break completion; }
            s.dispatch(completion);
        };
        let (started, starts) = std::sync::mpsc::channel();
        let releases: Vec<_> = (0..crate::jobs::MAX_JOBS).map(|_| {
            let (release, wait) = std::sync::mpsc::channel(); let started = started.clone();
            s.jobs.submit(Priority::Visible, move || { started.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); }); release
        }).collect();
        for _ in &releases { starts.recv_timeout(Duration::from_secs(5)).unwrap(); }
        s.dispatch(listed);
        replace(&mut s, "newer current\n");
        assert!(captured.upgrade().is_some(), "the pending query owns its captured version");
        if close { s.dispatch(Command::CloseResults); }
        else { s.dispatch(Command::Search { query: "newer".into(), scope: SearchIn::Project }); }
        for release in releases { release.send(()).unwrap(); }
        settle(&mut s, |s| captured.upgrade().is_none() && (close || searched(s)));
        if close { assert!(s.state().results().is_none()); }
        else { assert_eq!(s.state().results().unwrap().hits()[0].text.as_ref(), "newer current"); }
        assert_eq!(source_text(s.state().current_source().unwrap()), "newer current\n");
        assert_eq!(std::fs::read(file).unwrap(), b"saved\n");
    }
}

#[test]
fn project_text_overlay_preserves_folder_runs_filters_and_the_global_cap() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap(); let root = d.path().join("project"); std::fs::create_dir_all(root.join("deep")).unwrap();
    write(&root, "a.txt", b"needle disk_a\n");
    let b = write(&root, "b.txt", b"saved b\n");
    write(&root, "deep/c.txt", b"needle disk_c\n");
    let fourth = write(&root, "deep/d.txt", b"saved d\n");
    let mut s = session(d.path()); s.dispatch(Command::OpenRoot(root)); settle(&mut s, scanned);
    for (path, name, text) in [(b.clone(), "b.txt", "needle draft_b\n".repeat(4000)), (fourth.clone(), "d.txt", "needle draft_d\n".repeat(6000))] {
        open(&mut s, Command::OpenRetained(path), name);
        let source = s.state().current_source().unwrap(); let (expected, len) = (source.key(), source.len_bytes()); let tab = s.state().active().unwrap().id();
        s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..len, text: text.into() }], kind: EditKind::Replace });
    }
    s.dispatch(Command::Search { query: "needle".into(), scope: SearchIn::Matching("*.txt".into()) }); settle(&mut s, searched);
    let r = s.state().results().unwrap();
    assert_eq!(r.hits().len(), 10_000); assert!(r.limited()); assert_eq!(r.files(), 4);
    assert_eq!([0, 1, 4000, 4001, 4002, 9999].map(|i| r.hits()[i].path.as_ref()), ["a.txt", "b.txt", "b.txt", "deep/c.txt", "deep/d.txt", "deep/d.txt"]);
    let folders: Vec<_> = r.rows().iter().filter_map(|row| match row { search::Row::Dir { dir, .. } => Some(dir.as_ref()), _ => None }).collect();
    assert_eq!(folders, ["", "deep"], "each folder/file must remain one contiguous run");
    for i in [0, 1, 4001, 9999] { assert!(r.row_of_hit(i).is_some()); }
    s.dispatch(Command::Search { query: "needle".into(), scope: SearchIn::Matching("deep/".into()) }); settle(&mut s, searched);
    let r = s.state().results().unwrap(); assert_eq!(r.hits().len(), 6001); assert_eq!(r.files(), 2); assert!(!r.limited());
    assert!(r.hits().iter().all(|hit| hit.path.starts_with("deep/")));
    assert_eq!(std::fs::read(b).unwrap(), b"saved b\n"); assert_eq!(std::fs::read(fourth).unwrap(), b"saved d\n");
}

#[test]
fn declaration_overlay_keeps_the_global_best_200_and_total_match_count() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap(); let root = d.path().join("project"); std::fs::create_dir(&root).unwrap();
    let disk: String = (0..250).map(|i| format!("fn Needle{i:03}() {{}}\n")).collect();
    let a = write(&root, "a.rs", disk.as_bytes()); let b = write(&root, "b.rs", b"fn RemovedSaved() {}\n");
    let mut s = session(d.path()); s.dispatch(Command::OpenRoot(root)); settle(&mut s, |s| scanned(s) && s.state().index_coverage().complete());
    open(&mut s, Command::OpenRetained(b.clone()), "b.rs");
    let source = s.state().current_source().unwrap(); let (expected, len) = (source.key(), source.len_bytes()); let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..len, text: "fn Needle() {}\n".into() }], kind: EditKind::Replace });
    s.dispatch(Command::FindDeclarations("Needle".into())); settle(&mut s, searched);
    let r = s.state().results().unwrap(); assert_eq!(r.hits().len(), 200); assert!(r.limited());
    assert_eq!((r.hits()[0].path.as_ref(), r.hits()[0].text.as_ref()), ("b.rs", "Needle"), "a later draft's stronger match outranks the disk prefix matches");
    assert_eq!(r.hits().iter().filter(|hit| hit.path.as_ref() == "b.rs").count(), 1);
    assert_eq!(std::fs::read(a).unwrap(), disk.as_bytes()); assert_eq!(std::fs::read(b).unwrap(), b"fn RemovedSaved() {}\n");
}

#[test]
fn declaration_disk_identity_uses_extracted_decoded_text_with_or_without_cache() {
    let before = "// before\nfn café_target() {}\r\n";
    let after = "// change\nfn café_target() {}\r\n";
    for case in 0..4 {
        let encode = |text: &str| -> Vec<u8> { match case {
            0 => text.as_bytes().to_vec(),
            1 => [b"\xef\xbb\xbf".as_slice(), text.as_bytes()].concat(),
            2 => std::iter::once(0xfeff).chain(text.encode_utf16()).flat_map(u16::to_le_bytes).collect(),
            _ => text.chars().map(|ch| u8::try_from(ch as u32).unwrap()).collect(),
        }};
        let d = tempfile::tempdir().unwrap(); let root = d.path().join("project"); std::fs::create_dir(&root).unwrap();
        let a = write(&root, "a.rs", &encode(before)); let other = write(&root, "other.txt", b"stay here\n");
        let mut s = session(d.path()); s.dispatch(Command::SetPersistentIndex(case % 2 != 0));
        s.dispatch(Command::OpenRoot(root)); settle(&mut s, |s| scanned(s) && s.state().index_coverage().complete());
        open(&mut s, Command::OpenRetained(other.clone()), "other.txt");
        s.dispatch(Command::FindDeclarations("café_target".into()));
        let completion = loop {
            let completion = s.completions().recv_blocking_timeout().unwrap();
            if matches!(&completion, Completion::Analyzed { result, .. } if matches!(result.as_ref(), Completion::ProjectRanked { .. })) { break completion; }
            s.dispatch(completion);
        };
        let stamp = Stamp::of(&a).unwrap();
        std::fs::write(&a, encode(after)).unwrap();
        std::fs::File::options().write(true).open(&a).unwrap().set_modified(stamp.modified.unwrap()).unwrap();
        assert_eq!(Stamp::of(&a), Some(stamp), "fixture bypasses stamp-only detection, case {case}");
        s.dispatch(completion);
        settle(&mut s, |s| s.state().results().is_some_and(|r| r.is_stale(0) || r.preview().is_some()));
        let results = s.state().results().unwrap();
        assert_eq!((results.hits()[0].line, results.hits()[0].range.clone(), results.hits()[0].text.as_ref()), (1, 3..15, "café_target"));
        assert!(results.is_stale(0), "changed decoded source must not bless old positions, case {case}");
        assert!(results.preview().is_none());
        s.dispatch(Command::OpenResult(0));
        settle(&mut s, |s| s.state().notices().iter().any(|message| message.starts_with("This result")));
        assert_eq!(s.state().current_source().unwrap().source(), other.as_path());
        assert_eq!(std::fs::read(&a).unwrap(), encode(after));
    }
}

#[test]
fn project_results_preview_the_captured_draft_and_refuse_changed_target_activation() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project"); std::fs::create_dir(&root).unwrap();
    let a = write(&root, "a.txt", b"saved a\n");
    write(&root, "b.txt", b"needle disk\n");
    let mut s = session(d.path()); s.dispatch(Command::OpenRoot(root)); settle(&mut s, scanned);
    open(&mut s, Command::OpenRetained(a.clone()), "a.txt");
    let tab = s.state().active().unwrap().id();
    let replace = |s: &mut Session, text: &str| {
        let source = s.state().current_source().unwrap(); let (expected, len) = (source.key(), source.len_bytes());
        s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..len, text: text.into() }], kind: EditKind::Replace });
    };
    replace(&mut s, "\nneedle captured\n");
    s.dispatch(Command::Search { query: "needle".into(), scope: SearchIn::Project }); settle(&mut s, searched);
    let results = s.state().results().unwrap();
    let first = results.hits().iter().position(|hit| &*hit.path == "a.txt").unwrap();
    let other = results.hits().iter().position(|hit| &*hit.path == "b.txt").unwrap();
    replace(&mut s, "\n\nneedle newer\n");
    s.dispatch(Command::SelectResult(other)); settle(&mut s, |s| s.state().results().unwrap().preview().is_some_and(|p| p.index == other));
    s.dispatch(Command::SelectResult(first)); settle(&mut s, |s| s.state().results().unwrap().preview().is_some_and(|p| p.index == first));
    assert_eq!(s.state().results().unwrap().preview().unwrap().doc.text().as_ref(), "\nneedle captured\n");
    s.dispatch(Command::Select(Selection { anchor: 0, head: 0 }));
    let key = s.state().current_source().unwrap().key();
    s.dispatch(Command::OpenResult(first));
    settle(&mut s, |s| s.state().results().unwrap().is_stale(first) || s.state().selection().is_some_and(|p| p.head != 0));
    assert!(s.state().results().unwrap().is_stale(first));
    assert_eq!(s.state().selection(), Some(Selection { anchor: 0, head: 0 }));
    assert_eq!(s.state().current_source().unwrap().key(), key);
    assert_eq!(std::fs::read(a).unwrap(), b"saved a\n");
}

#[test]
fn project_declarations_overlay_two_shifted_drafts_without_persisting_them() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project"); std::fs::create_dir_all(root.join("nested")).unwrap();
    let a = write(&root, "a.rs", b"fn RemovedSavedA() {}\n");
    let b = write(&root, "nested/b.rs", b"fn RemovedSavedB() {}\n");
    let disk = write(&root, "nested/c.rs", b"fn UnopenedDraftOnly() {}\n");
    let mut s = session(d.path()); s.dispatch(Command::OpenRoot(root));
    settle(&mut s, |s| scanned(s) && s.state().index_coverage().complete());
    for (path, name, text) in [(a.clone(), "a.rs", "\nfn DraftOnlyA() {}\n"), (b.clone(), "b.rs", "\n\nfn DraftOnlyB() {}\n")] {
        open(&mut s, Command::OpenRetained(path), name);
        let source = s.state().current_source().unwrap(); let (expected, len) = (source.key(), source.len_bytes());
        let tab = s.state().active().unwrap().id();
        s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..len, text: text.into() }], kind: EditKind::Replace });
    }
    s.dispatch(Command::FindDeclarations("DraftOnly".into())); settle(&mut s, searched);
    let mut hits: Vec<_> = s.state().results().unwrap().hits().iter().map(|hit| (hit.path.to_string(), hit.line, hit.text.to_string())).collect(); hits.sort();
    assert_eq!(hits, [("a.rs".into(), 1, "DraftOnlyA".into()), ("nested/b.rs".into(), 2, "DraftOnlyB".into()), ("nested/c.rs".into(), 0, "UnopenedDraftOnly".into())]);
    s.dispatch(Command::FindDeclarations("RemovedSaved".into())); settle(&mut s, searched);
    assert!(s.state().results().unwrap().hits().is_empty());
    assert_eq!(std::fs::read(a).unwrap(), b"fn RemovedSavedA() {}\n");
    assert_eq!(std::fs::read(b).unwrap(), b"fn RemovedSavedB() {}\n");
    assert_eq!(std::fs::read(disk).unwrap(), b"fn UnopenedDraftOnly() {}\n");
}

#[test]
fn project_text_search_overlays_current_drafts_and_leaves_unopened_files_on_disk() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project"); std::fs::create_dir(&root).unwrap();
    let a = write(&root, "open.rs", b"fn removed_saved() {}\n");
    let b = write(&root, "unopened.rs", b"fn unopened_draft_only() {}\n");
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root));
    settle(&mut s, scanned);
    open(&mut s, Command::OpenRetained(a.clone()), "open.rs");
    let source = s.state().current_source().unwrap();
    let (expected, len) = (source.key(), source.len_bytes());
    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..len, text: "\n\nfn draft_only() {}\n".into() }], kind: EditKind::Replace });
    s.dispatch(Command::Search { query: "draft_only".into(), scope: SearchIn::Project });
    settle(&mut s, searched);
    let mut hits: Vec<_> = s.state().results().unwrap().hits().iter().map(|hit| (hit.path.to_string(), hit.line)).collect(); hits.sort();
    assert_eq!(hits, [("open.rs".into(), 2), ("unopened.rs".into(), 0)]);
    s.dispatch(Command::Search { query: "removed_saved".into(), scope: SearchIn::Project });
    settle(&mut s, searched);
    assert!(s.state().results().unwrap().hits().is_empty(), "saved matches removed from an open draft must disappear");
    assert_eq!(std::fs::read(a).unwrap(), b"fn removed_saved() {}\n");
    assert_eq!(std::fs::read(b).unwrap(), b"fn unopened_draft_only() {}\n");
}

#[test]
fn rejected_and_noop_edits_do_not_pin_reloadable_sources() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "unchanged.txt", "α-original".as_bytes());
    let b = write(d.path(), "other.txt", b"other");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(a), "unchanged.txt");
    s.cache = InactiveCache::new(0);
    let tab = s.state().active().unwrap().id();
    let source = s.state().current_source().unwrap();
    let (weak, expected) = (Arc::downgrade(source), source.key());
    s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 1..2, text: "x".into() }], kind: EditKind::Replace });
    s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..2, text: "α".into() }], kind: EditKind::Replace });
    assert_eq!(s.state().current_source().unwrap().key(), expected);
    assert!(!active_view(&s).can_undo());
    assert!(!active_view(&s).dirty());
    open(&mut s, Command::OpenRetained(b), "other.txt");
    assert!(weak.upgrade().is_none(), "no-op/rejection must not install a hidden confirmed-baseline owner");
}

#[test]
fn draft_history_retains_changes_without_pinning_every_intermediate_snapshot() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap();
    let original = "a".repeat(256 * 1024);
    let path = write(d.path(), "large-draft.txt", original.as_bytes());
    let mut s = session(d.path());
    open(&mut s, Command::Open(path.clone()), "large-draft.txt");
    let tab = s.state().active().unwrap().id();
    let baseline = Arc::downgrade(s.state().current_source().unwrap());
    let edit = |s: &mut Session, range, text: &str| {
        let expected = s.state().current_source().unwrap().key();
        s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range, text: text.into() }], kind: EditKind::Replace });
    };
    edit(&mut s, 0..original.len() - 16384, "");
    let partial = Arc::downgrade(s.state().current_source().unwrap());
    assert_eq!(s.state().current_source().unwrap().len_bytes(), 16384);
    assert!(s.state().current_source().unwrap().storage_bytes() >= original.len(), "a surviving original chunk accounts its whole backing");
    edit(&mut s, 0..16384, "tiny");
    assert!(partial.upgrade().is_none(), "history stores changes, not each intermediate snapshot");
    assert!(baseline.upgrade().is_some(), "the confirmed baseline is a real retained owner");
    assert!(s.state().current_source().unwrap().storage_bytes() < 64 * 1024);
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "a".repeat(16384));
    let undone = Arc::downgrade(s.state().current_source().unwrap());
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), original);
    assert!(!active_view(&s).dirty());
    assert!(undone.upgrade().is_none());
    edit(&mut s, 0..original.len(), "new branch");
    assert!(!active_view(&s).can_redo());
    assert_eq!(source_text(s.state().current_source().unwrap()), "new branch");
    assert_eq!(std::fs::read_to_string(path).unwrap(), original);
}

#[test]
fn draft_commands_preserve_image_read_only_admission_and_shared_availability() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "readonly.svg", br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"></svg>"#);
    let mut s = session(d.path());
    assert_eq!(s.disabled(CommandId::Undo), Some("No file is open"));
    open(&mut s, Command::Open(path.clone()), "readonly.svg");
    let before = s.state().current_source().unwrap().clone();
    let tab = s.state().active().unwrap().id();
    assert_eq!(active_view(&s).edit_disabled(), Some("Images are read-only"));
    s.dispatch(Command::Edit { tab, expected: before.key(), edits: vec![TextEdit { range: 0..0, text: "changed".into() }], kind: EditKind::Typing });
    assert!(Arc::ptr_eq(s.state().current_source().unwrap(), &before));
    assert!(!active_view(&s).can_undo());
    assert!(!active_view(&s).dirty());
    assert_eq!(s.disabled(CommandId::Undo), Some("Images are read-only"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), source_text(&before));
}

#[cfg(unix)]
#[test]
fn draft_semantic_readiness_survives_unloading_its_last_view() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "Cargo.toml", b"[package]\nname = \"unloaded_draft\"\n");
    let a = write(d.path(), "draft.rs", b"fn saved() {}\n");
    let b = write(d.path(), "other.txt", b"other stays current\n");
    let (mut s, _) = fake_semantic_session(d.path());
    open(&mut s, Command::OpenRetained(a.clone()), "draft.rs");
    settle(&mut s, |s| s.state().provider().is_some_and(|p| p.status == semantics::Status::Ready));
    let tab = s.state().active().unwrap().id();
    let (started, starts) = std::sync::mpsc::channel();
    let releases: Vec<_> = (0..crate::jobs::MAX_JOBS).map(|_| {
        let (release, wait) = std::sync::mpsc::channel(); let started = started.clone();
        s.jobs.submit(Priority::Visible, move || { started.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); }); release
    }).collect();
    for _ in &releases { starts.recv_timeout(Duration::from_secs(5)).unwrap(); }
    let source = s.state().current_source().unwrap();
    let (expected, len) = (source.key(), source.len_bytes());
    let text = "fn unsaved_unicode_é() {}\n";
    s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..len, text: text.into() }], kind: EditKind::Replace });
    let key = s.state().current_source().unwrap().key();
    s.dispatch(Command::OpenRetained(b.clone()));
    let b_tab = s.state().active().unwrap().id();
    let Ok((stamp, bytes)) = read(&b) else { panic!("stable disposable B") };
    let version = s.versions.fetch_add(1, Ordering::Relaxed) + 1;
    s.dispatch(Completion::Loaded { request: s.request, tab: b_tab, path: b.clone(), result: Ok((Some(stamp),
        Arc::new(Document::decode(b.clone(), version, &bytes)), std::fs::canonicalize(&b).ok())) });
    assert!(s.state().view(tab).is_none());
    assert!(s.state().tab_dirty(tab));
    for release in releases { release.send(()).unwrap(); }
    settle(&mut s, |s| s.state.semantics.desired().get(&a).is_some_and(|doc| doc.version() == key.version && doc.text().as_ref() == text));
    assert_eq!(s.state().current_source().unwrap().source(), b.as_path());
    assert_eq!(std::fs::read(a).unwrap(), b"fn saved() {}\n");
}

#[test]
fn dirty_and_clean_history_survive_real_cache_eviction_and_a_split_closing() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "history.txt", b"original\r\n");
    let mut s = session(d.path());
    open(&mut s, Command::Open(path.clone()), "history.txt");
    let tab = s.state().active().unwrap().id();
    s.cache = InactiveCache::new(0);
    let edit = |s: &mut Session, text: &str| {
        let expected = s.state().current_source().unwrap().key();
        s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..8, text: text.into() }], kind: EditKind::Replace });
    };
    edit(&mut s, "modified");
    let draft = Arc::downgrade(s.state().current_source().unwrap());
    let other = write(d.path(), "other.txt", b"reloadable");
    open(&mut s, Command::OpenRetained(other.clone()), "other.txt");
    let clean = Arc::downgrade(s.state().current_source().unwrap());
    assert!(s.state().view(tab).is_none(), "inactive view is released, draft is buffer-owned");
    assert!(draft.upgrade().is_some());
    assert!(s.cache.is_empty(), "dirty buffers never consume reloadable cache budget");
    open(&mut s, Command::OpenRetained(path.clone()), "history.txt");
    assert_eq!(source_text(s.state().current_source().unwrap()), "modified\r\n");
    assert!(active_view(&s).dirty());
    assert!(clean.upgrade().is_none(), "actual unrelated clean source was evicted");
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "original\r\n");
    assert!(!active_view(&s).dirty());
    let with_history = Arc::downgrade(s.state().current_source().unwrap());
    open(&mut s, Command::OpenRetained(other), "other.txt");
    assert!(with_history.upgrade().is_some(), "clean text with redo history is independently pinned");
    open(&mut s, Command::OpenRetained(path.clone()), "history.txt");
    s.dispatch(Command::Redo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "modified\r\n");
    s.dispatch(Command::Split(Split::Columns));
    s.dispatch(Command::Unsplit);
    let survivor = s.state().active().unwrap().id();
    s.dispatch(Command::Undo { tab: survivor });
    assert_eq!(source_text(s.state().current_source().unwrap()), "original\r\n");
    assert!(!active_view(&s).dirty());
    assert_eq!(std::fs::read(path).unwrap(), b"original\r\n");
}

#[test]
fn draft_history_groups_adjacent_same_view_edits_for_500ms_and_breaks_on_actions() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "groups.txt", b"A\n");
    let clock = Arc::new(std::sync::Mutex::new(std::time::Instant::now()));
    let observed = clock.clone();
    let mut s = session_clock(d.path(), Arc::new(move || *observed.lock().unwrap()));
    open(&mut s, Command::Open(path.clone()), "groups.txt");
    let tab = s.state().active().unwrap().id();
    let edit = |s: &mut Session, range, text: &str, kind| {
        let expected = s.state().current_source().unwrap().key();
        s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range, text: text.into() }], kind });
    };
    edit(&mut s, 0..0, "α", EditKind::Typing);
    *clock.lock().unwrap() += Duration::from_millis(500);
    edit(&mut s, 2..2, "😀", EditKind::Typing);
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "A\n", "one meaningful typing group");
    s.dispatch(Command::Redo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "α😀A\n");
    edit(&mut s, 6..6, "x", EditKind::Typing);
    *clock.lock().unwrap() += Duration::from_millis(501);
    edit(&mut s, 7..7, "y", EditKind::Typing);
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "α😀xA\n");
    s.dispatch(Command::Select(Selection { anchor: 7, head: 7 }));
    edit(&mut s, 7..7, "z", EditKind::Typing);
    s.dispatch(Command::Select(Selection { anchor: 8, head: 8 }));
    edit(&mut s, 8..8, "w", EditKind::Typing);
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "α😀xzA\n");
    edit(&mut s, 8..8, "PASTE", EditKind::Paste);
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "α😀xzA\n");
    edit(&mut s, 6..8, "", EditKind::Delete);
    edit(&mut s, 2..6, "", EditKind::Delete);
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "α😀xzA\n", "adjacent backspaces group without losing Unicode");
    edit(&mut s, 2..6, "", EditKind::Delete);
    edit(&mut s, 2..3, "", EditKind::Delete);
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "α😀xzA\n", "forward deletes form their own group");
    assert_eq!(std::fs::read(path).unwrap(), b"A\n");
}

#[test]
fn draft_transactions_reject_stale_invalid_and_overlapping_ranges_atomically() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "atomic.txt", "α😀\r\nbeta\n".as_bytes());
    let mut s = session(d.path());
    open(&mut s, Command::Open(path.clone()), "atomic.txt");
    let tab = s.state().active().unwrap().id();
    let baseline = s.state().current_source().unwrap().clone();
    let edit = |range, text: &str| TextEdit { range, text: text.into() };
    for edits in [vec![edit(1..2, "x")], vec![edit(0..2, "x"), edit(1..6, "y")], vec![edit(0..0, "x"), edit(0..0, "y")], vec![edit(0..100, "x")]] {
        s.dispatch(Command::Edit { tab, expected: baseline.key(), edits, kind: EditKind::Replace });
        assert!(Arc::ptr_eq(s.state().current_source().unwrap(), &baseline));
        assert!(!active_view(&s).can_undo());
        assert!(!active_view(&s).dirty());
        assert!(!s.state().active().unwrap().retained());
    }
    s.dispatch(Command::Edit { tab, expected: baseline.key(), edits: vec![edit(0..2, "x")], kind: EditKind::Replace });
    let current = s.state().current_source().unwrap().clone();
    s.dispatch(Command::Edit { tab, expected: baseline.key(), edits: vec![edit(0..1, "z")], kind: EditKind::Replace });
    assert!(Arc::ptr_eq(s.state().current_source().unwrap(), &current));
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "α😀\r\nbeta\n");
    assert!(!active_view(&s).can_undo());
    assert!(active_view(&s).can_redo());
    assert!(!active_view(&s).dirty());
    assert_eq!(s.disabled(CommandId::Undo), Some("Nothing to undo"));
    assert_eq!(s.disabled(CommandId::Redo), None);
    let fresh = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected: fresh, edits: vec![edit(0..2, ""), edit(2..6, "")], kind: EditKind::Replace });
    assert_eq!(source_text(s.state().current_source().unwrap()), "\r\nbeta\n");
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "α😀\r\nbeta\n");
    let fresh = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected: fresh, edits: vec![edit(0..2, "x")], kind: EditKind::Replace });
    let fresh = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected: fresh, edits: vec![edit(0..1, "α")], kind: EditKind::Replace });
    assert!(!active_view(&s).dirty(), "content equality, even with new undo entries, defines clean");
    assert!(active_view(&s).can_undo());
    assert_eq!(s.disabled(CommandId::Undo), None);
    assert_eq!(std::fs::read(&path).unwrap(), "α😀\r\nbeta\n".as_bytes());
}

#[test]
fn shared_draft_commands_replace_unicode_and_undo_without_writing_disk() {
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "draft.txt", "α😀\r\nbeta\n".as_bytes());
    let mut s = session(d.path());
    open(&mut s, Command::Open(path.clone()), "draft.txt");
    let first = s.state().active().unwrap().id();
    let original = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab: first, expected: original,
        edits: vec![TextEdit { range: 0..2, text: "é".into() }], kind: EditKind::Replace });
    assert_eq!(source_text(s.state().current_source().unwrap()), "é😀\r\nbeta\n");
    assert!(s.state().active().unwrap().retained());
    assert!(active_view(&s).dirty());
    assert!(s.state().current_source().unwrap().key().version > original.version);
    s.dispatch(Command::Split(Split::Columns));
    let second = s.state().active().unwrap().id();
    assert_ne!(first, second);
    s.dispatch(Command::Select(Selection { anchor: 8, head: 8 }));
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab: second, expected, edits: vec![TextEdit { range: 8..12, text: "BETA".into() }], kind: EditKind::Replace });
    let second_position = s.state().active().unwrap().selection();
    s.dispatch(Command::FocusPane(0));
    s.dispatch(Command::Select(Selection { anchor: 2, head: 2 }));
    s.dispatch(Command::Undo { tab: first });
    assert_eq!(source_text(s.state().current_source().unwrap()), "é😀\r\nbeta\n");
    assert!(active_view(&s).dirty());
    s.dispatch(Command::Undo { tab: first });
    assert_eq!(source_text(s.state().current_source().unwrap()), "α😀\r\nbeta\n");
    assert!(!active_view(&s).dirty());
    assert_eq!(s.state().shown(1).unwrap().selection(), second_position);
    let undone = s.state().current_source().unwrap().key();
    s.dispatch(Command::Redo { tab: second });
    assert_eq!(source_text(s.state().current_source().unwrap()), "é😀\r\nbeta\n");
    s.dispatch(Command::Redo { tab: second });
    assert_eq!(source_text(s.state().current_source().unwrap()), "é😀\r\nBETA\n");
    assert!(s.state().current_source().unwrap().key().version > undone.version);
    assert_eq!(s.state().shown(0).unwrap().selection(), Some(Selection { anchor: 2, head: 2 }));
    assert_eq!(source_text(pane_view(&s, 1).snapshot()), "é😀\r\nBETA\n");
    let mut independent = session(&d.path().join("other-session"));
    open(&mut independent, Command::Open(path.clone()), "draft.txt");
    assert_eq!(source_text(independent.state().current_source().unwrap()), "α😀\r\nbeta\n");
    assert!(!active_view(&independent).dirty());
    assert_eq!(std::fs::read(&path).unwrap(), "α😀\r\nbeta\n".as_bytes());
}

#[test]
fn rebuild_discards_saved_and_inflight_records_and_cache_optout_survives_reopen() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project"); std::fs::create_dir(&root).unwrap();
    let path = write(&root, "a.rb", b"class Alpha; end\n");
    let mut s = session(d.path());
    assert!(s.disabled(CommandId::RebuildIndex).is_some());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, |s| s.state.index.coverage().indexed == 1 && s.state.index.coverage().complete());
    open(&mut s, Command::Open(path.clone()), "a.rb");
    s.watcher = Watcher::new(|_| true);
    let saved = d.path().join("cache/declarations-v1").join(crate::index_cache::digest(root.as_os_str().as_encoded_bytes())).join("records");
    settle(&mut s, |_| saved.exists());
    let generation = s.project_generation.load(Ordering::Relaxed);
    let revision = s.index_revision.load(Ordering::Relaxed);
    let old = index::index_file(&root, "a.rb", || true);
    s.dispatch(Command::FindDeclarations("Omega".into()));
    std::fs::write(&path, "class Omega; end\n").unwrap();
    s.dispatch(Command::RebuildIndex);
    assert!(s.disabled(CommandId::RebuildIndex).is_some());
    s.dispatch(Completion::Indexed { generation, revision, files: vec![(Arc::from("a.rb"), old)] });
    assert_eq!(s.state.index.coverage().indexed, 0, "held pre-rebuild chunk is obsolete");
    settle(&mut s, |s| !s.index_rebuilding && s.state.index.coverage().indexed == 1 && s.state.index.coverage().complete()
        && s.state.results.as_ref().is_some_and(|r| r.done)
        && s.state.current_source().is_some_and(|d| source_text(d).contains("Omega")));
    let captured = s.state.results.as_ref().unwrap();
    assert!(captured.hits.is_empty(), "the captured Alpha query does not silently switch to refreshed text");
    assert!(captured.error().is_some_and(|error| error.contains("rerun")), "reloaded query sources must visibly become stale");
    s.dispatch(Command::RerunSearch);
    settle(&mut s, searched);
    assert_eq!(s.state.results.as_ref().unwrap().hits.len(), 1);
    assert_eq!(index::rank(&s.state.index.snapshot(), "Alpha", || true).unwrap().1, 0);
    assert_eq!(index::rank(&s.state.index.snapshot(), "Omega", || true).unwrap().1, 1);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "class Omega; end\n");
    s.dispatch(Command::SetPersistentIndex(false));
    assert!(s.index_cache.is_none());
    settle(&mut s, |s| s.state.index.coverage().complete()
        && !read_settings(&d.path().join("state/lenz-reader/settings.json")).persistent_index);
    drop(s);
    let mut reopened = session(d.path());
    assert_eq!(reopened.on(CommandId::PersistentIndex), Some(false));
    reopened.dispatch(Command::OpenRoot(root));
    settle(&mut reopened, |s| s.state.index.coverage().indexed == 1 && s.state.index.coverage().complete());
    assert!(reopened.index_cache.is_none());
}

#[test]
fn a_rebuild_completion_cannot_reset_a_different_project() {
    let d = tempfile::tempdir().unwrap();
    let one = d.path().join("one"); let two = d.path().join("two");
    std::fs::create_dir(&one).unwrap(); std::fs::create_dir(&two).unwrap();
    write(&one, "a.rb", b"class Alpha; end\n"); write(&two, "b.rb", b"class Beta; end\n");
    let mut s = session(d.path()); s.dispatch(Command::OpenRoot(one));
    settle(&mut s, |s| s.state.index.coverage().indexed == 1);
    s.dispatch(Command::RebuildIndex);
    let rx = s.completions();
    let completion = loop {
        let c = rx.recv_blocking_timeout().unwrap();
        if matches!(c, Completion::IndexReset { .. }) { break c; }
        s.dispatch(c);
    };
    s.dispatch(Command::OpenRoot(two.clone()));
    s.dispatch(completion);
    settle(&mut s, |s| s.state.index.coverage().indexed == 1 && s.state.index.coverage().complete());
    assert_eq!(s.state.project().unwrap().root(), two);
    assert!(!s.index_rebuilding);
    assert_eq!(index::rank(&s.state.index.snapshot(), "Beta", || true).unwrap().1, 1);
}

#[test]
fn saved_declarations_match_fresh_extraction_across_languages_and_content_changes() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project");
    std::fs::create_dir(&root).unwrap();
    let fixtures = [
        ("a.rs", "// café 𝄞\r\npub struct Alpha;\r\nimpl Alpha { pub fn call(&self) {} }\r\n"),
        ("a.py", "# café 𝄞\r\nclass Alpha:\r\n    def call(self):\r\n        return 1\r\n"),
        ("a.java", "// café 𝄞\r\nclass Alpha { int call() { return 1; } }\r\n"),
        ("a.kt", "// café 𝄞\r\nclass Alpha { fun call(): Int { return 1 } }\r\n"),
        ("a.js", "// café 𝄞\r\nexport class Alpha { call() { return 1; } }\r\n"),
        ("a.ts", "// café 𝄞\r\nexport interface Alpha { call(): number; }\r\n"),
        ("a.tsx", "// café 𝄞\r\nexport function Alpha() { return <div>hello</div>; }\r\n"),
        ("a.html", "<!-- café 𝄞 -->\r\n<div id=\"Alpha\"><span>hello</span></div>\r\n"),
        ("a.css", "/* café 𝄞 */\r\n.Alpha { color: red; }\r\n"),
    ];
    let mut modified = Vec::new();
    for (name, text) in fixtures {
        let path = write(&root, name, text.as_bytes());
        modified.push(std::fs::metadata(path).unwrap().modified().unwrap());
    }
    let snapshot = |s: &Session| {
        let mut got: Vec<_> = s.state.index.snapshot().into_iter().map(|(p, items)| (p.to_string(), format!("{items:?}"))).collect();
        got.sort();
        got
    };
    let check = |s: &Session| {
        let got = snapshot(s);
        let mut errors = 0;
        let expected: Vec<_> = got.iter().map(|(path, _)| {
            let index::FileIndex::Done { items, syntax_errors, .. } = index::index_file(&root, path, || true) else { panic!("{path}") };
            errors += usize::from(syntax_errors);
            assert!(!items.is_empty(), "{path}");
            (path.clone(), format!("{items:?}"))
        }).collect();
        assert_eq!(got, expected, "names, kinds, ranges, Unicode offsets and parents must survive persistence");
        assert_eq!(s.state.index.coverage().syntax_errors, errors);
    };
    let complete = |s: &Session| s.state.index.coverage().indexed == fixtures.len() && s.state.index.coverage().complete();
    let saved = d.path().join("cache/declarations-v1").join(crate::index_cache::digest(root.as_os_str().as_encoded_bytes())).join("records");
    let mut cold = session(d.path());
    cold.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut cold, |s| complete(s) && saved.exists());
    check(&cold);
    let original = snapshot(&cold);
    drop(cold);
    let mut warm = session(d.path());
    warm.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut warm, complete);
    check(&warm);
    assert_eq!(snapshot(&warm), original);
    warm.dispatch(Command::SetPersistentIndex(false));
    settle(&mut warm, complete);
    assert!(warm.index_cache.is_none());
    assert_eq!(snapshot(&warm), original);
    warm.dispatch(Command::SetPersistentIndex(true));
    settle(&mut warm, complete);
    drop(warm);
    // Branch-like rewrites keep byte lengths and mtimes: only content can authorize reuse.
    for ((name, text), time) in fixtures.iter().zip(&modified) {
        let path = root.join(name);
        std::fs::write(&path, text.replace("Alpha", "Omega")).unwrap();
        std::fs::File::options().write(true).open(path).unwrap().set_times(std::fs::FileTimes::new().set_modified(*time)).unwrap();
    }
    let mut changed = session(d.path());
    changed.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut changed, complete);
    check(&changed);
    assert_eq!(index::rank(&changed.state.index.snapshot(), "Alpha", || true).unwrap().1, 0);
    assert!(snapshot(&changed).iter().all(|(_, records)| records.contains("Omega")));
    changed.watcher = Watcher::new(|_| true);
    for (name, text) in fixtures { std::fs::write(root.join(name), text).unwrap(); }
    changed.dispatch(Command::RebuildIndex);
    settle(&mut changed, |s| !s.index_rebuilding && complete(s));
    assert_eq!(snapshot(&changed), original);
    for (name, text) in fixtures { assert_eq!(std::fs::read(root.join(name)).unwrap(), text.as_bytes()); }
}

fn opened(s: &Session) -> bool {
    s.state().loading().is_none() && (s.state().current_source().is_some() || !s.state().notices().is_empty())
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn an_empty_session_offers_open_and_explains_the_rest() {
    let d = tempfile::tempdir().unwrap();
    let s = session(d.path());
    let actions = s.actions();
    let disabled = |id| actions.iter().find(|a| a.id == id).unwrap().disabled;
    assert_eq!(disabled(CommandId::Open), None);
    assert_eq!(disabled(CommandId::ChooseTheme), None);
    assert_eq!(disabled(CommandId::Reload), Some("No file is open"));
    assert_eq!(disabled(CommandId::Copy), Some("No file is open"));
    assert_eq!(actions.iter().map(|a| a.label).collect::<Vec<_>>(), ["Open File…", "Open Folder…", "Open Recent Folder…", "Find File…", "Search in Project…", "Find in File…", "Next Match", "Previous Match", "Close Find", "Find: Match Case", "Find: Whole Word", "Find: Regular Expression", "List All Matches", "Go to Line…", "Soft Wrap", "Sticky Scroll", "Breadcrumbs", "Focus Breadcrumbs", "File Outline", "Find Declaration…", "Rerun Search", "Close Results", "Back", "Forward", "Toggle Bookmark", "Set Bookmark Mnemonic…", "Show Bookmarks…", "Next Bookmark", "Previous Bookmark", "Recent Files…", "Recent Locations…", "Keep Tab Open", "Next Tab", "Previous Tab", "Reload", "Close Tab", "Close Window", "Quit All Windows", "Select All", "Copy", "Undo", "Redo", "Save", "Recheck Saved File", "Confirm Rechecked Save", "Copy Path", "Copy Path:Line", "Open in Editor", "Open with Default App", "Compare Saved Disk Inputs…", "Compare with Last Saved Version", "Close Saved Comparison", "Compare Selected Saved Disk Inputs", "Collapse", "Expand", "Collapse All", "Expand All", "Collapse Comments and Imports", "Extend Selection", "Shrink Selection", "Change theme", "About Lenz Code", "New Window", "Go to Definition", "Peek Definition", "Go to Type Definition", "Go to Implementation", "Find Usages", "Call Hierarchy", "Call Hierarchy: Callers", "Call Hierarchy: Callees", "Type Hierarchy", "Type Hierarchy: Supertypes", "Type Hierarchy: Subtypes", "Quick Documentation", "Documentation on Hover", "File Icons", "Inlay Hints: Types", "Inlay Hints: Parameter Names", "Semantic Highlighting", "Find Symbol (language server)…", "File Symbols (language server)", "Set Workspace Root…", "Language Server Status…", "Server Details…", "Copy Language Server Details", "Choose Language Server Runtime…", "Restart Language Server", "Exclude Ruby Tooling Gems for This Project", "Reset LSP Settings for This Project", "Reuse Saved Declaration Index", "Rebuild Project Index", "Disable Language Server for This Folder", "Enable Language Server for This Folder", "Image Source / Rendered", "Markdown Source / Preview", "Show / Hide Explorer", "Select Opened File in Explorer", "Explorer: Folders First", "Explorer: All by Name", "Git Branches…", "New Branch…", "Checkout Tag or Revision…", "Show Local Changes", "Compare with Branch, Tag or Revision…", "Compare with Base Branch", "Show Changes Since Branch Start", "Compare with Worktree…", "Resolve Conflicts in Lenz Diff", "Annotate with Git Blame", "Close Annotations", "Show Commit…", "Show Diff", "Copy Revision", "Split Right", "Split Down", "Close Split", "Focus Other Pane", "Move Tab to Other Pane", "Search Everywhere…", "Find Symbol…", "Find Action…", "Status details…"]);
    assert_eq!(disabled(CommandId::FoldersFirst), Some("Already sorted this way"));
    assert_eq!(disabled(CommandId::LocateFile), Some("No file is open"));
    assert_eq!(disabled(CommandId::ImageSource), Some("Not an image"));
    assert_eq!(disabled(CommandId::MarkdownSource), Some("Not a Markdown file"));
    assert_eq!(disabled(CommandId::Symbols), Some("No folder is open"));
    assert_eq!(disabled(CommandId::LanguageServer), None);
    assert_eq!(disabled(CommandId::CopyServerDetails), None);
    assert!(disabled(CommandId::ServerRuntime).is_some());
    assert_eq!(disabled(CommandId::RestartServer), Some("No language server for this file"));
    assert_eq!(disabled(CommandId::Definition), Some("No file is open"));
    assert_eq!(disabled(CommandId::WorkspaceRoot), Some("No file is open"));
    assert_eq!(disabled(CommandId::FindFile), None, "file search can show recent files before a folder opens");
    assert_eq!(disabled(CommandId::Back), Some("Nothing to go back to"));
    assert_eq!(disabled(CommandId::Close), Some("No tab is open"));
    assert_eq!(disabled(CommandId::NextTab), Some("Only one tab is open"));
    assert_eq!(disabled(CommandId::OpenRecent), Some("No recent folders"));
    assert_eq!(disabled(CommandId::OpenInEditor), Some("No file is open"));
    assert_eq!(disabled(CommandId::CompareWith), Some("No file or folder is open"));
    assert!(s.state().current_source().is_none() && s.state().loading().is_none());
}

#[test]
fn opening_shows_text_then_highlights_without_touching_the_file() {
    let d = tempfile::tempdir().unwrap();
    let bytes = b"fn main() {\r\n    let caf\xc3\xa9 = 1;\r\n}\r\n";
    let path = write(d.path(), "main.rs", bytes);
    let mut s = session(d.path());
    assert!(s.dispatch(Command::Open(path.clone())).repaint);
    assert_eq!(s.state().loading(), Some(path.as_path()));
    settle(&mut s, |s| s.state().current_source().is_some());
    let doc = s.state().current_source().unwrap().clone();
    assert_eq!(source_line(&doc, 1), "    let café = 1;");
    settle(&mut s, |s| s.state().highlights().is_some());
    assert!(!s.state().highlights().unwrap().spans(0..1)[0].is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), bytes, "reading never writes");
}

#[test]
fn selection_is_clamped_to_character_boundaries_and_copies_source_text() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "a.txt", "héllo\r\nwörld\n".as_bytes());
    let mut s = session(d.path());
    s.dispatch(Command::Open(path));
    settle(&mut s, opened);
    assert_eq!(s.disabled(CommandId::Copy), Some("Nothing is selected"));
    s.dispatch(Command::Select(Selection { anchor: 2, head: 8 }));
    assert_eq!(s.state().selection(), Some(Selection { anchor: 1, head: 8 }), "inside é snaps back");
    assert_eq!(s.dispatch(Command::Copy).copy.as_deref(), Some("éllo\r\n"));
    s.dispatch(Command::Select(Selection { anchor: 1000, head: 0 }));
    assert_eq!(s.dispatch(Command::Copy).copy.as_deref(), Some("héllo\r\nwörld\n"));
    s.dispatch(Command::SelectAll);
    assert_eq!(s.state().selection().unwrap().range(), 0.."héllo\r\nwörld\n".len());
}

#[test]
fn scrolling_is_clamped_to_the_document() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "a.txt", b"1\n2\n3");
    let mut s = session(d.path());
    assert!(!s.dispatch(Command::Scroll(5)).repaint, "nothing to scroll");
    s.dispatch(Command::Open(path));
    settle(&mut s, opened);
    s.dispatch(Command::Scroll(99));
    assert_eq!(s.state().top_line(), 2);
}

#[test]
fn unavailable_files_report_instead_of_opening() {
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::Open(d.path().join("missing.rs")));
    settle(&mut s, opened);
    assert!(s.state().current_source().is_none());
    assert!(s.state().notices().iter().any(|n| n.starts_with("Cannot open")), "{:?}", s.state().notices());
    s.dispatch(Command::Open(d.path().to_owned()));
    settle(&mut s, |s| s.state().notices().iter().any(|n| n.contains("not a regular file")));
}

#[test]
fn a_superseded_load_is_discarded() {
    let d = tempfile::tempdir().unwrap();
    let (a, b) = (write(d.path(), "a.txt", b"a\n"), write(d.path(), "b.txt", b"b\n"));
    let mut s = session(d.path());
    s.dispatch(Command::Open(a.clone()));
    let (stale, tab) = (s.request, s.state().active().unwrap().id());
    s.dispatch(Command::Open(b.clone()));
    // The first load's result arrives after the second open.
    let doc = Arc::new(Document::decode(a.clone(), 99, b"a\n"));
    let effect = s.dispatch(Completion::Loaded { request: stale, tab, path: a, result: Ok((None, doc, None)) });
    assert_eq!(effect, Effect::default());
    assert_eq!(s.state().loading(), Some(b.as_path()));
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source() == b));
    assert_eq!(source_line(s.state().current_source().unwrap(), 0), "b");
}

#[test]
fn stale_highlights_are_discarded() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "a.rs", b"fn a() {}\n");
    let mut s = session(d.path());
    s.dispatch(Command::Open(path));
    settle(&mut s, |s| s.state().current_source().is_some());
    let old = s.state().current_source().unwrap().key().version - 1;
    let parsed = highlight::parse_while(highlight::Language::Rust, "fn a() {}\n".into(), || true);
    assert_eq!(s.dispatch(Completion::Highlighted { version: old, parsed, folds: None, declarations: None }), Effect::default());
}

#[test]
fn reload_reads_a_new_version_and_keeps_the_last_content_when_the_file_disappears() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "a.txt", b"one\ntwo\nthree\n");
    let mut s = session(d.path());
    s.dispatch(Command::Open(path.clone()));
    settle(&mut s, opened);
    let first = s.state().current_source().unwrap().key().version;
    s.dispatch(Command::Scroll(2));
    std::fs::write(&path, b"uno\n").unwrap();
    s.dispatch(Command::Reload);
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.key().version > first));
    assert_eq!(source_line(s.state().current_source().unwrap(), 0), "uno");
    assert_eq!(s.state().top_line(), 1, "kept, clamped to the new length");
    std::fs::remove_file(&path).unwrap();
    s.dispatch(Command::Reload);
    settle(&mut s, |s| s.state().loading().is_none());
    assert_eq!(source_line(s.state().current_source().unwrap(), 0), "uno");
    assert!(s.state().notices().iter().any(|n| n.starts_with("Unavailable")));
    assert!(!path.exists(), "the file is never recreated");
}

#[test]
fn leaving_a_file_keeps_it_inactive_and_returning_reuses_it() {
    let d = tempfile::tempdir().unwrap();
    let (a, b) = (write(d.path(), "a.txt", b"alpha\n"), write(d.path(), "b.txt", b"beta\n"));
    let mut s = session(d.path());
    s.dispatch(Command::Open(a.clone()));
    settle(&mut s, opened);
    let first = s.state().current_source().unwrap().clone();
    let current = s.state().current().unwrap();
    s.dispatch(Command::Open(b));
    settle(&mut s, |s| s.state().current_source().is_some());
    assert_eq!(s.inactive_bytes(), current.storage_bytes());
    s.dispatch(Command::Open(a.clone()));
    settle(&mut s, |s| s.state().current_source().is_some());
    assert!(Arc::ptr_eq(s.state().current_source().unwrap(), &first), "served from the cache, not read again");
    assert!(Arc::ptr_eq(&s.state().current().unwrap(), &current), "the same indexed text is reused");
    assert_eq!(s.inactive_bytes(), doc_bytes(&s, "b.txt"));
    // A file changed since it was left is read again.
    s.dispatch(Command::Open(d.path().join("b.txt")));
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source().ends_with("b.txt")));
    std::fs::write(&a, b"alpha, longer now\n").unwrap();
    s.dispatch(Command::Open(a));
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source().ends_with("a.txt")));
    assert_eq!(source_line(s.state().current_source().unwrap(), 0), "alpha, longer now");
    s.dispatch(Command::Close);
    assert!(s.state().current_source().is_none());
    assert_eq!(s.disabled(CommandId::Reload), Some("No file is open"));
}

#[test]
fn large_files_skip_syntax_and_say_so() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "big.rs", "fn a() {}\n".repeat(crate::document::LARGE_LINES + 1).as_bytes());
    let mut s = session(d.path());
    s.dispatch(Command::Open(path));
    settle(&mut s, opened);
    let doc = s.state().current_source().unwrap().clone();
    assert!(doc.is_large());
    assert!(s.state().notices().iter().any(|n| n.starts_with("Large file")));
    std::thread::sleep(Duration::from_millis(100));
    while let Ok(c) = s.completions().try_recv() {
        assert!(!matches!(c, Completion::Highlighted { .. }), "no syntax job for a large file");
    }
}

#[test]
fn theme_selection_is_the_readers_own() {
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::Theme(ThemeCommand::Select(lenz_content::theme::ThemeSelection::Named("light".into()))));
    assert_eq!(s.state().palette().bg, lenz_content::theme::LIGHT.bg);
    assert_eq!(s.theme_id(), "light");
    drop(s);
    let saved = std::fs::read_to_string(d.path().join("config/lenz-reader/appearance.json")).unwrap();
    assert!(saved.contains("\"light\""));
    assert!(!d.path().join("config/lenz").exists(), "Lenz's settings are untouched");
}

#[test]
fn theme_preview_does_not_select_or_save_and_selection_finishes_it() {
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    let source = s.theme_source();
    s.dispatch(Command::PreviewTheme(Some("light".into())));
    assert_eq!(s.state().palette().bg, lenz_content::theme::LIGHT.bg);
    assert_eq!(s.theme_id(), "system");
    assert_eq!(s.theme_source(), source);
    assert!(!d.path().join("config/lenz-reader/appearance.json").exists());
    let effect = s.dispatch(Command::Theme(ThemeCommand::Select(lenz_content::theme::ThemeSelection::System)));
    assert!(effect.repaint, "ending a preview repaints even when the saved selection stays the same");
    assert_eq!(s.state().palette().bg, lenz_content::theme::DARCULA.bg);
    s.dispatch(Command::PreviewTheme(Some("light".into())));
    s.dispatch(Command::PreviewTheme(None));
    assert_eq!(s.state().palette().bg, lenz_content::theme::DARCULA.bg);
    s.dispatch(Command::PreviewTheme(Some("light".into())));
    s.dispatch(Command::Theme(ThemeCommand::Select(lenz_content::theme::ThemeSelection::Named("gruvbox".into()))));
    assert_eq!(s.theme_id(), "gruvbox");
    assert_eq!(s.state().palette().bg, 0x282828);
    s.dispatch(Command::PreviewTheme(None));
    assert_eq!(s.state().palette().bg, 0x282828);
}

#[test]
fn cancelling_theme_preview_restores_the_latest_system_appearance() {
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::PreviewTheme(Some("gruvbox".into())));
    s.dispatch(Command::Theme(ThemeCommand::NativeAppearance(false)));
    assert_eq!(s.state().palette().bg, 0x282828);
    s.dispatch(Command::PreviewTheme(None));
    assert_eq!(s.state().palette().bg, lenz_content::theme::LIGHT.bg);
    s.dispatch(Command::PreviewTheme(Some("not-an-installed-theme".into())));
    assert_eq!(s.state().palette().bg, lenz_content::theme::LIGHT.bg);
    assert_eq!(s.theme_id(), "system");
}

#[test]
fn keyboard_motions_extend_the_selection_on_character_boundaries() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "a.txt", "aé\r\nlonger line\n€".as_bytes());
    let mut s = session(d.path());
    s.dispatch(Command::Open(path));
    settle(&mut s, opened);
    let mut head = |m| { s.dispatch(Command::Extend(m)); s.state().selection().unwrap().head };
    assert_eq!(head(Motion::Right), 1);
    assert_eq!(head(Motion::Right), 3, "over é");
    assert_eq!(head(Motion::Right), 4, "into the line ending, then past it");
    assert_eq!(head(Motion::Right), 5);
    assert_eq!(head(Motion::Down), 17, "column 0 of the last line");
    assert_eq!(head(Motion::Right), 20, "over €");
    assert_eq!(head(Motion::Up), 8, "the same byte column on line 1");
    assert_eq!(head(Motion::LineStart), 5);
    assert_eq!(head(Motion::LineEnd), 16);
    assert_eq!(head(Motion::Left), 15);
    assert_eq!(head(Motion::LineStart), 5);
    assert_eq!(head(Motion::Up), 0);
    assert_eq!(head(Motion::Up), 0, "the first line's start stays");
    let sel = s.state().selection().unwrap();
    assert_eq!(sel.anchor, 0);
}

/// Opening and closing many files keeps reloadable content within the inactive budget and releases the rest.
#[test]
fn repeated_opening_releases_content_beyond_the_budget() {
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    let mut first = None;
    for i in 0..48 {
        let path = write(d.path(), &format!("{i}.txt"), &vec![b'a'; 1 << 20]);
        s.dispatch(Command::Open(path));
        settle(&mut s, |s| s.state().current_source().is_some());
        if i == 0 {
            first = Some(Arc::downgrade(s.state().current_source().unwrap()));
        }
        assert!(s.inactive_bytes() <= crate::cache::INACTIVE_BYTES, "{} bytes kept", s.inactive_bytes());
    }
    s.dispatch(Command::Close);
    assert!(s.inactive_bytes() > 16 << 20, "recently left files stay for a quick return");
    assert!(first.unwrap().upgrade().is_none(), "the oldest left file was released");
}

/// Component timing, not the window startup targets: `cargo test --release -p reader-core -- --ignored
/// --nocapture open_perf`. Each trial is a fresh session (empty reader cache, warm OS page cache) opening an
/// ordinary 2,000-line Rust file: time to text shown, then to syntax ready.
#[test]
#[ignore = "timing: run in release, alone"]
fn open_perf() {
    let d = tempfile::tempdir().unwrap();
    let unit = "/// Doc.\npub fn item(a: u32) -> Option<String> {\n    let s = \"text\";\n    if a > 0 { Some(format!(\"{a}{s}\")) } else { None }\n}\n";
    for (name, copies) in [("ordinary-56k", 400), ("ordinary-1mib", 7500)] {
        open_trials(d.path(), name, &unit.repeat(copies));
    }
}

fn open_trials(dir: &Path, name: &str, source: &str) {
    let path = write(dir, &format!("{name}.rs"), source.as_bytes());
    let lines = source.lines().count() + 1;
    let (mut shown, mut highlighted) = (Vec::new(), Vec::new());
    for trial in 0..101 {
        let mut s = session(dir);
        let started = std::time::Instant::now();
        s.dispatch(Command::Open(path.clone()));
        let rx = s.completions();
        while s.state().current_source().is_none() {
            s.dispatch(rx.recv_blocking_timeout().expect("load"));
        }
        let text = started.elapsed();
        while s.state().highlights().is_none() {
            s.dispatch(rx.recv_blocking_timeout().expect("syntax"));
        }
        let syntax = started.elapsed();
        assert_eq!(s.state().current_source().unwrap().line_count(), lines);
        if trial > 0 {
            shown.push(text.as_secs_f64() * 1000.0);
            highlighted.push(syntax.as_secs_f64() * 1000.0);
        }
    }
    let report = |name: &str, v: &mut Vec<f64>| {
        v.sort_by(f64::total_cmp);
        let p = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
        eprintln!("READER_PERF {{\"name\":{name:?},\"unit\":\"ms\",\"samples\":{v:?}}}");
        eprintln!("{name}: median {:.3} ms, p95 {:.3} ms, max {:.3} ms over {} trials", p(0.5), p(0.95), v[v.len() - 1], v.len());
    };
    eprintln!("{name}: {} bytes, {lines} lines, release={}, source {}", source.len(), !cfg!(debug_assertions), env!("CARGO_PKG_VERSION"));
    report(&format!("reader/open-{name}-text-shown"), &mut shown);
    report(&format!("reader/open-{name}-syntax-ready"), &mut highlighted);
}

fn tree(dir: &Path, files: &[&str]) {
    for f in files {
        let p = dir.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, f.as_bytes()).unwrap();
    }
}

fn scanned(s: &Session) -> bool {
    s.state().project().is_some_and(|p| !matches!(p.coverage(), crate::project::Coverage::Scanning(_)))
}

#[test]
fn opening_a_folder_discovers_it_and_the_finder_ranks_its_files() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    tree(&root, &["src/main.rs", "src/ui/main.rs", "README.md", ".gitignore", "build/out.rs"]);
    std::fs::write(root.join(".gitignore"), "build/\n").unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    assert!(matches!(s.state().project().unwrap().coverage(), crate::project::Coverage::Scanning(_)), "partial until the walk ends");
    settle(&mut s, scanned);
    let p = s.state().project().unwrap();
    assert_eq!(p.coverage(), &crate::project::Coverage::Complete(4));
    assert_eq!(p.rows().iter().map(|r| &*r.name).collect::<Vec<_>>(), ["src", ".gitignore", "README.md"], "folders first by default");
    assert_eq!(s.disabled(CommandId::FindFile), None);
    s.dispatch(Command::FindFile("main".into()));
    settle(&mut s, |s| s.state().project().unwrap().finder().unwrap().ready);
    let f = s.state().project().unwrap().finder().unwrap();
    assert_eq!(f.hits.iter().map(|h| &**h).collect::<Vec<_>>(), ["src/main.rs", "src/ui/main.rs"], "duplicate names by path");
    assert_eq!(f.searched, 4);
    // Selecting a result opens it; an ignored file opened explicitly is readable too.
    s.dispatch(Command::Open(root.join(&*f.hits[1].clone())));
    settle(&mut s, |s| s.state().current_source().is_some());
    assert_eq!(source_line(s.state().current_source().unwrap(), 0), "src/ui/main.rs");
    s.dispatch(Command::Open(root.join("build/out.rs")));
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source().ends_with("out.rs")));
    s.dispatch(Command::ToggleDir("src".into()));
    assert!(s.state().project().unwrap().rows().iter().any(|r| &*r.path == "src/ui"));
    s.dispatch(Command::CloseFinder);
    assert!(s.state().project().unwrap().finder().is_none());
}

#[test]
fn the_tree_order_is_remembered_and_the_opened_file_can_be_located() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    tree(&root, &["src/ui/view.rs", "src/lib.rs", "a.md"]);
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, scanned);
    let names = |s: &Session| s.state().project().unwrap().rows().iter().map(|r| r.path.to_string()).collect::<Vec<_>>();
    assert_eq!(names(&s), ["src", "a.md"]);
    assert_eq!(s.disabled(CommandId::LocateFile), Some("No file is open"));
    s.dispatch(Command::Open(root.join("src/ui/view.rs")));
    settle(&mut s, opened);
    assert_eq!(s.state().active_in_tree().as_deref(), Some("src/ui/view.rs"));
    assert_eq!(s.disabled(CommandId::LocateFile), None);
    s.dispatch(Command::Locate);
    assert_eq!(names(&s), ["src", "src/ui", "src/ui/view.rs", "src/lib.rs", "a.md"]);
    std::fs::write(d.path().join("outside.rs"), "").unwrap();
    s.dispatch(Command::Open(d.path().join("outside.rs")));
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source().ends_with("outside.rs")));
    assert_eq!(s.disabled(CommandId::LocateFile), Some("The file is not in the explorer"));
    s.dispatch(Command::SetFoldersFirst(false));
    assert_eq!(names(&s), ["a.md", "src", "src/lib.rs", "src/ui", "src/ui/view.rs"]);
    // A new session reads the order back.
    let saved = d.path().join("state/lenz-reader/settings.json");
    settle(&mut s, |_| saved.exists());
    drop(s);
    let mut s = session(d.path());
    assert!(!s.state().folders_first());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, scanned);
    assert_eq!(names(&s), ["a.md", "src", "src/lib.rs", "src/ui", "src/ui/view.rs"], "the workspace remembers expanded folders too");
}

#[test]
fn replacing_the_root_discards_the_old_scan_and_remembers_roots() {
    let d = tempfile::tempdir().unwrap();
    let (a, b) = (d.path().join("a"), d.path().join("b"));
    tree(&a, &["a1.rs", "a2.rs"]);
    tree(&b, &["b1.rs"]);
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(a.clone()));
    let old = s.state().project().unwrap().generation();
    s.dispatch(Command::OpenRoot(b.clone()));
    // A late batch from the first root does not reach the second.
    s.dispatch(Completion::Scanned { generation: old, batch: crate::project::Batch { paths: vec!["a1.rs".into()], done: true, ..Default::default() } });
    settle(&mut s, scanned);
    let p = s.state().project().unwrap();
    assert_eq!((p.root(), p.files()), (b.as_path(), 1));
    assert_eq!(s.state().recent(), [b.clone(), a.clone()]);
    assert_eq!(s.disabled(CommandId::OpenRecent), None);
    drop(s);
    let recent_file = d.path().join("state/lenz-reader/recent-roots.json");
    for _ in 0..200 {
        if read_recent(&recent_file).len() == 2 { break; }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(session(d.path()).state().recent(), [b, a], "kept outside the checkout across sessions");
}

#[test]
fn missing_and_empty_roots_say_so() {
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(d.path().join("missing")));
    settle(&mut s, scanned);
    assert!(matches!(s.state().project().unwrap().coverage(), crate::project::Coverage::Unavailable(_)));
    let empty = d.path().join("empty");
    std::fs::create_dir(&empty).unwrap();
    s.dispatch(Command::OpenRoot(empty));
    settle(&mut s, scanned);
    assert_eq!(s.state().project().unwrap().coverage(), &crate::project::Coverage::Complete(0));
    assert!(s.state().project().unwrap().rows().is_empty());
}

/// Finder timing with the inventory loaded: `cargo test --release -p reader-core -- --ignored --nocapture finder_perf`.
#[test]
#[ignore = "timing: run in release, alone"]
fn finder_perf() {
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    let root = d.path().join("big");
    std::fs::create_dir(&root).unwrap();
    s.dispatch(Command::OpenRoot(root));
    let generation = s.state().project().unwrap().generation();
    // 100,000 synthetic paths across nested directories, as discovery batches.
    let paths: Vec<String> = (0..100_000).map(|i| format!("crate_{}/src/module_{}/file_{i}.rs", i % 37, i % 211)).collect();
    settle(&mut s, scanned); // the empty real walk finishes first
    for chunk in paths.chunks(crate::project::BATCH) {
        s.dispatch(Completion::Scanned { generation, batch: crate::project::Batch { paths: chunk.to_vec(), ..Default::default() } });
    }
    s.dispatch(Completion::Scanned { generation, batch: crate::project::Batch { done: true, ..Default::default() } });
    assert_eq!(s.state().project().unwrap().files(), 100_000);
    let queries = ["f", "fi", "file_9", "mod", "module_42/f", "c3s", "rs", "zzz", "file_99999", "crate_1/src"];
    let mut samples = Vec::new();
    for round in 0..101 {
        for q in queries {
            s.dispatch(Command::CloseFinder);
            let started = std::time::Instant::now();
            s.dispatch(Command::FindFile(q.to_string()));
            let rx = s.completions();
            while !s.state().project().unwrap().finder().unwrap().ready {
                s.dispatch(rx.recv_blocking_timeout().expect("ranked"));
            }
            if round > 0 {
                samples.push(started.elapsed().as_secs_f64() * 1000.0);
            }
        }
    }
    samples.sort_by(f64::total_cmp);
    let p = |q: f64| samples[((samples.len() - 1) as f64 * q).round() as usize];
    eprintln!("READER_PERF {{\"name\":\"reader/finder-100k\",\"unit\":\"ms\",\"samples\":{samples:?}}}");
    eprintln!("finder over 100,000 files: median {:.2} ms, p95 {:.2} ms, max {:.2} ms over {} queries", p(0.5), p(0.95), samples[samples.len() - 1], samples.len());
    assert!(p(0.95) < 50.0, "finder p95 {:.1} ms exceeds the 50 ms target", p(0.95));
}

#[test]
fn replacing_selection_expansion_bounds_queued_source_owners() {
    let d = tempfile::tempdir().unwrap(); let file = write(d.path(), "expand.rs", b"fn main() { let value = 1; value; }\n"); let mut s = session(d.path());
    open(&mut s, Command::Open(file), "expand.rs"); settle(&mut s, |s| s.state().highlights().is_some());
    let (started, entered) = std::sync::mpsc::channel();
    let releases: Vec<_> = (0..crate::jobs::MAX_JOBS).map(|_| { let (release, wait) = std::sync::mpsc::channel(); let started = started.clone();
        s.jobs.submit(Priority::Background, move || { started.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); }); release }).collect();
    for _ in &releases { entered.recv_timeout(Duration::from_secs(5)).unwrap(); }
    for index in 0..129 {
        let at = index % 10 + 3; s.dispatch(Command::Select(Selection { anchor: at, head: at })); s.dispatch(Command::ExtendSelection);
    }
    let queued = s.jobs.queued();
    for release in releases { release.send(()).unwrap(); }
    settle(&mut s, |s| s.expanding.is_none());
    assert!(queued <= 2 * crate::jobs::MAX_JOBS, "superseded syntax selection requests retained {queued} source closures");
}

#[test]
fn split_find_consumers_keep_independent_latest_queries_for_one_snapshot() {
    let d = tempfile::tempdir().unwrap(); let file = write(d.path(), "split.txt", b"alpha beta beta gamma gamma gamma\n");
    let mut s = session(d.path()); open(&mut s, Command::Open(file), "split.txt");
    s.dispatch(Command::Split(Split::Columns));
    let first = s.state().shown(0).unwrap().id(); let second = s.state().shown(1).unwrap().id();
    let (started, entered) = std::sync::mpsc::channel();
    let releases: Vec<_> = (0..crate::jobs::MAX_JOBS).map(|_| { let (release, wait) = std::sync::mpsc::channel(); let started = started.clone();
        s.jobs.submit(Priority::Visible, move || { started.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); }); release }).collect();
    for _ in &releases { entered.recv_timeout(Duration::from_secs(5)).unwrap(); }
    s.dispatch(Command::FocusPane(0)); s.dispatch(Command::Find("alpha".into()));
    s.dispatch(Command::FocusPane(1)); s.dispatch(Command::Find("beta".into()));
    s.dispatch(Command::FocusPane(0));
    for _ in 0..128 { s.dispatch(Command::Find("gamma".into())); }
    for release in releases { release.send(()).unwrap(); }
    settle(&mut s, |s| [first, second].iter().all(|id| s.state.tabs.iter().find(|t| t.id == *id).unwrap().find.as_ref().is_some_and(|f| !f.pending)));
    assert_eq!(s.state.tabs.iter().find(|t| t.id == first).unwrap().find.as_ref().unwrap().matches.len(), 3);
    assert_eq!(s.state.tabs.iter().find(|t| t.id == second).unwrap().find.as_ref().unwrap().matches.len(), 2);
}

#[test]
fn visible_find_is_admitted_when_background_analysis_submissions_are_saturated() {
    let d = tempfile::tempdir().unwrap(); let mut s = session(d.path());
    let mut inputs = Vec::new();
    // Explicit analysis owners keep these inputs ready while admission is gated.
    let mut background_documents = Vec::new();
    for i in 0..3 {
        let path = write(d.path(), &format!("background-{i}.txt"), b"background\n");
        s.dispatch(Command::OpenRetained(path)); settle(&mut s, |s| s.state().loading().is_none() && s.state().current_source().is_some());
        let snapshot = s.state().view(s.state.active.unwrap()).unwrap().snapshot().clone();
        let doc = analysis_document(&mut s);
        inputs.push(crate::analysis::Input::new(snapshot, Arc::downgrade(&doc), 0, false));
        background_documents.push(doc);
    }
    let path = write(d.path(), "foreground.txt", b"needle\n"); open(&mut s, Command::Open(path), "foreground.txt");
    let (started, entered) = std::sync::mpsc::channel();
    let releases: Vec<_> = (0..crate::jobs::MAX_JOBS).map(|_| { let (release, wait) = std::sync::mpsc::channel(); let started = started.clone();
        s.jobs.submit(Priority::Visible, move || { started.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); }); release }).collect();
    for _ in &releases { entered.recv_timeout(Duration::from_secs(5)).unwrap(); }
    let mut background = Vec::new();
    for input in inputs {
        let (release, wait) = std::sync::mpsc::channel(); background.push(release);
        s.analysis.request(input, crate::analysis::Consumer::Index, 1, Priority::Background, move |_, _lease| { let _ = wait.recv_timeout(Duration::from_secs(5)); });
    }
    s.dispatch(Command::SelectAll); // admit the three current background consumers
    s.dispatch(Command::Find("needle".into()));
    releases[0].send(()).unwrap();
    let rx = s.completions(); let until = std::time::Instant::now() + Duration::from_millis(500); let mut found = false;
    while std::time::Instant::now() < until {
        if let Ok(c) = rx.try_recv() {
            found |= matches!(&c, Completion::Analyzed { result, .. } if matches!(result.as_ref(), Completion::Matched { .. }));
            s.dispatch(c); if found { break; }
        } else { std::thread::yield_now(); }
    }
    for release in background { let _ = release.send(()); }
    for release in releases.iter().skip(1) { let _ = release.send(()); }
    settle(&mut s, |s| s.state().active().unwrap().find().is_some_and(|f| !f.pending));
    assert!(found, "visible work must reach the pool's visible queue despite saturated background admission");
}

#[test]
fn equal_version_numbers_in_different_buffers_cannot_share_analysis_results() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "a.rs", b"fn a() {}\n"); let b = write(d.path(), "b.rs", b"fn b() {}\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(a), "a.rs");
    s.dispatch(Command::Split(Split::Columns));
    open(&mut s, Command::Open(b), "b.rs");
    settle(&mut s, |s| s.state().shown(0).unwrap().highlights.is_some() && s.state().shown(1).unwrap().highlights.is_some());
    let tabs: Vec<_> = [s.state.active.unwrap(), s.state.other.unwrap()].into_iter().collect();
    for tab in &tabs {
        let old = s.state().view(*tab).unwrap().snapshot().clone();
        let text = format!("fn only_{}() {{}}\n", tab.0);
        let current = Arc::new(old.apply_edits(99, &[crate::text::TextEdit { range: 0..old.len_bytes(), text: text.into() }]).unwrap());
        s.state.sources.buffers.get_mut(&old.key().buffer).unwrap().snapshot = Some(current);
        s.tab_mut(*tab).unwrap().new_content();
    }
    let (started, entered) = std::sync::mpsc::channel();
    let releases: Vec<_> = (0..crate::jobs::MAX_JOBS).map(|_| { let (release, wait) = std::sync::mpsc::channel(); let started = started.clone();
        s.jobs.submit(Priority::Visible, move || { started.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); }); release }).collect();
    for _ in &releases { entered.recv_timeout(Duration::from_secs(5)).unwrap(); }
    s.dispatch(Command::FocusPane(0)); s.dispatch(Command::FocusPane(1));
    releases[0].send(()).unwrap();
    let rx = s.completions();
    let first = loop { let c = rx.recv_blocking_timeout().unwrap(); if matches!(&c, Completion::Analyzed { result, .. } if matches!(result.as_ref(), Completion::Highlighted { .. })) { break c; } s.dispatch(c); };
    s.dispatch(first);
    let installed = tabs.iter().filter(|tab| s.state.tabs.iter().find(|t| t.id == **tab).unwrap().highlights.is_some()).count();
    for release in releases.iter().skip(1) { release.send(()).unwrap(); }
    settle(&mut s, |s| s.state().shown(0).unwrap().highlights.is_some() && s.state().shown(1).unwrap().highlights.is_some());
    assert_eq!(installed, 1, "a result carries buffer identity in addition to version");
}

#[test]
fn a_panicking_old_consumer_releases_its_version_and_admits_the_latest_find() {
    let d = tempfile::tempdir().unwrap();
    let file = write(d.path(), "panic.txt", b"old\n");
    let mut s = session(d.path());
    open(&mut s, Command::Open(file.clone()), "panic.txt");
    let tab = s.state.active.unwrap();
    let snapshot = s.state().view(tab).unwrap().snapshot().clone();
    let worker_document = analysis_document(&mut s);
    let loaded = Arc::downgrade(&worker_document);
    let input = crate::analysis::Input::new(snapshot, loaded, s.project_generation.load(Ordering::Relaxed), true);
    let (started, entered) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    // The injected unwind is at the real CPU execution boundary; the actual pool catches it.
    s.analysis.request(input, crate::analysis::Consumer::Syntax, 99, Priority::Background, move |_, _lease| {
        started.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); panic!("controlled analysis unwind");
    });
    s.dispatch(Command::Find("latest".into()));
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    let mut replaced = None;
    for text in ["discarded\n", "latest\n"] {
        std::fs::write(&file, text).unwrap();
        s.dispatch(Command::Reload);
        let version = s.versions.fetch_add(1, Ordering::Relaxed) + 1;
        s.dispatch(Completion::Loaded { request: s.request, tab, path: file.clone(), result: Ok((Stamp::of(&file), Arc::new(Document::decode(file.clone(), version, text.as_bytes())), std::fs::canonicalize(&file).ok())) });
        if replaced.is_none() { replaced = Some(Arc::downgrade(s.state().view(tab).unwrap().snapshot())); }
    }
    let released = replaced.unwrap().upgrade().is_none();
    release.send(()).unwrap();
    settle(&mut s, |s| s.state().active().unwrap().find().is_some_and(|f| !f.pending));
    assert!(released, "replacing pending demand drops its snapshot before the old worker returns");
    assert_eq!(s.state().active().unwrap().find().unwrap().matches.len(), 1);
    assert_eq!(source_text(s.state().current_source().unwrap()), "latest\n");
}

#[test]
fn visible_find_promotes_its_queued_background_materialization() {
    for operation in [Command::Find("latest_draft".into()), Command::Documentation] {
    let finding = matches!(operation, Command::Find(_));
    let d = tempfile::tempdir().unwrap();
    let file = write(d.path(), "draft.rs", b"fn saved() {}\n");
    let mut s = session(d.path());
    open(&mut s, Command::Open(file), "draft.rs");
    settle(&mut s, |s| s.state().highlights().is_some());
    let tab = s.state.active.unwrap();
    let old = s.state().view(tab).unwrap().snapshot().clone();
    let current = Arc::new(old.apply_edits(old.key().version + 1,
        &[crate::text::TextEdit { range: 0..old.len_bytes(), text: Arc::from("fn latest_draft() {}\n") }]).unwrap());
    s.state.sources.buffers.get_mut(&current.key().buffer).unwrap().snapshot = Some(current.clone());
    s.active_mut().unwrap().new_content();
    let (started, entered) = std::sync::mpsc::channel();
    let releases: Vec<_> = (0..crate::jobs::MAX_JOBS).map(|_| {
        let (release, wait) = std::sync::mpsc::channel(); let started = started.clone();
        s.jobs.submit(Priority::Visible, move || { started.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); }); release
    }).collect();
    for _ in &releases { entered.recv_timeout(Duration::from_secs(5)).unwrap(); }
    let (sentinel, sentinel_rx) = std::sync::mpsc::channel();
    let count = s.analysis.materializations.clone();
    s.jobs.submit(Priority::Background, move || { sentinel.send(count.load(Ordering::Acquire)).unwrap(); });
    // The split requests syntax and LSP readiness in the background while all workers are gated.
    s.dispatch(Command::Split(Split::Columns));
    s.dispatch(Command::Select(Selection { anchor: 3, head: 3 }));
    s.dispatch(operation);
    releases[0].send(()).unwrap();
    let until = std::time::Instant::now() + Duration::from_secs(5);
    let before_sentinel = loop {
        if let Ok(count) = sentinel_rx.try_recv() { break count; }
        assert!(std::time::Instant::now() < until, "promoted dependency and consumer did not release the executor reservation");
        while let Ok(completion) = s.completions().try_recv() { s.dispatch(completion); }
        std::thread::yield_now();
    };
    for release in releases.iter().skip(1) { release.send(()).unwrap(); }
    if finding { settle(&mut s, |s| s.state().active().unwrap().find().is_some_and(|f| !f.pending)); }
    assert_eq!(before_sentinel, 1, "new visible demand must promote an already queued background materializer");
    if finding { assert_eq!(s.state().active().unwrap().find().unwrap().matches.len(), 1); }
    }
}

#[cfg(unix)]
fn fake_semantic_session(dir: &Path) -> (Session, PathBuf) { fake_semantic_session_with_env(dir, &[]) }

#[cfg(unix)]
fn fake_semantic_session_with_env(dir: &Path, env: &[(&str, &str)]) -> (Session, PathBuf) {
    let profile = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_owned();
    let mut build = std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    build.args(["build", "-q", "-p", "reader-core", "--example", "fake_lsp"]);
    if profile.ends_with("release") { build.arg("--release"); }
    assert!(build.status().unwrap().success());
    let log = dir.join("fake.log");
    let script = dir.join("fake-server");
    let mut sh = format!("#!/bin/sh\nexport FAKE_LSP_LOG='{}'\n", log.display());
    for (key, value) in env { sh.push_str(&format!("export {key}='{}'\n", value.replace('\'', "'\\''"))); }
    sh.push_str(&format!("exec '{}' \"$@\"\n", profile.join("examples/fake_lsp").display()));
    std::fs::write(&script, sh).unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let mut s = session(dir);
    let providers = dir.join("config/lenz-reader/providers.json");
    let mut settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&providers).unwrap()).unwrap();
    settings["providers"]["rust-analyzer"]["executable"] = serde_json::json!(script);
    std::fs::write(providers, settings.to_string()).unwrap();
    s.state.semantics.reload_runtime("rust-analyzer");
    (s, log)
}


#[cfg(unix)]
fn desired_matches_current(s: &Session, path: &Path) -> bool {
    s.state().current_source().is_some_and(|source| source.source() == path
        && s.state.semantics.desired().get(path).is_some_and(|doc| doc.version() == source.key().version))
}

#[cfg(unix)]
fn desired_text_probe(s: &Session, path: &Path) -> (std::sync::Weak<str>, usize) {
    let desired = s.state.semantics.desired();
    let doc = desired.get(path).unwrap();
    assert_eq!(doc.version(), s.state().current_source().unwrap().key().version);
    // Outgoing::Text and TextFlight retain this same Arc; drop the cloned map on return.
    (Arc::downgrade(doc.text()), doc.text().len())
}

#[cfg(unix)]
fn release_historical_protocol_reader_owners(s: &mut Session) {
    // Current text stays usable; historical source/cache/analysis owners must be gone.
    s.cache = InactiveCache::new(0);
    s.release_current();
    s.analysis.close();
    s.analysis.retain_sources(|_| false);
    // Occupying all existing workers behind earlier work is a drain barrier, not another pool.
    let (entered, starts) = std::sync::mpsc::channel();
    let releases: Vec<_> = (0..crate::jobs::MAX_JOBS).map(|_| {
        let (release, wait) = std::sync::mpsc::channel(); let entered = entered.clone();
        s.jobs.submit(Priority::Background, move || { entered.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); });
        release
    }).collect();
    for _ in &releases { starts.recv_timeout(Duration::from_secs(5)).expect("old CPU work drained"); }
    for release in releases { release.send(()).unwrap(); }
    while let Ok(completion) = s.completions().try_recv() { s.dispatch(completion); }
    s.dispatch(Completion::AnalysisWake);
    assert!(s.cache.is_empty());
    assert!(s.state.sources.buffers.values().filter_map(|entry| entry.snapshot.as_ref()).all(|source| {
        s.state().current_source().is_some_and(|current| source.key() == current.key())
    }), "historical frontend source owners were released");
}

#[cfg(unix)]
fn protocol_messages(log: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(log).unwrap_or_default().lines().map(|line| serde_json::from_str(line).unwrap()).collect()
}

#[cfg(unix)]
#[test]
fn a_blocked_writer_keeps_only_bounded_text_versions_and_sends_current_text_before_asking() {
    let d = tempfile::tempdir().unwrap(); let root = d.path().join("project");
    std::fs::create_dir_all(root.join("src")).unwrap();
    write(&root, "Cargo.toml", b"[package]\nname = \"writer\"\n");
    let release = d.path().join("release"); let release_text = release.to_string_lossy();
    let source = |version| format!("fn keep() {{}}\n/*version-{version:02} {}*/\n", "x".repeat(256 * 1024));
    let path = write(&root, "src/main.rs", source(0).as_bytes());
    let (mut s, log) = fake_semantic_session_with_env(d.path(), &[("FAKE_LSP_PAUSE_INPUT", &release_text), ("FAKE_LSP_NO_DECOR", "1")]);
    s.dispatch(Command::Open(path.clone()));
    settle(&mut s, |s| desired_matches_current(s, &path) && s.state().provider().is_some_and(|v| matches!(v.status, semantics::Status::Ready | semantics::Status::Indexing(_))));
    settle(&mut s, |_| release.with_extension("paused").exists());
    let mut texts = Vec::new();
    for version in 1..=32 {
        settle(&mut s, |s| desired_matches_current(s, &path));
        texts.push(desired_text_probe(&s, &path).0);
        std::fs::write(&path, source(version)).unwrap(); s.dispatch(Command::Reload);
        let expected = format!("version-{version:02}");
        settle(&mut s, |s| s.state().loading().is_none() && s.state().current_source().is_some_and(|current| source_text(current).contains(&expected)));
    }
    settle(&mut s, |s| desired_matches_current(s, &path) && s.state().highlights().is_some());
    release_historical_protocol_reader_owners(&mut s);
    assert!(texts[0].upgrade().is_some(), "positive control: first blocked historical payload survives reader-owner release");
    let retained = texts.iter().filter(|text| text.upgrade().is_some()).count();
    assert!(retained <= 3, "the blocked protocol writer retained {retained} superseded text versions");
    s.dispatch(Command::Select(Selection { anchor: 3, head: 3 })); s.dispatch(Command::Definition { peek: false });
    assert!(s.state().asking().is_some());
    std::fs::write(&release, b"").unwrap(); settle(&mut s, |s| s.state().asking().is_none());
    settle(&mut s, |_| texts.iter().all(|text| text.upgrade().is_none()));
    let messages = protocol_messages(&log);
    let asked = messages.iter().position(|m| m["method"] == "textDocument/definition").unwrap();
    let current_text = source(32);
    let changed = messages.iter().position(|m| m["method"] == "textDocument/didChange" && m["params"]["contentChanges"][0]["text"].as_str() == Some(&current_text)).unwrap();
    assert!(changed < asked, "a request cannot overtake its exact current text");
}

#[cfg(unix)]
#[test]
fn decoded_expansion_and_closed_uri_churn_obey_the_global_writer_byte_budget() {
    let d = tempfile::tempdir().unwrap(); let root = d.path().join("project");
    std::fs::create_dir_all(root.join("src")).unwrap();
    write(&root, "Cargo.toml", b"[package]\nname = \"writer\"\n");
    let start = write(&root, "src/start.rs", b"fn start() {}\n");
    let release = d.path().join("release"); let release_text = release.to_string_lossy();
    let (mut s, log) = fake_semantic_session_with_env(d.path(), &[("FAKE_LSP_PAUSE_INPUT", &release_text), ("FAKE_LSP_NO_DECOR", "1")]);
    s.dispatch(Command::Open(start.clone())); settle(&mut s, |s| desired_matches_current(s, &start));
    settle(&mut s, |_| release.with_extension("paused").exists()); s.dispatch(Command::Close);
    // Raw UTF16 bytes remain eligible while CJK expands to three UTF8 bytes per character.
    let mut bytes = vec![0xff, 0xfe];
    bytes.extend("/*".encode_utf16().chain(std::iter::repeat_n(0x4e00, 2_500_000)).chain("*/\n".encode_utf16()).flat_map(u16::to_le_bytes));
    let mut old = Vec::new();
    for i in 0..4 {
        let path = write(&root, &format!("src/expanded-{i}.rs"), &bytes);
        s.dispatch(Command::Open(path.clone())); settle(&mut s, |s| desired_matches_current(s, &path));
        let current = s.state().current_source().unwrap();
        assert!(!current.is_large(), "existing raw-byte eligibility must be preserved");
        assert!(current.len_bytes() > 7 << 20, "fixture must demonstrate legacy decoding expansion: {}", current.encoding());
        old.push(desired_text_probe(&s, &path)); s.dispatch(Command::Close);
    }
    // Replace each cached source before measuring the transport's historical ownership.
    for i in 0..4 {
        let path = write(&root, &format!("src/expanded-{i}.rs"), b"fn replacement() {}\n");
        s.dispatch(Command::Open(path.clone()));
        settle(&mut s, |s| desired_matches_current(s, &path) && s.state().current_source().is_some_and(|source| source_text(source).starts_with("fn replacement")));
        s.dispatch(Command::Close);
    }
    s.dispatch(Command::Open(start.clone()));
    settle(&mut s, |s| desired_matches_current(s, &start) && s.state().highlights().is_some());
    release_historical_protocol_reader_owners(&mut s);
    assert!(old[0].0.upgrade().is_some(), "positive control: first expanded blocked payload survives reader-owner release");
    let retained: usize = old.iter().filter(|(text, _)| text.upgrade().is_some()).map(|(_, bytes)| bytes).sum();
    assert!(retained <= 15 << 20, "closed historical UTF8 payload retained {retained} bytes");
    s.dispatch(Command::Select(Selection { anchor: 3, head: 3 })); s.dispatch(Command::Definition { peek: false });
    assert!(s.state().asking().is_some()); std::fs::write(&release, b"").unwrap();
    settle(&mut s, |s| s.state().asking().is_none());
    settle(&mut s, |_| old.iter().all(|(text, _)| text.upgrade().is_none()));
    let messages = protocol_messages(&log);
    assert!(messages.iter().filter(|m| m["method"] == "textDocument/didOpen" && m["params"]["textDocument"]["uri"].as_str().is_some_and(|uri| uri.ends_with("start.rs"))).count() >= 2);
    assert!(s.state().provider().is_some_and(|v| matches!(v.status, semantics::Status::Ready | semantics::Status::Indexing(_))), "backpressure defers without failing the provider");
}

#[test]
fn semantic_sync_does_not_materialize_plain_drafts_without_a_provider() {
    for (name, draft) in [("plain.txt", "plain draft".repeat(10_000)), ("large.rs", "x".repeat(crate::document::LARGE_BYTES + 1))] {
    let d = tempfile::tempdir().unwrap(); let file = write(d.path(), name, b"saved\n"); let mut s = session(d.path());
    open(&mut s, Command::Open(file.clone()), name);
    let (started, entered) = std::sync::mpsc::channel();
    let releases: Vec<_> = (0..crate::jobs::MAX_JOBS).map(|_| { let (release, wait) = std::sync::mpsc::channel(); let started = started.clone();
        s.jobs.submit(Priority::Background, move || { started.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); }); release }).collect();
    for _ in &releases { entered.recv_timeout(Duration::from_secs(5)).unwrap(); }
    while let Ok(c) = s.completions().try_recv() { s.dispatch(c); }
    let tab = s.state.active.unwrap(); let old = s.state().view(tab).unwrap().snapshot().clone();
    let current = Arc::new(old.apply_edits(old.key().version + 1, &[crate::text::TextEdit { range: 0..old.len_bytes(), text: Arc::from(draft) }]).unwrap());
    s.state.sources.buffers.get_mut(&old.key().buffer).unwrap().snapshot = Some(current.clone()); s.tab_mut(tab).unwrap().new_content();
    s.dispatch(Command::Scroll(0));
    for release in releases { release.send(()).unwrap(); }
    let until = std::time::Instant::now() + Duration::from_millis(200);
    while std::time::Instant::now() < until {
        if let Ok(c) = s.completions().try_recv() { s.dispatch(c); } else { std::thread::sleep(Duration::from_millis(1)); }
    }
    assert_eq!(s.analysis.materializations.load(Ordering::Acquire), 0, "LSP demand without a provider must not copy an otherwise idle plain draft");
    assert!(Arc::ptr_eq(s.state().view(tab).unwrap().snapshot(), &current));
    assert_eq!(std::fs::read(file).unwrap(), b"saved\n");
    }
}

#[cfg(unix)]
#[test]
fn background_semantic_readiness_starts_in_the_dispatch_that_registers_it() {
    let d = tempfile::tempdir().unwrap(); write(d.path(), "Cargo.toml", b"[package]\nname = \"background\"\n");
    let file = write(d.path(), "background.rs", b"fn saved() {}\n"); let (mut s, _) = fake_semantic_session(d.path());
    open(&mut s, Command::Open(file.clone()), "background.rs");
    settle(&mut s, |s| active_view(s).tokens().is_some() && active_view(s).inlay_hints().is_some() && s.state().highlights().is_some());
    let (started, entered) = std::sync::mpsc::channel();
    let releases: Vec<_> = (0..crate::jobs::MAX_JOBS).map(|_| { let (release, wait) = std::sync::mpsc::channel(); let started = started.clone();
        s.jobs.submit(Priority::Background, move || { started.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); }); release }).collect();
    for _ in &releases { entered.recv_timeout(Duration::from_secs(5)).unwrap(); }
    while let Ok(c) = s.completions().try_recv() { s.dispatch(c); }
    let tab = s.state.active.unwrap(); let old = s.state().view(tab).unwrap().snapshot().clone();
    let current = Arc::new(old.apply_edits(old.key().version + 1, &[crate::text::TextEdit { range: 0..old.len_bytes(), text: Arc::from("fn current_background() {}\n") }]).unwrap());
    s.state.sources.buffers.get_mut(&old.key().buffer).unwrap().snapshot = Some(current.clone()); s.tab_mut(tab).unwrap().new_content();
    s.dispatch(Command::Scroll(0)); // only the footer requests semantic readiness
    for release in releases { release.send(()).unwrap(); }
    let until = std::time::Instant::now() + Duration::from_millis(500);
    while s.state.semantics.desired().get(&file).is_none_or(|d| d.version() != current.key().version) && std::time::Instant::now() < until {
        if let Ok(c) = s.completions().try_recv() {
            // Suppress unrelated provider/watch events: only the materializer's own finalization
            // may drive readiness, so an incidental message cannot conceal a missing submission.
            if matches!(c, Completion::AnalysisWake) { s.dispatch(c); }
        } else { std::thread::sleep(Duration::from_millis(1)); }
    }
    assert!(s.state.semantics.desired().get(&file).is_some_and(|d| d.version() == current.key().version), "new footer demand must not depend on an unrelated input or the five-second tick");
    assert_eq!(std::fs::read(file).unwrap(), b"fn saved() {}\n");
}

#[cfg(unix)]
#[test]
fn already_resolved_semantic_targets_do_not_navigate_after_the_origin_changes() {
    let d = tempfile::tempdir().unwrap(); write(d.path(), "Cargo.toml", b"[package]\nname = \"targets\"\n");
    let file = write(d.path(), "main.rs", b"fn main() { target(); }\n"); write(d.path(), "lib.rs", b"pub fn target() {}\n");
    let (mut s, _) = fake_semantic_session(d.path()); open(&mut s, Command::Open(file.clone()), "main.rs");
    settle(&mut s, |s| s.state().provider().is_some_and(|v| v.status == semantics::Status::Ready));
    s.dispatch(Command::Select(Selection { anchor: 12, head: 12 })); s.dispatch(Command::Definition { peek: false });
    let rx = s.completions();
    let old = loop { let c = rx.recv_blocking_timeout().unwrap();
        if matches!(&c, Completion::Targets { .. }) || matches!(&c, Completion::Analyzed { result, .. } if matches!(result.as_ref(), Completion::Targets { .. })) { break c; }
        s.dispatch(c);
    };
    std::fs::write(&file, b"fn main() { changed(); }\n").unwrap(); s.dispatch(Command::Reload);
    settle(&mut s, |s| s.state().loading().is_none() && s.state().current_source().is_some_and(|d| source_text(d).contains("changed")));
    let current = s.state().view(s.state.active.unwrap()).unwrap().snapshot().clone();
    s.dispatch(old);
    assert_eq!(s.state().active().unwrap().source(), file.as_path(), "the request's origin must be revalidated after target resolution");
    assert!(Arc::ptr_eq(s.state().view(s.state.active.unwrap()).unwrap().snapshot(), &current));
}

#[cfg(unix)]
#[test]
fn replacing_semantic_target_replies_bounds_actual_document_retaining_jobs() {
    let d = tempfile::tempdir().unwrap(); write(d.path(), "Cargo.toml", b"[package]\nname = \"targets\"\n");
    let file = write(d.path(), "main.rs", b"fn main() { target(); }\n"); let target = write(d.path(), "lib.rs", b"pub fn target() {}\n");
    let (mut s, _) = fake_semantic_session(d.path()); open(&mut s, Command::Open(file), "main.rs");
    settle(&mut s, |s| s.state().provider().is_some_and(|v| v.status == semantics::Status::Ready) && s.state().highlights().is_some());
    let source_document = analysis_document(&mut s);
    let server = s.state.semantics.session_of(&source_document).unwrap();
    let (started, entered) = std::sync::mpsc::channel();
    let releases: Vec<_> = (0..crate::jobs::MAX_JOBS).map(|_| { let (release, wait) = std::sync::mpsc::channel(); let started = started.clone();
        s.jobs.submit(Priority::Background, move || { started.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); }); release }).collect();
    for _ in &releases { entered.recv_timeout(Duration::from_secs(5)).unwrap(); }
    for _ in 0..129 {
        s.dispatch(Command::Select(Selection { anchor: 12, head: 12 })); s.dispatch(Command::Definition { peek: false });
        let request = s.state.asking.as_ref().unwrap().request;
        s.semantic_reply(semantics::Reply { lane: semantics::Lane::Foreground, request, session: server, encoding: s.state.semantics.encoding(server), provider: "rust-analyzer", indexing: false, sources: Default::default(),
            result: Ok(serde_json::json!([{ "uri": semantics::lsp::file_uri(&target), "range": { "start": {"line": 0,"character": 7}, "end": {"line":0,"character":13} } }])) });
        s.analysis.pump(&s.jobs, &s.tx, &s.project_generation);
    }
    let queued = s.jobs.queued();
    for release in releases { release.send(()).unwrap(); }
    settle(&mut s, |s| s.state().active().is_some_and(|t| t.source() == target));
    assert!(queued <= 2 * crate::jobs::MAX_JOBS, "superseded semantic target replies retained {queued} document-map closures");
}

#[cfg(unix)]
#[test]
fn semantic_capacity_and_worker_failure_finish_the_current_spinner() {
    use crate::analysis::{ResultFence, ResultSlot, ResultStage};
    let d = tempfile::tempdir().unwrap(); write(d.path(), "Cargo.toml", b"[package]\nname = \"targets\"\n");
    let file = write(d.path(), "main.rs", b"fn main() { target(); }\n");
    let (mut s, _) = fake_semantic_session(d.path()); open(&mut s, Command::Open(file), "main.rs");
    settle(&mut s, |s| s.state().provider().is_some_and(|v| v.status == semantics::Status::Ready));
    let source_document = analysis_document(&mut s);
    let server = s.state.semantics.session_of(&source_document).unwrap();
    s.dispatch(Command::Select(Selection { anchor: 12, head: 12 })); s.dispatch(Command::Definition { peek: false });
    let asking = s.state.asking.as_ref().unwrap();
    let fence = ResultFence { slot: ResultSlot::Semantic { server, lane: semantics::Lane::Foreground }, project: asking.project, request: asking.request, stage: ResultStage::Targets, result_set: None, node: None, source: asking.source.clone() };
    s.analysis.request_result(fence, Priority::Visible, || |_| panic!("semantic resolver failed")).unwrap();
    s.analysis.pump(&s.jobs, &s.tx, &s.project_generation);
    settle(&mut s, |s| s.state.asking.is_none());
    assert!(s.state.messages.iter().any(|m| m.contains("cancelled or could not complete")));
    s.dispatch(Command::Definition { peek: false });
    let request = s.state.asking.as_ref().unwrap().request;
    let project = s.project_generation.load(Ordering::Acquire);
    for n in 1000..1065 {
        let fence = ResultFence { slot: ResultSlot::Semantic { server: n, lane: semantics::Lane::Foreground }, project, request: 1, stage: ResultStage::Targets, result_set: None, node: None, source: None };
        if s.analysis.request_result(fence, Priority::Visible, || |_| {}).is_err() { break; }
    }
    let effect = s.semantic_reply(semantics::Reply { lane: semantics::Lane::Foreground, request, session: server, encoding: s.state.semantics.encoding(server), provider: "rust-analyzer", indexing: false, sources: Default::default(), result: Ok(serde_json::json!([])) });
    assert!(effect.repaint); assert!(s.state.asking.is_none(), "capacity pressure must end this server's answer visibly");
    assert!(s.state.messages.iter().any(|m| m.contains("capacity is full")));
}

#[cfg(unix)]
#[test]
fn replacing_hover_replies_keeps_provider_markup_out_of_unbounded_worker_queues() {
    let d = tempfile::tempdir().unwrap(); write(d.path(), "Cargo.toml", b"[package]\nname = \"hover\"\n");
    let file = write(d.path(), "hover.rs", b"fn target() {}\n"); let (mut s, _) = fake_semantic_session(d.path());
    open(&mut s, Command::Open(file), "hover.rs");
    settle(&mut s, |s| s.state().provider().is_some_and(|v| v.status == semantics::Status::Ready) && s.state().highlights().is_some() && active_view(s).inlay_hints().is_some() && active_view(s).tokens().is_some());
    s.dispatch(Command::Select(Selection { anchor: 3, head: 3 }));
    let (started, entered) = std::sync::mpsc::channel();
    let releases: Vec<_> = (0..crate::jobs::MAX_JOBS).map(|_| { let (release, wait) = std::sync::mpsc::channel(); let started = started.clone();
        s.jobs.submit(Priority::Background, move || { started.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); }); release }).collect();
    for _ in &releases { entered.recv_timeout(Duration::from_secs(5)).unwrap(); }
    let rx = s.completions();
    for _ in 0..129 {
        s.dispatch(Command::Documentation);
        loop {
            let c = rx.recv_blocking_timeout().unwrap();
            let replied = matches!(&c, Completion::Semantic(semantics::Event::Message { message, .. }) if message["result"]["contents"]["kind"] == "markdown");
            s.dispatch(c); if replied { break; }
        }
    }
    let queued = s.jobs.queued();
    for release in releases { release.send(()).unwrap(); }
    settle(&mut s, |s| s.state().documentation().is_some());
    assert!(queued <= 2 * crate::jobs::MAX_JOBS, "superseded hover markup retained {queued} worker closures");
}

#[cfg(unix)]
#[test]
fn documentation_from_before_a_version_preserving_rename_is_not_published() {
    let d = tempfile::tempdir().unwrap(); write(d.path(), "Cargo.toml", b"[package]\nname = \"rename\"\n");
    let file = write(d.path(), "old.rs", b"fn target() {}\n"); let (mut s, _) = fake_semantic_session(d.path());
    open(&mut s, Command::Open(file.clone()), "old.rs");
    settle(&mut s, |s| s.state().provider().is_some_and(|v| v.status == semantics::Status::Ready));
    s.dispatch(Command::Select(Selection { anchor: 3, head: 3 })); s.dispatch(Command::Documentation);
    let rx = s.completions();
    let old = loop { let c = rx.recv_blocking_timeout().unwrap(); if matches!(&c, Completion::DocumentationReady { .. }) || matches!(&c, Completion::Analyzed { result, .. } if matches!(result.as_ref(), Completion::DocumentationReady { .. })) { break c; } s.dispatch(c); };
    let tab = s.state.active.unwrap(); let version = s.state().view(tab).unwrap().snapshot().key().version;
    let token = s.state().active().unwrap().refresh; let renamed = d.path().join("renamed.rs");
    std::fs::rename(&file, &renamed).unwrap();
    s.dispatch(Completion::Moved { tab, token, to: renamed.clone(), stamp: Stamp::of(&renamed), canonical: std::fs::canonicalize(&renamed).ok() });
    assert_eq!(s.state().view(tab).unwrap().snapshot().key().version, version);
    s.dispatch(old);
    assert!(s.state().documentation().is_none(), "source metadata must match after rendering, not only tab and version");
    s.dispatch(Command::Documentation); settle(&mut s, |s| s.state().documentation().is_some());
    let token = s.state().active().unwrap().refresh; let again = d.path().join("again.rs"); std::fs::rename(&renamed, &again).unwrap();
    s.dispatch(Completion::Moved { tab, token, to: again.clone(), stamp: Stamp::of(&again), canonical: std::fs::canonicalize(&again).ok() });
    assert!(s.state().documentation().is_none(), "already displayed documentation also belongs to its source metadata");
}

#[cfg(unix)]
#[test]
fn decoded_decorations_from_a_restarted_provider_cannot_complete_a_new_request() {
    let d = tempfile::tempdir().unwrap(); write(d.path(), "Cargo.toml", b"[package]\nname = \"restart\"\n");
    let file = write(d.path(), "restart.rs", b"fn target() { let value = 1; }\n");
    let (mut s, log) = fake_semantic_session(d.path());
    open(&mut s, Command::Open(file.clone()), "restart.rs");
    let rx = s.completions();
    let old = loop { let c = rx.recv_blocking_timeout().unwrap();
        if matches!(&c, Completion::Analyzed { result, .. } if matches!(result.as_ref(), Completion::Decorated { data, .. } if data.kind() == Decor::Tokens)) { break c; }
        s.dispatch(c);
    };
    let source_document = analysis_document(&mut s);
    let old_session = s.state.semantics.session_of(&source_document).unwrap();
    let old_request = match &old { Completion::Analyzed { result, .. } => match result.as_ref() { Completion::Decorated { request, .. } => *request, _ => unreachable!() }, _ => unreachable!() };
    assert!(active_view(&s).tokens().is_none());
    s.dispatch(Command::RestartServer);
    settle(&mut s, |s| !s.state.semantics.has_session(old_session) && s.state.semantics.latest(&file, Decor::Tokens) == Some(old_request));
    s.dispatch(old);
    let stale_installed = active_view(&s).tokens().is_some();
    settle(&mut s, |s| active_view(s).tokens().is_some());
    assert!(!stale_installed, "a restarted provider can reuse its request numbers; the server session must also match");
    let other = write(d.path(), "other.rs", b"fn other() {}\n");
    s.dispatch(Command::Split(Split::Columns)); open(&mut s, Command::Open(other), "other.rs"); s.dispatch(Command::FocusPane(0));
    settle(&mut s, |s| s.state().shown(0).unwrap().highlights.is_some() && s.state().shown(1).unwrap().highlights.is_some());
    let tab = s.state.active.unwrap(); let old = s.state().view(tab).unwrap().snapshot().clone();
    let current = Arc::new(old.apply_edits(old.key().version + 1, &[crate::text::TextEdit { range: 0..old.len_bytes(), text: Arc::from("fn target() { let changed = 2; }\n") }]).unwrap());
    s.state.sources.buffers.get_mut(&old.key().buffer).unwrap().snapshot = Some(current.clone()); s.tab_mut(tab).unwrap().new_content();
    let (started, entered) = std::sync::mpsc::channel();
    let releases: Vec<_> = (0..crate::jobs::MAX_JOBS).map(|_| { let (release, wait) = std::sync::mpsc::channel(); let started = started.clone();
        s.jobs.submit(Priority::Visible, move || { started.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); }); release }).collect();
    for _ in &releases { entered.recv_timeout(Duration::from_secs(5)).unwrap(); }
    s.dispatch(Command::Select(Selection { anchor: 3, head: 3 })); s.dispatch(Command::Documentation);
    s.dispatch(Command::RestartServer);
    // Resolve the known disposable Cargo root at the filesystem boundary while CPU work is gated,
    // allowing the replacement real fake-provider process to initialize before text readiness.
    let provider = semantics::provider::for_language(highlight::Language::Rust).unwrap().0;
    s.dispatch(Completion::Semantic(semantics::Event::Resolved { provider, dir: d.path().to_owned(), root: Some(d.path().to_owned()), serve: provider }));
    settle(&mut s, |s| s.state().provider().is_some_and(|p| p.status == semantics::Status::Ready));
    for release in releases { release.send(()).unwrap(); }
    settle(&mut s, |s| s.state.semantics.desired().get(&file).is_some_and(|d| d.version() == current.key().version));
    // A barrier request drains the same FIFO after the new source; cancelled documentation must not
    // be revived in the replacement server once its materialization finally becomes ready.
    s.dispatch(Command::FileSymbols); settle(&mut s, |s| s.state().asking().is_none());
    let frames: Vec<serde_json::Value> = std::fs::read_to_string(log).unwrap().lines().filter_map(|line| serde_json::from_str(line).ok()).collect();
    assert!(!frames.iter().any(|m| m["method"] == "textDocument/hover"), "provider restart cancels materialization-waiting requests");
}

#[cfg(unix)]
#[test]
fn semantic_requests_wait_for_the_current_admitted_snapshot_and_preserve_saved_text() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "Cargo.toml", b"[package]\nname = \"draft\"\n");
    let saved = b"fn saved() {}\n";
    let file = write(d.path(), "draft.rs", saved);
    let (mut s, log) = fake_semantic_session(d.path());
    open(&mut s, Command::Open(file.clone()), "draft.rs");
    settle(&mut s, |s| s.state().provider().is_some_and(|v| v.status == semantics::Status::Ready));
    let tab = s.state.active.unwrap();
    let old = s.state().view(tab).unwrap().snapshot().clone();
    let text = "fn current_draft() { let unicode = \"é𝄞\"; }\n";
    let current = Arc::new(old.apply_edits(old.key().version + 1,
        &[crate::text::TextEdit { range: 0..old.len_bytes(), text: Arc::from(text) }]).unwrap());
    s.state.sources.buffers.get_mut(&current.key().buffer).unwrap().snapshot = Some(current.clone());
    s.active_mut().unwrap().new_content();
    s.dispatch(Command::Select(Selection { anchor: 3, head: 3 }));
    s.dispatch(Command::Documentation);
    s.dispatch(Command::HighlightAt);
    settle(&mut s, |s| s.state().documentation().is_some() && s.state().occurrences().is_some());
    assert_eq!(s.state().documentation().unwrap().at, 3..16, "the name range comes from current text");
    assert_eq!(s.state().documentation().unwrap().version, current.key().version);
    assert_eq!(s.state().occurrences().unwrap().ranges[0].0, 3..16);
    s.dispatch(Command::Definition { peek: false });
    settle(&mut s, |s| s.state().asking().is_none());
    let frames: Vec<serde_json::Value> = std::fs::read_to_string(log).unwrap().lines().filter_map(|line| serde_json::from_str(line).ok()).collect();
    let request = frames.iter().position(|m| m["method"] == "textDocument/definition").unwrap();
    let change = frames.iter().position(|m| m["method"] == "textDocument/didChange" && m["params"]["contentChanges"][0]["text"] == text);
    assert!(change.is_some_and(|change| change < request), "exact draft text must precede its request: {frames:?}");
    assert_eq!(s.analysis.materializations.load(Ordering::Relaxed), 1);
    assert!(Arc::ptr_eq(s.state().view(tab).unwrap().snapshot(), &current));
    assert_eq!(std::fs::read(file).unwrap(), saved);
}

#[cfg(unix)]
#[test]
fn semantic_navigation_refuses_target_coordinates_from_before_another_pane_edit() {
    #[derive(Clone, Copy)]
    enum Change { BeforeReply, AfterResolutionUndo, Unrelated }
    for change in [Change::BeforeReply, Change::AfterResolutionUndo, Change::Unrelated] {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "Cargo.toml", b"[package]\nname = \"target-context\"\n");
        let a_text = "fn main() { helper(); }\n";
        let b_text = "pub fn helper() {}\n";
        let a = write(d.path(), "main.rs", a_text.as_bytes());
        let b = write(d.path(), "lib.rs", b_text.as_bytes());
        let (mut s, _) = fake_semantic_session_with_env(d.path(), &[("FAKE_LSP_NO_DECOR", "1")]);
        open(&mut s, Command::OpenRetained(a.clone()), "main.rs");
        settle(&mut s, |s| s.state().provider().is_some_and(|v| v.status == semantics::Status::Ready));
        s.dispatch(Command::Split(Split::Columns));
        open(&mut s, Command::OpenRetained(b.clone()), "lib.rs");
        let target = s.state().active().unwrap().id();
        let expected = s.state().view(target).unwrap().snapshot().key();
        s.dispatch(Command::Edit { tab: target, expected, edits: vec![crate::text::TextEdit { range: 0..0, text: "// é😀 target draft\n".into() }], kind: crate::text::EditKind::Paste });
        let target_document = analysis_document(&mut s);
        settle(&mut s, |s| s.state.semantics.session_of(&target_document).is_some());
        let server = s.state.semantics.session_of(&target_document).unwrap();
        drop(target_document);
        settle(&mut s, |s| s.state.semantics.sent(server).get(&b).is_some_and(|doc| doc.version() == s.state().view(target).unwrap().snapshot().key().version));
        let unrelated = d.path().join("other.rs");
        let edited = if matches!(change, Change::Unrelated) {
            std::fs::write(&unrelated, "fn another() {}\n").unwrap();
            open(&mut s, Command::OpenRetained(unrelated.clone()), "other.rs");
            s.state().active().unwrap().id()
        } else { target };
        s.dispatch(Command::FocusPane(0));
        let at = a_text.find("helper").unwrap();
        s.dispatch(Command::Select(Selection { anchor: at, head: at }));
        let origin = s.state().current_source().unwrap().clone();
        let origin_selection = s.state().selection();
        s.dispatch(Command::Definition { peek: false });
        let rx = s.completions();
        let held = loop {
            let completion = rx.recv_blocking_timeout().unwrap();
            let target_reply = if matches!(change, Change::AfterResolutionUndo) {
                matches!(&completion, Completion::Analyzed { result, .. } if matches!(result.as_ref(), Completion::Targets { .. }))
            } else {
                matches!(&completion, Completion::Semantic(semantics::Event::Message { message, .. })
                    if message["result"]["uri"].as_str().is_some_and(|uri| uri.ends_with("lib.rs"))
                        || message["result"].as_array().is_some_and(|items| items.iter().any(|item| item["uri"].as_str().is_some_and(|uri| uri.ends_with("lib.rs")))))
            };
            if target_reply { break completion; }
            s.dispatch(completion);
        };
        let expected = s.state().view(edited).unwrap().snapshot().key();
        s.dispatch(Command::Edit { tab: edited, expected, edits: vec![crate::text::TextEdit { range: 0..0, text: "// another shift\n".into() }], kind: crate::text::EditKind::Paste });
        if matches!(change, Change::AfterResolutionUndo) { s.dispatch(Command::Undo { tab: edited }); }
        let changed = s.state().view(edited).unwrap().snapshot().clone();
        settle(&mut s, |s| s.state.semantics.sent(server).get(changed.source()).is_some_and(|doc| doc.version() == changed.key().version));
        s.dispatch(held);
        settle(&mut s, |s| s.state().asking().is_none());
        if matches!(change, Change::Unrelated) {
            assert_eq!(s.state().current_source().unwrap().source(), b);
            assert_eq!(s.dispatch(Command::Copy).copy.as_deref(), Some("helper"), "an unrelated source edit must not invalidate a valid target");
        } else {
            assert!(Arc::ptr_eq(s.state().current_source().unwrap(), &origin), "a target edit cannot make the old reply move the origin pane");
            assert_eq!(s.state().selection(), origin_selection);
            assert!(s.state().notices().iter().any(|notice| notice.contains("target") && notice.contains("changed")), "{:?}", s.state().notices());
        }
        assert!(Arc::ptr_eq(s.state().view(edited).unwrap().snapshot(), &changed));
        assert_eq!(std::fs::read(a).unwrap(), a_text.as_bytes());
        assert_eq!(std::fs::read(b).unwrap(), b_text.as_bytes());
    }
}

#[cfg(unix)]
#[test]
fn semantic_results_keep_the_captured_target_draft_and_refuse_later_activation() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "Cargo.toml", b"[package]\nname = \"peek-target\"\n");
    let a_text = "fn main() { helper(); }\n";
    let b_text = "pub fn helper() {}\n";
    let a = write(d.path(), "main.rs", a_text.as_bytes());
    let b = write(d.path(), "lib.rs", b_text.as_bytes());
    write(d.path(), "other.rs", b"pub fn helper() {}\n");
    let (mut s, _) = fake_semantic_session_with_env(d.path(), &[("FAKE_LSP_NO_DECOR", "1")]);
    open(&mut s, Command::OpenRetained(a.clone()), "main.rs");
    settle(&mut s, |s| s.state().provider().is_some_and(|v| v.status == semantics::Status::Ready));
    s.dispatch(Command::Split(Split::Columns));
    open(&mut s, Command::OpenRetained(b.clone()), "lib.rs");
    let target = s.state().active().unwrap().id();
    let expected = s.state().view(target).unwrap().snapshot().key();
    s.dispatch(Command::Edit { tab: target, expected, edits: vec![crate::text::TextEdit { range: 0..0, text: "// é😀 target draft\n".into() }], kind: crate::text::EditKind::Paste });
    let captured = s.state().view(target).unwrap().snapshot().clone();
    let target_document = analysis_document(&mut s);
    settle(&mut s, |s| s.state.semantics.session_of(&target_document).is_some());
    let server = s.state.semantics.session_of(&target_document).unwrap();
    drop(target_document);
    settle(&mut s, |s| s.state.semantics.sent(server).get(&b).is_some_and(|doc| doc.version() == captured.key().version));
    s.dispatch(Command::FocusPane(0));
    let at = a_text.find("helper").unwrap();
    s.dispatch(Command::Select(Selection { anchor: at, head: at }));
    let origin = s.state().current_source().unwrap().clone();
    let selection = s.state().selection();
    s.dispatch(Command::Definition { peek: true });
    settle(&mut s, |s| s.state().results().is_some_and(|results| results.results_kind() == ResultsKind::Peek && results.preview().is_some()));
    assert_eq!(s.state().results().unwrap().hits()[0].line, 1);
    assert_eq!(s.state().results().unwrap().preview().unwrap().doc.version(), captured.key().version);
    let expected = captured.key();
    s.dispatch(Command::Edit { tab: target, expected, edits: vec![crate::text::TextEdit { range: 0..0, text: "// newer target\n".into() }], kind: crate::text::EditKind::Paste });
    let changed = s.state().view(target).unwrap().snapshot().clone();
    s.dispatch(Command::SelectResult(1));
    settle(&mut s, |s| s.state().results().unwrap().preview().is_some_and(|preview| preview.index == 1));
    s.dispatch(Command::SelectResult(0));
    settle(&mut s, |s| s.state().results().unwrap().preview().is_some_and(|preview| preview.index == 0));
    assert_eq!(s.state().results().unwrap().preview().unwrap().doc.version(), captured.key().version, "a peek retains the version named by its row");
    s.dispatch(Command::OpenResult(0));
    settle(&mut s, |s| s.state().results().unwrap().is_stale(0));
    assert!(Arc::ptr_eq(s.state().current_source().unwrap(), &origin));
    assert_eq!(s.state().selection(), selection);
    assert!(Arc::ptr_eq(s.state().view(target).unwrap().snapshot(), &changed));
    assert_eq!(std::fs::read(a).unwrap(), a_text.as_bytes());
    assert_eq!(std::fs::read(b).unwrap(), b_text.as_bytes());
}

#[cfg(unix)]
#[test]
fn outgoing_hierarchy_refuses_a_changed_item_even_when_its_call_site_is_unchanged() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "Cargo.toml", b"[package]\nname = \"hierarchy-target\"\n");
    let a_text = "fn main() {\n    helper();\n}\n";
    let b_text = "pub fn helper() {}\n";
    let a = write(d.path(), "main.rs", a_text.as_bytes());
    let b = write(d.path(), "lib.rs", b_text.as_bytes());
    let (mut s, log) = fake_semantic_session_with_env(d.path(), &[("FAKE_LSP_NO_DECOR", "1")]);
    open(&mut s, Command::OpenRetained(a.clone()), "main.rs");
    settle(&mut s, |s| s.state().provider().is_some_and(|v| v.status == semantics::Status::Ready));
    s.dispatch(Command::Split(Split::Columns)); open(&mut s, Command::OpenRetained(b.clone()), "lib.rs");
    let target = s.state().active().unwrap().id();
    s.dispatch(Command::FocusPane(0));
    s.dispatch(Command::Select(Selection { anchor: 3, head: 3 }));
    s.dispatch(Command::CallHierarchy(Direction::Callees));
    settle(&mut s, |s| s.state.results.as_ref().and_then(|r| r.hierarchy.as_ref()).is_some_and(|hierarchy| hierarchy.nodes.len() == 2));
    let hierarchy = s.state.results.as_ref().unwrap().hierarchy.as_ref().unwrap();
    assert_eq!(hierarchy.nodes[1].site.path.as_ref(), a.to_string_lossy());
    assert!(hierarchy.nodes[1].call.item["uri"].as_str().unwrap().ends_with("lib.rs"));
    let frames = || std::fs::read_to_string(&log).unwrap().lines().filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok()).filter(|frame| frame["method"] == "callHierarchy/outgoingCalls").count();
    let requests = frames();
    let expected = s.state().view(target).unwrap().snapshot().key();
    s.dispatch(Command::Edit { tab: target, expected, edits: vec![crate::text::TextEdit { range: 0..0, text: "// changed item\n".into() }], kind: crate::text::EditKind::Paste });
    let changed = s.state().view(target).unwrap().snapshot().clone();
    s.dispatch(Command::ToggleCall(1));
    settle(&mut s, |s| s.state.results.as_ref().and_then(|r| r.hierarchy.as_ref()).is_some_and(|hierarchy| !matches!(hierarchy.nodes[1].state, hierarchy::Children::Loading(_))));
    assert!(matches!(&s.state.results.as_ref().unwrap().hierarchy.as_ref().unwrap().nodes[1].state, hierarchy::Children::Failed(why) if why.contains("changed")), "a node's reusable item is fenced separately from its displayed call site");
    assert_eq!(frames(), requests, "obsolete item coordinates must not be sent to the provider");
    assert!(Arc::ptr_eq(s.state().view(target).unwrap().snapshot(), &changed));
    assert_eq!(s.state().current_source().unwrap().source(), a);
    assert_eq!(std::fs::read(a).unwrap(), a_text.as_bytes());
    assert_eq!(std::fs::read(b).unwrap(), b_text.as_bytes());
}

#[cfg(unix)]
#[test]
fn semantic_targets_resolve_canonical_provider_uris_from_the_captured_draft_alias() {
    let d = tempfile::tempdir().unwrap(); let root = d.path().join("project"); std::fs::create_dir(&root).unwrap();
    write(&root, "Cargo.toml", b"[package]\nname = \"alias-target\"\n");
    let saved = "pub fn helper() {}\n";
    let actual = write(d.path(), "external.rs", saved.as_bytes());
    let alias = root.join("lib.rs"); std::os::unix::fs::symlink(&actual, &alias).unwrap();
    let a = write(&root, "main.rs", b"fn main() { helper(); }\n");
    let (mut s, _) = fake_semantic_session_with_env(d.path(), &[("FAKE_LSP_NO_DECOR", "1"), ("FAKE_LSP_CANONICAL_URIS", "1")]);
    open(&mut s, Command::OpenRetained(alias.clone()), "lib.rs");
    settle(&mut s, |s| s.state().provider().is_some_and(|provider| provider.status == semantics::Status::Ready));
    let tab = s.state().active().unwrap().id(); let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..0, text: "// é😀 shifted\n".into() }], kind: crate::text::EditKind::Paste });
    let draft = s.state().current_source().unwrap().clone();
    let doc = analysis_document(&mut s);
    settle(&mut s, |s| s.state.semantics.session_of(&doc).is_some());
    let server = s.state.semantics.session_of(&doc).unwrap(); drop(doc);
    settle(&mut s, |s| s.state.semantics.sent(server).get(&alias).is_some_and(|doc| doc.version() == draft.key().version));
    open(&mut s, Command::OpenRetained(a.clone()), "main.rs");
    s.dispatch(Command::Select(Selection { anchor: 12, head: 12 })); s.dispatch(Command::Definition { peek: false });
    settle(&mut s, |s| s.state().asking().is_none());
    assert!(Arc::ptr_eq(s.state().current_source().unwrap(), &draft), "canonical target coordinates belong to the same admitted alias draft: {:?}", s.state().notices());
    assert_eq!(s.dispatch(Command::Copy).copy.as_deref(), Some("helper"));
    assert_eq!(std::fs::read(&actual).unwrap(), saved.as_bytes()); assert_eq!(std::fs::read(alias).unwrap(), saved.as_bytes());
    assert_eq!(std::fs::read(a).unwrap(), b"fn main() { helper(); }\n");
}

#[cfg(unix)]
#[test]
fn held_semantic_reply_blocks_a_retired_canonical_alias_after_rename() {
    let d = tempfile::tempdir().unwrap(); let root = d.path().join("project"); std::fs::create_dir(&root).unwrap();
    write(&root, "Cargo.toml", b"[package]\nname = \"retired-alias\"\n");
    let saved = "pub fn helper() {}\n";
    let actual = write(d.path(), "external.rs", saved.as_bytes());
    let alias = root.join("lib.rs"); std::os::unix::fs::symlink(&actual, &alias).unwrap();
    let a = write(&root, "main.rs", b"fn main() { helper(); }\n");
    let (mut s, _) = fake_semantic_session_with_env(d.path(), &[("FAKE_LSP_NO_DECOR", "1"), ("FAKE_LSP_CANONICAL_URIS", "1")]);
    open(&mut s, Command::OpenRetained(a.clone()), "main.rs");
    settle(&mut s, |s| s.state().provider().is_some_and(|provider| provider.status == semantics::Status::Ready));
    s.dispatch(Command::Split(Split::Columns)); open(&mut s, Command::OpenRetained(alias.clone()), "lib.rs");
    let tab = s.state().active().unwrap().id(); let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..0, text: "// é😀 shifted\n".into() }], kind: crate::text::EditKind::Paste });
    let draft = s.state().current_source().unwrap().clone(); let doc = analysis_document(&mut s);
    settle(&mut s, |s| s.state.semantics.session_of(&doc).is_some());
    let server = s.state.semantics.session_of(&doc).unwrap(); drop(doc);
    settle(&mut s, |s| s.state.semantics.sent(server).get(&alias).is_some_and(|doc| doc.version() == draft.key().version));
    s.dispatch(Command::FocusPane(0)); s.dispatch(Command::Select(Selection { anchor: 12, head: 12 }));
    let origin = s.state().current_source().unwrap().clone(); let selection = s.state().selection();
    s.dispatch(Command::Definition { peek: false });
    let held = loop {
        let completion = s.completions().recv_blocking_timeout().unwrap();
        if matches!(&completion, Completion::Semantic(semantics::Event::Message { message, .. })
            if message["result"]["uri"].as_str().is_some_and(|uri| uri.ends_with("external.rs"))) { break completion; }
        s.dispatch(completion);
    };
    // Retire the canonical name itself, not just a symlink spelling of an unchanged target.
    let moved = d.path().join("relocated.rs"); std::fs::rename(&actual, &moved).unwrap();
    std::fs::remove_file(&alias).unwrap(); let next_alias = root.join("renamed.rs");
    std::os::unix::fs::symlink(&moved, &next_alias).unwrap();
    let replacement = "// different replacement\npub fn helper() {}\n"; std::fs::write(&actual, replacement).unwrap();
    let token = s.state.tabs.iter().find(|t| t.id == tab).unwrap().refresh;
    s.dispatch(Completion::Moved { tab, token, to: next_alias.clone(), stamp: Stamp::of(&next_alias), canonical: Some(moved.clone()) });
    let renamed = s.state().view(tab).unwrap().snapshot().clone();
    assert_eq!(renamed.key(), draft.key(), "rename preserves the version but changes naming identity");
    settle(&mut s, |s| s.state.semantics.sent(server).get(&next_alias).is_some_and(|doc| Arc::ptr_eq(doc.source_meta(), renamed.source_meta())));
    s.dispatch(held); settle(&mut s, |s| s.state().asking().is_none());
    assert!(Arc::ptr_eq(s.state().current_source().unwrap(), &origin), "old coordinates must not open replacement bytes at the retired canonical name");
    assert_eq!(s.state().selection(), selection);
    assert!(s.state().notices().iter().any(|notice| notice.contains("target") && notice.contains("changed")), "{:?}", s.state().notices());
    assert!(Arc::ptr_eq(s.state().view(tab).unwrap().snapshot(), &renamed));
    assert_eq!(source_text(&renamed), source_text(&draft));
    assert_eq!(std::fs::read(&actual).unwrap(), replacement.as_bytes());
    assert_eq!(std::fs::read(&moved).unwrap(), saved.as_bytes());
    assert_eq!(std::fs::read(next_alias).unwrap(), saved.as_bytes());
    assert_eq!(std::fs::read(a).unwrap(), b"fn main() { helper(); }\n");
}

#[test]
fn large_encoded_sources_keep_their_existing_syntax_reduction() {
    let d = tempfile::tempdir().unwrap();
    let text = format!("/*{}*/", "x".repeat(3 << 20));
    let mut bytes = vec![0xff, 0xfe];
    bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
    let file = write(d.path(), "encoded.rs", &bytes);
    let mut s = session(d.path());
    open(&mut s, Command::Open(file), "encoded.rs");
    assert!(s.state().current_source().unwrap().is_large());
    let (started, entered) = std::sync::mpsc::channel();
    let releases: Vec<_> = (0..crate::jobs::MAX_JOBS).map(|_| {
        let (release, wait) = std::sync::mpsc::channel();
        let started = started.clone();
        s.jobs.submit(Priority::Background, move || { started.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); });
        release
    }).collect();
    for _ in &releases { entered.recv_timeout(Duration::from_secs(5)).unwrap(); }
    while let Ok(completion) = s.completions().try_recv() { s.dispatch(completion); }
    let parsed = s.state().highlights().is_some();
    for release in releases { release.send(()).unwrap(); }
    assert!(!parsed, "a read-path migration must not enable parsing previously reduced encoded files");
}

#[test]
fn an_old_analysis_result_cannot_complete_a_reopened_view_request() {
    let d = tempfile::tempdir().unwrap();
    let file = write(d.path(), "again.rs", b"fn current() {}\n");
    let mut s = session(d.path());
    open(&mut s, Command::Open(file.clone()), "again.rs");
    let receiver = s.completions();
    let old = loop {
        let completion = receiver.recv_blocking_timeout().unwrap();
        if matches!(&completion, Completion::Analyzed { result, .. } if matches!(result.as_ref(), Completion::Highlighted { .. })) { break completion; }
        s.dispatch(completion);
    };
    s.dispatch(Command::Close);
    let (started, entered) = std::sync::mpsc::channel();
    let releases: Vec<_> = (0..crate::jobs::MAX_JOBS).map(|_| {
        let (release, wait) = std::sync::mpsc::channel();
        let started = started.clone();
        s.jobs.submit(Priority::Visible, move || { started.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); });
        release
    }).collect();
    for _ in &releases { entered.recv_timeout(Duration::from_secs(5)).unwrap(); }
    s.dispatch(Command::Open(file.clone()));
    let tab = s.state.active.unwrap();
    s.dispatch(Completion::Checked { request: s.request, tab, path: file.clone(), stamp: Stamp::of(&file) });
    assert!(s.state().highlights().is_none());
    s.dispatch(old);
    let stale_installed = s.state().highlights().is_some();
    for release in releases { release.send(()).unwrap(); }
    settle(&mut s, |s| s.state().highlights().is_some());
    assert!(!stale_installed, "a previous admission's response cannot complete a new view request");
}

#[test]
fn rebuilding_the_index_uses_an_open_buffers_admitted_snapshot() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project"); std::fs::create_dir(&root).unwrap();
    let file = write(&root, "draft.rs", b"fn saved() {}\n");
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root));
    settle(&mut s, |s| s.state().project().is_some_and(|p| matches!(p.coverage(), crate::project::Coverage::Complete(_)))
        && s.state().index_coverage().complete() && s.state().index_coverage().indexed == 1);
    open(&mut s, Command::Open(file.clone()), "draft.rs");
    let tab = s.state.active.unwrap();
    let initial = s.state().view(tab).unwrap().snapshot().clone();
    let current = Arc::new(initial.apply_edits(initial.key().version + 1,
        &[crate::text::TextEdit { range: 0..initial.len_bytes(), text: Arc::from("fn draft_only() {}\n") }]).unwrap());
    s.state.sources.buffers.get_mut(&current.key().buffer).unwrap().snapshot = Some(current.clone());
    s.active_mut().unwrap().new_content();
    s.dispatch(Command::RebuildIndex);
    settle(&mut s, |s| !s.index_rebuilding && s.state().index_coverage().complete());
    s.dispatch(Command::FindDeclarations("draft_only".into()));
    settle(&mut s, |s| s.state().results().is_some_and(|r| r.done));
    assert_eq!(s.state().results().unwrap().hits.len(), 1, "current declarations must not come from old saved text");
    assert_eq!(std::fs::read(file).unwrap(), b"fn saved() {}\n");
    assert!(Arc::ptr_eq(s.state().view(tab).unwrap().snapshot(), &current));
}

#[test]
fn markdown_syntax_and_find_use_the_same_current_admission() {
    let d = tempfile::tempdir().unwrap();
    let file = write(d.path(), "draft.md", b"# saved\n");
    let mut s = session(d.path());
    open(&mut s, Command::Open(file.clone()), "draft.md");
    settle(&mut s, |s| s.state().highlights().is_some());
    let tab = s.state.active.unwrap();
    let initial = s.state().view(tab).unwrap().snapshot().clone();
    let text = "# draft-only\n\n```rust\nfn current() {}\n```\n";
    let current = Arc::new(initial.apply_edits(initial.key().version + 1,
        &[crate::text::TextEdit { range: 0..initial.len_bytes(), text: Arc::from(text) }]).unwrap());
    s.state.sources.buffers.get_mut(&current.key().buffer).unwrap().snapshot = Some(current.clone());
    s.active_mut().unwrap().new_content();
    s.dispatch(Command::Split(Split::Columns));
    s.dispatch(Command::ToggleMarkdownSource);
    s.dispatch(Command::FocusPane(0));
    s.dispatch(Command::Find("draft-only".into()));
    settle(&mut s, |s| s.state().shown(1).unwrap().preview.as_ref().is_some_and(|p| p.as_ref().is_ok_and(|p| p.doc.source.as_ref() == text))
        && s.state().active().unwrap().find().is_some_and(|f| !f.pending));
    assert_eq!(s.analysis.materializations.load(Ordering::Relaxed), 1);
    assert_eq!(s.state().active().unwrap().find().unwrap().matches.len(), 1);
    assert!(Arc::ptr_eq(s.state().view(tab).unwrap().snapshot(), &current));
    assert_eq!(std::fs::read(file).unwrap(), b"# saved\n");
}

#[test]
fn syntax_and_find_share_one_current_materialization_across_split_views() {
    let d = tempfile::tempdir().unwrap();
    let file = write(d.path(), "draft.rs", b"fn saved() {}\n");
    let mut s = session(d.path());
    open(&mut s, Command::Open(file.clone()), "draft.rs");
    settle(&mut s, |s| s.state().highlights().is_some());
    let tab = s.state.active.unwrap();
    let initial = s.state().view(tab).unwrap().snapshot().clone();
    let current = Arc::new(initial.apply_edits(initial.key().version + 1,
        &[crate::text::TextEdit { range: 0..initial.len_bytes(), text: Arc::from("fn draft_only() {}\n") }]).unwrap());
    s.state.sources.buffers.get_mut(&current.key().buffer).unwrap().snapshot = Some(current.clone());
    s.active_mut().unwrap().new_content();
    s.dispatch(Command::SetBreadcrumbs(true));
    s.dispatch(Command::Split(Split::Columns));
    s.dispatch(Command::Find("draft_only".into()));
    settle(&mut s, |s| s.state().shown(0).unwrap().declarations.as_ref().is_some_and(|d| d.items.iter().any(|d| d.name.as_ref() == "draft_only"))
        && s.state().shown(1).unwrap().declarations.as_ref().is_some_and(|d| d.items.iter().any(|d| d.name.as_ref() == "draft_only"))
        && s.state().active().unwrap().find().is_some_and(|f| !f.pending));
    assert_eq!(s.state().active().unwrap().find().unwrap().matches.len(), 1);
    assert_eq!(s.analysis.materializations.load(Ordering::Relaxed), 1, "the admitted version is shared by syntax and Find");
    assert!(Arc::ptr_eq(s.state().view(tab).unwrap().snapshot(), &current));
    assert_eq!(std::fs::read(file).unwrap(), b"fn saved() {}\n");
}

#[test]
fn current_reader_status_uses_snapshot_coordinates_before_worker_admission() {
    let d = tempfile::tempdir().unwrap();
    let file = write(d.path(), "current-status.txt", b"saved\n");
    let mut s = session(d.path());
    open(&mut s, Command::Open(file.clone()), "current-status.txt");
    let tab = s.state().active().unwrap().id();
    let initial = s.state().view(tab).unwrap().snapshot().clone();
    let current = Arc::new(initial.apply_edits(initial.key().version + 1,
        &[crate::text::TextEdit { range: 0..initial.len_bytes(), text: Arc::from("one\ntwo\n😀end\n") }]).unwrap());
    s.state.sources.buffers.get_mut(&current.key().buffer).unwrap().snapshot = Some(current.clone());
    s.dispatch(Command::GoToLine(LineColumn { line: 3, column: Some(2) }));
    assert_eq!(s.status_position(), "3:5", "status must use the current source, before a worker needs contiguous text");
    s.dispatch(Command::SelectAll);
    assert_eq!(s.dispatch(Command::Copy).copy.as_deref(), Some("one\ntwo\n😀end\n"));
    assert!(Arc::ptr_eq(s.state().view(tab).unwrap().snapshot(), &current));
    assert_eq!(std::fs::read(file).unwrap(), b"saved\n");
}

#[test]
fn find_materializes_the_current_snapshot_without_replacing_it() {
    let d = tempfile::tempdir().unwrap();
    let file = write(d.path(), "draft.txt", b"saved\n");
    let mut s = session(d.path());
    open(&mut s, Command::Open(file.clone()), "draft.txt");
    let tab = s.state.active.unwrap();
    let initial = s.state().view(tab).unwrap().snapshot().clone();
    let current = Arc::new(initial.apply_edits(initial.key().version + 1,
        &[crate::text::TextEdit { range: 0..initial.len_bytes(), text: Arc::from("draft-only\n") }]).unwrap());
    // Installing a current draft must leave the saved identity distinct from analysis reuse.
    s.state.sources.buffers.get_mut(&current.key().buffer).unwrap().snapshot = Some(current.clone());
    s.dispatch(Command::Find("draft-only".into()));
    assert_eq!(s.state().active().unwrap().find().unwrap().version, current.key().version);
    settle(&mut s, |s| s.state().active().unwrap().find().is_some_and(|f| !f.pending));
    assert_eq!(s.state().active().unwrap().find().unwrap().matches.len(), 1);
    assert!(Arc::ptr_eq(s.state().view(tab).unwrap().snapshot(), &current));
    assert_eq!(source_text(s.state().current_source().unwrap()), "draft-only\n");
    assert_eq!(source_text(&initial), "saved\n");
    let saved = s.state.sources.buffers[&current.key().buffer].saved.as_ref().unwrap();
    assert!(saved.matches(&initial), "analysis must preserve the confirmed saved identity");
    assert!(!saved.matches(&current), "an admitted draft is not a new saved baseline");
    assert_eq!(std::fs::read(&file).unwrap(), b"saved\n");
}

#[test]
fn reloading_without_a_find_releases_superseded_pending_analysis_text() {
    let d = tempfile::tempdir().unwrap();
    let file = write(d.path(), "versions.txt", b"alpha\n");
    let mut s = session(d.path());
    open(&mut s, Command::Open(file.clone()), "versions.txt");
    let (started, entered) = std::sync::mpsc::channel();
    let releases: Vec<_> = (0..crate::jobs::MAX_JOBS).map(|_| {
        let (release, wait) = std::sync::mpsc::channel();
        let started = started.clone();
        s.jobs.submit(Priority::Visible, move || {
            started.send(()).unwrap();
            let _ = wait.recv_timeout(Duration::from_secs(5));
        });
        release
    }).collect();
    for _ in &releases { entered.recv_timeout(Duration::from_secs(5)).unwrap(); }
    s.dispatch(Command::Find("alpha".into()));
    let mut superseded = None;
    for text in ["beta\n", "gamma\n"] {
        std::fs::write(&file, text).unwrap();
        s.dispatch(Command::Reload);
        let tab = s.state.active.unwrap();
        let version = s.versions.fetch_add(1, Ordering::Relaxed) + 1;
        // The filesystem boundary is held; deliver the exact decoded bytes through its real completion.
        s.dispatch(Completion::Loaded { request: s.request, tab, path: file.clone(),
            result: Ok((Stamp::of(&file), Arc::new(Document::decode(file.clone(), version, text.as_bytes())), std::fs::canonicalize(&file).ok())) });
        if superseded.is_none() {
            superseded = Some(Arc::downgrade(s.state().view(tab).unwrap().snapshot()));
            s.dispatch(Command::CloseFind);
        }
    }
    let released = superseded.unwrap().upgrade().is_none();
    for release in releases { release.send(()).unwrap(); }
    s.dispatch(Command::Find("gamma".into()));
    settle(&mut s, |s| s.state().active().unwrap().find().is_some_and(|f| !f.pending));
    assert_eq!(source_text(s.state().current_source().unwrap()), "gamma\n");
    assert_eq!(s.state().active().unwrap().find().unwrap().matches.len(), 1);
    assert!(released, "a superseded pending snapshot must be released before blocked work resumes");
}

#[test]
fn replacing_pending_find_requests_bounds_retained_document_owners() {
    let d = tempfile::tempdir().unwrap();
    let file = write(d.path(), "find.txt", b"needle\nother\n");
    let mut s = session(d.path());
    open(&mut s, Command::Open(file), "find.txt");
    let doc = analysis_document(&mut s);
    let before = Arc::strong_count(&doc);
    let (started, entered) = std::sync::mpsc::channel();
    let releases: Vec<_> = (0..crate::jobs::MAX_JOBS).map(|_| {
        let (release, wait) = std::sync::mpsc::channel();
        let started = started.clone();
        s.jobs.submit(Priority::Visible, move || {
            started.send(()).unwrap();
            let _ = wait.recv_timeout(Duration::from_secs(5));
        });
        release
    }).collect();
    for _ in &releases { entered.recv_timeout(Duration::from_secs(5)).unwrap(); }
    for i in 0..128 { s.dispatch(Command::Find(format!("superseded-{i}"))); }
    s.dispatch(Command::Find("needle".into()));
    let retained = Arc::strong_count(&doc);
    for release in releases { release.send(()).unwrap(); }
    settle(&mut s, |s| s.state().active().unwrap().find().is_some_and(|f| !f.pending));
    let found = s.state().active().unwrap().find().unwrap();
    assert_eq!(found.query, "needle");
    assert_eq!(found.matches.len(), 1);
    assert!(retained <= before + 8, "superseded queued requests retained {} extra Documents", retained - before);
}

#[test]
fn dropping_a_split_session_cancels_its_other_panes_running_parser() {
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    let a = write(d.path(), "a.txt", b"first\n");
    let b = write(d.path(), "b.txt", b"second\n");
    open(&mut s, Command::Open(a), "a.txt");
    s.dispatch(Command::Split(Split::Columns));
    open(&mut s, Command::Open(b), "b.txt");
    let version = pane_view(&s, 0).snapshot().key().version;
    let wanted = s.wanted(version);
    let receiver = s.completions();
    let (started, entered) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let (finished, result) = std::sync::mpsc::channel();
    s.jobs.submit(Priority::Background, move || {
        let first = std::sync::atomic::AtomicBool::new(true);
        let wait = std::sync::Mutex::new(wait);
        let parsed = highlight::parse_with(Language::Rust, "fn cancelled() { let value = 1; }\n".repeat(2_000).into(),
            highlight::Want { folds: true, declarations: true }, move || {
                if first.swap(false, Ordering::Relaxed) {
                    started.send(()).unwrap();
                    if wait.lock().unwrap().recv_timeout(Duration::from_secs(5)).is_err() { return false; }
                }
                wanted()
            });
        finished.send(parsed.is_none()).unwrap();
    });
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    let dropping = std::thread::spawn(move || drop(s));
    let until = std::time::Instant::now() + Duration::from_secs(5);
    while !receiver.is_closed() && std::time::Instant::now() < until { std::thread::yield_now(); }
    let closed = receiver.is_closed();
    release.send(()).unwrap();
    let cancelled = result.recv_timeout(Duration::from_secs(5)).unwrap();
    dropping.join().unwrap();
    assert!(closed, "session teardown reached worker cancellation");
    assert!(cancelled, "the real parser must observe cancellation after its session closes");
}

#[test]
fn dropping_a_session_with_a_full_channel_does_not_hang() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("many");
    let names: Vec<String> = (0..(COMPLETIONS + 8) * crate::project::BATCH).map(|i| format!("d{}/f{i}", i % 50)).collect();
    tree(&root, &names.iter().map(String::as_str).collect::<Vec<_>>());
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root));
    let started = std::time::Instant::now();
    while s.completions().len() < COMPLETIONS && started.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(s.completions().len(), COMPLETIONS, "the scan filled the bounded channel and waits");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || { drop(s); tx.send(()).unwrap(); });
    rx.recv_timeout(Duration::from_secs(5)).expect("dropping returns");
}

fn titles(s: &Session) -> Vec<String> {
    s.state().tabs().iter().map(|t| format!("{}{}{}", t.title(), if t.retained() { "" } else { "~" }, if s.state().active().is_some_and(|a| a.id() == t.id()) { "*" } else { "" })).collect()
}

fn open(s: &mut Session, c: Command, name: &str) {
    s.dispatch(c);
    settle(s, |s| s.state().current_source().is_some_and(|d| d.source().ends_with(name)) && s.state().loading().is_none());
}

#[test]
fn single_opens_reuse_the_preview_and_retained_tabs_stay() {
    let d = tempfile::tempdir().unwrap();
    let [a, b, c] = ["a.rs", "b.rs", "c.rs"].map(|n| write(d.path(), n, n.as_bytes()));
    let mut s = session(d.path());
    open(&mut s, Command::Open(a.clone()), "a.rs");
    open(&mut s, Command::Open(b.clone()), "b.rs");
    assert_eq!(titles(&s), ["b.rs~*"], "the preview tab was reused");
    open(&mut s, Command::OpenRetained(b.clone()), "b.rs");
    assert_eq!(titles(&s), ["b.rs*"], "double-click retains the previewed source");
    open(&mut s, Command::Open(c.clone()), "c.rs");
    open(&mut s, Command::Open(a.clone()), "a.rs");
    assert_eq!(titles(&s), ["b.rs", "a.rs~*"]);
    // Opening a retained source activates its tab.
    open(&mut s, Command::Open(b.clone()), "b.rs");
    assert_eq!(titles(&s), ["b.rs*", "a.rs~"]);
    let preview = s.state().tabs()[1].id();
    s.dispatch(Command::Pin(preview));
    open(&mut s, Command::Open(c.clone()), "c.rs");
    assert_eq!(titles(&s), ["b.rs", "c.rs~*", "a.rs"], "a new preview opens beside the active tab");
    assert_eq!(s.disabled(CommandId::PinTab), None);
    s.dispatch(Command::Close);
    assert_eq!(titles(&s), ["b.rs", "a.rs*"], "closing activates a neighbour");
    for t in s.state().tabs().iter().map(|t| t.id()).collect::<Vec<_>>() {
        s.dispatch(Command::CloseTab(t));
    }
    assert!(s.state().tabs().is_empty() && s.state().current_source().is_none());
}

#[test]
fn each_tab_keeps_its_own_position_and_released_content_reloads_there() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "a.txt", "line\n".repeat(100).as_bytes());
    let b = write(d.path(), "b.txt", "other\n".repeat(100).as_bytes());
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(a.clone()), "a.txt");
    s.dispatch(Command::Scroll(40));
    s.dispatch(Command::Select(Selection { anchor: 205, head: 209 }));
    s.dispatch(Command::ScrollColumn(7));
    let a_tab = s.state().active().unwrap().id();
    open(&mut s, Command::OpenRetained(b.clone()), "b.txt");
    assert_eq!((s.state().top_line(), s.state().selection(), s.state().column()), (0, None, 0), "b has its own state");
    let a_state = &s.state().tabs()[0];
    assert!(!a_state.loaded(), "the inactive tab's content went to the inactive cache");
    assert_eq!((a_state.top_line(), a_state.column()), (40, 7));
    assert!(s.inactive_bytes() > 0);
    s.dispatch(Command::ActivateTab(a_tab));
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source().ends_with("a.txt")));
    assert_eq!((s.state().top_line(), s.state().selection().map(|x| x.range()), s.state().column()), (40, Some(205..209), 7));
}

#[test]
fn back_and_forward_restore_positions_and_reopen_closed_sources() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "a.txt", "alpha\n".repeat(50).as_bytes());
    let b = write(d.path(), "b.txt", "beta\n".repeat(50).as_bytes());
    let c = write(d.path(), "c.txt", b"gamma\n");
    let mut s = session(d.path());
    open(&mut s, Command::Open(a.clone()), "a.txt");
    s.dispatch(Command::Scroll(20));
    open(&mut s, Command::Open(b.clone()), "b.txt");
    s.dispatch(Command::Scroll(10));
    assert!(s.state().can_back() && !s.state().can_forward());
    // Back reopens a in the preview tab (b's tab was the preview) at line 20.
    open(&mut s, Command::Back, "a.txt");
    assert_eq!(s.state().top_line(), 20);
    assert!(s.state().can_forward());
    open(&mut s, Command::Forward, "b.txt");
    assert_eq!(s.state().top_line(), 10, "b's position was kept when leaving it");
    // Scrolling and pinning are not steps.
    s.dispatch(Command::Scroll(12));
    s.dispatch(Command::Pin(s.state().active().unwrap().id()));
    open(&mut s, Command::Back, "a.txt");
    // A new navigation after Back clears Forward.
    open(&mut s, Command::Open(c.clone()), "c.txt");
    assert!(!s.state().can_forward());
    // Closing every tab keeps history: Back reopens where the reader was.
    for t in s.state().tabs().iter().map(|t| t.id()).collect::<Vec<_>>() {
        s.dispatch(Command::CloseTab(t));
    }
    assert!(s.state().current_source().is_none());
    open(&mut s, Command::Back, "c.txt");
    open(&mut s, Command::Back, "a.txt");
    assert_eq!(s.state().top_line(), 20);
}

#[test]
fn back_relocates_by_context_after_the_file_changes() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "a.rs", b"fn one() {}\nfn two() {}\nfn target() {}\n");
    let b = write(d.path(), "b.rs", b"fn b() {}\n");
    let mut s = session(d.path());
    open(&mut s, Command::Open(a.clone()), "a.rs");
    s.dispatch(Command::Scroll(2));
    s.dispatch(Command::Select(Selection { anchor: 24, head: 26 }));
    open(&mut s, Command::Open(b.clone()), "b.rs");
    std::fs::write(&a, b"// new\n// lines\nfn one() {}\nfn two() {}\nfn target() {}\n").unwrap();
    open(&mut s, Command::Back, "a.rs");
    assert_eq!(s.state().top_line(), 4, "found by its line, two lines down");
}

#[test]
fn separate_sessions_are_independent() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "a.txt", b"a\n");
    let (mut one, mut two) = (session(d.path()), session(d.path()));
    open(&mut one, Command::Open(a.clone()), "a.txt");
    assert!(two.state().tabs().is_empty() && !two.state().can_back());
    open(&mut two, Command::OpenRetained(a.clone()), "a.txt");
    one.dispatch(Command::Close);
    assert_eq!(titles(&two), ["a.txt*"], "closing in one session leaves the other");
}

/// Cached tab switches and Back/Forward: `cargo test --release -p reader-core -- --ignored --nocapture tab_perf`.
#[test]
#[ignore = "timing: run in release, alone"]
fn tab_perf() {
    let d = tempfile::tempdir().unwrap();
    let unit = "/// Doc.\npub fn item(a: u32) -> Option<String> {\n    let s = \"text\";\n    Some(format!(\"{a}{s}\"))\n}\n";
    let paths: Vec<PathBuf> = (0..4).map(|i| write(d.path(), &format!("f{i}.rs"), unit.repeat(2000).as_bytes())).collect();
    let mut s = session(d.path());
    for p in &paths {
        open(&mut s, Command::OpenRetained(p.clone()), p.file_name().unwrap().to_str().unwrap());
    }
    let ids: Vec<TabId> = s.state().tabs().iter().map(|t| t.id()).collect();
    let (mut switch, mut history) = (Vec::new(), Vec::new());
    // Ten untimed warmup rounds, then 500: 2,000 switches and 1,000 Back/Forward steps.
    for round in 0..510 {
        let timed = round >= 10;
        for (i, id) in ids.iter().enumerate() {
            let started = std::time::Instant::now();
            s.dispatch(Command::ActivateTab(*id));
            let released = s.state().current_source().is_none();
            settle(&mut s, |s| s.state().current_source().is_some() && s.state().loading().is_none());
            if timed { switch.push(started.elapsed().as_secs_f64() * 1000.0); }
            if timed && i > 0 { assert!(released, "an inactive tab's content was released and comes back from the cache"); }
        }
        for c in [Command::Back, Command::Forward] {
            let started = std::time::Instant::now();
            s.dispatch(c);
            settle(&mut s, |s| s.state().current_source().is_some() && s.state().loading().is_none());
            if timed { history.push(started.elapsed().as_secs_f64() * 1000.0); }
        }
    }
    for (name, v) in [("reader/tab-switch-cached", &mut switch), ("reader/back-forward-cached", &mut history)] {
        v.sort_by(f64::total_cmp);
        let p = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
        eprintln!("READER_PERF {{\"name\":{name:?},\"unit\":\"ms\",\"samples\":{v:?}}}");
        eprintln!("{name}: median {:.3} ms, p95 {:.3} ms, max {:.3} ms over {}", p(0.5), p(0.95), v[v.len() - 1], v.len());
        assert!(p(0.95) < 50.0);
    }
}

#[test]
fn closing_the_active_tab_keeps_it_reachable_through_history() {
    let d = tempfile::tempdir().unwrap();
    let [a, b, c] = ["a.txt", "b.txt", "c.txt"].map(|n| write(d.path(), n, n.as_bytes()));
    let mut s = session(d.path());
    for (p, n) in [(&a, "a.txt"), (&b, "b.txt"), (&c, "c.txt")] {
        open(&mut s, Command::OpenRetained(p.clone()), n);
    }
    s.dispatch(Command::Close); // c closes; b shows
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source().ends_with("b.txt")));
    open(&mut s, Command::Back, "c.txt");
    assert_eq!(titles(&s), ["a.txt", "b.txt", "c.txt~*"], "the closed source reopens in a preview tab");
    open(&mut s, Command::Back, "b.txt");
    open(&mut s, Command::Back, "a.txt");
    open(&mut s, Command::Forward, "b.txt");
    open(&mut s, Command::Forward, "c.txt");
    // a → b, b closes, then d: b stays between a and d.
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(a.clone()), "a.txt");
    open(&mut s, Command::OpenRetained(b.clone()), "b.txt");
    s.dispatch(Command::Close);
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source().ends_with("a.txt")));
    open(&mut s, Command::OpenRetained(c.clone()), "c.txt");
    open(&mut s, Command::Back, "a.txt");
    open(&mut s, Command::Back, "b.txt");
}

#[test]
fn a_kept_tab_whose_file_disappeared_stays_and_says_so() {
    let d = tempfile::tempdir().unwrap();
    let (a, b) = (write(d.path(), "a.txt", b"alpha\n"), write(d.path(), "b.txt", b"beta\n"));
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(a.clone()), "a.txt");
    let a_tab = s.state().active().unwrap().id();
    open(&mut s, Command::OpenRetained(b), "b.txt");
    std::fs::remove_file(&a).unwrap();
    s.dispatch(Command::ActivateTab(a_tab));
    settle(&mut s, |s| s.state().loading().is_none());
    assert_eq!(titles(&s), ["a.txt*", "b.txt"], "the tab stays");
    assert!(s.state().current_source().is_none());
    assert!(s.state().notices().iter().any(|n| n.starts_with("Unavailable")), "{:?}", s.state().notices());
    assert!(!a.exists(), "never recreated");
}

/// Replace `path` atomically, as editors and agents do: write a temporary file beside it and rename over it.
fn replace(path: &Path, bytes: &[u8]) {
    let temp = path.with_extension("tmp-write");
    std::fs::write(&temp, bytes).unwrap();
    std::fs::rename(&temp, path).unwrap();
}

#[test]
fn an_external_replacement_advances_the_version_and_follows_the_context() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "a.rs", b"fn one() {}\nfn two() {}\nfn target() {}\n");
    let other = write(d.path(), "o.rs", b"fn o() {}\n");
    let mut s = session(d.path());
    open(&mut s, Command::Open(other), "o.rs");
    open(&mut s, Command::Open(a.clone()), "a.rs");
    s.dispatch(Command::Scroll(2));
    s.dispatch(Command::Select(Selection { anchor: 27, head: 33 }));
    let (version, history) = (active_view(&s).snapshot().key().version, s.state().can_back());
    replace(&a, b"// header\n// more\nfn one() {}\nfn two() {}\nfn target() {}\n");
    settle(&mut s, |s| active_view(s).snapshot().key().version > version);
    let tab = s.state().active().unwrap();
    assert_eq!(source_line(s.state().current_source().unwrap(), 4), "fn target() {}");
    assert_eq!((tab.top_line(), tab.selection().map(|x| x.range())), (4, Some(45..51)), "the selection follows its line");
    assert!(!tab.retained() && s.state().can_back() == history, "the role and history are unchanged");
    // Rewriting the same bytes is not a new version.
    let version = active_view(&s).snapshot().key().version;
    replace(&a, b"// header\n// more\nfn one() {}\nfn two() {}\nfn target() {}\n");
    std::thread::sleep(Duration::from_millis(300));
    while let Ok(c) = s.completions().try_recv() {
        s.dispatch(c);
    }
    assert_eq!(active_view(&s).snapshot().key().version, version);
}

#[test]
fn shortening_deleting_and_restoring_keep_the_reader_honest() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "a.txt", "line\n".repeat(50).as_bytes());
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(a.clone()), "a.txt");
    s.dispatch(Command::Scroll(40));
    let version = active_view(&s).snapshot().key().version;
    std::fs::write(&a, b"short\n").unwrap();
    settle(&mut s, |s| active_view(s).snapshot().key().version > version);
    assert_eq!(s.state().top_line(), 1, "clamped to the new length");
    assert!(s.state().notices().iter().any(|n| n.starts_with("Approximate")));
    std::fs::remove_file(&a).unwrap();
    settle(&mut s, |s| s.state().active().unwrap().unavailable().is_some());
    assert_eq!(source_line(s.state().current_source().unwrap(), 0), "short", "the last content stays");
    assert!(s.state().notices().iter().any(|n| n.starts_with("Unavailable")));
    assert!(!a.exists());
    std::fs::write(&a, b"back again\n").unwrap();
    settle(&mut s, |s| s.state().active().unwrap().unavailable().is_none());
    assert_eq!(source_line(s.state().current_source().unwrap(), 0), "back again");
}

#[test]
fn a_released_tabs_file_deleted_comes_back_from_the_cache_and_is_kept() {
    let d = tempfile::tempdir().unwrap();
    let (a, b) = (write(d.path(), "a.txt", b"alpha\n"), write(d.path(), "b.txt", b"beta\n"));
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(a.clone()), "a.txt");
    let a_tab = s.state().active().unwrap().id();
    open(&mut s, Command::OpenRetained(b), "b.txt");
    assert!(!s.state().tabs()[0].loaded());
    std::fs::remove_file(&a).unwrap();
    settle(&mut s, |s| s.state().tabs()[0].unavailable().is_some());
    assert!(s.state().tabs()[0].loaded(), "taken back from the cache before it could be evicted");
    assert_eq!(s.inactive_bytes(), 0);
    s.dispatch(Command::ActivateTab(a_tab));
    assert_eq!(source_line(s.state().current_source().unwrap(), 0), "alpha");
    // Leaving it again does not release it: it is no longer reloadable.
    let b_tab = s.state().tabs()[1].id();
    s.dispatch(Command::ActivateTab(b_tab));
    settle(&mut s, |s| s.state().current_source().is_some());
    assert!(s.state().tabs()[0].loaded());
    s.dispatch(Command::CloseTab(a_tab));
    assert_eq!(s.state().tabs().len(), 1, "closing releases it");
}

#[test]
fn a_confirmed_rename_is_followed_and_a_same_name_replacement_is_not() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "a.txt", b"content\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(a.clone()), "a.txt");
    let moved = d.path().join("moved.txt");
    std::fs::rename(&a, &moved).unwrap();
    settle(&mut s, |s| s.state().active().unwrap().source() == moved);
    assert_eq!(source_line(s.state().current_source().unwrap(), 0), "content");
    assert!(s.state().active().unwrap().unavailable().is_none());
    // A different file appearing where the old one was is not the same document.
    std::fs::write(&a, b"impostor\n").unwrap();
    std::thread::sleep(Duration::from_millis(300));
    while let Ok(c) = s.completions().try_recv() {
        s.dispatch(c);
    }
    assert_eq!(s.state().active().unwrap().source(), moved);
    assert_eq!(source_line(s.state().current_source().unwrap(), 0), "content");
}

#[test]
fn rapid_changes_end_on_the_last_content_and_recheck_works_without_events() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "a.txt", b"0\n");
    let mut s = session(d.path());
    open(&mut s, Command::Open(a.clone()), "a.txt");
    for i in 1..=50 {
        replace(&a, format!("{i}\n").as_bytes());
    }
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| source_line(d, 0) == "50"));
    // A late result for an older check does nothing.
    let id = s.state().active().unwrap().id();
    let stale = Completion::Refreshed { tab: id, token: 0, result: Refresh::Gone("old".into()) };
    assert_eq!(s.dispatch(stale), Effect::default());
    // Recheck (focus regained) finds what notifications would have.
    s.watcher.set(Default::default());
    std::fs::write(&a, b"rechecked\n").unwrap();
    s.dispatch(Command::Recheck);
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| source_line(d, 0) == "rechecked"));
}

#[test]
fn the_project_inventory_follows_created_and_deleted_files() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    tree(&root, &["a.rs", "src/b.rs"]);
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, scanned);
    std::fs::write(root.join("src/new.rs"), b"").unwrap();
    std::fs::remove_file(root.join("a.rs")).unwrap();
    let files = |s: &Session| s.state().project().unwrap().rows().iter().map(|r| r.path.to_string()).collect::<Vec<_>>();
    s.dispatch(Command::ToggleDir("src".into()));
    settle(&mut s, |s| !s.state().project().unwrap().refreshing() && files(s).contains(&"src/new.rs".to_string()));
    assert_eq!(files(&s), ["src", "src/b.rs", "src/new.rs"]);
}

#[test]
fn switching_roots_quickly_follows_only_the_last_one() {
    let d = tempfile::tempdir().unwrap();
    let roots: Vec<PathBuf> = (0..5).map(|i| { let r = d.path().join(format!("r{i}")); tree(&r, &[&format!("f{i}.rs")]); r }).collect();
    let mut s = session(d.path());
    for r in &roots {
        s.dispatch(Command::OpenRoot(r.clone()));
    }
    settle(&mut s, scanned);
    assert_eq!(s.state().project().unwrap().rows()[0].path.as_ref(), "f4.rs");
    // Changes in an old root do not touch the current inventory.
    std::fs::write(roots[0].join("late.rs"), b"").unwrap();
    std::fs::write(roots[4].join("new.rs"), b"").unwrap();
    settle(&mut s, |s| !s.state().project().unwrap().refreshing() && s.state().project().unwrap().files() == 2);
    let names: Vec<String> = s.state().project().unwrap().rows().iter().map(|r| r.path.to_string()).collect();
    assert_eq!(names, ["f4.rs", "new.rs"]);
}

/// Refresh latency and memory under repeated changes:
/// `cargo test --release -p reader-core -- --ignored --nocapture refresh_perf`.
#[test]
#[ignore = "timing: run in release, alone"]
fn refresh_perf() {
    let rss = memory_mib;
    let d = tempfile::tempdir().unwrap();
    let unit = "/// Doc.\npub fn item(a: u32) -> Option<String> {\n    let s = \"text\";\n    Some(format!(\"{a}{s}\"))\n}\n";
    let body = unit.repeat(12_000); // ~1 MiB
    let a = write(d.path(), "big.rs", body.as_bytes());
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(a.clone()), "big.rs");
    let (mut latency, mut memory) = (Vec::new(), Vec::new());
    // Ten untimed warmup changes, then 1,000.
    for i in 0..1010 {
        let version = active_view(&s).snapshot().key().version;
        let started = std::time::Instant::now();
        replace(&a, format!("// change {i}\n{body}").as_bytes());
        settle(&mut s, |s| active_view(s).snapshot().key().version > version);
        if i >= 10 {
            latency.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        if i % 50 == 0 && let Some(m) = rss() {
            memory.push(m);
        }
    }
    latency.sort_by(f64::total_cmp);
    let p = |q: f64| latency[((latency.len() - 1) as f64 * q).round() as usize];
    eprintln!("READER_PERF {{\"name\":\"reader/refresh-1mib-watch-to-applied\",\"unit\":\"ms\",\"samples\":{latency:?}}}");
    eprintln!("READER_PERF {{\"name\":\"reader/refresh-rss-timeline\",\"unit\":\"MiB\",\"samples\":{memory:?}}}");
    eprintln!("refresh of a 1 MiB file, write to applied (includes the {} ms settle): median {:.1} ms, p95 {:.1} ms over {}", crate::watch::SETTLE.as_millis(), p(0.5), p(0.95), latency.len());
    eprintln!("RSS every 50 changes: {memory:?}");
    let (first, last) = (memory[1], memory[memory.len() - 1]);
    assert!(last - first < 32.0, "memory grew {:.1} MiB over 1,000 changes", last - first);
}

#[test]
fn refreshes_keep_encodings_line_endings_roles_and_bytes() {
    let d = tempfile::tempdir().unwrap();
    let latin = write(d.path(), "latin.txt", b"caf\xe9 au lait, cr\xe8me br\xfbl\xe9e\r\nsecond\r\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(latin.clone()), "latin.txt");
    let version = active_view(&s).snapshot().key().version;
    let after = b"caf\xe9 au lait, cr\xe8me br\xfbl\xe9e\r\nchanged\r\n".to_vec();
    replace(&latin, &after);
    settle(&mut s, |s| active_view(s).snapshot().key().version > version);
    let doc = s.state().current_source().unwrap();
    assert_eq!((source_line(doc, 1), doc.encoding()), ("changed".to_string(), "windows-1252"));
    assert_eq!(doc.copy_range(doc.line_range(0).unwrap().end..doc.line_range(1).unwrap().start).unwrap(), "\r\n", "line endings kept");
    assert!(s.state().notices().iter().any(|n| n == "Encoding: windows-1252"));
    assert!(s.state().active().unwrap().retained(), "a kept tab stays kept");
    s.dispatch(Command::SelectAll);
    s.dispatch(Command::Copy);
    assert_eq!(std::fs::read(&latin).unwrap(), after, "reader actions never write");
}

#[test]
fn a_refresh_during_a_pending_load_does_not_cross_it() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "a.txt", b"first\n");
    let mut s = session(d.path());
    s.dispatch(Command::Open(a.clone()));
    // Before the load lands, the file changes and a recheck runs.
    replace(&a, b"second\n");
    s.dispatch(Command::Recheck);
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| source_line(d, 0) == "second") && s.state().loading().is_none());
}

#[test]
fn two_sessions_watching_one_file_both_follow_it() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "a.txt", b"one\n");
    let (mut x, mut y) = (session(d.path()), session(d.path()));
    open(&mut x, Command::Open(a.clone()), "a.txt");
    open(&mut y, Command::OpenRetained(a.clone()), "a.txt");
    replace(&a, b"two\n");
    settle(&mut x, |s| s.state().current_source().is_some_and(|d| source_line(d, 0) == "two"));
    settle(&mut y, |s| s.state().current_source().is_some_and(|d| source_line(d, 0) == "two"));
    x.dispatch(Command::Close);
    assert!(y.state().current_source().is_some(), "closing in one window leaves the other");
}

#[test]
fn an_editor_backup_rename_keeps_the_reader_on_the_original_path() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "a.txt", b"old\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(a.clone()), "a.txt");
    // Like an editor with backups: move the file aside, then write the new one in its place.
    std::fs::rename(&a, d.path().join("a.txt~")).unwrap();
    std::fs::write(&a, b"new\n").unwrap();
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| source_line(d, 0) == "new"));
    assert_eq!(s.state().active().unwrap().source(), a.as_path());
}

#[cfg(unix)]
#[test]
fn two_spellings_of_one_file_share_a_tab_and_keep_the_requested_name() {
    let d = tempfile::tempdir().unwrap();
    let real = write(d.path(), "real.txt", b"x\n");
    let link = d.path().join("link.txt");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(link.clone()), "link.txt");
    assert_eq!(s.state().active().unwrap().title(), "link.txt");
    let dotted = d.path().join("sub/../link.txt");
    std::fs::create_dir(d.path().join("sub")).unwrap();
    s.dispatch(Command::OpenRetained(dotted));
    assert_eq!(s.state().tabs().len(), 1, "`..` normalizes to the same identity");
    s.dispatch(Command::OpenRetained(real));
    settle(&mut s, |s| s.state().loading().is_none() && s.state().current_source().is_some());
    assert_eq!(s.state().tabs().len(), 1, "the symlink and its target are one document");
    assert_eq!(s.state().active().unwrap().title(), "link.txt");
}

fn searched(s: &Session) -> bool {
    s.state().results().is_some_and(|r| r.done())
}


#[test]
fn active_file_search_finishes_after_its_source_view_is_released() {
    let d = tempfile::tempdir().unwrap();
    let a_bytes = b"A-only needle\nsecond needle\n"; let b_bytes = b"B stays current\n";
    let a = write(d.path(), "A.txt", a_bytes); let b = write(d.path(), "B.txt", b_bytes);
    let mut s = session(d.path()); open(&mut s, Command::OpenRetained(a.clone()), "A.txt");
    let a_tab = s.state().active().unwrap().id();
    let (entered, starts) = std::sync::mpsc::channel();
    let releases: Vec<_> = (0..crate::jobs::MAX_JOBS).map(|_| {
        let (release, wait) = std::sync::mpsc::channel(); let entered = entered.clone();
        s.jobs.submit(Priority::Visible, move || { entered.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); });
        release
    }).collect();
    for _ in &releases { starts.recv_timeout(Duration::from_secs(5)).unwrap(); }
    s.dispatch(Command::Search { query: "needle".into(), scope: SearchIn::ActiveFile });
    let result_id = s.state().results().unwrap().id();
    assert!(!searched(&s));
    s.dispatch(Command::OpenRetained(b.clone()));
    let b_tab = s.state().active().unwrap().id();
    let Ok((stamp, bytes)) = read(&b) else { panic!("stable disposable B") };
    let version = s.versions.fetch_add(1, Ordering::Relaxed) + 1;
    // Deliver the pending real filesystem read while the CPU executor remains gated.
    s.dispatch(Completion::Loaded { request: s.request, tab: b_tab, path: b.clone(),
        result: Ok((Some(stamp), Arc::new(Document::decode(b.clone(), version, &bytes)), std::fs::canonicalize(&b).ok())) });
    assert!(s.state().view(a_tab).is_none(), "A's reloadable view was released");
    assert_eq!(s.state().current_source().unwrap().source(), b.as_path());
    assert_eq!(s.state().results().unwrap().id(), result_id, "the results remain independently visible");
    for release in releases { release.send(()).unwrap(); }
    let until = std::time::Instant::now() + Duration::from_secs(2);
    while !searched(&s) && std::time::Instant::now() < until {
        if let Ok(completion) = s.completions().try_recv() { s.dispatch(completion); }
        else { std::thread::yield_now(); }
    }
    assert!(searched(&s), "a retained file search must finish after switching away from its source view");
    let results = s.state().results().unwrap();
    assert_eq!(results.id(), result_id);
    assert_eq!(results.hits().iter().map(|hit| (hit.line, hit.text.as_ref())).collect::<Vec<_>>(), [(0, "A-only needle"), (1, "second needle")]);
    assert!(results.hits().iter().all(|hit| hit.path.as_ref() == a.to_string_lossy()));
    assert_eq!(s.state().current_source().unwrap().source(), b.as_path());
    assert_eq!(source_text(s.state().current_source().unwrap()).as_bytes(), b_bytes);
    assert_eq!(std::fs::read(a).unwrap(), a_bytes); assert_eq!(std::fs::read(b).unwrap(), b_bytes);
}

#[test]
fn queued_result_validation_rechecks_the_retained_current_target() {
    let d = tempfile::tempdir().unwrap();
    let file = write(d.path(), "target.txt", b"token\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(file.clone()), "target.txt");
    s.dispatch(Command::Search { query: "token".into(), scope: SearchIn::ActiveFile });
    settle(&mut s, |s| s.state().results().is_some_and(|r| r.done()));
    let tab = s.state.active.unwrap();
    let old = s.state().view(tab).unwrap().snapshot().clone();
    let (started, entered) = std::sync::mpsc::channel();
    let releases: Vec<_> = (0..crate::jobs::MAX_JOBS).map(|_| {
        let (release, wait) = std::sync::mpsc::channel(); let started = started.clone();
        s.jobs.submit(Priority::Visible, move || { started.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); });
        release
    }).collect();
    for _ in &releases { entered.recv_timeout(Duration::from_secs(5)).unwrap(); }
    s.dispatch(Command::OpenResult(0));
    let current = Arc::new(old.apply_edits(old.key().version + 1,
        &[crate::text::TextEdit { range: 0..old.len_bytes(), text: Arc::from("changed target\n") }]).unwrap());
    s.state.sources.buffers.get_mut(&current.key().buffer).unwrap().snapshot = Some(current.clone());
    s.tab_mut(tab).unwrap().new_content();
    for release in releases { release.send(()).unwrap(); }
    settle(&mut s, |s| s.state().notices().iter().any(|n| n.contains("captured draft changed")));
    assert!(s.state().results().unwrap().is_stale(0));
    assert!(Arc::ptr_eq(s.state().view(tab).unwrap().snapshot(), &current));
    s.dispatch(Command::SelectAll);
    assert_eq!(s.dispatch(Command::Copy).copy.as_deref(), Some("changed target\n"));
    assert_eq!(std::fs::read(file).unwrap(), b"token\n");
}

#[test]
fn project_search_previews_without_moving_and_opens_validated_results() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    tree(&root, &["a.rs", "src/b.rs", "target/c.rs"]);
    std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
    std::fs::write(root.join("a.rs"), "fn needle() {}\r\nlet x = NEEDLE;\n").unwrap();
    std::fs::write(root.join("src/b.rs"), "// café needle\n").unwrap();
    std::fs::write(root.join("target/c.rs"), "needle\n").unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, scanned);
    let other = root.join("src/b.rs");
    open(&mut s, Command::OpenRetained(other.clone()), "b.rs");
    s.dispatch(Command::Scroll(0));
    let history = s.state().can_back();
    s.dispatch(Command::Search { query: "Needle".into(), scope: SearchIn::Project });
    settle(&mut s, searched);
    let r = s.state().results().unwrap();
    let mut got: Vec<_> = r.hits().iter().map(|h| (h.path.to_string(), h.line, h.range.start, h.range.end)).collect();
    got.sort();
    assert_eq!(got, [("a.rs".into(), 0, 3, 9), ("a.rs".into(), 1, 8, 14), ("src/b.rs".into(), 0, 9, 15)], "ignored files are not searched; ranges are bytes on the line");
    assert_eq!((r.kind().as_str(), r.limited(), r.partial_inventory(), r.files()), ("Text matches", false, false, 3));
    // Selecting previews: the main area stays on b.rs and history does not move.
    let a_index = r.hits().iter().position(|h| &*h.path == "a.rs" && h.line == 1).unwrap();
    s.dispatch(Command::SelectResult(a_index));
    settle(&mut s, |s| s.state().results().unwrap().preview().is_some_and(|p| p.index == a_index));
    assert_eq!(s.state().current_source().unwrap().source(), other);
    assert_eq!(s.state().can_back(), history);
    // Opening checks the file, then moves the main area to the match and records history.
    s.dispatch(Command::OpenResult(a_index));
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source().ends_with("a.rs")) && s.state().loading().is_none());
    let sel = s.state().selection().unwrap().range();
    assert_eq!(&source_text(s.state().current_source().unwrap())[sel], "NEEDLE");
    assert!(s.state().can_back());
    assert!(s.state().results().is_some(), "the results stay for the next one");
}

#[test]
fn project_matches_list_by_folder_then_file_and_folders_and_files_collapse() {
    use crate::search::{Group, Row};
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    tree(&root, &["a.rs", "src/b.rs", "src/c.rs", "src/d/e.rs", "web/f.ts"]);
    for (name, text) in [("a.rs", "needle\n"), ("src/b.rs", "needle needle\nneedle\n"), ("src/c.rs", "x needle\n"), ("src/d/e.rs", "needle\n"), ("web/f.ts", "none\n")] {
        std::fs::write(root.join(name), text).unwrap();
    }
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, scanned);
    s.dispatch(Command::Search { query: "needle".into(), scope: SearchIn::Project });
    settle(&mut s, searched);
    let r = s.state().results().unwrap();
    let paths: Vec<_> = r.hits().iter().map(|h| h.path.to_string()).collect();
    assert_eq!(paths, ["a.rs", "src/b.rs", "src/b.rs", "src/b.rs", "src/c.rs", "src/d/e.rs"], "folder by folder");
    let (dir, file) = (|d: &str, count, open| Row::Dir { dir: d.into(), count, open }, |p: &str, count, open| Row::File { path: p.into(), count, open });
    assert_eq!(r.rows(), [dir("", 1, true), file("a.rs", 1, true), Row::Hit(0), dir("src", 4, true), file("src/b.rs", 3, true), Row::Hit(1), Row::Hit(2), Row::Hit(3), file("src/c.rs", 1, true), Row::Hit(4), dir("src/d", 1, true), file("src/d/e.rs", 1, true), Row::Hit(5)]);
    assert_eq!(r.row_of_hit(4), Some(9));
    s.dispatch(Command::SelectResult(2));
    // A file folds to its row, a folder to its own; the selection and its preview stay.
    s.dispatch(Command::ToggleResultGroup(Group::File("src/b.rs".into())));
    let r = s.state().results().unwrap();
    assert_eq!(r.rows()[3..6], [dir("src", 4, true), file("src/b.rs", 3, false), file("src/c.rs", 1, true)]);
    assert_eq!((r.row_of_hit(2), r.selected()), (None, Some(2)));
    s.dispatch(Command::ToggleResultGroup(Group::Dir("src".into())));
    assert_eq!(s.state().results().unwrap().rows()[3..], [dir("src", 4, false), dir("src/d", 1, true), file("src/d/e.rs", 1, true), Row::Hit(5)]);
    s.dispatch(Command::ToggleResultGroup(Group::Dir("src".into())));
    assert_eq!(s.state().results().unwrap().rows()[4], file("src/b.rs", 3, false), "a file stays folded inside a reopened folder");
    // Revealing a result unfolds what hides it; its line is coloured off the UI thread when asked.
    s.dispatch(Command::ToggleResultGroup(Group::Dir("src".into())));
    s.dispatch(Command::RevealResult(2));
    let r = s.state().results().unwrap();
    assert_eq!((r.row_of_hit(2), r.matched_files(), r.root_name().as_deref()), (Some(6), 4, Some("proj")));
    s.dispatch(Command::ColourResults(vec![0, 99]));
    settle(&mut s, |s| s.state().results().unwrap().line_spans(0).is_some());
    assert!(s.state().results().unwrap().line_spans(1).is_none(), "only lines asked for");
    // A file search lists its matches without grouping.
    open(&mut s, Command::OpenRetained(root.join("src/b.rs")), "b.rs");
    s.dispatch(Command::Search { query: "needle".into(), scope: SearchIn::ActiveFile });
    settle(&mut s, searched);
    assert_eq!(s.state().results().unwrap().rows(), [Row::Hit(0), Row::Hit(1), Row::Hit(2)]);
}

#[test]
fn a_file_pattern_limits_a_project_search_and_its_rerun() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    tree(&root, &["a.rs", "src/b.rs", "src/c.rs", "src/b.test.rs", "web/f.ts"]);
    for name in ["a.rs", "src/b.rs", "src/c.rs", "src/b.test.rs", "web/f.ts"] {
        std::fs::write(root.join(name), "needle\n").unwrap();
    }
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, scanned);
    let found = |s: &Session| s.state().results().unwrap().hits().iter().map(|h| h.path.to_string()).collect::<Vec<_>>();
    s.dispatch(Command::Search { query: "needle".into(), scope: SearchIn::Matching("src/, !*.test.rs".into()) });
    settle(&mut s, searched);
    assert_eq!(found(&s), ["src/b.rs", "src/c.rs"]);
    let r = s.state().results().unwrap();
    assert_eq!((r.pattern(), r.files()), ("src/, !*.test.rs", 2), "the files the pattern picked");
    std::fs::write(root.join("a.rs"), "changed, still a needle\n").unwrap();
    s.dispatch(Command::RerunSearch);
    settle(&mut s, searched);
    assert_eq!(found(&s), ["src/b.rs", "src/c.rs"], "a rerun keeps the pattern");
    s.dispatch(Command::Search { query: "needle".into(), scope: SearchIn::Matching("src/[".into()) });
    settle(&mut s, searched);
    let r = s.state().results().unwrap();
    assert!(r.error().is_some_and(|e| e.contains("src/[")) && r.hits().is_empty(), "a bad pattern is reported, nothing searched");
}

#[test]
fn a_previewed_match_is_syntax_coloured_from_its_own_parse_or_its_tab() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    tree(&root, &["a.rs", "b.rs"]);
    std::fs::write(root.join("a.rs"), "fn needle() {}\n").unwrap();
    std::fs::write(root.join("b.rs"), "// needle\nfn other() {}\n").unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, scanned);
    s.dispatch(Command::Search { query: "needle".into(), scope: SearchIn::Project });
    settle(&mut s, searched);
    // Not open anywhere: parsed off the UI thread for the preview.
    settle(&mut s, |s| s.state().results().unwrap().preview().is_some_and(|p| p.index == 0 && p.highlights().is_some()));
    let spans = s.state().results().unwrap().preview().unwrap().highlights().unwrap().spans(0..1);
    assert!(spans[0].iter().any(|(_, c)| *c == lenz_content::highlight::Capture::Keyword));
    s.dispatch(Command::SelectResult(1));
    settle(&mut s, |s| s.state().results().unwrap().preview().is_some_and(|p| p.index == 1 && p.highlights().is_some()));
    let preview = s.state().results().unwrap().preview().unwrap();
    assert_eq!(preview.highlights().unwrap().spans(0..1)[0][0].1, lenz_content::highlight::Capture::Comment);
    // Away and back before anything finishes: the file shown last is still coloured.
    std::fs::write(root.join("b.rs"), "// needle, changed\n").unwrap();
    settle(&mut s, |s| s.state().results().unwrap().is_stale(1));
    for i in [1, 0, 1, 0, 1] {
        s.dispatch(Command::SelectResult(i));
    }
    settle(&mut s, |s| s.state().results().unwrap().preview().is_none()
        && s.state().notices().iter().any(|n| n.contains("search source changed")));
    assert!(s.state().results().unwrap().is_stale(1));
    assert!(s.state().current_source().is_none(), "a refused preview must not navigate or publish current text");
    assert_eq!(std::fs::read(root.join("b.rs")).unwrap(), b"// needle, changed\n");
    s.dispatch(Command::RerunSearch);
    settle(&mut s, searched);
    s.dispatch(Command::SelectResult(1));
    settle(&mut s, |s| s.state().results().unwrap().preview().is_some_and(|p| p.index == 1 && p.highlights().is_some() && p.doc.text().contains("changed")));
}

#[test]
fn changed_targets_are_stale_and_are_not_opened_when_they_no_longer_match() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    tree(&root, &["a.rs"]);
    std::fs::write(root.join("a.rs"), "target here\n").unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, scanned);
    s.dispatch(Command::Search { query: "target".into(), scope: SearchIn::Project });
    settle(&mut s, searched);
    std::fs::write(root.join("a.rs"), "renamed here\n").unwrap();
    settle(&mut s, |s| s.state().results().unwrap().is_stale(0));
    s.dispatch(Command::OpenResult(0));
    settle(&mut s, |s| s.state().notices().iter().any(|n| n.starts_with("This result no longer matches")));
    assert!(s.state().current_source().is_none(), "the view stays where it was");
    s.dispatch(Command::RerunSearch);
    settle(&mut s, searched);
    assert!(s.state().results().unwrap().hits().is_empty());
}

#[test]
fn file_search_limits_cancellation_and_root_replacement() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "big.txt", "x x\n".repeat(6000).as_bytes());
    let mut s = session(d.path());
    assert_eq!(s.disabled(CommandId::FindInFile), Some("No file is open"));
    open(&mut s, Command::Open(a.clone()), "big.txt");
    s.dispatch(Command::Search { query: "X".into(), scope: SearchIn::ActiveFile });
    settle(&mut s, searched);
    let r = s.state().results().unwrap();
    assert_eq!((r.hits().len(), r.limited()), (search::RESULT_LIMIT, true), "frequent matches stop at the limit");
    // A newer search replaces it; closing stops it.
    s.dispatch(Command::Search { query: "absent".into(), scope: SearchIn::ActiveFile });
    settle(&mut s, searched);
    assert!(s.state().results().unwrap().hits().is_empty());
    s.dispatch(Command::CloseResults);
    assert!(s.state().results().is_none());
    assert_eq!(s.disabled(CommandId::RerunSearch), Some("No search results"));
    // Project results go with their root.
    let root = d.path().join("p");
    tree(&root, &["f.rs"]);
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, scanned);
    s.dispatch(Command::Search { query: "f".into(), scope: SearchIn::Project });
    s.dispatch(Command::OpenRoot(d.path().join("p")));
    assert!(s.state().results().is_none());
}

/// First results and completion over the pinned Lenz sources:
/// `READER_CORPUS=<dir> cargo test --release -p reader-core -- --ignored --nocapture search_perf`.
#[test]
#[ignore = "timing: needs a corpus; run in release, alone"]
fn search_perf() {
    let Ok(corpus) = std::env::var("READER_CORPUS") else {
        eprintln!("skipped: READER_CORPUS not set");
        return;
    };
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(PathBuf::from(&corpus)));
    settle(&mut s, scanned);
    let files = s.state().project().unwrap().files();
    for (query, label) in [("fn ", "frequent"), ("theme", "ordinary"), ("zzqqxxnotpresent", "absent")] {
        let (mut first, mut complete) = (Vec::new(), Vec::new());
        for round in 0..101 {
            let started = std::time::Instant::now();
            s.dispatch(Command::Search { query: query.into(), scope: SearchIn::Project });
            let rx = s.completions();
            let mut first_at = None;
            while !searched(&s) {
                s.dispatch(rx.recv_blocking_timeout().expect("search"));
                if first_at.is_none() && s.state().results().is_some_and(|r| !r.hits().is_empty()) {
                    first_at = Some(started.elapsed());
                }
            }
            if round > 0 {
                complete.push(started.elapsed().as_secs_f64() * 1000.0);
                if let Some(f) = first_at { first.push(f.as_secs_f64() * 1000.0); }
            }
        }
        let r = s.state().results().unwrap();
        let med = |v: &mut Vec<f64>| { v.sort_by(f64::total_cmp); v.get(v.len() / 2).copied().unwrap_or(f64::NAN) };
        eprintln!("READER_PERF {{\"name\":\"reader/search-{label}-complete\",\"unit\":\"ms\",\"samples\":{complete:?}}}");
        eprintln!("READER_PERF {{\"name\":\"reader/search-{label}-first\",\"unit\":\"ms\",\"samples\":{first:?}}}");
        eprintln!("{label} {query:?} over {files} files: {} results{}, first median {:.1} ms, complete median {:.1} ms (100 runs)",
            r.hits().len(), if r.limited() { " (limit)" } else { "" }, med(&mut first), med(&mut complete));
    }
}

#[test]
fn a_late_result_check_does_not_move_the_view_after_another_navigation() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    tree(&root, &["a.rs", "b.rs"]);
    std::fs::write(root.join("a.rs"), "needle\n").unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, scanned);
    s.dispatch(Command::Search { query: "needle".into(), scope: SearchIn::Project });
    settle(&mut s, searched);
    s.dispatch(Command::OpenResult(0));
    // Before the check lands, the reader opens something else.
    open(&mut s, Command::Open(root.join("b.rs")), "b.rs");
    std::thread::sleep(Duration::from_millis(100));
    while let Ok(c) = s.completions().try_recv() {
        s.dispatch(c);
    }
    assert!(s.state().current_source().unwrap().source().ends_with("b.rs"));
}

#[test]
fn file_search_reruns_and_validates_against_the_held_document() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "a.txt", b"one needle\n");
    let b = write(d.path(), "b.txt", b"no match\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(a.clone()), "a.txt");
    s.dispatch(Command::Search { query: "needle".into(), scope: SearchIn::ActiveFile });
    settle(&mut s, searched);
    open(&mut s, Command::OpenRetained(b), "b.txt");
    s.dispatch(Command::RerunSearch);
    settle(&mut s, searched);
    assert_eq!(s.state().results().unwrap().hits().len(), 1, "the same file, not the active one");
    // The file disappears; the reader still holds its content, so the result opens from it.
    let a_tab = s.state().tabs()[0].id();
    s.dispatch(Command::ActivateTab(a_tab));
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source().ends_with("a.txt")));
    std::fs::remove_file(&a).unwrap();
    settle(&mut s, |s| s.state().active().unwrap().unavailable().is_some());
    s.dispatch(Command::OpenResult(0));
    settle(&mut s, |s| s.state().selection().is_some());
    assert_eq!(s.state().selection().unwrap().range(), 4..10);
}

#[test]
fn unreadable_files_and_impossible_searches_are_reported() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    tree(&root, &["a.rs", "b.rs"]);
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, scanned);
    std::fs::remove_file(root.join("b.rs")).unwrap();
    s.dispatch(Command::Search { query: "a".into(), scope: SearchIn::Project });
    settle(&mut s, |s| searched(s) && s.state().results().unwrap().failed() == 1);
    // Changes reported before a file's results arrive still mark them stale.
    s.dispatch(Command::Search { query: "rs".into(), scope: SearchIn::Project });
    let id_paths = root.join("a.rs");
    s.dispatch(Completion::Changed(crate::watch::Change { paths: [id_paths].into(), ..Default::default() }));
    settle(&mut s, searched);
    let r = s.state().results().unwrap();
    assert!((0..r.hits().len()).all(|i| !r.hits()[i].path.ends_with("a.rs") || r.is_stale(i)));
}

#[test]
fn previews_reuse_the_snapshot_for_results_in_the_same_file() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    tree(&root, &["a.rs"]);
    std::fs::write(root.join("a.rs"), "x\nx\nx\n").unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, scanned);
    s.dispatch(Command::Search { query: "x".into(), scope: SearchIn::Project });
    settle(&mut s, |s| searched(s) && s.state().results().unwrap().preview().is_some());
    let first = s.state().results().unwrap().preview().unwrap().doc.clone();
    s.dispatch(Command::SelectResult(2));
    let second = s.state().results().unwrap().preview().unwrap().doc.clone();
    assert!(Arc::ptr_eq(&first, &second), "no read per arrow press within a file");
}

fn indexed(s: &Session) -> bool {
    let c = s.state().index_coverage();
    scanned(s) && c.candidates > 0 && c.pending == 0
}

/// Deliver only explicit watch events, so a precise OS event cannot hide a coarse-event regression.
fn settle_without_watch(s: &mut Session, done: impl Fn(&Session) -> bool) {
    let rx = s.completions();
    let until = std::time::Instant::now() + Duration::from_secs(10);
    while !done(s) {
        assert!(std::time::Instant::now() < until, "timed out");
        if let Ok(c) = rx.recv_blocking_timeout() && !matches!(c, Completion::Changed(_)) {
            s.dispatch(c);
        }
    }
}

#[test]
fn coarse_changes_refresh_closed_files_and_an_open_declaration_search() {
    for overflow in [false, true] {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("project");
        tree(&root, &["app/a.rb", "app/gone.rb", "app/move.rb", "apple/stable.rb"]);
        std::fs::write(root.join("app/a.rb"), "class Alpha; end\n").unwrap();
        std::fs::write(root.join("app/gone.rb"), "class Removed; end\n").unwrap();
        std::fs::write(root.join("app/move.rb"), "class Moved; end\n").unwrap();
        std::fs::write(root.join("apple/stable.rb"), "class Stable; end\n").unwrap();
        let mut s = session(d.path());
        s.dispatch(Command::OpenRoot(root.clone()));
        settle_without_watch(&mut s, indexed);
        s.dispatch(Command::FindDeclarations("".into()));
        settle_without_watch(&mut s, searched);
        assert_eq!(s.state().results().unwrap().hits().len(), 4);
        let stable = s.state.index.snapshot().into_iter().find(|(p, _)| &**p == "apple/stable.rb").unwrap().1;
        std::fs::write(root.join("app/a.rb"), "class Omega; end\n").unwrap();
        std::fs::remove_file(root.join("app/gone.rb")).unwrap();
        std::fs::rename(root.join("app/move.rb"), root.join("app/renamed.rb")).unwrap();
        std::fs::write(root.join("app/new.rb"), "class Added; end\n").unwrap();
        s.dispatch(Completion::Changed(Change { paths: if overflow { Default::default() } else { [root.join("app")].into() }, overflow, ..Default::default() }));
        settle_without_watch(&mut s, |s| indexed(s) && !s.state().project().unwrap().refreshing() && searched(s));
        let mut names: Vec<_> = s.state().results().unwrap().hits().iter().map(|h| (h.path.to_string(), h.text.to_string())).collect();
        names.sort();
        assert_eq!(names, [("app/a.rb".into(), "Omega".into()), ("app/new.rb".into(), "Added".into()), ("app/renamed.rb".into(), "Moved".into()), ("apple/stable.rb".into(), "Stable".into())], "overflow={overflow}");
        if !overflow {
            let after = s.state.index.snapshot().into_iter().find(|(p, _)| &**p == "apple/stable.rb").unwrap().1;
            assert!(Arc::ptr_eq(&stable, &after), "app must not invalidate apple");
        }
        // A deleted directory cannot be recognized by is_dir(); the old inventory still identifies it.
        std::fs::remove_dir_all(root.join("app")).unwrap();
        s.dispatch(Completion::Changed(Change { paths: [root.join("app")].into(), structure: true, ..Default::default() }));
        settle_without_watch(&mut s, |s| indexed(s) && !s.state().project().unwrap().refreshing() && searched(s));
        assert_eq!(s.state().results().unwrap().hits().iter().map(|h| h.text.as_ref()).collect::<Vec<_>>(), ["Stable"]);
    }
}

#[test]
fn coarse_changes_reject_a_chunk_and_ranking_started_before_the_change() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project");
    tree(&root, &["a.rb"]);
    std::fs::write(root.join("a.rb"), "class Alpha; end\n").unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle_without_watch(&mut s, indexed);
    s.dispatch(Command::FindDeclarations("Alpha".into()));
    let old_search = s.state().results().unwrap().id;
    let rel: Arc<str> = "a.rb".into();
    s.state.index.invalidate(&rel, true);
    assert_eq!(s.state.index.next_chunk().as_slice(), std::slice::from_ref(&rel));
    let old_index = index::index_file(&root, &rel, || true);
    let generation = s.state().project().unwrap().generation();
    for name in ["Omega", "Sigma"] {
        std::fs::write(root.join("a.rb"), format!("class {name}; end\n")).unwrap();
        s.dispatch(Completion::Changed(Change { overflow: true, ..Default::default() }));
    }
    assert_ne!(s.state().results().unwrap().id, old_search, "an old ranking must not publish after invalidation");
    s.dispatch(Completion::Indexed { generation, revision: s.index_revision.load(Ordering::Relaxed), files: vec![(rel, old_index)] });
    assert!(s.state.index.snapshot().is_empty(), "the old worker result must be rejected");
    settle_without_watch(&mut s, |s| indexed(s) && !s.state().project().unwrap().refreshing() && searched(s));
    assert!(s.state().results().unwrap().hits().is_empty());
    s.dispatch(Command::FindDeclarations("Sigma".into()));
    settle_without_watch(&mut s, searched);
    assert_eq!(s.state().results().unwrap().hits().len(), 1);
}

#[test]
fn watch_registration_checks_missed_edits_without_rebuilding_unchanged_records() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project");
    tree(&root, &["a.rb", "stable.rb", "gone.rb"]);
    std::fs::write(root.join("a.rb"), "class Alpha; end\n").unwrap();
    std::fs::write(root.join("stable.rb"), "class Stable; end\n").unwrap();
    let mut s = session(d.path());
    s.watcher = Watcher::new(|_| true); // only the explicit registration below reaches this session
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, indexed);
    let stable = s.state.index.snapshot().into_iter().find(|(p, _)| &**p == "stable.rb").unwrap().1;
    std::fs::write(root.join("a.rb"), "class OmegaChanged; end\n").unwrap();
    std::fs::remove_file(root.join("gone.rb")).unwrap();
    std::fs::write(root.join("new.rb"), "class Added; end\n").unwrap();
    s.dispatch(Command::FindDeclarations("OmegaChanged".into()));
    s.dispatch(Completion::Changed(Change { registered: [root].into(), ..Default::default() }));
    settle(&mut s, |s| indexed(s) && !s.state().project().unwrap().refreshing() && s.state().results().is_some_and(|r| r.done() && r.hits().len() == 1));
    let records = s.state.index.snapshot();
    assert_eq!(records.len(), 3);
    assert!(Arc::ptr_eq(&stable, &records.iter().find(|(p, _)| &**p == "stable.rb").unwrap().1));
    assert!(records.iter().any(|(p, _)| &**p == "new.rb"));
}

#[test]
fn the_outline_lists_declarations_in_order_and_opens_one_as_a_step() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "a.rs", "mod m {\n    pub struct Café;\n    fn inner() {}\n}\nfn outer() {}\n".as_bytes());
    let mut s = session(d.path());
    assert_eq!(s.disabled(CommandId::Outline), Some("No file is open"));
    open(&mut s, Command::Open(a), "a.rs");
    s.dispatch(Command::Outline);
    settle(&mut s, searched);
    let r = s.state().results().unwrap();
    assert_eq!(r.kind(), "Outline (syntactic candidates)");
    let got: Vec<(String, usize, u16)> = r.hits().iter().map(|h| (h.text.to_string(), h.line, h.decl.unwrap().1)).collect();
    assert_eq!(got, [("m".into(), 0, 0), ("Café".into(), 1, 1), ("inner".into(), 2, 1), ("outer".into(), 4, 0)]);
    assert!(!r.syntax_errors());
    s.dispatch(Command::OpenResult(1));
    settle(&mut s, |s| s.state().selection().is_some());
    let sel = s.state().selection().unwrap().range();
    assert_eq!(&source_text(s.state().current_source().unwrap())[sel], "Café");
    // Incomplete source is outlined as far as it parses, and says so.
    let b = write(d.path(), "b.rs", b"fn ok() {}\nfn broken( {\n");
    open(&mut s, Command::Open(b), "b.rs");
    s.dispatch(Command::Outline);
    settle(&mut s, searched);
    assert!(s.state().results().unwrap().syntax_errors());
    let txt = write(d.path(), "notes.txt", b"plain\n");
    open(&mut s, Command::Open(txt), "notes.txt");
    assert_eq!(s.disabled(CommandId::Outline), Some("No outline for this language"));
}

#[test]
fn declaration_search_covers_the_project_and_follows_changes() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    tree(&root, &["src/lib.rs", "app.py", "web/x.ts", "README.md"]);
    std::fs::write(root.join("src/lib.rs"), "pub struct Widget;\nimpl Widget { pub fn render(&self) {} }\n").unwrap();
    std::fs::write(root.join("app.py"), "class WidgetFactory:\n    def build(self): pass\n").unwrap();
    std::fs::write(root.join("web/x.ts"), "export function renderWidget() {}\n").unwrap();
    std::fs::write(root.join("big.rs"), "fn x() {}\n".repeat(crate::document::LARGE_LINES + 1)).unwrap();
    std::fs::write(root.join("web/s.css"), ".wide,\n.tall { color: red }\n").unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    // Asked while the project is still being indexed: once indexing settles, the query covers every file.
    s.dispatch(Command::FindDeclarations("widget".into()));
    settle(&mut s, |s| indexed(s) && searched(s) && s.state().results().unwrap().hits().len() == 3);
    let c = s.state().index_coverage();
    assert_eq!((c.candidates, c.indexed, c.too_large, c.complete()), (5, 4, 1, true), "README is not a candidate; the large file is counted, not parsed");
    let r = s.state().results().unwrap();
    assert_eq!(r.kind(), "Declarations (syntactic candidates)");
    let names: Vec<String> = r.hits().iter().map(|h| h.text.to_string()).collect();
    assert_eq!(names[0], "Widget", "the closest name first: {names:?}");
    assert!(names.contains(&"WidgetFactory".to_string()) && names.contains(&"renderWidget".to_string()));
    assert!(r.coverage().unwrap().complete());
    // A renamed declaration: the old name leaves the index, the new one arrives.
    std::fs::write(root.join("app.py"), "class GadgetFactory:\n    pass\n").unwrap();
    settle(&mut s, |s| { let c = s.state().index_coverage(); c.pending == 0 && c.indexed == 4 } );
    let mut found = Vec::new();
    // The watcher reports a busy folder a second into a burst; a full parallel test run takes up to about two.
    let until = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < until {
        s.dispatch(Command::FindDeclarations("factory".into()));
        settle(&mut s, searched);
        found = s.state().results().unwrap().hits().iter().map(|h| h.text.to_string()).collect();
        if found == ["GadgetFactory"] { break; }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(found, ["GadgetFactory"]);
    // Opening a declaration result validates it and moves there.
    s.dispatch(Command::FindDeclarations("render".into()));
    settle(&mut s, searched);
    let i = s.state().results().unwrap().hits().iter().position(|h| &*h.text == "render").unwrap();
    s.dispatch(Command::OpenResult(i));
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source().ends_with("lib.rs")) && s.state().selection().is_some());
    let sel = s.state().selection().unwrap().range();
    assert_eq!(&source_text(s.state().current_source().unwrap())[sel], "render");
    // A name spanning lines (a selector list) is shown and validated by its first line.
    s.dispatch(Command::FindDeclarations("wide".into()));
    settle(&mut s, searched);
    let i = s.state().results().unwrap().hits().iter().position(|h| &*h.text == ".wide,").unwrap();
    s.dispatch(Command::OpenResult(i));
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source().ends_with("s.css")) && s.state().selection().is_some());
    let sel = s.state().selection().unwrap().range();
    assert_eq!(&source_text(s.state().current_source().unwrap())[sel], ".wide,");
    // A deleted large file leaves the coverage counts.
    std::fs::remove_file(root.join("big.rs")).unwrap();
    settle(&mut s, |s| s.state().index_coverage().candidates == 4);
    assert_eq!(s.state().index_coverage().too_large, 0);
}

#[test]
fn a_new_root_drops_the_old_index() {
    let d = tempfile::tempdir().unwrap();
    let (a, b) = (d.path().join("a"), d.path().join("b"));
    tree(&a, &["one.rs"]);
    tree(&b, &["two.rs"]);
    std::fs::write(a.join("one.rs"), "fn alpha() {}\n").unwrap();
    std::fs::write(b.join("two.rs"), "fn beta() {}\n").unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(a));
    s.dispatch(Command::OpenRoot(b));
    settle(&mut s, indexed);
    s.dispatch(Command::FindDeclarations("a".into()));
    settle(&mut s, searched);
    let names: Vec<String> = s.state().results().unwrap().hits().iter().map(|h| h.text.to_string()).collect();
    assert_eq!(names, ["beta"]);
}

/// Indexing a large project while the reader stays responsive:
/// `READER_CORPUS=<dir> cargo test --release -p reader-core -- --ignored --nocapture index_perf`.
#[test]
#[ignore = "timing: needs a corpus; run in release, alone"]
fn index_perf() {
    let Ok(corpus) = std::env::var("READER_CORPUS") else {
        eprintln!("skipped: READER_CORPUS not set");
        return;
    };
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    let started = std::time::Instant::now();
    s.dispatch(Command::OpenRoot(PathBuf::from(&corpus)));
    // While indexing runs, opening files and ranking stay quick.
    let mut interactions = Vec::new();
    let rx = s.completions();
    while !indexed(&s) {
        if let Ok(c) = rx.recv_blocking_timeout() { s.dispatch(c); }
        if scanned(&s) && interactions.len() < 200 {
            let t = std::time::Instant::now();
            s.dispatch(Command::FindDeclarations("new".into()));
            interactions.push(t.elapsed().as_secs_f64() * 1000.0);
        }
    }
    let took = started.elapsed();
    let c = s.state().index_coverage();
    let mut ranks = Vec::new();
    for q in ["new", "render", "theme", "zzzq", "a"].iter().cycle().take(500) {
        let t = std::time::Instant::now();
        s.dispatch(Command::FindDeclarations(q.to_string()));
        while !searched(&s) { s.dispatch(rx.recv_blocking_timeout().expect("ranked")); }
        ranks.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    for v in [&mut interactions, &mut ranks] { v.sort_by(f64::total_cmp); }
    let p = |v: &[f64], q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
    eprintln!("READER_PERF {{\"name\":\"reader/declarations-query\",\"unit\":\"ms\",\"samples\":{ranks:?}}}");
    eprintln!("indexed {} of {} files ({} too large, {} with syntax errors) in {:.0} ms; records {:.1} MiB", c.indexed, c.candidates, c.too_large, c.syntax_errors, took.as_secs_f64() * 1000.0, s.state().index.bytes() as f64 / 1048576.0);
    eprintln!("dispatch during indexing: median {:.3} ms, max {:.3} ms over {}", p(&interactions, 0.5), p(&interactions, 1.0), interactions.len());
    eprintln!("declaration query to results: median {:.2} ms, p95 {:.2} ms over {}", p(&ranks, 0.5), p(&ranks, 0.95), ranks.len());
}

#[test]
fn only_real_uri_schemes_are_virtual_sources() {
    for (path, virtual_source) in [("jdt://contents/java.base/x", true), ("fake-virtual:///a.java", true), ("C://Users/x", false), ("file:///tmp/a", false), ("/tmp/jdt://x", false), ("relative/a.rs", false), ("a1+b.c://x", true)] {
        assert_eq!(is_virtual(Path::new(path)), virtual_source, "{path}");
    }
    assert!(matches!(read(Path::new("jdt://contents/x")), Err(ReadError::Failed(e)) if e.contains("not a file")));
}

fn png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    use image::ImageEncoder;
    let pixels: Vec<u8> = (0..w * h).flat_map(|_| rgba).collect();
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out).write_image(&pixels, w, h, image::ExtendedColorType::Rgba8).unwrap();
    out
}

#[test]
fn images_follow_external_changes_and_keep_their_last_content_when_gone() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "a.png", &png(4, 3, [255, 0, 0, 255]));
    let mut s = session(d.path());
    s.dispatch(Command::Open(path.clone()));
    settle(&mut s, |s| s.state().current_source().is_some());
    let first = s.state().current_source().unwrap().clone();
    assert_eq!(first.image().unwrap().pixels.as_ref().unwrap().get_pixel(0, 0).0, [255, 0, 0, 255]);
    // Same size, other pixels (no text either way): a new version.
    std::fs::write(&path, png(4, 3, [0, 0, 255, 255])).unwrap();
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.key().version != first.key().version));
    let doc = s.state().current_source().unwrap();
    assert_eq!(doc.image().unwrap().pixels.as_ref().unwrap().get_pixel(0, 0).0, [0, 0, 255, 255]);
    // Deleted: the last image stays, said to be unavailable; nothing is written.
    std::fs::remove_file(&path).unwrap();
    settle(&mut s, |s| s.state().notices().iter().any(|n| n.starts_with("Unavailable")));
    assert!(s.state().current_source().unwrap().image().unwrap().pixels.is_ok());
    assert!(!path.exists());
}

/// Release timing and memory for images: a 3840×2160 noise PNG (about 32 MiB) opened (read and decoded off the UI thread) until it
/// is shown, and resident memory over 100 cycles of opening two such images in turn in the preview tab.
#[test]
#[ignore = "timing: run in release, alone"]
fn image_perf() {
    let rss = memory_mib;
    let d = tempfile::tempdir().unwrap();
    // Noise: a photograph-like PNG (a flat one decodes far faster than real images).
    let noise = |name: &str, seed: u32| {
        use image::ImageEncoder;
        let mut x: u32 = seed;
        let pixels: Vec<u8> = (0..3840 * 2160 * 4).map(|i| if i % 4 == 3 { 255 } else { x ^= x << 13; x ^= x >> 17; x ^= x << 5; x as u8 }).collect();
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out).write_image(&pixels, 3840, 2160, image::ExtendedColorType::Rgba8).unwrap();
        write(d.path(), name, &out)
    };
    // Two of them, alternating: the inactive cache (32 MiB) holds at most one, so every open decodes.
    let (big, small) = (noise("big.png", 0x9E37_79B9), noise("other.png", 0x1234_5679));
    let mut s = session(d.path());
    let mut opens = Vec::new();
    let mut memory = Vec::new();
    let mut peak: f64 = 0.0;
    for i in 0..100 {
        for path in [&big, &small] {
            let t = std::time::Instant::now();
            s.dispatch(Command::Open(path.clone()));
            settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source() == path.as_path() && d.image().is_some_and(|i| i.pixels.is_ok())));
            if i >= 2 {
                opens.push(t.elapsed().as_secs_f64() * 1000.0);
            }
            peak = peak.max(rss().unwrap_or(0.0));
        }
        if i % 10 == 9 && let Some(m) = rss() {
            memory.push((m * 10.0).round() / 10.0);
        }
    }
    opens.sort_by(f64::total_cmp);
    eprintln!("READER_PERF {{\"name\":\"reader/image-4k-uhd-open-shown\",\"unit\":\"ms\",\"samples\":{opens:?}}}");
    eprintln!("4K UHD noise PNG open to shown: median {:.1} ms, p95 {:.1} ms over {}", opens[opens.len() / 2], opens[opens.len() * 95 / 100], opens.len());
    eprintln!("RSS every 10 cycles (MiB): {memory:?}; sampled peak {peak:.1} MiB; inactive cache {} B", s.inactive_bytes());
    let (first, last) = (memory[2], *memory.last().unwrap());
    assert!(last <= first + 16.0, "repeated opening is stable: {memory:?}");
}

#[test]
fn history_returns_to_images_like_any_source() {
    let d = tempfile::tempdir().unwrap();
    let image = write(d.path(), "a.png", &png(2, 2, [1, 2, 3, 255]));
    let text = write(d.path(), "b.txt", b"text\n");
    let mut s = session(d.path());
    for p in [&image, &text] {
        s.dispatch(Command::Open(p.clone()));
        settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source() == p.as_path()));
    }
    s.dispatch(Command::Back);
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source() == image.as_path() && d.image().is_some()));
    s.dispatch(Command::Forward);
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source() == text.as_path()));
}

fn rendered(s: &Session) -> Option<Arc<crate::markdown::Preview>> {
    match s.state().rendering()? {
        Rendering::Preview(p) => Some(p.clone()),
        _ => None,
    }
}

#[test]
fn markdown_opens_as_source_and_previews_on_request_in_place() {
    let d = tempfile::tempdir().unwrap();
    let text = "# Title\r\n\r\nFirst paragraph.\r\n\r\n```rust\r\nfn main() {}\r\n```\r\n\r\n## Later\r\n\r\nLast words.\r\n";
    let md = write(d.path(), "README.md", text.as_bytes());
    let plain = write(d.path(), "notes.txt", b"plain\n");
    let mut s = session(d.path());
    s.dispatch(Command::Open(plain.clone()));
    settle(&mut s, |s| s.state().current_source().is_some());
    assert_eq!(s.disabled(CommandId::MarkdownSource), Some("Not a Markdown file"));
    assert!(s.state().rendering().is_none());
    assert!(s.preview_control(0).is_none());
    s.dispatch(Command::Open(md.clone()));
    // The source first: no preview is made until asked for.
    settle(&mut s, |s| matches!(s.state().rendering(), Some(Rendering::Source)));
    assert!(s.state().active().unwrap().markdown_source());
    assert_eq!(s.preview_control(0).map(|a| (a.id, a.label, a.disabled)), Some((CommandId::MarkdownSource, "Preview", None)));
    assert!(s.preview_control(2).is_none());
    assert!(!s.toolbar().iter().any(|(id, _)| *id == CommandId::MarkdownSource));
    s.dispatch(Command::ToggleMarkdownSource);
    // The preview, made off the UI thread: its blocks map back to source lines, code blocks are highlighted.
    settle(&mut s, |s| rendered(s).is_some());
    let p = rendered(&s).unwrap();
    let doc = s.state().current_source().unwrap().clone();
    let lines: Vec<_> = p.doc.blocks.iter().map(|b| doc.line_of(b.source.start)).collect();
    assert_eq!(lines, [0, 2, 4, 8, 10]);
    assert!(p.syntax.get(&2).is_some_and(|s| !s.is_empty()), "the Rust block's keywords");
    assert_eq!(s.disabled(CommandId::MarkdownSource), None);
    assert_eq!(s.preview_control(0).unwrap().label, "Source");
    // Reading on in the preview and switching to the source keeps the place, without a history step.
    let back = s.state().can_back();
    s.dispatch(Command::Scroll(8));
    s.dispatch(Command::ToggleMarkdownSource);
    assert!(matches!(s.state().rendering(), Some(Rendering::Source)));
    assert_eq!((s.state().top_line(), s.state().can_back()), (8, back));
    s.dispatch(Command::ToggleMarkdownSource);
    settle(&mut s, |s| rendered(s).is_some());
    assert_eq!((s.state().top_line(), s.state().can_back()), (8, back));
    assert!(!s.state().active().unwrap().retained(), "the preview tab stays the preview tab");
    assert_eq!(std::fs::read(&md).unwrap(), text.as_bytes());
}

#[test]
fn markdown_previews_show_local_images_inside_the_folder_only() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("repo");
    std::fs::create_dir_all(root.join("docs/img")).unwrap();
    std::fs::write(root.join("docs/img/a.png"), png(3, 2, [9, 8, 7, 255])).unwrap();
    std::fs::write(root.join("docs/img/broken.png"), b"not a png").unwrap();
    std::fs::write(d.path().join("outside.png"), png(1, 1, [0, 0, 0, 255])).unwrap();
    let md = root.join("docs/readme.md");
    std::fs::write(&md, "![a](img/a.png)\n\n![b](img/broken.png)\n\n![c](../../outside.png)\n\n![d](img/missing.png)\n\n![e](https://example.com/e.png)\n").unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    s.dispatch(Command::Open(md.clone()));
    settle(&mut s, |s| s.state().current_source().is_some());
    s.dispatch(Command::ToggleMarkdownSource);
    settle(&mut s, |s| rendered(s).is_some_and(|p| p.images.is_some()));
    let p = rendered(&s).unwrap();
    let images = p.images.as_ref().unwrap();
    assert_eq!(images[&0].as_ref().unwrap().dimensions(), (3, 2));
    assert_eq!(images[&1].as_ref().err(), Some(&"cannot be decoded, or over the preview's image size limit"));
    for b in 2..5 {
        assert!(images[&b].as_ref().is_err_and(|e| e.contains("outside the folder")), "block {b}");
    }
    assert!(p.bytes() >= 3 * 2 * 4);
    // Switching to the source releases them.
    s.dispatch(Command::ToggleMarkdownSource);
    assert!(Arc::strong_count(&p) == 1 && images[&0].as_ref().is_ok_and(|i| Arc::strong_count(i) == 1), "only this test holds the preview");
}

#[test]
fn markdown_previews_follow_external_changes_and_say_when_they_cannot_be_shown() {
    let d = tempfile::tempdir().unwrap();
    let md = write(d.path(), "a.md", b"# One\n");
    let mut s = session(d.path());
    s.dispatch(Command::Open(md.clone()));
    settle(&mut s, |s| s.state().current_source().is_some());
    s.dispatch(Command::ToggleMarkdownSource);
    settle(&mut s, |s| rendered(s).is_some());
    let first = s.state().current_source().unwrap().key().version;
    // Replaced: the new version is previewed; an older version's preview arriving late is ignored.
    std::fs::write(&md, b"# Two\n\nMore.\n").unwrap();
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.key().version != first) && rendered(s).is_some());
    assert_eq!(rendered(&s).unwrap().doc.blocks.len(), 2);
    let stale = crate::markdown::render("# Old\n".into(), || true).unwrap();
    s.dispatch(Completion::Rendered { version: first, preview: Ok(stale) });
    assert_eq!(rendered(&s).unwrap().doc.blocks.len(), 2, "an obsolete version's preview is rejected");
    // Images decoded for another preview of this version (one replaced by a quick toggle) are rejected too.
    let version = s.state().current_source().unwrap().key().version;
    let other = crate::markdown::render("# Two\n\nMore.\n".into(), || true).unwrap().doc;
    s.dispatch(Completion::PreviewImages { version, doc: other, images: [(99, Err("foreign"))].into() });
    assert!(rendered(&s).unwrap().images.as_ref().is_none_or(|i| !i.contains_key(&99)));
    // Deleted: the last preview stays, said to be unavailable; nothing is written.
    std::fs::remove_file(&md).unwrap();
    settle(&mut s, |s| s.state().notices().iter().any(|n| n.starts_with("Unavailable")));
    assert_eq!(rendered(&s).unwrap().doc.blocks.len(), 2);
    assert!(!md.exists());
    // Beyond the parser's limits: the source, and why.
    let big = write(d.path(), "big.md", "word ".repeat(1 << 20).as_bytes());
    s.dispatch(Command::Open(big.clone()));
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source() == big));
    if s.state().active().unwrap().markdown_source() {
        s.dispatch(Command::ToggleMarkdownSource);
    }
    settle(&mut s, |s| s.state().notices().iter().any(|n| n.starts_with("Markdown preview unavailable")));
    assert!(matches!(s.state().rendering(), Some(Rendering::Source)));
    assert_eq!(s.state().notices().iter().find(|n| n.starts_with("Markdown")).unwrap(), "Markdown preview unavailable: its source bytes exceed the preview's limit; showing the source");
    assert_eq!(s.disabled(CommandId::MarkdownSource), Some("This document cannot be previewed (the notice says why)"));
    assert_eq!(s.preview_control(0).unwrap().disabled, s.disabled(CommandId::MarkdownSource));
}

/// Release timing and memory for Markdown: a 200 KiB document with code blocks and four local 512×512 images opened
/// until its preview and images are ready, alternating with a second one over 100 cycles, then 50 external rewrites
/// re-rendered in place.
#[test]
#[ignore = "timing: run in release, alone"]
fn markdown_perf() {
    let rss = memory_mib;
    let d = tempfile::tempdir().unwrap();
    for i in 0..4u8 {
        std::fs::write(d.path().join(format!("i{i}.png")), png(512, 512, [i * 40, 100, 200, 255])).unwrap();
    }
    let doc = |title: &str| {
        let mut s = format!("# {title}\n\n");
        while s.len() < 200 << 10 {
            s.push_str("## Section\n\nSome *emphasis*, `code` and a [link](https://example.com) in a paragraph that goes on for a while.\n\n- one\n- two\n  - nested\n\n```rust\nfn main() { let x = 1; println!(\"{x}\"); }\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n");
        }
        s + "![a](i0.png)\n\n![b](i1.png)\n\n![c](i2.png)\n\n![d](i3.png)\n"
    };
    let (a, b) = (write(d.path(), "a.md", doc("A").as_bytes()), write(d.path(), "b.md", doc("B").as_bytes()));
    let mut s = session(d.path());
    let ready = |s: &Session, path: &Path| s.state().current_source().is_some_and(|d| d.source() == path) && rendered(s).is_some_and(|p| p.images.is_some());
    // The preview tab shows previews from here on (its presentation state stays with it).
    s.dispatch(Command::Open(a.clone()));
    settle(&mut s, |s| s.state().current_source().is_some());
    s.dispatch(Command::ToggleMarkdownSource);
    let (mut opens, mut memory, mut peak) = (Vec::new(), Vec::new(), 0.0f64);
    for i in 0..100 {
        for path in [&a, &b] {
            let t = std::time::Instant::now();
            s.dispatch(Command::Open(path.clone()));
            settle(&mut s, |s| ready(s, path));
            if i >= 2 {
                opens.push(t.elapsed().as_secs_f64() * 1000.0);
            }
            peak = peak.max(rss().unwrap_or(0.0));
        }
        if i % 10 == 9 && let Some(m) = rss() {
            memory.push((m * 10.0).round() / 10.0);
        }
    }
    let p = rendered(&s).unwrap();
    assert_eq!((p.images.as_ref().unwrap().values().filter(|i| i.is_ok()).count(), p.syntax.len() > 100), (4, true));
    let (mut refreshes, mut after) = (Vec::new(), Vec::new());
    let rewrites: usize = std::env::var("READER_MD_REWRITES").ok().and_then(|n| n.parse().ok()).unwrap_or(50);
    for i in 0..rewrites {
        let version = s.state().current_source().unwrap().key().version;
        let t = std::time::Instant::now();
        std::fs::write(&b, doc(&format!("B{i}"))).unwrap();
        settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.key().version != version) && rendered(s).is_some_and(|p| p.images.is_some()));
        refreshes.push(t.elapsed().as_secs_f64() * 1000.0);
        if i % 10 == 9 && let Some(m) = rss() {
            after.push((m * 10.0).round() / 10.0);
        }
    }
    for v in [&mut opens, &mut refreshes] {
        v.sort_by(f64::total_cmp);
    }
    eprintln!("READER_PERF {{\"name\":\"reader/markdown-200k-open-previewed\",\"unit\":\"ms\",\"samples\":{opens:?}}}");
    eprintln!("READER_PERF {{\"name\":\"reader/markdown-200k-refresh-previewed\",\"unit\":\"ms\",\"samples\":{refreshes:?}}}");
    eprintln!("200 KiB Markdown, 4 images, open to previewed: median {:.1} ms, p95 {:.1} ms over {}", opens[opens.len() / 2], opens[opens.len() * 95 / 100], opens.len());
    eprintln!("external rewrite to re-previewed (includes the watcher's coalescing): median {:.1} ms, p95 {:.1} ms over {}", refreshes[refreshes.len() / 2], refreshes[refreshes.len() * 95 / 100], refreshes.len());
    eprintln!("RSS every 10 cycles (MiB): {memory:?}; sampled peak {peak:.1} MiB; every 10 rewrites: {after:?}");
    let (first, last) = (memory[2], *memory.last().unwrap());
    assert!(last <= first + 16.0, "repeated opening is stable: {memory:?}");
    assert!(after.last().unwrap() <= &(after[0] + 16.0), "repeated refreshing is stable: {after:?}");
}

/// Launch-acceptance component workloads on a pinned corpus (`READER_CORPUS`, named by `READER_CORPUS_NAME`): an
/// ordinary file (≤1 MiB) opened in a fresh session (empty reader cache) and again from the inactive cache, 100
/// each; cached tab switches and Back/Forward, and fuzzy file results, 1,000 samples each after warm-up; project
/// text search first results and completion, 100 per query. Each reports p95 against its target; the text
/// search completion target is `READER_SEARCH_TARGET_MS` (2,000 for Lenz, 5,000 for Metabase).
#[test]
#[ignore = "timing: needs a corpus; run in release, alone"]
fn acceptance_perf() {
    let Ok(corpus) = std::env::var("READER_CORPUS") else {
        eprintln!("skipped: READER_CORPUS not set");
        return;
    };
    let name = std::env::var("READER_CORPUS_NAME").unwrap_or_else(|_| "corpus".into());
    let search_target: f64 = std::env::var("READER_SEARCH_TARGET_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(2000.0);
    let root = PathBuf::from(&corpus);
    let p95 = |v: &mut Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v[((v.len() - 1) as f64 * 0.95).round() as usize]
    };
    let mut verdicts = Vec::new();
    let mut report = |metric: &str, mut samples: Vec<f64>, target: f64| {
        eprintln!("READER_PERF {{\"name\":\"reader/{name}/{metric}\",\"unit\":\"ms\",\"samples\":{samples:?}}}");
        let n = samples.len();
        let within = samples.iter().filter(|&&x| x <= target).count() as f64 / n as f64;
        let p = p95(&mut samples);
        verdicts.push(format!("{metric}: p95 {p:.1} ms, {:.1}% within {target} ms over {n} — {}", within * 100.0, if within >= 0.95 { "PASS" } else { "FAIL" }));
    };
    // The corpus as the reader scans it.
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, scanned);
    let files: Vec<PathBuf> = {
        let mut out = Vec::new();
        let mut dirs = vec![root.clone()];
        while let Some(dir) = dirs.pop() {
            for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let p = e.path();
                if p.file_name().is_some_and(|n| n == ".git") {
                    continue;
                }
                if p.is_dir() { dirs.push(p) } else { out.push(p) }
            }
        }
        out.sort();
        out
    };
    let bytes: u64 = files.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum();
    eprintln!("corpus {name}: {corpus}; {} files listed by the reader ({} skipped), {} files and {:.1} MiB on disk outside .git", s.state().project().unwrap().files(), s.state().project().unwrap().skipped(), files.len(), bytes as f64 / 1048576.0);
    // Ordinary source files up to 1 MiB: text, not binary, spread over the corpus.
    let ordinary: Vec<PathBuf> = files
        .iter()
        .filter(|p| p.extension().is_some_and(|e| ["rs", "py", "ts", "tsx", "js", "clj", "cljs", "cljc", "java", "kt", "md", "css", "html", "yaml", "yml", "json", "toml", "rb", "sql"].contains(&e.to_str().unwrap_or_default())))
        .filter(|p| std::fs::metadata(p).is_ok_and(|m| m.len() > 0 && m.len() <= 1 << 20))
        .cloned()
        .collect();
    let step = (ordinary.len() / 100).max(1);
    let picks: Vec<&PathBuf> = ordinary.iter().step_by(step).take(100).collect();
    let largest = ordinary.iter().map(|p| std::fs::metadata(p).map_or(0, |m| m.len())).max().unwrap_or(0);
    eprintln!("open set: {} ordinary files (largest candidate {:.0} KiB)", picks.len(), largest as f64 / 1024.0);
    // Fresh reader cache: a new session per open.
    let mut fresh = Vec::new();
    for p in &picks {
        let d = tempfile::tempdir().unwrap();
        let mut s = session(d.path());
        let started = std::time::Instant::now();
        s.dispatch(Command::Open((*p).clone()));
        settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source() == p.as_path()) && s.state().loading().is_none());
        fresh.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    report("open-fresh-cache", fresh, 100.0);
    // From the inactive cache: each file again in the preview tab.
    let mut cached = Vec::new();
    for round in 0..2 {
        for p in &picks {
            let started = std::time::Instant::now();
            s.dispatch(Command::Open((*p).clone()));
            settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source() == p.as_path()) && s.state().loading().is_none());
            if round == 1 {
                cached.push(started.elapsed().as_secs_f64() * 1000.0);
            }
        }
    }
    report("open-cached", cached, 100.0);
    // Ten retained tabs: switches and history steps.
    for p in picks.iter().take(10) {
        s.dispatch(Command::OpenRetained((*p).clone()));
        settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source() == p.as_path()) && s.state().loading().is_none());
    }
    let ids: Vec<_> = s.state().tabs().iter().filter(|t| t.retained()).map(|t| t.id()).collect();
    let mut switches = Vec::new();
    for i in 0..1100 {
        let id = ids[i % ids.len()];
        let started = std::time::Instant::now();
        s.dispatch(Command::ActivateTab(id));
        settle(&mut s, |s| s.state().active().is_some_and(|t| t.id() == id) && s.state().current_source().is_some() && s.state().loading().is_none());
        if i >= 100 {
            switches.push(started.elapsed().as_secs_f64() * 1000.0);
        }
    }
    report("tab-switch-cached", switches, 50.0);
    let mut history = Vec::new();
    for i in 0..1100 {
        let started = std::time::Instant::now();
        s.dispatch(if i % 2 == 0 { Command::Back } else { Command::Forward });
        settle(&mut s, |s| s.state().current_source().is_some() && s.state().loading().is_none());
        if i >= 100 {
            history.push(started.elapsed().as_secs_f64() * 1000.0);
        }
    }
    report("history-step-cached", history, 50.0);
    // Fuzzy file results over the loaded inventory.
    let queries = ["main", "test", "src", "readme", "conf", "util", "x", "index", "mod", "zzqq"];
    let mut finder = Vec::new();
    for i in 0..1100 {
        s.dispatch(Command::CloseFinder);
        let started = std::time::Instant::now();
        s.dispatch(Command::FindFile(queries[i % queries.len()].to_string()));
        let rx = s.completions();
        while !s.state().project().unwrap().finder().unwrap().ready {
            s.dispatch(rx.recv_blocking_timeout().expect("ranked"));
        }
        if i >= 100 {
            finder.push(started.elapsed().as_secs_f64() * 1000.0);
        }
    }
    s.dispatch(Command::CloseFinder);
    report("finder", finder, 50.0);
    // Project text search: first results and completion (or the explicit limit).
    for (query, label) in [("fn ", "frequent"), ("theme", "ordinary"), ("zzqqxxnotpresent", "absent")] {
        let (mut first, mut complete) = (Vec::new(), Vec::new());
        let mut limited = false;
        for round in 0..101 {
            let started = std::time::Instant::now();
            s.dispatch(Command::Search { query: query.into(), scope: SearchIn::Project });
            let rx = s.completions();
            let mut first_at = None;
            while !searched(&s) {
                s.dispatch(rx.recv_blocking_timeout().expect("search"));
                if first_at.is_none() && s.state().results().is_some_and(|r| !r.hits().is_empty()) {
                    first_at = Some(started.elapsed());
                }
            }
            if round > 0 {
                complete.push(started.elapsed().as_secs_f64() * 1000.0);
                first.extend(first_at.map(|f| f.as_secs_f64() * 1000.0));
            }
            limited = s.state().results().unwrap().limited();
        }
        eprintln!("search {label} {query:?}: {} results{}", s.state().results().unwrap().hits().len(), if limited { " (explicit limit)" } else { "" });
        if !first.is_empty() {
            report(&format!("search-{label}-first"), first, 250.0);
        }
        report(&format!("search-{label}-complete"), complete, search_target);
    }
    for v in &verdicts {
        eprintln!("ACCEPTANCE {name} {v}");
    }
}

/// The fixed-working-set stress run on a copy of a corpus (`READER_CORPUS`): ten retained tabs cycled, cached
/// switches and Back/Forward, fuzzy file queries, project searches, and one file rewritten externally on a cycle,
/// for `READER_STRESS_MINUTES` (default 15): five minutes of warm-up, then one-minute windows of settled RSS
/// (sampled every 5 s, the window's median). Correctness throughout: the rewritten file is shown as on disk,
/// search counts stay what the working set implies, and no other file's bytes change. The final three windows'
/// median must stay within the larger of 16 MiB or 5% of the first three's. A passing ceiling is not leak proof.
#[test]
#[ignore = "stress: needs a corpus; run in release, alone, for 15 minutes"]
fn stress_perf() {
    let Ok(corpus) = std::env::var("READER_CORPUS") else {
        eprintln!("skipped: READER_CORPUS not set");
        return;
    };
    let minutes: u64 = std::env::var("READER_STRESS_MINUTES").ok().and_then(|v| v.parse().ok()).unwrap_or(15);
    let rss = || memory_mib().unwrap_or(0.0);
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("corpus");
    assert!(std::process::Command::new("cp").args(["-a", &corpus, root.to_str().unwrap()]).status().unwrap().success());
    let _ = std::fs::remove_dir_all(root.join(".git"));
    let edit = root.join("stress_edit.rs");
    let versions = ["// stress A\nfn alpha() {}\n", "// stress B, longer\nfn beta() {}\nfn gamma() {}\n"];
    std::fs::write(&edit, versions[0]).unwrap();
    // The working set: ten source files besides the edited one.
    let mut sources: Vec<PathBuf> = Vec::new();
    let mut dirs = vec![root.clone()];
    while let Some(dir) = dirs.pop() {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = e.path();
            let source = p.extension().is_some_and(|x| ["rs", "clj", "cljs", "cljc", "ts", "tsx", "js", "py", "java", "kt", "rb"].contains(&x.to_str().unwrap_or_default()));
            if p.is_dir() { dirs.push(p) } else if source && p != edit && std::fs::metadata(&p).is_ok_and(|m| m.len() <= 1 << 20) { sources.push(p) }
        }
    }
    sources.sort();
    let working: Vec<PathBuf> = sources.iter().step_by((sources.len() / 10).max(1)).take(10).cloned().collect();
    let digest = |p: &Path| std::fs::read(p).map(|b| Document::fingerprint(&b)).unwrap_or(0);
    let before: Vec<u64> = working.iter().map(|p| digest(p)).collect();
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, scanned);
    for p in &working {
        open(&mut s, Command::OpenRetained(p.clone()), &p.file_name().unwrap().to_string_lossy());
    }
    open(&mut s, Command::OpenRetained(edit.clone()), "stress_edit.rs");
    let ids: Vec<_> = s.state().tabs().iter().map(|t| t.id()).collect();
    // What the searches must find: "theme" counted once from disk (the edited file never contains it).
    s.dispatch(Command::Search { query: "theme".into(), scope: SearchIn::Project });
    settle(&mut s, searched);
    let theme_hits = s.state().results().unwrap().hits().len();
    let started = std::time::Instant::now();
    // Five minutes of warm-up and one-minute windows for the full run (shorter runs scale them down).
    let warm = Duration::from_secs((5 * 60).min(minutes * 60 / 3));
    let window = Duration::from_secs(((minutes * 60 - warm.as_secs()) / 10).clamp(1, 60));
    let (mut windows, mut current, mut next_sample) = (Vec::<f64>::new(), Vec::<f64>::new(), started + warm);
    let (mut iterations, mut rewrites, mut window_end) = (0u64, 0u64, started + warm + window);
    let queries = ["main", "mod", "theme", "util", "zzqq"];
    // Until ten windows are settled (the run's minutes, plus the last window's closing).
    while windows.len() < 10 && started.elapsed() < Duration::from_secs(minutes * 60) + window * 2 {
        iterations += 1;
        let i = iterations as usize;
        let id = ids[i % ids.len()];
        s.dispatch(Command::ActivateTab(id));
        settle(&mut s, |s| s.state().active().is_some_and(|t| t.id() == id) && s.state().current_source().is_some() && s.state().loading().is_none());
        s.dispatch(if i.is_multiple_of(2) { Command::Back } else { Command::Forward });
        settle(&mut s, |s| s.state().loading().is_none());
        s.dispatch(Command::FindFile(queries[i % queries.len()].into()));
        settle(&mut s, |s| s.state().project().unwrap().finder().is_some_and(|f| f.ready));
        s.dispatch(Command::CloseFinder);
        if i.is_multiple_of(20) {
            s.dispatch(Command::Search { query: "theme".into(), scope: SearchIn::Project });
            settle(&mut s, searched);
            assert_eq!(s.state().results().unwrap().hits().len(), theme_hits, "search results stay what the files hold");
            s.dispatch(Command::CloseResults);
        }
        if i.is_multiple_of(50) {
            // An external rewrite of the edited file: shown as on disk once refreshed.
            rewrites += 1;
            let text = versions[(rewrites % 2) as usize];
            std::fs::write(&edit, text).unwrap();
            let tab = ids[ids.len() - 1];
            s.dispatch(Command::ActivateTab(tab));
            settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source() == edit && source_text(d) == text));
        }
        // Ownership: the tabs stay the working set, released content stays within the inactive cache's budget,
        // and nothing else holds documents.
        if i.is_multiple_of(100) {
            assert_eq!(s.state().tabs().len(), ids.len());
            assert!(s.inactive_bytes() <= crate::cache::INACTIVE_BYTES, "{}", s.inactive_bytes());
            assert!(s.cache_entries().len() <= ids.len());
        }
        let now = std::time::Instant::now();
        if now >= next_sample {
            current.push(rss());
            next_sample = now + Duration::from_secs(5);
        }
        if now >= window_end && !current.is_empty() {
            current.sort_by(f64::total_cmp);
            windows.push(current[current.len() / 2]);
            current.clear();
            window_end = now + window;
        }
    }
    let took = started.elapsed();
    let after: Vec<u64> = working.iter().map(|p| digest(p)).collect();
    assert_eq!(after, before, "no file of the working set changed");
    let med = |v: &[f64]| { let mut v = v.to_vec(); v.sort_by(f64::total_cmp); v[v.len() / 2] };
    eprintln!("READER_PERF {{\"name\":\"reader/stress-settled-rss-windows\",\"unit\":\"MiB\",\"samples\":{windows:?}}}");
    eprintln!("memory: {}", memory_kind());
    eprintln!("stress over {} files: {:.1} min ({} iterations, {rewrites} external rewrites), warm-up {} s, {} windows of {} s: {windows:?}", s.state().project().unwrap().files(), took.as_secs_f64() / 60.0, iterations, warm.as_secs(), windows.len(), window.as_secs());
    assert_eq!(windows.len(), 10, "ten windows: {windows:?}");
    let (first, last) = (med(&windows[..3]), med(&windows[windows.len() - 3..]));
    let allowed = 16.0f64.max(first * 0.05);
    eprintln!("first three windows' median {first:.1} MiB, final three {last:.1} MiB, allowed growth {allowed:.1} MiB");
    assert!(last <= first + allowed, "settled memory grew: {windows:?}");
    // No continuing upward trend: the final three windows are not each above every earlier window.
    let earlier_max = windows[..windows.len() - 3].iter().cloned().fold(f64::MIN, f64::max);
    assert!(windows[windows.len() - 3..].iter().any(|w| *w <= earlier_max), "a continuing upward trend: {windows:?}");
}

#[test]
fn a_folder_moved_in_while_browsing_is_watched_once_its_rescan_completes() {
    // The watched folders are cached per inventory; a navigation during the rescan a move causes (the file count
    // unchanged) must not pin the old set.
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    tree(&root, &["a.rs", "b.rs"]);
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, scanned);
    open(&mut s, Command::Open(root.join("a.rs")), "a.rs");
    std::fs::create_dir(root.join("new")).unwrap();
    std::fs::rename(root.join("b.rs"), root.join("new/b.rs")).unwrap();
    // A rescan starts, and the watched folders are asked for before its walk ends (as a navigation would).
    s.rescan();
    s.rewatch();
    settle(&mut s, |s| s.state().project().is_some_and(|p| p.files() == 2 && p.dirs().any(|d| d == "new") && !p.refreshing()));
    open(&mut s, Command::OpenRetained(root.join("a.rs")), "a.rs");
    // A file added inside the new folder is noticed only if that folder is watched.
    tree(&root, &["new/c.rs"]);
    settle(&mut s, |s| s.state().project().is_some_and(|p| p.files() == 3));
}

/// This process's memory in MiB: Linux resident size (`VmRSS`); on macOS its physical footprint (what the kernel
/// charges it, Activity Monitor's "Memory") through `proc_pid_rusage`, else its `ps` resident size, which also counts
/// freed pages not yet reclaimed. `None` when neither can be read. The `MEMORY_KIND` printed by the workloads says which.
fn memory_mib() -> Option<f64> {
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        return status.lines().find_map(|l| l.strip_prefix("VmRSS:")).and_then(|v| v.split_whitespace().next()?.parse::<f64>().ok()).map(|kib| kib / 1024.0);
    }
    let pid = std::process::id().to_string();
    // The footprint needs libproc; the core forbids unsafe code, so a short Python ctypes call reads it.
    const FOOTPRINT: &str = "import ctypes,sys\nclass R(ctypes.Structure): _fields_=[('uuid',ctypes.c_uint8*16)]+[(f'f{i}',ctypes.c_uint64) for i in range(40)]\nr=R(); lib=ctypes.CDLL('/usr/lib/libproc.dylib')\nsys.exit(1) if lib.proc_pid_rusage(int(sys.argv[1]),4,ctypes.byref(r)) else print(r.f7)";
    let footprint = std::process::Command::new("python3").args(["-c", FOOTPRINT, &pid]).output().ok().filter(|o| o.status.success()).and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<f64>().ok());
    footprint.map(|b| b / 1048576.0).or_else(|| {
        let out = std::process::Command::new("ps").args(["-o", "rss=", "-p", &pid]).output().ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse::<f64>().ok().map(|kib| kib / 1024.0)
    })
}

/// Which memory figure [`memory_mib`] reports here.
fn memory_kind() -> &'static str {
    if std::path::Path::new("/proc/self/status").exists() { "Linux VmRSS" } else { "macOS physical footprint (proc_pid_rusage), else ps resident size" }
}


#[test]
fn launched_in_a_folder_the_reader_opens_it_but_never_home_or_the_filesystem_root() {
    let home = PathBuf::from("/home/u");
    let at = |arg: Option<&str>, cwd: &str| launch_target(arg.map(PathBuf::from), Some(PathBuf::from(cwd)), Some(home.clone()));
    // An argument wins: a file or a folder, relative to where it was started.
    assert_eq!(at(Some("src/main.rs"), "/home/u/proj"), Some(PathBuf::from("/home/u/proj/src/main.rs")));
    assert_eq!(at(Some("/etc/hosts"), "/home/u/proj"), Some(PathBuf::from("/etc/hosts")));
    // No argument: the folder it was started in…
    assert_eq!(at(None, "/home/u/proj"), Some(PathBuf::from("/home/u/proj")));
    // …but not the home folder (where desktop launchers start apps) or the filesystem root.
    assert_eq!(at(None, "/home/u"), None);
    assert_eq!(at(None, "/home/u/"), None);
    assert_eq!(at(None, "/"), None);
    assert_eq!(launch_target(None, None, Some(home)), None);
}

#[test]
fn a_file_switched_to_its_preview_opens_as_a_preview_again() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "a.md", b"# A\n");
    let b = write(d.path(), "b.md", b"# B\n");
    let txt = write(d.path(), "c.txt", b"c\n");
    let mut s = session(d.path());
    open(&mut s, Command::Open(a.clone()), "a.md");
    s.dispatch(Command::ToggleMarkdownSource);
    settle(&mut s, |s| rendered(s).is_some());
    // The preview tab moves on to another file, then comes back (history): still a preview.
    open(&mut s, Command::Open(txt), "c.txt");
    s.dispatch(Command::Back);
    settle(&mut s, |s| rendered(s).is_some_and(|p| p.doc.text.contains('A')));
    // Another Markdown file opens as its source.
    open(&mut s, Command::Open(b), "b.md");
    assert!(matches!(s.state().rendering(), Some(Rendering::Source)));
    // Switching back to the source is remembered too.
    open(&mut s, Command::Open(a.clone()), "a.md");
    settle(&mut s, |s| rendered(s).is_some());
    s.dispatch(Command::ToggleMarkdownSource);
    open(&mut s, Command::Open(write(d.path(), "d.txt", b"d\n")), "d.txt");
    open(&mut s, Command::Open(a), "a.md");
    assert!(matches!(s.state().rendering(), Some(Rendering::Source)));
}

fn pane_names(s: &Session, pane: u8) -> Vec<String> {
    s.state().pane_tabs(pane).map(|t| t.title()).collect()
}

fn shown_name(s: &Session, pane: u8) -> Option<String> {
    s.state().shown(pane).map(|t| t.title())
}

#[test]
fn a_split_shows_the_source_twice_and_each_pane_keeps_its_tabs_and_place() {
    let d = tempfile::tempdir().unwrap();
    let text: String = (0..200).map(|i| format!("fn f{i}() {{}}\n")).collect();
    let a = write(d.path(), "a.rs", text.as_bytes());
    let b = write(d.path(), "b.rs", b"fn b() {}\n");
    let mut s = session(d.path());
    assert_eq!(s.disabled(CommandId::Unsplit), Some("The file panel is not split"));
    s.dispatch(Command::Open(a.clone()));
    settle(&mut s, opened);
    s.dispatch(Command::Scroll(50));
    s.dispatch(Command::Split(Split::Columns));
    assert_eq!((s.state().split(), s.state().focused_pane()), (Some(Split::Columns), 1));
    assert_eq!(s.disabled(CommandId::SplitRight), Some("Already split side by side"));
    let (left, right) = (s.state().shown(0).unwrap(), s.state().shown(1).unwrap());
    assert!(Arc::ptr_eq(s.state().view(left.id()).unwrap().snapshot(), s.state().view(right.id()).unwrap().snapshot()), "one snapshot for both");
    assert_eq!(right.top_line(), 50, "the new pane opens at the same place");
    // Each pane scrolls on its own; both stay coloured.
    s.dispatch(Command::Scroll(120));
    assert_eq!((s.state().shown(0).unwrap().top_line(), s.state().top_line()), (50, 120));
    settle(&mut s, |s| pane_view(s, 0).highlights().is_some() && pane_view(s, 1).highlights().is_some());
    // Opening in the focused pane leaves the other showing its file, in memory.
    s.dispatch(Command::Open(b.clone()));
    settle(&mut s, opened);
    assert_eq!((pane_names(&s, 0), pane_names(&s, 1)), (vec!["a.rs".to_string()], vec!["a.rs".to_string(), "b.rs".to_string()]));
    assert_eq!((shown_name(&s, 0).as_deref(), shown_name(&s, 1).as_deref()), (Some("a.rs"), Some("b.rs")));
    assert!(s.state().shown(0).unwrap().loaded());
    s.dispatch(Command::FocusPane(0));
    assert_eq!((s.state().focused_pane(), s.state().top_line()), (0, 50));
    // Activating the other pane's tab focuses that pane.
    let b_tab = s.state().pane_tabs(1).find(|t| t.title() == "b.rs").unwrap().id();
    s.dispatch(Command::ActivateTab(b_tab));
    assert_eq!(s.state().focused_pane(), 1);
    // Closing the other pane's shown tab shows its neighbour there.
    let a0 = s.state().shown(0).unwrap().id();
    s.dispatch(Command::FocusPane(1));
    s.dispatch(Command::CloseTab(b_tab));
    assert_eq!(shown_name(&s, 1).as_deref(), Some("a.rs"));
    assert_eq!(s.state().shown(0).unwrap().id(), a0);
}

#[test]
fn closing_a_split_panes_last_tab_closes_the_pane() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "a.rs", b"fn a() {}\n");
    let b = write(d.path(), "b.rs", b"fn b() {}\n");
    let mut s = session(d.path());
    s.dispatch(Command::Open(a.clone()));
    settle(&mut s, opened);
    // The new pane's last tab, closed with Close: the left pane remains, focused.
    s.dispatch(Command::Split(Split::Columns));
    s.dispatch(Command::Open(b.clone()));
    settle(&mut s, opened);
    s.dispatch(Command::Close);
    assert_eq!((pane_names(&s, 1), s.state().split()), (vec!["a.rs".to_string()], Some(Split::Columns)), "a tab is left");
    s.dispatch(Command::Close);
    assert_eq!(s.state().split(), None);
    assert_eq!((pane_names(&s, 0), shown_name(&s, 0).as_deref(), s.state().focused_pane()), (vec!["a.rs".to_string()], Some("a.rs"), 0));
    // The left pane's last tab, closed from its tab bar while the right is focused: the right pane's files remain.
    s.dispatch(Command::Split(Split::Rows));
    s.dispatch(Command::Open(b.clone()));
    settle(&mut s, opened);
    let left = s.state().pane_tabs(0).next().unwrap().id();
    s.dispatch(Command::CloseTab(left));
    assert_eq!(s.state().split(), None);
    assert_eq!((pane_names(&s, 0), shown_name(&s, 0).as_deref()), (vec!["a.rs".to_string(), "b.rs".to_string()], Some("b.rs")));
    // Dragging a pane's last tab into the other pane closes the split too, with or without a twin there.
    for twin in [false, true] {
        s.dispatch(Command::Split(Split::Columns));
        if !twin {
            s.dispatch(Command::Open(a.clone()));
            settle(&mut s, opened);
            let dup = s.state().pane_tabs(1).find(|t| t.title() == "b.rs").map(|t| t.id());
            if let Some(dup) = dup { s.dispatch(Command::CloseTab(dup)); }
            let a0 = s.state().pane_tabs(0).find(|t| t.title() == "a.rs").unwrap().id();
            s.dispatch(Command::CloseTab(a0));
        }
        assert_eq!(s.state().split(), Some(Split::Columns), "precondition, twin {twin}");
        let only: Vec<_> = s.state().pane_tabs(1).map(|t| t.id()).collect();
        assert_eq!(only.len(), 1, "twin {twin}: {:?}", pane_names(&s, 1));
        s.dispatch(Command::MoveTab { tab: only[0], pane: 0, index: 0 });
        assert_eq!(s.state().split(), None, "twin {twin}");
        assert!(s.state().active().is_some(), "twin {twin}");
    }
    // Unsplit, the last tab closes and leaves an empty window.
    s.dispatch(Command::Close);
    s.dispatch(Command::Close);
    assert_eq!((s.state().tabs().len(), s.state().split()), (0, None));
}

#[test]
fn tabs_move_within_and_across_panes_and_the_split_is_remembered() {
    let d = tempfile::tempdir().unwrap();
    let files: Vec<PathBuf> = ["a.rs", "b.rs", "c.rs"].iter().map(|n| write(d.path(), n, format!("// {n}\n").as_bytes())).collect();
    let mut s = session(d.path());
    for f in &files {
        s.dispatch(Command::OpenRetained(f.clone()));
        settle(&mut s, opened);
    }
    assert_eq!(pane_names(&s, 0), ["a.rs", "b.rs", "c.rs"]);
    // Reordering: c to the front.
    let id = |s: &Session, pane: u8, name: &str| s.state().pane_tabs(pane).find(|t| t.title() == name).unwrap().id();
    s.dispatch(Command::MoveTab { tab: id(&s, 0, "c.rs"), pane: 0, index: 0 });
    assert_eq!(pane_names(&s, 0), ["c.rs", "a.rs", "b.rs"]);
    assert_eq!(s.dispatch(Command::MoveTab { tab: id(&s, 0, "a.rs"), pane: 1, index: 0 }), Effect::default(), "no second pane yet");
    s.dispatch(Command::Split(Split::Rows));
    s.dispatch(Command::FocusPane(0));
    // a goes to the second pane: shown and focused there; its old pane shows its neighbour.
    s.dispatch(Command::ActivateTab(id(&s, 0, "a.rs")));
    settle(&mut s, opened);
    s.dispatch(Command::MoveTab { tab: id(&s, 0, "a.rs"), pane: 1, index: 0 });
    settle(&mut s, |s| s.state().shown(0).is_some_and(|t| t.loaded()));
    assert_eq!((pane_names(&s, 0), pane_names(&s, 1)), (vec!["c.rs".to_string(), "b.rs".to_string()], vec!["a.rs".to_string(), "c.rs".to_string()]));
    assert_eq!((s.state().focused_pane(), shown_name(&s, 1).as_deref(), shown_name(&s, 0).as_deref()), (1, Some("a.rs"), Some("b.rs")));
    // A source the target pane has already: that tab is shown, the moved one goes.
    s.dispatch(Command::MoveTab { tab: id(&s, 1, "c.rs"), pane: 0, index: 9 });
    assert_eq!((pane_names(&s, 0), pane_names(&s, 1)), (vec!["c.rs".to_string(), "b.rs".to_string()], vec!["a.rs".to_string()]));
    assert_eq!((s.state().focused_pane(), shown_name(&s, 0).as_deref()), (0, Some("c.rs")));
    s.dispatch(Command::SetLayout(Layout { explorer_width: Some(300), results_permille: None, split_permille: Some(600) }));
    // A new session starts split the same way, sized the same, with an empty second pane.
    let saved = d.path().join("state/lenz-reader/settings.json");
    settle(&mut s, |_| std::fs::read_to_string(&saved).is_ok_and(|t| t.contains("600")));
    s.dispatch(Command::Unsplit);
    assert_eq!((pane_names(&s, 0), s.state().split()), (vec!["c.rs".to_string(), "b.rs".to_string(), "a.rs".to_string()], None), "the other pane's tabs follow");
    assert!(s.state().shown(1).is_none());
    s.dispatch(Command::Split(Split::Columns));
    settle(&mut s, |_| std::fs::read_to_string(&saved).is_ok_and(|t| t.contains("Columns")));
    drop(s);
    let s = session(d.path());
    assert_eq!((s.state().split(), s.state().layout().split_permille), (Some(Split::Columns), Some(600)));
    assert!(s.state().shown(1).is_none() && s.state().pane_tabs(1).next().is_none());
}

const NESTED: &str = "// a\n// b\nfn f() {\n    if x {\n        y();\n    }\n}\nfn g() {\n    z();\n}\n";

/// A session showing `src` as `a.rs`, its folds computed.
fn folded(d: &Path, src: &str) -> Session {
    let path = write(d, "a.rs", src.as_bytes());
    let mut s = session(d);
    s.dispatch(Command::Open(path));
    settle(&mut s, |s| s.state().view(s.state.active.unwrap()).is_some_and(|v| v.folds().is_some()));
    s
}

fn caret(s: &mut Session, line: usize) {
    let at = s.state().current_source().unwrap().line_range(line).unwrap().start;
    s.dispatch(Command::Select(Selection { anchor: at, head: at }));
}

fn shown(s: &Session, n: usize) -> Vec<usize> {
    active_view(s).visible().lines(0, n)
}

#[test]
fn collapse_expand_and_all_change_the_visible_lines() {
    let d = tempfile::tempdir().unwrap();
    let mut s = folded(d.path(), NESTED);
    assert_eq!(s.disabled(CommandId::Collapse), None);
    caret(&mut s, 4); // inside `if x`
    s.dispatch(Command::Collapse);
    assert_eq!(shown(&s, 20), vec![0, 1, 2, 3, 6, 7, 8, 9, 10]);
    let head = s.state().selection().unwrap().head;
    assert_eq!(head, s.state().current_source().unwrap().line_range(3).unwrap().end, "the caret leaves the hidden lines for the fold's first line");
    s.dispatch(Command::Collapse); // again: the next enclosing region, fn f
    assert_eq!(shown(&s, 20), vec![0, 1, 2, 7, 8, 9, 10]);
    s.dispatch(Command::Expand);
    assert_eq!(shown(&s, 20), vec![0, 1, 2, 3, 6, 7, 8, 9, 10], "Expand opens the fold on the caret's line only");
    s.dispatch(Command::ExpandAll);
    s.dispatch(Command::CollapseAll);
    assert_eq!(shown(&s, 20), vec![0, 2, 7, 10]);
    s.dispatch(Command::ExpandAll);
    s.dispatch(Command::CollapseCommentsImports);
    assert_eq!(shown(&s, 3), vec![0, 2, 3]);
    s.dispatch(Command::ToggleFold(0));
    assert_eq!(shown(&s, 3), vec![0, 1, 2]);
}

#[test]
fn scrolling_and_shift_keys_move_over_collapsed_lines() {
    let d = tempfile::tempdir().unwrap();
    let mut s = folded(d.path(), NESTED);
    s.dispatch(Command::ToggleFold(2));
    s.dispatch(Command::ScrollRows(3));
    assert_eq!(s.state().top_line(), 7, "0 1 2 then 7");
    s.dispatch(Command::ScrollRows(-1));
    assert_eq!(s.state().top_line(), 2);
    s.dispatch(Command::Scroll(4));
    assert_eq!(s.state().top_line(), 2, "an absolute scroll into a fold shows its first line");
    s.dispatch(Command::Scroll(usize::MAX));
    assert_eq!(s.state().top_line(), 10, "the last line (the empty one after the final line ending)");
    caret(&mut s, 2);
    s.dispatch(Command::Extend(Motion::Down));
    assert_eq!(s.state().current_source().unwrap().line_of(s.state().selection().unwrap().head), 7);
    s.dispatch(Command::Extend(Motion::Up));
    assert_eq!(s.state().current_source().unwrap().line_of(s.state().selection().unwrap().head), 2);
}

#[test]
fn navigation_into_a_collapsed_region_unfolds_it() {
    let d = tempfile::tempdir().unwrap();
    let mut s = folded(d.path(), NESTED);
    s.dispatch(Command::ToggleFold(2));
    assert!(active_view(&s).visible().hidden(4));
    s.dispatch(Command::Search { query: "y()".into(), scope: SearchIn::ActiveFile });
    settle(&mut s, searched);
    s.dispatch(Command::OpenResult(0));
    settle(&mut s, |s| s.state().selection().is_some_and(|x| !x.range().is_empty()));
    let t = s.state().active().unwrap();
    assert_eq!(s.state().view(t.id()).unwrap().snapshot().line_of(t.selection().unwrap().head), 4);
    assert!(!active_view(&s).visible().hidden(4), "a result inside a collapsed region unfolds it");
}

#[test]
fn reload_keeps_matching_folds_and_drops_the_rest() {
    let d = tempfile::tempdir().unwrap();
    let mut s = folded(d.path(), NESTED);
    s.dispatch(Command::ToggleFold(2));
    s.dispatch(Command::ToggleFold(7));
    // Line 2 still starts `fn f`; line 7 now starts a comment run.
    std::fs::write(d.path().join("a.rs"), NESTED.replace("fn g() {\n    z();\n}\n", "// c\n// d\n")).unwrap();
    let before = active_view(&s).snapshot().key().version;
    s.dispatch(Command::Reload);
    settle(&mut s, |s| s.state().view(s.state.active.unwrap()).is_some_and(|v| v.snapshot().key().version > before && v.folds().is_some()));
    assert!(active_view(&s).visible().hidden(3) && !active_view(&s).visible().hidden(8));
    // A file shorter than every fold.
    std::fs::write(d.path().join("a.rs"), "x\n").unwrap();
    let before = active_view(&s).snapshot().key().version;
    s.dispatch(Command::Reload);
    settle(&mut s, |s| s.state().view(s.state.active.unwrap()).is_some_and(|v| v.snapshot().key().version > before && v.folds().is_some()));
    assert_eq!(shown(&s, 5), vec![0, 1]);
}

#[test]
fn panes_fold_independently() {
    let d = tempfile::tempdir().unwrap();
    let mut s = folded(d.path(), NESTED);
    s.dispatch(Command::Split(Split::Columns));
    s.dispatch(Command::ToggleFold(2));
    assert!(active_view(&s).visible().hidden(3));
    s.dispatch(Command::FocusPane(0));
    assert!(!active_view(&s).visible().hidden(3), "the other pane's tab keeps its own folds");
}

#[test]
fn fold_commands_explain_when_unavailable() {
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    assert_eq!(s.disabled(CommandId::Collapse), Some("No file is open"));
    s.dispatch(Command::Open(write(d.path(), "a.txt", b"plain\n")));
    settle(&mut s, opened);
    assert_eq!(s.disabled(CommandId::CollapseAll), Some("No folds for this file type"));
    let big = format!("fn f() {{\n{}}}\n", "    x();\n".repeat(crate::document::LARGE_LINES));
    s.dispatch(Command::Open(write(d.path(), "big.rs", big.as_bytes())));
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.is_large()));
    assert_eq!(s.disabled(CommandId::Expand), Some("Folding is off for large files"));
    assert!(CommandId::ALL.contains(&CommandId::CollapseCommentsImports));
    s.dispatch(Command::Open(write(d.path(), "c.rs", b"fn c() {}\n")));
    settle(&mut s, |s| s.state().view(s.state.active.unwrap()).is_some_and(|v| v.tab().source().ends_with("c.rs") && v.folds().is_some()));
    assert!(s.context_menu().contains(&MenuEntry::Command(CommandId::Collapse)));
}

#[test]
fn extend_and_shrink_selection_follow_the_syntax() {
    let d = tempfile::tempdir().unwrap();
    let src = "fn f() {\n    g(\"hello world\");\n}\n";
    let mut s = folded(d.path(), src);
    let at = src.find("world").unwrap() + 1;
    s.dispatch(Command::Select(Selection { anchor: at, head: at }));
    s.dispatch(Command::ExtendSelection);
    s.dispatch(Command::ExtendSelection); // pressed again while the steps parse: applied once
    settle(&mut s, |s| s.state().selection().is_some_and(|x| !x.range().is_empty()));
    let sel = |s: &Session| src[s.state().selection().unwrap().range()].to_string();
    assert_eq!(sel(&s), "world");
    s.dispatch(Command::ExtendSelection);
    assert_eq!(sel(&s), "hello world");
    s.dispatch(Command::ExtendSelection);
    assert_eq!(sel(&s), "\"hello world\"");
    assert_eq!(s.disabled(CommandId::ShrinkSelection), None);
    s.dispatch(Command::ShrinkSelection);
    s.dispatch(Command::ShrinkSelection);
    assert_eq!(sel(&s), "world");
    s.dispatch(Command::ShrinkSelection);
    assert!(s.state().selection().unwrap().range().is_empty(), "back to the caret");
    assert_eq!(s.disabled(CommandId::ShrinkSelection), Some("Nothing to shrink"));
    // A selection made another way starts a new expansion.
    s.dispatch(Command::ExtendSelection);
    s.dispatch(Command::Select(Selection { anchor: 0, head: 2 }));
    assert_eq!(s.disabled(CommandId::ShrinkSelection), Some("Nothing to shrink"));
}

#[test]
fn click_select_word_then_group_then_ctrl_w_continues() {
    let d = tempfile::tempdir().unwrap();
    let src = "fn f() {\n    g(\"hello world\", 1);\n}\n";
    let mut s = folded(d.path(), src);
    let sel = |s: &Session| src[s.state().selection().unwrap().range()].to_string();
    let at = src.find("world").unwrap() + 2;
    s.dispatch(Command::ClickSelect { at, clicks: 2 });
    assert_eq!(sel(&s), "world");
    s.dispatch(Command::ClickSelect { at, clicks: 3 });
    settle(&mut s, |s| s.state().selection().is_some_and(|x| x.range().len() > "world".len()));
    assert_eq!(sel(&s), "hello world", "the next Extend Selection step");
    s.dispatch(Command::ExtendSelection);
    assert_eq!(sel(&s), "\"hello world\"", "Ctrl+W goes on from the group");
    s.dispatch(Command::ClickSelect { at, clicks: 1 });
    assert_eq!(s.state().selection().unwrap().range(), at..at);
    let comma = src.find(", 1").unwrap();
    s.dispatch(Command::ClickSelect { at: comma, clicks: 2 });
    assert_eq!(sel(&s), "\",", "a punctuation run is a word");
}

#[test]
fn a_triple_click_keeps_the_view_where_it_was_clicked() {
    let d = tempfile::tempdir().unwrap();
    let body: String = (0..100).map(|i| format!("    let v{i} = {i};\n")).collect();
    let src = format!("fn f() {{\n{body}}}\n");
    let mut s = folded(d.path(), &src);
    s.dispatch(Command::SetTextSize { pane: 0, cols: 80, rows: 10 });
    s.dispatch(Command::ClickSelect { at: 1, clicks: 3 });
    settle(&mut s, |s| s.state().selection().is_some_and(|x| x.range().len() > 2));
    assert_eq!(&src[s.state().selection().unwrap().range()], src.trim_end(), "the whole function");
    assert_eq!(s.state().top_line(), 0, "the clicked line stays in view");
    // Ctrl+W still brings the selection's end into view.
    s.dispatch(Command::Select(Selection { anchor: 1, head: 1 }));
    s.dispatch(Command::ExtendSelection);
    settle(&mut s, |s| s.state().selection().is_some_and(|x| !x.range().is_empty()));
    s.dispatch(Command::ExtendSelection);
    assert!(s.state().top_line() > 0, "Ctrl+W reveals the end");
}

#[test]
fn click_select_in_plain_text_triple_is_the_line() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "a.txt", b"alpha beta\ngamma\n");
    let mut s = session(d.path());
    s.dispatch(Command::Open(path));
    settle(&mut s, opened);
    s.dispatch(Command::ClickSelect { at: 7, clicks: 3 });
    assert_eq!(s.state().selection().unwrap().range(), 0..10, "at once: no parse");
    s.dispatch(Command::ClickSelect { at: 16, clicks: 2 });
    assert_eq!(s.state().selection().unwrap().range(), 11..16, "at the end of a line: the word before it");
}

#[test]
fn extend_without_a_grammar_uses_words_lines_and_paragraphs() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "a.txt", b"alpha beta\ngamma\n\ndelta\n");
    let mut s = session(d.path());
    s.dispatch(Command::Open(path));
    settle(&mut s, opened);
    s.dispatch(Command::Select(Selection { anchor: 7, head: 7 }));
    s.dispatch(Command::ExtendSelection);
    assert_eq!(s.state().selection().unwrap().range(), 6..10, "at once: no parse");
    s.dispatch(Command::ExtendSelection);
    assert_eq!(s.state().selection().unwrap().range(), 0..10);
}

#[test]
fn plain_expansion_preserves_fragmented_unicode_and_defers_unbounded_runs() {
    let d = tempfile::tempdir().unwrap();
    let original = "π alpha βeta\r\nγamma\r\n\r\nδelta\r\n";
    let path = write(d.path(), "unicode.txt", original.as_bytes());
    let mut s = session(d.path());
    open(&mut s, Command::Open(path.clone()), "unicode.txt");
    let tab = s.state().active().unwrap().id();
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..0, text: "λ ".into() }], kind: crate::text::EditKind::Replace });
    let text = format!("λ {original}");
    let at = text.find("βeta").unwrap() + "β".len();
    s.dispatch(Command::Select(Selection { anchor: at, head: at }));
    for expected in ["βeta", "λ π alpha βeta", "λ π alpha βeta\r\nγamma", &text] {
        s.dispatch(Command::ExtendSelection);
        assert_eq!(s.state().selected_text().as_deref(), Some(expected), "ordinary expansion is immediate across UTF-8 chunks and CRLF");
    }
    s.dispatch(Command::ShrinkSelection);
    assert_eq!(s.state().selected_text().as_deref(), Some("λ π alpha βeta\r\nγamma"));
    assert_eq!(std::fs::read(path).unwrap(), original.as_bytes());
    let long = "a".repeat(128 << 10);
    let path = write(d.path(), "long.txt", long.as_bytes());
    open(&mut s, Command::Open(path.clone()), "long.txt");
    s.dispatch(Command::Select(Selection { anchor: 65536, head: 65536 }));
    s.dispatch(Command::ExtendSelection);
    assert_eq!(s.state().selection().unwrap().range(), 65536..65536, "an unbounded run stays on admitted background work");
    settle(&mut s, |s| s.state().selection().unwrap().range() == (0..long.len()));
    assert_eq!(std::fs::read(path).unwrap(), long.as_bytes());
}

#[test]
fn extend_on_empty_and_edge_carets() {
    let d = tempfile::tempdir().unwrap();
    for (name, src) in [("e.rs", ""), ("w.rs", "fn f() {}\n\n   \n"), ("e.txt", "")] {
        let path = write(d.path(), name, src.as_bytes());
        let mut s = session(d.path());
        s.dispatch(Command::Open(path));
        settle(&mut s, opened);
        for at in [0, src.len()] {
            s.dispatch(Command::Select(Selection { anchor: at, head: at }));
            s.dispatch(Command::ExtendSelection);
            settle(&mut s, |s| s.expanding.is_none());
            let r = s.state().selection().unwrap().range();
            assert!(r.end <= src.len(), "{name} {at}: {r:?}");
        }
    }
}

#[test]
fn extend_selection_is_in_the_catalog() {
    let d = tempfile::tempdir().unwrap();
    let s = session(d.path());
    assert!(CommandId::ALL.contains(&CommandId::ExtendSelection) && CommandId::ALL.contains(&CommandId::ShrinkSelection));
    assert_eq!(CommandId::ExtendSelection.label(), "Extend Selection");
    assert_eq!(CommandId::ShrinkSelection.label(), "Shrink Selection");
    assert_eq!(s.disabled(CommandId::ExtendSelection), Some("No file is open"));
}

#[test]
fn repeated_extend_and_tab_switches_keep_only_short_chains() {
    let d = tempfile::tempdir().unwrap();
    let src = "fn f() {\n    let x = (1 + 2) * 3;\n}\n".repeat(1000);
    let mut s = folded(d.path(), &src);
    let a = s.state().active().unwrap().id();
    let other = write(d.path(), "b.txt", b"b\n");
    for _ in 0..20 {
        s.dispatch(Command::Select(Selection { anchor: 30, head: 30 }));
        s.dispatch(Command::ExtendSelection);
        settle(&mut s, |s| s.state().selection().is_some_and(|x| !x.range().is_empty()));
        assert!(s.state().active().unwrap().expansion.as_ref().is_some_and(|e| e.steps.len() < 64));
        s.dispatch(Command::OpenRetained(other.clone()));
        settle(&mut s, opened);
        s.dispatch(Command::ActivateTab(a));
        settle(&mut s, |s| s.state().view(s.state.active.unwrap()).is_some_and(|v| v.tab().id() == a && v.folds().is_some()));
    }
}

#[test]
fn copy_with_a_caret_copies_its_line_and_across_folds_copies_the_source() {
    let d = tempfile::tempdir().unwrap();
    let mut s = folded(d.path(), NESTED);
    caret(&mut s, 4);
    assert_eq!(s.disabled(CommandId::Copy), None, "a caret copies its line");
    assert_eq!(s.dispatch(Command::Copy).copy.as_deref(), Some("        y();\n"));
    s.dispatch(Command::ToggleFold(2));
    let doc = s.state().current_source().unwrap().clone();
    s.dispatch(Command::Select(Selection { anchor: doc.line_range(2).unwrap().start, head: doc.line_range(7).unwrap().start }));
    assert_eq!(s.dispatch(Command::Copy).copy.as_deref(), Some("fn f() {\n    if x {\n        y();\n    }\n}\n"));
    assert_eq!(s.state().selected_text().as_deref(), Some("fn f() {\n    if x {\n        y();\n    }\n}\n"));
    // The line ending as written; the last line without one copies as it is.
    let path = write(d.path(), "c.txt", b"a\r\nlast");
    s.dispatch(Command::Open(path));
    settle(&mut s, |s| s.state().active().is_some_and(|t| t.source().ends_with("c.txt") && t.loaded()));
    s.dispatch(Command::Select(Selection { anchor: 0, head: 0 }));
    assert_eq!(s.dispatch(Command::Copy).copy.as_deref(), Some("a\r\n"));
    assert_eq!(s.state().selected_text().as_deref(), None, "a caret selects nothing");
    s.dispatch(Command::Select(Selection { anchor: 5, head: 5 }));
    assert_eq!(s.dispatch(Command::Copy).copy.as_deref(), Some("last"));
}

#[test]
fn copy_text_copies_what_the_adapter_hands_over_and_the_preview_menu_has_copy() {
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    assert_eq!(s.dispatch(Command::CopyText("x".into())).copy.as_deref(), Some("x"));
    assert_eq!(s.dispatch(Command::CopyText(String::new())).copy, None);
    let path = write(d.path(), "r.md", b"# T\n\n- a\n- b\n");
    s.dispatch(Command::Open(path));
    settle(&mut s, opened);
    s.dispatch(Command::ToggleMarkdownSource);
    settle(&mut s, |s| matches!(s.state().rendering(), Some(Rendering::Preview(_))));
    assert_eq!(s.context_menu().first(), Some(&MenuEntry::Command(CommandId::Copy)));
    assert_eq!(s.disabled(CommandId::Copy), None, "the adapter holds the preview's selection");
}

#[test]
fn shrinking_after_a_scroll_brings_the_selection_back_into_view() {
    let d = tempfile::tempdir().unwrap();
    let src: String = (0..50).map(|i| format!("line {i}\n")).collect();
    let path = write(d.path(), "a.txt", src.as_bytes());
    let mut s = session(d.path());
    s.dispatch(Command::Open(path));
    settle(&mut s, opened);
    caret(&mut s, 10);
    for _ in 0..4 {
        s.dispatch(Command::ExtendSelection);
        settle(&mut s, |s| s.expanding.is_none());
    }
    assert_eq!(s.state().selection().unwrap().range(), 0..src.len());
    s.dispatch(Command::Scroll(usize::MAX));
    s.dispatch(Command::ShrinkSelection);
    s.dispatch(Command::ShrinkSelection);
    let head = s.state().current_source().unwrap().line_of(s.state().selection().unwrap().head);
    assert!(s.state().top_line() <= head, "top {} head {head}", s.state().top_line());
}

#[test]
fn collapsing_keeps_a_selection_that_reaches_into_the_region() {
    let d = tempfile::tempdir().unwrap();
    let mut s = folded(d.path(), NESTED);
    let doc = s.state().current_source().unwrap().clone();
    let inside = Selection { anchor: doc.line_range(3).unwrap().start, head: doc.line_range(5).unwrap().end };
    s.dispatch(Command::Select(inside));
    s.dispatch(Command::CollapseAll);
    assert_eq!(s.state().selection(), Some(inside), "the selection still covers the hidden lines");
    assert_eq!(s.dispatch(Command::Copy).copy.as_deref(), Some("    if x {\n        y();\n    }"));
    // Expand with the head on a hidden line opens the region showing it.
    s.dispatch(Command::Expand);
    assert!(!active_view(&s).visible().hidden(3));
    // A caret on a hidden line still moves to the fold's first line.
    caret(&mut s, 4);
    s.dispatch(Command::CollapseAll);
    assert_eq!(s.state().selection().unwrap().head, doc.line_range(2).unwrap().end);
}

#[test]
fn the_visible_lines_are_kept_until_the_folds_change() {
    let d = tempfile::tempdir().unwrap();
    let mut s = folded(d.path(), NESTED);
    s.dispatch(Command::CollapseAll);
    let (a, b) = (active_view(&s).visible(), active_view(&s).visible());
    assert!(Arc::ptr_eq(&a, &b), "made once while nothing changes");
    s.dispatch(Command::ToggleFold(0));
    let c = active_view(&s).visible();
    assert!(!Arc::ptr_eq(&a, &c) && !c.hidden(1), "a fold change makes them again");
    s.dispatch(Command::ToggleFold(0));
    s.dispatch(Command::ExpandAll);
    assert!(!active_view(&s).visible().hidden(3));
}

#[test]
#[ignore = "timing: run in release"]
fn visible_lines_perf() {
    let d = tempfile::tempdir().unwrap();
    let src = "fn f() {\n    x();\n}\n".repeat(33_000);
    let mut s = folded(d.path(), &src);
    s.dispatch(Command::CollapseAll);
    let t = s.state().active().unwrap();
    let start = std::time::Instant::now();
    for _ in 0..1000 {
        std::hint::black_box(active_view(&s).visible().lines(t.top_line(), 60));
    }
    eprintln!("visible().lines(60) with 33k collapsed regions: {:.1} µs per call", start.elapsed().as_secs_f64() * 1000.0);
}

fn found(s: &Session) -> bool {
    s.state().active().and_then(|t| t.find()).is_some_and(|f| !f.pending())
}

fn find(s: &mut Session, text: &str) {
    s.dispatch(Command::Find(text.into()));
    settle(s, found);
}

#[test]
fn typing_finds_selects_the_first_match_at_or_after_the_caret_and_f3_wraps() {
    let d = tempfile::tempdir().unwrap();
    let mut s = folded(d.path(), "foo\nbar foo\nfoo\n");
    caret(&mut s, 1);
    find(&mut s, "foo");
    let f = s.state().active().unwrap().find().unwrap();
    assert_eq!((f.matches().len(), f.current()), (3, Some(1)));
    let sel = s.state().selection().unwrap();
    assert_eq!(sel.anchor.min(sel.head)..sel.anchor.max(sel.head), 8..11);
    s.dispatch(Command::FindStep { forward: true });
    s.dispatch(Command::FindStep { forward: true });
    assert_eq!(s.state().active().unwrap().find().unwrap().current(), Some(0), "wrapped");
    assert!(s.state().notices().iter().any(|n| n.contains("wrapped")), "{:?}", s.state().notices());
    s.dispatch(Command::FindStep { forward: false });
    assert_eq!(s.state().active().unwrap().find().unwrap().current(), Some(2), "back over the start");
}

#[test]
fn stale_find_results_are_dropped() {
    let d = tempfile::tempdir().unwrap();
    let mut s = folded(d.path(), &"foo bar\n".repeat(50_000));
    s.dispatch(Command::Find("fo".into()));
    s.dispatch(Command::Find("bar".into()));
    settle(&mut s, found);
    let f = s.state().active().unwrap().find().unwrap();
    assert_eq!((f.query(), f.matches().len(), f.capped()), ("bar", crate::find::MATCH_LIMIT, true), "the newer query's matches, capped");
}

#[test]
fn options_rerun_the_search_and_close_drops_the_state() {
    let d = tempfile::tempdir().unwrap();
    let mut s = folded(d.path(), "Foo foo food\n");
    find(&mut s, "foo");
    assert_eq!(s.state().active().unwrap().find().unwrap().matches().len(), 3);
    s.dispatch(Command::ToggleFindOption(FindOption::Case));
    settle(&mut s, found);
    assert_eq!(s.state().active().unwrap().find().unwrap().matches().len(), 2);
    s.dispatch(Command::ToggleFindOption(FindOption::Word));
    settle(&mut s, found);
    assert_eq!(s.state().active().unwrap().find().unwrap().matches(), &[(0, 4..7)]);
    assert!(s.find_options().case && s.find_options().word);
    s.dispatch(Command::ToggleFindOption(FindOption::Regex));
    s.dispatch(Command::Find("(".into()));
    settle(&mut s, found);
    assert_eq!(s.state().active().unwrap().find().unwrap().error(), Some("unclosed group"));
    assert_eq!(s.disabled(CommandId::NextMatch), None);
    s.dispatch(Command::CloseFind);
    assert!(s.state().active().unwrap().find().is_none());
    assert_eq!(s.disabled(CommandId::NextMatch), Some("No find is open"));
}

#[test]
fn reload_reruns_find_and_a_folded_match_unfolds() {
    let d = tempfile::tempdir().unwrap();
    let mut s = folded(d.path(), "fn a() {\n    x\n}\nfn b() {\n    target\n}\n");
    s.dispatch(Command::CollapseAll);
    assert!(active_view(&s).visible().hidden(4));
    find(&mut s, "target");
    assert!(!active_view(&s).visible().hidden(4), "the match's region unfolded");
    write(d.path(), "a.rs", b"target\nfn a() {}\n");
    s.dispatch(Command::Reload);
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| source_line(d, 0) == "target") && found(s));
    assert_eq!(s.state().active().unwrap().find().unwrap().matches(), &[(0, 0..6)]);
}

#[test]
fn find_is_unavailable_without_a_file_and_in_previews() {
    let d = tempfile::tempdir().unwrap();
    let s = session(d.path());
    assert_eq!(s.disabled(CommandId::FindInFile), Some("No file is open"));
    let md = write(d.path(), "a.md", b"# x\n");
    let mut s = session(d.path());
    s.dispatch(Command::Open(md));
    settle(&mut s, |s| matches!(s.state().rendering(), Some(Rendering::Source)));
    assert_eq!(s.disabled(CommandId::FindInFile), None, "the source finds");
    s.dispatch(Command::ToggleMarkdownSource);
    settle(&mut s, |s| rendered(s).is_some());
    assert_eq!(s.disabled(CommandId::FindInFile), Some("Find works on the source; Ctrl+Shift+V shows it"));
}

fn caret_at(s: &Session) -> (usize, usize) {
    let d = s.state().current_source().unwrap();
    let head = s.state().selection().unwrap().head;
    let line = d.line_of(head);
    (line, head - d.line_range(line).unwrap().start)
}

#[test]
fn go_to_line_clamps_unfolds_and_records_history() {
    let d = tempfile::tempdir().unwrap();
    let mut s = folded(d.path(), "fn a() {\n    one\n    two\n}\nlast\n");
    caret(&mut s, 0);
    s.dispatch(Command::CollapseAll);
    s.dispatch(Command::GoToLine(LineColumn { line: 3, column: Some(5) }));
    assert_eq!(caret_at(&s), (2, 4));
    assert!(!active_view(&s).visible().hidden(2), "the target's region unfolded");
    s.dispatch(Command::GoToLine(LineColumn { line: 99, column: Some(99) }));
    assert_eq!(caret_at(&s), (5, 0), "the last line (empty after the final newline), its end");
    s.dispatch(Command::Back);
    assert_eq!(caret_at(&s).0, 2, "Back returns to the previous place");
    assert_eq!(s.disabled(CommandId::GoToLine), None);
}

#[test]
fn the_finder_takes_name_colon_line() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    tree(&root, &["src/session.rs", "README.md"]);
    std::fs::write(root.join("src/session.rs"), "x\n".repeat(200)).unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, scanned);
    s.dispatch(Command::FindFile("session.rs:120".into()));
    settle(&mut s, |s| s.state().project().unwrap().finder().unwrap().ready);
    let f = s.state().project().unwrap().finder().unwrap();
    assert_eq!(f.hits.iter().map(|h| &**h).collect::<Vec<_>>(), ["src/session.rs"], "the name ranks, the line does not");
    assert_eq!(f.at(), Some(LineColumn { line: 120, column: None }));
    s.dispatch(Command::OpenAt { path: root.join("src/session.rs"), at: LineColumn { line: 120, column: None } });
    settle(&mut s, |s| s.state().current_source().is_some() && s.state().selection().is_some());
    assert_eq!(caret_at(&s), (119, 0));
}

fn txt(d: &Path, text: &str) -> Session {
    let path = write(d, "a.txt", text.as_bytes());
    let mut s = session(d);
    s.dispatch(Command::Open(path));
    settle(&mut s, opened);
    s
}

#[test]
fn wrapped_rows_scroll_page_and_end() {
    let d = tempfile::tempdir().unwrap();
    let mut s = txt(d.path(), &format!("{}\nshort\n", "word ".repeat(40)));
    s.dispatch(Command::SetTextSize { pane: 0, cols: 20, rows: 10 });
    s.dispatch(Command::SetSoftWrap(true));
    assert!(s.state().soft_wrap());
    let t = s.state().active().unwrap();
    let rows = source_rows(active_view(&s), 30, s.state().wrap_cols(t));
    assert_eq!(rows.iter().filter(|r| r.line == 0).count(), 10, "200 columns in rows of 20");
    s.dispatch(Command::ScrollRows(3));
    let t = s.state().active().unwrap();
    assert_eq!((t.top_line(), t.top_row()), (0, 3));
    s.dispatch(Command::ScrollRows(-5));
    let t = s.state().active().unwrap();
    assert_eq!((t.top_line(), t.top_row()), (0, 0), "clamped at the start");
    s.dispatch(Command::Scroll(usize::MAX));
    let t = s.state().active().unwrap();
    assert_eq!(source_rows(active_view(&s), 5, s.state().wrap_cols(t)).last().unwrap().line, 2, "End shows the last line");
}

#[test]
fn wrap_toggle_and_resize_keep_the_top_line() {
    let d = tempfile::tempdir().unwrap();
    let mut s = txt(d.path(), &"long line of text here\n".repeat(500));
    s.dispatch(Command::SetTextSize { pane: 0, cols: 10, rows: 10 });
    s.dispatch(Command::Scroll(300));
    s.dispatch(Command::SetSoftWrap(true));
    assert_eq!((s.state().active().unwrap().top_line(), s.state().active().unwrap().top_row()), (300, 0));
    s.dispatch(Command::ScrollRows(1));
    assert_eq!(s.state().active().unwrap().top_row(), 1);
    s.dispatch(Command::SetTextSize { pane: 0, cols: 40, rows: 10 });
    assert_eq!((s.state().active().unwrap().top_line(), s.state().active().unwrap().top_row()), (300, 0), "a resize re-wraps from the top line");
    s.dispatch(Command::SetSoftWrap(false));
    assert_eq!(s.state().active().unwrap().top_line(), 300);
    assert!(!s.state().soft_wrap());
}

#[test]
fn shift_down_moves_one_visual_row_and_folds_still_skip() {
    let d = tempfile::tempdir().unwrap();
    let mut s = folded(d.path(), &format!("fn a() {{\n    {}\n}}\nfn b() {{}}\n", "x ".repeat(30)));
    s.dispatch(Command::SetTextSize { pane: 0, cols: 20, rows: 10 });
    s.dispatch(Command::SetSoftWrap(true));
    let at = s.state().current_source().unwrap().line_range(1).unwrap().start + 6;
    s.dispatch(Command::Select(Selection { anchor: at, head: at }));
    s.dispatch(Command::Extend(Motion::Down));
    let head = s.state().selection().unwrap().head;
    assert_eq!(s.state().current_source().unwrap().line_of(head), 1, "the next visual row is still line 1");
    assert!(head > at + 10, "one row further: {head} from {at}");
    s.dispatch(Command::Extend(Motion::Up));
    assert_eq!(s.state().selection().unwrap().head, at, "back to the same column");
    s.dispatch(Command::CollapseAll);
    let t = s.state().active().unwrap();
    assert!(source_rows(active_view(&s), 10, s.state().wrap_cols(t)).iter().all(|r| r.line != 1), "collapsed lines are not rows");
}

#[test]
fn large_files_refuse_wrap_and_wrapping_resets_then_clamps_native_pan() {
    let d = tempfile::tempdir().unwrap();
    let mut s = txt(d.path(), "abc\n");
    s.dispatch(Command::SetTextSize { pane: 0, cols: 20, rows: 10 });
    s.dispatch(Command::ScrollColumn(5));
    s.dispatch(Command::SetSoftWrap(true));
    assert_eq!(s.state().active().unwrap().column(), 0);
    let old_viewport = s.native_viewport(0).unwrap();
    s.dispatch(Command::ScrollColumn(5));
    assert_eq!(s.state().active().unwrap().column(), 5, "native-overwide wrapped rows permit explicit pan");
    s.dispatch(Command::ClampColumn { viewport: old_viewport, max: 0 });
    assert_eq!(s.state().active().unwrap().column(), 5, "stale measured extents cannot undo a later pan");
    let current_viewport = s.native_viewport(0).unwrap();
    s.dispatch(Command::ClampColumn { viewport: current_viewport, max: 0 });
    assert_eq!(s.state().active().unwrap().column(), 0, "current complete native extents clamp a fitting wrapped row");
    let big = write(d.path(), "big.txt", "x\n".repeat(crate::document::LARGE_BYTES / 2 + 1).as_bytes());
    s.dispatch(Command::Open(big));
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.is_large()));
    assert_eq!(s.disabled(CommandId::SoftWrap), Some("Too large to wrap"));
    let t = s.state().active().unwrap();
    assert_eq!(s.state().wrap_cols(t), None);
}

#[test]
fn finds_in_two_panes_run_independently() {
    let d = tempfile::tempdir().unwrap();
    let mut s = folded(d.path(), &"foo bar\n".repeat(50_000));
    s.dispatch(Command::Split(Split::Columns));
    s.dispatch(Command::FocusPane(1));
    s.dispatch(Command::Find("foo".into()));
    s.dispatch(Command::FocusPane(0));
    s.dispatch(Command::Find("bar".into()));
    let both = |s: &Session| s.state().tabs().iter().filter(|t| t.find().is_some()).all(|t| !t.find().unwrap().pending());
    settle(&mut s, both);
    let counts: Vec<_> = s.state().tabs().iter().filter_map(|t| t.find()).map(|f| (f.query().to_owned(), f.matches().len())).collect();
    assert_eq!(counts.len(), 2, "{counts:?}");
    assert!(counts.iter().all(|(_, n)| *n == crate::find::MATCH_LIMIT), "neither pane's search was cancelled: {counts:?}");
}

#[test]
fn a_running_find_keeps_the_last_matches_and_never_says_no_matches() {
    let d = tempfile::tempdir().unwrap();
    let mut s = folded(d.path(), &"foo bar\n".repeat(50_000));
    s.dispatch(Command::Find("fo".into()));
    let f = s.state().active().unwrap().find().unwrap();
    assert!(f.pending());
    assert_eq!(f.counter(), "…", "searching, not \"No matches\"");
    settle(&mut s, found);
    s.dispatch(Command::Find("foo".into()));
    let f = s.state().active().unwrap().find().unwrap();
    assert!(f.pending() && !f.matches().is_empty(), "the last matches stay drawn until the new ones arrive");
    assert_ne!(f.counter(), "No matches");
    settle(&mut s, found);
    assert_eq!(s.state().active().unwrap().find().unwrap().query(), "foo");
}

#[test]
fn matches_and_go_to_line_come_into_view_counting_wrapped_rows() {
    let d = tempfile::tempdir().unwrap();
    // Each line is three rows at 10 columns.
    let text: String = (0..40).map(|i| format!("aaaaaaa bbbbbbb cccc{i:03}\n")).collect();
    let mut s = txt(d.path(), &text);
    s.dispatch(Command::SetTextSize { pane: 0, cols: 10, rows: 10 });
    s.dispatch(Command::SetSoftWrap(true));
    let shown = |s: &Session| { let t = s.state().active().unwrap(); source_rows(active_view(s), 10, s.state().wrap_cols(t)).iter().map(|r| r.line).collect::<Vec<_>>() };
    find(&mut s, "cccc008");
    let head = s.state().selection().unwrap().head;
    let line = s.state().current_source().unwrap().line_of(head);
    assert!(shown(&s).contains(&line), "the match's line is on screen: {:?}", shown(&s));
    s.dispatch(Command::GoToLine(LineColumn { line: 16, column: None }));
    let rows = { let t = s.state().active().unwrap(); source_rows(active_view(&s), 10, s.state().wrap_cols(t)) };
    let at = rows.iter().position(|r| r.line == 15).expect("line 16 on screen");
    assert_eq!(at, 3, "three rows below the top, as other navigation: {rows:?}");
}

#[test]
fn shift_down_into_a_shorter_wrapped_row_stays_on_that_row() {
    let d = tempfile::tempdir().unwrap();
    let mut s = txt(d.path(), "xxxxxxxxxxx\naaaa bbbb cccc dddd\nlast\n");
    s.dispatch(Command::SetTextSize { pane: 0, cols: 12, rows: 10 });
    s.dispatch(Command::SetSoftWrap(true));
    s.dispatch(Command::Select(Selection { anchor: 11, head: 11 }));
    s.dispatch(Command::Extend(Motion::Down));
    let doc = s.state().current_source().unwrap().clone();
    let head = s.state().selection().unwrap().head;
    let start = doc.line_range(1).unwrap().start;
    assert_eq!(doc.line_of(head), 1);
    assert!(head - start < 10, "on line 2's first row (\"aaaa bbbb \"), not its second: byte {}", head - start);
    s.dispatch(Command::Extend(Motion::Down));
    let head = s.state().selection().unwrap().head;
    assert_eq!(doc.line_of(head), 1, "then line 2's second row, not line 3");
}

const STICKY: &str = "mod m {\n    fn f() {\n        a;\n        b;\n        c;\n    }\n}\n";

/// A session showing `src` as `a.rs` with its declarations.
fn declared(d: &Path, src: &str) -> Session {
    let mut s = folded(d, src);
    settle(&mut s, |s| s.state().view(s.state.active.unwrap()).is_some_and(|v| v.declarations().is_some()));
    s
}

fn sticky_of(s: &Session) -> Vec<usize> {
    let crate::layout::Query::Ready(sticky) = active_view(s).layout_sticky(5, s.state().wrap_cols(s.state().active().unwrap())) else { panic!("fixture sticky layout pending") };
    sticky.iter().map(|k| k.line).collect()
}

#[test]
fn sticky_lines_follow_the_top_line() {
    let d = tempfile::tempdir().unwrap();
    let mut s = declared(d.path(), STICKY);
    s.dispatch(Command::Scroll(0));
    assert!(sticky_of(&s).is_empty(), "the header itself is on top");
    s.dispatch(Command::Scroll(3));
    assert_eq!(sticky_of(&s), [0, 1]);
}

#[test]
fn a_sticky_jump_goes_to_the_line_and_back_returns() {
    let d = tempfile::tempdir().unwrap();
    let mut s = declared(d.path(), STICKY);
    caret(&mut s, 4);
    s.dispatch(Command::Scroll(3));
    s.dispatch(Command::GoToLine(LineColumn { line: 2, column: None }));
    assert_eq!(caret_at(&s).0, 1);
    s.dispatch(Command::Back);
    assert_eq!(caret_at(&s).0, 4, "Back returns");
}

#[test]
fn toggles_are_saved_and_turning_one_on_parses_for_declarations() {
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::SetStickyScroll(false));
    s.dispatch(Command::SetBreadcrumbs(false));
    assert!(!s.state().sticky_scroll() && !s.state().breadcrumbs());
    let file = d.path().join("state/lenz-reader/settings.json");
    let saved = |key: &str| std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).and_then(|v| v.get(key)?.as_bool());
    settle(&mut s, |_| saved("sticky_scroll") == Some(false) && saved("breadcrumbs") == Some(false));
    let path = write(d.path(), "a.rs", STICKY.as_bytes());
    s.dispatch(Command::Open(path));
    settle(&mut s, |s| s.state().view(s.state.active.unwrap()).is_some_and(|v| v.folds().is_some()));
    assert!(active_view(&s).declarations().is_none(), "both off: no declarations asked for");
    s.dispatch(Command::SetStickyScroll(true));
    settle(&mut s, |s| s.state().view(s.state.active.unwrap()).is_some_and(|v| v.declarations().is_some() && v.folds().is_some()));
}

#[test]
fn a_reload_drops_the_old_versions_declarations() {
    let d = tempfile::tempdir().unwrap();
    let mut s = declared(d.path(), STICKY);
    s.dispatch(Command::Scroll(3));
    write(d.path(), "a.rs", b"x\n");
    s.dispatch(Command::Reload);
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.line_count() <= 2));
    let lines = s.state().current_source().unwrap().line_count();
    assert!(sticky_of(&s).iter().all(|&l| l < lines), "no header past the new text: {:?}", sticky_of(&s));
}

#[test]
fn wrapped_continuation_rows_of_a_header_make_it_stick() {
    let d = tempfile::tempdir().unwrap();
    let mut s = declared(d.path(), "mod a_module_name_far_longer_than_ten_columns {\n    x;\n    y;\n}\n");
    s.dispatch(Command::SetSoftWrap(true));
    s.dispatch(Command::SetTextSize { pane: 0, cols: 10, rows: 20 });
    s.dispatch(Command::ScrollRows(1));
    let t = s.state().active().unwrap();
    assert_eq!((t.top_line(), t.top_row() > 0), (0, true));
    assert_eq!(sticky_of(&s), [0]);
}

const TWO: &str = "mod m {\n    fn f() {\n        a;\n    }\n    fn g() {}\n}\n";

/// A session with `dir` open as the project (its `files` written) and `open` shown with its declarations.
fn project_with(dir: &Path, files: &[(&str, &str)], open: &str) -> Session {
    for (name, text) in files {
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    let mut s = session(dir);
    s.dispatch(Command::OpenRoot(dir.to_owned()));
    settle(&mut s, scanned);
    open_and_settle(&mut s, &dir.join(open));
    s
}

fn open_and_settle(s: &mut Session, path: &Path) {
    s.dispatch(Command::Open(path.to_owned()));
    let p = path.to_owned();
    settle(s, |s| s.state().current_source().is_some_and(|d| d.source() == p) && s.state().view(s.state.active.unwrap()).is_some_and(|v| v.declarations().is_some() || v.snapshot().is_large()));
}

fn crumb_names(s: &Session) -> Vec<String> {
    s.state().breadcrumbs_for(0).unwrap().crumbs.iter().map(|c| match c {
        Crumb::Folder { name, .. } | Crumb::File { name } => name.to_string(),
        Crumb::Decl { name, .. } => name.to_string(),
    }).collect()
}

#[test]
fn breadcrumbs_show_the_path_then_the_chain_at_the_caret() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    let mut s = project_with(&root, &[("src/ui/a.rs", TWO)], "src/ui/a.rs");
    caret(&mut s, 2);
    assert_eq!(crumb_names(&s), ["src", "ui", "a.rs", "m", "f"]);
    let at = TWO.find("g()").unwrap();
    s.dispatch(Command::Select(Selection { anchor: at, head: at }));
    assert_eq!(crumb_names(&s), ["src", "ui", "a.rs", "m", "g"]);
    caret(&mut s, 4);
    assert_eq!(crumb_names(&s), ["src", "ui", "a.rs", "m"], "the indentation before `fn g` is only inside m");
    s.dispatch(Command::SetBreadcrumbs(false));
    assert!(s.state().breadcrumbs_for(0).is_none(), "off");
}

#[test]
fn the_list_after_a_crumb_holds_its_level() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    let mut s = project_with(&root, &[("src/ui/a.rs", TWO), ("src/ui/b.rs", ""), ("src/ui/w/c.rs", "")], "src/ui/a.rs");
    caret(&mut s, 2);
    let labels = |s: &Session, i: usize| s.state().siblings(0, i).into_iter().map(|x| x.label).collect::<Vec<_>>();
    assert_eq!(labels(&s, 1), ["w", "a.rs", "b.rs"], "after `ui`: its entries, folders first");
    assert_eq!(labels(&s, 2), ["w", "a.rs", "b.rs"], "after `a.rs`: the entries of its folder");
    assert_eq!(labels(&s, 3), ["m"], "after `m`: the declarations at its depth");
    assert_eq!(labels(&s, 4), ["f", "g"], "after `f`: the declarations beside it under m");
    assert!(matches!(s.state().siblings(0, 4)[1].target, SiblingTarget::Line(4)));
    assert!(matches!(&s.state().siblings(0, 1)[0].target, SiblingTarget::Folder(rel) if &**rel == "src/ui/w"));
    assert!(matches!(&s.state().siblings(0, 1)[1].target, SiblingTarget::File(rel) if &**rel == "src/ui/a.rs"));
}

#[test]
fn reveal_in_tree_expands_a_folder() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    let mut s = project_with(&root, &[("src/ui/a.rs", TWO), ("src/ui/w/c.rs", "")], "src/ui/a.rs");
    s.dispatch(Command::RevealInTree("src/ui/w".into()));
    let rows: Vec<String> = s.state().project().unwrap().rows().iter().map(|r| r.path.to_string()).collect();
    assert!(rows.contains(&"src/ui/w/c.rs".to_string()), "{rows:?}");
}

#[test]
fn a_file_outside_the_project_shows_its_full_path_and_a_large_file_no_chain() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    let mut s = project_with(&root, &[("src/a.rs", TWO)], "src/a.rs");
    let outside = d.path().join("elsewhere/x.rs");
    std::fs::create_dir_all(outside.parent().unwrap()).unwrap();
    std::fs::write(&outside, TWO).unwrap();
    open_and_settle(&mut s, &outside);
    let names = crumb_names(&s);
    assert!(names.contains(&"elsewhere".to_string()) && names.contains(&"x.rs".to_string()) && names.len() > 4, "the absolute path's folders lead: {names:?}");
    let big = root.join("big.rs");
    std::fs::write(&big, "fn f() {}\n".repeat(crate::document::LARGE_LINES + 1)).unwrap();
    open_and_settle(&mut s, &big);
    let b = s.state().breadcrumbs_for(0).unwrap();
    assert!(b.large && !b.crumbs.iter().any(|c| matches!(c, Crumb::Decl { .. })));
}

#[test]
fn a_header_slides_by_shown_rows_over_a_collapsed_region() {
    let d = tempfile::tempdir().unwrap();
    let src = format!("mod m {{\n    impl A {{\n        fn f() {{\n            if x {{\n{}            }}\n        }}\n    }}\n}}\n", "                y;\n".repeat(10));
    let mut s = declared(d.path(), &src);
    caret(&mut s, 3);
    s.dispatch(Command::Collapse);
    s.dispatch(Command::Scroll(3));
    // Shown: the collapsed `if` (line 3), then f's `}` (15), impl's (16), m's (17): f's scope has ended a row down.
    let rows: Vec<usize> = source_rows(active_view(&s), 4, None).iter().map(|r| r.line).collect();
    assert_eq!(rows, [3, 15, 16, 17]);
    assert_eq!(sticky_of(&s), [0, 1], "fn f's header would sit below its own end");
}

/// `mod m { fn f() { 30 lines } }` in a 20-row pane, scrolled to line 10: `mod m` and `fn f` are pinned.
fn banded(d: &Path) -> Session {
    let src = format!("mod m {{\n    fn f() {{\n{}    }}\n}}\n", "        a;\n".repeat(30));
    let mut s = declared(d, &src);
    s.dispatch(Command::SetTextSize { pane: 0, cols: 80, rows: 20 });
    s.dispatch(Command::Scroll(10));
    assert_eq!(sticky_of(&s), [0, 1]);
    s
}

/// The row line `line` is shown on, from the top.
fn row_of(s: &Session, line: usize) -> Option<usize> {
    source_rows(active_view(s), 20, None).iter().position(|r| r.line == line)
}

#[test]
fn navigation_lands_below_the_sticky_band() {
    let d = tempfile::tempdir().unwrap();
    let mut s = banded(d.path());
    s.dispatch(Command::GoToLine(LineColumn { line: 11, column: None })); // line 10: on row 0, under the band
    let band = sticky_of(&s).len();
    assert!(row_of(&s, 10).is_some_and(|r| r >= band), "row {:?} under a band of {band}", row_of(&s, 10));
}

#[test]
fn moving_the_caret_up_scrolls_it_clear_of_the_band() {
    let d = tempfile::tempdir().unwrap();
    let mut s = banded(d.path());
    caret(&mut s, 12);
    s.dispatch(Command::Scroll(10));
    s.dispatch(Command::Extend(Motion::Up));
    s.dispatch(Command::Extend(Motion::Up));
    let band = sticky_of(&s).len();
    assert!(row_of(&s, 10).is_some_and(|r| r >= band), "row {:?} under a band of {band}", row_of(&s, 10));
}

/// The places file `root`'s session saves to, under the test's state directory.
fn places_file(dir: &Path, root: &Path) -> PathBuf {
    crate::places::files_for(&dir.join("state/lenz-reader/projects"), &normalized(root)).0
}

fn open_project(s: &mut Session, root: &Path, file: &str) {
    s.dispatch(Command::OpenRoot(root.to_owned()));
    settle(s, scanned);
    s.dispatch(Command::Open(root.join(file)));
    settle(s, opened);
}

fn caret_at_line(s: &mut Session, line: usize) {
    let at = s.state().current_source().unwrap().line_range(line).unwrap().start;
    s.dispatch(Command::Select(Selection { anchor: at, head: at }));
}

#[test]
fn leaving_files_and_navigating_record_recent_places() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    tree(&root, &["a.rs", "b.rs"]);
    std::fs::write(root.join("a.rs"), "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n").unwrap();
    let mut s = session(d.path());
    open_project(&mut s, &root, "a.rs");
    s.dispatch(Command::GoToLine(LineColumn { line: 12, column: None }));
    s.dispatch(Command::Open(root.join("b.rs")));
    settle(&mut s, opened);
    let p = s.state().places();
    assert_eq!(p.recent_files().front(), Some(&normalized(&root.join("a.rs"))));
    assert!(p.recent_locations().iter().any(|m| m.source.ends_with("a.rs") && m.line == 11), "{:?}", p.recent_locations());
}

#[test]
fn bookmarks_survive_a_restart() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    tree(&root, &["a.rs"]);
    std::fs::write(root.join("a.rs"), "x\ny\nz\n").unwrap();
    let mut s = session(d.path());
    open_project(&mut s, &root, "a.rs");
    caret_at_line(&mut s, 2);
    s.dispatch(Command::SetMnemonic('4'));
    drop(s); // within the debounce: the drop writes it
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, |s| !s.state().places().bookmarks().is_empty());
    let m = s.state().places().mnemonic('4').unwrap();
    assert_eq!((m.source.to_path_buf(), m.line), (normalized(&root.join("a.rs")), 2));
    assert!(s.state().places().recent_files().contains(&normalized(&root.join("a.rs"))), "left at exit");
}

#[test]
fn two_roots_keep_their_own_places() {
    let d = tempfile::tempdir().unwrap();
    let (one, two) = (d.path().join("one"), d.path().join("two"));
    tree(&one, &["a.rs"]);
    tree(&two, &["b.rs"]);
    let mut s = session(d.path());
    open_project(&mut s, &one, "a.rs");
    s.dispatch(Command::ToggleBookmark);
    open_project(&mut s, &two, "b.rs");
    assert!(s.state().places().bookmarks().is_empty(), "the second root starts with its own");
    drop(s);
    let saved = |root: &Path| crate::places::read(&places_file(d.path(), root), &normalized(root)).unwrap();
    assert_eq!(saved(&one).bookmarks().len(), 1);
    assert!(saved(&two).bookmarks().is_empty());
}

#[test]
fn a_broken_places_file_says_so_and_starts_empty() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    tree(&root, &["a.rs"]);
    let file = places_file(d.path(), &root);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, b"{ broken").unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, |s| s.state().notices().iter().any(|n| n.starts_with("Bookmarks and recent places could not be read")));
    assert!(s.state().places().bookmarks().is_empty());
}

#[test]
fn a_reload_moves_bookmarks_with_their_line() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    tree(&root, &["a.rs"]);
    std::fs::write(root.join("a.rs"), "a\nfn f() {}\nb\n").unwrap();
    let mut s = session(d.path());
    open_project(&mut s, &root, "a.rs");
    caret_at_line(&mut s, 1);
    s.dispatch(Command::ToggleBookmark);
    std::fs::write(root.join("a.rs"), "new\nnew\na\nfn f() {}\nb\n").unwrap();
    s.dispatch(Command::Reload);
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.line_count() == 6));
    let lines: Vec<usize> = s.state().places().bookmarks().iter().map(|m| m.line).collect();
    assert_eq!(lines, [3]);
}

#[test]
fn places_without_a_folder_last_for_the_session() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "a.rs", b"x\n");
    let mut s = session(d.path());
    s.dispatch(Command::Open(path));
    settle(&mut s, opened);
    s.dispatch(Command::ToggleBookmark);
    assert_eq!(s.state().places().bookmarks().len(), 1);
    drop(s);
    assert!(!d.path().join("state/lenz-reader/projects").exists(), "nothing written without a root");
}

fn bookmarked_project(d: &Path) -> (Session, PathBuf) {
    let root = d.join("p");
    tree(&root, &["a.rs", "b.rs", "img.png"]);
    std::fs::write(root.join("a.rs"), "0\n1\n2\n3\n4\n5\n6\n7\n8\n9\n").unwrap();
    std::fs::write(root.join("b.rs"), "b0\nb1\n").unwrap();
    std::fs::write(root.join("img.png"), png(2, 2, [0, 0, 0, 255])).unwrap();
    let mut s = session(d);
    open_project(&mut s, &root, "a.rs");
    (s, root)
}

fn caret_line(s: &Session) -> usize {
    let st = s.state();
    st.current_source().unwrap().line_of(st.selection().unwrap().head)
}

#[test]
fn bookmark_commands_say_why_they_cannot_run() {
    let d = tempfile::tempdir().unwrap();
    let s = session(d.path());
    assert_eq!(s.disabled(CommandId::ToggleBookmark), Some("No file is open"));
    assert_eq!(s.disabled(CommandId::ShowBookmarks), Some("No bookmarks"));
    assert_eq!(s.disabled(CommandId::NextBookmark), Some("No bookmarks"));
    assert_eq!(s.disabled(CommandId::RecentFiles), Some("No recent files"));
    assert_eq!(s.disabled(CommandId::RecentLocations), Some("No recent locations"));
    let (mut s, root) = bookmarked_project(d.path());
    assert_eq!(s.disabled(CommandId::ToggleBookmark), None);
    s.dispatch(Command::Open(root.join("img.png")));
    settle(&mut s, opened);
    assert_eq!(s.disabled(CommandId::ToggleBookmark), Some("Bookmarks need a text view"));
    assert_eq!(s.disabled(CommandId::SetBookmarkMnemonic), Some("Bookmarks need a text view"));
    let before = s.state().places().clone();
    s.dispatch(Command::ToggleBookmark);
    assert_eq!(*s.state().places(), before, "nothing is added over an image");
}

#[test]
fn bookmark_keys_where_none_can_be_set_say_why() {
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::SetMnemonic('3'));
    assert!(s.state().notices().iter().any(|n| n == "No file is open"), "{:?}", s.state().notices());
    drop(s);
    let (mut s, root) = bookmarked_project(d.path());
    s.dispatch(Command::Open(root.join("img.png")));
    settle(&mut s, opened);
    s.dispatch(Command::ToggleBookmark);
    assert!(s.state().notices().iter().any(|n| n == "Bookmarks need a text view"), "{:?}", s.state().notices());
}

#[test]
fn draft_transactions_move_bookmarks_and_reading_history_with_unicode_text() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project"); std::fs::create_dir(&root).unwrap();
    let original = "first\r\nα😀 target\r\nlast\r\n";
    let path = write(&root, "anchors.txt", original.as_bytes());
    let mut s = session(d.path());
    open_project(&mut s, &root, "anchors.txt");
    s.dispatch(Command::GoToLine(LineColumn { line: 2, column: Some(4) }));
    let marked = s.state().selection().unwrap().head;
    s.dispatch(Command::SetMnemonic('2'));
    s.dispatch(Command::GoToLine(LineColumn { line: 3, column: Some(2) }));
    let tab = s.state().active().unwrap().id();
    let expected = s.state().current_source().unwrap().key();
    let inserted = "é😀 new\r\n";
    s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..0, text: inserted.into() }], kind: crate::text::EditKind::Paste });
    let bookmark = s.state().places().bookmarks().iter().find(|m| m.mnemonic == Some('2')).unwrap();
    assert_eq!((bookmark.line, bookmark.offset), (2, marked - "first\r\n".len()));
    s.dispatch(Command::Back);
    assert_eq!(s.state().selection().unwrap().head, marked + inserted.len(), "Back follows the edited source anchor rather than its former line/byte");
    s.dispatch(Command::JumpMnemonic('2'));
    assert_eq!(s.state().selection().unwrap().head, marked + inserted.len());
    s.dispatch(Command::Undo { tab });
    assert_eq!(s.state().places().bookmarks().iter().find(|m| m.mnemonic == Some('2')).unwrap().line, 1);
    assert_eq!(source_text(s.state().current_source().unwrap()), original);
    assert_eq!(std::fs::read(path).unwrap(), original.as_bytes());
}

#[test]
fn edited_source_anchors_use_right_bias_at_insertions_and_replacements() {
    let d = tempfile::tempdir().unwrap();
    let original = "first\n😀 target\nlast\n";
    let path = write(d.path(), "bias.txt", original.as_bytes());
    let mut s = session(d.path()); open(&mut s, Command::OpenRetained(path.clone()), "bias.txt");
    s.dispatch(Command::GoToLine(LineColumn { line: 2, column: Some(1) }));
    s.dispatch(Command::SetMnemonic('2'));
    let tab = s.state().active().unwrap().id();
    let start = "first\n".len();
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: start..start, text: "é\n".into() }], kind: crate::text::EditKind::Paste });
    let bookmark = s.state().places().bookmarks().iter().find(|m| m.mnemonic == Some('2')).unwrap();
    assert_eq!((bookmark.line, bookmark.offset), (2, 0), "an insertion at the anchor keeps it with the following source");
    assert_eq!(s.state().selection().unwrap().head, start + "é\n".len());
    let expected = s.state().current_source().unwrap().key();
    let point = start + "é\n".len();
    s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: point..point + "😀 ".len(), text: "Ω".into() }], kind: crate::text::EditKind::Replace });
    let bookmark = s.state().places().bookmarks().iter().find(|m| m.mnemonic == Some('2')).unwrap();
    assert_eq!((bookmark.line, bookmark.offset), (2, "Ω".len()), "an anchor inside replaced text collapses after the replacement");
    assert_eq!(s.state().selection().unwrap().head, point + "Ω".len());
    s.dispatch(Command::JumpMnemonic('2'));
    assert_eq!(s.state().selection().unwrap().head, point + "Ω".len());
    assert_eq!(std::fs::read(path).unwrap(), original.as_bytes());
}

#[test]
fn a_numbered_bookmark_jumps_back_through_history_and_unfolds() {
    let d = tempfile::tempdir().unwrap();
    let (mut s, root) = bookmarked_project(d.path());
    caret_at_line(&mut s, 7);
    s.dispatch(Command::SetMnemonic('2'));
    s.dispatch(Command::Open(root.join("b.rs")));
    settle(&mut s, opened);
    s.dispatch(Command::JumpMnemonic('2'));
    settle(&mut s, |s| s.state().active().is_some_and(|t| t.source().ends_with("a.rs")) && opened(s) && s.state().selection().is_some());
    assert_eq!(caret_line(&s), 7);
    // The jump is two steps, as Open-at-line is: into a.rs, then to the line. Two Backs return to b.rs.
    s.dispatch(Command::Back);
    settle(&mut s, opened);
    s.dispatch(Command::Back);
    settle(&mut s, |s| opened(s) && s.state().active().is_some_and(|t| t.source().ends_with("b.rs")));
    s.dispatch(Command::JumpMnemonic('9'));
    assert!(s.state().notices().iter().any(|n| n == "Bookmark 9 is not set"));
}

#[test]
fn a_bookmark_inside_a_collapsed_region_opens_it() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    tree(&root, &["a.rs"]);
    std::fs::write(root.join("a.rs"), "fn f() {\n    let x = 1;\n    let y = 2;\n}\n").unwrap();
    let mut s = session(d.path());
    open_project(&mut s, &root, "a.rs");
    settle(&mut s, |s| s.state().active().is_some_and(|t| t.folds.is_some()));
    caret_at_line(&mut s, 2);
    s.dispatch(Command::SetMnemonic('1'));
    caret_at_line(&mut s, 0);
    s.dispatch(Command::Collapse);
    assert!(active_view(&s).visible().hidden(2));
    s.dispatch(Command::JumpMnemonic('1'));
    assert!(!active_view(&s).visible().hidden(2));
    assert_eq!(caret_line(&s), 2);
}

#[test]
fn next_and_previous_bookmark_wrap_across_files() {
    let d = tempfile::tempdir().unwrap();
    let (mut s, root) = bookmarked_project(d.path());
    caret_at_line(&mut s, 3);
    s.dispatch(Command::ToggleBookmark);
    s.dispatch(Command::Open(root.join("b.rs")));
    settle(&mut s, opened);
    caret_at_line(&mut s, 1);
    s.dispatch(Command::ToggleBookmark);
    s.dispatch(Command::StepBookmark { forward: true });
    settle(&mut s, |s| opened(s) && s.state().active().unwrap().source().ends_with("a.rs"));
    assert_eq!(caret_line(&s), 3, "wrapped from b.rs to a.rs");
    s.dispatch(Command::StepBookmark { forward: false });
    settle(&mut s, |s| opened(s) && s.state().active().unwrap().source().ends_with("b.rs"));
    assert_eq!(caret_line(&s), 1);
}

#[test]
fn recent_files_put_the_previous_file_first_and_the_current_second() {
    let d = tempfile::tempdir().unwrap();
    let (mut s, root) = bookmarked_project(d.path());
    s.dispatch(Command::Open(root.join("b.rs")));
    settle(&mut s, opened);
    let rows = s.state().place_rows(PlaceList::RecentFiles, "");
    assert_eq!(rows.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(), ["a.rs", "b.rs"]);
    s.dispatch(Command::GoToPlace(rows[0].place.clone()));
    settle(&mut s, |s| opened(s) && s.state().active().unwrap().source().ends_with("a.rs"));
    let rows = s.state().place_rows(PlaceList::RecentFiles, "b");
    assert_eq!(rows.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(), ["b.rs"], "filtered");
}

#[test]
fn place_rows_show_where_and_forget_on_request() {
    let d = tempfile::tempdir().unwrap();
    let (mut s, root) = bookmarked_project(d.path());
    caret_at_line(&mut s, 4);
    s.dispatch(Command::SetMnemonic('c'));
    let rows = s.state().place_rows(PlaceList::Bookmarks, "");
    assert_eq!((rows[0].label.as_str(), rows[0].detail.as_str()), ("C  a.rs:5", "4"));
    assert!(s.state().place_rows(PlaceList::Bookmarks, "zzz").is_empty());
    s.dispatch(Command::ForgetPlace(rows[0].place.clone()));
    assert!(s.state().places().bookmarks().is_empty());
    let _ = root;
}

#[test]
fn missing_files_are_marked_once_checked() {
    let d = tempfile::tempdir().unwrap();
    let (mut s, root) = bookmarked_project(d.path());
    s.dispatch(Command::Open(root.join("b.rs")));
    settle(&mut s, opened);
    std::fs::remove_file(root.join("a.rs")).unwrap();
    s.dispatch(Command::CheckPlaces);
    settle(&mut s, |s| s.state().place_rows(PlaceList::RecentFiles, "").iter().any(|r| r.missing));
    let rows = s.state().place_rows(PlaceList::RecentFiles, "");
    assert!(rows.iter().find(|r| r.label == "a.rs").unwrap().missing);
}

#[test]
fn a_missing_files_check_from_before_a_root_switch_is_dropped() {
    let d = tempfile::tempdir().unwrap();
    let (mut s, root) = bookmarked_project(d.path());
    let generation = s.project_generation.load(std::sync::atomic::Ordering::Relaxed);
    let two = d.path().join("two");
    tree(&two, &["b.rs"]);
    s.dispatch(Command::OpenRoot(two));
    s.dispatch(Completion::PlacesChecked { generation, missing: [root.join("a.rs")].into() });
    assert!(s.state.missing.is_empty(), "the old project's check");
}

#[test]
fn a_jump_follows_the_line_into_a_file_changed_while_closed() {
    let d = tempfile::tempdir().unwrap();
    let (mut s, root) = bookmarked_project(d.path());
    caret_at_line(&mut s, 5);
    s.dispatch(Command::SetMnemonic('5'));
    s.dispatch(Command::Open(root.join("b.rs")));
    settle(&mut s, opened);
    s.dispatch(Command::CloseTab(s.state().tabs().iter().find(|t| t.source().ends_with("a.rs")).map_or(TabId(0), |t| t.id)));
    std::fs::write(root.join("a.rs"), "new\n0\n1\n2\n3\n4\n5\n6\n").unwrap();
    s.dispatch(Command::JumpMnemonic('5'));
    settle(&mut s, |s| opened(s) && s.state().active().unwrap().source().ends_with("a.rs") && s.state().selection().is_some());
    assert_eq!(caret_line(&s), 6, "line \"5\" moved down one");
}

#[test]
fn a_root_left_before_its_places_are_read_keeps_its_file() {
    let d = tempfile::tempdir().unwrap();
    let (one, two) = (d.path().join("one"), d.path().join("two"));
    tree(&one, &["a.rs"]);
    tree(&two, &["b.rs"]);
    let mut s = session(d.path());
    open_project(&mut s, &one, "a.rs");
    s.dispatch(Command::ToggleBookmark);
    drop(s);
    let mut s = session(d.path());
    // Navigate in `one` before its places are read, then leave it: nothing read means nothing to save over.
    s.dispatch(Command::OpenRoot(one.clone()));
    s.dispatch(Command::Open(one.join("a.rs")));
    s.dispatch(Command::OpenRoot(two.clone()));
    drop(s);
    let saved = crate::places::read(&places_file(d.path(), &one), &normalized(&one)).unwrap();
    assert_eq!(saved.bookmarks().len(), 1);
}

#[test]
fn an_unreadable_places_file_is_set_aside_before_anything_is_saved() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    tree(&root, &["a.rs"]);
    let file = places_file(d.path(), &root);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, b"{ broken").unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, |s| s.state().notices().iter().any(|n| n.starts_with("Bookmarks and recent places could not be read")));
    s.dispatch(Command::Open(root.join("a.rs")));
    settle(&mut s, opened);
    s.dispatch(Command::ToggleBookmark);
    drop(s);
    assert_eq!(std::fs::read(file.with_extension("json.unreadable")).unwrap(), b"{ broken");
    assert_eq!(crate::places::read(&file, &normalized(&root)).unwrap().bookmarks().len(), 1);
}

#[test]
fn a_places_file_of_another_version_is_left_alone_and_another_roots_is_set_aside() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    tree(&root, &["a.rs"]);
    let file = places_file(d.path(), &root);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let newer = br#"{"v":2,"marks":{}}"#;
    std::fs::write(&file, newer).unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, |s| s.state().notices().iter().any(|n| n == "Places file is from another version; not saved"));
    s.dispatch(Command::Open(root.join("a.rs")));
    settle(&mut s, opened);
    s.dispatch(Command::ToggleBookmark);
    drop(s);
    assert_eq!(std::fs::read(&file).unwrap(), newer, "untouched, and nothing saved over it");
    // A file copied from another folder, named for this one: a folder that exists, by its own real path.
    std::fs::write(&file, crate::places::Places::default().to_bytes(&std::fs::canonicalize(d.path()).unwrap())).unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, |s| s.state().notices().iter().any(|n| n.starts_with("Bookmarks and recent places could not be read")));
    drop(s);
    assert!(file.with_extension("json.unreadable").exists(), "set aside like a broken file");
}

#[test]
fn two_windows_on_one_folder_keep_each_others_bookmarks() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    tree(&root, &["a.rs"]);
    std::fs::write(root.join("a.rs"), "x\ny\nz\n").unwrap();
    let (mut one, mut two) = (session(d.path()), session(d.path()));
    for (s, line) in [(&mut one, 0), (&mut two, 1)] {
        open_project(s, &root, "a.rs");
        caret_at_line(s, line);
        s.dispatch(Command::ToggleBookmark);
    }
    drop(one);
    drop(two);
    let saved = crate::places::read(&places_file(d.path(), &root), &normalized(&root)).unwrap();
    assert_eq!(saved.bookmarks().iter().map(|m| m.line).collect::<Vec<_>>(), [0, 1]);
}

#[test]
fn a_merged_save_from_before_a_change_or_a_root_switch_is_dropped() {
    let d = tempfile::tempdir().unwrap();
    let (mut s, _root) = bookmarked_project(d.path());
    let generation = s.project_generation.load(std::sync::atomic::Ordering::Relaxed);
    let mut merged = crate::places::Places::default();
    merged.left_file(Path::new("/elsewhere/x.rs"));
    let theirs = |s: &Session| s.state().places().recent_files().contains(&PathBuf::from("/elsewhere/x.rs"));
    let seq = s.places_seq;
    caret_at_line(&mut s, 1);
    s.dispatch(Command::ToggleBookmark);
    s.dispatch(Completion::PlacesMerged { generation, seq, places: merged.clone() });
    assert!(!theirs(&s), "a change was made since: its own save brings them");
    let seq = s.places_seq;
    s.dispatch(Completion::PlacesMerged { generation, seq, places: merged.clone() });
    assert!(theirs(&s), "the current save's merge is taken");
    let two = d.path().join("two");
    tree(&two, &["b.rs"]);
    s.dispatch(Command::OpenRoot(two));
    let seq = s.places_seq;
    s.dispatch(Completion::PlacesMerged { generation, seq, places: merged });
    assert!(!theirs(&s), "another root's");
}

#[cfg(unix)]
#[test]
fn a_symlinked_root_shares_its_targets_places() {
    let d = tempfile::tempdir().unwrap();
    let real = d.path().join("real");
    tree(&real, &["a.rs"]);
    std::fs::write(real.join("a.rs"), "x\ny\n").unwrap();
    let link = d.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let mut s = session(d.path());
    open_project(&mut s, &link, "a.rs");
    caret_at_line(&mut s, 1);
    s.dispatch(Command::SetMnemonic('2'));
    drop(s);
    let projects = d.path().join("state/lenz-reader/projects");
    assert!(crate::places::file_for(&projects, &std::fs::canonicalize(&real).unwrap()).exists(), "named for the target");
    assert!(!crate::places::file_for(&projects, &normalized(&link)).exists(), "not for the link");
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(real.clone()));
    settle(&mut s, |s| s.state().places().mnemonic('2').is_some());
    assert_eq!(*s.state().places().mnemonic('2').unwrap().source, *normalized(&real.join("a.rs")));
}

#[cfg(unix)]
#[test]
fn a_places_file_under_the_old_name_is_read_and_saved_under_the_new() {
    let d = tempfile::tempdir().unwrap();
    let real = d.path().join("real");
    tree(&real, &["a.rs"]);
    let link = d.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let projects = d.path().join("state/lenz-reader/projects");
    let (old, new) = (crate::places::file_for(&projects, &normalized(&link)), crate::places::file_for(&projects, &std::fs::canonicalize(&real).unwrap()));
    let mut p = crate::places::Places::default();
    p.set_mnemonic(crate::places::Mark { source: normalized(&link.join("a.rs")).into(), line: 1, anchor: "y".into(), offset: 0, mnemonic: None, moved: false }, '3');
    crate::places::write_atomic(&old, &p.to_bytes(&normalized(&link)));
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(link.clone()));
    settle(&mut s, |s| s.state().places().mnemonic('3').is_some());
    drop(s);
    let saved = crate::places::read(&new, &normalized(&link)).unwrap();
    assert_eq!(saved.mnemonic('3').map(|m| m.line), Some(1));
}

/// Apply background results until `done` holds, holding back the project's places read: changes made meanwhile are
/// made before it.
fn settle_unread(s: &mut Session, done: impl Fn(&Session) -> bool) {
    let rx = s.completions();
    let until = std::time::Instant::now() + Duration::from_secs(10);
    while !done(s) {
        assert!(std::time::Instant::now() < until, "timed out");
        if let Ok(c) = rx.recv_blocking_timeout()
            && !matches!(c, Completion::PlacesRead { .. })
        {
            s.dispatch(c);
        }
    }
}

#[test]
fn a_bookmark_made_before_the_places_are_read_is_saved_with_them() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    tree(&root, &["a.rs"]);
    std::fs::write(root.join("a.rs"), "x\ny\nz\n").unwrap();
    let mut s = session(d.path());
    open_project(&mut s, &root, "a.rs");
    caret_at_line(&mut s, 0);
    s.dispatch(Command::SetMnemonic('1'));
    drop(s);
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    s.dispatch(Command::Open(root.join("a.rs")));
    settle_unread(&mut s, opened);
    caret_at_line(&mut s, 2);
    s.dispatch(Command::SetMnemonic('7'));
    drop(s);
    let saved = crate::places::read(&places_file(d.path(), &root), &normalized(&root)).unwrap();
    assert_eq!(saved.mnemonic('1').map(|m| m.line), Some(0), "the file's own stay");
    assert_eq!(saved.mnemonic('7').map(|m| m.line), Some(2), "and the early one is added");
}

#[test]
fn a_bookmark_made_before_an_unreadable_places_file_is_read_writes_nothing() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    tree(&root, &["a.rs"]);
    let file = places_file(d.path(), &root);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, b"{ broken").unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    s.dispatch(Command::Open(root.join("a.rs")));
    settle_unread(&mut s, opened);
    s.dispatch(Command::ToggleBookmark);
    drop(s);
    assert_eq!(std::fs::read(&file).unwrap(), b"{ broken");
    assert!(!file.with_extension("json.unreadable").exists());
}

#[test]
fn tabs_from_before_a_root_switch_are_not_recorded_in_the_new_project() {
    let d = tempfile::tempdir().unwrap();
    let (one, two) = (d.path().join("one"), d.path().join("two"));
    tree(&one, &["a.rs"]);
    tree(&two, &["b.rs"]);
    std::fs::write(one.join("a.rs"), "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n").unwrap();
    let mut s = session(d.path());
    open_project(&mut s, &one, "a.rs");
    s.dispatch(Command::OpenRoot(two.clone()));
    settle(&mut s, scanned);
    s.dispatch(Command::GoToLine(LineColumn { line: 12, column: None }));
    s.dispatch(Command::Open(two.join("b.rs")));
    settle(&mut s, opened);
    let p = s.state().places();
    let old = normalized(&one);
    assert!(!p.recent_files().iter().any(|f| f.starts_with(&old)), "{:?}", p.recent_files());
    assert!(!p.recent_locations().iter().any(|m| m.source.starts_with(&old)), "{:?}", p.recent_locations());
}

#[test]
fn a_file_opened_before_the_first_folder_is_recorded_in_it() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    tree(&root, &["a.rs", "b.rs"]);
    std::fs::write(root.join("a.rs"), "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n").unwrap();
    let mut s = session(d.path());
    // `lenz-reader a.rs`, then Open Folder on its folder.
    s.dispatch(Command::Open(root.join("a.rs")));
    settle(&mut s, opened);
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, scanned);
    s.dispatch(Command::GoToLine(LineColumn { line: 12, column: None }));
    s.dispatch(Command::Open(root.join("b.rs")));
    settle(&mut s, opened);
    let p = s.state().places();
    let a = normalized(&root.join("a.rs"));
    assert!(p.recent_files().contains(&a), "{:?}", p.recent_files());
    assert!(p.recent_locations().iter().any(|m| *m.source == *a), "{:?}", p.recent_locations());
}

#[test]
fn up_and_down_keep_the_drawn_column_across_hints() {
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    let path = write(d.path(), "a.txt", b"abcdefgh\nabcdefgh\n");
    s.dispatch(Command::Open(path));
    settle(&mut s, opened);
    s.dispatch(Command::SetSoftWrap(true));
    s.dispatch(Command::SetTextSize { pane: 0, cols: 40, rows: 10 });
    let version = s.state().current_source().unwrap().key().version;
    let hint = crate::decor::Hint { byte: 1, label: "XYZ".into(), kind: crate::decor::HintKind::Type };
    s.state.tabs[0].hints = Some((version, std::sync::Arc::new(crate::decor::Hints::new(vec![(1, hint)]))));
    s.state.tabs[0].decor_source = Some(SemanticSource::new(active_view(&s).snapshot()));
    // Column 5 of line 0 is drawn above "aXYZb|c…": byte 2 of line 1.
    s.dispatch(Command::Select(Selection { anchor: 5, head: 5 }));
    s.dispatch(Command::Extend(Motion::Down));
    assert_eq!(s.state().selection().unwrap().head, 9 + 2);
    s.dispatch(Command::Extend(Motion::Up));
    assert_eq!(s.state().selection().unwrap().head, 5);
}

#[test]
fn decoration_settings_default_on_are_saved_and_show_their_state() {
    let d = tempfile::tempdir().unwrap();
    let file = d.path().join("state/lenz-reader/settings.json");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, r#"{"semantic_highlighting": false}"#).unwrap();
    let mut s = session(d.path());
    assert!(s.state().inlay_types() && s.state().inlay_parameters() && !s.state().semantic_highlighting(), "a missing key reads as on");
    let on = |s: &Session, id: CommandId| s.actions().into_iter().find(|a| a.id == id).and_then(|a| a.on);
    assert_eq!((on(&s, CommandId::InlayTypes), on(&s, CommandId::SemanticHighlighting), on(&s, CommandId::Copy)), (Some(true), Some(false), None));
    assert_eq!(s.actions().iter().find(|a| a.id == CommandId::InlayParameters).map(|a| a.label), Some("Inlay Hints: Parameter Names"));
    s.dispatch(Command::SetInlayTypes(false));
    let saved = |key: &str| std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).and_then(|v| v.get(key)?.as_bool());
    settle(&mut s, |_| saved("inlay_types") == Some(false));
    assert_eq!(on(&s, CommandId::InlayTypes), Some(false));
}

#[test]
fn every_toggle_shows_its_state_in_the_menu() {
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    let on = |s: &Session, id: CommandId| s.actions().into_iter().find(|a| a.id == id).and_then(|a| a.on);
    let ids = [CommandId::SoftWrap, CommandId::StickyScroll, CommandId::Breadcrumbs, CommandId::DocumentationOnHover, CommandId::FileIcons, CommandId::MatchCase, CommandId::WholeWord, CommandId::RegularExpression];
    assert_eq!(ids.map(|id| on(&s, id)), [Some(false), Some(true), Some(true), Some(true), Some(true), Some(false), Some(false), Some(false)]);
    s.dispatch(Command::SetSoftWrap(true));
    s.dispatch(Command::SetStickyScroll(false));
    s.dispatch(Command::SetBreadcrumbs(false));
    s.dispatch(Command::SetDocumentationOnHover(false));
    s.dispatch(Command::SetFileIcons(false));
    for o in [FindOption::Case, FindOption::Word, FindOption::Regex] {
        s.dispatch(Command::ToggleFindOption(o));
    }
    assert_eq!(ids.map(|id| on(&s, id)), [Some(true), Some(false), Some(false), Some(false), Some(false), Some(true), Some(true), Some(true)]);
    // File Icons: in the View menu, always available, and remembered.
    assert_eq!((CommandId::FileIcons.group(), s.disabled(CommandId::FileIcons)), (crate::actions::CommandGroup::View, None));
    let saved = d.path().join("state/lenz-reader/settings.json");
    settle(&mut s, |_| !read_settings(&saved).file_icons);
    drop(s);
    assert!(!session(d.path()).state().file_icons());
}

#[test]
fn up_from_a_hints_byte_at_a_wrap_break_leaves_its_drawn_row() {
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    let path = write(d.path(), "a.txt", b"zz\nab cd\n");
    s.dispatch(Command::Open(path));
    settle(&mut s, opened);
    s.dispatch(Command::SetSoftWrap(true));
    s.dispatch(Command::SetTextSize { pane: 0, cols: 6, rows: 10 });
    let version = s.state().current_source().unwrap().key().version;
    let hint = crate::decor::Hint { byte: 2, label: ": xyz".into(), kind: crate::decor::HintKind::Type };
    s.state.tabs[0].hints = Some((version, std::sync::Arc::new(crate::decor::Hints::new(vec![(1, hint)]))));
    s.state.tabs[0].decor_source = Some(SemanticSource::new(active_view(&s).snapshot()));
    // Line 1 is drawn "ab: " then "xyz cd": the caret at its byte 2 is drawn at the end of the first row.
    let view = active_view(&s);
    let rows: Vec<_> = (0..2).map(|index| match view.layout_row(1, index, Some(6)) {
        crate::layout::Query::Ready(Some(row)) => row,
        _ => panic!("fixture hint row must be ready"),
    }).collect();
    assert_eq!(rows.iter().map(|r| r.display.clone()).collect::<Vec<_>>(), vec![0..4, 4..10]);
    s.dispatch(Command::Select(Selection { anchor: 5, head: 5 }));
    s.dispatch(Command::Extend(Motion::Up));
    assert_eq!(s.state().selection().unwrap().head, 2, "up to line 0 (its end, column 2), not within line 1");
}

#[test]
fn a_superseded_syntax_level_stops_reading_relations() {
    let text = "trait S {}\nimpl S for C {}\n";
    let doc = Arc::new(Document::decode("/a.rs".into(), 1, text.as_bytes()));
    let at = text.find("S for").unwrap();
    let hit = crate::semantics::hit_at(std::path::Path::new("/a.rs"), &doc, at, at + 1);
    let usage = || crate::semantics::Target { path: "/a.rs".into(), stamp: None, decoded_digest: None, doc: doc.clone(), hit: hit.clone(), at };
    let latest = Arc::new(std::sync::atomic::AtomicU64::new(5));
    let level = syntax_level(vec![usage()], Direction::Subtypes, crate::semantics::lsp::Encoding::Utf16, latest.clone(), 5);
    assert_eq!(level.iter().map(|(c, _)| c.name.as_str()).collect::<Vec<_>>(), ["C"]);
    assert!(syntax_level(vec![usage()], Direction::Subtypes, crate::semantics::lsp::Encoding::Utf16, latest, 4).is_empty(), "another level was asked for since");
}

#[test]
fn a_usage_past_the_snippet_columns_still_finds_its_relation() {
    let text = format!("trait S {{}}\n/*{}*/ impl S for C {{}}\n", "é".repeat(300)); // multibyte: bytes ≠ chars
    let doc = Arc::new(Document::decode("/a.rs".into(), 1, text.as_bytes()));
    let at = text.find("S for").unwrap();
    let hit = crate::semantics::hit_at(std::path::Path::new("/a.rs"), &doc, at, at + 1);
    assert!(hit.text.len() < at - doc.line_range(1).start, "the row's snippet stops before the usage");
    let usage = crate::semantics::Target { path: "/a.rs".into(), stamp: None, decoded_digest: None, doc: doc.clone(), hit, at };
    let level = syntax_level(vec![usage], Direction::Subtypes, crate::semantics::lsp::Encoding::Utf16, Arc::new(std::sync::atomic::AtomicU64::new(1)), 1);
    assert_eq!(level.iter().map(|(c, _)| c.name.as_str()).collect::<Vec<_>>(), ["C"]);
}

#[test]
fn a_regex_search_result_opens() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    tree(&root, &["a.rs"]);
    std::fs::write(root.join("a.rs"), "let fooo = 1;\n").unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, scanned);
    s.dispatch(Command::ToggleFindOption(FindOption::Regex));
    s.dispatch(Command::Search { query: "fo+".into(), scope: SearchIn::Project });
    settle(&mut s, searched);
    assert_eq!(s.state().results().unwrap().hits().len(), 1);
    s.dispatch(Command::OpenResult(0));
    settle(&mut s, |s| s.state().current_source().is_some() || !s.state().notices().is_empty());
    assert_eq!(s.state().notices(), &[] as &[String], "no notice");
    assert!(s.state().current_source().unwrap().source().ends_with("a.rs"));
}

#[test]
fn a_find_over_a_file_that_shrinks_on_disk_steps_safely() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "a.rs", b"a\nb\nc\nfoo\n");
    let mut s = session(d.path());
    open(&mut s, Command::Open(a.clone()), "a.rs");
    find(&mut s, "foo");
    let version = active_view(&s).snapshot().key().version;
    replace(&a, b"x\n");
    settle(&mut s, |s| active_view(s).snapshot().key().version > version);
    s.dispatch(Command::FindStep { forward: true }); // before the new matches arrive
    settle(&mut s, found);
    s.dispatch(Command::FindStep { forward: true });
    assert!(s.state().active().unwrap().find().unwrap().matches().is_empty(), "no match in the new text");
}

#[test]
fn the_old_path_of_a_followed_rename_opens_the_file_now_there() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "a.txt", b"content\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(a.clone()), "a.txt");
    let tab = s.state().active().unwrap().id();
    let original = s.state().view(tab).unwrap().snapshot().key().buffer;
    let moved = d.path().join("moved.txt");
    std::fs::rename(&a, &moved).unwrap();
    settle(&mut s, |s| s.state().active().unwrap().source() == moved);
    assert_eq!(s.state().view(tab).unwrap().snapshot().source(), moved);
    assert_eq!(s.state().view(tab).unwrap().snapshot().key().buffer, original);
    std::fs::write(&a, b"new\n").unwrap();
    s.dispatch(Command::Open(a.clone()));
    settle(&mut s, opened);
    assert_eq!(s.state().active().unwrap().source(), a, "a tab for a.txt");
    assert_eq!(source_line(s.state().current_source().unwrap(), 0), "new");
    assert_ne!(s.state().current().unwrap().key().buffer, original);
}

#[test]
fn an_open_at_superseded_before_it_lands_does_not_fire_later() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "a.rs", "x\n".repeat(200).as_bytes());
    let b = write(d.path(), "b.rs", b"fn b() {}\n");
    let mut s = session(d.path());
    s.dispatch(Command::OpenAt { path: a.clone(), at: LineColumn { line: 150, column: None } });
    s.dispatch(Command::Open(b));
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source().ends_with("b.rs")));
    open(&mut s, Command::Open(a), "a.rs");
    assert_eq!((s.state().top_line(), s.state().selection().map(|x| x.head)), (0, None), "a plain open shows the top");
}

#[test]
fn a_released_tabs_file_renamed_is_followed() {
    let d = tempfile::tempdir().unwrap();
    let (a, b) = (write(d.path(), "a.txt", b"alpha\n"), write(d.path(), "b.txt", b"beta\n"));
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(a.clone()), "a.txt");
    open(&mut s, Command::OpenRetained(b), "b.txt");
    let moved = d.path().join("m.txt");
    std::fs::rename(&a, &moved).unwrap();
    settle(&mut s, |s| s.state().tabs()[0].source() == moved || s.state().tabs()[0].unavailable().is_some());
    assert_eq!(s.state().tabs()[0].source(), moved, "followed, as the active tab's is");
}

#[test]
fn closing_the_split_from_an_empty_pane_keeps_a_file_shown() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "a.rs", b"fn a() {}\n");
    let mut s = session(d.path());
    open(&mut s, Command::Open(a), "a.rs");
    s.dispatch(Command::Split(Split::Columns));
    let t = s.state().shown(1).unwrap().id();
    // Closing the pane's last tab closes the split from that empty pane.
    s.dispatch(Command::CloseTab(t));
    assert_eq!(s.state().split(), None);
    assert!(s.state().active().is_some(), "a.rs is still shown");
}

#[test]
fn files_opened_after_a_root_switch_are_recorded() {
    let d = tempfile::tempdir().unwrap();
    let (one, two) = (d.path().join("one"), d.path().join("two"));
    tree(&one, &["a.rs"]);
    tree(&two, &["b.rs", "c.rs"]);
    let mut s = session(d.path());
    open_project(&mut s, &one, "a.rs");
    s.dispatch(Command::OpenRoot(two.clone()));
    settle(&mut s, scanned);
    s.dispatch(Command::Open(two.join("b.rs")));
    settle(&mut s, opened);
    s.dispatch(Command::Open(two.join("c.rs")));
    settle(&mut s, opened);
    let b = normalized(&two.join("b.rs"));
    assert!(s.state().places().recent_files().iter().any(|f| *f == b), "{:?}", s.state().places().recent_files());
}

#[test]
fn bookmarks_follow_a_followed_rename() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "a.txt", b"zero\none\ntwo\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(a.clone()), "a.txt");
    caret(&mut s, 1);
    s.dispatch(Command::ToggleBookmark);
    assert!(s.state().places().bookmarks().len() == 1, "precondition: one bookmark");
    let moved = d.path().join("moved.txt");
    std::fs::rename(&a, &moved).unwrap();
    settle(&mut s, |s| s.state().active().unwrap().source() == moved);
    let source = s.state().active().unwrap().source().to_owned();
    assert!(s.state().places().bookmark(&source, 1).is_some(), "{:?}", s.state().places().bookmarks());
}

#[test]
fn a_failed_bookmark_jump_does_not_fire_later() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "a.rs", "x\n".repeat(200).as_bytes());
    let b = write(d.path(), "b.rs", b"fn b() {}\n");
    let mut s = session(d.path());
    open(&mut s, Command::Open(a.clone()), "a.rs");
    caret(&mut s, 150);
    s.dispatch(Command::SetMnemonic('3'));
    open(&mut s, Command::Open(b), "b.rs"); // in a.rs's preview tab: a.rs has no tab now
    std::fs::remove_file(&a).unwrap();
    s.dispatch(Command::JumpMnemonic('3'));
    settle(&mut s, |s| s.state().notices().iter().any(|n| n.starts_with("Cannot open")));
    std::fs::write(&a, "x\n".repeat(200)).unwrap();
    open(&mut s, Command::Open(a), "a.rs");
    assert_eq!((s.state().top_line(), s.state().selection().map(|x| x.head)), (0, None), "a plain open shows the top");
}

#[test]
fn a_jump_whose_tab_closed_before_it_landed_does_not_fire_later() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "a.rs", "x\n".repeat(200).as_bytes());
    let b = write(d.path(), "b.rs", b"fn b() {}\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(b.clone()), "b.rs");
    s.dispatch(Command::OpenAt { path: a.clone(), at: LineColumn { line: 150, column: None } });
    let id = s.state().active().unwrap().id();
    s.dispatch(Command::CloseTab(id));
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source().ends_with("b.rs")) && s.state().loading().is_none());
    open(&mut s, Command::Open(a), "a.rs");
    assert_eq!((s.state().top_line(), s.state().selection().map(|x| x.head)), (0, None), "a plain open shows the top");
}

#[test]
fn a_jump_left_for_another_tab_before_it_landed_does_not_fire_later() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "a.rs", "x\n".repeat(200).as_bytes());
    let b = write(d.path(), "b.rs", b"fn b() {}\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(b.clone()), "b.rs");
    let bid = s.state().active().unwrap().id();
    s.dispatch(Command::OpenAt { path: a.clone(), at: LineColumn { line: 150, column: None } });
    s.dispatch(Command::ActivateTab(bid));
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source().ends_with("b.rs")) && s.state().loading().is_none());
    open(&mut s, Command::Open(a), "a.rs");
    assert_eq!((s.state().top_line(), s.state().selection().map(|x| x.head)), (0, None), "a plain open shows the top");
}

#[test]
fn next_bookmark_passes_a_bookmark_past_the_end_of_its_file() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "a.rs", b"0\n1\n2\n3\n4\n");
    let b = write(d.path(), "b.rs", b"fn b() {}\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(b.clone()), "b.rs");
    caret(&mut s, 0);
    s.dispatch(Command::ToggleBookmark);
    open(&mut s, Command::OpenRetained(a.clone()), "a.rs");
    caret(&mut s, 3);
    s.dispatch(Command::ToggleBookmark);
    let v = active_view(&s).snapshot().key().version;
    replace(&a, b"x\n");
    settle(&mut s, |s| active_view(s).snapshot().key().version > v);
    caret(&mut s, 0);
    s.dispatch(Command::StepBookmark { forward: true });
    settle(&mut s, opened);
    assert_eq!(caret_line(&s), 1, "the bookmark past the end: the last line");
    s.dispatch(Command::StepBookmark { forward: true });
    settle(&mut s, opened);
    assert!(s.state().current_source().unwrap().source().ends_with("b.rs"), "the second Next reaches b.rs: {:?}", s.state().places().bookmarks().iter().map(|m| (m.source.to_path_buf(), m.line)).collect::<Vec<_>>());
}

#[test]
fn a_change_on_disk_with_find_open_keeps_the_view() {
    let d = tempfile::tempdir().unwrap();
    let mut text = String::from("foo\n");
    text.push_str(&"x\n".repeat(300));
    let a = write(d.path(), "a.rs", text.as_bytes());
    let mut s = session(d.path());
    s.dispatch(Command::SetTextSize { pane: 0, cols: 80, rows: 20 });
    open(&mut s, Command::Open(a.clone()), "a.rs");
    find(&mut s, "foo");
    s.dispatch(Command::Scroll(200));
    let top = s.state().top_line();
    assert!(top >= 150, "precondition: scrolled away, top {top}");
    let v = active_view(&s).snapshot().key().version;
    text.push_str("appended\n");
    replace(&a, text.as_bytes());
    settle(&mut s, |s| active_view(s).snapshot().key().version > v && found(s));
    assert_eq!(s.state().top_line(), top, "an external append does not move the view");
}

#[test]
fn a_released_tabs_file_renamed_twice_is_followed_both_times() {
    let d = tempfile::tempdir().unwrap();
    let (a, b) = (write(d.path(), "a.txt", b"alpha\n"), write(d.path(), "b.txt", b"beta\n"));
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(a.clone()), "a.txt");
    open(&mut s, Command::OpenRetained(b), "b.txt");
    let (m, n) = (d.path().join("m.txt"), d.path().join("n.txt"));
    std::fs::rename(&a, &m).unwrap();
    settle(&mut s, |s| s.state().tabs()[0].source() == m);
    std::fs::rename(&m, &n).unwrap();
    settle(&mut s, |s| s.state().tabs()[0].source() == n || s.state().tabs()[0].unavailable().is_some());
    assert_eq!(s.state().tabs()[0].source(), n, "followed again");
}

#[test]
fn catalog_resolution_shares_availability_and_current_settings() {
    use crate::actions::{CatalogAction, InterfaceAction};
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    for id in CommandId::ALL {
        match (s.disabled(id), s.action(id)) {
            (Some(why), CatalogAction::Disabled(actual)) => assert_eq!(actual, why, "{id:?}"),
            (None, CatalogAction::Command(_) | CatalogAction::Interface(_)) => {},
            other => panic!("{id:?}: {other:?}"),
        }
    }
    assert_eq!(s.action(CommandId::Open), CatalogAction::Interface(InterfaceAction::Open));
    assert_eq!(s.action(CommandId::SoftWrap), CatalogAction::Command(Command::SetSoftWrap(!s.state().soft_wrap())));
    let CatalogAction::Command(c) = s.action(CommandId::SoftWrap) else { panic!("soft wrap") };
    s.dispatch(c);
    assert_eq!(s.action(CommandId::SoftWrap), CatalogAction::Command(Command::SetSoftWrap(!s.state().soft_wrap())));
}

#[test]
fn catalog_tab_navigation_wraps_in_the_focused_pane() {
    use crate::actions::CatalogAction;
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    let first = write(d.path(), "one.txt", b"one\n");
    let second = write(d.path(), "two.txt", b"two\n");
    s.dispatch(Command::OpenRetained(first));
    settle(&mut s, opened);
    let one = s.state().active().unwrap().id();
    s.dispatch(Command::OpenRetained(second));
    settle(&mut s, opened);
    let two = s.state().active().unwrap().id();
    let third = write(d.path(), "three.txt", b"three\n");
    s.dispatch(Command::OpenRetained(third));
    settle(&mut s, opened);
    let three = s.state().active().unwrap().id();
    assert_eq!(s.action(CommandId::NextTab), CatalogAction::Command(Command::ActivateTab(one)));
    assert_eq!(s.action(CommandId::PreviousTab), CatalogAction::Command(Command::ActivateTab(two)));
    let CatalogAction::Command(c) = s.action(CommandId::NextTab) else { panic!("next tab") };
    s.dispatch(c);
    assert_eq!(s.action(CommandId::NextTab), CatalogAction::Command(Command::ActivateTab(two)));
    assert_eq!(s.action(CommandId::PreviousTab), CatalogAction::Command(Command::ActivateTab(three)));
    s.dispatch(Command::Split(Split::Columns));
    assert!(matches!(s.action(CommandId::NextTab), CatalogAction::Disabled(_)));
    s.dispatch(Command::FocusPane(0));
    assert_eq!(s.action(CommandId::NextTab), CatalogAction::Command(Command::ActivateTab(two)));
    assert_eq!(std::fs::read(d.path().join("one.txt")).unwrap(), b"one\n");
}

#[test]
fn catalog_find_seed_uses_single_line_selection_then_the_tabs_query() {
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    assert_eq!(s.find_seed(), "");
    let file = write(d.path(), "seed.txt", b"first\nsecond\n");
    s.dispatch(Command::Open(file));
    settle(&mut s, opened);
    s.dispatch(Command::Find("old".into()));
    s.dispatch(Command::Select(Selection { anchor: 0, head: 5 }));
    assert_eq!(s.find_seed(), "first");
    s.dispatch(Command::Select(Selection { anchor: 0, head: 8 }));
    assert_eq!(s.find_seed(), "old");
    s.dispatch(Command::Select(Selection { anchor: 0, head: 0 }));
    assert_eq!(s.find_seed(), "old");
}

#[test]
fn extending_selection_keeps_its_head_in_the_reported_viewport_with_and_without_wrap() {
    for wrap in [false, true] {
        let d = tempfile::tempdir().unwrap();
        let text = (0..100).map(|i| format!("line {i}: abcdefghijklmnopqrstuvwxyz\n")).collect::<String>();
        let file = write(d.path(), "selection.txt", text.as_bytes());
        let mut s = session(d.path());
        s.dispatch(Command::Open(file.clone()));
        settle(&mut s, opened);
        s.dispatch(Command::SetTextSize { pane: 0, cols: 18, rows: 8 });
        s.dispatch(Command::SetSoftWrap(wrap));
        s.dispatch(Command::Select(Selection { anchor: 0, head: 0 }));
        for _ in 0..30 {
            s.dispatch(Command::Extend(Motion::Down));
            let tab = s.state().active().unwrap();
            let view = s.state().view(tab.id()).unwrap();
            let head = tab.selection().unwrap().head;
            let line = view.snapshot().line_of(head);
            let cols = s.state().wrap_cols(tab);
            let crate::layout::Query::Ready(Some(position)) = view.layout_position(head, cols) else { panic!("fixture caret layout pending") };
            assert!(source_rows(view, 8, cols).iter().any(|r| r.line == line && r.index == position.row), "selection head left the viewport (wrap={wrap})");
        }
        assert_eq!(std::fs::read(file).unwrap(), text.as_bytes());
    }
}

#[test]
fn workspace_restores_tabs_positions_split_and_explorer_without_loading_inactive_files() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project");
    tree(&root, &["src/a.txt", "src/b.txt", "c.txt"]);
    let source = "héllo workspace\n".repeat(120);
    for file in ["src/a.txt", "src/b.txt", "c.txt"] { std::fs::write(root.join(file), &source).unwrap(); }
    let mut one = session(d.path());
    open_project(&mut one, &root, "src/a.txt");
    one.dispatch(Command::Pin(one.state.active.unwrap()));
    one.dispatch(Command::Select(Selection { anchor: 8, head: 3 }));
    one.dispatch(Command::Scroll(25));
    one.dispatch(Command::ScrollColumn(3));
    one.dispatch(Command::OpenRetained(root.join("src/b.txt")));
    settle(&mut one, opened);
    one.dispatch(Command::OpenRetained(root.join("c.txt")));
    settle(&mut one, opened);
    one.dispatch(Command::Split(Split::Rows));
    one.dispatch(Command::FocusPane(1));
    one.dispatch(Command::Scroll(40));
    one.dispatch(Command::SetLayout(Layout { explorer_width: Some(220), split_permille: Some(650), results_permille: None }));
    one.dispatch(Command::RevealInTree("src/a.txt".into()));
    one.dispatch(Command::SetExplorer(crate::workspace::Explorer { visible: false, selected: Some("src/b.txt".into()), top: Some("src".into()) }));
    let snapshot = |s: &Session| s.state.tabs.iter().map(|t| (t.source.clone(), t.pane, t.retained, t.selection, t.top_line, t.column)).collect::<Vec<_>>();
    let expected = snapshot(&one);
    let active = one.state.active().unwrap().source.clone();
    drop(one);
    let mut two = session(d.path());
    two.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut two, |s| scanned(s) && s.state.current_source().is_some() && s.state.shown(0).is_some_and(|t| t.loaded));
    assert_eq!(snapshot(&two), expected);
    assert_eq!(two.state.active().unwrap().source, active);
    assert_eq!(two.state.focused, 1);
    assert_eq!(two.state.split(), Some(Split::Rows));
    assert_eq!(two.state.layout().explorer_width, Some(220));
    assert!(!two.state.explorer.visible);
    assert!(two.state.project().unwrap().rows().iter().any(|r| &*r.path == "src/a.txt"));
    assert_eq!(two.state.tabs.iter().filter(|t| t.loaded).count(), 2, "only the two shown tabs are loaded");
    assert_eq!(std::fs::read_to_string(root.join("src/a.txt")).unwrap(), source);
}

#[test]
fn workspace_directory_switches_flush_old_state_and_restore_each_directory() {
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("one"); let b = d.path().join("two");
    tree(&a, &["a.txt"]); tree(&b, &["b.txt"]);
    let mut s = session(d.path());
    open_project(&mut s, &a, "a.txt");
    s.dispatch(Command::OpenRoot(b.clone())); settle(&mut s, scanned);
    assert!(s.state.tabs.is_empty());
    s.dispatch(Command::Open(b.join("b.txt"))); settle(&mut s, opened);
    s.dispatch(Command::OpenRoot(a.clone()));
    settle(&mut s, |s| s.state.current_source().is_some_and(|d| d.source() == a.join("a.txt")));
    assert_eq!(s.state.tabs.len(), 1);
    s.dispatch(Command::OpenRoot(b.clone()));
    settle(&mut s, |s| s.state.current_source().is_some_and(|d| d.source() == b.join("b.txt")));
    assert_eq!(s.state.tabs.len(), 1);
}

#[test]
fn workspace_skips_missing_tabs_and_clamps_positions_in_changed_utf8_files() {
    let d = tempfile::tempdir().unwrap(); let root = d.path().join("project");
    tree(&root, &["a.txt", "b.txt", "c.txt"]);
    std::fs::write(root.join("b.txt"), "long text\n".repeat(100)).unwrap();
    let mut s = session(d.path());
    open_project(&mut s, &root, "a.txt");
    s.dispatch(Command::Pin(s.state.active.unwrap()));
    s.dispatch(Command::OpenRetained(root.join("b.txt"))); settle(&mut s, opened);
    s.dispatch(Command::Select(Selection { anchor: 800, head: 801 })); s.dispatch(Command::Scroll(90));
    s.dispatch(Command::OpenRetained(root.join("c.txt"))); settle(&mut s, opened);
    drop(s);
    std::fs::remove_file(root.join("a.txt")).unwrap(); std::fs::remove_file(root.join("c.txt")).unwrap();
    std::fs::write(root.join("b.txt"), "é\n").unwrap();
    let mut s = session(d.path()); s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, |s| s.state.current_source().is_some());
    assert_eq!(s.state.tabs.len(), 1);
    assert!(s.state.active().unwrap().source.ends_with("b.txt"));
    assert!(s.state.top_line() < s.state.current_source().unwrap().line_count());
    for at in [s.state.selection().unwrap().anchor, s.state.selection().unwrap().head] { assert!(source_text(s.state.current_source().unwrap()).is_char_boundary(at)); }
}

#[test]
fn workspace_late_restore_does_not_replace_a_file_the_user_just_opened() {
    let d = tempfile::tempdir().unwrap(); let root = d.path().join("project"); tree(&root, &["a.txt", "b.txt"]);
    let mut s = session(d.path()); open_project(&mut s, &root, "a.txt"); drop(s);
    let mut s = session(d.path()); s.dispatch(Command::OpenRoot(root.clone()));
    let completion = loop { let c = s.completions().recv_blocking_timeout().unwrap(); if matches!(c, Completion::PlacesRead { .. }) { break c; } s.dispatch(c); };
    s.dispatch(Command::Open(root.join("b.txt")));
    s.dispatch(completion);
    settle(&mut s, opened);
    assert_eq!(s.state.tabs.len(), 1);
    assert!(s.state.active().unwrap().source.ends_with("b.txt"));
}

#[test]
fn workspace_closing_before_restore_finishes_keeps_the_saved_tabs() {
    let d = tempfile::tempdir().unwrap(); let root = d.path().join("project"); tree(&root, &["a.txt"]);
    let mut s = session(d.path()); open_project(&mut s, &root, "a.txt"); drop(s);
    let file = places_file(d.path(), &root);
    let before: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    let mut s = session(d.path()); s.dispatch(Command::OpenRoot(root)); drop(s);
    let after: serde_json::Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    assert_eq!(before["workspace"], after["workspace"]);
}

#[test]
fn unified_navigation_finds_files_symbols_and_actions_without_loading_search_candidates() {
    use crate::navigation::{Chosen, Kind, Scope as N};
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "wrap.rs", b"// source\nfn wrap() {}\n");
    let mut s = session(dir.path());
    s.dispatch(Command::OpenRoot(dir.path().to_owned()));
    settle(&mut s, |s| s.state().project().is_some_and(|p| matches!(p.coverage(), project::Coverage::Complete(_))) && s.state().index_coverage().complete());
    s.dispatch(Command::SearchEverywhere {scope: N::All, query: "wrap".into()});
    settle(&mut s, |s| s.state().navigation().is_some_and(|n| !n.pending));
    let n = s.state().navigation().unwrap();
    for kind in [Kind::File,Kind::Symbol,Kind::Action] { assert!(n.rows.iter().any(|r| r.kind == kind), "{kind:?}: {:?}", n.rows); }
    assert!(s.state().current_source().is_none() && s.state().results().is_none(), "ranking must not open documents or replace search results");
    let row = n.rows.iter().position(|r| r.kind == Kind::Symbol && r.label == "wrap").unwrap();
    let chosen = s.navigation_choice(n.revision, row).unwrap();
    let Chosen::Command(command) = chosen else { panic!("expected source navigation") };
    s.dispatch(command);
    settle(&mut s, |s| s.state().current_source().is_some() && s.state().active().is_some_and(|t| t.selection().is_some()));
    assert!(s.state().navigation().is_none(), "validated activation closes the picker after success");
    assert_eq!(s.state().active().unwrap().source(), path);
    assert_eq!(s.state().selection().unwrap().head, "// source\n".len());
    assert_eq!(std::fs::read(path).unwrap(), b"// source\nfn wrap() {}\n");
}

#[test]
fn unified_navigation_rejects_stale_queries_indices_closure_and_replaced_roots() {
    use crate::navigation::Scope as N;
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    s.dispatch(Command::SearchEverywhere {scope:N::Actions,query:"wrap".into()});
    settle(&mut s, |s| s.state().navigation().is_some_and(|n| !n.pending));
    let old = s.state().navigation().unwrap().clone();
    assert!(s.navigation_choice(old.revision, usize::MAX).is_none());
    assert!(s.navigation_choice(old.revision.wrapping_add(1), 0).is_none());
    s.dispatch(Command::SearchEverywhere {scope:N::Actions,query:"reload".into()});
    assert!(!s.dispatch(Completion::Navigation {request:old.request,files:0,indexed:0,found:Some((old.rows.clone(),1))}).repaint);
    assert!(s.navigation_choice(old.revision,0).is_none());
    settle(&mut s, |s| s.state().navigation().is_some_and(|n| !n.pending));
    let n = s.state().navigation().unwrap();
    assert_eq!(n.rows[0].label,"Reload");
    assert_eq!(n.rows[0].disabled,Some("No file is open"));
    assert!(s.navigation_choice(n.revision,0).is_none());
    s.dispatch(Command::CloseSearchEverywhere);
    s.dispatch(Completion::Navigation {request:old.request,files:0,indexed:0,found:Some((old.rows.clone(),1))});
    assert!(s.state().navigation().is_none());
    s.dispatch(Command::SearchEverywhere {scope:N::Actions,query:"".into()});
    s.dispatch(Command::OpenRoot(dir.path().join("other")));
    s.dispatch(Completion::Navigation {request:old.request,files:0,indexed:0,found:Some((old.rows,1))});
    assert!(s.state().navigation().is_none());
}

#[test]
fn unified_navigation_updates_live_action_values_and_deduplicates_recent_files() {
    use crate::navigation::{Kind,Scope as N};
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(),"opened.txt",b"read only\n");
    let mut s = session(dir.path());
    s.dispatch(Command::Open(path.clone()));
    settle(&mut s, opened);
    s.dispatch(Command::SearchEverywhere {scope:N::Actions,query:"soft wrap".into()});
    settle(&mut s, |s| s.state().navigation().is_some_and(|n| !n.pending));
    let revision = s.state().navigation().unwrap().revision;
    assert_eq!(s.state().navigation().unwrap().rows[0].on,Some(false));
    s.dispatch(Command::SetSoftWrap(true));
    assert_eq!(s.state().navigation().unwrap().revision,revision);
    assert_eq!(s.state().navigation().unwrap().rows[0].on,Some(true));
    s.dispatch(Command::SearchEverywhere {scope:N::All,query:"".into()});
    settle(&mut s, |s| s.state().navigation().is_some_and(|n| !n.pending));
    assert_eq!(s.state().navigation().unwrap().rows.iter().filter(|r|r.kind == Kind::File && r.label == "opened.txt").count(),1);
    assert_eq!(std::fs::read(path).unwrap(),b"read only\n");
}

#[test]
fn unified_navigation_refreshes_symbols_when_a_same_size_index_snapshot_is_replaced() {
    use crate::navigation::Scope as N;
    let dir = tempfile::tempdir().unwrap();
    let path=write(dir.path(),"a.rs",b"fn alpha() {}\n");
    let mut s=session(dir.path());
    s.dispatch(Command::OpenRoot(dir.path().to_owned()));
    settle(&mut s,|s|s.state().project().is_some_and(|p|matches!(p.coverage(),project::Coverage::Complete(_)))&&s.state().index_coverage().complete());
    s.dispatch(Command::SearchEverywhere{scope:N::Symbols,query:"alpha".into()});
    let request=s.state().navigation().unwrap().request;
    std::fs::write(&path,b"fn bravo() {}\n").unwrap();
    let rel:Arc<str>="a.rs".into();
    s.state.index.invalidate(&rel,true);
    assert_eq!(s.state.index.next_chunk().as_slice(),std::slice::from_ref(&rel));
    let indexed=index::index_file(dir.path(),&rel,||true);
    let generation=s.state().project().unwrap().generation();
    s.dispatch(Completion::Indexed{generation,revision:s.index_revision.load(Ordering::Relaxed),files:vec![(rel,indexed)]});
    assert!(s.state().navigation().unwrap().dirty);
    settle(&mut s,|s|s.state().navigation().is_some_and(|n|!n.pending&&n.request!=request));
    assert!(s.state().navigation().unwrap().rows.is_empty(),"the old declaration must not survive a same-count replacement");
}

#[test]
fn find_action_lists_every_command_however_many() {
    use crate::navigation::Scope as N;
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    s.dispatch(Command::SearchEverywhere { scope: N::Actions, query: String::new() });
    settle(&mut s, |s| s.state().navigation().is_some_and(|n| !n.pending));
    let listed = s.state().navigation().unwrap().rows.len();
    assert!(crate::navigation::LIMIT < CommandId::ALL.len(), "more commands than the other lists' limit");
    assert_eq!(listed, s.actions().len(), "every command is reachable without typing");
}

#[test]
fn unified_navigation_removes_symbols_after_the_last_source_is_deleted() {
    use crate::navigation::Scope as N;
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "a.rs", b"fn alpha() {}\n");
    let mut s = session(dir.path());
    s.dispatch(Command::OpenRoot(dir.path().to_owned()));
    settle(&mut s, |s| s.state().project().is_some_and(|p| matches!(p.coverage(), project::Coverage::Complete(_))) && s.state().index_coverage().complete());
    s.dispatch(Command::SearchEverywhere { scope: N::Symbols, query: "alpha".into() });
    settle(&mut s, |s| s.state().navigation().is_some_and(|n| !n.pending));
    let old = s.state().navigation().unwrap().revision;
    assert_eq!(s.state().navigation().unwrap().rows[0].label, "alpha");
    std::fs::remove_file(&path).unwrap();
    s.dispatch(Completion::Changed(Change { paths: [path].into(), structure: true, ..Change::default() }));
    settle(&mut s, |s| s.state().project().is_some_and(|p| !p.refreshing()) && s.state().navigation().is_some_and(|n| !n.pending));
    assert!(s.state().navigation().unwrap().rows.is_empty(), "deleted declarations must disappear without another indexed file completing");
    assert!(s.navigation_choice(old, 0).is_none());
}

#[test]
fn pyright_failures_offer_runtime_repair_and_version_context() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "main.py", b"def main(): pass\n");
    let mut s = session(d.path());
    s.dispatch(Command::Open(path)); settle(&mut s, opened);
    assert_eq!(s.disabled(CommandId::ServerRuntime), None, "Python must offer an in-app runtime repair");
    let details = s.language_server_details();
    assert!(details.contains("Server version:"));
    assert!(details.contains("1.1.414"), "show the tested Pyright version independently of a failed launch");
    s.dispatch(Command::OpenServerRuntime);
    assert!(s.state().server_runtime().is_some());
    assert!(s.state().server_runtime().unwrap().message.contains("Node"));
}

#[test]
fn caret_arrows_move_without_selecting_and_reveal_unicode_columns_and_crlf() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "caret.txt", "éx\r\nabcdef\n終端\n".as_bytes());
    let mut s = session(d.path()); s.dispatch(Command::Open(path.clone())); settle(&mut s, opened);
    s.dispatch(Command::SetTextSize { pane: 0, cols: 2, rows: 2 });
    s.dispatch(Command::MoveCaret(Motion::Right));
    assert_eq!(s.state().selection(), Some(Selection { anchor: 2, head: 2 }));
    s.dispatch(Command::MoveCaret(Motion::Down));
    assert_eq!(s.state().selection().unwrap().head, 6, "keep display column 1, not UTF-8 byte column 2");
    s.dispatch(Command::MoveCaret(Motion::Right));
    assert_eq!(s.state().selection(), Some(Selection { anchor: 7, head: 7 }));
    let goal = s.native_reveal(0).expect("navigation requests native horizontal measurement").clone();
    assert_eq!(goal.source, 7);
    s.dispatch(Command::MeasuredReveal { goal, cols: 2, bounds: 2..3 });
    assert!(s.state().column() > 0, "reveal the caret horizontally after exact native feedback");
    s.dispatch(Command::Select(Selection {anchor:0,head:3}));
    s.dispatch(Command::MoveCaret(Motion::Left));
    assert_eq!(s.state().selection(), Some(Selection {anchor:0,head:0}), "Left collapses a range to its start");
    s.dispatch(Command::Select(Selection {anchor:0,head:3}));
    s.dispatch(Command::MoveCaret(Motion::Right));
    assert_eq!(s.state().selection(), Some(Selection {anchor:3,head:3}));
    s.dispatch(Command::MoveCaret(Motion::Right));
    assert_eq!(s.state().selection().unwrap().head,5,"CRLF is crossed as one line break");
    s.dispatch(Command::MoveCaret(Motion::Left));
    assert_eq!(s.state().selection().unwrap().head,3);
    assert_eq!(std::fs::read(path).unwrap(),"éx\r\nabcdef\n終端\n".as_bytes());
}

#[test]
fn caret_vertical_movement_uses_expanded_tabs_and_inlay_columns() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "caret.txt", b"abcdef\n\tab\n");
    let mut s = session(d.path());
    s.dispatch(Command::Open(path)); settle(&mut s, opened);
    s.dispatch(Command::Select(Selection { anchor: 6, head: 6 }));
    s.dispatch(Command::MoveCaret(Motion::Down));
    assert_eq!(s.state().selection().unwrap().head, 10, "expanded tab preserves display column six");
    let version = s.state().current_source().unwrap().key().version;
    let hint = crate::decor::Hint { byte: 1, label: "XYZ".into(), kind: crate::decor::HintKind::Type };
    s.state.tabs[0].hints = Some((version, Arc::new(crate::decor::Hints::new(vec![(1, hint)]))));
    s.state.tabs[0].decor_source = Some(SemanticSource::new(active_view(&s).snapshot()));
    s.dispatch(Command::Select(Selection { anchor: 6, head: 6 }));
    s.dispatch(Command::MoveCaret(Motion::Down));
    assert_eq!(s.state().selection().unwrap().head, 8, "column six falls in the hint after the tab");
    s.dispatch(Command::Select(Selection { anchor: 10, head: 10 }));
    s.dispatch(Command::MoveCaret(Motion::Up));
    assert_eq!(s.state().selection().unwrap().head, 6, "a shorter source line clamps to its end");
}

#[test]
fn recent_navigation_uses_only_mru_files_and_validates_forget_identity() {
    use crate::navigation::{Scope,Chosen};
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("src")).unwrap(); std::fs::create_dir_all(d.path().join("tests")).unwrap();
    let a=write(d.path(),"src/Same.txt",b"a\n"); let b=write(d.path(),"tests/Same.txt",b"b\n");
    write(d.path(),"unopened.txt",b"not recent\n");
    let mut s=session(d.path()); s.dispatch(Command::OpenRoot(d.path().into())); settle(&mut s,scanned);
    s.dispatch(Command::Open(a.clone()));settle(&mut s,opened);
    s.dispatch(Command::Open(b.clone()));settle(&mut s,opened);
    s.dispatch(Command::SearchEverywhere{scope:Scope::Recent,query:String::new()});
    assert!(!s.state().navigation().unwrap().pending, "Ctrl-E, Enter can immediately choose the previous file");
    settle(&mut s,|s|s.state().navigation().is_some_and(|n|!n.pending));
    let old=s.state().navigation().unwrap().revision;
    assert_eq!(s.state().navigation().unwrap().rows.len(),2);
    assert_eq!(s.state().navigation().unwrap().rows[0].detail,"src/Same.txt");
    let Some(Chosen::Command(Command::GoToPlace(Place::File(path))))=s.navigation_choice(old,0) else {panic!("recent selection restores a file place")};
    assert_eq!(path,a);
    s.dispatch(Command::ForgetNavigation{revision:old.wrapping_add(1),index:0});
    assert_eq!(s.state().navigation().unwrap().rows.len(),2);
    s.dispatch(Command::ForgetNavigation{revision:old,index:0});
    settle(&mut s,|s|s.state().navigation().is_some_and(|n|!n.pending&&n.revision!=old));
    assert_eq!(s.state().navigation().unwrap().rows.len(),1);
    assert!(s.navigation_choice(old,0).is_none());
    assert!(a.exists()&&b.exists(),"forgetting never deletes source files");
}

#[test]
fn status_details_keeps_complete_messages_while_brief_and_controls_stay_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let long = format!("Runtime error: {}\nInstall the compatible runtime", "日本語e\u{301}".repeat(120));
    s.state.messages.push(long.clone());
    assert_eq!(s.status_message(), long);
    assert!(s.status_details().contains(&long));
    assert!(lenz_content::wrap::width(&crate::chrome::ellipsize(&s.status_message(), 40)) <= 40);
    assert_eq!(s.action(CommandId::StatusDetails), crate::actions::CatalogAction::Interface(crate::actions::InterfaceAction::StatusDetails));
    assert!(s.toolbar().iter().any(|(id, _)| *id == CommandId::FindFile));
    assert_eq!(CommandId::StatusDetails.shortcut(), Some("Shift+F2"));
}

#[test]
fn server_menus_share_actions_and_keep_diagnostics_in_copyable_details() {
    use crate::{actions::{CatalogAction, CommandGroup, InterfaceAction}, chrome::MenuTarget};
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    for path in [None, Some(write(d.path(), "note.txt", b"plain text")), Some(write(d.path(), "main.py", b"def main(): pass"))] {
        if let Some(path) = path { s.dispatch(Command::Open(path)); settle(&mut s, opened); }
        let rows = s.language_server_rows();
        assert_eq!(rows.len(), 9);
        assert_eq!(rows[0].1, Some(CommandId::ServerDetails));
        assert!(rows.iter().all(|(_, action)| action.is_some() && *action != Some(CommandId::CopyServerDetails)));
        let menu = s.menu(Some(CommandGroup::LanguageServers));
        assert_eq!(menu.len(), rows.len() + 1);
        for (menu, (label, id)) in menu.iter().skip(1).zip(rows) {
            let id = id.unwrap();
            assert_eq!((&menu.label, menu.target, menu.disabled), (&label, MenuTarget::Run(id), s.disabled(id)));
        }
        assert_eq!(s.action(CommandId::ServerDetails), CatalogAction::Interface(InterfaceAction::ServerDetails));
        assert_eq!(s.action(CommandId::CopyServerDetails), CatalogAction::Command(Command::CopyText(s.language_server_details())));
        assert!(s.status_details().contains(&s.language_server_details()));
    }
}

#[test]
fn changing_projects_rejects_pending_library_discovery() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let root = dir.path().join("one");
    std::fs::create_dir_all(&root).unwrap();
    s.dispatch(Command::OpenRoot(root));
    let generation = s.project_generation.load(Ordering::Relaxed);
    let path = dir.path().join("node_modules/package/index.ts");
    let read = s.state.libraries.reveal(&path, true).pop().unwrap().read(|| true);
    let next = dir.path().join("two");
    std::fs::create_dir_all(&next).unwrap();
    s.dispatch(Command::OpenRoot(next.clone()));
    assert!(!s.dispatch(Completion::LibraryRead { generation, read }).repaint);
    assert_eq!(s.state.project.as_ref().unwrap().root(), next);
    assert!(!s.state.tree_rows().any(|r| r.name.as_ref() == "Libraries"));
}

#[test]
fn about_is_in_the_application_menu_and_names_the_version() {
    let d = tempfile::tempdir().unwrap();
    let s = session(d.path());
    let rows = s.menu(None);
    let at = rows.iter().position(|r| r.target == crate::chrome::MenuTarget::Run(CommandId::About)).unwrap();
    assert_eq!(rows[at - 1].target, crate::chrome::MenuTarget::Run(CommandId::ChooseTheme), "right after Change theme");
    assert_eq!((rows[at].label.as_str(), rows[at].disabled), ("About Lenz Code", None));
    assert_eq!(CommandId::About.group(), crate::actions::CommandGroup::Application);
    assert!(matches!(s.action(CommandId::About), crate::actions::CatalogAction::Interface(crate::actions::InterfaceAction::About)));
    let text = crate::chrome::about();
    assert!(text.starts_with("Code reader") && text.contains(&format!("Version: {}", env!("CARGO_PKG_VERSION"))) && text.contains("Project: https://github.com/aantoniadis/lenz"), "{text}");
}

fn open_host(vars: &[(&str, &str)]) -> lenz_content::open::Host {
    let vars: std::collections::HashMap<String, String> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    lenz_content::open::Host { var: Box::new(move |k| vars.get(k).cloned()), on_path: Box::new(|_| false), macos: false }
}

#[test]
fn open_in_editor_goes_to_the_caret_and_compare_with_starts_lenz_diff_beside_it() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::create_dir_all(root.join("other")).unwrap();
    let a = write(&root, "a.rs", "fn a() {}\nlet é = 1;\n".as_bytes());
    let b = write(&root, "b.rs", b"fn b() {}\n");
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    open(&mut s, Command::Open(a.clone()), "a.rs");
    s.dispatch(Command::Select(Selection { anchor: 16, head: 16 }));
    let plan = s.editor_plan(&open_host(&[("VISUAL", "code")]), None, false).unwrap();
    let file = a.to_string_lossy().into_owned();
    assert_eq!((plan.launch.program.as_str(), plan.launch.args.clone()), ("code", vec!["--goto".to_string(), format!("{file}:2:6")]), "line 2, after `let é`");
    assert_eq!(plan.launch.cwd.as_deref(), Some(root.as_path()));
    let system = s.editor_plan(&open_host(&[("VISUAL", "code")]), None, true).unwrap();
    assert_eq!((system.launch.program.as_str(), system.launch.args.clone()), ("xdg-open", vec![file.clone()]));
    let fallback = s.editor_plan(&open_host(&[]), None, false).unwrap();
    assert_eq!(s.editor_plan(&open_host(&[]), Some("zed"), false).unwrap().launch.args, [format!("{file}:2:6")], "the settings file's editor");
    assert_eq!((fallback.launch.program.as_str(), fallback.toast.as_deref()), ("xdg-open", Some(lenz_content::open::NO_EDITOR_TOAST)));

    assert_eq!(s.compare_source(None).unwrap(), a);
    assert_eq!(s.compare_source(Some("sub")).unwrap(), root.join("sub"));
    assert!(s.compare_source(Some("jdt://contents/x")).is_err());
    let bin = d.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let exe = bin.join("lenz-code");
    let missing = s.compare_launch(&a, &b, &exe, false).unwrap_err();
    assert!(missing.contains("Lenz Diff is not beside Lenz Code"), "{missing}");
    for name in ["lenz-diff", "lenz-diff-tui"] {
        std::fs::write(bin.join(name), b"").unwrap();
    }
    let launch = s.compare_launch(&a, &b, &exe, false).unwrap();
    assert_eq!(launch.program, bin.join("lenz-diff").to_string_lossy());
    assert_eq!(launch.args, ["--no-index".to_string(), file.clone(), b.to_string_lossy().into_owned()]);
    assert_eq!(s.compare_launch(&root.join("sub"), &root.join("other"), &exe, true).unwrap().program, bin.join("lenz-diff-tui").to_string_lossy());
    assert!(s.compare_launch(&a, &root.join("sub"), &exe, false).unwrap_err().contains("a folder with a folder"));
    assert!(s.compare_launch(&a, &a, &exe, false).unwrap_err().contains("different"));
    assert!(s.compare_launch(&a, &root.join("gone.rs"), &exe, false).unwrap_err().contains("no such file"));

    s.dispatch(Command::Notice("Cannot start code: gone".into()));
    assert_eq!(s.status_message(), "Cannot start code: gone", "both status bars show it");
    let menu = s.context_menu();
    for id in [CommandId::OpenInEditor, CommandId::OpenWithDefaultApp, CommandId::CompareWith, CommandId::SplitRight, CommandId::SplitDown] {
        assert!(menu.contains(&MenuEntry::Command(id)), "{id:?}");
    }
    assert!(!menu.contains(&MenuEntry::Command(CommandId::Unsplit)));
    assert!(!s.toolbar().iter().any(|(id, _)| *id == CommandId::Open), "Open stays in the File menu, not the toolbar");
}

mod blame {
    use super::*;
    use crate::blame::ANNOTATION_COLS;
    use lenz_content::git::testing::{commit_all, git, repo, write as put};

    /// The active tab's annotation labels, line by line ("" for a line without one).
    fn labels(s: &Session) -> Vec<String> {
        let t = s.state().active().unwrap();
        (0..s.state().view(t.id()).unwrap().line_count()).map(|l| s.annotation(t.id(), l).map_or(String::new(), |a| a.label)).collect()
    }
    fn blamed(s: &Session) -> bool {
        s.state().active().and_then(|t| t.blame.as_ref()).is_some_and(|b| b.done)
    }
    fn request(s: &Session) -> u64 {
        s.state().active().unwrap().blame.as_ref().unwrap().request
    }
    fn annotate(s: &mut Session) {
        s.dispatch(Command::SetAnnotate(true));
        settle(s, blamed);
    }
    fn today() -> String {
        lenz_content::blame::date(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64)
    }

    #[test]
    fn annotations_stream_for_a_committed_file() {
        let r = repo();
        put(r.path(), "a.txt", "one\ntwo\n");
        commit_all(r.path(), "first");
        let mut s = session(r.path());
        s.dispatch(Command::Open(r.path().join("a.txt")));
        settle(&mut s, opened);
        assert_eq!(s.state().active().unwrap().annotation_cols(), 0, "off: no column");
        assert_eq!(s.on(CommandId::AnnotateBlame), Some(false));
        annotate(&mut s);
        let label = format!("{} t", today());
        assert_eq!(&labels(&s)[..2], [label.clone(), label]);
        let t = s.state().active().unwrap().id();
        assert_eq!(s.state().active().unwrap().annotation_cols(), ANNOTATION_COLS);
        assert_eq!(s.annotation(t, 0).unwrap().shade, Some(1.0), "one commit: all newest");
        assert_eq!(s.on(CommandId::AnnotateBlame), Some(true));
        s.dispatch(Command::SetAnnotate(false));
        assert_eq!((s.state().active().unwrap().annotation_cols(), s.annotation(t, 0).is_none()), (0, true));
    }

    #[test]
    fn lossless_legacy_encoding_keeps_unchanged_commits_when_one_draft_line_changes() {
        let r = repo();
        let bytes = b"caf\xe9 au lait, cr\xe8me br\xfbl\xe9e\r\n\xe9l\xe8ve d\xe9j\xe0 vu\r\nlast\r\n";
        let path = r.path().join("latin.txt");
        std::fs::write(&path, bytes).unwrap();
        commit_all(r.path(), "legacy");
        let sha = git(r.path(), &["rev-parse", "HEAD"]).trim().to_string();
        let mut s = session(r.path());
        s.dispatch(Command::Open(path.clone()));
        settle(&mut s, opened);
        assert_eq!(active_view(&s).snapshot().source_meta().original_encoding(), "windows-1252");
        annotate(&mut s);
        let tab = s.state().active().unwrap().id();
        for line in 0..3 { assert_eq!(s.annotation(tab, line).unwrap().commit.sha, sha, "unchanged encoded line {line}"); }
        let snapshot = active_view(&s).snapshot();
        let expected = snapshot.key();
        let range = snapshot.line_range(2).unwrap();
        s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range, text: "changed".into() }], kind: crate::text::EditKind::Replace });
        settle(&mut s, blamed);
        for line in [0, 1] { assert_eq!(s.annotation(tab, line).unwrap().commit.sha, sha); }
        assert_eq!(s.annotation(tab, 2).unwrap().label, "Not committed");
        let snapshot = active_view(&s).snapshot();
        let expected = snapshot.key();
        let range = snapshot.line_range(2).unwrap();
        s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range, text: "👩".into() }], kind: crate::text::EditKind::Replace });
        settle(&mut s, blamed);
        assert!(matches!(s.state().active().unwrap().blame_status(), Some(crate::blame::Status::Failed(error)) if error.contains("windows-1252 can't represent")));
        assert!(s.annotation(tab, 0).is_none());
        assert_eq!(s.disabled(CommandId::CopyRevision), Some("Blame failed"));
        s.dispatch(Command::Undo { tab });
        settle(&mut s, blamed);
        for line in [0, 1] { assert_eq!(s.annotation(tab, line).unwrap().commit.sha, sha); }
        assert_eq!(s.annotation(tab, 2).unwrap().label, "Not committed");
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn external_edit_reruns_and_marks_the_line_not_committed() {
        let r = repo();
        put(r.path(), "a.txt", "one\ntwo\n");
        commit_all(r.path(), "first");
        let mut s = session(r.path());
        s.dispatch(Command::Open(r.path().join("a.txt")));
        settle(&mut s, opened);
        annotate(&mut s);
        std::fs::write(r.path().join("a.txt"), "one\nTWO\n").unwrap();
        settle(&mut s, |s| blamed(s) && labels(s)[1] == "Not committed");
        let t = s.state().active().unwrap().id();
        assert_eq!(s.annotation(t, 1).unwrap().shade, None);
        assert!(s.annotation(t, 0).unwrap().shade.is_some());
    }

    #[test]
    fn untracked_draft_blames_every_current_line_without_rereading_saved_bytes() {
        let r = repo();
        put(r.path(), "tracked.txt", "head\n");
        commit_all(r.path(), "head");
        put(r.path(), "new.txt", "saved\n");
        let mut s = session(r.path());
        s.dispatch(Command::Open(r.path().join("new.txt")));
        settle(&mut s, opened);
        annotate(&mut s);
        let tab = s.state().active().unwrap().id();
        let expected = active_view(&s).snapshot().key();
        s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..0, text: "é\n👩🏽‍💻\n".into() }], kind: crate::text::EditKind::Paste });
        settle(&mut s, blamed);
        assert_eq!(labels(&s)[..3], ["Not committed", "Not committed", "Not committed"]);
        assert_eq!(std::fs::read_to_string(r.path().join("new.txt")).unwrap(), "saved\n");
    }

    #[test]
    fn draft_insert_delete_and_undo_invalidate_blame_before_recomputing_current_bytes() {
        use crate::text::{EditKind, TextEdit};
        let r = repo();
        let original = "one\ntwo\nthree\n";
        put(r.path(), "a.txt", original);
        commit_all(r.path(), "original");
        let sha = git(r.path(), &["rev-parse", "HEAD"]).trim().to_string();
        let mut s = session(r.path());
        s.dispatch(Command::Open(r.path().join("a.txt")));
        settle(&mut s, opened);
        annotate(&mut s);
        let tab = s.state().active().unwrap().id();
        s.dispatch(Command::Select(Selection { anchor: 4, head: 4 }));
        assert_eq!(s.disabled(CommandId::CopyRevision), None);
        let edit = |s: &mut Session, range, text: &str| {
            let expected = s.state().view(tab).unwrap().snapshot().key();
            s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range, text: text.into() }], kind: EditKind::Replace });
            assert!(s.annotation(tab, 0).is_none(), "old attribution disappears synchronously");
            assert!(s.disabled(CommandId::CopyRevision).is_some(), "no old commit action while pending");
            assert_eq!(s.status_message(), "Blame pending…");
            settle(s, blamed);
        };
        edit(&mut s, 4..4, "new\n");
        assert_eq!(source_text(active_view(&s).snapshot()), "one\nnew\ntwo\nthree\n");
        assert_eq!(s.annotation(tab, 1).unwrap().label, "Not committed");
        for line in [0, 2, 3] { assert_eq!(s.annotation(tab, line).unwrap().commit.sha, sha); }
        edit(&mut s, 8..12, "");
        assert_eq!(source_text(active_view(&s).snapshot()), "one\nnew\nthree\n");
        assert_eq!(s.annotation(tab, 2).unwrap().commit.sha, sha);
        s.dispatch(Command::Undo { tab });
        assert!(s.annotation(tab, 0).is_none());
        settle(&mut s, blamed);
        assert_eq!(source_text(active_view(&s).snapshot()), "one\nnew\ntwo\nthree\n");
        assert_eq!(s.annotation(tab, 2).unwrap().commit.sha, sha);
        s.dispatch(Command::Undo { tab });
        assert!(s.annotation(tab, 0).is_none());
        settle(&mut s, blamed);
        assert_eq!(source_text(active_view(&s).snapshot()), original);
        for line in 0..3 { assert_eq!(s.annotation(tab, line).unwrap().commit.sha, sha); }
        assert_eq!(std::fs::read_to_string(r.path().join("a.txt")).unwrap(), original);
    }

    #[test]
    fn switching_tabs_keeps_annotations_without_a_new_job() {
        let r = repo();
        put(r.path(), "a.txt", "a\n");
        put(r.path(), "b.txt", "b\n");
        commit_all(r.path(), "both");
        let mut s = session(r.path());
        s.dispatch(Command::OpenRetained(r.path().join("a.txt")));
        settle(&mut s, opened);
        annotate(&mut s);
        let before = request(&s);
        s.dispatch(Command::OpenRetained(r.path().join("b.txt")));
        settle(&mut s, |s| opened(s) && s.state().current_source().is_some_and(|d| d.source().ends_with("b.txt")));
        assert_eq!(s.on(CommandId::AnnotateBlame), Some(false), "per tab");
        let a = s.state().tabs().iter().find(|t| t.source().ends_with("a.txt")).unwrap().id();
        s.dispatch(Command::ActivateTab(a));
        settle(&mut s, |s| opened(s) && s.state().current_source().is_some_and(|d| d.source().ends_with("a.txt")));
        assert_eq!((request(&s), blamed(&s)), (before, true), "kept, not blamed again");
        assert!(labels(&s)[0].ends_with(" t"));
    }

    #[test]
    fn rename_rebind_invalidates_attribution_even_when_draft_key_and_disk_stamp_are_unchanged() {
        let r = repo();
        put(r.path(), "a.txt", "one\ntwo\n");
        commit_all(r.path(), "original");
        let mut s = session(r.path());
        s.dispatch(Command::Open(r.path().join("a.txt")));
        settle(&mut s, opened);
        annotate(&mut s);
        let tab = s.state().active().unwrap().id();
        s.dispatch(Command::ShowCommitAt { tab, line: 0 });
        assert!(s.state().commit_popup().is_some());
        let old = request(&s);
        let key = active_view(&s).snapshot().key();
        let token = s.state().active().unwrap().refresh;
        let renamed = r.path().join("renamed.txt");
        std::fs::rename(r.path().join("a.txt"), &renamed).unwrap();
        s.dispatch(Completion::Moved { tab, token, to: renamed.clone(), stamp: Stamp::of(&renamed), canonical: std::fs::canonicalize(&renamed).ok() });
        assert_eq!(active_view(&s).snapshot().key(), key);
        assert!(s.annotation(tab, 0).is_none(), "new path must not retain old attribution/actions");
        assert!(s.state().commit_popup().is_none());
        assert!(request(&s) > old);
        settle(&mut s, blamed);
        assert_eq!(labels(&s)[..2], ["Not committed", "Not committed"]);
        assert_eq!(std::fs::read_to_string(renamed).unwrap(), "one\ntwo\n");
    }

    #[test]
    fn a_stale_run_is_dropped_and_only_the_latest_applies() {
        let r = repo();
        put(r.path(), "a.txt", "a\nb\nc\n");
        commit_all(r.path(), "first");
        let mut s = session(r.path());
        s.dispatch(Command::Open(r.path().join("a.txt")));
        settle(&mut s, opened);
        s.dispatch(Command::SetAnnotate(true));
        let first = request(&s);
        s.dispatch(Command::SetAnnotate(false));
        s.dispatch(Command::SetAnnotate(true));
        s.dispatch(Command::Reload);
        settle(&mut s, |s| opened(s) && blamed(s));
        assert!(request(&s) > first);
        assert!(labels(&s)[..3].iter().all(|l| l.ends_with(" t")), "{:?}", labels(&s));
    }

    #[test]
    fn stale_batches_and_errors_cannot_revive_commit_actions_after_edit_or_failure() {
        let (r, mut s, _) = annotated_repo();
        let tab = s.state().active().unwrap().id();
        let old_request = request(&s);
        let old_commit = s.annotation(tab, 0).unwrap().commit.clone();
        s.dispatch(Command::ShowCommitAt { tab, line: 0 });
        assert!(s.state().commit_popup().is_some());
        let expected = active_view(&s).snapshot().key();
        s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..0, text: "new\n".into() }], kind: crate::text::EditKind::Replace });
        let current = request(&s);
        assert!(current > old_request);
        assert!(s.state().commit_popup().is_none());
        for done in [Some(Ok(())), Some(Err("stale failure".into()))] {
            s.dispatch(Completion::Blamed { tab, request: old_request, root: Some(r.path().into()), hunks: vec![(0..2, old_commit.clone())], done });
            assert!(s.annotation(tab, 0).is_none());
            assert_eq!(s.disabled(CommandId::CopyRevision), Some("Blame pending"));
        }
        s.dispatch(Completion::Blamed { tab, request: current, root: None, hunks: vec![], done: Some(Err("current failure".into())) });
        assert_eq!(s.state().active().unwrap().blame_status(), Some(crate::blame::Status::Failed("current failure")));
        s.dispatch(Completion::Blamed { tab, request: current, root: Some(r.path().into()), hunks: vec![(0..2, old_commit)], done: Some(Ok(())) });
        assert!(s.annotation(tab, 0).is_none());
        for id in [CommandId::ShowCommit, CommandId::ShowCommitDiff, CommandId::CopyRevision] { assert_eq!(s.disabled(id), Some("Blame failed")); }
        s.dispatch(Command::ShowCommitAt { tab, line: 0 });
        assert!(s.state().commit_popup().is_none());
        assert_eq!(std::fs::read_to_string(r.path().join("a.txt")).unwrap(), "one\nTWO\n");
    }

    #[test]
    fn reload_reruns_after_a_commit_with_unchanged_bytes() {
        let r = repo();
        put(r.path(), "a.txt", "one\n");
        commit_all(r.path(), "first");
        put(r.path(), "a.txt", "one\ntwo\n");
        let mut s = session(r.path());
        s.dispatch(Command::Open(r.path().join("a.txt")));
        settle(&mut s, opened);
        annotate(&mut s);
        assert_eq!(labels(&s)[1], "Not committed");
        git(r.path(), &["commit", "-qam", "second"]);
        s.dispatch(Command::Reload);
        settle(&mut s, |s| opened(s) && blamed(s) && labels(s)[1] != "Not committed");
        let t = s.state().active().unwrap().id();
        assert_eq!(s.annotation(t, 1).unwrap().commit.summary, "second");
    }

    #[test]
    fn outside_git_keeps_a_visible_failure_without_commit_actions() {
        let d = tempfile::tempdir().unwrap();
        let path = write(d.path(), "a.txt", b"x\n");
        let mut s = session(d.path());
        s.dispatch(Command::Open(path));
        settle(&mut s, opened);
        s.dispatch(Command::SetAnnotate(true));
        settle(&mut s, |s| s.state().active().is_some_and(|t| matches!(t.blame_status(), Some(crate::blame::Status::Failed(_)))));
        assert_eq!(s.status_message(), "Annotate with Git Blame: Not in a Git repository");
        assert_eq!(s.on(CommandId::AnnotateBlame), Some(true));
        assert_eq!(s.disabled(CommandId::ShowCommit), Some("Blame failed"));
        assert!(s.caret_annotation().is_none());
    }

    #[test]
    fn disabled_reasons() {
        let d = tempfile::tempdir().unwrap();
        let mut s = session(d.path());
        assert_eq!(s.disabled(CommandId::AnnotateBlame), Some("No file is open"));
        assert_eq!(s.disabled(CommandId::CloseAnnotations), Some("No file is open"));
        let png = d.path().join("i.png");
        image::RgbaImage::new(2, 2).save(&png).unwrap();
        s.dispatch(Command::Open(png));
        settle(&mut s, opened);
        assert_eq!(s.disabled(CommandId::AnnotateBlame), Some("Only text can be annotated"));
        s.dispatch(Command::Open(write(d.path(), "a.txt", b"x\n")));
        settle(&mut s, |s| opened(s) && s.state().current_source().is_some_and(|d| d.image().is_none()));
        assert_eq!(s.disabled(CommandId::AnnotateBlame), None);
        assert_eq!(s.disabled(CommandId::CloseAnnotations), Some("Annotations are off"));
    }

    #[test]
    fn split_copy_keeps_its_own_toggle() {
        let r = repo();
        put(r.path(), "a.txt", "a\n");
        commit_all(r.path(), "first");
        let mut s = session(r.path());
        s.dispatch(Command::Open(r.path().join("a.txt")));
        settle(&mut s, opened);
        annotate(&mut s);
        s.dispatch(Command::Split(Split::Columns));
        settle(&mut s, blamed);
        assert!(labels(&s)[0].ends_with(" t"), "the copy is annotated too");
        s.dispatch(Command::SetAnnotate(false));
        assert!(s.state().shown(0).unwrap().blame.is_some(), "the other pane keeps its annotations");
    }

    fn annotated_repo() -> (tempfile::TempDir, Session, String) {
        let r = repo();
        put(r.path(), "a.txt", "one\ntwo\n");
        commit_all(r.path(), "First line\n\nThe body.");
        put(r.path(), "a.txt", "one\nTWO\n");
        let mut s = session(r.path());
        s.dispatch(Command::Open(r.path().join("a.txt")));
        settle(&mut s, opened);
        annotate(&mut s);
        let sha = git(r.path(), &["rev-parse", "HEAD"]).trim().to_string();
        (r, s, sha)
    }

    #[test]
    fn show_commit_loads_details_and_the_popup_text() {
        let (_r, mut s, sha) = annotated_repo();
        let t = s.state().active().unwrap().id();
        s.dispatch(Command::Select(Selection { anchor: 6, head: 6 }));
        s.dispatch(Command::ShowCommitAt { tab: t, line: 0 });
        assert_eq!(s.state().selection().unwrap().range(), 0..0, "the caret goes to the clicked line");
        assert!(s.commit_text().unwrap().starts_with(&format!("Loading commit {}…", &sha[..7])));
        settle(&mut s, |s| s.state().commit_popup().is_some_and(|p| p.details.is_some()));
        let text = s.commit_text().unwrap();
        assert!(text.starts_with(&format!("commit {sha}\nAuthor: t <t@t>\nDate:   ")), "{text}");
        assert!(text.contains("\n    First line\n\n    The body.\n"), "{text}");
        assert!(text.ends_with("\nA a.txt"), "{text}");
        s.dispatch(Command::CloseCommit);
        assert!(s.state().commit_popup().is_none());
    }

    #[test]
    fn copy_revision_copies_the_full_sha() {
        let (_r, mut s, sha) = annotated_repo();
        s.dispatch(Command::Select(Selection { anchor: 0, head: 0 }));
        assert_eq!(s.disabled(CommandId::CopyRevision), None);
        let effect = s.dispatch(Command::CopyRevision);
        assert_eq!(effect.copy.as_deref(), Some(sha.as_str()));
        assert_eq!(s.status_message(), format!("Copied {}", &sha[..7]));
    }

    #[test]
    fn show_diff_launches_lenz_diff_on_the_commit() {
        let (r, mut s, sha) = annotated_repo();
        s.dispatch(Command::Select(Selection { anchor: 0, head: 0 }));
        let bin = tempfile::tempdir().unwrap();
        let exe = bin.path().join("lenz-code");
        let missing = s.commit_diff_launch(&exe, false).unwrap_err();
        assert!(missing.starts_with("Show Diff: Lenz Diff is not beside Lenz Code ("), "{missing}");
        assert!(missing.ends_with("); reinstall Lenz"), "{missing}");
        std::fs::write(bin.path().join("lenz-diff"), "").unwrap();
        std::fs::write(bin.path().join("lenz-diff-tui"), "").unwrap();
        let launch = s.commit_diff_launch(&exe, false).unwrap();
        assert_eq!(launch.program, bin.path().join("lenz-diff").to_string_lossy());
        // The root commit is compared with the empty tree.
        let empty = git(r.path(), &["hash-object", "-t", "tree", "/dev/null"]).trim().to_string();
        assert_eq!(launch.args, [empty, sha.clone()]);
        assert_eq!(launch.cwd.as_deref(), Some(std::fs::canonicalize(r.path()).unwrap().as_path()));
        assert!(s.commit_diff_launch(&exe, true).unwrap().program.ends_with("lenz-diff-tui"));
        // A later commit is compared with its first parent.
        commit_all(r.path(), "second");
        s.dispatch(Command::Reload);
        settle(&mut s, |s| opened(s) && blamed(s) && labels(s)[1] != "Not committed");
        s.dispatch(Command::Select(Selection { anchor: 4, head: 4 }));
        let second = git(r.path(), &["rev-parse", "HEAD"]).trim().to_string();
        assert_eq!(s.commit_diff_launch(&exe, false).unwrap().args, [format!("{second}^1"), second]);
    }

    #[test]
    fn uncommitted_and_unblamed_lines_disable_the_commit_commands() {
        let (_r, mut s, _) = annotated_repo();
        s.dispatch(Command::Select(Selection { anchor: 4, head: 4 }));
        for id in [CommandId::ShowCommit, CommandId::ShowCommitDiff, CommandId::CopyRevision] {
            assert_eq!(s.disabled(id), Some("This line is not committed"));
        }
        s.dispatch(Command::SetAnnotate(false));
        assert_eq!(s.disabled(CommandId::ShowCommit), Some("Annotations are off"));
    }

    #[test]
    fn status_line_shows_the_caret_commit() {
        let (_r, mut s, sha) = annotated_repo();
        s.dispatch(Command::Select(Selection { anchor: 0, head: 0 }));
        assert_eq!(s.status_message(), format!("{} · t · {} · First line", &sha[..7], today()));
        s.dispatch(Command::Select(Selection { anchor: 4, head: 4 }));
        assert_eq!(s.status_message(), "Not committed");
        s.dispatch(Command::Notice("Something happened".into()));
        assert_eq!(s.status_message(), "Something happened", "notices come first");
    }

    #[test]
    fn context_menu_offers_the_blame_entries() {
        let (_r, mut s, _) = annotated_repo();
        let has = |s: &Session, id| s.context_menu().contains(&MenuEntry::Command(id));
        assert!([CommandId::ShowCommit, CommandId::ShowCommitDiff, CommandId::CopyRevision, CommandId::CloseAnnotations].iter().all(|&id| has(&s, id)));
        assert!(!has(&s, CommandId::AnnotateBlame));
        s.dispatch(Command::SetAnnotate(false));
        assert!(has(&s, CommandId::AnnotateBlame));
        assert!(!has(&s, CommandId::ShowCommit));
    }

    #[test]
    fn closing_the_tab_closes_the_popup() {
        let (_r, mut s, _) = annotated_repo();
        let t = s.state().active().unwrap().id();
        s.dispatch(Command::ShowCommitAt { tab: t, line: 0 });
        assert!(s.state().commit_popup().is_some());
        s.dispatch(Command::CloseTab(t));
        assert!(s.state().commit_popup().is_none());
    }

    #[test]
    fn loading_and_svg_source_reasons() {
        let r = repo();
        put(r.path(), "a.txt", "one\n");
        put(r.path(), "i.svg", "<svg xmlns='http://www.w3.org/2000/svg' width='2' height='2'/>\n");
        commit_all(r.path(), "first");
        let mut s = session(r.path());
        s.dispatch(Command::Open(r.path().join("a.txt")));
        assert_eq!(s.disabled(CommandId::AnnotateBlame), Some("The file is still loading"));
        settle(&mut s, opened);
        assert_eq!(s.disabled(CommandId::AnnotateBlame), None);
        s.dispatch(Command::Open(r.path().join("i.svg")));
        settle(&mut s, |s| opened(s) && s.state().current_source().is_some_and(|d| d.image().is_some()));
        assert_eq!(s.disabled(CommandId::AnnotateBlame), Some("Only text can be annotated"), "the rendered image");
        s.dispatch(Command::ToggleImageSource);
        assert!(s.state().active().unwrap().image_source());
        assert_eq!(s.disabled(CommandId::AnnotateBlame), None, "an SVG shown as its source");
        annotate(&mut s);
        assert_eq!(labels(&s)[0], format!("{} t", today()));
    }

    #[test]
    fn annotations_turn_off_after_the_file_goes() {
        let (r, mut s, _) = annotated_repo();
        std::fs::remove_file(r.path().join("a.txt")).unwrap();
        settle(&mut s, |s| s.state().active().is_some_and(|t| t.unavailable.is_some()));
        assert_eq!(s.disabled(CommandId::CloseAnnotations), None, "turning off is always possible");
        assert_eq!(s.disabled(CommandId::AnnotateBlame), None);
        s.dispatch(Command::SetAnnotate(false));
        assert!(s.state().active().unwrap().blame.is_none());
        assert_eq!(s.disabled(CommandId::AnnotateBlame), Some("The file is unavailable"), "and not back on");
    }

    #[test]
    #[ignore = "timing: cargo test -p reader-core --release --lib blame_timing -- --ignored --nocapture"]
    fn blame_timing_on_a_large_file_with_a_long_history() {
        let r = repo();
        let mut lines: Vec<String> = (0..50_000).map(|i| format!("    let value_{i} = compute({i});")).collect();
        let mut seed = 7u64;
        let built = std::time::Instant::now();
        for n in 0..200 {
            for _ in 0..50 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let i = (seed >> 33) as usize % lines.len();
                lines[i] = format!("    let value_{i} = edited({n});");
            }
            put(r.path(), "big.rs", lines.join("\n") + "\n");
            commit_all(r.path(), &format!("commit {n}"));
        }
        eprintln!("built 200 commits of 50,000 lines in {:?}", built.elapsed());
        let mut s = session(r.path());
        s.dispatch(Command::Open(r.path().join("big.rs")));
        settle(&mut s, opened);
        for round in 0..3 {
            s.dispatch(Command::SetAnnotate(false));
            let started = std::time::Instant::now();
            s.dispatch(Command::SetAnnotate(true));
            settle(&mut s, |s| { let t = s.state().active().unwrap(); (0..50_000).step_by(997).any(|l| t.annotation(l).is_some()) });
            let first = started.elapsed();
            settle(&mut s, blamed);
            let done = started.elapsed();
            let t = s.state().active().unwrap();
            let b = t.blame.as_ref().unwrap();
            let commits: std::collections::HashSet<_> = b.lines.iter().flatten().map(Arc::as_ptr).collect();
            let bytes = b.lines.capacity() * std::mem::size_of::<Option<Arc<lenz_content::blame::Commit>>>();
            // A frame's worth of labels (60 rows), as the adapters ask for them.
            let frame = std::time::Instant::now();
            for _ in 0..100 {
                for l in 25_000..25_060 {
                    std::hint::black_box(s.annotation(t.id(), l).map(|a| a.label));
                }
            }
            let per_frame = frame.elapsed() / 100;
            eprintln!("round {round}: first annotation {first:?}, done {done:?}, line table {} KiB, {} commits, 60 labels {per_frame:?}", bytes / 1024, commits.len());
        }
    }
}

mod git_branches {
    use super::*;
    use crate::branches::{BranchAction, BranchTarget, GitDiff};
    use lenz_content::git::testing::{commit_all, git, repo, write as put};

    /// A repository on `main` with `a.txt`, branches `other` and `feature/x`, tag `v1`, a remote branch and a second
    /// worktree; a session with it open as its folder.
    pub(super) fn repo_session() -> (tempfile::TempDir, tempfile::TempDir, Session) {
        let r = repo();
        git(r.path(), &["config", "user.name", "t"]);
        git(r.path(), &["config", "user.email", "t@t"]);
        put(r.path(), "a.txt", "main\n");
        commit_all(r.path(), "main");
        for b in ["other", "feature/x"] {
            git(r.path(), &["branch", b]);
        }
        git(r.path(), &["tag", "v1"]);
        git(r.path(), &["remote", "add", "origin", &r.path().to_string_lossy()]);
        git(r.path(), &["update-ref", "refs/remotes/origin/topic", "HEAD"]);
        let wt = tempfile::tempdir().unwrap();
        git(r.path(), &["worktree", "add", "-q", &wt.path().join("second").to_string_lossy(), "other"]);
        let mut s = session(&r.path().join(".lenz-test"));
        s.dispatch(Command::OpenRoot(r.path().to_path_buf()));
        settle(&mut s, |s| s.branch_label().is_some());
        (r, wt, s)
    }
    fn tick(s: &mut Session) {
        s.dispatch(Input::Completion(Completion::Semantic(crate::semantics::Event::Tick)));
    }
    fn open_branches(s: &mut Session) {
        s.dispatch(Command::OpenBranches);
        settle(s, |s| s.state().branches().is_some_and(|b| !b.loading));
    }

    #[test]
    fn the_branch_shows_and_follows_a_switch_made_elsewhere() {
        let (r, _wt, mut s) = repo_session();
        assert_eq!(s.branch_label().as_deref(), Some("⎇ main"));
        git(r.path(), &["switch", "-q", "feature/x"]);
        tick(&mut s);
        settle(&mut s, |s| s.branch_label().as_deref() == Some("⎇ feature/x"));
        let sha = git(r.path(), &["rev-parse", "HEAD"]).trim().to_string();
        git(r.path(), &["switch", "-q", "--detach", "HEAD"]);
        tick(&mut s);
        settle(&mut s, |s| s.branch_label() == Some(format!("⎇ {}", &sha[..7])));
    }

    #[test]
    fn outside_git_there_is_no_branch_and_the_git_commands_say_why() {
        let d = tempfile::tempdir().unwrap();
        let mut s = session(d.path());
        s.dispatch(Command::Open(write(d.path(), "a.txt", b"x\n")));
        settle(&mut s, opened);
        settle(&mut s, |s| s.state().repo_checked());
        assert_eq!(s.branch_label(), None);
        for id in [CommandId::GitBranches, CommandId::NewBranch, CommandId::CheckoutRevision] {
            assert_eq!(s.disabled(id), Some("Not in a Git repository"), "{id:?}");
            assert_eq!(id.group(), crate::actions::CommandGroup::Git);
        }
        // An unborn branch: the branch to be.
        let r = repo();
        let mut s = session(&r.path().join(".lenz-test"));
        s.dispatch(Command::OpenRoot(r.path().to_path_buf()));
        settle(&mut s, |s| s.branch_label().is_some());
        assert_eq!(s.branch_label().as_deref(), Some("⎇ main"));
        assert_eq!(s.disabled(CommandId::GitBranches), None);
    }

    #[test]
    fn a_folder_that_becomes_a_repository_shows_its_branch() {
        let d = tempfile::tempdir().unwrap();
        let mut s = session(&d.path().join(".lenz-test"));
        s.dispatch(Command::Open(write(d.path(), "a.txt", b"x\n")));
        settle(&mut s, |s| opened(s) && s.state().repo_checked());
        assert_eq!(s.branch_label(), None);
        git(d.path(), &["init", "-q", "-b", "main"]);
        // Opening the folder looks again …
        s.dispatch(Command::OpenRoot(d.path().to_path_buf()));
        settle(&mut s, |s| s.branch_label().is_some());
        assert_eq!(s.branch_label().as_deref(), Some("⎇ main"));
        // … and so does the tick once a `.git` appears where there was none.
        let e = tempfile::tempdir().unwrap();
        let mut s = session(&e.path().join(".lenz-test"));
        s.dispatch(Command::OpenRoot(e.path().to_path_buf()));
        settle(&mut s, |s| s.state().repo_checked());
        assert_eq!(s.branch_label(), None);
        git(e.path(), &["init", "-q", "-b", "trunk"]);
        tick(&mut s);
        settle(&mut s, |s| s.branch_label().is_some());
        assert_eq!(s.branch_label().as_deref(), Some("⎇ trunk"));
    }

    #[test]
    fn a_worktree_whose_folder_is_gone_is_marked_and_offers_nothing() {
        let (_r, wt, mut s) = repo_session();
        std::fs::remove_dir_all(wt.path().join("second")).unwrap();
        open_branches(&mut s);
        let row = s.state().branch_rows("second").into_iter().next().unwrap();
        assert!(row.label.ends_with(" · missing"), "{}", row.label);
        assert!(s.branch_actions(&row.target).is_empty());
    }

    #[test]
    fn the_popup_lists_sections_marks_the_current_one_and_filters() {
        let (r, wt, mut s) = repo_session();
        git(r.path(), &["switch", "-q", "feature/x"]);
        git(r.path(), &["switch", "-q", "main"]);
        open_branches(&mut s);
        let rows = s.state().branch_rows("");
        let shown: Vec<(&str, &str, bool)> = rows.iter().map(|r| (r.section, r.label.as_str(), r.current)).collect();
        assert_eq!(&shown[..2], [("", "New Branch…", false), ("", "Checkout Tag or Revision…", false)]);
        assert!(shown.contains(&("Recent", "feature/x", false)));
        for (section, label) in [("Local", "feature/x"), ("Local", "other"), ("Remote", "origin/topic"), ("Tags", "v1")] {
            assert!(shown.contains(&(section, label, false)), "{section} {label}: {shown:?}");
        }
        assert!(shown.contains(&("Local", "main", true)), "the current branch is marked");
        let worktrees: Vec<_> = rows.iter().filter(|r| r.section == "Worktrees").collect();
        assert_eq!(worktrees.len(), 2);
        assert!(worktrees.iter().any(|w| w.current && w.label.starts_with("main ")));
        assert!(worktrees.iter().any(|w| matches!(&w.target, BranchTarget::Worktree(p) if p.ends_with("second")) && w.label.starts_with("other ")));
        let _ = wt;
        // Filtering keeps only matching rows (any case), sections and all.
        let found: Vec<_> = s.state().branch_rows("TOP").into_iter().map(|r| r.label).collect();
        assert_eq!(found, ["origin/topic"]);
        assert!(s.state().branch_rows("nothing like it").is_empty());
        s.dispatch(Command::CloseBranches);
        assert!(s.state().branches().is_none());
    }

    #[test]
    fn each_row_offers_its_actions() {
        let (_r, _wt, mut s) = repo_session();
        open_branches(&mut s);
        let rows = s.state().branch_rows("");
        let actions = |label: &str| {
            let row = rows.iter().find(|r| r.label == label || r.label.starts_with(&format!("{label} "))).unwrap();
            s.branch_actions(&row.target).into_iter().map(|(l, _)| l).collect::<Vec<_>>()
        };
        assert_eq!(actions("other"), ["Checkout", "New Branch from Here…", "Compare with Working Tree", "Show Changes Since Branching from Here"]);
        assert_eq!(actions("main"), ["New Branch from Here…", "Compare with Working Tree"], "the current branch is not checked out again");
        let enter = |label: &str| s.branch_enter(&rows.iter().find(|r| r.label == label || r.label.starts_with(&format!("{label} "))).unwrap().target);
        assert_eq!((enter("main"), enter("other"), enter("v1")), (None, Some(BranchAction::Checkout), Some(BranchAction::Checkout)), "Enter never opens a prompt");
        assert_eq!(actions("origin/topic"), ["Checkout", "New Branch from Here…", "Compare with Working Tree", "Show Changes Since Branching from Here"]);
        assert_eq!(actions("v1"), ["Checkout", "New Branch from Here…", "Compare with Working Tree"]);
        let second = rows.iter().find(|r| matches!(&r.target, BranchTarget::Worktree(p) if p.ends_with("second"))).unwrap();
        assert_eq!(
            s.branch_actions(&second.target).into_iter().collect::<Vec<_>>(),
            [("Open", BranchAction::Open), ("Open in New Window", BranchAction::OpenInNewWindow), ("Compare with Working Tree", BranchAction::CompareWithWorkingTree)]
        );
        let here = rows.iter().find(|r| r.section == "Worktrees" && r.current).unwrap();
        assert_eq!(s.branch_actions(&here.target).into_iter().map(|(l, _)| l).collect::<Vec<_>>(), ["Open in New Window"]);
        assert_eq!(s.branch_enter(&here.target), None, "Enter on the current worktree opens nothing");
    }

    /// A Lenz Diff beside a Lenz Code in a throwaway folder.
    fn exe() -> (tempfile::TempDir, PathBuf) {
        let bin = tempfile::tempdir().unwrap();
        for name in ["lenz-diff", "lenz-diff-tui"] {
            std::fs::write(bin.path().join(name), "").unwrap();
        }
        let exe = bin.path().join("lenz-code");
        (bin, exe)
    }
    fn args(s: &Session, diff: &GitDiff, exe: &Path) -> Result<Vec<String>, String> {
        s.git_diff_launch(diff, exe, false).map(|l| l.args)
    }

    #[test]
    fn local_changes_and_a_revision_open_lenz_diff_on_the_working_tree() {
        let (r, _wt, s) = repo_session();
        let (_bin, exe) = exe();
        let launch = s.git_diff_launch(&GitDiff::LocalChanges, &exe, true).unwrap();
        assert_eq!((launch.program.ends_with("lenz-diff-tui"), launch.args, launch.cwd), (true, vec!["HEAD".to_string()], Some(std::fs::canonicalize(r.path()).unwrap())));
        assert_eq!(args(&s, &GitDiff::Revision(" v1 ".into()), &exe), Ok(vec!["v1".to_string()]));
        assert_eq!(args(&s, &GitDiff::Revision("nope".into()), &exe), Err("Compare: No commit named \"nope\"".into()));
        // A branch named like a Lenz Diff subcommand goes by its commit.
        git(r.path(), &["branch", "log"]);
        let sha = git(r.path(), &["rev-parse", "HEAD"]).trim().to_string();
        assert_eq!(args(&s, &GitDiff::Revision("log".into()), &exe), Ok(vec![sha]));
        let none = tempfile::tempdir().unwrap();
        assert!(s.git_diff_launch(&GitDiff::LocalChanges, &none.path().join("lenz-code"), false).unwrap_err().contains("Lenz Diff is not beside Lenz Code"));
        // No commit yet: nothing to compare with.
        let fresh = repo();
        let mut s = session(&fresh.path().join(".lenz-test"));
        s.dispatch(Command::OpenRoot(fresh.path().to_path_buf()));
        settle(&mut s, |s| s.branch_label().is_some());
        // No commit yet: every file is new.
        assert_eq!(args(&s, &GitDiff::LocalChanges, &exe), Ok(vec!["4b825dc642cb6eb9a060e54bf8d69288fbee4904".to_string()]));
        assert_eq!(args(&s, &GitDiff::BranchChanges(None), &exe), Err("Show Changes Since Branch Start: there is no commit yet".into()));
        assert_eq!(args(&s, &GitDiff::BaseBranch, &exe), Err("Compare with Base Branch: there is no commit yet".into()));
    }

    #[test]
    fn branch_changes_count_from_where_the_branch_split_from_its_base() {
        let (r, _wt, s) = repo_session();
        let (_bin, exe) = exe();
        let split = git(r.path(), &["rev-parse", "HEAD"]).trim().to_string();
        assert_eq!(args(&s, &GitDiff::BranchChanges(None), &exe), Err("Show Changes Since Branch Start: main is the base branch; Show Local Changes shows its changes".into()));
        git(r.path(), &["switch", "-q", "feature/x"]);
        put(r.path(), "a.txt", "x
");
        commit_all(r.path(), "x");
        assert_eq!(args(&s, &GitDiff::BranchChanges(None), &exe), Ok(vec![split.clone()]));
        assert_eq!(args(&s, &GitDiff::BranchChanges(Some("refs/heads/other".into())), &exe), Ok(vec![split.clone()]));
        git(r.path(), &["switch", "-q", "--detach", "HEAD"]);
        assert_eq!(args(&s, &GitDiff::BranchChanges(None), &exe), Ok(vec![split.clone()]), "a detached HEAD is never the base");
        // On main with origin/main as the base.
        git(r.path(), &["switch", "-q", "main"]);
        git(r.path(), &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        git(r.path(), &["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"]);
        let mut s = session(&r.path().join(".lenz-test"));
        s.dispatch(Command::OpenRoot(r.path().to_path_buf()));
        settle(&mut s, |s| s.branch_label().is_some());
        assert!(args(&s, &GitDiff::BranchChanges(None), &exe).unwrap_err().contains("main is the base branch"));
        // No base at all.
        let lone = repo();
        put(lone.path(), "a.txt", "a
");
        commit_all(lone.path(), "1");
        git(lone.path(), &["branch", "-q", "-m", "main", "dev"]);
        let mut s = session(&lone.path().join(".lenz-test"));
        s.dispatch(Command::OpenRoot(lone.path().to_path_buf()));
        settle(&mut s, |s| s.branch_label().is_some());
        assert_eq!(args(&s, &GitDiff::BranchChanges(None), &exe), Err("Show Changes Since Branch Start: no main or master branch to compare with".into()));
    }

    #[test]
    fn rows_compare_and_worktrees_compare_folder_with_folder() {
        let (r, wt, mut s) = repo_session();
        let (_bin, exe) = exe();
        open_branches(&mut s);
        let rows = s.state().branch_rows("");
        let target = |label: &str| rows.iter().find(|r| r.label == label).unwrap().target.clone();
        let diff = |t: &BranchTarget, a| GitDiff::of(t, a);
        assert_eq!(diff(&target("v1"), BranchAction::CompareWithWorkingTree), Some(GitDiff::Revision("refs/tags/v1".into())));
        assert_eq!(diff(&target("other"), BranchAction::ChangesSinceBranching), Some(GitDiff::BranchChanges(Some("refs/heads/other".into()))));
        assert_eq!(diff(&target("origin/topic"), BranchAction::CompareWithWorkingTree), Some(GitDiff::Revision("refs/remotes/origin/topic".into())));
        assert_eq!(diff(&target("other"), BranchAction::Checkout), None);
        let second = wt.path().join("second");
        let launch = s.git_diff_launch(&GitDiff::Worktree(second.clone()), &exe, false).unwrap();
        let root = std::fs::canonicalize(r.path()).unwrap();
        assert_eq!(launch.args, ["--no-index".to_string(), root.to_string_lossy().into_owned(), second.to_string_lossy().into_owned()]);
        assert_eq!(launch.cwd, Some(root));
        assert!(s.git_diff_launch(&GitDiff::Worktree(wt.path().join("gone")), &exe, false).is_err());
    }

    #[test]
    fn a_bare_repository_is_never_a_worktree_to_compare_with() {
        // The `repo.git` beside its worktrees layout.
        let d = tempfile::tempdir().unwrap();
        let seed = repo();
        put(seed.path(), "a.txt", "a\n");
        commit_all(seed.path(), "1");
        git(d.path(), &["clone", "-q", "--bare", &seed.path().to_string_lossy(), "b.git"]);
        let bare = d.path().join("b.git");
        for (name, branch) in [("one", "x"), ("two", "y")] {
            git(&bare, &["worktree", "add", "-q", "-b", branch, &d.path().join(name).to_string_lossy()]);
        }
        let mut s = session(&d.path().join(".lenz-test"));
        s.dispatch(Command::OpenRoot(d.path().join("one")));
        settle(&mut s, |s| s.branch_label().is_some());
        s.dispatch(Command::OpenWorktrees);
        settle(&mut s, |s| s.state().branches().is_some_and(|b| !b.loading));
        let rows: Vec<_> = s.state().branch_rows("").into_iter().map(|r| r.target).collect();
        assert_eq!(rows, [BranchTarget::Worktree(d.path().join("two"))]);
        s.dispatch(Command::CloseBranches);
        open_branches(&mut s);
        let bare_row = s.state().branch_rows("(bare)").into_iter().next().expect("listed in the full popup");
        assert!(s.branch_actions(&bare_row.target).is_empty());
    }

    #[test]
    fn a_name_that_is_also_a_file_goes_to_lenz_diff_as_its_commit() {
        let (r, _wt, s) = repo_session();
        let (_bin, exe) = exe();
        let sha = git(r.path(), &["rev-parse", "HEAD"]).trim().to_string();
        git(r.path(), &["branch", "docs"]);
        put(r.path(), "docs/x.md", "x\n");
        assert_eq!(args(&s, &GitDiff::Revision("docs".into()), &exe), Ok(vec![sha.clone()]));
        put(r.path(), "HEAD", "a file named HEAD\n");
        assert_eq!(args(&s, &GitDiff::LocalChanges, &exe), Ok(vec![sha]));
    }

    #[test]
    fn compare_with_worktree_runs_in_the_repository_and_says_its_name() {
        let (r, wt, mut s) = repo_session();
        let (_bin, exe) = exe();
        // A file open from the repository, no folder: the launch still runs in the repository.
        s.dispatch(Command::OpenWorktree(wt.path().join("second")));
        settle(&mut s, |s| s.branch_label().as_deref() == Some("⎇ other"));
        let launch = s.git_diff_launch(&GitDiff::Worktree(r.path().to_path_buf()), &exe, false).unwrap();
        assert_eq!(launch.cwd, Some(std::fs::canonicalize(wt.path().join("second")).unwrap()));
        let gone = s.git_diff_launch(&GitDiff::Worktree(wt.path().join("gone")), &exe, false).unwrap_err();
        assert!(gone.starts_with("Compare with Worktree: "), "{gone}");
    }

    #[test]
    fn compare_with_worktree_lists_only_the_other_worktrees() {
        let (_r, wt, mut s) = repo_session();
        s.dispatch(Command::OpenWorktrees);
        settle(&mut s, |s| s.state().branches().is_some_and(|b| !b.loading));
        let rows = s.state().branch_rows("");
        assert_eq!(rows.iter().map(|r| r.target.clone()).collect::<Vec<_>>(), [BranchTarget::Worktree(wt.path().join("second"))]);
        assert_eq!(s.branch_enter(&rows[0].target), Some(BranchAction::CompareWithWorkingTree));
        assert!(s.state().branch_rows("nothing like it").is_empty());
        s.dispatch(Command::CloseBranches);
        // With the other one gone there is nothing to compare with.
        std::fs::remove_dir_all(wt.path().join("second")).unwrap();
        s.dispatch(Command::OpenWorktrees);
        settle(&mut s, |s| s.state().branches().is_some_and(|b| !b.loading));
        assert!(s.state().branch_rows("").is_empty());
        assert_eq!(s.state().branches().unwrap().error.as_deref(), Some("No other worktree"));
        // The full popup is back to normal next time.
        s.dispatch(Command::CloseBranches);
        open_branches(&mut s);
        assert!(s.state().branch_rows("").iter().any(|r| r.section == "Local"));
    }

    #[test]
    fn compare_with_base_branch_compares_with_its_tip() {
        let (r, _wt, s) = repo_session();
        let (_bin, exe) = exe();
        assert_eq!(args(&s, &GitDiff::BaseBranch, &exe), Ok(vec!["refs/heads/main".to_string()]), "on main itself: its tip");
        git(r.path(), &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        git(r.path(), &["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"]);
        assert_eq!(args(&s, &GitDiff::BaseBranch, &exe), Ok(vec!["refs/heads/main".to_string()]), "the local main first");
        git(r.path(), &["switch", "-q", "feature/x"]);
        git(r.path(), &["branch", "-q", "-D", "main"]);
        assert_eq!(args(&s, &GitDiff::BaseBranch, &exe), Ok(vec!["refs/remotes/origin/main".to_string()]), "else the remote's default");
        let lone = repo();
        put(lone.path(), "a.txt", "a\n");
        commit_all(lone.path(), "1");
        git(lone.path(), &["branch", "-q", "-m", "main", "dev"]);
        let mut s = session(&lone.path().join(".lenz-test"));
        s.dispatch(Command::OpenRoot(lone.path().to_path_buf()));
        settle(&mut s, |s| s.branch_label().is_some());
        assert_eq!(args(&s, &GitDiff::BaseBranch, &exe), Err("Compare with Base Branch: no main or master branch to compare with".into()));
    }

    #[test]
    fn compare_with_a_revision_picks_from_branches_and_tags_or_takes_typed_text() {
        let (_r, _wt, mut s) = repo_session();
        s.dispatch(Command::OpenRevisions);
        settle(&mut s, |s| s.state().branches().is_some_and(|b| !b.loading));
        assert_eq!(s.state().branches().unwrap().heading(), "Compare with Branch, Tag or Revision");
        let rows = s.state().branch_rows("");
        let sections: std::collections::BTreeSet<_> = rows.iter().map(|r| r.section).collect();
        assert_eq!(sections.into_iter().collect::<Vec<_>>(), ["Local", "Remote", "Tags"], "no commands, Recent or Worktrees");
        let v1 = rows.iter().find(|r| r.label == "v1").unwrap();
        assert_eq!(s.branch_enter(&v1.target), Some(BranchAction::CompareWithWorkingTree));
        assert_eq!(GitDiff::of(&v1.target, BranchAction::CompareWithWorkingTree), Some(GitDiff::Revision("refs/tags/v1".into())));
        // Typed text: a first row offers it as a revision, then the matches.
        let typed = s.state().branch_rows("HEAD~1");
        assert_eq!((typed[0].label.as_str(), &typed[0].target), ("Compare with “HEAD~1”", &BranchTarget::Revision("HEAD~1".into())));
        assert_eq!(s.branch_enter(&typed[0].target), Some(BranchAction::CompareWithWorkingTree));
        assert_eq!(GitDiff::of(&typed[0].target, BranchAction::CompareWithWorkingTree), Some(GitDiff::Revision("HEAD~1".into())));
        let other = s.state().branch_rows("oth");
        assert_eq!(other.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(), ["Compare with “oth”", "other"]);
        let exact = s.state().branch_rows("other");
        assert_eq!(exact.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(), ["other"], "a listed name is picked from the list");
        assert_eq!(s.branch_actions(&exact[0].target).into_iter().map(|(l, _)| l).collect::<Vec<_>>(), ["Compare with Working Tree", "Show Changes Since Branching from Here"], "the pick list only compares");
        // The full popup is unchanged next time.
        s.dispatch(Command::CloseBranches);
        open_branches(&mut s);
        assert!(s.state().branch_rows("").iter().any(|r| r.section == "Recent" || r.section.is_empty()) && s.state().branch_rows("HEAD~1").iter().all(|r| !matches!(r.target, BranchTarget::Revision(_))));
    }

    #[test]
    fn the_comparisons_are_git_commands_and_say_why_outside_git() {
        let d = tempfile::tempdir().unwrap();
        let mut s = session(d.path());
        s.dispatch(Command::Open(write(d.path(), "a.txt", b"x\n")));
        settle(&mut s, |s| opened(s) && s.state().repo_checked());
        for id in [CommandId::ShowLocalChanges, CommandId::CompareWithRevision, CommandId::CompareWithBase, CommandId::ShowBranchChanges, CommandId::CompareWithWorktree] {
            assert_eq!(s.disabled(id), Some("Not in a Git repository"), "{id:?}");
            assert_eq!(id.group(), crate::actions::CommandGroup::Git);
        }
        let (_bin, exe) = exe();
        assert_eq!(args(&s, &GitDiff::LocalChanges, &exe), Err("Show Local Changes: Not in a Git repository".into()));
    }

    use crate::branches::CheckoutChoice;
    use lenz_content::branches::Target;

    fn checkout(s: &mut Session, target: Target) {
        s.dispatch(Command::Checkout(target));
        settle(s, |s| !s.state().checking_out());
    }
    fn choose(s: &mut Session, c: CheckoutChoice) {
        s.dispatch(Command::CheckoutChoice(c));
        settle(s, |s| !s.state().checking_out());
    }
    fn on_disk(r: &tempfile::TempDir, f: &str) -> String {
        std::fs::read_to_string(r.path().join(f)).unwrap_or_default()
    }

    #[test]
    fn checkout_reloads_open_files_and_marks_removed_ones_unavailable_in_both_panes() {
        let (r, _wt, mut s) = repo_session();
        git(r.path(), &["switch", "-q", "feature/x"]);
        put(r.path(), "b.txt", "b on x\n");
        git(r.path(), &["rm", "-q", "a.txt"]);
        commit_all(r.path(), "x drops a, adds b");
        git(r.path(), &["switch", "-q", "main"]);
        put(r.path(), "b.txt", "");
        std::fs::remove_file(r.path().join("b.txt")).unwrap();
        s.dispatch(Command::OpenRetained(r.path().join("a.txt")));
        settle(&mut s, opened);
        s.dispatch(Command::Split(Split::Columns));
        checkout(&mut s, Target::Branch("feature/x".into()));
        assert_eq!(s.branch_label().as_deref(), Some("⎇ feature/x"));
        settle(&mut s, |s| (0..2).all(|p| s.state().shown(p).is_some_and(|t| t.unavailable.is_some())));
        assert!(s.status_details().contains("Checked out feature/x"), "{}", s.status_details());
        assert!(s.state().branches().is_none(), "the popup closes");
        checkout(&mut s, Target::Branch("main".into()));
        settle(&mut s, |s| (0..2).all(|p| s.state().shown(p).is_some_and(|t| t.unavailable.is_none() && t.loaded)));
        assert_eq!(on_disk(&r, "a.txt"), "main\n");
    }

    #[test]
    fn tracked_changes_in_the_way_offer_smart_force_or_cancel() {
        let (r, _wt, mut s) = repo_session();
        git(r.path(), &["switch", "-q", "feature/x"]);
        put(r.path(), "a.txt", "1\n2\n3\n");
        commit_all(r.path(), "x");
        git(r.path(), &["switch", "-q", "main"]);
        put(r.path(), "a.txt", "1\n2\n3\n");
        commit_all(r.path(), "main too");
        git(r.path(), &["switch", "-q", "feature/x"]);
        put(r.path(), "a.txt", "X\n2\n3\n");
        commit_all(r.path(), "x first line");
        git(r.path(), &["switch", "-q", "main"]);
        put(r.path(), "a.txt", "1\n2\nZ\n");
        checkout(&mut s, Target::Branch("feature/x".into()));
        let p = s.state().checkout_problem().unwrap();
        assert_eq!((p.tracked.clone(), p.untracked.clone()), (vec!["a.txt".to_string()], vec![]));
        assert_eq!(s.checkout_choices().into_iter().map(|c| c.0).collect::<Vec<_>>(), ["Smart Checkout", "Force Checkout", "Cancel"]);
        assert!(s.checkout_text().unwrap().contains("a.txt"));
        choose(&mut s, CheckoutChoice::Cancel);
        assert!(s.state().checkout_problem().is_none());
        assert_eq!(s.branch_label().as_deref(), Some("⎇ main"));
        // Smart: switched, the edit merged back.
        checkout(&mut s, Target::Branch("feature/x".into()));
        choose(&mut s, CheckoutChoice::Smart);
        assert_eq!(s.branch_label().as_deref(), Some("⎇ feature/x"));
        assert_eq!(on_disk(&r, "a.txt"), "X\n2\nZ\n");
        // Force, back to main: a second step names everything discarded, then discards it.
        put(r.path(), "a.txt", "X\n2\nZZ\n");
        checkout(&mut s, Target::Branch("main".into()));
        choose(&mut s, CheckoutChoice::Force);
        assert_eq!(s.state().checkout_problem().unwrap().discards.as_deref(), Some(&["a.txt".to_string()][..]));
        assert_eq!(s.checkout_choices().into_iter().map(|c| c.0).collect::<Vec<_>>(), ["Force Checkout", "Cancel"]);
        assert!(s.checkout_text().unwrap().contains("discard"), "{:?}", s.checkout_text());
        assert_eq!(on_disk(&r, "a.txt"), "X\n2\nZZ\n", "nothing yet");
        choose(&mut s, CheckoutChoice::ConfirmForce);
        assert_eq!(s.branch_label().as_deref(), Some("⎇ main"));
        assert_eq!(on_disk(&r, "a.txt"), "1\n2\n3\n");
    }

    #[test]
    fn untracked_files_in_the_way_offer_only_cancel() {
        let (r, _wt, mut s) = repo_session();
        git(r.path(), &["switch", "-q", "feature/x"]);
        put(r.path(), "new.txt", "theirs\n");
        commit_all(r.path(), "x adds new");
        git(r.path(), &["switch", "-q", "main"]);
        put(r.path(), "new.txt", "mine\n");
        checkout(&mut s, Target::Branch("feature/x".into()));
        assert_eq!(s.state().checkout_problem().unwrap().untracked, ["new.txt"]);
        assert_eq!(s.checkout_choices().into_iter().map(|c| c.0).collect::<Vec<_>>(), ["Cancel"]);
        assert!(s.checkout_text().unwrap().contains("Move or delete"), "{:?}", s.checkout_text());
        choose(&mut s, CheckoutChoice::Smart); // not offered: nothing happens
        assert_eq!(on_disk(&r, "new.txt"), "mine\n");
    }

    #[test]
    fn remote_branches_tags_and_revisions_check_out() {
        let (r, _wt, mut s) = repo_session();
        checkout(&mut s, Target::Remote("origin/topic".into()));
        assert_eq!(s.branch_label().as_deref(), Some("⎇ topic"));
        checkout(&mut s, Target::Detached("v1".into()));
        let sha = git(r.path(), &["rev-parse", "v1"]).trim().to_string();
        assert_eq!(s.branch_label(), Some(format!("⎇ {}", &sha[..7])));
        assert_eq!(s.validate_revision("v1"), Ok(sha));
        assert_eq!(s.validate_revision("nope"), Err("No commit named \"nope\"".into()));
        checkout(&mut s, Target::Branch("no-such-branch".into()));
        assert!(s.status_message().contains("no-such-branch"), "git's own words: {}", s.status_message());
    }

    #[test]
    fn new_branches_are_validated_and_made() {
        let (_r, _wt, mut s) = repo_session();
        assert_eq!(s.validate_branch_name("other"), Err("A branch named \"other\" already exists".into()));
        assert_eq!(s.validate_branch_name("bad..name"), Err("\"bad..name\" is not a valid branch name".into()));
        assert_eq!(s.validate_branch_name("fresh"), Ok(()));
        s.dispatch(Command::NewBranch { name: "kept".into(), from: None, checkout: false });
        settle(&mut s, |s| !s.state().checking_out());
        assert_eq!(s.branch_label().as_deref(), Some("⎇ main"));
        assert!(s.status_message().contains("Created branch kept"), "{}", s.status_message());
        s.dispatch(Command::NewBranch { name: "fresh".into(), from: Some("v1".into()), checkout: true });
        settle(&mut s, |s| !s.state().checking_out());
        assert_eq!(s.branch_label().as_deref(), Some("⎇ fresh"));
    }

    #[test]
    fn a_checkout_answered_after_the_folder_changed_is_still_reported_there() {
        let (r, wt, mut s) = repo_session();
        s.dispatch(Command::Checkout(Target::Branch("feature/x".into())));
        s.dispatch(Command::OpenRoot(wt.path().join("second")));
        settle(&mut s, |s| !s.state().checking_out() && s.branch_label().as_deref() == Some("⎇ other"));
        let said = format!("Checked out feature/x in {}", std::fs::canonicalize(r.path()).unwrap().display());
        assert!(s.status_details().contains(&said), "{}", s.status_details());
        assert_eq!(s.branch_label().as_deref(), Some("⎇ other"), "this folder's branch is untouched");
    }

    #[test]
    fn a_new_branch_over_blocking_changes_retries_itself_from_the_dialog() {
        let (r, _wt, mut s) = repo_session();
        git(r.path(), &["switch", "-q", "feature/x"]);
        put(r.path(), "a.txt", "x\n");
        commit_all(r.path(), "x");
        git(r.path(), &["switch", "-q", "main"]);
        // An earlier checkout, so a stale target would show.
        checkout(&mut s, Target::Branch("other".into()));
        checkout(&mut s, Target::Branch("main".into()));
        put(r.path(), "a.txt", "mine\n");
        s.dispatch(Command::NewBranch { name: "fresh".into(), from: Some("refs/heads/feature/x".into()), checkout: true });
        settle(&mut s, |s| !s.state().checking_out());
        assert!(s.checkout_text().unwrap().starts_with("Checking out fresh would overwrite"), "{:?}", s.checkout_text());
        choose(&mut s, CheckoutChoice::Smart);
        assert_eq!(s.branch_label().as_deref(), Some("⎇ fresh"));
        assert!(s.status_message().contains("kept in the stash"), "{}", s.status_message());
    }

    #[test]
    fn a_second_checkout_waits_for_the_first() {
        let (r, _wt, mut s) = repo_session();
        // A post-checkout hook that takes a while keeps the first checkout running.
        let hook = r.path().join(".git/hooks/post-checkout");
        std::fs::write(&hook, "#!/bin/sh\nsleep 1\n").unwrap();
        std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        s.dispatch(Command::Checkout(Target::Branch("feature/x".into())));
        assert_eq!(s.disabled(CommandId::GitBranches), Some("A checkout is running"));
        assert_eq!(s.disabled(CommandId::NewBranch), Some("A checkout is running"));
        s.dispatch(Command::Checkout(Target::Detached("v1".into())));
        assert!(s.status_message().contains("A checkout is running"), "{}", s.status_message());
        settle(&mut s, |s| !s.state().checking_out());
        assert_eq!(s.branch_label().as_deref(), Some("⎇ feature/x"), "the first one finished; the second never ran");
        assert_eq!(s.disabled(CommandId::GitBranches), None);
    }

    #[test]
    fn a_reftable_repository_shows_and_follows_its_branch() {
        let r = tempfile::tempdir().unwrap();
        if !lenz_content::git::testing::try_git(r.path(), &["init", "-q", "-b", "main", "--ref-format=reftable"]) {
            return;
        }
        put(r.path(), "a.txt", "a\n");
        commit_all(r.path(), "a");
        let mut s = session(&r.path().join(".lenz-test"));
        s.dispatch(Command::OpenRoot(r.path().to_path_buf()));
        settle(&mut s, |s| s.branch_label().is_some());
        assert_eq!(s.branch_label().as_deref(), Some("⎇ main"));
        std::thread::sleep(Duration::from_millis(20));
        git(r.path(), &["switch", "-q", "-c", "next"]);
        tick(&mut s);
        settle(&mut s, |s| s.branch_label().as_deref() == Some("⎇ next"));
    }

    #[test]
    fn a_worktree_opens_as_the_folder() {
        let (_r, wt, mut s) = repo_session();
        s.dispatch(Command::OpenWorktree(wt.path().join("second")));
        settle(&mut s, |s| s.branch_label().as_deref() == Some("⎇ other"));
        assert_eq!(s.state().project().unwrap().root(), wt.path().join("second"));
    }

    #[test]
    fn blame_runs_again_after_a_checkout_that_keeps_the_bytes() {
        let (r, _wt, mut s) = repo_session();
        // `twin` has a.txt with the same bytes from a commit of its own.
        git(r.path(), &["switch", "-q", "--orphan", "twin"]);
        put(r.path(), "a.txt", "main\n");
        commit_all(r.path(), "twin's own a");
        git(r.path(), &["switch", "-q", "main"]);
        s.dispatch(Command::Open(r.path().join("a.txt")));
        settle(&mut s, opened);
        s.dispatch(Command::SetAnnotate(true));
        settle(&mut s, |s| s.state().active().is_some_and(|t| t.annotation(0).is_some()));
        assert_eq!(s.state().active().unwrap().annotation(0).unwrap().commit.summary, "main");
        checkout(&mut s, Target::Branch("twin".into()));
        settle(&mut s, |s| s.state().active().and_then(|t| t.annotation(0)).is_some_and(|a| a.commit.summary == "twin's own a"));
    }

    #[test]
    fn a_smart_checkout_that_conflicts_keeps_the_stash_and_offers_lenz_diff() {
        let (r, _wt, mut s) = repo_session();
        git(r.path(), &["switch", "-q", "feature/x"]);
        put(r.path(), "a.txt", "x\n");
        commit_all(r.path(), "x");
        git(r.path(), &["switch", "-q", "main"]);
        assert_eq!(s.disabled(CommandId::ResolveConflicts), Some("Smart Checkout left no conflicts"));
        put(r.path(), "a.txt", "mine\n");
        checkout(&mut s, Target::Branch("feature/x".into()));
        choose(&mut s, CheckoutChoice::Smart);
        assert_eq!(s.branch_label().as_deref(), Some("⎇ feature/x"));
        assert!(s.status_message().contains("kept in the stash"), "{}", s.status_message());
        assert_eq!(s.disabled(CommandId::ResolveConflicts), None);
        let bin = tempfile::tempdir().unwrap();
        std::fs::write(bin.path().join("lenz-diff"), "").unwrap();
        let launch = s.merge_launch(&bin.path().join("lenz-code"), false).unwrap();
        assert_eq!((launch.args, launch.cwd), (vec!["merge".to_string()], Some(std::fs::canonicalize(r.path()).unwrap())));
    }

    #[test]
    #[ignore = "timing: cargo test -p reader-core --release --lib branches_timing -- --ignored --nocapture"]
    fn branches_timing_with_a_thousand_refs() {
        let (r, _wt, mut s) = repo_session();
        let head = git(r.path(), &["rev-parse", "HEAD"]).trim().to_string();
        let refs: String = (0..1000).map(|i| match i % 2 { 0 => format!("create refs/heads/feature/b{i} {head}\n"), _ => format!("create refs/tags/t{i} {head}\n") }).collect();
        lenz_content::git::run_with_input(r.path(), &["update-ref", "--stdin"], refs.into_bytes()).unwrap();
        for round in 0..5 {
            s.dispatch(Command::CloseBranches);
            let started = std::time::Instant::now();
            open_branches(&mut s);
            let loaded = started.elapsed();
            let filter = std::time::Instant::now();
            let n = s.state().branch_rows("b99").len();
            eprintln!("round {round}: popup rows {} loaded in {loaded:?}; filter 'b99' {n} rows in {:?}", s.state().branch_rows("").len(), filter.elapsed());
        }
        let label = std::time::Instant::now();
        for _ in 0..1000 { std::hint::black_box(s.branch_label()); }
        eprintln!("branch label: {:?} per call", label.elapsed() / 1000);
    }
}


#[test]
fn two_explorer_entries_compare_with_ctrl_d() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    for sub in ["sub", "other"] {
        std::fs::create_dir_all(root.join(sub)).unwrap();
    }
    let a = write(&root, "a.rs", b"fn a() {}\n");
    let b = write(&root, "b.rs", b"fn b() {}\n");
    write(&root, "sub/x.rs", b"x\n");
    write(&root, "other/x.rs", b"y\n");
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, |s| s.state().tree_rows().count() >= 4);
    let bin = d.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    for name in ["lenz-diff", "lenz-diff-tui"] {
        std::fs::write(bin.join(name), b"").unwrap();
    }
    let exe = bin.join("lenz-code");
    let mark_two = "Mark two files or folders in the explorer (Ctrl+click, or Space)";
    assert_eq!(s.disabled(CommandId::CompareSelected), Some(mark_two));
    // Ctrl+click on b.rs while a.rs is highlighted: both are marked.
    s.dispatch(Command::MarkTreeRow { path: "b.rs".into(), selected: Some("a.rs".into()) });
    assert_eq!(s.state().tree_marks(), ["a.rs".into(), "b.rs".into()] as [std::sync::Arc<str>; 2]);
    assert_eq!(s.disabled(CommandId::CompareSelected), None);
    let launch = s.compare_selected_launch(&exe, false).unwrap();
    assert_eq!(launch.args, ["--no-index".to_string(), a.to_string_lossy().into_owned(), b.to_string_lossy().into_owned()]);
    // Ctrl+click again unmarks: one mark compares nothing.
    s.dispatch(Command::MarkTreeRow { path: "b.rs".into(), selected: Some("b.rs".into()) });
    assert_eq!(s.state().tree_marks(), ["a.rs".into()] as [std::sync::Arc<str>; 1]);
    assert_eq!(s.disabled(CommandId::CompareSelected), Some(mark_two));
    assert_eq!(s.compare_selected_launch(&exe, false).unwrap_err(), format!("Compare Selected: {mark_two}"));
    // A third mark replaces the older; folders compare; a file with a folder is refused in Compare Selected's words.
    s.dispatch(Command::MarkTreeRow { path: "sub".into(), selected: None });
    s.dispatch(Command::MarkTreeRow { path: "other".into(), selected: None });
    assert_eq!(s.state().tree_marks(), ["sub".into(), "other".into()] as [std::sync::Arc<str>; 2]);
    assert_eq!(s.compare_selected_launch(&exe, true).unwrap().args[1..], [root.join("sub").to_string_lossy().into_owned(), root.join("other").to_string_lossy().into_owned()]);
    s.dispatch(Command::ClearTreeMarks);
    s.dispatch(Command::MarkTreeRow { path: "a.rs".into(), selected: Some("sub".into()) });
    assert_eq!(s.compare_selected_launch(&exe, false).unwrap_err(), "Compare Selected: pick a file to compare with a file, or a folder with a folder");
    // A marked entry that is no longer listed (its folder collapsed) is said so at use.
    s.dispatch(Command::ClearTreeMarks);
    s.dispatch(Command::ToggleDir("sub".into()));
    settle(&mut s, |s| s.state().tree_rows().any(|r| &*r.path == "sub/x.rs"));
    s.dispatch(Command::MarkTreeRow { path: "sub/x.rs".into(), selected: Some("a.rs".into()) });
    s.dispatch(Command::ToggleDir("sub".into()));
    assert!(s.compare_selected_launch(&exe, false).unwrap_err().ends_with("one marked entry is no longer listed"));
    // A deleted file: the missing side is named.
    s.dispatch(Command::ClearTreeMarks);
    s.dispatch(Command::MarkTreeRow { path: "b.rs".into(), selected: Some("a.rs".into()) });
    std::fs::remove_file(&a).unwrap();
    assert_eq!(s.compare_selected_launch(&exe, false).unwrap_err(), format!("Compare Selected: no such file or folder: {}", a.display()));
    // A library or source URI has no file: never marked.
    s.dispatch(Command::ClearTreeMarks);
    s.dispatch(Command::MarkTreeRow { path: "jdt://contents/x".into(), selected: Some("b.rs".into()) });
    assert!(s.state().tree_marks().is_empty());
    // Opening another folder forgets the marks.
    s.dispatch(Command::MarkTreeRow { path: "b.rs".into(), selected: None });
    s.dispatch(Command::OpenRoot(d.path().to_path_buf()));
    assert!(s.state().tree_marks().is_empty());
    assert_eq!(CommandId::CompareSelected.shortcut(), Some("Ctrl+D"));
    assert_eq!(CommandId::CompareSelected.label(), "Compare Selected Saved Disk Inputs");
}

#[test]
fn current_views_share_source_identity_and_preserve_source_coordinates() {
    let d = tempfile::tempdir().unwrap();
    let bytes: Vec<u8> = [0xff, 0xfe].into_iter().chain("α\r\n😀z\n".encode_utf16().flat_map(u16::to_le_bytes)).collect();
    let path = write(d.path(), "current.txt", &bytes);
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path.clone()), "current.txt");
    let first = s.state().active().unwrap().id();
    let snapshot = s.state().view(first).unwrap().snapshot().clone();
    assert_eq!(snapshot.source_meta().original_encoding(), "UTF-16LE");
    assert!(snapshot.source_meta().had_bom());
    s.dispatch(Command::Split(Split::Columns));
    let second = s.state().active().unwrap().id();
    assert_ne!(first, second);
    assert!(Arc::ptr_eq(&snapshot, s.state().view(second).unwrap().snapshot()));
    s.dispatch(Command::GoToLine(LineColumn { line: 2, column: Some(2) }));
    assert_eq!(s.state().selection(), Some(Selection { anchor: 8, head: 8 }));
    assert_ne!(s.state().view(first).unwrap().tab().selection(), s.state().selection());
    s.dispatch(Command::SelectAll);
    assert_eq!(s.dispatch(Command::Copy).copy.as_deref(), Some("α\r\n😀z\n"));
    let mut other = session(&d.path().join("independent"));
    open(&mut other, Command::Open(path), "current.txt");
    assert!(!Arc::ptr_eq(&snapshot, other.state().view(other.state().active().unwrap().id()).unwrap().snapshot()));
    s.dispatch(Command::GoToLine(LineColumn { line: 99, column: None }));
    assert_eq!(s.state().selection(), Some(Selection { anchor: 10, head: 10 }), "Reader retains the final empty source line");
}

#[test]
fn current_text_is_released_with_evicted_clean_content_and_reloaded_with_its_identity() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "a.txt", &[b'a'; 128]);
    let b = write(d.path(), "b.txt", &[b'b'; 128]);
    let c = write(d.path(), "c.txt", &[b'c'; 128]);
    let fourth = write(d.path(), "d.txt", &[b'd'; 128]);
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(a.clone()), "a.txt");
    let tab = s.state().active().unwrap().id();
    let admitted = s.state().view(tab).unwrap().snapshot().clone();
    let budget = admitted.storage_bytes() * 2;
    s.cache = InactiveCache::new(budget);
    let worker_document = analysis_document(&mut s);
    let loaded = Arc::downgrade(&worker_document);
    assert!(s.analysis.ensure(crate::analysis::Input::new(admitted.clone(), loaded.clone(), 0, true), Priority::Background));
    assert!(s.analysis.ready(&admitted, 0).is_some(), "matching saved text registers synchronously");
    drop(admitted);
    drop(worker_document);
    let snapshot = s.state().view(tab).unwrap().snapshot();
    let key = snapshot.key();
    let weak = Arc::downgrade(snapshot);
    open(&mut s, Command::OpenRetained(b.clone()), "b.txt");
    assert!(s.state().view(tab).is_none());
    assert!(weak.upgrade().is_some(), "the inactive cache retains reusable text");
    assert!(s.cache.peek(&a).is_some_and(|cached| std::sync::Weak::ptr_eq(&weak, &Arc::downgrade(&cached))), "reuse comes from the actual inactive cache");
    open(&mut s, Command::OpenRetained(a.clone()), "a.txt");
    assert!(Arc::ptr_eq(&weak.upgrade().unwrap(), s.state().view(tab).unwrap().snapshot()));
    open(&mut s, Command::OpenRetained(b), "b.txt");
    open(&mut s, Command::OpenRetained(c), "c.txt");
    open(&mut s, Command::OpenRetained(fourth), "d.txt");
    assert!(weak.upgrade().is_none(), "eviction frees the actual indexed text");
    assert!(loaded.upgrade().is_none(), "admission must not turn evicted frontend data into a strong cache");
    assert!(s.inactive_bytes() <= budget, "retained source storage stays within the fixture budget");
    s.dispatch(Command::ActivateTab(tab));
    settle(&mut s, |s| s.state().view(tab).is_some());
    let current = s.state().view(tab).unwrap();
    assert_eq!(current.snapshot().key().buffer, key.buffer);
    assert_eq!(current.snapshot().copy_range(0..128).as_deref(), Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
}

#[test]
fn current_view_loading_empty_and_image_sources_keep_reader_behavior() {
    let d = tempfile::tempdir().unwrap();
    let empty = write(d.path(), "empty.txt", b"");
    let image = write(d.path(), "image.png", &png(1, 1, [255, 0, 0, 255]));
    let mut s = session(d.path());
    assert!(s.state().view(TabId(0)).is_none());
    s.dispatch(Command::SelectAll);
    s.dispatch(Command::Copy);
    s.dispatch(Command::Reload);
    s.dispatch(Command::OpenRetained(empty));
    let tab = s.state().active().unwrap().id();
    assert!(s.state().view(tab).is_none(), "a load is admitted only on completion");
    settle(&mut s, |s| s.state().view(tab).is_some());
    s.dispatch(Command::GoToLine(LineColumn { line: 9, column: Some(9) }));
    assert_eq!(s.state().selection(), Some(Selection { anchor: 0, head: 0 }));
    assert_eq!(s.dispatch(Command::Copy).copy, None);
    open(&mut s, Command::OpenRetained(image), "image.png");
    let view = s.state().view(s.state().active().unwrap().id()).unwrap();
    assert!(view.snapshot().image().is_some());
    assert_eq!(s.disabled(CommandId::GoToLine), Some("An image has no lines"));
}

#[cfg(unix)]
#[test]
fn canonical_source_aliases_keep_one_current_identity() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "source.txt", b"original\n");
    let alias = d.path().join("alias.txt");
    std::os::unix::fs::symlink(&path, &alias).unwrap();
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(alias), "alias.txt");
    let first = s.state().active().unwrap().id();
    let current = s.state().view(first).unwrap().snapshot().clone();
    s.dispatch(Command::Split(Split::Columns));
    s.dispatch(Command::OpenRetained(path));
    settle(&mut s, opened);
    assert!(Arc::ptr_eq(&current, s.state().view(s.state().active().unwrap().id()).unwrap().snapshot()));
}

#[test]
fn current_caret_and_word_selection_keep_unicode_coordinates_on_a_long_line() {
    let d = tempfile::tempdir().unwrap();
    let text = format!("{}😀z", "a ".repeat(450_000));
    let path = write(d.path(), "long.txt", text.as_bytes());
    let mut s = session(d.path());
    open(&mut s, Command::Open(path), "long.txt");
    s.dispatch(Command::GoToLine(LineColumn { line: 1, column: Some(900_002) }));
    assert_eq!(s.state().selection(), Some(Selection { anchor: 900_004, head: 900_004 }));
    s.dispatch(Command::MoveCaret(Motion::Left));
    assert_eq!(s.state().selection(), Some(Selection { anchor: 900_000, head: 900_000 }));
    s.dispatch(Command::ClickSelect { at: 900_002, clicks: 2 });
    assert_eq!(s.dispatch(Command::Copy).copy.as_deref(), Some("😀"));
    let view = s.state().view(s.state().active().unwrap().id()).unwrap();
    let window = view.line_window(0, 900_000, 4).unwrap();
    assert_eq!(window.text, "😀");
    assert_eq!(window.bytes, 900_000..900_004);
    assert_eq!(window.utf16, 900_000..900_002);
}

#[test]
fn current_refresh_updates_split_compatibility_without_merging_view_positions_or_sessions() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "refresh.rs", b"fn alpha() {}\nfn beta() {}\n");
    let mut s = session(d.path());
    let mut other = session(&d.path().join("other"));
    open(&mut s, Command::OpenRetained(path.clone()), "refresh.rs");
    open(&mut other, Command::OpenRetained(path.clone()), "refresh.rs");
    s.watcher = Watcher::new(|_| true);
    other.watcher = Watcher::new(|_| true);
    let first = s.state().active().unwrap().id();
    s.dispatch(Command::Select(Selection { anchor: 3, head: 8 }));
    s.dispatch(Command::Split(Split::Columns));
    let second = s.state().active().unwrap().id();
    s.dispatch(Command::GoToLine(LineColumn { line: 2, column: Some(4) }));
    let second_position = s.state().selection();
    settle(&mut s, |s| s.state().tabs().iter().all(|tab| s.state().view(tab.id()).is_some_and(|view| view.highlights().is_some())));
    let previous = s.state().view(first).unwrap().snapshot().key();
    std::fs::write(&path, b"fn alpha() {}\nfn zeta() {}\n").unwrap();
    s.dispatch(Command::Reload);
    settle(&mut s, |s| s.state().view(second).unwrap().snapshot().key() != previous);
    let left = s.state().view(first).unwrap();
    let right = s.state().view(second).unwrap();
    assert!(Arc::ptr_eq(left.snapshot(), right.snapshot()));
    assert_eq!(left.tab().buffer, right.tab().buffer, "both views resolve the shared source identity");
    assert_eq!(left.copy_selection().as_deref(), Some("alpha"));
    assert_eq!(right.tab().selection(), second_position);
    assert_eq!(right.snapshot().copy_range(17..21).as_deref(), Some("zeta"));
    assert_eq!(source_line(left.snapshot(), 1), "fn zeta() {}");
    assert!(left.highlights().is_none() && right.highlights().is_none(), "derived products from the previous version are not presented");
    let other = other.state().view(other.state().active().unwrap().id()).unwrap();
    assert_eq!(other.snapshot().copy_range(17..21).as_deref(), Some("beta"));
    assert_eq!(source_line(other.snapshot(), 0), "fn alpha() {}");
}

#[test]
fn current_cached_rename_reactivation_updates_path_and_preserves_held_snapshot() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "before.txt", b"original\r\n");
    let moved = d.path().join("after.txt");
    let other = write(d.path(), "other.txt", b"other\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path.clone()), "before.txt");
    let tab = s.state().active().unwrap().id();
    let held = s.state().view(tab).unwrap().snapshot().clone();
    open(&mut s, Command::OpenRetained(other.clone()), "other.txt");
    std::fs::remove_file(other).unwrap();
    settle(&mut s, |s| s.state().active().is_some_and(|tab| tab.unavailable().is_some()));
    std::fs::rename(&path, &moved).unwrap();
    settle(&mut s, |s| s.state().tabs().iter().any(|t| t.id() == tab && t.source() == moved));
    s.dispatch(Command::ActivateTab(tab));
    settle(&mut s, |s| s.state().view(tab).is_some());
    let current = s.state().view(tab).unwrap().snapshot();
    assert_eq!(current.source(), moved);
    assert_eq!(current.key().buffer, held.key().buffer);
    assert_eq!(held.source(), path, "a retained snapshot keeps its original metadata");
    s.dispatch(Command::SelectAll);
    assert_eq!(s.dispatch(Command::Copy).copy.as_deref(), Some("original\r\n"));
}
