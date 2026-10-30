//! Themes shared by Lenz and its code reader: palettes, built-in themes, the user theme format, the saved
//! selection and the service that loads, saves and follows them. Reader and Diff share canonical Lenz paths
//! and overrides ([`ThemePaths::shared`], [`ThemeOverrides::shared_from_env`]); diff colour mapping and GPUI colour
//! conversion stay with the products.
//!
//! Colours of IntelliJ's Darcula and IntelliJ Light schemes (JetBrains intellij-community, Apache-2.0):
//! editor, diff and syntax colours from the schemes; chrome colours from the IDE themes, approximated.
//! Inline Changes use a stronger shade of each diff colour than the scheme's line colour.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub bg: u32,
    pub fg: u32,
    pub dim: u32,
    pub gutter_bg: u32,
    pub num_fg: u32,
    pub header_bg: u32,
    pub sidebar_bg: u32,
    /// Overlays: toasts, prompts, menus, popups.
    pub status_bg: u32,
    pub border: u32,
    /// The current row in lists (File Navigator, switcher, results).
    pub selected_bg: u32,
    pub fold_bg: u32,
    pub sel_bg: u32,
    pub caret: u32,
    pub found_bg: u32,
    pub banner_bg: u32,
    /// Commit hashes and other highlights in popups.
    pub accent: u32,
    pub error: u32,
    /// Blame Annotation shade of the newest commit (the oldest has none).
    pub blame_new: u32,
    /// Change Block lines, Inline Changes and Connector edges: inserted, deleted, modified, unresolved Conflict.
    pub block: [u32; 4],
    pub inline: [u32; 4],
    pub edge: [u32; 4],
    /// File status: added, deleted, modified, renamed, conflicted.
    pub status: [u32; 5],
    pub syntax: Syntax,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Syntax {
    pub keyword: u32,
    pub string: u32,
    pub number: u32,
    pub comment: u32,
    pub doc_comment: u32,
    pub function: u32,
    pub constant: u32,
    pub macro_: u32,
    pub type_: u32,
    pub builtin_var: u32,
    pub tag: u32,
    pub attribute: u32,
    pub label: u32,
    pub link: u32,
}

pub const DARCULA: Palette = Palette {
    bg: 0x2B2B2B,
    fg: 0xA9B7C6,
    dim: 0x787878,
    gutter_bg: 0x313335,
    num_fg: 0x606366,
    header_bg: 0x313335,
    sidebar_bg: 0x2F3133,
    status_bg: 0x3C3F41,
    border: 0x555555,
    selected_bg: 0x0D293E,
    fold_bg: 0x353739,
    sel_bg: 0x214283,
    caret: 0xBBBBBB,
    found_bg: 0x32593D,
    banner_bg: 0x5C4B1E,
    accent: 0xCC7832,
    error: 0xD25252,
    blame_new: 0x4E6A8C,
    block: [0x294436, 0x484A4A, 0x385570, 0x45302B],
    inline: [0x3C6B48, 0x656E76, 0x4E78A8, 0x6B3C35],
    edge: [0x5C9A6C, 0x8A8F94, 0x6A93C8, 0xC75450],
    status: [0x629755, 0x8C8C8C, 0x6897BB, 0x9876AA, 0xD25252],
    syntax: Syntax {
        keyword: 0xCC7832,
        string: 0x6A8759,
        number: 0x6897BB,
        comment: 0x808080,
        doc_comment: 0x629755,
        function: 0xFFC66D,
        constant: 0x9876AA,
        macro_: 0x4EADE5,
        type_: 0x6FAFBD,
        builtin_var: 0x94558D,
        tag: 0xE8BF6A,
        attribute: 0xBBB529,
        label: 0x20999D,
        link: 0x287BDE,
    },
};

pub const LIGHT: Palette = Palette {
    bg: 0xFFFFFF,
    fg: 0x080808,
    dim: 0x8C8C8C,
    gutter_bg: 0xF5F5F5,
    num_fg: 0x999999,
    header_bg: 0xF2F2F2,
    sidebar_bg: 0xF7F8FA,
    status_bg: 0xEBECF0,
    border: 0xC9CCD6,
    selected_bg: 0xD5E1FF,
    fold_bg: 0xEDEEF0,
    sel_bg: 0xA6D2FF,
    caret: 0x000000,
    found_bg: 0xFFE59E,
    banner_bg: 0xFCEFC2,
    accent: 0x0033B3,
    error: 0xC7222D,
    blame_new: 0xC6DAF7,
    block: [0xBAEEBA, 0xCBCBCB, 0xBCCFF9, 0xFFD5CC],
    inline: [0x8FDB8F, 0xA9A9A9, 0x8EAEF5, 0xFFB0A1],
    edge: [0x56A556, 0x999999, 0x6F95E6, 0xE0614F],
    status: [0x067D17, 0x6C6C6C, 0x0032A0, 0x871094, 0xC7222D],
    syntax: Syntax {
        keyword: 0x0033B3,
        string: 0x067D17,
        number: 0x1750EB,
        comment: 0x8C8C8C,
        doc_comment: 0x8C8C8C,
        function: 0x00627A,
        constant: 0x871094,
        macro_: 0x1F542E,
        type_: 0x000000,
        builtin_var: 0x871094,
        tag: 0x0033B3,
        attribute: 0x9E880D,
        label: 0x2F8C8C,
        link: 0x2A5DB0,
    },
};

impl Palette {
pub fn syntax_color(&self, capture: crate::highlight::Capture) -> u32 {
    use crate::highlight::Capture;
    let c = &self.syntax;
    match capture {
        Capture::Keyword | Capture::ConstantBuiltin | Capture::TypeBuiltin | Capture::Escape | Capture::Heading | Capture::Strong => c.keyword,
        Capture::String => c.string,
        Capture::Comment => c.comment,
        Capture::DocComment => c.doc_comment,
        Capture::Number => c.number,
        Capture::Constant | Capture::Property | Capture::Emphasis => c.constant,
        Capture::Function => c.function,
        Capture::Macro => c.macro_,
        Capture::Type => c.type_,
        Capture::VariableBuiltin => c.builtin_var,
        Capture::Tag => c.tag,
        Capture::Attribute => c.attribute,
        Capture::Label => c.label,
        Capture::Link => c.link,
        Capture::Variable | Capture::Parameter | Capture::Operator | Capture::Punctuation | Capture::Module => self.fg,
    }
}

/// `color` for text on `fill` when it stays readable there (3:1, WCAG's floor for large text and UI parts), else the
/// plain foreground: a changed word keeps its syntax color on its highlight unless that color would all but vanish.
pub fn readable_on(&self, color: u32, fill: u32) -> u32 {
    if contrast(color, fill) >= 3.0 { color } else { self.fg }
}

/// Popups, the palette and menus: a background lifted off the toolbar and panes, and a border that reads on it.
pub fn raised(&self) -> (u32, u32) {
    // dark themes lift the popup off the toolbar; light ones use white and let the border carry the edge
    let bg = if luminance(self.bg) < 0.4 { mix(self.header_bg.max_by_luminance(self.status_bg), 0xffffff, 0.04) } else { 0xffffff };
    (bg, mix(self.fg, bg, 0.45))
}

}

/// `a` moved `t` of the way to `b`, per channel.
pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let channel = |shift: u32| {
        let (x, y) = (((a >> shift) & 0xff) as f32, ((b >> shift) & 0xff) as f32);
        ((x + (y - x) * t).round().clamp(0.0, 255.0) as u32) << shift
    };
    channel(16) | channel(8) | channel(0)
}

/// WCAG contrast ratio of two colors, 1 to 21.
pub fn contrast(a: u32, b: u32) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// Relative luminance (WCAG), 0 to 1.
pub fn luminance(color: u32) -> f32 {
    let linear = |c: u32| { let c = c as f32 / 255.0; if c <= 0.039_28 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) } };
    0.2126 * linear((color >> 16) & 0xff) + 0.7152 * linear((color >> 8) & 0xff) + 0.0722 * linear(color & 0xff)
}

trait Brighter { fn max_by_luminance(self, other: u32) -> u32; }
impl Brighter for u32 { fn max_by_luminance(self, other: u32) -> u32 { if luminance(other) > luminance(self) { other } else { self } } }

#[cfg(test)]
mod tests;

mod loader;
mod presets;
pub use loader::{LoadedTheme, ThemeError, ThemeInputKind, load_theme, parse_theme};
pub use loader::read_bounded;
pub use presets::{blend, builtin, builtin_entries};
#[derive(Clone, Debug)]
pub struct ThemeEntry { pub id: String, pub label: String, pub palette: std::sync::Arc<Palette> }
mod controller;
mod store;
pub use controller::{ThemeCommand,ThemeSnapshot,ThemeEffect};
pub use store::{ThemePaths,ThemeOverrides,ThemeSelection,discover_themes};
#[cfg(test)]
use controller::ThemeController;
#[cfg(test)]
use store::{read_selection,save_selection,valid_id};
mod reading;
mod watch;
pub use reading::{callout_color, callout_icon, highlight_bg, nearest_hue, removed_color};
mod service;
pub use service::{ThemeService,ThemeUpdate};
