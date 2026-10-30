use super::*;

#[gpui::test]
fn gui_tree_errors_preserve_names_dirty_git_marks_and_folder_arrow_hits(cx: &mut TestAppContext) {
    use lenz_content::{branches::FileStatus, git::testing::{repo, write as put, commit_all}};
    let project = repo(); put(project.path(), "sub/bad.rs", "fn main() {}\n"); commit_all(project.path(), "base");
    put(project.path(), "sub/bad.rs", "fn broken( {\n");
    let f = fixture(cx, None);
    f.view.update(cx, |r, cx| r.open_root(project.path().to_path_buf(), cx)); settle(cx, &f, scanned);
    f.view.update(cx, |r, cx| {
        r.dispatch(Command::RevealInTree("sub/bad.rs".into()), cx);
        r.dispatch(Command::OpenRetained(project.path().join("sub/bad.rs")), cx);
    });
    settle(cx, &f, |r| r.session.state().active().is_some_and(|t| r.session.state().diagnostics(t.id()).is_some()) && r.tree_lines(usize::MAX).iter().any(|row| row.name == "bad.rs"));
    let mut v = visual(cx, &f); v.simulate_resize(size(px(900.0),px(500.0))); draw(&mut v);
    f.view.update(&mut v, |r, _| r.enable_draft_input_for_test());
    v.simulate_keystrokes("end space");
    drop(v); settle(cx, &f, |r| r.session.state().active().is_some_and(|t| r.session.state().diagnostics(t.id()).is_some())); let mut v = visual(cx, &f);
    for theme in ["darcula", "github-light"] {
        f.view.update(&mut v, |r,cx| r.dispatch(Command::Theme(ThemeCommand::Select(ThemeSelection::from_id(theme).unwrap())),cx)); draw(&mut v);
        f.view.read_with(&v, |r,_| {
            let rows = r.tree_lines(usize::MAX);
            let folder = rows.iter().find(|row| row.name == "sub").unwrap();
            let file = rows.iter().find(|row| row.name == "bad.rs").unwrap();
            assert!(folder.error && file.error);
            assert!(file.dirty && file.status == Some(FileStatus::Modified), "error, unsaved mark and Git status remain independent");
            assert!(rows.iter().all(|row| !row.name.starts_with('!')), "error labels never alter filesystem names");
        });
    }
    let arrow = f.view.read_with(&v, |r,_| {
        let index = r.session.state().tree_rows().position(|row| &*row.name == "sub").unwrap() - r.tree_scroll;
        point(px(8.0),px(toolbar_h() + line_h() + (index as f32 + 0.5)*line_h()))
    });
    v.simulate_click(arrow,Modifiers::default()); draw(&mut v);
    assert!(!f.view.read_with(&v, |r,_| r.tree_lines(usize::MAX).iter().any(|row| row.name == "bad.rs")), "error decoration does not move the folder arrow hit");
    v.simulate_click(arrow,Modifiers::default()); draw(&mut v);
    assert!(f.view.read_with(&v, |r,_| r.tree_lines(usize::MAX).iter().any(|row| row.name == "bad.rs")));
    for (w,h) in [(240.0,160.0),(1.0,1.0),(900.0,500.0)] { v.simulate_resize(size(px(w),px(h))); draw(&mut v); }
    assert_eq!(std::fs::read(project.path().join("sub/bad.rs")).unwrap(), b"fn broken( {\n");
}
