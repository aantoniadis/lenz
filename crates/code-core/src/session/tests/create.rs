use super::*;
use crate::create::Kind;
use crate::workspace::Explorer;

/// Highlight `row` in the explorer, as an adapter would.
fn highlight(s: &mut Session, row: Option<&str>) {
    s.dispatch(Command::SetExplorer(Explorer { visible: true, selected: row.map(str::to_string), top: None }));
}

#[test]
fn a_new_file_is_created_in_the_highlighted_folder_and_opens_as_that_file() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    let mut s = project_with(&root, &[("src/a.rs", "fn a() {}\n")], "src/a.rs");
    highlight(&mut s, Some("src"));
    assert_eq!(s.create_target().as_deref(), Some("src"), "the highlighted folder holds the new file");
    assert_eq!(s.create_refusal("src", "new.rs"), None);
    assert_eq!(s.disabled(CommandId::NewExplorerFile), None);

    s.dispatch(Command::CreateEntry { kind: Kind::File, dir: "src".into(), name: "new.rs".into() });
    assert_eq!(std::fs::read(root.join("src/new.rs")).unwrap(), b"", "the file exists before it is opened");
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source() == root.join("src/new.rs")));
    let tab = s.state().active().unwrap().id();
    {
        let t = s.state().active().unwrap();
        assert_eq!((t.title(), t.source().to_owned()), ("new.rs".to_string(), root.join("src/new.rs")));
        assert!(!t.untitled() && t.is_file(), "a created file is a real file, not an untitled draft");
    }
    assert!(!s.state().tab_dirty(tab));
    assert_eq!(source_text(s.state().current_source().unwrap()), "");
    assert_eq!(s.state().tree_reveal().map(|(path, _)| path), Some("src/new.rs".into()));
    assert!(s.state().tree_rows().any(|r| &*r.path == "src/new.rs"), "the explorer shows it without waiting for a rescan");

    // It is an ordinary file: an edit is dirty and undoable, and Save writes those exact bytes.
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..0, text: "fn b() {}\n".into() }], kind: crate::text::EditKind::Paste });
    assert!(s.state().tab_dirty(tab));
    s.dispatch(Command::Save { tab });
    settle(&mut s, |s| !s.state().tab_dirty(tab));
    assert_eq!(std::fs::read(root.join("src/new.rs")).unwrap(), b"fn b() {}\n");
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "");
    assert!(s.state().tab_dirty(tab), "undo after Save does not become a clean boundary");
    // A created file is not a recovered draft.
    assert!(s.recovered_drafts().is_empty());
}

#[test]
fn a_new_folder_is_created_empty_and_shown_open_in_the_explorer() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    let mut s = project_with(&root, &[("src/a.rs", "fn a() {}\n")], "src/a.rs");
    highlight(&mut s, Some("src"));
    s.dispatch(Command::CreateEntry { kind: Kind::Folder, dir: "src".into(), name: "ui".into() });
    assert!(root.join("src/ui").is_dir());
    assert!(std::fs::read_dir(root.join("src/ui")).unwrap().next().is_none(), "nothing else is made");
    let rows: Vec<(String, Option<bool>)> = s.state().tree_rows().map(|r| (r.path.to_string(), r.dir)).collect();
    assert!(rows.contains(&("src/ui".to_string(), Some(true))), "the new folder shows, open: {rows:?}");
    assert_eq!(s.state().tree_reveal().map(|(path, _)| path), Some("src/ui".into()));
    // The new folder is the target for the next create without waiting for a rescan.
    highlight(&mut s, Some("src/ui"));
    assert_eq!(s.create_target().as_deref(), Some("src/ui"));
    s.dispatch(Command::CreateEntry { kind: Kind::File, dir: "src/ui".into(), name: "view.rs".into() });
    settle(&mut s, |s| s.state().active().is_some_and(|t| t.source() == root.join("src/ui/view.rs")));
    assert_eq!(std::fs::read(root.join("src/ui/view.rs")).unwrap(), b"");
}

#[test]
fn a_name_another_entry_takes_and_a_missing_or_foreign_folder_create_nothing() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    let mut s = project_with(&root, &[("src/a.rs", "fn a() {}\n")], "src/a.rs");
    highlight(&mut s, Some("src/a.rs"));
    assert_eq!(s.create_target().as_deref(), Some("src"), "a highlighted file creates beside it");
    assert_eq!(s.create_refusal("src", "a.rs").as_deref(), Some("src/a.rs already exists"));
    assert_eq!(s.create_refusal("", "src").as_deref(), Some("src already exists"), "a folder's name is taken too");
    assert!(s.create_refusal("src", "").is_some());
    assert!(s.create_refusal("src", "a/b.rs").is_some());
    assert_eq!(s.create_refusal("gone", "x.rs").as_deref(), Some("The folder no longer exists"));
    assert!(s.create_refusal("/etc", "x").is_some());
    assert!(s.create_refusal("../out", "x").is_some());
    let before: Vec<_> = std::fs::read_dir(&root).unwrap().map(|e| e.unwrap().file_name()).collect();
    s.dispatch(Command::CreateEntry { kind: Kind::File, dir: "src".into(), name: "a.rs".into() });
    assert_eq!(s.status_message(), "New file: src/a.rs already exists. Nothing was created.");
    s.dispatch(Command::CreateEntry { kind: Kind::Folder, dir: "gone".into(), name: "x".into() });
    assert_eq!(s.status_message(), "New folder: The folder no longer exists. Nothing was created.");
    let after: Vec<_> = std::fs::read_dir(&root).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(before, after);
    assert!(!root.join("gone").exists());
    assert_eq!(std::fs::read(root.join("src/a.rs")).unwrap(), b"fn a() {}\n");
}

#[test]
fn a_file_that_appeared_after_the_name_was_checked_is_not_replaced() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    let mut s = project_with(&root, &[("src/a.rs", "fn a() {}\n")], "src/a.rs");
    assert_eq!(s.create_refusal("src", "new.rs"), None);
    // Another program wrote the name between the prompt's check and Enter.
    std::fs::write(root.join("src/new.rs"), "theirs\n").unwrap();
    s.dispatch(Command::CreateEntry { kind: Kind::File, dir: "src".into(), name: "new.rs".into() });
    assert_eq!(std::fs::read(root.join("src/new.rs")).unwrap(), b"theirs\n");
    assert!(s.status_message().starts_with("New file: src/new.rs already exists"), "{}", s.status_message());
    assert!(!s.state().tabs().iter().any(|t| t.source() == root.join("src/new.rs")), "no tab claims it");
}

#[test]
fn a_folder_whose_mode_refuses_new_entries_is_read_only() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    let mut s = project_with(&root, &[("src/a.rs", "fn a() {}\n")], "src/a.rs");
    let folder = root.join("src");
    let mode = std::fs::metadata(&folder).unwrap().permissions().mode();
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).unwrap();
    let bite = std::fs::write(folder.join("probe"), "").is_ok();
    if bite {
        // Root ignores the bits: the check can only be shown where they bite.
        std::fs::remove_file(folder.join("probe")).unwrap();
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(mode)).unwrap();
        return;
    }
    assert_eq!(s.create_refusal("src", "new.rs").as_deref(), Some("The folder is read-only"));
    s.dispatch(Command::CreateEntry { kind: Kind::Folder, dir: "src".into(), name: "ui".into() });
    assert!(s.status_message().contains("The folder is read-only. Nothing was created."), "{}", s.status_message());
    assert!(!folder.join("ui").exists() && !folder.join("new.rs").exists() && !folder.join("probe").exists());
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(mode)).unwrap();
}

#[test]
fn created_entries_survive_the_rescan_the_watcher_starts() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    let mut s = project_with(&root, &[("src/a.rs", "fn a() {}\n")], "src/a.rs");
    s.dispatch(Command::CreateEntry { kind: Kind::Folder, dir: "".into(), name: "docs".into() });
    s.dispatch(Command::CreateEntry { kind: Kind::File, dir: "docs".into(), name: "readme.md".into() });
    settle(&mut s, |s| s.state().active().is_some_and(|t| t.source() == root.join("docs/readme.md")));
    s.dispatch(Completion::Changed(Change { paths: [root.clone()].into(), structure: true, ..Change::default() }));
    settle(&mut s, |s| s.state().project().is_some_and(|p| !p.refreshing() && matches!(p.coverage(), crate::project::Coverage::Complete(_))));
    let rows: Vec<String> = s.state().tree_rows().map(|r| r.path.to_string()).collect();
    assert!(rows.contains(&"docs".to_string()) && rows.contains(&"docs/readme.md".to_string()), "{rows:?}");
    // The kept file still reads as its own file.
    assert!(s.state().tabs().iter().any(|t| t.title() == "readme.md"));
}

#[test]
fn without_a_folder_there_is_nothing_to_create_in() {
    let d = tempfile::tempdir().unwrap();
    let s = session(d.path());
    assert_eq!(s.create_target(), None);
    assert_eq!(s.disabled(CommandId::NewExplorerFile), Some("No folder is open"));
    assert_eq!(s.action(CommandId::NewExplorerFolder), crate::actions::CatalogAction::Disabled("No folder is open"));
    assert_eq!(s.create_refusal("", "x.rs").as_deref(), Some("No folder is open"));
    assert_eq!(s.state().tree_reveal(), None);
}
