use super::*;

#[test]
fn annotation_width_is_clamped_and_shared_without_changing_document_bytes() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "note.txt", b"unchanged\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path.clone()), "note.txt");
    s.dispatch(Command::SetAnnotate(true));
    let tab = s.state().active().unwrap().id();
    assert_eq!(s.state().annotation_cols(s.state().active().unwrap()), 23);
    for (requested, expected) in [(40, 40), (0, 12), (usize::MAX, 80)] {
        s.dispatch(Command::ResizeAnnotations { width: requested, commit: false });
        assert_eq!(s.state().annotation_cols(s.state().active().unwrap()), expected);
        assert_eq!(s.state().layout().annotation_cols(), expected);
    }
    s.dispatch(Command::Split(Split::Columns));
    assert!(s.state().tabs().iter().filter(|t| t.annotation_cols() != 0).all(|t| s.state().annotation_cols(t) == 80));
    assert!(s.state().view(tab).unwrap().snapshot().copy_range(0..10).is_some());
    assert_eq!(std::fs::read(path).unwrap(), b"unchanged\n");
}

#[test]
fn a_wider_annotation_reveals_the_author_and_persists_only_when_committed() {
    use lenz_content::git::testing::{commit_all, git, repo, write as put};
    let r = repo();
    put(r.path(), "note.txt", "text\n"); commit_all(r.path(), "Initial");
    git(r.path(), &["commit", "--amend", "--no-edit", "--author=A deliberately long author name <author@example.invalid>"]);
    let mut s = session(r.path());
    open(&mut s, Command::OpenRetained(r.path().join("note.txt")), "note.txt");
    s.dispatch(Command::SetAnnotate(true));
    let tab = s.state().active().unwrap().id();
    settle(&mut s, |s| s.annotation(tab, 0).is_some());
    assert!(!s.annotation(tab, 0).unwrap().label.ends_with("A deliberately long author name"));
    s.dispatch(Command::ResizeAnnotations { width: 50, commit: false });
    assert!(s.annotation(tab, 0).unwrap().label.ends_with("A deliberately long author name"));
    s.dispatch(Command::ResizeAnnotations { width: 50, commit: true });
    let settings = s.settings_file.clone();
    settle(&mut s, |_| std::fs::read(&settings).ok().and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok()).is_some_and(|value| value["layout"]["annotation_width"] == 50));
    assert_eq!(session(r.path()).state().layout().annotation_cols(), 50);
}

#[test]
fn annotation_resize_preserves_a_saved_comparison() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "note.txt", b"text\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path.clone()), "note.txt");
    s.dispatch(Command::SetAnnotate(true));
    s.dispatch(Command::CompareSaved);
    assert!(s.state().saved_comparison().is_some());
    let tabs = s.state().tabs().iter().map(|t| t.id()).collect::<Vec<_>>();
    s.dispatch(Command::ResizeAnnotations { width: 35, commit: false });
    assert!(s.state().saved_comparison().is_some());
    assert_eq!(s.state().tabs().iter().map(|t| t.id()).collect::<Vec<_>>(), tabs);
    assert_eq!(std::fs::read(path).unwrap(), b"text\n");
}

#[test]
fn live_annotation_resize_does_not_checkpoint_the_workspace() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "note.txt", b"text\n");
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(d.path().into()));
    open(&mut s, Command::OpenRetained(path), "note.txt");
    settle(&mut s, |s| s.state().project().is_some());
    s.dispatch(Command::Pin(s.state().active().unwrap().id()));
    let checkpoint = s.state.places.workspace.clone();
    assert!(checkpoint.is_some());
    s.dispatch(Command::ResizeAnnotations { width: 35, commit: false });
    assert_eq!(s.state.places.workspace, checkpoint);
    s.dispatch(Command::ResizeAnnotations { width: 35, commit: true });
    assert_eq!(s.state.places.workspace.as_ref().unwrap().layout.annotation_cols(), 35);
}
