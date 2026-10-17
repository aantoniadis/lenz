//! Real event and cell checks for the GUI-like terminal chrome.
use crate::app::{App, tests::{pair, ready}};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use lenz::markdown::SourcePos as _;
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

fn draw(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| app.render(f)).unwrap();
    terminal.backend().buffer().clone()
}
fn text(buffer: &Buffer) -> String {
    use unicode_width::UnicodeWidthStr;
    let mut result=String::new();
    for y in 0..buffer.area.height {let mut x=0;while x<buffer.area.width {let symbol=buffer[(x,y)].symbol();result.push_str(symbol);x=x.saturating_add(symbol.width().max(1) as u16);}}
    result
}
fn locate(buffer: &Buffer, needle: &str) -> (u16,u16) {
    use unicode_width::UnicodeWidthStr;
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let mut row=String::new();let mut column=x;while column<buffer.area.width {let symbol=buffer[(column,y)].symbol();row.push_str(symbol);column=column.saturating_add(symbol.width().max(1) as u16);}
            if row.starts_with(needle) { return (x,y); }
        }
    }
    panic!("missing {needle:?}: {}",text(buffer))
}
fn key(app: &mut App, code: KeyCode) {
    assert!(app.handle(Event::Key(KeyEvent::new(code,KeyModifiers::NONE))));
}
fn click(app: &mut App, at: (u16,u16)) {
    assert!(app.handle(Event::Mouse(MouseEvent{kind:MouseEventKind::Down(MouseButton::Left),column:at.0,row:at.1,modifiers:KeyModifiers::NONE})));
}

fn markdown_repository_source_hash(root:&std::path::Path)->u64 {
    use std::hash::{Hash,Hasher};let mut hash=std::collections::hash_map::DefaultHasher::new();
    for name in ["gone.md","new.md","ref.md"]{std::fs::read(root.join(name)).ok().hash(&mut hash);}hash.finish()
}
fn pair_markdown(left: &str, right: &str) -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("left.md"), left).unwrap();
    std::fs::write(dir.path().join("right.md"), right).unwrap();
    let req = lenz::cli::parse(&["--no-index".into(), "left.md".into(), "right.md".into()], dir.path()).unwrap();
    let mut app = App::open(req, dir.path().into());
    ready(&mut app);
    // most checks here are about two panes; Inline tests choose it explicitly
    app.session.execute(lenz::commands::Command::SetMarkdownLayout(lenz::markdown::MarkdownLayout::SideBySide));
    (dir, app)
}

#[test]
fn markdown_tui_toolbar_renders_headings_and_accounts_for_link_edits() {
    let (_dir, mut app) = pair_markdown("# Title\n\n[guide](old)\n", "# Title\n\n[guide](new)\n");
    let screen = draw(&mut app, 160, 36);
    click(&mut app, locate(&screen, "Side by side"));
    ready(&mut app);
    let screen = draw(&mut app, 160, 36);
    let content = text(&screen);
    assert!(content.contains("Title"));
    assert!(!content.contains("# Title"));
    assert!(content.contains("Markup change"));
}

#[test]
fn files_pane_horizontal_keys_do_not_move_the_diff_selection() {
    let (_dir, mut app) = pair("old words\n", "new words\n");
    app.navigator.visible = Some(true);
    let screen = draw(&mut app, 160, 30);
    let at = (app.diff_area.x + app.diff_area.width / 2 + 12, app.diff_area.y + 2);
    click(&mut app, at);
    let selection = app.selection.unwrap();
    click(&mut app, locate(&screen, "Files (1)"));
    for code in [KeyCode::Left, KeyCode::Right] {
        key(&mut app, code);
        assert_eq!(app.selection, Some(selection));
    }
}

#[test]
fn clicking_diff_transfers_keyboard_focus_from_repository_log() {
    use lenz::repository_session::{Pane, RepositoryCommand};
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    std::fs::write(dir.path().join("file.txt"), "old\n".repeat(100)).unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-qm", "Initial"]);
    std::fs::write(dir.path().join("file.txt"), "new\n".repeat(100)).unwrap();
    let req = lenz::cli::parse(&["log".into()], dir.path()).unwrap();
    let mut app = App::open(req, dir.path().into());
    ready(&mut app);
    app.repository.as_mut().unwrap().core.execute(RepositoryCommand::SelectRow { row: 0, extend: false, two: false });
    ready(&mut app);
    draw(&mut app, 160, 40);
    app.repository.as_mut().unwrap().core.execute(RepositoryCommand::Focus(Pane::Log));
    let at = (app.diff_area.x + 12, app.diff_area.y + 2);
    click(&mut app, at);
    assert_eq!(app.repository.as_ref().unwrap().core.picker.focus, Pane::Diff);
    let before = app.selection.unwrap();
    key(&mut app, KeyCode::Down);
    assert!(app.selection.unwrap().head.line > before.head.line);
}

#[test]
fn search_prompt_and_results_movement_do_not_move_the_diff_caret() {
    for query in ["", "unmatched"] {
    let (_dir, mut app) = pair("old words\n", "new words\n");
    draw(&mut app, 160, 30);
    let at = (app.diff_area.x + app.diff_area.width / 2 + 12, app.diff_area.y + 2);
    click(&mut app, at);
    let selection = app.selection;
    app.handle(Event::Key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL | KeyModifiers::SHIFT)));
    for character in query.chars() { key(&mut app, KeyCode::Char(character)); }
    key(&mut app, KeyCode::Enter);
    ready(&mut app);
    for code in [KeyCode::Left, KeyCode::Right, KeyCode::Home, KeyCode::End] {
        key(&mut app, code);
        assert_eq!(app.selection, selection, "query {query:?}, key {code:?}");
    }
    }
}

#[test]
fn files_keep_navigation_focus_after_search_and_mouse_wheel() {
    let dir = tempfile::tempdir().unwrap();
    for side in ["left", "right"] {
        std::fs::create_dir(dir.path().join(side)).unwrap();
        for i in 0..40 {
            std::fs::write(dir.path().join(side).join(format!("{i:02}.txt")), format!("{side} words\n").repeat(100)).unwrap();
        }
    }
    let req = lenz::cli::parse(&["--no-index".into(), "left".into(), "right".into()], dir.path()).unwrap();
    let mut app = App::open(req, dir.path().into());
    ready(&mut app);
    let screen = draw(&mut app, 160, 30);
    click(&mut app, locate(&screen, " M 00.txt"));
    key(&mut app, KeyCode::Down); ready(&mut app);
    assert_eq!(app.session.current(), 1);
    key(&mut app, KeyCode::End); ready(&mut app);
    assert_eq!(app.session.current(), 39);
    let screen = draw(&mut app, 160, 30);
    let at = locate(&screen, "Files (40)");
    let anchor = app.session.view().anchor_line();
    app.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::ScrollUp, column: at.0, row: at.1, modifiers: KeyModifiers::NONE }));
    assert_eq!(app.session.view().anchor_line(), anchor);
    assert_ne!(text(&draw(&mut app, 160, 30)), text(&screen));
    app.handle(Event::Key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL | KeyModifiers::SHIFT)));
    for c in "no-matches".chars() { key(&mut app, KeyCode::Char(c)); }
    key(&mut app, KeyCode::Enter); ready(&mut app);
    let screen = draw(&mut app, 160, 30);
    click(&mut app, locate(&screen, "Files (40)"));
    key(&mut app, KeyCode::Up); ready(&mut app);
    assert_eq!(app.session.current(), 38, "search must give keyboard focus back to the clicked files pane");
    draw(&mut app, 35, 8);
    assert!(!app.navigator.focused, "a hidden narrow-layout pane cannot retain focus");
}

#[test]
fn tab_and_shift_tab_reach_files_without_a_repository_picker() {
    let (_dir, mut app) = pair("old words\n", "new words\n");
    app.navigator.visible = Some(true);
    let screen = draw(&mut app, 160, 30);
    let at = (app.diff_area.x + 12, app.diff_area.y + 2);
    click(&mut app, at);
    key(&mut app, KeyCode::BackTab);
    assert!(app.navigator.focused);
    key(&mut app, KeyCode::Tab);
    assert!(!app.navigator.focused);
    assert_eq!(app.focus, lenz::view::Side::Left);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.focus, lenz::view::Side::Right);
    key(&mut app, KeyCode::Tab);
    assert!(app.navigator.focused);
    click(&mut app, locate(&screen, "Files (1)"));
    key(&mut app, KeyCode::BackTab);
    assert!(!app.navigator.focused);
    assert_eq!(app.focus, lenz::view::Side::Right);
}
#[test]
fn gui_like_toolbar_dropdown_uses_real_mouse_and_keyboard() {
    let (_dir,mut app)=pair(" a \n","a\n");
    let screen=draw(&mut app,160,30);click(&mut app,locate(&screen,"⚙"));
    let screen=draw(&mut app,160,30);for row in ["Layout: Split","Whitespace: Compare all","Highlight: Words","Scroll: Shared"] {assert!(text(&screen).contains(row),"{row}");}
    click(&mut app,locate(&screen,"Whitespace:"));let screen=draw(&mut app,160,30);assert!(text(&screen).contains("Ignore + trim chunks"));
    key(&mut app,KeyCode::Down);key(&mut app,KeyCode::Down);key(&mut app,KeyCode::Enter);ready(&mut app);assert_eq!(app.session.options().whitespace,lenz_engine::WhitespaceMode::Ignore);
    let screen=draw(&mut app,160,30);click(&mut app,locate(&screen,"⚙"));let screen=draw(&mut app,160,30);assert!(text(&screen).contains("Whitespace: Ignore"));click(&mut app,locate(&screen,"Highlight:"));
    key(&mut app,KeyCode::Down);key(&mut app,KeyCode::Enter);ready(&mut app);assert_eq!(app.session.options().highlight,lenz_engine::HighlightLevel::Char);
    let screen=draw(&mut app,160,30);click(&mut app,locate(&screen,"⚙"));let screen=draw(&mut app,160,30);click(&mut app,locate(&screen,"Scroll:"));key(&mut app,KeyCode::Down);key(&mut app,KeyCode::Enter);assert_eq!(app.session.view().mode(),lenz::view::ScrollMode::Aligned);
    let screen=draw(&mut app,160,30);click(&mut app,locate(&screen,"⚙"));let screen=draw(&mut app,160,30);click(&mut app,locate(&screen,"Layout:"));key(&mut app,KeyCode::Down);key(&mut app,KeyCode::Enter);assert!(app.session.view().unified(),"Layout is in the Diff menu");
}
fn git(dir:&std::path::Path,args:&[&str]) {
    let result=std::process::Command::new("git").args(["-c","user.name=Tester","-c","user.email=test@example.com","-c","core.hooksPath=/dev/null","-c","commit.gpgsign=false"]).args(args).current_dir(dir).output().unwrap();
    assert!(result.status.success(),"{}",String::from_utf8_lossy(&result.stderr));
}
#[test]
fn gui_like_log_lists_commit_files_and_clicks_open_the_correct_file() {
    use lenz::repository_session::{Pane,RepositoryCommand};
    let dir=tempfile::tempdir().unwrap();git(dir.path(),&["init","-q"]);
    std::fs::create_dir(dir.path().join("alpha")).unwrap();std::fs::create_dir(dir.path().join("beta")).unwrap();
    std::fs::write(dir.path().join("alpha/first.rs"),"original alpha\n").unwrap();std::fs::write(dir.path().join("deleted.txt"),"old deletion\n").unwrap();
    git(dir.path(),&["add","."]);git(dir.path(),&["commit","-qm","First visible commit"]);
    std::fs::write(dir.path().join("alpha/first.rs"),"changed alpha\n").unwrap();std::fs::write(dir.path().join("beta/second.rs"),"new beta contents\n").unwrap();std::fs::remove_file(dir.path().join("deleted.txt")).unwrap();
    git(dir.path(),&["add","-A"]);git(dir.path(),&["commit","-qm","Second visible commit"]);
    let req=lenz::cli::parse(&["log".into()],dir.path()).unwrap();let mut app=App::open(req,dir.path().into());ready(&mut app);
    let repo=app.repository.as_mut().unwrap();let row=(0..repo.core.picker.rows()).find(|&row|repo.core.picker.log_row(row).is_some_and(|i|repo.core.picker.log.subject(i)=="Second visible commit")).unwrap();
    repo.core.execute(RepositoryCommand::SelectRow{row,extend:false,two:false});ready(&mut app);
    let screen=draw(&mut app,160,40);
    let repository_diff=app.repository.as_ref().unwrap().area(app.popup_area);
    assert!(app.diff_area.x>repository_diff.x,"commit file navigator must sit beside the diff");
    assert!(text(&screen).contains("Files (3)"));
    let (x,y)=locate(&screen,"second.rs");assert!(x<app.diff_area.x);
    click(&mut app,(x,y));ready(&mut app);
    assert_eq!(app.session.comparisons()[app.session.current()].path,"beta/second.rs");
    assert!(text(&draw(&mut app,160,40)).contains("new beta contents"));
    assert!(app.handle(Event::Key(KeyEvent::new(KeyCode::Char('f'),KeyModifiers::CONTROL|KeyModifiers::SHIFT))));
    assert!(!app.navigator.focused,"opening search transfers focus from files");
    for c in "contents".chars(){key(&mut app,KeyCode::Char(c));}
    key(&mut app,KeyCode::Enter);ready(&mut app);
    assert!(!app.set_search.as_ref().unwrap().model.editing(),"file focus must not consume search submission");
    assert_eq!(app.set_search.as_ref().unwrap().model.hits().len(),1);
    key(&mut app,KeyCode::Esc);ready(&mut app);
    app.repository.as_mut().unwrap().core.execute(RepositoryCommand::Focus(Pane::Log));
    key(&mut app,KeyCode::End);ready(&mut app);
    let screen=draw(&mut app,160,40);
    assert!(text(&screen).contains("Files (2)"));
    assert!(!text(&screen).contains("second.rs"),"stale commit files must disappear");
    key(&mut app,KeyCode::F(1));
    let help=draw(&mut app,160,40);
    assert!(text(&help).contains("Commits/Files"),"repository help must cover the file navigator: {}",text(&help));
    assert_eq!(help[(40,20)].bg,crate::chrome::color(crate::chrome::palette().status_bg),"file pane must not paint over repository help");
}

#[test]
fn help_is_a_readable_scrollable_shortcut_table_at_narrow_sizes() {
    let (_dir,mut app)=pair("old\n","new\n");
    key(&mut app,KeyCode::Char('?'));
    let screen=draw(&mut app,80,24);
    assert!(text(&screen).contains("Keyboard shortcuts"));
    assert!(text(&screen).contains("Keys")&&text(&screen).contains("Action"));
    assert!(text(&screen).contains("Esc close"));
    key(&mut app,KeyCode::End);
    let screen=draw(&mut app,55,16);
    assert!(text(&screen).contains("F8 / Shift-F8"));
    key(&mut app,KeyCode::Esc);
    assert!(app.prompt.is_none());
    for width in 0..7 {for height in 0..7 {let _=draw(&mut app,width,height);let _=draw(&mut app,width,height);}}
}

#[test]
fn narrow_toolbar_keeps_last_controls_and_dropdown_choices_reachable() {
    let (_dir,mut app)=pair("old\n","new\n");
    // At 35 columns everything fits (no ⋯); at 20 the last buttons fold into ⋯.
    let screen=draw(&mut app,35,12);assert!(!text(&screen).contains("⋯")&&text(&screen).contains("↑")&&text(&screen).contains("?"));
    let screen=draw(&mut app,20,6);let bar:String=(0..20).map(|x|screen[(x,0)].symbol().to_string()).collect();assert!(text(&screen).contains("⋯")&&!bar.contains("?"),"{bar}");
    assert!(app.handle(Event::Key(KeyEvent::new(KeyCode::F(10),KeyModifiers::ALT))));key(&mut app,KeyCode::End);key(&mut app,KeyCode::Enter);key(&mut app,KeyCode::End);
    let s=text(&draw(&mut app,20,6));assert!(s.contains("All commands…"),"{s}");key(&mut app,KeyCode::Enter);assert!(app.command_palette.is_some());
    typed(&mut app,"keyboard");key(&mut app,KeyCode::Enter);assert!(app.prompt==Some(crate::app::Prompt::Help));assert!(text(&draw(&mut app,35,12)).contains("Keyboard shortcuts"));key(&mut app,KeyCode::Esc);
    let screen=draw(&mut app,160,30);click(&mut app,locate(&screen,"⚙"));let screen=draw(&mut app,160,30);click(&mut app,locate(&screen,"Whitespace:"));key(&mut app,KeyCode::End);
    assert!(text(&draw(&mut app,160,8)).contains("Ignore + trim chunks"));key(&mut app,KeyCode::Enter);ready(&mut app);assert_eq!(app.session.options().whitespace,lenz_engine::WhitespaceMode::IgnoreTrimChunks);
}

fn many_files(app:&App)->std::sync::Arc<lenz::changeset::ChangeSet> {
    let mut set=app.session.change_set().unwrap().as_ref().clone();let comparison=set.comparisons[0].clone();
    set.comparisons=(0..10000).map(|i|lenz::changeset::Comparison{path:format!("dir{:03}/file{i:05}.txt",i/100),old_path:format!("dir{:03}/file{i:05}.txt",i/100),..comparison.clone()}).collect();std::sync::Arc::new(set)
}
#[test]
fn file_navigator_reveals_switches_and_preserves_manual_scroll() {
    let (_dir,app)=pair("old\n","new\n");let set=many_files(&app);
    let mut files=crate::files::Files::default();let area=ratatui::layout::Rect::new(0,0,160,30);
    let mut terminal=Terminal::new(TestBackend::new(160,30)).unwrap();
    for index in [0,9999,0]{files.prepare(Some(set.clone()),&lenz::folders::Folders::default(),index,false,area);terminal.draw(|f|files.draw(f,crate::chrome::palette())).unwrap();assert!(text(terminal.backend().buffer()).contains(&format!("file{index:05}.txt")));}
    files.handle(&Event::Mouse(MouseEvent{kind:MouseEventKind::ScrollDown,column:5,row:5,modifiers:KeyModifiers::NONE}));
    files.prepare(Some(set.clone()),&lenz::folders::Folders::default(),0,false,area);terminal.draw(|f|files.draw(f,crate::chrome::palette())).unwrap();
    assert!(!text(terminal.backend().buffer()).contains("file00000.txt"));
    assert_eq!(std::sync::Arc::strong_count(&set),1,"navigator must not retain an obsolete Change Set");
}
#[test]
#[ignore="release toolbar/file navigator/help stress workload"]
fn terminal_perf_chrome() {
    use std::time::Instant;
    let (_dir,mut app)=pair("old\n","new\n");let set=many_files(&app);
    let mut files=crate::files::Files::default();let area=ratatui::layout::Rect::new(0,0,160,50);
    let mut terminal=Terminal::new(TestBackend::new(160,50)).unwrap();
    files.prepare(Some(set.clone()),&lenz::folders::Folders::default(),0,false,area);terminal.draw(|f|files.draw(f,crate::chrome::palette())).unwrap();
    let mut render=Vec::new();let mut switches=Vec::new();let mut rss=Vec::new();
    for i in 0..300 {
        let start=Instant::now();files.prepare(Some(set.clone()),&lenz::folders::Folders::default(),if i%2==0{9999}else{0},false,area);terminal.draw(|f|files.draw(f,crate::chrome::palette())).unwrap();switches.push(start.elapsed().as_secs_f64()*1000.);
        let start=Instant::now();files.prepare(Some(set.clone()),&lenz::folders::Folders::default(),if i%2==0{9999}else{0},false,area);terminal.draw(|f|files.draw(f,crate::chrome::palette())).unwrap();render.push(start.elapsed().as_secs_f64()*1000.);
        if let Some(value)=crate::perf::rss_mib(){rss.push(value);}
    }
    crate::perf::record("tui/chrome/cached-render-10k-files","ms",&render);
    crate::perf::record("tui/chrome/switch-render-10k-files","ms",&switches);
    if !rss.is_empty(){crate::perf::record("tui/chrome/stress-resident-10k-files","MiB",&rss);}
    key(&mut app,KeyCode::Char('?'));let mut help=Vec::new();
    for i in 0..100 {key(&mut app,if i%2==0{KeyCode::Home}else{KeyCode::End});let start=Instant::now();terminal.draw(|f|app.render(f)).unwrap();help.push(start.elapsed().as_secs_f64()*1000.);}
    crate::perf::record("tui/chrome/help-render","ms",&help);
}

#[test]
fn about_menu_is_modal_and_restores_comparison_after_dismissal() {
    let (_dir,mut app)=pair("old\n","new\n");
    key(&mut app,KeyCode::Char(':'));
    for c in "about".chars(){key(&mut app,KeyCode::Char(c));}
    assert!(text(&draw(&mut app,100,30)).contains("About Lenz"));
    key(&mut app,KeyCode::Enter);
    let screen=draw(&mut app,100,30);
    for required in ["About Lenz","Version","Platform","MIT; engine Apache-2.0","MIT","github.com/aantoniadis/lenz","Close"] {
        assert!(text(&screen).contains(required),"missing {required}: {}",text(&screen));
    }
    let options=app.session.options();
    key(&mut app,KeyCode::Char('w'));
    assert_eq!(app.session.options(),options,"About must not pass keys to comparison");
    assert!(text(&draw(&mut app,100,30)).contains("About Lenz"));
    let screen=draw(&mut app,100,30);click(&mut app,locate(&screen,"Close"));
    assert!(!text(&draw(&mut app,100,30)).contains("About Lenz"));
    let screen=draw(&mut app,160,30);click(&mut app,locate(&screen,"☰"));let screen=draw(&mut app,160,30);click(&mut app,locate(&screen,"About Lenz"));
    key(&mut app,KeyCode::Esc);
    assert!(app.prompt.is_none());
}

#[test]
fn about_resizes_scrolls_and_renders_logo_without_touching_the_document() {
    let (_dir,mut app)=pair("old\n","new\n");
    let document=app.session.document_arc().unwrap();
    let screen=draw(&mut app,160,30);click(&mut app,locate(&screen,"☰"));let screen=draw(&mut app,160,30);click(&mut app,locate(&screen,"About Lenz"));
    let screen=draw(&mut app,100,30);
    assert!(screen.content.iter().filter(|c|c.symbol()=="▀").count()>=24*12);
    let colors=screen.content.iter().filter(|c|c.symbol()=="▀").map(|c|format!("{:?}",c.fg)).collect::<std::collections::HashSet<_>>();
    assert!(colors.len()>20,"logo must retain the original colors");
    for width in 0..9 {for height in 0..9 {let _=draw(&mut app,width,height);}}
    key(&mut app,KeyCode::End);
    assert!(text(&draw(&mut app,30,10)).contains("github.com/aantoniadis/lenz".split_at(20).1));
    key(&mut app,KeyCode::Home);
    assert!(text(&draw(&mut app,30,10)).contains("Side-by-side"));
    key(&mut app,KeyCode::Enter);
    assert!(app.about.is_none());
    assert!(std::sync::Arc::ptr_eq(&document,&app.session.document_arc().unwrap()));
}

#[test]
#[ignore="release About reopen/resize/resident-memory workload"]
fn terminal_perf_about() {
    let (_dir,mut app)=pair("old\n","new\n");
    let document=app.session.document_arc().unwrap();
    let mut open=Vec::new();let mut close=Vec::new();let mut rss=Vec::new();
    let mut terminal=Terminal::new(TestBackend::new(160,30)).unwrap();
    terminal.draw(|f|app.render(f)).unwrap();
    for i in 0..300 {
        key(&mut app,KeyCode::Char(':'));for c in "about".chars(){key(&mut app,KeyCode::Char(c));}
        let start=std::time::Instant::now();key(&mut app,KeyCode::Enter);terminal.draw(|f|app.render(f)).unwrap();open.push(start.elapsed().as_secs_f64()*1000.0);
        assert!(text(terminal.backend().buffer()).contains("MIT; engine Apache-2.0"));
        if i%10==0{for (w,h) in [(40,10),(1,1),(160,30)] {app.handle(Event::Resize(w,h));terminal.resize(ratatui::layout::Rect::new(0,0,w,h)).unwrap();terminal.draw(|f|app.render(f)).unwrap();}}
        let start=std::time::Instant::now();key(&mut app,KeyCode::Esc);terminal.draw(|f|app.render(f)).unwrap();close.push(start.elapsed().as_secs_f64()*1000.0);
        if let Some(value)=crate::perf::rss_mib(){rss.push(value);}
    }
    assert!(std::sync::Arc::ptr_eq(&document,&app.session.document_arc().unwrap()));
    crate::perf::record("tui/about/open-render","ms",&open);crate::perf::record("tui/about/close-render","ms",&close);
    if !rss.is_empty(){crate::perf::record("tui/about/reopen-resident","MiB",&rss);}
}

#[test]
fn standalone_about_opens_outside_git_and_closes_to_an_empty_view(){
    let dir=tempfile::tempdir().unwrap();
    let request=lenz::cli::parse(&["--about".into()],dir.path()).unwrap();
    let mut app=crate::app::App::open(request,dir.path().to_path_buf());
    while app.busy(){app.poll();std::thread::sleep(std::time::Duration::from_millis(1));}
    assert!(text(&draw(&mut app,100,30)).contains("MIT; engine Apache-2.0"));
    key(&mut app,KeyCode::Esc);assert!(app.about.is_none());
    assert!(!app.failed());assert!(app.session.comparisons().is_empty());
    assert!(!text(&draw(&mut app,100,30)).contains("About Lenz"));
}

fn discard_app()->(tempfile::TempDir,App){
    let dir=tempfile::Builder::new().prefix(".lenz-discard-").tempdir_in(std::env::current_dir().unwrap()).unwrap();git(dir.path(),&["init","-q"]);std::fs::write(dir.path().join("file.txt"),"head\n").unwrap();git(dir.path(),&["add","file.txt"]);git(dir.path(),&["commit","-qm","initial"]);
    std::fs::write(dir.path().join("file.txt"),b"indexed\r\n").unwrap();git(dir.path(),&["add","file.txt"]);std::fs::write(dir.path().join("file.txt"),b"working\n").unwrap();
    let spec=lenz::changeset::Spec::Git{cached:false,revs:Vec::new(),paths:Vec::new(),untracked:false};let set=std::sync::Arc::new(lenz::changeset::ChangeSet::load(&spec,dir.path()).unwrap());let mut app=App::new(lenz::session::Session::from_set_named(set,spec,Default::default(),lenz::view::ScrollMode::Aligned,Some("file.md".into()),None));app.navigator.visible=Some(true);ready(&mut app);(dir,app)
}
fn action_ready(app:&mut App){let until=std::time::Instant::now()+std::time::Duration::from_secs(5);while app.session.file_actions().busy(){app.poll();assert!(std::time::Instant::now()<until);std::thread::sleep(std::time::Duration::from_millis(2));}}
fn file_context(app:&mut App,action:lenz::file_actions::FileAction){
    let screen=draw(app,160,30);let(x,y)=locate(&screen,"M file.txt");app.handle(Event::Mouse(MouseEvent{kind:MouseEventKind::Down(MouseButton::Right),column:x,row:y,modifiers:KeyModifiers::NONE}));
    let menu=draw(app,160,30);let at=locate(&menu,action.label());click(app,at);action_ready(app);
}
#[test]
fn tui_file_right_click_discard_is_modal_preserves_index_and_undo_survives_refresh(){
    use lenz::file_actions::FileAction;let(dir,mut app)=discard_app();let screen=draw(&mut app,160,30);click(&mut app,locate(&screen,"Side by side"));ready(&mut app);assert_eq!(app.session.markdown_mode(),lenz::markdown::PresentationMode::Rendered);let path=dir.path().join("file.txt");file_context(&mut app,FileAction::Discard);
    assert!(app.session.file_actions().confirmation().unwrap().contains("file.txt"));
    for (w,h)in[(30,8),(8,5),(1,1),(160,30)]{draw(&mut app,w,h);key(&mut app,KeyCode::End);key(&mut app,KeyCode::Up);app.handle(Event::Key(KeyEvent::new(KeyCode::Right,KeyModifiers::ALT|KeyModifiers::SHIFT)));click(&mut app,(0,0));assert_eq!(std::fs::read(&path).unwrap(),b"working\n");}
    key(&mut app,KeyCode::Esc);assert!(app.session.file_actions().confirmation().is_none());
    file_context(&mut app,FileAction::Discard);key(&mut app,KeyCode::Enter);action_ready(&mut app);ready(&mut app);
    assert_eq!(std::fs::read(&path).unwrap(),b"indexed\r\n");assert_eq!(lenz::git::run(dir.path(),&["show",":file.txt"]).unwrap(),b"indexed\r\n");assert_eq!(app.session.comparisons().len(),1);
    file_context(&mut app,FileAction::UndoDiscard);ready(&mut app);assert_eq!(std::fs::read(&path).unwrap(),b"working\n");
}
#[test]
fn tui_text_context_preserves_selection_and_never_leaks_menu_navigation(){
    let(_dir,mut app)=pair("old\n","new\n");ready(&mut app);draw(&mut app,160,30);
    let side=lenz::view::Side::Right;app.selection=Some(lenz::selection::Selection::all(side,app.session.document().unwrap().lines_on(side)));let selection=app.selection;
    app.handle(Event::Mouse(MouseEvent{kind:MouseEventKind::Down(MouseButton::Right),column:app.diff_area.x+app.diff_area.width/2+10,row:app.diff_area.y+2,modifiers:KeyModifiers::NONE}));let menu=draw(&mut app,160,30);assert!(text(&menu).contains("Copy selection"));assert!(locate(&menu,"All commands…").1.saturating_sub(locate(&menu,"Copy selection").1)<12);assert!(text(&menu).contains("All commands…"));assert!(!text(&menu).contains("Apply current change"),"outside-workspace text omits permanent read-only edits");
    let at=locate(&menu,"All commands…");click(&mut app,at);assert!(app.command_palette.is_some());assert!(text(&draw(&mut app,160,30)).contains("Commands:"));key(&mut app,KeyCode::Esc);
    app.handle(Event::Key(KeyEvent::new(KeyCode::F(10),KeyModifiers::SHIFT)));
    for code in [KeyCode::End,KeyCode::Up,KeyCode::Left,KeyCode::Right]{key(&mut app,code);assert_eq!(app.selection,selection);}
    key(&mut app,KeyCode::Esc);app.navigator.focused=true;app.handle(Event::Key(KeyEvent::new(KeyCode::F(10),KeyModifiers::SHIFT)));assert!(text(&draw(&mut app,160,30)).contains("Discard working-tree changes"));
}

#[test]
fn tui_commit_right_click_compares_with_working_tree_and_exposes_details(){
    let dir=tempfile::tempdir().unwrap();git(dir.path(),&["init","-q"]);std::fs::write(dir.path().join("file.txt"),"head\n").unwrap();git(dir.path(),&["add","."]);git(dir.path(),&["commit","-qm","Context commit"]);std::fs::write(dir.path().join("file.txt"),"working\n").unwrap();
    let request=lenz::cli::parse(&["log".into()],dir.path()).unwrap();let mut app=App::open(request,dir.path().into());ready(&mut app);let screen=draw(&mut app,160,40);let(x,y)=locate(&screen,"Context commit");
    app.handle(Event::Mouse(MouseEvent{kind:MouseEventKind::Down(MouseButton::Right),column:x,row:y,modifiers:KeyModifiers::NONE}));let screen=draw(&mut app,160,40);assert!(text(&screen).contains("Copy commit details"));click(&mut app,locate(&screen,"Compare with working tree"));ready(&mut app);
    assert!(matches!(app.session.spec(),lenz::changeset::Spec::Git{cached:false,revs,..}if revs.len()==1));assert_eq!(app.session.document().unwrap().text(lenz::view::Side::Right),"working\n");
}

#[test]
fn horizontal_wheel_and_keyboard_scroll_only_the_target_text_pane() {
    let (_dir,mut app)=pair(&format!("LEFT{}\n","x".repeat(300)),&format!("RIGHT{}\n","y".repeat(300)));
    app.navigator.visible=Some(false);draw(&mut app,100,20);
    let at=(app.diff_area.x+app.diff_area.width-10,app.diff_area.y+2);
    let anchor=app.session.view().anchor_line();
    app.handle(Event::Mouse(MouseEvent{kind:MouseEventKind::ScrollRight,column:at.0,row:at.1,modifiers:KeyModifiers::NONE}));
    let screen=text(&draw(&mut app,100,20));assert!(screen.contains("LEFT"));assert!(!screen.contains("RIGHT"),"right horizontal wheel must reach the diff");assert_eq!(app.session.view().anchor_line(),anchor);
    click(&mut app,at);let caret=app.selection;
    app.handle(Event::Key(KeyEvent::new(KeyCode::Right,KeyModifiers::CONTROL|KeyModifiers::ALT)));
    assert_eq!(app.selection,caret);
    app.handle(Event::Mouse(MouseEvent{kind:MouseEventKind::ScrollUp,column:at.0,row:at.1,modifiers:KeyModifiers::SHIFT}));
    assert_eq!(app.session.view().anchor_line(),anchor);
    assert!(text(&draw(&mut app,100,20)).contains("LEFT"));
}

#[test]
fn horizontal_scroll_clamps_wide_tabs_on_resize_and_stays_inside_modals(){
    let (dir,mut app)=pair("LEFT\n",&format!("{}END\n","界\t".repeat(20)));app.navigator.visible=Some(false);draw(&mut app,100,20);
    let at=(app.diff_area.right()-10,app.diff_area.y+2);
    for _ in 0..100{app.handle(Event::Mouse(MouseEvent{kind:MouseEventKind::ScrollRight,column:at.0,row:at.1,modifiers:KeyModifiers::NONE}));}
    let screen=text(&draw(&mut app,100,20));assert!(screen.contains("END"));assert!(screen.contains("LEFT"));let offsets=app.horizontal;
    app.handle(Event::Key(KeyEvent::new(KeyCode::F(10),KeyModifiers::SHIFT)));
    for modifiers in [KeyModifiers::NONE,KeyModifiers::SHIFT]{app.handle(Event::Mouse(MouseEvent{kind:MouseEventKind::ScrollLeft,column:at.0,row:at.1,modifiers}));}
    app.handle(Event::Key(KeyEvent::new(KeyCode::Right,KeyModifiers::CONTROL|KeyModifiers::ALT)));assert_eq!(app.horizontal,offsets);key(&mut app,KeyCode::Esc);
    draw(&mut app,400,20);assert_eq!(app.horizontal,[0;2]);
    app.focus=lenz::view::Side::Right;draw(&mut app,20,10);for _ in 0..100{app.handle(Event::Key(KeyEvent::new(KeyCode::Right,KeyModifiers::CONTROL|KeyModifiers::ALT)));}assert!(text(&draw(&mut app,20,10)).contains("END"));
    let at_end=app.horizontal[1];key(&mut app,KeyCode::Left);assert!(app.horizontal[1]<at_end);
    std::fs::write(dir.path().join("right.txt"),"short\n").unwrap();key(&mut app,KeyCode::F(5));ready(&mut app);draw(&mut app,100,20);assert_eq!(app.horizontal,[0;2]);
}

#[test]
fn pr_details_toolbar_sections_complete_comments_and_tiny_resize_keep_file_bytes(){
    use lenz::pr_session::{PrState,DetailsTab};
    let (dir,mut app)=pair("old text\n","new text\n");
    let req=lenz::cli::parse(&["--no-index".into(),"left.txt".into(),"right.txt".into()],dir.path()).unwrap();
    let mut repository=crate::repository::Repository::open(req,dir.path().into());
    let mut pr=lenz::pr::PullRequest{number:9,title:"PR description".into(),author:lenz::pr::Account{login:"author".into()},state:lenz::pr::State::Open,is_draft:false,body:String::new(),base_ref_name:"main".into(),head_ref_name:"feature".into(),head_ref_oid:"head".into(),url:"https://github.com/o/r/pull/9".into(),additions:1,deletions:0,changed_files:1,is_cross_repository:false,head_repository:None,head_repository_owner:None};
    pr.body="Description 漢字 é\n".repeat(100)+"DESCRIPTION_END";
    let mut state=PrState::new(pr,lenz::pr::Fetched{number:14130,merge_base:"base".into(),head:"head".into()});
    state.marks=None;
    state.seed_review(lenz::pr::Review{files:vec![],discussion:vec![lenz::pr::DiscussionComment{comment:lenz::pr::Comment{author:None,body:"Discussion 漢字\n".repeat(100)+"DISCUSSION_END",created_at:"2026-10-05".into(),..Default::default()},review_state:Some("APPROVED".into()),..Default::default()}],placed:Default::default(),listed:Default::default(),..Default::default()},&[]).unwrap();
    repository.core.pr=Some(state);app.repository=Some(repository);
    draw(&mut app,160,30);app.run_action(lenz::catalog::Action::Pr(lenz::catalog::PrAction::Details(DetailsTab::Conversation)));
    assert!(text(&draw(&mut app,160,30)).contains("Description 漢字"));
    key(&mut app,KeyCode::End);assert!(text(&draw(&mut app,160,30)).contains("DISCUSSION_END"),"the conversation, whole");
    {let pr=app.repository.as_mut().unwrap().core.pr.as_mut().unwrap();let mut review=pr.review.clone().unwrap();review.discussion[0].comment.body="REFRESHED_COMMENT".into();pr.seed_review(review,&[]).unwrap();}
    assert!(text(&draw(&mut app,160,30)).contains("REFRESHED_COMMENT"));
    let options=app.session.options();let focus=app.focus;
    for event in [Event::Key(KeyEvent::new(KeyCode::Char('i'),KeyModifiers::NONE)),Event::Key(KeyEvent::new(KeyCode::Char('z'),KeyModifiers::CONTROL)),Event::Key(KeyEvent::new(KeyCode::Right,KeyModifiers::CONTROL|KeyModifiers::ALT)),Event::Mouse(MouseEvent{kind:MouseEventKind::ScrollRight,column:155,row:27,modifiers:KeyModifiers::NONE})]{assert!(app.handle(event));}
    assert_eq!(app.session.options(),options);assert_eq!(app.focus,focus);
    for (w,h) in [(20,10),(0,0),(1,1),(8,3),(32,12),(160,30)]{app.handle(Event::Resize(w,h));draw(&mut app,w,h);key(&mut app,KeyCode::End);key(&mut app,KeyCode::Up);}
    key(&mut app,KeyCode::Char('2'));let commits=text(&draw(&mut app,160,30));assert!(commits.contains("wants to merge"),"{commits}");
    key(&mut app,KeyCode::Esc);assert!(!app.repository.as_ref().unwrap().modal());
    app.repository.as_mut().unwrap().core.execute(lenz::repository_session::RepositoryCommand::PrDetails(DetailsTab::Conversation));
    let screen=draw(&mut app,160,30);click(&mut app,locate(&screen,"Commits"));
    assert_eq!(app.repository.as_ref().unwrap().core.pr.as_ref().unwrap().details().unwrap().tab,DetailsTab::Commits);
    app.repository.as_mut().unwrap().core.execute(lenz::repository_session::RepositoryCommand::PrDetails(DetailsTab::Conversation));
    draw(&mut app,160,30);key(&mut app,KeyCode::End);
    for theme in lenz::theme::builtin_entries() {
        app.palette=theme.palette.clone();app.theme_revision+=1;
        let screen=draw(&mut app,160,30);
        assert!(text(&screen).contains("REFRESHED_COMMENT"),"{}",theme.id);
        let at=locate(&screen,"Conversation");
        assert_eq!(screen[at].bg,crate::chrome::color(theme.palette.selected_bg),"{}",theme.id);
    }
    // the toolbar's Files changed tab returns to the diff
    let screen=draw(&mut app,160,30);let at=locate(&screen,"Files changed");assert_eq!(at.1,0,"in the toolbar");
    click(&mut app,(at.0+1,at.1));assert!(app.repository.as_ref().unwrap().core.pr.as_ref().unwrap().details().is_none());
    assert_eq!(std::fs::read_to_string(dir.path().join("right.txt")).unwrap(),"new text\n");
}

#[test]
fn markdown_pending_and_ready_keyboard_history_guards_keep_source_undo() {
    use lenz::{commands::Command, markdown::PresentationMode, view::Side};
    let cwd=std::env::current_dir().unwrap();
    let dir=tempfile::tempdir_in(&cwd).unwrap();
    let (left,right)=(dir.path().join("left.md"),dir.path().join("right.md"));
    std::fs::write(&left,"**old**\n").unwrap();std::fs::write(&right,"**new**\n").unwrap();
    let req=lenz::cli::parse(&["--no-index".into(),left.display().to_string(),right.display().to_string()],&cwd).unwrap();
    let mut app=App::open(req,cwd);ready(&mut app);
    app.handle(Event::Key(KeyEvent::new(KeyCode::Right,KeyModifiers::ALT|KeyModifiers::SHIFT)));ready(&mut app);
    assert_eq!(std::fs::read_to_string(&right).unwrap(),"**new**\n");assert!(app.session.dirty());
    app.handle(Event::Key(KeyEvent::new(KeyCode::Char('s'),KeyModifiers::CONTROL)));ready(&mut app);
    assert_eq!(std::fs::read_to_string(&right).unwrap(),"**old**\n");assert!(app.session.applied()[0].active);
    app.session.execute(Command::SetPresentation(PresentationMode::Rendered));
    assert!(app.session.markdown_loading());
    for loaded in [false,true] {
        if loaded {ready(&mut app);}
        for (code,mods) in [(KeyCode::Char('z'),KeyModifiers::CONTROL),(KeyCode::Char('y'),KeyModifiers::CONTROL)] {
            app.handle(Event::Key(KeyEvent::new(code,mods)));
            assert_eq!(app.session.notice(),Some("Switch to Source to edit"));
            assert_eq!(std::fs::read_to_string(&right).unwrap(),"**old**\n");
            assert!(app.session.applied()[0].active);
        }
    }
    key(&mut app,KeyCode::Char('A'));
    assert!(text(&draw(&mut app,120,24)).contains("Switch to Source to edit"));
    key(&mut app,KeyCode::Enter);
    assert!(app.session.applied()[0].active);
    assert_eq!(app.session.markdown_mode(),PresentationMode::Rendered);
    app.session.execute(Command::SetPresentation(PresentationMode::Source));
    app.focus=Side::Right;
    app.handle(Event::Key(KeyEvent::new(KeyCode::Char('z'),KeyModifiers::CONTROL)));ready(&mut app);
    app.handle(Event::Key(KeyEvent::new(KeyCode::Char('s'),KeyModifiers::CONTROL)));ready(&mut app);
    assert_eq!(std::fs::read_to_string(&right).unwrap(),"**new**\n");
}

fn rendered_markdown(app: &mut App, w: u16, h: u16) -> Buffer {
    let buffer=draw(app,w,h);click(app,locate(&buffer,"Side by side"));ready(app);draw(app,w,h)
}
fn modified_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    assert!(app.handle(Event::Key(KeyEvent::new(code,modifiers))));
}
#[test]
fn markdown_tui_unicode_selection_copy_context_and_source_selection_are_separate() {
    use lenz::{view::Side, commands::Interaction};
    let (_dir,mut app)=pair_markdown("# Old\n", "# 安全 e\u{301} 👩‍💻\n\n**bold** [guide](target)\n");
    let original=lenz::selection::Selection::caret(Side::Left,lenz::selection::Pos::new(0,1));app.selection=Some(original);
    let screen=rendered_markdown(&mut app,160,36);let at=locate(&screen,"安全");click(&mut app,at);
    modified_key(&mut app,KeyCode::Right,KeyModifiers::SHIFT);
    assert_eq!(app.session.copy_text(Interaction::Copy,app.selection,app.focus).unwrap(),"安");
    assert!(app.session.copy_text(Interaction::CopyMarkdownSource,app.selection,app.focus).unwrap().starts_with("# 安全"));
    assert_eq!(app.selection,Some(original));
    app.handle(Event::Mouse(MouseEvent {kind:MouseEventKind::Drag(MouseButton::Left),column:at.0+8,row:at.1,modifiers:KeyModifiers::NONE}));
    let copied=app.session.copy_text(Interaction::Copy,app.selection,app.focus).unwrap();assert!(copied.starts_with("安全 e\u{301}"));
    modified_key(&mut app,KeyCode::Char('a'),KeyModifiers::CONTROL);
    let copied=app.session.copy_text(Interaction::Copy,app.selection,app.focus).unwrap();assert!(copied.contains("bold"));assert!(!copied.contains("**"));
    app.handle(Event::Mouse(MouseEvent {kind:MouseEventKind::Down(MouseButton::Right),column:at.0,row:at.1,modifiers:KeyModifiers::NONE}));
    let content=text(&draw(&mut app,160,36));assert!(content.contains("Show in source")&&!content.contains("Copy Markdown source"));key(&mut app,KeyCode::End);key(&mut app,KeyCode::Enter);assert!(app.command_palette.is_some());for ch in "Undo".chars(){key(&mut app,KeyCode::Char(ch));}assert!(text(&draw(&mut app,160,36)).contains("Switch to Source to edit"));
    key(&mut app,KeyCode::Esc);
    let screen=draw(&mut app,160,36);click(&mut app,locate(&screen,"Source"));
    assert_eq!(app.selection,Some(original));assert!(text(&draw(&mut app,160,36)).contains("# 安全"));
}
#[test]
fn markdown_tui_hidden_find_marker_preserves_source_selection_and_shows_source() {
    let (_dir,mut app)=pair_markdown("[guide](oldtarget)\n", "[guide](newtarget)\n");
    let selection=lenz::selection::Selection::caret(lenz::view::Side::Right,lenz::selection::Pos::new(0,2));app.selection=Some(selection);
    rendered_markdown(&mut app,160,36);
    key(&mut app,KeyCode::Char('/'));for c in "newtarget".chars(){key(&mut app,KeyCode::Char(c));}key(&mut app,KeyCode::Enter);
    assert_eq!(app.selection,Some(selection));
    let screen=draw(&mut app,160,36);click(&mut app,locate(&screen,"Hidden source match"));
    assert_eq!(app.session.markdown_mode(),lenz::markdown::PresentationMode::Source);
    assert!(text(&draw(&mut app,160,36)).contains("[guide](newtarget)"));
}
#[test]
fn markdown_tui_markup_marker_uses_original_hunk_anchor() {
    let (_dir,mut app)=pair_markdown("# Heading\n\n[guide](old)\n", "# Heading\n\n[guide](new)\n");
    let screen=rendered_markdown(&mut app,160,36);let projection=app.session.markdown_projection().unwrap();let anchor=projection.location(0,lenz::view::Side::Left).unwrap();
    click(&mut app,locate(&screen,"Markup change"));assert_eq!(app.session.markdown_mode(),lenz::markdown::PresentationMode::Source);assert_eq!(app.session.markdown_anchor(anchor.side),Some(anchor));
}
#[test]
fn markdown_tui_all_themes_and_tiny_sizes_keep_unicode_and_clamp_end_up() {
    let source=format!("# Heading\n\n> *quoted*\n\n- [x] item\n\n{}", "安全 café words wrapping here.\n\n".repeat(60));
    let (_dir,mut app)=pair_markdown(&source.replace("Heading","Old heading"),&source);rendered_markdown(&mut app,160,36);
    for entry in lenz::theme::builtin_entries() {
        app.palette=entry.palette;app.theme_revision+=1;
        for (w,h) in [(0,0),(1,1),(8,3),(32,12),(160,36)] {
            app.handle(Event::Resize(w,h));draw(&mut app,w,h);key(&mut app,KeyCode::End);draw(&mut app,w,h);key(&mut app,KeyCode::Up);let screen=draw(&mut app,w,h);
            if w==160 { assert!(text(&screen).contains("wrapping")); }
        }
        key(&mut app,KeyCode::Home);let screen=draw(&mut app,160,36);let at=locate(&screen,"Heading");assert!(screen[at].modifier.contains(ratatui::style::Modifier::BOLD));assert_eq!(screen[at].fg,crate::chrome::color(app.palette.accent));
    }
}
#[test]
fn markdown_tui_code_tables_keep_horizontal_input_and_pointer_side() {
    use lenz::view::Side;
    let src="```rust\nfn very_long_name() { let 安全 = 123456789; } // beyond viewport\n```\n\n| Name | Amount |\n| :--- | ---: |\n| a long column with 安全 | 12345678901234567890 |\n";
    let (_dir,mut app)=pair_markdown(&src.replace("123456789", "987654321"),src);let screen=rendered_markdown(&mut app,100,24);
    assert!(text(&screen).contains("fn very_long_name"));let old_focus=app.focus;
    app.handle(Event::Mouse(MouseEvent {kind:MouseEventKind::ScrollRight,column:app.diff_area.x+2,row:app.diff_area.y+2,modifiers:KeyModifiers::NONE}));
    assert!(app.horizontal[0]>0);assert_eq!(app.horizontal[1],0);assert_eq!(app.focus,old_focus);
    let screen=draw(&mut app,100,24);let left_cells=(0..50).map(|x|screen[(x,app.diff_area.y+2)].symbol()).collect::<String>();assert!(!left_cells.contains("fn very_long_name"));
    app.focus=Side::Right;modified_key(&mut app,KeyCode::Right,KeyModifiers::CONTROL|KeyModifiers::ALT);assert!(app.horizontal[1]>0);
    key(&mut app,KeyCode::End);draw(&mut app,100,24);key(&mut app,KeyCode::Up);assert!(text(&draw(&mut app,100,24)).contains("Amount"));
}
#[test]
fn markdown_tui_modal_suppression_blocks_selection_mode_and_writes() {
    let (dir,mut app)=pair_markdown("# Old\n", "# New\n");let screen=rendered_markdown(&mut app,160,36);
    let selection=app.selection;let rendered=app.session.markdown_selection();let at=locate(&screen,"Source");
    app.about=Some(Default::default());click(&mut app,at);modified_key(&mut app,KeyCode::Right,KeyModifiers::ALT|KeyModifiers::SHIFT);modified_key(&mut app,KeyCode::Char('z'),KeyModifiers::CONTROL);
    assert_eq!(app.selection,selection);assert_eq!(app.session.markdown_selection(),rendered);assert_eq!(app.session.markdown_mode(),lenz::markdown::PresentationMode::Rendered);assert_eq!(std::fs::read(dir.path().join("right.md")).unwrap(),b"# New\n");
    key(&mut app,KeyCode::Esc);assert!(app.about.is_none());
}
#[test]
fn markdown_tui_hidden_pane_keyboard_focus_and_wheel_do_not_leak() {
    use lenz::view::Side;
    let src="# Heading\n\nparagraph\n\n".repeat(70);let (_dir,mut app)=pair_markdown(&src.replace("paragraph", "old paragraph"),&src);rendered_markdown(&mut app,160,36);
    app.focus=Side::Left;modified_key(&mut app,KeyCode::Char('a'),KeyModifiers::CONTROL);let left=app.session.markdown_selection().unwrap();
    key(&mut app,KeyCode::Tab);assert_eq!(app.focus,Side::Right);draw(&mut app,32,12);modified_key(&mut app,KeyCode::Right,KeyModifiers::SHIFT);assert_eq!(app.session.markdown_selection().unwrap().side,Side::Right);assert_ne!(app.session.markdown_selection().unwrap(),left);
    key(&mut app,KeyCode::Esc);app.session.execute(lenz::commands::Command::SetScroll(lenz::view::ScrollMode::Aligned));draw(&mut app,160,36);
    let left=app.session.markdown_anchor(Side::Left);
    app.handle(Event::Mouse(MouseEvent {kind:MouseEventKind::ScrollDown,column:app.diff_area.right()-2,row:app.diff_area.y+2,modifiers:KeyModifiers::NONE}));draw(&mut app,160,36);assert_eq!(app.session.markdown_anchor(Side::Left),left);
}

#[test]
fn markdown_tui_native_quote_rule_list_and_table_geometry() {
    let src="> quoted words\n\n- outer\n  - inner\n\n---\n\n| Name | Value |\n| :--- | ---: |\n| wide name | 7 |\n";
    let (_dir,mut app)=pair_markdown(&src.replace("quoted", "old quote"),src);let screen=rendered_markdown(&mut app,160,36);
    let quote=locate(&screen,"quoted");assert_eq!(screen[(quote.0-2,quote.1)].symbol(),"│");
    let outer=locate(&screen,"outer");let inner=locate(&screen,"inner");assert!(inner.0>outer.0);
    assert!(text(&screen).contains("────────"));
    let name=locate(&screen,"Name");let value=locate(&screen,"Value");let seven=locate(&screen,"7");assert_eq!(seven.0+1,value.0+5);assert!(text(&screen).contains("│"));assert!(value.0>name.0);
}

#[test]
fn markdown_tui_source_toolbar_and_command_menu_restore_scrolled_source_anchor() {
    use lenz::{view::Side, markdown::PresentationMode};
    let src=(0..90).map(|i|format!("# Section {i}\n\nparagraph {i}\n\n")).collect::<String>();
    let (_dir,mut app)=pair_markdown(&src.replace("paragraph", "old words"),&src);
    let selection=lenz::selection::Selection::caret(Side::Right,lenz::selection::Pos::new(0,2));app.selection=Some(selection);
    rendered_markdown(&mut app,160,36);
    for command_menu in [false,true] {
        draw(&mut app,160,36);key(&mut app,KeyCode::End);key(&mut app,KeyCode::Up);let screen=draw(&mut app,160,36);
        let anchor=app.session.markdown_anchor(Side::Right).unwrap();let line=app.session.markdown_projection().unwrap().pair.sides[1].source_pos(anchor.byte).unwrap().line;
        if command_menu {key(&mut app,KeyCode::Char(':'));for c in "view: source".chars(){key(&mut app,KeyCode::Char(c));}key(&mut app,KeyCode::Enter);}else{click(&mut app,locate(&screen,"Source"));}
        assert_eq!(app.session.markdown_mode(),PresentationMode::Source);assert_eq!(app.selection,Some(selection));assert!(app.session.view().line_visible(Side::Right,line));
        let screen=draw(&mut app,160,36);click(&mut app,locate(&screen,"Side by side"));ready(&mut app);
    }
}
#[test]
fn markdown_tui_go_to_and_find_do_not_replace_source_selection() {
    let src="# First\n\n**target**\n\n# Last\n";
    let (_dir,mut app)=pair_markdown(&src.replace("First","Old"),src);
    let selection=lenz::selection::Selection::caret(lenz::view::Side::Right,lenz::selection::Pos::new(0,2));app.selection=Some(selection);rendered_markdown(&mut app,160,36);
    key(&mut app,KeyCode::Char('g'));key(&mut app,KeyCode::Char('5'));key(&mut app,KeyCode::Enter);assert_eq!(app.selection,Some(selection));assert!(text(&draw(&mut app,160,36)).contains("Last"));
    key(&mut app,KeyCode::Char('/'));for c in "target".chars(){key(&mut app,KeyCode::Char(c));}key(&mut app,KeyCode::Enter);assert_eq!(app.selection,Some(selection));
    let screen=draw(&mut app,160,36);assert!(screen.content.iter().any(|c|c.symbol()=="t"&&c.bg==crate::chrome::color(app.palette.found_bg)));key(&mut app,KeyCode::F(3));assert_eq!(app.selection,Some(selection));
}

#[test]
fn markdown_tui_keyboard_caret_moves_through_wrapped_native_rows() {
    let src=format!("{}\n", "one two three 安全 e\u{301} words ".repeat(25));let (_dir,mut app)=pair_markdown(&src.replace("one","old"),&src);rendered_markdown(&mut app,160,36);key(&mut app,KeyCode::Tab);app.focus=lenz::view::Side::Right;
    let screen=draw(&mut app,32,12);let at=locate(&screen,"one two");click(&mut app,at);let initial=app.session.markdown_selection().unwrap().head;
    key(&mut app,KeyCode::Down);let selection=app.session.markdown_selection().unwrap();assert!(selection.head>initial&&selection.head<100,"{selection:?}");
    key(&mut app,KeyCode::Up);assert_eq!(app.session.markdown_selection().unwrap().head,initial);
    let screen=draw(&mut app,32,12);assert!(screen.content.iter().any(|c|c.modifier.contains(ratatui::style::Modifier::REVERSED)));
}

#[test]
fn markdown_tui_pending_render_request_blocks_source_caret_and_write_shortcuts() {
    let (dir,mut app)=pair_markdown("# Old\n", "# New\n");let screen=draw(&mut app,160,36);
    let selection=lenz::selection::Selection::caret(lenz::view::Side::Right,lenz::selection::Pos::new(0,2));app.selection=Some(selection);
    click(&mut app,locate(&screen,"Side by side"));assert!(app.session.markdown_loading());
    let at=(app.diff_area.right()-4,app.diff_area.y+2);click(&mut app,at);modified_key(&mut app,KeyCode::Down,KeyModifiers::SHIFT);modified_key(&mut app,KeyCode::Char('a'),KeyModifiers::CONTROL);
    assert_eq!(app.selection,Some(selection));modified_key(&mut app,KeyCode::Right,KeyModifiers::ALT|KeyModifiers::SHIFT);modified_key(&mut app,KeyCode::Char('z'),KeyModifiers::CONTROL);assert_eq!(std::fs::read(dir.path().join("right.md")).unwrap(),b"# New\n");
    click(&mut app,locate(&screen,"Source"));ready(&mut app);assert_eq!(app.session.markdown_mode(),lenz::markdown::PresentationMode::Source);
}

#[test]
fn markdown_tui_right_click_targets_pointer_pane_and_end_caret_is_visible() {
    let (_dir,mut app)=pair_markdown("# Old\n", "# New\n");let screen=rendered_markdown(&mut app,160,36);let right=locate(&screen,"New");click(&mut app,right);key(&mut app,KeyCode::End);
    let screen=draw(&mut app,160,36);assert!(text(&screen).contains("New▏"));
    let left=locate(&screen,"Old");app.handle(Event::Mouse(MouseEvent {kind:MouseEventKind::Down(MouseButton::Right),column:left.0,row:left.1,modifiers:KeyModifiers::NONE}));assert_eq!(app.focus,lenz::view::Side::Left);assert_eq!(app.session.markdown_selection().unwrap().side,lenz::view::Side::Left);assert_eq!(app.session.markdown_mode(),lenz::markdown::PresentationMode::Rendered);
    key(&mut app,KeyCode::Esc);
}

#[test]
fn markdown_tui_controls_follow_core_eligibility_without_changing_ordinary_source_chrome() {
    let (_dir,mut app)=pair("old\n","new\n");let screen=draw(&mut app,160,36);assert!(!text(&screen).contains("Preview"));assert!(!text(&screen).contains("Source"));
    // the shared catalog lists no Markdown views for a plain comparison
    key(&mut app,KeyCode::Char(':'));for c in "Inline".chars(){key(&mut app,KeyCode::Char(c));}let content=text(&draw(&mut app,160,36));assert!(content.contains("No matching commands"),"{content}");key(&mut app,KeyCode::Enter);key(&mut app,KeyCode::Esc);assert_eq!(app.session.markdown_mode(),lenz::markdown::PresentationMode::Source);assert!(app.session.markdown_projection().is_none());
}

#[test]
fn markdown_tui_source_switch_restores_visible_line_inside_long_code_block() {
    use lenz::view::Side;
    let src = format!(
        "```text\n{}```\n",
        (0..240)
            .map(|i| format!("line_{i:03}_visible\n"))
            .collect::<String>()
    );
    let (_dir, mut app) = pair_markdown(&src.replace("line_000", "old_000"), &src);
    let original = lenz::selection::Selection::caret(Side::Right, lenz::selection::Pos::new(0, 2));
    app.selection = Some(original);
    rendered_markdown(&mut app, 160, 36);
    key(&mut app, KeyCode::End);
    key(&mut app, KeyCode::Up);
    let screen = draw(&mut app, 160, 36);
    let x = app.diff_area.x + app.diff_area.width / 2 + 3; // past the change-mark margin
    let y = app.diff_area.y + 2;
    let visible = (x..screen.area.width)
        .map(|x| screen[(x, y)].symbol())
        .collect::<String>();
    let number = visible.strip_prefix("line_").unwrap()[..3]
        .parse::<u32>()
        .unwrap();
    assert!(number > 180);
    click(&mut app, locate(&screen, "Source"));
    assert!(
        app.session.view().line_visible(Side::Right, number + 1),
        "source viewport must contain formerly visible code line {number}"
    );
    assert_eq!(app.selection, Some(original));
}
#[test]
fn markdown_tui_quoted_code_horizontal_maximum_reveals_unique_suffix() {
    let line = format!("{}UNIQUE_END", "a".repeat(100));
    let src = format!("> ```text\n> {line}\n> ```\n");
    let (_dir, mut app) = pair_markdown(&src.replace("aaa", "bbb"), &src);
    rendered_markdown(&mut app, 160, 36);
    let at = (app.diff_area.right() - 2, app.diff_area.y + 2);
    for _ in 0..100 {
        app.handle(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollRight,
            column: at.0,
            row: at.1,
            modifiers: KeyModifiers::NONE,
        }));
    }
    let screen = draw(&mut app, 160, 36);
    assert!(text(&screen).contains("UNIQUE_END"));
}
#[test]
fn markdown_tui_wrapped_end_home_keep_visual_row_affinity() {
    let src = format!("{}\n", "one two three 安全 e\u{301} words ".repeat(30));
    let (_dir, mut app) = pair_markdown(&src.replace("one", "old"), &src);
    rendered_markdown(&mut app, 160, 36);
    app.focus = lenz::view::Side::Right;
    let screen = draw(&mut app, 32, 12);
    let at = locate(&screen, "one two");
    click(&mut app, at);
    let initial = app.session.markdown_selection().unwrap().head;
    key(&mut app, KeyCode::End);
    let end = app.session.markdown_selection().unwrap();
    key(&mut app, KeyCode::End);
    assert_eq!(
        app.session.markdown_selection(),
        Some(end),
        "repeated End must stay on the displayed row"
    );
    let screen = draw(&mut app, 32, 12);
    let caret_rows = (0..screen.area.height)
        .filter(|&y| {
            (0..screen.area.width).any(|x| {
                // a caret on a cell, or after a row's last cell (word wrap leaves room there)
                screen[(x, y)]
                    .modifier
                    .contains(ratatui::style::Modifier::REVERSED) || screen[(x, y)].symbol() == "▏"
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(caret_rows, vec![at.1]);
    key(&mut app, KeyCode::Home);
    assert_eq!(app.session.markdown_selection().unwrap().head, initial);
    modified_key(&mut app, KeyCode::End, KeyModifiers::SHIFT);
    let selected = app.session.markdown_selection().unwrap();
    assert_eq!(selected.head, end.head);
    assert_eq!(
        app.session
            .copy_text(lenz::commands::Interaction::Copy, app.selection, app.focus)
            .unwrap(),
        app.session.markdown_projection().unwrap().pair.sides[1].text[initial..end.head]
    );
    key(&mut app, KeyCode::Right);
    let screen = draw(&mut app, 32, 12);
    assert!((0..screen.area.width).any(|x| {
        screen[(x, at.1 + 1)]
            .modifier
            .contains(ratatui::style::Modifier::REVERSED)
    }));
    let bottom = app.diff_area.bottom() - 3;
    let point = (app.diff_area.x, bottom);
    click(&mut app, point);
    let start = app.session.markdown_selection().unwrap().head;
    let before = draw(&mut app, 32, 12);
    let top_text = (0..before.area.width)
        .map(|x| before[(x, at.1)].symbol())
        .collect::<String>();
    key(&mut app, KeyCode::End);
    let tail = app.session.markdown_selection();
    key(&mut app, KeyCode::End);
    assert_eq!(app.session.markdown_selection(), tail);
    let after = draw(&mut app, 32, 12);
    assert_eq!(
        (0..after.area.width)
            .map(|x| after[(x, at.1)].symbol())
            .collect::<String>(),
        top_text
    );
    assert!((0..after.area.width).any(|x| {
        after[(x, bottom)]
            .modifier
            .contains(ratatui::style::Modifier::REVERSED) || after[(x, bottom)].symbol() == "▏"
    }));
    key(&mut app, KeyCode::Home);
    assert_eq!(app.session.markdown_selection().unwrap().head, start);
}

#[test]
fn markdown_tui_repository_log_keeps_preference_and_added_deleted_source_hunks() {
    use lenz::{markdown::PresentationMode, repository_session::{Pane, RepositoryCommand}};
    let dir=tempfile::tempdir().unwrap();let d=dir.path();git(d,&["init","-q","-b","main"]);
    std::fs::write(d.join("gone.md"),"# Deleted\n").unwrap();
    std::fs::write(d.join("ref.md"),"[guide][id]\n\n[id]: old\n").unwrap();
    git(d,&["add","."]);git(d,&["commit","-qm","Before Markdown"]);
    std::fs::remove_file(d.join("gone.md")).unwrap();std::fs::write(d.join("new.md"),"# Added\n").unwrap();
    std::fs::write(d.join("ref.md"),"[guide][id]\n\n[id]: new\n").unwrap();
    git(d,&["add","-A"]);git(d,&["commit","-qm","After Markdown"]);
    let before=std::fs::read(d.join("ref.md")).unwrap();let source_hash_before=markdown_repository_source_hash(d);
    let request=lenz::cli::parse(&["log".into()],d).unwrap();let mut app=App::open(request,d.into());ready(&mut app);
    app.session.execute(lenz::commands::Command::SetMarkdownLayout(lenz::markdown::MarkdownLayout::SideBySide));
    assert_eq!(app.session.markdown_mode(),PresentationMode::Source);
    for name in ["gone.md","new.md","ref.md"] {
        let screen=draw(&mut app,180,40);click(&mut app,locate(&screen,name));ready(&mut app);
        let screen=draw(&mut app,180,40);click(&mut app,locate(&screen,"Side by side"));ready(&mut app);
        let screen=draw(&mut app,180,40);
        assert!(text(&screen).contains(if name=="gone.md" {"Deleted"}else if name=="new.md"{"Added"}else{"guide"}));
        let projection=app.session.markdown_projection().unwrap();
        for id in 0..app.session.view().change_count(){assert!(projection.represents_hunk(id));}
        for id in 0..app.session.view().change_count(){key(&mut app,KeyCode::Char('n'));assert_eq!(app.session.view().current_change(),Some(id),"hunk {id}");}
        if name=="ref.md" {
            // Leave a Source caret at the changed definition, then inspect the unchanged rendered label.
            let screen=draw(&mut app,180,40);click(&mut app,locate(&screen,"Source"));let screen=draw(&mut app,180,40);let (x,y)=locate(&screen,"[id]: new");click(&mut app,(x+2,y));key(&mut app,KeyCode::Char('b'));ready(&mut app);
            let expected=app.visual.annotation(lenz::view::Side::Right,0).unwrap().commit.sha.clone();
            assert_ne!(expected,app.visual.annotation(lenz::view::Side::Right,2).unwrap().commit.sha);
            let screen=draw(&mut app,180,40);click(&mut app,locate(&screen,"Side by side"));ready(&mut app);let screen=draw(&mut app,180,40);let (x,y)=locate(&screen,"guide");let right=x+app.diff_area.width/2+1;click(&mut app,(right+1,y));assert_eq!(app.focus,lenz::view::Side::Right);key(&mut app,KeyCode::Char('B'));ready(&mut app);assert!(app.visual.commit_visible());
            assert!(matches!(app.visual.commit_key(KeyEvent::new(KeyCode::Char('c'),KeyModifiers::NONE)),Some(crate::visual::BlameEffect::Copy(sha)) if sha==expected));key(&mut app,KeyCode::Esc);
            let screen=draw(&mut app,180,40);let a=app.session.markdown_projection().unwrap().location(0,lenz::view::Side::Left).unwrap();click(&mut app,locate(&screen,"Markup change"));assert_eq!(app.session.markdown_anchor(a.side),Some(a));let screen=draw(&mut app,180,40);click(&mut app,locate(&screen,"Side by side"));ready(&mut app);
        }
    }
    app.repository.as_mut().unwrap().core.execute(RepositoryCommand::Focus(Pane::Log));
    key(&mut app,KeyCode::Down);ready(&mut app);
    assert_eq!(app.session.markdown_mode(),PresentationMode::Rendered,"repository picks retain comparison preference");
    let source_hash_after=markdown_repository_source_hash(d);assert_eq!(source_hash_before,source_hash_after);
    assert_eq!(std::fs::read(d.join("ref.md")).unwrap(),before);
}

#[test]
fn markdown_tui_literal_math_and_mapped_editor_use_actual_input() {
    let (dir,mut app)=pair_markdown("# Old\n","# 漢字\n\n$**bold**$ and $$*math*$$\n\nentity &amp;\n");
    let screen=draw(&mut app,180,40);click(&mut app,locate(&screen,"Side by side"));ready(&mut app);let screen=draw(&mut app,180,40);
    assert!(text(&screen).contains("$**bold**$ and $$*math*$$"));
    let at=locate(&screen,"漢字");click(&mut app,at);key(&mut app,KeyCode::Char('o'));
    assert_eq!(app.inspected_position,Some((0,lenz::view::Side::Right,1,3)));
    let (x,y)=locate(&screen,"entity &");click(&mut app,(x+7,y));key(&mut app,KeyCode::Char('o'));
    assert_eq!(app.session.notice(),Some("Show in source to inspect this position"));
    let source=draw(&mut app,180,40);click(&mut app,locate(&source,"Source"));
    assert_eq!(app.session.markdown_anchor(lenz::view::Side::Right).unwrap().byte,"# 漢字\n\n$**bold**$ and $$*math*$$\n\nentity ".len());
    assert_eq!(std::fs::read_to_string(dir.path().join("right.md")).unwrap(),"# 漢字\n\n$**bold**$ and $$*math*$$\n\nentity &amp;\n");
}

#[test]
fn source_load_more_branches_click_reads_the_next_page() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"]);
    std::fs::write(root.join("file.txt"), "content\n").unwrap();
    git(root, &["add", "."]); git(root, &["commit", "-qm", "Initial"]);
    for i in 0..44 { git(root, &["branch", &format!("topic-{i:02}")]); }
    let req = lenz::cli::parse(&["log".into()], root).unwrap();
    let mut app = App::open(req, root.into());
    ready(&mut app);
    assert_eq!(app.repository.as_ref().unwrap().core.picker.sources.branches.len(), 20);
    let screen = draw(&mut app, 180, 50);
    click(&mut app, locate(&screen, "Load more branches"));
    ready(&mut app);
    assert_eq!(app.repository.as_ref().unwrap().core.picker.sources.branches.len(), 40);
}

#[cfg(feature="gui-test")]
#[test]
fn pr_click_shows_a_persistent_loader_in_dark_light_and_narrow_layouts() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"]);
    std::fs::write(root.join("ref.md"), "# Before\n").unwrap();
    git(root, &["add", "."]); git(root, &["commit", "-qm", "Before"]);
    std::fs::write(root.join("ref.md"), "# After\n").unwrap();
    git(root, &["commit", "-qam", "After"]);
    let _provider = lenz::pr::test_support::markdown_provider(root);
    let req = lenz::cli::parse(&["log".into()], root).unwrap();
    let mut app = App::open(req, root.into());
    ready(&mut app);
    let before = std::fs::read(root.join("ref.md")).unwrap();
    let old = app.session.document().map(|d| d.text(lenz::view::Side::Right).to_string());
    let screen = draw(&mut app, 180, 40);
    click(&mut app, locate(&screen, "Markdown PR"));
    app.session.set_notice("");
    for (width, height, theme) in [(180, 40, "darcula"), (80, 24, "light"), (32, 12, "light")] {
        app.palette = std::sync::Arc::new(*lenz::theme::builtin(theme).unwrap());
        let screen = draw(&mut app, width, height);
        let rendered = text(&screen);
        assert!(rendered.contains("Loading pull request"), "{width}×{height}: {rendered}");
        assert_eq!(app.session.document().map(|d| d.text(lenz::view::Side::Right).to_string()), old);
    }
    let screen = draw(&mut app, 180, 40);
    click(&mut app, locate(&screen, "Local changes"));
    let cancelled = text(&draw(&mut app, 180, 40));
    assert!(!cancelled.contains("Loading pull request") && !cancelled.contains("Fetching pull request"), "another source clears PR progress feedback");
    ready(&mut app);
    assert!(app.repository.as_ref().unwrap().core.pr.is_none(), "a stale PR completion must stay cancelled");
    let screen = draw(&mut app, 180, 40);
    click(&mut app, locate(&screen, "Markdown PR"));
    ready(&mut app);
    assert!(!text(&draw(&mut app, 180, 40)).contains("Loading pull request"));
    assert_eq!(app.repository.as_ref().unwrap().core.pr.as_ref().unwrap().pr.number, 9);
    assert!(!app.repository.as_ref().unwrap().core.picker.shown);
    key(&mut app, KeyCode::F(10));
    for ch in "Refresh pull request".chars() { key(&mut app, KeyCode::Char(ch)); }
    key(&mut app, KeyCode::Enter);
    assert!(text(&draw(&mut app, 32, 12)).contains("Loading pull request"), "refreshing in Focus also shows the loader");
    ready(&mut app);
    assert!(!text(&draw(&mut app, 32, 12)).contains("Loading pull request"));
    assert_eq!(std::fs::read(root.join("ref.md")).unwrap(), before);
}

#[cfg(feature="gui-test")]
#[test]
fn markdown_tui_offline_pr_real_prompt_and_source_hunks() {
    let dir=tempfile::tempdir().unwrap();let d=dir.path();git(d,&["init","-q","-b","main"]);
    std::fs::write(d.join("gone.md"),"# Deleted\n").unwrap();std::fs::write(d.join("ref.md"),"[guide][id]\n\n[id]: old\n").unwrap();git(d,&["add","."]);git(d,&["commit","-qm","Before Markdown"]);
    std::fs::remove_file(d.join("gone.md")).unwrap();std::fs::write(d.join("new.md"),"# Added\n").unwrap();std::fs::write(d.join("ref.md"),"[guide][id]\n\n[id]: new\n").unwrap();git(d,&["add","-A"]);git(d,&["commit","-qm","After Markdown"]);
    let provider=lenz::pr::test_support::markdown_provider(d);let before=std::fs::read(d.join("ref.md")).unwrap();let source_hash_before=markdown_repository_source_hash(d);let req=lenz::cli::parse(&["log".into()],d).unwrap();let mut app=App::open(req,d.into());ready(&mut app);
    let screen=draw(&mut app,180,40);click(&mut app,locate(&screen,"Repository"));
    key(&mut app,KeyCode::Char('P'));assert!(app.repository.as_ref().unwrap().modal());key(&mut app,KeyCode::Char('9'));key(&mut app,KeyCode::Enter);ready(&mut app);
    let core=&app.repository.as_ref().unwrap().core;
    assert!(core.pr.is_some(),"offline PR did not open: {:?}; {:?}",core.notice,app.session.notice());
    assert_eq!(core.pr.as_ref().unwrap().pr.number,9);
    let screen=draw(&mut app,180,40);click(&mut app,locate(&screen,"ref.md"));ready(&mut app);let screen=draw(&mut app,180,40);click(&mut app,locate(&screen,"Side by side"));ready(&mut app);
    let projection=app.session.markdown_projection().unwrap();for id in 0..app.session.view().change_count(){assert!(projection.represents_hunk(id));}let anchor=projection.location(0,lenz::view::Side::Left).unwrap();
    key(&mut app,KeyCode::Char('n'));assert_eq!(app.session.view().current_change(),Some(0));let screen=draw(&mut app,180,40);click(&mut app,locate(&screen,"Markup change"));assert_eq!(app.session.markdown_anchor(anchor.side),Some(anchor));
    let screen=draw(&mut app,180,40);click(&mut app,locate(&screen,"Side by side"));ready(&mut app);key(&mut app,KeyCode::Char('d'));key(&mut app,KeyCode::Char(':'));key(&mut app,KeyCode::Esc);
    let source_hash_after=markdown_repository_source_hash(d);assert_eq!(source_hash_before,source_hash_after);
    assert_eq!(std::fs::read(d.join("ref.md")).unwrap(),before);let calls=std::fs::read_to_string(d.join(".git/markdown-provider/requests.log")).unwrap();assert!(calls.contains("pr view 9")&&calls.contains("api graphql"));drop(provider);
}

#[test]
fn markdown_tui_real_file_switch_reload_clears_stale_selection_and_layout() {
    use lenz::markdown::PresentationMode;
    let dir=tempfile::tempdir().unwrap();let left=dir.path().join("left");let right=dir.path().join("right");std::fs::create_dir(&left).unwrap();std::fs::create_dir(&right).unwrap();
    for name in ["a.md","b.txt","c.md"]{std::fs::write(left.join(name),"# Old\n").unwrap();std::fs::write(right.join(name),"# New\n\n**words**\n".repeat(100)).unwrap();}
    let req=lenz::cli::parse(&["--no-index".into(),"left".into(),"right".into()],dir.path()).unwrap();let mut app=App::open(req,dir.path().into());ready(&mut app);let screen=draw(&mut app,180,40);click(&mut app,locate(&screen,"Side by side"));ready(&mut app);let screen=draw(&mut app,180,40);click(&mut app,locate(&screen,"New"));
    let pair=std::sync::Arc::downgrade(&app.session.markdown_projection().unwrap().pair);assert!(app.session.markdown_selection().is_some());
    key(&mut app,KeyCode::Char(']'));assert!(app.session.markdown_selection().is_none());key(&mut app,KeyCode::Char(']'));ready(&mut app);draw(&mut app,180,40);
    assert_eq!(app.session.current(),2);assert_eq!(app.session.markdown_mode(),PresentationMode::Rendered);assert!(pair.upgrade().is_none());
    std::fs::write(right.join("c.md"),"# Tiny\n").unwrap();key(&mut app,KeyCode::F(5));ready(&mut app);let screen=draw(&mut app,32,12);key(&mut app,KeyCode::End);key(&mut app,KeyCode::Up);assert!(text(&screen).contains("Tiny"));
    assert_eq!(app.session.markdown_projection().unwrap().pair.sides[1].source.as_ref(),"# Tiny\n");assert_eq!(std::fs::read(right.join("c.md")).unwrap(),b"# Tiny\n");
}

#[test]
fn markdown_tui_status_distinguishes_pending_complete_source_and_plain_fallback() {
    let (_dir,mut app)=pair_markdown("# Old\n","# New\n\n```rust\nfn main() {}\n```\n");
    let screen=draw(&mut app,180,40);click(&mut app,locate(&screen,"Side by side"));
    let pending=text(&draw(&mut app,180,40));assert!(pending.contains("Preparing rendered Markdown"));assert!(!pending.contains("Rendered Markdown"));
    ready(&mut app);draw(&mut app,180,40);ready(&mut app);
    let complete=text(&draw(&mut app,180,40));assert!(complete.contains("Rendered Markdown"));assert!(!complete.contains("Preparing rendered Markdown"));
    app.session.set_notice("Retained note");assert!(text(&draw(&mut app,180,40)).contains("Rendered Markdown · Retained note"));
    let screen=draw(&mut app,180,40);click(&mut app,locate(&screen,"Source"));let source=text(&draw(&mut app,180,40));assert!(!source.contains("Rendered Markdown")&&!source.contains("Preparing rendered Markdown"));
    let (_dir,mut plain)=pair("old\n","new\n");key(&mut plain,KeyCode::Char(':'));for c in "Inline".chars(){key(&mut plain,KeyCode::Char(c));}key(&mut plain,KeyCode::Enter);key(&mut plain,KeyCode::Esc);let source=text(&draw(&mut plain,180,40));assert_eq!(plain.session.markdown_mode(),lenz::markdown::PresentationMode::Source);assert!(!source.contains("Rendered Markdown")&&!source.contains("Preparing rendered Markdown"));
}

#[test]
fn markdown_tui_actual_empty_constructor_lists_all_commands_and_stays_source() {
    let dir=tempfile::tempdir().unwrap();let req=lenz::cli::parse(&["--about".into()],dir.path()).unwrap();let mut app=App::open(req,dir.path().into());ready(&mut app);key(&mut app,KeyCode::Esc);draw(&mut app,32,12);key(&mut app,KeyCode::Char(':'));
    let rows=app.command_palette.as_ref().unwrap().rows().map(|r|r.entry.action).collect::<Vec<_>>();
    for command in lenz::commands::ALL.iter().filter(|c|!matches!(c,lenz::commands::Command::CycleScroll|lenz::commands::Command::CycleWhitespace|lenz::commands::Command::CycleHighlight)) {assert!(rows.contains(&lenz::catalog::Action::Comparison(*command)),"{command:?}");}
    assert_eq!(app.session.disabled(lenz::commands::Command::SetPresentation(lenz::markdown::PresentationMode::Rendered)),Some("Comparison loading"));
    key(&mut app,KeyCode::Esc);modified_key(&mut app,KeyCode::Char('z'),KeyModifiers::CONTROL);key(&mut app,KeyCode::F(5));ready(&mut app);draw(&mut app,1,1);
    assert!(app.session.document().is_none()&&!app.session.markdown_loading()&&app.session.markdown_projection().is_none());
}

fn markdown_right_text(app: &App, screen: &Buffer) -> String {
    let x=if app.diff_area.width<70 {app.diff_area.x}else{app.diff_area.x+app.diff_area.width/2+1};
    (app.diff_area.y+2..app.diff_area.bottom().saturating_sub(2)).flat_map(|y|(x..app.diff_area.right()).map(move |x|screen[(x,y)].symbol())).collect()
}

#[test]
fn markdown_final_navigation_reveals_exact_hunks_inside_one_paragraph() {
    let left=(0..120).map(|i|format!("LINE_{i:03} α界 original words extend across the row here\n")).collect::<String>();
    let right=left.replace("LINE_003 α界 original", "RC003 α界 changed").replace("LINE_070 α界 original", "RC070 α界 changed");
    let (_dir,mut app)=pair_markdown(&left,&right);
    let screen=draw(&mut app,120,36);click(&mut app,locate(&screen,"Side by side"));ready(&mut app);
    assert_eq!(app.session.view().change_count(),2);
    for (id,label) in [(0,"RC003"),(1,"RC070")] {
        key(&mut app,KeyCode::Char(if id==0 {'p'}else{'n'}));let screen=draw(&mut app,120,36);
        assert_eq!(app.session.view().current_change(),Some(id));
        assert!(text(&screen).contains(label),"hunk {id}: {}",text(&screen));
        let (x,y)=locate(&screen,label);assert!(screen[(x,y)].fg!=screen[(x,y+1)].fg||screen[(x,y)].modifier.contains(ratatui::style::Modifier::UNDERLINED),"visible changed text must be decorated");
    }
    key(&mut app,KeyCode::Char('p'));assert_eq!(app.session.view().current_change(),Some(0));assert!(text(&draw(&mut app,120,36)).contains("RC003"));
}

#[test]
fn markdown_final_source_viewport_and_reflow_preserve_identifiable_content() {
    fn key(app:&mut App,code:KeyCode){app.handle(Event::Key(KeyEvent::new(code,KeyModifiers::NONE)));}
    let source=(0..500).map(|i|format!("Paragraph_{i:03} α界 words words words words words words words words words words\n\n")).collect::<String>();
    let (_dir,mut app)=pair_markdown(&source.replace("Paragraph_000","Old_000"),&source);
    draw(&mut app,120,36);
    key(&mut app,KeyCode::Char('g'));for c in "501".chars(){key(&mut app,KeyCode::Char(c));}key(&mut app,KeyCode::Enter);
    let screen=draw(&mut app,120,36);assert!(text(&screen).contains("Paragraph_250"));
    let source_state=(app.session.view().current_change(),app.session.view().top_line_on(lenz::view::Side::Right),app.selection);
    click(&mut app,locate(&screen,"Side by side"));ready(&mut app);
    assert_eq!((app.session.view().current_change(),app.session.view().top_line_on(lenz::view::Side::Right),app.selection),source_state);
    assert!(text(&draw(&mut app,120,36)).contains("Paragraph_250"),"cold entry must use Source position");
    for width in [200,80,120] {assert!(text(&draw(&mut app,width,36)).contains("Paragraph_250"),"reflow width {width}");}
    key(&mut app,KeyCode::End);let screen=draw(&mut app,120,36);click(&mut app,locate(&screen,"Source"));modified_key(&mut app,KeyCode::Home,KeyModifiers::CONTROL);
    let screen=draw(&mut app,120,36);assert!(text(&screen).contains("Paragraph_000"));
    key(&mut app,KeyCode::Char(':'));key(&mut app,KeyCode::Home);key(&mut app,KeyCode::Down);key(&mut app,KeyCode::Enter);ready(&mut app);
    assert!(text(&draw(&mut app,120,36)).contains("Paragraph_000"),"warm entry must replace retained tail anchor");
}

#[test]
fn markdown_final_reflow_keeps_long_paragraph_source_anchor() {
    fn key(app:&mut App,code:KeyCode){app.handle(Event::Key(KeyEvent::new(code,KeyModifiers::NONE)));}
    let source=(0..500).map(|i|format!("Paragraph_{i:03} α界 words words words words words words words words words words\n")).collect::<String>();
    let (_dir,mut app)=pair_markdown(&source.replace("Paragraph_000","Old_000"),&source);
    let screen=draw(&mut app,120,36);click(&mut app,locate(&screen,"Side by side"));ready(&mut app);
    key(&mut app,KeyCode::Char('g'));for c in "251".chars(){key(&mut app,KeyCode::Char(c));}key(&mut app,KeyCode::Enter);
    let initial=draw(&mut app,120,36);
    // Source line start is deliberately tested separately from a row's interior point.
    assert!(text(&initial).contains("Paragraph_250"));
    for width in [200,80,120,32,200] {let screen=draw(&mut app,width,36);assert!(markdown_right_text(&app,&screen).contains("Paragraph_250"),"reflow width {width}: {}",markdown_right_text(&app,&screen));}
    for _ in 0..3 {key(&mut app,KeyCode::Char('s'));let screen=draw(&mut app,120,36);assert!(markdown_right_text(&app,&screen).contains("Paragraph_250"));}
}

#[test]
fn markdown_final_markup_marker_navigation_and_source_activation() {
    let left=format!("{}[same](old)\n\n{}[same](before)\n", "long words α界 ".repeat(500),"second long words α界 ".repeat(500));
    let right=left.replace("](old)","](new)").replace("](before)","](after)");
    let (_dir,mut app)=pair_markdown(&left,&right);
    let screen=draw(&mut app,120,36);click(&mut app,locate(&screen,"Side by side"));ready(&mut app);
    for (id,code) in [(0,'p'),(1,'n'),(0,'p')] {
        key(&mut app,KeyCode::Char(code));let screen=draw(&mut app,120,36);
        assert_eq!(app.session.view().current_change(),Some(id));
        let anchor=app.session.markdown_projection().unwrap().location(id,lenz::view::Side::Left).unwrap();
        let at=locate(&screen,"Markup change");click(&mut app,at);
        assert_eq!(app.session.markdown_mode(),lenz::markdown::PresentationMode::Source);
        assert_eq!(app.session.markdown_anchor(anchor.side),Some(anchor));
        let screen=draw(&mut app,120,36);click(&mut app,locate(&screen,"Side by side"));ready(&mut app);
    }
}

#[test]
fn pending_edits_toolbar_save_discard_and_quit_guard_use_actual_keys_and_disk() {
    use lenz::edit_session::EditMenuCommand as Edit;
    let cwd=std::env::current_dir().unwrap();let dir=tempfile::tempdir_in(&cwd).unwrap();let left=dir.path().join("left.txt");let path=dir.path().join("right.txt");std::fs::write(&left,"old text\n").unwrap();std::fs::write(&path,"new text\n").unwrap();let request=lenz::cli::parse(&["--no-index".into(),left.display().to_string(),path.display().to_string()],&cwd).unwrap();let mut app=App::new(lenz::session::Session::open(request,cwd));ready(&mut app);
    draw(&mut app,240,36);assert!(!toolbar_row(&mut app,240).contains("⤓"));
    assert!(app.session.edit_disabled(Edit::Revert).is_some());modified_key(&mut app,KeyCode::Right,KeyModifiers::ALT|KeyModifiers::SHIFT);ready(&mut app);assert!(app.session.dirty());assert_eq!(app.session.document().unwrap().text(lenz::view::Side::Right),"old text\n");assert_eq!(std::fs::read_to_string(&path).unwrap(),"new text\n");
    modified_key(&mut app,KeyCode::Char('q'),KeyModifiers::CONTROL);assert!(!app.quit);assert!(app.session.notice().unwrap().contains("Unsaved changes"));assert_eq!(std::fs::read_to_string(&path).unwrap(),"new text\n");
    key(&mut app,KeyCode::Char(':'));typed(&mut app,"discard unsaved");let painted=draw(&mut app,240,36);click(&mut app,locate(&painted,"  Discard unsaved"));ready(&mut app);assert!(!app.session.dirty());assert_eq!(app.session.document().unwrap().text(lenz::view::Side::Right),"new text\n");assert_eq!(std::fs::read_to_string(&path).unwrap(),"new text\n");
    app.edit_menu(Edit::ApplyRight);ready(&mut app);assert!(app.session.dirty());modified_key(&mut app,KeyCode::Char('s'),KeyModifiers::CONTROL);ready(&mut app);assert!(!app.session.dirty());assert_eq!(std::fs::read_to_string(&path).unwrap(),"old text\n");
    modified_key(&mut app,KeyCode::Char('z'),KeyModifiers::CONTROL);ready(&mut app);assert!(app.session.dirty());assert_eq!(std::fs::read_to_string(&path).unwrap(),"old text\n");modified_key(&mut app,KeyCode::Char('s'),KeyModifiers::CONTROL);ready(&mut app);assert_eq!(std::fs::read_to_string(&path).unwrap(),"new text\n");
    modified_key(&mut app,KeyCode::Char('q'),KeyModifiers::CONTROL);assert!(app.quit);
}

#[test]
fn compact_toolbar_keeps_primary_actions_and_mouse_palettes_modal() {
    let (dir,mut app)=crate::app::tests::writable_pair(&"old\n".repeat(80),&"new\n".repeat(80));
    app.icons=crate::icons::Icons::Plain;
    let screen=draw(&mut app,80,24);assert!(app.diff_area.y<=2);
    for label in ["☰","↑","↓","⚙","Files"] {assert!(text(&screen).contains(label),"{label}");}
    click(&mut app,locate(&screen,"F10 commands"));typed(&mut app,"find");key(&mut app,KeyCode::Enter);assert!(app.prompt==Some(crate::app::Prompt::Search));key(&mut app,KeyCode::Esc);
    let before=app.session.view().frame();let bytes=std::fs::read(dir.path().join("right.txt")).unwrap();
    let screen=draw(&mut app,80,24);click(&mut app,locate(&screen,"⚙"));assert!(app.toolbar.menu_open());
    let reason=text(&draw(&mut app,80,24));assert!(reason.contains("Whitespace:"));
    app.handle(Event::Mouse(MouseEvent{kind:MouseEventKind::ScrollDown,column:40,row:12,modifiers:KeyModifiers::NONE}));assert_eq!(app.session.view().frame(),before);key(&mut app,KeyCode::Esc);
    let screen=draw(&mut app,48,16);click(&mut app,locate(&screen,"☰"));let screen=draw(&mut app,48,16);click(&mut app,locate(&screen,"About Lenz"));assert!(app.about.is_some());
    key(&mut app,KeyCode::Esc);key(&mut app,KeyCode::Char(':'));for c in "about".chars(){key(&mut app,KeyCode::Char(c));}
    let screen=draw(&mut app,48,16);click(&mut app,locate(&screen,"About Lenz"));assert!(app.about.is_some());
    key(&mut app,KeyCode::Char('w'));assert_eq!(std::fs::read(dir.path().join("right.txt")).unwrap(),bytes);key(&mut app,KeyCode::Esc);
    let screen=draw(&mut app,80,24);click(&mut app,locate(&screen,"☰"));let screen=draw(&mut app,80,24);let options=app.session.options();
    app.handle(Event::Mouse(MouseEvent{kind:MouseEventKind::ScrollDown,column:40,row:12,modifiers:KeyModifiers::NONE}));assert_eq!(app.session.options(),options);assert_eq!(app.session.view().frame(),before);
    click(&mut app,locate(&screen,"Nerd Font icons"));assert!(app.icons.nerd());let screen=draw(&mut app,80,24);assert!(text(&screen).contains("\u{f07b} Files"));assert!(text(&screen).contains("?"));assert_eq!(std::fs::read(dir.path().join("right.txt")).unwrap(),bytes);
}

#[test]
fn palette_find_targets_files_and_commits_and_repository_palette_rows_click() {
    use lenz::repository_session::{Pane,RepositoryCommand};
    let dir=tempfile::tempdir().unwrap();git(dir.path(),&["init","-q"]);
    for path in ["a.txt","b.txt"]{std::fs::write(dir.path().join(path),"old\n").unwrap();}
    git(dir.path(),&["add","."]);git(dir.path(),&["commit","-qm","Initial"]);
    for path in ["a.txt","b.txt"]{std::fs::write(dir.path().join(path),"new\n").unwrap();}
    let req=lenz::cli::parse(&["log".into()],dir.path()).unwrap();let mut app=App::open(req,dir.path().into());ready(&mut app);
    app.repository.as_mut().unwrap().core.execute(RepositoryCommand::SelectRow{row:0,extend:false,two:false});ready(&mut app);
    app.icons=crate::icons::Icons::Plain;let screen=draw(&mut app,160,36);click(&mut app,locate(&screen,"Files (2)"));let spec=app.session.spec().clone();let file=app.session.current();
    let screen=draw(&mut app,160,36);click(&mut app,locate(&screen,"F10 commands"));typed(&mut app,"find");key(&mut app,KeyCode::Enter);assert!(app.navigator.search.active());assert!(app.prompt.is_none());key(&mut app,KeyCode::Esc);
    app.navigator.focused=false;app.repository.as_mut().unwrap().core.execute(RepositoryCommand::Focus(Pane::Log));
    let screen=draw(&mut app,160,36);click(&mut app,locate(&screen,"F10 commands"));typed(&mut app,"find");key(&mut app,KeyCode::Enter);assert!(app.repository.as_ref().unwrap().core.picker.pane_search.active());assert!(app.prompt.is_none());key(&mut app,KeyCode::Char('I'));assert_eq!(app.session.spec(),&spec);assert_eq!(app.session.current(),file);
    key(&mut app,KeyCode::F(10));for c in "about".chars(){key(&mut app,KeyCode::Char(c));}let screen=draw(&mut app,80,24);click(&mut app,locate(&screen,"About Lenz"));assert!(app.about.is_some());
}

fn preview(app: &mut App, layout: lenz::markdown::MarkdownLayout) -> Buffer {
    let screen = draw(app, 160, 40);
    click(app, locate(&screen, if layout == lenz::markdown::MarkdownLayout::Inline { "Inline" } else { "Side by side" }));
    ready(app);
    draw(app, 160, 40);
    draw(app, 160, 40)
}
fn style_at(buffer: &Buffer, needle: &str) -> ratatui::style::Style {
    let (x, y) = locate(buffer, needle);
    buffer[(x, y)].style()
}

#[test]
fn markdown_tui_shows_obsidian_extras_with_terminal_styles() {
    use ratatui::style::Modifier;
    let source = "---\nstatus: draft\ntags:\n  - service\n---\n# T\n\nRead ==the rules== #howto.[^n]\n\n> [!warning] Size limit\n> body\n\n> [!tip]\n> tip body\n\n- [x] done item\n\n[^n]: A note.\n";
    let (_dir, mut app) = pair_markdown(source, &source.replace("# T", "# T2"));
    let screen = preview(&mut app, lenz::markdown::MarkdownLayout::SideBySide);
    let content = text(&screen);
    assert!(content.contains("Properties") && content.contains("status") && content.contains("service"), "{content}");
    assert!(!content.contains("==") && !content.contains("[!warning]"));
    assert!(content.contains("Tip"), "an untitled callout shows its kind");
    assert!(content.contains("tip body"), "the TUI does not fold callouts");
    assert!(content.contains("1."), "footnotes are numbered");
    let p = app.palette.clone();
    assert_eq!(style_at(&screen, "the rules").bg, Some(crate::chrome::color(lenz::theme::highlight_bg(&p))));
    assert!(style_at(&screen, "Size limit").add_modifier.contains(Modifier::BOLD));
    assert_eq!(style_at(&screen, "Size limit").fg, Some(crate::chrome::color(lenz::theme::callout_color(&p, lenz::markdown::CalloutKind::Warning))));
    assert!(style_at(&screen, "done item").add_modifier.contains(Modifier::CROSSED_OUT));
}

#[test]
fn markdown_tui_front_matter_label_shows_at_the_top() {
    let source = include_str!("../../app/fixtures/markdown/reading-view.md");
    let (_dir, mut app) = pair_markdown(source, include_str!("../../app/fixtures/markdown/reading-view-new.md"));
    let screen = preview(&mut app, lenz::markdown::MarkdownLayout::SideBySide);
    assert!(text(&screen).contains("Properties"), "{}", text(&screen));
}

#[test]
fn markdown_tui_inline_shows_one_column_with_struck_and_inserted_words() {
    use ratatui::style::Modifier;
    let (_dir, mut app) = pair_markdown("# A\n\nAlpha beta gamma.\n\nGone.\n", "# A\n\nAlpha delta gamma.\n\nAdded 漢字 e\u{301}.\n");
    let screen = preview(&mut app, lenz::markdown::MarkdownLayout::Inline);
    let content = text(&screen);
    assert_eq!(content.matches("Alpha").count(), 1, "unchanged words once");
    // like the GUI's outlines: coloured, underlined or struck, no fill that could pass for a highlight
    let (beta, delta) = (style_at(&screen, "beta"), style_at(&screen, "delta"));
    assert!(beta.add_modifier.contains(Modifier::CROSSED_OUT));
    assert_eq!(beta.fg, Some(crate::chrome::color(lenz::theme::removed_color(&app.palette))));
    assert!(delta.add_modifier.contains(Modifier::UNDERLINED));
    assert_eq!(delta.fg, Some(crate::chrome::color(app.palette.status[0])));
    assert_ne!(delta.bg, Some(crate::chrome::color(app.palette.inline[0])));
    assert!(style_at(&screen, "Gone").add_modifier.contains(Modifier::CROSSED_OUT));
    assert!(content.contains("Added 漢字"));
    assert!(content.contains("Inline") && content.contains("Side by side") && !content.contains("Preview"), "Source | Inline | Side by side in the toolbar");
    // one pane: no divider column in the body
    let (x, y) = locate(&screen, "Alpha");
    let line: String = (0..screen.area.width).map(|c| screen[(c, y)].symbol().to_string()).collect();
    assert!((0..screen.area.width).all(|c| screen[(c, y)].symbol() != "│" || c < x), "{line:?}");
}

#[test]
fn markdown_tui_inline_narrow_and_switch() {
    let (_dir, mut app) = pair_markdown("Alpha beta.\n", "Alpha delta.\n");
    let screen = preview(&mut app, lenz::markdown::MarkdownLayout::Inline);
    click(&mut app, locate(&screen, "Side by side"));
    assert_eq!(app.session.markdown_layout(), lenz::markdown::MarkdownLayout::SideBySide);
    app.session.execute(lenz::commands::Command::SetMarkdownLayout(lenz::markdown::MarkdownLayout::Inline));
    let narrow = draw(&mut app, 60, 30);
    let content = text(&narrow);
    assert!(content.contains("Alpha") && content.contains("delta") && content.contains("beta"));
}

#[test]
fn markdown_tui_inline_selection_copy_and_switching_keep_their_place() {
    let body: String = (0..60).map(|i| format!("Paragraph {i}.\n\n")).collect();
    let (_dir, mut app) = pair_markdown(&format!("{body}Alpha beta gamma.\n\n{body}"), &format!("{body}Alpha delta gamma.\n\n{body}"));
    let _ = preview(&mut app, lenz::markdown::MarkdownLayout::Inline);
    app.session.execute(lenz::commands::Command::NextChange);
    let screen = draw(&mut app, 160, 40);
    let at = locate(&screen, "beta");
    let line: String = (0..screen.area.width).map(|c| screen[(c, at.1)].symbol().to_string()).collect();
    click(&mut app, at);
    assert_eq!(app.session.markdown_selection().map(|s| s.side), Some(lenz::view::Side::Left), "a removed word selects the old side: {at:?} {line:?}");
    // switching layout keeps the change on screen
    app.session.execute(lenz::commands::Command::SetMarkdownLayout(lenz::markdown::MarkdownLayout::SideBySide));
    let screen = draw(&mut app, 160, 40);
    assert!(text(&screen).contains("Alpha"));
    app.session.execute(lenz::commands::Command::SetMarkdownLayout(lenz::markdown::MarkdownLayout::Inline));
    let screen = draw(&mut app, 160, 40);
    assert!(text(&screen).contains("delta"));
}

#[test]
fn markdown_tui_inline_tab_keeps_keyboard_scrolling() {
    let body: String = (0..120).map(|i| format!("Paragraph {i}.\n\n")).collect();
    let (_dir, mut app) = pair_markdown(&format!("Alpha beta.\n\n{body}"), &format!("Alpha delta.\n\n{body}"));
    let screen = preview(&mut app, lenz::markdown::MarkdownLayout::Inline);
    assert!(text(&screen).contains("Paragraph 1."));
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::PageDown);
    key(&mut app, KeyCode::PageDown);
    let screen = draw(&mut app, 160, 40);
    assert!(!text(&screen).contains("Paragraph 1."), "PageDown after Tab scrolls the Inline view");
    assert_eq!(app.focus, lenz::view::Side::Right);
}

#[test]
fn markdown_tui_switching_from_inline_after_scrolling_keeps_both_panes_together() {
    let body: String = (0..120).map(|i| format!("Paragraph {i}.\n\n")).collect();
    let (_dir, mut app) = pair_markdown(&format!("Alpha beta.\n\n{body}"), &format!("Alpha delta.\n\n{body}"));
    let _ = preview(&mut app, lenz::markdown::MarkdownLayout::Inline);
    for _ in 0..4 { key(&mut app, KeyCode::PageDown); }
    draw(&mut app, 160, 40);
    app.session.execute(lenz::commands::Command::SetMarkdownLayout(lenz::markdown::MarkdownLayout::SideBySide));
    draw(&mut app, 160, 40);
    let screen = draw(&mut app, 160, 40);
    let content = text(&screen);
    let first = |from: usize| { let i = content[from..].find("Paragraph ").unwrap() + from; content[i..i + 14].split('.').next().unwrap().to_string() };
    let left = first(0);
    assert!(!content.contains("Alpha"), "still scrolled down: {left}");
    assert_eq!(content.matches(&format!("{left}.")).count(), 2, "both panes show {left}");
}

#[test]
fn markdown_tui_inline_untitled_callout_label_precedes_a_changed_first_paragraph() {
    let (_dir, mut app) = pair_markdown("> [!tip]\n> old words here\n>\n> second part\n", "> [!tip]\n> new words here\n>\n> second part\n");
    let screen = preview(&mut app, lenz::markdown::MarkdownLayout::Inline);
    let (_, tip) = locate(&screen, "✦ Tip");
    let (_, words) = locate(&screen, "words here");
    assert!(tip < words, "the kind label comes first: {}", text(&screen));
}

#[test]
fn markdown_tui_side_by_side_marks_changed_words_like_inline() {
    use ratatui::style::Modifier;
    let (_dir, mut app) = pair_markdown("Alpha beta gamma.\n", "Alpha delta gamma.\n");
    let screen = preview(&mut app, lenz::markdown::MarkdownLayout::SideBySide);
    let (beta, delta) = (style_at(&screen, "beta"), style_at(&screen, "delta"));
    assert_eq!(beta.fg, Some(crate::chrome::color(lenz::theme::removed_color(&app.palette))));
    assert!(delta.add_modifier.contains(Modifier::UNDERLINED));
    assert_eq!(delta.fg, Some(crate::chrome::color(app.palette.status[0])));
    assert_ne!(style_at(&screen, "Alpha").fg, delta.fg, "unchanged words keep their colour");
}

fn typed(app: &mut App, s: &str) { for c in s.chars() { key(app, KeyCode::Char(c)); } }

#[test]
fn tui_palette_runs_catalog_entries_and_explains_disabled_ones() {
    let (dir, mut app) = pair("a\nb\nc\n", "a\nB\nc\n");
    let before = std::fs::read(dir.path().join("right.txt")).unwrap();
    draw(&mut app, 160, 30);
    key(&mut app, KeyCode::Char(':'));
    assert!(app.command_palette.is_some());
    typed(&mut app, "next change");
    let screen = text(&draw(&mut app, 160, 30));
    assert!(screen.contains("Next change"), "{screen}");
    key(&mut app, KeyCode::Enter);
    assert!(app.command_palette.is_none());
    assert_eq!(app.session.view().current_change(), Some(0));
    // a disabled entry says why and runs nothing
    key(&mut app, KeyCode::F(10));
    typed(&mut app, "previous file");
    assert!(text(&draw(&mut app, 160, 30)).contains("first file"));
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.session.current(), 0);
    key(&mut app, KeyCode::Esc);
    assert!(app.command_palette.is_none());
    assert_eq!(std::fs::read(dir.path().join("right.txt")).unwrap(), before, "typing into the palette never edits");
}

#[test]
fn tui_palette_fits_a_tiny_terminal_and_closes_with_esc() {
    let (_dir, mut app) = pair("a\n", "b\n");
    key(&mut app, KeyCode::F(10));
    let _ = draw(&mut app, 40, 10);
    typed(&mut app, "zzzz-nothing");
    let _ = draw(&mut app, 40, 10);
    key(&mut app, KeyCode::Esc);
    assert!(app.command_palette.is_none());
}

#[test]
fn tui_toolbar_is_lean_and_its_menus_come_from_the_catalog() {
    let (_dir, mut app) = pair("a\nb\n", "a\nB\n");
    let screen = draw(&mut app, 160, 30);
    let bar: String = (0..160).map(|x| screen[(x, 0)].symbol().to_string()).collect();
    for label in ["☰", "↑", "↓", "⚙", "?"] { assert!(bar.contains(label), "{label}: {bar}"); }
    assert!(!bar.contains("⋯"), "no ⋯ when every button fits: {bar}");
    assert!(bar.trim_end().ends_with('?'), "? at the right end, as in the GUI: {bar}");
    for gone in ["Find", "Edit ▾", "View ▾", "More ▾", "Prev", "Next"] { assert!(!bar.contains(gone), "{gone}: {bar}"); }
    click(&mut app, locate(&screen, "☰"));
    let menu = text(&draw(&mut app, 160, 30));
    for label in ["Settings…", "Change theme", "Keyboard shortcuts", "Report last error"] { assert!(menu.contains(label), "{label}"); }
    key(&mut app, KeyCode::Esc);
    let screen = draw(&mut app, 160, 30);
    click(&mut app, locate(&screen, "⚙"));
    let menu = text(&draw(&mut app, 160, 30));
    for label in ["Layout: Split", "Highlight: Words", "Scroll: Shared"] { assert!(menu.contains(label), "{label}"); }
    key(&mut app, KeyCode::Esc);
    let screen = draw(&mut app, 160, 30);
    click(&mut app, locate(&screen, "⚙"));
    let menu = draw(&mut app, 160, 30);
    click(&mut app, locate(&menu, "Whitespace"));
    let menu = draw(&mut app, 160, 30);
    click(&mut app, locate(&menu, "Trim"));
    assert_eq!(app.session.options().whitespace, lenz_engine::WhitespaceMode::Trim);
    // narrower: everything still fits, so no ⋯; it comes back when buttons have to fold (see the narrow-toolbar test)
    let narrow = text(&draw(&mut app, 50, 20));
    assert!(!narrow.contains("⋯") && narrow.contains("☰") && narrow.contains("?"));
}

fn alt(app: &mut App, c: char) { assert!(app.handle(Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)))); }

#[test]
fn tui_focus_hides_branches_and_history_and_keeps_files() {
    let (_dir, mut app) = two_commit_log();
    draw(&mut app, 160, 36);
    let core = |app: &App| app.layout.panels(app.repository.as_ref().map(|r| &r.core));
    let before = core(&app);
    assert!(before.branches && before.history);
    let files = app.navigator.shown();
    alt(&mut app, '0');
    draw(&mut app, 160, 36);
    assert!(app.layout.focused(app.repository.as_ref().map(|r| &r.core)));
    assert_eq!((core(&app).branches, core(&app).history, core(&app).files), (false, false, files), "Focus keeps Files");
    assert_eq!(app.navigator.shown(), files);
    key(&mut app, KeyCode::Esc);
    draw(&mut app, 160, 36);
    assert_eq!((core(&app).branches, core(&app).history, app.navigator.shown()), (true, true, files), "Esc restores");
    // the toolbar's Focus button does the same
    let screen = draw(&mut app, 160, 36);
    click(&mut app, locate(&screen, "Focus"));
    draw(&mut app, 160, 36);
    assert!(app.layout.focused(app.repository.as_ref().map(|r| &r.core)));
}

#[test]
fn tui_plain_window_has_no_overview() {
    let (_dir, mut app) = pair("a\n", "b\n");
    draw(&mut app, 160, 30);
    if !app.navigator.shown() { alt(&mut app, '1'); draw(&mut app, 160, 30); }
    alt(&mut app, '0');
    draw(&mut app, 160, 30);
    assert!(app.navigator.shown(), "no repository: Focus is unavailable and changes nothing");
    assert!(!app.layout.focused(None));
}

#[test]
fn tui_pane_header_switches_markdown_views_and_image_modes() {
    let (_dir, mut app) = pair_markdown("# A\n\nold\n", "# A\n\nnew\n");
    let screen = draw(&mut app, 160, 30);
    let row = app.diff_area.y + 1;
    let header: String = (0..160).map(|x| screen[(x, row)].symbol().to_string()).collect();
    assert!(header.contains("Source") && header.contains("Inline") && header.contains("Side by side"), "{header}");
    let toolbar: String = (0..160).map(|x| screen[(x, 0)].symbol().to_string()).collect();
    assert!(!toolbar.contains("Inline"), "the modes left the toolbar");
    click(&mut app, locate(&screen, "Inline"));
    ready(&mut app);
    assert_eq!(app.session.markdown_mode(), lenz::markdown::PresentationMode::Rendered);
    let (_dir2, mut plain) = pair("a\n", "b\n");
    let screen = draw(&mut plain, 160, 30);
    assert!(!text(&screen).contains("Side by side"), "only Markdown has the switch");
    // images: the four modes, and no Markdown entries
    let dir = tempfile::tempdir().unwrap();
    let mut pixels = lenz::image_session::Pixels::new(8, 8);
    for (path, rgba) in [("old.png", [210, 32, 64, 255]), ("new.png", [40, 90, 230, 255])] {
        for pixel in pixels.pixels_mut() { pixel.0 = rgba; }
        pixels.save(dir.path().join(path)).unwrap();
    }
    let request = lenz::cli::parse(&["--no-index".into(), dir.path().join("old.png").display().to_string(), dir.path().join("new.png").display().to_string()], dir.path()).unwrap();
    let mut image = App::new(lenz::session::Session::open(request, dir.path().into()));
    ready(&mut image);
    let screen = draw(&mut image, 160, 30);
    let row = image.diff_area.y + 1;
    let header: String = (0..160).map(|x| screen[(x, row)].symbol().to_string()).collect();
    for mode in ["2-up", "Swipe", "Onion skin", "Difference"] { assert!(header.contains(mode), "{mode}: {header}"); }
    assert!(!text(&screen).contains("Inline"));
    click(&mut image, locate(&screen, "Swipe"));
    assert_eq!(image.visual.image_state().mode(), lenz::image_session::Mode::Swipe);
}

#[test]
fn tui_back_and_forward_return_to_the_previous_commit() {
    let dir = tempfile::tempdir().unwrap(); let d = dir.path();
    git(d, &["init", "-q", "-b", "main"]);
    for n in 0..3 { std::fs::write(d.join("a.txt"), format!("{n}\n")).unwrap(); git(d, &["add", "."]); git(d, &["commit", "-qm", &format!("c{n}")]); }
    let request = lenz::cli::parse(&["log".into()], d).unwrap(); let mut app = App::open(request, d.into()); ready(&mut app);
    draw(&mut app, 160, 30);
    let anchor = |app: &App| app.repository.as_ref().unwrap().core.picker.anchor;
    let first = anchor(&app);
    key(&mut app, KeyCode::Down); ready(&mut app);
    let second = anchor(&app);
    assert_ne!(first, second);
    app.handle(Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::ALT)));
    ready(&mut app);
    assert_eq!(anchor(&app), first, "Alt+Left goes back");
    app.handle(Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::ALT))); ready(&mut app);
    assert_eq!(anchor(&app), second, "Alt+Right goes forward");
    let screen = draw(&mut app, 160, 30);
    click(&mut app, locate(&screen, "◀")); ready(&mut app);
    assert_eq!(anchor(&app), first, "◀ goes back");
    // outside a repository the keys do nothing
    let (pdir, mut plain) = pair("a\n", "b\n");
    let before = std::fs::read(pdir.path().join("right.txt")).unwrap();
    plain.handle(Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::ALT)));
    plain.handle(Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::ALT)));
    ready(&mut plain);
    assert_eq!(plain.session.current(), 0);
    assert!(!plain.session.dirty());
    assert_eq!(std::fs::read(pdir.path().join("right.txt")).unwrap(), before);
}

#[test]
fn tui_diff_context_menu_shows_only_what_is_under_the_pointer() {
    // inside the working directory, so both sides are writable
    let cwd = std::env::current_dir().unwrap(); let dir = tempfile::tempdir_in(&cwd).unwrap();
    let (left, right) = (dir.path().join("left.txt"), dir.path().join("right.txt"));
    std::fs::write(&left, "a\nb\n").unwrap(); std::fs::write(&right, "a\nB\n").unwrap();
    let request = lenz::cli::parse(&["--no-index".into(), left.display().to_string(), right.display().to_string()], &cwd).unwrap();
    let mut app = App::new(lenz::session::Session::open(request, cwd)); ready(&mut app);
    let screen = draw(&mut app, 160, 30);
    // the line, not a "B" in the random directory name of a pane title
    let (x, y) = locate(&screen, "2  B"); let x = x + 3;
    let right = |app: &mut App, x: u16, y: u16| { app.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Right), column: x, row: y, modifiers: KeyModifiers::NONE })); };
    right(&mut app, x, y);
    let menu = text(&draw(&mut app, 160, 30));
    assert!(!menu.contains("Copy selection") && !menu.contains("Copy reference"), "no selection, no copy: {menu}");
    assert!(menu.contains("Move change"), "the change under the pointer: writable {:?} block {:?} at {:?}", app.session.document().map(|d|[d.is_writable(lenz::view::Side::Left),d.is_writable(lenz::view::Side::Right)]), app.block_at(x,y), (x,y));
    assert!(!menu.contains("Copy file path"), "nothing that is not under the pointer: {menu}");
    assert!(menu.contains("All commands…"));
    let screen = draw(&mut app, 160, 30);
    click(&mut app, locate(&screen, "All commands…"));
    assert!(app.command_palette.is_some(), "the full list is the palette");
    key(&mut app, KeyCode::Esc);
    // with a selection, Copy and Copy reference
    app.selection = Some(lenz::selection::Selection { side: lenz::view::Side::Right, anchor: lenz::selection::Pos::new(1, 0), head: lenz::selection::Pos::new(1, 1) });
    right(&mut app, x, y);
    let menu = text(&draw(&mut app, 160, 30));
    assert!(menu.contains(lenz::commands::Interaction::Copy.label()) && menu.contains(lenz::commands::Interaction::CopyReference.label()), "{menu}");
}

#[test]
fn tui_palette_shows_terminal_keys_where_they_differ_from_the_gui() {
    let (_dir, mut app) = two_commit_log();
    draw(&mut app, 160, 30);
    key(&mut app, KeyCode::F(10));
    typed(&mut app, "focus");
    let screen = text(&draw(&mut app, 160, 30));
    assert!(screen.contains("alt-0"), "Focus is Alt+0 here; f opens Find file: {screen}");
}

#[test]
fn tui_back_forward_outside_a_repository_says_why() {
    let (_dir, mut app) = pair("a\n", "b\n");
    app.handle(Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::ALT)));
    assert!(app.session.notice().is_some_and(|n| n.contains("repository")), "{:?}", app.session.notice());
}

#[test]
fn tui_diff_options_entry_opens_the_diff_menu() {
    let (_dir, mut app) = pair("a\n", "b\n");
    draw(&mut app, 160, 30);
    key(&mut app, KeyCode::Char(':'));
    typed(&mut app, "diff options");
    key(&mut app, KeyCode::Enter);
    assert!(app.command_palette.is_none() && app.toolbar.menu_open(), "the ⚙ menu, not the palette again");
    assert!(text(&draw(&mut app, 160, 30)).contains("Highlight: Words"));
    key(&mut app, KeyCode::Esc);
    // too narrow for the ⚙ button: the palette's Diff group instead
    draw(&mut app, 8, 6);
    key(&mut app, KeyCode::Char(':'));
    typed(&mut app, "diff options");
    key(&mut app, KeyCode::Enter);
    assert!(app.command_palette.as_ref().is_some_and(|p| p.query().starts_with("diff")));
}

#[test]
fn tui_header_switch_ignores_a_cell_from_a_comparison_no_longer_shown() {
    let (dir, mut app) = pair_markdown("# A\n\nold\n", "# A\n\nnew\n");
    let screen = draw(&mut app, 160, 30);
    let at = locate(&screen, "Inline");
    // the comparison changes before the next frame
    std::fs::write(dir.path().join("a.txt"), "a\n").unwrap();
    std::fs::write(dir.path().join("b.txt"), "b\n").unwrap();
    app.session = lenz::session::Session::open(lenz::cli::parse(&["--no-index".into(), "a.txt".into(), "b.txt".into()], dir.path()).unwrap(), dir.path().into());
    ready(&mut app);
    app.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: at.0, row: at.1, modifiers: KeyModifiers::NONE }));
    assert_eq!(app.session.markdown_mode(), lenz::markdown::PresentationMode::Source);
    assert!(app.session.notice().is_none(), "a stale cell runs nothing: {:?}", app.session.notice());
}

fn two_commit_log() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap(); let d = dir.path();
    git(d, &["init", "-q", "-b", "main"]);
    std::fs::write(d.join("a.txt"), "first\n").unwrap(); git(d, &["add", "."]); git(d, &["commit", "-qm", "Subject one"]);
    std::fs::write(d.join("a.txt"), "SECOND\n").unwrap(); git(d, &["commit", "-qam", "Subject two"]);
    git(d, &["branch", "side"]);
    let request = lenz::cli::parse(&["log".into()], d).unwrap(); let mut app = App::open(request, d.into()); ready(&mut app);
    (dir, app)
}

#[test]
fn tui_history_keeps_its_rows_while_a_restarted_log_has_none() {
    let (_dir, mut app) = two_commit_log();
    assert!(text(&draw(&mut app, 160, 36)).contains("Subject two"));
    app.repository.as_mut().unwrap().core.execute(lenz::repository_session::RepositoryCommand::Walk(lenz::history::Branches::All));
    assert!(app.repository.as_ref().unwrap().core.holding_log(), "the log restarted and has no rows yet");
    assert!(text(&draw(&mut app, 160, 36)).contains("Subject two"), "the last rows stay up");
}

#[test]
fn tui_comparison_stays_up_until_the_next_one_is_read() {
    let (_dir, mut app) = two_commit_log();
    let screen = text(&draw(&mut app, 160, 36));
    assert!(screen.contains("SECOND"), "{screen}");
    let row = (0..app.repository.as_ref().unwrap().core.picker.rows()).find(|&r| app.repository.as_ref().unwrap().core.picker.log_row(r).is_some_and(|i| app.repository.as_ref().unwrap().core.picker.log.subject(i) == "Subject one")).unwrap();
    app.repository.as_mut().unwrap().core.execute(lenz::repository_session::RepositoryCommand::SelectRow { row, extend: false, two: false });
    // the event loop draws after the click, before the new Change Set can arrive
    draw(&mut app, 160, 36);
    let started = std::time::Instant::now();
    while !app.session.loading() { app.poll(); assert!(started.elapsed().as_secs() < 5); std::thread::sleep(std::time::Duration::from_millis(1)); }
    let held = text(&draw(&mut app, 160, 36));
    assert!(held.contains("SECOND") && !held.contains("Loading comparison"), "the old comparison stays while the new one is read: {held}");
    ready(&mut app);
    assert!(text(&draw(&mut app, 160, 36)).contains("first"));
}

#[test]
fn tui_markdown_callout_title_click_folds_and_unfolds() {
    let src = "# Notes\n\n> [!note]- Folded title\n> secret body\n\nafter one\n";
    let (_dir, mut app) = pair_markdown(&src.replace("after one", "after old"), src);
    let screen = preview(&mut app, lenz::markdown::MarkdownLayout::Inline);
    assert!(text(&screen).contains("Folded title") && !text(&screen).contains("secret body"));
    click(&mut app, locate(&screen, "Folded title"));
    assert!(text(&draw(&mut app, 160, 40)).contains("secret body"), "a click on the title opens it");
    let screen = draw(&mut app, 160, 40);
    click(&mut app, locate(&screen, "Folded title"));
    assert!(!text(&draw(&mut app, 160, 40)).contains("secret body"), "and closes it again");
    // the keyboard: the caret on the title row, then Enter
    let screen = draw(&mut app, 160, 40);
    click(&mut app, locate(&screen, "Notes"));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    assert!(text(&draw(&mut app, 160, 40)).contains("secret body"), "Enter on the title opens it");
}

#[test]
fn tui_markdown_preview_draws_local_images() {
    let (dir, mut app) = pair_markdown("# Pictures\n\n![a diagram](pic.png)\n\nold text\n", "# Pictures\n\n![a diagram](pic.png)\n\nnew text\n");
    let mut pixels = lenz::image_session::Pixels::new(400, 200);
    for (x, _, pixel) in pixels.enumerate_pixels_mut() { pixel.0 = if x < 200 { [230, 40, 40, 255] } else { [40, 40, 230, 255] }; }
    pixels.save(dir.path().join("pic.png")).unwrap();
    preview(&mut app, lenz::markdown::MarkdownLayout::Inline);
    // decoded in the background, then laid out
    ready(&mut app);
    let screen = draw(&mut app, 160, 40);
    let red = ratatui::style::Color::Rgb(230, 40, 40);
    let blue = ratatui::style::Color::Rgb(40, 40, 230);
    let cells: Vec<_> = screen.content.iter().filter(|c| c.symbol() == "▀").collect();
    assert!(cells.iter().any(|c| c.fg == red) && cells.iter().any(|c| c.fg == blue), "the picture in colour cells");
    let rows: std::collections::BTreeSet<u16> = (0..screen.area.height).filter(|&y| (0..screen.area.width).any(|x| screen[(x, y)].symbol() == "▀")).collect();
    assert!(!rows.is_empty() && rows.len() <= 20, "{} rows", rows.len());
    assert!(text(&screen).contains("new text"));
    // a missing image keeps its placeholder text
    std::fs::remove_file(dir.path().join("pic.png")).unwrap();
    let (_dir2, mut other) = pair_markdown("![gone](nowhere.png)\n\nold\n", "![gone](nowhere.png)\n\nnew\n");
    preview(&mut other, lenz::markdown::MarkdownLayout::Inline);
    ready(&mut other);
    let screen = draw(&mut other, 160, 40);
    assert!(!screen.content.iter().any(|c| c.symbol() == "▀"));
    assert!(text(&screen).contains("gone"), "the placeholder stays");
}

#[test]
fn tui_markdown_images_use_kitty_placeholders_where_the_terminal_has_them() {
    let (dir, mut app) = pair_markdown("![a diagram](pic.png)\n\nold text\n", "![a diagram](pic.png)\n\nnew text\n");
    let mut pixels = lenz::image_session::Pixels::new(400, 200);
    for pixel in pixels.pixels_mut() { pixel.0 = [230, 40, 40, 255]; }
    pixels.save(dir.path().join("pic.png")).unwrap();
    app.markdown.images.enable_kitty();
    preview(&mut app, lenz::markdown::MarkdownLayout::Inline);
    ready(&mut app);
    let screen = draw(&mut app, 160, 40);
    let cells: Vec<_> = screen.content.iter().filter(|c| c.symbol().starts_with('\u{10EEEE}')).collect();
    assert!(!cells.is_empty() && !screen.content.iter().any(|c| c.symbol() == "▀"), "placeholders, not colour cells");
    // row 0, column 0 of the first placeholder: the first two diacritics
    assert!(cells.iter().any(|c| c.symbol() == "\u{10EEEE}\u{0305}\u{0305}"));
    let ratatui::style::Color::Rgb(r, g, b) = cells[0].fg else { panic!("the image id is the foreground colour") };
    let id = u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b);
    let sent = app.markdown.images.take_graphics().concat();
    assert!(sent.starts_with(&format!("\x1b_Ga=t,i={id},q=2,f=32,")), "{}", &sent[..sent.len().min(80)]);
    assert!(sent.ends_with(&format!("\x1b_Ga=p,U=1,i={id},c=")) || sent.contains(&format!("\x1b_Ga=p,U=1,i={id},c=")), "a separate virtual placement");
    assert!(sent.contains(",m=1;") || sent.contains(",m=0;"));
    assert!(sent.ends_with("\x1b\\") && sent.matches("\x1b_G").all(|_| true));
    for chunk in sent.split("\x1b_G").skip(1) { assert!(chunk.split(';').nth(1).map_or(0, |p| p.trim_end_matches("\x1b\\").len()) <= 4096); }
    assert!(app.markdown.images.take_graphics().is_empty(), "sent once");
    draw(&mut app, 160, 40);
    assert!(app.markdown.images.take_graphics().is_empty(), "still once after a redraw");
    // another document: the old image is deleted from the terminal
    let (_other_dir, other) = pair_markdown("x\n", "y\n");
    app.session = other.session;
    ready(&mut app);
    preview(&mut app, lenz::markdown::MarkdownLayout::Inline);
    let deleted = app.markdown.images.take_graphics().concat();
    assert!(deleted.contains(&format!("a=d,d=I,i={id}")), "{deleted}");
}

#[test]
fn kitty_graphics_detection_follows_the_terminal_and_the_override() {
    use crate::md_images::kitty_supported as k;
    let env = |pairs: &'static [(&'static str, &'static str)]| move |name: &str| pairs.iter().find(|(n, _)| *n == name).map(|(_, v)| v.to_string());
    assert!(k(env(&[("TERM_PROGRAM", "ghostty")])));
    assert!(k(env(&[("TERM", "xterm-kitty")])));
    assert!(k(env(&[("KITTY_WINDOW_ID", "3")])));
    assert!(!k(env(&[("TERM", "alacritty")])));
    assert!(!k(env(&[("TERM_PROGRAM", "ghostty"), ("TMUX", "/tmp/x")])), "tmux needs passthrough");
    assert!(!k(env(&[("TERM_PROGRAM", "ghostty"), ("LENZ_IMAGES", "cells")])));
    assert!(k(env(&[("TERM", "foot"), ("LENZ_IMAGES", "kitty")])));
}

#[test]
fn tui_repository_panes_open_the_shared_palette() {
    use lenz::repository_session::{Pane, RepositoryCommand};
    let (_dir, mut app) = two_commit_log();
    draw(&mut app, 160, 36);
    for pane in [Pane::Log, Pane::Sidebar] {
        app.repository.as_mut().unwrap().core.execute(RepositoryCommand::Focus(pane));
        key(&mut app, KeyCode::F(10));
        assert!(app.command_palette.is_some(), "F10 in {pane:?}");
        key(&mut app, KeyCode::Esc);
    }
    // `:` too, outside the lists (where it is search text)
    app.repository.as_mut().unwrap().core.execute(RepositoryCommand::Focus(Pane::Diff));
    key(&mut app, KeyCode::Char(':'));
    assert!(app.command_palette.is_some(), ": in the comparison");
    typed(&mut app, "all branches");
    let screen = text(&draw(&mut app, 160, 36));
    assert!(screen.contains("branches"), "{screen}");
    key(&mut app, KeyCode::Enter);
    assert!(app.command_palette.is_none());
    assert_eq!(app.repository.as_ref().unwrap().core.picker.branches, lenz::history::Branches::All);
}

#[test]
fn tui_palette_ends_an_active_list_search_like_esc() {
    use lenz::repository_session::{Pane, RepositoryCommand};
    let (_dir, mut app) = two_commit_log();
    draw(&mut app, 160, 36);
    app.repository.as_mut().unwrap().core.execute(RepositoryCommand::Focus(Pane::Log));
    typed(&mut app, "two");
    assert!(app.repository.as_ref().unwrap().core.picker.pane_search.active());
    key(&mut app, KeyCode::F(10));
    assert!(app.command_palette.is_some());
    assert!(!app.repository.as_ref().unwrap().core.picker.pane_search.active(), "the search ends as with Esc");
}

#[test]
fn tui_history_filter_from_focus_restores_the_panels_focus_hid() {
    use lenz::catalog::{Action, RepoAction};
    let (_dir, mut app) = two_commit_log();
    draw(&mut app, 160, 36);
    let before = app.layout.panels(app.repository.as_ref().map(|r| &r.core));
    assert!(before.branches && before.history);
    alt(&mut app, '0');
    assert!(app.layout.focused(app.repository.as_ref().map(|r| &r.core)));
    app.run_action(Action::Repository(RepoAction::Filter));
    let after = app.layout.panels(app.repository.as_ref().map(|r| &r.core));
    assert!(after.branches && after.history, "Focus ends: Branches comes back too");
    key(&mut app, KeyCode::Esc);
}

#[test]
fn tui_pr_tabs_give_way_before_markdown_modes_in_a_narrow_header() {
    let (dir, mut app) = pair_markdown("# A\n\nold\n", "# A\n\nnew\n");
    let req = lenz::cli::parse(&["--no-index".into(), "left.md".into(), "right.md".into()], dir.path()).unwrap();
    let mut repository = crate::repository::Repository::open(req, dir.path().into());
    let pr: lenz::pr::PullRequest =lenz::pr::PullRequest{number:9,title:"PR description".into(),author:lenz::pr::Account{login:"author".into()},state:lenz::pr::State::Open,is_draft:false,body:String::new(),base_ref_name:"main".into(),head_ref_name:"feature".into(),head_ref_oid:"head".into(),url:"https://github.com/o/r/pull/9".into(),additions:1,deletions:0,changed_files:1,is_cross_repository:false,head_repository:None,head_repository_owner:None};
    let mut state = lenz::pr_session::PrState::new(pr, lenz::pr::Fetched { number: 14130, merge_base: "base".into(), head: "head".into() });
    state.marks = None;
    repository.core.pr = Some(state);
    app.repository = Some(repository);
    app.layout_command(lenz::layout::LayoutCommand::ToggleFocus);
    assert!(app.layout.focused(app.repository.as_ref().map(|r| &r.core)));
    let wide = text(&draw(&mut app, 200, 30));
    assert!(wide.contains("Files changed") && wide.contains("Source"), "{wide}");
    let narrow = text(&draw(&mut app, 100, 30));
    assert!(narrow.contains("Source"), "Markdown modes stay when the PR tabs do not fit: {narrow}");
}

fn toolbar_row(app: &mut App, width: u16) -> String {
    let screen = draw(app, width, 36);
    (0..width).map(|x| screen[(x, 0)].symbol().to_string()).collect()
}

#[test]
fn tui_read_only_toolbar_hides_edit_buttons_in_overview_and_focus() {
    let (_dir, mut app) = two_commit_log();
    for focus in [false, true] {
        if focus { alt(&mut app, '0'); }
        for width in [160, 80, 40] {
            let row = toolbar_row(&mut app, width);
            for label in ["Undo", "Redo", "Save"] {
                assert!(!row.contains(label), "read-only toolbar: {row}");
            }
        }
    }
}

#[test]
fn tui_source_changes_use_backgrounds_without_underlining() {
    use ratatui::style::Modifier;
    let (_dir, mut app) = pair("let value = old;\n", "let value = new;\n");
    draw(&mut app, 160, 36);
    ready(&mut app);
    for theme in lenz::theme::builtin_entries() {
        app.palette=theme.palette;
        for width in [160,48] {
            let screen = draw(&mut app, width, 36);
            let changed = style_at(&screen, "new");
            assert!(!changed.add_modifier.contains(Modifier::UNDERLINED), "{}: {changed:?}",theme.id);
            assert_eq!(changed.bg, Some(crate::chrome::color(app.palette.inline[2])));
            assert_eq!(changed.fg, Some(crate::chrome::color(app.palette.fg)));
        }
    }
}

#[test]
fn tui_editable_toolbar_hides_edit_buttons_even_with_a_dirty_draft() {
    let (_dir,mut app)=crate::app::tests::writable_pair("old\n","new\n");
    for dirty in [false,true] {
        if dirty {modified_key(&mut app,KeyCode::Right,KeyModifiers::ALT|KeyModifiers::SHIFT);ready(&mut app);assert!(app.session.dirty());}
        for width in [160,80,48] {
            let row=toolbar_row(&mut app,width);
            for label in ["Undo","Redo","Save","↶","↷","⤓"] {assert!(!row.contains(label),"{row}");}
        }
    }
    modified_key(&mut app,KeyCode::Char('s'),KeyModifiers::CONTROL);ready(&mut app);assert!(!app.session.dirty());
}

#[test]
fn tui_focus_toolbar_matches_the_gui() {
    let (_dir, mut app) = two_commit_log();
    let overview = toolbar_row(&mut app, 160);
    assert_eq!(overview.split_whitespace().take(2).collect::<Vec<_>>(), ["☰", "Focus"], "☰, then Focus: {overview}");
    assert!(!overview.contains("History") && overview.contains("☰"), "History toggles at the bottom: {overview}");
    alt(&mut app, '0');
    let focus = toolbar_row(&mut app, 160);
    assert_eq!(focus.split_whitespace().take(2).collect::<Vec<_>>(), ["☰", "Overview"], "Overview where Focus was: {focus}");
    for want in ["Overview", "◀", "⚙", "?"] { assert!(focus.contains(want), "{want} missing: {focus}"); }
    assert!(!focus.contains("⋯") && focus.trim_end().ends_with('?'), "? at the right end, no ⋯: {focus}");
    for gone in ["History", "Branches", "Focus "] { assert!(!focus.contains(gone), "{gone} in Focus: {focus}"); }
    let (_dir2, mut plain) = pair("a\n", "b\n");
    let plain_row = toolbar_row(&mut plain, 160);
    assert!(plain_row.contains("⚙") && !plain_row.contains("Overview"), "a plain window is Focus without Overview: {plain_row}");
}

#[test]
fn tui_shows_the_selection_title_in_the_diff_title_and_history_header() {
    let (_dir, mut app) = two_commit_log();
    let mut screen = text(&draw(&mut app, 160, 36));
    for _ in 0..200 { if app.repository.as_ref().unwrap().core.picker.refs.is_some() { break; } std::thread::sleep(std::time::Duration::from_millis(5)); app.handle(Event::Resize(160, 36)); screen = text(&draw(&mut app, 160, 36)); }
    let title = app.repository.as_ref().unwrap().core.selection_title().expect("a commit is selected").text;
    assert!(title.starts_with("main ("), "{title}");
    assert!(screen.matches(&title).count() >= 2, "diff title and History header: {screen}");
}

#[test]
fn tui_files_enter_and_double_click_enter_focus() {
    let (_dir, mut app) = two_commit_log();
    let focused = |app: &App| app.layout.focused(app.repository.as_ref().map(|r| &r.core));
    let screen = draw(&mut app, 160, 36);
    let at = locate(&screen, "M a.txt");
    click(&mut app, at);
    assert!(!focused(&app), "one click opens the file");
    click(&mut app, at);
    assert!(focused(&app), "a double click enters Focus");
    // Files holds the keyboard: Esc hands it back, then leaves Focus
    key(&mut app, KeyCode::Esc);
    if focused(&app) { key(&mut app, KeyCode::Esc); }
    assert!(!focused(&app));
    let screen = draw(&mut app, 160, 36);
    click(&mut app, locate(&screen, "M a.txt"));
    std::thread::sleep(std::time::Duration::from_millis(450));
    key(&mut app, KeyCode::Enter);
    assert!(focused(&app), "Enter on Files enters Focus");
}

fn pr_log() -> (tempfile::TempDir, App) {
    let (dir, mut app) = two_commit_log();
    let mut pr = lenz::pr::PullRequest { number: 9, title: "PR description".into(), author: lenz::pr::Account { login: "author".into() }, state: lenz::pr::State::Open, is_draft: false, body: String::new(), base_ref_name: "main".into(), head_ref_name: "feature".into(), head_ref_oid: "head".into(), url: "https://github.com/o/r/pull/9".into(), additions: 1, deletions: 0, changed_files: 1, is_cross_repository: false, head_repository: None, head_repository_owner: None };
    pr.body = "Description line\n".repeat(60) + "DESCRIPTION_END";
    let mut state = lenz::pr_session::PrState::new(pr, lenz::pr::Fetched { number: 9, merge_base: "base".into(), head: "head".into() });
    state.marks = None;
    app.repository.as_mut().unwrap().core.pr = Some(state);
    (dir, app)
}

#[test]
fn tui_pr_details_are_a_focus_tab_not_a_dialog() {
    use lenz::catalog::{Action, PrAction};
    use lenz::pr_session::DetailsTab;
    let (_dir, mut app) = pr_log();
    draw(&mut app, 160, 36);
    app.run_action(Action::Pr(PrAction::Details(DetailsTab::Conversation)));
    let screen = text(&draw(&mut app, 160, 36));
    fn core(app: &App) -> Option<&lenz::repository_session::RepositorySession> { app.repository.as_ref().map(|r| &r.core) }
    assert!(app.layout.focused(core(&app)), "details enter Focus");
    assert!(!app.repository.as_ref().unwrap().modal(), "a tab, not a modal dialog");
    assert!(screen.contains("Description line") && screen.contains("Overview") && screen.contains("Files changed"), "{screen}");
    let options = app.session.options();
    key(&mut app, KeyCode::Char('i'));
    assert_eq!(app.session.options(), options, "keys never reach the hidden diff");
    key(&mut app, KeyCode::End);
    assert!(text(&draw(&mut app, 160, 36)).contains("DESCRIPTION_END"));
    key(&mut app, KeyCode::F(10));
    assert!(app.command_palette.is_some(), "the palette opens over the tab");
    key(&mut app, KeyCode::Esc);
    assert!(app.repository.as_ref().unwrap().core.pr.as_ref().unwrap().details().is_some());
    key(&mut app, KeyCode::Esc);
    assert!(app.repository.as_ref().unwrap().core.pr.as_ref().unwrap().details().is_none(), "Esc returns to the diff");
    assert!(app.layout.focused(core(&app)), "and stays in Focus");
    app.run_action(Action::Pr(PrAction::Details(DetailsTab::Commits)));
    app.run_action(Action::Comparison(lenz::commands::Command::NextChange));
    assert!(app.repository.as_ref().unwrap().core.pr.as_ref().unwrap().details().is_none(), "a diff action shows the diff");
    app.run_action(Action::Pr(PrAction::Details(DetailsTab::Conversation)));
    app.layout_command(lenz::layout::LayoutCommand::LeaveFocus);
    assert!(app.repository.as_ref().unwrap().core.pr.as_ref().unwrap().details().is_none(), "the Overview shows the diff");
}

#[test]
fn tui_unified_is_one_column_with_selection_on_either_side() {
    use lenz::commands::{Command, DiffLayout};
    let (dir, mut app) = pair("a\nbeta\nc\n", "a\nBETA\nc\n");
    draw(&mut app, 120, 20);
    app.run_action(lenz::catalog::Action::Comparison(Command::SetDiffLayout(DiffLayout::Unified)));
    assert!(app.session.view().unified(), "the terminal has Unified too");
    let screen = draw(&mut app, 120, 20);
    let (bx, by) = locate(&screen, "beta");
    let (cx, cy) = locate(&screen, "BETA");
    assert_eq!((bx, by + 1), (cx, cy), "one column: the deleted line, then the inserted one below it");
    click(&mut app, locate(&screen, "⚙"));
    assert!(text(&draw(&mut app, 120, 20)).contains("Layout: Unified"), "the Diff menu names the layout");
    key(&mut app, KeyCode::Esc);
    for (at, side) in [((bx, by), lenz::view::Side::Left), ((cx, cy), lenz::view::Side::Right)] {
        click(&mut app, at);
        click(&mut app, at);
        assert_eq!(app.selection.map(|s| s.side), Some(side), "a double click selects the word on the row's side");
        assert!(app.selection.is_some_and(|s| !s.is_empty()));
    }
    let bytes = std::fs::read(dir.path().join("right.txt")).unwrap();
    app.run_action(lenz::catalog::Action::Edit(lenz::edit_session::EditMenuCommand::Undo));
    assert_eq!(std::fs::read(dir.path().join("right.txt")).unwrap(), bytes, "read-only");
    app.run_action(lenz::catalog::Action::Comparison(Command::SetDiffLayout(DiffLayout::Split)));
    assert!(!app.session.view().unified());
}

#[test]
fn tui_focus_names_the_selection_above_files() {
    let (_dir, mut app) = two_commit_log();
    for _ in 0..200 { if app.repository.as_ref().unwrap().core.picker.refs.is_some() { break; } std::thread::sleep(std::time::Duration::from_millis(5)); app.handle(Event::Resize(160, 36)); }
    let title = app.repository.as_ref().unwrap().core.selection_title().unwrap().text;
    alt(&mut app, '0');
    draw(&mut app, 160, 36);
    if !app.navigator.shown() { alt(&mut app, '1'); }
    let screen = text(&draw(&mut app, 160, 36));
    assert!(app.navigator.shown());
    assert!(screen.contains(&format!("┌{title}")), "the Files column is headed by the selection: {screen}");
    // the commit message box carries the name, the list below it keeps its own
    let buffer = draw(&mut app, 160, 36);
    assert!(locate(&buffer, &format!("┌{title}")).1 < locate(&buffer, "┌Files (1)").1, "{screen}");
    assert_eq!(screen.matches(&title).count(), 1, "named once: {screen}");
}

#[test]
fn tui_focus_and_overview_buttons_match_and_back_forward_stay() {
    let (_dir, mut app) = two_commit_log();
    let overview = toolbar_row(&mut app, 160);
    alt(&mut app, '0');
    let focus = toolbar_row(&mut app, 160);
    assert!(focus.contains("◀") && focus.contains("▶"), "Back and Forward in Focus too: {focus}");
    assert_eq!(overview.find('◀'), focus.find('◀'), "Focus and Overview are one size, so ◀ stays put:\n{overview}\n{focus}");
}

#[test]
fn tui_panel_switches_sit_glued_at_the_bottom() {
    let (_dir, mut app) = two_commit_log();
    let bar = toolbar_row(&mut app, 160);
    for gone in ["Branches", "History", "Files"] { assert!(!bar.contains(gone), "{gone} left the toolbar: {bar}"); }
    assert!(bar.contains("?"), "{bar}");
    let screen = draw(&mut app, 160, 36);
    let bottom: String = (0..160).map(|x| screen[(x, 35)].symbol().to_string()).collect();
    assert!(bottom.contains("Branches│History│Files"), "glued at the bottom: {bottom}");
    let history = |app: &App| app.layout.panels(app.repository.as_ref().map(|r| &r.core)).history;
    let before = history(&app);
    click(&mut app, locate(&screen, "History│"));
    assert_eq!(history(&app), !before, "a click toggles History");
    alt(&mut app, '0');
    let focus = toolbar_row(&mut app, 160);
    assert!(!focus.contains("≡") && focus.contains("?"), "{focus}");
    let screen = draw(&mut app, 160, 36);
    click(&mut app, locate(&screen, "History│"));
    assert!(app.layout.focused(app.repository.as_ref().map(|r| &r.core)), "Focus refuses History");
}


#[test]
fn tui_a_click_outside_closes_any_popup() {
    let (_dir, mut app) = pair("a\n", "b\n");
    let corner = |app: &mut App| { draw(app, 120, 30); app.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: 118, row: 27, modifiers: KeyModifiers::NONE })); draw(app, 120, 30); };
    let mut stuck = Vec::new();
    // the palette
    key(&mut app, KeyCode::F(10)); assert!(app.command_palette.is_some());
    corner(&mut app); if app.command_palette.is_some() { stuck.push("palette"); app.command_palette = None; }
    // a toolbar dropdown
    let screen = draw(&mut app, 120, 30); click(&mut app, locate(&screen, "⚙")); assert!(app.toolbar.menu_open());
    corner(&mut app); if app.toolbar.menu_open() { stuck.push("toolbar menu"); key(&mut app, KeyCode::Esc); }
    // help
    key(&mut app, KeyCode::Char('?')); assert!(app.prompt == Some(crate::app::Prompt::Help));
    corner(&mut app); if app.prompt.is_some() { stuck.push("help"); app.prompt = None; }
    // about
    app.about = Some(Default::default());
    corner(&mut app); if app.about.is_some() { stuck.push("about"); app.about = None; }
    // a right-click menu
    let screen = draw(&mut app, 120, 30); let at = locate(&screen, "b");
    app.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Right), column: at.0, row: at.1, modifiers: KeyModifiers::NONE }));
    assert!(app.modal_input(), "a menu opened");
    corner(&mut app); if app.modal_input() { stuck.push("context menu"); key(&mut app, KeyCode::Esc); }
    assert!(stuck.is_empty(), "a click outside leaves these open: {stuck:?}");
}

#[test]
fn tui_a_click_outside_closes_find() {
    let (_dir, mut app) = pair("alpha\nbeta\n", "alpha\nBETA\n");
    draw(&mut app, 120, 30);
    let corner = |app: &mut App| { app.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: 20, row: 20, modifiers: KeyModifiers::NONE })); draw(app, 120, 30); };
    app.handle(Event::Key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL)));
    for c in "beta".chars() { key(&mut app, KeyCode::Char(c)); }
    assert!(app.prompt == Some(crate::app::Prompt::Search));
    corner(&mut app);
    assert!(app.prompt.is_none(), "a click outside closes Find");
    app.handle(Event::Key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL | KeyModifiers::SHIFT)));
    assert!(app.set_search.as_ref().is_some_and(|s| s.model.editing()));
    corner(&mut app);
    assert!(app.set_search.as_ref().is_none_or(|s| !s.model.editing()), "a click outside closes Find in files while typed");
}

fn tui_fingerprint(app: &App) -> String {
    let core = app.repository.as_ref().map(|r| &r.core);
    format!("{:?}", (
        (app.session.options(), app.session.view().mode(), app.session.view().current_change(), app.session.view().unified(), app.session.current(), app.session.context_hidden()),
        (app.selection, app.prompt, app.command_palette.is_some(), app.toolbar.menu_open(), app.session.find().is_some(), app.set_search.as_ref().map(|s| s.model.editing())),
        (app.layout.panels(core), app.layout.focused(core), app.navigator.shown(), app.blame, app.session.dirty()),
        core.map(|c| (c.picker.branches.clone(), c.picker.compare.clone(), c.picker.anchor, c.picker.filter.clone(), c.pr.as_ref().and_then(|p| p.details().map(|d| d.tab)))),
    ))
}

fn tui_press(app: &mut App, shortcut: &str) {
    let parts: Vec<&str> = shortcut.split('-').collect();
    let (mods, name) = parts.split_at(parts.len() - 1);
    let mut m = KeyModifiers::NONE;
    if mods.contains(&"ctrl") { m |= KeyModifiers::CONTROL; }
    if mods.contains(&"shift") { m |= KeyModifiers::SHIFT; }
    if mods.contains(&"alt") { m |= KeyModifiers::ALT; }
    let code = match name[0] {
        "escape" => KeyCode::Esc, "left" => KeyCode::Left, "right" => KeyCode::Right, "" => KeyCode::Char('-'),
        k => { let c = k.chars().next().unwrap(); KeyCode::Char(if m.contains(KeyModifiers::SHIFT) { c.to_ascii_uppercase() } else { c }) }
    };
    app.handle(Event::Key(KeyEvent::new(code, m)));
}

fn tui_check_shortcuts(make: &dyn Fn() -> (tempfile::TempDir, App), wrong: &mut Vec<String>) {
    let (_d, mut probe) = make();
    draw(&mut probe, 160, 36);
    let entries: Vec<_> = crate::palette::terminal_keys(lenz::catalog::entries(&probe.scope())).into_iter().filter(|e| e.shortcut.is_some()).collect();
    let prep = |app: &mut App| { draw(app, 160, 36); app.navigator.focused = false; if let Some(r) = app.repository.as_mut() { r.core.execute(lenz::repository_session::RepositoryCommand::Focus(lenz::repository_session::Pane::Diff)); } };
    let settle = |app: &mut App| { for _ in 0..30 { draw(app, 160, 36); std::thread::sleep(std::time::Duration::from_millis(2)); } };
    for e in entries {
        let shortcut = e.shortcut.unwrap();
        let (_a, mut by_key) = make(); prep(&mut by_key); tui_press(&mut by_key, shortcut); settle(&mut by_key);
        let (_b, mut by_cmd) = make(); prep(&mut by_cmd); by_cmd.run_action(e.action); settle(&mut by_cmd);
        let (k, c) = (tui_fingerprint(&by_key), tui_fingerprint(&by_cmd));
        if k != c { wrong.push(format!("{shortcut} for {:?} ({})\n  key: {k}\n  cmd: {c}", e.action, e.label)); }
    }
}

#[test]
fn tui_palette_shortcuts_do_what_they_say() {
    let mut wrong = Vec::new();
    tui_check_shortcuts(&|| pair("a\nold one\nb\nc\nd\ne\nf\ng\nh\ni\nj\nold two\nk\n", "a\nnew one\nb\nc\nd\ne\nf\ng\nh\ni\nj\nnew two\nk\n"), &mut wrong);
    tui_check_shortcuts(&two_commit_log, &mut wrong);
    tui_check_shortcuts(&pr_log, &mut wrong);
    tui_check_shortcuts(&|| { let (d, mut app) = two_commit_log(); app.layout_command(lenz::layout::LayoutCommand::ToggleFocus); (d, app) }, &mut wrong);
    tui_check_shortcuts(&|| { let (d, mut app) = pr_log(); app.layout_command(lenz::layout::LayoutCommand::ToggleFocus); (d, app) }, &mut wrong);
    assert!(wrong.is_empty(), "shortcuts that do not match their entry:\n{}", wrong.join("\n"));
}

#[test]
fn tui_focus_shows_the_commit_message_above_the_files() {
    let (_d, mut app) = two_commit_log();
    let overview = text(&draw(&mut app, 160, 40));
    assert!(!overview.contains("▾ Subject two"), "the Overview has no panel");
    app.layout_command(lenz::layout::LayoutCommand::ToggleFocus);
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !text(&draw(&mut app, 160, 40)).contains("▾ Subject two") { app.poll(); assert!(std::time::Instant::now() < until, "message read"); std::thread::sleep(std::time::Duration::from_millis(2)); }
    let screen = draw(&mut app, 160, 40);
    // the file list starts below the panel, and its rows still open their files
    let (_, subject_row) = locate(&screen, "▾ Subject two");
    let file = locate(&screen, "M a.txt");
    assert!(file.1 > subject_row + 1, "{}", text(&screen));
    click(&mut app, file);
    // right click: copy the commit under the pointer
    let at = locate(&screen, "▾ Subject two");
    app.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Right), column: at.0, row: at.1, modifiers: KeyModifiers::NONE }));
    let menu = text(&draw(&mut app, 160, 40));
    assert!(menu.contains("Copy commit message") && menu.contains("Copy commit hash"), "{menu}");
    key(&mut app, KeyCode::Esc);
    // with the menu open, a click on the panel only closes the menu
    app.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Right), column: at.0, row: at.1 + 1, modifiers: KeyModifiers::NONE }));
    assert!(text(&draw(&mut app, 160, 40)).contains("Copy commit message"));
    click(&mut app, at);
    let after = text(&draw(&mut app, 160, 40));
    assert!(after.contains("▾ Subject two") && !after.contains("Copy commit message"), "{after}");
    // its first line collapses it, and the palette brings it back
    let at = locate(&draw(&mut app, 160, 40), "▾ Subject two");
    click(&mut app, at);
    assert!(text(&draw(&mut app, 160, 40)).contains("▸ Subject two"));
    app.run_action(lenz::catalog::Action::Repository(lenz::catalog::RepoAction::ToggleMessage));
    assert!(text(&draw(&mut app, 160, 40)).contains("▾ Subject two"));

    // a range: ↑ from the first file enters the panel, ↓ walks the commits, Enter opens one, Esc gives Files back
    let d = _d.path();
    std::fs::write(d.join("a.txt"), "third\n").unwrap(); git(d, &["commit", "-qam", "Subject three"]);
    let spec = lenz::repository_session::branch_comparison("HEAD~2", "HEAD", false);
    let set = lenz::changeset::ChangeSet::load(&spec, d).unwrap();
    app.repository.as_mut().unwrap().core.showing(std::sync::Arc::new(set), spec);
    while !text(&draw(&mut app, 160, 40)).contains("▾ 2 commits") { app.poll(); assert!(std::time::Instant::now() < until, "range read"); std::thread::sleep(std::time::Duration::from_millis(2)); }
    app.navigator.focused = true;
    app.commit_panel.focused = false;
    key(&mut app, KeyCode::Up);
    assert!(app.commit_panel.focused && !app.navigator.focused);
    key(&mut app, KeyCode::Down);
    assert_eq!(app.repository.as_ref().unwrap().core.message_cursor(), 1);
    key(&mut app, KeyCode::Enter);
    let opened = text(&draw(&mut app, 160, 40));
    assert!(opened.contains("▾ Subject two") && opened.contains("▸ Subject three"), "{opened}");
    assert!(app.handle(Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))));
    assert!(!app.quit, "Ctrl+C in the panel copies");
    key(&mut app, KeyCode::Esc);
    assert!(!app.commit_panel.focused && app.navigator.focused);
}

#[test]
fn tui_pr_conversation_clicks_fold_jump_open_commits_and_copy() {
    use lenz::catalog::{Action, PrAction};
    use lenz::pr_session::DetailsTab;
    let (_dir, mut app) = pr_log();
    draw(&mut app, 160, 40);
    let path = app.session.comparisons()[0].path.clone();
    {
        let thread = lenz::pr::Thread { path: path.clone(), diff_side: lenz::view::Side::Right, line: Some(1), start_line: None, original_line: Some(1), is_outdated: false, is_resolved: true,
            comments: vec![lenz::pr::Comment { author: Some(lenz::pr::Account { login: "bob".into() }), body: "THREAD_BODY".into(), created_at: "2026-10-05T00:00:00Z".into(), url: "https://x/c1".into(), ..Default::default() }], subject_type: String::new() };
        let mut review = lenz::pr::Review { placed: [((path.clone(), lenz::view::Side::Right, 1), vec![thread])].into(), ..Default::default() };
        review.overview.commits = vec![lenz::pr::PrCommit { sha: "abc1234567".into(), message: "Commit subject\n\nCommit body".into(), author: "octocat".into(), date: "2026-10-05T00:00:00Z".into() }];
        review.overview.commit_total = 1;
        app.repository.as_mut().unwrap().core.pr.as_mut().unwrap().seed_review(review, &[]).unwrap();
    }
    app.run_action(Action::Pr(PrAction::Details(DetailsTab::Conversation)));
    key(&mut app, KeyCode::End);
    let screen = draw(&mut app, 160, 40);
    assert!(!text(&screen).contains("THREAD_BODY"), "a resolved thread starts folded");
    click(&mut app, locate(&screen, "▸ "));
    let screen = draw(&mut app, 160, 40);
    assert!(text(&screen).contains("THREAD_BODY"), "a click unfolds it: {}", text(&screen));
    // right click on the card: copy its comment or link
    let at = locate(&screen, "THREAD_BODY");
    app.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Right), column: at.0, row: at.1, modifiers: KeyModifiers::NONE }));
    let menu = text(&draw(&mut app, 160, 40));
    assert!(menu.contains("Copy comment") && menu.contains("Copy link to comment"), "{menu}");
    key(&mut app, KeyCode::Esc);
    // path:line jumps to Files changed
    let screen = draw(&mut app, 160, 40);
    click(&mut app, locate(&screen, &format!("{path}:1 ↗")));
    assert!(app.repository.as_ref().unwrap().core.pr.as_ref().unwrap().details().is_none(), "Files changed");
    // Commits: a commit's hash opens Files changed for it alone
    app.run_action(Action::Pr(PrAction::Details(DetailsTab::Commits)));
    let screen = draw(&mut app, 160, 40);
    assert!(text(&screen).contains("Commit subject"));
    click(&mut app, locate(&screen, "abc1234 ↗"));
    assert_eq!(app.repository.as_ref().unwrap().core.pr.as_ref().unwrap().shown_commit.as_deref(), Some("abc1234567"));
}

#[test]
fn tui_pr_tabs_copy_text_hide_files_keep_their_place_and_sit_in_the_middle() {
    use lenz::catalog::{Action, PrAction};
    use lenz::pr_session::DetailsTab;
    let (_dir, mut app) = pr_log();
    {
        let mut review = lenz::pr::Review::default();
        review.overview.commits = (0..80).map(|i| lenz::pr::PrCommit { sha: format!("{i:07}abc"), message: format!("Commit {i}"), author: "octocat".into(), date: "2026-10-05T00:00:00Z".into() }).collect();
        review.overview.commit_total = 80;
        app.repository.as_mut().unwrap().core.pr.as_mut().unwrap().seed_review(review, &[]).unwrap();
    }
    draw(&mut app, 160, 36);
    app.run_action(Action::Pr(PrAction::Details(DetailsTab::Conversation)));
    let screen = draw(&mut app, 160, 36);
    assert!(!app.navigator.shown(), "no file list on Conversation");
    // the tabs sit in the middle of the toolbar
    let bar = toolbar_row(&mut app, 160);
    let (start, end) = (bar.find("Conversation").unwrap(), bar.find("Files changed").unwrap() + "Files changed".len());
    let (start, end) = (bar[..start].chars().count(), bar[..end].chars().count());
    assert!(((start + end) / 2).abs_diff(80) <= 3, "{bar}");
    assert!(app.header_switch.is_empty(), "not in the diff header");
    assert_eq!(locate(&screen, "PR description  #9").1, app.diff_area.y, "the tab starts at the diff area's top, no empty rows");
    // a drag selects text and Ctrl+C copies it (not quitting)
    let at = locate(&screen, "Description line");
    click(&mut app, at);
    app.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Drag(MouseButton::Left), column: at.0 + 11, row: at.1, modifiers: KeyModifiers::NONE }));
    app.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Up(MouseButton::Left), column: at.0 + 11, row: at.1, modifiers: KeyModifiers::NONE }));
    let screen = draw(&mut app, 160, 36);
    assert_eq!(screen[at].bg, crate::chrome::color(app.palette.sel_bg), "the selection shows");
    assert!(app.handle(Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))));
    assert!(!app.quit, "Ctrl+C with a selection copies");
    // Commits keeps its place across Files changed
    app.run_action(Action::Pr(PrAction::Details(DetailsTab::Commits)));
    draw(&mut app, 160, 36);
    key(&mut app, KeyCode::PageDown);
    let scrolled = text(&draw(&mut app, 160, 36));
    key(&mut app, KeyCode::Esc);
    draw(&mut app, 160, 36);
    assert!(app.navigator.shown(), "Files changed has the file list");
    app.run_action(Action::Pr(PrAction::Details(DetailsTab::Commits)));
    assert_eq!(text(&draw(&mut app, 160, 36)), scrolled, "back where it was");
    // Back from one commit returns to the whole PR's Commits
    app.repository.as_mut().unwrap().core.execute(lenz::repository_session::RepositoryCommand::PrCommit(Some("0000001abc".into())));
    app.run_action(Action::Repository(lenz::catalog::RepoAction::Back));
    let pr = app.repository.as_ref().unwrap().core.pr.as_ref().unwrap();
    assert_eq!((pr.shown_commit.as_deref(), pr.details().map(|d| d.tab)), (None, Some(DetailsTab::Commits)));
}

#[test]
fn tui_pr_has_no_toolbar_menu_and_refreshes_from_its_sidebar_row() {
    let (_dir, mut app) = pr_log();
    {
        let core = &mut app.repository.as_mut().unwrap().core;
        core.picker.sources.prs = vec![lenz::pr::ListedPr { number: 9, title: "Sidebar PR".into(), author: lenz::pr::Account { login: "a".into() }, head_ref_name: "h".into(), base_ref_name: "b".into(), is_draft: false, updated_at: String::new() }];
    }
    let bar = toolbar_row(&mut app, 160);
    assert!(!bar.contains("PR ▾") && !bar.contains("Viewed"), "{bar}");
    let screen = draw(&mut app, 160, 36);
    let at = locate(&screen, "#9 Sidebar PR");
    app.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Right), column: at.0, row: at.1, modifiers: KeyModifiers::NONE }));
    let menu = text(&draw(&mut app, 160, 36));
    assert!(menu.contains("Refresh pull request"), "{menu}");
}

#[test]
fn tui_a_pr_opened_on_conversation_still_lists_files_on_files_changed() {
    let (_dir, mut app) = pr_log();
    // as `lenz-tui pr N` does: before Files was ever drawn, the session shows Conversation and the app enters Focus
    app.repository.as_mut().unwrap().core.pr.as_mut().unwrap().execute(lenz::pr_session::PrCommand::Show(lenz::pr_session::DetailsTab::Conversation));
    app.poll();
    draw(&mut app, 160, 36);
    assert!(app.layout.focused(app.repository.as_ref().map(|r| &r.core)) && !app.navigator.shown());
    app.run_action(lenz::catalog::Action::Pr(lenz::catalog::PrAction::Diff));
    draw(&mut app, 160, 36);
    assert!(app.navigator.shown(), "Files changed lists the files");
}

#[test]
fn markdown_tui_side_by_side_marks_an_added_section_by_its_bar_alone() {
    use ratatui::style::Modifier;
    let old = "# Title\n\nThe service keeps accounts in balance.\n\n## End\n\nBye.\n";
    let new = "# Title\n\nThe service keeps every account in balance.\n\n## Importing\n\nOpening balances come from a CSV file with one row per account.\n\n## End\n\nBye.\n";
    let (_dir, mut app) = pair_markdown(old, new);
    let screen = rendered_markdown(&mut app, 160, 36);
    let palette = app.palette.clone();
    let (x, y) = locate(&screen, "Opening balances");
    let row: String = (0..screen.area.width).map(|c| screen[(c, y)].symbol()).collect();
    let bar = row.find('▌').map(|b| row[..b].chars().count() as u16).expect("the added paragraph has a change mark");
    assert_eq!(screen[(bar, y)].fg, crate::chrome::color(palette.block[0]), "in the added color");
    assert!((x..x + 16).all(|c| !screen[(c, y)].modifier.contains(Modifier::UNDERLINED)), "no word outlines on a block added whole");
    // the paragraph edited in place keeps its word outline
    let (x, y) = locate(&screen, "every account");
    assert!(screen[(x, y)].modifier.contains(Modifier::UNDERLINED), "the changed words stay outlined");
}

#[test]
fn the_bottom_bar_spans_the_window_under_the_files_panel() {
    let dir = tempfile::tempdir().unwrap();
    for (side, text) in [("left", "a\n"), ("right", "b\n")] {
        std::fs::create_dir(dir.path().join(side)).unwrap();
        for name in ["one.txt", "two.txt"] { std::fs::write(dir.path().join(side).join(name), text).unwrap(); }
    }
    let req = lenz::cli::parse(&["--no-index".into(), "left".into(), "right".into()], dir.path()).unwrap();
    let mut app = App::open(req, dir.path().into());
    ready(&mut app);
    let (w, h) = (120, 30);
    let screen = draw(&mut app, w, h);
    assert!(app.navigator.shown(), "a folder comparison shows Files");
    let row = |y: u16| (0..w).map(|x| screen[(x, y)].symbol()).collect::<String>();
    let bottom = row(h - 1);
    let files = bottom.find("Files").expect("the panel switch is on the bottom bar");
    assert!(bottom[..files].chars().count() <= 3, "it starts at the window's left edge: {bottom:?}");
    assert!(bottom.trim_end().ends_with("q quit"), "the keys end at the right edge: {bottom:?}");
    let bar = [app.palette.status_bg, app.palette.selected_bg].map(crate::chrome::color);
    assert!((0..w).all(|x| bar.contains(&screen[(x, h - 1)].bg)), "one bar across the window (the shown panel's switch selected)");
    assert!(row(h - 2).starts_with('└'), "the Files panel ends above the bar: {:?}", row(h - 2));
}
