//! Window-owned sizing preferences. Desired sizes survive temporary viewport constraints.
use super::*;
use crate::pane_layout::{clamp_size, split, three};

const MIN_PANE: f32 = 120.0;
const HANDLE: f32 = 3.0;
#[derive(Clone, Copy, Debug, PartialEq)]
enum Divider { Picker, Navigator, Log, Panel, Version, Comparison, Merge(usize) }
#[derive(Clone, Copy)]
pub(super) struct PaneDrag { divider: Divider, offset: f32 }
pub(super) struct PaneSizes {
    pub(super) comparison: f32,
    pub(super) merge: [f32; 2],
    pub(super) navigator: f32,
    pub(super) picker: f32,
    pub(super) log: f32,
    pub(super) panel: f32,
    pub(super) version: Option<f32>,
    pub(super) version_scroll: f32,
    pub(super) viewport: [f32; 2],
    pub(super) drag: Option<PaneDrag>,
}
impl Default for PaneSizes {
    fn default() -> Self {
        Self { comparison: 0.5, merge: [1.0/3.0,2.0/3.0], navigator: SIDEBAR_W,
            picker: 240.0, log: (LOG_ROWS+1.0)*line_h(), panel: PANEL_ROWS*line_h(),
            version: None, version_scroll: 0.0, viewport: [1600.0,900.0], drag: None }
    }
}
impl Viewer {
    pub(super) fn comparison_widths(&self, width: f32) -> [f32; 2] {
        let total=(width-GUTTER_W).max(0.0);
        // the Inline Preview is one full-width pane
        if self.markdown_inline() || self.comparison.view.unified() && self.image_doc().is_none() {return [0.0,total];}
        let left=split(total,self.panes.comparison,MIN_PANE);
        [left,(total-left).max(0.0)]
    }
    pub(super) fn merge_widths(&self, width: f32) -> [f32; 3] {
        three((width-2.0*GUTTER_W).max(0.0),self.panes.merge,MIN_PANE)
    }
    pub(super) fn log_rows(&self) -> f32 { (self.log_h()/line_h()-1.0).max(0.0) }
    pub(super) fn panel_rows(&self) -> f32 { (self.panel_h()/line_h()-1.0).max(0.0) }
    fn pane_handles(&self) -> Vec<(Divider,f32,f32,f32,f32)> {
        let [w,h]=self.panes.viewport;
        let top=toolbar_h()+self.log_h();
        let bottom=(h-self.panel_h()).max(top);
        let mut handles=Vec::new();
        if self.picker_w()>0.0 {handles.push((Divider::Picker,self.picker_w()-HANDLE,toolbar_h(),2.0*HANDLE,(h-toolbar_h()).max(0.0)));}
        if self.sidebar_w()>0.0 {handles.push((Divider::Navigator,self.diff_x()-HANDLE,top,2.0*HANDLE,(bottom-top).max(0.0)));}
        if self.log_h()>0.0 {handles.push((Divider::Log,self.picker_w(),top-HANDLE,(w-self.picker_w()).max(0.0),2.0*HANDLE));}
        if self.panel_h()>0.0 {handles.push((Divider::Panel,self.picker_w(),bottom-HANDLE,(w-self.picker_w()).max(0.0),2.0*HANDLE));}
        let version_h=self.picker_v_height();
        let content_bottom=(bottom-version_h).max(top);
        if version_h>0.0 {handles.push((Divider::Version,self.diff_x(),content_bottom-HANDLE,(w-self.diff_x()).max(0.0),2.0*HANDLE));}
        let width=(w-self.diff_x()).max(0.0);
        if self.merge.is_some() {
            let widths=self.merge_widths(width);
            for i in 0..2 {let edge=self.diff_x()+widths[..=i].iter().sum::<f32>()+i as f32*GUTTER_W+GUTTER_W/2.0;
                handles.push((Divider::Merge(i),edge-HANDLE,top,2.0*HANDLE,(content_bottom-top).max(0.0)));}
        } else if (self.image_doc().is_none()||self.iview.mode()==image_view::Mode::TwoUp)&&!self.markdown_inline()&&!self.pr_details_shown()&&!self.summary_shown() {
            let edge=self.diff_x()+self.comparison_widths(width)[0]+GUTTER_W/2.0;
            handles.push((Divider::Comparison,edge-HANDLE,top,2.0*HANDLE,(content_bottom-top).max(0.0)));
        }
        handles
    }
    pub(super) fn resize_blocked(&self) -> bool {
        self.discard_modal()||self.theme_chooser.is_some()||self.about.is_some()
            ||self.help||self.goto.is_some()||self.switcher.is_some()||self.popup.is_some()||self.menu.is_some()||self.command_palette.is_some()||self.diff_options.is_some()
            ||self.comparison.find.is_some()
            ||self.conflicts.as_ref().is_some_and(|s|s.confirming())
            ||self.search.as_ref().is_some_and(|s|s.editing)
            ||self.merge.as_ref().is_some_and(|s|s.ask.is_some()||s.find_editing())||self.picker_v.as_ref().is_some_and(|s|s.ask.is_some())
    }
    fn available_handles(&self)->Vec<(Divider,f32,f32,f32,f32)> {
        let mut handles=Vec::new();
        for (divider,x,y,w,h) in self.pane_handles() {
            let mut blocked=Vec::new();
            match divider {
                Divider::Merge(_)=>if let Some(merge)=&self.merge {
                    for (cx,cy,_) in &merge.controls {if x<self.diff_x()+cx+16.0&&x+w>self.diff_x()+cx-2.0 {
                        blocked.push((self.top()+cy,self.top()+cy+line_h()));
                    }}
                },
                Divider::Comparison if self.image_doc().is_none()&&!self.markdown_active()=>{
                    for c in self.comparison.view.frame().connectors.iter().filter(|c|c.foldable) {
                        let top=c.left.0.min(c.right.0);
                        blocked.push((self.top()+top*line_h(),self.top()+c.left.1.max(c.right.1).max(top+1.0)*line_h()));
                    }
                },
                _=>{}
            }
            blocked.sort_by(|a,b|a.0.total_cmp(&b.0));
            let mut start=y;
            for (a,b) in blocked {let a=a.max(y).min(y+h);let b=b.max(y).min(y+h);
                if a>start {handles.push((divider,x,start,w,a-start));}start=start.max(b);
            }
            if start<y+h {handles.push((divider,x,start,w,y+h-start));}
        }
        handles
    }
    fn resize_hit(&self,x:f32,y:f32)->Option<Divider> {
        if self.resize_blocked(){return None;}
        self.available_handles().into_iter().find(|(_,hx,hy,w,h)|x>=*hx&&x<*hx+*w&&y>=*hy&&y<*hy+*h).map(|h|h.0)
    }
    pub(super) fn begin_resize(&mut self,x:f32,y:f32)->bool {
        let Some(divider)=self.resize_hit(x,y)else{return false;};
        let handle=self.pane_handles().into_iter().find(|h|h.0==divider).unwrap();
        let edge=if matches!(divider,Divider::Log|Divider::Panel|Divider::Version){handle.2+HANDLE}else{handle.1+HANDLE};
        self.panes.drag=Some(PaneDrag{divider,offset:if matches!(divider,Divider::Log|Divider::Panel|Divider::Version){y-edge}else{x-edge}});
        self.dragging=false;self.markdown.dragging=false;self.iview.drag=None;
        true
    }
    pub(super) fn resize_move(&mut self,x:f32,y:f32,pressed:bool,cx:&mut Context<Self>)->bool {
        if self.resize_blocked(){self.panes.drag=None;return false;}
        let Some(drag)=self.panes.drag else{return false;};
        if !pressed{self.panes.drag=None;return false;}
        let [w,h]=self.panes.viewport;
        let px=x-drag.offset;
        let py=y-drag.offset;
        match drag.divider {
            Divider::Picker=>self.panes.picker=clamp_size(w,px,100.0,MIN_PANE+self.sidebar_w()),
            Divider::Navigator=>self.panes.navigator=clamp_size((w-self.picker_w()).max(0.0),px-self.nav_x(),100.0,MIN_PANE),
            Divider::Log=>self.panes.log=clamp_size((h-toolbar_h()-self.panel_h()).max(0.0),py-toolbar_h(),2.0*line_h(),hdr_h()+2.0*line_h()),
            Divider::Panel=>self.panes.panel=clamp_size((h-self.top()).max(0.0),h-py,2.0*line_h(),2.0*line_h()),
            Divider::Version=>self.panes.version=Some(clamp_size((h-self.top()-self.panel_h()).max(0.0),h-self.panel_h()-py,3.0*line_h(),2.0*line_h())),
            Divider::Comparison=>{let total=(w-self.diff_x()-GUTTER_W).max(0.0);if total>0.0{self.panes.comparison=split(total,(px-self.diff_x()-GUTTER_W/2.0)/total,MIN_PANE)/total;}},
            Divider::Merge(i)=>{let total=(w-self.diff_x()-2.0*GUTTER_W).max(0.0);if total>0.0{
                let widths=self.merge_widths(w-self.diff_x());
                let desired=px-self.diff_x()-(i as f32+0.5)*GUTTER_W;
                let min=MIN_PANE.min(total/3.0);
                let (lo,hi)=if i==0{(min,widths[0]+widths[1]-min)}else{(widths[0]+min,total-min)};
                self.panes.merge[i]=desired.clamp(lo,hi.max(lo))/total;
            }}
        }
        cx.notify();true
    }
    pub(super) fn reset_pane_sizes(&mut self) {
        let viewport=self.panes.viewport;
        self.panes=PaneSizes{viewport,..PaneSizes::default()};
        self.dragging=false;self.drag=(0.0,0.0,0.0);self.markdown.dragging=false;self.iview.drag=None;
    }
    pub(super) fn resize_handles(&self)->Vec<gpui::Div> {
        if self.resize_blocked(){return Vec::new();}
        self.available_handles().into_iter().map(|(d,x,y,w,h)|{let handle=div().absolute().left(px(x)).top(px(y)).w(px(w)).h(px(h));
            let handle=if matches!(d,Divider::Log|Divider::Panel|Divider::Version){handle.border_t_1()}else{handle.border_l_1()};
            handle.border_color(rgb(self.palette.border))
            .cursor(if matches!(d,Divider::Log|Divider::Panel|Divider::Version){gpui::CursorStyle::ResizeUpDown}else{gpui::CursorStyle::ResizeLeftRight})}).collect()
    }
}
