//! Terminal chrome: shared GUI palette and bounded, keyboard/mouse-accessible toolbars.
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::{Frame, buffer::Buffer, layout::Rect, style::{Color, Modifier, Style}, widgets::{Block, Borders, Clear}};
use unicode_width::UnicodeWidthStr;
use std::borrow::Cow;

#[cfg(test)]
pub fn palette()-> &'static lenz::theme::Palette {
    // Legacy fixture expectation only; production palettes are owned snapshots.
    static PALETTE:std::sync::OnceLock<&'static lenz::theme::Palette>=std::sync::OnceLock::new();
    PALETTE.get_or_init(||if std::env::var("LENZ_APPEARANCE").is_ok_and(|s|s=="light"){&lenz::theme::LIGHT}else{&lenz::theme::DARCULA})
}
pub fn initial_palette()-> &'static lenz::theme::Palette {
    #[cfg(test)] let palette=palette();
    #[cfg(not(test))] let palette=&lenz::theme::DARCULA;
    palette
}
pub fn color(rgb:u32)->Color { Color::Rgb((rgb>>16)as u8,(rgb>>8)as u8,rgb as u8) }
pub fn style_with(palette:&lenz::theme::Palette)->Style { Style::default().fg(color(palette.fg)).bg(color(palette.bg)) }

#[derive(Clone)]
pub struct Item<A> { pub label:Cow<'static,str>, pub action:A, pub active:bool, pub disabled:Option<Cow<'static,str>>, pub choices:Vec<Item<A>>, overflow:bool,
    /// In the bar's middle (a pull request's tabs).
    center:bool,
    /// At the bar's right end (help), as in the GUI.
    right:bool }
impl<A> Item<A> {
    pub fn new(label:impl Into<Cow<'static,str>>,action:A)->Self {Self{label:label.into(),action,active:false,disabled:None,choices:Vec::new(),overflow:false,center:false,right:false}}
    pub fn center(mut self)->Self {self.center=true;self}
    pub fn right(mut self)->Self {self.right=true;self}
    pub fn active(mut self,on:bool)->Self {self.active=on;self}
    pub fn disabled(mut self,why:Option<&'static str>)->Self {self.disabled=why.map(Cow::Borrowed);self}
    pub fn disabled_message(mut self,why:Option<String>)->Self {self.disabled=why.map(Cow::Owned);self}
    pub fn choices(mut self,choices:Vec<Item<A>>)->Self {self.choices=choices;self}
    pub fn overflow(mut self)->Self {self.overflow=true;self}
}
pub struct Bar<A> {
    items:Vec<Item<A>>, hits:Vec<(usize,Rect)>, selected:Option<usize>, first:usize,
    dropdown:Option<usize>, path:Vec<usize>, row:usize, menu_first:usize, popup:Rect, bounds:Rect, notice:Option<Cow<'static,str>>,
    painted:Option<Buffer>,paint_revision:u64,paint_focus:(Option<usize>,usize),
}
impl<A> Default for Bar<A> {
    fn default()->Self {Self{items:Vec::new(),hits:Vec::new(),selected:None,first:0,dropdown:None,path:Vec::new(),row:0,menu_first:0,popup:Rect::default(),bounds:Rect::default(),notice:None,painted:None,paint_revision:0,paint_focus:(None,0)}}
}
impl<A:Copy> Bar<A> {
    pub fn focused(&self)->bool {self.selected.is_some()}
    pub fn menu_open(&self)->bool { self.dropdown.is_some() || self.notice.is_some() }
    pub fn draw_with_theme(&mut self,frame:&mut Frame,area:Rect,items:Vec<Item<A>>,palette:&lenz::theme::Palette,revision:u64)->Rect {
        let items=compact(items,area);
        // A file switch can replace text controls with image controls while a menu is open.
        if self.dropdown.is_some_and(|i|self.items.get(i).zip(items.get(i)).is_none_or(|(old,new)|old.label!=new.label||old.choices.iter().map(|r|&r.label).ne(new.choices.iter().map(|r|&r.label)))) || self.selected.is_some_and(|i|i>=items.len()){self.close();}
        let reuse=self.paint_revision==revision&&self.bounds==area&&self.paint_focus==(self.selected,self.first)&&self.items.len()==items.len()
            &&self.items.iter().zip(&items).all(|(old,new)|old.label==new.label&&old.active==new.active&&old.disabled==new.disabled);
        self.paint_revision=revision;self.bounds=area;self.items=items;
        if area.width==0||area.height==0{self.painted=None;self.hits.clear();return area;}
        if reuse && let Some(painted)=&self.painted {
            // Only the toolbar's (at most two) cell rows are cached, never a document.
            copy_rows(painted,frame.buffer_mut(),painted.area);
            return Rect::new(area.x,area.y+painted.area.height,area.width,area.height-painted.area.height);
        }
        self.hits.clear();
        let max_rows=area.height.saturating_sub(5).clamp(1,2);
        if let Some(at)=self.selected && at<self.first {self.first=at;}
        self.first=self.first.min(self.items.len().saturating_sub(1));
        let mut x=area.x;let mut y=area.y;
        let width_of=|item:&Item<A>|(item.label.width()as u16).saturating_add(2).min(area.width);
        // the middle group waits until the rest is placed (unless the bar is scrolled)
        let centered=|item:&Item<A>|item.center&&self.first==0;
        let right=|item:&Item<A>|item.right&&self.first==0;
        for (i,item) in self.items.iter().enumerate().skip(self.first).filter(|(_,item)|!centered(item)&&!right(item)) {
            let width=width_of(item);
            if x>area.x && x.saturating_add(width)>area.right() {x=area.x;y=y.saturating_add(1);}
            if y>=area.y+max_rows {break;}
            self.hits.push((i,Rect::new(x,y,width,1)));x=x.saturating_add(width).saturating_add(1);
        }
        let middle:Vec<usize>=self.items.iter().enumerate().filter(|(_,item)|centered(item)).map(|(i,_)|i).collect();
        if !middle.is_empty() && y<area.y+max_rows {
            let total=middle.iter().map(|&i|width_of(&self.items[i])+1).sum::<u16>().saturating_sub(1);
            let mid=area.x+area.width.saturating_sub(total)/2;
            // in the middle of the first row when the rest leaves it free, else right after the rest
            if y==area.y && mid>=x && mid.saturating_add(total)<=area.right() {x=mid;}
            for i in middle {
                let width=width_of(&self.items[i]);
                if x>area.x && x.saturating_add(width)>area.right() {x=area.x;y=y.saturating_add(1);}
                if y>=area.y+max_rows {break;}
                self.hits.push((i,Rect::new(x,y,width,1)));x=x.saturating_add(width).saturating_add(1);
            }
        }
        // the right end of the first row when it is free, else right after the rest
        for i in (0..self.items.len()).filter(|&i|right(&self.items[i])) {
            let width=width_of(&self.items[i]);
            let end=area.right().saturating_sub(width);
            if y==area.y && end>=x {x=end;}
            else if x>area.x && x.saturating_add(width)>area.right() {x=area.x;y=y.saturating_add(1);}
            if y>=area.y+max_rows {break;}
            self.hits.push((i,Rect::new(x,y,width,1)));x=x.saturating_add(width).saturating_add(1);
        }
        // Keyboard focus can reach controls beyond the current terminal width.
        if let Some(at)=self.selected && !self.hits.iter().any(|(i,_)|*i==at) && at<self.items.len() {
            self.first=at;let items=std::mem::take(&mut self.items);return self.draw_with_theme(frame,area,items,palette,revision);
        }
        let height=(y-area.y+1).min(area.height);
        frame.buffer_mut().set_style(Rect::new(area.x,area.y,area.width,height),style_with(palette).bg(color(palette.header_bg)));
        for &(index,rect) in &self.hits {
            let item=&self.items[index];let selected=self.selected==Some(index);
            let bg=if selected||item.active{palette.selected_bg}else{palette.header_bg};
            let fg=if item.disabled.is_some(){palette.dim}else{palette.fg};
            let s=style_with(palette).fg(color(fg)).bg(color(bg)).add_modifier(if selected{Modifier::BOLD}else{Modifier::empty()});
            frame.buffer_mut().set_style(rect,s);
            crate::visual::safe_text(frame.buffer_mut(),rect,&format!(" {} ",item.label),s);
        }
        let rect=Rect::new(area.x,area.y,area.width,height);let mut painted=Buffer::empty(rect);
        copy_rows(frame.buffer_mut(),&mut painted,rect);self.painted=Some(painted);self.paint_focus=(self.selected,self.first);
        Rect::new(area.x,area.y+height,area.width,area.height-height)
    }
    fn activate(&mut self,index:usize)->Option<A> {
        let item=self.items.get(index)?;
        if let Some(reason)=&item.disabled {self.selected=Some(index);self.notice=Some(reason.clone());return None;}
        self.notice=None;
        if !item.choices.is_empty() {
            self.selected=Some(index);self.dropdown=Some(index);self.path.clear();
            self.row=item.choices.iter().position(|r|r.active).unwrap_or(0);self.menu_first=0;None
        } else {let action=item.action;self.close();Some(action)}
    }
    /// Open the dropdown of the drawn item labelled `label`, as a click on it would; false when it is not drawn.
    pub fn open(&mut self,label:&str)->bool {
        let Some(index)=self.hits.iter().map(|&(i,_)|i).find(|&i|self.items.get(i).is_some_and(|item|item.label==label&&!item.choices.is_empty()))else{return false;};
        self.activate(index);self.dropdown==Some(index)
    }
    pub fn close(&mut self) {self.selected=None;self.dropdown=None;self.first=0;self.path.clear();self.notice=None;self.popup=Rect::default();}
    fn menu_rows(&self)->&[Item<A>] {
        let Some(mut item)=self.dropdown.and_then(|index|self.items.get(index))else{return &[];};
        for &index in &self.path {let Some(child)=item.choices.get(index)else{return &[];};item=child;}
        &item.choices
    }
    fn choose_row(&mut self,index:usize)->Option<A> {
        let row=self.menu_rows().get(index)?.clone();
        if let Some(reason)=row.disabled {self.row=index;self.notice=Some(reason);return None;}
        self.notice=None;
        if !row.choices.is_empty(){self.path.push(index);self.row=row.choices.iter().position(|r|r.active).unwrap_or(0);self.menu_first=0;return None;}
        let action=row.action;self.close();Some(action)
    }
    fn menu_back(&mut self) {if let Some(index)=self.path.pop(){self.row=index;self.menu_first=0;self.notice=None;}else{self.close();}}
    pub fn handle(&mut self,event:&Event)->(bool,Option<A>) {
        match event {
            Event::Key(key) if key.kind!=KeyEventKind::Release => {
                if key.code==KeyCode::F(10) {if self.selected.is_some(){self.close();}else{self.selected=Some(0);}return(true,None);}
                let Some(at)=self.selected else{return(false,None)};
                let last=self.items.len().saturating_sub(1);
                if self.dropdown.is_some() {
                    let last_row=self.menu_rows().len().saturating_sub(1);
                    match key.code {
                        KeyCode::Esc=>self.close(),
                        KeyCode::Left|KeyCode::BackTab=>self.menu_back(),
                        KeyCode::Up=>self.row=self.row.saturating_sub(1),
                        KeyCode::Down=>self.row=(self.row+1).min(last_row),
                        KeyCode::Home=>self.row=0,KeyCode::End=>self.row=last_row,
                        KeyCode::PageUp=>self.row=self.row.saturating_sub(8),KeyCode::PageDown=>self.row=(self.row+8).min(last_row),
                        KeyCode::Enter|KeyCode::Right|KeyCode::Char(' ')=>return(true,self.choose_row(self.row)),
                        _=>{}
                    }
                } else {
                    match key.code {
                        KeyCode::Esc=>self.close(),
                        KeyCode::Left|KeyCode::BackTab=>self.selected=Some(at.saturating_sub(1)),
                        KeyCode::Right|KeyCode::Tab=>self.selected=Some(if key.modifiers.contains(KeyModifiers::SHIFT){at.saturating_sub(1)}else{(at+1).min(last)}),
                        KeyCode::Home=>self.selected=Some(0),KeyCode::End=>self.selected=Some(last),
                        KeyCode::Enter|KeyCode::Down|KeyCode::Char(' ')=>return(true,self.activate(at)),
                        _=>{}
                    }
                }
                (true,None)
            }
            Event::Mouse(mouse) if mouse.kind==MouseEventKind::Down(MouseButton::Left)=>{
                let point=(mouse.column,mouse.row).into();
                if self.dropdown.is_some() && self.popup.contains(point) {
                    if mouse.row>self.popup.y&&mouse.row<self.popup.bottom().saturating_sub(if self.popup.height>=4{2}else{1}) {
                        let row=self.menu_first+usize::from(mouse.row-self.popup.y-1);return(true,self.choose_row(row));
                    }
                    return(true,None);
                }
                if let Some(&(index,_))=self.hits.iter().find(|(_,rect)|rect.contains(point)) {return(true,self.activate(index));}
                if self.selected.is_some(){self.close();return(true,None);}
                (false,None)
            }
            Event::Mouse(mouse) if self.dropdown.is_some() || self.notice.is_some()=>{
                if self.dropdown.is_some() {let last=self.menu_rows().len().saturating_sub(1);match mouse.kind {MouseEventKind::ScrollUp=>self.row=self.row.saturating_sub(3),MouseEventKind::ScrollDown=>self.row=(self.row+3).min(last),_=>{}}}
                (true,None)
            }
            Event::Paste(_) if self.dropdown.is_some()=>(true,None),
            _=>(false,None)
        }
    }
    pub fn draw_menu_with_theme(&mut self,frame:&mut Frame,palette:&lenz::theme::Palette) {
        let Some(index)=self.dropdown else{
            if let Some(reason)=&self.notice {let area=self.bounds;if area.width>2&&area.height>2{let height=area.height.min(3);let rect=Rect::new(area.x,area.y+area.height-height,area.width,height);frame.render_widget(Clear,rect);frame.render_widget(Block::default().borders(Borders::ALL).style(style_with(palette).bg(color(palette.status_bg))),rect);crate::visual::safe_text(frame.buffer_mut(),Rect::new(rect.x+1,rect.y+1,rect.width-2,1),reason,style_with(palette));}}
            return;
        };
        if self.items.get(index).is_none(){self.close();return;}
        let rows=self.menu_rows().to_vec();
        let mut title=self.items[index].label.trim_end_matches(" ▾").to_string();
        let mut item=&self.items[index];for &index in &self.path {if let Some(child)=item.choices.get(index){title.push_str(" / ");title.push_str(child.label.trim_end_matches(" ▾"));item=child;}}
        let area=self.bounds;if area.width<4||area.height<4{return;}
        let width=(rows.iter().map(|r|r.label.width()).max().unwrap_or(0)as u16+6).min(area.width);
        let height=(rows.len()as u16+3).min(area.height.saturating_sub(1));
        let anchor=self.hits.iter().find(|(i,_)|*i==index).map_or(area,|(_,r)|*r);
        self.popup=Rect::new(anchor.x.min(area.right()-width),(anchor.y+1).min(area.bottom()-height),width,height);
        frame.render_widget(Clear,self.popup);
        frame.render_widget(Block::default().borders(Borders::ALL).title(title).style(style_with(palette).bg(color(palette.status_bg))),self.popup);
        let visible=height.saturating_sub(3).max(1)as usize;
        self.menu_first=if self.row<self.menu_first{self.row}else if self.row>=self.menu_first+visible{self.row.saturating_add(1).saturating_sub(visible)}else{self.menu_first};
        self.menu_first=self.menu_first.min(rows.len().saturating_sub(visible));
        for (i,row) in rows.iter().enumerate().skip(self.menu_first).take(visible) {
            let rect=Rect::new(self.popup.x+1,self.popup.y+1+(i-self.menu_first) as u16,width-2,1);
            let bg=if self.row==i{palette.selected_bg}else{palette.status_bg};
            let s=style_with(palette).bg(color(bg)).fg(color(if row.disabled.is_some(){palette.dim}else{palette.fg}));
            frame.buffer_mut().set_style(rect,s);
            crate::visual::safe_text(frame.buffer_mut(),rect,&format!("{} {}",if row.active{"✓"}else{" "},row.label),s);
        }
        if height>=4 {let reason=rows.get(self.row).and_then(|r|r.disabled.as_deref()).unwrap_or("Enter selects · Esc closes");crate::visual::safe_text(frame.buffer_mut(),Rect::new(self.popup.x+1,self.popup.bottom()-2,width-2,1),reason,style_with(palette).fg(color(palette.dim)));}
    }
}

// Keep More visible at compact widths. Hidden controls retain their short
// submenus; direct actions take two clicks and settings keep their hierarchy.
fn compact<A:Copy>(mut items:Vec<Item<A>>,area:Rect)->Vec<Item<A>> {
    if !items.last().is_some_and(|item|item.overflow) || area.width==0 {return items;}
    let mut more=items.pop().unwrap();let max_rows=area.height.saturating_sub(5).clamp(1,2);
    let rows=|items:&[Item<A>],more:&Item<A>|{let mut x=0u16;let mut rows=1u16;for item in items.iter().chain(std::iter::once(more)){let width=(item.label.width()as u16).saturating_add(2).min(area.width);if x>0&&x.saturating_add(width)>area.width{rows+=1;x=0;}x=x.saturating_add(width).saturating_add(1);}rows};
    let mut keep=items.len();while keep>0&&rows(&items[..keep],&more)>max_rows{keep-=1;}
    let mut hidden=items.split_off(keep);
    // ⋯ appears only when something is folded into it
    if hidden.is_empty() && more.choices.is_empty() {return items;}
    // a ⋯ that runs something itself keeps that as its last row once it holds the hidden items
    if more.choices.is_empty() && !hidden.is_empty() {hidden.push(Item::new("All commands…",more.action));}
    hidden.append(&mut more.choices);more.choices=hidden;items.push(more);items

}

pub(crate) fn copy_rows(from:&Buffer,to:&mut Buffer,area:Rect){
    for y in area.y..area.bottom(){let source=from.index_of(area.x,y);let dest=to.index_of(area.x,y);let width=usize::from(area.width);to.content[dest..dest+width].clone_from_slice(&from.content[source..source+width]);}
}
