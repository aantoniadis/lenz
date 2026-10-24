//! A scripted language server for the reader's protocol tests (an example: built by `cargo test`, never shipped). It logs every message it reads and
//! answers definitions by name: `fn NAME`, `struct NAME` or `let NAME` in open documents and `.rs` files under
//! the root. Names steer it: `slow_*` answers late, `crash_*` exits, `virt*` points at a virtual source,
//! `error_*` fails. The environment sets its capabilities (`FAKE_LSP_*`).
//! Decorations: `let NAME` gets a type hint `: i32` after NAME, `add(` a parameter hint `a:` (not after `fn `); tokens
//! for `let [mut] NAME` (variable, `mut` → mutable) and `fn NAME` (function, `old_*` → deprecated).
//! `FAKE_LSP_NO_DECOR` offers neither, `FAKE_LSP_SLOW_DECOR` answers hints 300 ms late, `FAKE_LSP_REFRESH` asks for a
//! hint refresh after its first hint reply, `FAKE_LSP_BUSY_DECOR` (with `FAKE_LSP_INDEXING`) refuses the first hint
//! request with `ContentModified` (`FAKE_LSP_EMPTY_DECOR`: answers it empty, as rust-analyzer does while loading), then
//! finishes indexing. `FAKE_LSP_STALE_DECOR` holds its first hint answer (labels `: old`), asks for a refresh, and
//! sends it after the first answer made after that refresh.
//! `FAKE_LSP_TYPE_HIERARCHY` offers type hierarchy: `struct`/`trait`/`class` names; supertypes from `impl T for X` and
//! `trait X: T`, subtypes the other way.

use std::{
    collections::HashMap,
    io::{BufReader, Write},
    path::PathBuf,
};

use reader_core::semantics::lsp::{self, read_message, write_message};
use serde_json::{Value, json};

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

struct Fake {
    utf8: bool,
    docs: HashMap<String, String>,
    root: Option<PathBuf>,
    log: Option<std::fs::File>,
}

impl Fake {
    fn log(&mut self, v: &Value) {
        if let Some(f) = &mut self.log {
            let _ = writeln!(f, "{v}");
            let _ = f.flush();
        }
    }

    /// Where type `name` is declared (`struct`, `trait`, `class`, `interface`): its item, kind 23/11/5/11.
    fn type_item(&self, name: &str) -> Option<Value> {
        for (uri, text) in self.files() {
            for (l, line) in text.split('\n').enumerate() {
                for (kw, kind) in [("struct ", 23), ("trait ", 11), ("class ", 5), ("interface ", 11)] {
                    let Some(i) = line.find(&format!("{kw}{name}")) else { continue };
                    let s = i + kw.len();
                    if line[s + name.len()..].chars().next().is_some_and(|c| c.is_alphanumeric() || c == '_') {
                        continue;
                    }
                    let range = json!({ "start": { "line": l, "character": self.units(&line[..s]) }, "end": { "line": l, "character": self.units(&line[..s + name.len()]) } });
                    return Some(json!({ "name": name, "kind": kind, "uri": uri, "range": range, "selectionRange": range }));
                }
            }
        }
        None
    }

    /// `(sub, sup)` name pairs from `impl SUP for SUB` and `trait SUB: SUP + …` lines.
    fn type_pairs(&self) -> Vec<(String, String)> {
        let word = |s: &str| s.trim().split(|c: char| !(c.is_alphanumeric() || c == '_')).next().unwrap_or_default().to_owned();
        let mut out = Vec::new();
        for (_, text) in self.files() {
            for line in text.split('\n') {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("impl ")
                    && let Some((sup, sub)) = rest.split_once(" for ")
                {
                    out.push((word(sub), word(sup)));
                } else if let Some(rest) = line.strip_prefix("pub trait ").or(line.strip_prefix("trait "))
                    && let Some((sub, sups)) = rest.split_once(':')
                {
                    out.extend(sups.split('+').map(|s| (word(sub), word(s.trim_end_matches(|c: char| c == '{' || c == '}' || c.is_whitespace())))));
                }
            }
        }
        out
    }

    /// The name a `let` (or `let mut`) on `line` binds: its byte range.
    fn let_name(&self, line: &str) -> Option<(usize, usize)> {
        let i = line.find("let ")? + 4;
        let start = if line[i..].starts_with("mut ") { i + 4 } else { i };
        let end = start + line[start..].find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(line.len() - start);
        (start < end).then_some((start, end))
    }

    fn units(&self, s: &str) -> usize {
        if self.utf8 { s.len() } else { s.encode_utf16().count() }
    }

    /// The byte offset of `character` units into `line`.
    fn byte(&self, line: &str, character: usize) -> usize {
        let mut units = 0;
        for (i, c) in line.char_indices() {
            if units >= character {
                return i;
            }
            units += if self.utf8 { c.len_utf8() } else { c.len_utf16() };
        }
        line.len()
    }

    fn text(&self, uri: &str) -> Option<String> {
        self.docs.get(uri).cloned().or_else(|| std::fs::read_to_string(lsp::uri_path(uri)?).ok())
    }

    fn name_at(&self, uri: &str, pos: &Value) -> Option<String> {
        let text = self.text(uri)?;
        let line = text.split('\n').nth(pos["line"].as_u64()? as usize)?.trim_end_matches('\r');
        let at = self.byte(line, pos["character"].as_u64()? as usize);
        let word = |c: char| c.is_alphanumeric() || c == '_';
        let start = line[..at].rfind(|c: char| !word(c)).map_or(0, |i| i + line[i..].chars().next().map_or(1, char::len_utf8));
        let end = line[at..].find(|c: char| !word(c)).map_or(line.len(), |i| at + i);
        (start < end).then(|| line[start..end].to_owned())
    }

    fn files(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self.docs.iter().map(|(u, t)| (u.clone(), t.clone())).collect();
        // The root, and `FAKE_LSP_EXTRA` (standing in for dependencies outside it).
        let mut dirs: Vec<PathBuf> = self.root.iter().cloned().chain(env("FAKE_LSP_EXTRA").map(PathBuf::from)).collect();
        while let Some(d) = dirs.pop() {
            let Ok(entries) = std::fs::read_dir(&d) else { continue };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    dirs.push(p);
                } else if p.extension().is_some_and(|x| ["rs", "py", "ts", "tsx", "js", "jsx"].iter().any(|e| x == *e)) {
                    let uri = lsp::file_uri(&p);
                    if !self.docs.contains_key(&uri)
                        && let Ok(t) = std::fs::read_to_string(&p)
                    {
                        out.push((uri, t));
                    }
                }
            }
        }
        out.sort();
        out
    }

    /// Every whole-word use of `name` that is not its declaration.
    fn references(&self, name: &str) -> Vec<Value> {
        let mut found = Vec::new();
        for (uri, text) in self.files() {
            for (l, line) in text.split('\n').enumerate() {
                let line = line.trim_end_matches('\r');
                let word = |c: char| c.is_alphanumeric() || c == '_';
                for (i, _) in line.match_indices(name) {
                    let before = line[..i].chars().next_back();
                    let after = line[i + name.len()..].chars().next();
                    let declared = ["fn ", "struct ", "let "].iter().any(|k| line[..i].ends_with(k));
                    if before.is_some_and(word) || after.is_some_and(word) || declared {
                        continue;
                    }
                    let (a, b) = (self.units(&line[..i]), self.units(&line[..i + name.len()]));
                    found.push(json!({ "uri": uri, "range": { "start": { "line": l, "character": a }, "end": { "line": l, "character": b } } }));
                }
            }
        }
        found
    }

    /// The function holding `line` of `text`: the nearest line at or above it starting `fn NAME`.
    fn enclosing(text: &str, line: usize) -> Option<(usize, String)> {
        let lines: Vec<&str> = text.split('\n').take(line + 1).collect();
        lines.iter().enumerate().rev().find_map(|(l, s)| s.strip_prefix("fn ").map(|r| (l, r.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect())))
    }

    /// A call-hierarchy item for function `name` declared (`fn name`) on `line` of `uri`.
    fn item(&self, uri: &str, text: &str, line: usize, name: &str) -> Value {
        let l = text.split('\n').nth(line).unwrap_or_default();
        let s = l.find(&format!("fn {name}")).map_or(0, |i| i + 3);
        let (a, b) = (self.units(&l[..s]), self.units(&l[..s + name.len()]));
        let range = json!({ "start": { "line": line, "character": a }, "end": { "line": line, "character": b } });
        json!({ "name": name, "kind": 12, "uri": uri, "range": range, "selectionRange": range })
    }

    /// Whole-word calls `name(` in `line` (not its declaration): their ranges.
    fn calls_in(&self, line: &str, l: usize, name: &str) -> Vec<Value> {
        let word = |c: char| c.is_alphanumeric() || c == '_';
        line.match_indices(&format!("{name}("))
            .filter(|(i, _)| !line[..*i].chars().next_back().is_some_and(word) && !line[..*i].ends_with("fn "))
            .map(|(i, _)| json!({ "start": { "line": l, "character": self.units(&line[..i]) }, "end": { "line": l, "character": self.units(&line[..i + name.len()]) } }))
            .collect()
    }

    fn definitions(&self, name: &str) -> Vec<Value> {
        let mut found = Vec::new();
        for (uri, text) in self.files() {
            for (l, line) in text.split('\n').enumerate() {
                let line = line.trim_end_matches('\r');
                for kw in ["fn ", "struct ", "let ", "def ", "class ", "function "] {
                    let needle = format!("{kw}{name}");
                    if let Some(i) = line.find(&needle) {
                        let s = i + kw.len();
                        let after = line[s + name.len()..].chars().next();
                        if after.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                            continue;
                        }
                        let (a, b) = (self.units(&line[..s]), self.units(&line[..s + name.len()]));
                        let target_uri = if env("FAKE_LSP_CANONICAL_URIS").is_some() {
                            lsp::uri_path(&uri).and_then(|path| std::fs::canonicalize(path).ok()).map(|path| lsp::file_uri(&path)).unwrap_or_else(|| uri.clone())
                        } else { uri.clone() };
                        found.push(json!({ "uri": target_uri, "range": { "start": { "line": l, "character": a }, "end": { "line": l, "character": b } } }));
                    }
                }
            }
        }
        found
    }
}

fn main() {
    if std::env::args().any(|a| a == "--version") {
        if env("FAKE_LSP_VERSION_FAIL").is_some() {
            eprintln!("fake-lsp: no toolchain");
            if env("FAKE_LSP_VERSION_FAIL").as_deref() == Some("mise") {
                eprintln!("mise ERROR No version is set for shim: rust-analyzer");
                eprintln!("Set a default version: mise use -g rust@stable");
            }
            std::process::exit(1);
        }
        println!("fake-lsp 1.0.0");
        return;
    }
    if let Some(mode) = env("FAKE_LSP_START_FAIL") {
        eprintln!("mise ERROR No version is set for shim: pyright-langserver");
        eprintln!("Set a global default version with one of following:");
        eprintln!("mise use -g node@26.8.1");
        if mode == "noisy" {
            let mut stderr = std::io::stderr().lock();
            let _ = stderr.write_all(b"\x1b[31m\x07");
            let _ = stderr.write_all("\u{202e}".as_bytes());
            for _ in 0..10_000 {
                let _ = stderr.write_all("é𝄞".as_bytes());
            }
            let _ = stderr.write_all(b"END OF UNTERMINATED NOISE");
        } else {
            eprintln!("mise ERROR Version: 2026.10.1 linux-x64");
            eprintln!("mise ERROR Run with --verbose or MISE_VERBOSE=1 for more information");
        }
        std::process::exit(3);
    }
    let log = env("FAKE_LSP_LOG").and_then(|p| std::fs::OpenOptions::new().create(true).append(true).open(p).ok());
    let encoding = env("FAKE_LSP_ENCODING");
    let mut fake = Fake { utf8: encoding.as_deref() == Some("utf-8"), docs: HashMap::new(), root: None, log };
    fake.log(&json!({ "pid": std::process::id() }));
    if env("FAKE_LSP_CHILD").is_some()
        && let Ok(child) = std::process::Command::new("sleep").arg("60").stdout(std::process::Stdio::null()).spawn()
    {
        fake.log(&json!({ "child": child.id() }));
    }
    // Memory a large server would hold, touched so it is resident.
    let _ballast: Vec<u8> = env("FAKE_LSP_ALLOC_MB").and_then(|m| m.parse::<usize>().ok()).map_or_else(Vec::new, |mb| vec![1u8; mb << 20]);
    let mut input = BufReader::new(std::io::stdin());
    let mut out = std::io::stdout();
    let (mut refreshed, mut refused) = (false, false);
    let mut deferred: Option<(Value, Vec<Value>)> = None;
    let mut refresh_answered = false;
    while let Some(m) = read_message(&mut input) {
        fake.log(&m);
        let id = m.get("id").cloned();
        let method = m["method"].as_str().unwrap_or_default().to_owned();
        refresh_answered |= m["id"] == "rf-1" && m.get("result").is_some();
        let reply = |out: &mut std::io::Stdout, result: Value| {
            let _ = write_message(out, &json!({ "jsonrpc": "2.0", "id": id, "result": result }));
        };
        match method.as_str() {
            "initialize" => {
                if let Some(ms) = env("FAKE_LSP_INIT_DELAY_MS").and_then(|v| v.parse().ok()) {
                    std::thread::sleep(std::time::Duration::from_millis(ms));
                }
                fake.root = m["params"]["rootUri"].as_str().and_then(lsp::uri_path);
                let mut caps = json!({ "textDocumentSync": 1 });
                if env("FAKE_LSP_NO_DEFINITION").is_none() {
                    caps["definitionProvider"] = json!(true);
                }
                if env("FAKE_LSP_NO_NAVIGATION").is_none() {
                    caps["implementationProvider"] = json!(true);
                    caps["typeDefinitionProvider"] = json!(true);
                    caps["callHierarchyProvider"] = json!(true);
                    caps["documentHighlightProvider"] = json!(true);
                    caps["hoverProvider"] = json!(true);
                }
                if env("FAKE_LSP_NO_REFERENCES").is_none() {
                    caps["referencesProvider"] = json!(true);
                    caps["documentSymbolProvider"] = json!(true);
                    if env("FAKE_LSP_NO_WORKSPACE_SYMBOLS").is_none() {
                        caps["workspaceSymbolProvider"] = json!(true);
                    }
                }
                fake.log(&json!({ "args": std::env::args().skip(1).collect::<Vec<_>>(), "env": env("FAKE_LSP_MARK") }));
                if let Some(e) = &encoding
                    && e != "none"
                {
                    caps["positionEncoding"] = json!(e);
                }
                if env("FAKE_LSP_TYPE_HIERARCHY").is_some() {
                    caps["typeHierarchyProvider"] = json!(true);
                }
                if env("FAKE_LSP_NO_DECOR").is_none() {
                    caps["inlayHintProvider"] = json!(true);
                    caps["semanticTokensProvider"] = json!({ "legend": { "tokenTypes": ["variable", "function", "keyword"], "tokenModifiers": ["mutable", "deprecated"] }, "full": true });
                }
                reply(&mut out, json!({ "capabilities": caps, "serverInfo": { "name": "fake" } }));
            }
            "initialized" => {
                if let Some(release) = env("FAKE_LSP_PAUSE_INPUT") {
                    let release = PathBuf::from(release);
                    let _ = std::fs::write(release.with_extension("paused"), b"");
                    let until = std::time::Instant::now() + std::time::Duration::from_secs(15);
                    while !release.exists() && std::time::Instant::now() < until { std::thread::sleep(std::time::Duration::from_millis(1)); }
                }
                // As rust-analyzer does when asked for `serverStatusNotification`.
                let quiescent = env("FAKE_LSP_INDEXING").is_none();
                let _ = write_message(&mut out, &json!({ "jsonrpc": "2.0", "method": "experimental/serverStatus", "params": { "health": "ok", "quiescent": quiescent } }));
                if env("FAKE_LSP_INDEXING").is_some() {
                    let _ = write_message(&mut out, &json!({ "jsonrpc": "2.0", "id": "p1", "method": "window/workDoneProgress/create", "params": { "token": "idx" } }));
                    let _ = write_message(&mut out, &json!({ "jsonrpc": "2.0", "method": "$/progress", "params": { "token": "idx", "value": { "kind": "begin", "title": "Indexing", "message": "0/3" } } }));
                }
                // A burst of progress, as a real server loading a large workspace sends.
                if let Some(n) = env("FAKE_LSP_PROGRESS_FLOOD").and_then(|v| v.parse::<usize>().ok()) {
                    let progress = |value: Value| json!({ "jsonrpc": "2.0", "method": "$/progress", "params": { "token": "load", "value": value } });
                    let _ = write_message(&mut out, &progress(json!({ "kind": "begin", "title": "Loading" })));
                    for i in 0..n {
                        let _ = write_message(&mut out, &progress(json!({ "kind": "report", "message": format!("{i}/{n}") })));
                    }
                    let _ = write_message(&mut out, &progress(json!({ "kind": "end" })));
                }
                // As JDT LS reports its readiness.
                if env("FAKE_LSP_JDT").is_some() {
                    let status = |t: &str| json!({ "jsonrpc": "2.0", "method": "language/status", "params": { "type": t, "message": t } });
                    let _ = write_message(&mut out, &status("Starting"));
                    let _ = write_message(&mut out, &status("ServiceReady"));
                }
                if env("FAKE_LSP_ASK_CONFIG").is_some() {
                    let _ = write_message(&mut out, &json!({ "jsonrpc": "2.0", "id": "cfg-1", "method": "workspace/configuration", "params": { "items": [{ "section": "rust-analyzer" }, { "section": "rust-analyzer.cargo" }] } }));
                }
                if env("FAKE_LSP_EDIT").is_some() {
                    let edit = json!({ "changes": { lsp::file_uri(&fake.root.clone().unwrap_or_default().join("src/lib.rs")): [{ "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } }, "newText": "// edited\n" }] } });
                    let _ = write_message(&mut out, &json!({ "jsonrpc": "2.0", "id": "edit-1", "method": "workspace/applyEdit", "params": { "edit": edit } }));
                }
            }
            "textDocument/didOpen" => {
                let d = &m["params"]["textDocument"];
                fake.docs.insert(d["uri"].as_str().unwrap_or_default().into(), d["text"].as_str().unwrap_or_default().into());
                if env("FAKE_LSP_CLOSE_INPUT_AFTER_OPEN").is_some() {
                    // The provider exits but a helper keeps stdout open: input failure must stop the
                    // owned group even though the output reader cannot observe EOF yet.
                    let child = std::process::Command::new("sleep").arg("60").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::inherit()).spawn().unwrap();
                    fake.log(&json!({ "input_child": child.id() }));
                    std::process::exit(0);
                }
            }
            "textDocument/didChange" => {
                let uri = m["params"]["textDocument"]["uri"].as_str().unwrap_or_default().to_owned();
                if let Some(t) = m["params"]["contentChanges"][0]["text"].as_str() {
                    fake.docs.insert(uri, t.into());
                }
            }
            "textDocument/didClose" => {
                fake.docs.remove(m["params"]["textDocument"]["uri"].as_str().unwrap_or_default());
            }
            "textDocument/definition" => {
                let p = &m["params"];
                let name = fake.name_at(p["textDocument"]["uri"].as_str().unwrap_or_default(), &p["position"]).unwrap_or_default();
                if name.starts_with("slow_") {
                    std::thread::sleep(std::time::Duration::from_millis(400));
                }
                if name.starts_with("crash_") {
                    eprintln!("fake-lsp: crashed on purpose");
                    std::process::exit(3);
                }
                if name.starts_with("error_") {
                    let _ = write_message(&mut out, &json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32603, "message": "fake failure" } }));
                    continue;
                }
                if name.starts_with("virt") {
                    reply(&mut out, json!([{ "uri": "fake-virtual:///pkg/Generated.java?=jar%2Fx", "range": { "start": { "line": 2, "character": 9 }, "end": { "line": 2, "character": 13 } } }]));
                    continue;
                }
                let mut found = fake.definitions(&name);
                match found.len() {
                    0 => reply(&mut out, Value::Null),
                    1 => reply(&mut out, found.remove(0)),
                    _ => reply(&mut out, Value::Array(found)),
                }
            }
            // `impl NAME for TYPE` lines are NAME's implementations.
            "textDocument/implementation" => {
                let p = &m["params"];
                let name = fake.name_at(p["textDocument"]["uri"].as_str().unwrap_or_default(), &p["position"]).unwrap_or_default();
                let needle = format!("impl {name} for ");
                let mut found = Vec::new();
                for (uri, text) in fake.files() {
                    for (l, line) in text.split('\n').enumerate() {
                        if let Some(i) = line.find(&needle) {
                            let s = i + needle.len();
                            let ty: String = line[s..].chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                            let (a, b) = (fake.units(&line[..s]), fake.units(&line[..s + ty.len()]));
                            found.push(json!({ "uri": uri, "range": { "start": { "line": l, "character": a }, "end": { "line": l, "character": b } } }));
                        }
                    }
                }
                reply(&mut out, Value::Array(found));
            }
            // `let NAME: TYPE`: NAME's type definition is TYPE's.
            "textDocument/typeDefinition" => {
                let p = &m["params"];
                let uri = p["textDocument"]["uri"].as_str().unwrap_or_default();
                let name = fake.name_at(uri, &p["position"]).unwrap_or_default();
                let text = fake.text(uri).unwrap_or_default();
                let ty = text.split('\n').find_map(|l| l.split_once(&format!("let {name}: ")).map(|(_, r)| r.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect::<String>()));
                let found = ty.map(|t| fake.definitions(&t)).unwrap_or_default();
                reply(&mut out, if found.is_empty() { Value::Null } else { Value::Array(found) });
            }
            // Every whole-word occurrence in this document; an assignment (`NAME =`) is a write.
            "textDocument/documentHighlight" => {
                let p = &m["params"];
                let uri = p["textDocument"]["uri"].as_str().unwrap_or_default();
                let name = fake.name_at(uri, &p["position"]).unwrap_or_default();
                let word = |c: char| c.is_alphanumeric() || c == '_';
                let mut found = Vec::new();
                for (l, line) in fake.text(uri).unwrap_or_default().split('\n').enumerate() {
                    for (i, _) in line.match_indices(&name).filter(|_| !name.is_empty()) {
                        if line[..i].chars().next_back().is_some_and(word) || line[i + name.len()..].chars().next().is_some_and(word) {
                            continue;
                        }
                        let write = line[i + name.len()..].trim_start().starts_with('=') && !line[i + name.len()..].trim_start().starts_with("==");
                        let range = json!({ "start": { "line": l, "character": fake.units(&line[..i]) }, "end": { "line": l, "character": fake.units(&line[..i + name.len()]) } });
                        found.push(json!({ "range": range, "kind": if write { 3 } else { 2 } }));
                    }
                }
                reply(&mut out, Value::Array(found));
            }
            // A name with a definition is documented; anything else has nothing to say.
            "textDocument/hover" => {
                let p = &m["params"];
                let name = fake.name_at(p["textDocument"]["uri"].as_str().unwrap_or_default(), &p["position"]).unwrap_or_default();
                let known = !name.is_empty() && !fake.definitions(&name).is_empty();
                reply(&mut out, if known { json!({ "contents": { "kind": "markdown", "value": format!("```rust\nfn {name}()\n```\n\nDocs for **{name}**.") } }) } else { Value::Null });
            }
            "textDocument/prepareCallHierarchy" => {
                let p = &m["params"];
                let name = fake.name_at(p["textDocument"]["uri"].as_str().unwrap_or_default(), &p["position"]).unwrap_or_default();
                let found = fake.definitions(&name).into_iter().find_map(|d| {
                    let uri = d["uri"].as_str()?.to_owned();
                    let text = fake.text(&uri)?;
                    let line = d["range"]["start"]["line"].as_u64()? as usize;
                    text.split('\n').nth(line)?.contains(&format!("fn {name}")).then(|| fake.item(&uri, &text, line, &name))
                });
                reply(&mut out, found.map_or(Value::Null, |i| json!([i])));
            }
            // Callers: every function whose body calls the item's function, at those calls.
            "callHierarchy/incomingCalls" => {
                let name = m["params"]["item"]["name"].as_str().unwrap_or_default().to_owned();
                if name.starts_with("slow_") {
                    std::thread::sleep(std::time::Duration::from_millis(400));
                }
                if name.starts_with("error_") {
                    let _ = write_message(&mut out, &json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32603, "message": "fake failure" } }));
                    continue;
                }
                if name.starts_with("crash_") {
                    eprintln!("fake-lsp: crashed on purpose");
                    std::process::exit(3);
                }
                let mut found: Vec<(String, usize, String, Vec<Value>)> = Vec::new();
                for (uri, text) in fake.files() {
                    for (l, line) in text.split('\n').enumerate() {
                        let ranges = fake.calls_in(line.trim_end_matches('\r'), l, &name);
                        let Some((at, caller)) = (!ranges.is_empty()).then(|| Fake::enclosing(&text, l)).flatten() else { continue };
                        match found.iter_mut().find(|f| f.0 == uri && f.1 == at) {
                            Some(f) => f.3.extend(ranges),
                            None => found.push((uri.clone(), at, caller, ranges)),
                        }
                    }
                }
                let calls: Vec<Value> = found.iter().map(|(uri, at, caller, ranges)| json!({ "from": fake.item(uri, &fake.text(uri).unwrap_or_default(), *at, caller), "fromRanges": ranges })).collect();
                reply(&mut out, Value::Array(calls));
            }
            // Callees: every function with a definition that the item's body calls.
            "callHierarchy/outgoingCalls" => {
                let item = &m["params"]["item"];
                let uri = item["uri"].as_str().unwrap_or_default().to_owned();
                let text = fake.text(&uri).unwrap_or_default();
                let first = item["range"]["start"]["line"].as_u64().unwrap_or(0) as usize;
                let mut found: Vec<(String, Value, Vec<Value>)> = Vec::new();
                for (l, line) in text.split('\n').enumerate().skip(first + 1).take_while(|(_, l)| !l.starts_with("fn ")) {
                    let line = line.trim_end_matches('\r');
                    let names: Vec<String> = line.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).map(str::to_owned).collect();
                    for callee in names {
                        let ranges = fake.calls_in(line, l, &callee);
                        if ranges.is_empty() || found.iter().any(|f| f.0 == callee) {
                            if let Some(f) = found.iter_mut().find(|f| f.0 == callee) {
                                for r in ranges {
                                    if !f.2.contains(&r) {
                                        f.2.push(r);
                                    }
                                }
                            }
                            continue;
                        }
                        let target = fake.definitions(&callee).into_iter().find_map(|d| {
                            let u = d["uri"].as_str()?.to_owned();
                            let t = fake.text(&u)?;
                            let at = d["range"]["start"]["line"].as_u64()? as usize;
                            t.split('\n').nth(at)?.contains(&format!("fn {callee}")).then(|| fake.item(&u, &t, at, &callee))
                        });
                        if let Some(t) = target {
                            found.push((callee, t, ranges));
                        }
                    }
                }
                reply(&mut out, Value::Array(found.into_iter().map(|(_, to, ranges)| json!({ "to": to, "fromRanges": ranges })).collect()));
            }
            "textDocument/references" => {
                let p = &m["params"];
                let name = fake.name_at(p["textDocument"]["uri"].as_str().unwrap_or_default(), &p["position"]).unwrap_or_default();
                let refs = fake.references(&name);
                reply(&mut out, Value::Array(refs));
            }
            // Nested symbols: an indented `fn` belongs to the `struct`/`impl` line above it.
            "textDocument/documentSymbol" => {
                let text = fake.text(m["params"]["textDocument"]["uri"].as_str().unwrap_or_default()).unwrap_or_default();
                let mut top: Vec<Value> = Vec::new();
                for (l, line) in text.split('\n').enumerate() {
                    let line = line.trim_end_matches('\r');
                    for (kw, kind) in [("fn ", 12), ("struct ", 23), ("impl ", 5)] {
                        let Some(i) = line.find(kw) else { continue };
                        let s = i + kw.len();
                        let name: String = line[s..].chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                        let (a, b) = (fake.units(&line[..s]), fake.units(&line[..s + name.len()]));
                        let range = json!({ "start": { "line": l, "character": a }, "end": { "line": l, "character": b } });
                        let symbol = json!({ "name": name, "kind": if i > 0 && kind == 12 { 6 } else { kind }, "range": range, "selectionRange": range, "children": [] });
                        match top.last_mut() {
                            Some(parent) if i > 0 => parent["children"].as_array_mut().unwrap().push(symbol),
                            _ => top.push(symbol),
                        }
                        break;
                    }
                }
                reply(&mut out, Value::Array(top));
            }
            // JDT LS's documented retrieval of a class file's source.
            "java/classFileContents" => {
                let uri = m["params"]["uri"].as_str().unwrap_or_default();
                reply(&mut out, json!(format!("// retrieved from {uri}\npublic class Generated {{\n    void virt() {{}}\n}}\n")));
            }
            "workspace/symbol" if env("FAKE_LSP_SILENT_SYMBOLS").is_some() => {}
            "workspace/symbol" => {
                let q = m["params"]["query"].as_str().unwrap_or_default().to_lowercase();
                let mut symbols = Vec::new();
                for (uri, text) in fake.files() {
                    for (l, line) in text.split('\n').enumerate() {
                        for kw in ["fn ", "struct "] {
                            if let Some(i) = line.find(kw) {
                                let s = i + kw.len();
                                let name: String = line[s..].chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                                if !name.is_empty() && name.to_lowercase().contains(&q) {
                                    let (a, b) = (fake.units(&line[..s]), fake.units(&line[..s + name.len()]));
                                    symbols.push(json!({ "name": name, "kind": 12, "location": { "uri": uri, "range": { "start": { "line": l, "character": a }, "end": { "line": l, "character": b } } } }));
                                }
                            }
                        }
                    }
                }
                reply(&mut out, Value::Array(symbols));
            }
            "textDocument/prepareTypeHierarchy" => {
                let p = &m["params"];
                let name = fake.name_at(p["textDocument"]["uri"].as_str().unwrap_or_default(), &p["position"]).unwrap_or_default();
                reply(&mut out, fake.type_item(&name).map_or(Value::Null, |i| json!([i])));
            }
            "typeHierarchy/supertypes" | "typeHierarchy/subtypes" => {
                let name = m["params"]["item"]["name"].as_str().unwrap_or_default().to_owned();
                let up = method == "typeHierarchy/supertypes";
                let found: Vec<Value> = fake
                    .type_pairs()
                    .into_iter()
                    .filter_map(|(sub, sup)| match up {
                        true if sub == name => fake.type_item(&sup),
                        false if sup == name => fake.type_item(&sub),
                        _ => None,
                    })
                    .collect();
                reply(&mut out, Value::Array(found));
            }
            "textDocument/inlayHint" => {
                if env("FAKE_LSP_SLOW_DECOR").is_some() {
                    std::thread::sleep(std::time::Duration::from_millis(300));
                }
                let text = fake.text(m["params"]["textDocument"]["uri"].as_str().unwrap_or_default()).unwrap_or_default();
                let mut hints = Vec::new();
                for (l, line) in text.split('\n').enumerate() {
                    let line = line.trim_end_matches('\r');
                    if let Some((_, name_end)) = fake.let_name(line) {
                        hints.push(json!({ "position": { "line": l, "character": fake.units(&line[..name_end]) }, "label": ": i32", "kind": 1 }));
                    }
                    for (i, _) in line.match_indices("add(").filter(|(i, _)| !line[..*i].ends_with("fn ")) {
                        hints.push(json!({ "position": { "line": l, "character": fake.units(&line[..i + 4]) }, "label": [{ "value": "a" }, { "value": ":" }], "kind": 2, "paddingRight": true }));
                    }
                }
                if (env("FAKE_LSP_BUSY_DECOR").is_some() || env("FAKE_LSP_EMPTY_DECOR").is_some()) && !refused {
                    refused = true;
                    let _ = match env("FAKE_LSP_EMPTY_DECOR") {
                        Some(_) => write_message(&mut out, &json!({ "jsonrpc": "2.0", "id": id, "result": [] })),
                        None => write_message(&mut out, &json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32801, "message": "content modified" } })),
                    };
                    let _ = write_message(&mut out, &json!({ "jsonrpc": "2.0", "method": "$/progress", "params": { "token": "idx", "value": { "kind": "end" } } }));
                    let _ = write_message(&mut out, &json!({ "jsonrpc": "2.0", "method": "experimental/serverStatus", "params": { "health": "ok", "quiescent": true } }));
                    continue;
                }
                // An answer made before a refresh, sent after the newer one.
                if env("FAKE_LSP_STALE_DECOR").is_some() && !refreshed {
                    refreshed = true;
                    let old = hints.iter().map(|h| { let mut h = h.clone(); h["label"] = json!(": old"); h }).collect();
                    deferred = Some((id.clone().unwrap_or_default(), old));
                    let _ = write_message(&mut out, &json!({ "jsonrpc": "2.0", "id": "rf-1", "method": "workspace/inlayHint/refresh" }));
                    continue;
                }
                reply(&mut out, Value::Array(hints));
                // Once the refresh was answered, and after the newest answer is surely shown.
                if refresh_answered && let Some((old_id, old)) = deferred.take() {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    let _ = write_message(&mut out, &json!({ "jsonrpc": "2.0", "id": old_id, "result": old }));
                }
                if env("FAKE_LSP_REFRESH").is_some() && !refreshed {
                    refreshed = true;
                    let _ = write_message(&mut out, &json!({ "jsonrpc": "2.0", "id": "rf-1", "method": "workspace/inlayHint/refresh" }));
                }
            }
            "textDocument/semanticTokens/full" => {
                let text = fake.text(m["params"]["textDocument"]["uri"].as_str().unwrap_or_default()).unwrap_or_default();
                let mut found = Vec::new(); // (line, start units, length units, type, modifiers)
                for (l, line) in text.split('\n').enumerate() {
                    let line = line.trim_end_matches('\r');
                    if let Some((start, end)) = fake.let_name(line) {
                        let mutable = line[..start].ends_with("mut ");
                        found.push((l, fake.units(&line[..start]), fake.units(&line[start..end]), 0, usize::from(mutable)));
                    }
                    if let Some(i) = line.find("fn ") {
                        let name: String = line[i + 3..].chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                        let deprecated = if name.starts_with("old_") { 2 } else { 0 };
                        found.push((l, fake.units(&line[..i + 3]), fake.units(&name), 1, deprecated));
                    }
                }
                found.sort();
                let (mut data, mut at) = (Vec::new(), (0, 0));
                for (l, s, len, ty, mods) in found {
                    data.extend([l - at.0, if l == at.0 { s - at.1 } else { s }, len, ty, mods]);
                    at = (l, s);
                }
                reply(&mut out, json!({ "data": data }));
            }
            "shutdown" => {
                if env("FAKE_LSP_IGNORE_EXIT").is_none() {
                    reply(&mut out, Value::Null);
                }
            }
            "exit" if env("FAKE_LSP_IGNORE_EXIT").is_none() => std::process::exit(0),
            _ => {}
        }
    }
    // A server that ignores shutdown and its input closing: only a kill stops it.
    if env("FAKE_LSP_IGNORE_EXIT").is_some() {
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
}
