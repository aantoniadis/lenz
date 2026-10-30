//! Explorer clipboard and saved-byte copies. Adapters only ask for a name and present the shared result.
use super::*;
use std::sync::{Mutex, atomic::{AtomicBool, AtomicUsize}};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathKind { Name, Full, Relative }

/// Captures the project and destination while its name prompt is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasteEntry { source: PathBuf, root: PathBuf, dir: String }
impl PasteEntry {
    pub fn source(&self) -> &Path { &self.source }
    pub fn folder(&self) -> &str { &self.dir }
}

pub(super) struct CopyWork { id: u64, request: PasteEntry, name: String, cancel: Arc<AtomicBool> }
/// Identifies the latest clipboard request across this process's windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipboardTicket(u64);
#[derive(Default)]
pub(super) struct ClipboardService {
    latest: AtomicU64,
    cancel: Mutex<Option<Arc<AtomicBool>>>,
    pub(super) gate: Mutex<()>,
}
impl ClipboardService {
    pub(super) fn begin(&self, cancel: Option<Arc<AtomicBool>>) -> ClipboardTicket {
        let mut previous = self.cancel.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(previous) = previous.take() { previous.store(true, Ordering::Relaxed); }
        *previous = cancel;
        ClipboardTicket(self.latest.fetch_add(1, Ordering::Relaxed) + 1)
    }
    pub(super) fn current(&self, ticket: ClipboardTicket) -> bool { self.latest.load(Ordering::Relaxed) == ticket.0 }
    fn wait(&self, ticket: ClipboardTicket, cancel: Option<&AtomicBool>) -> Option<std::sync::MutexGuard<'_, ()>> {
        loop {
            if !self.current(ticket) || cancel.is_some_and(|c| c.load(Ordering::Relaxed)) { return None; }
            match self.gate.try_lock() {
                Ok(guard) => return Some(guard),
                Err(std::sync::TryLockError::Poisoned(e)) => return Some(e.into_inner()),
                Err(std::sync::TryLockError::WouldBlock) => std::thread::sleep(std::time::Duration::from_millis(5)),
            }
        }
    }
}
pub(super) struct ClipboardWork {
    sequence: u64,
    file: Option<(ClipboardTicket, Arc<AtomicBool>)>,
    pub(super) text: Option<ClipboardTicket>,
    pub(super) service: Arc<ClipboardService>,
    jobs: Arc<AtomicUsize>,
}
impl Default for ClipboardWork {
    fn default() -> Self {
        #[cfg(not(any(test, feature = "test-support")))]
        let service = {
            static SERVICE: std::sync::OnceLock<Arc<ClipboardService>> = std::sync::OnceLock::new();
            SERVICE.get_or_init(Arc::default).clone()
        };
        // Each test owns its fake desktop; multi-window tests explicitly share its service.
        #[cfg(any(test, feature = "test-support"))]
        let service = Arc::default();
        Self { sequence: 0, file: None, text: None, service, jobs: Arc::default() }
    }
}
struct ClipboardLease(Arc<AtomicUsize>);
impl ClipboardLease {
    fn new(jobs: &Arc<AtomicUsize>) -> Self { jobs.fetch_add(1, Ordering::Relaxed); Self(jobs.clone()) }
}
impl Drop for ClipboardLease {
    fn drop(&mut self) { self.0.fetch_sub(1, Ordering::Relaxed); }
}
impl ClipboardWork {
    pub(super) fn cancel(&mut self) {
        if let Some((_, cancel)) = &self.file { cancel.store(true, Ordering::Relaxed); }
    }
}

impl Session {
    #[cfg(any(test, feature = "test-support"))]
    pub fn disable_desktop_file_clipboard_for_test(&mut self) { self.skip_desktop_file_clipboard = true; }

    pub(super) fn explorer_path(&self) -> Option<PathBuf> {
        let project = self.state.project.as_ref()?;
        let selected = self.state.explorer.selected.as_deref().unwrap_or("");
        if !crate::create::is_relative(selected) || !(selected.is_empty() || project.has_file(selected) || project.has_dir(selected)) { return None; }
        Some(if selected.is_empty() { project.root().to_owned() } else { project.root().join(selected) })
    }

    pub(super) fn copy_explorer_path(&mut self, kind: PathKind) -> Effect {
        let Some(path) = self.explorer_path() else { return Effect::default() };
        let text = match kind {
            PathKind::Name => path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string()),
            PathKind::Full => path.display().to_string(),
            PathKind::Relative => self.state.explorer.selected.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| ".".into()),
        };
        Effect { repaint: true, copy: Some(text), ..Effect::default() }
    }

    pub(super) fn copy_explorer_entry(&mut self) -> Effect {
        if self.disabled(CommandId::CopyExplorerEntry).is_some() { return Effect::default(); }
        let Some(path) = self.explorer_path() else { return Effect::default() };
        self.explorer_clipboard = Some(path.clone());
        self.state.messages.push("File copied for tree Paste. Copies use saved disk contents; unsaved drafts stay in their tabs.".into());
        #[cfg(any(test, feature = "test-support"))]
        if self.skip_desktop_file_clipboard { return Effect { repaint: true, ..Effect::default() }; }
        if !self.jobs.has_workers() {
            self.state.messages.push("Tree Copy is ready. Desktop file clipboard needs a background worker; retry after reopening Lenz.".into());
            return Effect { repaint: true, ..Effect::default() };
        }
        self.file_clipboard.cancel();
        self.file_clipboard.sequence += 1;
        let request = self.file_clipboard.sequence;
        let cancel = Arc::new(AtomicBool::new(false));
        let service = self.file_clipboard.service.clone();
        let ticket = service.begin(Some(cancel.clone()));
        self.file_clipboard.file = Some((ticket, cancel.clone()));
        let tx = self.tx.clone();
        let lease = ClipboardLease::new(&self.file_clipboard.jobs);
        self.jobs.submit(Priority::Visible, move || {
            let result = match service.wait(ticket, Some(&cancel)) {
                Some(_guard) => lenz_content::file_clipboard::copy_file(&path, &cancel),
                None => Err("Obsolete clipboard request".into()),
            };
            drop(lease);
            let _ = tx.send_blocking(Completion::FileClipboard { request, result });
        });
        Effect { repaint: true, ..Effect::default() }
    }

    pub(super) fn file_clipboard_finished(&mut self, request: u64, result: Result<(), String>) -> Effect {
        if self.file_clipboard.sequence != request { return Effect::default(); }
        let Some((ticket, _)) = self.file_clipboard.file.take() else { return Effect::default() };
        if !self.file_clipboard.service.current(ticket) { return Effect::default(); }
        self.state.messages.push(match result {
            Ok(()) => "File copied to the tree and desktop clipboard. Paste in a destination folder; copies use saved disk contents.".into(),
            Err(why) => format!("Tree Copy is ready. Desktop file clipboard: {why}"),
        });
        Effect { repaint: true, ..Effect::default() }
    }

    /// Clipboard effects are consumed immediately; a ticket fences delayed delivery in either adapter.
    pub fn clipboard_text_current(&self, ticket: ClipboardTicket) -> bool { self.file_clipboard.service.current(ticket) }

    pub(super) fn prepare_clipboard_text(&mut self, effect: &mut Effect) {
        if effect.copy_ticket.is_some() { return; }
        let Some(text) = effect.copy.take() else { return };
        let service = self.file_clipboard.service.clone();
        let ticket = service.begin(None);
        let ready = match service.gate.try_lock() {
            Ok(_) | Err(std::sync::TryLockError::Poisoned(_)) => true,
            Err(std::sync::TryLockError::WouldBlock) => false,
        };
        if ready {
            self.file_clipboard.text = None;
            effect.copy = Some(text); effect.copy_ticket = Some(ticket);
        } else if self.jobs.has_workers() {
            self.file_clipboard.text = Some(ticket);
            let tx = self.tx.clone();
            let lease = ClipboardLease::new(&self.file_clipboard.jobs);
            self.jobs.submit(Priority::Visible, move || {
                drop(service.wait(ticket, None));
                drop(lease);
                let _ = tx.send_blocking(Completion::ClipboardTextReady { ticket, text });
            });
        } else {
            self.file_clipboard.text = None;
            self.state.messages.push("Clipboard is busy and no background worker is available; retry Copy.".into());
        }
    }

    pub(super) fn clipboard_text_ready(&mut self, ticket: ClipboardTicket, text: String) -> Effect {
        if self.file_clipboard.text != Some(ticket) { return Effect::default(); }
        if !self.clipboard_text_current(ticket) { self.file_clipboard.text = None; return Effect::default(); }
        Effect { repaint: true, copy: Some(text), copy_ticket: Some(ticket), ..Effect::default() }
    }

    pub(super) fn clipboard_text_published(&mut self, ticket: ClipboardTicket) -> Effect {
        if self.file_clipboard.text == Some(ticket) { self.file_clipboard.text = None; }
        Effect { repaint: true, ..Effect::default() }
    }

    pub(crate) fn clipboard_close_ready(&self) -> bool {
        self.file_clipboard.jobs.load(Ordering::Relaxed) == 0
            && self.file_clipboard.text.is_none_or(|ticket| !self.clipboard_text_current(ticket))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn share_clipboard_for_test(&mut self, other: &Session) { self.file_clipboard.service = other.file_clipboard.service.clone(); }

    pub fn paste_entry_request(&self) -> Result<PasteEntry, &'static str> {
        if let Some(why) = self.disabled(CommandId::PasteExplorerEntry) { return Err(why); }
        let project = self.state.project.as_ref().ok_or("No folder is open")?;
        Ok(PasteEntry { source: self.explorer_clipboard.clone().ok_or("Copy a file or folder first")?, root: project.root().to_owned(), dir: self.create_target().ok_or("No destination folder is selected")? })
    }

    /// Keep the original leaf name; the prompt allows another name when copying beside the original.
    pub fn paste_entry_start(&self, request: &PasteEntry) -> String {
        request.source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    pub fn paste_entry_refusal(&self, request: &PasteEntry, name: &str) -> Option<String> {
        if self.explorer_copy.is_some() { return Some("A file copy is running; wait or cancel it first".into()); }
        if !self.jobs.has_workers() { return Some("A background worker is unavailable; retry after reopening Lenz".into()); }
        if self.state.project.as_ref().is_none_or(|p| p.root() != request.root) { return Some("The destination project changed; reopen Paste".into()); }
        if let Some(why) = self.create_refusal(&request.dir, name) { return Some(why); }
        if let Some(why) = self.relocate_blocked(&request.source) { return Some(why); }
        let target = request.root.join(&request.dir).join(name);
        if target.starts_with(&request.source) { return Some("A folder cannot be copied into itself".into()); }
        match std::fs::symlink_metadata(&request.source) {
            Ok(m) if m.is_file() || m.is_dir() => None,
            Ok(_) => Some("Copy supports regular files and folders; symbolic links and special files are refused".into()),
            Err(e) => Some(format!("Cannot read the copied entry: {e}")),
        }
    }

    pub(super) fn paste_explorer_entry(&mut self, request: PasteEntry, name: String) -> Effect {
        let effect = Effect { repaint: true, ..Effect::default() };
        if let Some(why) = self.paste_entry_refusal(&request, &name) {
            self.state.messages.push(format!("Paste: {why}. Nothing was copied.")); return effect;
        }
        self.save_seq += 1;
        let id = self.save_seq;
        let cancel = Arc::new(AtomicBool::new(false));
        let target = request.root.join(&request.dir).join(&name);
        let (source, tx, stopped) = (request.source.clone(), self.tx.clone(), cancel.clone());
        self.explorer_copy = Some(CopyWork { id, request, name, cancel });
        self.state.messages.push(format!("Copying saved contents to {}… Cancel File Copy stops it before publication.", target.display()));
        self.jobs.submit(Priority::Background, move || {
            let result = crate::explorer_copy::copy(&source, &target, &stopped);
            let _ = tx.send_blocking(Completion::ExplorerCopied { request: id, result });
        });
        effect
    }

    pub(super) fn cancel_explorer_copy(&mut self) -> Effect {
        if let Some(work) = &self.explorer_copy { work.cancel.store(true, Ordering::Relaxed); }
        Effect { repaint: self.explorer_copy.is_some(), ..Effect::default() }
    }

    pub(super) fn explorer_copied(&mut self, request: u64, result: Result<(), String>) -> Effect {
        if self.explorer_copy.as_ref().is_none_or(|w| w.id != request) { return Effect::default(); }
        let Some(work) = self.explorer_copy.take() else { return Effect::default() };
        let target = work.request.root.join(&work.request.dir).join(&work.name);
        match result {
            Ok(()) => {
                if self.state.project.as_ref().is_some_and(|p| p.root() == work.request.root) {
                    let rel: Arc<str> = crate::create::relative(&work.request.dir, &work.name).into();
                    if let Some(project) = &mut self.state.project {
                        if target.is_dir() { project.add_folder(rel.clone()); } else { project.add_file(rel.clone()); }
                    }
                    self.state.tree_revision += 1;
                    self.show_created(rel);
                    self.rescan();
                }
                self.state.messages.push(format!("Copied saved contents to {}", target.display()));
            }
            Err(why) => self.state.messages.push(format!("Paste: {why}")),
        }
        Effect { repaint: true, ..Effect::default() }
    }
}
