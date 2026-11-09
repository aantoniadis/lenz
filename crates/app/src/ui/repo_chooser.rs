//! Native presentation of the shared repository / file / folder opening workflow.
use super::*;
use crate::line_edit::Edit;
use crate::repo_picker::{Choice, Mode, PickerCommand, RepoPicker};
use std::sync::Mutex;

static CHOSEN: Mutex<Option<Choice>> = Mutex::new(None);
#[cfg(all(test, feature = "gui-test"))]
static TEST_CHOOSER: Mutex<()> = Mutex::new(());
pub fn chosen() -> Option<Choice> {
    CHOSEN.lock().ok()?.take()
}
pub struct RepoChooser {
    pub(super) core: RepoPicker,
    focus: FocusHandle,
    font: Font,
    rows: Vec<(usize, f32)>,
    browse: (f32, f32, f32),
    submit: (f32, f32, f32),
    cancel: (f32, f32, f32),
    request: usize,
}
const PAD: f32 = 16.0;
impl RepoChooser {
    pub fn new(core: RepoPicker, font: Font, cx: &mut Context<Self>) -> Self {
        Self {
            core,
            focus: cx.focus_handle(),
            font,
            rows: Vec::new(),
            browse: (0., 0., 0.),
            submit: (0., 0., 0.),
            cancel: (0., 0., 0.),
            request: 0,
        }
    }
    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }
    pub(super) fn command(&mut self, command: PickerCommand, cx: &mut Context<Self>) {
        if let Some(choice) = self.core.execute(command) {
            if let Ok(mut chosen) = CHOSEN.lock() {
                *chosen = Some(choice);
            }
            cx.quit();
        }
        cx.notify();
    }
    fn browse(&mut self, cx: &mut Context<Self>) {
        self.request = self.request.wrapping_add(1);
        let (request, mode, field) = (self.request, self.core.mode, self.core.field);
        let answer = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: mode == Mode::Files,
            directories: mode != Mode::Files,
            multiple: false,
            prompt: Some("Open".into()),
        });
        cx.spawn(async move |this, cx| {
            let picked = match answer.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next().map(Ok),
                Ok(Err(_)) => Some(Err(
                    "The file dialog could not open. Type or paste a path instead.".to_string(),
                )),
                _ => None,
            };
            this.update(cx, |c, cx| {
                if c.request != request || c.core.mode != mode || c.core.field != field {
                    return;
                }
                match picked {
                    Some(Ok(path)) => c.command(PickerCommand::Choose(path), cx),
                    Some(Err(message)) => {
                        c.core.error = Some(message);
                        cx.notify();
                    }
                    None => {}
                }
            })
            .ok();
        })
        .detach();
    }
    pub(super) fn on_key(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let ctrl = k.modifiers.control || k.modifiers.platform;
        let word = ctrl || k.modifiers.alt;
        if matches!(k.key.as_str(), "f1" | "f2" | "f3") && self.core.comparisons {
            let mode = match k.key.as_str() {
                "f1" => Mode::Files,
                "f2" => Mode::Folders,
                _ => Mode::Repository,
            };
            return self.command(PickerCommand::Mode(mode), cx);
        }
        if k.key == "escape" || ctrl && matches!(k.key.as_str(), "q" | "w") {
            cx.quit();
            return;
        }
        if let Some(command)=lenz_gpui::field::command(ev,cx) {
            if let Some(text)=self.core.edit_field(command).copy {cx.write_to_clipboard(ClipboardItem::new_string(text));}
            cx.notify();return;
        }
        if ctrl {
            match k.key.as_str() {
                "o" => return self.browse(cx),
                "a" => return self.command(PickerCommand::SelectAll, cx),
                "v" => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                        self.command(PickerCommand::InsertText(text), cx);
                    }
                    return;
                }
                "c" | "x" => {
                    if let Some(text) = self.core.active().selection() {
                        cx.write_to_clipboard(ClipboardItem::new_string(text.into()));
                        if k.key == "x" {
                            self.command(PickerCommand::Edit(Edit::Delete), cx);
                        }
                    }
                    return;
                }
                "space" => return self.command(PickerCommand::Complete, cx),
                _ => {}
            }
        }
        let edit = match k.key.as_str() {
            "left" | "right"
                if k.modifiers.platform && !k.modifiers.control && !k.modifiers.alt =>
            {
                Some(if k.key == "left" {
                    Edit::Start
                } else {
                    Edit::End
                })
            }
            "left" => Some(if word { Edit::WordLeft } else { Edit::Left }),
            "right" => Some(if word { Edit::WordRight } else { Edit::Right }),
            "home" => Some(Edit::Start),
            "end" => Some(Edit::End),
            "backspace" => Some(if word {
                Edit::BackspaceWord
            } else {
                Edit::Backspace
            }),
            "delete" => Some(if word { Edit::DeleteWord } else { Edit::Delete }),
            _ => None,
        };
        if let Some(edit) = edit {
            self.command(PickerCommand::Edit(edit), cx);
            return;
        }
        let command = match k.key.as_str() {
            "enter" => Some(PickerCommand::Submit),
            "tab" => Some(if self.core.mode == Mode::Repository {
                PickerCommand::Complete
            } else {
                PickerCommand::NextField
            }),
            "up" => Some(PickerCommand::Up),
            "down" => Some(PickerCommand::Down),
            _ if !ctrl && !k.modifiers.alt => k
                .key_char
                .clone()
                .or_else(|| (k.key.chars().count() == 1).then(|| k.key.clone()))
                .map(PickerCommand::InsertText),
            _ => None,
        };
        if let Some(command) = command {
            self.command(command, cx);
        }
    }
    fn field_row(&self) -> usize {
        if self.core.comparisons { 6 } else { 3 }
    }
    fn on_click(&mut self, ev: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        let hit =
            |(bx, by, bw): (f32, f32, f32)| x >= bx && x < bx + bw && y >= by && y < by + line_h();
        if hit(self.browse) {
            return self.browse(cx);
        }
        if hit(self.cancel) {
            cx.quit();
            return;
        }
        if hit(self.submit) {
            return self.command(PickerCommand::Submit, cx);
        }
        if self.core.comparisons {
            for (i, mode) in Mode::ALL.into_iter().enumerate() {
                if y >= PAD + (2 + i) as f32 * line_h() && y < PAD + (3 + i) as f32 * line_h() {
                    return self.command(PickerCommand::Mode(mode), cx);
                }
            }
        }
        for i in 0..self.core.fields() {
            let top = PAD + (self.field_row() + i) as f32 * line_h();
            if y >= top && y < top + line_h() {
                return self.command(PickerCommand::Field(i), cx);
            }
        }
        let row = self
            .rows
            .iter()
            .find(|(_, top)| y >= *top && y < top + line_h())
            .map(|(i, _)| *i);
        if let Some(path) = row.and_then(|i| self.core.matches().get(i).map(|p| p.to_path_buf())) {
            self.command(PickerCommand::Choose(path), cx);
        }
    }
    fn lines(
        &self,
        p: &theme::Palette,
        width: f32,
    ) -> Vec<(String, u32, Option<u32>, Option<usize>)> {
        let mut lines = vec![
            (
                if self.core.comparisons {
                    "New comparison"
                } else {
                    "Open a repository"
                }
                .into(),
                p.accent,
                None,
                None,
            ),
            (
                format!("Paths relative to {}", self.core.cwd().display()),
                p.dim,
                None,
                None,
            ),
        ];
        if self.core.comparisons {
            for (i, mode) in Mode::ALL.into_iter().enumerate() {
                lines.push((
                    format!(
                        "{} {}  (F{})",
                        if mode == self.core.mode { "●" } else { "○" },
                        mode.label(),
                        i + 1
                    ),
                    p.accent,
                    (mode == self.core.mode).then_some(p.selected_bg),
                    None,
                ));
            }
        }
        lines.push((String::new(), p.fg, None, None));
        let columns = ((width - 2. * PAD) / (font_size() * 0.62)).max(1.) as usize;
        for i in 0..self.core.fields() {
            let field = if i == 0 {
                &self.core.input
            } else {
                &self.core.right
            };
            let shown = field.visible(columns.saturating_sub(8));
            let mut value = shown.text.to_string();
            if self.core.field == i {
                value.insert(shown.prefix.len(), '▏');
            }
            lines.push((
                format!("{}: {value}", self.core.mode.field(i)),
                p.fg,
                Some(if field.selected() {
                    p.selected_bg
                } else {
                    p.status_bg
                }),
                None,
            ));
        }
        let error = self.core.error.as_deref().unwrap_or_default();
        let starts = crate::wrap::wrap_starts(error, columns);
        let ends = starts.iter().copied().chain(std::iter::once(error.len()));
        for (start, end) in std::iter::once(0)
            .chain(starts.iter().copied())
            .zip(ends)
            .take(3)
        {
            lines.push((
                error.get(start..end).unwrap_or_default().trim_end().into(),
                p.error,
                None,
                None,
            ));
        }
        if self.core.mode == Mode::Repository {
            let matches = self.core.matches();
            lines.push((
                if matches.is_empty() {
                    "No recent repositories"
                } else {
                    "Recent repositories"
                }
                .into(),
                p.accent,
                None,
                None,
            ));
            for (i, path) in matches.iter().enumerate() {
                lines.push((
                    path.display().to_string(),
                    p.fg,
                    (i == self.core.selected).then_some(p.selected_bg),
                    Some(i),
                ));
            }
        }
        lines
    }
}
impl Render for RepoChooser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.global::<themes::GuiThemes>().0.snapshot().palette;
        let viewport = window.viewport_size();
        let (w, h) = (
            f32::from(viewport.width).max(0.),
            f32::from(viewport.height).max(0.),
        );
        let all = self.lines(&p, w);
        let fixed = all.iter().take_while(|l| l.3.is_none()).count();
        let room = (((h - 2. * PAD) / line_h()).floor() as usize).saturating_sub(fixed + 3);
        let selected = all
            .iter()
            .position(|l| l.2.is_some() && l.3.is_some())
            .map_or(0, |i| i - fixed);
        let skip = (selected + 1).saturating_sub(room.max(1));
        let lines: Vec<_> = all[..fixed]
            .iter()
            .cloned()
            .chain(all[fixed..].iter().skip(skip).take(room).cloned())
            .collect();
        self.rows = lines
            .iter()
            .enumerate()
            .filter_map(|(row, l)| l.3.map(|i| (i, PAD + row as f32 * line_h())))
            .collect();
        let by = (h - PAD - line_h()).max(0.);
        let button_w = ((w - 2. * PAD - 16.) / 3.).clamp(0., 160.);
        self.browse = (PAD, by, button_w);
        self.cancel = ((w - PAD - 2. * button_w - 8.).max(0.), by, button_w);
        self.submit = ((w - PAD - button_w).max(0.), by, button_w);
        let (browse, cancel, submit, font) =
            (self.browse, self.cancel, self.submit, self.font.clone());
        let columns=((w-2.*PAD)/(font_size()*0.62)).max(1.) as usize;
        let input_row=self.field_row();let active=self.core.field;
        let fields:Vec<_>=(0..self.core.fields()).map(|i|{
            let field=if i==0{&self.core.input}else{&self.core.right};
            (i,self.core.mode.field(i),field.cursor().snapshot(field.text(),columns.saturating_sub(8)))
        }).collect();
        let label = self.core.mode.submit();
        let hint = if self.core.mode == Mode::Repository {
            "Tab complete · ↑/↓ recent · Ctrl+A select · Enter open · Esc cancel"
        } else {
            "Tab next path · Ctrl+Space complete · Enter compare · Esc cancel"
        };
        div()
            .size_full()
            .bg(rgb(p.bg))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_click))
            .child(
                canvas(
                    |_, _, _| {},
                    move |_, _, window, cx| {
                        let bounds = Bounds {
                            origin: point(px(0.), px(0.)),
                            size: size(px(w), px((by - line_h()).max(0.))),
                        };
                        window.with_content_mask(Some(ContentMask { bounds }), |window| {
                            for (row, (s, color, bg, _)) in lines.iter().enumerate() {
                                let y = PAD + row as f32 * line_h();
                                if let Some(bg) = bg {
                                    fill(
                                        window,
                                        PAD - 4.,
                                        y,
                                        (w - 2. * PAD + 8.).max(0.),
                                        line_h(),
                                        *bg,
                                    );
                                }
                                if let Some((i,label,snapshot))=fields.iter().find(|(i,_,_)|row==input_row+*i) {
                                    text(window,cx,&font,PAD,y,&format!("{label}: "),*color);
                                    let x=PAD+7.*font_size()*0.6;
                                    lenz_gpui::field::paint(lenz_gpui::field::View{text:&snapshot.text,cursor:snapshot.cursor,
                                        bounds:Bounds{origin:point(px(x),px(y)),size:size(px((w-PAD-x).max(0.)),px(line_h()))},char_w:font_size()*0.6,active:*i==active},window,cx,
                                        lenz_gpui::Metrics{font:&font,font_size:font_size(),line_h:line_h(),num_w:0.},&p);
                                } else { text(window, cx, &font, PAD, y, s, *color); }
                            }
                        });
                        text(window, cx, &font, PAD, (by - line_h()).max(0.), hint, p.dim);
                        for ((x, y, width), label) in [
                            (
                                browse,
                                if browse.2 < 150. {
                                    "Browse…"
                                } else {
                                    "Browse… (Ctrl+O)"
                                },
                            ),
                            (cancel, "Cancel (Esc)"),
                            (submit, label),
                        ] {
                            fill(window, x, y, width, line_h(), p.selected_bg);
                            text(window, cx, &font, x + 4., y, label, p.accent);
                        }
                    },
                )
                .size_full(),
            )
    }
}
#[cfg(all(test, feature = "gui-test"))]
mod gui_tests {
    use super::*;
    use crate::ui::merge_view::gui_tests::event;
    use gpui::TestAppContext;

    fn git(dir: &Path) {
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .current_dir(dir)
                .status()
                .unwrap()
                .success()
        );
    }

    #[gpui::test]
    fn gui_repo_chooser_types_completes_explains_and_opens(cx: &mut TestAppContext) {
        let _guard = TEST_CHOOSER.lock().unwrap();
        let base = tempfile::tempdir().unwrap();
        std::fs::create_dir(base.path().join("project")).unwrap();
        git(&base.path().join("project"));
        std::fs::create_dir(base.path().join("plain")).unwrap();
        let recent = tempfile::tempdir().unwrap();
        git(recent.path());
        cx.update(themes::install);
        let core = RepoPicker::with_recent(base.path().into(), vec![recent.path().into()]);
        let chooser = cx.new(|cx| RepoChooser::new(core, gpui::font("monospace"), cx));
        let window = cx
            .update(|cx| cx.open_window(Default::default(), |_, _| chooser.clone()))
            .unwrap();
        let key = |cx: &mut TestAppContext, name: &str, ctrl: bool| {
            window
                .update(cx, |c, w, cx| {
                    c.on_key(&event(name, ctrl, false, false), w, cx)
                })
                .unwrap();
            cx.executor().run_until_parked();
        };
        for name in ["p", "l", "tab"] {
            key(cx, name, false);
        }
        assert_eq!(
            chooser.read_with(cx, |c, _| c.core.input.text().to_string()),
            "plain/"
        );
        key(cx, "enter", false);
        assert!(
            chooser
                .read_with(cx, |c, _| c.core.error.clone())
                .unwrap()
                .starts_with("Not a Git repository")
        );
        assert_eq!(chosen(), None, "a refused folder keeps the chooser open");
        for _ in 0.."plain/".len() {
            key(cx, "backspace", false);
        }
        key(cx, "x", true);
        assert_eq!(
            chooser.read_with(cx, |c, _| c.core.input.text().to_string()),
            "",
            "modified keys do not type"
        );
        let lines = chooser.read_with(cx, |c, cx| {
            c.lines(
                &cx.global::<themes::GuiThemes>().0.snapshot().palette,
                800.0,
            )
        });
        assert!(
            lines
                .iter()
                .any(|l| l.3 == Some(0) && l.0 == recent.path().display().to_string()),
            "recent row listed"
        );
        for name in ["p", "r", "tab", "enter"] {
            key(cx, name, false);
        }
        assert_eq!(
            chosen().unwrap(),
            Choice::Repository(crate::changeset::toplevel(&base.path().join("project")).unwrap())
        );
    }
    #[gpui::test]
    fn gui_comparison_chooser_replaces_paths_pastes_and_refuses_wrong_types(
        cx: &mut TestAppContext,
    ) {
        let _guard = TEST_CHOOSER.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        for name in ["left é.txt", "right.txt"] {
            std::fs::write(dir.path().join(name), name).unwrap();
        }
        cx.update(themes::install);
        let picker = RepoPicker::with_recent(dir.path().into(), vec![]).with_comparisons(true);
        let chooser = cx.new(|cx| RepoChooser::new(picker, gpui::font("monospace"), cx));
        let window = cx
            .update(|cx| cx.open_window(Default::default(), |_, _| chooser.clone()))
            .unwrap();
        let key = |cx: &mut TestAppContext, name: &str, ctrl: bool| {
            window
                .update(cx, |c, w, cx| {
                    c.on_key(&event(name, ctrl, false, false), w, cx)
                })
                .unwrap();
            cx.executor().run_until_parked();
        };
        cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("/missing/path".into())));
        key(cx, "v", true);
        key(cx, "enter", false);
        assert!(chooser.read_with(cx, |c, _| c.core.error.is_some()));
        key(cx, "a", true);
        cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("left é.txt".into())));
        key(cx, "v", true);
        key(cx, "home", false);
        key(cx, "end", false);
        key(cx, "tab", false);
        cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("right.txt".into())));
        key(cx, "v", true);
        key(cx, "f2", false);
        key(cx, "enter", false);
        assert!(chooser.read_with(cx, |c, _| c.core.error.as_ref().unwrap().contains("folder")));
        key(cx, "f1", false);
        key(cx, "enter", false);
        assert_eq!(
            chosen(),
            Some(Choice::Comparison {
                left: dir.path().join("left é.txt"),
                right: dir.path().join("right.txt")
            })
        );
        assert_eq!(
            std::fs::read(dir.path().join("right.txt")).unwrap(),
            b"right.txt"
        );
    }
}
