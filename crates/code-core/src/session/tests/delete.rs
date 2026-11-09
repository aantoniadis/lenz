//! Deleting explorer entries through the Session command seam: the trash by default and back again, permanent removal
//! only when asked, and no unsaved or deleted work lost or brought back.
use super::*;
use crate::trash::Removal;
use crate::workspace::Explorer;

/// A project with `files`, `open` shown, and a private home trash on the same device.
fn project(d: &Path, files: &[(&str, &str)], open: &str) -> (Session, PathBuf) {
    let root = d.join("proj");
    for (name, text) in files {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let mut s = session(d);
    s.set_trash_home_for_test(d.join("data/Trash"));
    open_project(&mut s, &root, open);
    (s, root)
}

fn delete(s: &mut Session, removal: Removal, from: &str) { s.dispatch(Command::DeleteEntry { removal, from: from.into() }); }

#[test]
fn a_deleted_folder_goes_to_the_trash_closes_its_tabs_and_comes_back() {
    let d = tempfile::tempdir().unwrap();
    let (mut s, root) = project(d.path(), &[("dir/a.txt", "a\n"), ("dir/sub/b.txt", "b\n"), ("other.txt", "o\n")], "dir/a.txt");
    s.dispatch(Command::Pin(s.state().active().unwrap().id()));
    s.dispatch(Command::OpenRetained(root.join("other.txt")));
    settle(&mut s, opened);
    s.dispatch(Command::SetExplorer(Explorer { visible: true, selected: Some("dir".into()), top: None }));
    assert_eq!(s.disabled(CommandId::DeleteExplorerEntry), None);
    assert_eq!(s.disabled(CommandId::RestoreDeleted), Some("Nothing was moved to the trash in this session"));
    let prompt = s.delete_prompt(Removal::Trash, "dir");
    assert!(prompt.starts_with("Delete the folder dir and everything in it? It moves to the trash") && prompt.ends_with("Its open tab closes."), "{prompt}");

    delete(&mut s, Removal::Trash, "dir");
    assert!(!root.join("dir").exists());
    let trash = d.path().join("data/Trash");
    assert_eq!(std::fs::read(trash.join("files/dir/sub/b.txt")).unwrap(), b"b\n");
    assert!(std::fs::read_to_string(trash.join("info/dir.trashinfo")).unwrap().contains("/proj/dir\nDeletionDate="));
    assert_eq!(s.state().tabs.iter().map(|t| t.title()).collect::<Vec<_>>(), ["other.txt"], "the deleted file's tab closed");
    let project = s.state().project().unwrap();
    assert!(!project.has_file("dir/a.txt") && !project.has_file("dir/sub/b.txt") && project.has_file("other.txt"));
    assert!(s.state().notices().iter().any(|m| m == "Moved dir to the trash. Restore Last Deleted Entry puts it back."), "{:?}", s.state().notices());
    assert_eq!(std::fs::read(root.join("other.txt")).unwrap(), b"o\n");

    assert_eq!(s.disabled(CommandId::RestoreDeleted), None);
    s.dispatch(Command::RestoreDeleted);
    assert_eq!(std::fs::read(root.join("dir/sub/b.txt")).unwrap(), b"b\n");
    assert!(!trash.join("info/dir.trashinfo").exists());
    settle(&mut s, |s| s.state().project().is_some_and(|p| p.has_file("dir/a.txt") && !p.refreshing()));
    assert_eq!(s.disabled(CommandId::RestoreDeleted), Some("Nothing was moved to the trash in this session"));
}

#[test]
fn unsaved_work_refuses_the_delete_and_permanent_removal_is_final() {
    let d = tempfile::tempdir().unwrap();
    let (mut s, root) = project(d.path(), &[("a.txt", "a\n"), ("b.txt", "b\n")], "a.txt");
    let tab = s.state().active().unwrap().id();
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..0, text: "draft ".into() }], kind: crate::text::EditKind::Paste });
    for removal in [Removal::Trash, Removal::Permanent] {
        assert_eq!(s.delete_refusal(removal, "a.txt").as_deref(), Some("a.txt has unsaved changes: save or discard them first"));
        delete(&mut s, removal, "a.txt");
        assert_eq!(std::fs::read(root.join("a.txt")).unwrap(), b"a\n", "nothing was deleted");
        assert!(s.state().tab_dirty(tab), "the draft is kept");
    }
    assert!(s.state().notices().iter().any(|m| m.contains("Nothing was deleted")));
    // A clean file, permanently: no trash entry, and no restore offered for it.
    assert!(s.delete_prompt(Removal::Permanent, "b.txt").contains("cannot be restored"));
    delete(&mut s, Removal::Permanent, "b.txt");
    assert!(!root.join("b.txt").exists());
    assert!(!d.path().join("data/Trash/files/b.txt").exists());
    assert_eq!(s.disabled(CommandId::RestoreDeleted), Some("Nothing was moved to the trash in this session"));
    assert!(!s.state().project().unwrap().has_file("b.txt"));
    // Stale: what is already gone is refused, not guessed at.
    delete(&mut s, Removal::Trash, "b.txt");
    assert!(s.state().notices().iter().any(|m| m.contains("b.txt is no longer there")));
}

#[test]
fn deleted_work_is_not_offered_back_by_recovery_and_a_blocked_restore_keeps_the_trash_entry() {
    let d = tempfile::tempdir().unwrap();
    let (mut s, root) = project(d.path(), &[("a.txt", "a\n")], "a.txt");
    let tab = s.state().active().unwrap().id();
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..0, text: "draft ".into() }], kind: crate::text::EditKind::Paste });
    s.flush_recovery_for_test();
    // Discarding the draft closes the question; then the clean file is deleted.
    s.dispatch(Command::CloseTab(tab));
    let prompt = s.close_prompt().unwrap();
    s.dispatch(Command::ChooseClose { ticket: prompt.ticket, buffer: prompt.buffer, choice: crate::close::Choice::Discard });
    delete(&mut s, Removal::Trash, "a.txt");
    assert!(!root.join("a.txt").exists());
    // Something new where it was: the restore refuses and the entry stays in the trash, offered again.
    std::fs::write(root.join("a.txt"), b"new\n").unwrap();
    s.dispatch(Command::RestoreDeleted);
    assert!(s.state().notices().iter().any(|m| m.contains("exists again; nothing was restored")), "{:?}", s.state().notices());
    assert_eq!(std::fs::read(root.join("a.txt")).unwrap(), b"new\n");
    assert_eq!(s.disabled(CommandId::RestoreDeleted), None);
    s.flush_recovery_for_test();
    s.interrupt_for_test();
    let next = session(d.path());
    assert!(next.recovered_drafts().is_empty(), "discarded and deleted work does not come back");
}

#[test]
fn deleting_a_real_entry_protects_its_dirty_alias_but_deleting_the_alias_leaves_the_real_document() {
    for removal in [Removal::Trash, Removal::Permanent] {
        for dirty_alias in [true, false] {
            let d = tempfile::tempdir().unwrap();
            let root = d.path().join("proj");
            std::fs::create_dir(&root).unwrap();
            let real = write(&root, "real.txt", b"saved\n");
            let alias = root.join("alias.txt");
            std::os::unix::fs::symlink(&real, &alias).unwrap();
            let mut s = session(d.path());
            s.set_trash_home_for_test(d.path().join("data/Trash"));
            open_project(&mut s, &root, if dirty_alias { "alias.txt" } else { "real.txt" });
            assert!(s.state().tree_rows().any(|row| &*row.path == "alias.txt"));
            let tab = s.state().active().unwrap().id();
            let expected = s.state().current_source().unwrap().key();
            s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..0, text: "draft ".into() }], kind: crate::text::EditKind::Paste });
            let draft = source_text(s.state().current_source().unwrap());
            if dirty_alias {
                assert!(s.delete_prompt(removal, "real.txt").contains("Its open tab closes"));
                assert!(s.delete_refusal(removal, "real.txt").unwrap().contains("unsaved changes"));
                delete(&mut s, removal, "real.txt");
                assert_eq!(std::fs::read(&real).unwrap(), b"saved\n");
                assert_eq!(source_text(s.state().current_source().unwrap()), draft);
                assert!(s.state().tab_dirty(tab));
                s.dispatch(Command::Undo { tab });
                delete(&mut s, removal, "real.txt");
                assert!(!real.exists());
                assert!(std::fs::symlink_metadata(&alias).is_ok(), "the dangling symlink entry itself remains");
                assert!(s.state().tabs().iter().all(|t| t.id() != tab), "the clean alias tab closes with its deleted target");
            } else {
                assert_eq!(s.delete_refusal(removal, "alias.txt"), None);
                assert!(!s.delete_prompt(removal, "alias.txt").contains("open tab"));
                delete(&mut s, removal, "alias.txt");
                assert_eq!(std::fs::read(&real).unwrap(), b"saved\n");
                assert!(std::fs::symlink_metadata(&alias).is_err());
                assert!(s.state().tab_dirty(tab));
                assert_eq!(source_text(s.state().current_source().unwrap()), draft);
            }
        }
    }
}

#[test]
fn deleting_a_real_entry_checks_saves_and_undecided_recovery_through_aliases() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    std::fs::create_dir(&root).unwrap();
    let real = write(&root, "real.txt", b"saved\n");
    let alias = root.join("alias.txt");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let mut s = session(d.path());
    open_project(&mut s, &root, "alias.txt");
    let buffer = s.state().current_source().unwrap().key().buffer;
    for status in [crate::save::Status::Saving, crate::save::Status::Uncertain("unknown".into())] {
        s.state.sources.buffers.get_mut(&buffer).unwrap().save.status = status;
        assert!(s.delete_refusal(Removal::Permanent, "real.txt").is_some());
        delete(&mut s, Removal::Permanent, "real.txt");
        assert_eq!(std::fs::read(&real).unwrap(), b"saved\n");
    }
    s.state.sources.buffers.get_mut(&buffer).unwrap().save.status = crate::save::Status::Ready;
    s.state.undecided_recovery = vec![alias];
    assert!(s.delete_refusal(Removal::Permanent, "real.txt").unwrap().contains("recovered draft"));
}
