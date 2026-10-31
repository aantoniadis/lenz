//! Terminal presentation. Coordinates here are cells; every source hit keeps its UTF-8 byte.
mod forms;
mod activity;
mod terminal;
mod details;
use crate::app::{App, Focus, Prompt};
use lenz_content::find::FindOption;
use lenz_content::icons::Icon;
use lenz_content::{markdown::BlockKind, theme::Palette};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};
use code_core::{
    navigation::{Kind, Scope},
    session::{Command, CommandId, MenuEntry, Rendering, Split, StartItem, State, View},
    tabs::{Tab, TabId},
    text::SnapshotKey,
};
use std::{ops::Range, sync::Arc};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Default)]
pub(crate) struct Geometry {
    pub controls: Vec<(CommandId, Rect)>,
    /// The toolbar's application menu button.
    pub app_menu: Rect,
    pub command_menus: Vec<Rect>,
    pub tree: Rect,
    /// The file panes' area: split by the shared layout's share, dragged at the panes' boundary.
    pub reading: Rect,
    pub results: Rect,
    pub activity: Rect,
    pub terminal: crate::terminal::Geometry,
    pub panes: Vec<PaneGeometry>,
    pub modal: Rect,
    pub modal_rows: Rect,
    pub modal_content_rows: usize,
    pub modal_items: Vec<usize>,
    /// The column between side-by-side panes (empty when not split so).
    pub divider: Rect,
    pub form_buttons: Option<[Rect; 2]>,
    pub form_checkbox: Rect,
    pub details_copy: Rect,
    /// The commit popup's Show Diff button.
    pub details_diff: Rect,
    /// The checkout-problem dialog's buttons, in the core's order.
    pub details_choices: Vec<Rect>,
    pub details_close: Rect,
    /// The completion popup's rows, and the index of its first shown suggestion.
    pub completion: Rect,
    pub completion_first: usize,
    /// The browsed suggestion's documentation and the parameter information, as drawn.
    pub completion_doc: Rect,
    pub completion_doc_top: usize,
    pub completion_doc_rows: usize,
    completion_doc_layout: Option<Arc<MarkdownLayout>>,
    pub signature: Rect,
    /// The quick-fix list's rows, and the index of its first shown action.
    pub code_actions: Rect,
    pub code_actions_first: usize,
    pub navigation_scopes: Vec<(Scope, Rect)>,
    pub navigation_revision: Option<u64>,
    /// Project search: its text and file-pattern fields, and each option's toggle.
    pub search_fields: Vec<Rect>,
    pub search_toggles: Vec<(CommandId, Rect)>,
}

impl Geometry {
    pub(crate) fn invalidate_source(&mut self) {
        for pane in &mut self.panes {
            pane.source = None;
        }
    }
}

#[derive(Default)]
pub(crate) struct PaneGeometry {
    pub csv: Option<crate::app::csv::Geometry>,
    pub controls: Vec<(CommandId, Rect)>,
    pub pane: u8,
    pub preview: bool,
    pub preview_top: usize,
    pub preview_total: usize,
    pub preview_source_lines: std::sync::Arc<[usize]>,
    markdown: Option<std::sync::Arc<MarkdownLayout>>,
    pub tabs: Vec<(TabId, Rect)>,
    pub tab_bar: Rect,
    pub crumbs: Vec<(usize, Rect)>,
    /// The start screen's rows, by their index in the core's start items.
    pub start: Vec<(usize, Rect)>,
    pub text: Rect,
    /// Where the gutter starts, and the blame column's width there (0: none).
    pub gutter_x: u16,
    pub ann: u16,
    pub rows: Vec<(u16, usize, usize)>,
    pub hits: Vec<TextHit>,
    source: Option<SourceIdentity>,
}

impl PaneGeometry {
    pub(crate) fn source_view<'a>(&self, app: &'a App) -> Option<View<'a>> {
        let state = app.session.state();
        let tab = state.shown(self.pane)?;
        let view = state.view(tab.id())?;
        self.source.as_ref().filter(|held| held.matches(&view, app))?;
        Some(view)
    }
}

struct SourceIdentity {
    tab: TabId,
    snapshot: SnapshotKey,
    metadata: Arc<code_core::document::SourceMeta>,
    top: (usize, usize),
    column: usize,
    cols: Option<usize>,
    size: Option<(usize, usize)>,
    visible: Arc<code_core::folds::Visible>,
    breadcrumbs: bool,
    split: Option<Split>,
    explorer: bool,
    results: bool,
    terminal: bool,
    rendering: SourceRendering,
    image_source: bool,
    hints: Option<Arc<code_core::decor::Hints>>,
    declarations: Option<Arc<lenz_content::highlight::Declarations>>,
    sticky: bool,
    annotation_cols: usize,
}

fn same_arc<T>(held: Option<&Arc<T>>, current: Option<&Arc<T>>) -> bool {
    match (held, current) {
        (None, None) => true,
        (Some(held), Some(current)) => Arc::ptr_eq(held, current),
        _ => false,
    }
}

enum SourceRendering {
    Text,
    Pending,
    Preview(Arc<code_core::markdown::Preview>),
}

impl SourceRendering {
    fn new(view: &View<'_>) -> Self {
        match view.rendering() {
            Some(Rendering::Preview(preview)) => Self::Preview(preview.clone()),
            Some(Rendering::Pending) => Self::Pending,
            _ => Self::Text,
        }
    }
    fn matches(&self, view: &View<'_>) -> bool {
        match (self, view.rendering()) {
            (Self::Text, None | Some(Rendering::Source)) | (Self::Pending, Some(Rendering::Pending)) => true,
            (Self::Preview(held), Some(Rendering::Preview(current))) => Arc::ptr_eq(held, current),
            _ => false,
        }
    }
}

impl SourceIdentity {
    fn new(view: &View<'_>, app: &App) -> Self {
        let state = app.session.state();
        let tab = view.tab();
        Self {
            tab: tab.id(), snapshot: view.snapshot().key(),
            metadata: view.snapshot().source_meta().clone(),
            top: (tab.top_line(), tab.top_row()), column: tab.column(), cols: state.wrap_cols(tab),
            size: state.text_size(tab.pane()), visible: view.visible(),
            breadcrumbs: state.breadcrumbs(), split: state.split(),
            explorer: app.tree_shown && state.has_tree(), results: state.results().is_some(),
            terminal: state.terminal().shown(),
            rendering: SourceRendering::new(view), image_source: tab.image_source(),
            hints: view.inlay_hints().cloned(), declarations: view.declarations_handle().cloned(),
            sticky: state.sticky_scroll(), annotation_cols: state.annotation_cols(tab),
        }
    }
    fn matches(&self, view: &View<'_>, app: &App) -> bool {
        let state = app.session.state();
        let tab = view.tab();
        app.session.native_reveal(tab.pane()).is_none()
            && self.tab == tab.id() && self.snapshot == view.snapshot().key()
            && Arc::ptr_eq(&self.metadata, view.snapshot().source_meta())
            && self.top == (tab.top_line(), tab.top_row()) && self.column == tab.column()
            && self.cols == state.wrap_cols(tab) && self.size == state.text_size(tab.pane())
            && Arc::ptr_eq(&self.visible, &view.visible())
            && self.breadcrumbs == state.breadcrumbs() && self.split == state.split()
            && self.explorer == (app.tree_shown && state.has_tree())
            && self.results == state.results().is_some()
            && self.terminal == state.terminal().shown()
            && self.rendering.matches(view) && self.image_source == tab.image_source()
            && same_arc(self.hints.as_ref(), view.inlay_hints())
            && same_arc(self.declarations.as_ref(), view.declarations_handle())
            && self.sticky == state.sticky_scroll() && self.annotation_cols == state.annotation_cols(tab)
    }
}

/// Byte `at` of `tab`'s shown text as its line and the byte within that line, as text hits record them.
fn line_byte(state: &code_core::session::State, tab: code_core::tabs::TabId, at: usize) -> Option<(usize, usize)> {
    let snapshot = state.view(tab)?.snapshot().clone();
    let line = snapshot.line_of(at);
    Some((line, at - snapshot.line_range(line)?.start))
}

/// Underline the cells a diagnostic of the version shown covers (an empty range marks the cell it starts at), errors
/// and warnings in their colour; the status line names the one at the caret.
fn mark_problems(buf: &mut Buffer, state: &code_core::session::State, tab: code_core::tabs::TabId, hits: &[TextHit], p: &Palette) {
    use code_core::semantics::diagnostics::Severity;
    let Some(found) = state.diagnostics(tab).filter(|f| !f.items.is_empty()) else { return };
    let Some(snapshot) = state.view(tab).map(|v| v.snapshot().clone()) else { return };
    // Hits name a line and a byte within it; the problems are document bytes.
    let at = |h: &TextHit| snapshot.line_range(h.line).map(|r| r.start + h.byte);
    let (Some(low), Some(high)) = (hits.iter().filter_map(at).min(), hits.iter().filter_map(at).max()) else { return };
    let near: Vec<_> = found.items.iter().filter(|d| d.range.start <= high && d.range.end >= low).collect();
    if near.is_empty() { return; }
    for hit in hits {
        let Some(byte) = at(hit) else { continue };
        let covers = |d: &&&code_core::semantics::diagnostics::Diagnostic| d.range.start <= byte && (byte < d.range.end || d.range.start == d.range.end && byte == d.range.start);
        let Some(d) = near.iter().filter(covers).min_by_key(|d| d.severity) else { continue };
        let width = if d.range.is_empty() { 1 } else { hit.width.max(1) };
        for x in hit.x..hit.x.saturating_add(width) {
            let Some(cell) = buf.cell_mut((x, hit.y)) else { continue };
            let mut style = cell.style().add_modifier(Modifier::UNDERLINED);
            match d.severity {
                Severity::Error => style = style.fg(color(p.error)),
                Severity::Warning => style = style.fg(color(p.accent)),
                Severity::Information | Severity::Hint => {}
            }
            cell.set_style(style);
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TextHit {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub line: usize,
    pub byte: usize,
}

/// The terminal About logo as half-block cells (two pixel rows per cell), blended over `bg`.
fn draw_about_logo(buf: &mut Buffer, area: Rect, bg: u32) {
    let size = lenz_content::about::LOGO_TERMINAL_SIZE;
    let pixel = |x: usize, y: usize| {
        let i = (y * size + x) * 4;
        let [r, g, b, a] = [0, 1, 2, 3].map(|k| u32::from(lenz_content::about::LOGO_TERMINAL[i + k]));
        let mix = |c: u32, shift: u32| (c * a + ((bg >> shift) & 0xFF) * (255 - a)) / 255;
        Color::Rgb(mix(r, 16) as u8, mix(g, 8) as u8, mix(b, 0) as u8)
    };
    for row in 0..area.height.min((size / 2) as u16) {
        for col in 0..area.width.min(size as u16) {
            let (x, y) = (col as usize, row as usize * 2);
            if let Some(cell) = buf.cell_mut((area.x + col, area.y + row)) {
                cell.set_symbol("▀").set_fg(pixel(x, y)).set_bg(pixel(x, y + 1));
            }
        }
    }
}

pub(crate) fn color(rgb: u32) -> Color {
    Color::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}
fn style(fg: u32, bg: u32) -> Style {
    Style::default().fg(color(fg)).bg(color(bg))
}
fn line(buf: &mut Buffer, area: Rect, text: &str, st: Style) {
    if area.height > 0 {
        buf.set_stringn(area.x, area.y, text, area.width as usize, st);
    }
}
fn row(area: Rect, offset: u16) -> Rect {
    Rect::new(
        area.x,
        area.y.saturating_add(offset),
        area.width,
        u16::from(offset < area.height),
    )
}
fn fill(buf: &mut Buffer, area: Rect, st: Style) {
    buf.set_style(area, st);
}
fn inner(area: Rect) -> Rect {
    Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    )
}
/// [`clamp`] for a list the wheel scrolls on its own: `at` is scrolled into view only when it moved since `seen`.
fn clamp_moved(at: &mut usize, top: &mut usize, seen: &mut usize, len: usize, height: usize) {
    let moved = (*at).min(len.saturating_sub(1)) != *seen;
    if moved {
        clamp(at, top, len, height);
    } else {
        *at = (*at).min(len.saturating_sub(1));
        *top = (*top).min(len.saturating_sub(height));
    }
    *seen = *at;
}

fn clamp(at: &mut usize, top: &mut usize, len: usize, height: usize) {
    *at = (*at).min(len.saturating_sub(1));
    *top = (*top).min(len.saturating_sub(height));
    if *at < *top {
        *top = *at;
    }
    if height > 0 && *at >= top.saturating_add(height) {
        *top = *at + 1 - height;
    }
}

/// The most suggestions the completion popup shows at once (Page Up/Down moves by this many).
pub(crate) const COMPLETION_ROWS: usize = 8;

impl App {
    /// The completion popup: under the name being completed (above it without room), clamped to the terminal,
    /// scrolled to the selection. Unavailable suggestions are dim.
    fn draw_completion(&self, frame: &mut ratatui::Frame, geometry: &mut Geometry, p: &Palette) {
        let state = self.session.state();
        let Some(c) = state.completion() else { return };
        let area = frame.area();
        let Some(pane) = geometry.panes.iter().find(|g| state.shown(g.pane).is_some_and(|t| t.id() == c.tab)) else { return };
        let Some((at_line, word)) = line_byte(state, c.tab, c.word()) else { return };
        let at_line = Some(at_line);
        let anchor = pane.hits.iter().filter(|h| Some(h.line) == at_line && h.byte <= word)
            .max_by_key(|h| h.byte).map(|h| if h.byte == word { (h.x, h.y) } else { (h.x + h.width, h.y) });
        let Some((x, y)) = anchor.or_else(|| pane.hits.iter().find(|h| Some(h.line) == at_line).map(|h| (h.x, h.y))) else { return };
        let n = c.rows().len();
        let below = area.bottom().saturating_sub(y + 1);
        let room = below.max(y.saturating_sub(area.y)) as usize;
        let room=if c.documentation_preview().is_some() && area.width<72 {room.min((area.height as usize/2).max(1))}else{room};
        let shown = n.min(COMPLETION_ROWS).min(room);
        if shown == 0 || area.width == 0 { return; }
        let first = c.selected().saturating_sub(shown - 1).min(n - shown);
        let label = |r: &code_core::session::Suggestion| match &r.detail { Some(d) => format!(" {}  {d} ", r.label), None => format!(" {} ", r.label) };
        let width = c.rows().skip(first).take(shown).map(|r| label(r).width()+r.kind.map_or(0,str::len)+2).max().unwrap_or(1).min(area.width as usize).min(48) as u16;
        let top = if below as usize >= shown { y + 1 } else { y - shown as u16 };
        let left = x.min(area.right().saturating_sub(width));
        let stack=c.documentation_preview().is_some() && area.right().saturating_sub(left+width)<12 && left.saturating_sub(area.x)<12;
        let top=if stack {top.min(area.bottom().saturating_sub(shown as u16+(area.height/2).max(1)))}else{top};
        let popup = Rect::new(left, top, width, shown as u16);
        frame.render_widget(Clear, popup);
        for (i, r) in c.rows().enumerate().skip(first).take(shown) {
            let selected = i == c.selected();
            let bg = if selected { p.selected_bg } else { p.status_bg };
            let at = row(popup, (i - first) as u16);
            fill(frame.buffer_mut(), at, style(p.fg, bg));
            let group = i > 0 && c.rows().nth(i-1).is_some_and(|previous|previous.kind != r.kind);
            let kind = r.kind.unwrap_or("");
            let kind_width = (kind.width() as u16+1).min(at.width/2);
            let label_area = Rect::new(at.x+1,at.y,at.width.saturating_sub(kind_width+1),1);
            if selected { line(frame.buffer_mut(),Rect::new(at.x,at.y,1,1),"›",style(p.accent,bg).add_modifier(Modifier::BOLD)); }
            let matches = c.matches(&r.label);
            let mut spans = Vec::new(); let mut offset=0;
            for matched in matches {
                if offset < matched.start { spans.push(ratatui::text::Span::styled(r.label.get(offset..matched.start).unwrap_or_default(),style(if r.unavailable.is_some(){p.dim}else{p.fg},bg))); }
                spans.push(ratatui::text::Span::styled(r.label.get(matched.clone()).unwrap_or_default(),style(p.accent,bg).add_modifier(Modifier::BOLD|Modifier::UNDERLINED))); offset=matched.end;
            }
            if offset < r.label.len() { spans.push(ratatui::text::Span::styled(r.label.get(offset..).unwrap_or_default(),style(if r.unavailable.is_some(){p.dim}else{p.fg},bg))); }
            if let Some(detail)=&r.detail { spans.push(ratatui::text::Span::styled(format!("  {detail}"),style(p.dim,bg))); }
            frame.render_widget(ratatui::widgets::Paragraph::new(ratatui::text::Line::from(spans)),label_area);
            if kind_width>0 { line(frame.buffer_mut(),Rect::new(at.right()-kind_width,at.y,kind_width,1),kind,style(p.accent,bg).add_modifier(if group {Modifier::UNDERLINED}else{Modifier::empty()})); }
        }
        geometry.completion = popup;
        geometry.completion_first = first;
        let Some(preview) = c.documentation_preview() else { return };
        let right = area.right().saturating_sub(popup.right());
        let left = popup.x.saturating_sub(area.x);
        let (dx, cols, stacked) = if right >= 12 { (popup.right(), right.min(60), false) } else if left >= 12 { (popup.x-left.min(60),left.min(60),false) } else { (area.x,area.width,true) };
        if cols==0 {return;}
        let Some(view)=state.view(c.tab) else{return};
        let layout = self.layout.completion_doc_layout.as_ref().filter(|l|Arc::ptr_eq(&l.preview,preview)&&l.width==cols).cloned()
            .unwrap_or_else(||Arc::new(MarkdownLayout::new(preview.clone(),cols,view.snapshot())));
        let height = layout.rows.len().min((area.height as usize/2).max(1)) as u16;
        let top = if self.layout.completion_doc_layout.as_ref().is_some_and(|l|Arc::ptr_eq(&l.preview,preview)) {self.layout.completion_doc_top}else{0};
        let top = top.min(layout.rows.len().saturating_sub(height as usize));
        let dy = if stacked { if popup.bottom().saturating_add(height)<=area.bottom(){popup.bottom()}else{popup.y.saturating_sub(height)} }else{popup.y.min(area.bottom().saturating_sub(height))};
        let rect=Rect::new(dx,dy,cols,height);
        frame.render_widget(Clear,rect); fill(frame.buffer_mut(),rect,style(p.fg,p.bg));
        let mut doc_geometry=PaneGeometry{text:rect,preview_top:top,..Default::default()};
        draw_markdown(frame.buffer_mut(),&mut doc_geometry,None,&layout,p);
        geometry.completion_doc=rect; geometry.completion_doc_top=top; geometry.completion_doc_rows=layout.rows.len(); geometry.completion_doc_layout=Some(layout);

    }

    /// The quick fixes: under the caret's row (above it without room), scrolled to the selection; actions that are
    /// not applied are dim, and the selected one's reason shows on a last row.
    fn draw_code_actions(&self, frame: &mut ratatui::Frame, geometry: &mut Geometry, p: &Palette) {
        let state = self.session.state();
        let Some(c) = state.code_actions() else { return };
        let area = frame.area();
        let Some(pane) = geometry.panes.iter().find(|g| state.shown(g.pane).is_some_and(|t| t.id() == c.tab)) else { return };
        let Some((at_line, caret)) = line_byte(state, c.tab, c.caret()) else { return };
        let at_line = Some(at_line);
        let Some((x, y)) = pane.hits.iter().filter(|h| Some(h.line) == at_line && h.byte <= caret).max_by_key(|h| h.byte).map(|h| (h.x, h.y))
            .or_else(|| pane.hits.iter().find(|h| Some(h.line) == at_line).map(|h| (h.x, h.y))) else { return };
        let reason = c.actions.get(c.selected).and_then(|a| a.unavailable.as_deref());
        let n = c.actions.len();
        let below = area.bottom().saturating_sub(y + 1);
        let room = (below.max(y.saturating_sub(area.y)) as usize).saturating_sub(usize::from(reason.is_some()));
        let shown = n.min(COMPLETION_ROWS).min(room);
        if shown == 0 || area.width == 0 { return; }
        let first = c.selected.saturating_sub(shown - 1).min(n - shown);
        let label = |a: &code_core::session::CodeAction| format!(" {}{} ", if a.preferred { "★ " } else { "" }, a.title);
        let width = (c.actions.iter().skip(first).take(shown).map(|a| label(a).width()).chain(reason.map(|r| r.width() + 2)).max().unwrap_or(1) as u16).min(area.width).min(72);
        let height = shown as u16 + u16::from(reason.is_some());
        let top = if below >= height { y + 1 } else { y.saturating_sub(height) };
        let left = x.min(area.right().saturating_sub(width));
        let popup = Rect::new(left, top, width, shown as u16);
        frame.render_widget(Clear, Rect::new(left, top, width, height));
        for (i, a) in c.actions.iter().enumerate().skip(first).take(shown) {
            let bg = if i == c.selected { p.selected_bg } else { p.status_bg };
            let at = row(popup, (i - first) as u16);
            fill(frame.buffer_mut(), at, style(p.fg, bg));
            line(frame.buffer_mut(), at, &code_core::chrome::ellipsize(&label(a), width as usize), style(if a.unavailable.is_some() { p.dim } else { p.fg }, bg));
        }
        if let Some(reason) = reason {
            let at = Rect::new(left, popup.bottom(), width, 1);
            fill(frame.buffer_mut(), at, style(p.dim, p.status_bg));
            line(frame.buffer_mut(), at, &code_core::chrome::ellipsize(&format!(" {reason}"), width as usize), style(p.dim, p.status_bg));
        }
        geometry.code_actions = popup;
        geometry.code_actions_first = first;
    }

    /// Parameter information: above the caret's row (under it without room), the active parameter marked with the
    /// selection colour, then the overload count and the documentation.
    fn draw_signature(&self, frame: &mut ratatui::Frame, geometry: &mut Geometry, p: &Palette) {
        let state = self.session.state();
        let Some(h) = state.signature_help() else { return };
        let Some(sig) = h.active_signature() else { return };
        let area = frame.area();
        let Some(pane) = geometry.panes.iter().find(|g| state.shown(g.pane).is_some_and(|t| t.id() == h.tab)) else { return };
        let Some((at_line, caret)) = line_byte(state, h.tab, h.caret()) else { return };
        let at_line = Some(at_line);
        let Some((x, y)) = pane.hits.iter().filter(|hit| Some(hit.line) == at_line && hit.byte <= caret).max_by_key(|hit| hit.byte).map(|hit| (hit.x, hit.y))
            .or_else(|| pane.hits.iter().find(|hit| Some(hit.line) == at_line).map(|hit| (hit.x, hit.y))) else { return };
        if area.width < 3 || area.height == 0 { return; }
        let cols = area.width.min(80);
        let mut lines: Vec<(String, Option<std::ops::Range<usize>>, bool)> = vec![(sig.label.clone(), sig.active_parameter.map(|i| sig.parameters[i].label.clone()).filter(|r| !r.is_empty()), false)];
        if h.signatures.len() > 1 { lines.push((format!("{} of {} signatures", h.active + 1, h.signatures.len()), None, true)); }
        if let Some(doc) = sig.active_parameter.and_then(|i| sig.parameters[i].documentation.as_deref()) { lines.extend(wrap(doc, cols.saturating_sub(2) as usize, 3).into_iter().map(|l| (l, None, false))); }
        if let Some(doc) = sig.documentation.as_deref() { lines.extend(wrap(doc, cols.saturating_sub(2) as usize, 3).into_iter().map(|l| (l, None, true))); }
        let width = (lines.iter().map(|l| l.0.width() + 2).max().unwrap_or(1) as u16).min(cols);
        let above = y.saturating_sub(area.y);
        let below = area.bottom().saturating_sub(y + 1);
        let shown = (lines.len() as u16).min(above.max(below));
        if shown == 0 { return; }
        let top = if above >= shown { y - shown } else { y + 1 };
        let rect = Rect::new(x.min(area.right().saturating_sub(width)), top, width, shown);
        frame.render_widget(Clear, rect);
        for (i, (text, active, dim)) in lines.iter().enumerate().take(shown as usize) {
            let at = row(rect, i as u16);
            fill(frame.buffer_mut(), at, style(p.fg, p.status_bg));
            line(frame.buffer_mut(), at, &format!(" {text}"), style(if *dim { p.dim } else { p.fg }, p.status_bg));
            // The active parameter: its cells take the selection colour (and keep their text), not colour alone.
            if let Some((before, param)) = active.as_ref().and_then(|r| Some((text.get(..r.start)?, text.get(r.clone())?))) {
                let start = at.x + 1 + before.width() as u16;
                let len = param.width() as u16;
                for cx in start..(start + len).min(at.right()) {
                    let cell = &mut frame.buffer_mut()[(cx, at.y)];
                    cell.set_style(style(p.fg, p.selected_bg).add_modifier(ratatui::style::Modifier::UNDERLINED));
                }
            }
        }
        geometry.signature = rect;
    }

    pub fn draw(&mut self, frame: &mut ratatui::Frame) {
        self.show_documentation();
        for pane in 0..2 { self.session.dispatch(Command::SetNativeContext { pane, context: 1 }); }
        self.source_cache.borrow_mut().begin(self.session.state());
        let area = frame.area();
        let palette = **self.session.state().palette();
        let p = &palette;
        let mut geometry = Geometry::default();
        fill(frame.buffer_mut(), area, style(p.fg, p.bg));
        if area.is_empty() {
            self.layout = geometry;
            for command in self.source_cache.borrow_mut().finish() { self.source_redraw |= self.session.dispatch(command).repaint; }
            self.sync_prompts();
            return;
        }
        self.draw_toolbar(frame, &mut geometry, p);
        let mut body = Rect::new(
            area.x,
            area.y.saturating_add(1),
            area.width,
            area.height.saturating_sub(1 + u16::from(area.height >= 2)),
        );
        self.draw_activity(frame, &mut geometry, &mut body, p);
        self.draw_terminal(frame, &mut geometry, &mut body, p);
        // A quarter of the width unless its right edge was dragged; the text keeps at least 24 columns.
        let tree_width =
            if self.tree_shown && self.session.state().has_tree() && area.width >= 36 {
                let dragged = self.explorer_drag.or_else(|| self.session.state().layout().explorer_columns.map(|w| w.min(u32::from(u16::MAX)) as u16));
                dragged.map_or((area.width / 4).clamp(16, 32), |w| w.clamp(12, area.width - 24))
            } else {
                0
            };
        geometry.tree = Rect::new(body.x, body.y, tree_width, body.height);
        let main = Rect::new(
            body.x + tree_width,
            body.y,
            body.width.saturating_sub(tree_width),
            body.height,
        );
        // A third of the height, or the share dragged or remembered (shared with the GUI); the text keeps 3 rows.
        let results_height = if self.session.state().results().is_some() && main.height >= 6 {
            match self.results_drag.or(self.session.state().layout().results_permille) {
                Some(k) => (((u32::from(main.height) * k + 500) / 1000) as u16).max(4).min(main.height.saturating_sub(3).max(4)),
                None => (main.height / 3).max(4),
            }
        } else {
            0
        };
        geometry.results = Rect::new(
            main.x,
            main.bottom().saturating_sub(results_height),
            main.width,
            results_height,
        );
        let reading = Rect::new(
            main.x,
            main.y,
            main.width,
            main.height.saturating_sub(results_height),
        );
        geometry.reading = reading;
        // The first pane's share, as dragged or remembered (shared with the GUI); each pane keeps a few cells.
        let share = self.split_drag.or(self.session.state().layout().split_permille).map_or(500, |k| k.clamp(100, 900));
        let cut = |len: u16| ((u32::from(len) * share + 500) / 1000) as u16;
        let panels = match self.session.state().split() {
            Some(Split::Columns) => {
                let half = cut(reading.width).clamp(reading.width.min(12), reading.width.saturating_sub(12).max(reading.width.min(12)));
                // The first pane's last column is the divider (and the boundary to drag).
                geometry.divider = Rect::new(reading.x + half.saturating_sub(1), reading.y, u16::from(half > 1), reading.height);
                vec![
                    Rect::new(reading.x, reading.y, half.saturating_sub(u16::from(half > 1)), reading.height),
                    Rect::new(
                        reading.x + half,
                        reading.y,
                        reading.width - half,
                        reading.height,
                    ),
                ]
            }
            Some(Split::Rows) => {
                let half = cut(reading.height).clamp(reading.height.min(4), reading.height.saturating_sub(4).max(reading.height.min(4)));
                vec![
                    Rect::new(reading.x, reading.y, reading.width, half),
                    Rect::new(
                        reading.x,
                        reading.y + half,
                        reading.width,
                        reading.height - half,
                    ),
                ]
            }
            None => vec![reading],
        };
        // Resize commands own view clamping and wrap state in the core.
        for (pane, rect) in panels.iter().copied().enumerate() {
            let text = text_area(self.session.state(), pane as u8, rect);
            if self
                .layout
                .panes
                .get(pane)
                .is_none_or(|old| (old.text.width, old.text.height) != (text.width, text.height))
            {
                self.dispatch(Command::SetTextSize {
                    pane: pane as u8,
                    cols: text.width as usize,
                    rows: text.height as usize,
                });
            }
        }
        // The explorer's last column is its edge, drawn as the split's divider is: the cell to drag.
        let edge = u16::from(geometry.tree.width >= 2);
        let listing = Rect { width: geometry.tree.width - edge, ..geometry.tree };
        self.draw_tree(frame, listing, p);
        if edge == 1 {
            for y in geometry.tree.top()..geometry.tree.bottom() {
                frame.buffer_mut()[(listing.right(), y)].set_symbol("│").set_style(style(p.border, p.bg));
            }
        }
        if listing.width >= 3 && listing.height > 0 {
            let rect = Rect::new(listing.right() - 3, listing.y, 3, 1);
            geometry
                .controls
                .push((code_core::session::CommandId::LocateFile, rect));
            line(
                frame.buffer_mut(),
                rect,
                " ◎ ",
                style(
                    if self
                        .session
                        .disabled(code_core::session::CommandId::LocateFile)
                        .is_none()
                    {
                        p.fg
                    } else {
                        p.dim
                    },
                    p.header_bg,
                ),
            );
        }
        geometry.tree.y = geometry
            .tree
            .y
            .saturating_add(u16::from(geometry.tree.height > 0));
        geometry.tree.height = geometry.tree.height.saturating_sub(1);
        for (pane, rect) in panels.into_iter().enumerate() {
            if self.session.state().focused_pane() == pane as u8 && self.session.state().shown(pane as u8).is_some_and(|tab| self.session.state().csv(tab.id()).is_some()) {
                let area = text_area(self.session.state(), pane as u8, rect);
                self.session.dispatch(Command::Csv(code_core::csv::Command::Size { rows: area.height.saturating_sub(2) as usize, columns: usize::from(area.width / 18).max(1) }));
            }
            geometry
                .panes
                .push(draw_pane(frame, self, pane as u8, rect, p));
        }
        for pane in &geometry.panes {
            geometry.controls.extend(pane.controls.iter().copied());
            if pane.preview
                && let Some(tab) = self.session.state().shown(pane.pane)
                && let Some(view) = self.session.state().view(tab.id())
            {
                self.preview_scroll.insert(tab.id(), (crate::app::PreviewSource::new(&view), pane.preview_top));
            }
        }
        for y in geometry.divider.top()..geometry.divider.bottom() {
            frame.buffer_mut()[(geometry.divider.x, y)].set_symbol("│").set_style(style(p.border, p.bg));
        }
        let results_area = geometry.results;
        self.draw_results(frame, &mut geometry, results_area, p);
        if geometry.results.width >= 64
            && self
                .session
                .state()
                .results()
                .is_some_and(|r| r.preview().is_some())
        {
            geometry.results.width /= 2;
        }
        geometry.results.y = geometry
            .results
            .y
            .saturating_add(u16::from(geometry.results.height > 0));
        geometry.results.height = geometry.results.height.saturating_sub(1);
        if area.height >= 2 {
            self.draw_status(frame, &mut geometry, p);
        }
        if self.session.state().terminal().focused() && self.modal.is_none()
            && let Some(screen) = geometry.terminal.frame.as_ref()
            && let Some((row, column)) = screen.cursor
            && row < geometry.terminal.cells.height as usize && column < geometry.terminal.cells.width as usize {
            frame.set_cursor_position((geometry.terminal.cells.x + column as u16, geometry.terminal.cells.y + row as u16));
        }
        if self.modal.is_some() {
            self.draw_modal(frame, &mut geometry, p);
        } else if let Some(doc) = self.session.state().documentation() {
            let popup = centered(
                area,
                area.width.saturating_sub(4).min(88),
                area.height.saturating_sub(4).min(24),
            );
            frame.render_widget(Clear, popup);
            frame.render_widget(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Documentation · Esc closes ")
                    .style(style(p.fg, p.status_bg)),
                popup,
            );
            frame.render_widget(
                Paragraph::new(doc.preview.doc.text.as_str())
                    .wrap(Wrap { trim: false })
                    .style(style(p.fg, p.status_bg)),
                inner(popup),
            );
        } else {
            self.draw_signature(frame, &mut geometry, p);
            self.draw_completion(frame, &mut geometry, p);
            self.draw_code_actions(frame, &mut geometry, p);
        }
        self.layout = geometry;
        for command in self.source_cache.borrow_mut().finish() { self.source_redraw |= self.session.dispatch(command).repaint; }
        self.sync_prompts();
    }

    fn draw_toolbar(&self, frame: &mut ratatui::Frame, geometry: &mut Geometry, p: &Palette) {
        let area = row(frame.area(), 0);
        fill(frame.buffer_mut(), area, style(p.fg, p.header_bg));
        let menu = " ☰ Menu ";
        let width = (menu.width() as u16).min(area.width);
        let rect = Rect::new(area.x, area.y, width, 1);
        let open = self
            .modal
            .as_ref()
            .is_some_and(|m| matches!(m.kind, Prompt::Menu(_)));
        let st = style(p.fg, if open { p.selected_bg } else { p.header_bg }).add_modifier(Modifier::BOLD);
        fill(frame.buffer_mut(), rect, st);
        line(
            frame.buffer_mut(),
            rect,
            &code_core::chrome::ellipsize(menu, width as usize),
            st,
        );
        geometry.app_menu = rect;
        let mut x = rect.right().saturating_add(1);
        for (id, label) in self.session.toolbar() {
            let label = if id == CommandId::GitBranches { self.session.branch_label().unwrap_or_else(|| label.into()) } else { label.into() };
            let room = area.right().saturating_sub(x);
            if room < 3 { break; }
            let label = code_core::chrome::ellipsize(&label, room.saturating_sub(2) as usize);
            let width = (label.width() + 2) as u16;
            if width > area.right().saturating_sub(x) {
                break;
            }
            let rect = Rect::new(x, area.y, width, 1);
            let disabled = self.session.disabled(id).is_some();
            let active = id == CommandId::ToggleExplorer && self.tree_shown;
            let bg = if active { p.selected_bg } else { p.header_bg };
            let st = style(if disabled { p.dim } else { p.fg }, bg).add_modifier(if active {
                Modifier::BOLD | Modifier::UNDERLINED
            } else {
                Modifier::empty()
            });
            fill(frame.buffer_mut(), rect, st);
            line(frame.buffer_mut(), rect, &format!(" {label} "), st);
            geometry.controls.push((id, rect));
            x = rect.right().saturating_add(1);
        }
    }

    fn draw_status(&self, frame: &mut ratatui::Frame, geometry: &mut Geometry, p: &Palette) {
        let area = row(frame.area(), frame.area().height - 1);
        fill(frame.buffer_mut(), area, style(p.fg, p.status_bg));
        let commands = if area.width >= 120 { " F10 Commands " } else { " F10 " };
        let commands_width = (commands.width() as u16).min(area.width);
        let commands_rect = Rect::new(area.right() - commands_width, area.y, commands_width, 1);
        line(
            frame.buffer_mut(),
            commands_rect,
            &code_core::chrome::ellipsize(commands, commands_width as usize),
            style(p.fg, p.status_bg).add_modifier(Modifier::UNDERLINED),
        );
        geometry.command_menus.push(commands_rect);
        let log_width = 10.min(area.width.saturating_sub(commands_width));
        let log_rect = Rect::new(commands_rect.x.saturating_sub(log_width), area.y, log_width, 1);
        line(frame.buffer_mut(), log_rect, &format!("Logs ({})", self.session.state().activity_log().len()), style(p.accent, p.status_bg));
        geometry.controls.push((CommandId::ToggleActivityLog, log_rect));
        // "Results (4)" left of the log while the results panel is hidden; a click shows it again.
        let hidden = self.session.hidden_results_label().map(|label| format!("{label} ")).unwrap_or_default();
        let results_width = (hidden.width() as u16).min(area.width.saturating_sub(commands_width + log_width) / 3);
        let results_rect = Rect::new(log_rect.x.saturating_sub(results_width), area.y, results_width, 1);
        if results_width > 0 {
            line(frame.buffer_mut(), results_rect, &code_core::chrome::ellipsize(&hidden, results_width as usize), style(p.accent, p.status_bg));
            geometry.controls.push((CommandId::ToggleResults, results_rect));
        }
        let available = area.width.saturating_sub(commands_width + log_width + results_width);
        let state = self.session.state();
        let lsp = format!(" {} [F2] ", state.language_server_label());
        let sql = self.session.sql_dialect_label().map(|label| format!(" {label} ")).unwrap_or_default();
        let save_notice = area.width < 60 && state.active().and_then(|tab| state.view(tab.id()))
            .is_some_and(|view| view.save_status().message().is_some());
        let position = match self.independent_view {
            Some((index, count)) => format!(
                " {} · View {index}/{count} ",
                self.session.status_position()
            ),
            None => format!(" {} ", self.session.status_position()),
        };
        let position_width = if save_notice { 0 } else { (position.width() as u16).min(available / 2) };
        let position_rect = Rect::new(area.x, area.y, position_width, 1);
        line(
            frame.buffer_mut(),
            position_rect,
            &code_core::chrome::ellipsize(&position, position_width as usize),
            style(p.fg, p.status_bg),
        );
        let available = available.saturating_sub(position_width);
        let lsp_width = if save_notice { 0 } else { (lsp.width() as u16).min(available / 2) };
        let lsp_rect = Rect::new(position_rect.right(), area.y, lsp_width, 1);
        let attention = state.provider().is_some_and(|v| {
            matches!(
                v.status,
                code_core::semantics::Status::Missing(_)
                    | code_core::semantics::Status::Failed(_)
                    | code_core::semantics::Status::Disabled
            )
        });
        line(
            frame.buffer_mut(),
            lsp_rect,
            &code_core::chrome::ellipsize(&lsp, lsp_width as usize),
            style(if attention { p.error } else { p.accent }, p.status_bg)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        );
        geometry
            .controls
            .push((CommandId::LanguageServer, lsp_rect));
        // The SQL dialect the file is written for, including "not set": never assumed, always visible.
        let sql_width = if save_notice { 0 } else { (sql.width() as u16).min(available.saturating_sub(lsp_width) / 2) };
        let sql_rect = Rect::new(lsp_rect.right(), area.y, sql_width, 1);
        if sql_width > 0 {
            line(
                frame.buffer_mut(),
                sql_rect,
                &code_core::chrome::ellipsize(&sql, sql_width as usize),
                style(p.accent, p.status_bg).add_modifier(Modifier::UNDERLINED),
            );
            geometry.controls.push((CommandId::SqlDialect, sql_rect));
        }
        // The open draft's change-marker baseline: which version its letters are compared against.
        let baseline = self.session.markers_baseline().map(|b| format!(" vs {} ", b.short())).unwrap_or_default();
        let baseline_width = if save_notice || area.width < 60 { 0 } else { (baseline.width() as u16).min(available.saturating_sub(lsp_width + sql_width) / 4) };
        let baseline_rect = Rect::new(sql_rect.right(), area.y, baseline_width, 1);
        if baseline_width > 0 {
            line(
                frame.buffer_mut(),
                baseline_rect,
                &code_core::chrome::ellipsize(&baseline, baseline_width as usize),
                style(p.fg, p.status_bg),
            );
            geometry.controls.push((CommandId::StatusDetails, baseline_rect));
        }
        let message_rect = Rect::new(
            baseline_rect.right(),
            area.y,
            results_rect.x.saturating_sub(baseline_rect.right()),
            1,
        );
        let message = self.session.status_message();
        line(
            frame.buffer_mut(),
            message_rect,
            &code_core::chrome::ellipsize(&format!(" {message}"), message_rect.width as usize),
            style(p.fg, p.status_bg),
        );
        geometry
            .controls
            .push((CommandId::StatusDetails, message_rect));
    }

    fn draw_tree(&mut self, frame: &mut ratatui::Frame, area: Rect, p: &Palette) {
        if area.is_empty() {
            return;
        }
        fill(frame.buffer_mut(), area, style(p.fg, p.sidebar_bg));
        let title = self.session.state().explorer_search().label().map(std::borrow::Cow::Owned).unwrap_or_else(|| self.session.state().project().map(|p| p.root().file_name().unwrap_or_default().to_string_lossy()).unwrap_or_else(|| "Files".into()));
        line(
            frame.buffer_mut(),
            area,
            &format!(
                " {}",
                Icon::FolderOpen.label(
                    self.nerd_icons(),
                    &title
                )
            ),
            style(
                if self.focus == Focus::Tree {
                    p.accent
                } else {
                    p.fg
                },
                p.header_bg,
            )
            .add_modifier(Modifier::BOLD),
        );
        let rows = self.session.state().tree_rows();
        clamp_moved(
            &mut self.tree_at,
            &mut self.tree_top,
            &mut self.tree_seen,
            rows.clone().count(),
            area.height.saturating_sub(1) as usize,
        );
        for (i, entry) in rows
            .enumerate()
            .skip(self.tree_top)
            .take(area.height.saturating_sub(1) as usize)
        {
            let marked = self.session.state().tree_marks().contains(&entry.path);
            let bg = if i == self.tree_at && self.focus == Focus::Tree {
                p.selected_bg
            } else if marked {
                p.fold_bg
            } else {
                p.sidebar_bg
            };
            let r = row(area, (i - self.tree_top + 1) as u16);
            fill(frame.buffer_mut(), r, style(p.fg, bg));
            let mark = match entry.dir {
                Some(true) => "▾",
                Some(false) => "▸",
                None => " ",
            };
            let icon = match entry.dir {
                Some(open) => lenz_content::file_icons::folder(open),
                None => lenz_content::file_icons::file(&entry.name),
            };
            let name = icon.label(self.nerd_icons(), &entry.name);
            let error = self.session.state().row_problem_count(entry) > 0;
            line(
                frame.buffer_mut(),
                r,
                &format!(" {}{mark} {}", "  ".repeat(entry.depth.min(20)), name),
                style(if error { p.error } else if entry.hidden() { p.dim } else { p.fg }, bg).add_modifier(
                    if i == self.tree_at && self.focus == Focus::Tree {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    },
                ),
            );
            let prefix = format!(" {}{mark} {}", "  ".repeat(entry.depth.min(20)), icon.label(self.nerd_icons(), ""));
            let name_x = r.x.saturating_add(unicode_width::UnicodeWidthStr::width(prefix.as_str()) as u16);
            for range in self.session.state().explorer_search().ranges(&entry.name) {
                let Some(matched) = entry.name.get(range.clone()) else { continue };
                let x = name_x.saturating_add(unicode_width::UnicodeWidthStr::width(entry.name.get(..range.start).unwrap_or_default()) as u16);
                if x < r.right() {
                    line(frame.buffer_mut(), Rect { x, width: r.right() - x, ..r }, matched, style(p.bg, p.accent).add_modifier(Modifier::BOLD));
                }
            }
            // At the row's end, right to left: the Compare Selected check, the Git status letter (its color is
            // extra), and the unsaved-draft mark; each over the name if it is long.
            let state = self.session.state();
            let marks = [
                marked.then_some(("✓", p.fg)),
                state.row_status(entry).map(|s| (s.letter(), s.color(p))),
                state.row_dirty(entry).then_some(("●", p.accent)),
                error.then_some(("•", p.error)),
            ];
            // Each mark keeps a blank cell on both sides, so a long name never runs into or past it.
            let mut at = r.right().saturating_sub(2);
            let mut drawn = false;
            for (symbol, fg) in marks.into_iter().flatten() {
                if at <= r.x {
                    break;
                }
                frame.buffer_mut()[(at, r.y)].set_symbol(symbol).set_style(style(fg, bg));
                frame.buffer_mut()[(at + 1, r.y)].set_symbol(" ");
                drawn = true;
                at -= 2;
            }
            if drawn && at + 1 > r.x {
                frame.buffer_mut()[(at + 1, r.y)].set_symbol(" ");
            }
            let icon_x = r.x.saturating_add(3 + 2 * entry.depth.min(20) as u16);
            if self.nerd_icons() && icon_x < r.right() {
                frame.buffer_mut()[(icon_x, r.y)].set_fg(color(icon.color_on(p, bg)));
            }
        }
    }

    fn draw_results(&mut self, frame: &mut ratatui::Frame, geometry: &mut Geometry, area: Rect, p: &Palette) {
        if area.is_empty() {
            return;
        }
        let Some(results) = self.session.state().results() else {
            return;
        };
        fill(frame.buffer_mut(), area, style(p.fg, p.sidebar_bg));
        // What was asked in bold, then what was found; the keys at the right when they fit.
        let header = self.session.results_header().unwrap_or_default();
        let title = format!(" {}", header.title);
        let head = Icon::Search.label(self.nerd_icons(), &title);
        let title_fg = if header.error { p.error } else if self.focus == Focus::Results { p.accent } else { p.fg };
        fill(frame.buffer_mut(), Rect::new(area.x, area.y, area.width, 1), style(p.fg, p.header_bg));
        line(frame.buffer_mut(), area, &head, style(title_fg, p.header_bg).add_modifier(Modifier::BOLD));
        let detail_x = area.x.saturating_add(head.width() as u16 + 2);
        // The buttons at the right, over a narrow heading's text; the keys left of them when they fit.
        let mut right = area.right();
        for (id, label) in self.session.results_controls().into_iter().rev() {
            let width = (label.width() as u16 + 2).min(right.saturating_sub(area.x));
            if width == 0 { break; }
            right -= width;
            let rect = Rect::new(right, area.y, width, 1);
            line(frame.buffer_mut(), rect, &format!(" {label} "), style(p.accent, p.header_bg));
            geometry.controls.push((id, rect));
        }
        // The found count and progress up to the buttons, ending in `…` when cut.
        let room = right.saturating_sub(detail_x + 1) as usize;
        if room > 0 {
            let mut detail = header.detail.clone();
            if detail.width() > room {
                let mut used = 0;
                detail = detail.chars().take_while(|c| { used += unicode_width::UnicodeWidthChar::width(*c).unwrap_or(0); used < room }).collect();
                detail.push('…');
            }
            line(frame.buffer_mut(), Rect::new(detail_x, area.y, room as u16, 1), &detail, style(p.dim, p.header_bg));
        }
        let hint_w = header.hint.width() as u16 + 1;
        if detail_x + header.detail.width() as u16 + 3 + hint_w <= right {
            line(frame.buffer_mut(), Rect::new(right - hint_w, area.y, hint_w, 1), &header.hint, style(p.dim, p.header_bg));
        }
        let list_width = if results.preview().is_some() && area.width >= 64 {
            area.width / 2
        } else {
            area.width
        };
        let list = Rect::new(
            area.x,
            area.y + 1,
            list_width,
            area.height.saturating_sub(1),
        );
        clamp_moved(
            &mut self.results_at,
            &mut self.results_top,
            &mut self.results_seen,
            results.rows().len(),
            list.height as usize,
        );
        let grouped = results.grouped();
        for (i, entry) in results
            .rows()
            .iter()
            .enumerate()
            .skip(self.results_top)
            .take(list.height as usize)
        {
            use code_core::search::Row;
            // The row's text, the part of it dimmed, and the matches emphasized (byte ranges of the text).
            let indent = "  ".repeat(results.row_depth(i).min(16));
            // A folder or file of the tree: its chevron, icon and name (as the explorer shows them), then how many
            // results it holds, dimmed.
            let nerd = self.nerd_icons();
            let node = |label: &str, icon: lenz_content::file_icons::Icon, open: bool, count: usize| {
                let head = format!(" {indent}{} {}", if open { "▾" } else { "▸" }, icon.label(nerd, label));
                let tail = format!("  {count}");
                (format!("{head}{tail}"), head.len()..head.len() + tail.len(), Vec::new())
            };
            let (text, dim, matches) = match entry {
                Row::Dir { label, count, open, .. } => node(&format!("{label}/"), lenz_content::file_icons::folder(*open), *open, *count),
                Row::File { path, count, open } => {
                    let name = results.file_label(path).0;
                    node(name, lenz_content::file_icons::file(name), *open, *count)
                }
                Row::Hit(h) => {
                    let Some(hit) = results.hits().get(*h) else { continue };
                    let call = results.call(*h).map_or(String::new(), |c| {
                        format!(
                            "{}{} ",
                            "  ".repeat(c.depth.min(16) as usize),
                            if c.fold == code_core::search::Fold::Closed { "▸" } else { "▾" }
                        )
                    });
                    // Under its file a line shows its number and its text without the indentation, every result on
                    // it emphasized; elsewhere its place comes first.
                    let prefix = if grouped { format!(" {indent}{:>4}  ", hit.line + 1) } else { format!(" {call}{}  ", results.location(*h).unwrap_or_default()) };
                    let stale = if results.is_stale(*h) { " [changed]" } else { "" };
                    // A long line starts a little before its match, after `…`, so the match is in view.
                    let full = hit.text.replace(['\t', '\r'], " ");
                    let indent = hit.indent().min(full.len());
                    let room = (list.width as usize).saturating_sub(prefix.len() + stale.len());
                    let start = code_core::search::clip_start(&full, indent, hit.range.clone(), room);
                    let ellipsis = if start > indent { "…" } else { "" };
                    let body = full.get(start..).unwrap_or_default();
                    let at = |b: usize| prefix.len() + ellipsis.len() + b.saturating_sub(start).min(body.len());
                    let matches = results.hits()[results.line_hits(*h)].iter()
                        .filter(|m| m.decl.is_none() && m.range.end <= m.text.len())
                        .map(|m| at(m.range.start)..at(m.range.end))
                        .collect();
                    (format!("{prefix}{ellipsis}{body}{stale}"), 0..prefix.len() + ellipsis.len(), matches)
                }
            };
            let r = row(list, (i - self.results_top) as u16);
            let bg = if i == self.results_at && self.focus == Focus::Results {
                p.selected_bg
            } else {
                p.sidebar_bg
            };
            fill(frame.buffer_mut(), r, style(p.fg, bg));
            line(frame.buffer_mut(), r, &text, style(p.fg, bg));
            // Recolour by display column: the dimmed part, then the matches.
            let column = |b: usize| r.x.saturating_add(text.get(..b).map_or(0, |t| t.width()) as u16);
            for (range, st) in std::iter::once((dim, style(p.dim, bg))).chain(matches.into_iter().map(|m| (m, style(p.fg, p.found_bg)))) {
                let (from, to) = (column(range.start).min(r.right()), column(range.end).min(r.right()));
                if from < to {
                    frame.buffer_mut().set_style(Rect::new(from, r.y, to - from, 1), st);
                }
            }
        }
        if list_width < area.width
            && let Some(preview) = results.preview()
        {
            let hit = results.hits().get(preview.index);
            // The preview names its file and line; its rows start below that.
            let title = Rect::new(area.x + list_width + 1, list.y, area.width.saturating_sub(list_width + 1), list.height.min(1));
            line(frame.buffer_mut(), title, &results.location(preview.index).unwrap_or_default(), style(p.dim, p.sidebar_bg));
            let first = hit.map_or(0, |h| h.line.saturating_sub(list.height.saturating_sub(1) as usize / 2));
            let preview_area = Rect::new(
                area.x + list_width + 1,
                list.y + list.height.min(1),
                area.width.saturating_sub(list_width + 1),
                list.height.saturating_sub(1),
            );
            for (n, l) in (first..preview.doc.line_count())
                .take(preview_area.height as usize)
                .enumerate()
            {
                let bg = if hit.is_some_and(|h| h.line == l) {
                    p.found_bg
                } else {
                    p.bg
                };
                line(
                    frame.buffer_mut(),
                    row(preview_area, n as u16),
                    &format!("{:>4} ", l + 1),
                    style(p.fg, bg),
                );
                let shown = code_core::shown::shown_line(preview.doc.line(l), &[]);
                let spans = preview
                    .highlights()
                    .and_then(|parsed| parsed.spans(l..l + 1).pop())
                    .unwrap_or_default();
                for cell in source_cells(
                    &shown,
                    0..shown.text.len(),
                    0,
                    preview_area.width.saturating_sub(5) as usize,
                ) {
                    let fg = spans
                        .iter()
                        .find(|(range, _)| range.contains(&cell.byte))
                        .map_or(p.fg, |(_, capture)| p.syntax_color(*capture));
                    let selected = hit.is_some_and(|h| h.line == l && h.range.contains(&cell.byte));
                    frame.buffer_mut().set_stringn(
                        preview_area.x + 5 + cell.x,
                        preview_area.y + n as u16,
                        cell.text,
                        cell.width as usize,
                        style(fg, if selected { p.found_bg } else { p.bg }),
                    );
                }
            }
        }
    }

    /// Project search as a centered dialog: labelled fields, option toggles and a hint (or why the expression is
    /// refused), wrapping in a narrow terminal.
    fn draw_search(&mut self, frame: &mut ratatui::Frame, geometry: &mut Geometry, p: &Palette) {
        let Some(modal) = &self.modal else { return };
        let options = [
            CommandId::MatchCase,
            CommandId::WholeWord,
            CommandId::RegularExpression,
        ];
        let o = self.session.find_options();
        let on = [o.case, o.word, o.regex];
        let width = frame.area().width.saturating_sub(2).min(72);
        let inner_w = width.saturating_sub(4);
        let labels: Vec<(CommandId, String)> = options
            .into_iter()
            .zip(FindOption::ALL)
            .zip(on)
            .map(|((id, option), on)| {
                let mark = if on { "✓" } else { "○" };
                (id, format!(" {mark} {} {} ", option.symbol(), option.label()))
            })
            .collect();
        // Toggle rows: (row, x offset, width) per toggle, wrapping when the next does not fit.
        let (mut rows, mut x) = (0u16, 0u16);
        let mut placed = Vec::new();
        for (_, label) in &labels {
            let w = (label.width() as u16).min(inner_w);
            if x > 0 && x + w > inner_w {
                (rows, x) = (rows + 1, 0);
            }
            placed.push((rows, x, w));
            x += w + 2;
        }
        let refused = self.session.search_refusal(&modal.input);
        let hint = match &refused {
            Some(e) => format!("{e} — fix the expression to search"),
            None => "Enter search · Tab switch field · Alt+C/W/R options · Esc close".to_owned(),
        };
        let hint = wrap_text(&hint, inner_w.max(1) as usize);
        let toggles_y = 5;
        let hint_y = toggles_y + rows + 2;
        let height = hint_y + hint.len() as u16 + 3;
        let area = centered(frame.area(), width, height);
        geometry.modal = area;
        if area.is_empty() {
            return;
        }
        frame.render_widget(Clear, area);
        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(color(p.border)))
                .title(" Search in Project ")
                .title_style(
                    Style::default()
                        .fg(color(p.fg))
                        .add_modifier(Modifier::BOLD),
                )
                .style(style(p.fg, p.status_bg)),
            area,
        );
        let content = inner(area);
        let content = Rect::new(
            content.x.saturating_add(1),
            content.y,
            content.width.saturating_sub(2),
            content.height,
        );
        if content.is_empty() {
            return;
        }
        const LABEL: u16 = 7;
        let fields = [
            ("Find", &modal.input, "Text to search for"),
            ("Files", &modal.secondary, "All files — e.g. *.rs, src/, !*.test.ts"),
        ];
        for (i, (label, value, placeholder)) in fields.into_iter().enumerate() {
            let r = row(content, 1 + 2 * i as u16);
            let active = modal.secondary_focus == (i == 1);
            line(
                frame.buffer_mut(),
                r,
                label,
                if active {
                    style(p.accent, p.status_bg).add_modifier(Modifier::BOLD)
                } else {
                    style(p.dim, p.status_bg)
                },
            );
            let field = Rect::new(
                r.x + LABEL.min(r.width),
                r.y,
                r.width.saturating_sub(LABEL),
                r.height,
            );
            geometry.search_fields.push(field);
            fill(frame.buffer_mut(), field, style(p.fg, p.bg));
            let text_area = Rect::new(
                field.x.saturating_add(1),
                field.y,
                field.width.saturating_sub(2),
                field.height,
            );
            let shown = tail_columns(value, text_area.width.saturating_sub(1) as usize);
            if value.is_empty() {
                line(frame.buffer_mut(), text_area, placeholder, style(p.dim, p.bg));
            } else {
                line(frame.buffer_mut(), text_area, shown, style(p.fg, if active && modal.selected { p.selected_bg } else { p.bg }));
            }
            if active && text_area.height > 0 && text_area.width > 0 {
                frame.set_cursor_position((
                    text_area.x
                        + (shown.width() as u16).min(text_area.width.saturating_sub(1)),
                    text_area.y,
                ));
            }
        }
        for (((id, label), (dy, dx, w)), on) in labels.iter().zip(placed).zip(on) {
            let r = row(content, toggles_y + dy);
            let rect = Rect::new(
                r.x + dx.min(r.width),
                r.y,
                w.min(r.width.saturating_sub(dx)),
                r.height,
            );
            line(
                frame.buffer_mut(),
                rect,
                label,
                if on {
                    style(p.accent, p.selected_bg).add_modifier(Modifier::BOLD)
                } else {
                    style(p.fg, p.bg)
                },
            );
            geometry.search_toggles.push((*id, rect));
        }
        for (i, text) in hint.iter().enumerate() {
            line(
                frame.buffer_mut(),
                row(content, hint_y + i as u16),
                text,
                style(if refused.is_some() { p.error } else { p.dim }, p.status_bg),
            );
        }
    }

    fn draw_navigation(
        &mut self,
        frame: &mut ratatui::Frame,
        geometry: &mut Geometry,
        p: &Palette,
    ) {
        let status = self.session.navigation_status();
        let nerd = self.nerd_icons();
        let Some(search) = self.session.state().navigation() else {
            return;
        };
        let Some(modal) = &mut self.modal else { return };
        let area = centered(
            frame.area(),
            frame.area().width.saturating_sub(2).min(100),
            frame.area().height.saturating_sub(2).min(28),
        );
        geometry.modal = area;
        geometry.navigation_revision = Some(search.revision);
        if area.is_empty() {
            return;
        }
        frame.render_widget(Clear, area);
        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(color(p.border)))
                .title(if search.scope == Scope::Recent {
                    " Recent Files "
                } else {
                    " Search Everywhere "
                })
                .title_style(
                    Style::default()
                        .fg(color(p.fg))
                        .add_modifier(Modifier::BOLD),
                )
                .style(style(p.fg, p.status_bg)),
            area,
        );
        let content = inner(area);
        if content.is_empty() {
            return;
        }
        let mut x = content.x;
        let scopes: &[Scope] = if search.scope == Scope::Recent {
            &[Scope::Recent]
        } else {
            &Scope::ALL
        };
        for &scope in scopes {
            if content.width < 42 && scope != search.scope {
                continue;
            }
            let label = format!(" {} ", scope.label());
            let width = (label.width() as u16).min(content.right().saturating_sub(x));
            if width == 0 {
                break;
            }
            let rect = Rect::new(x, content.y, width, 1);
            let selected = scope == search.scope;
            line(
                frame.buffer_mut(),
                rect,
                &label,
                style(
                    if selected { p.accent } else { p.dim },
                    if selected { p.selected_bg } else { p.status_bg },
                )
                .add_modifier(if selected {
                    Modifier::BOLD | Modifier::UNDERLINED
                } else {
                    Modifier::empty()
                }),
            );
            geometry.navigation_scopes.push((scope, rect));
            x = x.saturating_add(width + 1);
        }
        if content.height > 1 {
            let query = tail_columns(&modal.input, content.width.saturating_sub(3) as usize);
            line(
                frame.buffer_mut(),
                row(content, 1),
                &format!("› {query}"),
                style(p.fg, p.status_bg),
            );
            if content.width > 2 {
                frame.set_cursor_position((
                    content.x
                        + 2
                        + query.width().min(content.width.saturating_sub(3) as usize) as u16,
                    content.y + 1,
                ));
            }
        }
        let list_y = 2.min(content.height);
        let reserve = u16::from(content.height > list_y);
        let list = Rect::new(
            content.x,
            content.y + list_y,
            content.width,
            content.height.saturating_sub(list_y + reserve),
        );
        geometry.modal_rows = list;
        let row_height = if list.height >= 4 { 2 } else { 1 };
        let visible = (list.height / row_height).max(1) as usize;
        modal.at = modal.at.min(search.rows.len().saturating_sub(1));
        modal.top = modal.top.min(search.rows.len().saturating_sub(visible));
        if modal.reveal_selection {
            clamp(&mut modal.at, &mut modal.top, search.rows.len(), visible);
            modal.reveal_selection = false;
        }
        geometry.modal_content_rows = search.rows.len();
        for (index, result) in search.rows.iter().enumerate().skip(modal.top).take(visible) {
            let screen = ((index - modal.top) * row_height as usize) as u16;
            if screen >= list.height {
                break;
            }
            let selected = index == modal.at;
            let bg = if selected { p.selected_bg } else { p.status_bg };
            let fg = if result.disabled.is_some() {
                p.dim
            } else {
                p.fg
            };
            let icon = match result.kind {
                Kind::File => lenz_content::file_icons::file(&result.label)
                    .label(nerd, &result.label),
                Kind::Symbol => Icon::Book.label(nerd, &result.label),
                Kind::Action => Icon::Commands.label(nerd, &result.label),
            };
            let setting = result.on.map_or("", |on| if on { "✓ " } else { "○ " });
            let label = format!(
                "{}{}{}  {}{}",
                if selected { "› " } else { "  " },
                setting,
                icon,
                result.kind.label(),
                result
                    .shortcut
                    .as_deref()
                    .map(|s| format!("  {s}"))
                    .unwrap_or_default()
            );
            let rect = row(list, screen);
            fill(frame.buffer_mut(), rect, style(fg, bg));
            line(
                frame.buffer_mut(),
                rect,
                &label,
                style(fg, bg).add_modifier(if selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
            );
            geometry.modal_items.push(index);
            if row_height > 1 {
                let detail = result.disabled.unwrap_or(&result.detail);
                let rect = row(list, screen + 1);
                fill(frame.buffer_mut(), rect, style(p.dim, bg));
                line(
                    frame.buffer_mut(),
                    rect,
                    &format!("  {detail}"),
                    style(p.dim, bg),
                );
                geometry.modal_items.push(index);
            }
        }
        if search.rows.is_empty() && list.height > 0 {
            line(
                frame.buffer_mut(),
                list,
                if search.pending {
                    "  Searching…"
                } else {
                    "  No matching results"
                },
                style(p.dim, p.status_bg),
            );
        }
        if reserve > 0 {
            line(
                frame.buffer_mut(),
                row(content, content.height - 1),
                &status,
                style(p.dim, p.status_bg),
            );
        }
    }

    fn draw_modal(&mut self, frame: &mut ratatui::Frame, geometry: &mut Geometry, p: &Palette) {
        if let Some(text) = self.form_text() {
            self.draw_form(frame, geometry, p, text);
            return;
        }
        if self
            .modal
            .as_ref()
            .is_some_and(|m| m.kind == Prompt::Navigation)
        {
            self.draw_navigation(frame, geometry, p);
            return;
        }
        if self
            .modal
            .as_ref()
            .is_some_and(|m| m.kind == Prompt::Search)
        {
            self.draw_search(frame, geometry, p);
            return;
        }
        let title = if self.modal.as_ref().is_some_and(|m| m.kind == Prompt::Branches) { self.session.state().branches().map_or_else(|| "Branches".to_string(), |b| b.heading().to_string()) } else { self.modal_title() };
        let details_buttons = self.details_buttons();
        let theme_source = self
            .modal
            .as_ref()
            .filter(|m| m.kind == Prompt::Themes)
            .map(|_| self.session.theme_source());
        let runtime_message = self
            .session
            .state()
            .server_runtime()
            .map(|m| m.message.clone());
        let choices = if self
            .modal
            .as_ref()
            .is_some_and(|m| matches!(m.kind, Prompt::LanguageServer | Prompt::ServerRuntime))
        {
            self.choices_for(
                frame
                    .area()
                    .width
                    .saturating_sub(4)
                    .min(98)
                    .saturating_sub(2)
                    .max(1) as usize,
            )
        } else {
            self.choices()
        };
        let Some(modal) = &mut self.modal else {
            return;
        };
        let context = matches!(modal.kind, Prompt::Context { .. } | Prompt::Menu(_));
        let action_list = context || matches!(modal.kind, Prompt::Commands | Prompt::LanguageServer);
        let themes = modal.kind == Prompt::Themes;
        let mut separators = Vec::new();
        let area = if let Prompt::Menu(_) = modal.kind {
            // A dropdown under the toolbar's button, as wide as its longest row with its shortcut.
            let hint = |c: &crate::app::Choice| match c.target { crate::app::ChoiceTarget::Action(id) => self.session.key_hint(id).map_or(0, |key| key.width() + 3), _ => 0 };
            let width = (choices.iter().map(|c| c.label.width() + hint(c)).max().unwrap_or(0) + 6)
                .max(34)
                .min(frame.area().width as usize) as u16;
            // Above the status bar; a long group scrolls.
            let height = (choices.len() as u16 + 3).min(frame.area().height.saturating_sub(2));
            Rect::new(frame.area().x, frame.area().y + 1.min(frame.area().height), width, height)
        } else if let Prompt::Context { x, y, ref entries } = modal.kind {
            let width = (choices.iter().map(|c| c.label.width()).max().unwrap_or(0) + 4)
                .max(20)
                .min(frame.area().width as usize) as u16;
            let height = (entries.len() + 3).min(frame.area().height as usize) as u16;
            let mut choice = 0;
            for entry in entries {
                match entry {
                    MenuEntry::Separator => separators.push(choice),
                    MenuEntry::Command(_) => choice += 1,
                }
            }
            Rect::new(
                x.min(frame.area().right().saturating_sub(width)),
                y.min(frame.area().bottom().saturating_sub(height)),
                width,
                height,
            )
        } else {
            let (mut width, mut height) = (
                frame.area().width.saturating_sub(2).min(if themes { 70 } else { 100 }),
                frame.area().height.saturating_sub(2).min(if themes { 22 } else { 28 }),
            );
            // A dialog is as wide as its longest line (at least 44 columns) and as tall as its text and buttons.
            if let Prompt::Details { body, logo: false, .. } = &modal.kind {
                let longest = body.lines().map(UnicodeWidthStr::width).max().unwrap_or(0) as u16;
                width = width.min(longest.saturating_add(4).max(44));
                let inner = width.saturating_sub(2);
                let rows = wrap_text(body, inner.max(1) as usize).len() as u16;
                let footer = details::footer_rows(&details_buttons, inner, height.saturating_sub(3));
                height = height.min(rows.saturating_add(footer).saturating_add(3 + u16::from(footer > 0)));
            }
            if modal.kind == Prompt::Find {
                // A small box at the top right, below the toolbar and tabs, so the matches stay in view.
                let area = frame.area();
                let width = area.width.min(64);
                Rect::new(area.right() - width, area.y + 2.min(area.height), width, 5.min(area.height.saturating_sub(2)))
            } else {
                centered(frame.area(), width, height)
            }
        };
        geometry.modal = area;
        if area.is_empty() {
            return;
        }
        frame.render_widget(Clear, area);
        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(color(p.border)))
                .title(if title.is_empty() { String::new() } else { format!(" {title} ") })
                .title_style(
                    Style::default()
                        .fg(color(p.fg))
                        .add_modifier(Modifier::BOLD),
                )
                .style(style(p.fg, p.status_bg)),
            area,
        );
        let content = inner(area);
        if matches!(modal.kind, Prompt::Help | Prompt::Details { .. }) {
            let mut content = content;
            if let Prompt::Details { kind, .. } = modal.kind {
                let rows = details::footer_rows(&details_buttons, content.width, content.height.saturating_sub(1)).min(content.height);
                let footer = Rect::new(content.x, content.bottom().saturating_sub(rows), content.width, rows);
                content.height = content.height.saturating_sub(rows + u16::from(content.height > rows + 2));
                details::paint_footer(frame, geometry, footer, &details_buttons, kind, p);
            }
            let body = match &modal.kind {
                Prompt::Details { body, .. } => body.as_str(),
                _ => HELP,
            };
            // About's logo at the right, when the text keeps at least 30 columns beside it.
            let logo = lenz_content::about::LOGO_TERMINAL_SIZE as u16;
            if matches!(modal.kind, Prompt::Details { logo: true, .. }) && content.width >= logo + 32 && content.height >= logo / 2 {
                draw_about_logo(frame.buffer_mut(), Rect::new(content.right() - logo, content.y, logo, logo / 2), p.status_bg);
                content.width -= logo + 2;
            }
            let wrapped = wrap_text(body, content.width.max(1) as usize);
            geometry.modal_content_rows = wrapped.len();
            geometry.modal_rows = content;
            modal.top = modal
                .top
                .min(wrapped.len().saturating_sub(content.height as usize));
            for (n, text) in wrapped
                .iter()
                .skip(modal.top)
                .take(content.height as usize)
                .enumerate()
            {
                line(
                    frame.buffer_mut(),
                    row(content, n as u16),
                    text,
                    style(p.fg, p.status_bg).add_modifier(if text.split_once(':').is_some_and(|(label, _)| label.len() <= 40 && !label.contains('/') && !label.contains('`')) { Modifier::BOLD } else { Modifier::empty() }),
                );
            }
            return;
        }
        let input_rows = if context || themes || modal.kind == Prompt::LanguageServer {
            0
        } else {
            1 + usize::from(
                modal.kind == Prompt::Find
                    || matches!(modal.kind, Prompt::Search)
                    || !modal.secondary.is_empty()
                    || modal.secondary_focus,
            )
        };
        for i in 0..input_rows.min(content.height as usize) {
            let active = (i == 1) == modal.secondary_focus;
            let prefix = if modal.kind == Prompt::Branches { "Filter: " } else if active { "› " } else { "  " };
            let room = content.width.saturating_sub(prefix.len() as u16) as usize;
            let text = field_text(modal, i == 1);
            let tail = tail_columns(&text, room);
            let bg = if modal.kind == Prompt::Branches { p.bg } else { p.status_bg };
            fill(frame.buffer_mut(), row(content, i as u16), style(p.fg, bg));
            line(frame.buffer_mut(), row(content, i as u16), &format!("{prefix}{tail}"), style(if active { p.fg } else { p.dim }, bg));
        }
        let list_y = input_rows.min(content.height as usize) as u16;
        let reserve = u16::from(!themes && content.height > list_y);
        geometry.modal_rows = Rect::new(
            content.x,
            content.y + list_y,
            content.width,
            content.height.saturating_sub(list_y + reserve),
        );
        let list = geometry.modal_rows;
        // Wrapped details participate in scrolling: selection remains a choice index, top is a visual row.
        let width = list.width.max(1) as usize;
        let mut rendered = Vec::new();
        if themes {
            for header in [
                "↑/↓ choose · Enter select · Esc close".to_owned(),
                format!("Current: {}", theme_source.as_deref().unwrap_or_default()),
            ] {
                for text in wrap_text(&header, width) {
                    rendered.push((usize::MAX, text, false));
                }
            }
        }
        let mut selected = 0..0;
        modal.at = modal.at.min(choices.len().saturating_sub(1));
        for (i, choice) in choices.iter().enumerate() {
            if modal.kind == Prompt::LanguageServer && matches!(i, 4 | 8) {
                rendered.push((usize::MAX, if i == 4 { "Setup" } else { "Indexing" }.to_string(), true));
            }
            if separators.contains(&i) {
                rendered.push((usize::MAX, "─".repeat(width), true));
            }
            let start = rendered.len();
            let suffix = if action_list {
                String::new()
            } else {
                choice
                    .disabled
                    .map(|why| format!(" — {why}"))
                    .or_else(|| choice.detail.as_ref().map(|d| format!(" — {d}")))
                    .unwrap_or_default()
            };
            let shortcut = if action_list { match choice.target { crate::app::ChoiceTarget::Action(id) => self.session.key_hint(id), _ => None } } else { None };
            let shortcut = shortcut.filter(|key| key.width() + 3 < width / 2);
            let label_width = shortcut.as_ref().map_or(width, |key| width.saturating_sub(key.width() + 3).max(1));
            for (part, text) in wrap_text(
                &format!(
                    "{}{}{}",
                    if i == modal.at { "› " } else { "  " },
                    choice.label,
                    suffix
                ),
                label_width,
            ).into_iter().enumerate() {
                let text = match shortcut.as_ref().filter(|key| part == 0 && key.width() + 3 < width / 2) {
                    Some(key) => format!("{text}{}{key}", " ".repeat(width.saturating_sub(text.width() + key.width()))),
                    None => text,
                };
                rendered.push((i, text, choice.disabled.is_some()));
            }
            if i == modal.at {
                selected = start..rendered.len();
            }
        }
        geometry.modal_content_rows = rendered.len();
        modal.top = modal
            .top
            .min(rendered.len().saturating_sub(list.height as usize));
        if modal.reveal_selection {
            if selected.start < modal.top {
                modal.top = selected.start;
            }
            if list.height > 0 && selected.start >= modal.top + list.height as usize {
                modal.top = selected.start + 1 - list.height as usize;
            }
            modal.reveal_selection = false;
        }
        for (n, (i, text, disabled)) in rendered
            .iter()
            .skip(modal.top)
            .take(list.height as usize)
            .enumerate()
        {
            geometry.modal_items.push(*i);
            let bg = if *i == modal.at {
                p.selected_bg
            } else {
                p.status_bg
            };
            let r = row(list, n as u16);
            fill(frame.buffer_mut(), r, style(p.fg, bg));
            line(
                frame.buffer_mut(),
                r,
                text,
                style(
                    if themes && *i == usize::MAX {
                        p.accent
                    } else if *disabled {
                        p.dim
                    } else {
                        p.fg
                    },
                    bg,
                ),
            );
        }
        if reserve > 0 {
            // The find prompt says which match is current, or why the expression cannot be matched.
            let find = (modal.kind == Prompt::Find).then(|| self.session.state().active().and_then(|tab| tab.find())).flatten();
            let finding = find.is_some_and(|find| find.error().is_some());
            let hint = if modal.secondary_focus { FIND_HINT } else { FIND_QUERY_HINT };
            let find_footer = find.map(|find| {
                let counter = find.counter();
                if counter.is_empty() { hint.to_owned() } else { format!("{counter} · {hint}") }
            });
            let footer = modal
                .error
                .as_deref()
                .or(find_footer.as_deref())
                .or_else(|| {
                    action_list
                        .then(|| choices.get(modal.at).and_then(|c| c.disabled))
                        .flatten()
                })
                .or_else(|| {
                    (modal.kind == Prompt::ServerRuntime)
                        .then_some(runtime_message.as_deref())
                        .flatten()
                })
                .unwrap_or(if modal.kind == Prompt::LanguageServer {
                    "Ctrl+C copies details · Enter runs action · Esc closes"
                } else if modal.kind == Prompt::Menu(None) {
                    "Enter / → open · Esc closes"
                } else if matches!(modal.kind, Prompt::Menu(Some(_))) {
                    "Enter runs · ← back · Esc closes"
                } else {
                    "Enter accepts · Esc closes · ↑↓ selects · Tab field"
                });
            line(
                frame.buffer_mut(),
                row(content, content.height - 1),
                footer,
                style(
                    if modal.error.is_some() || finding {
                        p.error
                    } else {
                        p.dim
                    },
                    p.status_bg,
                ),
            );
        }
        if !context
            && !themes
            && modal.kind != Prompt::LanguageServer
            && content.height > 0
            && content.width > 2
        {
            let text = field_text(modal, modal.secondary_focus);
            frame.set_cursor_position((
                content.x
                    + 2
                    + UnicodeWidthStr::width(tail_columns(
                        &text,
                        content.width.saturating_sub(3) as usize,
                    ))
                    .min(content.width.saturating_sub(3) as usize) as u16,
                content.y + u16::from(modal.secondary_focus).min(content.height - 1),
            ));
        }
    }
}

/// What the find prompt's footer says under its fields.
const FIND_HINT: &str = "Enter replaces · Alt+A all · Tab field · Esc closes";
const FIND_QUERY_HINT: &str = "Enter next · F3 / Shift+F3 · Tab replace · Esc closes";

/// A prompt field as drawn: the find prompt's second row is its replacement text, labelled so an empty one is
/// clear; the caret column follows the same text.
fn field_text(modal: &crate::app::Modal, second: bool) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    let text = if second { modal.secondary.as_str() } else { modal.input.as_str() };
    if second && modal.kind == Prompt::Find { Cow::Owned(format!("{}: {text}", CommandId::ReplaceMatch.label())) } else { Cow::Borrowed(text) }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width.min(area.width),
        height.min(area.height),
    )
}

/// Cells for the largest line number `lines` (no allocation: this runs for every drawn row).
fn number_cells(lines: usize) -> u16 {
    lines.checked_ilog10().map_or(1, |d| d as u16 + 1)
}

/// The blame column's cells left of the line numbers in pane `pane` of `width` columns (0: none, also when it would
/// leave the text fewer than 8 columns).
fn annotation_cells(state: &State, pane: u8, width: u16) -> u16 {
    let Some(t) = state.shown(pane) else { return 0 };
    let Some(view) = state.view(t.id()) else { return 0 };
    let text = (view.snapshot().image().is_none() || t.image_source()) && !matches!(state.view(t.id()).and_then(|v| v.rendering()), Some(Rendering::Preview(_) | Rendering::Pending));
    let cells = state.annotation_cols(t) as u16 + 1;
    let digits = number_cells(view.line_count()) + 3;
    if text && cells > 1 && width >= digits + cells + 8 { cells } else { 0 }
}

/// Cells the draft change markers draw in, left of the line numbers: the change letter, then the deletion above.
const MARKER_CELLS: u16 = 2;

/// Those cells for pane `pane`: a text file has them (it is given a baseline), an image or preview does not.
fn marker_cells(state: &State, pane: u8) -> u16 {
    let Some(tab) = state.shown(pane) else { return 0 };
    let Some(view) = state.view(tab.id()) else { return 0 };
    let text = (view.snapshot().image().is_none() || tab.image_source())
        && !matches!(view.rendering(), Some(Rendering::Preview(_) | Rendering::Pending));
    if text { MARKER_CELLS } else { 0 }
}

fn text_area(state: &State, pane: u8, area: Rect) -> Rect {
    let bars = 1 + u16::from(state.breadcrumbs());
    let gutter = state
        .shown(pane)
        .and_then(|t| state.view(t.id()))
        .map_or(3 + MARKER_CELLS, |view| number_cells(view.line_count()) + 3 + marker_cells(state, pane))
        .saturating_add(annotation_cells(state, pane, area.width))
        .min(area.width);
    Rect::new(
        area.x + gutter,
        area.y + bars.min(area.height),
        area.width.saturating_sub(gutter),
        area.height.saturating_sub(bars),
    )
}

/// Nothing open: the start screen's commands, then each group of recents under its heading, each a click target.
fn draw_start(frame: &mut ratatui::Frame, app: &App, items: &[StartItem], geometry: &mut PaneGeometry, p: &Palette) {
    let area = geometry.text;
    let area = Rect::new(area.x.saturating_add(2), area.y.saturating_add(1), area.width.saturating_sub(2), area.height.saturating_sub(1));
    let (mut offset, mut heading) = (0, None);
    for (i, item) in items.iter().enumerate() {
        let label = match item {
            StartItem::Command(id) => app.session.key_hint(*id).map_or_else(|| id.label().to_string(), |key| format!("{}  {key}", id.label())),
            StartItem::RecentFolder(path) | StartItem::RecentFile(path) => path.display().to_string(),
        };
        // Each group of recents under a blank row and its heading.
        if item.heading() != heading {
            heading = item.heading();
            if let Some(h) = heading {
                line(frame.buffer_mut(), row(area, offset + 1), h, style(p.dim, p.bg));
                offset += 2;
            }
        }
        let at = row(area, offset);
        let at = Rect { width: at.width.min(label.width() as u16), ..at };
        line(frame.buffer_mut(), at, &label, style(p.accent, p.bg));
        if !at.is_empty() {
            geometry.start.push((i, at));
        }
        offset += 1;
    }
}

fn draw_pane(
    frame: &mut ratatui::Frame,
    app: &App,
    pane: u8,
    area: Rect,
    p: &Palette,
) -> PaneGeometry {
    let state = app.session.state();
    let focus = app.focus;
    let crumb_at = app.crumb_at;
    let preview_selection = app.preview_selection.clone();
    let preview_scroll = state
        .shown(pane)
        .and_then(|tab| app.preview_scroll.get(&tab.id()).cloned());
    let old_markdown = app
        .layout
        .panes
        .get(pane as usize)
        .and_then(|pane| pane.markdown.clone());
    let mut geometry = PaneGeometry {
        pane,
        text: text_area(state, pane, area),
        gutter_x: area.x,
        ann: annotation_cells(state, pane, area.width),
        ..Default::default()
    };
    if area.is_empty() {
        return geometry;
    }
    let active = state.shown(pane).map(Tab::id);
    fill(frame.buffer_mut(), row(area, 0), style(p.fg, p.header_bg));
    let mut tab_end = area.right();
    if let Some(control) = app.session.csv_control(pane).or_else(|| app.session.preview_control(pane)).filter(|_| area.width >= 17) {
        let button = Rect::new(area.right() - 9, area.y, 9, 1);
        let st = style(if control.disabled.is_none() { p.fg } else { p.dim }, p.fold_bg);
        fill(frame.buffer_mut(), button, st);
        line(frame.buffer_mut(), button, &format!(" {} ", control.label), st);
        geometry.controls.push((control.id, button));
        tab_end = button.x;
    }
    let mut x = area.x;
    let tab_width = tab_end.saturating_sub(x);
    geometry.tab_bar = Rect::new(x, area.y, tab_width, 1);
    let tabs: Vec<_> = state.pane_tabs(pane).collect();
    let widths: Vec<_> = tabs
        .iter()
        .map(|tab| {
            (UnicodeWidthStr::width(tab.title().as_str()) as u16
                + UnicodeWidthStr::width(state.problem_badge(tab.id()).as_str()) as u16
                + 4
                + if state.tab_dirty(tab.id()) { 2 } else { 0 }
                + if tab.recovered() { 2 } else { 0 }
                + if state.tab_status(tab.id()).is_some() { 2 } else { 0 }
                + if app.nerd_icons() { 2 } else { 0 })
            .min(32)
            .min(tab_width)
        })
        .collect();
    let selected = tabs
        .iter()
        .position(|tab| Some(tab.id()) == active)
        .unwrap_or(0);
    let mut last = widths.len().saturating_sub(1);
    let mut used = widths.get(last).copied().unwrap_or(0);
    while last > 0 && used.saturating_add(widths[last - 1]) <= tab_width {
        last -= 1;
        used += widths[last];
    }
    let mut strips = app.tab_strips.get();
    let strip = &mut strips[usize::from(pane)];
    strip.last = last;
    strip.first = strip.first.min(last);
    if strip.shown != active { strip.manual = false; }
    if !strip.manual {
        if selected < strip.first { strip.first = selected; }
        else if widths.get(strip.first..selected.saturating_add(1)).unwrap_or_default().iter().fold(0u16, |sum, w| sum.saturating_add(*w)) > tab_width {
            strip.first = selected;
            let mut used = widths.get(selected).copied().unwrap_or(0);
            while strip.first > 0 && used.saturating_add(widths[strip.first - 1]) <= tab_width {
                strip.first -= 1; used += widths[strip.first];
            }
        }
        strip.shown = active;
    }
    let first = strip.first.min(last);
    app.tab_strips.set(strips);
    for (i, tab) in tabs.iter().enumerate().skip(first) {
        // The unsaved mark before the name, the Git status letter after it: independent of each other.
        let status = state.tab_status(tab.id());
        let named = format!(
            " {}{}{}{}",
            state.problem_badge(tab.id()),
            if tab.recovered() { "⟲ " } else { "" },
            if state.tab_dirty(tab.id()) { "● " } else { "" },
            lenz_content::file_icons::file(&tab.title()).label(app.nerd_icons(), &tab.title()),
        );
        let text = format!(
            "{named}{}{} ",
            status.map_or(String::new(), |s| format!(" {}", s.letter())),
            if tab.retained() { "" } else { " ◇" }
        );
        let width = widths[i].min(tab_end.saturating_sub(x));
        if width == 0 {
            break;
        }
        let r = Rect::new(x, area.y, width, 1);
        geometry.tabs.push((tab.id(), r));
        line(
            frame.buffer_mut(),
            r,
            &text,
            style(
                if Some(tab.id()) == active {
                    p.accent
                } else {
                    p.dim
                },
                if Some(tab.id()) == active {
                    p.bg
                } else {
                    p.header_bg
                },
            )
            .add_modifier(if Some(tab.id()) == active {
                Modifier::BOLD
                    | if focus == Focus::Text && pane == state.focused_pane() {
                        Modifier::UNDERLINED
                    } else {
                        Modifier::empty()
                    }
            } else {
                Modifier::empty()
            }),
        );
        if app.nerd_icons() && width > 1 {
            let icon = lenz_content::file_icons::file(&tab.title());
            let bg = if Some(tab.id()) == active {
                p.bg
            } else {
                p.header_bg
            };
            frame.buffer_mut()[(r.x + 1, r.y)].set_fg(color(icon.color_on(p, bg)));
        }
        if let Some(s) = status
            && let letter = r.x + UnicodeWidthStr::width(named.as_str()) as u16 + 1
            && letter < r.right()
        {
            frame.buffer_mut()[(letter, r.y)].set_fg(color(s.color(p)));
        }
        x += width;
    }
    if state.breadcrumbs() && area.height > 1 {
        let bar_row = Rect::new(area.x, area.y + 1, area.width, 1);
        fill(frame.buffer_mut(), bar_row, style(p.fg, p.bg));
        if let Some(bar) = state.breadcrumbs_for(pane) {
            use lenz_content::highlight::DeclKind;
            use code_core::breadcrumbs::Crumb;
            let nerd = app.nerd_icons();
            let file = bar.crumbs.iter().find_map(|c| match c {
                Crumb::File { name } => Some(name.as_ref()),
                _ => None,
            });
            let note = if bar.large { " large file " } else { "" };
            let focused =
                (focus == Focus::Breadcrumbs && pane == state.focused_pane()).then_some(crumb_at);
            // Each crumb is padded by a cell on each side, with a "›" between; a file's icon takes two more.
            let kept = bar.fit(
                |name| (name.width() + 2 + if nerd && file == Some(name) { 2 } else { 0 }) as f32,
                1.0,
                f32::from(area.width.saturating_sub(note.width() as u16)),
                focused,
            );
            let last = bar.crumbs.len().saturating_sub(1);
            let (mut x, mut gap) = (area.x, false);
            for (i, crumb) in bar.crumbs.iter().enumerate() {
                let shown = if kept.get(i) == Some(&true) {
                    gap = false;
                    Some(crumb)
                } else if !gap {
                    gap = true;
                    None
                } else {
                    continue;
                };
                if x > area.x {
                    line(
                        frame.buffer_mut(),
                        Rect::new(x, bar_row.y, area.right().saturating_sub(x).min(1), 1),
                        "›",
                        style(p.dim, p.bg),
                    );
                    x += 1;
                }
                let text = match shown {
                    Some(Crumb::File { name }) => {
                        format!(" {} ", lenz_content::file_icons::file(name).label(nerd, name))
                    }
                    Some(c) => format!(" {} ", c.name()),
                    None => " … ".to_owned(),
                };
                let width = (text.width() as u16).min(area.right().saturating_sub(x));
                if width == 0 {
                    break;
                }
                let r = Rect::new(x, bar_row.y, width, 1);
                let is_focused = shown.is_some() && focused == Some(i);
                let fg = match shown {
                    None | Some(Crumb::Folder { .. }) => p.dim,
                    Some(Crumb::File { .. }) => p.fg,
                    Some(Crumb::Decl { kind, .. }) => match kind {
                        DeclKind::Function | DeclKind::Method | DeclKind::Component => {
                            p.syntax.function
                        }
                        DeclKind::Type | DeclKind::Interface | DeclKind::Impl | DeclKind::Module => {
                            p.syntax.type_
                        }
                        DeclKind::Constant => p.syntax.constant,
                        DeclKind::Macro => p.syntax.macro_,
                        DeclKind::Element | DeclKind::Selector => p.syntax.tag,
                        DeclKind::Heading => p.accent,
                    },
                };
                let bg = if is_focused { p.selected_bg } else { p.bg };
                line(
                    frame.buffer_mut(),
                    r,
                    &text,
                    style(if is_focused { p.accent } else { fg }, bg).add_modifier(
                        if is_focused || (shown.is_some() && i == last) {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        },
                    ),
                );
                if let Some(Crumb::File { name }) = shown
                    && nerd
                    && width > 1
                {
                    let icon = lenz_content::file_icons::file(name);
                    frame.buffer_mut()[(r.x + 1, r.y)].set_fg(color(icon.color_on(p, bg)));
                }
                if shown.is_some() {
                    geometry.crumbs.push((i, r));
                }
                x += width;
            }
            let note_w = note.width() as u16;
            if note_w > 0 && area.right().saturating_sub(note_w) >= x {
                line(
                    frame.buffer_mut(),
                    Rect::new(area.right() - note_w, bar_row.y, note_w, 1),
                    note,
                    style(p.dim, p.bg).add_modifier(Modifier::ITALIC),
                );
            }
        }
    }
    let Some(tab) = state.shown(pane) else {
        let items = if pane == 0 { state.start_items() } else { Vec::new() };
        if items.is_empty() {
            line(
                frame.buffer_mut(),
                geometry.text,
                "Open a file with Ctrl+P or F10",
                style(p.dim, p.bg),
            );
        }
        draw_start(frame, app, &items, &mut geometry, p);
        return geometry;
    };
    let Some(view) = state.view(tab.id()) else {
        line(
            frame.buffer_mut(),
            geometry.text,
            "Loading…",
            style(p.dim, p.bg),
        );
        return geometry;
    };
    if state.csv(tab.id()).is_some() {
        geometry.csv = crate::app::csv::draw(frame, app, pane, geometry.text, p);
        return geometry;
    }
    geometry.source = Some(SourceIdentity::new(&view, app));
    if let Some(image) = view.snapshot().image().filter(|_| !tab.image_source()) {
        match &image.pixels {
            Ok(pixels) => draw_image(frame.buffer_mut(), geometry.text, pixels, p),
            Err(why) => line(
                frame.buffer_mut(),
                geometry.text,
                &format!("Image {}×{} · {why}", image.width, image.height),
                style(p.dim, p.bg),
            ),
        }
        return geometry;
    }
    if let Some(rendering) = state.view(tab.id()).and_then(|v| v.rendering()) {
        match rendering {
            Rendering::Preview(preview) => {
                geometry.preview = true;
                let remap = old_markdown.as_ref().is_some_and(|layout| {
                    layout.width != geometry.text.width
                        || !std::sync::Arc::ptr_eq(&layout.preview, preview)
                });
                let anchor = preview_scroll.as_ref()
                    .filter(|(source, _)| source.matches(&view))
                    .and_then(|(_, top)| {
                        old_markdown
                            .as_ref()
                            .filter(|old| std::sync::Arc::ptr_eq(&old.preview.doc, &preview.doc))
                            .and_then(|old| old.rows.get(*top))
                            .map(|row| (row.block, row.range.start))
                    });
                let layout = old_markdown
                    .filter(|layout| {
                        layout.width == geometry.text.width
                            && std::sync::Arc::ptr_eq(&layout.preview, preview)
                    })
                    .unwrap_or_else(|| {
                        std::sync::Arc::new(MarkdownLayout::new(
                            preview.clone(),
                            geometry.text.width,
                            view.snapshot(),
                        ))
                    });
                geometry.preview_total = layout.rows.len();
                geometry.preview_source_lines = layout.source_lines.clone();
                geometry.preview_top = preview_scroll
                    .filter(|(source, _)| source.matches(&view))
                    .map(|(_, top)| {
                        if remap && let Some((block, offset)) = anchor {
                            layout
                                .rows
                                .iter()
                                .position(|row| {
                                    row.block >= block
                                        && (row.range.end > offset || row.range.start >= offset)
                                })
                                .unwrap_or(top)
                        } else {
                            top
                        }
                    })
                    .unwrap_or_else(|| {
                        layout
                            .source_lines
                            .partition_point(|&line| line < tab.top_line())
                    })
                    .min(
                        layout
                            .rows
                            .len()
                            .saturating_sub(geometry.text.height as usize),
                    );
                draw_markdown(
                    frame.buffer_mut(),
                    &mut geometry,
                    preview_selection
                        .filter(|(id, source, _)| *id == tab.id() && source.matches(&view))
                        .map(|(_, _, selection)| selection),
                    &layout,
                    p,
                );
                geometry.markdown = Some(layout);
                return geometry;
            }
            Rendering::Pending => {
                line(
                    frame.buffer_mut(),
                    geometry.text,
                    "Preparing Markdown preview…",
                    style(p.dim, p.bg),
                );
                return geometry;
            }
            Rendering::Source => {}
        }
    }
    let cols = state.wrap_cols(tab);
    use code_core::layout::Query;
    let (sticky, sticky_pending) = if state.sticky_scroll() {
        match view.layout_sticky(code_core::tabs::sticky_cap(geometry.text.height as usize), cols) {
            Query::Ready(lines) => (lines, None),
            Query::Pending(work) => (Arc::from([]), Some(work)),
        }
    } else { (Arc::from([]), None) };
    let layout = view.layout_rows(geometry.text.height as usize, cols);
    let layout_complete = layout.pending.is_none() && sticky_pending.is_none();
    let rows = layout.rows;
    if rows.is_empty() && layout.pending.is_some() {
        line(frame.buffer_mut(), geometry.text, "Preparing text…", style(p.dim, p.bg));
    }
    let mut extent = Some(0usize);
    for (n, r) in rows.iter().enumerate() {
        let y = geometry.text.y + n as u16;
        let native = native_source_row(app, pane, &view, r, cols, geometry.text.width);
        extent = extent.zip(native.as_ref().and_then(|row| row.extent)).map(|(a, b)| a.max(b));
        draw_source_row(
            frame.buffer_mut(),
            state,
            &view,
            r,
            native,
            geometry.text,
            y,
            area.x,
            false,
            &mut geometry.hits,
            p,
        );
        geometry.rows.push((y, r.line, r.index));
    }
    // Past the last line the number column keeps its background down the pane, without numbers.
    if layout.pending.is_none() {
        for y in geometry.text.y + rows.len() as u16..geometry.text.bottom() {
            fill(frame.buffer_mut(), Rect::new(area.x, y, geometry.text.x.saturating_sub(area.x), 1), style(p.num_fg, p.gutter_bg));
        }
    }
    for pinned in sticky.iter() {
        let r = match view.layout_row(pinned.line, 0, cols) {
            Query::Ready(Some(row)) => row,
            Query::Ready(None) => continue,
            Query::Pending(work) => { app.source_cache.borrow_mut().layout(pane, tab.id(), work); continue; }
        };
        let y = geometry.text.y + pinned.row as u16;
        let native = native_source_row(app, pane, &view, &r, cols, geometry.text.width);
        extent = extent.zip(native.as_ref().and_then(|row| row.extent)).map(|(a, b)| a.max(b));
        geometry.hits.retain(|h| h.y != y);
        geometry.rows.retain(|(cy, ..)| *cy != y);
        // The pinned line covers the whole row: nothing of the scrolled line beneath shows past its end.
        let row = Rect::new(area.x, y, geometry.text.right().saturating_sub(area.x), 1);
        ratatui::widgets::Widget::render(ratatui::widgets::Clear, row, frame.buffer_mut());
        fill(frame.buffer_mut(), row, style(p.fg, p.header_bg));
        draw_source_row(
            frame.buffer_mut(),
            state,
            &view,
            &r,
            native,
            geometry.text,
            y,
            area.x,
            true,
            &mut geometry.hits,
            p,
        );
        geometry.rows.push((y, r.line, r.index));
    }
    mark_problems(frame.buffer_mut(), state, tab.id(), &geometry.hits, p);
    if let Some(work) = layout.pending.or(sticky_pending) {
        app.source_cache.borrow_mut().layout(pane, tab.id(), work);
    }
    if layout_complete && let Some(extent) = extent {
        app.source_cache.borrow_mut().clamp(app.session.native_viewport(pane), &view, geometry.text.width as usize, extent);
    }
    geometry
}

fn native_source_row(app: &App, pane: u8, view: &View<'_>, row: &code_core::layout::Row, cols: Option<usize>, width: u16) -> Option<crate::source::Row> {
    if cols.is_none() {
        return Some(app.source_cache.borrow_mut().row(pane, view, row.line, view.tab().column(), width as usize, app.session.projection(pane).map(AsRef::as_ref), app.session.native_reveal(pane).filter(|goal| goal.identity.line() == row.line)));
    }
    let held = app.source_cache.borrow().cursor(pane, view, row.line, row.display.start);
    let cursor = if let Some(cursor) = held { cursor } else {
        match view.layout_cursor(row.line, row.display.start, cols) {
            code_core::layout::Query::Ready(cursor) => cursor?,
            code_core::layout::Query::Pending(work) => { app.source_cache.borrow_mut().layout(pane, view.tab().id(), work); return None; }
        }
    };
    let goal = app.session.native_reveal(pane).filter(|goal| goal.identity.line() == row.line).filter(|goal| {
        match view.layout_position(goal.source, cols) {
            code_core::layout::Query::Ready(position) => position.is_some_and(|position| position.row == row.index),
            code_core::layout::Query::Pending(work) => { app.source_cache.borrow_mut().layout(pane, view.tab().id(), work); false }
        }
    });
    Some(app.source_cache.borrow_mut().wrapped(pane, view, cursor, row.display.clone(), row.complete.then(|| row.end_byte()), view.tab().column(), row.indent, width as usize, app.session.projection(pane).map(AsRef::as_ref), goal))
}

struct SourceCell<'a> {
    x: u16,
    width: u16,
    byte: usize,
    text: &'a str,
}
fn source_cells(
    shown: &code_core::shown::Shown,
    range: Range<usize>,
    skip: usize,
    width: usize,
) -> Vec<SourceCell<'_>> {
    let mut cells = Vec::new();
    let mut col = 0usize;
    let Some(text) = shown.text.get(range.clone()) else {
        return cells;
    };
    for (byte, grapheme) in text.grapheme_indices(true) {
        let w = UnicodeWidthStr::width(grapheme);
        if col >= skip && col + w <= skip.saturating_add(width) {
            let display = range.start + byte;
            let source = shown
                .hints
                .iter()
                .find(|(_, r, _)| r.contains(&display))
                .map_or_else(
                    || {
                        shown
                            .map
                            .partition_point(|&d| d <= display)
                            .saturating_sub(1)
                    },
                    |(b, ..)| *b,
                );
            cells.push(SourceCell {
                x: (col - skip) as u16,
                width: w as u16,
                byte: source,
                text: grapheme,
            });
        }
        col += w;
        if col >= skip.saturating_add(width) {
            break;
        }
    }
    cells
}

#[allow(clippy::too_many_arguments)]
fn draw_source_row(
    buf: &mut Buffer,
    state: &State,
    view: &View<'_>,
    r: &code_core::layout::Row,
    native: Option<crate::source::Row>,
    area: Rect,
    y: u16,
    gutter_x: u16,
    sticky: bool,
    hits: &mut Vec<TextHit>,
    p: &Palette,
) {
    if area.width == 0 || y >= area.bottom() {
        return;
    }
    let tab = view.tab();
    let Some(source_line) = view.snapshot().line_range(r.line) else { return };
    let changed = state.comparison_changed_line(tab.id(), r.line);
    let base = if sticky { p.header_bg } else if changed { p.block[2] } else { p.bg };
    fill(
        buf,
        Rect::new(gutter_x, y, area.right().saturating_sub(gutter_x), 1),
        style(p.fg, base),
    );
    let bookmark = state.places().bookmark(tab.source(), r.line);
    let fold = if r.index == 0 {
        view.fold_at(r.line)
    } else {
        None
    };
    let mark = if changed { "▌" } else if bookmark.is_some() { "●" } else { " " };
    let fold_mark = fold.map_or(" ", |(_, shut)| if shut { "▸" } else { "▾" });
    // The blame column first: the line's date and author on its commit's age shade (none on wrapped and sticky rows).
    let digits = number_cells(view.line_count()) as usize + 3;
    let ann = (area.x.saturating_sub(gutter_x) as usize).saturating_sub(digits + usize::from(MARKER_CELLS)) as u16;
    if ann > 0 {
        let a = matches!(tab.blame_status(), Some(code_core::blame::Status::Ready))
            .then(|| tab.annotation_with_width(r.line, state.layout().annotation_cols())).flatten().filter(|_| r.index == 0 && !sticky);
        let bg = match a.as_ref().and_then(|a| a.shade) {
            Some(s) => lenz_content::theme::mix(p.gutter_bg, p.blame_new, s * 0.6),
            None => p.gutter_bg,
        };
        let fg = if a.as_ref().is_some_and(|a| a.shade.is_some()) { p.fg } else { p.dim };
        let status = tab.blame_status().and_then(code_core::blame::Status::label).filter(|_| r.index == 0 && !sticky);
        let label = status.map_or_else(|| a.map_or(String::new(), |a| format!("{:<1$}", a.label, ann as usize)), str::to_owned);
        let cell = Rect::new(gutter_x, y, ann, 1);
        fill(buf, cell, style(fg, bg));
        line(buf, cell, &label, style(fg, bg));
    }
    let gutter_x = gutter_x + ann;
    let gutter = area.x.saturating_sub(gutter_x) as usize;
    // The draft's change against its baseline: the change's letter, then whether lines above it were deleted.
    let markers = state.markers(tab.id()).filter(|_| r.index == 0);
    let change = markers.and_then(|m| m.change(r.line));
    let deleted = markers.map_or(0, |m| m.deleted_above(r.line));
    let marker = format!(
        "{}{}",
        change.map_or(" ", code_core::markers::LineChange::letter),
        if deleted > 0 { code_core::markers::DELETED } else { " " }
    );
    let number = if r.index == 0 {
        format!(
            "{marker}{}{:>width$}{}",
            mark,
            state.line_number(tab.id(), r.line),
            fold_mark,
            width = gutter.saturating_sub(2 + MARKER_CELLS as usize)
        )
    } else {
        " ".repeat(gutter)
    };
    line(
        buf,
        Rect::new(gutter_x, y, area.x - gutter_x, 1),
        &number,
        style(
            if bookmark.is_some() {
                p.accent
            } else {
                p.num_fg
            },
            p.gutter_bg,
        ),
    );
    if r.index == 0 && gutter_x.saturating_add(MARKER_CELLS) <= area.x {
        if let Some(change) = change {
            buf[(gutter_x, y)].set_fg(color(change.color(p)));
        }
        if deleted > 0 {
            buf[(gutter_x + 1, y)].set_fg(color(code_core::markers::deleted_color(p)));
        }
    }
    let Some(native) = native.as_ref() else {
        line(buf, Rect::new(area.x, y, area.width, 1), "Preparing text…", style(p.dim, base));
        return;
    };
    if native.failed || (native.pending && native.cells.is_empty()) {
        line(buf, Rect::new(area.x, y, area.width, 1), if native.failed { "Text layout failed" } else { "Preparing text…" }, style(p.dim, base));
        return;
    }
    let start = source_line.start;
    // Every selection of the view: the primary one and the cursors added below or above it.
    let selections: Vec<_> = tab.selections().collect();
    let find = tab.find();
    let occurrences = (state.active().is_some_and(|a| a.id() == tab.id()))
        .then(|| state.occurrences())
        .flatten();
    let skip = tab.column().saturating_sub(r.indent);
    let indent = r.indent.saturating_sub(tab.column()).min(area.width as usize);
    let mut end_x = area.x + indent as u16;
    for cell in native.cells.iter() {
        let source = cell.byte;
        let byte = source.saturating_sub(start);
        let width = cell.width.min(area.width as usize) as u16;
        let mut st = style(p.fg, base);
        let (capture, mods) = view.style_at(r.line, byte);
        if let Some(capture) = capture { st = st.fg(color(p.syntax_color(capture))); }
        if mods & code_core::decor::MUTABLE != 0 { st = st.add_modifier(Modifier::UNDERLINED); }
        if mods & code_core::decor::DEPRECATED != 0 { st = st.add_modifier(Modifier::CROSSED_OUT); }
        let hint = cell.hint;
        if hint {
            st = style(p.dim, base).add_modifier(Modifier::ITALIC);
        }
        if !hint && occurrences.is_some_and(|o| o.ranges.iter().any(|(r, _)| r.contains(&source))) {
            st = st.bg(color(p.fold_bg));
        }
        if !hint
            && find.is_some_and(|f| {
                f.in_line(r.line)
                    .iter()
                    .any(|(_, range)| range.contains(&byte))
            })
        {
            st = st.bg(color(p.found_bg));
        }
        if !hint && selections.iter().any(|s| s.range().contains(&source)) {
            st = st.bg(color(p.sel_bg));
        }
        if selections.iter().any(|s| s.anchor == s.head && s.head == source) && cell.caret {
            st = st.add_modifier(Modifier::REVERSED);
        }
        let x = area.x + indent as u16 + cell.column.saturating_sub(skip) as u16;
        buf.set_stringn(x, y, if cell.width > area.width as usize { "▏" } else { cell.text() }, width as usize, st);
        hits.push(TextHit {
            x,
            y,
            width: width.max(1),
            line: r.line,
            byte,
        });
        end_x = x + width;
    }
    if end_x < area.right() && let Some(end) = native.end.map(|end| end.saturating_sub(start)) {
        hits.push(TextHit {
            x: end_x,
            y,
            width: area.right() - end_x,
            line: r.line,
            byte: end,
        });
        if !sticky && r.last && start + end == source_line.end
            && native.extent.is_some_and(|extent| extent > tab.column())
            && selections.iter().any(|s| view.snapshot().line_ending_selected(r.line, s.range())) {
            buf.set_stringn(end_x, y, " ", 1, style(p.fg, p.sel_bg));
        }
        if selections.iter().any(|s| s.anchor == s.head && s.head == start + end) {
            buf.set_stringn(
                end_x,
                y,
                " ",
                1,
                style(p.fg, base).add_modifier(Modifier::REVERSED),
            );
        }
        if fold.is_some_and(|(_, shut)| shut) {
            buf.set_stringn(
                end_x,
                y,
                " …",
                (area.right() - end_x) as usize,
                style(p.dim, p.fold_bg),
            );
        }
    }
    if r.last && native.end.is_some() && !native.pending && let Some(problem) = state.inline_problem(tab.id(), r.line) {
        use code_core::semantics::diagnostics::Severity;
        let fg = match problem.severity { Severity::Error => p.error, Severity::Warning => p.accent, Severity::Information | Severity::Hint => p.dim };
        // Empty/EOF diagnostics have no glyph to underline: keep one source-coordinate anchor visible.
        if problem.empty {
            if let Some(hit) = hits.iter().find(|hit| hit.y == y && hit.line == r.line && hit.byte == problem.byte) {
                if let Some(cell) = buf.cell_mut((hit.x, y)) {
                    if problem.byte == source_line.len() { cell.set_symbol("!"); }
                    cell.set_style(style(fg, base).add_modifier(Modifier::UNDERLINED));
                }
            } else if area.x > gutter_x && let Some(cell) = buf.cell_mut((area.x - 1, y)) {
                if cell.symbol().trim().is_empty() { cell.set_symbol("!"); }
                cell.set_style(style(fg, p.gutter_bg).add_modifier(Modifier::UNDERLINED));
            }
        }
        let after = end_x.saturating_add(if fold.is_some_and(|(_, shut)| shut) { 2 } else { 0 });
        let x = after.saturating_add(2);
        if x < area.right() {
            let extra = if problem.other_count > 0 { format!(" (+{})", problem.other_count) } else { String::new() };
            let message = format!("{}: {}{extra}", problem.severity.label(), problem.message);
            line(buf, Rect::new(x, y, area.right() - x, 1), &message, style(fg, base));
        }
    }

}

fn draw_image(buf: &mut Buffer, area: Rect, pixels: &lenz_content::image::Pixels, p: &Palette) {
    if area.is_empty() || pixels.width() == 0 || pixels.height() == 0 {
        return;
    }
    let scale = (area.width as f64 / pixels.width() as f64)
        .min(area.height as f64 * 2.0 / pixels.height() as f64);
    let width = (pixels.width() as f64 * scale).floor().max(1.0) as u16;
    let height = (pixels.height() as f64 * scale / 2.0).ceil().max(1.0) as u16;
    let blend = |pixel: &[u8; 4]| {
        let a = pixel[3] as u32;
        let channel = |i: usize, shift: u32| {
            ((pixel[i] as u32 * a + ((p.bg >> shift) & 255) * (255 - a)) / 255) as u8
        };
        Color::Rgb(channel(0, 16), channel(1, 8), channel(2, 0))
    };
    for y in 0..height.min(area.height) {
        for x in 0..width.min(area.width) {
            let sx = ((x as f64 / scale) as u32).min(pixels.width() - 1);
            let sy = ((y as f64 * 2.0 / scale) as u32).min(pixels.height() - 1);
            let sy2 = (((y as f64 * 2.0 + 1.0) / scale) as u32).min(pixels.height() - 1);
            buf.set_stringn(
                area.x + x,
                area.y + y,
                "▀",
                1,
                Style::default()
                    .fg(blend(&pixels.get_pixel(sx, sy).0))
                    .bg(blend(&pixels.get_pixel(sx, sy2).0)),
            );
        }
    }
}

struct MarkdownLayout {
    preview: std::sync::Arc<code_core::markdown::Preview>,
    width: u16,
    rows: Vec<MarkdownRow>,
    source_lines: std::sync::Arc<[usize]>,
}

struct MarkdownRow {
    block: usize,
    range: Range<usize>,
    kind: MarkdownRowKind,
}

enum MarkdownRowKind {
    Text { first: bool },
    Image(usize),
    MissingImage,
    Blank,
}

impl MarkdownLayout {
    fn new(
        preview: std::sync::Arc<code_core::markdown::Preview>,
        width: u16,
        source: &code_core::text::TextSnapshot,
    ) -> Self {
        let mut rows = Vec::new();
        let mut lines = Vec::new();
        for (index, block) in preview.doc.blocks.iter().enumerate() {
            let mut push = |range: Range<usize>, kind| {
                let offset =
                    preview_source(&preview.doc, range.start).unwrap_or(block.source.start);
                let line = source.line_of(offset.min(source.len_bytes()));
                lines.push(line.max(lines.last().copied().unwrap_or(0)));
                rows.push(MarkdownRow {
                    block: index,
                    range,
                    kind,
                });
            };
            if matches!(block.kind, BlockKind::Image { .. }) {
                if let Some(Ok(image)) = preview
                    .images
                    .as_ref()
                    .and_then(|images| images.get(&index))
                {
                    let (_, _, height) = markdown_image_size(image, width);
                    for row in 0..height {
                        push(block.text.clone(), MarkdownRowKind::Image(row));
                    }
                } else {
                    push(block.text.clone(), MarkdownRowKind::MissingImage);
                }
            } else {
                let prefix = markdown_prefix(block, width);
                let cols = (width as usize)
                    .saturating_sub(UnicodeWidthStr::width(prefix.as_str()))
                    .max(1);
                let body = markdown_body(block);
                for (i, range) in wrap_ranges(
                    preview.doc.text.get(body.clone()).unwrap_or_default(),
                    cols,
                )
                .into_iter()
                .enumerate()
                {
                    push(
                        body.start + range.start..body.start + range.end,
                        MarkdownRowKind::Text { first: i == 0 },
                    );
                }
            }
            // Items of one list follow each other without a blank row, as the desktop spaces them closely.
            let next_item = preview.doc.blocks.get(index + 1).is_some_and(|b| matches!(b.kind, BlockKind::ListItem { .. }));
            if !(matches!(block.kind, BlockKind::ListItem { .. }) && next_item) {
                push(block.text.end..block.text.end, MarkdownRowKind::Blank);
            }
        }
        Self {
            preview,
            width,
            rows,
            source_lines: lines.into(),
        }
    }
}

fn markdown_image_size(image: &lenz_content::image::Pixels, width: u16) -> (f64, u16, usize) {
    // Bound image layout metadata even for extremely tall, narrow decoded images.
    const MAX_ROWS: usize = 20;
    let scale = (width as f64 / image.width().max(1) as f64)
        .min((MAX_ROWS * 2) as f64 / image.height().max(1) as f64)
        .min(1.0);
    (
        scale,
        (image.width() as f64 * scale).floor().max(1.0) as u16,
        (image.height() as f64 * scale / 2.0).ceil().max(1.0) as usize,
    )
}

fn preview_source(doc: &lenz_content::markdown::MarkdownDocument, offset: usize) -> Option<usize> {
    use lenz_content::markdown::Origin;
    let i = doc
        .segments
        .partition_point(|segment| segment.text.end <= offset);
    let segment = doc.segments.get(i)?;
    if !segment.text.contains(&offset) {
        return None;
    }
    match &segment.origin {
        Origin::Direct(range) => Some(range.start + offset - segment.text.start),
        Origin::Normalized(range) => Some(range.start),
        Origin::Synthetic => None,
    }
}

/// A block's text as drawn after its prefix: a list item's text starts with its marker (and a task's `[ ] `), which
/// the prefix shows instead.
fn markdown_body(block: &lenz_content::markdown::Block) -> Range<usize> {
    match &block.kind {
        BlockKind::ListItem { marker, checked } => (block.text.start + marker.len() + 1 + if checked.is_some() { 4 } else { 0 }).min(block.text.end)..block.text.end,
        _ => block.text.clone(),
    }
}

fn markdown_prefix(block: &lenz_content::markdown::Block, width: u16) -> String {
    match &block.kind {
        BlockKind::ListItem { marker, checked } => format!(
            "{}{} ",
            "  ".repeat(block.list_depth.min(12) as usize),
            checked.map_or_else(
                || marker.clone(),
                |checked| if checked { "☑".into() } else { "☐".into() }
            )
        ),
        BlockKind::Rule => "─".repeat(width as usize),
        _ if block.quote_depth > 0 => "│ ".into(),
        _ => String::new(),
    }
}

fn wrap_ranges(text: &str, columns: usize) -> Vec<Range<usize>> {
    let (mut first, mut width) = (0, 0);
    let mut space = None;
    let mut rows = Vec::new();
    for (at, g) in text.grapheme_indices(true) {
        if g == "\n" {
            rows.push(first..at);
            first = at + 1;
            width = 0;
            space = None;
            continue;
        }
        let w = if g == "\t" {
            4
        } else {
            UnicodeWidthStr::width(g)
        };
        if width + w > columns.max(1) && width > 0 {
            if let Some((end, used)) = space.filter(|&(end, _)| end > first && end <= at) {
                rows.push(first..end);
                first = end;
                width -= used;
            }
            if width + w > columns.max(1) && width > 0 {
                rows.push(first..at);
                first = at;
                width = 0;
            }
            space = None;
        }
        width += w;
        if g.chars().all(char::is_whitespace) { space = Some((at + g.len(), width)); }
    }
    rows.push(first..text.len());
    rows
}

fn draw_markdown(
    buf: &mut Buffer,
    geometry: &mut PaneGeometry,
    selection: Option<code_core::session::Selection>,
    layout: &MarkdownLayout,
    p: &Palette,
) {
    let area = geometry.text;
    let preview = &layout.preview;
    for (n, row) in layout
        .rows
        .iter()
        .skip(geometry.preview_top)
        .take(area.height as usize)
        .enumerate()
    {
        let y = area.y + n as u16;
        let block = &preview.doc.blocks[row.block];
        match row.kind {
            MarkdownRowKind::Blank => continue,
            MarkdownRowKind::Image(image_row) => {
                if let Some(Ok(image)) = preview
                    .images
                    .as_ref()
                    .and_then(|images| images.get(&row.block))
                {
                    let (scale, width, _) = markdown_image_size(image, area.width);
                    if scale <= 0.0 {
                        continue;
                    }
                    for x in 0..width.min(area.width) {
                        let sx = ((x as f64 / scale) as u32).min(image.width() - 1);
                        let sy = ((image_row as f64 * 2.0 / scale) as u32).min(image.height() - 1);
                        let sy2 = (((image_row as f64 * 2.0 + 1.0) / scale) as u32)
                            .min(image.height() - 1);
                        let pixel = |sy| {
                            let rgba = image.get_pixel(sx, sy).0;
                            let alpha = rgba[3] as u32;
                            let ch = |i: usize, shift: u32| {
                                ((rgba[i] as u32 * alpha + ((p.bg >> shift) & 255) * (255 - alpha))
                                    / 255) as u8
                            };
                            Color::Rgb(ch(0, 16), ch(1, 8), ch(2, 0))
                        };
                        buf.set_stringn(
                            area.x + x,
                            y,
                            "▀",
                            1,
                            Style::default().fg(pixel(sy)).bg(pixel(sy2)),
                        );
                    }
                }
                continue;
            }
            MarkdownRowKind::MissingImage => {
                if let BlockKind::Image { alt, .. } = &block.kind {
                    let why = preview
                        .images
                        .as_ref()
                        .and_then(|images| images.get(&row.block))
                        .and_then(|image| image.as_ref().err())
                        .copied()
                        .unwrap_or("loading image…");
                    line(
                        buf,
                        Rect::new(area.x, y, area.width, 1),
                        &format!("[Image: {alt} · {why}]"),
                        style(p.dim, p.bg),
                    );
                }
                continue;
            }
            MarkdownRowKind::Text { first } => {
                let prefix = markdown_prefix(block, area.width);
                let indent =
                    UnicodeWidthStr::width(prefix.as_str()).min(area.width as usize) as u16;
                if first {
                    line(
                        buf,
                        Rect::new(area.x, y, area.width, 1),
                        &prefix,
                        style(p.accent, p.bg),
                    );
                }
                let mut x = area.x + indent;
                geometry.rows.push((y, n, 0));
                for (local, grapheme) in preview.doc.text[row.range.clone()].grapheme_indices(true)
                {
                    let offset = row.range.start + local;
                    let mut st = style(p.fg, p.bg);
                    if matches!(block.kind, BlockKind::Heading(_)) {
                        st = st.fg(color(p.accent)).add_modifier(Modifier::BOLD);
                    }
                    if let Some(run) = block.runs.iter().find(|run| run.text.contains(&offset)) {
                        let s = run.style;
                        if s.strong {
                            st = st.add_modifier(Modifier::BOLD);
                        }
                        if s.emphasis {
                            st = st.add_modifier(Modifier::ITALIC);
                        }
                        if s.strike {
                            st = st.add_modifier(Modifier::CROSSED_OUT);
                        }
                        if s.link || s.wikilink {
                            st = st
                                .fg(color(p.syntax.link))
                                .add_modifier(Modifier::UNDERLINED);
                        }
                        if s.code {
                            st = st.fg(color(p.syntax.string)).bg(color(p.gutter_bg));
                        }
                        if s.highlight {
                            st = st.bg(color(p.found_bg));
                        }
                    }
                    if let Some(spans) = preview.syntax.get(&row.block)
                        && let Some((_, capture)) =
                            spans.iter().find(|(range, _)| range.contains(&offset))
                    {
                        st = st.fg(color(p.syntax_color(*capture)));
                    }
                    if selection.is_some_and(|s| s.range().contains(&offset)) {
                        st = st.bg(color(p.sel_bg));
                    }
                    if selection.is_some_and(|s| s.anchor == s.head && s.head == offset) {
                        st = st.add_modifier(Modifier::REVERSED);
                    }
                    let width = if grapheme == "\t" {
                        4
                    } else {
                        UnicodeWidthStr::width(grapheme) as u16
                    };
                    if x.saturating_add(width) <= area.right() {
                        buf.set_stringn(
                            x,
                            y,
                            if grapheme == "\t" { "    " } else { grapheme },
                            width as usize,
                            st,
                        );
                        geometry.hits.push(TextHit {
                            x,
                            y,
                            width: width.max(1),
                            line: n,
                            byte: offset,
                        });
                    }
                    x = x.saturating_add(width);
                }
                if x < area.right() {
                    geometry.hits.push(TextHit {
                        x,
                        y,
                        width: area.right() - x,
                        line: n,
                        byte: row.range.end,
                    });
                }
            }
        }
    }
}

fn tail_columns(text: &str, columns: usize) -> &str {
    let mut width = 0;
    let mut start = text.len();
    for (i, g) in text.grapheme_indices(true).rev() {
        width += UnicodeWidthStr::width(g);
        if width > columns {
            break;
        }
        start = i;
    }
    &text[start..]
}

/// `text` wrapped to `columns`, at most `max` lines, the last ending in `…` when cut.
fn wrap(text: &str, columns: usize, max: usize) -> Vec<String> {
    let mut lines: Vec<String> = text.lines().flat_map(|l| wrap_text(l, columns.max(8))).collect();
    while lines.last().is_some_and(|l| l.trim().is_empty()) { lines.pop(); }
    if lines.len() > max {
        lines.truncate(max);
        if let Some(last) = lines.last_mut() { last.push('…'); }
    }
    lines
}

/// Dialog and list text in rows of `columns` cells, broken after spaces and punctuation as the editor wraps (a word
/// longer than the row is cut at the edge).
pub(crate) fn wrap_text(text: &str, columns: usize) -> Vec<String> {
    let mut rows = Vec::new();
    for line in text.split('\n') {
        let mut at = 0;
        for end in lenz_content::wrap::wrap_starts(line, columns).into_iter().chain([line.len()]) {
            rows.push(line.get(at..end).unwrap_or_default().trim_end_matches(' ').to_owned());
            at = end;
        }
    }
    rows
}

const HELP: &str = "Lenz Code\n\nF10 opens every shared editor command, including its availability and disabled reason. Type to filter, use arrows to choose, Enter to run.\n\nCtrl+K / Double Shift  Search Everywhere\nCtrl+P / Ctrl+Shift+N  Find file (name:line:column)\nCtrl+Alt+Shift+N  Find symbol\nCtrl+Shift+A / Ctrl+Shift+P  Find action\nTab / Shift+Tab in Search Everywhere  Change scope\nCtrl+O  Open file\nF10 → Open folder  Browse folder\nCtrl+F  Find in file\nF3 / Shift+F3  Next / previous match\nAlt+C / W / R  Find case / word / regex · Tab in find: replace, Alt+A: replace all\nCtrl+Shift+F  Search project\nCtrl+G  Go to line\nCtrl+Shift+E  Focus explorer\nTab / Shift+Tab  Indent / outdent in editable source; cycle controls outside source input mode\nArrows / PageUp / PageDown  Navigate\nShift+arrows  Extend selection\nCtrl+A / Ctrl+C  Select all / copy\nCtrl+S  Save draft\nCtrl+Z / Ctrl+Shift+Z  Undo / redo\nTyping / bracketed paste  Edit source on supported input backends\nClick or drag  Select source text (double: word, triple: its syntax group)\nAlt+B  Annotate with Git blame (click an annotation for its commit)\nCtrl+`  Git branches (or click the branch in the top bar)\nSpace in the explorer, then Ctrl+D  Compare two entries\nRight-click text  Shared context actions\nClick a fold mark  Fold or unfold\nCtrl+F4  Close tab\nCtrl+W / Ctrl+Shift+W  Grow / shrink selection (VS Code keymap: Alt+Shift+Right / Left)\nCtrl+Alt+Down / Up  Add a cursor below / above (in both keymaps; the palette has both)\nAlt+Shift+Down / Up  Duplicate the line or selection below / above\nCtrl+Shift+Down / Up  Move the line or selection down / up (VS Code keymap: Alt+Down / Up)\nCtrl+/ or Alt+/  Comment or uncomment the selection\nAlt+J  Select the word, then the next occurrence, at every cursor\nEsc  Back to one cursor, then close a dialog or pending request\nCtrl+Q  Close this view (Ctrl+Shift+Q: all views)\nF2  Language server status and recovery\nShift+F2  Full status details (Ctrl+C copies)\nEsc  Close dialog, documentation or cancel a pending request\n\nThe Application group in F10 holds Keymap… (preset, rebinding, every chord). F10 also provides bookmarks, history, results, provider controls, themes, split panes, breadcrumbs, hints, sticky scroll, source/preview presentation and the Git comparisons in Lenz Diff.\n\nTerminal capabilities\nImages use true-color half-block cells; Markdown images fit within 20 terminal rows. Text uses terminal fonts and cell widths; italic, underline and strike depend on the terminal. Markdown is rendered from the same parsed document and local images as the GUI. Source selection keeps UTF-8 coordinates. Clipboard uses OSC 52 and requires terminal support. Double Shift needs modifier key reports from a terminal supporting the enhanced keyboard protocol; Ctrl+K always opens Search Everywhere. Modified arrows are reported as such only by terminals that send them, and Ctrl+/ often arrives as Ctrl+_, so Add Cursor Below / Above, Move Lines, Duplicate Lines and Toggle Comment stay in the Edit menu and the palette (F10). Independent windows retain their own views.\n\nEditing changes an unsaved draft; Ctrl+S writes it. Closing its last view asks Save, Discard or Cancel. Read-only mode blocks editing. Git checkouts and branches also change your repository.\n\nEsc closes this help.";

#[cfg(test)]
#[path = "render_tests.rs"]
mod render_tests;
