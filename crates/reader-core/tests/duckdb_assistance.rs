use reader_core::duckdb_assistance::complete;

#[test]
fn duckdb_completes_actual_query_local_names_without_metadata() {
    for (sql, expected) in [
        (
            "SELECT u. FROM (SELECT 1 AS id, 'x' AS name) u",
            vec!["id", "name"],
        ),
        (
            "WITH source AS (SELECT 1 AS id), copied AS (SELECT * FROM source) SELECT c. FROM copied c",
            vec!["id"],
        ),
        (
            "SELECT (SELECT n. FROM (SELECT 2 AS nested) n) FROM (SELECT 1 AS outer_id) o",
            vec!["nested"],
        ),
        (
            "WITH \"Source\" (\"User ID\", value) AS (VALUES (1, 'x')) SELECT \"Alias Name\". FROM \"Source\" \"Alias Name\"",
            vec!["\"User ID\"", "value"],
        ),
        (
            "SELECT * FROM (SELECT 1 AS alias_col) a WHERE a.",
            vec!["alias_col"],
        ),
        (
            "WITH c AS (SELECT 1 AS cte_col) SELECT * FROM c WHERE c.",
            vec!["cte_col"],
        ),
        (
            "SELECT * FROM (SELECT * FROM (SELECT 1 AS nested_col) n) q WHERE q.",
            vec!["nested_col"],
        ),
        (
            "SELECT * FROM (SELECT 1 AS \"Mixed Column\") q WHERE q.",
            vec!["\"Mixed Column\""],
        ),
    ] {
        let cursor = sql.find(". ").map_or(sql.len(), |at| at + 1);
        assert_eq!(complete(sql, cursor).unwrap(), expected, "{sql}");
    }
}

#[test]
fn duckdb_specific_completion_and_explicit_catalog_are_usable() {
    use reader_core::duckdb_assistance::complete_with_catalog;
    use std::collections::BTreeMap;
    let sql = "FROM (SELECT 1 AS id, 2 AS value) u SELECT u. GROUP BY ALL";
    assert_eq!(
        complete(sql, sql.find(". ").unwrap() + 1).unwrap(),
        vec!["id", "value"]
    );
    let catalog = BTreeMap::from([("main.users".into(), vec!["id".into(), "name".into()])]);
    let sql = "SELECT main.users. FROM main.users";
    assert_eq!(
        complete_with_catalog(sql, 18, &catalog).unwrap(),
        vec!["id", "name"]
    );
    assert!(complete(sql, 18).unwrap().is_empty());
    let partial = "SELECT u.na FROM main.users u";
    assert_eq!(
        complete_with_catalog(partial, 11, &catalog).unwrap(),
        vec!["name"]
    );

    let sql = "WITH users AS (SELECT 1 AS id) SELECT * FROM users u WHERE u.";
    assert_eq!(complete(sql, sql.len()).unwrap(), vec!["id"]);
    assert!(complete("SELECT λ", 8).is_err());
    assert!(complete("SELECT 'unterminated", 19).is_err());
}

#[test]
#[ignore = "requires explicitly supplied LENZ_TEST_DUCKDB; no automatic installation"]
fn native_duckdb_diagnostics_formatting_and_function_help() {
    use reader_core::duckdb_assistance::DuckDb;
    let native =
        DuckDb::new(std::env::var_os("LENZ_TEST_DUCKDB").expect("explicit native executable"));
    let sql = "SELECT '😀',\n  )";
    let error = native.diagnose(sql).unwrap_err();
    assert_eq!(error.byte_offset, sql.find(')'));
    assert_eq!(error.byte_length, Some(1));
    assert_eq!(
        native.diagnose("SELECT 1 +").unwrap_err().byte_offset,
        Some(9)
    );
    native
        .diagnose("FROM range(3) SELECT * EXCLUDE (range) GROUP BY ALL")
        .unwrap();
    native
        .diagnose("SELECT list_transform([1,2], lambda x: x + 1)")
        .unwrap();
    assert!(native.diagnose("SELECT * EXCLUDE (").is_err());
    let sql = "-- keep 😀\nselect [1,2] as xs, $$a; --b$$ as literal /* exact */ from range(2) group by all;";
    let formatted = native.format(sql).unwrap();
    assert!(formatted.contains("-- keep 😀"));
    assert!(formatted.contains("$$a; --b$$"));
    assert!(formatted.contains("/* exact */"));
    native.diagnose(&formatted).unwrap();
    assert_eq!(native.format(&formatted).unwrap(), formatted);
    assert!(native.format("SELECT 'unterminated").is_err());
    let help = native.function_help("list_transform").unwrap();
    assert_eq!(help[0].parameters, vec!["list", "lambda"]);
    assert_eq!(help[0].return_type.as_deref(), Some("ANY[]"));
    assert!(help[0].description.as_deref().unwrap().contains("lambda"));
    assert!(native.function_help("not_a_function").unwrap().is_empty());
}

#[test]
fn bare_schema_completion_returns_expected_relation_only_with_explicit_metadata() {
    use reader_core::duckdb_assistance::complete_with_catalog;
    use std::collections::BTreeMap;
    let catalog = BTreeMap::from([("ready.fixture".into(), vec!["id".into()])]);
    for sql in ["SELECT * FROM ready.", "SELECT * FROM ready.fi"] {
        assert_eq!(
            complete_with_catalog(sql, sql.len(), &catalog).unwrap(),
            vec!["fixture"]
        );
        assert!(complete(sql, sql.len()).unwrap().is_empty());
    }
}

#[test]
#[ignore = "requires explicitly supplied LENZ_TEST_DUCKDB; no automatic installation"]
fn native_duckdb_signature_help_tracks_incomplete_list_arguments() {
    use reader_core::duckdb_assistance::DuckDb;
    let native = DuckDb::new(std::env::var_os("LENZ_TEST_DUCKDB").unwrap());
    let sql = "SELECT list_transform([1, 2], lambda x:";
    let help = native.signature_help(sql, sql.len()).unwrap().unwrap();
    assert_eq!(help.active_parameter, 1);
    assert_eq!(help.overloads[0].function_name, "list_transform");
    assert!(
        native
            .signature_help("SELECT no_such_fn(", 18)
            .unwrap()
            .is_none()
    );
}

#[test]
fn duckdb_exclude_projection_does_not_offer_removed_column() {
    let sql = "WITH c AS (SELECT * EXCLUDE (hidden) FROM (SELECT 1 AS id, 2 AS hidden) x) SELECT c. FROM c";
    assert_eq!(
        complete(sql, sql.find("c. ").unwrap() + 2).unwrap(),
        vec!["id"]
    );
}

#[cfg(unix)]
#[test]
#[ignore = "owned native executable, fixture and process-boundary evidence"]
fn native_boundary_keeps_destructive_and_quote_escape_documents_as_data() {
    use reader_core::duckdb_assistance::DuckDb;
    use std::{os::unix::fs::PermissionsExt, process::Command};
    let binary = std::path::PathBuf::from(std::env::var_os("LENZ_TEST_DUCKDB").unwrap());
    let owned = tempfile::tempdir().unwrap();
    let database = owned.path().join("private.duckdb");
    let sentinel = owned.path().join("exported.csv");
    let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
    // Explicit fixture setup/verification are separate from every assistance request.
    assert!(
        Command::new(&binary)
            .args(["-no-agent", "-init", "/dev/null"])
            .arg(&database)
            .args([
                "-c",
                "CREATE TABLE users AS SELECT 1 AS id, 'unchanged' AS name;"
            ])
            .status()
            .unwrap()
            .success()
    );
    let original = std::fs::read(&database).unwrap();
    let boundary = owned.path().join("boundary.jsonl");
    let wrapper = owned.path().join("observed-native");
    let wrapper_source = format!(
        "#!/usr/bin/python3\nimport json,os,sys\nwith open({}, 'a') as f: f.write(json.dumps(sys.argv[1:])+'\\n')\nos.execv({}, [{}]+sys.argv[1:])\n",
        serde_json::to_string(&boundary).unwrap(),
        serde_json::to_string(&binary).unwrap(),
        serde_json::to_string(&binary).unwrap()
    );
    let wrapper_source = if let Some(trace_dir) = std::env::var_os("LENZ_SQL_TRACE_DIR") {
        let trace_dir = std::path::PathBuf::from(trace_dir);
        std::fs::create_dir_all(&trace_dir).unwrap();
        wrapper_source.replace(
            &format!("os.execv({}, [{}]+sys.argv[1:])", serde_json::to_string(&binary).unwrap(), serde_json::to_string(&binary).unwrap()),
            &format!("os.execv('/usr/bin/strace', ['strace','-f','-e','trace=network,openat','-o', {}+'/'+str(os.getpid())+'.strace', {}]+sys.argv[1:])", serde_json::to_string(&trace_dir).unwrap(), serde_json::to_string(&binary).unwrap()))
    } else {
        wrapper_source
    };
    std::fs::write(&wrapper, wrapper_source).unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let native = DuckDb::new(&wrapper);
    let documents = [
        "DROP TABLE users; UPDATE users SET name='changed';".to_owned(),
        format!(
            "COPY (SELECT 'export') TO {};",
            quote(sentinel.to_str().unwrap())
        ),
        format!(
            "ATTACH {} AS private; UPDATE private.users SET name='changed';",
            quote(database.to_str().unwrap())
        ),
        format!(
            "SELECT 'x''); COPY (SELECT 1) TO {}; --'",
            sentinel.display()
        ),
        "DROP TABLE users; SELECT 1 +".to_owned(),
    ];
    for (index, sql) in documents.iter().enumerate() {
        let result = native.diagnose(sql);
        if index == 4 {
            assert!(result.is_err());
        } else {
            result.unwrap();
        }
    }
    let trace = std::fs::read_to_string(&boundary).unwrap();
    let observed: Vec<Vec<String>> = trace
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(observed.len(), documents.len());
    for (args, sql) in observed.iter().zip(&documents) {
        assert_eq!(
            &args[..6],
            ["-no-agent", "-init", "/dev/null", "-json", ":memory:", "-c"]
        );
        assert_eq!(args.len(), 7);
        let expected = format!(
            "SET autoinstall_known_extensions=false; SET autoload_known_extensions=false; SET enable_external_access=false; WITH p AS (SELECT json_serialize_sql({}) AS j) SELECT j->'error' AS error, j->>'error_type' AS error_type, j->>'error_message' AS message, j->>'position' AS position, j->>'location' AS location FROM p;",
            quote(sql)
        );
        assert_eq!(args[6], expected);
    }
    assert!(!sentinel.exists());
    assert_eq!(std::fs::read(&database).unwrap(), original);
    let verified = Command::new(&binary)
        .args(["-no-agent", "-init", "/dev/null", "-readonly", "-json"])
        .arg(&database)
        .args(["-c", "SELECT id,name FROM users"])
        .output()
        .unwrap();
    assert!(verified.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&verified.stdout).unwrap(),
        serde_json::json!([{"id":1,"name":"unchanged"}])
    );
    if let Some(dir) = std::env::var_os("LENZ_SQL_EVIDENCE") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("native-boundary.jsonl"), trace).unwrap();
        std::fs::write(dir.join("native-fixture.json"), serde_json::to_vec_pretty(&serde_json::json!({"data_unchanged":true,"database_bytes_unchanged":true,"export_absent":true,"assistance_opened_database":false,"edited_statement_submissions":0,"explain_submissions":0,"fixture_verification":serde_json::from_slice::<serde_json::Value>(&verified.stdout).unwrap()})).unwrap()).unwrap();
    }
}

#[test]
fn unavailable_native_tool_and_invalid_source_return_bounded_errors() {
    use reader_core::duckdb_assistance::DuckDb;
    let native = DuckDb::new("/nonexistent/lenz-owned-missing-duckdb");
    let source = "SELECT 'private-source-token'";
    let error = native.diagnose(source).unwrap_err();
    assert!(error.byte_offset.is_none());
    assert!(!error.message.contains("private-source-token"));
    assert!(native.format(source).is_err());
    assert!(native.diagnose("SELECT '\0'").is_err());
    assert!(native.diagnose(&"x".repeat(32_769)).is_err());
    assert!(complete("SELECT 1", usize::MAX).is_err());
    let deep = format!("SELECT {}1{}", "(".repeat(5000), ")".repeat(5000));
    assert!(complete(&deep, 7).is_err());
}

#[test]
#[ignore = "owned release measurement, not a production frontend performance claim"]
fn duckdb_assistance_measurements() {
    use reader_core::duckdb_assistance::DuckDb;
    let native = DuckDb::new(std::env::var_os("LENZ_TEST_DUCKDB").unwrap());
    let output = std::path::PathBuf::from(std::env::var_os("LENZ_SQL_EVIDENCE").unwrap());
    std::fs::create_dir_all(&output).unwrap();
    let sql =
        "WITH source AS (SELECT 1 AS id), copied AS (SELECT * FROM source) SELECT c. FROM copied c";
    let cursor = sql.find("c. ").unwrap() + 2;
    let mut local = Vec::new();
    let mut parses = Vec::new();
    let mut native_help = Vec::new();
    for _ in 0..20 {
        let start = std::time::Instant::now();
        assert_eq!(complete(sql, cursor).unwrap(), vec!["id"]);
        local.push(start.elapsed().as_nanos());
    }
    for _ in 0..8 {
        let start = std::time::Instant::now();
        native
            .diagnose("SELECT list_transform([1,2], lambda x: x + 1)")
            .unwrap();
        parses.push(start.elapsed().as_nanos());
        let start = std::time::Instant::now();
        assert_eq!(
            native.function_help("list_transform").unwrap()[0].parameters,
            vec!["list", "lambda"]
        );
        native_help.push(start.elapsed().as_nanos());
    }
    let error_sql = "SELECT '😀',\n  )";
    let error = native.diagnose(error_sql).unwrap_err();
    assert_eq!(error.byte_offset, error_sql.find(')'));
    let formatted = native
        .format("select * exclude (id) from ready.fixture group by all;")
        .unwrap();
    std::fs::write(output.join("measurements.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "schema":1,"local_completion_ns":local,"native_syntax_ns":parses,"native_help_ns":native_help,
        "completion":{"source":sql,"cursor_byte":cursor,"result":["id"]},
        "diagnostic":{"source":error_sql,"message":error.message,"byte_offset":error.byte_offset,"byte_length":error.byte_length},
        "format":{"source":"select * exclude (id) from ready.fixture group by all;","result":formatted},
        "scope":"isolated prerequisite API on a contended development host; no production UI or worker claim"
    })).unwrap()).unwrap();
}

#[test]
#[ignore = "explicit native parser classification; no statements are executed"]
fn native_non_select_serialization_limit_is_not_reported_as_invalid_syntax() {
    use reader_core::duckdb_assistance::DuckDb;
    let native = DuckDb::new(std::env::var_os("LENZ_TEST_DUCKDB").unwrap());
    for sql in [
        "CREATE TABLE absent_table(id INTEGER)",
        "DROP TABLE absent_table",
        "INSERT INTO absent_table VALUES (1)",
        "UPDATE absent_table SET id = 2",
        "DELETE FROM absent_table",
        "EXPLAIN SELECT * FROM absent_table",
    ] {
        assert_eq!(native.diagnose(sql), Ok(()), "{sql}");
    }
    for sql in [
        "UPDATE absent_table SET",
        "CREATE TABLE absent_table(id INTEGER); SELECT 1 +",
        "DROP TABLE absent_table; SELECT '😀', )",
    ] {
        let error = native.diagnose(sql).unwrap_err();
        assert!(error.byte_offset.is_some(), "{sql}: {error:?}");
        assert!(!error.message.contains("Only SELECT statements"));
    }
}
