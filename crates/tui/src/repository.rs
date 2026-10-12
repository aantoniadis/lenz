//! Terminal repository/history and read-only PR adapter. Behavior lives in the shared core.
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use lenz::{
    changeset::{ChangeSet, Spec},
    cli::Request,
    history::Branches,
    repository_session::{
        Pane, RepositoryCommand as Command, RepositoryEffect, RepositorySession, SourceAction,
        SourcePalette,
    },
};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier},
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use std::path::PathBuf;

const HELP: &str = "Repository commands (F6/Tab cycles Sidebar / Commits / Files / Diff):\nCtrl-R picker; Ctrl-S sidebar; Ctrl-L log; F5 refresh; F10 command palette; Shift-F10 context menu\nSidebar: arrows select; Enter opens; / filters; Esc clears\nc compare selected branch against current; C tips; o open worktree; a all; b current\nCommits/Files: type or Ctrl-F to find without changing the comparison\nUp/Down or F3/Shift-F3 step matches; Enter opens; Esc cancels\nCtrl-A clears query; Ctrl-C copies commit hash or file path; Ctrl-Q quits\nAll rows stay visible and matches are highlighted. F1 opens help.\nOutside search, Shift-arrows select a range; Ctrl-arrows select a second commit.\nHistory filters and actions are in the F10 command palette and context menus.\nPR controls in the comparison: P open; r refresh; d details; v viewed; t/T threads\nPR review is read-only. Viewed Marks are saved locally.";
#[derive(Clone, Copy, PartialEq)]
enum Prompt {
    Filter,
    Sources,
    Pr,
}

pub struct Repository {
    pub core: RepositorySession,
    pub quit: bool,
    prompt: Option<Prompt>,
    input: String,
    command_selected: usize,
    command_hits:Vec<(usize,Rect)>,prompt_area:Rect,
    overlay_scroll: u16,
    pub(crate) pr_dialog:crate::pr_details::Dialog,
    current: usize,
    current_path: String,
    /// The repository path git follows for the file on screen: the archive's for a file inside one.
    history_path: String,
    size: (u16, u16),
    bounds:Rect,
    effects: Vec<RepositoryEffect>,
    /// F10 or `:` in a repository pane: the App opens the command palette.
    pub(crate) palette_request: bool,
    /// The log rows and title last drawn, for a restarted log's hold.
    held_log: Option<(Vec<ratatui::text::Line<'static>>,String)>,
    pub(crate) interaction:Option<lenz::commands::Interaction>,
    context_row:Option<u32>,
    log_search_origin:Option<f32>,
    manual_scroll: [bool; 2],
    pub(crate) source_width:Option<f32>,
    pub(crate) log_height:Option<f32>,
    pub(crate) resizer:crate::panes::Resizer,
}

impl Repository {
    pub fn open(request: Request, cwd: PathBuf) -> Self {
        Self {
            core: RepositorySession::open(&request, cwd),
            quit: false,
            prompt: None,
            input: String::new(),
            command_selected: 0,command_hits:Vec::new(),prompt_area:Rect::default(),
            overlay_scroll: 0,pr_dialog:Default::default(),
            current: 0,
            current_path: String::new(),
            history_path: String::new(),
            size: (80, 24),
            bounds:Rect::new(0,0,80,24),
            effects: Vec::new(),held_log:None,palette_request:false,interaction:None,
            context_row:None,log_search_origin:None, manual_scroll: [false; 2],source_width:None,log_height:None,resizer:Default::default(),
        }
    }
    pub fn poll(&mut self) -> bool {
        let mut changed=self.core.poll();
        let before=self.core.picker.pane_search.current();
        changed|=self.core.picker.poll_pane_search();
        if self.core.picker.pane_search.current()!=before{self.reveal_log_search();}
        changed
    }
    pub fn busy(&self) -> bool {
        self.core.busy()||self.core.picker.pane_search.searching(self.core.picker.rows()as usize)
    }
    pub fn take_change_set(&mut self) -> Option<(std::sync::Arc<ChangeSet>, Spec)> {
        self.core.take_change_set()
    }
    pub fn take_effects(&mut self) -> Vec<RepositoryEffect> {
        let mut effects = self.core.take_effects();
        effects.append(&mut self.effects);
        effects
    }
    pub fn set_current(&mut self, index: usize, path: &str, history_path: &str) {
        self.current = index;
        self.current_path = path.into();
        self.history_path = history_path.into();
    }
    pub fn area(&self, area: Rect) -> Rect {
        let picker = &self.core.picker;
        if !picker.shown && self.core.pr.is_none() {
            return area;
        }
        let left = if picker.shown && picker.sidebar {
            crate::panes::size(area.width,self.source_width.unwrap_or(f32::from((area.width / 3).min(30))),12.0,32.0)
        } else {
            0
        };
        let top = if picker.shown && picker.log_shown {
            crate::panes::size(area.height,self.log_height.unwrap_or(f32::from((area.height / 3).max(3))),3.0,4.0)
        // the PR's line over Files changed; a reading tab has the PR's own header
        } else if self.core.pr.as_ref().is_some_and(|p| p.details().is_none()) {
            1.min(area.height)
        } else {
            0
        };
        Rect::new(
            area.x + left,
            area.y + top,
            area.width.saturating_sub(left),
            area.height.saturating_sub(top),
        )
    }
    pub(crate) fn reset_pane_sizes(&mut self) {self.source_width=None;self.log_height=None;self.resizer.cancel();}
    pub(crate) fn resize_input(&mut self,event:&Event,blocked:bool)->bool {
        let blocked=blocked||self.modal();
        match self.resizer.handle(event,blocked) {
            crate::panes::Resize::None=>false,crate::panes::Resize::Captured=>true,
            crate::panes::Resize::Move(0,width)=>{self.source_width=Some((width+1.0).max(0.0));true},
            crate::panes::Resize::Move(_,height)=>{self.log_height=Some((height+1.0).max(0.0));true}
        }
    }
    fn execute(&mut self, command: Command) {
        self.core.execute(command);
    }
    /// End a commit search as Esc does, back at the accepted position.
    pub(crate) fn cancel_log_search(&mut self) {
        if !self.core.picker.pane_search.active() { return; }
        self.core.picker.pane_search.cancel();
        if let Some(scroll) = self.log_search_origin.take() { self.core.picker.scroll = scroll; }
        self.manual_scroll[1] = true;
    }
    /// The history filter prompt, in the log.
    pub(crate) fn begin_filter(&mut self) {
        if !self.core.picker.shown || !self.core.picker.log_shown { self.core.execute(Command::SetPanels { branches: self.core.picker.shown && self.core.picker.sidebar, history: true }); }
        self.core.execute(Command::Focus(lenz::repository_session::Pane::Log));
        self.begin(Prompt::Filter);
    }
    fn begin(&mut self, prompt: Prompt) {
        self.prompt = Some(prompt);
        self.command_selected = 0;
        self.input = match prompt {
            Prompt::Filter => self.core.picker.filter.clone(),
            Prompt::Sources => self.core.picker.sources.filter.clone(),
            _ => String::new(),
        };
    }
    fn source(&self) -> Option<SourceAction> {
        self.core
            .picker
            .sources
            .rows_with_palette(&self.core.picker.branches, source_palette(&lenz::theme::DARCULA))
            .get(self.core.picker.sources.selected)
            .and_then(|row| row.action.clone())
    }
    fn compare(&mut self, merge_base: bool, selected: bool) {
        if let Some(SourceAction::Branch(right, _)) = self.source() {
            let left = if selected {
                self.core
                    .picker
                    .log_row(self.core.picker.anchor)
                    .map(|row| self.core.picker.log.id(row))
                    .unwrap_or_else(|| "HEAD".into())
            } else {
                "HEAD".into()
            };
            self.execute(Command::Compare {
                left,
                right,
                merge_base,
            });
        } else {
            self.core.notice = Some("Select a branch in the repository sidebar".into());
        }
    }
    fn worktree(&mut self) {
        if let Some(SourceAction::Worktree(path)) = self.source() {
            self.effects.push(RepositoryEffect::OpenWorktree(path));
        } else {
            self.core.notice = Some("Select a worktree in the repository sidebar".into());
        }
    }
    fn run_command(&mut self, name: &str) {
        if name==lenz::commands::Interaction::ReportLastError.label(){self.interaction=Some(lenz::commands::Interaction::ReportLastError);return;}
        if name==lenz::commands::Interaction::ResetPaneSizes.label(){self.reset_pane_sizes();self.interaction=Some(lenz::commands::Interaction::ResetPaneSizes);return;}
        if name==lenz::commands::Interaction::ChangeTheme.label(){self.interaction=Some(lenz::commands::Interaction::ChangeTheme);return;}
        if name==lenz::about::TITLE {self.interaction=Some(lenz::commands::Interaction::About);return;}
        match name {
            "filter"=>self.begin(Prompt::Filter),
            "help"=>self.core.overlay=Some(HELP.into()),
            "local" => self.execute(Command::Source(SourceAction::Local)),
            "staged" => self.execute(Command::Index),
            "all" => self.execute(Command::Walk(Branches::All)),
            "current" => self.execute(Command::Walk(Branches::Current)),
            "compare" => self.compare(true, false),
            "compare-tips" => self.compare(false, false),
            "compare-selected" => self.compare(false, true),
            "parent" => self.execute(Command::Parent(self.core.picker.merge_parent + 1)),
            "follow" => self.execute(Command::FileHistory(self.history_path.clone())),
            "graph" => self.execute(Command::WriteGraph),
            "hash" => self.execute(Command::CopyHash),
            "subject" => self.execute(Command::CopySubject),
            "show-in-graph" => self.execute(Command::ShowInGraph),
            "details" => self.execute(Command::CopyDetails),
            "url" => self.execute(Command::CopyUrl),
            "open-url" => self.execute(Command::OpenUrl),
            "worktree" => self.worktree(),
            "pr" => self.begin(Prompt::Pr),
            "refresh-repository" => self.execute(Command::RefreshRepository),
            "refresh" => self.execute(Command::RefreshPr),
            "body" => self.execute(Command::ToggleBody),
            "conversation"=>self.execute(Command::PrDetails(lenz::pr_session::DetailsTab::Conversation)),
            "commits"=>self.execute(Command::PrDetails(lenz::pr_session::DetailsTab::Commits)),
            "viewed" => self.execute(Command::Viewed {
                index: self.current,
                jump: true,
            }),
            "threads" | "outdated" => self.execute(Command::Threads {
                path: self.current_path.clone(),
                outdated: name == "outdated",
            }),
            "check-head" => self.execute(Command::CheckPrHead),
            "sidebar" => self.execute(Command::ToggleSidebar),
            "log" => self.execute(Command::ToggleLog),
            "focus" => self.execute(Command::TogglePicker),
            _ => self.core.notice = Some(format!("Unknown repository command: {name}")),
        }
    }
    pub(crate) fn context_commands(&mut self,point:Option<(u16,u16)>)->Option<Vec<Command>>{
        if !self.core.picker.shown{return None;}
        let area=self.bounds;let diff=self.area(area);let picker=&self.core.picker;
        let row=if let Some((x,y))=point{
            if !picker.log_shown||x<diff.x||x>=area.right()||y<=area.y||y>=diff.y.saturating_sub(1){return None;}
            u32::from(y-area.y-1)+picker.scroll.floor()as u32
        }else{if picker.focus!=Pane::Log{return None;}picker.pane_search.current().map(|i|i as u32).unwrap_or(picker.anchor)};
        if row>=picker.rows(){return None;}self.context_row=Some(row);
        Some(RepositorySession::log_context_commands(row,&self.history_path))
    }
    /// A sidebar row's right-click entries under `point` (the open PR's Refresh).
    pub(crate) fn source_menu(&self,(x,y):(u16,u16))->Option<Vec<(&'static str,lenz::catalog::Action)>>{
        let picker=&self.core.picker;let area=self.bounds;let diff=self.area(area);
        if !picker.shown||!picker.sidebar||x>=diff.x||!area.contains((x,y).into())||y<=area.y||y+1>=area.bottom(){return None;}
        Some(self.core.source_menu(usize::from(y-area.y-1)+picker.sources.scroll as usize))
    }
    /// "Compare with Branch…" for the context menu's commit.
    pub(crate) fn context_branch_targets(&self)->Vec<(String,Command)> {self.context_row.map_or_else(Vec::new,|row|self.core.branch_targets(row))}
    pub(crate) fn context_commit(&self)->Option<String> {let row=self.context_row?;self.core.picker.log_row(row).map(|i|self.core.picker.log.id(i))}
    pub(crate) fn context_disabled(&self,command:&Command)->Option<&'static str> {
        let row=self.context_row?;self.core.row_disabled(row,command)
    }
    pub(crate) fn execute_context(&mut self,command:Command) {
        let Some(row)=self.context_row else{return;};
        self.core.execute_on_row(row,command);
    }
    fn begin_log_search(&mut self) {
        let picker=&mut self.core.picker;
        if !picker.pane_search.active(){self.log_search_origin=Some(picker.scroll);picker.pane_search.start(picker.second.unwrap_or(picker.anchor)as usize);}
        self.manual_scroll[1]=true;
    }
    fn reveal_log_search(&mut self) {
        let visible=self.area(self.bounds).y.saturating_sub(self.bounds.y+2).max(1)as usize;
        let picker=&mut self.core.picker;
        if let Some(row)=picker.pane_search.current(){if row<picker.scroll as usize{picker.scroll=row as f32;}else if row>=picker.scroll as usize+visible{picker.scroll=row.saturating_add(1).saturating_sub(visible)as f32;}}
        self.manual_scroll[1]=true;
    }
    fn log_search_input(&mut self,key:crossterm::event::KeyEvent)->bool {
        let ctrl=key.modifiers==KeyModifiers::CONTROL;let shift=key.modifiers.contains(KeyModifiers::SHIFT);
        if key.code==KeyCode::F(10)&&!shift{self.palette_request=true;return true;}
        if key.code==KeyCode::F(1){self.core.overlay=Some(HELP.into());return true;}
        if ctrl&&key.code==KeyCode::Char('c')&&!shift {
            let picker=&self.core.picker;let row=picker.pane_search.current().unwrap_or(picker.second.unwrap_or(picker.anchor)as usize);
            if let Some(index)=picker.log_row(row as u32){self.effects.push(RepositoryEffect::Copy(picker.log.id(index)));}
            else {self.core.notice=Some("Uncommitted changes has no commit hash".into());}
            return true;
        }
        if ctrl&&key.code==KeyCode::Char('a'){self.begin_log_search();self.core.picker.pane_search.set_query(String::new());return true;}
        if ctrl&&key.code==KeyCode::Char('f')&&!shift{self.begin_log_search();return true;}
        if key.modifiers.difference(KeyModifiers::SHIFT).is_empty()&&let KeyCode::Char(ch)=key.code {self.begin_log_search();self.core.picker.pane_search.append(&ch.to_string());self.core.picker.poll_pane_search();self.reveal_log_search();return true;}
        if !self.core.picker.pane_search.active(){return false;}
        match key.code {
            KeyCode::Up|KeyCode::Down|KeyCode::F(3)=>{
                self.core.picker.poll_pane_search();self.core.picker.pane_search.step(key.code==KeyCode::Down||key.code==KeyCode::F(3)&&!shift);self.reveal_log_search();
            }
            KeyCode::Backspace=>{self.core.picker.pane_search.backspace();self.core.picker.poll_pane_search();self.reveal_log_search();}
            KeyCode::Enter=>{if let Some(row)=self.core.picker.pane_search.confirm(){self.log_search_origin=Some(self.core.picker.scroll);self.execute(Command::SelectRow{row:row as u32,extend:false,two:false});}}
            KeyCode::Esc=>{self.core.picker.pane_search.cancel();if let Some(scroll)=self.log_search_origin.take(){self.core.picker.scroll=scroll;}self.manual_scroll[1]=true;}
            KeyCode::Home|KeyCode::End|KeyCode::PageUp|KeyCode::PageDown=>{
                let height=self.area(self.bounds).y.saturating_sub(self.bounds.y+2).max(1)as usize;let picker=&mut self.core.picker;let max=(picker.rows()as usize).saturating_sub(height);let first=picker.scroll as usize;
                picker.scroll=match key.code{KeyCode::Home=>0,KeyCode::End=>max,KeyCode::PageUp=>first.saturating_sub(height),_=>first.saturating_add(height).min(max)}as f32;self.manual_scroll[1]=true;
            },
            _=>return false,
        }
        true
    }
    pub fn handle(&mut self, event: Event) -> bool {
        if self.resize_input(&event,false){return true;}
        if self.core.pr.as_ref().is_some_and(|p|p.details().is_some()){
            if event==Event::FocusGained {self.execute(Command::CheckPrHead);return true;}
            // standalone (the app routes these first): the tab over the diff
            let area=self.area(self.bounds);let area=Rect::new(area.x,area.y.saturating_add(2),area.width,area.height.saturating_sub(4));
            let Some(pr)=self.core.pr.as_mut() else {return false};
            let (taken,outcome)=self.pr_dialog.handle(&event,pr,area);
            match outcome {
                Some(crate::pr_details::Outcome::Pr(command))=>pr.execute(command),
                Some(crate::pr_details::Outcome::Commit(sha))=>self.execute(Command::PrCommit(sha)),
                Some(crate::pr_details::Outcome::Url(_)|crate::pr_details::Outcome::Line{..}|crate::pr_details::Outcome::Copy(_))|None=>{}
            }
            if taken{return true;}
        }
        match event {
            Event::Mouse(mouse) => {
                if self.core.overlay.is_some() {
                    match mouse.kind {
                        MouseEventKind::ScrollUp => {
                            self.overlay_scroll = self.overlay_scroll.saturating_sub(3)
                        }
                        MouseEventKind::ScrollDown => {
                            self.overlay_scroll = self.overlay_scroll.saturating_add(3)
                        }
                        _ => {}
                    }
                    return true;
                }
                if self.prompt.is_some() {
                    // a click outside the prompt closes it
                    if matches!(mouse.kind,MouseEventKind::Down(MouseButton::Left))&&!self.prompt_area.contains((mouse.column,mouse.row).into()){self.prompt=None;}
                    return true;
                }
                let area = self.bounds;
                let diff = self.area(area);
                let picker = &self.core.picker;
                if !picker.shown || !area.contains((mouse.column, mouse.row).into()) {
                    return false;
                }
                let pane = if picker.sidebar && mouse.column < diff.x {
                    Pane::Sidebar
                } else if picker.log_shown && mouse.row < diff.y {
                    Pane::Log
                } else {
                    return false;
                };
                let (top, height, scroll) = match pane {
                    Pane::Sidebar => (area.y, area.height, picker.sources.scroll),
                    Pane::Log => (area.y, diff.y - area.y, picker.scroll),
                    Pane::Diff => unreachable!(),
                };
                let slot = usize::from(pane == Pane::Log);
                match mouse.kind {
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                        self.manual_scroll[slot] = true;
                        self.execute(Command::Scroll {
                            pane,
                            rows: if mouse.kind == MouseEventKind::ScrollUp {
                                -3.0
                            } else {
                                3.0
                            },
                            visible: height.saturating_sub(2) as f32,
                        });
                    }
                    MouseEventKind::Down(MouseButton::Left) => {
                        self.manual_scroll[slot] = false;
                        self.execute(Command::Focus(pane));
                        if pane == Pane::Log
                            && mouse.row == top
                            && self.core.picker.merge_parents() > 1
                        {
                            self.run_command("parent");
                        } else if mouse.row > top && mouse.row < top + height.saturating_sub(1) {
                            let row = u32::from(mouse.row - top - 1) + scroll as u32;
                            match pane {
                                Pane::Sidebar => self.execute(Command::SourceSelect(row as usize)),
                                Pane::Log if row < self.core.picker.rows() => {
                                    self.execute(Command::SelectRow {
                                        row,
                                        extend: mouse.modifiers.contains(KeyModifiers::SHIFT),
                                        two: mouse.modifiers.contains(KeyModifiers::CONTROL),
                                    })
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
                true
            }
            Event::Resize(width, height) => {
                self.size = (width, height);
                false
            }
            Event::FocusGained => {
                if self.core.pr.is_none() {
                    return false;
                }
                self.execute(Command::CheckPrHead);
                true
            }
            Event::Paste(text) if self.prompt.is_some() => {
                self.input.push_str(&text.replace(['\n', '\r'], " "));
                if self.prompt == Some(Prompt::Filter) {
                    self.execute(Command::Filter(self.input.clone()));
                }
                true
            }
            Event::Paste(text) if !self.modal()&&self.core.picker.shown&&self.core.picker.focus==Pane::Log=>{self.begin_log_search();self.core.picker.pane_search.append(&text.replace(['\n','\r']," "));self.core.picker.poll_pane_search();self.reveal_log_search();true}
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                self.manual_scroll = [false; 2];
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                let shift = key.modifiers.contains(KeyModifiers::SHIFT);
                if ctrl && key.code == KeyCode::Char('c') && !(self.core.picker.shown&&self.core.picker.focus==Pane::Log&&!self.modal()) {
                    self.quit = true;
                    return true;
                }
                if ctrl && self.modal() {if self.prompt.is_some()&&key.code==KeyCode::Char('a'){self.input.clear();self.command_selected=0;}return true;}
                if ctrl {
                    match key.code {
                        KeyCode::Char('r') => {
                            self.execute(Command::TogglePicker);
                            return true;
                        }
                        KeyCode::Char('s') if self.core.picker.focus==Pane::Sidebar => {
                            self.execute(Command::ToggleSidebar);
                            return true;
                        }
                        KeyCode::Char('l') => {
                            self.execute(Command::ToggleLog);
                            return true;
                        }
                        _ => {}
                    }
                }
                if !self.modal()&&self.core.picker.shown&&self.core.picker.focus==Pane::Log&&ctrl&&matches!(key.code,KeyCode::Char('c')|KeyCode::Char('f')|KeyCode::Char('a'))&&!shift {
                    return self.log_search_input(key);
                }
                if ctrl
                    && !matches!(
                        key.code,
                        KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown
                        | KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End
                    )
                {
                    return false;
                }
                if key.code == KeyCode::F(6) {
                    if !self.core.picker.shown {
                        self.execute(Command::TogglePicker);
                    }
                    let next = match self.core.picker.focus {
                        Pane::Sidebar => Pane::Log,
                        Pane::Log => Pane::Diff,
                        Pane::Diff => Pane::Sidebar,
                    };
                    self.execute(Command::Focus(next));
                    return true;
                }
                if self.core.overlay.is_some() {
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('q') => {
                            self.execute(Command::CloseOverlay);
                            self.overlay_scroll = 0;
                        }
                        KeyCode::Home=>self.overlay_scroll=0,KeyCode::End=>self.overlay_scroll=u16::MAX,
                        KeyCode::Up => self.overlay_scroll = self.overlay_scroll.saturating_sub(1),
                        KeyCode::Down => {
                            self.overlay_scroll = self.overlay_scroll.saturating_add(1)
                        }
                        KeyCode::PageUp => {
                            self.overlay_scroll =
                                self.overlay_scroll.saturating_sub(self.size.1 / 2)
                        }
                        KeyCode::PageDown => {
                            self.overlay_scroll =
                                self.overlay_scroll.saturating_add(self.size.1 / 2)
                        }
                        _ => {}
                    }
                    return true;
                }
                if let Some(prompt) = self.prompt {
                    match key.code {
                        KeyCode::Esc => {
                            self.prompt = None;
                            self.input.clear();
                        }
                        KeyCode::Backspace => {
                            self.input.pop();
                            self.command_selected = 0;
                        }
                        KeyCode::Enter => {
                            self.prompt = None;
                            let input = std::mem::take(&mut self.input);
                            match prompt {
                                Prompt::Filter => {
                                    self.execute(Command::Filter(input));
                                    self.execute(Command::ApplyFilter);
                                }
                                Prompt::Sources => self.execute(Command::SourceFilter(input)),
                                Prompt::Pr => self.execute(Command::OpenPr(
                                    (!input.trim().is_empty()).then(|| input.trim().into()),
                                )),
                            }
                        }
                        KeyCode::Char(character) if !ctrl => {
                            self.input.push(character);
                            self.command_selected = 0;
                        }
                        _ => {}
                    }
                    if self.prompt == Some(Prompt::Filter) {
                        self.execute(Command::Filter(self.input.clone()));
                    }
                    return true;
                }
                if self.core.picker.shown&&self.core.picker.focus==Pane::Log&&self.log_search_input(key){return true;}
                if self.core.picker.shown
                    && self.core.picker.focus == Pane::Log
                    && key.code == KeyCode::Char('t')
                    && let Some((right, merge_base)) = self.core.picker.compare.clone() {
                    self.execute(Command::Compare {
                        left: "HEAD".into(),
                        right,
                        merge_base: !merge_base,
                    });
                    return true;
                }
                // PR controls remain reachable from the comparison pane; all other keys there go to its adapter.
                if self.core.pr.is_some() {
                    match key.code {
                        KeyCode::Char('v') => {
                            self.run_command("viewed");
                            return true;
                        }
                        KeyCode::Char('r') => {
                            self.run_command("refresh");
                            return true;
                        }
                        KeyCode::Char('d') => {
                            self.run_command("body");
                            return true;
                        }
                        KeyCode::Char('t') => {
                            self.run_command("threads");
                            return true;
                        }
                        KeyCode::Char('T') => {
                            self.run_command("outdated");
                            return true;
                        }
                        _ => {}
                    }
                }
                if !self.core.picker.shown || self.core.picker.focus == Pane::Diff {
                    return false;
                }
                if shift && key.modifiers.contains(KeyModifiers::ALT) && matches!(key.code, KeyCode::Left | KeyCode::Right) { return false; }
                if key.code == KeyCode::Char('q') {
                    self.quit = true;
                    return true;
                }
                if key.code == KeyCode::Tab || key.code == KeyCode::BackTab {
                    let backwards = shift || key.code == KeyCode::BackTab;
                    let pane = match (self.core.picker.focus, backwards) {
                        (Pane::Sidebar, false) | (Pane::Diff, true) => Pane::Log,
                        (Pane::Log, false) | (Pane::Sidebar, true) => Pane::Diff,
                        (Pane::Diff, false) | (Pane::Log, true) => Pane::Sidebar,
                    };
                    self.execute(Command::Focus(pane));
                    return true;
                }
                match key.code {
                    KeyCode::F(1) | KeyCode::Char('?') => {
                        self.core.overlay = Some(HELP.into());
                        return true;
                    }
                    // the shared command palette, as everywhere
                    KeyCode::F(10) | KeyCode::Char(':') => {
                        self.palette_request = true;
                        return true;
                    }
                    KeyCode::Char('P') => {
                        self.begin(Prompt::Pr);
                        return true;
                    }
                    KeyCode::Char('a') => {
                        self.run_command("all");
                        return true;
                    }
                    KeyCode::Char('b') => {
                        self.run_command("current");
                        return true;
                    }
                    KeyCode::Char('s') => {
                        self.run_command("staged");
                        return true;
                    }
                    KeyCode::Char('g') => {
                        self.run_command("graph");
                        return true;
                    }
                    KeyCode::Char('y') => {
                        self.run_command("hash");
                        return true;
                    }
                    KeyCode::Char('Y') => {
                        self.run_command("details");
                        return true;
                    }
                    KeyCode::Char('u') => {
                        self.run_command("url");
                        return true;
                    }
                    KeyCode::Char('U') => {
                        self.run_command("open-url");
                        return true;
                    }
                    KeyCode::Char('h') => {
                        self.run_command("check-head");
                        return true;
                    }
                    _ => {}
                }
                if self.core.picker.focus == Pane::Sidebar {
                    match key.code {
                        KeyCode::Char('/') => self.begin(Prompt::Sources),
                        KeyCode::Up => self.execute(Command::SourceMove(-1)),
                        KeyCode::Down => self.execute(Command::SourceMove(1)),
                        KeyCode::Home => self.execute(Command::SourceMove(-i32::MAX)),
                        KeyCode::End => self.execute(Command::SourceMove(i32::MAX)),
                        KeyCode::Left | KeyCode::Right => return true,
                        KeyCode::PageUp => {
                            self.execute(Command::SourceMove(-(self.size.1 as i32 / 2)))
                        }
                        KeyCode::PageDown => {
                            self.execute(Command::SourceMove(self.size.1 as i32 / 2))
                        }
                        KeyCode::Enter | KeyCode::Char(' ') => {
                            if let Some(source) = self.source() {
                                self.execute(Command::Source(source));
                            }
                        }
                        KeyCode::Char('c') => self.compare(true, false),
                        KeyCode::Char('C') => self.compare(false, false),
                        KeyCode::Char('o') => self.worktree(),
                        KeyCode::Esc => self.execute(Command::SourceFilter(String::new())),
                        KeyCode::Backspace => {
                            let mut text = self.core.picker.sources.filter.clone();
                            text.pop();
                            self.execute(Command::SourceFilter(text));
                        }
                        KeyCode::Char(character) if !ctrl => {
                            let text = format!("{}{character}", self.core.picker.sources.filter);
                            self.execute(Command::SourceFilter(text));
                        }
                        _ => return false,
                    }
                } else {
                    let delta = match key.code {
                        KeyCode::Up => Some(-1),
                        KeyCode::Down => Some(1),
                        KeyCode::PageUp => Some(-10),
                        KeyCode::PageDown => Some(10),
                        _ => None,
                    };
                    if let Some(delta) = delta {
                        self.execute(Command::Move {
                            delta,
                            extend: shift,
                            two: ctrl,
                        });
                    } else {
                        match key.code {
                            KeyCode::Left | KeyCode::Right => return true,
                            KeyCode::Home|KeyCode::End=>self.execute(Command::SelectRow{row:if key.code==KeyCode::Home{0}else{self.core.picker.rows().saturating_sub(1)},extend:shift,two:ctrl}),
                            KeyCode::Enter => self.execute(Command::Select),
                            KeyCode::Char('m') => self.run_command("parent"),
                            KeyCode::Char('/') => self.begin(Prompt::Filter),
                            KeyCode::Char('f') => self.run_command("follow"),
                            KeyCode::Esc => self.execute(Command::ClearFilter),
                            _ => return false,
                        }
                    }
                }
                true
            }
            _ => false,
        }
    }
    #[cfg(test)]
    pub fn render(&mut self, frame: &mut Frame) {
        let area=frame.area();self.render_in(frame,area,crate::chrome::initial_palette());
        // standalone: the details tab over the diff (the app draws it in its own diff body)
        let diff=self.area(area);
        if let Some(pr)=self.core.pr.as_mut().filter(|p|p.details().is_some()){self.pr_dialog.draw(frame,pr,Rect::new(diff.x,diff.y.saturating_add(2),diff.width,diff.height.saturating_sub(4)),crate::chrome::initial_palette());}
        self.draw_popup(frame,area,crate::chrome::initial_palette());
    }
    pub fn modal(&self)->bool {self.prompt.is_some()||self.core.overlay.is_some()}
    pub fn render_in(&mut self, frame: &mut Frame, area:Rect,palette:&lenz::theme::Palette) {
        self.resizer.grips.clear();
        let diff=self.area(area);
        if self.core.picker.shown {
            if self.core.picker.sidebar&&diff.x>area.x {self.resizer.grips.push(crate::panes::Grip::vertical(Rect::new(diff.x-1,area.y,1,area.height),0,area.x));}
            if self.core.picker.log_shown&&diff.y>area.y {self.resizer.grips.push(crate::panes::Grip::horizontal(Rect::new(diff.x,diff.y-1,diff.width,1),1,area.y));}
        }
        self.bounds=area;
        self.size = (area.width, area.height);
        let diff = self.area(area);
        let selection = self.core.selection_title().map_or(String::new(), |t| format!(" · {}", safe_label(&t.text)));
        let pr_title = self.core.pr.as_ref().map(|pr| {
            let count = pr.viewed_count(self.core.comparisons());
            format!(
                "PR #{} · {}/{} viewed · {} · {}",
                pr.pr.number,
                count.0,
                count.1,
                self.core
                    .comparisons()
                    .get(self.current)
                    .and_then(|comparison| pr.view_status(comparison))
                    .map_or("unmarked", |status| match status {
                        lenz::pr::ViewStatus::Viewed => "viewed",
                        lenz::pr::ViewStatus::ChangedSinceViewed => "changed since viewed",
                        lenz::pr::ViewStatus::Unviewed => "unviewed",
                    }),
                safe_label(&pr.pr.title)
            )
        });
        let holding = self.core.holding_log();
        let picker = &mut self.core.picker;
        if let Some(title) = &pr_title
            && (!picker.shown || !picker.log_shown)
            && diff.y > area.y
        {
            frame.render_widget(
                Paragraph::new(title.clone()).style(crate::chrome::style_with(palette).fg(crate::chrome::color(palette.accent))),
                Rect::new(diff.x, area.y, diff.width, 1.min(area.height)),
            );
        }
        if picker.shown {
            if picker.sidebar {
                let sidebar = Rect::new(area.x, area.y, diff.x - area.x, area.height);
                let rows = picker
                    .sources
                    .rows_with_palette(&picker.branches, source_palette(palette));
                let height = sidebar.height.saturating_sub(2).max(1) as usize;
                let selected = picker.sources.selected.min(rows.len().saturating_sub(1));
                let first = if self.manual_scroll[0] {
                    picker.sources.scroll as usize
                } else if selected < picker.sources.scroll as usize {
                    selected
                } else if selected >= picker.sources.scroll as usize + height {
                    selected + 1 - height
                } else {
                    picker.sources.scroll as usize
                };
                picker.sources.scroll = first as f32;
                let lines: Vec<_> = rows
                    .iter()
                    .enumerate()
                    .skip(first)
                    .take(height)
                    .map(|(index, row)| {
                        let color = Color::Rgb(
                            (row.color >> 16) as u8,
                            (row.color >> 8) as u8,
                            row.color as u8,
                        );
                        let mut style = crate::chrome::style_with(palette).fg(color);
                        if row.current || picker.focus == Pane::Sidebar && selected == index {
                            style = style.bg(crate::chrome::color(palette.selected_bg)).add_modifier(Modifier::BOLD);
                        }
                        Line::styled(safe_label(&row.text), style)
                    })
                    .collect();
                let title = if picker.sources.filter.is_empty() {
                    "Repository".into()
                } else {
                    format!("Sources: {}", safe_label(&picker.sources.filter))
                };
                frame.render_widget(Clear, sidebar);
                frame.render_widget(
                    Paragraph::new(lines).style(crate::chrome::style_with(palette).bg(crate::chrome::color(palette.sidebar_bg)))
                        .block(Block::default().borders(Borders::ALL).title(title)),
                    sidebar,
                );
            }
            if picker.log_shown {
                let log = Rect::new(diff.x, area.y, diff.width, diff.y - area.y);
                let height = log.height.saturating_sub(2).max(1) as u32;
                let selected = picker.pane_search.current().map(|i|i as u32).unwrap_or(picker.second.unwrap_or(picker.anchor));
                let first = if self.manual_scroll[1] || picker.pane_search.active() {
                    picker.scroll as u32
                } else if selected < picker.scroll as u32 {
                    selected
                } else if selected >= picker.scroll as u32 + height {
                    selected + 1 - height
                } else {
                    picker.scroll as u32
                };
                picker.scroll = first as f32;
                let mut lines = Vec::new();
                for row in first..(first + height).min(picker.rows()) {
                    let text = if let Some(index) = picker.log_row(row) {
                        let id = picker.log.id(index);
                        let refs = picker
                            .refs
                            .as_ref()
                            .map(|refs| {
                                refs.at(&id)
                                    .iter()
                                    .map(|reference| reference.name.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .unwrap_or_default();
                        let graph = if picker.matches.is_none() {
                            graph_row(
                                &picker.log,
                                index,
                                log.width.saturating_sub(15) as usize / 3,
                            )
                        } else {
                            String::new()
                        };
                        let query=picker.pane_search.query();let email=picker.log.author(index).1;
                        let matched=if picker.pane_search.active(){format!("{}{}",if lenz::pane_search::contains(&id,query){format!("{id} · ")}else{String::new()},if lenz::pane_search::contains(email,query){format!("{email} · ")}else{String::new()})}else{String::new()};
                        format!(
                            "{} {}{} {}{}  {}",
                            graph,
                            matched,
                            &id[..id.len().min(8)],
                            if refs.is_empty() {
                                String::new()
                            } else {
                                format!("[{refs}] ")
                            },
                            picker.log.subject(index),
                            picker.log.author(index).0
                        )
                    } else {
                        "± Uncommitted changes".into()
                    };
                    let candidate=picker.pane_search.current()==Some(row as usize);
                    let style = if if picker.pane_search.active(){candidate}else{picker.selected(row)} {
                        crate::chrome::style_with(palette)
                            .bg(crate::chrome::color(palette.selected_bg))
                            .add_modifier(Modifier::BOLD)
                    } else {
                        crate::chrome::style_with(palette)
                    };
                    lines.push(crate::pane_search::line(safe_label(&text),&picker.pane_search,style,palette));
                }
                let title = if picker.pane_search.active(){crate::pane_search::status(&picker.pane_search,picker.rows()as usize)}else{pr_title.clone().unwrap_or_else(|| {
                    format!(
                        "Log {}{}",
                        safe_label(&picker.title_branch),
                        match picker.more_label() {
                            Some(more) => format!(" · {more}"),
                            None if picker.loading => " · loading".into(),
                            None => String::new(),
                        }
                    ) + &selection
                })};
                // a restarted log with no rows yet keeps its last rows up (shared LOG_HOLD); clicks reach nothing there
                let (lines,title)=match &self.held_log {
                    Some(held) if holding=>held.clone(),
                    _=>{self.held_log=Some((lines.clone(),title.clone()));(lines,title)}
                };
                frame.render_widget(Clear, log);
                frame.render_widget(
                    Paragraph::new(lines).style(crate::chrome::style_with(palette).bg(crate::chrome::color(palette.sidebar_bg)))
                        .block(Block::default().borders(Borders::ALL).title(title)),
                    log,
                );
            }
        }
        if let Some(notice) = self
            .core
            .notice
            .as_ref()
            .or(self.core.picker.error.as_ref())
        {
            frame.render_widget(
                Paragraph::new(format!("F2 details · {}",safe_label(notice)))
                    .style(crate::chrome::style_with(palette).fg(crate::chrome::color(palette.accent)).bg(crate::chrome::color(palette.status_bg))),
                Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
            );
        }
        self.resizer.draw(frame.buffer_mut(),palette);
    }
    pub fn draw_popup(&mut self,frame:&mut Frame,area:Rect,palette:&lenz::theme::Palette){
        // kept while the PR is open: each tab's scroll survives Files changed
        if self.core.pr.is_none(){self.pr_dialog=Default::default();}
        self.command_hits.clear();self.prompt_area=Rect::default();
        if let Some(prompt) = self.prompt {
            let label = match prompt {
                Prompt::Filter => "Log filter",
                Prompt::Sources => "Repository source filter",
                Prompt::Pr => "Pull request number/URL (empty: current)",
            };
            let lines = vec![Line::from(format!("{label}: {}▏", safe_label(&self.input)))];
            let popup = Rect::new(
                area.x + 1,
                area.y + 1,
                area.width.saturating_sub(2),
                (lines.len() as u16 + 2).min(area.height.saturating_sub(2)),
            );
            self.prompt_area=popup;
            frame.render_widget(Clear, popup);
            frame.render_widget(
                Paragraph::new(lines).style(crate::chrome::style_with(palette).bg(crate::chrome::color(palette.status_bg))).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("F2 details · Enter executes · Esc closes"),
                ),
                popup,
            );
        }
        if let Some(text) = &self.core.overlay {
            if text==HELP{let mut scroll=self.overlay_scroll as usize;crate::help::draw(frame,area,crate::help::REPOSITORY,&mut scroll,palette);self.overlay_scroll=scroll as u16;return;}
            let popup = Rect::new(
                area.x + 1,
                area.y + 1,
                area.width.saturating_sub(2),
                area.height.saturating_sub(2),
            );
            frame.render_widget(Clear, popup);
            frame.render_widget(
                Paragraph::new(safe(text)).style(crate::chrome::style_with(palette).bg(crate::chrome::color(palette.status_bg)))
                    .wrap(Wrap { trim: false })
                    .scroll((self.overlay_scroll, 0))
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title("Esc closes · arrows scroll"),
                    ),
                popup,
            );
        }
    }
}
/// Cell topology from the shared history's edges; horizontal joins keep merge parents distinguishable.
fn graph_row(log: &lenz::history::Log, row: u32, lanes: usize) -> String {
    let lanes = lanes.min(log.width() as usize);
    if lanes == 0 {
        return String::new();
    }
    let mut cells = vec![0u8; lanes * 2 - 1];
    let mut arrows = Vec::new();
    for edge in log.edges(row..row + 1) {
        let lane = edge.lane as usize * 2;
        let endpoint = if edge.child == row {
            Some((edge.child_col, 2u8))
        } else if edge.parent == Some(row) {
            Some((edge.parent_col, 1u8))
        } else {
            None
        };
        if let Some((column, vertical)) = endpoint {
            if let Some(cell) = cells.get_mut(lane) {
                *cell |= vertical;
            }
            let column = column as usize * 2;
            let (from, to) = (column.min(lane), column.max(lane));
            for position in from..=to.min(cells.len() - 1) {
                if position > from {
                    cells[position] |= 4;
                }
                if position < to {
                    cells[position] |= 8;
                }
            }
            if edge.stub {
                arrows.push((lane, if vertical == 2 { '↧' } else { '↥' }));
            }
        } else if !edge.stub
            && let Some(cell) = cells.get_mut(lane)
        {
            *cell |= 3;
        }
    }
    let mut cells: Vec<char> = cells
        .into_iter()
        .map(|bits| match bits {
            0 => ' ',
            1..=3 => '│',
            4 | 8 | 12 => '─',
            5 => '┘',
            9 => '└',
            6 => '┐',
            10 => '┌',
            7 => '┤',
            11 => '├',
            13 => '┴',
            14 => '┬',
            _ => '┼',
        })
        .collect();
    for (lane, arrow) in arrows {
        if let Some(cell) = cells.get_mut(lane) {
            *cell = arrow;
        }
    }
    if let Some(cell) = cells.get_mut(log.column(row) as usize * 2) {
        *cell = '●';
    }
    cells.into_iter().collect()
}

pub(crate) fn safe_label(text: &str) -> String {
    text.chars()
        .map(|character| {
            if lenz::document::hides_text(character) {
                '�'
            } else {
                character
            }
        })
        .collect()
}
fn safe(text: &str) -> String {
    text.chars()
        .map(|character| {
            if lenz::document::hides_text(character) && character != '\n' {
                '�'
            } else {
                character
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;
    use ratatui::{Terminal, backend::TestBackend};
    fn git(root: &std::path::Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fn ready(repository: &mut Repository) {
        let started = std::time::Instant::now();
        while repository.busy() {
            repository.poll();
            assert!(started.elapsed().as_secs() < 10);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    pub(super) fn fixture() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        std::fs::write(dir.path().join("file"), "first\n").unwrap();
        git(dir.path(), &["add", "file"]);
        git(dir.path(), &["commit", "-qm", "First commit"]);
        std::fs::write(dir.path().join("file"), "second\n").unwrap();
        git(dir.path(), &["commit", "-qam", "Second commit"]);
        git(dir.path(), &["branch", "topic"]);
        let request = lenz::cli::parse(&["log".into()], dir.path()).unwrap();
        let mut repository = Repository::open(request, dir.path().into());
        ready(&mut repository);
        (dir, repository)
    }
    fn key(repository: &mut Repository, key: KeyCode, modifiers: KeyModifiers) {
        assert!(repository.handle(Event::Key(KeyEvent::new(key, modifiers))));
    }
    fn screen(repository: &mut Repository) -> String {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| repository.render(frame)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<Vec<_>>()
            .join("")
    }
    #[test]
    fn long_log_pauses_with_a_label_and_end_resumes_reading() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        for i in 0..40 {
            std::fs::write(dir.path().join("file"), format!("{i}\n")).unwrap();
            git(dir.path(), &["add", "file"]);
            git(dir.path(), &["commit", "-qm", &format!("c{i}")]);
        }
        let request = lenz::cli::parse(&["log".into()], dir.path()).unwrap();
        let mut repo = Repository::open(request, dir.path().into());
        repo.core.picker.window = 5;
        repo.core.picker.restart(lenz::history::Branches::All, None);
        repo.core.picker.start_stream();
        ready(&mut repo);
        let paused = repo.core.picker.log.len();
        assert!(repo.core.picker.paused && paused < 40, "a long history pauses ({paused} rows)");
        assert!(screen(&mut repo).contains("more as you scroll"), "the log title says more is coming");
        repo.core.picker.focus = lenz::repository_session::Pane::Log;
        key(&mut repo, KeyCode::End, KeyModifiers::NONE);
        repo.poll();
        assert!(repo.busy(), "End in the log resumes reading");
        ready(&mut repo);
        assert!(repo.core.picker.log.len() > paused);
    }
    #[test]
    fn pane_repository_source_and_log_drag_preserve_selected_commit_and_reset() {
        let (_dir,mut repo)=fixture();screen(&mut repo);repo.take_change_set();let anchor=repo.core.picker.anchor;let before=repo.area(repo.bounds);
        let mouse=|kind,x,y|Event::Mouse(crossterm::event::MouseEvent{kind,column:x,row:y,modifiers:KeyModifiers::NONE});
        repo.handle(mouse(MouseEventKind::Down(MouseButton::Left),before.x-1,12));repo.handle(mouse(MouseEventKind::Drag(MouseButton::Left),40,20));repo.handle(mouse(MouseEventKind::Up(MouseButton::Left),40,20));screen(&mut repo);
        assert_eq!(repo.area(repo.bounds).x,41);assert_eq!(repo.core.picker.anchor,anchor);assert!(repo.take_change_set().is_none());
        let grip=repo.resizer.grips.iter().find(|g|g.id==1).unwrap().area;repo.handle(mouse(MouseEventKind::Down(MouseButton::Left),grip.x+2,grip.y));repo.handle(mouse(MouseEventKind::Drag(MouseButton::Left),grip.x+2,14));repo.handle(mouse(MouseEventKind::Up(MouseButton::Left),grip.x+2,14));screen(&mut repo);assert_eq!(repo.area(repo.bounds).y,15);assert_eq!(repo.core.picker.anchor,anchor);
        repo.begin(Prompt::Filter);repo.handle(mouse(MouseEventKind::Down(MouseButton::Left),40,15));repo.handle(mouse(MouseEventKind::Drag(MouseButton::Left),20,15));assert_eq!(repo.source_width,Some(41.0));repo.prompt=None;
        repo.run_command(lenz::commands::Interaction::ResetPaneSizes.label());screen(&mut repo);assert_eq!(repo.area(repo.bounds),before);assert_eq!(repo.interaction,Some(lenz::commands::Interaction::ResetPaneSizes));
        for (w,h) in [(1,1),(20,4),(100,30)] {let mut terminal=Terminal::new(TestBackend::new(w,h)).unwrap();repo.handle(Event::Resize(w,h));terminal.draw(|f|repo.render(f)).unwrap();}
    }
    #[test]
    fn commit_typing_searches_without_loading_and_context_copy_targets_candidate() {
        let (_dir,mut repo)=fixture();screen(&mut repo);repo.take_change_set();
        let selected=(repo.core.picker.anchor,repo.core.picker.second,repo.core.picker.range);let rows=repo.core.picker.rows();let scroll=repo.core.picker.scroll;
        for ch in "qbgynjk:/?".chars(){key(&mut repo,KeyCode::Char(ch),KeyModifiers::NONE);}
        assert_eq!(repo.core.picker.pane_search.query(),"qbgynjk:/?");assert_eq!((repo.core.picker.anchor,repo.core.picker.second,repo.core.picker.range),selected);assert!(repo.take_change_set().is_none());assert!(repo.take_effects().is_empty());assert!(repo.prompt.is_none()&&repo.core.overlay.is_none()&&!repo.quit);
        key(&mut repo,KeyCode::Esc,KeyModifiers::NONE);assert_eq!(repo.core.picker.scroll,scroll);
        key(&mut repo,KeyCode::Char('f'),KeyModifiers::CONTROL);assert!(repo.core.picker.pane_search.active());
        assert!(repo.handle(Event::Paste("FIRST".into())));ready(&mut repo);
        let row=repo.core.picker.pane_search.current().unwrap()as u32;let hash=repo.core.picker.log.id(repo.core.picker.log_row(row).unwrap());
        let painted=screen(&mut repo);assert!(painted.contains("First commit")&&painted.contains("Second commit")&&painted.contains("Find: FIRST"));assert_eq!(repo.core.picker.rows(),rows);
        repo.log_height=Some(3.0);let candidate=repo.core.picker.pane_search.current();
        key(&mut repo,KeyCode::End,KeyModifiers::NONE);assert_eq!(repo.core.picker.scroll,2.0);
        key(&mut repo,KeyCode::Home,KeyModifiers::NONE);assert_eq!(repo.core.picker.scroll,0.0);
        key(&mut repo,KeyCode::PageDown,KeyModifiers::NONE);assert_eq!(repo.core.picker.scroll,1.0);key(&mut repo,KeyCode::PageUp,KeyModifiers::NONE);assert_eq!(repo.core.picker.scroll,0.0);
        assert_eq!(repo.core.picker.pane_search.current(),candidate);assert_eq!(repo.core.picker.anchor,selected.0);assert!(repo.take_change_set().is_none());repo.log_height=None;
        key(&mut repo,KeyCode::F(3),KeyModifiers::NONE);key(&mut repo,KeyCode::F(3),KeyModifiers::SHIFT);assert!(repo.take_change_set().is_none());assert_eq!(repo.core.picker.anchor,selected.0);
        key(&mut repo,KeyCode::Char('c'),KeyModifiers::CONTROL);assert!(matches!(repo.take_effects().as_slice(),[RepositoryEffect::Copy(c)]if c==&hash));
        assert!(repo.context_commands(None).is_some());assert_eq!(repo.context_commit(),Some(hash.clone()));assert_eq!(repo.core.picker.anchor,selected.0);
        repo.execute_context(Command::CopyHash);assert!(matches!(repo.take_effects().as_slice(),[RepositoryEffect::Copy(c)]if c==&hash));assert_eq!(repo.core.picker.anchor,selected.0);
        key(&mut repo,KeyCode::Enter,KeyModifiers::NONE);ready(&mut repo);assert_eq!(repo.core.picker.anchor,row);assert!(repo.take_change_set().is_some());assert_eq!(repo.core.picker.pane_search.query(),"FIRST");
        let accepted_scroll=repo.core.picker.scroll;key(&mut repo,KeyCode::Char('a'),KeyModifiers::CONTROL);assert_eq!(repo.core.picker.pane_search.query(),"");
        repo.handle(Event::Key(KeyEvent::new(KeyCode::Char('c'),KeyModifiers::CONTROL|KeyModifiers::SHIFT)));assert!(repo.take_effects().is_empty());assert_eq!(repo.core.picker.pane_search.query(),"");
        key(&mut repo,KeyCode::Esc,KeyModifiers::NONE);assert_eq!(repo.core.picker.anchor,row);assert_eq!(repo.core.picker.scroll,accepted_scroll);
        key(&mut repo,KeyCode::F(10),KeyModifiers::NONE);assert!(repo.palette_request);assert!(!repo.core.picker.pane_search.active());
    }
    #[test]
    fn commit_email_and_hash_suffix_matches_are_painted_and_highlighted() {
        let (_dir,mut repo)=fixture();screen(&mut repo);repo.take_change_set();
        let index=repo.core.picker.log_row(repo.core.picker.anchor).unwrap();let hash=repo.core.picker.log.id(index);let email=repo.core.picker.log.author(index).1.to_string();
        let queries=[email,hash[32..].to_string()];
        for query in queries {
            key(&mut repo,KeyCode::Char('a'),KeyModifiers::CONTROL);repo.handle(Event::Paste(query.clone()));ready(&mut repo);
            let mut terminal=Terminal::new(TestBackend::new(160,30)).unwrap();terminal.draw(|f|repo.render(f)).unwrap();let b=terminal.backend().buffer();let log=repo.area(b.area);
            let mut highlighted=String::new();for y in b.area.y+1..log.y.saturating_sub(1){for x in log.x+1..b.area.right().saturating_sub(1){let cell=&b[(x,y)];if cell.bg==crate::chrome::color(crate::chrome::palette().found_bg){highlighted.push_str(cell.symbol());}}}
            assert!(highlighted.contains(&query),"query {query} must be highlighted in the actual commit row: {highlighted}");assert!(repo.take_change_set().is_none());
        }
    }
    #[test]
    fn real_input_shows_streamed_graph_selects_range_and_copies_commit() {
        let (_dir, mut repository) = fixture();
        let text = screen(&mut repository);
        assert!(
            text.contains("Second commit") && text.contains("First commit") && text.contains("●")
        );
        key(&mut repository, KeyCode::Down, KeyModifiers::SHIFT);
        ready(&mut repository);
        assert!(repository.core.picker.range && repository.core.picker.second.is_some());
        let (_, spec) = repository.take_change_set().unwrap();
        let Spec::Git { revs, .. } = spec else {
            panic!()
        };
        assert_eq!(revs.len(), 2);
        key(&mut repository, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(
            matches!(repository.take_effects().as_slice(), [RepositoryEffect::Copy(hash)] if hash.len() == 40)
        );
        key(&mut repository, KeyCode::F(1), KeyModifiers::NONE);
        assert!(screen(&mut repository).to_lowercase().contains("range"));
    }
    #[test]
    fn merged_history_has_visible_parent_joins_and_parent_switch_changes_comparison() {
        let (dir, _) = fixture();
        git(dir.path(), &["checkout", "-qb", "side"]);
        std::fs::write(dir.path().join("side"), "side\n").unwrap();
        git(dir.path(), &["add", "side"]);
        git(dir.path(), &["commit", "-qm", "Side commit"]);
        git(dir.path(), &["checkout", "-q", "topic"]);
        std::fs::write(dir.path().join("main"), "main\n").unwrap();
        git(dir.path(), &["add", "main"]);
        git(dir.path(), &["commit", "-qm", "Main commit"]);
        git(
            dir.path(),
            &["merge", "--no-ff", "-qm", "Merged parents", "side"],
        );
        let request = lenz::cli::parse(&["log".into()], dir.path()).unwrap();
        let mut repository = Repository::open(request, dir.path().into());
        ready(&mut repository);
        let text = screen(&mut repository);
        assert!(
            text.contains("Merged parents") && text.contains("●─"),
            "{text}"
        );
        assert_eq!(repository.core.picker.merge_parents(), 2);
        let (_, first) = repository.take_change_set().unwrap();
        repository.run_command("parent");
        ready(&mut repository);
        let (_, second) = repository.take_change_set().unwrap();
        assert_ne!(first, second);
        repository.run_command("filter");
        for character in "side".chars() {
            key(
                &mut repository,
                KeyCode::Char(character),
                KeyModifiers::NONE,
            );
        }
        key(&mut repository, KeyCode::Enter, KeyModifiers::NONE);
        ready(&mut repository);
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| repository.render(frame)).unwrap();
        let diff = repository.area(Rect::new(0, 0, 100, 30));
        let mut log_text = String::new();
        for y in 0..diff.y {
            for x in diff.x..100 {
                log_text.push_str(terminal.backend().buffer()[(x, y)].symbol());
            }
        }
        assert!(
            log_text.contains("Side commit") && !log_text.contains("●"),
            "{log_text}"
        );
    }
    #[test]
    fn real_mouse_source_headings_commit_ranges_and_wheel_stay_in_their_panes() {
        let (dir, mut repository) = fixture();
        repository.take_change_set();
        screen(&mut repository);
        let mouse = |repository: &mut Repository, kind, x, y, modifiers| {
            assert!(
                repository.handle(Event::Mouse(crossterm::event::MouseEvent {
                    kind,
                    column: x,
                    row: y,
                    modifiers
                }))
            );
        };
        let header = repository
            .core
            .picker
            .sources
            .rows_with_palette(&repository.core.picker.branches, source_palette(crate::chrome::palette()))
            .iter()
            .position(|row| matches!(row.action, Some(SourceAction::Toggle("Branches"))))
            .unwrap();
        mouse(
            &mut repository,
            MouseEventKind::Down(MouseButton::Left),
            2,
            header as u16 + 1,
            KeyModifiers::NONE,
        );
        assert!(
            repository
                .core
                .picker
                .sources
                .collapsed
                .contains("Branches")
        );
        assert!(
            repository.take_change_set().is_none(),
            "source heading must only collapse its group"
        );
        mouse(
            &mut repository,
            MouseEventKind::Down(MouseButton::Left),
            35,
            2,
            KeyModifiers::NONE,
        );
        ready(&mut repository);
        assert_eq!(repository.core.picker.anchor, 1);
        mouse(
            &mut repository,
            MouseEventKind::Down(MouseButton::Left),
            35,
            3,
            KeyModifiers::SHIFT,
        );
        ready(&mut repository);
        assert_eq!(repository.core.picker.second, Some(2));
        assert!(repository.core.picker.range);
        mouse(
            &mut repository,
            MouseEventKind::Down(MouseButton::Left),
            35,
            2,
            KeyModifiers::CONTROL,
        );
        ready(&mut repository);
        assert!(
            repository.core.picker.second.is_none(),
            "ctrl-clicking anchor removes second commit"
        );
        assert!(!repository.core.picker.range);
        for i in 0..12 {
            std::fs::write(dir.path().join("file"), format!("commit {i}\n")).unwrap();
            git(dir.path(), &["commit", "-qam", &format!("Commit {i}")]);
        }
        repository.core.execute(Command::Walk(Branches::Current));
        ready(&mut repository);
        screen(&mut repository);
        let anchor = repository.core.picker.anchor;
        mouse(
            &mut repository,
            MouseEventKind::ScrollDown,
            35,
            4,
            KeyModifiers::NONE,
        );
        let scroll = repository.core.picker.scroll;
        assert!(scroll > 0.0);
        screen(&mut repository);
        assert_eq!(
            repository.core.picker.scroll, scroll,
            "render must preserve mouse scroll even when selection is outside viewport"
        );
        assert_eq!(
            repository.core.picker.anchor, anchor,
            "wheel must not select or load a commit"
        );
        mouse(
            &mut repository,
            MouseEventKind::ScrollUp,
            35,
            4,
            KeyModifiers::NONE,
        );
        assert_eq!(repository.core.picker.scroll, 0.0);
    }
    #[test]
    fn keyboard_filters_sources_and_log_then_toggles_picker_and_focus() {
        let (_dir, mut repository) = fixture();
        repository.run_command("filter");
        for character in "first".chars() {
            key(
                &mut repository,
                KeyCode::Char(character),
                KeyModifiers::NONE,
            );
        }
        key(&mut repository, KeyCode::Enter, KeyModifiers::NONE);
        ready(&mut repository);
        let text = screen(&mut repository);
        assert!(text.contains("First commit") && !text.contains("Second commit"));
        key(&mut repository, KeyCode::Esc, KeyModifiers::NONE);
        key(&mut repository, KeyCode::F(6), KeyModifiers::NONE); // log to diff
        key(&mut repository, KeyCode::F(6), KeyModifiers::NONE); // diff to sidebar
        assert_eq!(repository.core.picker.focus, Pane::Sidebar);
        key(&mut repository, KeyCode::Char('z'), KeyModifiers::NONE);
        assert_eq!(repository.core.picker.sources.filter, "z");
        key(&mut repository, KeyCode::Esc, KeyModifiers::NONE);
        assert!(repository.core.picker.sources.filter.is_empty());
        key(&mut repository, KeyCode::Char('r'), KeyModifiers::CONTROL);
        assert!(!repository.core.picker.shown);
        assert_eq!(
            repository.area(Rect::new(0, 0, 100, 30)),
            Rect::new(0, 0, 100, 30)
        );
    }
    #[test]
    fn palette_refresh_updates_sources_and_history_without_resetting_filters() {
        let (dir,mut repository)=fixture();
        repository.core.execute(Command::Source(SourceAction::Local));ready(&mut repository);
        repository.core.execute(Command::SourceFilter("topic".into()));
        repository.core.execute(Command::Filter("author:Test".into()));
        repository.core.execute(Command::ApplyFilter);ready(&mut repository);
        repository.core.execute(Command::Focus(Pane::Sidebar));
        git(dir.path(),&["tag","new-tag"]);
        std::fs::write(dir.path().join("new-file"),"new\n").unwrap();
        git(dir.path(),&["add","new-file"]);git(dir.path(),&["commit","-qm","New refresh commit"]);
        std::fs::write(dir.path().join("file"),"changed\n").unwrap();
        // the palette's Refresh repository (the catalog action's command)
        let command=repository.core.action_command(lenz::catalog::RepoAction::Refresh,"file").unwrap().unwrap();
        repository.core.execute(command);ready(&mut repository);
        assert_eq!(repository.core.picker.sources.local,1);
        assert_eq!(repository.core.picker.sources.filter,"topic");
        assert_eq!(repository.core.picker.filter,"author:Test");
        assert_eq!(repository.core.picker.focus,Pane::Sidebar);
        assert!(screen(&mut repository).contains("New refresh commit"));
        assert!(repository.core.picker.sources.tags.iter().any(|tag|tag.name=="new-tag"));
        assert_eq!(repository.take_change_set().unwrap().0.comparisons.len(),1);
    }
    #[test]
    fn f10_asks_for_the_shared_palette_and_opens_no_prompt_of_its_own() {
        let (_dir, mut repository) = fixture();
        assert!(!repository.handle(Event::FocusGained));
        assert!(repository.core.notice.is_none());
        key(&mut repository, KeyCode::F(10), KeyModifiers::NONE);
        assert!(std::mem::take(&mut repository.palette_request), "the App opens the palette");
        assert!(repository.prompt.is_none());
        assert_eq!(safe("evil\x1b]52;c;payload\x07"), "evil�]52;c;payload�");
    }
}

#[cfg(test)]
mod pr_adapter_tests {
    use super::*;
    use crossterm::event::KeyEvent;
    use lenz::pr::{
        Account, Comment, Fetched, PullRequest, Review, State, Thread, ViewStatus, ViewedMarks,
    };
    use lenz::pr_session::PrState;
    use ratatui::{Terminal, backend::TestBackend};
    #[test]
    fn pr_body_existing_current_and_outdated_threads_and_personal_marks_render_from_keys() {
        let (dir, mut repository) = super::tests::fixture();
        let pr = PullRequest {
            number: 9,
            title: "Existing PR".into(),
            author: Account {
                login: "ann".into(),
            },
            state: State::Open,
            is_draft: false,
            body: "Existing description\n\x1bterminal control".into(),
            base_ref_name: "main".into(),
            head_ref_name: "feature".into(),
            head_ref_oid: "head".into(),
            url: "https://github.com/owner/repo/pull/9".into(),
            additions: 1,
            deletions: 0,
            changed_files: 1,
            is_cross_repository: false,
            head_repository: None,
            head_repository_owner: None,
        };
        let thread = Thread {
            path: "file".into(),
            diff_side: lenz::view::Side::Right,
            line: Some(2),
            start_line: None,
            original_line: Some(2),
            is_outdated: false,
            is_resolved: true,
            comments: vec![Comment {
                author: Some(Account {
                    login: "reviewer".into(),
                }),
                body: "Existing review".into(),
                created_at: "today".into(),
                ..Default::default()
            }],
            subject_type: String::new(),
        };
        let mut outdated = thread.clone();
        outdated.line = None;
        outdated.is_outdated = true;
        outdated.comments[0].body = "Previous review".into();
        let mut state=PrState::new(pr,Fetched{number:9,merge_base:"base".into(),head:"head".into()});
        state.marks=Some(ViewedMarks::open(dir.path()).unwrap());
        state.review=Some(Review{discussion:Vec::new(),files:Vec::new(),placed:[(("file".into(),lenz::view::Side::Right,2),vec![thread])].into(),listed:[("file".into(),vec![outdated])].into(),..Default::default()});
        repository.core.pr=Some(state);
        repository.set_current(0, "file", "file");
        repository.core.execute(Command::Focus(Pane::Diff));
        let mut terminal = Terminal::new(TestBackend::new(100, 25)).unwrap();
        let draw = |repository: &mut Repository, terminal: &mut Terminal<TestBackend>| {
            terminal.draw(|frame| repository.render(frame)).unwrap();
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<Vec<_>>()
                .join("")
        };
        let key = |repository: &mut Repository, code| {
            assert!(repository.handle(Event::Key(KeyEvent::new(code, KeyModifiers::NONE))))
        };
        key(&mut repository, KeyCode::Char('d'));
        let text = draw(&mut repository, &mut terminal);
        assert!(text.contains("Existing description") && text.contains("�terminal control"), "{text}");
        assert!(
            text.contains("ann wants to merge")
                && text.contains("into main from feature")
                && text.contains("● Open")
                && text.contains("+1 −0"),
            "{text}"
        );
        key(&mut repository, KeyCode::End);
        key(&mut repository, KeyCode::Up);
        assert!(draw(&mut repository,&mut terminal).contains("Existing description"),"End then Up must clamp a short PR description");
        key(&mut repository, KeyCode::Esc);
        key(&mut repository, KeyCode::Char('t'));
        let text = draw(&mut repository, &mut terminal);
        assert!(text.contains("Existing review") && text.contains("resolved"));
        key(&mut repository, KeyCode::Esc);
        key(&mut repository, KeyCode::Char('T'));
        let text = draw(&mut repository, &mut terminal);
        assert!(text.contains("Previous review") && text.contains("outdated"));
        // Mark storage remains a local file. No API call is needed for the transitions above.
        assert_eq!(
            repository
                .core
                .pr
                .as_ref()
                .unwrap()
                .marks
                .as_ref()
                .unwrap()
                .status("url", "file", "blob"),
            ViewStatus::Unviewed
        );
        assert!(!dir.path().join("viewed.json").exists());
        key(&mut repository, KeyCode::Esc);
        key(&mut repository, KeyCode::Char('v'));
        let comparison = &repository.core.comparisons()[0];
        assert_eq!(
            repository.core.pr.as_ref().unwrap().view_status(comparison),
            Some(ViewStatus::Viewed)
        );
        assert!(dir.path().join("viewed.json").exists());
        assert!(draw(&mut repository, &mut terminal).contains("1/1 viewed"));
        // Review state stays observable when the repository graph and sidebar are hidden.
        repository.core.execute(Command::TogglePicker);
        repository.core.pr.as_mut().unwrap().pr.title = "Long title ".repeat(30);
        assert_eq!(repository.area(Rect::new(0, 0, 100, 25)).y, 1);
        let text = draw(&mut repository, &mut terminal);
        assert!(text.contains("PR #9 · 1/1 viewed · viewed"), "{text}");
    }
}

fn source_palette(p:&lenz::theme::Palette)->SourcePalette {SourcePalette{fg:p.fg,dim:p.dim,accent:p.accent,status:p.status}}
