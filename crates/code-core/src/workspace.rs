//! A directory's last reading workspace; metadata only, never document contents.
use crate::session::{Layout, Selection, Split};
use std::path::{Path, PathBuf};

pub const MAX_TABS: usize = 200;
pub(crate) const MAX_ARCHIVES: usize = 8;

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
    /// Archive browser roots are metadata too; reopening a project keeps its browsed containers.
    #[serde(default)]
    pub archives: Vec<crate::archive::Location>,
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
        self.archives.truncate(MAX_ARCHIVES);
        let mut bytes = 0usize;
        self.archives.retain(|location| {
            bytes = bytes.saturating_add(location.estimated_bytes());
            bytes <= 64 << 10 && location.outer_path().is_file()
        });
        self.tabs.truncate(MAX_TABS);
        let mut active = None;
        let mut other = None;
        let mut index = 0;
        let mut kept = 0;
        self.tabs.retain_mut(|t| {
            let old = index;
            index += 1;
            let archive = crate::archive::Location::from_uri(&t.path);
            let path = if archive.is_some() || t.path.is_absolute() {
                t.path.clone()
            } else {
                root.join(&t.path)
            };
            if !archive.as_ref().map_or_else(|| path.is_file(), |location| location.outer_path().is_file()) {
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
        let explorer_path = |s: &str| relative(s) || crate::archive_tree::valid_identity(s);
        self.explorer.selected = self.explorer.selected.take().filter(|s| explorer_path(s));
        self.explorer.top = self.explorer.top.take().filter(|s| explorer_path(s));
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
        && !path.contains("://")
        && PathBuf::from(path)
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

#[cfg(test)]
mod archive_tests {
    use super::*;

    #[test]
    fn archive_workspace_identity_survives_resolve_and_relativize_without_os_joins() {
        let dir = tempfile::tempdir().unwrap();
        let container = dir.path().join("app.zip");
        std::fs::write(&container, lenz_archive::testing::zip(&[("folder/a.txt", b"saved")])).unwrap();
        let uri = crate::archive::Location::new(container.clone(), vec!["folder/a.txt".into()]).unwrap().to_uri();
        let mut workspace: Workspace = serde_json::from_value(serde_json::json!({
            "tabs": [{"path": uri, "selection": null, "top_line": 0, "pane": 0, "retained": true,
                "column": 0, "top_row": 0, "markdown_source": false, "image_source": false}],
            "active": 0, "other": null, "focused": 0, "split": null,
            "layout": serde_json::to_value(Layout::default()).unwrap(), "soft_wrap": false, "expanded": [], "explorer": {}
        })).unwrap();
        workspace.resolve(dir.path());
        assert_eq!(workspace.tabs.len(), 1);
        assert_eq!(workspace.tabs[0].path, uri);
        workspace.relativize(dir.path());
        assert_eq!(workspace.tabs[0].path, uri);
        std::fs::remove_file(container).unwrap();
        workspace.resolve(dir.path());
        assert!(workspace.tabs.is_empty());
        assert_eq!(workspace.missing, [uri]);
    }
}
