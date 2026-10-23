//! One bounded index chunk: current-buffer consumers and the remaining disk reads finish together.
use std::{path::PathBuf, sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}}};
use crate::{analysis::Ticket, index::FileIndex, session::Completion};

pub(crate) struct Item { pub path: Arc<str>, pub ticket: Option<Ticket>, pub result: Option<FileIndex> }
pub(crate) struct Batch {
    pub root: PathBuf,
    pub generation: u64,
    pub revision: u64,
    remaining: AtomicUsize,
    items: Mutex<Vec<Item>>,
    tx: async_channel::Sender<Completion>,
}
impl Batch {
    pub fn new(root: PathBuf, generation: u64, revision: u64, paths: Vec<Arc<str>>, tx: async_channel::Sender<Completion>) -> Arc<Self> {
        Arc::new(Self { root, generation, revision, remaining: AtomicUsize::new(1), items: Mutex::new(paths.into_iter().map(|path| Item { path, ticket: None, result: None }).collect()), tx })
    }
    pub fn part(self: &Arc<Self>, paths: Vec<Arc<str>>) -> Part {
        self.remaining.fetch_add(1, Ordering::Relaxed);
        Part { batch: self.clone(), paths }
    }
    pub fn seal(&self) { self.finish(); }
    fn finish(&self) {
        if self.remaining.fetch_sub(1, Ordering::AcqRel) == 1 { let _ = self.tx.try_send(Completion::AnalysisWake); }
    }
    pub fn take(&self) -> Option<Vec<Item>> {
        (self.remaining.load(Ordering::Acquire) == 0).then(|| std::mem::take(&mut *self.items.lock().unwrap_or_else(|e| e.into_inner())))
    }
}
pub(crate) struct Part { batch: Arc<Batch>, paths: Vec<Arc<str>> }
impl Part {
    pub fn record(&self, path: &str, ticket: Option<Ticket>, result: FileIndex) {
        if let Some(item) = self.batch.items.lock().unwrap_or_else(|e| e.into_inner()).iter_mut().find(|item| item.path.as_ref() == path) {
            item.ticket = ticket;
            item.result = Some(result);
        }
    }
    pub fn record_all(&self, ticket: Ticket, result: FileIndex) {
        for path in &self.paths { self.record(path, Some(ticket), result.clone()); }
    }
}
impl Drop for Part { fn drop(&mut self) { self.batch.finish(); } }
