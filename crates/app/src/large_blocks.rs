//! A global allocator for macOS that maps blocks of 1 MiB and up directly, and unmaps them on free.
//!
//! libmalloc keeps freed large blocks in a cache that stays charged to the process (four freed
//! blocks totalling 104 MiB stayed in the footprint, 2026-10-11) until memory pressure; neither
//! `malloc_zone_pressure_relief` nor any runtime call releases it, only `MallocLargeCache=0` in the
//! environment at process start. A 100k-line comparison's freed working buffers (31 MiB) and every
//! closed 4K image (64 MiB each) stayed resident that way. glibc returns blocks above its mmap
//! threshold to the system immediately; this does the same, keeping at most `CACHE_BYTES` of freed
//! mappings for same-size reuse. Smaller blocks go to the system allocator unchanged. Binaries opt in
//! with `#[global_allocator]`.

use std::alloc::{GlobalAlloc, Layout, System};

/// Blocks this size and larger are mapped directly.
pub const THRESHOLD: usize = 1 << 20;

pub struct LargeBlocks;

/// mmap returns page-aligned memory: at least 4 KiB on every macOS page size.
const MAPPED_ALIGN: usize = 4096;

fn mapped(layout: Layout) -> bool {
    layout.size() >= THRESHOLD && layout.align() <= MAPPED_ALIGN
}

fn page() -> usize {
    // SAFETY: sysconf has no preconditions.
    #[allow(unsafe_code)]
    let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    usize::try_from(size).unwrap_or(16384)
}

fn pages(size: usize) -> usize {
    let page = page();
    size.div_ceil(page) * page
}

#[allow(unsafe_code)] // anonymous mappings; see the SAFETY comments
mod vm {
    pub(super) fn map(size: usize) -> *mut u8 {
        // SAFETY: a fresh anonymous private mapping with no address hint; failure returns MAP_FAILED.
        let p = unsafe {
            libc::mmap(std::ptr::null_mut(), size, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_PRIVATE | libc::MAP_ANON, -1, 0)
        };
        if p == libc::MAP_FAILED { std::ptr::null_mut() } else { p.cast() }
    }

    /// SAFETY: `ptr..ptr+size` lies within a mapping from `map` that nothing uses afterwards.
    pub(super) unsafe fn unmap(ptr: *mut u8, size: usize) {
        // SAFETY: the caller's contract.
        unsafe { libc::munmap(ptr.cast(), size) };
    }
}

/// Freed mappings kept for reuse by an allocation of the same page-rounded size: repeated
/// rebuilds of one large array (a 100k-line merge action) would otherwise fault fresh pages each
/// time. Bounded, unlike libmalloc's cache: at most `SLOTS` mappings and `CACHE_BYTES` in total.
pub const CACHE_BYTES: usize = 16 << 20;
const SLOTS: usize = 4;

mod cache {
    use super::{CACHE_BYTES, SLOTS};
    use std::{cell::UnsafeCell, hint::spin_loop, sync::atomic::{AtomicBool, Ordering}};

    /// (address, page-rounded bytes); 0 bytes is an empty slot.
    struct Cache { busy: AtomicBool, slots: UnsafeCell<[(usize, usize); SLOTS]> }
    // SAFETY: `slots` is only touched while `busy` is held (see `with`).
    #[allow(unsafe_code)]
    unsafe impl Sync for Cache {}
    static CACHE: Cache = Cache { busy: AtomicBool::new(false), slots: UnsafeCell::new([(0, 0); SLOTS]) };

    /// Run `f` on the slots under the lock. It must not allocate (it runs inside the allocator).
    fn with<R>(f: impl FnOnce(&mut [(usize, usize); SLOTS]) -> R) -> R {
        while CACHE.busy.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() { spin_loop() }
        // SAFETY: the lock above gives exclusive access until it is released below.
        #[allow(unsafe_code)]
        let r = f(unsafe { &mut *CACHE.slots.get() });
        CACHE.busy.store(false, Ordering::Release);
        r
    }

    /// A cached mapping of exactly `bytes`, removed from the cache.
    pub(super) fn take(bytes: usize) -> Option<*mut u8> {
        with(|slots| slots.iter_mut().find(|s| s.1 == bytes).map(|s| std::mem::take(s).0 as *mut u8))
    }

    /// Keep `ptr` (`bytes`, page-rounded) for reuse. Returns the mappings to unmap: `ptr` itself when it
    /// does not fit, otherwise any evicted to make room.
    pub(super) fn put(ptr: *mut u8, bytes: usize) -> [(usize, usize); SLOTS + 1] {
        let mut out = [(0, 0); SLOTS + 1];
        if bytes > CACHE_BYTES { out[0] = (ptr as usize, bytes); return out; }
        with(|slots| {
            let mut evicted = 0;
            // Oldest first: slot 0 is evicted before later slots.
            while slots.iter().map(|s| s.1).sum::<usize>() + bytes > CACHE_BYTES || slots.iter().all(|s| s.1 != 0) {
                let Some(victim) = slots.iter().position(|s| s.1 != 0) else { break };
                out[evicted] = std::mem::take(&mut slots[victim]);
                evicted += 1;
                slots[victim..].rotate_left(1);
            }
            match slots.iter().position(|s| s.1 == 0) {
                Some(free) => slots[free] = (ptr as usize, bytes),
                None => out[evicted] = (ptr as usize, bytes), // no room after all: give it back
            }
        });
        out
    }
}

// SAFETY: blocks of `THRESHOLD` bytes or more with up to page alignment come from `vm::map` (or the
// reuse cache, which holds only such mappings) and go back through the cache or `vm::unmap`; everything
// else uses `System`. Rust passes the same layout to `dealloc` and `realloc` as it allocated with, so
// `mapped` picks the same source each time, and every method upholds GlobalAlloc's contract per source.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for LargeBlocks {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !mapped(layout) {
            // SAFETY: the caller's GlobalAlloc contract (non-zero size), forwarded unchanged.
            return unsafe { System.alloc(layout) };
        }
        cache::take(pages(layout.size())).unwrap_or_else(|| vm::map(layout.size()))
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if !mapped(layout) {
            // SAFETY: as `alloc`.
            return unsafe { System.alloc_zeroed(layout) };
        }
        // Fresh anonymous pages are zero: nothing to write, nothing touched (so not from the cache).
        vm::map(layout.size())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if !mapped(layout) {
            // SAFETY: `ptr` came from `System` with this layout (see the impl's comment).
            return unsafe { System.dealloc(ptr, layout) };
        }
        for (addr, bytes) in cache::put(ptr, pages(layout.size())) {
            if bytes != 0 {
                // SAFETY: a whole mapping from `vm::map` that the cache gave up; nothing refers to it.
                unsafe { vm::unmap(addr as *mut u8, bytes) }
            }
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: GlobalAlloc::realloc guarantees `new_size`, rounded up to `layout.align()`, does not overflow.
        let new = unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) };
        match (mapped(layout), mapped(new)) {
            // SAFETY: `ptr` came from `System` with `layout`, and the new block stays there.
            (false, false) => unsafe { System.realloc(ptr, layout, new_size) },
            (true, true) if new_size <= layout.size() || pages(new_size) == pages(layout.size()) => {
                // Shrinking (or growing within the last page): keep the mapping, return the pages past the end.
                let (keep, had) = (pages(new_size), pages(layout.size()));
                if keep < had {
                    // SAFETY: the pages `keep..had` lie in this block's mapping and hold no live data.
                    unsafe { vm::unmap(ptr.add(keep), had - keep) }
                }
                ptr
            }
            _ => {
                // SAFETY: `new` has a non-zero size (realloc's contract).
                let moved = unsafe { self.alloc(new) };
                if !moved.is_null() {
                    // SAFETY: both blocks are live, distinct and at least `min(old, new)` bytes long; the
                    // old block is freed with the layout it was allocated with.
                    unsafe {
                        std::ptr::copy_nonoverlapping(ptr, moved, layout.size().min(new_size));
                        self.dealloc(ptr, layout);
                    }
                }
                moved
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Safe wrappers for the tests: each owns its block from allocation to free.
    struct Block { ptr: *mut u8, layout: Layout }
    impl Block {
        fn new(size: usize, align: usize, zeroed: bool) -> Block {
            let layout = Layout::from_size_align(size, align).unwrap();
            assert_ne!(size, 0);
            // SAFETY: a non-zero layout.
            #[allow(unsafe_code)]
            let ptr = unsafe { if zeroed { LargeBlocks.alloc_zeroed(layout) } else { LargeBlocks.alloc(layout) } };
            assert!(!ptr.is_null() && (ptr as usize).is_multiple_of(align));
            Block { ptr, layout }
        }
        fn bytes(&mut self) -> &mut [u8] {
            // SAFETY: the block is live and `layout.size()` bytes long; `&mut self` makes the slice unique.
            #[allow(unsafe_code)]
            unsafe { std::slice::from_raw_parts_mut(self.ptr, self.layout.size()) }
        }
        fn resize(&mut self, size: usize) {
            assert_ne!(size, 0);
            // SAFETY: the block came from LargeBlocks with `layout`; the size is non-zero.
            #[allow(unsafe_code)]
            let ptr = unsafe { LargeBlocks.realloc(self.ptr, self.layout, size) };
            assert!(!ptr.is_null());
            self.ptr = ptr;
            self.layout = Layout::from_size_align(size, self.layout.align()).unwrap();
        }
    }
    impl Drop for Block {
        fn drop(&mut self) {
            // SAFETY: allocated by LargeBlocks with `layout`, freed once.
            #[allow(unsafe_code)]
            unsafe { LargeBlocks.dealloc(self.ptr, self.layout) }
        }
    }

    fn footprint() -> f64 {
        crate::memory_probe::footprint_mib().unwrap()
    }

    fn pattern(block: &mut Block) {
        for (i, b) in block.bytes().iter_mut().enumerate() { *b = (i % 251) as u8 }
    }

    fn holds_pattern(block: &mut Block, len: usize) -> bool {
        block.bytes()[..len].iter().enumerate().all(|(i, &b)| b == (i % 251) as u8)
    }

    #[test]
    fn freed_large_blocks_leave_the_process() {
        let before = footprint();
        let mut block = Block::new(64 << 20, 8, false);
        block.bytes().fill(1);
        let held = footprint();
        drop(block);
        let after = footprint();
        assert!(held - before > 60.0, "{before} -> {held}");
        assert!(after < held - 56.0, "a freed 64 MiB block must not stay charged: {before} -> {held} -> {after}");
    }

    #[test]
    fn a_freed_block_is_reused_for_the_same_size_and_the_cache_is_bounded() {
        // Churn of one size (a 100k-line array rebuilt per merge action) must not fault fresh pages every time.
        let first = Block::new(3 << 20, 8, false);
        let address = first.ptr;
        drop(first);
        let again = Block::new(3 << 20, 8, false);
        assert_eq!(again.ptr, address, "the freed mapping is reused");
        drop(again);
        // Far more than the cache's bound, freed: at most CACHE_BYTES stay charged.
        let before = footprint();
        let blocks: Vec<Block> = (0..8).map(|_| { let mut b = Block::new(8 << 20, 8, false); b.bytes().fill(1); b }).collect();
        drop(blocks);
        let after = footprint();
        assert!(after - before <= (CACHE_BYTES >> 20) as f64 + 4.0, "{before} -> {after}");
    }

    #[test]
    fn realloc_keeps_contents_across_every_size_transition() {
        let mib = 1 << 20;
        for (from, to) in [(3 * mib, 9 * mib), (9 * mib, 3 * mib), (100, 2 * mib), (2 * mib, 100), (300, 900), (3 * mib, 3 * mib + 1), (THRESHOLD, THRESHOLD - 1)] {
            let mut block = Block::new(from, 8, false);
            pattern(&mut block);
            block.resize(to);
            assert!(holds_pattern(&mut block, from.min(to)), "contents lost {from} -> {to}");
            block.bytes().fill(7); // the grown tail is writable
        }
    }

    #[test]
    fn zeroed_large_blocks_are_zero_and_small_ones_too() {
        for size in [64, 4096, 2 << 20] {
            let mut block = Block::new(size, 16, true);
            assert!(block.bytes().iter().all(|&b| b == 0), "{size}");
        }
    }

    #[test]
    fn over_aligned_and_small_blocks_use_the_system_allocator() {
        for (size, align) in [(2 << 20, 1 << 16), (512, 8), (THRESHOLD - 1, 4096)] {
            let mut block = Block::new(size, align, false);
            pattern(&mut block);
            assert!(holds_pattern(&mut block, size));
        }
        assert!(!mapped(Layout::from_size_align(2 << 20, 1 << 16).unwrap()));
        assert!(mapped(Layout::from_size_align(THRESHOLD, 4096).unwrap()));
    }
}
