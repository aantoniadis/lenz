//! Shared SQL scope resolution and guarded source formatting, with dialect supplied by each route.
use sqlparser::{
    ast::{Expr, Ident, Query, SelectItem, SetExpr, TableFactor, Visit, Visitor},
    parser::Parser,
};
use std::{collections::BTreeMap, ops::ControlFlow};

const CURSOR: &str = "__lenz_completion_cursor__";
#[derive(Clone, Default)]
struct Names { values: BTreeMap<String, Vec<String>>, folded: bool }
impl Names {
    fn empty(folded: bool) -> Self { Self { values: BTreeMap::new(), folded } }
    fn get(&self, name: &str) -> Option<&Vec<String>> {
        if !self.folded { return self.values.get(name); }
        self.values.iter().find(|(key, _)| names_equal(key, name, true)).map(|(_, value)| value)
    }
    fn relation(&self, name: &str) -> Option<&Vec<String>> {
        self.get(name).or_else(|| {
            // Only table resolution uses DuckDB's default schema; visible aliases stay exact.
            (self.folded && name_parts(name).len() == 1).then(|| self.get(&format!("\"main\".{name}"))).flatten()
        })
    }
    fn insert(&mut self, name: String, columns: Vec<String>) {
        if self.folded && let Some(old) = self.values.keys().find(|key| names_equal(key, &name, true)).cloned() { self.values.remove(&old); }
        self.values.insert(name, columns);
    }
    fn remove(&mut self, name: &str) -> Option<Vec<String>> {
        if !self.folded { return self.values.remove(name); }
        let key = self.values.keys().find(|key| names_equal(key, name, self.folded)).cloned()?;
        self.values.remove(&key)
    }
    fn keys(&self) -> impl Iterator<Item = &String> { self.values.keys() }
    fn iter(&self) -> impl Iterator<Item = (&String, &Vec<String>)> { self.values.iter() }
}
fn name_parts(name: &str) -> Vec<String> {
    use sqlparser::tokenizer::{Token, Tokenizer};
    Tokenizer::new(&sqlparser::dialect::DuckDbDialect {}, name).tokenize().unwrap_or_default().into_iter()
        .filter_map(|t| match t { Token::Word(w) => Some(w.value.to_ascii_lowercase()), _ => None }).collect()
}
fn names_equal(a: &str, b: &str, folded: bool) -> bool { if folded { name_parts(a) == name_parts(b) } else { a == b } }
fn prefix_matches(name: &str, prefix: &str, folded: bool) -> bool {
    if folded { name.trim_matches('"').to_ascii_lowercase().starts_with(&prefix.to_ascii_lowercase()) }
    else { name.trim_matches('"').starts_with(prefix) }
}
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
    let folded = dialect.is::<sqlparser::dialect::DuckDbDialect>();
    let range = completion_range(sql, cursor)?;
    let quoted = sql.as_bytes().get(range.start) == Some(&b'"');
    let typed = sql.get(range.start + usize::from(quoted)..cursor).ok_or("invalid completion prefix")?;
    let prefix = if quoted { typed.replace("\"\"", "\"") } else { typed.to_ascii_lowercase() };
    let head = sql.get(..range.start).ok_or("invalid completion boundary")?;
    let qualifier = head.trim_end().strip_suffix('.').map(|head| qualifier_name(dialect, head)).transpose()?;
    let tail = sql.get(range.end..).ok_or("invalid completion offset")?;
    let repaired = format!("{head}{CURSOR}{tail}");
    let all_tokens = significant_tokens(dialect, &repaired)?;
    if all_tokens.len() > 4096 { return Err("completion source exceeds 4096 tokens".into()); }
    let repaired = statement_at(dialect, &repaired, head.len())?.to_owned();
    let tokens = significant_tokens(dialect, &repaired)?;
    use sqlparser::tokenizer::Token;
    let mut significant = tokens.iter().filter(|token| !matches!(token, Token::Whitespace(_)));
    // An empty statement or its first partial word has no query-local scope yet.
    if matches!(significant.next(), Some(Token::Word(word)) if word.value == CURSOR) && significant.next().is_none() {
        return Ok(Vec::new());
    }
    let mut depth = 0usize;
    for token in &tokens {
        use sqlparser::tokenizer::Token;
        match token { Token::LParen | Token::LBracket => depth += 1, Token::RParen | Token::RBracket => depth = depth.saturating_sub(1), _ => {} }
        if depth > 64 { return Err("completion expression depth exceeds 64".into()); }
    }
    if !folded && qualifier.as_deref() == Some("pg_catalog") {
        return Ok(functions.iter().filter(|name| name.starts_with(&prefix)).map(|name| (*name).into()).collect());
    }
    let statements = Parser::parse_sql(dialect, &repaired).or_else(|_| {
        // Complete missing expression tails and delimiters while the user is still typing.
        let mut repair = repaired.clone();
        use sqlparser::tokenizer::Token;
        if tokens.last().is_some_and(|t| matches!(t, Token::Comma | Token::LParen | Token::Eq | Token::Plus | Token::Minus) || matches!(t, Token::Word(w) if ["WHERE", "AND", "OR", "ON", "HAVING"].contains(&w.value.to_ascii_uppercase().as_str()))) { repair.push_str(" NULL"); }
        for _ in 0..depth { repair.push(')'); }
        Parser::parse_sql(dialect, &repair)
    }).map_err(|e| e.to_string())?;
    let mut visitor = CompletionVisitor {
        functions,
        relation_cursor: false,
        expression_depth: 0,
        derived_visibility: Vec::new(),
        scopes: vec![Scope {
            definitions: Names { values: catalog.clone(), folded },
            visible: Names::empty(folded),
        }],
        qualifier,
        prefix: &prefix,
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
    derived_visibility: Vec<Option<Names>>,
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
    fn pre_visit_table_factor(&mut self, table: &TableFactor) -> ControlFlow<()> {
        let previous = if matches!(table, TableFactor::Derived { lateral: false, .. }) {
            self.scopes.last_mut().map(|scope| { let folded = scope.visible.folded; std::mem::replace(&mut scope.visible, Names::empty(folded)) })
        } else { None };
        self.derived_visibility.push(previous);
        ControlFlow::Continue(())
    }
    fn post_visit_table_factor(&mut self, _: &TableFactor) -> ControlFlow<()> {
        if let Some(Some(previous)) = self.derived_visibility.pop() && let Some(scope) = self.scopes.last_mut() { scope.visible = previous; }
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
                let qualifier = self.qualifier.as_ref();
                self.result = scope
                    .definitions
                    .keys()
                    .flat_map(|name| {
                        let short = (scope.definitions.folded && qualifier.is_none()).then(|| name.strip_prefix("\"main\".")).flatten();
                        std::iter::once(name.as_str()).chain(short)
                    })
                    .filter_map(|name| {
                        let suffix = match qualifier {
                            Some(qualifier) => qualified_suffix(name, qualifier, scope.definitions.folded)?,
                            None => name.to_owned(),
                        };
                        prefix_matches(&suffix, self.prefix, scope.definitions.folded).then_some(suffix)
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
                    .chain(self.functions.iter().filter_map(|name| qualified_suffix(name, qualifier, scope.folded)))
                    .collect()
            } else {
                let mut counts: BTreeMap<String, usize> = BTreeMap::new();
                for (name, columns) in scope.iter() {
                    // A schema-qualified relation and its short name denote the same source.
                    if name.rsplit_once('.').is_some_and(|(_, short)| scope.get(short) == Some(columns)) { continue; }
                    for column in columns { *counts.entry(column.clone()).or_default() += 1; }
                }
                counts.into_iter().filter_map(|(name, count)| (count == 1).then_some(name))
                    .chain(scope.keys().cloned()).chain(self.functions.iter().map(|name| (*name).to_owned())).collect()
            };
            self.result = candidates
                .into_iter()
                .filter(|s| prefix_matches(s, self.prefix, scope.folded))
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
            args,
            ..
        } => {
            let columns = if !alias.columns.is_empty() { alias.columns.iter().map(|c| ident_name(&c.name)).collect() }
                else if args.is_some() { function_columns(&object_name(name), scope.folded) }
                else { scope.relation(&object_name(name)).cloned().unwrap_or_default() };
            scope.insert(ident_name(&alias.name), columns);
        }
        TableFactor::UNNEST { alias, .. } => {
            let columns = alias.as_ref().filter(|a| !a.columns.is_empty()).map_or_else(|| vec!["unnest".into()], |a| a.columns.iter().map(|c| ident_name(&c.name)).collect());
            scope.insert(alias.as_ref().map_or_else(|| "unnest".into(), |a| ident_name(&a.name)), columns);
        }
        _ => {}
    }
}
fn function_columns(name: &str, folded: bool) -> Vec<String> {
    match name.rsplit('.').next().unwrap_or(name) {
        "range" if folded => vec!["range".into()],
        "generate_series" | "unnest" | "jsonb_array_elements" | "json_array_elements" => vec![name.rsplit('.').next().unwrap_or(name).into()],
        "jsonb_each" | "json_each" | "jsonb_each_text" | "json_each_text" => vec!["key".into(), "value".into()],
        _ => Vec::new(),
    }
}
fn outputs(query: &Query, inherited: &Names) -> Vec<String> {
    let mut scope = inherited.clone();
    add_ctes(query, &mut scope);
    let select = match &*query.body {
        SetExpr::Select(select) => select,
        SetExpr::Query(query) => return outputs(query, &scope),
        SetExpr::SetOperation { left, .. } => return set_outputs(left, &scope),
        _ => return Vec::new(),
    };
    select_outputs(select, &scope)
}
fn select_outputs(select: &sqlparser::ast::Select, inherited: &Names) -> Vec<String> {
    let mut scope = inherited.clone();
    for table in &select.from {
        add_relation(&table.relation, &mut scope);
        for join in &table.joins { add_relation(&join.relation, &mut scope); }
    }
    let relation = |table: &TableFactor| -> Vec<String> { match table {
        TableFactor::Table { name, alias, args, .. } => alias.as_ref().map(|a| ident_name(&a.name)).and_then(|name| scope.get(&name).cloned())
            .or_else(|| args.is_none().then(|| scope.relation(&object_name(name)).cloned()).flatten()).unwrap_or_else(|| if args.is_some() { function_columns(&object_name(name), scope.folded) } else { Vec::new() }),
        TableFactor::Derived { alias: Some(alias), .. } => scope.get(&ident_name(&alias.name)).cloned().unwrap_or_default(),
        TableFactor::UNNEST { alias, .. } => scope.get(&alias.as_ref().map_or_else(|| "unnest".into(), |a| ident_name(&a.name))).cloned().unwrap_or_default(),
        _ => Vec::new(),
    }};
    select.projection.iter().flat_map(|item| match item {
        SelectItem::ExprWithAlias { alias, .. } => vec![ident_name(alias)],
        SelectItem::UnnamedExpr(Expr::Identifier(ident)) => vec![ident_name(ident)],
        SelectItem::UnnamedExpr(Expr::CompoundIdentifier(parts)) => parts.last().map(ident_name).into_iter().collect(),
        SelectItem::UnnamedExpr(Expr::Function(function)) => function.name.0.last().and_then(|part| part.as_ident()).map(ident_name).into_iter().collect(),
        SelectItem::Wildcard(options) => {
            let columns = select.from.iter().flat_map(|table| std::iter::once(&table.relation).chain(table.joins.iter().map(|j| &j.relation))).flat_map(&relation).collect();
            wildcard(columns, options, select.exclude.as_ref(), scope.folded)
        }
        SelectItem::QualifiedWildcard(sqlparser::ast::SelectItemQualifiedWildcardKind::ObjectName(name), options) => wildcard(scope.get(&object_name(name)).cloned().unwrap_or_default(), options, select.exclude.as_ref(), scope.folded),
        _ => Vec::new(),
    }).collect()
}
fn wildcard(columns: Vec<String>, options: &sqlparser::ast::WildcardAdditionalOptions, exclude: Option<&sqlparser::ast::ExcludeSelectItem>, folded: bool) -> Vec<String> {
    let mut columns: Vec<_> = columns.into_iter().filter(|column| !excluded(options.opt_exclude.as_ref().or(exclude), column, folded)).collect();
    if let Some(rename) = &options.opt_rename {
        use sqlparser::ast::RenameSelectItem;
        let names = match rename { RenameSelectItem::Single(name) => std::slice::from_ref(name), RenameSelectItem::Multiple(names) => names };
        for column in &mut columns {
            if let Some(rename) = names.iter().find(|r| names_equal(&ident_name(&r.ident), column, folded)) { *column = ident_name(&rename.alias); }
        }
    }
    // REPLACE changes values, retaining the target column names; no expression or function is evaluated.
    columns
}
fn set_outputs(body: &SetExpr, inherited: &Names) -> Vec<String> {
    match body {
        SetExpr::Query(query) => outputs(query, inherited),
        SetExpr::SetOperation { left, .. } => set_outputs(left, inherited),
        SetExpr::Select(select) => select_outputs(select, inherited),
        _ => Vec::new(),
    }
}

fn excluded(exclude: Option<&sqlparser::ast::ExcludeSelectItem>, column: &str, folded: bool) -> bool {
    use sqlparser::ast::ExcludeSelectItem;
    match exclude {
        Some(ExcludeSelectItem::Single(name)) => names_equal(&object_name(name), column, folded),
        Some(ExcludeSelectItem::Multiple(names)) => {
            names.iter().any(|name| names_equal(&object_name(name), column, folded))
        }
        None => false,
    }
}

pub(crate) fn format_with_indent(dialect: &dyn sqlparser::dialect::Dialect, sql: &str, indent: sqlformat::Indent) -> Result<String, String> {
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
            indent,
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
        TableFactor::UNNEST { alias, .. } => vec![alias.as_ref().map_or_else(|| "unnest".into(), |a| ident_name(&a.name))],
        TableFactor::Table { name, args, .. } => {
            let full = object_name(name);
            let short = name
                .0
                .last()
                .and_then(|part| part.as_ident())
                .map(ident_name);
            let columns = if args.is_some() { function_columns(&full, definitions.folded) } else { definitions.relation(&full).cloned().unwrap_or_default() };
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
    Ok(calls.into_iter().rev().find_map(|call| call.filter(|(name, _)| !name.is_empty())))
}


/// Original-source name range at the caret, refusing string/comment positions. Quoted identifiers
/// include their quotes so a completion cannot create doubled delimiters or corrupt the SQL tail.
pub(crate) fn completion_range(sql: &str, cursor: usize) -> Result<std::ops::Range<usize>, String> {
    if !sql.is_char_boundary(cursor) || sql.len() > 262144 { return Err("invalid or oversized SQL completion source".into()); }
    let bytes = sql.as_bytes();
    let mut i = 0;
    let mut quoted = None;
    while i < cursor {
        match bytes.get(i).copied() {
            Some(b'\'') => {
                i += 1;
                loop {
                    if i >= cursor { return Err("completion is inside a SQL string".into()); }
                    match bytes.get(i) {
                        Some(b'\'') if bytes.get(i + 1) == Some(&b'\'') => i += 2,
                        Some(b'\'') => { i += 1; break; }
                        Some(b'\\') => i += 2,
                        Some(_) => i += 1,
                        None => return Err("unfinished SQL string".into()),
                    }
                }
            }
            Some(b'-') if bytes.get(i + 1) == Some(&b'-') => {
                i += 2;
                while i < cursor && bytes.get(i) != Some(&b'\n') { i += 1; }
                if i == cursor { return Err("completion is inside a SQL comment".into()); }
            }
            Some(b'/') if bytes.get(i + 1) == Some(&b'*') => {
                let mut nesting = 1usize; i += 2;
                while nesting > 0 {
                    if i >= cursor { return Err("completion is inside a SQL comment".into()); }
                    if bytes.get(i..i + 2) == Some(b"/*") { nesting += 1; i += 2; }
                    else if bytes.get(i..i + 2) == Some(b"*/") { nesting -= 1; i += 2; }
                    else { i += 1; }
                }
            }
            Some(b'"') => {
                let start = i; i += 1;
                while i < cursor {
                    if bytes.get(i) == Some(&b'"') {
                        if bytes.get(i + 1) == Some(&b'"') { i += 2; }
                        else { i += 1; break; }
                    } else { i += 1; }
                }
                if i == cursor && (bytes.get(i.saturating_sub(1)) != Some(&b'"') || i == start + 1) { quoted = Some(start); }
            }
            Some(b'$') => {
                let rest = sql.get(i..).unwrap_or_default();
                let tag_end = rest.get(1..).and_then(|r| r.find('$')).map(|n| n + 1);
                if let Some(end) = tag_end.filter(|&end| rest.get(1..end).is_some_and(|t| t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))) {
                    let tag = rest.get(..=end).unwrap_or_default();
                    let after = i + tag.len();
                    let close = sql.get(after..).and_then(|r| r.find(tag)).map(|n| after + n + tag.len());
                    if close.is_none_or(|end| end > cursor) { return Err("completion is inside a dollar string".into()); }
                    i = close.unwrap_or(cursor);
                } else { i += 1; }
            }
            Some(_) => i += 1,
            None => return Err("invalid SQL completion offset".into()),
        }
    }
    let name = |c: char| c.is_alphanumeric() || matches!(c, '_' | '$');
    let start = quoted.unwrap_or_else(|| sql.get(..cursor).unwrap_or_default().char_indices().rev().find(|(_, c)| !name(*c)).map_or(0, |(i,c)| i + c.len_utf8()));
    let mut end = cursor;
    for c in sql.get(cursor..).unwrap_or_default().chars() { if !name(c) { break; } end += c.len_utf8(); }
    if quoted.is_some() && bytes.get(end) == Some(&b'"') { end += 1; }
    Ok(start..end)
}


/// Only the statement at the caret supplies scope. A different statement may still be malformed.
fn statement_at<'a>(dialect: &dyn sqlparser::dialect::Dialect, sql: &'a str, cursor: usize) -> Result<&'a str, String> {
    use sqlparser::tokenizer::{Token, Tokenizer};
    let tokens = Tokenizer::new(dialect, sql).tokenize_with_location().map_err(|e| e.to_string())?;
    let mut semicolons = tokens.iter().filter(|t| t.token == Token::SemiColon).map(|t| (t.span.start.line, t.span.start.column)).peekable();
    let (mut line, mut column, mut start, mut end) = (1u64, 1u64, 0, sql.len());
    for (at, c) in sql.char_indices() {
        if semicolons.peek().copied() == Some((line, column)) {
            semicolons.next();
            if at < cursor { start = at + 1; } else { end = at; break; }
        }
        if c == '\n' { line += 1; column = 1; } else { column += 1; }
    }
    sql.get(start..end).ok_or_else(|| "invalid statement boundary".into())
}

fn qualified_suffix(name: &str, qualifier: &str, folded: bool) -> Option<String> {
    if !folded { return name.strip_prefix(&format!("{qualifier}.")).map(str::to_owned); }
    let parts = name_parts(name); let prefix = name_parts(qualifier);
    if !parts.starts_with(&prefix) || parts.len() <= prefix.len() { return None; }
    use sqlparser::tokenizer::{Token, Tokenizer};
    let words: Vec<_> = Tokenizer::new(&sqlparser::dialect::DuckDbDialect {}, name).tokenize().ok()?.into_iter().filter_map(|t| match t { Token::Word(w) => Some(w.to_string()), _ => None }).collect();
    Some(words.into_iter().skip(prefix.len()).collect::<Vec<_>>().join("."))
}
