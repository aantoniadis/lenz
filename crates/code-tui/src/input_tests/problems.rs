use super::*;

#[test]
fn problems_palette_opens_all_known_files_and_badges_render_in_tiny_layouts() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("bad.java");
    let second = dir.path().join("also.rs");
    std::fs::write(&first, b"class Bad { void f( { }\n").unwrap();
    std::fs::write(&second, b"fn broken( {\n").unwrap();
    let mut a = app(dir.path());
    for path in [&first, &second] {
        a.dispatch(Command::OpenRetained(path.clone()));
        settle(&mut a, |a| a.session.state().active().is_some_and(|t| t.source() == path && a.session.state().diagnostics(t.id()).is_some()));
    }
    let screen = draw_screen(&mut a, 100, 24);
    assert!(screen.contains('!'), "{screen}");
    key(&mut a, KeyCode::F(10), KeyModifiers::NONE);
    let row = a.choices().iter().position(|row| matches!(&row.target, ChoiceTarget::Action(CommandId::ShowProblems))).unwrap();
    for _ in 0..row { key(&mut a, KeyCode::Down, KeyModifiers::NONE); }
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(a.modal.is_none() && a.focus == Focus::Results);
    let results = a.session.state().results().unwrap();
    assert!(results.hits().iter().any(|h| h.path.ends_with("bad.java")));
    assert!(results.hits().iter().any(|h| h.path.ends_with("also.rs")));
    assert!(draw_screen(&mut a, 120, 30).contains("Problems"));
    draw(&mut a, 28, 6);
    draw(&mut a, 1, 1);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.session.state().results().is_none());
    assert_eq!(std::fs::read(first).unwrap(), b"class Bad { void f( { }\n");
}

#[test]
fn inline_parser_errors_append_after_source_without_changing_copy_or_empty_eof_hits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.xml");
    let source = "<Root></root>\n";
    std::fs::write(&path, source).unwrap();
    let mut a = App::new(config(dir.path()), Some(path.clone()));
    settle(&mut a, |a| a.session.state().active().is_some_and(|t| a.session.state().inline_problem(t.id(), 0).is_some()));
    let screen = draw_screen(&mut a, 110, 16);
    assert!(screen.contains("<Root></root>  Error: XML:"), "inline error follows the source: {screen}");
    a.dispatch(Command::SelectAll);
    assert_eq!(copied_selection(a.session.state()).as_deref(), Some(source), "inline annotations are never source bytes");
    let empty = dir.path().join("empty.xml"); std::fs::write(&empty, b"").unwrap();
    a.dispatch(Command::OpenRetained(empty));
    settle(&mut a, |a| a.session.state().active().is_some_and(|t| a.session.state().inline_problem(t.id(), 0).is_some_and(|p| p.empty)));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(110, 16)).unwrap();
    terminal.draw(|f| a.draw(f)).unwrap();
    let hit = a.layout.panes[0].hits.iter().find(|h| h.line == 0 && h.byte == 0).unwrap();
    let anchor = &terminal.backend().buffer()[(hit.x, hit.y)];
    assert!(anchor.modifier.contains(ratatui::style::Modifier::UNDERLINED));
    assert!(anchor.symbol().contains('!'), "empty EOF has a visible error anchor");
    for (w,h) in [(24,6),(1,1),(110,16)] { draw(&mut a,w,h); }
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}

#[test]
fn terminal_tree_errors_use_plain_names_coloured_names_and_separate_markers_without_moving_hits() {
    use lenz_content::{branches::FileStatus, git::testing::{repo, write as put, commit_all}};
    let project = repo(); put(project.path(), "sub/bad.rs", "fn main() {}\n"); commit_all(project.path(), "base");
    put(project.path(), "sub/bad.rs", "fn broken( {\n");
    let dir = tempfile::tempdir().unwrap();
    let mut a = App::new(config(dir.path()), Some(project.path().to_path_buf()));
    settle(&mut a, |a| a.session.state().tree_rows().any(|r| &*r.name == "sub"));
    a.dispatch(Command::RevealInTree("sub/bad.rs".into()));
    a.dispatch(Command::OpenRetained(project.path().join("sub/bad.rs"))); a.enable_draft_input_for_test();
    settle(&mut a, |a| a.session.state().active().is_some_and(|t| a.session.state().diagnostics(t.id()).is_some()));
    a.dispatch(Command::InsertText { text: " ".into(), kind: code_core::text::EditKind::Typing });
    settle(&mut a, |a| a.session.state().active().is_some_and(|t| a.session.state().diagnostics(t.id()).is_some()));
    settle(&mut a, |a| a.session.state().tree_rows().find(|r| &*r.name == "bad.rs")
        .is_some_and(|row| a.session.state().row_status(row) == Some(FileStatus::Modified)));
    for nerd in [true,false] {
        a.icons_override = Some(nerd);
        for theme in ["darcula","github-light"] {
            a.dispatch(Command::Theme(lenz_content::theme::ThemeCommand::Select(lenz_content::theme::ThemeSelection::from_id(theme).unwrap())));
            let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100,20)).unwrap(); terminal.draw(|f|a.draw(f)).unwrap();
            let area = a.layout.tree;
            for name in ["sub","bad.rs"] {
                let row = a.session.state().tree_rows().position(|r| &*r.name == name).unwrap();
                let y = area.y+(row-a.tree_top) as u16;
                let text = (area.x..area.right()).map(|x|terminal.backend().buffer()[(x,y)].symbol()).collect::<String>();
                let byte = text.find(name).unwrap(); let x = area.x + text[..byte].chars().count() as u16;
                assert_eq!(terminal.backend().buffer()[(x,y)].fg,crate::render::color(a.session.state().palette().error));
                assert!(text.contains('•'), "a non-colour error marker remains visible: {text}");
                assert!(!text.contains("!1") && !text.contains("!2"));
            }
            let entry = a.session.state().tree_rows().find(|r| &*r.name == "bad.rs").unwrap();
            assert!(a.session.state().row_dirty(entry)); assert_eq!(a.session.state().row_status(entry),Some(FileStatus::Modified));
        }
    }
    let area = a.layout.tree;
    let row = a.session.state().tree_rows().position(|r| &*r.name == "sub").unwrap();
    let y = area.y+(row-a.tree_top)as u16;
    click(&mut a,area.x+1,y);
    assert!(!a.session.state().tree_rows().any(|r| &*r.name == "bad.rs"),"folder arrow/name hit remains on its original row");
    for (w,h) in [(24,6),(1,1),(100,20)] {draw(&mut a,w,h);}
    assert_eq!(std::fs::read(project.path().join("sub/bad.rs")).unwrap(),b"fn broken( {\n");
}

#[test]
fn inline_terminal_server_problem_uses_most_severe_message_once_after_wrapped_line() {
    let dir = tempfile::tempdir().unwrap(); let root = dir.path().join("p"); std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname='inline'\nversion='0.1.0'\n").unwrap();
    let source = "fn main() { let unused_first = 1; bad_name; }\n"; let path = root.join("main.rs"); std::fs::write(&path,source).unwrap();
    let profile = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_owned();
    let mut build = std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_|"cargo".into()));
    build.args(["build","-q","-p","code-core","--example","fake_lsp"]); if profile.ends_with("release") {build.arg("--release");} assert!(build.status().unwrap().success());
    let cfg = config(dir.path()); std::fs::create_dir_all(cfg.providers.parent().unwrap()).unwrap();
    std::fs::write(&cfg.providers,serde_json::json!({"providers":{"rust-analyzer":{"executable":profile.join("examples/fake_lsp"),"env":{"FAKE_LSP_NO_DECOR":"1"}}}}).to_string()).unwrap();
    let mut a = App::new(cfg,Some(root)); a.dispatch(Command::OpenRetained(path.clone()));
    settle(&mut a,|a|a.session.state().active().is_some_and(|t|a.session.state().inline_problem(t.id(),0).is_some_and(|p|p.other_count==1)));
    let mut terminal=ratatui::Terminal::new(ratatui::backend::TestBackend::new(220,20)).unwrap();terminal.draw(|f|a.draw(f)).unwrap();
    let y=a.layout.panes[0].rows.iter().find(|(_,line,index)|*line==0&&*index==0).unwrap().0;
    let area=a.layout.panes[0].text;
    let row=(area.x..area.right()).map(|x|terminal.backend().buffer()[(x,y)].symbol()).collect::<String>();
    assert!(row.contains("Error: cannot find value `bad_name` in this scope (+1)"),"{row}");
    assert!(!row.contains("Warning:"));
    a.dispatch(Command::SetSoftWrap(true));
    let screen=draw_screen(&mut a,60,20);
    assert_eq!(screen.matches("Error:").count(),1,"one annotation follows the logical line's last segment: {screen}");
    a.dispatch(Command::SelectAll); assert_eq!(copied_selection(a.session.state()).as_deref(),Some(source));
    assert_eq!(std::fs::read_to_string(path).unwrap(),source);
}
