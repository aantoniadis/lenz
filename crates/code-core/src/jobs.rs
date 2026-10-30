//! The reader's CPU-heavy work, at most [`MAX_JOBS`] at a time. Loads (what the user waits to see) run before
//! syntax work. Callers make each job check whether it is still wanted.

use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex},
};

/// Jobs that may run at once.
pub const MAX_JOBS: usize = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Priority {
    /// Content the user is waiting to see.
    Visible,
    /// Decoration of content already shown.
    Background,
}

type Job = Box<dyn FnOnce() + Send>;

#[derive(Default)]
struct Queue {
    visible: VecDeque<Job>,
    background: VecDeque<(u64, Job)>,
    next: u64,
    active: usize,
    worker_count: usize,
    visible_reservations: usize,
    stop: bool,
}

/// Keeps executor capacity for a visible dependency until its consumer has been submitted.
pub(crate) struct VisibleReservation { shared: Arc<(Mutex<Queue>, Condvar)> }
impl Drop for VisibleReservation {
    fn drop(&mut self) {
        let (queue, wake) = &*self.shared;
        queue.lock().unwrap_or_else(|e| e.into_inner()).visible_reservations -= 1;
        wake.notify_all();
    }
}

pub struct Jobs {
    shared: Arc<(Mutex<Queue>, Condvar)>,
    workers: Vec<std::thread::JoinHandle<()>>,
}

impl Jobs {
    #[cfg(test)]
    pub(crate) fn without_workers_for_test() -> Self { Self { shared: Arc::default(), workers: Vec::new() } }

    pub fn new() -> Self {
        let shared: Arc<(Mutex<Queue>, Condvar)> = Arc::default();
        let workers: Vec<_> = (0..MAX_JOBS)
            .filter_map(|i| {
                let shared = shared.clone();
                std::thread::Builder::new().name(format!("reader-job-{i}")).spawn(move || work(&shared)).ok()
            })
            .collect();
        shared.0.lock().unwrap_or_else(|e| e.into_inner()).worker_count = workers.len();
        Self { shared, workers }
    }

    pub fn submit(&self, priority: Priority, job: impl FnOnce() + Send + 'static) {
        self.submit_promotable(priority, job);
    }

    #[cfg(test)]
    pub(crate) fn queued(&self) -> usize {
        let queue = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
        queue.visible.len() + queue.background.len()
    }

    pub(crate) fn has_workers(&self) -> bool { !self.workers.is_empty() }

    pub(crate) fn reserve_visible(&self) -> VisibleReservation {
        let (queue, wake) = &*self.shared;
        queue.lock().unwrap_or_else(|e| e.into_inner()).visible_reservations += 1;
        wake.notify_all();
        VisibleReservation { shared: self.shared.clone() }
    }

    /// A narrow executor handle: policy and cancellation remain with the submitting owner.
    pub(crate) fn submit_promotable(&self, priority: Priority, job: impl FnOnce() + Send + 'static) -> Option<u64> {
        if self.workers.is_empty() { job(); return None; }
        let (queue, wake) = &*self.shared;
        let mut q = queue.lock().unwrap_or_else(|e| e.into_inner());
        q.next += 1;
        let id = q.next;
        match priority {
            Priority::Visible => q.visible.push_back(Box::new(job)),
            Priority::Background => q.background.push_back((id, Box::new(job))),
        }
        wake.notify_one();
        Some(id)
    }

    pub(crate) fn promote(&self, id: u64) {
        let (queue, wake) = &*self.shared;
        let mut q = queue.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(at) = q.background.iter().position(|(queued, _)| *queued == id)
            && let Some((_, job)) = q.background.remove(at)
        { q.visible.push_back(job); wake.notify_one(); }
    }

}

impl Default for Jobs {
    fn default() -> Self {
        Self::new()
    }
}

fn work(shared: &(Mutex<Queue>, Condvar)) {
    let (queue, wake) = shared;
    loop {
        let job = {
            let mut q = queue.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if q.stop {
                    return;
                }
                let job = q.visible.pop_front().or_else(|| {
                    (q.visible_reservations == 0 || q.active + 1 < q.worker_count)
                        .then(|| q.background.pop_front().map(|(_, job)| job)).flatten()
                });
                if let Some(job) = job { q.active += 1; break job; }
                q = wake.wait(q).unwrap_or_else(|e| e.into_inner());
            }
        };
        // A panicking job loses its result, not the worker.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
        queue.lock().unwrap_or_else(|e| e.into_inner()).active -= 1;
        wake.notify_all();
    }
}

impl Drop for Jobs {
    fn drop(&mut self) {
        let (queue, wake) = &*self.shared;
        queue.lock().unwrap_or_else(|e| e.into_inner()).stop = true;
        wake.notify_all();
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn at_most_three_run_at_once_and_visible_work_goes_first() {
        let jobs = Jobs::new();
        let (running, peak) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let order = Arc::new(Mutex::new(Vec::new()));
        // Occupy every worker, each until its own release.
        let releases: Vec<_> = (0..MAX_JOBS)
            .map(|_| {
                let (release, wait) = std::sync::mpsc::channel::<()>();
                let (running, peak) = (running.clone(), peak.clone());
                jobs.submit(Priority::Visible, move || {
                    peak.fetch_max(running.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
                    let _ = wait.recv();
                    running.fetch_sub(1, Ordering::SeqCst);
                });
                release
            })
            .collect();
        while running.load(Ordering::SeqCst) < MAX_JOBS { std::thread::yield_now(); }
        // Background work queued before visible work.
        for (p, name) in [(Priority::Background, "syntax"), (Priority::Visible, "load")] {
            let (order, running, peak) = (order.clone(), running.clone(), peak.clone());
            jobs.submit(p, move || {
                peak.fetch_max(running.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
                order.lock().unwrap().push(name);
                running.fetch_sub(1, Ordering::SeqCst);
            });
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(order.lock().unwrap().is_empty(), "a fourth job waits for a free worker");
        // One free worker takes the queue in priority order.
        releases[0].send(()).unwrap();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while order.lock().unwrap().len() < 2 && std::time::Instant::now() < until { std::thread::yield_now(); }
        assert_eq!(*order.lock().unwrap(), ["load", "syntax"]);
        for r in &releases[1..] { r.send(()).unwrap(); }
        drop(jobs);
        assert_eq!(peak.load(Ordering::SeqCst), MAX_JOBS);
    }

    #[test]
    fn a_temporary_visible_reservation_restores_all_background_workers_even_after_a_panic() {
        use std::{sync::mpsc, time::Duration};
        let jobs = Jobs::new();
        let (entered, started) = mpsc::channel();
        let gates = |jobs: &Jobs| (0..MAX_JOBS).map(|_| {
            let (release, wait) = mpsc::channel(); let entered = entered.clone();
            jobs.submit(Priority::Background, move || { entered.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); });
            release
        }).collect::<Vec<_>>();
        let before = gates(&jobs);
        for _ in &before { started.recv_timeout(Duration::from_secs(2)).unwrap(); }
        for gate in before { gate.send(()).unwrap(); }
        let reservation = jobs.reserve_visible();
        let during = gates(&jobs);
        for _ in 0..MAX_JOBS - 1 { started.recv_timeout(Duration::from_secs(2)).unwrap(); }
        assert!(started.recv_timeout(Duration::from_millis(30)).is_err(), "background work must leave capacity for the next visible stage");
        jobs.submit(Priority::Visible, || panic!("visible stage failed"));
        let (sent, received) = mpsc::channel();
        jobs.submit(Priority::Visible, move || sent.send(()).unwrap());
        received.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(started.try_recv().is_err(), "panic cleanup must keep the live reservation");
        drop(reservation);
        started.recv_timeout(Duration::from_secs(2)).unwrap();
        for gate in during { gate.send(()).unwrap(); }
        let after = gates(&jobs);
        for _ in &after { started.recv_timeout(Duration::from_secs(2)).unwrap(); }
        for gate in after { gate.send(()).unwrap(); }
    }

    #[test]
    fn a_panicking_job_keeps_its_worker() {
        let jobs = Jobs::new();
        let (tx, rx) = std::sync::mpsc::channel();
        for _ in 0..MAX_JOBS { jobs.submit(Priority::Visible, || panic!("job failed")); }
        jobs.submit(Priority::Visible, move || tx.send(()).unwrap());
        rx.recv_timeout(std::time::Duration::from_secs(5)).expect("a worker survived");
    }
}
