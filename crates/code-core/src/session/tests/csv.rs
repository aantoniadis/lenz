use super::*;
use crate::csv::Command as C;
fn csv(s: &Session) -> &crate::csv::View { s.state.csv(s.state.active.unwrap()).unwrap() }
fn command(s: &mut Session, c: C) { s.dispatch(Command::Csv(c)); }
fn ready(s: &mut Session) { settle(s, |s| s.state.active.and_then(|tab| s.state.csv(tab)).is_some_and(|view| !view.pending)); }
fn fixture(source: &str) -> (tempfile::TempDir, Session, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "records.csv", source.as_bytes());
    let mut s = session(dir.path()); s.enable_text_input_for_test(); open(&mut s, Command::OpenRetained(path.clone()), "records.csv");
    command(&mut s, C::Toggle); ready(&mut s); (dir, s, path)
}
#[test]
fn filtered_sorted_csv_edits_target_source_rows_and_columns_preserving_raw_bytes_and_undo_save() {
    let source = "name,note,id\r\nb,\"two\nlines\",1\r\na,\"x,y\",2\r\na,tail,3";
    let (_dir, mut s, path) = fixture(source);
    assert_eq!((csv(&s).row_count(), csv(&s).column_count()), (3,3));
    assert_eq!(s.dispatch(Command::Copy).copy, Some("b".into()), "Copy uses the selected table cell, never hidden source selection");
    assert_eq!(s.action(CommandId::FindInFile), crate::actions::CatalogAction::Command(Command::Csv(C::BeginFilter)));
    command(&mut s, C::BeginFilter); command(&mut s, C::SetInput("a".into())); command(&mut s, C::Commit); ready(&mut s);
    assert_eq!(csv(&s).row_count(), 2);
    command(&mut s, C::Select { row: 0, column: 2 }); command(&mut s, C::Sort); ready(&mut s); command(&mut s, C::Sort); ready(&mut s);
    assert_eq!(csv(&s).cell(0,2), "3");
    assert_eq!(std::fs::read(&path).unwrap(), source.as_bytes(), "filter/sort never write");
    command(&mut s, C::Select { row: 0, column: 1 }); command(&mut s, C::BeginEdit);
    command(&mut s, C::SetInput("new,\"quote\"\n".into())); command(&mut s, C::Commit); ready(&mut s);
    let edited = "name,note,id\r\nb,\"two\nlines\",1\r\na,\"x,y\",2\r\na,\"new,\"\"quote\"\"\n\",3";
    assert_eq!(source_text(s.state.current_source().unwrap()), edited);
    command(&mut s, C::MoveColumn { right: true }); ready(&mut s);
    let moved = "name,id,note\r\nb,1,\"two\nlines\"\r\na,2,\"x,y\"\r\na,3,\"new,\"\"quote\"\"\n\"";
    assert_eq!(source_text(s.state.current_source().unwrap()), moved);
    command(&mut s, C::DropColumn); ready(&mut s);
    assert_eq!(source_text(s.state.current_source().unwrap()), "name,id\r\nb,1\r\na,2\r\na,3");
    let tab = s.state.active.unwrap();
    for expected in [moved, edited, source] { s.dispatch(Command::Undo { tab }); ready(&mut s); assert_eq!(source_text(s.state.current_source().unwrap()), expected); }
    s.dispatch(Command::Redo { tab }); ready(&mut s);
    s.dispatch(Command::Save { tab }); settle(&mut s, |s| !s.state.tab_dirty(tab));
    assert_eq!(std::fs::read_to_string(path).unwrap(), edited);
}
#[test]
fn csv_source_input_is_blocked_and_split_filters_are_independent_and_empty_layout_clamps() {
    let (_dir, mut s, path) = fixture("h,id\na,1\nb,2\n");
    let tab = s.state.active.unwrap();
    let key = s.state.current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected:key, edits:vec![crate::text::TextEdit{range:0..1,text:"destroy".into()}],kind:crate::text::EditKind::Replace });
    s.dispatch(Command::Paste("destroy".into()));
    assert_eq!(source_text(s.state.current_source().unwrap()), "h,id\na,1\nb,2\n");
    s.dispatch(Command::Split(Split::Columns)); ready(&mut s);
    let other = s.state.other.unwrap();
    command(&mut s, C::BeginFilter); command(&mut s,C::SetInput("a".into())); command(&mut s,C::Commit); ready(&mut s);
    assert_eq!(csv(&s).row_count(),1); assert_eq!(s.state.csv(other).unwrap().row_count(),2);
    command(&mut s,C::Size {rows:1,columns:1}); command(&mut s,C::Move {rows:isize::MAX,columns:isize::MAX}); command(&mut s,C::Move {rows:-1,columns:-1});
    assert_eq!((csv(&s).row,csv(&s).column,csv(&s).top,csv(&s).left),(0,0,0,0));
    let active=s.state.active.unwrap(); command(&mut s,C::Toggle); assert!(s.state.csv(active).is_none());
    assert_eq!(std::fs::read(path).unwrap(), b"h,id\na,1\nb,2\n");
    let (_dir,mut s,_path)=fixture(""); command(&mut s,C::Move {rows:isize::MAX,columns:isize::MAX}); command(&mut s,C::BeginEdit); command(&mut s,C::Commit);
    assert_eq!((csv(&s).row_count(),csv(&s).column_count(),csv(&s).top,csv(&s).left),(0,0,0,0));
}
#[test]
fn csv_stale_projection_cannot_reopen_source_or_replace_new_filter_and_malformed_falls_back() {
    let (_dir,mut s,_path)=fixture("h\na\nb\n");
    command(&mut s,C::BeginFilter); command(&mut s,C::SetInput("a".into())); command(&mut s,C::Commit);
    let rx=s.completions();
    let stale=loop {let c=rx.recv_blocking_timeout().unwrap();if matches!(&c,Completion::Analyzed{result,..} if matches!(result.as_ref(),Completion::CsvPrepared(_))){break c;}s.dispatch(c);};
    command(&mut s,C::Toggle); command(&mut s,C::Toggle);
    s.dispatch(stale); ready(&mut s); assert_eq!(csv(&s).row_count(),2);
    command(&mut s,C::Toggle); let tab=s.state.active.unwrap();let expected=s.state.current_source().unwrap().key();
    s.dispatch(Command::Edit {tab,expected,edits:vec![crate::text::TextEdit{range:0..expected_len(&s),text:"\"broken".into()}],kind:crate::text::EditKind::Replace});
    command(&mut s,C::Toggle); settle(&mut s,|s|s.state.csv(tab).is_none());
    assert_eq!(source_text(s.state.current_source().unwrap()),"\"broken");
    assert!(s.state.notices().iter().any(|message|message.contains("unclosed quote")));
}
fn expected_len(s:&Session)->usize{s.state.current_source().unwrap().len_bytes()}

#[test]
fn csv_pending_field_refuses_save_undo_close_readonly_and_tab_transitions_with_shared_reason() {
    let (dir,mut s,path)=fixture("h,v\r\na,old\r\n");let tab=s.state.active.unwrap();
    let other=write(dir.path(),"other.txt",b"other\n");open(&mut s,Command::OpenRetained(other.clone()),"other.txt");s.dispatch(Command::ActivateTab(tab));ready(&mut s);
    command(&mut s,C::Select{row:0,column:1});command(&mut s,C::BeginEdit);command(&mut s,C::SetInput("pending λ".into()));
    let snapshot=s.state.current_source().unwrap().key();let count=s.state.tabs.len();
    for id in [CommandId::Save,CommandId::SaveAs,CommandId::Undo,CommandId::Redo,CommandId::Reload,CommandId::Close,CommandId::ReadOnlyMode,CommandId::SplitRight] {assert!(s.disabled(id).is_some_and(|reason|reason.contains("Enter or Escape")),"{id:?}");}
    for c in [Command::Save{tab},Command::SaveAs{tab,path:dir.path().join("must-not-exist.csv")},Command::Undo{tab},Command::Redo{tab},Command::Reload,Command::Close,Command::CloseTab(tab),Command::SetReadOnly(true),Command::Split(Split::Columns),Command::Open(other),Command::PrepareClose{batch:42,intent:crate::close::Intent::Session}] {
        s.dispatch(c);assert_eq!(s.state.active,Some(tab));assert_eq!(s.state.current_source().unwrap().key(),snapshot);assert_eq!(s.state.tabs.len(),count);assert_eq!(csv(&s).input.as_ref().unwrap().value,"pending λ");assert_eq!(std::fs::read(&path).unwrap(),b"h,v\r\na,old\r\n");
    }
    assert!(!s.closed());assert!(!dir.path().join("must-not-exist.csv").exists());
    command(&mut s,C::Size{rows:1,columns:1});assert_eq!(csv(&s).left,1,"narrow resize keeps the edited cell visible");
    command(&mut s,C::Commit);ready(&mut s);s.dispatch(Command::Save{tab});settle(&mut s,|s|!s.state.tab_dirty(tab));assert_eq!(std::fs::read(path).unwrap(),"h,v\r\na,pending λ\r\n".as_bytes());
}

#[test]
#[ignore = "release CSV component and sustained transaction probe; run alone"]
fn csv_release_workload_costs() {
    use std::time::Instant;
    let source:Arc<str>=format!("key,note,id\r\n{}",(0..199_999).map(|n|format!("key{:04},\"row,{n} λ\",{n}\r\n",n%1000)).collect::<String>()).into();
    let mut samples=std::collections::BTreeMap::<&str,Vec<f64>>::new();
    let mut table=None;
    for _ in 0..5 {let started=Instant::now();let parsed=crate::csv::parse(source.clone(),||true).unwrap();samples.entry("parse_200k_records_ms").or_default().push(started.elapsed().as_secs_f64()*1000.);assert_eq!(parsed.records.len(),200_000);assert_eq!(parsed.columns,3);table=Some(Arc::new(parsed));}
    let table=table.unwrap();
    let mut ready=None;
    for _ in 0..5 {
        let started=Instant::now();let projection=crate::csv::prepare(table.clone(),true,"key0007",0,None,||true).unwrap();samples.entry("filter_200k_records_ms").or_default().push(started.elapsed().as_secs_f64()*1000.);assert_eq!(projection.rows.len(),200);
        let started=Instant::now();let projection=crate::csv::prepare(table.clone(),true,"",0,Some((0,false)),||true).unwrap();samples.entry("sort_200k_records_ms").or_default().push(started.elapsed().as_secs_f64()*1000.);assert_eq!(table.value(projection.rows[0],0),"key0000");assert_eq!(projection.rows.len(),199_999);ready=Some(Arc::new(projection));
        let started=Instant::now();let edits=crate::csv::columns(&table,&[2,0,1],||true).unwrap();samples.entry("move_column_200k_records_ms").or_default().push(started.elapsed().as_secs_f64()*1000.);assert_eq!(edits.len(),1);assert!(edits[0].text.starts_with("id,key,note\r\n0,key0000,\"row,0 λ\"\r\n"));
        let started=Instant::now();let edits=crate::csv::columns(&table,&[0,2],||true).unwrap();samples.entry("drop_column_200k_records_ms").or_default().push(started.elapsed().as_secs_f64()*1000.);assert!(edits[0].text.starts_with("key,id\r\nkey0000,0\r\n"));
    }
    let ready=ready.unwrap();let projection_bytes=ready.rows.capacity()*std::mem::size_of::<usize>();
    let view=crate::csv::View{data:Some(ready),pending:false,..crate::csv::View::default()};
    for _ in 0..7 {let started=Instant::now();for _ in 0..100 {for row in 50_000..50_060 {for column in 0..3 {let shown=view.cell_preview(row,column,64);assert!(!shown.is_empty());std::hint::black_box(shown);}}}samples.entry("viewport_60x3_ms").or_default().push(started.elapsed().as_secs_f64()*10.);}
    let small=format!("name,note,id\r\n{}",(0..500).map(|n|format!("key{n},note,{n}\r\n")).collect::<String>());let (_dir,mut s,path)=fixture(&small);let tab=s.state.active.unwrap();
    let started=Instant::now();let mut cycles=0;let mut memory=vec![(0.0,memory_mib())];let mut next_memory=1.0;
    while cycles<100 || started.elapsed().as_secs_f64()<10.0 {command(&mut s,C::Select{row:0,column:1});command(&mut s,C::BeginEdit);command(&mut s,C::SetInput("edited,λ".into()));command(&mut s,C::Commit);ready_session(&mut s);s.dispatch(Command::Undo{tab});ready_session(&mut s);assert_eq!(source_text(s.state.current_source().unwrap()),small);cycles+=1;let elapsed=started.elapsed().as_secs_f64();if elapsed>=next_memory{memory.push((elapsed,memory_mib()));next_memory=elapsed+1.0;}}
    let duration=started.elapsed().as_secs_f64();memory.push((duration,memory_mib()));samples.entry("edit_undo_sustained_total_ms").or_default().push(duration*1000.);assert_eq!(std::fs::read_to_string(path).unwrap(),small);
    let high_water_kib=std::fs::read_to_string("/proc/self/status").ok().and_then(|status|status.lines().find_map(|line|line.strip_prefix("VmHWM:")).and_then(|value|value.split_whitespace().next()?.parse::<u64>().ok()));
    eprintln!("CSV_PERF {}",serde_json::json!({"samples":samples,"source_bytes":source.len(),"parsed_source_and_index_bytes":table.bytes(),"projection_index_bytes":projection_bytes,"transaction_cycles":cycles,"actual_transaction_stress_seconds":duration,"memory_kind":memory_kind(),"memory_timeline_seconds_mib":memory,"linux_high_water_kib":high_water_kib,"scope":"candidate-only Rust component parse/filter/stable-sort/move/drop on 200000 records, bounded viewport decoding and 500-row Session edit/undo; excludes adapters, display and physical storage"}));
}
fn ready_session(s:&mut Session){ready(s)}

#[test]
fn csv_external_source_replacement_preserves_pending_field_until_explicit_cancel() {
    let (_dir,mut s,path)=fixture("h,v\na,old\n");let tab=s.state.active.unwrap();
    command(&mut s,C::Select{row:0,column:1});command(&mut s,C::BeginEdit);command(&mut s,C::SetInput("keep this λ".into()));
    let replacement=Arc::new(Document::decode(path.clone(),s.versions.fetch_add(1,Ordering::Relaxed)+1,b"h,v\nb,external\n"));
    std::fs::write(&path,b"h,v\nb,external\n").unwrap();
    let token=s.state.active().unwrap().refresh;
    s.dispatch(Completion::Refreshed{tab,token,result:Refresh::Changed(Stamp::of(&path).unwrap(),replacement)});
    assert_eq!(csv(&s).input.as_ref().unwrap().value,"keep this λ");assert!(csv(&s).error.as_deref().unwrap().contains("source changed"));
    command(&mut s,C::Commit);assert_eq!(csv(&s).input.as_ref().unwrap().value,"keep this λ");assert_eq!(s.dispatch(Command::Copy).copy,Some("keep this λ".into()));
    assert_eq!(source_text(s.state.current_source().unwrap()),"h,v\nb,external\n");
    command(&mut s,C::Cancel);ready(&mut s);assert_eq!(csv(&s).cell(0,1),"external");assert!(csv(&s).error.is_none());
    command(&mut s,C::BeginEdit);command(&mut s,C::SetInput("keep after delete".into()));
    let token=s.state.active().unwrap().refresh;
    std::fs::remove_file(&path).unwrap();
    s.dispatch(Completion::Refreshed{tab,token,result:Refresh::Gone("deleted externally".into())});
    command(&mut s,C::Commit);assert_eq!(csv(&s).input.as_ref().unwrap().value,"keep after delete");assert!(!path.exists());
    let buffer=s.state.current_source().unwrap().key().buffer;
    s.state.sources.buffers.get_mut(&buffer).unwrap().snapshot=None;
    assert_eq!(s.dispatch(Command::Copy).copy,Some("keep after delete".into()));
    command(&mut s,C::Cancel);assert!(csv(&s).input.is_none());
    command(&mut s,C::Toggle);assert!(s.state.csv(tab).is_none());
}

#[test]
fn csv_column_permission_change_keeps_source_and_filter_column_unchanged() {
    let (_dir,mut s,path)=fixture("name,note,id\na,first,1\nb,second,2\n");
    command(&mut s,C::BeginFilter);command(&mut s,C::SetInput("a".into()));command(&mut s,C::Commit);ready(&mut s);
    let before=source_text(s.state.current_source().unwrap());
    command(&mut s,C::MoveColumn{right:true});
    let rx=s.completions();let pending=loop{let c=rx.recv_blocking_timeout().unwrap();if matches!(&c,Completion::Analyzed{result,..} if matches!(result.as_ref(),Completion::CsvPrepared(_))){break c;}s.dispatch(c);};
    s.dispatch(Command::SetReadOnly(true));s.dispatch(pending);
    assert_eq!(source_text(s.state.current_source().unwrap()),before);assert_eq!(csv(&s).filter_column,0);assert_eq!(csv(&s).column,0);assert_eq!(csv(&s).row_count(),1);assert_eq!(csv(&s).cell(0,0),"a");assert_eq!(std::fs::read_to_string(path).unwrap(),before);
}
