use code_core::postgres_assistance::complete;

#[test]
fn completes_query_local_alias_without_a_catalog() {
    let sql = "SELECT u. FROM (SELECT 1 AS id, 'x' AS name) AS u";
    assert_eq!(complete(sql, 9).unwrap(), vec!["id", "name"]);
}

#[test]
fn completes_cte_chains_quoted_names_and_nested_scope() {
    for (sql, expected) in [
        (
            "WITH source AS (SELECT 1 AS id), copied AS (SELECT * FROM source) SELECT c. FROM copied c",
            vec!["id"],
        ),
        (
            "WITH \"Source\" (\"User ID\", value) AS (VALUES (1, 'x')) SELECT \"Alias\". FROM \"Source\" \"Alias\"",
            vec!["\"User ID\"", "value"],
        ),
        (
            "SELECT (SELECT inner_q. FROM (SELECT 2 AS nested) inner_q) FROM (SELECT 1 AS outer_id) outer_q",
            vec!["nested"],
        ),
    ] {
        let cursor = sql.find(". ").unwrap() + 1;
        assert_eq!(complete(sql, cursor).unwrap(), expected, "{sql}");
    }
}

#[test]
fn supplied_catalog_resolves_schema_qualified_base_table_and_alias() {
    use code_core::postgres_assistance::complete_with_catalog;
    use std::collections::BTreeMap;
    let catalog = BTreeMap::from([("public.users".into(), vec!["id".into(), "email".into()])]);
    for sql in [
        "SELECT public.users. FROM public.users",
        "SELECT u.em FROM public.users u",
    ] {
        let cursor = sql.find(" FROM").unwrap();
        let expected = if sql.contains("u.em") {
            vec!["email"]
        } else {
            vec!["email", "id"]
        };
        assert_eq!(
            complete_with_catalog(sql, cursor, &catalog).unwrap(),
            expected
        );
    }
    assert!(
        complete("SELECT u. FROM public.users u", 9)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn postgres_diagnostics_point_at_original_utf8_byte_and_eof() {
    use code_core::postgres_assistance::diagnose;
    let sql = "SELECT 'λ',\n  )";
    let error = diagnose(sql).unwrap_err();
    assert_eq!(error.byte_offset, Some(sql.find(")").unwrap()));
    assert_eq!(diagnose("SELECT 1 +").unwrap_err().byte_offset, Some(10));
    assert!(diagnose("SELECT payload->>'name', ARRAY[1,2]::int[], $$a; b$$ FROM records WHERE name ILIKE 'a%' RETURNING").is_err());
    assert!(diagnose("SELECT DISTINCT ON (id) id, payload->>'name', ARRAY[1,2]::int[], $$a; b$$ FROM records WHERE name ILIKE 'a%' ORDER BY id").is_ok());
}

#[test]
fn formatting_preserves_comments_dollar_strings_pg_operators_and_incomplete_input() {
    use code_core::postgres_assistance::format;
    let sql = "-- keep λ\nselect payload->>'name', ARRAY[1,2]::int[], $$begin; -- not comment$$ /*keep*/ from records where id =";
    let formatted = format(sql).unwrap();
    assert!(formatted.contains("-- keep λ"));
    assert!(formatted.contains("$$begin; -- not comment$$"));
    assert!(formatted.contains("/*keep*/"));
    assert!(formatted.ends_with('='));
    assert_eq!(format(&formatted).unwrap(), formatted);
    assert!(format("SELECT 'unterminated").is_err());
}

#[test]
fn postgres_function_help_and_partial_function_completion_are_offline() {
    use code_core::postgres_assistance::function_help;
    let help = function_help("pg_catalog.jsonb_build_object").unwrap();
    assert_eq!(help.signature, "jsonb_build_object(VARIADIC any) → jsonb");
    assert!(
        function_help("date_trunc")
            .unwrap()
            .signature
            .contains("timestamp")
    );
    assert_eq!(
        complete("SELECT jsonb_b", 14).unwrap(),
        vec!["jsonb_build_object"]
    );
    assert!(function_help("unknown_extension_function").is_none());
}

#[test]
fn postgres_identifier_case_and_quoted_alias_spaces_are_respected() {
    let sql = "WITH Source AS (SELECT 1 AS ID) SELECT s. FROM source S";
    assert_eq!(
        complete(sql, sql.find(". ").unwrap() + 1).unwrap(),
        vec!["id"]
    );
    let sql = "SELECT \"Alias Name\". FROM (SELECT 1 AS \"User ID\") \"Alias Name\"";
    assert_eq!(
        complete(sql, sql.find(". ").unwrap() + 1).unwrap(),
        vec!["\"User ID\""]
    );
}

#[test]
fn native_postgres_grammar_rejects_reserved_from_at_the_actual_token() {
    use code_core::postgres_assistance::diagnose;
    let sql = "SELECT 'λ',\n  FROM users";
    assert_eq!(
        diagnose(sql).unwrap_err().byte_offset,
        Some(sql.find("FROM").unwrap())
    );
    let sql = "SELECT 'λ', )";
    assert_eq!(
        diagnose(sql).unwrap_err().byte_offset,
        Some(sql.find(')').unwrap())
    );
}

#[test]
fn malicious_documents_are_data_and_never_execute_or_explain() {
    use code_core::postgres_assistance::{diagnose, format};
    let owned = tempfile::tempdir().unwrap();
    let sentinel = owned.path().join("executed");
    let fixture = owned.path().join("rows");
    std::fs::write(&fixture, "1,unchanged\n").unwrap();
    let sql = format!(
        "DROP TABLE users; UPDATE users SET name = 'changed'; COPY (SELECT 'quote''escape') TO PROGRAM 'touch {}';",
        sentinel.display()
    );
    diagnose(&sql).unwrap();
    let formatted = format(&sql).unwrap();
    assert!(formatted.contains("DROP"));
    eprintln!(
        "postgres-boundary {}",
        serde_json::json!({"source":sql,"syntax":"accepted","formatted":formatted})
    );
    assert!(!sentinel.exists());
    assert_eq!(std::fs::read_to_string(fixture).unwrap(), "1,unchanged\n");
    let sql = format!("COPY users TO '{}'", sentinel.display());
    diagnose(&sql).unwrap();
    assert!(!sentinel.exists());
}

#[test]
fn malformed_and_deep_documents_return_errors_without_panicking() {
    use code_core::postgres_assistance::{diagnose, format};
    assert_eq!(diagnose("SELECT '\0'").unwrap_err().byte_offset, Some(8));
    assert!(complete("SELECT λ", 8).is_err()); // Inside the UTF-8 scalar.
    assert!(complete("SELECT 1", usize::MAX).is_err());
    let deep = format!("SELECT {}1{}", "(".repeat(20_000), ")".repeat(20_000));
    assert!(diagnose(&deep).is_err());
    assert!(complete(&deep, 7).is_err());
    assert!(format("SELECT $tag$unterminated").is_err());
}

#[test]
fn query_local_completion_works_in_predicates_and_preserves_string_newline_semantics() {
    use code_core::postgres_assistance::{diagnose, format};
    let sql = "WITH users AS (SELECT 1 AS id) SELECT * FROM users u WHERE u.";
    assert_eq!(complete(sql, sql.len()).unwrap(), vec!["id"]);
    let sql = "SELECT 'foo'\n'bar';";
    diagnose(sql).unwrap();
    if let Ok(formatted) = format(sql) {
        diagnose(&formatted).unwrap();
    }
}

/// Run only in the owned evidence process; includes repeated successful and failing native parses.
#[test]
#[ignore]
fn postgres_assistance_measurements() {
    use code_core::postgres_assistance::{diagnose, format, function_help, signature_help};
    use std::time::{Duration, Instant};
    let root = std::path::PathBuf::from(
        std::env::var_os("POSTGRES_EVIDENCE_DIR").expect("owned evidence directory"),
    );
    let query = "WITH users AS (SELECT 1 AS id, 'λ' AS name) SELECT u. FROM users u";
    let cursor = query.find(". ").unwrap() + 1;
    let valid = "-- keep\nSELECT DISTINCT ON (id) id, jsonb_build_object('name', name), ARRAY[1,2]::int[], $$a; --b$$ FROM users WHERE name ILIKE 'a%' ORDER BY id";
    let invalid = "SELECT 'λ',\n FROM users";
    let mut completion_ns = Vec::new();
    let mut diagnostic_ns = Vec::new();
    let mut format_ns = Vec::new();
    let mut signature_ns = Vec::new();
    let mut payloads = Vec::new();
    for _ in 0..100 {
        let start = Instant::now();
        let completed = complete(query, cursor).unwrap();
        completion_ns.push(start.elapsed().as_nanos() as u64);
        assert_eq!(completed, vec!["id", "name"]);
        let start = Instant::now();
        let diagnostic = diagnose(invalid).unwrap_err();
        diagnostic_ns.push(start.elapsed().as_nanos() as u64);
        assert_eq!(diagnostic.byte_offset, invalid.find("FROM"));
        let start = Instant::now();
        let formatted = format(valid).unwrap();
        format_ns.push(start.elapsed().as_nanos() as u64);
        diagnose(&formatted).unwrap();
        let signature_source = "SELECT date_trunc('day', abs(";
        let start = Instant::now();
        let signature = signature_help(signature_source, signature_source.len())
            .unwrap()
            .unwrap();
        signature_ns.push(start.elapsed().as_nanos() as u64);
        assert_eq!(signature.function.name, "abs");
        if payloads.is_empty() {
            payloads.push(serde_json::json!({"completion": {"source":query,"byte_offset":cursor,"result":completed}, "diagnostic":{"source":invalid,"message":diagnostic.message,"byte_offset":diagnostic.byte_offset}, "format":{"source":valid,"result":formatted}, "help":function_help("jsonb_build_object").unwrap().signature}));
        }
    }
    let started = Instant::now();
    let mut next_sample = Duration::ZERO;
    let mut rss = Vec::new();
    let mut cycles = 0;
    while started.elapsed() < Duration::from_secs(60) {
        assert_eq!(complete(query, cursor).unwrap(), vec!["id", "name"]);
        diagnose(valid).unwrap();
        assert_eq!(
            diagnose(invalid).unwrap_err().byte_offset,
            invalid.find("FROM")
        );
        assert!(format(valid).unwrap().contains("$$a; --b$$"));
        cycles += 1;
        if started.elapsed() >= next_sample {
            rss.push(serde_json::json!({"elapsed_ms":started.elapsed().as_millis(),"status":std::fs::read_to_string("/proc/self/status").unwrap()}));
            next_sample += Duration::from_secs(1);
        }
    }
    std::fs::write(root.join("measurements.json"), serde_json::to_vec_pretty(&serde_json::json!({"schema":1,"provider":"pg_parse 0.15.0 / libpg_query PostgreSQL 18.4; sqlparser 0.63.0 PostgreSqlDialect scope adapter; sqlformat 0.5.0 PostgreSql + token/native guards", "completion_ns":completion_ns,"diagnostic_ns":diagnostic_ns,"format_ns":format_ns,"signature_ns":signature_ns,"payloads":payloads,"stress_elapsed_ms":started.elapsed().as_millis(),"stress_cycles":cycles,"rss":rss})).unwrap()).unwrap();
}

#[test]
fn malformed_unicode_separator_never_slices_inside_utf8() {
    let sql = "SELECT 1→";
    let _ = complete(sql, sql.len());
}

#[test]
fn completion_bounds_deep_expression_work() {
    let near_limit = format!("SELECT {}candidate", "1 + ".repeat(2_000));
    assert!(complete(&near_limit, near_limit.len()).is_err());
    let over_limit = format!("SELECT {}candidate", "1 + ".repeat(3_000));
    assert!(complete(&over_limit, over_limit.len()).is_err());
    let sql = format!("SELECT {}candidate", "1 + ".repeat(80));
    assert!(complete(&sql, sql.len()).is_err());
}

#[test]
fn aliases_hide_original_relation_names_and_catalogs_do_not_invent_local_columns() {
    let sql = "WITH users AS (SELECT 1 AS id) SELECT users. FROM users u";
    assert!(
        complete(sql, sql.find(". ").unwrap() + 1)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn incomplete_format_has_a_pinned_output_and_empty_documents_are_valid() {
    use code_core::postgres_assistance::{diagnose, format};
    assert_eq!(
        format("select id,name from users where id =").unwrap(),
        "select\n  id,\n  name\nfrom\n  users\nwhere\n  id ="
    );
    for source in ["", "-- comment only", "/* comment only */"] {
        diagnose(source).unwrap();
    }
}

#[test]
fn query_signature_help_tracks_nested_calls_and_active_argument() {
    use code_core::postgres_assistance::{function_help, signature_help};
    assert_eq!(
        function_help("abs").unwrap().signature,
        "abs(numeric) → numeric"
    );
    let sql = "SELECT abs(date_trunc('day', ";
    let help = signature_help(sql, sql.len()).unwrap().unwrap();
    assert_eq!(help.function.name, "date_trunc");
    assert_eq!(help.active_parameter, 1);
    let sql = "SELECT date_trunc('day', pg_catalog.abs(";
    let help = signature_help(sql, sql.len()).unwrap().unwrap();
    assert_eq!(help.function.name, "abs");
    assert_eq!(help.active_parameter, 0);
    assert!(signature_help("SELECT abs(1)", 13).unwrap().is_none());
}

#[test]
fn completes_union_recursive_cte_and_qualified_wildcard_outputs() {
    for (sql, expected) in [
        ("WITH c AS (SELECT 1 AS first_name UNION ALL SELECT 2 AS ignored_name) SELECT c. FROM c", vec!["first_name"]),
        ("WITH RECURSIVE c(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM c WHERE n < 5) SELECT c. FROM c", vec!["n"]),
        ("WITH a AS (SELECT 1 AS id), b AS (SELECT a.* FROM a) SELECT b. FROM b", vec!["id"]),
    ] {
        let at = sql.find(". FROM").unwrap() + 1;
        assert_eq!(complete(sql, at).unwrap(), expected, "{sql}");
    }
}

#[test]
fn completes_unfinished_quoted_identifiers_and_pg_catalog_functions() {
    let sql = "SELECT q.\"User N FROM (SELECT 1 AS \"User Name\") q";
    assert_eq!(complete(sql, sql.find(" FROM").unwrap()).unwrap(), ["\"User Name\""]);
    assert_eq!(complete("SELECT pg_catalog.jsonb_b", "SELECT pg_catalog.jsonb_b".len()).unwrap(), ["jsonb_build_object"]);
}

#[test]
fn completion_honors_non_lateral_visibility_and_ambiguous_column_names() {
    let sql = "SELECT * FROM (SELECT 1 AS secret) outer_q, (SELECT outer_q. ) hidden";
    assert!(complete(sql, sql.find("outer_q. ").unwrap() + "outer_q.".len()).unwrap().is_empty());
    let sql = "WITH a AS (SELECT 1 AS id, 2 AS only_a), b AS (SELECT 3 AS id) SELECT i FROM a JOIN b ON true";
    assert_eq!(complete(sql, sql.find("SELECT i FROM").unwrap() + 8).unwrap(), Vec::<String>::new());
}

#[test]
fn table_functions_expose_known_output_or_explicit_alias_columns_without_running_the_function() {
    for (sql, expected) in [
        ("SELECT g. FROM generate_series(1, 3) g(n)", vec!["n"]),
        ("SELECT g. FROM generate_series(1, 3) g", vec!["generate_series"]),
        ("SELECT j. FROM jsonb_each('{}'::jsonb) j", vec!["key", "value"]),
    ] {
        assert_eq!(complete(sql, sql.find(". FROM").unwrap() + 1).unwrap(), expected, "{sql}");
    }
}

#[test]
fn an_unfinished_different_statement_does_not_hide_current_statement_scope() {
    let sql = "SELECT q. FROM (SELECT 1 AS id) q;\nSELECT '𝄞', )";
    assert_eq!(complete(sql, sql.find("q. FROM").unwrap() + 2).unwrap(), ["id"]);
}

#[test]
fn empty_sql_statements_have_no_query_local_completions() {
    for sql in ["", " \r\n", "SELECT 1;\n\n", "SELECT 1;\n-- next query\n", "/* next query */ \n", "SELECT 1;\nsel", "selesct *\nfrom bars;\ns"] {
        for duck in [false, true] {
            let result = if duck { code_core::duckdb_assistance::complete(sql, sql.len()) } else { complete(sql, sql.len()) };
            assert_eq!(result.unwrap(), Vec::<String>::new(), "duck={duck}, SQL={sql:?}");
        }
    }
}
