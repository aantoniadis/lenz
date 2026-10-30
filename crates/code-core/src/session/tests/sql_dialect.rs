use super::*;
use crate::actions::{CatalogAction, InterfaceAction};

const FILE: [CommandId; 3] = [CommandId::SqlDialectPostgreSql, CommandId::SqlDialectDuckDb, CommandId::SqlDialectFollowProject];
const PROJECT: [CommandId; 3] = [CommandId::ProjectSqlDialectPostgreSql, CommandId::ProjectSqlDialectDuckDb, CommandId::ProjectSqlDialectUnset];

fn run(s: &mut Session, id: CommandId) {
    let CatalogAction::Command(command) = s.action(id) else { panic!("{id:?} is not a command: {:?}", s.action(id)) };
    s.dispatch(command);
}

fn toggles(s: &Session, ids: [CommandId; 3]) -> [Option<bool>; 3] {
    ids.map(|id| s.actions().iter().find(|a| a.id == id).and_then(|a| a.on))
}

fn settings(dir: &Path) -> Option<serde_json::Value> {
    std::fs::read(dir.join("state/lenz-code/settings.json")).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

#[test]
fn an_unset_sql_dialect_stays_visibly_unset_until_the_project_default_is_chosen_and_remembered() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    std::fs::create_dir(&root).unwrap();
    let sql = write(&root, "q.sql", b"select 1;\n");

    let mut s = session(d.path());
    assert_eq!(s.disabled(CommandId::SqlDialect), Some("No file is open"));
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, scanned);
    open(&mut s, Command::Open(sql.clone()), "q.sql");
    // Lenz never assumes PostgreSQL: the file has no dialect until someone chooses one.
    assert_eq!(s.sql_dialect_label().as_deref(), Some("SQL: not set · metadata off"));
    let details = s.sql_dialect_details();
    assert!(details.starts_with(&format!("SQL dialect for {}\nDialect: not set\n", sql.display())), "{details}");
    assert!(details.contains("Project default: not set\n"), "{details}");
    assert!(details.contains("Assistance: none — no dialect is chosen, so no SQL assistance runs and Lenz does not assume PostgreSQL."), "{details}");
    assert_eq!(toggles(&s, PROJECT), [Some(false), Some(false), Some(true)], "an unset project default is the choice");

    run(&mut s, CommandId::ProjectSqlDialectDuckDb);
    assert_eq!(s.sql_dialect_label().as_deref(), Some("SQL: DuckDB (project) · metadata off"));
    assert_eq!(toggles(&s, PROJECT), [Some(false), Some(true), Some(false)]);
    assert_eq!(toggles(&s, FILE), [Some(false), Some(false), Some(true)], "no file override follows the project");
    let details = s.sql_dialect_details();
    assert!(details.contains("Dialect: DuckDB (project default)\n"), "{details}");
    assert!(details.contains("Project default: DuckDB\n"), "{details}");
    assert!(details.contains("Assistance: DuckDB offline assistance — query-local completion"), "{details}");

    // Saved outside the project, read back by the next session.
    settle(&mut s, |s| s.sql_dialect_label().as_deref() == Some("SQL: DuckDB (project) · metadata off"));
    let saved = settings(d.path()).expect("settings written");
    assert_eq!(saved["projects"][root.to_str().unwrap()]["sql_dialect"], serde_json::json!("DuckDb"), "{saved}");
    drop(s);
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, scanned);
    open(&mut s, Command::Open(sql), "q.sql");
    assert_eq!(s.sql_dialect_label().as_deref(), Some("SQL: DuckDB (project) · metadata off"));
}

#[test]
fn a_file_choice_overrides_the_project_default_survives_restart_and_a_standalone_file_can_choose_its_own() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    std::fs::create_dir(&root).unwrap();
    let sql = write(&root, "q.sql", b"select 1;\n");
    let loose = write(d.path(), "loose.sql", b"select 2;\n");

    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, scanned);
    open(&mut s, Command::Open(sql.clone()), "q.sql");
    run(&mut s, CommandId::ProjectSqlDialectPostgreSql);
    run(&mut s, CommandId::SqlDialectDuckDb);
    assert_eq!(s.sql_dialect_label().as_deref(), Some("SQL: DuckDB (this file) · metadata off"));
    assert_eq!(toggles(&s, FILE), [Some(false), Some(true), Some(false)]);
    assert_eq!(toggles(&s, PROJECT), [Some(true), Some(false), Some(false)]);
    let details = s.sql_dialect_details();
    assert!(details.contains("Dialect: DuckDB (this file)\n"), "{details}");
    assert!(details.contains("Project default: PostgreSQL\n"), "{details}");
    assert!(details.contains("This file's choice is saved by Lenz in "), "{details}");

    // Saved for that file only, read back by the next session.
    settle(&mut s, |_| settings(d.path()).and_then(|v| v["sql_dialects"][sql.to_str().unwrap()].as_str().map(str::to_owned)).as_deref() == Some("DuckDb"));
    drop(s);
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, scanned);
    open(&mut s, Command::Open(sql.clone()), "q.sql");
    assert_eq!(s.sql_dialect_label().as_deref(), Some("SQL: DuckDB (this file) · metadata off"));

    // A file outside the open folder has no project default to inherit: it chooses its own.
    open(&mut s, Command::Open(loose.clone()), "loose.sql");
    assert_eq!(s.sql_dialect_label().as_deref(), Some("SQL: not set · metadata off"));
    assert!(s.sql_dialect_details().contains("Project default: this file is outside the open folder\n"), "{}", s.sql_dialect_details());
    run(&mut s, CommandId::SqlDialectPostgreSql);
    assert_eq!(s.sql_dialect_label().as_deref(), Some("SQL: PostgreSQL (this file) · metadata off"));
    settle(&mut s, |_| settings(d.path()).and_then(|v| v["sql_dialects"][loose.to_str().unwrap()].as_str().map(str::to_owned)).as_deref() == Some("PostgreSql"));

    // Following the project default forgets the file's own choice.
    open(&mut s, Command::Open(sql.clone()), "q.sql");
    run(&mut s, CommandId::SqlDialectFollowProject);
    assert_eq!(s.sql_dialect_label().as_deref(), Some("SQL: PostgreSQL (project) · metadata off"));
    assert_eq!(toggles(&s, FILE), [Some(false), Some(false), Some(true)]);
    settle(&mut s, |_| settings(d.path()).and_then(|v| v.get("sql_dialects").and_then(|m| m.get(sql.to_str().unwrap())).cloned()).is_none());
    assert_eq!(std::fs::read(&sql).unwrap(), b"select 1;\n", "choosing a dialect never writes the document");
    assert_eq!(std::fs::read(&loose).unwrap(), b"select 2;\n");
}

#[test]
fn dialect_choices_report_why_they_cannot_run_and_a_non_sql_file_shows_no_sql_status() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    std::fs::create_dir(&root).unwrap();
    let sql = write(&root, "q.sql", b"select 1;\n");
    let rs = write(&root, "main.rs", b"fn main() {}\n");

    let mut s = session(d.path());
    assert_eq!(s.disabled(CommandId::SqlDialect), Some("No file is open"));
    for id in FILE { assert_eq!(s.disabled(id), Some("No file is open"), "{id:?}"); assert_eq!(s.action(id), CatalogAction::Disabled("No file is open")); }
    for id in PROJECT { assert_eq!(s.disabled(id), Some("Open a folder to choose its SQL dialect"), "{id:?}"); }
    assert!(s.sql_dialect_label().is_none());
    assert_eq!(s.sql_dialect_details(), "No file is open.");

    s.dispatch(Command::NewDocument);
    settle(&mut s, |s| s.state().active().is_some_and(Tab::untitled) && s.state().current_source().is_some());
    assert_eq!(s.disabled(CommandId::SqlDialectPostgreSql), Some("Save this document as a file first"));
    assert!(s.sql_dialect_label().is_none(), "an untitled document has no SQL language yet");

    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, scanned);
    open(&mut s, Command::Open(rs), "main.rs");
    assert!(s.sql_dialect_label().is_none(), "a Rust file shows no SQL status even with a project default");
    assert_eq!((s.disabled(CommandId::SqlDialectDuckDb), s.disabled(CommandId::ProjectSqlDialectDuckDb)), (None, None));

    open(&mut s, Command::Open(sql.clone()), "q.sql");
    assert_eq!(s.action(CommandId::SqlDialect), CatalogAction::Interface(InterfaceAction::SqlDialect));
    assert_eq!(s.disabled(CommandId::SqlDialect), None);
}

#[test]
fn a_dialect_switch_project_change_or_edit_rejects_every_earlier_request_identity() {
    use crate::sql_dialect::Dialect;
    use crate::text::{EditKind, TextEdit};
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    std::fs::create_dir(&root).unwrap();
    let sql = write(&root, "q.sql", b"select 1;\n");
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root));
    settle(&mut s, scanned);
    open(&mut s, Command::Open(sql), "q.sql");
    assert_eq!(s.sql_identity(), None, "no dialect chosen, so no assistance request is issued");

    run(&mut s, CommandId::ProjectSqlDialectPostgreSql);
    let first = s.sql_identity().expect("the project default applies");
    assert_eq!(first.dialect, Dialect::PostgreSql);
    assert!(s.sql_identity_current(first));

    // A file override changes the effective dialect at once; the earlier request can no longer publish.
    run(&mut s, CommandId::SqlDialectDuckDb);
    assert!(!s.sql_identity_current(first));
    let second = s.sql_identity().expect("the file override applies");
    assert_eq!(second.dialect, Dialect::DuckDb);
    assert!(second.generation > first.generation, "the generation fences stale replies");

    // Editing the document makes a captured request stale, even under the same dialect.
    let tab = s.state().active().unwrap().id();
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..0, text: "-- note\n".into() }], kind: EditKind::Typing });
    assert!(!s.sql_identity_current(second));

    // Choosing the dialect already in force is not a context change.
    let third = s.sql_identity().expect("the file override still applies");
    run(&mut s, CommandId::SqlDialectDuckDb);
    assert_eq!(s.sql_identity(), Some(third));
    // Changing the project default under a file that follows it changes the identity too.
    run(&mut s, CommandId::SqlDialectFollowProject);
    let follows = s.sql_identity().expect("the project default applies again");
    run(&mut s, CommandId::ProjectSqlDialectDuckDb);
    assert!(!s.sql_identity_current(follows));
    // Unset issues no request at all.
    run(&mut s, CommandId::ProjectSqlDialectUnset);
    assert_eq!((s.sql_identity(), s.sql_dialect_label().as_deref()), (None, Some("SQL: not set · metadata off")));
}
