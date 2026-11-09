use super::*;
use crate::archive::Location;
use crate::text::{EditKind, TextEdit};

fn location(path: &Path, members: &[&str]) -> Location {
    Location::new(path.to_path_buf(), members.iter().map(|name| name.to_string()).collect()).unwrap()
}
fn open_entry(s: &mut Session, path: &Path, members: &[&str]) -> TabId {
    s.dispatch(Command::OpenArchiveEntry(location(path, members)));
    settle(s, |s| s.state().loading().is_none() && s.state().current_source().is_some_and(|source|
        source.source_meta().archive().is_some_and(|location| location.members == members)));
    s.state().active().unwrap().id()
}
fn replace(s: &mut Session, tab: TabId, text: &str) {
    let source = s.state().view(tab).unwrap().snapshot();
    let (expected, len) = (source.key(), source.len_bytes());
    s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..len, text: text.into() }], kind: EditKind::Replace });
}
fn idle(s: &Session, tab: TabId) -> bool {
    let buffer = s.state.tabs.iter().find(|item| item.id == tab).unwrap().buffer.unwrap();
    !s.state.sources.buffers[&buffer].save.status.busy()
}
fn member(path: &Path, chain: &[&str]) -> Vec<u8> {
    let parent = location(path, &chain[..chain.len()-1]);
    let index = crate::archive::open_index(&parent, &|| false).unwrap();
    let entry = crate::archive::member_named(&index, chain.last().unwrap()).unwrap();
    index.read_member(entry.id, &|| false).unwrap()
}
fn hold(s: &mut Session) -> Vec<std::sync::mpsc::Sender<()>> {
    let (started, entered) = std::sync::mpsc::channel();
    let gates = (0..crate::jobs::MAX_JOBS).map(|_| {
        let (release, wait) = std::sync::mpsc::channel(); let started = started.clone();
        s.jobs.submit(Priority::Visible, move || { started.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); });
        release
    }).collect::<Vec<_>>();
    for _ in &gates { entered.recv_timeout(Duration::from_secs(5)).unwrap(); }
    gates
}

#[test]
fn archive_member_save_serializes_siblings_and_retains_edits_newer_than_the_capture() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("bundle.zip");
    let binary = b"\x00\xff\x01sibling";
    std::fs::write(&path, lenz_archive::testing::zip(&[("a.txt", b"old a\r\n"), ("b.txt", b"old b\n"), ("binary.dat", binary)])).unwrap();
    let mut s = session(d.path());
    let a = open_entry(&mut s, &path, &["a.txt"]);
    let a_buffer = s.state().current_source().unwrap().key().buffer;
    assert_eq!(s.state().view(a).unwrap().input_newline(), "\r\n");
    replace(&mut s, a, "saved a\r\n");
    let b = open_entry(&mut s, &path, &["b.txt"]);
    assert_ne!(a_buffer, s.state().current_source().unwrap().key().buffer);
    replace(&mut s, b, "captured b\n");
    s.dispatch(Command::ActivateTab(a));
    let gates = hold(&mut s);
    s.dispatch(Command::Save { tab: a });
    s.dispatch(Command::ActivateTab(b));
    s.dispatch(Command::Save { tab: b });
    replace(&mut s, b, "newer b\n");
    for gate in gates { gate.send(()).unwrap(); }
    settle(&mut s, |s| idle(s, a) && idle(s, b));
    assert_eq!(member(&path, &["a.txt"]), b"saved a\r\n");
    assert_eq!(member(&path, &["b.txt"]), b"captured b\n");
    assert_eq!(member(&path, &["binary.dat"]), binary);
    assert!(!s.state().tab_dirty(a));
    assert!(s.state().tab_dirty(b));
    assert_eq!(source_text(s.state().current_source().unwrap()), "newer b\n");
    s.dispatch(Command::Save { tab: b });
    settle(&mut s, |s| idle(s, b));
    assert_eq!(member(&path, &["b.txt"]), b"newer b\n");
    assert!(!s.state().tab_dirty(b));
    s.dispatch(Command::Undo { tab: b });
    assert_eq!(source_text(s.state().current_source().unwrap()), "captured b\n");
    assert!(s.state().tab_dirty(b));
}

#[test]
fn archive_external_change_requires_fresh_consent_and_preserves_external_siblings() {
    use crate::reconcile::Choice;
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("bundle.zip");
    std::fs::write(&path, lenz_archive::testing::zip(&[("a.txt", b"old a\n"), ("b.txt", b"old b\n")])).unwrap();
    let mut s = session(d.path());
    let a = open_entry(&mut s, &path, &["a.txt"]);
    replace(&mut s, a, "my draft\n");
    let external = lenz_archive::testing::zip(&[("a.txt", b"their a\n"), ("b.txt", b"external sibling\n")]);
    std::fs::write(&path, &external).unwrap();
    s.dispatch(Command::Save { tab: a });
    settle(&mut s, |s| idle(s, a));
    assert!(matches!(s.state().view(a).unwrap().save_status(), crate::save::Status::Rejected(_)));
    assert_eq!(std::fs::read(&path).unwrap(), external);
    assert_eq!(source_text(s.state().current_source().unwrap()), "my draft\n");
    s.dispatch(Command::ResolveExternalChange);
    settle(&mut s, |s| s.reconcile_prompt().is_some_and(|prompt| prompt.disk.is_some()));
    let ticket = s.reconcile_prompt().unwrap().ticket;
    s.dispatch(Command::ChooseReconcile { ticket, choice: Choice::KeepDraft });
    assert_eq!(std::fs::read(&path).unwrap(), external);
    assert!(s.state().tab_dirty(a));
    s.dispatch(Command::ResolveExternalChange);
    settle(&mut s, |s| s.reconcile_prompt().is_some_and(|prompt| prompt.disk.is_some()));
    let ticket = s.reconcile_prompt().unwrap().ticket;
    s.dispatch(Command::ChooseReconcile { ticket, choice: Choice::SaveDraft });
    settle(&mut s, |s| !s.state().tab_dirty(a) && idle(s, a));
    assert_eq!(member(&path, &["a.txt"]), b"my draft\n");
    assert_eq!(member(&path, &["b.txt"]), b"external sibling\n");
}

#[test]
fn nested_archive_member_save_and_save_as_export_keep_both_containers_intact() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("outer.zip");
    let inner = lenz_archive::testing::tar(lenz_archive::Compression::Zstd, &[("src/a.rs", b"fn old() {}\n"), ("binary", b"\0\xff")]);
    std::fs::write(&path, lenz_archive::testing::zip(&[("inner.tar.zst", &inner), ("keep.txt", b"keep outer\n")])).unwrap();
    let mut s = session(d.path());
    let tab = open_entry(&mut s, &path, &["inner.tar.zst", "src/a.rs"]);
    assert!(s.state().active().unwrap().title().contains("a.rs") && s.state().active().unwrap().title().contains("inner.tar.zst"));
    assert!(s.state().notices().iter().any(|notice| notice.contains("outer.zip!/inner.tar.zst!/src/a.rs")));
    replace(&mut s, tab, "fn changed() {}\n");
    s.dispatch(Command::Save { tab });
    settle(&mut s, |s| idle(s, tab));
    assert_eq!(member(&path, &["inner.tar.zst", "src/a.rs"]), b"fn changed() {}\n");
    assert_eq!(member(&path, &["inner.tar.zst", "binary"]), b"\0\xff");
    assert_eq!(member(&path, &["keep.txt"]), b"keep outer\n");
    let before = std::fs::read(&path).unwrap();
    s.dispatch(Command::SaveAs { tab, path: path.clone() });
    assert!(s.save_as_prompt().is_none(), "export must not replace the archive containing its own source");
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let export = d.path().join("export.rs");
    replace(&mut s, tab, "fn exported() {}\n");
    s.dispatch(Command::SaveAs { tab, path: export.clone() });
    settle(&mut s, |s| s.state().active().unwrap().source() == export && idle(s, tab));
    assert!(s.state().current_source().unwrap().source_meta().archive().is_none());
    replace(&mut s, tab, "fn ordinary_save() {}\n");
    s.dispatch(Command::Save { tab });
    settle(&mut s, |s| idle(s, tab));
    assert_eq!(std::fs::read(&export).unwrap(), b"fn ordinary_save() {}\n");
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn dirty_archive_alias_protects_its_container_and_follows_a_real_container_rename() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project"); std::fs::create_dir(&root).unwrap();
    let path = root.join("bundle.zip");
    let alias = root.join("alias.zip");
    let other = write(&root, "other.txt", b"outside\n");
    std::fs::write(&path, lenz_archive::testing::zip(&[("src/main.rs", b"fn old() {}\n"), ("b.txt", b"sibling\n")])).unwrap();
    std::os::unix::fs::symlink(&path, &alias).unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone())); settle(&mut s, scanned);
    s.dispatch(Command::OpenRetained(alias.clone()));
    let original_b = location(&path, &["b.txt"]).to_uri();
    settle(&mut s, |s| s.state().tree_rows().any(|row| row.path.as_ref() == original_b.to_str().unwrap()));
    let tab = open_entry(&mut s, &alias, &["src/main.rs"]);
    assert_eq!(s.state().current_source().unwrap().language(), Some(Language::Rust));
    replace(&mut s, tab, "fn draft() {}\n");
    s.dispatch(Command::OpenRetained(other.clone())); settle(&mut s, opened);
    s.dispatch(Command::Back); settle(&mut s, |s| s.state().active().is_some_and(|active| active.id() == tab));
    assert!(s.delete_refusal(crate::trash::Removal::Permanent, "bundle.zip").unwrap().contains("unsaved"));
    s.dispatch(Command::RelocateEntry { kind: crate::relocate::Kind::Rename, from: "bundle.zip".into(), input: "renamed.zip".into() });
    let renamed = root.join("renamed.zip");
    assert!(s.state().tab_dirty(tab));
    assert_eq!(s.state().current_source().unwrap().language(), Some(Language::Rust));
    assert_eq!(s.state().archive_tree().roots(), &[location(&renamed, &[])]);
    s.dispatch(Command::Forward); settle(&mut s, |s| s.state().active().is_some_and(|tab| tab.source() == other));
    s.dispatch(Command::Back); settle(&mut s, |s| s.state().active().is_some_and(|active| active.id() == tab));
    assert_eq!(source_text(s.state().current_source().unwrap()), "fn draft() {}\n");
    let sibling = location(&renamed, &["b.txt"]).to_uri();
    settle(&mut s, |s| s.state().tree_rows().any(|row| row.path.as_ref() == sibling.to_str().unwrap()));
    s.dispatch(Command::ActivateTree { path: sibling.to_string_lossy().as_ref().into(), retain: true });
    settle(&mut s, |s| s.state().current_source().is_some_and(|source| source.source_meta().archive().is_some_and(|location| location.member_name() == Some("b.txt"))));
    assert_eq!(source_text(s.state().current_source().unwrap()), "sibling\n");
    s.dispatch(Command::ActivateTab(tab));
    assert_eq!(s.state().current_source().unwrap().language(), Some(Language::Rust));
    s.dispatch(Command::Save { tab });
    settle(&mut s, |s| idle(s, tab));
    assert_eq!(member(&renamed, &["src/main.rs"]), b"fn draft() {}\n");
}

#[test]
fn uncertain_archive_save_can_be_rechecked_from_another_dirty_member_after_close() {
    use lenz_content::write::readiness_fault::{at_path, Fault};
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("bundle.zip");
    std::fs::write(&path, lenz_archive::testing::zip(&[("a.txt", b"old a\n"), ("b.txt", b"old b\n")])).unwrap();
    let mut s = session(d.path());
    let a = open_entry(&mut s, &path, &["a.txt"]);
    replace(&mut s, a, "saved a\n");
    let b = open_entry(&mut s, &path, &["b.txt"]);
    replace(&mut s, b, "draft b\n");
    s.dispatch(Command::ActivateTab(a));
    let fault = at_path(&path, Fault::AfterReplace);
    s.dispatch(Command::Save { tab: a });
    settle(&mut s, |s| idle(s, a));
    drop(fault);
    assert!(matches!(s.state().view(a).unwrap().save_status(), crate::save::Status::Uncertain(_)));
    s.dispatch(Command::CloseTab(a));
    let prompt = s.close_prompt().unwrap();
    assert!(prompt.discard_disabled.is_none());
    s.dispatch(Command::ChooseClose { ticket: prompt.ticket, buffer: prompt.buffer, choice: crate::close::Choice::Discard });
    s.dispatch(Command::ActivateTab(b));
    assert!(s.disabled(CommandId::Save).is_some());
    assert_eq!(s.disabled(CommandId::RecheckSave), None, "a sibling must expose the container recheck route");
    s.dispatch(Command::RecheckSave { tab: b });
    settle(&mut s, |s| idle(s, b));
    assert!(matches!(s.state().view(b).unwrap().save_status(), crate::save::Status::Checked(crate::save::DiskMatch::Baseline)));
    s.dispatch(Command::ConfirmRecheckedSave { tab: b });
    assert!(s.state().tab_dirty(b));
    s.dispatch(Command::Save { tab: b });
    settle(&mut s, |s| idle(s, b));
    assert_eq!(member(&path, &["a.txt"]), b"saved a\n");
    assert_eq!(member(&path, &["b.txt"]), b"draft b\n");
}

#[test]
fn an_archive_name_cannot_make_a_device_an_unbounded_member_source() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("zero.zip");
    std::os::unix::fs::symlink("/dev/zero", &path).unwrap();
    assert!(crate::archive::read_location(&location(&path, &["a.txt"]), 1).is_err());
}

#[test]
fn a_late_workspace_does_not_replace_a_new_archive_browser_without_member_tabs() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project"); std::fs::create_dir(&root).unwrap();
    let old = root.join("old.zip"); let chosen = root.join("chosen.zip");
    for path in [&old, &chosen] { std::fs::write(path, lenz_archive::testing::zip(&[("a.txt", b"a\n")])).unwrap(); }
    let mut prior = session(d.path());
    prior.dispatch(Command::OpenRoot(root.clone())); settle(&mut prior, scanned);
    prior.dispatch(Command::OpenRetained(old));
    assert_eq!(prior.state().archive_tree().roots().len(), 1);
    drop(prior);
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root));
    let completion = loop {
        let completion = s.completions().recv_blocking_timeout().unwrap();
        if matches!(completion, Completion::PlacesRead { .. }) { break completion; }
        s.dispatch(completion);
    };
    let selected = location(&chosen, &[]);
    s.dispatch(Command::OpenArchiveEntry(selected.clone()));
    assert!(s.state().tabs().is_empty());
    s.dispatch(completion);
    assert_eq!(s.state().archive_tree().roots(), &[selected]);
    assert!(s.state().tabs().is_empty());
}

#[test]
fn archive_member_and_container_rows_show_dirty_state_without_requiring_a_project() {
    for with_project in [false, true] {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("project"); std::fs::create_dir(&root).unwrap();
        let path = root.join("bundle.zip");
        std::fs::write(&path, lenz_archive::testing::zip(&[("a.txt", b"a\n"), ("b.txt", b"b\n")])).unwrap();
        let mut s = session(d.path());
        if with_project { s.dispatch(Command::OpenRoot(root)); settle(&mut s, scanned); }
        s.dispatch(Command::OpenRetained(path.clone()));
        settle(&mut s, |s| s.state().tree_rows().any(|row| row.name.as_ref() == "a.txt"));
        let tab = open_entry(&mut s, &path, &["a.txt"]);
        replace(&mut s, tab, "draft\n");
        let dirty = |s: &Session, name: &str| s.state().tree_rows().any(|row| row.name.as_ref() == name && s.state().row_dirty(row));
        assert!(dirty(&s, "a.txt"));
        assert!(!dirty(&s, "b.txt"));
        assert!(dirty(&s, "bundle.zip"));
        s.dispatch(Command::Save { tab });
        settle(&mut s, |s| idle(s, tab) && s.state().tree_rows().any(|row| row.name.as_ref() == "a.txt"));
        assert!(!dirty(&s, "a.txt") && !dirty(&s, "bundle.zip"));
        assert_eq!(member(&path, &["a.txt"]), b"draft\n");
    }
}
