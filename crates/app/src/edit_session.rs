//! Shared pending two-way apply/undo/save workflow and retained applied-change history.
use std::{collections::{HashMap, HashSet}, ops::Range, path::{Path, PathBuf}, sync::Arc};
use crate::{document::TextDocument, moves, view::{Side, View}, write::{self, WriteError}};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditCommand { Apply { block: usize, onto: Side }, Undo, Redo, UndoApplied(u64), Save, Discard }

#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum EditMenuCommand { ApplyLeft, ApplyRight, Undo, Redo, History, Save, Discard, Revert }
impl EditMenuCommand {
    pub const ALL:&'static [Self]=&[Self::ApplyLeft,Self::ApplyRight,Self::Undo,Self::Redo,Self::History,Self::Save,Self::Discard,Self::Revert];
    pub fn label(self)->&'static str {match self{Self::ApplyLeft=>"Apply current change to left",Self::ApplyRight=>"Apply current change to right",Self::Undo=>"Undo applied change",Self::Redo=>"Redo applied change",Self::History=>"Show applied changes and individual Undo",Self::Save=>"Save changes",Self::Discard=>"Discard unsaved changes",Self::Revert=>"Revert current hunk"}}
    pub fn command(self,block:Option<usize>)->Option<EditCommand>{match self{Self::ApplyLeft|Self::ApplyRight|Self::Revert=>block.map(|block|EditCommand::Apply{block,onto:if self==Self::ApplyLeft{Side::Left}else{Side::Right}}),Self::Undo=>Some(EditCommand::Undo),Self::Redo=>Some(EditCommand::Redo),Self::Save=>Some(EditCommand::Save),Self::Discard=>Some(EditCommand::Discard),Self::History=>None}}
}
#[derive(Debug)]
pub enum EditError { Unavailable(String), Write(WriteError) }
impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self { Self::Unavailable(s) => f.write_str(s), Self::Write(e) => e.fmt(f) }
    }
}

pub struct Written { pub document: TextDocument, pub path: PathBuf, pub anchor: f32 }

#[derive(Debug, Clone)]
pub struct Applied {
    pub id: u64,
    pub side: Side,
    pub line: u32,
    pub old_lines: usize,
    pub new_lines: usize,
    pub active: bool,
    range: Range<usize>,
    before: String,
    after_hash: u64,
    valid: bool,
}
impl Applied {
    pub fn label(&self) -> String {
        format!("{} {} {} · line {} · {} → {} lines", if self.active { "✓ Applied" } else { "↶ Undone" }, self.id,
            if self.side == Side::Right { "→ right" } else { "← left" }, self.line + 1, self.old_lines, self.new_lines)
    }
    pub fn undo_reason(&self) -> Option<&'static str> {
        if !self.active { Some("already undone") } else if !self.valid { Some("overlaps a later change; use normal undo") } else { None }
    }
}
#[derive(Default)]
pub struct EditSession {
    histories: HashMap<usize, History>,
    pub(crate) edited: HashSet<PathBuf>,
    next_id: u64,
    /// The last input was a quit refused for unsaved drafts: quitting again closes without them.
    quit_asked: bool,
}

/// What a quit with unsaved drafts says, in both adapters.
pub const QUIT_QUESTION: &str = "Unsaved changes — Save keeps them; quit again to close without saving";
#[derive(Default)]
struct History { edits: moves::Edits, applied: Vec<Applied>, undo: Vec<(u64, bool)>, redo: Vec<(u64, bool)>, pending: Option<Pending>, saved_steps: Option<Checkpoint> }
struct Pending { current: [Arc<str>; 2], saved: [Arc<str>; 2], snapshots: [Result<write::Snapshot, &'static str>; 2], targets:[Option<PathBuf>;2], encodings:[Option<String>;2], notes:Vec<String> }
#[derive(Clone)]
struct Checkpoint { edits: moves::Edits, applied: Vec<Applied>, undo: Vec<(u64,bool)>, redo: Vec<(u64,bool)> }
impl Checkpoint {
    fn capture(h: &History) -> Self { Self { edits:h.edits.clone(),applied:h.applied.clone(),undo:h.undo.clone(),redo:h.redo.clone() } }
    fn restore(self,h:&mut History) { h.edits=self.edits;h.applied=self.applied;h.undo=self.undo;h.redo=self.redo; }
}
impl Pending {
    fn new(doc:&TextDocument)->Self { let current=[doc.text_arc(Side::Left),doc.text_arc(Side::Right)];Self {saved:current.clone(),current,snapshots:doc.writable.clone(),targets:doc.writable.each_ref().map(|s|s.as_ref().ok().and_then(|s|std::fs::canonicalize(&s.path).ok())),encodings:doc.encodings.clone(),notes:doc.notes.clone()} }
    fn dirty(&self,side:Side)->bool {self.current[side as usize]!=self.saved[side as usize]}
}
impl EditSession {
    pub fn dirty(&self,index:usize)->bool {self.histories.get(&index).and_then(|h|h.pending.as_ref()).is_some_and(|p|p.dirty(Side::Left)||p.dirty(Side::Right))}
    pub fn has_dirty(&self)->bool {self.histories.keys().any(|&i|self.dirty(i))}
    /// A quit request: true when nothing is unsaved, or this repeats a refused quit (as the Merge View's second
    /// Esc). The first asks with [`QUIT_QUESTION`].
    pub fn quit(&mut self)->bool {let again=std::mem::take(&mut self.quit_asked);if !self.has_dirty()||again{return true;}self.quit_asked=true;false}
    /// Any other input: a following quit asks again.
    pub fn keep_open(&mut self) {self.quit_asked=false;}
    pub fn dirty_files(&self)->Vec<usize> {let mut files=self.histories.keys().copied().filter(|&i|self.dirty(i)).collect::<Vec<_>>();files.sort_unstable();files}
    /// Rehydrate pending text after switching files, refresh or cache eviction.
    pub fn overlay(&self,index:usize,doc:&TextDocument,paths:[&Path;2])->Option<TextDocument> {
        if !self.dirty(index){return None;}
        if doc.placeholder_kind().is_some(){return self.recover(index,paths);}
        let p=self.histories.get(&index)?.pending.as_ref()?;
        let mut over=None;
        for side in [Side::Left,Side::Right] {if p.dirty(side) && let Ok(snapshot)=&p.snapshots[side as usize] {
            let source=over.as_ref().unwrap_or(doc);over=Some(source.edited(side,&p.current[side as usize],snapshot.clone(),paths[0],paths[1]));
        }}
        over
    }

    /// Recover a retained draft even when its disk source can no longer be loaded.
    pub fn recover(&self,index:usize,paths:[&Path;2])->Option<TextDocument> {
        if !self.dirty(index){return None;}
        let p=self.histories.get(&index)?.pending.as_ref()?;
        let mut doc=TextDocument::new(&p.current[0],&p.current[1],paths[0],paths[1]);
        doc.writable=p.snapshots.clone();doc.encodings=p.encodings.clone();doc.notes=p.notes.clone();Some(doc)
    }
    pub fn dirty_path(&self,path:&Path)->bool {
        self.histories.values().any(|h|h.pending.as_ref().is_some_and(|p|[Side::Left,Side::Right].into_iter().any(|side|p.dirty(side)&&p.snapshots[side as usize].as_ref().is_ok_and(|s|same_file(&s.path,path)))))
    }

    pub fn has_applied(&self)->bool{self.histories.values().any(|history|!history.applied.is_empty())}
    pub fn retain_applied_files(&self,old:&crate::changeset::ChangeSet,new:&mut crate::changeset::ChangeSet){
        if !self.has_applied() && !self.has_dirty(){return;}
        let retained={let identities=(0..new.comparisons.len()).filter_map(|i|crate::changeset::comparison_identity(new,i)).collect::<HashSet<_>>();
            let mut retained=self.histories.iter().filter(|(index,history)|(!history.applied.is_empty()||self.dirty(**index))&&crate::changeset::comparison_identity(old,**index).is_some_and(|identity|!identities.contains(&identity))).map(|(&i,_)|i).collect::<Vec<_>>();retained.sort_unstable();retained};
        for index in retained{new.retain_comparison(old,index);}
    }
    pub fn remap_files(&mut self,old:&crate::changeset::ChangeSet,new:&crate::changeset::ChangeSet){
        if self.histories.is_empty(){return;}
        let indices=(0..new.comparisons.len()).filter_map(|i|crate::changeset::comparison_identity(new,i).map(|identity|(identity,i))).collect::<HashMap<_,_>>();
        self.histories=std::mem::take(&mut self.histories).into_iter().filter_map(|(i,history)|crate::changeset::comparison_identity(old,i).and_then(|identity|indices.get(&identity).copied()).map(|index|(index,history))).collect();
    }

    pub fn applied(&self, index: usize) -> &[Applied] { self.histories.get(&index).map_or(&[], |h| &h.applied) }
    pub fn has_undo(&self,index:usize)->bool{self.histories.get(&index).is_some_and(|h|!h.undo.is_empty())}
    pub fn undo_file(&mut self,set:&crate::changeset::ChangeSet,index:usize)->Result<Written,EditError>{
        let c=set.comparisons.get(index).ok_or_else(||EditError::Unavailable("No file selected".into()))?;
        let paths=[std::path::PathBuf::from(&c.old_path),std::path::PathBuf::from(&c.path)];
        let doc=crate::document::TextDocument::load(set,index,&paths[0],&paths[1],false).map_err(EditError::Unavailable)?;
        let doc=self.overlay(index,&doc,[&paths[0],&paths[1]]).unwrap_or(doc);let view=doc.view(&[],1.0);self.execute(index,&doc,&view,EditCommand::Undo,[&paths[0],&paths[1]])
    }
    pub fn clear(&mut self, index: usize) { self.histories.remove(&index); }
    pub fn disabled(&self,index:usize,doc:Option<&TextDocument>,view:&View,command:EditMenuCommand)->Option<String>{
        let Some(doc)=doc else{return Some("Comparison loading".into());};
        match command {
            EditMenuCommand::ApplyLeft|EditMenuCommand::ApplyRight|EditMenuCommand::Revert=>{
                if view.current_change().is_none(){return Some("No change selected".into());}
                let side=if command==EditMenuCommand::ApplyLeft{Side::Left}else{Side::Right};doc.writable[side as usize].as_ref().err().map(|why|format!("Side is {why}"))
            },
            EditMenuCommand::Undo|EditMenuCommand::Redo=>self.histories.get(&index).and_then(|h|h.edits.next(command==EditMenuCommand::Redo,[doc.text(Side::Left),doc.text(Side::Right)])).is_none().then(||if command==EditMenuCommand::Undo{"Nothing to undo"}else{"Nothing to redo"}.into()),
            EditMenuCommand::History=>self.applied(index).is_empty().then(||"No applied changes".into()),
            EditMenuCommand::Save|EditMenuCommand::Discard=>(!self.dirty(index)).then(||"No unsaved changes".into())
        }
    }
    pub fn execute(&mut self, index: usize, doc: &TextDocument, view: &View, command: EditCommand, paths: [&Path; 2]) -> Result<Written, EditError> {
    crate::diagnostics::breadcrumb("Edit comparison draft");
        if matches!(command,EditCommand::Save|EditCommand::Discard) { return self.finish(index,doc,view,command,paths); }
        let mut anchor = view.anchor_line();
        let (side, text, event, line_counts) = match command {
            EditCommand::Apply { block, onto } => {
                let (left,right) = view.block_ranges().get(block).cloned().ok_or_else(|| EditError::Unavailable("No change selected".into()))?;
                let (into,from) = if onto == Side::Left { (left,right) } else { (right,left) };
                if onto == Side::Left && anchor >= into.end as f32 { anchor += from.len() as f32 - into.len() as f32; }
                (onto, moves::moved_text(doc.text(onto),doc.text(if onto == Side::Left { Side::Right } else { Side::Left }),into.clone(),from.clone()),
                    None, Some((into.start,into.len(),from.len())))
            }
            EditCommand::Undo | EditCommand::Redo => {
                let redo = command == EditCommand::Redo;
                let history = self.histories.get(&index).ok_or_else(|| EditError::Unavailable(if redo { "Nothing to redo" } else { "Nothing to undo" }.into()))?;
                let (side,text) = history.edits.next(redo,[doc.text(Side::Left),doc.text(Side::Right)])
                    .ok_or_else(|| EditError::Unavailable(if redo { "Nothing to redo" } else { "Nothing to undo" }.into()))?;
                let event = if redo { history.redo.last() } else { history.undo.last() }.copied();
                (side,text,event,None)
            }
            EditCommand::UndoApplied(id) => {
                let record = self.applied(index).iter().find(|a| a.id == id).ok_or_else(|| EditError::Unavailable("Applied change no longer available".into()))?;
                if let Some(reason) = record.undo_reason() { return Err(EditError::Unavailable(reason.into())); }
                let before = doc.text(record.side);
                let shown = before.get(record.range.clone()).ok_or_else(|| EditError::Unavailable("Applied change moved; use normal undo".into()))?;
                if moves::fingerprint(shown) != record.after_hash { return Err(EditError::Unavailable("Applied text changed; use normal undo".into())); }
                let mut text = String::with_capacity(before.len() - record.range.len() + record.before.len());
                text.push_str(&before[..record.range.start]); text.push_str(&record.before); text.push_str(&before[record.range.end..]);
                (record.side,text,Some((id,false)),None)
            }
            EditCommand::Save|EditCommand::Discard=>unreachable!("handled above"),
        };
        let snapshot = doc.writable[side as usize].as_ref().map_err(|why| EditError::Unavailable(format!("Can't change the {} Side: it is {why}",if side == Side::Left { "left" } else { "right" })))?;
        if self.histories.iter().any(|(&other,h)|other!=index && h.pending.as_ref().is_some_and(|p|[Side::Left,Side::Right].into_iter().any(|side|p.dirty(side)&&p.snapshots[side as usize].as_ref().is_ok_and(|s|same_file(&s.path,&snapshot.path))))) {
            return Err(EditError::Unavailable("This file has unsaved changes in another comparison; save or discard that draft first".into()));
        }
        // Editing only changes the draft. The original disk snapshot remains the Save guard.
        let snapshot = snapshot.clone();
        let path = snapshot.path.clone();
        let document = doc.edited(side,&text,snapshot,paths[0],paths[1]);
        let (before,after) = (doc.text(side),document.text(side));
        let history = self.histories.entry(index).or_default();
        let pending=history.pending.get_or_insert_with(||Pending::new(doc));
        // A previously clean side starts from the freshly loaded document. Already
        // dirty sides keep their original snapshots, even after external reloads.
        for clean in [Side::Left,Side::Right] {if !pending.dirty(clean) {
            let i=clean as usize;pending.saved[i]=doc.text_arc(clean);pending.current[i]=doc.text_arc(clean);pending.snapshots[i]=doc.writable[i].clone();
            pending.targets[i]=doc.writable[i].as_ref().ok().and_then(|s|std::fs::canonicalize(&s.path).ok());
        }}
        pending.current[side as usize]=document.text_arc(side);
        // A compact inverse patch also describes how existing retained ranges shift.
        let mut delta = moves::Edits::default();
        let (range,replaced) = delta.record_detail(side,before,after);
        let old = range.start..range.start + replaced.len();
        let shift = range.len() as isize - old.len() as isize;
        for record in history.applied.iter_mut().filter(|r| r.active && r.side == side && Some(r.id) != event.map(|e|e.0)) {
            if record.range.end <= old.start { continue; }
            if record.range.start >= old.end && !(old.is_empty() && record.range.is_empty() && record.range.start == old.start) {
                record.range = record.range.start.saturating_add_signed(shift)..record.range.end.saturating_add_signed(shift);
            } else { record.valid = false; }
        }
        match command {
            EditCommand::Apply { .. } => {
                self.next_id += 1;
                let (line,old_lines,new_lines) = line_counts.expect("apply has block ranges");
                history.applied.push(Applied { id:self.next_id,side,line,old_lines,new_lines,active:true,
                    after_hash:moves::fingerprint(&after[range.clone()]),range,before:replaced,valid:true });
                history.edits.record(side,before,after); history.undo.push((self.next_id,true)); history.redo.clear();
            }
            EditCommand::UndoApplied(id) => {
                history.edits.record(side,before,after); history.undo.push((id,false)); history.redo.clear();
                if let Some(record) = history.applied.iter_mut().find(|r|r.id == id) { record.active=false; record.range=range; }
            }
            EditCommand::Undo | EditCommand::Redo => {
                let redo = command == EditCommand::Redo;
                history.edits.stepped(redo,before,after);
                let event = if redo { history.redo.pop() } else { history.undo.pop() };
                if let Some((id,active)) = event {
                    if let Some(record) = history.applied.iter_mut().find(|r|r.id == id) {
                        record.active = if redo { active } else { !active };
                        record.range=range; record.valid=true; record.after_hash=moves::fingerprint(&after[record.range.clone()]);
                    }
                    if redo { history.undo.push((id,active)); } else { history.redo.push((id,active)); }
                }
            }
            EditCommand::Save|EditCommand::Discard=>unreachable!("handled above"),
        }
        if !history.pending.as_ref().is_some_and(|p|p.dirty(Side::Left)||p.dirty(Side::Right)){history.pending=None;}
        Ok(Written {document,path,anchor})
    }
    fn finish(&mut self,index:usize,doc:&TextDocument,view:&View,command:EditCommand,paths:[&Path;2])->Result<Written,EditError> {
        if !self.dirty(index){return Err(EditError::Unavailable("No unsaved changes".into()));}
        let h=self.histories.get_mut(&index).expect("dirty history");
        let pending=h.pending.as_mut().expect("dirty draft");
        let sides=[Side::Left,Side::Right].into_iter().filter(|&side|pending.dirty(side)).collect::<Vec<_>>();
        if command==EditCommand::Save {
            if sides.len()==2 && pending.snapshots[0].as_ref().ok().zip(pending.snapshots[1].as_ref().ok()).is_some_and(|(a,b)|same_file(&a.path,&b.path)) {
                return Err(EditError::Unavailable("Both sides refer to the same file; discard one draft before saving".into()));
            }
            for &side in &sides {
                let snapshot=pending.snapshots[side as usize].as_ref().map_err(|why|EditError::Unavailable((*why).into()))?;
                if std::fs::symlink_metadata(&snapshot.path).is_ok_and(|m|m.is_symlink()) && let Err(reason)=moves::editable(&crate::changeset::Content::File(snapshot.path.clone()),&std::env::temp_dir()) {
                    return Err(EditError::Unavailable(format!("Can't save this side: it is {reason}; unsaved changes kept")));
                }
                if let Ok(target)=std::fs::canonicalize(&snapshot.path) && pending.targets[side as usize].as_ref().is_some_and(|old|*old!=target) {
                    return Err(EditError::Write(WriteError::ChangedOnDisk));
                }
                write::validate_text(snapshot,&pending.current[side as usize]).map_err(EditError::Write)?;
            }
        }
        // Read every side before changing draft state so a failed reload keeps the whole draft.
        let mut reloaded=HashMap::new();
        if command==EditCommand::Discard {for &side in &sides {
            let snapshot=pending.snapshots[side as usize].as_ref().map_err(|why|EditError::Unavailable((*why).into()))?;
            let (snapshot,decoded)=snapshot.reread_as_read().map_err(EditError::Write)?;
            let text:Arc<str>=decoded.map_or_else(||Arc::from(""),|d|Arc::from(d.text));
            reloaded.insert(side as usize,(snapshot,text));
        }}
        let mut document=None;let mut path=PathBuf::new();let mut externally_changed=false;
        for side in sides {
            let i=side as usize;let snapshot=pending.snapshots[i].as_ref().map_err(|why|EditError::Unavailable((*why).into()))?;
            let (snapshot,text)=if command==EditCommand::Save {
                let snapshot=match write::write_text(snapshot,&pending.current[i]) {
                    Ok(snapshot)=>snapshot,
                    Err(error) if document.is_some()=>{h.saved_steps=None;return Err(EditError::Unavailable(format!("An earlier side was saved, but this side was not: {error}. Remaining unsaved changes are kept")));},
                    Err(error)=>return Err(EditError::Write(error)),
                };
                (snapshot,pending.current[i].clone())
            }else {
                let (snapshot,text)=reloaded.remove(&i).expect("discard preloaded every side");
                externally_changed|=text!=pending.saved[i];(snapshot,text)
            };
            path=snapshot.path.clone();pending.snapshots[i]=Ok(snapshot.clone());pending.saved[i]=text.clone();pending.current[i]=text.clone();
            let source=document.as_ref().unwrap_or(doc);document=Some(source.edited(side,&text,snapshot,paths[0],paths[1]));
            if command==EditCommand::Save {self.edited.insert(path.clone());}
        }
        if command==EditCommand::Save {h.saved_steps=Some(Checkpoint::capture(h));}
        else if !externally_changed && let Some(saved)=h.saved_steps.clone() {saved.restore(h);}
        else {h.edits=moves::Edits::default();h.applied.clear();h.undo.clear();h.redo.clear();h.saved_steps=None;}
        // Only dirty files retain full draft text; saved history keeps compact inverse patches.
        h.pending=None;
        let mut document=document.expect("dirty sides");
        if command==EditCommand::Discard {for writable in &mut document.writable {
            if let Ok(snapshot)=writable && std::fs::symlink_metadata(&snapshot.path).is_ok_and(|m|m.is_symlink()) && let Err(reason)=moves::editable(&crate::changeset::Content::File(snapshot.path.clone()),&std::env::temp_dir()) {*writable=Err(reason);}
        }}
        Ok(Written {document,path,anchor:view.anchor_line()})
    }

}

fn same_file(a:&Path,b:&Path)->bool {
    if a==b{return true;}
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(a),std::fs::metadata(b)) {(Ok(a),Ok(b))=>a.dev()==b.dev()&&a.ino()==b.ino(),_=>false}
}

#[cfg(test)]
mod tests {
    use super::*;
    use lenz_engine::Options;

    fn doc(left: &str, right: &str, l: &Path, r: &Path) -> TextDocument {
        std::fs::write(l, left).unwrap(); std::fs::write(r, right).unwrap();
        let mut doc = TextDocument::new(left, right, l, r);
        doc.writable = [Ok(write::Snapshot::read(l, None).unwrap().0), Ok(write::Snapshot::read(r, None).unwrap().0)];
        doc
    }
    fn save(history:&mut EditSession,index:usize,doc:TextDocument,l:&Path,r:&Path)->TextDocument {
        if !history.dirty(index){return doc;}
        let view=doc.view(&[],20.0);history.execute(index,&doc,&view,EditCommand::Save,[l,r]).unwrap().document
    }
    #[test]
    fn refresh_remaps_history_and_keeps_fully_applied_files_navigable(){
        let dir=tempfile::tempdir().unwrap();let (left,right)=(dir.path().join("left"),dir.path().join("right"));std::fs::create_dir_all(&left).unwrap();std::fs::create_dir_all(&right).unwrap();
        let (l,r)=(left.join("z.txt"),right.join("z.txt"));let mut document=doc("old\n","new\n",&l,&r);let spec=crate::changeset::Spec::Plain{left:left.clone(),right:right.clone()};let old=crate::changeset::ChangeSet::load(&spec,dir.path()).unwrap();let mut history=EditSession::default();let view=document.view(&document.line_diff(&Options::default()),20.0);document=history.execute(0,&document,&view,EditCommand::Apply{block:0,onto:Side::Right},[&l,&r]).unwrap().document;
        document=save(&mut history,0,document,&l,&r);std::fs::write(left.join("a.txt"),"A\n").unwrap();std::fs::write(right.join("a.txt"),"B\n").unwrap();let mut new=crate::changeset::ChangeSet::load(&spec,dir.path()).unwrap();assert!(new.comparisons.iter().all(|c|c.path!="z.txt"));history.retain_applied_files(&old,&mut new);history.remap_files(&old,&new);let index=new.comparisons.iter().position(|c|c.path=="z.txt").unwrap();assert_eq!(index,1);assert!(history.applied(0).is_empty());assert_eq!(history.applied(index).len(),1);let id=history.applied(index)[0].id;document=history.execute(index,&document,&document.view(&[],20.0),EditCommand::UndoApplied(id),[&l,&r]).unwrap().document;let _=save(&mut history,index,document,&l,&r);assert_eq!(std::fs::read_to_string(r).unwrap(),"new\n");
    }
    #[test]
    fn selective_undo_rebases_across_insertions_and_global_history_replays(){
        let dir=tempfile::tempdir().unwrap();let(l,r)=(dir.path().join("l"),dir.path().join("r"));
        let left="a\ninsert one\ninsert two\nb\nc\nd\ne\nf\ng\nh\ni\n";let right="a\nb\nc\nd\ne\nf\nG\nh\ni\n";
        let mut doc=doc(left,right,&l,&r);let mut history=EditSession::default();
        for block in [1,0]{let view=doc.view(&doc.line_diff(&Options::default()),20.0);doc=history.execute(0,&doc,&view,EditCommand::Apply{block,onto:Side::Right},[&l,&r]).unwrap().document;}
        doc=save(&mut history,0,doc,&l,&r);assert_eq!(std::fs::read_to_string(&r).unwrap(),left);let id=history.applied(0)[0].id;
        let view=doc.view(&[],20.0);doc=history.execute(0,&doc,&view,EditCommand::UndoApplied(id),[&l,&r]).unwrap().document;
        doc=save(&mut history,0,doc,&l,&r);assert_eq!(std::fs::read_to_string(&r).unwrap(),left.replace("\ng\n","\nG\n"));
        for command in [EditCommand::Undo,EditCommand::Undo,EditCommand::Undo]{let view=doc.view(&[],20.0);doc=history.execute(0,&doc,&view,command,[&l,&r]).unwrap().document;}
        doc=save(&mut history,0,doc,&l,&r);assert_eq!(std::fs::read_to_string(&r).unwrap(),right);
        for command in [EditCommand::Redo,EditCommand::Redo,EditCommand::Redo]{let view=doc.view(&[],20.0);doc=history.execute(0,&doc,&view,command,[&l,&r]).unwrap().document;}
        let _=save(&mut history,0,doc,&l,&r);assert_eq!(std::fs::read_to_string(&r).unwrap(),left.replace("\ng\n","\nG\n"));assert_eq!(history.applied(0).len(),2);
    }
    #[test]
    fn shared_apply_and_undo_keep_bom_crlf_and_compact_inverse_records(){
        let dir=tempfile::tempdir().unwrap();let(l,r)=(dir.path().join("l"),dir.path().join("r"));
        let original=b"\xEF\xBB\xBFold\r\nsame\r\n";std::fs::write(&l,"new\nsame\n").unwrap();std::fs::write(&r,original).unwrap();
        let(snapshot,decoded)=write::Snapshot::read(&r,None).unwrap();let mut doc=TextDocument::new("new\nsame\n",&decoded.text,&l,&r);doc.writable[1]=Ok(snapshot);
        let mut history=EditSession::default();let view=doc.view(&doc.line_diff(&Options::default()),20.0);doc=history.execute(0,&doc,&view,EditCommand::Apply{block:0,onto:Side::Right},[&l,&r]).unwrap().document;
        doc=save(&mut history,0,doc,&l,&r);assert_eq!(std::fs::read(&r).unwrap(),b"\xEF\xBB\xBFnew\r\nsame\r\n");assert!(history.applied(0)[0].before.len()<20);
        let view=doc.view(&[],20.0);doc=history.execute(0,&doc,&view,EditCommand::Undo,[&l,&r]).unwrap().document;let _=save(&mut history,0,doc,&l,&r);assert_eq!(std::fs::read(&r).unwrap(),original);
    }
    #[test]
    fn overlapping_later_apply_disables_selective_undo_without_writing(){
        let dir=tempfile::tempdir().unwrap();let(l,r)=(dir.path().join("l"),dir.path().join("r"));let mut doc=doc("first\n","old\n",&l,&r);let mut history=EditSession::default();
        let view=doc.view(&doc.line_diff(&Options::default()),20.0);doc=history.execute(0,&doc,&view,EditCommand::Apply{block:0,onto:Side::Right},[&l,&r]).unwrap().document;let id=history.applied(0)[0].id;
        std::fs::write(&l,"second\n").unwrap();let snapshot=write::Snapshot::read(&l,None).unwrap().0;doc=doc.edited(Side::Left,"second\n",snapshot,&l,&r);
        let view=doc.view(&doc.line_diff(&Options::default()),20.0);doc=history.execute(0,&doc,&view,EditCommand::Apply{block:0,onto:Side::Right},[&l,&r]).unwrap().document;
        doc=save(&mut history,0,doc,&l,&r);assert!(history.applied(0)[0].undo_reason().unwrap().contains("overlaps"));let view=doc.view(&[],20.0);assert!(matches!(history.execute(0,&doc,&view,EditCommand::UndoApplied(id),[&l,&r]),Err(EditError::Unavailable(_))));assert_eq!(std::fs::read_to_string(&r).unwrap(),"second\n");
    }
    #[test]
    fn independent_applied_changes_remain_visible_and_can_be_undone_out_of_order() {
        let dir = tempfile::tempdir().unwrap(); let (l,r)=(dir.path().join("l"),dir.path().join("r"));
        let left="a\nb\nc\nd\ne\nf\ng\n"; let right="a\nB\nc\nd\ne\nF\ng\n";
        let mut doc=doc(left,right,&l,&r); let mut history=EditSession::default();
        for block in [0,0] {
            let view=doc.view(&doc.line_diff(&Options::default()),20.0);
            doc=history.execute(0,&doc,&view,EditCommand::Apply {block,onto:Side::Right},[&l,&r]).unwrap().document;
        }
        doc=save(&mut history,0,doc,&l,&r);assert_eq!(std::fs::read_to_string(&r).unwrap(),left);
        let items=history.applied(0); assert_eq!(items.len(),2); assert!(items.iter().all(|a|a.active));
        let first=items[0].id;
        let view=doc.view(&doc.line_diff(&Options::default()),20.0);
        doc=history.execute(0,&doc,&view,EditCommand::UndoApplied(first),[&l,&r]).unwrap().document;
        doc=save(&mut history,0,doc,&l,&r);assert_eq!(std::fs::read_to_string(&r).unwrap(),"a\nB\nc\nd\ne\nf\ng\n");
        assert!(!history.applied(0)[0].active); assert!(history.applied(0)[1].active);
        let view=doc.view(&doc.line_diff(&Options::default()),20.0);
        doc=history.execute(0,&doc,&view,EditCommand::Undo,[&l,&r]).unwrap().document;let _=save(&mut history,0,doc,&l,&r);
        assert_eq!(std::fs::read_to_string(&r).unwrap(),left,"undo reverses selective undo too");
        assert!(history.applied(0)[0].active);
    }
    #[test]
    fn external_edits_and_readonly_versions_are_never_overwritten() {
        let dir=tempfile::tempdir().unwrap();let(l,r)=(dir.path().join("l"),dir.path().join("r"));
        let mut doc=doc("old\n","new\n",&l,&r);let view=doc.view(&doc.line_diff(&Options::default()),20.0);
        let mut history=EditSession::default();
        doc.writable[0]=Err("a git revision");
        assert!(matches!(history.execute(0,&doc,&view,EditCommand::Apply{block:0,onto:Side::Left},[&l,&r]),Err(EditError::Unavailable(_))));
        std::fs::write(&r,"external\n").unwrap();
        doc=history.execute(0,&doc,&view,EditCommand::Apply{block:0,onto:Side::Right},[&l,&r]).unwrap().document;
        let view=doc.view(&[],20.0);
        assert!(matches!(history.execute(0,&doc,&view,EditCommand::Save,[&l,&r]),Err(EditError::Write(WriteError::ChangedOnDisk))));
        assert_eq!(std::fs::read_to_string(&r).unwrap(),"external\n");assert!(history.dirty(0));assert_eq!(doc.text(Side::Right),"old\n");
        let fresh=TextDocument::load(&crate::changeset::ChangeSet::load(&crate::changeset::Spec::Plain{left:l.clone(),right:r.clone()},dir.path()).unwrap(),0,&l,&r,false).unwrap();
        let overlay=history.overlay(0,&fresh,[&l,&r]).unwrap();assert_eq!(overlay.text(Side::Right),"old\n");
        doc=history.execute(0,&overlay,&view,EditCommand::Discard,[&l,&r]).unwrap().document;
        assert_eq!(doc.text(Side::Right),"external\n");assert!(!history.has_dirty());assert!(history.applied(0).is_empty());
    }
    #[test]
    fn draft_undo_redo_save_and_discard_restore_saved_history() {
        let dir=tempfile::tempdir().unwrap();let(l,r)=(dir.path().join("l"),dir.path().join("r"));
        let mut doc=doc("old\n","new\n",&l,&r);let mut history=EditSession::default();
        let step=|h:&mut EditSession,d:TextDocument,c|{let v=d.view(&d.line_diff(&Options::default()),20.0);h.execute(0,&d,&v,c,[&l,&r]).unwrap().document};
        doc=step(&mut history,doc,EditCommand::Apply{block:0,onto:Side::Right});
        assert_eq!(doc.text(Side::Right),"old\n");assert_eq!(std::fs::read_to_string(&r).unwrap(),"new\n");assert!(history.has_dirty());
        doc=step(&mut history,doc,EditCommand::Undo);assert!(!history.has_dirty());assert!(history.histories[&0].pending.is_none());
        doc=step(&mut history,doc,EditCommand::Redo);assert!(history.dirty(0));
        doc=step(&mut history,doc,EditCommand::Save);assert_eq!(std::fs::read_to_string(&r).unwrap(),"old\n");assert!(!history.has_dirty());assert!(history.histories[&0].pending.is_none());
        let saved_id=history.applied(0)[0].id;
        doc=step(&mut history,doc,EditCommand::Undo);assert_eq!(doc.text(Side::Right),"new\n");assert_eq!(std::fs::read_to_string(&r).unwrap(),"old\n");
        doc=step(&mut history,doc,EditCommand::Discard);assert_eq!(doc.text(Side::Right),"old\n");assert!(history.applied(0)[0].active);assert_eq!(history.applied(0)[0].id,saved_id);assert!(!history.has_dirty());
        doc=step(&mut history,doc,EditCommand::UndoApplied(saved_id));assert_eq!(doc.text(Side::Right),"new\n");
        doc=step(&mut history,doc,EditCommand::Save);assert_eq!(std::fs::read_to_string(&r).unwrap(),"new\n");assert!(!history.applied(0)[0].active);
        doc=step(&mut history,doc,EditCommand::Undo);assert_eq!(doc.text(Side::Right),"old\n");
        assert_eq!(std::fs::read_to_string(&r).unwrap(),"new\n");
    }
    #[test]
    fn pending_draft_is_recoverable_after_disk_file_is_removed() {
        let dir=tempfile::tempdir().unwrap();let(l,r)=(dir.path().join("l"),dir.path().join("r"));
        let doc=doc("old\n","new\n",&l,&r);let mut history=EditSession::default();let view=doc.view(&doc.line_diff(&Options::default()),20.0);
        let draft=history.execute(0,&doc,&view,EditCommand::Apply{block:0,onto:Side::Right},[&l,&r]).unwrap().document;
        std::fs::remove_file(&r).unwrap();let recovered=history.recover(0,[&l,&r]).unwrap();assert_eq!(recovered.text(Side::Right),draft.text(Side::Right));
        assert!(matches!(history.execute(0,&recovered,&view,EditCommand::Save,[&l,&r]),Err(EditError::Write(WriteError::ChangedOnDisk))));
        assert!(!r.exists());assert!(history.has_dirty());
        let discarded=history.execute(0,&recovered,&view,EditCommand::Discard,[&l,&r]).unwrap().document;assert_eq!(discarded.text(Side::Right),"");assert!(!history.has_dirty());assert!(!r.exists());
    }
    #[test]
    fn preflight_checks_every_dirty_side_before_writing_any() {
        let dir=tempfile::tempdir().unwrap();let(l,r)=(dir.path().join("l"),dir.path().join("r"));let mut doc=doc("left\n","right\n",&l,&r);
        doc.writable[1]=Ok(write::Snapshot::read(&r,Some("windows-1252")).unwrap().0);
        let mut pending=Pending::new(&doc);pending.current=[Arc::from("changed left\n"),Arc::from("emoji 😀\n")];
        let mut history=EditSession::default();history.histories.insert(0,History{pending:Some(pending),..Default::default()});
        let recovered=history.recover(0,[&l,&r]).unwrap();let view=recovered.view(&[],20.0);
        assert!(matches!(history.execute(0,&recovered,&view,EditCommand::Save,[&l,&r]),Err(EditError::Write(WriteError::Unrepresentable{..}))));
        assert_eq!(std::fs::read_to_string(&l).unwrap(),"left\n");assert_eq!(std::fs::read_to_string(&r).unwrap(),"right\n");assert!(history.has_dirty());
    }
    #[test]
    fn aliased_comparisons_cannot_create_competing_drafts() {
        let dir=tempfile::tempdir().unwrap();let(l,r)=(dir.path().join("l"),dir.path().join("r"));let doc=doc("old\n","new\n",&l,&r);
        let mut history=EditSession::default();let view=doc.view(&doc.line_diff(&Options::default()),20.0);
        history.execute(0,&doc,&view,EditCommand::Apply{block:0,onto:Side::Right},[&l,&r]).unwrap();
        let alias=dir.path().join("hardlink");std::fs::hard_link(&r,&alias).unwrap();
        let mut aliased=TextDocument::new("old\n","new\n",&l,&alias);aliased.writable[1]=Ok(write::Snapshot::read(&alias,None).unwrap().0);
        let view=aliased.view(&aliased.line_diff(&Options::default()),20.0);
        assert!(matches!(history.execute(1,&aliased,&view,EditCommand::Apply{block:0,onto:Side::Right},[&l,&alias]),Err(EditError::Unavailable(_))));
        assert_eq!(history.dirty_files(),vec![0]);assert_eq!(std::fs::read_to_string(&r).unwrap(),"new\n");
    }

    #[test]
    fn newly_dirty_side_uses_its_fresh_baseline_while_other_draft_is_kept() {
        let dir=tempfile::tempdir().unwrap();let(l,r)=(dir.path().join("l"),dir.path().join("r"));let doc=doc("left\n","right\n",&l,&r);
        let mut history=EditSession::default();let view=doc.view(&doc.line_diff(&Options::default()),20.0);
        history.execute(0,&doc,&view,EditCommand::Apply{block:0,onto:Side::Left},[&l,&r]).unwrap();
        std::fs::write(&r,"external\n").unwrap();let mut fresh=TextDocument::new("left\n","external\n",&l,&r);
        fresh.writable=[Ok(write::Snapshot::read(&l,None).unwrap().0),Ok(write::Snapshot::read(&r,None).unwrap().0)];
        let doc=history.overlay(0,&fresh,[&l,&r]).unwrap();let view=doc.view(&doc.line_diff(&Options::default()),20.0);
        let doc=history.execute(0,&doc,&view,EditCommand::Apply{block:0,onto:Side::Right},[&l,&r]).unwrap().document;
        assert_eq!(std::fs::read_to_string(&r).unwrap(),"external\n");assert_eq!(doc.text(Side::Left),"right\n");
        let _=save(&mut history,0,doc,&l,&r);assert_eq!(std::fs::read_to_string(&l).unwrap(),"right\n");assert_eq!(std::fs::read_to_string(&r).unwrap(),"right\n");
    }
    #[test]
    fn discard_preserves_bomless_utf16_encoding_and_subsequent_saves() {
        let dir=tempfile::tempdir().unwrap();let(l,r)=(dir.path().join("l"),dir.path().join("r"));std::fs::write(&l,"draft\n").unwrap();
        let original="original\n".encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<_>>();std::fs::write(&r,&original).unwrap();
        let(snapshot,decoded)=write::Snapshot::read(&r,Some("UTF-16LE")).unwrap();let mut doc=TextDocument::new("draft\n",&decoded.text,&l,&r);doc.writable[1]=Ok(snapshot);
        let mut history=EditSession::default();let view=doc.view(&doc.line_diff(&Options::default()),20.0);
        doc=history.execute(0,&doc,&view,EditCommand::Apply{block:0,onto:Side::Right},[&l,&r]).unwrap().document;
        doc=history.execute(0,&doc,&view,EditCommand::Discard,[&l,&r]).unwrap().document;assert_eq!(doc.text(Side::Right),"original\n");assert_eq!(doc.writable[1].as_ref().unwrap().encoding.name(),"UTF-16LE");
        let view=doc.view(&doc.line_diff(&Options::default()),20.0);doc=history.execute(0,&doc,&view,EditCommand::Apply{block:0,onto:Side::Right},[&l,&r]).unwrap().document;
        let _=save(&mut history,0,doc,&l,&r);assert_eq!(std::fs::read(&r).unwrap(),"draft\n".encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<_>>());
    }
    #[test]
    fn identical_bytes_at_retargeted_symlink_are_not_a_safe_save() {
        let dir=tempfile::tempdir().unwrap();let(l,r)=(dir.path().join("l"),dir.path().join("r"));let doc=doc("draft\n","disk\n",&l,&r);
        let mut history=EditSession::default();let view=doc.view(&doc.line_diff(&Options::default()),20.0);
        let doc=history.execute(0,&doc,&view,EditCommand::Apply{block:0,onto:Side::Right},[&l,&r]).unwrap().document;
        let external=dir.path().join("external");std::fs::write(&external,"disk\n").unwrap();std::fs::remove_file(&r).unwrap();std::os::unix::fs::symlink(&external,&r).unwrap();
        assert!(history.execute(0,&doc,&view,EditCommand::Save,[&l,&r]).is_err());assert!(history.has_dirty());assert_eq!(std::fs::read_to_string(&external).unwrap(),"disk\n");
        let doc=history.execute(0,&doc,&view,EditCommand::Discard,[&l,&r]).unwrap().document;assert!(doc.writable[1].is_err());assert!(!history.has_dirty());
    }

    #[test]
    fn readiness_uncertain_save_retains_draft_history_and_confirmed_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let (left, right) = (dir.path().join("left"), dir.path().join("right"));
        let document = doc("draft\n", "disk\n", &left, &right);
        let mut history = EditSession::default();
        let view = document.view(&document.line_diff(&Options::default()), 20.0);
        let draft = history.execute(0, &document, &view, EditCommand::Apply { block: 0, onto: Side::Right }, [&left, &right]).unwrap().document;
        let result = write::readiness_fault::during(write::readiness_fault::Fault::AfterReplace,
            || history.execute(0, &draft, &view, EditCommand::Save, [&left, &right]));
        assert!(result.is_err());
        assert!(history.has_dirty() && history.has_undo(0));
        let pending = history.histories[&0].pending.as_ref().unwrap();
        assert_eq!(&*pending.current[1], "draft\n");
        assert_eq!(&*pending.saved[1], "disk\n");
        assert_eq!(std::fs::read_to_string(&right).unwrap(), "draft\n");
        assert!(matches!(history.execute(0, &draft, &view, EditCommand::Save, [&left, &right]), Err(EditError::Write(WriteError::ChangedOnDisk))));
    }

    #[test]
    fn symlink_replacement_before_apply_cannot_gain_write_permission() {
        let dir=tempfile::tempdir().unwrap();let(l,r)=(dir.path().join("l"),dir.path().join("r"));let doc=doc("draft\n","disk\n",&l,&r);
        let external=dir.path().join("external");std::fs::write(&external,"disk\n").unwrap();std::fs::remove_file(&r).unwrap();std::os::unix::fs::symlink(&external,&r).unwrap();
        let mut history=EditSession::default();let view=doc.view(&doc.line_diff(&Options::default()),20.0);let draft=history.execute(0,&doc,&view,EditCommand::Apply{block:0,onto:Side::Right},[&l,&r]).unwrap().document;
        assert!(history.execute(0,&draft,&view,EditCommand::Save,[&l,&r]).is_err());assert!(history.has_dirty());assert_eq!(std::fs::read_to_string(&external).unwrap(),"disk\n");
    }

}
