//! Only visible rows and columns become terminal cells. Source text is never ANSI.
use std::ops::Range;
use ratatui::{buffer::Buffer, layout::Rect, style::{Modifier, Style}, widgets::{Block, Borders, Clear, Paragraph}};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use lenz::{document::Placeholder, view::{Content, Side}};
use crate::app::{App, Prompt};

use crate::chrome::color;
fn base(palette:&lenz::theme::Palette) -> Style { crate::chrome::style_with(palette) }

/// Source viewport shared by painting, hit testing and caret scrolling.
#[cfg(test)]
pub(crate) fn text_geometry(area: Rect, side: Side, focus: Side, blame: bool) -> Rect {
    text_geometry_ratio(area,side,focus,blame,0.5)
}
pub(crate) fn text_geometry_ratio(area:Rect,side:Side,focus:Side,blame:bool,ratio:f32)->Rect {
    let narrow = area.width < 70;
    if narrow && side != focus { return Rect::new(area.x, area.y + 2, 0, 0); }
    let left = crate::panes::left(area.width,ratio);
    let x = area.x + if !narrow && side == Side::Right { left + 1 } else { 0 };
    let width = if narrow { area.width } else if side == Side::Left { left } else { area.width.saturating_sub(left + 1) };
    let gutter = width.min(7);
    let annotation = if blame { annotation_width(width,gutter) } else { 0 };
    Rect::new(x + gutter + annotation, area.y + 2, width - gutter - annotation, area.height.saturating_sub(4))
}

/// Unified: one full-width pane for both Sides.
pub(crate) fn unified_geometry(area:Rect,blame:bool)->Rect {
    let gutter=area.width.min(7);
    let annotation=if blame {annotation_width(area.width,gutter)} else {0};
    Rect::new(area.x+gutter+annotation,area.y+2,area.width-gutter-annotation,area.height.saturating_sub(4))
}

pub(crate) fn annotation_width(width:u16,gutter:u16)->u16 {width.saturating_sub(gutter).saturating_sub(4).min(26)}

/// Clip on grapheme boundaries, preserving terminal column widths and byte-based highlights.
#[allow(clippy::too_many_arguments)] // Palette plus existing cell/decorations inputs.
fn source(palette:&lenz::theme::Palette,buf: &mut Buffer, area: Rect, text: &str, offset: usize, style: Style, inline: &[Range<usize>], hits: &[Range<usize>]) {
    rich_source(palette,buf,area,text,offset,style,Decorations{inline,inline_color:None,hits,syntax:&[],selection:None,caret:None});
}
struct Decorations<'a>{inline:&'a [Range<usize>],inline_color:Option<u32>,hits:&'a [Range<usize>],syntax:&'a [(Range<usize>,lenz::highlight::Capture)],selection:Option<Range<usize>>,caret:Option<usize>}
fn rich_source(palette:&lenz::theme::Palette,buf:&mut Buffer,area:Rect,text:&str,offset:usize,style:Style,decorations:Decorations<'_>) {
    let Decorations{inline,inline_color,hits,syntax,selection,caret}=decorations;
    if area.width == 0 { return; }
    let limit = lenz::document::LONG_LINE;
    let end = if text.len() <= limit { text.len() } else { text.char_indices().nth(limit).map_or(text.len(), |(i, _)| i) };
    let text = &text[..end];
    let mut column = 0;
    for (byte, grapheme) in text.grapheme_indices(true) {
        if column >= offset + area.width as usize { break; }
        let capped = lenz::document::cap_marks(grapheme);
        let display = if grapheme == "\t" { "    " }
            else if grapheme.chars().any(lenz::document::hides_text) { "�" } else { capped.as_ref() };
        let width = display.width().max(1);
        let end = column + width;
        if column >= offset && end <= offset + area.width as usize {
            let mut s = style;
            let capture=syntax.iter().find(|(r,_)|r.start<byte+grapheme.len()&&r.end>byte).map(|(_,c)|*c);
            if let Some(capture)=capture{s=s.patch(crate::visual::syntax_style(palette,capture));}
            // a changed word keeps its syntax color where that reads on the fill, as in the GUI
            if let Some(bg)=inline_color && overlaps(inline, byte, grapheme.len()) {
                s = s.bg(color(bg)).fg(color(capture.map_or(palette.fg,|c|palette.readable_on(palette.syntax_color(c),bg))));
            }
            if overlaps(hits, byte, grapheme.len()) { s = s.bg(color(palette.found_bg)).fg(color(palette.fg)); }
            if selection.as_ref().is_some_and(|r|r.start<byte+grapheme.len()&&r.end>byte){s=s.bg(color(palette.sel_bg));}
            if caret.is_some_and(|at| at >= byte && at < byte + grapheme.len()) { s = s.add_modifier(Modifier::REVERSED); }
            buf.set_stringn(area.x + (column - offset) as u16, area.y, display, width, s);
        }
        column = end;
    }
    if caret == Some(text.len()) && column >= offset && column < offset + usize::from(area.width) {
        buf.set_string(area.x + (column - offset) as u16, area.y, "▏", style.fg(color(palette.accent)));
    }
}

fn overlaps(ranges: &[Range<usize>], byte: usize, len: usize) -> bool {
    ranges.get(ranges.partition_point(|r| r.end <= byte)).is_some_and(|r| r.start < byte + len)
}

fn label(palette:&lenz::theme::Palette,frame: &mut ratatui::Frame, area: Rect, text: &str, style: Style) {
    if area.width == 0 || area.height == 0 { return; }
    source(palette,frame.buffer_mut(), Rect { height: 1, ..area }, text, 0, style, &[], &[]);
}

pub fn draw(app: &mut App, frame: &mut ratatui::Frame) {
    let palette=app.palette.clone();let palette=&*palette;
    let area = app.diff_area;
    app.size = (area.width, area.height);
    frame.buffer_mut().set_style(area, base(palette));
    if area.width == 0 || area.height < 4 {app.markdown.suspend();}
    if area.width == 0 || area.height == 0 { return; }
    let applied_rows=app.applied_rows().min(area.height.saturating_sub(4)as usize);
    let body_h = area.height.saturating_sub(4+app.applied_panel_height());
    app.session.resize(body_h);
    // what is compared, then the file's place in the Change Set
    // in Focus, Files' title names the selection while it shows
    let in_files = app.layout.focused(app.repository.as_ref().map(|r| &r.core)) && app.navigator.shown();
    let named = app.repository.as_ref().and_then(|r| r.core.selection_title()).filter(|_| !in_files).map_or_else(|| "Lenz".into(), |t| t.text);
    let title = format!(" {named}  {}/{}",app.session.current()+usize::from(!app.session.comparisons().is_empty()),app.session.comparisons().len());
    label(palette,frame, Rect::new(area.x, area.y, area.width, 1), &title, base(palette).fg(color(palette.accent)).add_modifier(Modifier::BOLD));
    if area.height < 4 { return; }
    let (left_name, right_name) = app.session.labels();
    let inline = app.markdown_inline();
    let unified = app.unified();
    let narrow = area.width < 70 || inline || unified;
    let left_w = if narrow { area.width } else { app.left_width() };
    // Inline Markdown is one full-width pane of the new side
    let panes = if inline || unified { vec![(Side::Right, Rect::new(area.x, area.y+2, area.width, body_h))] }
        else if narrow { vec![(app.focus, Rect::new(area.x, area.y+2, area.width, body_h))] }
        else { vec![(Side::Left, Rect::new(area.x, area.y+2, left_w, body_h)), (Side::Right, Rect::new(area.x+left_w + 1, area.y+2, area.width - left_w - 1, body_h))] };
    let rendered=app.session.markdown_mode()==lenz::markdown::PresentationMode::Rendered;
    let model = (!rendered).then(||app.session.view().frame());
    for &(side, pane) in &panes {
        let name = if side == Side::Left { left_name } else { right_name };
        let encoding = app.session.document().and_then(|d| d.encoding(side));
        let title = format!("{} {}{}", if side == app.focus { "▶" } else { " " }, name, encoding.map_or(String::new(), |e| format!(" · {e}")));
        label(palette,frame, Rect::new(pane.x, area.y+1, pane.width, 1), &title, base(palette).fg(if side == app.focus { color(palette.accent) } else { color(palette.dim) }));
        if app.visual.image().is_some() || rendered { continue; }
        let model=model.as_ref().expect("source frame");
        // Unified: both Sides' rows in this one pane, each drawn from its own Side
        let lists=if unified {vec![(Side::Left,&model.left),(Side::Right,&model.right)]} else {vec![(side,if side == Side::Left { &model.left } else { &model.right })]};
        let pane_side=side;
        for (side,rows) in lists {
        let lines = app.session.document().map(|d| d.lines_on(side)).unwrap_or_default();
        for row in rows {
            if row.y < 0.0 || row.y >= f32::from(body_h) { continue; }
            let y = pane.y + row.y as u16;
            let tint = color(row.tint.map_or(palette.bg, |(kind, ignored)| lenz::theme::change_color(palette,kind,ignored,&palette.block)));
            let style = base(palette).bg(tint);
            frame.buffer_mut().set_style(Rect::new(pane.x, y, pane.width, 1), style);
            match row.content {
                Content::Line(line) => {
                    let gutter = pane.width.min(7);
                    label(palette,frame, Rect::new(pane.x, y, gutter, 1), &format!("{:>5} ", line + 1), style.fg(color(palette.dim)));
                    if let Some(text) = lines.get(line as usize) {
                        let mut hits = app.session.find().filter(|f| f.side() == side).map(|f| {
                            let hits = f.hits();
                            let start = hits.partition_point(|h| h.0 < line);
                            hits[start..].iter().take_while(|h| h.0 == line && h.1.start < lenz::document::LONG_LINE * 4).map(|h| h.1.clone()).collect::<Vec<_>>()
                        }).unwrap_or_default();
                        if let Some(search)=&app.set_search{hits.extend(search.model.hits_on_line(app.session.current(),side,line).iter().map(|h|h.range.clone()));hits.sort_by_key(|r|r.start);}
                        let ann=app.visual.annotation(side,line as usize);
                        let ann_w=if app.visual.blame_enabled(side){annotation_width(pane.width,gutter)}else{0};
                        if ann_w>0{let title=ann.map_or_else(||"…".into(),|a|if !a.commit.is_committed(){"Not committed".into()}else{format!("{} {} {}",&a.commit.sha[..a.commit.sha.len().min(7)],lenz::blame::date(a.commit.author_time),a.commit.author)});label(palette,frame,Rect::new(pane.x+gutter,y,ann_w,1),&title,style.fg(color(palette.dim)));}
                        let syntax=app.visual.syntax(side,line as usize);
                        let selected=app.selection.filter(|s|s.side==side);
                        let selection=selected.and_then(|s|s.on_line(line,text.len()));
                        let caret=selected.filter(|s|s.head.line==line).map(|s|s.head.col);
                        let source=app.source_rect(side);
                        let inline_color=row.tint.map(|(kind,ignored)|lenz::theme::change_color(palette,kind,ignored,&palette.inline));
                        rich_source(palette,frame.buffer_mut(), Rect::new(source.x, y, source.width, 1), text, app.horizontal[app.pane_side(side) as usize], style, Decorations{inline:&row.inline,inline_color,hits:&hits,syntax:&syntax,selection,caret});
                    }
                }
                Content::Fold { hidden, .. } => label(palette,frame, Rect::new(pane.x, y, pane.width, 1), &format!("  ⋯ {hidden} unchanged lines (click / z)"), style.fg(color(palette.dim))),
                Content::Folded { left, right, .. } => label(palette,frame, Rect::new(pane.x, y, pane.width, 1), &format!("  ▸ {left} / {right} lines (x to unfold)"), style.fg(color(palette.accent))),
                Content::Filler => {}
            }
        }
        }
        // Sticky lines over the pane's top rows, syntax colours only, and a rule under them
        let sticky=app.sticky_lines();
        let pinned=&sticky[if unified {1} else {pane_side as usize}];
        let style=base(palette).bg(color(palette.header_bg));
        for s in pinned {
            let y=pane.y+s.row as u16;
            frame.buffer_mut().set_style(Rect::new(pane.x,y,pane.width,1),style);
            let gutter=pane.width.min(7);
            label(palette,frame,Rect::new(pane.x,y,gutter,1),&format!("{:>5} ",s.line+1),style.fg(color(palette.dim)));
            let lines=app.session.document().map(|d|d.lines_on(s.side)).unwrap_or_default();
            if let Some(text)=lines.get(s.line as usize){
                let syntax=app.visual.syntax(s.side,s.line as usize);
                let source=app.source_rect(s.side);
                rich_source(palette,frame.buffer_mut(),Rect::new(source.x,y,source.width,1),text,app.horizontal[app.pane_side(s.side) as usize],style,Decorations{inline:&[],inline_color:None,hits:&[],syntax:&syntax,selection:None,caret:None});
            }
        }
        if let Some(n)=pinned.iter().map(|s|s.row+1).max(){
            let y=pane.y+n as u16;
            if y<pane.y+body_h{frame.buffer_mut().set_string(pane.x,y,"─".repeat(pane.width as usize),base(palette).fg(color(palette.dim)));}
        }
    }
    if rendered {app.markdown_draw(frame.buffer_mut(),&panes);}
    let pane=app.pr_pane();
    if let Some(repo)=app.repository.as_mut()&&let Some(pr)=repo.core.pr.as_mut().filter(|p|p.details().is_some()){repo.pr_dialog.draw(frame,pr,pane,palette);}
    // the header row across both panes: the PR's tabs sit in its middle
    let row=panes.first().zip(panes.last()).map(|(&(_,a),&(_,b))|Rect::new(a.x,area.y+1,b.right().saturating_sub(a.x),1));
    header_switch(app,frame,row,palette);
    // a PR tab covers both panes: no split line over it
    if !narrow && app.visual.image().is_none() && !app.pr_details_shown() && app.session.folders.summary().is_none() {
        for y in area.y+1..area.y+area.height.saturating_sub(2) { frame.buffer_mut().set_string(area.x+left_w, y, "│", base(palette).fg(color(palette.dim))); }
    }
    if app.visual.image().is_some(){app.visual.draw_image(frame.buffer_mut(),Rect::new(area.x,area.y+2,area.width,body_h),palette);}
    // a Folder Summary covers the comparison behind it
    let summary=app.session.folders.summary().is_some();
    if summary&&let Some(set)=app.session.change_set(){let pane=app.pr_pane();app.summary.draw(frame,pane,&set,&app.session.folders,palette);}
    let placeholder = app.session.document().and_then(|d| d.placeholder_kind()).map(|p| match p {
        Placeholder::Binary { sizes, differ } => format!("Binary / image comparison: {} and {} bytes · {}. No decodable image.", sizes[0], sizes[1], if *differ { "different" } else { "identical" }),
        Placeholder::Large { bytes } => format!("Oversized comparison ({:.1} MiB). Enter to load.", *bytes as f64 / (1 << 20) as f64),
        Placeholder::Unreadable(e) => e.clone(),
    });
    if let Some(message) = placeholder.as_deref().filter(|_|app.visual.image().is_none()).or_else(|| app.session.loading().then_some("Loading comparison…  q quits; file navigation remains available")).filter(|_|!summary) {
        label(palette,frame, Rect::new(area.x+1, area.y+3.min(area.height - 1), area.width.saturating_sub(2), 1), message, base(palette).fg(color(palette.accent)));
    }
    for (i,record) in app.session.applied().iter().rev().take(if summary{0}else{applied_rows}).collect::<Vec<_>>().into_iter().rev().enumerate(){
        let message=format!("{} · {} · A history",record.label(),app.session.applied_undo_disabled(record).unwrap_or("[Undo] click or A"));label(palette,frame,Rect::new(area.x,area.y+3+body_h+i as u16,area.width,1),&message,base(palette).fg(color(palette.accent)));
    }
    // the separator only before a message
    let status=match status_text(app){text if text.is_empty()=>"F2 details".to_string(),text=>format!("F2 details · {text}")};
    label(palette,frame, Rect::new(area.x, area.y+area.height - 2, area.width, 1), &status, base(palette).fg(color(palette.accent)));
    let lists=app.navigator.focused||app.repository.as_ref().is_some_and(|r|r.core.picker.shown&&r.core.picker.focus==lenz::repository_session::Pane::Log);
    let footer=if lists{" Type to find  ↑/↓ or F3 matches  Enter open  Esc cancel  F10 commands  Ctrl-Q quit"}else{" n/p changes  [/] files  / find  f files  F10 commands  ? help  q quit"};
    // a full-width bar under the side panels: the panel switches, glued, at the left and the keys at the right
    let y=app.popup_area.bottom().saturating_sub(1);
    let bar=Rect::new(app.popup_area.x,y,app.popup_area.width,1);
    frame.buffer_mut().set_style(bar,base(palette).bg(color(palette.status_bg)));
    let area=bar;
    let panels=app.layout.panels(app.repository.as_ref().map(|r|&r.core));
    let mut cells=Vec::new();
    if app.repository.is_some(){cells.extend([("Branches",panels.branches,lenz::catalog::Action::Layout(lenz::layout::LayoutCommand::ToggleBranches)),("History",panels.history,lenz::catalog::Action::Layout(lenz::layout::LayoutCommand::ToggleHistory))]);}
    let files=app.icons.label(crate::icons::Icon::Files,"Files");
    cells.push((files.as_ref(),app.navigator.shown(),lenz::catalog::Action::Layout(lenz::layout::LayoutCommand::ToggleFiles)));
    if app.layout.focused(app.repository.as_ref().map(|r|&r.core)) && let Some(r)=app.repository.as_ref().filter(|r|r.core.message_panel().is_some()) {
        cells.push((lenz::catalog::COMMIT_PANEL_LABEL,!r.core.message_collapsed(),lenz::catalog::Action::Repository(lenz::catalog::RepoAction::ToggleMessage)));
    }
    let mut cells_drawn=Vec::new();
    let mut x=area.x+1.min(area.width);
    for (i,(name,on,command)) in cells.into_iter().enumerate() {
        if x>=area.right() { break; }
        if i>0 {frame.buffer_mut().set_string(x,y,"│",base(palette).bg(color(palette.status_bg)).fg(color(palette.dim)));x+=1;}
        if x>=area.right() { break; }
        let enabled=lenz::catalog::Availability::disabled(app,command).is_none();
        let style=if on {base(palette).bg(color(palette.selected_bg)).fg(color(palette.fg))} else {base(palette).bg(color(palette.status_bg)).fg(color(if enabled {palette.fg} else {palette.dim}))};
        let width=unicode_width::UnicodeWidthStr::width(name) as u16;
        let cell=Rect::new(x,y,width.min(area.right().saturating_sub(x)),1);
        frame.buffer_mut().set_stringn(x,y,name,usize::from(cell.width),style);
        cells_drawn.push((cell,command));
        x+=width;
    }
    app.panel_switch=cells_drawn;
    let keys=footer.trim_start();
    let room=area.right().saturating_sub(x+2);
    let at=area.right().saturating_sub((unicode_width::UnicodeWidthStr::width(keys) as u16).min(room)+1);
    label(palette,frame, Rect::new(at, y, area.right().saturating_sub(at), 1), keys, base(palette).bg(color(palette.status_bg)).fg(color(palette.dim)));
    // "F10 commands" opens the palette on a click, like the GUI's "Commands" (the toolbar has no button for it)
    app.palette_hint=keys.find("F10 commands").map(|i|{
        let x=at+keys.get(..i).map_or(0,unicode_width::UnicodeWidthStr::width) as u16;
        Rect::new(x,y,12.min(area.right().saturating_sub(x)),1)
    }).filter(|r|r.width==12).unwrap_or_default();
}

/// The find options that are on, as " [Aa W .*]" (empty when none is).
pub(crate) fn option_marks(o: lenz::find::FindOptions) -> String {
    let marks: Vec<&str> = [(o.case, "Aa"), (o.word, "W"), (o.regex, ".*")].into_iter().filter(|(on, _)| *on).map(|(_, m)| m).collect();
    if marks.is_empty() { String::new() } else { format!(" [{}]", marks.join(" ")) }
}

pub(crate) fn status_text(app:&App)->String {
    let rendered=app.session.markdown_mode()==lenz::markdown::PresentationMode::Rendered;
    let status = if let Some(prompt) = app.prompt.filter(|p| matches!(p, Prompt::Search | Prompt::Line)) {
        if prompt == Prompt::Search {
            let tail = app.session.find().map_or(String::new(), |f| f.counter());
            format!("Find{} {}  {tail}", option_marks(app.session.find_options()), app.input)
        } else { format!("Line: {}", app.input) }
    } else if let Some(notice) = app.session.notice().or_else(||app.visual.notice()) { notice.into() }
    else if let Some(find) = app.session.find() { format!("Find: {}  {}", find.query(), find.counter()) }
    else if !app.session.applied().is_empty(){format!("{} applied records · A history / Undo · Ctrl-Z undo · Alt-Shift-←/→ apply",app.session.applied().len())}
    else if let Some(doc) = app.session.document() {
        let mut notes = doc.notes().to_vec();
        if let Some((l, r)) = doc.line_endings() { notes.push(format!("Line endings differ: {l:?} / {r:?}")); }
        if doc.widest() >= lenz::document::LONG_LINE { notes.push("Long lines show their first 10,000 characters".into()); }
        notes.join(" · ")
    } else { String::new() };
    let status=if app.session.dirty(){format!("Unsaved changes{} · Ctrl-S Save · Discard unsaved in commands",if status.is_empty(){String::new()}else{format!(" · {status}")})}else if app.session.has_dirty(){format!("{} files have unsaved changes · {status}",app.session.dirty_files().len())}else{status};
    if app.session.markdown_loading()||rendered {
        let complete=!app.session.busy()
            &&app.session.markdown_projection().is_some_and(|projection|app.markdown.ready(projection));
        let presentation=if complete {"Rendered Markdown"}else{"Preparing rendered Markdown"};
        if status.is_empty(){presentation.into()}else{format!("{presentation} · {status}")}
    }else{status}
}

/// A PR's tabs in the middle of the header row; Source | Inline | Side by side, or the image modes, at its right end.
/// Clicks run them.
fn header_switch(app:&mut App,frame:&mut ratatui::Frame,row:Option<Rect>,palette:&lenz::theme::Palette){
    use unicode_width::UnicodeWidthStr;
    app.header_switch.clear();
    let Some(row)=row else{return;};
    let mut entries=lenz::catalog::header_switch(&app.scope());
    let total=|es:&[lenz::catalog::Entry]|es.iter().map(|e|e.label.width() as u16+3).sum::<u16>();
    // too narrow: the PR tabs give way first, then everything; the palette still lists them
    if total(&entries)+12>row.width{entries.retain(|e|!matches!(e.action,lenz::catalog::Action::Pr(_)));}
    if entries.is_empty()||total(&entries)+12>row.width{return;}
    let (tabs,modes):(Vec<_>,Vec<_>)=entries.into_iter().partition(|e|matches!(e.action,lenz::catalog::Action::Pr(_)));
    let modes_x=row.right()-total(&modes);
    let tabs_x=(row.x+(row.width.saturating_sub(total(&tabs)))/2).min(modes_x.saturating_sub(total(&tabs)+1)).max(row.x);
    for (mut x,group) in [(tabs_x,tabs),(modes_x,modes)] {
        for entry in group {
            let width=entry.label.width() as u16+2;let cell=Rect::new(x,row.y,width,1);
            let style=if entry.current{base(palette).bg(color(palette.selected_bg)).fg(color(palette.fg))}else{base(palette).fg(color(palette.dim))};
            frame.buffer_mut().set_style(cell,style);label(palette,frame,cell,&format!(" {} ",entry.label),style);
            app.header_switch.push((cell,entry.action));x+=width+1;
        }
    }
}

pub(crate) fn draw_popup(app:&mut App,frame:&mut ratatui::Frame){
    let palette=app.palette.clone();let palette=&*palette;
    if let Some(about)=&mut app.about {about.draw(frame,app.popup_area,palette);return;}
    if let Some(prompt) = app.prompt.filter(|p| matches!(p, Prompt::Files | Prompt::Help | Prompt::Applied)) {
        if prompt==Prompt::Help {let rows=crate::help::COMPARISON.iter().chain(if app.repository.is_some(){crate::help::REPOSITORY}else{&[]}).copied().collect::<Vec<_>>();crate::help::draw(frame,app.popup_area,&rows,&mut app.selected,palette);}else{popup(app, frame, prompt);}
    }
}

fn popup(app: &mut App, frame: &mut ratatui::Frame, prompt: Prompt) {
    let owned_palette=app.palette.clone();let palette=&*owned_palette;
    let area = app.popup_area;
    let w = area.width.saturating_sub(4).min(110);
    let h = area.height.saturating_sub(4).min(20);
    app.popup_hits.clear();app.popup_rect=Rect::default();
    if w < 4 || h < 3 { return; }
    let rect = Rect::new(area.x+(area.width - w) / 2, area.y+(area.height - h) / 2, w, h);
    app.popup_rect=rect;
    frame.render_widget(Clear, rect);
    frame.render_widget(Block::default().borders(Borders::ALL).style(base(palette)), rect);
    let rows=popup_rows(app,prompt);
    let heading = format!(" {}{}", if prompt == Prompt::Files { "Files: " } else { "Applied history: " }, app.input);
    label(palette,frame, Rect::new(rect.x + 1, rect.y, rect.width - 2, 1), &heading, base(palette).fg(color(palette.accent)));
    let count = h.saturating_sub(2) as usize;
    let start = app.selected.saturating_sub(count.saturating_sub(1));
    for (i, row) in rows.iter().enumerate().skip(start).take(count) {
        let line = Rect::new(rect.x + 1, rect.y + 1 + (i - start) as u16, rect.width - 2, 1);
        app.popup_hits.push((i,line));
        let style = if i == app.selected && prompt != Prompt::Help { base(palette).bg(color(palette.selected_bg)) } else { base(palette) };
        frame.render_widget(Paragraph::new(" ").style(style), line);
        label(palette,frame, line, row, style);
    }
}

pub(crate) fn popup_rows(app:&App,prompt:Prompt)->Vec<String> {
    match prompt {
        Prompt::Applied=>app.session.applied().iter().map(|r|format!("{} · {}",r.label(),app.session.applied_undo_disabled(r).unwrap_or("Enter to Undo"))).collect(),
        Prompt::Files => app.files().iter().map(|&i| format!("{} {}", navigator_badge(app, i), app.session.comparisons()[i].path)).collect(),
        Prompt::Search | Prompt::Line | Prompt::Help => unreachable!(),
    }
}

fn navigator_badge(app: &App, index: usize) -> &'static str { lenz::navigator::badge(&app.session.comparisons()[index].status) }

/// Same projection as rich_source: capped text, grapheme widths and four-column tabs.
pub(crate) fn source_width(lines:&[String])->usize{
    lines.iter().map(|text|{
        if text.is_ascii(){let text=&text[..text.len().min(lenz::document::LONG_LINE)];return text.len()+3*text.bytes().filter(|b|*b==b'\t').count();}
        let end=text.char_indices().nth(lenz::document::LONG_LINE).map_or(text.len(),|(i,_)|i);
        text[..end].graphemes(true).map(|g|if g=="\t"{4}else if g.chars().any(lenz::document::hides_text){1}else{g.width().max(1)}).sum()
    }).max().unwrap_or(0)
}


#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::{Terminal, backend::TestBackend};

    fn ready(app: &mut App) {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            app.poll();
            if !app.busy() { break; }
            assert!(std::time::Instant::now() < until);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    #[test]
    fn image_headers_identify_switched_files_and_honor_the_name_option() {
        let dir = tempfile::tempdir().unwrap();
        let old = br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#f00"/></svg>"##;
        let new = br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#00f"/></svg>"##;
        for (side, bytes) in [("left", old.as_slice()), ("right", new.as_slice())] {
            std::fs::create_dir(dir.path().join(side)).unwrap();
            for name in ["first.svg", "second.svg"] {
                std::fs::write(dir.path().join(side).join(name), bytes).unwrap();
            }
        }
        let args = vec!["--no-index".into(), "left".into(), "right".into()];
        let request = lenz::cli::parse(&args, dir.path()).unwrap();
        let mut app = App::new(lenz::session::Session::open(request, dir.path().into()));
        ready(&mut app);
        assert_eq!(app.session.comparisons().len(), 2);
        let mut terminal = Terminal::new(TestBackend::new(160, 20)).unwrap();
        let header = |terminal: &Terminal<TestBackend>,y:u16| terminal.backend().buffer().content[160*usize::from(y)..160*usize::from(y+1)].iter().map(|cell| cell.symbol()).collect::<String>();
        terminal.draw(|frame| app.render(frame)).unwrap();
        assert!(app.visual.image().is_some());
        let (left, right) = app.session.labels();
        assert!(header(&terminal,app.diff_area.y+1).contains(left));
        assert!(header(&terminal,app.diff_area.y+1).contains(right));
        assert!(header(&terminal,app.diff_area.y+1).contains("first.svg"));

        app.handle(Event::Key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE)));
        ready(&mut app);
        terminal.draw(|frame| app.render(frame)).unwrap();
        assert!(app.visual.image().is_some());
        assert!(header(&terminal,app.diff_area.y+1).contains("second.svg"));
        assert!(!header(&terminal,app.diff_area.y+1).contains("first.svg"));

        let mut named_args = vec!["--name".into(), "Image review λ".into()];
        named_args.extend(args);
        let request = lenz::cli::parse(&named_args, dir.path()).unwrap();
        let mut named = App::new(lenz::session::Session::open(request, dir.path().into()));
        ready(&mut named);
        terminal.draw(|frame| named.render(frame)).unwrap();
        assert!(named.visual.image().is_some());
        assert_eq!(header(&terminal,named.diff_area.y+1).matches("Image review λ").count(), 2);
    }

    #[test]
    fn source_geometry_accounts_for_blame_narrow_views_and_nested_origins() {
        let area = Rect::new(10, 4, 120, 20);
        assert_eq!(text_geometry(area, Side::Left, Side::Right, false), Rect::new(17, 6, 53, 16));
        assert_eq!(text_geometry(area, Side::Left, Side::Right, true), Rect::new(43, 6, 27, 16));
        assert_eq!(text_geometry(area, Side::Right, Side::Right, true), Rect::new(104, 6, 26, 16));
        let narrow = Rect::new(10, 4, 69, 20);
        assert_eq!(text_geometry(narrow, Side::Right, Side::Right, true), Rect::new(43, 6, 36, 16));
        assert_eq!(text_geometry(narrow, Side::Left, Side::Right, false).width, 0);
        assert_eq!(text_geometry(Rect::new(0, 0, 70, 20), Side::Left, Side::Left, true).width, 4);
    }

    #[test]
    fn source_highlight_priority_and_syntax_match_the_gui() {
        for theme in lenz::theme::builtin_entries() {
            let palette=&theme.palette;
            let area=Rect::new(0,0,6,1);let mut buf=Buffer::empty(area);
            let syntax=[(0..6,lenz::highlight::Capture::Keyword)];
            let inline=1..5;let hits=2..4;
            rich_source(palette,&mut buf,area,"abcdef",0,base(palette),Decorations {
                inline:std::slice::from_ref(&inline),inline_color:Some(palette.inline[2]),hits:std::slice::from_ref(&hits),syntax:&syntax,selection:Some(3..4),caret:None,
            });
            for (x,bg,fg) in [(0,palette.bg,palette.syntax.keyword),(1,palette.inline[2],palette.readable_on(palette.syntax.keyword,palette.inline[2])),(2,palette.found_bg,palette.fg),(3,palette.sel_bg,palette.fg)] {
                assert_eq!(buf[(x,0)].bg,color(bg),"{}",theme.id);assert_eq!(buf[(x,0)].fg,color(fg));
                assert!(!buf[(x,0)].modifier.contains(Modifier::UNDERLINED));
            }
        }
    }

    #[test]
    fn caret_on_empty_lines_and_clipped_cells_stays_inside_source_viewport() {
        let area = Rect::new(3, 1, 6, 1);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 12, 3));
        let decorate = |caret| Decorations { inline: &[], inline_color:None, hits: &[], syntax: &[], selection: None, caret: Some(caret) };
        rich_source(crate::chrome::palette(),&mut buffer, area, "", 0, base(crate::chrome::palette()), decorate(0));
        assert_eq!(buffer[(3, 1)].symbol(), "▏");
        assert_eq!(buffer[(2, 1)].symbol(), " ");

        buffer.reset();
        rich_source(crate::chrome::palette(),&mut buffer, area, "a界\tb", 3, base(crate::chrome::palette()), decorate(0));
        assert!(!buffer.content.iter().any(|cell| cell.modifier.contains(Modifier::REVERSED)), "clipped caret must not appear at a different column");
        rich_source(crate::chrome::palette(),&mut buffer, area, "a界\tb", 3, base(crate::chrome::palette()), decorate("a界\tb".len()));
        assert_eq!(buffer[(8, 1)].symbol(), "▏");
        assert_eq!(buffer[(9, 1)].symbol(), " ");
    }

    #[test]
    fn clicked_unicode_caret_and_line_end_are_visible_in_the_comparison() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("left"), "a界\tb\n").unwrap();
        std::fs::write(dir.path().join("right"), "different\n").unwrap();
        let request = lenz::cli::parse(&["--no-index".into(), "left".into(), "right".into()], dir.path()).unwrap();
        let mut app = App::new(lenz::session::Session::open(request, dir.path().into()));
        ready(&mut app);
        let mut terminal = Terminal::new(TestBackend::new(120, 12)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        app.handle(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left), column: 8, row: app.diff_area.y+2,
            modifiers: KeyModifiers::NONE,
        }));
        terminal.draw(|frame| app.render(frame)).unwrap();
        let y=app.diff_area.y+2;let cell = &terminal.backend().buffer()[(8, y)];
        assert_eq!(cell.symbol(), "界");
        assert!(cell.modifier.contains(Modifier::REVERSED), "bare caret must be visible without a selected range");

        app.handle(Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)));
        terminal.draw(|frame| app.render(frame)).unwrap();
        assert_eq!(terminal.backend().buffer()[(15, y)].symbol(), "▏", "EOL caret follows Unicode and expanded tab cells");
        assert!(!terminal.backend().buffer()[(8, y)].modifier.contains(Modifier::REVERSED));
    }
}
