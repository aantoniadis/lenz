//! Markdown authoring through the Session command seam: Enter continues lists, `](` and `#` suggest project files
//! and headings (Lenz's own suggestions, with no language server), a preview beside the source follows the draft,
//! and positions and links in a preview lead back to the current source.
//!
//! Every edit is one transaction of the shown version, undone in one step. Background work (a file's headings) is
//! fenced by request identity, tab, version and caret, so an answer for text that has changed since is dropped.
use super::*;
use crate::markdown_authoring::{self as authoring, Heading};
use crate::text::{EditKind, TextEdit};
use lenz_content::markdown::{MarkdownDocument, Origin};

/// The Markdown link work waited for: one at a time, the latest wins.
pub(super) struct LinkWait {
    request: u64,
    purpose: Purpose,
}

enum Purpose {
    /// Suggestions for the fragment typed at `caret` of `tab`'s version `source`, replacing from `word`.
    Suggest { tab: TabId, source: SemanticSource, caret: usize, word: usize, trigger: CompletionTrigger, file: String },
    /// A followed link's heading: in the draft `source` of `tab` shows (`path` is `None`), or in file `path`, opened at
    /// it.
    Follow { tab: TabId, source: SemanticSource, path: Option<PathBuf>, fragment: String },
}

/// Where a link's headings are read from: a draft's snapshot (open, possibly unsaved), else the file on disk.
enum HeadingSource {
    Draft(Arc<crate::text::TextSnapshot>),
    Disk(PathBuf),
}

impl HeadingSource {
    /// Its headings while `current` holds (`None`: superseded).
    fn read(self, current: &dyn Fn() -> bool) -> Option<Result<Vec<Heading>, String>> {
        match self {
            HeadingSource::Draft(doc) => {
                if doc.len_bytes() > authoring::HEADING_BYTES { return Some(Err("it is larger than 4 MiB".into())); }
                let lines = (0..doc.line_count()).filter_map(|l| doc.line_range(l).and_then(|r| doc.range(r)));
                authoring::headings(lines, current).map(Ok)
            }
            HeadingSource::Disk(path) => {
                let markdown = path.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "md" | "markdown" | "mdown" | "mkd"));
                if !markdown { return Some(Err("it is not a Markdown file".into())); }
                match std::fs::metadata(&path) {
                    Ok(m) if !m.is_file() => return Some(Err("it is not a file".into())),
                    Ok(m) if m.len() > authoring::HEADING_BYTES as u64 => return Some(Err("it is larger than 4 MiB".into())),
                    Ok(_) => {}
                    Err(_) => return Some(Err("it was not found".into())),
                }
                let bytes = match std::fs::read(&path) {
                    Ok(bytes) => bytes,
                    Err(e) => return Some(Err(format!("it cannot be read ({e})"))),
                };
                let text = String::from_utf8_lossy(&bytes);
                authoring::headings(text.lines().map(std::borrow::Cow::Borrowed), current).map(Ok)
            }
        }
    }
}

/// The source byte (of the preview's version) a rendered byte of `doc` shows: exact for text written as it reads,
/// the start of its written form for normalized text (an entity, an escape), the block's start for generated glyphs.
fn source_of(doc: &MarkdownDocument, rendered: usize) -> Option<usize> {
    let i = doc.segments.partition_point(|s| s.text.end <= rendered);
    let segment = doc.segments.get(i).filter(|s| s.text.contains(&rendered))
        .or_else(|| doc.segments[..i].iter().rev().find(|s| s.text.end == rendered && !s.text.is_empty()));
    match segment.map(|s| (&s.origin, s.text.start)) {
        Some((Origin::Direct(source), start)) => return Some((source.start + rendered - start).min(source.end)),
        Some((Origin::Normalized(source), _)) => return Some(source.start),
        Some((Origin::Synthetic, _)) | None => {}
    }
    doc.blocks.iter().rev().find(|b| b.text.start <= rendered).or(doc.blocks.first()).map(|b| b.source.start)
}

/// The link whose text holds rendered byte `rendered`.
fn link_at(doc: &MarkdownDocument, rendered: usize) -> Option<&lenz_content::markdown::Link> {
    doc.links.iter().find(|l| l.text.start <= rendered && rendered < l.text.end)
        .or_else(|| doc.links.iter().find(|l| l.text.end == rendered && !l.text.is_empty()))
}

/// A destination that names something outside the file system: a scheme (`https:`, `mailto:`) or another host.
fn external(destination: &str) -> bool {
    destination.starts_with("//") || destination.split(['/', '#', '?']).next().is_some_and(|first| first.contains(':'))
}

impl Session {
    /// Enter in a Markdown draft: a list item's caret continues the list (renumbering the items after a new
    /// number when there is one caret) and an empty item ends it, every caret in one transaction, undone in one
    /// step. `None`: not Markdown, a selection, or no caret on a list item (a plain newline then).
    pub(super) fn markdown_newline(&mut self, newline: &str) -> Option<Effect> {
        let (tab, expected, ranges) = self.selection_ranges()?;
        let doc = self.state.view(tab)?.snapshot().clone();
        if doc.language() != Some(Language::Markdown) || ranges.iter().any(|r| !r.is_empty()) { return None; }
        let single = ranges.len() == 1;
        let mut edits = Vec::with_capacity(ranges.len());
        let mut listed = false;
        for range in &ranges {
            match authoring::list_enter(&doc, range.start, newline, single) {
                Some(list) => { listed = true; edits.extend(list); }
                None => edits.push(TextEdit { range: range.clone(), text: newline.into() }),
            }
        }
        if !listed { return None; }
        edits.sort_by_key(|e| (e.range.start, e.range.end));
        if edits.windows(2).any(|w| w[0].range.end > w[1].range.start) { return None; }
        Some(self.edit(tab, expected, edits, EditKind::Replace))
    }

    /// The root links of `file` resolve under, with the file's folder relative to it: the open folder when the file
    /// is inside it, else the file's own folder.
    fn link_base(&self, file: &Path) -> Option<(PathBuf, String)> {
        let dir = file.parent()?;
        match self.state.project.as_ref().filter(|p| file.starts_with(p.root())) {
            Some(p) => Some((p.root().to_path_buf(), p.relative(dir)?.to_string())),
            None => Some((dir.to_path_buf(), String::new())),
        }
    }

    /// Where a link's headings come from: an open draft of `path` (unsaved headings included), else its file.
    fn heading_source(&self, path: &Path) -> HeadingSource {
        let buffer = self.state.sources.aliases.get(path).copied()
            .or_else(|| self.state.tabs.iter().find(|t| t.source == path || t.canonical.as_deref() == Some(path)).and_then(|t| t.buffer));
        let draft = buffer.and_then(|id| self.state.sources.buffers.get(&id)).and_then(|entry| entry.snapshot.clone());
        draft.map_or_else(|| HeadingSource::Disk(path.to_path_buf()), HeadingSource::Draft)
    }

    /// Read `source`'s headings off the UI thread for `purpose`; a later request supersedes this one.
    fn ask_headings(&mut self, source: HeadingSource, purpose: Purpose) {
        let request = self.link_seq.fetch_add(1, Ordering::Relaxed) + 1;
        let (latest, tx) = (self.link_seq.clone(), self.tx.clone());
        self.link_wait = Some(LinkWait { request, purpose });
        self.jobs.submit(Priority::Visible, move || {
            let current = || latest.load(Ordering::Relaxed) == request;
            if !current() { return; }
            if let Some(headings) = source.read(&current) && current() {
                let _ = tx.send_blocking(Completion::LinkHeadings { request, headings });
            }
        });
    }

    /// Stop waiting for a link fragment's suggestions (a late answer is then dropped).
    pub(super) fn cancel_link_suggestions(&mut self) {
        if self.link_wait.take_if(|w| matches!(w.purpose, Purpose::Suggest { .. })).is_some() {
            self.link_seq.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Completion in a Markdown draft, which Lenz answers itself: in a link's destination, the files and folders of
    /// the folder typed so far (`](`), or the headings of the file named before `#` (this draft's own, unsaved ones
    /// included, when no file is named). `None`: not a Markdown draft (a language server's completion then).
    pub(super) fn link_completion(&mut self, trigger: CompletionTrigger) -> Option<Effect> {
        let tab = self.state.active?;
        let snapshot = self.state.view(tab)?.snapshot().clone();
        if snapshot.language() != Some(Language::Markdown) || snapshot.is_large() { return None; }
        let manual = trigger == CompletionTrigger::Manual;
        let say = |s: &mut Self, why: String| {
            if !manual { return Effect::default(); }
            s.state.messages.push(format!("Code completion: {why}"));
            Effect { repaint: true, ..Effect::default() }
        };
        if manual && let Some(why) = self.disabled(CommandId::Completion) { return Some(say(self, why.into())); }
        if !manual && self.text_action_disabled(CommandId::Completion).is_some() { return Some(Effect::default()); }
        self.cancel_completion();
        let open = self.state.completion.take().is_some();
        let Some(at) = self.state.selection().filter(|s| s.anchor == s.head) else { return Some(Effect { repaint: open, ..Effect::default() }) };
        let caret = at.head;
        let line = snapshot.line_range(snapshot.line_of(caret)).map_or(caret, |r| r.start);
        let before = snapshot.range(line..caret).unwrap_or_default().into_owned();
        let Some(destination) = authoring::destination(&before) else {
            let mut effect = say(self, "in Markdown, Lenz suggests link destinations: files after `](`, headings after `#`".into());
            effect.repaint |= open;
            return Some(effect);
        };
        let word = line + destination.word();
        let Some((root, dir)) = self.link_base(snapshot.source()) else { return Some(Effect::default()) };
        if destination.fragment.is_none() {
            let typed = destination.path.get(..destination.path.rfind('/').map_or(0, |i| i + 1)).unwrap_or_default();
            let Some(folder) = authoring::resolve(&dir, typed) else { return Some(say(self, "that folder is outside the open folder".into())) };
            let Some(project) = self.state.project.as_ref().filter(|p| p.root() == root) else {
                return Some(say(self, "files are suggested inside an open folder".into()));
            };
            let items: Vec<Suggestion> = project.children(&folder).into_iter().map(|(name, folder)| {
                let text = if folder { format!("{}/", authoring::encode(&name)) } else { authoring::encode(&name) };
                let label = if folder { format!("{name}/") } else { name.to_string() };
                Suggestion::local(label, Some(if folder { "folder" } else { "file" }.into()), &text, word..caret, &text, folder)
            }).collect();
            return Some(match Suggestions::local(tab, trigger, &snapshot, (word, caret), items) {
                Some(popup) => { self.state.completion = Some(popup); Effect { repaint: true, ..Effect::default() } }
                None => say(self, if folder.is_empty() { "no files match".into() } else { format!("no files in {folder}/ match") }),
            });
        }
        let (source, file) = if destination.path.is_empty() {
            (HeadingSource::Draft(snapshot.clone()), "this file".to_owned())
        } else {
            let Some(rel) = authoring::resolve(&dir, destination.path) else { return Some(say(self, "that file is outside the open folder".into())) };
            (self.heading_source(&root.join(&rel)), rel)
        };
        self.ask_headings(source, Purpose::Suggest { tab, source: SemanticSource::new(&snapshot), caret, word, trigger, file });
        Some(Effect { repaint: open, ..Effect::default() })
    }

    /// A file's headings arrived: suggested while the caret, version and tab they were asked for still are, or
    /// followed to.
    pub(super) fn link_headings(&mut self, request: u64, headings: Result<Vec<Heading>, String>) -> Effect {
        let Some(wait) = self.link_wait.take_if(|w| w.request == request) else { return Effect::default() };
        let repaint = Effect { repaint: true, ..Effect::default() };
        match wait.purpose {
            Purpose::Suggest { tab, source, caret, word, trigger, file } => {
                let current = self.state.active == Some(tab) && self.state.view(tab).is_some_and(|v| source.matches(v.snapshot()))
                    && self.state.selection().is_some_and(|s| s.anchor == caret && s.head == caret);
                if !current { return Effect::default(); }
                let Some(snapshot) = self.state.view(tab).map(|v| v.snapshot().clone()) else { return Effect::default() };
                let quiet = trigger != CompletionTrigger::Manual;
                let headings = match headings {
                    Ok(headings) => headings,
                    Err(_) if quiet => return Effect::default(),
                    Err(why) => { self.state.messages.push(format!("Code completion: no headings, {why}")); return repaint; }
                };
                let items = headings.into_iter().map(|h| {
                    let detail = format!("{} {}", "#".repeat(h.level as usize), h.text);
                    Suggestion::local(h.anchor.clone(), Some(detail), &h.anchor, word..caret, &h.anchor, false)
                }).collect();
                match Suggestions::local(tab, trigger, &snapshot, (word, caret), items) {
                    Some(popup) => { self.state.completion = Some(popup); repaint }
                    None if quiet => Effect::default(),
                    None => { self.state.messages.push(format!("Code completion: no headings of {file} match")); repaint }
                }
            }
            Purpose::Follow { tab, source, path, fragment } => {
                // Focus moved on to another file meanwhile: that navigation wins.
                let buffer = self.state.tabs.iter().find(|t| t.id == tab).and_then(|t| t.buffer);
                let here = self.state.active == Some(tab) || (path.is_none() && self.state.active().is_some_and(|t| t.buffer.is_some() && t.buffer == buffer));
                if !here { return Effect::default(); }
                let wanted = authoring::decode(&fragment).unwrap_or(fragment).to_lowercase();
                let found = headings.as_ref().ok().and_then(|h| h.iter().find(|h| h.anchor == wanted)).map(|h| h.line);
                match (path, found) {
                    (None, Some(line)) => {
                        let Some(view) = self.state.view(tab) else { return Effect::default() };
                        if !source.matches(view.snapshot()) {
                            self.state.messages.push("The draft changed while its headings were read; follow the link again".into());
                            return repaint;
                        }
                        let byte = view.snapshot().reader_line_range(line).start;
                        self.reveal_in_source(tab, byte)
                    }
                    (None, None) => { self.state.messages.push(format!("No heading “{wanted}” in this file")); repaint }
                    (Some(path), Some(line)) => self.command(Command::OpenAt { path, at: lenz_content::goto::LineColumn { line: line as u32 + 1, column: None } }),
                    (Some(path), None) => {
                        let effect = self.command(Command::Open(path));
                        self.state.messages.push(match headings {
                            Err(why) => format!("Opened without its heading “{wanted}”: {why}"),
                            Ok(_) => format!("Opened; it has no heading “{wanted}”"),
                        });
                        Effect { repaint: true, ..effect }
                    }
                }
            }
        }
    }

    /// Why a Markdown preview command cannot run now.
    pub(super) fn markdown_action_disabled(&self, id: CommandId) -> Option<&'static str> {
        if self.state.comparison.is_some() { return Some("Not in a saved comparison"); }
        let Some(tab) = self.state.active() else { return Some("No file is open") };
        let Some(view) = self.state.view(tab.id) else { return Some("No loaded file is open") };
        let Some(rendering) = view.rendering() else { return Some("Not a Markdown file") };
        match (id, rendering) {
            (CommandId::PreviewBeside, Rendering::Source) if view.snapshot().is_large() => Some("Large file: no preview"),
            (CommandId::PreviewBeside, Rendering::Source) => {
                let beside = self.state.other.and_then(|o| self.state.tabs.iter().find(|t| t.id == o)).is_some_and(|t| t.buffer == tab.buffer && !t.markdown_source);
                beside.then_some("The other pane shows its preview already")
            }
            (CommandId::PreviewBeside, _) => Some("This pane shows the preview; Show in Source opens its source"),
            (CommandId::ShowInSource, Rendering::Preview(_)) => None,
            (CommandId::ShowInSource | CommandId::FollowLink, Rendering::Pending) => Some("The preview is being prepared"),
            (CommandId::ShowInSource | CommandId::FollowLink, Rendering::Source) => Some("Only a preview is shown in its source"),
            (CommandId::FollowLink, Rendering::Preview(p)) => {
                let point = self.preview_point(tab.id, p);
                (!point.is_some_and(|r| link_at(&p.doc, r).is_some())).then_some("Click a link in the preview first")
            }
            _ => None,
        }
    }

    /// The rendered byte last chosen in `tab`'s preview `preview` (one chosen in another preview is not used).
    fn preview_point(&self, tab: TabId, preview: &crate::markdown::Preview) -> Option<usize> {
        self.state.preview_point.filter(|p| p.tab == tab && p.version == preview.version).map(|p| p.rendered.min(preview.doc.text.len()))
    }

    /// The active Markdown source's preview in the other pane (the panel split side by side first), where it follows
    /// the draft; the source pane keeps focus.
    pub(super) fn preview_beside(&mut self) -> Effect {
        if self.markdown_action_disabled(CommandId::PreviewBeside).is_some() { return Effect::default(); }
        let Some((source, buffer)) = self.state.active().map(|t| (t.id, t.buffer)) else { return Effect::default() };
        let pane = self.state.focused;
        let preview = if self.state.settings.split.is_none() {
            self.command(Command::Split(Split::Columns));
            self.state.active.filter(|id| *id != source)
        } else {
            self.focus_pane(1 - pane);
            let id = match self.state.tabs.iter().find(|t| t.pane == 1 - pane && t.buffer == buffer).map(|t| t.id) {
                Some(id) => id,
                None => {
                    self.next_tab += 1;
                    let Some(copy) = self.state.tabs.iter().find(|t| t.id == source).map(|t| t.duplicate(TabId(self.next_tab), 1 - pane)) else { return Effect::default() };
                    let id = copy.id;
                    self.state.tabs.push(copy);
                    id
                }
            };
            self.activate(id);
            Some(id)
        };
        if let Some(t) = preview.and_then(|id| self.tab_mut(id)) {
            t.markdown_source = false;
            t.preview = None;
            t.held = None;
            let id = t.id;
            self.renders.fetch_add(1, Ordering::Relaxed);
            self.render(id);
        }
        self.focus_pane(pane);
        Effect { repaint: true, ..Effect::default() }
    }

    /// Show the preview's chosen position (else its reading line) in its source, with the caret there.
    pub(super) fn show_in_source(&mut self) -> Effect {
        if self.markdown_action_disabled(CommandId::ShowInSource).is_some() { return Effect::default(); }
        let Some((tab, byte)) = self.state.active().and_then(|t| {
            let view = self.state.view(t.id)?;
            let Some(Rendering::Preview(p)) = view.rendering() else { return None };
            let chosen = self.preview_point(t.id, p).and_then(|r| source_of(&p.doc, r)).map(|b| view.preview_byte(b));
            Some((t.id, chosen.unwrap_or_else(|| view.snapshot().reader_line_range(t.top_line.min(view.snapshot().line_count() - 1)).start)))
        }) else { return Effect::default() };
        self.reveal_in_source(tab, byte)
    }

    /// Ctrl+click in a preview: follow a link there, else show the position in the source.
    pub(super) fn activate_preview(&mut self, point: PreviewPoint) -> Effect {
        self.state.preview_point = Some(point);
        if self.markdown_action_disabled(CommandId::FollowLink).is_none() { self.follow_link() } else { self.show_in_source() }
    }

    /// Follow the link at the preview's chosen position: `#heading` in this draft, a project file (at its heading),
    /// never a web link.
    pub(super) fn follow_link(&mut self) -> Effect {
        if self.markdown_action_disabled(CommandId::FollowLink).is_some() { return Effect::default(); }
        let repaint = Effect { repaint: true, ..Effect::default() };
        let Some((tab, snapshot, destination)) = self.state.active().and_then(|t| {
            let view = self.state.view(t.id)?;
            let Some(Rendering::Preview(p)) = view.rendering() else { return None };
            let link = link_at(&p.doc, self.preview_point(t.id, p)?)?;
            Some((t.id, view.snapshot().clone(), link.destination.to_string()))
        }) else { return Effect::default() };
        if external(&destination) {
            self.state.messages.push(format!("Lenz does not open web links: {destination}"));
            return repaint;
        }
        let (path, fragment) = match destination.split_once('#') {
            Some((path, fragment)) => (path, Some(fragment.to_owned())),
            None => (destination.as_str(), None),
        };
        let path = path.split('?').next().unwrap_or_default();
        if path.is_empty() {
            let Some(fragment) = fragment else { return Effect::default() };
            self.ask_headings(HeadingSource::Draft(snapshot.clone()), Purpose::Follow { tab, source: SemanticSource::new(&snapshot), path: None, fragment });
            return repaint;
        }
        let Some((root, dir)) = self.link_base(snapshot.source()) else { return Effect::default() };
        let Some(rel) = authoring::resolve(&dir, path) else {
            self.state.messages.push(format!("Link target outside the open folder: {path}"));
            return repaint;
        };
        let mut target = root.join(&rel);
        // A wiki link names a note without its extension.
        if !target.is_file() && target.extension().is_none() && target.with_extension("md").is_file() { target = target.with_extension("md"); }
        if !target.is_file() {
            self.state.messages.push(format!("Link target not found: {rel}"));
            return repaint;
        }
        match fragment {
            None => self.command(Command::Open(target)),
            Some(fragment) if target == snapshot.source() => {
                self.ask_headings(HeadingSource::Draft(snapshot.clone()), Purpose::Follow { tab, source: SemanticSource::new(&snapshot), path: None, fragment });
                repaint
            }
            Some(fragment) => {
                let source = self.heading_source(&target);
                self.ask_headings(source, Purpose::Follow { tab, source: SemanticSource::new(&snapshot), path: Some(target), fragment });
                repaint
            }
        }
    }

    /// Put the caret at `byte` of the current source of preview tab `tab`: in the other pane when it shows that
    /// source (focusing it), else by switching this pane to the source. A history step returns.
    fn reveal_in_source(&mut self, tab: TabId, byte: usize) -> Effect {
        let from = self.here();
        let buffer = self.state.tabs.iter().find(|t| t.id == tab).and_then(|t| t.buffer);
        let source = |t: &Tab| t.buffer.is_some() && t.buffer == buffer && t.markdown_source && t.loaded;
        let beside = self.state.other.and_then(|o| self.state.tabs.iter().find(|t| t.id == o)).is_some_and(source);
        if self.state.active != Some(tab) {
            // The source of that draft took focus meanwhile: the caret goes there; anywhere else, nowhere.
            if !self.state.active().is_some_and(source) { return Effect::default(); }
        } else if beside {
            self.focus_pane(1 - self.state.focused);
        } else if let Some(t) = self.tab_mut(tab) {
            t.markdown_source = true;
            t.preview = None;
            t.held = None;
            let source = t.source.clone();
            self.previewed.remove(&source);
            self.renders.fetch_add(1, Ordering::Relaxed);
        }
        let Some(doc) = self.state.current() else { return Effect::default() };
        let byte = doc.floor_char_boundary(byte.min(doc.len_bytes()));
        let line = doc.line_of(byte);
        let Some(t) = self.active_mut() else { return Effect::default() };
        t.place_selection(Some(Selection { anchor: byte, head: byte }));
        t.expansion = None;
        t.reveal(line);
        let id = t.id;
        self.bring_into_view(id, line, byte - doc.reader_line_range(line).start);
        if let Some(to) = self.here() { self.visit(from, to); }
        Effect { repaint: true, ..Effect::default() }
    }
}
