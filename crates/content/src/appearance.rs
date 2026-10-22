//! Font preferences shared by Reader and Diff, read at startup from `lenz/settings.toml`.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct FontPreferences {
    pub font_family: Option<String>,
    pub font_size: f32,
    pub reading_font: Option<String>,
}
impl Default for FontPreferences {
    fn default() -> Self { Self { font_family: None, font_size: 13.0, reading_font: None } }
}
pub const FONT_SIZES: std::ops::RangeInclusive<f32> = 8.0..=32.0;

impl FontPreferences {
    /// Apply one GUI setting. Unknown fields return None for the product's parser to handle.
    pub fn apply_field(&mut self, key: &str, value: &toml::Value) -> Option<Result<(), &'static str>> {
        Some(match key {
            "font_family" | "reading_font" => match value.as_str().map(str::trim).filter(|t| !t.is_empty() && t.len() <= 128) {
                Some(t) => { *if key == "font_family" { &mut self.font_family } else { &mut self.reading_font } = Some(t.to_owned()); Ok(()) },
                None => Err("must be a font name"),
            },
            "font_size" => match value.as_float().or_else(|| value.as_integer().map(|i| i as f64)) {
                Some(n) if n.is_finite() && (8.0..=32.0).contains(&n) => { self.font_size = n as f32; Ok(()) },
                Some(_) => Err("must be between 8 and 32"),
                None => Err("must be a number"),
            },
            _ => return None,
        })
    }
}

/// `$XDG_CONFIG_HOME/lenz/settings.toml`, else `~/.config/lenz/settings.toml`.
pub fn path() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute()).map(|h| h.join(".config")))?;
    Some(config.join("lenz/settings.toml"))
}

/// Read once at startup; changes affect the next launch. Other product settings are ignored.
pub fn load() -> (FontPreferences, Vec<String>) {
    path().map_or_else(Default::default, |p| load_at(&p))
}
pub fn load_at(path: &Path) -> (FontPreferences, Vec<String>) {
    if matches!(std::fs::symlink_metadata(path), Err(e) if e.kind() == std::io::ErrorKind::NotFound) { return Default::default(); }
    match crate::theme::read_bounded(path) {
        Ok(bytes) => match std::str::from_utf8(&bytes) {
            Ok(text) => parse(text),
            Err(_) => (FontPreferences::default(), vec![format!("{}: not UTF-8; using defaults", path.display())]),
        },
        Err(e) => (FontPreferences::default(), vec![format!("{}: {e}; using defaults", path.display())]),
    }
}

/// The settings file's `[tools] editor`, for Open in Editor when the environment names none (an app started from
/// the Dock or a desktop menu rarely has `$VISUAL`). Read when asked; any problem is no editor.
pub fn editor() -> Option<String> {
    editor_in(std::str::from_utf8(&crate::theme::read_bounded(&path()?).ok()?).ok()?)
}

pub fn editor_in(text: &str) -> Option<String> {
    let table = text.parse::<toml::Table>().ok()?;
    let editor = table.get("tools")?.get("editor")?.as_str()?.trim();
    (!editor.is_empty()).then(|| editor.to_string())
}

pub fn parse(text: &str) -> (FontPreferences, Vec<String>) {
    let mut fonts = FontPreferences::default();
    let table = match text.parse::<toml::Table>() {
        Ok(table) => table,
        Err(e) => return (fonts, vec![format!("settings.toml: {}; using defaults", e.message())]),
    };
    let mut notices = Vec::new();
    if let Some(value) = table.get("gui") {
        if let Some(fields) = value.as_table() {
            for (key, value) in fields {
                if let Some(Err(why)) = fonts.apply_field(key, value) {
                    notices.push(format!("settings.toml: gui.{key} {why}; using the default"));
                }
            }
        } else {
            notices.push("settings.toml: `gui` should be a [section]; ignored".into());
        }
    }
    (fonts, notices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_preferences_share_defaults_and_validation() {
        let (p, n) = parse("[gui]\nfont_family = ' Mono '\nfont_size = 18\nreading_font = 'Inter'\n[view]\nblame = true");
        assert!(n.is_empty(), "{n:?}");
        assert_eq!(p, FontPreferences { font_family: Some("Mono".into()), font_size: 18.0, reading_font: Some("Inter".into()) });
        for value in ["7", "33", "nan", "inf", "'large'"] {
            let (p, n) = parse(&format!("[gui]\nfont_size = {value}\nfont_family = ''\nreading_font = false"));
            assert_eq!(p, FontPreferences::default());
            assert_eq!(n.len(), 3, "{n:?}");
        }
        for value in [8, 32] {
            let (p, n) = parse(&format!("[gui]\nfont_size = {value}"));
            assert_eq!(p.font_size, value as f32);
            assert!(n.is_empty());
        }
        assert_eq!(parse("").0, FontPreferences::default());
        assert_eq!(parse("[gui").1.len(), 1);
    }

    #[test]
    fn the_editor_comes_from_tools_and_ignores_blanks_and_bad_files() {
        assert_eq!(editor_in("[tools]\neditor = \" code --wait \"\n"), Some("code --wait".into()));
        assert_eq!(editor_in("[tools]\neditor = \"  \"\n"), None);
        assert_eq!(editor_in("[gui]\nfont_size = 14\n"), None);
        assert_eq!(editor_in("[tools\n"), None);
    }

    #[test]
    fn bounded_font_loading_defaults_and_reports_invalid_files() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("settings.toml");
        assert_eq!(load_at(&path), (FontPreferences::default(), vec![]));
        std::fs::write(&path, "[gui]\nfont_size = 17").unwrap();
        assert_eq!(load_at(&path).0.font_size, 17.0);
        std::fs::write(&path, [0xff]).unwrap();
        assert_eq!(load_at(&path).1.len(), 1);
        std::fs::write(&path, vec![b'x'; 1024 * 1024 + 1]).unwrap();
        assert_eq!(load_at(&path).1.len(), 1);
    }
}
