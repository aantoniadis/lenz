//! An external archive-format restriction must never strand already recovered text.
use super::*;
use crate::archive::Location;

fn open_member(s: &mut Session, location: &Location) {
    s.dispatch(Command::OpenArchiveEntry(location.clone()));
    settle(s, |s| {
        s.state()
            .current_source()
            .is_some_and(|source| source.source_meta().archive() == Some(location))
            && s.state().loading().is_none()
    });
}

fn append_opaque_tail(path: &Path) -> Vec<u8> {
    let mut bytes = std::fs::read(path).unwrap();
    bytes.extend_from_slice(b"opaque trailing bytes");
    std::fs::write(path, &bytes).unwrap();
    bytes
}

#[test]
fn recovered_draft_from_archive_that_became_readonly_stays_editable_and_exports_exact_encoding() {
    for member_changed in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let original = b"\xff\xfes\0a\0v\0e\0d\0\r\0\n\0";
        let path = write(
            dir.path(),
            "work.zip",
            &lenz_archive::testing::zip(&[("a.txt", original)]),
        );
        let location = Location::new(path.clone(), vec!["a.txt".into()]).unwrap();
        {
            let mut s = session(dir.path());
            open_member(&mut s, &location);
            edit(&mut s, "recovered λ\r\n");
            s.flush_recovery_for_test();
        }
        if member_changed {
            std::fs::write(&path, lenz_archive::testing::zip(&[("a.txt", b"external")])).unwrap();
        }
        let container = append_opaque_tail(&path);
        let mut s = session(dir.path());
        let ticket = s.recovery_prompt().unwrap().ticket;
        s.dispatch(Command::ChooseRecoveredDraft {
            ticket,
            choice: Choice::Restore,
        });
        settle(&mut s, |s| {
            s.recovered_drafts().is_empty() && s.state().current_source().is_some()
        });
        assert_eq!(text(&s), "recovered λ\r\n");
        let tab = s.state().active().unwrap().id();
        assert_eq!(
            s.state().view(tab).unwrap().edit_disabled(),
            None,
            "already recovered work must stay editable"
        );
        assert!(
            s.disabled(CommandId::Save).is_some(),
            "the opaque archive must remain protected"
        );
        assert_eq!(
            s.disabled(CommandId::SaveAs),
            None,
            "a standalone export must remain available"
        );
        edit(&mut s, "edited recovery λ\r\n");
        let export = dir.path().join("export.txt");
        s.dispatch(Command::SaveAs {
            tab,
            path: export.clone(),
        });
        settle(&mut s, |s| {
            !s.state().view(tab).unwrap().save_status().busy()
        });
        let mut expected = vec![0xff, 0xfe];
        expected.extend(
            "edited recovery λ\r\n"
                .encode_utf16()
                .flat_map(u16::to_le_bytes),
        );
        assert_eq!(std::fs::read(export).unwrap(), expected);
        assert_eq!(std::fs::read(&path).unwrap(), container);
    }
}

#[test]
fn ordinary_readonly_archive_member_exports_but_remains_uneditable() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        "work.zip",
        &lenz_archive::testing::zip(&[("a.txt", b"member\r\n")]),
    );
    let container = append_opaque_tail(&path);
    let location = Location::new(path.clone(), vec!["a.txt".into()]).unwrap();
    let mut s = session(dir.path());
    open_member(&mut s, &location);
    let tab = s.state().active().unwrap().id();
    assert!(s.state().view(tab).unwrap().edit_disabled().is_some());
    assert!(s.disabled(CommandId::Save).is_some());
    assert_eq!(s.disabled(CommandId::SaveAs), None);
    edit(&mut s, "must not change the member");
    assert_eq!(text(&s), "member\r\n");
    let export = dir.path().join("export.txt");
    s.dispatch(Command::SaveAs {
        tab,
        path: export.clone(),
    });
    settle(&mut s, |s| {
        !s.state().view(tab).unwrap().save_status().busy()
    });
    assert_eq!(std::fs::read(export).unwrap(), b"member\r\n");
    assert_eq!(std::fs::read(path).unwrap(), container);
}
