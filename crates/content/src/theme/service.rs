use super::{
    controller::ThemeController,
    store::{read_selection, save_selection},
    *,
};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};
#[derive(Debug)]
pub enum ThemeUpdate {
    Registry {
        request: u64,
        entries: Vec<ThemeEntry>,
        notices: Vec<String>,
    },
    System {
        request: u64,
        theme: Option<LoadedTheme>,
        notice: Option<String>,
    },
    Selected {
        request: u64,
        selection: ThemeSelection,
        theme: Result<LoadedTheme, String>,
    },
    Saved {
        request: u64,
        error: Option<String>,
    },
}
#[derive(Default)]
pub(super) struct Pending {
    pub follow: bool,
    pub generation: u64,
    pub reload: bool,
    pub watch_failed: bool,
    pub stop: bool,
    registry: Option<u64>,
    select: Option<(u64, ThemeSelection)>,
    save: Option<(u64, ThemeSelection)>,
}
pub struct ThemeService {
    controller: ThemeController,
    paths: ThemePaths,
    generation: u64,
    registry_request: u64,
    pending: Arc<Mutex<Pending>>,
    wake: mpsc::SyncSender<()>,
    updates: async_channel::Receiver<ThemeUpdate>,
    worker: Option<std::thread::JoinHandle<()>>,
    worker_channels: Option<(mpsc::Receiver<()>, async_channel::Sender<ThemeUpdate>)>,
    notices: Vec<String>,
    selecting: bool,
    must_save: bool,
}
impl ThemeService {
    pub fn new(paths: ThemePaths, overrides: ThemeOverrides, native_dark: bool) -> Self {
        let mut service = Self::new_deferred(paths, overrides, native_dark);
        service.start();
        service
    }
    /// Resolve startup colors now; call start after first paint to activate I/O.
    /// User selection/chooser commands also activate a deferred service.
    pub fn new_deferred(paths: ThemePaths, overrides: ThemeOverrides, native_dark: bool) -> Self {
        let (selection, mut notices) = read_selection(&paths, &overrides);
        let must_save =
            overrides.theme.is_some() || overrides.appearance.is_some() || !notices.is_empty();
        let mut entries = builtin_entries();
        if let ThemeSelection::Named(id) = &selection
            && let Some(path) = paths.theme_path(id)
        {
            match load_theme(&path, ThemeInputKind::User) {
                Ok(t) => entries.push(ThemeEntry {
                    id: id.clone(),
                    label: t.name,
                    palette: Arc::new(t.palette),
                }),
                Err(e) => {
                    notices.push(format!("Selected theme unavailable: {e}; following system"))
                }
            }
        }
        let follows =
            selection == ThemeSelection::System || !entries.iter().any(|e| e.id == selection.id());
        let (system, notice) = if follows {
            super::watch::read_system(&paths)
        } else {
            (None, None)
        };
        notices.extend(notice);
        let controller = ThemeController::new(selection, entries, native_dark, system);
        let pending = Arc::new(Mutex::new(Pending {
            follow: follows,
            reload: follows,
            generation: 1,
            ..Default::default()
        }));
        let (wake, rx) = mpsc::sync_channel(1);
        let (tx, updates) = async_channel::bounded(1);
        Self {
            controller,
            paths,
            generation: 1,
            registry_request: 0,
            pending,
            wake,
            updates,
            worker: None,
            worker_channels: Some((rx, tx)),
            notices,
            selecting: false,
            must_save,
        }
    }
    pub fn start(&mut self) {
        let Some((rx, tx)) = self.worker_channels.take() else {
            return;
        };
        let paths = self.paths.clone();
        let state = self.pending.clone();
        let signal = self.wake.clone();
        match std::thread::Builder::new()
            .name("lenz-themes".into())
            .spawn(move || worker(paths, state, rx, signal, tx))
        {
            Ok(worker) => self.worker = Some(worker),
            Err(e) => self
                .notices
                .push(format!("Live theme updates unavailable: {e}")),
        }
        let _ = self.wake.try_send(());
    }
    pub fn snapshot(&self) -> ThemeSnapshot {
        self.controller.snapshot()
    }
    /// The palette a theme would show (`id` as the chooser lists it, "system" included), for a live preview while
    /// moving through the chooser: nothing is selected or saved. Both frontends.
    pub fn preview(&self, id: &str) -> Option<Arc<crate::theme::Palette>> {
        ThemeSelection::from_id(id).map(|selection| self.controller.resolved(&selection).0)
    }
    pub fn entries(&self) -> &[ThemeEntry] {
        &self.controller.entries
    }
    pub fn updates(&self) -> async_channel::Receiver<ThemeUpdate> {
        self.updates.clone()
    }
    pub fn take_notices(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notices)
    }
    fn queue(&self, f: impl FnOnce(&mut Pending)) {
        f(&mut self.pending.lock().unwrap_or_else(|e| e.into_inner()));
        let _ = self.wake.try_send(());
    }
    pub fn execute(&mut self, command: ThemeCommand) -> ThemeEffect {
        if !matches!(&command, ThemeCommand::NativeAppearance(_)) {
            self.start();
        }
        match command {
            ThemeCommand::OpenChooser => {
                self.registry_request += 1;
                let request = self.registry_request;
                self.queue(|p| p.registry = Some(request));
                ThemeEffect::default()
            }
            ThemeCommand::NativeAppearance(dark) => ThemeEffect {
                repaint: self.controller.native(dark),
                notice: None,
            },
            ThemeCommand::Select(selection) => {
                if let ThemeSelection::Named(id) = &selection
                    && self.paths.theme_path(id).is_some()
                {
                    self.selecting = true;
                    self.generation += 1;
                    let request = self.generation;
                    self.queue(|p| {
                        p.generation = request;
                        p.select = Some((request, selection));
                    });
                    return ThemeEffect::default();
                }
                if self.controller.selection == selection && !self.selecting && !self.must_save {
                    return ThemeEffect::default();
                }
                match self.controller.select(selection.clone()) {
                    Err(e) => ThemeEffect {
                        repaint: false,
                        notice: Some(e.into()),
                    },
                    Ok(repaint) => {
                        self.selecting = false;
                        self.must_save = false;
                        self.generation += 1;
                        let request = self.generation;
                        let follow = selection == ThemeSelection::System;
                        self.queue(|p| {
                            p.generation = request;
                            p.select = None;
                            p.follow = follow;
                            p.reload = follow;
                            p.save = Some((request, selection));
                        });
                        ThemeEffect {
                            repaint,
                            notice: None,
                        }
                    }
                }
            }
        }
    }
    pub fn apply(&mut self, update: ThemeUpdate) -> ThemeEffect {
        match update {
            ThemeUpdate::Registry {
                request,
                entries,
                mut notices,
            } if request == self.registry_request => {
                let was_named = self
                    .controller
                    .entries
                    .iter()
                    .any(|e| e.id == self.controller.selection.id());
                self.controller.registry(entries);
                let follows = self.controller.selection == ThemeSelection::System
                    || !self
                        .controller
                        .entries
                        .iter()
                        .any(|e| e.id == self.controller.selection.id());
                self.queue(|p| {
                    p.follow = follows;
                    p.reload = follows;
                });
                if was_named && follows {
                    notices.push("Selected theme unavailable; following system".into());
                }
                ThemeEffect {
                    repaint: true,
                    notice: (!notices.is_empty()).then(|| notices.join("; ")),
                }
            }
            ThemeUpdate::System {
                request,
                theme,
                notice,
            } if request == self.generation => ThemeEffect {
                repaint: self.controller.system(theme),
                notice,
            },
            ThemeUpdate::Saved { request, error } if request == self.generation => {
                self.must_save = error.is_some();
                ThemeEffect {
                    repaint: error.is_some(),
                    notice: error.map(|e| format!("Theme changed but was not saved: {e}")),
                }
            }
            ThemeUpdate::Selected {
                request,
                selection,
                theme,
            } if request == self.generation => {
                self.selecting = false;
                match theme {
                    Ok(theme) => {
                        self.must_save = false;
                        let id = selection.id().to_owned();
                        self.controller.entries.retain(|e| e.id != id);
                        self.controller.entries.push(ThemeEntry {
                            id,
                            label: theme.name,
                            palette: Arc::new(theme.palette),
                        });
                        let repaint = self.controller.select(selection.clone()).unwrap_or(false);
                        self.queue(|p| {
                            p.follow = false;
                            p.save = Some((request, selection));
                        });
                        ThemeEffect {
                            repaint,
                            notice: None,
                        }
                    }
                    Err(e) => {
                        self.controller.unavailable(selection.id());
                        let follow = self.controller.selection == ThemeSelection::System
                            || !self
                                .controller
                                .entries
                                .iter()
                                .any(|e| e.id == self.controller.selection.id());
                        self.queue(|p| {
                            p.follow = follow;
                            p.reload = follow;
                        });
                        ThemeEffect {
                            repaint: true,
                            notice: Some(format!("Theme no longer available: {e}")),
                        }
                    }
                }
            }
            _ => ThemeEffect::default(),
        }
    }
}
impl Drop for ThemeService {
    fn drop(&mut self) {
        self.updates.close();
        self.queue(|p| p.stop = true);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
// Notifications coalesce work; they must not shorten a settling deadline.
// Selection and shutdown still interrupt the wait immediately.
fn settle(
    rx: &mpsc::Receiver<()>,
    pending: &Mutex<Pending>,
    generation: u64,
    delay: Duration,
    again: &mut bool,
) -> bool {
    let deadline = Instant::now() + delay;
    loop {
        let p = pending.lock().unwrap_or_else(|e| e.into_inner());
        if p.stop || !p.follow || p.generation != generation {
            return false;
        }
        drop(p);
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return true;
        }
        match rx.recv_timeout(remaining) {
            Ok(()) => *again = true,
            Err(mpsc::RecvTimeoutError::Timeout) => return true,
            Err(mpsc::RecvTimeoutError::Disconnected) => return false,
        }
    }
}
fn worker(
    paths: ThemePaths,
    pending: Arc<Mutex<Pending>>,
    rx: mpsc::Receiver<()>,
    wake: mpsc::SyncSender<()>,
    tx: async_channel::Sender<ThemeUpdate>,
) {
    let mut watches = if cfg!(target_os = "linux") {
        super::watch::Watches::new(&paths, pending.clone(), wake).ok()
    } else {
        None
    };
    let mut fallback = cfg!(target_os = "linux") && watches.is_none();
    let mut fallback_notice = fallback;
    let mut last: Option<(Option<LoadedTheme>, Option<String>)> = None;
    let mut last_generation = 0;
    let mut again = false;
    loop {
        let signaled = if std::mem::take(&mut again) {
            true
        } else if fallback {
            rx.recv_timeout(Duration::from_secs(1)).is_ok()
        } else {
            rx.recv().is_ok()
        };
        if !signaled && !fallback {
            break;
        }
        let (registry, select, save, stop, follow, generation, reload, watch_failed) = {
            let mut p = pending.lock().unwrap_or_else(|e| e.into_inner());
            let out = (
                p.registry.take(),
                p.select.take(),
                p.save.take(),
                p.stop,
                p.follow,
                p.generation,
                p.reload || fallback,
                std::mem::take(&mut p.watch_failed),
            );
            p.reload = false;
            out
        };
        if watch_failed {
            fallback = true;
            fallback_notice = true;
            watches = None;
        }
        if let Some((request, selection)) = save {
            let error = save_selection(&paths, &selection)
                .err()
                .map(|e| e.to_string());
            if error.is_some() {
                let _ = tx.send_blocking(ThemeUpdate::Saved { request, error });
            }
        }
        if stop {
            break;
        }
        if let Some(request) = registry {
            let (entries, notices) = discover_themes(&paths);
            let _ = tx.send_blocking(ThemeUpdate::Registry {
                request,
                entries,
                notices,
            });
        }
        if let Some((request, selection)) = select {
            let theme = paths
                .theme_path(selection.id())
                .ok_or_else(|| "Invalid theme ID".into())
                .and_then(|p| load_theme(&p, ThemeInputKind::User).map_err(|e| e.to_string()));
            let _ = tx.send_blocking(ThemeUpdate::Selected {
                request,
                selection,
                theme,
            });
        }
        if follow && reload {
            if !fallback {
                if !settle(
                    &rx,
                    &pending,
                    generation,
                    Duration::from_millis(50),
                    &mut again,
                ) {
                    again = true;
                    continue;
                }
                if let Some(w) = &mut watches
                    && w.refresh().is_err()
                {
                    fallback = true;
                    fallback_notice = true;
                }
            }
            let mut result = super::watch::read_system(&paths);
            if result.0.is_none() {
                for delay in [50, 100, 200] {
                    if !settle(
                        &rx,
                        &pending,
                        generation,
                        Duration::from_millis(delay),
                        &mut again,
                    ) {
                        again = true;
                        break;
                    }
                    result = super::watch::read_system(&paths);
                    if result.0.is_some() {
                        break;
                    }
                }
            }
            let p = pending.lock().unwrap_or_else(|e| e.into_inner());
            if p.stop || !p.follow || p.generation != generation {
                drop(p);
                again = true;
                continue;
            }
            drop(p);
            if result.0.is_none()
                && result.1.is_none()
                && last.as_ref().is_some_and(|r| r.0.is_some())
            {
                result.1 = Some("Omarchy theme unavailable; using system appearance".into());
            }
            if fallback_notice {
                result.1 = Some(
                    "Filesystem notifications unavailable; checking Omarchy once per second".into(),
                );
                fallback_notice = false;
            }
            let equal = last_generation == generation
                && last.as_ref().is_some_and(|prev| {
                    prev.0.as_ref().map(|t| (&t.name, &t.palette))
                        == result.0.as_ref().map(|t| (&t.name, &t.palette))
                        && prev.1 == result.1
                });
            if !equal {
                let _ = tx.send_blocking(ThemeUpdate::System {
                    request: generation,
                    theme: result.0.clone(),
                    notice: result.1.clone(),
                });
                last = Some(result);
                last_generation = generation;
            }
        }
    }
}
