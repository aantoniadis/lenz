use super::{ThemeEntry, ThemeError, ThemeInputKind, builtin, load_theme, loader::read_bounded};
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
/// Theme storage, with optional read-only compatibility paths.
#[derive(Clone, Debug)]
pub struct ThemePaths {
    /// User themes, one `<name>.toml` per `user:<name>` ID.
    pub themes: PathBuf,
    /// The saved selection (`{"version":1,"theme":ID}`); other fields in it are preserved.
    pub selection: PathBuf,
    /// Omarchy `current` directories the System theme follows, first found wins.
    pub omarchy: [PathBuf; 2],
    legacy: Option<Box<ThemePaths>>,
}
impl ThemePaths {
    /// Canonical Lenz appearance with Lenz Code's old paths as read-only fallbacks: its `lenz-code` folder, then the
    /// `lenz-reader` folder it had as Lenz Reader until it first runs and moves it.
    pub fn shared() -> Self {
        let mut paths = Self::xdg("lenz");
        let mut code = Self::xdg("lenz-code");
        code.legacy = Some(Box::new(Self::xdg("lenz-reader")));
        paths.legacy = Some(Box::new(code));
        paths
    }
    pub fn shared_under(config_home: &Path, state_home: &Path) -> Self {
        let mut paths = Self::under(config_home, state_home, "lenz");
        let mut code = Self::under(config_home, state_home, "lenz-code");
        code.legacy = Some(Box::new(Self::under(config_home, state_home, "lenz-reader")));
        paths.legacy = Some(Box::new(code));
        paths
    }
    /// The first saved selection along the fallbacks; the canonical path when none is saved.
    fn selection_path(&self) -> &Path {
        let mut at = self;
        while !present(&at.selection) && let Some(legacy) = &at.legacy {
            if present(&legacy.selection) { return &legacy.selection; }
            at = legacy;
        }
        &self.selection
    }

    /// The XDG layout for `app`: `$XDG_CONFIG_HOME/<app>/themes`, `$XDG_CONFIG_HOME/<app>/appearance.json`, and
    /// Omarchy's `current` under `$XDG_STATE_HOME`, then `$XDG_CONFIG_HOME`. Relative XDG values are ignored.
    pub fn xdg(app: &str) -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| PathBuf::from("/dev/null"));
        let path = |key: &str, default: &str| {
            std::env::var_os(key)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home.join(default))
        };
        Self::under(&path("XDG_CONFIG_HOME", ".config"), &path("XDG_STATE_HOME", ".local/state"), app)
    }
    /// [`ThemePaths::xdg`] with the config and state homes given.
    pub fn under(config_home: &Path, state_home: &Path, app: &str) -> Self {
        Self {
            themes: config_home.join(app).join("themes"),
            selection: config_home.join(app).join("appearance.json"),
            omarchy: [state_home.join("omarchy/current"), config_home.join("omarchy/current")],
            legacy: None,
        }
    }
    pub fn theme_path(&self, id: &str) -> Option<PathBuf> {
        id.strip_prefix("user:")
            .filter(|_| valid_id(id))
            .map(|stem| {
                let file = format!("{stem}.toml");
                let canonical = self.themes.join(&file);
                std::iter::successors(self.legacy.as_deref(), |p| p.legacy.as_deref())
                    .map(|p| p.themes.join(&file))
                    .find(|path| present(path))
                    .filter(|_| !present(&canonical))
                    .unwrap_or(canonical)
            })
    }
}
/// A product's override of the saved selection: `theme` (an ID or `system`) wins over `appearance` (`light` or
/// `dark`). The labels name them in notices, e.g. the environment variables they came from.
#[derive(Clone, Debug)]
pub struct ThemeOverrides {
    pub theme: Option<String>,
    pub appearance: Option<String>,
    pub theme_label: &'static str,
    pub appearance_label: &'static str,
}
impl Default for ThemeOverrides {
    fn default() -> Self {
        Self { theme: None, appearance: None, theme_label: "theme override", appearance_label: "appearance override" }
    }
}
impl ThemeOverrides {
    /// LENZ_THEME > LENZ_APPEARANCE > LENZ_CODE_THEME > LENZ_CODE_APPEARANCE > LENZ_READER_THEME >
    /// LENZ_READER_APPEARANCE (Lenz Code's names from when it was Lenz Reader).
    /// An invalid higher-priority override is reported, never hidden by a legacy value.
    pub fn shared_from_env() -> Self {
        [("LENZ_THEME", "LENZ_APPEARANCE"), ("LENZ_CODE_THEME", "LENZ_CODE_APPEARANCE")].into_iter()
            .map(|(theme, appearance)| Self::from_env(theme, appearance))
            .find(|o| o.theme.is_some() || o.appearance.is_some())
            .unwrap_or_else(|| Self::from_env("LENZ_READER_THEME", "LENZ_READER_APPEARANCE"))
    }

    /// The overrides in the environment variables `theme_var` and `appearance_var`, labelled by their names.
    pub fn from_env(theme_var: &'static str, appearance_var: &'static str) -> Self {
        Self {
            theme: std::env::var(theme_var).ok(),
            appearance: std::env::var(appearance_var).ok(),
            theme_label: theme_var,
            appearance_label: appearance_var,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThemeSelection {
    System,
    Named(String),
}
impl ThemeSelection {
    pub fn id(&self) -> &str {
        match self {
            Self::System => "system",
            Self::Named(id) => id,
        }
    }
    pub fn from_id(id: &str) -> Option<Self> {
        if id == "system" {
            Some(Self::System)
        } else if valid_id(id) {
            Some(Self::Named(id.into()))
        } else {
            None
        }
    }
}
pub fn valid_id(id: &str) -> bool {
    builtin(id).is_some()
        || id.strip_prefix("user:").is_some_and(|s| {
            !s.is_empty()
                && s.len() <= 200
                && s != "."
                && s != ".."
                && !s
                    .chars()
                    .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
        })
}
pub fn read_selection(
    paths: &ThemePaths,
    overrides: &ThemeOverrides,
) -> (ThemeSelection, Vec<String>) {
    let mut notices = Vec::new();
    let file = paths.selection_path();
    let saved = match read_bounded(file) {
        Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(&bytes) {
            Ok(v) if v["version"] == 1 => v["theme"]
                .as_str()
                .and_then(ThemeSelection::from_id)
                .unwrap_or_else(|| {
                    notices.push("Invalid saved theme; following system".into());
                    ThemeSelection::System
                }),
            _ => {
                notices.push("Unsupported or invalid appearance settings; following system".into());
                ThemeSelection::System
            }
        },
        Err(e) if !present(file) => {
            let _ = e;
            ThemeSelection::System
        }
        Err(e) => {
            notices.push(format!("Cannot read appearance settings: {e}"));
            ThemeSelection::System
        }
    };
    if let Some(id) = &overrides.theme {
        if let Some(selection) = ThemeSelection::from_id(id) {
            return (selection, notices);
        }
        notices.push(format!("Unknown {}: {id}", overrides.theme_label));
    } else if let Some(a) = &overrides.appearance {
        match a.as_str() {
            "light" => return (ThemeSelection::Named("light".into()), notices),
            "dark" => return (ThemeSelection::Named("darcula".into()), notices),
            _ => notices.push(format!("Unknown {}: {a}", overrides.appearance_label)),
        }
    }
    (saved, notices)
}
pub fn save_selection(paths: &ThemePaths, selection: &ThemeSelection) -> Result<(), ThemeError> {
    let file = &paths.selection;
    let dir = file.parent().ok_or_else(|| ThemeError("Invalid appearance settings path".into()))?;
    std::fs::create_dir_all(dir)?;
    let mut value = if file.exists() {
        let bytes = read_bounded(file)?;
        serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .filter(|v| v.is_object() && v["version"] == 1)
            .unwrap_or_else(|| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    value["version"] = 1.into();
    value["theme"] = selection.id().into();
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let temp = dir.join(format!(
        ".appearance-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        f.write_all(&serde_json::to_vec_pretty(&value).map_err(|e| ThemeError(e.to_string()))?)?;
        f.sync_all()?;
        std::fs::rename(&temp, file)?;
        Ok(())
    })();
    let _ = std::fs::remove_file(temp);
    result
}
pub fn discover_themes(paths: &ThemePaths) -> (Vec<ThemeEntry>, Vec<String>) {
    let mut candidates = BTreeMap::new();
    let mut notices = Vec::new();
    let mut truncated = false;
    // Canonical filenames reserve their ID even when their contents are invalid.
    for dir in std::iter::successors(Some(paths), |p| p.legacy.as_deref()).map(|p| &p.themes) {
        let iter = match std::fs::read_dir(dir) {
            Ok(iter) => iter,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => { notices.push(e.to_string()); continue; }
        };
        for entry in iter.flatten() {
            let p = entry.path();
            if p.extension().is_some_and(|s| s == "toml") {
                candidates.entry(entry.file_name()).or_insert(p);
                if candidates.len() > 256 {
                    candidates.pop_last();
                    truncated = true;
                }
            }
        }
    }
    let mut entries = Vec::new();
    if truncated {
        notices.push("Only the first 256 user theme files are listed".into());
    }
    for p in candidates.into_values() {
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let id = format!("user:{stem}");
        if !valid_id(&id) {
            notices.push(format!("Invalid theme filename: {stem}"));
            continue;
        }
        match load_theme(&p, ThemeInputKind::User) {
            Ok(theme) => entries.push(ThemeEntry {
                id,
                label: theme.name,
                palette: std::sync::Arc::new(theme.palette),
            }),
            Err(e) => notices.push(format!("{}: {e}", display(&p))),
        }
    }
    (entries, notices)
}
fn display(p: &Path) -> String {
    p.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn present(path: &Path) -> bool {
    !matches!(std::fs::symlink_metadata(path), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}
