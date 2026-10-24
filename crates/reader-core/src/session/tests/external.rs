use super::*;
use crate::external::Choice;

#[test]
fn dirty_external_open_offers_saved_version_or_cancel_without_losing_history() {
    for default_app in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let (mut s, path, tab) = close_fixture(dir.path(), "external.txt");
        assert!(s.editor_plan(&open_host(&[("VISUAL", "code")]), None, default_app).is_err(), "the old raw plan API cannot bypass the shared decision");
        let command = s.external_command(&open_host(&[("VISUAL", "code")]), None, default_app).unwrap();
        assert!(s.dispatch(command).external.is_none());
        let prompt = s.external_prompt().unwrap();
        assert_eq!(prompt.path, path);
        assert_eq!(Choice::SaveAndOpen.label(), "Save and Open");
        assert_eq!(Choice::OpenSavedVersion.label(), "Open Saved Version");
        assert_eq!(Choice::Cancel.label(), "Cancel");
        assert!(s.dispatch(Command::ChooseExternal { ticket: prompt.ticket, choice: Choice::Cancel }).external.is_none());
        assert!(s.external_prompt().is_none());
        assert!(active_view(&s).dirty() && active_view(&s).can_undo());
        assert_eq!(source_text(s.state().current_source().unwrap()), "draft λ\r\n");

        let command = s.external_command(&open_host(&[("VISUAL", "code")]), None, default_app).unwrap();
        assert!(s.dispatch(command).external.is_none());
        let prompt = s.external_prompt().unwrap();
        let plan = s.dispatch(Command::ChooseExternal { ticket: prompt.ticket, choice: Choice::OpenSavedVersion }).external.unwrap();
        assert_eq!(plan.launch.program, if default_app { "xdg-open" } else { "code" });
        assert_eq!(plan.launch.args, [path.to_string_lossy().into_owned()], "dirty caret coordinates do not describe saved disk text");
        assert!(s.dispatch(Command::ChooseExternal { ticket: prompt.ticket, choice: Choice::OpenSavedVersion }).external.is_none());
        assert!(active_view(&s).dirty() && active_view(&s).can_undo());
        assert_eq!(std::fs::read(&path).unwrap(), b"saved\r\n");
        s.dispatch(Command::Undo { tab });
        assert_eq!(source_text(s.state().current_source().unwrap()), "saved\r\n");
    }
}

fn collect_until(s: &mut Session, done: impl Fn(&Session) -> bool) -> Vec<lenz_content::open::Plan> {
    let rx = s.completions();
    let until = std::time::Instant::now() + Duration::from_secs(10);
    let mut launches = Vec::new();
    while !done(s) {
        assert!(std::time::Instant::now() < until, "external operation timed out: {}", s.status_message());
        if let Ok(completion) = rx.recv_blocking_timeout() && let Some(plan) = s.dispatch(completion).external { launches.push(plan); }
    }
    launches
}

#[test]
fn rejected_or_uncertain_external_save_cancels_launch_and_requires_a_fresh_decision() {
    use crate::save::Status;
    use lenz_content::write::readiness_fault::{at_path, Fault};
    for fault in [Fault::BeforeReplace, Fault::AfterReplace] {
        let dir = tempfile::tempdir().unwrap();
        let (mut s, path, tab) = close_fixture(dir.path(), "failed-external.txt");
        let guard = at_path(&path, fault);
        let command = s.external_command(&open_host(&[("VISUAL", "code")]), None, false).unwrap();
        s.dispatch(command);
        let ticket = s.external_prompt().unwrap().ticket;
        assert!(s.dispatch(Command::ChooseExternal { ticket, choice: Choice::SaveAndOpen }).external.is_none());
        let launches = collect_until(&mut s, |s| !active_view(s).save_status().busy());
        assert!(launches.is_empty() && s.external_prompt().is_none());
        assert!(active_view(&s).dirty() && active_view(&s).can_undo());
        assert_eq!(source_text(s.state().current_source().unwrap()), "draft λ\r\n");
        assert!(s.dispatch(Command::ChooseExternal { ticket, choice: Choice::OpenSavedVersion }).external.is_none());
        drop(guard);
        if fault == Fault::AfterReplace {
            assert!(matches!(active_view(&s).save_status(), Status::Uncertain(_)));
            let command = s.external_command(&open_host(&[("VISUAL", "code")]), None, false).unwrap();
            s.dispatch(command);
            let prompt = s.external_prompt().unwrap();
            assert!(prompt.saved_disabled.is_some());
            assert!(s.dispatch(Command::ChooseExternal { ticket: prompt.ticket, choice: Choice::OpenSavedVersion }).external.is_none());
            s.dispatch(Command::ChooseExternal { ticket: prompt.ticket, choice: Choice::Cancel });
            s.dispatch(Command::RecheckSave { tab });
            assert!(collect_until(&mut s, |s| matches!(active_view(s).save_status(), Status::Checked(_))).is_empty());
            let command = s.external_command(&open_host(&[("VISUAL", "code")]), None, false).unwrap();
            s.dispatch(command);
            let prompt = s.external_prompt().unwrap();
            assert!(prompt.save_disabled.is_some() && prompt.saved_disabled.is_none());
            assert!(s.dispatch(Command::ChooseExternal { ticket: prompt.ticket, choice: Choice::OpenSavedVersion }).external.is_some());
            assert!(matches!(active_view(&s).save_status(), Status::Checked(_)), "opening saved bytes does not accept a new baseline");
        } else {
            assert!(matches!(active_view(&s).save_status(), Status::Rejected(_)));
            assert_eq!(std::fs::read(&path).unwrap(), b"saved\r\n");
        }
    }
}

#[test]
fn cancellation_edits_or_file_switching_never_resume_a_saved_external_launch() {
    for change in [0, 1, 2] {
        let dir = tempfile::tempdir().unwrap();
        let (mut s, path, tab) = close_fixture(dir.path(), "held-external.txt");
        let buffer = s.state().current_source().unwrap().key().buffer;
        let (entered, entered_rx) = std::sync::mpsc::channel();
        let gates: Vec<_> = (0..crate::jobs::MAX_JOBS).map(|_| {
            let (release, wait) = std::sync::mpsc::channel();
            let entered = entered.clone();
            s.jobs.submit(Priority::Visible, move || { entered.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(10)); });
            release
        }).collect();
        for _ in &gates { entered_rx.recv_timeout(Duration::from_secs(5)).unwrap(); }
        let command = s.external_command(&open_host(&[("VISUAL", "code")]), None, false).unwrap();
        s.dispatch(command);
        let ticket = s.external_prompt().unwrap().ticket;
        assert!(s.dispatch(Command::ChooseExternal { ticket, choice: Choice::SaveAndOpen }).external.is_none());
        assert!(s.external_prompt().unwrap().waiting);
        let effect = match change {
            0 => s.dispatch(Command::ChooseExternal { ticket, choice: Choice::Cancel }),
            1 => {
                let expected = s.state().current_source().unwrap().key();
                s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..0, text: "newer ".into() }], kind: crate::text::EditKind::Paste })
            }
            _ => {
                let other = write(dir.path(), "other.txt", b"other saved");
                s.dispatch(Command::OpenRetained(other))
            }
        };
        assert!(effect.external.is_none() && s.external_prompt().is_none());
        for gate in gates { gate.send(()).unwrap(); }
        assert!(collect_until(&mut s, |s| !s.state.sources.buffers[&buffer].save.status.busy()).is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), "draft λ\r\n".as_bytes(), "explicitly requested Save still completes independently");
        assert!(s.dispatch(Command::ChooseExternal { ticket, choice: Choice::SaveAndOpen }).external.is_none());
        s.dispatch(Command::ActivateTab(tab));
        assert!(s.state.sources.buffers[&buffer].draft.as_ref().unwrap().can_undo());
        let expected = if change == 1 { "newer draft λ\r\n" } else { "draft λ\r\n" };
        assert_eq!(source_text(s.state().current_source().unwrap()), expected);
    }
}

#[test]
fn save_and_external_open_emits_once_only_after_confirmed_bytes_and_keeps_undo() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, path, tab) = close_fixture(dir.path(), "save-external.txt");
    s.dispatch(Command::Select(Selection { anchor: 0, head: 0 }));
    let command = s.external_command(&open_host(&[("VISUAL", "code")]), None, false).unwrap();
    assert!(s.dispatch(command).external.is_none());
    let ticket = s.external_prompt().unwrap().ticket;
    let mut plan = s.dispatch(Command::ChooseExternal { ticket, choice: Choice::SaveAndOpen }).external;
    let rx = s.completions();
    let until = std::time::Instant::now() + Duration::from_secs(10);
    while plan.is_none() {
        assert!(std::time::Instant::now() < until, "external Save never finished: {}", s.status_message());
        if let Ok(completion) = rx.recv_blocking_timeout() { plan = s.dispatch(completion).external; }
    }
    assert_eq!(std::fs::read(&path).unwrap(), "draft λ\r\n".as_bytes());
    assert!(!active_view(&s).dirty());
    assert!(s.external_prompt().is_none());
    let plan = plan.unwrap();
    assert_eq!(plan.launch.args, ["--goto".to_owned(), format!("{}:1:1", path.display())]);
    assert!(s.dispatch(Command::ChooseExternal { ticket, choice: Choice::SaveAndOpen }).external.is_none());
    s.dispatch(Command::Undo { tab });
    assert!(active_view(&s).dirty());
    assert_eq!(source_text(s.state().current_source().unwrap()), "saved\r\n");
    assert_eq!(std::fs::read(path).unwrap(), "draft λ\r\n".as_bytes());
}
