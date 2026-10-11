//! Explicit diagnostic probe, never linked into either application executable.
//! Run with LENZ_LOG_DIR pointing to a disposable directory.
fn main() {
    lenz::diagnostics::install("GUI");
    lenz::diagnostics::breadcrumb("Reload comparison");
    match std::env::args().nth(1).as_deref() {
        Some("worker") => {
            std::thread::Builder::new()
                .name("lenz-probe-worker".into())
                .spawn(|| panic!("original worker probe failure"))
                .unwrap()
                .join()
                .unwrap();
        }
        Some("error") => lenz::diagnostics::failure(
            "Read comparison",
            "Permission denied (os error 13)",
            "Check file permissions, then reload.",
        ),
        _ => panic!("original main probe failure"),
    }
}
