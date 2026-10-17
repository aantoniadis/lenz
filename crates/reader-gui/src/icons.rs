//! File and folder icons for the tree and the tabs. Languages and tools with a logo get it: colored SVGs from
//! Devicons (<https://devicons.io>, MIT, `assets/devicons/LICENSE`), compiled in and drawn at [`SIZE`]. Everything
//! else (folders, images, archives, plain files) gets a Nerd Font glyph and a colour, folders in the theme's accent;
//! without a Nerd Font installed the reader draws a small coloured shape instead, so no missing-glyph boxes appear.

use gpui::Font;
use lenz_content::theme::{self, Palette};

/// What a file or folder looks like in the tree and the tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Icon {
    pub glyph: char,
    /// `0xRRGGBB`, for a dark background (see [`Icon::color`]).
    pub rgb: u32,
    pub folder: bool,
    /// Drawn instead of the glyph.
    pub logo: Option<Logo>,
}

/// Logical pixels an icon takes, a little larger than the text beside it.
pub const SIZE: f32 = 16.0;
/// The font size of a folder's glyph in a Nerd Font Mono, whose glyphs are one cell (0.6 em) wide: drawn this large
/// it is as wide as the logos ([`SIZE`]), so folders and files keep the same gap before their names. Other Nerd Fonts
/// draw glyphs about an em wide and keep [`SIZE`].
pub const FOLDER_SIZE: f32 = 26.0;

/// The size to draw `icon`'s glyph at in Nerd Font family `family`.
pub fn glyph_size(icon: Icon, family: &str) -> f32 {
    let mono = family.contains("Nerd Font Mono") || family.ends_with(" NFM");
    if icon.folder && mono { FOLDER_SIZE } else { SIZE }
}

/// A compiled-in Devicons SVG.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Logo {
    pub name: &'static str,
    svg: &'static [u8],
    /// Its black parts in this colour, so they show on dark themes too.
    tint: Option<u32>,
}

macro_rules! logo {
    ($name:literal) => {
        logo!($name, None)
    };
    ($name:literal, $tint:expr) => {
        Logo { name: $name, svg: include_bytes!(concat!("../assets/devicons/", $name, ".svg")), tint: $tint }
    };
}

impl Logo {
    /// Drawn `px` pixels square (straight alpha), black parts tinted; `None` if the SVG doesn't parse.
    pub fn rasterize(self, px: u32) -> Option<lenz_content::image::Pixels> {
        use resvg::{tiny_skia, usvg};
        let tree = usvg::Tree::from_data(self.svg, &usvg::Options::default()).ok()?;
        let mut pixmap = tiny_skia::Pixmap::new(px, px)?;
        let size = tree.size();
        let scale = px as f32 / size.width().max(size.height());
        let (dx, dy) = ((px as f32 - size.width() * scale) / 2.0, (px as f32 - size.height() * scale) / 2.0);
        resvg::render(&tree, tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, dx, dy), &mut pixmap.as_mut());
        let mut out = lenz_content::image::Pixels::new(px, px);
        for (o, c) in out.pixels_mut().zip(pixmap.pixels()) {
            let c = c.demultiply();
            let (r, g, b) = match self.tint {
                Some(t) if c.red().max(c.green()).max(c.blue()) < 0x30 => ((t >> 16) as u8, (t >> 8) as u8, t as u8),
                _ => (c.red(), c.green(), c.blue()),
            };
            o.0 = [r, g, b, c.alpha()];
        }
        Some(out)
    }
}

impl Icon {
    /// Its colour on this theme: brand colours as they are on dark themes, darkened to stay readable on light ones.
    pub fn color(self, p: &Palette) -> u32 {
        if self.folder {
            p.accent
        } else if theme::luminance(p.bg) >= 0.5 {
            theme::mix(self.rgb, 0x000000, 0.3)
        } else {
            self.rgb
        }
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
    Icon { glyph: if open { FOLDER_OPEN } else { FOLDER }, rgb: 0, folder: true, logo: None }
}

/// A file's icon, by its name: special names first, then its extension, else a plain file.
pub fn file(name: &str) -> Icon {
    let lower = name.to_ascii_lowercase();
    let special = match lower.as_str() {
        "cargo.toml" | "cargo.lock" | "rust-toolchain" | "rust-toolchain.toml" => Some(RUST),
        "package.json" | "package-lock.json" | ".npmrc" => Some(NPM),
        ".gitignore" | ".gitattributes" | ".gitmodules" | ".gitkeep" => Some(GIT),
        "dockerfile" | ".dockerignore" | "docker-compose.yml" | "docker-compose.yaml" | "compose.yaml" => Some(DOCKER),
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
        "md" | "markdown" | "mdx" => ('\u{e73e}', 0x519aba, Some(logo!("markdown", Some(0x519aba)))),
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
    Icon { glyph, rgb, folder: false, logo }
}

/// An installed Nerd Font to draw glyphs with: a "Nerd Font Mono" family first (single-width glyphs), else any
/// Nerd Font; `None` when there is none (shapes are drawn instead).
pub fn nerd_font(installed: &[String]) -> Option<Font> {
    let pick = |want: &dyn Fn(&str) -> bool| installed.iter().find(|f| want(f)).cloned();
    pick(&|f| f.contains("Nerd Font Mono") || f.ends_with(" NFM"))
        .or_else(|| pick(&|f| f.contains("Nerd Font") || f.ends_with(" NF")))
        .map(gpui::font)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_get_icons_by_special_name_then_extension_and_folders_open_or_closed() {
        assert_eq!(file("main.rs").glyph, '\u{e7a8}');
        assert_eq!(file("Cargo.toml").glyph, '\u{e7a8}', "a Cargo manifest is Rust, not generic configuration");
        assert_eq!(file("config.toml").glyph, '\u{e615}');
        assert_eq!(file("package.json").glyph, '\u{e71e}');
        assert_eq!(file("data.json").glyph, '\u{e60b}');
        assert_eq!(file("README.MD").glyph, '\u{e73e}', "extensions in any case");
        assert_eq!(file("view.tsx").glyph, file("view.jsx").glyph);
        assert_eq!(file(".gitignore").glyph, '\u{e702}');
        assert_eq!(file("yarn.lock").glyph, '\u{f023}');
        assert_eq!(file("LICENSE-MIT").glyph, '\u{f02d}');
        assert_eq!(file("logo.svg").glyph, file("photo.JPEG").glyph);
        assert_eq!(file("noextension").glyph, '\u{f15b}');
        assert_eq!(file("weird.zzz").glyph, '\u{f15b}');
        assert!(folder(true).folder && folder(true).glyph != folder(false).glyph);
        assert_eq!(glyph_size(folder(false), "JetBrainsMono Nerd Font Mono"), FOLDER_SIZE, "a folder fills the logos' width");
        assert_eq!(glyph_size(folder(true), "JetBrainsMono NFM"), FOLDER_SIZE);
        assert_eq!(glyph_size(folder(false), "JetBrainsMono Nerd Font"), SIZE, "an em-wide glyph would run into the name");
        assert_eq!(glyph_size(file("notes.txt"), "JetBrainsMono Nerd Font Mono"), SIZE);
        // Folders take the theme's accent; files keep their colour on dark themes and darken on light ones.
        let (dark, light) = (&theme::DARCULA, &theme::LIGHT);
        assert_eq!(folder(false).color(dark), dark.accent);
        assert_eq!(file("main.rs").color(dark), 0xdea584);
        assert_ne!(file("main.rs").color(light), 0xdea584);
        assert!(theme::luminance(file("app.js").color(light)) < theme::luminance(file("app.js").color(dark)));
    }

    #[test]
    fn languages_and_tools_get_their_logo_drawn_at_any_size_with_black_parts_tinted() {
        assert_eq!(file("main.rs").logo.map(|l| l.name), Some("rust"));
        assert_eq!(file("Cargo.toml").logo.map(|l| l.name), Some("rust"));
        assert_eq!(file("App.TSX").logo.map(|l| l.name), Some("react"));
        assert_eq!(file("Dockerfile").logo.map(|l| l.name), Some("docker-icon"));
        assert_eq!(file("notes.txt").logo, None, "plain kinds keep their glyph");
        assert_eq!(folder(true).logo, None);
        // Every logo parses and draws something at the sizes a display asks for.
        for name in ["a.rs", "a.py", "a.ts", "a.tsx", "a.js", "a.java", "a.kt", "a.rb", "a.md", "a.toml", "a.yaml", "a.html", "a.css", "a.scss", "a.less", "a.sh", "a.go", "a.c", "a.cpp", "a.cs", "a.php", "a.lua", "a.swift", "a.clj", "a.vue", "a.svelte", "package.json", ".gitignore", "Dockerfile", "Makefile"] {
            for px in [16, 32] {
                let pixels = file(name).logo.and_then(|l| l.rasterize(px)).unwrap_or_else(|| panic!("{name} at {px}"));
                assert_eq!(pixels.dimensions(), (px, px));
                assert!(pixels.pixels().filter(|p| p.0[3] > 0).count() as u32 > px * px / 8, "{name} draws too little at {px}");
            }
        }
        // Rust's logo is black: tinted, so no opaque pixel stays black on a dark theme.
        let rust = file("main.rs").logo.unwrap().rasterize(32).unwrap();
        assert!(rust.pixels().filter(|p| p.0[3] > 0x80).all(|p| p.0[..3].iter().any(|&c| c >= 0x30)));
    }

    #[test]
    fn a_nerd_font_mono_is_preferred_and_none_means_shapes() {
        let installed = |names: &[&str]| names.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(nerd_font(&installed(&["Inter", "CaskaydiaMono Nerd Font", "CaskaydiaMono Nerd Font Mono"])).map(|f| f.family.to_string()), Some("CaskaydiaMono Nerd Font Mono".into()));
        assert_eq!(nerd_font(&installed(&["JetBrainsMono NF"])).map(|f| f.family.to_string()), Some("JetBrainsMono NF".into()));
        assert_eq!(nerd_font(&installed(&["Inter", "DejaVu Sans Mono"])), None);
    }
}
