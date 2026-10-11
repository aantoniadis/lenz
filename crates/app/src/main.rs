/// Freed blocks of 1 MiB and up leave the process at once instead of staying in libmalloc's cache.
#[cfg(target_os = "macos")]
#[global_allocator]
static ALLOCATOR: lenz::large_blocks::LargeBlocks = lenz::large_blocks::LargeBlocks;

fn main() -> std::process::ExitCode {
    lenz::gui::run()
}
