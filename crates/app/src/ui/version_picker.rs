//! The Version Picker: a Conflicted File resolved by taking one version whole (binary, undecodable or very large
//! under `git mergetool`). One row per version above the usual 2-way view of Ours ⟷ Theirs, which shows images in
//! the image compare and other binaries as sizes; the version taken is the Result, saved exactly.

use super::*;
pub use crate::version_session::{Opened, PickAct, PickerState, open_files};

/// The band of version rows painted over the bottom of the diff area.
pub(super) struct PickerFrame {
    rows: Vec<(String, bool)>,
    height: f32,
    scroll: usize,
    head: String,
    ask: Option<String>,
}

impl PickerFrame {
    pub fn height(&self) -> f32 {
        self.height
    }
}

impl Viewer {
    /// Show `opened` as a Version Picker over the 2-way view. `mergetool`: one `git mergetool` file.
    pub fn start_picker(&mut self, opened: Opened, mergetool: bool, cx: &mut Context<Self>) {
        if !self.edits_may_close(cx) {return;}
        if mergetool {
            merge_view::set_exit(1);
        }
        self.picker_v = Some(PickerState::new(opened, mergetool));
        self.layout.set_files(false);
        self.panes.version_scroll=0.0;
        cx.notify();
    }

    pub(super) fn picker_v_title(&self) -> Option<String> {
        let s = self.picker_v.as_ref()?;
        Some(format!(
            "{} — pick a version{} — Lenz",
            s.o.path,
            if s.dirty { " ●" } else { "" }
        ))
    }

    /// The band's height in pixels (0 without a Version Picker).
    pub(super) fn picker_v_height(&self) -> f32 {
        let Some(s)=&self.picker_v else{return 0.0;};
        let default=(s.o.versions.len()+s.o.info.len()+2) as f32*line_h();
        crate::pane_layout::clamp_size((self.panes.viewport[1]-self.top()-self.panel_h()).max(0.0),self.panes.version.unwrap_or(default),3.0*line_h(),2.0*line_h())
    }

    pub(super) fn picker_v_frame(&self) -> Option<PickerFrame> {
        let s = self.picker_v.as_ref()?;
        let mut rows: Vec<(String, bool)> =
            s.o.versions
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let note = v
                        .error
                        .as_ref()
                        .map_or(v.note.clone(), |e| format!("can't be read: {e}"));
                    (
                        format!(
                            "{} {}  {}  —  {note}",
                            if s.chosen == Some(i) { "✓" } else { " " },
                            i + 1,
                            v.label
                        ),
                        s.chosen == Some(i),
                    )
                })
                .collect();
        rows.extend(s.o.info.iter().map(|l| {
            (
                if l.starts_with("$ ") {
                    format!("{l}   ⧉ click copies")
                } else {
                    l.clone()
                },
                false,
            )
        }));
        let anyway = if s.o.large_text {
            " · m merges it line by line anyway"
        } else {
            ""
        };
        let head = if s.o.versions.is_empty() {
            format!("{} is {}", s.o.path, s.o.why)
        } else {
            let mark = if s.mergetool {
                ""
            } else {
                " · then right-click the file: Mark resolved"
            };
            let base_left = if s.mergetool && self.set.comparisons.len() > 1 {
                " · b Base on the left"
            } else {
                ""
            };
            format!(
                "{} is {}: take one version whole — 1–{} take · ctrl-z undo · ctrl-s save · ctrl-enter Done · Esc cancel{anyway}{base_left}{mark}",
                s.o.path,
                s.o.why,
                s.o.versions.len(),
            )
        };
        let ask = s.question().map(str::to_owned);
        let height=self.picker_v_height();
        let visible=((height-2.0*line_h())/line_h()).max(0.0).floor() as usize;
        let scroll=(self.panes.version_scroll.floor() as usize).min(rows.len().saturating_sub(visible.max(1)));
        Some(PickerFrame { rows, head, ask, height, scroll })
    }

    /// A click in the band: take the row's version. `y` from the top of the diff area's panes.
    pub(super) fn picker_v_click(&mut self, y: f32, cx: &mut Context<Self>) -> bool {
        let Some(frame)=self.picker_v_frame()else{return false;};
        if y<line_h()||y>=frame.height()-line_h(){return true;}
        let scroll=frame.scroll;
        let Some(s) = self.picker_v.as_mut() else {return false;};
        let row = (y / line_h()).floor() as i64 - 1 + scroll as i64;
        let n = s.o.versions.len();
        if row >= 0 && (row as usize) < n {
            s.take(row as usize);
            cx.notify();
            return true;
        }
        let Some(_) = usize::try_from(row).ok().and_then(|r| s.o.info.get(r - n)) else {
            return false;
        };
        if let Some(cmd) = s.command(row as usize - n) {
            cx.write_to_clipboard(ClipboardItem::new_string(cmd));
            self.toast("Command copied".into(), cx);
        }
        true
    }

    /// Scroll the version band while retaining version numbers and keyboard shortcuts.
    pub(super) fn picker_v_wheel(&mut self,x:f32,y:f32,rows:f32,cx:&mut Context<Self>)->bool {
        let Some(frame)=self.picker_v_frame()else{return false;};
        let top=self.panes.viewport[1]-self.panel_h()-frame.height();
        if x<self.diff_x()||y<top||y>=top+frame.height(){return false;}
        if self.resize_blocked(){return true;}
        let visible=((frame.height()-2.0*line_h())/line_h()).max(0.0).floor() as usize;
        let max=frame.rows.len().saturating_sub(visible.max(1)) as f32;
        self.panes.version_scroll=(self.panes.version_scroll+rows).clamp(0.0,max);
        cx.notify();true
    }

    pub(super) fn picker_v_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Some(s) = self.picker_v.as_mut() else {
            return false;
        };
        let k = &ev.keystroke;
        let (key, shift, ctrl) = (
            k.key.as_str(),
            k.modifiers.shift,
            k.modifiers.control || k.modifiers.platform,
        );
        // Preserve the viewer's explicit quit routes through a question.
        // Mergetool q/Ctrl-W retain their existing Cancel confirmation below.
        if s.ask.is_some()
            && (key == "q" || key == "w" && ctrl)
            && !(s.mergetool && !shift && (key == "w" || !ctrl))
        {
            self.quit_if_ready(cx);
            return true;
        }
        // A confirmation consumes preview/menu input; only its existing
        // Save/Done/Cancel/Reload keys may reach the outer version workflow.
        if s.ask.is_some()
            && !matches!((key, shift, ctrl),
                ("s" | "enter", false, true) | ("escape", _, _)
                | ("r" | "q", false, false) | ("w", false, true))
        {
            return true;
        }
        if key==":" {self.open_palette(cx);return true;}
        let mergetool = s.mergetool;
        match (key, shift, ctrl) {
            (d, false, false) if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() && d != "0" => {
                s.take(d.parse::<usize>().unwrap_or(1) - 1);
            }
            ("z", false, true) => return self.picker_v_act(PickAct::Undo, cx),
            ("z", true, true) | ("y", false, true) => return self.picker_v_act(PickAct::Redo, cx),
            ("s", false, true) => return self.picker_v_act(PickAct::Save, cx),
            ("enter", false, true) => return self.picker_v_act(PickAct::Done, cx),
            ("r", false, false)
                if matches!(
                    s.ask,
                    Some(merge_view::Ask::ChangedOnDisk | merge_view::Ask::Overwrite { .. })
                ) =>
            {
                return self.picker_v_act(PickAct::Reload, cx);
            }
            ("escape", ..) if s.ask.is_some_and(|ask| ask != merge_view::Ask::Close) => {
                s.dismiss();
            }
            ("escape", ..) | ("q", false, false) | ("w", false, true)
                if mergetool || key == "escape" =>
            {
                return self.picker_v_act(PickAct::Cancel, cx);
            }
            ("b", false, false) if s.mergetool && self.set.comparisons.len() > 1 => {
                let next = 1 - self.comparison.current.min(1);
                self.open(next, Place::Top, cx);
            }
            ("m", false, false) if s.o.large_text => {
                let Some([b, o, t, m]) = s.o.files.clone() else {
                    return false;
                };
                let cwd = std::env::current_dir().unwrap_or_default();
                match merge_view::open_files(&b, &o, &t, &m, &cwd) {
                    Ok(opened) => {
                        self.picker_v = None;
                        self.start_merge(opened, merge_view::exit_code() == 1, cx);
                    }
                    Err(e) => self.comparison.banner = Some(e),
                }
            }
            // image modes, zoom and the rest of the viewer's keys still apply to the view below
            _ => return s.ask.is_some(),
        }
        cx.notify();
        true
    }

    /// Save, Done and Cancel, from their keys or the toolbar.
    pub(super) fn picker_v_act(&mut self, act: PickAct, cx: &mut Context<Self>) -> bool {
        let Some(s) = self.picker_v.as_mut() else {
            return false;
        };
        let outcome = s.execute(act);
        if let Some(e) = outcome.error {
            self.comparison.banner = Some(e);
        }
        if let Some(t) = outcome.toast {
            self.toast(t, cx);
        }
        if let Some(code) = outcome.quit && self.edits_may_close(cx) {merge_view::set_exit(code);cx.quit();}
        if outcome.done {
            self.conflict_done(cx);
        }
        if outcome.back {
            self.picker_v = None;
            self.toast(
                "Version Picker closed — click the file to open it again".into(),
                cx,
            );
        }
        if let Some(path) = outcome.mark {
            self.picker_v = None;
            self.mark_resolved(path, cx);
        }
        cx.notify();
        true
    }

    /// Whether the window may close now; otherwise asks first.
    pub fn picker_v_may_close(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(s) = self.picker_v.as_mut() else {
            return true;
        };
        let ready = s.may_close();
        if !ready {
            self.menu = None;
            self.command_palette = None;
            self.diff_options = None;
            cx.notify();
        }
        ready
    }
}

/// Paint the band at `(x0, y0)`, `w` wide.
#[allow(clippy::too_many_arguments)] // Palette plus adapter layout inputs.
pub(super) fn paint(palette: &theme::Palette,
    window: &mut Window,
    cx: &mut App,
    font: &Font,
    x0: f32,
    y0: f32,
    w: f32,
    f: &PickerFrame,
) {
    let h = f.height();
    let bounds=Bounds{origin:point(px(x0),px(y0)),size:size(px(w.max(0.0)),px(h.max(0.0)))};
    window.with_content_mask(Some(ContentMask{bounds}),|window| {
        fill(window, x0, y0 - 1.0, w, 1.0, palette.border);
        fill(window, x0, y0, w, h, palette.header_bg);
        text(window, cx, font, x0 + 12.0, y0, &f.head, palette.dim);
        let body=Bounds{origin:point(px(x0),px(y0+line_h())),size:size(px(w.max(0.0)),px((h-2.0*line_h()).max(0.0)))};
        window.with_content_mask(Some(ContentMask{bounds:body}),|window| {
            for (i, (row, chosen)) in f.rows.iter().skip(f.scroll).take(((h-2.0*line_h())/line_h()).max(0.0).ceil() as usize).enumerate() {
                let y = y0 + (i as f32 + 1.0) * line_h();
                if *chosen {fill(window, x0, y, w, line_h(), palette.selected_bg);}
                text(window, cx, font, x0 + 12.0, y, row, palette.fg);
            }
        });
        if let Some(ask) = &f.ask {
            let y = y0 + h - line_h();
            fill(window, x0, y, w, line_h(), palette.banner_bg);
            text(window, cx, font, x0 + 12.0, y, ask, palette.fg);
        }
    });
}

#[cfg(all(test, feature = "gui-test"))]
mod gui_tests {
    use super::super::merge_view::gui_tests::{event, fixture};
    use super::*;
    #[test]
    fn gui_picker_real_keys_render_selection_undo_save_bytes_and_confirm_cancel() {
        let mut cx = gpui::TestAppContext::single();
        cx.skip_drawing();
        let (dir, viewer) = fixture(&mut cx);
        let target = dir.path().join("binary");
        std::fs::write(&target, b"\0ours\xff").unwrap();
        let opened = Opened {
            path: "binary".into(),
            target: target.clone(),
            snapshot: crate::version_session::TargetSnapshot::capture(&target),
            versions: crate::version_session::versions(vec![
                ("Ours".into(), Some(b"\0ours\xff".to_vec())),
                ("Theirs".into(), Some(b"\0theirs\xfe".to_vec())),
            ]),
            why: "binary".into(),
            large_text: false,
            files: None,
            info: Vec::new(),
        };
        viewer.update(&mut cx, |v, cx| {
            v.merge = None;
            v.start_picker(opened, false, cx);
            v.picker_v_key(&event("2", false, false, false), cx);
        });
        assert!(viewer.read_with(&cx, |v, _| v.picker_v_frame().unwrap().rows[1].1));
        viewer.update(&mut cx, |v, cx| {
            v.picker_v_key(&event("z", true, false, false), cx);
        });
        assert_eq!(
            viewer.read_with(&cx, |v, _| v.picker_v.as_ref().unwrap().chosen),
            None
        );
        viewer.update(&mut cx, |v, cx| {
            v.picker_v_key(&event("y", true, false, false), cx);
            v.picker_v_key(&event("s", true, false, false), cx);
        });
        assert_eq!(std::fs::read(&target).unwrap(), b"\0theirs\xfe");
        viewer.update(&mut cx, |v, cx| {
            v.picker_v_key(&event("1", false, false, false), cx);
            v.picker_v_key(&event("escape", false, false, false), cx);
        });
        assert!(viewer.read_with(&cx, |v, _| {
            v.picker_v_frame()
                .unwrap()
                .ask
                .unwrap()
                .contains("Unsaved choice")
        }));
        viewer.update(&mut cx, |v, cx| {
            v.picker_v_key(&event("escape", false, false, false), cx);
        });
        assert!(viewer.read_with(&cx, |v, _| v.picker_v.is_none()));
        assert_eq!(std::fs::read(&target).unwrap(), b"\0theirs\xfe");
    }

    #[gpui::test]
    fn gui_version_band_drag_scroll_hits_reset_and_tiny_height(cx:&mut gpui::TestAppContext) {
        cx.skip_drawing();let (dir,viewer)=fixture(cx);let target=dir.path().join("binary");std::fs::write(&target,b"old").unwrap();
        let opened=Opened{path:"binary".into(),target:target.clone(),snapshot:crate::version_session::TargetSnapshot::capture(&target),
            versions:crate::version_session::versions(vec![("Ours".into(),Some(b"ours".to_vec())),("Theirs".into(),Some(b"theirs".to_vec()))]),
            why:"binary".into(),large_text:false,files:None,info:vec!["info 1".into(),"info 2".into(),"info 3".into()]};
        viewer.update(cx,|v,cx|{v.merge=None;v.start_picker(opened,false,cx);});
        let window=cx.update(|cx|cx.open_window(Default::default(),|_,_|viewer.clone())).unwrap();
        window.update(cx,|v,w,cx| {
            let _=v.render(w,cx);let initial=v.picker_v_height();let x=v.diff_x()+40.;let top=v.panes.viewport[1]-v.panel_h()-initial;
            v.on_mouse_down(&MouseDownEvent{button:MouseButton::Left,position:point(px(x),px(top)),modifiers:Default::default(),click_count:1,first_mouse:false},w,cx);
            assert!(v.panes.drag.is_some());assert_eq!(v.picker_v.as_ref().unwrap().chosen,None);
            v.resize_move(x,top+initial-3.0*line_h(),true,cx);v.on_mouse_up(&MouseUpEvent::default(),w,cx);
            assert!((v.picker_v_height()-3.0*line_h()).abs()<0.1);
            let band_top=v.panes.viewport[1]-v.panel_h()-v.picker_v_height();
            assert!(v.picker_v_wheel(x,band_top+line_h()+2.0,0.5,cx));
            assert_eq!(v.picker_v_frame().unwrap().scroll,0);
            assert!(v.picker_v_wheel(x,band_top+line_h()+2.0,0.5,cx));
            assert_eq!(v.picker_v_frame().unwrap().scroll,1);
            v.picker_v_click(line_h()+2.0,cx);assert_eq!(v.picker_v.as_ref().unwrap().chosen,Some(1));
            let height=v.panes.version;let _=v.render(w,cx);assert_eq!(v.panes.version,height);
            v.panes.viewport[1]=50.;assert!(v.picker_v_height().is_finite());assert!(v.picker_v_height()>=0.0);
            v.reset_pane_sizes();assert_eq!(v.panes.version,None);assert_eq!(v.panes.version_scroll,0.0);
        }).unwrap();
        assert_eq!(std::fs::read(target).unwrap(),b"old");
    }

    #[test]
    fn gui_picker_stale_save_delete_and_reload_preserve_external_bytes() {
        let mut cx = gpui::TestAppContext::single();
        cx.skip_drawing();
        let (dir, viewer) = fixture(&mut cx);
        let target = dir.path().join("binary");
        std::fs::write(&target, b"\0ours\xff").unwrap();
        let opened = Opened {
            path: "binary".into(),
            target: target.clone(),
            snapshot: crate::version_session::TargetSnapshot::capture(&target),
            versions: crate::version_session::versions(vec![
                ("Ours".into(), Some(b"\0ours\xff".to_vec())),
                ("Theirs".into(), Some(b"\0theirs\xfe".to_vec())),
                ("Delete".into(), None),
            ]),
            why: "binary".into(),
            large_text: false,
            files: None,
            info: Vec::new(),
        };
        viewer.update(&mut cx, |v, cx| {
            v.merge = None;
            v.start_picker(opened, false, cx);
        });
        let window = cx
            .update(|cx| cx.open_window(Default::default(), |_, _| viewer.clone()))
            .unwrap();
        let key = |cx: &mut gpui::TestAppContext, name: &str, ctrl: bool| {
            window
                .update(cx, |v, w, cx| {
                    v.on_key(&event(name, ctrl, false, false), w, cx)
                })
                .unwrap();
        };
        key(&mut cx, "2", false);
        std::fs::write(&target, b"\0external").unwrap();
        viewer.update(&mut cx, |v, cx| v.check_merge_on_disk(cx));
        assert!(viewer.read_with(&cx, |v, _| {
            v.picker_v_frame()
                .unwrap()
                .ask
                .unwrap()
                .contains("Changed on disk")
        }));
        key(&mut cx, "s", true);
        assert!(viewer.read_with(&cx, |v, _| {
            v.picker_v_frame()
                .unwrap()
                .ask
                .unwrap()
                .contains("Save again overwrites")
        }));
        assert_eq!(std::fs::read(&target).unwrap(), b"\0external");
        key(&mut cx, "s", true);
        assert_eq!(std::fs::read(&target).unwrap(), b"\0theirs\xfe");
        key(&mut cx, "1", false);
        std::fs::write(&target, b"\0second external").unwrap();
        key(&mut cx, "enter", true);
        assert_eq!(std::fs::read(&target).unwrap(), b"\0second external");
        key(&mut cx, "escape", false);
        assert!(viewer.read_with(&cx, |v, _| {
            v.picker_v
                .as_ref()
                .is_some_and(|p| p.ask.is_none() && p.dirty)
        }));
        key(&mut cx, "s", true);
        key(&mut cx, "r", false);
        assert!(viewer.read_with(&cx, |v, _| {
            v.picker_v
                .as_ref()
                .is_some_and(|p| p.chosen.is_none() && !p.dirty)
        }));
        assert_eq!(std::fs::read(&target).unwrap(), b"\0second external");
        key(&mut cx, "3", false);
        std::fs::write(&target, b"\0keep external").unwrap();
        key(&mut cx, "s", true);
        assert_eq!(std::fs::read(&target).unwrap(), b"\0keep external");
        key(&mut cx, ":", false);
        assert!(viewer.read_with(&cx, |v, _| v.menu.is_none()));
        key(&mut cx, "escape", false);
        key(&mut cx, ":", false);
        let row = viewer.read_with(&cx, |v, _| v.command_palette.as_ref().expect("palette").rows().position(|r| r.entry.action == crate::catalog::Action::Pick(PickAct::Reload)).expect("Reload listed"));
        key(&mut cx, "home", false);
        for _ in 0..row {
            key(&mut cx, "down", false);
        }
        key(&mut cx, "enter", false);
        assert!(viewer.read_with(&cx, |v, _| {
            v.picker_v
                .as_ref()
                .is_some_and(|p| p.chosen.is_none() && !p.dirty)
        }));
        key(&mut cx, "3", false);
        std::fs::write(&target, b"\0last external").unwrap();
        key(&mut cx, "enter", true);
        assert_eq!(std::fs::read(&target).unwrap(), b"\0last external");
        assert!(viewer.read_with(&cx, |v, _| {
            v.picker_v_frame()
                .unwrap()
                .ask
                .unwrap()
                .contains("Done again overwrites")
        }));
        key(&mut cx, "enter", true);
        assert!(!target.exists());
    }
}
