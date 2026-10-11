//! GUI layout/input for choosing a repository when `lenz` starts outside one. The shared core lists recent
//! repositories, completes paths and validates the choice; Browse… adds the platform folder dialog.
use super::*;
use crate::repo_picker::{PickerCommand, RepoPicker};
use std::sync::Mutex;

/// The repository chosen before the window closed; `None` when the user quit.
static CHOSEN: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn chosen() -> Option<PathBuf> { CHOSEN.lock().ok()?.take() }

pub struct RepoChooser {
    pub(super) core: RepoPicker,
    focus: FocusHandle,
    font: Font,
    /// Recent rows as last painted: (index into matches, top y).
    rows: Vec<(usize, f32)>,
    /// Browse… as last painted: x, y, width.
    browse: (f32, f32, f32),
}

const PAD: f32 = 16.0;

impl RepoChooser {
    pub fn new(core: RepoPicker, font: Font, cx: &mut Context<Self>) -> Self {
        Self { core, focus: cx.focus_handle(), font, rows: Vec::new(), browse: (0.0, 0.0, 0.0) }
    }

    pub fn focus_handle(&self) -> FocusHandle { self.focus.clone() }

    pub(super) fn command(&mut self, command: PickerCommand, cx: &mut Context<Self>) {
        if let Some(root) = self.core.execute(command) {
            if let Ok(mut chosen) = CHOSEN.lock() { *chosen = Some(root); }
            cx.quit();
        }
        cx.notify();
    }

    fn browse(&mut self, cx: &mut Context<Self>) {
        let answer = cx.prompt_for_paths(gpui::PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Open".into()) });
        cx.spawn(async move |this, cx| {
            let picked = match answer.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next().map(Ok),
                Ok(Err(e)) => Some(Err(format!("The folder dialog could not open ({e}); type a path instead"))),
                _ => None,
            };
            this.update(cx, |c, cx| match picked {
                Some(Ok(path)) => c.command(PickerCommand::Choose(path), cx),
                Some(Err(message)) => { c.core.error = Some(message); cx.notify(); }
                None => {}
            }).ok();
        }).detach();
    }

    pub(super) fn on_key(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let command = (ks.modifiers.control || ks.modifiers.platform).then_some(ks.key.as_str());
        match (command, ks.key.as_str()) {
            (Some("q" | "w"), _) | (None, "escape") => cx.quit(),
            (Some("o"), _) => self.browse(cx),
            (Some(_), _) => {}
            (None, "enter") => self.command(PickerCommand::Submit, cx),
            (None, "tab") => self.command(PickerCommand::Complete, cx),
            (None, "backspace") => self.command(PickerCommand::Backspace, cx),
            (None, "up") => self.command(PickerCommand::Up, cx),
            (None, "down") => self.command(PickerCommand::Down, cx),
            _ if !ks.modifiers.alt => {
                if let Some(t) = ks.key_char.clone().or_else(|| (ks.key.chars().count() == 1).then(|| ks.key.clone())) {
                    for c in t.chars() { self.core.execute(PickerCommand::Insert(c)); }
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    fn on_click(&mut self, ev: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        let (bx, by, bw) = self.browse;
        if x >= bx && x < bx + bw && y >= by && y < by + line_h() { return self.browse(cx); }
        let row = self.rows.iter().find(|(_, top)| y >= *top && y < top + line_h()).map(|(i, _)| *i);
        if let Some(path) = row.and_then(|i| self.core.matches().get(i).map(|p| p.to_path_buf())) {
            self.command(PickerCommand::Choose(path), cx);
        }
    }

    /// Lines to paint, top to bottom: (text, color, background); recent rows carry their index.
    fn lines(&self, palette: &theme::Palette) -> Vec<(String, u32, Option<u32>, Option<usize>)> {
        let mut lines = vec![
            ("Open a repository".into(), palette.accent, None, None),
            (format!("Not inside a Git repository: {}", self.core.cwd().display()), palette.dim, None, None),
            (String::new(), palette.fg, None, None),
            (format!("Path: {}▏", self.core.input), palette.fg, Some(palette.status_bg), None),
            (self.core.error.clone().unwrap_or_default(), palette.error, None, None),
        ];
        let matches = self.core.matches();
        lines.push((if matches.is_empty() { "No recent repositories" } else { "Recent repositories" }.into(), palette.accent, None, None));
        let selected = self.core.selected.min(matches.len().saturating_sub(1));
        lines.extend(matches.iter().enumerate().map(|(i, p)| (p.display().to_string(), palette.fg, (i == selected).then_some(palette.selected_bg), Some(i))));
        lines
    }
}

impl Render for RepoChooser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.global::<themes::GuiThemes>().0.snapshot().palette;
        let viewport = window.viewport_size();
        let (w, h) = (f32::from(viewport.width).max(0.0), f32::from(viewport.height).max(0.0));
        let all = self.lines(&palette);
        // the selected recent row stays on screen above the bottom bar
        let fixed = all.iter().take_while(|l| l.3.is_none()).count();
        let room = (((h - 2.0 * PAD) / line_h()).floor() as usize).saturating_sub(fixed + 2);
        let selected = all.iter().position(|l| l.2.is_some() && l.3.is_some()).map_or(0, |i| i - fixed);
        let skip = (selected + 1).saturating_sub(room.max(1));
        let lines: Vec<_> = all[..fixed].iter().cloned().chain(all[fixed..].iter().skip(skip).take(room).cloned()).collect();
        self.rows = lines.iter().enumerate().filter_map(|(row, l)| l.3.map(|i| (i, PAD + row as f32 * line_h()))).collect();
        let bar_y = (h - PAD - line_h()).max(0.0);
        let label = "Browse… (Ctrl+O)";
        self.browse = (PAD, bar_y, label.chars().count() as f32 * font_size() * 0.62 + 16.0);
        let font = self.font.clone();
        let browse = self.browse;
        div()
            .size_full()
            .bg(rgb(palette.bg))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_click))
            .child(canvas(|_, _, _| {}, move |_, _, window, cx| {
                let content = Bounds { origin: point(px(0.0), px(0.0)), size: size(px(w), px(bar_y)) };
                window.with_content_mask(Some(ContentMask { bounds: content }), |window| {
                    for (row, (s, color, bg, _)) in lines.iter().enumerate() {
                        let y = PAD + row as f32 * line_h();
                        if let Some(bg) = bg { fill(window, PAD - 4.0, y, (w - 2.0 * PAD + 8.0).max(0.0), line_h(), *bg); }
                        text(window, cx, &font, PAD, y, s, *color);
                    }
                });
                fill(window, browse.0, browse.1, browse.2, line_h(), palette.selected_bg);
                text(window, cx, &font, browse.0 + 8.0, browse.1, label, palette.accent);
                text(window, cx, &font, browse.0 + browse.2 + 16.0, browse.1, "Type a path · Tab complete · ↑/↓ recent · Enter open · Esc quit", palette.dim);
            }).size_full())
    }
}

#[cfg(all(test, feature = "gui-test"))]
mod gui_tests {
    use super::*;
    use crate::ui::merge_view::gui_tests::event;
    use gpui::TestAppContext;

    fn git(dir: &Path) {
        assert!(std::process::Command::new("git").args(["init", "-q"]).current_dir(dir).status().unwrap().success());
    }

    #[gpui::test]
    fn gui_repo_chooser_types_completes_explains_and_opens(cx: &mut TestAppContext) {
        let base = tempfile::tempdir().unwrap();
        std::fs::create_dir(base.path().join("project")).unwrap();
        git(&base.path().join("project"));
        std::fs::create_dir(base.path().join("plain")).unwrap();
        let recent = tempfile::tempdir().unwrap();
        git(recent.path());
        cx.update(themes::install);
        let core = RepoPicker::with_recent(base.path().into(), vec![recent.path().into()]);
        let chooser = cx.new(|cx| RepoChooser::new(core, gpui::font("monospace"), cx));
        let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| chooser.clone())).unwrap();
        let key = |cx: &mut TestAppContext, name: &str, ctrl: bool| {
            window.update(cx, |c, w, cx| c.on_key(&event(name, ctrl, false, false), w, cx)).unwrap();
            cx.executor().run_until_parked();
        };
        for name in ["p", "l", "tab"] { key(cx, name, false); }
        assert_eq!(chooser.read_with(cx, |c, _| c.core.input.clone()), "plain/");
        key(cx, "enter", false);
        assert!(chooser.read_with(cx, |c, _| c.core.error.clone()).unwrap().starts_with("Not a Git repository"));
        assert_eq!(chosen(), None, "a refused folder keeps the chooser open");
        for _ in 0.."plain/".len() { key(cx, "backspace", false); }
        key(cx, "x", true);
        assert_eq!(chooser.read_with(cx, |c, _| c.core.input.clone()), "", "modified keys do not type");
        let lines = chooser.read_with(cx, |c, cx| c.lines(&cx.global::<themes::GuiThemes>().0.snapshot().palette));
        assert!(lines.iter().any(|l| l.3 == Some(0) && l.0 == recent.path().display().to_string()), "recent row listed");
        for name in ["p", "r", "tab", "enter"] { key(cx, name, false); }
        assert_eq!(chosen().unwrap(), crate::changeset::toplevel(&base.path().join("project")).unwrap());
    }
}
