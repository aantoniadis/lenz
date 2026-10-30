use super::*;

#[test]
fn syntax_problems_survive_switching_and_list_all_known_files_without_a_server() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(dir.path(), "a.java", b"class A { void f( { }\n");
    let b = write(dir.path(), "b.rs", b"fn broken( {\n");
    let mut s = session(dir.path());
    open(&mut s, Command::OpenRetained(a.clone()), "a.java");
    let first = s.state().active().unwrap().id();
    settle(&mut s, |s| s.state().diagnostics(first).is_some());
    assert!(!s.state().diagnostics(first).unwrap().items.is_empty());
    assert!(s.state().problem_badge(first).starts_with('!'));
    open(&mut s, Command::OpenRetained(b.clone()), "b.rs");
    let second = s.state().active().unwrap().id();
    settle(&mut s, |s| s.state().diagnostics(second).is_some());
    assert!(s.state().path_problem_count(&a, false) > 0);
    s.dispatch(Command::ShowProblems);
    let results = s.state().results().unwrap();
    assert_eq!(results.results_kind(), ResultsKind::Problems);
    assert!(results.hits().iter().any(|h| h.path.ends_with("a.java")));
    assert!(results.hits().iter().any(|h| h.path.ends_with("b.rs")));
    let index = results.hits().iter().position(|h| h.path.ends_with("a.java")).unwrap();
    s.dispatch(Command::OpenResult(index));
    settle(&mut s, |s| s.state().active().is_some_and(|t| t.source() == a));
    assert_eq!(std::fs::read(a).unwrap(), b"class A { void f( { }\n");
}

#[test]
fn syntax_errors_recheck_edits_and_never_underline_stale_offsets() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "a.java", b"class A { void f( { }\n");
    let mut s = session(dir.path());
    open(&mut s, Command::OpenRetained(path.clone()), "a.java");
    let tab = s.state().active().unwrap().id();
    settle(&mut s, |s| s.state().diagnostics(tab).is_some());
    let snapshot = s.state().current_source().unwrap();
    let (expected, len) = (snapshot.key(), snapshot.len_bytes());
    s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..len, text: "class A {}\n".into() }], kind: crate::text::EditKind::Replace });
    assert!(s.state().diagnostics(tab).is_none(), "stale parser offsets cannot be drawn");
    assert!(s.state().problem_badge(tab).contains('?'), "pending check retains visible previous error state");
    settle(&mut s, |s| s.state().diagnostics(tab).is_some());
    assert!(s.state().diagnostics(tab).unwrap().items.is_empty());
    assert_eq!(s.state().problem_badge(tab), "");
    assert_eq!(s.state().path_problem_count(&path, false), 0);
    s.dispatch(Command::ShowProblems);
    assert!(s.state().results().unwrap().hits().is_empty());
    assert_eq!(std::fs::read(path).unwrap(), b"class A { void f( { }\n");
}

#[test]
fn problems_navigation_refuses_a_captured_draft_that_changed() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "a.java", b"class A { void f( { }\n");
    let mut s = session(dir.path());
    open(&mut s, Command::OpenRetained(path.clone()), "a.java");
    let tab = s.state().active().unwrap().id();
    settle(&mut s, |s| s.state().diagnostics(tab).is_some());
    s.dispatch(Command::ShowProblems);
    assert!(!s.state().results().unwrap().hits().is_empty());
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..0, text: "// newer\n".into() }], kind: crate::text::EditKind::Replace });
    s.dispatch(Command::OpenResult(0));
    assert!(s.state().messages.iter().any(|m| m.contains("captured draft changed")), "{:?}", s.state().messages);
    assert_eq!(std::fs::read(path).unwrap(), b"class A { void f( { }\n");
}

#[test]
fn java_bracket_expression_in_enum_main_is_reported_when_opened_without_a_server() {
    let text = "interface PgWireAuthenticator {\n    enum Trust implements PgWireAuthenticator {\n        INSTANCE;\n        public static void main(String[] args) {\n            System.out.println(\"Hello Lenz!\");\n            [1,2,3];\n        }\n    }\n}\n";
    let bad = text.find("[1,2,3]").unwrap();
    let syntax = highlight::parse_with(Language::Java, text.into(), highlight::Want::default(), || true).unwrap();
    let errors = syntax.errors.unwrap();
    eprintln!("JAVA_REPRO parser findings: {errors:?}");
    assert!(errors.iter().any(|error| error.range.start <= bad && error.range.end >= bad), "the parser must mark the invalid expression");
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "PgWireAuthenticator.java", text.as_bytes());
    let mut s = session(dir.path()); open(&mut s, Command::OpenRetained(path.clone()), "PgWireAuthenticator.java");
    let tab = s.state.active.unwrap(); settle(&mut s, |s| s.state.diagnostics(tab).is_some());
    let found = s.state.diagnostics(tab).unwrap();
    eprintln!("JAVA_REPRO session findings: {:?}", found.items.iter().map(|d| (&d.range, &d.message)).collect::<Vec<_>>());
    assert!(found.items.iter().any(|error| error.range.start <= bad && error.range.end >= bad));
    assert_eq!(std::fs::read(path).unwrap(), text.as_bytes());
}

fn inline_findings(s: &mut Session, items: Vec<(std::ops::Range<usize>, crate::semantics::diagnostics::Severity, &str)>) {
    use crate::semantics::diagnostics::{Diagnostic, FileDiagnostics};
    let snapshot = s.state.current_source().unwrap();
    let doc = Document::from_snapshot_parts(snapshot.source_meta().clone(), snapshot.key().version, source_text(snapshot).into());
    let problems = FileDiagnostics::new(&doc, items.into_iter().map(|(range,severity,message)| Diagnostic { range, severity, message:message.into(), source:None, code:None, raw:serde_json::Value::Null }).collect());
    s.dispatch(Completion::Highlighted { version: snapshot.key().version, parsed:None, folds:None, declarations:None, problems:Some(Arc::new(problems)) });
}

#[test]
fn inline_problem_groups_by_start_line_worst_severity_and_bounds_utf8_message() {
    use crate::semantics::diagnostics::Severity as S;
    let dir=tempfile::tempdir().unwrap(); let source="😀 alpha\r\nsecond\nthird";
    let path=write(dir.path(),"inline.txt",source.as_bytes()); let mut s=session(dir.path());open(&mut s,Command::OpenRetained(path),"inline.txt");let tab=s.state.active.unwrap();
    let long="λ".repeat(170);
    inline_findings(&mut s,vec![(5..8,S::Warning,"warning"),(1..2,S::Error,&long),(12..20,S::Information,"one line\nsecond line"),(19..19,S::Hint,"hint")]);
    let first=s.state.inline_problem(tab,0).unwrap();assert_eq!(first.severity,S::Error);assert_eq!(first.other_count,1);assert_eq!(first.byte,0,"an invalid interior-byte range cannot split a UTF8 caret");assert_eq!(first.message,"λ".repeat(160)+"…");assert!(!first.empty);
    let second=s.state.inline_problem(tab,1).unwrap();assert_eq!(second.message,"one line");assert_eq!(second.other_count,0);
    let third=s.state.inline_problem(tab,2).unwrap();assert_eq!(third.message,"hint");assert!(third.empty);assert_eq!(third.other_count,0,"multiline findings are not repeated on following lines");
    assert!(s.state.inline_problem(tab,usize::MAX).is_none());
}

#[test]
fn inline_problem_keeps_empty_and_eof_anchors_on_the_correct_logical_line() {
    use crate::semantics::diagnostics::Severity as S;
    for (source,line,byte) in [("",0,0),("λ",0,2),("λ\r\n",1,0)] {
        let dir=tempfile::tempdir().unwrap();let path=write(dir.path(),"eof.txt",source.as_bytes());let mut s=session(dir.path());open(&mut s,Command::OpenRetained(path),"eof.txt");let tab=s.state.active.unwrap();
        inline_findings(&mut s,vec![(source.len()..source.len(),S::Error,"Expected }" )]);
        let found=s.state.inline_problem(tab,line).unwrap();assert_eq!(found.byte,byte);assert!(found.empty);assert_eq!(found.message,"Expected }");if line>0{assert!(s.state.inline_problem(tab,line-1).is_none());}
    }
}

#[test]
fn inline_problem_rejects_old_draft_and_refresh_findings_until_current_result_arrives() {
    use crate::semantics::diagnostics::Severity as S;
    let dir=tempfile::tempdir().unwrap();let path=write(dir.path(),"fresh.txt",b"alpha\n");let mut s=session(dir.path());open(&mut s,Command::OpenRetained(path.clone()),"fresh.txt");let tab=s.state.active.unwrap();
    inline_findings(&mut s,vec![(0..1,S::Error,"old")]);assert!(s.state.inline_problem(tab,0).is_some());
    let expected=s.state.current_source().unwrap().key();s.dispatch(Command::Edit {tab,expected,edits:vec![crate::text::TextEdit{range:0..0,text:"new ".into()}],kind:crate::text::EditKind::Replace});assert!(s.state.inline_problem(tab,0).is_none());
    inline_findings(&mut s,vec![(4..5,S::Warning,"current draft")]);assert_eq!(s.state.inline_problem(tab,0).unwrap().message,"current draft");
    let version=s.versions.fetch_add(1,Ordering::Relaxed)+1;let replacement=Arc::new(Document::decode(path.clone(),version,b"disk\n"));let token=s.state.active().unwrap().refresh;
    // A refreshed clean source replaces this test's draft through the same admission boundary.
    s.state.sources.buffers.get_mut(&expected.buffer).unwrap().draft=None;
    std::fs::write(&path,b"disk\n").unwrap();s.dispatch(Completion::Refreshed{tab,token,result:Refresh::Changed(Stamp::of(&path).unwrap(),replacement)});assert!(s.state.inline_problem(tab,0).is_none());
    inline_findings(&mut s,vec![(4..4,S::Error,"current disk")]);assert_eq!(s.state.inline_problem(tab,0).unwrap().message,"current disk");
    inline_findings(&mut s,vec![]);assert!(s.state.inline_problem(tab,0).is_none());
}
