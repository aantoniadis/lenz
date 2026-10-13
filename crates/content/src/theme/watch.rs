use super::service::Pending;
use super::{LoadedTheme, ThemeInputKind, ThemePaths, load_theme, loader::read_bounded};
use notify::{RecursiveMode, Watcher};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex, mpsc::SyncSender},
};
pub(super) fn read_system(paths: &ThemePaths) -> (Option<LoadedTheme>, Option<String>) {
    if !cfg!(target_os = "linux") {
        return (None, None);
    }
    let mut error = None;
    for dir in &paths.omarchy {
        let file = dir.join("theme/colors.toml");
        if !file.exists() {
            continue;
        }
        match load_theme(&file, ThemeInputKind::Omarchy) {
            Ok(mut theme) => {
                theme.name = read_bounded(&dir.join("theme.name"))
                    .ok()
                    .and_then(|b| String::from_utf8(b).ok())
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control))
                    .unwrap_or_else(|| "Omarchy".into());
                return (Some(theme), None);
            }
            Err(e) => {
                error = Some(format!(
                    "Cannot follow Omarchy: {e}; using system appearance"
                ));
                break;
            }
        }
    }
    (None, error)
}
pub(super) struct Watches {
    watcher: notify::RecommendedWatcher,
    watched: BTreeMap<PathBuf, (u64, u64)>,
    targets: [PathBuf; 2],
}
impl Watches {
    pub fn new(
        paths: &ThemePaths,
        pending: Arc<Mutex<Pending>>,
        wake: SyncSender<()>,
    ) -> Result<Self, notify::Error> {
        let targets = paths.omarchy.clone();
        let watched_targets = targets.clone();
        let watcher =
            notify::recommended_watcher(move |result: Result<notify::Event, notify::Error>| {
                on_event(&watched_targets, &pending, &wake, result);
            })?;
        let mut this = Self {
            watcher,
            watched: BTreeMap::new(),
            targets,
        };
        this.refresh()?;
        Ok(this)
    }
    pub fn refresh(&mut self) -> Result<(), notify::Error> {
        let mut desired = BTreeMap::new();
        for target in &self.targets {
            let full = target.join("theme");
            let mut covered = false;
            // Bound watch descriptors, not the search for an existing ancestor.
            // Deep, absent XDG roots still need a watch to detect their creation.
            for (path, metadata) in full
                .ancestors()
                .filter_map(|path| {
                    path.metadata()
                        .ok()
                        .filter(|m| m.is_dir())
                        .map(|m| (path, m))
                })
                .take(12)
            {
                covered = true;
                #[cfg(unix)]
                let identity = {
                    use std::os::unix::fs::MetadataExt;
                    (metadata.dev(), metadata.ino())
                };
                #[cfg(not(unix))]
                let identity = (0, 0); // Notifications are activated only on Linux.
                desired.insert(path.to_owned(), identity);
            }
            if !covered {
                return Err(notify::Error::generic(
                    "No existing theme ancestor to watch",
                ));
            }
        }
        for (old, identity) in &self.watched {
            if desired.get(old) != Some(identity) {
                let _ = self.watcher.unwatch(old);
            }
        }
        for (path, identity) in &desired {
            if self.watched.get(path) != Some(identity) {
                self.watcher.watch(path, RecursiveMode::NonRecursive)?;
            }
        }
        self.watched = desired;
        Ok(())
    }
}

pub(super) fn on_event(
    targets: &[PathBuf; 2],
    pending: &Arc<Mutex<Pending>>,
    wake: &SyncSender<()>,
    result: Result<notify::Event, notify::Error>,
) {
    // Reading a watched palette must not feed back into another reload.
    if result
        .as_ref()
        .is_ok_and(|event| matches!(event.kind, notify::EventKind::Access(_)))
    {
        return;
    }
    let relevant = result.as_ref().map_or(true, |event| {
        event.paths.iter().any(|path| {
            targets
                .iter()
                .any(|target| path.starts_with(target) || target.starts_with(path))
        })
    });
    if relevant {
        let mut p = pending.lock().unwrap_or_else(|e| e.into_inner());
        if p.follow {
            p.reload = true;
        }
        p.watch_failed |= result.is_err();
        drop(p);
        let _ = wake.try_send(());
    }
}
