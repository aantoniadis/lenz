use super::{
    Palette,
    presets::{Seed, blend, from_seed},
};
use std::{io::Read, path::Path};
#[derive(Clone, Debug)]
pub struct LoadedTheme {
    pub name: String,
    pub palette: Palette,
}
#[derive(Clone, Copy)]
pub enum ThemeInputKind {
    User,
    Omarchy,
}
#[derive(Debug)]
pub struct ThemeError(pub String);
impl std::fmt::Display for ThemeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ThemeError {}
impl From<std::io::Error> for ThemeError {
    fn from(e: std::io::Error) -> Self {
        Self(e.to_string())
    }
}
fn invalid(key: &str) -> ThemeError {
    ThemeError(format!("Invalid theme field: {key}"))
}
fn rgb(value: &toml::Value, key: &str) -> Result<u32, ThemeError> {
    let s = value.as_str().ok_or_else(|| invalid(key))?;
    if s.len() != 7 || !s.starts_with('#') || !s[1..].bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid(key));
    }
    u32::from_str_radix(&s[1..], 16).map_err(|_| invalid(key))
}
/// A regular file of at most 64 KiB, opened without blocking on a FIFO.
pub fn read_bounded(path: &Path) -> Result<Vec<u8>, ThemeError> {
    if !std::fs::metadata(path)?.is_file() {
        return Err(ThemeError("Theme input is not a regular file".into()));
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(ThemeError("Theme input is not a regular file".into()));
    }
    let mut bytes = Vec::new();
    file.take(65537).read_to_end(&mut bytes)?;
    if bytes.len() > 65536 {
        return Err(ThemeError("Theme input exceeds 64 KiB".into()));
    }
    Ok(bytes)
}
pub fn load_theme(path: &Path, kind: ThemeInputKind) -> Result<LoadedTheme, ThemeError> {
    let bytes = read_bounded(path)?;
    parse_theme(
        std::str::from_utf8(&bytes).map_err(|_| invalid("UTF-8"))?,
        kind,
    )
}
pub fn parse_theme(text: &str, kind: ThemeInputKind) -> Result<LoadedTheme, ThemeError> {
    if text.len() > 65536 {
        return Err(ThemeError("Theme input exceeds 64 KiB".into()));
    }
    let value = toml::from_str::<toml::Value>(text).map_err(|e| ThemeError(e.to_string()))?;
    let table = value.as_table().ok_or_else(|| invalid("table"))?;
    let color = |key: &str| {
        table
            .get(key)
            .ok_or_else(|| invalid(key))
            .and_then(|v| rgb(v, key))
    };
    let optional = |key: &str, default: u32| table.get(key).map_or(Ok(default), |v| rgb(v, key));
    let dark = match table.get("mode").and_then(toml::Value::as_str) {
        Some("dark") => true,
        Some("light") => false,
        _ => return Err(invalid("mode")),
    };
    let name = match table.get("name").and_then(toml::Value::as_str) {
        Some(s) if !s.trim().is_empty() && s.len() <= 128 && !s.chars().any(char::is_control) => {
            s.to_owned()
        }
        _ if matches!(kind, ThemeInputKind::Omarchy) => "Omarchy".into(),
        _ => return Err(invalid("name")),
    };
    let bg = color("background")?;
    let fg = color("foreground")?;
    let accent = color("accent")?;
    let mut p = from_seed(Seed {
        bg,
        fg,
        accent,
        selection: optional("selection", blend(accent, bg, 30))?,
        muted: optional("dark_foreground", optional("muted", blend(fg, bg, 60))?)?,
        dark: optional(
            "dark_background",
            blend(if dark { 0 } else { 0xffffff }, bg, 8),
        )?,
        lighter: optional("lighter_background", blend(fg, bg, 10))?,
        red: color("red")?,
        green: color("green")?,
        blue: color("blue")?,
        yellow: color("yellow")?,
        cyan: color("cyan")?,
        magenta: color("magenta")?,
    });
    if let Some(value) = table.get("palette") {
        let overrides = value.as_table().ok_or_else(|| invalid("palette"))?;
        for (key, value) in overrides {
            macro_rules! scalars{($($field:ident),*)=>{match key.as_str(){$(stringify!($field)=>p.$field=rgb(value,key)?,)*"block"=>p.block=array(value,key)?,"inline"=>p.inline=array(value,key)?,"edge"=>p.edge=array(value,key)?,"status"=>p.status=array(value,key)?,_=>return Err(invalid(key))}}}
            scalars!(
                bg,
                fg,
                dim,
                gutter_bg,
                num_fg,
                header_bg,
                sidebar_bg,
                status_bg,
                border,
                selected_bg,
                fold_bg,
                sel_bg,
                caret,
                found_bg,
                banner_bg,
                accent,
                error,
                blame_new
            );
        }
    }
    if let Some(value) = table.get("syntax") {
        let overrides = value.as_table().ok_or_else(|| invalid("syntax"))?;
        for (key, value) in overrides {
            macro_rules! roles{($($field:ident),*)=>{match key.as_str(){$(stringify!($field)=>p.syntax.$field=rgb(value,key)?,)*_=>return Err(invalid(key))}}}
            roles!(
                keyword,
                string,
                number,
                comment,
                doc_comment,
                function,
                constant,
                macro_,
                type_,
                builtin_var,
                tag,
                attribute,
                label,
                link
            );
        }
    }
    Ok(LoadedTheme { name, palette: p })
}
fn array<const N: usize>(value: &toml::Value, key: &str) -> Result<[u32; N], ThemeError> {
    let a = value.as_array().ok_or_else(|| invalid(key))?;
    if a.len() != N {
        return Err(invalid(key));
    }
    let mut out = [0; N];
    for (i, value) in a.iter().enumerate() {
        out[i] = rgb(value, key)?;
    }
    Ok(out)
}
