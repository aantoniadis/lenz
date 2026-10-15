//! Shared comparison workflow for the terminal adapter. All state decisions use ComparisonSession.
use std::{path::PathBuf,sync::{Arc,mpsc::{Receiver,TryRecvError}},time::Instant};
use lenz_engine::{Budget,Options,Refinement};
use crate::{changeset::{ChangeSet,Comparison,Spec},cli::Request,commands::{self,Command},
    comparison_session::{self,ComparisonSession,Place,Transition},document::{TextDocument,PreparedDocument,Placeholder},
    edit_session::{EditSession,EditCommand,EditError,Applied},find::Find,selection::Pos,
    view::{Content,Side,View,ScrollMode},work,write::WriteError};
type Inbox<T> = Receiver<Option<T>>;
type Refinements = Vec<(usize,Refinement)>;

pub struct Session {
    state: ComparisonSession,
    set: Option<Arc<ChangeSet>>,
    spec: Spec,
    opening: Option<Inbox<Result<ChangeSet,String>>>,
    opening_preserve:bool,
    reading: Option<Inbox<Result<PreparedDocument,String>>>,
    diffing: Option<Inbox<(View,Budget)>>,
    refining_job: Option<Inbox<(Refinements,Budget)>>,
    document: Option<Arc<TextDocument>>,
    image: Option<Result<Arc<crate::image_session::ImageDoc>,String>>,
    name: Option<String>,
    cwd:PathBuf,
    place: Place,
    diff_started: Option<Instant>,
    notice: Option<String>,
    failed: bool,
    writes: EditSession,
    file_actions:crate::file_actions::FileActions,
}
impl std::ops::Deref for Session { type Target=ComparisonSession;fn deref(&self)->&Self::Target{&self.state} }
impl std::ops::DerefMut for Session {fn deref_mut(&mut self)->&mut Self::Target{&mut self.state} }
impl Session {
    pub fn open(request: Request,cwd:PathBuf)->Self {
        let mut state=ComparisonSession::new(View::new(0,0,&[],24.0),request.options);state.view.start_in(request.scroll);
        let spec=request.spec;let load=spec.clone();let load_cwd=cwd.clone();
        let opening=Some(work::submit(Arc::downgrade(&state.work_id),move||ChangeSet::load(&load,&load_cwd)));
        Self{state,set:None,spec,opening,opening_preserve:false,reading:None,diffing:None,refining_job:None,document:None,image:None,name:request.name,cwd,
            place:request.line.map_or(Place::FirstChange,|n|Place::Line(Side::Right,n)),diff_started:None,notice:None,failed:false,writes:EditSession::default(),file_actions:Default::default()}
    }
    pub fn from_set(set:Arc<ChangeSet>,spec:Spec,options:Options,mode:ScrollMode)->Self{
        Self::from_set_named(set,spec,options,mode,None,None)
    }
    pub fn from_set_named(set:Arc<ChangeSet>,spec:Spec,options:Options,mode:ScrollMode,name:Option<String>,line:Option<u32>)->Self{
        let cwd=set.git_root().map(PathBuf::from).unwrap_or_else(||std::env::current_dir().unwrap_or_default());
        let mut state=ComparisonSession::new(View::new(0,0,&[],24.0),options);state.view.start_in(mode);
        let mut session=Self{state,set:Some(set),spec,opening:None,opening_preserve:false,reading:None,diffing:None,refining_job:None,document:None,image:None,
            name,cwd,place:Place::FirstChange,diff_started:None,notice:None,failed:false,writes:EditSession::default(),file_actions:Default::default()};
        session.load(0,line.map_or(Place::FirstChange,|n|Place::Line(Side::Right,n)),false,false);session
    }
    pub fn replace_set(&mut self,mut set:Arc<ChangeSet>,spec:Spec,preserve:bool){
        if self.has_dirty() && (spec!=self.spec || !preserve) {
            if let Some(index)=self.dirty_files().first().copied(){self.open_file(index);}
            self.notice=Some("Unsaved changes — Save or Discard unsaved changes before changing comparison".into());return;
        }
        if preserve&&self.file_actions.has_undo(){self.file_actions.retain_files(Arc::make_mut(&mut set));}
        let anchor=self.view().anchor_line();let old=self.set.clone();let identity=old.as_ref().and_then(|set|crate::changeset::comparison_identity(set,self.current()));
        if preserve{if let Some(old)=&old{if self.writes.has_applied()||self.writes.has_dirty(){self.writes.retain_applied_files(old,Arc::make_mut(&mut set));}self.writes.remap_files(old,&set);self.state.remap_files(old,&set);}}
        else{self.writes=EditSession::default();}
        let index=if preserve{identity.and_then(|identity|(0..set.comparisons.len()).find(|&i|crate::changeset::comparison_identity(&set,i).as_ref()==Some(&identity))).unwrap_or(0)}else{0};
        if !preserve{self.state.folds.clear();}
        self.set=Some(set);self.spec=spec;self.opening=None;self.reading=None;self.diffing=None;self.refining_job=None;self.state.begin();self.state.markdown_invalidate();self.document=None;self.image=None;
        self.load(index,if preserve{Place::Anchor(anchor)}else{Place::FirstChange},false,false);
    }
    pub fn change_set(&self)->Option<Arc<ChangeSet>>{self.set.clone()}
    pub fn document_arc(&self)->Option<Arc<TextDocument>>{self.document.clone()}
    pub fn image_result(&self)->Option<Result<Arc<crate::image_session::ImageDoc>,String>>{self.image.clone()}
    pub fn spec(&self)->&Spec{&self.spec}
    pub fn revision(&self)->u64{self.state.revision()}
    pub fn poll(&mut self)->bool{
        let mut changed=false;
        if let Some(result)=self.file_actions.poll(){changed=true;match result{Ok(Some(_))=>{self.reload_set();self.notice=Some("File updated".into());},Ok(None)=>{},Err(error)=>self.notice=Some(format!("Nothing written: {error}"))}}

        if let Some(rx)=&self.opening{
            match rx.try_recv(){
                Ok(Some(Ok(set)))=>{self.opening=None;if self.opening_preserve{self.opening_preserve=false;self.replace_set(Arc::new(set),self.spec.clone(),true);}else{self.set=Some(Arc::new(set));self.load(0,self.place,false,false);}changed=true;},
                Ok(Some(Err(e)))=>{self.opening=None;self.recover_failure(e);changed=true;},
                Ok(None)=>{self.opening=None;self.recover_failure("Comparison loading was interrupted".into());changed=true;},
                Err(TryRecvError::Disconnected)=>{self.opening=None;self.interrupted();changed=true;},
                Err(TryRecvError::Empty)=>{}
            }
        }
        if let Some(rx)=&self.reading{
            match rx.try_recv(){
                Ok(Some(Ok(prepared)))=>{self.reading=None;let (lp,rp)=self.paths(self.state.current);let text=if prepared.image.is_some(){self.writes.recover(self.state.current,[&lp,&rp])}else{self.writes.overlay(self.state.current,&prepared.text,[&lp,&rp])}.unwrap_or(prepared.text);let doc=Arc::new(text);self.image=if self.writes.dirty(self.current()){None}else{prepared.image.map(|result|result.map(Arc::new))};self.state.show_text(&doc,self.place);self.document=Some(doc);self.compare();changed=true;},
                Ok(Some(Err(e)))=>{self.reading=None;self.recover_failure(e);changed=true;},
                Ok(None)=>{self.reading=None;self.recover_failure("Comparison loading was interrupted".into());changed=true;},
                Err(TryRecvError::Disconnected)=>{self.reading=None;self.interrupted();changed=true;},
                Err(TryRecvError::Empty)=>{}
            }
        }
        if let Some(rx)=&self.diffing{
            match rx.try_recv(){
                Ok(Some((mut view,budget)))=>{
                    self.diffing=None;self.diff_started=None;view.set_viewport_rows(self.state.view.viewport_rows());
                    if let Place::Line(side,line)=self.place {view.go_to_line(side,line);}
                    let next=self.state.complete(self.state.generation,view,budget,self.place,self.comparisons().len());
                    if let Some(doc)=&self.document{self.state.restore_position(doc,self.place);self.state.markdown_refresh(Some(doc));}
                    if let Some(index)=next{self.load(index,self.place,false,false);}changed=true;
                },
                Ok(None)|Err(TryRecvError::Disconnected)=>{self.diffing=None;self.diff_started=None;},
                Err(TryRecvError::Empty)=>{}
            }
        }
        if self.diff_started.is_some_and(|t|t.elapsed()>=std::time::Duration::from_secs(2)) && let Some(doc)=&self.document{
            let revision=self.state.generation;
            if self.state.expire(revision,doc){self.state.markdown_refresh(Some(doc));self.diffing=None;self.diff_started=None;changed=true;}
        }
        if let Some(rx)=&self.refining_job{
            match rx.try_recv(){
                Ok(Some((results,budget)))=>{self.state.view.apply_refinements(results);self.state.budget=budget;self.refining_job=None;self.state.markdown_refresh(self.document.as_deref());changed=true;},
                Ok(None)|Err(TryRecvError::Disconnected)=>self.refining_job=None,
                Err(TryRecvError::Empty)=>{}
            }
        }
        if !self.loading() && self.refining_job.is_none() && self.state.banner.is_none() && let Some(doc)=&self.document{
            let todo=self.state.refine_targets();
            if !todo.is_empty(){let(doc,options,mut budget)=(doc.clone(),self.state.options,self.state.budget.clone());
                self.refining_job=Some(work::submit(Arc::downgrade(&self.state.work_id),move||{
                    let refined=doc.refine(todo,&options,&mut budget);(refined,budget)
                }));
            }
        }
        changed |= self.state.markdown_poll();
        changed
    }
    fn interrupted(&mut self){crate::diagnostics::document_interrupted();let message="Comparison loading was interrupted — restart Lenz; Report last error includes the worker diagnostics".to_string();if !self.restore_draft(&message){self.notice=Some(message);self.failed=true;}}
    fn recover_failure(&mut self,error:String){if !self.restore_draft(&error){self.error(error);}}
    fn restore_draft(&mut self,error:&str)->bool {
        if self.set.as_ref().is_some_and(|s|self.current()<s.comparisons.len()) {
            let(lp,rp)=self.paths(self.current());if let Some(doc)=self.writes.recover(self.current(),[&lp,&rp]) {
                let doc=Arc::new(doc);self.state.show_text(&doc,self.place);self.document=Some(doc);self.failed=false;self.compare();self.notice=Some(format!("Draft retained: {error}"));return true;
            }
        }
        false
    }
    fn error(&mut self,e:String){crate::diagnostics::comparison_failure(&e);self.notice=Some(format!("{e} — check inputs, then reload; Report last error opens diagnostics"));self.failed=true;}
    fn paths(&self,index:usize)->(PathBuf,PathBuf){let c=&self.comparisons()[index];let name=self.name.as_deref().filter(|n|!n.is_empty());
        (PathBuf::from(name.unwrap_or(&c.old_path)),PathBuf::from(name.unwrap_or(&c.path)))}
    fn load(&mut self,index:usize,place:Place,force:bool,reuse:bool){
        let Some(set)=self.set.clone().filter(|s|index<s.comparisons.len())else{if self.comparisons().is_empty(){self.notice=Some("No differences".into());}return;};
        if index!=self.state.current && let Some(doc)=&self.document{self.state.remember_position(doc);}
        let doc=reuse.then(||self.document.clone()).flatten();let rows=self.state.view.viewport_rows();let mode=self.state.view.mode();
        self.state.begin();self.state.current=index;self.place=place;self.refining_job=None;self.diffing=None;self.diff_started=None;
        if let Some(doc)=doc{self.reading=None;self.document=Some(doc);self.notice=None;self.failed=false;self.compare();return;}
        self.state.markdown_invalidate();self.state.find=None;self.notice=None;self.failed=false;self.document=None;if !reuse{self.image=None;}self.state.view=View::new(0,0,&[],rows);self.state.view.start_in(mode);
        let(lp,rp)=self.paths(index);
        self.reading=Some(work::submit(Arc::downgrade(&self.state.work_id),move||PreparedDocument::load(&set,index,&lp,&rp,force)));
    }
    fn compare(&mut self){
        let Some(doc)=self.document.clone()else{return;};
        if doc.placeholder_kind().is_some()||self.image.as_ref().is_some_and(|r|r.is_ok()){self.state.diffed=self.state.generation;return;}
        let(options,rows,mode,collapse,folds,place)=(self.state.options,self.state.view.viewport_rows(),self.state.view.mode(),self.state.collapse,
            self.state.folds.get(&self.state.current).cloned().unwrap_or_default(),self.place);
        let position=matches!(place,Place::Anchor(_)).then(||self.state.view.recompare_position());
        self.diff_started=Some(Instant::now());
        self.diffing=Some(work::submit(Arc::downgrade(&self.state.work_id),move||comparison_session::prepare_with_position(&doc,options,rows,mode,collapse,&folds,place,position)));
    }
    pub fn comparisons(&self)->&[Comparison]{self.set.as_ref().map_or(&[],|s|&s.comparisons)}
    pub fn labels(&self)->(&str,&str){self.comparisons().get(self.state.current).map_or(("Left","Right"),|c|{
        let name=self.name.as_deref().filter(|n|!n.is_empty());(name.unwrap_or(&c.old_path),name.unwrap_or(&c.path))})}
    pub fn current(&self)->usize{self.state.current}
    pub fn document(&self)->Option<&TextDocument>{self.document.as_deref()}
    pub fn view(&self)->&View{&self.state.view}
    pub fn options(&self)->Options{self.state.options}
    /// The directory the comparison's paths are relative to.
    pub fn cwd(&self)->&std::path::Path{&self.cwd}
    /// Unchanged lines outside changes are folded away.
    pub fn context_hidden(&self)->bool{self.state.collapse}
    pub fn loading(&self)->bool{self.opening.is_some()||self.reading.is_some()||self.diffing.is_some()}
    pub fn busy(&self)->bool{self.markdown_loading()||self.file_actions.busy()||self.loading()||self.refining_job.is_some()}
    pub fn failed(&self)->bool{self.failed}
    pub fn notice(&self)->Option<&str>{self.notice.as_deref().or(self.state.markdown_notice()).or(self.state.banner.as_deref())}
    pub fn set_notice(&mut self,notice:impl Into<String>){self.notice=Some(notice.into());}
    pub fn find(&self)->Option<&Find>{self.state.find.as_ref()}
    pub fn disabled(&self,command:Command)->Option<&'static str>{if let Command::SetPresentation(mode)=command{return self.state.presentation_disabled(mode,self.document());}
        if let Command::Preview(_)=command{return self.state.presentation_disabled(crate::markdown::PresentationMode::Rendered,self.document());}commands::disabled(command,&self.state.view,self.loading()||self.document.is_none(),self.state.current,
        self.comparisons().len(),self.document.as_ref().is_some_and(|d|matches!(d.placeholder_kind(),Some(Placeholder::Large{..}))))}
    pub fn resize(&mut self,rows:u16){self.state.view.set_viewport_rows(f32::from(rows.max(1)));}
    pub fn scroll(&mut self,rows:f32,side:Side){self.state.press_again.reset();self.state.view.scroll_by(rows,side);}
    pub fn go_to_line(&mut self,side:Side,line:u32){self.state.go_to_line(side,line);}
    pub fn expand(&mut self,side:Side,row:u16){let frame=self.state.view.frame();let rows=if side==Side::Left{frame.left}else{frame.right};
        if let Some(r)=rows.iter().find(|r|r.y>=0.0&&r.y as u16==row){match r.content{
            Content::Fold{id,..}=>self.state.view.expand(id),Content::Folded{block,..}=>{if let Some(doc)=self.document.clone(){self.state.fold(&doc,Some(block as usize));}},_=>{}
        }}
    }
    /// Fold or unfold change block `block`.
    pub fn fold_block(&mut self,block:usize){if let Some(doc)=self.document.clone(){self.state.fold(&doc,Some(block));}}
    pub fn open_at(&mut self,side:Side,line:u32){self.place=Place::Line(side,line);if !self.loading(){self.state.view.go_to_line(side,line);}}
    pub fn open_at_line(&mut self,line:u32){self.open_at(Side::Right,line);}
    pub fn open_file(&mut self,index:usize){if index==self.state.current&&self.document.is_some(){return;};self.state.press_again.reset();self.load(index,Place::Top,false,false);}
    pub fn reload_set(&mut self){self.state.begin();self.state.markdown_invalidate();self.reading=None;self.diffing=None;self.refining_job=None;self.document=None;self.image=None;self.opening_preserve=true;self.place=Place::Anchor(self.view().anchor_line());let spec=self.spec.clone();let cwd=self.cwd.clone();self.opening=Some(work::submit(Arc::downgrade(&self.state.work_id),move||ChangeSet::load(&spec,&cwd)));}
    pub fn find_selection(&self)->Option<crate::selection::Selection>{let find=self.find()?;let(line,range)=find.hits().get(find.at()?)?;Some(crate::selection::Selection{side:find.side(),anchor:Pos::new(*line,range.start),head:Pos::new(*line,range.end)})}
    pub fn reload(&mut self){crate::diagnostics::breadcrumb("Reload comparison");let anchor=self.state.view.anchor_line();self.load(self.state.current,Place::Anchor(anchor),false,false);}
    pub fn execute(&mut self,command:Command){let loading=self.loading()||self.document.is_none();let large=self.document.as_ref().is_some_and(|d|matches!(d.placeholder_kind(),Some(Placeholder::Large{..})));
        let set=self.set.clone();let files=set.as_ref().map_or(&[][..],|s|s.comparisons.as_slice());
        self.notice=None;match self.state.command(command,files,loading,large){
            Transition::Presentation(mode)=>self.state.markdown_command(crate::markdown::MarkdownCommand::SetMode(mode),self.document.clone()),
            Transition::Repaint=>{if matches!(command,Command::ToggleFold|Command::UnfoldAll)&&let Some(doc)=&self.document{self.state.remember_folds(doc);}},
            Transition::Recompare=>self.load(self.state.current,Place::Anchor(self.state.view.anchor_line()),false,true),
            Transition::Load{index,place,force}=>self.load(index,place,force,false),Transition::Notice(msg)=>self.notice=Some(msg)
        }
    }
    pub fn edit_disabled(&self,command:crate::edit_session::EditMenuCommand)->Option<String>{if command==crate::edit_session::EditMenuCommand::Revert && !matches!(&self.spec,Spec::Git{cached,revs,..} if crate::changeset::right_is_worktree(*cached,revs)){return Some("Revert hunk requires a working-tree comparison".into());}self.state.edit_disabled(&self.writes,self.document(),command)}
    pub fn move_caret(&mut self,selection:crate::selection::Selection,command:crate::selection::CaretCommand,extend:bool)->crate::selection::Selection{let Some(doc)=self.document.clone()else{return selection;};self.state.move_source_caret(&doc,selection,command,extend)}
    pub fn copy_disabled(&self,command:crate::commands::Interaction,selection:Option<crate::selection::Selection>,side:Side)->Option<&'static str>{let Some(set)=self.set.as_ref()else{return Some("Comparison loading");};let Some(doc)=self.document()else{return Some("Comparison loading");};self.state.copy_disabled(command,crate::commands::CopyContext{set,index:self.current(),doc,view:self.view(),selection,side,path_label:self.name.as_deref()})}
    pub fn copy_text(&self,command:crate::commands::Interaction,selection:Option<crate::selection::Selection>,side:Side)->Result<String,&'static str>{let set=self.set.as_ref().ok_or("Comparison loading")?;let doc=self.document().ok_or("Comparison loading")?;self.state.copy_text(command,crate::commands::CopyContext{set,index:self.current(),doc,view:self.view(),selection,side,path_label:self.name.as_deref()})}
    pub fn dirty(&self)->bool{self.writes.dirty(self.current())}
    pub fn dirty_at(&self,index:usize)->bool{self.writes.dirty(index)}
    pub fn has_dirty(&self)->bool{self.writes.has_dirty()}
    pub fn dirty_files(&self)->Vec<usize>{self.writes.dirty_files()}
    pub fn has_file_undo(&self,index:usize)->bool{self.writes.has_undo(index)}
    pub fn undo_file(&mut self,index:usize)->Result<std::path::PathBuf,String>{let set=self.set.clone().ok_or("No comparison loaded")?;self.state.undo_file(&mut self.writes,&set,index).map(|written|written.path).map_err(|e|e.to_string())}
    pub fn file_actions(&self)->&crate::file_actions::FileActions{&self.file_actions}
    pub fn file_has_draft(&self,index:usize)->bool{self.dirty_at(index)||self.set.as_ref().and_then(|s|s.git_root().zip(s.comparisons.get(index))).is_some_and(|(root,c)|self.writes.dirty_path(&root.join(&c.path)))}
    pub fn request_discard(&mut self,index:usize)->Result<(),String>{if self.file_has_draft(index){return Err("Save or Discard unsaved changes before discarding working-tree changes".into());}let set=self.set.clone().ok_or("No comparison loaded")?;self.file_actions.request(&set,index,&self.spec)}
    pub fn confirm_discard(&mut self)->Result<(),String>{if self.file_actions.pending_path().is_some_and(|path|self.writes.dirty_path(path)){return Err("Save or Discard unsaved changes before discarding working-tree changes".into());}self.file_actions.confirm()}
    pub fn cancel_discard(&mut self){self.file_actions.cancel();}
    pub fn undo_discard(&mut self,index:usize)->Result<(),String>{if self.file_has_draft(index){return Err("Save or Discard unsaved changes before undoing Git discard".into());}let set=self.set.clone().ok_or("No comparison loaded")?;self.file_actions.undo(&set,index)}
    pub fn applied(&self)->&[Applied]{self.writes.applied(self.state.current)}
    pub fn edit(&mut self,command:EditCommand){
        let Some(doc)=self.document.clone()else{return;};let(lp,rp)=self.paths(self.state.current);
        match self.state.edit(&mut self.writes,&doc,command,[&lp,&rp]){
            Ok(written)=>{self.document=Some(Arc::new(written.document));self.state.begin();self.state.markdown_invalidate();self.refining_job=None;self.reading=None;self.diffing=None;
                self.place=Place::Anchor(written.anchor);self.state.find=None;self.compare();self.notice=Some(match command{
                    EditCommand::Apply{..}=>"Change applied — see applied history for Undo",EditCommand::Undo=>"Change undone",EditCommand::Redo=>"Change redone",EditCommand::UndoApplied(_)=>"Applied change undone",EditCommand::Save=>"Changes saved",EditCommand::Discard=>"Unsaved changes discarded"
                }.into());},
            Err(EditError::Write(WriteError::ChangedOnDisk))=>{self.notice=Some("The file changed on disk — nothing written; unsaved changes kept. Discard unsaved changes reloads the current file".into());},
            Err(e)=>self.notice=Some(e.to_string())
        }
    }
    pub fn search(&mut self,query:String,side:Side){self.search_at(query,side,Pos::new(self.state.view.anchor_line_on(side)as u32,0));}
    pub fn search_at(&mut self,query:String,side:Side,from:Pos){if let Some(doc)=&self.document
        && let Some((line,_))=self.state.search(doc,query,side,from){self.state.view.go_to_line(side,line+1);}}
    pub fn clear_find(&mut self){self.state.find=None;}
    pub fn find_options(&self)->lenz_content::find::FindOptions{self.state.find_options}
    /// Keep the options another find input (Change Set search) changed, for this window.
    pub fn set_find_options(&mut self,options:lenz_content::find::FindOptions){self.state.find_options=options;}
    /// Flip one find option for this window; an open find re-runs with it and shows its match.
    pub fn toggle_find_option(&mut self,option:lenz_content::find::FindOption){
        let Some(doc)=self.document.clone() else {self.state.find_options=self.state.find_options.toggled(option);return};
        let side=self.state.find.as_ref().map(|f|f.side);
        if let Some((line,_))=self.state.toggle_find_option(&doc,option)&&let Some(side)=side{self.state.view.go_to_line(side,line+1);}
    }
    pub fn find_step(&mut self,forward:bool){if let Some((side,line,_,wrapped))=self.state.search_step(forward){self.notice=wrapped.then(||"Search wrapped".into());self.state.view.go_to_line(side,line+1);}}
}

#[cfg(test)] mod tests {
    use super::*;

    #[test]
    fn pending_refresh_of_shrinking_file_list_preserves_draft_identity_and_write_target() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let dir = tempfile::tempdir_in(root).unwrap();
        for side in ["left", "right"] { std::fs::create_dir(dir.path().join(side)).unwrap(); }
        std::fs::write(dir.path().join("left/a.txt"), "old\n").unwrap();
        std::fs::write(dir.path().join("right/a.txt"), "new\n").unwrap();
        let request = crate::cli::parse(&["--no-index", "left", "right"].map(str::to_owned), dir.path()).unwrap();
        let mut session = Session::open(request, dir.path().into());
        let settle = |session: &mut Session| {
            let deadline = Instant::now() + std::time::Duration::from_secs(15);
            while session.busy() {
                session.poll();
                assert!(Instant::now() < deadline, "refresh did not settle");
                std::thread::yield_now();
            }
        };
        settle(&mut session);
        session.edit(EditCommand::Apply { block: 0, onto: Side::Right });
        settle(&mut session);
        for _ in 0..12 {
            for (side, text) in [("left", "pending old\n"), ("right", "pending new\n")] {
                std::fs::write(dir.path().join(side).join("b.txt"), text.repeat(2000)).unwrap();
            }
            session.reload_set();
            settle(&mut session);
            assert_eq!(session.comparisons().len(), 2);
            session.open_file(1);
            session.reload_set();
            for side in ["left", "right"] { std::fs::remove_file(dir.path().join(side).join("b.txt")).unwrap(); }
            session.reload_set();
            settle(&mut session);
            assert_eq!(session.comparisons().len(), 1);
            assert_eq!(session.comparisons()[session.current()].path, "a.txt");
            assert_eq!(session.document().unwrap().text(Side::Right), "old\n");
            assert!(session.has_dirty());
            assert_eq!(session.applied().last().map(|record| record.active), Some(true));
            assert_eq!(std::fs::read(dir.path().join("right/a.txt")).unwrap(), b"new\n");
        }
        session.edit(EditCommand::Save);
        assert_eq!(std::fs::read(dir.path().join("right/a.txt")).unwrap(), b"old\n");
        assert!(!session.has_dirty());
    }

    #[test]
    fn diagnostic_worker_interruption_child() {
        if std::env::var_os("LENZ_TEST_INTERRUPTED_WORKER").is_none() { return; }
        crate::diagnostics::install("TUI");
        let cwd = std::env::current_dir().unwrap();
        let request = crate::cli::parse(&["--about".into()], &cwd).unwrap();
        let mut session = Session::open(request, cwd);
        session.opening = Some(work::submit(Arc::downgrade(&session.state.work_id), || panic!("ORIGINAL_DOCUMENT_WORKER_CAUSE")));
        let until = Instant::now() + std::time::Duration::from_secs(5);
        while !session.failed() {
            session.poll();
            assert!(Instant::now() < until);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let saved = crate::diagnostics::last_report().unwrap();
        let text = std::fs::read_to_string(saved.path).unwrap();
        assert!(text.contains("ORIGINAL_DOCUMENT_WORKER_CAUSE"), "{text}");
        assert!(text.contains("Original worker report:"));
    }
    #[test]
    fn diagnostic_worker_interruption_retains_the_original_cause() {
        let dir = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "session::tests::diagnostic_worker_interruption_child", "--nocapture"])
            .env("LENZ_TEST_INTERRUPTED_WORKER", "1").env("LENZ_LOG_DIR", dir.path()).output().unwrap();
        assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    }
    #[test] fn click_ignores_a_fold_above_the_first_drawn_terminal_row(){
        let doc=TextDocument::new(&format!("old\n{}old tail\n","same\n".repeat(100)),&format!("new\n{}new tail\n","same\n".repeat(100)),std::path::Path::new("left"),std::path::Path::new("right"));
        let mut view=doc.view(&doc.line_diff(&Options::default()),24.0);view.start_in(ScrollMode::Aligned);view.set_collapsed(true);
        let fold_y=view.frame().left.iter().find(|r|matches!(r.content,Content::Fold{..})).unwrap().y;view.scroll_by(fold_y+0.5,Side::Left);let before=view.frame();
        assert!(before.left.iter().any(|r|r.y<0.0&&matches!(r.content,Content::Fold{..})));
        let state=ComparisonSession::new(view,Options::default());let mut session=Session{state,set:None,spec:Spec::Plain{left:"left".into(),right:"right".into()},opening:None,opening_preserve:false,reading:None,diffing:None,refining_job:None,document:Some(Arc::new(doc)),image:None,name:None,cwd:PathBuf::new(),place:Place::Top,diff_started:None,notice:None,failed:false,writes:EditSession::default(),file_actions:Default::default()};
        session.expand(Side::Left,0);assert_eq!(session.view().frame(),before);
    }
}

#[cfg(test)]
mod markdown_action_tests {
    use super::*;
    use crate::{commands::Interaction, edit_session::EditMenuCommand, markdown::{MarkdownCommand,PresentationMode,RenderSelection}};
    fn settle(session: &mut Session) {
        let until=Instant::now()+std::time::Duration::from_secs(5);
        while session.busy() { session.poll(); assert!(Instant::now()<until); std::thread::yield_now(); }
    }
    #[test]
    fn markdown_session_blocks_pending_ready_writes_and_restores_source_undo() {
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let dir=tempfile::tempdir_in(root).unwrap();
        let (left,right)=(dir.path().join("left.md"),dir.path().join("right.md"));
        std::fs::write(&left,"**old**\n").unwrap();std::fs::write(&right,"**new**\n").unwrap();
        let spec=Spec::Plain { left:left.clone(),right:right.clone() };
        let set=Arc::new(ChangeSet::load(&spec,dir.path()).unwrap());
        let mut session=Session::from_set(set,spec,Options::default(),ScrollMode::Shared);
        settle(&mut session);
        assert!(session.document().unwrap().writable[1].is_ok());
        session.edit(EditCommand::Apply { block:0,onto:Side::Right });settle(&mut session);
        assert_eq!(session.applied().len(),1);let id=session.applied()[0].id;
        let before=std::fs::read(&right).unwrap();let view=(session.view().anchor_line(),session.view().current_change(),session.view().folded_blocks());
        session.execute(Command::SetPresentation(PresentationMode::Rendered));
        assert!(session.markdown_loading());
        for ready in [false,true] {
            if ready { settle(&mut session);assert_eq!(session.markdown_mode(),PresentationMode::Rendered); }
            for cmd in [EditCommand::Undo,EditCommand::Redo,EditCommand::UndoApplied(id),EditCommand::Apply {block:0,onto:Side::Right}] {
                session.edit(cmd);assert_eq!(session.notice(),Some("Switch to Source to edit"));
                assert_eq!(std::fs::read(&right).unwrap(),before);assert!(session.applied()[0].active);assert_eq!(session.applied().len(),1);
            }
            assert_eq!(session.undo_file(0).unwrap_err(),"Switch to Source to edit");
            for cmd in [EditMenuCommand::Undo,EditMenuCommand::Redo,EditMenuCommand::ApplyLeft,EditMenuCommand::ApplyRight] {assert_eq!(session.edit_disabled(cmd).as_deref(),Some("Switch to Source to edit"));}
            assert_eq!(session.edit_disabled(EditMenuCommand::History),None);
        }
        let at=session.markdown_projection().unwrap().pair.sides[1].text.find("old").unwrap();
        let doc=session.document_arc();session.markdown_command(MarkdownCommand::Select(RenderSelection {side:Side::Right,anchor:at,head:at+3}),doc);
        assert_eq!(session.copy_text(Interaction::Copy,None,Side::Right).unwrap(),"old");
        assert_eq!(session.copy_text(Interaction::CopyMarkdownSource,None,Side::Right).unwrap(),"**old**\n");
        session.execute(Command::SetPresentation(PresentationMode::Source));
        // No view mutation or history entry was created by mode switching.
        assert_eq!((session.view().anchor_line(),session.view().current_change(),session.view().folded_blocks()),view);
        session.edit(EditCommand::Undo);settle(&mut session);
        assert_eq!(std::fs::read_to_string(&right).unwrap(),"**new**\n");assert!(!session.applied()[0].active);
        session.edit(EditCommand::Redo);settle(&mut session);
        assert_eq!(std::fs::read(&right).unwrap(),before);
    }
    #[test]
    fn draft_remains_actionable_after_worker_disconnect_and_failed_refresh() {
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");let dir=tempfile::tempdir_in(root).unwrap();let(left,right)=(dir.path().join("left"),dir.path().join("right"));
        std::fs::write(&left,"draft\n").unwrap();std::fs::write(&right,"disk\n").unwrap();let spec=Spec::Plain{left:left.clone(),right:right.clone()};let set=Arc::new(ChangeSet::load(&spec,dir.path()).unwrap());let mut session=Session::from_set(set,spec,Options::default(),ScrollMode::Shared);settle(&mut session);
        session.edit(EditCommand::Apply{block:0,onto:Side::Right});settle(&mut session);
        let(tx,rx)=std::sync::mpsc::channel();drop(tx);session.reading=Some(rx);session.document=None;session.poll();settle(&mut session);
        assert_eq!(session.document().unwrap().text(Side::Right),"draft\n");assert!(session.dirty());assert!(session.edit_disabled(EditMenuCommand::Discard).is_none());
        let(tx,rx)=std::sync::mpsc::channel();tx.send(Some(Err("fixture read failure".into()))).unwrap();session.opening_preserve=true;session.opening=Some(rx);session.document=None;session.poll();settle(&mut session);
        assert_eq!(session.document().unwrap().text(Side::Right),"draft\n");assert!(session.edit_disabled(EditMenuCommand::Save).is_none());assert_eq!(std::fs::read_to_string(&right).unwrap(),"disk\n");session.edit(EditCommand::Save);settle(&mut session);assert_eq!(std::fs::read_to_string(&right).unwrap(),"draft\n");assert!(!session.has_dirty());
    }
    #[test]
    fn rendered_mode_can_save_or_discard_an_existing_source_draft() {
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");let dir=tempfile::tempdir_in(root).unwrap();let(left,right)=(dir.path().join("left.md"),dir.path().join("right.md"));
        std::fs::write(&left,"**draft**\n").unwrap();std::fs::write(&right,"**disk**\n").unwrap();let spec=Spec::Plain{left:left.clone(),right:right.clone()};let set=Arc::new(ChangeSet::load(&spec,dir.path()).unwrap());let mut session=Session::from_set(set,spec,Options::default(),ScrollMode::Shared);settle(&mut session);
        session.edit(EditCommand::Apply{block:0,onto:Side::Right});settle(&mut session);session.execute(Command::SetPresentation(PresentationMode::Rendered));settle(&mut session);
        assert!(session.edit_disabled(EditMenuCommand::Save).is_none());session.edit(EditCommand::Save);settle(&mut session);assert_eq!(std::fs::read_to_string(&right).unwrap(),"**draft**\n");
        session.execute(Command::SetPresentation(PresentationMode::Source));session.edit(EditCommand::Undo);settle(&mut session);session.execute(Command::SetPresentation(PresentationMode::Rendered));settle(&mut session);
        assert!(session.edit_disabled(EditMenuCommand::Discard).is_none());session.edit(EditCommand::Discard);settle(&mut session);assert_eq!(session.document().unwrap().text(Side::Right),"**draft**\n");assert!(!session.has_dirty());assert_eq!(std::fs::read_to_string(&right).unwrap(),"**draft**\n");
    }

}

#[cfg(test)]
mod find_option_tests {
    use super::*;
    use lenz_content::find::FindOption;
    #[test]
    fn toggling_a_find_option_refinds_with_it() {
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let dir=tempfile::tempdir_in(root).unwrap();
        std::fs::write(dir.path().join("left.txt"),"Foo foo\n").unwrap();
        std::fs::write(dir.path().join("right.txt"),"x\n").unwrap();
        let request=crate::cli::parse(&["--no-index","left.txt","right.txt"].map(str::to_owned),dir.path()).unwrap();
        let mut s=Session::open(request,dir.path().into());
        let until=Instant::now()+std::time::Duration::from_secs(15);
        while s.busy() { s.poll(); assert!(Instant::now()<until); std::thread::yield_now(); }
        s.search("foo".into(),Side::Left);
        assert_eq!(s.find().unwrap().hits().len(),2);
        s.toggle_find_option(FindOption::Case);
        assert_eq!(s.find().unwrap().hits(),[(0,4..7)]);
        assert!(s.find_options().case);
    }
}
