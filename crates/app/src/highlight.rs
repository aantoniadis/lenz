//! Syntax highlighting: tree-sitter parses a Side in the background, then its compact highlight
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

/// One Side's highlights. No syntax tree or duplicate source text survives the background job.
pub struct Parsed {
    offsets: Vec<usize>,
    spans: Vec<Span>,
}

struct Span {
    start: u32,
    end: u32,
    capture: Capture,
}

/// Only lives while a Side's highlights are being built.
struct SyntaxTree<'a> {
    lang: Language,
    text: &'a str,
    /// Byte offset of each line's start; lines split on `\n` exactly like `lenz_engine::split_lines`.
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
    type Request = (Language, Arc<str>, Box<dyn Fn() -> bool + Send>, mpsc::SyncSender<Option<Parsed>>);
    // A small pool, so a merge's three panes or a comparison's two sides parse at once. Each worker hands its
    // freed tree back to the system after every parse, so an idle worker keeps no peak resident.
    static WORKERS: OnceLock<mpsc::Sender<Request>> = OnceLock::new();
    let workers = WORKERS.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Request>();
        let rx = Arc::new(std::sync::Mutex::new(rx));
        let count = std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, SYNTAX_WORKERS);
        for i in 0..count {
            let rx = rx.clone();
            std::thread::Builder::new().name(format!("lenz-highlight-{i}")).spawn(move || loop {
                let request = rx.lock().unwrap_or_else(std::sync::PoisonError::into_inner).recv();
                let Ok((lang, text, current, reply)) = request else { break };
                // A panic while parsing (in an unwinding build) answers "no highlights", and the worker lives on.
                let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| parse_now(lang, &text, &*current)));
                let _ = reply.send(parsed.unwrap_or(None));
                release_freed_memory();
            }).expect("start syntax worker");
        }
        tx
    });
    let (tx, rx) = mpsc::sync_channel(1);
    if workers.send((lang, text, Box::new(current), tx)).is_err() {
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

fn parse_now(lang: Language, text: &str, current: &dyn Fn() -> bool) -> Option<Parsed> {
    if !current() { return None; }
    let language = &config(lang).language; // compiled outside the arena: queries outlive every parse
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
    let line_starts = std::iter::once(0).chain(text.match_indices('\n').map(|(i, _)| i + 1)).collect();
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
    drop(syntax);
    parsed.spans.shrink_to_fit();
    Some(parsed)
}

impl Parsed {
    pub fn bytes(&self) -> usize {
        self.offsets.capacity() * size_of::<usize>() + self.spans.capacity() * size_of::<Span>()
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

    /// Line `l`'s byte range, without its `\n` and a trailing `\r`.
    fn line(&self, l: usize) -> Range<usize> {
        let start = self.line_starts[l];
        let end = self.line_starts.get(l + 1).map_or(self.text.len(), |next| next - 1);
        let end = if self.text.as_bytes()[start..end].ends_with(b"\r") { end - 1 } else { end };
        start..end
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
    fn parsed_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Parsed>();
    }

    /// The text of each span on `line`, with its capture.
    fn tokens(p: &Parsed, text: &str, line: usize) -> Vec<(String, Capture)> {
        let lines = lenz_engine::split_lines(text);
        p.spans(line..line + 1)
            .remove(0)
            .into_iter()
            .map(|(r, c)| (lines[line][r].to_string(), c))
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
        let n = lenz_engine::split_lines(&src).len();
        let mut parser = Parser::new();
        parser.set_language(&config(Rust).language).unwrap();
        let tree = SyntaxTree {
            lang: Rust, text: &src, tree: parser.parse(&src, None).unwrap(),
            line_starts: std::iter::once(0).chain(src.match_indices('\n').map(|(i, _)| i + 1)).collect(),
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

    /// `cargo test --release -p lenz -- --ignored --nocapture perf`
    #[test]
    #[ignore]
    fn perf() {
        use std::time::Instant;
        let src = match std::env::var("LENZ_PERF_FILE") {
            Ok(path) => std::fs::read_to_string(path).unwrap(),
            Err(_) => big_rust(100_000 / 9 + 1),
        };
        let n = lenz_engine::split_lines(&src).len();
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
        eprintln!(
            "{n} lines, {} KB: query compile {compile:?}, parse {parsed:?}, spans(60 lines) {per_window:?} ({count} spans over 100 windows)",
            src.len() / 1024
        );
        eprintln!("retained highlights: {:.1} MiB", p.bytes() as f64 / (1 << 20) as f64);
        assert!(p.bytes() < src.len() * 5, "highlight cache must stay compact");
        crate::perf::check(per_window.as_micros() < 250, "visible syntax lookup exceeds 250 us");
    }
}
