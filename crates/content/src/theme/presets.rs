//! Palette inputs from Omarchy stock colors.toml (MIT, omacom/omarchy).
use super::{Palette, Syntax, ThemeEntry};
#[derive(Clone, Copy)]
pub(super) struct Seed {
    pub bg: u32,
    pub fg: u32,
    pub accent: u32,
    pub selection: u32,
    pub muted: u32,
    pub dark: u32,
    pub lighter: u32,
    pub red: u32,
    pub green: u32,
    pub blue: u32,
    pub yellow: u32,
    pub cyan: u32,
    pub magenta: u32,
}
/// `a` weighted `percent`% against `b`, per channel, in integers (palette derivation and faded diff colours).
pub const fn blend(a: u32, b: u32, percent: u32) -> u32 {
    let mut out = 0;
    let mut shift = 0;
    while shift <= 16 {
        out |= ((((a >> shift) & 255) * percent + ((b >> shift) & 255) * (100 - percent)) / 100)
            << shift;
        shift += 8;
    }
    out
}
pub(super) const fn from_seed(s: Seed) -> Palette {
    Palette {
        bg: s.bg,
        fg: s.fg,
        dim: s.muted,
        gutter_bg: s.dark,
        num_fg: s.muted,
        header_bg: s.lighter,
        sidebar_bg: s.dark,
        status_bg: s.lighter,
        border: blend(s.fg, s.bg, 25),
        selected_bg: s.selection,
        fold_bg: s.lighter,
        sel_bg: s.selection,
        caret: s.fg,
        found_bg: blend(s.yellow, s.bg, 25),
        banner_bg: blend(s.yellow, s.bg, 20),
        accent: s.accent,
        error: s.red,
        blame_new: blend(s.blue, s.bg, 30),
        block: [
            blend(s.green, s.bg, 18),
            blend(s.red, s.bg, 18),
            blend(s.blue, s.bg, 18),
            blend(s.yellow, s.bg, 18),
        ],
        inline: [
            blend(s.green, s.bg, 35),
            blend(s.red, s.bg, 35),
            blend(s.blue, s.bg, 35),
            blend(s.yellow, s.bg, 35),
        ],
        edge: [s.green, s.red, s.blue, s.yellow],
        status: [s.green, s.red, s.blue, s.magenta, s.red],
        syntax: Syntax {
            keyword: s.magenta,
            string: s.green,
            number: s.yellow,
            comment: s.muted,
            doc_comment: s.muted,
            function: s.blue,
            constant: s.yellow,
            macro_: s.cyan,
            type_: s.yellow,
            builtin_var: s.red,
            tag: s.blue,
            attribute: s.yellow,
            label: s.cyan,
            link: s.accent,
        },
    }
}
const CATPPUCCIN: Palette = from_seed(Seed {
    bg: 0x1e1e2e,
    fg: 0xcdd6f4,
    accent: 0x89b4fa,
    selection: 0x45475a,
    muted: 0x6c7086,
    dark: 0x161622,
    lighter: 0x313244,
    red: 0xf38ba8,
    green: 0xa6e3a1,
    blue: 0x89b4fa,
    yellow: 0xf9e2af,
    cyan: 0x94e2d5,
    magenta: 0xf5c2e7,
});
const LATTE: Palette = from_seed(Seed {
    bg: 0xeff1f5,
    fg: 0x4c4f69,
    accent: 0x1e66f5,
    selection: 0xccd0da,
    muted: 0x5c5f77,
    dark: 0xe3e4e8,
    lighter: 0xdce0e8,
    red: 0xd20f39,
    green: 0x40a02b,
    blue: 0x1e66f5,
    yellow: 0xdf8e1d,
    cyan: 0x179299,
    magenta: 0xea76cb,
});
const TOKYO: Palette = from_seed(Seed {
    bg: 0x1a1b26,
    fg: 0xa9b1d6,
    accent: 0x7aa2f7,
    selection: 0x292e42,
    muted: 0x565f89,
    dark: 0x13141c,
    lighter: 0x24283b,
    red: 0xf7768e,
    green: 0x9ece6a,
    blue: 0x7aa2f7,
    yellow: 0xe0af68,
    cyan: 0x449dab,
    magenta: 0xad8ee6,
});
const GRUVBOX: Palette = from_seed(Seed {
    bg: 0x282828,
    fg: 0xd4be98,
    accent: 0x7daea3,
    selection: 0x504945,
    muted: 0x7c6f64,
    dark: 0x1e1e1e,
    lighter: 0x3c3836,
    red: 0xea6962,
    green: 0xa9b665,
    blue: 0x7daea3,
    yellow: 0xd8a657,
    cyan: 0x89b482,
    magenta: 0xd3869b,
});
pub const BUILTINS: &[(&str, &str)] = &[
    ("darcula", "Darcula"),
    ("light", "Light"),
    ("catppuccin", "Catppuccin Mocha"),
    ("catppuccin-latte", "Catppuccin Latte"),
    ("tokyo-night", "Tokyo Night"),
    ("gruvbox", "Gruvbox"),
];
pub fn builtin(id: &str) -> Option<&'static Palette> {
    match id {
        "darcula" => Some(&super::DARCULA),
        "light" => Some(&super::LIGHT),
        "catppuccin" => Some(&CATPPUCCIN),
        "catppuccin-latte" => Some(&LATTE),
        "tokyo-night" => Some(&TOKYO),
        "gruvbox" => Some(&GRUVBOX),
        _ => None,
    }
}
pub fn builtin_entries() -> Vec<ThemeEntry> {
    BUILTINS
        .iter()
        .map(|(id, label)| ThemeEntry {
            id: (*id).into(),
            label: (*label).into(),
            palette: std::sync::Arc::new(*builtin(id).unwrap()),
        })
        .collect()
}
