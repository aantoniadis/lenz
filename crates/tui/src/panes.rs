//! Cell geometry and mouse capture shared by terminal pane owners.
use crossterm::event::{Event, MouseButton, MouseEventKind};
use ratatui::{buffer::Buffer, layout::Rect};

pub fn left(width: u16, ratio: f32) -> u16 {
    lenz::pane_layout::split(f32::from(width.saturating_sub(1)), ratio, 12.0).round() as u16
}
pub fn three(area: Rect, ratios: [f32; 2]) -> [Rect; 3] {
    let widths = lenz::pane_layout::three(f32::from(area.width), ratios, 12.0);
    let a = (widths[0].round() as u16).min(area.width);
    let b = (widths[1].round() as u16).min(area.width - a);
    [Rect::new(area.x, area.y, a, area.height), Rect::new(area.x+a, area.y, b, area.height), Rect::new(area.x+a+b, area.y, area.width-a-b, area.height)]
}
pub fn size(available: u16, desired: f32, min: f32, remaining: f32) -> u16 {
    (lenz::pane_layout::clamp_size(f32::from(available), desired, min, remaining).round() as u16).min(available)
}
#[derive(Clone, Copy)]
pub struct Grip { pub area: Rect, pub id: u8, pub horizontal: bool, pub origin: u16, pub position: u16 }
impl Grip {
    pub fn vertical(area: Rect, id: u8, origin: u16) -> Self { Self { position: area.x, area, id, horizontal: false, origin } }
    pub fn horizontal(area: Rect, id: u8, origin: u16) -> Self { Self { position: area.y, area, id, horizontal: true, origin } }
    pub fn draw(&self, buf: &mut Buffer, palette: &lenz::theme::Palette) {
        let style=crate::chrome::style_with(palette).fg(crate::chrome::color(palette.border));
        // Bordered panes already paint their divider and may put a title on it.
        // Only fill a wholly empty, reserved separator; never turn title spaces
        // into strokes or overwrite document glyphs with the drag affordance.
        let empty=(self.area.y..self.area.bottom()).all(|y|(self.area.x..self.area.right()).all(|x|buf[(x,y)].symbol()==" "));
        let stroke=if self.horizontal {"─"}else{"│"};
        if empty {for y in self.area.y..self.area.bottom() {for x in self.area.x..self.area.right() {buf.set_string(x,y,stroke,style);}}}
        if self.area.width>0&&self.area.height>0 {
            let x=if self.horizontal {self.area.right().saturating_sub(2).max(self.area.x)}else{self.area.x};let y=self.area.y+self.area.height/2;
            if buf[(x,y)].symbol()==stroke {buf.set_string(x,y,if self.horizontal {"↕"}else{"↔"},style);}
        }
    }
}
#[derive(Default)]
pub struct Resizer { pub grips: Vec<Grip>, active: Option<(Grip, i32)> }
pub enum Resize { None, Captured, Move(u8, f32) }
impl Resizer {
    pub fn cancel(&mut self) { self.active=None; }
    pub fn handle(&mut self, event: &Event, blocked: bool) -> Resize {
        if blocked || matches!(event, Event::Resize(..)|Event::FocusLost) { self.cancel(); return Resize::None; }
        let Event::Mouse(mouse)=event else { return Resize::None; };
        if let Some((grip, offset))=self.active {
            match mouse.kind {
                MouseEventKind::Up(MouseButton::Left) => { self.cancel(); return Resize::Captured; }
                MouseEventKind::Drag(MouseButton::Left) => {
                    let coordinate=if grip.horizontal { mouse.row } else { mouse.column };
                    return Resize::Move(grip.id,(i32::from(coordinate)-offset-i32::from(grip.origin)) as f32);
                }
                MouseEventKind::Down(_) => self.cancel(),
                _ => return Resize::Captured,
            }
        }
        if mouse.kind==MouseEventKind::Down(MouseButton::Left) && let Some(grip)=self.grips.iter().find(|g|g.area.contains((mouse.column,mouse.row).into())).copied() {
            let coordinate=if grip.horizontal { mouse.row } else { mouse.column };
            self.active=Some((grip,i32::from(coordinate)-i32::from(grip.position)));
            return Resize::Captured;
        }
        Resize::None
    }
    pub fn draw(&self, buf: &mut Buffer, palette: &lenz::theme::Palette) { for grip in &self.grips { grip.draw(buf,palette); } }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pane_grip_preserves_titles_and_document_glyphs() {
        let area=Rect::new(0,0,50,5);let mut buf=Buffer::empty(area);
        let title="Find in Change Set · 2 matches · needle";
        buf.set_string(0,0,title,crate::chrome::style_with(crate::chrome::palette()));
        Grip::horizontal(Rect::new(0,0,50,1),0,0).draw(&mut buf,crate::chrome::palette());
        let painted=(0..50).map(|x|buf[(x,0)].symbol()).collect::<String>();assert!(painted.starts_with(title));
        buf.set_string(30,2,"x",crate::chrome::style_with(crate::chrome::palette()));
        Grip::vertical(Rect::new(30,0,1,5),0,0).draw(&mut buf,crate::chrome::palette());assert_eq!(buf[(30,2)].symbol(),"x");
    }
}
