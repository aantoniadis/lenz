//! Shared toolbar commands and status content; adapters own their layout.
pub mod forms;
use crate::{
    actions::CommandGroup,
    project::Coverage,
    semantics::{ProviderView, Status},
    session::{CommandId, Session},
};

/// What a row of the application menu does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuTarget {
    /// Show the group's commands.
    Group(CommandGroup),
    /// Back from a group's commands to the groups.
    Back,
    Run(CommandId),
    /// Search every command by name.
    AllCommands,
    /// A line between rows: nothing to select or run.
    Separator,
}

/// A row of the application menu: its label, why it cannot run, and what it does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuRow {
    pub label: String,
    pub disabled: Option<&'static str>,
    pub target: MenuTarget,
}

fn provider_message(mut provider: ProviderView) -> String {
    if provider.root.is_some() && matches!(provider.status, Status::Missing(_)) {
        return format!("{} unavailable · F2 → Install Language Server… or Server Details…", provider.name);
    }
    // Sampled RSS belongs in details, rather than moving the status text every sample.
    provider.memory = None;
    provider.summary()
}

/// Public identity shared by the desktop, terminal and language-server client.
pub const APP_NAME: &str = "Lenz Code";
pub const APP_ID: &str = "lenz-code";

/// Lenz Code's About title; its text is [`about`].
pub const ABOUT_TITLE: &str = "About Lenz Code";

/// Lenz Code's About text, the same in both frontends: what it is, then the shared application rows.
pub fn about() -> String {
    std::iter::once("Code editor with language-server assistance".to_owned())
        .chain(std::iter::once(String::new()))
        .chain(lenz_content::about::ROWS.iter().map(|(k, v)| format!("{k}: {v}")))
        .collect::<Vec<_>>()
        .join("\n")
}

impl Session {
    /// Frequent actions in both toolbars. Overflow remains available in the command menu.
    pub fn toolbar(&self) -> Vec<(CommandId, &'static str)> {
        let mut controls = self.navigation();
        for (id, label) in &mut controls {
            if *id == CommandId::ToggleExplorer {
                *label = "Explorer";
            }
        }
        if self.branch_label().is_some() { controls.push((CommandId::GitBranches, "Branches")); }
        controls
    }

    /// The shared menu hierarchy: root groups, every command, then after a line the application's own commands; or
    /// a parent link, direct commands and task submenus.
    pub fn menu(&self, group: Option<CommandGroup>) -> Vec<MenuRow> {
        let row = |label, disabled, target| MenuRow { label, disabled, target };
        let commands = |g: CommandGroup| self.actions().into_iter().filter(move |a| a.id.group() == g).map(|a| {
            let label = match a.on {
                Some(true) => format!("{} (on)", a.label),
                Some(false) => format!("{} (off)", a.label),
                None => a.label.to_string(),
            };
            row(label, a.disabled, MenuTarget::Run(a.id))
        });
        let children = |g: CommandGroup| CommandGroup::ALL.iter().filter(move |child| child.parent() == Some(g)).map(|&child| row(format!("{} ›", child.label()), None, MenuTarget::Group(child)));
        match group {
            None => {
                // Closing comes last, after the application's settings and About.
                let (quit, app): (Vec<_>, Vec<_>) = commands(CommandGroup::Application).partition(|r| matches!(r.target, MenuTarget::Run(CommandId::CloseWindow | CommandId::QuitAll)));
                CommandGroup::ALL.iter().filter(|g| g.parent().is_none() && **g != CommandGroup::Application)
                    .map(|&g| row(format!("{} ›", g.label()), None, MenuTarget::Group(g)))
                    .chain([row("All commands…".to_string(), None, MenuTarget::AllCommands), row(String::new(), None, MenuTarget::Separator)])
                    .chain(app)
                    .chain(children(CommandGroup::Application))
                    .chain(quit)
                    .collect()
            }
            Some(CommandGroup::LanguageServers) => std::iter::once(row(
                "‹ All menus — Language Servers".into(), None, MenuTarget::Back,
            )).chain(self.language_server_rows().into_iter().filter_map(|(label, id)| {
                id.filter(|id| id.group() == CommandGroup::LanguageServers).map(|id| row(label, self.disabled(id), MenuTarget::Run(id)))
            })).chain([row("Setup and Indexing ›".into(), None, MenuTarget::Group(CommandGroup::ServerSetup))]).collect(),
            Some(g) => std::iter::once(row(format!("‹ {} — {}", g.menu_parent().map_or("All menus", |p| p.label()), g.label()), None, g.menu_parent().map_or(MenuTarget::Back, MenuTarget::Group)))
                .chain(commands(g))
                .chain(children(g))
                .collect(),
        }
    }

    /// The primary status, without routine provider telemetry. Details retain the complete context.
    pub fn status_message(&self) -> String {
        let state = self.state();
        if let Some(asking) = state.asking() {
            return format!("{asking} (Escape cancels)");
        }
        if let Some(path) = state.loading() {
            return format!("Loading {}…", path.display());
        }
        if let Some(notice) = state.notices().last() {
            return notice.clone();
        }
        // A live snippet says which placeholder Tab is on, in both adapters, until it ends.
        if let Some(snippet) = self.snippet_hint() {
            return snippet;
        }
        // The problem at the caret, for the version shown: what Next Problem would also announce.
        if let Some(problem) = state.problem_at_caret() {
            return problem;
        }
        if let Some(provider) = state
            .provider()
            .filter(|p| matches!(p.status, Status::Missing(_) | Status::Failed(_)))
        {
            return provider_message(provider);
        }
        if let Some(status) = state.active().and_then(|tab| tab.blame_status()) {
            match status {
                crate::blame::Status::Pending => return "Blame pending…".into(),
                crate::blame::Status::Failed(error) => return format!("Blame failed: {error}"),
                crate::blame::Status::Ready => {}
            }
        }
        if let Some(a) = self.caret_annotation() {
            return match a.shade {
                None => a.label,
                Some(_) => {
                    let c = a.commit;
                    format!("{} · {} · {} · {}", c.sha.get(..7).unwrap_or(&c.sha), c.author, lenz_content::blame::date(c.author_time), c.summary)
                }
            };
        }
        // The caret line's change against its baseline, named in words, so the marker never rests on colour alone.
        // An annotation of that line is more specific about its past and comes first.
        if let Some(marker) = self.caret_marker() {
            return marker;
        }
        state.current_source().map_or_else(
            || "No file open — Ctrl+O file, F10 commands".into(),
            |d| shown_name(state, d),
        )
    }

    /// The live snippet's placeholder and what the keys do now, while Tab visits its placeholders rather than
    /// indenting (`None`: no snippet is live).
    pub fn snippet_hint(&self) -> Option<String> {
        let live = self.state().snippet()?;
        let (stop, count) = (live.stop(), live.stop_count());
        Some(if live.at_end() {
            format!("Snippet: placeholder {stop} of {count} · Tab ends · Esc done")
        } else {
            format!("Snippet: placeholder {stop} of {count} · Tab next · Esc done")
        })
    }

    /// The caret line's change marker in words (`Modified from HEAD (main · 1a2b3c4)`), when it has one.
    pub fn caret_marker(&self) -> Option<String> {
        let tab = self.state().active()?;
        let head = tab.selection()?.head;
        let line = self.state().view(tab.id())?.snapshot().line_of(head);
        self.state().markers(tab.id())?.line_label(line)
    }

    /// The active file's change-marker baseline, named (`HEAD (main · 1a2b3c4)`, a new file, or the last save).
    pub fn markers_baseline(&self) -> Option<&crate::markers::Baseline> {
        self.state().active().and_then(|tab| self.state().markers(tab.id()))?.baseline()
    }

    /// The shown draft's baseline and what changed against it, for the status details.
    pub fn markers_details(&self) -> Option<String> {
        let markers = self.state().active().and_then(|tab| self.state().markers(tab.id()))?;
        markers.summary().or_else(|| {
            let baseline = markers.baseline()?;
            Some(format!("vs {}", baseline.label()))
        })
    }

    /// Cursor position (line and character column, as Go to Line takes them) or image dimensions.
    pub fn status_position(&self) -> String {
        let Some(tab) = self.state().active() else {
            return String::new();
        };
        if let Some(image) = self.state().view(tab.id()).map(|v| v.snapshot())
            .and_then(|d| d.image())
            .filter(|_| !tab.image_source())
        {
            return format!("{}×{}", image.width, image.height);
        }
        let position = match tab.selection().zip(self.state().view(tab.id()).map(|v| v.snapshot())) {
            Some((selection, document)) => {
                format!("{}:{}", document.line_of(selection.head) + 1, document.column_at(selection.head) + 1)
            }
            None => format!("Line {}", tab.top_line() + 1),
        };
        self.state().vim_status().map_or_else(|| position.clone(), |mode| format!("{mode}  {position}"))
    }

    /// Full status snapshot, built only when requested so background samples cannot move a details dialog.
    pub fn status_details(&self) -> String {
        let state = self.state();
        let name = match (state.loading(), state.current_source()) {
            (Some(path), _) => format!("Loading {}…", path.display()),
            (None, Some(document)) => shown_name(state, document),
            (None, None) => "No file open — Ctrl+O file, F10 commands".into(),
        };
        let project = state.project().map(|project| {
            let root = project.root().display();
            match project.coverage() {
                Coverage::Scanning(n) => format!("{root}: scanning, {n} files so far"),
                Coverage::Complete(n) if project.refreshing() => {
                    format!("{root}: {n} files, refreshing")
                }
                Coverage::Complete(0) if project.skipped() == 0 => format!("{root}: no files"),
                Coverage::Complete(n) if project.skipped() == 0 => format!("{root}: {n} files"),
                Coverage::Complete(n) => format!(
                    "{root}: {n} files; {} unreadable entries skipped",
                    project.skipped()
                ),
                Coverage::Unavailable(error) => format!("{root}: unavailable ({error})"),
            }
        });
        let position = self.status_position();
        let image = state
            .active()
            .filter(|t| !t.image_source())
            .and_then(|t| state.view(t.id())?.snapshot().image())
            .map(|image| {
                format!(
                    "{}×{} · {:?} · {} bytes{}",
                    image.width,
                    image.height,
                    image.format,
                    image.byte_size,
                    if image.downscaled {
                        " · downscaled"
                    } else {
                        ""
                    }
                )
            });
        state
            .asking()
            .map(|asking| format!("{asking} (Escape cancels)"))
            .into_iter()
            .chain(state.notices())
            .chain([name])
            .chain((!position.is_empty()).then_some(position))
            .chain(self.markers_details())
            .chain(state.results().filter(|r| r.results_kind() == crate::search::ResultsKind::ProjectSyntax).map(|r| format!("{}\nThe captured project inventory respects ignore rules; links outside the project, binary/lossy text, large files, environment files and SQL without a scan provider are not checked. Per-file findings and the total result list are bounded; known explorer badges retain at most 1024 paths. Cancel Project Syntax Check stops background work; Rerun Search checks again.{}", r.kind(), if r.limited() { " The result limit was reached; coverage is incomplete." } else { "" })))
            .chain(state.terminal().active().map(|tab| format!("{} — {}\nInitial directory: {}\n{} terminal tabs{}", tab.label(), tab.status(), tab.cwd().display(), state.terminal().tabs().len(), tab.error().map_or(String::new(), |why| format!("\n{why}")))))
            .chain(image)
            .chain(project)
            .chain(state.provider().map(|p| p.summary()))
            .chain([self.language_server_details()])
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

/// One display line, cut by terminal columns with an ellipsis; the full text is kept by the caller.
pub fn ellipsize(text: &str, cols: usize) -> String {
    use lenz_content::wrap::width;
    let text: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if width(&text) <= cols {
        return text;
    }
    if cols == 0 {
        return String::new();
    }
    let mut used = 0;
    let end = text
        .char_indices()
        .find_map(|(i, c)| {
            used += width(c.encode_utf8(&mut [0; 4]));
            (used > cols - 1).then_some(i)
        })
        .unwrap_or(text.len());
    format!("{}…", text.get(..end).unwrap_or_default())
}

/// The active document's path, or an untitled document's name (its identity is not a path).
fn shown_name(state: &crate::session::State, document: &crate::text::TextSnapshot) -> String {
    match state.active().filter(|t| t.untitled()) {
        Some(tab) => tab.title(),
        None => document.source().display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn truncation_respects_wide_and_combining_characters_and_single_line_status() {
        assert_eq!(ellipsize("日本語", 4), "日…");
        assert_eq!(ellipsize("e\u{301}e\u{301}x", 2), "e\u{301}…");
        assert_eq!(ellipsize("one\ntwo\r\nthree", 80), "one two  three");
        assert_eq!(ellipsize("abc", 0), "");
        assert_eq!(ellipsize("abc", 1), "…");
        assert_eq!(ellipsize("日", 2), "日");
    }

    #[test]
    fn routine_memory_samples_do_not_change_the_status_message() {
        let mut provider = ProviderView {
            name: "Test server",
            root: None,
            status: Status::Failed("runtime incompatible\nUse Java 21".into()),
            version: Some("1.2".into()),
            untested: true,
            limits: "",
            memory: None,
            supports: vec![],
            error: None,
            environment: None,
        };
        let before = provider_message(provider.clone());
        provider.memory = Some((150 << 20, 30 << 20));
        assert_eq!(provider_message(provider.clone()), before);
        assert!(before.contains("runtime incompatible"));
        provider.status = Status::Missing("RAW_STARTUP_DIAGNOSTICS".into());
        provider.root = Some("/workspace".into());
        let missing = provider_message(provider);
        assert!(missing.contains("Install Language Server") && !missing.contains("RAW_STARTUP_DIAGNOSTICS"), "{missing}");
        assert!(before.contains("1.2"));
    }
}
