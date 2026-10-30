//! Presentation and input for the core's dirty-file external handoff.
use super::*;

impl ReaderView {
    pub(super) fn sync_external_prompt(&mut self) {
        match (self.session.external_prompt().map(|p| p.text()), &mut self.modal) {
            (Some(now), Some(Modal::StatusDetails {kind:Details::External,text,..})) => *text = now,
            (Some(text),None) => {
                self.modal_scroll.set(0);
                self.modal = Some(Modal::StatusDetails {title:"Open file externally",text,logo:None,kind:Details::External});
            }
            (None,Some(Modal::StatusDetails {kind:Details::External,..})) => self.modal=None,
            _ => {}
        }
    }

    pub(super) fn choose_external(&mut self, choice: code_core::external::Choice, cx: &mut Context<Self>) {
        if let Some(prompt) = self.session.external_prompt() {
            self.dispatch(Command::ChooseExternal {ticket:prompt.ticket,choice},cx);
        }
    }
}
