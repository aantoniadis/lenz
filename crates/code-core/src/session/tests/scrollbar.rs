use super::*;
use lenz_content::scrollbar::{Mark, MarkKind};

/// 200 lines: a Rust function on lines 20..=80 whose line 60 is a syntax error, "needle" on lines 10 and 150.
fn source() -> String {
    (0..200).map(|n| match n {
        20 => "fn a() {".to_owned(),
        60 => "    let = ;".to_owned(),
        80 => "}".to_owned(),
        10 | 150 => "// needle".to_owned(),
        n if (21..80).contains(&n) => format!("    let a{n} = {n};"),
        n => format!("// {n}"),
    }).collect::<Vec<_>>().join("\n")
}

fn fixture() -> (tempfile::TempDir, Session) {
    let d = tempfile::tempdir().unwrap();
    let mut s = folded(d.path(), &source());
    s.dispatch(Command::SetTextSize { pane: 0, cols: 80, rows: 20 });
    let tab = s.state().active().unwrap().id();
    settle(&mut s, |s| s.state().diagnostics(tab).is_some_and(|d| !d.items.is_empty()));
    (d, s)
}

#[test]
fn scrollbar_counts_shown_lines_and_marks_problems_finds_changes_bookmarks_and_the_caret() {
    let (_d, mut s) = fixture();
    caret(&mut s, 0);
    find(&mut s, "needle");
    caret(&mut s, 40);
    s.dispatch(Command::ToggleBookmark);
    caret(&mut s, 120);
    let (bar, marks) = s.state().scrollbar(0).unwrap();
    assert_eq!((bar.total, bar.top, bar.visible), (200.0, 0.0, 20.0));
    let kinds: Vec<_> = marks.iter().map(|m| (m.kind, m.target)).collect();
    assert!(kinds.contains(&(MarkKind::Error, 60)), "{kinds:?}");
    assert!(kinds.contains(&(MarkKind::FindCurrent, 10)) && kinds.contains(&(MarkKind::Find, 150)), "{kinds:?}");
    assert!(kinds.contains(&(MarkKind::Bookmark, 40)) && kinds.contains(&(MarkKind::Caret, 120)), "{kinds:?}");
}

#[test]
fn marks_in_folds_land_on_the_fold_row() {
    let (_d, mut s) = fixture();
    caret(&mut s, 40);
    s.dispatch(Command::Collapse);
    let (bar, marks) = s.state().scrollbar(0).unwrap();
    assert_eq!(bar.total, 140.0);
    let error = marks.iter().find(|m| m.kind == MarkKind::Error).unwrap();
    assert_eq!((error.row, error.target), (20.0, 60));
}

#[test]
fn git_changes_become_added_modified_and_deleted_marks() {
    use lenz_content::git::testing::{commit_all, repo, write as put};
    let d = tempfile::tempdir().unwrap();
    let r = repo();
    let base: Vec<String> = (0..100).map(|n| format!("line {n}")).collect();
    put(r.path(), "a.txt", &(base.join("\n") + "\n"));
    commit_all(r.path(), "base");
    let mut draft = base.clone();
    draft[5] = "changed".into();
    draft.remove(87);
    draft.splice(30..30, ["x".to_owned(), "y".to_owned(), "z".to_owned()]);
    put(r.path(), "a.txt", &(draft.join("\n") + "\n"));
    let mut s = session(&d.path().join("state"));
    s.dispatch(Command::OpenRoot(r.path().to_path_buf()));
    open(&mut s, Command::OpenRetained(r.path().join("a.txt")), "a.txt");
    s.dispatch(Command::SetTextSize { pane: 0, cols: 80, rows: 20 });
    let tab = s.state().active().unwrap().id();
    markers::settle_repo(&mut s, |s| s.state().markers(tab).is_some_and(|m| !m.pending() && m.change(5).is_some()));
    let (_, marks) = s.state().scrollbar(0).unwrap();
    assert!(marks.contains(&Mark { row: 30.0, rows: 3.0, kind: MarkKind::Added, target: 30 }), "{marks:?}");
    assert!(marks.contains(&Mark { row: 5.0, rows: 1.0, kind: MarkKind::Modified, target: 5 }), "{marks:?}");
    assert!(marks.iter().any(|m| m.kind == MarkKind::Deleted && m.target == 90), "{marks:?}");
}

#[test]
fn no_source_view_has_no_core_scrollbar() {
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    s.enable_text_input_for_test();
    let image = write(d.path(), "a.png", &png(4, 3, [255, 0, 0, 255]));
    open(&mut s, Command::OpenRetained(image), "a.png");
    s.dispatch(Command::SetTextSize { pane: 0, cols: 80, rows: 20 });
    assert!(s.state().scrollbar(0).is_none(), "an image shown as a picture");
    let md = write(d.path(), "a.md", b"# t\n\ntext\n");
    open(&mut s, Command::OpenRetained(md), "a.md");
    s.dispatch(Command::ToggleMarkdownSource);
    settle(&mut s, |s| matches!(s.state().rendering(), Some(Rendering::Preview(_))));
    assert!(s.state().scrollbar(0).is_none(), "a Markdown preview");
    let csv = write(d.path(), "a.csv", b"h,id\na,1\n");
    open(&mut s, Command::OpenRetained(csv), "a.csv");
    s.dispatch(Command::Csv(crate::csv::Command::Toggle));
    settle(&mut s, |s| s.state.active.and_then(|tab| s.state.csv(tab)).is_some_and(|v| !v.pending));
    assert!(s.state().scrollbar(0).is_none(), "a CSV table");
    s.dispatch(Command::Csv(crate::csv::Command::Toggle));
    assert!(s.state().scrollbar(0).is_some(), "the CSV source");
}
