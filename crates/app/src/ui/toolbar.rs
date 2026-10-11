//! The toolbar (frequent actions left, panels right), the Diff options popover and the
//! diff header's Source | Rendered switch. One slot list feeds rendering and the adapter tests.
use super::*;
use crate::catalog::{self, Availability};
use crate::layout::LayoutCommand;
use std::borrow::Cow;

/// One toolbar button.
pub(super) struct Slot { pub id:&'static str, pub label:Cow<'static,str>, pub tip:String, pub active:bool, pub dim:bool, pub tool:Tool, pub right:bool,
    /// In the toolbar's middle (a pull request's tabs).
    pub center:bool,
    /// A fixed width: Focus and Overview share one, so the buttons after them never move.
    pub width:Option<f32> }

fn slot(id:&'static str,label:&'static str,tip:&str,tool:Tool)->Slot{Slot{id,label:label.into(),tip:tip.into(),active:false,dim:false,tool,right:false,center:false,width:None}}

/// Each popover row's choices as (x, width).
type Cells = Vec<Vec<(f32,f32)>>;
/// A row's label and its choices: label, x, current, focused.
type PaintRow = (&'static str,Vec<(String,f32,bool,bool)>);

/// The Diff popover: the focused row and choice, and where the button was.
#[derive(Clone,Copy,Debug,Default)]
pub(super) struct DiffOptions { pub row: usize, pub col: usize, pub x: f32 }

impl Viewer {
    /// What the toolbar shows at the current width, in order.
    pub(super) fn toolbar_slots(&self)->Vec<Slot>{
        let width=self.panes.viewport[0];
        let mut out=Vec::new();
        if let Some(s)=&self.merge {
            use crate::merge_session::Act;
            let mut m=|id,label,tip:&str,act:Act|{let why=s.disabled(act);out.push(Slot{dim:why.is_some(),tip:why.map_or_else(||tip.to_string(),|why|format!("{tip} — {why}")),..slot(id,label,"",Tool::Merge(act))});};
            if width>=640. {
                m("m-prev","↑","Previous change",Act::PrevChange);m("m-next","↓","Next change",Act::NextChange);
                m("m-prev-c","⇡","Previous unresolved conflict",Act::PrevConflict);m("m-next-c","⇣","Next unresolved conflict",Act::NextConflict);
            }
            m("m-undo","↶","Undo (Ctrl/Cmd+Z)",Act::Undo);m("m-redo","↷","Redo (Ctrl/Cmd+Shift+Z)",Act::Redo);m("m-save","⤓","Save result (Ctrl/Cmd+S)",Act::Save);
            if width>=640. {m("m-done","Done","Save and finish / next unresolved file",Act::Done);m("m-cancel","Cancel","Close merge view (Esc)",Act::Cancel);}
        } else if let Some(s)=&self.picker_v {
            use version_picker::PickAct;
            let mut acts=vec![("p-undo","↶",PickAct::Undo),("p-redo","↷",PickAct::Redo),("p-save","⤓",PickAct::Save),("p-done","Done",PickAct::Done),("p-cancel","Cancel",PickAct::Cancel)];
            if !s.mergetool && width>=640. {acts.push(("p-mark","Mark resolved",PickAct::MarkResolved));}
            for (id,label,act) in acts {let why=s.disabled(act);out.push(Slot{dim:why.is_some(),tip:why.map_or_else(||act.label().to_string(),|why|format!("{} — {why}",act.label())),..slot(id,label,"",Tool::Pick(act))});}
        } else if self.layout.shows_focus(self.repo.as_ref()) {
            if self.repo.is_some() {
                out.push(Slot{width:Some(self.mode_button_w()),..slot("overview","Overview","Back to Overview (Esc)",Tool::Layout(LayoutCommand::LeaveFocus))});
                out.push(slot("nav-back","◀","Back to the previous place (Alt+Left)",Tool::Back));
                out.push(slot("nav-forward","▶","Forward again (Alt+Right)",Tool::Forward));
            }
            // a details tab hides the diff: its controls wait for the Diff tab
            let diff=!self.pr_details_shown();
            if diff {
                out.push(slot("prev-change","↑","Previous change (p / Shift-F7)",Tool::PrevChange));
                out.push(slot("next-change","↓","Next change (n / F7)",Tool::NextChange));
                if width>=640. {out.push(slot("prev-file","←","Previous file (Ctrl/Cmd+[)",Tool::PrevFile));out.push(slot("next-file","→","Next file (Ctrl/Cmd+])",Tool::NextFile));}
            }
            if diff {
                // the Overview's Diff menu, layout included
                if width>=640. && self.image_doc().is_none() {
                    out.push(Slot{active:self.diff_options.is_some(),..slot("diff-options","⚙","Layout, whitespace, highlighting, scrolling, context, case and blame",Tool::DiffOptions)});
                }
            }
            // the PR's tabs, in the middle, replace PR ▾ in Focus
            for (n,e) in catalog::pr_tabs(&self.scope()).into_iter().enumerate() {
                let id=["pr-tab-conversation","pr-tab-commits","pr-tab-files"].get(n).copied().unwrap_or("pr-tab");
                out.push(Slot{label:e.label.clone(),active:e.current,center:true,..slot(id,"",&e.label,Tool::Run(e.action))});
            }
            out.push(Slot{right:true,..slot("help","?","Keyboard shortcuts (?)",Tool::Help)});
        } else {
            out.push(slot("prev-change","↑","Previous change (p / Shift-F7)",Tool::PrevChange));
            out.push(slot("next-change","↓","Next change (n / F7)",Tool::NextChange));
            if width>=640. {out.push(slot("prev-file","←","Previous file (Ctrl/Cmd+[)",Tool::PrevFile));out.push(slot("next-file","→","Next file (Ctrl/Cmd+])",Tool::NextFile));}
            if width>=640. && self.image_doc().is_none() {out.push(Slot{active:self.diff_options.is_some(),..slot("diff-options","⚙","Layout, whitespace, highlighting, scrolling, context, case and blame",Tool::DiffOptions)});}
            out.push(Slot{right:true,..slot("help","?","Keyboard shortcuts (?)",Tool::Help)});
        }
        let focus=self.merge.is_none()&&self.picker_v.is_none()&&self.layout.shows_focus(self.repo.as_ref());
        // Overview: ☰, Focus (where Focus shows Overview), then Back and Forward
        if !focus && self.repo.is_some() {
            out.insert(0,slot("nav-back","◀","Back to the previous place (Alt+Left)",Tool::Back));
            out.insert(1,slot("nav-forward","▶","Forward again (Alt+Right)",Tool::Forward));
            if self.merge.is_none()&&self.picker_v.is_none() {
                out.insert(0,Slot{width:Some(self.mode_button_w()),..slot("focus","Focus","Focus on this selection: Files and the diff (f, Enter, double-click)",Tool::Layout(LayoutCommand::ToggleFocus))});
            }
        }
        let app_menu_open=self.menu.as_ref().is_some_and(|m|matches!(m.items.first(),Some((_,MenuAction::Run(catalog::Action::Help(catalog::HelpAction::Settings))))));
        out.insert(0,Slot{active:app_menu_open,..slot("app-menu","☰","Settings, theme, keyboard shortcuts, about",Tool::AppMenu)});
        // a button that can do nothing now looks it, and says why
        for s in out.iter_mut().filter(|s|!s.dim) {
            if let Some(why)=self.slot_disabled(s.tool) {s.dim=true;s.tip=format!("{} — {why}",s.tip);}
        }
        out
    }

    /// The Selection Title the Focus toolbar shows after Overview.
    /// The bottom bar's panel switches, glued left to right: Branches and History (repository views), Files.
    /// Focus refuses Branches and History; they stay, disabled, so the switch keeps its shape.
    pub(super) fn panel_switch(&self)->Vec<SwitchCell>{
        let panels=self.layout.panels(self.repo.as_ref());
        let mut cells=Vec::new();
        if self.repo.is_some() {cells.extend([("Branches",panels.branches,LayoutCommand::ToggleBranches),("History",panels.history,LayoutCommand::ToggleHistory)]);}
        cells.push(("Files",panels.files,LayoutCommand::ToggleFiles));
        let y=self.panes.viewport[1];
        let mut x=8.;
        cells.into_iter().map(|(label,on,command)|{
            let w=(label.len()+2) as f32*self.char_w;
            let cell=SwitchCell{x,y,w,label,on,enabled:self.layout.disabled(command,self.repo.as_ref()).is_none(),command};
            x+=w;cell
        }).collect()
    }

    /// The bottom bar's right end: the palette's key (the toolbar has no button for it). x, width, label.
    pub(super) fn palette_hint(&self)->(f32,f32,&'static str){
        let label=if cfg!(target_os="macos") {"Commands  Cmd+Shift+P"} else {"Commands  Ctrl+Shift+P"};
        let w=(label.chars().count()+2) as f32*self.char_w;
        ((self.panes.viewport[0]-8.-w).max(0.),w,label)
    }

    /// A click in the bottom bar; true when it was there.
    pub(super) fn panel_switch_click(&mut self,x:f32,y:f32,cx:&mut Context<Self>)->bool{
        if y<self.panes.viewport[1] {return false;}
        let (hx,hw,_)=self.palette_hint();
        if x>=hx&&x<hx+hw {self.open_palette(cx);return true;}
        if let Some(c)=self.panel_switch().into_iter().find(|c|x>=c.x&&x<c.x+c.w&&y>=c.y) {
            match self.layout.disabled(c.command,self.repo.as_ref()) {
                Some(why)=>self.toast(why.into(),cx),
                None=>self.layout_command(c.command,cx),
            }
        }
        true
    }

    /// The bottom bar: the panel switches, drawn where `panel_switch` places them.
    pub(super) fn footer(&self)->gpui::AnyElement{
        let p=&self.palette;
        let mut bar=div().h(px(footer_h())).w_full().flex_shrink_0().relative().bg(rgb(p.header_bg)).border_t_1().border_color(rgb(p.border))
            .text_size(px(font_size())).font_family(self.font.family.clone());
        let cells=self.panel_switch();
        let (first,last)=(cells.first().map(|c|c.x),cells.last().map(|c|c.x+c.w));
        if let (Some(a),Some(b))=(first,last) {
            // one outline around the group: the switches read as a single control
            bar=bar.child(div().absolute().left(px(a-1.)).top(px(2.)).w(px(b-a+2.)).h(px(footer_h()-5.)).rounded(px(4.)).border_1().border_color(rgb(p.border)));
        }
        for (i,c) in cells.into_iter().enumerate() {
            let color=if !c.enabled {p.dim} else {p.fg};
            bar=bar.child(div().absolute().left(px(c.x)).top(px(3.)).w(px(c.w)).h(px(footer_h()-7.)).flex().items_center().justify_center()
                .when(c.on,|d|d.bg(rgb(p.selected_bg))).when(i>0,|d|d.border_l_1().border_color(rgb(p.border)))
                .text_color(rgb(color)).child(c.label));
        }
        let (hx,hw,label)=self.palette_hint();
        bar=bar.child(div().absolute().left(px(hx)).top(px(3.)).w(px(hw)).h(px(footer_h()-7.)).flex().items_center().justify_center()
            .rounded(px(4.)).text_color(rgb(p.dim)).hover(|d|d.bg(rgb(p.status_bg))).child(label));
        bar.into_any_element()
    }

    /// Focus and Overview: room for the longer label.
    fn mode_button_w(&self)->f32 {("Overview".len()+2) as f32*self.char_w}

    /// Only while Files is hidden: otherwise Files' header names the selection.
    pub(super) fn focus_title(&self)->Option<crate::selection_title::SelectionTitle>{
        if !self.layout.focused(self.repo.as_ref()) || self.layout.files() {return None;}
        self.repo.as_ref()?.selection_title()
    }

    /// A toolbar click: with a dropdown open it only closes it (so its own button toggles it); unavailable buttons
    /// do nothing; otherwise the button's tool runs.
    pub(super) fn toolbar_click(&mut self,tool:Tool,x:f32,window:&mut Window,cx:&mut Context<Self>){
        if self.menu.is_some()||self.command_palette.is_some()||self.diff_options.is_some() {
            (self.menu,self.command_palette,self.diff_options)=(None,None,None);
            cx.notify();return;
        }
        if self.resize_blocked()||self.slot_disabled(tool).is_some() {return;}
        self.tool(tool,x,window,cx);
    }

    /// Why a toolbar button can do nothing now, from the shared catalog's availability.
    pub(super) fn slot_disabled(&self,tool:Tool)->Option<std::borrow::Cow<'static,str>>{
        use crate::commands::Command as C;
        let action=match tool {
            Tool::PrevChange=>catalog::Action::Comparison(C::PreviousChange),
            Tool::NextChange=>catalog::Action::Comparison(C::NextChange),
            Tool::PrevFile=>catalog::Action::Comparison(C::PreviousFile),
            Tool::NextFile=>catalog::Action::Comparison(C::NextFile),
            Tool::Merge(act)=>catalog::Action::Merge(act),
            Tool::Pick(act)=>catalog::Action::Pick(act),
            Tool::Back=>catalog::Action::Repository(catalog::RepoAction::Back),
            Tool::Forward=>catalog::Action::Repository(catalog::RepoAction::Forward),
            Tool::Layout(command)=>catalog::Action::Layout(command),
            Tool::Run(action)=>action,
            Tool::DiffOptions|Tool::AppMenu|Tool::Help=>return None,
            #[cfg(all(test,feature="gui-test"))]
            Tool::About|Tool::Theme=>return None,
        };
        Availability::disabled(self,action)
    }

    #[cfg(all(test,feature="gui-test"))]
    pub(super) fn toolbar_ids(&self)->Vec<&'static str>{self.toolbar_slots().iter().map(|s|s.id).collect()}

    pub(super) fn toolbar(&self,cx:&mut Context<Self>)->gpui::AnyElement{
        let p=&self.palette;
        let mut left=div().flex().flex_row().items_center().gap_1();
        let mut right=div().flex().flex_row().items_center().gap_1();
        let mut center=div().flex().flex_row().items_center().gap_1();
        let slots=self.toolbar_slots();
        // the middle group sits at the window's middle when the left group leaves room, else just after it
        let w=|s:&Slot|s.width.unwrap_or((s.label.chars().count()+1) as f32*self.char_w.max(7.)+8.)+4.;
        let (left_w,center_w):(f32,f32)=(slots.iter().filter(|s|!s.right&&!s.center).map(w).sum(),slots.iter().filter(|s|s.center).map(w).sum());
        let middle=left_w+16.<(self.panes.viewport[0]-center_w)/2.;
        for s in slots {
            let (tool,tip,palette)=(s.tool,s.tip,p.clone());
            let id=s.id;
            let button=div().id(id).debug_selector(move||id.into()).h(px(22.)).px_1().flex().flex_shrink_0().items_center().rounded(px(4.))
                .text_color(rgb(if s.dim{p.dim}else{p.fg})).when_some(s.width,|d,w|d.w(px(w)).justify_center())
                .when(s.active,|d|d.bg(rgb(p.selected_bg))).when(!s.dim,|d|d.hover(|d|d.bg(rgb(p.status_bg)))).child(SharedString::from(s.label))
                // an open popover or palette would sit under its own button's tooltip
                .when(!s.active||matches!(tool,Tool::Layout(_)),|d|d.tooltip(move|_,cx|cx.new(|_|Tip(tip.clone().into(),palette.clone())).into()))
                .on_mouse_down(MouseButton::Left,cx.listener(move|v,ev:&MouseDownEvent,w,cx|{cx.stop_propagation();v.toolbar_click(tool,f32::from(ev.position.x),w,cx);}));
            if s.center {center=center.child(button);} else if s.right {right=right.child(button);} else {left=left.child(button);}
            if id=="overview" && self.panes.viewport[0]>=900. && let Some(t)=self.focus_title() {
                let (detail,palette)=(t.detail,p.clone());
                left=left.child(div().id("selection-title").px_2().max_w(px(360.)).overflow_hidden().whitespace_nowrap().text_color(rgb(p.dim))
                    .child(t.text).tooltip(move|_,cx|cx.new(|_|Tip(detail.clone().into(),palette.clone())).into()));
            }
        }
        if self.panes.viewport[0]>=900. && let Some(s)=&self.merge {left=left.child(div().px_1().text_color(rgb(p.dim)).child(s.counters()));}
        let bar=div().h(px(toolbar_h())).w_full().relative().flex().flex_row().items_center().justify_between().px_1()
            .bg(rgb(p.header_bg)).border_b_1().border_color(rgb(p.border)).text_color(rgb(p.fg))
            .text_size(px(font_size())).font_family(self.font.family.clone());
        if middle {
            // over the bar, centred; its empty sides take no clicks
            bar.child(left).child(right).child(div().absolute().top_0().left_0().size_full().flex().justify_center().items_center().child(center)).into_any_element()
        } else {
            bar.child(left).child(div().flex_1().flex().justify_center().child(center)).child(right).into_any_element()
        }
    }

    pub(super) fn open_diff_options(&mut self,cx:&mut Context<Self>){self.toggle_diff_options(self.panes.viewport[0]/2.,cx);}

    pub(super) fn toggle_diff_options(&mut self,x:f32,cx:&mut Context<Self>){
        if self.diff_options.take().is_some(){cx.notify();return;}
        if self.modal().is_some_and(|m|m!=Modal::Menu){return;}
        let rows=catalog::diff_options(&self.scope());
        let Some(first)=rows.first() else {return};
        self.menu=None;
        self.diff_options=Some(DiffOptions{row:0,col:first.choices.iter().position(|c|c.current).unwrap_or(0),x});
        cx.notify();
    }

    /// Box x, y, width, height, and each row's choices as (x, width).
    pub(super) fn diff_options_layout(&self)->Option<(f32,f32,f32,f32,Cells)>{
        let state=self.diff_options?;
        let rows=catalog::diff_options(&self.scope());
        let label_w=12.*self.char_w;
        let widths:Vec<Vec<f32>>=rows.iter().map(|r|r.choices.iter().map(|c|(choice_label(c).chars().count()+2) as f32*self.char_w).collect()).collect();
        let inner=widths.iter().map(|w|w.iter().sum::<f32>()+w.len() as f32*6.).fold(0.,f32::max);
        let [vw,vh]=self.panes.viewport;
        let width=(label_w+inner+24.).min((vw-8.).max(0.));
        let height=rows.len() as f32*line_h()+8.;
        let (x,y)=(state.x.clamp(4.,(vw-width-4.).max(4.)),(toolbar_h()+2.).min((vh-height-1.).max(0.)));
        let cells=widths.iter().map(|w|{let mut at=x+12.+label_w;w.iter().map(|&cw|{let cell=(at,cw);at+=cw+6.;cell}).collect()}).collect();
        Some((x,y,width,height,cells))
    }

    #[cfg(all(test,feature="gui-test"))]
    pub(super) fn diff_option_point(&self,row:usize,col:usize)->(f32,f32){
        let (_,y,_,_,cells)=self.diff_options_layout().expect("popover open");
        let (x,w)=cells[row][col];(x+w/2.,y+4.+row as f32*line_h()+line_h()/2.)
    }

    fn choose_diff_option(&mut self,row:usize,col:usize,window:&mut Window,cx:&mut Context<Self>){
        let rows=catalog::diff_options(&self.scope());
        let Some(choice)=rows.get(row).and_then(|r|r.choices.get(col)) else {return};
        let action=choice.action;
        if let Some(reason)=Availability::disabled(&*self,action){self.toast(reason.into_owned(),cx);return;}
        self.run_action(action,window,cx);
        // stays open for more changes
        self.diff_options=Some(DiffOptions{row,col,x:self.diff_options.map_or(self.panes.viewport[0]/2.,|s|s.x)});
        cx.notify();
    }

    pub(super) fn diff_options_key(&mut self,ev:&KeyDownEvent,window:&mut Window,cx:&mut Context<Self>)->bool{
        let Some(mut state)=self.diff_options else {return false};
        let rows=catalog::diff_options(&self.scope());
        let count=|r:usize|rows.get(r).map_or(1,|row|row.choices.len().max(1));
        // a row is entered at its current value
        let current=|r:usize|rows.get(r).and_then(|row|row.choices.iter().position(|c|c.current)).unwrap_or(0);
        match ev.keystroke.key.as_str() {
            "escape"=>{self.diff_options=None;cx.notify();return true;}
            "up"=>{state.row=state.row.saturating_sub(1);state.col=current(state.row);}
            "down"=>{state.row=(state.row+1).min(rows.len().saturating_sub(1));state.col=current(state.row);}
            "left"=>state.col=state.col.saturating_sub(1),
            "right"=>state.col=(state.col+1).min(count(state.row)-1),
            "enter"|"space"=>{self.diff_options=Some(state);self.choose_diff_option(state.row,state.col,window,cx);return true;}
            _=>{}
        }
        self.diff_options=Some(state);cx.notify();true
    }

    /// A click on a choice chooses it; a click outside closes. True when the popover was open.
    pub(super) fn diff_options_click(&mut self,x:f32,y:f32,window:&mut Window,cx:&mut Context<Self>)->bool{
        let Some((bx,by,bw,bh,cells))=self.diff_options_layout() else {return false};
        if !(x>=bx&&x<bx+bw&&y>=by&&y<by+bh) {self.diff_options=None;cx.notify();return true;}
        let row=((y-by-4.)/line_h()).floor();
        if row>=0. && let Some(col)=cells.get(row as usize).and_then(|r|r.iter().position(|&(cx_,w)|x>=cx_&&x<cx_+w)) {
            self.choose_diff_option(row as usize,col,window,cx);
        }
        true
    }

    /// Lines to paint: box, and per row its label and (choice, x, current, focused).
    pub(super) fn diff_options_paint(&self)->Option<DiffOptionsPaint>{
        let state=self.diff_options?;
        let (x,y,width,height,cells)=self.diff_options_layout()?;
        let rows=catalog::diff_options(&self.scope()).into_iter().zip(cells).enumerate().map(|(r,(row,cells))|{
            (row.label,row.choices.iter().zip(cells).enumerate().map(|(c,(choice,(cx_,_)))|(choice_label(choice),cx_,choice.current,state.row==r&&state.col==c)).collect())
        }).collect();
        Some(DiffOptionsPaint{x,y,width,height,rows})
    }

    /// The diff header's switch at its right end: Source | Preview for Markdown, the image modes for images.
    /// Each cell: x, width, label, current, action.
    pub(super) fn header_switch(&self)->Vec<(f32,f32,String,bool,catalog::Action)>{
        let mut entries=catalog::header_switch(&self.scope());
        let pr=|e:&catalog::Entry|matches!(e.action,catalog::Action::Pr(_));
        // nothing of the file's until it is read, so a non-Markdown file never flashes Source | Preview
        if self.doc().is_none() {entries.retain(pr);}
        let room=self.panes.viewport[0]-8.-(self.diff_x()+8.);
        let width=|es:&[catalog::Entry]|es.iter().map(|e|(e.label.chars().count()+2) as f32*self.char_w+4.).sum::<f32>();
        // too narrow: the PR tabs give way first (the palette lists them), then everything
        if width(&entries)>room {entries.retain(|e|!pr(e));}
        if width(&entries)>room {entries.clear();}
        // the PR's tabs sit in the middle of the header, the file's own modes at its right end
        let (tabs,modes):(Vec<_>,Vec<_>)=entries.into_iter().partition(|e|pr(e));
        let lay=|es:Vec<catalog::Entry>,mut x:f32|->Vec<(f32,f32,String,bool,catalog::Action)>{
            es.into_iter().enumerate().map(|(n,e)|{if n>0{x+=4.;}let w=(e.label.chars().count()+2) as f32*self.char_w;let cell=(x,w,e.label.into_owned(),e.current,e.action);x+=w;cell}).collect()
        };
        let right=self.panes.viewport[0]-8.;
        let modes_x=right-width(&modes)+4.*(modes.len() as f32).min(1.);
        let tabs_w=width(&tabs)-4.*(tabs.len() as f32).min(1.);
        let middle=(self.diff_x()+right-tabs_w)/2.;
        let mut cells=lay(tabs,middle.min(modes_x-8.-tabs_w).max(self.diff_x()+8.));
        cells.extend(lay(modes,modes_x));
        cells
    }

    #[cfg(all(test,feature="gui-test"))]
    pub(super) fn header_switch_labels(&self)->Vec<String>{self.header_switch().into_iter().map(|c|c.2).collect()}

    /// A click on the header switch; true when it was one.
    pub(super) fn header_switch_click(&mut self,x:f32,y:f32,window:&mut Window,cx:&mut Context<Self>)->bool{
        if !(self.top()-hdr_h()..self.top()).contains(&y) {return false;}
        let Some(action)=self.header_switch().into_iter().find(|c|x>=c.0&&x<c.0+c.1).map(|c|c.4) else {return false};
        if let Some(reason)=Availability::disabled(&*self,action){self.toast(reason.into_owned(),cx);return true;}
        self.run_action(action,window,cx);true
    }
}

/// One panel switch in the bottom bar.
pub(super) struct SwitchCell { pub x:f32, pub y:f32, pub w:f32, pub label:&'static str, pub on:bool, pub enabled:bool, pub command:LayoutCommand }

pub(super) struct DiffOptionsPaint { pub x:f32, pub y:f32, pub width:f32, pub height:f32, pub rows:Vec<PaintRow> }

/// "Whitespace: Trim" shows as "Trim" in its row; toggles carry a box.
fn choice_label(choice:&catalog::Entry)->String{
    match choice.label.split_once(": ") {
        Some((_,value))=>value.to_string(),
        None=>format!("{} {}",if choice.current{"☑"}else{"☐"},choice.label),
    }
}
