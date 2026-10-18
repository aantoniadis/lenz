//! Single-cell Nerd Font Mono decorations shared by the terminal adapters.
use std::borrow::Cow;

#[derive(Clone, Copy)]
pub enum Icon {
    File,
    Folder,
    FolderOpen,
    Branch,
    History,
    Search,
    Commands,
    Help,
    Book,
    Position,
}

impl Icon {
    pub fn label(self, nerd: bool, text: &str) -> Cow<'_, str> {
        if !nerd {
            return Cow::Borrowed(text);
        }
        let glyph = match self {
            Self::File => '\u{f15b}',
            Self::Folder => '\u{f07b}',
            Self::FolderOpen => '\u{f07c}',
            Self::Branch => '\u{f126}',
            Self::History => '\u{f1da}',
            Self::Search => '\u{f002}',
            Self::Commands => '\u{f120}',
            Self::Help => '\u{f059}',
            Self::Book => '\u{f02d}',
            Self::Position => '\u{f041}',
        };
        format!("{glyph} {text}").into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn decorations_add_two_cells_and_preserve_unicode_labels() {
        for icon in [
            Icon::File,
            Icon::Folder,
            Icon::FolderOpen,
            Icon::Branch,
            Icon::History,
            Icon::Search,
            Icon::Commands,
            Icon::Help,
            Icon::Book,
            Icon::Position,
        ] {
            let plain = icon.label(false, "é界.rs");
            assert!(matches!(plain, Cow::Borrowed("é界.rs")));
            let nerd = icon.label(true, "é界.rs");
            assert!(nerd.ends_with(" é界.rs"));
            assert_eq!(nerd.width(), plain.width() + 2);
        }
    }
}
