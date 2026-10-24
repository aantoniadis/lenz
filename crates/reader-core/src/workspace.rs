//! A directory's last reading workspace; metadata only, never document contents.
use crate::session::{Layout, Selection, Split};
use std::path::{Path, PathBuf};

pub const MAX_TABS: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Explorer {
    pub visible: bool,
    pub selected: Option<String>,
    pub top: Option<String>,
}
impl Default for Explorer {
    fn default() -> Self {
        Self {
            visible: true,
            selected: None,
            top: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Workspace {
    pub tabs: Vec<SavedTab>,
    pub active: Option<usize>,
    pub other: Option<usize>,
    pub focused: u8,
    pub split: Option<Split>,
    pub layout: Layout,
    pub soft_wrap: bool,
    pub expanded: Vec<String>,
    pub explorer: Explorer,
    /// Tabs dropped on reading because their files are gone; never saved.
    #[serde(skip)]
    pub missing: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct SavedTab {
    pub path: PathBuf,
    pub selection: Option<Selection>,
    pub top_line: usize,
    pub pane: u8,
    pub retained: bool,
    pub column: usize,
    pub top_row: usize,
    pub markdown_source: bool,
    pub image_source: bool,
}

impl Workspace {
    /// Resolve paths and reject unsafe/unbounded metadata before it reaches the session; off the UI thread.
    pub fn resolve(&mut self, root: &Path) {
        self.tabs.truncate(MAX_TABS);
        let mut active = None;
        let mut other = None;
        let mut index = 0;
        let mut kept = 0;
        self.tabs.retain_mut(|t| {
            let old = index;
            index += 1;
            let path = if t.path.is_absolute() {
                t.path.clone()
            } else {
                root.join(&t.path)
            };
            if !path.is_file() {
                self.missing.push(path);
                return false;
            }
            if self.active == Some(old) {
                active = Some(kept);
            }
            if self.other == Some(old) {
                other = Some(kept);
            }
            t.path = path;
            let cap = crate::places::MAX_POSITION;
            t.top_line = t.top_line.min(cap);
            if let Some(selection) = &mut t.selection {
                selection.anchor = selection.anchor.min(cap);
                selection.head = selection.head.min(cap);
            }
            t.pane = t.pane.min(u8::from(self.split.is_some()));
            t.column = t.column.min(cap);
            t.top_row = t.top_row.min(cap);
            kept += 1;
            true
        });
        self.active = active.filter(|i| *i < self.tabs.len());
        self.other = other.filter(|i| *i < self.tabs.len());
        self.focused = self.focused.min(u8::from(self.split.is_some()));
        self.expanded.retain(|s| relative(s));
        self.expanded.truncate(10_000);
        self.explorer.selected = self.explorer.selected.take().filter(|s| relative(s));
        self.explorer.top = self.explorer.top.take().filter(|s| relative(s));
    }

    pub fn relativize(&mut self, root: &Path) {
        for t in &mut self.tabs {
            t.path = t
                .path
                .strip_prefix(root)
                .map_or_else(|_| t.path.clone(), Path::to_owned);
        }
    }
}
fn relative(path: &str) -> bool {
    !path.is_empty()
        && PathBuf::from(path)
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}
