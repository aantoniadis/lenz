//! Terminal input and cells for the shared merge workflow.
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind, MouseButton};
use lenz::{
    cli::Request,
    conflict_session::{FileView, ListAct, MergeWorkspace, State},
    merge::{BlockState, Pick},
    merge_session::{Act, Control},
    version_session::PickAct,
    view::{Content,Side},
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone,Copy)]
enum ToolbarAction { Run(lenz::catalog::Action), Palette, Version(usize), List(ListAct), CopyCommand(usize), Icons }

pub struct Merger {
    icons:crate::icons::Icons,
    resizer:crate::panes::Resizer,
    pane_ratios:[f32;2],files_width:Option<f32>,version_height:Option<f32>,
    comparison_ratio:f32,comparison_applied_height:Option<f32>,comparison_search_height:Option<f32>,
    pub palette:Arc<lenz::theme::Palette>,
    pub theme_revision:u64,
    pub theme_request:bool,
    pub workspace: MergeWorkspace,
    pub quit: bool,
    pub back: bool,
    pub new_view: Option<(Request, PathBuf)>,
    files_search:lenz::pane_search::PaneSearch,files_search_origin:Option<usize>,
    diff: Option<crate::app::App>,
    diff_path: Option<(String, bool, bool)>,
    menu: Option<usize>,menu_query:String,
    pub(crate) command_palette: Option<lenz::palette::Palette>,
    files_menu_target:Option<(usize,String)>,
    /// The change block a pointer menu is for: (file index, block, the current change before the menu).
    block_menu_target:Option<(usize,usize,Option<usize>)>,pending_files_action:Option<(usize,String,ListAct)>,
    menu_first:usize,menu_area:Rect,files_area:Rect,text_area:Rect,files_focused:bool,file_scroll:usize,last_file:Option<usize>,
    toolbar:crate::chrome::Bar<ToolbarAction>,
    help:Option<usize>,
    message:Option<crate::help::Message>,
    about:Option<crate::about::About>,
    report_request:lenz::diagnostics::ReportRequest,
    horizontal: [usize;3],
    columns:[Option<usize>;3],horizontal_version:Option<(u64,bool)>,
    height: u16,
    width: u16,
    preview_base: bool,
}
impl Merger {
    pub fn open(request: Request, cwd: PathBuf) -> Self {
        Self::new(MergeWorkspace::open(request, cwd))
    }
    pub fn from_repository(root: PathBuf, paths: Vec<String>) -> Self {
        Self::new(MergeWorkspace::from_repository(root, paths))
    }
    fn new(workspace: MergeWorkspace) -> Self {
        Self {
            icons:crate::icons::Icons::from_env(),resizer:Default::default(),pane_ratios:[1.0/3.0,2.0/3.0],files_width:None,version_height:None,comparison_ratio:0.5,comparison_applied_height:None,comparison_search_height:None,
            palette:Arc::new(*crate::chrome::initial_palette()),theme_revision:1,theme_request:false,
            workspace,
            quit: false,
            back: false,
            new_view: None,
            files_search:Default::default(),files_search_origin:None,diff: None,
            diff_path: None,
            menu: None,menu_query:String::new(),command_palette:None,files_menu_target:None,block_menu_target:None,pending_files_action:None,
            menu_first:0,menu_area:Rect::default(),files_area:Rect::default(),text_area:Rect::default(),files_focused:false,file_scroll:0,last_file:None,
            toolbar:Default::default(),help:None,message:None,about:None,report_request:Default::default(),
            horizontal:[0;3],columns:[None;3],horizontal_version:None,
            height: 20,
            width: 80,
            preview_base: false,
        }
    }
    pub(crate) fn cancel_pane_drag(&mut self) {self.resizer.cancel();if let Some(app)=&mut self.diff {app.cancel_pane_drag();}}
    fn reset_pane_sizes(&mut self) {
        self.pane_ratios=[1.0/3.0,2.0/3.0];self.files_width=None;self.version_height=None;self.comparison_ratio=0.5;self.comparison_applied_height=None;self.comparison_search_height=None;
        self.cancel_pane_drag();if let Some(app)=&mut self.diff {app.reset_pane_sizes();app.reset_sizes_requested=false;}
    }
    fn merge_panes(&self)->[Rect;3] {
        if self.text_area.width<45 {
            let focus=match &self.workspace.file {Some(FileView::Merge(m))=>m.find_pane(),_=>1};
            let mut panes=[Rect::new(self.text_area.x,self.text_area.y,0,0);3];panes[focus]=self.text_area;panes
        }else{crate::panes::three(self.text_area,self.pane_ratios)}
    }
    fn pointer_pane(&self,x:u16)->usize {
        let panes=self.merge_panes();panes.iter().position(|p|p.width>0&&x>=p.x&&x<p.right()).unwrap_or_else(||match &self.workspace.file{Some(FileView::Merge(m))=>m.find_pane(),_=>1})
    }
    fn resize_input(&mut self,event:&Event)->bool {
        let blocked=self.menu.is_some()||self.command_palette.is_some()||self.help.is_some()||self.message.is_some()||self.about.is_some()||self.toolbar.menu_open()||self.workspace.confirming()
            ||matches!(&self.workspace.file,Some(FileView::Merge(m))if m.ask.is_some()||m.find_editing())
            ||matches!(&self.workspace.file,Some(FileView::Picker(p))if p.ask.is_some())
            ||self.diff.as_ref().is_some_and(|a|a.modal_input()||a.toolbar.menu_open());
        if blocked||matches!(event,Event::Resize(..)|Event::FocusLost) {self.cancel_pane_drag();return false;}
        match self.resizer.handle(event,false) {
            crate::panes::Resize::None=>self.diff.as_mut().is_some_and(|a|a.resize_input(event)),crate::panes::Resize::Captured=>true,
            crate::panes::Resize::Move(0,width)=>{self.files_width=Some((width+1.0).max(0.0));true},
            crate::panes::Resize::Move(3,top)=>{self.version_height=Some((f32::from(self.text_area.height)-top).max(0.0));true},
            crate::panes::Resize::Move(id,width)=>{
                let total=f32::from(self.text_area.width);let min=12.0f32.min(total/3.0);
                if total>0.0 {let widths=lenz::pane_layout::three(total,self.pane_ratios,min);let desired=width+1.0;
                    if id==1 {self.pane_ratios[0]=desired.clamp(min,(widths[0]+widths[1]-min).max(min))/total;}
                    else {self.pane_ratios[1]=desired.clamp(widths[0]+min,(total-min).max(widths[0]+min))/total;}}
                true
            }
        }
    }
    fn request_theme(&mut self){if self.about.is_none()&&!matches!(&self.workspace.file,Some(FileView::Merge(m))if m.ask.is_some())&&!matches!(&self.workspace.file,Some(FileView::Picker(p))if p.ask.is_some()){self.theme_request=true;}}
    pub fn take_theme_request(&mut self)->bool{let nested=self.diff.as_mut().is_some_and(|a|std::mem::take(&mut a.theme_request));std::mem::take(&mut self.theme_request)||nested}
    pub fn exit_code(&self) -> u8 {
        self.workspace.exit_code
    }
    pub fn busy(&self) -> bool {
        self.files_search.searching(self.workspace.rows.len())||self.report_request.busy() || self.workspace.busy() || self.diff.as_ref().is_some_and(|a| a.busy())
    }
    pub fn poll(&mut self) -> bool {
        let reset=self.diff.as_mut().is_some_and(|a|std::mem::take(&mut a.reset_sizes_requested));
        if reset {self.reset_pane_sizes();}
        if let Some(app)=&self.diff {self.icons=app.icons;self.comparison_ratio=app.pane_ratio;self.comparison_applied_height=app.applied_height;if let Some(search)=&app.set_search {self.comparison_search_height=search.desired_height;}}
        let mut changed = self.workspace.poll();
        if changed{self.files_search.invalidate();}
        changed|=self.poll_files_search();
        if !self.workspace.busy()&&let Some((index,path,action))=self.pending_files_action.take(){
            if self.workspace.rows.get(index).is_some_and(|row|row.0==path)&&self.workspace.file.as_ref().is_some_and(|file|file.path()==path){self.list_command(action);}
            else {self.workspace.notice=Some("File changed while opening; choose the action again".into());}
            changed=true;
        }
        if let Some(result)=self.report_request.poll(){
            changed=true;
            match result {
                Ok(report)=>{self.workspace.notice=Some(format!("Review report: {}",report.path.display()));if let Err(error)=lenz::open::browse(&report.issue_url){self.workspace.notice=Some(format!("{error}; use lenz diff --tui report --last --print"));}},
                Err(error)=>self.workspace.notice=Some(error.to_string()),
            }
        }
        if self.preview_dirty() { self.workspace.quit = false; self.back = false; }
        self.quit = self.workspace.quit;
        if !self.preview_dirty() && self.workspace.root.is_some()
            && self.workspace.snapshot.is_none()
            && self.workspace.file.is_none()
            && !self.workspace.busy()
            && !self.back
        {
            self.back = true;
            changed = true;
        }
        let diff_path = match &self.workspace.file {
            Some(FileView::Marked { path, .. }) => Some((path.clone(), false, false)),
            Some(FileView::Picker(p)) => Some((p.o.path.clone(), self.preview_base, true)),
            _ => None,
        };
        // A refresh already in flight may remove the operation or change its view.
        // Keep the draft adapter alive until the user explicitly saves or discards it.
        if diff_path != self.diff_path && self.preview_dirty() {
            self.files_focused = false;
            self.workspace.notice = Some("Unsaved comparison changes — Save (Ctrl-S) or Discard unsaved changes first".into());
        } else if diff_path != self.diff_path {
            self.diff_path = diff_path;
            self.diff = match self.workspace.comparison(self.preview_base) {
                Ok(Some((set, spec))) => {
                    let mut app=crate::app::App::new(lenz::session::Session::from_set(Arc::new(set),spec,Default::default(),Default::default()));app.icons=self.icons;Some(app)
                }
                Ok(None) => None,
                Err(e) => {
                    self.workspace.notice = Some(e);
                    None
                }
            };
        }
        if let Some(app) = &mut self.diff {
            app.pane_ratio=self.comparison_ratio;app.applied_height=self.comparison_applied_height;if let Some(search)=&mut app.set_search {search.desired_height=self.comparison_search_height;}
            changed |= app.poll();
            self.theme_request|=std::mem::take(&mut app.theme_request);
            if let Some(request) = app.new_view.take() {
                self.new_view = Some(request);
                changed = true;
            }
        }
        changed
    }
    fn copy_command(&mut self, index: usize) {
        if let Some(command) = self.workspace.copyable_commands().get(index) {
            self.workspace.notice = Some(match crate::clipboard::copy(command) {
                Ok(()) => "Command copied (terminal OSC 52 clipboard)".into(),
                Err(e) => format!("Copy failed: {e}"),
            });
        } else {
            self.workspace.notice = Some("No continuation or submodule command to copy".into());
        }
    }
    /// A change menu closed without a choice: the current change goes back to where it was.
    fn dismiss_block_menu(&mut self){
        if let Some((file,_,previous))=self.block_menu_target.take() && file==self.workspace.current && let Some(FileView::Merge(m))=&mut self.workspace.file {
            m.current=previous.filter(|&i|i<m.m.blocks().len());
        }
    }
    fn open_menu(&mut self){self.menu_query.clear();self.menu=Some(0);}
    /// The file list's pointer menu: open, copy path, then the list actions.
    fn menu_labels(&self)->Vec<(String,Option<String>)>{
        if let Some((_,block,_))=self.block_menu_target {
            let controls=match &self.workspace.file {Some(FileView::Merge(m))=>m.controls_for(block),_=>Vec::new()};
            return controls.into_iter().map(|c|(lenz::catalog::pointer::control(c).to_string(),None)).chain(std::iter::once(("All commands…".into(),None))).collect();
        }
        std::iter::once(("Open file".into(),None)).chain(std::iter::once(("Copy file path".into(),None))).chain(ListAct::ALL.iter().map(|a|(a.label().into(),None))).collect()
    }
    fn menu_matches(&self)->Vec<usize>{let query=self.menu_query.to_lowercase();self.menu_labels().iter().enumerate().filter_map(|(index,(label,_))|label.to_lowercase().contains(&query).then_some(index)).collect()}
    fn menu_step(&mut self,amount:isize){let matches=self.menu_matches();let at=matches.iter().position(|i|Some(*i)==self.menu).unwrap_or(0);let next=at.saturating_add_signed(amount).min(matches.len().saturating_sub(1));self.menu=Some(matches.get(next).copied().unwrap_or(0));}
    fn comparison_commands(&mut self) -> bool {
        let Some(app) = &mut self.diff else {
            return false;
        };
        if app.prompting() {
            return false;
        }
        self.menu = None;self.files_menu_target=None;self.block_menu_target=None;
        app.handle(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char(':'),
            KeyModifiers::NONE,
        )))
    }
    fn preview_dirty(&self) -> bool {
        self.diff.as_ref().is_some_and(|app| app.session.has_dirty())
    }
    fn guard_preview_edits(&mut self) -> bool {
        if !self.diff.as_mut().is_some_and(|app| app.guard_pending_edits()) { return false; }
        self.files_focused = false;
        self.menu = None;
        self.files_menu_target = None;
        self.block_menu_target = None;
        self.pending_files_action = None;
        self.workspace.quit = false;
        self.quit = false;
        self.back = false;
        self.workspace.notice = Some("Unsaved comparison changes — Save (Ctrl-S) or Discard unsaved changes first".into());
        true
    }
    fn open_index(&mut self, index: usize) { if !self.guard_preview_edits() { self.workspace.open_index(index); } }
    fn list_command(&mut self, action: ListAct) { if !self.guard_preview_edits() { self.workspace.list_command(action); } }
    fn pick(&mut self, action: PickAct) { if !self.guard_preview_edits() { self.workspace.pick(action); } }
    fn close(&mut self) { if !self.guard_preview_edits() { self.workspace.close(); } }
    fn refresh(&mut self) { if !self.guard_preview_edits() { self.workspace.refresh(); } }
    fn switch_file(&mut self, forward: bool, unresolved: bool) { if !self.guard_preview_edits() { self.workspace.switch_file(forward, unresolved); } }
    fn open_current(&mut self) { if !self.guard_preview_edits() { self.workspace.open_current(); } }
    fn mark_current(&mut self) { if !self.guard_preview_edits() { self.workspace.mark_current(); } }
    fn reopen_current(&mut self) { if !self.guard_preview_edits() { self.workspace.reopen_current(); } }
    fn merge_anyway(&mut self) { if !self.guard_preview_edits() { self.workspace.merge_anyway(); } }
    fn action(&mut self, action: Act) {
        if self.guard_preview_edits() { return; }
        self.workspace.execute(action);
        self.quit = self.workspace.quit;
    }
    fn control(&mut self, choice: u32) {
        if self.guard_preview_edits() { return; }
        let Some(FileView::Merge(m)) = &mut self.workspace.file else {
            return;
        };
        let Some(i) = m
            .current
            .or_else(|| (0..m.m.blocks().len()).find(|&i| m.m.state(i) == BlockState::Unresolved))
        else {
            self.workspace.notice = Some("No change here".into());
            return;
        };
        let control = match choice {
            1 => Control::Accept(i, Pick::Ours),
            2 => Control::Accept(i, Pick::Theirs),
            3 => Control::Both(i),
            4 => Control::KeepBase(i),
            5 => Control::Reopen(i),
            _ => return,
        };
        m.control(control);
    }
    fn scroll(&mut self, amount: f32) {
        if let Some(FileView::Merge(m)) = &mut self.workspace.file {
            m.scroll_by(amount);
        }
    }
    /// The GUI's merge toolbar: ☰, change and conflict arrows, Undo/Redo/Save, Done/Cancel, ⋯ (the palette); the
    /// Version Picker's version buttons and the Conflict List's file actions stay as terminal mouse paths.
    fn toolbar_items(&self,width:u16)->Vec<crate::chrome::Item<ToolbarAction>> {
        use crate::chrome::Item;use ToolbarAction::*;use lenz::catalog::Action;
        let item=|label:std::borrow::Cow<'static,str>,action:Action|{let mut item=Item::new(label,Run(action));item.disabled=lenz::catalog::Availability::disabled(self,action);item};
        let mut app_menu:Vec<_>=lenz::catalog::app_menu().into_iter().map(|e|item(e.label,e.action)).collect();
        app_menu.push(Item::new("Nerd Font icons (Mono)",Icons).active(self.icons.nerd()));
        let mut items=vec![Item::new("☰",Palette).choices(app_menu)];
        let wide=width>=80;
        match &self.workspace.file {
            Some(FileView::Merge(_))=>{
                if wide {for (label,act) in [("↑",Act::PrevChange),("↓",Act::NextChange),("⇡",Act::PrevConflict),("⇣",Act::NextConflict)] {items.push(item(label.into(),Action::Merge(act)));}}
                for (label,act) in [("↶",Act::Undo),("↷",Act::Redo),("⤓",Act::Save)] {items.push(item(label.into(),Action::Merge(act)));}
                if wide {for act in [Act::Done,Act::Cancel] {items.push(item(if act==Act::Done{"Done"}else{"Cancel"}.into(),Action::Merge(act)));}}
            }
            Some(FileView::Picker(p))=>{
                for (label,act) in [("↶",PickAct::Undo),("↷",PickAct::Redo),("⤓",PickAct::Save)] {items.push(item(label.into(),Action::Pick(act)));}
                items.extend([item("Done".into(),Action::Pick(PickAct::Done)),item("Cancel".into(),Action::Pick(PickAct::Cancel))]);
                if !p.mergetool && wide {items.push(item("Mark resolved".into(),Action::Pick(PickAct::MarkResolved)));}
                for (i,version) in p.o.versions.iter().enumerate(){items.push(Item::new(format!("Take {}",version.label),Version(i)).active(p.chosen==Some(i)).disabled_message(version.error.clone()));}
            }
            Some(FileView::Marked{..})|None=>{}
        }
        if self.workspace.root.is_some(){items.push(Item::new("File actions ▾",Palette).choices(ListAct::ALL.map(|action|Item::new(action.label(),List(action)).disabled(self.workspace.list_disabled(action))).into()));}
        let copyable=self.workspace.copyable_commands();
        if !copyable.is_empty(){items.push(Item::new("Copy command ▾",Palette).choices(copyable.iter().enumerate().map(|(i,c)|Item::new(clean(c),CopyCommand(i))).collect()));}
        items.push(Item::new("⋯",Palette).overflow());
        items
    }
    fn toolbar_action(&mut self,action:ToolbarAction){
        match action {
            ToolbarAction::Icons=>{self.icons.toggle();if let Some(app)=&mut self.diff{app.icons=self.icons;}},
            ToolbarAction::Run(action)=>self.run_action(action),
            ToolbarAction::Palette=>self.open_palette(),
            ToolbarAction::Version(index)=>{if !self.guard_preview_edits() && let Some(FileView::Picker(p))=&mut self.workspace.file{p.take(index);}},
            ToolbarAction::List(action)=>self.list_command(action),
            ToolbarAction::CopyCommand(index)=>self.copy_command(index),
        }
        self.quit=self.workspace.quit;
    }
    fn files_content_area(&self)->Rect {let area=self.files_area;if self.files_search.active(){Rect::new(area.x,area.y+u16::from(area.height>0),area.width,area.height.saturating_sub(1))}else{area}}
    fn begin_files_search(&mut self) {if !self.files_search.active(){self.files_search_origin=Some(self.file_scroll);self.files_search.start(self.workspace.current);}}
    fn reveal_files_search(&mut self) {let height=usize::from(self.files_content_area().height).max(1);if let Some(index)=self.files_search.current(){if index<self.file_scroll{self.file_scroll=index;}else if index>=self.file_scroll+height{self.file_scroll=index.saturating_add(1).saturating_sub(height);}}}
    fn poll_files_search(&mut self)->bool {let before=self.files_search.current();let rows=&self.workspace.rows;let changed=self.files_search.poll(rows.len(),|i,q|lenz::pane_search::contains(&rows[i].0,q));if before!=self.files_search.current(){self.reveal_files_search();}changed}
    fn files_search_key(&mut self,key:crossterm::event::KeyEvent)->bool {
        let ctrl=key.modifiers==KeyModifiers::CONTROL;let shift=key.modifiers.contains(KeyModifiers::SHIFT);
        if key.code==KeyCode::F(10)&&key.modifiers.is_empty(){self.open_palette();return true;}
        if key.code==KeyCode::F(1){self.help=Some(0);return true;}
        if ctrl&&key.code==KeyCode::Char('c')&&!shift {
            if let Some((path,_))=self.workspace.rows.get(self.files_search.current().unwrap_or(self.workspace.current)){
                self.workspace.notice=Some(match crate::clipboard::copy(path){Ok(())=>"Path copied (terminal OSC 52 clipboard)".into(),Err(error)=>format!("Copy failed: {error}")});
            }return true;
        }
        if ctrl&&key.code==KeyCode::Char('f')&&!shift{self.begin_files_search();return true;}
        if ctrl&&key.code==KeyCode::Char('a'){self.begin_files_search();self.files_search.set_query(String::new());return true;}
        if key.modifiers.difference(KeyModifiers::SHIFT).is_empty()&&let KeyCode::Char(ch)=key.code{self.begin_files_search();self.files_search.append(&ch.to_string());self.poll_files_search();return true;}
        if !self.files_search.active(){return false;}
        match key.code {
            KeyCode::Up|KeyCode::Down|KeyCode::F(3)=>{self.poll_files_search();self.files_search.step(key.code==KeyCode::Down||key.code==KeyCode::F(3)&&!shift);self.reveal_files_search();}
            KeyCode::Backspace=>{self.files_search.backspace();self.poll_files_search();}
            KeyCode::Enter=>{if let Some(index)=self.files_search.confirm(){self.files_search_origin=Some(self.file_scroll);self.open_index(index);}}
            KeyCode::Esc=>{if let Some(scroll)=self.files_search_origin.take(){self.file_scroll=scroll;}self.files_search.cancel();}
            KeyCode::Home|KeyCode::End|KeyCode::PageUp|KeyCode::PageDown=>{
                let height=usize::from(self.files_content_area().height).max(1);let max=self.workspace.rows.len().saturating_sub(height);
                self.file_scroll=match key.code{KeyCode::Home=>0,KeyCode::End=>max,KeyCode::PageUp=>self.file_scroll.saturating_sub(height),_=>self.file_scroll.saturating_add(height).min(max)};
            },
            _=>return false,
        }true
    }
    fn details_text(&self)->String {
        if let Some(index)=self.menu&&let Some((label,reason))=self.menu_labels().get(index){return reason.as_ref().map_or_else(||label.clone(),|reason|format!("{label} — {reason}"));}
        if let Some(p)=&self.command_palette {return p.selected().and_then(|i|p.rows().nth(i)).map_or_else(||"No selected action".into(),|row|row.disabled.as_deref().map_or_else(||row.entry.label.to_string(),|why|format!("{} — {why}",row.entry.label)));}
        let mut parts=Vec::new();
        match &self.workspace.file {
            Some(FileView::Merge(m))=>{parts.extend(m.question().map(str::to_owned));parts.extend(m.error.clone());parts.extend(m.find_notice());parts.extend(m.notices());},
            Some(FileView::Picker(p))=>{
                parts.extend(p.question().map(str::to_owned));parts.extend(p.o.info.iter().cloned());
                parts.extend(p.o.versions.iter().map(|v|format!("{} — {}",v.label,v.error.as_deref().unwrap_or(&v.note))));
            },_=>{}
        }
        parts.extend(self.workspace.notice.clone());
        if let Some(app)=&self.diff{parts.push(crate::render::status_text(app));}
        if parts.is_empty(){"No status message".into()}else{parts.join("\n\n")}
    }
    pub fn handle(&mut self, event: Event) -> bool {
        let closing_details=matches!(event,Event::Key(k)if k.kind!=KeyEventKind::Release&&k.modifiers==KeyModifiers::CONTROL&&matches!(k.code,KeyCode::Char('q'|'c')));
        if !closing_details&&let Some(message)=&mut self.message {
            if let Event::Resize(w,h)=event {self.width=w;self.height=h;}
            if message.handle(&event){self.message=None;}
            return true;
        }
        if self.resize_input(&event){return true;}
        let list_copy=matches!(event,Event::Key(k)if k.kind!=KeyEventKind::Release&&k.modifiers==KeyModifiers::CONTROL&&k.code==KeyCode::Char('c'))&&self.files_focused&&self.about.is_none()&&self.help.is_none()&&self.message.is_none()&&self.menu.is_none()&&!self.workspace.confirming()&&!matches!(&self.workspace.file,Some(FileView::Merge(m))if m.ask.is_some()||m.find_editing())&&!matches!(&self.workspace.file,Some(FileView::Picker(p))if p.ask.is_some())&&self.diff.as_ref().is_none_or(|a|!a.modal_input());
        let quit=!list_copy&&matches!(event,Event::Key(k)if k.kind!=KeyEventKind::Release&&k.modifiers.contains(KeyModifiers::CONTROL)&&matches!(k.code,KeyCode::Char('q'|'c')));
        if quit && self.guard_preview_edits() { return true; }
        if !quit && let Some(about)=&mut self.about {
            if let Event::Resize(w,h)=event {self.width=w;self.height=h;}
            if about.handle(&event){self.about=None;}
            return true;
        }
        if !quit && let Some(app)=self.diff.as_mut().filter(|app|app.about.is_some()) {
            if let Event::Resize(w,h)=event{self.width=w;self.height=h;}
            return app.handle(event);
        }
        if !quit && let Some(scroll)=&mut self.help {
            match &event {
                Event::Key(k) if k.kind!=KeyEventKind::Release=>match k.code {
                    KeyCode::Esc|KeyCode::Char('q'|'?')=>self.help=None,
                    KeyCode::Home=>*scroll=0,KeyCode::End=>*scroll=usize::MAX,
                    KeyCode::Up=>*scroll=scroll.saturating_sub(1),KeyCode::Down=>*scroll=scroll.saturating_add(1),
                    KeyCode::PageUp=>*scroll=scroll.saturating_sub(10),KeyCode::PageDown=>*scroll=scroll.saturating_add(10),_=>{}
                },
                Event::Mouse(m)=>match m.kind{MouseEventKind::ScrollUp=>*scroll=scroll.saturating_sub(3),MouseEventKind::ScrollDown=>*scroll=scroll.saturating_add(3),_=>{}},
                _=>{}
            }
            return true;
        }
        if !quit&&matches!(event,Event::Key(k)if k.kind!=KeyEventKind::Release&&k.code==KeyCode::F(2)&&k.modifiers.is_empty())&&!matches!(&self.workspace.file,Some(FileView::Merge(m))if m.find_editing()&&m.ask.is_none()) {
            if self.diff.as_ref().is_some_and(|a|a.modal_input()){return self.diff.as_mut().unwrap().handle(event);}
            self.message=Some(crate::help::Message::new(self.details_text(),Rect::new(0,0,self.width,self.height)));return true;
        }
        if !quit && let Some(p)=&mut self.command_palette {
            use crate::palette::Input;use lenz::palette::Outcome;
            match crate::palette::handle(p,&event,crate::palette::rect(Rect::new(0,0,self.width,self.height),p)) {
                Input::Ignored=>{},
                Input::Handled=>return true,
                Input::Close=>{self.command_palette=None;return true;}
                Input::Done(Outcome::Run(action))=>{self.run_action(action);self.quit=self.workspace.quit;return true;}
                Input::Done(Outcome::Disabled(why))=>{self.workspace.notice=Some(why.into_owned());return true;}
                Input::Done(Outcome::Nothing)=>return true,
            }
        }
        let editing=matches!(&self.workspace.file,Some(FileView::Merge(m))if m.find_editing())||self.diff.as_ref().is_some_and(|a|a.prompting());
        let asking=matches!(&self.workspace.file,Some(FileView::Merge(m))if m.ask.is_some())||matches!(&self.workspace.file,Some(FileView::Picker(p))if p.ask.is_some())||self.workspace.confirming();
        if !quit&&asking&&!matches!(event,Event::Resize(..)|Event::FocusGained){
            let supported=matches!(event,Event::Key(k)if k.kind!=KeyEventKind::Release&&(k.code==KeyCode::Esc||(!k.modifiers.contains(KeyModifiers::CONTROL)&&matches!(k.code,KeyCode::Char('q'|'r'|'M')))||(k.modifiers.contains(KeyModifiers::CONTROL)&&matches!(k.code,KeyCode::Char('s')|KeyCode::Enter))));
            if !supported{return true;}
        }
        if !quit&&!asking&&self.menu.is_none()&&self.diff.as_ref().is_some_and(|a|a.modal_input())&&matches!(event,Event::Key(_)|Event::Mouse(_)|Event::Paste(_)){
            return self.diff.as_mut().unwrap().handle(event);
        }
        if !quit&&!editing&&!asking&&self.menu.is_none()&&self.toolbar.focused(){
            let(handled,action)=self.toolbar.handle(&event);
            if let Some(action)=action{self.toolbar_action(action);}
            if handled{return true;}
        }
        if !quit&&!asking&&self.menu.is_none()&&self.diff.as_ref().is_some_and(|app|app.toolbar.focused()){return self.diff.as_mut().unwrap().handle(event);}
        // F10 is the command palette here too (Alt+F10 still walks the toolbar)
        if !quit&&!editing&&!asking&&self.menu.is_none()&&matches!(event,Event::Key(k)if k.kind!=KeyEventKind::Release&&k.code==KeyCode::F(10)&&k.modifiers.is_empty()){self.open_palette();return true;}
        let context_request=matches!(event,Event::Mouse(m)if m.kind==MouseEventKind::Down(MouseButton::Right))||matches!(event,Event::Key(k)if k.kind!=KeyEventKind::Release&&k.code==KeyCode::F(10)&&k.modifiers.contains(KeyModifiers::SHIFT));
        if context_request&&self.menu.is_none(){
            if editing||asking{return true;}
            let files_request=self.files_area.width>0&&match event{Event::Mouse(mouse)=>self.files_area.contains((mouse.column,mouse.row).into()),Event::Key(_)=>self.files_focused,_=>false};
            if files_request{
                let index=if let Event::Mouse(mouse)=event{let content=self.files_content_area();if mouse.row<content.y{return true;}self.file_scroll+usize::from(mouse.row-content.y)}else{self.files_search.current().unwrap_or(self.workspace.current)};
                let Some((path,_))=self.workspace.rows.get(index)else{return true;};
                self.files_menu_target=Some((index,path.clone()));self.files_focused=true;
            }else if let Some(app)=&mut self.diff{self.files_focused=false;return app.handle(event);}
            // the change block under the pointer (Shift-F10: the current one) lists its choices; elsewhere, the palette
            else {
                let panes=self.merge_panes();
                let block=match (&self.workspace.file,&event) {
                    (Some(FileView::Merge(m)),Event::Mouse(mouse))=>{
                        // row 0 of a pane is its header
                        panes.iter().position(|p|p.width>0&&p.contains((mouse.column,mouse.row).into()))
                            .and_then(|pane|mouse.row.checked_sub(panes[pane].y+1).and_then(|row|m.block_on_row(pane,row)))
                    }
                    (Some(FileView::Merge(m)),_)=>m.current.filter(|&i|i<m.m.blocks().len()),
                    _=>None,
                };
                let Some(block)=block else {self.open_palette();return true;};
                let previous=match &mut self.workspace.file {Some(FileView::Merge(m))=>{let previous=m.current;m.focus_block(block);previous},_=>None};
                self.block_menu_target=Some((self.workspace.current,block,previous));
            }
            self.open_menu();return true;
        }
        if self.menu.is_some()&&let Event::Paste(text)=&event{self.menu_query.push_str(&text.replace(['\n','\r']," "));self.menu=Some(self.menu_matches().first().copied().unwrap_or(0));return true;}
        if let Event::Mouse(mouse)=event&&self.menu.is_some(){
            match mouse.kind{
                MouseEventKind::ScrollUp=>self.menu_step(-3),MouseEventKind::ScrollDown=>self.menu_step(3),
                MouseEventKind::Down(MouseButton::Left)=>{let area=self.menu_area;if area.contains((mouse.column,mouse.row).into())&&mouse.row>area.y&&mouse.row<area.bottom().saturating_sub(1){let slot=self.menu_first+usize::from(mouse.row-area.y-1);if let Some(&index)=self.menu_matches().get(slot){self.menu=Some(index);return self.handle(Event::Key(crossterm::event::KeyEvent::new(KeyCode::Enter,KeyModifiers::NONE)));}}else{self.menu=None;self.files_menu_target=None;self.dismiss_block_menu();}},_=>{}
            }return true;
        }
        if !quit&&!editing&&!asking&&self.menu.is_none(){
            if let Event::Mouse(mouse)=event{
                if self.files_area.width>0&&self.files_area.contains((mouse.column,mouse.row).into()){
                    let content=self.files_content_area();let visible=usize::from(content.height);let max=self.workspace.rows.len().saturating_sub(visible);
                    match mouse.kind{MouseEventKind::ScrollUp=>self.file_scroll=self.file_scroll.saturating_sub(3),MouseEventKind::ScrollDown=>self.file_scroll=(self.file_scroll+3).min(max),MouseEventKind::Down(MouseButton::Left)=>{self.files_focused=true;if mouse.row>=content.y{let index=self.file_scroll+usize::from(mouse.row-content.y);self.open_index(index);self.files_search_origin=Some(self.file_scroll);}},_=>{}}return true;
                }
                if self.text_area.contains((mouse.column,mouse.row).into())&&mouse.kind==MouseEventKind::Down(MouseButton::Left){self.files_focused=false;let pane=self.pointer_pane(mouse.column);if let Some(FileView::Merge(m))=&mut self.workspace.file{m.focus_pane(pane);return true;}}
                if matches!(mouse.kind,MouseEventKind::ScrollUp|MouseEventKind::ScrollDown|MouseEventKind::ScrollLeft|MouseEventKind::ScrollRight)&&!self.text_area.contains((mouse.column,mouse.row).into()){return true;}
            }
            if let Some(app)=&mut self.diff&&let Event::Key(k)=event&&k.kind!=KeyEventKind::Release&&matches!(k.code,KeyCode::Tab|KeyCode::BackTab|KeyCode::F(6)){
                use lenz::navigator::{InputPane as P,cycle_pane};
                let current=if self.files_focused{P::Files}else if app.set_search.as_ref().is_some_and(|s|s.focused){P::Results}else if app.focus==Side::Left{P::Left}else{P::Right};
                let panes=[(self.files_area.width>0).then_some(P::Files),Some(P::Left),Some(P::Right),app.set_search.as_ref().map(|_|P::Results)];
                let target=cycle_pane(current,&panes,k.code==KeyCode::BackTab||k.modifiers.contains(KeyModifiers::SHIFT)).unwrap();
                self.files_focused=target==P::Files;app.navigator.focused=false;if let Some(search)=&mut app.set_search{search.focused=target==P::Results;}
                match target{P::Left=>app.focus=Side::Left,P::Right=>app.focus=Side::Right,P::Files|P::Results=>{},P::Sources|P::Log|P::Middle=>unreachable!("picker preview pane cycle")};return true;
            }
            if self.files_focused&&let Event::Paste(text)=&event{self.begin_files_search();self.files_search.append(&text.replace(['\n','\r']," "));self.poll_files_search();return true;}
            if self.files_focused&&let Event::Key(k)=event&&k.kind!=KeyEventKind::Release{
                if self.files_search_key(k){return true;}
                let max=self.workspace.rows.len().saturating_sub(1);let index=match k.code{KeyCode::Up=>Some(self.workspace.current.saturating_sub(1)),KeyCode::Down=>Some((self.workspace.current+1).min(max)),KeyCode::Home=>Some(0),KeyCode::End=>Some(max),KeyCode::PageUp=>Some(self.workspace.current.saturating_sub(usize::from(self.files_area.height))),KeyCode::PageDown=>Some((self.workspace.current+usize::from(self.files_area.height)).min(max)),KeyCode::Left|KeyCode::Right if !k.modifiers.contains(KeyModifiers::ALT|KeyModifiers::SHIFT)=>return true,KeyCode::Esc|KeyCode::Enter=>{self.files_focused=false;return true;},_=>None};if let Some(index)=index{self.open_index(index);return true;}
            }
        }
        if !quit && !editing && self.menu.is_none() {
            if matches!(event,Event::Key(k)if k.kind!=KeyEventKind::Release&&k.code==KeyCode::Char('?')){self.help=Some(0);return true;}
            if !self.preview_dirty() && (matches!(self.workspace.file,Some(FileView::Merge(_)|FileView::Picker(_)))||self.diff.is_none()){let(handled,action)=self.toolbar.handle(&event);if let Some(action)=action{self.toolbar_action(action);}
                if handled{return true;}}
        }
        match event {
            Event::Resize(w, h) => {
                self.width = w;
                self.height = h;
                if let Some(app) = &mut self.diff {
                    app.handle(Event::Resize(w, h));
                }
                true
            }
            Event::Paste(text) => {
                if let Some(FileView::Merge(m)) = &mut self.workspace.file
                    && m.ask.is_none()
                    && m.find_editing()
                {
                    m.type_find(&text);
                    return true;
                }

                self.diff
                    .as_mut()
                    .is_some_and(|app| app.handle(Event::Paste(text)))
            }
            Event::FocusGained => {
                self.refresh();
                if let Some(FileView::Merge(m)) = &mut self.workspace.file {
                    m.check_disk();
                }
                if let Some(FileView::Picker(p)) = &mut self.workspace.file {
                    p.check_disk();
                }
                true
            }
            Event::Mouse(mouse) => {
                if let Some(app) = &mut self.diff {
                    return app.handle(Event::Mouse(mouse));
                }
                match mouse.kind {
                    MouseEventKind::ScrollRight|MouseEventKind::ScrollLeft=>{
                        let pane=self.pointer_pane(mouse.column);
                        self.scroll_horizontal(pane.min(2),if mouse.kind==MouseEventKind::ScrollLeft{-4}else{4});
                    }
                    MouseEventKind::ScrollDown|MouseEventKind::ScrollUp if mouse.modifiers.contains(KeyModifiers::SHIFT)=>{
                        let pane=self.pointer_pane(mouse.column);
                        self.scroll_horizontal(pane.min(2),if mouse.kind==MouseEventKind::ScrollUp{-4}else{4});
                    }
                    MouseEventKind::ScrollDown => self.scroll(3.0),
                    MouseEventKind::ScrollUp => self.scroll(-3.0),
                    _ => return false,
                }
                true
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                let shift = key.modifiers.contains(KeyModifiers::SHIFT);
                let alt = key.modifiers.contains(KeyModifiers::ALT);
                if ctrl && !shift && matches!(key.code, KeyCode::Char('q' | 'c')) {
                    self.close();
                    self.quit = self.workspace.quit;
                    return true;
                }
                if key.code == KeyCode::F(4) {
                    return self.comparison_commands();
                }
                if let Some(at) = self.menu {
                    match key.code {
                        KeyCode::Esc => {self.menu = None;self.files_menu_target=None;self.dismiss_block_menu();},
                        KeyCode::Up=>self.menu_step(-1),KeyCode::Down=>self.menu_step(1),
                        KeyCode::Home=>self.menu=Some(self.menu_matches().first().copied().unwrap_or(0)),
                        KeyCode::End=>self.menu=Some(self.menu_matches().last().copied().unwrap_or(0)),
                        KeyCode::PageUp=>self.menu_step(-10),KeyCode::PageDown=>self.menu_step(10),
                        KeyCode::Backspace=>{self.menu_query.pop();self.menu=Some(self.menu_matches().first().copied().unwrap_or(0));},
                        KeyCode::Char(ch) if key.modifiers.difference(KeyModifiers::SHIFT).is_empty()=>{self.menu_query.push(ch);self.menu=Some(self.menu_matches().first().copied().unwrap_or(0));},
                        KeyCode::Enter => {
                            if !self.menu_matches().contains(&at){return true;}
                            self.menu = None;
                            if let Some((file,block,_))=self.block_menu_target.take(){
                                let controls=match &self.workspace.file {Some(FileView::Merge(m)) if file==self.workspace.current=>m.controls_for(block),_=>{self.workspace.notice=Some("The file changed; open the menu again".into());return true;}};
                                match controls.get(at) {
                                    Some(&control)=>{if !self.guard_preview_edits() && let Some(FileView::Merge(m))=&mut self.workspace.file {m.control(control);}}
                                    None=>self.open_palette(),
                                }
                                self.quit=self.workspace.quit;return true;
                            }
                            if let Some((index,path))=self.files_menu_target.take(){
                                if self.workspace.rows.get(index).is_none_or(|row|row.0!=path){self.workspace.notice=Some("File list changed; open the menu again".into());return true;}
                                if at==0 {self.open_index(index);}
                                else if at==1 {self.workspace.notice=Some(match crate::clipboard::copy(&path){Ok(())=>"Path copied (terminal OSC 52 clipboard)".into(),Err(error)=>format!("Copy failed: {error}")});}
                                else if let Some(action)=ListAct::ALL.get(at-2).copied(){
                                    if matches!(action,ListAct::Refresh|ListAct::Close){self.list_command(action);}
                                    else {self.open_index(index);if self.workspace.busy(){self.pending_files_action=Some((index,path,action));}else{self.list_command(action);}}
                                }
                                self.quit=self.workspace.quit;return true;
                            }
                        }
                        _ => {}
                    }
                    self.quit = self.workspace.quit;
                    return true;
                }
                if self.preview_dirty() && matches!(self.workspace.file, Some(FileView::Picker(_))) && !ctrl && key.code == KeyCode::Char('b') {
                    self.guard_preview_edits();
                    return true;
                }
                if (self.preview_dirty() || !matches!(self.workspace.file, Some(FileView::Picker(_))))
                    && let Some(app) = &mut self.diff
                {
                    let reserved = (!ctrl
                        && matches!(
                            key.code,
                            KeyCode::Char('q' | '[' | ']' | 'M' | 'R' | 'o' | ':')
                        ))
                        || (ctrl && matches!(key.code, KeyCode::Char('q' | 'c' | 'r')));
                    if app.prompting() || !reserved {
                        return app.handle(Event::Key(key));
                    }
                }

                if let Some(FileView::Merge(m)) = &mut self.workspace.file
                    && m.ask.is_none()
                    && m.find_editing()
                {
                    match key.code {
                        KeyCode::Esc => {
                            m.dismiss_find();
                        }
                        KeyCode::Enter => m.finish_find(),
                        KeyCode::Backspace => m.backspace_find(),
                        KeyCode::Tab => m.cycle_find_pane(),
                        KeyCode::F(3) => {
                            m.find_step(!shift);
                        }
                        KeyCode::Char('a') if ctrl => m.clear_find_query(),
                        KeyCode::Char(c @ ('c' | 'w' | 'r')) if alt && !ctrl => { m.toggle_find_option(match c { 'c' => lenz::find::FindOption::Case, 'w' => lenz::find::FindOption::Word, _ => lenz::find::FindOption::Regex }); }
                        KeyCode::Char(c) if !ctrl && !alt => m.type_find(&c.to_string()),
                        _ => return false,
                    }
                    return true;
                }

                if matches!(self.workspace.file, Some(FileView::Picker(_))) {
                    if let Some(app) = &mut self.diff
                        && app.prompting()
                    {
                        return app.handle(Event::Key(key));
                    }

                    if !ctrl && key.code == KeyCode::Char('b') {
                        if self.guard_preview_edits() { return true; }
                        self.preview_base = !self.preview_base;
                        self.poll();
                        return true;
                    }
                    let reserved = (!ctrl
                        && matches!(
                            key.code,
                            KeyCode::Char('1'..='9')
                                | KeyCode::Esc
                                | KeyCode::Char(
                                    'q' | ':' | '[' | ']' | 'M' | 'R' | 'c' | 'C' | 'r'
                                )
                        ))
                        || (ctrl
                            && matches!(
                                key.code,
                                KeyCode::Enter
                                    | KeyCode::Char('s' | 'z' | 'Z' | 'y' | 'q' | 'c' | 'r' | 'R')
                            ))
                        || (!ctrl
                            && key.code == KeyCode::Char('m')
                            && matches!(&self.workspace.file,Some(FileView::Picker(p)) if p.o.large_text));
                    if !reserved && let Some(app) = &mut self.diff {
                        let mapped = if key.code == KeyCode::Char('!') {
                            crossterm::event::KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE)
                        } else {
                            key
                        };
                        return app.handle(Event::Key(mapped));
                    }
                }
                match (key.code, ctrl, shift, alt) {
                    (KeyCode::Char('/'), false, _, _) | (KeyCode::Char('f'), true, false, _) => {
                        if let Some(FileView::Merge(m)) = &mut self.workspace.file {
                            m.begin_find(m.find_options());
                        }
                    }
                    (KeyCode::F(3), _, _, _) => {
                        if let Some(FileView::Merge(m)) = &mut self.workspace.file {
                            m.find_step(!shift);
                        }
                    }
                    (KeyCode::Tab | KeyCode::BackTab | KeyCode::F(6), _, _, _) => {
                        if let Some(FileView::Merge(m)) = &mut self.workspace.file {
                            use lenz::navigator::{InputPane as P, cycle_pane};
                            let panes = [(self.files_area.width>0).then_some(P::Files),Some(P::Left), Some(P::Middle), Some(P::Right)];
                            let current = if self.files_focused{P::Files}else{[P::Left, P::Middle, P::Right][m.find_pane()]};
                            let target = cycle_pane(current, &panes, shift || key.code == KeyCode::BackTab).unwrap();
                            self.files_focused=target==P::Files;
                            if !self.files_focused{m.focus_pane(match target {
                                P::Left => 0, P::Middle => 1, P::Right => 2,
                                P::Sources | P::Log | P::Files | P::Results => unreachable!("merge text pane cycle"),
                            });}
                        }
                    }
                    (KeyCode::Char(':'), _, _, _) => self.open_palette(),
                    (KeyCode::Char('q' | 'c'), true, _, _) | (KeyCode::Char('q'), false, _, _) => {
                        self.close()
                    }
                    (KeyCode::Esc, _, _, _) if self.workspace.dismiss() => {}
                    (KeyCode::Esc, _, _, _) => match self.workspace.file {
                        Some(FileView::Picker(_)) => self.pick(PickAct::Cancel),
                        Some(FileView::Merge(_)) => self.action(Act::Cancel),
                        _ => self.close(),
                    },
                    (KeyCode::Char('['), false, _, _) => self.switch_file(false, false),
                    (KeyCode::Char(']'), false, _, _) => self.switch_file(true, false),
                    (KeyCode::Char('c'), false, _, _) => self.copy_command(0),
                    (KeyCode::Char('C'), false, _, _) => self.copy_command(1),
                    (KeyCode::Char('o'), false, _, _) => self.open_current(),
                    (KeyCode::Char('r'), true, false, _) if self.workspace.root.is_some() => {
                        self.leave_to_repository()
                    }
                    (KeyCode::Char('M'), false, _, _) => self.mark_current(),
                    (KeyCode::Char('R'), false, _, _) => self.reopen_current(),
                    (KeyCode::Char('r' | 'R'), true, true, _) => self.action(Act::StartOver),
                    (KeyCode::Char('r'), false, _, _) if matches!(&self.workspace.file, Some(FileView::Picker(p)) if matches!(p.ask, Some(lenz::merge_session::Ask::ChangedOnDisk | lenz::merge_session::Ask::Overwrite { .. }))) =>
                    {
                        self.pick(PickAct::Reload);
                    }
                    (KeyCode::Char('r'), false, _, _) => {
                        let reload = matches!(&self.workspace.file,Some(FileView::Merge(m)) if matches!(m.ask,Some(lenz::merge_session::Ask::ChangedOnDisk|lenz::merge_session::Ask::Overwrite{..})));
                        if reload || self.workspace.root.is_none() {
                            self.action(Act::Reload);
                        } else {
                            self.refresh();
                        }
                    }
                    (KeyCode::Up, false, _, false) => self.scroll(-1.0),
                    (KeyCode::Down, false, _, false) => self.scroll(1.0),
                    (KeyCode::PageUp, _, _, _) => {
                        self.scroll(-(self.height.saturating_sub(6) as f32))
                    }
                    (KeyCode::PageDown, _, _, _) => {
                        self.scroll(self.height.saturating_sub(6) as f32)
                    }
                    (KeyCode::Left, false, _, false)|(KeyCode::Left,true,false,true)=>{let pane=match &self.workspace.file{Some(FileView::Merge(m))=>m.find_pane(),_=>0};self.scroll_horizontal(pane,-4);}
                    (KeyCode::Right, false, _, false)|(KeyCode::Right,true,false,true)=>{let pane=match &self.workspace.file{Some(FileView::Merge(m))=>m.find_pane(),_=>0};self.scroll_horizontal(pane,4);}
                    (KeyCode::Right, false, true, true) => self.control(1),
                    (KeyCode::Left, false, true, true) => self.control(2),
                    (KeyCode::Down, false, true, true) => self.control(3),
                    (KeyCode::Backspace, false, true, true) => self.control(4),
                    (KeyCode::Char(d), false, _, false) if d.is_ascii_digit() && d != '0' => {
                        if let Some(FileView::Picker(p)) = &mut self.workspace.file {
                            p.take(d as usize - '1' as usize);
                        } else {
                            self.control(d as u32 - '0' as u32);
                        }
                    }
                    (KeyCode::Char('s'), true, _, _) => {
                        if matches!(self.workspace.file, Some(FileView::Picker(_))) {
                            self.pick(PickAct::Save);
                        } else {
                            self.action(Act::Save);
                        }
                    }
                    (KeyCode::Enter, true, _, _) => {
                        if matches!(self.workspace.file, Some(FileView::Picker(_))) {
                            self.pick(PickAct::Done);
                        } else {
                            self.action(Act::Done);
                        }
                    }
                    (KeyCode::Char('z'), true, false, _) => {
                        if matches!(self.workspace.file, Some(FileView::Picker(_))) {
                            self.pick(PickAct::Undo);
                        } else {
                            self.action(Act::Undo);
                        }
                    }
                    (KeyCode::Char('y'), true, _, _)
                    | (KeyCode::Char('z' | 'Z'), true, true, _) => {
                        if matches!(self.workspace.file, Some(FileView::Picker(_))) {
                            self.pick(PickAct::Redo);
                        } else {
                            self.action(Act::Redo);
                        }
                    }
                    (KeyCode::Char('n'), false, false, _) | (KeyCode::F(7), false, false, _) => {
                        self.action(Act::NextChange)
                    }
                    (KeyCode::Char('p'), false, false, _) | (KeyCode::F(7), false, true, _) => {
                        self.action(Act::PrevChange)
                    }
                    (KeyCode::Char('N'), false, _, _) | (KeyCode::Char('n'), false, true, _) => {
                        self.action(Act::NextConflict)
                    }
                    (KeyCode::Char('P'), false, _, _) | (KeyCode::Char('p'), false, true, _) => {
                        self.action(Act::PrevConflict)
                    }
                    (KeyCode::Char('b'), false, _, _) => self.action(Act::ToggleBase),
                    (KeyCode::Char('w'), false, _, _) => self.action(Act::Whitespace),
                    (KeyCode::Char('s'), false, _, _) => self.action(Act::Scroll),
                    (KeyCode::Char('a'), false, _, _) => self.action(Act::ApplyNonConflicting),
                    (KeyCode::Char('m'), false, _, _)
                        if matches!(self.workspace.file, Some(FileView::Picker(_))) =>
                    {
                        self.merge_anyway()
                    }
                    (KeyCode::Char('m'), false, _, _) => self.action(Act::ResolveSimple),
                    _ => {
                        if let Some(app) = &mut self.diff {
                            return app.handle(Event::Key(key));
                        }
                        return false;
                    }
                }
                self.quit = self.workspace.quit;
                true
            }
            _ => false,
        }
    }
    pub fn leave_to_repository(&mut self) {
        if self.guard_preview_edits() { return; }
        self.close();
        if self.workspace.quit {
            self.workspace.quit = false;
            self.quit = false;
            self.back = true;
        }
    }
    pub(crate) fn about_logo(&self)->Rect {
        self.about.as_ref().or_else(||self.diff.as_ref().and_then(|app|app.about.as_ref())).map_or(Rect::default(),|about|about.logo_rect)
    }
    pub fn render(&mut self, frame: &mut Frame<'_>) {
        let palette=self.palette.clone();let palette=&*palette;
        if let Some(app)=&mut self.diff{app.palette=self.palette.clone();app.theme_revision=self.theme_revision;}
        let full = frame.area();
        self.resizer.grips.clear();
        frame.buffer_mut().set_style(full,crate::chrome::style_with(palette));
        let nested=(self.preview_dirty()||!matches!(self.workspace.file,Some(FileView::Picker(_))))&&self.diff.is_some();
        let items=self.toolbar_items(full.width);
        let area=if nested{full}else{self.toolbar.draw_with_theme(frame,full,items,palette,self.theme_revision)};
        self.width = area.width;
        self.height = area.height;
        if nested
            && let Some(app) = &mut self.diff
        {
            app.render_in(frame,Rect::new(area.x,area.y+u16::from(area.height>0),area.width,area.height.saturating_sub(2)));
            let head = self
                .workspace
                .snapshot
                .as_ref()
                .map(|s| {
                    lenz::conflict_session::entry_text(
                        &s.op,
                        s.files.len(),
                        s.files.len() + s.marked.len(),
                    )
                    .join(" · ")
                })
                .unwrap_or_default();
            frame.render_widget(
                Paragraph::new(clean(&head)).style(crate::chrome::style_with(palette).fg(crate::chrome::color(palette.accent))),
                Rect::new(area.x, area.y, area.width, 1),
            );
            let foot = format!(
                "{} · [/] files · o opens merge · M stage · R reopens · F4 comparison commands · Ctrl-r repository · q quits",
                self.workspace
                    .notice
                    .as_deref()
                    .unwrap_or("Saved file against HEAD")
            );
            frame.render_widget(
                Paragraph::new(clean(&foot)),
                Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
            );
            self.render_menu(palette,frame, full);
            if let Some(p)=&mut self.command_palette{let rect=crate::palette::rect(full,p);crate::palette::paint(p,frame,rect,palette);}
            if let Some(scroll)=&mut self.help{crate::help::draw(frame,full,crate::help::MERGE,scroll,palette);}
        if let Some(about)=&mut self.about{about.draw(frame,full,palette);}
        if let Some(message)=&mut self.message{message.draw(frame,full,palette);}
            return;
        }

        let bands = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Min(1),
                Constraint::Length(3),
            ])
            .split(area);
        let header = if let Some(snap) = &self.workspace.snapshot {
            lenz::conflict_session::entry_text(
                &snap.op,
                snap.files.len(),
                snap.files.len() + snap.marked.len(),
            )
            .join(" · ")
        } else {
            "Merge — 1 Ours · 2 Theirs · 3 Both · 4 Base · 5 Reopen".into()
        };
        frame.render_widget(
            Paragraph::new(clean(&header)).style(crate::chrome::style_with(palette).fg(crate::chrome::color(palette.accent))),
            bands[0],
        );
        let file_changed=self.last_file!=Some(self.workspace.current);
        if file_changed{self.horizontal=[0;3];self.columns=[None;3];self.horizontal_version=None;}
        self.last_file=Some(self.workspace.current);
        let mut body = bands[1];self.files_area=Rect::default();
        if !self.workspace.rows.is_empty() && area.width >= 95 {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(crate::panes::size(body.width,self.files_width.unwrap_or(26.0),18.0,45.0)), Constraint::Min(1)])
                .split(body);
            body = split[1];
            self.files_area=split[0];let files_content=self.files_content_area();let visible=usize::from(files_content.height);
            if file_changed&&!self.files_search.active(){self.file_scroll=self.workspace.current.saturating_sub(visible/2);}
            self.file_scroll=self.file_scroll.min(self.workspace.rows.len().saturating_sub(visible));let start=self.file_scroll;
            let lines: Vec<_> = self
                .workspace
                .rows
                .iter()
                .enumerate()
                .skip(start)
                .take(files_content.height as usize)
                .map(|(i, (p, s))| {
                    let tag = match self.workspace.row_state(p, *s) {
                        State::Unresolved(Some(n)) => format!("●{n}"),
                        State::Unresolved(None) => "●".into(),
                        State::Clean => "○".into(),
                        State::Marked => "✓".into(),
                    };
                    crate::pane_search::line(
                        clean(&format!(
                            "{} {tag} {}",
                            if i == self.files_search.current().unwrap_or(self.workspace.current) {
                                ">"
                            } else {
                                " "
                            },lenz::file_icons::file(p).label(self.icons.nerd(),p)
                        )),
                        &self.files_search,
                        crate::chrome::style_with(palette).fg(if i == self.files_search.current().unwrap_or(self.workspace.current) {
                            crate::chrome::color(palette.accent)
                        } else {
                            crate::chrome::color(palette.fg)
                        }),palette,
                    )
                })
                .collect();
            frame.render_widget(
                Paragraph::new(lines).block(Block::default().borders(Borders::RIGHT)),
                files_content,
            );
            if self.icons.nerd() {
                for (row,(path,state)) in self.workspace.rows.iter().skip(start).take(files_content.height as usize).enumerate() {
                    let tag_width=match self.workspace.row_state(path,*state){State::Unresolved(Some(n))=>1+n.to_string().len() as u16,_=>1};
                    let x=files_content.x+3+tag_width;
                    if x<files_content.right().saturating_sub(1) {
                        let icon=lenz::file_icons::file(path);
                        frame.buffer_mut()[(x,files_content.y+row as u16)].set_fg(crate::chrome::color(icon.color_on(palette,palette.bg)));
                    }
                }
            }
        }
        if self.files_search.active()&&self.files_area.height>0 {frame.render_widget(Paragraph::new(crate::pane_search::status(&self.files_search,self.workspace.rows.len())).style(crate::chrome::style_with(palette).fg(crate::chrome::color(palette.accent))),Rect::new(self.files_area.x,self.files_area.y,self.files_area.width.saturating_sub(1),1));}
        self.text_area=body;
        if self.files_area.width>0 {self.resizer.grips.push(crate::panes::Grip::vertical(Rect::new(self.files_area.right()-1,self.files_area.y,1,self.files_area.height),0,self.files_area.x));}
        let merge_panes=self.merge_panes();
        if matches!(&self.workspace.file,Some(FileView::Merge(_)))&&body.width>=45 {
            for (index,pane) in merge_panes.iter().take(2).enumerate() {if pane.width>0 {self.resizer.grips.push(crate::panes::Grip::vertical(Rect::new(pane.right()-1,pane.y,1,pane.height),index as u8+1,body.x));}}
        }
        for pane in 0..3{self.clamp_horizontal(pane);}if self.files_area.width==0{self.files_focused=false;}
        let mut question = None;
        match &mut self.workspace.file {
            Some(FileView::Merge(m)) => {
                m.set_viewport_rows(body.height.saturating_sub(2) as f32);
                let (rows, _) = m.layout();
                let heads = m.headers();
                let panes = merge_panes;
                for p in 0..3 {
                    if panes[p].width==0||panes[p].height==0 {continue;}
                    let text = match p {
                        0 => m.m.side(Some(Pick::Ours)),
                        1 if m.base_shown => m.m.side(None),
                        1 => m.m.lines(),
                        _ => m.m.side(Some(Pick::Theirs)),
                    };
                    let mut lines = vec![Line::styled(
                        clean(&lenz::file_icons::file(&heads[p]).label(self.icons.nerd(),&heads[p])),
                        crate::chrome::style_with(palette).bg(crate::chrome::color(palette.header_bg)).fg(crate::chrome::color(palette.accent)).add_modifier(ratatui::style::Modifier::BOLD),
                    )];
                    for r in &rows[p] {
                        if r.y < 0.0 || r.y >= body.height.saturating_sub(2) as f32 {
                            continue;
                        }
                        match r.content {
                            Content::Line(n) => {
                                let block = lenz::merge_session::owner(
                                    m.m.blocks().len(),
                                    n,
                                    |i| match p {
                                        0 => m.m.blocks()[i].left.clone(),
                                        1 if m.base_shown => m.m.blocks()[i].base.clone(),
                                        1 => m.m.result_range(i),
                                        _ => m.m.blocks()[i].right.clone(),
                                    },
                                );
                                let tint=block.and_then(|i|m.tint(p,i));
                                let bg=tint.map_or(palette.bg,|(kind,ignored)|lenz::theme::change_color(palette,kind,ignored,&palette.block));
                                let style=crate::chrome::style_with(palette).bg(crate::chrome::color(bg));
                                let prefix = if block == m.current && block.is_some() {
                                    ">"
                                } else {
                                    " "
                                };
                                let syntax = m
                                    .parsed_pane(p)
                                    .and_then(|parsed| {
                                        parsed.spans(n as usize..n as usize + 1).pop()
                                    })
                                    .unwrap_or_default();
                                let mut spans = vec![Span::styled(
                                    format!("{prefix}{:>4} ", n + 1),
                                    crate::chrome::style_with(palette).fg(crate::chrome::color(palette.dim)),
                                )];
                                spans.extend(styled_source(
                                    palette,
                                    text.get(n as usize).map(String::as_str).unwrap_or(""),
                                    self.horizontal[p],
                                    panes[p].width.saturating_sub(7) as usize,
                                    style,
                                    &syntax,
                                    &r.inline,
                                    tint.map(|(kind,ignored)|lenz::theme::change_color(palette,kind,ignored,&palette.inline)),
                                    &m.find_ranges(p, n),
                                ));
                                lines.push(Line::from(spans));
                            }
                            Content::Filler => lines.push(Line::from("")),
                            Content::Fold { .. } => lines.push(Line::from("…")),
                            Content::Folded { .. } => lines.push(Line::from("Folded change")),
                        }
                    }
                    frame.render_widget(
                        Paragraph::new(lines).block(Block::default().borders(Borders::RIGHT)),
                        panes[p],
                    );
                    if self.icons.nerd() && panes[p].width>1 {
                        let icon=lenz::file_icons::file(&heads[p]);
                        frame.buffer_mut()[(panes[p].x,panes[p].y)].set_fg(crate::chrome::color(icon.color_on(palette,palette.header_bg)));
                    }
                }
                question = m
                    .question()
                    .map(str::to_owned)
                    .or_else(|| m.error.clone())
                    .or_else(|| m.find_notice());
                frame.render_widget(
                    Paragraph::new(m.counters()),
                    Rect::new(bands[0].x, bands[0].y + 1, bands[0].width, 1),
                );
            }
            Some(FileView::Picker(p)) => {
                let mut lines = vec![Line::styled(
                    clean(&format!(
                        "{} is {}: take one version whole",
                        p.o.path, p.o.why
                    )),
                    crate::chrome::style_with(palette).fg(crate::chrome::color(palette.status[2])),
                )];
                for (i, v) in p.o.versions.iter().enumerate() {
                    lines.push(Line::from(clean(&format!(
                        "{} {} {} — {}",
                        if p.chosen == Some(i) { "✓" } else { " " },
                        i + 1,
                        v.label,
                        v.error.as_deref().unwrap_or(&v.note)
                    ))));
                }
                lines.extend(
                    p.o.info
                        .iter()
                        .take(body.height as usize)
                        .map(|s| Line::from(clean(s))),
                );
                if p.o.large_text {
                    lines.push(Line::from("m: Merge anyway"));
                }
                let height = crate::panes::size(body.height,self.version_height.unwrap_or(lines.len()as f32+1.0),2.0,4.0);
                let band = Rect::new(body.x,body.bottom().saturating_sub(height),body.width,height);
                if let Some(app) = &mut self.diff {
                    app.render_in(
                        frame,
                        Rect::new(body.x,body.y,body.width,band.y.saturating_sub(body.y)),
                    );
                }
                frame.render_widget(Clear, band);
                frame.render_widget(Paragraph::new(lines), Rect::new(band.x,band.y+u16::from(band.height>0),band.width,band.height.saturating_sub(1)));
                if band.height>0 {self.resizer.grips.push(crate::panes::Grip::horizontal(Rect::new(band.x,band.y,band.width,1),3,body.y));}
                question = p.question().map(str::to_owned);
            }
            Some(FileView::Marked { path, text }) => {
                let lines: Vec<_> = std::iter::once(Line::from(clean(&format!(
                    "{path} — marked resolved; R reopens"
                ))))
                .chain(
                    text.lines()
                        .take(body.height as usize)
                        .map(|s| Line::from(clipped(s, self.horizontal[0], body.width as usize))),
                )
                .collect();
                frame.render_widget(Paragraph::new(lines), body);
            }
            None => {
                frame.render_widget(
                    Paragraph::new(if self.busy() {
                        "Loading merge…"
                    } else {
                        "No file open"
                    }),
                    body,
                );
            }
        }
        let footer = [question.map(|text|format!("F2 details · {text}")).unwrap_or_default(),format!("F2 details · {}",self.workspace.notice.clone().unwrap_or_else(||match &self.workspace.file {Some(FileView::Merge(m))=>m.notices().join(" · "),_=>String::new()})),"1–5 choose · n/p changes · N/P conflicts · [/] files · Ctrl-s Save · Ctrl-Enter Done · M stage · R reopen · c/C copy commands · : merge commands · F4 comparison commands · q quit".into()];
        frame.render_widget(
            Paragraph::new(
                footer
                    .iter()
                    .map(|s| Line::from(clean(s)))
                    .collect::<Vec<_>>(),
            ),
            bands[2],
        );
        self.resizer.draw(frame.buffer_mut(),palette);
        if let Some(app)=&mut self.diff{app.popup_area=full;crate::render::draw_popup(app,frame);crate::palette::draw(app,frame);app.draw_context(frame);}
        self.toolbar.draw_menu_with_theme(frame,palette);
        self.render_menu(palette,frame, full);
        if let Some(p)=&mut self.command_palette{let rect=crate::palette::rect(full,p);crate::palette::paint(p,frame,rect,palette);}
        if let Some(scroll)=&mut self.help{crate::help::draw(frame,full,crate::help::MERGE,scroll,palette);}
        if let Some(about)=&mut self.about{about.draw(frame,full,palette);}
        if let Some(message)=&mut self.message{message.draw(frame,full,palette);}
    }
    fn render_menu(&mut self,palette:&lenz::theme::Palette,frame:&mut Frame<'_>,area:Rect){
        let Some(at)=self.menu else{return;};
        if area.width<3||area.height<3{self.menu_area=Rect::default();return;}
        let labels=self.menu_labels();let matches=self.menu_matches();let selected=matches.iter().position(|&i|i==at).unwrap_or(0);
        let width=area.width.min(78);let height=area.height.min(matches.len().max(1)as u16+2);let visible=usize::from(height.saturating_sub(2));
        self.menu_first=selected.saturating_sub(visible.saturating_sub(1));
        let rect=Rect::new(area.x+(area.width-width)/2,area.y+(area.height-height)/2,width,height);self.menu_area=rect;
        let mut lines:Vec<_>=matches.iter().skip(self.menu_first).take(visible).map(|&index|{let(label,reason)=&labels[index];Line::from(format!("{} {}{}",if index==at{">"}else{" "},label,reason.as_ref().map(|r|format!(" — {r}")).unwrap_or_default()))}).collect();
        if lines.is_empty(){lines.push(Line::from("No matching commands"));}
        let title=if self.block_menu_target.is_some(){format!("Change · {}",self.menu_query)}else if let Some((_,path))=&self.files_menu_target{format!("Files: {} · {}",clean(path),self.menu_query)}else{format!("Merge commands: {}",if self.menu_query.is_empty(){"type to filter"}else{&self.menu_query})};
        frame.render_widget(Clear,rect);frame.render_widget(Paragraph::new(lines).style(crate::chrome::style_with(palette).bg(crate::chrome::color(palette.status_bg))).block(Block::default().borders(Borders::ALL).title(format!("F2 details · {title}"))),rect);
    }

}
impl Merger {
    /// What the merge view shows now, for the catalog: the merge, or the Version Picker with its comparison.
    fn scope(&self) -> lenz::catalog::Scope {
        use lenz::catalog::{Scope, Surface};
        match &self.workspace.file {
            Some(FileView::Picker(_)) => Scope { surface: Surface::VersionPicker, repository: false, pr: false, ..self.diff.as_ref().map_or_else(Scope::comparison, crate::app::App::scope) },
            Some(FileView::Merge(m)) => Scope { surface: Surface::Merge, find: m.find_editing().then(|| m.find_options()), ..Scope::comparison() },
            _ => Scope { surface: Surface::Merge, ..Scope::comparison() },
        }
    }
    fn open_palette(&mut self) {
        self.toolbar.close();
        self.menu = None;
        self.files_menu_target = None;
        self.block_menu_target = None;
        self.command_palette = Some(lenz::palette::Palette::new(lenz::catalog::entries(&self.scope()), &*self));
    }
    /// Run a catalog action: merge and version choices here, the Version Picker's comparison commands in its preview.
    fn run_action(&mut self, action: lenz::catalog::Action) {
        use lenz::catalog::{Action, HelpAction};
        use lenz::commands::Interaction as I;
        self.command_palette = None;
        match action {
            Action::Merge(act) => self.action(act),
            Action::Pick(act) => self.pick(act),
            // Merge find's options (the comparison's own find goes to the diff below).
            Action::Interaction(toggle @ (I::FindCase | I::FindWord | I::FindRegex)) if matches!(&self.workspace.file, Some(FileView::Merge(_))) => {
                if let (Some(FileView::Merge(m)), Some(option)) = (&mut self.workspace.file, toggle.find_option()) {
                    if m.find_editing() {
                        m.toggle_find_option(option);
                    } else {
                        self.workspace.notice = Some("No find is open".into());
                    }
                }
            }
            Action::Interaction(I::ChangeTheme) => self.request_theme(),
            Action::Interaction(I::About) => self.about = Some(Default::default()),
            Action::Interaction(I::ResetPaneSizes) => self.reset_pane_sizes(),
            Action::Interaction(I::ReportLastError) => match self.report_request.start() {
                Ok(_) => self.workspace.notice = Some("Preparing issue draft; review the local report before sharing".into()),
                Err(error) => self.workspace.notice = Some(format!("Cannot prepare report: {error}")),
            },
            Action::Help(HelpAction::Shortcuts) => self.help = Some(0),
            Action::Help(HelpAction::Settings) => self.workspace.notice = Some(crate::palette::open_settings()),
            Action::Comparison(_) | Action::Interaction(_) | Action::Edit(_) | Action::File(_) | Action::Image(_) | Action::Layout(_)
            | Action::Help(HelpAction::DiffOptions) | Action::Repository(_) | Action::Pr(_) | Action::Folder(_) => match &mut self.diff {
                Some(app) => app.run_action(action),
                None => self.workspace.notice = Some("No comparison here".into()),
            },
        }
    }
}

impl lenz::catalog::Availability for Merger {
    fn disabled(&self, action: lenz::catalog::Action) -> Option<std::borrow::Cow<'static, str>> {
        use lenz::catalog::{Action, HelpAction};
        use lenz::commands::Interaction as I;
        use std::borrow::Cow;
        match action {
            Action::Merge(act) => match &self.workspace.file { Some(FileView::Merge(m)) => m.disabled(act).map(Cow::Borrowed), _ => Some(Cow::Borrowed("Open a conflicted file first")) },
            Action::Pick(act) => match &self.workspace.file { Some(FileView::Picker(p)) => p.disabled(act).map(Cow::Borrowed), _ => Some(Cow::Borrowed("Only in the Version Picker")) },
            Action::Interaction(I::ChangeTheme | I::About | I::ResetPaneSizes | I::ReportLastError) | Action::Help(HelpAction::Shortcuts | HelpAction::Settings) => None,
            Action::Interaction(I::FindCase | I::FindWord | I::FindRegex) if let Some(FileView::Merge(m)) = &self.workspace.file => (!m.find_editing()).then_some(Cow::Borrowed("No find is open")),
            Action::Comparison(_) | Action::Interaction(_) | Action::Edit(_) | Action::File(_) | Action::Image(_) | Action::Layout(_)
            | Action::Help(HelpAction::DiffOptions) | Action::Repository(_) | Action::Pr(_) | Action::Folder(_) =>
                self.diff.as_ref().map_or(Some(Cow::Borrowed("No comparison here")), |app| lenz::catalog::Availability::disabled(app, action)),
        }
    }
}

fn clean(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c == '\t' {
                ' '
            } else if lenz::document::hides_text(c) {
                '�'
            } else {
                c
            }
        })
        .collect()
}
#[allow(clippy::too_many_arguments)] // Palette plus existing cell/decorations inputs.
fn styled_source(
    palette:&lenz::theme::Palette,
    s: &str,
    offset: usize,
    width: usize,
    base: Style,
    syntax: &[(std::ops::Range<usize>, lenz::highlight::Capture)],
    inline: &[std::ops::Range<usize>],
    inline_color: Option<u32>,
    hits: &[std::ops::Range<usize>],
) -> Vec<Span<'static>> {
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;
    let mut x = 0;
    let mut out = Vec::new();
    for (byte, g) in s
        .trim_end_matches(['\r', '\n'])
        .grapheme_indices(true)
        .take(lenz::document::LONG_LINE)
    {
        let display = if g=="\t"{"    ".into()}else{clean(g)};
        let w = display.width().max(1);
        if x + w > offset + width {
            break;
        }
        if x >= offset {
            let mut style = base;
            let at = syntax.partition_point(|(r, _)| r.end <= byte);
            if let Some((r, capture)) = syntax.get(at)
                && r.start < byte + g.len()
            {
                style = style.patch(crate::visual::syntax_style(palette,*capture));
            }

            if let Some(bg)=inline_color && inline
                .get(inline.partition_point(|r| r.end <= byte))
                .is_some_and(|r| r.start < byte + g.len())
            {
                style = style.bg(crate::chrome::color(bg)).fg(crate::chrome::color(palette.fg));
            }
            if hits
                .get(hits.partition_point(|r| r.end <= byte))
                .is_some_and(|r| r.start < byte + g.len())
            {
                style = style.bg(crate::chrome::color(palette.found_bg)).fg(crate::chrome::color(palette.fg));
            }
            out.push(Span::styled(display, style));
        }
        x += w;
    }
    out
}
fn clipped(s: &str, offset: usize, width: usize) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;
    let mut x = 0;
    let mut out = String::new();
    for g in s
        .trim_end_matches(['\r', '\n'])
        .graphemes(true)
        .take(lenz::document::LONG_LINE)
    {
        let g = clean(g);
        let w = g.width();
        if x + w > offset + width {
            break;
        }
        if x >= offset {
            out.push_str(&g);
        }
        x += w;
    }
    out
}

impl Merger {
    fn horizontal_max(&mut self,pane:usize)->usize{
        let Some(FileView::Merge(m))=&self.workspace.file else{return 0;};
        let version=(m.version,m.base_shown);
        if self.horizontal_version!=Some(version){self.columns=[None;3];self.horizontal_version=Some(version);}
        let lines=match pane{0=>m.m.side(Some(Pick::Ours)),1=>m.mid_lines(),_=>m.m.side(Some(Pick::Theirs))};
        let widest=*self.columns[pane].get_or_insert_with(||crate::render::source_width(lines));
        let panes=self.merge_panes();
        widest.saturating_sub(usize::from(panes[pane].width.saturating_sub(7)))
    }
    fn clamp_horizontal(&mut self,pane:usize){if self.horizontal[pane]==0{return;}let max=self.horizontal_max(pane);self.horizontal[pane]=self.horizontal[pane].min(max);}
    fn scroll_horizontal(&mut self,pane:usize,delta:isize){
        if delta<=0&&self.horizontal[pane]==0{return;}
        let max=self.horizontal_max(pane);self.horizontal[pane]=self.horizontal[pane].saturating_add_signed(delta).min(max);
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend, style::Modifier};
    fn key(app: &mut Merger, code: KeyCode, mods: KeyModifiers) {
        app.handle(Event::Key(KeyEvent::new(code, mods)));
    }
    #[test]
    fn merge_source_colors_follow_conflict_and_resolution_state_without_underlining() {
        let (_dir,mut app)=open_fixture([b"let n = 1;\n",b"let n = 2;\n",b"let n = 3;\n"]);
        for theme in lenz::theme::builtin_entries() {
            app.palette=theme.palette;
            key(&mut app,KeyCode::Char('5'),KeyModifiers::NONE);
            for resolved in [false,true] {
                if resolved {key(&mut app,KeyCode::Char('1'),KeyModifiers::NONE);}
                let mut terminal=Terminal::new(TestBackend::new(160,36)).unwrap();terminal.draw(|f|app.render(f)).unwrap();
                let buf=terminal.backend().buffer();let pane=app.merge_panes()[0];let at=(pane.x+14,pane.y+1);
                assert_eq!(buf[at].symbol(),"2");
                let palette=&app.palette;
                assert_eq!(buf[at].bg,crate::chrome::color(if resolved{palette.block[2]}else{palette.inline[3]}),"{} resolved={resolved}",theme.id);
                assert_eq!(buf[at].fg,crate::chrome::color(palette.fg));assert!(!buf[at].modifier.contains(Modifier::UNDERLINED));
                let theirs=(app.merge_panes()[2].x+14,at.1);assert_eq!(buf[theirs].symbol(),"3");
                assert_eq!(buf[theirs].bg,crate::chrome::color(if resolved{lenz::theme::change_color(palette,lenz::view::Kind::Modified,true,&palette.inline)}else{palette.inline[3]}));
            }
        }
    }
    #[test]
    fn tui_recovery_details_merge_and_picker_changed_disk_keep_confirmation_and_bytes() {
        for bytes in [[b"base\n".as_slice(), b"ours\n".as_slice(), b"theirs\n".as_slice()], [b"base\0".as_slice(), b"ours\0".as_slice(), b"theirs\0".as_slice()]] {
            let (dir,mut app)=open_fixture(bytes);
            key(&mut app,KeyCode::Char('1'),KeyModifiers::NONE);
            std::fs::write(dir.path().join("result"),b"external\n").unwrap();
            key(&mut app,KeyCode::Char('s'),KeyModifiers::CONTROL);
            let mut terminal=Terminal::new(TestBackend::new(48,18)).unwrap();
            terminal.draw(|f|app.render(f)).unwrap();
            let visible=terminal.backend().buffer().content().iter().map(|c|c.symbol()).collect::<String>();
            assert!(visible.contains("F2 details"),"{visible}");
            key(&mut app,KeyCode::F(2),KeyModifiers::NONE);
            terminal.draw(|f|app.render(f)).unwrap();
            let details=terminal.backend().buffer().content().iter().map(|c|c.symbol()).collect::<String>();
            let buffer=terminal.backend().buffer();
            let message=(2..15).flat_map(|y|(2..46).map(move |x|buffer[(x,y)].symbol())).collect::<String>();
            assert!(message.chars().filter(|c|!c.is_whitespace()).collect::<String>().contains("reloads"),"{details}");
            let version=match &app.workspace.file {Some(FileView::Merge(m))=>m.text(),Some(FileView::Picker(p))=>format!("{:?}",p.chosen),_=>panic!()};
            for (code,mods) in [(KeyCode::Char('s'),KeyModifiers::CONTROL),(KeyCode::Char('r'),KeyModifiers::NONE),(KeyCode::Char('2'),KeyModifiers::NONE),(KeyCode::Esc,KeyModifiers::NONE)] {key(&mut app,code,mods);}
            assert_eq!(std::fs::read(dir.path().join("result")).unwrap(),b"external\n");
            match &app.workspace.file {
                Some(FileView::Merge(m))=>{assert_eq!(m.text(),version);assert!(m.ask.is_some());assert!(m.dirty);},
                Some(FileView::Picker(p))=>{assert_eq!(format!("{:?}",p.chosen),version);assert!(p.ask.is_some());assert!(p.dirty);},_=>panic!()
            }
        }
    }
    #[test]
    fn merge_disk_question_precedes_existing_find() {
        use lenz::merge_session::Ask;
        for action in [KeyCode::Char('r'), KeyCode::Char('s'), KeyCode::Esc] {
            let (dir, mut app) = open_fixture([b"needle base\n", b"needle ours\n", b"needle theirs\n"]);
            key(&mut app, KeyCode::Char('f'), KeyModifiers::CONTROL);
            key(&mut app, KeyCode::Char('n'), KeyModifiers::NONE);
            let (find, draft) = match &app.workspace.file {
                Some(FileView::Merge(m)) => (m.find_notice(), m.text()),
                _ => panic!("expected merge"),
            };
            let path = dir.path().join("result");
            let external = "needle theirs\nextra\n";
            std::fs::write(&path, external).unwrap();
            app.handle(Event::FocusGained);
            assert!(matches!(&app.workspace.file, Some(FileView::Merge(m)) if m.ask == Some(Ask::ChangedOnDisk)));
            key(&mut app, action, if action == KeyCode::Char('s') { KeyModifiers::CONTROL } else { KeyModifiers::NONE });
            let Some(FileView::Merge(m)) = &app.workspace.file else { panic!("expected merge") };
            assert!(m.find_editing(), "question must receive {action:?} before Find");
            assert_eq!(m.find_notice(), find);
            assert_eq!(m.ask, if action == KeyCode::Char('s') { Some(Ask::Overwrite { done: false }) } else { None });
            assert_eq!(m.text(), if action == KeyCode::Char('r') { external.to_string() } else { draft });
            assert_eq!(std::fs::read_to_string(&path).unwrap(), external);
            if action == KeyCode::Char('s') {
                key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
                assert!(matches!(&app.workspace.file, Some(FileView::Merge(m)) if m.ask.is_none() && m.find_editing()));
                assert_eq!(std::fs::read_to_string(&path).unwrap(), external);
            }
            assert!(!app.quit);
        }
    }
    #[test]
    fn pane_merge_drag_focus_scroll_modal_cancel_and_reset_preserve_result() {
        let base=format!("base {}\n","x".repeat(180));let ours=format!("ours {}\n","x".repeat(180));let theirs=format!("theirs {}\n","x".repeat(180));
        let (dir,mut app)=open_fixture([base.as_bytes(),ours.as_bytes(),theirs.as_bytes()]);screen(&mut app);
        let original=std::fs::read(dir.path().join("result")).unwrap();let panes=app.merge_panes();let y=panes[0].y+1;
        let mouse=|kind,x,y|Event::Mouse(crossterm::event::MouseEvent{kind,column:x,row:y,modifiers:KeyModifiers::NONE});
        app.handle(mouse(MouseEventKind::Down(MouseButton::Left),panes[0].right()-1,y));app.handle(mouse(MouseEventKind::Drag(MouseButton::Left),app.text_area.x+54,0));app.handle(mouse(MouseEventKind::Up(MouseButton::Left),app.text_area.x+54,0));screen(&mut app);
        let panes=app.merge_panes();assert_eq!(panes[0].width,55);assert_eq!(app.pointer_pane(panes[0].right()-5),0);
        app.handle(mouse(MouseEventKind::Down(MouseButton::Left),panes[0].right()-5,y));assert!(matches!(&app.workspace.file,Some(FileView::Merge(m))if m.find_pane()==0));
        app.handle(mouse(MouseEventKind::ScrollRight,panes[0].right()-5,y));assert_eq!(app.horizontal,[4,0,0]);
        let ratios=app.pane_ratios;key(&mut app,KeyCode::Char(':'),KeyModifiers::NONE);app.handle(mouse(MouseEventKind::Down(MouseButton::Left),panes[0].right()-1,y));app.handle(mouse(MouseEventKind::Drag(MouseButton::Left),10,y));assert_eq!(app.pane_ratios,ratios);
        key(&mut app,KeyCode::Esc,KeyModifiers::NONE);app.toolbar_action(ToolbarAction::Run(lenz::catalog::Action::Interaction(lenz::commands::Interaction::ResetPaneSizes)));assert_eq!(app.pane_ratios,[1.0/3.0,2.0/3.0]);
        screen(&mut app);let panes=app.merge_panes();app.handle(mouse(MouseEventKind::Down(MouseButton::Left),panes[1].right()-1,y));app.handle(Event::FocusLost);app.handle(mouse(MouseEventKind::Drag(MouseButton::Left),110,y));assert_eq!(app.pane_ratios,[1.0/3.0,2.0/3.0]);
        for (w,h) in [(1,1),(20,4),(40,10),(160,40)] {let mut t=Terminal::new(TestBackend::new(w,h)).unwrap();app.handle(Event::Resize(w,h));t.draw(|f|app.render(f)).unwrap();}
        assert_eq!(std::fs::read(dir.path().join("result")).unwrap(),original);
    }
    #[test]
    fn reopening_one_resolution_restores_conflict_without_writing_result() {
        let (dir,mut app)=open_fixture([b"base\n",b"ours\n",b"theirs\n"]);screen(&mut app);let saved=std::fs::read(dir.path().join("result")).unwrap();
        key(&mut app,KeyCode::Char('1'),KeyModifiers::NONE);
        assert!(matches!(&app.workspace.file,Some(FileView::Merge(m))if m.m.unresolved()==0&&m.dirty));
        key(&mut app,KeyCode::Char('5'),KeyModifiers::NONE);
        assert!(matches!(&app.workspace.file,Some(FileView::Merge(m))if m.m.unresolved()==1));assert_eq!(std::fs::read(dir.path().join("result")).unwrap(),saved);
        key(&mut app,KeyCode::Char('z'),KeyModifiers::CONTROL);assert!(matches!(&app.workspace.file,Some(FileView::Merge(m))if m.m.unresolved()==0));assert_eq!(std::fs::read(dir.path().join("result")).unwrap(),saved);
    }
    #[test]
    fn merge_shift_tab_moves_backwards_through_visible_text_panes() {
        let (_dir, mut app) = open_fixture([b"base\n", b"ours\n", b"theirs\n"]);
        let pane = |app: &Merger| match &app.workspace.file { Some(FileView::Merge(m)) => m.find_pane(), _ => panic!("expected merge") };
        assert_eq!(pane(&app), 1);
        key(&mut app, KeyCode::BackTab, KeyModifiers::SHIFT);
        assert_eq!(pane(&app), 0);
        key(&mut app, KeyCode::BackTab, KeyModifiers::SHIFT);
        assert_eq!(pane(&app), 2);
        key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(pane(&app), 0);
    }
    #[test]
    fn merge_right_click_menu_and_mouse_focus_never_scroll_underlying_text(){
        let (dir,mut app)=open_fixture([b"base\n",b"ours\n",b"theirs\n"]);screen(&mut app);
        let area=app.text_area;let mouse=|kind,column,row|Event::Mouse(crossterm::event::MouseEvent{kind,column,row,modifiers:KeyModifiers::NONE});
        app.handle(mouse(MouseEventKind::Down(MouseButton::Left),area.x+area.width-1,area.y+1));assert!(matches!(&app.workspace.file,Some(FileView::Merge(m))if m.find_pane()==2));
        app.handle(mouse(MouseEventKind::Down(MouseButton::Right),area.x+1,area.y+1));assert!(app.menu.is_some()&&screen(&mut app).contains("Take Ours"),"the conflict under the pointer");
        let before=match &app.workspace.file{Some(FileView::Merge(m))=>format!("{:?}",m.layout()),_=>panic!()};app.handle(mouse(MouseEventKind::ScrollDown,area.x+1,area.y+1));
        assert!(matches!(&app.workspace.file,Some(FileView::Merge(m))if format!("{:?}",m.layout())==before));key(&mut app,KeyCode::Esc,KeyModifiers::NONE);assert_eq!(std::fs::read(dir.path().join("result")).unwrap(),b"ours\n");
        key(&mut app,KeyCode::Char('1'),KeyModifiers::NONE);key(&mut app,KeyCode::Esc,KeyModifiers::NONE);assert!(matches!(&app.workspace.file,Some(FileView::Merge(m))if m.ask.is_some()));
        app.handle(mouse(MouseEventKind::Down(MouseButton::Right),area.x+1,area.y+1));key(&mut app,KeyCode::F(10),KeyModifiers::SHIFT);assert!(app.menu.is_none());
    }
    #[test]
    fn merge_files_mouse_and_keyboard_stay_in_the_file_pane(){
        let dir=tempfile::tempdir().unwrap();let d=dir.path();git(d,&["init","-q","-b","main"],true);git(d,&["config","user.name","Test"],true);git(d,&["config","user.email","test@example.test"],true);
        for path in ["a.txt","b.txt"]{std::fs::write(d.join(path),"base\n").unwrap();}git(d,&["add","."],true);git(d,&["commit","-qm","base"],true);git(d,&["checkout","-qb","topic"],true);
        for path in ["a.txt","b.txt"]{std::fs::write(d.join(path),"theirs\n").unwrap();}git(d,&["commit","-qam","theirs"],true);git(d,&["checkout","-q","main"],true);
        for path in ["a.txt","b.txt"]{std::fs::write(d.join(path),"ours\n").unwrap();}git(d,&["commit","-qam","ours"],true);git(d,&["merge","-q","topic"],false);
        let mut app=Merger::from_repository(d.into(),Vec::new());settle(&mut app);screen(&mut app);assert!(app.files_area.width>0);
        let mouse=|kind,column,row|Event::Mouse(crossterm::event::MouseEvent{kind,column,row,modifiers:KeyModifiers::NONE});let original=app.files_area;
        app.handle(mouse(MouseEventKind::Down(MouseButton::Left),original.right()-1,original.y+1));app.handle(mouse(MouseEventKind::Drag(MouseButton::Left),34,original.y+1));app.handle(mouse(MouseEventKind::Up(MouseButton::Left),34,original.y+1));screen(&mut app);assert_eq!(app.files_area.width,35);assert_eq!(app.workspace.current,0);
        let area=app.files_area;
        app.files_focused=true;
        let before_path=app.workspace.file.as_ref().unwrap().path().to_string();
        for ch in "qbgynjk:/?".chars(){key(&mut app,KeyCode::Char(ch),KeyModifiers::NONE);assert_eq!(app.workspace.current,0);assert!(!app.quit);}
        assert_eq!(app.files_search.query(),"qbgynjk:/?");assert!(app.menu.is_none()&&app.help.is_none());
        key(&mut app,KeyCode::Esc,KeyModifiers::NONE);key(&mut app,KeyCode::Char('f'),KeyModifiers::CONTROL);app.handle(Event::Paste("B.TXT".into()));
        assert_eq!(app.files_search.current(),Some(1));assert_eq!(app.workspace.current,0);assert_eq!(app.workspace.file.as_ref().unwrap().path(),before_path);
        let previous_area=app.files_area;app.files_area.height=2;let candidate=app.files_search.current();
        key(&mut app,KeyCode::PageDown,KeyModifiers::NONE);assert_eq!(app.file_scroll,1);key(&mut app,KeyCode::PageUp,KeyModifiers::NONE);assert_eq!(app.file_scroll,0);key(&mut app,KeyCode::End,KeyModifiers::NONE);assert_eq!(app.file_scroll,1);key(&mut app,KeyCode::Home,KeyModifiers::NONE);assert_eq!(app.file_scroll,0);assert_eq!(app.files_search.current(),candidate);assert_eq!(app.workspace.current,0);app.files_area=previous_area;
        key(&mut app,KeyCode::F(3),KeyModifiers::NONE);key(&mut app,KeyCode::F(3),KeyModifiers::SHIFT);assert_eq!(app.workspace.current,0);
        key(&mut app,KeyCode::F(10),KeyModifiers::SHIFT);assert_eq!(app.files_menu_target.as_ref().unwrap().0,1);assert_eq!(app.workspace.current,0);assert!(screen(&mut app).contains("Copy file path"));
        key(&mut app,KeyCode::Down,KeyModifiers::NONE);key(&mut app,KeyCode::Enter,KeyModifiers::NONE);assert_eq!(app.workspace.current,0,"copying the candidate does not open it");assert!(!app.workspace.busy());
        key(&mut app,KeyCode::Enter,KeyModifiers::NONE);settle(&mut app);assert_eq!(app.workspace.current,1);assert_eq!(app.files_search.query(),"B.TXT");
        key(&mut app,KeyCode::Char('a'),KeyModifiers::CONTROL);assert_eq!(app.files_search.query(),"");key(&mut app,KeyCode::Esc,KeyModifiers::NONE);assert_eq!(app.workspace.current,1);
        app.workspace.open_index(0);settle(&mut app);screen(&mut app);
        app.handle(mouse(MouseEventKind::Down(MouseButton::Left),area.x+1,area.y+1));settle(&mut app);assert!(app.files_focused);assert_eq!(app.workspace.current,1);
        key(&mut app,KeyCode::Up,KeyModifiers::NONE);settle(&mut app);assert_eq!(app.workspace.current,0);screen(&mut app);assert_eq!(app.files_area.width,35);
        let before=match &app.workspace.file{Some(FileView::Merge(m))=>format!("{:?}",m.layout()),_=>panic!()};app.handle(mouse(MouseEventKind::ScrollDown,area.x+1,area.y));assert!(matches!(&app.workspace.file,Some(FileView::Merge(m))if format!("{:?}",m.layout())==before));
        key(&mut app,KeyCode::BackTab,KeyModifiers::SHIFT);assert!(!app.files_focused);assert!(matches!(&app.workspace.file,Some(FileView::Merge(m))if m.find_pane()==2));key(&mut app,KeyCode::Tab,KeyModifiers::NONE);assert!(app.files_focused);
    }
    #[test]
    fn version_picker_files_and_preview_context_keep_input_in_the_active_pane(){
        let dir=tempfile::tempdir().unwrap();let d=dir.path();git(d,&["init","-q","-b","main"],true);git(d,&["config","user.name","Test"],true);git(d,&["config","user.email","test@example.test"],true);
        for path in ["a.txt","b.txt"]{std::fs::write(d.join(path),"base\n").unwrap();}git(d,&["add","."],true);git(d,&["commit","-qm","base"],true);git(d,&["checkout","-qb","topic"],true);
        git(d,&["rm","a.txt","b.txt"],true);git(d,&["commit","-qm","delete"],true);git(d,&["checkout","-q","main"],true);
        for path in ["a.txt","b.txt"]{std::fs::write(d.join(path),"ours\n").unwrap();}git(d,&["commit","-qam","ours"],true);git(d,&["merge","-q","topic"],false);
        let mut app=Merger::from_repository(d.into(),Vec::new());settle(&mut app);screen(&mut app);assert!(matches!(app.workspace.file,Some(FileView::Picker(_))));assert!(app.diff.is_some());
        let mouse=|kind,column,row|Event::Mouse(crossterm::event::MouseEvent{kind,column,row,modifiers:KeyModifiers::NONE});let files=app.files_area;
        app.handle(mouse(MouseEventKind::Down(MouseButton::Left),files.x+1,files.y+1));settle(&mut app);assert!(app.files_focused);assert_eq!(app.workspace.current,1);
        key(&mut app,KeyCode::Up,KeyModifiers::NONE);settle(&mut app);screen(&mut app);assert_eq!(app.workspace.current,0);
        let before=app.diff.as_ref().unwrap().session.view().frame();app.handle(mouse(MouseEventKind::ScrollDown,files.x+1,files.y));assert_eq!(app.diff.as_ref().unwrap().session.view().frame(),before);
        key(&mut app,KeyCode::BackTab,KeyModifiers::SHIFT);assert!(!app.files_focused);assert_eq!(app.diff.as_ref().unwrap().focus,Side::Right);key(&mut app,KeyCode::Tab,KeyModifiers::NONE);assert!(app.files_focused);
        app.handle(Event::Paste("B.TXT".into()));assert_eq!(app.files_search.current(),Some(1));assert_eq!(app.workspace.current,0);assert!(matches!(app.workspace.file,Some(FileView::Picker(_))));
        key(&mut app,KeyCode::F(10),KeyModifiers::SHIFT);assert_eq!(app.files_menu_target.as_ref().unwrap().0,1);assert_eq!(app.workspace.current,0);key(&mut app,KeyCode::Esc,KeyModifiers::NONE);key(&mut app,KeyCode::Esc,KeyModifiers::NONE);
        let text=app.diff.as_ref().unwrap().diff_area;app.handle(mouse(MouseEventKind::Down(MouseButton::Right),text.x+2,text.y+2));assert!(screen(&mut app).contains("All commands…"));assert!(!app.files_focused);
        key(&mut app,KeyCode::Char('q'),KeyModifiers::NONE);assert!(!app.quit);assert_eq!(app.workspace.current,0);
        app.handle(mouse(MouseEventKind::Down(MouseButton::Left),files.x+1,files.y+1));assert_eq!(app.workspace.current,0,"a preview modal must own even an outer file-list click");
        app.handle(mouse(MouseEventKind::Down(MouseButton::Left),files.x+1,files.y+1));settle(&mut app);assert_eq!(app.workspace.current,1);
        for path in ["a.txt","b.txt"]{assert_eq!(std::fs::read(d.join(path)).unwrap(),b"ours\n");}
    }
    #[test]
    fn markdown_tui_version_picker_preview_has_real_rendered_controls_and_modal_priority() {
        let dir=tempfile::tempdir().unwrap();let d=dir.path();git(d,&["init","-q","-b","main"],true);git(d,&["config","user.name","Test"],true);git(d,&["config","user.email","test@example.test"],true);
        std::fs::write(d.join("a.md"),"# Base\n").unwrap();git(d,&["add","."],true);git(d,&["commit","-qm","base"],true);git(d,&["checkout","-qb","topic"],true);git(d,&["rm","a.md"],true);git(d,&["commit","-qm","delete"],true);git(d,&["checkout","-q","main"],true);
        std::fs::write(d.join("a.md"),"# Ours\n\n**safe**\n").unwrap();git(d,&["commit","-qam","ours"],true);git(d,&["merge","-q","topic"],false);
        let mut app=Merger::from_repository(d.into(),Vec::new());settle(&mut app);assert!(matches!(app.workspace.file,Some(FileView::Picker(_))));
        let mut terminal=ratatui::Terminal::new(ratatui::backend::TestBackend::new(160,36)).unwrap();terminal.draw(|f|app.render(f)).unwrap();
        let locate=|buffer:&ratatui::buffer::Buffer,needle:&str|{for y in 0..buffer.area.height {for x in 0..buffer.area.width {let row=(x..buffer.area.width).map(|x|buffer[(x,y)].symbol()).collect::<String>();if row.starts_with(needle){return(x,y);}}}panic!("missing {needle}");};
        let version_grip=app.resizer.grips.iter().find(|g|g.id==3).unwrap().area;let before=app.version_height;
        let version_mouse=|kind,y|Event::Mouse(crossterm::event::MouseEvent{kind,column:version_grip.x+2,row:y,modifiers:KeyModifiers::NONE});
        app.handle(version_mouse(MouseEventKind::Down(MouseButton::Left),version_grip.y));app.handle(version_mouse(MouseEventKind::Drag(MouseButton::Left),version_grip.y-2));app.handle(version_mouse(MouseEventKind::Up(MouseButton::Left),version_grip.y-2));terminal.draw(|f|app.render(f)).unwrap();assert_ne!(app.version_height,before);assert!(matches!(app.workspace.file,Some(FileView::Picker(_))));
        let at=locate(terminal.backend().buffer(),"Inline");app.handle(Event::Mouse(crossterm::event::MouseEvent {kind:MouseEventKind::Down(MouseButton::Left),column:at.0,row:at.1,modifiers:KeyModifiers::NONE}));settle(&mut app);terminal.draw(|f|app.render(f)).unwrap();
        assert_eq!(app.diff.as_ref().unwrap().session.markdown_mode(),lenz::markdown::PresentationMode::Rendered);
        let content=terminal.backend().buffer().content.iter().map(|c|c.symbol()).collect::<String>();assert!(content.contains("Ours"));assert!(!content.contains("# Ours"));assert!(!content.contains("**safe**"));
        app.diff.as_mut().unwrap().about=Some(Default::default());key(&mut app,KeyCode::Char('2'),KeyModifiers::NONE);key(&mut app,KeyCode::Char('z'),KeyModifiers::CONTROL);assert_eq!(std::fs::read(d.join("a.md")).unwrap(),b"# Ours\n\n**safe**\n");assert!(matches!(app.workspace.file,Some(FileView::Picker(_))));
        key(&mut app,KeyCode::Esc,KeyModifiers::NONE);assert!(app.diff.as_ref().unwrap().about.is_none());
    }
    #[test]
    fn markdown_tui_version_picker_marker_and_outer_save_keep_preview_read_only() {
        let dir=tempfile::tempdir().unwrap();let d=dir.path();
        for (name,bytes) in [("base",b"binary\0base".as_slice()),("ours.md",b"[guide](old)\n".as_slice()),("theirs.md",b"[guide](new)\n".as_slice()),("result.md",b"[guide](old)\n".as_slice())]{std::fs::write(d.join(name),bytes).unwrap();}
        let source_bytes_before=[std::fs::read(d.join("ours.md")).unwrap(),std::fs::read(d.join("theirs.md")).unwrap()];
        let source_hash=|bytes:&[Vec<u8>;2]|{use std::hash::{Hash,Hasher};let mut h=std::collections::hash_map::DefaultHasher::new();bytes.hash(&mut h);h.finish()};let source_hash_before=source_hash(&source_bytes_before);
        let req=lenz::cli::parse(&["merge","base","ours.md","theirs.md","result.md"].map(str::to_owned),d).unwrap();let mut app=Merger::open(req,d.into());settle(&mut app);assert!(matches!(app.workspace.file,Some(FileView::Picker(_))));
        let draw=|app:&mut Merger|{let mut terminal=Terminal::new(TestBackend::new(180,40)).unwrap();terminal.draw(|f|app.render(f)).unwrap();terminal.backend().buffer().clone()};
        let locate=|b:&ratatui::buffer::Buffer,needle:&str|{for y in 0..b.area.height{for x in 0..b.area.width{let text=(x..b.area.width).map(|x|b[(x,y)].symbol()).collect::<String>();if text.starts_with(needle){return(x,y);}}}panic!("missing {needle}");};
        let click=|app:&mut Merger,(x,y)|{app.handle(Event::Mouse(crossterm::event::MouseEvent{kind:MouseEventKind::Down(MouseButton::Left),column:x,row:y,modifiers:KeyModifiers::NONE}));};
        let b=draw(&mut app);click(&mut app,locate(&b,"Inline"));settle(&mut app);let b=draw(&mut app);
        let preview=app.diff.as_ref().unwrap();let source_view=preview.session.view();let projection=preview.session.markdown_projection().unwrap();for id in 0..source_view.change_count(){assert!(projection.represents_hunk(id));}let anchor=projection.location(0,Side::Left).unwrap();
        key(&mut app,KeyCode::Char('n'),KeyModifiers::NONE);assert_eq!(app.diff.as_ref().unwrap().session.view().current_change(),Some(0));click(&mut app,locate(&b,"Markup change"));assert_eq!(app.diff.as_ref().unwrap().session.markdown_anchor(anchor.side),Some(anchor));
        let b=draw(&mut app);click(&mut app,locate(&b,"Inline"));settle(&mut app);key(&mut app,KeyCode::Char('2'),KeyModifiers::NONE);key(&mut app,KeyCode::Char('s'),KeyModifiers::CONTROL);assert_eq!(std::fs::read(d.join("result.md")).unwrap(),source_bytes_before[1]);
        assert!(matches!(&app.workspace.file,Some(FileView::Picker(p)) if p.chosen==Some(1)&&!p.dirty));
        key(&mut app,KeyCode::Char('z'),KeyModifiers::CONTROL);
        assert!(matches!(&app.workspace.file,Some(FileView::Picker(p)) if p.chosen.is_none()&&p.dirty));
        let screen=draw(&mut app);let visible=screen.content().iter().map(|c|c.symbol()).collect::<String>();assert!(!visible.contains("✓ 2 Theirs"),"Undo must remove the chosen-version checkmark: {visible}");
        assert_eq!(std::fs::read(d.join("result.md")).unwrap(),source_bytes_before[1],"Undo is unsaved until the next Save");
        key(&mut app,KeyCode::Char('s'),KeyModifiers::CONTROL);
        assert_eq!(std::fs::read(d.join("result.md")).unwrap(),source_bytes_before[0]);
        assert!(matches!(&app.workspace.file,Some(FileView::Picker(p)) if p.chosen.is_none()&&!p.dirty));
        key(&mut app,KeyCode::Char('y'),KeyModifiers::CONTROL);
        key(&mut app,KeyCode::Char('s'),KeyModifiers::CONTROL);
        assert_eq!(std::fs::read(d.join("result.md")).unwrap(),source_bytes_before[1]);
        assert_eq!(app.diff.as_ref().unwrap().session.markdown_mode(),lenz::markdown::PresentationMode::Rendered);let source_bytes_after=[std::fs::read(d.join("ours.md")).unwrap(),std::fs::read(d.join("theirs.md")).unwrap()];let source_hash_after=source_hash(&source_bytes_after);assert_eq!(source_hash_before,source_hash_after);assert_eq!(source_bytes_before,source_bytes_after);
    }
    fn toolbar_row(app: &mut Merger, width: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..width).map(|x| buffer[(x, 0)].symbol().to_string()).collect()
    }
    #[test]
    fn merge_toolbar_is_lean_like_the_gui() {
        let (_dir, mut app) = open_fixture([b"a\nb\nc\n", b"a\nX\nc\n", b"a\nY\nc\n"]);
        let bar = toolbar_row(&mut app, 160);
        for label in ["☰", "↑", "↓", "⇡", "⇣", "↶", "↷", "⤓", "Done", "Cancel"] { assert!(bar.contains(label), "{label}: {bar}"); }
        for gone in ["Undo", "Redo", "Save", "Find", "Ours", "Theirs", "Both", "Edit ▾", "View ▾", "More ▾"] { assert!(!bar.contains(gone), "{gone}: {bar}"); }
        // ☰ holds the app menu
        let bar_buffer = { let mut t = Terminal::new(TestBackend::new(160, 24)).unwrap(); t.draw(|f| app.render(f)).unwrap(); t.backend().buffer().clone() };
        let x = (0..160).find(|&x| bar_buffer[(x, 0)].symbol() == "☰").unwrap();
        app.handle(Event::Mouse(crossterm::event::MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: x, row: 0, modifiers: KeyModifiers::NONE }));
        let menu = screen(&mut app);
        for label in ["Settings…", "Change theme", "Keyboard shortcuts", "About Lenz", "Nerd Font icons"] { assert!(menu.contains(label), "{label}"); }
        key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        // Reset pane sizes is in the merge palette
        key(&mut app, KeyCode::Char(':'), KeyModifiers::NONE);
        app.handle(Event::Paste("reset pane".into()));
        assert!(screen(&mut app).contains("Reset pane sizes"));
        key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        // narrow (as the GUI below 640 px): the arrows and Done/Cancel leave the bar for the palette; Save stays
        let narrow = toolbar_row(&mut app, 40);
        assert!(narrow.contains("☰") && narrow.contains("⤓") && !narrow.contains("⋯") && !narrow.contains("⇡"), "{narrow}");
    }
    #[test]
    fn version_picker_toolbar_is_lean_like_the_gui() {
        let (_dir, mut app) = open_fixture([b"\0base", b"\0ours", b"\0theirs"]);
        let bar = toolbar_row(&mut app, 200);
        for label in ["☰", "↶", "↷", "⤓", "Done", "Cancel", "Take"] { assert!(bar.contains(label), "{label}: {bar}"); }
        for gone in ["Undo", "Redo", "Save", "Find", "Edit ▾", "View ▾", "More ▾"] { assert!(!bar.contains(gone), "{gone}: {bar}"); }
    }
    #[test]
    fn merge_change_menu_closed_without_a_choice_restores_the_current_change() {
        let (_dir, mut app) = open_fixture([b"a\nb\nc\nd\ne\nf\ng\n", b"a\nX\nc\nd\ne\nf\nP\n", b"a\nY\nc\nd\ne\nf\nQ\n"]);
        screen(&mut app);
        let current = |app: &Merger| match &app.workspace.file { Some(FileView::Merge(m)) => m.current, _ => None };
        let before = current(&app);
        let panes = app.merge_panes();
        let mouse = |kind, column, row| Event::Mouse(crossterm::event::MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE });
        // the second conflict, text row 6 (row 0 is the header)
        app.handle(mouse(MouseEventKind::Down(MouseButton::Right), panes[2].x + 2, panes[2].y + 7));
        assert!(app.menu.is_some());
        assert_ne!(current(&app), before, "the menu is for the change under the pointer");
        key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(current(&app), before, "Esc puts the current change back");
        app.handle(mouse(MouseEventKind::Down(MouseButton::Right), panes[2].x + 2, panes[2].y + 7));
        app.handle(mouse(MouseEventKind::Down(MouseButton::Left), 0, 23));
        assert_eq!(current(&app), before, "a click outside the menu too");
    }
    #[test]
    fn merge_shift_f10_with_a_stale_current_change_opens_the_palette() {
        let (_dir, mut app) = open_fixture([b"a\nb\nc\n", b"a\nX\nc\n", b"a\nY\nc\n"]);
        screen(&mut app);
        if let Some(FileView::Merge(m)) = &mut app.workspace.file { m.current = Some(99); }
        key(&mut app, KeyCode::F(10), KeyModifiers::SHIFT);
        assert!(app.command_palette.is_some() && app.menu.is_none());
    }
    #[test]
    fn merge_right_click_lists_the_choices_for_the_conflict_under_the_pointer() {
        let (dir, mut app) = open_fixture([b"a\nb\nc\n", b"a\nX\nc\n", b"a\nY\nc\n"]);
        screen(&mut app);
        let before = std::fs::read(dir.path().join("result")).unwrap();
        let panes = app.merge_panes();
        let mouse = |kind, column, row| Event::Mouse(crossterm::event::MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE });
        // row 0 is the pane header; text line 2 ("X"/"b"/"Y") is the conflict
        app.handle(mouse(MouseEventKind::Down(MouseButton::Right), panes[2].x + 2, panes[2].y + 2));
        assert!(app.command_palette.is_none());
        let menu = screen(&mut app);
        for label in ["Take Ours", "Take Theirs", "Take both, Ours first", "Keep the Base", "All commands…"] { assert!(menu.contains(label), "{label}: {menu}"); }
        assert!(!menu.contains("Reset this conflict resolution"), "unresolved: nothing to reset");
        app.handle(Event::Paste("theirs".into()));
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(matches!(&app.workspace.file, Some(FileView::Merge(m)) if m.m.lines().iter().any(|l| l.starts_with('Y'))), "Theirs taken");
        assert_eq!(std::fs::read(dir.path().join("result")).unwrap(), before, "nothing written until Save");
        // resolved now: the same menu offers to reset it
        app.handle(mouse(MouseEventKind::Down(MouseButton::Right), panes[2].x + 2, panes[2].y + 2));
        assert!(screen(&mut app).contains("Reset this conflict resolution"));
        key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        // an unchanged line has no choices: the palette
        app.handle(mouse(MouseEventKind::Down(MouseButton::Right), panes[2].x + 2, panes[2].y + 1));
        assert!(app.command_palette.is_some());
        key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        // Shift-F10: the current change
        key(&mut app, KeyCode::F(10), KeyModifiers::SHIFT);
        assert!(screen(&mut app).contains("Take Ours"));
        key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(std::fs::read(dir.path().join("result")).unwrap(), before);
    }
    #[test]
    fn merge_f10_and_colon_open_the_shared_palette_and_run_merge_actions() {
        let (dir, mut app) = open_fixture([b"a\nb\nc\nd\ne\nf\ng\n", b"a\nX\nc\nd\ne\nf\nY\n", b"a\nZ\nc\nd\ne\nf\nW\n"]);
        let before = std::fs::read(dir.path().join("result")).unwrap();
        let current = |app: &Merger| match &app.workspace.file { Some(FileView::Merge(m)) => m.current, _ => None };
        let first = current(&app);
        key(&mut app, KeyCode::F(10), KeyModifiers::NONE);
        assert!(app.command_palette.is_some());
        app.handle(Event::Paste("next unresolved".into()));
        assert!(screen(&mut app).contains("Next unresolved conflict"));
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(app.command_palette.is_none());
        assert_ne!(current(&app), first, "Enter ran the merge action");
        key(&mut app, KeyCode::Char(':'), KeyModifiers::NONE);
        assert!(app.command_palette.is_some());
        app.handle(Event::Paste("zz".into()));
        key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(app.command_palette.is_none());
        assert_eq!(std::fs::read(dir.path().join("result")).unwrap(), before, "typing into the palette never edits");
    }
    fn open_fixture(bytes: [&[u8]; 3]) -> (tempfile::TempDir, Merger) {
        let dir = tempfile::tempdir().unwrap();
        for (name, bytes) in ["base", "ours", "theirs"].into_iter().zip(bytes) {
            std::fs::write(dir.path().join(name), bytes).unwrap();
        }
        std::fs::write(dir.path().join("result"), bytes[1]).unwrap();
        let args = ["merge", "base", "ours", "theirs", "result"].map(str::to_owned);
        let request = lenz::cli::parse(&args, dir.path()).unwrap();
        let mut app = Merger::open(request, dir.path().to_owned());
        for _ in 0..10000 {
            app.poll();
            if !app.busy() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(!app.busy());
        (dir, app)
    }
    fn screen(app: &mut Merger) -> String {
        let mut terminal = Terminal::new(TestBackend::new(120, 18)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect()
    }
    #[test]
    fn merge_render_handles_tiny_terminal_sizes() {
        let (_dir,mut app)=open_fixture([b"base\n",b"ours\n",b"theirs\n"]);
        for width in 0..9 {for height in 0..9 {
            let mut terminal=Terminal::new(TestBackend::new(width,height)).unwrap();
            terminal.draw(|f|app.render(f)).unwrap();
        }}
    }
    #[test]
    fn compact_merge_search_menu_clicks_and_dropdown_wheel_preserve_choices() {
        let (dir,mut app)=open_fixture([b"base\n",b"ours\n",b"theirs\n"]);
        app.icons=crate::icons::Icons::Plain;
        let mut terminal=Terminal::new(TestBackend::new(80,24)).unwrap();terminal.draw(|f|app.render(f)).unwrap();
        let locate=|buf:&ratatui::buffer::Buffer,needle:&str|{for y in 0..buf.area.height{for x in 0..buf.area.width{let row=(x..buf.area.width).map(|x|buf[(x,y)].symbol()).collect::<String>();if row.starts_with(needle){return(x,y);}}}panic!("missing {needle}");};
        let click=|app:&mut Merger,at:(u16,u16)|app.handle(Event::Mouse(crossterm::event::MouseEvent{kind:MouseEventKind::Down(MouseButton::Left),column:at.0,row:at.1,modifiers:KeyModifiers::NONE}));
        assert!(app.text_area.y<=6);let at=locate(terminal.backend().buffer(),"☰");assert!(click(&mut app,at));terminal.draw(|f|app.render(f)).unwrap();
        let before=match &app.workspace.file{Some(FileView::Merge(m))=>format!("{:?}",m.layout()),_=>panic!()};
        app.handle(Event::Mouse(crossterm::event::MouseEvent{kind:MouseEventKind::ScrollDown,column:20,row:15,modifiers:KeyModifiers::NONE}));
        assert_eq!(match &app.workspace.file{Some(FileView::Merge(m))=>format!("{:?}",m.layout()),_=>panic!()},before);key(&mut app,KeyCode::Esc,KeyModifiers::NONE);
        key(&mut app,KeyCode::Char(':'),KeyModifiers::NONE);for ch in "about".chars(){key(&mut app,KeyCode::Char(ch),KeyModifiers::NONE);}
        terminal.backend_mut().resize(48,16);terminal.resize(Rect::new(0,0,48,16)).unwrap();terminal.draw(|f|app.render(f)).unwrap();let at=locate(terminal.backend().buffer(),"About Lenz");assert!(click(&mut app,at));assert!(app.about.is_some());
        key(&mut app,KeyCode::Char('2'),KeyModifiers::NONE);key(&mut app,KeyCode::Char('s'),KeyModifiers::CONTROL);assert_eq!(std::fs::read(dir.path().join("result")).unwrap(),b"ours\n");
    }

    #[test]
    fn merge_toolbar_and_help_use_real_input_without_changing_result() {
        let (dir,mut app)=open_fixture([b"base\n",b"ours\n",b"theirs\n"]);
        let mut terminal=Terminal::new(TestBackend::new(160,30)).unwrap();
        terminal.draw(|f|app.render(f)).unwrap();
        let buffer=terminal.backend().buffer();
        assert!(buffer.content.iter().map(|c|c.symbol()).collect::<String>().contains("⤓"));
        key(&mut app,KeyCode::Char('?'),KeyModifiers::NONE);
        assert!(screen(&mut app).contains("Keyboard shortcuts"));
        key(&mut app,KeyCode::Esc,KeyModifiers::NONE);
        assert!(!app.quit);
        key(&mut app,KeyCode::Char('2'),KeyModifiers::NONE);
        // Alt+F10 walks the toolbar: ☰ ↑ ↓ ⇡ ⇣ Undo Redo Save
        key(&mut app,KeyCode::F(10),KeyModifiers::ALT);
        for _ in 0..7 {key(&mut app,KeyCode::Right,KeyModifiers::NONE);}
        key(&mut app,KeyCode::Enter,KeyModifiers::NONE);
        assert_eq!(std::fs::read(dir.path().join("result")).unwrap(),b"theirs\n");
        assert!(!app.quit,"Save must not finish the merge");
    }
    #[test]
    fn merge_edit_icons_use_actual_mouse_targets_in_each_theme() {
        for theme in lenz::theme::builtin_entries() {
            let (dir,mut app)=open_fixture([b"base\n",b"ours\n",b"theirs\n"]);
            app.palette=theme.palette;
            key(&mut app,KeyCode::Char('2'),KeyModifiers::NONE);
            let mut terminal=Terminal::new(TestBackend::new(48,24)).unwrap();
            for label in ["⤓","↶","↷"] {
                terminal.draw(|f|app.render(f)).unwrap();
                let x=(0..48).find(|&x|terminal.backend().buffer()[(x,0)].symbol()==label).unwrap();
                app.handle(Event::Mouse(crossterm::event::MouseEvent{kind:MouseEventKind::Down(MouseButton::Left),column:x,row:0,modifiers:KeyModifiers::NONE}));
                assert_eq!(std::fs::read(dir.path().join("result")).unwrap(),b"theirs\n");
            }
            assert!(matches!(&app.workspace.file,Some(FileView::Merge(m)) if m.m.lines().join("\n")=="theirs\n"));
        }
    }
    #[test]
    fn about_merge_menu_blocks_choices_and_save_then_restores_controls() {
        let (dir,mut app)=open_fixture([b"base\n",b"ours\n",b"theirs\n"]);
        key(&mut app,KeyCode::Char(':'),KeyModifiers::NONE);
        app.handle(Event::Paste("about".into()));
        assert!(screen(&mut app).contains("About Lenz"));
        key(&mut app,KeyCode::Enter,KeyModifiers::NONE);
        assert!(screen(&mut app).contains("MIT; engine Apache-2.0"));
        key(&mut app,KeyCode::Char('2'),KeyModifiers::NONE);
        key(&mut app,KeyCode::Char('s'),KeyModifiers::CONTROL);
        assert_eq!(std::fs::read(dir.path().join("result")).unwrap(),b"ours\n");
        assert!(screen(&mut app).contains("About Lenz"));
        key(&mut app,KeyCode::Esc,KeyModifiers::NONE);
        key(&mut app,KeyCode::Char('2'),KeyModifiers::NONE);
        key(&mut app,KeyCode::Char('s'),KeyModifiers::CONTROL);
        assert_eq!(std::fs::read(dir.path().join("result")).unwrap(),b"theirs\n");
    }
    #[test]
    fn keys_render_three_versions_and_save_undo_redo_exact_result() {
        let (dir, mut app) = open_fixture([b"base\n", b"ours\n", b"theirs\n"]);
        let view = screen(&mut app);
        assert!(view.contains("Ours") && view.contains("Result") && view.contains("Theirs"));
        key(&mut app, KeyCode::Char('2'), KeyModifiers::NONE);
        key(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
        assert!(screen(&mut app).contains("1 conflict"));
        key(&mut app, KeyCode::Char('y'), KeyModifiers::CONTROL);
        key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(
            std::fs::read(dir.path().join("result")).unwrap(),
            b"theirs\n"
        );
        key(&mut app, KeyCode::Enter, KeyModifiers::CONTROL);
        assert!(app.quit);
        assert_eq!(app.exit_code(), 0);
    }
    #[test]
    fn done_cancel_and_changed_file_require_visible_confirmation() {
        let (dir, mut app) = open_fixture([b"base\n", b"ours\n", b"theirs\n"]);
        key(&mut app, KeyCode::Enter, KeyModifiers::CONTROL);
        assert!(!app.quit);
        assert!(screen(&mut app).contains("conflicts remain"));
        key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        key(&mut app, KeyCode::Char('1'), KeyModifiers::NONE);
        std::fs::write(dir.path().join("result"), b"external\n").unwrap();
        key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert!(screen(&mut app).contains("changed on disk"));
        assert_eq!(
            std::fs::read(dir.path().join("result")).unwrap(),
            b"external\n"
        );
        key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(std::fs::read(dir.path().join("result")).unwrap(), b"ours\n");
        key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(!app.quit);
        key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(app.quit);
        assert_eq!(app.exit_code(), 1);
    }
    #[test]
    fn binary_picker_keys_write_selected_bytes_without_decoding() {
        let (dir, mut app) = open_fixture([b"\0base", b"\0ours\xff", b"\0theirs\xfe"]);
        assert!(screen(&mut app).contains("take one version whole"));
        key(&mut app, KeyCode::Char('2'), KeyModifiers::NONE);
        key(&mut app, KeyCode::Enter, KeyModifiers::CONTROL);
        assert!(app.quit);
        assert_eq!(
            std::fs::read(dir.path().join("result")).unwrap(),
            b"\0theirs\xfe"
        );
    }
    fn git(root: &std::path::Path, args: &[&str], success: bool) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert_eq!(status.success(), success, "git {args:?}");
    }
    fn settle(app: &mut Merger) {
        for _ in 0..10000 {
            app.poll();
            if !app.busy() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("merge did not settle");
    }
    #[test]
    fn conflict_list_stages_saved_output_reopens_and_refreshes_real_git() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        git(d, &["init", "-q", "-b", "main"], true);
        git(d, &["config", "user.name", "Lenz test"], true);
        git(d, &["config", "user.email", "test@example.invalid"], true);
        std::fs::write(d.join("f.txt"), b"base\n").unwrap();
        git(d, &["add", "."], true);
        git(d, &["commit", "-qm", "base"], true);
        git(d, &["checkout", "-qb", "feature"], true);
        std::fs::write(d.join("f.txt"), b"theirs\n").unwrap();
        git(d, &["commit", "-qam", "theirs"], true);
        git(d, &["checkout", "-q", "main"], true);
        std::fs::write(d.join("f.txt"), b"ours\n").unwrap();
        git(d, &["commit", "-qam", "ours"], true);
        git(d, &["merge", "-q", "feature"], false);
        let mut app = Merger::from_repository(d.to_owned(), Vec::new());
        settle(&mut app);
        assert!(screen(&mut app).contains("Merging feature"));
        key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        settle(&mut app);
        assert!(
            app.diff.is_some(),
            "cancel shows the saved file through the real comparison adapter"
        );
        key(&mut app, KeyCode::Char(':'), KeyModifiers::NONE);
        assert!(
            app.command_palette.is_some() && screen(&mut app).contains("Next unresolved conflict"),
            "closed merge previews still present the conflict commands"
        );
        key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        key(&mut app, KeyCode::Char('o'), KeyModifiers::NONE);
        settle(&mut app);
        assert!(matches!(app.workspace.file, Some(FileView::Merge(_))));
        key(&mut app, KeyCode::Char('2'), KeyModifiers::NONE);
        key(&mut app, KeyCode::Char('M'), KeyModifiers::SHIFT);
        assert!(screen(&mut app).contains("save them first"));
        assert_eq!(lenz::conflicts::list(d).unwrap().len(), 1);
        key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
        key(&mut app, KeyCode::Char('M'), KeyModifiers::SHIFT);
        settle(&mut app);
        assert!(lenz::conflicts::list(d).unwrap().is_empty());
        assert!(screen(&mut app).contains("All resolved"));
        assert_eq!(std::fs::read(d.join("f.txt")).unwrap(), b"theirs\n");
        key(&mut app, KeyCode::Char('R'), KeyModifiers::SHIFT);
        settle(&mut app);
        assert_eq!(lenz::conflicts::list(d).unwrap().len(), 1);
        assert!(
            std::fs::read_to_string(d.join("f.txt"))
                .unwrap()
                .contains("<<<<<<<")
        );
        assert!(screen(&mut app).contains("1 conflict"));
    }

    fn deleted_fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        git(d, &["init", "-q", "-b", "main"], true);
        git(d, &["config", "user.name", "Lenz test"], true);
        git(d, &["config", "user.email", "test@example.invalid"], true);
        std::fs::write(d.join("gone"), b"base\n").unwrap();
        git(d, &["add", "."], true);
        git(d, &["commit", "-qm", "base"], true);
        git(d, &["checkout", "-qb", "feature"], true);
        std::fs::write(d.join("gone"), b"changed\n").unwrap();
        git(d, &["commit", "-qam", "theirs"], true);
        git(d, &["checkout", "-q", "main"], true);
        git(d, &["rm", "-q", "gone"], true);
        git(d, &["commit", "-qm", "ours delete"], true);
        git(d, &["merge", "-q", "feature"], false);
        dir
    }
    #[test]
    fn version_picker_delete_stages_deletion_without_recreating_the_file() {
        let dir = deleted_fixture();
        let d = dir.path();
        let mut app = Merger::from_repository(d.to_owned(), Vec::new());
        settle(&mut app);
        assert!(screen(&mut app).contains("Delete"));
        key(&mut app, KeyCode::Char('2'), KeyModifiers::NONE);
        key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert!(!d.join("gone").exists());
        key(&mut app, KeyCode::Char('M'), KeyModifiers::SHIFT);
        settle(&mut app);
        assert!(lenz::conflicts::list(d).unwrap().is_empty());
        assert!(!d.join("gone").exists());
        key(&mut app, KeyCode::Char('R'), KeyModifiers::SHIFT);
        settle(&mut app);
        assert_eq!(lenz::conflicts::list(d).unwrap().len(), 1);
        assert!(matches!(app.workspace.file, Some(FileView::Picker(_))));
    }

    #[test]
    fn menu_all_from_and_block_choices_base_toggle_and_start_over_are_wired() {
        let (dir, mut app) = open_fixture([b"base\n", b"ours\n", b"theirs\n"]);
        key(&mut app, KeyCode::Char(':'), KeyModifiers::NONE);
        app.handle(Event::Paste("take all theirs".into()));
        assert!(screen(&mut app).contains("Take all Theirs"));
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(
            std::fs::read(dir.path().join("result")).unwrap(),
            b"theirs\n"
        );
        key(
            &mut app,
            KeyCode::Char('R'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert!(screen(&mut app).contains("1 conflict"));
        key(&mut app, KeyCode::Char('b'), KeyModifiers::NONE);
        assert!(screen(&mut app).contains("Base"));
        key(&mut app, KeyCode::Char('b'), KeyModifiers::NONE);
        key(&mut app, KeyCode::Char('3'), KeyModifiers::NONE);
        key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(
            std::fs::read(dir.path().join("result")).unwrap(),
            b"ours\ntheirs\n"
        );
        key(&mut app, KeyCode::Char('4'), KeyModifiers::NONE);
        key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(std::fs::read(dir.path().join("result")).unwrap(), b"base\n");
    }

    #[test]
    fn picker_comparison_keeps_real_find_paste_and_scroll_input() {
        let (_dir, mut binary) = open_fixture([b"\0base", b"\0ours", b"\0theirs"]);
        key(&mut binary, KeyCode::Char('f'), KeyModifiers::CONTROL);
        assert!(
            binary.diff.as_ref().unwrap().prompt.is_none(),
            "binary placeholders correctly disable text Find"
        );
        assert!(binary.diff.as_ref().unwrap().session.notice().is_some());
        key(&mut binary, KeyCode::Char(':'), KeyModifiers::NONE);
        // the Version Picker's own commands: the palette's Edit group
        binary.handle(Event::Paste("edit".into()));
        let rendered = screen(&mut binary);
        for label in [
            "Undo version choice",
            "Nothing to undo",
            "Save chosen version",
            "Done",
            "Cancel",
        ] {
            assert!(
                rendered.contains(label),
                "whole-version commands remain visible: {label}"
            );
        }
        key(&mut binary, KeyCode::Esc, KeyModifiers::NONE);
        key(&mut binary, KeyCode::Char('2'), KeyModifiers::NONE);
        assert!(matches!(&binary.workspace.file,Some(FileView::Picker(p)) if p.chosen==Some(1)));
        let dir = deleted_fixture();
        let mut app = Merger::from_repository(dir.path().into(), Vec::new());
        settle(&mut app);
        assert!(matches!(app.workspace.file, Some(FileView::Picker(_))));
        key(&mut app, KeyCode::Char('f'), KeyModifiers::CONTROL);
        assert!(app.diff.as_ref().unwrap().prompt.is_some());
        app.handle(Event::Paste("changed".into()));
        assert!(
            app.diff
                .as_ref()
                .unwrap()
                .session
                .find()
                .is_some_and(|f| !f.hits().is_empty())
        );
        assert!(screen(&mut app).contains("changed"));
        key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(!app.quit);
    }

    #[test]
    fn nested_preview_draft_survives_workspace_actions_and_operation_disappearance() {
        let dir = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let d = dir.path();
        git(d, &["init", "-q", "-b", "main"], true);
        git(d, &["config", "user.name", "Test"], true);
        git(d, &["config", "user.email", "test@example.invalid"], true);
        for path in ["a.txt", "b.txt"] { std::fs::write(d.join(path), "base\n").unwrap(); }
        git(d, &["add", "."], true); git(d, &["commit", "-qm", "base"], true);
        git(d, &["checkout", "-qb", "topic"], true);
        for path in ["a.txt", "b.txt"] { std::fs::write(d.join(path), "theirs\n").unwrap(); }
        git(d, &["commit", "-qam", "theirs"], true); git(d, &["checkout", "-q", "main"], true);
        for path in ["a.txt", "b.txt"] { std::fs::write(d.join(path), "ours\n").unwrap(); }
        git(d, &["commit", "-qam", "ours"], true); git(d, &["merge", "-q", "topic"], false);
        let mut app = Merger::from_repository(d.into(), Vec::new()); settle(&mut app); screen(&mut app);
        for i in 0..2 {
            if i == 1 { let index=app.workspace.rows.iter().position(|row|row.0=="b.txt").unwrap();app.open_index(index);settle(&mut app); }
            key(&mut app, KeyCode::Char('2'), KeyModifiers::NONE);
            key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
            key(&mut app, KeyCode::Char('M'), KeyModifiers::SHIFT); settle(&mut app);
        }
        let first=app.workspace.rows.iter().position(|row|row.0=="a.txt").unwrap();app.open_index(first);settle(&mut app);screen(&mut app);
        assert!(app.diff.is_some());
        key(&mut app, KeyCode::Right, KeyModifiers::ALT | KeyModifiers::SHIFT); settle(&mut app);
        assert!(app.preview_dirty(), "the editable HEAD preview must create a real draft");
        assert_eq!(std::fs::read(d.join("a.txt")).unwrap(), b"theirs\n");
        let path = app.diff_path.clone(); let index = app.workspace.current;
        for (code, mods) in [
            (KeyCode::Char('q'), KeyModifiers::NONE), (KeyCode::Char('q'), KeyModifiers::CONTROL),
            (KeyCode::Esc, KeyModifiers::NONE), (KeyCode::Char(']'), KeyModifiers::NONE),
            (KeyCode::Char('o'), KeyModifiers::NONE), (KeyCode::Char('M'), KeyModifiers::SHIFT),
            (KeyCode::Char('R'), KeyModifiers::SHIFT), (KeyCode::Char('r'), KeyModifiers::CONTROL),
        ] {
            key(&mut app, code, mods); settle(&mut app);
            assert!(app.preview_dirty()); assert_eq!(app.diff_path, path); assert_eq!(app.workspace.current, index);
            assert!(!app.quit && !app.back); assert_eq!(std::fs::read(d.join("a.txt")).unwrap(), b"theirs\n");
        }
        // The list menu must refuse both staging and rewriting conflict markers.
        for action in [ListAct::MarkResolved, ListAct::Reopen, ListAct::Refresh, ListAct::Close] {
            // the toolbar's File actions ▾
            app.toolbar_action(ToolbarAction::List(action)); settle(&mut app);
            assert!(app.preview_dirty()); assert_eq!(app.diff_path, path);
            assert_eq!(std::fs::read(d.join("a.txt")).unwrap(), b"theirs\n");
        }
        key(&mut app, KeyCode::BackTab, KeyModifiers::SHIFT);
        app.handle(Event::Paste("b.txt".into())); key(&mut app, KeyCode::Enter, KeyModifiers::NONE); settle(&mut app);
        assert_eq!(app.workspace.current, index); assert!(app.preview_dirty());
        app.handle(Event::FocusGained); settle(&mut app); assert!(app.preview_dirty());
        app.leave_to_repository(); assert!(!app.back && app.preview_dirty());
        // Simulate a refresh already in flight when the draft was created: the
        // external commit ends the operation, but polling must retain the draft.
        git(d, &["commit", "-qm", "Resolved"], true);
        app.workspace.refresh(); settle(&mut app);
        assert!(app.workspace.snapshot.is_none()); assert!(app.preview_dirty()); assert!(!app.back && !app.quit);
        assert!(screen(&mut app).contains("Unsaved changes"));
        key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL); settle(&mut app);
        assert_eq!(std::fs::read(d.join("a.txt")).unwrap(), b"ours\n");
        assert!(!app.preview_dirty());
    }

    #[test]
    fn closed_and_marked_previews_offer_real_comparison_copy_and_editor_commands() {
        let dir = deleted_fixture();
        let mut app = Merger::from_repository(dir.path().into(), Vec::new());
        settle(&mut app);
        key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        settle(&mut app);
        for marked in [false, true] {
            if marked {
                key(&mut app, KeyCode::Char('M'), KeyModifiers::SHIFT);
                settle(&mut app);
                assert!(lenz::conflicts::list(dir.path()).unwrap().is_empty());
            }
            key(&mut app, KeyCode::F(4), KeyModifiers::NONE);
            assert!(app.diff.as_ref().unwrap().command_palette.is_some());
            app.handle(Event::Paste("copy file path".into()));
            assert!(screen(&mut app).contains("Copy file path"));
            key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
            assert!(
                app.diff
                    .as_ref()
                    .unwrap()
                    .session
                    .notice()
                    .unwrap()
                    .contains("Copied")
            );
            key(&mut app, KeyCode::F(4), KeyModifiers::NONE);
            app.handle(Event::Paste("editor".into()));
            let diff = app.diff.as_ref().unwrap();
            assert!(
                diff.interaction_disabled(lenz::commands::Interaction::Editor)
                    .is_none()
            );
            assert!(screen(&mut app).contains("Open focused side in editor"));
            key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        }
        git(dir.path(), &["commit", "-qm", "Resolved"], true);
        assert!(app.handle(Event::FocusGained));
        settle(&mut app);
        assert!(app.back, "finished operation returns to repository");
        assert!(!app.quit);
    }

    #[test]
    fn picker_stale_save_reload_and_delete_use_real_keys_without_losing_external_bytes() {
        let (dir, mut app) = open_fixture([b"\0base", b"\0ours", b"\0theirs"]);
        let target = dir.path().join("result");
        key(&mut app, KeyCode::Char('2'), KeyModifiers::NONE);
        std::fs::write(&target, b"\0external").unwrap();
        app.handle(Event::FocusGained);
        assert!(screen(&mut app).contains("Changed on disk"));
        key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert!(screen(&mut app).contains("Save again overwrites"));
        assert_eq!(std::fs::read(&target).unwrap(), b"\0external");
        key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(std::fs::read(&target).unwrap(), b"\0theirs");
        key(&mut app, KeyCode::Char('1'), KeyModifiers::NONE);
        std::fs::write(&target, b"\0new external").unwrap();
        key(&mut app, KeyCode::Enter, KeyModifiers::CONTROL);
        assert!(!app.quit);
        assert!(screen(&mut app).contains("Done again overwrites"));
        key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(
            !app.quit,
            "Esc dismisses stale confirmation, keeping the choice"
        );
        key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
        key(&mut app, KeyCode::Char('r'), KeyModifiers::NONE);
        assert!(
            matches!(&app.workspace.file, Some(FileView::Picker(p)) if p.chosen.is_none() && !p.dirty)
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"\0new external");
        let deletion = deleted_fixture();
        let mut app = Merger::from_repository(deletion.path().into(), Vec::new());
        settle(&mut app);
        let target = deletion.path().join("gone");
        key(&mut app, KeyCode::Char('2'), KeyModifiers::NONE);
        std::fs::write(&target, b"external keep\n").unwrap();
        key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(std::fs::read(&target).unwrap(), b"external keep\n");
        assert!(screen(&mut app).contains("Save again overwrites"));
        key(&mut app, KeyCode::Char('r'), KeyModifiers::NONE);
        assert!(
            matches!(&app.workspace.file, Some(FileView::Picker(p)) if p.chosen.is_none() && !p.dirty)
        );
        key(&mut app, KeyCode::Char('2'), KeyModifiers::NONE);
        std::fs::write(&target, b"second external keep\n").unwrap();
        key(&mut app, KeyCode::Enter, KeyModifiers::CONTROL);
        assert_eq!(std::fs::read(&target).unwrap(), b"second external keep\n");
        key(&mut app, KeyCode::Enter, KeyModifiers::CONTROL);
        assert!(!target.exists());
    }

    #[test]
    fn image_picker_real_input_preserves_image_controls_and_exact_version_bytes() {
        fn bmp(rgb: [u8; 3]) -> Vec<u8> {
            let mut bytes = vec![0; 58];
            bytes[..2].copy_from_slice(b"BM");
            for (offset, value) in [(2, 58u32), (10, 54), (14, 40), (18, 1), (22, 1), (34, 4)] {
                bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            }
            bytes[26..28].copy_from_slice(&1u16.to_le_bytes());
            bytes[28..30].copy_from_slice(&24u16.to_le_bytes());
            bytes[54..57].copy_from_slice(&[rgb[2], rgb[1], rgb[0]]);
            bytes
        }
        let (base, ours, theirs) = (bmp([0, 0, 0]), bmp([255, 0, 0]), bmp([0, 0, 255]));
        let (dir, mut app) = open_fixture([&base, &ours, &theirs]);
        assert!(app.diff.as_ref().unwrap().visual.image().is_some());
        key(&mut app, KeyCode::Char(':'), KeyModifiers::NONE);
        app.handle(Event::Paste("about".into()));
        assert!(screen(&mut app).contains("About Lenz"));
        key(&mut app,KeyCode::Enter,KeyModifiers::NONE);
        assert!(screen(&mut app).contains("MIT; engine Apache-2.0"));
        key(&mut app,KeyCode::Char('m'),KeyModifiers::NONE);
        key(&mut app,KeyCode::Char('s'),KeyModifiers::CONTROL);
        assert_eq!(std::fs::read(dir.path().join("result")).unwrap(),ours);
        assert_eq!(app.diff.as_ref().unwrap().visual.image_state().mode(),lenz::image_session::Mode::TwoUp);
        key(&mut app,KeyCode::Esc,KeyModifiers::NONE);
        // F4 opens the nested comparison's command menu. Its About must also
        // block the surrounding version picker and save controls.
        key(&mut app,KeyCode::F(4),KeyModifiers::NONE);
        for c in "about".chars(){key(&mut app,KeyCode::Char(c),KeyModifiers::NONE);}
        key(&mut app,KeyCode::Enter,KeyModifiers::NONE);
        assert!(app.diff.as_ref().unwrap().about.is_some(),"{}",screen(&mut app));
        key(&mut app,KeyCode::Char('2'),KeyModifiers::NONE);
        key(&mut app,KeyCode::Char('s'),KeyModifiers::CONTROL);
        key(&mut app,KeyCode::F(10),KeyModifiers::NONE);
        assert_eq!(std::fs::read(dir.path().join("result")).unwrap(),ours);
        assert!(app.diff.as_ref().unwrap().about.is_some());
        assert!(app.menu.is_none());
        key(&mut app,KeyCode::Esc,KeyModifiers::NONE);
        assert!(app.diff.as_ref().unwrap().about.is_none());
        // the picker's comparison commands are in the same palette (F4 opens the comparison's own)
        key(&mut app,KeyCode::F(4),KeyModifiers::NONE);
        assert!(app.diff.as_ref().unwrap().command_palette.is_some());
        app.handle(Event::Paste("cycle image mode".into()));
        assert!(screen(&mut app).contains("Cycle image mode"));
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(
            app.diff.as_ref().unwrap().visual.image_state().mode(),
            lenz::image_session::Mode::Swipe
        );
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        assert!(
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .any(|c| c.symbol() == "▀"),
            "the version chooser presents the actual comparison's image cells"
        );
        key(&mut app, KeyCode::Char('m'), KeyModifiers::NONE);
        assert_eq!(
            app.diff.as_ref().unwrap().visual.image_state().mode(),
            lenz::image_session::Mode::Onion
        );
        key(&mut app, KeyCode::Char('!'), KeyModifiers::NONE);
        key(&mut app, KeyCode::Char('2'), KeyModifiers::NONE);
        key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(std::fs::read(dir.path().join("result")).unwrap(), theirs);
    }

    fn conflicted_pair() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        git(d, &["init", "-q", "-b", "main"], true);
        git(d, &["config", "user.name", "Lenz perf"], true);
        git(d, &["config", "user.email", "test@example.invalid"], true);
        for name in ["a.txt", "b.txt"] {
            std::fs::write(d.join(name), b"base\n").unwrap();
        }
        git(d, &["add", "."], true);
        git(d, &["commit", "-qm", "base"], true);
        git(d, &["checkout", "-qb", "feature"], true);
        for name in ["a.txt", "b.txt"] {
            std::fs::write(d.join(name), b"theirs\n").unwrap();
        }
        git(d, &["commit", "-qam", "theirs"], true);
        git(d, &["checkout", "-q", "main"], true);
        for name in ["a.txt", "b.txt"] {
            std::fs::write(d.join(name), b"ours\n").unwrap();
        }
        git(d, &["commit", "-qam", "ours"], true);
        git(d, &["merge", "-q", "feature"], false);
        dir
    }
    #[test]
    fn conflict_list_switching_retains_each_files_unsaved_choices_and_undo() {
        let dir = conflicted_pair();
        let mut app = Merger::from_repository(dir.path().into(), Vec::new());
        settle(&mut app);
        key(&mut app, KeyCode::Char('2'), KeyModifiers::NONE);
        assert!(
            screen(&mut app).contains("●0 a.txt"),
            "active unsaved choices must update the sidebar count"
        );
        key(&mut app, KeyCode::Char(']'), KeyModifiers::NONE);
        settle(&mut app);
        key(&mut app, KeyCode::Char('1'), KeyModifiers::NONE);
        assert!(
            screen(&mut app).contains("●0 a.txt"),
            "stashed choices must keep their live sidebar count"
        );
        key(&mut app, KeyCode::Char('['), KeyModifiers::NONE);
        settle(&mut app);
        assert!(
            matches!(&app.workspace.file,Some(FileView::Merge(m)) if m.m.lines().join("\n")=="theirs\n" && m.dirty)
        );
        key(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
        assert!(matches!(&app.workspace.file,Some(FileView::Merge(m)) if m.m.unresolved()==1));
        key(&mut app, KeyCode::Char(']'), KeyModifiers::NONE);
        settle(&mut app);
        assert!(
            matches!(&app.workspace.file,Some(FileView::Merge(m)) if m.m.lines().join("\n")=="ours\n" && m.dirty)
        );
        assert!(
            std::fs::read_to_string(dir.path().join("a.txt"))
                .unwrap()
                .contains("<<<<<<<")
        );
        key(&mut app, KeyCode::Char('['), KeyModifiers::NONE);
        settle(&mut app);
        key(&mut app, KeyCode::Char('y'), KeyModifiers::CONTROL);
        key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert!(
            screen(&mut app).contains("○ a.txt"),
            "saving a resolved Result makes the sidebar row clean before refresh"
        );
        key(&mut app, KeyCode::Char(']'), KeyModifiers::NONE);
        settle(&mut app);
        assert!(
            screen(&mut app).contains("○ a.txt"),
            "retained saved Result rows remain clean"
        );
    }

    #[test]
    #[ignore = "release shared merge core and real terminal lifecycle workload"]
    fn terminal_perf_merge() {
        use crate::perf;
        use std::time::Instant;
        let base: String = (0..100_000)
            .map(|i| format!("let v{i} = compute({i});\n"))
            .collect();
        let edit = |every: usize, tag: &str| {
            base.lines()
                .enumerate()
                .map(|(i, l)| {
                    if i % every == 0 {
                        format!("{l} // {tag}\n")
                    } else {
                        format!("{l}\n")
                    }
                })
                .collect::<String>()
        };
        let (ours, theirs) = (edit(89, "ours"), edit(97, "theirs"));
        let started = Instant::now();
        let (dir, mut app) = open_fixture([base.as_bytes(), ours.as_bytes(), theirs.as_bytes()]);
        perf::record(
            "tui/merge/open-100k",
            "ms",
            &[started.elapsed().as_secs_f64() * 1000.],
        );
        let mut terminal = Terminal::new(TestBackend::new(160, 50)).unwrap();
        let mut changes = Vec::new();
        let mut render = Vec::new();
        let mut resident = Vec::new();
        for _ in 0..80 {
            let started = Instant::now();
            key(&mut app, KeyCode::Char('2'), KeyModifiers::NONE);
            key(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
            key(&mut app, KeyCode::Char('y'), KeyModifiers::CONTROL);
            key(&mut app, KeyCode::Char('n'), KeyModifiers::NONE);
            changes.push(started.elapsed().as_secs_f64() * 1000.);
            let started = Instant::now();
            terminal.draw(|f| app.render(f)).unwrap();
            render.push(started.elapsed().as_secs_f64() * 1000.);
            if let Some(rss) = perf::rss_mib() {
                resident.push(rss);
            }
        }
        perf::record("tui/merge/accept-undo-redo-next-100k", "ms", &changes);
        perf::record("tui/merge/render-100k", "ms", &render);
        if !resident.is_empty() {
            perf::record("tui/merge/stress-resident-100k", "MiB", &resident);
        }
        key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert!(std::fs::read(dir.path().join("result")).unwrap() != ours.as_bytes());
        let mut resize = Vec::new();
        for (w, h) in [(80, 24), (200, 60), (30, 10)] {
            terminal.backend_mut().resize(w, h);
            terminal.resize(Rect::new(0, 0, w, h)).unwrap();
            let started = Instant::now();
            app.handle(Event::Resize(w, h));
            terminal.draw(|f| app.render(f)).unwrap();
            resize.push(started.elapsed().as_secs_f64() * 1000.);
        }
        perf::record("tui/merge/resize", "ms", &resize);
        let started = Instant::now();
        for _ in 0..200 {
            let request = lenz::cli::parse(
                &["merge", "base", "ours", "theirs", "result"].map(str::to_owned),
                dir.path(),
            )
            .unwrap();
            let workspace = MergeWorkspace::open(request, dir.path().into());
            drop(workspace);
        }
        let fence = Arc::new(());
        let rx = lenz::work::submit(Arc::downgrade(&fence), || ());
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap(),
            Some(())
        );
        settle(&mut app);
        perf::record(
            "tui/merge/cancel-queued-200",
            "ms",
            &[started.elapsed().as_secs_f64() * 1000.],
        );
        drop(app);
        drop(terminal);
        if let Some(rss) = perf::rss_mib() {
            perf::record("tui/merge/cancel-settled-resident", "MiB", &[rss]);
        }
        let (binary_dir, mut picker) = open_fixture([b"\0base", b"\0ours\xff", b"\0theirs\xfe"]);
        let mut samples = Vec::new();
        for _ in 0..100 {
            let started = Instant::now();
            key(&mut picker, KeyCode::Char('2'), KeyModifiers::NONE);
            key(&mut picker, KeyCode::Char('z'), KeyModifiers::CONTROL);
            key(&mut picker, KeyCode::Char('y'), KeyModifiers::CONTROL);
            std::hint::black_box(screen(&mut picker));
            samples.push(started.elapsed().as_secs_f64() * 1000.);
        }
        key(&mut picker, KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(
            std::fs::read(binary_dir.path().join("result")).unwrap(),
            b"\0theirs\xfe"
        );
        perf::record("tui/merge/version-picker-input-render", "ms", &samples);
        let git_dir = conflicted_pair();
        let started = Instant::now();
        let mut list = Merger::from_repository(git_dir.path().into(), Vec::new());
        settle(&mut list);
        perf::record(
            "tui/merge/conflict-list-open",
            "ms",
            &[started.elapsed().as_secs_f64() * 1000.],
        );
        let mut times = Vec::new();
        for _ in 0..100 {
            let started = Instant::now();
            key(&mut list, KeyCode::Char('2'), KeyModifiers::NONE);
            key(&mut list, KeyCode::Char(']'), KeyModifiers::NONE);
            settle(&mut list);
            std::hint::black_box(screen(&mut list));
            times.push(started.elapsed().as_secs_f64() * 1000.);
        }
        assert_eq!(lenz::conflicts::list(git_dir.path()).unwrap().len(), 2);
        perf::record(
            "tui/merge/conflict-list-retained-switch-render",
            "ms",
            &times,
        );
    }

    #[test]
    fn merge_find_real_input_shows_query_matches_and_search_background() {
        let (_dir, mut app) =
            open_fixture([b"needle base\n", b"needle ours\n", b"needle theirs\n"]);
        key(&mut app, KeyCode::Char('/'), KeyModifiers::NONE);
        for c in "needle".chars() {
            key(&mut app, KeyCode::Char(c), KeyModifiers::NONE);
        }
        assert!(screen(&mut app).contains("Find Result: needle"));
        key(&mut app, KeyCode::Char('w'), KeyModifiers::ALT);
        assert!(screen(&mut app).contains("Find Result [W]: needle"), "Alt+W in merge find");
        key(&mut app, KeyCode::Char('w'), KeyModifiers::ALT);
        // The command menu's toggles act on merge find too.
        let case = lenz::catalog::Action::Interaction(lenz::commands::Interaction::FindCase);
        assert_eq!(lenz::catalog::Availability::disabled(&app, case), None);
        assert!(app.scope().find.is_some(), "the menu shows the toggles' state");
        app.run_action(case);
        assert!(screen(&mut app).contains("Find Result [Aa]: needle"), "Find: Match Case from the menu");
        app.run_action(case);
        let mut terminal = Terminal::new(TestBackend::new(120, 18)).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        assert!(
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .any(|c| c.bg == crate::chrome::color(crate::chrome::palette().found_bg)),
            "find hits must be visibly highlighted"
        );
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        key(&mut app, KeyCode::F(3), KeyModifiers::NONE);
        key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(!app.quit);
        assert!(!screen(&mut app).contains("Find Result: needle"));
    }
}

#[cfg(test)]
mod horizontal_tests {
    use super::*;
    use crossterm::event::{KeyEvent,MouseEvent};
    use ratatui::{Terminal,backend::TestBackend};
    #[test]
    fn horizontal_merge_switching_files_in_a_narrow_terminal_drops_old_widths(){
        let dir=tempfile::tempdir().unwrap();
        let git=|args:&[&str]|{let out=std::process::Command::new("git").args(["-c","user.name=Lenz test","-c","user.email=lenz@example.invalid"]).args(args).current_dir(dir.path()).output().unwrap();assert!(out.status.success(),"{}",String::from_utf8_lossy(&out.stderr));};
        git(&["init","-q","-b","main"]);
        for(name,text)in[("a.txt",format!("base{}\n","x".repeat(300))),("b.txt","base\n".into())]{std::fs::write(dir.path().join(name),text).unwrap();}
        git(&["add","."]);git(&["commit","-qm","base"]);git(&["branch","theirs"]);
        for(branch,prefix)in[("main","ours"),("theirs","theirs")]{git(&["checkout","-q",branch]);std::fs::write(dir.path().join("a.txt"),format!("{prefix}{}\n","x".repeat(300))).unwrap();std::fs::write(dir.path().join("b.txt"),format!("{prefix}\n")).unwrap();git(&["commit","-qam",prefix]);}
        git(&["checkout","-q","main"]);let out=std::process::Command::new("git").args(["merge","--no-commit","theirs"]).current_dir(dir.path()).output().unwrap();assert!(!out.status.success());
        let mut app=Merger::from_repository(dir.path().into(),vec![]);for _ in 0..1000{app.poll();if !app.busy(){break;}std::thread::sleep(std::time::Duration::from_millis(1));}
        let mut terminal=Terminal::new(TestBackend::new(60,20)).unwrap();terminal.draw(|f|app.render(f)).unwrap();assert_eq!(app.files_area.width,0);
        app.handle(Event::Mouse(MouseEvent{kind:MouseEventKind::ScrollRight,column:50,row:app.text_area.y+2,modifiers:KeyModifiers::NONE}));terminal.draw(|f|app.render(f)).unwrap();assert!(app.horizontal[2]>0);
        app.handle(Event::Key(KeyEvent::new(KeyCode::Char(']'),KeyModifiers::NONE)));for _ in 0..1000{app.poll();if !app.busy(){break;}std::thread::sleep(std::time::Duration::from_millis(1));}
        terminal.draw(|f|app.render(f)).unwrap();assert_eq!(app.workspace.current,1);assert_eq!(app.horizontal,[0;3]);
        app.handle(Event::Mouse(MouseEvent{kind:MouseEventKind::ScrollRight,column:50,row:app.text_area.y+2,modifiers:KeyModifiers::NONE}));terminal.draw(|f|app.render(f)).unwrap();assert_eq!(app.horizontal,[0;3]);
    }
    #[test]
    fn horizontal_merge_wheel_targets_one_pane_and_is_blocked_by_a_menu(){
        let dir=tempfile::tempdir().unwrap();
        for(name,prefix)in[("base","BASE"),("ours","OURS"),("theirs","THEIRS"),("result","BASE")]{std::fs::write(dir.path().join(name),format!("{prefix}{}\n","x".repeat(400))).unwrap();}
        let req=lenz::cli::parse(&["merge".into(),"base".into(),"ours".into(),"theirs".into(),"result".into()],dir.path()).unwrap();
        let mut app=Merger::open(req,dir.path().into());for _ in 0..1000{app.poll();if !app.busy(){break;}std::thread::sleep(std::time::Duration::from_millis(1));}
        let mut terminal=Terminal::new(TestBackend::new(120,20)).unwrap();terminal.draw(|f|app.render(f)).unwrap();
        let x=app.text_area.right()-10;let y=app.text_area.y+2;
        app.handle(Event::Mouse(MouseEvent{kind:MouseEventKind::ScrollRight,column:x,row:y,modifiers:KeyModifiers::NONE}));terminal.draw(|f|app.render(f)).unwrap();
        let text=terminal.backend().buffer().content.iter().map(|c|c.symbol()).collect::<String>();assert!(text.contains("OURS"));assert!(!text.contains("THEIRS"),"horizontal wheel must scroll the pointed merge pane");
        app.handle(Event::Mouse(MouseEvent{kind:MouseEventKind::Down(MouseButton::Left),column:app.text_area.x+10,row:y,modifiers:KeyModifiers::NONE}));
        app.handle(Event::Key(KeyEvent::new(KeyCode::Right,KeyModifiers::CONTROL|KeyModifiers::ALT)));assert_eq!(app.horizontal,[4,0,4]);
        let before=std::fs::read(dir.path().join("result")).unwrap();app.handle(Event::Key(KeyEvent::new(KeyCode::F(10),KeyModifiers::SHIFT)));
        app.handle(Event::Mouse(MouseEvent{kind:MouseEventKind::ScrollRight,column:x,row:y,modifiers:KeyModifiers::NONE}));
        app.handle(Event::Key(KeyEvent::new(KeyCode::Right,KeyModifiers::CONTROL|KeyModifiers::ALT)));assert_eq!(app.horizontal,[4,0,4]);
        assert_eq!(std::fs::read(dir.path().join("result")).unwrap(),before);
    }
}
