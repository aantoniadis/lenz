//! Shared SQL scope resolution and guarded source formatting, with dialect supplied by each route.
use sqlparser::{
    ast::{Expr, Ident, Query, SelectItem, SetExpr, TableFactor, Visit, Visitor},
    parser::Parser,
};
use std::{collections::BTreeMap, ops::ControlFlow};

const CURSOR: &str = "__lenz_completion_cursor__";
type Names = BTreeMap<String, Vec<String>>;
#[derive(Clone, Default)]
struct Scope {
    definitions: Names,
    visible: Names,
}

pub(crate) fn complete(
    dialect: &dyn sqlparser::dialect::Dialect,
    functions: &[&str],
    sql: &str,
    cursor: usize,
    catalog: &BTreeMap<String, Vec<String>>,
) -> Result<Vec<String>, String> {
    if !sql.is_char_boundary(cursor) || sql.contains(CURSOR) {
        return Err("invalid completion offset or reserved cursor identifier".into());
    }
    if sql.len() > 262_144 {
        return Err("completion source exceeds 256 KiB".into());
    }
    let tokens = significant_tokens(dialect, sql)?;
    if tokens.len() > 4_096 {
        return Err("completion source exceeds 4096 tokens".into());
    }
    let before = sql.get(..cursor).ok_or("invalid completion offset")?;
    let start = before
        .char_indices()
        .rev()
        .find(|(_, c)| !c.is_alphanumeric() && *c != '_')
        .map_or(0, |(offset, ch)| offset + ch.len_utf8());
    let (head, prefix) = before
        .split_at_checked(start)
        .ok_or("invalid completion boundary")?;
    let qualifier = head
        .strip_suffix('.')
        .map(|head| qualifier_name(dialect, head))
        .transpose()?;
    let tail = sql.get(cursor..).ok_or("invalid completion offset")?;
    let repaired = format!("{head}{CURSOR}{tail}");
    let statements = Parser::parse_sql(dialect, &repaired).map_err(|e| e.to_string())?;
    let mut visitor = CompletionVisitor {
        functions,
        relation_cursor: false,
        expression_depth: 0,
        scopes: vec![Scope {
            definitions: catalog.clone(),
            visible: Names::new(),
        }],
        qualifier,
        prefix,
        result: Vec::new(),
    };
    if statements.visit(&mut visitor).is_break() {
        return Err("completion expression depth exceeds 64".into());
    }
    visitor.result.sort();
    visitor.result.dedup();
    Ok(visitor.result)
}

struct CompletionVisitor<'a> {
    functions: &'a [&'a str],
    relation_cursor: bool,
    expression_depth: usize,
    scopes: Vec<Scope>,
    qualifier: Option<String>,
    prefix: &'a str,
    result: Vec<String>,
}
impl Visitor for CompletionVisitor<'_> {
    type Break = ();
    fn pre_visit_expr(&mut self, _: &Expr) -> ControlFlow<()> {
        self.expression_depth += 1;
        if self.expression_depth > 64 {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }
    fn post_visit_expr(&mut self, _: &Expr) -> ControlFlow<()> {
        self.expression_depth -= 1;
        ControlFlow::Continue(())
    }
    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<()> {
        let mut scope = self.scopes.last().cloned().unwrap_or_default();
        add_ctes(query, &mut scope.definitions);
        self.scopes.push(scope);
        ControlFlow::Continue(())
    }
    fn post_visit_query(&mut self, _: &Query) -> ControlFlow<()> {
        self.scopes.pop();
        ControlFlow::Continue(())
    }
    fn pre_visit_select(&mut self, select: &sqlparser::ast::Select) -> ControlFlow<()> {
        let mut scope = self.scopes.last().cloned().unwrap_or_default();
        for table in &select.from {
            add_visible_relation(&table.relation, &scope.definitions, &mut scope.visible);
            for join in &table.joins {
                add_visible_relation(&join.relation, &scope.definitions, &mut scope.visible);
            }
        }
        if let Some(current) = self.scopes.last_mut() {
            *current = scope;
        }
        ControlFlow::Continue(())
    }
    fn pre_visit_relation(&mut self, relation: &sqlparser::ast::ObjectName) -> ControlFlow<()> {
        if relation
            .0
            .iter()
            .any(|part| part.as_ident().is_some_and(|ident| ident.value == CURSOR))
        {
            self.relation_cursor = true;
            if let Some(scope) = self.scopes.last() {
                let qualifier = self.qualifier.as_ref().map(|q| format!("{q}."));
                self.result = scope
                    .definitions
                    .keys()
                    .filter_map(|name| {
                        let suffix = match &qualifier {
                            Some(qualifier) => name.strip_prefix(qualifier)?,
                            None => name.as_str(),
                        };
                        suffix
                            .trim_matches('"')
                            .starts_with(self.prefix)
                            .then(|| suffix.to_owned())
                    })
                    .collect();
            }
        }
        ControlFlow::Continue(())
    }
    fn pre_visit_ident(&mut self, ident: &Ident) -> ControlFlow<()> {
        if ident.value == CURSOR && !self.relation_cursor {
            let scope = self
                .scopes
                .last()
                .map(|s| s.visible.clone())
                .unwrap_or_default();
            let candidates: Vec<_> = if let Some(qualifier) = &self.qualifier {
                scope
                    .get(qualifier)
                    .into_iter()
                    .flatten()
                    .cloned()
                    .collect()
            } else {
                scope
                    .values()
                    .flatten()
                    .chain(scope.keys())
                    .cloned()
                    .chain(self.functions.iter().map(|name| (*name).to_owned()))
                    .collect()
            };
            self.result = candidates
                .into_iter()
                .filter(|s| s.trim_matches('"').starts_with(self.prefix))
                .collect();
        }
        ControlFlow::Continue(())
    }
}
fn add_ctes(query: &Query, scope: &mut Names) {
    if let Some(with) = &query.with {
        for cte in &with.cte_tables {
            let columns = if cte.alias.columns.is_empty() {
                outputs(&cte.query, scope)
            } else {
                cte.alias
                    .columns
                    .iter()
                    .map(|column| ident_name(&column.name))
                    .collect()
            };
            scope.insert(ident_name(&cte.alias.name), columns);
        }
    }
}
fn add_relation(table: &TableFactor, scope: &mut Names) {
    match table {
        TableFactor::Derived {
            subquery,
            alias: Some(alias),
            ..
        } => {
            let columns = if alias.columns.is_empty() {
                outputs(subquery, scope)
            } else {
                alias
                    .columns
                    .iter()
                    .map(|column| ident_name(&column.name))
                    .collect()
            };
            scope.insert(ident_name(&alias.name), columns);
        }
        TableFactor::Table {
            name,
            alias: Some(alias),
            ..
        } => {
            let columns = scope.get(&object_name(name)).cloned().unwrap_or_default();
            scope.insert(ident_name(&alias.name), columns);
        }
        _ => {}
    }
}
fn outputs(query: &Query, inherited: &Names) -> Vec<String> {
    let mut scope = inherited.clone();
    add_ctes(query, &mut scope);
    let SetExpr::Select(select) = &*query.body else {
        return Vec::new();
    };
    for table in &select.from {
        add_relation(&table.relation, &mut scope);
    }
    select
        .projection
        .iter()
        .flat_map(|item| match item {
            SelectItem::ExprWithAlias { alias, .. } => vec![ident_name(alias)],
            SelectItem::UnnamedExpr(Expr::Identifier(ident)) => vec![ident_name(ident)],
            SelectItem::UnnamedExpr(Expr::CompoundIdentifier(parts)) => {
                parts.last().map(ident_name).into_iter().collect()
            }
            SelectItem::Wildcard(options) => select
                .from
                .iter()
                .flat_map(|t| match &t.relation {
                    TableFactor::Table { name, alias, .. } => scope
                        .get(
                            &alias
                                .as_ref()
                                .map_or_else(|| object_name(name), |a| ident_name(&a.name)),
                        )
                        .cloned()
                        .unwrap_or_default(),
                    TableFactor::Derived {
                        alias: Some(alias), ..
                    } => scope
                        .get(&ident_name(&alias.name))
                        .cloned()
                        .unwrap_or_default(),
                    _ => Vec::new(),
                })
                .filter(|column| {
                    !excluded(
                        options.opt_exclude.as_ref().or(select.exclude.as_ref()),
                        column,
                    )
                })
                .collect(),
            _ => Vec::new(),
        })
        .collect()
}

fn excluded(exclude: Option<&sqlparser::ast::ExcludeSelectItem>, column: &str) -> bool {
    use sqlparser::ast::ExcludeSelectItem;
    match exclude {
        Some(ExcludeSelectItem::Single(name)) => object_name(name) == column,
        Some(ExcludeSelectItem::Multiple(names)) => {
            names.iter().any(|name| object_name(name) == column)
        }
        None => false,
    }
}

pub(crate) fn format(
    dialect: &dyn sqlparser::dialect::Dialect,
    sql: &str,
) -> Result<String, String> {
    let before = significant_tokens(dialect, sql)?;
    // sqlformat does not tokenize PostgreSQL dollar strings; protect their exact source bytes.
    use sqlparser::tokenizer::{Token, Tokenizer};
    let mut masked = sql.to_owned();
    let mut replacements = Vec::new();
    let tokens = Tokenizer::new(dialect, sql)
        .tokenize_with_location()
        .map_err(|e| e.to_string())?;
    for (index, token) in tokens.iter().enumerate().rev() {
        if matches!(token.token, Token::DollarQuotedString(_)) {
            let start = source_offset(
                sql,
                token.span.start.line as usize,
                token.span.start.column as usize,
            )
            .ok_or("invalid token start")?;
            let end = source_offset(
                sql,
                token.span.end.line as usize,
                token.span.end.column as usize,
            )
            .ok_or("invalid token end")?;
            let placeholder = format!("'__lenz_dollar_{index}__'");
            if sql.contains(&placeholder) {
                return Err("reserved formatter placeholder in source".into());
            }
            replacements.push((
                placeholder.clone(),
                sql.get(start..end)
                    .ok_or("invalid literal range")?
                    .to_owned(),
            ));
            masked.replace_range(start..end, &placeholder);
        }
    }
    let mut formatted = sqlformat::format(
        &masked,
        &sqlformat::QueryParams::None,
        &sqlformat::FormatOptions {
            dialect: sqlformat::Dialect::PostgreSql,
            ..Default::default()
        },
    );
    for (placeholder, literal) in replacements {
        formatted = formatted.replace(&placeholder, &literal);
    }
    if significant_tokens(dialect, &formatted)? != before {
        return Err("formatter changed SQL tokens; original text retained".into());
    }
    Ok(formatted)
}
pub(crate) fn significant_tokens(
    dialect: &dyn sqlparser::dialect::Dialect,
    sql: &str,
) -> Result<Vec<sqlparser::tokenizer::Token>, String> {
    use sqlparser::tokenizer::{Token, Tokenizer, Whitespace};
    Ok(Tokenizer::new(dialect, sql)
        .tokenize()
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|token| {
            !matches!(
                token,
                Token::Whitespace(Whitespace::Space | Whitespace::Newline | Whitespace::Tab)
            )
        })
        .collect())
}

fn source_offset(sql: &str, line: usize, column: usize) -> Option<usize> {
    let mut current_line = 1;
    let mut current_column = 1;
    for (offset, ch) in sql.char_indices() {
        if (current_line, current_column) == (line, column) {
            return Some(offset);
        }
        if ch == '\n' {
            current_line += 1;
            current_column = 1;
        } else {
            current_column += 1;
        }
    }
    ((current_line, current_column) == (line, column)).then_some(sql.len())
}

fn ident_name(ident: &Ident) -> String {
    if ident.quote_style.is_some() {
        ident.to_string()
    } else {
        ident.value.to_ascii_lowercase()
    }
}
fn object_name(name: &sqlparser::ast::ObjectName) -> String {
    name.0
        .iter()
        .map(|part| {
            part.as_ident()
                .map(ident_name)
                .unwrap_or_else(|| part.to_string())
        })
        .collect::<Vec<_>>()
        .join(".")
}
fn qualifier_name(dialect: &dyn sqlparser::dialect::Dialect, head: &str) -> Result<String, String> {
    use sqlparser::tokenizer::{Token, Tokenizer};
    let tokens = Tokenizer::new(dialect, head)
        .tokenize()
        .map_err(|e| e.to_string())?;
    let mut parts = Vec::new();
    let mut word = true;
    for token in tokens.iter().rev() {
        match (word, token) {
            (true, Token::Word(value)) => {
                parts.push(ident_name(&Ident {
                    value: value.value.clone(),
                    quote_style: value.quote_style,
                    span: sqlparser::tokenizer::Span::empty(),
                }));
                word = false;
            }
            (false, Token::Period) => word = true,
            _ => break,
        }
    }
    parts.reverse();
    Ok(parts.join("."))
}

fn add_visible_relation(table: &TableFactor, definitions: &Names, visible: &mut Names) {
    let mut resolved = definitions.clone();
    add_relation(table, &mut resolved);
    let names = match table {
        TableFactor::Table {
            alias: Some(alias), ..
        }
        | TableFactor::Derived {
            alias: Some(alias), ..
        } => vec![ident_name(&alias.name)],
        TableFactor::Table { name, .. } => {
            let full = object_name(name);
            let short = name
                .0
                .last()
                .and_then(|part| part.as_ident())
                .map(ident_name);
            let columns = definitions.get(&full).cloned().unwrap_or_default();
            visible.insert(full, columns.clone());
            if let Some(short) = short {
                visible.insert(short, columns);
            }
            return;
        }
        _ => return,
    };
    for name in names {
        visible.insert(name.clone(), resolved.remove(&name).unwrap_or_default());
    }
}

/// Innermost call and zero-based argument index, including incomplete array arguments.
pub(crate) fn call_at(
    dialect: &dyn sqlparser::dialect::Dialect,
    sql: &str,
    cursor: usize,
) -> Result<Option<(String, usize)>, String> {
    use sqlparser::tokenizer::Token;
    if !sql.is_char_boundary(cursor) {
        return Err("invalid signature offset".into());
    }
    let tokens = significant_tokens(
        dialect,
        sql.get(..cursor).ok_or("invalid signature offset")?,
    )?;
    let mut calls: Vec<Option<(String, usize)>> = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::LParen => {
                let name = match tokens.get(index.wrapping_sub(1)) {
                    Some(Token::Word(word)) => {
                        let mut name = word.to_string();
                        if matches!(tokens.get(index.wrapping_sub(2)), Some(Token::Period))
                            && let Some(Token::Word(schema)) = tokens.get(index.wrapping_sub(3))
                        {
                            name = format!("{schema}.{name}");
                        }
                        name
                    }
                    _ => String::new(),
                };
                calls.push(Some((name, 0)));
            }
            Token::LBracket | Token::LBrace => calls.push(None),
            Token::RParen | Token::RBracket | Token::RBrace => {
                calls.pop();
            }
            Token::Comma => {
                if let Some(Some((_, active_parameter))) = calls.last_mut() {
                    *active_parameter += 1;
                }
            }
            _ => {}
        }
    }
    Ok(calls.pop().flatten())
}
