//! A small cell fallback; Screens replaces it with pixel artwork in graphics terminals.
use crossterm::event::{Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};
use ratatui::{Frame, layout::Rect, style::{Color, Style}, widgets::{Block, Borders, Clear}};
use crate::chrome::{color, style_with};

const LOGO: &[u8] = lenz::about::LOGO_TERMINAL;
#[derive(Default)]
pub struct About { scroll:usize, close:Rect, rect:Rect, pub(crate) logo_rect:Rect }
impl About {
    /// All input belongs to the dialog until dismissed. True means close.
    pub fn handle(&mut self,event:&Event)->bool {
        match event {
            Event::Key(k) if k.kind!=KeyEventKind::Release=>match k.code {
                KeyCode::Esc|KeyCode::Enter=>return true,
                KeyCode::Up=>self.scroll=self.scroll.saturating_sub(1),
                KeyCode::Down=>self.scroll=self.scroll.saturating_add(1),
                KeyCode::PageUp=>self.scroll=self.scroll.saturating_sub(8),
                KeyCode::PageDown=>self.scroll=self.scroll.saturating_add(8),
                KeyCode::Home=>self.scroll=0,KeyCode::End=>self.scroll=usize::MAX,_=>{}
            },
            Event::Mouse(m)=>match m.kind {
                // the Close bar, or a click outside the box
                MouseEventKind::Down(MouseButton::Left) if self.close.contains((m.column,m.row).into())||!self.rect.contains((m.column,m.row).into())=>return true,
                MouseEventKind::ScrollUp=>self.scroll=self.scroll.saturating_sub(3),
                MouseEventKind::ScrollDown=>self.scroll=self.scroll.saturating_add(3),_=>{}
            },_=>{}
        }
        false
    }
    pub fn draw(&mut self,frame:&mut Frame,area:Rect,palette:&lenz::theme::Palette) {
        self.close=Rect::default();
        self.logo_rect=Rect::default();
        if area.width<4||area.height<4{return;}
        let w=area.width.saturating_sub(2).min(80);let h=area.height.saturating_sub(2).min(18);
        let rect=Rect::new(area.x+(area.width-w)/2,area.y+(area.height-h)/2,w,h);
        self.rect=rect;
        let base=style_with(palette).bg(color(palette.status_bg));
        frame.render_widget(Clear,rect);
        frame.render_widget(Block::default().borders(Borders::ALL).title(format!(" {} ",lenz::about::TITLE)).style(base),rect);
        let logo=w>=78&&h>=16;
        let x=rect.x+2+if logo{26}else{0};let width=usize::from(rect.right().saturating_sub(x+2)).max(1);
        let mut lines=Vec::new();
        for text in std::iter::once(lenz::about::DESCRIPTION.to_owned()).chain(std::iter::once(String::new())).chain(lenz::about::ROWS.iter().map(|(k,v)|format!("{k}: {v}"))) {
            if text.is_empty(){lines.push(String::new());}else{for chunk in text.as_bytes().chunks(width){lines.push(String::from_utf8_lossy(chunk).into_owned());}}
        }
        let visible=usize::from(h.saturating_sub(4));self.scroll=self.scroll.min(lines.len().saturating_sub(visible));
        for (i,line) in lines.iter().skip(self.scroll).take(visible).enumerate(){crate::visual::safe_text(frame.buffer_mut(),Rect::new(x,rect.y+1+i as u16,width as u16,1),line,base);}
        if logo {
            self.logo_rect=Rect::new(rect.x+2,rect.y+1,24,12);
            for y in 0..12 {for x in 0..24 {
                let upper=(y*2*24+x)*4;let lower=upper+24*4;
                let fg=blend(&LOGO[upper..upper+4],palette.status_bg);let bg=blend(&LOGO[lower..lower+4],palette.status_bg);
                frame.buffer_mut()[(rect.x+2+x as u16,rect.y+1+y as u16)].set_symbol("▀").set_style(Style::default().fg(fg).bg(bg));
            }}
        }
        self.close=Rect::new(rect.x+1,rect.bottom().saturating_sub(2),w.saturating_sub(2),1);
        crate::visual::safe_text(frame.buffer_mut(),self.close," Close · Esc / Enter",base.fg(color(palette.accent)));
    }
}
fn blend(pixel:&[u8],background:u32)->Color {
    let a=u32::from(pixel[3]);
    let c=|channel:usize,shift:u32|((u32::from(pixel[channel])*a+((background>>shift)&255)*(255-a)+127)/255)as u8;
    Color::Rgb(c(0,16),c(1,8),c(2,0))
}
