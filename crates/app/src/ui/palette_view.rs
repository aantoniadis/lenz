//! The command palette over the shared catalog: what the window shows, dispatch, keys, mouse and layout.
use super::*;
use crate::catalog::{self, Action, Availability, HelpAction, PrAction, RepoAction, Scope, Surface};
use crate::palette::{Outcome, Palette, PaletteCommand};
use std::borrow::Cow;

impl Viewer {
    /// What the window shows now, for the catalog.
    pub(super) fn scope(&self)->Scope{
        let surface=if self.merge.is_some(){Surface::Merge}else if self.picker_v.is_some(){Surface::VersionPicker}else{Surface::Comparison};
        // a loading file may still be Markdown: list View: Rendered, disabled with its reason
        let markdown=self.comparison.presentation_disabled(crate::markdown::PresentationMode::Rendered,self.doc().map(|d|&d.text))!=Some("Not a Markdown comparison");
        Scope{surface,sticky:self.sticky_on,unified:self.comparison.view.unified(),repository:self.repo.is_some(),focused:self.layout.focused(self.repo.as_ref()),pr_tab:self.repo.as_ref().and_then(|r|r.pr.as_ref()).and_then(|p|p.details()).map(|d|d.tab),pr:self.repo.as_ref().is_some_and(|r|r.pr.is_some()),image:self.image_doc().is_some(),markdown,
              options:self.comparison.options,scroll:self.comparison.view.mode(),presentation:self.comparison.markdown_mode(),
              context_hidden:self.comparison.collapse,blame:self.blame_on,image_mode:self.iview.state().mode(),markdown_layout:self.comparison.markdown_layout(),find:self.find_open().then_some(self.comparison.find_options)}
    }

    pub(super) fn open_palette(&mut self,cx:&mut Context<Self>){
        if self.modal().is_some_and(|m|m!=Modal::Menu){return;}
        self.menu=None;
        self.command_palette=Some(Palette::new(catalog::entries(&self.scope()),&*self));
        cx.notify();
    }

    /// Run a catalog action; every variant is handled here and nowhere else. The palette closes first, so no row
    /// index outlives a comparison the action replaces.
    /// Why the one-column layout cannot show now (the core allows it; what is on screen may not).
    pub(super) fn unified_disabled(&self)->Option<&'static str>{
        if self.merge.is_some()||self.picker_v.is_some() {Some("Unified is for comparisons")}
        else {crate::commands::unified_disabled(self.image_doc().is_some(),self.comparison.markdown_mode()==crate::markdown::PresentationMode::Rendered)}
    }

    pub(super) fn run_action(&mut self,action:Action,window:&mut Window,cx:&mut Context<Self>){
        // an action on the diff or a file shows the diff first
        let on_diff=matches!(action,Action::Comparison(_)|Action::Edit(_)|Action::File(_)|Action::Image(_)
            |Action::Interaction(_)|Action::Repository(crate::catalog::RepoAction::Back|crate::catalog::RepoAction::Forward)
            |Action::Pr(PrAction::MarkViewedNext|PrAction::Threads|PrAction::OutdatedThreads))
            &&!matches!(action,Action::Interaction(crate::commands::Interaction::About|crate::commands::Interaction::ChangeTheme|crate::commands::Interaction::ReportLastError|crate::commands::Interaction::ResetPaneSizes));
        if on_diff&&self.pr_details_shown(){self.pr_command(crate::pr_session::PrCommand::Close,cx);}
        use crate::repository_session::RepositoryCommand as R;
        self.command_palette=None;
        match action {
            Action::Comparison(command@crate::commands::Command::SetDiffLayout(crate::commands::DiffLayout::Unified)) if let Some(why)=self.unified_disabled() => {let _=command;self.toast(why.into(),cx);}
            Action::Comparison(command)=>self.comparison_command(command,cx),
            Action::Interaction(command)=>self.interaction(command,cx),
            Action::Edit(command)=>self.edit_menu(command,cx),
            Action::Folder(action)=>self.folder_action(action,cx),
            Action::File(file)=>{let set=self.set.clone();self.file_action(file,self.comparison.current,&set,cx);},
            Action::Image(index)=>if let Some(entry)=crate::image_session::image_commands().get(index){let viewport=self.image_viewport(window);if let Some(doc)=self.image_doc(){self.iview.command(&doc,entry.command,viewport);}},
            Action::Repository(RepoAction::Refresh)=>self.refresh_repository(cx),
            Action::Repository(RepoAction::Branches)=>self.branch_menu(self.panes.viewport[0]/3.,cx),
            Action::Repository(RepoAction::Filter)=>{
                // from Focus: the panels it hid come back first
                if self.layout.focused(self.repo.as_ref()){self.layout_command(crate::layout::LayoutCommand::LeaveFocus,cx);}
                if self.repo.as_ref().is_some_and(|r|!r.picker.shown||!r.picker.log_shown){self.layout_command(crate::layout::LayoutCommand::ToggleHistory,cx);}
                if let Some(r)=self.repo.as_mut(){r.execute(R::Focus(crate::repository_session::Pane::Log));r.picker.filter_editing=true;}
            }
            Action::Repository(RepoAction::OpenWorktree)=>match self.repo.as_ref().map(|r|r.selected_worktree()) {
                Some(Ok(path))=>self.new_window(&path,cx),
                Some(Err(why))=>self.toast(why.into(),cx),
                None=>{}
            },
            Action::Repository(action @ (RepoAction::StagedChanges|RepoAction::CopyUrl|RepoAction::OpenUrl|RepoAction::LocalChanges|RepoAction::AllBranches
                |RepoAction::CurrentBranch|RepoAction::CompareBranch{..}|RepoAction::CompareCommit|RepoAction::NextParent|RepoAction::FileHistory
                |RepoAction::WriteGraph|RepoAction::CopyHash|RepoAction::CopySubject|RepoAction::CopyDetails|RepoAction::ShowInGraph|RepoAction::ToggleMessage|RepoAction::BranchChanges))=>{
                let path=self.set.comparisons.get(self.comparison.current).map_or(String::new(),|c|c.repository_path().to_owned());
                match self.repo.as_ref().map(|r|r.action_command(action,&path)) {
                    Some(Ok(Some(command)))=>self.repository_command(command,cx),
                    Some(Err(why))=>self.toast(why.into(),cx),
                    Some(Ok(None))|None=>{}
                }
            }
            Action::Repository(RepoAction::NewBranch)=>self.git_prompt=Some(super::checkout::GitPrompt::NewBranch{from:None,name:String::new(),checkout:true,error:None}),
            Action::Repository(RepoAction::CheckoutRevision)=>self.git_prompt=Some(super::checkout::GitPrompt::Revision{input:String::new(),error:None}),
            Action::Repository(RepoAction::Back)=>self.navigate(false,cx),
            Action::Repository(RepoAction::Forward)=>self.navigate(true,cx),
            Action::Pr(PrAction::Details(tab))=>self.pr_command(crate::pr_session::PrCommand::Show(tab),cx),
            Action::Pr(PrAction::Diff)=>self.pr_command(crate::pr_session::PrCommand::Close,cx),
            Action::Pr(PrAction::MarkViewedNext)=>{self.pr_key("v",false,cx);},
            Action::Pr(PrAction::Refresh)=>{self.pr_key("r",false,cx);},
            Action::Pr(PrAction::OutdatedThreads)=>{self.pr_key("t",true,cx);},
            Action::Pr(PrAction::Threads)=>{let path=self.set.comparisons.get(self.comparison.current).map_or(String::new(),|c|c.path.clone());self.repository_command(R::Threads{path,outdated:false},cx);},
            Action::Pr(PrAction::CheckHead)=>self.repository_command(R::CheckPrHead,cx),
            Action::Merge(act)=>self.merge_act(act,cx),
            Action::Pick(act)=>{self.picker_v_act(act,cx);},
            Action::Layout(command)=>self.layout_command(command,cx),
            Action::Help(HelpAction::Shortcuts)=>{self.help=true;self.help_scroll=0;},
            Action::Help(HelpAction::DiffOptions)=>self.open_diff_options(cx),
            Action::Help(HelpAction::Settings)=>self.open_settings(cx),
        }
        cx.notify();
    }

    /// Open the settings file with the system's default app, first creating the commented template when there is none.
    pub(super) fn open_settings(&mut self,cx:&mut Context<Self>){
        let path=match crate::settings::ensure_template(){Ok(path)=>path,Err(e)=>return self.toast(format!("Cannot create settings: {e}"),cx)};
        #[cfg(all(test,feature="gui-test"))]
        { self.toast(format!("Settings: {}",path.display()),cx); }
        #[cfg(not(all(test,feature="gui-test")))]
        {
            let plan=crate::open::plan(&crate::open::Host::current(),&crate::open::Target{file:&path,binary:false,line:None,column:None,cwd:None},true);
            if let Err(e)=crate::open::spawn(&plan.launch){return self.toast(format!("Cannot start {}: {e}",plan.launch.program),cx);}
            self.toast(plan.toast.unwrap_or_else(||format!("Settings apply on the next start: {}",path.display())),cx);
        }
    }

    /// The ☰ menu under its button.
    pub(super) fn open_app_menu(&mut self,x:f32,cx:&mut Context<Self>){self.open_entry_menu(catalog::app_menu(),x,cx);}

    /// A toolbar dropdown of catalog entries under its button; again closes it.
    pub(super) fn open_entry_menu(&mut self,entries:Vec<catalog::Entry>,x:f32,cx:&mut Context<Self>){
        if self.modal().is_some_and(|m|m!=Modal::Menu){return;}
        if self.menu.take().is_some(){cx.notify();return;}
        // current values carry a ✓ when the list has any
        let marks=entries.iter().any(|e|e.current);
        let items=entries.into_iter().map(|e|{
            let mark=if !marks {""} else if e.current {"✓ "} else {"  "};
            let label=Availability::disabled(&*self,e.action).map_or_else(||format!("{mark}{}",e.label),|why|format!("{mark}{} ({why})",e.label));
            (label,MenuAction::Run(e.action))
        }).collect();
        self.show_menu(Menu{x,y:toolbar_h(),items,hover:None});cx.notify();
    }

    /// Back (`forward` false) or Forward: the session restores the log and selection; the file and line follow once
    /// the comparison settles, as a launch's saved place does.
    pub(super) fn navigate(&mut self,forward:bool,cx:&mut Context<Self>){
        if self.pr_details_shown(){self.pr_command(crate::pr_session::PrCommand::Close,cx);}
        let Some(r)=self.repo.as_mut() else {return};
        if forward {r.go_forward()} else {r.go_back()}
        if let Some(restore)=r.take_nav_restore() {self.pending_place=Some(restore);}
        self.present_repository(cx);
        self.poll_repository(cx);
        cx.notify();
    }

    /// x, y, width and the number of entry rows (the query line comes first).
    pub(super) fn palette_layout(&self)->(f32,f32,f32,usize){
        let [vw,vh]=self.panes.viewport;
        let width=(72.0*self.char_w).min((vw-8.).max(0.));
        let y=(toolbar_h()+4.).min((vh-line_h()-8.).max(0.));
        let rows=(((vh-y-8.).max(0.)/line_h()).floor() as usize).saturating_sub(1).min(14);
        (((vw-width)/2.).max(0.),y,width,rows)
    }

    pub(super) fn palette_key(&mut self,ev:&KeyDownEvent,window:&mut Window,cx:&mut Context<Self>)->bool{
        let (_,_,_,rows)=self.palette_layout();
        let Some(p)=self.command_palette.as_mut() else {return false};
        let ks=&ev.keystroke;
        let page=rows.max(1) as isize;
        match ks.key.as_str() {
            "escape"=>self.command_palette=None,
            "enter"=>{let outcome=p.run();self.palette_outcome(outcome,window,cx);return true;}
            "down"=>p.command(PaletteCommand::Move(1)),"up"=>p.command(PaletteCommand::Move(-1)),
            "pagedown"=>p.command(PaletteCommand::Move(page)),"pageup"=>p.command(PaletteCommand::Move(-page)),
            "home"=>p.command(PaletteCommand::Home),"end"=>p.command(PaletteCommand::End),
            "backspace"=>p.command(PaletteCommand::Backspace),
            _ if !(ks.modifiers.control||ks.modifiers.platform||ks.modifiers.alt)=>{
                if let Some(t)=ks.key_char.clone().or_else(||(ks.key.chars().count()==1).then(||ks.key.clone())){p.command(PaletteCommand::Type(t));}
            }
            _=>{}
        }
        cx.notify();true
    }

    fn palette_outcome(&mut self,outcome:Outcome,window:&mut Window,cx:&mut Context<Self>){
        match outcome {
            Outcome::Run(action)=>self.run_action(action,window,cx),
            Outcome::Disabled(reason)=>self.toast(reason.into_owned(),cx),
            Outcome::Nothing=>{}
        }
        cx.notify();
    }

    /// A click on the palette runs the row under it; a click outside closes it. True when the palette was open.
    pub(super) fn palette_click(&mut self,x:f32,y:f32,window:&mut Window,cx:&mut Context<Self>)->bool{
        let (px_,py,w,rows)=self.palette_layout();
        let Some(p)=self.command_palette.as_mut() else {return false};
        let inside=x>=px_&&x<px_+w&&y>=py&&y<py+(rows+1) as f32*line_h()+8.;
        if !inside {self.command_palette=None;cx.notify();return true;}
        let range=p.window(rows);
        let row=((y-py-4.)/line_h()).floor()-1.;
        if row>=0. && (row as usize)<range.len() {
            p.command(PaletteCommand::Select(range.start+row as usize));
            let outcome=p.run();self.palette_outcome(outcome,window,cx);
        }
        cx.notify();true
    }

    /// Lines to paint: query, then (label, group and shortcut, dim, selected) per visible row, then the selected row's
    /// disabled reason.
    pub(super) fn palette_paint(&mut self)->Option<PalettePaint>{
        let (x,y,width,rows)=self.palette_layout();
        let p=self.command_palette.as_mut()?;
        let range=p.window(rows);let selected=p.selected();
        let lines=p.rows().enumerate().skip(range.start).take(range.len()).map(|(i,row)|{
            let mark=if row.entry.current{"✓ "}else{"  "};
            let reason=row.disabled.as_deref().filter(|_|selected==Some(i)).map_or(String::new(),|why|format!(" — {why}"));
            // macOS: the palette names the Command key the bindings accept
            let key=row.entry.shortcut.map(|k|if cfg!(target_os="macos") {k.replace("ctrl-","cmd-")} else {k.to_string()});
            let right=[Some(row.entry.group.label().to_string()),key].into_iter().flatten().collect::<Vec<_>>().join("  ");
            (format!("{mark}{}{reason}",row.entry.label),right,row.disabled.is_some(),selected==Some(i))
        }).collect();
        let empty=p.is_empty();
        Some(PalettePaint{x,y,width,query:format!("› {}▏",p.query()),lines,empty})
    }
}

pub(super) struct PalettePaint { pub x:f32, pub y:f32, pub width:f32, pub query:String, pub lines:Vec<(String,String,bool,bool)>, pub empty:bool }

impl Availability for Viewer {
    fn disabled(&self,action:Action)->Option<Cow<'static,str>>{
        match action {
            Action::Comparison(crate::commands::Command::SetPresentation(mode))=>self.comparison.presentation_disabled(mode,self.doc().map(|d|&d.text)).map(Cow::Borrowed),
            Action::Comparison(crate::commands::Command::Preview(_))=>self.comparison.presentation_disabled(crate::markdown::PresentationMode::Rendered,self.doc().map(|d|&d.text)).map(Cow::Borrowed),
            Action::Comparison(crate::commands::Command::SetDiffLayout(crate::commands::DiffLayout::Unified))=>self.unified_disabled().map(Cow::Borrowed),
            Action::Comparison(command)=>crate::commands::disabled(command,&self.comparison.view,self.doc().is_none(),self.comparison.current,self.set.comparisons.len(),self.large()).map(Cow::Borrowed),
            Action::Interaction(command)=>self.interaction_disabled(command).map(Cow::Borrowed),
            Action::Folder(action)=>self.comparison.folders.disabled(action,Some(&self.set),self.comparison.current).map(Cow::Borrowed),
            Action::Edit(command)=>self.edit_disabled(command).map(Cow::Owned),
            Action::File(file)=>{
                let why=if file==crate::file_actions::FileAction::UndoLenz{self.comparison.markdown_edit_disabled().map(str::to_string)}else{None};
                why.or_else(||self.file_disabled(file,self.comparison.current)).map(Cow::Owned)
            }
            Action::Image(index)=>crate::image_session::image_commands().get(index).zip(self.image_doc()).and_then(|(e,doc)|e.disabled(&doc,self.iview.state())).map(Cow::Borrowed),
            Action::Repository(RepoAction::Refresh|RepoAction::Branches|RepoAction::NewBranch|RepoAction::CheckoutRevision)=>None,
            Action::Repository(RepoAction::Filter)=>None,
            Action::Repository(action @ (RepoAction::StagedChanges|RepoAction::CopyUrl|RepoAction::OpenUrl|RepoAction::LocalChanges|RepoAction::AllBranches
                |RepoAction::CurrentBranch|RepoAction::CompareBranch{..}|RepoAction::CompareCommit|RepoAction::NextParent|RepoAction::FileHistory
                |RepoAction::WriteGraph|RepoAction::CopyHash|RepoAction::CopySubject|RepoAction::CopyDetails|RepoAction::ShowInGraph|RepoAction::OpenWorktree|RepoAction::ToggleMessage|RepoAction::BranchChanges))=>{
                let path=self.set.comparisons.get(self.comparison.current).map_or("",|c|c.repository_path());
                self.repo.as_ref().and_then(|r|r.action_command(action,path).err()).map(Cow::Borrowed)
            }
            Action::Repository(RepoAction::Back)=>(!self.repo.as_ref().is_some_and(|r|r.can_go_back())).then_some(Cow::Borrowed("Nowhere to go back to")),
            Action::Repository(RepoAction::Forward)=>(!self.repo.as_ref().is_some_and(|r|r.can_go_forward())).then_some(Cow::Borrowed("Nowhere to go forward to")),
            Action::Pr(_)=>None,
            Action::Merge(act)=>self.merge.as_ref().and_then(|s|s.disabled(act)).map(Cow::Borrowed),
            Action::Pick(act)=>self.picker_v.as_ref().and_then(|s|s.disabled(act)).map(Cow::Borrowed),
            Action::Layout(command)=>self.layout.disabled(command,self.repo.as_ref()).map(Cow::Borrowed),
            Action::Help(_)=>None,
        }
    }
}
