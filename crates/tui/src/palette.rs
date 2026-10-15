//! The command palette over the shared catalog (`:` and F10): scope, availability, dispatch, drawing and input.
use crate::app::{App, Prompt};
use crate::chrome::color;
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use lenz::catalog::{self, Action, Availability, HelpAction, PrAction, RepoAction, Scope, Surface};
use lenz::palette::{Outcome, Palette, PaletteCommand};
use lenz::repository_session::RepositoryCommand as R;
use ratatui::{layout::Rect, style::Modifier, widgets::{Block, Borders, Clear}};
use std::borrow::Cow;

impl App {
    /// What the comparison shows now, for the catalog.
    pub(crate) fn scope(&self) -> Scope {
        let markdown = self.session.disabled(lenz::commands::Command::SetPresentation(lenz::markdown::PresentationMode::Rendered)) != Some("Not a Markdown comparison");
        Scope { surface: Surface::Comparison, sticky: self.visual.sticky, unified: self.session.view().unified(), repository: self.repository.is_some(), focused: self.layout.focused(self.repository.as_ref().map(|r| &r.core)), pr_tab: self.repository.as_ref().and_then(|r| r.core.pr.as_ref()).and_then(|p| p.details()).map(|d| d.tab), pr: self.repository.as_ref().is_some_and(|r| r.core.pr.is_some()),
                image: self.visual.image().is_some(), markdown, options: self.session.options(), scroll: self.session.view().mode(),
                presentation: self.session.markdown_mode(), context_hidden: self.session.context_hidden(), blame: self.blame[self.focus as usize],
                image_mode: self.visual.image_state().mode(), markdown_layout: self.session.markdown_layout(), find: self.find_open().then(|| self.session.find_options()) }
    }

    pub(crate) fn open_palette(&mut self) {
        self.toolbar.close();
        self.prompt = None;
        // a list search ends as with Esc: the palette's commands act on the accepted selection
        self.navigator.cancel_search();
        if let Some(repo) = &mut self.repository { repo.cancel_log_search(); }
        self.command_palette = Some(Palette::new(terminal_keys(catalog::entries(&self.scope())), &*self));
    }

    /// Run a catalog action; every variant is handled here and nowhere else. The palette closes first.
    pub(crate) fn run_action(&mut self, action: Action) {
        // an action on the diff or a file shows the diff first
        let on_diff = matches!(action, Action::Comparison(_) | Action::Edit(_) | Action::File(_) | Action::Image(_) | Action::Interaction(_)
            | Action::Repository(RepoAction::Back | RepoAction::Forward) | Action::Pr(PrAction::MarkViewedNext | PrAction::Threads | PrAction::OutdatedThreads))
            && !matches!(action, Action::Interaction(lenz::commands::Interaction::About | lenz::commands::Interaction::ChangeTheme | lenz::commands::Interaction::ReportLastError | lenz::commands::Interaction::ResetPaneSizes));
        if on_diff && self.pr_details_shown() { self.pr_command(lenz::pr_session::PrCommand::Close); }
        self.run_action_inner(action);
        self.sync_pr_tab();
    }
    fn run_action_inner(&mut self, action: Action) {
        self.command_palette = None;
        match action {
            Action::Comparison(lenz::commands::Command::SetDiffLayout(lenz::commands::DiffLayout::Unified)) if let Some(why) = self.unified_disabled() => self.session.set_notice(why),
            Action::Comparison(command) => { if let Some(why) = self.session.disabled(command) { self.session.set_notice(why); } else { self.comparison_command(command); } }
            // as Ctrl-F: the focused Files, Branches or History pane's own search, else Find in the comparison
            Action::Interaction(lenz::commands::Interaction::Find) => { self.handle(Event::Key(crossterm::event::KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL))); }
            Action::Interaction(command) => self.interaction(command),
            Action::Edit(command) => self.edit_menu(command),
            Action::File(file) => { if let Some(why) = Availability::disabled(&*self, action) { self.session.set_notice(why); } else { self.file_action(file, self.session.current()); } }
            Action::Image(index) => if let Some(entry) = lenz::image_session::image_commands().get(index) { self.visual.image_execute(entry.command, self.image_area()); },
            Action::Repository(RepoAction::Refresh) => self.reload(),
            Action::Repository(RepoAction::Branches) => self.show_branches(),
            Action::Repository(RepoAction::Filter) => {
                // from Focus: the panels it hid come back first
                if self.layout.focused(self.repository.as_ref().map(|r| &r.core)) { self.layout_command(lenz::layout::LayoutCommand::LeaveFocus); }
                if let Some(repo) = &mut self.repository { repo.begin_filter(); }
            }
            Action::Repository(RepoAction::OpenWorktree) => match self.repository.as_ref().map(|r| r.core.selected_worktree()) {
                Some(Ok(path)) => self.repository_effect(lenz::repository_session::RepositoryEffect::OpenWorktree(path)),
                Some(Err(why)) => self.session.set_notice(why),
                None => {}
            },
            Action::Repository(action @ (RepoAction::StagedChanges | RepoAction::CopyUrl | RepoAction::OpenUrl | RepoAction::LocalChanges
                | RepoAction::AllBranches | RepoAction::CurrentBranch | RepoAction::CompareBranch { .. } | RepoAction::CompareCommit
                | RepoAction::NextParent | RepoAction::FileHistory | RepoAction::WriteGraph | RepoAction::CopyHash | RepoAction::CopySubject
                | RepoAction::CopyDetails | RepoAction::ShowInGraph | RepoAction::ToggleMessage)) => {
                let path = self.session.comparisons().get(self.session.current()).map_or(String::new(), |c| c.path.clone());
                match self.repository.as_ref().map(|r| r.core.action_command(action, &path)) {
                    Some(Ok(Some(command))) => self.repository_command(command),
                    Some(Err(why)) => self.session.set_notice(why),
                    Some(Ok(None)) | None => {}
                }
            }
            Action::Repository(RepoAction::Back) => self.navigate(false),
            Action::Repository(RepoAction::Forward) => self.navigate(true),
            Action::Pr(PrAction::Details(tab)) => self.repository_command(R::PrDetails(tab)),
            Action::Pr(PrAction::Diff) => self.repository_command(R::CloseOverlay),
            Action::Pr(PrAction::MarkViewedNext) => self.repository_command(R::Viewed { index: self.session.current(), jump: true }),
            Action::Pr(PrAction::Refresh) => self.repository_command(R::RefreshPr),
            Action::Pr(PrAction::Threads) => {
                let path = self.session.comparisons().get(self.session.current()).map(|c| c.path.clone()).unwrap_or_default();
                self.repository_command(R::Threads { path, outdated: false });
            }
            Action::Pr(PrAction::CheckHead) => self.repository_command(R::CheckPrHead),
            Action::Pr(PrAction::OutdatedThreads) => {
                let path = self.session.comparisons().get(self.session.current()).map(|c| c.path.clone()).unwrap_or_default();
                self.repository_command(R::Threads { path, outdated: true });
            }
            // the merge view and Version Picker run these; a comparison never lists them
            Action::Merge(_) | Action::Pick(_) => self.session.set_notice("Only in the merge view"),
            Action::Layout(command) => self.layout_command(command),
            Action::Help(HelpAction::Shortcuts) => self.begin(Prompt::Help),
            // the ⚙ menu; when it folded into ⋯ on a narrow terminal, the palette's Diff group
            Action::Help(HelpAction::DiffOptions) => if !self.toolbar.open("⚙") {
                self.open_palette();
                if let Some(p) = &mut self.command_palette { p.command(PaletteCommand::Type("diff ".into())); }
            },
            Action::Help(HelpAction::Settings) => self.open_settings(),
        }
    }

    pub(crate) fn repository_command(&mut self, command: R) {
        let Some(repo) = &mut self.repository else { return };
        if let Some(why) = repo.core.disabled(&command) { self.session.set_notice(why); return; }
        repo.core.execute(command);
    }

    /// Branches…: the repository sidebar, shown and focused.
    fn show_branches(&mut self) {
        let Some(repo) = &mut self.repository else { return };
        if !repo.core.picker.shown { repo.core.execute(R::TogglePicker); }
        if !repo.core.picker.sidebar { repo.core.execute(R::ToggleSidebar); }
        repo.core.execute(R::Focus(lenz::repository_session::Pane::Sidebar));
        self.navigator.focused = false;
    }

    /// Back (`forward` false) or Forward: the session restores the log and selection; the file and line follow once
    /// the comparison settles, as a launch's saved place does.
    fn navigate(&mut self, forward: bool) {
        if self.repository.is_none() { return; }
        if let Some(why) = Availability::disabled(&*self, Action::Repository(if forward { RepoAction::Forward } else { RepoAction::Back })) { self.session.set_notice(why); return; }
        let Some(repo) = &mut self.repository else { return };
        if forward { repo.core.go_forward() } else { repo.core.go_back() }
        if let Some(restore) = repo.core.take_nav_restore() { self.pending_place = Some(restore); }
    }

    /// Files, Branches, History and Focus through the shared layout.
    pub(crate) fn layout_command(&mut self, command: lenz::layout::LayoutCommand) {
        use lenz::layout::LayoutCommand as L;
        if let Some(why) = self.layout.disabled(command, self.repository.as_ref().map(|r| &r.core)) { self.session.set_notice(why); return; }
        if command == L::ToggleFiles && !self.navigator.can_show() && !self.navigator.shown() { self.begin(Prompt::Files); return; }
        // until a command decides it, Files follows the Change Set (shown for several files or a repository)
        // what Files shows by default, a list hidden for a PR's reading tab included
        let default = self.repository.as_ref().is_some_and(|r| r.core.picker.shown) || self.session.comparisons().len() > 1;
        if self.navigator.visible != Some(self.layout.files()) { self.layout.set_files(self.navigator.visible.unwrap_or(default)); }
        self.layout.execute(command, self.repository.as_mut().map(|r| &mut r.core));
        // details are a Focus tab: the Overview shows the diff
        if !self.layout.focused(self.repository.as_ref().map(|r| &r.core)) && let Some(pr) = self.repository.as_mut().and_then(|r| r.core.pr.as_mut()) && pr.details().is_some() {
            pr.execute(lenz::pr_session::PrCommand::Close);
        }
        self.navigator.visible = Some(self.layout.files());
        if !self.layout.files() { self.navigator.focused = false; }
    }

    fn open_settings(&mut self) { let notice = open_settings(); self.session.set_notice(notice); }

}

impl Availability for App {
    fn disabled(&self, action: Action) -> Option<Cow<'static, str>> {
        match action {
            Action::Comparison(lenz::commands::Command::SetDiffLayout(lenz::commands::DiffLayout::Unified)) => self.unified_disabled().map(Cow::Borrowed),
            Action::Comparison(command) => self.session.disabled(command).map(Cow::Borrowed),
            Action::Interaction(command) => self.interaction_disabled(command).map(Cow::Borrowed),
            Action::Edit(command) => self.session.edit_disabled(command).map(Cow::Owned),
            Action::File(file) => {
                let why = if file == lenz::file_actions::FileAction::UndoLenz { self.session.markdown_edit_disabled() } else { None };
                why.or_else(|| self.file_disabled(file, self.session.current())).map(Cow::Borrowed)
            }
            Action::Image(index) => lenz::image_session::image_commands().get(index).zip(self.visual.image()).and_then(|(e, doc)| e.disabled(doc, self.visual.image_state())).map(Cow::Borrowed),
            Action::Repository(RepoAction::Refresh | RepoAction::Branches) => None,
            Action::Repository(RepoAction::Filter) => None,
            Action::Repository(action @ (RepoAction::StagedChanges | RepoAction::CopyUrl | RepoAction::OpenUrl | RepoAction::LocalChanges
                | RepoAction::AllBranches | RepoAction::CurrentBranch | RepoAction::CompareBranch { .. } | RepoAction::CompareCommit
                | RepoAction::NextParent | RepoAction::FileHistory | RepoAction::WriteGraph | RepoAction::CopyHash | RepoAction::CopySubject
                | RepoAction::CopyDetails | RepoAction::ShowInGraph | RepoAction::OpenWorktree | RepoAction::ToggleMessage)) => {
                let path = self.session.comparisons().get(self.session.current()).map_or("", |c| c.path.as_str());
                self.repository.as_ref().and_then(|r| r.core.action_command(action, path).err()).map(Cow::Borrowed)
            }
            Action::Repository(RepoAction::Back) => (!self.repository.as_ref().is_some_and(|r| r.core.can_go_back())).then_some(Cow::Borrowed("Nowhere to go back to")),
            Action::Repository(RepoAction::Forward) => (!self.repository.as_ref().is_some_and(|r| r.core.can_go_forward())).then_some(Cow::Borrowed("Nowhere to go forward to")),
            Action::Pr(_) => None,
            Action::Merge(_) | Action::Pick(_) => Some(Cow::Borrowed("Only in the merge view")),
            Action::Layout(command) => self.layout.disabled(command, self.repository.as_ref().map(|r| &r.core)).map(Cow::Borrowed),
            Action::Help(_) => None,
        }
    }
}

/// The catalog's key hints, with the terminal's keys where they differ from the GUI's (`f` opens Find file here).
pub(crate) fn terminal_keys(mut entries: Vec<catalog::Entry>) -> Vec<catalog::Entry> {
    for entry in &mut entries {
        if entry.action == Action::Layout(lenz::layout::LayoutCommand::ToggleFocus) { entry.shortcut = Some("alt-0"); }
        // Ctrl+[ is Escape in a terminal
        if entry.action == Action::Comparison(lenz::commands::Command::PreviousFile) { entry.shortcut = Some("["); }
        if entry.action == Action::Comparison(lenz::commands::Command::NextFile) { entry.shortcut = Some("]"); }
    }
    entries
}

/// Open the settings file with the system's default app, first creating the commented template when there is none;
/// the notice to show.
pub(crate) fn open_settings() -> String {
    let path = match lenz::settings::ensure_template() { Ok(path) => path, Err(e) => return format!("Cannot create settings: {e}") };
    #[cfg(test)]
    return format!("Settings: {}", path.display());
    #[cfg(not(test))]
    {
        let plan = lenz::open::plan(&lenz::open::Host::current(), &lenz::open::Target { file: &path, binary: false, line: None, column: None, cwd: None }, true);
        if let Err(e) = lenz::open::spawn(&plan.launch) { return format!("Cannot start {}: {e}", plan.launch.program); }
        plan.toast.unwrap_or_else(|| format!("Settings apply on the next start: {}", path.display()))
    }
}

/// The palette box: centred under the toolbar in `area`, as tall as its rows allow.
pub(crate) fn rect(area: Rect, p: &Palette) -> Rect {
    let w = 70.min(area.width.saturating_sub(2));
    let h = (p.len().max(1) as u16 + 3).min(area.height.saturating_sub(2));
    Rect::new(area.x + (area.width - w) / 2, area.y + 1.min(area.height), w, h)
}

/// Query line, then one row per matching entry; disabled rows dim with their reason.
pub(crate) fn paint(p: &mut Palette, frame: &mut ratatui::Frame, rect: Rect, palette: &lenz::theme::Palette) {
    if rect.width < 4 || rect.height < 3 { return; }
    let base = crate::chrome::style_with(palette);
    frame.render_widget(Clear, rect);
    frame.render_widget(Block::default().borders(Borders::ALL).style(base), rect);
    let inner = Rect::new(rect.x + 1, rect.y + 1, rect.width - 2, rect.height - 2);
    let buf = frame.buffer_mut();
    crate::visual::safe_text(buf, Rect { height: 1, ..inner }, &format!("Commands: {}▏", p.query()), base.fg(color(palette.accent)));
    let range = p.window(usize::from(inner.height.saturating_sub(1)));
    let selected = p.selected();
    if p.is_empty() && inner.height > 1 { crate::visual::safe_text(buf, Rect::new(inner.x, inner.y + 1, inner.width, 1), "No matching commands", base.add_modifier(Modifier::DIM)); }
    for (row, (i, entry)) in p.rows().enumerate().skip(range.start).take(range.len()).enumerate() {
        let line = Rect::new(inner.x, inner.y + 1 + row as u16, inner.width, 1);
        let mut style = if selected == Some(i) { base.bg(color(palette.selected_bg)) } else { base };
        if entry.disabled.is_some() { style = style.add_modifier(Modifier::DIM); }
        let mark = if entry.entry.current { "✓ " } else { "  " };
        let reason = entry.disabled.as_deref().map_or(String::new(), |why| format!(" ({why})"));
        let right = [Some(entry.entry.group.label()), entry.entry.shortcut].into_iter().flatten().collect::<Vec<_>>().join("  ");
        buf.set_style(line, style);
        crate::visual::safe_text(buf, line, &format!("{mark}{}{reason}  · {right}", entry.entry.label), style);
    }
}

/// What an event did to an open palette.
pub(crate) enum Input { Ignored, Handled, Close, Done(Outcome) }

/// Keys, wheel, clicks and paste while the palette in `rect` is open; it takes every key and click (it is modal).
pub(crate) fn handle(p: &mut Palette, event: &Event, rect: Rect) -> Input {
    match event {
        Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
            KeyCode::Esc => return Input::Close,
            KeyCode::Enter => return Input::Done(p.run()),
            KeyCode::Up => p.command(PaletteCommand::Move(-1)),
            KeyCode::Down => p.command(PaletteCommand::Move(1)),
            KeyCode::PageUp => p.command(PaletteCommand::Move(-8)),
            KeyCode::PageDown => p.command(PaletteCommand::Move(8)),
            KeyCode::Home => p.command(PaletteCommand::Home),
            KeyCode::End => p.command(PaletteCommand::End),
            KeyCode::Backspace => p.command(PaletteCommand::Backspace),
            KeyCode::Char(c) if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => p.command(PaletteCommand::Type(c.to_string())),
            _ => {}
        },
        Event::Key(_) => {}
        Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::ScrollUp => p.command(PaletteCommand::Move(-3)),
            MouseEventKind::ScrollDown => p.command(PaletteCommand::Move(3)),
            MouseEventKind::Down(MouseButton::Left) => {
                if !rect.contains((mouse.column, mouse.row).into()) { return Input::Close; }
                let first_row = rect.y + 2;
                let range = p.window(usize::from(rect.height.saturating_sub(3)));
                if mouse.row >= first_row && usize::from(mouse.row - first_row) < range.len() {
                    p.command(PaletteCommand::Select(range.start + usize::from(mouse.row - first_row)));
                    return Input::Done(p.run());
                }
            }
            _ => {}
        },
        Event::Paste(text) => p.command(PaletteCommand::Type(text.chars().filter(|c| !c.is_control()).collect())),
        _ => return Input::Ignored,
    }
    Input::Handled
}

pub(crate) fn draw(app: &mut App, frame: &mut ratatui::Frame) {
    let theme = app.palette.clone();
    let area = app.popup_area;
    if let Some(p) = &mut app.command_palette { let rect = rect(area, p); paint(p, frame, rect, &theme); }
}

pub(crate) fn input(app: &mut App, event: &Event) -> bool {
    let area = app.popup_area;
    let Some(p) = &mut app.command_palette else { return false };
    match handle(p, event, rect(area, p)) {
        Input::Ignored => return false,
        Input::Handled => {}
        Input::Close => app.command_palette = None,
        Input::Done(Outcome::Run(action)) => app.run_action(action),
        Input::Done(Outcome::Disabled(why)) => app.session.set_notice(why.into_owned()),
        Input::Done(Outcome::Nothing) => {}
    }
    true
}
