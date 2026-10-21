//! Adapter-owned horizontal geometry. Widths are computed only on first use per document.
use super::*;
use unicode_width::UnicodeWidthStr;

pub(super) fn line_columns(line:&str)->usize{
    if line.is_ascii(){let text=&line[..line.len().min(LONG_LINE)];return text.len()+3*text.bytes().filter(|b|*b==b'\t').count();}
    let end=line.char_indices().nth(LONG_LINE).map_or(line.len(),|(i,_)|i);let text=&line[..end];
    text.width()+4*text.bytes().filter(|b|*b==b'\t').count()
}
pub(super) fn columns(lines: &[String]) -> usize {
    lines.iter().map(|line|line_columns(line)).max().unwrap_or(0)
}
impl Viewer {
    fn side_max(&mut self, side: Side) -> f32 {
        let width=(self.text_w[side as usize]-self.ann_w(side)).max(0.0);
        let Some(loaded)=self.loaded.get_mut(&self.comparison.current) else{return 0.0;};
        let widest=*loaded.columns[side as usize].get_or_insert_with(||columns(loaded.doc.lines_on(side)));
        (widest as f32*self.char_w-width).max(0.0)
    }
    /// Whether both Sides share one sideways position (every Scroll Mode but Independent).
    pub(super) fn shared_horizontal(&self) -> bool { self.comparison.view.mode().shares_horizontal() }
    /// A shared position reaches the end of the wider Side.
    fn horizontal_max(&mut self, side: Side) -> f32 {
        if self.shared_horizontal(){self.side_max(Side::Left).max(self.side_max(Side::Right))}else{self.side_max(side)}
    }
    /// Moving `side` moves both when they share a position.
    fn set_horizontal(&mut self, side: Side, x: f32) {
        if self.shared_horizontal(){self.hscroll=[x;2];}else{self.hscroll[side as usize]=x;}
    }
    pub(super) fn clamp_horizontal(&mut self, side: Side) {
        if self.hscroll[side as usize]==0.0&&!self.shared_horizontal(){return;}
        let max=self.horizontal_max(side);
        self.set_horizontal(side,self.hscroll[side as usize].clamp(0.0,max));
    }
    pub(super) fn scroll_horizontal(&mut self, side: Side, pixels: f32) {
        if pixels<=0.0&&self.hscroll[side as usize]==0.0{return;}
        let max=self.horizontal_max(side);
        self.set_horizontal(side,(self.hscroll[side as usize]+pixels).clamp(0.0,max));
    }
    /// After a switch out of Independent, the Sides take the focused one's position.
    pub(super) fn sync_horizontal(&mut self) {
        if self.shared_horizontal()&&self.hscroll[0]!=self.hscroll[1]{let side=self.pane_side(self.diff_focus);self.clamp_horizontal(side);}
    }
}
