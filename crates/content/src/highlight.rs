//! Syntax highlighting: tree-sitter parses a document in the background, then its compact highlight
//! spans are kept instead of the much larger syntax tree. Painting only copies visible spans.
//! gpui-free and synchronous; mapping a [`Capture`] to a colour is the UI's job.
//!
//! Bundled grammars — all MIT, every crate depends only on `tree-sitter-language` so they share
//! the one `tree-sitter` 0.27 runtime. Each grammar's own highlight query is used, combined in the
//! order its upstream `tree-sitter.json` lists them (later patterns win on the same node).
//!
//! | Language | Crate | Licence | Query |
//! |---|---|---|---|
//! | Rust | [tree-sitter-rust 0.24.2](https://github.com/tree-sitter/tree-sitter-rust) | MIT | bundled + `@number` patch |
//! | C | [tree-sitter-c 0.24.2](https://github.com/tree-sitter/tree-sitter-c) | MIT | bundled |
//! | C++ | [tree-sitter-cpp 0.23.4](https://github.com/tree-sitter/tree-sitter-cpp) | MIT | C + C++ |
//! | Go | [tree-sitter-go 0.25.0](https://github.com/tree-sitter/tree-sitter-go) | MIT | bundled |
//! | Zig | [tree-sitter-zig 1.1.2](https://github.com/tree-sitter-grammars/tree-sitter-zig) | MIT | bundled |
//! | Java | [tree-sitter-java 0.23.5](https://github.com/tree-sitter/tree-sitter-java) | MIT | bundled |
//! | Kotlin | [tree-sitter-kotlin-sg 0.4.1](https://github.com/ast-grep/tree-sitter-kotlin) | MIT | bundled |
//! | C# | [tree-sitter-c-sharp 0.23.5](https://github.com/tree-sitter/tree-sitter-c-sharp) | MIT | bundled |
//! | Python | [tree-sitter-python 0.25.0](https://github.com/tree-sitter/tree-sitter-python) | MIT | bundled |
//! | Ruby | [tree-sitter-ruby 0.23.1](https://github.com/tree-sitter/tree-sitter-ruby) | MIT | bundled |
//! | PHP | [tree-sitter-php 0.24.2](https://github.com/tree-sitter/tree-sitter-php) | MIT | bundled |
//! | Lua | [tree-sitter-lua 0.5.0](https://github.com/tree-sitter-grammars/tree-sitter-lua) | MIT | bundled |
//! | Bash | [tree-sitter-bash 0.25.1](https://github.com/tree-sitter/tree-sitter-bash) | MIT | bundled |
//! | JavaScript/JSX | [tree-sitter-javascript 0.25.0](https://github.com/tree-sitter/tree-sitter-javascript) | MIT | JS + JSX |
//! | TypeScript/TSX | [tree-sitter-typescript 0.23.2](https://github.com/tree-sitter/tree-sitter-typescript) | MIT | TS (+ JSX) + JS |
//! | HTML | [tree-sitter-html 0.23.2](https://github.com/tree-sitter/tree-sitter-html) | MIT | bundled |
//! | CSS | [tree-sitter-css 0.25.0](https://github.com/tree-sitter/tree-sitter-css) | MIT | bundled |
//! | JSON | [tree-sitter-json 0.24.8](https://github.com/tree-sitter/tree-sitter-json) | MIT | bundled |
//! | YAML | [tree-sitter-yaml 0.7.2](https://github.com/tree-sitter-grammars/tree-sitter-yaml) | MIT | bundled |
//! | TOML | [tree-sitter-toml-ng 0.7.0](https://github.com/tree-sitter-grammars/tree-sitter-toml) | MIT | bundled |
//! | SQL | [tree-sitter-sequel 0.3.11](https://github.com/derekstride/tree-sitter-sql) | MIT | bundled |
//! | Markdown | [tree-sitter-md 0.5.3](https://github.com/tree-sitter-grammars/tree-sitter-markdown) | MIT | block only |
//! | Dockerfile | [tree-sitter-containerfile 0.9.2](https://github.com/wharflab/tree-sitter-containerfile) | MIT | bundled |
//! | Makefile | [tree-sitter-make 1.1.1](https://github.com/tree-sitter-grammars/tree-sitter-make) | MIT | bundled |
//!
//! Kotlin uses ast-grep's maintained fork of fwcd's grammar because `tree-sitter-kotlin` (0.3.8)
//! pins `tree-sitter < 0.23` and `tree-sitter-kotlin-ng` ships no highlight query.
//! Dockerfile uses wharflab's maintained fork of camdencheek's grammar (`tree-sitter-dockerfile`
//! 0.2.0 pins `tree-sitter 0.20`).
//!
//! Not done yet: injections (HTML/JS/CSS inside PHP, HTML `<script>`/`<style>`, code in Markdown
//! fences), the Markdown inline grammar (emphasis, code spans, links in paragraphs), and
//! `locals` queries.

use std::{cmp::Reverse, ops::{ControlFlow, Range}, path::Path, sync::{mpsc, Arc, OnceLock}};

use tree_sitter::{ParseOptions, Parser, Query, QueryCursor, StreamingIterator, Tree};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    C,
    Cpp,
    Go,
    Zig,
    Java,
    Kotlin,
    CSharp,
    Python,
    Ruby,
    Php,
    Lua,
    Bash,
    /// JavaScript including JSX.
    JavaScript,
    TypeScript,
    Tsx,
    Html,
    Css,
    Json,
    Yaml,
    Toml,
    Sql,
    Markdown,
    Dockerfile,
    Makefile,
}

impl Language {
    pub const ALL: [Language; 25] = [
        Self::Rust,
        Self::C,
        Self::Cpp,
        Self::Go,
        Self::Zig,
        Self::Java,
        Self::Kotlin,
        Self::CSharp,
        Self::Python,
        Self::Ruby,
        Self::Php,
        Self::Lua,
        Self::Bash,
        Self::JavaScript,
        Self::TypeScript,
        Self::Tsx,
        Self::Html,
        Self::Css,
        Self::Json,
        Self::Yaml,
        Self::Toml,
        Self::Sql,
        Self::Markdown,
        Self::Dockerfile,
        Self::Makefile,
    ];
}

/// The semantic class of a highlighted span, mapped from the grammar's capture names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capture {
    Keyword,
    String,
    /// An escape sequence inside a string.
    Escape,
    Comment,
    DocComment,
    Number,
    Constant,
    /// `true`, `false`, `null`, `nil`, `None`…
    ConstantBuiltin,
    Function,
    /// Rust `println!`, C macros.
    Macro,
    Type,
    /// Primitive types (`i32`, `int`, `string`).
    TypeBuiltin,
    Variable,
    /// `self`, `this`, `super`…
    VariableBuiltin,
    Parameter,
    Property,
    Operator,
    Punctuation,
    Tag,
    Attribute,
    Label,
    Module,
    Heading,
    Link,
}

/// One document's highlights. No syntax tree or duplicate source text survives the background job.
pub struct Parsed {
    offsets: Vec<usize>,
    spans: Vec<Span>,
}

struct Span {
    start: u32,
    end: u32,
    capture: Capture,
}

/// Only lives while a document's highlights are being built.
struct SyntaxTree<'a> {
    lang: Language,
    text: &'a str,
    /// [`crate::lines::line_starts`] of `text`.
    line_starts: Vec<usize>,
    tree: Tree,
}

pub fn detect(path: &Path, first_line: &str) -> Option<Language> {
    by_name(&path.file_name()?.to_string_lossy()).or_else(|| by_shebang(first_line))
}

fn by_name(name: &str) -> Option<Language> {
    use Language::*;
    let name = name.to_ascii_lowercase();
    Some(match name.as_str() {
        "dockerfile" | "containerfile" => Dockerfile,
        "makefile" | "gnumakefile" | "bsdmakefile" => Makefile,
        // `*.lock` is not one format: only the TOML / JSON ones, by name.
        "cargo.lock" | "poetry.lock" | "uv.lock" | "pipfile" => Toml,
        "composer.lock" | "flake.lock" | "pipfile.lock" => Json,
        "gemfile" | "rakefile" | "guardfile" | "podfile" | "vagrantfile" | "brewfile" => Ruby,
        "pkgbuild" | "apkbuild" | ".bashrc" | ".bash_profile" | ".bash_aliases" | ".bash_logout" | ".profile"
        | ".zshrc" | ".zshenv" | ".zprofile" => Bash,
        _ if name.starts_with("dockerfile.") || name.starts_with("containerfile.") => Dockerfile,
        _ => match name.rsplit_once('.')?.1 {
            "rs" => Rust,
            "c" => C,
            // `.h` as C++: its grammar parses C headers too, C's can't parse C++ ones.
            "h" | "cc" | "cpp" | "cxx" | "c++" | "hpp" | "hh" | "hxx" | "h++" | "ipp" | "inl" | "tpp" => Cpp,
            "go" => Go,
            "zig" | "zon" => Zig,
            "java" => Java,
            "kt" | "kts" => Kotlin,
            "cs" | "csx" => CSharp,
            "py" | "pyi" | "pyw" => Python,
            "rb" | "rake" | "gemspec" | "ru" => Ruby,
            "php" | "phtml" => Php,
            "lua" => Lua,
            "sh" | "bash" | "zsh" | "ksh" => Bash,
            "js" | "mjs" | "cjs" | "jsx" => JavaScript,
            "ts" | "mts" | "cts" => TypeScript,
            "tsx" => Tsx,
            "html" | "htm" | "xhtml" => Html,
            "css" => Css,
            "json" | "jsonc" | "geojson" | "webmanifest" => Json,
            "yaml" | "yml" => Yaml,
            "toml" => Toml,
            "sql" => Sql,
            "md" | "markdown" | "mdown" | "mkd" => Markdown,
            "dockerfile" | "containerfile" => Dockerfile,
            "mk" | "mak" | "make" => Makefile,
            _ => return None,
        },
    })
}

/// `#!/bin/bash`, `#!/usr/bin/env -S python3 -u`, `#!/usr/bin/python3.12`…
fn by_shebang(first_line: &str) -> Option<Language> {
    use Language::*;
    let mut words = first_line.strip_prefix("#!")?.split_whitespace();
    let mut program = words.next()?.rsplit('/').next()?;
    if program == "env" {
        program = words.find(|w| !w.starts_with('-') && !w.contains('='))?;
    }
    Some(match program.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.') {
        "python" | "pypy" => Python,
        "bash" | "sh" | "zsh" | "ksh" | "dash" | "ash" => Bash,
        "node" | "nodejs" => JavaScript,
        "ruby" => Ruby,
        "php" => Php,
        "lua" | "luajit" => Lua,
        "make" | "gmake" => Makefile,
        _ => return None,
    })
}

struct Config {
    language: tree_sitter::Language,
    query: Query,
    /// Per capture index: `None` ignores it, `Some(None)` clears (`@none`), `Some(Some(c))` paints `c`.
    captures: Vec<Option<Option<Capture>>>,
}

/// The grammar and its compiled highlight query, built once per language (tens of ms).
fn config(lang: Language) -> &'static Config {
    use Language::*;
    static CONFIGS: [OnceLock<Config>; Language::ALL.len()] = [const { OnceLock::new() }; Language::ALL.len()];
    crate::syntax_arena::install();
    CONFIGS[lang as usize].get_or_init(|| {
        let (language, queries): (tree_sitter::Language, &[&str]) = match lang {
            // Patch: the bundled query captures numbers as `@constant.builtin` (like `true`).
            Rust => (
                tree_sitter_rust::LANGUAGE.into(),
                &[tree_sitter_rust::HIGHLIGHTS_QUERY, "[(integer_literal) (float_literal)] @number"],
            ),
            C => (tree_sitter_c::LANGUAGE.into(), &[tree_sitter_c::HIGHLIGHT_QUERY]),
            Cpp => (tree_sitter_cpp::LANGUAGE.into(), &[tree_sitter_c::HIGHLIGHT_QUERY, tree_sitter_cpp::HIGHLIGHT_QUERY]),
            Go => (tree_sitter_go::LANGUAGE.into(), &[tree_sitter_go::HIGHLIGHTS_QUERY]),
            Zig => (tree_sitter_zig::LANGUAGE.into(), &[tree_sitter_zig::HIGHLIGHTS_QUERY]),
            Java => (tree_sitter_java::LANGUAGE.into(), &[tree_sitter_java::HIGHLIGHTS_QUERY]),
            Kotlin => (tree_sitter_kotlin_sg::LANGUAGE.into(), &[tree_sitter_kotlin_sg::HIGHLIGHTS_QUERY]),
            CSharp => (tree_sitter_c_sharp::LANGUAGE.into(), &[tree_sitter_c_sharp::HIGHLIGHTS_QUERY]),
            Python => (tree_sitter_python::LANGUAGE.into(), &[tree_sitter_python::HIGHLIGHTS_QUERY]),
            Ruby => (tree_sitter_ruby::LANGUAGE.into(), &[tree_sitter_ruby::HIGHLIGHTS_QUERY]),
            Php => (tree_sitter_php::LANGUAGE_PHP.into(), &[tree_sitter_php::HIGHLIGHTS_QUERY]),
            Lua => (tree_sitter_lua::LANGUAGE.into(), &[tree_sitter_lua::HIGHLIGHTS_QUERY]),
            Bash => (tree_sitter_bash::LANGUAGE.into(), &[tree_sitter_bash::HIGHLIGHT_QUERY]),
            JavaScript => (
                tree_sitter_javascript::LANGUAGE.into(),
                &[tree_sitter_javascript::HIGHLIGHT_QUERY, tree_sitter_javascript::JSX_HIGHLIGHT_QUERY],
            ),
            TypeScript => (
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                &[tree_sitter_typescript::HIGHLIGHTS_QUERY, tree_sitter_javascript::HIGHLIGHT_QUERY],
            ),
            Tsx => (
                tree_sitter_typescript::LANGUAGE_TSX.into(),
                &[
                    tree_sitter_typescript::HIGHLIGHTS_QUERY,
                    tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                    tree_sitter_javascript::HIGHLIGHT_QUERY,
                ],
            ),
            Html => (tree_sitter_html::LANGUAGE.into(), &[tree_sitter_html::HIGHLIGHTS_QUERY]),
            Css => (tree_sitter_css::LANGUAGE.into(), &[tree_sitter_css::HIGHLIGHTS_QUERY]),
            Json => (tree_sitter_json::LANGUAGE.into(), &[tree_sitter_json::HIGHLIGHTS_QUERY]),
            Yaml => (tree_sitter_yaml::LANGUAGE.into(), &[tree_sitter_yaml::HIGHLIGHTS_QUERY]),
            Toml => (tree_sitter_toml_ng::LANGUAGE.into(), &[tree_sitter_toml_ng::HIGHLIGHTS_QUERY]),
            Sql => (tree_sitter_sequel::LANGUAGE.into(), &[tree_sitter_sequel::HIGHLIGHTS_QUERY]),
            Markdown => (tree_sitter_md::LANGUAGE.into(), &[tree_sitter_md::HIGHLIGHT_QUERY_BLOCK]),
            Dockerfile => (tree_sitter_containerfile::LANGUAGE.into(), &[tree_sitter_containerfile::HIGHLIGHTS_QUERY]),
            Makefile => (tree_sitter_make::LANGUAGE.into(), &[tree_sitter_make::HIGHLIGHTS_QUERY]),
        };
        let query =
            Query::new(&language, &queries.join("\n")).unwrap_or_else(|e| panic!("{lang:?} highlight query: {e}"));
        let captures = query.capture_names().iter().map(|name| capture(name)).collect();
        Config { language, query, captures }
    })
}

/// Map a tree-sitter capture name (`keyword.return`, `string.escape`…) onto the closed set.
fn capture(name: &str) -> Option<Option<Capture>> {
    use Capture::*;
    Some(Some(match name {
        "none" => return Some(None),
        "comment.documentation" | "comment.doc" => DocComment,
        "string.escape" | "escape" => Escape,
        "string.special.symbol" => Constant,
        "string.special.key" => Property,
        "constant.builtin" | "boolean" => ConstantBuiltin,
        "type.builtin" => TypeBuiltin,
        "type.qualifier" => Keyword,
        "variable.builtin" => VariableBuiltin,
        "variable.parameter" | "parameter" => Parameter,
        "variable.member" | "field" => Property,
        "function.macro" => Macro,
        "text.title" | "markup.heading" => Heading,
        "text.uri" | "text.reference" | "markup.link" => Link,
        "text.literal" => String,
        _ => match name.split('.').next().unwrap_or_default() {
            "keyword" | "conditional" | "repeat" | "include" | "import" | "exception" | "storageclass" | "preproc" => {
                Keyword
            }
            "string" | "character" => String,
            "comment" => Comment,
            "number" | "float" => Number,
            "constant" => Constant,
            "function" | "method" => Function,
            "type" | "constructor" => Type,
            "variable" => Variable,
            "property" => Property,
            "operator" => Operator,
            "punctuation" | "delimiter" => Punctuation,
            "tag" => Tag,
            "attribute" => Attribute,
            "label" => Label,
            "module" | "namespace" => Module,
            _ => return None,
        },
    }))
}

#[cfg(test)]
pub fn parse(lang: Language, text: &str) -> Parsed {
    parse_while(lang, text.into(), || true).unwrap()
}

/// Called off the UI thread. Obsolete documents leave the queue (or stop parsing) promptly.
pub fn parse_while(lang: Language, text: Arc<str>, current: impl Fn() -> bool + Send + 'static) -> Option<Parsed> {
    parse_with(lang, text, Want::default(), current).map(|s| s.parsed)
}

/// As [`parse_while`], also returning the document's fold regions, found in the same tree.
pub fn parse_folded_while(lang: Language, text: Arc<str>, current: impl Fn() -> bool + Send + 'static) -> Option<(Parsed, Folds)> {
    parse_with(lang, text, Want { folds: true, declarations: false }, current).map(|s| (s.parsed, s.folds.unwrap_or_default()))
}

/// What a parse finds besides the highlight spans, from the same tree.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Want {
    pub folds: bool,
    /// Declarations, for languages that have them ([`has_declarations`]).
    pub declarations: bool,
}

/// One parse's results: the spans, and what [`Want`] asked for.
pub struct Syntax {
    pub parsed: Parsed,
    pub folds: Option<Folds>,
    pub declarations: Option<Declarations>,
}

/// As [`parse_while`], also returning what `want` asks for, found in the same tree.
pub fn parse_with(lang: Language, text: Arc<str>, want: Want, current: impl Fn() -> bool + Send + 'static) -> Option<Syntax> {
    on_worker(move || parse_now(lang, &text, &current, want))
}

/// Run one parse job on the syntax pool and wait for its answer.
fn on_worker<R: Send + 'static>(job: impl FnOnce() -> Option<R> + Send + 'static) -> Option<R> {
    type Job = Box<dyn FnOnce() + Send>;
    // A small pool, so a merge's three panes or a comparison's two sides parse at once. Each worker hands its
    // freed tree back to the system after every parse, so an idle worker keeps no peak resident.
    static WORKERS: OnceLock<mpsc::Sender<Job>> = OnceLock::new();
    let workers = WORKERS.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        let rx = Arc::new(std::sync::Mutex::new(rx));
        let count = std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, SYNTAX_WORKERS);
        for i in 0..count {
            let rx = rx.clone();
            std::thread::Builder::new().name(format!("lenz-highlight-{i}")).spawn(move || loop {
                let job = rx.lock().unwrap_or_else(std::sync::PoisonError::into_inner).recv();
                let Ok(job) = job else { break };
                job();
                release_freed_memory();
            }).expect("start syntax worker");
        }
        tx
    });
    let (tx, rx) = mpsc::sync_channel(1);
    let job: Job = Box::new(move || {
        // A panic while parsing (in an unwinding build) answers "nothing", and the worker lives on.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
        let _ = tx.send(result.unwrap_or(None));
    });
    if workers.send(job).is_err() {
        return None; // no worker left: plain text, not a second panic
    }
    rx.recv().ok().flatten()
}

/// Parsers that may run at once: a three-way merge's panes.
const SYNTAX_WORKERS: usize = 3;

/// glibc keeps a freed syntax tree's pages in the worker's arena, so one parse of a large file stayed
/// resident for the life of the process. Hand them back after each parse, off the UI thread.
#[allow(unsafe_code)] // the one FFI call in the crate; see `lib.rs`
fn release_freed_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: malloc_trim only returns free heap pages to the kernel; no live allocation is touched.
    unsafe {
        libc::malloc_trim(0);
    }
}

fn parse_now(lang: Language, text: &str, current: &dyn Fn() -> bool, want: Want) -> Option<Syntax> {
    if !current() { return None; }
    let language = &config(lang).language; // compiled outside the arena: queries outlive every parse
    let tags = if want.declarations { tags(lang) } else { None }; // the same, for the tags query
    // Declared before the parser, tree and query cursors, so it is dropped after all of them.
    let _arena = crate::syntax_arena::Scope::begin(text.len());
    let mut parser = Parser::new();
    parser.set_language(language).expect("grammar ABI is checked by tests");
    let mut progress = |_: &tree_sitter::ParseState| if current() { ControlFlow::Continue(()) } else { ControlFlow::Break(()) };
    let tree = parser.parse_with_options(
        &mut |offset, _| text.as_bytes().get(offset..).unwrap_or_default(),
        None,
        Some(ParseOptions::new().progress_callback(&mut progress)),
    )?;
    let line_starts = crate::lines::line_starts(text);
    let syntax = SyntaxTree { lang, text, line_starts, tree };
    let n = syntax.line_starts.len();
    let mut parsed = Parsed { offsets: Vec::with_capacity(n + 1), spans: Vec::new() };
    // Bound temporary query captures and per-byte paint; a full-file query can dwarf the tree.
    for start in (0..n).step_by(256) {
        if !current() { return None; }
        for line in syntax.spans(start..(start + 256).min(n)) {
            parsed.offsets.push(parsed.spans.len());
            parsed.spans.extend(line.into_iter().map(|(r, capture)| Span { start: r.start as u32, end: r.end as u32, capture }));
        }
    }
    parsed.offsets.push(parsed.spans.len());
    let folds = if want.folds { Some(syntax.folds(current)?) } else { None };
    let declarations = match tags {
        Some(tags) => Some(declarations_in(tags, lang, text, &syntax.tree, current)?),
        None => None,
    };
    drop(syntax);
    parsed.spans.shrink_to_fit();
    Some(Syntax { parsed, folds, declarations })
}

impl Parsed {
    pub fn bytes(&self) -> usize {
        self.offsets.capacity() * size_of::<usize>() + self.spans.capacity() * size_of::<Span>()
    }

    /// A capture at one line-relative byte, without cloning the line's span list.
    pub fn capture_at(&self, line: usize, byte: usize) -> Option<Capture> {
        let from = *self.offsets.get(line)?;
        let to = *self.offsets.get(line.checked_add(1)?)?;
        let spans = self.spans.get(from..to)?;
        let at = spans.partition_point(|span| span.end as usize <= byte);
        spans.get(at).filter(|span| span.start as usize <= byte).map(|span| span.capture)
    }

    pub fn spans(&self, lines: Range<usize>) -> Vec<Vec<(Range<usize>, Capture)>> {
        let n = self.offsets.len() - 1;
        (lines.start.min(n)..lines.end.min(n)).map(|l| {
            self.spans[self.offsets[l]..self.offsets[l + 1]].iter()
                .map(|s| (s.start as usize..s.end as usize, s.capture)).collect()
        }).collect()
    }
}

impl SyntaxTree<'_> {
    /// For each line in `lines` (clamped to the text), the highlighted byte ranges within that
    /// line (excluding its `\n` / `\r\n`), sorted and non-overlapping.
    pub fn spans(&self, lines: Range<usize>) -> Vec<Vec<(Range<usize>, Capture)>> {
        let n = self.line_starts.len();
        let lines = lines.start.min(n)..lines.end.min(n);
        if lines.is_empty() {
            return Vec::new();
        }
        let bounds: Vec<Range<usize>> = lines.clone().map(|l| self.line(l)).collect();
        let window = self.line_starts[lines.start]..self.line_starts.get(lines.end).copied().unwrap_or(self.text.len());

        let cfg = config(self.lang);
        let mut found = Vec::new();
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(window);
        let mut captures = cursor.captures(&cfg.query, self.tree.root_node(), self.text.as_bytes());
        while let Some((m, i)) = captures.next() {
            let c = m.captures()[*i];
            if let Some(paint) = cfg.captures[c.index as usize] {
                found.push((c.node.byte_range(), paint));
            }
        }
        // tree-sitter-highlight's precedence: an inner node paints over its parent, and for the
        // same node the later pattern wins (captures arrive in pattern order; the sort is stable).
        found.sort_by_key(|(r, _)| (r.start, Reverse(r.end)));

        let mut paint: Vec<Vec<Option<Capture>>> = bounds.iter().map(|b| vec![None; b.len()]).collect();
        for (range, capture) in found {
            let first = self.line_starts.partition_point(|&s| s <= range.start) - 1;
            for l in first.max(lines.start)..lines.end {
                let b = &bounds[l - lines.start];
                if b.start >= range.end {
                    break;
                }
                let (s, e) = (range.start.max(b.start), range.end.min(b.end));
                if s < e {
                    paint[l - lines.start][s - b.start..e - b.start].fill(capture);
                }
            }
        }
        paint.into_iter().map(runs).collect()
    }

    fn line(&self, l: usize) -> Range<usize> {
        crate::lines::line_range(self.text, &self.line_starts, l)
    }
}

/// Merge per-byte paint into runs.
fn runs(paint: Vec<Option<Capture>>) -> Vec<(Range<usize>, Capture)> {
    let mut out: Vec<(Range<usize>, Capture)> = Vec::new();
    for (i, p) in paint.into_iter().enumerate() {
        let Some(c) = p else { continue };
        match out.last_mut() {
            Some((r, last)) if r.end == i && *last == c => r.end += 1,
            _ => out.push((i..i + 1, c)),
        }
    }
    out
}

/// What a fold region holds: code, a run of comments, or a run of imports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FoldKind {
    Block,
    Comment,
    Imports,
}

/// Lines `first..=last` (0-based) that fold into `first`; `closes`: the last line is only closing punctuation, so a
/// collapsed region can show it after its placeholder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fold {
    pub first: u32,
    pub last: u32,
    pub kind: FoldKind,
    pub closes: bool,
}

/// A document's fold regions, sorted by first line, at most one per first line (the outermost).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Folds {
    pub regions: Vec<Fold>,
}

impl Folds {
    pub fn bytes(&self) -> usize {
        self.regions.capacity() * size_of::<Fold>()
    }

    /// The region starting on `line`.
    pub fn starting(&self, line: usize) -> Option<Fold> {
        let i = self.regions.partition_point(|f| (f.first as usize) < line);
        self.regions.get(i).filter(|f| f.first as usize == line).copied()
    }
}

/// Node kinds that import or include something, across the bundled grammars.
const IMPORT_KINDS: &[&str] = &[
    "use_declaration",
    "extern_crate_declaration",
    "import_statement",
    "import_from_statement",
    "future_import_statement",
    "import_declaration",
    "preproc_include",
    "using_directive",
    "import_header",
    "namespace_use_declaration",
];

/// Per node kind id of `lang`'s grammar: whether it is a comment or an import, built once (cheaper than comparing each
/// node's kind name).
fn run_kinds(lang: Language) -> &'static [Option<FoldKind>] {
    static KINDS: [OnceLock<Vec<Option<FoldKind>>>; Language::ALL.len()] = [const { OnceLock::new() }; Language::ALL.len()];
    KINDS[lang as usize].get_or_init(|| {
        let language = &config(lang).language;
        (0..language.node_kind_count() as u16)
            .map(|id| {
                let kind = language.node_kind_for_id(id).unwrap_or_default();
                if kind.contains("comment") {
                    Some(FoldKind::Comment)
                } else if IMPORT_KINDS.contains(&kind) {
                    Some(FoldKind::Imports)
                } else {
                    None
                }
            })
            .collect()
    })
}

impl SyntaxTree<'_> {
    /// Every named node spanning two lines or more (the outermost per first line), with consecutive comment lines and
    /// consecutive import lines merged into one region each. `None` once `current` says the work is obsolete.
    fn folds(&self, current: &dyn Fn() -> bool) -> Option<Folds> {
        let mut found: Vec<Fold> = Vec::new();
        // Comments and imports, merged into runs afterwards: (first, last, kind).
        let mut runs: Vec<(u32, u32, FoldKind)> = Vec::new();
        let runs_by_kind = run_kinds(self.lang);
        let mut cursor = self.tree.walk();
        let mut seen = 0usize;
        'walk: loop {
            let node = cursor.node();
            seen += 1;
            if seen.is_multiple_of(4096) && !current() {
                return None;
            }
            let run = runs_by_kind.get(usize::from(node.kind_id())).copied().flatten();
            let first = node.start_position().row as u32;
            let end = node.end_position();
            // A node ending at a line's start (its line ending included) ends on the line before.
            let last = if end.column == 0 && end.row > 0 { end.row - 1 } else { end.row } as u32;
            // Not the root (depth 0): it spans the whole text.
            if node.is_named() && cursor.depth() > 0 {
                // A comment or import joins a run only when it starts its line (a trailing comment is part of the code).
                let leads = || self.text.get(self.line_starts[first as usize]..node.start_byte()).is_some_and(|b| b.trim().is_empty());
                match run {
                    Some(k) if leads() => runs.push((first, last, k)),
                    Some(_) => {}
                    None if last > first => found.push(Fold { first, last, kind: FoldKind::Block, closes: self.closes(last as usize) }),
                    None => {}
                }
            }
            // Comments and imports fold as a whole; a node on one line holds no region (nor a comment alone on a line).
            if run.is_none() && last > first && cursor.goto_first_child() {
                continue;
            }
            loop {
                if cursor.goto_next_sibling() {
                    continue 'walk;
                }
                if !cursor.goto_parent() {
                    break 'walk;
                }
            }
        }
        // Runs on consecutive lines of one kind merge; a single multi-line comment is a region on its own.
        runs.sort_unstable_by_key(|r| (r.0, r.1));
        let mut merged: Vec<(u32, u32, FoldKind)> = Vec::new();
        for r in runs {
            match merged.last_mut() {
                Some(m) if m.2 == r.2 && r.0 <= m.1 + 1 => m.1 = m.1.max(r.1),
                _ => merged.push(r),
            }
        }
        found.extend(merged.into_iter().filter(|r| r.1 > r.0).map(|(first, last, kind)| Fold { first, last, kind, closes: self.closes(last as usize) }));
        // The outermost region per first line: by first line, widest first, the first of each line kept.
        found.sort_unstable_by_key(|f| (f.first, Reverse(f.last)));
        found.dedup_by_key(|f| f.first);
        found.shrink_to_fit();
        Some(Folds { regions: found })
    }

    /// Whether line `l` holds only closing punctuation (`}`, `)`, `]`, then `;` or `,`) or `end`.
    fn closes(&self, l: usize) -> bool {
        let t = self.text.get(self.line(l)).unwrap_or_default().trim().trim_end_matches([';', ',']);
        t == "end" || (!t.is_empty() && t.chars().all(|c| matches!(c, '}' | ')' | ']')))
    }
}

/// Bracket and quote pairs whose inside is a selection step before the whole node.
const PAIRS: &[(&str, &str)] = &[("{", "}"), ("(", ")"), ("[", "]"), ("\"", "\""), ("'", "'"), ("`", "`"), ("string_start", "string_end")];

/// The ranges a selection grows through from `range`, smallest first: the word at a caret, a string's or bracketed
/// block's inside before the whole, then each enclosing node with a different range, up to the whole text. One parse
/// on the syntax pool, inside its arena; no tree outlives the call. `None` once `current` says the work is obsolete.
pub fn selection_steps_while(lang: Language, text: Arc<str>, range: Range<usize>, current: impl Fn() -> bool + Send + 'static) -> Option<Vec<Range<usize>>> {
    on_worker(move || steps_now(lang, &text, range, &current))
}

fn steps_now(lang: Language, text: &str, range: Range<usize>, current: &dyn Fn() -> bool) -> Option<Vec<Range<usize>>> {
    if !current() {
        return None;
    }
    let range = range.start.min(text.len())..range.end.min(text.len());
    let mut out: Vec<Range<usize>> = Vec::new();
    // Kept only when it strictly holds the last step (or the starting range).
    let mut push = |r: Range<usize>| {
        let last = out.last().cloned().unwrap_or(range.clone());
        if r.start <= last.start && r.end >= last.end && r != last {
            out.push(r);
        }
    };
    // The word at, or just before, a caret.
    if range.is_empty() {
        let word = |c: char| c.is_alphanumeric() || c == '_';
        let (before, after) = (text.get(..range.start).unwrap_or_default(), text.get(range.start..).unwrap_or_default());
        let start = before.char_indices().rev().take_while(|(_, c)| word(*c)).last().map_or(range.start, |(i, _)| i);
        let end = after.char_indices().take_while(|(_, c)| word(*c)).last().map_or(range.start, |(i, c)| range.start + i + c.len_utf8());
        if start < end {
            push(start..end);
        }
    }
    let language = &config(lang).language; // compiled outside the arena
    {
        // Declared before the parser and tree, so it is dropped after both.
        let _arena = crate::syntax_arena::Scope::begin(text.len());
        let mut parser = Parser::new();
        parser.set_language(language).ok()?;
        let mut progress = |_: &tree_sitter::ParseState| if current() { ControlFlow::Continue(()) } else { ControlFlow::Break(()) };
        let tree = parser.parse_with_options(
            &mut |offset, _| text.as_bytes().get(offset..).unwrap_or_default(),
            None,
            Some(ParseOptions::new().progress_callback(&mut progress)),
        )?;
        let mut node = tree.root_node().descendant_for_byte_range(range.start, range.end);
        while let Some(n) = node {
            let count = n.child_count();
            if count >= 2
                && let (Some(open), Some(close)) = (n.child(0), n.child(count - 1))
                && PAIRS.iter().any(|(a, b)| open.kind() == *a && close.kind() == *b)
                && open.end_byte() <= close.start_byte()
            {
                let inside = text.get(open.end_byte()..close.start_byte()).unwrap_or_default();
                let (s, e) = (open.end_byte() + inside.len() - inside.trim_start().len(), close.start_byte() - (inside.len() - inside.trim_end().len()));
                if s < e {
                    push(s..e);
                }
            }
            push(n.byte_range());
            node = n.parent();
        }
    }
    push(0..text.len());
    Some(out)
}

/// What a declaration is, by syntax. A syntactic candidate, not proof of what a reference resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeclKind {
    Module,
    /// Struct, class, enum, object, type alias.
    Type,
    /// Trait, interface.
    Interface,
    Function,
    Method,
    /// A function or constant whose name starts with a capital letter in JS/TS (a UI component by convention).
    Component,
    Constant,
    Macro,
    /// An HTML element with an `id`.
    Element,
    /// A CSS rule's selectors.
    Selector,
    /// A Rust `impl` block, named by its header (`impl<T> Show for S`).
    Impl,
    /// A Markdown section, named by its heading; it runs to the next heading of the same or a higher level.
    Heading,
}

impl DeclKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Module => "module",
            Self::Type => "type",
            Self::Interface => "interface",
            Self::Function => "function",
            Self::Method => "method",
            Self::Component => "component",
            Self::Constant => "constant",
            Self::Macro => "macro",
            Self::Element => "element",
            Self::Selector => "selector",
            Self::Impl => "impl",
            Self::Heading => "heading",
        }
    }
}

/// One declaration, compact: no tree, no copy of the source beyond its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub kind: DeclKind,
    /// The name as written, at most [`NAME_CHARS`] characters.
    pub name: Box<str>,
    /// Byte ranges of the name and of the whole declaration.
    pub name_range: Range<u32>,
    pub range: Range<u32>,
    /// The declaration's first and last source lines (0-based; a range ending at a line's start ends on the line before).
    pub first_line: u32,
    pub last_line: u32,
    /// Declarations enclosing this one.
    pub depth: u16,
    /// The innermost enclosing declaration's index in [`Declarations::items`].
    pub parent: Option<u32>,
}

impl Declaration {
    /// Memory this record holds, for index budgets.
    pub fn bytes(&self) -> usize {
        size_of::<Self>() + self.name.len()
    }
}

/// A document's declarations, in source order, and whether its syntax had errors (so the list may be partial).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Declarations {
    pub items: Vec<Declaration>,
    pub syntax_errors: bool,
}

/// Characters of a declaration's name kept.
pub const NAME_CHARS: usize = 120;

/// A node's first and last lines; an end at a line's start (after its newline) belongs to the line before.
fn lines_of(node: tree_sitter::Node<'_>) -> (u32, u32) {
    let (start, end) = (node.start_position(), node.end_position());
    let last = if end.column == 0 && end.row > start.row { end.row - 1 } else { end.row };
    (start.row as u32, last as u32)
}

/// One pinned header: the declaration's index in [`Declarations::items`], its first line, and the row (0 = top) it is
/// drawn on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sticky {
    pub index: u32,
    pub line: u32,
    pub row: u32,
}

impl Declarations {
    /// The declarations containing byte `at`, outermost first.
    pub fn enclosing(&self, at: u32) -> Vec<&Declaration> {
        let start = self.items.partition_point(|d| d.range.start <= at).checked_sub(1);
        self.refs(self.chain(start, |d| d.range.start <= at && at < d.range.end))
    }

    /// The declarations spanning source line `line`, outermost first.
    pub fn enclosing_line(&self, line: u32) -> Vec<&Declaration> {
        self.refs(self.line_chain(line))
    }

    fn line_chain(&self, line: u32) -> Vec<u32> {
        let start = self.items.partition_point(|d| d.first_line <= line).checked_sub(1);
        self.chain(start, |d| d.first_line <= line && line <= d.last_line)
    }

    /// From item `at` up through its parents, the ones `holds` accepts, outermost first.
    fn chain(&self, at: Option<usize>, holds: impl Fn(&Declaration) -> bool) -> Vec<u32> {
        let mut out = Vec::new();
        let mut at = at.map(|i| i as u32);
        while let Some(i) = at {
            let Some(d) = self.items.get(i as usize) else { break };
            if holds(d) {
                out.push(i);
            }
            at = d.parent;
        }
        out.reverse();
        out
    }

    fn refs(&self, ids: Vec<u32>) -> Vec<&Declaration> {
        ids.into_iter().filter_map(|i| self.items.get(i as usize)).collect()
    }

    /// The headers to pin when `top` is the first shown line (`partial`: only its continuation rows are shown): the
    /// declarations around `top` whose first line is above it, outermost on row 0, at most `max`, innermost kept. A
    /// header whose scope ends within the stack moves up to end on its last line; one that would share a row with an
    /// outer header is dropped.
    pub fn sticky(&self, top: u32, partial: bool, max: usize) -> Vec<Sticky> {
        self.sticky_rows(top, partial, max, |line| line.saturating_sub(top))
    }

    /// As [`Declarations::sticky`], for a view that does not show every line below `top` on its own row (folds,
    /// wrapped lines): `row_of(line)` is the row (0 = top) where source line `line` starts, `u32::MAX` past the view.
    pub fn sticky_rows(&self, top: u32, partial: bool, max: usize, row_of: impl Fn(u32) -> u32) -> Vec<Sticky> {
        let ids: Vec<u32> = self
            .line_chain(top)
            .into_iter()
            .filter(|&i| self.items.get(i as usize).is_some_and(|d| d.last_line > d.first_line && (d.first_line < top || partial && d.first_line == top)))
            .collect();
        let mut out: Vec<Sticky> = Vec::with_capacity(ids.len().min(max));
        for (slot, &index) in ids.iter().skip(ids.len().saturating_sub(max)).enumerate() {
            let Some(d) = self.items.get(index as usize) else { continue };
            let row = (slot as u32).min(row_of(d.last_line));
            if out.last().is_some_and(|s| s.row >= row) {
                continue;
            }
            out.push(Sticky { index, line: d.first_line, row });
        }
        out
    }
}

/// Kotlin's definitions (from ast-grep's tags.scm, MIT, which the crate does not export).
const KOTLIN_TAGS: &str = r#"
(class_declaration "class" (type_identifier) @name) @definition.class
(class_declaration "interface" (type_identifier) @name) @definition.interface
(object_declaration (type_identifier) @name) @definition.class
(function_declaration (simple_identifier) @name) @definition.function
(source_file (property_declaration (variable_declaration (simple_identifier) @name)) @definition.constant)
(class_body (property_declaration (variable_declaration (simple_identifier) @name)) @definition.constant)
(enum_entry (simple_identifier) @name) @definition.constant
(type_alias (type_identifier) @name) @definition.type
(companion_object (type_identifier) @name) @definition.class
"#;
/// HTML elements with an `id`, by that id.
const HTML_TAGS: &str = "
(element (start_tag (attribute (attribute_name) @_attr (quoted_attribute_value (attribute_value) @name)) (#eq? @_attr \"id\"))) @definition.element
(element (self_closing_tag (attribute (attribute_name) @_attr (quoted_attribute_value (attribute_value) @name)) (#eq? @_attr \"id\"))) @definition.element
";
/// TypeScript `namespace` blocks, which the upstream tags query omits.
const TS_NAMESPACES: &str = "(internal_module name: (_) @name) @definition.module";
/// CSS rules, by their selectors.
const CSS_TAGS: &str = "(rule_set (selectors) @name) @definition.selector";
/// Rust `impl` blocks, which the upstream tags query omits; named by their header.
const RUST_IMPLS: &str = "(impl_item type: (_) @name) @definition.impl";
/// Markdown sections, by their heading's text.
const MARKDOWN_TAGS: &str = "
(section (atx_heading heading_content: (_) @name)) @definition.heading
(section (setext_heading heading_content: (_) @name)) @definition.heading
";

struct Tags {
    query: Query,
    name: u32,
    /// Per capture index: the kind a `@definition.*` capture declares.
    kinds: Vec<Option<DeclKind>>,
}

/// Whether [`declarations_while`] supports `lang`.
pub fn has_declarations(lang: Language) -> bool {
    tags_queries(lang).is_some()
}

fn tags_queries(lang: Language) -> Option<&'static [&'static str]> {
    use Language::*;
    Some(match lang {
        Rust => &[tree_sitter_rust::TAGS_QUERY, RUST_IMPLS],
        Python => &[tree_sitter_python::TAGS_QUERY],
        Ruby => &[tree_sitter_ruby::TAGS_QUERY],
        Java => &[tree_sitter_java::TAGS_QUERY],
        Kotlin => &[KOTLIN_TAGS],
        JavaScript => &[tree_sitter_javascript::TAGS_QUERY],
        TypeScript | Tsx => &[tree_sitter_typescript::TAGS_QUERY, tree_sitter_javascript::TAGS_QUERY, TS_NAMESPACES],
        Html => &[HTML_TAGS],
        Css => &[CSS_TAGS],
        Markdown => &[MARKDOWN_TAGS],
        _ => return None,
    })
}

/// The tags query for `lang`, compiled once outside any syntax arena (it outlives every parse).
fn tags(lang: Language) -> Option<&'static Tags> {
    static TAGS: [OnceLock<Option<Tags>>; Language::ALL.len()] = [const { OnceLock::new() }; Language::ALL.len()];
    crate::syntax_arena::install();
    TAGS[lang as usize]
        .get_or_init(|| {
            let queries = tags_queries(lang)?;
            // A query that does not compile means no declarations for the language (a test checks they all do).
            let query = Query::new(&config(lang).language, &queries.join("\n")).ok()?;
            let name = query.capture_index_for_name("name")?;
            let kinds = query
                .capture_names()
                .iter()
                .map(|c| {
                    Some(match c.strip_prefix("definition.")? {
                        "module" | "namespace" => DeclKind::Module,
                        "class" | "type" | "enum" | "struct" | "union" => DeclKind::Type,
                        "interface" | "trait" => DeclKind::Interface,
                        "function" => DeclKind::Function,
                        "method" => DeclKind::Method,
                        "constant" => DeclKind::Constant,
                        "macro" => DeclKind::Macro,
                        "element" => DeclKind::Element,
                        "selector" => DeclKind::Selector,
                        "impl" => DeclKind::Impl,
                        "heading" => DeclKind::Heading,
                        _ => return None,
                    })
                })
                .collect();
            Some(Tags { query, name, kinds })
        })
        .as_ref()
}

/// A subtype–supertype pair written in source: `impl Show for S`, `trait Fancy: Show`, `class A(B)`,
/// `class A extends B implements I`, `class A : B(), I`, `class A < B` / `include M`. Byte ranges into the text:
/// each type's last plain name (`a::Show<T>` → `Show`) and the header it is written in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relation {
    pub sub: Range<usize>,
    pub sup: Range<usize>,
    /// The declaration or impl header holding the relation (to its body).
    pub site: Range<usize>,
    /// [`DeclKind::Impl`] for a Rust impl, [`DeclKind::Interface`] for a trait or interface, [`DeclKind::Module`] for
    /// a Ruby module, else [`DeclKind::Type`].
    pub sub_kind: DeclKind,
    /// [`DeclKind::Interface`] for a trait bound, an implemented interface or a mixin, else [`DeclKind::Type`].
    pub sup_kind: DeclKind,
}

const RUST_RELATIONS: &str = "
(impl_item
  trait: [(type_identifier) @sup.interface
          (scoped_type_identifier name: (type_identifier) @sup.interface)
          (generic_type type: [(type_identifier) @sup.interface (scoped_type_identifier name: (type_identifier) @sup.interface)])]
  type: [(type_identifier) @sub
         (scoped_type_identifier name: (type_identifier) @sub)
         (generic_type type: [(type_identifier) @sub (scoped_type_identifier name: (type_identifier) @sub)])
         (reference_type type: [(type_identifier) @sub
                                (scoped_type_identifier name: (type_identifier) @sub)
                                (generic_type type: [(type_identifier) @sub (scoped_type_identifier name: (type_identifier) @sub)])])]) @site
(trait_item
  name: (type_identifier) @sub
  bounds: (trait_bounds [(type_identifier) @sup.interface
                         (scoped_type_identifier name: (type_identifier) @sup.interface)
                         (generic_type type: [(type_identifier) @sup.interface (scoped_type_identifier name: (type_identifier) @sup.interface)])])) @site
";

const PYTHON_RELATIONS: &str = "
(class_definition
  name: (identifier) @sub
  superclasses: (argument_list [(identifier) @sup (attribute attribute: (identifier) @sup) (subscript value: [(identifier) @sup (attribute attribute: (identifier) @sup)])])) @site
";

const TS_RELATIONS: &str = "
([(class_declaration name: (type_identifier) @sub (class_heritage (extends_clause value: [(identifier) @sup (member_expression property: (property_identifier) @sup)])))
  (abstract_class_declaration name: (type_identifier) @sub (class_heritage (extends_clause value: [(identifier) @sup (member_expression property: (property_identifier) @sup)])))]) @site
([(class_declaration name: (type_identifier) @sub (class_heritage (implements_clause [(type_identifier) @sup.interface (generic_type name: [(type_identifier) @sup.interface (nested_type_identifier name: (type_identifier) @sup.interface)]) (nested_type_identifier name: (type_identifier) @sup.interface)])))
  (abstract_class_declaration name: (type_identifier) @sub (class_heritage (implements_clause [(type_identifier) @sup.interface (generic_type name: [(type_identifier) @sup.interface (nested_type_identifier name: (type_identifier) @sup.interface)]) (nested_type_identifier name: (type_identifier) @sup.interface)])))]) @site
(interface_declaration name: (type_identifier) @sub
  (extends_type_clause type: [(type_identifier) @sup.interface (generic_type name: [(type_identifier) @sup.interface (nested_type_identifier name: (type_identifier) @sup.interface)]) (nested_type_identifier name: (type_identifier) @sup.interface)])) @site
";

const JS_RELATIONS: &str = "
(class_declaration name: (identifier) @sub
  (class_heritage [(identifier) @sup (member_expression property: (property_identifier) @sup)])) @site
";

const JAVA_RELATIONS: &str = "
(class_declaration name: (identifier) @sub
  superclass: (superclass [(type_identifier) @sup (generic_type (type_identifier) @sup) (generic_type (scoped_type_identifier (type_identifier) @sup .)) (scoped_type_identifier (type_identifier) @sup .)])) @site
([(class_declaration name: (identifier) @sub
    interfaces: (super_interfaces (type_list [(type_identifier) @sup.interface (generic_type (type_identifier) @sup.interface) (generic_type (scoped_type_identifier (type_identifier) @sup.interface .)) (scoped_type_identifier (type_identifier) @sup.interface .)])))
  (enum_declaration name: (identifier) @sub
    interfaces: (super_interfaces (type_list [(type_identifier) @sup.interface (generic_type (type_identifier) @sup.interface) (generic_type (scoped_type_identifier (type_identifier) @sup.interface .)) (scoped_type_identifier (type_identifier) @sup.interface .)])))
  (record_declaration name: (identifier) @sub
    interfaces: (super_interfaces (type_list [(type_identifier) @sup.interface (generic_type (type_identifier) @sup.interface) (generic_type (scoped_type_identifier (type_identifier) @sup.interface .)) (scoped_type_identifier (type_identifier) @sup.interface .)])))]) @site
(interface_declaration name: (identifier) @sub
  (extends_interfaces (type_list [(type_identifier) @sup.interface (generic_type (type_identifier) @sup.interface) (generic_type (scoped_type_identifier (type_identifier) @sup.interface .)) (scoped_type_identifier (type_identifier) @sup.interface .)]))) @site
";

const KOTLIN_RELATIONS: &str = "
([(class_declaration (type_identifier) @sub (delegation_specifier [(constructor_invocation [(user_type (type_identifier) @sup .) (user_type (type_identifier) @sup . (type_arguments) .)]) (user_type (type_identifier) @sup.interface .) (user_type (type_identifier) @sup.interface . (type_arguments) .)]))
  (object_declaration (type_identifier) @sub (delegation_specifier [(constructor_invocation [(user_type (type_identifier) @sup .) (user_type (type_identifier) @sup . (type_arguments) .)]) (user_type (type_identifier) @sup.interface .) (user_type (type_identifier) @sup.interface . (type_arguments) .)]))]) @site
";

const RUBY_RELATIONS: &str = "
(class name: [(constant) @sub (scope_resolution name: (constant) @sub)]
  superclass: (superclass [(constant) @sup (scope_resolution name: (constant) @sup)])) @site
(class name: [(constant) @sub (scope_resolution name: (constant) @sub)]
  body: (body_statement (call method: (identifier) @_include arguments: (argument_list [(constant) @sup.interface (scope_resolution name: (constant) @sup.interface)]) (#eq? @_include \"include\")))) @site
(module name: [(constant) @sub (scope_resolution name: (constant) @sub)]
  body: (body_statement (call method: (identifier) @_include arguments: (argument_list [(constant) @sup.interface (scope_resolution name: (constant) @sup.interface)]) (#eq? @_include \"include\")))) @site
";

struct Relations {
    query: Query,
    sub: u32,
    sup: u32,
    sup_interface: u32,
    site: u32,
}

fn relations_query(lang: Language) -> Option<&'static str> {
    use Language::*;
    Some(match lang {
        Rust => RUST_RELATIONS,
        Python => PYTHON_RELATIONS,
        TypeScript | Tsx => TS_RELATIONS,
        JavaScript => JS_RELATIONS,
        Java => JAVA_RELATIONS,
        Kotlin => KOTLIN_RELATIONS,
        Ruby => RUBY_RELATIONS,
        _ => return None,
    })
}

/// The relations query for `lang`, compiled once outside any syntax arena.
fn relations(lang: Language) -> Option<&'static Relations> {
    static RELATIONS: [OnceLock<Option<Relations>>; Language::ALL.len()] = [const { OnceLock::new() }; Language::ALL.len()];
    crate::syntax_arena::install();
    RELATIONS[lang as usize]
        .get_or_init(|| {
            let query = Query::new(&config(lang).language, relations_query(lang)?).ok()?;
            let index = |name: &str| query.capture_index_for_name(name);
            let (sub, sup, site) = (index("sub")?, index("sup").unwrap_or(u32::MAX), index("site")?);
            let sup_interface = index("sup.interface").unwrap_or(u32::MAX);
            Some(Relations { query, sub, sup, sup_interface, site })
        })
        .as_ref()
}

/// Whether [`type_relations_while`] supports `lang`.
pub fn has_type_relations(lang: Language) -> bool {
    relations(lang).is_some()
}

/// `text`'s subtype–supertype pairs, parsed on the syntax pool inside its arena; `None` when `lang` has none, or
/// once `current` says the work is obsolete.
pub fn type_relations_while(lang: Language, text: Arc<str>, current: impl Fn() -> bool + Send + 'static) -> Option<Vec<Relation>> {
    relations(lang)?;
    on_worker(move || relations_now(lang, &text, &current))
}

fn relations_now(lang: Language, text: &str, current: &dyn Fn() -> bool) -> Option<Vec<Relation>> {
    let rel = relations(lang)?;
    if !current() {
        return None;
    }
    let _arena = crate::syntax_arena::Scope::begin(text.len());
    let mut parser = Parser::new();
    parser.set_language(&config(lang).language).ok()?;
    let mut progress = |_: &tree_sitter::ParseState| if current() { ControlFlow::Continue(()) } else { ControlFlow::Break(()) };
    let tree = parser.parse_with_options(&mut |offset, _| text.as_bytes().get(offset..).unwrap_or_default(), None, Some(ParseOptions::new().progress_callback(&mut progress)))?;
    let mut out = Vec::new();
    {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&rel.query, tree.root_node(), text.as_bytes());
        while let Some(m) = matches.next() {
            let node = |index: u32| m.captures().iter().find(|c| c.index == index).map(|c| c.node);
            let (Some(sub), Some(site)) = (node(rel.sub), node(rel.site)) else { continue };
            if site.kind() == "impl_item" {
                // `impl !Send for S` says S is not one; in `impl<T> Show for T` the type is any type, not a declaration.
                let mut walk = site.walk();
                let negative = site.children(&mut walk).any(|c| c.kind() == "!");
                let name = sub.utf8_text(text.as_bytes()).ok();
                let blanket = site.child_by_field_name("type_parameters").is_some_and(|params| {
                    let mut walk = params.walk();
                    params.named_children(&mut walk).filter_map(|p| p.child_by_field_name("name")).any(|n| n.utf8_text(text.as_bytes()).ok() == name)
                });
                if negative || blanket {
                    continue;
                }
            }
            let (sup, sup_kind) = match (node(rel.sup), node(rel.sup_interface)) {
                (Some(n), _) => (n, DeclKind::Type),
                (None, Some(n)) => (n, DeclKind::Interface),
                (None, None) => continue,
            };
            let mut walk = site.walk();
            let interface_kw = site.children(&mut walk).any(|c| c.kind() == "interface");
            let sub_kind = match site.kind() {
                "impl_item" => DeclKind::Impl,
                "trait_item" | "interface_declaration" => DeclKind::Interface,
                "module" => DeclKind::Module,
                _ if interface_kw => DeclKind::Interface,
                _ => DeclKind::Type,
            };
            // The header: the site up to its body.
            let mut walk = site.walk();
            let body = site.child_by_field_name("body").or_else(|| site.children(&mut walk).find(|c| c.kind().ends_with("body") || c.kind() == "declaration_list"));
            let site = site.start_byte()..body.map_or(site.end_byte(), |b| b.start_byte());
            out.push(Relation { sub: sub.byte_range(), sup: sup.byte_range(), site, sub_kind, sup_kind });
        }
    }
    out.sort_by_key(|r| (r.sup.start, r.sub.start));
    out.dedup();
    current().then_some(out)
}

/// `text`'s declarations, parsed on the syntax pool inside its arena; `None` when `lang` has none, or once `current`
/// says the work is obsolete.
pub fn declarations_while(lang: Language, text: Arc<str>, current: impl Fn() -> bool + Send + 'static) -> Option<Declarations> {
    tags(lang)?;
    on_worker(move || declarations_now(lang, &text, &current))
}

fn declarations_now(lang: Language, text: &str, current: &dyn Fn() -> bool) -> Option<Declarations> {
    let tags = tags(lang)?;
    if !current() {
        return None;
    }
    let language = &config(lang).language;
    // Declared before the parser, tree and query cursors, so it is dropped after all of them.
    let _arena = crate::syntax_arena::Scope::begin(text.len());
    let mut parser = Parser::new();
    parser.set_language(language).ok()?;
    let mut progress = |_: &tree_sitter::ParseState| if current() { ControlFlow::Continue(()) } else { ControlFlow::Break(()) };
    let tree = parser.parse_with_options(
        &mut |offset, _| text.as_bytes().get(offset..).unwrap_or_default(),
        None,
        Some(ParseOptions::new().progress_callback(&mut progress)),
    )?;
    declarations_in(tags, lang, text, &tree, current)
}

/// The declarations in `tree`, a parse of `text`; `None` once `current` says the work is obsolete.
fn declarations_in(tags: &Tags, lang: Language, text: &str, tree: &tree_sitter::Tree, current: &dyn Fn() -> bool) -> Option<Declarations> {
    let mut found: Vec<Declaration> = Vec::new();
    {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&tags.query, tree.root_node(), text.as_bytes());
        let mut seen = 0usize;
        while let Some(m) = matches.next() {
            seen += 1;
            if seen.is_multiple_of(4096) && !current() {
                return None;
            }
            let (Some(name), Some((decl, kind))) = (
                m.captures().iter().find(|c| c.index == tags.name).map(|c| c.node.byte_range()),
                m.captures().iter().find_map(|c| tags.kinds[c.index as usize].map(|k| (c.node, k))),
            ) else {
                continue;
            };
            // Rust's upstream query calls every function in a `{ … }` item list a method; only `impl` and `trait`
            // bodies hold methods (a `mod` body holds functions).
            let kind = match (lang, kind) {
                (Language::Rust, DeclKind::Method)
                    if decl.parent().and_then(|list| list.parent()).is_some_and(|owner| !matches!(owner.kind(), "impl_item" | "trait_item")) =>
                {
                    DeclKind::Function
                }
                _ => kind,
            };
            let (first_line, last_line) = lines_of(decl);
            let name = match kind {
                // The header up to the body: `impl<T> Show for S`.
                DeclKind::Impl => decl.start_byte()..decl.child_by_field_name("body").map_or(decl.end_byte(), |b| b.start_byte()),
                _ => name,
            };
            let decl = decl.byte_range();
            // Headings and impl headers carry surrounding spaces: their name range covers the trimmed text, and the name
            // joins the header's lines with single spaces. Other names stay as written.
            let header = matches!(kind, DeclKind::Impl | DeclKind::Heading);
            let Some(raw) = text.get(name.clone()) else { continue };
            let lead = if header { raw.len() - raw.trim_start().len() } else { 0 };
            let name = name.start + lead..name.start + lead + if header { raw.trim().len() } else { raw.len() };
            let Some(name_text) = text.get(name.clone()) else { continue };
            let name_text: String = if header { name_text.split_whitespace().collect::<Vec<_>>().join(" ") } else { name_text.to_owned() };
            let name_text: Box<str> = name_text.chars().take(NAME_CHARS).collect::<String>().into();
            // JS/TS components: capitalised function or constant declarations, by convention.
            let component = matches!(lang, Language::JavaScript | Language::TypeScript | Language::Tsx)
                && matches!(kind, DeclKind::Function | DeclKind::Constant)
                && name_text.starts_with(|c: char| c.is_uppercase());
            found.push(Declaration {
                kind: if component { DeclKind::Component } else { kind },
                name: name_text,
                name_range: name.start as u32..name.end as u32,
                range: decl.start as u32..decl.end as u32,
                first_line,
                last_line,
                depth: 0,
                parent: None,
            });
        }
    }
    let syntax_errors = tree.root_node().has_error();
    // Source order; one record per name (combined queries may match a declaration twice); nesting depth and parent.
    found.sort_by_key(|d| (d.name_range.start, d.name_range.end));
    found.dedup_by(|b, a| a.name_range == b.name_range);
    found.sort_by_key(|d| (d.range.start, std::cmp::Reverse(d.range.end)));
    let mut open: Vec<(u32, u32)> = Vec::new();
    for (i, d) in found.iter_mut().enumerate() {
        open.retain(|&(end, _)| end > d.range.start);
        d.depth = open.len() as u16;
        d.parent = open.last().map(|&(_, p)| p);
        open.push((d.range.end, i as u32));
    }
    found.shrink_to_fit();
    Some(Declarations { items: found, syntax_errors })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dropped parse tree must leave the process, on every platform: tree-sitter builds ~90 MiB
    /// of nodes for a 100k-line file, and libmalloc kept about half of that charged after the
    /// first teardown of concurrent parses (macOS 27, 2026-10-11) until a later parse reused it.
    #[test]
    #[ignore = "process-wide memory: other tests skew it; scripts/ci-local.sh runs it alone"]
    fn concurrent_parses_return_the_trees_memory_to_the_system() {
        let text: Arc<str> = (0..60_000).map(|i| format!("fn compute_{i}(a: u32) -> u32 {{ a + {i} }} // {i}\n")).collect::<String>().into();
        let _warm = parse(Rust, "fn a() {}\n"); // compile the query outside the measurement
        let before = crate::memory_probe::footprint_mib().expect("memory probe");
        let threads: Vec<_> = (0..2).map(|_| { let t = text.clone(); std::thread::spawn(move || parse_while(Rust, t, || true).unwrap()) }).collect();
        let parsed: Vec<Parsed> = threads.into_iter().map(|h| h.join().unwrap()).collect();
        let held = crate::memory_probe::footprint_mib().unwrap();
        let spans_mib = parsed.iter().map(Parsed::bytes).sum::<usize>() as f64 / (1024.0 * 1024.0);
        drop(parsed);
        std::thread::sleep(std::time::Duration::from_millis(50)); // workers release after replying
        let after = crate::memory_probe::footprint_mib().unwrap();
        // Concurrent tests add noise of a few MiB; without the arena this retained over 100 MiB.
        assert!(after - before < 24.0, "retained {:.1} MiB after two parses (held {:.1} with the spans, spans {spans_mib:.1})", after - before, held - before);
    }
    /// Declaration extraction parses inside the same arena: its tree leaves the process too, and only the compact
    /// records stay.
    #[test]
    #[ignore = "process-wide memory: other tests skew it; scripts/ci-local.sh runs it alone"]
    fn declaration_extraction_returns_the_trees_memory_to_the_system() {
        let text: Arc<str> = (0..60_000).map(|i| format!("pub fn compute_{i}(a: u32) -> u32 {{ a + {i} }}\n")).collect::<String>().into();
        let _warm = declarations_while(Rust, "fn a() {}\n".into(), || true);
        let before = crate::memory_probe::footprint_mib().expect("memory probe");
        let found = declarations_while(Rust, text, || true).unwrap();
        assert_eq!(found.items.len(), 60_000);
        let records = found.items.iter().map(Declaration::bytes).sum::<usize>() as f64 / (1024.0 * 1024.0);
        std::thread::sleep(std::time::Duration::from_millis(50));
        let after = crate::memory_probe::footprint_mib().unwrap();
        assert!(after - before < records + 16.0, "retained {:.1} MiB for {records:.1} MiB of records", after - before);
    }
    /// Fold regions come from the highlight parse inside its arena: the tree leaves the process, only the compact
    /// regions stay.
    #[test]
    #[ignore = "process-wide memory: other tests skew it; scripts/ci-local.sh runs it alone"]
    fn fold_parses_return_the_trees_memory_to_the_system() {
        let text: Arc<str> = (0..60_000).map(|i| format!("fn compute_{i}(a: u32) -> u32 {{\n    a + {i}\n}}\n")).collect::<String>().into();
        let _warm = parse_folded_while(Rust, "fn a() {\n}\n".into(), || true);
        let before = crate::memory_probe::footprint_mib().expect("memory probe");
        let (parsed, folds) = parse_folded_while(Rust, text, || true).unwrap();
        assert_eq!(folds.regions.len(), 60_000);
        let kept = (parsed.bytes() + folds.bytes()) as f64 / (1024.0 * 1024.0);
        std::thread::sleep(std::time::Duration::from_millis(50));
        let after = crate::memory_probe::footprint_mib().unwrap();
        assert!(after - before < kept + 16.0, "retained {:.1} MiB for {kept:.1} MiB of spans and regions", after - before);
    }
    /// Selection steps parse inside the arena too: repeated expansions keep only their short chains.
    #[test]
    #[ignore = "process-wide memory: other tests skew it; scripts/ci-local.sh runs it alone"]
    fn selection_steps_return_the_trees_memory_to_the_system() {
        let text: Arc<str> = (0..60_000).map(|i| format!("fn compute_{i}(a: u32) -> u32 {{\n    (a + {i}) * 2\n}}\n")).collect::<String>().into();
        let _warm = selection_steps_while(Rust, "fn a() {}\n".into(), 3..3, || true);
        let before = crate::memory_probe::footprint_mib().expect("memory probe");
        for at in [100, 500_000, 1_500_000] {
            assert!(selection_steps_while(Rust, text.clone(), at..at, || true).is_some_and(|s| s.len() < 64));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        let after = crate::memory_probe::footprint_mib().unwrap();
        assert!(after - before < 16.0, "retained {:.1} MiB after three expansions", after - before);
    }
    use Capture::{Comment, Escape, Function, Heading, Keyword, Number, Property, Tag, TypeBuiltin};
    use Language::*;

    fn d(path: &str, first: &str) -> Option<Language> {
        detect(Path::new(path), first)
    }

    #[test]
    fn detects_by_extension() {
        let table = [
            ("src/main.rs", Some(Rust)),
            ("a.c", Some(C)),
            ("a.h", Some(Cpp)),
            ("a.cpp", Some(Cpp)),
            ("a.hpp", Some(Cpp)),
            ("a.cc", Some(Cpp)),
            ("a.go", Some(Go)),
            ("a.zig", Some(Zig)),
            ("A.java", Some(Java)),
            ("a.kt", Some(Kotlin)),
            ("build.gradle.kts", Some(Kotlin)),
            ("a.cs", Some(CSharp)),
            ("a.py", Some(Python)),
            ("a.pyi", Some(Python)),
            ("a.rb", Some(Ruby)),
            ("a.php", Some(Php)),
            ("a.lua", Some(Lua)),
            ("a.sh", Some(Bash)),
            ("a.bash", Some(Bash)),
            ("a.js", Some(JavaScript)),
            ("a.mjs", Some(JavaScript)),
            ("a.jsx", Some(JavaScript)),
            ("a.ts", Some(TypeScript)),
            ("a.mts", Some(TypeScript)),
            ("a.tsx", Some(Tsx)),
            ("a.html", Some(Html)),
            ("a.htm", Some(Html)),
            ("a.css", Some(Css)),
            ("a.json", Some(Json)),
            ("a.yaml", Some(Yaml)),
            ("a.yml", Some(Yaml)),
            ("a.toml", Some(Toml)),
            ("a.sql", Some(Sql)),
            ("README.md", Some(Markdown)),
            ("a.markdown", Some(Markdown)),
            ("rules.mk", Some(Makefile)),
            ("app.dockerfile", Some(Dockerfile)),
            ("A.RS", Some(Rust)),
            ("a.txt", None),
            ("a", None),
            ("a.rs.orig", None),
        ];
        for (path, want) in table {
            assert_eq!(d(path, ""), want, "{path}");
        }
    }

    #[test]
    fn detects_by_file_name() {
        let table = [
            ("Dockerfile", Some(Dockerfile)),
            ("docker/Dockerfile.dev", Some(Dockerfile)),
            ("Containerfile", Some(Dockerfile)),
            ("Makefile", Some(Makefile)),
            ("makefile", Some(Makefile)),
            ("GNUmakefile", Some(Makefile)),
            ("Cargo.lock", Some(Toml)),
            ("poetry.lock", Some(Toml)),
            ("uv.lock", Some(Toml)),
            ("composer.lock", Some(Json)),
            ("flake.lock", Some(Json)),
            ("yarn.lock", None),
            ("Gemfile.lock", None),
            ("Gemfile", Some(Ruby)),
            ("Rakefile", Some(Ruby)),
            ("PKGBUILD", Some(Bash)),
            (".bashrc", Some(Bash)),
            (".zshrc", Some(Bash)),
        ];
        for (path, want) in table {
            assert_eq!(d(path, ""), want, "{path}");
        }
    }

    #[test]
    fn detects_by_shebang_only_as_fallback() {
        let table = [
            ("#!/usr/bin/env python3", Some(Python)),
            ("#!/usr/bin/python3.12 -u", Some(Python)),
            ("#!/usr/bin/env -S python3 -u", Some(Python)),
            ("#!/bin/bash", Some(Bash)),
            ("#!/bin/sh -e", Some(Bash)),
            ("#! /usr/bin/env zsh", Some(Bash)),
            ("#!/usr/bin/env node", Some(JavaScript)),
            ("#!/usr/bin/env ruby", Some(Ruby)),
            ("#!/usr/bin/env php", Some(Php)),
            ("#!/usr/bin/env lua", Some(Lua)),
            ("#!/usr/bin/make -f", Some(Makefile)),
            ("#!/usr/bin/env perl", None),
            ("# just a comment", None),
            ("", None),
        ];
        for (first, want) in table {
            assert_eq!(d("bin/tool", first), want, "{first}");
        }
        // The file name wins over the shebang.
        assert_eq!(d("tool.rb", "#!/bin/bash"), Some(Ruby));
    }

    #[test]
    fn every_grammar_loads_and_its_query_compiles() {
        for lang in Language::ALL {
            // `config` panics with the query error if a query does not compile.
            assert!(!config(lang).captures.is_empty(), "{lang:?}");
            parse(lang, "x\n");
        }
    }

    #[test]
    fn every_language_highlights_a_keyword_or_literal() {
        let samples = [
            (Rust, "pub struct A;", "pub", Keyword),
            (C, "return 0;", "return", Keyword),
            (Cpp, "class A {};", "class", Keyword),
            (Go, "package main", "package", Keyword),
            (Zig, "const x = 1;", "const", Keyword),
            (Java, "class A {}", "class", Keyword),
            (Kotlin, "fun main() {}", "fun", Keyword),
            (CSharp, "class A {}", "class", Keyword),
            (Python, "import os", "import", Keyword),
            (Ruby, "def f; end", "def", Keyword),
            (Php, "<?php echo 1;", "echo", Keyword),
            (Lua, "local x = 1", "local", Keyword),
            (Bash, "if true; then :; fi", "if", Keyword),
            (JavaScript, "const a = <div/>;", "const", Keyword),
            (TypeScript, "interface A {}", "interface", Keyword),
            (Tsx, "const a = <div/>;", "const", Keyword),
            (Html, "<p class=\"a\">", "p", Tag),
            (Css, "a { color: red; }", "color", Property),
            (Json, "{\"a\": 1}", "1", Number),
            (Yaml, "a: 1", "1", Number),
            (Toml, "a = 1", "1", Number),
            (Sql, "SELECT 1", "SELECT", Keyword),
            (Markdown, "# T\n", "T", Heading),
            (Dockerfile, "FROM alpine", "FROM", Keyword),
            (Makefile, "ifdef X\nendif", "ifdef", Keyword),
        ];
        for (lang, src, token, capture) in samples {
            let got = tokens(&parse(lang, src), src, 0);
            assert!(has(&got, token, capture), "{lang:?}: {got:?}");
        }
    }

    #[test]
    fn every_language_survives_empty_unicode_crlf_and_lossy_text() {
        let samples = ["", "\n", "é \u{FFFD}\u{FFFD} 日本 🦀\r\n\"\u{FFFD}\r\n", "\r\n\r\n", "(\u{FFFD}"];
        for lang in Language::ALL {
            for src in samples {
                let p = parse(lang, src);
                let starts = crate::lines::line_starts(src);
                let spans = p.spans(0..starts.len());
                assert_eq!(spans.len(), starts.len(), "{lang:?} {src:?}");
                for (l, line) in spans.iter().enumerate() {
                    let r = crate::lines::line_range(src, &starts, l);
                    let text = &src[r.clone()];
                    for (span, _) in line {
                        assert!(span.end <= text.len() && text.is_char_boundary(span.start) && text.is_char_boundary(span.end), "{lang:?} {src:?} line {l}: {span:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn lossy_decoded_crlf_text_highlights_in_source_coordinates() {
        let decoded = crate::encoding::decode(b"// \xFF\r\nfn caf\xC3\xA9() {}\r\n", Some("utf-8"));
        assert!(decoded.lossy);
        let src = decoded.text.as_str();
        let p = parse(Rust, src);
        assert_eq!(tokens(&p, src, 0), vec![("// \u{FFFD}".to_string(), Comment)]);
        assert!(has(&tokens(&p, src, 1), "café", Function), "{:?}", tokens(&p, src, 1));
    }


    fn decls(lang: Language, src: &str) -> (Vec<(DeclKind, String, u16)>, bool) {
        let d = declarations_while(lang, src.into(), || true).unwrap();
        for x in &d.items {
            assert_eq!(&src[x.name_range.start as usize..x.name_range.end as usize], &*x.name, "{lang:?}: the name range is the name");
            assert!(x.range.start <= x.name_range.start && x.name_range.end <= x.range.end, "{lang:?}: {x:?}");
        }
        (d.items.into_iter().map(|x| (x.kind, x.name.into(), x.depth)).collect(), d.syntax_errors)
    }

    fn decl(found: &[(DeclKind, String, u16)], kind: DeclKind, name: &str, depth: u16) -> bool {
        found.iter().any(|(k, n, d)| *k == kind && n == name && *d == depth)
    }

    fn sticky_names(lang: Language, src: &str, top: u32, partial: bool) -> Vec<(String, u32, u32)> {
        let s = parse_with(lang, src.into(), Want { folds: false, declarations: true }, || true).unwrap();
        let d = s.declarations.unwrap();
        d.sticky(top, partial, 5).iter().map(|k| (d.items[k.index as usize].name.to_string(), k.line, k.row)).collect()
    }

    #[test]
    fn other_names_keep_their_text_as_written() {
        let (found, _) = decls(Css, ".wide,\n.tall { color: red }\n");
        assert!(decl(&found, DeclKind::Selector, ".wide,\n.tall", 0), "{found:?}");
    }

    #[test]
    fn rust_impl_blocks_are_declarations_that_hold_their_methods() {
        let src = "struct S;\nimpl<T> Show for S {\n    fn show(&self) {\n        1;\n    }\n}\n";
        let (found, _) = decls(Rust, src);
        assert!(decl(&found, DeclKind::Impl, "impl<T> Show for S", 0), "{found:?}");
        assert!(decl(&found, DeclKind::Method, "show", 1), "{found:?}");
    }

    #[test]
    fn markdown_headings_nest_by_level() {
        let src = "# One\n\ntext\n\n## Two\n\nmore\n\n# Three\n";
        let (found, _) = decls(Markdown, src);
        assert!(decl(&found, DeclKind::Heading, "One", 0), "{found:?}");
        assert!(decl(&found, DeclKind::Heading, "Two", 1), "{found:?}");
        assert!(decl(&found, DeclKind::Heading, "Three", 0), "{found:?}");
    }

    #[test]
    fn a_header_sticks_once_its_first_line_scrolls_past() {
        let src = "mod m {\n    fn f() {\n        a;\n        b;\n    }\n}\n";
        assert_eq!(sticky_names(Rust, src, 0, false), vec![], "the top line is the header itself");
        assert_eq!(sticky_names(Rust, src, 0, true), vec![("m".into(), 0, 0)], "wrapped continuation rows of the header");
        assert_eq!(sticky_names(Rust, src, 2, false), vec![("m".into(), 0, 0), ("f".into(), 1, 1)]);
    }

    #[test]
    fn a_header_is_pushed_up_by_its_scopes_end() {
        let src = "mod m {\n    fn f() {\n        a;\n    }\n    fn g() {}\n}\n";
        // Top line 3 is f's closing line: f would move up to row 3 - 3 = 0, which m holds, so f is dropped.
        assert_eq!(sticky_names(Rust, src, 3, false), vec![("m".into(), 0, 0)]);
        assert_eq!(sticky_names(Rust, src, 2, false), vec![("m".into(), 0, 0), ("f".into(), 1, 1)]);
    }

    #[test]
    fn deep_nesting_keeps_the_innermost_five() {
        let mut src = String::new();
        for i in 0..7 {
            src += &format!("{}mod m{i} {{\n", "    ".repeat(i));
        }
        src += &format!("{}x;\n", "    ".repeat(7)).repeat(10); // a body taller than the stack, so no header is pushed up
        for i in (0..7).rev() {
            src += &format!("{}}}\n", "    ".repeat(i));
        }
        let names: Vec<String> = sticky_names(Rust, &src, 7, false).into_iter().map(|(n, _, _)| n).collect();
        assert_eq!(names, ["m2", "m3", "m4", "m5", "m6"]);
    }

    #[test]
    fn enclosing_walks_outermost_first() {
        let src = "mod m {\n    fn f() {\n        a;\n    }\n}\n";
        let s = parse_with(Rust, src.into(), Want { folds: false, declarations: true }, || true).unwrap();
        let d = s.declarations.unwrap();
        let at = src.find("a;").unwrap() as u32;
        let names: Vec<&str> = d.enclosing(at).iter().map(|x| &*x.name).collect();
        assert_eq!(names, ["m", "f"]);
        assert_eq!(d.enclosing_line(2).len(), 2);
        assert!(d.enclosing(0).iter().map(|x| &*x.name).eq(["m"]));
    }

    #[test]
    fn declarations_are_off_unless_asked_and_crlf_lines_count() {
        let src = "mod m {\r\n    fn f() {\r\n        a;\r\n    }\r\n}\r\n";
        let off = parse_with(Rust, src.into(), Want { folds: true, declarations: false }, || true).unwrap();
        assert!(off.declarations.is_none() && off.folds.is_some());
        assert_eq!(sticky_names(Rust, src, 2, false), vec![("m".into(), 0, 0), ("f".into(), 1, 1)]);
    }

    #[test]
    fn rust_modules_types_traits_and_functions_nest() {
        let src = "mod outer {\n    pub struct Café;\n    pub trait Greet { fn hi(&self); }\n    impl Café { pub fn new() -> Self { Café } }\n    fn helper() {}\n}\nmacro_rules! m { () => {} }\nfn helper() {}\n";
        let (found, errors) = decls(Rust, src);
        assert!(!errors);
        assert!(decl(&found, DeclKind::Module, "outer", 0));
        assert!(decl(&found, DeclKind::Type, "Café", 1), "{found:?}");
        assert!(decl(&found, DeclKind::Interface, "Greet", 1));
        assert!(decl(&found, DeclKind::Impl, "impl Café", 1) && decl(&found, DeclKind::Method, "new", 2), "an impl holds its methods: {found:?}");
        assert!(decl(&found, DeclKind::Function, "helper", 1) && decl(&found, DeclKind::Function, "helper", 0), "duplicate names stay apart");
        assert!(decl(&found, DeclKind::Macro, "m", 0));
    }

    #[test]
    fn python_ruby_java_and_kotlin_declarations() {
        let (py, _) = decls(Python, "class Ä:\n    def m(self):\n        def inner(): pass\n\ndef f(): pass\n");
        assert!(decl(&py, DeclKind::Type, "Ä", 0) && decl(&py, DeclKind::Function, "m", 1) && decl(&py, DeclKind::Function, "inner", 2) && decl(&py, DeclKind::Function, "f", 0), "{py:?}");
        let (rb, _) = decls(Ruby, "module M\n  class C\n    def go; end\n    def self.make; end\n  end\nend\n");
        assert!(decl(&rb, DeclKind::Module, "M", 0) && decl(&rb, DeclKind::Type, "C", 1) && decl(&rb, DeclKind::Method, "go", 2) && decl(&rb, DeclKind::Method, "make", 2), "{rb:?}");
        let (java, _) = decls(Java, "interface I { void a(); }\nclass C implements I {\n  public void a() {}\n  class Inner {}\n}\n");
        assert!(decl(&java, DeclKind::Interface, "I", 0) && decl(&java, DeclKind::Type, "C", 0) && decl(&java, DeclKind::Method, "a", 1) && decl(&java, DeclKind::Type, "Inner", 1), "{java:?}");
        let (kt, _) = decls(Kotlin, "class Box(val x: Int) {\n  fun open() {}\n  companion object Factory { fun make() = Box(1) }\n  val size = 2\n}\nobject Single\ntypealias B = Box\nval top = 1\ninterface Shape { fun area(): Int }\nfun f() { val local = 1 }\n");
        assert!(decl(&kt, DeclKind::Type, "Box", 0) && decl(&kt, DeclKind::Function, "open", 1) && decl(&kt, DeclKind::Type, "Single", 0) && decl(&kt, DeclKind::Type, "B", 0) && decl(&kt, DeclKind::Constant, "top", 0), "{kt:?}");
        assert!(kt.iter().any(|(_, n, _)| n == "make"), "{kt:?}");
        assert!(decl(&kt, DeclKind::Interface, "Shape", 0) && decl(&kt, DeclKind::Constant, "size", 1), "{kt:?}");
        assert!(!kt.iter().any(|(_, n, _)| n == "local"), "a function's locals are not declarations: {kt:?}");
    }

    #[test]
    fn javascript_typescript_and_tsx_with_named_components() {
        let (js, _) = decls(JavaScript, "class A { run() {} }\nfunction helper() {}\nconst Button = () => <b/>;\nfunction App() { return <Button/>; }\n");
        assert!(decl(&js, DeclKind::Type, "A", 0) && decl(&js, DeclKind::Method, "run", 1) && decl(&js, DeclKind::Function, "helper", 0), "{js:?}");
        assert!(decl(&js, DeclKind::Component, "Button", 0) && decl(&js, DeclKind::Component, "App", 0), "{js:?}");
        let (ts, _) = decls(TypeScript, "interface Shape { area(): number }\nnamespace Geo { export function circle() {} }\nclass Sq implements Shape { area() { return 1 } }\n");
        assert!(decl(&ts, DeclKind::Interface, "Shape", 0) && decl(&ts, DeclKind::Module, "Geo", 0) && decl(&ts, DeclKind::Function, "circle", 1) && decl(&ts, DeclKind::Type, "Sq", 0), "{ts:?}");
        let (tsx, _) = decls(Tsx, "export function Card(): JSX.Element { return <div/>; }\nconst useThing = () => 1;\n");
        assert!(decl(&tsx, DeclKind::Component, "Card", 0) && decl(&tsx, DeclKind::Function, "useThing", 0), "{tsx:?}");
    }

    #[test]
    fn html_ids_and_css_selectors() {
        let (html, _) = decls(Html, "<main id=\"app\"><section id=\"größe\"><p>x</p></section><img id=\"logo\"/></main>\n");
        let ids: Vec<&str> = html.iter().map(|(_, n, _)| n.as_str()).collect();
        assert_eq!(ids, ["app", "größe", "logo"]);
        assert!(decl(&html, DeclKind::Element, "größe", 1));
        let (css, _) = decls(Css, ".a, .b > p { color: red }\n@media (min-width: 1px) { #main { x: y } }\n");
        assert!(decl(&css, DeclKind::Selector, ".a, .b > p", 0) && css.iter().any(|(k, n, _)| *k == DeclKind::Selector && n == "#main"), "{css:?}");
    }

    #[test]
    fn every_declaration_query_compiles() {
        for lang in Language::ALL.into_iter().filter(|&l| has_declarations(l)) {
            assert!(tags(lang).is_some(), "{lang:?} tags query");
        }
    }

    #[test]
    fn incomplete_source_still_yields_what_parsed_and_says_so() {
        let (found, errors) = decls(Rust, "fn ok() {}\nfn broken( {\nstruct After;\n");
        assert!(errors);
        assert!(decl(&found, DeclKind::Function, "ok", 0), "{found:?}");
        assert!(declarations_while(Json, "{}".into(), || true).is_none(), "unsupported languages say so");
        assert!(!has_declarations(Json) && has_declarations(Css) && has_declarations(Markdown));
        assert!(declarations_while(Rust, "fn a() {}".into(), || false).is_none(), "obsolete work stops");
    }

    #[test]
    fn parsed_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Parsed>();
    }

    /// The text of each span on `line`, with its capture.
    fn tokens(p: &Parsed, text: &str, line: usize) -> Vec<(String, Capture)> {
        let starts = crate::lines::line_starts(text);
        let line_text = &text[crate::lines::line_range(text, &starts, line)];
        p.spans(line..line + 1)
            .remove(0)
            .into_iter()
            .map(|(r, c)| (line_text[r].to_string(), c))
            .collect()
    }

    fn has(tokens: &[(String, Capture)], text: &str, capture: Capture) -> bool {
        tokens.iter().any(|(t, c)| t == text && *c == capture)
    }

    #[test]
    fn rust_tokens() {
        let src = "// note\nfn main() {\n    let s = \"hi\\n\";\n    let n = 42;\n}\n";
        let p = parse(Rust, src);
        assert_eq!(tokens(&p, src, 0), vec![("// note".to_string(), Comment)]);
        let l1 = tokens(&p, src, 1);
        assert!(has(&l1, "fn", Keyword), "{l1:?}");
        assert!(has(&l1, "main", Function), "{l1:?}");
        let l2 = tokens(&p, src, 2);
        assert!(has(&l2, "let", Keyword), "{l2:?}");
        assert!(has(&l2, "\"hi", Capture::String), "{l2:?}");
        assert!(has(&l2, "\\n", Escape), "{l2:?}");
        assert!(has(&tokens(&p, src, 3), "42", Number));
    }

    #[test]
    fn python_and_typescript_tokens() {
        let src = "def f(x):\n    return None  # done\n";
        let p = parse(Python, src);
        assert!(has(&tokens(&p, src, 0), "def", Keyword));
        let l1 = tokens(&p, src, 1);
        assert!(has(&l1, "return", Keyword), "{l1:?}");
        assert!(has(&l1, "# done", Comment), "{l1:?}");

        let src = "function f(x: number): string { return 'a'; }\n";
        let l0 = tokens(&parse(TypeScript, src), src, 0);
        assert!(has(&l0, "function", Keyword), "{l0:?}");
        assert!(has(&l0, "number", TypeBuiltin), "{l0:?}");
        assert!(has(&l0, "'a'", Capture::String), "{l0:?}");
    }

    #[test]
    fn markdown_code_fence_content_stays_plain() {
        let src = "# Title\n\n```\ncode\n```\n";
        let p = parse(Markdown, src);
        assert!(has(&tokens(&p, src, 0), "Title", Heading));
        assert_eq!(tokens(&p, src, 3), vec![]);
    }

    #[test]
    fn multi_line_constructs_cover_every_line() {
        let src = "let a = 1;\n/* one\n   two\n   three */\nlet b = \"x\ny\nz\";\n";
        let p = parse(Rust, src);
        // Requested window starts inside the comment: the middle line has no node starting in it.
        assert_eq!(p.spans(2..3), vec![vec![(0..6, Comment)]]);
        assert_eq!(p.spans(3..4)[0][0], (0..11, Comment));
        assert_eq!(p.spans(5..6), vec![vec![(0..1, Capture::String)]]);

        let src = "x = \"\"\"\ndoc\n\"\"\"\n";
        let p = parse(Python, src);
        assert_eq!(p.spans(1..2), vec![vec![(0..3, Capture::String)]]);
    }

    #[test]
    fn crlf_and_line_ends_are_excluded() {
        let src = "// a\r\nfn x() {}\r\n";
        let p = parse(Rust, src);
        let spans = p.spans(0..3);
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0], vec![(0..4, Comment)]);
        assert!(spans[2].is_empty());
        // Out-of-range windows are clamped.
        assert_eq!(p.spans(2..10).len(), 1);
        assert!(p.spans(5..10).is_empty());
    }

    fn big_rust(copies: usize) -> String {
        let unit = "/// Doc for item.\npub fn item(a: u32) -> Option<String> {\n    /* block\n       comment */\n    let s = \"multi\nline\";\n    if a > 0 { Some(format!(\"{a}\")) } else { None }\n}\n\n";
        unit.repeat(copies)
    }

    #[test]
    fn window_matches_whole_file_restricted() {
        let src = big_rust(300);
        let p = parse(Rust, &src);
        let n = crate::lines::line_starts(&src).len();
        let mut parser = Parser::new();
        parser.set_language(&config(Rust).language).unwrap();
        let tree = SyntaxTree {
            lang: Rust, text: &src, tree: parser.parse(&src, None).unwrap(),
            line_starts: crate::lines::line_starts(&src),
        };
        let whole = tree.spans(0..n);
        assert_eq!(p.spans(0..n), whole, "chunk boundaries preserve the original full-tree highlighting");
        assert_eq!(whole.len(), n);
        for window in [1000..1060, 1003..1004, 1004..1063, n - 5..n] {
            assert_eq!(p.spans(window.clone()), whole[window.clone()], "{window:?}");
        }
    }

    #[test]
    fn obsolete_parses_stop_without_poisoning_the_worker() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        assert!(parse_while(Rust, "fn old() {}".into(), || false).is_none());
        let calls = AtomicUsize::new(0);
        assert!(parse_while(Rust, big_rust(1000).into(), move || calls.fetch_add(1, Ordering::Relaxed) < 2).is_none());
        assert!(parse_while(Rust, "fn current() {}".into(), || true).is_some());
    }

    /// `cargo test --release -p lenz-content -- --ignored --nocapture perf`
    #[test]
    #[ignore]
    fn perf() {
        use std::time::Instant;
        let src = match std::env::var("LENZ_PERF_FILE") {
            Ok(path) => std::fs::read_to_string(path).unwrap(),
            Err(_) => big_rust(100_000 / 9 + 1),
        };
        let n = crate::lines::line_starts(&src).len();
        let t = Instant::now();
        config(Rust);
        let compile = t.elapsed();
        let t = Instant::now();
        let p = parse(Rust, &src);
        let parsed = t.elapsed();
        let t = Instant::now();
        let mut count = 0;
        for start in (0..n.saturating_sub(60)).step_by(n / 100) {
            count += p.spans(start..start + 60).iter().map(Vec::len).sum::<usize>();
        }
        let per_window = t.elapsed() / 100;
        crate::perf::record("syntax/query-compile", "ms", &[compile.as_secs_f64() * 1000.0]);
        crate::perf::record("syntax/parse", "ms", &[parsed.as_secs_f64() * 1000.0]);
        crate::perf::record("syntax/visible-spans", "ms", &[per_window.as_secs_f64() * 1000.0]);
        crate::perf::record("syntax/retained", "MiB", &[p.bytes() as f64 / (1 << 20) as f64]);
        // The same parse with folds and declarations (sticky headers), and sticky lookups over the result.
        let t = Instant::now();
        let full = parse_with(Rust, src.as_str().into(), Want { folds: true, declarations: true }, || true).unwrap();
        let with_declarations = t.elapsed();
        let d = full.declarations.unwrap_or_default();
        let t = Instant::now();
        let mut pinned = 0;
        for top in (0..n as u32).step_by((n / 1000).max(1)) {
            pinned += d.sticky(top, false, 5).len();
        }
        let per_lookup = t.elapsed() / 1000;
        crate::perf::record("syntax/parse-declarations", "ms", &[with_declarations.as_secs_f64() * 1000.0]);
        crate::perf::record("syntax/sticky-lookup", "us", &[per_lookup.as_secs_f64() * 1e6]);
        eprintln!("parse with folds and {} declarations {with_declarations:?}, sticky lookup {per_lookup:?} ({pinned} pinned)", d.items.len());
        crate::perf::check(per_lookup.as_micros() < 50, "sticky lookup exceeds 50 us");
        eprintln!(
            "{n} lines, {} KB: query compile {compile:?}, parse {parsed:?}, spans(60 lines) {per_window:?} ({count} spans over 100 windows)",
            src.len() / 1024
        );
        eprintln!("retained highlights: {:.1} MiB", p.bytes() as f64 / (1 << 20) as f64);
        assert!(p.bytes() < src.len() * 5, "highlight cache must stay compact");
        crate::perf::check(per_window.as_micros() < 250, "visible syntax lookup exceeds 250 us");
    }

    fn folds(lang: Language, src: &str) -> Vec<(u32, u32, FoldKind, bool)> {
        let (_, f) = parse_folded_while(lang, src.into(), || true).unwrap();
        f.regions.iter().map(|r| (r.first, r.last, r.kind, r.closes)).collect()
    }

    #[test]
    fn rust_blocks_comments_and_imports_fold_outermost_per_line() {
        let src = "use a::b;\nuse c::d;\n\n// one\n// two\nfn f() {\n    let x = [\n        1,\n    ];\n}\nfn g() {}\n";
        assert_eq!(folds(Rust, src), vec![
            (0, 1, FoldKind::Imports, false),
            (3, 4, FoldKind::Comment, false),
            (5, 9, FoldKind::Block, true),
            (6, 8, FoldKind::Block, true),
        ]);
    }

    #[test]
    fn comments_after_code_never_fold_the_code() {
        assert_eq!(folds(Rust, "fn f() {\n    let a = 1; // x\n    let b = 2; // y\n    let c = 3;\n}\n"), vec![(0, 4, FoldKind::Block, true)]);
        assert_eq!(folds(Python, "x = 1  # a\ny = 2  # b\n"), vec![]);
        assert_eq!(folds(Rust, "let a = 1; // x\n// y\n// z\n"), vec![(1, 2, FoldKind::Comment, false)]);
    }

    #[test]
    fn a_multi_line_import_closes_like_a_block() {
        assert_eq!(folds(Rust, "use a::{\n    b,\n};\nfn f() {}\n"), vec![(0, 2, FoldKind::Imports, true)]);
    }

    #[test]
    fn python_bodies_fold_without_a_closing_line() {
        let src = "import os\nimport sys\ndef f():\n    a = 1\n    return a\n";
        // The body is a region of its own (its first statement line folds the rest), as every multi-line node is.
        assert_eq!(folds(Python, src), vec![(0, 1, FoldKind::Imports, false), (2, 4, FoldKind::Block, false), (3, 4, FoldKind::Block, false)]);
    }

    #[test]
    fn json_objects_and_arrays_fold() {
        let src = "{\n  \"a\": [\n    1\n  ],\n  \"b\": 2\n}\n";
        assert_eq!(folds(Json, src), vec![(0, 5, FoldKind::Block, true), (1, 3, FoldKind::Block, true)]);
    }

    #[test]
    fn folds_handle_crlf_and_no_final_newline() {
        assert_eq!(folds(Rust, "fn f() {\r\n    1\r\n}"), vec![(0, 2, FoldKind::Block, true)]);
        assert_eq!(folds(Rust, "fn f() {}\r\n"), vec![]);
        assert_eq!(folds(Rust, ""), vec![]);
    }

    #[test]
    fn multi_line_strings_and_block_comments_fold() {
        let src = "/* a\n   b */\nconst S: &str = \"x\ny\";\n";
        assert_eq!(folds(Rust, src), vec![(0, 1, FoldKind::Comment, false), (2, 3, FoldKind::Block, false)]);
    }

    #[test]
    fn obsolete_fold_parses_stop() {
        assert!(parse_folded_while(Rust, "fn a() {\n}\n".into(), || false).is_none());
    }

    fn steps(lang: Language, src: &str, at: Range<usize>) -> Vec<&str> {
        selection_steps_while(lang, src.into(), at, || true).unwrap().into_iter().map(|r| &src[r]).collect()
    }

    #[test]
    fn steps_grow_from_a_caret_in_a_name_to_the_whole_text() {
        let src = "fn f() {\n    let total = a + b;\n}\n";
        let at = src.find("total").unwrap() + 2;
        let s = steps(Rust, src, at..at);
        assert_eq!(s[0], "total");
        let stmt = s.iter().position(|t| *t == "let total = a + b;").expect("the statement is a step");
        assert!(stmt < s.iter().position(|t| t.starts_with('{')).unwrap(), "{s:?}");
        assert_eq!(*s.last().unwrap(), src);
    }

    #[test]
    fn strings_and_blocks_select_their_inside_first() {
        let src = "fn f() {\n    g(\"hello world\");\n}\n";
        let at = src.find("world").unwrap() + 1;
        let s = steps(Rust, src, at..at);
        assert_eq!(&s[..3], &["world", "hello world", "\"hello world\""]);
        let body = s.iter().position(|t| *t == "g(\"hello world\");").unwrap();
        assert_eq!(s[body + 1], "{\n    g(\"hello world\");\n}", "a block's inside equals its only statement, so the braces come next");
    }

    #[test]
    fn steps_strictly_grow_and_contain_the_start() {
        let src = "def f(x):\n    return [x, 2]\n";
        let at = src.find('2').unwrap();
        let s = selection_steps_while(Python, src.into(), at..at + 1, || true).unwrap();
        for w in s.windows(2) {
            assert!(w[1].start <= w[0].start && w[1].end >= w[0].end && w[1] != w[0], "{s:?}");
        }
        assert!(s[0].start <= at && s[0].end > at && s[0] != (at..at + 1));
        assert_eq!(s.last(), Some(&(0..src.len())));
    }

    #[test]
    fn steps_on_empty_text_and_edges() {
        assert_eq!(selection_steps_while(Rust, "".into(), 0..0, || true), Some(vec![]));
        let src = "fn f() {}\n";
        assert_eq!(selection_steps_while(Rust, src.into(), 0..src.len(), || true), Some(vec![]));
        let end = selection_steps_while(Rust, src.into(), src.len()..src.len(), || true).unwrap();
        assert_eq!(end.last(), Some(&(0..src.len())));
        assert!(selection_steps_while(Rust, src.into(), 0..0, || false).is_none());
    }

    fn relations(lang: Language, text: &str) -> Vec<(String, String, DeclKind, DeclKind)> {
        type_relations_while(lang, text.into(), || true)
            .unwrap_or_default()
            .into_iter()
            .map(|r| (text[r.sub].to_owned(), text[r.sup].to_owned(), r.sub_kind, r.sup_kind))
            .collect()
    }

    fn pairs(lang: Language, text: &str) -> Vec<(String, String)> {
        relations(lang, text).into_iter().map(|(a, b, ..)| (a, b)).collect()
    }

    fn p(sub: &str, sup: &str) -> (String, String) {
        (sub.to_owned(), sup.to_owned())
    }

    #[test]
    fn rust_relations_use_the_last_plain_name() {
        let text = "trait Show {}\ntrait Fancy: Show + a::Loud<u8> {}\nimpl Show for S {}\nimpl<T> a::Show for b::Wrap<T> {}\nimpl S {}\n// impl Show for Comment {}\nfn f() { let s = \"impl Show for Str {}\"; }\n";
        let mut got = pairs(Language::Rust, text);
        got.sort();
        assert_eq!(got, vec![p("Fancy", "Loud"), p("Fancy", "Show"), p("S", "Show"), p("Wrap", "Show")]);
        let kinds = relations(Language::Rust, text);
        assert!(kinds.iter().any(|(s, _, k, sk)| s == "S" && *k == DeclKind::Impl && *sk == DeclKind::Interface));
        assert!(kinds.iter().any(|(s, _, k, _)| s == "Fancy" && *k == DeclKind::Interface));
    }

    #[test]
    fn python_bases_skip_keyword_arguments() {
        let mut got = pairs(Language::Python, "class A(B, mod.C, metaclass=M):\n    pass\nclass D:\n    pass\n");
        got.sort();
        assert_eq!(got, vec![p("A", "B"), p("A", "C")]);
    }

    #[test]
    fn typescript_and_javascript_extends_and_implements() {
        let ts = "class A extends B implements I, ns.J<T> {}\ninterface K extends I, L<T> {}\nabstract class M extends N {}\n";
        let mut got = pairs(Language::TypeScript, ts);
        got.sort();
        assert_eq!(got, vec![p("A", "B"), p("A", "I"), p("A", "J"), p("K", "I"), p("K", "L"), p("M", "N")]);
        assert_eq!(pairs(Language::Tsx, ts).len(), 6);
        assert_eq!(pairs(Language::JavaScript, "class A extends mod.B {}\nclass C {}\n"), vec![p("A", "B")]);
    }

    #[test]
    fn java_kotlin_and_ruby_supertypes() {
        let mut java = pairs(Language::Java, "class A extends B implements I, p.J<T> {}\ninterface K extends I {}\n");
        java.sort();
        assert_eq!(java, vec![p("A", "B"), p("A", "I"), p("A", "J"), p("K", "I")]);
        let mut kt = relations(Language::Kotlin, "class A : B(), I {}\ninterface K : I\nobject O : I\n");
        kt.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        assert_eq!(kt.iter().map(|(a, b, ..)| p(a, b)).collect::<Vec<_>>(), vec![p("A", "B"), p("A", "I"), p("K", "I"), p("O", "I")]);
        assert!(kt.iter().any(|(a, b, _, sk)| a == "A" && b == "B" && *sk == DeclKind::Type));
        assert!(kt.iter().any(|(a, b, _, sk)| a == "A" && b == "I" && *sk == DeclKind::Interface));
        let mut rb = pairs(Language::Ruby, "class A < Base\n  include Mixin\n  include Mod::Other\nend\nclass B::C < D::E\nend\n");
        rb.sort();
        assert_eq!(rb, vec![p("A", "Base"), p("A", "Mixin"), p("A", "Other"), p("C", "E")]);
    }

    #[test]
    fn java_qualified_generic_supertypes() {
        let mut got = pairs(Language::Java, "class A extends p.B<T> {}\ninterface K extends q.I<T>, J {}\n");
        got.sort();
        assert_eq!(got, vec![p("A", "B"), p("K", "I"), p("K", "J")]);
    }

    #[test]
    fn java_enum_and_record_implement_interfaces() {
        let text = "enum E implements I, p.J<T> { X }\nrecord R(int x) implements K {}\n";
        let mut got = pairs(Language::Java, text);
        got.sort();
        assert_eq!(got, vec![p("E", "I"), p("E", "J"), p("R", "K")]);
        assert!(relations(Language::Java, text).iter().all(|(_, _, k, sk)| *k == DeclKind::Type && *sk == DeclKind::Interface));
    }

    #[test]
    fn rust_blanket_impl_has_no_subtype() {
        let text = "trait Show {}\nimpl<T> Show for T {}\nimpl<T: Debug> Show for &T {}\nimpl<U> Show for U where U: Debug {}\nimpl<T: Show> Show for Box<T> {}\nimpl<'a, U> Show for Wrap<'a, U> {}\n";
        let mut got = pairs(Language::Rust, text);
        got.sort();
        assert_eq!(got, vec![p("Box", "Show"), p("Wrap", "Show")]);
    }

    #[test]
    fn rust_negative_impl_has_no_subtype() {
        assert_eq!(pairs(Language::Rust, "impl !Send for S {}\nimpl Show for S {}\n"), vec![p("S", "Show")]);
    }

    #[test]
    fn rust_impl_for_a_reference_names_the_type() {
        let mut got = pairs(Language::Rust, "impl Show for &S {}\nimpl<'a> Show for &'a mut b::T<u8> {}\n");
        got.sort();
        assert_eq!(got, vec![p("S", "Show"), p("T", "Show")]);
    }

    #[test]
    fn rust_impl_for_dyn_has_no_subtype_and_keeps_the_rest() {
        let text = "impl Show for dyn Other {}\nimpl Show for &dyn Other {}\nimpl Show for Plain {}\n";
        assert_eq!(pairs(Language::Rust, text), vec![p("Plain", "Show")]);
    }

    #[test]
    fn ruby_include_in_a_module() {
        let text = "module Greeter\n  include Mixin\n  include Mod::Other\nend\nclass A\n  include Kind\nend\n";
        let mut got = relations(Language::Ruby, text);
        got.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        let got: Vec<(String, String, DeclKind)> = got.into_iter().map(|(a, b, k, _)| (a, b, k)).collect();
        assert_eq!(got, vec![("A".into(), "Kind".into(), DeclKind::Type), ("Greeter".into(), "Mixin".into(), DeclKind::Module), ("Greeter".into(), "Other".into(), DeclKind::Module)]);
    }

    #[test]
    fn relations_span_lines_and_other_languages_have_none() {
        let text = "class A(\n    B,\n):\n    pass\n";
        let r = type_relations_while(Language::Python, text.into(), || true).unwrap();
        assert_eq!((&text[r[0].sub.clone()], &text[r[0].sup.clone()], r[0].site.start), ("A", "B", 0));
        assert!(!has_type_relations(Language::Go) && type_relations_while(Language::Go, "type A struct{}".into(), || true).is_none());
        for lang in [Language::Rust, Language::Python, Language::TypeScript, Language::Tsx, Language::JavaScript, Language::Java, Language::Kotlin, Language::Ruby] {
            assert!(has_type_relations(lang), "{lang:?}: its query compiles");
        }
        assert!(type_relations_while(Language::Rust, "impl A for B {}".into(), || false).is_none(), "obsolete: nothing");
    }

    #[test]
    #[ignore = "timing; run in release"]
    fn type_relations_perf() {
        let samples = [
            (Language::Rust, "trait T{} impl T for S{} struct S;\n"),
            (Language::Python, "class A(B):\n    pass\n"),
            (Language::TypeScript, "class A extends B implements I {}\n"),
            (Language::Java, "class A extends B implements I {}\n"),
            (Language::Kotlin, "class A : B(), I {}\n"),
            (Language::Ruby, "class A < B\nend\n"),
        ];
        for (lang, line) in samples {
            let text: Arc<str> = line.repeat(10_000 / line.matches('\n').count()).into();
            let t = std::time::Instant::now();
            let n = type_relations_while(lang, text, || true).map_or(0, |r| r.len());
            println!("{lang:?}: {n} relations in {:?}", t.elapsed());
        }
    }

    #[test]
    fn generic_bases_count_in_kotlin_and_python() {
        let mut kt = pairs(Language::Kotlin, "class A : Comparable<A>\nclass B : ArrayList<String>()\nclass C : p.Base<Int>(), q.I<C>\n");
        kt.sort();
        assert_eq!(kt, vec![p("A", "Comparable"), p("B", "ArrayList"), p("C", "Base"), p("C", "I")]);
        let mut py = pairs(Language::Python, "class A(Generic[T]):\n    pass\nclass B(mod.Base[int], Protocol):\n    pass\n");
        py.sort();
        assert_eq!(py, vec![p("A", "Generic"), p("B", "Base"), p("B", "Protocol")]);
    }
}
