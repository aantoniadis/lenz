use super::*;
use crate::sql_dialect::Dialect;

fn fixture(text: &str) -> (tempfile::TempDir, Session, PathBuf, TabId) {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "query.sql", text.as_bytes());
    let mut s = session(dir.path());
    open(&mut s, Command::OpenRetained(path.clone()), "query.sql");
    s.enable_text_input_for_test();
    s.dispatch(Command::SetSqlDialect(Some(Dialect::PostgreSql)));
    let tab = s.state().active().unwrap().id();
    (dir, s, path, tab)
}

#[test]
fn postgres_offline_completion_and_parameter_help_use_the_current_draft_without_a_server() {
    let original = "-- 𝄞\r\nWITH source AS (SELECT 1 AS id, 2 AS \"User Name\") SELECT s. FROM source s";
    let (_dir, mut s, path, tab) = fixture(original);
    let at = original.find("s. FROM").unwrap() + 2;
    s.dispatch(Command::Select(Selection { anchor: at, head: at }));
    assert_eq!(s.disabled(CommandId::Completion), None);
    s.dispatch(Command::Complete);
    settle(&mut s, |s| s.state().completion().is_some());
    let labels = s.state().completion().unwrap().rows().map(|r| r.label.clone()).collect::<Vec<_>>();
    assert_eq!(labels, ["\"User Name\"", "id"]);
    s.dispatch(Command::AcceptCompletion);
    assert_eq!(source_text(s.state().current_source().unwrap()), original.replace("s. FROM", "s.\"User Name\" FROM"));
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), original);
    let expected = s.state().current_source().unwrap().key();
    let call = "SELECT jsonb_build_object('x', ARRAY[1, 2], 'y', ";
    s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..original.len(), text: call.into() }], kind: crate::text::EditKind::Replace });
    s.dispatch(Command::Select(Selection { anchor: call.len(), head: call.len() }));
    assert_eq!(s.disabled(CommandId::ParameterInfo), None);
    s.dispatch(Command::ParameterInfo);
    settle(&mut s, |s| s.state().signature_help().is_some());
    assert!(s.state().signature_help().unwrap().active_signature().unwrap().label.contains("jsonb_build_object"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), original);
}

#[test]
fn postgres_native_diagnostics_and_formatting_are_exact_and_dialect_switches_reject_old_results() {
    let original = "SELECT '𝄞',\r\n  )";
    let (_dir, mut s, path, tab) = fixture(original);
    settle(&mut s, |s| s.state().diagnostics(tab).is_some());
    let problems = s.state().diagnostics(tab).unwrap();
    assert_eq!(problems.items[0].range, original.find(')').unwrap()..original.len());
    let expected = s.state().current_source().unwrap().key();
    let query = "select jsonb_build_object('x', $$𝄞; -- literal$$), ARRAY[1,2]::int[]";
    s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..original.len(), text: query.into() }], kind: crate::text::EditKind::Replace });
    assert_eq!(s.disabled(CommandId::FormatDocument), None);
    s.dispatch(Command::Format(super::super::format_work::FormatKind::Format));
    settle(&mut s, |s| s.state().asking().is_none() && s.state().notices().iter().any(|n| n.starts_with("Format Document:")));
    let formatted = source_text(s.state().current_source().unwrap());
    assert_ne!(formatted, query);
    assert!(formatted.contains("$$𝄞; -- literal$$"));
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), query);
    s.dispatch(Command::Format(super::super::format_work::FormatKind::Format));
    s.dispatch(Command::SetSqlDialect(None));
    assert!(s.state().diagnostics(tab).is_none());
    for _ in 0..30 { if let Ok(reply) = s.completions().try_recv() { s.dispatch(reply); } }
    assert_eq!(source_text(s.state().current_source().unwrap()), query);
    assert_eq!(s.disabled(CommandId::Completion), Some("Choose an SQL dialect first (SQL Dialect…)"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), original);
}

#[test]
fn sql_replies_captured_before_dialect_change_edit_or_escape_never_reappear() {
    let source = "SELECT q. FROM (SELECT 1 AS id) q";
    let (_dir, mut s, _path, tab) = fixture(source);
    let at = source.find("q. FROM").unwrap() + 2;
    s.dispatch(Command::Select(Selection { anchor: at, head: at }));
    s.dispatch(Command::Complete);
    let receiver = s.completions();
    let saved = loop {
        let next = receiver.recv_blocking_timeout().unwrap();
        if matches!(&next, Completion::Analyzed { result, .. } if matches!(&**result, Completion::Sql(reply) if matches!(reply.operation, super::super::sql_work::Operation::Complete(_)))) { break next; }
        s.dispatch(next);
    };
    s.dispatch(Command::SetSqlDialect(Some(Dialect::DuckDb)));
    s.dispatch(saved);
    assert!(s.state().completion().is_none());
    s.dispatch(Command::SetSqlDialect(Some(Dialect::PostgreSql)));
    s.dispatch(Command::Complete);
    let saved = loop {
        let next = receiver.recv_blocking_timeout().unwrap();
        if matches!(&next, Completion::Analyzed { result, .. } if matches!(&**result, Completion::Sql(reply) if matches!(reply.operation, super::super::sql_work::Operation::Complete(_)))) { break next; }
        s.dispatch(next);
    };
    s.dispatch(Command::CloseCompletion);
    s.dispatch(saved);
    assert!(s.state().completion().is_none());
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..source.len(), text: "select 1+2".into() }], kind: crate::text::EditKind::Replace });
    s.dispatch(Command::Format(super::super::format_work::FormatKind::Format));
    let saved = loop {
        let next = receiver.recv_blocking_timeout().unwrap();
        if matches!(&next, Completion::Analyzed { result, .. } if matches!(&**result, Completion::Sql(reply) if matches!(reply.operation, super::super::sql_work::Operation::Format(_)))) { break next; }
        s.dispatch(next);
    };
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..0, text: "-- newer\n".into() }], kind: crate::text::EditKind::Replace });
    s.dispatch(saved);
    assert_eq!(source_text(s.state().current_source().unwrap()), "-- newer\nselect 1+2");
}

#[test]
fn duckdb_query_local_completion_remains_available_without_the_optional_native_tool() {
    let source = "WITH c AS (SELECT 1 AS \"Mixed Column\") SELECT c. FROM c";
    let (_dir, mut s, path, tab) = fixture(source);
    s.dispatch(Command::SetDuckDbExecutable(Some(PathBuf::from("/nonexistent/duckdb"))));
    s.dispatch(Command::SetSqlDialect(Some(Dialect::DuckDb)));
    let at = source.find("c. FROM").unwrap() + 2;
    s.dispatch(Command::Select(Selection { anchor: at, head: at }));
    assert_eq!(s.disabled(CommandId::Completion), None);
    assert!(s.disabled(CommandId::ParameterInfo).is_some());
    s.dispatch(Command::Complete);
    settle(&mut s, |s| s.state().completion().is_some());
    s.dispatch(Command::AcceptCompletion);
    assert_eq!(source_text(s.state().current_source().unwrap()), source.replace("c. FROM", "c.\"Mixed Column\" FROM"));
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), source);
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}

#[test]
#[ignore = "requires explicit LENZ_TEST_DUCKDB, no automatic installation"]
fn native_duckdb_editor_diagnostics_signatures_and_formatting_use_the_selected_tool() {
    let source = "SELECT '𝄞',\r\n  )";
    let (_dir, mut s, path, tab) = fixture(source);
    let binary = PathBuf::from(std::env::var_os("LENZ_TEST_DUCKDB").expect("select the native fixture executable"));
    s.dispatch(Command::SetDuckDbExecutable(Some(binary)));
    s.dispatch(Command::SetSqlDialect(Some(Dialect::DuckDb)));
    settle(&mut s, |s| s.state().diagnostics(tab).is_some());
    assert_eq!(s.state().diagnostics(tab).unwrap().items[0].range, source.find(')').unwrap()..source.len());
    let call = "SELECT list_transform([1, 2], lambda x:";
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..source.len(), text: call.into() }], kind: crate::text::EditKind::Replace });
    s.dispatch(Command::Select(Selection { anchor: call.len(), head: call.len() }));
    s.dispatch(Command::ParameterInfo);
    settle(&mut s, |s| s.state().signature_help().is_some());
    let help = s.state().signature_help().unwrap().active_signature().unwrap();
    assert!(help.label.starts_with("list_transform(list:")); assert_eq!(help.active_parameter, Some(1));
    let query = "from (select 1 as id, 2 as hidden) q select * exclude(hidden) group by all;";
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..call.len(), text: query.into() }], kind: crate::text::EditKind::Replace });
    s.dispatch(Command::Format(super::super::format_work::FormatKind::Format));
    settle(&mut s, |s| s.state().notices().iter().any(|n| n.starts_with("Format Document:")));
    assert_ne!(source_text(s.state().current_source().unwrap()), query);
    s.dispatch(Command::Undo { tab }); assert_eq!(source_text(s.state().current_source().unwrap()), query);
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}

#[test]
fn metadata_requires_explicit_connect_and_rejects_expiry_disconnect_and_old_replies() {
    let (_dir, mut s, path, _tab) = fixture("SELECT t. FROM public.lenz_metadata_fixture t");
    assert!(s.sql_metadata_details().contains("disconnected"));
    s.dispatch(Command::ConfigureSqlMetadata("postgres://user:secret@host/db".into()));
    assert_eq!(s.sql_metadata_source(), "");
    s.dispatch(Command::ConfigureSqlMetadata("lenz-test".into()));
    assert!(s.sql_metadata_details().contains("configured; disconnected"));
    assert!(s.state().activity_log().back().unwrap().contains("source configured"));
    assert_eq!(s.sql_work.metadata.pending, None);
    let catalog = || crate::sql_metadata::Catalog { relations: vec![crate::sql_metadata::Relation { name:"public.lenz_metadata_fixture".into(), columns:vec![crate::sql_metadata::Column{name:"id".into(),data_type:"integer".into()}] }],functions:vec![],file:None };
    let seq = s.sql_work.metadata.seq.fetch_add(1, Ordering::AcqRel) + 1;
    s.sql_work.metadata.pending = Some(seq);
    s.sql_metadata_reply(seq, Ok(catalog()));
    assert!(s.sql_catalog().is_some());
    let identity = s.sql_identity().unwrap();
    s.sql_work.metadata.catalog.as_mut().unwrap().0 = std::time::Instant::now() - std::time::Duration::from_secs(301);
    assert!(s.sql_catalog().is_none());
    s.sync_sql();
    assert!(!s.sql_identity_current(identity));
    assert!(s.sql_metadata_details().contains("expired"));
    s.dispatch(Command::DisconnectSqlMetadata);
    s.sql_metadata_reply(seq, Ok(catalog()));
    assert!(s.sql_catalog().is_none());
    s.dispatch(Command::SetSqlDialect(Some(Dialect::DuckDb)));
    s.sync_sql();
    assert_eq!(s.sql_metadata_source(), "");
    assert!(s.state().activity_log().iter().any(|line| line.contains("configuration cleared after changing")));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "SELECT t. FROM public.lenz_metadata_fixture t");
}

#[test]
#[ignore = "requires LENZ_TEST_PG_SERVICE and a disposable PostgreSQL metadata fixture"]
fn native_postgres_metadata_is_explicit_read_only_and_assists_the_current_draft() {
    let service = std::env::var("LENZ_TEST_PG_SERVICE").unwrap();
    let text = "DROP TABLE public.lenz_metadata_fixture; UPDATE public.lenz_metadata_fixture SET id=0; COPY public.lenz_metadata_fixture TO '/tmp/lenz-never-export'; SELECT ''; -- '\\n\\! touch /tmp/lenz-never-sentinel\nSELECT t. FROM public.lenz_metadata_fixture t";
    let (_dir, mut s, path, tab) = fixture(text);
    s.dispatch(Command::ConfigureSqlMetadata(service));
    assert!(s.sql_catalog().is_none());
    s.dispatch(Command::ConnectSqlMetadata);
    settle(&mut s, |s| s.sql_work.metadata.pending.is_none());
    assert!(s.sql_catalog().is_some(), "{}", s.sql_metadata_details());
    let catalog = s.sql_catalog().unwrap();
    assert_eq!(catalog.documentation("id"), Some("public.lenz_metadata_fixture.id: integer".into()));
    assert!(catalog.functions.iter().any(|f| f.name == "public.lenz_metadata_function" && f.result == "integer"));
    let at = text.rfind("t. FROM").unwrap()+2;
    s.dispatch(Command::Select(Selection{anchor:at,head:at})); s.dispatch(Command::Complete);
    settle(&mut s, |s| s.state().completion().is_some());
    assert_eq!(s.state().completion().unwrap().rows().map(|r| r.label.as_str()).collect::<Vec<_>>(), ["id","note"]);
    s.dispatch(Command::AcceptCompletion);
    s.dispatch(Command::Undo{tab});
    assert_eq!(source_text(s.state().current_source().unwrap()),text);
    let call = "SELECT public.lenz_metadata_function(";
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit{tab,expected,edits:vec![crate::text::TextEdit{range:0..text.len(),text:call.into()}],kind:crate::text::EditKind::Replace});
    s.dispatch(Command::Select(Selection{anchor:call.len(),head:call.len()})); s.dispatch(Command::ParameterInfo);
    settle(&mut s,|s| s.state().signature_help().is_some());
    assert!(s.state().signature_help().unwrap().active_signature().unwrap().label.contains("value integer"));
    s.dispatch(Command::RefreshSqlMetadata); settle(&mut s,|s| s.sql_work.metadata.pending.is_none()); assert!(s.sql_catalog().is_some());
    s.dispatch(Command::DisconnectSqlMetadata); assert!(s.sql_catalog().is_none());
    s.dispatch(Command::ConfigureSqlMetadata("lenz-service-that-does-not-exist".into()));
    s.dispatch(Command::ConnectSqlMetadata); settle(&mut s,|s| s.sql_work.metadata.pending.is_none());
    assert!(s.sql_metadata_details().contains("failed"));
    assert_eq!(std::fs::read_to_string(path).unwrap(),text);
}

#[test]
#[ignore = "requires LENZ_TEST_DUCKDB and LENZ_TEST_DUCKDB_FILE disposable fixture"]
fn native_duckdb_metadata_reads_only_the_explicit_file_and_rejects_obsolete_snapshots() {
    let file = std::env::var("LENZ_TEST_DUCKDB_FILE").unwrap();
    let executable = PathBuf::from(std::env::var_os("LENZ_TEST_DUCKDB").unwrap());
    let text = "DROP TABLE main.lenz_metadata_fixture; UPDATE main.lenz_metadata_fixture SET id=0; COPY main.lenz_metadata_fixture TO '/tmp/lenz-never-export'; SELECT 'first\n.shell touch /tmp/lenz-never-sentinel\n.read /tmp/lenz-never-private\nlast'; SELECT t. FROM main.lenz_metadata_fixture t";
    let (_dir, mut s,path,tab) = fixture(text);
    s.dispatch(Command::SetDuckDbExecutable(Some(executable)));
    s.dispatch(Command::SetSqlDialect(Some(Dialect::DuckDb)));
    s.dispatch(Command::ConfigureSqlMetadata(file.clone()));
    assert!(s.sql_metadata_details().contains("configured; disconnected"));
    assert!(s.sql_catalog().is_none());
    let before = std::fs::read(&file).unwrap();
    s.dispatch(Command::ConnectSqlMetadata);
    settle(&mut s,|s|s.sql_work.metadata.pending.is_none());
    let catalog=s.sql_catalog().unwrap_or_else(||panic!("{}",s.sql_metadata_details()));
    assert!(catalog.documentation("id").unwrap().contains("INTEGER"));
    assert!(catalog.functions.iter().any(|f|f.name == "\"main\".\"lenz_metadata_function\""));
    let at=text.rfind("t. FROM").unwrap()+2;
    s.dispatch(Command::Select(Selection{anchor:at,head:at}));s.dispatch(Command::Complete);
    settle(&mut s,|s|s.state().completion().is_some());
    assert_eq!(s.state().completion().unwrap().rows().map(|r|r.label.as_str()).collect::<Vec<_>>(),["id","note"]);
    s.dispatch(Command::AcceptCompletion); s.dispatch(Command::Undo{tab});
    assert_eq!(source_text(s.state().current_source().unwrap()),text);
    let completion_text="SELECT main.lenz_metadata_";
    let expected=s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit{tab,expected,edits:vec![crate::text::TextEdit{range:0..text.len(),text:completion_text.into()}],kind:crate::text::EditKind::Replace});
    s.dispatch(Command::Select(Selection{anchor:completion_text.len(),head:completion_text.len()}));s.dispatch(Command::Complete);
    settle(&mut s,|s|s.state().completion().is_some());
    assert_eq!(s.state().completion().unwrap().rows().map(|r|r.label.as_str()).collect::<Vec<_>>(),["\"lenz_metadata_function\""]);
    s.dispatch(Command::CloseCompletion);
    let call="SELECT main.lenz_metadata_function(";
    let expected=s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit{tab,expected,edits:vec![crate::text::TextEdit{range:0..completion_text.len(),text:call.into()}],kind:crate::text::EditKind::Replace});
    s.dispatch(Command::Select(Selection{anchor:call.len(),head:call.len()}));s.dispatch(Command::ParameterInfo);
    settle(&mut s,|s|s.state().signature_help().is_some());
    assert!(s.state().signature_help().unwrap().active_signature().unwrap().label.contains("not evaluated"));
    s.dispatch(Command::RefreshSqlMetadata);settle(&mut s,|s|s.sql_work.metadata.pending.is_none());assert!(s.sql_catalog().is_some());
    assert_eq!(std::fs::read(&file).unwrap(),before);
    s.dispatch(Command::DisconnectSqlMetadata);assert!(s.sql_catalog().is_none());
    s.dispatch(Command::ConfigureSqlMetadata(format!("{file}.missing")));s.dispatch(Command::ConnectSqlMetadata);
    settle(&mut s,|s|s.sql_work.metadata.pending.is_none());assert!(s.sql_metadata_details().contains("does not exist"));
    assert!(!Path::new(&format!("{file}.missing")).exists());
    assert_eq!(std::fs::read_to_string(path).unwrap(),text);
}

#[test]
fn duckdb_catalog_completes_default_schema_table_names_and_columns() {
    for (dialect, source, expected) in [
        (Dialect::DuckDb, "select * from us|", vec!["\"user.data\"", "\"users\""]),
        (Dialect::DuckDb, "select u.| from users u", vec!["id"]),
        (Dialect::DuckDb, "select u.| from main.users u", vec!["id"]),
        (Dialect::DuckDb, "select users.| from users", vec!["id"]),
        (Dialect::DuckDb, "select u.| from USERS u", vec!["id"]),
        (Dialect::DuckDb, "select users.| from main.users u", vec![]),
        (Dialect::DuckDb, "with c as (select * from users) select c.| from c", vec!["id"]),
        (Dialect::DuckDb, "with c as (select users.* from main.users u) select c.| from c", vec![]),
        (Dialect::DuckDb, "with c as (select users.*) select c.| from c", vec![]),
        (Dialect::DuckDb, "with c as (select * from range(3)) select c.| from c", vec!["range"]),
        (Dialect::DuckDb, "with users as (select 1 as local_id) select u.| from main.users u", vec!["id"]),
        (Dialect::DuckDb, "select u.| from other.users u", vec!["other_id"]),
        (Dialect::DuckDb, "select u.| from other_only u", vec![]),
        (Dialect::DuckDb, "with users as (select 1 as local_id) select u.| from users u", vec!["local_id"]),
        (Dialect::DuckDb, "select u.| from \"user.data\" u", vec!["id"]),
        (Dialect::DuckDb, "select u.| from \"odd\"\"name\" u", vec!["id"]),
        (Dialect::PostgreSql, "select u.| from users u", vec![]),
    ] {
        let caret = source.find('|').unwrap();
        let source = source.replace('|', "");
        let (_dir, mut s, _path, _) = fixture(&source);
        s.dispatch(Command::SetDuckDbExecutable(Some(PathBuf::from("/nonexistent/duckdb"))));
        s.dispatch(Command::SetSqlDialect(Some(dialect)));
        let request = s.sql_work.metadata.seq.fetch_add(1, Ordering::AcqRel) + 1;
        s.sql_work.metadata.pending = Some(request);
        s.sql_metadata_reply(request, Ok(crate::sql_metadata::Catalog {
            relations: [
                ("\"main\".\"range\"", "id"), ("\"main\".\"users\"", "id"), ("\"other\".\"users\"", "other_id"),
                ("\"other\".\"other_only\"", "other_id"), ("\"main\".\"user.data\"", "id"),
                ("\"main\".\"odd\"\"name\"", "id"),
            ].into_iter().map(|(name, column)| crate::sql_metadata::Relation {
                name: name.into(), columns: vec![crate::sql_metadata::Column { name: column.into(), data_type: "integer".into() }],
            }).collect(), ..Default::default()
        }));
        s.dispatch(Command::Select(Selection { anchor: caret, head: caret }));
        s.dispatch(Command::Complete);
        settle(&mut s, |s| s.state().completion().is_some() || s.state().notices().iter().any(|n| n.starts_with("Code completion:")));
        let labels = s.state().completion().map(|c| c.rows().map(|r| r.label.as_str()).collect::<Vec<_>>()).unwrap_or_default();
        assert_eq!(labels, expected, "{source}");
    }
}

#[test]
fn completion_after_a_finished_sql_statement_reports_no_names_without_claiming_disconnection() {
    let source = "select *\nfrom users;\n\n";
    for dialect in [Dialect::PostgreSql, Dialect::DuckDb] {
        for connected in [true, false] {
            let (_dir, mut s, path, _) = fixture(source);
            s.dispatch(Command::SetDuckDbExecutable(Some(PathBuf::from("/nonexistent/duckdb"))));
            s.dispatch(Command::SetSqlDialect(Some(dialect)));
            if connected {
                let request = s.sql_work.metadata.seq.fetch_add(1, Ordering::AcqRel) + 1;
                s.sql_work.metadata.pending = Some(request);
                s.sql_metadata_reply(request, Ok(crate::sql_metadata::Catalog {
                    relations: vec![crate::sql_metadata::Relation { name: "main.users".into(), columns: vec![crate::sql_metadata::Column { name: "id".into(), data_type: "integer".into() }] }],
                    ..Default::default()
                }));
            }
            s.dispatch(Command::Select(Selection { anchor: source.len(), head: source.len() }));
            s.dispatch(Command::Complete);
            settle(&mut s, |s| s.state().notices().iter().any(|n| n.starts_with("Code completion:")));
            assert_eq!(s.state().notices().iter().find(|n| n.starts_with("Code completion:")).unwrap(), "Code completion: no names match at this position");
            assert_eq!(s.sql_catalog().is_some(), connected);
            assert!(s.state().completion().is_none());
            assert_eq!(source_text(s.state().current_source().unwrap()), source);
            assert_eq!(std::fs::read_to_string(path).unwrap(), source);
        }
    }
}

#[test]
fn metadata_failures_stay_in_activity_after_disconnect_and_status_clear() {
    let (_dir, mut s, _path, _tab) = fixture("SELECT 1");
    s.sync_sql();
    let request = s.sql_work.metadata.seq.fetch_add(1, std::sync::atomic::Ordering::AcqRel) + 1;
    s.sql_work.metadata.pending = Some(request);
    s.sql_metadata_reply(request, Err("SQLite extension unavailable: installation required".into()));
    assert!(!s.state().activity_log_shown());
    assert!(s.state().activity_log().back().unwrap().contains("SQLite extension unavailable"));
    s.dispatch(Command::DisconnectSqlMetadata);
    s.state.messages.clear();
    assert!(s.state().activity_log().back().unwrap().contains("SQLite extension unavailable"));
    s.dispatch(Command::ToggleActivityLog);
    assert!(s.state().activity_log_shown());
    s.dispatch(Command::ClearActivityLog);
    assert!(s.state().activity_log().is_empty());
}

#[test]
#[ignore = "requires LENZ_TEST_DUCKDB and a checkpointed LENZ_TEST_SQLITE_BACKUP"]
fn native_sqlite_backup_connects_and_completes_unqualified_tables_in_the_session() {
    let source = PathBuf::from(std::env::var_os("LENZ_TEST_SQLITE_BACKUP").unwrap());
    let original = std::fs::read(&source).unwrap();
    let (dir, mut s, path, tab) = fixture("SELECT 1;");
    let database = dir.path().join("backup.db");
    std::fs::write(&database, &original).unwrap();
    s.dispatch(Command::SetDuckDbExecutable(Some(PathBuf::from(std::env::var_os("LENZ_TEST_DUCKDB").unwrap()))));
    s.dispatch(Command::SetSqlDialect(Some(Dialect::DuckDb)));
    s.dispatch(Command::ConfigureSqlMetadata(database.to_str().unwrap().into()));
    s.dispatch(Command::ConnectSqlMetadata);
    settle(&mut s, |s| s.sql_work.metadata.pending.is_none());
    let catalog = s.sql_catalog().unwrap_or_else(|| panic!("{}", s.sql_metadata_details()));
    let relation = catalog.relations.iter().find(|r| r.name.starts_with("\"main\".") && !r.columns.is_empty()).unwrap();
    let short = relation.name.strip_prefix("\"main\".").unwrap();
    let query = format!("SELECT t. FROM {short} t");
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..9, text: query.into() }], kind: crate::text::EditKind::Replace });
    s.dispatch(Command::Select(Selection { anchor: 9, head: 9 }));
    s.dispatch(Command::Complete);
    settle(&mut s, |s| s.state().completion().is_some() || s.state().notices().iter().any(|n| n.starts_with("Code completion:")));
    let labels = s.state().completion().map(|c| c.rows().map(|r| r.label.as_str()).collect::<Vec<_>>()).unwrap_or_default();
    assert!(labels.contains(&relation.columns[0].name.as_str()), "{labels:?}");
    assert!(s.state().activity_log().iter().any(|line| line.contains("SQL metadata: connected;")));
    assert_eq!(std::fs::read(&source).unwrap(), original);
    assert_eq!(std::fs::read(&database).unwrap(), original);
    assert_eq!(std::fs::read_to_string(path).unwrap(), "SELECT 1;");
    eprintln!("Session connected: {} relations; unqualified table completion passed; database and SQL bytes unchanged", catalog.relations.len());
}
