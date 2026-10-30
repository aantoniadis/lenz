//! Process memory for benchmark and test probes: the memory the kernel still charges to the process.
//!
//! Linux reports `VmRSS`. macOS reports `phys_footprint` (Activity Monitor's "Memory") rather than the
//! resident size `ps` shows: libmalloc marks freed pages reusable instead of returning them, and the
//! resident size keeps counting those until the kernel reclaims them, which it may never do. Only test
//! and benchmark targets compile this module (`perf-probe`); the shipping binaries do not.

/// Memory charged to this process, in MiB, or `None` where unsupported.
pub fn footprint_mib() -> Option<f64> {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let kib: f64 = status.lines().find_map(|l| l.strip_prefix("VmRSS:"))?.split_whitespace().next()?.parse().ok()?;
        Some(kib / 1024.0)
    }
    #[cfg(target_os = "macos")]
    {
        macos::phys_footprint().map(|bytes| bytes as f64 / (1024.0 * 1024.0))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)] // one audited libproc query about this process; no pointer outlives the call
mod macos {
    pub fn phys_footprint() -> Option<u64> {
        // SAFETY: rusage_info_v4 is plain integer fields and a byte array; all-zero is a valid value.
        let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
        // SAFETY: `info` is a correctly sized, writable rusage_info_v4 that the flavor RUSAGE_INFO_V4 selects.
        let rc = unsafe {
            libc::proc_pid_rusage(std::process::id() as libc::c_int, libc::RUSAGE_INFO_V4, (&raw mut info).cast::<libc::rusage_info_t>())
        };
        (rc == 0).then_some(info.ri_phys_footprint)
    }
}

#[cfg(test)]
mod tests {
    use super::footprint_mib;

    #[test]
    #[ignore = "process-wide memory: other tests skew it; scripts/ci-local.sh runs it alone"]
    fn footprint_counts_live_heap_not_freed_pages() {
        let before = footprint_mib().expect("footprint on this platform");
        #[cfg(not(target_os = "linux"))]
        let blocks: Vec<Box<[u8; 96]>> = (0..500_000).map(|i| Box::new([i as u8; 96])).collect();
        // VmRSS includes glibc's retained small-block arenas. A large touched allocation is returned on free;
        // expecting every small free to reduce RSS confuses allocator retention with live ownership.
        #[cfg(target_os = "linux")]
        let blocks = vec![1u8; 64 << 20];
        std::hint::black_box(&blocks);
        let held = footprint_mib().unwrap();
        assert!(held - before > 40.0, "touched live allocation should show: {before} -> {held} MiB");
        drop(blocks);
        let after = footprint_mib().unwrap();
        // Freed pages leave the charge (even where the resident size keeps counting them); what remains
        // is allocator retention, which a fresh process cannot bound exactly.
        assert!(after < held - 8.0, "frees must reduce the charge: {before} -> {held} -> {after} MiB");
        let resident = crate::perf::rss_mib().unwrap();
        assert!(after <= resident + 1.0, "charged memory cannot exceed resident pages: {after} vs {resident} MiB");
    }
}
