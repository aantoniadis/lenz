use super::*;
use lenz_content::git::testing::{repo, write as put};
use std::sync::mpsc;

#[test]
fn session_drop_cancels_a_running_repository_status_reader_before_joining_workers() {
    let state = tempfile::tempdir().unwrap();
    let repository = repo();
    put(repository.path(), "changed.txt", "a path that reaches the cancellation callback\n");
    let s = session(state.path());
    let current = s.git_status_requests.clone();
    let request = current.load(Ordering::Relaxed);
    let root = repository.path().to_path_buf();
    let (started, entered) = mpsc::channel();
    let (finished, outcome) = mpsc::channel();
    s.jobs.submit(Priority::Visible, move || {
        started.send(()).unwrap();
        // Model an active reader at its next cancellation boundary. The fallback
        // lets the old, noncancelling Drop finish and fail instead of hanging.
        let until = std::time::Instant::now() + Duration::from_secs(3);
        while current.load(Ordering::Relaxed) == request && std::time::Instant::now() < until {
            std::thread::sleep(Duration::from_millis(1));
        }
        let result = checked_git_status(|| lenz_content::branches::status(
            &root, crate::branches::STATUS_LIMIT,
            &|| current.load(Ordering::Relaxed) != request,
        ));
        // status() returns only after killing and waiting for its Git child.
        finished.send(result).unwrap();
    });
    entered.recv_timeout(Duration::from_secs(5)).unwrap();

    drop(s);

    assert_eq!(outcome.recv_timeout(Duration::from_secs(5)).unwrap(), Err("superseded".into()),
        "the owner must cancel an active status read before joining its worker");
}
