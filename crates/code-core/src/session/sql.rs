//! Offline SQL requests use the shared admitted-version analysis scheduler. No database is opened.
use super::*;
use crate::sql_dialect::{Dialect, Identity};
use crate::semantics::diagnostics::{Diagnostic, FileDiagnostics, Severity};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
pub enum Operation { Check, Complete(CompletionTrigger), Signature(bool), Format(u64) }
impl Operation { fn lane(self) -> usize { match self { Self::Check => 0, Self::Complete(_) => 1, Self::Signature(_) => 2, Self::Format(_) => 3 } } }
#[derive(Debug)]
pub enum Output {
    Problems(Arc<FileDiagnostics>),
    Names { range: std::ops::Range<usize>, names: Vec<String> },
    Signatures(Vec<super::signature::Signature>),
    Formatted(format_work::ToolResult),
    Failed(String),
}
#[derive(Debug)]
pub struct Reply { pub identity: Identity, pub tab: TabId, pub request: u64, pub caret: usize, pub operation: Operation, pub output: Output }
type Context = (Identity, u64, usize);
#[derive(Default)]
pub(super) struct Work {
    checked: Option<Context>,
    pending: [Option<(Context, u64)>; 4],
    pub(super) executable: Option<PathBuf>,
    pub(super) native_checked: bool,
    pub(super) metadata: Metadata,
}
#[derive(Default)]
pub(super) struct Metadata {
    pub(super) scope: Option<(Dialect, u64, PathBuf)>,
    pub(super) source: String,
    pub(super) seq: Arc<AtomicU64>,
    pub(super) pending: Option<u64>,
    pub(super) catalog: Option<(std::time::Instant, Arc<crate::sql_metadata::Catalog>)>,
    failure: Option<String>,
    file_checked: Option<std::time::Instant>,
}
impl Drop for Metadata { fn drop(&mut self) { self.seq.fetch_add(1, Ordering::Release); } }
const CATALOG_AGE: std::time::Duration = std::time::Duration::from_secs(300);

#[derive(Default)]
pub(super) struct View { pub problems: HashMap<crate::text::BufferId, (Identity, Arc<FileDiagnostics>)>, pub notes: HashMap<crate::text::BufferId, (Identity, String)> }

impl Session {
    fn sql_context(&self) -> Option<Context> {
        Some((self.sql_identity()?, self.project_generation.load(Ordering::Acquire), Arc::as_ptr(self.state.current_source()?.source_meta()) as usize))
    }
    pub(super) fn sql_action_disabled(&self, id: CommandId) -> Option<&'static str> {
        match self.state.sql_dialect().and_then(|s| s.dialect) {
            None => Some("Choose an SQL dialect first (SQL Dialect…)"),
            Some(Dialect::PostgreSql) => None,
            Some(Dialect::DuckDb) if id == CommandId::Completion => None,
            Some(Dialect::DuckDb) if self.sql_work.executable.is_none() => Some("Install DuckDB on PATH or configure duckdb_executable (SQL Dialect…)"),
            Some(Dialect::DuckDb) => None,
        }
    }

    pub(super) fn sql_cancel(&mut self, lane: usize) {
        if let Some((identity, _)) = self.sql_work.pending.get_mut(lane).and_then(Option::take) {
            self.analysis.cancel_consumer(identity.0.key.buffer, crate::analysis::Consumer::Sql(lane as u8));
        }
    }
    pub(super) fn sql_context_changed(&mut self) {
        for lane in 0..4 { self.sql_cancel(lane); }
        self.sql_work.checked = None;
        self.state.sql_view.problems.clear(); self.state.sql_view.notes.clear();
        self.close_completion(); self.close_signature(); self.cancel_format();
    }

    pub(super) fn sync_sql(&mut self) {
        self.sync_sql_metadata();
        let identity = self.sql_identity();
        let context = self.sql_context();
        if identity.is_some_and(|i| i.dialect == Dialect::DuckDb) && !self.sql_work.native_checked {
            self.sql_work.executable = self.state.settings.duckdb_executable.clone().or_else(|| semantics::provider::find_executable(Path::new("duckdb"), std::env::var_os("PATH").as_deref())).filter(|p| p.is_file());
            self.sql_work.native_checked = true;
        }
        if context == self.sql_work.checked { return; }
        for lane in 1..4 {
            if self.sql_work.pending[lane].is_some_and(|(old, _)| Some(old) != context) {
                self.sql_cancel(lane);
                if lane == 3 { self.cancel_format(); }
            }
        }
        self.sql_work.checked = context;
        self.state.sql_view.problems.retain(|buffer, _| self.state.sources.buffers.contains_key(buffer));
        self.state.sql_view.notes.retain(|buffer, _| self.state.sources.buffers.contains_key(buffer));
        self.sql_cancel(0);
        if identity.is_some() { self.sql_request(Operation::Check); }
    }

    pub(super) fn sql_request(&mut self, operation: Operation) -> Effect {
        let repaint = Effect { repaint: true, ..Effect::default() };
        let Some(context) = self.sql_context() else { return repaint };
        let identity = context.0;
        let Some(tab) = self.state.active else { return repaint };
        let Some(snapshot) = self.state.view(tab).map(|v| v.snapshot().clone()) else { return repaint };

        let lane = operation.lane();
        self.sql_cancel(lane);
        self.semantic_seq += 1;
        let request = self.semantic_seq;
        let caret = self.state.selection().map_or(0, |s| s.head);
        let loaded = self.state.sources.buffers.get(&identity.key.buffer).map(|e| e.document.clone()).unwrap_or_default();
        let input = crate::analysis::Input::new(snapshot, loaded, self.project_generation.load(Ordering::Acquire), true);
        self.sql_work.pending[lane] = Some((context, request));
        let executable = self.sql_work.executable.clone();
        let catalog = self.sql_catalog();
        let indent = if lane == 3 {
            let unit = self.indent_unit();
            if unit.style == crate::preferences::Style::Tab { sqlformat::Indent::Tabs } else { sqlformat::Indent::Spaces(u8::try_from(unit.width).unwrap_or(16)) }
        } else { sqlformat::Indent::Spaces(2) };
        self.analysis.request(input, crate::analysis::Consumer::Sql(lane as u8), request,
            if lane == 0 { Priority::Background } else { Priority::Visible }, move |doc, lease| {
                if !lease.wanted() { return; }
                let native = executable.map(|path| crate::duckdb_assistance::DuckDb::new(path).with_cancellation(lease.cancellation()));
                let output = if identity.dialect == Dialect::DuckDb { evaluate_duck(&doc, caret, operation, indent, native.as_ref(), catalog.as_deref()) } else { evaluate(&doc, caret, operation, indent, catalog.as_deref()) };
                if lease.wanted() { lease.send(Completion::Sql(Box::new(Reply { identity, tab, request, caret, operation, output }))); }
            });
        repaint
    }

    pub(super) fn sql_reply(&mut self, reply: Reply) -> Effect {
        if self.sql_work.metadata.catalog.as_ref().is_some_and(|(_,c)| !c.file_current()) {
            self.disconnect_sql_metadata(); self.sql_work.metadata.failure = Some("metadata file changed; reconnect required".into()); self.state.messages.record("SQL metadata warning: file changed; reconnect required");
        }
        let lane = reply.operation.lane();
        if self.sql_work.pending[lane].is_none_or(|(context, request)| request != reply.request || Some(context) != self.sql_context()) { return Effect::default(); }
        self.sql_work.pending[lane] = None;
        let repaint = Effect { repaint: true, ..Effect::default() };
        let caret_current = self.state.selection().is_some_and(|s| s.anchor == reply.caret && s.head == reply.caret);
        let catalog = self.sql_catalog();
        match reply.output {
            Output::Problems(problems) => { self.state.sql_view.problems.insert(reply.identity.key.buffer, (reply.identity, problems)); }
            Output::Names { range, names } if caret_current => {
                let Some(snapshot) = self.state.view(reply.tab).map(|v| v.snapshot().clone()) else { return repaint };
                let items = names.into_iter().map(|name| {
                    let mut item = Suggestion::local(name.clone(), Some(format!("{} offline", reply.identity.dialect.label())), &name, range.clone(), &name, false);
                    if reply.identity.dialect == Dialect::PostgreSql { item.documentation = crate::postgres_assistance::function_help(&name).map(|h| format!("{}\n{}", h.signature, h.documentation)); }
                    if let Some(docs) = catalog.as_ref().and_then(|c| c.documentation(&name)) { item.documentation = Some(docs); item.detail = Some("Explicit catalog snapshot".into()); }
                    item
                }).collect();
                let trigger = match reply.operation { Operation::Complete(trigger) => trigger, _ => CompletionTrigger::Manual };
                self.state.completion = Suggestions::local(reply.tab, trigger, &snapshot, (range.start, reply.caret), items);
                self.describe_selected();
                if self.state.completion.is_none() && trigger == CompletionTrigger::Manual { self.state.messages.push("Code completion: no names match at this position".into()); }
            }
            Output::Signatures(signatures) if caret_current => {
                let Some(snapshot) = self.state.view(reply.tab).map(|v| v.snapshot().clone()) else { return repaint };
                self.state.signature = super::signature::SignatureHelp::local(reply.tab, SemanticSource::new(&snapshot), reply.caret, signatures);
                if self.state.signature.is_none() && matches!(reply.operation, Operation::Signature(true)) { self.state.messages.push("Parameter info: no offline builtin help at this call".into()); }
            }
            Output::Formatted(result) => if let Operation::Format(request) = reply.operation { return self.project_formatted(request, result); },
            Output::Failed(why) => match reply.operation {
                Operation::Complete(trigger) => return self.completion_unavailable(trigger, why),
                Operation::Signature(manual) => return self.signature_unavailable(manual, why),
                Operation::Format(request) => return self.project_formatted(request, format_work::ToolResult::Failed(why)),
                Operation::Check => { self.state.sql_view.notes.insert(reply.identity.key.buffer, (reply.identity, why)); }
            },
            Output::Names { .. } | Output::Signatures(_) => {}
        }
        repaint
    }
}

fn evaluate(doc: &Arc<Document>, caret: usize, operation: Operation, indent: sqlformat::Indent, catalog: Option<&crate::sql_metadata::Catalog>) -> Output {
    use crate::postgres_assistance as pg;
    let text = doc.text();
    if text.len() > 262144 { return Output::Failed("offline SQL assistance is limited to 256 KiB; the draft remains editable".into()); }
    if let Some(output) = catalog_output(text, caret, operation, catalog, Dialect::PostgreSql) { return output; }
    match operation {
        Operation::Check => {
            let error = pg::diagnose(text).err();
            if let Some(error) = error.as_ref().filter(|e| e.byte_offset.is_none()) { return Output::Failed(error.message.clone()); }
            let items = error.into_iter().map(|error| {
                let start = error.byte_offset.unwrap_or(0).min(text.len());
                let rest = text.get(start..).unwrap_or_default();
                let length = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').map(char::len_utf8).sum::<usize>();
                let end = start + if length > 0 { length } else { rest.chars().next().map_or(0, char::len_utf8) };
                Diagnostic { range: start..end, severity: Severity::Error, message: error.message, source: Some("PostgreSQL syntax (offline)".into()), code: None, raw: serde_json::Value::Null }
            }).collect();
            Output::Problems(Arc::new(FileDiagnostics::new(doc, items)))
        }
        Operation::Complete(_) => match crate::sql_assistance::completion_range(text, caret).and_then(|range| pg::complete(text, caret).map(|names| (range, names))) {
            Ok((range, names)) => Output::Names { range, names }, Err(why) => Output::Failed(why),
        },
        Operation::Signature(_) => match pg::signature_help(text, caret) {
            Ok(help) => {
                let signatures = help.into_iter().filter_map(|h| {
                    let label = h.function.signature;
                    let params = label.split_once('(').and_then(|(_, tail)| tail.split_once(')')).map_or(Vec::new(), |(args, _)| args.split(',').map(|name| serde_json::json!({"label": name.trim()})).collect());
                    super::signature::signature(&serde_json::json!({ "label": label, "documentation": h.function.documentation, "parameters": params, "activeParameter": h.active_parameter }), &serde_json::Value::Null, semantics::lsp::Encoding::Utf8)
                }).collect();
                Output::Signatures(signatures)
            }
            Err(why) => Output::Failed(why),
        },
        Operation::Format(_) => match pg::format_with_indent(text, indent) {
            Ok(text) => Output::Formatted(format_work::ToolResult::Formatted { tool: "PostgreSQL offline formatter", edit: format_work::difference(doc.text(), text) }),
            Err(why) => Output::Formatted(format_work::ToolResult::Failed(why)),
        },
    }
}

fn evaluate_duck(doc: &Arc<Document>, caret: usize, operation: Operation, indent: sqlformat::Indent, native: Option<&crate::duckdb_assistance::DuckDb>, catalog: Option<&crate::sql_metadata::Catalog>) -> Output {
    let text = doc.text();
    if text.len() > 262144 { return Output::Failed("SQL assistance is limited to 256 KiB".into()); }
    if let Some(output) = catalog_output(text, caret, operation, catalog, Dialect::DuckDb) { return output; }
    if let Operation::Complete(_) = operation {
        return match crate::sql_assistance::completion_range(text, caret).and_then(|range| crate::duckdb_assistance::complete(text, caret).map(|names| (range, names))) {
            Ok((range, names)) => Output::Names { range, names }, Err(why) => Output::Failed(why),
        };
    }
    let Some(native) = native else { return Output::Failed("install DuckDB on PATH or configure duckdb_executable in Lenz settings; query-local completion still works".into()) };
    match operation {
        Operation::Check => {
            let error = native.diagnose(text).err();
            if let Some(error) = error.as_ref().filter(|e| e.byte_offset.is_none()) { return Output::Failed(error.message.clone()); }
            let items = error.into_iter().map(|error| {
                let start = error.byte_offset.unwrap_or(0);
                let length = error.byte_length.unwrap_or_else(|| text.get(start..).and_then(|s| s.chars().next()).map_or(0, char::len_utf8));
                Diagnostic { range: start..start + length, severity: Severity::Error, message: error.message, source: Some("DuckDB syntax (offline)".into()), code: None, raw: serde_json::Value::Null }
            }).collect();
            Output::Problems(Arc::new(FileDiagnostics::new(doc, items)))
        }
        Operation::Signature(_) => match native.signature_help(text, caret) {
            Ok(help) => Output::Signatures(help.into_iter().flat_map(|h| h.overloads.into_iter().map(move |f| {
                let parameters = f.parameters.iter().zip(&f.parameter_types).map(|(p,t)| serde_json::json!({"label": format!("{p}: {t}")})).collect::<Vec<_>>();
                let label = format!("{}({}) → {}", f.function_name, f.parameters.iter().zip(&f.parameter_types).map(|(p,t)| format!("{p}: {t}")).collect::<Vec<_>>().join(", "), f.return_type.as_deref().unwrap_or("unknown"));
                super::signature::signature(&serde_json::json!({"label":label,"documentation":f.description,"parameters":parameters,"activeParameter":h.active_parameter}), &serde_json::Value::Null, semantics::lsp::Encoding::Utf8)
            })).flatten().collect()),
            Err(why) => Output::Failed(why),
        },
        Operation::Format(_) => match native.format_with_indent(text, indent) {
            Ok(text) => Output::Formatted(format_work::ToolResult::Formatted { tool: "DuckDB offline formatter", edit: format_work::difference(doc.text(), text) }),
            Err(why) => Output::Formatted(format_work::ToolResult::Failed(why)),
        },
        Operation::Complete(_) => Output::Failed("completion was superseded".into()),
    }
}

fn catalog_output(text: &str, caret: usize, operation: Operation, catalog: Option<&crate::sql_metadata::Catalog>, dialect: Dialect) -> Option<Output> {
    let catalog = catalog?;
    let parser: &dyn sqlparser::dialect::Dialect = match dialect { Dialect::PostgreSql => &sqlparser::dialect::PostgreSqlDialect {}, Dialect::DuckDb => &crate::duckdb_assistance::editing_dialect() };
    match operation {
        Operation::Complete(_) => {
            let names = catalog.names();
            let funcs = catalog.functions.iter().map(|f| f.name.as_str()).collect::<Vec<_>>();
            Some(match crate::sql_assistance::completion_range(text, caret).and_then(|range| crate::sql_assistance::complete(parser, &funcs, text, caret, &names).map(|names| (range, names))) {
                Ok((range, mut names)) => {
                    if let Ok(offline) = match dialect { Dialect::PostgreSql => crate::postgres_assistance::complete(text, caret), Dialect::DuckDb => crate::duckdb_assistance::complete(text, caret) } { names.extend(offline); names.sort(); names.dedup(); }
                    Output::Names { range, names }
                }, Err(why) => Output::Failed(why),
            })
        }
        Operation::Signature(_) => {
            let (name, parameter) = crate::sql_assistance::call_at(parser, text, caret).ok()??;
            let matches = catalog.functions.iter().filter(|f| if dialect == Dialect::DuckDb { duck_name(&f.name) == duck_name(&name) } else { f.name == name }).take(8).filter_map(|f| {
                let parameters = f.arguments.split(',').map(|p| serde_json::json!({"label":p.trim()})).collect::<Vec<_>>();
                super::signature::signature(&serde_json::json!({"label":format!("{}({}) → {}",f.name,f.arguments,f.result),"documentation":"Explicit read-only catalog snapshot; function never executed", "parameters":parameters,"activeParameter":parameter}), &serde_json::Value::Null, semantics::lsp::Encoding::Utf8)
            }).collect::<Vec<_>>();
            (!matches.is_empty()).then_some(Output::Signatures(matches))
        }
        _ => None,
    }
}

impl Session {
    fn metadata_scope(&self) -> Option<(Dialect, u64, PathBuf)> {
        let dialect = self.sql_identity()?.dialect;
        let source = self.state.current_source()?.source();
        let root = self.state.project.as_ref().map(|p| p.root()).filter(|r| source.starts_with(r)).unwrap_or_else(|| source.parent().unwrap_or(source));
        Some((dialect, self.project_generation.load(Ordering::Acquire), root.to_owned()))
    }
    fn sync_sql_metadata(&mut self) {
        if self.sql_work.metadata.file_checked.is_none_or(|at| at.elapsed() >= std::time::Duration::from_secs(1)) {
            self.sql_work.metadata.file_checked = Some(std::time::Instant::now());
            if self.sql_work.metadata.catalog.as_ref().is_some_and(|(_,c)| !c.file_current()) { self.disconnect_sql_metadata(); self.sql_work.metadata.failure = Some("metadata file changed; reconnect required".into()); self.state.messages.record("SQL metadata warning: file changed; reconnect required"); }
        }
        if self.sql_work.metadata.catalog.as_ref().is_some_and(|(at,_)| at.elapsed() >= CATALOG_AGE) {
            self.disconnect_sql_metadata();
            self.sql_work.metadata.failure = Some("catalog expired; refresh required".into());
            self.state.messages.record("SQL metadata warning: catalog expired; refresh required");
        }
        let scope = self.metadata_scope();
        if self.sql_work.metadata.scope != scope {
            if !self.sql_work.metadata.source.is_empty() {
                self.state.messages.record("SQL metadata: configuration cleared after changing the active SQL file context, dialect or workspace; configure and connect again");
            }
            let seq = self.sql_work.metadata.seq.clone(); seq.fetch_add(1, Ordering::Release);
            self.sql_work.metadata = Metadata { scope, seq, source: String::new(), pending: None, catalog: None, failure: None, file_checked: None };
            self.sql_generation += 1;
            self.sql_context_changed();
        }
    }
    pub(super) fn sql_catalog(&self) -> Option<Arc<crate::sql_metadata::Catalog>> {
        let metadata = &self.sql_work.metadata;
        if metadata.scope != self.metadata_scope() { return None; }
        metadata.catalog.as_ref().filter(|(at,_)| at.elapsed() < CATALOG_AGE).filter(|(_,c)| c.file_current()).map(|(_,c)| c.clone())
    }
    pub fn sql_metadata_source(&self) -> &str { if self.sql_work.metadata.scope == self.metadata_scope() { &self.sql_work.metadata.source } else { "" } }
    pub fn sql_metadata_prompt(&self) -> &'static str { if self.sql_identity().is_some_and(|i| i.dialect == Dialect::DuckDb) { "Absolute local DuckDB or SQLite file path · Enter configures only; use Connect SQL Metadata next" } else { "PostgreSQL libpq service name · Enter configures only; use Connect SQL Metadata next" } }
    pub(super) fn sql_metadata_status(&self) -> String {
        let m = &self.sql_work.metadata;
        if m.scope != self.metadata_scope() { return "metadata off".into(); }
        if m.pending.is_some() { return "metadata loading".into(); }
        if m.failure.is_some() { return "metadata failed".into(); }
        m.catalog.as_ref().map_or_else(|| "metadata off".into(), |(at,_)| if at.elapsed() < CATALOG_AGE { format!("metadata {}s",at.elapsed().as_secs()) } else { "metadata stale".into() })
    }
    pub fn sql_metadata_details(&self) -> String {
        if self.sql_work.metadata.scope != self.metadata_scope() || self.sql_work.metadata.source.is_empty() { return "Metadata: disconnected; configure a metadata source and explicitly Connect. PostgreSQL passwords stay in libpq files. Opening files never connects.".into(); }
        let m = &self.sql_work.metadata;
        if m.pending.is_some() { return "Metadata: connecting for a read-only snapshot…".into(); }
        if let Some(why) = &m.failure { return format!("Metadata: disconnected; {why}. Check the source and permissions, then Connect again."); }
        if let Some((at,catalog)) = &m.catalog {
            return format!("Metadata: {} · snapshot age {}s · {} relations, {} function overloads · connection closed after fetch. Refresh explicitly; expires after 300s.", if at.elapsed() < CATALOG_AGE { "snapshot loaded" } else { "stale; assistance rejected" }, at.elapsed().as_secs(),catalog.relations.len(),catalog.functions.len());
        }
        "Metadata: configured; disconnected. Connect explicitly to fetch read-only metadata.".into()
    }
    pub(super) fn sql_metadata_disabled(&self, id: CommandId) -> Option<&'static str> {
        if self.sql_identity().is_none() { return Some("Open an SQL file and select its dialect first"); }
        if id != CommandId::ConfigureSqlMetadata && id != CommandId::DisconnectSqlMetadata && self.sql_identity().is_some_and(|i| i.dialect == Dialect::DuckDb) && self.sql_work.executable.is_none() { return Some("Install DuckDB or configure duckdb_executable first"); }
        match id {
            CommandId::ConfigureSqlMetadata => None,
            CommandId::DisconnectSqlMetadata => (self.sql_work.metadata.pending.is_none() && self.sql_work.metadata.catalog.is_none() && self.sql_work.metadata.failure.is_none()).then_some("Metadata is disconnected"),
            _ if self.sql_metadata_source().is_empty() => Some("Configure SQL Metadata first"),
            _ if self.sql_work.metadata.pending.is_some() => Some("A metadata request is in progress; Disconnect cancels it"),
            _ if id == CommandId::RefreshSqlMetadata && self.sql_work.metadata.catalog.is_none() => Some("Connect SQL Metadata first"),
            _ => None,
        }
    }
    pub(super) fn configure_sql_metadata(&mut self, source: String) -> Effect {
        self.sync_sql_metadata();
        if self.sql_metadata_disabled(CommandId::ConfigureSqlMetadata).is_some() { return Effect::default(); }
        let source = source.trim().to_owned();
        if !source.is_empty() && self.sql_identity().is_some_and(|i| i.dialect == Dialect::PostgreSql) && !crate::sql_metadata::valid_service(&source) { self.state.messages.push("Use a libpq service name, not credentials or a connection string".into()); return Effect {repaint:true,..Effect::default()}; }
        if self.sql_identity().is_some_and(|i| i.dialect == Dialect::DuckDb) && !source.is_empty() && (!Path::new(&source).is_absolute() || source.chars().any(|c| c.is_control())) { self.state.messages.push("Choose an absolute local DuckDB or SQLite path without control characters".into()); return Effect { repaint:true,..Effect::default() }; }
        self.disconnect_sql_metadata();
        self.sql_work.metadata.source = source;
        self.state.messages.record(if self.sql_work.metadata.source.is_empty() { "SQL metadata: configuration cleared" } else { "SQL metadata: source configured; choose Connect SQL Metadata to load it" });
        Effect {repaint:true,..Effect::default()}
    }
    pub(super) fn disconnect_sql_metadata(&mut self) -> Effect {
        let m = &mut self.sql_work.metadata;
        m.seq.fetch_add(1, Ordering::Release); m.pending = None; m.catalog = None; m.failure = None;
        self.sql_generation += 1; self.sql_context_changed();
        Effect {repaint:true,..Effect::default()}
    }
    pub(super) fn connect_sql_metadata(&mut self) -> Effect {
        self.sync_sql_metadata();
        if self.sql_metadata_disabled(CommandId::ConnectSqlMetadata).is_some() { return Effect::default(); }
        self.disconnect_sql_metadata();
        let dialect = self.sql_identity().map(|i| i.dialect);
        let executable = self.sql_work.executable.clone();
        let m = &mut self.sql_work.metadata;
        let request = m.seq.fetch_add(1, Ordering::AcqRel) + 1;
        let (source, seq, tx) = (m.source.clone(), m.seq.clone(), self.tx.clone());
        m.pending = Some(request);
        self.state.messages.record("SQL metadata: connecting…");
        self.jobs.submit(Priority::Visible, move || {
            let current = || seq.load(Ordering::Acquire) == request;
            let result = if dialect == Some(Dialect::DuckDb) { executable.as_ref().ok_or_else(|| "DuckDB executable is unavailable".to_owned()).and_then(|e| crate::sql_metadata::duckdb(e,Path::new(&source),&current)) } else { crate::sql_metadata::postgres(Path::new("psql"), &source, &current) };
            let _ = tx.send_blocking(Completion::SqlMetadata { request, result });
        });
        Effect {repaint:true,..Effect::default()}
    }
    pub(super) fn sql_metadata_reply(&mut self, request: u64, result: Result<crate::sql_metadata::Catalog, String>) -> Effect {
        self.sync_sql_metadata();
        let m = &mut self.sql_work.metadata;
        if m.pending != Some(request) || m.seq.load(Ordering::Acquire) != request { return Effect::default(); }
        m.pending = None;
        match result.and_then(|c| if c.file_current() { Ok(c) } else { Err("Metadata file changed; reconnect required".into()) }) { Ok(c) => {
            self.state.messages.record(&format!("SQL metadata: connected; {} relations, {} functions", c.relations.len(), c.functions.len()));
            m.catalog = Some((std::time::Instant::now(), Arc::new(c)));
        }, Err(why) => { self.state.messages.push(format!("SQL metadata failed: {why}")); m.failure = Some(why); } }
        self.sql_generation += 1; self.sql_context_changed();
        Effect {repaint:true,..Effect::default()}
    }
}

fn duck_name(name: &str) -> Vec<String> {
    use sqlparser::tokenizer::{Token,Tokenizer};
    Tokenizer::new(&sqlparser::dialect::DuckDbDialect {},name).tokenize().unwrap_or_default().into_iter().filter_map(|t| if let Token::Word(w)=t {Some(w.value.to_ascii_lowercase())}else{None}).collect()
}
