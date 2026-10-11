//! Persistent GUI-style File Navigator; build grouped rows only when the Change Set changes.
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use lenz::{changeset::ChangeSet,navigator::{self,NavRow}};
use ratatui::{Frame,layout::Rect,style::Modifier,widgets::{Block,Borders,Paragraph}};
use std::sync::{Arc,Weak};
use crate::chrome::{color,style_with};

#[derive(Default)]
pub struct Files {
    set:Weak<ChangeSet>,rows:Vec<NavRow>,current:usize,selected:usize,scroll:usize,reveal_current:bool,
    pub(crate) search:lenz::pane_search::PaneSearch,search_origin:Option<(usize,usize)>,
    pub focused:bool,pub visible:Option<bool>,pub(crate) area:Rect,available:Rect,
    /// Replaces "Files (N)" as the box title (Focus names the selection there).
    pub(crate) heading:Option<String>,
    /// Hidden for now without changing whether Files is on (a PR's Conversation and Commits).
    pub(crate) suppressed:bool,
    pub(crate) desired_width:Option<f32>,pub(crate) resizer:crate::panes::Resizer,
}
impl Files {
    pub fn prepare(&mut self,set:Option<Arc<ChangeSet>>,current:usize,repository_shown:bool,area:Rect)->Rect {
        let resized=self.available.height!=area.height;
        self.available=area;
        if let Some(set)=set {
            if !self.set.ptr_eq(&Arc::downgrade(&set)) {
                self.rows=navigator::grouped_rows(&set.comparisons,&set.groups);self.set=Arc::downgrade(&set);
                self.selected=current;self.scroll=0;self.reveal_current=true;self.search.invalidate();self.search_origin=Some((current,0));
            }
            if self.current!=current {self.selected=current;self.reveal_current=true;if self.search.active(){self.search_origin=Some((current,self.scroll));}}self.current=current;
            let show=!self.suppressed&&self.visible.unwrap_or(repository_shown||set.comparisons.len()>1);
            let width=if show&&area.width>=50&&area.height>=5{crate::panes::size(area.width,self.desired_width.unwrap_or(f32::from((area.width/4).clamp(18,30))),18.0,32.0)}else{0};
            if resized{self.reveal_current=true;}
            self.area=Rect::new(area.x,area.y,width,area.height);
        } else {self.rows.clear();self.set=Weak::new();self.search.cancel();self.search_origin=None;self.area=Rect::new(area.x,area.y,0,0);}
        self.resizer.grips.clear();
        if self.area.width>0 {self.resizer.grips.push(crate::panes::Grip::vertical(Rect::new(self.area.right()-1,self.area.y,1,self.area.height),0,area.x));}
        if self.area.width==0{self.focused=false;}
        self.poll_search();
        Rect::new(area.x+self.area.width,area.y,area.width-self.area.width,area.height)
    }
    pub fn resize_input(&mut self,event:&Event,blocked:bool)->bool {match self.resizer.handle(event,blocked){crate::panes::Resize::None=>false,crate::panes::Resize::Captured=>true,crate::panes::Resize::Move(_,width)=>{self.desired_width=Some((width+1.0).max(0.0));true}}}
    pub fn file_at(&self,column:u16,row:u16)->Option<usize>{
        if !self.shown()||!self.area.contains((column,row).into())||row<=self.area.y||row>=self.area.bottom().saturating_sub(1){return None;}
        match self.rows.get(self.scroll+usize::from(row-self.area.y-1)){Some(NavRow::File{index,..})=>Some(*index),_=>None}
    }
    pub fn can_show(&self)->bool{self.available.width>=50&&self.available.height>=5}
    pub fn shown(&self)->bool {self.area.width>0}
    pub(crate) fn candidate(&self)->usize {self.selected}
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
    pub fn handle(&mut self,event:&Event)->(bool,Option<usize>) {
        if !self.shown(){return(false,None);}
        let max=self.set.upgrade().map_or(0,|s|s.comparisons.len().saturating_sub(1));
        match event {
            Event::Mouse(m) if self.area.contains((m.column,m.row).into())=>{
                match m.kind {
                    MouseEventKind::ScrollUp=>self.scroll=self.scroll.saturating_sub(3),
                    MouseEventKind::ScrollDown=>self.scroll=(self.scroll+3).min(self.rows.len().saturating_sub(self.area.height.saturating_sub(2)as usize)),
                    MouseEventKind::Down(MouseButton::Left)=>{
                        self.focused=true;
                        if m.row>self.area.y&&m.row<self.area.bottom().saturating_sub(1) && let Some(NavRow::File{index,..})=self.rows.get(self.scroll+usize::from(m.row-self.area.y-1)) {
                            self.selected= *index;self.search_origin=Some((self.selected,self.scroll));return(true,Some(*index));
                        }
                    }
                    _=>{}
                }
                (true,None)
            }
            Event::Paste(text) if self.focused=>{self.begin_search();self.search.append(&text.replace(['\n','\r']," "));self.poll_search();(true,None)}
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
                        KeyCode::Enter=>{if let Some(index)=self.search.confirm(){self.selected=index;self.search_origin=Some((index,self.scroll));return(true,Some(index));}return(true,None);}
                        KeyCode::Esc=>{if let Some((selected,scroll))=self.search_origin.take(){self.selected=selected;self.scroll=scroll;}self.search.cancel();self.reveal_current=false;return(true,None);}
                        KeyCode::Home|KeyCode::End|KeyCode::PageUp|KeyCode::PageDown=>{
                            let height=usize::from(self.area.height.saturating_sub(2)).max(1);let max=self.rows.len().saturating_sub(height);
                            self.scroll=match k.code{KeyCode::Home=>0,KeyCode::End=>max,KeyCode::PageUp=>self.scroll.saturating_sub(height),_=>self.scroll.saturating_add(height).min(max)};
                            self.reveal_current=false;return(true,None);
                        },
                        _=>{}
                    }
                }
                let selected=match k.code {
                    KeyCode::Down=>(self.selected+1).min(max),KeyCode::Up=>self.selected.saturating_sub(1),
                    KeyCode::Home=>0,KeyCode::End=>max,
                    KeyCode::PageDown=>(self.selected+usize::from(self.area.height.saturating_sub(2))).min(max),
                    KeyCode::PageUp=>self.selected.saturating_sub(usize::from(self.area.height.saturating_sub(2))),
                    KeyCode::Left|KeyCode::Right if !k.modifiers.contains(KeyModifiers::ALT|KeyModifiers::SHIFT)=>return(true,None),
                    KeyCode::Enter=>{self.focused=false;return(true,Some(self.selected));}
                    KeyCode::Esc=>{self.focused=false;return(true,None);}
                    _=>return(false,None),
                };
                self.selected=selected;self.reveal_file(selected);(true,Some(selected))
            }
            _=>(false,None)
        }
    }
    fn reveal(&mut self,row:usize){let height=usize::from(self.area.height.saturating_sub(2));if row<self.scroll{self.scroll=row;}else if row>=self.scroll+height{self.scroll=row.saturating_add(1).saturating_sub(height);}}
    pub fn draw(&mut self,frame:&mut Frame,palette:&lenz::theme::Palette) {
        if !self.shown(){return;}
        let area=self.area;let Some(set)=self.set.upgrade()else{return;};
        let title=if self.search.active(){crate::pane_search::status(&self.search,set.comparisons.len())}else{self.heading.clone().unwrap_or_else(||format!("Files ({})",set.comparisons.len()))};
        frame.render_widget(Block::default().borders(Borders::ALL).title(title).style(style_with(palette).bg(color(palette.sidebar_bg))),area);
        let height=usize::from(area.height.saturating_sub(2));
        // Keep file switching visible, without snapping manually scrolled rows back every frame.
        if self.reveal_current&&!self.search.active(){self.reveal_current=false;if let Some(row)=self.rows.iter().position(|r|matches!(r,NavRow::File{index,..}if *index==if self.search.active(){self.selected}else{self.current})){self.reveal(row);}}
        for (row,item) in self.rows.iter().enumerate().skip(self.scroll).take(height) {
            let(label,fg,selected)=match item {
                NavRow::Group(name,count)=>(format!("{name} ({count})"),palette.fg,false),
                NavRow::Dir(dir)=>(format!("▾ {dir}"),palette.dim,false),
                NavRow::File{index,name,from}=>{
                    let c=&set.comparisons[*index];let kind=match navigator::badge(&c.status){"A"=>0,"D"=>1,"M"=>2,"R"=>3,_=>4};
                    (format!(" {} {}{}",navigator::badge(&c.status),if self.search.active(){c.path.as_str()}else{name.as_str()},from.as_ref().map_or(String::new(),|p|format!(" ← {p}"))),palette.status[kind],*index==if self.search.active(){self.selected}else{self.current})
                }
            };
            let rect=Rect::new(area.x+1,area.y+1+(row-self.scroll)as u16,area.width.saturating_sub(2),1);
            let bg=if selected{palette.selected_bg}else{palette.sidebar_bg};let s=style_with(palette).fg(color(fg)).bg(color(bg)).add_modifier(if selected&&self.focused{Modifier::BOLD}else{Modifier::empty()});
            frame.buffer_mut().set_style(rect,s);frame.render_widget(Paragraph::new(crate::pane_search::line(label,&self.search,s,palette)),rect);
        }
        self.resizer.draw(frame.buffer_mut(),palette);
        if set.comparisons.is_empty() {crate::visual::safe_text(frame.buffer_mut(),Rect::new(area.x+1,area.y+1,area.width.saturating_sub(2),1),"No changed files",style_with(palette));}
    }
}
