//! The SQL dialect choice, its status bar label and its details, through the terminal's real input and render.
use super::*;

fn sql_project(dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let root = dir.join("project");
    std::fs::create_dir_all(&root).unwrap();
    let sql = root.join("q.sql");
    std::fs::write(&sql, "select 1;\n").unwrap();
    (root, sql)
}

fn palette(a: &mut App, query: &str) {
    key(a, KeyCode::F(10), KeyModifiers::NONE);
    a.handle(Event::Paste(query.into()));
    key(a, KeyCode::Enter, KeyModifiers::NONE);
}

fn screen(a: &mut App, width: u16, height: u16) -> String {
    draw_screen(a, width, height)
}

#[test]
fn the_status_bar_shows_the_chosen_sql_dialect_and_its_details_open_from_that_label() {
    let d = tempfile::tempdir().unwrap();
    let (root, sql) = sql_project(d.path());
    let mut a = app(d.path());
    a.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut a, |a| a.session.state().project().is_some());
    a.dispatch(Command::OpenRetained(sql.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());

    let drawn = screen(&mut a, 100, 20);
    assert!(drawn.contains("SQL: not set"), "{drawn}");
    assert!(!drawn.contains("PostgreSQL"), "Lenz does not assume PostgreSQL:\n{drawn}");

    // The palette lists the shared commands and Enter runs the one drawn for "DuckDB for this file".
    palette(&mut a, "file: duckdb");
    assert_eq!(a.session.status_message(), "SQL dialect for q.sql: DuckDB");
    let drawn = screen(&mut a, 100, 20);
    assert!(drawn.contains("SQL: DuckDB (this file)"), "{drawn}");

    // The label is a control: clicking it opens the same details the dialogs show.
    let label = a.layout.controls.iter().find(|(id, _)| *id == CommandId::SqlDialect).map(|(_, rect)| *rect).expect("the SQL status control");
    click(&mut a, label.x + 1, label.y);
    assert_eq!(a.modal_title(), "SQL dialect");
    let Some(Prompt::Details { body, .. }) = a.modal.as_ref().map(|m| &m.kind) else { panic!("the status label opens the details dialog") };
    let rows: Vec<&str> = body.lines().collect();
    assert_eq!(rows[..3], [
        format!("SQL dialect for {}", sql.display()).as_str(),
        "Dialect: DuckDB (this file)",
        "Project default: not set",
    ]);
    assert!(rows[3].starts_with("Assistance: DuckDB offline assistance — query-local completion"), "{rows:?}");
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.modal.is_none());

    // A project default applies once the file follows it, and the status says which one wins.
    palette(&mut a, "project sql dialect: duckdb");
    palette(&mut a, "follow project default");
    let drawn = screen(&mut a, 100, 20);
    assert!(drawn.contains("SQL: DuckDB (project)"), "{drawn}");
    // A tiny terminal still draws and dismisses without panicking, and the label simply does not fit.
    screen(&mut a, 26, 8);
    screen(&mut a, 1, 1);

    // Settings persist asynchronously: wait for the complete latest snapshot before inspecting disk.
    let settings_path = d.path().join("config/settings.json");
    settle(&mut a, |_| std::fs::read(&settings_path).ok().and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok()).is_some_and(|settings|
        settings["projects"][root.to_str().unwrap()]["sql_dialect"] == serde_json::json!("DuckDb")
            && settings.get("sql_dialects").and_then(|m| m.get(sql.to_str().unwrap())).is_none()));
    let settings: serde_json::Value = serde_json::from_slice(&std::fs::read(settings_path).unwrap()).unwrap();
    assert_eq!(settings["projects"][root.to_str().unwrap()]["sql_dialect"], serde_json::json!("DuckDb"), "{settings}");
    assert_eq!(settings.get("sql_dialects").and_then(|m| m.get(sql.to_str().unwrap())), None, "following the project forgets the file's own choice: {settings}");
    assert_eq!(std::fs::read(&sql).unwrap(), b"select 1;\n", "choosing a dialect writes neither the document nor the project");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
}
