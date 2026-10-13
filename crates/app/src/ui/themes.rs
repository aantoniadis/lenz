//! GUI controls; selection and I/O belong to the shared theme service.
use super::*;
use crate::theme::{
    ThemeCommand, ThemeEntry, ThemeSelection, ThemeService,
};
pub(crate) struct GuiThemes(
    pub ThemeService,
    pub Option<String>,
    #[cfg(test)]
    #[allow(dead_code)]
    pub Option<tempfile::TempDir>,
);
impl gpui::Global for GuiThemes {}
pub(crate) fn install(cx: &mut App) {
    if cx.has_global::<GuiThemes>() {
        return;
    }
    #[cfg(test)]
    let scratch = tempfile::tempdir().unwrap();
    #[cfg(test)]
    let paths = crate::theme::ThemePaths::under(&scratch.path().join("config"), &scratch.path().join("state"), "lenz");
    #[cfg(not(test))]
    let paths = crate::theme::paths_from_env();
    #[cfg(test)]
    let overrides = crate::theme::ThemeOverrides::default();
    #[cfg(not(test))]
    let overrides = crate::theme::overrides_from_env();
    let mut service = ThemeService::new(paths, overrides, true);
    let rx = service.updates();
    let notice = service.take_notices().into_iter().next();
    cx.set_global(GuiThemes(
        service,
        notice,
        #[cfg(test)]
        Some(scratch),
    ));
    #[cfg(test)]
    drop(rx);
    #[cfg(not(test))]
    cx.spawn(async move |cx| {
        while let Ok(update) = rx.recv().await {
            cx.update_global::<GuiThemes, _>(|g, _| {
                let effect = g.0.apply(update);
                g.1 = effect.notice;
            });
        }
    })
    .detach();
}
/// The chooser's box: x, y, width, height.
fn chooser_box(viewport: gpui::Size<Pixels>) -> (f32, f32, f32, f32) {
    let w = (f32::from(viewport.width) - 16.0).clamp(0.0, 600.0);
    let h = (f32::from(viewport.height) - 16.0).clamp(0.0, 400.0);
    ((f32::from(viewport.width) - w) / 2.0, (f32::from(viewport.height) - h) / 2.0, w, h)
}
pub(super) struct ThemeChooser {
    pub entries: Vec<ThemeEntry>,
    pub selected: usize,
    pub scroll: usize,
    pub hits: Vec<(usize, Bounds<Pixels>)>,
    source: String,
}
impl ThemeChooser {
    fn refresh(&mut self, service: &ThemeService) {
        let id = self.entries.get(self.selected).map(|e| e.id.clone());
        self.entries = vec![ThemeEntry {
            id: "system".into(),
            label: "Follow system".into(),
            palette: Arc::new(theme::DARCULA),
        }];
        self.entries.extend_from_slice(service.entries());
        self.selected = id
            .and_then(|id| self.entries.iter().position(|e| e.id == id))
            .unwrap_or(0);
        self.source = service.snapshot().source;
    }
}
impl Viewer {
    pub(super) fn open_themes(&mut self, cx: &mut Context<Self>) {
        if self.about.is_some()
            || self.merge.as_ref().is_some_and(|m| m.ask.is_some())
            || self.picker_v.as_ref().is_some_and(|p| p.ask.is_some())
            || self.conflicts.as_ref().is_some_and(|c| c.confirming())
        {
            return;
        }
        cx.update_global::<GuiThemes, _>(|g, _| {
            g.0.execute(ThemeCommand::OpenChooser);
        });
        let service = &cx.global::<GuiThemes>().0;
        let mut chooser = ThemeChooser {
            entries: Vec::new(),
            selected: 0,
            scroll: 0,
            hits: Vec::new(),
            source: String::new(),
        };
        chooser.refresh(service);
        chooser.selected = chooser
            .entries
            .iter()
            .position(|e| e.id == service.snapshot().selection.id())
            .unwrap_or(0);
        self.theme_chooser = Some(chooser);
        self.theme_preview = None;
        cx.notify();
    }
    fn select_theme(&mut self, index: usize, cx: &mut Context<Self>) {
        let selection = self
            .theme_chooser
            .as_ref()
            .and_then(|c| c.entries.get(index))
            .and_then(|e| ThemeSelection::from_id(&e.id));
        if let Some(selection) = selection {
            let effect = cx
                .update_global::<GuiThemes, _>(|g, _| g.0.execute(ThemeCommand::Select(selection)));
            if let Some(notice) = effect.notice {
                self.notice(notice, cx);
            }
        }
        self.theme_chooser = None;
        self.theme_preview = None;
        cx.notify();
    }
    pub(super) fn theme_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Some(c) = &mut self.theme_chooser else {
            return false;
        };
        c.selected = c.selected.min(c.entries.len().saturating_sub(1));
        match ev.keystroke.key.as_str() {
            "escape" => self.theme_chooser = None,
            "enter" => {
                let i = c.selected;
                self.select_theme(i, cx);
            }
            "up" => c.selected = c.selected.saturating_sub(1),
            "down" => c.selected = (c.selected + 1).min(c.entries.len().saturating_sub(1)),
            "home" => c.selected = 0,
            "end" => c.selected = c.entries.len().saturating_sub(1),
            "pageup" => c.selected = c.selected.saturating_sub(8),
            "pagedown" => c.selected = (c.selected + 8).min(c.entries.len().saturating_sub(1)),
            _ => {}
        }
        self.preview_theme(cx);
        cx.notify();
        true
    }
    /// Show the highlighted theme now, without selecting or saving it.
    fn preview_theme(&mut self, cx: &mut Context<Self>) {
        let id = self.theme_chooser.as_ref().and_then(|c| c.entries.get(c.selected)).map(|e| e.id.clone());
        self.theme_preview = id.and_then(|id| cx.global::<GuiThemes>().0.preview(&id));
    }
    pub(super) fn theme_click(
        &mut self,
        ev: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(c) = &self.theme_chooser else {
            return false;
        };
        let at = c
            .hits
            .iter()
            .find(|(_, r)| r.contains(&ev.position))
            .map(|(i, _)| *i);
        let (x, y, w, h) = chooser_box(_window.viewport_size());
        let (px_, py_) = (f32::from(ev.position.x), f32::from(ev.position.y));
        if let Some(i) = at {
            self.select_theme(i, cx);
        } else if !(px_ >= x && px_ < x + w && py_ >= y && py_ < y + h) {
            // a click outside closes it, the theme from before kept
            self.theme_chooser = None;
            self.theme_preview = None;
            cx.notify();
        }
        true
    }
    pub(super) fn theme_overlay(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let chooser = self.theme_chooser.as_mut()?;
        chooser.refresh(&cx.global::<GuiThemes>().0);
        let (x, y, w, h) = chooser_box(window.viewport_size());
        let visible = ((h - 8.0).max(0.0) / line_h()) as usize;
        let columns = ((w - 24.0).max(0.0) / self.char_w.max(8.0))
            .floor()
            .max(1.0) as usize;
        let mut content = Vec::new();
        for (label, index) in std::iter::once((
            "Theme · ↑/↓ choose · Enter select · Esc close".to_owned(),
            None,
        ))
        .chain(std::iter::once((
            format!("Current: {}", chooser.source),
            None,
        )))
        .chain(
            chooser
                .entries
                .iter()
                .enumerate()
                .map(|(i, e)| (e.label.clone(), Some(i))),
        ) {
            let chars = label.chars().collect::<Vec<_>>();
            content.extend(
                chars
                    .chunks(columns)
                    .map(|c| (c.iter().collect::<String>(), index)),
            );
        }
        chooser.scroll = chooser.scroll.min(content.len().saturating_sub(visible));
        if let Some(at) = content
            .iter()
            .position(|(_, i)| *i == Some(chooser.selected))
        {
            if at < chooser.scroll {
                chooser.scroll = at;
            } else if visible > 0 && at >= chooser.scroll + visible {
                chooser.scroll = at + 1 - visible;
            }
        }
        chooser.hits.clear();
        let rows = content
            .into_iter()
            .enumerate()
            .skip(chooser.scroll)
            .take(visible)
            .map(|(row, (label, index))| {
                let r = Bounds {
                    origin: point(
                        px(x + 8.0),
                        px(y + 4.0 + (row - chooser.scroll) as f32 * line_h()),
                    ),
                    size: size(px((w - 16.0).max(0.0)), px(line_h())),
                };
                if let Some(i) = index {
                    chooser.hits.push((i, r));
                }
                (label, r, index == Some(chooser.selected), index.is_none())
            })
            .collect::<Vec<_>>();
        let palette = self.palette.clone();
        let font = self.font.clone();
        Some(
            div()
                .absolute()
                .size_full()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, ev, window, cx| {
                        this.theme_click(ev, window, cx);
                        cx.stop_propagation();
                    }),
                )
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                .child(
                    canvas(
                        |_, _, _| {},
                        move |_, _, window, cx| {
                            fill(window, x, y, w, h, palette.status_bg);
                            let mask = ContentMask {
                                bounds: Bounds {
                                    origin: point(px(x), px(y)),
                                    size: size(px(w), px(h)),
                                },
                            };
                            window.with_content_mask(Some(mask), |window| {
                                for (label, r, selected, heading) in &rows {
                                    let yy = f32::from(r.origin.y);
                                    if *selected {
                                        fill(
                                            window,
                                            x + 8.0,
                                            yy,
                                            (w - 16.0).max(0.0),
                                            line_h(),
                                            palette.selected_bg,
                                        );
                                    }
                                    text(
                                        window,
                                        cx,
                                        &font,
                                        x + 12.0,
                                        yy,
                                        label,
                                        if *heading { palette.accent } else { palette.fg },
                                    );
                                }
                            });
                        },
                    )
                    .size_full(),
                )
                .into_any_element(),
        )
    }
}
