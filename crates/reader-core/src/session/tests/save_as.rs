use super::*;
use crate::text::{EditKind, TextEdit};

fn type_text(s: &mut Session, tab: TabId, text: &str) {
    let current = s.state().view(tab).unwrap().snapshot().clone();
    let end = current.len_bytes();
    s.dispatch(Command::Edit { tab, expected: current.key(), edits: vec![TextEdit { range: end..end, text: text.into() }], kind: EditKind::Paste });
}

fn new_document(s: &mut Session) -> TabId {
    s.dispatch(Command::NewDocument);
    let tab = s.state().active().unwrap();
    assert!(tab.untitled());
    tab.id()
}

#[test]
fn untitled_document_keeps_identity_history_and_close_protection_without_a_path() {
    let d = tempfile::tempdir().unwrap();
    let other = write(d.path(), "other.txt", b"other\n");
    let mut s = session(d.path());
    let tab = new_document(&mut s);
    assert_eq!(s.state().active().unwrap().title(), "Untitled-1");
    assert_eq!(source_text(s.state().current_source().unwrap()), "");
    assert!(!s.state().tab_dirty(tab), "an empty untitled document has nothing to lose");
    assert!(s.disabled(CommandId::Reload).is_some());
    assert!(s.disabled(CommandId::SaveAs).is_none());
    for id in [CommandId::CopyPath, CommandId::CopyPathLine, CommandId::CompareSaved, CommandId::OpenInEditor, CommandId::AnnotateBlame, CommandId::ToggleBookmark] {
        assert_eq!(s.disabled(id), Some(UNTITLED_NO_FILE), "{id:?} needs a disk file");
    }
    assert!(s.dispatch(Command::CopyPath { line: false }).copy.is_none());
    let buffer = s.state().current_source().unwrap().key().buffer;
    type_text(&mut s, tab, "draft λ\n");
    assert!(s.state().tab_dirty(tab));
    assert_eq!(s.action(CommandId::Save), crate::actions::CatalogAction::Interface(crate::actions::InterfaceAction::SaveAs), "Save chooses a file for an untitled document");
    s.dispatch(Command::Save { tab });
    assert!(s.state.messages.iter().any(|m| m == "Use Save As to choose where to save this document"));
    // A second untitled document has its own identity; switching files never reads or evicts the draft.
    let second = new_document(&mut s);
    assert_eq!(s.state().active().unwrap().title(), "Untitled-2");
    assert_ne!(s.state().current_source().unwrap().key().buffer, buffer);
    s.cache = InactiveCache::new(0);
    open(&mut s, Command::OpenRetained(other), "other.txt");
    s.dispatch(Command::ActivateTab(tab));
    assert_eq!(s.state().current_source().unwrap().key().buffer, buffer);
    assert_eq!(source_text(s.state().current_source().unwrap()), "draft λ\n");
    assert!(active_view(&s).can_undo());
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "");
    s.dispatch(Command::Redo { tab });
    // Closing the dirty draft asks first; Save points at Save As, Cancel keeps it, Discard removes it.
    s.dispatch(Command::CloseTab(tab));
    let prompt = s.close_prompt().expect("dirty untitled close is protected");
    assert_eq!(prompt.path, Path::new("Untitled-1"));
    assert_eq!(prompt.save_disabled, Some("Use Save As to choose where to save this document"));
    s.dispatch(Command::ChooseClose { ticket: prompt.ticket, buffer: prompt.buffer, choice: crate::close::Choice::Cancel });
    assert!(s.state().tabs().iter().any(|t| t.id() == tab));
    assert_eq!(source_text(s.state().view(tab).unwrap().snapshot()), "draft λ\n");
    s.dispatch(Command::CloseTab(tab));
    let prompt = s.close_prompt().unwrap();
    s.dispatch(Command::ChooseClose { ticket: prompt.ticket, buffer: prompt.buffer, choice: crate::close::Choice::Discard });
    settle(&mut s, |s| !s.state().tabs().iter().any(|t| t.id() == tab));
    // The clean untitled document closes without a prompt; nothing touched the disk.
    s.dispatch(Command::CloseTab(second));
    assert!(s.close_prompt().is_none());
    assert!(!s.state().tabs().iter().any(|t| t.id() == second));
    assert!(!std::fs::read_dir(d.path()).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().contains("ntitled")));
}

#[test]
fn save_as_moves_every_view_only_after_the_new_file_is_confirmed() {
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    let tab = new_document(&mut s);
    type_text(&mut s, tab, "fn main() {}\r\n");
    s.dispatch(Command::Split(Split::Columns));
    let twin = s.state().active().unwrap().id();
    assert_ne!(twin, tab);
    assert_eq!(s.state().view(twin).unwrap().snapshot().key().buffer, s.state().view(tab).unwrap().snapshot().key().buffer, "both panes share the untitled draft");
    let target = d.path().join("main.rs");
    s.dispatch(Command::SaveAs { tab: twin, path: target.clone() });
    assert!(s.state().tabs().iter().all(|t| t.untitled()), "nothing moves before the write is confirmed");
    settle(&mut s, |s| !s.state().tab_dirty(tab));
    assert_eq!(std::fs::read(&target).unwrap(), b"fn main() {}\r\n", "exact text, UTF-8 without a BOM");
    for id in [tab, twin] {
        let t = s.state().tabs().iter().find(|t| t.id() == id).unwrap();
        assert!(!t.untitled());
        assert_eq!(t.source(), target);
        assert_eq!(t.title(), "main.rs");
        assert_eq!(s.state().view(id).unwrap().snapshot().language(), Some(lenz_content::highlight::Language::Rust));
    }
    assert!(s.state().view(tab).unwrap().can_undo(), "history is retained");
    assert!(s.disabled(CommandId::Reload).is_none());
    // Later edits are dirty against the new file and plain Save writes it.
    type_text(&mut s, tab, "// more\n");
    assert!(s.disabled(CommandId::Save).is_none());
    s.dispatch(Command::Save { tab });
    settle(&mut s, |s| !s.state().tab_dirty(tab));
    assert_eq!(std::fs::read(&target).unwrap(), b"fn main() {}\r\n// more\n");
    s.dispatch(Command::Undo { tab });
    assert!(s.state().tab_dirty(tab));
}

#[test]
fn save_as_of_a_file_keeps_its_encoding_and_leaves_the_original_untouched() {
    let d = tempfile::tempdir().unwrap();
    let original = write(d.path(), "legacy.txt", b"\xff\xfea\x00\r\x00\n\x00");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(original.clone()), "legacy.txt");
    let tab = s.state().active().unwrap().id();
    type_text(&mut s, tab, "λ");
    let copy = d.path().join("copy.txt");
    s.dispatch(Command::SaveAs { tab, path: copy.clone() });
    settle(&mut s, |s| !s.state().tab_dirty(tab));
    assert_eq!(std::fs::read(&copy).unwrap(), b"\xff\xfea\x00\r\x00\n\x00\xbb\x03");
    assert_eq!(std::fs::read(&original).unwrap(), b"\xff\xfea\x00\r\x00\n\x00");
    assert_eq!(s.state().active().unwrap().source(), copy);
    // The original opens as its own, separate document again.
    open(&mut s, Command::OpenRetained(original.clone()), "legacy.txt");
    assert_eq!(source_text(s.state().current_source().unwrap()), "a\r\n");
    assert_ne!(s.state().active().unwrap().id(), tab);
}

fn messages(s: &Session) -> String { s.state().messages.join("\n") }

#[test]
fn save_as_replaces_an_existing_file_only_after_an_explicit_current_decision() {
    use crate::save::ReplaceChoice;
    let d = tempfile::tempdir().unwrap();
    let existing = write(d.path(), "existing.txt", b"keep me\n");
    let mut s = session(d.path());
    let tab = new_document(&mut s);
    type_text(&mut s, tab, "new text\n");
    s.dispatch(Command::SaveAs { tab, path: existing.clone() });
    let prompt = s.save_as_prompt().expect("an existing target asks first");
    assert_eq!(prompt.path, existing);
    assert!(prompt.text().contains("already exists"));
    assert!(s.disabled(CommandId::SaveAs).is_some());
    s.dispatch(Command::ChooseReplace { ticket: prompt.ticket, choice: ReplaceChoice::Cancel });
    assert!(s.save_as_prompt().is_none());
    assert_eq!(std::fs::read(&existing).unwrap(), b"keep me\n");
    assert!(s.state().active().unwrap().untitled() && s.state().tab_dirty(tab));
    // An edit retires the decision: Replace then writes nothing.
    s.dispatch(Command::SaveAs { tab, path: existing.clone() });
    let stale = s.save_as_prompt().unwrap();
    type_text(&mut s, tab, "more\n");
    assert!(s.save_as_prompt().is_none());
    s.dispatch(Command::ChooseReplace { ticket: stale.ticket, choice: ReplaceChoice::Replace });
    assert_eq!(std::fs::read(&existing).unwrap(), b"keep me\n");
    // A different file appearing at the path after the decision is refused.
    s.dispatch(Command::SaveAs { tab, path: existing.clone() });
    let prompt = s.save_as_prompt().unwrap();
    std::fs::remove_file(&existing).unwrap();
    std::fs::write(&existing, b"someone else\n").unwrap();
    s.dispatch(Command::ChooseReplace { ticket: prompt.ticket, choice: ReplaceChoice::Replace });
    settle(&mut s, |s| !s.state().view(tab).unwrap().save_status().busy());
    assert_eq!(std::fs::read(&existing).unwrap(), b"someone else\n");
    assert!(messages(&s).contains("Save As rejected"), "{}", messages(&s));
    assert!(s.state().active().unwrap().untitled() && s.state().tab_dirty(tab));
    // The confirmed replacement.
    s.dispatch(Command::SaveAs { tab, path: existing.clone() });
    let prompt = s.save_as_prompt().unwrap();
    s.dispatch(Command::ChooseReplace { ticket: prompt.ticket, choice: ReplaceChoice::Replace });
    settle(&mut s, |s| !s.state().tab_dirty(tab));
    assert_eq!(std::fs::read(&existing).unwrap(), b"new text\nmore\n");
    assert_eq!(s.state().active().unwrap().source(), existing);
}

#[test]
fn save_as_refuses_invalid_or_occupied_destinations_before_writing() {
    let d = tempfile::tempdir().unwrap();
    let open_file = write(d.path(), "open.txt", b"open\n");
    std::fs::create_dir(d.path().join("folder")).unwrap();
    std::os::unix::fs::symlink(d.path().join("nowhere"), d.path().join("dangling")).unwrap();
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(open_file.clone()), "open.txt");
    let file_tab = s.state().active().unwrap().id();
    let tab = new_document(&mut s);
    type_text(&mut s, tab, "x");
    for (path, why) in [
        (PathBuf::from("relative.txt"), "absolute"),
        (d.path().join("missing/new.txt"), "folder does not exist"),
        (d.path().join("folder"), "is a folder"),
        (d.path().join("dangling"), "broken symbolic link"),
        (open_file.clone(), "open in another tab"),
    ] {
        s.state.messages.clear();
        s.dispatch(Command::SaveAs { tab, path: path.clone() });
        assert!(s.save_as_prompt().is_none());
        assert!(messages(&s).contains(why), "{path:?}: {}", messages(&s));
        assert!(s.state().active().unwrap().untitled());
    }
    open(&mut s, Command::OpenRetained(open_file.clone()), "open.txt");
    assert_eq!(s.state().active().unwrap().id(), file_tab);
    s.state.messages.clear();
    s.dispatch(Command::SaveAs { tab: file_tab, path: open_file.clone() });
    assert!(messages(&s).contains("use Save"), "{}", messages(&s));
    assert_eq!(std::fs::read(&open_file).unwrap(), b"open\n");
    assert!(!d.path().join("nowhere").exists() && !d.path().join("relative.txt").exists());
}

#[test]
fn save_as_failures_keep_the_original_document_and_never_clobber_a_new_entry() {
    use lenz_content::write::readiness_fault::{at_path, Fault};
    let d = tempfile::tempdir().unwrap();
    let original = write(d.path(), "original.txt", b"original\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(original.clone()), "original.txt");
    let tab = s.state().active().unwrap().id();
    type_text(&mut s, tab, "draft\n");
    for (fault, exists) in [(Fault::BeforeReplace, false), (Fault::AfterReplace, true)] {
        let target = d.path().join(format!("copy-{exists}.txt"));
        let guard = at_path(&target, fault);
        s.dispatch(Command::SaveAs { tab, path: target.clone() });
        settle(&mut s, |s| !s.state().view(tab).unwrap().save_status().busy());
        drop(guard);
        assert_eq!(target.exists(), exists);
        assert!(messages(&s).contains(if exists { "uncertain" } else { "rejected" }), "{}", messages(&s));
        assert_eq!(s.state().active().unwrap().source(), original, "only a confirmed write moves the document");
        assert!(s.state().tab_dirty(tab) && s.state().view(tab).unwrap().can_undo());
        assert_eq!(std::fs::read(&original).unwrap(), b"original\n");
        assert!(s.disabled(CommandId::Save).is_none(), "the original file's Save state is unaffected");
    }
    // A file created between validation and the write is never replaced.
    let raced = d.path().join("raced.txt");
    let (started, entered) = std::sync::mpsc::channel();
    let gates: Vec<_> = (0..crate::jobs::MAX_JOBS).map(|_| {
        let (release, wait) = std::sync::mpsc::channel(); let started = started.clone();
        s.jobs.submit(Priority::Visible, move || { started.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); }); release
    }).collect();
    for _ in &gates { entered.recv_timeout(Duration::from_secs(5)).unwrap(); }
    s.dispatch(Command::SaveAs { tab, path: raced.clone() });
    std::fs::write(&raced, b"theirs\n").unwrap();
    for gate in gates { gate.send(()).unwrap(); }
    settle(&mut s, |s| !s.state().view(tab).unwrap().save_status().busy());
    assert_eq!(std::fs::read(&raced).unwrap(), b"theirs\n");
    assert_eq!(s.state().active().unwrap().source(), original);
    assert!(s.state().tab_dirty(tab));
}
