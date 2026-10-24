//! Presentation and input for Save As: the platform's save dialog, then the core's Replace decision.
use super::*;
use reader_core::save::ReplaceChoice;

impl ReaderView {
    pub(super) fn prompt_save_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.session.state().active().map(|t| t.id()) else { return };
        let (dir, name) = self.session.save_as_start();
        let answer = cx.prompt_for_new_path(&dir, name.as_deref());
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(path))) = answer.await {
                let _ = this.update(cx, |v, cx| v.dispatch(Command::SaveAs { tab, path }, cx));
            }
        })
        .detach();
    }

    pub(super) fn sync_replace_prompt(&mut self) {
        match (self.session.save_as_prompt().map(|p| p.text()), &mut self.modal) {
            (Some(now), Some(Modal::StatusDetails { kind: Details::Replace, text, .. })) => *text = now,
            (Some(text), None) => {
                self.modal_scroll.set(0);
                self.modal = Some(Modal::StatusDetails { title: "Replace existing file?", text, logo: None, kind: Details::Replace });
            }
            (None, Some(Modal::StatusDetails { kind: Details::Replace, .. })) => self.modal = None,
            _ => {}
        }
    }

    pub(super) fn choose_replace(&mut self, choice: ReplaceChoice, cx: &mut Context<Self>) {
        if let Some(prompt) = self.session.save_as_prompt() {
            self.dispatch(Command::ChooseReplace { ticket: prompt.ticket, choice }, cx);
        }
    }
}
