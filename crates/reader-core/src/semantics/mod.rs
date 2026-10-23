//! Semantic providers: language servers launched off the UI thread when a relevant source opens, keyed by
//! provider and resolved workspace root, kept in sync with the documents the reader shows, and asked for
//! definitions. Every server event re-enters the session as a completion tagged with its server session, so
//! replies from a replaced or stopped server are ignored.

pub mod hierarchy;
pub mod hover;
pub mod lsp;
pub mod java;
pub mod runtime;
pub mod provider;

use std::{
    collections::{HashMap, VecDeque},
    io::BufReader,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
};

use serde_json::{Value, json};

use crate::{decor::{Decor, Legend}, document::Document, session::Completion};
use lsp::Encoding;
use provider::{Launch, PROVIDERS, Provider, Settings};

/// What the reader's clock says now; tests drive it.
pub type Clock = Arc<dyn Fn() -> std::time::Instant + Send + Sync>;

/// How long a server with no open documents and nothing pending stays warm before it is stopped.
pub const IDLE: std::time::Duration = std::time::Duration::from_secs(600);
/// Combined server memory above which the reader says so (it never stops servers for it).
pub const MEMORY_NOTICE: u64 = 2 << 30;
/// Usages and symbols kept at most (the panel's limit).
pub const USAGE_LIMIT: usize = crate::search::RESULT_LIMIT;
/// Usages a type-hierarchy level reads at most (and children it shows).
pub const TYPE_LEVEL_LIMIT: usize = 200;

/// What the reader asks a provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Question {
    /// The definition at a position (also for a peek).
    Definition,
    /// Every reference to the symbol at a position, its declaration excluded.
    Usages,
    /// Workspace symbols matching a query.
    Symbols,
    /// One document's symbols, nested.
    FileSymbols,
    /// The implementations of the trait, interface or abstract member at a position.
    Implementation,
    /// The definition of the type of the symbol at a position.
    TypeDefinition,
    /// The call-hierarchy item at a position (the root of a hierarchy).
    PrepareCallHierarchy,
    /// The callers of a call-hierarchy item.
    IncomingCalls,
    /// What a call-hierarchy item calls.
    OutgoingCalls,
    /// The type-hierarchy item at a position (the root of a hierarchy from the server).
    PrepareTypeHierarchy,
    /// A type-hierarchy item's supertypes.
    Supertypes,
    /// A type-hierarchy item's subtypes.
    Subtypes,
    /// Usages of a type, its declaration included: a level of a type hierarchy built from syntax.
    TypeUsages,
    /// Documentation of the symbol at a position.
    Hover,
    /// The occurrences in its document of the symbol at a position.
    Highlight,
}

/// Which of a server's requests a question replaces: each lane has at most one pending, so a background question
/// (documentation on a pointer rest, occurrences at the caret, a hierarchy level) never cancels the user's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lane {
    Foreground,
    Hover,
    Highlight,
    Hierarchy,
}

impl Question {
    fn method(self) -> &'static str {
        match self {
            Question::Definition => "textDocument/definition",
            Question::Usages => "textDocument/references",
            Question::Symbols => "workspace/symbol",
            Question::FileSymbols => "textDocument/documentSymbol",
            Question::Implementation => "textDocument/implementation",
            Question::TypeDefinition => "textDocument/typeDefinition",
            Question::PrepareCallHierarchy => "textDocument/prepareCallHierarchy",
            Question::IncomingCalls => "callHierarchy/incomingCalls",
            Question::OutgoingCalls => "callHierarchy/outgoingCalls",
            Question::PrepareTypeHierarchy => "textDocument/prepareTypeHierarchy",
            Question::Supertypes => "typeHierarchy/supertypes",
            Question::Subtypes => "typeHierarchy/subtypes",
            Question::TypeUsages => "textDocument/references",
            Question::Hover => "textDocument/hover",
            Question::Highlight => "textDocument/documentHighlight",
        }
    }

    fn capability(self) -> &'static str {
        match self {
            Question::Definition => "definitionProvider",
            Question::Usages => "referencesProvider",
            Question::Symbols => "workspaceSymbolProvider",
            Question::FileSymbols => "documentSymbolProvider",
            Question::Implementation => "implementationProvider",
            Question::TypeDefinition => "typeDefinitionProvider",
            Question::PrepareCallHierarchy | Question::IncomingCalls | Question::OutgoingCalls => "callHierarchyProvider",
            Question::PrepareTypeHierarchy | Question::Supertypes | Question::Subtypes => "typeHierarchyProvider",
            Question::TypeUsages => "referencesProvider",
            Question::Hover => "hoverProvider",
            Question::Highlight => "documentHighlightProvider",
        }
    }

    /// What it finds, for messages.
    pub fn noun(self) -> &'static str {
        match self {
            Question::Definition => "definitions",
            Question::Usages => "usages",
            Question::Symbols => "workspace symbols",
            Question::FileSymbols => "file symbols",
            Question::Implementation => "implementations",
            Question::TypeDefinition => "type definitions",
            Question::PrepareCallHierarchy | Question::IncomingCalls | Question::OutgoingCalls => "call hierarchy",
            Question::PrepareTypeHierarchy | Question::Supertypes | Question::Subtypes => "type hierarchy",
            Question::TypeUsages => "usages",
            Question::Hover => "documentation",
            Question::Highlight => "occurrence highlighting",
        }
    }

    /// The lane its request goes in.
    pub fn lane(self) -> Lane {
        match self {
            Question::Hover => Lane::Hover,
            Question::Highlight => Lane::Highlight,
            // A hierarchy's root is the user's own request; its levels load in the background.
            Question::IncomingCalls | Question::OutgoingCalls | Question::Supertypes | Question::Subtypes | Question::TypeUsages => Lane::Hierarchy,
            Question::Definition | Question::Usages | Question::Symbols | Question::FileSymbols | Question::Implementation | Question::TypeDefinition | Question::PrepareCallHierarchy | Question::PrepareTypeHierarchy => Lane::Foreground,
        }
    }

    /// Every question, for capability views.
    pub const ALL: [Question; 15] = [
        Question::Definition,
        Question::Usages,
        Question::Symbols,
        Question::FileSymbols,
        Question::Implementation,
        Question::TypeDefinition,
        Question::PrepareCallHierarchy,
        Question::IncomingCalls,
        Question::OutgoingCalls,
        Question::Hover,
        Question::Highlight,
        Question::PrepareTypeHierarchy,
        Question::Supertypes,
        Question::Subtypes,
        Question::TypeUsages,
    ];
}

/// Servers that may be starting (launched, not yet initialized) at once; more wait their turn. Not a cap on
/// running servers.
pub const MAX_STARTING: usize = 2;
/// Messages queued for a server's input at most; a server that stops reading is reported, never waited for.
pub const OUTBOX: usize = 1024;
/// Historical source payload awaiting a completed writer flush, per server. Desired live documents and
/// the last written snapshots are separate owners; serialization scratch belongs to the writer alone.
const TEXT_FRAMES: usize = 3;
const TEXT_BYTES: usize = 3 * crate::document::LARGE_BYTES;
/// How long a stopping server gets to exit after `shutdown`/`exit` before its process group is killed.
pub const STOP_GRACE: std::time::Duration = std::time::Duration::from_secs(3);
/// After initializing, a server without a readiness status (progress reports do not say the workspace is
/// loaded) may still be loading it for about this long: its answers meanwhile are marked possibly incomplete.
pub const UNREPORTED_WARMUP: std::time::Duration = std::time::Duration::from_secs(10);
/// Definition targets kept at most.
pub const TARGET_LIMIT: usize = 200;
/// How long a server may take to answer `initialize` before it is given up on (its startup slot freed).
pub const START_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// A server: which provider, for which workspace root, launched how.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub provider: usize,
    pub root: PathBuf,
    pub launch: Launch,
}

/// What a provider is doing for a workspace, as the user is told it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Finding the workspace root, or waiting for a free startup slot.
    Queued,
    /// Checking the executable, launching and initializing it.
    Starting,
    /// Initialized; still loading or indexing (what it reports, if anything).
    Indexing(String),
    /// Initialized and reporting no work in progress.
    Ready,
    /// Not installed, unusable, or no workspace: with setup guidance.
    Missing(String),
    /// Stopped by itself, or unusable after start; never restarted automatically.
    Failed(String),
    /// Turned off for this folder by the user.
    Disabled,
}

/// A provider's state for one workspace, for presentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderView {
    pub name: &'static str,
    pub root: Option<PathBuf>,
    pub status: Status,
    /// The version it reported, and whether it is one the reader was tested with.
    pub version: Option<String>,
    pub untested: bool,
    pub limits: &'static str,
    /// Resident memory of the server and, separately, of the helpers in its process group, when last sampled.
    pub memory: Option<(u64, u64)>,
    /// What it offers, once initialized (empty until then).
    pub supports: Vec<Question>,
    /// The last error it logged or showed (`window/logMessage`, `window/showMessage`); it may keep working.
    pub error: Option<String>,
}

impl ProviderView {
    /// Short, persistent status control; full errors and recovery actions live in the status dialog.
    pub fn status_label(&self) -> String {
        match &self.status {
            Status::Queued => "LSP waiting",
            Status::Starting => "LSP starting",
            Status::Indexing(what) if !what.is_empty() => return format!("LSP {}indexing {what}", self.error_mark()),
            Status::Indexing(_) => return format!("LSP {}indexing", self.error_mark()),
            Status::Ready => return format!("LSP {}ready", self.error_mark()),
            Status::Missing(_) => "LSP unavailable",
            Status::Failed(_) => "LSP stopped",
            Status::Disabled => "LSP off",
        }.to_owned()
    }

    /// First, so a narrow status bar keeps it.
    fn error_mark(&self) -> &'static str {
        if self.error.is_some() { "error · " } else { "" }
    }

    /// Whether it offers `question` (false until it has initialized).
    pub fn offers(&self, question: Question) -> bool {
        self.supports.contains(&question)
    }

    /// One line for a status bar.
    pub fn summary(&self) -> String {
        let state = match &self.status {
            Status::Queued => "waiting to start".to_string(),
            Status::Starting => "starting…".to_string(),
            Status::Indexing(what) => format!("indexing: {what}"),
            Status::Ready => "ready".to_string(),
            Status::Missing(why) => format!("unavailable: {why}"),
            Status::Failed(why) => format!("stopped: {why}"),
            Status::Disabled => "disabled for this folder".to_string(),
        };
        let untested = match (&self.version, self.untested) {
            (Some(v), true) => format!(" (untested version {v})"),
            (None, true) => " (version unknown: untested)".to_string(),
            _ => String::new(),
        };
        let mib = |b: u64| b >> 20;
        let memory = match self.memory {
            Some((server, 0)) => format!(", {} MiB", mib(server)),
            Some((server, helpers)) => format!(", {} MiB + {} MiB helpers", mib(server), mib(helpers)),
            None => String::new(),
        };
        format!("{}{untested}: {state}{memory}", self.name)
    }
}

/// A target a provider named outside the file system: its opaque URI, kept with the provider and server
/// session that produced it (meaningless to any other).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VirtualSource {
    pub uri: Arc<str>,
    pub provider: &'static str,
    pub session: u64,
}

impl VirtualSource {
    /// Why it cannot be shown when its provider has no retrieval method the reader implements. The reader never
    /// guesses commands or fetches arbitrary schemes.
    pub fn unavailable(&self) -> String {
        format!("{} offers no documented way for the reader to show {}", self.provider, self.uri)
    }

    /// The name of the file it stands for (the URI's last path segment, without a query), for titles and
    /// language detection.
    pub fn name(&self) -> String {
        let path = self.uri.split(['?', '#']).next().unwrap_or_default();
        path.rsplit('/').next().unwrap_or_default().to_string()
    }

    /// Whether `provider` implements retrieving it.
    pub fn retrievable(&self) -> bool {
        PROVIDERS.iter().any(|p| p.name == self.provider && p.virtual_contents.is_some())
    }
}

/// A target outside the file system, with the range the provider named in it.
#[derive(Clone, Debug)]
pub struct VirtualTarget {
    pub source: VirtualSource,
    pub range: Value,
}

/// Server events, delivered through the session's completion channel.
/// A workspace root and the provider serving it.
type Served = (usize, PathBuf);

pub enum Event {
    /// The workspace root for sources in `dir` by provider `provider`'s rules (or the override), or none.
    /// `provider` is the language's; `serve` the provider serving `root` ([`provider::serving`]).
    Resolved { provider: usize, dir: PathBuf, root: Option<PathBuf>, serve: usize },
    /// The executable answered `--version` (or could not be run).
    Detected { session: u64, version: Result<String, String> },
    Message { session: u64, message: Value },
    /// The answer to decoration request `id` (`failed`: an error), as its bytes: decoded off the UI thread.
    Decoration { session: u64, id: i64, failed: bool, body: Vec<u8> },
    Exited { session: u64, reason: String },
    /// A complete text frame was flushed before the writer advanced to the following message.
    Written { session: u64, token: u64 },
    /// [`START_TIMEOUT`] passed since launch.
    StartTimeout { session: u64 },
    /// Time passed: idle servers may stop.
    Tick,
    /// Resident bytes per server session: the server's own, and its process group's other members'.
    Memory(Vec<(u64, u64, u64)>),
}

/// A definition reply for the session to resolve, or why there is none.
pub struct Reply {
    /// The lane it was asked in.
    pub lane: Lane,
    pub request: u64,
    pub session: u64,
    pub encoding: Encoding,
    pub provider: &'static str,
    pub result: Result<Value, String>,
    /// The server was still indexing when it answered.
    pub indexing: bool,
}

/// What the session must do after an event.
#[derive(Default)]
pub struct Outcome {
    pub repaint: bool,
    /// Answers: one per request a message completes, or each request still pending when a server exits.
    pub replies: Vec<Reply>,
    /// A server became ready: documents can be opened and a waiting request sent.
    pub ready: bool,
    /// Decoration replies to decode.
    pub decorations: Vec<Decoration>,
}

/// A decoration reply for the session to decode, about the version it was asked about.
pub struct Decoration {
    pub session: u64,
    /// The request it answers: applied only while it is the latest asked about its document and kind.
    pub id: i64,
    pub path: PathBuf,
    pub version: u64,
    pub kind: Decor,
    pub encoding: Encoding,
    pub legend: Arc<Legend>,
    /// The response's bytes.
    pub body: Vec<u8>,
}

struct Server {
    key: Key,
    /// The document whose opening started it (JDT LS imports a plain folder from it).
    trigger: PathBuf,
    session: u64,
    status: Status,
    version: Option<String>,
    /// The last error-level message it logged or showed.
    error: Option<String>,
    capabilities: Value,
    encoding: Encoding,
    outbox: async_channel::Sender<Outgoing>,
    /// The outbox's other end, until the server launches and its writer takes it.
    inbox: Option<async_channel::Receiver<Outgoing>>,
    process: Arc<Mutex<Process>>,
    next_id: i64,
    /// The initialize request's id, until it is answered.
    initializing: Option<i64>,
    /// Documents the server has open, with the snapshot it was sent.
    open: HashMap<PathBuf, Arc<Document>>,
    /// Latest live demand, coalesced while the writer has no text credit.
    desired: HashMap<PathBuf, Arc<Document>>,
    /// Exact fully written snapshots used to interpret replies.
    written: HashMap<PathBuf, Arc<Document>>,
    /// Credits survive close/reopen and rename until their writer acknowledgement arrives.
    flights: HashMap<u64, TextFlight>,
    open_epoch: HashMap<PathBuf, u64>,
    next_text: u64,
    text_bytes: usize,
    desired_epoch: u64,
    deferred: HashMap<Lane, Deferred>,
    failed: HashMap<Lane, Reply>,
    refresh: std::collections::HashSet<PathBuf>,
    /// External changes a client-watched server may defer while it holds the document open.
    dirty_open: HashMap<PathBuf, bool>,
    /// The definition request in flight: the session's request id and the protocol id.
    /// Each lane's pending request: the session request and its JSON-RPC id.
    pending: HashMap<Lane, (u64, i64)>,
    /// Work-done progress in progress, by token.
    progress: HashMap<String, Progress>,
    /// rust-analyzer's `experimental/serverStatus`: not quiescent while loading the workspace.
    busy: bool,
    /// A tab holds one of its documents (shown or released).
    held: bool,
    /// Since when no tab has held its documents and nothing has been pending.
    idle_since: Option<std::time::Instant>,
    /// When it answered `initialize`.
    initialized_at: Option<std::time::Instant>,
    memory: Option<(u64, u64)>,
    /// The server's semantic token legend (empty without one).
    legend: Arc<Legend>,
    /// Per decoration kind: the version each document was last asked about, that request's id, and whether the
    /// answer is provisional (asked while the server was loading, or refused): asked again once it is ready.
    asked: [HashMap<PathBuf, (u64, i64, bool)>; 2],
    /// Decoration requests in flight, by id: the document, its version and the kind.
    decor: HashMap<i64, (PathBuf, u64, Decor)>,
}

struct TextFlight { path: PathBuf, document: Arc<Document>, epoch: u64 }
struct Deferred { method: &'static str, params: Value, request: u64, epoch: u64 }

#[derive(Default)]
struct Progress {
    title: String,
    message: String,
    percentage: Option<u64>,
}

impl Progress {
    fn update(&mut self, value: &Value) {
        if let Some(message) = value["message"].as_str() { self.message = message.to_owned(); }
        if let Some(percentage) = value["percentage"].as_u64().filter(|p| *p <= 100) { self.percentage = Some(percentage); }
    }

    fn summary(&self) -> String {
        let percentage = self.percentage.map_or(String::new(), |p| format!("{p}% "));
        format!("{percentage}{}{}{}", self.title, if self.message.is_empty() { "" } else { " " }, self.message)
    }
}

/// A message for a server's input. Document text travels shared and is encoded by the writer thread, so the UI
/// thread never copies a document to send it.
enum Outgoing {
    Message(Value),
    /// `textDocument/didOpen` (with a language) or `didChange` (full text) for `uri` at `version`.
    Text { token: u64, uri: String, language: Option<&'static str>, version: u64, text: Arc<str> },
}

impl Outgoing {
    fn write(self, out: &mut impl std::io::Write) -> std::io::Result<()> {
        let message = match self {
            Outgoing::Message(m) => m,
            Outgoing::Text { uri, language: Some(language), version, text, .. } => json!({
                "jsonrpc": "2.0", "method": "textDocument/didOpen",
                "params": { "textDocument": { "uri": uri, "languageId": language, "version": version, "text": &*text } },
            }),
            Outgoing::Text { uri, language: None, version, text, .. } => json!({
                "jsonrpc": "2.0", "method": "textDocument/didChange",
                "params": { "textDocument": { "uri": uri, "version": version }, "contentChanges": [{ "text": &*text }] },
            }),
        };
        lsp::write_message(out, &message)
    }
}

/// The launched process, shared with the threads that start and stop it.
#[derive(Default)]
///
/// Whoever takes `child` kills its process group, then reaps it: never the other way round, so a reaped
/// process's id (which the system may reuse) is never signalled.
struct Process {
    child: Option<Child>,
    stopped: bool,
    /// Its output closed (it exited, or is about to).
    closed: bool,
}

/// A request about a position, waiting for its server to start.
struct Waiting {
    request: u64,
    question: Question,
    path: PathBuf,
    offset: usize,
    version: u64,
}

pub struct Semantics {
    settings: Settings,
    settings_file: PathBuf,
    servers: Vec<Server>,
    /// Servers waiting for a startup slot, in order.
    queued: VecDeque<u64>,
    /// Workspace roots, with the provider serving each, per (the language's provider, directory): `None` while
    /// resolving.
    roots: HashMap<(usize, PathBuf), Option<Option<Served>>>,
    next_session: u64,
    waiting: Option<Waiting>,
    /// The browsing root, as of the last sync.
    browse: Option<PathBuf>,
    /// The server session last sent a question of the user's: the one whose answer opened a dependency.
    last_asked: Option<u64>,
    /// Server-originated notices for the session to show (declined edits, server errors).
    pub(crate) notices: Vec<String>,
    idle: std::time::Duration,
    clock: Clock,
    /// The reader's cache folder, outside any checkout: servers' workspace data lives under it.
    cache: PathBuf,
}

impl Semantics {
    pub fn new(settings_file: PathBuf, cache: PathBuf, idle: std::time::Duration, clock: Clock) -> Self {
        Self {
            cache,
            idle,
            clock,
            settings: Settings::read(&settings_file),
            settings_file,
            servers: Vec::new(),
            queued: VecDeque::new(),
            roots: HashMap::new(),
            next_session: 0,
            waiting: None,
            browse: None,
            last_asked: None,
            notices: Vec::new(),
        }
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn settings_file(&self) -> &Path {
        &self.settings_file
    }

    /// Eligibility can be checked before materializing a draft. Workspace resolution and executable
    /// detection remain downstream, on their existing paths.
    pub(crate) fn wants_text(&self, language: Option<lenz_content::highlight::Language>, large: bool, browse: Option<&Path>) -> bool {
        !large && language.and_then(provider::for_language).is_some_and(|(p, _)| !self.settings.disabled(&PROVIDERS[provider::primary(p)], browse))
    }

    /// The provider for `doc`'s language and, once resolved, its workspace root and the provider serving it.
    pub(crate) fn resolved(&self, doc: &Document) -> Option<(usize, Option<Option<PathBuf>>)> {
        self.resolved_source(doc.source_meta())
    }
    pub(crate) fn resolved_source(&self, source: &crate::document::SourceMeta) -> Option<(usize, Option<Option<PathBuf>>)> {
        let (p, _) = provider::for_language(source.language()?)?;
        let dir = source.source().parent()?.to_owned();
        Some(match self.roots.get(&(p, dir)) {
            Some(Some(Some((q, root)))) => (*q, Some(Some(root.clone()))),
            Some(Some(None)) => (p, Some(None)),
            _ => (p, None),
        })
    }

    fn server(&self, provider: usize, root: &Path) -> Option<&Server> {
        self.servers.iter().find(|s| s.key.provider == provider && s.key.root == root)
    }

    /// The provider state for `doc`, for presentation; `None` when no provider serves its language.
    pub fn view(&self, doc: &Document) -> Option<ProviderView> {
        self.view_source(doc.source_meta(), doc.is_large())
    }
    pub(crate) fn view_source(&self, source: &crate::document::SourceMeta, large: bool) -> Option<ProviderView> {
        let (p, root) = self.resolved_source(source)?;
        let provider = &PROVIDERS[p];
        let view = |root: Option<PathBuf>, status| ProviderView { name: provider.name, root, status, version: None, untested: false, limits: provider.limits, memory: None, supports: Vec::new(), error: None };
        if large {
            return Some(view(None, Status::Missing("not used for large files".into())));
        }
        if self.disabled_for(p, source.source()) {
            return Some(view(None, Status::Disabled));
        }
        Some(match root {
            None => view(None, Status::Queued),
            Some(None) => view(None, Status::Missing(provider.no_root.into())),
            Some(Some(root)) => match self.server(p, &root) {
                None => view(Some(root), Status::Queued),
                Some(s) => s.view(),
            },
        })
    }

    /// Every server this session runs or has tried to run, in launch order.
    pub fn views(&self) -> Vec<ProviderView> {
        self.servers.iter().map(Server::view).collect()
    }

    /// Workspace manifests changed: roots are resolved again for the next documents shown.
    pub fn manifests_changed(&mut self) {
        self.roots.clear();
    }

    /// Bring servers in line with `docs`, the documents shown: resolve roots (through `resolve`, off the UI
    /// thread), queue servers for new workspaces, launch at most [`MAX_STARTING`] at a time, and open, update or
    /// close documents in initialized servers. Large documents are never sent.
    /// `held` lists every tab's source and language (loaded or released): a server with any keeps warm.
    pub fn sync(&mut self, docs: &[Arc<Document>], held: &[(&Path, Option<lenz_content::highlight::Language>)], browse: Option<&Path>, resolve: &mut dyn FnMut(usize, PathBuf, PathBuf, Option<PathBuf>, Settings), events: &async_channel::Sender<Completion>) {
        self.browse = browse.map(Path::to_owned);
        let mut wanted: HashMap<u64, HashMap<PathBuf, Arc<Document>>> = HashMap::new();
        for doc in docs.iter().filter(|d| !d.is_large()) {
            let Some((p, _)) = doc.language().and_then(provider::for_language) else { continue };
            if self.disabled_for(p, doc.source()) {
                continue;
            }
            let Some(dir) = doc.source().parent().map(Path::to_owned) else { continue };
            let (q, root) = match self.roots.get(&(p, dir.clone())) {
                Some(Some(Some((q, root)))) => (*q, root.clone()),
                Some(_) => continue, // resolving, or no workspace
                None if let Some(joined) = self.joined(p, doc.source(), browse) => {
                    self.roots.insert((p, dir), Some(Some(joined.clone())));
                    joined
                }
                None => {
                    self.roots.insert((p, dir.clone()), None);
                    let over = self.settings.root_override(&PROVIDERS[p], browse, doc.source());
                    resolve(p, dir, doc.source().to_owned(), over, self.settings.clone());
                    continue;
                }
            };
            let session = match self.server(q, &root) {
                Some(s) => s.session,
                None => self.queue(Key { provider: q, root, launch: self.settings.launch(&PROVIDERS[q]) }, doc.source().to_owned()),
            };
            wanted.entry(session).or_default().insert(doc.source().to_owned(), doc.clone());
        }
        self.start_queued(events);
        let now = (self.clock)();
        let roots: Vec<(usize, PathBuf)> = held
            .iter()
            .filter_map(|&(path, language)| {
                let (p, _) = language.and_then(provider::for_language)?;
                self.roots.get(&(p, path.parent()?.to_owned()))?.clone()?
            })
            .collect();
        for s in &mut self.servers {
            s.held = roots.iter().any(|(p, r)| *p == s.key.provider && *r == s.key.root);
            let docs = wanted.remove(&s.session).unwrap_or_default();
            if s.initialized() {
                s.reconcile(docs);
            }
            s.touch(now);
        }
    }

    /// The workspace of a server of `provider` for `file`, when it is a dependency outside the browsing root and
    /// every such server's workspace: a server's project navigated there, and one of the dependency's own (its whole
    /// workspace) would cost as much memory again. The server last asked (whose answer opened it) first, then one
    /// whose workspace holds the browsing root, else the first started; none: the dependency resolves as any file.
    fn joined(&self, provider: usize, file: &Path, browse: Option<&Path>) -> Option<Served> {
        // Under the browsing root (a vendored checkout, a project's node_modules or virtualenv), or not a dependency:
        // its own rules apply.
        if browse.is_some_and(|b| file.starts_with(b)) || !PROVIDERS[provider].is_dependency(file) {
            return None;
        }
        let servers = || self.servers.iter().filter(|s| provider::primary(s.key.provider) == provider);
        if servers().any(|s| file.starts_with(&s.key.root)) {
            return None;
        }
        let asked = |s: &&Server| self.last_asked == Some(s.session);
        let holds = |s: &&Server| browse.is_some_and(|b| b.starts_with(&s.key.root));
        servers().find(asked).or_else(|| servers().find(holds)).or_else(|| servers().next()).map(|s| (s.key.provider, s.key.root.clone()))
    }

    /// Whether `provider` is turned off for this window's browsing root: for every file the window shows, its
    /// dependencies included.
    fn disabled_for(&self, provider: usize, _file: &Path) -> bool {
        self.settings.disabled(&PROVIDERS[provider::primary(provider)], self.browse.as_deref())
    }

    /// Stop servers idle for longer than the idle period (no open documents, nothing pending).
    pub fn expire(&mut self) -> bool {
        let now = (self.clock)();
        let idle = self.idle;
        let waiting = self.waiting.as_ref().map(|w| w.path.clone());
        let (stop, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.servers)
            .into_iter()
            .partition(|s| s.idle_since.is_some_and(|t| now.duration_since(t) >= idle) && !waiting.as_ref().is_some_and(|w| w.starts_with(&s.key.root)));
        self.servers = keep;
        let stopped = !stop.is_empty();
        stop.into_iter().for_each(Server::stop);
        stopped
    }

    /// A new server for `key`, waiting for a startup slot.
    fn queue(&mut self, key: Key, trigger: PathBuf) -> u64 {
        self.next_session += 1;
        let session = self.next_session;
        let (outbox, inbox) = async_channel::bounded(OUTBOX);
        self.servers.push(Server {
            key,
            trigger,
            session,
            status: Status::Queued,
            version: None,
            capabilities: Value::Null,
            encoding: Encoding::Utf16,
            outbox,
            inbox: Some(inbox),
            process: Arc::default(),
            next_id: 0,
            initializing: None,
            open: HashMap::new(), desired: HashMap::new(), written: HashMap::new(),
            flights: HashMap::new(), open_epoch: HashMap::new(), next_text: 0, text_bytes: 0,
            desired_epoch: 0, deferred: HashMap::new(), failed: HashMap::new(), refresh: Default::default(),
            dirty_open: HashMap::new(),
            pending: HashMap::new(),
            progress: HashMap::new(),
            error: None,
            busy: false,
            held: true,
            idle_since: None,
            initialized_at: None,
            memory: None,
            legend: Default::default(),
            asked: Default::default(),
            decor: HashMap::new(),
        });
        self.queued.push_back(session);
        session
    }

    fn start_queued(&mut self, events: &async_channel::Sender<Completion>) {
        while self.servers.iter().filter(|s| s.status == Status::Starting).count() < MAX_STARTING {
            let Some(session) = self.queued.pop_front() else { return };
            let Some(s) = self.servers.iter_mut().find(|s| s.session == session) else { continue };
            let Some(inbox) = s.inbox.take() else { continue };
            s.status = Status::Starting;
            let provider = &PROVIDERS[s.key.provider];
            // Initialize is the first message the server reads, whenever it starts.
            s.next_id += 1;
            s.initializing = Some(s.next_id);
            let name = s.key.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let mut initialize = json!({
                "jsonrpc": "2.0", "id": s.next_id, "method": "initialize",
                "params": {
                    "processId": std::process::id(),
                    "clientInfo": { "name": "lenz-reader" },
                    "rootUri": lsp::file_uri(&s.key.root),
                    "workspaceFolders": [{ "uri": lsp::file_uri(&s.key.root), "name": name }],
                    "initializationOptions": self.settings.initialization(provider, Some(&s.key.root)),
                    "capabilities": {
                        "general": { "positionEncodings": if provider.utf8 { json!(["utf-8", "utf-16"]) } else { json!(["utf-16"]) } },
                        "textDocument": {
                            "synchronization": { "dynamicRegistration": false },
                            "definition": { "linkSupport": true },
                            "references": {},
                            "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
                            "typeHierarchy": { "dynamicRegistration": false },
                            // typescript-language-server offers call hierarchy only to a client that declares it.
                            "callHierarchy": { "dynamicRegistration": false },
                            "inlayHint": { "dynamicRegistration": false },
                            "semanticTokens": {
                                "dynamicRegistration": false, "requests": { "full": true }, "formats": ["relative"],
                                "tokenTypes": crate::decor::TOKEN_TYPES, "tokenModifiers": crate::decor::TOKEN_MODIFIERS,
                                "overlappingTokenSupport": false, "multilineTokenSupport": false,
                            },
                        },
                        "workspace": {
                            "applyEdit": false, "configuration": true, "workspaceFolders": true, "symbol": {},
                            "inlayHint": { "refreshSupport": true }, "semanticTokens": { "refreshSupport": true },
                        },
                        "window": { "workDoneProgress": true },
                        "experimental": (provider.experimental)(),
                    },
                },
            });
            // A folder without project files: JDT LS imports it (an invisible project, kept in its `-data` folder) from
            // the files that triggered it.
            if provider.id == "jdtls" {
                add_trigger_file(&mut initialize["params"]["initializationOptions"], &lsp::file_uri(&s.trigger));
            }
            // Changed files are reported only to a server that relies on the client for them.
            if provider.watched_files {
                initialize["params"]["capabilities"]["workspace"]["didChangeWatchedFiles"] = json!({ "dynamicRegistration": false });
            }
            let _ = s.send(initialize);
            // A server workspace-data folder of its own, per project and launch, in the reader's cache.
            let data = provider.workspace_data.map(|flag| (flag, self.cache.join(provider.id).join(provider::data_folder(&s.key.root, &s.key.launch))));
            spawn_server(s.key.clone(), provider, data, self.cache.join(provider.id), session, inbox, s.process.clone(), events.clone());
        }
    }

    /// Ask `question` (definition or usages) about `offset` of `doc`, already synchronized. A server still
    /// starting (or a root still resolving) keeps the request until it is ready; `Err` says why it cannot be
    /// asked at all.
    pub fn ask(&mut self, doc: &Arc<Document>, offset: usize, request: u64, question: Question) -> Result<(), String> {
        self.cancel_lane(question.lane());
        let Some((p, root)) = self.resolved(doc) else {
            return Err(match doc.language() {
                Some(l) => format!("No semantic provider for {l:?}"),
                None => "No semantic provider for this file".into(),
            });
        };
        let name = PROVIDERS[p].name;
        if doc.is_large() {
            return Err(format!("Large file: {name} is not used for it"));
        }
        if self.disabled_for(p, doc.source()) {
            return Err(format!("{name} is disabled for this folder"));
        }
        let wait = Waiting { request, question, path: doc.source().to_owned(), offset, version: doc.version() };
        // Only the user's own request waits for a server to start; a background one is simply not asked.
        let background = question.lane() != Lane::Foreground;
        let not_ready = || format!("{name} is not ready yet");
        let root = match root {
            Some(Some(root)) => root,
            Some(None) => return Err(format!("{name}: {}", PROVIDERS[p].no_root)),
            None if background => return Err(not_ready()),
            None => {
                self.waiting = Some(wait);
                return Ok(());
            }
        };
        let Some(s) = self.servers.iter_mut().find(|s| s.key.provider == p && s.key.root == root) else {
            if background {
                return Err(not_ready());
            }
            self.waiting = Some(wait);
            return Ok(());
        };
        match &s.status {
            Status::Missing(why) => return Err(format!("{name} is unavailable: {why}")),
            Status::Failed(why) => return Err(format!("{name} stopped: {why}")),
            _ if !s.initialized() && background => return Err(not_ready()),
            _ if !s.initialized() => {
                self.waiting = Some(wait);
                return Ok(());
            }
            _ => {}
        }
        if !s.supports(question) {
            return Err(format!("{name} does not provide {}", question.noun()));
        }
        if !s.desired.get(doc.source()).is_some_and(|d| same_document(d, doc)) {
            return Err(format!("{name} has not been sent this version of the file"));
        }
        let mut params = json!({ "textDocument": { "uri": lsp::file_uri(doc.source()) }, "position": lsp::position(doc, offset, s.encoding) });
        match question {
            Question::Usages => params["context"] = json!({ "includeDeclaration": false }),
            Question::FileSymbols => params = json!({ "textDocument": { "uri": lsp::file_uri(doc.source()) } }),
            Question::IncomingCalls | Question::OutgoingCalls => return Err("a call-hierarchy level is asked about its item".into()),
            Question::Supertypes | Question::Subtypes => return Err("a type-hierarchy level is asked about its item".into()),
            Question::TypeUsages => return Err("type usages are asked at a location".into()),
            Question::Definition | Question::Symbols | Question::Implementation | Question::TypeDefinition | Question::PrepareCallHierarchy | Question::PrepareTypeHierarchy | Question::Hover | Question::Highlight => {}
        }
        if !background {
            self.last_asked = Some(s.session);
        }
        s.request(question, params, request)
    }

    /// Whether server session `session` is running and initialized (can be asked).
    pub fn can_ask(&self, session: u64) -> bool {
        self.servers.iter().any(|s| s.session == session && s.initialized())
    }

    /// Ask server session `session` about one of its call-hierarchy items (`question`: its callers or callees).
    pub fn ask_item(&mut self, session: u64, item: Value, request: u64, question: Question) -> Result<(), String> {
        self.cancel_lane(question.lane());
        let s = self.servers.iter_mut().find(|s| s.session == session && s.initialized()).ok_or("the language server stopped or restarted; run Call Hierarchy again")?;
        if !s.supports(question) {
            return Err(format!("{} does not provide {}", PROVIDERS[s.key.provider].name, question.noun()));
        }
        if question.lane() == Lane::Foreground {
            self.last_asked = Some(session);
        }
        s.request(question, json!({ "item": item }), request)
    }

    /// Ask server session `session` about `position` in `uri` (a file it need not have open): a level of a type
    /// hierarchy built from usages.
    pub fn ask_at(&mut self, session: u64, uri: &str, position: Value, request: u64, question: Question) -> Result<(), String> {
        self.cancel_lane(question.lane());
        let s = self.servers.iter_mut().find(|s| s.session == session && s.initialized()).ok_or("the language server stopped or restarted; run Type Hierarchy again")?;
        if !s.supports(question) {
            return Err(format!("{} does not provide type hierarchy", PROVIDERS[s.key.provider].name));
        }
        let mut params = json!({ "textDocument": { "uri": uri }, "position": position });
        if question == Question::TypeUsages {
            params["context"] = json!({ "includeDeclaration": true });
        }
        if question.lane() == Lane::Foreground {
            self.last_asked = Some(session);
        }
        s.request(question, params, request)
    }

    /// The initialized server holding `doc` at its version: its session.
    pub fn session_of(&self, doc: &Document) -> Option<u64> {
        self.servers.iter().find(|s| s.initialized() && s.open.get(doc.source()).is_some_and(|d| same_document(d, doc))).map(|s| s.session)
    }

    /// Ask every initialized server under browsing root `browse` (all, without one) for workspace symbols
    /// matching `query`. Returns the providers asked; `Err` when none can answer.
    pub fn symbols(&mut self, query: &str, request: u64, browse: Option<&Path>) -> Result<Vec<&'static str>, String> {
        self.cancel();
        let mut asked = Vec::new();
        let mut why = "No language server is running for this folder yet; open one of its source files first".to_string();
        for s in &mut self.servers {
            if !browse.is_none_or(|b| s.key.root.starts_with(b) || b.starts_with(&s.key.root)) || !s.initialized() {
                continue;
            }
            if !s.supports(Question::Symbols) {
                why = format!("{} does not provide workspace symbols", PROVIDERS[s.key.provider].name);
                continue;
            }
            if s.request(Question::Symbols, json!({ "query": query }), request).is_ok() {
                asked.push(PROVIDERS[s.key.provider].name);
            }
        }
        if asked.is_empty() { Err(why) } else { Ok(asked) }
    }

    /// A request waiting for its server, if `doc` is its document as it was asked about (else it is dropped).
    pub fn take_waiting(&mut self, doc: &Document) -> Option<(u64, usize, Question)> {
        let w = self.waiting.take()?;
        (w.path == doc.source() && w.version == doc.version()).then_some((w.request, w.offset, w.question))
    }

    /// The request waiting for its server to start, if any.
    pub fn waiting(&self) -> Option<u64> {
        self.waiting.as_ref().map(|w| w.request)
    }

    /// Stop waiting for or asking about the current request (best effort at the protocol level: replies are
    /// still checked against the current request).
    pub fn cancel(&mut self) {
        self.cancel_lane(Lane::Foreground);
    }

    /// Stop asking about `lane`'s request (for the foreground, also stop waiting for a server to start).
    pub fn cancel_lane(&mut self, lane: Lane) {
        if lane == Lane::Foreground {
            self.waiting = None;
        }
        let now = (self.clock)();
        for s in &mut self.servers {
            s.deferred.remove(&lane);
            if let Some((_, id)) = s.pending.remove(&lane) {
                s.notify("$/cancelRequest", json!({ "id": id }));
            }
            s.touch(now);
        }
    }

    /// Restart the server for `doc`'s workspace (also one that failed or was not found): the old session's
    /// requests and virtual sources become obsolete; documents reopen in the new one.
    pub fn restart(&mut self, doc: &Document) -> bool {
        self.restart_source(doc.source_meta())
    }
    pub(crate) fn restart_source(&mut self, source: &crate::document::SourceMeta) -> bool {
        self.reload();
        let Some((p, Some(Some(root)))) = self.resolved_source(source) else { return false };
        let Some(i) = self.servers.iter().position(|s| s.key.provider == p && s.key.root == root) else { return false };
        let s = self.servers.remove(i);
        self.queued.retain(|q| *q != s.session);
        // Resolved again: which server serves the root may have changed (TypeScript 7 installed or removed).
        let primary = provider::primary(p);
        self.roots.retain(|(q, _), r| !(*q == primary && r.as_ref().is_some_and(|r| r.as_ref().is_some_and(|(_, at)| *at == root))));
        if self.waiting.as_ref().is_some_and(|w| w.path.starts_with(&root)) {
            self.waiting = None;
        }
        s.stop();
        true
    }

    /// Turn `provider` off (or on again) for browsing root `browse`, remembered across restarts. Turning it off
    /// stops its servers there. Returns the settings to save.
    pub fn set_enabled(&mut self, provider: usize, browse: PathBuf, enabled: bool) -> Settings {
        self.reload();
        let disabled = &mut self.settings.providers.entry(PROVIDERS[provider].id.into()).or_default().disabled;
        if enabled {
            disabled.remove(&browse);
        } else {
            disabled.insert(browse.clone());
            // Every server of this provider the window started (dependencies' workspaces too).
            let (stop, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.servers).into_iter().partition(|s| provider::primary(s.key.provider) == provider);
            self.servers = keep;
            let servers = &self.servers;
            self.queued.retain(|q| servers.iter().any(|s| s.session == *q));
            self.waiting = None;
            stop.into_iter().for_each(Server::stop);
        }
        self.settings.clone()
    }

    pub fn reload_runtime(&mut self, provider: &str) {
        self.reload();
        let (stop, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.servers).into_iter().partition(|s| PROVIDERS[s.key.provider].id == provider);
        self.servers = keep;
        self.queued.retain(|id| self.servers.iter().any(|s| s.session == *id));
        self.roots.retain(|(p, _), _| PROVIDERS[*p].id != provider);
        stop.into_iter().for_each(Server::stop);
    }

    /// Reload saved overrides and stop only the affected workspace/provider; other projects keep running.
    pub fn reload_project(&mut self, provider: usize, root: &Path) {
        self.reload();
        let primary = provider::primary(provider);
        if self.waiting.as_ref().is_some_and(|w| w.path.parent().is_some_and(|dir|
            self.roots.get(&(primary, dir.to_owned())).is_some_and(|r|
                r.as_ref().is_some_and(|r| r.as_ref().is_some_and(|(p, at)| *p == provider && at == root))))) {
            self.waiting = None;
        }
        self.roots.retain(|_, r| !r.as_ref().is_some_and(|r| r.as_ref().is_some_and(|(p, at)| *p == provider && at == root)));
        let (stop, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.servers).into_iter().partition(|s| s.key.provider == provider && s.key.root == root);
        self.servers = keep;
        self.queued.retain(|id| self.servers.iter().any(|s| s.session == *id));
        stop.into_iter().for_each(Server::stop);
    }

    /// Process groups of running servers, for memory sampling.
    pub fn groups(&self) -> Vec<(u64, u32)> {
        self.servers.iter().filter_map(|s| Some((s.session, s.process.lock().ok()?.child.as_ref()?.id()))).collect()
    }

    /// Combined resident memory of every server's process group, when sampled.
    pub fn memory(&self) -> u64 {
        self.servers.iter().filter_map(|s| s.memory).map(|(a, b)| a + b).sum()
    }

    /// Whether any server exists (running, starting or failed).
    pub fn running(&self) -> bool {
        !self.servers.is_empty()
    }

    /// What the clock says now.
    pub fn now(&self) -> std::time::Instant {
        (self.clock)()
    }

    /// Re-read the settings file (the user may have edited it) before acting on or saving settings.
    fn reload(&mut self) {
        self.settings = Settings::read(&self.settings_file);
    }

    /// The display name of server session `session`: its provider and project (workspace root's name).
    pub fn origin(&self, session: u64) -> Option<String> {
        let s = self.servers.iter().find(|s| s.session == session)?;
        let root = s.key.root.file_name().map_or_else(|| s.key.root.display().to_string(), |n| n.to_string_lossy().into_owned());
        Some(format!("{} · {root}", PROVIDERS[s.key.provider].name))
    }

    /// Initializing/restarting servers have no settled baseline for content-based watch recovery.
    pub(crate) fn can_filter_changes(&self) -> bool {
        let mut found = false;
        for s in self.servers.iter().filter(|s| PROVIDERS[s.key.provider].watched_files) {
            if s.status != Status::Ready { return false; }
            found = true;
        }
        found
    }

    /// Tell client-watched servers about files under their root: true means changed/present, false deleted.
    /// Ruby LSP also indexes previously unknown files on Changed. Directory events are expanded by the session.
    pub fn files_changed(&mut self, changes: &[(PathBuf, bool)]) {
        for s in self.servers.iter_mut().filter(|s| s.initialized() && PROVIDERS[s.key.provider].watched_files) {
            let events: Vec<Value> = changes.iter().filter(|(p, _)| p.starts_with(&s.key.root)).map(|(p, exists)| json!({ "uri": lsp::file_uri(p), "type": if *exists { 2 } else { 3 } })).collect();
            if !events.is_empty() {
                s.notify("workspace/didChangeWatchedFiles", json!({ "changes": events }));
            }
            for (path, exists) in changes {
                if path.starts_with(&s.key.root) && s.open.contains_key(path) {
                    s.dirty_open.insert(path.clone(), *exists);
                    if *exists { s.refresh_index(path); }
                }
            }
        }
    }

    /// Why no initialized server under browsing root `browse` can be asked for workspace symbols, if none can.
    pub fn symbols_unavailable(&self, browse: Option<&Path>) -> Option<&'static str> {
        let running = || self.servers.iter().filter(|s| s.initialized() && browse.is_none_or(|b| s.key.root.starts_with(b) || b.starts_with(&s.key.root)));
        match running().next() {
            None => Some("No language server is running for this folder yet; open one of its source files"),
            Some(_) if running().any(|s| s.supports(Question::Symbols)) => None,
            Some(_) => Some("The language servers running for this folder do not provide workspace symbols; find declarations instead"),
        }
    }

    pub fn event(&mut self, event: Event, events: &async_channel::Sender<Completion>) -> Outcome {
        let mut outcome = match event {
            Event::Resolved { provider, dir, root, serve } => {
                // One choice per root: a server of the language already there keeps serving it.
                let serve = root.as_ref().and_then(|r| self.servers.iter().find(|s| provider::primary(s.key.provider) == provider && s.key.root == *r)).map_or(serve, |s| s.key.provider);
                self.roots.insert((provider, dir), Some(root.map(|r| (serve, r))));
                Outcome { repaint: true, ready: true, ..Outcome::default() }
            }
            Event::Detected { session, version } => {
                let Some(s) = self.servers.iter_mut().find(|s| s.session == session) else { return Outcome::default() };
                match version {
                    Ok(v) => s.version = Some(v).filter(|v| !v.is_empty()),
                    Err(why) => {
                        s.status = Status::Missing(format!("{why}. {}", PROVIDERS[s.key.provider].setup));
                        s.initializing = None;
                    }
                }
                Outcome { repaint: true, ..Outcome::default() }
            }
            Event::Exited { session, reason } => {
                let Some(s) = self.servers.iter_mut().find(|s| s.session == session) else { return Outcome::default() };
                if !matches!(s.status, Status::Missing(_)) {
                    s.status = Status::Failed(reason);
                }
                s.initializing = None;
                // A failed input writer can leave stdout held by a helper; do not rely on output EOF
                // to stop and reap the owned group.
                s.halt();
                s.open.clear(); s.desired.clear(); s.written.clear(); s.open_epoch.clear();
                s.flights.clear(); s.text_bytes = 0; s.refresh.clear();
                s.asked = Default::default();
                s.decor.clear();
                s.memory = None;
                let name = PROVIDERS[s.key.provider].name;
                // Every request still pending fails, in its own lane.
                let mut replies: Vec<_> = s
                    .pending
                    .drain()
                    .map(|(lane, (request, _))| Reply { lane, request, session, encoding: s.encoding, provider: name, result: Err(format!("{name} stopped before answering")), indexing: false })
                    .collect();
                replies.extend(s.deferred.drain().map(|(lane, d)| Reply { lane, request: d.request, session, encoding: s.encoding, provider: name, result: Err(format!("{name} stopped before answering")), indexing: false }));
                Outcome { repaint: true, replies, ready: false, ..Outcome::default() }
            }
            Event::Written { session, token } => {
                let Some(s) = self.servers.iter_mut().find(|s| s.session == session) else { return Outcome::default() };
                if let Some(frame) = s.flights.remove(&token) {
                    s.text_bytes -= frame.document.text().len();
                    if s.open_epoch.get(&frame.path) == Some(&frame.epoch) { s.written.insert(frame.path, frame.document); }
                    s.pump_text();
                }
                Outcome { ready: true, ..Outcome::default() }
            }
            Event::StartTimeout { session } => {
                let Some(s) = self.servers.iter_mut().find(|s| s.session == session && s.initializing.is_some()) else { return Outcome::default() };
                // It never finished initializing: stop it and say so; its slot goes to the next server.
                s.initializing = None;
                s.status = Status::Failed(format!("did not finish starting within {} s", START_TIMEOUT.as_secs()));
                s.halt();
                Outcome { repaint: true, ..Outcome::default() }
            }
            Event::Decoration { session, id, failed, body } => {
                let Some(s) = self.servers.iter_mut().find(|s| s.session == session) else { return Outcome::default() };
                s.decorated(id, failed, body)
            }
            Event::Message { session, message } => {
                let Some(s) = self.servers.iter_mut().find(|s| s.session == session) else { return Outcome::default() };
                let outcome = s.message(message, &self.settings, (self.clock)(), &mut self.notices);
                s.touch((self.clock)());
                outcome
            }
            // The session handles ticks (expiry and sampling); nothing else sends them here.
            Event::Tick => Outcome::default(),
            Event::Memory(samples) => {
                for (session, server, helpers) in samples {
                    if let Some(s) = self.servers.iter_mut().find(|s| s.session == session) {
                        s.memory = Some((server, helpers));
                    }
                }
                Outcome { repaint: true, ..Outcome::default() }
            }
        };
        for s in &mut self.servers { outcome.replies.extend(s.failed.drain().map(|(_, reply)| reply)); }
        self.start_queued(events);
        outcome
    }

    /// Set (or clear) the workspace root for `provider`'s files under browsing root `browse`. Its servers stop;
    /// documents reopen under the new root on the next sync. Returns the settings to save.
    pub fn set_root(&mut self, provider: usize, browse: PathBuf, root: Option<PathBuf>) -> Settings {
        self.reload();
        let roots = &mut self.settings.providers.entry(PROVIDERS[provider].id.into()).or_default().roots;
        match root {
            Some(r) => roots.insert(browse, r),
            None => roots.remove(&browse),
        };
        self.roots.retain(|(p, _), _| *p != provider);
        let (stop, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.servers).into_iter().partition(|s| provider::primary(s.key.provider) == provider);
        self.servers = keep;
        let servers = &self.servers;
        self.queued.retain(|q| servers.iter().any(|s| s.session == *q));
        stop.into_iter().for_each(Server::stop);
        self.settings.clone()
    }

    pub fn restart_project(&mut self, browse: &Path) {
        let targets: Vec<_> = self.servers.iter().filter(|s| s.key.root.starts_with(browse) || browse.starts_with(&s.key.root))
            .map(|s| (s.key.provider, s.key.root.clone())).collect();
        for (provider, root) in targets { self.reload_project(provider, &root); }
    }

    /// Stop every server now (project closure): `shutdown` and `exit`, then the process group is killed if it
    /// lingers. Nothing waits on the UI thread.
    pub fn stop_all(&mut self) {
        self.waiting = None;
        self.queued.clear();
        self.roots.clear();
        std::mem::take(&mut self.servers).into_iter().for_each(Server::stop);
    }

    /// Ask `source`'s server (its very session) for the text of that virtual source, through the provider's
    /// documented method. `Err` when its session has ended or the provider has none.
    pub fn retrieve(&mut self, source: &VirtualSource, request: u64) -> Result<(), String> {
        let s = self.servers.iter_mut().find(|s| s.session == source.session).ok_or_else(|| format!("the {} session that named it has ended; ask again", source.provider))?;
        let method = PROVIDERS[s.key.provider].virtual_contents.ok_or_else(|| source.unavailable())?;
        if !s.initialized() { return Err(format!("{} is not ready", source.provider)); }
        s.request_method(Lane::Foreground, method, json!({ "uri": &*source.uri }), request)
    }

    /// The encoding of server session `session` (how positions in its answers count).
    pub fn encoding(&self, session: u64) -> Encoding {
        self.servers.iter().find(|s| s.session == session).map_or(Encoding::Utf16, |s| s.encoding)
    }

    /// Whether server session `session` is still this session's (not stopped or replaced).
    pub fn has_session(&self, session: u64) -> bool {
        self.servers.iter().any(|s| s.session == session)
    }

    pub(crate) fn take_failures(&mut self) -> Vec<Reply> {
        self.servers.iter_mut().flat_map(|s| s.failed.drain().map(|(_, reply)| reply)).collect()
    }

    /// The last desired documents, retained while a replacement is being prepared off the UI thread.
    pub(crate) fn desired(&self) -> HashMap<PathBuf, Arc<Document>> {
        self.servers.iter().flat_map(|s| s.desired.iter().map(|(p, d)| (p.clone(), d.clone()))).collect()
    }

    /// The snapshots server session `session` has open: what its positions are about.
    pub fn sent(&self, session: u64) -> HashMap<PathBuf, Arc<Document>> {
        self.servers.iter().find(|s| s.session == session).map(|s| s.written.clone()).unwrap_or_default()
    }

    /// Ask the servers holding `docs` (at their versions) for the decorations `wanted` they offer and were not
    /// asked about these versions.
    pub fn decorate(&mut self, docs: &[Arc<Document>], wanted: &[Decor]) {
        for doc in docs {
            let Some(s) = self.servers.iter_mut().find(|s| s.initialized() && s.open.get(doc.source()).is_some_and(|d| same_document(d, doc))) else { continue };
            for &kind in wanted {
                s.decorate(doc, kind);
            }
        }
    }

    /// The latest request asked about `path` for `kind` (`None`: none is current, e.g. after a refresh).
    pub fn latest(&self, path: &Path, kind: Decor) -> Option<i64> {
        self.servers.iter().find_map(|s| s.asked[kind as usize].get(path).map(|&(_, id, _)| id))
    }

    /// Forget every request about `kind` (its setting changed): pending ones are cancelled; the next sync asks again.
    pub fn forget(&mut self, kind: Decor) {
        for s in &mut self.servers {
            let pending: Vec<i64> = s.decor.iter().filter(|(_, d)| d.2 == kind).map(|(id, _)| *id).collect();
            for id in pending {
                s.decor.remove(&id);
                s.notify("$/cancelRequest", json!({ "id": format!("{}{id}", lsp::DECORATION) }));
            }
            s.asked[kind as usize].clear();
        }
    }

    /// The name of `doc`'s running server when it does not offer `kind` (`None`: it does, or none runs for it).
    pub fn lacks(&self, doc: &Document, kind: Decor) -> Option<&'static str> {
        self.lacks_source(doc.source(), kind)
    }
    pub(crate) fn lacks_source(&self, source: &Path, kind: Decor) -> Option<&'static str> {
        let s = self.servers.iter().find(|s| s.initialized() && s.open.contains_key(source))?;
        (!s.offers(kind)).then_some(PROVIDERS[s.key.provider].name)
    }

    /// Process ids of launched servers.
    pub fn pids(&self) -> Vec<u32> {
        self.servers.iter().filter_map(|s| s.process.lock().ok()?.child.as_ref().map(Child::id)).collect()
    }
}

impl Drop for Semantics {
    fn drop(&mut self) {
        self.stop_all();
    }
}

fn same_document(a: &Document, b: &Document) -> bool {
    a.version() == b.version() && Arc::ptr_eq(a.source_meta(), b.source_meta())
}

impl Server {
    fn view(&self) -> ProviderView {
        let p = &PROVIDERS[self.key.provider];
        ProviderView {
            name: p.name,
            root: Some(self.key.root.clone()),
            status: self.status.clone(),
            version: self.version.clone(),
            // An unknown version is as untested as an unexpected one.
            untested: self.initialized() && self.version.as_ref().is_none_or(|v| p.tested.is_none_or(|t| !provider::is_tested(v, t))),
            limits: p.limits,
            memory: self.memory,
            supports: Question::ALL.into_iter().filter(|q| self.initialized() && self.supports(*q)).collect(),
            error: self.error.clone(),
        }
    }

    /// Whether its capabilities include `question`'s operation.
    fn supports(&self, question: Question) -> bool {
        self.capabilities.get(question.capability()).is_some_and(|v| !v.is_null() && *v != Value::Bool(false))
    }

    /// The answer to decoration request `id`: to decode, unless it failed (then asked again once the server has
    /// loaded) or was cancelled.
    fn decorated(&mut self, id: i64, failed: bool, body: Vec<u8>) -> Outcome {
        let Some((path, version, kind)) = self.decor.remove(&id) else { return Outcome::default() };
        if failed {
            if let Some(asked) = self.asked[kind as usize].get_mut(&path).filter(|(v, ..)| *v == version) {
                asked.2 = true;
            }
            return Outcome::default();
        }
        Outcome { decorations: vec![Decoration { session: self.session, id, path, version, kind, encoding: self.encoding, legend: self.legend.clone(), body }], ..Outcome::default() }
    }

    /// Whether its capabilities include `kind`.
    fn offers(&self, kind: Decor) -> bool {
        self.capabilities.get(kind.capability()).is_some_and(|v| !v.is_null() && *v != Value::Bool(false))
    }

    /// Ask for `kind` about `doc` (open at this version) unless asked already; an older request is cancelled.
    fn decorate(&mut self, doc: &Document, kind: Decor) {
        if !self.offers(kind) || self.text_pending() {
            return;
        }
        match self.asked[kind as usize].get(doc.source()) {
            Some(&(version, ..)) if version == doc.version() => return,
            Some(&(_, id, _)) if self.decor.remove(&id).is_some() => self.notify("$/cancelRequest", json!({ "id": format!("{}{id}", lsp::DECORATION) })),
            _ => {}
        }
        let uri = lsp::file_uri(doc.source());
        let params = match kind {
            Decor::Hints => json!({ "textDocument": { "uri": uri }, "range": { "start": { "line": 0, "character": 0 }, "end": lsp::position(doc, doc.text().len(), self.encoding) } }),
            Decor::Tokens => json!({ "textDocument": { "uri": uri } }),
        };
        self.next_id += 1;
        let id = self.next_id;
        if self.send(json!({ "jsonrpc": "2.0", "id": format!("{}{id}", lsp::DECORATION), "method": kind.method(), "params": params })).is_ok() {
            let early = self.status != Status::Ready;
            self.asked[kind as usize].insert(doc.source().to_owned(), (doc.version(), id, early));
            self.decor.insert(id, (doc.source().to_owned(), doc.version(), kind));
            self.idle_since = None;
        }
    }

    /// Send `question`'s request for session request `request`.
    fn request(&mut self, question: Question, params: Value, request: u64) -> Result<(), String> {
        self.request_method(question.lane(), question.method(), params, request)
    }

    fn request_method(&mut self, lane: Lane, method: &'static str, params: Value, request: u64) -> Result<(), String> {
        // Workspace and hierarchy questions also depend on preceding cross-file updates. Preserve the
        // old FIFO barrier while coalescing text, with one replaceable intent per existing lane.
        if self.text_pending() {
            self.deferred.insert(lane, Deferred { method, params, request, epoch: self.desired_epoch });
            self.idle_since = None;
            return Ok(());
        }
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        self.pending.insert(lane, (request, id));
        self.idle_since = None;
        Ok(())
    }

    /// Idle when no tab holds its documents and nothing is pending, from the first moment that is so.
    fn touch(&mut self, now: std::time::Instant) {
        let idle = !self.held && self.open.is_empty() && self.pending.is_empty() && self.deferred.is_empty() && self.flights.is_empty() && self.initializing.is_none();
        self.idle_since = if idle { Some(self.idle_since.unwrap_or(now)) } else { None };
    }

    fn initialized(&self) -> bool {
        self.initializing.is_none() && matches!(self.status, Status::Indexing(_) | Status::Ready)
    }

    fn send(&self, message: Value) -> Result<(), String> {
        self.queue(Outgoing::Message(message))
    }

    fn queue(&self, message: Outgoing) -> Result<(), String> {
        self.outbox.try_send(message).map_err(|_| format!("{} is not reading its input", PROVIDERS[self.key.provider].name))
    }

    fn notify(&self, method: &str, params: Value) {
        let _ = self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    fn refresh_index(&mut self, path: &Path) {
        // Ruby LSP indexes open-file changes during document requests; watcher reports alone are ignored.
        if PROVIDERS[self.key.provider].id != "ruby-lsp" || !self.supports(Question::FileSymbols) { return; }
        if self.text_pending() { self.refresh.insert(path.to_owned()); return; }
        self.next_id += 1;
        // No pending lane: the reply is intentionally discarded and cannot replace a user's navigation.
        if let Err(why) = self.send(json!({ "jsonrpc": "2.0", "id": self.next_id, "method": Question::FileSymbols.method(), "params": { "textDocument": { "uri": lsp::file_uri(path) } } })) {
            self.status = Status::Failed(why);
            self.halt();
        }
    }

    /// Open, update or close documents so the server eventually holds exactly the live desired set.
    fn reconcile(&mut self, docs: HashMap<PathBuf, Arc<Document>>) {
        if docs.len() != self.desired.len() || docs.iter().any(|(p, d)| self.desired.get(p).is_none_or(|old| !same_document(old, d))) {
            self.desired_epoch += 1;
            let deferred = std::mem::take(&mut self.deferred);
            for (lane, d) in deferred { self.failed.insert(lane, self.failed_reply(lane, d.request, "the source changed before the request was sent")); }
        }
        let gone: Vec<PathBuf> = self.open.keys().filter(|p| !docs.contains_key(*p)).cloned().collect();
        for path in gone {
            self.open.remove(&path);
            self.written.remove(&path);
            self.open_epoch.remove(&path);
            self.refresh.remove(&path);
            self.asked.iter_mut().for_each(|a| { a.remove(&path); });
            self.decor.retain(|_, d| d.0 != path);
            // The close stays after any in-flight old text and before a future reopen. Its credit
            // remains outstanding, including when the same URI returns before the acknowledgement.
            self.notify("textDocument/didClose", json!({ "textDocument": { "uri": lsp::file_uri(&path) } }));
            if let Some(exists) = self.dirty_open.remove(&path) {
                self.notify("workspace/didChangeWatchedFiles", json!({ "changes": [{ "uri": lsp::file_uri(&path), "type": if exists { 2 } else { 3 } }] }));
            }
        }
        self.desired = docs;
        self.refresh.retain(|p| self.desired.contains_key(p));
        self.pump_text();
    }

    fn text_pending(&self) -> bool {
        self.desired.iter().any(|(path, doc)| self.open.get(path).is_none_or(|sent| !same_document(sent, doc)))
    }

    fn failed_reply(&self, lane: Lane, request: u64, why: &str) -> Reply {
        Reply { lane, request, session: self.session, encoding: self.encoding, provider: PROVIDERS[self.key.provider].name, result: Err(why.to_owned()), indexing: false }
    }

    fn pump_text(&mut self) {
        if !self.initialized() { return; }
        let wanted: Vec<_> = self.desired.iter().filter(|(path, doc)| self.open.get(*path).is_none_or(|sent| !same_document(sent, doc)))
            .map(|(path, doc)| (path.clone(), doc.clone())).collect();
        for (path, doc) in wanted {
            if self.flights.len() == TEXT_FRAMES { break; }
            if self.flights.values().any(|f| f.path == path) || doc.text().len() > TEXT_BYTES.saturating_sub(self.text_bytes) { continue; }
            self.next_text += 1;
            let token = self.next_text;
            let language = (!self.open.contains_key(&path)).then(|| doc.language().map_or("plaintext", |l| (PROVIDERS[self.key.provider].language_id)(l, &path)));
            let epoch = *self.open_epoch.entry(path.clone()).or_insert(token);
            if let Err(why) = self.queue(Outgoing::Text { token, uri: lsp::file_uri(&path), language, version: doc.version(), text: doc.text().clone() }) {
                self.status = Status::Failed(why);
                self.halt();
                return;
            }
            self.text_bytes += doc.text().len();
            self.flights.insert(token, TextFlight { path: path.clone(), document: doc.clone(), epoch });
            self.open.insert(path.clone(), doc);
            if language.is_none() && PROVIDERS[self.key.provider].watched_files {
                self.dirty_open.insert(path.clone(), true);
                self.refresh.insert(path);
            }
        }
        if self.text_pending() { return; }
        for (lane, d) in std::mem::take(&mut self.deferred) {
            if d.epoch != self.desired_epoch { self.failed.insert(lane, self.failed_reply(lane, d.request, "the source changed before the request was sent")); }
            else if let Err(why) = self.request_method(lane, d.method, d.params, d.request) { self.failed.insert(lane, self.failed_reply(lane, d.request, &why)); }
        }
        for path in std::mem::take(&mut self.refresh) { self.refresh_index(&path); }
    }

    fn message(&mut self, m: Value, settings: &Settings, now: std::time::Instant, notices: &mut Vec<String>) -> Outcome {
        let name = PROVIDERS[self.key.provider].name;
        match (m.get("method").and_then(Value::as_str), m.get("id").cloned()) {
            (Some(method), Some(id)) => {
                // A request from the server: the reader never applies edits; configuration is the user's settings.
                let result = match method {
                    "workspace/applyEdit" => {
                        notices.push(format!("{name} asked to edit files; declined: the reader never writes to the project"));
                        Ok(json!({ "applied": false, "failureReason": "The reader is read-only and never edits files" }))
                    }
                    "workspace/configuration" => {
                        let provider = &PROVIDERS[self.key.provider];
                        let items = m["params"]["items"].as_array().map_or(&[][..], Vec::as_slice);
                        Ok(Value::Array(items.iter().map(|i| settings.configuration(provider, Some(&self.key.root), i["section"].as_str())).collect()))
                    }
                    "window/workDoneProgress/create" | "client/registerCapability" | "client/unregisterCapability" | "window/showMessageRequest" => Ok(Value::Null),
                    // Its results changed: forget what was asked; the next sync asks again.
                    "workspace/inlayHint/refresh" => {
                        self.asked[Decor::Hints as usize].clear();
                        self.decor.retain(|_, d| d.2 != Decor::Hints); // their answers are superseded
                        Ok(Value::Null)
                    }
                    "workspace/semanticTokens/refresh" => {
                        self.asked[Decor::Tokens as usize].clear();
                        self.decor.retain(|_, d| d.2 != Decor::Tokens);
                        Ok(Value::Null)
                    }
                    "workspace/workspaceFolders" => Ok(json!([{ "uri": lsp::file_uri(&self.key.root), "name": "" }])),
                    _ => Err(json!({ "code": -32601, "message": format!("{method} is not supported by the reader") })),
                };
                let _ = self.send(match result {
                    Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
                    Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": e }),
                });
                Outcome::default()
            }
            (Some(method), None) => self.notification(method, &m["params"], notices),
            (None, Some(id)) => {
                let id = id.as_i64();
                if id.is_some() && id == self.initializing {
                    self.initializing = None;
                    if let Some(e) = m.get("error") {
                        self.status = Status::Failed(format!("initialization failed: {}", e["message"].as_str().unwrap_or("unknown error")));
                        return Outcome { repaint: true, ..Outcome::default() };
                    }
                    self.capabilities = m["result"]["capabilities"].clone();
                    self.legend = Arc::new(Legend::new(&self.capabilities["semanticTokensProvider"]["legend"]));
                    self.initialized_at = Some(now);
                    if self.version.is_none() {
                        // Its own name and version, for providers without a version flag.
                        let info = &m["result"]["serverInfo"];
                        self.version = info["version"].as_str().map(|v| match info["name"].as_str() {
                            Some(n) => format!("{n} {v}"),
                            None => v.to_owned(),
                        });
                    }
                    self.encoding = Encoding::negotiated(&self.capabilities);
                    self.notify("initialized", json!({}));
                    // A server that reports its own status is busy until it first says otherwise.
                    self.busy = PROVIDERS[self.key.provider].reports_status;
                    self.status = Status::Indexing(String::new());
                    self.update_status();
                    return Outcome { repaint: true, ready: true, ..Outcome::default() };
                }
                let Some(lane) = self.pending.iter().find(|(_, (_, p))| Some(*p) == id).map(|(l, _)| *l) else { return Outcome::default() };
                let Some((request, _)) = self.pending.remove(&lane) else { return Outcome::default() };
                let result = match m.get("error") {
                    Some(e) => Err(format!("{name} failed: {}", e["message"].as_str().unwrap_or("unknown error"))),
                    None => Ok(m.get("result").cloned().unwrap_or(Value::Null)),
                };
                // Still indexing, or (for a server that never says) just started.
                let no_status = !PROVIDERS[self.key.provider].reports_status;
                let indexing = matches!(self.status, Status::Indexing(_)) || no_status && self.initialized_at.is_some_and(|t| now.duration_since(t) < UNREPORTED_WARMUP);
                Outcome { repaint: true, replies: vec![Reply { lane, request, session: self.session, encoding: self.encoding, provider: name, result, indexing }], ready: false, ..Outcome::default() }
            }
            (None, None) => Outcome::default(),
        }
    }

    fn notification(&mut self, method: &str, params: &Value, notices: &mut Vec<String>) -> Outcome {
        match method {
            "$/progress" => {
                let token = match &params["token"] {
                    Value::String(s) => s.clone(),
                    t => t.to_string(),
                };
                let value = &params["value"];
                match value["kind"].as_str() {
                    Some("end") => {
                        self.progress.remove(&token);
                    }
                    Some("begin") => {
                        let mut progress = Progress { title: value["title"].as_str().unwrap_or_default().to_owned(), ..Progress::default() };
                        progress.update(value);
                        self.progress.insert(token, progress);
                    }
                    Some("report") => {
                        if let Some(progress) = self.progress.get_mut(&token) { progress.update(value); }
                    }
                    _ => return Outcome::default(),
                }
            }
            "experimental/serverStatus" => self.busy = params["quiescent"] == Value::Bool(false),
            // JDT LS: `Starting` … `ServiceReady`; `Error` says what went wrong.
            "language/status" => match params["type"].as_str() {
                Some("ServiceReady" | "Started") => self.busy = false,
                Some("Starting") => self.busy = true,
                Some("Error") => {
                    notices.push(format!("{}: {}", PROVIDERS[self.key.provider].name, params["message"].as_str().unwrap_or_default()));
                    return Outcome { repaint: true, ..Outcome::default() };
                }
                _ => return Outcome::default(),
            },
            // An error it reports (Ruby LSP 0.26.11 logs a failed `initialized` and keeps running): kept to be shown.
            "window/logMessage" if params["type"].as_u64() == Some(1) => {
                self.error = Some(error_text(params["message"].as_str().unwrap_or_default()));
                return Outcome { repaint: true, ..Outcome::default() };
            }
            "window/showMessage" if params["type"].as_u64().is_some_and(|t| t <= 2) => {
                if params["type"].as_u64() == Some(1) {
                    self.error = Some(error_text(params["message"].as_str().unwrap_or_default()));
                }
                notices.push(format!("{}: {}", PROVIDERS[self.key.provider].name, params["message"].as_str().unwrap_or_default()));
                return Outcome { repaint: true, ..Outcome::default() };
            }
            _ => return Outcome::default(),
        }
        let before = self.status.clone();
        self.update_status();
        if matches!(before, Status::Indexing(_)) && self.status == Status::Ready {
            // What was asked while loading may be empty (rust-analyzer) or refused (`ContentModified`): asked again
            // (the answer still to come is dropped).
            for a in &mut self.asked {
                let decor = &mut self.decor;
                a.retain(|_, (_, id, early)| {
                    if *early {
                        decor.remove(id); // superseded by the answer asked for next
                    }
                    !*early
                });
            }
        }
        Outcome { repaint: before != self.status, ..Outcome::default() }
    }

    /// Initialization is not readiness: reported work in progress (or rust-analyzer's own status) keeps it
    /// indexing.
    fn update_status(&mut self) {
        if !self.initialized() {
            return;
        }
        self.status = match (self.progress.iter().min_by(|a, b| a.0.cmp(b.0)), self.busy) {
            (Some((_, w)), _) => Status::Indexing(w.summary()),
            (None, true) => Status::Indexing("loading the workspace".into()),
            (None, false) => Status::Ready,
        };
    }

    fn stop(self) {
        self.halt();
    }

    /// `shutdown` and `exit`, then (off the UI thread) wait briefly and kill the process group if it lingers.
    fn halt(&self) {
        let _ = self.send(json!({ "jsonrpc": "2.0", "id": self.next_id + 1, "method": "shutdown" }));
        self.notify("exit", Value::Null);
        self.outbox.close(); // the writer sends what is queued, then closes the server's input
        let process = self.process.clone();
        if let Ok(mut p) = process.lock() {
            p.stopped = true; // a launch still in progress kills what it starts; its output reader leaves it to us
            if p.child.is_none() {
                return;
            }
        }
        let _ = std::thread::Builder::new().name("reader-server-stop".into()).spawn(move || {
            let deadline = std::time::Instant::now() + STOP_GRACE;
            // Its output closing means it exited (unreaped, so its id is still reserved) or is about to.
            while std::time::Instant::now() < deadline && process.lock().is_ok_and(|p| !p.closed && p.child.is_some()) {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let child = process.lock().ok().and_then(|mut p| p.child.take());
            if let Some(mut child) = child {
                kill_group(&mut child); // build helpers it started may outlive it
                let _ = child.wait();
            }
        });
    }
}

/// Kill the server and anything it started in its process group (build helpers, proc-macro servers).
fn kill_group(child: &mut Child) {
    #[cfg(unix)]
    {
        let _ = Command::new("kill").args(["-KILL", &format!("-{}", child.id())]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    let _ = child.kill();
}

/// A server's error message as shown: its first lines, displayable and bounded.
fn error_text(message: &str) -> String {
    let mut text = message.lines().take(12).map(|l| lenz_content::display::display_line(l).0).collect::<Vec<_>>().join("\n");
    let mut end = text.len().min(2000);
    while !text.is_char_boundary(end) { end -= 1; }
    if end < text.len() { text.truncate(end); text.push('…'); }
    text
}

/// Keep the start and end of stderr, including unterminated lines, within 8 KiB of raw bytes.
#[derive(Default)]
struct Stderr {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    omitted: usize,
}

impl Stderr {
    fn push(&mut self, bytes: &[u8]) {
        const HALF: usize = 4096;
        let first = bytes.len().min(HALF - self.head.len());
        self.head.extend_from_slice(&bytes[..first]);
        let bytes = &bytes[first..];
        let overflow = (self.tail.len() + bytes.len()).saturating_sub(HALF);
        self.omitted = self.omitted.saturating_add(overflow);
        self.tail.drain(..overflow.min(self.tail.len()));
        self.tail.extend(&bytes[bytes.len().saturating_sub(HALF)..]);
    }

    fn summary(&self) -> String {
        let mut bytes = self.head.clone();
        if self.omitted > 0 {
            bytes.extend_from_slice(format!(" | [stderr truncated: {} bytes omitted] | ", self.omitted).as_bytes());
        }
        bytes.extend(self.tail.iter());
        String::from_utf8_lossy(&bytes).lines().map(str::trim).filter(|s| !s.is_empty())
            .map(|line| lenz_content::display::expand_tabs(line).0).collect::<Vec<_>>().join(" | ")
    }
}

/// Runtime evidence is gathered only after a failure, with the launch's own executable search path.
fn runtime_failure(key: &Key, provider: &Provider, exe: &Path) -> String {
    let (label, flag, runtime) = match provider.id {
        "pyright" => ("Node.js", "--version", provider::find_executable(Path::new("node"), key.launch.path())),
        "jdtls" => {
            let home = key.launch.env.iter().find(|(k, _)| k == "JAVA_HOME").map(|(_, v)| std::ffi::OsString::from(v))
                .or_else(|| std::env::var_os("JAVA_HOME"));
            let java = java::configured(&key.launch.args)
                .or_else(|| home.map(|home| PathBuf::from(home).join("bin").join(if cfg!(windows) { "java.exe" } else { "java" })))
                .map(|p| provider::find_executable(&p, key.launch.path()).unwrap_or(p))
                .or_else(|| provider::find_executable(Path::new("java"), key.launch.path()));
            ("Java", "-version", java)
        }
        _ => return String::new(),
    };
    let details = match runtime {
        Some(path) => {
            let version = runtime::version_in(&path, flag, &key.launch.env, Some(&key.root)).unwrap_or_else(|e| format!("version unavailable: {e}"));
            let version = version.lines().collect::<Vec<_>>().join(" | ");
            format!("; {label}: {} ({version})", path.display())
        }
        None => format!("; {label}: executable not found on the launch PATH"),
    };
    let requirement = (provider.id == "pyright").then(|| exe.canonicalize().ok()).flatten().and_then(|real| {
        real.ancestors().skip(1).take(4).find_map(|dir| {
            let json: Value = serde_json::from_slice(&lenz_content::theme::read_bounded(&dir.join("package.json")).ok()?).ok()?;
            (json["name"] == "pyright").then(|| json["engines"]["node"].as_str().map(str::to_owned)).flatten()
        })
    });
    // Settings, executable paths and manifests are external text too.
    lenz_content::display::expand_tabs(&format!("{details}{}", requirement.map(|r| format!("; Pyright package engines.node: {r}")).unwrap_or_default())).0
}

/// Launch a server off the UI thread: check `--version`, start the process in its own process group, then pump
/// its input from `inbox` and its output into `events` until it exits.
#[allow(clippy::too_many_arguments)]
fn spawn_server(key: Key, provider: &'static Provider, data: Option<(&'static str, PathBuf)>, cache: PathBuf, session: u64, inbox: async_channel::Receiver<Outgoing>, process: Arc<Mutex<Process>>, events: async_channel::Sender<Completion>) {
    let send = move |e: Event| events.send_blocking(Completion::Semantic(e)).is_ok();
    let _ = std::thread::Builder::new().name(format!("reader-{}", provider.id)).spawn(move || {
        // Not on PATH and not named in the settings: the server an installed VS Code ships, run with `node`.
        let mut args: Vec<String> = provider.fixed_args.iter().map(|a| a.to_string()).chain(key.launch.args.iter().cloned()).collect();
        let mut bundled_version = None;
        let fallback = provider.bundled.filter(|_| key.launch.executable == Path::new(provider.executable)).and_then(|s| provider::bundled_server(&provider::VSCODE_EXTENSIONS, s));
        // Its own lookup for this workspace (typescript-go: the project's tsc) when the settings name none.
        let locate = provider.locate.filter(|_| key.launch.executable == Path::new(provider.executable));
        let found = match locate {
            // Its own lookup decides (it searches PATH too, for a version it can run).
            Some(locate) => locate(&key.root, &key.launch),
            None => provider::find_executable(&key.launch.executable, key.launch.path()),
        };
        if locate.is_some() && found.is_none() {
            send(Event::Detected { session, version: Err(format!("no `{}` it can run was found in the project's node_modules/.bin or on PATH", provider.executable)) });
            return;
        }
        let found = found.or_else(|| {
            let (script, version) = fallback.clone()?;
            let node = provider::find_executable(Path::new("node"), key.launch.path())?;
            args.insert(0, script.display().to_string());
            bundled_version = Some(version);
            Some(node)
        });
        if found.is_none()
            && let Some((script, _)) = &fallback
        {
            send(Event::Detected { session, version: Err(format!("VS Code's built-in server ({}) needs Node.js, and `node` was not found on PATH (set `env.PATH` for it in providers.json if your shell adds it)", script.display())) });
            return;
        }
        let Some(exe) = found else {
            let path = key.launch.path().map_or_else(|| std::env::var_os("PATH").unwrap_or_default(), |p| p.to_owned());
            let searched = std::env::split_paths(&path).count();
            send(Event::Detected {
                session,
                version: Err(format!(
                    "`{}` was not found on PATH ({searched} directories searched; the reader does not read shell startup files, so set `executable` or `env.PATH` for it in providers.json if your shell adds it)",
                    key.launch.executable.display()
                )),
            });
            return;
        };
        // The version, as the provider says it can be found (else its `initialize` answer names it).
        // A version flag that fails means it cannot run; a version that cannot be read is only unknown.
        let version = match provider.version {
            _ if let Some(v) = &bundled_version => Ok(if v.is_empty() { String::new() } else { format!("{v} (its built-in server)") }),
            provider::Version::Flag(args) => runtime::version_args_in(&exe, args, &key.launch.env, Some(&key.root))
                .map(|v| v.lines().next().unwrap_or_default().trim().to_owned())
                .map_err(|e| lenz_content::display::expand_tabs(&format!("`{} {}` failed: {}", exe.display(), args.join(" "), e.lines().collect::<Vec<_>>().join(" | "))).0),
            provider::Version::Package(name) => Ok(provider::package_version(&exe, name).map(|v| format!("{name} {v}")).unwrap_or_default()),
            provider::Version::Initialize => Ok(String::new()),
        };
        let ok = version.is_ok();
        if !send(Event::Detected { session, version }) || !ok {
            return;
        }
        let mut command = Command::new(&exe);
        command.args(&args);
        if let Some((flag, folder)) = &data {
            let _ = std::fs::create_dir_all(folder);
            command.arg(flag).arg(folder);
        }
        // The provider's own variables for this root first; the user's `env` wins.
        let root_env = match (provider.root_env)(&key.root, &cache, &exe, &key.launch.env) {
            Ok(env) => env,
            Err(reason) => {
                send(Event::Exited { session, reason: lenz_content::display::expand_tabs(&reason).0 });
                return;
            }
        };
        command.envs(root_env).envs(key.launch.env.iter().map(|(k, v)| (k, v))).current_dir(&key.root).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        let mut child = match command.spawn() {
            Ok(c) => c,
            Err(e) => {
                let reason = format!("could not start `{}`: {e}{}", exe.display(), runtime_failure(&key, provider, &exe));
                send(Event::Exited { session, reason: lenz_content::display::expand_tabs(&reason).0 });
                return;
            }
        };
        let (stdin, stdout, stderr) = (child.stdin.take(), child.stdout.take(), child.stderr.take());
        {
            let Ok(mut p) = process.lock() else { return };
            if p.stopped {
                kill_group(&mut child);
                let _ = child.wait();
                return;
            }
            p.child = Some(child);
        }
        {
            let send = send.clone();
            let _ = std::thread::Builder::new().name(format!("reader-{}-timer", provider.id)).spawn(move || {
                std::thread::sleep(START_TIMEOUT); // ignored once it is initialized
                send(Event::StartTimeout { session });
            });
        }
        let stderr_log: Arc<Mutex<Stderr>> = Arc::default();
        let (stderr_done, stderr_eof) = std::sync::mpsc::sync_channel(1);
        let stderr_eof = Arc::new(Mutex::new(stderr_eof));
        if let Some(stderr) = stderr {
            let stderr_log = stderr_log.clone();
            let _ = std::thread::Builder::new().name(format!("reader-{}-err", provider.id)).spawn(move || {
                use std::io::Read;
                let mut stderr = stderr;
                let mut chunk = [0; 2048];
                loop {
                    match stderr.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => if let Ok(mut log) = stderr_log.lock() { log.push(&chunk[..n]); },
                    }
                }
                let _ = stderr_done.send(());
            });
        }
        if let Some(mut stdin) = stdin {
            let send = send.clone();
            let (process, key, exe, stderr_log, stderr_eof) = (process.clone(), key.clone(), exe.clone(), stderr_log.clone(), stderr_eof.clone());
            let _ = std::thread::Builder::new().name(format!("reader-{}-in", provider.id)).spawn(move || {
                while let Ok(m) = inbox.recv_blocking() {
                    let token = match &m { Outgoing::Text { token, .. } => Some(*token), Outgoing::Message(_) => None };
                    if let Err(error) = m.write(&mut stdin) {
                        inbox.close();
                        while inbox.try_recv().is_ok() {}
                        // Whichever I/O thread takes the child owns shutdown and its final details.
                        // This also terminates a helper holding stdout open after stdin fails.
                        let child = process.lock().ok().and_then(|mut p| { p.closed = true; if p.stopped { None } else { p.child.take() } });
                        if let Some(child) = child {
                            let details = finish_server(child, &key, provider, &exe, &stderr_log, &stderr_eof);
                            send(Event::Exited { session, reason: format!("could not write to server: {error}; {details}") });
                        }
                        return;
                    }
                    // Publish before advancing: any reply to the following request is enqueued after
                    // this acknowledgement and therefore sees the exact last-written snapshot.
                    if let Some(token) = token && !send(Event::Written { session, token }) { return; }
                }
            });
        }
        let Some(stdout) = stdout else { return };
        let mut reader = BufReader::new(stdout);
        while let Some(body) = lsp::read_frame(&mut reader) {
            // Decoration answers stay bytes (they can be megabytes of JSON); everything else is parsed here.
            let event = match lsp::decoration_id(&body) {
                Some((id, failed)) => Event::Decoration { session, id, failed, body },
                None => match serde_json::from_slice(&body) {
                    Ok(message) => Event::Message { session, message },
                    Err(_) => break, // as before: a malformed message ends the connection
                },
            };
            if !send(event) {
                break; // the session is gone: its stopper takes the process
            }
        }
        // End of output: it exited, or is unusable. Unless it is being stopped (the stopper reaps it), take it:
        // kill what is left of its group, then reap it for its exit status.
        let child = process.lock().ok().and_then(|mut p| {
            p.closed = true;
            if p.stopped { None } else { p.child.take() }
        });
        let Some(child) = child else { return };
        let reason = finish_server(child, &key, provider, &exe, &stderr_log, &stderr_eof);
        send(Event::Exited { session, reason });
    });
}


/// The I/O thread that owns the child preserves exit status and bounded stderr evidence while reaping it.
fn finish_server(mut child: Child, key: &Key, provider: &Provider, exe: &Path, stderr_log: &Mutex<Stderr>, stderr_eof: &Mutex<std::sync::mpsc::Receiver<()>>) -> String {
    kill_group(&mut child);
    let status = child.wait().map_or_else(|e| e.to_string(), |s| s.to_string());
    // A descendant can inherit stderr: never wait indefinitely for its EOF.
    if let Ok(eof) = stderr_eof.lock() { let _ = eof.recv_timeout(std::time::Duration::from_millis(100)); }
    let stderr = stderr_log.lock().map(|log| log.summary()).unwrap_or_default();
    let reason = format!("`{}` exited ({status}){}{}", exe.display(), if stderr.is_empty() { String::new() } else { format!(": {stderr}") }, runtime_failure(key, provider, exe));
    lenz_content::display::expand_tabs(&reason).0
}


/// A definition target in a file, read and located: the snapshot it was located in (with its stamp when read
/// from disk), and the result row.
pub struct Target {
    pub path: PathBuf,
    pub stamp: Option<crate::cache::Stamp>,
    pub doc: Arc<Document>,
    pub hit: crate::search::Hit,
    /// Where it starts in `doc` (the row's snippet stops at [`crate::search::SNIPPET_CHARS`] characters).
    pub at: usize,
}

/// What a definition reply named.
#[derive(Default)]
pub struct Targets {
    pub found: Vec<Target>,
    /// Targets outside the file system: shown only through their provider's retrieval method.
    pub unavailable: Vec<VirtualTarget>,
    /// Targets whose files could not be read or whose positions do not exist there.
    pub failed: Vec<String>,
    /// More targets than the limit: the rest are not shown.
    pub limited: bool,
}

/// A target file's snapshot, and its stamp when it was read from disk.
type Snapshot = (Option<crate::cache::Stamp>, Arc<Document>);

/// Reads a file target from disk: its stamp and snapshot.
pub type ReadFile<'a> = &'a dyn Fn(&Path) -> Result<(crate::cache::Stamp, Arc<Document>), String>;

/// Locate the targets of a reply: locations, location links or symbols (a symbol without a range is its
/// file's start), at most `limit`. Files are taken from `open` (the snapshots the server was sent) or read once
/// each by `read`; positions convert in `encoding`.
pub fn targets(reply: &Value, limit: usize, encoding: Encoding, provider: &'static str, session: u64, open: &HashMap<PathBuf, Arc<Document>>, read: ReadFile) -> Targets {
    targets_while(reply, limit, encoding, provider, session, open, read, &|| true)
}

#[expect(clippy::too_many_arguments, reason = "the existing target resolver plus its admission cancellation predicate")]
pub(crate) fn targets_while(reply: &Value, limit: usize, encoding: Encoding, provider: &'static str, session: u64, open: &HashMap<PathBuf, Arc<Document>>, read: ReadFile, current: &dyn Fn() -> bool) -> Targets {
    let start = json!({ "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } });
    let mut files: HashMap<PathBuf, Result<Snapshot, String>> = HashMap::new();
    let items = match reply {
        Value::Array(items) => items.as_slice(),
        Value::Object(_) => std::slice::from_ref(reply),
        _ => &[],
    };
    let mut out = Targets::default();
    let mut seen = std::collections::HashSet::new();
    for item in items {
        if !current() { break; }
        // A link's selection range is the name; its full range is the whole declaration. A symbol has a location.
        let (uri, range) = match (item.get("targetUri"), item.get("location")) {
            (Some(uri), _) => (uri, item.get("targetSelectionRange").or(item.get("targetRange"))),
            (None, Some(l)) => (&l["uri"], l.get("range").or(Some(&start))),
            (None, None) => (&item["uri"], item.get("range")),
        };
        let (Some(uri), Some(range)) = (uri.as_str(), range) else { continue };
        if !seen.insert((uri.to_owned(), range["start"].to_string())) {
            continue;
        }
        if out.found.len() + out.unavailable.len() + out.failed.len() == limit {
            out.limited = true;
            break;
        }
        let Some(path) = lsp::uri_path(uri) else {
            out.unavailable.push(VirtualTarget { source: VirtualSource { uri: uri.into(), provider, session }, range: range.clone() });
            continue;
        };
        let file = files.entry(path.clone()).or_insert_with(|| match open.get(&path) {
            Some(doc) => Ok((None, doc.clone())),
            None => read(&path).map(|(stamp, doc)| (Some(stamp), doc)),
        });
        let (stamp, doc) = match file {
            Ok((stamp, doc)) => (*stamp, doc.clone()),
            Err(e) => {
                out.failed.push(format!("{}: {e}", path.display()));
                continue;
            }
        };
        let (Some(start), Some(end)) = (lsp::offset(&doc, &range["start"], encoding), lsp::offset(&doc, &range["end"], encoding)) else {
            out.failed.push(format!("{}: the position is not in the file as read", path.display()));
            continue;
        };
        let hit = hit_at(&path, &doc, start, end);
        out.found.push(Target { path, stamp, doc, hit, at: start });
    }
    out
}

/// The result row for bytes `start..end` of `doc` (read from `path`): its line, the range within the whole line (as
/// text search gives it; it may lie past the shown text), the line's text.
pub fn hit_at(path: &Path, doc: &Document, start: usize, end: usize) -> crate::search::Hit {
    let line = doc.line_of(start);
    let bounds = doc.line_range(line);
    let text: Box<str> = doc.line(line).chars().take(crate::search::SNIPPET_CHARS).collect::<String>().into();
    let range = start - bounds.start..end.clamp(start, bounds.end) - bounds.start;
    crate::search::Hit { path: path.to_string_lossy().into(), line, range, text, decl: None, origin: None }
}

/// Resident bytes of each server session: the server process itself (whose id is its group's), and the other
/// members of its process group (the build helpers and proc-macro servers it started), from `ps` (Linux and
/// macOS; on macOS resident size, not physical footprint). Groups `ps` does not list are left out.
pub fn sample_memory(groups: &[(u64, u32)]) -> Vec<(u64, u64, u64)> {
    let Ok(out) = Command::new("ps").args(["-A", "-o", "pid=,pgid=,rss="]).stdin(Stdio::null()).stderr(Stdio::null()).output() else { return Vec::new() };
    let mut kib: HashMap<u32, (u64, u64)> = HashMap::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let f: Vec<Option<u64>> = line.split_whitespace().map(|x| x.parse().ok()).collect();
        if let [Some(pid), Some(pgid), Some(rss)] = f[..] {
            let e = kib.entry(pgid as u32).or_default();
            if pid == pgid { e.0 += rss } else { e.1 += rss }
        }
    }
    groups.iter().filter_map(|&(session, pgid)| kib.get(&pgid).map(|&(s, h)| (session, s * 1024, h * 1024))).collect()
}

/// What to say about combined server memory: nothing up to [`MEMORY_NOTICE`]; above it, the total. The reader
/// never stops servers for it.
pub fn memory_notice(bytes: u64) -> Option<String> {
    (bytes > MEMORY_NOTICE).then(|| format!("Language servers use {:.1} GiB (above 2 GiB); disable or restart ones you do not need", bytes as f64 / (1u64 << 30) as f64))
}

/// A document's symbols as outline rows, in source order with their nesting depth: from nested
/// `DocumentSymbol`s or flat `SymbolInformation`s. Each row shows the source text at the symbol's name (so it
/// is validated like a declaration); at most `limit`.
pub fn file_symbols(reply: &Value, doc: &Arc<Document>, encoding: Encoding, limit: usize) -> Targets {
    file_symbols_while(reply, doc, encoding, limit, &|| true)
}

pub(crate) fn file_symbols_while(reply: &Value, doc: &Arc<Document>, encoding: Encoding, limit: usize, current: &dyn Fn() -> bool) -> Targets {
    fn walk(items: &[Value], depth: u16, doc: &Arc<Document>, encoding: Encoding, limit: usize, out: &mut Targets, current: &dyn Fn() -> bool) {
        for item in items {
            if !current() { return; }
            if out.found.len() == limit {
                out.limited = true;
                return;
            }
            let range = item.get("selectionRange").or(item.get("range")).or(item["location"].get("range"));
            let located = range.and_then(|r| Some((lsp::offset(doc, &r["start"], encoding)?, lsp::offset(doc, &r["end"], encoding)?)));
            match located {
                Some((start, end)) => {
                    let line = doc.line_of(start);
                    let bounds = doc.line_range(line);
                    let name = &doc.text()[start..end.clamp(start, bounds.end)];
                    let kind = decl_kind(item["kind"].as_u64().unwrap_or(0));
                    let (a, b) = (start - bounds.start, end.clamp(start, bounds.end) - bounds.start);
                    let hit = crate::search::Hit { path: doc.source().to_string_lossy().into(), line, range: a..b, text: name.into(), decl: Some((kind, depth)), origin: None };
                    out.found.push(Target { path: doc.source().to_owned(), stamp: None, doc: doc.clone(), hit, at: start });
                }
                None => out.failed.push(format!("{}: a symbol outside the file as sent", item["name"].as_str().unwrap_or("?"))),
            }
            if let Some(children) = item["children"].as_array() {
                walk(children, depth + 1, doc, encoding, limit, out, current);
            }
        }
    }
    let mut out = Targets::default();
    walk(reply.as_array().map_or(&[][..], Vec::as_slice), 0, doc, encoding, limit, &mut out, current);
    out.found.sort_by_key(|t| (t.hit.line, t.hit.range.start));
    out
}

/// The outline kind for a protocol `SymbolKind`.
/// The declaration kind a language server's symbol kind stands for.
pub fn decl_kind(kind: u64) -> lenz_content::highlight::DeclKind {
    use lenz_content::highlight::DeclKind;
    match kind {
        1..=4 => DeclKind::Module,
        5 | 10 | 23 | 26 => DeclKind::Type,
        11 => DeclKind::Interface,
        6 | 9 => DeclKind::Method,
        12 | 25 => DeclKind::Function,
        _ => DeclKind::Constant,
    }
}

/// A declaration kind as a row names it. Lenz has one kind for traits and interfaces: in Rust it is a trait.
pub fn kind_label(kind: lenz_content::highlight::DeclKind, path: &str) -> &'static str {
    match kind {
        lenz_content::highlight::DeclKind::Interface if path.ends_with(".rs") => "trait",
        _ => kind.label(),
    }
}

/// JDT LS's `triggerFiles` in its initialization options, unless the user set their own; options that are not an
/// object (the user's, as written) are sent unchanged.
fn add_trigger_file(options: &mut Value, uri: &str) {
    if options.is_null() {
        *options = json!({});
    }
    if let Some(o) = options.as_object_mut() {
        o.entry("triggerFiles").or_insert_with(|| json!([uri]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stderr_retains_bounded_raw_head_and_tail_across_arbitrary_chunks() {
        let mut log = Stderr::default();
        log.push(b"root cause\nrecovery hint\n");
        for _ in 0..20_000 {
            log.push("é𝄞\x1b\u{202e}".as_bytes());
            assert!(log.head.len() + log.tail.len() <= 8192);
        }
        log.push(b"final recovery hint");
        let shown = log.summary();
        assert!(shown.starts_with("root cause | recovery hint | "));
        assert!(shown.ends_with("final recovery hint"));
        assert!(shown.contains("stderr truncated"));
        assert!(!shown.chars().any(lenz_content::display::hides_text));
        assert!(shown.len() < 25_000);
    }

    #[test]
    fn trigger_files_join_object_options_and_leave_others_alone() {
        let uri = "file:///p/A.java";
        let mut none = serde_json::Value::Null;
        add_trigger_file(&mut none, uri);
        assert_eq!(none, serde_json::json!({ "triggerFiles": [uri] }));
        let mut theirs = serde_json::json!({ "triggerFiles": ["file:///p/B.java"], "x": 1 });
        add_trigger_file(&mut theirs, uri);
        assert_eq!(theirs, serde_json::json!({ "triggerFiles": ["file:///p/B.java"], "x": 1 }), "the user's own stand");
        for odd in [serde_json::json!([1]), serde_json::json!("s"), serde_json::json!(3)] {
            let mut v = odd.clone();
            add_trigger_file(&mut v, uri);
            assert_eq!(v, odd, "not an object: sent as the user wrote it");
        }
    }

    #[test]
    fn flat_file_symbols_are_rows_at_their_names_first_line() {
        let doc = Arc::new(Document::decode(PathBuf::from("/t.py"), 1, "class Ä:\n    def f(self):\n        pass\n".as_bytes()));
        // Flat `SymbolInformation`: whole-statement ranges, no nesting.
        let reply = json!([
            { "name": "f", "kind": 6, "location": { "uri": "file:///t.py", "range": { "start": { "line": 1, "character": 4 }, "end": { "line": 2, "character": 12 } } } },
            { "name": "Ä", "kind": 5, "location": { "uri": "file:///t.py", "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 2, "character": 12 } } } },
        ]);
        let t = file_symbols(&reply, &doc, Encoding::Utf16, 10);
        let rows: Vec<(usize, &str, u16)> = t.found.iter().map(|t| (t.hit.line, &*t.hit.text, t.hit.decl.unwrap().1)).collect();
        assert_eq!(rows, [(0, "class Ä:", 0), (1, "def f(self):", 0)], "source order, the first line of each range");
        assert!(file_symbols(&json!([{ "name": "x", "location": { "range": { "start": { "line": 9, "character": 0 }, "end": { "line": 9, "character": 1 } } } }]), &doc, Encoding::Utf16, 10).failed.len() == 1);
    }

    #[test]
    fn target_and_symbol_resolution_stop_after_cancellation_even_for_open_documents() {
        let doc = Arc::new(Document::decode(PathBuf::from("/cancel.rs"), 1, b"abcdef\n"));
        let open = HashMap::from([(doc.source().to_owned(), doc.clone())]);
        let values = Value::Array((0..6).map(|column| json!({ "uri": "file:///cancel.rs", "range": { "start": {"line":0,"character":column}, "end": {"line":0,"character":column+1} } })).collect());
        let steps = std::cell::Cell::new(0);
        let current = || { let step = steps.get() + 1; steps.set(step); step <= 2 };
        let targets = targets_while(&values, 20, Encoding::Utf16, "test", 1, &open, &|_| panic!("already open"), &current);
        assert_eq!(targets.found.len(), 2);
        steps.set(0);
        let symbols = file_symbols_while(&values, &doc, Encoding::Utf16, 20, &current);
        assert_eq!(symbols.found.len(), 2);
    }

}
