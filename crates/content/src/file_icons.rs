//! File-type glyphs, brand colors and logo identities shared by all reader and diff adapters.
//! SVG bytes and rasterization stay in the GUI adapter.
use crate::theme::{self, Palette};
use std::borrow::Cow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Icon {
    pub glyph: char,
    pub rgb: u32,
    pub folder: bool,
    pub logo: Option<Logo>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Logo {
    pub name: &'static str,
    pub tint: Option<u32>,
}

macro_rules! logo {
    ($name:literal) => {
        logo!($name, None)
    };
    ($name:literal, $tint:expr) => {
        Logo {
            name: $name,
            tint: $tint,
        }
    };
}

impl Icon {
    pub fn label(self, nerd: bool, text: &str) -> Cow<'_, str> {
        if nerd {
            format!("{} {text}", self.glyph).into()
        } else {
            Cow::Borrowed(text)
        }
    }
    pub fn color_on(self, palette: &Palette, background: u32) -> u32 {
        palette.readable_on(color(self.rgb, self.folder, palette), background)
    }
}

/// Brand colors darken on light themes; folders use the current accent.
pub fn color(rgb: u32, folder: bool, palette: &Palette) -> u32 {
    if folder {
        palette.accent
    } else if theme::luminance(palette.bg) >= 0.5 {
        theme::mix(rgb, 0x000000, 0.3)
    } else {
        rgb
    }
}

const FOLDER: char = '\u{f07b}';
const FOLDER_OPEN: char = '\u{f07c}';
const FILE: char = '\u{f15b}';

type Kind = (char, u32, Option<Logo>);

const RUST: Kind = ('\u{e7a8}', 0xdea584, Some(logo!("rust", Some(0xdea584))));
const NPM: Kind = ('\u{e71e}', 0xcb3837, Some(logo!("npm-icon")));
const GIT: Kind = ('\u{e702}', 0xe37933, Some(logo!("git-icon")));
const DOCKER: Kind = ('\u{f308}', 0x2496ed, Some(logo!("docker-icon")));
const MAKE: Kind = ('\u{f085}', 0x6d8086, Some(logo!("make-icon")));
const LICENSE: Kind = ('\u{f02d}', 0xcbcb41, None);
const CONFIG: Kind = ('\u{e615}', 0x6d8086, None);
const IMAGE: Kind = ('\u{f1c5}', 0xa074c4, None);
const ARCHIVE: Kind = ('\u{f1c6}', 0xb59f7b, None);
const LOCK: Kind = ('\u{f023}', 0x8a8a8a, None);

pub fn folder(open: bool) -> Icon {
    Icon {
        glyph: if open { FOLDER_OPEN } else { FOLDER },
        rgb: 0,
        folder: true,
        logo: None,
    }
}

/// A file's icon, by its name: special names first, then its extension, else a plain file.
pub fn file(name: &str) -> Icon {
    let lower = name.rsplit('/').next().unwrap_or(name).to_ascii_lowercase();
    let special = match lower.as_str() {
        "cargo.toml" | "cargo.lock" | "rust-toolchain" | "rust-toolchain.toml" => Some(RUST),
        "package.json" | "package-lock.json" | ".npmrc" => Some(NPM),
        ".gitignore" | ".gitattributes" | ".gitmodules" | ".gitkeep" => Some(GIT),
        "dockerfile"
        | ".dockerignore"
        | "docker-compose.yml"
        | "docker-compose.yaml"
        | "compose.yaml" => Some(DOCKER),
        "makefile" | "gnumakefile" | "justfile" => Some(MAKE),
        "license" | "license-mit" | "license-apache" | "licence" | "copying" => Some(LICENSE),
        _ if lower.starts_with("license") => Some(LICENSE),
        _ if lower.ends_with(".lock") || lower.ends_with("-lock.json") => Some(LOCK),
        _ => None,
    };
    let ext = lower.rsplit_once('.').map_or("", |(_, e)| e);
    let (glyph, rgb, logo) = special.unwrap_or(match ext {
        "rs" => RUST,
        "py" | "pyi" => ('\u{e73c}', 0x5a9fd4, Some(logo!("python"))),
        "ts" | "mts" | "cts" => ('\u{e628}', 0x519aba, Some(logo!("typescript-icon"))),
        "tsx" | "jsx" => ('\u{e7ba}', 0x61dafb, Some(logo!("react"))),
        "js" | "mjs" | "cjs" => ('\u{e74e}', 0xcbcb41, Some(logo!("javascript"))),
        "java" => ('\u{e738}', 0xcc3e44, Some(logo!("java"))),
        "kt" | "kts" => ('\u{e634}', 0xa074c4, Some(logo!("kotlin-icon"))),
        "rb" | "rake" | "gemspec" => ('\u{e739}', 0xcc3e44, Some(logo!("ruby"))),
        "md" | "markdown" | "mdx" => (
            '\u{e73e}',
            0x519aba,
            Some(logo!("markdown", Some(0x519aba))),
        ),
        "json" | "jsonc" => ('\u{e60b}', 0xcbcb41, None),
        "toml" => ('\u{e615}', 0x6d8086, Some(logo!("toml", Some(0x9c4221)))),
        "yaml" | "yml" => ('\u{e615}', 0x6d8086, Some(logo!("yaml", Some(0xcb171e)))),
        "ini" | "cfg" | "conf" | "env" => CONFIG,
        "html" | "htm" | "xhtml" => ('\u{e736}', 0xe37933, Some(logo!("html-5", Some(0xe34f26)))),
        "css" => ('\u{e749}', 0xa074c4, Some(logo!("css-3"))),
        "scss" | "sass" => ('\u{e749}', 0xf55385, Some(logo!("sass"))),
        "less" => ('\u{e749}', 0xf55385, Some(logo!("less"))),
        "sh" | "bash" | "zsh" | "fish" => ('\u{e795}', 0x8dc149, Some(logo!("bash-icon"))),
        "go" => ('\u{e627}', 0x519aba, Some(logo!("gopher"))),
        "c" | "h" => ('\u{e61e}', 0x599eff, Some(logo!("c"))),
        "cc" | "cpp" | "cxx" | "hpp" | "hh" => ('\u{e61d}', 0x599eff, Some(logo!("c-plusplus"))),
        "cs" => ('\u{f031b}', 0xa074c4, Some(logo!("c-sharp"))),
        "php" => ('\u{e73d}', 0xa074c4, Some(logo!("php"))),
        "lua" => ('\u{e620}', 0x519aba, Some(logo!("lua"))),
        "zig" => ('\u{e6a9}', 0xe37933, None),
        "swift" => ('\u{e755}', 0xe37933, Some(logo!("swift"))),
        "clj" | "cljs" | "cljc" | "edn" => ('\u{e768}', 0x8dc149, Some(logo!("clojure"))),
        "vue" => ('\u{e6a0}', 0x8dc149, Some(logo!("vue"))),
        "svelte" => ('\u{e697}', 0xff3e00, Some(logo!("svelte-icon"))),
        "sql" => ('\u{e706}', 0xdad8d8, None),
        "xml" => ('\u{e619}', 0xe37933, None),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "svg" | "tiff" => IMAGE,
        "pdf" => ('\u{f1c1}', 0xcc3e44, None),
        "zip" | "tar" | "gz" | "tgz" | "xz" | "bz2" | "7z" | "zst" => ARCHIVE,
        "txt" | "text" | "log" => ('\u{f15c}', 0x8a8a8a, None),
        _ => (FILE, 0x8a8a8a, None),
    });
    Icon {
        glyph,
        rgb,
        folder: false,
        logo,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn file_types_keep_single_cell_glyphs_and_readable_colors() {
        for (name, glyph) in [
            ("src/main.rs", '\u{e7a8}'),
            ("src/Cargo.toml", '\u{e7a8}'),
            ("run.py", '\u{e73c}'),
            ("README.MD", '\u{e73e}'),
            ("app.ts", '\u{e628}'),
            (".gitignore", '\u{e702}'),
            ("unknown", '\u{f15b}'),
        ] {
            let icon = file(name);
            assert_eq!(icon.glyph, glyph, "{name}");
            assert_eq!(icon.label(true, name).width(), name.width() + 2);
            assert!(matches!(icon.label(false, name), Cow::Borrowed(_)));
            for theme in theme::builtin_entries() {
                for bg in [
                    theme.palette.bg,
                    theme.palette.sidebar_bg,
                    theme.palette.selected_bg,
                ] {
                    assert!(
                        theme::contrast(icon.color_on(&theme.palette, bg), bg) >= 3.0,
                        "{} {name}",
                        theme.id
                    );
                }
            }
        }
    }
}
