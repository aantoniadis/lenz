use super::*;

fn fixture() -> (tempfile::TempDir, PathBuf, Session) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project"); std::fs::create_dir_all(&root).unwrap();
    let mut s = session(&dir.path().join("private-state"));
    write(&root,"a.txt", "λ foo\r\nfoo\nlast".as_bytes());
    write(&root,"b.txt", b"foo food\n");
    s.dispatch(Command::OpenRoot(root.clone())); settle(&mut s, scanned); s.enable_text_input_for_test();
    (dir,root,s)
}
fn search(s: &mut Session) {
    s.dispatch(Command::Search { query:"foo".into(), scope:SearchIn::Matching("*.txt".into()) });
    settle(s, |s| s.state().results().is_some_and(|r|r.done()));
    assert_eq!(s.disabled(CommandId::ProjectReplace),None);
}
fn preview(s: &mut Session, text: &str) {
    search(s); s.dispatch(Command::PreviewProjectReplace(text.into())); settle(s,|s|!s.workspace_busy());
    assert!(s.workspace_preview().is_some(),"{:?}",s.state().notices());
}
#[test]
fn workspace_replace_previews_selects_applies_to_drafts_and_undoes_per_file() {
    let (_d,root,mut s) = fixture(); preview(&mut s,"bar");
    assert_eq!(s.state().tabs().len(),0,"preview never opens tabs");
    assert!(s.workspace_preview().unwrap().text().contains("+ \"bar\""));
    s.dispatch(Command::ToggleWorkspaceFile(1));
    s.dispatch(Command::ApplyWorkspaceEdit); settle(&mut s,|s|!s.workspace_busy());
    assert_eq!(s.state().tabs().len(),1);
    let tab = s.state().tabs()[0].id();
    assert_eq!(source_text(s.state.source(tab).unwrap()),"λ bar\r\nbar\nlast");
    assert_eq!(std::fs::read(root.join("a.txt")).unwrap(),"λ foo\r\nfoo\nlast".as_bytes());
    s.dispatch(Command::Undo {tab});
    assert_eq!(source_text(s.state.source(tab).unwrap()),"λ foo\r\nfoo\nlast");
}
#[test]
fn workspace_replace_uses_unsaved_drafts_and_refuses_stale_apply_without_partial_changes() {
    let (_d,root,mut s) = fixture();
    open(&mut s,Command::OpenRetained(root.join("a.txt")),"a.txt");
    s.dispatch(Command::InsertText {text:"foo ".into(),kind:crate::text::EditKind::Paste});
    preview(&mut s,"bar");
    assert_eq!(s.workspace_preview().unwrap().files().len(),2);
    s.dispatch(Command::InsertText {text:"newer".into(),kind:crate::text::EditKind::Paste});
    s.dispatch(Command::ApplyWorkspaceEdit); settle(&mut s,|s|!s.workspace_busy());
    assert!(source_text(s.state().current_source().unwrap()).contains("newer"));
    assert!(source_text(s.state().current_source().unwrap()).contains("foo"));
    assert_eq!(s.state().tabs().len(),1,"no other affected file opened after failed validation");
    assert_eq!(std::fs::read(root.join("b.txt")).unwrap(),b"foo food\n");
}
#[test]
fn workspace_replace_disk_change_refuses_whole_preview_and_cancel_drops_late_reply() {
    let (_d,root,mut s) = fixture(); preview(&mut s,"bar");
    std::fs::write(root.join("b.txt"),b"different\n").unwrap();
    s.dispatch(Command::ApplyWorkspaceEdit); settle(&mut s,|s|!s.workspace_busy());
    assert!(s.state().tabs().is_empty());
    s.dispatch(Command::CancelWorkspaceEdit); assert!(s.workspace_preview().is_none());
    search(&mut s); s.dispatch(Command::PreviewProjectReplace("z".into())); s.dispatch(Command::CancelWorkspaceEdit);
    let request=0;
    s.dispatch(Completion::WorkspacePrepared {request,apply:false,result:Err("late".into())});
    assert!(s.workspace_preview().is_none());
}
#[test]
fn workspace_replace_empty_replacement_preserves_line_endings_and_regex_is_literal() {
    for replacement in ["$1", ""] {
        let (_d,_root,mut s)=fixture();
        s.dispatch(Command::ToggleFindOption(FindOption::Regex));
        preview(&mut s,replacement); s.dispatch(Command::ApplyWorkspaceEdit); settle(&mut s,|s|!s.workspace_busy());
        for tab in s.state().tabs() {
            let expected=if tab.source().ends_with("a.txt") { format!("λ {replacement}\r\n{replacement}\nlast") } else { format!("{replacement} {replacement}d\n") };
            assert_eq!(source_text(s.state.source(tab.id()).unwrap()),expected);
        }
    }
}

#[test]
#[cfg(unix)]
fn workspace_symbol_rename_roundtrips_a_server_reply_before_applying_multiple_drafts() {
    let d=tempfile::tempdir().unwrap(); let root=d.path().join("project");std::fs::create_dir(&root).unwrap();
    write(&root,"Cargo.toml",b"[package]\nname = \"rename-fixture\"\n");
    let path=write(&root,"a.rs",b"fn target() {}\n"); let other=write(&root,"b.rs",b"fn caller() { target(); }\n");
    let (mut s,_log)=fake_semantic_session_with_env(d.path(), &[("FAKE_LSP_RENAME","1")]);
    s.dispatch(Command::OpenRoot(root));settle(&mut s,scanned);s.enable_text_input_for_test();
    open(&mut s,Command::OpenRetained(other.clone()),"b.rs");
    s.dispatch(Command::InsertText {text:"// unsaved λ\n".into(),kind:crate::text::EditKind::Paste});
    let other_tab=s.state().active().unwrap().id();
    settle(&mut s,|s|desired_matches_current(s,&other));
    open(&mut s,Command::OpenRetained(path.clone()),"a.rs");
    s.dispatch(Command::Select(Selection {anchor:4,head:4}));
    settle(&mut s,|s|s.state.semantics.views().iter().any(|v|v.supports.contains(&semantics::Question::Rename)));
    s.dispatch(Command::RenameSymbol("renamed".into()));settle(&mut s,|s|!s.workspace_busy());
    assert!(s.workspace_preview().is_some(),"{:?}",s.state().notices());
    assert_eq!(s.workspace_preview().unwrap().files().len(),2);
    assert_eq!(std::fs::read(&other).unwrap(),b"fn caller() { target(); }\n");
    s.dispatch(Command::ApplyWorkspaceEdit);settle(&mut s,|s|!s.workspace_busy());
    assert_eq!(s.state().tabs().iter().filter_map(|t|s.state.source(t.id()).map(|s|s.key().buffer)).collect::<std::collections::HashSet<_>>().len(),2);
    for tab in s.state().tabs() {
        let source=s.state.source(tab.id()).unwrap();assert_eq!(source.source(),tab.source());
        let expected=if tab.source()==path { "fn renamed() {}\n" } else { "// unsaved λ\nfn caller() { renamed(); }\n" };
        assert_eq!(source_text(source),expected);
    }
    assert_eq!(std::fs::read(path).unwrap(),b"fn target() {}\n");
    s.dispatch(Command::Undo {tab:other_tab});
    assert_eq!(source_text(s.state.source(other_tab).unwrap()),"// unsaved λ\nfn caller() { target(); }\n");
}

#[test]
fn workspace_replace_keeps_existing_unsaved_text_and_undo_history() {
    let (_d,root,mut s)=fixture();
    let path=root.join("a.txt");
    open(&mut s,Command::OpenRetained(path.clone()),"a.txt");
    s.dispatch(Command::InsertText {text:"unsaved ".into(),kind:crate::text::EditKind::Paste});
    preview(&mut s,"bar");s.dispatch(Command::ApplyWorkspaceEdit);settle(&mut s,|s|!s.workspace_busy());
    assert_eq!(source_text(s.state().current_source().unwrap()),"unsaved λ bar\r\nbar\nlast");
    let tab=s.state().active().unwrap().id();
    s.dispatch(Command::Undo {tab});
    assert_eq!(source_text(s.state().current_source().unwrap()),"unsaved λ foo\r\nfoo\nlast");
    assert_eq!(std::fs::read(path).unwrap(),"λ foo\r\nfoo\nlast".as_bytes());
}

#[test]
#[ignore = "explicit release workspace-edit latency and repeated apply/undo probe"]
fn workspace_edit_release_probe() {
    let d=tempfile::tempdir().unwrap();let root=d.path().join("project");std::fs::create_dir(&root).unwrap();
    let original="λ foo\r\n".repeat(100);
    for i in 0..40 {write(&root,&format!("{i:03}.txt"),original.as_bytes());}
    let mut s=session(&d.path().join("private-state"));s.dispatch(Command::OpenRoot(root.clone()));settle(&mut s,scanned);s.enable_text_input_for_test();
    let duration=std::env::var("LENZ_WORKSPACE_PROBE_SECONDS").ok().and_then(|s|s.parse().ok()).unwrap_or(20.0);
    let start=std::time::Instant::now();let mut preparation=Vec::new();let mut application=Vec::new();let mut cycles=0;
    while start.elapsed().as_secs_f64()<duration {
        search(&mut s);let now=std::time::Instant::now();
        s.dispatch(Command::PreviewProjectReplace("bar".into()));settle(&mut s,|s|!s.workspace_busy());
        assert_eq!(s.workspace_preview().unwrap().files().len(),40);preparation.push(now.elapsed().as_secs_f64()*1000.);
        let now=std::time::Instant::now();s.dispatch(Command::ApplyWorkspaceEdit);settle(&mut s,|s|!s.workspace_busy());application.push(now.elapsed().as_secs_f64()*1000.);
        let tabs:Vec<_>=s.state().tabs().iter().map(|t|t.id()).collect();
        assert_eq!(tabs.iter().copied().collect::<std::collections::HashSet<_>>().len(),40);
        assert!(s.state().tabs().iter().all(|t|s.state.source(t.id()).is_some_and(|source|source.source()==t.source())));
        for tab in tabs {
            assert_eq!(source_text(s.state.source(tab).unwrap()),original.replace("foo","bar"));
            s.dispatch(Command::Undo {tab});assert_eq!(source_text(s.state.source(tab).unwrap()),original);
        }
        assert_eq!(std::fs::read(root.join("000.txt")).unwrap(),original.as_bytes());cycles+=1;
    }
    preparation.sort_by(f64::total_cmp);application.sort_by(f64::total_cmp);
    eprintln!("WORKSPACE_PROBE {}",serde_json::json!({"cycles":cycles,"seconds":start.elapsed().as_secs_f64(),"files":40,"edits":4000,"prepare_ms_median":preparation[preparation.len()/2],"apply_ms_median":application[application.len()/2],"correctness":"drafts and undo exact; disk unchanged"}));
}

#[test]
fn workspace_replace_save_preserves_legacy_encoding_and_exact_endings() {
    let (_d,root,mut s)=fixture();let path=root.join("a.txt");std::fs::write(&path,b"caf\xe9 foo\r\n").unwrap();
    open(&mut s,Command::OpenRetained(path.clone()),"a.txt");preview(&mut s,"bar");
    s.dispatch(Command::ApplyWorkspaceEdit);settle(&mut s,|s|!s.workspace_busy());
    assert_eq!(source_text(s.state().current_source().unwrap()),"café bar\r\n");
    let tab=s.state().active().unwrap().id();s.dispatch(Command::Save {tab});
    settle(&mut s,|_|std::fs::read(&path).is_ok_and(|b|b==b"caf\xe9 bar\r\n"));
    s.dispatch(Command::Undo {tab});assert_eq!(source_text(s.state().current_source().unwrap()),"café foo\r\n");
}

#[test]
fn workspace_replace_reveals_an_affected_draft_when_the_original_tab_has_no_matches() {
    let (_d,root,mut s)=fixture();let untouched=write(&root,"untouched.txt",b"keep this file\n");
    open(&mut s,Command::OpenRetained(untouched.clone()),"untouched.txt");
    preview(&mut s,"bar");s.dispatch(Command::ApplyWorkspaceEdit);settle(&mut s,|s|!s.workspace_busy());
    assert_eq!(s.state().active().unwrap().source(),root.join("a.txt"));
    assert_eq!(source_text(s.state().current_source().unwrap()),"λ bar\r\nbar\nlast");
    assert_eq!(std::fs::read(untouched).unwrap(),b"keep this file\n");
}

#[test]
fn workspace_replace_refuses_excessive_expansion_without_changing_drafts_or_files() {
    let (_d,root,mut s)=fixture();let path=root.join("a.txt");let original="foo ".repeat(9000);
    std::fs::write(&path,original.as_bytes()).unwrap();
    search(&mut s);s.dispatch(Command::PreviewProjectReplace("x".repeat(64 << 10)));settle(&mut s,|s|!s.workspace_busy());
    assert!(s.workspace_preview().is_none());assert!(s.state().tabs().is_empty());
    assert!(s.state().notices().last().is_some_and(|n|n.contains("32 MiB")));
    assert_eq!(std::fs::read(path).unwrap(),original.as_bytes());
}
