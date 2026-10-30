//! Details keep their actions visible; narrow terminals stack buttons instead of clipping them away.
use super::*;
use crate::app::Details;

pub(crate) struct Button {
    pub(crate) label: String,
    key: String,
    disabled: bool,
    destructive: bool,
}
impl Button {
    fn new(label: &str, key: &str, disabled: bool, destructive: bool) -> Self {
        Self { label: label.to_owned(), key: key.to_owned(), disabled, destructive }
    }
}
impl App {
    pub(crate) fn details_buttons(&self) -> Vec<Button> {
        let Some(crate::app::Modal { kind: Prompt::Details { kind, .. }, .. }) = &self.modal else { return Vec::new() };
        let b = Button::new;
        let close = || b("Close", "Esc", false, false);
        match kind {
            Details::Terminal(_) => vec![b(self.session.state().terminal().prompt().map_or("Confirm", |p| p.accept), "Enter", false, true), b("Cancel", "Esc", false, false)],
            Details::Install => self.session.server_install_actions().iter().map(|id| b(id.label(), "", self.session.disabled(*id).is_some(), false)).chain([close()]).collect(),
            Details::External => code_core::external::Choice::ALL.into_iter().zip(["s", "o", "Esc"]).map(|(c, k)| b(c.label(), k, self.session.external_prompt().is_some_and(|p| p.disabled(c).is_some()), false)).collect(),
            Details::Reconcile => code_core::reconcile::Choice::ALL.into_iter().map(|c| b(c.label(), c.key(), self.session.reconcile_prompt().is_some_and(|p| p.disabled(c).is_some()), matches!(c, code_core::reconcile::Choice::TakeDisk | code_core::reconcile::Choice::SaveDraft))).collect(),
            Details::Replace => code_core::save::ReplaceChoice::ALL.into_iter().zip(["r", "Esc"]).map(|(c, k)| b(c.label(), k, false, c == code_core::save::ReplaceChoice::Replace)).collect(),
            Details::Recovered => code_core::recovery::Choice::ALL.into_iter().zip(["r", "d", "Esc"]).map(|(c, k)| b(c.label(), k, self.session.recovery_prompt().is_some_and(|p| p.disabled(c).is_some()), c == code_core::recovery::Choice::Discard)).collect(),
            Details::Close => [code_core::close::Choice::Save, code_core::close::Choice::Discard, code_core::close::Choice::Cancel].into_iter().zip(["s", "d", "Esc"]).map(|(c, k)| {
                let prompt = self.session.close_prompt();
                let disabled = prompt.as_ref().is_some_and(|p| match c { code_core::close::Choice::Save => p.save_disabled.is_some(), code_core::close::Choice::Discard => p.discard_disabled.is_some(), code_core::close::Choice::Cancel => false });
                b(prompt.map_or(c.label(), |p| p.label(c)), k, disabled, c == code_core::close::Choice::Discard)
            }).collect(),
            Details::Checkout => self.session.checkout_choices().into_iter().map(|(label, c)| {
                use code_core::branches::CheckoutChoice;
                b(label, match c { CheckoutChoice::Smart => "s", CheckoutChoice::Force => "f", CheckoutChoice::ConfirmForce => "y", CheckoutChoice::Cancel => "Esc" }, false, matches!(c, CheckoutChoice::Force | CheckoutChoice::ConfirmForce))
            }).collect(),
            Details::Change => vec![b("Show Diff", "d", false, false), b("Revert Change", "r", false, true), close()],
            Details::Sql => vec![b("SQL menu", "m", false, false), b("Copy", "Ctrl+C", false, false), close()],
            Details::Commit => vec![b("Show Diff", "d", false, false), b("Copy Revision", "c", false, false), close()],
            Details::Text => vec![b("Copy", "Ctrl+C", false, false), close()],
        }
    }
}

fn columns(buttons: &[Button], width: u16, max_rows: u16) -> usize {
    let desired = buttons.iter().map(|b| b.label.width() + b.key.width() + 3).max().unwrap_or(1);
    ((width as usize + 1) / (desired + 1)).max(buttons.len().div_ceil(max_rows.max(1) as usize)).clamp(1, buttons.len().max(1))
}

pub(super) fn footer_rows(buttons: &[Button], width: u16, max_rows: u16) -> u16 {
    buttons.len().div_ceil(columns(buttons, width, max_rows)) as u16
}

pub(super) fn paint_footer(frame: &mut ratatui::Frame, geometry: &mut Geometry, area: Rect, buttons: &[Button], kind: Details, p: &Palette) {
    let cols = columns(buttons, area.width, area.height);
    let width = area.width.saturating_sub(cols.saturating_sub(1) as u16) / cols as u16;
    let mut rects = Vec::new();
    for (i, button) in buttons.iter().enumerate() {
        let x = area.x.saturating_add((i % cols) as u16 * (width + 1)).min(area.right());
        let y = area.y.saturating_add((i / cols) as u16).min(area.bottom());
        let rect = Rect::new(x, y, width.min(area.right() - x), u16::from(y < area.bottom()));
        let fg = if button.disabled { p.dim } else if button.destructive { p.error } else { p.fg };
        let bg = if i == 0 && !button.destructive && !button.disabled { p.selected_bg } else { p.status_bg };
        let suffix = if button.key.is_empty() { "]".to_string() } else { format!(" {}]", button.key) };
        let label = code_core::chrome::ellipsize(&button.label, (width as usize).saturating_sub(suffix.width() + 1));
        let shown = code_core::chrome::ellipsize(&format!("[{label}{suffix}"), width as usize);
        line(frame.buffer_mut(), rect, &shown, style(fg, bg).add_modifier(Modifier::BOLD));
        rects.push(rect);
    }
    match kind {
        Details::Text => { geometry.details_copy = rects.first().copied().unwrap_or_default(); geometry.details_close = rects.get(1).copied().unwrap_or_default(); }
        Details::Change | Details::Sql | Details::Commit => { geometry.details_diff = rects.first().copied().unwrap_or_default(); geometry.details_copy = rects.get(1).copied().unwrap_or_default(); geometry.details_close = rects.get(2).copied().unwrap_or_default(); }
        Details::Terminal(_) | Details::Install | Details::External | Details::Reconcile | Details::Replace | Details::Recovered | Details::Close | Details::Checkout => geometry.details_choices = rects,
    }
}
