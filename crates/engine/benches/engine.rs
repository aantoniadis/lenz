//! Engine performance guard on fixed inputs generated at bench time (nothing large is committed).
//!
//! `cargo bench -p lenz-engine --bench engine`                          report medians
//! `cargo bench -p lenz-engine --bench engine -- --check`               also exit 1 above 2× budget
//! `cargo bench -p lenz-engine --bench engine -- --write-fixtures DIR`  write the pairs for a manual run of `lenz`
//!
//! A plain harness, not criterion: the gate is one median against one number, criterion's statistics and
//! reports add nothing to that, and its dependency tree would multiply this crate's build time.
#[path = "../../../benches/support.rs"]
mod perf;
/// The shared bench module's memory probe; this bench measures time, not residency.
mod memory_probe {
    pub fn footprint_mib() -> Option<f64> {
        None
    }
}

use std::path::{Path, PathBuf};
use std::time::Instant;

use lenz_engine::{HighlightLevel, MergeOptions, Options, WhitespaceMode, compare, merge_lines, split_lines};

/// Budgets in ms for `--check`, which fails when a median exceeds twice its budget.
const BUDGETS: &[(&str, f64)] =
    &[("generated-100k Default/Word", 60.0), ("generated-100k Default/Line", 60.0), ("merge-100k Default", 110.0)];

const WHITESPACE: [WhitespaceMode; 4] =
    [WhitespaceMode::Default, WhitespaceMode::Trim, WhitespaceMode::Ignore, WhitespaceMode::IgnoreTrimChunks];
const HIGHLIGHT: [HighlightLevel; 3] = [HighlightLevel::Word, HighlightLevel::Char, HighlightLevel::Line];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).filter(|a| a != "--bench").collect();
    let generated = generate(100_000);
    let different = all_different(100_000);
    if let Some(i) = args.iter().position(|a| a == "--write-fixtures") {
        let dir = PathBuf::from(args.get(i + 1).expect("--write-fixtures needs a directory"));
        write_pair(&dir.join("generated-100k"), &generated);
        write_pair(&dir.join("different-100k"), &different);
        println!("wrote {0}/generated-100k and {0}/different-100k", dir.display());
        return;
    }

    let quick = args.iter().any(|a| a == "--quick");
    let mut medians = Vec::new();
    let mut run = |name: String, iters: usize, f: &mut dyn FnMut() -> usize| {
        let iters = if quick { 3 } else { iters };
        if iters > 1 {
            f(); // warm-up
        }
        let mut times: Vec<f64> = (0..iters)
            .map(|_| {
                let t = Instant::now();
                std::hint::black_box(f());
                t.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        perf::record(&name, "ms", &times);
        times.sort_by(f64::total_cmp);
        let median = times[times.len() / 2];
        println!("{name:<38} median {median:>8.1} ms  min {:>8.1} ms  ({iters} runs)", times[0]);
        medians.push((name, median));
    };

    let (l, r) = (split_lines(&generated.0), split_lines(&generated.1));
    for whitespace in WHITESPACE {
        for highlight in HIGHLIGHT {
            let options = Options { whitespace, highlight, ..Options::default() };
            run(format!("generated-100k {whitespace:?}/{highlight:?}"), 11, &mut || compare(&l, &r, &options).len());
        }
    }
    // Default whitespace only, few runs: Char refinement of the one 100k-line block takes seconds
    let (l, r) = (split_lines(&different.0), split_lines(&different.1));
    for highlight in HIGHLIGHT {
        if quick && highlight == HighlightLevel::Char { continue; }
        let options = Options { highlight, ..Options::default() };
        let iters = if highlight == HighlightLevel::Char { 1 } else { 3 };
        run(format!("different-100k Default/{highlight:?}"), iters, &mut || compare(&l, &r, &options).len());
    }

    match tokio_change_set() {
        Ok(files) => {
            let files: Vec<_> = files.iter().map(|(l, r)| (split_lines(l), split_lines(r))).collect();
            for highlight in HIGHLIGHT {
                let options = Options { highlight, ..Options::default() };
                let name = format!("tokio {} files Default/{highlight:?}", files.len());
                run(name, 11, &mut || files.iter().map(|(l, r)| compare(l, r, &options).len()).sum());
            }
        }
        Err(why) => println!("tokio 1.52.1 -> 1.53.1 skipped: {why}"),
    }

    // 3-way: Ours = the generated edits, Theirs = other edits to every 97th line (some land on Ours' and conflict);
    // `merge_lines` also test-resolves every Conflict for the magic wand
    let theirs: String = generated
        .0
        .lines()
        .enumerate()
        .map(|(i, l)| if i % 97 == 0 { format!("{l} // theirs\n") } else { format!("{l}\n") })
        .collect();
    let (b, o, t) = (split_lines(&generated.0), split_lines(&generated.1), split_lines(&theirs));
    for whitespace in [WhitespaceMode::Default, WhitespaceMode::Ignore] {
        let options = MergeOptions { whitespace };
        run(format!("merge-100k {whitespace:?}"), 11, &mut || merge_lines(&o, &b, &t, &options).len());
    }

    let mut big = String::new();
    while big.len() < 50 << 20 {
        big.push_str(&generated.0);
        big.push('\n');
    }
    run("split_lines 50 MB".into(), 11, &mut || split_lines(&big).len());

    if args.iter().any(|a| a == "--check") {
        let mut failed = false;
        for &(name, budget) in BUDGETS {
            let median = medians.iter().find(|(n, _)| n == name).expect("budgeted case was not run").1;
            let ok = median <= 2.0 * budget;
            failed |= !ok;
            println!("{} {name}: median {median:.1} ms, budget {budget} ms, limit {} ms", if ok { "ok  " } else { "FAIL" }, 2.0 * budget);
        }
        perf::check(!failed, "engine performance budget exceeded");
    }
}

/// Rust-like code with scattered modifications, insertions and deletions: the prototype's 100k-line pair
/// (same seed and generator, so numbers compare with the spec's measurements).
fn generate(n: usize) -> (String, String) {
    let mut seed: u64 = 0x5eed;
    let mut rnd = move |m: u64| {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) % m
    };
    let mut left = Vec::with_capacity(n);
    let mut f = 0;
    while left.len() < n {
        f += 1;
        left.push(format!("/// Computes the value for item {f}."));
        left.push(format!("pub fn compute_{f}(input: &Input, limit: usize) -> Result<u64> {{"));
        left.push(format!("    let mut total = {};", f * 7));
        left.push("    for item in input.items.iter().take(limit) {".into());
        left.push(format!("        if item.weight > {} {{", f % 13));
        left.push("            total += item.weight * item.count;".into());
        left.push("        } else {".into());
        left.push(format!("            total -= helper_{}(item, \"skip\")?;", f % 17));
        left.push("        }".into());
        left.push("    }".into());
        left.push("    Ok(total)".into());
        left.push("}".into());
        left.push(String::new());
    }
    left.truncate(n);
    let mut right = Vec::with_capacity(n);
    let mut i = 0;
    while i < left.len() {
        let gap = 30 + rnd(60) as usize;
        let end = (i + gap).min(left.len());
        right.extend_from_slice(&left[i..end]);
        i = end;
        if i >= left.len() {
            break;
        }
        match rnd(4) {
            0 => {
                // modify 1-3 lines: rename / tweak words
                for _ in 0..1 + rnd(3) {
                    if i < left.len() {
                        right.push(left[i].replace("total", "sum").replace("item", "entry").replace("> ", ">= "));
                        i += 1;
                    }
                }
            }
            1 => {
                for k in 0..1 + rnd(6) {
                    right.push(format!("    log::debug!(\"checkpoint {k} reached\");"));
                }
            }
            2 => i = (i + 1 + rnd(6) as usize).min(left.len()),
            _ => {
                i = (i + 2 + rnd(3) as usize).min(left.len());
                for k in 0..1 + rnd(5) {
                    right.push(format!("    let tmp_{k} = cache.get(&key_{k}).copied().unwrap_or_default();"));
                }
            }
        }
    }
    (left.join("\n"), right.join("\n"))
}

/// No line in common: one 100k-line modified block, the worst case for line diff and refinement.
fn all_different(n: usize) -> (String, String) {
    let side = |p: &str| (0..n).map(|i| format!("{p}{i} = {p}_value({i});")).collect::<Vec<_>>().join("\n");
    (side("left"), side("right"))
}

fn write_pair(dir: &Path, (l, r): &(String, String)) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("left.rs"), l).unwrap();
    std::fs::write(dir.join("right.rs"), r).unwrap();
}

/// tokio 1.52.1 -> 1.53.1 from the local cargo registry: every changed text file, an added or removed file
/// against empty text.
fn tokio_change_set() -> Result<Vec<(String, String)>, String> {
    let home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".cargo")))
        .ok_or("no CARGO_HOME or HOME")?;
    let src = home.join("registry/src");
    let find = |name: &str| {
        std::fs::read_dir(&src).ok()?.flatten().map(|d| d.path().join(name)).find(|p| p.is_dir())
    };
    let (Some(old), Some(new)) = (find("tokio-1.52.1"), find("tokio-1.53.1")) else {
        return Err(format!("tokio-1.52.1 and tokio-1.53.1 not both in {}", src.display()));
    };
    let (mut paths, mut new_paths) = (Vec::new(), Vec::new());
    walk(&old, &old, &mut paths);
    walk(&new, &new, &mut new_paths);
    paths.extend(new_paths);
    paths.sort();
    paths.dedup();
    let read = |root: &Path, p: &Path| std::fs::read(root.join(p)).map(|b| String::from_utf8(b).ok()).unwrap_or(Some(String::new()));
    Ok(paths
        .iter()
        .filter_map(|p| Some((read(&old, p)?, read(&new, p)?)))
        .filter(|(l, r)| l != r)
        .collect())
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, out);
        } else {
            out.push(path.strip_prefix(root).unwrap().to_path_buf());
        }
    }
}
