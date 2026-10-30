//! Compact forms; all acceptance and validation remain in App's existing prompt handler.
use super::*;
use code_core::chrome::forms::{Form, FormText};

impl App {
    pub(crate) fn form_text(&self) -> Option<FormText> {
        let form = match &self.modal.as_ref()?.kind {
            Prompt::Create { kind, dir } => Form::Create(*kind, dir),
            Prompt::SaveAs(_) => Form::SaveAs,
            Prompt::RenameTerminal { .. } => Form::RenameTerminal,
            Prompt::NewScratch => Form::Scratch,
            Prompt::Relocate { kind, from } => Form::Relocate(*kind, from),
            Prompt::PasteEntry(request) => Form::Paste(request),
            Prompt::ConfirmDelete { removal, from } => Form::Delete(*removal, from),
            Prompt::NewBranch { from, .. } => Form::Branch(from.as_deref()),
            Prompt::Revision => Form::Revision,
            Prompt::GoTo => Form::GoTo,
            Prompt::WorkspaceRoot => Form::Workspace,
            Prompt::SqlMetadata => Form::SqlMetadata,
            _ => return None,
        };
        Some(self.session.form_text(form))
    }

    pub(super) fn draw_form(&mut self, frame: &mut ratatui::Frame, geometry: &mut Geometry, p: &Palette, text: FormText) {
        let Some(modal) = &mut self.modal else { return };
        let width = frame.area().width.saturating_sub(2).min(72);
        let cols = width.saturating_sub(4).max(1) as usize;
        let lines: Vec<_> = modal.error.as_deref().into_iter().map(|s| (s, true)).chain([(text.description.as_str(), false)])
            .flat_map(|(s, error)| wrap_text(s, cols).into_iter().map(move |s| (s, error))).collect();
        let check = if let Prompt::NewBranch { checkout, .. } = modal.kind { Some(checkout) } else { None };
        let compact = frame.area().height < 14;
        let head = u16::from(text.field.is_some()) * if compact { 1 } else { 3 } + u16::from(check.is_some());
        let area = centered(frame.area(), width, (head + lines.len().min(u16::MAX as usize - 10) as u16 + 6).min(frame.area().height.saturating_sub(2)));
        geometry.modal = area;
        if area.is_empty() { return; }
        frame.render_widget(Clear, area);
        frame.render_widget(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(Style::default().fg(color(p.border))).title(format!(" {} ", text.title))
            .title_style(Style::default().fg(color(p.fg)).add_modifier(Modifier::BOLD)).style(style(p.fg, p.status_bg)), area);
        let content = inner(area);
        if content.is_empty() { return; }
        let mut y = content.y;
        if let Some(label) = text.field {
            if !compact {
                line(frame.buffer_mut(), Rect::new(content.x, y, content.width, 1), label, style(p.fg, p.status_bg));
                y = y.saturating_add(1).min(content.bottom());
            }
            if y < content.bottom() {
                let field = Rect::new(content.x, y, content.width, 1);
                fill(frame.buffer_mut(), field, style(p.fg, p.bg));
                let prefix = if compact { format!("{}: ", code_core::chrome::ellipsize(label, content.width as usize / 3)) } else { " ".into() };
                let shown = tail_columns(&modal.input, (content.width as usize).saturating_sub(prefix.width() + 1));
                line(frame.buffer_mut(), field, &format!("{prefix}{shown}"), style(p.fg, if modal.selected { p.selected_bg } else { p.bg }).add_modifier(Modifier::UNDERLINED));
                frame.set_cursor_position((content.x + (shown.width() as u16 + prefix.width() as u16).min(content.width.saturating_sub(1)), y));
            }
            y = y.saturating_add(if compact { 1 } else { 2 }).min(content.bottom());
        }
        if let Some(checked) = check && y < content.bottom() {
            geometry.form_checkbox = Rect::new(content.x, y, content.width, 1);
            line(frame.buffer_mut(), geometry.form_checkbox, if checked { "[x] Check out the new branch (Tab)" } else { "[ ] Check out the new branch (Tab)" }, style(p.fg, p.status_bg));
            y += 1;
        }
        let footer_y = content.bottom().saturating_sub(1);
        geometry.modal_rows = Rect::new(content.x, y.min(footer_y), content.width, footer_y.saturating_sub(y).saturating_sub(u16::from(!compact)));
        geometry.modal_content_rows = lines.len();
        modal.top = modal.top.min(lines.len().saturating_sub(geometry.modal_rows.height as usize));
        for (i, (s, error)) in lines.iter().skip(modal.top).take(geometry.modal_rows.height as usize).enumerate() {
            line(frame.buffer_mut(), row(geometry.modal_rows, i as u16), s, style(if *error { p.readable_on(p.error, p.status_bg) } else { p.fg }, p.status_bg));
        }
        let half = content.width / 2;
        let buttons = [Rect::new(content.x, footer_y, half, 1), Rect::new(content.x + half, footer_y, content.width - half, 1)];
        // The action first and Cancel last, as in every other dialog.
        for (i, (rect, label)) in buttons.iter().zip([format!("{}  Enter", text.submit), "Cancel  Esc".to_string()]).enumerate() {
            let fg = if i == 0 && text.destructive { p.error } else { p.fg };
            let bg = if i == 0 && !text.destructive { p.selected_bg } else { p.status_bg };
            line(frame.buffer_mut(), *rect, &code_core::chrome::ellipsize(&format!("[{label}]"), rect.width as usize), style(fg, bg).add_modifier(Modifier::BOLD));
        }
        geometry.form_buttons = Some(buttons);
    }
}
