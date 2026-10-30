//! Shared text-comparison state and transitions; adapters own resources and scheduling.
use crate::markdown::SourcePos as _;
use std::{collections::HashMap, sync::Arc};
use lenz_engine::{Budget, Options};
use crate::{changeset::Comparison, commands::{self, Command, Effect}, document::TextDocument,
    find::Find, navigator::PressAgain, selection::Pos, view::{Side, View}};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Place { Top, Anchor(f32), Line(Side, u32), FirstChange, LastChange }

#[derive(Debug, PartialEq)]
pub enum Transition { Repaint, Recompare, Presentation(crate::markdown::PresentationMode), Load { index: usize, place: Place, force: bool }, Notice(String) }

#[cfg(feature="gui")]
#[derive(Default)]
pub(crate) struct DiffCache(pub(crate) HashMap<usize, (Options, View, Budget)>);
#[cfg(feature="gui")]
impl DiffCache {
    pub(crate) fn put(&mut self, index: usize, options: Options, view: View, budget: Budget) { self.0.insert(index, (options, view, budget)); }
    pub(crate) fn take(&mut self, index: usize, options: Options) -> Option<(View, Budget)> {
        let (old, view, budget) = self.0.remove(&index)?;
        (old == options).then_some((view, budget))
    }
}

pub struct ComparisonSession {
    pub(crate) current: usize,
    pub(crate) view: View,
    pub(crate) options: Options,
    pub(crate) collapse: bool,
    pub(crate) press_again: PressAgain,
    pub(crate) find: Option<Find>,
    /// Case, whole word, regex: this window's find options (comparison find, merge find, Change Set search).
    pub find_options: lenz_content::find::FindOptions,
    pub(crate) generation: u64,
    pub(crate) work_id: Arc<()>,
    pub(crate) diffed: u64,
    pub(crate) refining: bool,
    pub(crate) budget: Budget,
    #[cfg(feature="gui")]
    pub(crate) cache: DiffCache,
    markdown: Option<Box<crate::markdown::MarkdownState>>,
    markdown_revision: u64,
    markdown_layout: crate::markdown::MarkdownLayout,
    presentation_notice: Option<&'static str>,
    positions:HashMap<usize,(Options,[u64;2],crate::view::Presentation,u64)>,
    pub(crate) folds: HashMap<usize, Vec<(String, String)>>,
    pub(crate) banner: Option<String>,
    /// Collapsed folders and the Folder Summary of the File Navigator.
    pub folders: crate::folders::Folders,
    /// The comparison set has no files (an empty window, a selection without changes): nothing is loading.
    pub(crate) empty: bool,
}

impl ComparisonSession {
    pub fn new(view: View, options: Options) -> Self {
        Self { current: 0, view, options, collapse: true, press_again: PressAgain::default(), find: None, find_options: Default::default(),
            generation: 0, work_id: Arc::new(()), diffed: 0, refining: false, budget: Budget::default(),
            #[cfg(feature="gui")]
            cache: DiffCache::default(), markdown: None, markdown_revision: 0, markdown_layout: crate::settings::get().view.markdown_layout, presentation_notice: None, positions:HashMap::new(), folds: HashMap::new(), banner: None, folders: Default::default(), empty: false }
    }
    /// The change blocks to refine next: near the viewport in Source; in the Markdown Preview every change, a
    /// batch at a time, since its word outlines show anywhere in the document.
    pub fn refine_targets(&self) -> Vec<(usize, lenz_engine::ChangeBlock)> {
        if self.markdown_mode() == crate::markdown::PresentationMode::Rendered {
            self.view.unrefined_blocks(256)
        } else {
            self.view.blocks_to_refine()
        }
    }
    /// Inline or Side by side, for the GUI Preview.
    pub fn markdown_layout(&self) -> crate::markdown::MarkdownLayout { self.markdown_layout }
    pub fn markdown_command(&mut self, command: crate::markdown::MarkdownCommand, doc: Option<Arc<TextDocument>>) {
        self.markdown_command_document(command, doc.as_deref());
    }
    pub fn markdown_command_document(&mut self, command: crate::markdown::MarkdownCommand, doc: Option<&TextDocument>) {
        use crate::markdown::{MarkdownCommand, MarkdownState, PresentationMode, source_diff};
        self.presentation_notice = None;
        if let MarkdownCommand::SetMode(mode) = command {
            if let Some(reason) = self.presentation_disabled(mode, doc) { self.presentation_notice = Some(reason); return; }
            // the Preview has its own layout: Unified gives way to Split
            if mode == PresentationMode::Rendered { self.view.set_unified(false); }
            if mode == PresentationMode::Rendered { self.markdown.get_or_insert_with(|| Box::new(MarkdownState::new())); }
        }
        let Some(markdown) = &mut self.markdown else { return; };
        let entering = matches!(command, MarkdownCommand::SetMode(PresentationMode::Rendered))
            && markdown.mode() == PresentationMode::Source;
        let source_lines = entering.then(|| [self.view.top_line_on(Side::Left) as usize, self.view.top_line_on(Side::Right) as usize]);
        let diff = matches!(command, MarkdownCommand::SetMode(PresentationMode::Rendered))
            .then(|| doc.map(|doc| source_diff(doc, &self.view, self.markdown_revision))).flatten();
        markdown.execute_document(command, doc, diff);
        if let Some(source_lines) = source_lines && let Some(doc) = doc {
            for (side, line) in [Side::Left, Side::Right].into_iter().zip(source_lines) {
                let text = doc.text(side);
                let byte = text.split_inclusive('\n').take(line).map(str::len).sum();
                markdown.execute_document(MarkdownCommand::SetAnchor(crate::markdown::SourceAnchor { side, byte }), None, None);
            }
        }
        if let MarkdownCommand::ShowSource(anchor) = command
            && markdown.mode() == PresentationMode::Source
            && markdown.anchor(anchor.side) == Some(anchor)
            && let Some(pos) = markdown.projection().and_then(|p| p.pair.sides[anchor.side as usize].source_pos(anchor.byte))
        { self.view.go_to_line(anchor.side, pos.line + 1); }

    }
    pub fn applied_undo_disabled(&self, record: &crate::edit_session::Applied) -> Option<&'static str> {
        self.edit_blocked().or_else(|| record.undo_reason())
    }
    /// Preserve the source line convention (one-based, clamped by View) while
    /// giving either rendered adapter the corresponding decoded-byte anchor.
    pub fn go_to_line(&mut self, side: Side, line: u32) {
        self.view.go_to_line(side, line);
        if let Some(markdown) = &mut self.markdown
            && let Some(projection) = markdown.projection() {
            let doc = &projection.pair.sides[side as usize];
            let last = doc.line_starts.len().saturating_sub(if doc.source.ends_with('\n') { 2 } else { 1 });
            let index = (line.saturating_sub(1) as usize).min(last);
            let anchor = crate::markdown::SourceAnchor { side, byte: doc.line_starts[index] };
            markdown.execute_document(crate::markdown::MarkdownCommand::SetAnchor(anchor), None, None);
        }
    }
    fn anchor_find(&mut self) {
        let Some(find) = &self.find else { return; };
        let Some((line, columns)) = find.at.and_then(|i| find.hits.get(i)) else { return; };
        let Some(markdown) = &mut self.markdown else { return; };
        if markdown.mode() != crate::markdown::PresentationMode::Rendered { return; }
        if let Some(start) = markdown.projection().and_then(|p| p.pair.sides[find.side as usize].line_starts.get(*line as usize)).copied() {
            markdown.execute_document(crate::markdown::MarkdownCommand::SetAnchor(crate::markdown::SourceAnchor { side: find.side, byte: start + columns.start }), None, None);
        }
    }
    pub fn markdown_selection(&self) -> Option<crate::markdown::RenderSelection> { self.markdown.as_ref().and_then(|state| state.selection()) }
    /// Shared write gate for both owning adapters, before history or disk mutation.
    pub fn edit(&self, edits: &mut crate::edit_session::EditSession, doc: &TextDocument, command: crate::edit_session::EditCommand, paths: [&std::path::Path; 2]) -> Result<crate::edit_session::Written, crate::edit_session::EditError> {
        if !matches!(command,crate::edit_session::EditCommand::Save|crate::edit_session::EditCommand::Discard) && let Some(reason) = self.edit_blocked() { return Err(crate::edit_session::EditError::Unavailable(reason.into())); }
        edits.execute(self.current, doc, &self.view, command, paths)
    }
    pub fn edit_disabled(&self, edits: &crate::edit_session::EditSession, doc: Option<&TextDocument>, command: crate::edit_session::EditMenuCommand) -> Option<String> {
        if !matches!(command,crate::edit_session::EditMenuCommand::History|crate::edit_session::EditMenuCommand::Save|crate::edit_session::EditMenuCommand::Discard) && let Some(reason) = self.edit_blocked() { return Some(reason.into()); }
        if doc.is_none() { return Some(self.absent().into()); }
        edits.disabled(self.current, doc, &self.view, command)
    }
    /// Why there's no document: none to load, or not loaded yet.
    pub fn absent(&self) -> &'static str { if self.empty { commands::NOTHING_OPEN } else { "Comparison loading" } }
    pub fn undo_file(&self, edits: &mut crate::edit_session::EditSession, set: &crate::changeset::ChangeSet, index: usize) -> Result<crate::edit_session::Written, crate::edit_session::EditError> {
        if let Some(reason) = self.edit_blocked() { return Err(crate::edit_session::EditError::Unavailable(reason.into())); }
        edits.undo_file(set, index)
    }
    pub fn move_source_caret(&mut self, doc: &TextDocument, selection: crate::selection::Selection, command: crate::selection::CaretCommand, extend: bool) -> crate::selection::Selection {
        if self.markdown_edit_disabled().is_some() { return selection; }
        crate::selection::move_caret(selection, doc.lines_on(selection.side), &mut self.view, command, extend)
    }
    pub fn copy_disabled(&self, command: commands::Interaction, mut context: commands::CopyContext<'_>) -> Option<&'static str> {
        if self.markdown_mode() == crate::markdown::PresentationMode::Rendered && matches!(command, commands::Interaction::Copy | commands::Interaction::CopyMarkdownSource) {
            return self.markdown_selection().filter(|s| s.anchor != s.head).filter(|s| self.markdown_projection().is_some_and(|p| p.valid_selection(*s))).is_none().then_some("No selected text");
        }
        if self.markdown_mode() == crate::markdown::PresentationMode::Rendered {
            context.selection = self.markdown_selection().and_then(|s| self.markdown_projection()?.source_selection(s));
            if let Some(selection) = self.markdown_selection() { context.side = selection.side; }
        }
        commands::copy_disabled(command, context)
    }
    pub fn copy_text(&self, command: commands::Interaction, mut context: commands::CopyContext<'_>) -> Result<String, &'static str> {
        if self.markdown_mode() == crate::markdown::PresentationMode::Rendered {
            if matches!(command, commands::Interaction::Copy | commands::Interaction::CopyMarkdownSource) {
                let selected = self.markdown_selection().filter(|s| s.anchor != s.head).ok_or("No selected text")?;
                let projection = self.markdown_projection().ok_or("Comparison loading")?;
                return (if command == commands::Interaction::Copy { projection.copy_visible(selected) } else { projection.copy_source(selected) }).map_err(|_| "Selection is no longer valid");
            }
            context.selection = self.markdown_selection().and_then(|s| self.markdown_projection()?.source_selection(s));
            if let Some(selection) = self.markdown_selection() { context.side = selection.side; }
        }
        commands::copy_text(command, context)
    }
    /// Source search remains authoritative; mapped empty ranges expose hidden hits.
    pub fn markdown_find_hit(&self) -> Option<(crate::markdown::SourceAnchor, std::ops::Range<usize>, Vec<std::ops::Range<usize>>)> {
        let find = self.find.as_ref()?;
        let (line, columns) = find.hits.get(find.at?)?;
        let projection = self.markdown_projection()?;
        let start = *projection.pair.sides[find.side as usize].line_starts.get(*line as usize)?;
        let source = start + columns.start .. start + columns.end;
        Some((crate::markdown::SourceAnchor { side: find.side, byte: source.start }, source.clone(), projection.find_source(find.side, source)))
    }
    /// Called only when source diff geometry/inline decorations change, never on each frame.
    pub fn markdown_refresh(&mut self, doc: Option<&TextDocument>) {
        let Some(markdown) = &mut self.markdown else { return; };
        self.markdown_revision = self.markdown_revision.wrapping_add(1);
        let diff = (markdown.preferred_mode() == crate::markdown::PresentationMode::Rendered)
            .then(|| doc.filter(|doc| crate::markdown::eligible(doc)).map(|doc| crate::markdown::source_diff(doc, &self.view, self.markdown_revision))).flatten();
        markdown.refresh(doc, diff);
    }
    /// Precise source location for editor, annotation and line-thread inspection.
    /// A normalized or generated glyph offers Source instead of a fabricated caret.
    pub fn markdown_inspection_position(&self, side: Side) -> Result<Option<crate::selection::Pos>, &'static str> {
        use crate::markdown::{Origin, PresentationMode};
        if self.markdown_mode() != PresentationMode::Rendered { return Ok(None); }
        let reason = "Show in source to inspect this position";
        let projection = self.markdown_projection().ok_or(reason)?;
        let doc = &projection.pair.sides[side as usize];
        if let Some(selection) = self.markdown_selection().filter(|s| s.side == side) {
            if !projection.valid_selection(selection) { return Err(reason); }
            let byte = selection.head;
            let index=doc.segments.partition_point(|s|s.text.end<=byte);
            let segment = doc.segments.get(index).filter(|s|s.text.contains(&byte))
                .or_else(|| (byte == doc.text.len()).then(|| doc.segments.iter().rev().find(|s| s.text.end == byte && !s.text.is_empty())).flatten())
                .ok_or(reason)?;
            return match &segment.origin {
                Origin::Direct(source) => doc.source_pos(source.start + byte - segment.text.start).map(Some).ok_or(reason),
                Origin::Normalized(_) | Origin::Synthetic => Err(reason),
            };
        }
        self.markdown_anchor(side).and_then(|a| doc.source_pos(a.byte)).map(Some).ok_or(reason)
    }
    /// Carry a view's presentation choice across an owning Session replacement.
    /// This retains only the enum; accepted document refresh starts preparation.
    pub fn markdown_inherit_presentation(&mut self, previous: &Self) {
        let mode = previous.markdown_preferred_mode();
        if mode == crate::markdown::PresentationMode::Source && self.markdown.is_none() { return; }
        let state=self.markdown.get_or_insert_with(|| Box::new(crate::markdown::MarkdownState::new()));
        state.invalidate();
        state.execute_document(crate::markdown::MarkdownCommand::SetMode(mode), None, None);
    }
    /// Prefer `mode` for the Markdown files this view opens, without a notice for a current file that is not one
    /// (the settings file's `[view] markdown`).
    pub fn prefer_presentation(&mut self, mode: crate::markdown::PresentationMode) {
        if mode == crate::markdown::PresentationMode::Source && self.markdown.is_none() { return; }
        let state = self.markdown.get_or_insert_with(|| Box::new(crate::markdown::MarkdownState::new()));
        state.execute_document(crate::markdown::MarkdownCommand::SetMode(mode), None, None);
    }
    pub fn markdown_preferred_mode(&self) -> crate::markdown::PresentationMode {
        self.markdown.as_ref().map_or(crate::markdown::PresentationMode::Source, |state| state.preferred_mode())
    }
    pub fn markdown_invalidate(&mut self) { self.presentation_notice = None; if let Some(markdown) = &mut self.markdown { markdown.invalidate(); } }
    pub fn markdown_poll(&mut self) -> bool { self.markdown.as_mut().is_some_and(|state| state.poll()) }
    pub fn markdown_loading(&self) -> bool { self.markdown.as_ref().is_some_and(|state| state.loading()) }
    pub fn markdown_projection(&self) -> Option<&Arc<crate::markdown::Projection>> { self.markdown.as_ref().and_then(|state| state.projection()) }
    pub fn markdown_anchor(&self, side: Side) -> Option<crate::markdown::SourceAnchor> { self.markdown.as_ref().and_then(|state| state.anchor(side)) }
    pub fn markdown_mode(&self) -> crate::markdown::PresentationMode { self.markdown.as_ref().map_or(crate::markdown::PresentationMode::Source, |state| state.mode()) }
    pub fn markdown_notice(&self) -> Option<&str> { self.presentation_notice.or_else(|| self.markdown.as_ref().and_then(|state| state.notice())) }
    /// Why edits cannot run now: the Markdown Preview, or the read-only Unified layout.
    pub fn edit_blocked(&self) -> Option<&'static str> {
        self.markdown_edit_disabled().or(self.view.unified().then_some("Switch to Split to edit"))
    }
    pub fn markdown_edit_disabled(&self) -> Option<&'static str> { self.markdown.as_ref().and_then(|state| state.edit_disabled()) }
    pub fn presentation_disabled(&self, mode: crate::markdown::PresentationMode, doc: Option<&TextDocument>) -> Option<&'static str> {
        match mode {
            crate::markdown::PresentationMode::Source => None,
            crate::markdown::PresentationMode::Rendered => match doc {
                None => Some(self.absent()),
                Some(doc) if !crate::markdown::eligible(doc) => Some("Not a Markdown comparison"),
                Some(_) => None,
            },
        }
    }
    pub fn remap_files(&mut self,old:&crate::changeset::ChangeSet,new:&crate::changeset::ChangeSet){
        if self.positions.is_empty()&&self.folds.is_empty(){return;}
        let indices=(0..new.comparisons.len()).filter_map(|i|crate::changeset::comparison_identity(new,i).map(|key|(key,i))).collect::<HashMap<_,_>>();
        self.positions=std::mem::take(&mut self.positions).into_iter().filter_map(|(i,entry)|crate::changeset::comparison_identity(old,i).and_then(|key|indices.get(&key)).map(|&index|(index,entry))).collect();
        self.folds=std::mem::take(&mut self.folds).into_iter().filter_map(|(i,entry)|crate::changeset::comparison_identity(old,i).and_then(|key|indices.get(&key)).map(|&index|(index,entry))).collect();
    }
    pub fn view(&self) -> &View { &self.view }
    pub fn options(&self) -> Options { self.options }
    pub fn revision(&self) -> u64 { self.generation }
    pub fn begin(&mut self) -> u64 {
        crate::diagnostics::breadcrumb("Load or refresh comparison");
        if let Some(markdown) = &mut self.markdown { markdown.cancel(); }
        self.generation += 1; self.work_id = Arc::new(()); self.refining = false; self.banner = None; self.generation
    }
    pub fn command(&mut self, command: Command, files: &[Comparison], loading: bool, large: bool) -> Transition {
        if let Some(reason) = commands::disabled(command, &self.view, loading, self.current, files.len(), large) {
            return Transition::Notice(reason.into());
        }
        // stepping through changes shows the file behind a Folder Summary
        if matches!(command, Command::NextChange | Command::PreviousChange) && self.folders.summary().is_some() {
            self.folders.close_summary();
            self.folders.reveal(files, self.current);
            return Transition::Repaint;
        }
        let effect = commands::execute(command, &mut self.view, &mut self.options, &mut self.collapse);
        if matches!(command, Command::NextChange | Command::PreviousChange)
            && let Some(hunk) = self.view.current_change()
            && let Some(markdown) = &mut self.markdown {
            for side in [Side::Left, Side::Right] {
                if let Some(anchor) = markdown.projection().and_then(|p| p.location(hunk, side)) {
                    markdown.execute_document(crate::markdown::MarkdownCommand::SetAnchor(anchor), None, None);
                }
            }
        }
        if !matches!(effect, Effect::Edge(_)) { self.press_again.reset(); }
        match effect {
            Effect::Repaint => Transition::Repaint,
            Effect::Presentation(mode) => Transition::Presentation(mode),
            Effect::MarkdownLayout(layout) => { self.markdown_layout = layout; Transition::Repaint }
            Effect::Preview(layout) => { self.markdown_layout = layout; Transition::Presentation(crate::markdown::PresentationMode::Rendered) }
            Effect::Recompare => Transition::Recompare,
            Effect::LoadLarge => Transition::Load { index: self.current, place: Place::Top, force: true },
            // files a collapsed folder hides are skipped
            Effect::File(forward) => match self.folders.step(files, self.current, forward) {
                Some(index) => Transition::Load { index, place: Place::Top, force: false },
                None => Transition::Notice(format!("No {} visible file", if forward { "later" } else { "earlier" })),
            },
            Effect::Edge(forward) => {
                match self.folders.step(files, self.current, forward) {
                    None => { self.press_again.reset(); Transition::Notice(format!("{} change in the Change Set", if forward { "Last" } else { "First" })) }
                    Some(index) if self.press_again.at_edge(forward) => Transition::Load { index, place: if forward { Place::FirstChange } else { Place::LastChange }, force: false },
                    Some(index) => Transition::Notice(format!("Press {} again for {}", if forward { "n" } else { "p" }, files[index].path)),
                }
            }
        }
    }
    pub fn fold(&mut self, doc: &TextDocument, block: Option<usize>) {
        match block { Some(i) => self.view.toggle_fold(i), None => self.view.unfold_all() }
        let texts = self.view.folded_blocks().iter().map(|b| doc.block_text(b)).collect::<Vec<_>>();
        if texts.is_empty() { self.folds.remove(&self.current); } else { self.folds.insert(self.current, texts); }
    }
    pub fn remember_folds(&mut self, doc: &TextDocument) {
        let texts = self.view.folded_blocks().iter().map(|b| doc.block_text(b)).collect::<Vec<_>>();
        if texts.is_empty() { self.folds.remove(&self.current); } else { self.folds.insert(self.current, texts); }
    }
    pub fn remember_position(&mut self,doc:&TextDocument){
        if self.diffed!=self.generation{return;}
        self.positions.insert(self.current,(self.options,[crate::moves::fingerprint(doc.text(Side::Left)),crate::moves::fingerprint(doc.text(Side::Right))],self.view.presentation(),self.generation));
        if self.positions.len()>8 && let Some(index)=self.positions.iter().min_by_key(|(_,entry)|entry.3).map(|(&i,_)|i){self.positions.remove(&index);}
    }
    pub fn restore_position(&mut self,doc:&TextDocument,place:Place){
        if !matches!(place,Place::Top){return;}
        if let Some((options,hashes,saved,_))=self.positions.get(&self.current) && *options==self.options && *hashes==[crate::moves::fingerprint(doc.text(Side::Left)),crate::moves::fingerprint(doc.text(Side::Right))]{self.view.restore_presentation(saved);}
    }
    pub fn search(&mut self, doc: &TextDocument, query: String, side: Side, from: Pos) -> Option<(u32, std::ops::Range<usize>)> {
        self.find = Some(Find::new(query, self.find_options, side, doc.lines_on(side), from));
        self.anchor_find();
        self.find.as_ref().and_then(|f| f.at.map(|i| f.hits[i].clone()))
    }
    /// Flip one find option and re-run the open find with it, from its current match.
    pub fn toggle_find_option(&mut self, doc: &TextDocument, option: lenz_content::find::FindOption) -> Option<(u32, std::ops::Range<usize>)> {
        self.find_options = self.find_options.toggled(option);
        let f = self.find.as_ref()?;
        let from = f.at.and_then(|i| f.hits.get(i)).map_or(Pos::new(0, 0), |(l, r)| Pos::new(*l, r.start));
        let (query, side) = (f.query.clone(), f.side);
        self.search(doc, query, side, from)
    }
    pub fn search_step(&mut self, forward: bool) -> Option<(Side, u32, std::ops::Range<usize>, bool)> {
        let find = self.find.as_mut()?;
        let (i, wrapped) = find.next_index(forward)?;
        find.at = Some(i);
        let (line, range) = find.hits[i].clone();
        let side = find.side;
        self.anchor_find();
        Some((side, line, range, wrapped))
    }
    pub fn show_text(&mut self, doc: &TextDocument, place: Place) {
        let mut view = doc.view(&[], self.view.viewport_rows());
        view.start_in(self.view.mode());
        match place { Place::Line(side, line) => view.go_to_line(side, line), Place::Anchor(line) => view.place_left_at(line), _ => {} }
        view.set_unified(self.view.unified());
        self.view = view;
    }
    /// Reject obsolete completions, then skip unchanged files while stepping across a Change Set.
    /// Installs a finished diff; for an unchanged file reached by n/p, the next visible file to try instead.
    pub fn complete(&mut self, generation: u64, mut view: View, budget: Budget, place: Place, files: &[Comparison]) -> Option<usize> {
        if generation != self.generation { return None; }
        if matches!(place,Place::Anchor(_)){view.retain_recompare_position(&self.view.recompare_position());}
        let empty = view.change_count() == 0;
        view.set_unified(self.view.unified());
        self.view = view; self.budget = budget; self.diffed = generation;
        let next = match place { Place::FirstChange => self.folders.step(files, self.current, true), Place::LastChange => self.folders.step(files, self.current, false), _ => None };
        next.filter(|_| empty)
    }
    pub fn expire(&mut self, generation: u64, doc: &TextDocument) -> bool {
        if generation != self.generation || self.diffed == generation { return false; }
        let anchor = self.view.anchor_line(); self.begin(); self.diffed = self.generation;
        self.show_text(doc, Place::Anchor(anchor));
        self.banner = Some("Unable to calculate diff — file too big / too many changes".into()); true
    }
}

pub fn prepare(doc: &TextDocument, options: Options, rows: f32, mode: crate::view::ScrollMode, collapse: bool,
    folds: &[(String, String)], place: Place) -> (View, Budget) {
    prepare_with_position(doc,options,rows,mode,collapse,folds,place,None)
}
#[allow(clippy::too_many_arguments)] // Optional compact presentation, never a copy of the document or view.
pub(crate) fn prepare_with_position(doc:&TextDocument,options:Options,rows:f32,mode:crate::view::ScrollMode,collapse:bool,folds:&[(String,String)],place:Place,position:Option<crate::view::RecomparePosition>)->(View,Budget){
    let mut view = doc.view(&doc.line_diff(&options), rows);
    view.start_in(mode); view.set_collapsed(collapse); doc.refold(&mut view, folds);
    match place {
        Place::Top => {}, Place::Anchor(line) => view.place_left_at(line), Place::Line(side,line) => view.go_to_line(side,line),
        Place::FirstChange => view.jump_to_first_change(), Place::LastChange => view.jump_to_last_change(),
    }
    if let Some(position)=position{view.retain_recompare_position(&position);}
    let mut budget = Budget::default(); doc.refine_visible(&mut view, &options, &mut budget); (view,budget)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changeset::ChangeSet;

    #[test]
    fn unified_layout_carries_to_new_views_and_refuses_edits() {
        use crate::commands::{Command, DiffLayout};
        let doc = Arc::new(TextDocument::new("a\nb\n", "a\nc\n", std::path::Path::new("a.txt"), std::path::Path::new("b.txt")));
        let view = doc.view(&doc.line_diff(&Options::default()), 40.);
        let mut s = ComparisonSession::new(view, Options::default());
        s.command(Command::SetDiffLayout(DiffLayout::Unified), &[], false, false);
        assert!(s.view.unified());
        assert_eq!(s.edit_blocked(), Some("Switch to Split to edit"));
        s.show_text(&doc, Place::Top);
        assert!(s.view.unified(), "a new view keeps Unified");
        s.command(Command::SetDiffLayout(DiffLayout::Split), &[], false, false);
        assert!(!s.view.unified());
        assert_eq!(s.edit_blocked(), None);
    }

    #[test]
    fn the_preview_refines_every_change_not_only_the_visible_ones() {
        let body: String = (0..1000).map(|i| format!("Paragraph {i} words.\n\n")).collect();
        let right = body.replace("Paragraph 3 words", "Paragraph 3 changed").replace("Paragraph 900 words", "Paragraph 900 changed");
        let doc = Arc::new(TextDocument::new(&body, &right, std::path::Path::new("a.md"), std::path::Path::new("b.md")));
        let view = doc.view(&doc.line_diff(&Options::default()), 40.);
        let mut s = ComparisonSession::new(view, Options::default());
        assert_eq!(s.refine_targets().len(), 1, "Source refines what is near the viewport");
        s.markdown_command(crate::markdown::MarkdownCommand::SetMode(crate::markdown::PresentationMode::Rendered), Some(doc));
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while s.markdown_mode() != crate::markdown::PresentationMode::Rendered {
            s.markdown_poll();
            assert!(std::time::Instant::now() < until);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(s.refine_targets().len(), 2, "the Preview shows every change, so all are refined");
    }

    #[test]
    fn the_markdown_layout_is_a_comparison_preference() {
        use crate::markdown::MarkdownLayout;
        let mut s = ComparisonSession::new(View::new(0, 0, &[], 30.), Options::default());
        assert_eq!(s.markdown_layout(), MarkdownLayout::Inline);
        assert!(matches!(s.command(Command::SetMarkdownLayout(MarkdownLayout::SideBySide), &[], false, false), Transition::Repaint));
        assert_eq!(s.markdown_layout(), MarkdownLayout::SideBySide);
        // choosing a Preview layout also asks for the Preview
        assert!(matches!(s.command(Command::Preview(MarkdownLayout::Inline), &[], false, false), Transition::Presentation(crate::markdown::PresentationMode::Rendered)));
        assert_eq!(s.markdown_layout(), MarkdownLayout::Inline);
    }

    #[test]
    fn late_diff_and_deadline_results_cannot_replace_current_generation() {
        let mut state = ComparisonSession::new(View::new(2, 2, &[], 10.0), Options::default());
        for _ in 0..12 {
            let old = state.begin();
            let current = state.begin();
            state.complete(current, View::new(3, 1, &[], 10.0), Budget::default(), Place::Top, &[]);
            assert_eq!(state.complete(old, View::new(100, 100, &[], 10.0), Budget::default(), Place::Top, &[]), None);
            let old_doc = TextDocument::new("obsolete\n", "obsolete\n", std::path::Path::new("old"), std::path::Path::new("old"));
            assert!(!state.expire(old, &old_doc));
            assert_eq!(state.revision(), current);
            assert_eq!(state.diffed, current);
            let frame = state.view.frame();
            let real_lines = |rows: &[crate::view::Row]| rows.iter().filter_map(|row| match row.content {
                crate::view::Content::Line(line) => Some(line), _ => None,
            }).collect::<Vec<_>>();
            assert_eq!(real_lines(&frame.left), vec![0, 1, 2]);
            assert_eq!(real_lines(&frame.right), vec![0]);
            assert!(state.banner.is_none());
        }
    }

    #[test]
    fn markdown_state_source_owner_never_allocates_presentation() {
        let doc = TextDocument::new("# a", "# b", std::path::Path::new("a.md"), std::path::Path::new("b.md"));
        let mut owner = ComparisonSession::new(doc.view(&[], 20.0), Options::default());
        owner.markdown_refresh(Some(&doc));
        owner.markdown_command_document(crate::markdown::MarkdownCommand::SetMode(crate::markdown::PresentationMode::Source), Some(&doc));
        assert!(owner.markdown.is_none());
        assert!(!owner.markdown_poll());
        owner.markdown_command_document(crate::markdown::MarkdownCommand::SetMode(crate::markdown::PresentationMode::Rendered), None);
        assert_eq!(owner.markdown_notice(), Some("Comparison loading"));
        assert!(owner.banner.is_none(), "presentation notice must not suppress source refinement");
        assert!(owner.markdown.is_none());
    }

    #[test]
    fn crossing_file_boundary_requires_a_second_press_in_both_directions() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["left", "right"] { std::fs::create_dir(dir.path().join(name)).unwrap(); }
        for i in 0..2 {
            std::fs::write(dir.path().join(format!("left/{i}")), "old\n").unwrap();
            std::fs::write(dir.path().join(format!("right/{i}")), "new\n").unwrap();
        }
        let set = ChangeSet::load(&crate::changeset::Spec::Plain { left: dir.path().join("left"), right: dir.path().join("right") }, dir.path()).unwrap();
        let doc = TextDocument::new("old\n", "new\n", std::path::Path::new("left"), std::path::Path::new("right"));
        let mut state = ComparisonSession::new(doc.view(&doc.line_diff(&Options::default()), 20.0), Options::default());
        state.view.jump_to_first_change();
        assert!(matches!(state.command(Command::NextChange, &set.comparisons, false, false), Transition::Notice(_)));
        assert_eq!(state.command(Command::NextChange, &set.comparisons, false, false), Transition::Load { index: 1, place: Place::FirstChange, force: false });
        state.current = 1;
        assert!(matches!(state.command(Command::PreviousChange, &set.comparisons, false, false), Transition::Notice(_)));
        assert_eq!(state.command(Command::PreviousChange, &set.comparisons, false, false), Transition::Load { index: 0, place: Place::LastChange, force: false });
    }
}
