//! Shared terminal sessions, input and immutable visible cell projections.
mod input;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod model;
mod worker;
pub(crate) use input::MAX_PASTE;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) use input::{encode_key, encode_paste, quote_paths};
pub use worker::Process;

use crate::keymap::KeyPress;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Id(pub u64);

/// A terminal's default text and background and its 16 ANSI colours, all from the theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colors {
    pub fg: u32,
    pub bg: u32,
    pub ansi: [u32; 16],
}
impl Colors {
    pub fn of(palette: &lenz_content::theme::Palette) -> Self {
        Self { fg: palette.fg, bg: palette.bg, ansi: palette.terminal }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseKind {
    Press(u8),
    Release(u8),
    Move(u8),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Show,
    Hide,
    New,
    Restart,
    Focus(bool),
    Activate(Id),
    Rename { id: Id, label: String },
    Next {
        backwards: bool,
    },
    Close,
    Stop,
    Confirm {
        ticket: u64,
        accept: bool,
    },
    Key(KeyPress),
    /// The theme changed: redraw with its colours.
    Colors(Colors),
    Text(String),
    Paste(String),
    Paths(Vec<PathBuf>),
    Resize {
        columns: usize,
        rows: usize,
    },
    Scroll {
        lines: i32,
        local: bool,
    },
    ScrollTo {
        top: bool,
    },
    Select {
        row: usize,
        column: usize,
        extend: bool,
    },
    Copy,
    Find,
    FindClose,
    Search { query: String, backwards: bool, advance: bool },
    Mouse {
        row: usize,
        column: usize,
        kind: MouseKind,
        ctrl: bool,
        alt: bool,
        shift: bool,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    pub text: String,
    pub fg: Option<u32>,
    pub bg: Option<u32>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikeout: bool,
    pub dim: bool,
    pub selected: bool,
    pub search_match: bool,
    pub wide_spacer: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub columns: usize,
    pub rows: usize,
    pub cells: Vec<Cell>,
    pub cursor: Option<(usize, usize)>,
    pub scrollback: usize,
    /// Query and result; None means output/resize invalidated its coordinates.
    pub search: Option<(String, Option<bool>)>,
}

pub struct Tab {
    pub(crate) id: Id,
    pub(crate) generation: u64,
    pub(crate) label: String,
    pub(crate) cwd: PathBuf,
    pub(crate) running: bool,
    pub(crate) starting: bool,
    pub(crate) status: String,
    pub(crate) error: Option<String>,
    pub(crate) frame: Option<Arc<Frame>>,
}
impl Tab {
    pub fn id(&self) -> Id {
        self.id
    }
    pub fn label(&self) -> &str {
        &self.label
    }
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }
    pub fn running(&self) -> bool {
        self.running
    }
    pub fn starting(&self) -> bool {
        self.starting
    }
    pub fn status(&self) -> &str {
        &self.status
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn frame(&self) -> Option<&Arc<Frame>> {
        self.frame.as_ref()
    }
}

/// A confirmation the panel asks for: both adapters show its title, text and accept label.
pub struct Prompt {
    pub ticket: u64,
    pub title: &'static str,
    pub text: String,
    /// The accepting button's label; the other button cancels.
    pub accept: &'static str,
}
#[derive(Default)]
pub struct State {
    pub(crate) tabs: Vec<Tab>,
    pub(crate) active: Option<Id>,
    pub(crate) shown: bool,
    pub(crate) focused: bool,
    pub(crate) prompt: Option<Prompt>,
    pub(crate) find: Option<String>,
}
impl State {
    pub fn shown(&self) -> bool {
        self.shown
    }
    pub fn focused(&self) -> bool {
        self.focused && self.shown
    }
    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }
    pub fn active(&self) -> Option<&Tab> {
        self.tabs.iter().find(|t| Some(t.id) == self.active)
    }
    pub fn find(&self) -> Option<&str> { self.find.as_deref() }
    pub fn find_status(&self) -> &'static str {
        match self.find() {
            None | Some("") => "Enter text",
            Some(query) => match self.active().and_then(|t| t.frame()).and_then(|f| f.search.as_ref()).filter(|(q,_)| q == query) {
                Some((_,Some(true))) => "Match",
                Some((_,Some(false))) => "No matches",
                Some((_,None)) => "Output changed · Enter searches",
                None => "Searching…",
            }
        }
    }
    /// A bounded, horizontally revealed find row for either adapter.
    pub fn find_label(&self, columns: usize, marked: &str) -> Option<String> {
        let query = self.find()?;
        let status = self.find_status();
        let help = if columns > 64 { format!("  {status} · Enter/Shift+Enter · Esc") } else { format!("  {status}") };
        let room = columns.saturating_sub(7 + lenz_content::wrap::width(&help));
        let value = format!("{query}{marked}");
        let mut used = 0;
        let tail = value.char_indices().rev().take_while(|(_,c)| {
            used += lenz_content::wrap::width(&c.to_string()); used <= room
        }).last().map_or(value.len(), |(at,_)| at);
        Some(format!("Find: {}▏{help}", value.get(tail..).unwrap_or_default()))
    }
    pub fn prompt(&self) -> Option<&Prompt> {
        self.prompt.as_ref()
    }
}
