//! Go-to-line input: `line`, `line:column` (or `line,column`), and a finder's `name:line[:column]`. 1-based.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineColumn {
    pub line: u32,
    pub column: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GotoError;

impl fmt::Display for GotoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("expected line or line:column")
    }
}

impl std::error::Error for GotoError {}

/// A positive decimal number, digits only.
fn number(text: &str) -> Option<u32> {
    let text = text.trim();
    (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())).then(|| text.parse::<u32>().ok()).flatten().filter(|n| *n > 0)
}

pub fn parse_line_column(text: &str) -> Result<LineColumn, GotoError> {
    let (line, column) = match text.split_once([':', ',']) {
        Some((line, column)) => (line, Some(number(column).ok_or(GotoError)?)),
        None => (text, None),
    };
    Ok(LineColumn { line: number(line).ok_or(GotoError)?, column })
}

/// `name:line` or `name:line:column` → the name and the place; anything else is all name.
pub fn split_path_line(text: &str) -> (&str, Option<LineColumn>) {
    let Some((head, last)) = text.rsplit_once(':') else { return (text, None) };
    let Some(last) = number(last).filter(|_| !last.contains(' ')) else { return (text, None) };
    match head.rsplit_once(':').and_then(|(name, line)| number(line).filter(|_| !line.contains(' ')).map(|l| (name, l))) {
        Some((name, line)) => (name, Some(LineColumn { line, column: Some(last) })),
        None => (head, Some(LineColumn { line: last, column: None })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lc(line: u32, column: Option<u32>) -> LineColumn {
        LineColumn { line, column }
    }

    #[test]
    fn lines_and_columns_parse_one_based() {
        assert_eq!(parse_line_column("120"), Ok(lc(120, None)));
        assert_eq!(parse_line_column(" 120:5 "), Ok(lc(120, Some(5))));
        assert_eq!(parse_line_column("120,5"), Ok(lc(120, Some(5))));
        assert_eq!(parse_line_column("7 : 2"), Ok(lc(7, Some(2))));
    }

    #[test]
    fn anything_else_is_refused_with_one_message() {
        for bad in ["", "0", "x", "12x", "1:0", "1:", ":4", "-3", "99999999999", "1:2:3"] {
            assert_eq!(parse_line_column(bad), Err(GotoError), "{bad:?}");
        }
        assert_eq!(GotoError.to_string(), "expected line or line:column");
    }

    #[test]
    fn finder_queries_split_off_a_line() {
        assert_eq!(split_path_line("session.rs:120"), ("session.rs", Some(lc(120, None))));
        assert_eq!(split_path_line("session.rs:120:5"), ("session.rs", Some(lc(120, Some(5)))));
        assert_eq!(split_path_line("driver"), ("driver", None));
        assert_eq!(split_path_line("driver:"), ("driver:", None), "typing in progress keeps the name");
        assert_eq!(split_path_line("c:/odd"), ("c:/odd", None));
        assert_eq!(split_path_line("x:0"), ("x:0", None));
    }
}
