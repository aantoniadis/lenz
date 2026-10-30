//! Platform handles for the shared all-session close protocol.
use super::*;
use gpui::BorrowAppContext;
use code_core::close::{CloseCoordinator, Progress, SessionId};
#[derive(Clone)]
struct Entry {
    id: SessionId,
    view: gpui::WeakEntity<ReaderView>,
    window: gpui::AnyWindowHandle,
}
#[derive(Default)]
struct ReaderClose {
    coordinator: CloseCoordinator,
    entries: Vec<Entry>,
    scheduled: bool,
}
impl gpui::Global for ReaderClose {}

pub(super) fn register(view: &gpui::Entity<ReaderView>, window: &Window, cx: &mut App) {
    if !cx.has_global::<ReaderClose>() {
        cx.default_global::<ReaderClose>();
        cx.on_window_closed(|cx, window| {
            let removed = cx
                .global::<ReaderClose>()
                .entries
                .iter()
                .filter(|e| e.window.window_id() == window)
                .map(|e| e.id)
                .collect::<Vec<_>>();
            let state = cx.global_mut::<ReaderClose>();
            for id in removed {
                state.coordinator.unregister(id);
            }
            state.entries.retain(|e| e.window.window_id() != window);
            schedule(cx);
        })
        .detach();
    }
    let id = view.read(cx).session.id();
    let entry = Entry {
        id,
        view: view.downgrade(),
        window: window.window_handle(),
    };
    let state = cx.default_global::<ReaderClose>();
    state.coordinator.register(id);
    state.entries.push(entry);
    schedule(cx);
}
fn with_sessions<R>(
    entries: &[Entry],
    sessions: &mut [&mut Session],
    cx: &mut App,
    callback: &mut dyn FnMut(&mut [&mut Session]) -> R,
) -> Option<R> {
    let Some((entry, tail)) = entries.split_first() else {
        return Some(callback(sessions));
    };
    entry
        .view
        .update(cx, |view, cx| {
            let mut all: Vec<_> = sessions.iter_mut().map(|s| &mut **s).collect();
            all.push(&mut view.session);
            with_sessions(tail, &mut all, cx, callback)
        })
        .ok()
        .flatten()
}
pub(super) fn request(id: SessionId, all: bool, cx: &mut App) {
    if !cx.has_global::<ReaderClose>() {
        return;
    }
    cx.defer(move |cx| {
        let entries = cx
            .global::<ReaderClose>()
            .entries
            .iter()
            .filter(|e| all || e.id == id)
            .cloned()
            .collect::<Vec<_>>();
        let mut coordinator =
            cx.update_global::<ReaderClose, _>(|s, _| std::mem::take(&mut s.coordinator));
        with_sessions(&entries, &mut [], cx, &mut |sessions| {
            coordinator.begin(sessions)
        });
        cx.update_global::<ReaderClose, _>(|s, _| s.coordinator = coordinator);
        pump(cx);
    });
}
pub(super) fn schedule(cx: &mut App) {
    if !cx.has_global::<ReaderClose>() {
        return;
    }
    let state = cx.global_mut::<ReaderClose>();
    if !state.coordinator.pending() || state.scheduled {
        return;
    }
    state.scheduled = true;
    cx.defer(pump);
}
fn pump(cx: &mut App) {
    let entries = cx.global::<ReaderClose>().entries.clone();
    for entry in &entries {
        let _ = entry.view.update(cx, |v, _| v.remember_explorer());
    }
    let mut coordinator = cx.update_global::<ReaderClose, _>(|s, _| {
        s.scheduled = false;
        std::mem::take(&mut s.coordinator)
    });
    for entry in &entries {
        if entry.view.upgrade().is_none() {
            coordinator.unregister(entry.id);
        }
    }
    let live: Vec<_> = entries
        .iter()
        .filter(|e| e.view.upgrade().is_some())
        .cloned()
        .collect();
    let result = with_sessions(&live, &mut [], cx, &mut |sessions| {
        coordinator.progress(sessions)
    });
    cx.update_global::<ReaderClose, _>(|s, _| s.coordinator = coordinator);
    // Every core commit has finished before performing any window effect.
    if let Some(Progress::Committed(ids)) = result {
        cx.update_global::<ReaderClose, _>(|s, _| {
            for id in &ids {
                s.coordinator.unregister(*id);
            }
            s.entries.retain(|e| !ids.contains(&e.id));
        });
        for entry in live.iter().filter(|e| ids.contains(&e.id)) {
            let _ = entry
                .window
                .update(cx, |_, window, _| window.remove_window());
        }
    } else {
        for entry in &live {
            let _ = entry.view.update(cx, |v, cx| {
                v.sync_close_prompt();
                cx.notify();
            });
        }
        if let Some(Progress::Pending(id)) = result
            && let Some(entry) = live.iter().find(|e| e.id == id)
        {
            let _ = entry
                .window
                .update(cx, |_, window, _| window.activate_window());
        }
    }
}
impl ReaderView {
    pub(super) fn window_should_close(&mut self, cx: &mut Context<Self>) -> bool {
        if self.session.closed() {
            return true;
        }
        if self.modal.is_none() {
            request(self.session.id(), false, cx);
        }
        false
    }
    pub(super) fn sync_close_prompt(&mut self) {
        match (
            self.session.close_prompt().map(|p| p.text()),
            &mut self.modal,
        ) {
            (
                Some(now),
                Some(Modal::StatusDetails {
                    kind: Details::Close,
                    text,
                    ..
                }),
            ) => *text = now,
            (Some(text), None) => {
                self.modal_scroll.set(0);
                self.modal = Some(Modal::StatusDetails {
                    title: "Unsaved changes",
                    text,
                    logo: None,
                    kind: Details::Close,
                });
            }
            (
                None,
                Some(Modal::StatusDetails {
                    kind: Details::Close,
                    ..
                }),
            ) => self.modal = None,
            _ => {}
        }
    }
    pub(super) fn choose_close(
        &mut self,
        choice: code_core::close::Choice,
        cx: &mut Context<Self>,
    ) {
        if let Some(p) = self.session.close_prompt() {
            self.dispatch(
                Command::ChooseClose {
                    ticket: p.ticket,
                    buffer: p.buffer,
                    choice,
                },
                cx,
            );
        }
    }
}
