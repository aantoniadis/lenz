//! Semantic providers through the session's public commands and state, against a scripted fake server
//! (the `fake_lsp` example). A fake proves the reader's client behavior, not a real server's quality.
#![cfg(unix)]

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use lenz_content::theme::{ThemeOverrides, ThemePaths};
use reader_core::{
    search::ResultsKind,
    semantics::hierarchy::Direction,
    semantics::Status,
    session::{Command, CommandId, Completion, Config, Selection, Session},
};
use lenz_content::highlight::Capture;
use serde_json::Value;

fn current_text(state: &reader_core::session::State) -> String {
    state.current_source().map(|s| s.copy_range(0..s.len_bytes()).unwrap()).unwrap_or_default()
}
fn active_view(s: &Session) -> Option<reader_core::session::View<'_>> {
    s.state().view(s.state().active()?.id())
}
fn projected(view: &reader_core::session::View<'_>, line: usize) -> String {
    let mut cursor = view.display_start(line).unwrap();
    let mut text = String::new();
    loop { let window = view.display_window(&mut cursor, 4096).unwrap(); text.push_str(&window.shown.text); if window.complete { return text; } }
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    log: PathBuf,
    s: Session,
}

/// A Cargo package with `files`, and a session whose rust-analyzer is the fake run with `env`.
fn fixture(env: &[(&str, &str)], files: &[(&str, &str)]) -> Fixture {
    fixture_with(env, files, serde_json::json!({}), std::sync::Arc::new(Instant::now))
}

/// As [`fixture`], with more rust-analyzer settings and the session's clock.
fn fixture_with(env: &[(&str, &str)], files: &[(&str, &str)], settings: Value, clock: reader_core::semantics::Clock) -> Fixture {
    fixture_as("rust-analyzer", env, files, settings, clock)
}

/// As [`fixture_with`], with the fake standing in for provider `id`.
fn fixture_as(id: &str, env: &[(&str, &str)], files: &[(&str, &str)], settings: Value, clock: reader_core::semantics::Clock) -> Fixture {
    fixture_input(id, env, files, settings, clock, false)
}

/// As [`fixture`], with draft text input enabled (as on a validated backend).
fn editing_fixture(env: &[(&str, &str)], files: &[(&str, &str)]) -> Fixture {
    fixture_input("rust-analyzer", env, files, serde_json::json!({}), std::sync::Arc::new(Instant::now), true)
}

fn fixture_input(id: &str, env: &[(&str, &str)], files: &[(&str, &str)], settings: Value, clock: reader_core::semantics::Clock, text_input: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proj");
    write(&root, "Cargo.toml", "[package]\nname = \"proj\"\n");
    for (name, text) in files {
        write(&root, name, text);
    }
    let log = dir.path().join("fake.log");
    let script = dir.path().join("fake-server");
    let mut sh = String::from("#!/bin/sh\n");
    sh += &format!("export FAKE_LSP_LOG='{}'\n", log.display());
    for (k, v) in env {
        sh += &format!("export {k}='{v}'\n");
    }
    sh += &format!("exec '{}' \"$@\"\n", fake_lsp().display());
    std::fs::write(&script, sh).unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let providers = dir.path().join("config/providers.json");
    std::fs::create_dir_all(providers.parent().unwrap()).unwrap();
    let mut ra = settings;
    // `"executable": null` leaves the provider to find its own (the fake is put where it looks).
    match ra.get("executable") {
        Some(Value::Null) => drop(ra.as_object_mut().unwrap().remove("executable")),
        _ => ra["executable"] = serde_json::json!(script),
    }
    // Only the fake runs: every other provider is pointed nowhere.
    let mut all: serde_json::Map<String, Value> = reader_core::semantics::provider::PROVIDERS.iter().map(|p| (p.id.to_string(), serde_json::json!({ "executable": format!("/nonexistent/{}", p.id) }))).collect();
    all.insert(id.into(), ra);
    std::fs::write(&providers, serde_json::json!({ "providers": all }).to_string()).unwrap();
    let s = Session::with_text_input(Config {
        themes: ThemePaths::under(&dir.path().join("config"), &dir.path().join("state"), "lenz-reader"),
        recent: dir.path().join("state/recent-roots.json"),
        settings: dir.path().join("state/settings.json"),
        providers,
        cache: dir.path().join("cache"),
        idle: reader_core::semantics::IDLE,
        clock,
        overrides: ThemeOverrides::default(),
        dark: true,
    }, text_input);
    Fixture { _dir: dir, root, log, s }
}

/// The fake server: the `fake_lsp` example, built (once per test run; a no-op when fresh) next to this test's
/// profile, since a narrower `cargo test` does not build examples.
fn fake_lsp() -> PathBuf {
    let profile = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_owned();
    let exe = profile.join("examples/fake_lsp");
    static BUILD: std::sync::Once = std::sync::Once::new();
    BUILD.call_once(|| {
        let mut cargo = std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
        cargo.args(["build", "-q", "-p", "reader-core", "--example", "fake_lsp"]).current_dir(env!("CARGO_MANIFEST_DIR"));
        if profile.ends_with("release") {
            cargo.arg("--release");
        }
        assert!(cargo.status().is_ok_and(|s| s.success()), "building the fake server failed");
    });
    exe
}

fn write(root: &Path, name: &str, text: &str) -> PathBuf {
    let p = root.join(name);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, text).unwrap();
    p
}

#[test]
fn project_initialization_settings_are_isolated_by_language_server_root() {
    let mut f = fixture_as("ruby-lsp", &[], &[
        ("one/Gemfile", ""), ("one/app/a.rb", "class One; end\n"),
        ("two/Gemfile", ""), ("two/app/b.rb", "class Two; end\n"),
    ], serde_json::json!({"settings":{"indexing":{"excludedGems":["global-tool"]},"keep":true}}), std::sync::Arc::new(Instant::now));
    f.open("one/app/a.rb");
    let config = f._dir.path().join("config/providers.json");
    let mut value: Value = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
    value["providers"]["ruby-lsp"]["projects"] = serde_json::json!({
        f.root.join("one").to_str().unwrap(): {"indexing":{"excludedGems":["project-tool"]}}
    });
    std::fs::write(config, value.to_string()).unwrap();
    f.s.dispatch(Command::RestartServer);
    let log = f.log.clone();
    f.settle_until(move || messages(&log).iter().filter(|m| m["method"] == "initialize").count() >= 2);
    f.settle(ready);
    f.open("two/app/b.rb");
    let all = messages(&f.log);
    let init: Vec<_> = all.iter().filter(|m| m["method"] == "initialize").collect();
    assert_eq!(init.len(), 3);
    assert_eq!(init[1]["params"]["initializationOptions"]["indexing"]["excludedGems"], serde_json::json!(["project-tool"]));
    assert_eq!(init[2]["params"]["initializationOptions"]["indexing"]["excludedGems"], serde_json::json!(["global-tool"]));
    assert_eq!(init[1]["params"]["initializationOptions"]["keep"], true);
    assert_eq!(init[1]["params"]["initializationOptions"]["formatter"], "none");
}

#[test]
fn project_workspace_configuration_matches_its_initialization_options() {
    let mut f = fixture(&[("FAKE_LSP_ASK_CONFIG", "1")], &[("src/lib.rs", "pub fn one() {}\n")]);
    f.open("src/lib.rs");
    let log = f.log.clone();
    f.settle_until(move || messages(&log).iter().any(|m| m["id"] == "cfg-1" && m.get("result").is_some()));
    let file = f._dir.path().join("config/providers.json");
    let mut value: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    value["providers"]["rust-analyzer"]["settings"] = serde_json::json!({"cargo":{"features":"global"},"keep":true});
    value["providers"]["rust-analyzer"]["projects"] = serde_json::json!({f.root.to_str().unwrap():{"cargo":{"features":["local"]}}});
    std::fs::write(file, value.to_string()).unwrap();
    f.s.dispatch(Command::RestartServer);
    let log = f.log.clone();
    f.settle_until(move || messages(&log).iter().filter(|m| m["id"] == "cfg-1" && m.get("result").is_some()).count() >= 2);
    let all = messages(&f.log);
    let init = all.iter().rev().find(|m| m["method"] == "initialize").unwrap();
    let reply = all.iter().rev().find(|m| m["id"] == "cfg-1" && m.get("result").is_some()).unwrap();
    assert_eq!(init["params"]["initializationOptions"]["cargo"]["features"], serde_json::json!(["local"]));
    assert_eq!(reply["result"][0], init["params"]["initializationOptions"]);
    assert_eq!(reply["result"][1], serde_json::json!({"features":["local"]}));
    assert_eq!(reply["result"][0]["keep"], true);
}

#[test]
fn rebuilding_the_project_index_restarts_its_server_and_preserves_sources() {
    let source = "class Alpha; end\n";
    let mut f = fixture_as("ruby-lsp", &[], &[("Gemfile", ""), ("a.rb", source)], serde_json::json!({}), std::sync::Arc::new(Instant::now));
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("a.rb");
    f.settle(|s| s.state().index_coverage().indexed >= 1 && s.state().index_coverage().complete());
    f.s.dispatch(Command::RebuildIndex);
    let log = f.log.clone();
    f.settle_until(move || messages(&log).iter().filter(|m| m["method"] == "initialize").count() >= 2);
    f.settle(|s| ready(s) && s.state().index_coverage().complete());
    assert_eq!(f.s.state().servers().len(), 1);
    assert_eq!(std::fs::read_to_string(f.root.join("a.rb")).unwrap(), source);
    let init: Vec<_> = messages(&f.log).into_iter().filter(|m| m["method"] == "initialize").collect();
    assert_eq!(init.len(), 2);
    assert_eq!(init[0]["params"]["initializationOptions"], init[1]["params"]["initializationOptions"]);
}

#[test]
fn rebuilding_finishes_symbol_searches_started_before_or_during_the_reset() {
    let mut f = fixture_as("ruby-lsp", &[("FAKE_LSP_SILENT_SYMBOLS", "1")], &[("Gemfile", ""), ("a.rb", "class A; end\n")], serde_json::json!({}), std::sync::Arc::new(Instant::now));
    f.s.dispatch(Command::OpenRoot(f.root.clone())); f.open("a.rb");
    f.s.dispatch(Command::Symbols("A".into()));
    f.s.dispatch(Command::RebuildIndex);
    assert!(f.s.state().asking().is_none());
    assert!(f.s.state().results().unwrap().done());
    let rx = f.s.completions();
    let reset = loop {
        if let Ok(c) = recv(&rx) {
            if matches!(c, Completion::IndexReset { .. }) { break c; }
            f.s.dispatch(c);
        }
    };
    f.s.dispatch(Command::Symbols("A".into()));
    assert!(f.s.state().asking().is_some());
    f.s.dispatch(reset);
    assert!(f.s.state().asking().is_none());
    assert!(f.s.state().results().unwrap().done());
    assert!(f.s.state().results().unwrap().error().unwrap().contains("rebuilding"));
}

#[test]
fn project_exclusion_commands_save_restart_reset_and_preserve_other_servers() {
    let mut f = fixture_as("ruby-lsp", &[], &[
        ("one/Gemfile", ""), ("one/app/a.rb", "class One; end\n"),
        ("two/Gemfile", ""), ("two/app/b.rb", "class Two; end\n"),
    ], serde_json::json!({}), std::sync::Arc::new(Instant::now));
    f.open("one/app/a.rb");
    f.open("two/app/b.rb");
    f.open("one/app/a.rb");
    assert!(f.s.disabled(CommandId::ExcludeRubyTooling).is_none());
    f.s.dispatch(Command::ExcludeRubyTooling);
    assert!(f.s.disabled(CommandId::ResetProjectServerSettings).is_some());
    f.s.dispatch(Command::ExcludeRubyTooling); // a duplicate while saving is ignored
    let log = f.log.clone();
    f.settle_until(move || messages(&log).iter().filter(|m| m["method"] == "initialize").count() >= 3);
    f.settle(ready);
    assert!(f.s.disabled(CommandId::ExcludeRubyTooling).is_none(), "exclusions can be turned off without resetting other settings");
    assert!(f.s.disabled(CommandId::ResetProjectServerSettings).is_none());
    let settings = reader_core::semantics::provider::Settings::read(&f._dir.path().join("config/providers.json"));
    assert!(settings.ruby_tooling_excluded(&f.root.join("one")));
    assert!(!settings.has_project("ruby-lsp", &f.root.join("two")));
    f.s.dispatch(Command::ResetProjectServerSettings);
    let log = f.log.clone();
    f.settle_until(move || messages(&log).iter().filter(|m| m["method"] == "initialize").count() >= 4);
    f.settle(ready);
    assert!(f.s.disabled(CommandId::ExcludeRubyTooling).is_none());
    assert!(f.s.disabled(CommandId::ResetProjectServerSettings).is_some());
    let all = messages(&f.log);
    let init: Vec<_> = all.iter().filter(|m| m["method"] == "initialize").collect();
    assert_eq!(init.len(), 4);
    assert_eq!(init.iter().filter(|m| m["params"]["rootUri"].as_str().unwrap().ends_with("/two")).count(), 1, "other projects keep their server");
    assert!(init[2]["params"]["initializationOptions"]["indexing"]["excludedGems"].as_array().unwrap().contains(&serde_json::json!("parser")));
    assert!(init[3]["params"]["initializationOptions"]["indexing"]["excludedGems"].as_array().unwrap().contains(&serde_json::json!("parser")));
}

#[test]
fn project_settings_save_failure_leaves_the_server_and_file_unchanged() {
    let mut f = fixture_as("ruby-lsp", &[], &[("Gemfile", ""), ("a.rb", "class A; end\n")], serde_json::json!({}), std::sync::Arc::new(Instant::now));
    f.open("a.rb");
    let file = f._dir.path().join("config/providers.json");
    std::fs::write(&file, "{broken").unwrap();
    f.s.dispatch(Command::ExcludeRubyTooling);
    f.settle(|s| s.state().notices().iter().any(|n| n.contains("were not changed")));
    assert!(ready(&f.s));
    assert_eq!(std::fs::read_to_string(file).unwrap(), "{broken");
    assert_eq!(messages(&f.log).iter().filter(|m| m["method"] == "initialize").count(), 1);
    assert!(f.s.disabled(CommandId::ExcludeRubyTooling).is_none());
}

#[test]
fn a_project_save_completion_does_not_retarget_after_switching_projects() {
    let mut f = fixture_as("ruby-lsp", &[], &[
        ("one/Gemfile", ""), ("one/a.rb", "class One; end\n"),
        ("two/Gemfile", ""), ("two/b.rb", "class Two; end\n"),
    ], serde_json::json!({}), std::sync::Arc::new(Instant::now));
    f.open("one/a.rb");
    f.s.dispatch(Command::ExcludeRubyTooling);
    let next = f.root.join("two/b.rb");
    f.s.dispatch(Command::Open(next.clone()));
    let rx = f.s.completions();
    let mut saved = None;
    let deadline = Instant::now() + Duration::from_secs(15);
    while saved.is_none() || !ready(&f.s) || f.s.state().current_source().is_none_or(|d| d.source() != next) {
        assert!(Instant::now() < deadline);
        if let Ok(c) = recv(&rx) {
            if matches!(&c, Completion::ProjectProviderSaved { .. }) { saved = Some(c); }
            else { f.s.dispatch(c); }
        }
    }
    f.s.dispatch(saved.unwrap());
    let settings = reader_core::semantics::provider::Settings::read(&f._dir.path().join("config/providers.json"));
    assert!(settings.ruby_tooling_excluded(&f.root.join("one")));
    assert!(!settings.has_project("ruby-lsp", &f.root.join("two")));
    assert_eq!(f.s.state().current_source().unwrap().source(), next);
    assert!(ready(&f.s));
    f.open("one/a.rb");
    let all = messages(&f.log);
    let init: Vec<_> = all.iter().filter(|m| m["method"] == "initialize").collect();
    assert_eq!(init.len(), 3);
    assert_eq!(init.iter().filter(|m| m["params"]["rootUri"].as_str().unwrap().ends_with("/two")).count(), 1);
}

#[test]
fn a_project_save_finishes_a_symbol_search_started_while_saving() {
    let mut f = fixture_as("ruby-lsp", &[("FAKE_LSP_SILENT_SYMBOLS", "1")], &[("Gemfile", ""), ("a.rb", "class A; end\n")], serde_json::json!({}), std::sync::Arc::new(Instant::now));
    f.open("a.rb");
    f.s.dispatch(Command::ExcludeRubyTooling);
    let rx = f.s.completions();
    let deadline = Instant::now() + Duration::from_secs(15);
    let saved = loop {
        assert!(Instant::now() < deadline);
        if let Ok(c) = recv(&rx) {
            if matches!(&c, Completion::ProjectProviderSaved { .. }) { break c; }
            f.s.dispatch(c);
        }
    };
    f.s.dispatch(Command::Symbols("A".into()));
    assert!(f.s.state().asking().is_some());
    assert!(!f.s.state().results().unwrap().done());
    f.s.dispatch(saved);
    assert!(f.s.state().asking().is_none());
    let results = f.s.state().results().unwrap();
    assert!(results.done());
    assert!(results.error().unwrap().contains("settings changed"));
}

impl Fixture {
    /// Apply background results until `done` holds; `each` checks every state on the way.
    fn settle_checking(&mut self, done: impl Fn(&Session) -> bool, each: impl Fn(&Session)) {
        let rx = self.s.completions();
        let until = Instant::now() + Duration::from_secs(15);
        while !done(&self.s) {
            assert!(Instant::now() < until, "timed out; servers {:?}; notices {:?}", self.s.state().servers(), self.s.state().notices());
            if let Ok(c) = recv(&rx) {
                self.s.dispatch(c);
                each(&self.s);
            }
        }
    }

    fn settle(&mut self, done: impl Fn(&Session) -> bool) {
        self.settle_checking(done, |_| {});
    }

    /// Open `name` and wait until its server has it.
    fn open(&mut self, name: &str) {
        let path = self.root.join(name);
        self.s.dispatch(Command::Open(path.clone()));
        self.settle(|s| s.state().current_source().is_some_and(|d| d.source() == path) && ready(s));
        let log = self.log.clone();
        self.settle_until(move || messages(&log).iter().any(|m| m["method"] == "textDocument/didOpen" && m["params"]["textDocument"]["uri"].as_str().is_some_and(|u| u.ends_with(name))));
    }

    /// Dispatch whatever arrives until `cond` (about the outside world) holds.
    fn settle_until(&mut self, cond: impl Fn() -> bool) {
        let rx = self.s.completions();
        let until = Instant::now() + Duration::from_secs(15);
        while !cond() {
            assert!(Instant::now() < until, "timed out");
            if let Ok(c) = recv(&rx) {
                self.s.dispatch(c);
            }
        }
    }

    /// Put the caret on the first `needle` in the active document (at its `skip`th character).
    fn caret(&mut self, needle: &str) -> usize {
        let doc = self.s.state().current_source().unwrap().clone();
        let at = doc.copy_range(0..doc.len_bytes()).unwrap().find(needle).unwrap_or_else(|| panic!("{needle} not in the document"));
        self.s.dispatch(Command::Select(Selection { anchor: at + 1, head: at + 1 }));
        at
    }

    fn definition(&mut self, needle: &str, peek: bool) {
        self.caret(needle);
        self.s.dispatch(Command::Definition { peek });
    }

    fn asked(&mut self) {
        self.settle(|s| s.state().asking().is_none());
    }

    fn notice(&self, part: &str) -> bool {
        self.s.state().notices().iter().any(|n| n.contains(part))
    }

    fn selected_text(&self) -> String {
        let doc = self.s.state().current_source().unwrap();
        doc.copy_range(self.s.state().selection().unwrap().range()).unwrap()
    }
}

fn recv(rx: &async_channel::Receiver<Completion>) -> Result<Completion, ()> {
    let until = Instant::now() + Duration::from_millis(50);
    loop {
        match rx.try_recv() {
            Ok(c) => return Ok(c),
            Err(_) if Instant::now() >= until => return Err(()),
            Err(_) => std::thread::sleep(Duration::from_millis(1)),
        }
    }
}

fn ready(s: &Session) -> bool {
    s.state().provider().is_some_and(|v| matches!(v.status, Status::Ready | Status::Indexing(_)))
}

fn messages(log: &Path) -> Vec<Value> {
    std::fs::read_to_string(log).unwrap_or_default().lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

fn alive(pid: u64) -> bool {
    std::process::Command::new("kill").args(["-0", &pid.to_string()]).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
        && !std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| s.split(' ').nth(2) == Some("Z"))
}

const LIB: &str = "pub fn helper() {}\npub struct Thing;\n";
const MAIN: &str = "fn main() {\n    let s = \"é𝄞\"; helper();\n    let t: Thing = Thing;\n    nothing_here();\n}\n";

#[test]
fn starts_on_the_first_relevant_source_and_follows_a_unicode_definition() {
    for encoding in ["none", "utf-8"] {
        let mut f = fixture(&[("FAKE_LSP_ENCODING", encoding)], &[("src/lib.rs", LIB), ("src/main.rs", MAIN), ("README.md", "# x\n")]);
        f.s.dispatch(Command::OpenRoot(f.root.clone()));
        f.settle(|s| s.state().project().is_some_and(|p| matches!(p.coverage(), reader_core::project::Coverage::Complete(_))));
        // Opening a folder, or a file in a language without a provider, starts nothing.
        f.s.dispatch(Command::Open(f.root.join("README.md")));
        f.settle(|s| s.state().current_source().is_some());
        std::thread::sleep(Duration::from_millis(100));
        while let Ok(c) = recv(&f.s.completions()) {
            f.s.dispatch(c);
        }
        assert!(f.s.state().servers().is_empty() && f.s.state().provider().is_none() && !f.log.exists());
        assert_eq!(f.s.disabled(CommandId::Definition), Some("No language server for this language"));
        f.open("src/main.rs");
        let log = messages(&f.log);
        let init = log.iter().find(|m| m["method"] == "initialize").unwrap();
        assert_eq!(init["params"]["capabilities"]["general"]["positionEncodings"], serde_json::json!(["utf-8", "utf-16"]));
        assert_eq!(init["params"]["capabilities"]["workspace"]["applyEdit"], false);
        assert!(init["params"]["rootUri"].as_str().unwrap().ends_with("/proj"));
        let open = log.iter().find(|m| m["method"] == "textDocument/didOpen").unwrap();
        assert_eq!(open["params"]["textDocument"]["text"], MAIN, "the exact snapshot shown");
        assert_eq!(open["params"]["textDocument"]["languageId"], "rust");
        let view = f.s.state().provider().unwrap();
        assert!(view.summary().starts_with("rust-analyzer (untested version fake-lsp 1.0.0)"), "{}", view.summary());

        // Past the multibyte string: the column is converted in the negotiated encoding both ways.
        f.definition("helper();", false);
        f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("lib.rs")));
        assert_eq!(f.selected_text(), "helper", "{encoding}");
        assert!(f.s.state().can_back());
        f.s.dispatch(Command::Back);
        f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("main.rs")));
        assert_eq!(std::fs::read_to_string(f.root.join("src/main.rs")).unwrap(), MAIN, "reading never writes");
    }
}

#[test]
fn crlf_sources_convert_positions_on_their_own_lines() {
    for encoding in ["none", "utf-8"] {
        let lib = "pub struct Ünit;\r\n\r\npub fn helper() {}\r\n";
        let main = "fn main() {\r\n    let s = \"é𝄞\"; helper();\r\n}\r\n";
        let mut f = fixture(&[("FAKE_LSP_ENCODING", encoding)], &[("src/lib.rs", lib), ("src/main.rs", main)]);
        f.open("src/main.rs");
        f.definition("helper();", false);
        f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("lib.rs")));
        assert_eq!(f.selected_text(), "helper", "{encoding}");
        assert_eq!(f.s.state().selection().unwrap().range().start, lib.find("helper").unwrap());
        assert_eq!(std::fs::read(f.root.join("src/lib.rs")).unwrap(), lib.as_bytes(), "line endings untouched");
    }
}

#[test]
fn several_targets_are_candidates_and_peek_does_not_move() {
    let mut f = fixture(&[], &[("src/a.rs", "fn dup() {}\n"), ("src/b.rs", "fn other() {}\nfn dup() {}\n"), ("src/main.rs", "fn main() { dup(); helper(); }\n"), ("src/lib.rs", LIB)]);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("src/main.rs");
    f.definition("dup()", false);
    f.settle(|s| s.state().results().is_some_and(|r| r.hits().len() == 2 && r.preview().is_some()));
    let r = f.s.state().results().unwrap();
    assert_eq!((r.kind(), r.results_kind()), ("Definitions (rust-analyzer · proj)".to_string(), ResultsKind::Definitions));
    assert!(f.s.state().current_source().unwrap().source().ends_with("main.rs"), "candidates do not move the main area");
    f.s.dispatch(Command::OpenResult(1));
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("b.rs")));
    assert_eq!(f.selected_text(), "dup");
    assert_eq!(f.s.state().selection().unwrap().range().start, "fn other() {}\nfn ".len());
    // Peek at a single target: shown in the panel, the reading area stays.
    f.s.dispatch(Command::Back);
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("main.rs")));
    f.definition("helper", true);
    f.settle(|s| s.state().results().is_some_and(|r| r.results_kind() == ResultsKind::Peek && r.preview().is_some()));
    assert!(f.s.state().current_source().unwrap().source().ends_with("main.rs"));
    let r = f.s.state().results().unwrap();
    assert_eq!(r.kind(), "Peek definition (rust-analyzer · proj)");
    assert!(r.preview().unwrap().doc.source().ends_with("lib.rs"));
}

#[test]
fn missing_unsupported_failed_empty_and_virtual_answers_are_distinct() {
    // Not installed: setup guidance, and the commands say why they cannot run.
    let mut f = fixture(&[], &[("src/main.rs", MAIN)]);
    let providers = f.root.parent().unwrap().join("config/providers.json");
    std::fs::write(&providers, r#"{"providers":{"rust-analyzer":{"executable":"/nonexistent/rust-analyzer"}}}"#).unwrap();
    let mut m = Session::new(Config {
        themes: ThemePaths::under(&f.root.join("../c2"), &f.root.join("../s2"), "lenz-reader"),
        recent: f.root.join("../s2/recent.json"),
        settings: f.root.join("../s2/settings.json"),
        providers,
        cache: f.root.join("../cache"),
        idle: reader_core::semantics::IDLE,
        clock: std::sync::Arc::new(std::time::Instant::now),
        overrides: ThemeOverrides::default(),
        dark: true,
    });
    std::mem::swap(&mut f.s, &mut m);
    f.s.dispatch(Command::Open(f.root.join("src/main.rs")));
    f.settle(|s| s.state().provider().is_some_and(|v| matches!(v.status, Status::Missing(_))));
    let Status::Missing(why) = f.s.state().provider().unwrap().status else { unreachable!() };
    assert!(why.contains("was not found") && why.contains("rustup component add rust-analyzer"), "{why}");
    let rows = f.s.language_server_rows();
    assert!(f.s.language_server_details().contains("rustup component add rust-analyzer"));
    assert!(f.s.language_server_details().contains("config/providers.json"));
    assert!(rows.iter().any(|(_, action)| *action == Some(CommandId::RestartServer)));
    let reader_core::actions::CatalogAction::Command(copy) = f.s.action(CommandId::CopyServerDetails) else { panic!("copy must be available") };
    let copied = f.s.dispatch(copy).copy.unwrap();
    assert!(copied.contains(&why) && copied.contains("Workspace:") && copied.contains("Executable:") && copied.contains("Provider settings:"));
    assert!(!copied.contains("Restart Language Server\n\n"));
    assert_eq!(f.s.state().provider().unwrap().status_label(), "LSP unavailable");
    assert_eq!(f.s.disabled(CommandId::Definition), Some("The language server is unavailable (see the status bar)"));
    drop(m);

    // A server that cannot answer --version is unusable, not running.
    let mut f = fixture(&[("FAKE_LSP_VERSION_FAIL", "1")], &[("src/main.rs", MAIN)]);
    f.s.dispatch(Command::Open(f.root.join("src/main.rs")));
    f.settle(|s| s.state().provider().is_some_and(|v| matches!(&v.status, Status::Missing(w) if w.contains("--version` failed:") && w.contains("fake-lsp: no toolchain"))));

    // No definition capability: unsupported, not an empty success.
    let mut f = fixture(&[("FAKE_LSP_NO_DEFINITION", "1")], &[("src/main.rs", MAIN), ("src/lib.rs", LIB)]);
    f.open("src/main.rs");
    f.definition("helper", false);
    assert!(f.notice("rust-analyzer does not provide definitions"), "{:?}", f.s.state().notices());

    let mut f = fixture(&[], &[("src/main.rs", "fn main() { nothing(); error_x(); virt(); }\n")]);
    f.open("src/main.rs");
    f.definition("nothing", false);
    f.asked();
    assert!(f.notice("rust-analyzer found no definition here"), "{:?}", f.s.state().notices());
    f.definition("error_x", false);
    f.asked();
    assert!(f.notice("rust-analyzer failed: fake failure"));
    f.definition("virt", false);
    f.asked();
    assert!(f.notice("source unavailable: rust-analyzer offers no documented way for the reader to show fake-virtual:///pkg/Generated.java?=jar%2Fx"), "{:?}", f.s.state().notices());
    assert!(f.s.state().current_source().unwrap().source().ends_with("main.rs"));
}

#[test]
fn recovery_rows_keep_their_command_identity_while_server_status_changes() {
    let mut f = fixture(&[("FAKE_LSP_INIT_DELAY_MS", "200")], &[("src/main.rs", "fn main() { crash_now(); }\n")]);
    let actions = |s: &Session| s.language_server_rows().into_iter().take(4).map(|(_, id)| id).collect::<Vec<_>>();
    let expected = vec![Some(CommandId::ServerDetails), Some(CommandId::RestartServer), Some(CommandId::DisableServer), Some(CommandId::WorkspaceRoot)];
    f.s.dispatch(Command::Open(f.root.join("src/main.rs")));
    f.settle_checking(ready, |s| assert_eq!(actions(s), expected));
    assert_eq!(actions(&f.s), expected);
    f.definition("crash_now", false);
    f.settle_checking(|s| s.state().provider().is_some_and(|v| matches!(v.status, Status::Failed(_))), |s| assert_eq!(actions(s), expected));
    assert_eq!(actions(&f.s), expected);
}

#[test]
fn late_cancelled_and_outdated_replies_are_rejected() {
    let mut f = fixture(&[], &[("src/main.rs", "fn main() { slow_one(); fast(); }\nfn slow_one() {}\nfn fast() {}\n")]);
    f.open("src/main.rs");
    // A later request replaces a slow one: only the later answer moves the view.
    f.definition("slow_one();", false);
    f.definition("fast();", false);
    f.asked();
    assert_eq!(f.selected_text(), "fast");
    std::thread::sleep(Duration::from_millis(500)); // the slow reply arrives now, and is ignored
    f.settle_until(|| true);
    while let Ok(c) = recv(&f.s.completions()) {
        f.s.dispatch(c);
    }
    assert_eq!(f.selected_text(), "fast");
    // Cancelled: reading and the selection are untouched.
    f.definition("slow_one();", false);
    let before = f.s.state().selection();
    assert!(f.s.state().asking().is_some());
    f.s.dispatch(Command::CancelSemantic);
    assert!(f.s.state().asking().is_none());
    std::thread::sleep(Duration::from_millis(500));
    while let Ok(c) = recv(&f.s.completions()) {
        f.s.dispatch(c);
    }
    assert_eq!(f.s.state().selection(), before);
    let cancels = messages(&f.log).iter().filter(|m| m["method"] == "$/cancelRequest").count();
    assert!(cancels >= 2, "best-effort protocol cancellation is sent");
    // The file changes while the server answers: the answer is about the old text, and is not used.
    f.definition("slow_one();", false);
    std::fs::write(f.root.join("src/main.rs"), "fn main() { slow_one(); fast(); }\n\nfn slow_one() {}\nfn fast() {}\n").unwrap();
    f.settle(|s| s.state().asking().is_none());
    assert!(f.notice("the file changed while the language server answered"), "{:?}", f.s.state().notices());
    assert_eq!(f.s.state().selection().unwrap().range(), "fn main() { s".len().."fn main() { s".len());
    // The server got the new text before anything else is asked.
    let log = f.log.clone();
    f.settle_until(move || messages(&log).iter().any(|m| m["method"] == "textDocument/didChange" && m["params"]["contentChanges"][0]["text"].as_str().is_some_and(|t| t.contains("\n\nfn slow_one"))));
}

#[test]
fn edits_are_declined_and_initialization_is_not_readiness() {
    let mut f = fixture(&[("FAKE_LSP_EDIT", "1"), ("FAKE_LSP_INDEXING", "1")], &[("src/lib.rs", LIB), ("src/main.rs", MAIN)]);
    f.open("src/main.rs");
    f.settle(|s| s.state().provider().is_some_and(|v| v.status == Status::Indexing("Indexing 0/3".into())));
    let log = f.log.clone();
    f.settle_until(move || messages(&log).iter().any(|m| m["id"] == "edit-1"));
    let answer = messages(&f.log).into_iter().find(|m| m["id"] == "edit-1").unwrap();
    assert_eq!(answer["result"]["applied"], false);
    assert!(f.notice("asked to edit files; declined"));
    assert_eq!(std::fs::read_to_string(f.root.join("src/lib.rs")).unwrap(), LIB, "never edited");
    // Still indexing: requests run, and an empty answer says it may be incomplete.
    f.definition("nothing_here", false);
    f.asked();
    assert!(f.notice("it may still be loading or indexing its workspace"), "{:?}", f.s.state().notices());
}

#[test]
fn a_failed_version_probe_keeps_the_recovery_hint() {
    let mut f = fixture(&[("FAKE_LSP_VERSION_FAIL", "mise")], &[("src/main.rs", MAIN)]);
    f.s.dispatch(Command::Open(f.root.join("src/main.rs")));
    f.settle(|s| s.state().provider().is_some_and(|v| matches!(v.status, Status::Missing(_))));
    let Status::Missing(why) = f.s.state().provider().unwrap().status else { unreachable!() };
    assert!(why.contains("No version is set for shim: rust-analyzer") && why.contains("mise use -g rust@stable"), "{why}");
}

#[test]
fn startup_failure_keeps_the_actual_cause_and_recovery_hint_in_copied_details() {
    let mut f = fixture_as("pyright", &[("FAKE_LSP_START_FAIL", "mise")], &[("pyproject.toml", "[project]\nname = 'proj'\n"), ("main.py", "x = 1\n")], serde_json::json!({}), std::sync::Arc::new(Instant::now));
    f.s.dispatch(Command::Open(f.root.join("main.py")));
    f.settle(|s| s.state().provider().is_some_and(|v| matches!(v.status, Status::Failed(_))));
    let Status::Failed(why) = f.s.state().provider().unwrap().status else { unreachable!() };
    assert!(why.contains("No version is set for shim: pyright-langserver"), "{why}");
    assert!(why.contains("mise use -g node@26.8.1"), "{why}");
    assert!(why.contains(&f.root.parent().unwrap().join("fake-server").display().to_string()), "{why}");
    let reader_core::actions::CatalogAction::Command(copy) = f.s.action(CommandId::CopyServerDetails) else { panic!("copy must be available") };
    let copied = f.s.dispatch(copy).copy.unwrap();
    assert!(copied.contains(&why) && copied.contains("mise use -g node@26.8.1"));
}

#[test]
fn startup_failure_reports_the_runtime_found_on_the_effective_launch_path() {
    let dir = tempfile::tempdir().unwrap();
    let node = dir.path().join("node");
    std::fs::write(&node, "#!/bin/sh\necho v18.20.1\n").unwrap();
    std::fs::set_permissions(&node, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let mut f = fixture_as("pyright", &[("FAKE_LSP_START_FAIL", "mise")], &[("pyproject.toml", "[project]\nname = 'proj'\n"), ("main.py", "x = 1\n")], serde_json::json!({"env":{"PATH":dir.path()}}), std::sync::Arc::new(Instant::now));
    std::fs::write(f.root.parent().unwrap().join("package.json"), r#"{"name":"pyright","version":"1.1.407","engines":{"node":">=20"}}"#).unwrap();
    f.s.dispatch(Command::Open(f.root.join("main.py")));
    f.settle(|s| s.state().provider().is_some_and(|v| matches!(v.status, Status::Failed(_))));
    let Status::Failed(why) = f.s.state().provider().unwrap().status else { unreachable!() };
    assert!(why.contains("v18.20.1") && why.contains(&node.display().to_string()), "{why}");
    assert!(why.contains("engines.node") && why.contains(">=20"), "{why}");
}

#[test]
fn java_startup_failure_reports_the_configured_runtime_before_java_home() {
    let dir = tempfile::tempdir().unwrap();
    let java = dir.path().join("java");
    std::fs::write(&java, "#!/bin/sh\ntest \"$1\" = -version || exit 42\necho 'openjdk version \"17.0.13\"' >&2\n").unwrap();
    std::fs::set_permissions(&java, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let mut f = fixture_as("jdtls", &[("FAKE_LSP_START_FAIL", "mise")], &[("pom.xml", "<project/>\n"), ("src/Main.java", "class Main {}\n")], serde_json::json!({"args":[format!("--java-executable={}", java.display())],"env":{"JAVA_HOME":"/nonexistent/jdk"}}), std::sync::Arc::new(Instant::now));
    f.s.dispatch(Command::Open(f.root.join("src/Main.java")));
    f.settle(|s| s.state().provider().is_some_and(|v| matches!(v.status, Status::Failed(_))));
    let Status::Failed(why) = f.s.state().provider().unwrap().status else { unreachable!() };
    assert!(why.contains("openjdk version \"17.0.13\"") && why.contains(&java.display().to_string()), "{why}");
    assert!(!why.contains("/nonexistent/jdk"), "{why}");
}

#[test]
fn startup_stderr_is_byte_bounded_and_hidden_controls_are_safe_without_a_newline() {
    let mut f = fixture(&[("FAKE_LSP_START_FAIL", "noisy")], &[("src/main.rs", MAIN)]);
    let start = Instant::now();
    f.s.dispatch(Command::Open(f.root.join("src/main.rs")));
    f.settle(|s| s.state().provider().is_some_and(|v| matches!(v.status, Status::Failed(_))));
    let Status::Failed(why) = f.s.state().provider().unwrap().status else { unreachable!() };
    assert!(start.elapsed() < Duration::from_secs(5));
    assert!(why.contains("No version is set for shim") && why.contains("mise use -g node@26.8.1"), "{why}");
    assert!(why.contains("stderr truncated"), "missing visible truncation");
    assert!(why.contains("END OF UNTERMINATED NOISE"));
    assert!(why.len() < 26_000, "{} bytes", why.len());
    assert!(!why.chars().any(lenz_content::display::hides_text), "hidden controls escaped display safety");
    assert!(why.contains('é') && why.contains('𝄞'));
}

#[test]
fn a_crash_is_reported_and_not_restarted() {
    let mut f = fixture(&[], &[("src/main.rs", "fn main() { crash_now(); }\n")]);
    f.open("src/main.rs");
    f.definition("crash_now", false);
    f.settle(|s| s.state().provider().is_some_and(|v| matches!(v.status, Status::Failed(_))));
    f.asked();
    let Status::Failed(why) = f.s.state().provider().unwrap().status else { unreachable!() };
    let reader_core::actions::CatalogAction::Command(copy) = f.s.action(CommandId::CopyServerDetails) else { panic!("copy must be available") };
    assert!(f.s.dispatch(copy).copy.unwrap().contains(&why));

    assert!(why.contains("exit status: 3") && why.contains("crashed on purpose"), "{why}");
    assert!(f.notice("rust-analyzer stopped before answering"));
    assert_eq!(f.s.disabled(CommandId::Definition), Some("The language server stopped (see the status bar)"));
    std::thread::sleep(Duration::from_millis(300));
    while let Ok(c) = recv(&f.s.completions()) {
        f.s.dispatch(c);
    }
    assert_eq!(messages(&f.log).iter().filter(|m| m.get("pid").is_some()).count(), 1, "no automatic restart");
}

#[test]
fn at_most_two_servers_start_at_once_and_each_workspace_has_its_own() {
    let mut f = fixture(&[("FAKE_LSP_INIT_DELAY_MS", "300")], &[]);
    for p in ["a", "b", "c"] {
        write(&f.root, &format!("{p}/Cargo.toml"), &format!("[package]\nname = \"{p}\"\n"));
        write(&f.root, &format!("{p}/src/lib.rs"), "pub fn x() {}\n");
    }
    std::fs::remove_file(f.root.join("Cargo.toml")).unwrap(); // three separate packages
    let max = std::cell::Cell::new(0);
    let starting = |s: &Session| s.state().servers().iter().filter(|v| v.status == Status::Starting).count();
    // Each workspace's file opens once the previous server is launching (an inactive tab's server keeps running).
    for (i, p) in ["a", "b", "c"].into_iter().enumerate() {
        f.s.dispatch(Command::OpenRetained(f.root.join(format!("{p}/src/lib.rs"))));
        f.settle_checking(|s| s.state().servers().len() == i + 1, |s| max.set(max.get().max(starting(s))));
    }
    assert!(f.s.state().servers().iter().any(|v| v.status == Status::Queued), "{:?}", f.s.state().servers());
    f.settle_checking(
        |s| s.state().servers().len() == 3 && s.state().servers().iter().all(|v| matches!(v.status, Status::Ready | Status::Indexing(_))),
        |s| max.set(max.get().max(starting(s))),
    );
    assert_eq!(max.get(), 2, "the third waited for a startup slot");
    let roots: std::collections::BTreeSet<_> = f.s.state().servers().into_iter().filter_map(|v| v.root).collect();
    assert_eq!(roots.len(), 3);
    // Another file of an existing workspace reuses its server.
    write(&f.root, "a/src/more.rs", "fn y() {}\n");
    f.s.dispatch(Command::OpenRetained(f.root.join("a/src/more.rs")));
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("more.rs")) && ready(s));
    assert_eq!(f.s.state().servers().len(), 3);
}

#[test]
fn writer_failure_fails_the_request_and_stops_a_helper_that_keeps_output_open() {
    let mut f = fixture(&[("FAKE_LSP_CLOSE_INPUT_AFTER_OPEN", "1"), ("FAKE_LSP_NO_DECOR", "1")], &[("src/main.rs", "fn target() {}\n")]);
    f.open("src/main.rs");
    let log = f.log.clone(); f.settle_until(move || messages(&log).iter().any(|m| m.get("input_child").is_some()));
    let child = messages(&f.log).iter().find_map(|m| m["input_child"].as_u64()).unwrap();
    f.definition("target", false); f.asked();
    assert!(matches!(f.s.state().provider().unwrap().status, Status::Failed(_)));
    let until = Instant::now() + Duration::from_secs(8);
    while alive(child) && Instant::now() < until { if let Ok(c) = recv(&f.s.completions()) { f.s.dispatch(c); } }
    let stopped = !alive(child);
    if !stopped {
        // Clean the deliberately broken provider before reporting a failure; ownership remains with
        // Session, including signalling its reserved process group and reaping the exited parent.
        let other = f._dir.path().join("cleanup"); std::fs::create_dir(&other).unwrap(); f.s.dispatch(Command::OpenRoot(other));
        let until = Instant::now() + Duration::from_secs(8);
        while alive(child) && Instant::now() < until { std::thread::sleep(Duration::from_millis(10)); }
    }
    assert!(!alive(child), "cleanup must release the disposable helper");
    assert!(stopped, "writer failure must stop the owned group even when its output remains open");
}

#[test]
fn closing_the_project_stops_servers_and_their_helpers() {
    for ignore_exit in [false, true] {
        let env: &[(&str, &str)] = if ignore_exit { &[("FAKE_LSP_CHILD", "1"), ("FAKE_LSP_IGNORE_EXIT", "1")] } else { &[("FAKE_LSP_CHILD", "1")] };
        let mut f = fixture(env, &[("src/main.rs", MAIN)]);
        f.s.dispatch(Command::OpenRoot(f.root.clone()));
        f.open("src/main.rs");
        let log = messages(&f.log);
        let pid = log.iter().find_map(|m| m["pid"].as_u64()).unwrap();
        let child = log.iter().find_map(|m| m["child"].as_u64()).unwrap();
        assert!(alive(pid) && alive(child));
        let other = f.root.parent().unwrap().join("other");
        std::fs::create_dir_all(&other).unwrap();
        f.s.dispatch(Command::OpenRoot(other));
        assert!(f.s.state().servers().is_empty());
        let until = Instant::now() + Duration::from_secs(8);
        while (alive(pid) || alive(child)) && Instant::now() < until {
            while let Ok(c) = recv(&f.s.completions()) {
                f.s.dispatch(c);
            }
        }
        assert!(!alive(pid) && !alive(child), "server and helper stopped (ignoring exit: {ignore_exit})");
        if !ignore_exit {
            assert!(messages(&f.log).iter().any(|m| m["method"] == "shutdown") && messages(&f.log).iter().any(|m| m["method"] == "exit"));
        }
    }
    // Closing the window (dropping the session) stops them too.
    let mut f = fixture(&[], &[("src/main.rs", MAIN)]);
    f.open("src/main.rs");
    let pid = messages(&f.log).iter().find_map(|m| m["pid"].as_u64()).unwrap();
    drop(f.s);
    let until = Instant::now() + Duration::from_secs(8);
    while alive(pid) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!alive(pid));
}

#[test]
fn an_explicit_root_is_remembered_and_used() {
    let mut f = fixture(&[], &[("src/main.rs", MAIN), ("inner/x.rs", "fn x() {}\n")]);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("inner/x.rs");
    assert_eq!(f.s.state().provider().unwrap().root.as_deref(), Some(f.root.as_path()));
    let inner = f.root.join("inner");
    f.s.dispatch(Command::SetWorkspaceRoot(Some(inner.clone())));
    f.settle(|s| s.state().provider().is_some_and(|v| v.root.as_deref() == Some(inner.as_path()) && matches!(v.status, Status::Ready | Status::Indexing(_))));
    let roots: Vec<String> = messages(&f.log).iter().filter(|m| m["method"] == "initialize").map(|m| m["params"]["rootUri"].as_str().unwrap().to_owned()).collect();
    assert_eq!(roots.len(), 2);
    assert!(roots[1].ends_with("/proj/inner"));
    let file = f.root.parent().unwrap().join("config/providers.json");
    let saved = || std::fs::read_to_string(&file).unwrap_or_default();
    let until = Instant::now() + Duration::from_secs(5);
    while !saved().contains("/proj/inner") && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
    let settings: Value = serde_json::from_str(&saved()).unwrap();
    assert_eq!(settings["providers"]["rust-analyzer"]["roots"][f.root.to_str().unwrap()], inner.to_str().unwrap());
}

#[test]
fn large_files_are_never_sent_and_dependencies_show_their_origin() {
    let dir = tempfile::tempdir().unwrap();
    let dep = dir.path().join("home/.cargo/registry/src/index/dep-1.0");
    write(&dep, "src/lib.rs", "pub fn from_dep() {}\n");
    write(&dep, "src/sibling.rs", "pub fn sibling() {}\n");
    let extra = dep.to_str().unwrap().to_owned();
    let big = "fn x() {}\n".repeat(reader_core::document::LARGE_LINES + 1);
    let mut f = fixture(&[("FAKE_LSP_EXTRA", &extra)], &[("src/main.rs", "fn main() { from_dep(); }\n"), ("src/big.rs", &big)]);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.s.dispatch(Command::OpenRetained(f.root.join("src/big.rs")));
    f.settle(|s| s.state().current_source().is_some());
    assert_eq!(f.s.disabled(CommandId::Definition), Some("Large file: no language server"));
    f.open("src/main.rs");
    assert!(!messages(&f.log).iter().any(|m| m["params"]["textDocument"]["uri"].as_str().is_some_and(|u| u.ends_with("big.rs"))));
    f.definition("from_dep", false);
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source().starts_with(&dep)));
    assert_eq!(f.selected_text(), "from_dep");
    assert!(f.notice("Read-only source from a Cargo dependency"), "{:?}", f.s.state().notices());
    f.settle(|s| s.state().tree_rows().any(|r| r.name.as_ref() == "sibling.rs"));
    assert!(f.s.state().tree_rows().any(|r| r.name.as_ref() == "Libraries"));
    assert_eq!(f.s.state().project().unwrap().root(), f.root);
    assert_eq!(f.s.disabled(CommandId::LocateFile), None);
    assert!(!f.s.state().project().unwrap().children("src").iter().any(|(name, _)| name.as_ref() == "sibling.rs"));
    let sibling = dep.join("src/sibling.rs");
    f.s.dispatch(Command::ActivateTree { path: sibling.to_str().unwrap().into(), retain: false });
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source() == sibling));
    assert_eq!(current_text(f.s.state()).as_str(), "pub fn sibling() {}\n");
    assert_eq!(messages(&f.log).iter().filter(|m| m["method"] == "initialize").count(), 1, "library browsing reuses the project's server");
    f.s.dispatch(Command::OpenRoot(f._dir.path().join("another")));
    f.settle(|s| s.state().project().is_some_and(|p| !matches!(p.coverage(), reader_core::project::Coverage::Scanning(_))));
    assert!(!f.s.state().tree_rows().any(|r| r.name.as_ref() == "Libraries"));
}

fn percentile(samples: &mut [f64], p: f64) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[((samples.len() - 1) as f64 * p).round() as usize]
}

#[test]
fn indexing_progress_keeps_percentage_title_and_partial_reports() {
    let mut f = fixture(&[], &[("src/main.rs", MAIN)]);
    f.open("src/main.rs");
    f.settle(|s| s.state().provider().is_some_and(|p| p.status == Status::Ready));
    let mut progress = |token: Value, value: Value| {
        f.s.dispatch(Completion::Semantic(reader_core::semantics::Event::Message {
            session: 1,
            message: serde_json::json!({"jsonrpc":"2.0", "method":"$/progress", "params":{"token":token,"value":value}}),
        }));
        (f.s.state().language_server_label(), f.s.state().provider().unwrap().summary())
    };
    let (label, detail) = progress(serde_json::json!(7), serde_json::json!({"kind":"begin", "title":"Indexing Ruby files", "message":"app/models", "percentage":0}));
    assert!(label.contains("0%"), "{label}");
    assert!(detail.contains("Indexing Ruby files") && detail.contains("app/models"), "{detail}");
    let (label, detail) = progress(serde_json::json!(7), serde_json::json!({"kind":"report", "percentage":42}));
    assert!(label.contains("42%"), "{label}");
    assert!(detail.contains("Indexing Ruby files") && detail.contains("app/models"), "{detail}");
    let (label, detail) = progress(serde_json::json!(7), serde_json::json!({"kind":"report", "message":"app/controllers"}));
    assert!(label.contains("42%") && detail.contains("Indexing Ruby files") && detail.contains("app/controllers"), "{label}: {detail}");
    for percentage in [serde_json::json!(-1), serde_json::json!(101), serde_json::json!("80"), Value::Null] {
        let (label, _) = progress(serde_json::json!(7), serde_json::json!({"kind":"report", "percentage":percentage}));
        assert!(label.contains("42%"), "invalid percentage replaced the last valid report: {label}");
    }
    progress(serde_json::json!("other"), serde_json::json!({"kind":"begin", "title":"Loading gems"}));
    let (label, _) = progress(serde_json::json!(7), serde_json::json!({"kind":"end"}));
    assert!(label.contains("indexing") && !label.contains('%'), "another task is still running: {label}");
    let (label, _) = progress(serde_json::json!("other"), serde_json::json!({"kind":"end"}));
    assert_eq!(label, "LSP ready");
    let (label, _) = progress(serde_json::json!("next"), serde_json::json!({"kind":"begin", "title":"Loading gems"}));
    assert!(label.contains("indexing") && !label.contains('%'), "{label}");
}

/// An error the server logs or shows (Ruby LSP 0.26.11 logs a failed `initialized` and keeps running) is kept and
/// shown, without stopping the server; other log levels are not.
#[test]
fn server_errors_are_shown_while_it_keeps_running() {
    let mut f = fixture(&[], &[("src/main.rs", MAIN)]);
    f.open("src/main.rs");
    f.settle(|s| s.state().provider().is_some_and(|p| p.status == Status::Ready));
    let mut message = |method: &str, kind: u64, text: &str| {
        f.s.dispatch(Completion::Semantic(reader_core::semantics::Event::Message {
            session: 1,
            message: serde_json::json!({"jsonrpc":"2.0", "method":method, "params":{"type":kind, "message":text}}),
        }));
        (f.s.state().language_server_label(), f.s.language_server_details(), f.s.state().provider().unwrap().status.clone())
    };
    let (label, details, _) = message("window/logMessage", 4, "Finished initializing");
    assert_eq!(label, "LSP ready");
    assert!(!details.contains("Server error"), "{details}");
    let (label, details, status) = message("window/logMessage", 1, "Error processing initialized: Unable to activate rubocop-1.73.2 (Gem::ConflictError)\n\tfrom specification.rb:1383");
    assert_eq!((label.as_str(), status), ("LSP error · ready", Status::Ready), "shown, still running");
    assert!(details.contains("Server error: Error processing initialized") && details.contains("Gem::ConflictError") && details.contains("specification.rb:1383"), "{details}");
    let (_, details, _) = message("window/showMessage", 1, "Error while indexing");
    assert!(details.contains("Error while indexing") && !details.contains("Gem::ConflictError"), "the latest error: {details}");
    assert!(f.s.state().notices().iter().any(|n| n.contains("Error while indexing")), "a shown error is still a notice");
}

/// Release timing with the fake server: the reader's own cost of a definition round trip (the fake answers
/// instantly), and dispatch latency while a server floods progress during startup.
#[test]
#[ignore = "timing: run in release, alone"]
fn definition_perf() {
    let lib: String = (0..2000).map(|i| format!("pub fn f{i}() {{}}\n")).collect();
    let mut f = fixture(&[("FAKE_LSP_PROGRESS_FLOOD", "20000")], &[("src/lib.rs", &lib), ("src/main.rs", "fn main() { f1999(); }\n")]);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    let path = f.root.join("src/main.rs");
    f.s.dispatch(Command::Open(path.clone()));
    // Every input applied while the server starts and reports 20,000 progress messages.
    let mut dispatch = Vec::new();
    let rx = f.s.completions();
    let until = Instant::now() + Duration::from_secs(60);
    let mut quiet = 0;
    // Until the server is ready and has gone quiet (the whole flood applied).
    while !(f.s.state().provider().is_some_and(|v| v.status == Status::Ready) && f.s.state().current_source().is_some() && quiet >= 4) {
        assert!(Instant::now() < until);
        let next = recv(&rx);
        quiet = if next.is_ok() { 0 } else { quiet + 1 };
        if let Ok(c) = next {
            let t = Instant::now();
            f.s.dispatch(c);
            dispatch.push(t.elapsed().as_secs_f64() * 1000.0);
            let t = Instant::now();
            f.s.dispatch(Command::Scroll(0));
            dispatch.push(t.elapsed().as_secs_f64() * 1000.0);
        }
    }
    let n = dispatch.len();
    println!("dispatch while the server loads: median {:.4} ms, p99 {:.4} ms, max {:.4} ms over {n}", percentile(&mut dispatch, 0.5), percentile(&mut dispatch, 0.99), percentile(&mut dispatch, 1.0));
    // Warm definitions: ask, wait for the target, go back; 1,000 after 20 untimed.
    let mut times = Vec::new();
    for i in 0..1020 {
        f.settle(|s| s.state().current_source().is_some_and(|d| d.source() == path));
        f.caret("f1999");
        let t = Instant::now();
        f.s.dispatch(Command::Definition { peek: false });
        f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("lib.rs")));
        if i >= 20 {
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        assert_eq!(f.selected_text(), "f1999");
        f.s.dispatch(Command::Back);
    }
    println!("READER_PERF {}", serde_json::json!({ "name": "reader/definition-fake-warm", "unit": "ms", "samples": times }));
    println!("warm definition to target shown (fake server): median {:.3} ms, p95 {:.3} ms over {}", percentile(&mut times, 0.5), percentile(&mut times, 0.95), times.len());
}

/// The real rust-analyzer on a small fixture: definition, peek and its reported version. Runs only when
/// rust-analyzer is installed (`READER_RUST_ANALYZER` names one, else PATH); the reader never installs it.
#[test]
#[ignore = "needs an installed rust-analyzer; run in release"]
fn real_rust_analyzer() {
    let exe = std::env::var_os("READER_RUST_ANALYZER").map(PathBuf::from).or_else(|| reader_core::semantics::provider::find_executable(Path::new("rust-analyzer"), None));
    let Some(exe) = exe.filter(|e| std::process::Command::new(e).arg("--version").output().is_ok_and(|o| o.status.success())) else {
        println!("SKIPPED: rust-analyzer is not installed");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("ra-fixture");
    write(&root, "Cargo.toml", "[package]\nname = \"ra_fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n");
    write(&root, "src/lib.rs", "pub mod shapes;\npub use shapes::Circle;\n\n/// Ünïcödé docs.\npub fn área(c: &Circle) -> f64 { c.r * c.r * 3.14 }\n");
    write(&root, "src/shapes.rs", "pub struct Circle {\n    pub r: f64,\n}\n\nimpl Circle {\n    pub fn new(r: f64) -> Self { Self { r } }\n}\n");
    write(&root, "src/main.rs", "use ra_fixture::{área, Circle};\nfn main() {\n    let s = \"𝄞\"; let c = Circle::new(1.0);\n    println!(\"{s} {}\", área(&c));\n}\n");
    let providers = dir.path().join("providers.json");
    std::fs::write(&providers, serde_json::json!({ "providers": { "rust-analyzer": { "executable": exe } } }).to_string()).unwrap();
    let s = Session::new(Config { themes: ThemePaths::under(&dir.path().join("c"), &dir.path().join("s"), "lenz-reader"), recent: dir.path().join("s/r.json"), settings: dir.path().join("s/settings.json"), providers, cache: dir.path().join("cache"), idle: reader_core::semantics::IDLE, clock: std::sync::Arc::new(Instant::now), overrides: ThemeOverrides::default(), dark: true });
    let mut f = Fixture { _dir: dir, root: root.clone(), log: PathBuf::new(), s };
    f.s.dispatch(Command::OpenRoot(root.clone()));
    let main = root.join("src/main.rs");
    f.s.dispatch(Command::Open(main.clone()));
    let started = Instant::now();
    let rx = f.s.completions();
    while !f.s.state().provider().is_some_and(|v| v.status == Status::Ready) {
        assert!(started.elapsed() < Duration::from_secs(180), "{:?}", f.s.state().provider());
        if let Ok(c) = recv(&rx) {
            f.s.dispatch(c);
        }
    }
    let view = f.s.state().provider().unwrap();
    println!("rust-analyzer {:?}: ready (quiescent) in {:.1} s", view.version, started.elapsed().as_secs_f64());
    let mut times = Vec::new();
    for i in 0..105 {
        f.settle(|s| s.state().current_source().is_some_and(|d| d.source() == main));
        f.caret("new(1.0)");
        let t = Instant::now();
        f.s.dispatch(Command::Definition { peek: false });
        f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("shapes.rs")) || s.state().asking().is_none() && !s.state().notices().is_empty());
        assert!(f.s.state().current_source().unwrap().source().ends_with("shapes.rs"), "{:?}", f.s.state().notices());
        if i >= 5 {
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        assert_eq!(f.selected_text(), "new");
        f.s.dispatch(Command::Back);
    }
    // Past a non-BMP character on the line: UTF-8 or UTF-16 columns, whichever was negotiated.
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source() == main));
    f.caret("área(&c)");
    f.s.dispatch(Command::Definition { peek: true });
    f.settle(|s| s.state().results().is_some_and(|r| r.preview().is_some()));
    assert!(f.s.state().results().unwrap().preview().unwrap().doc.source().ends_with("lib.rs"));
    assert_eq!(f.s.state().results().unwrap().hits()[0].text.as_ref(), "pub fn área(c: &Circle) -> f64 { c.r * c.r * 3.14 }");
    println!("READER_PERF {}", serde_json::json!({ "name": "reader/definition-rust-analyzer-warm", "unit": "ms", "samples": times }));
    println!("warm definition to target shown (rust-analyzer): median {:.1} ms, p95 {:.1} ms over {}", percentile(&mut times, 0.5), percentile(&mut times, 0.95), times.len());
}

#[test]
fn a_renamed_file_reopens_under_its_new_name_and_a_file_without_a_workspace_says_so() {
    let mut f = fixture(&[], &[("src/main.rs", MAIN)]);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("src/main.rs");
    std::fs::rename(f.root.join("src/main.rs"), f.root.join("src/renamed.rs")).unwrap();
    let log = f.log.clone();
    f.settle_until(move || {
        let m = messages(&log);
        let uri = |m: &Value| m["params"]["textDocument"]["uri"].as_str().unwrap_or_default().to_owned();
        m.iter().any(|m| m["method"] == "textDocument/didClose" && uri(m).ends_with("/main.rs")) && m.iter().any(|m| m["method"] == "textDocument/didOpen" && uri(m).ends_with("/renamed.rs"))
    });
    // Asked before its (missing) workspace is known: the answer is why, not an endless wait.
    let loose = write(f.root.parent().unwrap(), "loose/x.rs", "fn x() { x(); }\n");
    f.s.dispatch(Command::Open(loose));
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("x.rs")));
    f.caret("x()");
    f.s.dispatch(Command::Definition { peek: false });
    f.asked();
    assert!(f.notice("no Cargo.toml or rust-project.json encloses this file; set a workspace root"), "{:?}", f.s.state().notices());
}

fn tick(f: &mut Fixture) {
    f.s.dispatch(Completion::Semantic(reader_core::semantics::Event::Tick));
}

fn pids(log: &Path) -> Vec<u64> {
    messages(log).iter().filter_map(|m| m["pid"].as_u64()).collect()
}

const USES: &str = "fn main() {\n    helper();\n    let x = helper;\n}\n";

#[test]
fn usages_list_semantic_references_with_their_provenance() {
    let mut f = fixture(&[], &[("src/lib.rs", "pub fn helper() {}\npub fn lonely() {}\n"), ("src/main.rs", USES), ("src/other.rs", "fn o() { crate::helper(); }\n")]);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("src/main.rs");
    f.caret("helper();");
    f.s.dispatch(Command::Usages);
    f.settle(|s| s.state().results().is_some_and(|r| r.results_kind() == ResultsKind::Usages && r.preview().is_some()));
    let r = f.s.state().results().unwrap();
    assert_eq!(r.kind(), "Usages (rust-analyzer · proj)");
    let mut places: Vec<(String, usize)> = r.hits().iter().map(|h| (Path::new(&*h.path).file_name().unwrap().to_string_lossy().into_owned(), h.line)).collect();
    places.sort();
    assert_eq!(places, [("main.rs".to_string(), 1), ("main.rs".to_string(), 2), ("other.rs".to_string(), 0)], "the declaration is not a usage");
    assert!(f.s.state().current_source().unwrap().source().ends_with("main.rs"), "listing usages does not move");
    // Rerun asks about the same place, wherever the caret is now.
    f.caret("main");
    f.s.dispatch(Command::RerunSearch);
    f.settle(|s| s.state().asking().is_none() && s.state().results().is_some_and(|r| r.hits().len() == 3));
    // Opening one is a validated, committed navigation.
    let i = f.s.state().results().unwrap().hits().iter().position(|h| h.path.ends_with("other.rs")).unwrap();
    f.s.dispatch(Command::OpenResult(i));
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("other.rs")));
    assert_eq!(f.selected_text(), "helper");
    // None: said so, with the provider's limits.
    f.s.dispatch(Command::Open(f.root.join("src/lib.rs")));
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("lib.rs")));
    f.caret("lonely");
    f.s.dispatch(Command::Usages);
    f.asked();
    assert!(f.notice("Usages: rust-analyzer found no usages. Known limits:"), "{:?}", f.s.state().notices());
    // Unsupported by the server: not an empty success.
    let mut g = fixture(&[("FAKE_LSP_NO_REFERENCES", "1")], &[("src/main.rs", USES)]);
    g.open("src/main.rs");
    g.caret("helper();");
    g.s.dispatch(Command::Usages);
    assert!(g.notice("Usages: rust-analyzer does not provide usages"), "{:?}", g.s.state().notices());
}

#[test]
fn only_servers_relying_on_the_client_hear_of_changed_files() {
    // rust-analyzer watches files itself: it is neither offered nor sent changed-file reports (Ruby LSP is: its
    // real test edits a closed file).
    let mut f = fixture(&[], &[("src/main.rs", MAIN), ("src/lib.rs", LIB)]);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("src/main.rs");
    f.settle(ready);
    std::fs::write(f.root.join("src/lib.rs"), format!("// moved\n{LIB}")).unwrap();
    f.settle(|s| s.state().index_coverage().indexed > 0);
    std::thread::sleep(std::time::Duration::from_millis(500));
    f.s.dispatch(Command::Recheck);
    let log = messages(&f.log);
    assert!(log.iter().all(|m| m["method"] != "workspace/didChangeWatchedFiles"));
    let init = log.iter().find(|m| m["method"] == "initialize").unwrap();
    assert!(init["params"]["capabilities"]["workspace"].get("didChangeWatchedFiles").is_none());
}

#[test]
fn ruby_receives_coarse_changes_for_closed_added_renamed_and_deleted_files() {
    for overflow in [false, true] {
        let mut f = fixture_as("ruby-lsp", &[], &[
            ("Gemfile", ""), ("main.rb", "class Main; end\n"),
            ("app/a.rb", "class Alpha; end\n"), ("app/gone.rb", "class Gone; end\n"),
            ("app/move.rb", "class Moved; end\n"), ("apple/stable.rb", "class Stable; end\n"),
        ], serde_json::json!({}), std::sync::Arc::new(Instant::now));
        f.s.dispatch(Command::OpenRoot(f.root.clone()));
        f.open("main.rb");
        f.settle(|s| s.state().project().is_some_and(|p| !p.refreshing() && matches!(p.coverage(), reader_core::project::Coverage::Complete(_))) && s.state().index_coverage().complete());
        let start = messages(&f.log).len();
        write(&f.root, "app/a.rb", "class Omega; end\n");
        std::fs::remove_file(f.root.join("app/gone.rb")).unwrap();
        std::fs::rename(f.root.join("app/move.rb"), f.root.join("app/renamed.rb")).unwrap();
        write(&f.root, "app/new.rb", "class Added; end\n");
        f.s.dispatch(Completion::Changed(reader_core::watch::Change {
            paths: if overflow { Default::default() } else { [f.root.join("app")].into() }, overflow, ..Default::default()
        }));
        let rx = f.s.completions();
        let until = Instant::now() + Duration::from_secs(10);
        // Precise OS notifications would hide a missing directory/overflow notification.
        while f.s.state().project().unwrap().refreshing() || !f.s.state().index_coverage().complete() {
            assert!(Instant::now() < until);
            if let Ok(c) = rx.try_recv() && !matches!(c, Completion::Changed(_)) { f.s.dispatch(c); }
            std::thread::yield_now();
        }
        f.s.dispatch(Command::FileSymbols);
        assert!(f.s.state().asking().is_some());
        while f.s.state().asking().is_some() {
            assert!(Instant::now() < until);
            if let Ok(c) = rx.try_recv() && !matches!(c, Completion::Changed(_)) { f.s.dispatch(c); }
            std::thread::yield_now();
        }
        let log = messages(&f.log);
        let events: Vec<_> = log[start..].iter().filter(|m| m["method"] == "workspace/didChangeWatchedFiles")
            .flat_map(|m| m["params"]["changes"].as_array().unwrap()).collect();
        for (name, kind) in [("app/a.rb", 2), ("app/gone.rb", 3), ("app/move.rb", 3), ("app/renamed.rb", 2), ("app/new.rb", 2)] {
            let uri = reader_core::semantics::lsp::file_uri(&f.root.join(name));
            assert!(events.iter().any(|e| e["uri"] == uri && e["type"] == kind), "missing {name} ({kind}), overflow={overflow}: {events:?}");
        }
        assert!(!events.iter().any(|e| e["uri"].as_str().unwrap().ends_with("apple/stable.rb")), "unchanged file was reported, overflow={overflow}");
        assert_eq!(log.iter().filter(|m| m["method"] == "initialize").count(), 1, "file changes must not restart the server");
    }
}

// Suppress real watch delivery so it cannot conceal a missing recovery notification.
fn settle_without_watch(f: &mut Fixture, done: impl Fn(&Session) -> bool) {
    let until = Instant::now() + Duration::from_secs(10);
    while !done(&f.s) {
        assert!(Instant::now() < until, "recovery timed out: {:?}; {:?}; {:?}", f.s.state().index_coverage(), f.s.state().provider(), f.s.state().notices());
        if let Ok(c) = f.s.completions().try_recv() && !matches!(c, Completion::Changed(_)) { f.s.dispatch(c); }
        std::thread::yield_now();
    }
}

#[test]
fn ruby_overflow_skips_unchanged_contents_but_preserves_updates_reversals_and_optout() {
    let mut f = fixture_as("ruby-lsp", &[], &[
        ("Gemfile", ""), ("main.rb", "class Main; end\n"),
        ("app/a.rb", "class Alpha; end\n"), ("app/stable.rb", "class Stable; end\n"),
    ], serde_json::json!({}), std::sync::Arc::new(Instant::now));
    f.s.dispatch(Command::OpenRoot(f.root.clone())); f.open("main.rb");
    settle_without_watch(&mut f, |s| ready(s) && s.state().index_coverage().complete() && s.state().index_coverage().indexed == 4);
    let path = f.root.join("app/a.rb");
    let time = std::fs::metadata(&path).unwrap().modified().unwrap();
    // An exact-file change, a directory change and the subsequent overflow must all advance the baseline.
    for (text, event, changed) in [
        ("class Omega; end\n", 0, true), ("class Omega; end\n", 0, false),
        ("class Alpha; end\n", 1, true), ("class Omega; end\n", 0, true),
        ("class Alpha; end\n", 2, true), ("class Omega; end\n", 0, true),
        ("class Alpha; end\n", 3, true),
    ] {
        std::fs::write(&path, text).unwrap();
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(time).unwrap();
        let start = messages(&f.log).len();
        f.s.dispatch(Completion::Changed(reader_core::watch::Change {
            overflow: event == 0 || event == 3,
            paths: match event { 1 | 3 => [path.clone()].into(), 2 => [f.root.join("app")].into(), _ => Default::default() },
            ..Default::default()
        }));
        settle_without_watch(&mut f, |s| !s.state().project().unwrap().refreshing() && s.state().index_coverage().complete());
        f.s.dispatch(Command::FileSymbols); settle_without_watch(&mut f, |s| s.state().asking().is_none());
        let all = messages(&f.log);
        let events: Vec<_> = all[start..].iter().filter(|m| m["method"] == "workspace/didChangeWatchedFiles")
            .flat_map(|m| m["params"]["changes"].as_array().unwrap()).collect();
        assert_eq!(events.iter().any(|e| e["uri"] == reader_core::semantics::lsp::file_uri(&path)), changed, "{text}, event={event}: {events:?}");
        assert_eq!(events.iter().any(|e| e["uri"].as_str().unwrap().ends_with("app/stable.rb")), event == 2, "only ordinary directory events retain broad reports");
    }
    f.s.dispatch(Command::SetPersistentIndex(false));
    settle_without_watch(&mut f, |s| s.state().index_coverage().complete());
    let start = messages(&f.log).len();
    f.s.dispatch(Completion::Changed(reader_core::watch::Change { overflow: true, ..Default::default() }));
    settle_without_watch(&mut f, |s| !s.state().project().unwrap().refreshing() && s.state().index_coverage().complete());
    f.s.dispatch(Command::FileSymbols); settle_without_watch(&mut f, |s| s.state().asking().is_none());
    assert!(messages(&f.log)[start..].iter().filter(|m| m["method"] == "workspace/didChangeWatchedFiles")
        .flat_map(|m| m["params"]["changes"].as_array().unwrap()).any(|e| e["uri"].as_str().unwrap().ends_with("app/stable.rb")), "opt-out restores full recovery");
    assert_eq!(std::fs::read_to_string(path).unwrap(), "class Alpha; end\n");
}

#[test]
fn ruby_overflow_handles_both_completion_orders_registration_and_newer_changes() {
    for scan_first in [false, true] {
        for overlap in [false, true] {
            let mut f = fixture_as("ruby-lsp", &[], &[
                ("Gemfile", ""), ("main.rb", "class Main; end\n"),
                ("a.rb", "class Alpha; end\n"), ("stable.rb", "class Stable; end\n"),
            ], serde_json::json!({}), std::sync::Arc::new(Instant::now));
            f.s.dispatch(Command::OpenRoot(f.root.clone())); f.open("main.rb");
            settle_without_watch(&mut f, |s| ready(s) && s.state().index_coverage().complete() && s.state().index_coverage().indexed == 4);
            let start = messages(&f.log).len();
            let path = write(&f.root, "a.rb", "class Omega; end\n");
            f.s.dispatch(Completion::Changed(reader_core::watch::Change { overflow: true, ..Default::default() }));
            let mut scan = Vec::new(); let mut scan_done = false; let mut indexed = None;
            let until = Instant::now() + Duration::from_secs(10);
            while !scan_done || indexed.is_none() {
                assert!(Instant::now() < until);
                match f.s.completions().try_recv() {
                    Ok(c @ Completion::Indexed { .. }) => indexed = Some(c),
                    Ok(c @ Completion::Scanned { .. }) => {
                        if let Completion::Scanned { batch, .. } = &c { scan_done |= batch.done; }
                        scan.push(c);
                    }
                    Ok(Completion::Changed(_)) => {},
                    Ok(c) => { f.s.dispatch(c); },
                    Err(_) => std::thread::yield_now(),
                }
            }
            if scan_first { for c in scan.drain(..) { f.s.dispatch(c); } }
            else { f.s.dispatch(indexed.take().unwrap()); }
            f.s.dispatch(Completion::Changed(reader_core::watch::Change { registered: [f.root.clone()].into(), ..Default::default() }));
            write(&f.root, "a.rb", "class Gamma; end\n");
            f.s.dispatch(Completion::Changed(reader_core::watch::Change {
                paths: if overlap { Default::default() } else { [path].into() }, overflow: overlap, ..Default::default()
            }));
            if scan_first { f.s.dispatch(indexed.take().unwrap()); }
            else { for c in scan { f.s.dispatch(c); } }
            settle_without_watch(&mut f, |s| !s.state().project().unwrap().refreshing() && s.state().index_coverage().complete());
            f.s.dispatch(Command::FindDeclarations("Gamma".into()));
            settle_without_watch(&mut f, |s| s.state().results().is_some_and(|r| r.done()));
            assert!(f.s.state().results().unwrap().hits().iter().any(|h| h.text.as_ref() == "Gamma"));
            f.s.dispatch(Command::FileSymbols); settle_without_watch(&mut f, |s| s.state().asking().is_none());
            let log = messages(&f.log);
            let events: Vec<_> = log[start..].iter().filter(|m| m["method"] == "workspace/didChangeWatchedFiles")
                .flat_map(|m| m["params"]["changes"].as_array().unwrap()).collect();
            assert!(events.iter().any(|e| e["uri"].as_str().unwrap().ends_with("/a.rb")));
            assert_eq!(events.iter().any(|e| e["uri"].as_str().unwrap().ends_with("/stable.rb")), overlap,
                "overlap falls back; registration alone does not lose deferred comparisons: scan_first={scan_first}");
        }
    }
}

#[test]
fn ruby_refreshes_a_changed_open_document_and_rechecks_it_after_close() {
    let mut f = fixture_as("ruby-lsp", &[], &[
        ("Gemfile", ""), ("a.rb", "class Alpha; end\n"), ("b.rb", "class Beta; end\n"),
    ], serde_json::json!({}), std::sync::Arc::new(Instant::now));
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("a.rb");
    let start = messages(&f.log).len();
    write(&f.root, "a.rb", "class Omega; end\n");
    f.s.dispatch(Completion::Changed(reader_core::watch::Change { paths: [f.root.join("a.rb")].into(), ..Default::default() }));
    f.settle(|s| s.state().current_source().is_some_and(|d| d.copy_range(0..d.len_bytes()).unwrap().contains("Omega")));
    let log = f.log.clone();
    f.settle_until(move || messages(&log)[start..].iter().any(|m| m["method"] == "textDocument/didChange"));
    f.open("b.rb");
    let log = f.log.clone();
    let uri = reader_core::semantics::lsp::file_uri(&f.root.join("a.rb"));
    f.settle_until(move || {
        let messages = messages(&log);
        let tail = &messages[start..];
        let Some(closed) = tail.iter().position(|m| m["method"] == "textDocument/didClose" && m["params"]["textDocument"]["uri"] == uri) else { return false };
        tail[closed + 1..].iter().any(|m| m["method"] == "workspace/didChangeWatchedFiles" && m["params"]["changes"].as_array().unwrap().iter().any(|c| c["uri"] == uri && c["type"] == 2))
    });
    let log = messages(&f.log);
    let tail = &log[start..];
    let changed = tail.iter().position(|m| m["method"] == "textDocument/didChange").unwrap();
    let closed = tail.iter().position(|m| m["method"] == "textDocument/didClose").unwrap();
    assert!(tail[changed + 1..closed].iter().any(|m| m["method"] == "textDocument/documentSymbol"), "refresh must follow the new text");
    assert!(f.s.state().results().is_none(), "the internal reply must not open a symbol panel");
    assert!(f.s.state().asking().is_none());
}

#[test]
fn servers_without_workspace_symbols_say_so_rather_than_finding_nothing() {
    // As the VS Code HTML and CSS servers: definitions and file symbols, no workspace symbols.
    let mut f = fixture(&[("FAKE_LSP_NO_WORKSPACE_SYMBOLS", "1")], &[("src/main.rs", USES)]);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("src/main.rs");
    f.settle(ready);
    assert_eq!(f.s.disabled(CommandId::Symbols), Some("The language servers running for this folder do not provide workspace symbols; find declarations instead"));
    f.s.dispatch(Command::Symbols("helper".into()));
    assert!(f.notice("Symbols: rust-analyzer does not provide workspace symbols"), "{:?}", f.s.state().notices());
    assert!(f.s.state().results().is_none_or(|r| r.results_kind() != ResultsKind::Symbols), "not an empty success");
}

#[test]
fn workspace_symbols_come_from_every_running_server_of_the_folder() {
    let mut f = fixture(&[], &[]);
    std::fs::remove_file(f.root.join("Cargo.toml")).unwrap();
    for (p, name) in [("a", "alpha_helper"), ("b", "beta_helper")] {
        write(&f.root, &format!("{p}/Cargo.toml"), &format!("[package]\nname = \"{p}\"\n"));
        write(&f.root, &format!("{p}/src/lib.rs"), &format!("pub fn {name}() {{}}\npub struct Other;\n"));
    }
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    // Nothing running yet: the command says why rather than returning nothing.
    assert_eq!(f.s.disabled(CommandId::Symbols), Some("No language server is running for this folder yet; open one of its source files"));
    f.s.dispatch(Command::Symbols("helper".into()));
    assert!(f.notice("Symbols: No language server is running"));
    for p in ["a", "b"] {
        f.s.dispatch(Command::OpenRetained(f.root.join(format!("{p}/src/lib.rs"))));
        f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with(format!("{p}/src/lib.rs"))) && ready(s));
    }
    f.settle(|s| s.state().servers().iter().filter(|v| matches!(v.status, Status::Ready)).count() == 2);
    f.s.dispatch(Command::Symbols("HELPER".into()));
    f.settle(|s| s.state().asking().is_none() && s.state().results().is_some_and(|r| r.done()));
    let r = f.s.state().results().unwrap();
    assert_eq!(r.kind(), "Symbols (language servers)");
    let mut names: Vec<String> = r.hits().iter().map(|h| h.text[h.range.clone()].to_string()).collect();
    names.sort();
    assert_eq!(names, ["alpha_helper", "beta_helper"]);
    let mut origins: Vec<String> = r.hits().iter().map(|h| h.origin.as_deref().unwrap_or_default().to_string()).collect();
    origins.sort();
    assert_eq!(origins, ["rust-analyzer · a", "rust-analyzer · b"], "merged answers keep their provider and project");
    f.s.dispatch(Command::OpenResult(0));
    f.settle(|s| s.state().selection().is_some_and(|_| s.state().current_source().is_some_and(|d| d.copy_range(0..d.len_bytes()).unwrap().contains("_helper"))));
}

#[test]
fn launch_overrides_settings_and_configuration_are_the_users() {
    let settings = serde_json::json!({ "args": ["--mark"], "env": { "FAKE_LSP_MARK": "from-settings" }, "settings": { "checkOnSave": true, "cargo": { "features": "all" } } });
    let mut f = fixture_with(&[("FAKE_LSP_ASK_CONFIG", "1")], &[("src/main.rs", USES)], settings, std::sync::Arc::new(Instant::now));
    f.open("src/main.rs");
    let log = f.log.clone();
    f.settle_until(move || messages(&log).iter().any(|m| m["id"] == "cfg-1"));
    let log = messages(&f.log);
    let launch = log.iter().find(|m| m.get("args").is_some()).unwrap();
    assert_eq!((&launch["args"], &launch["env"]), (&serde_json::json!(["--mark"]), &serde_json::json!("from-settings")));
    let init = log.iter().find(|m| m["method"] == "initialize").unwrap();
    assert_eq!(init["params"]["initializationOptions"], serde_json::json!({ "checkOnSave": true, "cargo": { "features": "all" } }));
    let cfg = log.iter().find(|m| m["id"] == "cfg-1").unwrap();
    assert_eq!(cfg["result"], serde_json::json!([{ "checkOnSave": true, "cargo": { "features": "all" } }, { "features": "all" }]));
    // An executable the launch's own PATH lacks: explained, without reading shell startup files.
    let mut g = fixture_with(&[], &[("src/main.rs", USES)], serde_json::json!({ "env": { "PATH": "/nonexistent" } }), std::sync::Arc::new(Instant::now));
    let providers = g.root.parent().unwrap().join("config/providers.json");
    std::fs::write(&providers, r#"{"providers":{"rust-analyzer":{"executable":"rust-analyzer","env":{"PATH":"/nonexistent"}}}}"#).unwrap();
    g.s = Session::new(Config {
        themes: ThemePaths::under(&g.root.join("../c3"), &g.root.join("../s3"), "lenz-reader"),
        recent: g.root.join("../s3/recent.json"),
        settings: g.root.join("../s3/settings.json"),
        providers,
        cache: f.root.join("../cache"),
        idle: reader_core::semantics::IDLE,
        clock: std::sync::Arc::new(Instant::now),
        overrides: ThemeOverrides::default(),
        dark: true,
    });
    g.s.dispatch(Command::Open(g.root.join("src/main.rs")));
    g.settle(|s| s.state().provider().is_some_and(|v| matches!(v.status, Status::Missing(_))));
    let Status::Missing(why) = g.s.state().provider().unwrap().status else { unreachable!() };
    assert!(why.contains("1 directories searched") && why.contains("does not read shell startup files"), "{why}");
}

/// A Cargo dependency outside the project (a git checkout with a workspace of its own, as `~/.cargo/git` holds):
/// shown after a jump, it joins the project's server, never one of its own for the dependency's whole workspace.
#[test]
fn a_dependency_outside_the_project_joins_the_running_server() {
    let mut f = fixture(&[], &[("src/main.rs", MAIN), ("src/lib.rs", LIB)]);
    f.open("src/main.rs");
    let dep = f.root.parent().unwrap().join("home/.cargo/git/checkouts/dep-0123/abc");
    write(&dep, "Cargo.toml", "[workspace]\nmembers = [\"crates/x\"]\n");
    write(&dep, "crates/x/Cargo.toml", "[package]\nname = \"x\"\n");
    let file = write(&dep, "crates/x/src/lib.rs", "pub fn x() {}\n");
    f.s.dispatch(Command::Open(file.clone()));
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source() == file) && ready(s));
    let log = f.log.clone();
    f.settle_until(move || messages(&log).iter().any(|m| m["method"] == "textDocument/didOpen" && m["params"]["textDocument"]["uri"].as_str().is_some_and(|u| u.ends_with("crates/x/src/lib.rs"))));
    assert_eq!(f.s.state().servers().len(), 1, "{:?}", f.s.state().servers());
    assert_eq!(f.s.state().provider().unwrap().root.as_deref(), Some(f.root.as_path()));
}

/// With two servers running, a dependency joins the one last asked (whose jump opened it), not the first started:
/// the other's crate graph may not hold it.
#[test]
fn a_dependency_joins_the_server_last_asked() {
    let mut f = fixture(&[], &[("src/main.rs", MAIN), ("src/lib.rs", LIB)]);
    let other = f.root.parent().unwrap().join("other");
    write(&other, "Cargo.toml", "[package]\nname = \"other\"\n");
    let main = write(&other, "src/main.rs", MAIN);
    f.open("src/main.rs");
    f.s.dispatch(Command::Open(main.clone()));
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source() == main) && ready(s) && s.state().servers().len() == 2);
    let log = f.log.clone();
    f.settle_until(move || messages(&log).iter().any(|m| m["method"] == "textDocument/didOpen" && m["params"]["textDocument"]["uri"].as_str().is_some_and(|u| u.ends_with("other/src/main.rs"))));
    f.definition("helper()", false);
    f.asked();
    let dep = f.root.parent().unwrap().join("home/.cargo/git/checkouts/dep-0123/abc");
    let file = write(&dep, "src/lib.rs", "pub fn x() {}\n");
    write(&dep, "Cargo.toml", "[package]\nname = \"x\"\n");
    f.s.dispatch(Command::Open(file.clone()));
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source() == file) && ready(s));
    assert_eq!(f.s.state().provider().unwrap().root.as_deref(), Some(other.as_path()));
    assert_eq!(f.s.state().servers().len(), 2);
}

/// A file inside a running server's workspace or under the browsing root keeps its own workspace rules even where
/// dependencies live by path (a vendored checkout, a project's node_modules or virtualenv): only a dependency
/// outside both joins.
#[test]
fn a_dependency_inside_a_workspace_or_the_browsing_root_keeps_its_own_rules() {
    let mut f = fixture(&[], &[("src/main.rs", MAIN)]);
    f.open("src/main.rs");
    let inner = f.root.join("vendor/.cargo/registry/src/index-0123/y-1.0.0");
    write(&inner, "Cargo.toml", "[package]\nname = \"y\"\n");
    let file = write(&inner, "src/lib.rs", "pub fn y() {}\n");
    f.s.dispatch(Command::Open(file.clone()));
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source() == file) && ready(s));
    assert_eq!(f.s.state().provider().unwrap().root.as_deref(), Some(inner.as_path()), "inside the project's workspace");
    // Under the browsing root, outside every workspace (opening a folder stops its servers: the project's again).
    let browse = f.root.parent().unwrap().to_owned();
    f.s.dispatch(Command::OpenRoot(browse.clone()));
    f.open("src/main.rs");
    let dep = browse.join("vendor/.cargo/registry/src/index-0123/x-1.0.0");
    write(&dep, "Cargo.toml", "[package]\nname = \"x\"\n");
    let file = write(&dep, "src/lib.rs", "pub fn x() {}\n");
    f.s.dispatch(Command::Open(file.clone()));
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source() == file) && ready(s));
    assert_eq!(f.s.state().provider().unwrap().root.as_deref(), Some(dep.as_path()));
}

/// A dependency opened with no server running (from recent places, say): its own server, as any project, even
/// under a browsing root (whose folder may hold several projects).
#[test]
fn a_dependency_opened_first_gets_its_own_server() {
    let mut f = fixture(&[], &[("src/main.rs", MAIN)]);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    let dep = f.root.parent().unwrap().join("home/.cargo/registry/src/index-0123/x-1.0.0");
    write(&dep, "Cargo.toml", "[package]\nname = \"x\"\n");
    let file = write(&dep, "src/lib.rs", "pub fn x() {}\n");
    f.s.dispatch(Command::Open(file.clone()));
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source() == file) && ready(s));
    assert_eq!(f.s.state().provider().unwrap().root.as_deref(), Some(dep.as_path()));
}

#[test]
fn idle_servers_stop_on_the_clock_but_not_while_a_tab_holds_their_files() {
    let now = std::sync::Arc::new(std::sync::Mutex::new(Instant::now()));
    let clock = now.clone();
    let mut f = fixture_with(&[], &[("src/main.rs", USES), ("src/lib.rs", "pub fn helper() {}\n"), ("README.md", "# x\n")], serde_json::json!({}), std::sync::Arc::new(move || *clock.lock().unwrap()));
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("src/main.rs");
    let pid = pids(&f.log)[0];
    // Another file replaces it in the preview tab, but a kept tab still holds lib.rs: warm.
    f.s.dispatch(Command::OpenRetained(f.root.join("src/lib.rs")));
    f.s.dispatch(Command::Open(f.root.join("README.md")));
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("README.md")));
    *now.lock().unwrap() += Duration::from_secs(3600);
    tick(&mut f);
    assert_eq!(f.s.state().servers().len(), 1, "a kept tab holds its file");
    // Closed: idle from now; stopped once the idle period passes, not before.
    let lib = f.s.state().tabs().iter().find(|t| t.source().ends_with("lib.rs")).unwrap().id();
    f.s.dispatch(Command::CloseTab(lib));
    tick(&mut f);
    *now.lock().unwrap() += reader_core::semantics::IDLE - Duration::from_secs(1);
    tick(&mut f);
    assert_eq!(f.s.state().servers().len(), 1);
    *now.lock().unwrap() += Duration::from_secs(2);
    tick(&mut f);
    assert!(f.s.state().servers().is_empty());
    let until = Instant::now() + Duration::from_secs(8);
    while alive(pid) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!alive(pid));
    // Opening a Rust file again starts a new one.
    f.open("src/main.rs");
    assert_eq!(pids(&f.log).len(), 2);
}

#[test]
fn disable_and_restart_are_per_folder_and_invalidate_old_answers() {
    let mut f = fixture(&[], &[("src/main.rs", "fn main() { slow_one(); crash_now(); }\nfn slow_one() {}\n")]);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("src/main.rs");
    // Restart while a slow answer is pending: the old session's answer is not used.
    f.definition("slow_one();", false);
    f.s.dispatch(Command::RestartServer);
    assert!(f.s.state().asking().is_none());
    f.settle(ready);
    std::thread::sleep(Duration::from_millis(500));
    while let Ok(c) = recv(&f.s.completions()) {
        f.s.dispatch(c);
    }
    assert_eq!(f.s.state().selection().unwrap().range().start, "fn main() { s".len(), "the old answer did not move the view");
    assert_eq!(pids(&f.log).len(), 2);
    // After a crash, Restart brings it back (never automatically).
    f.definition("crash_now", false);
    f.settle(|s| s.state().provider().is_some_and(|v| matches!(v.status, Status::Failed(_))));
    assert_eq!(f.s.disabled(CommandId::RestartServer), None);
    assert_eq!(f.s.state().language_server_label(), "LSP stopped");
    assert!(f.s.language_server_details().contains("After fixing setup"));
    f.s.dispatch(Command::RestartServer);
    f.settle(ready);
    assert_eq!(pids(&f.log).len(), 3);
    // Disabled for this folder: stopped, explained, remembered; reading goes on.
    f.s.dispatch(Command::SetServerEnabled(false));
    assert_eq!(f.s.state().language_server_label(), "LSP off");
    assert!(f.s.language_server_rows().iter().any(|(_, id)| *id == Some(CommandId::EnableServer)));
    assert!(!f.s.language_server_rows().iter().any(|(_, id)| *id == Some(CommandId::DisableServer)));
    assert_eq!(f.s.state().provider().unwrap().status, Status::Disabled);
    assert!(f.s.state().servers().is_empty());
    assert_eq!(f.s.disabled(CommandId::Definition), Some("The language server is disabled for this folder"));
    f.settle(|s| s.disabled(CommandId::EnableServer).is_none());
    assert_eq!(f.s.disabled(CommandId::DisableServer), Some("Already disabled for this folder"));
    let file = f.root.parent().unwrap().join("config/providers.json");
    let saved = || std::fs::read_to_string(&file).unwrap_or_default();
    let until = Instant::now() + Duration::from_secs(5);
    while !saved().contains("disabled") && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
    let settings: Value = serde_json::from_str(&saved()).unwrap();
    assert_eq!(settings["providers"]["rust-analyzer"]["disabled"], serde_json::json!([f.root]));
    f.s.dispatch(Command::Search { query: "slow_one".into(), scope: reader_core::session::SearchIn::ActiveFile });
    f.settle(|s| s.state().results().is_some_and(|r| r.done() && r.hits().len() == 2));
    f.s.dispatch(Command::SetServerEnabled(true));
    f.settle(ready);
    assert_eq!(pids(&f.log).len(), 4);
}

#[test]
fn server_memory_counts_the_server_and_its_helpers_separately() {
    let mut f = fixture(&[("FAKE_LSP_ALLOC_MB", "64"), ("FAKE_LSP_CHILD", "1")], &[("src/main.rs", USES)]);
    f.open("src/main.rs");
    tick(&mut f);
    f.settle(|s| s.state().provider().is_some_and(|v| v.memory.is_some_and(|(server, helpers)| server >= 64 << 20 && helpers > 0)));
    let view = f.s.state().provider().unwrap();
    let (server, helpers) = view.memory.unwrap();
    assert!(helpers < server / 4, "the helper (a sleep) is counted apart: {server} {helpers}");
    assert!(view.summary().contains(" MiB + ") && view.summary().ends_with(" MiB helpers"), "{}", view.summary());
    assert!(reader_core::semantics::memory_notice(server + helpers).is_none(), "below the notice threshold");
    assert!(!f.s.state().notices().iter().any(|n| n.starts_with("Language servers use")));
    assert_eq!(reader_core::semantics::memory_notice(3 << 30).as_deref(), Some("Language servers use 3.0 GiB (above 2 GiB); disable or restart ones you do not need"));
}

/// Release timing with the fake server: usages (500 references over 50 files, all read and located by the
/// reader) from asking to results listed, 200 samples after 10 untimed.
#[test]
#[ignore = "timing: run in release, alone"]
fn usages_perf() {
    let mut files: Vec<(String, String)> = (0..50).map(|i| (format!("src/m{i}.rs"), (0..10).map(|j| format!("fn u{i}_{j}() {{ target(); }}\n")).collect())).collect();
    files.push(("src/lib.rs".into(), "pub fn target() {}\n".into()));
    let files: Vec<(&str, &str)> = files.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let mut f = fixture(&[], &files);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("src/lib.rs");
    let mut times = Vec::new();
    for i in 0..210 {
        f.caret("target");
        let t = Instant::now();
        f.s.dispatch(Command::Usages);
        f.settle(|s| s.state().asking().is_none() && s.state().results().is_some_and(|r| r.hits().len() == 500));
        if i >= 10 {
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        f.s.dispatch(Command::CloseResults);
    }
    println!("READER_PERF {}", serde_json::json!({ "name": "reader/usages-fake-500", "unit": "ms", "samples": times }));
    println!("usages to 500 results listed (fake server): median {:.2} ms, p95 {:.2} ms over {}", percentile(&mut times, 0.5), percentile(&mut times, 0.95), times.len());
}

#[test]
fn restart_uses_edited_settings_and_silent_servers_do_not_hold_a_symbol_search() {
    let now = std::sync::Arc::new(std::sync::Mutex::new(Instant::now()));
    let clock = now.clone();
    let mut f = fixture_with(&[("FAKE_LSP_SILENT_SYMBOLS", "1")], &[("src/main.rs", USES)], serde_json::json!({}), std::sync::Arc::new(move || *clock.lock().unwrap()));
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("src/main.rs");
    // A server that never answers: the search ends after its wait, saying so.
    f.s.dispatch(Command::Symbols("main".into()));
    assert!(f.s.state().asking().is_some());
    *now.lock().unwrap() += reader_core::session::SYMBOLS_WAIT;
    tick(&mut f);
    assert!(f.s.state().asking().is_none() && f.s.state().results().is_some_and(|r| r.done()));
    assert!(f.notice("Symbols: 1 language server(s) did not answer within 30 s"), "{:?}", f.s.state().notices());
    // The settings file edited by hand: Restart reads it (a missing executable, then the right one again).
    let file = f.root.parent().unwrap().join("config/providers.json");
    let good = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, r#"{"providers":{"rust-analyzer":{"executable":"/nonexistent/edited"}}}"#).unwrap();
    f.s.dispatch(Command::RestartServer);
    f.settle(|s| s.state().provider().is_some_and(|v| matches!(&v.status, Status::Missing(w) if w.contains("/nonexistent/edited"))));
    std::fs::write(&file, good).unwrap();
    f.s.dispatch(Command::RestartServer);
    f.settle(ready);
}

#[test]
fn file_symbols_are_a_semantic_outline_of_the_snapshot_sent() {
    let text = "struct Área;\nimpl Área {\n    fn new() {}\n    fn größe() {}\n}\nfn main() {}\n";
    let mut f = fixture(&[], &[("src/main.rs", text)]);
    f.open("src/main.rs");
    f.s.dispatch(Command::FileSymbols);
    f.settle(|s| s.state().results().is_some_and(|r| r.results_kind() == ResultsKind::FileSymbols));
    let r = f.s.state().results().unwrap();
    assert_eq!(r.kind(), "File symbols (rust-analyzer · proj)");
    let rows: Vec<(String, u16, usize)> = r.hits().iter().map(|h| (h.text.to_string(), h.decl.unwrap().1, h.line)).collect();
    assert_eq!(rows, [("Área".to_string(), 0, 0), ("Área".to_string(), 0, 1), ("new".to_string(), 1, 2), ("größe".to_string(), 1, 3), ("main".to_string(), 0, 5)]);
    // Opening one moves there, as an outline entry does.
    f.s.dispatch(Command::OpenResult(3));
    f.settle(|s| s.state().selection().is_some_and(|x| x.range().start == text.find("größe").unwrap()));
    assert_eq!(f.selected_text(), "größe");
}

#[test]
fn python_projects_nest_take_explicit_roots_and_say_an_unknown_version_is_untested() {
    let files = [("pyproject.toml", ""), ("svc/pyproject.toml", ""), ("svc/app.py", "def serve():\n    pass\n\nserve()\n")];
    let mut f = fixture_as("pyright", &[], &files, serde_json::json!({}), std::sync::Arc::new(Instant::now));
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("svc/app.py");
    let view = f.s.state().provider().unwrap();
    assert_eq!((view.name, view.root.as_deref()), ("Pyright", Some(f.root.join("svc").as_path())), "the nested project");
    assert!(view.summary().starts_with("Pyright (version unknown: untested): "), "{}", view.summary());
    f.definition("serve()\n", false);
    f.settle(|s| s.state().asking().is_none() && s.state().selection().is_some_and(|x| x.anchor != x.head));
    assert_eq!(f.selected_text(), "serve");
    // An explicit root: the outer project.
    f.s.dispatch(Command::SetWorkspaceRoot(Some(f.root.clone())));
    f.settle(|s| s.state().provider().is_some_and(|v| v.root.as_deref() == Some(s.state().project().unwrap().root()) && matches!(v.status, Status::Ready)));
    let roots: Vec<String> = messages(&f.log).iter().filter(|m| m["method"] == "initialize").map(|m| m["params"]["rootUri"].as_str().unwrap().to_owned()).collect();
    assert!(roots[0].ends_with("/proj/svc") && roots[1].ends_with("/proj"), "{roots:?}");
    let open = messages(&f.log).into_iter().find(|m| m["method"] == "textDocument/didOpen").unwrap();
    assert_eq!(open["params"]["textDocument"]["languageId"], "python");
}

#[test]
fn typescript_tsx_and_javascript_share_one_server_with_their_own_language_ids() {
    let files = [("package.json", "{}"), ("tsconfig.json", "{}"), ("src/a.ts", "export function make() {}\n"), ("src/b.tsx", "make();\n"), ("src/c.js", "make();\n"), ("src/d.jsx", "make();\n"), ("nested/tsconfig.json", "{}"), ("nested/e.ts", "make();\n")];
    let mut f = fixture_as("typescript-language-server", &[], &files, serde_json::json!({}), std::sync::Arc::new(Instant::now));
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    for name in ["src/a.ts", "src/b.tsx", "src/c.js", "src/d.jsx", "nested/e.ts"] {
        f.s.dispatch(Command::OpenRetained(f.root.join(name)));
        f.open(name);
    }
    assert_eq!(f.s.state().servers().len(), 1, "one server for the folder's TS, TSX and JS, nested configurations included");
    let ids: Vec<(String, String)> = messages(&f.log)
        .iter()
        .filter(|m| m["method"] == "textDocument/didOpen")
        .map(|m| (m["params"]["textDocument"]["uri"].as_str().unwrap().rsplit('/').next().unwrap().to_owned(), m["params"]["textDocument"]["languageId"].as_str().unwrap().to_owned()))
        .collect();
    for want in [("a.ts", "typescript"), ("b.tsx", "typescriptreact"), ("c.js", "javascript"), ("d.jsx", "javascriptreact"), ("e.ts", "typescript")] {
        assert!(ids.contains(&(want.0.into(), want.1.into())), "{want:?} in {ids:?}");
    }
    f.s.dispatch(Command::Open(f.root.join("src/b.tsx")));
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("b.tsx")));
    f.definition("make", false);
    f.settle(|s| s.state().asking().is_none() && s.state().current_source().is_some_and(|d| d.source().ends_with("a.ts")));
    assert_eq!(f.selected_text(), "make");
}

#[test]
fn a_definition_during_a_no_status_servers_warm_up_says_it_may_stop_short() {
    let now = std::sync::Arc::new(std::sync::Mutex::new(Instant::now()));
    let clock = now.clone();
    let files = [("pyproject.toml", ""), ("a.py", "def make():\n    pass\n\nmake()\n")];
    let mut f = fixture_as("pyright", &[], &files, serde_json::json!({}), std::sync::Arc::new(move || *clock.lock().unwrap()));
    f.open("a.py");
    f.definition("make()\n", false);
    f.settle(|s| s.state().asking().is_none() && s.state().selection().is_some_and(|x| x.anchor != x.head));
    assert!(f.notice("Pyright may still be loading its workspace; if this is not the declaration, ask again shortly"), "{:?}", f.s.state().notices());
    *now.lock().unwrap() += reader_core::semantics::UNREPORTED_WARMUP;
    f.definition("make()\n", false);
    f.settle(|s| s.state().asking().is_none() && s.state().selection().is_some_and(|x| x.anchor != x.head));
    assert!(!f.notice("may still be loading"), "warm: no notice {:?}", f.s.state().notices());
}

#[test]
fn a_selected_name_is_asked_about_at_its_start() {
    let mut f = fixture(&[], &[("src/main.rs", "fn main() { helper(); }\nfn helper() {}\n")]);
    f.open("src/main.rs");
    let at = "fn main() { ".len();
    f.s.dispatch(Command::Select(Selection { anchor: at + "helper".len(), head: at }));
    f.s.dispatch(Command::Usages);
    f.s.dispatch(Command::Select(Selection { anchor: at, head: at + "helper".len() }));
    f.s.dispatch(Command::Usages);
    f.asked();
    let positions: Vec<u64> = messages(&f.log).iter().filter(|m| m["method"] == "textDocument/references").map(|m| m["params"]["position"]["character"].as_u64().unwrap()).collect();
    assert_eq!(positions, [at as u64, at as u64], "either direction: the name's start, never past its end");
}

#[test]
fn a_virtual_definition_target_is_retrieved_into_a_read_only_tab_of_its_session() {
    let files = [("pom.xml", ""), ("src/A.java", "class A {\n    void m() { virt(); }\n}\n")];
    let mut f = fixture_as("jdtls", &[("FAKE_LSP_JDT", "1")], &files, serde_json::json!({}), std::sync::Arc::new(Instant::now));
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("src/A.java");
    f.settle(|s| s.state().provider().is_some_and(|v| v.status == Status::Ready));
    // Its own workspace-data folder in the reader's cache, per project and launch.
    let launch = messages(&f.log).into_iter().find(|m| m.get("args").is_some()).unwrap();
    let args: Vec<String> = serde_json::from_value(launch["args"].clone()).unwrap();
    // Eclipse metadata kept out of the project, then its own workspace-data folder.
    assert_eq!(args[0], "--jvm-arg=-Djava.import.generatesMetadataFilesAtProjectRoot=false");
    assert_eq!(args[1], "-data");
    assert!(args[2].contains("/cache/jdtls/proj-"), "{args:?}");
    f.definition("virt()", false);
    f.settle(|s| s.state().asking().is_none() && s.state().active().is_some_and(|t| t.retrieved().is_some()));
    let tab = f.s.state().active().unwrap();
    assert_eq!(tab.title(), "Generated.java (Eclipse JDT LS)");
    assert_eq!(f.selected_text(), "virt");
    assert_eq!(f.s.state().current_source().unwrap().language(), Some(lenz_content::highlight::Language::Java));
    assert!(f.notice("Read-only source retrieved from Eclipse JDT LS (fake-virtual:///pkg/Generated.java?=jar%2Fx)"), "{:?}", f.s.state().notices());
    assert_eq!(f.s.disabled(CommandId::Definition), Some("Not available in source a language server returned"));
    // Never read from disk: Reload is refused, and nothing says it is unavailable.
    assert_eq!(f.s.disabled(CommandId::Reload), Some("Source a language server returned cannot be reloaded; ask for the definition again"));
    f.s.dispatch(Command::Reload);
    std::thread::sleep(Duration::from_millis(100));
    while let Ok(c) = recv(&f.s.completions()) {
        f.s.dispatch(c);
    }
    assert!(!f.s.state().notices().iter().any(|n| n.starts_with("Unavailable")), "{:?}", f.s.state().notices());
    let asked = messages(&f.log).into_iter().find(|m| m["method"] == "java/classFileContents").unwrap();
    assert_eq!(asked["params"]["uri"], "fake-virtual:///pkg/Generated.java?=jar%2Fx");
    assert!(!messages(&f.log).iter().any(|m| m["method"] == "textDocument/didOpen" && m["params"]["textDocument"]["uri"].as_str().unwrap().starts_with("fake-virtual")), "never sent back to a server");
    // History: back to the file, forward to the retrieved tab (still held), and nothing on disk.
    f.s.dispatch(Command::Back);
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("A.java")));
    f.s.dispatch(Command::Forward);
    f.settle(|s| s.state().active().is_some_and(|t| t.retrieved().is_some()));
    let uri = f.s.state().active().unwrap().source().to_string_lossy().into_owned();
    assert!(f.s.state().tree_rows().any(|r| r.path.as_ref() == uri));
    f.s.dispatch(Command::Close);
    f.s.dispatch(Command::ActivateTree { path: uri.clone().into(), retain: false });
    f.settle(|s| s.state().active().is_some_and(|t| t.retrieved().is_some()));
    assert_eq!(messages(&f.log).iter().filter(|m| m["method"] == "java/classFileContents").count(), 2);
    // Its server restarted: still shown, said to be from an ended session.
    f.s.dispatch(Command::Back);
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("A.java")));
    f.s.dispatch(Command::RestartServer);
    f.settle(|s| s.state().provider().is_some_and(|v| v.status == Status::Ready));
    f.s.dispatch(Command::Forward);
    f.settle(|s| s.state().active().is_some_and(|t| t.retrieved().is_some()));
    assert!(f.notice("its server session has ended"), "{:?}", f.s.state().notices());
    // Closed, it is not held: Back to it says so instead of reading a path that does not exist.
    f.s.dispatch(Command::Close);
    f.s.dispatch(Command::Open(f.root.join("src/A.java")));
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("A.java")));
    f.s.dispatch(Command::Back);
    assert!(f.notice("was retrieved from a language server and is no longer held"), "{:?}", f.s.state().notices());
    f.s.dispatch(Command::ActivateTree { path: uri.into(), retain: false });
    assert!(f.notice("Library source unavailable"));
}

#[test]
fn background_lanes_never_cancel_a_pending_definition() {
    let mut f = fixture(&[], &[("src/lib.rs", "pub fn slow_target() {}\n"), ("src/main.rs", "fn main() { slow_target(); }\n")]);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("src/main.rs");
    f.definition("slow_target()", false); // the fake answers slow_* after 400 ms
    assert!(f.s.state().asking().is_some());
    // A highlight (a background lane) asked meanwhile.
    f.s.dispatch(Command::HighlightAt);
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("lib.rs"))); // the definition still arrived
    let log = messages(&f.log);
    assert!(!log.iter().any(|m| m["method"] == "$/cancelRequest"), "nothing was cancelled");
}

#[test]
fn the_context_menu_lists_navigation_then_copy_and_copies_paths() {
    let mut f = fixture(&[], &[("src/main.rs", "fn main() {\n    helper();\n}\nfn helper() {}\n")]);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("src/main.rs");
    f.caret("helper();");
    use reader_core::session::MenuEntry::{Command as C, Separator};
    assert_eq!(f.s.context_menu(), vec![
        C(CommandId::Definition), C(CommandId::PeekDefinition), C(CommandId::TypeDefinition),
        C(CommandId::Implementation), C(CommandId::Usages), C(CommandId::CallHierarchy),
        C(CommandId::TypeHierarchy), C(CommandId::Documentation), Separator,
        C(CommandId::Collapse), C(CommandId::Expand), C(CommandId::CollapseAll), C(CommandId::ExpandAll), Separator,
        C(CommandId::Copy), C(CommandId::Cut), C(CommandId::Paste), C(CommandId::Delete), C(CommandId::ReadOnlyMode),
        C(CommandId::CopyPath), C(CommandId::CopyPathLine), C(CommandId::LocateFile), Separator,
        C(CommandId::AnnotateBlame), Separator, C(CommandId::OpenInEditor), C(CommandId::OpenWithDefaultApp),
        C(CommandId::ResolveExternalChange), C(CommandId::CompareWith), C(CommandId::CompareSaved), C(CommandId::CloseSavedComparison), Separator,
        C(CommandId::SplitRight), C(CommandId::SplitDown),
    ]);
    for id in [CommandId::Cut, CommandId::Paste, CommandId::Delete] {
        assert_eq!(f.s.disabled(id), Some("Text input is gated on this unvalidated platform/backend"));
    }
    assert_eq!(f.s.disabled(CommandId::ReadOnlyMode), None);
    assert_eq!(f.s.disabled(CommandId::CompareSaved), None);
    assert_eq!(f.s.disabled(CommandId::CloseSavedComparison), Some("No saved comparison is open"));
    assert_eq!(f.s.dispatch(Command::CopyPath { line: false }).copy.as_deref(), Some("src/main.rs"));
    assert_eq!(f.s.dispatch(Command::CopyPath { line: true }).copy.as_deref(), Some("src/main.rs:2"));
    assert_eq!(CommandId::CopyPathLine.label(), "Copy Path:Line");
    assert!(CommandId::ALL.contains(&CommandId::CopyPath));
    assert_eq!(f.s.disabled(CommandId::CopyPath), None);
}

#[test]
fn implementations_and_type_definitions_jump_or_list() {
    let lib = "pub trait Shape {}\npub struct Circle;\npub struct Square;\nimpl Shape for Circle {}\nimpl Shape for Square {}\n";
    let main = "fn main() {\n    let c: Circle = Circle;\n    let s: &dyn Shape = &c;\n}\n";
    let mut f = fixture(&[], &[("src/lib.rs", lib), ("src/main.rs", main)]);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("src/main.rs");
    // Two implementations: listed, the main area stays.
    f.caret("Shape =");
    f.s.dispatch(Command::Implementation);
    f.settle(|s| s.state().results().is_some_and(|r| r.results_kind() == ResultsKind::Implementations && r.hits().len() == 2 && r.preview().is_some()));
    assert!(f.s.state().current_source().unwrap().source().ends_with("main.rs"));
    assert_eq!(f.s.state().results().unwrap().kind(), "Implementations (rust-analyzer · proj)");
    // One type definition: jumps there with the name selected.
    f.caret("c: Circle");
    f.s.dispatch(Command::TypeDefinition);
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("lib.rs")));
    assert_eq!(f.selected_text(), "Circle");
    // None: said so.
    f.s.dispatch(Command::Back);
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("main.rs")));
    f.caret("main");
    f.s.dispatch(Command::Implementation);
    f.asked();
    assert!(f.notice("Implementations: rust-analyzer found no implementations"), "{:?}", f.s.state().notices());
    // Unsupported: disabled with a reason.
    let mut g = fixture(&[("FAKE_LSP_NO_NAVIGATION", "1")], &[("src/main.rs", main)]);
    g.open("src/main.rs");
    g.caret("Circle");
    assert_eq!(g.s.disabled(CommandId::Implementation), Some("The language server does not offer implementations"));
    assert_eq!(g.s.disabled(CommandId::TypeDefinition), Some("The language server does not offer type definitions"));
}

const CALLS: &str = "fn main() {\n    helper();\n    other();\n}\nfn other() {\n    helper();\n    other();\n}\nfn helper() {}\n";

#[test]
fn call_hierarchy_loads_levels_lazily_toggles_direction_and_opens_call_sites() {
    use reader_core::{search::Fold, semantics::hierarchy::Direction};
    let mut f = fixture(&[], &[("src/main.rs", CALLS)]);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("src/main.rs");
    f.caret("helper() {}");
    f.s.dispatch(Command::CallHierarchy(Direction::Callers));
    f.settle(|s| s.state().results().is_some_and(|r| r.results_kind() == ResultsKind::Hierarchy && r.hits().len() == 3));
    let r = f.s.state().results().unwrap();
    assert_eq!(r.kind(), "Callers of helper (rust-analyzer · proj)");
    assert_eq!(r.hits().iter().map(|h| h.line).collect::<Vec<_>>(), [8, 1, 5], "the root at its name, then each caller at its call");
    assert_eq!(r.call(0).unwrap().fold, Fold::Open);
    assert_eq!(r.call(1).unwrap().fold, Fold::Closed);
    assert_eq!((r.call(2).unwrap().depth, r.call(2).unwrap().name.as_str()), (1, "other"));
    // Unfold `other`: its callers are `main` and itself (marked, not followed).
    f.s.dispatch(Command::ToggleCall(2));
    assert_eq!(f.s.state().results().unwrap().call(2).unwrap().fold, Fold::Loading);
    f.settle(|s| s.state().results().is_some_and(|r| r.hits().len() == 5));
    let r = f.s.state().results().unwrap();
    assert_eq!(r.call(4).unwrap().fold, Fold::Recursive);
    assert_eq!(r.call(3).unwrap().depth, 2);
    // Folding it again hides its level; unfolding shows it without asking.
    f.s.dispatch(Command::ToggleCall(2));
    assert_eq!(f.s.state().results().unwrap().hits().len(), 3);
    f.s.dispatch(Command::ToggleCall(2));
    assert_eq!(f.s.state().results().unwrap().hits().len(), 5);
    // Opening a row goes to its call site.
    f.s.dispatch(Command::OpenResult(1));
    f.settle(|s| s.state().selection().is_some_and(|sel| s.state().current_source().is_some_and(|d| d.line_of(sel.head) == 1)));
    assert_eq!(f.selected_text(), "helper");
    // The other way: what main calls.
    f.caret("main");
    f.s.dispatch(Command::CallHierarchy(Direction::Callees));
    f.settle(|s| s.state().results().is_some_and(|r| r.kind().starts_with("Callees of main") && r.hits().len() == 3));
    assert_eq!(f.s.state().results().unwrap().hits().iter().map(|h| h.line).collect::<Vec<_>>(), [0, 1, 2]);
    // Switching direction keeps the root.
    f.s.dispatch(Command::CallHierarchy(Direction::Callers));
    f.settle(|s| s.state().results().is_some_and(|r| r.kind().starts_with("Callers of main")));
    assert!(!messages(&f.log).iter().any(|m| m["method"] == "$/cancelRequest"), "hierarchy levels never cancel each other or a definition");
}

#[test]
fn a_failed_level_says_why_and_unfolding_again_retries() {
    use reader_core::{search::Fold, semantics::hierarchy::Direction};
    let mut f = fixture(&[], &[("src/main.rs", "fn main() { error_x(); }\nfn error_x() {}\n")]);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("src/main.rs");
    f.caret("error_x() {}");
    f.s.dispatch(Command::CallHierarchy(Direction::Callers));
    f.settle(|s| s.state().results().is_some_and(|r| matches!(r.call(0).map(|c| c.fold), Some(Fold::Failed(_)))));
    f.s.dispatch(Command::ToggleCall(0));
    assert_eq!(f.s.state().results().unwrap().call(0).unwrap().fold, Fold::Loading);
    f.settle(|s| s.state().results().is_some_and(|r| matches!(r.call(0).map(|c| c.fold), Some(Fold::Failed(_)))));
    // Nothing callable at the caret.
    f.caret("{ error");
    f.s.dispatch(Command::CallHierarchy(Direction::Callers));
    f.asked();
    assert!(f.notice("Call hierarchy: No call hierarchy here"), "{:?}", f.s.state().notices());
    // Unsupported: disabled with a reason.
    let mut g = fixture(&[("FAKE_LSP_NO_NAVIGATION", "1")], &[("src/main.rs", CALLS)]);
    g.open("src/main.rs");
    g.caret("helper() {}");
    assert_eq!(g.s.disabled(CommandId::CallHierarchy), Some("The language server does not offer call hierarchy"));
}

#[test]
fn occurrences_of_the_name_at_the_caret_are_marked_and_stale_ones_dropped() {
    let mut f = fixture(&[], &[("src/main.rs", "fn main() {\n    let total = 1;\n    let x = total + total;\n}\n")]);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("src/main.rs");
    let at = f.caret("total + ");
    f.s.dispatch(Command::HighlightAt);
    f.settle(|s| s.state().occurrences().is_some());
    let o = f.s.state().occurrences().unwrap();
    assert_eq!(o.ranges.len(), 3);
    assert_eq!(o.ranges.iter().filter(|(_, write)| *write).count(), 1, "the assignment is a write");
    assert!(o.ranges.iter().any(|(r, _)| r.start == at));
    // Within another occurrence the marks stay; off every one they go at once.
    f.caret("total = 1");
    assert!(f.s.state().occurrences().is_some());
    f.caret("main");
    assert!(f.s.state().occurrences().is_none());
    // An answer for an earlier caret position is dropped.
    f.caret("total + ");
    f.s.dispatch(Command::HighlightAt);
    f.caret("x =");
    f.s.dispatch(Command::HighlightAt);
    f.settle(|s| s.state().occurrences().is_some_and(|o| o.ranges.len() == 1));
    // Keywords ask nothing (once the syntax is known).
    f.settle(|s| active_view(s).is_some_and(|t| t.highlights().is_some()));
    let asked = |f: &Fixture| messages(&f.log).iter().filter(|m| m["method"] == "textDocument/documentHighlight").count();
    let before = asked(&f);
    f.caret("let total");
    f.s.dispatch(Command::HighlightAt);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(asked(&f), before);
}

#[test]
fn quick_documentation_at_the_caret_or_pointer_replaces_older_requests() {
    use reader_core::session::DocTrigger;
    let mut f = fixture(&[], &[("src/main.rs", "fn main() { helper(); }\nfn helper() {}\n")]);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("src/main.rs");
    f.caret("helper();");
    f.s.dispatch(Command::Documentation);
    f.settle(|s| s.state().documentation().is_some());
    let d = f.s.state().documentation().unwrap();
    assert_eq!(d.trigger, DocTrigger::Caret);
    assert_eq!(&current_text(f.s.state())[d.at.clone()], "helper");
    assert!(d.preview.doc.source.contains("Docs for **helper**"));
    f.s.dispatch(Command::CloseDocumentation);
    assert!(f.s.state().documentation().is_none());
    // At the pointer: the caret stays; a keyword asks nothing; nothing known shows nothing and says nothing.
    f.settle(|s| active_view(s).is_some_and(|t| t.highlights().is_some()));
    let tab = f.s.state().active().unwrap().id();
    let caret = f.s.state().selection().unwrap();
    f.s.dispatch(Command::DocumentationAt { tab, offset: 1 }); // in `fn`
    f.s.dispatch(Command::DocumentationAt { tab, offset: "fn ".len() + 1 }); // `main`
    f.settle(|s| s.state().documentation().is_some());
    assert_eq!(f.s.state().documentation().unwrap().trigger, DocTrigger::Pointer);
    assert_eq!(f.s.state().selection().unwrap(), caret);
    let hovers = messages(&f.log).iter().filter(|m| m["method"] == "textDocument/hover").count();
    assert_eq!(hovers, 2, "the keyword was not asked about");
    // Switching tabs closes it.
    f.s.dispatch(Command::Open(f.root.join("Cargo.toml")));
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("Cargo.toml")));
    assert!(f.s.state().documentation().is_none());
    f.s.dispatch(Command::Back);
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("main.rs")));
    // At the caret with nothing to say: said so.
    f.caret("{ helper");
    f.s.dispatch(Command::Documentation);
    f.settle(|s| s.state().notices().iter().any(|n| n.contains("No documentation here")));
    // The setting is remembered.
    assert!(f.s.state().documentation_on_hover());
    f.s.dispatch(Command::SetDocumentationOnHover(false));
    assert!(!f.s.state().documentation_on_hover());
    let settings = f._dir.path().join("state/settings.json");
    f.settle_until(|| std::fs::read_to_string(&settings).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()).is_some_and(|v| v["documentation_on_hover"] == false));
}

#[test]
fn a_level_pending_when_its_server_stops_fails_and_says_to_ask_again() {
    use reader_core::{search::Fold, semantics::hierarchy::Direction};
    let mut f = fixture(&[], &[("src/main.rs", "fn main() { crash_x(); }\nfn crash_x() {}\n")]);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("src/main.rs");
    f.caret("crash_x() {}");
    f.s.dispatch(Command::CallHierarchy(Direction::Callers));
    f.settle(|s| s.state().results().is_some_and(|r| matches!(r.call(0).map(|c| c.fold), Some(Fold::Failed(_)))));
    // Unfolding again cannot reach the stopped server: it fails at once, saying what to do.
    f.s.dispatch(Command::ToggleCall(0));
    let fold = f.s.state().results().unwrap().call(0).unwrap().fold;
    assert!(matches!(&fold, Fold::Failed(why) if why.contains("Call Hierarchy again")), "{fold:?}");
}

#[test]
fn documentation_closes_when_its_tab_is_left_and_does_not_come_back() {
    let mut f = fixture(&[], &[("src/main.rs", "fn main() { helper(); }\nfn helper() {}\n")]);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("src/main.rs");
    f.caret("helper();");
    f.s.dispatch(Command::Documentation);
    f.settle(|s| s.state().documentation().is_some());
    f.s.dispatch(Command::Open(f.root.join("Cargo.toml")));
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("Cargo.toml")));
    f.s.dispatch(Command::Back);
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("main.rs")));
    assert!(f.s.state().documentation().is_none(), "closed, not hidden");
}

fn sent(log: &Path, method: &str) -> Vec<Value> {
    messages(log).into_iter().filter(|m| m["method"] == method).collect()
}

const HINTED: &str = "fn main() {\n    let mut v = add(1);\n}\nfn old_add(a: i32) -> i32 { a }\n";

#[test]
fn decorations_are_asked_after_the_document_opens_with_the_client_capabilities() {
    let mut f = fixture(&[], &[("src/main.rs", HINTED)]);
    f.open("src/main.rs");
    let log = f.log.clone();
    f.settle_until(|| !sent(&log, "textDocument/inlayHint").is_empty() && !sent(&log, "textDocument/semanticTokens/full").is_empty());
    let init = &sent(&f.log, "initialize")[0]["params"]["capabilities"];
    assert_eq!(init["textDocument"]["semanticTokens"]["formats"], serde_json::json!(["relative"]));
    assert!(init["textDocument"]["semanticTokens"]["tokenModifiers"].as_array().unwrap().iter().any(|m| m == "mutable"));
    assert_eq!(init["textDocument"]["semanticTokens"]["requests"]["full"], true);
    assert!(init["textDocument"]["inlayHint"].is_object());
    assert_eq!((init["workspace"]["inlayHint"]["refreshSupport"].clone(), init["workspace"]["semanticTokens"]["refreshSupport"].clone()), (Value::Bool(true), Value::Bool(true)));
    let all = messages(&f.log);
    let index = |method: &str| all.iter().position(|m| m["method"] == method).unwrap();
    assert!(index("textDocument/didOpen") < index("textDocument/inlayHint"), "the server has the text first");
    let hint = &sent(&f.log, "textDocument/inlayHint")[0]["params"];
    assert!(hint["textDocument"]["uri"].as_str().unwrap().ends_with("src/main.rs"));
    assert_eq!(hint["range"]["end"]["line"], 4, "the whole document");
}

#[test]
fn a_new_version_cancels_the_older_request_and_asks_again() {
    let mut f = fixture(&[("FAKE_LSP_SLOW_DECOR", "1")], &[("src/main.rs", HINTED)]);
    f.open("src/main.rs");
    let log = f.log.clone();
    f.settle_until(|| !sent(&log, "textDocument/inlayHint").is_empty());
    write(&f.root, "src/main.rs", &HINTED.replace("add(1)", "add(2)"));
    f.s.dispatch(Command::Reload);
    f.settle_until(|| sent(&log, "textDocument/inlayHint").len() >= 2);
    // The fake answers one request at a time (300 ms each), so the older one is still pending when the text changes.
    f.settle_until(|| !sent(&log, "$/cancelRequest").is_empty());
    let hint_ids: Vec<Value> = sent(&f.log, "textDocument/inlayHint").iter().map(|m| m["id"].clone()).collect();
    assert!(sent(&f.log, "$/cancelRequest").iter().any(|m| hint_ids.contains(&m["params"]["id"])), "an older hint request is cancelled");
    let all = messages(&f.log);
    let change = all.iter().position(|m| m["method"] == "textDocument/didChange").unwrap();
    let again = all.iter().rposition(|m| m["method"] == "textDocument/inlayHint").unwrap();
    assert!(change < again, "asked again about the new text");
}

#[test]
fn a_refresh_request_is_answered_and_asks_again() {
    let mut f = fixture(&[("FAKE_LSP_REFRESH", "1")], &[("src/main.rs", HINTED)]);
    f.open("src/main.rs");
    let log = f.log.clone();
    let answered = |all: &[Value]| all.iter().position(|m| m["id"] == "rf-1" && m.get("result").is_some_and(Value::is_null) && m.get("error").is_none());
    f.settle_until(|| {
        let all = messages(&log);
        answered(&all).is_some_and(|a| all[a..].iter().any(|m| m["method"] == "textDocument/inlayHint"))
    });
}

#[test]
fn a_request_refused_while_indexing_is_asked_again_once_ready() {
    let mut f = fixture(&[("FAKE_LSP_INDEXING", "1"), ("FAKE_LSP_BUSY_DECOR", "1")], &[("src/main.rs", HINTED)]);
    f.open("src/main.rs");
    let log = f.log.clone();
    f.settle_until(|| sent(&log, "textDocument/inlayHint").len() >= 2);
}

#[test]
fn hints_answered_empty_while_indexing_are_asked_again_once_ready() {
    let mut f = fixture(&[("FAKE_LSP_INDEXING", "1"), ("FAKE_LSP_EMPTY_DECOR", "1")], &[("src/main.rs", HINTED)]);
    f.open("src/main.rs");
    f.settle(|s| active_view(s).and_then(|t| t.inlay_hints().cloned()).is_some_and(|h| !h.is_empty()));
}

#[test]
fn an_answer_from_before_a_refresh_arriving_last_is_not_shown() {
    let mut f = fixture(&[("FAKE_LSP_STALE_DECOR", "1")], &[("src/main.rs", HINTED)]);
    f.open("src/main.rs");
    let log = f.log.clone();
    f.settle_until(|| sent(&log, "textDocument/inlayHint").len() >= 2);
    // Every state on the way: never the older answer once the newer one was shown.
    let newer = std::cell::Cell::new(false);
    let until = Instant::now() + Duration::from_millis(900);
    f.settle_checking(|_| Instant::now() > until, |s| {
        let text = active_view(s).map(|t| projected(&t, 1)).unwrap_or_default();
        newer.set(newer.get() || text.contains(": i32"));
        assert!(!(newer.get() && text.contains(": old")), "{text}");
    });
    assert_eq!(projected(&active_view(&f.s).unwrap(), 1), "    let mut v: i32 = add(a: 1);");
}

#[test]
fn a_server_without_decorations_is_never_asked() {
    let mut f = fixture(&[("FAKE_LSP_NO_DECOR", "1")], &[("src/main.rs", HINTED)]);
    f.open("src/main.rs");
    let until = Instant::now() + Duration::from_millis(500);
    f.settle_until(|| Instant::now() > until);
    assert!(sent(&f.log, "textDocument/inlayHint").is_empty() && sent(&f.log, "textDocument/semanticTokens/full").is_empty());
}

fn decorated(s: &Session) -> bool {
    active_view(s).is_some_and(|t| t.inlay_hints().is_some_and(|h| !h.is_empty()) && t.tokens().is_some_and(|k| !k.is_empty()))
}

#[test]
fn hints_and_tokens_show_on_the_text_they_were_asked_about() {
    let mut f = fixture(&[], &[("src/main.rs", HINTED)]);
    f.open("src/main.rs");
    f.settle(decorated);
    let t = active_view(&f.s).unwrap();
    assert_eq!(projected(&t, 1), "    let mut v: i32 = add(a: 1);");
    let v = "    let mut ".len();
    assert_eq!(t.style_at(1, v), (Some(Capture::Variable), reader_core::decor::MUTABLE));
    assert_eq!(t.style_at(1, v + 1).1, 0);
    assert!((3..10).all(|byte| t.style_at(3, byte) == (Some(Capture::Function), reader_core::decor::DEPRECATED)));
    assert_eq!(t.style_at(3, 10).1, 0);
    assert_eq!(std::fs::read_to_string(f.root.join("src/main.rs")).unwrap(), HINTED, "the file is unchanged");
}

#[test]
fn a_result_for_an_older_version_is_never_shown() {
    let mut f = fixture(&[("FAKE_LSP_SLOW_DECOR", "1")], &[("src/main.rs", HINTED)]);
    f.open("src/main.rs");
    let log = f.log.clone();
    f.settle_until(|| !sent(&log, "textDocument/inlayHint").is_empty());
    let newer = HINTED.replace("let mut v", "let mut value");
    write(&f.root, "src/main.rs", &newer);
    f.s.dispatch(Command::Reload);
    // Every state on the way shows hints only where the newer text has its name.
    f.settle_checking(decorated, |s| {
        if let Some(t) = active_view(s) && let Some(h) = t.inlay_hints() {
            assert!(h.line(1).iter().all(|h| h.kind != reader_core::decor::HintKind::Type || h.byte as usize == "    let mut value".len()), "{:?}", h.line(1));
        }
    });
    assert_eq!(projected(&active_view(&f.s).unwrap(), 1), "    let mut value: i32 = add(a: 1);");
}

#[test]
fn turning_a_kind_off_drops_it_and_asks_again_for_the_rest() {
    let mut f = fixture(&[], &[("src/main.rs", HINTED)]);
    f.open("src/main.rs");
    f.settle(decorated);
    f.s.dispatch(Command::SetInlayTypes(false));
    assert!(active_view(&f.s).unwrap().inlay_hints().is_none(), "dropped at once");
    f.settle(|s| active_view(s).unwrap().inlay_hints().is_some());
    assert_eq!(projected(&active_view(&f.s).unwrap(), 1), "    let mut v = add(a: 1);");
    let asked = sent(&f.log, "textDocument/semanticTokens/full").len();
    f.s.dispatch(Command::SetSemanticHighlighting(false));
    assert!(active_view(&f.s).unwrap().tokens().is_none(), "syntax colours only");
    f.s.dispatch(Command::SetSemanticHighlighting(true));
    f.settle(|s| active_view(s).unwrap().tokens().is_some());
    assert!(sent(&f.log, "textDocument/semanticTokens/full").len() > asked, "asked again");
}

#[test]
fn turning_hints_on_says_when_the_server_has_none() {
    let mut f = fixture(&[("FAKE_LSP_NO_DECOR", "1")], &[("src/main.rs", HINTED)]);
    f.open("src/main.rs");
    f.s.dispatch(Command::SetInlayTypes(false));
    f.s.dispatch(Command::SetInlayTypes(true));
    f.s.dispatch(Command::SetSemanticHighlighting(false));
    f.s.dispatch(Command::SetSemanticHighlighting(true));
    let notices = f.s.state().notices();
    assert!(notices.iter().any(|n| n.starts_with("No inlay hints from ")), "{notices:?}");
    assert!(notices.iter().any(|n| n.starts_with("No semantic tokens from ")), "{notices:?}");
}

#[test]
fn leaving_a_file_releases_its_decorations() {
    let mut f = fixture(&[], &[("src/main.rs", HINTED), ("src/lib.rs", "pub fn x() { let y = 1; }\n")]);
    f.open("src/main.rs");
    f.settle(decorated);
    // Kept as a tab (not replaced as a preview); activating another releases its content.
    let id = f.s.state().active().unwrap().id();
    f.s.dispatch(Command::Pin(id));
    f.open("src/lib.rs");
    let main = f.root.join("src/main.rs");
    let t = f.s.state().tabs().iter().find(|t| t.source() == main).unwrap();
    assert!(!t.loaded() && f.s.state().view(t.id()).is_none());
}

#[test]
fn both_panes_share_one_request_per_version() {
    let mut f = fixture(&[], &[("src/main.rs", HINTED)]);
    f.open("src/main.rs");
    f.settle(decorated);
    let before = sent(&f.log, "textDocument/inlayHint").len();
    f.s.dispatch(Command::Split(reader_core::session::Split::Columns));
    f.settle(|s| s.state().shown(1).and_then(|t| s.state().view(t.id())).is_some_and(|t| t.inlay_hints().is_some() && t.tokens().is_some()));
    let until = Instant::now() + Duration::from_millis(200);
    f.settle_until(|| Instant::now() > until);
    assert_eq!(sent(&f.log, "textDocument/inlayHint").len(), before, "the other pane shares the version's hints");
}

#[test]
fn a_restarted_server_is_asked_again() {
    let mut f = fixture(&[], &[("src/main.rs", HINTED)]);
    f.open("src/main.rs");
    f.settle(decorated);
    f.s.dispatch(Command::RestartServer);
    let log = f.log.clone();
    f.settle_until(|| sent(&log, "initialize").len() >= 2 && sent(&log, "textDocument/inlayHint").len() >= 2);
    f.settle(decorated);
}

#[test]
fn hints_arriving_keep_a_wrapped_view_valid() {
    let long = format!("fn main() {{\n    let v = add(1); {}\n}}\n", "x".repeat(200));
    let mut f = fixture(&[], &[("src/main.rs", &long)]);
    f.s.dispatch(Command::SetSoftWrap(true));
    f.s.dispatch(Command::SetTextSize { pane: 0, cols: 30, rows: 5 });
    f.open("src/main.rs");
    f.s.dispatch(Command::ScrollRows(4)); // into line 1's later rows
    f.settle(decorated);
    let s = f.s.state();
    let t = s.active().unwrap();
    let layout = s.view(t.id()).unwrap().layout_rows(5, s.wrap_cols(t));
    assert!(layout.pending.is_none());
    let rows = layout.rows;
    assert_eq!(rows.len(), 5, "a full pane");
    assert_eq!(rows[0].line, t.top_line());
}

#[test]
#[ignore = "memory over churn; run in release, alone"]
fn decor_churn() {
    let files: Vec<(String, String)> = (0..10).map(|i| (format!("src/m{i}.rs"), format!("fn f{i}() {{\n{}}}\n", "    let mut v = add(1);\n".repeat(500)))).collect();
    let refs: Vec<(&str, &str)> = files.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let mut f = fixture(&[], &refs);
    let rss = || std::fs::read_to_string("/proc/self/statm").ok().and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok()).unwrap_or(0) * 4;
    for round in 0..100 {
        let (name, _) = &files[round % 10];
        f.open(name);
        f.settle(decorated);
        f.s.dispatch(Command::SetInlayTypes(round % 2 == 0));
        if round % 10 == 9 {
            println!("round {} rss {} KiB", round + 1, rss());
        }
    }
}

const SHAPES: &str = "pub trait Shape {}\npub trait Round: Shape {}\npub struct Circle;\npub struct Square;\nimpl Shape for Circle {}\nimpl Shape for Square {}\nimpl Round for Circle {}\n";

/// The type hierarchy's rows: (depth, name).
fn type_rows(s: &Session) -> Vec<(u16, String)> {
    let r = s.state().results().unwrap();
    (0..r.hits().len()).filter_map(|i| r.call(i)).map(|c| (c.depth, c.name)).collect()
}

fn hierarchy_settled(s: &Session) -> bool {
    s.state().results().is_some_and(|r| r.results_kind() == ResultsKind::Hierarchy && (0..r.hits().len()).filter_map(|i| r.call(i)).all(|c| c.fold != reader_core::search::Fold::Loading))
}

#[test]
fn the_server_type_hierarchy_loads_levels_and_switches_direction() {
    let mut f = fixture(&[("FAKE_LSP_TYPE_HIERARCHY", "1")], &[("src/lib.rs", SHAPES)]);
    f.open("src/lib.rs");
    f.caret("Shape");
    f.s.dispatch(Command::TypeHierarchy(Direction::Subtypes));
    f.settle(hierarchy_settled);
    assert!(f.s.state().results().unwrap().kind().starts_with("Subtypes of Shape ("), "{}", f.s.state().results().unwrap().kind());
    let mut rows = type_rows(&f.s);
    rows[1..].sort();
    assert_eq!(rows, vec![(0, "Shape".into()), (1, "Circle".into()), (1, "Round".into()), (1, "Square".into())]);
    f.s.dispatch(Command::HierarchyDirection(Direction::Supertypes));
    f.settle(hierarchy_settled);
    assert_eq!(type_rows(&f.s), vec![(0, "Shape".into())], "Shape has no supertypes");
    assert!(sent(&f.log, "typeHierarchy/subtypes").len() == 1 && sent(&f.log, "typeHierarchy/supertypes").len() == 1);
}

#[test]
fn a_server_level_shows_all_its_children_past_the_usages_limit() {
    // 250 subtypes answered by the server itself: the 200-usage cap is for levels built from usages only.
    let impls: String = (0..250).map(|i| format!("pub struct S{i};\nimpl Shape for S{i} {{}}\n")).collect();
    let mut f = fixture(&[("FAKE_LSP_TYPE_HIERARCHY", "1")], &[("src/lib.rs", &format!("pub trait Shape {{}}\n{impls}"))]);
    f.open("src/lib.rs");
    f.caret("Shape");
    f.s.dispatch(Command::TypeHierarchy(Direction::Subtypes));
    f.settle(hierarchy_settled);
    assert_eq!(type_rows(&f.s).len(), 251, "the root and every subtype");
}

#[test]
fn not_a_type_says_so() {
    let mut f = fixture(&[("FAKE_LSP_TYPE_HIERARCHY", "1")], &[("src/lib.rs", "fn helper() {}\n")]);
    f.open("src/lib.rs");
    f.caret("helper");
    f.s.dispatch(Command::TypeHierarchy(Direction::Subtypes));
    f.settle(|s| s.state().notices().iter().any(|n| n == "Type hierarchy: Not a type"));
}

#[test]
fn a_restarted_server_retries_a_type_level() {
    let mut f = fixture(&[("FAKE_LSP_TYPE_HIERARCHY", "1")], &[("src/lib.rs", SHAPES)]);
    f.open("src/lib.rs");
    f.caret("Circle");
    f.s.dispatch(Command::TypeHierarchy(Direction::Supertypes));
    f.settle(hierarchy_settled);
    f.s.dispatch(Command::RestartServer);
    f.settle(|s| s.state().provider().is_some_and(|v| v.status == Status::Ready));
    // The tree's items belong to the old session: unfolding a child says why, and running it again works.
    f.s.dispatch(Command::ToggleCall(1));
    f.settle(hierarchy_settled);
    let r = f.s.state().results().unwrap();
    assert!(matches!(r.call(1).unwrap().fold, reader_core::search::Fold::Failed(_)), "{:?}", r.call(1).unwrap().fold);
    f.s.dispatch(Command::TypeHierarchy(Direction::Supertypes));
    f.settle(hierarchy_settled);
    let mut rows = type_rows(&f.s);
    rows[1..].sort();
    assert_eq!(rows, vec![(0, "Circle".into()), (1, "Round".into()), (1, "Shape".into())]);
}

#[test]
fn the_syntax_type_hierarchy_finds_impls_supertraits_and_traits_across_files() {
    let impls = "use crate::*;\nimpl Shape for Circle {}\nimpl<T> a::Shape for b::Wrap<T> {}\nimpl Round for Circle {}\n";
    let mut f = fixture(&[], &[("src/lib.rs", "pub trait Shape {}\npub trait Round: Shape {}\npub struct Circle;\npub struct Wrap<T>(T);\nmod impls;\n"), ("src/impls.rs", impls)]);
    f.open("src/lib.rs");
    f.caret("Shape");
    f.s.dispatch(Command::TypeHierarchy(Direction::Subtypes));
    f.settle(hierarchy_settled);
    let mut rows = type_rows(&f.s);
    rows[1..].sort();
    assert_eq!(rows, vec![(0, "Shape".into()), (1, "Circle".into()), (1, "Round".into()), (1, "Wrap".into())]);
    let r = f.s.state().results().unwrap();
    let labels: Vec<&str> = (1..r.hits().len()).filter_map(|i| r.call(i)).map(|c| c.label).collect();
    assert!(labels.contains(&"impl") && labels.contains(&"trait"), "{labels:?}");
    // Circle's supertypes: the traits it has an impl for, in a file the server never opened.
    f.caret("Circle");
    f.s.dispatch(Command::TypeHierarchy(Direction::Supertypes));
    f.settle(hierarchy_settled);
    let mut rows = type_rows(&f.s);
    rows[1..].sort();
    assert_eq!(rows, vec![(0, "Circle".into()), (1, "Round".into()), (1, "Shape".into())]);
    // Unfolding a child asks at its occurrence in impls.rs (not open in the server): Round's supertypes.
    let round = (0..f.s.state().results().unwrap().hits().len()).find(|&i| f.s.state().results().unwrap().call(i).is_some_and(|c| c.name == "Round")).unwrap();
    f.s.dispatch(Command::ToggleCall(round));
    f.settle(hierarchy_settled);
    assert!(type_rows(&f.s).contains(&(2, "Shape".into())), "{:?}", type_rows(&f.s));
    assert!(sent(&f.log, "typeHierarchy/subtypes").is_empty(), "the syntax path asks no type hierarchy");
}

#[test]
fn python_and_typescript_subtypes_by_syntax() {
    let mut f = fixture_as("pyright", &[], &[("pkg/base.py", "class Base:\n    pass\n"), ("pkg/kids.py", "from base import Base\nclass Kid(Base):\n    pass\nclass Other(Kid, metaclass=M):\n    pass\n")], serde_json::json!({}), std::sync::Arc::new(Instant::now));
    f.open("pkg/base.py");
    f.caret("Base");
    f.s.dispatch(Command::TypeHierarchy(Direction::Subtypes));
    f.settle(hierarchy_settled);
    assert_eq!(type_rows(&f.s), vec![(0, "Base".into()), (1, "Kid".into())]);
    let mut f = fixture_as("typescript-language-server", &[], &[("tsconfig.json", "{}"), ("src/a.ts", "export interface I {}\nexport class Base {}\n"), ("src/b.ts", "import { Base, I } from './a';\nclass Kid extends Base implements I {}\n")], serde_json::json!({}), std::sync::Arc::new(Instant::now));
    f.open("src/a.ts");
    f.caret("I");
    f.s.dispatch(Command::TypeHierarchy(Direction::Subtypes));
    f.settle(hierarchy_settled);
    assert_eq!(type_rows(&f.s), vec![(0, "I".into()), (1, "Kid".into())]);
}

#[test]
fn a_non_type_name_has_an_empty_level() {
    let mut f = fixture(&[], &[("src/lib.rs", "fn helper() {}\nfn main() { helper(); }\n")]);
    f.open("src/lib.rs");
    f.caret("helper");
    f.s.dispatch(Command::TypeHierarchy(Direction::Subtypes));
    f.settle(hierarchy_settled);
    assert_eq!(type_rows(&f.s), vec![(0, "helper".into())]);
    assert_eq!(f.s.state().results().unwrap().call(0).unwrap().fold, reader_core::search::Fold::Leaf);
}

#[test]
fn a_large_file_is_skipped_by_the_syntax_path() {
    let big = format!("impl Shape for Big {{}}\n{}", "// pad\n".repeat(800_000));
    let mut f = fixture(&[], &[("src/lib.rs", "pub trait Shape {}\npub struct Small;\nimpl Shape for Small {}\n"), ("src/big.rs", &big)]);
    f.open("src/lib.rs");
    f.caret("Shape");
    f.s.dispatch(Command::TypeHierarchy(Direction::Subtypes));
    f.settle(hierarchy_settled);
    assert_eq!(type_rows(&f.s), vec![(0, "Shape".into()), (1, "Small".into())]);
}

#[test]
fn switching_direction_drops_the_level_in_flight() {
    let mut f = fixture(&[], &[("src/lib.rs", SHAPES)]);
    f.open("src/lib.rs");
    f.caret("Shape");
    f.s.dispatch(Command::TypeHierarchy(Direction::Subtypes));
    f.s.dispatch(Command::HierarchyDirection(Direction::Supertypes));
    f.settle(hierarchy_settled);
    assert!(f.s.state().results().unwrap().kind().starts_with("Supertypes of Shape ("), "{}", f.s.state().results().unwrap().kind());
    assert_eq!(type_rows(&f.s), vec![(0, "Shape".into())]);
}

#[test]
#[ignore = "memory over churn; run in release, alone"]
fn type_hierarchy_churn() {
    let mut f = fixture(&[], &[("src/lib.rs", SHAPES)]);
    f.open("src/lib.rs");
    let rss = || std::fs::read_to_string("/proc/self/statm").ok().and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok()).unwrap_or(0) * 4;
    for round in 0..50 {
        f.caret("Shape");
        f.s.dispatch(Command::TypeHierarchy(Direction::Subtypes));
        f.settle(hierarchy_settled);
        f.s.dispatch(Command::CloseResults);
        if round % 10 == 9 {
            println!("round {} rss {} KiB", round + 1, rss());
        }
    }
}

#[test]
fn a_level_cut_at_the_usage_limit_says_it_may_be_incomplete() {
    // 250 usages: past a type level's 200 (Find Usages would read all of them).
    let uses: String = (0..250).map(|i| format!("fn f{i}(x: &dyn Shape) {{}}\n")).collect();
    let mut f = fixture(&[], &[("src/lib.rs", &format!("pub trait Shape {{}}\n{uses}pub struct Late;\nimpl Shape for Late {{}}\n"))]);
    f.open("src/lib.rs");
    f.caret("Shape");
    f.s.dispatch(Command::TypeHierarchy(Direction::Subtypes));
    f.settle(hierarchy_settled);
    f.settle(|s| s.state().notices().iter().any(|n| n.starts_with("Type hierarchy: only the first 200 usages of Shape") && n.contains("may be incomplete")));
}

#[test]
fn a_type_cycle_is_marked_and_duplicates_merge() {
    let mut f = fixture(&[("FAKE_LSP_TYPE_HIERARCHY", "1")], &[("src/lib.rs", "pub trait A: B {}\npub trait B: A {}\n")]);
    f.open("src/lib.rs");
    f.caret("A");
    f.s.dispatch(Command::TypeHierarchy(Direction::Supertypes));
    f.settle(hierarchy_settled);
    f.s.dispatch(Command::ToggleCall(1)); // B's supertypes: A again
    f.settle(hierarchy_settled);
    let r = f.s.state().results().unwrap();
    assert_eq!(r.call(2).map(|c| (c.name, c.fold)), Some(("A".into(), reader_core::search::Fold::Recursive)));
    // By syntax: `Fancy: Shape + Shape` names Shape twice on one declaration: one row.
    let mut f = fixture(&[], &[("src/lib.rs", "pub trait Shape {}\npub trait Fancy: Shape + Shape {}\n")]);
    f.open("src/lib.rs");
    f.caret("Shape");
    f.s.dispatch(Command::TypeHierarchy(Direction::Subtypes));
    f.settle(hierarchy_settled);
    assert_eq!(type_rows(&f.s), vec![(0, "Shape".into()), (1, "Fancy".into())]);
}

#[test]
fn without_type_hierarchy_or_usages_the_command_says_why() {
    let mut f = fixture(&[("FAKE_LSP_NO_REFERENCES", "1")], &[("src/lib.rs", SHAPES)]);
    f.open("src/lib.rs");
    f.caret("Shape");
    let why = f.s.actions().into_iter().find(|a| a.id == CommandId::TypeHierarchy).and_then(|a| a.disabled);
    assert_eq!(why, Some("rust-analyzer does not provide type hierarchy"));
}

#[test]
fn a_level_read_while_indexing_may_be_incomplete() {
    let mut f = fixture(&[("FAKE_LSP_INDEXING", "1")], &[("src/lib.rs", SHAPES)]);
    f.open("src/lib.rs");
    f.settle(|s| s.state().provider().is_some_and(|v| matches!(v.status, Status::Indexing(_))));
    f.caret("Shape");
    f.s.dispatch(Command::TypeHierarchy(Direction::Subtypes));
    f.settle(|s| hierarchy_settled(s) && s.state().results().unwrap().call(0).is_some_and(|c| c.fold != reader_core::search::Fold::Closed));
    assert!(f.s.state().results().unwrap().provider_indexing(), "the level was read while the server indexed");
}

#[test]
fn the_syntax_root_has_the_kind_of_the_declaration_at_the_caret() {
    // Sticky scroll and breadcrumbs off: the tab holds no declarations, so the root's are parsed in the background.
    let mut f = fixture(&[], &[("src/lib.rs", SHAPES)]);
    f.s.dispatch(Command::SetStickyScroll(false));
    f.s.dispatch(Command::SetBreadcrumbs(false));
    f.open("src/lib.rs");
    f.caret("Shape");
    f.s.dispatch(Command::TypeHierarchy(Direction::Subtypes));
    f.settle(|s| hierarchy_settled(s) && s.state().results().unwrap().call(0).is_some_and(|c| !c.label.is_empty()));
    assert_eq!(f.s.state().results().unwrap().call(0).unwrap().label, "trait");
    // Breadcrumbs on: the tab's own declarations name it.
    f.s.dispatch(Command::SetBreadcrumbs(true));
    f.settle(|s| active_view(s).is_some_and(|t| t.declarations().is_some()));
    f.caret("Circle");
    f.s.dispatch(Command::TypeHierarchy(Direction::Supertypes));
    f.settle(hierarchy_settled);
    assert_eq!(f.s.state().results().unwrap().call(0).unwrap().label, "type");
    // A name no declaration's name holds (`Circle` in an impl header): no kind.
    f.caret("Circle {}");
    f.s.dispatch(Command::TypeHierarchy(Direction::Supertypes));
    f.settle(hierarchy_settled);
    assert_eq!(f.s.state().results().unwrap().call(0).unwrap().label, "");
}

#[test]
fn a_question_waiting_for_its_server_ends_when_the_file_changes() {
    let mut f = fixture(&[("FAKE_LSP_INIT_DELAY_MS", "800")], &[("src/lib.rs", LIB), ("src/main.rs", MAIN)]);
    let main = f.root.join("src/main.rs");
    f.s.dispatch(Command::Open(main.clone()));
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source() == main));
    f.definition("helper();", false);
    assert!(f.s.state().asking().is_some() && !ready(&f.s), "precondition: waiting for the server");
    let version = f.s.state().current_source().unwrap().key().version;
    std::fs::write(&main, format!("// changed\n{MAIN}")).unwrap();
    f.settle(|s| s.state().current_source().unwrap().key().version > version);
    assert!(!ready(&f.s), "precondition: the file changed before the server was ready");
    f.settle(ready);
    let until = Instant::now() + Duration::from_millis(1000);
    while Instant::now() < until {
        if let Ok(c) = recv(&f.s.completions()) {
            f.s.dispatch(c);
        }
    }
    assert!(f.s.state().asking().is_none(), "still asking: {:?}", f.s.state().asking());
}

#[test]
fn a_definition_past_the_snippet_width_selects_its_name() {
    let lib = format!("{}pub fn helper() {{}}\n", " ".repeat(250));
    let mut f = fixture(&[], &[("src/lib.rs", &lib), ("src/main.rs", MAIN)]);
    f.open("src/main.rs");
    f.definition("helper();", false);
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("lib.rs")));
    assert_eq!(f.selected_text(), "helper");
}

#[test]
fn initialize_declares_call_hierarchy_support() {
    // typescript-language-server advertises call hierarchy only to a client that declares it.
    let mut f = fixture(&[], &[("src/lib.rs", LIB), ("src/main.rs", MAIN)]);
    f.open("src/main.rs");
    let init = messages(&f.log).into_iter().find(|m| m["method"] == "initialize").unwrap();
    assert!(init["params"]["capabilities"]["textDocument"].get("callHierarchy").is_some(), "{}", init["params"]["capabilities"]["textDocument"]);
}

/// A TypeScript 7 project (its TypeScript has no tsserver) is served by typescript-go, launched as the project's own
/// `node_modules/.bin/tsc` with `--lsp --stdio`, and named so.
#[test]
fn a_typescript_7_project_is_served_by_typescript_go_from_its_own_tsc() {
    let files = [("package.json", "{}\n"), ("node_modules/typescript/package.json", "{ \"name\": \"typescript\", \"version\": \"7.0.2\" }\n"), ("src/a.ts", "export const a = 1;\n"), ("src/lib/b.ts", "export const b = 2;\n")];
    // Nothing on PATH: only the project's own tsc can serve it.
    let mut f = fixture_as("typescript-go", &[], &files, serde_json::json!({ "executable": null, "env": { "PATH": "/nonexistent" } }), std::sync::Arc::new(Instant::now));
    let tsc = f.root.join("node_modules/.bin/tsc");
    std::fs::create_dir_all(tsc.parent().unwrap()).unwrap();
    std::fs::copy(f.root.parent().unwrap().join("fake-server"), &tsc).unwrap();
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("src/a.ts");
    let view = f.s.state().provider().unwrap();
    assert_eq!((view.name, view.root.as_deref()), ("typescript-go", Some(f.root.as_path())));
    let launch = messages(&f.log).into_iter().find(|m| m.get("args").is_some()).unwrap();
    assert_eq!(launch["args"], serde_json::json!(["--lsp", "--stdio"]));
    // The project now has a TypeScript with tsserver: a file in another folder of the same root still shares the
    // running server (one choice per root), until Restart chooses again.
    write(&f.root, "node_modules/typescript/lib/tsserver.js", "");
    f.open("src/lib/b.ts");
    assert_eq!(f.s.state().servers().len(), 1, "{:?}", f.s.state().servers());
    assert_eq!(f.s.state().provider().unwrap().name, "typescript-go");
    f.s.dispatch(Command::RestartServer);
    f.settle(|s| s.state().provider().is_some_and(|v| v.name == "typescript-language-server"));
    // Turning TypeScript off for the folder stops whichever server serves it.
    std::fs::remove_file(f.root.join("node_modules/typescript/lib/tsserver.js")).unwrap();
    f.s.dispatch(Command::RestartServer);
    f.settle(|s| s.state().provider().is_some_and(|v| v.name == "typescript-go" && matches!(v.status, Status::Ready | Status::Indexing(_))));
    f.s.dispatch(Command::SetServerEnabled(false));
    f.settle(|s| s.state().servers().iter().all(|v| v.name != "typescript-go"));
}

#[test]
fn server_runtime_selection_saves_restarts_and_keeps_disabled_servers_disabled() {
    let mut f = fixture_as("jdtls", &[], &[("Main.java", "class Main {}\n")], serde_json::json!({}), std::sync::Arc::new(Instant::now));
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.settle(|s| s.state().project().is_some());
    f.open("Main.java");
    let count = messages(&f.log).iter().filter(|m| m.get("args").is_some()).count();
    let java = f._dir.path().join("java with spaces");
    std::fs::write(&java, "#!/bin/sh\ntest \"$1\" = -version\n").unwrap();
    std::fs::set_permissions(&java, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let settings = f._dir.path().join("config/providers.json");
    let before = std::fs::read(&settings).unwrap();
    f.s.dispatch(Command::OpenServerRuntime);
    f.settle(|s| s.state().server_runtime().unwrap().message.starts_with("Enter"));
    f.s.dispatch(Command::CloseServerRuntime);
    assert!(f.s.state().server_runtime().is_none());
    assert_eq!(std::fs::read(&settings).unwrap(), before);
    f.s.dispatch(Command::OpenServerRuntime);
    f.s.dispatch(Command::SetServerRuntime(Some(f._dir.path().join("missing"))));
    f.settle(|s| s.state().server_runtime().unwrap().message.contains("not changed"));
    assert_eq!(std::fs::read(&settings).unwrap(), before);
    assert!(ready(&f.s));
    f.s.dispatch(Command::SetServerRuntime(Some(java.clone())));
    assert!(f.s.disabled(CommandId::DisableServer).is_some());
    f.s.dispatch(Command::SetServerEnabled(false)); // Must not race the settings save.
    f.settle(|s| s.state().server_runtime().unwrap().message.contains("saved."));
    let log = f.log.clone();
    f.settle_until(move || messages(&log).iter().filter(|m| m.get("args").is_some()).count() > count);
    f.settle(ready);
    let arg = format!("--java-executable={}", java.display());
    assert!(messages(&f.log).iter().any(|m| m["args"].as_array().is_some_and(|a| a.contains(&serde_json::json!(arg)))));
    let value: Value = serde_json::from_slice(&std::fs::read(&settings).unwrap()).unwrap();
    assert_eq!(value["providers"]["jdtls"]["args"], serde_json::json!([arg]));
    f.s.dispatch(Command::SetServerEnabled(false));
    f.settle(|s| s.disabled(CommandId::ServerRuntime).is_none());
    f.s.dispatch(Command::SetServerRuntime(None));
    f.settle(|s| s.state().server_runtime().unwrap().message.contains("saved."));
    f.settle(|s| s.state().language_server_label() == "LSP off");
    assert_eq!(std::fs::read_to_string(f.root.join("Main.java")).unwrap(), "class Main {}\n");
}

#[test]
fn an_ignored_dependency_inside_the_project_appears_in_libraries_only() {
    let mut f = fixture_as("typescript-language-server", &[], &[
        ("package.json", "{}"), (".gitignore", "node_modules/\n"), ("main.ts", "from_dep();\n"),
        ("node_modules/@test/library/index.ts", "export function from_dep() {}\n"),
        ("node_modules/@test/library/sibling.ts", "export function sibling() {}\n"),
    ], serde_json::json!({}), std::sync::Arc::new(Instant::now));
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("main.ts");
    f.definition("from_dep", false);
    f.settle(|s| s.state().tree_rows().any(|r| r.name.as_ref() == "sibling.ts"));
    assert!(f.s.state().current_source().unwrap().source().ends_with("node_modules/@test/library/index.ts"));
    assert!(f.s.state().tree_rows().any(|r| r.name.as_ref() == "Libraries"));
    assert!(!f.s.state().project().unwrap().children("").iter().any(|(name, _)| name.as_ref() == "node_modules"));
    assert_eq!(f.s.state().project().unwrap().root(), f.root);
}

#[test]
fn a_library_source_held_in_another_pane_opens_without_reading_a_uri_from_disk() {
    let mut f = fixture_as("jdtls", &[("FAKE_LSP_JDT", "1")], &[("pom.xml", ""), ("src/A.java", "class A { void m() { virt(); } }\n")], serde_json::json!({}), std::sync::Arc::new(Instant::now));
    f.open("src/A.java");
    f.definition("virt()", false);
    f.settle(|s| s.state().active().is_some_and(|t| t.retrieved().is_some()));
    let uri = f.s.state().active().unwrap().source().to_string_lossy().into_owned();
    let doc = f.s.state().current_source().unwrap().clone();
    f.s.dispatch(Command::Split(reader_core::session::Split::Columns));
    // Pane 1 keeps a file of its own (closing its last tab would close the split) and lets go of the source.
    f.open("src/A.java");
    let twin = f.s.state().pane_tabs(1).find(|t| t.retrieved().is_some()).unwrap().id();
    f.s.dispatch(Command::CloseTab(twin));
    f.s.dispatch(Command::FocusPane(1));
    f.s.dispatch(Command::ActivateTree { path: uri.into(), retain: false });
    assert_eq!(f.s.state().focused_pane(), 1);
    assert!(f.s.state().active().unwrap().retrieved().is_some());
    assert!(std::sync::Arc::ptr_eq(f.s.state().current_source().unwrap(), &doc));
    assert_eq!(messages(&f.log).iter().filter(|m| m["method"] == "java/classFileContents").count(), 1);
}

#[test]
fn standalone_navigation_within_the_servers_workspace_does_not_create_a_library() {
    let mut f = fixture(&[], &[("src/main.rs", "fn main() { helper(); }\n"), ("src/inner/helper.rs", "pub fn helper() {}\n")]);
    f.open("src/main.rs");
    assert!(f.s.state().project().is_none());
    f.definition("helper()", false);
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("inner/helper.rs")));
    assert!(!f.s.state().tree_rows().any(|r| r.name.as_ref() == "Libraries"));
}

impl Fixture {
    /// Type `text` at the caret as one keystroke of the active document's draft.
    fn type_text(&mut self, text: &str) {
        let tab = self.s.state().active().unwrap().id();
        let view = self.s.state().view(tab).unwrap();
        let at = view.tab().selection().map(|s| s.range()).unwrap();
        let expected = view.snapshot().key();
        self.s.dispatch(Command::Edit { tab, expected, edits: vec![reader_core::text::TextEdit { range: at, text: text.into() }], kind: reader_core::text::EditKind::Typing });
    }
    fn labels(&self) -> Vec<String> {
        self.s.state().completion().map(|c| c.rows().map(|r| r.label.clone()).collect()).unwrap_or_default()
    }
}

#[test]
fn manual_completion_accepts_one_undoable_replacement_of_the_current_version() {
    let text = "fn main() { hel }\nfn helper() {}\nfn hello_world() {}\n";
    let mut f = editing_fixture(&[], &[("src/main.rs", text)]);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("src/main.rs");
    let at = text.find("hel }").unwrap() + 3;
    f.s.dispatch(Command::Select(Selection { anchor: at, head: at }));
    f.s.dispatch(Command::Complete);
    f.settle(|s| s.state().completion().is_some());
    assert_eq!(f.labels(), ["hello_world", "helper"]);
    assert_eq!(f.s.state().completion().unwrap().selected(), 0);
    f.s.dispatch(Command::MoveCompletion(1));
    assert_eq!(f.s.state().completion().unwrap().selected(), 1);
    f.s.dispatch(Command::AcceptCompletion);
    assert!(f.s.state().completion().is_none());
    assert_eq!(current_text(f.s.state()), text.replacen("hel }", "helper }", 1));
    assert_eq!(f.s.state().selection().unwrap().head, at + 3, "the caret follows the inserted name");
    assert_eq!(std::fs::read_to_string(f.root.join("src/main.rs")).unwrap(), text, "only the draft changes");
    let tab = f.s.state().active().unwrap().id();
    f.s.dispatch(Command::Undo { tab });
    assert_eq!(current_text(f.s.state()), text, "one Undo restores the completion");
}

#[test]
fn dismissed_or_left_completion_is_not_reopened_by_its_late_reply() {
    let text = "fn main() { slow_ }\nfn slow_alpha() {}\n";
    let mut f = editing_fixture(&[], &[("src/main.rs", text)]);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("src/main.rs");
    let at = text.find("slow_ }").unwrap() + 5;
    let asked = |log: &Path| messages(log).iter().filter(|m| m["method"] == "textDocument/completion").count();
    let wait_late = |f: &mut Fixture, n: usize| {
        let log = f.log.clone();
        f.settle_until(move || asked(&log) == n);
        let until = Instant::now() + Duration::from_millis(700);
        while Instant::now() < until { let rx = f.s.completions(); if let Ok(c) = recv(&rx) { f.s.dispatch(c); } }
    };
    // Escape before the slow answer: it never opens.
    f.s.dispatch(Command::Select(Selection { anchor: at, head: at }));
    f.s.dispatch(Command::Complete);
    f.s.dispatch(Command::CloseCompletion);
    wait_late(&mut f, 1);
    assert!(f.s.state().completion().is_none(), "a dismissed popup stays dismissed");
    // Leaving for another tab: it opens on neither.
    f.s.dispatch(Command::Complete);
    f.s.dispatch(Command::Open(f.root.join("Cargo.toml")));
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("Cargo.toml")));
    wait_late(&mut f, 2);
    assert!(f.s.state().completion().is_none());
    f.s.dispatch(Command::Back);
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("main.rs")));
    assert!(f.s.state().completion().is_none(), "not kept for its return");
    // Moving the caret: the answer about the old place is dropped.
    f.s.dispatch(Command::Select(Selection { anchor: at, head: at }));
    f.s.dispatch(Command::Complete);
    f.s.dispatch(Command::Select(Selection { anchor: 1, head: 1 }));
    wait_late(&mut f, 3);
    assert!(f.s.state().completion().is_none());
    assert_eq!(current_text(f.s.state()), text);
}

#[test]
fn typing_opens_filters_and_closes_completion_without_accepting_punctuation() {
    let text = "fn main() {  }\nfn helper() {}\nfn hello() {}\nstruct Item;\n";
    let mut f = editing_fixture(&[], &[("src/main.rs", text)]);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("src/main.rs");
    let at = text.find("{  }").unwrap() + 2;
    f.s.dispatch(Command::Select(Selection { anchor: at, head: at }));
    assert!(f.s.state().auto_completion());
    f.type_text("h");
    f.settle(|s| s.state().completion().is_some());
    assert_eq!(f.s.state().completion().unwrap().trigger, reader_core::session::CompletionTrigger::Typing);
    assert_eq!(f.labels(), ["hello", "helper"]);
    // Continuing the name keeps it open, filtered to the new version at once.
    f.type_text("elp");
    assert_eq!(f.labels(), ["helper"]);
    // Punctuation types itself and closes the popup.
    f.type_text("(");
    assert!(f.s.state().completion().is_none());
    assert_eq!(current_text(f.s.state()), text.replacen("{  }", "{ help( }", 1));
    // A trigger character asks too; Backspace past the name's start closes it.
    f.type_text("Item.");
    f.settle(|s| s.state().completion().is_some());
    assert_eq!(f.s.state().completion().unwrap().word(), f.s.state().selection().unwrap().head);
    f.s.dispatch(Command::Delete { backward: true });
    assert!(f.s.state().completion().is_none());
    // Off: typing asks nothing, the command still works, and the setting is remembered.
    f.s.dispatch(Command::SetAutoCompletion(false));
    let before = messages(&f.log).iter().filter(|m| m["method"] == "textDocument/completion").count();
    f.type_text(" he");
    let until = Instant::now() + Duration::from_millis(300);
    while Instant::now() < until { let rx = f.s.completions(); if let Ok(c) = recv(&rx) { f.s.dispatch(c); } }
    assert!(f.s.state().completion().is_none());
    assert_eq!(messages(&f.log).iter().filter(|m| m["method"] == "textDocument/completion").count(), before);
    f.s.dispatch(Command::Complete);
    f.settle(|s| s.state().completion().is_some());
    assert_eq!(f.labels(), ["hello", "helper"]);
    let settings = f._dir.path().join("state/settings.json");
    f.settle_until(|| std::fs::read_to_string(&settings).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()).is_some_and(|v| v["auto_completion"] == false));
    assert_eq!(std::fs::read_to_string(f.root.join("src/main.rs")).unwrap(), text);
}

#[test]
fn unavailable_completion_says_why_and_never_applies_part_of_a_suggestion() {
    let text = "fn main() { with }\n";
    let mut f = editing_fixture(&[], &[("src/main.rs", text), ("notes.txt", "he\n")]);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("src/main.rs");
    let at = text.find("with }").unwrap() + 4;
    f.s.dispatch(Command::Select(Selection { anchor: at, head: at }));
    f.s.dispatch(Command::Complete);
    f.settle(|s| s.state().completion().is_some());
    assert_eq!(f.labels(), ["with_import"]);
    let row = f.s.state().completion().unwrap().rows().next().unwrap().clone();
    assert_eq!(row.unavailable, Some("Suggestions with import or other extra edits are not supported yet"));
    f.s.dispatch(Command::AcceptCompletion);
    assert!(f.notice("import or other extra edits"));
    assert_eq!(current_text(f.s.state()), text, "neither the name nor the import is applied");
    f.s.dispatch(Command::CloseCompletion);
    // A snippet is refused the same way.
    f.type_text(" snip");
    f.settle(|s| s.state().completion().is_some());
    assert_eq!(f.labels(), ["snippet_call"]);
    let before = current_text(f.s.state());
    f.s.dispatch(Command::AcceptCompletion);
    assert!(f.notice("Snippet suggestions are not supported yet"));
    assert_eq!(current_text(f.s.state()), before);
    // No provider for the language: the command is unavailable and says so.
    f.s.dispatch(Command::Open(f.root.join("notes.txt")));
    f.settle(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("notes.txt")));
    f.s.dispatch(Command::Select(Selection { anchor: 2, head: 2 }));
    assert_eq!(f.s.disabled(CommandId::Completion), Some("No language server for this language"));
    f.s.dispatch(Command::Complete);
    assert!(f.notice("Code completion: No language server for this language"));
}

#[test]
fn a_server_without_completion_is_reported_as_unavailable() {
    let mut f = editing_fixture(&[("FAKE_LSP_NO_COMPLETION", "1")], &[("src/main.rs", "fn main() { he }\nfn helper() {}\n")]);
    f.s.dispatch(Command::OpenRoot(f.root.clone()));
    f.open("src/main.rs");
    f.caret("e }");
    assert_eq!(f.s.disabled(CommandId::Completion), Some("The language server does not offer completion"));
    f.type_text("l");
    assert!(f.s.state().completion().is_none());
    assert!(!messages(&f.log).iter().any(|m| m["method"] == "textDocument/completion"));
}
