use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind, MouseButton};
use lenz::{commands::{self, Command}, navigator, session::Session, view::Side};
use unicode_segmentation::UnicodeSegmentation;

fn caret_scroll(column:usize,offset:usize,width:usize)->usize{
    if width==0{return column;}
    let margin=2.min(width.saturating_sub(1)/2);
    if column<offset+margin{column.saturating_sub(margin)}else if column>=offset+width-margin{column.saturating_sub(width-margin-1)}else{offset}
}

#[derive(Clone, Copy, PartialEq,Debug)]
pub(crate) enum Prompt { Search, Line, Files, Help, Applied }

#[derive(Clone, Copy)]
pub(crate) enum ToolbarAction { Run(lenz::catalog::Action), Palette, Icons }

pub struct App {
    /// The restored position, applied once the repository settles; then the place being recorded.
    pub(crate) pending_place:Option<lenz::place::Restore>,
    place_recorder:Option<lenz::place::Recorder>,
    place_root:Option<std::path::PathBuf>,
    /// `[view] markdown = "rendered"`: switch to Rendered once the first document has loaded, when it is Markdown.
    start_rendered:bool,
    pub(crate) icons:crate::icons::Icons,
    pub(crate) pane_ratio:f32,
    pub(crate) reset_sizes_requested:bool,
    pub(crate) applied_height:Option<f32>,
    pub(crate) resizer:crate::panes::Resizer,
    pub palette:std::sync::Arc<lenz::theme::Palette>,
    pub theme_revision:u64,
    pub theme_request:bool,
    pub session: Session,
    pub(crate) context:crate::context::Menu,
    pub(crate) popup_hits:Vec<(usize,ratatui::layout::Rect)>,
    pub(crate) popup_rect:ratatui::layout::Rect,
    pub(crate) about: Option<crate::about::About>,
    pub(crate) message:Option<crate::help::Message>,
    report_request: lenz::diagnostics::ReportRequest,
    pub(crate) toolbar:crate::chrome::Bar<ToolbarAction>,
    pub(crate) navigator:crate::files::Files,
    /// The Folder Summary's scroll and area; its content is the session's.
    pub(crate) summary:crate::summary::SummaryPane,
    pub quit: bool,
    pub repository: Option<crate::repository::Repository>,
    pub(crate) visual: crate::visual::Visual,
    pub(crate) markdown: crate::markdown::Adapter,
    visual_revision: Option<(u64,usize)>,
    visual_document: Option<std::sync::Weak<lenz::document::TextDocument>>,
    pub(crate) blame: [bool;2],
    pub(crate) command_palette: Option<lenz::palette::Palette>,
    /// Files visibility and Focus; the repository session holds Branches and History.
    pub(crate) layout: lenz::layout::Layout,
    /// The pane header's mode cells as last drawn.
    pub(crate) header_switch: Vec<(ratatui::layout::Rect,lenz::catalog::Action)>,
    /// The bottom line's panel switches, as last drawn.
    pub(crate) panel_switch: Vec<(ratatui::layout::Rect,lenz::layout::LayoutCommand)>,
    /// The bottom bar's "F10 commands": a click opens the palette, as the GUI's "Commands" does.
    pub(crate) palette_hint: ratatui::layout::Rect,
    /// Repository views: the comparison's cells as last drawn, and when a new Change Set replaced it, so the old one
    /// stays up until the new one is read (at most the shared SWAP_WAIT) instead of blinking to Loading.
    held_diff: Option<ratatui::buffer::Buffer>,
    swap_at: Option<std::time::Instant>,
    pub(crate) selection: Option<lenz::selection::Selection>,
    pub(crate) popup_area:ratatui::layout::Rect,
    pub(crate) diff_area: ratatui::layout::Rect,
    auto_repository:bool,
    pub(crate) set_search:Option<crate::search::Search>,
    pub conflicts: Option<std::path::PathBuf>,
    pub new_view:Option<(lenz::cli::Request,std::path::PathBuf)>,
    comparison_name:Option<String>,initial_line:Option<u32>,
    mouse_click:Option<(std::time::Instant,Side,lenz::selection::Pos,u8)>,
    /// The last file clicked in Files, for a double click.
    files_click:Option<(std::time::Instant,usize)>,
    /// Focus's commit message panel, above the Files list.
    pub(crate) commit_panel:crate::commit_panel::CommitPanel,
    pending_hit:Option<lenz::find::Hit>,
    #[cfg_attr(test,allow(dead_code))]
    open_token:std::sync::Arc<()>,
    #[cfg(test)]
    pub(crate) inspected_position:Option<(usize,Side,usize,usize)>,
    opening_editor:Option<std::sync::mpsc::Receiver<Option<Result<lenz::open::Plan,String>>>>,
    pub(crate) focus: Side,
    pub(crate) horizontal: [usize;2],
    columns: [Option<usize>;2],
    pub(crate) prompt: Option<Prompt>,
    pub(crate) input: String,
    pub(crate) selected: usize,
    pub(crate) size: (u16, u16),
}

impl App {
    pub fn new(session: Session) -> Self {
        Self { command_palette:None,layout:Default::default(),header_switch:Vec::new(),panel_switch:Vec::new(),palette_hint:Default::default(),held_diff:None,swap_at:None,pending_place:None,place_recorder:None,place_root:None,start_rendered:false,icons:crate::icons::Icons::from_env(),reset_sizes_requested:false,pane_ratio:0.5,applied_height:None,resizer:Default::default(),palette:std::sync::Arc::new(*crate::chrome::initial_palette()),theme_revision:1,theme_request:false,session, context:Default::default(),popup_hits:Vec::new(),popup_rect:Default::default(), about:None,message:None, report_request:Default::default(), toolbar:Default::default(), navigator:Default::default(), summary:Default::default(), quit: false, repository:None, visual:crate::visual::Visual::with_sticky(lenz::settings::get().view.sticky_scroll),markdown:Default::default(), visual_revision:None, visual_document:None, blame:[false;2], selection:None, diff_area:ratatui::layout::Rect::new(0,0,80,24),popup_area:ratatui::layout::Rect::new(0,0,80,24), conflicts:None,new_view:None,comparison_name:None,initial_line:None,mouse_click:None,files_click:None,commit_panel:Default::default(), auto_repository:false,set_search:None,pending_hit:None,open_token:std::sync::Arc::new(()),opening_editor:None, #[cfg(test)] inspected_position:None, focus: Side::Right, horizontal: [0;2], columns:[None;2], prompt: None,
            input: String::new(), selected: 0, size: (80, 24) }
    }

    pub fn open(mut request:lenz::cli::Request,cwd:std::path::PathBuf)->Self {
        // tests never read or write the user's own places
        let launch=if cfg!(test){None}else{lenz::place::for_launch(&mut request,&cwd)};
        Self::open_place(request,cwd,launch)
    }
    #[cfg(test)]
    pub(crate) fn open_with_place(mut request:lenz::cli::Request,cwd:std::path::PathBuf,state_home:&std::path::Path)->Self {
        let launch=lenz::place::for_launch_in(&mut request,&cwd,true,state_home);
        Self::open_place(request,cwd,launch)
    }
    /// Plain `lenz` and `lenz log` reopen at the saved place and record the place as it changes (see [`lenz::place`]).
    fn open_place(request:lenz::cli::Request,cwd:std::path::PathBuf,launch:Option<lenz::place::Launch>)->Self {
        let mut app=Self::open_request(request,cwd);
        if let Some(launch)=launch {
            if let (Some(commit),Some(repo))=(launch.restore.as_ref().and_then(|r|r.commit.clone()),app.repository.as_mut()) {
                repo.core.want_commit(commit);
            }
            app.pending_place=launch.restore;
            app.place_recorder=Some(lenz::place::Recorder::new(launch.file));
            app.place_root=Some(launch.root);
        }
        app
    }
    fn open_request(request:lenz::cli::Request,cwd:std::path::PathBuf)->Self {
        let repository=matches!(request.spec,lenz::changeset::Spec::Git{..}).then(||crate::repository::Repository::open(lenz::cli::Request{
            launch:request.launch.clone(),spec:request.spec.clone(),options:request.options,name:request.name.clone(),scroll:request.scroll,blame:request.blame,line:request.line,markdown:request.markdown
        },cwd.clone()));
        let auto_repository=matches!(request.launch,lenz::cli::Launch::Diff)&&matches!(&request.spec,lenz::changeset::Spec::Git{cached:false,revs,paths,..}
        if revs.is_empty()&&paths.is_empty());
        let about=request.launch==lenz::cli::Launch::About;
        let blame=request.blame;let initial_settings=(request.name.clone(),request.line);let start_rendered=request.markdown==lenz::markdown::PresentationMode::Rendered;
        let mut app=Self::new(Session::open(request,cwd));app.repository=repository;app.auto_repository=auto_repository;app.blame=[blame;2];app.comparison_name=initial_settings.0;app.initial_line=initial_settings.1;app.start_rendered=start_rendered;if about{app.about=Some(Default::default());}app
    }
    /// Opens the restored file at its line once the repository has settled, then records where the user is.
    fn restore_and_record_place(&mut self)->bool {
        let mut changed=false;
        let settled=self.repository.as_ref().is_none_or(|r|r.core.settled());
        // a commit the log never reached (amended or rebased away) leaves the normal start, not its file in another commit
        if settled && !self.session.loading() && let Some(pending)=self.pending_place.take()
            && pending.commit.as_deref().is_none_or(|c|self.repository.as_ref().is_some_and(|r|r.core.restored(c)))
            && let Some(index)=pending.file.as_deref().and_then(|f|self.session.comparisons().iter().position(|c|c.path==f))
        {
            self.session.open_file(index);
            if let Some(line)=pending.line { self.session.open_at(Side::Right,line); }
            changed=true;
        }
        if self.pending_place.is_none() && let (Some(recorder),Some(root))=(self.place_recorder.as_mut(),self.place_root.clone()) {
            let source=self.repository.as_ref().and_then(|r|r.core.place_source()).unwrap_or(lenz::place::Source::Local);
            let file=self.session.comparisons().get(self.session.current()).map(|c|c.path.clone());
            let line=self.session.document().is_some().then(||self.session.view().anchor_line_on(Side::Right) as u32+1);
            recorder.record(lenz::place::Place{version:lenz::place::VERSION,root,source,file,line});
        }
        changed
    }
    pub fn poll(&mut self)->bool {
        let mut changed=self.session.poll();
        // a tab the session opened (a PR's Conversation) shows in Focus
        self.sync_pr_tab();
        if self.start_rendered && !self.session.loading() && let Some(doc)=self.session.document_arc() {
            self.start_rendered=false;
            // a quiet default: files that cannot be rendered stay in Source without a notice
            if self.session.presentation_disabled(lenz::markdown::PresentationMode::Rendered,Some(&doc)).is_none() {
                self.session.execute(lenz::commands::Command::SetPresentation(lenz::markdown::PresentationMode::Rendered));changed=true;
            } else {
                self.session.prefer_presentation(lenz::markdown::PresentationMode::Rendered);
            }
        }
        if let Some(result)=self.report_request.poll(){changed=true;match result{Ok(report)=>{self.session.set_notice(format!("Review report: {}",report.path.display()));self.open_url(&report.issue_url);},Err(error)=>self.session.set_notice(error.to_string())}}
        if let Some(repo)=&mut self.repository {
            changed|=repo.poll();
            if let Some((set,spec))=repo.take_change_set(){self.swap_at=Some(std::time::Instant::now());let preserve=repo.core.take_refresh_placement();if self.session.has_dirty(){let refused=spec!=*self.session.spec()||!preserve;self.session.replace_set(set,spec,preserve);if refused{self.navigator.focused=false;repo.core.execute(lenz::repository_session::RepositoryCommand::Focus(lenz::repository_session::Pane::Diff));}}else if preserve{self.session.replace_set(set,spec,true);}else{let name=self.comparison_name.clone();let line=self.initial_line.take();let mut next=Session::from_set_named(set,spec,self.session.options(),self.session.view().mode(),name,line);next.markdown_inherit_presentation(&self.session);self.session=next;}self.visual_revision=None;self.selection=None;self.set_search=None;self.pending_hit=None;changed=true;}
            let (path,history)=self.session.comparisons().get(self.session.current()).map_or(("",""),|c|(c.path.as_str(),c.repository_path()));repo.set_current(self.session.current(),path,history);
            let effects=repo.take_effects();for effect in effects{self.repository_effect(effect);changed=true;}
        }
        if self.auto_repository && !self.session.loading(){self.auto_repository=false;
            if let Some(set)=self.session.change_set() && let Some(root)=set.git_root(){
                if set.comparisons.iter().any(|c|c.status==lenz::changeset::Status::Conflicted){self.conflicts=Some(root.to_path_buf());}
                else if set.comparisons.is_empty() && let Ok(req)=lenz::cli::parse(&["log".into()],root){self.repository=Some(crate::repository::Repository::open(req,root.to_path_buf()));changed=true;}
            }
        }
        if let Some(search)=&mut self.set_search{changed|=search.poll();if let Some(hit)=search.take_hit(){self.session.open_file(hit.index);self.session.open_at(hit.side,hit.line+1);self.focus=hit.side;self.pending_hit=Some(hit);changed=true;}
        if search.closed{self.set_search=None;}}
        let revision=(self.session.revision(),self.session.current());
        if self.visual_revision!=Some(revision) {
            let same_document=self.visual_revision.is_some_and(|(_,index)|index==revision.1)&&self.visual_document.as_ref().and_then(std::sync::Weak::upgrade).zip(self.session.document_arc()).is_some_and(|(old,new)|std::sync::Arc::ptr_eq(&old,&new));
            if same_document {self.visual_revision=Some(revision);}else{
            self.markdown.clear();
            self.columns=[None;2];
            if self.visual_revision.is_none_or(|(_,index)|index!=revision.1){self.horizontal=[0;2];}
            self.visual.clear();self.selection=None;
            self.visual_document=None;
            if let (Some(set),Some(doc))=(self.session.change_set(),self.session.document_arc()) {
                self.visual_document=Some(std::sync::Arc::downgrade(&doc));
                self.visual.load_prepared(set,self.session.current(),doc,self.session.spec(),self.session.image_result());self.visual_revision=Some(revision);
                for side in [Side::Left,Side::Right]{if self.blame[side as usize] && self.visual.blame_available(side){self.visual.toggle_blame(side);}}changed=true;
            }
            }
        }
        if !self.session.loading() && let Some(hit)=self.pending_hit.take()&&hit.index==self.session.current() && self.session.markdown_mode()!=lenz::markdown::PresentationMode::Rendered {self.selection=Some(lenz::selection::Selection{side:hit.side,anchor:lenz::selection::Pos::new(hit.line,hit.range.start),head:lenz::selection::Pos::new(hit.line,hit.range.end)});}
        if let Some(job)=&self.opening_editor && let Ok(result)=job.try_recv(){self.opening_editor=None;changed=true;match result{Some(Ok(plan))=>{if let Err(e)=lenz::open::spawn(&plan.launch){self.session.set_notice(format!("Cannot start editor: {e}"));}else if let Some(note)=plan.toast{self.session.set_notice(note);}},Some(Err(e))=>self.session.set_notice(e),None=>{}}}
        if self.session.markdown_mode()!=lenz::markdown::PresentationMode::Rendered {self.markdown.clear();}
        changed|=self.restore_and_record_place();
        // the held comparison's time is up while the new one is still being read: show Loading now
        if self.session.loading()&&self.swap_at.is_some_and(|t|t.elapsed()>=lenz::repository_session::SWAP_WAIT){self.swap_at=None;changed=true;}
        changed|=self.navigator.poll_search();changed|=self.markdown.poll();changed|=self.visual.poll();changed
    }
    pub fn failed(&self)->bool{self.session.failed()||self.repository.as_ref().is_some_and(|r|r.core.failed)}
    pub fn busy(&self)->bool {self.navigator.busy()||self.report_request.busy()||self.session.busy()||self.markdown.busy()||self.visual.busy()||self.opening_editor.is_some()||self.repository.as_ref().is_some_and(|r|r.busy())||self.set_search.as_ref().is_some_and(|s|s.busy())}
    pub(crate) fn repository_effect(&mut self,effect:lenz::repository_session::RepositoryEffect){
        use lenz::repository_session::RepositoryEffect::*;
        match effect {
            Copy(text)=>self.copy(&text),OpenUrl(url)=>self.open_url(&url),
            OpenFile(index)=>self.session.open_file(index),Conflicts=>self.conflicts=self.session.change_set().and_then(|s|s.git_root().map(std::path::Path::to_path_buf)),
            OpenWorktree(cwd)=>{if let Ok(req)=lenz::cli::parse(&["log".into()],&cwd){self.new_view=Some((req,cwd));}}
            // a commit in a view of its own (F8 switches views)
            OpenRevisions{root,revs}=>{if let Ok(req)=lenz::cli::parse(&revs,&root){self.new_view=Some((req,root));}}
        }
    }
    pub(crate) fn copy(&mut self,text:&str){match crate::clipboard::copy(text){Ok(())=>self.session.set_notice("Copied (terminal OSC 52 clipboard)"),Err(e)=>self.session.set_notice(format!("Copy failed: {e}"))}}
    pub(crate) fn open_url(&mut self,url:&str){if let Err(e)=lenz::open::browse(url){self.session.set_notice(e);}}
    fn blame_effect(&mut self,effect:crate::visual::BlameEffect){
        use crate::visual::BlameEffect::*;
        match effect {
            Copy(text)=>self.copy(&text),OpenCommit(sha)=>{if let Some(root)=self.session.change_set().and_then(|s|s.git_root().map(std::path::Path::to_path_buf)){if let Some(url)=lenz::repository_session::github_url(&root){self.open_url(&format!("{url}/commit/{sha}"));}else{self.session.set_notice("No supported repository URL");}}},
            CompareCommit(sha)=>self.show_commit(sha,None),
            AnnotateParent{sha,path,line}=>self.show_commit(sha,Some((path,line))),
        }
    }
    fn show_commit(&mut self,sha:String,annotation:Option<(String,usize)>){
                let Some(root)=self.session.change_set().and_then(|s|s.git_root().map(std::path::Path::to_path_buf))else{return;};
                let parent=lenz::blame::parent_or_empty_tree(&root,&sha).unwrap_or_else(|_|format!("{sha}^1"));
                let mut args=vec![parent,sha];if let Some((path,line))=annotation{args.splice(0..0,["--blame".into(),"--line".into(),line.to_string()]);args.extend(["--".into(),path]);}
        if let Ok(req)=lenz::cli::parse(&args,&root){self.new_view=Some((req,root));}
    }
    fn open_editor(&mut self,default_app:bool){self.open_editor_plan(self.session.current(),self.focus,default_app);}
    pub(crate) fn open_editor_at(&mut self,index:usize,side:Side){self.open_editor_plan(index,side,false);}
    fn open_editor_plan(&mut self,index:usize,side:Side,default_app:bool){
        let Some(set)=self.session.change_set()else{return;};
        let mapped=if index==self.session.current(){match self.session.markdown_inspection_position(side){Ok(pos)=>pos,Err(reason)=>{self.session.set_notice(reason);return;}}}else{None};
        let selection=mapped.map(|p|lenz::selection::Selection::caret(side,p)).or_else(||self.selection.filter(|s|s.side==side&&index==self.session.current()));
        let line=selection.map_or(self.session.view().anchor_line_on(side)as usize+1,|s|s.head.line as usize+1);
        let column=selection.and_then(|s|self.session.document().and_then(|d|d.lines_on(side).get(s.head.line as usize)).map(|text|text[..s.head.col.min(text.len())].chars().count()+1)).unwrap_or(1);
        #[cfg(test)]
        {let _=(set,default_app);self.inspected_position=Some((index,side,line,column));}
        #[cfg(not(test))]
        {
            let binary=self.session.document().is_some_and(|d|d.placeholder_kind().is_some()||d.notes().iter().any(|n|n.contains("textconv")));
            self.open_token=std::sync::Arc::new(());self.opening_editor=Some(lenz::work::submit(std::sync::Arc::downgrade(&self.open_token),move||lenz::open::comparison_plan(&set,index,side,binary,Some(line),Some(column),default_app)));
        }
    }
    pub(crate) fn mouse_position(&self,column:u16,row:u16)->Option<(Side,lenz::selection::Pos,bool)>{
        use unicode_width::UnicodeWidthStr;
        let area=self.diff_area;if !area.contains((column,row).into())||row<area.y+2||row>=area.bottom().saturating_sub(2+self.applied_panel_height()){return None;}
        let narrow=area.width<70||self.unified();let side=self.side_at(column,row);
        let left=self.left_width();let pane_x=area.x+if side==Side::Right&&!narrow{left+1}else{0};let pane_w=if narrow{area.width}else if side==Side::Left{left}else{area.width-left-1};
        let gutter=pane_w.min(7);let source=self.source_rect(side);
        let frame=self.session.view().frame();let rows=if side==Side::Left{frame.left}else{frame.right};let visible=row-area.y-2;
        let line=rows.iter().find(|r|r.y>=0.0&&r.y as u16==visible).and_then(|r|if let lenz::view::Content::Line(line)=r.content{Some(line)}else{None})?;
        let text=self.session.document()?.lines_on(side).get(line as usize)?;let target=usize::from(column.saturating_sub(source.x))+self.horizontal[self.pane_side(side) as usize];
        let mut cell=0;let mut byte=text.len();for(i,g)in text.grapheme_indices(true){let width=if g=="\t"{4}else if g.chars().any(lenz::document::hides_text){1}else{g.width().max(1)};if cell+width>target{byte=i;break;}cell+=width;}
        Some((side,lenz::selection::Pos::new(line,byte),column>=pane_x+gutter&&column<source.x))
    }
    /// The change block under a pointer in the diff panes: a line's block, or a folded block's row.
    pub(crate) fn block_at(&self,column:u16,row:u16)->Option<usize>{
        let area=self.diff_area;if !area.contains((column,row).into())||row<area.y+2||row>=area.bottom().saturating_sub(2+self.applied_panel_height()){return None;}
        let side=self.side_at(column,row);let frame=self.session.view().frame();let rows=if side==Side::Left{frame.left}else{frame.right};let visible=row-area.y-2;
        match rows.iter().find(|r|r.y>=0.0&&r.y as u16==visible)?.content {
            lenz::view::Content::Line(line)=>self.session.view().block_index(side,line),
            lenz::view::Content::Folded{block,..}=>Some(block as usize),
            _=>None,
        }
    }
    fn select_motion(&mut self,command:lenz::selection::CaretCommand,extend:bool){
        if let Some(reason)=self.session.markdown_edit_disabled(){self.session.set_notice(reason);return;}
        let at=lenz::selection::Pos::new(self.session.view().anchor_line_on(self.focus)as u32,0);
        let selection=self.selection.filter(|s|s.side==self.focus).unwrap_or_else(||lenz::selection::Selection::caret(self.focus,at));
        let selection=self.session.move_caret(selection,command,extend);self.selection=Some(selection);
        if let Some(text)=self.session.document().and_then(|d|d.lines_on(selection.side).get(selection.head.line as usize)){
            use unicode_width::UnicodeWidthStr;let column=text[..selection.head.col.min(text.len())].graphemes(true).map(|g|if g=="\t"{4}else{g.width().max(1)}).sum::<usize>();
            let pane=self.pane_side(self.focus);let width=usize::from(self.source_rect(self.focus).width);self.horizontal[pane as usize]=caret_scroll(column,self.horizontal[pane as usize],width);self.clamp_horizontal(pane);
        }
    }
    fn edit(&mut self,command:lenz::edit_session::EditCommand){self.session.edit(command);}
    pub(crate) fn guard_pending_edits(&mut self)->bool {
        if !self.session.has_dirty(){return false;}
        if let Some(index)=self.session.dirty_files().first().copied() && index!=self.session.current(){self.session.open_file(index);}
        self.prompt=None;self.about=None;self.toolbar.close();self.clear_context();if self.set_search.as_ref().is_some_and(|search|search.model.editing()){self.set_search=None;}
        self.navigator.focused=false;if let Some(repo)=&mut self.repository{repo.quit=false;repo.core.execute(lenz::repository_session::RepositoryCommand::Focus(lenz::repository_session::Pane::Diff));}
        self.quit=false;self.session.set_notice("Unsaved changes — Save (Ctrl-S) or Discard unsaved changes before closing or changing comparisons");true
    }
    fn request_quit(&mut self){if !self.guard_pending_edits(){self.quit=true;}}
    pub(crate) fn edit_menu(&mut self,command:lenz::edit_session::EditMenuCommand){
        if let Some(reason)=self.session.edit_disabled(command){self.session.set_notice(reason);}
        else if command==lenz::edit_session::EditMenuCommand::History{self.prompt=Some(Prompt::Applied);self.selected=0;}
        else if let Some(command)=command.command(self.session.view().current_change()){self.edit(command);}
    }
    fn apply(&mut self,onto:Side){let block=self.selection.filter(|s|self.session.view().line_visible(s.side,s.head.line)).and_then(|s|self.session.view().block_index(s.side,s.head.line)).or_else(||self.session.view().current_change());if let Some(block)=block{self.edit(lenz::edit_session::EditCommand::Apply{block,onto});}}
    pub(crate) fn left_width(&self)->u16 {crate::panes::left(self.diff_area.width,self.pane_ratio)}
    /// One full-width column holding both Sides (Source only; images and the Preview keep their layouts).
    pub(crate) fn unified(&self)->bool {self.session.view().unified()&&self.visual.image().is_none()&&self.session.markdown_mode()!=lenz::markdown::PresentationMode::Rendered}
    pub(crate) fn unified_disabled(&self)->Option<&'static str> {lenz::commands::unified_disabled(self.visual.image().is_some(),self.session.markdown_mode()==lenz::markdown::PresentationMode::Rendered)}
    /// The header lines each pane pins (index 0 left, 1 right; Unified: index 1), while sticky scroll is on.
    pub(crate) fn sticky_lines(&self)->[Vec<lenz::sticky::StickyLine>;2]{
        let rendered=self.session.markdown_mode()==lenz::markdown::PresentationMode::Rendered;
        if !self.visual.sticky||rendered||self.visual.image().is_some()||self.session.document().is_none_or(|d|d.placeholder_kind().is_some()){return Default::default();}
        let rows=self.session.view().viewport_rows().max(0.0) as usize;
        lenz::sticky::lines(&self.session.view().frame(),self.unified(),[self.visual.declarations(Side::Left),self.visual.declarations(Side::Right)],lenz::sticky::cap(rows))
    }
    /// The sticky line under the pointer, as (its Side, its 0-based line).
    fn sticky_at(&self,column:u16,row:u16)->Option<(Side,u32)>{
        let area=self.diff_area;if !area.contains((column,row).into())||row<area.y+2{return None;}
        let visible=u32::from(row-area.y-2);let pane=if self.unified(){1}else{self.pointer_side(column) as usize};
        self.sticky_lines().get(pane)?.iter().find(|s|s.row==visible).map(|s|(s.side,s.line))
    }
    /// The Side whose geometry (pane, horizontal scroll) applies: Unified paints both in one pane.
    pub(crate) fn pane_side(&self,side:Side)->Side {if self.unified(){Side::Right}else{side}}
    /// Where `side`'s text starts and how wide it is.
    pub(crate) fn source_rect(&self,side:Side)->ratatui::layout::Rect {
        if self.unified(){crate::render::unified_geometry(self.diff_area,self.visual.blame_enabled(side))}
        else{crate::render::text_geometry_ratio(self.diff_area,side,self.focus,self.visual.blame_enabled(side),self.pane_ratio)}
    }
    /// The Side under the pointer: by column in Split, by the row's Side in Unified (deleted rows are the left's).
    pub(crate) fn side_at(&self,column:u16,row:u16)->Side {
        if !self.unified(){return self.pointer_side(column);}
        let visible=f32::from(row.saturating_sub(self.diff_area.y+2));
        let frame=self.session.view().frame();
        if frame.left.iter().any(|r|r.y>=0.0&&r.y.floor()==visible){Side::Left}else{Side::Right}
    }
    pub(crate) fn pointer_side(&self,column:u16)->Side {if self.markdown_inline() {Side::Right} else if self.diff_area.width<70 {self.focus}else if column<self.diff_area.x+self.left_width(){Side::Left}else{Side::Right}}
    pub(crate) fn applied_panel_height(&self)->u16 {
        if self.visual.image().is_some()||self.session.applied().is_empty(){return 0;}
        crate::panes::size(self.diff_area.height.saturating_sub(4),self.applied_height.unwrap_or((self.session.applied().len().min(3)+1)as f32),2.0,1.0)
    }
    pub(crate) fn applied_rows(&self)->usize {self.session.applied().len().min(self.applied_panel_height().saturating_sub(1)as usize)}
    pub(crate) fn cancel_pane_drag(&mut self) {
        self.resizer.cancel();self.navigator.resizer.cancel();
        if let Some(search)=&mut self.set_search {search.resizer.cancel();}
        if let Some(repo)=&mut self.repository {repo.resizer.cancel();}
    }
    pub(crate) fn reset_pane_sizes(&mut self) {
        self.pane_ratio=0.5;self.applied_height=None;self.resizer.cancel();
        self.navigator.desired_width=None;self.navigator.resizer.cancel();
        if let Some(search)=&mut self.set_search {search.desired_height=None;search.resizer.cancel();}
        if let Some(repo)=&mut self.repository {repo.reset_pane_sizes();}
        self.visual.pane_ratio=Some(0.5);
    }
    pub(crate) fn resize_input(&mut self,event:&Event)->bool {
        let blocked=self.modal_input()||self.toolbar.menu_open();
        let own=self.resizer.handle(event,blocked);
        let files=self.navigator.resize_input(event,blocked);
        let search=self.set_search.as_mut().is_some_and(|s|s.resize_input(event,blocked));
        let repo=self.repository.as_mut().is_some_and(|r|r.resize_input(event,blocked));
        match own {
            crate::panes::Resize::None=>files||search||repo,
            crate::panes::Resize::Captured=>true,
            crate::panes::Resize::Move(0,width)=>{let total=f32::from(self.diff_area.width.saturating_sub(1));if total>0.0{self.pane_ratio=lenz::pane_layout::split(total,width/total,12.0)/total;self.visual.pane_ratio=Some(self.pane_ratio);}true},
            crate::panes::Resize::Move(_,top)=>{self.applied_height=Some((f32::from(self.diff_area.height.saturating_sub(2))-top).max(0.0));true}
        }
    }
    pub(crate) fn image_area(&self)->ratatui::layout::Rect{ratatui::layout::Rect::new(self.diff_area.x,self.diff_area.y+2,self.diff_area.width,self.diff_area.height.saturating_sub(4))}

    pub(crate) fn interaction_disabled(&self,command:commands::Interaction)->Option<&'static str>{
        let mapped=self.session.markdown_inspection_position(self.focus);
        if matches!(command,commands::Interaction::Editor|commands::Interaction::DefaultApp|commands::Interaction::InspectBlame) && let Err(reason)=mapped {return Some(reason);}
        if matches!(command,commands::Interaction::Copy|commands::Interaction::CopyMarkdownSource) && (self.session.markdown_mode()==lenz::markdown::PresentationMode::Rendered || command==commands::Interaction::CopyMarkdownSource) {return self.session.copy_disabled(command,self.selection,self.focus);}let line=mapped.ok().flatten().map_or_else(||self.selection.filter(|s|s.side==self.focus).map_or(self.session.view().anchor_line_on(self.focus)as usize,|s|s.head.line as usize),|p|p.line as usize);if matches!(command,commands::Interaction::CopyLeftChange|commands::Interaction::CopyRightChange)&&let Some(reason)=self.session.copy_disabled(command,self.selection,self.focus){return Some(reason);}command.disabled(&commands::InteractionContext{find_open:self.find_open(),document:self.session.document().is_some(),text:self.visual.image().is_none()&&self.session.document().is_some_and(|d|d.placeholder_kind().is_none()),selection:self.selection.is_some_and(|s|!s.is_empty()),repository:self.session.change_set().is_some_and(|s|s.git_root().is_some()),annotation:self.visual.annotation(self.focus,line).is_some()})}
    pub(crate) fn reload(&mut self){if let Some(repo)=&mut self.repository{repo.core.execute(lenz::repository_session::RepositoryCommand::RefreshRepository);self.session.reload_set();}else{self.session.reload();}}
    /// A find prompt or Change Set search input is open: the find option toggles apply to it.
    /// The 1-based line and character column a Line prompt asks for (column 1 when none is given).
    pub(crate) fn find_open(&self)->bool{self.prompt==Some(Prompt::Search)||self.set_search.as_ref().is_some_and(|s|s.model.editing())}
    /// Flip one find option for the open input; the window keeps it (Change Set search shares it).
    pub(crate) fn toggle_find_option(&mut self,option:lenz::find::FindOption){
        if let Some(search)=self.set_search.as_mut().filter(|s|s.model.editing()){search.model.toggle_option(option);if self.session.find_options()!=search.model.options(){self.session.toggle_find_option(option);}return;}
        self.session.toggle_find_option(option);
        if self.prompt==Some(Prompt::Search)&&self.session.markdown_edit_disabled().is_none()&&let Some(selection)=self.session.find_selection(){self.selection=Some(selection);}
    }
    fn open_set_search(&mut self){self.navigator.focused=false;if let Some(search)=&mut self.set_search{search.edit();}else if let Some(set)=self.session.change_set(){self.set_search=Some(crate::search::Search::open(set,false,self.session.find_options()));}}
    pub(crate) fn interaction(&mut self,command:commands::Interaction){
        use commands::Interaction::*;
        if let Some(reason)=self.interaction_disabled(command){self.session.set_notice(reason);return;}
        match command {
            StickyScroll=>{self.visual.sticky = !self.visual.sticky;if self.visual.sticky&&let Some(doc)=self.session.document_arc(){self.visual.reparse_for_sticky(doc);}},
            ScrollLeft|ScrollRight=>self.scroll_horizontal(self.focus,if command==ScrollLeft{-4}else{4}),
            FindCase|FindWord|FindRegex=>{if let Some(option)=command.find_option(){self.toggle_find_option(option);}},
            Find=>self.begin(Prompt::Search),FindSet=>self.open_set_search(),GoToLine=>self.begin(Prompt::Line),
            Copy|CopyMarkdownSource|CopyPath|CopyReference|CopyLeftChange|CopyRightChange=>{match self.session.copy_text(command,self.selection,self.focus){Ok(text)=>self.copy(&text),Err(reason)=>self.session.set_notice(reason)}},SelectAll=>{if self.session.markdown_loading(){if let Some(reason)=self.session.markdown_edit_disabled(){self.session.set_notice(reason);}}else if self.session.markdown_mode()==lenz::markdown::PresentationMode::Rendered{self.markdown_select_all();}else if let Some(doc)=self.session.document(){self.selection=Some(lenz::selection::Selection::all(self.focus,doc.lines_on(self.focus)));}},
            Editor|DefaultApp=>self.open_editor(command==DefaultApp),Blame=>{self.blame[self.focus as usize]= !self.blame[self.focus as usize];self.visual.toggle_blame(self.focus);},
            ChangeTheme=>self.theme_request=true,
            ResetPaneSizes=>{self.reset_pane_sizes();self.reset_sizes_requested=true;},
            About=>self.about=Some(Default::default()),
            ReportLastError=>{if let Err(error)=self.report_request.start(){self.session.set_notice(format!("Cannot prepare report: {error}"));}else{self.session.set_notice("Preparing issue draft; review the local report before sharing");}},
            InspectBlame=>{match self.session.markdown_inspection_position(self.focus){Ok(mapped)=>{let line=mapped.map_or_else(||self.selection.filter(|s|s.side==self.focus).map_or(self.session.view().anchor_line_on(self.focus)as usize,|s|s.head.line as usize),|p|p.line as usize);self.visual.inspect_blame(self.focus,line);},Err(reason)=>self.session.set_notice(reason)}},Reload=>self.reload()
        }
    }
    pub(crate) fn files(&self) -> Vec<usize> {
        let (query, _) = navigator::query_line(&self.input);
        let paths = self.session.comparisons().iter().map(|c| c.path.as_str()).collect::<Vec<_>>();
        navigator::rank(query, &paths)
    }
    pub(crate) fn begin(&mut self, prompt: Prompt) {
        self.input = if prompt == Prompt::Search && self.session.markdown_mode()==lenz::markdown::PresentationMode::Rendered {self.session.markdown_selection().filter(|s|s.anchor.abs_diff(s.head)<=4096).and_then(|_|self.session.copy_text(commands::Interaction::Copy,self.selection,self.focus).ok()).filter(|s|!s.contains('\n')).unwrap_or_else(||self.session.find().map_or(String::new(),|f|f.query().into()))} else if prompt == Prompt::Search { self.selection.filter(|s|!s.is_empty()&&s.anchor.line==s.head.line).and_then(|s|self.session.document().map(|d|s.text(d.lines_on(s.side)))).unwrap_or_else(||self.session.find().map_or(String::new(),|f|f.query().into())) } else { String::new() };
        self.prompt = Some(prompt);
        self.selected = 0;
        if prompt==Prompt::Search { self.search_input(); }
    }
    fn search_input(&mut self){let selected=if self.session.markdown_mode()==lenz::markdown::PresentationMode::Rendered {self.session.markdown_selection().and_then(|s|self.session.markdown_projection()?.source_selection(s))}else{self.selection};let from=selected.filter(|s|s.side==self.focus).map_or_else(||lenz::selection::Pos::new(self.session.view().anchor_line_on(self.focus)as u32,0),|s|s.ordered().0);self.session.search_at(self.input.clone(),self.focus,from);if self.session.markdown_edit_disabled().is_none() && let Some(selection)=self.session.find_selection(){self.selection=Some(selection);}}
    fn type_text(&mut self, text: &str) {
        for ch in text.chars().filter(|c| !c.is_control()) {
            if self.input.len() >= 4096 { break; }
            self.input.push(ch);
        }
        self.selected = 0;
        if self.prompt == Some(Prompt::Search) { self.search_input(); }
    }

    /// ⚙: the shared Diff menu (layout, whitespace, highlighting, scrolling, context, case, blame), one row each.
    fn diff_menu_item(&self,scope:&lenz::catalog::Scope)->Option<crate::chrome::Item<ToolbarAction>> {
        use crate::chrome::Item;use ToolbarAction::*;
        let item=|label:std::borrow::Cow<'static,str>,action:lenz::catalog::Action|{let mut item=Item::new(label,Run(action));item.disabled=lenz::catalog::Availability::disabled(self,action);item};
        let options=lenz::catalog::diff_options(scope);
        if options.is_empty() {return None;}
        let rows=options.into_iter().map(|row|{
            let prefix=format!("{}: ",row.label);let single=row.choices.len()==1;
            let current=row.choices.iter().find(|e|e.current).filter(|_|!single).map(|e|e.label.trim_start_matches(&prefix).to_string());
            let label=current.map_or_else(||format!("{} ▾",row.label),|c|format!("{}: {c} ▾",row.label));
            let choices=row.choices.into_iter().map(|e|{let label:std::borrow::Cow<'static,str>=e.label.trim_start_matches(&prefix).to_string().into();item(label,e.action).active(e.current)}).collect();
            Item::new(label,Palette).choices(choices)
        }).collect();
        Some(Item::new("⚙",Palette).choices(rows))
    }
    /// Focus: navigation, editing, ⚙ (as in the Overview), ≡ Unchanged, Files and ? — the GUI's Focus toolbar.
    fn focus_toolbar_items(&self,width:u16,scope:&lenz::catalog::Scope)->Vec<crate::chrome::Item<ToolbarAction>> {
        use crate::chrome::Item;use ToolbarAction::*;
        use lenz::catalog::{Action,HelpAction};use lenz::layout::LayoutCommand as L;
        let item=|label:std::borrow::Cow<'static,str>,action:Action|{let mut item=Item::new(label,Run(action));item.disabled=lenz::catalog::Availability::disabled(self,action);item};
        let entries=|entries:Vec<lenz::catalog::Entry>|entries.into_iter().map(|e|item(e.label,e.action).active(e.current)).collect::<Vec<_>>();
        let mut app_menu=entries(lenz::catalog::app_menu());app_menu.push(Item::new("Nerd Font icons (Mono)",Icons).active(self.icons.nerd()));
        let mut items=vec![Item::new("☰",Palette).choices(app_menu)];
        if self.repository.is_some(){
            items.push(item("Overview".into(),Action::Layout(L::LeaveFocus)));
            items.extend([item("◀".into(),Action::Repository(lenz::catalog::RepoAction::Back)),item("▶".into(),Action::Repository(lenz::catalog::RepoAction::Forward))]);
        }
        // a PR's reading tab hides the diff: its controls wait for Files changed, as in the GUI
        if scope.pr_tab.is_none() {
            items.extend([item("↑".into(),Action::Comparison(Command::PreviousChange)),item("↓".into(),Action::Comparison(Command::NextChange))]);
            if width>=100 {items.extend([item("←".into(),Action::Comparison(Command::PreviousFile)),item("→".into(),Action::Comparison(Command::NextFile))]);}
            if self.visual.image().is_none() && let Some(diff)=self.diff_menu_item(scope){items.push(diff);}
        }
        items.push(item("?".into(),Action::Help(HelpAction::Shortcuts)).right());
        // the PR's tabs, in the bar's middle
        items.extend(lenz::catalog::pr_tabs(scope).into_iter().map(|e|item(e.label,e.action).active(e.current).center()));
        items.push(Item::new("⋯",Palette).overflow());
        items
    }

    fn toolbar_items(&self,width:u16)->Vec<crate::chrome::Item<ToolbarAction>> {
        use crate::chrome::Item;use ToolbarAction::*;
        use lenz::catalog::{Action,RepoAction};use lenz::layout::LayoutCommand as L;
        let item=|label:std::borrow::Cow<'static,str>,action:Action|{let mut item=Item::new(label,Run(action));item.disabled=lenz::catalog::Availability::disabled(self,action);item};
        let entries=|entries:Vec<lenz::catalog::Entry>|entries.into_iter().map(|e|item(e.label,e.action).active(e.current)).collect::<Vec<_>>();
        let scope=self.scope();
        if self.layout.shows_focus(self.repository.as_ref().map(|r|&r.core)) {return self.focus_toolbar_items(width,&scope);}
        let mut app_menu=entries(lenz::catalog::app_menu());app_menu.push(Item::new("Nerd Font icons (Mono)",Icons).active(self.icons.nerd()));
        // ☰, then Focus where Focus shows Overview
        let mut items=vec![Item::new("☰",Palette).choices(app_menu)];
        // as wide as "Overview", so the buttons after it stay put between the views
        if self.repository.is_some(){items.push(item(" Focus  ".into(),Action::Layout(L::ToggleFocus)));}
        if self.repository.is_some(){items.extend([item("◀".into(),Action::Repository(RepoAction::Back)),item("▶".into(),Action::Repository(RepoAction::Forward))]);}
        items.extend([item("↑".into(),Action::Comparison(Command::PreviousChange)),item("↓".into(),Action::Comparison(Command::NextChange))]);
        if width>=100 {items.extend([item("←".into(),Action::Comparison(Command::PreviousFile)),item("→".into(),Action::Comparison(Command::NextFile))]);}
        if let Some(diff)=self.diff_menu_item(&scope){items.push(diff);}
        items.push(item("?".into(),Action::Help(lenz::catalog::HelpAction::Shortcuts)).right());
        items.push(Item::new("⋯",Palette).overflow());
        items
    }
    /// While a new Change Set's first file is read (at most SWAP_WAIT), the last comparison stays on screen; otherwise
    /// remember what was drawn while one is on its way. Only repository views swap Change Sets.
    fn hold_comparison(&mut self,buf:&mut ratatui::buffer::Buffer){
        if self.repository.is_none(){self.held_diff=None;return;}
        let area=self.diff_area;
        let loading=self.session.loading();
        if loading && self.swap_at.is_some_and(|t|t.elapsed()<lenz::repository_session::SWAP_WAIT) {
            if let Some(held)=&self.held_diff && held.area==area {crate::chrome::copy_rows(held,buf,area);}
            return;
        }
        if loading {return;}
        self.swap_at=None;
        // copy only while a Change Set is on its way: the frame drawn after the click that asked for it
        let pending=self.repository.as_ref().is_some_and(|r|!r.core.settled());
        if !pending||area.area()==0 {self.held_diff=None;return;}
        let held=self.held_diff.get_or_insert_with(||ratatui::buffer::Buffer::empty(area));
        if held.area!=area {*held=ratatui::buffer::Buffer::empty(area);}
        crate::chrome::copy_rows(buf,held,area);
    }
    /// A prompt or the command palette has the keyboard.
    pub(crate) fn prompting(&self)->bool{self.prompt.is_some()||self.command_palette.is_some()}
    fn toolbar_action(&mut self,action:ToolbarAction){
        match action {
            ToolbarAction::Icons=>self.icons.toggle(),
            ToolbarAction::Run(action)=>self.run_action(action),
            ToolbarAction::Palette=>self.open_palette(),
        }
    }
    fn cycle_pane(&mut self, backwards: bool) {
        use lenz::{navigator::InputPane as P, repository_session::{Pane, RepositoryCommand as C}};
        let picker = self.repository.as_ref().map(|r| &r.core.picker);
        let current = if self.navigator.focused { P::Files }
            else if self.set_search.as_ref().is_some_and(|s| s.focused && !s.model.editing()) { P::Results }
            else if picker.is_some_and(|p| p.shown && p.focus == Pane::Sidebar) { P::Sources }
            else if picker.is_some_and(|p| p.shown && p.focus == Pane::Log) { P::Log }
            else if self.focus == Side::Left && !self.markdown_inline() { P::Left } else { P::Right };
        let panes = [picker.is_some_and(|p| p.shown && p.sidebar).then_some(P::Sources),
            picker.is_some_and(|p| p.shown && p.log_shown).then_some(P::Log),
            // Inline Markdown has one pane: the new side's
            self.navigator.shown().then_some(P::Files), (!self.markdown_inline()).then_some(P::Left), None, Some(P::Right),
            self.set_search.as_ref().is_some_and(|s| !s.model.editing()).then_some(P::Results)];
        let Some(target) = navigator::cycle_pane(current, &panes, backwards) else { return; };
        self.navigator.focused = target == P::Files;
        if let Some(search) = &mut self.set_search { search.focused = target == P::Results; }
        if let Some(repo) = &mut self.repository {
            repo.core.execute(C::Focus(match target {
                P::Sources => Pane::Sidebar, P::Log => Pane::Log,
                P::Files | P::Left | P::Middle | P::Right | P::Results => Pane::Diff,
            }));
        }
        match target {
            P::Left => self.focus = Side::Left,
            P::Right => self.focus = Side::Right,
            P::Sources | P::Log | P::Files | P::Results => {},
            P::Middle => unreachable!("comparison has no middle pane"),
        }
    }

    fn prompt_len(&self)->usize {
        match self.prompt {Some(Prompt::Files)=>self.files().len(),Some(Prompt::Applied)=>self.session.applied().len(),_=>0}
    }
    fn popup_mouse(&mut self,event:&Event)->bool {
        let Event::Mouse(mouse)=event else{return false;};
        // Find, Go to line and Find in files while typed: a click off their status line closes them, then acts as usual
        if mouse.kind==MouseEventKind::Down(MouseButton::Left) {
            let on_status=mouse.row==self.diff_area.bottom().saturating_sub(2)&&self.diff_area.contains((mouse.column,mouse.row).into());
            if !on_status {
                if let Some(prompt)=self.prompt.filter(|p|matches!(p,Prompt::Search|Prompt::Line)) {self.prompt=None;if prompt==Prompt::Search{self.session.clear_find();}}
                if self.set_search.as_ref().is_some_and(|s|s.model.editing()) {self.set_search=None;}
            }
        }
        if self.prompt==Some(Prompt::Help){match mouse.kind{MouseEventKind::ScrollUp=>self.selected=self.selected.saturating_sub(3),MouseEventKind::ScrollDown=>self.selected=self.selected.saturating_add(3),
            // a click outside the table closes it
            MouseEventKind::Down(MouseButton::Left) if !crate::help::rect(self.popup_area).contains((mouse.column,mouse.row).into())=>self.prompt=None,_=>{}}return true;}
        if !matches!(self.prompt,Some(Prompt::Files|Prompt::Applied)) {return false;}
        let last=self.prompt_len().saturating_sub(1);
        match mouse.kind {
            MouseEventKind::ScrollUp=>self.selected=self.selected.saturating_sub(3),
            MouseEventKind::ScrollDown=>self.selected=(self.selected+3).min(last),
            MouseEventKind::Down(MouseButton::Left)=>{
                if let Some(&(index,_))=self.popup_hits.iter().find(|(_,rect)|rect.contains((mouse.column,mouse.row).into())) {self.selected=index;self.handle(Event::Key(crossterm::event::KeyEvent::new(KeyCode::Enter,KeyModifiers::NONE)));}
                else if !self.popup_rect.contains((mouse.column,mouse.row).into()){self.prompt=None;}
            },_=>{}
        }
        true
    }
    fn details_text(&self)->String {
        if let Some(text)=self.context_details(){return text;}
        if let Some(p)=&self.command_palette {return p.selected().and_then(|i|p.rows().nth(i)).map_or_else(||"No selected action".into(),|row|row.disabled.as_deref().map_or_else(||row.entry.label.to_string(),|why|format!("{} — {why}",row.entry.label)));}
        if let Some(prompt @ (Prompt::Applied|Prompt::Files))=self.prompt {
            return crate::render::popup_rows(self,prompt).get(self.selected).cloned().unwrap_or_else(||"No selected action".into());
        }
        let mut text=crate::render::status_text(self);
        if let Some(repository)=&self.repository&&let Some(notice)=repository.core.notice.as_ref().or(repository.core.picker.error.as_ref()){text.push_str("\n\n");text.push_str(notice);}
        if text.is_empty(){"No status message".into()}else{text}
    }
    /// Where a pull request's details tab draws: the diff body below the pane headers.
    pub(crate) fn pr_pane(&self) -> ratatui::layout::Rect {
        // from the diff area's top: the tabs are in the toolbar, so the diff's title and header rows have nothing to say
        let a = self.diff_area;
        ratatui::layout::Rect::new(a.x, a.y, a.width, a.height.saturating_sub(2))
    }
    pub(crate) fn pr_details_shown(&self) -> bool {
        self.repository.as_ref().and_then(|r| r.core.pr.as_ref()).is_some_and(|p| p.details().is_some())
    }
    /// A Folder Summary and a PR details tab both take the diff area: showing a summary closes the tab.
    pub(crate) fn close_pr_details(&mut self) {
        if self.pr_details_shown() { self.pr_command(lenz::pr_session::PrCommand::Close); }
    }
    /// ... and a PR details tab, however it opened, closes a summary.
    fn yield_summary(&mut self) {
        if self.pr_details_shown() && self.session.folders.summary().is_some() { self.session.close_summary(); }
    }
    pub(crate) fn pr_command(&mut self, command: lenz::pr_session::PrCommand) {
        // a details tab takes the diff area from a Folder Summary
        if matches!(command, lenz::pr_session::PrCommand::Show(_) | lenz::pr_session::PrCommand::ShowThreads(_)) && self.session.folders.summary().is_some() { self.session.close_summary(); }
        if let Some(pr) = self.repository.as_mut().and_then(|r| r.core.pr.as_mut()) { pr.execute(command); }
        self.sync_pr_tab();
    }
    /// Details are a Focus tab: shown from the Overview, they enter Focus.
    pub(crate) fn sync_pr_tab(&mut self) {
        if self.pr_details_shown() && !self.layout.focused(self.repository.as_ref().map(|r| &r.core)) {
            self.layout_command(lenz::layout::LayoutCommand::ToggleFocus);
        }
    }
    /// The Folder Summary takes its keys and the pointer over it; the rest reach the app.
    fn summary_event(&mut self, event: &Event) -> bool {
        use crate::summary::Outcome;
        let Some(summary) = self.session.folders.summary().cloned() else { return false };
        match self.summary.handle(event, &summary) {
            Some(Outcome::Select(delta)) => self.session.folders.select(delta),
            Some(Outcome::OpenRow(row)) => { self.session.folders.select(row as isize - summary.selected as isize); self.session.open_summary_file(); self.horizontal = [0; 2]; }
            Some(Outcome::Open) => { self.session.open_summary_file(); self.horizontal = [0; 2]; }
            Some(Outcome::Close) => self.session.close_summary(),
            Some(Outcome::Collapse) => self.session.folder_action(lenz::folders::FolderAction::Collapse),
            Some(Outcome::Expand) => self.session.folder_action(lenz::folders::FolderAction::Expand),
            Some(Outcome::Taken) => {}
            None => return false,
        }
        if matches!(event, Event::Mouse(m) if m.kind == MouseEventKind::Down(MouseButton::Left)) { self.navigator.focused = false; }
        true
    }
    fn pr_tab_event(&mut self, event: &Event) -> bool {
        if !self.pr_details_shown() || matches!(event, Event::Key(k) if k.kind == KeyEventKind::Release) { return false; }
        let area = self.pr_pane();
        let Some(repo) = self.repository.as_mut() else { return false };
        let Some(pr) = repo.core.pr.as_mut() else { return false };
        let (taken, outcome) = repo.pr_dialog.handle(event, pr, area);
        match outcome {
            Some(crate::pr_details::Outcome::Pr(command)) => self.pr_command(command),
            Some(crate::pr_details::Outcome::Commit(sha)) => self.repository_command(lenz::repository_session::RepositoryCommand::PrCommit(sha)),
            Some(crate::pr_details::Outcome::Url(url)) => self.open_url(&url),
            Some(crate::pr_details::Outcome::Copy(text)) => self.copy(&text),
            Some(crate::pr_details::Outcome::Line { path, line, left }) => match self.session.comparisons().iter().position(|c| c.path == path) {
                Some(index) => {
                    self.pr_command(lenz::pr_session::PrCommand::Close);
                    self.session.open_file(index);
                    self.session.open_at(if left { Side::Left } else { Side::Right }, line);
                }
                None => self.session.set_notice(format!("{path} is not in this comparison")),
            },
            None => {}
        }
        taken
    }
    pub fn handle(&mut self, event: Event) -> bool {
        let handled = self.handle_event(event);
        self.sync_pr_tab();
        handled
    }
    fn handle_event(&mut self, event: Event) -> bool {
        let closing_details=matches!(event,Event::Key(k)if k.kind!=KeyEventKind::Release&&k.modifiers==KeyModifiers::CONTROL&&matches!(k.code,KeyCode::Char('q'|'c')));
        if !closing_details && let Some(message)=&mut self.message {
            if let Event::Resize(w,h)=event {self.size=(w,h);}
            if message.handle(&event){self.message=None;}
            return true;
        }
        if matches!(event,Event::Key(k)if k.kind!=KeyEventKind::Release&&k.code==KeyCode::F(2)&&k.modifiers.is_empty())&&self.about.is_none()&&!self.visual.commit_visible()&&matches!(self.prompt,None|Some(Prompt::Files|Prompt::Applied))&&self.set_search.as_ref().is_none_or(|s|!s.model.editing())&&self.repository.as_ref().is_none_or(|r|!r.modal()) {
            self.message=Some(crate::help::Message::new(self.details_text(),self.popup_area));return true;
        }
        if !matches!(event,Event::Key(k)if k.kind!=KeyEventKind::Release&&k.modifiers==KeyModifiers::CONTROL&&matches!(k.code,KeyCode::Char('q'|'c')))&&crate::palette::input(self,&event){return true;}
        if self.resize_input(&event){return true;}
        let closing=matches!(event,Event::Key(k)if k.kind!=KeyEventKind::Release&&k.modifiers==KeyModifiers::CONTROL&&matches!(k.code,KeyCode::Char('q'|'c')));
        if !closing&&!self.modal_input()&&self.toolbar.focused(){
            let(handled,action)=self.toolbar.handle(&event);
            if let Some(action)=action{self.toolbar_action(action);}
            if handled{return true;}
        }
        if self.context_input(&event){return true;}
        if self.popup_mouse(&event){return true;}
        if self.repository.as_ref().is_some_and(|r|r.core.picker.focus!=lenz::repository_session::Pane::Diff){self.navigator.focused=false;}
        if let Event::Key(key)=event && key.kind!=KeyEventKind::Release {
            let copying=key.modifiers==KeyModifiers::CONTROL&&key.code==KeyCode::Char('c');
            // a selection on a PR tab: Ctrl+C copies it rather than quitting
            if copying&&!self.modal_input()&&self.pr_details_shown()&&self.pr_tab_event(&event){return true;}
            if copying&&self.commit_panel.focused&&!self.navigator.focused&&!self.modal_input(){self.copy_message();return true;}
            if copying&&self.navigator.focused&&!self.modal_input(){if let Some(path)=self.navigator.copy_path(){self.copy(&path);}return true;}
            let log_copy=copying&&self.repository.as_ref().is_some_and(|r|!r.modal()&&r.core.picker.shown&&r.core.picker.focus==lenz::repository_session::Pane::Log);
            if !log_copy&&key.modifiers.contains(KeyModifiers::CONTROL)&&matches!(key.code,KeyCode::Char('c'|'q'))&&!key.modifiers.contains(KeyModifiers::SHIFT){self.request_quit();return true;}
            if self.visual.commit_visible(){if let Some(effect)=self.visual.commit_key(key){self.blame_effect(effect);}return true;}
        }
        if let Some(about)=&mut self.about {
            if let Event::Resize(w,h)=event {self.size=(w,h);}
            if about.handle(&event){self.about=None;}
            return true;
        }
        // Search entry is modal: a focused File Navigator must not consume its Enter or clicks.
        if self.prompt.is_none() && self.repository.as_ref().is_none_or(|r|!r.modal()) && let Some(search)=&mut self.set_search && search.model.editing() && (search.handle(event.clone()) || matches!(event,Event::Mouse(_) | Event::Key(_) | Event::Paste(_))){self.session.set_find_options(search.model.options());return true;}
        if self.prompt.is_none() && self.repository.as_ref().is_none_or(|r|!r.modal()) && !self.visual.commit_visible(){
            if matches!(event,Event::Key(k)if k.kind!=KeyEventKind::Release&&k.modifiers.contains(KeyModifiers::ALT)&&k.code==KeyCode::Char('1')){self.layout_command(lenz::layout::LayoutCommand::ToggleFiles);return true;}
            if let Event::Key(k)=event&&k.kind!=KeyEventKind::Release&&k.modifiers==KeyModifiers::ALT&&let KeyCode::Char(c @ ('0'|'2'|'3'))=k.code {
                use lenz::layout::LayoutCommand as L;self.layout_command(match c {'0'=>L::ToggleFocus,'2'=>L::ToggleBranches,_=>L::ToggleHistory});return true;
            }
            // Back / Forward between repository places; nothing outside a repository
            if let Event::Key(k)=event&&k.kind!=KeyEventKind::Release&&k.modifiers==KeyModifiers::ALT&&matches!(k.code,KeyCode::Left|KeyCode::Right) {
                if self.repository.is_some(){self.run_action(lenz::catalog::Action::Repository(if k.code==KeyCode::Left{lenz::catalog::RepoAction::Back}else{lenz::catalog::RepoAction::Forward}));}
                else{self.session.set_notice("Back and Forward work in repository views");}
                return true;
            }
            if matches!(event,Event::Key(k)if k.kind!=KeyEventKind::Release&&k.code==KeyCode::F(10)&&k.modifiers.is_empty()) {
                self.toolbar.close();
                self.open_palette();return true;
            }
            let (handled,action)=self.toolbar.handle(&event);
            if let Some(action)=action{self.toolbar_action(action);}
            if handled{return true;}
            if let Event::Mouse(m)=event&&m.kind==MouseEventKind::Down(MouseButton::Left)&&let Some(&(_,command))=self.panel_switch.iter().find(|(cell,_)|cell.contains((m.column,m.row).into())){self.layout_command(command);return true;}
            if let Event::Mouse(m)=event&&m.kind==MouseEventKind::Down(MouseButton::Left)&&self.palette_hint.contains((m.column,m.row).into()){self.open_palette();return true;}
            if let Event::Mouse(m)=event&&m.kind==MouseEventKind::Down(MouseButton::Left)&&let Some(&(_,action))=self.header_switch.iter().find(|(cell,_)|cell.contains((m.column,m.row).into()))
                // the cells are from the last frame: only a mode the comparison shown now offers
                &&lenz::catalog::header_switch(&self.scope()).iter().any(|e|e.action==action){self.run_action(action);return true;}
            // a details tab takes its keys (Tab switches its sections) and the pointer over it
            if !self.navigator.focused && self.pr_tab_event(&event) {return true;}
            if !self.navigator.focused && self.summary_event(&event) {return true;}
            if let Event::Key(k) = event && k.kind != KeyEventKind::Release
                && matches!(k.code, KeyCode::Tab | KeyCode::BackTab | KeyCode::F(6)) {
                if k.code == KeyCode::F(6) && let Some(repo) = &mut self.repository && !repo.core.picker.shown {
                    repo.core.execute(lenz::repository_session::RepositoryCommand::TogglePicker);
                }
                self.cycle_pane(k.code == KeyCode::BackTab || k.modifiers.contains(KeyModifiers::SHIFT));
                return true;
            }
            let pane_search_focus=self.navigator.focused||self.repository.as_ref().is_some_and(|r|r.core.picker.shown&&r.core.picker.focus==lenz::repository_session::Pane::Log);
            let ordinary_find_step=matches!(event,Event::Key(key) if key.code==KeyCode::F(3))&&(pane_search_focus||self.session.find().is_some_and(|f|!f.hits().is_empty()))&&self.set_search.as_ref().is_none_or(|s|!s.model.editing());
            let pane_search_cancel=matches!(event,Event::Key(k)if k.kind!=KeyEventKind::Release&&k.code==KeyCode::Esc)&&(self.navigator.focused&&self.navigator.search.active()||self.repository.as_ref().is_some_and(|r|r.core.picker.shown&&r.core.picker.focus==lenz::repository_session::Pane::Log&&r.core.picker.pane_search.active()));
            if self.prompt.is_none() && !ordinary_find_step && !pane_search_cancel && let Some(search)=&mut self.set_search {
                if search.handle(event.clone()) {
                    if matches!(event,Event::Mouse(m)if m.kind==MouseEventKind::Down(MouseButton::Left)) { self.navigator.focused=false; }
                    return true;
                }
                if matches!(event,Event::Mouse(m)if m.kind==MouseEventKind::Down(MouseButton::Left)) { search.focused=false; }
            }
            if matches!(event,Event::Key(k)if k.kind!=KeyEventKind::Release&&k.modifiers.contains(KeyModifiers::CONTROL|KeyModifiers::SHIFT)&&matches!(k.code,KeyCode::Char('f'|'F'))){self.open_set_search();return true;}
            let(taken,command)=self.commit_panel.handle(&event);
            if taken {
                if matches!(event,Event::Mouse(m)if m.kind==MouseEventKind::Down(MouseButton::Left)){self.commit_panel.focused=true;self.navigator.focused=false;}
                if let Some(command)=command{self.repository_command(command);}
                return true;
            }
            if self.commit_panel_key(&event){return true;}
            let files_had_keyboard=self.navigator.focused;
            if let Some(set)=self.session.change_set(){self.navigator.sync(&set,&self.session.folders,self.session.current());}
            let(handled,pick)=self.navigator.handle(&event);
            let index=match pick {
                Some(crate::files::Pick::File(index))=>{self.session.open_file(index);self.horizontal=[0;2];Some(index)}
                Some(crate::files::Pick::Toggle(folder))=>{self.session.toggle_folder(&folder);None}
                Some(crate::files::Pick::Summary{folder,group})=>{self.close_pr_details();self.session.show_summary(&folder,group);None}
                Some(crate::files::Pick::FolderOf(index))=>{self.close_pr_details();self.session.show_folder_of(index);None}
                None=>None,
            };
            // Overview: Enter or a double click on a file enters Focus on it
            let overview=self.repository.is_some()&&!self.layout.focused(self.repository.as_ref().map(|r|&r.core));
            let enter=files_had_keyboard&&matches!(event,Event::Key(k)if k.kind!=KeyEventKind::Release&&k.code==KeyCode::Enter&&k.modifiers.is_empty());
            let double=match (index,&event) {
                (Some(index),Event::Mouse(m)) if m.kind==MouseEventKind::Down(MouseButton::Left) => {
                    let now=std::time::Instant::now();
                    let again=self.files_click.is_some_and(|(at,i)|i==index&&now.duration_since(at)<std::time::Duration::from_millis(400));
                    self.files_click=(!again).then_some((now,index));again
                }
                _ => false,
            };
            if overview&&(enter||double){self.layout_command(lenz::layout::LayoutCommand::ToggleFocus);return true;}
            // choosing a file shows its diff
            if index.is_some()&&self.pr_details_shown(){self.pr_command(lenz::pr_session::PrCommand::Close);}
            if handled{if self.navigator.focused && let Some(repo)=&mut self.repository{repo.core.execute(lenz::repository_session::RepositoryCommand::Focus(lenz::repository_session::Pane::Diff));}return true;}
            // what Files left would act on the hidden diff
            if self.pr_tab_event(&event){return true;}
            if matches!(event,Event::Mouse(m)if m.kind==MouseEventKind::Down(MouseButton::Left)) {
                self.navigator.focused=false;
                if matches!(event,Event::Mouse(m)if self.diff_area.contains((m.column,m.row).into())) && let Some(repo)=&mut self.repository {
                    repo.core.execute(lenz::repository_session::RepositoryCommand::Focus(lenz::repository_session::Pane::Diff));
                }
            }
        }
        if self.prompt.is_none() && let Some(repo)=&mut self.repository && repo.handle(event.clone()){let quit=repo.quit;let interaction=repo.interaction.take();let palette=std::mem::take(&mut repo.palette_request);if let Some(command)=interaction{self.interaction(command);}
            if palette{self.open_palette();}
            if quit{self.request_quit();}return true;}
        if self.prompt.is_none() && self.markdown_input(&event){return true;}
        if self.prompt.is_none() && let Event::Key(key)=event && key.kind!=KeyEventKind::Release {
            let ctrl=key.modifiers.contains(KeyModifiers::CONTROL);let shift=key.modifiers.contains(KeyModifiers::SHIFT);let alt=key.modifiers.contains(KeyModifiers::ALT);
            if ctrl && shift && key.code==KeyCode::Char('f'){self.open_set_search();return true;}
            if ctrl && shift && key.code==KeyCode::Char('c') || !ctrl && key.code==KeyCode::Char('y') {self.interaction(commands::Interaction::Copy);return true;}
            if ctrl && key.code==KeyCode::Char('a'){self.interaction(commands::Interaction::SelectAll);return true;}
            if ctrl && !shift && key.code==KeyCode::Char('s'){self.edit_menu(lenz::edit_session::EditMenuCommand::Save);return true;}
            if ctrl && key.code==KeyCode::Char('z'){self.edit(if shift{lenz::edit_session::EditCommand::Redo}else{lenz::edit_session::EditCommand::Undo});return true;}
            if ctrl && key.code==KeyCode::Char('y'){self.edit(lenz::edit_session::EditCommand::Redo);return true;}
            if ctrl && alt && !shift && matches!(key.code,KeyCode::Left|KeyCode::Right) && self.visual.image().is_none(){self.interaction(if key.code==KeyCode::Left{commands::Interaction::ScrollLeft}else{commands::Interaction::ScrollRight});return true;}
            if alt && shift && matches!(key.code,KeyCode::Left|KeyCode::Right){self.apply(if key.code==KeyCode::Left{Side::Left}else{Side::Right});return true;}
            if key.code==KeyCode::Char('A'){self.begin(Prompt::Applied);return true;}
            if key.code==KeyCode::Char('b'){self.blame[self.focus as usize]= !self.blame[self.focus as usize];self.visual.toggle_blame(self.focus);return true;}
            if key.code==KeyCode::Char('B'){self.interaction(commands::Interaction::InspectBlame);return true;}
            if matches!(key.code,KeyCode::Char('o'|'O')){self.open_editor(shift||key.code==KeyCode::Char('O'));return true;}
            if key.code==KeyCode::F(5){self.reload();return true;}
            if self.visual.image().is_none() && !alt && (shift||self.selection.is_some()) && matches!(key.code,KeyCode::Left|KeyCode::Right|KeyCode::Up|KeyCode::Down|KeyCode::Home|KeyCode::End|KeyCode::PageUp|KeyCode::PageDown){use lenz::selection::CaretCommand;let command=match key.code{KeyCode::Left=>CaretCommand::Left,KeyCode::Right=>CaretCommand::Right,KeyCode::Up=>CaretCommand::Up,KeyCode::Down=>CaretCommand::Down,KeyCode::PageUp=>CaretCommand::PageUp,KeyCode::PageDown=>CaretCommand::PageDown,KeyCode::Home=>CaretCommand::Home(ctrl),KeyCode::End=>CaretCommand::End(ctrl),_=>unreachable!()};self.select_motion(command,shift);return true;}

            if key.code!=KeyCode::Char(':') && self.visual.image_key(key,self.image_area()){return true;}
        }
        match event {
            Event::Resize(w, h) => { self.size = (w, h); true }
            Event::Paste(text) if self.prompt.is_some() => { self.type_text(&text); true }
            Event::Mouse(mouse) if self.prompt.is_none() => {
                if !self.diff_area.contains((mouse.column,mouse.row).into()){return false;}
                let count=self.applied_rows() as u16;let start=self.diff_area.bottom().saturating_sub(1+self.applied_panel_height());
                if count>0 && mouse.row>=start && mouse.row<start+count && mouse.kind==MouseEventKind::Down(MouseButton::Left){let index=self.session.applied().len()-usize::from(count)+usize::from(mouse.row-start);if let Some(record)=self.session.applied().get(index){self.edit(lenz::edit_session::EditCommand::UndoApplied(record.id));}return true;}
                let side = self.pane_side(self.pointer_side(mouse.column));
                match mouse.kind {
                    MouseEventKind::ScrollRight => self.scroll_horizontal(side,4),
                    MouseEventKind::ScrollLeft => self.scroll_horizontal(side,-4),
                    MouseEventKind::ScrollDown if mouse.modifiers.contains(KeyModifiers::SHIFT)=>self.scroll_horizontal(side,4),
                    MouseEventKind::ScrollUp if mouse.modifiers.contains(KeyModifiers::SHIFT)=>self.scroll_horizontal(side,-4),
                    MouseEventKind::ScrollDown => self.session.scroll(3.0, side),
                    MouseEventKind::ScrollUp => self.session.scroll(-3.0, side),
                    MouseEventKind::Down(MouseButton::Left) if let Some((side,line))=self.sticky_at(mouse.column,mouse.row) => {self.focus=self.pane_side(side);self.session.go_to_line(side,line+1);}
                    MouseEventKind::Down(MouseButton::Left) => {
                        if let Some((side,pos,annotation))=self.mouse_position(mouse.column,mouse.row){self.focus=side;if annotation{self.visual.inspect_blame(side,pos.line as usize);}else if mouse.modifiers.contains(KeyModifiers::SHIFT)&&let Some(selection)=self.selection.as_mut().filter(|s|s.side==side){selection.head=pos;}else{let now=std::time::Instant::now();let count=self.mouse_click.filter(|(time,old_side,old_pos,_)|now.duration_since(*time)<std::time::Duration::from_millis(400)&&*old_side==side&&*old_pos==pos).map_or(1,|(_,_,_,count)|count%3+1);self.mouse_click=Some((now,side,pos,count));let lines=self.session.document().map_or(&[][..],|d|d.lines_on(side));self.selection=Some(match count{2=>lenz::selection::Selection::word(side,lines,pos),3=>lenz::selection::Selection::line(side,lines,pos.line),_=>lenz::selection::Selection::caret(side,pos)});}}else{self.focus=side;if mouse.row>=self.diff_area.y+2{self.session.expand(side,mouse.row-self.diff_area.y-2);}}
                    }
                    MouseEventKind::Drag(MouseButton::Left)=>{let top=self.diff_area.y+2;let bottom=self.diff_area.bottom().saturating_sub(3+self.applied_panel_height());if mouse.row<top{self.session.scroll(-3.0,self.focus);}else if mouse.row>bottom{self.session.scroll(3.0,self.focus);}
        if let Some((side,pos,_))=self.mouse_position(mouse.column,mouse.row.clamp(top,bottom.max(top)))&&let Some(selection)=self.selection.as_mut().filter(|s|s.side==side){selection.head=pos;}},
                    _ => return false,
                }
                true
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                let control = key.modifiers.contains(KeyModifiers::CONTROL);
                let shift = key.modifiers.contains(KeyModifiers::SHIFT);
                if control && matches!(key.code, KeyCode::Char('c' | 'q')) { self.request_quit(); return true; }
                if let Some(prompt) = self.prompt {
                    if prompt == Prompt::Search && key.modifiers.contains(KeyModifiers::ALT) && let KeyCode::Char(c @ ('c' | 'w' | 'r')) = key.code {
                        self.toggle_find_option(match c { 'c' => lenz::find::FindOption::Case, 'w' => lenz::find::FindOption::Word, _ => lenz::find::FindOption::Regex });
                        return true;
                    }
                    match key.code {
                        KeyCode::Esc => { self.prompt = None; if prompt == Prompt::Search { self.session.clear_find(); } }
                        KeyCode::Enter => {
                            match prompt {
                                Prompt::Search => self.search_input(),
                                Prompt::Line=>{match lenz_line(&self.input){Err(e)=>self.session.set_notice(e.to_string()),Ok((line,col))=>{self.session.go_to_line(self.focus,line);let raw=line.saturating_sub(1);let col=col.saturating_sub(1);if self.session.markdown_edit_disabled().is_none() && let Some(text)=self.session.document().and_then(|d|d.lines_on(self.focus).get(raw as usize)){let byte=text.char_indices().nth(col).map_or(text.len(),|(i,_)|i);self.selection=Some(lenz::selection::Selection::caret(self.focus,lenz::selection::Pos::new(raw,byte)));}}}},
                                Prompt::Files => {
                                    if let Some(&index) = self.files().get(self.selected) {
                                        self.session.open_file(index);
                                        self.horizontal = [0;2];
                                        if let (_, Some(line)) = navigator::query_line(&self.input) { self.session.open_at_line(line); }
                                    }
                                }
                                Prompt::Applied => {if let Some(record)=self.session.applied().get(self.selected){self.edit(lenz::edit_session::EditCommand::UndoApplied(record.id));}},
                                Prompt::Help => {}
                            }
                            self.prompt = None;
                        }
                        KeyCode::Home if matches!(prompt,Prompt::Files|Prompt::Applied)=>self.selected=0,
                        KeyCode::End if matches!(prompt,Prompt::Files|Prompt::Applied)=>self.selected=self.prompt_len().saturating_sub(1),
                        KeyCode::PageUp if matches!(prompt,Prompt::Files|Prompt::Applied)=>self.selected=self.selected.saturating_sub(8),
                        KeyCode::PageDown if matches!(prompt,Prompt::Files|Prompt::Applied)=>self.selected=(self.selected+8).min(self.prompt_len().saturating_sub(1)),
                        KeyCode::Home if prompt==Prompt::Help=>self.selected=0,
                        KeyCode::End if prompt==Prompt::Help=>self.selected=usize::MAX,
                        KeyCode::PageUp if prompt==Prompt::Help=>self.selected=self.selected.saturating_sub(10),
                        KeyCode::PageDown if prompt==Prompt::Help=>self.selected=self.selected.saturating_add(10),
                        KeyCode::Up => self.selected = self.selected.saturating_sub(1),
                        KeyCode::Down => {
                            let len = match prompt { Prompt::Files => self.files().len(), Prompt::Applied=>self.session.applied().len(), Prompt::Help=>usize::MAX, _ => 1 };
                            self.selected = self.selected.saturating_add(1).min(len.saturating_sub(1));
                        }
                        KeyCode::Backspace => {
                            if let Some((i, _)) = self.input.grapheme_indices(true).next_back() { self.input.truncate(i); }
                            self.selected = 0;
                            if prompt == Prompt::Search { self.search_input(); }
                        }
                        KeyCode::Char(ch) if !control && prompt != Prompt::Help => self.type_text(&ch.to_string()),
                        _ => return false,
                    }
                    return true;
                }
                let page = (self.session.view().viewport_rows() - 2.0).max(1.0);
                let alt=key.modifiers.contains(KeyModifiers::ALT);
                match key.code {
                    // Find option toggles (they say why when no find is open), never the plain letters.
                    KeyCode::Char('c') if alt => self.interaction(commands::Interaction::FindCase),
                    KeyCode::Char('w') if alt => self.interaction(commands::Interaction::FindWord),
                    KeyCode::Char('r') if alt => self.interaction(commands::Interaction::FindRegex),
                    KeyCode::Char('q') => self.request_quit(),
                    KeyCode::Char('/') => self.interaction(commands::Interaction::Find),
                    KeyCode::Char('f') if control => self.interaction(commands::Interaction::Find),
                    KeyCode::Char('g') => self.interaction(commands::Interaction::GoToLine),
                    KeyCode::Char('p') if control => self.begin(Prompt::Files),
                    KeyCode::Char('f') => self.begin(Prompt::Files),
                    KeyCode::F(10) | KeyCode::Char(':') => self.open_palette(),
                    KeyCode::F(1) | KeyCode::Char('?') => self.begin(Prompt::Help),
                    KeyCode::F(3) => {self.session.find_step(!shift);if self.session.markdown_edit_disabled().is_none() && let Some(selection)=self.session.find_selection(){self.selection=Some(selection);}},
                    KeyCode::Enter => self.session.execute(Command::LoadLarge),
                    KeyCode::Char('n') | KeyCode::F(7) if !shift => self.session.execute(Command::NextChange),
                    KeyCode::Char('p') | KeyCode::F(7) => self.session.execute(Command::PreviousChange),
                    KeyCode::Char(']') => { self.session.execute(Command::NextFile); self.horizontal = [0;2]; }
                    KeyCode::Char('[') => { self.session.execute(Command::PreviousFile); self.horizontal = [0;2]; }
                    KeyCode::Char('z') => self.session.execute(Command::ToggleContext),
                    KeyCode::Char('x') if shift => self.session.execute(Command::UnfoldAll),
                    KeyCode::Char('x') => self.session.execute(Command::ToggleFold),
                    KeyCode::Char('X') => self.session.execute(Command::UnfoldAll),
                    KeyCode::Char('s') => self.session.execute(Command::CycleScroll),
                    KeyCode::Char('w') => self.session.execute(Command::CycleWhitespace),
                    KeyCode::Char('h') => self.session.execute(Command::CycleHighlight),
                    KeyCode::Char('i') => self.session.execute(Command::ToggleCase),
                    KeyCode::Down | KeyCode::Char('j') => self.session.scroll(1.0, self.focus),
                    KeyCode::Up | KeyCode::Char('k') => self.session.scroll(-1.0, self.focus),
                    KeyCode::PageDown | KeyCode::Char(' ') => self.session.scroll(page, self.focus),
                    KeyCode::PageUp => self.session.scroll(-page, self.focus),
                    KeyCode::Home => self.session.scroll(f32::NEG_INFINITY, self.focus),
                    KeyCode::End => self.session.scroll(f32::INFINITY, self.focus),
                    KeyCode::Right => self.scroll_horizontal(self.focus,4),
                    KeyCode::Left => self.scroll_horizontal(self.focus,-4),
                    KeyCode::Esc => {if self.session.find().is_some(){self.session.clear_find();}else if self.selection.is_none_or(|s|s.is_empty())&&self.layout.focused(self.repository.as_ref().map(|r|&r.core)){self.layout_command(lenz::layout::LayoutCommand::LeaveFocus);}else{self.selection=self.selection.map(|s|lenz::selection::Selection::caret(s.side,s.head));}},
                    _ => return false,
                }
                true
            }
            _ => false,
        }
    }

    pub fn render(&mut self,frame:&mut ratatui::Frame){let area=frame.area();self.render_in(frame,area);}
    pub fn render_in(&mut self,frame:&mut ratatui::Frame,area:ratatui::layout::Rect){
        self.yield_summary();
        if self.session.markdown_mode()!=lenz::markdown::PresentationMode::Rendered {self.markdown.clear();}
        let palette=self.palette.clone();
        self.popup_area=area;
        let items=self.toolbar_items(area.width);let body=self.toolbar.draw_with_theme(frame,area,items,&palette,self.theme_revision);
        // the bottom row is a full-width bar (panel switches and keys), as in the GUI: side panels end above it
        let footer_row=1.min(body.height);
        let panels=ratatui::layout::Rect{height:body.height-footer_row,..body};
        let repo_area=self.repository.as_ref().map_or(panels,|r|r.area(panels));
        let repository_shown=self.repository.as_ref().is_some_and(|r|r.core.picker.shown);
        // a PR's Conversation and Commits read without the file list; Files changed has it
        self.navigator.suppressed=self.pr_details_shown();
        let comparison=self.navigator.prepare(self.session.change_set(),&self.session.folders,self.session.current(),repository_shown,repo_area);
        // the comparison keeps its last row, which the full-width bar draws over
        let comparison=ratatui::layout::Rect{height:comparison.height+footer_row,..comparison};
        let core=self.repository.as_ref().map(|r|&r.core).filter(|c|self.layout.focused(Some(c)));
        let lines=core.and_then(|c|c.message_lines(crate::commit_panel::CommitPanel::columns(self.navigator.area)));
        let title=core.and_then(|c|c.selection_title()).map_or_else(String::new,|t|t.text);
        let cursor=core.map_or(0,|c|c.message_cursor());
        // a range walks its commits; one commit (or a collapsed panel) has the keyboard on its first line
        let range=core.is_some_and(|c|!c.message_collapsed()&&matches!(c.message_panel(),Some(lenz::repository_session::MessagePanel::Ready(m))if matches!(m.query,lenz::commit_message::Query::Range(_))));
        let current=if range{lenz::commit_message::LineKind::Commit(cursor)}else{lenz::commit_message::LineKind::Toggle};
        self.commit_panel.place(&mut self.navigator.area,lines,title,current,cursor);
        self.diff_area=self.set_search.as_ref().map_or(comparison,|s|s.area(comparison));
        self.visual.pane_ratio=Some(self.pane_ratio);
        self.resizer.grips.clear();
        let image_two_up=self.visual.image().is_some()&&self.visual.image_state().mode()==lenz::image_session::Mode::TwoUp;
        // a Folder Summary has no panes to divide
        if self.diff_area.width>=70&&(self.visual.image().is_none()||image_two_up)&&self.diff_area.height>=4&&!self.markdown_inline()&&self.session.folders.summary().is_none() {
            self.resizer.grips.push(crate::panes::Grip::vertical(ratatui::layout::Rect::new(self.diff_area.x+self.left_width(),self.diff_area.y+1,1,self.diff_area.height.saturating_sub(3+self.applied_panel_height())),0,self.diff_area.x));
        }
        let applied=self.applied_panel_height();
        if applied>0 {self.resizer.grips.push(crate::panes::Grip::horizontal(ratatui::layout::Rect::new(self.diff_area.x,self.diff_area.bottom().saturating_sub(2+applied),self.diff_area.width,1),1,self.diff_area.y));}
        for side in [Side::Left,Side::Right]{self.clamp_horizontal(side);}
        crate::render::draw(self,frame);
        self.hold_comparison(frame.buffer_mut());
        self.resizer.draw(frame.buffer_mut(),&palette);
        if let Some(repo)=&mut self.repository{repo.render_in(frame,panels,&palette);}
        let focused=self.layout.focused(self.repository.as_ref().map(|r|&r.core));
        // the commit message panel carries the title when it is shown
        self.navigator.heading=(focused&&self.commit_panel.area.height==0).then(||self.repository.as_ref().and_then(|r|r.core.selection_title()).map(|t|t.text)).flatten();
        self.navigator.draw(frame,&palette);
        self.commit_panel.draw(frame,&palette);
        if let Some(search)=&mut self.set_search{search.render_in(frame,comparison,&palette);}
        if let Some(repo)=&mut self.repository{repo.draw_popup(frame,area,&palette);}
        self.toolbar.draw_menu_with_theme(frame,&palette);crate::render::draw_popup(self,frame);crate::palette::draw(self,frame);self.visual.draw_commit(frame.buffer_mut(),area,&palette);self.draw_context(frame);if let Some(message)=&mut self.message{message.draw(frame,area,&palette);}
    }

}

impl App {
    fn horizontal_max(&mut self,side:Side)->usize{
        if self.session.markdown_mode()==lenz::markdown::PresentationMode::Rendered{return self.markdown.horizontal_max(side);}
        let Some(doc)=self.session.document()else{return 0;};
        let mut widest=*self.columns[side as usize].get_or_insert_with(||crate::render::source_width(doc.lines_on(side)));
        // Unified scrolls both Sides together: the wider of the two
        if self.unified(){let other=if side==Side::Left{Side::Right}else{Side::Left};widest=widest.max(*self.columns[other as usize].get_or_insert_with(||crate::render::source_width(doc.lines_on(other))));}
        let width=usize::from(self.source_rect(side).width);
        widest.saturating_sub(width)
    }
    pub(crate) fn clamp_horizontal(&mut self,side:Side){
        if self.horizontal[side as usize]==0{return;}
        let max=self.horizontal_max(side);self.horizontal[side as usize]=self.horizontal[side as usize].min(max);
    }
    fn scroll_horizontal(&mut self,side:Side,delta:isize){
        if self.visual.image().is_some(){return;}
        if delta<=0&&self.horizontal[side as usize]==0{return;}
        let max=self.horizontal_max(side);self.horizontal[side as usize]=self.horizontal[side as usize].saturating_add_signed(delta).min(max);
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reopening_a_repository_returns_to_the_commit_file_and_line() {
        let tmp = tempfile::tempdir().unwrap();
        // the session keys places by git's resolved root (macOS temp dirs sit behind /var -> /private/var)
        let root = tmp.path().canonicalize().unwrap();
        let git = |args: &[&str]| { let o = std::process::Command::new("git").env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1")
            .args(["-c", "user.name=T", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"]).args(args).current_dir(root.as_path()).output().unwrap(); assert!(o.status.success()); String::from_utf8(o.stdout).unwrap() };
        git(&["init", "-q", "-b", "main"]);
        std::fs::write(root.as_path().join("a.txt"), "a\n").unwrap();
        std::fs::write(root.as_path().join("b.txt"), (1..=300).map(|i| format!("line {i}\n")).collect::<String>()).unwrap();
        git(&["add", "-A"]); git(&["commit", "-qm", "one"]);
        let first = git(&["rev-parse", "HEAD"]).trim().to_string();
        std::fs::write(root.as_path().join("b.txt"), (1..=300).map(|i| format!("LINE {i}\n")).collect::<String>()).unwrap();
        git(&["commit", "-qam", "two"]);
        let state = tempfile::tempdir().unwrap();
        let file = lenz::place::path_in(state.path(), root.as_path());
        lenz::place::save_to(&file, &lenz::place::Place { version: lenz::place::VERSION, root: root.as_path().into(),
            source: lenz::place::Source::Log { commit: first.clone() }, file: Some("b.txt".into()), line: Some(200) }).unwrap();
        let request = lenz::cli::parse_with(&[], root.as_path(), &Default::default()).unwrap();
        let mut app = App::open_with_place(request, root.as_path().into(), state.path());
        ready(&mut app);
        let repo = app.repository.as_ref().unwrap();
        assert_eq!(repo.core.place_source(), Some(lenz::place::Source::Log { commit: first }));
        assert_eq!(app.session.comparisons()[app.session.current()].path, "b.txt");
        assert!((app.session.view().anchor_line_on(Side::Right) as i64 + 1 - 200).abs() <= 1);
        drop(app);
        assert_eq!(lenz::place::load_from(&file, root.as_path()).unwrap().file.as_deref(), Some("b.txt"));
    }

    #[test]
    fn the_settings_markdown_default_applies_to_a_later_markdown_file() {
        use lenz::markdown::PresentationMode;
        let view = lenz::settings::View { markdown: PresentationMode::Rendered, ..Default::default() };
        let dir = tempfile::tempdir().unwrap();
        for side in ["left", "right"] {
            std::fs::create_dir(dir.path().join(side)).unwrap();
            std::fs::write(dir.path().join(side).join("a.txt"), format!("{side}\n")).unwrap();
            std::fs::write(dir.path().join(side).join("b.md"), format!("# {side}\n")).unwrap();
        }
        let args = ["--no-index".to_string(), "left".into(), "right".into()];
        let mut app = App::open(lenz::cli::parse_with(&args, dir.path(), &view).unwrap(), dir.path().into());
        ready(&mut app);
        assert_eq!(app.session.comparisons()[app.session.current()].path, "a.txt");
        assert_eq!(app.session.markdown_mode(), PresentationMode::Source);
        app.session.open_file(1);
        ready(&mut app);
        let until = Instant::now() + Duration::from_secs(10);
        while app.session.markdown_loading() { app.poll(); assert!(Instant::now() < until); thread::sleep(Duration::from_millis(2)); }
        assert_eq!(app.session.markdown_mode(), PresentationMode::Rendered, "b.md starts Rendered");
    }

    #[test]
    fn the_settings_markdown_default_starts_markdown_rendered_and_leaves_other_files_alone() {
        use lenz::markdown::PresentationMode;
        let view = lenz::settings::View { markdown: PresentationMode::Rendered, ..Default::default() };
        for (name, rendered) in [("a.md", true), ("a.txt", false)] {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join(format!("left-{name}")), "# Old\n").unwrap();
            std::fs::write(dir.path().join(format!("right-{name}")), "# New\n").unwrap();
            let args = ["--no-index".to_string(), format!("left-{name}"), format!("right-{name}")];
            let mut app = App::open(lenz::cli::parse_with(&args, dir.path(), &view).unwrap(), dir.path().into());
            ready(&mut app);
            let until = Instant::now() + Duration::from_secs(10);
            while app.session.markdown_loading() { app.poll(); assert!(Instant::now() < until); thread::sleep(Duration::from_millis(2)); }
            assert_eq!(app.session.markdown_mode() == PresentationMode::Rendered, rendered, "{name}");
            assert!(app.session.notice().is_none_or(|n| !n.contains("Markdown")), "no refusal notice for {name}: {:?}", app.session.notice());
        }
    }

    #[test]
    fn report_command_is_reachable_before_the_document_loads() {
        let (dir, mut app) = pair("old\n", "new\n");
        app.session = Session::open(lenz::cli::parse(&["--about".into()], dir.path()).unwrap(), dir.path().into());
        key(&mut app, KeyCode::Char(':'));
        for c in "report last error".chars() { key(&mut app, KeyCode::Char(c)); }
        let output = screen(&mut app, 40, 8);
        assert!(output.contains("Report last error"), "{output}");
        let command = commands::Interaction::ReportLastError;
        assert!(app.interaction_disabled(command).is_none());
        key(&mut app, KeyCode::Enter);
        assert!(app.prompt.is_none());
        assert_eq!(std::fs::read_to_string(dir.path().join("right.txt")).unwrap(), "new\n");
    }
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};
    use std::{time::{Duration, Instant}, thread};

    fn pane_mouse(app:&mut App,kind:MouseEventKind,x:u16,y:u16) {
        app.handle(Event::Mouse(crossterm::event::MouseEvent{kind,column:x,row:y,modifiers:KeyModifiers::NONE}));
    }

    /// A Rust comparison: `fn f` (renamed on the right) with a body taller than the screen, inside `mod m`.
    fn sticky_pair() -> (tempfile::TempDir, App) {
        // Every fourth body line differs, so no unchanged run is long enough to collapse.
        let body = |changed: &str| (0..80).map(|i| if i % 4 == 0 { format!("        {changed}{i};\n") } else { "        a;\n".to_string() }).collect::<String>();
        let (left, right) = (format!("mod m {{\n    fn old() {{\n{}    }}\n}}\n", body("x")), format!("mod m {{\n    fn new() {{\n{}    }}\n}}\n", body("y")));
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("left.rs"), left).unwrap();
        std::fs::write(dir.path().join("right.rs"), right).unwrap();
        let args = vec!["--no-index".into(), "left.rs".into(), "right.rs".into()];
        let mut app = App::new(Session::open(lenz::cli::parse(&args, dir.path()).unwrap(), dir.path().into()));
        ready(&mut app);
        app.session.scroll(10.0, Side::Right);
        (dir, app)
    }
    fn screen_rows(app: &mut App, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        terminal.backend().buffer().content.chunks(width as usize).map(|row| row.iter().map(|c| c.symbol()).collect()).collect()
    }
    #[test]
    fn sticky_headers_pin_in_each_pane_and_a_click_jumps() {
        let (_dir, mut app) = sticky_pair();
        let rows = screen_rows(&mut app, 120, 30);
        let body = (app.diff_area.y + 2) as usize;
        assert!(rows[body].contains("mod m {"), "{rows:#?}");
        assert!(rows[body + 1].contains("fn old() {") && rows[body + 1].contains("fn new() {"), "each side from its own text: {rows:#?}");
        assert!(rows[body + 2].contains("───"), "a rule under the sticky lines: {rows:#?}");
        assert_eq!(app.sticky_lines().map(|l| l.iter().map(|s| (s.line, s.row)).collect::<Vec<_>>()), [vec![(0, 0), (1, 1)], vec![(0, 0), (1, 1)]]);
        let (x, y) = (app.diff_area.x + app.left_width() + 12, app.diff_area.y + 3);
        pane_mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x, y);
        assert!(app.session.view().frame().right_top <= 1.0, "the click on fn new's sticky line shows it");
    }
    #[test]
    fn sticky_scroll_turned_on_after_a_parse_without_declarations() {
        let dir = tempfile::tempdir().unwrap();
        let body = |changed: &str| (0..80).map(|i| if i % 4 == 0 { format!("        {changed}{i};\n") } else { "        a;\n".to_string() }).collect::<String>();
        std::fs::write(dir.path().join("left.rs"), format!("mod m {{\n    fn old() {{\n{}    }}\n}}\n", body("x"))).unwrap();
        std::fs::write(dir.path().join("right.rs"), format!("mod m {{\n    fn new() {{\n{}    }}\n}}\n", body("y"))).unwrap();
        let args = vec!["--no-index".into(), "left.rs".into(), "right.rs".into()];
        let mut app = App::new(Session::open(lenz::cli::parse(&args, dir.path()).unwrap(), dir.path().into()));
        app.visual.sticky = false; // as `[view] sticky_scroll = false` starts it
        ready(&mut app);
        app.session.scroll(10.0, Side::Right);
        app.interaction(commands::Interaction::StickyScroll);
        ready(&mut app);
        assert_eq!(app.sticky_lines()[1].len(), 2, "the declarations are parsed when it turns on");
    }
    #[test]
    fn sticky_scroll_toggles_and_tiny_panes_pin_nothing() {
        let (_dir, mut app) = sticky_pair();
        app.interaction(commands::Interaction::StickyScroll);
        assert!(app.sticky_lines().iter().all(Vec::is_empty));
        let rows = screen_rows(&mut app, 120, 30);
        assert!(!rows[(app.diff_area.y + 2) as usize].contains("mod m {"), "off");
        app.interaction(commands::Interaction::StickyScroll);
        ready(&mut app);
        assert_eq!(app.sticky_lines()[1].len(), 2, "on again");
        let rows = screen_rows(&mut app, 120, 8);
        assert!(!rows.iter().any(|r| r.contains("mod m {")), "a pane under 6 rows pins nothing: {rows:#?}");
    }
    #[test]
    fn pane_drag_source_capture_unequal_hit_testing_reset_and_modal_guard() {
        let (dir,mut app)=pair("old alpha\n", "new beta\n");screen(&mut app,120,30);
        app.selection=Some(lenz::selection::Selection::caret(Side::Right,lenz::selection::Pos::new(0,4)));
        let selection=app.selection;let y=app.diff_area.y+2;let divider=app.diff_area.x+app.left_width();
        pane_mouse(&mut app,MouseEventKind::Down(MouseButton::Left),divider,y);
        pane_mouse(&mut app,MouseEventKind::Drag(MouseButton::Left),84,29);
        pane_mouse(&mut app,MouseEventKind::Up(MouseButton::Left),84,29);
        screen(&mut app,120,30);assert_eq!(app.left_width(),84);assert_eq!(app.selection,selection);
        let text=crate::render::text_geometry_ratio(app.diff_area,Side::Right,app.focus,false,app.pane_ratio);
        assert_eq!(app.mouse_position(text.x+4,y).unwrap().1.col,4);
        assert_eq!(app.pointer_side(70),Side::Left);
        let ratio=app.pane_ratio;app.handle(Event::Resize(160,40));screen(&mut app,160,40);assert_eq!(app.pane_ratio,ratio);assert!(app.left_width()>84);
        key(&mut app,KeyCode::Char(':'));let divider=app.diff_area.x+app.left_width();pane_mouse(&mut app,MouseEventKind::Down(MouseButton::Left),divider,y);pane_mouse(&mut app,MouseEventKind::Drag(MouseButton::Left),50,y);assert_eq!(app.pane_ratio,ratio);
        key(&mut app,KeyCode::Esc);app.interaction(commands::Interaction::ResetPaneSizes);screen(&mut app,160,40);assert_eq!(app.left_width(),80);
        let divider=app.diff_area.x+app.left_width();pane_mouse(&mut app,MouseEventKind::Down(MouseButton::Left),divider,y);app.handle(Event::FocusLost);pane_mouse(&mut app,MouseEventKind::Drag(MouseButton::Left),40,y);assert_eq!(app.pane_ratio,0.5);
        app.pane_ratio=0.7;for (w,h) in [(1,1),(20,4),(69,10),(160,40)] {app.handle(Event::Resize(w,h));screen(&mut app,w,h);assert_eq!(app.pane_ratio,0.7);}
        assert_eq!(std::fs::read_to_string(dir.path().join("right.txt")).unwrap(),"new beta\n");
    }
    #[test]
    fn pane_unequal_blame_keeps_source_visible_and_hittable() {
        let dir=tempfile::tempdir().unwrap();
        let git=|args:&[&str]| {let output=std::process::Command::new("git").args(["-c","user.name=Test","-c","user.email=test@example.invalid","-c","commit.gpgsign=false"]).args(args).current_dir(dir.path()).output().unwrap();assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));};
        git(&["init","-q"]);std::fs::write(dir.path().join("file.txt"),"old alpha\n").unwrap();git(&["add","file.txt"]);git(&["commit","-qm","old"]);
        std::fs::write(dir.path().join("file.txt"),"new beta\n").unwrap();git(&["commit","-qam","new"]);
        let request=lenz::cli::parse(&["HEAD~".into(),"HEAD".into()],dir.path()).unwrap();let mut app=App::open(request,dir.path().into());ready(&mut app);app.pane_ratio=0.1;assert!(app.visual.toggle_blame(Side::Left));ready(&mut app);assert!(app.visual.blame_enabled(Side::Left));
        let rendered=screen(&mut app,120,30);let source=crate::render::text_geometry_ratio(app.diff_area,Side::Left,app.focus,true,app.pane_ratio);
        assert!(source.width>=4);assert!(rendered.contains("old "));assert_eq!(app.mouse_position(source.x+2,source.y).unwrap().1.col,2);
        assert_eq!(source.x,app.diff_area.x+7+crate::render::annotation_width(app.left_width(),7));
    }
    #[test]
    fn pane_drag_sidebar_and_results_keep_file_and_text_selection() {
        let (_dir,mut app)=pair("old alpha\n","new beta\n");app.navigator.visible=Some(true);screen(&mut app,160,40);
        let before=app.navigator.area;let current=app.session.current();
        pane_mouse(&mut app,MouseEventKind::Down(MouseButton::Left),before.right()-1,before.y+2);pane_mouse(&mut app,MouseEventKind::Drag(MouseButton::Left),44,before.y+2);pane_mouse(&mut app,MouseEventKind::Up(MouseButton::Left),44,before.y+2);
        screen(&mut app,160,40);assert_eq!(app.navigator.area.width,45);assert_eq!(app.session.current(),current);
        let set=app.session.change_set().unwrap();let mut search=crate::search::Search::open(set.clone(),false,Default::default());
        search.model.execute(lenz::search_session::SearchCommand::Query("beta".into()),&set,6.0);search.model.execute(lenz::search_session::SearchCommand::Start,&set,6.0);app.set_search=Some(search);ready(&mut app);screen(&mut app,160,40);
        let grip=app.set_search.as_ref().unwrap().resizer.grips[0];let initial_height=app.diff_area.height;
        pane_mouse(&mut app,MouseEventKind::Down(MouseButton::Left),grip.area.x+2,grip.area.y);pane_mouse(&mut app,MouseEventKind::Drag(MouseButton::Left),grip.area.x+2,grip.area.y-5);pane_mouse(&mut app,MouseEventKind::Up(MouseButton::Left),grip.area.x+2,grip.area.y-5);
        screen(&mut app,160,40);assert_eq!(app.diff_area.height,initial_height-5);assert_eq!(app.session.current(),current);
        app.interaction(commands::Interaction::ResetPaneSizes);screen(&mut app,160,40);assert_eq!(app.navigator.area.width,before.width);assert_eq!(app.diff_area.height,initial_height);
    }

    pub fn ready(app: &mut App) {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            app.poll();
            if !app.busy() { break; }
            assert!(Instant::now() < until, "session failed to finish loading");
            thread::sleep(Duration::from_millis(2));
        }
    }
    pub fn pair(left: &str, right: &str) -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        pair_in(dir,left,right)
    }
    pub fn writable_pair(left: &str, right: &str) -> (tempfile::TempDir, App) {
        pair_in(tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap(),left,right)
    }
    fn pair_in(dir:tempfile::TempDir,left:&str,right:&str)->(tempfile::TempDir,App) {
        std::fs::write(dir.path().join("left.txt"), left).unwrap();
        std::fs::write(dir.path().join("right.txt"), right).unwrap();
        let args = vec!["--no-index".into(), "left.txt".into(), "right.txt".into()];
        let req = lenz::cli::parse(&args, dir.path()).unwrap();
        let mut app = App::new(Session::open(req, dir.path().into()));
        ready(&mut app);
        (dir, app)
    }
    pub fn key(app: &mut App, key: KeyCode) { app.handle(Event::Key(KeyEvent::new(key, KeyModifiers::NONE))); }
    pub fn screen(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        use unicode_width::UnicodeWidthStr;
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for row in buffer.content.chunks(width as usize) {
            let mut x = 0;
            while x < row.len() { let symbol = row[x].symbol(); text.push_str(symbol); x += symbol.width().max(1); }
        }
        text
    }

    #[test]
    fn pane_applied_history_drag_changes_viewport_without_undoing_or_writing() {
        let cwd=std::env::current_dir().unwrap();let dir=tempfile::Builder::new().prefix(".lenz-write-test-").tempdir_in(&cwd).unwrap();
        let left=dir.path().join("left");let right=dir.path().join("right");std::fs::write(&left,"old\n").unwrap();std::fs::write(&right,"new\n").unwrap();
        let req=lenz::cli::parse(&["--no-index".into(),left.display().to_string(),right.display().to_string()],&cwd).unwrap();let mut app=App::new(Session::open(req,cwd));ready(&mut app);
        app.handle(Event::Key(KeyEvent::new(KeyCode::Right,KeyModifiers::ALT|KeyModifiers::SHIFT)));ready(&mut app);screen(&mut app,120,30);
        let count=app.session.applied().len();let bytes=std::fs::read(&right).unwrap();let grip=app.resizer.grips.iter().find(|g|g.id==1).unwrap().area;let before=app.applied_panel_height();
        pane_mouse(&mut app,MouseEventKind::Down(MouseButton::Left),grip.x+5,grip.y);pane_mouse(&mut app,MouseEventKind::Drag(MouseButton::Left),grip.x+5,grip.y-4);pane_mouse(&mut app,MouseEventKind::Up(MouseButton::Left),grip.x+5,grip.y-4);screen(&mut app,120,30);
        assert_eq!(app.applied_panel_height(),before+4);assert_eq!(app.session.applied().len(),count);assert_eq!(std::fs::read(&right).unwrap(),bytes);
        app.interaction(commands::Interaction::ResetPaneSizes);screen(&mut app,120,30);assert_eq!(app.applied_panel_height(),before);
    }
    #[test]
    fn real_apply_history_click_and_undo_redo_preserve_disk_bytes(){
        let cwd=std::env::current_dir().unwrap();let dir=tempfile::Builder::new().prefix(".lenz-write-test-").tempdir_in(&cwd).unwrap();
        let left=dir.path().join("left");let right=dir.path().join("right");std::fs::write(&left,"old\n").unwrap();std::fs::write(&right,"new\n").unwrap();
        let req=lenz::cli::parse(&["--no-index".into(),left.display().to_string(),right.display().to_string()],&cwd).unwrap();let mut app=App::new(Session::open(req,cwd));ready(&mut app);
        app.handle(Event::Key(KeyEvent::new(KeyCode::Right,KeyModifiers::ALT|KeyModifiers::SHIFT)));ready(&mut app);assert_eq!(std::fs::read_to_string(&right).unwrap(),"new\n");assert_eq!(app.session.document().unwrap().text(Side::Right),"old\n");assert!(app.session.dirty());assert!(screen(&mut app,160,30).contains("Unsaved changes"));
        app.handle(Event::Key(KeyEvent::new(KeyCode::Char('s'),KeyModifiers::CONTROL)));ready(&mut app);assert_eq!(std::fs::read_to_string(&right).unwrap(),"old\n");assert!(!app.session.dirty());
        key(&mut app,KeyCode::Char('A'));assert!(screen(&mut app,160,30).contains("Enter to Undo"));key(&mut app,KeyCode::Enter);ready(&mut app);assert_eq!(std::fs::read_to_string(&right).unwrap(),"old\n");assert_eq!(app.session.document().unwrap().text(Side::Right),"new\n");assert!(screen(&mut app,160,30).contains("↶ Undone"));
        app.handle(Event::Key(KeyEvent::new(KeyCode::Char('s'),KeyModifiers::CONTROL)));ready(&mut app);assert_eq!(std::fs::read_to_string(&right).unwrap(),"new\n");
        for (code,expected) in [('z',"old\n"),('y',"new\n")] {app.handle(Event::Key(KeyEvent::new(KeyCode::Char(code),KeyModifiers::CONTROL)));ready(&mut app);assert_eq!(app.session.document().unwrap().text(Side::Right),expected);app.handle(Event::Key(KeyEvent::new(KeyCode::Char('s'),KeyModifiers::CONTROL)));ready(&mut app);assert_eq!(std::fs::read_to_string(&right).unwrap(),expected);}
        app.handle(Event::Key(KeyEvent::new(KeyCode::Right,KeyModifiers::ALT|KeyModifiers::SHIFT)));ready(&mut app);std::fs::write(&right,"external\n").unwrap();app.handle(Event::Key(KeyEvent::new(KeyCode::Char('s'),KeyModifiers::CONTROL)));ready(&mut app);assert_eq!(std::fs::read_to_string(&right).unwrap(),"external\n");assert!(app.session.notice().unwrap().to_lowercase().contains("changed on disk"));assert!(app.session.dirty());assert_eq!(app.session.document().unwrap().text(Side::Right),"old\n");
    }

    #[test]
    fn apply_uses_visible_change_when_the_old_caret_has_scrolled_away(){
        let cwd=std::env::current_dir().unwrap();let dir=tempfile::Builder::new().prefix(".lenz-write-test-").tempdir_in(&cwd).unwrap();
        let left=dir.path().join("left");let right=dir.path().join("right");
        let middle="same\n".repeat(100);let left_text=format!("old first\n{middle}old last\n");let right_text=format!("new first\n{middle}new last\n");
        std::fs::write(&left,&left_text).unwrap();std::fs::write(&right,&right_text).unwrap();
        let req=lenz::cli::parse(&["--no-index".into(),left.display().to_string(),right.display().to_string()],&cwd).unwrap();let mut app=App::new(Session::open(req,cwd));ready(&mut app);
        app.selection=Some(lenz::selection::Selection::caret(Side::Right,lenz::selection::Pos::new(0,0)));
        app.session.go_to_line(Side::Right,102);assert!(!app.session.view().line_visible(Side::Right,0));
        app.handle(Event::Key(KeyEvent::new(KeyCode::Right,KeyModifiers::ALT|KeyModifiers::SHIFT)));ready(&mut app);
        assert_eq!(std::fs::read_to_string(&right).unwrap(),right_text);assert_eq!(app.session.document().unwrap().text(Side::Right),format!("new first\n{middle}old last\n"));
        app.handle(Event::Key(KeyEvent::new(KeyCode::Char('s'),KeyModifiers::CONTROL)));ready(&mut app);
        assert_eq!(std::fs::read_to_string(&right).unwrap(),format!("new first\n{middle}old last\n"));
    }
    #[test]
    fn line_column_selection_mouse_and_pending_file_line_share_byte_positions(){
        let(_dir,mut app)=pair("a\n", "安全 café\n");screen(&mut app,120,24);key(&mut app,KeyCode::Char('g'));for c in "1:4".chars(){key(&mut app,KeyCode::Char(c));}key(&mut app,KeyCode::Enter);
        assert_eq!(app.selection.unwrap().head.col,7);app.handle(Event::Key(KeyEvent::new(KeyCode::Right,KeyModifiers::SHIFT)));assert_eq!(app.selection.unwrap().text(app.session.document().unwrap().lines_on(Side::Right)),"c");
        app.handle(Event::Mouse(crossterm::event::MouseEvent{kind:MouseEventKind::Down(MouseButton::Left),column:70,row:app.diff_area.y+2,modifiers:KeyModifiers::NONE}));assert_eq!(app.selection.unwrap().head.col,3);
        app.handle(Event::Mouse(crossterm::event::MouseEvent{kind:MouseEventKind::Drag(MouseButton::Left),column:73,row:app.diff_area.y+2,modifiers:KeyModifiers::NONE}));assert!(!app.selection.unwrap().is_empty());
    }

    #[test]
    fn escape_collapses_selection_to_its_visible_caret(){
        let (_dir,mut app)=pair("a\n","some text\n");
        app.selection=Some(lenz::selection::Selection{side:Side::Right,anchor:lenz::selection::Pos::new(0,0),head:lenz::selection::Pos::new(0,4)});
        key(&mut app,KeyCode::Esc);let selection=app.selection.unwrap();assert!(selection.is_empty());assert_eq!(selection.head.col,4);
        assert!(screen(&mut app,120,24).contains("some text"));
    }

    #[test]
    fn caret_scroll_keeps_insertion_visible_in_tiny_and_wide_text_panes(){
        for width in 1..80 {let mut offset=0;for column in (0..160).chain((0..160).rev()){offset=caret_scroll(column,offset,width);assert!(column>=offset&&column<offset+width,"column {column}, offset {offset}, width {width}");}}
    }
    #[test]
    fn real_keys_fold_multiline_changes_and_render_their_summary() {
        let (_dir, mut app) = pair("old one\nold two\n", "new one\nnew two\n");
        assert!(screen(&mut app, 100, 20).contains("new one"));
        key(&mut app, KeyCode::Char('x'));
        assert!(app.session.view().folded(0));
        let rendered = screen(&mut app, 100, 20);
        assert!(rendered.contains("2 / 2"), "{rendered}");
        assert!(!rendered.contains("new one"));
        key(&mut app, KeyCode::Char('X'));
        assert!(screen(&mut app, 100, 20).contains("new one"));
    }

    #[test]
    fn one_line_fold_is_disabled_in_the_command_menu() {
        let (_dir, mut app) = pair("old\n", "new\n");
        key(&mut app, KeyCode::Char(':'));
        for ch in "fold/unfold".chars() { key(&mut app, KeyCode::Char(ch)); }
        assert!(screen(&mut app, 120, 20).contains("no multiline change selected"));
        key(&mut app, KeyCode::Enter);
        assert!(!app.session.view().folded(0));
    }

    fn alt(app: &mut App, ch: char) { app.handle(Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::ALT))); }

    #[test]
    fn find_prompt_shows_and_toggles_options() {
        let (_dir, mut app) = pair("x\n", "Foo foo food\n");
        key(&mut app, KeyCode::Char('/'));
        for ch in "foo".chars() { key(&mut app, KeyCode::Char(ch)); }
        assert_eq!(app.session.find().unwrap().hits().len(), 3);
        alt(&mut app, 'c');
        assert_eq!(app.session.find().unwrap().hits(), [(0, 4..7), (0, 8..11)]);
        alt(&mut app, 'w');
        assert_eq!(app.session.find().unwrap().hits(), [(0, 4..7)], "the same matches as the window's find bar");
        assert!(crate::render::status_text(&app).starts_with("Find [Aa W] foo"), "{}", crate::render::status_text(&app));
        alt(&mut app, 'r');
        key(&mut app, KeyCode::Char('('));
        assert!(crate::render::status_text(&app).ends_with("unclosed group"), "{}", crate::render::status_text(&app));
        key(&mut app, KeyCode::Esc);
        assert!(app.prompt.is_none(), "a lone Escape still closes the prompt");
        let ws = app.session.options().whitespace;
        key(&mut app, KeyCode::Char('w'));
        ready(&mut app);
        assert_ne!(app.session.options().whitespace, ws, "a plain w after Escape is the normal key");
        assert!(app.session.find_options().word && app.session.find_options().regex, "options stay for the window");
    }

    #[test]
    fn go_to_line_reports_bad_input() {
        let (_dir, mut app) = pair("a\nb\n", "a\nc\n");
        key(&mut app, KeyCode::Char('g'));
        for ch in "x1".chars() { key(&mut app, KeyCode::Char(ch)); }
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.session.notice(), Some("expected line or line:column"));
        key(&mut app, KeyCode::Char('g'));
        for ch in "2,1".chars() { key(&mut app, KeyCode::Char(ch)); }
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.selection.map(|s| s.head.line), Some(1));
    }

    #[test]
    fn change_set_search_takes_options_and_changed_only_is_alt_o() {
        let (_dir, mut app) = pair("needle Needle\n", "needle\n");
        app.interaction(commands::Interaction::FindSet);
        for ch in "Needle".chars() { key(&mut app, KeyCode::Char(ch)); }
        alt(&mut app, 'c');
        let search = app.set_search.as_ref().unwrap();
        assert!(search.model.options().case && app.session.find_options().case);
        assert!(search.prompt_text().contains("[Aa]"), "{}", search.prompt_text());
        alt(&mut app, 'o');
        assert!(app.set_search.as_ref().unwrap().model.changed_only());
        assert!(app.set_search.as_ref().unwrap().prompt_text().contains("Alt-O changed lines only: on"));
    }

    #[test]
    fn search_and_options_use_shared_behavior() {
        let (_dir, mut app) = pair("café\nother\ncafé\n", "CAFÉ\nother\nCAFÉ\n");
        key(&mut app, KeyCode::Char('/'));
        for ch in "café".chars() { key(&mut app, KeyCode::Char(ch)); }
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.session.find().unwrap().hits().len(), 2);assert_eq!(app.selection.unwrap().text(app.session.document().unwrap().lines_on(Side::Right)),"CAFÉ");
        key(&mut app, KeyCode::F(3));
        assert_eq!(app.session.find().unwrap().at(), Some(1));
        key(&mut app, KeyCode::Char('i'));
        ready(&mut app);
        assert!(app.session.options().ignore_case);
        assert!(app.session.view().frame().connectors.iter().all(|c| c.ignored));
    }

    #[test]
    fn opening_find_recomputes_for_the_focused_side_and_caret() {
        let (_dir, mut app) = pair("word word\n", "word word word\n");
        app.session.search("word".into(), Side::Right);
        app.focus=Side::Left;
        app.selection=Some(lenz::selection::Selection::caret(Side::Left,lenz::selection::Pos::new(0,5)));
        key(&mut app,KeyCode::Char('/'));
        let find=app.session.find().unwrap();
        assert_eq!(find.side(),Side::Left);
        assert_eq!(find.hits().len(),2);
        assert_eq!(find.at(),Some(1));
        assert_eq!(app.selection.unwrap().ordered().0.col,5);
    }

    #[test]
    fn ordinary_find_navigation_takes_priority_over_change_set_search() {
        let (_dir, mut app) = pair("word\nword\n", "word\nword\nword\n");
        let set=app.session.change_set().unwrap();
        let mut search=crate::search::Search::open(set.clone(),false,Default::default());
        search.model.execute(lenz::search_session::SearchCommand::Query("word".into()),&set,6.0);
        search.model.execute(lenz::search_session::SearchCommand::Start,&set,6.0);
        for _ in 0..1000 {search.poll();if !search.busy(){break;}std::thread::sleep(std::time::Duration::from_millis(1));}
        assert!(!search.model.hits().is_empty());search.focused=false;
        app.set_search=Some(search);
        key(&mut app,KeyCode::Char('/'));
        for ch in "word".chars(){key(&mut app,KeyCode::Char(ch));}
        key(&mut app,KeyCode::Enter);
        let set_at=app.set_search.as_ref().unwrap().model.at();
        key(&mut app,KeyCode::F(3));
        assert_eq!(app.session.find().unwrap().at(),Some(1));
        assert_eq!(app.set_search.as_ref().unwrap().model.at(),set_at);
    }

    #[test]
    fn reopening_change_set_find_retains_query_and_changed_only() {
        let (_dir, mut app) = pair("word\n", "word changed\n");
        app.interaction(commands::Interaction::FindSet);
        let set=app.session.change_set().unwrap();
        let search=app.set_search.as_mut().unwrap();
        search.model.execute(lenz::search_session::SearchCommand::Query("word".into()),&set,6.0);
        search.model.execute(lenz::search_session::SearchCommand::ToggleChangedOnly,&set,6.0);
        search.model.execute(lenz::search_session::SearchCommand::Start,&set,6.0);
        search.focused=false;
        app.handle(Event::Key(KeyEvent::new(KeyCode::Char('f'),KeyModifiers::CONTROL|KeyModifiers::SHIFT)));
        let search=app.set_search.as_ref().unwrap();
        assert_eq!(search.model.query(),"word");
        assert!(search.model.changed_only());assert!(search.model.editing());
    }

    #[test]
    fn option_recompare_preserves_visible_rows_find_selection_and_visual_jobs() {
        let (_dir, mut app) = pair("word old\n", "word new\n");
        key(&mut app,KeyCode::Char('/'));for ch in "word".chars(){key(&mut app,KeyCode::Char(ch));}key(&mut app,KeyCode::Enter);
        ready(&mut app);
        let selection=app.selection.unwrap();let weak=app.visual.cancellation_weak().unwrap();
        let doc=app.session.document_arc().unwrap();
        key(&mut app,KeyCode::Char('w'));
        assert!(!app.session.view().frame().right.is_empty());
        app.poll();
        assert!(std::sync::Arc::ptr_eq(&doc,&app.session.document_arc().unwrap()));
        assert_eq!(app.session.find().unwrap().query(),"word");
        assert_eq!(app.selection.unwrap(),selection);assert!(weak.upgrade().is_some());
        ready(&mut app);assert_eq!(app.selection.unwrap(),selection);assert!(weak.upgrade().is_some());
    }

    #[test]
    fn image_mode_and_zoom_survive_text_option_recompare(){
        let dir=tempfile::tempdir().unwrap();
        for (name,color) in [("left.bmp",10),("right.bmp",200)]{
            let mut bytes=vec![0u8;58];bytes[..2].copy_from_slice(b"BM");
            for (offset,value) in [(2,58u32),(10,54),(14,40),(18,1),(22,1),(34,4)]{bytes[offset..offset+4].copy_from_slice(&value.to_le_bytes());}
            bytes[26..28].copy_from_slice(&1u16.to_le_bytes());bytes[28..30].copy_from_slice(&24u16.to_le_bytes());bytes[54..57].fill(color);
            std::fs::write(dir.path().join(name),bytes).unwrap();
        }
        let req=lenz::cli::parse(&["--no-index".into(),dir.path().join("left.bmp").display().to_string(),dir.path().join("right.bmp").display().to_string()],dir.path()).unwrap();
        let mut app=App::new(Session::open(req,dir.path().into()));ready(&mut app);screen(&mut app,120,24);
        key(&mut app,KeyCode::Char('m'));key(&mut app,KeyCode::Char('+'));
        let mode=app.visual.image_state().mode();let viewport=crate::visual::image_viewport(app.visual.image_state(),app.image_area());let zoom=app.visual.image_state().zoom(app.visual.image().unwrap(),viewport);let weak=app.visual.cancellation_weak().unwrap();
        key(&mut app,KeyCode::Char('w'));ready(&mut app);
        assert_eq!(app.visual.image_state().mode(),mode);assert_eq!(app.visual.image_state().zoom(app.visual.image().unwrap(),viewport),zoom);assert!(weak.upgrade().is_some());
    }

    #[test]
    fn terminal_controls_are_visible_and_unicode_survives_resize() {
        let (_dir, mut app) = pair("a\n", "安全 e\u{301} 👩‍💻\t\u{1b}[31m\n");
        let rendered = screen(&mut app, 100, 12);
        assert!(rendered.contains("安全 e\u{301} 👩‍💻"), "{rendered}");
        assert!(!rendered.contains('\u{1b}'));
        for (w, h) in [(1, 1), (20, 4), (80, 24), (200, 60)] {
            app.handle(Event::Resize(w, h));
            screen(&mut app, w, h);
        }
        key(&mut app, KeyCode::Char('q'));
        assert!(app.quit);
    }

    #[test]
    fn rapid_file_switches_discard_old_results_and_keep_folds() {
        let dir = tempfile::tempdir().unwrap();
        for side in ["old", "new"] {
            std::fs::create_dir(dir.path().join(side)).unwrap();
            for file in ["a", "b", "c"] {
                std::fs::write(dir.path().join(side).join(file), format!("{side} {file} first\n{side} {file} second\n")).unwrap();
            }
        }
        let args = vec!["--no-index".into(), "old".into(), "new".into()];
        let req = lenz::cli::parse(&args, dir.path()).unwrap();
        let mut app = App::new(Session::open(req, dir.path().into()));
        ready(&mut app);
        key(&mut app, KeyCode::Char('x'));
        app.pane_ratio=0.7;
        assert!(app.session.view().folded(0));
        for _ in 0..20 {
            key(&mut app, KeyCode::Char(']'));
            key(&mut app, KeyCode::Char(']'));
            key(&mut app, KeyCode::Char('['));
            key(&mut app, KeyCode::Char('['));
        }
        ready(&mut app);
        assert_eq!(app.session.current(), 0);
        assert_eq!(app.pane_ratio,0.7);
        assert!(app.session.view().folded(0));
        key(&mut app, KeyCode::Char('X'));
        assert!(screen(&mut app, 100, 20).contains("new a first"));
        assert!(!app.session.failed());
    }

    #[test]
    fn binary_and_oversized_inputs_show_placeholders_without_decoding() {
        let (_dir, mut app) = pair("\0old", "\0new");
        assert!(screen(&mut app, 140, 20).contains("Binary / image comparison"));
        let dir = tempfile::tempdir().unwrap();
        for name in ["left", "right"] {
            std::fs::File::create(dir.path().join(name)).unwrap().set_len(lenz::document::LARGE + if name == "left" { 1 } else { 2 }).unwrap();
        }
        let req = lenz::cli::parse(&["--no-index".into(), "left".into(), "right".into()], dir.path()).unwrap();
        let mut app = App::new(Session::open(req, dir.path().into()));
        ready(&mut app);
        assert!(screen(&mut app, 100, 20).contains("Oversized comparison"));
        assert!(app.session.document().unwrap().lines_on(Side::Right).iter().all(String::is_empty));
    }


    #[test]
    fn mouse_word_line_and_offscreen_drag_use_shared_selection(){
        let (_dir,mut app)=pair("old word\n",&("new word\n".repeat(80)));screen(&mut app,100,20);
        let source_y=app.diff_area.y+2;let click=|kind,row|Event::Mouse(crossterm::event::MouseEvent{kind,column:62,row,modifiers:KeyModifiers::NONE});
        app.handle(click(MouseEventKind::Down(MouseButton::Left),source_y));app.handle(click(MouseEventKind::Down(MouseButton::Left),source_y));
        assert_eq!(app.selection.unwrap().text(app.session.document().unwrap().lines_on(Side::Right)),"word");
        app.handle(click(MouseEventKind::Down(MouseButton::Left),source_y));assert_eq!(app.selection.unwrap().text(app.session.document().unwrap().lines_on(Side::Right)),"new word\n");
        let before=app.session.view().anchor_line_on(Side::Right);app.handle(click(MouseEventKind::Drag(MouseButton::Left),19));assert!(app.session.view().anchor_line_on(Side::Right)>before);assert!(!app.selection.unwrap().is_empty());
    }
    #[test]
    fn independent_scroll_uses_the_focused_side_as_lead() {
        let left = (0..100).map(|i| format!("old {i}\n")).collect::<String>();
        let right = (0..200).map(|i| format!("new {i}\n")).collect::<String>();
        let (_dir, mut app) = pair(&left, &right);
        screen(&mut app, 100, 20);
        key(&mut app, KeyCode::Char('s'));
        let before = app.session.view().frame();
        key(&mut app, KeyCode::Down);
        let frame = app.session.view().frame();
        assert_eq!(app.session.view().mode(), lenz::view::ScrollMode::Aligned);
        assert!((frame.right_top - before.right_top - 1.0).abs() < 0.0001);
    }

    #[test]
    fn missing_paths_report_an_error_without_hanging() {
        let dir = tempfile::tempdir().unwrap();
        let req = lenz::cli::parse(&["--no-index".into(), "missing".into(), "absent".into()], dir.path()).unwrap();
        let mut app = App::new(Session::open(req, dir.path().into()));
        ready(&mut app);
        assert!(app.session.failed());
        assert!(app.session.notice().is_some());
        key(&mut app, KeyCode::Char('q'));
        assert!(app.quit);
    }

    #[test]
    #[ignore = "release terminal rendering and input benchmark"]
    fn terminal_perf() {
        use crate::perf;
        let left = (0..100_000).map(|i| format!("old {i} café 👩‍💻\n")).collect::<String>();
        let right = left.replace("old", "new");
        let (_dir, mut app) = pair(&left, &right);
        let mut terminal = Terminal::new(TestBackend::new(160, 50)).unwrap();
        let mut samples = Vec::new();
        let mut redraw_bytes = Vec::new();
        for _ in 0..300 {
            let started = Instant::now();
            key(&mut app, KeyCode::Down);
            terminal.draw(|f| app.render(f)).unwrap();
            samples.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        assert!(app.session.view().anchor_line_on(Side::Right) >= 299.0);
        for (w, h) in [(80, 24), (200, 60), (30, 10)] {
            let started = Instant::now();
            terminal.backend_mut().resize(w, h);
            terminal.resize(ratatui::layout::Rect::new(0, 0, w, h)).unwrap();
            terminal.draw(|f| app.render(f)).unwrap();
            redraw_bytes.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        perf::record("tui/scroll-render-100k", "ms", &samples);
        perf::record("tui/resize", "ms", &redraw_bytes);
        if let Some(rss) = perf::rss_mib() { perf::record("tui/resident-100k", "MiB", &[rss]); }
        let (_dir, mut app) = pair("old\n", &"word ".repeat(1_000_000));
        app.session.search("word".into(), Side::Right);
        let mut times = Vec::new();
        for _ in 0..30 {
            let start = Instant::now();
            terminal.draw(|f| app.render(f)).unwrap();
            times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        perf::record("tui/long-line-find-render", "ms", &times);
        times.sort_by(f64::total_cmp);
        perf::check(times[times.len() / 2] < 16.0, "terminal long-line repaint exceeds 16 ms");
    }
}

#[cfg(test)]
#[path = "chrome_tests.rs"]
mod chrome_tests;

/// The line and 1-based character column a Line prompt asks for (column 1 when none is given).
fn lenz_line(input: &str) -> Result<(u32, usize), lenz::goto::GotoError> {
    lenz::goto::parse_line_column(input).map(|at| (at.line, at.column.unwrap_or(1) as usize))
}
