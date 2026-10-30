use super::*;
use code_core::csv::Command as C;
fn table(a: &App) -> &code_core::csv::View { a.session.state().csv(a.session.state().active().unwrap().id()).unwrap() }
fn ready(a: &mut App) { settle(a, |a| a.session.state().active().and_then(|tab| a.session.state().csv(tab.id())).is_some_and(|table| !table.pending)); }
#[test]
fn csv_terminal_keys_mouse_inline_edit_filter_sort_move_drop_undo_and_source() {
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("rows.csv");let source="name,note,id\r\nb,\"two\nlines\",1\r\na,\"x,y\",2\r\na,tail,3";
    std::fs::write(&path,source).unwrap();let mut a=App::new(config(dir.path()),Some(path.clone()));a.enable_draft_input_for_test();
    settle(&mut a,|a|a.session.state().current_source().is_some());draw(&mut a,100,20);
    let button=a.layout.controls.iter().find(|(id,_)|*id==CommandId::CsvTable).unwrap().1;click(&mut a,button.x,button.y);ready(&mut a);
    assert!(draw_screen(&mut a,100,20).contains("name"));
    key(&mut a,KeyCode::Char('/'),KeyModifiers::NONE);key(&mut a,KeyCode::Char('a'),KeyModifiers::NONE);key(&mut a,KeyCode::Enter,KeyModifiers::NONE);ready(&mut a);assert_eq!(table(&a).row_count(),2);
    key(&mut a,KeyCode::Right,KeyModifiers::NONE);key(&mut a,KeyCode::Right,KeyModifiers::NONE);key(&mut a,KeyCode::Char('s'),KeyModifiers::NONE);ready(&mut a);key(&mut a,KeyCode::Char('s'),KeyModifiers::NONE);ready(&mut a);assert_eq!(table(&a).cell(0,2),"3");
    key(&mut a,KeyCode::Left,KeyModifiers::NONE);key(&mut a,KeyCode::Enter,KeyModifiers::NONE);key(&mut a,KeyCode::Char('a'),KeyModifiers::CONTROL);
    a.handle(Event::Paste("λ,\"edited\"".into()));
    let pending=draw_screen(&mut a,100,20);assert!(pending.contains("λ,\"edited\"▏"),"pending value is painted inline: {pending}");
    let before=a.session.state().current_source().unwrap().copy_range(0..source.len()).unwrap();
    key(&mut a,KeyCode::Char('s'),KeyModifiers::CONTROL);key(&mut a,KeyCode::Char('z'),KeyModifiers::CONTROL);a.dispatch(Command::SetReadOnly(true));
    assert_eq!(table(&a).input.as_ref().unwrap().value,"λ,\"edited\"");assert_eq!(a.session.state().current_source().unwrap().copy_range(0..source.len()).unwrap(),before);assert_eq!(std::fs::read_to_string(&path).unwrap(),source);
    key(&mut a,KeyCode::Enter,KeyModifiers::NONE);ready(&mut a);assert_eq!(table(&a).cell(0,1),"λ,\"edited\"");
    key(&mut a,KeyCode::Right,KeyModifiers::CONTROL|KeyModifiers::ALT);ready(&mut a);assert_eq!(table(&a).heading(2),"note");
    a.run(CommandId::CsvDropColumn);ready(&mut a);assert_eq!(table(&a).column_count(),2);
    key(&mut a,KeyCode::Char('z'),KeyModifiers::CONTROL);ready(&mut a);assert_eq!(table(&a).column_count(),3);
    for theme in ["darcula","github-light"] {a.dispatch(Command::Theme(lenz_content::theme::ThemeCommand::Select(lenz_content::theme::ThemeSelection::from_id(theme).unwrap())));for (w,h) in [(100,20),(20,6),(3,2),(1,1)] {draw(&mut a,w,h);}}
    draw(&mut a,100,20);let area=a.layout.panes[0].text;
    click(&mut a,area.x+1,area.y+1);assert_eq!(table(&a).column,0);
    a.handle(Event::Mouse(MouseEvent{kind:MouseEventKind::Down(MouseButton::Right),column:area.x+1,row:area.y+1,modifiers:KeyModifiers::NONE}));
    assert!(matches!(a.modal.as_ref().map(|m|&m.kind),Some(Prompt::Context{entries,..}) if entries.contains(&MenuEntry::Command(CommandId::CsvDropColumn))));
    key(&mut a,KeyCode::Esc,KeyModifiers::NONE);key(&mut a,KeyCode::Char('t'),KeyModifiers::NONE);assert!(a.session.state().csv(a.session.state().active().unwrap().id()).is_none());
    assert_eq!(std::fs::read_to_string(path).unwrap(),source);
}
#[test]
fn csv_terminal_long_cells_and_stale_clicks_are_bounded_and_modal_keys_never_edit_source() {
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("long.csv");let source=format!("h,v\na,\"{}\"\nb,short\n","\"\"".repeat(100_000));std::fs::write(&path,&source).unwrap();let mut a=App::new(config(dir.path()),Some(path.clone()));a.enable_draft_input_for_test();settle(&mut a,|a|a.session.state().current_source().is_some());a.run(CommandId::CsvTable);ready(&mut a);
    let started=Instant::now();draw(&mut a,80,12);assert!(started.elapsed()<Duration::from_secs(2));assert!(a.layout.panes[0].hits.is_empty(),"source hit-testing is disabled in Table");
    a.dispatch(Command::Csv(C::Select{row:0,column:1}));key(&mut a,KeyCode::Enter,KeyModifiers::NONE);assert!(table(&a).input.is_none(),"large field uses Source instead of copying into inline editor");
    let stale_area=a.layout.panes[0].text;
    a.dispatch(Command::Csv(C::Sort));
    click(&mut a,stale_area.x+1,stale_area.y+2);
    assert_eq!(table(&a).column,1,"stale table hit rectangles cannot select a differently projected cell");
    ready(&mut a);
    key(&mut a,KeyCode::F(10),KeyModifiers::NONE);for code in [KeyCode::Char('x'),KeyCode::Backspace,KeyCode::Delete,KeyCode::Enter]{key(&mut a,code,KeyModifiers::NONE);}key(&mut a,KeyCode::Esc,KeyModifiers::NONE);
    assert_eq!(std::fs::read_to_string(path).unwrap(),source);
}

#[test]
fn csv_terminal_partial_final_column_can_scroll_fully_into_view_and_old_scroll_hits_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("narrow.csv");
    let source = format!("a,b,c\n{}", "first,second,ABCDEFGHIJKLMNOPQ\n".repeat(20));
    std::fs::write(&path, &source).unwrap();
    let mut a = App::new(config(dir.path()), Some(path.clone())); a.enable_draft_input_for_test();
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.run(CommandId::CsvTable); ready(&mut a);
    draw(&mut a, 32, 12);
    a.dispatch(Command::Csv(C::Select { row: 0, column: 2 }));
    let screen = draw_screen(&mut a, 32, 12);
    assert!(screen.contains("ABCDEFGHIJKLMNOPQ"), "a partial last column must become fully visible when selected: {} cols={} left={} area={:?}", screen, table(&a).columns, table(&a).left, a.layout.panes[0].text);
    a.dispatch(Command::Csv(C::Select { row: 0, column: 0 })); draw(&mut a, 32, 12);
    let old = a.layout.panes[0].text;
    a.dispatch(Command::Csv(C::Scroll { rows: 4, columns: 1 }));
    let before = (table(&a).row, table(&a).column);
    click(&mut a, old.x + 1, old.y + 2);
    assert_eq!((table(&a).row, table(&a).column), before, "old scroll geometry cannot select an unintended source row/column");
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}
