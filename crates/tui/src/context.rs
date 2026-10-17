//! Terminal menus translate pointer and keyboard input to shared semantic commands.
use crate::app::App;
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use lenz::{
    commands::Interaction,
    file_actions::FileAction,
    repository_session::{RepositoryCommand, command_label},
};
use ratatui::{
    layout::Rect,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use std::sync::Arc;
enum Action {
    Palette,
    Run(lenz::catalog::Action),
    Fold(usize),
    Apply(usize, lenz::view::Side),
    File(FileAction, usize, Arc<lenz::changeset::ChangeSet>),
    Text(Interaction),
    Repository(RepositoryCommand),
    /// The branches to compare the context commit with, as a second menu.
    CompareWithBranch,
    /// Put this text on the clipboard.
    Copy(String),
}
struct Item {
    label: String,
    reason: Option<String>,
    action: Action,
}
#[derive(Default)]
pub(crate) struct Menu {
    items: Vec<Item>,
    selected: usize,
    first: usize,
    area: Rect,
    question_scroll: u16,
    commit: Option<String>,
}
impl App {
    pub(crate) fn clear_context(&mut self){self.context.items.clear();self.session.cancel_discard();}
    pub(crate) fn modal_input(&self)->bool{
        !self.context.items.is_empty()||self.command_palette.is_some()||self.session.file_actions().busy()||self.session.file_actions().confirmation().is_some()||self.prompt.is_some()||self.about.is_some()||self.message.is_some()||self.visual.commit_visible()||self.repository.as_ref().is_some_and(|r|r.modal())||self.set_search.as_ref().is_some_and(|s|s.model.editing())
    }
    pub(crate) fn context_details(&self)->Option<String> {
        self.session.file_actions().confirmation().or_else(||self.context.items.get(self.context.selected).map(|item|item.reason.as_ref().map_or_else(||item.label.clone(),|reason|format!("{} — {reason}",item.label))))
    }
    pub(crate) fn file_disabled(&self, action: FileAction, index: usize) -> Option<&'static str> {
        let set = self.session.change_set()?;
        if matches!(action,FileAction::Discard|FileAction::UndoDiscard)&&self.session.file_has_draft(index){return Some("Save or discard unsaved changes first");}
        self.session.file_actions().disabled(
            action,
            &set,
            index,
            self.session.spec(),
            self.session.has_file_undo(index),
        )
    }
    pub(crate) fn file_action(&mut self, action: FileAction, index: usize) {
        if let Some(reason) = self.file_disabled(action, index) {
            self.session.set_notice(reason);
            return;
        }
        let Some(set) = self.session.change_set() else {
            return;
        };
        let Some(c) = set.comparisons.get(index) else {
            return;
        };
        let result = match action {
            FileAction::CopyPath => crate::clipboard::copy(&c.path)
                .map(|()| {
                    self.session
                        .set_notice("Copied file path via terminal OSC 52")
                })
                .map_err(|e| e.to_string()),
            FileAction::Editor => {
                let side = if c.status == lenz::changeset::Status::Deleted {
                    lenz::view::Side::Left
                } else {
                    lenz::view::Side::Right
                };
                self.open_editor_at(index, side);
                Ok(())
            }
            FileAction::History => {
                if let Some(repo) = &mut self.repository {
                    repo.core
                        .execute(RepositoryCommand::FileHistory(c.repository_path().to_owned()));
                } else if let Some(root) = set.git_root() {
                    let args = vec!["log".into(), "--follow".into(), c.repository_path().to_owned()];
                    if let Ok(request) = lenz::cli::parse(&args, root) {
                        self.new_view = Some((request, root.into()));
                    }
                }
                Ok(())
            }
            FileAction::UndoLenz => self
                .session
                .undo_file(index)
                .map(|_| self.session.reload_set()),
            FileAction::Discard => self.session.request_discard(index),
            FileAction::UndoDiscard => self.session.undo_discard(index),
        };
        if let Err(error) = result {
            self.session.set_notice(error);
        }
    }
    pub(crate) fn context_input(&mut self, event: &Event) -> bool {
        if matches!(event,Event::Key(k)if k.modifiers.contains(KeyModifiers::CONTROL)&&!k.modifiers.contains(KeyModifiers::SHIFT)&&matches!(k.code,KeyCode::Char('q'|'c')))
        {
            return false;
        }
        if self.session.file_actions().busy() {
            return true;
        }
        if self.session.file_actions().confirmation().is_some() {
            if let Event::Key(k) = event
                && k.kind != KeyEventKind::Release
            {
                match k.code {
                    KeyCode::Esc => {
                        self.session.cancel_discard();
                        self.context.question_scroll = 0;
                    }
                    KeyCode::Enter => {
                        if let Err(error) = self.session.confirm_discard() {
                            self.session.set_notice(error);
                        }
                        self.context.question_scroll = 0;
                    }
                    KeyCode::Down | KeyCode::PageDown => {
                        self.context.question_scroll =
                            self.context.question_scroll.saturating_add(1)
                    }
                    KeyCode::Up | KeyCode::PageUp => {
                        self.context.question_scroll =
                            self.context.question_scroll.saturating_sub(1)
                    }
                    KeyCode::Home => self.context.question_scroll = 0,
                    KeyCode::End => self.context.question_scroll = u16::MAX,
                    _ => {}
                }
            }
            if let Event::Mouse(m) = event {
                match m.kind {
                    MouseEventKind::ScrollDown => {
                        self.context.question_scroll =
                            self.context.question_scroll.saturating_add(3)
                    }
                    MouseEventKind::ScrollUp => {
                        self.context.question_scroll =
                            self.context.question_scroll.saturating_sub(3)
                    }
                    MouseEventKind::Down(MouseButton::Left)
                        if self.context.area.contains((m.column, m.row).into())
                            && m.row == self.context.area.bottom().saturating_sub(2) =>
                    {
                        if m.column < self.context.area.x + self.context.area.width / 2 {
                            self.session.cancel_discard();
                        } else if let Err(error) = self.session.confirm_discard() {
                            self.session.set_notice(error);
                        }
                    }
                    _ => {}
                }
            }
            let width = usize::from(self.context.area.width.saturating_sub(2));
            let visible = usize::from(self.context.area.height.saturating_sub(3));
            let max = self
                .session
                .file_actions()
                .confirmation()
                .map_or(0, |question| {
                    wrapped_lines(&question, width).saturating_sub(visible)
                });
            self.context.question_scroll = self
                .context
                .question_scroll
                .min(max.min(u16::MAX as usize) as u16);
            return true;
        }
        if !self.context.items.is_empty() {
            let mut activate = false;
            let mut close = false;
            let last = self.context.items.len() - 1;
            match event {
                Event::Key(k) if k.kind != KeyEventKind::Release => match k.code {
                    KeyCode::Esc => close = true,
                    KeyCode::Enter => activate = true,
                    KeyCode::Up => self.context.selected = self.context.selected.saturating_sub(1),
                    KeyCode::Down => self.context.selected = (self.context.selected + 1).min(last),
                    KeyCode::Home => self.context.selected = 0,
                    KeyCode::End => self.context.selected = last,
                    KeyCode::PageUp => {
                        self.context.selected = self.context.selected.saturating_sub(8)
                    }
                    KeyCode::PageDown => {
                        self.context.selected = (self.context.selected + 8).min(last)
                    }
                    _ => {}
                },
                Event::Mouse(m) => match m.kind {
                    MouseEventKind::ScrollUp => {
                        self.context.selected = self.context.selected.saturating_sub(3)
                    }
                    MouseEventKind::ScrollDown => {
                        self.context.selected = (self.context.selected + 3).min(last)
                    }
                    MouseEventKind::Down(MouseButton::Left) => {
                        if self.context.area.contains((m.column, m.row).into())
                            && m.row > self.context.area.y
                            && m.row < self.context.area.bottom().saturating_sub(2)
                        {
                            self.context.selected = (self.context.first
                                + usize::from(m.row - self.context.area.y - 1))
                            .min(last);
                            activate = true;
                        } else {
                            close = true;
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
            if activate {
                let item = self.context.items.remove(self.context.selected);
                self.context.items.clear();
                if let Some(reason) = item.reason {
                    self.session.set_notice(reason);
                } else {
                    match item.action {
                        Action::Palette => self.open_palette(),
                        Action::Run(action) => self.run_action(action),
                        Action::Fold(block) => self.session.fold_block(block),
                        Action::Apply(block, onto) => self.session.edit(lenz::edit_session::EditCommand::Apply { block, onto }),
                        Action::Text(command) => self.interaction(command),
                        Action::Copy(text) => self.copy(&text),
                        Action::File(action, index, set) => {
                            if self
                                .session
                                .change_set()
                                .is_some_and(|current| Arc::ptr_eq(&current, &set))
                            {
                                self.file_action(action, index)
                            } else {
                                self.session.set_notice("Files changed — reopen the menu");
                            }
                        }
                        Action::CompareWithBranch => {
                            let targets = self.repository.as_ref().map_or_else(Vec::new, |r| r.context_branch_targets());
                            self.context.items = targets
                                .into_iter()
                                .map(|(label, command)| Item { label, reason: None, action: Action::Repository(command) })
                                .collect();
                            (self.context.selected, self.context.first) = (0, 0);
                        }
                        Action::Repository(command) => {
                            if let Some(repo) = &mut self.repository {
                                let current=repo.context_commit();
                                if current == self.context.commit {
                                    repo.execute_context(command);
                                } else {
                                    self.session.set_notice("Commits changed — reopen the menu");
                                }
                            }
                        }
                    }
                }
            }
            if close {
                self.context.items.clear();
            }
            return true;
        }
        let point = match event {
            Event::Mouse(m) if m.kind == MouseEventKind::Down(MouseButton::Right) => {
                Some((m.column, m.row))
            }
            Event::Key(k)
                if k.kind != KeyEventKind::Release
                    && k.code == KeyCode::F(10)
                    && k.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                None
            }
            _ => return false,
        };
        // An existing dialog owns right click and Shift-F10 too.
        if self.prompt.is_some()
            || self.about.is_some()
            || self.repository.as_ref().is_some_and(|r| r.modal())
            || self.set_search.as_ref().is_some_and(|s| s.model.editing())
            || self.visual.commit_visible()
        {
            return true;
        }
        let file = point
            .and_then(|(x, y)| self.navigator.file_at(x, y))
            .or_else(|| {
                (point.is_none() && self.navigator.focused).then_some(self.navigator.candidate())
            });
        self.context.selected = 0;
        self.context.first = 0;
        self.context.commit = None;
        let commit = point.zip(self.repository.as_ref()).and_then(|((x, y), r)| self.commit_panel.commit_at(x, y, &r.core));
        // over a PR tab: only its card's copy menu; the diff under it is hidden
        let pane = self.pr_pane();
        if self.pr_details_shown() && let Some((x, y)) = point.filter(|(x, y)| pane.contains((*x, *y).into())) {
            let card = self.repository.as_ref().and_then(|r| r.pr_dialog.card_at(x, y));
            let Some(card) = card else { return true };
            self.context.items = vec![Item { label: "Copy comment".into(), reason: None, action: Action::Copy(card.body) }];
            if !card.url.is_empty() { self.context.items.push(Item { label: "Copy link to comment".into(), reason: None, action: Action::Copy(card.url) }); }
        } else if let Some(entries) = point.and_then(|p| self.repository.as_ref().and_then(|r| r.source_menu(p))) {
            // a sidebar row: only its own entries, and no menu without any
            if entries.is_empty() { return true; }
            self.context.items = entries.into_iter().map(|(label, action)| Item { label: label.into(), reason: lenz::catalog::Availability::disabled(&*self, action).map(|r| r.into_owned()), action: Action::Run(action) }).collect();
        } else if let Some((message, sha)) = commit {
            self.context.items = vec![
                Item { label: "Copy commit message".into(), reason: None, action: Action::Copy(message) },
                Item { label: "Copy commit hash".into(), reason: None, action: Action::Copy(sha) },
            ];
        } else if let Some(index) = file
            && let Some(set) = self.session.change_set()
        {
            self.context.items = FileAction::ALL
                .iter()
                .map(|&action| Item {
                    label: action.label().into(),
                    reason: self.file_disabled(action, index).map(str::to_string),
                    action: Action::File(action, index, set.clone()),
                })
                .collect();
        } else if let Some(commands) = self
            .repository
            .as_mut()
            .and_then(|r| r.context_commands(point))
        {
            self.context.commit = self.repository.as_ref().and_then(|r| {
                r.context_commit()
            });
            self.context.items = commands
                .into_iter()
                .map(|command| Item {
                    label: command_label(&command).into(),
                    reason: self
                        .repository
                        .as_ref()
                        .and_then(|r| r.context_disabled(&command))
                        .map(str::to_string),
                    action: Action::Repository(command),
                })
                .collect();
            let no_branches = self.repository.as_ref().is_none_or(|r| r.context_branch_targets().is_empty());
            self.context.items.push(Item {
                label: "Compare with Branch…".into(),
                reason: no_branches.then(|| "No commit or branches".into()),
                action: Action::CompareWithBranch,
            });
        } else if point.is_none()
            || point.is_some_and(|(x, y)| self.diff_area.contains((x, y).into()))
        {
            // only what is under the pointer; everything else is in the palette
            let text=|app:&App,command:Interaction|Item{label:command.label().into(),reason:app.interaction_disabled(command).map(str::to_string),action:Action::Text(command)};
            if self.session.markdown_mode()==lenz::markdown::PresentationMode::Rendered {
                if let Some((x,y))=point {self.markdown_context_pointer(x,y);}
                let source=lenz::commands::Command::SetPresentation(lenz::markdown::PresentationMode::Source);
                self.context.items=vec![Item{label:lenz::catalog::pointer::SHOW_IN_SOURCE.into(),reason:None,action:Action::Run(lenz::catalog::Action::Comparison(source))},text(self,Interaction::Copy),text(self,Interaction::Editor)];
                if self.visual.blame_enabled(self.focus){self.context.items.push(text(self,Interaction::InspectBlame));}
            } else {
                if self.session.markdown_edit_disabled().is_none() && let Some((x, y)) = point
                    && let Some((side, pos, _)) = self.mouse_position(x, y)
                {
                    self.focus = side;
                    if self.selection.is_none_or(|s| s.is_empty()) {
                        self.selection = Some(lenz::selection::Selection::caret(side, pos));
                    }
                }
                self.context.items.clear();
                if self.visual.image().is_none() && self.selection.is_some_and(|s| !s.is_empty()) {
                    for command in [Interaction::Copy, Interaction::CopyReference] {if self.interaction_disabled(command).is_none(){self.context.items.push(text(self,command));}}
                }
                let block=match point {Some((x,y))=>self.block_at(x,y),None=>self.session.view().current_change()};
                if let Some(block)=block.filter(|_|self.visual.image().is_none()) {
                    if self.session.view().can_fold(block) {
                        let label=lenz::catalog::pointer::fold(self.session.view().folded(block));
                        self.context.items.push(Item{label:label.into(),reason:None,action:Action::Fold(block)});
                    }
                    let writable=self.session.document().map_or([false;2],|doc|[lenz::view::Side::Left,lenz::view::Side::Right].map(|side|doc.is_writable(side)));
                    if writable[1] {self.context.items.push(Item{label:lenz::catalog::pointer::move_change(lenz::view::Side::Right).into(),reason:None,action:Action::Apply(block,lenz::view::Side::Right)});}
                    if writable[0] {self.context.items.push(Item{label:lenz::catalog::pointer::move_change(lenz::view::Side::Left).into(),reason:None,action:Action::Apply(block,lenz::view::Side::Left)});}
                }
                self.context.items.push(text(self,Interaction::Editor));
                if self.visual.blame_enabled(self.focus) {
                    if self.interaction_disabled(Interaction::InspectBlame).is_none(){self.context.items.push(text(self,Interaction::InspectBlame));}
                    self.context.items.push(Item{label:lenz::catalog::pointer::HIDE_ANNOTATIONS.into(),reason:None,action:Action::Text(Interaction::Blame)});
                } else if self.interaction_disabled(Interaction::Blame).is_none() {
                    self.context.items.push(Item{label:lenz::catalog::pointer::ANNOTATE_WITH_BLAME.into(),reason:None,action:Action::Text(Interaction::Blame)});
                }
            }
            self.context.items.push(Item{label:"All commands…".into(),reason:None,action:Action::Palette});


        }
        true
    }
    pub(crate) fn draw_context(&mut self, frame: &mut ratatui::Frame) {
        let palette = &self.palette;
        let bounds = frame.area();
        if bounds.width < 3 || bounds.height < 3 {
            self.context.question_scroll = 0;
            self.context.area = Rect::default();
            return;
        }
        let question = self.session.file_actions().confirmation();
        if question.is_none() && self.context.items.is_empty() {
            return;
        }
        let width = bounds.width.min(70);
        let height = if question.is_some() {
            bounds.height.min(14)
        } else {
            bounds.height.min(self.context.items.len() as u16 + 3)
        };
        let area = Rect::new(
            bounds.x + (bounds.width - width) / 2,
            bounds.y + (bounds.height - height) / 2,
            width,
            height,
        );
        self.context.area = area;
        frame.render_widget(Clear, area);
        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .title(if question.is_some() {
                    "F2 details · Discard"
                } else {
                    "F2 details · Actions"
                })
                .style(
                    crate::chrome::style_with(palette).bg(crate::chrome::color(palette.status_bg)),
                ),
            area,
        );
        let inner = Rect::new(area.x + 1, area.y + 1, width - 2, height.saturating_sub(3));
        if let Some(question) = question {
            let lines = wrapped_lines(&question, usize::from(inner.width));
            let paragraph = Paragraph::new(question).wrap(Wrap { trim: false });
            self.context.question_scroll = self
                .context
                .question_scroll
                .min(lines.saturating_sub(usize::from(inner.height)) as u16);
            frame.render_widget(paragraph.scroll((self.context.question_scroll, 0)), inner);
            crate::visual::safe_text(
                frame.buffer_mut(),
                Rect::new(area.x + 1, area.bottom() - 2, width - 2, 1),
                "Cancel (Esc)          Discard (Enter)",
                crate::chrome::style_with(palette),
            );
        } else {
            let visible = usize::from(inner.height);
            self.context.first = if self.context.selected < self.context.first {
                self.context.selected
            } else if self.context.selected >= self.context.first + visible {
                self.context
                    .selected
                    .saturating_add(1)
                    .saturating_sub(visible)
            } else {
                self.context.first
            };
            for (i, item) in self
                .context
                .items
                .iter()
                .enumerate()
                .skip(self.context.first)
                .take(visible)
            {
                let rect = Rect::new(
                    inner.x,
                    inner.y + (i - self.context.first) as u16,
                    inner.width,
                    1,
                );
                let style = crate::chrome::style_with(palette)
                    .fg(crate::chrome::color(if item.reason.is_some() {
                        palette.dim
                    } else {
                        palette.fg
                    }))
                    .bg(crate::chrome::color(if i == self.context.selected {
                        palette.selected_bg
                    } else {
                        palette.status_bg
                    }));
                frame.buffer_mut().set_style(rect, style);
                crate::visual::safe_text(frame.buffer_mut(), rect, &item.label, style);
            }
            let reason = self
                .context
                .items
                .get(self.context.selected)
                .and_then(|i| i.reason.as_deref())
                .unwrap_or("F2 details · Enter selects · Esc closes");
            crate::visual::safe_text(
                frame.buffer_mut(),
                Rect::new(area.x + 1, area.bottom() - 2, width - 2, 1),
                reason,
                crate::chrome::style_with(palette),
            );
        }
    }
}

fn wrapped_lines(text: &str, width: usize) -> usize {
    use unicode_width::UnicodeWidthStr;
    if width == 0 {
        return 0;
    }
    let mut count = 0;
    let mut column = 0;
    for word in text.split_inclusive(' ') {
        let n = word.width();
        if column > 0 && column + n > width {
            count += 1;
            column = 0;
        }
        column += n;
        while column > width {
            count += 1;
            column -= width;
        }
    }
    count + usize::from(column > 0)
}
