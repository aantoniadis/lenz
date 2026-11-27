//! Project tabs in a window: several [`ReaderView`]s, each its own session and project, under a project tab strip,
//! shown one at a time or Side by Side as columns. Each shown view is given a frame and lays itself out in it as it
//! would in a window that size; pointer input reaches it in its own coordinates. With one project the window is that
//! view alone, exactly as before: no strip, no frame.
use super::*;
use code_core::projects::{self, Action, Layout, Projects};
use gpui::{Entity, prelude::FluentBuilder};

/// What a view asks of the window's project host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectRequest {
    Action(Action),
    /// Open Folder in New Project Tab: the folder chosen.
    OpenFolder(PathBuf),
}

impl gpui::EventEmitter<ProjectRequest> for ReaderView {}

/// A divider between Side by Side projects, in pixels.
const DIVIDER: f32 = 1.0;

/// A window's projects.
pub struct ProjectsWindow {
    views: Vec<Entity<ReaderView>>,
    projects: Projects,
    make_config: std::rc::Rc<dyn Fn(bool) -> Config>,
    text_input: bool,
    /// The shown projects' frames as last laid out (window coordinates).
    pub columns: Vec<(usize, Bounds<Pixels>)>,
}

impl ProjectsWindow {
    /// A window holding one project, opened on `path` (a file or folder) when given.
    pub fn new(make_config: std::rc::Rc<dyn Fn(bool) -> Config>, text_input: bool, path: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut host = Self { views: Vec::new(), projects: Projects::new(0), make_config, text_input, columns: Vec::new() };
        host.open(path, window, cx);
        host
    }

    pub fn views(&self) -> &[Entity<ReaderView>] { &self.views }
    pub fn projects(&self) -> Projects { self.projects }
    pub fn active_view(&self) -> Option<&Entity<ReaderView>> { self.views.get(self.projects.active()) }

    /// Open a new project tab at the end (on `path` when given) and give it the keys.
    pub fn open(&mut self, path: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let config = (self.make_config)(dark(window));
        let make_config = self.make_config.clone();
        let text_input = self.text_input;
        let view = cx.new(|cx| ReaderView::new(Session::with_text_input(config, text_input), path, cx).with_config(make_config));
        view.update(cx, |_, cx| {
            cx.observe_window_appearance(window, |v, window, cx| {
                v.dispatch(Command::Theme(ThemeCommand::NativeAppearance(dark(window))), cx);
            })
            .detach();
            // Coming back to the window: recheck open sources against the disk.
            cx.observe_window_activation(window, |v, window, cx| {
                if window.is_window_active() {
                    v.dispatch(Command::Recheck, cx);
                }
            })
            .detach();
        });
        close::register(&view, window, cx);
        cx.subscribe_in(&view, window, Self::request).detach();
        self.views.push(view);
        let index = self.projects.opened();
        self.focus(index, window, cx);
        cx.notify();
    }

    /// Give project `index` the keys.
    pub fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index == self.projects.active() || !self.projects.activate(index) { return; }
        self.focus(index, window, cx);
        if let Some(view) = self.views.get(index) { view.update(cx, |v, cx| v.dispatch(Command::Recheck, cx)); }
        cx.notify();
    }

    fn focus(&self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.views.get(index) {
            let focus = view.read(cx).focus.clone();
            window.focus(&focus, cx);
        }
    }

    /// The project whose session is `id` was closed (its unsaved work already decided). Whether none is left.
    pub(crate) fn remove(&mut self, id: code_core::close::SessionId, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if let Some(index) = self.views.iter().position(|v| v.read(cx).session.id() == id) {
            self.views.remove(index);
            self.projects.remove(index);
            let active = self.projects.active();
            self.focus(active, window, cx);
            cx.notify();
        }
        self.views.is_empty()
    }

    /// The project whose session is `id` has a close question: show it.
    pub(crate) fn reveal(&mut self, id: code_core::close::SessionId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.views.iter().position(|v| v.read(cx).session.id() == id) { self.activate(index, window, cx); }
    }

    /// The window's own close button: every project closes, each asking about its unsaved work.
    pub(crate) fn window_should_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.views.iter().all(|v| v.read(cx).session.closed()) { return true; }
        if self.views.iter().all(|v| v.read(cx).modal.is_none()) { close::request_window(window.window_handle(), cx); }
        false
    }

    fn request(&mut self, view: &Entity<ReaderView>, request: &ProjectRequest, window: &mut Window, cx: &mut Context<Self>) {
        // A request comes from the view with the keys; make sure the set agrees before stepping from it.
        if let Some(index) = self.views.iter().position(|v| v == view) { self.projects.activate(index); }
        match request {
            ProjectRequest::OpenFolder(path) => self.open(Some(path.clone()), window, cx),
            ProjectRequest::Action(action) => match action {
                Action::New => self.open(None, window, cx),
                Action::Next | Action::Previous => {
                    let mut set = self.projects;
                    let to = set.step(*action == Action::Next);
                    self.activate(to, window, cx);
                }
                Action::ToggleSideBySide => { self.projects.toggle_layout(); cx.notify(); }
                // The view asks for its folder itself and closes through the close protocol.
                Action::OpenFolder | Action::Close => {}
            },
        }
    }

    /// A project tab as IntelliJ shows a project window's tab: an equal share of the strip, `project – where` centred
    /// (where: the shown file, else the folder), `●` for unsaved work, the active one raised, and its close mark.
    fn strip_tab(&self, index: usize, active: bool, palette: &lenz_content::theme::Palette, font: &Font, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let (title, place, dirty) = self.views.get(index).map(|v| { let s = &v.read(cx).session; (s.project_title(), s.project_place(), s.project_dirty()) }).unwrap_or_default();
        let label = match (place.is_empty(), dirty) {
            (true, d) => format!("{title}{}", if d { " ●" } else { "" }),
            (false, d) => format!("{title}{} – {place}", if d { " ●" } else { "" }),
        };
        div()
            .id(("project-tab", index))
            .debug_selector(move || format!("project-tab-{index}"))
            .flex().flex_row().items_center().flex_1().min_w(px(80.0)).h(px(toolbar_h() - 6.0)).mx(px(2.0)).px(px(10.0)).gap(px(6.0))
            .rounded(px(6.0))
            .font(font.clone()).text_size(px(font_size() * 0.92))
            .bg(rgb(if active { palette.bg } else { palette.header_bg }))
            .when(active, |d| d.border_1().border_color(rgb(palette.border)))
            .text_color(rgb(if active { palette.fg } else { palette.dim }))
            .child(div().flex_1().min_w(px(0.0)).overflow_hidden().whitespace_nowrap().text_ellipsis().text_center().child(label))
            .child(div().id(("project-close", index)).debug_selector(move || format!("project-close-{index}")).px(px(3.0)).text_color(rgb(palette.dim)).child("×")
                .on_mouse_down(MouseButton::Left, cx.listener(move |host, _: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    host.close(index, window, cx);
                })))
            .on_mouse_down(MouseButton::Left, cx.listener(move |host, _: &MouseDownEvent, window, cx| host.activate(index, window, cx)))
    }

    /// Close project `index` (its tab's close mark), asking about its unsaved work first.
    pub fn close(&mut self, index: usize, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.views.get(index) {
            let id = view.read(cx).session.id();
            close::request(id, false, cx);
        }
    }
}

impl Render for ProjectsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let set = self.projects;
        for (i, view) in self.views.iter().enumerate() {
            view.update(cx, |v, _| v.session.set_projects(set.info(i)));
        }
        let Some(active_view) = self.views.get(set.active()).cloned() else { return div().size_full().into_any_element() };
        if !set.strip() {
            active_view.update(cx, |v, _| v.frame = None);
            self.columns.clear();
            return div().size_full().child(active_view).into_any_element();
        }
        let (palette, font, char_w) = { let v = active_view.read(cx); (v.session.state().palette().clone(), v.font.clone(), v.char_w) };
        let size = window.viewport_size();
        let (width, height) = (f32::from(size.width), f32::from(size.height));
        let strip_h = toolbar_h().min(height);
        let body_h = (height - strip_h).max(0.0);
        let min = projects::MIN_COLUMN_CELLS as f32 * char_w.max(1.0);
        let capacity = projects::capacity(width.max(0.0) as u32, min as u32, DIVIDER as u32);
        let shown = set.shown(capacity);
        let columns = projects::columns(width.max(0.0) as u32, shown.len(), DIVIDER as u32);
        self.columns.clear();
        let mut body = Vec::new();
        for (index, column) in shown.zip(columns) {
            let frame = Bounds { origin: gpui::point(px(column.start as f32), px(strip_h)), size: gpui::size(px((column.end - column.start) as f32), px(body_h)) };
            let view = self.views[index].clone();
            view.update(cx, |v, _| v.frame = Some(frame));
            self.columns.push((index, frame));
            body.push(
                div().id(("project", index)).absolute().left(frame.origin.x).top(frame.origin.y).w(frame.size.width).h(frame.size.height)
                    .when(index != set.active() && set.layout() == Layout::SideBySide, |d| d.opacity(0.92))
                    .capture_any_mouse_down(cx.listener(move |host, _: &MouseDownEvent, window, cx| host.activate(index, window, cx)))
                    .child(view)
                    .into_any_element(),
            );
            if column.start > 0 {
                let accent = index == set.active() || index == set.active() + 1;
                body.push(div().absolute().left(px(column.start as f32 - DIVIDER)).top(px(strip_h)).w(px(DIVIDER)).h(px(body_h)).bg(rgb(if accent { palette.accent } else { palette.border })).into_any_element());
            }
        }
        let tabs: Vec<_> = (0..self.views.len()).map(|i| self.strip_tab(i, i == set.active(), &palette, &font, cx)).collect();
        let toggle = match set.layout() { Layout::Tabs => "◫ Side by Side", Layout::SideBySide => "▭ Tabs" };
        let strip = div()
            .id("project-strip")
            .absolute().left(px(0.0)).top(px(0.0)).w(px(width)).h(px(strip_h))
            .flex().flex_row().items_center().overflow_hidden()
            .bg(rgb(palette.header_bg)).border_b_1().border_color(rgb(palette.border))
            .font(font.clone()).text_size(px(font_size()))
            .px(px(4.0))
            .children(tabs)
            .child(div().id("project-new").debug_selector(|| "project-new".into()).px(px(10.0)).h_full().flex().items_center().text_color(rgb(palette.accent)).child("+")
                .on_mouse_down(MouseButton::Left, cx.listener(|host, _: &MouseDownEvent, window, cx| host.open(None, window, cx))))
            .child(div().id("project-layout").debug_selector(|| "project-layout".into()).px(px(10.0)).h_full().flex().items_center().text_color(rgb(palette.fg)).bg(rgb(palette.status_bg)).child(toggle)
                .on_mouse_down(MouseButton::Left, cx.listener(|host, _: &MouseDownEvent, _, cx| { host.projects.toggle_layout(); cx.notify(); })));
        div().id("projects").size_full().relative().bg(rgb(palette.bg)).child(strip).children(body).into_any_element()
    }
}

/// Open a window with one project tab, on `path` when given.
pub fn open_projects_window(cx: &mut App, make_config: std::rc::Rc<dyn Fn(bool) -> Config>, path: Option<PathBuf>) -> Result<gpui::WindowHandle<ProjectsWindow>, String> {
    let options = WindowOptions {
        app_id: Some(code_core::chrome::APP_ID.into()),
        focus: true,
        titlebar: Some(TitlebarOptions { title: Some(code_core::chrome::APP_NAME.into()), ..Default::default() }),
        ..Default::default()
    };
    cx.open_window(options, |window, cx| {
        let text_input = text_input_supported(cx.compositor_name());
        let host = cx.new(|cx| ProjectsWindow::new(make_config, text_input, path, window, cx));
        let closing = host.downgrade();
        window.on_window_should_close(cx, move |window, cx| closing.update(cx, |h, cx| h.window_should_close(window, cx)).unwrap_or(true));
        host
    })
    .map_err(|e| e.to_string())
}

impl ReaderView {
    /// Show the core's question of where to open a chosen project while it is pending.
    pub(super) fn sync_open_project_prompt(&mut self) {
        match (self.session.open_project_prompt().map(|p| p.text()), &mut self.modal) {
            (Some(now), Some(Modal::StatusDetails { kind: Details::OpenProject, text, .. })) => *text = now,
            (Some(text), None) => {
                self.modal_scroll.set(0);
                self.modal = Some(Modal::StatusDetails { title: "Open Project", text, logo: None, kind: Details::OpenProject });
            }
            (None, Some(Modal::StatusDetails { kind: Details::OpenProject, .. })) => self.modal = None,
            _ => {}
        }
    }

    pub(super) fn choose_open_project(&mut self, choice: code_core::projects::OpenChoice, cx: &mut Context<Self>) {
        if let Some(ticket) = self.session.open_project_prompt().map(|p| p.ticket) {
            self.dispatch(Command::ChooseOpenProject { ticket, choice }, cx);
        }
    }
}
