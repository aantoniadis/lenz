//! The optional settings file, `$XDG_CONFIG_HOME/lenz/settings.toml`: read once at startup. Lenz writes it only to
//! create the commented template when **Settings…** finds no file.
//! Every problem becomes a notice and that key's default; nothing in the file can stop Lenz from starting.
//! Precedence (flag, then environment, then this file, then the default) is applied by each consumer.
use std::{path::{Path, PathBuf}, sync::OnceLock};

use lenz_engine::{HighlightLevel, WhitespaceMode};

use crate::{markdown::PresentationMode, view::ScrollMode};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Settings { pub gui: Gui, pub view: View, pub tools: Tools, pub session: SessionSettings }

pub use lenz_content::appearance::FontPreferences as Gui;

#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub whitespace: WhitespaceMode,
    pub ignore_case: bool,
    pub highlight: HighlightLevel,
    pub scroll: ScrollMode,
    pub blame: bool,
    pub markdown: PresentationMode,
    pub markdown_layout: crate::markdown::MarkdownLayout,
    /// Start in the one-column Unified layout (GUI).
    pub unified: bool,
    /// Pin the enclosing declarations' first lines at the top of each pane.
    pub sticky_scroll: bool,
}
impl Default for View {
    fn default() -> Self {
        View {
            whitespace: WhitespaceMode::default(),
            ignore_case: false,
            highlight: HighlightLevel::default(),
            scroll: ScrollMode::default(),
            blame: false,
            markdown: PresentationMode::default(),
            markdown_layout: crate::markdown::MarkdownLayout::default(),
            unified: false,
            sticky_scroll: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icons { Plain, Nerd }

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Tools { pub editor: Option<String>, pub icons: Option<Icons>, pub crash_dir: Option<PathBuf> }

#[derive(Debug, Clone, PartialEq)]
pub struct SessionSettings { pub restore_place: bool }
impl Default for SessionSettings { fn default() -> Self { SessionSettings { restore_place: true } } }

pub use lenz_content::appearance::{FONT_SIZES, path};

/// Every setting, commented out: what **Settings…** creates when there is no file yet.
pub const TEMPLATE: &str = "\
# Lenz settings. Uncomment a line to change it; Lenz reads this file at startup.

[gui]
# font_family = \"JetBrains Mono\"   # default: first installed of a built-in monospace list
# font_size = 13                   # 8–32
# reading_font = \"Inter\"          # Markdown Preview text; default: first installed of a built-in sans list

[view]                             # starting options of each new comparison
# whitespace = \"default\"           # default | trim | ignore | ignore-chunks
# ignore_case = false
# highlight = \"word\"               # word | char | line
# scroll = \"shared\"                # shared | aligned | padded | independent
# blame = false
# markdown = \"source\"              # source | preview
# markdown_layout = \"inline\"       # inline | side-by-side (Preview)
# layout = \"split\"                # split | unified (GUI)
# sticky_scroll = true             # pin the enclosing declarations at the top of each pane

[tools]
# editor = \"nvim --clean\"
# icons = \"plain\"                  # nerd (default) | plain (TUI)
# crash_dir = \"~/lenz-crashes\"

[session]
# restore_place = true
";

/// The settings file, created from [`TEMPLATE`] when missing; an existing file is never touched.
pub fn ensure_template() -> std::io::Result<PathBuf> {
    let path = path().ok_or_else(|| std::io::Error::other("no HOME or XDG_CONFIG_HOME"))?;
    ensure_template_at(&path)?;
    Ok(path)
}

pub fn ensure_template_at(path: &Path) -> std::io::Result<()> {
    if let Some(dir) = path.parent() { std::fs::create_dir_all(dir)?; }
    match std::fs::OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => std::io::Write::write_all(&mut file, TEMPLATE.as_bytes()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}

/// The file as Lenz runs with it; a missing file is the defaults without a notice.
pub fn load() -> (Settings, Vec<String>) {
    let Some(path) = path() else { return Default::default() };
    if std::fs::symlink_metadata(&path).is_err() {
        return Default::default();
    }
    match crate::theme::read_bounded(&path) {
        Ok(bytes) => match std::str::from_utf8(&bytes) {
            Ok(text) => parse(text, std::env::var_os("HOME").map(PathBuf::from).as_deref()),
            Err(_) => (Settings::default(), vec![format!("{}: not UTF-8; using defaults", path.display())]),
        },
        Err(e) => (Settings::default(), vec![format!("{}: {e}; using defaults", path.display())]),
    }
}

static CURRENT: OnceLock<Settings> = OnceLock::new();
static NOTICES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Keeps `notices` (from [`load`]) until a frontend has somewhere to show them.
pub fn stash_notices(notices: Vec<String>) {
    NOTICES.lock().unwrap_or_else(std::sync::PoisonError::into_inner).extend(notices);
}

/// The stashed notices, once.
pub fn take_notices() -> Vec<String> {
    std::mem::take(&mut *NOTICES.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
}

/// The stashed notices as one line, once: both frontends show a single notice at a time.
pub fn take_notice() -> Option<String> {
    let notices = take_notices();
    (!notices.is_empty()).then(|| notices.join(" · "))
}

/// Sets the process's settings; the first call wins (startup).
pub fn init(settings: Settings) {
    let _ = CURRENT.set(settings);
}

/// The process's settings; the defaults until [`init`].
pub fn get() -> &'static Settings {
    static DEFAULT: OnceLock<Settings> = OnceLock::new();
    CURRENT.get().unwrap_or_else(|| DEFAULT.get_or_init(Settings::default))
}

/// `text` as settings; `home` expands a leading `~` in paths.
pub fn parse(text: &str, home: Option<&Path>) -> (Settings, Vec<String>) {
    let mut s = Settings::default();
    let mut notices = Vec::new();
    let table = match text.parse::<toml::Table>() {
        Ok(t) => t,
        Err(e) => return (s, vec![format!("settings.toml: {}; using defaults", e.message())]),
    };
    for (section, value) in &table {
        let Some(fields) = value.as_table() else {
            notices.push(format!("settings.toml: `{section}` should be a [section]; ignored"));
            continue;
        };
        for (key, value) in fields {
            let name = format!("{section}.{key}");
            let mut bad = |why: &str| notices.push(format!("settings.toml: {name} {why}; using the default"));
            let text = || value.as_str();
            let flag = || value.as_bool();
            match (section.as_str(), key.as_str()) {
                ("gui", "font_family" | "reading_font" | "font_size") => {
                    if let Some(Err(why)) = s.gui.apply_field(key, value) { bad(why); }
                },
                ("view", "whitespace") => match text() {
                    Some("default") => s.view.whitespace = WhitespaceMode::Default,
                    Some("trim") => s.view.whitespace = WhitespaceMode::Trim,
                    Some("ignore") => s.view.whitespace = WhitespaceMode::Ignore,
                    Some("ignore-chunks") => s.view.whitespace = WhitespaceMode::IgnoreTrimChunks,
                    _ => bad("must be default, trim, ignore or ignore-chunks"),
                },
                ("view", "ignore_case") => match flag() { Some(b) => s.view.ignore_case = b, None => bad("must be true or false") },
                ("view", "highlight") => match text() {
                    Some("word") => s.view.highlight = HighlightLevel::Word,
                    Some("char") => s.view.highlight = HighlightLevel::Char,
                    Some("line") => s.view.highlight = HighlightLevel::Line,
                    _ => bad("must be word, char or line"),
                },
                ("view", "scroll") => match text() {
                    Some("shared") => s.view.scroll = ScrollMode::Shared,
                    Some("aligned") => s.view.scroll = ScrollMode::Aligned,
                    Some("padded") => s.view.scroll = ScrollMode::Padded,
                    Some("independent") => s.view.scroll = ScrollMode::Independent,
                    _ => bad("must be shared, aligned, padded or independent"),
                },
                ("view", "blame") => match flag() { Some(b) => s.view.blame = b, None => bad("must be true or false") },
                ("view", "sticky_scroll") => match flag() { Some(b) => s.view.sticky_scroll = b, None => bad("must be true or false") },
                ("view", "layout") => match text() {
                    Some("split") => s.view.unified = false,
                    Some("unified") => s.view.unified = true,
                    _ => bad("must be \"split\" or \"unified\""),
                },
                ("view", "markdown") => match text() {
                    Some("source") => s.view.markdown = PresentationMode::Source,
                    Some("preview" | "rendered") => s.view.markdown = PresentationMode::Rendered,
                    _ => bad("must be source or preview"),
                },
                ("view", "markdown_layout") => match text() {
                    Some("inline") => s.view.markdown_layout = crate::markdown::MarkdownLayout::Inline,
                    Some("side-by-side") => s.view.markdown_layout = crate::markdown::MarkdownLayout::SideBySide,
                    _ => bad("must be inline or side-by-side"),
                },
                ("tools", "editor") => match text().map(str::trim).filter(|t| !t.is_empty()) {
                    Some(t) => s.tools.editor = Some(t.to_string()),
                    None => bad("must be a command"),
                },
                ("tools", "icons") => match text() {
                    Some("plain") => s.tools.icons = Some(Icons::Plain),
                    Some("nerd") => s.tools.icons = Some(Icons::Nerd),
                    _ => bad("must be plain or nerd"),
                },
                ("tools", "crash_dir") => match text().and_then(|t| expand(t, home)) {
                    Some(p) => s.tools.crash_dir = Some(p),
                    None => bad("must be an absolute path or start with ~/"),
                },
                ("session", "restore_place") => match flag() { Some(b) => s.session.restore_place = b, None => bad("must be true or false") },
                _ => notices.push(format!("settings.toml: unknown setting {name}; ignored")),
            }
        }
    }
    (s, notices)
}

fn expand(path: &str, home: Option<&Path>) -> Option<PathBuf> {
    match path.strip_prefix("~/") {
        Some(rest) => home.map(|h| h.join(rest)),
        None => Some(PathBuf::from(path)).filter(|p| p.is_absolute()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn template_parses_to_the_defaults_and_is_created_once() {
        let (parsed, notices) = parse(TEMPLATE, None);
        assert!(notices.is_empty(), "{notices:?}");
        assert_eq!(format!("{parsed:?}"), format!("{:?}", Settings::default()));
        assert!(parsed.view.sticky_scroll, "sticky scroll is on by default");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lenz/settings.toml");
        ensure_template_at(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), TEMPLATE);
        std::fs::write(&path, "[view]\nblame = true\n").unwrap();
        ensure_template_at(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[view]\nblame = true\n", "an existing file is kept");
    }

    #[test]
    fn markdown_layout_defaults_to_inline_and_rejects_unknown_values() {
        assert_eq!(parse("", None).0.view.markdown_layout, crate::markdown::MarkdownLayout::Inline);
        let (s, notices) = parse("[view]\nmarkdown_layout = \"stacked\"\n", None);
        assert_eq!(s.view.markdown_layout, crate::markdown::MarkdownLayout::Inline);
        assert!(notices.iter().any(|n| n.contains("view.markdown_layout")));
    }

    #[test]
    fn markdown_accepts_preview_and_the_old_rendered() {
        for value in ["preview", "rendered"] {
            let (s, notices) = parse(&format!("[view]\nmarkdown = \"{value}\"\n"), None);
            assert!(notices.is_empty());
            assert_eq!(s.view.markdown, PresentationMode::Rendered);
        }
    }

    #[test]
    fn scroll_accepts_all_four_modes() {
        for (value, mode) in [("shared", ScrollMode::Shared), ("aligned", ScrollMode::Aligned), ("padded", ScrollMode::Padded), ("independent", ScrollMode::Independent)] {
            assert_eq!(parse(&format!("[view]\nscroll = \"{value}\"\n"), None).0.view.scroll, mode);
        }
    }

    use super::*;
    use lenz_engine::{HighlightLevel, WhitespaceMode};

    #[test]
    fn a_missing_or_empty_file_is_all_defaults_without_notices() {
        let (s, notices) = parse("", None);
        assert_eq!(s, Settings::default());
        assert!(notices.is_empty());
        assert_eq!(s.gui.font_size, 13.0);
        assert!(s.session.restore_place);
    }

    #[test]
    fn every_documented_key_is_read() {
        let (s, notices) = parse(r#"
            [gui]
            font_family = "Iosevka"
            font_size = 15
            reading_font = "Inter"
            [view]
            whitespace = "ignore-chunks"
            ignore_case = true
            highlight = "char"
            scroll = "padded"
            blame = true
            layout = "unified"
            sticky_scroll = false
            markdown = "rendered"
            markdown_layout = "side-by-side"
            [tools]
            editor = "nvim --clean"
            icons = "nerd"
            crash_dir = "~/crashes"
            [session]
            restore_place = false
        "#, Some(Path::new("/home/u")));
        assert!(notices.is_empty(), "{notices:?}");
        assert_eq!((s.gui.font_family.as_deref(), s.gui.font_size), (Some("Iosevka"), 15.0));
        assert_eq!(s.gui.reading_font.as_deref(), Some("Inter"));
        assert!(s.view.unified, "layout = unified");
        assert!(!s.view.sticky_scroll, "sticky_scroll = false");
        assert_eq!((s.view.whitespace, s.view.ignore_case, s.view.highlight), (WhitespaceMode::IgnoreTrimChunks, true, HighlightLevel::Char));
        assert_eq!((s.view.scroll, s.view.blame, s.view.markdown), (crate::view::ScrollMode::Padded, true, crate::markdown::PresentationMode::Rendered));
        assert_eq!((s.tools.editor.as_deref(), s.tools.icons), (Some("nvim --clean"), Some(Icons::Nerd)));
        assert_eq!(s.tools.crash_dir.as_deref(), Some(Path::new("/home/u/crashes")));
        assert_eq!(s.view.markdown_layout, crate::markdown::MarkdownLayout::SideBySide);
        assert!(!s.session.restore_place);
    }

    #[test]
    fn a_bad_value_falls_back_for_that_key_only_and_says_so() {
        let (s, notices) = parse("[gui]\nfont_size = \"big\"\nfont_family = \"Mono\"\n[view]\nhighlight = \"paragraph\"\n", None);
        assert_eq!((s.gui.font_size, s.gui.font_family.as_deref(), s.view.highlight), (13.0, Some("Mono"), HighlightLevel::Word));
        assert_eq!(notices.len(), 2, "{notices:?}");
        assert!(notices.iter().any(|n| n.contains("gui.font_size")) && notices.iter().any(|n| n.contains("view.highlight")));
        let (s, notices) = parse("[gui]\nfont_size = 200\n", None);
        assert_eq!(s.gui.font_size, 13.0);
        assert!(notices[0].contains("8") && notices[0].contains("32"), "{notices:?}");
    }

    #[test]
    fn unknown_keys_and_sections_are_named_and_ignored() {
        let (s, notices) = parse("[gui]\nfnt_size = 15\n[colours]\nx = 1\n", None);
        assert_eq!(s, Settings::default());
        assert!(notices.iter().any(|n| n.contains("gui.fnt_size")) && notices.iter().any(|n| n.contains("colours")), "{notices:?}");
    }

    #[test]
    fn invalid_toml_is_all_defaults_and_one_notice() {
        let (s, notices) = parse("[gui\nfont_size = ", None);
        assert_eq!(s, Settings::default());
        assert_eq!(notices.len(), 1);
    }

    #[test]
    fn crash_dir_must_be_absolute_or_start_with_tilde() {
        let (s, notices) = parse("[tools]\ncrash_dir = \"relative/dir\"\n", Some(Path::new("/home/u")));
        assert_eq!(s.tools.crash_dir, None);
        assert!(notices[0].contains("tools.crash_dir"));
        let (s, _) = parse("[tools]\ncrash_dir = \"~/x\"\n", None);
        assert_eq!(s.tools.crash_dir, None, "no HOME, no ~");
    }

    #[test]
    fn stashed_notices_are_taken_once() {
        stash_notices(vec!["one".into()]);
        assert_eq!(take_notices(), ["one"]);
        assert!(take_notices().is_empty());
    }

    #[test]
    fn all_stashed_notices_become_one_line() {
        stash_notices(vec!["first".into(), "second".into()]);
        stash_notices(vec!["font".into()]);
        assert_eq!(take_notice().as_deref(), Some("first · second · font"));
        assert_eq!(take_notice(), None);
    }

    #[test]
    fn get_before_init_is_the_defaults() {
        assert_eq!(get().gui.font_size, 13.0);
    }
}
