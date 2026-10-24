//! Presentation and input for recovered drafts: one dialog per offer, with the core's labels and reasons.
use super::*;
use reader_core::recovery::Choice;

impl ReaderView {
    pub(super) fn sync_recovery_prompt(&mut self) {
        match (self.session.recovery_prompt().map(|prompt| prompt.text()), &mut self.modal) {
            (Some(now), Some(Modal::StatusDetails { kind: Details::Recovered, text, .. })) => *text = now,
            (Some(text), None) => {
                self.modal_scroll.set(0);
                self.modal = Some(Modal::StatusDetails { title: "Recovered unsaved work", text, logo: None, kind: Details::Recovered });
            }
            (None, Some(Modal::StatusDetails { kind: Details::Recovered, .. })) => self.modal = None,
            _ => {}
        }
    }

    pub(super) fn choose_recovery(&mut self, choice: Choice, cx: &mut Context<Self>) {
        if let Some(prompt) = self.session.recovery_prompt() {
            self.dispatch(Command::ChooseRecoveredDraft { ticket: prompt.ticket, choice }, cx);
        }
    }
}
