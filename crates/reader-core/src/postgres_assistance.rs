//! Offline PostgreSQL assistance. Accepts text only; has no database or process interface.
use crate::sql_assistance;
use sqlparser::dialect::PostgreSqlDialect;
use std::collections::BTreeMap;

/// Query-local completion without external catalog access.
pub fn complete(sql: &str, cursor: usize) -> Result<Vec<String>, String> {
    complete_with_catalog(sql, cursor, &BTreeMap::new())
}

/// Complete against an explicitly supplied immutable catalog snapshot.
pub fn complete_with_catalog(
    sql: &str,
    cursor: usize,
    catalog: &BTreeMap<String, Vec<String>>,
) -> Result<Vec<String>, String> {
    sql_assistance::complete(
        &PostgreSqlDialect {},
        &FUNCTIONS.iter().map(|f| f.name).collect::<Vec<_>>(),
        sql,
        cursor,
        catalog,
    )
}

/// A syntax diagnostic in the original document. Resource/recursion errors have no location.
#[derive(Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub message: String,
    pub byte_offset: Option<usize>,
}

/// PostgreSQL syntax only, never database validation. Positions are UTF-8 bytes.
pub fn diagnose(sql: &str) -> Result<(), Diagnostic> {
    if let Some(byte_offset) = sql.find('\0') {
        return Err(Diagnostic {
            message: "PostgreSQL source contains a NUL byte".into(),
            byte_offset: Some(byte_offset),
        });
    }
    pg_parse::normalize(sql).map(|_| ()).map_err(|error| {
        let byte_offset = match &error {
            pg_parse::Error::ParseError { cursorpos, .. } if *cursorpos > 0 => {
                let character = usize::try_from(*cursorpos - 1).ok();
                character.and_then(|index| {
                    sql.char_indices()
                        .map(|(offset, _)| offset)
                        .chain(std::iter::once(sql.len()))
                        .nth(index)
                })
            }
            _ => None,
        };
        Diagnostic {
            message: error.to_string(),
            byte_offset,
        }
    })
}

/// Guarded source-preserving PostgreSQL whitespace formatting.
pub fn format(sql: &str) -> Result<String, String> {
    let formatted = sql_assistance::format(&PostgreSqlDialect {}, sql)?;
    if diagnose(sql).is_ok() && diagnose(&formatted).is_err() {
        return Err("formatter changed PostgreSQL syntax; original text retained".into());
    }
    Ok(formatted)
}

/// Curated offline PostgreSQL builtins, not extension or catalog overload discovery.
#[derive(Debug)]
pub struct FunctionHelp {
    pub name: &'static str,
    pub signature: &'static str,
    pub documentation: &'static str,
}
const FUNCTIONS: &[FunctionHelp] = &[
    FunctionHelp {
        name: "abs",
        signature: "abs(numeric) → numeric",
        documentation: "Absolute value; PostgreSQL also preserves the input type for its integer and floating point overloads.",
    },
    FunctionHelp {
        name: "jsonb_build_object",
        signature: "jsonb_build_object(VARIADIC any) → jsonb",
        documentation: "Build a JSONB object from alternating keys and values.",
    },
    FunctionHelp {
        name: "date_trunc",
        signature: "date_trunc(text, timestamp) → timestamp",
        documentation: "Truncate a timestamp to the requested precision. PostgreSQL also supports timestamptz and interval overloads.",
    },
];
/// Returns curated help for an unqualified or pg_catalog-qualified builtin name.
pub fn function_help(name: &str) -> Option<&'static FunctionHelp> {
    let name = name.strip_prefix("pg_catalog.").unwrap_or(name);
    FUNCTIONS
        .iter()
        .find(|help| help.name.eq_ignore_ascii_case(name))
}

/// Help for the currently open function call, using a zero-based argument index.
#[derive(Debug)]
pub struct SignatureHelp {
    pub function: &'static FunctionHelp,
    pub active_parameter: usize,
}

/// Token-based call nesting works while arguments and enclosing statements are incomplete.
pub fn signature_help(sql: &str, cursor: usize) -> Result<Option<SignatureHelp>, String> {
    Ok(
        sql_assistance::call_at(&PostgreSqlDialect {}, sql, cursor)?.and_then(
            |(name, active_parameter)| {
                function_help(&name).map(|function| SignatureHelp {
                    function,
                    active_parameter,
                })
            },
        ),
    )
}
