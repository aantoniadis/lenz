//! The external-change dialog: the core owns the sides and the decisions, this window shows and dispatches them.
use super::*;

impl ReaderView {
    pub(super) fn sync_reconcile_prompt(&mut self) {
        match (self.session.reconcile_prompt().map(|prompt| prompt.text()), &mut self.modal) {
            (Some(now), Some(Modal::StatusDetails { kind: Details::Reconcile, text, .. })) => *text = now,
            (Some(text), None) => {
                self.modal_scroll.set(0);
                self.modal = Some(Modal::StatusDetails { title: "File changed on disk", text, logo: None, kind: Details::Reconcile });
            }
            // A comparison opened from the dialog takes its place; the dialog returns when it closes.
            (None, Some(Modal::StatusDetails { kind: Details::Reconcile, .. })) => self.modal = None,
            _ => {}
        }
    }

    pub(super) fn choose_reconcile(&mut self, choice: code_core::reconcile::Choice, cx: &mut Context<Self>) {
        if let Some(prompt) = self.session.reconcile_prompt() {
            self.dispatch(Command::ChooseReconcile { ticket: prompt.ticket, choice }, cx);
        }
    }

    /// The dialog's buttons, greyed where the core says a choice cannot run yet.
    pub(super) fn reconcile_disabled(&self) -> Option<Vec<bool>> {
        let prompt = self.session.reconcile_prompt()?;
        Some(code_core::reconcile::Choice::ALL.iter().map(|choice| prompt.disabled(*choice).is_some()).collect())
    }
}
