//! Paired release-only scan probe. Legacy is the matcher from main 6f7a186;
//! its byte-window iterator is retained here to compare identical inputs in one process.
use super::*;
use std::{hint::black_box, time::{Duration, Instant}};

#[allow(dead_code)]
#[path = "perf/legacy.rs"]
mod legacy;
use legacy::Matcher as Legacy;

fn legacy_matcher(query: &Query) -> Legacy {
    Legacy::new(&legacy::Query { text: query.text.clone(), options: legacy::FindOptions { case: query.options.case, word: query.options.word, regex: query.options.regex } }).unwrap()
}
fn legacy_collect(matcher: &Legacy, lines: &[String]) -> Matches<usize> {
    legacy::collect_matches(matcher, lines.iter().enumerate().map(|(i, line)| (i, line.as_str())), || true).unwrap()
}
fn collect(matcher: &Matcher, lines: &[String]) -> Matches<usize> {
    collect_matches(matcher, lines.iter().enumerate().map(|(i, line)| (i, line.as_str())), || true).unwrap()
}
fn rss() -> Option<f64> {
    std::fs::read_to_string("/proc/self/status").ok()?.lines().find_map(|line| line.strip_prefix("VmRSS:")?.split_whitespace().next()?.parse::<f64>().ok()).map(|n| n / 1024.)
}

#[test]
#[ignore = "normal-release paired Find scanner timing and sustained correctness probe"]
fn find_release_probe() {
    assert!(!black_box(cfg!(debug_assertions)), "run this probe with the normal release profile");
    let ordinary: Vec<String> = (0..100_000).map(|i| format!("let value_{i} = compute({i}); // ordinary ASCII source line")).collect();
    let unicode: Vec<String> = (0..100_000).map(|i| format!("let value_{i} = compute({i}); // café λ source line")).collect();
    let dense = vec!["foo ".repeat(MATCH_LIMIT + 1)];
    let long = vec!["a".repeat(4 << 20)];
    let cases = [
        ("ascii-absent", &ordinary, "not_present", FindOptions::default()),
        ("ascii-sensitive", &ordinary, "value_99999", FindOptions { case: true, ..Default::default() }),
        ("ascii-common-capped", &ordinary, "VALUE", FindOptions::default()),
        ("ascii-dense-capped", &dense, "foo", FindOptions { word: true, ..Default::default() }),
        ("repeated-prefix-absent", &long, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaab", FindOptions::default()),
        ("unicode-sensitive", &unicode, "café λ", FindOptions { case: true, ..Default::default() }),
        ("unicode-fold", &unicode, "CAFÉ", FindOptions::default()),
        ("regex", &ordinary, r"value_9\d{4}\b", FindOptions { regex: true, ..Default::default() }),
    ];
    let mut records = Vec::new();
    for (name, lines, text, options) in &cases {
        let query = Query { text: (*text).into(), options: *options };
        let old = legacy_matcher(&query); let new = Matcher::new(&query).unwrap();
        let expected = legacy_collect(&old, lines); assert_eq!(collect(&new, lines), expected);
        for pair in 0..7 {
            for candidate in if pair % 2 == 0 { [false, true] } else { [true, false] } {
                let start = Instant::now();
                let found = black_box(if candidate { collect(&new, black_box(lines)) } else { legacy_collect(&old, black_box(lines)) });
                let ms = start.elapsed().as_secs_f64() * 1000.;
                assert_eq!(found, expected);
                records.push(serde_json::json!({"case":name,"pair":pair,"candidate":candidate,"ms":ms,"hits":found.0.len(),"capped":found.1}));
            }
        }
    }
    let seconds: f64 = std::env::var("LENZ_FIND_STRESS_SECONDS").ok().and_then(|s| s.parse().ok()).unwrap_or(20.);
    assert!(seconds.is_finite() && seconds > 0.);
    let query = Query { text: "not_present".into(), ..Default::default() };
    let old = legacy_matcher(&query); let new = Matcher::new(&query).unwrap();
    let start = Instant::now(); let mut cycles = 0; let mut memory = Vec::new();
    while start.elapsed().as_secs_f64() < seconds {
        assert_eq!(black_box(collect(&new, black_box(&ordinary))), legacy_collect(&old, black_box(&ordinary)));
        cycles += 1;
        if cycles % 20 == 0 { memory.push(serde_json::json!({"seconds":start.elapsed().as_secs_f64(),"rss_mib":rss()})); }
    }
    eprintln!("FIND_PROBE {}", serde_json::json!({"workload_version":2,"baseline":"6f7a186 exact original matcher and collector","endpoint":"completed hot matcher scan including bounded collection; excludes UI/materialization and query construction","pairs":7,"samples":records,"stress_seconds":start.elapsed().as_secs_f64(),"stress_cycles":cycles,"memory":memory}));
    assert!(start.elapsed() >= Duration::from_secs_f64(seconds));
}
