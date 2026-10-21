//! Shared toolbar commands and status content; adapters own their layout.
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
}

/// A row of the application menu: its label, why it cannot run, and what it does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuRow {
    pub label: String,
    pub disabled: Option<&'static str>,
    pub target: MenuTarget,
}

fn provider_message(mut provider: ProviderView) -> String {
    // Sampled RSS belongs in details, rather than moving the status text every sample.
    provider.memory = None;
    provider.summary()
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
        controls.extend([
            (CommandId::Open, "Open"),
            (CommandId::FindFile, "Find file"),
            (CommandId::SearchProject, "Search"),
        ]);
        controls
    }

    /// The application menu in both frontends: the command groups (`None`; Application is its theme chooser) and
    /// All commands, or `group`'s way back and its commands with their states.
    pub fn menu(&self, group: Option<CommandGroup>) -> Vec<MenuRow> {
        let row = |label, disabled, target| MenuRow { label, disabled, target };
        match group {
            None => CommandGroup::ALL
                .iter()
                .map(|&g| match g {
                    CommandGroup::Application => row(
                        CommandId::ChooseTheme.label().to_string(),
                        self.disabled(CommandId::ChooseTheme),
                        MenuTarget::Run(CommandId::ChooseTheme),
                    ),
                    _ => row(format!("{} ›", g.label()), None, MenuTarget::Group(g)),
                })
                .chain([row("All commands…".to_string(), None, MenuTarget::AllCommands)])
                .collect(),
            Some(CommandGroup::LanguageServers) => std::iter::once(row(
                "‹ All menus — Language Servers".into(), None, MenuTarget::Back,
            )).chain(self.language_server_rows().into_iter().filter_map(|(label, id)| {
                id.map(|id| row(label, self.disabled(id), MenuTarget::Run(id)))
            })).collect(),
            Some(g) => std::iter::once(row(format!("‹ All menus — {}", g.label()), None, MenuTarget::Back))
                .chain(self.actions().into_iter().filter(|a| a.id.group() == g).map(|a| {
                    let label = match a.on {
                        Some(true) => format!("{} (on)", a.label),
                        Some(false) => format!("{} (off)", a.label),
                        None => a.label.to_string(),
                    };
                    row(label, a.disabled, MenuTarget::Run(a.id))
                }))
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
        if let Some(provider) = state
            .provider()
            .filter(|p| matches!(p.status, Status::Missing(_) | Status::Failed(_)))
        {
            return provider_message(provider);
        }
        state.document().map_or_else(
            || "No file open — Ctrl+O file, F10 commands".into(),
            |d| d.source().display().to_string(),
        )
    }

    /// Cursor position (byte column, as in the existing terminal status) or image dimensions.
    pub fn status_position(&self) -> String {
        let Some(tab) = self.state().active() else {
            return String::new();
        };
        if let Some(image) = tab
            .document()
            .and_then(|d| d.image())
            .filter(|_| !tab.image_source())
        {
            return format!("{}×{}", image.width, image.height);
        }
        match tab.selection().zip(tab.document()) {
            Some((selection, document)) => {
                let line = document.line_of(selection.head);
                format!(
                    "{}:{}",
                    line + 1,
                    selection.head - document.line_range(line).start + 1
                )
            }
            None => format!("Line {}", tab.top_line() + 1),
        }
    }

    /// Full status snapshot, built only when requested so background samples cannot move a details dialog.
    pub fn status_details(&self) -> String {
        let state = self.state();
        let name = match (state.loading(), state.document()) {
            (Some(path), _) => format!("Loading {}…", path.display()),
            (None, Some(document)) => document.source().display().to_string(),
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
            .and_then(|t| t.document()?.image())
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
        };
        let before = provider_message(provider.clone());
        provider.memory = Some((150 << 20, 30 << 20));
        assert_eq!(provider_message(provider), before);
        assert!(before.contains("runtime incompatible"));
        assert!(before.contains("1.2"));
    }
}
