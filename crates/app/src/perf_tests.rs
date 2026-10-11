//! Sustained model/worker stress on Linux and macOS, without a display or user files.
use std::{path::Path, sync::Arc, time::{Duration, Instant}};
use crate::{find, images, merge::{MergeResult, Pick}, moves::Edits, perf, ui::Doc, view::{ScrollMode, Side}, work};
use lenz_engine::{MergeOptions, Options, merge_lines, split_lines};

#[test]
#[ignore = "sustained workload: run via scripts/perf-suite.py"]
fn workload_stress() {
    let seconds: u64 = std::env::var("LENZ_STRESS_SECONDS").unwrap_or_else(|_| "10".into()).parse().unwrap();
    let base: Arc<str> = (0..100_000).map(|i| format!("let value_{i} = compute({i}); // café λ needle\n")).collect::<String>().into();
    let edited: Arc<str> = base.replace("compute(0)", "changed(0)").into();
    let lines: Vec<String> = base.lines().map(str::to_owned).collect();
    let old = image::RgbaImage::from_pixel(2048, 2048, image::Rgba([20, 80, 160, 255]));
    let new = image::RgbaImage::from_pixel(2048, 2048, image::Rgba([220, 100, 40, 255]));
    let (b, o, t) = (split_lines(&base), split_lines(&edited), split_lines(&base));
    let mut merge = MergeResult::new(&o, &b, &t, merge_lines(&o, &b, &t, &MergeOptions::default()));
    let opening = merge.lines().to_vec();
    let mut documents = Vec::new();
    let mut times = Vec::new();
    let mut rss = Vec::new();
    let mut memory_times = Vec::new();
    let mut undo_times = Vec::new();
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(seconds) || times.len() < 3 {
        let step = Instant::now();
        let token = Arc::new(());
        let (left, right) = (base.clone(), edited.clone());
        let doc = work::document(Arc::downgrade(&token), move || {
            let doc = Doc::new(&left, &right, Path::new("left.rs"), Path::new("right.rs"));
            let blocks = doc.line_diff(&Options::default());
            assert_eq!(blocks.len(), 1);
            let mut view = doc.view(&blocks, 60.0);
            for mode in [ScrollMode::Shared, ScrollMode::Aligned, ScrollMode::Padded] {
                view.set_mode(mode);
                for _ in 0..20 { view.scroll_by(1000.0, Side::Left); std::hint::black_box(view.frame()); }
            }
            Arc::new(doc)
        }).unwrap();
        documents.push(doc);
        // Fixed four-document working set; repeatedly evict, allocate and load on the real worker.
        if documents.len() > 4 { documents.remove(0); }
        let dead = Arc::downgrade(&token);
        drop(token);
        assert!(work::document(dead, || panic!("obsolete work ran")).is_none());
        assert_eq!(find::in_lines(&lines, "CAFÉ").len(), 100_000);
        assert_eq!(find::in_lines(&lines, "absent").len(), 0);
        let image = images::blend(&old, &new, 0.5);
        assert_eq!(*image.get_pixel(2000, 2000), image::Rgba([120, 90, 100, 255]));
        assert!(merge.accept(0, Pick::Theirs));
        assert!(merge.undo());
        assert_eq!(merge.lines(), opening);
        assert!(merge.redo());
        assert!(merge.undo());
        let undo_start = Instant::now();
        let mut edits = Edits::default();
        edits.record(Side::Right, &base, &edited);
        let (_, undone) = edits.next(false, [&base, &edited]).unwrap();
        assert_eq!(undone, &*base);
        edits.stepped(false, &edited, &undone);
        assert_eq!(edits.next(true, [&base, &undone]).unwrap().1, &*edited);
        undo_times.push(undo_start.elapsed().as_secs_f64() * 1000.0);
        times.push(step.elapsed().as_secs_f64() * 1000.0);
        if (times.len().is_multiple_of(10) || rss.is_empty()) && let Some(mib) = perf::rss_mib() {
            rss.push(mib);
            memory_times.push(started.elapsed().as_secs_f64());
        }
    }
    if let Some(mib) = perf::rss_mib() { rss.push(mib); memory_times.push(started.elapsed().as_secs_f64()); }
    perf::record("stress/mixed-cycle", "ms", &times);
    perf::record("stress/undo-redo", "ms", &undo_times);
    if !rss.is_empty() {
        perf::record("stress/rss-timeline", "MiB", &rss);
        perf::record("stress/rss-time", "s", &memory_times);
    }
    eprintln!("{} mixed cycles in {:.1}s; no rendering is included", times.len(), started.elapsed().as_secs_f64());
}
