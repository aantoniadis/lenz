//! Readiness experiment only: these owners and stores are not production editor APIs.
use std::{cell::Cell, ops::Range, path::PathBuf, sync::Arc};
use reader_core::{cache::{InactiveCache, Stamp}, document::Document};

#[derive(Clone, Copy, Debug)]
enum Kind { Gap, Chunks }

#[derive(Clone)]
struct Gap { bytes: Vec<u8>, start: usize, end: usize }
impl Gap {
    fn new(value: &str) -> Self {
        let mut bytes = value.as_bytes().to_vec();
        bytes.resize(value.len() + 1024, 0);
        Self { bytes, start: value.len(), end: value.len() + 1024 }
    }
    fn replace(&mut self, range: Range<usize>, value: &str) -> usize {
        let moved = self.start.abs_diff(range.start);
        if range.start < self.start {
            self.bytes.copy_within(range.start..self.start, self.end - moved);
            self.start -= moved; self.end -= moved;
        } else {
            self.bytes.copy_within(self.end..self.end + moved, self.start);
            self.start += moved; self.end += moved;
        }
        self.end += range.len();
        let mut copied = moved + value.len();
        if self.end - self.start < value.len() {
            let extra = value.len().max(1024);
            let old_len = self.bytes.len();
            self.bytes.resize(old_len + extra, 0);
            self.bytes.copy_within(self.end..old_len, self.end + extra);
            copied += old_len - self.end;
            self.end += extra;
        }
        self.bytes[self.start..self.start + value.len()].copy_from_slice(value.as_bytes());
        self.start += value.len();
        copied
    }
}

#[derive(Clone)]
struct Chunk { text: String, newlines: Vec<usize> }
impl Chunk {
    fn split(text: &str) -> Vec<Arc<Self>> {
        let mut chunks = Vec::new();
        let mut start = 0;
        while start < text.len() {
            let mut end = (start + 4096).min(text.len());
            while !text.is_char_boundary(end) { end -= 1; }
            let text = text[start..end].to_owned();
            let newlines = text.match_indices('\n').map(|(i,_)| i).collect();
            chunks.push(Arc::new(Self { text, newlines }));
            start = end;
        }
        if chunks.is_empty() { chunks.push(Arc::new(Self { text: String::new(), newlines: Vec::new() })); }
        chunks
    }
}

#[derive(Clone)]
enum Storage { Gap(Gap), Chunks(Arc<Vec<Arc<Chunk>>>) }
#[derive(Clone)]
struct Text { data: Storage, copied: usize, root_pointer_bytes: usize, materializations: Cell<usize>, materialized_bytes: Cell<usize> }
impl Text {
    fn new(kind: Kind, value: &str) -> Self {
        Self { data: match kind { Kind::Gap => Storage::Gap(Gap::new(value)), Kind::Chunks => Storage::Chunks(Arc::new(Chunk::split(value))) },
            copied: 0, root_pointer_bytes: 0, materializations: Cell::new(0), materialized_bytes: Cell::new(0) }
    }
    fn visit(&self, mut f: impl FnMut(&[u8])) {
        match &self.data {
            Storage::Gap(g) => { f(&g.bytes[..g.start]); f(&g.bytes[g.end..]); }
            Storage::Chunks(chunks) => for chunk in chunks.iter() { f(chunk.text.as_bytes()); },
        }
    }
    fn len(&self) -> usize { let mut len = 0; self.visit(|s| len += s.len()); len }
    fn slice(&self, range: Range<usize>) -> String {
        if range.start == 0 && range.end == self.len() {
            self.materializations.set(self.materializations.get()+1);
            self.materialized_bytes.set(self.materialized_bytes.get()+range.len());
        }
        let mut bytes = Vec::with_capacity(range.len());
        let mut offset = 0;
        self.visit(|s| {
            let start = range.start.saturating_sub(offset).min(s.len());
            let end = range.end.saturating_sub(offset).min(s.len());
            if end > start { bytes.extend_from_slice(&s[start..end]); }
            offset += s.len();
        });
        String::from_utf8(bytes).unwrap()
    }
    fn boundary(&self, at: usize) -> bool {
        let mut offset = 0; let mut byte = None;
        self.visit(|s| { if at >= offset && at < offset + s.len() { byte = Some(s[at-offset]); } offset += s.len(); });
        byte.map_or(at == offset, |b| b & 0xc0 != 0x80)
    }
    fn replace(&mut self, range: Range<usize>, text: &str) {
        assert!(range.start <= range.end && self.boundary(range.start) && self.boundary(range.end));
        match &mut self.data {
            Storage::Gap(g) => self.copied += g.replace(range, text),
            Storage::Chunks(chunks) => {
                let locate = |at: usize| {
                    let mut offset = 0;
                    for (i, chunk) in chunks.iter().enumerate() {
                        if at <= offset + chunk.text.len() { return (i, at-offset); }
                        offset += chunk.text.len();
                    }
                    unreachable!("validated range")
                };
                let (first, start) = locate(range.start);
                let (last, end) = locate(range.end);
                let neighbour = chunks.get(last+1).map_or("", |c| c.text.as_str());
                let mut rebuilt = String::with_capacity(start + text.len() + chunks[last].text.len()-end + neighbour.len());
                rebuilt.push_str(&chunks[first].text[..start]);
                rebuilt.push_str(text);
                rebuilt.push_str(&chunks[last].text[end..]);
                // Rechunk one neighbour too, so repeated short edits do not grow one fragment per key.
                let after = if last + 1 < chunks.len() { rebuilt.push_str(&chunks[last+1].text); last+2 } else { last+1 };
                self.copied += rebuilt.len()*2; // rebuild, then owned chunk text
                self.root_pointer_bytes += (first + chunks.len()-after)*size_of::<Arc<Chunk>>();
                let mut next = Vec::with_capacity(chunks.len()+rebuilt.len()/4096+2);
                next.extend_from_slice(&chunks[..first]);
                next.extend(Chunk::split(&rebuilt));
                next.extend_from_slice(&chunks[after..]);
                *chunks = Arc::new(next);
            }
        }
    }
    fn string(&self) -> String {
        self.slice(0..self.len())
    }
    fn line_start(&self, mut line: usize) -> usize {
        if line == 0 { return 0; }
        match &self.data {
            Storage::Chunks(chunks) => {
                let mut offset = 0;
                for chunk in chunks.iter() {
                    if line <= chunk.newlines.len() { return offset + chunk.newlines[line-1] + 1; }
                    line -= chunk.newlines.len(); offset += chunk.text.len();
                }
                offset
            }
            Storage::Gap(_) => {
                let mut offset = 0; let mut found = None;
                self.visit(|s| {
                    if found.is_none() {
                        for (i,b) in s.iter().enumerate() {
                            if *b == b'\n' { line -= 1; if line == 0 { found = Some(offset+i+1); break; } }
                        }
                    }
                    offset += s.len();
                });
                found.unwrap_or(offset)
            }
        }
    }
    fn line(&self, line: usize) -> String {
        let mut text = self.slice(self.line_start(line)..self.line_start(line+1));
        if text.ends_with('\n') { text.pop(); }
        if text.ends_with('\r') { text.pop(); }
        text
    }
}

#[derive(Clone)]
struct Edit { range: Range<usize>, text: String }
struct Transaction { forward: Vec<Edit>, inverse: Vec<Edit> }

#[derive(Default)]
struct Latest { next: Option<(u64, Text)> }
impl Latest {
    fn offer(&mut self, version: u64, text: Text) { self.next = Some((version, text)); }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BufferId(u64);
struct View { buffer: BufferId, selection: Range<usize> }
struct Owner {
    id: BufferId,
    path: PathBuf,
    text: Text,
    saved: String,
    version: u64,
    views: Vec<View>,
    cache: InactiveCache,
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
}
impl Owner {
    fn new(kind: Kind, text: &str) -> Self {
        Self { id: BufferId(1), path: "a.txt".into(), text: Text::new(kind,text), saved: text.into(), version: 1,
            views: vec![View { buffer: BufferId(1), selection: 0..0 }, View { buffer: BufferId(1), selection: 1..1 }],
            cache: InactiveCache::new(128), undo: Vec::new(), redo: Vec::new() }
    }
    fn edit(&mut self, edits: &[(Range<usize>, &str)]) {
        assert!(edits.windows(2).all(|w| w[0].0.end <= w[1].0.start));
        let forward: Vec<_> = edits.iter().map(|(range,text)| Edit { range: range.clone(), text: (*text).into() }).collect();
        let mut delta = 0isize;
        let inverse = forward.iter().map(|edit| {
            let start = edit.range.start.checked_add_signed(delta).unwrap();
            delta += edit.text.len() as isize - edit.range.len() as isize;
            Edit { range: start..start+edit.text.len(), text: self.text.slice(edit.range.clone()) }
        }).collect();
        self.apply(&forward);
        self.undo.push(Transaction { forward, inverse });
        self.redo.clear();
    }
    fn apply(&mut self, edits: &[Edit]) {
        for edit in edits.iter().rev() {
            self.text.replace(edit.range.clone(), &edit.text);
            let transform = |at: usize| if at < edit.range.start { at } else if at >= edit.range.end {
                at - edit.range.len() + edit.text.len()
            } else { edit.range.start + edit.text.len() };
            for view in &mut self.views { view.selection = transform(view.selection.start)..transform(view.selection.end); }
        }
        self.version += 1;
    }
    fn save(&mut self) { self.saved = self.text.string(); }
    fn undo(&mut self) { if let Some(t) = self.undo.pop() { self.apply(&t.inverse); self.redo.push(t); } }
    fn redo(&mut self) { if let Some(t) = self.redo.pop() { self.apply(&t.forward); self.undo.push(t); } }
    fn accept_worker(&self, version: u64) -> bool { version == self.version }
}

#[test]
fn shared_views_keep_one_draft() {
    for kind in [Kind::Gap, Kind::Chunks] {
        let mut owner = Owner::new(kind, "abc\ndef\n");
        let independent = Owner::new(kind, "abc\ndef\n");
        assert_eq!(owner.views[0].buffer, owner.views[1].buffer);
        assert_ne!(owner.views[0].selection, owner.views[1].selection);
        owner.edit(&[(0..0, "new\n")]);
        assert_eq!(owner.text.string(), "new\nabc\ndef\n");
        assert_eq!(owner.views[1].selection, 5..5);
        let id = owner.id;
        owner.path = "renamed.txt".into();
        assert_eq!(owner.views[0].buffer, id);
        assert_eq!(independent.text.string(), "abc\ndef\n");
    }
}

#[test]
fn saved_history_survives_cache_pressure() {
    for kind in [Kind::Gap, Kind::Chunks] {
        let mut owner = Owner::new(kind, "original\n");
        owner.cache = InactiveCache::new(16*1024);
        owner.edit(&[(0..8, "changed")]);
        owner.save();
        assert_eq!(owner.saved, "changed\n");
        let stamp = Stamp { len: 4096, modified: None, file: None };
        for n in 0..20 {
            owner.cache.put(stamp, Arc::new(reader_core::text::TextSnapshot::from_document(reader_core::text::BufferId(n),
                &Document::decode(format!("other-{n}").into(), n, &vec![b'x';4096]))));
            if n == 0 { assert!(owner.cache.peek(std::path::Path::new("other-0")).is_some(), "pressure fixture must actually enter the cache"); }
        }
        assert!(owner.cache.peek(std::path::Path::new("other-0")).is_none());
        assert!(owner.cache.peek(std::path::Path::new("other-19")).is_some());
        assert!(owner.cache.used() <= 16*1024);
        owner.undo();
        assert_eq!(owner.text.string(), "original\n");
        assert_eq!(owner.saved, "changed\n");
        owner.redo();
        assert_eq!(owner.text.string(), "changed\n");
    }
}

#[test]
fn old_worker_snapshot_cannot_replace_new_text() {
    for kind in [Kind::Gap, Kind::Chunks] {
        let mut owner = Owner::new(kind, "old\n");
        let old = owner.text.clone();
        let version = owner.version;
        owner.edit(&[(0..3, "new")]);
        assert!(!owner.accept_worker(version));
        assert!(owner.accept_worker(owner.version));
        assert_eq!(old.string(), "old\n");
        assert_eq!(owner.text.string(), "new\n");
    }
}

#[test]
fn unicode_multirange_undo_round_trips() {
    for kind in [Kind::Gap, Kind::Chunks] {
        let mut owner = Owner::new(kind, "α😀beta\r\nlast\n");
        owner.edit(&[(2..6, "🦀"), (12..16, "τέλος")]);
        assert_eq!(owner.text.string(), "α🦀beta\r\nτέλος\n");
        owner.edit(&[(0..0, "start\n")]);
        assert_eq!(owner.text.line(2), "τέλος");
        owner.undo(); owner.undo();
        assert_eq!(owner.text.string(), "α😀beta\r\nlast\n");
        owner.redo(); owner.redo();
        assert_eq!(owner.text.string(), "start\nα🦀beta\r\nτέλος\n");
    }
}

#[test]
fn stores_match_replacements_across_chunk_boundaries() {
    for kind in [Kind::Gap, Kind::Chunks] {
        let initial = "α😀abcdef\r\n".repeat(1500);
        let mut text = Text::new(kind, &initial);
        let mut oracle = initial;
        for (range, inserted) in [(4092..8200, "replacement\n"), (0..0, "front"), (5..5, "😀")] {
            // Select known UTF-8 boundaries independently of the candidate store.
            let start = (0..=range.start).rev().find(|&i| oracle.is_char_boundary(i)).unwrap();
            let end = (range.end..=oracle.len()).find(|&i| oracle.is_char_boundary(i)).unwrap();
            oracle.replace_range(start..end, inserted);
            text.replace(start..end, inserted);
            assert_eq!(text.string(), oracle);
        }
        for (n, line) in oracle.split('\n').enumerate() { assert_eq!(text.line(n), line.trim_end_matches('\r')); }
    }
}

#[test]
fn cancelled_parse_does_not_publish_a_snapshot() {
    use lenz_content::highlight::{self, Language, Want};
    let owner = Owner::new(Kind::Chunks, "fn main() {}\n");
    assert!(highlight::parse_with(Language::Rust, owner.text.string().into(), Want::default(), || false).is_none());
}

#[test]
fn latest_pending_snapshot_releases_superseded_versions() {
    let mut owner = Owner::new(Kind::Chunks, "old\n");
    let running_version = owner.version;
    let running = owner.text.clone();
    owner.edit(&[(0..0,"first")]);
    let Storage::Chunks(first) = &owner.text.data else { unreachable!() };
    let dropped = Arc::downgrade(first);
    let mut latest = Latest::default();
    latest.offer(owner.version, owner.text.clone());
    for _ in 0..100 { owner.edit(&[(0..0,"x")]); latest.offer(owner.version, owner.text.clone()); }
    assert!(dropped.upgrade().is_none(), "obsolete pending text must be released");
    let (version, snapshot) = latest.next.take().unwrap();
    assert_eq!(version, owner.version);
    assert_eq!(snapshot.string(), format!("{}firstold\n", "x".repeat(100)));
    assert!(latest.next.is_none());
    assert!(!owner.accept_worker(running_version));
    assert_eq!(running.string(), "old\n");
}

#[test]
fn a_parse_can_be_cancelled_after_work_has_started() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use lenz_content::highlight::{self, Language, Want};
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let text: Arc<str> = "fn item() { let value = 1 + 2; }\n".repeat(10000).into();
    let result = highlight::parse_with(Language::Rust, text, Want::default(), move || count.fetch_add(1, Ordering::SeqCst) < 2);
    assert!(result.is_none());
    assert!(calls.load(Ordering::SeqCst) >= 3);
}

fn rss() -> serde_json::Value {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let field = |name: &str| status.lines().find_map(|s| s.strip_prefix(name))
        .and_then(|s| s.split_whitespace().next()?.parse::<u64>().ok());
    serde_json::json!({"rss_kib":field("VmRSS:"),"peak_kib":field("VmHWM:")})
}

fn timed<T>(f: impl FnOnce() -> T) -> (T, u128) {
    let now = std::time::Instant::now();
    let result = f();
    (result, now.elapsed().as_nanos())
}

/// The ordinary renderer consumes Document::line; measure that handoff as well as store access.
fn handoff(text: &Text, rust: bool) -> serde_json::Value {
    use lenz_content::highlight::{self, Language, Want};
    use std::io::Write;
    let (flat, flatten_ns) = timed(|| text.string());
    let ((doc, parsed), analysis_ns) = timed(|| {
        let doc = Document::decode(if rust { "fixture.rs" } else { "fixture.txt" }.into(), 1, flat.as_bytes());
        let parsed = if rust {
            highlight::parse_with(Language::Rust, doc.text().clone(), Want { folds: true, declarations: true }, || true).is_some()
        } else { false };
        (doc, parsed)
    });
    let middle = if doc.line_count() < 60 { 0 } else { doc.line_count()/2 };
    let (_, viewport_ns) = timed(|| {
        for line in middle..(middle+60).min(doc.line_count()) { std::hint::black_box(doc.line(line)); }
    });
    let (shown_bytes, shown_ns) = timed(|| (middle..(middle+60).min(doc.line_count()))
        .map(|line| reader_core::shown::shown_line(doc.line(line), &[]).text.len()).sum::<usize>());
    struct Counter(usize);
    impl Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> { self.0 += bytes.len(); Ok(bytes.len()) }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    let (wire_bytes, lsp_ns) = timed(|| {
        let message = serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":"file:///fixture.rs","version":1},"contentChanges":[{"text":doc.text().as_ref()}]}});
        let mut count = Counter(0);
        reader_core::semantics::lsp::write_message(&mut count,&message).unwrap();
        count.0
    });
    assert!(wire_bytes > flat.len());
    serde_json::json!({"flatten_ns":flatten_ns,"document_and_parse_ns":analysis_ns,"parsed_rust":parsed,
        "document_viewport_ns":viewport_ns,"shown_viewport_ns":shown_ns,"shown_bytes":shown_bytes,"lsp_ns":lsp_ns,"wire_bytes":wire_bytes})
}

#[test]
#[ignore = "release-only readiness measurement; EDITOR_STORE=gap|chunks, EDITOR_EVIDENCE_DIR=<run directory>"]
fn editor_storage_measurements() {
    use std::{io::Write, time::{Duration, Instant}};
    use serde_json::json;
    let name = std::env::var("EDITOR_STORE").expect("select gap or chunks so RSS is measured in separate processes");
    let kind = match name.as_str() { "gap" => Kind::Gap, "chunks" => Kind::Chunks, _ => panic!("unknown store") };
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join(std::env::var_os("EDITOR_EVIDENCE_DIR").expect("evidence directory"));
    std::fs::create_dir_all(&out).unwrap();
    let mut log = std::fs::OpenOptions::new().write(true).create_new(true).open(out.join(format!("storage-{name}.jsonl"))).unwrap();
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/perf/editor-readiness/fixtures");
    let values = [("small-rust", "rust/module_0.rs"), ("large-text", "large.txt"), ("long-unicode", "long-unicode.txt")];
    let mut inputs = Vec::new();
    for (label, file) in values {
        let text = std::fs::read_to_string(fixtures.join(file)).unwrap();
        let owner = Owner::new(kind,&text);
        let initial = handoff(&owner.text,label == "small-rust");
        writeln!(log,"{}",json!({"case":label,"phase":"initial-handoff","bytes":text.len(),"metrics":initial,"memory":rss()})).unwrap();
        inputs.push((label,text,owner));
    }
    let seconds = std::env::var("EDITOR_STRESS_SECONDS").map_or(180, |s| s.parse::<u64>().unwrap());
    assert!(seconds >= 1);
    let started = Instant::now();
    let mut iteration = 0;
    while started.elapsed() < Duration::from_secs(seconds) {
        for (label, original, owner) in &mut inputs {
            assert!(owner.text.string() == *original);
            owner.text.copied = 0;
            owner.text.root_pointer_bytes = 0;
            owner.text.materializations.set(0);
            owner.text.materialized_bytes.set(0);
            let mut middle = original.len()/2;
            while !original.is_char_boundary(middle) { middle -= 1; }
            let (snapshot, snapshot_ns) = timed(|| owner.text.clone());
            let mut oracle = original.clone();
            let mut edit_ns = Vec::new();
            let operations = [(0..0,"begin\n"), (middle+6..middle+6,"α😀\npaste\n"), (oracle.len()+"begin\nα😀\npaste\n".len()..oracle.len()+"begin\nα😀\npaste\n".len(),"end\n")];
            for (range, inserted) in operations {
                let (_, ns) = timed(|| owner.edit(&[(range.clone(),inserted)]));
                edit_ns.push(ns);
                oracle.replace_range(range,inserted);
            }
            let last = oracle.len();
            let (_, multirange_ns) = timed(|| owner.edit(&[(0..0,"left"),(last..last,"right")]));
            oracle.push_str("right"); oracle.insert_str(0,"left");
            let (_, distant_ns) = timed(|| {
                for n in 0..16 { let at = if n%2==0 {0} else {owner.text.len()}; owner.edit(&[(at..at,"z")]); }
            });
            oracle.insert_str(0,"zzzzzzzz"); oracle.push_str("zzzzzzzz");
            assert!(owner.text.string() == oracle, "edit trace mismatch {label}");
            let lines = oracle.split('\n').count();
            let first = if *label == "long-unicode" { 0 } else { lines/2 };
            let (_, viewport_ns) = timed(|| {
                for line in first..(first+60).min(lines) { std::hint::black_box(owner.text.line(line)); }
            });
            let expected_lines: Vec<_> = oracle.split('\n').collect();
            if *label == "long-unicode" {
                let visible_bytes: usize = (first..(first+60).min(lines)).map(|line| owner.text.line(line).len()).sum();
                assert!(visible_bytes > original.len()/2, "long-line workload must visit the long Unicode content");
            }
            for (line, expected) in expected_lines.iter().enumerate().take((first+60).min(lines)).skip(first) {
                assert_eq!(owner.text.line(line), expected.trim_end_matches('\r'));
            }
            let (_, save_ns) = timed(|| owner.save());
            assert!(owner.saved == oracle);
            let mut undo_ns = 0;
            let (_, ns) = timed(|| for _ in 0..20 { owner.undo(); }); undo_ns += ns;
            assert!(owner.text.string() == *original);
            let (_, redo_ns) = timed(|| for _ in 0..20 { owner.redo(); });
            assert!(owner.text.string() == oracle);
            assert!(snapshot.string() == *original, "held worker snapshot changed");
            let analysis = (iteration % 8 == 0).then(|| handoff(&owner.text,*label == "small-rust"));
            let snapshot_copy_bytes = match kind { Kind::Gap => original.len()+1024, Kind::Chunks => 0 };
            writeln!(log,"{}",json!({"iteration":iteration,"case":label,"elapsed_seconds":started.elapsed().as_secs_f64(),
                "edit_ns":edit_ns,"multirange_ns":multirange_ns,"distant_edits_ns":distant_ns,"store_viewport_ns":viewport_ns,
                "snapshot_ns":snapshot_ns,"snapshot_copy_bytes":snapshot_copy_bytes,"save_materialize_ns":save_ns,
                "undo_ns":undo_ns,"redo_ns":redo_ns,"store_copied_bytes":owner.text.copied,
                "root_pointer_copy_bytes":owner.text.root_pointer_bytes,
                "full_materializations":owner.text.materializations.get(),"materialized_bytes":owner.text.materialized_bytes.get(),
                "handoff":analysis,"memory":rss()})).unwrap();
            for _ in 0..20 { owner.undo(); }
            assert!(owner.text.string() == *original);
        }
        iteration += 1;
    }
    let seconds = started.elapsed().as_secs_f64();
    writeln!(log,"{}",json!({"phase":"complete","iterations":iteration,"elapsed_seconds":seconds,"memory":rss()})).unwrap();
    println!("{name}: {iteration} full three-fixture traces in {seconds:.3}s; all text/snapshot/undo assertions passed");
}
