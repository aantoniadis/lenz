//! DuckDB query-local assistance; catalog snapshots are explicit and never opened here.
use sqlparser::dialect::DuckDbDialect;
use std::collections::BTreeMap;
// DuckDB 2.0 accepts wildcard RENAME; sqlparser 0.63's base dialect omits that flag.
// Derive rather than duplicating its grammar/flags, retaining its type identity for parser checks.
sqlparser::derive_dialect!(EditingDialect, DuckDbDialect, preserve_type_id = true,
    overrides = { supports_select_wildcard_rename = true });

pub(crate) fn editing_dialect() -> impl sqlparser::dialect::Dialect { EditingDialect::new() }

/// Query-local completion, including bare qualifiers, without opening any engine.
pub fn complete(sql: &str, cursor: usize) -> Result<Vec<String>, String> {
    complete_with_catalog(sql, cursor, &BTreeMap::new())
}

/// Metadata is an immutable caller-supplied snapshot, never a database path.
pub fn complete_with_catalog(
    sql: &str,
    cursor: usize,
    catalog: &BTreeMap<String, Vec<String>>,
) -> Result<Vec<String>, String> {
    crate::sql_assistance::complete(
        &EditingDialect::new(),
        &["abs", "avg", "coalesce", "count", "date_trunc", "generate_series", "json_extract", "length", "list_aggregate", "list_filter", "list_transform", "lower", "max", "min", "range", "regexp_extract", "round", "string_agg", "sum", "unnest", "upper"],
        sql,
        cursor,
        catalog,
    )
}

/// Original-source syntax position. Unavailable tools/resource errors have no source range.
#[derive(Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub message: String,
    pub byte_offset: Option<usize>,
    pub byte_length: Option<usize>,
}

/// Explicitly selected native executable. Every request opens only an in-memory engine.
/// Run these bounded synchronous requests off the UI thread; this is not a catalog connection.
pub struct DuckDb {
    executable: std::path::PathBuf,
    current: std::sync::Arc<dyn Fn() -> bool + Send + Sync>,
}

#[derive(serde::Deserialize)]
struct Parsed {
    error: bool,
    error_type: Option<String>,
    message: Option<String>,
    position: Option<String>,
    location: Option<String>,
}

/// Builtin signatures from the selected DuckDB binary, with at most eight overloads.
#[derive(Debug, serde::Deserialize)]
pub struct FunctionHelp {
    pub function_name: String,
    pub parameters: Vec<String>,
    pub parameter_types: Vec<String>,
    pub return_type: Option<String>,
    pub description: Option<String>,
}

impl DuckDb {
    pub fn new(executable: impl Into<std::path::PathBuf>) -> Self {
        Self {
            executable: executable.into(), current: std::sync::Arc::new(|| true),
        }
    }

    pub(crate) fn with_cancellation(mut self, current: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        self.current = std::sync::Arc::new(current); self
    }

    /// Native DuckDB grammar only: no binding, schema validation, user statement or EXPLAIN execution.
    pub fn diagnose(&self, sql: &str) -> Result<(), Diagnostic> {
        let failure = |message| Diagnostic {
            message,
            byte_offset: None,
            byte_length: None,
        };
        let literal = literal(sql).map_err(failure)?;
        let query = format!(
            "WITH p AS (SELECT json_serialize_sql({literal}) AS j) SELECT j->'error' AS error, j->>'error_type' AS error_type, j->>'error_message' AS message, j->>'position' AS position, j->>'location' AS location FROM p"
        );
        let parsed: Vec<Parsed> = self.query(&query).map_err(failure)?;
        let parsed = parsed
            .first()
            .ok_or_else(|| failure("DuckDB returned no parser result".into()))?;
        // The parser accepts all statements before serialization rejects non-SELECT ASTs.
        if !parsed.error
            || (parsed.error_type.as_deref() == Some("not implemented")
                && parsed.message.as_deref()
                    == Some("Only SELECT statements can be serialized to json!"))
        {
            return Ok(());
        }
        let offset = parsed
            .position
            .as_deref()
            .and_then(|p| p.parse::<usize>().ok())
            .filter(|offset| sql.is_char_boundary(*offset));
        let length = parsed
            .location
            .as_deref()
            .and_then(|value| serde_json::from_str::<[usize; 2]>(value).ok())
            .and_then(|[start, len]| {
                (Some(start) == offset
                    && start
                        .checked_add(len)
                        .is_some_and(|end| sql.is_char_boundary(end)))
                .then_some(len)
            });
        Err(Diagnostic {
            message: parsed
                .message
                .clone()
                .unwrap_or_else(|| "DuckDB parser failed".into()),
            byte_offset: offset,
            byte_length: length,
        })
    }

    /// Guarded whitespace formatting, preserving comments/literals and valid native syntax.
    /// Incomplete syntax can still be formatted; failure retains the caller's original draft.
    pub fn format(&self, sql: &str) -> Result<String, String> {
        self.format_with_indent(sql, sqlformat::Indent::Spaces(2))
    }

    pub(crate) fn format_with_indent(&self, sql: &str, indent: sqlformat::Indent) -> Result<String, String> {
        literal(sql)?;
        let formatted = crate::sql_assistance::format_with_indent(&DuckDbDialect {}, sql, indent)?;
        let original = self.diagnose(sql);
        if original
            .as_ref()
            .is_err_and(|error| error.byte_offset.is_none())
        {
            return Err("DuckDB syntax validation unavailable; original text retained".into());
        }
        if original.is_ok() && self.diagnose(&formatted).is_err() {
            return Err("formatter changed DuckDB syntax; original text retained".into());
        }
        Ok(formatted)
    }

    /// Native builtin help only. No user catalog is opened and the name is passed as data.
    pub fn function_help(&self, name: &str) -> Result<Vec<FunctionHelp>, String> {
        if name.len() > 256 {
            return Err("function name exceeds 256 bytes".into());
        }
        let name = literal(name)?;
        self.query(&format!("SELECT function_name, parameters, parameter_types, return_type, left(description, 512) AS description FROM duckdb_functions() WHERE lower(function_name) = lower({name}) LIMIT 8"))
    }

    /// Native builtin help for the innermost incomplete function call.
    pub fn signature_help(
        &self,
        sql: &str,
        cursor: usize,
    ) -> Result<Option<SignatureHelp>, String> {
        let Some((name, active_parameter)) =
            crate::sql_assistance::call_at(&DuckDbDialect {}, sql, cursor)?
        else {
            return Ok(None);
        };
        let overloads = self.function_help(&name)?;
        Ok((!overloads.is_empty()).then_some(SignatureHelp {
            overloads,
            active_parameter,
        }))
    }

    fn query<T: serde::de::DeserializeOwned>(&self, query: &str) -> Result<T, String> {
        let script = format!(
            "SET threads=1; SET memory_limit='128MB'; SET autoinstall_known_extensions=false; SET autoload_known_extensions=false; SET enable_external_access=false; {query};"
        );
        let output = crate::sql_process::run(&self.executable,
            &["-no-agent", "-init", "/dev/null", "-json", ":memory:"], &format!("{script}\n"), &[],
            std::time::Duration::from_secs(3), 131072, &*self.current)
            .map_err(|_| "DuckDB parser unavailable or request exceeded its resource limit".to_owned())?;
        serde_json::from_str(&output)
            .map_err(|_| "DuckDB returned invalid or oversized assistance output".into())
    }
}

fn literal(text: &str) -> Result<String, String> {
    // Bound native work. Encode source as data on one CLI line so embedded dot commands cannot run.
    if text.len() > 32_768 || text.contains('\0') {
        return Err("DuckDB native assistance requires NUL-free source up to 32 KiB".into());
    }
    let hex = text.as_bytes().iter().map(|b| format!("{b:02x}")).collect::<String>();
    Ok(format!("decode(from_hex('{hex}'))"))
}

/// Signatures and zero-based argument index for the current builtin call.
#[derive(Debug)]
pub struct SignatureHelp {
    pub overloads: Vec<FunctionHelp>,
    pub active_parameter: usize,
}
