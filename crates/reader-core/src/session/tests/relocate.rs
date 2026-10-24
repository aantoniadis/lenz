//! Renaming and moving explorer entries through the Session command seam: drafts, history, places and the tree follow
//! a confirmed move, and a refused or failed one changes nothing.
use super::*;
use crate::relocate::Kind;
use crate::workspace::Explorer;

fn highlight(s: &mut Session, row: &str) {
    s.dispatch(Command::SetExplorer(Explorer { visible: true, selected: Some(row.to_string()), top: None }));
}

fn relocate(s: &mut Session, kind: Kind, from: &str, input: &str) {
    s.dispatch(Command::RelocateEntry { kind, from: from.into(), input: input.into() });
}

/// A project with `files`, `open` shown: plain text has no declarations to wait for.
fn project(root: &Path, files: &[(&str, &str)], open: &str) -> Session {
    for (name, text) in files {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let mut s = session(root);
    open_project(&mut s, root, open);
    s
}

fn text(s: &Session) -> String { source_text(s.state().current_source().unwrap()) }

fn rows(s: &Session) -> Vec<String> { s.state().tree_rows().map(|r| r.path.to_string()).collect() }

#[test]
fn renaming_an_open_dirty_file_keeps_its_draft_history_and_places_and_saves_under_the_new_name() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    let mut s = project_with(&root, &[("src/a.rs", "fn a() {}\n"), ("src/b.rs", "fn b() {}\n")], "src/a.rs");
    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::ToggleBookmark);
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..0, text: "// draft\n".into() }], kind: crate::text::EditKind::Paste });
    highlight(&mut s, "src/a.rs");
    assert_eq!(s.relocate_target().as_deref(), Some("src/a.rs"));
    assert_eq!(s.disabled(CommandId::RenameExplorerEntry), None);
    assert_eq!(s.relocate_start(Kind::Rename, "src/a.rs"), "a.rs");
    assert_eq!(s.relocate_refusal(Kind::Rename, "src/a.rs", "b.rs").as_deref(), Some("src/b.rs already exists"));
    assert_eq!(s.relocate_refusal(Kind::Rename, "src/a.rs", "c.txt"), None);

    relocate(&mut s, Kind::Rename, "src/a.rs", "c.txt");
    assert!(!root.join("src/a.rs").exists());
    assert_eq!(std::fs::read(root.join("src/c.txt")).unwrap(), b"fn a() {}\n", "the file moves as it is on disk; the draft is not written");
    assert_eq!(std::fs::read(root.join("src/b.rs")).unwrap(), b"fn b() {}\n");
    let t = s.state().active().unwrap();
    assert_eq!((t.id(), t.source().to_owned(), t.title()), (tab, root.join("src/c.txt"), "c.txt".to_string()));
    assert_eq!(s.state().current_source().unwrap().source(), root.join("src/c.txt"));
    assert_eq!(s.state().current_source().unwrap().language(), None, "a .txt name is plain text");
    assert!(s.state().tab_dirty(tab));
    assert_eq!(text(&s), "// draft\nfn a() {}\n");
    assert!(rows(&s).contains(&"src/c.txt".to_string()) && !rows(&s).contains(&"src/a.rs".to_string()), "{:?}", rows(&s));
    assert_eq!(s.state().tree_reveal().map(|(path, _)| path), Some("src/c.txt".into()));
    assert!(s.state().notices().iter().any(|m| m.contains("Renamed src/a.rs to c.txt") && m.contains("References in other files were not changed")), "{:?}", s.state().notices());
    assert!(s.state().places().bookmarks().iter().all(|m| &*m.source == root.join("src/c.txt").as_path()), "bookmarks follow the file");

    // Undo and redo stay with the renamed document: no history step revives the old path.
    s.dispatch(Command::Undo { tab });
    assert_eq!(text(&s), "fn a() {}\n");
    assert!(!s.state().tab_dirty(tab));
    assert_eq!(s.state().current_source().unwrap().source(), root.join("src/c.txt"));
    s.dispatch(Command::Redo { tab });
    assert_eq!(s.state().current_source().unwrap().source(), root.join("src/c.txt"));
    // The confirmed file is the same file: Save writes it under its new name and nothing at the old one.
    assert_eq!(s.disabled(CommandId::Save), None);
    s.dispatch(Command::Save { tab });
    settle(&mut s, |s| !s.state().tab_dirty(tab));
    assert_eq!(std::fs::read(root.join("src/c.txt")).unwrap(), b"// draft\nfn a() {}\n");
    assert!(!root.join("src/a.rs").exists());
}

#[test]
fn moving_a_folder_carries_its_shown_and_released_files_and_their_history() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    let mut s = project(&root, &[("dir/x.txt", "x\n"), ("dir/sub/y.txt", "y\n"), ("dest/keep.txt", "k\n"), ("other.txt", "o\n")], "dir/x.txt");
    let x = s.state().active().unwrap().id();
    s.dispatch(Command::Pin(x));
    s.dispatch(Command::OpenRetained(root.join("dir/sub/y.txt")));
    settle(&mut s, opened);
    let y = s.state().active().unwrap().id();
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab: y, expected, edits: vec![crate::text::TextEdit { range: 0..0, text: "draft ".into() }], kind: crate::text::EditKind::Paste });
    s.dispatch(Command::OpenRetained(root.join("other.txt")));
    settle(&mut s, opened);
    assert!(!s.state().tabs.iter().find(|t| t.id == x).unwrap().loaded(), "x.txt is released");
    highlight(&mut s, "dir");
    assert_eq!(s.relocate_start(Kind::Move, "dir"), "");
    assert_eq!(s.relocate_refusal(Kind::Move, "dir", "dir/sub").as_deref(), Some("A folder cannot move into itself"));
    assert_eq!(s.relocate_refusal(Kind::Move, "dir", "missing").as_deref(), Some("The folder missing does not exist"));

    relocate(&mut s, Kind::Move, "dir", "dest");
    assert!(!root.join("dir").exists());
    assert_eq!(std::fs::read(root.join("dest/dir/sub/y.txt")).unwrap(), b"y\n");
    let source = |id: TabId| s.state().tabs.iter().find(|t| t.id == id).unwrap().source().to_owned();
    assert_eq!((source(x), source(y)), (root.join("dest/dir/x.txt"), root.join("dest/dir/sub/y.txt")));
    assert!(s.state().tab_dirty(y), "the draft moved with its file");
    let tree = rows(&s);
    assert!(tree.contains(&"dest/dir".to_string()) && !tree.iter().any(|r| r.starts_with("dir")), "{tree:?}");
    let project = s.state().project().unwrap();
    assert!(project.has_file("dest/dir/sub/y.txt") && project.has_file("dest/dir/x.txt") && !project.has_file("dir/x.txt"), "the inventory lists the files at once");
    // The released tab reads its file from the new path; Back reaches the draft at its new path.
    s.dispatch(Command::ActivateTab(x));
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source() == root.join("dest/dir/x.txt")) && s.state().loading().is_none());
    assert_eq!(text(&s), "x\n");
    s.dispatch(Command::ActivateTab(y));
    settle(&mut s, opened);
    assert_eq!(text(&s), "draft y\n");
}

#[test]
fn refused_stale_and_colliding_moves_change_nothing() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    let mut s = project(&root, &[("a.txt", "a\n"), ("b.txt", "b\n"), ("dir/inner.txt", "i\n")], "a.txt");
    let tab = s.state().active().unwrap().id();
    // A taken name: the dialog says so, and a command sent anyway moves nothing.
    relocate(&mut s, Kind::Rename, "a.txt", "b.txt");
    assert_eq!((std::fs::read(root.join("a.txt")).unwrap(), std::fs::read(root.join("b.txt")).unwrap()), (b"a\n".to_vec(), b"b\n".to_vec()));
    assert!(s.state().notices().iter().any(|m| m == "Rename: b.txt already exists. Nothing was moved."), "{:?}", s.state().notices());
    assert_eq!(s.state().active().unwrap().source(), root.join("a.txt"));
    // A stale request: the entry was moved elsewhere meanwhile, so no tab is retargeted to a guess.
    std::fs::rename(root.join("b.txt"), root.join("b-moved.txt")).unwrap();
    relocate(&mut s, Kind::Rename, "b.txt", "c.txt");
    assert!(!root.join("c.txt").exists() && root.join("b-moved.txt").exists());
    assert!(s.state().notices().iter().any(|m| m.contains("b.txt is no longer there")));
    // Impossible names never reach the disk.
    for (kind, input) in [(Kind::Rename, "x/y"), (Kind::Rename, ".."), (Kind::Move, "../outside"), (Kind::Move, "/tmp")] {
        assert!(s.relocate_refusal(kind, "a.txt", input).is_some(), "{input}");
    }
    // An undecided recovered draft inside the folder keeps it where its record expects it.
    s.state.undecided_recovery = vec![root.join("dir/inner.txt")];
    assert!(s.relocate_refusal(Kind::Rename, "dir", "renamed").unwrap().contains("recovered draft"));
    s.state.undecided_recovery.clear();
    assert_eq!(s.state().active().unwrap().id(), tab);
    assert!(root.join("a.txt").exists() && root.join("dir/inner.txt").exists());
}

#[test]
fn nothing_to_move_explains_why() {
    let d = tempfile::tempdir().unwrap();
    assert_eq!(session(d.path()).disabled(CommandId::RenameExplorerEntry), Some("No folder is open"));
    let mut s = project(&d.path().join("proj"), &[("a.txt", "a\n")], "a.txt");
    s.dispatch(Command::SetExplorer(Explorer { visible: true, selected: None, top: None }));
    assert_eq!(s.disabled(CommandId::MoveExplorerEntry), Some("Highlight a file or folder in the explorer"));
}
