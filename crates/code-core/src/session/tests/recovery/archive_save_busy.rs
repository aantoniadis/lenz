//! Restore cannot invalidate an export receipt or release its destination while its worker runs.
use super::*;
use crate::archive::Location;

fn gate_workers(s: &Session) -> Vec<std::sync::mpsc::Sender<()>> {
    let (started, entered) = std::sync::mpsc::channel();
    let gates: Vec<_> = (0..crate::jobs::MAX_JOBS)
        .map(|_| {
            let (release, wait) = std::sync::mpsc::channel();
            let started = started.clone();
            s.jobs.submit(Priority::Visible, move || {
                started.send(()).unwrap();
                let _ = wait.recv_timeout(Duration::from_secs(10));
            });
            release
        })
        .collect();
    for _ in &gates {
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
    }
    gates
}

fn open_member(s: &mut Session, location: &Location) {
    s.dispatch(Command::OpenArchiveEntry(location.clone()));
    settle(s, |s| {
        s.state()
            .current_source()
            .is_some_and(|source| source.source_meta().archive() == Some(location))
            && s.state().loading().is_none()
    });
}

#[test]
fn archive_recovery_restore_preserves_pending_save_as_capture_and_destination_reservation() {
    for restore_completion_already_pending in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let container = lenz_archive::testing::zip(&[("a.txt", b"saved\r\n")]);
        let path = write(dir.path(), "work.zip", &container);
        let location = Location::new(path.clone(), vec!["a.txt".into()]).unwrap();
        {
            let mut s = session(dir.path());
            open_member(&mut s, &location);
            edit(&mut s, "recovered draft\r\n");
            s.flush_recovery_for_test();
        }
        let mut s = session(dir.path());
        let ticket = s.recovery_prompt().unwrap().ticket;
        s.dispatch(Command::ChooseRecoveredDraft {
            ticket,
            choice: Choice::Later,
        });
        open_member(&mut s, &location);
        let tab = s.state().active().unwrap().id();
        let buffer = s.state().view(tab).unwrap().snapshot().key().buffer;
        assert!(!s.state().tab_dirty(tab));
        let pending_restore = if restore_completion_already_pending {
            s.dispatch(Command::ShowRecoveredDrafts);
            s.dispatch(Command::ChooseRecoveredDraft {
                ticket,
                choice: Choice::Restore,
            });
            s.dispatch(Command::ChooseRecoveredDraft {
                ticket,
                choice: Choice::Later,
            });
            // Let the real reader finish, but retain its result until Save As has captured this clean buffer.
            Some(loop {
                let completion = s.completions().recv_blocking_timeout().unwrap();
                if matches!(completion, Completion::ArchiveRecovered { .. }) {
                    break completion;
                }
                s.dispatch(completion);
            })
        } else {
            None
        };
        let gates = gate_workers(&s);
        let export = dir.path().join("export.txt");
        s.dispatch(Command::SaveAs {
            tab,
            path: export.clone(),
        });
        assert!(s.state().view(tab).unwrap().save_status().busy());
        let request = s.state.sources.buffers[&buffer]
            .save
            .captured
            .as_ref()
            .unwrap()
            .request;
        s.dispatch(Command::ShowRecoveredDrafts);
        if let Some(completion) = pending_restore {
            s.dispatch(completion);
        } else {
            assert!(s.recovery_prompt().unwrap().restore_disabled.is_some());
            s.dispatch(Command::ChooseRecoveredDraft {
                ticket,
                choice: Choice::Restore,
            });
        }
        assert!(s.recovery_prompt().unwrap().restore_disabled.is_some());
        assert!(
            !s.recovery.restoring(ticket),
            "only the ongoing save blocks this clean buffer's restore"
        );
        assert_eq!(s.recovered_drafts().len(), 1);
        assert_eq!(text(&s), "saved\r\n");
        assert!(s.state().view(tab).unwrap().save_status().busy());
        assert_eq!(
            s.state.sources.buffers[&buffer]
                .save
                .captured
                .as_ref()
                .unwrap()
                .request,
            request
        );
        assert!(s.save_as_destination_reserved(&export));
        assert!(!export.exists());
        for gate in gates {
            gate.send(()).unwrap();
        }
        settle(&mut s, |s| {
            !s.state().view(tab).unwrap().save_status().busy()
        });
        assert_eq!(std::fs::read(&export).unwrap(), b"saved\r\n");
        assert_eq!(std::fs::read(&path).unwrap(), container);
        assert!(!s.save_as_destination_reserved(&export));
        assert_eq!(
            s.recovered_drafts().len(),
            1,
            "the unsaved recovery draft remains undecided"
        );
        assert_eq!(s.recovery_prompt().unwrap().restore_disabled, None);
        s.dispatch(Command::ChooseRecoveredDraft {
            ticket,
            choice: Choice::Restore,
        });
        settle(&mut s, |s| {
            s.recovered_drafts().is_empty() && text(s) == "recovered draft\r\n"
        });
        assert_eq!(std::fs::read(&export).unwrap(), b"saved\r\n");
        assert_eq!(std::fs::read(path).unwrap(), container);
    }
}
