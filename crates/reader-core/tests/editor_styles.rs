use std::{sync::Arc, time::{Duration, Instant}};
use lenz_content::highlight::Capture;
use reader_core::session::{Command, Config, Session};

#[test]
fn snapshot_style_queries_keep_syntax_semantic_precedence_and_modifier_flags() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"style-query\"\nversion = \"0.1.0\"\n").unwrap();
    let path = root.join("main.rs");
    let source = format!("fn old_helper() {{ let mut value = 1; }} // muted\n{}fn ordinary() {{}}\n// fn old_hidden() {{ let mut misleading = 1; }}\n", " ".repeat(12_000));
    std::fs::write(&path, &source).unwrap();
    let profile = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_owned();
    let mut build = std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    build.args(["build", "-q", "-p", "reader-core", "--example", "fake_lsp"]).current_dir(env!("CARGO_MANIFEST_DIR"));
    if profile.ends_with("release") { build.arg("--release"); }
    assert!(build.status().unwrap().success());
    let fake = profile.join(format!("examples/fake_lsp{}", std::env::consts::EXE_SUFFIX));
    let providers = dir.path().join("providers.json");
    std::fs::write(&providers, serde_json::json!({"providers":{"rust-analyzer":{"executable":fake}}}).to_string()).unwrap();
    let mut s = Session::new(Config {
        themes: lenz_content::theme::ThemePaths::under(&dir.path().join("config"), &dir.path().join("state"), "lenz-reader"),
        recent: dir.path().join("recent.json"), settings: dir.path().join("settings.json"), providers,
        cache: dir.path().join("cache"), idle: Duration::ZERO, clock: Arc::new(Instant::now), overrides: Default::default(), dark: true,
    });
    s.dispatch(Command::OpenRoot(root));
    s.dispatch(Command::Open(path.clone()));
    s.dispatch(Command::SetSemanticHighlighting(true));
    let until = Instant::now() + Duration::from_secs(10);
    while !s.state().active().and_then(|tab| s.state().view(tab.id())).is_some_and(|view| view.highlights().is_some() && view.tokens().is_some()) {
        assert!(Instant::now() < until, "syntax or semantic tokens timed out");
        while let Ok(completion) = s.completions().try_recv() { s.dispatch(completion); }
        std::thread::yield_now();
    }
    let tab = s.state().active().unwrap().id();
    let view = s.state().view(tab).unwrap();
    assert_eq!(view.style_at(0, 0), (Some(Capture::Keyword), 0));
    assert_eq!(view.style_at(0, 3), (Some(Capture::Function), reader_core::decor::DEPRECATED));
    assert_eq!(view.style_at(0, source.find("value").unwrap()), (Some(Capture::Variable), reader_core::decor::MUTABLE));
    assert_eq!(view.style_at(0, source.find("muted").unwrap()), (Some(Capture::Comment), 0));
    assert_eq!(view.style_at(1, 12_000), (Some(Capture::Keyword), 0));
    assert_eq!(view.style_at(2, 6), (Some(Capture::Function), reader_core::decor::DEPRECATED));
    assert_eq!(view.linkable(2, 9), None, "syntax comments remain suppressed for hover despite a semantic color override");
    assert_eq!(view.style_at(usize::MAX, 0), (None, 0));
    assert_eq!(view.style_at(0, usize::MAX), (None, 0));
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}
