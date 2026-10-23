//! Context-specific shortcut tables, wrapped and scrolled in terminal cells.
use ratatui::{Frame,layout::Rect,style::Modifier,widgets::{Block,Borders,Clear}};
use unicode_width::UnicodeWidthStr;
use crate::chrome::{color,style_with};
pub type Row=(&'static str,&'static str);
pub const COMPARISON:&[Row]=&[
    ("","Comparison"),("n / p, F7 / Shift-F7","Next / previous change"),("[ / ]","Previous / next file"),
    ("Arrows, j / k","Scroll focused side"),("PgUp / PgDn, Home / End","Page / jump to start or end"),("Tab / Shift-Tab","Focus next / previous visible pane"),
    ("z / x / X","Context / fold change / unfold all"),("s / w / h / i","Scroll / whitespace / highlight / ignore case"),
    ("","Navigation and search"),(": / F10 / Alt-F10","Command palette / focus toolbar"),("☰ / ⚙ / PR / ⋯","App menu / diff options / pull request / all commands"),("F6","Cycle repository, log, files and comparison"),
    ("Alt-1 / Alt-2 / Alt-3","Show / hide Files / Branches / History"),("Alt-0, Esc","Focus: the diff alone / restore the panels"),("Alt-Left / Alt-Right","Back / forward (repository views)"),("f / Ctrl-P","Find file (name:line)"),("g","Go to line or line:column"),
    ("/ / Ctrl-F","Find in focused side"),("F3 / Shift-F3","Next / previous match"),("Ctrl-Shift-F","Find across Change Set (Alt-O: changed lines only)"),("Alt-C / Alt-W / Alt-R","Find options: match case / whole word / regex"),
    ("","Selection and editing"),("Click / drag","Select text; double/triple click word/syntax group"),("Shift-arrows / Ctrl-A","Extend selection / select all"),
    ("Right click / Shift-F10","Text, file or commit actions"),("y / Ctrl-Shift-C","Copy selection via terminal OSC 52"),("Alt-Shift-Left/Right","Apply hunk to draft on left / right"),("Ctrl-Z / Ctrl-Y","Undo / redo draft edits"),("Ctrl-S","Save current draft to disk"),("Discard unsaved","Discard draft; keep saved file"),("Revert hunk","Restore selected right hunk from left into draft"),("A","Applied history and selective Undo"),
    ("","Images"),("m","Cycle 2-up, Swipe, Onion skin, Difference"),("0 / 1, + / -","Fit / actual size, zoom in / out"),("Arrows / , .","Pan / adjust opacity or split"),
    ("F2","Full status or selected action details; arrows scroll"),("Nerd Font icons","Optional ☰ setting; plain labels stay visible"),("","Other actions"),("b / B","Blame / inspect commit"),(": Sticky Scroll","Pin the enclosing declarations at the top of each pane; click one to jump"),("o / O","Open editor / default application"),("F5","Reload comparison"),(":","All commands and disabled reasons"),
    ("F8 / Shift-F8","Switch independent views"),("q / Ctrl-C","Close current view / quit"),
];
pub const REPOSITORY:&[Row]=&[
    ("","Repository panes"),("F10 / F6","Command palette / cycle sources, commits, files, diff"),("Ctrl-R","Show / hide repository picker"),
    ("Ctrl-S / Ctrl-L","In Sources: toggle sources / toggle log"),("Tab / Shift-Tab","Move between picker panes and files"),
    ("","Sources"),("Arrows / Enter","Select and open source"),("/ / Esc","Filter sources / clear filter"),("c / C","Compare branch merge base / tips"),
    ("o / Shift-F10","Open worktree in new view / row's actions (Checkout…)"),("a / b","All / current branch history"),("s","Staged changes"),
    ("","Commits/Files"),("Type / Ctrl-F","Find literal text; keep every row visible"),("Up/Down, F3/Shift-F3","Step matches without opening a comparison"),
    ("Enter / Esc","Open candidate / cancel and restore position"),("Ctrl-A / Ctrl-C","Clear query / copy candidate hash or path"),("PgUp/PgDn, Home/End","Scroll list without opening a comparison"),
    ("Shift-arrows / Ctrl-arrows","Range / second commit outside search"),("F10 / Shift-F10","Commands / candidate context actions"),("Alt-F10","Focus toolbar; arrows choose; Enter opens"),
    ("","Pull requests (read-only)"),("P / r","Open number or URL / refresh"),("d / v","Description / local Viewed mark"),("t / T","Current / outdated review threads"),("h","Check remote head"),
    ("F2","Full status or selected command details"),("","Other actions"),(":refresh-repository","Reload sources and history"),(": / ?","Commands / keyboard shortcuts"),("Ctrl-Q","Close view / quit; q also closes from diff"),
];
pub const MERGE:&[Row]=&[
    ("","Merge choices"),("1 / 2 / 3 / 4 / 5","Ours / Theirs / Both / Base / Reopen"),("Right click / Shift-F10","Choices for the change under the pointer"),("a / m","Apply non-conflicting / resolve simple"),("Ctrl-Z / Ctrl-Y","Undo / redo"),
    ("","Files"),("Type / Ctrl-F","Find paths without switching the open file"),("Up/Down, F3/Shift-F3","Step matches; Enter opens; Esc cancels"),("Ctrl-A / Ctrl-C","Clear query / copy candidate path"),("PgUp/PgDn, Home/End","Scroll list without opening a file"),("F10 / Shift-F10","Commands / candidate context actions"),
    ("","Navigation"),("n / p","Next / previous change"),("N / P","Next / previous unresolved conflict"),("[ / ]","Previous / next file"),("b","Toggle Base / Result"),("w / s","Whitespace / shared or padded scroll"),
    ("/ / F3 / Shift-F3","Find / next / previous match"),("Tab","Focus next pane"),("F4","Full nested comparison commands"),("F10 / : / Alt-F10","Command palette / focus toolbar"),
    ("","Saving and conflicts"),("Ctrl-S / Ctrl-Enter","Save / finish"),("Esc / q","Cancel or dismiss / close"),("r","Reload changed file"),("Ctrl-Shift-R","Start over"),
    ("M / R","Stage saved file / reopen conflict"),("o","Open closed merge"),("Ctrl-R","Return to repository"),("c / C","Copy continuation / submodule command"),
    ("F2","Full recovery or selected command details"),("","Versions and views"),("Digits","Take complete version, including deletion"),("!","1:1 image in Version Picker"),("F8 / Shift-F8","Switch independent views"),(": / ?","Commands / keyboard shortcuts"),
];
fn lines(rows:&[Row],width:usize)->Vec<(String,bool)> {
    let column=if width>=60{25}else{18};let mut out=vec![(if width>=40{format!("{:<column$} Action","Keys")}else{"Keys / Action".into()},true)];
    for &(key,action) in rows {
        if key.is_empty(){out.push((String::new(),false));out.push((action.into(),true));continue;}
        let stacked=width<40||key.width()>column;
        if stacked{out.push((key.into(),true));}
        let indent=if stacked{2}else{column+1};let available=width.saturating_sub(indent).max(1);
        let mut chunk=String::new();let mut first=true;
        for word in action.split_whitespace() {
            if !chunk.is_empty()&&chunk.width()+1+word.width()>available {
                out.push((format!("{}{}",if !stacked&&first{format!("{key:<column$} ")}else{" ".repeat(indent)},chunk),false));chunk.clear();first=false;
            }
            if !chunk.is_empty(){chunk.push(' ');}chunk.push_str(word);
        }
        out.push((format!("{}{}",if !stacked&&first{format!("{key:<column$} ")}else{" ".repeat(indent)},chunk),false));
    }
    out
}

/// A snapshot of a clipped notice or action reason; wrapping is lazy and cached.
pub(crate) struct Message { text:String, rows:Vec<String>, width:u16, scroll:usize, area:Rect }
impl Message {
    pub(crate) fn new(text:String,area:Rect)->Self {
        let mut dialog=Self{text,rows:Vec::new(),width:0,scroll:0,area};dialog.prepare(area);dialog
    }
    fn popup(area:Rect)->Rect {
        let inset_x=u16::from(area.width>2);let inset_y=u16::from(area.height>2);
        Rect::new(area.x.saturating_add(inset_x),area.y.saturating_add(inset_y),area.width.saturating_sub(2*inset_x),area.height.saturating_sub(2*inset_y))
    }
    fn prepare(&mut self,area:Rect) {
        self.area=area;let width=Self::popup(area).width.saturating_sub(2).max(1);
        if self.width!=width {self.rows.clear();crate::pr_details::wrap_into(&self.text,width,&mut self.rows);self.width=width;}
        self.scroll=self.scroll.min(self.max());
    }
    fn max(&self)->usize {self.rows.len().saturating_sub(usize::from(Self::popup(self.area).height.saturating_sub(3)))}
    pub(crate) fn handle(&mut self,event:&crossterm::event::Event)->bool {
        use crossterm::event::{Event,KeyCode,KeyEventKind,MouseEventKind};
        if let Event::Resize(w,h)=event {self.prepare(Rect::new(0,0,*w,*h));}else{self.prepare(self.area);}
        let page=usize::from(Self::popup(self.area).height.saturating_sub(4)).max(1);
        match event {
            Event::Key(k) if k.kind!=KeyEventKind::Release=>match k.code {
                KeyCode::Esc|KeyCode::Enter|KeyCode::F(2)=>return true,
                KeyCode::Home=>self.scroll=0,KeyCode::End=>self.scroll=self.max(),
                KeyCode::Up=>self.scroll=self.scroll.saturating_sub(1),KeyCode::Down=>self.scroll=self.scroll.saturating_add(1),
                KeyCode::PageUp=>self.scroll=self.scroll.saturating_sub(page),KeyCode::PageDown=>self.scroll=self.scroll.saturating_add(page),_=>{}
            },
            Event::Mouse(m)=>match m.kind{MouseEventKind::ScrollUp=>self.scroll=self.scroll.saturating_sub(3),MouseEventKind::ScrollDown=>self.scroll=self.scroll.saturating_add(3),_=>{}},_=>{}
        }
        self.scroll=self.scroll.min(self.max());false
    }
    pub(crate) fn draw(&mut self,frame:&mut Frame,area:Rect,palette:&lenz::theme::Palette) {
        self.prepare(area);let popup=Self::popup(area);if popup.width<3||popup.height<4{return;}
        let style=style_with(palette).bg(color(palette.status_bg));
        frame.render_widget(Clear,popup);
        frame.render_widget(Block::default().borders(Borders::ALL).title(" Details ").style(style),popup);
        for (i,row) in self.rows.iter().skip(self.scroll).take(usize::from(popup.height.saturating_sub(3))).enumerate(){crate::visual::safe_text(frame.buffer_mut(),Rect::new(popup.x+1,popup.y+1+i as u16,popup.width-2,1),row,style);}
        crate::visual::safe_text(frame.buffer_mut(),Rect::new(popup.x+1,popup.bottom()-2,popup.width-2,1),"↑/↓ PgUp/PgDn Home/End · Esc close",style.fg(color(palette.accent)));
    }
}
/// The shortcut table's box in `area`.
pub(crate) fn rect(area:Rect)->Rect {
    let w=area.width.saturating_sub(4).min(96);let h=area.height.saturating_sub(2).min(30);
    Rect::new(area.x+(area.width-w)/2,area.y+(area.height-h)/2,w,h)
}
pub fn draw(frame:&mut Frame,area:Rect,rows:&[Row],scroll:&mut usize,palette:&lenz::theme::Palette) {
    if area.width<8||area.height<5{return;}
    let rect=rect(area);let (w,h)=(rect.width,rect.height);
    let entries=lines(rows,w.saturating_sub(4)as usize);let visible=h.saturating_sub(4)as usize;
    *scroll=(*scroll).min(entries.len().saturating_sub(visible));
    frame.render_widget(Clear,rect);
    frame.render_widget(Block::default().borders(Borders::ALL).title(" Keyboard shortcuts ").style(style_with(palette).bg(color(palette.status_bg))),rect);
    for (i,(text,heading)) in entries.iter().skip(*scroll).take(visible).enumerate(){
        let row=Rect::new(rect.x+2,rect.y+1+i as u16,w.saturating_sub(4),1);
        let s=style_with(palette).bg(color(palette.status_bg)).fg(color(if *heading{palette.accent}else{palette.fg})).add_modifier(if *heading{Modifier::BOLD}else{Modifier::empty()});
        crate::visual::safe_text(frame.buffer_mut(),row,text,s);
    }
    let footer=Rect::new(rect.x+1,rect.bottom().saturating_sub(2),w.saturating_sub(2),1);
    crate::visual::safe_text(frame.buffer_mut(),footer,"↑/↓ scroll · PgUp/PgDn · Esc close",style_with(palette).bg(color(palette.status_bg)).fg(color(palette.dim)));
}
