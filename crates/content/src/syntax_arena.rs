//! Tree-sitter's memory for one parse, in a private mapping the parse returns whole.
//!
//! A 100k-line file becomes about two million tree nodes (~90 MiB) that tree-sitter frees only when the
//! tree is dropped. Left to the system allocator, that teardown stays charged to the process: glibc keeps
//! it in the worker's arena until `malloc_trim`, and libmalloc kept about half of it dirty after the first
//! teardown of concurrent parses (macOS 27, 2026-10-11) with no API to hand it back. Tree-sitter allocates
//! almost nothing it frees before the end of a parse, so a bump allocator costs no extra peak, and
//! `munmap` returns every page on every platform the moment the parse ends.
//!
//! The library-wide allocator hook dispatches on a thread-local scope: inside a parse, blocks come from the
//! mapping; outside, or once the mapping is full, from libc. Frees of mapping blocks are no-ops; the
//! mapping goes away with the scope, so every tree-sitter object a scope creates must be dropped before
//! the scope ends (parser, tree, query cursors). Persistent objects (compiled queries) are created with no
//! scope active and live in libc.
use std::{cell::Cell, ffi::c_void, ptr, sync::Once};

/// Block alignment libc guarantees and tree-sitter may assume.
const ALIGN: usize = 16;

#[derive(Clone, Copy)]
struct Mapping {
    base: *mut u8,
    capacity: usize,
    used: usize,
}

thread_local! {
    static CURRENT: Cell<Option<Mapping>> = const { Cell::new(None) };
}

/// Routes tree-sitter's allocations through this module. Idempotent; call before any other tree-sitter use.
#[allow(unsafe_code)] // the one allocator installation
pub(crate) fn install() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // SAFETY: called before the first tree-sitter object exists (every entry point calls `install`
        // first), exactly once, and the four functions form one allocator family: `free` and `realloc`
        // accept blocks from either the mapping or libc.
        unsafe {
            tree_sitter::set_allocator(Some(tree_sitter::Allocator { malloc: arena_malloc, calloc: arena_calloc, realloc: arena_realloc, free: arena_free }));
        }
    });
}

/// One parse's mapping, active on this thread until dropped. Declare it before the parser so it drops last.
pub(crate) struct Scope {
    mapping: Option<Mapping>,
}

impl Scope {
    /// Reserve address space for a parse of `text_len` bytes; pages are committed as they are touched.
    /// Larger trees spill to libc, so a small reservation is correct, only slower to release.
    pub(crate) fn begin(text_len: usize) -> Scope {
        let capacity = text_len.saturating_mul(64).clamp(8 << 20, 4 << 30);
        Scope { mapping: Mapping::reserve(capacity) }.activate()
    }

    fn activate(self) -> Scope {
        CURRENT.with(|c| {
            assert!(c.get().is_none(), "nested syntax arenas");
            c.set(self.mapping);
        });
        self
    }

    /// Bytes handed out so far, for tests and sizing.
    #[cfg(test)]
    pub(crate) fn used(&self) -> usize {
        CURRENT.with(|c| c.get().map_or(0, |m| m.used))
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|c| c.set(None));
        if let Some(mapping) = self.mapping.take() {
            mapping.release();
        }
    }
}

#[allow(unsafe_code)] // the mapping's FFI: anonymous mmap/munmap, and libc for blocks outside it
impl Mapping {
    fn reserve(capacity: usize) -> Option<Mapping> {
        #[cfg(target_os = "linux")]
        let flags = libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE;
        #[cfg(not(target_os = "linux"))]
        let flags = libc::MAP_PRIVATE | libc::MAP_ANON;
        // SAFETY: an anonymous private mapping with no file and no address hint; failure returns MAP_FAILED.
        let base = unsafe { libc::mmap(ptr::null_mut(), capacity, libc::PROT_READ | libc::PROT_WRITE, flags, -1, 0) };
        (base != libc::MAP_FAILED).then(|| Mapping { base: base.cast(), capacity, used: 0 })
    }

    fn release(self) {
        // SAFETY: `base..base+capacity` is exactly the mapping `reserve` created, and no pointer into it
        // is used afterwards: the scope's tree-sitter objects were dropped before the scope.
        unsafe { libc::munmap(self.base.cast(), self.capacity) };
    }

    fn contains(&self, p: *mut u8) -> bool {
        let start = self.base as usize;
        (start..start + self.capacity).contains(&(p as usize))
    }

    /// Bytes from `p` to the end of the mapping: the most a block starting at `p` can span.
    fn remaining_from(&self, p: *mut u8) -> usize {
        self.base as usize + self.capacity - p as usize
    }
}

/// A block from the current mapping, or `None` outside a scope or when the mapping is full.
#[allow(unsafe_code)] // pointer arithmetic inside the mapping
fn bump(size: usize) -> Option<*mut u8> {
    CURRENT.with(|c| {
        let mut m = c.get()?;
        // Zero-size requests still get their own address, as libc gives them.
        let rounded = size.max(1).checked_add(ALIGN - 1)? & !(ALIGN - 1);
        let end = m.used.checked_add(rounded)?;
        (end <= m.capacity).then(|| {
            // SAFETY: `used` stays within the mapping; the result is 16-aligned because the base is
            // page-aligned and every block size is rounded up to 16.
            let p = unsafe { m.base.add(m.used) };
            m.used = end;
            c.set(Some(m));
            p
        })
    })
}

fn in_current(p: *mut u8) -> Option<Mapping> {
    CURRENT.with(|c| c.get().filter(|m| m.contains(p)))
}

#[allow(unsafe_code)] // tree-sitter's C allocator hooks; each falls through to libc outside the mapping
mod hooks {
    use super::*;

    pub(super) unsafe extern "C" fn arena_malloc(size: usize) -> *mut c_void {
        // SAFETY: plain libc malloc when no mapping block is available.
        bump(size).map_or_else(|| unsafe { libc::malloc(size) }, |p| p.cast())
    }

    pub(super) unsafe extern "C" fn arena_calloc(count: usize, size: usize) -> *mut c_void {
        // Mapping pages are fresh and never reused, so they are already zero.
        // SAFETY: plain libc calloc when no mapping block is available.
        count.checked_mul(size).and_then(bump).map_or_else(|| unsafe { libc::calloc(count, size) }, |p| p.cast())
    }

    pub(super) unsafe extern "C" fn arena_realloc(p: *mut c_void, size: usize) -> *mut c_void {
        let old = p.cast::<u8>();
        let Some(mapping) = in_current(old) else {
            // SAFETY: `p` is null or a libc block (nothing else leaves the mapping).
            return unsafe { libc::realloc(p, size) };
        };
        // SAFETY: a fresh block (mapping or libc) of the requested size.
        let new = unsafe { arena_malloc(size) };
        if !new.is_null() {
            // The old block's size is unknown; copying up to `size` bytes, clamped to the mapping, reads
            // only our own mapping (bytes past the old block belong to later blocks and are merely read).
            let bytes = size.min(mapping.remaining_from(old));
            // SAFETY: both ranges lie in memory this process owns; `copy` tolerates overlap.
            unsafe { ptr::copy(old, new.cast::<u8>(), bytes) };
        }
        new
    }

    pub(super) unsafe extern "C" fn arena_free(p: *mut c_void) {
        if p.is_null() || in_current(p.cast()).is_some() {
            return; // the scope returns the whole mapping
        }
        // SAFETY: a libc block: tree-sitter frees only what its allocator family produced.
        unsafe { libc::free(p) }
    }
}
use hooks::{arena_calloc, arena_free, arena_malloc, arena_realloc};

#[cfg(test)]
#[allow(unsafe_code)] // exercises the C hooks directly
mod tests {
    use super::*;

    // Safe wrappers: each hook is sound for any size, and the pointers below come from the hooks.
    fn m(size: usize) -> *mut u8 {
        // SAFETY: the hook allocates from the mapping or libc.
        unsafe { arena_malloc(size) }.cast()
    }
    fn c(count: usize, size: usize) -> *mut u8 {
        // SAFETY: as `m`, zeroed.
        unsafe { arena_calloc(count, size) }.cast()
    }
    fn r(p: *mut u8, size: usize) -> *mut u8 {
        // SAFETY: `p` came from `m`, `c`, `r` or libc, and is not used again.
        unsafe { arena_realloc(p.cast(), size) }.cast()
    }
    fn f(p: *mut u8) {
        // SAFETY: `p` came from `m`, `c`, `r` or libc, and is freed once.
        unsafe { arena_free(p.cast()) }
    }
    fn bytes<'a>(p: *mut u8, n: usize) -> &'a [u8] {
        // SAFETY: the block at `p` was allocated with at least `n` bytes and is live.
        unsafe { std::slice::from_raw_parts(p, n) }
    }
    fn fill(p: *mut u8, data: &[u8]) {
        // SAFETY: the block at `p` holds at least `data.len()` bytes.
        unsafe { p.copy_from(data.as_ptr(), data.len()) }
    }

    #[test]
    fn blocks_are_aligned_zeroed_distinct_and_counted() {
        let scope = Scope::begin(0);
        let a = m(24);
        let b = c(3, 40);
        assert_eq!(a as usize % ALIGN, 0);
        assert_eq!(b as usize % ALIGN, 0);
        assert_eq!(b as usize - a as usize, 32, "24 bytes round up to 32");
        assert!(bytes(b, 120).iter().all(|&x| x == 0));
        assert_eq!(scope.used(), 32 + 128);
        f(a);
        f(b);
        assert_eq!(scope.used(), 160, "frees are no-ops inside a scope");
    }

    #[test]
    fn zero_size_blocks_are_distinct() {
        let _scope = Scope::begin(0);
        let a = m(0);
        let b = m(0);
        assert_ne!(a, b);
        assert!(!a.is_null() && in_current(a).is_some());
    }

    #[test]
    fn realloc_keeps_contents_inside_the_mapping_and_from_libc() {
        let _scope = Scope::begin(0);
        // SAFETY: plain libc allocation of 8 bytes, grown and freed through the hooks below.
        let outside = unsafe { libc::malloc(8) }.cast::<u8>();
        fill(outside, b"libc blk");
        let grown = r(outside, 64);
        assert_eq!(bytes(grown, 8), b"libc blk");
        f(grown);

        let inside = m(8);
        fill(inside, b"mapping!");
        let grown = r(inside, 1 << 20);
        assert_ne!(grown, inside);
        assert_eq!(bytes(grown, 8), b"mapping!");
        f(grown);
    }

    #[test]
    fn a_full_mapping_spills_to_libc_and_frees_cleanly() {
        let scope = Scope::begin(0); // 8 MiB
        let blocks: Vec<*mut u8> = (0..1200).map(|_| m(8192)).collect();
        assert_eq!(scope.used(), 8 << 20, "the mapping fills completely");
        let spilled = blocks.iter().filter(|&&p| in_current(p).is_none()).count();
        assert_eq!(spilled, 1200 - 1024);
        for p in blocks {
            f(p);
        }
    }

    #[test]
    fn without_a_scope_the_hooks_are_plain_libc() {
        assert!(CURRENT.with(|c| c.get().is_none()));
        let p = c(4, 4);
        assert!(in_current(p).is_none());
        let p = r(p, 64);
        f(p);
    }

    #[test]
    fn ending_a_scope_returns_touched_pages() {
        let before = crate::memory_probe::footprint_mib().unwrap();
        let scope = Scope::begin(4 << 20); // 256 MiB reserved
        let p = m(64 << 20);
        // SAFETY: the block holds 64 MiB.
        unsafe { ptr::write_bytes(p, 1, 64 << 20) };
        let touched = crate::memory_probe::footprint_mib().unwrap();
        assert!(touched - before > 60.0, "{before} -> {touched}");
        drop(scope);
        let after = crate::memory_probe::footprint_mib().unwrap();
        // Other tests allocate concurrently; the 64 MiB itself must be gone.
        assert!(after < touched - 56.0, "{before} -> {touched} -> {after}");
    }
}
