//! Archive browsing changes the explorer overlay, never the physical project or open documents.
use super::*;
use crate::archive_tree::{Listed, Target};

impl Session {
    /// A CLI/Open File/archive-member request whose destination is itself an archive.
    pub(super) fn browse_archive(&mut self, location: crate::archive::Location) -> Effect {
        let location = match location.canonicalized() {
            Ok(location) => location,
            Err(error) => {
                self.state.messages.push(format!("Open archive: {error}"));
                return Effect {
                    repaint: true,
                    ..Effect::default()
                };
            }
        };
        let identity = self.state.archive_tree.open(location.clone());
        let first = self
            .state
            .archive_tree
            .roots()
            .iter()
            .filter(|l| {
                l.container == location.container && location.members.starts_with(&l.members)
            })
            .map(|l| l.members.len())
            .min()
            .unwrap_or(location.members.len());
        for n in first..=location.members.len() {
            let ancestor = crate::archive::Location {
                container: location.container.clone(),
                members: location.members[..n].to_vec(),
            };
            if self.state.archive_tree.needs_listing(&ancestor) {
                self.request_archive_listing(ancestor);
            }
        }
        let reveal = self
            .state
            .archive_tree
            .take_reveal()
            .or_else(|| {
                self.state
                    .archive_tree
                    .roots()
                    .iter()
                    .find(|l| {
                        l.container == location.container
                            && location.members.starts_with(&l.members)
                    })
                    .map(|l| l.to_uri().to_string_lossy().as_ref().into())
            })
            .unwrap_or(identity);
        self.reveal_archive_row(reveal);
        Effect {
            repaint: true,
            ..Effect::default()
        }
    }

    fn request_archive_listing(&mut self, location: crate::archive::Location) {
        let Some(request) = self
            .archive_browser
            .request(&mut self.state.archive_tree, location)
        else {
            return;
        };
        let tx = self.tx.clone();
        self.jobs.submit(Priority::Visible, move || {
            let listed = request.read();
            let _ = tx.send_blocking(Completion::ArchiveListed(listed));
        });
    }

    fn reveal_archive_row(&mut self, path: Arc<str>) {
        self.state.tree_revision = self.state.tree_revision.wrapping_add(1);
        self.show_created(path);
    }

    /// Called before the existing physical-project/library activation code.
    pub(super) fn activate_archive_tree(&mut self, path: &str, retain: bool) -> Option<Effect> {
        if !self.state.archive_tree.is_virtual(path) {
            self.state.archive_tree.reveal(path);
        }
        let target = self.state.archive_tree.target(path)?;
        match target {
            Target::Archive(location) => {
                let expanded = self.state.archive_tree.expanded(path);
                self.state.archive_tree.set_expanded(path.into(), !expanded);
                if expanded {
                    self.archive_browser.cancel_branch(&location);
                } else if self.state.archive_tree.needs_listing(&location) {
                    self.request_archive_listing(location);
                }
            }
            Target::Directory => {
                let expanded = self.state.archive_tree.expanded(path);
                self.state.archive_tree.set_expanded(path.into(), !expanded);
            }
            Target::Member(location) => {
                // The source coordinator retains or previews this member through ordinary tab identity.
                let uri = location.to_uri();
                return Some(self.navigate(uri, retain, None, true));
            }
            Target::Retry(location) => {
                self.archive_browser.cancel_branch(&location);
                self.request_archive_listing(location);
            }
            Target::Up(identity) => {
                let parent = self.state.archive_tree.parent(&identity);
                if let Some(Target::Archive(location)) = self.state.archive_tree.target(&identity) {
                    self.archive_browser.cancel_branch(&location);
                }
                if let Some(parent) = parent {
                    self.state.archive_tree.set_expanded(identity, false);
                    self.reveal_archive_row(parent);
                } else {
                    self.state.archive_tree.close_root(&identity);
                    let next = self.state.tree_rows().next().map(|r| r.path.clone());
                    if let Some(row) = next {
                        self.reveal_archive_row(row);
                    } else {
                        self.state.explorer.selected = None;
                        self.state.tree_reveal = None;
                        self.state.tree_revision = self.state.tree_revision.wrapping_add(1);
                    }
                }
            }
            Target::Unavailable(error) => self.state.messages.push(error),
            Target::Status => {}
        }
        Some(Effect {
            repaint: true,
            ..Effect::default()
        })
    }

    /// Virtual parent traversal never converts an archive URI into a filesystem `..` path.
    pub(super) fn archive_tree_parent(&mut self, path: &str) -> Option<Effect> {
        if !self.state.archive_tree.is_virtual(path) {
            return None;
        }
        if let Some(parent) = self.state.archive_tree.parent(path) {
            self.reveal_archive_row(parent);
        } else if let Some(Target::Archive(location)) = self.state.archive_tree.target(path) {
            self.archive_browser.cancel_branch(&location);
            self.state.archive_tree.close_root(path);
            let next = self.state.tree_rows().next().map(|r| r.path.clone());
            if let Some(next) = next {
                self.reveal_archive_row(next);
            } else {
                self.state.explorer.selected = None;
                self.state.tree_reveal = None;
                self.state.tree_revision = self.state.tree_revision.wrapping_add(1);
            }
        }
        Some(Effect {
            repaint: true,
            ..Effect::default()
        })
    }

    pub(super) fn archive_reveal(&mut self, path: &str) -> Option<Effect> {
        if self.state.archive_tree.reveal(path) {
            self.reveal_archive_row(path.into());
            return Some(Effect {
                repaint: true,
                ..Effect::default()
            });
        }
        let location = crate::archive::Location::from_uri(Path::new(path))?;
        let location = match location.canonicalized() {
            Ok(location) => location,
            Err(error) => {
                self.state.messages.push(format!("Locate archive: {error}"));
                return Some(Effect {
                    repaint: true,
                    ..Effect::default()
                });
            }
        };
        let uri: Arc<str> = location.to_uri().to_string_lossy().as_ref().into();
        let browser = location.parent().unwrap_or(location);
        self.browse_archive(browser);
        self.state.archive_tree.request_reveal(uri);
        if let Some(path) = self.state.archive_tree.take_reveal() {
            self.reveal_archive_row(path);
        }
        Some(Effect {
            repaint: true,
            ..Effect::default()
        })
    }

    pub(super) fn archive_listed(&mut self, listed: Listed) -> Effect {
        let changed = self
            .archive_browser
            .apply(&mut self.state.archive_tree, listed);
        if changed && let Some(path) = self.state.archive_tree.take_reveal() {
            self.reveal_archive_row(path);
        }
        Effect {
            repaint: changed,
            ..Effect::default()
        }
    }

    /// A saved outer container invalidates every listing and cached nested layer from that snapshot.
    pub(super) fn archive_container_changed(&mut self, container: &Path) {
        self.archive_browser.invalidate(container);
        for location in self.state.archive_tree.invalidate(container) {
            self.request_archive_listing(location);
        }
    }

    /// Follow an ordinary explorer rename without replacing a project or any member draft.
    pub(super) fn archive_container_moved(&mut self, from: &Path, to: &Path) {
        let real_entry = std::fs::symlink_metadata(to).is_ok_and(|m| !m.file_type().is_symlink());
        let canonical_from = real_entry
            .then(|| {
                Some(
                    std::fs::canonicalize(from.parent()?)
                        .ok()?
                        .join(from.file_name()?),
                )
            })
            .flatten();
        let canonical_to = real_entry.then(|| std::fs::canonicalize(to).ok()).flatten();
        let moved: Vec<_> = self
            .state
            .archive_tree
            .roots()
            .iter()
            .filter_map(|location| {
                let relative = location
                    .container
                    .strip_prefix(from)
                    .ok()
                    .map(|p| (p, to))
                    .or_else(|| {
                        Some((
                            location
                                .container
                                .strip_prefix(canonical_from.as_deref()?)
                                .ok()?,
                            canonical_to.as_deref()?,
                        ))
                    })?;
                let mut next = location.clone();
                next.container = if relative.0.as_os_str().is_empty() {
                    relative.1.to_path_buf()
                } else {
                    relative.1.join(relative.0)
                };
                Some((location.clone(), next))
            })
            .collect();
        for (previous, _) in &moved {
            self.archive_browser.invalidate(&previous.container);
            self.state
                .archive_tree
                .close_root(&previous.to_uri().to_string_lossy());
        }
        for (_, next) in moved {
            self.browse_archive(next);
        }
        // The relocate command selects its physical destination after this helper returns.
        self.state.archive_tree.clear_reveal();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(dir: &Path) -> Session {
        let providers = dir.join("providers.json");
        let none: serde_json::Map<String, serde_json::Value> =
            crate::semantics::provider::PROVIDERS
                .iter()
                .map(|p| {
                    (
                        p.id.into(),
                        serde_json::json!({"executable":format!("/nonexistent/{}",p.id)}),
                    )
                })
                .collect();
        std::fs::write(
            &providers,
            serde_json::json!({"providers":none}).to_string(),
        )
        .unwrap();
        Session::with_text_input(
            Config {
                themes: ThemePaths::under(&dir.join("config"), &dir.join("state"), "lenz-code"),
                recent: dir.join("state/recent.json"),
                settings: dir.join("settings.json"),
                providers,
                cache: dir.join("cache"),
                idle: std::time::Duration::ZERO,
                clock: Arc::new(std::time::Instant::now),
                overrides: ThemeOverrides::default(),
                dark: true,
            },
            true,
        )
    }
    fn settle(session: &mut Session, done: impl Fn(&Session) -> bool) {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done(session) {
            assert!(
                std::time::Instant::now() < until,
                "archive browser did not finish"
            );
            if let Ok(completion) = session.completions().try_recv() {
                session.dispatch(completion);
            } else {
                std::thread::yield_now();
            }
        }
    }

    #[test]
    fn opening_and_closing_archive_overlay_preserves_project_and_dirty_document() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("physical.txt"), b"saved bytes").unwrap();
        let archive = dir.path().join("empty.tar");
        std::fs::write(&archive, [0u8; 1024]).unwrap();
        let mut s = session(dir.path());
        s.dispatch(Command::OpenRoot(root.clone()));
        settle(&mut s, |s| {
            s.state
                .project
                .as_ref()
                .is_some_and(|p| matches!(p.coverage(), project::Coverage::Complete(_)))
        });
        s.dispatch(Command::NewDocument);
        s.dispatch(Command::InsertText {
            text: "unsaved draft".into(),
            kind: crate::text::EditKind::Paste,
        });
        let draft = s.state.active().unwrap().id();
        assert!(s.state.tab_dirty(draft));
        s.dispatch(Command::Open(archive.clone()));
        settle(&mut s, |s| {
            s.state
                .archive_tree
                .rows()
                .iter()
                .any(|r| r.name.as_ref() == "(Empty archive)")
        });
        assert_eq!(s.state.project.as_ref().unwrap().root(), root);
        assert_eq!(s.state.active().unwrap().id(), draft);
        assert!(s.state.tab_dirty(draft));
        let source = s.state.current_source().unwrap();
        assert_eq!(
            source.copy_range(0..source.len_bytes()).unwrap(),
            "unsaved draft"
        );
        let archive_row = s.state.archive_tree.rows()[0].path.clone();
        s.dispatch(Command::ArchiveTreeParent { path: archive_row });
        assert!(s.state.archive_tree.rows().is_empty());
        assert_eq!(s.state.project.as_ref().unwrap().root(), root);
        assert_eq!(s.state.active().unwrap().id(), draft);
        assert!(s.state.tab_dirty(draft));
        assert_eq!(
            std::fs::read(root.join("physical.txt")).unwrap(),
            b"saved bytes"
        );
        assert_eq!(std::fs::read(archive).unwrap(), [0u8; 1024]);
    }

    #[test]
    fn archive_only_error_retry_and_up_return_to_start_without_fake_documents() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("broken.tar");
        std::fs::write(&archive, b"invalid archive bytes").unwrap();
        let mut s = session(dir.path());
        s.dispatch(Command::Open(archive.clone()));
        settle(&mut s, |s| {
            s.state
                .archive_tree
                .rows()
                .iter()
                .any(|r| r.name.contains("Cannot open archive"))
        });
        assert!(s.state.tabs().is_empty());
        assert!(s.state.project.is_none());
        assert!(s.state.start_items().is_empty());
        let retry = s
            .state
            .archive_tree
            .rows()
            .iter()
            .find(|r| r.name.contains("Cannot open archive"))
            .unwrap()
            .path
            .clone();
        std::fs::write(&archive, [0u8; 1024]).unwrap();
        s.dispatch(Command::ActivateTree {
            path: retry,
            retain: false,
        });
        settle(&mut s, |s| {
            s.state
                .archive_tree
                .rows()
                .iter()
                .any(|r| r.name.as_ref() == "(Empty archive)")
        });
        let up = s
            .state
            .archive_tree
            .rows()
            .iter()
            .find(|r| r.name.as_ref() == "↑ Up / close archive")
            .unwrap()
            .path
            .clone();
        s.dispatch(Command::ActivateTree {
            path: up,
            retain: false,
        });
        assert!(!s.state.has_tree());
        assert!(!s.state.start_items().is_empty());
        assert!(s.state.tabs().is_empty());
        assert!(s.state.project.is_none());
    }

    #[test]
    fn locate_archive_member_rebuilds_cleared_browser_after_project_open_and_resolves_alias() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("bundle.tar");
        let bytes = lenz_archive::testing::tar(
            lenz_archive::Compression::None,
            &[
                ("dir/plain.txt", b"plain original\n"),
                ("untouched.bin", b"\0\xff"),
            ],
        );
        std::fs::write(&archive, &bytes).unwrap();
        let member =
            crate::archive::Location::new(archive.clone(), vec!["dir/plain.txt".into()]).unwrap();
        let mut s = session(dir.path());
        s.dispatch(Command::OpenArchiveEntry(member.clone()));
        settle(&mut s, |s| {
            s.state
                .active()
                .is_some_and(|t| t.loaded() && t.source() == member.to_uri())
        });
        let tab = s.state.active().unwrap().id();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        std::fs::write(project.join("keep.txt"), b"physical bytes").unwrap();
        s.dispatch(Command::OpenRoot(project.clone()));
        settle(&mut s, |s| {
            s.state
                .project
                .as_ref()
                .is_some_and(|p| matches!(p.coverage(), project::Coverage::Complete(_)))
        });
        assert_eq!(
            s.state.active().unwrap().id(),
            tab,
            "opening the first folder retains standalone tabs"
        );
        assert!(s.state.archive_tree.rows().is_empty());
        s.dispatch(Command::InsertText {
            text: "draft ".into(),
            kind: crate::text::EditKind::Paste,
        });
        assert!(s.state.tab_dirty(tab));
        let snapshot = s.state.current_source().unwrap().key();
        let alias = dir.path().join("alias.tar");
        std::os::unix::fs::symlink(&archive, &alias).unwrap();
        let requested = crate::archive::Location::new(alias, member.members.clone()).unwrap();
        s.dispatch(Command::RevealInTree(
            requested.to_uri().to_string_lossy().as_ref().into(),
        ));
        settle(&mut s, |s| {
            s.state
                .archive_tree
                .rows()
                .iter()
                .any(|r| r.path.as_ref() == member.to_uri().to_string_lossy())
        });
        assert_eq!(
            s.state.archive_tree.roots()[0].container,
            archive.canonicalize().unwrap()
        );
        assert!(
            s.state
                .archive_tree
                .rows()
                .iter()
                .any(|r| r.name.as_ref() == "dir" && r.dir == Some(true))
        );
        assert_eq!(
            s.state.tree_reveal.as_ref().unwrap().0.as_ref(),
            member.to_uri().to_string_lossy()
        );
        assert_eq!(s.state.project.as_ref().unwrap().root(), project);
        assert_eq!(s.state.active().unwrap().id(), tab);
        assert_eq!(s.state.current_source().unwrap().key(), snapshot);
        assert!(s.state.tab_dirty(tab));
        assert_eq!(std::fs::read(&archive).unwrap(), bytes);
        assert_eq!(
            std::fs::read(project.join("keep.txt")).unwrap(),
            b"physical bytes"
        );
    }

    #[test]
    fn renaming_archive_parent_through_project_alias_retargets_browser_and_keeps_dirty_member() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real-project");
        std::fs::create_dir_all(real.join("old")).unwrap();
        let alias = dir.path().join("project-alias");
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        let bytes = lenz_archive::testing::tar(
            lenz_archive::Compression::None,
            &[
                ("dir/plain.txt", b"plain original\n"),
                ("untouched.bin", b"\0\xff"),
            ],
        );
        std::fs::write(real.join("old/bundle.tar"), &bytes).unwrap();
        let mut s = session(dir.path());
        s.dispatch(Command::OpenRoot(alias.clone()));
        settle(&mut s, |s| {
            s.state
                .project
                .as_ref()
                .is_some_and(|p| matches!(p.coverage(), project::Coverage::Complete(_)))
        });
        s.dispatch(Command::Open(alias.join("old/bundle.tar")));
        settle(&mut s, |s| {
            s.state
                .archive_tree
                .rows()
                .iter()
                .any(|r| r.name.as_ref() == "dir")
        });
        let member = crate::archive::Location::new(
            real.join("old/bundle.tar"),
            vec!["dir/plain.txt".into()],
        )
        .unwrap();
        s.dispatch(Command::OpenArchiveEntry(member));
        settle(&mut s, |s| s.state.active().is_some_and(|t| t.loaded()));
        let tab = s.state.active().unwrap().id();
        s.dispatch(Command::InsertText {
            text: "draft ".into(),
            kind: crate::text::EditKind::Paste,
        });
        assert!(s.state.tab_dirty(tab));
        let source = s.state.current_source().unwrap();
        let draft = source.copy_range(0..source.len_bytes()).unwrap();
        let buffer = source.key().buffer;
        let panes: Vec<_> = s.state.tabs().iter().map(|t| (t.id(), t.pane())).collect();
        s.dispatch(Command::RelocateEntry {
            kind: crate::relocate::Kind::Rename,
            from: "old".into(),
            input: "new".into(),
        });
        settle(&mut s, |s| {
            s.state
                .archive_tree
                .roots()
                .iter()
                .any(|l| l.container == real.join("new/bundle.tar"))
                && s.state
                    .archive_tree
                    .rows()
                    .iter()
                    .any(|r| r.name.as_ref() == "dir")
        });
        assert!(
            s.state
                .archive_tree
                .roots()
                .iter()
                .all(|l| !l.container.starts_with(real.join("old")))
        );
        assert_eq!(s.state.project.as_ref().unwrap().root(), alias);
        assert_eq!(s.state.active().unwrap().id(), tab);
        assert_eq!(
            s.state
                .tabs()
                .iter()
                .map(|t| (t.id(), t.pane()))
                .collect::<Vec<_>>(),
            panes
        );
        assert!(s.state.tab_dirty(tab));
        let source = s.state.current_source().unwrap();
        assert_eq!(source.key().buffer, buffer);
        assert_eq!(source.copy_range(0..source.len_bytes()).unwrap(), draft);
        assert!(!real.join("old").exists());
        assert_eq!(std::fs::read(real.join("new/bundle.tar")).unwrap(), bytes);
    }
}
