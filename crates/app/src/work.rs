//! Background document work. A superseded request must not load or diff an abandoned file.

use std::sync::{Weak, OnceLock, mpsc};

pub fn document<T: Send + 'static>(current: Weak<()>, job: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    // A disconnected receiver means the job panicked (in an unwinding build); the panic hook recorded it.
    submit(current, job).recv().ok().flatten()
}

/// Queue document work without blocking an event loop. Superseded jobs return `None`.
pub fn submit<T: Send + 'static>(current: Weak<()>, job: impl FnOnce() -> T + Send + 'static) -> mpsc::Receiver<Option<T>> {
    type Job = Box<dyn FnOnce() + Send>;
    static WORKER: OnceLock<mpsc::Sender<Job>> = OnceLock::new();
    let (tx, rx) = mpsc::sync_channel(1);
    if current.strong_count() == 0 { let _ = tx.send(None); return rx; }
    // Reuse this thread's allocation arena instead of leaving a large-file peak in every
    // executor thread. Background callers may wait on the receiver; event loops poll it.
    let worker = WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new().name("lenz-document".into()).spawn(move || {
            // One job's panic (in an unwinding build) must not take the worker, and every later document, with it.
            for job in rx { let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)); }
        }).expect("start document worker");
        tx
    });
    // A dead worker drops the job, and `tx` with it: the receiver reports a disconnection, not a second panic.
    let _ = worker.send(Box::new(move || {
        let result = (current.strong_count() > 0).then(job);
        let _ = tx.send(result);
    }));
    rx
}

/// Like [`submit`], but on its own short-lived thread: for jobs that only wait on another pool (syntax
/// parsing), so they never hold the document worker. Superseded jobs return `None`.
pub fn detached<T: Send + 'static>(current: Weak<()>, job: impl FnOnce() -> T + Send + 'static) -> mpsc::Receiver<Option<T>> {
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("lenz-syntax-wait".into())
        .spawn(move || {
            let result = (current.strong_count() > 0).then(job);
            let _ = tx.send(result);
        })
        .expect("start syntax waiter");
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn a_panicking_job_reports_nothing_and_the_worker_survives() {
        let current = Arc::new(());
        assert_eq!(document(Arc::downgrade(&current), || -> &str { panic!("worker job failure") }), None);
        assert_eq!(document(Arc::downgrade(&current), || "next document"), Some("next document"));
    }

    #[test]
    fn abandoned_work_is_skipped_and_the_next_document_still_loads() {
        let current = Arc::new(());
        let abandoned = Arc::downgrade(&current);
        drop(current);
        assert_eq!(document(abandoned, || "obsolete document"), None);
        let current = Arc::new(());
        assert_eq!(document(Arc::downgrade(&current), || "current document"), Some("current document"));
    }

    #[test]
    fn queued_work_cancelled_after_submission_cannot_publish_or_block_replacement() {
        use std::time::Duration;
        let owner = Arc::new(());
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let held = submit(Arc::downgrade(&owner), move || {
            started_tx.send(()).unwrap();
            // A failing assertion cannot permanently strand the shared worker.
            let _ = release_rx.recv_timeout(Duration::from_secs(15));
        });
        started_rx.recv_timeout(Duration::from_secs(15)).unwrap();
        let old = Arc::new(());
        let obsolete = submit(Arc::downgrade(&old), || "obsolete file");
        let replacement = submit(Arc::downgrade(&owner), || "replacement file");
        drop(old);
        release_tx.send(()).unwrap();
        assert_eq!(held.recv_timeout(Duration::from_secs(15)).unwrap(), Some(()));
        assert_eq!(obsolete.recv_timeout(Duration::from_secs(15)).unwrap(), None);
        assert_eq!(replacement.recv_timeout(Duration::from_secs(15)).unwrap(), Some("replacement file"));
    }
}
