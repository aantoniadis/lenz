//! A pane's breadcrumbs (the path, then the declarations at the caret) and the list after each crumb.

use std::sync::Arc;

use lenz_content::highlight::{DeclKind, Declarations};

use crate::{project::Project, session::Rendering, tabs::Tab};

/// One crumb of a pane's bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Crumb {
    /// A folder: its name and its path (project-relative, or absolute outside the project).
    Folder { name: Arc<str>, rel: Arc<str> },
    File { name: Arc<str> },
    /// A declaration holding the caret: its first line and its index in the tab's declarations.
    Decl { kind: DeclKind, name: Box<str>, line: usize, index: u32 },
}

/// A pane's bar: its crumbs, and whether the file is too large for declarations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Breadcrumbs {
    pub crumbs: Vec<Crumb>,
    pub large: bool,
}

/// Where an entry of a crumb's list leads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SiblingTarget {
    /// A project folder, shown in the explorer.
    Folder(Arc<str>),
    /// A project file, opened.
    File(Arc<str>),
    /// A line of the shown document.
    Line(usize),
}

impl Crumb {
    pub fn name(&self) -> &str {
        match self {
            Self::Folder { name, .. } | Self::File { name } => name,
            Self::Decl { name, .. } => name,
        }
    }
}

impl Breadcrumbs {
    /// Which crumbs fit in `avail` when each takes `width(name) + sep` and a run of left-out ones one "…" with its
    /// `sep`: folders go first, then declarations before the innermost; the file, the innermost and `keep` stay.
    pub fn fit(&self, width: impl Fn(&str) -> f32, sep: f32, avail: f32, keep: Option<usize>) -> Vec<bool> {
        let n = self.crumbs.len();
        let file = self.crumbs.iter().position(|c| matches!(c, Crumb::File { .. }));
        let mut kept = vec![true; n];
        let total = |kept: &[bool]| {
            let (mut t, mut gap) = (0.0, false);
            for (c, k) in self.crumbs.iter().zip(kept) {
                if *k {
                    t += width(c.name()) + sep;
                } else if !gap {
                    t += width("…") + sep;
                }
                gap = !*k;
            }
            t
        };
        for i in (0..file.unwrap_or(0)).chain(file.map_or(0, |f| f + 1)..n.saturating_sub(1)) {
            if total(&kept) <= avail {
                break;
            }
            if Some(i) != keep {
                kept[i] = false;
            }
        }
        kept
    }
}

/// One entry of the list after a crumb.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sibling {
    pub label: String,
    pub dir: bool,
    pub target: SiblingTarget,
}

/// The bar for `tab`: its path relative to `project` (its absolute path outside it), then the declarations holding
/// its caret (the top line's start without one). Previews and images show the path only.
pub(crate) fn build(tab: &Tab, view: Option<&crate::session::View<'_>>, project: Option<&Project>) -> Breadcrumbs {
    let mut crumbs = Vec::new();
    match project.and_then(|p| p.relative(&tab.source).filter(|rel| p.has_file(rel))) {
        Some(rel) => {
            let mut at = 0;
            for part in rel.split('/') {
                let end = at + part.len();
                if end == rel.len() {
                    crumbs.push(Crumb::File { name: part.into() });
                } else {
                    crumbs.push(Crumb::Folder { name: part.into(), rel: rel.get(..end).unwrap_or_default().into() });
                }
                at = end + 1;
            }
        }
        None if tab.untitled() => crumbs.push(Crumb::File { name: tab.title().into() }),
        None => {
            let mut path = std::path::PathBuf::new();
            let parts: Vec<_> = tab.source.components().collect();
            for (i, c) in parts.iter().enumerate() {
                path.push(c);
                let name: Arc<str> = c.as_os_str().to_string_lossy().into();
                if i + 1 == parts.len() {
                    crumbs.push(Crumb::File { name });
                } else if !matches!(c, std::path::Component::RootDir | std::path::Component::Prefix(_)) {
                    crumbs.push(Crumb::Folder { name, rel: path.to_string_lossy().into() });
                }
            }
        }
    }
    let doc = view.map(|v| v.snapshot());
    let large = doc.is_some_and(|d| d.is_large());
    let text = doc.is_some_and(|d| d.image().is_none() || tab.image_source) && !matches!(view.and_then(|v| v.rendering()), Some(Rendering::Preview(_) | Rendering::Pending));
    if let (Some(doc), Some(d), true) = (doc, view.and_then(|v| v.declarations()), text) {
        let at = tab.selection().map_or_else(|| doc.reader_line_range(tab.top_line().min(doc.line_count().saturating_sub(1))).start, |s| s.head);
        let at = u32::try_from(at).unwrap_or(u32::MAX);
        for decl in d.enclosing(at) {
            let index = index_of(d, decl);
            crumbs.push(Crumb::Decl { kind: decl.kind, name: decl.name.clone(), line: decl.first_line as usize, index });
        }
    }
    Breadcrumbs { crumbs, large }
}

/// `decl`'s index in `d` (it borrows from `d.items`): by its start, then identity.
fn index_of(d: &Declarations, decl: &lenz_content::highlight::Declaration) -> u32 {
    let start = d.items.partition_point(|x| x.range.start < decl.range.start);
    let i = d.items.get(start..).unwrap_or_default().iter().position(|x| std::ptr::eq(x, decl)).map_or(0, |i| start + i);
    u32::try_from(i).unwrap_or(u32::MAX)
}

/// The list after crumb `i` of `bar`: a folder's entries (project folders only), the entries of a file's folder,
/// the declarations at a declaration's depth under the same parent.
pub(crate) fn inside(bar: &Breadcrumbs, i: usize, tab: &Tab, view: Option<&crate::session::View<'_>>, project: Option<&Project>) -> Vec<Sibling> {
    let entries = |rel: &str| -> Vec<Sibling> {
        project.map_or_else(Vec::new, |p| {
            p.children(rel)
                .into_iter()
                .map(|(name, dir)| {
                    let path: Arc<str> = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}").into() };
                    Sibling { label: name.to_string(), dir, target: if dir { SiblingTarget::Folder(path) } else { SiblingTarget::File(path) } }
                })
                .collect()
        })
    };
    match bar.crumbs.get(i) {
        Some(Crumb::Folder { rel, .. }) => entries(rel),
        Some(Crumb::File { .. }) => match i.checked_sub(1).and_then(|j| bar.crumbs.get(j)) {
            Some(Crumb::Folder { rel, .. }) => entries(rel),
            _ if project.is_some_and(|p| p.relative(&tab.source).is_some_and(|r| p.has_file(&r))) => entries(""),
            _ => Vec::new(),
        },
        Some(Crumb::Decl { index, .. }) => {
            let Some(d) = view.and_then(|v| v.declarations()) else { return Vec::new() };
            let parent = d.items.get(*index as usize).and_then(|x| x.parent);
            d.items
                .iter()
                .filter(|x| x.parent == parent)
                .map(|x| Sibling { label: x.name.to_string(), dir: false, target: SiblingTarget::Line(x.first_line as usize) })
                .collect()
        }
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_drops_folders_then_outer_declarations_and_keeps_the_file_innermost_and_focus() {
        let decl = |name: &str| Crumb::Decl { kind: DeclKind::Function, name: name.into(), line: 0, index: 0 };
        let bar = Breadcrumbs {
            crumbs: vec![
                Crumb::Folder { name: "src".into(), rel: "src".into() },
                Crumb::Folder { name: "ui".into(), rel: "src/ui".into() },
                Crumb::File { name: "a.rs".into() },
                decl("Outer"),
                decl("inner"),
            ],
            large: false,
        };
        let w = |s: &str| s.chars().count() as f32;
        assert_eq!(bar.fit(w, 3.0, 100.0, None), vec![true; 5]);
        // src+3 ui+3 a.rs+3 Outer+3 inner+3 = 33; "…"+3 for src is 31, and one "…" for both folders 27.
        assert_eq!(bar.fit(w, 3.0, 32.0, None), vec![false, true, true, true, true]);
        assert_eq!(bar.fit(w, 3.0, 30.0, None), vec![false, false, true, true, true]);
        assert_eq!(bar.fit(w, 3.0, 20.0, None), vec![false, false, true, false, true]);
        assert_eq!(bar.fit(w, 3.0, 0.0, None), vec![false, false, true, false, true], "never the file or innermost");
        assert_eq!(bar.fit(w, 3.0, 20.0, Some(1)), vec![false, true, true, false, true], "the focused crumb stays");
    }
}
