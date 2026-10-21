//! Real language servers on pinned fixtures, through the session's public commands. Each test runs only when
//! its server is installed (the reader never installs one): `READER_<NAME>` names the executable and
//! `READER_<NAME>_PATH` the `PATH` it runs with, else it is looked up on `PATH`. Run in release, alone:
//! `cargo test --release -p reader-core --test providers -- --ignored --nocapture --test-threads 1`.
#![cfg(unix)]

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use lenz_content::theme::{ThemeOverrides, ThemePaths};
use reader_core::{
    search::ResultsKind,
    semantics::{Status, provider::PROVIDERS},
    session::{Command, Completion, Config, Selection, Session},
};
use serde_json::{Value, json};

struct Real {
    _dir: tempfile::TempDir,
    root: PathBuf,
    s: Session,
}

/// The installed server for provider `id`, with the `PATH` to run it with: from `READER_<VAR>` and
/// `READER_<VAR>_PATH`, else `PATH`. `None` (the test is skipped) when it is not installed.
fn installed(id: &str, var: &str) -> Option<(PathBuf, Option<String>)> {
    let path = std::env::var(format!("READER_{var}_PATH")).ok();
    let provider = PROVIDERS.iter().find(|p| p.id == id)?;
    let exe = std::env::var_os(format!("READER_{var}")).map(PathBuf::from).or_else(|| reader_core::semantics::provider::find_executable(Path::new(provider.executable), path.as_deref().map(std::ffi::OsStr::new)))?;
    exe.is_file().then_some((exe, path))
}

fn write(root: &Path, name: &str, text: &str) -> PathBuf {
    let p = root.join(name);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, text).unwrap();
    p
}

/// A fixture project and a session whose provider `id` is the installed server (every other provider off).
fn real(id: &str, exe: &Path, path: Option<&str>, files: &[(&str, &str)]) -> Real {
    real_with(id, exe, path, Value::Null, files)
}

/// As [`real`], with the provider's own `settings`.
fn real_with(id: &str, exe: &Path, path: Option<&str>, settings: Value, files: &[(&str, &str)]) -> Real {
    real_many(&[(id, exe, path, settings)], files)
}

/// As [`real`], with several installed servers `(id, executable, PATH, settings)` (every other provider off).
fn real_many(servers: &[(&str, &Path, Option<&str>, Value)], files: &[(&str, &str)]) -> Real {
    // Not a hidden directory: Pyright (and others) exclude `**/.*` from the workspace by default.
    let dir = tempfile::Builder::new().prefix("reader-fixture-").tempdir().unwrap();
    let root = dir.path().join("fixture");
    for (name, text) in files {
        write(&root, name, text);
    }
    let mut all: serde_json::Map<String, Value> = PROVIDERS.iter().map(|p| (p.id.to_string(), json!({ "executable": format!("/nonexistent/{}", p.id) }))).collect();
    for (id, exe, path, settings) in servers {
        // An empty executable keeps the provider's own (and its fallbacks).
        let mut me = if exe.as_os_str().is_empty() { json!({}) } else { json!({ "executable": exe }) };
        if let Some(path) = path {
            me["env"] = json!({ "PATH": path });
        }
        if !settings.is_null() {
            me["settings"] = settings.clone();
        }
        all.insert(id.to_string(), me);
    }
    let providers = dir.path().join("config/providers.json");
    write(dir.path(), "config/providers.json", &json!({ "providers": all }).to_string());
    let s = Session::new(Config {
        themes: ThemePaths::under(&dir.path().join("config"), &dir.path().join("state"), "lenz-reader"),
        recent: dir.path().join("state/recent.json"),
        settings: dir.path().join("state/settings.json"),
        providers,
        cache: dir.path().join("cache"),
        idle: reader_core::semantics::IDLE,
        clock: std::sync::Arc::new(Instant::now),
        overrides: ThemeOverrides::default(),
        dark: true,
    });
    Real { _dir: dir, root, s }
}

fn recv(rx: &async_channel::Receiver<Completion>) -> Option<Completion> {
    let until = Instant::now() + Duration::from_millis(50);
    loop {
        if let Ok(c) = rx.try_recv() {
            return Some(c);
        }
        if Instant::now() >= until {
            return None;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

impl Real {
    fn settle(&mut self, what: &str, limit: Duration, done: impl Fn(&Session) -> bool) {
        let rx = self.s.completions();
        let until = Instant::now() + limit;
        while !done(&self.s) {
            assert!(Instant::now() < until, "timed out: {what}; provider {:?}; notices {:?}", self.s.state().provider(), self.s.state().notices());
            if let Some(c) = recv(&rx) {
                self.s.dispatch(c);
            }
        }
    }

    /// Open `name` and wait until its server is ready (reported quiescent, or without reported work).
    fn open_ready(&mut self, name: &str) {
        let path = self.root.join(name);
        self.s.dispatch(Command::OpenRoot(self.root.clone()));
        self.s.dispatch(Command::Open(path.clone()));
        self.settle("ready", Duration::from_secs(180), |s| s.state().document().is_some_and(|d| d.source() == path) && s.state().provider().is_some_and(|v| v.status == Status::Ready));
    }

    fn show(&mut self, name: &str) {
        let path = self.root.join(name);
        self.s.dispatch(Command::Open(path.clone()));
        self.settle("open", Duration::from_secs(10), |s| s.state().document().is_some_and(|d| d.source() == path));
    }

    fn caret(&mut self, needle: &str, skip: usize) {
        let at = self.s.state().document().unwrap().text().find(needle).unwrap_or_else(|| panic!("{needle}")) + skip;
        self.s.dispatch(Command::Select(Selection { anchor: at, head: at }));
    }

    fn selected(&self) -> String {
        let doc = self.s.state().document().unwrap();
        doc.text()[self.s.state().selection().unwrap().range()].to_owned()
    }

    fn notices(&self) -> Vec<String> {
        self.s.state().notices()
    }

    /// Ask for the definition at `needle`; wait for the main area to move (or an answer to say why not).
    fn definition(&mut self, needle: &str, skip: usize) -> Duration {
        self.caret(needle, skip);
        let t = Instant::now();
        let before = self.s.state().selection();
        self.s.dispatch(Command::Definition { peek: false });
        self.settle("definition", Duration::from_secs(60), |s| {
            s.state().asking().is_none() && s.state().loading().is_none() && s.state().document().is_some() && (s.state().selection() != before || s.state().results().is_some() || !s.state().notices().is_empty())
        });
        t.elapsed()
    }

    /// Ask for the definition at `needle` until it lands in `file`: early answers (a server still loading its
    /// project) may be empty or stop short, and must say so.
    fn definition_until(&mut self, needle: &str, skip: usize, file: &str) {
        let shown = self.s.state().document().unwrap().source().to_owned();
        let asked = Instant::now();
        loop {
            self.definition(needle, skip);
            if file_of(&self.s) == file {
                return;
            }
            assert!(self.notices().iter().any(|n| n.contains("may still be loading")), "{:?}", self.notices());
            assert!(asked.elapsed() < Duration::from_secs(30), "{:?}", self.notices());
            self.s.dispatch(Command::Open(shown.clone()));
            self.settle("reopen", Duration::from_secs(10), |s| s.state().document().is_some_and(|d| d.source() == shown));
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    /// Ask for the definition at `needle` until it reaches a file whose path contains `fragment` (directly, or as
    /// a candidate then opened): early answers may come before the server has indexed that far.
    fn definition_into(&mut self, needle: &str, skip: usize, fragment: &str) {
        let shown = self.s.state().document().unwrap().source().to_owned();
        let asked = Instant::now();
        loop {
            self.definition(needle, skip);
            if let Some(i) = self.s.state().results().and_then(|r| r.hits().iter().position(|h| h.path.contains(fragment))) {
                self.s.dispatch(Command::OpenResult(i));
                self.settle("candidate", Duration::from_secs(10), |s| s.state().document().is_some_and(|d| d.source().to_string_lossy().contains(fragment)));
            }
            if self.s.state().document().unwrap().source().to_string_lossy().contains(fragment) {
                self.s.dispatch(Command::CloseResults);
                return;
            }
            assert!(asked.elapsed() < Duration::from_secs(30), "{:?}", self.notices());
            self.s.dispatch(Command::Open(shown.clone()));
            self.settle("reopen", Duration::from_secs(10), |s| s.state().document().is_some_and(|d| d.source() == shown));
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    fn back(&mut self, name: &str) {
        self.s.dispatch(Command::Back);
        let path = self.root.join(name);
        self.settle("back", Duration::from_secs(10), |s| s.state().document().is_some_and(|d| d.source() == path));
    }

    fn usages(&mut self, needle: &str, skip: usize) -> Duration {
        self.caret(needle, skip);
        let t = Instant::now();
        self.s.dispatch(Command::Usages);
        self.settle("usages", Duration::from_secs(60), |s| s.state().asking().is_none());
        t.elapsed()
    }

    fn memory(&mut self) -> (u64, u64) {
        self.s.dispatch(Completion::Semantic(reader_core::semantics::Event::Tick));
        self.settle("memory", Duration::from_secs(10), |s| s.state().provider().is_some_and(|v| v.memory.is_some()));
        self.s.state().provider().unwrap().memory.unwrap()
    }
}

fn file_of(s: &Session) -> String {
    s.state().document().unwrap().source().file_name().unwrap().to_string_lossy().into_owned()
}

fn percentile(samples: &mut [f64], p: f64) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[((samples.len() - 1) as f64 * p).round() as usize]
}

fn report(name: &str, mut samples: Vec<f64>) {
    println!("READER_PERF {}", json!({ "name": name, "unit": "ms", "samples": samples }));
    println!("{name}: median {:.1} ms, p95 {:.1} ms, max {:.1} ms over {}", percentile(&mut samples, 0.5), percentile(&mut samples, 0.95), percentile(&mut samples, 1.0), samples.len());
}

const PY_GEOMETRY: &str = "\"\"\"Shapes.\"\"\"\n\n\nclass Círculo:\n    def __init__(self, radio: float) -> None:\n        self.radio = radio\n\n    def área(self) -> float:\n        return 3.14159 * self.radio ** 2\n\n\ndef make(radio: float) -> Círculo:\n    return Círculo(radio)\n";
const PY_APP: &str = "import os\r\nfrom pkg.geometry import Círculo, make\r\n\r\n\r\ndef main() -> None:\r\n    s = \"𝄞\"; c = make(2.0)\r\n    print(s, c.área(), os.path.join(\"a\", \"b\"), Círculo)\r\n    print(make(1.0).área())\r\n";

/// Pyright on a pinned Python fixture: definitions (Unicode, CRLF, methods, the standard library through its
/// bundled stubs), usages, workspace and file symbols, a nested project, an external edit, its reported
/// version, warm latency and memory.
#[test]
#[ignore = "needs an installed Pyright; run in release, alone"]
fn real_pyright() {
    let Some((exe, path)) = installed("pyright", "PYRIGHT") else {
        println!("SKIPPED: Pyright is not installed (set READER_PYRIGHT / READER_PYRIGHT_PATH)");
        return;
    };
    let files = [
        ("pyproject.toml", "[project]\nname = \"fixture\"\nversion = \"0\"\n"),
        ("pkg/__init__.py", ""),
        ("pkg/geometry.py", PY_GEOMETRY),
        ("app.py", PY_APP),
        ("svc/pyproject.toml", "[project]\nname = \"svc\"\nversion = \"0\"\n"),
        ("svc/svc.py", "def serve() -> int:\n    return 1\n\n\nserve()\n"),
    ];
    let mut f = real("pyright", &exe, path.as_deref(), &files);
    let started = Instant::now();
    f.open_ready("app.py");
    let view = f.s.state().provider().unwrap();
    println!("Pyright {:?} (untested: {}) ready in {:.1} s; root {:?}", view.version, view.untested, started.elapsed().as_secs_f64(), view.root);
    assert_eq!(view.root.as_deref(), Some(f.root.as_path()));
    // Definitions: a function after a non-BMP character on a CRLF line, a class, a method after non-ASCII.
    f.definition("make(2.0)", 0);
    assert_eq!((file_of(&f.s), f.selected()), ("geometry.py".to_string(), "make".to_string()), "{:?}", f.notices());
    f.back("app.py");
    f.definition("área()", 1);
    assert_eq!((file_of(&f.s), f.selected()), ("geometry.py".to_string(), "área".to_string()), "{:?}", f.notices());
    f.back("app.py");
    // The standard library: Pyright's bundled stubs, opened read-only with their origin.
    f.definition("join(", 0);
    if let Some(r) = f.s.state().results() {
        // Several stub declarations (overloads, platform variants) are candidates: open the first.
        println!("Pyright join candidates: {:?}", r.hits().iter().map(|h| (h.path.to_string(), h.line)).collect::<Vec<_>>());
        f.s.dispatch(Command::OpenResult(0));
        f.settle("candidate", Duration::from_secs(10), |s| s.state().loading().is_none() && s.state().document().is_some_and(|d| d.source().to_string_lossy().contains("typeshed")));
        f.s.dispatch(Command::CloseResults);
    }
    println!("join: {:?} {:?}", f.s.state().document().unwrap().source(), f.notices());
    assert!(f.s.state().document().unwrap().source().to_string_lossy().contains("typeshed"), "{:?}", f.s.state().document().unwrap().source());
    assert!(f.notices().iter().any(|n| n == "Read-only source from Pyright's bundled type stubs"), "{:?}", f.notices());
    f.back("app.py");
    // Usages of `make`, asked at its declaration: both calls and the import, not the definition itself.
    f.show("pkg/geometry.py");
    f.usages("def make", 4);
    let r = f.s.state().results().unwrap();
    assert_eq!(r.results_kind(), ResultsKind::Usages);
    let mut places: Vec<(String, usize)> = r.hits().iter().map(|h| (Path::new(&*h.path).file_name().unwrap().to_string_lossy().into_owned(), h.line)).collect();
    places.sort();
    println!("Pyright usages of make: {places:?}; notices {:?}", f.notices());
    assert!(places.contains(&("app.py".into(), 5)) && places.contains(&("app.py".into(), 7)), "{places:?}");
    assert!(!places.iter().any(|(f, l)| f == "geometry.py" && *l == 11), "the definition is not a usage: {places:?}");
    f.s.dispatch(Command::CloseResults);
    // Workspace and file symbols.
    // Pyright does not say when its workspace is loaded: an early empty answer must say it may be incomplete.
    let asked = Instant::now();
    let names = loop {
        f.s.dispatch(Command::Symbols("Círc".into()));
        f.settle("symbols", Duration::from_secs(30), |s| s.state().asking().is_none());
        let r = f.s.state().results().unwrap();
        let names: Vec<String> = r.hits().iter().map(|h| h.text[h.range.clone()].to_string()).collect();
        if !names.is_empty() {
            break names;
        }
        assert!(r.provider_indexing(), "an empty early answer is marked possibly incomplete");
        assert!(asked.elapsed() < Duration::from_secs(30));
        std::thread::sleep(Duration::from_millis(250));
    };
    println!("Pyright workspace symbols after {:.1} s", asked.elapsed().as_secs_f64());
    println!("Pyright workspace symbols for Círc: {names:?}");
    assert!(names.contains(&"Círculo".to_string()), "{names:?} {:?}", f.notices());
    f.show("pkg/geometry.py");
    f.s.dispatch(Command::FileSymbols);
    f.settle("file symbols", Duration::from_secs(30), |s| s.state().asking().is_none());
    let rows: Vec<(String, u16)> = f.s.state().results().unwrap().hits().iter().map(|h| (h.text.to_string(), h.decl.unwrap().1)).collect();
    println!("Pyright file symbols: {rows:?}");
    assert!(rows.contains(&("Círculo".into(), 0)) && rows.contains(&("área".into(), 1)) && rows.contains(&("make".into(), 0)), "{rows:?}");
    f.s.dispatch(Command::CloseResults);
    // An external edit: the server answers about the new text.
    std::fs::write(f.root.join("app.py"), format!("# moved down\r\n{PY_APP}")).unwrap();
    f.show("app.py");
    f.settle("refresh", Duration::from_secs(10), |s| s.state().document().is_some_and(|d| d.text().starts_with("# moved")));
    f.definition("make(1.0)", 0);
    assert_eq!((file_of(&f.s), f.selected()), ("geometry.py".to_string(), "make".to_string()), "{:?}", f.notices());
    f.back("app.py");
    // A nested project gets its own server.
    f.show("svc/svc.py");
    f.settle("nested", Duration::from_secs(120), |s| s.state().servers().len() == 2 && s.state().provider().is_some_and(|v| v.status == Status::Ready));
    assert_eq!(f.s.state().provider().unwrap().root.as_deref(), Some(f.root.join("svc").as_path()));
    f.definition("serve()\n", 0);
    assert_eq!(f.selected(), "serve");
    // Warm latency: definitions and usages, 100 each after 5 untimed.
    f.show("app.py");
    let mut defs = Vec::new();
    for i in 0..105 {
        let t = f.definition("make(2.0)", 0);
        assert_eq!(file_of(&f.s), "geometry.py");
        if i >= 5 {
            defs.push(t.as_secs_f64() * 1000.0);
        }
        f.back("app.py");
    }
    report("reader/pyright-definition-warm", defs);
    let mut uses = Vec::new();
    for i in 0..105 {
        let t = f.usages("make(2.0)", 0);
        assert!(f.s.state().results().is_some_and(|r| r.hits().len() >= 2));
        if i >= 5 {
            uses.push(t.as_secs_f64() * 1000.0);
        }
        f.s.dispatch(Command::CloseResults);
    }
    report("reader/pyright-usages-warm", uses);
    let (server, helpers) = f.memory();
    println!("Pyright memory: server {} MiB, helpers {} MiB", server >> 20, helpers >> 20);
    for (name, text) in files {
        if name != "app.py" {
            assert_eq!(std::fs::read_to_string(f.root.join(name)).unwrap(), text, "{name} unchanged");
        }
    }
}

/// Pyright without its runtime: found, but Node.js is not on the PATH it runs with. Explained, not started.
#[test]
#[ignore = "needs an installed Pyright"]
fn real_pyright_without_node() {
    let Some((exe, _)) = installed("pyright", "PYRIGHT") else {
        println!("SKIPPED: Pyright is not installed");
        return;
    };
    let empty = tempfile::tempdir().unwrap();
    let mut f = real("pyright", &exe, Some(empty.path().to_str().unwrap()), &[("pyproject.toml", ""), ("a.py", "x = 1\n")]);
    f.s.dispatch(Command::Open(f.root.join("a.py")));
    f.settle("failed", Duration::from_secs(20), |s| s.state().provider().is_some_and(|v| matches!(v.status, Status::Failed(_) | Status::Missing(_))));
    let view = f.s.state().provider().unwrap();
    println!("Pyright without Node: {}", view.summary());
    assert!(view.summary().contains("node"), "{}", view.summary());
}

fn places(s: &Session) -> Vec<(String, usize)> {
    let mut p: Vec<(String, usize)> = s.state().results().map_or(Vec::new(), |r| r.hits().iter().map(|h| (Path::new(&*h.path).file_name().unwrap().to_string_lossy().into_owned(), h.line)).collect());
    p.sort();
    p
}

/// Ask for workspace symbols until an answer has some (an empty early answer must be marked incomplete).
fn symbols(f: &mut Real, query: &str) -> Vec<String> {
    let asked = Instant::now();
    loop {
        f.s.dispatch(Command::Symbols(query.into()));
        f.settle("symbols", Duration::from_secs(30), |s| s.state().asking().is_none());
        let r = f.s.state().results().unwrap();
        let names: Vec<String> = r.hits().iter().map(|h| h.text.to_string()).collect();
        if !names.is_empty() {
            return names;
        }
        assert!(r.provider_indexing(), "an empty early answer is marked possibly incomplete: {:?}", f.notices());
        assert!(asked.elapsed() < Duration::from_secs(60));
        std::thread::sleep(Duration::from_millis(250));
    }
}

const TS_GEOMETRY: &str = "export class Círculo {\n  constructor(public radio: number) {}\n  área(): number {\n    return 3.14159 * this.radio ** 2;\n  }\n}\n\nexport function make(radio: number): Círculo {\n  return new Círculo(radio);\n}\n";
const TS_APP: &str = "import { make, Círculo } from \"./geometry\";\r\nimport { twice } from \"./util.js\";\r\n\r\nconst s = \"𝄞\"; const c = make(2);\r\nconsole.log(s, c.área(), Math.max(1, twice(2)), Círculo);\r\n";
const TS_FILES: [(&str, &str); 11] = [
    ("package.json", "{ \"name\": \"fixture\", \"private\": true, \"type\": \"module\" }\n"),
    ("tsconfig.json", "{ \"compilerOptions\": { \"strict\": true, \"jsx\": \"preserve\", \"allowJs\": true, \"module\": \"esnext\", \"moduleResolution\": \"bundler\", \"target\": \"es2022\", \"lib\": [\"es2022\", \"dom\"], \"types\": [\"shapes\"] }, \"include\": [\"src\", \"packages\"] }\n"),
    ("src/geometry.ts", TS_GEOMETRY),
    ("src/app.ts", TS_APP),
    ("src/view.tsx", "import { make } from \"./geometry\";\n\nexport const Badge = () => <b>{make(1).área()}</b>;\n"),
    ("src/util.js", "export function twice(x) {\n  return x * 2;\n}\n"),
    ("src/main.js", "import { twice } from \"./util.js\";\n\ntwice(3);\n"),
    ("packages/ui/button.ts", "import { make } from \"../../src/geometry\";\n\nexport const size = make(3).radio;\n"),
    // A type declaration package (named in `types`: TypeScript 6 includes none by default).
    ("node_modules/@types/shapes/package.json", "{ \"name\": \"@types/shapes\", \"version\": \"1.0.0\", \"types\": \"index.d.ts\" }\n"),
    ("node_modules/@types/shapes/index.d.ts", "declare function areaOf(radius: number): number;\n"),
    ("src/extra.ts", "export const a = areaOf(2);\n"),
];

/// typescript-language-server on pinned TypeScript, TSX and JavaScript fixtures: definitions (Unicode, CRLF,
/// TS→JS, TSX, JS, TypeScript's library), usages across all three, workspace and file symbols, one server for
/// nested configurations, an external edit, versions, warm latency and memory. `READER_TSSERVER` names a
/// TypeScript `tsserver.js` (set as the user's `tsserver.path`) when the global TypeScript has none.
#[test]
#[ignore = "needs an installed typescript-language-server and TypeScript with tsserver; run in release, alone"]
fn real_typescript() {
    let Some((exe, path)) = installed("typescript-language-server", "TYPESCRIPT") else {
        println!("SKIPPED: typescript-language-server is not installed");
        return;
    };
    let settings = std::env::var("READER_TSSERVER").map_or(Value::Null, |p| json!({ "tsserver": { "path": p } }));
    let mut f = real_with("typescript-language-server", &exe, path.as_deref(), settings.clone(), &TS_FILES);
    let started = Instant::now();
    f.open_ready("src/app.ts");
    let view = f.s.state().provider().unwrap();
    println!("typescript-language-server {:?} (untested: {}) ready in {:.1} s; tsserver {settings}", view.version, view.untested, started.elapsed().as_secs_f64());
    assert_eq!(view.root.as_deref(), Some(f.root.as_path()));
    // TypeScript: after a non-BMP character on a CRLF line; a method after non-ASCII; TS to JS; the library.
    // Early on (tsserver still loading the project) it may stop at the import: said so, and right when asked again.
    let asked = Instant::now();
    loop {
        f.definition("make(2)", 0);
        if file_of(&f.s) == "geometry.ts" {
            break;
        }
        println!("early answer at {:.1} s: {} {:?}", asked.elapsed().as_secs_f64(), file_of(&f.s), f.notices());
        assert!(f.notices().iter().any(|n| n.contains("may still be loading its workspace")), "{:?}", f.notices());
        assert!(asked.elapsed() < Duration::from_secs(30));
        f.s.dispatch(Command::Open(f.root.join("src/app.ts")));
        std::thread::sleep(Duration::from_millis(250));
    }
    assert_eq!(f.selected(), "make");
    f.back("src/app.ts");
    f.definition("área()", 1);
    assert_eq!((file_of(&f.s), f.selected()), ("geometry.ts".into(), "área".into()), "{:?}", f.notices());
    f.back("src/app.ts");
    f.definition("twice(2)", 0);
    assert_eq!((file_of(&f.s), f.selected()), ("util.js".into(), "twice".into()), "{:?}", f.notices());
    f.back("src/app.ts");
    f.definition("max(", 0);
    if f.s.state().results().is_some() {
        f.s.dispatch(Command::OpenResult(0));
        f.settle("candidate", Duration::from_secs(10), |s| s.state().loading().is_none() && s.state().document().is_some_and(|d| d.source().to_string_lossy().contains("lib.")));
        f.s.dispatch(Command::CloseResults);
    }
    println!("Math.max: {:?} {:?}", f.s.state().document().unwrap().source(), f.notices());
    assert!(f.notices().iter().any(|n| n == "Read-only source from TypeScript's library declarations"), "{:?}", f.notices());
    f.back("src/app.ts");
    // TSX and JavaScript files: the same server.
    f.show("src/view.tsx");
    f.definition("make(1)", 0);
    assert_eq!((file_of(&f.s), f.selected()), ("geometry.ts".into(), "make".into()), "{:?}", f.notices());
    f.back("src/view.tsx");
    f.show("src/main.js");
    f.definition("twice(3)", 0);
    assert_eq!((file_of(&f.s), f.selected()), ("util.js".into(), "twice".into()), "{:?}", f.notices());
    // A type declaration package: opened read-only with its origin, served by the same server.
    f.show("src/extra.ts");
    f.definition_until("areaOf(2)", 0, "index.d.ts");
    assert_eq!((file_of(&f.s), f.selected()), ("index.d.ts".into(), "areaOf".into()), "{:?}", f.notices());
    assert!(f.notices().iter().any(|n| n == "Read-only source from a type declaration package"), "{:?}", f.notices());
    assert_eq!(f.s.state().servers().len(), 1, "a dependency does not start a server of its own");
    f.show("packages/ui/button.ts");
    f.definition("make(3)", 0);
    assert_eq!(file_of(&f.s), "geometry.ts", "{:?}", f.notices());
    assert_eq!(f.s.state().servers().len(), 1, "nested configuration and every file kind share one server");
    // Usages of `make`, asked at its declaration: the calls and imports in TS, TSX and the nested package.
    f.show("src/geometry.ts");
    f.usages("function make", 9);
    let p = places(&f.s);
    println!("usages of make: {p:?}");
    for want in [("app.ts", 3), ("view.tsx", 2), ("button.ts", 2)] {
        assert!(p.contains(&(want.0.to_string(), want.1)), "{want:?} in {p:?}");
    }
    f.s.dispatch(Command::CloseResults);
    // JavaScript usages: `twice` from its declaration, used in JS and TS.
    f.show("src/util.js");
    f.usages("function twice", 9);
    let p = places(&f.s);
    println!("usages of twice: {p:?}");
    assert!(p.contains(&("main.js".into(), 2)) && p.contains(&("app.ts".into(), 4)), "{p:?}");
    f.s.dispatch(Command::CloseResults);
    // A replaced question: a definition asked and at once replaced by usages: only the usages apply.
    f.show("src/app.ts");
    f.caret("make(2)", 0);
    f.s.dispatch(Command::Definition { peek: false });
    f.s.dispatch(Command::Usages);
    f.settle("replaced", Duration::from_secs(30), |s| s.state().asking().is_none());
    assert_eq!(file_of(&f.s), "app.ts", "the replaced definition did not move the view");
    assert_eq!(f.s.state().results().map(|r| r.results_kind()), Some(ResultsKind::Usages));
    // A stale target: the file changes after the usages were listed; opening it is refused, rerun updates it.
    f.show("src/geometry.ts");
    f.usages("function make", 9);
    std::fs::write(f.root.join("src/view.tsx"), format!("// shifted\n{}", TS_FILES[4].1)).unwrap();
    let i = f.s.state().results().unwrap().hits().iter().position(|h| h.path.ends_with("view.tsx")).unwrap();
    f.s.dispatch(Command::OpenResult(i));
    f.settle("stale", Duration::from_secs(10), |s| s.state().notices().iter().any(|n| n.starts_with("This result no longer matches")));
    assert_eq!(file_of(&f.s), "geometry.ts");
    std::fs::write(f.root.join("src/view.tsx"), TS_FILES[4].1).unwrap();
    f.s.dispatch(Command::CloseResults);
    let names = symbols(&mut f, "Círc");
    println!("workspace symbols: {names:?}");
    assert!(names.iter().any(|n| n.contains("Círculo")), "{names:?}");
    f.s.dispatch(Command::CloseResults);
    f.s.dispatch(Command::FileSymbols);
    f.settle("file symbols", Duration::from_secs(30), |s| s.state().asking().is_none());
    let rows: Vec<(String, u16)> = f.s.state().results().unwrap().hits().iter().map(|h| (h.text.to_string(), h.decl.unwrap().1)).collect();
    println!("file symbols: {rows:?}");
    assert!(rows.contains(&("Círculo".into(), 0)) && rows.contains(&("área".into(), 1)) && rows.contains(&("make".into(), 0)), "{rows:?}");
    f.s.dispatch(Command::CloseResults);
    // An external edit.
    std::fs::write(f.root.join("src/app.ts"), format!("// moved\r\n{TS_APP}")).unwrap();
    f.show("src/app.ts");
    f.settle("refresh", Duration::from_secs(10), |s| s.state().document().is_some_and(|d| d.text().starts_with("// moved")));
    f.definition("make(2)", 0);
    assert_eq!((file_of(&f.s), f.selected()), ("geometry.ts".into(), "make".into()), "{:?}", f.notices());
    f.back("src/app.ts");
    let mut defs = Vec::new();
    for i in 0..105 {
        let t = f.definition("make(2)", 0);
        assert_eq!(file_of(&f.s), "geometry.ts");
        if i >= 5 {
            defs.push(t.as_secs_f64() * 1000.0);
        }
        f.back("src/app.ts");
    }
    report("reader/typescript-definition-warm", defs);
    f.show("src/geometry.ts");
    let mut uses = Vec::new();
    for i in 0..105 {
        let t = f.usages("function make", 9);
        assert!(f.s.state().results().is_some_and(|r| r.hits().len() >= 3));
        if i >= 5 {
            uses.push(t.as_secs_f64() * 1000.0);
        }
        f.s.dispatch(Command::CloseResults);
    }
    report("reader/typescript-usages-warm", uses);
    let (server, helpers) = f.memory();
    println!("typescript-language-server memory: server {} MiB, helpers (tsserver) {} MiB", server >> 20, helpers >> 20);
    for (name, text) in TS_FILES.iter().filter(|(n, _)| *n != "src/app.ts") {
        assert_eq!(std::fs::read_to_string(f.root.join(name)).unwrap(), *text, "{name} unchanged (app.ts was edited by the test)");
    }
}

/// typescript-go (TypeScript 7's `tsc --lsp --stdio`) on the TypeScript fixture, with no project TypeScript and
/// typescript-language-server not installed: chosen for the workspace, then definitions (Unicode, CRLF, TS→JS, TSX,
/// JS, a declaration package, TypeScript's library), usages, workspace and file symbols, one server, an external
/// edit, its version, warm latency and memory. `READER_TSGO` names a TypeScript 7 `tsc` (else `tsc` on PATH, skipped
/// when older).
#[test]
#[ignore = "needs TypeScript 7 (`tsc --lsp`); run in release, alone"]
fn real_typescript_go() {
    let Some((exe, path)) = installed("typescript-go", "TSGO") else {
        println!("SKIPPED: tsc is not installed");
        return;
    };
    let version = reader_core::semantics::provider::package_version(&exe, "typescript").unwrap_or_default();
    if !version.split('.').next().and_then(|m| m.parse::<u32>().ok()).is_some_and(|m| m >= 7) {
        println!("SKIPPED: tsc is TypeScript {version:?}, not 7");
        return;
    }
    let mut f = real("typescript-go", &exe, path.as_deref(), &TS_FILES);
    let started = Instant::now();
    f.open_ready("src/app.ts");
    let view = f.s.state().provider().unwrap();
    println!("typescript-go {:?} (untested: {}) ready in {:.1} s", view.version, view.untested, started.elapsed().as_secs_f64());
    assert_eq!((view.name, view.root.as_deref()), ("typescript-go", Some(f.root.as_path())));
    let asked = Instant::now();
    loop {
        f.definition("make(2)", 0);
        if file_of(&f.s) == "geometry.ts" {
            break;
        }
        println!("early answer at {:.1} s: {} {:?}", asked.elapsed().as_secs_f64(), file_of(&f.s), f.notices());
        assert!(asked.elapsed() < Duration::from_secs(30));
        f.s.dispatch(Command::Open(f.root.join("src/app.ts")));
        std::thread::sleep(Duration::from_millis(250));
    }
    assert_eq!(f.selected(), "make");
    f.back("src/app.ts");
    f.definition("área()", 1);
    assert_eq!((file_of(&f.s), f.selected()), ("geometry.ts".into(), "área".into()), "{:?}", f.notices());
    f.back("src/app.ts");
    f.definition("twice(2)", 0);
    assert_eq!((file_of(&f.s), f.selected()), ("util.js".into(), "twice".into()), "{:?}", f.notices());
    f.back("src/app.ts");
    f.definition("max(", 0);
    if f.s.state().results().is_some() {
        f.s.dispatch(Command::OpenResult(0));
        f.settle("candidate", Duration::from_secs(10), |s| s.state().loading().is_none());
        f.s.dispatch(Command::CloseResults);
    }
    println!("Math.max: {:?} {:?}", f.s.state().document().map(|d| d.source().to_owned()), f.notices());
    assert!(f.notices().iter().any(|n| n == "Read-only source from TypeScript's library declarations"), "{:?}", f.notices());
    f.show("src/view.tsx");
    f.definition("make(1)", 0);
    assert_eq!((file_of(&f.s), f.selected()), ("geometry.ts".into(), "make".into()), "{:?}", f.notices());
    f.show("src/main.js");
    f.definition("twice(3)", 0);
    assert_eq!((file_of(&f.s), f.selected()), ("util.js".into(), "twice".into()), "{:?}", f.notices());
    f.show("src/extra.ts");
    f.definition_until("areaOf(2)", 0, "index.d.ts");
    assert!(f.notices().iter().any(|n| n == "Read-only source from a type declaration package"), "{:?}", f.notices());
    f.show("packages/ui/button.ts");
    f.definition("make(3)", 0);
    assert_eq!(file_of(&f.s), "geometry.ts", "{:?}", f.notices());
    assert_eq!(f.s.state().servers().len(), 1, "nested configuration, every file kind and a dependency share one server");
    f.show("src/geometry.ts");
    f.usages("function make", 9);
    let p = places(&f.s);
    println!("usages of make: {p:?}");
    for want in [("app.ts", 3), ("view.tsx", 2), ("button.ts", 2)] {
        assert!(p.contains(&(want.0.to_string(), want.1)), "{want:?} in {p:?}");
    }
    f.s.dispatch(Command::CloseResults);
    let names = symbols(&mut f, "Círc");
    println!("workspace symbols: {names:?}");
    assert!(names.iter().any(|n| n.contains("Círculo")), "{names:?}");
    f.s.dispatch(Command::CloseResults);
    f.s.dispatch(Command::FileSymbols);
    f.settle("file symbols", Duration::from_secs(30), |s| s.state().asking().is_none());
    let rows: Vec<String> = f.s.state().results().unwrap().hits().iter().map(|h| h.text.to_string()).collect();
    println!("file symbols: {rows:?}");
    assert!(rows.iter().any(|r| r == "Círculo") && rows.iter().any(|r| r == "make"), "{rows:?}");
    f.s.dispatch(Command::CloseResults);
    std::fs::write(f.root.join("src/app.ts"), format!("// moved\r\n{TS_APP}")).unwrap();
    f.show("src/app.ts");
    f.settle("refresh", Duration::from_secs(10), |s| s.state().document().is_some_and(|d| d.text().starts_with("// moved")));
    f.definition("make(2)", 0);
    assert_eq!((file_of(&f.s), f.selected()), ("geometry.ts".into(), "make".into()), "{:?}", f.notices());
    f.back("src/app.ts");
    let mut defs = Vec::new();
    for i in 0..105 {
        let t = f.definition("make(2)", 0);
        assert_eq!(file_of(&f.s), "geometry.ts");
        if i >= 5 {
            defs.push(t.as_secs_f64() * 1000.0);
        }
        f.back("src/app.ts");
    }
    report("reader/typescript-go-definition-warm", defs);
    let (server, helpers) = f.memory();
    println!("typescript-go memory: server {} MiB, helpers {} MiB", server >> 20, helpers >> 20);
    for (name, text) in TS_FILES.iter().filter(|(n, _)| *n != "src/app.ts") {
        assert_eq!(std::fs::read_to_string(f.root.join(name)).unwrap(), *text, "{name} unchanged");
    }
}

/// The choice as installed, nothing set: with typescript-language-server installed beside a TypeScript 7 (no
/// tsserver anywhere) and `tsc` 7 on PATH, a project without its own TypeScript is served by typescript-go.
/// Skipped where typescript-language-server has a TypeScript with tsserver of its own.
#[test]
#[ignore = "needs typescript-language-server installed beside TypeScript 7, and `tsc` 7 on PATH"]
fn real_typescript_7_installation_chooses_typescript_go() {
    let (Some(_), Some((tsc, _))) = (installed("typescript-language-server", "TYPESCRIPT"), installed("typescript-go", "TSGO")) else {
        println!("SKIPPED: typescript-language-server or tsc is not installed");
        return;
    };
    if !reader_core::semantics::provider::package_version(&tsc, "typescript").is_some_and(|v| v.starts_with('7')) {
        println!("SKIPPED: tsc is not TypeScript 7");
        return;
    }
    // Empty executables: each provider finds its own, as for a user with no settings.
    let mut f = real_many(&[("typescript-language-server", Path::new(""), None, Value::Null), ("typescript-go", Path::new(""), None, Value::Null)], &TS_FILES);
    f.s.dispatch(Command::Open(f.root.join("src/app.ts")));
    f.settle("chosen", Duration::from_secs(30), |s| s.state().provider().is_some_and(|v| v.status == Status::Ready));
    let view = f.s.state().provider().unwrap();
    println!("chosen: {} {:?}", view.name, view.version);
    assert_eq!(view.name, "typescript-go", "{:?}", view.status);
    f.definition("make(2)", 0);
    assert_eq!(file_of(&f.s), "geometry.ts", "{:?}", f.notices());
}

/// typescript-language-server without a usable TypeScript (TypeScript 7's native compiler has no tsserver): it
/// fails to initialize, and the reader says why.
#[test]
#[ignore = "needs an installed typescript-language-server"]
fn real_typescript_without_tsserver() {
    let Some((exe, path)) = installed("typescript-language-server", "TYPESCRIPT") else {
        println!("SKIPPED: typescript-language-server is not installed");
        return;
    };
    let mut f = real_with("typescript-language-server", &exe, path.as_deref(), json!({ "tsserver": { "path": "/nonexistent/tsserver.js" } }), &TS_FILES);
    f.s.dispatch(Command::Open(f.root.join("src/app.ts")));
    f.settle("failed", Duration::from_secs(30), |s| s.state().provider().is_some_and(|v| matches!(v.status, Status::Failed(_))));
    let summary = f.s.state().provider().unwrap().summary();
    println!("without tsserver: {summary}");
    assert!(summary.contains("initialization failed") && summary.contains("TypeScript"), "{summary}");
}

const JAVA_CIRCULO: &str = "package geo;\n\npublic class Circulo {\n    private final double radio;\n\n    public Circulo(double radio) {\n        this.radio = radio;\n    }\n\n    public double área() {\n        return Math.PI * radio * radio;\n    }\n\n    public static Circulo make(double radio) {\n        return new Circulo(radio);\n    }\n}\n";
const JAVA_APP: &str = "import geo.Circulo;\r\n\r\npublic class App {\r\n    public static void main(String[] args) {\r\n        String s = \"𝄞\"; Circulo c = Circulo.make(2);\r\n        System.out.println(s + c.área() + Circulo.make(1).área());\r\n    }\r\n}\r\n";

fn tree(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut dirs = vec![root.to_owned()];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                dirs.push(p);
            } else {
                out.push(p.strip_prefix(root).unwrap().display().to_string());
            }
        }
    }
    out.sort();
    out
}

/// Eclipse JDT LS on a pinned plain Java source tree (no build file: nothing to download): definitions (Unicode,
/// CRLF), a JDK class retrieved as source through `java/classFileContents`, usages, workspace and file symbols,
/// an external edit, a restart, its version, what it writes, warm latency and memory.
#[test]
#[ignore = "needs an installed jdtls and Java 21+; run in release, alone"]
fn real_jdtls() {
    let Some((exe, path)) = installed("jdtls", "JDTLS") else {
        println!("SKIPPED: jdtls is not installed");
        return;
    };
    let files = [("src/geo/Circulo.java", JAVA_CIRCULO), ("src/App.java", JAVA_APP)];
    let mut f = real("jdtls", &exe, path.as_deref(), &files);
    let before = tree(&f.root);
    let started = Instant::now();
    f.open_ready("src/App.java");
    let view = f.s.state().provider().unwrap();
    println!("JDT LS {:?} (untested: {}) ready in {:.1} s; JAVA_HOME {:?}", view.version, view.untested, started.elapsed().as_secs_f64(), std::env::var("JAVA_HOME"));
    f.definition_until("make(2)", 0, "Circulo.java");
    assert_eq!(f.selected(), "make");
    f.back("src/App.java");
    f.definition("área()", 1);
    assert_eq!((file_of(&f.s), f.selected()), ("Circulo.java".into(), "área".into()), "{:?}", f.notices());
    f.back("src/App.java");
    // A JDK class: a virtual target, retrieved as source into a kept read-only tab.
    f.caret("println", 0);
    f.s.dispatch(Command::Definition { peek: false });
    f.settle("retrieved", Duration::from_secs(60), |s| s.state().asking().is_none() && (s.state().active().is_some_and(|t| t.retrieved().is_some()) || !s.state().notices().is_empty()));
    let tab = f.s.state().active().unwrap();
    println!("println: {} {:?}", tab.title(), f.notices());
    assert!(tab.retrieved().is_some_and(|v| v.uri.starts_with("jdt://")), "{:?}", f.notices());
    assert!(tab.title().starts_with("PrintStream") && tab.title().ends_with("(Eclipse JDT LS)"));
    assert_eq!(f.selected(), "println");
    assert!(f.s.state().document().unwrap().text().contains("class PrintStream"));
    f.back("src/App.java");
    // Usages of `make`, asked at its declaration.
    f.show("src/geo/Circulo.java");
    f.usages("Circulo make", 8);
    let p = places(&f.s);
    println!("usages of make: {p:?}");
    assert!(p.contains(&("App.java".into(), 4)) && p.contains(&("App.java".into(), 5)), "{p:?}");
    f.s.dispatch(Command::CloseResults);
    let names = symbols(&mut f, "Circ");
    println!("workspace symbols: {names:?}");
    assert!(names.iter().any(|n| n.contains("Circulo")), "{names:?}");
    f.s.dispatch(Command::CloseResults);
    f.s.dispatch(Command::FileSymbols);
    f.settle("file symbols", Duration::from_secs(30), |s| s.state().asking().is_none());
    let rows: Vec<(String, u16)> = f.s.state().results().unwrap().hits().iter().map(|h| (h.text.to_string(), h.decl.unwrap().1)).collect();
    println!("file symbols: {rows:?}");
    assert!(rows.contains(&("Circulo".into(), 0)) && rows.iter().any(|(n, d)| n == "área" && *d == 1) && rows.iter().any(|(n, d)| n == "make" && *d == 1), "{rows:?}");
    f.s.dispatch(Command::CloseResults);
    // An external edit.
    std::fs::write(f.root.join("src/App.java"), format!("// moved\r\n{JAVA_APP}")).unwrap();
    f.show("src/App.java");
    f.settle("refresh", Duration::from_secs(10), |s| s.state().document().is_some_and(|d| d.text().starts_with("// moved")));
    f.definition_until("make(2)", 0, "Circulo.java");
    f.back("src/App.java");
    let mut defs = Vec::new();
    for i in 0..105 {
        let t = f.definition("make(2)", 0);
        assert_eq!(file_of(&f.s), "Circulo.java");
        if i >= 5 {
            defs.push(t.as_secs_f64() * 1000.0);
        }
        f.back("src/App.java");
    }
    report("reader/jdtls-definition-warm", defs);
    f.show("src/geo/Circulo.java");
    let mut uses = Vec::new();
    for i in 0..105 {
        let t = f.usages("Circulo make", 8);
        assert!(f.s.state().results().is_some_and(|r| r.hits().len() >= 2));
        if i >= 5 {
            uses.push(t.as_secs_f64() * 1000.0);
        }
        f.s.dispatch(Command::CloseResults);
    }
    report("reader/jdtls-usages-warm", uses);
    let (server, helpers) = f.memory();
    println!("JDT LS memory: server {} MiB, helpers {} MiB", server >> 20, helpers >> 20);
    // A restart: a new session; the old one's answers and handles are obsolete.
    f.s.dispatch(Command::RestartServer);
    f.settle("restarted", Duration::from_secs(180), |s| s.state().provider().is_some_and(|v| v.status == Status::Ready));
    f.show("src/App.java");
    f.definition_until("make(2)", 0, "Circulo.java");
    // Above 2 GiB together: said, never acted on.
    println!("memory notice: {:?}", f.notices().iter().find(|n| n.starts_with("Language servers use")));
    let after = tree(&f.root);
    println!("files written into the project by JDT LS: {:?}", after.iter().filter(|p| !before.contains(p)).collect::<Vec<_>>());
    assert_eq!(std::fs::read_to_string(f.root.join("src/geo/Circulo.java")).unwrap(), JAVA_CIRCULO);
}

/// jdtls without Java: its launcher cannot start the server, and the reader says why.
#[test]
#[ignore = "needs an installed jdtls"]
fn real_jdtls_without_java() {
    let Some((exe, _)) = installed("jdtls", "JDTLS") else {
        println!("SKIPPED: jdtls is not installed");
        return;
    };
    // Python 3 for its launcher, but no Java anywhere it looks.
    let bin = tempfile::tempdir().unwrap();
    let python = reader_core::semantics::provider::find_executable(Path::new("python3"), None).unwrap();
    std::os::unix::fs::symlink(python, bin.path().join("python3")).unwrap();
    let mut f = real("jdtls", &exe, Some(bin.path().to_str().unwrap()), &[("src/A.java", "class A {}\n")]);
    let file = f._dir.path().join("config/providers.json");
    let mut settings: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    settings["providers"]["jdtls"]["env"]["JAVA_HOME"] = json!("/nonexistent");
    std::fs::write(&file, settings.to_string()).unwrap();
    f.s.dispatch(Command::Open(f.root.join("src/A.java")));
    f.settle("first", Duration::from_secs(30), |s| s.state().provider().is_some_and(|v| matches!(v.status, Status::Failed(_) | Status::Missing(_) | Status::Ready | Status::Indexing(_))));
    f.s.dispatch(Command::RestartServer); // with the settings written above
    f.settle("failed", Duration::from_secs(30), |s| s.state().provider().is_some_and(|v| matches!(v.status, Status::Failed(_))));
    let summary = f.s.state().provider().unwrap().summary();
    println!("without Java: {summary}");
    assert!(summary.to_lowercase().contains("java"), "{summary}");
}

const CSS: &str = ":root {\r\n  --accent: #c33;\r\n  --café: 1px;\r\n}\r\n.card { border: var(--café) solid; color: var(--accent); }\r\n@keyframes spin { to { transform: rotate(1turn); } }\r\n.card .title { animation: spin 1s; color: var(--accent); }\r\n";
const HTML: &str = "<!doctype html>\n<link rel=\"stylesheet\" href=\"style.css\">\n<style>:root { --gap: 2px; } .local { margin: var(--gap); }</style>\n<div class=\"card local\" id=\"main\">𝄞 <a href=\"#main\">top</a></div>\n<script>\nfunction greet(name) { return \"hi \" + name; }\nconst é = greet(\"x\"); greet(é);\n</script>\n";

/// The server VS Code ships (else `vscode-langservers-extracted` on PATH), with `node` on `READER_NODE_PATH`
/// (else PATH): `None` when neither is installed.
fn web_server(id: &str) -> Option<Option<String>> {
    let path = std::env::var("READER_NODE_PATH").ok();
    let p = PROVIDERS.iter().find(|p| p.id == id)?;
    let on_path = reader_core::semantics::provider::find_executable(Path::new(p.executable), path.as_deref().map(std::ffi::OsStr::new)).is_some();
    let bundled = reader_core::semantics::provider::bundled_server(&reader_core::semantics::provider::VSCODE_EXTENSIONS, p.bundled?).is_some()
        && reader_core::semantics::provider::find_executable(Path::new("node"), path.as_deref().map(std::ffi::OsStr::new)).is_some();
    (on_path || bundled).then_some(path)
}

/// VS Code's CSS server on a pinned stylesheet: custom properties and keyframes within the file (Unicode, CRLF),
/// usages, file symbols, no workspace symbols (said, not an empty success), selectors not linked to HTML, an
/// external edit, the version, warm latency and memory.
#[test]
#[ignore = "needs VS Code (or vscode-langservers-extracted) and Node.js; run in release, alone"]
fn real_css() {
    let Some(path) = web_server("vscode-css") else {
        println!("SKIPPED: no VS Code CSS server");
        return;
    };
    let mut f = real("vscode-css", Path::new(""), path.as_deref(), &[("site/style.css", CSS), ("site/index.html", HTML)]);
    let started = Instant::now();
    f.open_ready("site/style.css");
    let view = f.s.state().provider().unwrap();
    println!("VS Code CSS {:?} (untested: {}) ready in {:.1} s", view.version, view.untested, started.elapsed().as_secs_f64());
    // A custom property after a non-ASCII one on a CRLF line, and a keyframes name.
    f.definition("--accent);", 3);
    assert_eq!((file_of(&f.s), f.selected()), ("style.css".into(), "--accent".into()));
    f.definition("var(--café)", 6);
    assert_eq!(f.selected(), "--café");
    f.definition("spin 1s", 1);
    assert_eq!(f.s.state().document().unwrap().line_of(f.s.state().selection().unwrap().head), 5);
    f.usages("--accent:", 3);
    // The declaration comes too (the server ignores `includeDeclaration: false`).
    assert_eq!(places(&f.s), [("style.css".into(), 1), ("style.css".into(), 4), ("style.css".into(), 6)]);
    f.s.dispatch(Command::FileSymbols);
    f.settle("file symbols", Duration::from_secs(10), |s| s.state().asking().is_none());
    assert!(f.s.state().results().is_some_and(|r| r.hits().len() >= 3));
    // No workspace symbols: said so.
    assert_eq!(f.s.disabled(reader_core::session::CommandId::Symbols), Some("The language servers running for this folder do not provide workspace symbols; find declarations instead"));
    // A selector is not a definition the server can follow (no text match stands in).
    f.definition(".card .title", 2);
    assert!(f.notices().iter().any(|n| n.contains("VS Code CSS found no definition")), "{:?}", f.notices());
    // An external edit: the new text is what it answers about.
    std::fs::write(f.root.join("site/style.css"), format!("/* moved */\n\n{CSS}")).unwrap();
    f.settle("refresh", Duration::from_secs(10), |s| s.state().document().is_some_and(|d| d.text().starts_with("/* moved")));
    f.definition("--accent);", 3);
    assert_eq!(f.s.state().document().unwrap().line_of(f.s.state().selection().unwrap().head), 3);
    let mut warm = Vec::new();
    for i in 0..105 {
        let t = f.definition("--accent);", 3).as_secs_f64() * 1000.0;
        if i >= 5 {
            warm.push(t);
        }
    }
    report("reader/css-definition-warm", warm);
    let (rss, _) = f.memory();
    println!("VS Code CSS memory: {:.1} MiB", rss as f64 / 1048576.0);
    assert_eq!(files(&f.root), ["site/index.html", "site/style.css"], "nothing written into the project");
}

/// VS Code's HTML server on a pinned page: definitions and usages inside its own <script> and <style> (Unicode),
/// classes and ids not followed to other files, no workspace symbols, warm latency and memory.
#[test]
#[ignore = "needs VS Code (or vscode-langservers-extracted) and Node.js; run in release, alone"]
fn real_html() {
    let Some(path) = web_server("vscode-html") else {
        println!("SKIPPED: no VS Code HTML server");
        return;
    };
    let mut f = real("vscode-html", Path::new(""), path.as_deref(), &[("site/style.css", CSS), ("site/index.html", HTML)]);
    let started = Instant::now();
    f.open_ready("site/index.html");
    let view = f.s.state().provider().unwrap();
    println!("VS Code HTML {:?} (untested: {}) ready in {:.1} s", view.version, view.untested, started.elapsed().as_secs_f64());
    // A function in the page's script, called after a non-ASCII name.
    f.definition("greet(é)", 1);
    assert_eq!((file_of(&f.s), f.selected()), ("index.html".into(), "greet".into()));
    assert_eq!(f.s.state().document().unwrap().line_of(f.s.state().selection().unwrap().head), 5);
    f.usages("greet(name)", 1);
    // The declaration and both calls (the server ignores `includeDeclaration: false`).
    assert_eq!(places(&f.s), [("index.html".into(), 5), ("index.html".into(), 6), ("index.html".into(), 6)]);
    // A custom property of the page's own style.
    f.definition("var(--gap)", 5);
    assert_eq!(f.selected(), "--gap");
    // A class is not followed to the stylesheet (no text match stands in).
    f.definition("card local", 1);
    assert!(f.notices().iter().any(|n| n.contains("VS Code HTML found no definition")), "{:?}", f.notices());
    assert_eq!(f.s.disabled(reader_core::session::CommandId::Symbols), Some("The language servers running for this folder do not provide workspace symbols; find declarations instead"));
    let mut warm = Vec::new();
    for i in 0..105 {
        let t = f.definition("greet(é)", 1).as_secs_f64() * 1000.0;
        if i >= 5 {
            warm.push(t);
        }
    }
    report("reader/html-definition-warm", warm);
    let (rss, _) = f.memory();
    println!("VS Code HTML memory: {:.1} MiB", rss as f64 / 1048576.0);
    assert_eq!(files(&f.root), ["site/index.html", "site/style.css"], "nothing written into the project");
}

/// Without Node.js the built-in servers cannot run: said, with what to install; reading goes on.
#[test]
#[ignore = "needs VS Code's built-in servers"]
fn real_web_without_node() {
    if reader_core::semantics::provider::bundled_server(&reader_core::semantics::provider::VSCODE_EXTENSIONS, PROVIDERS.iter().find(|p| p.id == "vscode-css").unwrap().bundled.unwrap()).is_none() {
        println!("SKIPPED: no VS Code");
        return;
    }
    let empty = tempfile::tempdir().unwrap();
    let mut f = real("vscode-css", Path::new(""), empty.path().to_str(), &[("site/style.css", CSS)]);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.show("site/style.css");
    f.settle("missing", Duration::from_secs(10), |s| s.state().provider().is_some_and(|v| matches!(v.status, Status::Missing(_))));
    let Status::Missing(why) = f.s.state().provider().unwrap().status else { unreachable!() };
    assert!(why.contains("VS Code's built-in server") && why.contains("needs Node.js"), "{why}");
    assert!(f.s.state().document().unwrap().text().starts_with(":root"));
}

/// The files under `root`, relative and sorted.
fn files(root: &Path) -> Vec<String> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() { walk(&p, root, out) } else { out.push(p.strip_prefix(root).unwrap().to_string_lossy().into_owned()) }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

const RB_LOCK: &str = "GEM\n  remote: https://rubygems.org/\n  specs:\n\nPLATFORMS\n  ruby\n\nDEPENDENCIES\n\nBUNDLED WITH\n  4.0.15\n";
const RB_SHAPES: &str = "module Shapes\n  AREA_UNIT = \"m²\"\n\n  class Círculo\n    def initialize(radio)\n      @radio = radio\n    end\n\n    def área\n      3.14159 * @radio * @radio\n    end\n  end\nend\n";
const RB_APP: &str = "require_relative \"shapes\"\r\n\r\ns = \"𝄞\"; c = Shapes::Círculo.new(2)\r\nputs s, c.área, Shapes::AREA_UNIT\r\nputs Shapes::Círculo.new(1).área\r\nFile.join(\"a\", \"b\")\r\n";

/// Ruby LSP on a pinned fixture through its reproducible launch (`BUNDLE_GEMFILE` always set): definitions of a
/// constant, a class and a method (Unicode, CRLF), into RBS core signatures; constant usages separately from
/// method usages; workspace and file symbols; a nested bundle; an external edit; nothing written into the project
/// and no gem installed or updated; versions, warm latency and memory.
#[test]
#[ignore = "needs an installed Ruby LSP; run in release, alone"]
fn real_ruby() {
    let Some((exe, path)) = installed("ruby-lsp", "RUBY_LSP") else {
        println!("SKIPPED: Ruby LSP is not installed (set READER_RUBY_LSP / READER_RUBY_LSP_PATH)");
        return;
    };
    let gems = || std::process::Command::new("gem").args(["list", "--local"]).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let installed_before = gems();
    let mut f = real("ruby-lsp", &exe, path.as_deref(), &[("Gemfile", "source \"https://rubygems.org\"\n"), ("Gemfile.lock", RB_LOCK), ("lib/shapes.rb", RB_SHAPES), ("lib/app.rb", RB_APP), ("engines/billing/Gemfile", "source \"https://rubygems.org\"\n"), ("engines/billing/lib/invoice.rb", "module Billing\n  class Invoice\n    def total = 1\n  end\nend\n\nBilling::Invoice.new.total\n")]);
    let before = files(&f.root);
    let started = Instant::now();
    f.open_ready("lib/app.rb");
    let view = f.s.state().provider().unwrap();
    println!("Ruby LSP {:?} (untested: {}) ready in {:.1} s", view.version, view.untested, started.elapsed().as_secs_f64());
    assert_eq!(view.root.as_deref(), Some(f.root.as_path()));
    // A class after a non-BMP character on a CRLF line (early answers may be empty while it indexes, and say so).
    f.definition_until("Círculo.new(2)", 1, "shapes.rb");
    assert_eq!(f.selected(), "Círculo");
    f.show("lib/app.rb");
    f.definition("AREA_UNIT", 1);
    // A constant's target is its whole assignment.
    assert_eq!((file_of(&f.s), f.selected()), ("shapes.rb".into(), "AREA_UNIT = \"m²\"".into()));
    f.back("lib/app.rb");
    f.definition("c.área", 3);
    assert_eq!((file_of(&f.s), f.selected()), ("shapes.rb".into(), "área".into()));
    f.back("lib/app.rb");
    // Into RBS core signatures, shown with its origin.
    f.definition_into("File.join", 1, ".rbs");
    assert!(f.notices().iter().any(|n| n == "Read-only source from RBS core signatures"), "{:?}", f.notices());
    f.back("lib/app.rb");
    // Constant usages, then method usages: separately.
    // (Lines from 0; the declaration comes too: it ignores `includeDeclaration: false`.)
    f.usages("AREA_UNIT", 1);
    assert_eq!(places(&f.s), [("app.rb".into(), 3), ("shapes.rb".into(), 1)]);
    f.usages("c.área", 3);
    assert_eq!(places(&f.s), [("app.rb".into(), 3), ("app.rb".into(), 4), ("shapes.rb".into(), 8)], "method usages by name");
    // Workspace symbols match qualified names (a short name inside a long namespace can fall below its threshold).
    assert!(symbols(&mut f, "Shapes::Círculo").iter().any(|t| t.contains("class Círculo")));
    f.show("lib/shapes.rb");
    f.s.dispatch(Command::FileSymbols);
    f.settle("file symbols", Duration::from_secs(10), |s| s.state().asking().is_none());
    assert!(f.s.state().results().is_some_and(|r| !r.hits().is_empty()));
    // A nested bundle is its own project.
    f.s.dispatch(Command::Open(f.root.join("engines/billing/lib/invoice.rb")));
    f.settle("nested", Duration::from_secs(60), |s| s.state().provider().is_some_and(|v| v.status == Status::Ready && v.root.as_deref().is_some_and(|r| r.ends_with("engines/billing"))));
    let asked = Instant::now();
    loop {
        f.definition("Invoice.new", 1);
        if f.s.state().document().unwrap().line_of(f.s.state().selection().unwrap().head) == 1 {
            break;
        }
        assert!(asked.elapsed() < Duration::from_secs(30), "{:?}", f.notices());
        std::thread::sleep(Duration::from_millis(250));
    }
    // A bundle locked (offline) to an installed gem: its source opens with its origin.
    write(&f.root, "cli/Gemfile", "source \"https://rubygems.org\"\ngem \"thor\"\n");
    write(&f.root, "cli/app.rb", "require \"thor\"\n\nclass Cli < Thor\nend\n");
    let locked = std::process::Command::new("bundle").args(["lock", "--local"]).current_dir(f.root.join("cli")).env("BUNDLE_GEMFILE", f.root.join("cli/Gemfile")).output().is_ok_and(|o| o.status.success());
    if locked {
        f.s.dispatch(Command::Open(f.root.join("cli/app.rb")));
        f.settle("cli", Duration::from_secs(60), |s| s.state().provider().is_some_and(|v| v.status == Status::Ready && v.root.as_deref().is_some_and(|r| r.ends_with("cli"))));
        // Thor is reopened in many of its files: candidates, each opening the gem's source.
        f.definition_into("Thor\n", 1, "/gems/thor-");
        assert!(f.notices().iter().any(|n| n == "Read-only source from an installed gem"), "{:?}", f.notices());
    } else {
        println!("SKIPPED: the thor gem is not installed for an offline lock");
    }
    // An external edit moves the class.
    f.show("lib/app.rb");
    std::fs::write(f.root.join("lib/shapes.rb"), format!("# moved\n\n{RB_SHAPES}")).unwrap();
    f.show("lib/shapes.rb");
    f.settle("refresh", Duration::from_secs(10), |s| s.state().document().is_some_and(|d| d.text().starts_with("# moved")));
    f.show("lib/app.rb");
    let moved = Instant::now();
    loop {
        f.definition("Círculo.new(2)", 1);
        if f.s.state().document().unwrap().line_of(f.s.state().selection().unwrap().head) == 5 {
            break;
        }
        assert!(moved.elapsed() < Duration::from_secs(30), "{:?}", f.notices());
        f.show("lib/app.rb");
        std::thread::sleep(Duration::from_millis(250));
    }
    f.show("lib/app.rb");
    let mut warm = Vec::new();
    for i in 0..105 {
        let t = f.definition("AREA_UNIT", 1).as_secs_f64() * 1000.0;
        if i >= 5 {
            warm.push(t);
        }
        f.back("lib/app.rb");
    }
    report("reader/ruby-definition-warm", warm);
    let (rss, _) = f.memory();
    println!("Ruby LSP memory: {:.1} MiB", rss as f64 / 1048576.0);
    let mut after = files(&f.root);
    after.retain(|p| p != "lib/shapes.rb" && !p.starts_with("cli/"));
    let mut expected = before.clone();
    expected.retain(|p| p != "lib/shapes.rb");
    assert_eq!(after, expected, "nothing written into the project (no .ruby-lsp)");
    assert_eq!(std::fs::read_to_string(f.root.join("Gemfile.lock")).unwrap(), RB_LOCK, "the lockfile is untouched");
    assert_eq!(gems(), installed_before, "no gem installed or updated");
}

/// Installed versions of a Ruby LSP add-on gem that conflict (each activates its own version of another gem), as
/// in a version manager's gem set that has seen several: indexing finishes, because only the bundle's add-ons load
/// (0.0.3 launched it outside a bundle, its `initialized` failed with `Gem::ConflictError` and indexing stayed at 0%).
/// The composed bundle stays in the reader's cache.
#[test]
#[ignore = "needs an installed Ruby LSP and RubyGems; run in release, alone"]
fn real_ruby_conflicting_addon_versions() {
    let Some((exe, path)) = installed("ruby-lsp", "RUBY_LSP") else {
        println!("SKIPPED: Ruby LSP is not installed (set READER_RUBY_LSP / READER_RUBY_LSP_PATH)");
        return;
    };
    let gems = tempfile::tempdir().unwrap();
    let home = gems.path().join("home");
    for (name, version, addon, dependency) in [("sharedep", "1.0.0", "", ""), ("sharedep", "2.0.0", "", ""),
        ("lspaddon", "1.0.0", "gem \"sharedep\", \"1.0.0\"\n", "s.add_dependency \"sharedep\", \"1.0.0\""),
        ("lspaddon", "2.0.0", "gem \"sharedep\", \"2.0.0\"\n", "s.add_dependency \"sharedep\", \"2.0.0\"")] {
        let src = gems.path().join(format!("{name}-{version}"));
        write(&src, &format!("lib/ruby_lsp/{name}/addon.rb"), addon);
        write(&src, &format!("lib/{name}.rb"), "");
        write(&src, &format!("{name}.gemspec"), &format!("Gem::Specification.new {{ |s| s.name = {name:?}; s.version = {version:?}; s.summary = \"x\"; s.authors = [\"x\"]; s.files = Dir[\"lib/**/*.rb\"]; {dependency} }}\n"));
        let run = |args: &[&str]| std::process::Command::new("gem").args(args).current_dir(&src).output().is_ok_and(|o| o.status.success());
        // An explicit directory: a default `--user-install` (Arch's gemrc) would ignore GEM_HOME.
        assert!(run(&["build", &format!("{name}.gemspec")]) && run(&["install", "--local", "--ignore-dependencies", "--no-user-install", "--install-dir", &home.to_string_lossy(), &format!("{name}-{version}.gem")]), "fixture gem {name}-{version}");
    }
    let gem_path = std::process::Command::new("gem").args(["env", "gempath"]).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned()).unwrap();
    // Visible to the server as to `gem` itself.
    // SAFETY: a live test run alone; no other thread reads the environment meanwhile.
    unsafe { std::env::set_var("GEM_PATH", format!("{}:{gem_path}", home.display())) };
    let mut f = real("ruby-lsp", &exe, path.as_deref(), &[("Gemfile", "source \"https://rubygems.org\"\n"), ("Gemfile.lock", RB_LOCK), ("lib/shapes.rb", RB_SHAPES), ("lib/app.rb", RB_APP)]);
    let before = files(&f.root);
    f.open_ready("lib/app.rb");
    // Launched as 0.0.3 did, no index was ever built: definitions stayed unanswered (red within 30 s).
    f.definition_until("Círculo.new(2)", 1, "shapes.rb");
    assert_eq!(f.s.state().provider().unwrap().error, None, "{}", f.s.language_server_details());
    assert_eq!(files(&f.root), before, "nothing written into the project (no .ruby-lsp)");
    assert_eq!(std::fs::read_to_string(f.root.join("Gemfile.lock")).unwrap(), RB_LOCK, "the lockfile is untouched");
    let listed = std::process::Command::new("gem").args(["list", "--local", "lspaddon"]).env_remove("GEM_PATH").output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap();
    assert!(!listed.contains("lspaddon"), "fixture gems stay in their own directory: {listed}");
}

const KT_POM: &str = "<project xmlns=\"http://maven.apache.org/POM/4.0.0\">\n  <modelVersion>4.0.0</modelVersion>\n  <groupId>fixture</groupId><artifactId>mixed</artifactId><version>1</version>\n  <properties><kotlin.version>2.2.21</kotlin.version><maven.compiler.release>21</maven.compiler.release></properties>\n  <dependencies>\n    <dependency><groupId>org.jetbrains.kotlin</groupId><artifactId>kotlin-stdlib</artifactId><version>${kotlin.version}</version></dependency>\n  </dependencies>\n  <build>\n    <plugins>\n      <plugin><groupId>org.jetbrains.kotlin</groupId><artifactId>kotlin-maven-plugin</artifactId><version>${kotlin.version}</version>\n        <configuration><sourceDirs><sourceDir>src/main/kotlin</sourceDir><sourceDir>src/main/java</sourceDir></sourceDirs></configuration>\n      </plugin>\n    </plugins>\n  </build>\n</project>\n";
const KT_SHAPES: &str = "package geo\n\nclass Círculo(val radio: Double) {\n    fun área(): Double = 3.14 * radio * radio\n}\n\nfun make(r: Double) = Círculo(r)\n\nfun useJava() = app.Helper.twice(2)\n";
const KT_MAIN: &str = "package geo\r\n\r\nfun main() {\r\n    val s = \"𝄞\"; val c = make(2.0)\r\n    println(s + c.área() + make(1.0).área())\r\n}\r\n";
const KT_HELPER: &str = "package app;\n\npublic class Helper {\n    public static int twice(int x) { return x * 2; }\n    public static double use() { return geo.ShapesKt.make(1.0).área(); }\n}\n";
const KT_FILES: [(&str, &str); 4] = [("pom.xml", KT_POM), ("src/main/kotlin/geo/Shapes.kt", KT_SHAPES), ("src/main/kotlin/geo/Main.kt", KT_MAIN), ("src/main/java/app/Helper.java", KT_HELPER)];

/// Whether the fixture's Kotlin 2.2.21 Maven plugin and standard library are in the local Maven repository (so its
/// import needs no download).
fn kotlin_cached() -> bool {
    let m2 = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".m2/repository/org/jetbrains/kotlin");
    ["kotlin-maven-plugin/2.2.21", "kotlin-stdlib/2.2.21"].iter().all(|d| m2.join(d).is_dir())
}

/// Kotlin LSP on a pinned mixed Maven fixture: Kotlin to Kotlin (Unicode, CRLF) and Kotlin to Java definitions,
/// usages, file symbols, the standard library reported unavailable (inside a jar), an external edit, its version,
/// nothing written into the project, warm latency and memory.
#[test]
#[ignore = "needs an installed Kotlin LSP and Kotlin 2.2.21 in ~/.m2; run in release, alone"]
fn real_kotlin() {
    let Some((exe, path)) = installed("kotlin-lsp", "KOTLIN") else {
        println!("SKIPPED: Kotlin LSP is not installed (set READER_KOTLIN)");
        return;
    };
    if !kotlin_cached() {
        println!("SKIPPED: Kotlin 2.2.21 is not in ~/.m2 (the import would download it)");
        return;
    }
    let mut f = real("kotlin-lsp", &exe, path.as_deref(), &KT_FILES);
    let before = files(&f.root);
    let started = Instant::now();
    f.open_ready("src/main/kotlin/geo/Main.kt");
    let view = f.s.state().provider().unwrap();
    println!("Kotlin LSP {:?} (untested: {}) ready in {:.1} s", view.version, view.untested, started.elapsed().as_secs_f64());
    assert_eq!(view.root.as_deref(), Some(f.root.as_path()));
    // After a non-BMP character on a CRLF line; early answers (it is still importing) may be empty.
    f.definition_into("make(2.0)", 1, "Shapes.kt");
    assert_eq!(f.selected(), "make");
    f.show("src/main/kotlin/geo/Main.kt");
    f.definition("c.área", 3);
    assert_eq!((file_of(&f.s), f.selected()), ("Shapes.kt".into(), "área".into()));
    // Kotlin to Java.
    f.show("src/main/kotlin/geo/Shapes.kt");
    f.definition("twice(2)", 1);
    assert_eq!((file_of(&f.s), f.selected()), ("Helper.java".into(), "twice".into()));
    f.show("src/main/kotlin/geo/Main.kt");
    f.usages("make(2.0)", 1);
    let usages = places(&f.s);
    // Its index has the project's Java sources: the Java caller is among them.
    assert_eq!(usages, [("Helper.java".into(), 4), ("Main.kt".into(), 3), ("Main.kt".into(), 4)]);
    // The standard library is inside a jar: reported unavailable, not opened.
    f.definition("println", 1);
    assert!(f.notices().iter().any(|n| n.starts_with("Definition: source unavailable")), "{:?}", f.notices());
    f.s.dispatch(Command::FileSymbols);
    f.settle("file symbols", Duration::from_secs(10), |s| s.state().asking().is_none());
    assert!(f.s.state().results().is_some_and(|r| !r.hits().is_empty()));
    // An external edit moves the function.
    std::fs::write(f.root.join("src/main/kotlin/geo/Shapes.kt"), format!("// moved\n\n{KT_SHAPES}")).unwrap();
    f.show("src/main/kotlin/geo/Shapes.kt");
    f.settle("refresh", Duration::from_secs(10), |s| s.state().document().is_some_and(|d| d.text().starts_with("// moved")));
    f.show("src/main/kotlin/geo/Main.kt");
    let moved = Instant::now();
    loop {
        f.definition("make(2.0)", 1);
        if file_of(&f.s) == "Shapes.kt" && f.s.state().document().unwrap().line_of(f.s.state().selection().unwrap().head) == 8 {
            break;
        }
        assert!(moved.elapsed() < Duration::from_secs(30), "{:?}", f.notices());
        f.show("src/main/kotlin/geo/Main.kt");
        std::thread::sleep(Duration::from_millis(250));
    }
    f.show("src/main/kotlin/geo/Main.kt");
    let mut warm = Vec::new();
    for i in 0..105 {
        let t = f.definition("c.área", 3).as_secs_f64() * 1000.0;
        if i >= 5 {
            warm.push(t);
        }
        f.back("src/main/kotlin/geo/Main.kt");
    }
    report("reader/kotlin-definition-warm", warm);
    let (rss, helpers) = f.memory();
    println!("Kotlin LSP memory: {:.1} MiB (helpers {:.1} MiB)", rss as f64 / 1048576.0, helpers as f64 / 1048576.0);
    let mut after = files(&f.root);
    after.retain(|p| !p.ends_with("Shapes.kt"));
    let mut expected = before;
    expected.retain(|p| !p.ends_with("Shapes.kt"));
    assert_eq!(after, expected, "nothing written into the project");
}

/// Kotlin LSP and JDT LS on one mixed project, each for its own files: Java to Java through JDT LS, Java to Kotlin
/// not followed (said, with JDT LS's limits), Kotlin to Java through Kotlin LSP; their answers are never combined.
#[test]
#[ignore = "needs installed Kotlin LSP and JDT LS, and Kotlin 2.2.21 in ~/.m2; run in release, alone"]
fn real_kotlin_with_jdtls() {
    let (Some((kt, kt_path)), Some((jdt, jdt_path))) = (installed("kotlin-lsp", "KOTLIN"), installed("jdtls", "JDTLS")) else {
        println!("SKIPPED: Kotlin LSP or JDT LS is not installed");
        return;
    };
    if !kotlin_cached() {
        println!("SKIPPED: Kotlin 2.2.21 is not in ~/.m2");
        return;
    }
    let mut f = real_many(&[("kotlin-lsp", &kt, kt_path.as_deref(), Value::Null), ("jdtls", &jdt, jdt_path.as_deref(), Value::Null)], &KT_FILES);
    let before = files(&f.root);
    f.open_ready("src/main/java/app/Helper.java");
    assert_eq!(f.s.state().provider().unwrap().name, "Eclipse JDT LS", "Java files belong to JDT LS");
    // Java to Kotlin: JDT LS does not index Kotlin.
    f.definition("make(1.0)", 1);
    assert!(f.notices().iter().any(|n| n.contains("Eclipse JDT LS found no definition") && n.contains("Kotlin files are not part of its index")), "{:?}", f.notices());
    // Kotlin to Java through Kotlin LSP, in the same project.
    f.s.dispatch(Command::Open(f.root.join("src/main/kotlin/geo/Shapes.kt")));
    f.settle("kotlin", Duration::from_secs(120), |s| s.state().provider().is_some_and(|v| v.name == "Kotlin LSP" && v.status == Status::Ready));
    f.definition_into("twice(2)", 1, "Helper.java");
    assert_eq!(f.selected(), "twice");
    assert_eq!(f.s.state().servers().len(), 2);
    // Usages of the Java method from its declaration: JDT LS's answer only, said to leave out Kotlin callers.
    f.show("src/main/java/app/Helper.java");
    f.usages("twice(int", 1);
    assert!(f.s.state().results().is_none_or(|r| r.hits().iter().all(|h| !h.path.ends_with(".kt"))), "{:?}", places(&f.s));
    assert!(f.notices().iter().any(|n| n == "Usages: Kotlin callers are not included (JDT LS does not index Kotlin)"), "{:?}", f.notices());
    // Both imported the Maven project (JDT LS keeps its metadata in its workspace-data folder).
    assert_eq!(files(&f.root), before, "nothing written into the project");
}

/// Kotlin LSP on a Kotlin-only Maven project: definitions after a restart (Restart Language Server); a folder
/// without a build is not imported (said, with its limits); a missing server says what to install.
#[test]
#[ignore = "needs an installed Kotlin LSP and Kotlin 2.2.21 in ~/.m2; run in release, alone"]
fn real_kotlin_only_restart_no_build_and_missing() {
    let Some((exe, path)) = installed("kotlin-lsp", "KOTLIN") else {
        println!("SKIPPED: Kotlin LSP is not installed (set READER_KOTLIN)");
        return;
    };
    if !kotlin_cached() {
        println!("SKIPPED: Kotlin 2.2.21 is not in ~/.m2");
        return;
    }
    let pom = KT_POM.replace("<sourceDir>src/main/java</sourceDir>", "");
    let only = [("pom.xml", pom.as_str()), ("src/main/kotlin/geo/Shapes.kt", "package geo\n\nfun make(r: Double) = r * 2\n"), ("src/main/kotlin/geo/Main.kt", KT_MAIN.split("println").next().unwrap()), ("plain/Loose.kt", "fun loose() = 1\n\nval x = loose()\n"), ("plain/Use.kt", "val y = loose()\n")];
    let main = "package geo\r\n\r\nfun main() {\r\n    val s = \"𝄞\"; val c = make(2.0)\r\n}\r\n";
    let mut f = real("kotlin-lsp", &exe, path.as_deref(), &only);
    write(&f.root, "src/main/kotlin/geo/Main.kt", main);
    f.open_ready("src/main/kotlin/geo/Main.kt");
    f.definition_into("make(2.0)", 1, "Shapes.kt");
    // Restarted: a new session answers again.
    f.show("src/main/kotlin/geo/Main.kt");
    f.s.dispatch(Command::RestartServer);
    assert!(f.s.state().provider().is_some_and(|v| v.status != Status::Ready), "restarting");
    f.settle("restart", Duration::from_secs(120), |s| s.state().provider().is_some_and(|v| v.status == Status::Ready));
    f.definition_into("make(2.0)", 1, "Shapes.kt");
    // A folder without a build (its own browsing root): not imported, said with the provider's limits.
    let plain = f.root.join("plain");
    f.s.dispatch(Command::OpenRoot(plain.clone()));
    f.s.dispatch(Command::Open(plain.join("Loose.kt")));
    f.settle("plain", Duration::from_secs(120), |s| s.state().provider().is_some_and(|v| v.status == Status::Ready && v.root.as_deref() == Some(plain.as_path())));
    std::thread::sleep(Duration::from_secs(3));
    // Within the file it still resolves; across files it does not, and says so with its limits.
    f.definition("loose()\n", 1);
    assert_eq!((file_of(&f.s), f.s.state().document().unwrap().line_of(f.s.state().selection().unwrap().head)), ("Loose.kt".into(), 0));
    f.show("plain/Use.kt");
    std::thread::sleep(Duration::from_secs(3));
    f.definition("loose()", 1);
    assert!(f.notices().iter().any(|n| n.contains("Kotlin LSP found no definition") && n.contains("a folder without a build is not imported")), "{:?}", f.notices());
    // Missing: what to install.
    let mut g = real("kotlin-lsp", Path::new("/nonexistent/kotlin-lsp"), None, &only);
    g.s.dispatch(Command::OpenRoot(g.root.clone()));
    g.show("src/main/kotlin/geo/Shapes.kt");
    g.settle("missing", Duration::from_secs(10), |s| s.state().provider().is_some_and(|v| matches!(v.status, Status::Missing(_))));
    let Status::Missing(why) = g.s.state().provider().unwrap().status else { unreachable!() };
    assert!(why.contains("Install JetBrains' standalone Kotlin LSP"), "{why}");
    assert!(g.s.state().document().unwrap().text().starts_with("package geo"), "reading goes on");
}

const RS_FILES: [(&str, &str); 6] = [
    ("Cargo.toml", "[workspace]\nmembers = [\"shapes\", \"app\"]\nexclude = [\"loose\"]\nresolver = \"2\"\n"),
    ("shapes/Cargo.toml", "[package]\nname = \"shapes\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
    ("shapes/src/lib.rs", "pub struct Círculo {\n    pub radio: f64,\n}\n\nimpl Círculo {\n    pub fn área(&self) -> f64 {\n        3.14159 * self.radio * self.radio\n    }\n}\n\npub fn make(radio: f64) -> Círculo {\n    Círculo { radio }\n}\n"),
    ("app/Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nshapes = { path = \"../shapes\" }\n"),
    ("app/src/main.rs", "use shapes::{make, Círculo};\r\n\r\nfn main() {\r\n    let s = \"𝄞\"; let c: Círculo = make(2.0);\r\n    println!(\"{s} {}\", c.área() + make(1.0).área());\r\n    let v: Vec<u8> = Vec::new();\r\n    drop(v);\r\n}\r\n"),
    ("loose/Cargo.toml", "[package]\nname = \"loose\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
];

/// rust-analyzer on a pinned Cargo workspace: definitions across crates (Unicode, CRLF), into the standard
/// library (rust-src) with its origin, usages, workspace and file symbols, one server for the workspace, a crate
/// outside it with its own root, an external edit, its version, nothing written into the project, warm latency
/// and memory.
#[test]
#[ignore = "needs an installed rust-analyzer; run in release, alone"]
fn real_rust() {
    let Some((exe, path)) = installed("rust-analyzer", "RUST_ANALYZER") else {
        println!("SKIPPED: rust-analyzer is not installed");
        return;
    };
    let mut files = RS_FILES.to_vec();
    files.push(("loose/src/lib.rs", "pub fn lone() {}\n\npub fn caller() {\n    lone();\n}\n"));
    let mut f = real("rust-analyzer", &exe, path.as_deref(), &files);
    // Lockfiles, as projects commit them (cargo creates one when it is missing: a documented side effect).
    for dir in [f.root.clone(), f.root.join("loose")] {
        assert!(std::process::Command::new("cargo").args(["generate-lockfile", "--offline"]).current_dir(&dir).status().is_ok_and(|s| s.success()));
    }
    let before = files_of(&f.root);
    let locks: Vec<String> = ["Cargo.lock", "loose/Cargo.lock"].iter().map(|l| std::fs::read_to_string(f.root.join(l)).unwrap()).collect();
    let started = Instant::now();
    f.open_ready("app/src/main.rs");
    let view = f.s.state().provider().unwrap();
    println!("rust-analyzer {:?} (untested: {}) ready in {:.1} s", view.version, view.untested, started.elapsed().as_secs_f64());
    assert!(!view.untested, "the tested version");
    assert_eq!(view.root.as_deref(), Some(f.root.as_path()), "the workspace, not the member");
    // First answers once ready (cold): definition, then usages.
    let first = f.definition("c.área", 3);
    f.show("app/src/main.rs");
    let first_usages = f.usages("make(2.0)", 1);
    println!("rust-analyzer first answers when ready: definition {:.1} ms, usages {:.1} ms", first.as_secs_f64() * 1000.0, first_usages.as_secs_f64() * 1000.0);
    f.show("app/src/main.rs");
    // A function in another crate after a non-BMP character on a CRLF line; a method after non-ASCII.
    f.definition_into("make(2.0)", 1, "lib.rs");
    assert_eq!(f.selected(), "make");
    f.show("app/src/main.rs");
    f.definition("c.área", 3);
    assert_eq!((file_of(&f.s), f.selected()), ("lib.rs".into(), "área".into()));
    // Into the standard library (rust-src), with its origin.
    f.show("app/src/main.rs");
    f.definition("Vec::new", 6);
    assert!(f.s.state().document().unwrap().source().to_string_lossy().contains("/rustlib/src/"), "{:?}", f.s.state().document().map(|d| d.source().to_owned()));
    assert!(f.notices().iter().any(|n| n == "Read-only source from the Rust standard library"), "{:?}", f.notices());
    // The project's server answers there: no server of its own for the standard library's workspace.
    f.s.dispatch(Command::FileSymbols);
    f.settle("std file symbols", Duration::from_secs(60), |s| s.state().asking().is_none());
    assert!(f.s.state().results().is_some_and(|r| r.hits().iter().any(|h| h.text.contains("new"))), "{:?}", f.notices());
    f.s.dispatch(Command::CloseResults);
    // Understood, not only parsed: a definition from there into another standard-library file.
    f.definition("buf: RawVec<T, A>", 5);
    assert!(f.s.state().document().unwrap().source().to_string_lossy().contains("/raw_vec"), "{:?} {:?}", f.s.state().document().map(|d| d.source().to_owned()), f.notices());
    assert_eq!(f.s.state().servers().len(), 1, "{:?}", f.s.state().servers());
    f.show("app/src/main.rs");
    f.usages("make(2.0)", 1);
    assert_eq!(places(&f.s), [("main.rs".into(), 0), ("main.rs".into(), 3), ("main.rs".into(), 4)], "uses and the import (the declaration left out)");
    assert!(symbols(&mut f, "Círculo").iter().any(|t| t.contains("pub struct Círculo")));
    f.show("shapes/src/lib.rs");
    f.s.dispatch(Command::FileSymbols);
    f.settle("file symbols", Duration::from_secs(10), |s| s.state().asking().is_none());
    assert!(f.s.state().results().is_some_and(|r| r.hits().len() >= 4));
    // A crate outside the workspace: its own server, rooted at it.
    f.s.dispatch(Command::Open(f.root.join("loose/src/lib.rs")));
    f.settle("loose", Duration::from_secs(120), |s| s.state().provider().is_some_and(|v| v.status == Status::Ready && v.root.as_deref().is_some_and(|r| r.ends_with("loose"))));
    assert_eq!(f.s.state().servers().len(), 2);
    // An external edit moves the function.
    std::fs::write(f.root.join("shapes/src/lib.rs"), format!("// moved\n\n{}", RS_FILES[2].1)).unwrap();
    f.show("shapes/src/lib.rs");
    f.settle("refresh", Duration::from_secs(10), |s| s.state().document().is_some_and(|d| d.text().starts_with("// moved")));
    f.show("app/src/main.rs");
    let moved = Instant::now();
    loop {
        f.definition("make(2.0)", 1);
        if file_of(&f.s) == "lib.rs" && f.s.state().document().unwrap().line_of(f.s.state().selection().unwrap().head) == 12 {
            break;
        }
        assert!(moved.elapsed() < Duration::from_secs(30), "{:?}", f.notices());
        f.show("app/src/main.rs");
        std::thread::sleep(Duration::from_millis(250));
    }
    f.show("app/src/main.rs");
    let (mut warm, mut usages) = (Vec::new(), Vec::new());
    for i in 0..105 {
        let t = f.definition("c.área", 3).as_secs_f64() * 1000.0;
        f.back("app/src/main.rs");
        let u = f.usages("make(2.0)", 1).as_secs_f64() * 1000.0;
        if i >= 5 {
            warm.push(t);
            usages.push(u);
        }
    }
    report("reader/rust-definition-warm", warm);
    report("reader/rust-usages-warm", usages);
    let (rss, helpers) = f.memory();
    println!("rust-analyzer memory: {:.1} MiB (helpers {:.1} MiB)", rss as f64 / 1048576.0, helpers as f64 / 1048576.0);
    let mut after = files_of(&f.root);
    after.retain(|p| p != "shapes/src/lib.rs");
    let mut expected = before;
    expected.retain(|p| p != "shapes/src/lib.rs");
    assert_eq!(after, expected, "nothing written into the project (no target/)");
    assert_eq!(["Cargo.lock", "loose/Cargo.lock"].iter().map(|l| std::fs::read_to_string(f.root.join(l)).unwrap()).collect::<Vec<_>>(), locks, "lockfiles untouched");
}

/// The project's files, `target/` and lockfiles included (a build or a resolve would add them).
fn files_of(root: &Path) -> Vec<String> {
    files(root)
}

/// rust-analyzer cold on a real project (`READER_CORPUS`, a disposable checkout: the pinned Lenz revision):
/// launch to ready (quiescent; its builds go to the reader's cache), then the first definition and usages once
/// ready, then warm samples. Records the project's own numbers; nothing is asserted about them but correctness.
#[test]
#[ignore = "needs an installed rust-analyzer and a corpus; run in release, alone"]
fn real_rust_corpus_readiness() {
    let (Some((exe, path)), Ok(corpus)) = (installed("rust-analyzer", "RUST_ANALYZER"), std::env::var("READER_CORPUS")) else {
        println!("SKIPPED: rust-analyzer or READER_CORPUS missing");
        return;
    };
    let mut f = real("rust-analyzer", &exe, path.as_deref(), &[]);
    f.root = PathBuf::from(corpus);
    let file = "crates/engine/src/lib.rs";
    let started = Instant::now();
    f.open_ready(file);
    let ready = started.elapsed();
    let first = f.definition("merge_lines", 2);
    let target = file_of(&f.s);
    f.show(file);
    let first_usages = f.usages("merge_lines", 2);
    let usages = places(&f.s).len();
    f.show(file);
    let mut warm = Vec::new();
    for i in 0..105 {
        let t = f.definition("merge_lines", 2).as_secs_f64() * 1000.0;
        f.show(file);
        if i >= 5 {
            warm.push(t);
        }
    }
    let (rss, helpers) = f.memory();
    println!("rust-analyzer on {}: ready in {:.1} s; first definition {:.1} ms (into {target}), first usages {:.1} ms ({usages} places); memory {:.0} MiB + helpers {:.0} MiB", f.root.display(), ready.as_secs_f64(), first.as_secs_f64() * 1000.0, first_usages.as_secs_f64() * 1000.0, rss as f64 / 1048576.0, helpers as f64 / 1048576.0);
    assert_eq!(target, "merge.rs");
    report("reader/rust-corpus-definition-warm", warm);
}

/// typescript-language-server cold on a real frontend (`READER_CORPUS`: the pinned Metabase checkout, without
/// `node_modules`): ready, the first definition across files through its path aliases, first usages, warm samples.
#[test]
#[ignore = "needs typescript-language-server, a tsserver and a corpus; run in release, alone"]
fn real_typescript_corpus_readiness() {
    let (Some((exe, path)), Ok(corpus)) = (installed("typescript-language-server", "TYPESCRIPT"), std::env::var("READER_CORPUS")) else {
        println!("SKIPPED: typescript-language-server or READER_CORPUS missing");
        return;
    };
    let settings = std::env::var("READER_TSSERVER").map_or(Value::Null, |p| json!({ "tsserver": { "path": p } }));
    let mut f = real_with("typescript-language-server", &exe, path.as_deref(), settings, &[]);
    f.root = PathBuf::from(corpus);
    let file = "frontend/src/metabase/visualizations/visualizations/SmartScalar/utils.ts";
    let started = Instant::now();
    f.open_ready(file);
    let ready = started.elapsed();
    let asked = Instant::now();
    f.definition_into(": formatNumber(n", 3, "numbers.tsx");
    let first = asked.elapsed();
    f.show(file);
    let first_usages = f.usages(": formatNumber(n", 3);
    let usages = places(&f.s).len();
    f.show(file);
    let mut warm = Vec::new();
    for i in 0..105 {
        let t = f.definition(": formatNumber(n", 3).as_secs_f64() * 1000.0;
        f.show(file);
        if i >= 5 {
            warm.push(t);
        }
    }
    let (rss, helpers) = f.memory();
    println!("typescript-language-server on {}: ready in {:.1} s; first definition reached in {:.0} ms (retries while it loads), first usages {:.0} ms ({usages} places); memory {:.0} MiB + tsserver {:.0} MiB", f.root.display(), ready.as_secs_f64(), first.as_secs_f64() * 1000.0, first_usages.as_secs_f64() * 1000.0, rss as f64 / 1048576.0, helpers as f64 / 1048576.0);
    report("reader/typescript-corpus-definition-warm", warm);
}

/// rust-analyzer's inlay hints and semantic tokens: a type, a parameter name and a mutable variable.
#[test]
#[ignore = "needs an installed rust-analyzer; run in release, alone"]
fn real_rust_analyzer_decorations() {
    let Some((exe, path)) = installed("rust-analyzer", "RUST_ANALYZER") else {
        println!("SKIPPED: rust-analyzer is not installed");
        return;
    };
    let main = "fn main() {\n    let mut v = vec![1];\n    v.push(2);\n    takes(5);\n}\nfn takes(count: i32) -> i32 {\n    count\n}\n";
    let mut f = real("rust-analyzer", &exe, path.as_deref(), &[("Cargo.toml", "[package]\nname = \"p\"\nedition = \"2021\"\n"), ("src/main.rs", main)]);
    f.open_ready("src/main.rs");
    let v = "    let mut ".len();
    let mutable = move |t: &reader_core::tabs::Tab| t.spans(1).iter().any(|(r, _, m)| r.start == v && m & reader_core::decor::MUTABLE != 0);
    // Answers made while it loads may lack the semantics; the final ones (asked again once ready) have them.
    f.settle("decorations", Duration::from_secs(120), |s| s.state().active().is_some_and(|t| t.hints().is_some_and(|h| h.len() >= 2) && mutable(t)));
    let t = f.s.state().active().unwrap();
    println!("line 1: {}\nline 3: {}", t.shown_line(1).text, t.shown_line(3).text);
    assert!(t.shown_line(1).text.contains("v: Vec<i32"), "{}", t.shown_line(1).text);
    assert!(t.shown_line(3).text.contains("count: 5"), "{}", t.shown_line(3).text);
    println!("hints {} ({} B), tokens {} ({} B)", t.hints().unwrap().len(), t.hints().unwrap().bytes(), t.tokens().unwrap().len(), t.tokens().unwrap().bytes());
}

#[test]
#[ignore = "needs an installed rust-analyzer; run in release, alone"]
fn real_rust_analyzer_type_hierarchy_by_syntax() {
    let Some((exe, path)) = installed("rust-analyzer", "RUST_ANALYZER") else { return println!("SKIPPED: rust-analyzer is not installed") };
    let lib = "pub trait Shape {}\npub trait Round: Shape {}\npub struct Circle;\nimpl Shape for Circle {}\nimpl Round for Circle {}\n";
    let mut f = real("rust-analyzer", &exe, path.as_deref(), &[("Cargo.toml", "[package]\nname = \"p\"\nedition = \"2021\"\n"), ("src/lib.rs", lib)]);
    f.open_ready("src/lib.rs");
    f.caret("Shape", 0);
    f.s.dispatch(Command::TypeHierarchy(reader_core::semantics::hierarchy::Direction::Subtypes));
    f.settle("subtypes", Duration::from_secs(60), |s| s.state().results().is_some_and(|r| r.hits().len() == 3));
    f.caret("Circle", 0);
    f.s.dispatch(Command::TypeHierarchy(reader_core::semantics::hierarchy::Direction::Supertypes));
    f.settle("supertypes", Duration::from_secs(60), |s| s.state().results().is_some_and(|r| r.kind().starts_with("Supertypes of Circle") && r.hits().len() == 3));
}

#[test]
#[ignore = "needs an installed jdtls; run in release, alone"]
fn real_jdtls_type_hierarchy() {
    let Some((exe, path)) = installed("jdtls", "JDTLS") else { return println!("SKIPPED: jdtls is not installed") };
    // An Eclipse project (no downloads): JDT LS searches a plain source tree for neither subtypes nor implementations.
    let project = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<projectDescription><name>fixture</name><comment></comment><projects></projects><buildSpec><buildCommand><name>org.eclipse.jdt.core.javabuilder</name><arguments></arguments></buildCommand></buildSpec><natures><nature>org.eclipse.jdt.core.javanature</nature></natures></projectDescription>\n";
    let classpath = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<classpath><classpathentry kind=\"src\" path=\"src\"/><classpathentry kind=\"con\" path=\"org.eclipse.jdt.launching.JRE_CONTAINER\"/><classpathentry kind=\"output\" path=\"bin\"/></classpath>\n";
    let files = [(".project", project), (".classpath", classpath), ("src/geo/Shape.java", "package geo;\npublic interface Shape {}\n"), ("src/geo/Circle.java", "package geo;\npublic class Circle implements Shape {}\n"), ("src/geo/Big.java", "package geo;\npublic class Big extends Circle {}\n")];
    let mut f = real("jdtls", &exe, path.as_deref(), &files);
    f.open_ready("src/geo/Shape.java");
    f.caret("Shape", 0);
    // JDT LS may answer subtypes empty while its search index still fills after it reports ready: ask again.
    let has = |s: &reader_core::session::Session| s.state().results().is_some_and(|r| (0..r.hits().len()).filter_map(|i| r.call(i)).any(|c| c.name == "Circle"));
    let started = Instant::now();
    while !has(&f.s) {
        assert!(started.elapsed() < Duration::from_secs(180), "no subtypes from JDT LS");
        f.s.dispatch(Command::TypeHierarchy(reader_core::semantics::hierarchy::Direction::Subtypes));
        let until = Instant::now() + Duration::from_secs(3);
        f.settle("a level", Duration::from_secs(60), |s| Instant::now() > until && s.state().results().is_some_and(|r| r.call(0).is_some_and(|c| c.fold != reader_core::search::Fold::Loading)) || has(s));
    }
    println!("JDT LS subtypes after {:.1} s", started.elapsed().as_secs_f64());
}

/// JDT LS on a build-tool project with the reader's trigger file: Go to Definition and subtypes still work, and the
/// build importer, not a second project, serves it.
fn jdtls_on_a_build_project(build: (&str, &str), dir: &str) {
    let Some((exe, path)) = installed("jdtls", "JDTLS") else { return println!("SKIPPED: jdtls is not installed") };
    let src = |n: &str| format!("{dir}/geo/{n}");
    let (shape, circle, app) = (src("Shape.java"), src("Circle.java"), src("App.java"));
    let files = [build, (shape.as_str(), "package geo;\npublic interface Shape {}\n"), (circle.as_str(), "package geo;\npublic class Circle implements Shape {}\n"), (app.as_str(), "package geo;\npublic class App {\n    Shape s = new Circle();\n}\n")];
    let mut f = real("jdtls", &exe, path.as_deref(), &files);
    let started = Instant::now();
    f.open_ready(&app);
    f.definition_until("Circle()", 0, "Circle.java");
    println!("definition after {:.1} s", started.elapsed().as_secs_f64());
    f.s.dispatch(Command::Open(f.root.join(&shape)));
    f.settle("shape", Duration::from_secs(30), |s| s.state().document().is_some_and(|d| d.source().ends_with("Shape.java")));
    f.caret("Shape", 0);
    let has = |s: &reader_core::session::Session| s.state().results().is_some_and(|r| (0..r.hits().len()).filter_map(|i| r.call(i)).any(|c| c.name == "Circle"));
    while !has(&f.s) {
        assert!(started.elapsed() < Duration::from_secs(240), "no subtypes from JDT LS");
        f.s.dispatch(Command::TypeHierarchy(reader_core::semantics::hierarchy::Direction::Subtypes));
        let until = Instant::now() + Duration::from_secs(3);
        f.settle("a level", Duration::from_secs(60), |s| Instant::now() > until && s.state().results().is_some_and(|r| r.call(0).is_some_and(|c| c.fold != reader_core::search::Fold::Loading)) || has(s));
    }
    let rows = f.s.state().results().map_or(0, |r| (0..r.hits().len()).filter_map(|i| r.call(i)).filter(|c| c.name == "Circle").count());
    assert_eq!(rows, 1, "one Circle: not imported twice");
    println!("subtypes after {:.1} s", started.elapsed().as_secs_f64());
}

#[test]
#[ignore = "needs an installed jdtls and Maven's local repository; run in release, alone"]
fn real_jdtls_on_a_maven_project() {
    let pom = "<project xmlns=\"http://maven.apache.org/POM/4.0.0\"><modelVersion>4.0.0</modelVersion><groupId>t</groupId><artifactId>t</artifactId><version>1</version><properties><maven.compiler.release>17</maven.compiler.release></properties></project>\n";
    jdtls_on_a_build_project(("pom.xml", pom), "src/main/java");
}

#[test]
#[ignore = "needs an installed jdtls and Gradle; run in release, alone"]
fn real_jdtls_on_a_gradle_project() {
    jdtls_on_a_build_project(("build.gradle", "plugins { id 'java' }\n"), "src/main/java");
}

/// JDT LS on a plain folder of sources (no `.project`, no build file): subtypes, and nothing written into the
/// folder. `READER_JDTLS_TRY` (JSON; `{root}` becomes the folder's URI) replaces jdtls's provider settings, to try a
/// configuration; the server is restarted to read it.
#[test]
#[ignore = "needs an installed jdtls; run in release, alone"]
fn real_jdtls_type_hierarchy_on_a_plain_folder() {
    let Some((exe, path)) = installed("jdtls", "JDTLS") else { return println!("SKIPPED: jdtls is not installed") };
    let files = [("src/geo/Shape.java", "package geo;\npublic interface Shape {}\n"), ("src/geo/Circle.java", "package geo;\npublic class Circle implements Shape {}\n"), ("src/geo/Big.java", "package geo;\npublic class Big extends Circle {}\n")];
    let mut f = real("jdtls", &exe, path.as_deref(), &files);
    let tried = std::env::var("READER_JDTLS_TRY").ok();
    if let Some(t) = &tried {
        let uri = format!("file://{}", f.root.display());
        let file = f.root.parent().unwrap().join("config/providers.json");
        let mut all: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        all["providers"]["jdtls"]["settings"] = serde_json::from_str(&t.replace("{root}", &uri)).unwrap();
        std::fs::write(&file, all.to_string()).unwrap();
    }
    f.open_ready("src/geo/Shape.java");
    if tried.is_some() {
        f.s.dispatch(Command::RestartServer);
        f.settle("restarted", Duration::from_secs(180), |s| s.state().provider().is_some_and(|v| v.status == Status::Ready));
    }
    f.caret("Shape", 0);
    let has = |s: &reader_core::session::Session| s.state().results().is_some_and(|r| (0..r.hits().len()).filter_map(|i| r.call(i)).any(|c| c.name == "Circle"));
    let started = Instant::now();
    while !has(&f.s) {
        assert!(started.elapsed() < Duration::from_secs(180), "no subtypes from JDT LS on a plain folder");
        f.s.dispatch(Command::TypeHierarchy(reader_core::semantics::hierarchy::Direction::Subtypes));
        let until = Instant::now() + Duration::from_secs(3);
        f.settle("a level", Duration::from_secs(60), |s| Instant::now() > until && s.state().results().is_some_and(|r| r.call(0).is_some_and(|c| c.fold != reader_core::search::Fold::Loading)) || has(s));
    }
    println!("JDT LS subtypes on a plain folder after {:.1} s", started.elapsed().as_secs_f64());
    // Read-only: the folder holds exactly the files written (no .project, .classpath, .settings, bin).
    let mut found = Vec::new();
    let mut dirs = vec![f.root.clone()];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            if e.path().is_dir() { dirs.push(e.path()) } else { found.push(e.path().strip_prefix(&f.root).unwrap().to_string_lossy().into_owned()) }
        }
    }
    found.sort();
    let mut written: Vec<String> = files.iter().map(|(n, _)| n.to_string()).collect();
    written.sort();
    assert_eq!(found, written, "nothing written into the folder");
}

/// Time to a type-hierarchy level on a real workspace: the subtypes of `Default` in a copy of this repository
/// (rust-analyzer's usages, read against syntax relations).
#[test]
#[ignore = "timing; needs an installed rust-analyzer; run in release, alone"]
fn real_rust_analyzer_type_hierarchy_timing() {
    let Some((exe, path)) = installed("rust-analyzer", "RUST_ANALYZER") else { return println!("SKIPPED: rust-analyzer is not installed") };
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let mut files: Vec<(String, String)> = Vec::new();
    let mut dirs = vec![repo.join("crates")];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                dirs.push(p);
            } else if p.extension().is_some_and(|x| x == "rs" || x == "toml") {
                files.push((p.strip_prefix(&repo).unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&p).unwrap()));
            }
        }
    }
    // A `[patch]` path is relative to the repository; the copy points at the same place.
    let manifest = std::fs::read_to_string(repo.join("Cargo.toml")).unwrap().replace("\"../", &format!("\"{}/../", repo.display()));
    files.push(("Cargo.toml".into(), manifest));
    files.push(("Cargo.lock".into(), std::fs::read_to_string(repo.join("Cargo.lock")).unwrap()));
    let refs: Vec<(&str, &str)> = files.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let mut f = real("rust-analyzer", &exe, path.as_deref(), &refs);
    f.open_ready("crates/reader-core/src/session.rs");
    f.caret("Default for Settings", 0);
    let started = Instant::now();
    f.s.dispatch(Command::TypeHierarchy(reader_core::semantics::hierarchy::Direction::Subtypes));
    f.settle("level", Duration::from_secs(120), |s| s.state().results().is_some_and(|r| r.call(0).is_some_and(|c| c.fold != reader_core::search::Fold::Loading && c.fold != reader_core::search::Fold::Closed)));
    let r = f.s.state().results().unwrap();
    println!("subtypes of Default: level of {} in {} ms", r.hits().len() - 1, started.elapsed().as_millis());
    // The same server's usages at the same place, for comparison: most of the level's time is the server's.
    f.caret("Default for Settings", 0);
    let started = Instant::now();
    f.s.dispatch(Command::Usages);
    f.settle("usages", Duration::from_secs(120), |s| s.state().results().is_some_and(|r| r.results_kind() == reader_core::search::ResultsKind::Usages && r.done()));
    println!("usages of Default: {} in {} ms", f.s.state().results().unwrap().hits().len(), started.elapsed().as_millis());
    f.caret("Default for Settings", 0);
    let started = Instant::now();
    f.s.dispatch(Command::TypeHierarchy(reader_core::semantics::hierarchy::Direction::Subtypes));
    f.settle("level again", Duration::from_secs(120), |s| s.state().results().is_some_and(|r| r.results_kind() == reader_core::search::ResultsKind::Hierarchy && r.call(0).is_some_and(|c| c.fold != reader_core::search::Fold::Loading && c.fold != reader_core::search::Fold::Closed)));
    println!("subtypes of Default again (warm): level of {} in {} ms", f.s.state().results().unwrap().hits().len() - 1, started.elapsed().as_millis());
}
