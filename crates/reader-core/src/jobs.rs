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
    stop: bool,
}

pub struct Jobs {
    shared: Arc<(Mutex<Queue>, Condvar)>,
    workers: Vec<std::thread::JoinHandle<()>>,
}

impl Jobs {
    pub fn new() -> Self {
        let shared: Arc<(Mutex<Queue>, Condvar)> = Arc::default();
        let workers = (0..MAX_JOBS)
            .filter_map(|i| {
                let shared = shared.clone();
                std::thread::Builder::new().name(format!("reader-job-{i}")).spawn(move || work(&shared)).ok()
            })
            .collect();
        Self { shared, workers }
    }

    pub fn submit(&self, priority: Priority, job: impl FnOnce() + Send + 'static) {
        self.submit_promotable(priority, job);
    }

    pub(crate) fn has_workers(&self) -> bool { !self.workers.is_empty() }

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
                if let Some(job) = q.visible.pop_front().or_else(|| q.background.pop_front().map(|(_, job)| job)) {
                    break job;
                }
                q = wake.wait(q).unwrap_or_else(|e| e.into_inner());
            }
        };
        // A panicking job loses its result, not the worker.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
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
    fn a_panicking_job_keeps_its_worker() {
        let jobs = Jobs::new();
        let (tx, rx) = std::sync::mpsc::channel();
        for _ in 0..MAX_JOBS { jobs.submit(Priority::Visible, || panic!("job failed")); }
        jobs.submit(Priority::Visible, move || tx.send(()).unwrap());
        rx.recv_timeout(std::time::Duration::from_secs(5)).expect("a worker survived");
    }
}
