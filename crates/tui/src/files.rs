//! Persistent GUI-style File Navigator; build grouped rows only when the Change Set changes.
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use lenz::{changeset::ChangeSet,folders::Folders,navigator::{self,NavRow}};
use ratatui::{Frame,layout::Rect,style::Modifier,widgets::Paragraph};
use std::sync::{Arc,Weak};
use crate::chrome::{color,style_with};

/// What a Files event asks the app to do.
#[derive(Debug,Clone,PartialEq)]
pub enum Pick {
    File(usize),
    /// A folder heading's arrow: collapse or expand.
    Toggle(String),
    /// A folder heading's name: its summary; `group` is the heading's Change Set group.
    Summary{folder:String,group:Option<usize>},
    /// Left on a file: the summary of its folder.
    FolderOf(usize),
}

#[derive(Default)]
pub struct Files {
    pub(crate) icons:crate::icons::Icons,
    set:Weak<ChangeSet>,revision:u64,rows:Vec<NavRow>,current:usize,
    /// Whether a summary is shown, and its cached visible heading row.
    summary:bool,summary_row:Option<usize>,selected:usize,scroll:usize,reveal_current:bool,
    pub(crate) search:lenz::pane_search::PaneSearch,search_origin:Option<(usize,usize)>,
    pub focused:bool,pub visible:Option<bool>,pub(crate) area:Rect,available:Rect,
    /// Replaces "Files (N)" as the box title (Focus names the selection there).
    pub(crate) heading:Option<String>,
    /// Hidden for now without changing whether Files is on (a PR's Conversation and Commits).
    pub(crate) suppressed:bool,
    pub(crate) desired_width:Option<f32>,pub(crate) resizer:crate::panes::Resizer,
}
impl Files {
    pub fn prepare(&mut self,set:Option<Arc<ChangeSet>>,folders:&Folders,current:usize,repository_shown:bool,area:Rect)->Rect {
        let resized=self.available.height!=area.height;
        self.available=area;
        if let Some(set)=set {
            self.sync(&set,folders,current);
            if self.current!=current {self.selected=current;self.reveal_current=true;if self.search.active(){self.search_origin=Some((current,self.scroll));}}self.current=current;
            let show=!self.suppressed&&self.visible.unwrap_or(repository_shown||set.comparisons.len()>1);
            let width=if show&&area.width>=50&&area.height>=5{crate::panes::size(area.width,self.desired_width.unwrap_or(f32::from((area.width/4).clamp(18,30))),18.0,32.0)}else{0};
            if resized{self.reveal_current=true;}
            self.area=Rect::new(area.x,area.y,width,area.height);
        } else {self.rows.clear();self.summary=false;self.summary_row=None;self.set=Weak::new();self.search.cancel();self.search_origin=None;self.area=Rect::new(area.x,area.y,0,0);}
        self.resizer.grips.clear();
        if self.area.width>0 {self.resizer.grips.push(crate::panes::Grip::vertical(Rect::new(self.area.right()-1,self.area.y,1,self.area.height),0,area.x));}
        if self.area.width==0{self.focused=false;}
        self.poll_search();
        Rect::new(area.x+self.area.width,area.y,area.width-self.area.width,area.height)
    }
    /// Rows and the summary heading as the session has them now: before input as well as before drawing, so keys
    /// pressed between frames act on what they see.
    pub fn sync(&mut self,set:&Arc<ChangeSet>,folders:&Folders,current:usize){
        self.summary=folders.summary().is_some();
        if !self.set.ptr_eq(&Arc::downgrade(set)) {
            self.rows=folders.rows(set);self.summary_row=folders.summary_row(set,&self.rows);self.revision=folders.revision();self.set=Arc::downgrade(set);
            self.selected=current;self.scroll=0;self.reveal_current=true;self.search.invalidate();self.search_origin=Some((current,0));
        } else if self.revision!=folders.revision() {
            // a folder collapsed or expanded: same Change Set, other rows
            self.rows=folders.rows(set);self.summary_row=folders.summary_row(set,&self.rows);self.revision=folders.revision();self.reveal_current=true;
            self.scroll=self.scroll.min(self.rows.len().saturating_sub(1));
        }
    }
    pub fn resize_input(&mut self,event:&Event,blocked:bool)->bool {match self.resizer.handle(event,blocked){crate::panes::Resize::None=>false,crate::panes::Resize::Captured=>true,crate::panes::Resize::Move(_,width)=>{self.desired_width=Some((width+1.0).max(0.0));true}}}
    pub fn file_at(&self,column:u16,row:u16)->Option<usize>{
        if !self.shown()||!self.area.contains((column,row).into())||row<=self.area.y||row>=self.area.bottom().saturating_sub(1){return None;}
        match self.rows.get(self.scroll+usize::from(row-self.area.y-1)){Some(NavRow::File{index,..})=>Some(*index),_=>None}
    }
    pub fn can_show(&self)->bool{self.available.width>=50&&self.available.height>=5}
    pub fn shown(&self)->bool {self.area.width>0}
    pub(crate) fn candidate(&self)->usize {self.selected}
    /// The keyboard is on the last file row (not a heading's summary): ↓ from it enters the commit panel.
    pub(crate) fn on_last_file(&self)->bool {!self.summary&&self.rows.iter().rev().find_map(|r|if let NavRow::File{index,..}=r{Some(*index)}else{None})==Some(self.selected)}
    /// End a file search as Esc does, back at the accepted selection.
    pub(crate) fn cancel_search(&mut self){
        if !self.search.active(){return;}
        if let Some((selected,scroll))=self.search_origin.take(){self.selected=selected;self.scroll=scroll;}
        self.search.cancel();self.reveal_current=false;
    }
    pub(crate) fn copy_path(&self)->Option<String> {self.set.upgrade()?.comparisons.get(self.selected).map(|c|c.path.clone())}
    pub(crate) fn busy(&self)->bool {self.search.searching(self.set.upgrade().map_or(0,|s|s.comparisons.len()))}
    fn begin_search(&mut self) {
        if !self.search.active(){self.search_origin=Some((self.selected,self.scroll));self.search.start(self.selected);}
        self.reveal_current=false;
    }
    pub(crate) fn poll_search(&mut self)->bool {
        let Some(set)=self.set.upgrade()else{return false;};
        let before=self.search.current();
        let changed=self.search.poll(set.comparisons.len(),|i,q|lenz::pane_search::contains(&set.comparisons[i].path,q));
        if let Some(index)=self.search.current(){self.selected=index;if before!=Some(index){self.reveal_file(index);}}
        changed
    }
    fn reveal_file(&mut self,index:usize) {if let Some(row)=self.rows.iter().position(|r|matches!(r,NavRow::File{index:i,..}if *i==index)){self.reveal(row);}}
    /// The Change Set group of row `row`: how many group headings precede it, less one.
    fn group_of(&self,row:usize)->Option<usize>{self.rows[..row].iter().filter(|r|matches!(r,NavRow::Group(..))).count().checked_sub(1)}
    /// The row the keyboard is on: the shown summary's heading, else the selected file.
    fn cursor(&self)->Option<usize>{
        if self.summary {self.summary_row}else{self.rows.iter().position(|r|matches!(r,NavRow::File{index,..}if *index==self.selected))}
    }
    /// What choosing row `row` does: a file opens, a heading shows its summary.
    fn pick(&self,row:usize)->Option<Pick>{
        match self.rows.get(row)? {
            NavRow::File{index,..}=>Some(Pick::File(*index)),
            NavRow::Dir{path,..}=>Some(Pick::Summary{folder:path.clone(),group:self.group_of(row)}),
            NavRow::Group(..)=>None,
        }
    }
    pub fn handle(&mut self,event:&Event)->(bool,Option<Pick>) {
        if !self.shown(){return(false,None);}
        // the keys walk files and folder headings, as a tree: group headings are only labels
        let walkable:Vec<usize>=(0..self.rows.len()).filter(|&r|!matches!(self.rows[r],NavRow::Group(..))).collect();
        let at=self.cursor().and_then(|row|walkable.iter().position(|&r|r==row));
        let step=|delta:isize|->Option<usize>{walkable.get(at.map_or(0,|a|a.saturating_add_signed(delta).min(walkable.len().saturating_sub(1)))).copied()};
        match event {
            Event::Mouse(m) if self.area.contains((m.column,m.row).into())=>{
                match m.kind {
                    MouseEventKind::ScrollUp=>self.scroll=self.scroll.saturating_sub(3),
                    MouseEventKind::ScrollDown=>self.scroll=(self.scroll+3).min(self.rows.len().saturating_sub(self.area.height.saturating_sub(2)as usize)),
                    MouseEventKind::Down(MouseButton::Left)=>{
                        self.focused=true;
                        let row=self.scroll+usize::from(m.row.saturating_sub(self.area.y+1));
                        if m.row>self.area.y&&m.row<self.area.bottom().saturating_sub(1) {
                            match self.rows.get(row) {
                                Some(NavRow::File{index,..})=>{self.selected= *index;self.search_origin=Some((self.selected,self.scroll));return(true,Some(Pick::File(*index)));}
                                // the arrow (its first two cells) collapses or expands; the name shows the summary
                                Some(NavRow::Dir{path,..}) if m.column<self.area.x+3=>return(true,Some(Pick::Toggle(path.clone()))),
                                Some(NavRow::Dir{path,..})=>return(true,Some(Pick::Summary{folder:path.clone(),group:self.group_of(row)})),
                                _=>{}
                            }
                        }
                    }
                    _=>{}
                }
                (true,None)
            }
            Event::Paste(text) if self.focused=>{self.begin_search();self.search.append(&lenz::document::one_line(text));self.poll_search();(true,None)}
            Event::Key(k) if self.focused&&k.kind!=KeyEventKind::Release=>{
                let ctrl=k.modifiers==KeyModifiers::CONTROL;
                if ctrl&&k.code==KeyCode::Char('a'){self.begin_search();self.search.set_query(String::new());return(true,None);}
                if ctrl&&k.code==KeyCode::Char('f')&&!k.modifiers.contains(KeyModifiers::SHIFT) {self.begin_search();return(true,None);}
                if k.modifiers.difference(KeyModifiers::SHIFT).is_empty()&&let KeyCode::Char(ch)=k.code {
                    self.begin_search();self.search.append(&ch.to_string());self.poll_search();return(true,None);
                }
                if self.search.active() {
                    match k.code {
                        KeyCode::Up|KeyCode::Down|KeyCode::F(3)=>{
                            self.poll_search();let forward=k.code==KeyCode::Down||k.code==KeyCode::F(3)&&!k.modifiers.contains(KeyModifiers::SHIFT);
                            if let Some(index)=self.search.step(forward){self.selected=index;self.reveal_file(index);}return(true,None);
                        }
                        KeyCode::Backspace=>{self.search.backspace();self.poll_search();return(true,None);}
                        KeyCode::Enter=>{if let Some(index)=self.search.confirm(){self.selected=index;self.search_origin=Some((index,self.scroll));return(true,Some(Pick::File(index)));}return(true,None);}
                        KeyCode::Esc=>{if let Some((selected,scroll))=self.search_origin.take(){self.selected=selected;self.scroll=scroll;}self.search.cancel();self.reveal_current=false;return(true,None);}
                        KeyCode::Home|KeyCode::End|KeyCode::PageUp|KeyCode::PageDown=>{
                            let height=usize::from(self.area.height.saturating_sub(2)).max(1);let max=self.rows.len().saturating_sub(height);
                            self.scroll=match k.code{KeyCode::Home=>0,KeyCode::End=>max,KeyCode::PageUp=>self.scroll.saturating_sub(height),_=>self.scroll.saturating_add(height).min(max)};
                            self.reveal_current=false;return(true,None);
                        },
                        _=>{}
                    }
                }
                let page=self.area.height.saturating_sub(2) as isize;
                let heading=self.cursor().and_then(|row|match &self.rows[row]{NavRow::Dir{path,collapsed,..}=>Some((path.clone(),*collapsed)),_=>None});
                let row=match k.code {
                    KeyCode::Down=>step(1),KeyCode::Up=>step(-1),
                    KeyCode::Home=>walkable.first().copied(),KeyCode::End=>walkable.last().copied(),
                    KeyCode::PageDown=>step(page),KeyCode::PageUp=>step(-page),
                    // on a heading Left collapses and Right expands it; Left on a file goes to its folder's heading
                    KeyCode::Left if k.modifiers.is_empty()=>return(true,Some(match heading{Some((path,false))=>Pick::Toggle(path),Some(_)=>return(true,None),None=>Pick::FolderOf(self.selected)})),
                    KeyCode::Right if k.modifiers.is_empty()=>return(true,heading.filter(|(_,collapsed)|*collapsed).map(|(path,_)|Pick::Toggle(path))),
                    KeyCode::Left|KeyCode::Right if !k.modifiers.contains(KeyModifiers::ALT|KeyModifiers::SHIFT)=>return(true,None),
                    KeyCode::Enter if heading.is_some()=>return(true,heading.map(|(path,_)|Pick::Toggle(path))),
                    KeyCode::Enter=>{self.focused=false;return(true,Some(Pick::File(self.selected)));}
                    KeyCode::Esc=>{self.focused=false;return(true,None);}
                    _=>return(false,None),
                };
                let Some(row)=row else{return(true,None)};
                if let Some(NavRow::File{index,..})=self.rows.get(row){self.selected= *index;}
                self.reveal(row);(true,self.pick(row))
            }
            _=>(false,None)
        }
    }
    fn reveal(&mut self,row:usize){let height=usize::from(self.area.height.saturating_sub(2));if row<self.scroll{self.scroll=row;}else if row>=self.scroll+height{self.scroll=row.saturating_add(1).saturating_sub(height);}}
    pub fn draw(&mut self,frame:&mut Frame,palette:&lenz::theme::Palette) {
        if !self.shown(){return;}
        let area=self.area;let Some(set)=self.set.upgrade()else{return;};
        let title=if self.search.active(){crate::pane_search::status(&self.search,set.comparisons.len())}else{self.heading.clone().unwrap_or_else(||format!("Files ({})",set.comparisons.len()))};
        let title=self.icons.label(crate::icons::Icon::Folder,&title);
        frame.render_widget(crate::chrome::pane(title.as_ref(),palette,self.focused).style(style_with(palette).bg(color(palette.sidebar_bg))),area);
        let height=usize::from(area.height.saturating_sub(2));
        // Keep file switching visible, without snapping manually scrolled rows back every frame.
        if self.reveal_current&&!self.search.active(){self.reveal_current=false;if let Some(row)=self.cursor(){self.reveal(row);}}
        for (row,item) in self.rows.iter().enumerate().skip(self.scroll).take(height) {
            let(label,fg,selected,decoration)=match item {
                NavRow::Group(name,count)=>(format!("{name} ({count})"),palette.fg,false,None),
                // the heading whose summary shows reads as the current row, in the text colour
                NavRow::Dir{path,collapsed,files,..}=>{
                    let current=self.summary_row==Some(row);
                    let icon=lenz::file_icons::folder(!collapsed);
                    let name=icon.label(self.icons.nerd(),path);
                    let label=if *collapsed{format!("▸ {name} · {files} file{}",if *files==1{""}else{"s"})}else{format!("▾ {name}")};
                    (label,if current{palette.fg}else{palette.dim},current,Some((2,icon)))
                }
                NavRow::File{index,name,from}=>{
                    let c=&set.comparisons[*index];let kind=match navigator::badge(&c.status){"A"=>0,"D"=>1,"M"=>2,"R"=>3,_=>4};
                    let icon=lenz::file_icons::file(&c.path);
                    let name=icon.label(self.icons.nerd(),if self.search.active(){c.path.as_str()}else{name.as_str()});
                    (format!(" {} {}{}",navigator::badge(&c.status),name,from.as_ref().map_or(String::new(),|p|format!(" ← {p}"))),palette.status[kind],*index==if self.search.active(){self.selected}else{self.current}&&(self.search.active()||!self.summary),Some((3,icon)))
                }
            };
            let rect=Rect::new(area.x+1,area.y+1+(row-self.scroll)as u16,area.width.saturating_sub(2),1);
            let bg=if selected{palette.selected_bg}else{palette.sidebar_bg};let s=style_with(palette).fg(color(fg)).bg(color(bg)).add_modifier(if selected&&self.focused{Modifier::BOLD}else{Modifier::empty()});
            frame.buffer_mut().set_style(rect,s);frame.render_widget(Paragraph::new(crate::pane_search::line(label,&self.search,s,palette)),rect);
            if self.icons.nerd() && let Some((offset,icon))=decoration && rect.width>offset {
                frame.buffer_mut()[(rect.x+offset,rect.y)].set_fg(color(icon.color_on(palette,bg)));
            }
        }
        self.resizer.draw(frame.buffer_mut(),palette);
        if set.comparisons.is_empty() {crate::visual::safe_text(frame.buffer_mut(),Rect::new(area.x+1,area.y+1,area.width.saturating_sub(2),1),"No changed files",style_with(palette));}
    }
}
