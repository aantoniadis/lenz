//! Shared by benchmark/test targets only; no instrumentation in the shipping binary.
pub fn record(name: &str, unit: &str, samples: &[f64]) {
    assert!(!samples.is_empty() && samples.iter().all(|v| v.is_finite() && *v >= 0.0));
    if std::env::var_os("LENZ_PERF_JSON").is_some() {
        eprintln!("LENZ_PERF {{\"name\":{name:?},\"unit\":{unit:?},\"samples\":{samples:?}}}");
    }
}

pub fn check(ok: bool, message: &str) {
    if std::env::var_os("LENZ_PERF_REPORT_ONLY").is_none() { assert!(ok, "{message}"); }
}

/// Memory charged to this process in MiB: Linux VmRSS, macOS phys_footprint (not the resident size `ps`
/// shows, which keeps counting freed pages libmalloc marked reusable). Every crate including this module
/// provides `crate::memory_probe::footprint_mib` (the engine bench's returns `None`).
#[allow(dead_code)]
pub fn rss_mib() -> Option<f64> {
    super::memory_probe::footprint_mib()
}
