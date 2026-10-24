//! Actual shared-Session editing/history workload. Common12/13 APIs permit paired historical builds.
//! Run only as an explicitly released campaign; ordinary unit self-checks use a fixed two-cycle trace.
use reader_core::{
    session::{Command, Config, Session},
    tabs::TabId,
    text::{EditKind, TextEdit},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn text(session: &Session) -> String {
    let source = session.state().current_source().expect("loaded fixture");
    source
        .copy_range(0..source.len_bytes())
        .expect("oracle range")
}
fn memory() -> Value {
    let Ok(raw) = fs::read_to_string("/proc/self/status") else {
        return Value::Null;
    };
    let field = |key: &str| {
        raw.lines()
            .find_map(|line| line.strip_prefix(key))
            .and_then(|v| v.split_whitespace().next()?.parse::<u64>().ok())
    };
    json!({"rss_kib":field("VmRSS:"),"peak_rss_kib":field("VmHWM:")})
}
fn session(root: &Path) -> Session {
    let providers = root.join("providers.json");
    fs::create_dir_all(root).unwrap();
    let map: serde_json::Map<String, Value> = reader_core::semantics::provider::PROVIDERS
        .iter()
        .map(|p| {
            (
                p.id.into(),
                json!({"executable":format!("/nonexistent/{}",p.id)}),
            )
        })
        .collect();
    fs::write(&providers, json!({"providers":map}).to_string()).unwrap();
    Session::new(Config {
        themes: lenz_content::theme::ThemePaths::under(
            &root.join("config"),
            &root.join("state"),
            "lenz-reader",
        ),
        recent: root.join("recent.json"),
        settings: root.join("settings.json"),
        providers,
        cache: root.join("cache"),
        idle: Duration::ZERO,
        clock: Arc::new(Instant::now),
        overrides: Default::default(),
        dark: true,
    })
}
fn drain(session: &mut Session) {
    while let Ok(done) = session.completions().try_recv() {
        session.dispatch(done);
    }
}
fn settle(session: &mut Session, done: impl Fn(&Session) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done(session) {
        assert!(Instant::now() < deadline, "shared command timed out");
        drain(session);
        std::thread::yield_now();
    }
}
fn open(session: &mut Session, path: &Path) -> TabId {
    session.dispatch(Command::OpenRetained(path.to_owned()));
    settle(session, |s| {
        s.state()
            .current_source()
            .is_some_and(|v| v.source() == path)
            && s.state().loading().is_none()
    });
    session.state().active().unwrap().id()
}
fn edit(
    session: &mut Session,
    tab: TabId,
    range: std::ops::Range<usize>,
    value: &str,
    kind: EditKind,
) -> f64 {
    let expected = session.state().view(tab).unwrap().snapshot().key();
    let started = Instant::now();
    session.dispatch(Command::Edit {
        tab,
        expected,
        edits: vec![TextEdit {
            range,
            text: value.into(),
        }],
        kind,
    });
    started.elapsed().as_secs_f64() * 1000.0
}
struct Case {
    label: String,
    path: PathBuf,
    tab: TabId,
    original: String,
    oracle: String,
    saved: String,
    transactions: u64,
    inserted: u64,
    removed: u64,
}
struct Workload {
    primary: Session,
    independent: Session,
    independent_text: String,
    cases: Vec<Case>,
    cycle: u64,
    started: Instant,
}
impl Workload {
    fn new(out: &Path, inputs: &[(&str, &Path)]) -> Self {
        let mut primary = session(&out.join("primary"));
        let mut cases = Vec::new();
        let sources = out.join("sources");
        fs::create_dir(&sources).unwrap();
        for (label, input) in inputs {
            let original = fs::read_to_string(input).unwrap();
            let path = sources.join(input.file_name().unwrap());
            fs::write(&path, &original).unwrap();
            let tab = open(&mut primary, &path);
            cases.push(Case {
                label: (*label).into(),
                path,
                tab,
                original: original.clone(),
                oracle: original.clone(),
                saved: original,
                transactions: 0,
                inserted: 0,
                removed: 0,
            });
        }
        let mut independent = session(&out.join("independent"));
        let other = open(&mut independent, &cases[0].path);
        edit(
            &mut independent,
            other,
            0..0,
            "independent ",
            EditKind::Replace,
        );
        let independent_text = format!("independent {}", cases[0].original);
        Self {
            primary,
            independent,
            independent_text,
            cases,
            cycle: 0,
            started: Instant::now(),
        }
    }
    fn step(&mut self, out: &mut impl Write, save_every: u64) -> Result<()> {
        for case in &mut self.cases {
            self.primary.dispatch(Command::ActivateTab(case.tab));
            settle(&mut self.primary, |s| {
                s.state().current_source().is_some() && s.state().loading().is_none()
            });
            assert_eq!(text(&self.primary), case.oracle);
            let prior = case.oracle.clone();
            let mut at = match self.cycle % 3 {
                0 => 0,
                1 => case.oracle.len() / 2,
                _ => case.oracle.len(),
            };
            while !case.oracle.is_char_boundary(at) {
                at -= 1;
            }
            let end = if self.cycle % 3 == 1 {
                at + case.oracle[at..].chars().next().map_or(0, char::len_utf8)
            } else {
                at
            };
            let inserted = if self.cycle.is_multiple_of(9) {
                "λ😀\r\npaste\n".to_owned()
            } else {
                format!("#{}λ", self.cycle)
            };
            let latency = edit(
                &mut self.primary,
                case.tab,
                at..end,
                &inserted,
                if self.cycle.is_multiple_of(9) {
                    EditKind::Paste
                } else {
                    EditKind::Replace
                },
            );
            case.oracle.replace_range(at..end, &inserted);
            case.transactions += 1;
            case.inserted += inserted.len() as u64;
            case.removed += (end - at) as u64;
            assert_eq!(text(&self.primary), case.oracle);
            let mut undo_ms = None;
            let mut redo_ms = None;
            let mut save_ms = None;
            if self.cycle.is_multiple_of(8) {
                let now = Instant::now();
                self.primary.dispatch(Command::Undo { tab: case.tab });
                undo_ms = Some(now.elapsed().as_secs_f64() * 1000.0);
                assert_eq!(text(&self.primary), prior);
                let now = Instant::now();
                self.primary.dispatch(Command::Redo { tab: case.tab });
                redo_ms = Some(now.elapsed().as_secs_f64() * 1000.0);
                assert_eq!(text(&self.primary), case.oracle);
            }
            if self.cycle.is_multiple_of(save_every) {
                let captured = case.oracle.clone();
                let now = Instant::now();
                self.primary.dispatch(Command::Save { tab: case.tab });
                // Exercise captured-save fencing against a newer edit using the common13 API.
                if self.cycle.is_multiple_of(save_every * 4) {
                    edit(
                        &mut self.primary,
                        case.tab,
                        0..0,
                        "newer ",
                        EditKind::Replace,
                    );
                    case.oracle.insert_str(0, "newer ");
                    case.transactions += 1;
                    case.inserted += 6;
                }
                settle(&mut self.primary, |s| {
                    matches!(
                        s.state().view(case.tab).unwrap().save_status(),
                        reader_core::save::Status::Ready
                    )
                });
                save_ms = Some(now.elapsed().as_secs_f64() * 1000.0);
                assert_eq!(fs::read_to_string(&case.path)?, captured);
                case.saved = captured;
                assert_eq!(
                    self.primary.state().view(case.tab).unwrap().dirty(),
                    case.oracle != case.saved
                );
            }
            if self.cycle.is_multiple_of(32) {
                self.primary
                    .dispatch(Command::Split(reader_core::session::Split::Columns));
                let twin = self.primary.state().active().unwrap().id();
                assert_ne!(twin, case.tab);
                assert_eq!(
                    self.primary
                        .state()
                        .view(twin)
                        .unwrap()
                        .snapshot()
                        .key()
                        .buffer,
                    self.primary
                        .state()
                        .view(case.tab)
                        .unwrap()
                        .snapshot()
                        .key()
                        .buffer
                );
                self.primary.dispatch(Command::FocusPane(0));
                self.primary.dispatch(Command::Unsplit);
            }
            drain(&mut self.primary);
            drain(&mut self.independent);
            assert_eq!(text(&self.independent), self.independent_text);
            let view = self.primary.state().view(case.tab).unwrap();
            writeln!(
                out,
                "{}",
                json!({"schema":1,"phase":"cycle","case":case.label,"cycle":self.cycle,
                "seconds":self.started.elapsed().as_secs_f64(),"edit_ms":latency,"undo_ms":undo_ms,"redo_ms":redo_ms,
                "save_confirmed_ms":save_ms,"transactions":case.transactions,"inserted_bytes":case.inserted,"removed_bytes":case.removed,
                "current_bytes":case.oracle.len(),"current_storage_bytes":view.snapshot().storage_bytes(),"syntax_ready":view.highlights().is_some(),"memory":memory()})
            )?;
        }
        self.cycle += 1;
        Ok(())
    }
    fn verify_history(&mut self) -> Result<()> {
        for case in &self.cases {
            self.primary.dispatch(Command::ActivateTab(case.tab));
            settle(&mut self.primary, |s| {
                s.state().current_source().is_some() && s.state().loading().is_none()
            });
            for _ in 0..case.transactions {
                self.primary.dispatch(Command::Undo { tab: case.tab });
            }
            assert_eq!(
                text(&self.primary),
                case.original,
                "all history retained for {}",
                case.label
            );
            assert!(!self.primary.state().view(case.tab).unwrap().can_undo());
            for _ in 0..case.transactions {
                self.primary.dispatch(Command::Redo { tab: case.tab });
            }
            assert_eq!(text(&self.primary), case.oracle);
            assert_eq!(
                self.primary.state().view(case.tab).unwrap().dirty(),
                case.oracle != case.saved
            );
            assert_eq!(fs::read_to_string(&case.path)?, case.saved);
        }
        drain(&mut self.independent);
        assert_eq!(text(&self.independent), self.independent_text);
        Ok(())
    }
}
fn sha(path: &Path) -> Result<String> {
    // Hash large executables without contaminating this process's RSS/HWM with a full binary copy.
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 4 {
        return Err(
            "usage: foundation-edit <NEW output> <readiness fixtures> <seconds>=180 <source SHA>"
                .into(),
        );
    }
    let out = PathBuf::from(&args[0]);
    let fixtures = PathBuf::from(&args[1]);
    let seconds: u64 = args[2].parse()?;
    if seconds < 180 {
        return Err(
            "campaign requires at least180 seconds; use unit self-checks for tooling validation"
                .into(),
        );
    }
    let files = [
        fixtures.join("rust/module_0.rs"),
        fixtures.join("large.txt"),
        fixtures.join("long-unicode.txt"),
    ];
    let rust = fs::read_to_string(&files[0])?;
    let large = fs::read_to_string(&files[1])?;
    assert_eq!(rust.lines().count(), 2000);
    assert_eq!(large.lines().count(), 100000);
    assert_eq!(fs::metadata(&files[2])?.len(), 900001);
    let fixture_hashes: Vec<_> = files.iter().map(|path| sha(path)).collect::<Result<_>>()?;
    fs::create_dir(&out)?;
    let mut log = BufWriter::new(
        fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(out.join("samples.jsonl"))?,
    );
    writeln!(
        log,
        "{}",
        json!({"schema":1,"phase":"identity","source_claim":args[3],"workload_sha256":format!("{:x}",Sha256::digest(include_bytes!("foundation_edit.rs"))),"executable_sha256":sha(&std::env::current_exe()?)?,
        "fixtures":files.iter().map(|p|Ok(json!({"file":p,"sha256":sha(p)?}))).collect::<Result<Vec<_>>>()?,
        "scope":"actual shared Session commands; oracle and observer overhead excluded from edit dispatch samples but included in process RSS; no GUI/IME latency claim; history growth is intentional"})
    )?;
    let mut work = Workload::new(
        &out,
        &[
            ("rust", &files[0]),
            ("text", &files[1]),
            ("unicode", &files[2]),
        ],
    );
    writeln!(
        log,
        "{}",
        json!({"schema":1,"phase":"loaded","memory":memory(),"history_transactions":0})
    )?;
    while work.started.elapsed() < Duration::from_secs(seconds) {
        work.step(&mut log, 64)?;
        log.flush()?;
    }
    let actual = work.started.elapsed().as_secs_f64();
    let memory_before_validation = memory();
    let validation = Instant::now();
    work.verify_history()?;
    for (path, expected) in files.iter().zip(&fixture_hashes) {
        assert_eq!(&sha(path)?, expected, "readiness originals stay unchanged");
    }
    writeln!(
        log,
        "{}",
        json!({"schema":1,"phase":"complete","requested_seconds":seconds,"actual_editing_seconds":actual,
        "cycles":work.cycle,"history_validation_seconds":validation.elapsed().as_secs_f64(),"memory_before_full_undo":memory_before_validation,"memory_after_full_redo":memory(),
        "checks":["exact text each edit/Undo/Redo","confirmed captured Save with newer edit retained","split buffer identity","independent dirty Session unchanged","all independent history transactions survive full Undo/Redo","only explicit Saves modify fixture copies"],
        "limits":["RSS includes deterministic oracle storage and history; no leak inference","no history cap","providers disabled; syntax background analysis only","candidate-only new UI timings are a separate workload"]})
    )?;
    log.flush()?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn common_session_harness_self_check_uses_exact_oracles_and_full_history() {
        let d = tempfile::tempdir().unwrap();
        let sources: Vec<_> = ["input.rs", "other.txt", "unicode.txt"]
            .into_iter()
            .map(|name| {
                let source = d.path().join(name);
                fs::write(&source, "fn original() {}\r\nλ😀\n").unwrap();
                source
            })
            .collect();
        let out = d.path().join("run");
        fs::create_dir(&out).unwrap();
        let inputs: Vec<_> = ["rust", "text", "unicode"]
            .into_iter()
            .zip(sources.iter().map(PathBuf::as_path))
            .collect();
        let mut work = Workload::new(&out, &inputs);
        let mut log = Vec::new();
        for _ in 0..2 {
            work.step(&mut log, 1).unwrap();
        }
        work.verify_history().unwrap();
        let rows: Vec<Value> = String::from_utf8(log)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(rows.len(), 6);
        assert_eq!(rows[3]["transactions"], 3);
        assert_eq!(
            fs::read_to_string(&sources[0]).unwrap(),
            "fn original() {}\r\nλ😀\n"
        );
    }
}
