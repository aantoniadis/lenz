//! Real language servers on pinned fixtures, through the session's public commands. Each test runs only when
//! its server is installed (the reader never installs one): `READER_<NAME>` names the executable and
//! `READER_<NAME>_PATH` the `PATH` it runs with, else it is looked up on `PATH`. Run in release, alone:
//! `cargo test --release -p code-core --test providers -- --ignored --nocapture --test-threads 1`.
#![cfg(unix)]

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use lenz_content::theme::{ThemeOverrides, ThemePaths};
use code_core::{
    search::ResultsKind,
    semantics::{Status, provider::PROVIDERS},
    session::{Command, Completion, Config, Selection, Session},
};
use serde_json::{Value, json};

fn current_text(state: &code_core::session::State) -> String {
    state.current_source().map(|s| s.copy_range(0..s.len_bytes()).unwrap()).unwrap_or_default()
}
fn active_view(s: &Session) -> Option<code_core::session::View<'_>> {
    s.state().view(s.state().active()?.id())
}
fn projected(view: &code_core::session::View<'_>, line: usize) -> String {
    let mut cursor = view.display_start(line).unwrap();
    let mut text = String::new();
    loop { let window = view.display_window(&mut cursor, 4096).unwrap(); text.push_str(&window.shown.text); if window.complete { return text; } }
}

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
    let exe = std::env::var_os(format!("READER_{var}")).map(PathBuf::from).or_else(|| code_core::semantics::provider::find_executable(Path::new(provider.executable), path.as_deref().map(std::ffi::OsStr::new)))?;
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
    real_entries(|_| servers.iter().map(|(id, exe, path, settings)| {
        // An empty executable keeps the provider's own (and its fallbacks).
        let mut me = if exe.as_os_str().is_empty() { json!({}) } else { json!({ "executable": exe }) };
        if let Some(path) = path {
            me["env"] = json!({ "PATH": path });
        }
        if !settings.is_null() {
            me["settings"] = settings.clone();
        }
        (id.to_string(), me)
    }).collect(), files)
}

/// As [`real`], with the whole providers.json entry of each installed server, made knowing the fixture's root (for
/// per-project settings); every other provider off.
fn real_entries(entries: impl FnOnce(&Path) -> Vec<(String, Value)>, files: &[(&str, &str)]) -> Real {
    // Not a hidden directory: Pyright (and others) exclude `**/.*` from the workspace by default.
    let dir = tempfile::Builder::new().prefix("reader-fixture-").tempdir().unwrap();
    let root = dir.path().join("fixture");
    for (name, text) in files {
        write(&root, name, text);
    }
    let mut all: serde_json::Map<String, Value> = PROVIDERS.iter().map(|p| (p.id.to_string(), json!({ "executable": format!("/nonexistent/{}", p.id) }))).collect();
    all.extend(entries(&root));
    let providers = dir.path().join("config/providers.json");
    write(dir.path(), "config/providers.json", &json!({ "providers": all }).to_string());
    // Draft input as on a validated backend: completion edits the draft (never the fixture on disk).
    let s = Session::with_text_input(Config {
        themes: ThemePaths::under(&dir.path().join("config"), &dir.path().join("state"), "lenz-code"),
        recent: dir.path().join("state/recent.json"),
        settings: dir.path().join("state/settings.json"),
        providers,
        cache: dir.path().join("cache"),
        idle: code_core::semantics::IDLE,
        clock: std::sync::Arc::new(Instant::now),
        overrides: ThemeOverrides::default(),
        dark: true,
    }, true);
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
        self.settle("ready", Duration::from_secs(180), |s| s.state().current_source().is_some_and(|d| d.source() == path) && s.state().provider().is_some_and(|v| v.status == Status::Ready));
    }

    fn show(&mut self, name: &str) {
        let path = self.root.join(name);
        self.s.dispatch(Command::Open(path.clone()));
        self.settle("open", Duration::from_secs(10), |s| s.state().current_source().is_some_and(|d| d.source() == path));
    }

    fn caret(&mut self, needle: &str, skip: usize) {
        let at = current_text(self.s.state()).find(needle).unwrap_or_else(|| panic!("{needle}")) + skip;
        self.s.dispatch(Command::Select(Selection { anchor: at, head: at }));
    }

    fn selected(&self) -> String {
        let doc = self.s.state().current_source().unwrap();
        doc.copy_range(self.s.state().selection().unwrap().range()).unwrap()
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
            s.state().asking().is_none() && s.state().loading().is_none() && s.state().current_source().is_some() && (s.state().selection() != before || s.state().results().is_some() || !s.state().notices().is_empty())
        });
        t.elapsed()
    }

    /// Ask for the definition at `needle` until it lands in `file`: early answers (a server still loading its
    /// project) may be empty or stop short, and must say so.
    fn definition_until(&mut self, needle: &str, skip: usize, file: &str) {
        let shown = self.s.state().current_source().unwrap().source().to_owned();
        let asked = Instant::now();
        loop {
            self.definition(needle, skip);
            if file_of(&self.s) == file {
                return;
            }
            assert!(self.notices().iter().any(|n| n.contains("may still be loading")), "{:?}", self.notices());
            assert!(asked.elapsed() < Duration::from_secs(30), "{:?}", self.notices());
            self.s.dispatch(Command::Open(shown.clone()));
            self.settle("reopen", Duration::from_secs(10), |s| s.state().current_source().is_some_and(|d| d.source() == shown));
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    /// Ask for the definition at `needle` until it reaches a file whose path contains `fragment` (directly, or as
    /// a candidate then opened): early answers may come before the server has indexed that far.
    fn definition_into(&mut self, needle: &str, skip: usize, fragment: &str) {
        let shown = self.s.state().current_source().unwrap().source().to_owned();
        let asked = Instant::now();
        loop {
            self.definition(needle, skip);
            if let Some(i) = self.s.state().results().and_then(|r| r.hits().iter().position(|h| h.path.contains(fragment))) {
                self.s.dispatch(Command::OpenResult(i));
                self.settle("candidate", Duration::from_secs(10), |s| s.state().current_source().is_some_and(|d| d.source().to_string_lossy().contains(fragment)));
            }
            if self.s.state().current_source().unwrap().source().to_string_lossy().contains(fragment) {
                self.s.dispatch(Command::CloseResults);
                return;
            }
            assert!(asked.elapsed() < Duration::from_secs(30), "{:?}", self.notices());
            self.s.dispatch(Command::Open(shown.clone()));
            self.settle("reopen", Duration::from_secs(10), |s| s.state().current_source().is_some_and(|d| d.source() == shown));
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    fn back(&mut self, name: &str) {
        self.s.dispatch(Command::Back);
        let path = self.root.join(name);
        self.settle("back", Duration::from_secs(10), |s| s.state().current_source().is_some_and(|d| d.source() == path));
    }

    fn usages(&mut self, needle: &str, skip: usize) -> Duration {
        self.caret(needle, skip);
        let t = Instant::now();
        self.s.dispatch(Command::Usages);
        self.settle("usages", Duration::from_secs(60), |s| s.state().asking().is_none());
        t.elapsed()
    }

    fn memory(&mut self) -> (u64, u64) {
        self.s.dispatch(Completion::Semantic(code_core::semantics::Event::Tick));
        self.settle("memory", Duration::from_secs(10), |s| s.state().provider().is_some_and(|v| v.memory.is_some()));
        self.s.state().provider().unwrap().memory.unwrap()
    }
}

fn file_of(s: &Session) -> String {
    s.state().current_source().unwrap().source().file_name().unwrap().to_string_lossy().into_owned()
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
        f.settle("candidate", Duration::from_secs(10), |s| s.state().loading().is_none() && s.state().current_source().is_some_and(|d| d.source().to_string_lossy().contains("typeshed")));
        f.s.dispatch(Command::CloseResults);
    }
    println!("join: {:?} {:?}", f.s.state().current_source().unwrap().source(), f.notices());
    assert!(f.s.state().current_source().unwrap().source().to_string_lossy().contains("typeshed"), "{:?}", f.s.state().current_source().unwrap().source());
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
    f.settle("refresh", Duration::from_secs(10), |s| s.state().current_source().is_some_and(|d| d.copy_range(0..d.len_bytes()).unwrap().starts_with("# moved")));
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

/// A virtual environment made at `dir` by the installed `python3 -m venv`, with package `name` (`body` its
/// `__init__.py`) installed into it by the environment's own pip, offline, from a wheel written here.
fn python_venv(dir: &Path, name: &str, body: &str) {
    let made = std::process::Command::new("python3").args(["-m", "venv"]).arg(dir).output().expect("python3 is installed");
    assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
    let wheels = tempfile::tempdir().unwrap();
    let wheel = wheels.path().join(format!("{name}-1.0-py3-none-any.whl"));
    const WHEEL: &str = "import base64, hashlib, sys, zipfile\nname, body, out = sys.argv[1:4]\ninfo = f'{name}-1.0.dist-info'\nfiles = {f'{name}/__init__.py': body, f'{name}/py.typed': '', f'{info}/METADATA': f'Metadata-Version: 2.1\\nName: {name}\\nVersion: 1.0\\n', f'{info}/WHEEL': 'Wheel-Version: 1.0\\nGenerator: lenz-test\\nRoot-Is-Purelib: true\\nTag: py3-none-any\\n'}\nrecord = []\nwith zipfile.ZipFile(out, 'w') as z:\n    for path, text in files.items():\n        data = text.encode()\n        z.writestr(path, data)\n        digest = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b'=').decode()\n        record.append(f'{path},sha256={digest},{len(data)}')\n    record.append(f'{info}/RECORD,,')\n    z.writestr(f'{info}/RECORD', '\\n'.join(record) + '\\n')\n";
    let built = std::process::Command::new("python3").args(["-I", "-c", WHEEL, name, body]).arg(&wheel).output().unwrap();
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let pip = std::process::Command::new(dir.join("bin/python")).args(["-m", "pip", "install", "--no-index", "--no-deps", "--disable-pip-version-check", "-q"]).arg(&wheel).output().unwrap();
    assert!(pip.status.success(), "{}", String::from_utf8_lossy(&pip.stderr));
}

/// The `site-packages` folder of the virtual environment at `dir`.
fn site_packages(dir: &Path) -> PathBuf {
    let lib = std::fs::read_dir(dir.join("lib")).unwrap().filter_map(Result::ok).map(|e| e.path()).find(|p| p.file_name().unwrap().to_string_lossy().starts_with("python3")).unwrap();
    lib.join("site-packages")
}

/// The Python version a virtual environment records.
fn venv_version(dir: &Path) -> String {
    let cfg = std::fs::read_to_string(dir.join("pyvenv.cfg")).unwrap();
    cfg.lines().find_map(|l| l.strip_prefix("version = ").or_else(|| l.strip_prefix("version_info = "))).unwrap().trim().to_owned()
}

/// `dir/bin` before the `PATH` Pyright would otherwise run with: an "activated" environment other than the project's.
fn activated(dir: &Path, path: Option<&str>) -> String {
    let rest = path.map_or_else(|| std::env::var("PATH").unwrap_or_default(), str::to_owned);
    format!("{}:{rest}", dir.join("bin").display())
}

const PY_SHAPES: &str = "def area(radius: float) -> float:\n    \"\"\"The area of a circle of this radius.\"\"\"\n    return 3.14159 * radius * radius\n\n\nclass Circle:\n    def __init__(self, radius: float) -> None:\n        self.radius = radius\n";
const PY_ENV_APP: &str = "import sys\nimport os\nimport shapes_dep\nimport other_dep\n\nr = shapes_dep.area(2.0)\nprint(r, os.sep, sys.argv, other_dep)\ns  =  \"𝄞\"\n";
const PY_ENV_PYPROJECT: &str = "[project]\nname = \"envfix\"\nversion = \"0\"\n\n[tool.ruff]\nline-length = 60\n\n[tool.ruff.format]\nquote-style = \"single\"\n";

/// The Python fixture: a project whose `pyrightconfig.json` names `venv` (`env` is made, holding `shapes_dep`, installed
/// by pip; `None`: no configuration) and another environment (`other_dep`, outside the project) first on Pyright's
/// PATH, as an activated one would be.
fn python_env_fixture(exe: &Path, path: Option<&str>, venv: Option<&str>, env_name: &str) -> (Real, PathBuf, PathBuf, tempfile::TempDir) {
    let outside = tempfile::Builder::new().prefix("reader-other-").tempdir().unwrap();
    let other = outside.path().join("other");
    python_venv(&other, "other_dep", "def other() -> int:\n    return 1\n");
    let config = venv.map(|v| format!("{{ \"venvPath\": \".\", \"venv\": \"{v}\" }}\n"));
    let mut files = vec![("pyproject.toml", PY_ENV_PYPROJECT), ("app.py", PY_ENV_APP)];
    if let Some(config) = &config { files.push(("pyrightconfig.json", config)); }
    let f = real("pyright", exe, Some(&activated(&other, path)), &files);
    let env = f.root.join(env_name);
    python_venv(&env, "shapes_dep", PY_SHAPES);
    (f, env, other, outside)
}

/// Pyright in a project whose configuration names its own virtual environment while another one is activated (first on
/// PATH): Server Details names the configured environment, its version and packages; the dependency installed in it
/// completes, has parameter information and a definition in its `site-packages`, and the other environment's package
/// is reported unresolved. Format Document and Organize Imports run the project's ruff configuration (ruff from PATH:
/// the environment has none), each one Undo step, and a format answered after an edit is refused. The files on disk
/// never change. Needs Pyright, python3 with venv and pip, and ruff.
#[test]
#[ignore = "needs an installed Pyright, python3 with venv and pip, and ruff; run alone"]
fn real_pyright_configured_environment_editing() {
    let Some((exe, path)) = installed("pyright", "PYRIGHT") else {
        println!("SKIPPED: Pyright is not installed (set READER_PYRIGHT / READER_PYRIGHT_PATH)");
        return;
    };
    let (mut f, env, _other, _outside) = python_env_fixture(&exe, path.as_deref(), Some("env"), "env");
    f.open_ready("app.py");
    let tab = f.tab();
    let view = f.s.state().provider().unwrap();
    println!("Pyright {:?}; {:?}", view.version, view.environment);
    let config = f.root.join("pyrightconfig.json");
    let expected = format!("Python environment: {} (venv \"env\" with venvPath \".\" in {}), Python {}; installed packages from {}", env.display(), config.display(), venv_version(&env), site_packages(&env).display());
    assert_eq!(view.environment, Some(Ok(expected.clone())));
    assert!(f.s.language_server_details().contains(&format!("\n\n{expected}\n\n")));
    // The dependency of the configured environment resolves; the activated environment's does not.
    f.settle("problems", Duration::from_secs(60), |s| s.state().active().and_then(|t| s.state().diagnostics(t.id())).is_some_and(|d| d.items.iter().any(|p| p.message.contains("other_dep"))));
    let problems = f.problems();
    println!("problems: {problems:?}");
    assert!(problems.iter().any(|p| p.contains("Import \"other_dep\" could not be resolved")), "{problems:?}");
    assert!(!problems.iter().any(|p| p.contains("shapes_dep")), "{problems:?}");
    // Completion of the dependency's members, as one Undo step with the typed name.
    f.caret("s  =", 0);
    f.type_at_caret("shapes_dep.ar\n");
    f.caret("shapes_dep.ar\n", 13);
    let typed = current_text(f.s.state());
    let row = f.complete_until("area");
    let members: Vec<String> = f.s.state().completion().map(|c| c.rows().map(|r| r.label.clone()).collect()).unwrap_or_default();
    println!("members: {members:?}");
    f.s.dispatch(Command::SelectCompletion(row));
    f.s.dispatch(Command::AcceptCompletion);
    f.settle("accepted", Duration::from_secs(30), |s| current_text(s.state()).contains("shapes_dep.area\n"));
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), typed);
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), PY_ENV_APP);
    // Parameter information from the installed package.
    f.caret("area(2.0)", 5);
    let seen = f.notices().len();
    f.s.dispatch(Command::ParameterInfo);
    f.settle("signature", Duration::from_secs(30), move |s| s.state().signature_help().is_some() || s.state().notices().len() > seen);
    let sig = f.s.state().signature_help().and_then(|h| h.active_signature()).map(|s| (s.label.clone(), s.active_parameter));
    println!("signature: {sig:?}");
    assert_eq!(sig, Some(("(radius: float) -> float".to_owned(), Some(0))));
    f.s.dispatch(Command::CloseSignature);
    // Its definition is in the configured environment's site-packages, shown read-only with its origin.
    f.definition_into("area(2.0)", 1, "site-packages/shapes_dep");
    let shown = f.s.state().current_source().unwrap().source().to_owned();
    assert_eq!(shown, site_packages(&env).join("shapes_dep/__init__.py"));
    assert!(f.notices().iter().any(|n| n == "Read-only source from an installed Python package"), "{:?}", f.notices());
    f.back("app.py");
    // Quick fixes: Pyright offers none.
    f.caret("other_dep\n", 2);
    let seen = f.notices().len();
    f.s.dispatch(Command::QuickFix);
    f.settle("quick fixes", Duration::from_secs(30), move |s| s.state().code_actions().is_some() || s.state().notices().len() > seen);
    println!("quick fixes: {:?} {:?}", f.s.state().code_actions().map(|c| c.actions.len()), f.notices().last());
    assert!(f.s.state().code_actions().is_none_or(|c| c.actions.is_empty()), "{:?}", f.notices());
    // ruff formats with the project's configuration (single quotes), as one Undo step.
    f.s.dispatch(Command::Format(code_core::session::FormatKind::Format));
    f.settle("formatted", Duration::from_secs(30), |s| s.state().asking().is_none());
    let formatted = current_text(f.s.state());
    println!("formatted:\n{formatted}\n{:?}", f.notices().last());
    assert_eq!(formatted, PY_ENV_APP.replace("s  =  \"𝄞\"", "s = '𝄞'"));
    assert!(f.notices().iter().any(|n| n == "Format Document: done with ruff"), "{:?}", f.notices());
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), PY_ENV_APP);
    // ruff's import sorting: the standard library first, the dependencies after.
    f.s.dispatch(Command::Format(code_core::session::FormatKind::OrganizeImports));
    f.settle("organized", Duration::from_secs(30), |s| s.state().asking().is_none());
    let organized = current_text(f.s.state());
    println!("organized:\n{organized}");
    assert!(organized.starts_with("import os\nimport sys\n\nimport other_dep\nimport shapes_dep\n"), "{organized}");
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), PY_ENV_APP);
    // An edit supersedes formatting, whether it reaches the tool before the edit or is cancelled before launch.
    f.s.dispatch(Command::Format(code_core::session::FormatKind::Format));
    f.caret("r = shapes", 0);
    f.type_at_caret("# x\n");
    f.settle("stale format", Duration::from_secs(30), |s| s.state().asking().is_none());
    assert_eq!(current_text(f.s.state()), PY_ENV_APP.replacen("r = shapes", "# x\nr = shapes", 1), "{:?}", f.notices());
    assert!(f.notices().iter().any(|n| n == "Format Document: the file changed while ruff ran"
        || n == "Format Document: the file changed before the project's formatter could run"), "{:?}", f.notices());
    for (name, text) in [("app.py", PY_ENV_APP), ("pyproject.toml", PY_ENV_PYPROJECT)] { assert_eq!(std::fs::read_to_string(f.root.join(name)).unwrap(), text); }
    assert!(!f.root.join(".ruff_cache").exists());
}

/// The configured environment is missing: Pyright is not started (with the activated environment, or any other), the
/// status says why and what to do, editing goes on, and ruff still formats. Needs Pyright and ruff.
#[test]
#[ignore = "needs an installed Pyright, python3 with venv and pip, and ruff; run alone"]
fn real_pyright_missing_environment_is_not_replaced() {
    let Some((exe, path)) = installed("pyright", "PYRIGHT") else {
        println!("SKIPPED: Pyright is not installed (set READER_PYRIGHT / READER_PYRIGHT_PATH)");
        return;
    };
    let (mut f, _env, _other, _outside) = python_env_fixture(&exe, path.as_deref(), Some("gone"), "env");
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.s.dispatch(Command::Open(f.root.join("app.py")));
    f.settle("stopped", Duration::from_secs(30), |s| s.state().provider().is_some_and(|v| matches!(v.status, Status::Failed(_))));
    let gone = f.root.join("gone");
    let why = format!("The configured Python environment {} does not exist (venv \"gone\" with venvPath \".\" in {}). Lenz does not start Pyright with another environment: create it (for example `python3 -m venv {}`) or correct the setting, then choose Restart Language Server", gone.display(), f.root.join("pyrightconfig.json").display(), gone.display());
    assert_eq!(f.s.state().provider().unwrap().status, Status::Failed(why));
    let details = f.s.language_server_details();
    println!("details:\n{details}");
    assert!(details.contains("\n\nThe language server was not started: correct the project environment named above") && !details.contains("Setup: "), "{details}");
    assert!(f.s.state().servers().iter().all(|v| v.memory.is_none()), "no server process runs");
    let tab = f.tab();
    f.caret("r = shapes", 0);
    f.type_at_caret("# typed\n");
    assert_eq!(current_text(f.s.state()), PY_ENV_APP.replacen("r = shapes", "# typed\nr = shapes", 1));
    f.s.dispatch(Command::Format(code_core::session::FormatKind::Format));
    f.settle("formatted", Duration::from_secs(30), |s| s.state().asking().is_none());
    assert!(current_text(f.s.state()).ends_with("s = '𝄞'\n"), "{:?}", f.notices());
    f.s.dispatch(Command::Undo { tab });
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), PY_ENV_APP);
    assert_eq!(std::fs::read_to_string(f.root.join("app.py")).unwrap(), PY_ENV_APP);
}

/// No environment configured, a project `.venv` (holding `shapes_dep`) and another environment first on PATH: Pyright
/// uses the PATH one, as it says; Server Details names it and the unconfigured `.venv` it does not use, and the
/// dependencies resolve accordingly. Needs Pyright and python3 with venv and pip.
#[test]
#[ignore = "needs an installed Pyright and python3 with venv and pip; run alone"]
fn real_pyright_without_configuration_says_which_environment_it_uses() {
    let Some((exe, path)) = installed("pyright", "PYRIGHT") else {
        println!("SKIPPED: Pyright is not installed (set READER_PYRIGHT / READER_PYRIGHT_PATH)");
        return;
    };
    let (mut f, venv, other, _outside) = python_env_fixture(&exe, path.as_deref(), None, ".venv");
    f.open_ready("app.py");
    let line = f.s.state().provider().unwrap().environment.clone().and_then(Result::ok).unwrap_or_default();
    println!("{line}");
    let python = other.join("bin/python3");
    let version = std::process::Command::new(&python).arg("--version").output().unwrap();
    let version = String::from_utf8_lossy(&version.stdout).trim().to_owned();
    assert_eq!(line, format!("Python environment: none configured, so Pyright uses the first python3 (else python) on its PATH: {} ({version}). {} holds a virtual environment that is not configured, so Pyright does not use it. To use a project environment, set venvPath and venv in pyrightconfig.json or [tool.pyright] in pyproject.toml, or python.pythonPath in Lenz's provider settings", python.display(), venv.display()));
    f.settle("problems", Duration::from_secs(60), |s| s.state().active().and_then(|t| s.state().diagnostics(t.id())).is_some_and(|d| d.items.iter().any(|p| p.message.contains("shapes_dep"))));
    let problems = f.problems();
    println!("problems: {problems:?}");
    assert!(problems.iter().any(|p| p.contains("Import \"shapes_dep\" could not be resolved")), "{problems:?}");
    assert!(!problems.iter().any(|p| p.contains("other_dep")), "{problems:?}");
}

/// black configured instead of ruff, with Pyright missing: Format Document runs black from PATH with the project's
/// line length as one Undo step, Organize Imports says black does not organize imports, and the file on disk never
/// changes. Needs black on PATH.
#[test]
#[ignore = "needs black on PATH; run alone"]
fn real_black_formats_python_without_a_language_server() {
    if code_core::semantics::provider::find_executable(Path::new("black"), None).is_none() {
        println!("SKIPPED: black is not installed");
        return;
    }
    let text = "def f(alpha, beta, gamma):\n  return [alpha, beta, gamma, 'delta']\n";
    let mut f = real("pyright", Path::new("/nonexistent/pyright-langserver"), None, &[("pyproject.toml", "[tool.black]\nline-length = 30\n"), ("app.py", text)]);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.show("app.py");
    f.settle("missing", Duration::from_secs(30), |s| s.state().provider().is_some_and(|v| matches!(v.status, Status::Missing(_))));
    let tab = f.tab();
    f.s.dispatch(Command::Format(code_core::session::FormatKind::Format));
    f.settle("formatted", Duration::from_secs(30), |s| s.state().asking().is_none());
    let formatted = current_text(f.s.state());
    println!("formatted:\n{formatted}\n{:?}", f.notices());
    assert_eq!(formatted, "def f(alpha, beta, gamma):\n    return [\n        alpha,\n        beta,\n        gamma,\n        \"delta\",\n    ]\n");
    assert!(f.notices().iter().any(|n| n == "Format Document: done with black"), "{:?}", f.notices());
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), text);
    f.s.dispatch(Command::Format(code_core::session::FormatKind::OrganizeImports));
    f.settle("organize", Duration::from_secs(30), |s| s.state().asking().is_none());
    assert!(f.notices().iter().any(|n| n == "Organize Imports: black does not organize imports; Pyright offers no organize imports either (configure ruff for this project to use it)"), "{:?}", f.notices());
    assert_eq!(std::fs::read_to_string(f.root.join("app.py")).unwrap(), text);
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
        f.settle("candidate", Duration::from_secs(10), |s| s.state().loading().is_none() && s.state().current_source().is_some_and(|d| d.source().to_string_lossy().contains("lib.")));
        f.s.dispatch(Command::CloseResults);
    }
    println!("Math.max: {:?} {:?}", f.s.state().current_source().unwrap().source(), f.notices());
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
    f.settle("refresh", Duration::from_secs(10), |s| s.state().current_source().is_some_and(|d| d.copy_range(0..d.len_bytes()).unwrap().starts_with("// moved")));
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
    let version = code_core::semantics::provider::package_version(&exe, "typescript").unwrap_or_default();
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
    println!("Math.max: {:?} {:?}", f.s.state().current_source().map(|d| d.source().to_owned()), f.notices());
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
    f.settle("refresh", Duration::from_secs(10), |s| s.state().current_source().is_some_and(|d| d.copy_range(0..d.len_bytes()).unwrap().starts_with("// moved")));
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
    if !code_core::semantics::provider::package_version(&tsc, "typescript").is_some_and(|v| v.starts_with('7')) {
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
    // The server ignores an invalid configured path when its own installation has a usable fallback.
    // Copy its self-contained CLI into an installation without TypeScript, with only Node on PATH.
    let isolated = tempfile::tempdir().unwrap();
    let original = std::fs::canonicalize(&exe).unwrap();
    let package = original.parent().unwrap().parent().unwrap();
    let lib = isolated.path().join("node_modules/typescript-language-server/lib");
    std::fs::create_dir_all(&lib).unwrap();
    let isolated_exe = lib.join("cli.mjs");
    std::fs::copy(&original, &isolated_exe).unwrap();
    std::fs::copy(package.join("package.json"), lib.parent().unwrap().join("package.json")).unwrap();
    let bin = isolated.path().join("bin"); std::fs::create_dir(&bin).unwrap();
    let node = code_core::semantics::provider::find_executable(Path::new("node"), path.as_deref().map(std::ffi::OsStr::new)).expect("the installed server needs Node");
    std::os::unix::fs::symlink(std::fs::canonicalize(node).unwrap(), bin.join("node")).unwrap();
    let mut f = real_with("typescript-language-server", &isolated_exe, bin.to_str(), json!({ "tsserver": { "path": "/nonexistent/tsserver.js" } }), &TS_FILES);
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
    // JDT LS returns the declaration span for decompiled JDK code when no JDK source archive is installed.
    assert!(matches!(f.selected().as_str(), "println" | "public void println(String x) {"), "{}", f.selected());
    assert!(current_text(f.s.state()).contains("class PrintStream"));
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
    f.settle("refresh", Duration::from_secs(10), |s| s.state().current_source().is_some_and(|d| d.copy_range(0..d.len_bytes()).unwrap().starts_with("// moved")));
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

const MAVEN_POM: &str = "<project xmlns=\"http://maven.apache.org/POM/4.0.0\">\n  <modelVersion>4.0.0</modelVersion>\n  <groupId>demo</groupId>\n  <artifactId>demo</artifactId>\n  <version>1</version>\n  <properties>\n    <maven.compiler.release>21</maven.compiler.release>\n    <project.build.sourceEncoding>UTF-8</project.build.sourceEncoding>\n  </properties>\n  <dependencies>\n    <dependency>\n      <groupId>com.google.code.gson</groupId>\n      <artifactId>gson</artifactId>\n      <version>2.11.0</version>\n    </dependency>\n  </dependencies>\n</project>\n";
const MAVEN_APP: &str = "package demo;\n\npublic class App {\n    public static void main(String[] args) {\n        Gson g = new Gson();\n            String s = g.toJson(args);\n        System.out.println(s);\n    }\n}\n";

impl Real {
    fn tab(&self) -> code_core::tabs::TabId { self.s.state().active().unwrap().id() }
    fn problems(&self) -> Vec<String> {
        self.s.state().diagnostics(self.tab()).map(|d| d.items.iter().map(|p| format!("{}: {}", p.severity.label(), p.message)).collect()).unwrap_or_default()
    }
    fn type_at_caret(&mut self, text: &str) {
        self.s.dispatch(Command::InsertText { text: text.into(), kind: code_core::text::EditKind::Typing });
    }
    /// Ask for completion at the caret until a suggestion labelled `label` can be accepted: its row.
    fn complete_until(&mut self, label: &str) -> usize {
        let asked = Instant::now();
        loop {
            let seen = self.notices().len();
            self.s.dispatch(Command::Complete);
            self.settle("completion", Duration::from_secs(60), move |s| s.state().completion().is_some() || s.state().notices().len() > seen);
            if let Some(row) = self.s.state().completion().and_then(|c| c.rows().position(|r| r.label == label && r.unavailable.is_none())) { return row; }
            let rows: Option<Vec<String>> = self.s.state().completion().map(|c| c.rows().take(8).map(|r| format!("{} {:?}", r.label, r.unavailable)).collect());
            println!("completion for {label} at {:.1} s: {rows:?}; {:?}", asked.elapsed().as_secs_f64(), self.notices().last());
            self.s.dispatch(Command::CloseCompletion);
            assert!(asked.elapsed() < Duration::from_secs(30), "{label} was never offered");
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

/// jdtls editing a Maven project that depends on Gson: the dependency is discovered (an unresolved `Gson` gets an
/// import fix from the dependency), and completion with its import, parameter information, quick fixes, formatting
/// and organize imports each apply as one undoable edit of the draft, never of the file; a format answered after an
/// edit is refused.
#[test]
#[ignore = "needs an installed jdtls, Java 21+ and network access to Maven Central for the fixture's dependency; run in release, alone"]
fn real_jdtls_maven_editing() {
    let Some((exe, path)) = installed("jdtls", "JDTLS") else {
        println!("SKIPPED: jdtls is not installed");
        return;
    };
    let mut f = real("jdtls", &exe, path.as_deref(), &[("pom.xml", MAVEN_POM), ("src/main/java/demo/App.java", MAVEN_APP)]);
    let started = Instant::now();
    f.open_ready("src/main/java/demo/App.java");
    println!("JDT LS {:?} ready in {:.1} s", f.s.state().provider().unwrap().version, started.elapsed().as_secs_f64());
    // Problems for the shown version: `Gson` is unresolved until it is imported.
    f.settle("problems", Duration::from_secs(180), |s| s.state().active().and_then(|t| s.state().diagnostics(t.id())).is_some_and(|d| d.items.iter().any(|p| p.message.contains("Gson"))));
    println!("problems: {:?}", f.problems());
    // The import fix comes from the Maven dependency: proof the project and its classpath were discovered.
    f.caret("Gson g", 1);
    let seen = f.notices().len();
    f.s.dispatch(Command::QuickFix);
    f.settle("quick fixes", Duration::from_secs(60), move |s| s.state().code_actions().is_some() || s.state().notices().len() > seen);
    let titles: Vec<String> = f.s.state().code_actions().map(|c| c.actions.iter().map(|a| format!("{} [{}]", a.title, a.unavailable.as_deref().unwrap_or("ok"))).collect()).unwrap_or_default();
    println!("quick fixes at Gson: {titles:?}; notices {:?}", f.notices());
    let import = f.s.state().code_actions().unwrap().actions.iter().position(|a| a.title.contains("com.google.gson") && a.unavailable.is_none()).unwrap_or_else(|| panic!("{titles:?}"));
    f.s.dispatch(Command::SelectCodeAction(import));
    f.s.dispatch(Command::ApplyCodeAction);
    let imported = current_text(f.s.state());
    assert!(imported.contains("import com.google.gson.Gson;"), "{imported}");
    assert_eq!(imported.replace("import com.google.gson.Gson;\n", "").replace("\n\n\n", "\n\n"), MAVEN_APP, "only the import line was added");
    let tab = f.tab();
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), MAVEN_APP, "one Undo takes the fix back");
    f.s.dispatch(Command::Redo { tab });
    assert_eq!(current_text(f.s.state()), imported);
    f.settle("Gson resolved", Duration::from_secs(60), |s| s.state().active().and_then(|t| s.state().diagnostics(t.id())).is_some_and(|d| !d.items.iter().any(|p| p.message.contains("Gson cannot be resolved"))));
    // Completion with an import: `ArrayL` completes to `ArrayList`, adding `import java.util.ArrayList;`.
    f.caret("        System.out", 8);
    f.type_at_caret("var names = new ArrayL\n        ");
    f.caret("ArrayL\n", 6);
    let seen = f.notices().len();
    f.s.dispatch(Command::Complete);
    f.settle("completion", Duration::from_secs(60), move |s| s.state().completion().is_some_and(|c| c.rows().any(|r| r.label.starts_with("ArrayList"))) || s.state().notices().len() > seen);
    let labels: Vec<String> = f.s.state().completion().map(|c| c.rows().take(8).map(|r| r.label.clone()).collect()).unwrap_or_default();
    println!("completion: {labels:?}; notices {:?}", f.notices());
    let row = f.s.state().completion().unwrap().rows().position(|r| r.label.starts_with("ArrayList") && r.detail.as_deref().is_some_and(|d| d.contains("java.util"))).unwrap_or_else(|| panic!("{labels:?}"));
    f.s.dispatch(Command::SelectCompletion(row));
    f.s.dispatch(Command::AcceptCompletion);
    f.settle("accepted", Duration::from_secs(30), |s| current_text(s.state()).contains("import java.util.ArrayList;"));
    let completed = current_text(f.s.state());
    println!("after completion:\n{completed}");
    assert!(completed.contains("new ArrayList"), "{completed}");
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), imported.replacen("        System.out", "        var names = new ArrayL\n        System.out", 1), "one Undo takes the name and its import back");
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), imported);
    // Parameter information inside `g.toJson(`.
    f.caret("toJson(args", 7);
    let seen = f.notices().len();
    f.s.dispatch(Command::ParameterInfo);
    f.settle("signature", Duration::from_secs(30), move |s| s.state().signature_help().is_some() || s.state().notices().len() > seen);
    let signatures: Vec<String> = f.s.state().signature_help().map(|h| h.signatures.iter().map(|s| s.label.clone()).collect()).unwrap_or_default();
    println!("signatures: {signatures:?}; notices {:?}", f.notices());
    assert!(signatures.iter().any(|l| l.contains("toJson(Object")), "{signatures:?}");
    f.s.dispatch(Command::CloseSignature);
    // Formatting re-indents the misplaced line as one Undo step.
    f.s.dispatch(Command::Format(code_core::session::FormatKind::Format));
    f.settle("formatted", Duration::from_secs(60), |s| s.state().asking().is_none());
    let formatted = current_text(f.s.state());
    println!("formatted:\n{formatted}; notices {:?}", f.notices());
    assert!(formatted.contains("\n        String s = g.toJson(args);"), "{formatted}");
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), imported);
    // A format answered after an edit is refused; the draft keeps only the edit.
    f.caret("public class", 0);
    f.s.dispatch(Command::Format(code_core::session::FormatKind::Format));
    f.type_at_caret("final ");
    f.settle("stale format", Duration::from_secs(60), |s| s.state().asking().is_none());
    let typed = imported.replacen("public class", "final public class", 1);
    assert_eq!(current_text(f.s.state()), typed, "{:?}", f.notices());
    // Organize imports removes the unused import it is given.
    f.caret("import com.google.gson.Gson;", 0);
    f.type_at_caret("import java.util.Map;\n");
    f.s.dispatch(Command::Format(code_core::session::FormatKind::OrganizeImports));
    f.settle("organized", Duration::from_secs(60), |s| s.state().asking().is_none());
    let organized = current_text(f.s.state());
    println!("organized:\n{organized}; notices {:?}", f.notices());
    assert!(!organized.contains("import java.util.Map;") && organized.contains("import com.google.gson.Gson;"), "{organized}");
    assert_eq!(std::fs::read_to_string(f.root.join("src/main/java/demo/App.java")).unwrap(), MAVEN_APP, "every edit stayed in the draft");
    println!("capabilities: {:?}", f.s.state().provider().unwrap().supports);
}

const GRADLE_BUILD: &str = "plugins {\n    id 'java'\n}\n\nrepositories {\n    mavenCentral()\n}\n\ndependencies {\n    implementation 'com.google.code.gson:gson:2.11.0'\n}\n";

/// jdtls on a Gradle project depending on Gson: the build is imported (with the installed Gradle when one is on
/// `PATH`, else the one Buildship downloads), the dependency's import fix applies as one undoable edit, and
/// formatting follows.
#[test]
#[ignore = "needs an installed jdtls, Java 21+, Gradle or network access, and Maven Central for the dependency; run in release, alone"]
fn real_jdtls_gradle_editing() {
    let Some((exe, path)) = installed("jdtls", "JDTLS") else {
        println!("SKIPPED: jdtls is not installed");
        return;
    };
    // An installed Gradle, when there is one: `GRADLE_HOME`, else the one `gradle` on PATH belongs to.
    let gradle = std::env::var_os("GRADLE_HOME").map(PathBuf::from)
        .or_else(|| code_core::semantics::provider::find_executable(Path::new("gradle"), None).and_then(|g| std::fs::canonicalize(g).ok()).and_then(|g| Some(g.parent()?.parent()?.to_owned())));
    let settings = gradle.as_ref().map_or(Value::Null, |home| json!({ "settings": { "java": { "import": { "gradle": { "home": home } } } } }));
    let mut f = real_with("jdtls", &exe, path.as_deref(), settings, &[("build.gradle", GRADLE_BUILD), ("settings.gradle", "rootProject.name = 'demo'\n"), ("src/main/java/demo/App.java", MAVEN_APP)]);
    let started = Instant::now();
    f.open_ready("src/main/java/demo/App.java");
    println!("JDT LS ready in {:.1} s with Gradle {:?}", started.elapsed().as_secs_f64(), gradle);
    f.settle("problems", Duration::from_secs(300), |s| s.state().active().and_then(|t| s.state().diagnostics(t.id())).is_some_and(|d| d.items.iter().any(|p| p.message.contains("Gson"))));
    println!("problems: {:?}", f.problems());
    // The import fix from the Gradle dependency: retried while the build is still being imported.
    let asked = Instant::now();
    let import = loop {
        f.caret("Gson g", 1);
        let seen = f.notices().len();
        f.s.dispatch(Command::QuickFix);
        f.settle("quick fixes", Duration::from_secs(60), move |s| s.state().code_actions().is_some() || s.state().notices().len() > seen);
        let found = f.s.state().code_actions().and_then(|c| c.actions.iter().position(|a| a.title.contains("com.google.gson") && a.unavailable.is_none()));
        if let Some(i) = found { break i; }
        println!("no import fix yet: {:?}", f.s.state().code_actions().map(|c| c.actions.iter().map(|a| a.title.clone()).collect::<Vec<_>>()));
        f.s.dispatch(Command::CloseCodeActions);
        assert!(asked.elapsed() < Duration::from_secs(240), "the Gradle dependency was never offered: {:?}", f.notices());
        std::thread::sleep(Duration::from_secs(2));
    };
    println!("import fix found after {:.1} s", asked.elapsed().as_secs_f64());
    f.s.dispatch(Command::SelectCodeAction(import));
    f.s.dispatch(Command::ApplyCodeAction);
    let imported = current_text(f.s.state());
    assert!(imported.contains("import com.google.gson.Gson;"), "{imported}");
    let tab = f.tab();
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), MAVEN_APP);
    f.s.dispatch(Command::Redo { tab });
    f.s.dispatch(Command::Format(code_core::session::FormatKind::Format));
    f.settle("formatted", Duration::from_secs(60), |s| s.state().asking().is_none());
    assert!(current_text(f.s.state()).contains("\n        String s = g.toJson(args);"), "{:?}", f.notices());
    assert_eq!(std::fs::read_to_string(f.root.join("src/main/java/demo/App.java")).unwrap(), MAVEN_APP);
    assert_eq!(std::fs::read_to_string(f.root.join("build.gradle")).unwrap(), GRADLE_BUILD, "the build file is never edited");
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
    let python = code_core::semantics::provider::find_executable(Path::new("python3"), None).unwrap();
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
    let on_path = code_core::semantics::provider::find_executable(Path::new(p.executable), path.as_deref().map(std::ffi::OsStr::new)).is_some();
    let bundled = code_core::semantics::provider::bundled_server(&code_core::semantics::provider::VSCODE_EXTENSIONS, p.bundled?).is_some()
        && code_core::semantics::provider::find_executable(Path::new("node"), path.as_deref().map(std::ffi::OsStr::new)).is_some();
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
    assert_eq!(f.s.state().current_source().unwrap().line_of(f.s.state().selection().unwrap().head), 5);
    f.usages("--accent:", 3);
    // The declaration comes too (the server ignores `includeDeclaration: false`).
    assert_eq!(places(&f.s), [("style.css".into(), 1), ("style.css".into(), 4), ("style.css".into(), 6)]);
    f.s.dispatch(Command::FileSymbols);
    f.settle("file symbols", Duration::from_secs(10), |s| s.state().asking().is_none());
    assert!(f.s.state().results().is_some_and(|r| r.hits().len() >= 3));
    // No workspace symbols: said so.
    assert_eq!(f.s.disabled(code_core::session::CommandId::Symbols), Some("The language servers running for this folder do not provide workspace symbols; find declarations instead"));
    // A selector is not a definition the server can follow (no text match stands in).
    f.definition(".card .title", 2);
    assert!(f.notices().iter().any(|n| n.contains("VS Code CSS found no definition")), "{:?}", f.notices());
    // An external edit: the new text is what it answers about.
    std::fs::write(f.root.join("site/style.css"), format!("/* moved */\n\n{CSS}")).unwrap();
    f.settle("refresh", Duration::from_secs(10), |s| s.state().current_source().is_some_and(|d| d.copy_range(0..d.len_bytes()).unwrap().starts_with("/* moved")));
    f.definition("--accent);", 3);
    assert_eq!(f.s.state().current_source().unwrap().line_of(f.s.state().selection().unwrap().head), 3);
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
    assert_eq!(f.s.state().current_source().unwrap().line_of(f.s.state().selection().unwrap().head), 5);
    f.usages("greet(name)", 1);
    // The declaration and both calls (the server ignores `includeDeclaration: false`).
    assert_eq!(places(&f.s), [("index.html".into(), 5), ("index.html".into(), 6), ("index.html".into(), 6)]);
    // A custom property of the page's own style.
    f.definition("var(--gap)", 5);
    assert_eq!(f.selected(), "--gap");
    // A class is not followed to the stylesheet (no text match stands in).
    f.definition("card local", 1);
    assert!(f.notices().iter().any(|n| n.contains("VS Code HTML found no definition")), "{:?}", f.notices());
    assert_eq!(f.s.disabled(code_core::session::CommandId::Symbols), Some("The language servers running for this folder do not provide workspace symbols; find declarations instead"));
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

/// A page for editing: a stylesheet and a script of its own with one problem each (after a non-BMP character on the
/// stylesheet's line), a list whose second item waits for its closing tag, and an unclosed `<section>`.
const HTML_EDIT: &str = "<!doctype html>\n<html>\n<head>\n<style>/* 𝄞 */ .card { colr: red; }\n.list { }\n</style>\n</head>\n<body>\n<ul class=\"list\">\n<li>𝄞 one</li>\n<li>two</\n</ul>\n<p>𝄞 x</p>\n<section>𝄞 open\n<script>\nfunction greet(name, punct) { return \"hi \" + name + punct; }\nlet a = ;\n</script>\n</body>\n</html>\n";

/// [`HTML_EDIT`] as VS Code's HTML server 4.10.0 formats it with three-space indentation and `indentInnerHtml`.
const HTML_FORMATTED: &str = "<!doctype html>\n<html>\n   <head>\n      <style>\n         /* 𝄞 */\n         .card {\n            colr: red;\n         }\n\n         .list {}\n      </style>\n   </head>\n   <body>\n      <ul class=\"list\">\n         <li>𝄞 one</li>\n         <li>two</ </ul>\n            <p>𝄞 x</p>\n            <section>𝄞 open\n               <script>\n                  function greet(name, punct) {return \"hi \" + name + punct;}\n                  let a = ;\n               </script>\n   </body>\n</html>";

/// VS Code's HTML server editing a page (draft only): problems of its <style> and <script> at their exact
/// original-source ranges; a tag name completed after a non-BMP character (UTF-16 ranges) and closed when its `>`
/// is typed, a closing tag completed after `</` and one closed when `</` is typed, an attribute snippet with the
/// caret between its quotes, a CSS property in <style> with its value suggestions, a JavaScript function in
/// <script> with parameter information, each one Undo step; Format Document under the page's `.editorconfig` and
/// the provider's `format` settings, embedded CSS and JavaScript included; the missing quick fixes and organize
/// imports explained. The files on disk never change.
#[test]
#[ignore = "needs VS Code (or vscode-langservers-extracted) and Node.js; run alone"]
fn real_html_editing() {
    let Some(path) = web_server("vscode-html") else {
        println!("SKIPPED: no VS Code HTML server");
        return;
    };
    let page = [("site/.editorconfig", "root = true\n\n[*.html]\nindent_style = space\nindent_size = 3\n"), ("site/index.html", HTML_EDIT)];
    // The page's own `format` settings, and its stylesheet's lint settings (the `css` section it reads).
    let settings = json!({ "format": { "indentInnerHtml": true, "extraLiners": "" }, "css": { "lint": { "emptyRules": "ignore" } } });
    let mut f = real_with("vscode-html", Path::new(""), path.as_deref(), settings, &page);
    f.open_ready("site/index.html");
    let tab = f.tab();
    let view = f.s.state().provider().unwrap();
    println!("VS Code HTML {:?} (untested: {}); capabilities {:?}", view.version, view.untested, view.supports);
    // Problems of the embedded stylesheet and script, placed on the original text (`.list { }` is an empty rule the
    // settings ignore).
    f.settle("problems", Duration::from_secs(60), |s| s.state().active().and_then(|t| s.state().diagnostics(t.id())).is_some_and(|d| d.items.len() >= 2));
    let found: Vec<(String, String, String)> = f.s.state().diagnostics(tab).unwrap().items.iter().map(|d| (d.severity.label().to_owned(), d.message.clone(), HTML_EDIT[d.range.clone()].to_owned())).collect();
    println!("problems: {found:?}");
    assert_eq!(found, [("Warning".into(), "Unknown property: 'colr'".into(), "colr".into()), ("Error".into(), "Expression expected.".into(), ";".into())]);
    let colr = HTML_EDIT.find("colr").unwrap();
    assert_eq!(f.s.state().diagnostics(tab).unwrap().items[0].range, colr..colr + 4, "after `𝄞` on its line: UTF-16 columns read as bytes exactly");
    // Missing assistance is explained, not attempted (at the stylesheet's problem).
    f.caret("colr", 1);
    assert_eq!(f.s.disabled(code_core::session::CommandId::QuickFix), Some("The language server does not offer quick fixes"));
    assert_eq!(f.s.disabled(code_core::session::CommandId::OrganizeImports), Some("The language server does not offer organize imports"));
    // Format Document: three spaces from `.editorconfig`, the provider's `format` settings (head and body indented),
    // the stylesheet and script formatted by their own languages; one Undo step.
    f.s.dispatch(Command::Format(code_core::session::FormatKind::Format));
    f.settle("formatted", Duration::from_secs(60), |s| s.state().asking().is_none());
    let formatted = current_text(f.s.state());
    println!("formatted:\n{formatted}\nnotices {:?}", f.notices());
    // The server's own reading of the unfinished markup (the `</` and the unclosed <section> nest what follows); its
    // `endWithNewline` setting is off, so the final newline goes.
    assert_eq!(formatted, HTML_FORMATTED);
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), HTML_EDIT, "one Undo takes the formatting back");
    // A tag name after `𝄞`: the server's range replaces exactly the typed prefix.
    f.caret("𝄞 x</p>", "𝄞 x".len());
    f.type_at_caret(" <sp");
    let tag = f.complete_until("span");
    f.s.dispatch(Command::SelectCompletion(tag));
    f.s.dispatch(Command::AcceptCompletion);
    f.settle("tag", Duration::from_secs(30), |s| current_text(s.state()).contains("<p>𝄞 x <span</p>"));
    let named = current_text(f.s.state());
    // Typing its `>` closes it, with the caret between the tags; one Undo takes the closing tag back.
    f.type_at_caret(">");
    f.settle("closed", Duration::from_secs(30), |s| current_text(s.state()).contains("<span></span>"));
    assert!(current_text(f.s.state()).contains("<p>𝄞 x <span></span></p>"));
    let inside = current_text(f.s.state()).find("<span></span>").unwrap() + "<span>".len();
    assert_eq!(f.s.state().selection().map(|s| s.range()), Some(inside..inside), "the caret stays between the tags");
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), named.replace("<span</p>", "<span></p>"), "the typed `>` stays");
    for _ in 0..3 { f.s.dispatch(Command::Undo { tab }); }
    assert_eq!(current_text(f.s.state()), HTML_EDIT);
    // A closing tag completed after `</`: the server replaces the `/` with the whole `/li>`.
    f.caret("two</\n", "two</".len());
    let close = f.complete_until("/li");
    f.s.dispatch(Command::SelectCompletion(close));
    f.s.dispatch(Command::AcceptCompletion);
    f.settle("closing tag", Duration::from_secs(30), |s| current_text(s.state()).contains("<li>two</li>\n</ul>"));
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), HTML_EDIT);
    // Typing `</` after an unclosed element closes it.
    f.caret("𝄞 open\n", "𝄞 open".len());
    f.type_at_caret("</");
    f.settle("closed by </", Duration::from_secs(30), |s| current_text(s.state()).contains("<section>𝄞 open</section>\n"));
    f.s.dispatch(Command::Undo { tab });
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), HTML_EDIT);
    // An attribute: its snippet leaves the caret between the quotes.
    f.caret("<p>𝄞 x", 2);
    f.type_at_caret(" ti");
    let title = f.complete_until("title");
    f.s.dispatch(Command::SelectCompletion(title));
    f.s.dispatch(Command::AcceptCompletion);
    f.settle("attribute", Duration::from_secs(30), |s| current_text(s.state()).contains("<p title=\"\">𝄞 x</p>"));
    let quotes = current_text(f.s.state()).find("<p title=\"").unwrap() + "<p title=\"".len();
    assert_eq!(f.s.state().selection().map(|s| s.range()), Some(quotes..quotes));
    f.s.dispatch(Command::Undo { tab });
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), HTML_EDIT);
    // CSS in the page's <style>: a property, the caret before its `;`, and its values suggested at once.
    f.caret(".list { }", ".list { ".len());
    f.type_at_caret("colo");
    let color = f.complete_until("color");
    f.s.dispatch(Command::SelectCompletion(color));
    f.s.dispatch(Command::AcceptCompletion);
    f.settle("property", Duration::from_secs(30), |s| current_text(s.state()).contains(".list { color: ;}"));
    let value = current_text(f.s.state()).find(".list { color: ").unwrap() + ".list { color: ".len();
    assert_eq!(f.s.state().selection().map(|s| s.range()), Some(value..value), "the caret is where the value goes");
    f.settle("values", Duration::from_secs(30), |s| s.state().completion().is_some_and(|c| c.rows().any(|r| r.label == "red")));
    let red = f.s.state().completion().unwrap().rows().position(|r| r.label == "red").unwrap();
    f.s.dispatch(Command::SelectCompletion(red));
    f.s.dispatch(Command::AcceptCompletion);
    f.settle("value", Duration::from_secs(30), |s| current_text(s.state()).contains(".list { color: red;}"));
    f.s.dispatch(Command::Undo { tab });
    f.s.dispatch(Command::Undo { tab });
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), HTML_EDIT);
    // JavaScript in the page's <script>: the page's function, then its parameters while typing the call.
    f.caret("let a = ;", "let a = ".len());
    f.type_at_caret("gre");
    let greet = f.complete_until("greet");
    f.s.dispatch(Command::SelectCompletion(greet));
    f.s.dispatch(Command::AcceptCompletion);
    f.settle("function", Duration::from_secs(30), |s| current_text(s.state()).contains("let a = greet;"));
    f.type_at_caret("(\"x\", ");
    let seen = f.notices().len();
    f.s.dispatch(Command::ParameterInfo);
    f.settle("signature", Duration::from_secs(30), move |s| s.state().signature_help().is_some() || s.state().notices().len() > seen);
    let sig = f.s.state().signature_help().and_then(|h| h.active_signature()).map(|s| (s.label.clone(), s.active_parameter));
    println!("signature: {sig:?}");
    assert_eq!(sig, Some(("greet(name: any, punct: any): string".into(), Some(1))));
    f.s.dispatch(Command::CloseSignature);
    assert_eq!(files(&f.root), ["site/.editorconfig", "site/index.html"], "nothing written into the project");
    assert_eq!(std::fs::read_to_string(f.root.join("site/index.html")).unwrap(), HTML_EDIT, "the page is untouched on disk");
}

/// Without Node.js the built-in servers cannot run: said, with what to install; reading goes on.
#[test]
#[ignore = "needs VS Code's built-in server or vscode-langservers-extracted"]
fn real_web_without_node() {
    let extracted = installed("vscode-css", "CSS").map(|(exe, _)| exe);
    if extracted.is_none() && code_core::semantics::provider::bundled_server(&code_core::semantics::provider::VSCODE_EXTENSIONS, PROVIDERS.iter().find(|p| p.id == "vscode-css").unwrap().bundled.unwrap()).is_none() {
        println!("SKIPPED: no installed CSS server");
        return;
    }
    let empty = tempfile::tempdir().unwrap();
    let npm_launcher = extracted.is_some();
    let mut f = real("vscode-css", extracted.as_deref().unwrap_or(Path::new("")), empty.path().to_str(), &[("site/style.css", CSS)]);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.show("site/style.css");
    f.settle("missing", Duration::from_secs(10), |s| s.state().provider().is_some_and(|v| matches!(v.status, Status::Missing(_) | Status::Failed(_))));
    let status = f.s.state().provider().unwrap().status;
    match status {
        Status::Failed(why) if npm_launcher => assert!(why.contains("exit status: 127") && why.contains("node"), "{why}"),
        Status::Missing(why) if !npm_launcher => assert!(why.contains("VS Code's built-in server") && why.contains("needs Node.js"), "{why}"),
        other => panic!("unexpected missing-runtime outcome: {other:?}"),
    }
    assert!(current_text(f.s.state()).starts_with(":root"));
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
        if f.s.state().current_source().unwrap().line_of(f.s.state().selection().unwrap().head) == 1 {
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
    f.settle("refresh", Duration::from_secs(10), |s| s.state().current_source().is_some_and(|d| d.copy_range(0..d.len_bytes()).unwrap().starts_with("# moved")));
    f.show("lib/app.rb");
    let moved = Instant::now();
    loop {
        f.definition("Círculo.new(2)", 1);
        if f.s.state().current_source().unwrap().line_of(f.s.state().selection().unwrap().head) == 5 {
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

const KT_EDIT: &str = "package geo\n\nfun edited(): String {\n    val d = LocalDate.now()\n        val c = make(2.0)\n    return \"$d $c\"\n}\n";

/// Kotlin LSP editing the mixed fixture through its own commands (allowlisted, user decision 2026-10-25): the
/// `LocalDate` import fix, completion of a Java class's member from Kotlin and of a library class with its import,
/// parameter information, formatting and organize imports each change the draft as one Undo step, never the file;
/// a fix whose edit comes back for text changed meanwhile is refused; commands outside the allowlist stay refused.
#[test]
#[ignore = "needs an installed Kotlin LSP and Kotlin 2.2.21 in ~/.m2; run in release, alone"]
fn real_kotlin_editing() {
    let Some((exe, path)) = installed("kotlin-lsp", "KOTLIN") else { println!("SKIPPED: Kotlin LSP is not installed"); return };
    if !kotlin_cached() { println!("SKIPPED: Kotlin 2.2.21 is not in ~/.m2"); return; }
    let mut files = KT_FILES.to_vec();
    files.push(("src/main/kotlin/geo/Edit.kt", KT_EDIT));
    let mut f = real("kotlin-lsp", &exe, path.as_deref(), &files);
    f.open_ready("src/main/kotlin/geo/Edit.kt");
    let tab = f.tab();
    f.settle("problems", Duration::from_secs(120), |s| s.state().active().and_then(|t| s.state().diagnostics(t.id())).is_some_and(|d| d.items.iter().any(|p| p.message.contains("LocalDate"))));
    assert_eq!(f.problems(), ["Error: Unresolved reference 'LocalDate'."]);
    // The import fix: its command sends the edit back.
    f.caret("LocalDate", 2);
    let seen = f.notices().len();
    f.s.dispatch(Command::QuickFix);
    f.settle("quick fixes", Duration::from_secs(60), move |s| s.state().code_actions().is_some() || s.state().notices().len() > seen);
    let actions = f.s.state().code_actions().unwrap().actions.clone();
    let import = actions.iter().position(|a| a.title.contains("java.time.LocalDate")).unwrap_or_else(|| panic!("{actions:?}"));
    assert!(actions[import].unavailable.is_none());
    assert!(actions.iter().filter(|a| a.title.starts_with("Extract")).all(|a| a.unavailable.is_some()), "commands outside the allowlist stay refused");
    f.s.dispatch(Command::SelectCodeAction(import));
    f.s.dispatch(Command::ApplyCodeAction);
    f.settle("imported", Duration::from_secs(30), |s| current_text(s.state()).contains("import java.time.LocalDate"));
    let imported = current_text(f.s.state());
    println!("imported:\n{imported}");
    assert_eq!(imported.replace("import java.time.LocalDate\n", "").replace("\n\n\n", "\n\n"), KT_EDIT, "only the import was added");
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), KT_EDIT, "one Undo takes it back");
    f.s.dispatch(Command::Redo { tab });
    // Problems for the version shown (asked of the server, which stops publishing): `LocalDate` resolves.
    f.settle("resolved", Duration::from_secs(60), |s| s.state().active().and_then(|t| s.state().diagnostics(t.id())).is_some_and(|d| !d.items.iter().any(|p| p.message.contains("LocalDate"))));
    println!("problems after the import: {:?}", f.problems());
    // Kotlin to Java: completing a member of the project's Java class.
    f.caret("    return", 4);
    f.type_at_caret("val t = app.Helper.tw\n    ");
    f.caret("Helper.tw\n", 9);
    let seen = f.notices().len();
    f.s.dispatch(Command::Complete);
    f.settle("completion", Duration::from_secs(60), move |s| s.state().completion().is_some() || s.state().notices().len() > seen);
    let labels: Vec<String> = f.s.state().completion().map(|c| c.rows().take(6).map(|r| r.label.clone()).collect()).unwrap_or_default();
    println!("completion of Helper.tw: {labels:?}");
    let row = f.s.state().completion().unwrap().rows().position(|r| r.label.starts_with("twice")).unwrap_or_else(|| panic!("{labels:?}"));
    f.s.dispatch(Command::SelectCompletion(row));
    f.s.dispatch(Command::AcceptCompletion);
    f.settle("completed", Duration::from_secs(30), |s| current_text(s.state()).contains("app.Helper.twice"));
    println!("completed:\n{}", current_text(f.s.state()));
    f.s.dispatch(Command::Undo { tab });
    assert!(current_text(f.s.state()).contains("app.Helper.tw\n"), "one Undo takes the completion back");
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), imported);
    // Parameter information.
    f.caret("make(2.0)", 5);
    let seen = f.notices().len();
    f.s.dispatch(Command::ParameterInfo);
    f.settle("signature", Duration::from_secs(30), move |s| s.state().signature_help().is_some() || s.state().notices().len() > seen);
    assert_eq!(f.s.state().signature_help().map(|h| h.signatures.iter().map(|s| s.label.clone()).collect::<Vec<_>>()), Some(vec!["make(r: Double): Círculo".to_owned()]));
    f.s.dispatch(Command::CloseSignature);
    // Formatting, as one Undo step.
    f.s.dispatch(Command::Format(code_core::session::FormatKind::Format));
    f.settle("formatted", Duration::from_secs(60), |s| s.state().asking().is_none());
    let formatted = current_text(f.s.state());
    assert!(formatted.contains("\n    val c = make(2.0)\n"), "{formatted}");
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), imported);
    // Organize imports: an unused import is removed, through its command.
    f.caret("import java.time.LocalDate", 0);
    f.type_at_caret("import java.util.UUID\n");
    f.s.dispatch(Command::Format(code_core::session::FormatKind::OrganizeImports));
    f.settle("organized", Duration::from_secs(60), |s| !current_text(s.state()).contains("java.util.UUID"));
    assert_eq!(current_text(f.s.state()), imported);
    // A fix whose edit comes back after the text changed is refused whole.
    f.s.dispatch(Command::Undo { tab });
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), imported);
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), KT_EDIT);
    f.settle("unresolved again", Duration::from_secs(60), |s| s.state().active().and_then(|t| s.state().diagnostics(t.id())).is_some_and(|d| d.items.iter().any(|p| p.message.contains("LocalDate"))));
    f.caret("LocalDate", 2);
    f.s.dispatch(Command::QuickFix);
    f.settle("quick fixes again", Duration::from_secs(60), |s| s.state().code_actions().is_some());
    let import = f.s.state().code_actions().unwrap().actions.iter().position(|a| a.title.contains("java.time.LocalDate")).unwrap();
    f.s.dispatch(Command::SelectCodeAction(import));
    f.s.dispatch(Command::ApplyCodeAction);
    f.caret("fun edited", 0);
    f.type_at_caret("// x\n");
    f.settle("refused", Duration::from_secs(30), |s| s.state().notices().iter().any(|n| n.starts_with("Quick fixes: the file changed")) || current_text(s.state()).contains("import java.time"));
    println!("after a fix raced by typing:\n{}\nnotices {:?}", current_text(f.s.state()), f.notices());
    assert_eq!(current_text(f.s.state()), KT_EDIT.replacen("fun edited", "// x\nfun edited", 1), "the late edit is refused");
    assert_eq!(std::fs::read_to_string(f.root.join("src/main/kotlin/geo/Edit.kt")).unwrap(), KT_EDIT);
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
    f.settle("refresh", Duration::from_secs(10), |s| s.state().current_source().is_some_and(|d| d.copy_range(0..d.len_bytes()).unwrap().starts_with("// moved")));
    f.show("src/main/kotlin/geo/Main.kt");
    let moved = Instant::now();
    loop {
        f.definition("make(2.0)", 1);
        if file_of(&f.s) == "Shapes.kt" && f.s.state().current_source().unwrap().line_of(f.s.state().selection().unwrap().head) == 8 {
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
    assert_eq!((file_of(&f.s), f.s.state().current_source().unwrap().line_of(f.s.state().selection().unwrap().head)), ("Loose.kt".into(), 0));
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
    assert!(current_text(g.s.state()).starts_with("package geo"), "reading goes on");
}

const RS_FILES: [(&str, &str); 6] = [
    ("Cargo.toml", "[workspace]\nmembers = [\"shapes\", \"app\"]\nexclude = [\"loose\"]\nresolver = \"2\"\n"),
    ("shapes/Cargo.toml", "[package]\nname = \"shapes\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
    ("shapes/src/lib.rs", "pub struct Círculo {\n    pub radio: f64,\n}\n\nimpl Círculo {\n    pub fn área(&self) -> f64 {\n        3.14159 * self.radio * self.radio\n    }\n}\n\npub fn make(radio: f64) -> Círculo {\n    Círculo { radio }\n}\n"),
    ("app/Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nshapes = { path = \"../shapes\" }\n"),
    ("app/src/main.rs", "use shapes::{make, Círculo};\r\n\r\nfn main() {\r\n    let s = \"𝄞\"; let c: Círculo = make(2.0);\r\n    println!(\"{s} {}\", c.área() + make(1.0).área());\r\n    let v: Vec<u8> = Vec::new();\r\n    drop(v);\r\n}\r\n"),
    ("loose/Cargo.toml", "[package]\nname = \"loose\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
];

/// rust-analyzer completion through the session: a manual request after a non-BMP character on a CRLF line
/// (UTF-16 replacement ranges), acceptance as one undoable draft edit, a member list opened by typing `.`, and
/// the fixture on disk unchanged. Acceptance waits for the server to fill the item in; its snippet is expanded.
#[test]
#[ignore = "needs an installed rust-analyzer; run in release, alone"]
fn real_rust_completion() {
    let Some((exe, path)) = installed("rust-analyzer", "RUST_ANALYZER") else {
        println!("SKIPPED: rust-analyzer is not installed");
        return;
    };
    let main = "fn helper_function() -> u32 {\r\n    1\r\n}\r\n\r\nfn main() {\r\n    let s = String::from(\"𝄞\"); helper_fu\r\n}\r\n";
    let mut f = real("rust-analyzer", &exe, path.as_deref(), &[("Cargo.toml", "[package]\nname = \"complete\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"), ("src/main.rs", main)]);
    assert!(std::process::Command::new("cargo").args(["generate-lockfile", "--offline"]).current_dir(&f.root).status().is_ok_and(|s| s.success()));
    f.open_ready("src/main.rs");
    let at = main.find("helper_fu\r").unwrap() + "helper_fu".len();
    f.s.dispatch(Command::Select(Selection { anchor: at, head: at }));
    f.s.dispatch(Command::Complete);
    f.settle("completion", Duration::from_secs(60), |s| s.state().completion().is_some());
    let rows: Vec<_> = f.s.state().completion().unwrap().rows().map(|r| (r.label.clone(), r.unavailable)).collect();
    println!("rows: {rows:?}");
    let row = rows.iter().position(|(label, why)| label.starts_with("helper_function") && why.is_none()).expect("helper_function is offered");
    f.s.dispatch(Command::SelectCompletion(row));
    f.s.dispatch(Command::AcceptCompletion);
    // Applied once the server has filled the suggestion in (its imports, if any).
    f.settle("accepted", Duration::from_secs(30), |s| !current_text(s.state()).contains("helper_fu\r"));
    let accepted = current_text(f.s.state());
    assert_eq!(accepted, main.replacen("); helper_fu\r", "); helper_function()\r", 1), "only the call was inserted (its snippet)");
    // A member list, opened by typing the trigger character.
    let tab = f.s.state().active().unwrap().id();
    let caret = f.s.state().selection().unwrap().head;
    let expected = f.s.state().view(tab).unwrap().snapshot().key();
    f.s.dispatch(Command::Edit { tab, expected, edits: vec![code_core::text::TextEdit { range: caret..caret, text: "; s.".into() }], kind: code_core::text::EditKind::Typing });
    f.settle("member completion", Duration::from_secs(60), |s| s.state().completion().is_some_and(|c| c.rows().any(|r| r.label.starts_with("len"))));
    f.s.dispatch(Command::CloseCompletion);
    f.s.dispatch(Command::Undo { tab });
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), main, "each step undoes on its own");
    assert_eq!(std::fs::read_to_string(f.root.join("src/main.rs")).unwrap(), main);
}

const RS_EDIT: &str = "fn main() {\n    let mut m = HashMap::<u32, u32>::new();\n        m.insert(1, 2);\n    println!(\"{}\", m.len());\n}\n\nfn Bad_Name() {}\n";

/// rust-analyzer editing a Cargo package: problems for the shown version, the auto-import fix for `HashMap` as one
/// Undo step, type-aware member completion, completion of `HashSet` with its `use` line (resolved on acceptance)
/// as one Undo step, parameter information, rustfmt through Format Document, and a format answered after an edit
/// refused. The file on disk never changes.
#[test]
#[ignore = "needs an installed rust-analyzer with rust-src and rustfmt; run in release, alone"]
fn real_rust_editing() {
    let Some((exe, path)) = installed("rust-analyzer", "RUST_ANALYZER") else {
        println!("SKIPPED: rust-analyzer is not installed");
        return;
    };
    let mut f = real("rust-analyzer", &exe, path.as_deref(), &[("Cargo.toml", "[package]\nname = \"edit\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"), ("src/main.rs", RS_EDIT)]);
    assert!(std::process::Command::new("cargo").args(["generate-lockfile", "--offline"]).current_dir(&f.root).status().is_ok_and(|s| s.success()));
    f.open_ready("src/main.rs");
    let tab = f.tab();
    // rust-analyzer's own diagnostics only (Lenz starts it without cargo check): a function name in the wrong case.
    println!("problems: {:?}", f.problems());
    // The auto-import fix at `HashMap`.
    f.caret("HashMap::<", 3);
    let seen = f.notices().len();
    f.s.dispatch(Command::QuickFix);
    f.settle("quick fixes", Duration::from_secs(60), move |s| s.state().code_actions().is_some() || s.state().notices().len() > seen);
    let titles: Vec<String> = f.s.state().code_actions().map(|c| c.actions.iter().map(|a| format!("{} [{}]", a.title, a.unavailable.as_deref().unwrap_or("ok"))).collect()).unwrap_or_default();
    println!("quick fixes: {titles:?}; notices {:?}", f.notices());
    let fix = f.s.state().code_actions().and_then(|c| c.actions.iter().position(|a| a.title.contains("std::collections::HashMap") && a.unavailable.is_none())).unwrap_or_else(|| panic!("{titles:?}"));
    f.s.dispatch(Command::SelectCodeAction(fix));
    f.s.dispatch(Command::ApplyCodeAction);
    let imported = current_text(f.s.state());
    println!("imported:\n{imported}");
    assert!(imported.starts_with("use std::collections::HashMap;\n"), "{imported}");
    assert_eq!(imported.replacen("use std::collections::HashMap;\n\n", "", 1), RS_EDIT, "only the use line was added");
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), RS_EDIT);
    f.s.dispatch(Command::Redo { tab });
    f.settle("problems", Duration::from_secs(120), |s| s.state().active().and_then(|t| s.state().diagnostics(t.id())).is_some_and(|d| d.items.iter().any(|p| p.message.contains("Bad_Name"))));
    println!("problems after the import: {:?}", f.problems());
    // Type-aware member completion on the map.
    f.caret("m.len()", 2);
    let seen = f.notices().len();
    f.s.dispatch(Command::Complete);
    f.settle("members", Duration::from_secs(60), move |s| s.state().completion().is_some() || s.state().notices().len() > seen);
    let members: Vec<String> = f.s.state().completion().map(|c| c.rows().map(|r| r.label.clone()).collect()).unwrap_or_default();
    println!("members: {:?}", members.iter().take(12).collect::<Vec<_>>());
    for m in ["len", "insert", "contains_key"] { assert!(members.iter().any(|l| l == m), "{m} in {members:?}"); }
    f.s.dispatch(Command::CloseCompletion);
    // Completion with its import: `HashS` completes to `HashSet` and adds `use std::collections::HashSet;`.
    f.caret("    println", 4);
    f.type_at_caret("let s = HashS\n    ");
    f.caret("HashS\n", 5);
    let typed = current_text(f.s.state());
    let seen = f.notices().len();
    f.s.dispatch(Command::Complete);
    f.settle("completion", Duration::from_secs(60), move |s| s.state().completion().is_some_and(|c| c.rows().any(|r| r.label.starts_with("HashSet"))) || s.state().notices().len() > seen);
    let rows: Vec<String> = f.s.state().completion().map(|c| c.rows().take(8).map(|r| format!("{} {:?} {:?}", r.label, r.detail, r.unavailable)).collect()).unwrap_or_default();
    println!("completion: {rows:?}");
    let row = f.s.state().completion().unwrap().rows().position(|r| r.label.starts_with("HashSet") && r.unavailable.is_none()).unwrap_or_else(|| panic!("{rows:?}"));
    f.s.dispatch(Command::SelectCompletion(row));
    f.s.dispatch(Command::AcceptCompletion);
    f.settle("accepted", Duration::from_secs(30), |s| current_text(s.state()).contains("HashSet") && !current_text(s.state()).contains("HashS\n"));
    let completed = current_text(f.s.state());
    println!("completed:\n{completed}");
    assert!(completed.contains("use std::collections::{HashMap, HashSet};") || completed.contains("use std::collections::HashSet;"), "{completed}");
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), typed, "one Undo takes the name and its import back");
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), imported);
    // Parameter information inside `m.insert(`.
    f.caret("insert(1", 7);
    let seen = f.notices().len();
    f.s.dispatch(Command::ParameterInfo);
    f.settle("signature", Duration::from_secs(30), move |s| s.state().signature_help().is_some() || s.state().notices().len() > seen);
    let sig = f.s.state().signature_help().and_then(|h| h.active_signature()).map(|s| (s.label.clone(), s.active_parameter));
    println!("signature: {sig:?}");
    assert!(sig.as_ref().is_some_and(|(l, p)| l.contains("insert(&mut self, k: u32, v: u32)") && *p == Some(0)), "{sig:?}");
    f.s.dispatch(Command::CloseSignature);
    // rustfmt through the server, as one Undo step.
    f.s.dispatch(Command::Format(code_core::session::FormatKind::Format));
    f.settle("formatted", Duration::from_secs(60), |s| s.state().asking().is_none());
    let formatted = current_text(f.s.state());
    assert!(formatted.contains("\n    m.insert(1, 2);\n"), "{formatted}; {:?}", f.notices());
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), imported);
    // A format answered after an edit is refused.
    f.caret("fn main", 0);
    f.s.dispatch(Command::Format(code_core::session::FormatKind::Format));
    f.type_at_caret("// x\n");
    f.settle("stale format", Duration::from_secs(60), |s| s.state().asking().is_none());
    assert_eq!(current_text(f.s.state()), imported.replacen("fn main", "// x\nfn main", 1), "{:?}", f.notices());
    assert_eq!(std::fs::read_to_string(f.root.join("src/main.rs")).unwrap(), RS_EDIT);
}

const GO_MAIN: &str = "package main\n\nimport (\n\t\"fmt\"\n\n\t\"github.com/google/uuid\"\n)\n\nfunc main() {\n\tid := uuid.New()\n\t\tfmt.Println(id.String(), strings.ToUpper(\"x\"))\n}\n";

/// gopls editing a Go module that requires `github.com/google/uuid`: problems for the shown version, Organize Imports
/// adding the missing standard-library import as one Undo step, completion of the dependency's members (proof of
/// module context), parameter information, gofmt through Format Document, and a format answered after an edit
/// refused. The file on disk never changes. (gopls guesses a bare `"uuid"` path for a dependency no file imports yet,
/// so the fixture imports it.)
#[test]
#[ignore = "needs an installed gopls and Go, and the module proxy for github.com/google/uuid; run in release, alone"]
fn real_gopls_editing() {
    let Some((exe, path)) = installed("gopls", "GOPLS") else {
        println!("SKIPPED: gopls is not installed");
        return;
    };
    let mut f = real("gopls", &exe, path.as_deref(), &[("go.mod", "module example.com/demo\n\ngo 1.24\n\nrequire github.com/google/uuid v1.6.0\n"), ("main.go", GO_MAIN)]);
    let tidy = std::process::Command::new("go").args(["mod", "download", "github.com/google/uuid"]).current_dir(&f.root).status();
    assert!(tidy.is_ok_and(|s| s.success()), "the module's dependency must be downloadable");
    let sum = std::process::Command::new("go").args(["mod", "download", "-json", "github.com/google/uuid@v1.6.0"]).current_dir(&f.root).output().unwrap();
    let sum: Value = serde_json::from_slice(&sum.stdout).unwrap();
    std::fs::write(f.root.join("go.sum"), format!("github.com/google/uuid v1.6.0 {}\ngithub.com/google/uuid v1.6.0/go.mod {}\n", sum["Sum"].as_str().unwrap(), sum["GoModSum"].as_str().unwrap())).unwrap();
    f.open_ready("main.go");
    let tab = f.tab();
    println!("capabilities: {:?}", f.s.state().provider().unwrap().supports);
    f.settle("problems", Duration::from_secs(120), |s| s.state().active().and_then(|t| s.state().diagnostics(t.id())).is_some_and(|d| d.items.iter().any(|p| p.message.contains("strings"))));
    println!("problems: {:?}", f.problems());
    // Organize Imports adds `strings`, as one Undo step.
    f.s.dispatch(Command::Format(code_core::session::FormatKind::OrganizeImports));
    f.settle("organized", Duration::from_secs(60), |s| s.state().asking().is_none());
    let organized = current_text(f.s.state());
    println!("organized:\n{organized}\nnotices {:?}", f.notices());
    assert!(organized.contains("\t\"strings\"\n") && organized.contains("\"github.com/google/uuid\""), "{organized}");
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), GO_MAIN);
    f.s.dispatch(Command::Redo { tab });
    f.settle("resolved", Duration::from_secs(60), |s| s.state().active().and_then(|t| s.state().diagnostics(t.id())).is_some_and(|d| !d.items.iter().any(|p| p.message.contains("undefined"))));
    // The dependency's members.
    f.caret("uuid.New()", 5);
    let seen = f.notices().len();
    f.s.dispatch(Command::Complete);
    f.settle("members", Duration::from_secs(60), move |s| s.state().completion().is_some() || s.state().notices().len() > seen);
    let members: Vec<String> = f.s.state().completion().map(|c| c.rows().map(|r| r.label.clone()).collect()).unwrap_or_default();
    println!("uuid members: {:?}", members.iter().take(10).collect::<Vec<_>>());
    assert!(members.iter().any(|m| m == "New") && members.iter().any(|m| m == "Parse"), "{members:?}");
    f.s.dispatch(Command::CloseCompletion);
    // Parameter information inside `strings.ToUpper(`.
    f.caret("ToUpper(\"x\"", 8);
    let seen = f.notices().len();
    f.s.dispatch(Command::ParameterInfo);
    f.settle("signature", Duration::from_secs(30), move |s| s.state().signature_help().is_some() || s.state().notices().len() > seen);
    let sig = f.s.state().signature_help().and_then(|h| h.active_signature()).map(|s| s.label.clone());
    println!("signature: {sig:?}");
    assert!(sig.as_deref().is_some_and(|l| l.contains("ToUpper(s string) string")), "{sig:?}");
    f.s.dispatch(Command::CloseSignature);
    // gofmt through the server, as one Undo step.
    f.s.dispatch(Command::Format(code_core::session::FormatKind::Format));
    f.settle("formatted", Duration::from_secs(60), |s| s.state().asking().is_none());
    let formatted = current_text(f.s.state());
    assert!(formatted.contains("\n\tfmt.Println(id.String()"), "{formatted}; {:?}", f.notices());
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), organized);
    // A format answered after an edit is refused.
    f.caret("func main", 0);
    f.s.dispatch(Command::Format(code_core::session::FormatKind::Format));
    f.type_at_caret("// x\n");
    f.settle("stale format", Duration::from_secs(60), |s| s.state().asking().is_none());
    assert_eq!(current_text(f.s.state()), organized.replacen("func main", "// x\nfunc main", 1), "{:?}", f.notices());
    assert_eq!(std::fs::read_to_string(f.root.join("main.go")).unwrap(), GO_MAIN);
}

const TS_EDIT_FILES: [(&str, &str); 7] = [
    ("package.json", "{ \"name\": \"edit\", \"private\": true }\n"),
    ("tsconfig.json", "{ \"compilerOptions\": { \"strict\": true, \"jsx\": \"preserve\", \"allowJs\": true, \"checkJs\": true, \"noEmit\": true }, \"include\": [\"src\"] }\n"),
    ("src/jsx.d.ts", "declare namespace JSX { interface IntrinsicElements { [name: string]: any } interface Element {} }\n"),
    ("src/util.ts", "export function formatName(first: string, last: string): string {\n  return `${first} ${last}`;\n}\nexport const unusedThing = 1;\n"),
    ("src/app.ts", "import { unusedThing } from \"./util\";\n\nconst s = \"𝄞\"; const full = formatNa\nconsole.log(s, full);\n    console.log(formatName(\"a\", \"b\"));\n"),
    ("src/legacy.js", "/** @type {{ size: number, label: string }} */\nconst box = { size: 1, label: \"x\" };\nbox.\n"),
    ("src/App.tsx", "import { Button } from \"./Button\";\n\nexport const app = <Button lab />;\n"),
];
const TS_BUTTON: (&str, &str) = ("src/Button.tsx", "export function Button(props: { label: string; onPress?: () => void }) {\n  return <button>{props.label}</button>;\n}\n");

/// The tsserver Lenz's tests hand typescript-language-server: `READER_TSSERVER`, else the global npm one.
fn tsserver_settings() -> Value {
    let path = std::env::var("READER_TSSERVER").ok().or_else(|| {
        let root = std::process::Command::new("npm").args(["root", "-g"]).output().ok()?;
        let lib = PathBuf::from(String::from_utf8_lossy(&root.stdout).trim()).join("typescript/lib");
        lib.join("tsserver.js").is_file().then(|| lib.to_string_lossy().into_owned())
    });
    path.map_or(Value::Null, |p| json!({ "tsserver": { "path": p } }))
}

/// typescript-language-server editing TS, JS and TSX: completion with its auto-import after a non-BMP character
/// (UTF-16 ranges) as one Undo step, problems for the shown version, the add-import quick fix, parameter
/// information, formatting, Organize Imports sorting imports, JSDoc-typed members in JavaScript, and a
/// component's props completed in TSX. The files on disk never change.
#[test]
#[ignore = "needs an installed typescript-language-server and TypeScript; run in release, alone"]
fn real_typescript_editing() {
    let Some((exe, path)) = installed("typescript-language-server", "TYPESCRIPT") else {
        println!("SKIPPED: typescript-language-server is not installed");
        return;
    };
    let mut files = TS_EDIT_FILES.to_vec();
    files.push(TS_BUTTON);
    let mut f = real_with("typescript-language-server", &exe, path.as_deref(), tsserver_settings(), &files);
    f.open_ready("src/app.ts");
    let tab = f.tab();
    let app = TS_EDIT_FILES[4].1;
    // Completion of an export of another module, with its import, after `𝄞` on the same line.
    // tsserver may answer nothing while it loads the project: asked again until it lists the export.
    let asked = Instant::now();
    loop {
        f.caret("formatNa\n", 8);
        let seen = f.notices().len();
        f.s.dispatch(Command::Complete);
        f.settle("completion", Duration::from_secs(60), move |s| s.state().completion().is_some() || s.state().notices().len() > seen);
        if f.s.state().completion().is_some_and(|c| c.rows().any(|r| r.label == "formatName")) { break; }
        println!("completion at {:.1} s: {:?} {:?}", asked.elapsed().as_secs_f64(), f.s.state().completion().map(|c| c.rows().take(5).map(|r| r.label.clone()).collect::<Vec<_>>()), f.notices().last());
        f.s.dispatch(Command::CloseCompletion);
        assert!(asked.elapsed() < Duration::from_secs(30), "formatName was never offered");
        std::thread::sleep(Duration::from_millis(500));
    }
    let rows: Vec<String> = f.s.state().completion().map(|c| c.rows().take(6).map(|r| format!("{} {:?} {:?}", r.label, r.detail, r.unavailable)).collect()).unwrap_or_default();
    println!("completion: {rows:?}; notices {:?}", f.notices());
    let row = f.s.state().completion().unwrap().rows().position(|r| r.label == "formatName" && r.unavailable.is_none()).unwrap_or_else(|| panic!("{rows:?}"));
    f.s.dispatch(Command::SelectCompletion(row));
    f.s.dispatch(Command::AcceptCompletion);
    f.settle("accepted", Duration::from_secs(30), |s| current_text(s.state()).contains("full = formatName"));
    let completed = current_text(f.s.state());
    println!("completed:\n{completed}");
    assert!(completed.contains("import { formatName, unusedThing } from \"./util\";") || completed.contains("import { unusedThing, formatName } from \"./util\";"), "{completed}");
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), app, "one Undo takes the name and its import back");
    // Problems for the shown version, then the add-import fix for a use without an import.
    f.settle("problems", Duration::from_secs(60), |s| s.state().active().and_then(|t| s.state().diagnostics(t.id())).is_some_and(|d| d.items.iter().any(|p| p.message.contains("formatName"))));
    println!("problems: {:?}", f.problems());
    f.caret("formatName(\"a\"", 3);
    let seen = f.notices().len();
    f.s.dispatch(Command::QuickFix);
    f.settle("quick fixes", Duration::from_secs(60), move |s| s.state().code_actions().is_some() || s.state().notices().len() > seen);
    let titles: Vec<String> = f.s.state().code_actions().map(|c| c.actions.iter().map(|a| format!("{} [{}]", a.title, a.unavailable.as_deref().unwrap_or("ok"))).collect()).unwrap_or_default();
    println!("quick fixes: {titles:?}");
    let fix = f.s.state().code_actions().and_then(|c| c.actions.iter().position(|a| a.title.contains("import from \"./util\"") && a.unavailable.is_none())).unwrap_or_else(|| panic!("{titles:?}"));
    f.s.dispatch(Command::SelectCodeAction(fix));
    f.s.dispatch(Command::ApplyCodeAction);
    let fixed = current_text(f.s.state());
    println!("fixed:\n{fixed}");
    assert!(fixed.contains("formatName") && fixed.lines().next().is_some_and(|l| l.contains("formatName")), "{fixed}");
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), app);
    f.s.dispatch(Command::Redo { tab });
    // Parameter information (with `formatName` imported).
    f.caret("formatName(\"a\"", 11);
    let seen = f.notices().len();
    f.s.dispatch(Command::ParameterInfo);
    f.settle("signature", Duration::from_secs(30), move |s| s.state().signature_help().is_some() || s.state().notices().len() > seen);
    let sig = f.s.state().signature_help().and_then(|h| h.active_signature()).map(|s| (s.label.clone(), s.active_parameter));
    println!("signature: {sig:?}");
    assert!(sig.as_ref().is_some_and(|(l, p)| l.contains("formatName(first: string, last: string): string") && *p == Some(0)), "{sig:?}");
    f.s.dispatch(Command::CloseSignature);
    // Formatting re-indents the misplaced line, as one Undo step.
    f.s.dispatch(Command::Format(code_core::session::FormatKind::Format));
    f.settle("formatted", Duration::from_secs(60), |s| s.state().asking().is_none());
    let formatted = current_text(f.s.state());
    println!("formatted:\n{formatted}");
    assert!(formatted.contains("\nconsole.log(formatName(\"a\", \"b\"));"), "{formatted}");
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), fixed);
    // Organize Imports sorts and combines (typescript-language-server's `source.organizeImports`; removing unused
    // imports is its separate `source.removeUnusedImports.ts`).
    f.caret("import { formatName", 0);
    f.type_at_caret("import { z } from \"./zz\";\n");
    let unsorted = current_text(f.s.state());
    f.s.dispatch(Command::Format(code_core::session::FormatKind::OrganizeImports));
    f.settle("organized", Duration::from_secs(60), |s| s.state().asking().is_none());
    let organized = current_text(f.s.state());
    println!("organized:\n{organized}; notices {:?}", f.notices());
    assert!(organized.find("./util").zip(organized.find("./zz")).is_some_and(|(u, z)| u < z), "{organized}");
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), unsorted);
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), fixed);
    // JavaScript: members from a JSDoc type.
    f.show("src/legacy.js");
    f.caret("box.\n", 4);
    let seen = f.notices().len();
    f.s.dispatch(Command::Complete);
    f.settle("js members", Duration::from_secs(60), move |s| s.state().completion().is_some() || s.state().notices().len() > seen);
    let members: Vec<String> = f.s.state().completion().map(|c| c.rows().map(|r| r.label.clone()).collect()).unwrap_or_default();
    println!("js members: {members:?}");
    assert!(members.iter().any(|m| m == "size") && members.iter().any(|m| m == "label"), "{members:?}");
    f.s.dispatch(Command::CloseCompletion);
    // TSX: a component's props.
    f.show("src/App.tsx");
    f.caret("lab />", 3);
    let seen = f.notices().len();
    f.s.dispatch(Command::Complete);
    f.settle("props", Duration::from_secs(60), move |s| s.state().completion().is_some() || s.state().notices().len() > seen);
    let props: Vec<String> = f.s.state().completion().map(|c| c.rows().map(|r| r.label.clone()).collect()).unwrap_or_default();
    println!("props: {props:?}");
    assert!(props.iter().any(|p| p.starts_with("label")), "{props:?}");
    f.s.dispatch(Command::CloseCompletion);
    for (name, text) in &files { assert_eq!(&std::fs::read_to_string(f.root.join(name)).unwrap(), text, "{name} is untouched on disk"); }
}

const RB_EDIT_SHAPES: &str = "# frozen_string_literal: true\n\nmodule Shapes\n  class Círculo\n    def initialize(radio)\n      @radio = radio\n    end\n\n    def área\n      3.14159 * @radio * @radio\n    end\n  end\nend\n";
const RB_EDIT_APP: &str = "# frozen_string_literal: true\n\nrequire_relative \"shapes\"\n\ns = \"𝄞\"\nc = Shapes::Círculo.new(2)\nputs s, c.área\nunused = 1\n    puts Shapes\n";

/// Ruby LSP editing a bundled project whose Gemfile has RuboCop: constant and method completion after a non-BMP
/// character, parameter information, RuboCop's problems and a fix, and RuboCop formatting, each one Undo step; the
/// files on disk never change. Printed first: what it offers.
#[test]
#[ignore = "needs installed ruby-lsp and rubocop gems; run in release, alone"]
fn real_ruby_editing() {
    let Some((exe, path)) = installed("ruby-lsp", "RUBY_LSP") else {
        println!("SKIPPED: Ruby LSP is not installed");
        return;
    };
    let mut f = real("ruby-lsp", &exe, path.as_deref(), &[("Gemfile", "source \"https://rubygems.org\"\n\ngem \"rubocop\"\n"), (".rubocop.yml", "AllCops:\n  NewCops: disable\n  SuggestExtensions: false\n"), ("lib/shapes.rb", RB_EDIT_SHAPES), ("lib/app.rb", RB_EDIT_APP)]);
    let lock = std::process::Command::new("bundle").args(["lock", "--local"]).current_dir(&f.root).output().unwrap();
    assert!(lock.status.success(), "{}", String::from_utf8_lossy(&lock.stderr));
    f.open_ready("lib/app.rb");
    let tab = f.tab();
    println!("capabilities: {:?}", f.s.state().provider().unwrap().supports);
    f.settle("problems", Duration::from_secs(120), |s| s.state().active().and_then(|t| s.state().diagnostics(t.id())).is_some_and(|d| !d.items.is_empty()));
    println!("problems: {:?}", f.problems());
    // Diagnostics can arrive while Ruby LSP is still indexing installed gems and the project's constants.
    f.settle("Ruby indexing after diagnostics", Duration::from_secs(60), |s| s.state().provider().is_some_and(|v| v.status == Status::Ready));
    // Method completion after `𝄞` on the line.
    f.caret("c.área", 2);
    let seen = f.notices().len();
    f.s.dispatch(Command::Complete);
    f.settle("methods", Duration::from_secs(60), move |s| s.state().completion().is_some() || s.state().notices().len() > seen);
    let methods: Vec<String> = f.s.state().completion().map(|c| c.rows().take(10).map(|r| r.label.clone()).collect()).unwrap_or_default();
    println!("methods: {methods:?}; notices {:?}", f.notices());
    f.s.dispatch(Command::CloseCompletion);
    // A constant of the project, on a line after `𝄞` (UTF-16 positions).
    f.caret("puts Shapes\n", 12);
    f.type_at_caret("Shapes::C\n");
    f.caret("Shapes::C\n", 9);
    let typed = current_text(f.s.state());
    // Ruby LSP queues didChange on its worker but parses incoming requests on its reader thread. Wait for the
    // changed draft's diagnostics before asking completion, so the request is parsed after that edit is applied.
    let changed = f.s.state().view(tab).unwrap().snapshot().clone();
    f.settle("Ruby changed-draft diagnostics", Duration::from_secs(60), |s| s.state().diagnostics(tab).is_some_and(|d| d.describes(&changed)));
    let seen = f.notices().len();
    f.s.dispatch(Command::Complete);
    f.settle("constants", Duration::from_secs(60), move |s| s.state().completion().is_some() || s.state().notices().len() > seen);
    let constants: Vec<String> = f.s.state().completion().map(|c| c.rows().map(|r| r.label.clone()).collect()).unwrap_or_default();
    println!("constants: {constants:?}; notices {:?}", f.notices());
    let row = constants.iter().position(|c| c.ends_with("Círculo")).unwrap_or_else(|| panic!("{constants:?}"));
    f.s.dispatch(Command::SelectCompletion(row));
    f.s.dispatch(Command::AcceptCompletion);
    f.settle("accepted", Duration::from_secs(30), |s| current_text(s.state()).contains("\nShapes::Círculo\n"));
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), typed, "one Undo takes the completion back");
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), RB_EDIT_APP);
    // Parameter information: Ruby LSP names the constructor without `initialize`'s parameters.
    f.caret("new(2)", 4);
    let seen = f.notices().len();
    f.s.dispatch(Command::ParameterInfo);
    f.settle("signature", Duration::from_secs(30), move |s| s.state().signature_help().is_some() || s.state().notices().len() > seen);
    let signatures = f.s.state().signature_help().map(|h| h.signatures.iter().map(|s| s.label.clone()).collect::<Vec<_>>());
    println!("signature: {signatures:?}");
    assert_eq!(signatures, Some(vec!["new()".to_owned()]));
    f.s.dispatch(Command::CloseSignature);
    // RuboCop's problems, and its fix for the useless assignment, as one Undo step.
    f.settle("rubocop problems", Duration::from_secs(60), |s| s.state().active().and_then(|t| s.state().diagnostics(t.id())).is_some_and(|d| d.items.iter().any(|p| p.message.starts_with("Lint/UselessAssignment"))));
    f.caret("unused = 1", 2);
    let seen = f.notices().len();
    f.s.dispatch(Command::QuickFix);
    f.settle("quick fixes", Duration::from_secs(60), move |s| s.state().code_actions().is_some() || s.state().notices().len() > seen);
    let fix = f.s.state().code_actions().and_then(|c| c.actions.iter().position(|a| a.title == "Autocorrect Lint/UselessAssignment" && a.unavailable.is_none())).unwrap_or_else(|| panic!("{:?}", f.notices()));
    f.s.dispatch(Command::SelectCodeAction(fix));
    f.s.dispatch(Command::ApplyCodeAction);
    let fixed = current_text(f.s.state());
    println!("fixed:\n{fixed}");
    assert!(!fixed.contains("unused = 1") && fixed.contains("\n1\n"), "{fixed}");
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), RB_EDIT_APP);
    // RuboCop formatting, as one Undo step.
    f.s.dispatch(Command::Format(code_core::session::FormatKind::Format));
    f.settle("formatted", Duration::from_secs(60), |s| s.state().asking().is_none());
    let formatted = current_text(f.s.state());
    println!("formatted:\n{formatted}");
    assert!(formatted.contains("require_relative 'shapes'\n") && formatted.contains("\nc = Shapes::Círculo.new(2)\n") && formatted.contains("s = '𝄞'") && formatted.contains("\nputs Shapes\n"), "{formatted}");
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), RB_EDIT_APP);
    for (name, text) in [("lib/app.rb", RB_EDIT_APP), ("lib/shapes.rb", RB_EDIT_SHAPES)] { assert_eq!(std::fs::read_to_string(f.root.join(name)).unwrap(), text); }

}

/// Ruby LSP on a folder without a Gemfile: it starts in a bundle of its own (in the reader's cache) and completes the
/// project's constants; no Gemfile, lockfile or `.ruby-lsp` is written into the folder.
#[test]
#[ignore = "needs an installed ruby-lsp gem; run alone"]
fn real_ruby_without_a_gemfile() {
    let Some((exe, path)) = installed("ruby-lsp", "RUBY_LSP") else {
        println!("SKIPPED: Ruby LSP is not installed");
        return;
    };
    let app = "require_relative \"shapes\"\n\nShapes::C\n";
    let mut f = real("ruby-lsp", &exe, path.as_deref(), &[("lib/shapes.rb", RB_EDIT_SHAPES), ("lib/app.rb", app)]);
    let before = tree(&f.root);
    f.open_ready("lib/app.rb");
    f.caret("Shapes::C\n", 9);
    let asked = Instant::now();
    let constants = loop {
        let seen = f.notices().len();
        f.s.dispatch(Command::Complete);
        f.settle("constants", Duration::from_secs(60), move |s| s.state().completion().is_some() || s.state().notices().len() > seen);
        let found: Vec<String> = f.s.state().completion().map(|c| c.rows().map(|r| r.label.clone()).collect()).unwrap_or_default();
        if !found.is_empty() || asked.elapsed() > Duration::from_secs(30) { break found; }
        std::thread::sleep(Duration::from_millis(500));
    };
    println!("constants without a Gemfile: {constants:?}");
    assert!(constants.iter().any(|c| c.ends_with("Círculo")), "{constants:?}; {:?}", f.notices());
    assert_eq!(tree(&f.root), before, "nothing written into the folder");
}

/// A minimal Rails 8.1 application (Active Record on SQLite, Action Controller and Action View; its schema written as
/// Rails dumps it: rubocop-rails 2.38.0 fails on a `t.timestamps` there) whose bundle has
/// RuboCop with rubocop-rails: a model with an association and a validation, a controller with a callback, a route
/// helper and an unzoned `Time.now`, and a helper using a route helper.
const RAILS_APP: [(&str, &str); 17] = [
    ("Gemfile", "source \"https://rubygems.org\"\n\ngem \"rails\"\ngem \"sqlite3\"\ngem \"rubocop\"\ngem \"rubocop-rails\"\n"),
    (".rubocop.yml", "plugins:\n  - rubocop-rails\nAllCops:\n  NewCops: disable\n  SuggestExtensions: false\nStyle/Documentation:\n  Enabled: false\n"),
    ("Rakefile", "# frozen_string_literal: true\n\nrequire_relative \"config/application\"\nRails.application.load_tasks\n"),
    ("bin/rails", "#!/usr/bin/env ruby\n# frozen_string_literal: true\n\nAPP_PATH = File.expand_path(\"../config/application\", __dir__)\nrequire_relative \"../config/boot\"\nrequire \"rails/commands\"\n"),
    ("config/boot.rb", "# frozen_string_literal: true\n\nENV[\"BUNDLE_GEMFILE\"] ||= File.expand_path(\"../Gemfile\", __dir__)\nrequire \"bundler/setup\"\n"),
    ("config/application.rb", "# frozen_string_literal: true\n\nrequire_relative \"boot\"\nrequire \"rails\"\nrequire \"active_model/railtie\"\nrequire \"active_record/railtie\"\nrequire \"action_controller/railtie\"\nrequire \"action_view/railtie\"\nBundler.require(*Rails.groups)\n\nmodule Blog\n  class Application < Rails::Application\n    config.load_defaults 8.1\n    config.eager_load = false\n    config.logger = Logger.new(nil)\n  end\nend\n"),
    ("config/environment.rb", "# frozen_string_literal: true\n\nrequire_relative \"application\"\nRails.application.initialize!\n"),
    ("config/database.yml", "development:\n  adapter: sqlite3\n  database: db/development.sqlite3\n"),
    ("config/routes.rb", "Rails.application.routes.draw do\n  resources :posts\nend\n"),
    ("db/schema.rb", "ActiveRecord::Schema[8.1].define(version: 1) do\n  create_table \"posts\" do |t|\n    t.string \"title\", null: false\n    t.text \"body\"\n    t.datetime \"created_at\", null: false\n    t.datetime \"updated_at\", null: false\n  end\n\n  create_table \"comments\" do |t|\n    t.integer \"post_id\", null: false\n    t.text \"text\"\n  end\nend\n"),
    ("app/models/application_record.rb", "# frozen_string_literal: true\n\nclass ApplicationRecord < ActiveRecord::Base\n  primary_abstract_class\nend\n"),
    ("app/models/post.rb", RAILS_POST),
    ("app/models/comment.rb", "# frozen_string_literal: true\n\nclass Comment < ApplicationRecord\n  belongs_to :post\nend\n"),
    ("app/controllers/application_controller.rb", "# frozen_string_literal: true\n\nclass ApplicationController < ActionController::Base\nend\n"),
    ("app/controllers/posts_controller.rb", RAILS_CONTROLLER),
    ("app/helpers/posts_helper.rb", RAILS_HELPER),
    ("app/helpers/application_helper.rb", "# frozen_string_literal: true\n\nmodule ApplicationHelper\nend\n"),
];
const RAILS_POST: &str = "# frozen_string_literal: true\n\nclass Post < ApplicationRecord\n  has_many :comments\n  validates :title, presence: true\n\n  def comment_count\n    comments.size\n  end\nend\n";
const RAILS_CONTROLLER: &str = "# frozen_string_literal: true\n\nclass PostsController < ApplicationController\n  before_action :set_post, only: :show\n\n  def index\n    @posts = Post.where(ti)\n    @checked = Time.now\n      redirect_to posts_path\n  end\n\n  def show; end\n\n  private\n\n  def set_post\n    @post = Post.find(post_id)\n  end\n\n  def post_id\n    params[:id]\n  end\nend\n";
const RAILS_HELPER: &str = "# frozen_string_literal: true\n\nmodule PostsHelper\n  def post_link(post)\n    link_to(post.title, post_path(post))\n  end\nend\n";

/// The installed gems the Rails fixture needs that are missing (none: it can run).
fn rails_gems_missing() -> Vec<&'static str> {
    let listed = std::process::Command::new("gem").args(["list", "--local"]).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    ["ruby-lsp-rails", "railties", "sqlite3", "rubocop", "rubocop-rails"].into_iter().filter(|g| !listed.lines().any(|l| l.starts_with(&format!("{g} (")))).collect()
}

/// Lock the fixture's bundle from installed gems and load its schema into its development database (inside the
/// fixture), as a developer's `bundle install` and `bin/rails db:prepare` would have.
fn rails_setup(root: &Path) {
    std::fs::set_permissions(root.join("bin/rails"), std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    for args in [&["lock", "--local"][..], &["exec", "rails", "db:schema:load"]] {
        let out = std::process::Command::new("bundle").args(args).current_dir(root).env("BUNDLE_GEMFILE", root.join("Gemfile")).output().unwrap();
        assert!(out.status.success(), "bundle {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }
}

/// Ruby LSP with its Rails add-on on a Rails application: application context from the booted app (a model's
/// columns in its documentation and as `where` suggestions, also on a line typed into the draft; association and
/// route-helper targets) and from the add-on's static analysis (callback targets, association methods as
/// suggestions, DSL calls as file symbols), in a model, a controller and a helper; parameter information from the
/// bundled Active Record; rubocop-rails problems with a fix and RuboCop formatting, each one Undo step. The files on
/// disk never change; the booted application writes only its database's own files.
#[test]
#[ignore = "needs installed ruby-lsp, ruby-lsp-rails, rails, sqlite3, rubocop and rubocop-rails gems; run alone"]
fn real_rails_editing() {
    let Some((exe, path)) = installed("ruby-lsp", "RUBY_LSP") else {
        println!("SKIPPED: Ruby LSP is not installed");
        return;
    };
    let missing = rails_gems_missing();
    if !missing.is_empty() {
        println!("SKIPPED: gems not installed: {missing:?}");
        return;
    }
    let mut f = real("ruby-lsp", &exe, path.as_deref(), &RAILS_APP);
    rails_setup(&f.root);
    let before = files(&f.root);
    let controller = "app/controllers/posts_controller.rb";
    f.open_ready(controller);
    let tab = f.tab();
    println!("capabilities: {:?}", f.s.state().provider().unwrap().supports);
    // A model's columns come from the booted application (its server starts after Ruby LSP initializes).
    f.caret("Post.where", 1);
    let asked = Instant::now();
    let documentation = loop {
        let seen = f.notices().len();
        f.s.dispatch(Command::Documentation);
        f.settle("documentation", Duration::from_secs(60), move |s| s.state().documentation().is_some() || s.state().notices().len() > seen);
        let text = f.s.state().documentation().map(|d| d.preview.doc.text.clone()).unwrap_or_default();
        if text.contains("Columns") || asked.elapsed() > Duration::from_secs(60) { break text; }
        f.s.dispatch(Command::CloseDocumentation);
        std::thread::sleep(Duration::from_millis(500));
    };
    println!("documentation of Post:\n{documentation}");
    for column in ["title: string - not null", "body: text", "created_at: datetime - not null"] {
        assert!(documentation.contains(column), "{column}: {documentation}; {}", f.s.language_server_details());
    }
    f.s.dispatch(Command::CloseDocumentation);
    // The model's columns as `where` suggestions, with what they filter.
    f.caret("where(ti)", 8);
    let seen = f.notices().len();
    f.s.dispatch(Command::Complete);
    f.settle("columns", Duration::from_secs(60), move |s| s.state().completion().is_some() || s.state().notices().iter().skip(seen).any(|n| n.starts_with("Code completion")));
    let rows: Vec<(String, Option<String>)> = f.s.state().completion().map(|c| c.rows().map(|r| (r.label.clone(), r.detail.clone())).collect()).unwrap_or_default();
    println!("where suggestions: {rows:?}");
    assert_eq!(rows, [("title".to_owned(), Some("Filter Post records by title".to_owned()))]);
    f.s.dispatch(Command::AcceptCompletion);
    f.settle("accepted", Duration::from_secs(30), |s| current_text(s.state()) != RAILS_CONTROLLER);
    assert_eq!(current_text(f.s.state()), RAILS_CONTROLLER.replacen("where(ti)", "where(title: )", 1));
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), RAILS_CONTROLLER, "one Undo takes the suggestion back");
    // On a line typed into the draft (never saved): another model's columns.
    f.caret("    redirect_to", 0);
    f.type_at_caret("    Comment.where(te)\n");
    f.s.dispatch(Command::CloseCompletion);
    let typed = current_text(f.s.state());
    f.caret("where(te)", 8);
    // Seen once: the first answer for a model the add-on had not looked up yet was empty; asked again.
    let asked = Instant::now();
    let rows = loop {
        let seen = f.notices().len();
        f.s.dispatch(Command::Complete);
        f.settle("draft columns", Duration::from_secs(60), move |s| s.state().completion().is_some() || s.state().notices().iter().skip(seen).any(|n| n.starts_with("Code completion")));
        let rows: Vec<(String, Option<String>)> = f.s.state().completion().map(|c| c.rows().map(|r| (r.label.clone(), r.detail.clone())).collect()).unwrap_or_default();
        println!("draft where suggestions at {:.1} s: {rows:?}; notices {:?}", asked.elapsed().as_secs_f64(), f.notices());
        if !rows.is_empty() || asked.elapsed() > Duration::from_secs(30) { break rows; }
        f.s.dispatch(Command::CloseCompletion);
        std::thread::sleep(Duration::from_millis(500));
    };
    assert_eq!(rows, [("text".to_owned(), Some("Filter Comment records by text".to_owned()))]);
    f.s.dispatch(Command::AcceptCompletion);
    let draft = typed.clone();
    f.settle("accepted", Duration::from_secs(30), move |s| current_text(s.state()) != draft);
    assert_eq!(current_text(f.s.state()), typed.replacen("where(te)", "where(text: )", 1));
    f.s.dispatch(Command::Undo { tab });
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), RAILS_CONTROLLER);
    // Parameter information from the bundled Active Record, with the caret after the argument. (On a call inside the
    // parentheses, `post_id` or `params[:id]`, Ruby LSP 0.26.11 infers that call's receiver and answers nothing.)
    // It resolves `find` through the gems' index, which keeps growing after the project's own files are indexed:
    // asked again while it answers nothing.
    f.caret("find(post_id)", 12);
    let asked = Instant::now();
    let signatures = loop {
        let seen = f.notices().len();
        f.s.dispatch(Command::ParameterInfo);
        f.settle("signature", Duration::from_secs(30), move |s| s.state().signature_help().is_some() || s.state().notices().len() > seen);
        let signatures = f.s.state().signature_help().map(|h| h.signatures.iter().map(|s| s.label.clone()).collect::<Vec<_>>());
        println!("signature at {:.1} s: {signatures:?}; status {:?}", asked.elapsed().as_secs_f64(), f.s.state().provider().map(|v| v.status.clone()));
        if signatures.is_some() || asked.elapsed() > Duration::from_secs(30) { break signatures; }
        std::thread::sleep(Duration::from_millis(1000));
    };
    assert_eq!(signatures, Some(vec!["find(*ids)".to_owned()]));
    f.s.dispatch(Command::CloseSignature);
    // A callback's method; a route helper's route (from the booted application).
    f.definition(":set_post", 2);
    assert_eq!((file_of(&f.s), f.s.state().current_source().unwrap().line_of(f.s.state().selection().unwrap().head)), ("posts_controller.rb".into(), 15));
    f.show(controller);
    f.definition("posts_path", 2);
    assert_eq!((file_of(&f.s), f.s.state().current_source().unwrap().line_of(f.s.state().selection().unwrap().head)), ("routes.rb".into(), 1));
    // rubocop-rails' problem and its fix, as one Undo step.
    f.show(controller);
    f.settle("rails problems", Duration::from_secs(60), |s| s.state().active().and_then(|t| s.state().diagnostics(t.id())).is_some_and(|d| d.items.iter().any(|p| p.message.starts_with("Rails/TimeZone"))));
    println!("problems: {:?}", f.problems());
    f.caret("Time.now", 6);
    let seen = f.notices().len();
    f.s.dispatch(Command::QuickFix);
    f.settle("quick fixes", Duration::from_secs(60), move |s| s.state().code_actions().is_some() || s.state().notices().len() > seen);
    let titles: Vec<String> = f.s.state().code_actions().map(|c| c.actions.iter().map(|a| a.title.clone()).collect()).unwrap_or_default();
    println!("quick fixes: {titles:?}");
    let fix = f.s.state().code_actions().and_then(|c| c.actions.iter().position(|a| a.title == "Autocorrect Rails/TimeZone" && a.unavailable.is_none())).unwrap_or_else(|| panic!("{titles:?}"));
    f.s.dispatch(Command::SelectCodeAction(fix));
    f.s.dispatch(Command::ApplyCodeAction);
    assert_eq!(current_text(f.s.state()), RAILS_CONTROLLER.replacen("Time.now", "Time.zone.now", 1));
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), RAILS_CONTROLLER);
    // RuboCop formatting with the project's configuration, as one Undo step.
    f.s.dispatch(Command::Format(code_core::session::FormatKind::Format));
    f.settle("formatted", Duration::from_secs(60), |s| s.state().asking().is_none());
    let formatted = current_text(f.s.state());
    println!("formatted:\n{formatted}");
    assert!(formatted.contains("\n    @checked = Time.now\n    redirect_to posts_path\n"), "{formatted}; {:?}", f.notices());
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), RAILS_CONTROLLER);
    // The model: an association's target (from the booted application), its methods as suggestions, and the DSL
    // calls among the file's symbols.
    f.show("app/models/post.rb");
    f.definition(":comments", 2);
    assert_eq!((file_of(&f.s), f.s.state().current_source().unwrap().line_of(f.s.state().selection().unwrap().head)), ("comment.rb".into(), 2));
    f.show("app/models/post.rb");
    // (Typed on a line of its own: inside the existing `comments` Ruby LSP 0.26.11 suggests nothing.)
    let post_tab = f.tab();
    f.caret("    comments.size", 0);
    f.type_at_caret("    comm\n");
    f.s.dispatch(Command::CloseCompletion);
    f.caret("    comm\n", 8);
    let seen = f.notices().len();
    f.s.dispatch(Command::Complete);
    f.settle("association methods", Duration::from_secs(60), move |s| s.state().completion().is_some() || s.state().notices().iter().skip(seen).any(|n| n.starts_with("Code completion")));
    let rows: Vec<(String, Option<String>)> = f.s.state().completion().map(|c| c.rows().map(|r| (r.label.clone(), r.detail.clone())).collect()).unwrap_or_default();
    println!("association methods: {rows:?}; notices {:?}", f.notices());
    assert!(rows.contains(&("comments".to_owned(), Some("() post.rb".to_owned()))) && rows.contains(&("comments=".to_owned(), Some("(comments) post.rb".to_owned()))), "{rows:?}");
    f.s.dispatch(Command::CloseCompletion);
    f.s.dispatch(Command::Undo { tab: post_tab });
    assert_eq!(current_text(f.s.state()), RAILS_POST);
    f.s.dispatch(Command::FileSymbols);
    f.settle("file symbols", Duration::from_secs(10), |s| s.state().asking().is_none());
    let symbols: Vec<String> = f.s.state().results().map(|r| r.hits().iter().map(|h| h.text.to_string()).collect()).unwrap_or_default();
    println!("file symbols: {symbols:?}");
    // The add-on's `has_many :comments` and `validates :title`, shown (as every file symbol) by the source text of
    // their names.
    assert_eq!(symbols, ["Post", "comments", "title", "comment_count"]);
    f.s.dispatch(Command::CloseResults);
    // The helper: a route helper's route.
    f.show("app/helpers/posts_helper.rb");
    f.definition("post_path(post)", 2);
    assert_eq!((file_of(&f.s), f.s.state().current_source().unwrap().line_of(f.s.state().selection().unwrap().head)), ("routes.rb".into(), 1));
    for (name, text) in [(controller, RAILS_CONTROLLER), ("app/models/post.rb", RAILS_POST), ("app/helpers/posts_helper.rb", RAILS_HELPER)] {
        assert_eq!(std::fs::read_to_string(f.root.join(name)).unwrap(), text, "{name} is untouched on disk");
    }
    // The booted application opens its SQLite database, which keeps its write-ahead files beside it while open:
    // the application's own doing, as in any editor running the add-on. Nothing else is written.
    let database = |p: &String| !p.starts_with("db/development.sqlite3");
    let after = files(&f.root);
    println!("written by the booted application: {:?}", after.iter().filter(|p| !before.contains(p)).collect::<Vec<_>>());
    assert_eq!(after.into_iter().filter(database).collect::<Vec<_>>(), before.into_iter().filter(database).collect::<Vec<_>>(), "nothing else written into the application");
}

/// A Rails application whose boot fails (here its environment raises): the Rails add-on's server cannot start, so
/// application context is missing, and Lenz says why; ordinary Ruby editing (constants, RuboCop problems and
/// formatting) stays available.
#[test]
#[ignore = "needs installed ruby-lsp, ruby-lsp-rails, rails, sqlite3, rubocop and rubocop-rails gems; run alone"]
fn real_rails_that_cannot_boot() {
    let Some((exe, path)) = installed("ruby-lsp", "RUBY_LSP") else {
        println!("SKIPPED: Ruby LSP is not installed");
        return;
    };
    let missing = rails_gems_missing();
    if !missing.is_empty() {
        println!("SKIPPED: gems not installed: {missing:?}");
        return;
    }
    let mut f = real("ruby-lsp", &exe, path.as_deref(), &RAILS_APP);
    rails_setup(&f.root);
    write(&f.root, "config/environment.rb", "# frozen_string_literal: true\n\nrequire_relative \"application\"\nraise \"the test application does not boot\"\n");
    let controller = "app/controllers/posts_controller.rb";
    f.open_ready(controller);
    let tab = f.tab();
    // What the add-on reports when its server fails to boot.
    f.settle("add-on error", Duration::from_secs(60), |s| s.state().provider().is_some_and(|v| v.error.is_some()));
    let details = f.s.language_server_details();
    println!("details:\n{details}");
    assert!(details.contains("Ruby LSP Rails failed to initialize server") && details.contains("the test application does not boot"), "{details}");
    // Ordinary editing: RuboCop's problems and formatting, and the project's constants.
    f.settle("problems", Duration::from_secs(60), |s| s.state().active().and_then(|t| s.state().diagnostics(t.id())).is_some_and(|d| d.items.iter().any(|p| p.message.starts_with("Rails/TimeZone"))));
    f.s.dispatch(Command::Format(code_core::session::FormatKind::Format));
    f.settle("formatted", Duration::from_secs(60), |s| s.state().asking().is_none());
    assert!(current_text(f.s.state()).contains("\n    @checked = Time.now\n    redirect_to posts_path\n"), "{:?}", f.notices());
    f.s.dispatch(Command::Undo { tab });
    f.caret("Post.where", 2);
    let seen = f.notices().len();
    f.s.dispatch(Command::Complete);
    f.settle("constants", Duration::from_secs(60), move |s| s.state().completion().is_some() || s.state().notices().iter().skip(seen).any(|n| n.starts_with("Code completion")));
    let labels: Vec<String> = f.s.state().completion().map(|c| c.rows().map(|r| r.label.clone()).collect()).unwrap_or_default();
    println!("constants: {labels:?}");
    assert!(labels.iter().any(|l| l == "Post" || l == "PostsController" || l == "PostsHelper"), "{labels:?}");
    f.s.dispatch(Command::CloseCompletion);
    // Without the booted application: no columns.
    f.caret("where(ti)", 8);
    let seen = f.notices().len();
    f.s.dispatch(Command::Complete);
    f.settle("no columns", Duration::from_secs(60), move |s| s.state().completion().is_some() || s.state().notices().len() > seen || s.state().asking().is_none());
    assert!(!f.s.state().completion().is_some_and(|c| c.rows().any(|r| r.label == "title")));
    assert_eq!(current_text(f.s.state()), RAILS_CONTROLLER);
}

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
    assert!(f.s.state().current_source().unwrap().source().to_string_lossy().contains("/rustlib/src/"), "{:?}", f.s.state().current_source().map(|d| d.source().to_owned()));
    assert!(f.notices().iter().any(|n| n == "Read-only source from the Rust standard library"), "{:?}", f.notices());
    // The project's server answers there: no server of its own for the standard library's workspace.
    f.s.dispatch(Command::FileSymbols);
    f.settle("std file symbols", Duration::from_secs(60), |s| s.state().asking().is_none());
    assert!(f.s.state().results().is_some_and(|r| r.hits().iter().any(|h| h.text.contains("new"))), "{:?}", f.notices());
    f.s.dispatch(Command::CloseResults);
    // Understood, not only parsed: a definition from there into another standard-library file.
    f.definition("buf: RawVec<T, A>", 5);
    assert!(f.s.state().current_source().unwrap().source().to_string_lossy().contains("/raw_vec"), "{:?} {:?}", f.s.state().current_source().map(|d| d.source().to_owned()), f.notices());
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
    f.settle("refresh", Duration::from_secs(10), |s| s.state().current_source().is_some_and(|d| d.copy_range(0..d.len_bytes()).unwrap().starts_with("// moved")));
    f.show("app/src/main.rs");
    let moved = Instant::now();
    loop {
        f.definition("make(2.0)", 1);
        if file_of(&f.s) == "lib.rs" && f.s.state().current_source().unwrap().line_of(f.s.state().selection().unwrap().head) == 12 {
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
    let mutable = move |t: &code_core::session::View<'_>| t.style_at(1, v).1 & code_core::decor::MUTABLE != 0;
    // Answers made while it loads may lack the semantics; the final ones (asked again once ready) have them.
    f.settle("decorations", Duration::from_secs(120), |s| active_view(s).is_some_and(|t| t.inlay_hints().is_some_and(|h| h.len() >= 2) && mutable(&t)));
    let t = active_view(&f.s).unwrap();
    println!("line 1: {}\nline 3: {}", projected(&t, 1), projected(&t, 3));
    assert!(projected(&t, 1).contains("v: Vec<i32"), "{}", projected(&t, 1));
    assert!(projected(&t, 3).contains("count: 5"), "{}", projected(&t, 3));
    println!("hints {} ({} B), tokens {} ({} B)", t.inlay_hints().unwrap().len(), t.inlay_hints().unwrap().bytes(), t.tokens().unwrap().len(), t.tokens().unwrap().bytes());
}

#[test]
#[ignore = "needs an installed rust-analyzer; run in release, alone"]
fn real_rust_analyzer_type_hierarchy_by_syntax() {
    let Some((exe, path)) = installed("rust-analyzer", "RUST_ANALYZER") else { return println!("SKIPPED: rust-analyzer is not installed") };
    let lib = "pub trait Shape {}\npub trait Round: Shape {}\npub struct Circle;\nimpl Shape for Circle {}\nimpl Round for Circle {}\n";
    let mut f = real("rust-analyzer", &exe, path.as_deref(), &[("Cargo.toml", "[package]\nname = \"p\"\nedition = \"2021\"\n"), ("src/lib.rs", lib)]);
    f.open_ready("src/lib.rs");
    f.caret("Shape", 0);
    f.s.dispatch(Command::TypeHierarchy(code_core::semantics::hierarchy::Direction::Subtypes));
    f.settle("subtypes", Duration::from_secs(60), |s| s.state().results().is_some_and(|r| r.hits().len() == 3));
    f.caret("Circle", 0);
    f.s.dispatch(Command::TypeHierarchy(code_core::semantics::hierarchy::Direction::Supertypes));
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
    let has = |s: &code_core::session::Session| s.state().results().is_some_and(|r| (0..r.hits().len()).filter_map(|i| r.call(i)).any(|c| c.name == "Circle"));
    let started = Instant::now();
    while !has(&f.s) {
        assert!(started.elapsed() < Duration::from_secs(180), "no subtypes from JDT LS");
        f.s.dispatch(Command::TypeHierarchy(code_core::semantics::hierarchy::Direction::Subtypes));
        let until = Instant::now() + Duration::from_secs(3);
        f.settle("a level", Duration::from_secs(60), |s| Instant::now() > until && s.state().results().is_some_and(|r| r.call(0).is_some_and(|c| c.fold != code_core::search::Fold::Loading)) || has(s));
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
    f.settle("shape", Duration::from_secs(30), |s| s.state().current_source().is_some_and(|d| d.source().ends_with("Shape.java")));
    f.caret("Shape", 0);
    let has = |s: &code_core::session::Session| s.state().results().is_some_and(|r| (0..r.hits().len()).filter_map(|i| r.call(i)).any(|c| c.name == "Circle"));
    while !has(&f.s) {
        assert!(started.elapsed() < Duration::from_secs(240), "no subtypes from JDT LS");
        f.s.dispatch(Command::TypeHierarchy(code_core::semantics::hierarchy::Direction::Subtypes));
        let until = Instant::now() + Duration::from_secs(3);
        f.settle("a level", Duration::from_secs(60), |s| Instant::now() > until && s.state().results().is_some_and(|r| r.call(0).is_some_and(|c| c.fold != code_core::search::Fold::Loading)) || has(s));
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
    let has = |s: &code_core::session::Session| s.state().results().is_some_and(|r| (0..r.hits().len()).filter_map(|i| r.call(i)).any(|c| c.name == "Circle"));
    let started = Instant::now();
    while !has(&f.s) {
        assert!(started.elapsed() < Duration::from_secs(180), "no subtypes from JDT LS on a plain folder");
        f.s.dispatch(Command::TypeHierarchy(code_core::semantics::hierarchy::Direction::Subtypes));
        let until = Instant::now() + Duration::from_secs(3);
        f.settle("a level", Duration::from_secs(60), |s| Instant::now() > until && s.state().results().is_some_and(|r| r.call(0).is_some_and(|c| c.fold != code_core::search::Fold::Loading)) || has(s));
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
    f.open_ready("crates/code-core/src/session.rs");
    f.caret("Default for Settings", 0);
    let started = Instant::now();
    f.s.dispatch(Command::TypeHierarchy(code_core::semantics::hierarchy::Direction::Subtypes));
    f.settle("level", Duration::from_secs(120), |s| s.state().results().is_some_and(|r| r.call(0).is_some_and(|c| c.fold != code_core::search::Fold::Loading && c.fold != code_core::search::Fold::Closed)));
    let r = f.s.state().results().unwrap();
    println!("subtypes of Default: level of {} in {} ms", r.hits().len() - 1, started.elapsed().as_millis());
    // The same server's usages at the same place, for comparison: most of the level's time is the server's.
    f.caret("Default for Settings", 0);
    let started = Instant::now();
    f.s.dispatch(Command::Usages);
    f.settle("usages", Duration::from_secs(120), |s| s.state().results().is_some_and(|r| r.results_kind() == code_core::search::ResultsKind::Usages && r.done()));
    println!("usages of Default: {} in {} ms", f.s.state().results().unwrap().hits().len(), started.elapsed().as_millis());
    f.caret("Default for Settings", 0);
    let started = Instant::now();
    f.s.dispatch(Command::TypeHierarchy(code_core::semantics::hierarchy::Direction::Subtypes));
    f.settle("level again", Duration::from_secs(120), |s| s.state().results().is_some_and(|r| r.results_kind() == code_core::search::ResultsKind::Hierarchy && r.call(0).is_some_and(|c| c.fold != code_core::search::Fold::Loading && c.fold != code_core::search::Fold::Closed)));
    println!("subtypes of Default again (warm): level of {} in {} ms", f.s.state().results().unwrap().hits().len() - 1, started.elapsed().as_millis());
}

#[test]
#[ignore = "probe"]
fn real_rust_problems_probe() {
    let Some((exe, path)) = installed("rust-analyzer", "RUST_ANALYZER") else { return };
    let text = "fn main() {\n    let v: Vec<u8> = Vec::new();\n    v.lenn();\n    let x: u32 = \"no\";\n}\n";
    let mut f = real("rust-analyzer", &exe, path.as_deref(), &[("Cargo.toml", "[package]\nname = \"edit\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"), ("src/main.rs", text)]);
    assert!(std::process::Command::new("cargo").args(["generate-lockfile", "--offline"]).current_dir(&f.root).status().is_ok_and(|s| s.success()));
    f.open_ready("src/main.rs");
    let until = Instant::now() + Duration::from_secs(40);
    while Instant::now() < until { if let Some(c) = recv(&f.s.completions()) { f.s.dispatch(c); } }
    println!("PROBE problems: {:?} shown version: {}", f.problems(), f.s.state().diagnostics(f.tab()).is_some());
}

/// Without gopls: a Go file says it is missing and how to install it (Lenz never installs it), and editing goes on.
#[test]
fn a_missing_gopls_says_how_to_install_it_and_editing_goes_on() {
    let text = "package main\n\nfunc main() {}\n";
    let mut f = real("gopls", Path::new("/nonexistent/gopls"), None, &[("go.mod", "module example.com/demo\n\ngo 1.24\n"), ("main.go", text)]);
    let path = f.root.join("main.go");
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.s.dispatch(Command::Open(path.clone()));
    f.settle("missing", Duration::from_secs(30), |s| s.state().provider().is_some_and(|v| matches!(v.status, Status::Missing(_))));
    let details = f.s.language_server_details();
    assert!(details.contains("go install golang.org/x/tools/gopls@latest") && details.contains("never installs"), "{details}");
    f.caret("func main", 0);
    f.type_at_caret("// edited\n");
    assert_eq!(current_text(f.s.state()), text.replacen("func main", "// edited\nfunc main", 1));
}

/// A JSON schema with keys, an enum, descriptions, a required key and a nested object, written at test time.
const YAML_SCHEMA: &str = r#"{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "type": "object",
  "additionalProperties": false,
  "required": ["name"],
  "properties": {
    "name": { "type": "string", "description": "The service name." },
    "mode": { "type": "string", "enum": ["fast", "safe"], "description": "How it runs." },
    "retries": { "type": "integer", "minimum": 0, "description": "How often to retry." },
    "tags": { "type": "array", "items": { "type": "string" }, "description": "Labels for the service." },
    "server": {
      "type": "object", "description": "Where it listens.", "required": ["host"],
      "properties": { "host": { "type": "string", "description": "Host name." }, "port": { "type": "integer", "description": "Port number." } }
    }
  }
}"#;
/// Mapped to the schema by the project's settings: an enum value it does not accept, a list item of the wrong type
/// after a non-BMP character on its line, and a nested object missing its required key.
const YAML_APP: &str = "# 𝄞 service\nname: \"café\"\nmode: slow\ntags: [𝄞, 7]\nserver:\n  port: 80\n";
/// Its schema named by a modeline, relative to the file.
const YAML_MODELINE: &str = "# yaml-language-server: $schema=../schemas/service.schema.json\nname: 1\n";
/// A remote schema: never fetched unless the settings allow downloads.
const YAML_REMOTE: &str = "# yaml-language-server: $schema=http://schemas.example.invalid/remote.json\nkey: 1\n";
/// Tab indentation, which YAML forbids.
const YAML_TABS: &str = "a:\n\tb: 1\n";
const YAML_REMOTE_SCHEMA: &str = r#"{ "type": "object", "properties": { "key": { "type": "string", "description": "A remote key." } } }"#;

/// A local HTTP proxy recording the first line of every request it receives, answering `body` for a GET of
/// `served` and 404 otherwise: what the server would download, made visible.
fn recording_proxy(served: &'static str, body: &'static str) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use std::io::{BufRead, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = std::io::BufReader::new(stream);
            let mut first = String::new();
            let _ = reader.read_line(&mut first);
            let mut line = String::new();
            while reader.read_line(&mut line).is_ok_and(|n| n > 2) { line.clear(); }
            let first = first.trim().to_owned();
            let found = first == format!("GET {served} HTTP/1.1");
            log.lock().unwrap().push(first);
            let mut stream = reader.into_inner();
            let reply = if found { format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()) }
                else { "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned() };
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    (address, seen)
}

/// The yaml-language-server fixture: the schema, a project setting mapping `config/*.yaml` to it, a modeline file, a
/// remote-schema file and `.editorconfig` asking for four columns; the server's downloads routed through `proxy`.
fn yaml_fixture(exe: &Path, path: Option<&str>, proxy: &str, settings: Value) -> Real {
    let files = [
        ("schemas/service.schema.json", YAML_SCHEMA), ("config/app.yaml", YAML_APP), ("other/modeline.yaml", YAML_MODELINE),
        ("remote.yaml", YAML_REMOTE), ("tabs.yaml", YAML_TABS), (".editorconfig", "root = true\n\n[*.yaml]\nindent_size = 4\n"),
    ];
    real_entries(|root| {
        let mut env = json!({ "HTTP_PROXY": proxy, "HTTPS_PROXY": proxy });
        if let Some(path) = path { env["PATH"] = json!(path); }
        vec![("yaml-language-server".into(), json!({
            "executable": exe, "env": env, "settings": settings,
            "projects": { root.to_str().unwrap(): { "schemas": { "./schemas/service.schema.json": "config/*.yaml" } } },
        }))]
    }, &files)
}

impl Real {
    /// The active file's problems as (severity, message, the text at its range).
    fn problem_texts(&self) -> Vec<(String, String, String)> {
        let text = current_text(self.s.state());
        self.s.state().diagnostics(self.tab()).map(|d| d.items.iter().map(|p| (p.severity.label().to_owned(), p.message.clone(), text[p.range.clone()].to_owned())).collect()).unwrap_or_default()
    }
    fn problems_settle(&mut self, count: usize) {
        self.settle("problems", Duration::from_secs(60), move |s| s.state().active().and_then(|t| s.state().diagnostics(t.id())).is_some_and(|d| d.items.len() >= count));
    }
}

/// yaml-language-server editing configuration under a schema the project maps to it: problems at their exact ranges
/// (after a non-BMP character), key completion with the schema's description and its snippet as one Undo step, enum
/// values replacing the typed one, hover documentation, Enter indenting under a new key, Format Document with
/// `.editorconfig`'s size and the user's `format` settings as one Undo step, a schema named by a modeline, a tab
/// converted to the project's spaces by the server's quick fix, and a remote schema refused with nothing downloaded
/// (the server's HTTP proxy hears nothing). Files on disk never change.
#[test]
#[ignore = "needs an installed yaml-language-server and Node.js; run alone"]
fn real_yaml_schema_editing() {
    let Some((exe, path)) = installed("yaml-language-server", "YAML_LANGUAGE_SERVER") else {
        println!("SKIPPED: yaml-language-server is not installed");
        return;
    };
    let (proxy, heard) = recording_proxy("http://schemas.example.invalid/remote.json", YAML_REMOTE_SCHEMA);
    let mut f = yaml_fixture(&exe, path.as_deref(), &proxy, json!({ "format": { "singleQuote": true } }));
    let before = files(&f.root);
    f.open_ready("config/app.yaml");
    let tab = f.tab();
    let view = f.s.state().provider().unwrap();
    println!("yaml-language-server {:?} (untested: {}); capabilities {:?}", view.version, view.untested, view.supports);
    assert_eq!((view.version.as_deref(), view.untested), (Some("yaml-language-server 2.0.0"), false));
    f.problems_settle(3);
    let found = f.problem_texts();
    println!("problems: {found:?}");
    assert_eq!(found, [
        ("Error".into(), "Value is not accepted. Valid values: \"fast\", \"safe\".".into(), "slow".into()),
        ("Error".into(), "Incorrect type. Expected \"string\".".into(), "7".into()),
        ("Error".into(), "Missing property \"host\".".into(), "server".into()),
    ]);
    let seven = YAML_APP.find("𝄞, 7]").unwrap() + "𝄞, ".len();
    assert_eq!(f.s.state().diagnostics(tab).unwrap().items[1].range, seven..seven + 1, "UTF-16 columns after `𝄞` placed exactly");
    // A key from the schema, with its description; its snippet's value placeholder selected; one Undo step.
    f.caret("  port: 80\n", "  port: 80\n".len());
    f.type_at_caret("re");
    let retries = f.complete_until("retries");
    let row = f.s.state().completion().unwrap().rows().nth(retries).unwrap().clone();
    assert_eq!(row.documentation.as_deref(), Some("How often to retry."));
    let typed = current_text(f.s.state());
    f.s.dispatch(Command::SelectCompletion(retries));
    f.s.dispatch(Command::AcceptCompletion);
    f.settle("key", Duration::from_secs(30), |s| current_text(s.state()).ends_with("  port: 80\nretries: 0"));
    assert_eq!(f.selected(), "0", "the value placeholder is selected");
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), typed);
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), YAML_APP);
    // The enum's values, described; choosing one replaces the value that was there.
    f.caret("mode: slow", "mode: ".len());
    let safe = f.complete_until("safe");
    let rows: Vec<(String, Option<String>)> = f.s.state().completion().unwrap().rows().map(|r| (r.label.clone(), r.documentation.clone())).collect();
    assert_eq!(rows, [("fast".into(), Some("How it runs.".into())), ("safe".into(), Some("How it runs.".into()))]);
    f.s.dispatch(Command::SelectCompletion(safe));
    f.s.dispatch(Command::AcceptCompletion);
    f.settle("value", Duration::from_secs(30), |s| current_text(s.state()).contains("\nmode: safe\n"));
    assert_eq!(current_text(f.s.state()), YAML_APP.replacen("mode: slow", "mode: safe", 1));
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), YAML_APP);
    // A key's description on hover.
    f.caret("name:", 1);
    let seen = f.notices().len();
    f.s.dispatch(Command::Documentation);
    f.settle("documentation", Duration::from_secs(30), move |s| s.state().documentation().is_some() || s.state().notices().len() > seen);
    let documentation = f.s.state().documentation().map(|d| d.preview.doc.text.clone()).unwrap_or_default();
    println!("documentation: {documentation:?}");
    assert!(documentation.starts_with("The service name."), "{documentation:?}; {:?}", f.notices());
    f.s.dispatch(Command::CloseDocumentation);
    // Enter after a new key opens its block with `.editorconfig`'s four columns.
    f.caret("  port: 80\n", "  port: 80\n".len());
    f.type_at_caret("labels:");
    f.type_at_caret("\n");
    assert_eq!(current_text(f.s.state()), format!("{YAML_APP}labels:\n    "));
    // (Typing within half a second is one Undo step.)
    for _ in 0..2 { if current_text(f.s.state()) != YAML_APP { f.s.dispatch(Command::Undo { tab }); } }
    assert_eq!(current_text(f.s.state()), YAML_APP);
    // Format Document: four columns from `.editorconfig`, single quotes from the user's `format` settings.
    f.s.dispatch(Command::Format(code_core::session::FormatKind::Format));
    f.settle("formatted", Duration::from_secs(60), |s| s.state().asking().is_none() && current_text(s.state()) != YAML_APP);
    let formatted = current_text(f.s.state());
    println!("formatted:\n{formatted}\nnotices {:?}", f.notices());
    assert_eq!(formatted, "# 𝄞 service\nname: 'café'\nmode: slow\ntags: [𝄞, 7]\nserver:\n    port: 80\n");
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), YAML_APP, "one Undo takes the formatting back");
    // A schema named by a modeline, relative to the file.
    f.show("other/modeline.yaml");
    f.problems_settle(1);
    assert_eq!(f.problem_texts(), [("Error".into(), "Incorrect type. Expected \"string\".".into(), "1".into())]);
    // A tab used as indentation: the server's quick fix converts it to the project's four spaces, as one Undo step.
    f.show("tabs.yaml");
    f.problems_settle(1);
    let tabbed = f.problem_texts();
    println!("tabs: {tabbed:?}");
    assert_eq!(tabbed[0], ("Error".into(), "Tabs are not allowed as indentation".into(), "\tb: 1".into()));
    f.caret("\tb", 0);
    let seen = f.notices().len();
    f.s.dispatch(Command::QuickFix);
    f.settle("quick fixes", Duration::from_secs(30), move |s| s.state().code_actions().is_some() || s.state().notices().len() > seen);
    let titles: Vec<String> = f.s.state().code_actions().map(|c| c.actions.iter().map(|a| format!("{} [{}]", a.title, a.unavailable.as_deref().unwrap_or("ok"))).collect()).unwrap_or_default();
    println!("quick fixes: {titles:?}; notices {:?}", f.notices());
    assert_eq!(titles, ["Convert Tab to Spaces [ok]", "Convert all Tabs to Spaces [ok]"]);
    f.s.dispatch(Command::SelectCodeAction(0));
    f.s.dispatch(Command::ApplyCodeAction);
    assert_eq!(current_text(f.s.state()), "a:\n    b: 1\n");
    let tabs = f.tab();
    f.s.dispatch(Command::Undo { tab: tabs });
    assert_eq!(current_text(f.s.state()), YAML_TABS);
    // A remote schema: refused, said in the file's problems, and nothing asked of the network.
    f.show("remote.yaml");
    f.problems_settle(1);
    let refused = f.problem_texts();
    println!("remote: {refused:?}");
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0].1, "Unable to load schema from 'http://schemas.example.invalid/remote.json': Lenz does not download schemas; use a local schema file, or set `remoteSchemas` to true in this provider's settings.");
    let details = f.s.language_server_details();
    assert!(details.contains("Schemas: local files only (1 schema mapping in providers.json, `$schema` keys and modelines in files); remote schemas and the SchemaStore catalog are not downloaded"), "{details}");
    assert!(details.contains("Tested with Lenz: yaml-language-server 2.0.0."), "{details}");
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(*heard.lock().unwrap(), Vec::<String>::new(), "no download was attempted");
    assert_eq!(files(&f.root), before, "nothing written into the project");
    for (name, text) in [("config/app.yaml", YAML_APP), ("other/modeline.yaml", YAML_MODELINE), ("remote.yaml", YAML_REMOTE), ("tabs.yaml", YAML_TABS)] {
        assert_eq!(std::fs::read_to_string(f.root.join(name)).unwrap(), text, "{name} is untouched on disk");
    }
}

/// With `remoteSchemas` turned on, the same remote schema is downloaded (through the server's proxy) and applied:
/// the proof that the refusal above is Lenz's, not a failed network.
#[test]
#[ignore = "needs an installed yaml-language-server and Node.js; run alone"]
fn real_yaml_downloads_a_remote_schema_only_when_allowed() {
    let Some((exe, path)) = installed("yaml-language-server", "YAML_LANGUAGE_SERVER") else {
        println!("SKIPPED: yaml-language-server is not installed");
        return;
    };
    let (proxy, heard) = recording_proxy("http://schemas.example.invalid/remote.json", YAML_REMOTE_SCHEMA);
    let mut f = yaml_fixture(&exe, path.as_deref(), &proxy, json!({ "remoteSchemas": true }));
    f.open_ready("remote.yaml");
    f.problems_settle(1);
    assert_eq!(f.problem_texts(), [("Error".into(), "Incorrect type. Expected \"string\".".into(), "1".into())]);
    assert_eq!(*heard.lock().unwrap(), ["GET http://schemas.example.invalid/remote.json HTTP/1.1"]);
    let details = f.s.language_server_details();
    assert!(details.contains("the server downloads remote schemas, as its provider settings allow"), "{details}");
    assert_eq!(std::fs::read_to_string(f.root.join("remote.yaml")).unwrap(), YAML_REMOTE);
}

/// Without yaml-language-server: a YAML file says it is missing and how to install it (Lenz never installs it), and
/// editing goes on with YAML's own indentation.
#[test]
fn a_missing_yaml_language_server_says_how_to_install_it_and_editing_goes_on() {
    let text = "server:\n";
    let mut f = real("yaml-language-server", Path::new("/nonexistent/yaml-language-server"), None, &[("app.yaml", text)]);
    let path = f.root.join("app.yaml");
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.s.dispatch(Command::Open(path.clone()));
    f.settle("missing", Duration::from_secs(30), |s| s.state().provider().is_some_and(|v| matches!(v.status, Status::Missing(_))));
    let details = f.s.language_server_details();
    assert!(details.contains("npm install -g yaml-language-server") && details.contains("never installs"), "{details}");
    f.caret("server:", "server:".len());
    f.type_at_caret("\n");
    f.type_at_caret("port: 80");
    assert_eq!(current_text(f.s.state()), "server:\n  port: 80\n");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
}

/// A tool written at test time (from another process, so no descriptor of this one keeps it open for writing).
fn shell_tool(path: &Path, script: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let ok = std::process::Command::new("/bin/sh").args(["-c", "printf '%s' \"$1\" > \"$2\" && chmod +x \"$2\"", "fixture"]).arg(script).arg(path).status().unwrap();
    assert!(ok.success());
}

impl Real {
    /// The problems shown, with their codes and the text they cover.
    fn coded_problems(&self) -> Vec<(String, String, String)> {
        let text = current_text(self.s.state());
        self.s.state().diagnostics(self.tab()).map(|d| d.items.iter().map(|p| (p.severity.label().to_owned(), p.code.clone().unwrap_or_default(), text[p.range.clone()].to_owned())).collect()).unwrap_or_default()
    }
    fn fix_titles(&self) -> Vec<String> {
        self.s.state().code_actions().map(|c| c.actions.iter().map(|a| format!("{} [{}]", a.title, a.unavailable.as_deref().unwrap_or("ok"))).collect()).unwrap_or_default()
    }
}

fn shellcheck_current(s: &Session) -> bool {
    s.state().current_source().is_some_and(|doc| s.current_shell_lint().and_then(|lint| lint.diagnostics.as_ref()).is_some_and(|found| found.describes(doc)))
}

/// ShellCheck (a stand-in recording its arguments) checks a script as the shell Lenz identifies (here from the
/// project's `.shellcheckrc` for a script without a shebang, and from a shebang), on standard input; its problem is
/// placed exactly after a character outside the BMP, and its fix applies as one Undo step, from the Quick Fix list.
/// Server Details names the tool, its version and the shell. Files on disk never change.
#[test]
fn shellcheck_checks_as_the_identified_shell_and_its_fix_applies_as_one_undo_step() {
    let tools = tempfile::tempdir().unwrap();
    let log = tools.path().join("calls.log");
    let report = r#"{"comments":[{"file":"-","line":2,"endLine":2,"column":12,"endColumn":14,"level":"info","code":2086,"message":"Double quote to prevent globbing and word splitting.","fix":{"replacements":[{"line":2,"endLine":2,"column":12,"endColumn":12,"insertionPoint":"afterEnd","precedence":7,"replacement":"\""},{"line":2,"endLine":2,"column":14,"endColumn":14,"insertionPoint":"beforeStart","precedence":7,"replacement":"\""}]}}]}"#;
    let shellcheck = tools.path().join("bin/shellcheck");
    shell_tool(&shellcheck, &format!("#!/bin/sh\nif [ \"$1\" = --version ]; then printf 'ShellCheck - stand-in\\nversion: 0.0.1-test\\n'; exit 0; fi\nprintf '%s\\n' \"$* in $(pwd)\" >> '{}'\ncat > /dev/null\nprintf '%s' '{report}'\nexit 1\n", log.display()));
    let script = "greet() {\n  echo \"😀\" $1\n}\n";
    let bash = "#!/bin/bash\necho hi\n";
    let mut f = real_with("bash-language-server", Path::new("/nonexistent/bash-language-server"), None, json!({ "shellcheckPath": shellcheck, "shfmt": { "path": "" } }), &[(".shellcheckrc", "shell=sh\n"), ("run.sh", script), ("b.sh", bash)]);
    let before = files(&f.root);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.s.dispatch(Command::Open(f.root.join("run.sh")));
    f.settle("ShellCheck checked", Duration::from_secs(30), shellcheck_current);
    assert_eq!(f.coded_problems(), [("Info".into(), "SC2086".into(), "$1".into())]);
    let calls = std::fs::read_to_string(&log).unwrap();
    assert_eq!(calls.lines().count(), 1, "one check of the version shown: {calls}");
    assert!(calls.starts_with(&format!("--format=json1 --shell=sh --external-sources --source-path={} - in {}", f.root.display(), f.root.display())), "{calls}");
    let details = f.s.language_server_details();
    println!("{details}");
    for row in [format!("Shell: sh, from `shell=` in {}.", f.root.join(".shellcheckrc").display()), format!("Problems: ShellCheck 0.0.1-test ({}), checking as sh;", shellcheck.display()),
        "Format Document: shfmt is turned off (its path is empty in the provider settings)".into(), "npm install -g bash-language-server".into()] {
        assert!(details.contains(&row), "{row}\n{details}");
    }
    // ShellCheck's fix, then disabling its rule for the file; the fix is one Undo step.
    let tab = f.tab();
    f.caret("$1", 1);
    f.s.dispatch(Command::QuickFix);
    assert_eq!(f.fix_titles(), ["Apply fix for SC2086: Double quote to prevent globbing and word splitting. [ok]", "Disable ShellCheck rule SC2086 for the entire file [ok]"]);
    f.s.dispatch(Command::SelectCodeAction(0));
    f.s.dispatch(Command::ApplyCodeAction);
    assert_eq!(current_text(f.s.state()), script.replace("$1", "\"$1\""));
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), script);
    // Disabling the rule inserts a file-wide directive before the first command.
    f.caret("$1", 0);
    f.settle("ShellCheck checked again", Duration::from_secs(30), shellcheck_current);
    f.s.dispatch(Command::QuickFix);
    f.s.dispatch(Command::SelectCodeAction(1));
    f.s.dispatch(Command::ApplyCodeAction);
    assert_eq!(current_text(f.s.state()), format!("# shellcheck disable=SC2086\n{script}"));
    f.s.dispatch(Command::Undo { tab });
    // A shebang names the shell of another script.
    f.show("b.sh");
    f.settle("bash checked", Duration::from_secs(30), |_| std::fs::read_to_string(&log).is_ok_and(|c| c.contains("--shell=bash")));
    assert_eq!(files(&f.root), before, "nothing written or created in the project");
    assert_eq!(std::fs::read_to_string(f.root.join("run.sh")).unwrap(), script);
}

/// With bash-language-server, ShellCheck and shfmt all missing: Server Details names each one missing and how to
/// install it (Lenz never installs them), the problems list and Quick Fix say ShellCheck is missing, Format Document
/// that shfmt is, Organize Imports is disabled for shell scripts, and typing, Undo and Save go on.
#[test]
fn a_missing_shell_toolchain_says_what_is_missing_and_editing_goes_on() {
    let text = "#!/bin/sh\necho hi\n";
    let mut f = real_with("bash-language-server", Path::new("/nonexistent/bash-language-server"), None, json!({ "shellcheckPath": "/nonexistent/shellcheck", "shfmt": { "path": "/nonexistent/shfmt" } }), &[("run.sh", text)]);
    let path = f.root.join("run.sh");
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.s.dispatch(Command::Open(path.clone()));
    f.settle("missing", Duration::from_secs(30), |s| s.state().provider().is_some_and(|v| matches!(v.status, Status::Missing(_))) && s.language_server_details().contains("Problems: "));
    let details = f.s.language_server_details();
    println!("{details}");
    for row in ["npm install -g bash-language-server", "Shell: sh, from the shebang `#!/bin/sh`.", "Problems: ShellCheck is missing (/nonexistent/shellcheck does not exist), so no ShellCheck diagnostics are available. Install it with your package manager (`apt install shellcheck`",
        "Format Document: shfmt is missing (/nonexistent/shfmt does not exist). Install it", "Lenz never installs it"] {
        assert!(details.contains(row), "{row}\n{details}");
    }
    assert!(f.s.problems_details().starts_with("ShellCheck is missing (/nonexistent/shellcheck does not exist), so no ShellCheck diagnostics are available."), "{}", f.s.problems_details());
    f.caret("echo", 0);
    f.s.dispatch(Command::QuickFix);
    assert!(f.notices().last().is_some_and(|n| n.starts_with("Quick fixes: ShellCheck is missing")), "{:?}", f.notices());
    f.s.dispatch(Command::Format(code_core::session::FormatKind::Format));
    f.settle("format refused", Duration::from_secs(30), |s| s.state().notices().iter().any(|n| n.starts_with("Format Document: shfmt is missing (/nonexistent/shfmt does not exist).")));
    assert_eq!(f.s.disabled(code_core::session::CommandId::OrganizeImports), Some("Shell scripts have no imports to organize"));
    let tab = f.tab();
    f.type_at_caret("# edited\n");
    let edited = text.replacen("echo", "# edited\necho", 1);
    assert_eq!(current_text(f.s.state()), edited);
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), text);
    f.s.dispatch(Command::Redo { tab });
    f.s.dispatch(Command::Save { tab });
    f.settle("saved", Duration::from_secs(30), |_| std::fs::read_to_string(&path).is_ok_and(|t| t == edited));
}

const SH_BODY: &str = "# 😀 deploy\ngreeting=\"héllo\"\ngreet() {\n  local name=$1\n  if [[ -n $name ]]; then\n    echo \"😀 $greeting\" $name\n  fi\n  case $name in\n  w*) echo w ;;\n  esac\n}\ngreet \"wörld\"\n";
const SH_FORMATTED: &str = "# 😀 deploy\ngreeting=\"héllo\"\ngreet() {\n   local name=$1\n   if [[ -n $name ]]; then\n      echo \"😀 $greeting\" $name\n   fi\n   case $name in\n      w*) echo w ;;\n   esac\n}\ngreet \"wörld\"\n";
const SH_EDITORCONFIG: &str = "root = true\n[*.{sh,bash}]\nindent_style = space\nindent_size = 3\nswitch_case_indent = true\n";

/// The installed shell tools, as the real tests need them: bash-language-server (with the `PATH` to run it), and
/// ShellCheck and shfmt on `PATH`. `None` (skipped) unless all three are there.
fn shell_tools() -> Option<(PathBuf, Option<String>)> {
    let (exe, path) = installed("bash-language-server", "BASH_LANGUAGE_SERVER")?;
    let on_path = |name: &str| code_core::semantics::provider::find_executable(Path::new(name), path.as_deref().map(std::ffi::OsStr::new));
    (on_path("shellcheck").is_some() && on_path("shfmt").is_some()).then_some((exe, path))
}

/// bash-language-server with ShellCheck and shfmt, which Lenz runs itself, on a Bash script and an sh script with the
/// same body: each gets its own shell's problems at exact ranges (after characters outside the BMP), a script without
/// a shebang the shell of its project's `.shellcheckrc`; variable and function completion, each one Undo step, and
/// Bash's builtins offered to the Bash script only;
/// ShellCheck's quick fix as one Undo step; Format Document with `.editorconfig`'s size and `switch_case_indent`, one
/// Undo step, refused for an sh script with a Bash array (shfmt as sh); Server Details names each tool, its version
/// and the shell. Nothing is written to the project and no script runs.
#[test]
#[ignore = "needs bash-language-server (Node.js), ShellCheck and shfmt; run alone"]
fn real_shell_editing() {
    let Some((exe, path)) = shell_tools() else {
        println!("SKIPPED: bash-language-server, shellcheck or shfmt is not installed");
        return;
    };
    let bash = format!("#!/usr/bin/env bash\n{SH_BODY}");
    let sh = format!("#!/bin/sh\n{SH_BODY}");
    // Each script would leave a file behind if it ran.
    let util = "helper() {\n  local x=1\n  echo \"$x\" > ran-util\n}\n";
    let arrays = "#!/bin/sh\nlist=(a b)\necho \"${list[0]}\" > ran-arrays\n";
    let mut f = real_with("bash-language-server", &exe, path.as_deref(), Value::Null, &[(".editorconfig", SH_EDITORCONFIG), ("deploy.sh", &bash), ("posix.sh", &sh), ("lib/.shellcheckrc", "shell=sh\n"), ("lib/util.sh", util), ("arrays.sh", arrays)]);
    let before = files(&f.root);
    f.open_ready("deploy.sh");
    let tab = f.tab();
    let view = f.s.state().provider().unwrap();
    println!("bash-language-server {:?} (untested: {}); capabilities {:?}", view.version, view.untested, view.supports);
    assert_eq!((view.version.as_deref(), view.untested), (Some("bash-language-server 5.8.1"), false));
    // Bash: only the unquoted `$name` after `😀`.
    f.problems_settle(1);
    assert_eq!(f.coded_problems(), [("Info".into(), "SC2086".into(), "$name".into())]);
    let name = bash.find("\" $name").unwrap() + 2;
    assert_eq!(f.s.state().diagnostics(tab).unwrap().items[0].range, name..name + 5, "after `😀`, placed exactly");
    let details = f.s.language_server_details();
    println!("{details}");
    for row in ["Server version: bash-language-server 5.8.1", "Shell: Bash, from the shebang `#!/usr/bin/env bash`.", "Problems: ShellCheck 0.9.0 (", "checking as bash;", "Format Document: shfmt 3.8.0 (", "as `-ln=bash`", "Lenz itself provides quick fixes and formatting (below)"] {
        assert!(details.contains(row), "{row}\n{details}");
    }
    // A variable, then a function, from the script's own declarations; each one Undo step after its typing.
    f.caret("greet \"wörld\"\n", "greet \"wörld\"\n".len());
    f.type_at_caret("echo \"$gree");
    let typed = current_text(f.s.state());
    let row = f.complete_until("greeting");
    let rows: Vec<String> = f.s.state().completion().unwrap().rows().map(|r| r.label.clone()).collect();
    println!("variables: {rows:?}");
    f.s.dispatch(Command::SelectCompletion(row));
    f.s.dispatch(Command::AcceptCompletion);
    f.settle("variable", Duration::from_secs(30), |s| current_text(s.state()).ends_with("echo \"$greeting"));
    assert_eq!(current_text(f.s.state()), format!("{bash}echo \"$greeting"));
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), typed);
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), bash);
    f.type_at_caret("gre");
    let row = f.complete_until("greet");
    f.s.dispatch(Command::SelectCompletion(row));
    f.s.dispatch(Command::AcceptCompletion);
    f.settle("function", Duration::from_secs(30), |s| current_text(s.state()).ends_with("\ngreet"));
    assert_eq!(current_text(f.s.state()), format!("{bash}greet"));
    for _ in 0..2 { if current_text(f.s.state()) != bash { f.s.dispatch(Command::Undo { tab }); } }
    assert_eq!(current_text(f.s.state()), bash);
    // Bash's own builtins are offered to a Bash script (an sh script's below are not).
    let builtins = |f: &mut Real| -> Vec<String> {
        let tab = f.tab();
        let before = current_text(f.s.state());
        f.caret("greet \"wörld\"\n", "greet \"wörld\"\n".len());
        f.type_at_caret("ty");
        f.complete_until("type");
        let rows = f.s.state().completion().unwrap().rows().map(|r| r.label.clone()).collect();
        f.s.dispatch(Command::CloseCompletion);
        f.s.dispatch(Command::Undo { tab });
        assert_eq!(current_text(f.s.state()), before);
        rows
    };
    let offered = builtins(&mut f);
    println!("Bash `ty`: {offered:?}");
    assert!(offered.iter().any(|r| r == "typeset"), "{offered:?}");
    // ShellCheck's fix quotes `$name`, one Undo step.
    f.problems_settle(1);
    f.caret("\" $name", 3);
    f.s.dispatch(Command::QuickFix);
    let titles = f.fix_titles();
    println!("quick fixes: {titles:?}; notices {:?}", f.notices());
    assert_eq!(titles, ["Apply fix for SC2086: Double quote to prevent globbing and word splitting. [ok]", "Disable ShellCheck rule SC2086 for the entire file [ok]"]);
    f.s.dispatch(Command::SelectCodeAction(0));
    f.s.dispatch(Command::ApplyCodeAction);
    assert_eq!(current_text(f.s.state()), bash.replace("\" $name", "\" \"$name\""));
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), bash);
    // shfmt as Bash: `.editorconfig`'s three spaces and indented cases, one Undo step.
    f.s.dispatch(Command::Format(code_core::session::FormatKind::Format));
    f.settle("formatted", Duration::from_secs(60), |s| s.state().asking().is_none() && current_text(s.state()) != bash);
    assert_eq!(current_text(f.s.state()), format!("#!/usr/bin/env bash\n{SH_FORMATTED}"), "{:?}", f.notices());
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), bash);
    // sh: the same body has sh's problems too.
    f.show("posix.sh");
    f.problems_settle(4);
    let found = f.coded_problems();
    println!("sh problems: {found:?}");
    assert_eq!(found, [
        ("Warning".into(), "SC3043".into(), "local name".into()),
        ("Info".into(), "SC2086".into(), "$1".into()),
        ("Warning".into(), "SC3010".into(), "[[ -n $name ]]".into()),
        ("Info".into(), "SC2086".into(), "$name".into()),
    ]);
    let details = f.s.language_server_details();
    for row in ["Shell: sh, from the shebang `#!/bin/sh`.", "checking as sh;", "as `-ln=posix`"] { assert!(details.contains(row), "{row}\n{details}"); }
    let offered = builtins(&mut f);
    println!("sh `ty`: {offered:?}");
    assert!(!offered.iter().any(|r| r == "typeset"), "Bash's `typeset` is not offered to an sh script: {offered:?}");
    // A script without a shebang: its project's `.shellcheckrc` names sh.
    f.show("lib/util.sh");
    f.problems_settle(1);
    assert_eq!(f.coded_problems(), [("Warning".into(), "SC3043".into(), "local x".into())]);
    let details = f.s.language_server_details();
    let rc = format!("Shell: sh, from `shell=` in {}.", f.root.join("lib/.shellcheckrc").display());
    assert!(details.contains(&rc), "{details}");
    // shfmt as sh refuses a Bash array; nothing changes.
    f.show("arrays.sh");
    let seen = f.notices().len();
    f.s.dispatch(Command::Format(code_core::session::FormatKind::Format));
    f.settle("refused", Duration::from_secs(60), move |s| s.state().notices().len() > seen && s.state().asking().is_none());
    let refused = f.notices().last().cloned().unwrap_or_default();
    println!("{refused}");
    assert!(refused.starts_with("Format Document: shfmt failed as sh: ") && refused.contains("arrays are a bash/mksh feature"), "{refused}");
    assert_eq!(current_text(f.s.state()), arrays);
    assert_eq!(files(&f.root), before, "nothing written, and no script ran");
}

/// The `shell` setting of bash-language-server (here per project) names the shell of a script that names none, over
/// the project's `.shellcheckrc`; a zsh script is not given to ShellCheck or shfmt, and both say so.
#[test]
fn the_shell_setting_names_a_scripts_shell_and_zsh_is_left_unchecked() {
    let tools = tempfile::tempdir().unwrap();
    let log = tools.path().join("calls.log");
    let shellcheck = tools.path().join("bin/shellcheck");
    shell_tool(&shellcheck, &format!("#!/bin/sh\nif [ \"$1\" = --version ]; then printf 'version: 0.0.1-test\\n'; exit 0; fi\nprintf '%s\\n' \"$*\" >> '{}'\ncat > /dev/null\nprintf '%s' '{{\"comments\":[]}}'\n", log.display()));
    let shfmt = tools.path().join("bin/shfmt");
    shell_tool(&shfmt, "#!/bin/sh\nif [ \"$1\" = --version ]; then echo v0.0.2-test; exit 0; fi\nexit 3\n");
    let mut f = real_entries(|root| vec![("bash-language-server".into(), json!({ "executable": "/nonexistent/bash-language-server",
        "settings": { "shellcheckPath": shellcheck, "shfmt": { "path": shfmt } }, "projects": { root.to_str().unwrap(): { "shell": "ksh" } } }))],
        &[(".shellcheckrc", "shell=sh\n"), ("run.sh", "echo hi\n"), ("prompt.zsh", "setopt promptsubst\nprint -r -- ${(@kv)options}\n")]);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.s.dispatch(Command::Open(f.root.join("run.sh")));
    f.settle("ShellCheck checked", Duration::from_secs(30), shellcheck_current);
    assert!(std::fs::read_to_string(&log).unwrap().contains("--shell=ksh"));
    let details = f.s.language_server_details();
    for row in ["Shell: ksh, from the `shell` setting of bash-language-server in providers.json.", "Format Document: shfmt 0.0.2-test (", "as `-ln=mksh`"] { assert!(details.contains(row), "{row}\n{details}"); }
    assert_eq!(f.s.problems_details(), "ShellCheck reports no problems in this file.");
    f.show("prompt.zsh");
    f.settle("zsh", Duration::from_secs(30), |s| s.language_server_details().contains("Shell: zsh") && s.state().highlights().is_some());
    let details = f.s.language_server_details();
    for row in ["Shell: zsh, from its name (.zsh).", "Problems: ShellCheck 0.0.1-test (", "did not check this text: ShellCheck does not check zsh scripts", "Format Document: shfmt 0.0.2-test (", "does not format zsh"] {
        assert!(details.contains(row), "{row}\n{details}");
    }
    assert_eq!(f.s.problems_details(), "ShellCheck did not check this text: ShellCheck does not check zsh scripts.");
    assert!(f.s.state().diagnostics(f.tab()).is_none(), "the Bash grammar cannot validate zsh expansions");
    let seen = f.notices().len();
    f.s.dispatch(Command::Format(code_core::session::FormatKind::Format));
    f.settle("refused", Duration::from_secs(30), move |s| s.state().notices().len() > seen);
    assert!(f.notices().last().is_some_and(|n| n.starts_with("Format Document: shfmt does not format zsh scripts")), "{:?}", f.notices());
    assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(), 1, "zsh is never given to ShellCheck");
}

const TPL_GEMFILE: &str = "source \"https://rubygems.org\"\n\ngem \"rubocop\"\n";
const TPL_GREETER: &str = "# frozen_string_literal: true\n\nmodule Greeter\n  def self.greet(name, excited: false)\n    excited ? \"#{name}!\" : name\n  end\nend\n";
const TPL_ERB: &str = "<ul class=\"𝄞\">\r\n<% names = [\"é\", \"𝄞\"] %>\r\n<li title=\"<%= Greeter.greet(\"𝄞\") %>\">𝄞 <%= Greeter.gre %></li>\r\n<li><%= Greeter.greet(names.first, excited: true) %></li>\r\n<% total = [1,\r\n  2].sum %>\r\n<p>𝄞 </p>\r\n<%= total] %>\r\n</ul>\r\n";
const TPL_HAML: &str = "%ul.list(title=\"😀\")\r\n  - names = [\"é\", \"𝄞\"]\r\n  %li= Greeter.greet(\"𝄞\")\r\n  %li= Greeter.gre\r\n  %li Hi #{Greeter.greet(names.first, excited: true)} 𝄞\r\n  %li= total]\r\n";

/// A bundled project (RuboCop in its Gemfile) with a helper module and the template `name`: its servers ready.
fn template_project(name: &str, text: &str, html: bool) -> Option<Real> {
    let Some((exe, path)) = installed("ruby-lsp", "RUBY_LSP") else { println!("SKIPPED: Ruby LSP is not installed"); return None };
    let mut servers = vec![("ruby-lsp", exe, path, Value::Null)];
    if html {
        let Some(node) = web_server("vscode-html") else { println!("SKIPPED: no VS Code HTML server"); return None };
        servers.push(("vscode-html", PathBuf::new(), node, Value::Null));
    }
    let servers: Vec<(&str, &Path, Option<&str>, Value)> = servers.iter().map(|(id, exe, path, settings)| (*id, exe.as_path(), path.as_deref(), settings.clone())).collect();
    let files = [("Gemfile", TPL_GEMFILE), (".rubocop.yml", "AllCops:\n  NewCops: disable\n  SuggestExtensions: false\n"), ("lib/greeter.rb", TPL_GREETER), (name, text)];
    let mut f = real_many(&servers, &files);
    let lock = std::process::Command::new("bundle").args(["lock", "--local"]).current_dir(&f.root).output().unwrap();
    assert!(lock.status.success(), "{}", String::from_utf8_lossy(&lock.stderr));
    f.open_ready(name);
    Some(f)
}

impl Real {
    /// Ask for parameter information at `needle` (+`skip` bytes): its labels and active parameter.
    fn parameters_at(&mut self, needle: &str, skip: usize) -> Option<(Vec<String>, Option<usize>)> {
        self.caret(needle, skip);
        let seen = self.notices().len();
        self.s.dispatch(Command::ParameterInfo);
        self.settle("parameter information", Duration::from_secs(30), move |s| s.state().signature_help().is_some() || s.state().notices().len() > seen);
        let found = self.s.state().signature_help().map(|h| (h.signatures.iter().map(|s| s.label.clone()).collect(), h.active_signature().and_then(|s| s.active_parameter)));
        println!("parameters at {needle:?}+{skip}: {found:?}; {:?}", self.notices().last());
        self.s.dispatch(Command::CloseSignature);
        found
    }

    /// Complete `label` at the caret and wait until the draft reads `expected`; one Undo restores `before`.
    fn complete_into(&mut self, label: &str, expected: &str, before: &str) {
        let row = self.complete_until(label);
        self.s.dispatch(Command::SelectCompletion(row));
        self.s.dispatch(Command::AcceptCompletion);
        let want = expected.to_owned();
        self.settle("accepted", Duration::from_secs(30), move |s| current_text(s.state()) == want);
        let tab = self.tab();
        self.s.dispatch(Command::Undo { tab });
        assert_eq!(current_text(self.s.state()), before, "one Undo takes the suggestion back");
    }
}

/// Ruby LSP and the HTML server through an ERB template (ticket 57): its Ruby goes to Ruby LSP as a projection under
/// the template's URI and version, its markup to the HTML server with the Ruby tags hidden. Ruby's syntax problem at
/// the template's own `]` (after CRLF, non-BMP and multiline Ruby), no "void context" warning about an output tag;
/// `Greeter.gre` completed to `greet` inside its tag and parameter information inside a call (UTF-16, after `𝄞`);
/// a tag name completed and a typed tag closed in the markup; Format Document says no ERB formatter is configured. Each change one Undo step; the files on disk unchanged.
#[test]
#[ignore = "needs installed ruby-lsp and rubocop gems and vscode-html-language-server (Node.js); run alone"]
fn real_erb_template_editing() {
    let name = "app/views/greet.html.erb";
    let Some(mut f) = template_project(name, TPL_ERB, true) else { return };
    f.problems_settle(2);
    let problems = f.problem_texts();
    println!("problems: {problems:?}");
    assert_eq!(problems, [("Error".to_owned(), "unexpected ']', expecting end-of-input".to_owned(), "]".to_owned()), ("Error".into(), "unexpected ']', ignoring it".into(), "]".into())]);
    let close = TPL_ERB.find("total]").unwrap() + 5;
    assert_eq!(f.s.state().diagnostics(f.tab()).unwrap().items[0].range, close..close + 1, "the template's own `]` byte");
    f.caret("Greeter.gre %>", 11);
    f.complete_into("greet", &TPL_ERB.replacen("Greeter.gre %>", "Greeter.greet %>", 1), TPL_ERB);
    let label = vec!["greet(name, excited: <default>)".to_owned()];
    assert_eq!(f.parameters_at("first, excited", 7), Some((label.clone(), Some(1))), "the keyword parameter");
    assert_eq!(f.parameters_at("greet(\"𝄞\")", 7), Some((label, Some(0))), "inside the string, on a line with `𝄞`");
    // Markup: a tag name from the HTML server's suggestions, typed after `𝄞`.
    f.caret("<p>𝄞 ", "<p>𝄞 ".len());
    f.type_at_caret("<sp");
    let typed = TPL_ERB.replacen("<p>𝄞 ", "<p>𝄞 <sp", 1);
    f.complete_into("span", &TPL_ERB.replacen("<p>𝄞 ", "<p>𝄞 <span", 1), &typed);
    let tab = f.tab();
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), TPL_ERB);
    // A typed `<b>` in the markup is closed by the HTML server, its own Undo step.
    f.caret("</ul>", 0);
    for c in "<b>".chars() { f.type_at_caret(&c.to_string()); }
    let closed = TPL_ERB.replacen("</ul>", "<b></b></ul>", 1);
    f.settle("closing tag", Duration::from_secs(30), move |s| current_text(s.state()) == closed);
    for _ in 0..4 { f.s.dispatch(Command::Undo { tab }); }
    assert_eq!(current_text(f.s.state()), TPL_ERB);
    // Templates are formatted by a configured erb_lint (ticket 58), and this project configures none.
    f.s.dispatch(Command::Format(code_core::session::FormatKind::Format));
    f.settle("formatted", Duration::from_secs(60), |s| s.state().notices().iter().any(|n| n.starts_with("Format Document")));
    assert_eq!(f.notices().last().map(String::as_str), Some("Format Document: no ERB formatter is configured for this project (an .erb_lint.yml for erb_lint); Ruby LSP does not format templates, and Lenz never installs formatters"));
    assert_eq!(current_text(f.s.state()), TPL_ERB);
    let details = f.s.language_server_details();
    assert!(details.contains("ERB template Ruby: 6 regions mapped exactly to the template") && details.contains("ERB template markup: 7 regions"), "{details}");
    for (name, text) in [(name, TPL_ERB), ("lib/greeter.rb", TPL_GREETER)] { assert_eq!(std::fs::read_to_string(f.root.join(name)).unwrap(), text, "{name} is untouched on disk"); }
}

/// Ruby LSP through a Haml template (ticket 57): the Ruby of its scripts and interpolation, mapped line by line. The
/// syntax problem at the template's `]`, `Greeter.gre` completed in a `%li=` script, parameter information inside an
/// interpolation; the markup is never sent as HTML and a caret there says so.
#[test]
#[ignore = "needs installed ruby-lsp and rubocop gems; run alone"]
fn real_haml_template_editing() {
    let name = "app/views/list.html.haml";
    let Some(mut f) = template_project(name, TPL_HAML, false) else { return };
    f.problems_settle(1);
    let problems = f.problem_texts();
    println!("problems: {problems:?}");
    assert!(!problems.is_empty() && problems.iter().all(|(severity, _, text)| severity == "Error" && text == "]"), "{problems:?}");
    f.caret("Greeter.gre\r", 11);
    f.complete_into("greet", &TPL_HAML.replacen("Greeter.gre\r", "Greeter.greet\r", 1), TPL_HAML);
    assert_eq!(f.parameters_at("first, excited", 7), Some((vec!["greet(name, excited: <default>)".to_owned()], Some(1))));
    f.caret("ul.list", 1);
    f.s.dispatch(Command::Complete);
    assert!(f.s.state().completion().is_some_and(|c| c.rows().any(|r| r.label == "ul")), "Haml tags are suggested locally");
    f.s.dispatch(Command::CloseCompletion);
    assert!(f.s.language_server_details().contains("Haml template Ruby: 5 regions mapped exactly to the template"), "{}", f.s.language_server_details());
    assert_eq!(std::fs::read_to_string(f.root.join(name)).unwrap(), TPL_HAML);
}

/// A Rails view (CRLF, non-BMP text, a block over an instance variable, a helper call, a syntax error), a partial with
/// strict locals and a layout yielding, in the Rails fixture with erb_lint in its bundle.
const RAILS_VIEW: &str = "<h1 title=\"𝄞\">Posts 𝄞</h1>\r\n<% @posts.each do |post| %>\r\n  <p>é <%= link_to(\"é𝄞\", posts_path, class: \"x\") %></p>\r\n  <%=post_link(post)%>\r\n  <%= link_to \"Edit\", edit_post_path(post) %>\r\n  <%= link_t %>\r\n  <%= post_l %>\r\n<% end %>\r\n<%= [1,\r\n  2].sum] %>\r\n<p>𝄞 </p>\r\n";
const RAILS_PARTIAL: &str = "<%# locals: (post:, note: \"𝄞\") -%>\n<p><%= no %> <%=link_to post.title, post_path(post)%></p>\n";
const RAILS_LAYOUT: &str = "<!DOCTYPE html>\n<html><body><%= yield %></body></html>\n";
const ERB_LINT_DEFAULTS: &str = "---\nEnableDefaultLinters: true\n";

/// Whether erb_lint is installed (it runs from the fixture's bundle).
fn erb_lint_missing() -> bool {
    let listed = std::process::Command::new("gem").args(["list", "--local", "erb_lint"]).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    !listed.lines().any(|l| l.starts_with("erb_lint ("))
}

/// The Rails fixture with views, erb_lint in its bundle and an `.erb_lint.yml`, Ruby LSP (with its Rails add-on) and
/// the HTML server: its index view open with its servers ready. `None` (skipped) when a tool is missing.
fn rails_view_project() -> Option<Real> {
    let Some((exe, path)) = installed("ruby-lsp", "RUBY_LSP") else { println!("SKIPPED: Ruby LSP is not installed"); return None };
    let missing = rails_gems_missing();
    if !missing.is_empty() || erb_lint_missing() { println!("SKIPPED: gems not installed: {missing:?} (erb_lint: {})", erb_lint_missing()); return None; }
    let Some(node) = web_server("vscode-html") else { println!("SKIPPED: no VS Code HTML server"); return None };
    let gemfile = format!("{}gem \"erb_lint\"\n", RAILS_APP[0].1);
    let mut files: Vec<(&str, &str)> = RAILS_APP.iter().skip(1).copied().collect();
    files.extend([("Gemfile", gemfile.as_str()), (".erb_lint.yml", ERB_LINT_DEFAULTS), ("app/views/posts/index.html.erb", RAILS_VIEW), ("app/views/posts/_post.html.erb", RAILS_PARTIAL), ("app/views/layouts/application.html.erb", RAILS_LAYOUT)]);
    let mut f = real_many(&[("ruby-lsp", &exe, path.as_deref(), Value::Null), ("vscode-html", Path::new(""), node.as_deref(), Value::Null)], &files);
    rails_setup(&f.root);
    f.open_ready("app/views/posts/index.html.erb");
    Some(f)
}

impl Real {
    /// Ask for parameter information at `needle` (+`skip`) until it answers (the gem index may still be loading).
    fn parameters_until(&mut self, needle: &str, skip: usize) -> Option<(Vec<String>, Option<usize>)> {
        let asked = Instant::now();
        loop {
            let found = self.parameters_at(needle, skip);
            if found.is_some() || asked.elapsed() > Duration::from_secs(30) { return found; }
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    /// Ask for the definition at `needle` in `from` until it opens `file` (a booting add-on answers nothing at first).
    fn definition_at_until(&mut self, needle: &str, skip: usize, from: &str, file: &str) {
        let asked = Instant::now();
        loop {
            self.definition(needle, skip);
            if file_of(&self.s) == file { return; }
            println!("definition at {needle:?} at {:.1} s: {}; {:?}", asked.elapsed().as_secs_f64(), file_of(&self.s), self.notices().last());
            assert!(asked.elapsed() < Duration::from_secs(60), "{:?}", self.notices());
            self.show(from);
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    /// Format Document; the notice it ends with.
    fn format_notice(&mut self) -> String {
        let seen = self.notices().iter().filter(|n| n.starts_with("Format Document:")).count();
        self.s.dispatch(Command::Format(code_core::session::FormatKind::Format));
        self.settle("formatted", Duration::from_secs(60), move |s| s.state().notices().iter().filter(|n| n.starts_with("Format Document:")).count() > seen);
        self.notices().into_iter().rfind(|n| n.starts_with("Format Document:")).unwrap()
    }
}

/// Ruby LSP with its Rails add-on through Rails views (ticket 58): the view's Ruby runs as Action View with the
/// application's and its controller's helpers, so Ruby LSP completes `link_to` and the app's own `post_link` and
/// answers their parameter information, documentation and definitions (and the add-on a route helper's route); a
/// partial's strict locals complete; a layout's `yield` is no error; Ruby's syntax problem lands on the template's `]`
/// after CRLF and non-BMP text; the HTML server completes and closes tags in the markup; Format Document runs the
/// project's erb_lint from its bundle (its failure on CRLF with its default linters said, the CRLF line ends kept
/// otherwise). Each change one Undo step; the files on disk unchanged.
#[test]
#[ignore = "needs installed ruby-lsp, ruby-lsp-rails, rails, sqlite3, rubocop, rubocop-rails and erb_lint gems and vscode-html-language-server (Node.js); run alone"]
fn real_rails_view_editing() {
    let Some(mut f) = rails_view_project() else { return };
    let view = "app/views/posts/index.html.erb";
    let before = files(&f.root);
    let tab = f.tab();
    f.problems_settle(1);
    let problems = f.problem_texts();
    println!("problems: {problems:?}");
    assert!(!problems.is_empty() && problems.iter().all(|(severity, _, text)| severity == "Error" && text == "]"), "{problems:?}");
    // Action View's own helper and the application's.
    f.caret("link_t %>", 6);
    f.complete_into("link_to", &RAILS_VIEW.replacen("link_t %>", "link_to %>", 1), RAILS_VIEW);
    f.caret("post_l %>", 6);
    f.complete_into("post_link", &RAILS_VIEW.replacen("post_l %>", "post_link %>", 1), RAILS_VIEW);
    let link_to = "link_to(name = <default>, options = <default>, html_options = <default>, &block)".to_owned();
    assert_eq!(f.parameters_until("posts_path, class", 12), Some((vec![link_to.clone()], Some(2))), "html_options, after `é𝄞` on a CRLF line");
    assert_eq!(f.parameters_until("post_link(post)", 10), Some((vec!["post_link(post)".to_owned()], Some(0))));
    f.caret("link_to(\"é", 2);
    let asked = Instant::now();
    let documentation = loop {
        let seen = f.notices().len();
        f.s.dispatch(Command::Documentation);
        f.settle("documentation", Duration::from_secs(30), move |s| s.state().documentation().is_some() || s.state().notices().len() > seen);
        let text = f.s.state().documentation().map(|d| d.preview.doc.text.clone()).unwrap_or_default();
        f.s.dispatch(Command::CloseDocumentation);
        if !text.is_empty() || asked.elapsed() > Duration::from_secs(30) { break text; }
        std::thread::sleep(Duration::from_millis(500));
    };
    assert!(documentation.contains("Creates an anchor element"), "{documentation}");
    // The add-on's route (once the application has booted), and the application's helper method.
    f.definition_at_until("edit_post_path(post)", 2, view, "routes.rb");
    assert_eq!((file_of(&f.s), f.s.state().current_source().unwrap().line_of(f.s.state().selection().unwrap().head)), ("routes.rb".into(), 1));
    f.show(view);
    f.definition_at_until("post_link(post)", 2, view, "posts_helper.rb");
    assert_eq!((file_of(&f.s), f.s.state().current_source().unwrap().line_of(f.s.state().selection().unwrap().head)), ("posts_helper.rb".into(), 3));
    f.show(view);
    // Ruby LSP 0.26.11 (with these add-ons) answers no definition after non-ASCII text on the caret's line, in a
    // `.rb` file as here (reproduced by a JSON-RPC probe without Lenz): the server's limit, said.
    f.definition("posts_path, class", 2);
    assert_eq!(file_of(&f.s), "index.html.erb");
    assert!(f.notices().last().is_some_and(|n| n.starts_with("Definition: Ruby LSP found no definition here")), "{:?}", f.notices().last());
    // The controller's instance variables are not known to Ruby LSP in a view (said in Server Details).
    f.caret("@posts.each", 3);
    let seen = f.notices().len();
    f.s.dispatch(Command::Complete);
    f.settle("instance variables", Duration::from_secs(30), move |s| s.state().completion().is_some() || s.state().notices().len() > seen);
    let rows: Vec<String> = f.s.state().completion().map(|c| c.rows().map(|r| r.label.clone()).collect()).unwrap_or_default();
    println!("instance variables: {rows:?}; {:?}", f.notices().last());
    assert!(!rows.iter().any(|r| r == "@posts"), "{rows:?}");
    f.s.dispatch(Command::CloseCompletion);
    let details = f.s.language_server_details();
    println!("details:\n{details}");
    assert!(details.contains("ERB template view context: a Rails view (Action View), with ApplicationHelper and PostsHelper; controller instance variables and route helpers are not known to Ruby LSP's index"), "{details}");
    assert!(details.contains(&format!("Format Document runs the project's erb_lint, not a language server: erb_lint (configured by {}); run with --autocorrect by `bundle exec` in the bundle of {}", f.root.join(".erb_lint.yml").display(), f.root.join("Gemfile").display())), "{details}");
    // Markup: a tag completed after `𝄞` on a CRLF line, and a typed one closed, each its own Undo step.
    f.caret("<p>𝄞 </p>", "<p>𝄞 ".len());
    f.type_at_caret("<sp");
    let typed = RAILS_VIEW.replacen("<p>𝄞 </p>", "<p>𝄞 <sp</p>", 1);
    f.complete_into("span", &RAILS_VIEW.replacen("<p>𝄞 </p>", "<p>𝄞 <span</p>", 1), &typed);
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), RAILS_VIEW);
    f.caret("</h1>", 0);
    for c in "<b>".chars() { f.type_at_caret(&c.to_string()); }
    let closed = RAILS_VIEW.replacen("</h1>", "<b></b></h1>", 1);
    f.settle("closing tag", Duration::from_secs(30), move |s| current_text(s.state()) == closed);
    for _ in 0..4 { f.s.dispatch(Command::Undo { tab }); }
    assert_eq!(current_text(f.s.state()), RAILS_VIEW);
    // erb_lint 0.9.0's TrailingWhitespace linter fails on CRLF files: said, nothing changed.
    let notice = f.format_notice();
    println!("format with the default linters: {notice}");
    assert!(notice.starts_with("Format Document: erb_lint could not process this template (begin_pos location ") && notice.ends_with("); nothing changed"), "{notice}");
    assert_eq!(current_text(f.s.state()), RAILS_VIEW);
    // Without it: corrected, with the file's CRLF line ends.
    write(&f.root, ".erb_lint.yml", "---\nEnableDefaultLinters: true\nlinters:\n  TrailingWhitespace:\n    enabled: false\n");
    assert_eq!(f.format_notice(), "Format Document: done with erb_lint");
    assert_eq!(current_text(f.s.state()), RAILS_VIEW.replacen("<%=post_link(post)%>", "<%= post_link(post) %>", 1));
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), RAILS_VIEW, "one Undo takes it back");
    // The partial: its strict locals complete; erb_lint corrects it.
    let partial = "app/views/posts/_post.html.erb";
    f.show(partial);
    f.caret("no %>", 2);
    f.complete_into("note", &RAILS_PARTIAL.replacen("no %>", "note %>", 1), RAILS_PARTIAL);
    assert_eq!(f.format_notice(), "Format Document: done with erb_lint");
    assert_eq!(current_text(f.s.state()), RAILS_PARTIAL.replacen("<%=link_to post.title, post_path(post)%>", "<%= link_to post.title, post_path(post) %>", 1));
    let partial_tab = f.tab();
    f.s.dispatch(Command::Undo { tab: partial_tab });
    assert_eq!(current_text(f.s.state()), RAILS_PARTIAL);
    // The layout: `yield` in a template is no syntax error (its Ruby runs in a method, as Rails compiles it).
    f.show("app/views/layouts/application.html.erb");
    let layout_tab = f.tab();
    f.settle("layout problems", Duration::from_secs(60), move |s| s.state().diagnostics(layout_tab).is_some());
    assert_eq!(f.problems(), Vec::<String>::new());
    for (name, text) in [(view, RAILS_VIEW), (partial, RAILS_PARTIAL), ("app/views/layouts/application.html.erb", RAILS_LAYOUT), ("app/helpers/posts_helper.rb", RAILS_HELPER)] {
        assert_eq!(std::fs::read_to_string(f.root.join(name)).unwrap(), text, "{name} is untouched on disk");
    }
    let database = |p: &String| !p.starts_with("db/development.sqlite3");
    assert_eq!(files(&f.root).into_iter().filter(database).collect::<Vec<_>>(), before.into_iter().filter(database).collect::<Vec<_>>(), "nothing else written into the application");
}

/// Haml-specific product behavior with real Ruby LSP and haml-lint, including the implicit
/// helper receiver of an app/views template and formatting without executing its Ruby.
#[test]
#[ignore = "needs installed ruby-lsp, rubocop, haml 7.5.1 and haml_lint; run alone"]
fn real_haml_blocks_helpers_and_configured_formatting() {
    let source = "%ul(title=\"😀\")\r\n  - names = [\"𝄞\"]\r\n  - names.each do |name|\r\n    - if name\r\n      %li{title: Greeter.greet(name)}= Greeter.gre\r\n    - else\r\n      %li Hi #{Greeter.greet(name, excited: true)}\r\n  = self.helper_gre\r\n  %p= 1+2";
    let file = "app/views/posts/show.html.haml";
    let Some(mut f) = template_project(file, source, false) else { panic!("Ruby LSP must be installed for this opt-in test") };
    write(&f.root, "app/helpers/application_helper.rb", "module ApplicationHelper\n  def helper_greet(name)\n    name\n  end\nend\n");
    write(&f.root, ".haml-lint.yml", "linters:\n  RuboCop:\n    enabled: true\n  FinalNewline:\n    enabled: false\n");
    f.s.dispatch(Command::RestartServer);
    f.settle("restart", Duration::from_secs(60), |s| s.state().provider().is_some_and(|p| matches!(p.status, Status::Ready)));
    let tab = f.tab();
    f.caret("Greeter.gre\r", 11);
    f.complete_into("greet", &source.replacen("Greeter.gre\r", "Greeter.greet\r", 1), source);
    assert_eq!(f.parameters_at("name, excited", 6), Some((vec!["greet(name, excited: <default>)".into()], Some(1))));
    f.caret("self.helper_gre", "self.helper_gre".len());
    let row = f.complete_until("helper_greet");
    f.s.dispatch(Command::SelectCompletion(row)); f.s.dispatch(Command::AcceptCompletion);
    f.settle("helper accepted", Duration::from_secs(30), |s| current_text(s.state()).contains("self.helper_greet"));
    assert_eq!(current_text(f.s.state()), source.replace("self.helper_gre", "self.helper_greet"), "{:?}", f.notices());
    f.s.dispatch(Command::Undo { tab }); assert_eq!(current_text(f.s.state()), source);
    let before = f.notices().len();
    f.s.dispatch(Command::Format(code_core::session::FormatKind::Format));
    f.settle("Haml format", Duration::from_secs(30), |s| s.state().notices().len() > before && s.state().asking().is_none());
    assert_eq!(f.notices().last().map(String::as_str), Some("Format Document: done with haml-lint"));
    let formatted = current_text(f.s.state());
    assert!(formatted.contains("1 + 2"), "{formatted:?}");
    assert!(!formatted.ends_with('\n'), "missing final newline preserved");
    assert_eq!(formatted.matches('\r').count(), source.matches('\r').count());
    f.s.dispatch(Command::Undo { tab }); assert_eq!(current_text(f.s.state()), source);
    assert_eq!(std::fs::read_to_string(f.root.join(file)).unwrap(), source);
}

#[test]
fn builtin_shell_syntax_errors_remain_visible_when_shellcheck_is_missing_and_configured_zsh_is_unchecked() {
    for shell in ["sh", "zsh"] {
        let text = "if then\n";
        let mut f = real_entries(|root| vec![("bash-language-server".into(), json!({
            "executable": "/nonexistent/bash-language-server",
            "settings": { "shellcheckPath": "/nonexistent/shellcheck", "shfmt": { "path": "" } },
            "projects": { root.to_str().unwrap(): { "shell": shell } }
        }))], &[("broken.sh", text)]);
        f.s.dispatch(Command::OpenRoot(f.root.clone())); f.s.dispatch(Command::Open(f.root.join("broken.sh")));
        f.settle("parser and missing ShellCheck", Duration::from_secs(30), |s| s.state().highlights().is_some() && s.current_shell_lint().and_then(|lint| lint.note()).is_some());
        let details = f.s.problems_details();
        assert!(details.contains("ShellCheck is missing") && !details.contains("ShellCheck reports no problems"), "{details}");
        if shell == "sh" {
            assert!(f.s.state().diagnostics(f.tab()).is_some_and(|d| d.items.iter().any(|d| d.source.as_deref() == Some("syntax parser"))), "{details}");
            assert!(details.contains("Error:") && details.contains("syntax parser"), "{details}");
        } else {
            assert!(f.s.state().diagnostics(f.tab()).is_none(), "the configured zsh dialect must not borrow Bash syntax errors");
        }
        assert_eq!(std::fs::read(f.root.join("broken.sh")).unwrap(), text.as_bytes());
    }
}
