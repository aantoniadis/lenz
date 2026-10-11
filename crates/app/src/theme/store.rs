use super::{ThemeEntry, ThemeError, ThemeInputKind, builtin, load_theme, loader::read_bounded};
use std::{
    collections::BTreeSet,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
#[derive(Clone, Debug)]
pub struct ThemePaths {
    pub config_home: PathBuf,
    pub state_home: PathBuf,
}
impl ThemePaths {
    pub fn from_env() -> Self {
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
        Self {
            config_home: path("XDG_CONFIG_HOME", ".config"),
            state_home: path("XDG_STATE_HOME", ".local/state"),
        }
    }
    pub fn theme_path(&self, id: &str) -> Option<PathBuf> {
        id.strip_prefix("user:")
            .filter(|_| valid_id(id))
            .map(|stem| {
                self.config_home
                    .join("lenz/themes")
                    .join(format!("{stem}.toml"))
            })
    }
    pub fn omarchy(&self) -> [PathBuf; 2] {
        [
            self.state_home.join("omarchy/current"),
            self.config_home.join("omarchy/current"),
        ]
    }
}
#[derive(Clone, Debug, Default)]
pub struct ThemeOverrides {
    pub theme: Option<String>,
    pub appearance: Option<String>,
}
impl ThemeOverrides {
    pub fn from_env() -> Self {
        Self {
            theme: std::env::var("LENZ_THEME").ok(),
            appearance: std::env::var("LENZ_APPEARANCE").ok(),
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
    let file = paths.config_home.join("lenz/appearance.json");
    let saved = match read_bounded(&file) {
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
        Err(e) if !file.exists() => {
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
        notices.push(format!("Unknown LENZ_THEME: {id}"));
    } else if let Some(a) = &overrides.appearance {
        match a.as_str() {
            "light" => return (ThemeSelection::Named("light".into()), notices),
            "dark" => return (ThemeSelection::Named("darcula".into()), notices),
            _ => notices.push(format!("Unknown LENZ_APPEARANCE: {a}")),
        }
    }
    (saved, notices)
}
pub fn save_selection(paths: &ThemePaths, selection: &ThemeSelection) -> Result<(), ThemeError> {
    let dir = paths.config_home.join("lenz");
    std::fs::create_dir_all(&dir)?;
    let file = dir.join("appearance.json");
    let mut value = if file.exists() {
        let bytes = read_bounded(&file)?;
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
        std::fs::rename(&temp, &file)?;
        Ok(())
    })();
    let _ = std::fs::remove_file(temp);
    result
}
pub fn discover_themes(paths: &ThemePaths) -> (Vec<ThemeEntry>, Vec<String>) {
    let dir = paths.config_home.join("lenz/themes");
    let iter = match std::fs::read_dir(dir) {
        Ok(i) => i,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Vec::new(), Vec::new()),
        Err(e) => return (Vec::new(), vec![e.to_string()]),
    };
    let mut candidates = BTreeSet::new();
    let mut truncated = false;
    for entry in iter.flatten() {
        let p = entry.path();
        if p.extension().is_some_and(|s| s == "toml") {
            candidates.insert(p);
            if candidates.len() > 256 {
                candidates.pop_last();
                truncated = true;
            }
        }
    }
    let mut entries = Vec::new();
    let mut notices = Vec::new();
    if truncated {
        notices.push("Only the first 256 user theme files are listed".into());
    }
    for p in candidates {
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
