//! List search previews are separate from the file and commit actually opened.
use super::*;

impl Viewer {
    fn files_have_focus(&self) -> bool { self.layout.files() && self.nav_focused }
    fn log_has_focus(&self) -> bool {
        !self.nav_focused && self.repo.as_ref().map(|r| &r.picker).is_some_and(|p|p.shown && p.log_shown && p.focus == Pane::Log)
    }

    pub(super) fn begin_file_search(&mut self, retain: bool) {
        if !self.file_search.active() {
            self.file_search_scroll = Some(self.nav_scroll);
            self.file_search.start(self.comparison.current);
            if !retain { self.file_search.set_query(String::new()); }
        }
    }
    pub(super) fn begin_log_search(&mut self, retain: bool) {
        let Some(p) = self.repo.as_mut().map(|r| &mut r.picker) else { return };
        if !p.pane_search.active() {
            self.log_search_scroll = Some(p.scroll);
            p.pane_search.start(p.second.unwrap_or(p.anchor) as usize);
            if !retain { p.pane_search.set_query(String::new()); }
        }
    }

    /// Called after modal routing, before plain diff and repository bindings.
    pub(super) fn pane_search_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let files = self.files_have_focus();
        let log = self.log_has_focus();
        let ks = &ev.keystroke;
        let key = ks.key.as_str();
        let cmd = ks.modifiers.control || ks.modifiers.platform;
        let shift = ks.modifiers.shift;
        let alt = ks.modifiers.alt;
        if !files && !log {
            if cmd && !alt && !shift && key=="f" && self.repo.as_ref().map(|r| &r.picker).is_some_and(|p|p.shown && p.sidebar && p.focus==Pane::Sidebar) {
                self.comparison.find=None;
                cx.notify();
                return true;
            }
            return false;
        }
        if cmd && !alt && key == "f" && shift {
            if log {self.repo.as_mut().map(|r| &mut r.picker).unwrap().filter_editing=false;}
            let (query, changed) = self.search.as_ref().map_or((String::new(), false), |s|(s.query.clone(),s.changed_only));
            self.search = Some(SetSearch::new(query, changed, self.comparison.find_options));
        } else if cmd && !alt && key == "f" {
            if log {self.repo.as_mut().map(|r| &mut r.picker).unwrap().filter_editing=false;}
            // A diff find bar from a different pane must not steal list typing.
            self.comparison.find = None;
            if files { self.begin_file_search(true); } else { self.begin_log_search(true); }
        } else if cmd && !alt && !shift && key == "c" {
            let text = if files {
                self.set.comparisons.get(self.file_search.current().unwrap_or(self.comparison.current)).map(|c|c.path.clone())
            } else {
                self.repo.as_ref().map(|r| &r.picker).and_then(|p|p.clipboard_text(&crate::repository_session::RepositoryCommand::CopyHash,
                    p.pane_search.current().map_or(p.anchor,|r|r as u32)))
            };
            if let Some(text) = text { cx.write_to_clipboard(ClipboardItem::new_string(text)); }
        } else if cmd && !alt && !shift && matches!(key,"a"|"v") {
            self.comparison.find=None;
            if files {self.begin_file_search(true);}else{self.begin_log_search(true);}
            if key=="a" {
                if files {self.file_search.set_query(String::new());}
                else {self.repo.as_mut().map(|r| &mut r.picker).unwrap().pane_search.set_query(String::new());}
            } else if let Some(text)=cx.read_from_clipboard().and_then(|c|c.text()) {
                let text=crate::document::one_line(&text);
                if files {self.file_search.append(&text);}
                else {self.repo.as_mut().map(|r| &mut r.picker).unwrap().pane_search.append(&text);}
            }
        } else if cmd && !alt && !shift && log && matches!(key,"b"|"g"|"m"|"t"|"/") {
            self.log_key(key, false, true, cx);
        } else if !cmd && !alt {
            let active = if files { self.file_search.active() } else { self.repo.as_ref().map(|r| &r.picker).unwrap().pane_search.active() };
            let has_query = if files { !self.file_search.query().is_empty() } else { !self.repo.as_ref().map(|r| &r.picker).unwrap().pane_search.query().is_empty() };
            match key {
                "escape" if active || has_query => {
                    if files {
                        self.file_search.cancel();
                        if active && let Some(scroll)=self.file_search_scroll.take() { self.nav_scroll=scroll; }
                    } else if let Some(p)=self.repo.as_mut().map(|r| &mut r.picker) {
                        p.pane_search.cancel();
                        if active && let Some(scroll)=self.log_search_scroll.take() { p.scroll=scroll; }
                    }
                }
                "enter" if active => {
                    if files {
                        if let Some(index)=self.file_search.confirm() {
                            self.file_search_scroll=Some(self.nav_scroll);
                            self.open(index, Place::Top, cx);
                        }
                    } else if let Some(p)=self.repo.as_mut().map(|r| &mut r.picker) && let Some(row)=p.pane_search.confirm() {
                        self.log_search_scroll=Some(p.scroll);
                        p.select_row(row as u32, false, false);
                        self.pick_change_set(cx);
                    }
                }
                "up" | "down" | "f3" if active || key=="f3" && has_query => {
                    let forward=key=="down" || key=="f3" && !shift;
                    if files { self.file_search.step(forward); self.reveal_file_search(); }
                    else {
                        let rows=self.log_rows();
                        if let Some(p)=self.repo.as_mut().map(|r| &mut r.picker) { p.pane_search.step(forward); reveal_log_search(p, rows); }
                    }
                }
                "backspace" if active => {
                    if files { self.file_search.backspace(); }
                    else { self.repo.as_mut().map(|r| &mut r.picker).unwrap().pane_search.backspace(); }
                }
                "home" | "end" | "pageup" | "pagedown" if active => {
                    // Paging during preview only scrolls rows; Enter owns loading.
                    if files {
                        let page=self.comparison.view.viewport_rows().max(1.0);
                        let max=(self.nav.len() as f32-page).max(0.0);
                        self.nav_scroll=match key {"home"=>0.0,"end"=>max,"pageup"=>(self.nav_scroll-page).max(0.0),_=>(self.nav_scroll+page).min(max)};
                    } else {
                        let page=self.log_rows().max(1.0);
                        let p=self.repo.as_mut().map(|r| &mut r.picker).unwrap();
                        let max=(p.rows() as f32-page).max(0.0);
                        p.scroll=match key {"home"=>0.0,"end"=>max,"pageup"=>(p.scroll-page).max(0.0),_=>(p.scroll+page).min(max)};
                    }
                }
                _ => {
                    let Some(text)=Self::typed(ev).filter(|t| !t.is_empty() && t.chars().all(|c|!c.is_control())) else { return false };
                    self.comparison.find=None;
                    if files { self.begin_file_search(false); self.file_search.append(&text); }
                    else { self.begin_log_search(false); self.repo.as_mut().map(|r| &mut r.picker).unwrap().pane_search.append(&text); }
                }
            }
        } else {
            if log && cmd && !alt && matches!(key,"up"|"down")
                && !self.repo.as_ref().map(|r| &r.picker).unwrap().pane_search.active() { return false; }
            // Keep standard window / pane shortcuts. Other modified characters are
            // consumed here so they cannot fall through into plain diff actions.
            if matches!(key,"tab"|"f10"|"f1"|"f5") || cmd && matches!(key,"q"|"w"|"p"|"r"|"l"|"z"|"y"|"enter"|"["|"]")
                || cmd && key=="s"
                || alt && matches!(key,"1"|"2"|"3") { return false; }
        }
        cx.notify();
        true
    }

    fn reveal_file_search(&mut self) {
        let Some(index)=self.file_search.current() else { return };
        let Some(row)=self.nav.iter().position(|r|matches!(r,NavRow::File{index:i,..} if *i==index)) else { return };
        let visible=self.comparison.view.viewport_rows().max(1.0);
        let row=row as f32;
        if row<self.nav_scroll { self.nav_scroll=row; }
        else if row>=self.nav_scroll+visible { self.nav_scroll=(row-visible+1.0).max(0.0); }
    }

    pub(super) fn poll_pane_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let before=self.file_search.current();
        let changed=self.file_search.poll(self.set.comparisons.len(), |i,q|crate::pane_search::contains(&self.set.comparisons[i].path,q));
        if changed { if self.file_search.current()!=before { self.reveal_file_search(); } cx.notify(); }
        let log_rows=self.log_rows();
        let mut pending=self.file_search.searching(self.set.comparisons.len());
        if let Some(p)=self.repo.as_mut().map(|r| &mut r.picker) {
            let before=p.pane_search.current();
            if p.poll_pane_search() { if p.pane_search.current()!=before { reveal_log_search(p,log_rows); } cx.notify(); }
            pending |= p.pane_search.searching(p.rows() as usize);
        }
        if pending { window.request_animation_frame(); }
    }

    pub(super) fn file_search_header(&self) -> String {
        if self.file_search.active() || !self.file_search.query().is_empty() {
            let mut status=self.file_search.status();
            if self.file_search.searching(self.set.comparisons.len()) {
                status=status.replace("No matches","0 matches so far");
                status.push_str(" · searching…");
            }
            status
        } else { self.nav_header() }
    }
}

fn reveal_log_search(p: &mut Picker, visible: f32) {
    let Some(row)=p.pane_search.current() else { return };
    let row=row as f32;
    let visible=visible.max(1.0);
    if row<p.scroll {p.scroll=row;}
    else if row>=p.scroll+visible {p.scroll=(row-visible+1.0).max(0.0);}
}
