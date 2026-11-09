//! Terminal transport and cell placement for the shared one-line editor. No GUI dependency.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use lenz_content::field_edit::{Command, Cursor, Key};
use ratatui::{Frame, layout::Rect, style::Style};

pub fn command(key: KeyEvent) -> Option<Command> {
    let text = if let KeyCode::Char(c) = key.code {
        c.to_string()
    } else {
        String::new()
    };
    let name = match key.code {
        KeyCode::Left => "left",
        KeyCode::Right => "right",
        KeyCode::Home => "home",
        KeyCode::End => "end",
        KeyCode::Backspace => "backspace",
        KeyCode::Delete => "delete",
        KeyCode::Char(_) => &text.to_ascii_lowercase(),
        _ => return None,
    };
    Key {
        name,
        text: (!text.is_empty()).then_some(text.as_str()),
        control: key.modifiers.contains(KeyModifiers::CONTROL),
        command: key.modifiers.contains(KeyModifiers::SUPER),
        alt: key.modifiers.contains(KeyModifiers::ALT),
        shift: key.modifiers.contains(KeyModifiers::SHIFT),
    }
    .command()
}

pub struct Field<'a> {
    pub text: &'a str,
    pub cursor: Cursor,
    pub area: Rect,
    pub active: bool,
}

pub fn paint(frame: &mut Frame, field: Field<'_>, style: Style, selected: Style) {
    let area = field.area;
    if area.width == 0 || area.height == 0 {
        return;
    }
    frame.render_widget(ratatui::widgets::Clear, area);
    let shown = field.cursor.visible(field.text, usize::from(area.width));
    frame.buffer_mut().set_style(area, style);
    frame.buffer_mut().set_stringn(
        area.x,
        area.y,
        lenz_content::display::field_text(shown.text).as_ref(),
        usize::from(area.width),
        style,
    );
    let start = field
        .cursor
        .caret(field.text)
        .saturating_sub(shown.prefix.len());
    if let Some(range) = field.cursor.selection(field.text) {
        let a = range
            .start
            .max(start)
            .saturating_sub(start)
            .min(shown.text.len());
        let b = range.end.saturating_sub(start).min(shown.text.len());
        if a < b {
            let left = lenz_content::display::field_width(shown.text.get(..a).unwrap_or_default())
                .min(usize::from(area.width));
            let right = lenz_content::display::field_width(shown.text.get(..b).unwrap_or_default())
                .min(usize::from(area.width));
            frame.buffer_mut().set_style(
                Rect::new(
                    area.x + left as u16,
                    area.y,
                    right.saturating_sub(left) as u16,
                    1,
                ),
                selected,
            );
        }
    }
    if field.active {
        frame.set_cursor_position((area.x + (shown.caret as u16).min(area.width - 1), area.y));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend, style::Color};
    #[test]
    fn actual_events_select_unicode_and_paint_only_the_field_at_tiny_sizes() {
        let mut text = "aé👩🏽‍💻b".to_string();
        let mut cursor = Cursor::default();
        for key in [
            KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT),
        ] {
            cursor.execute(&mut text, command(key).unwrap());
        }
        assert_eq!(
            cursor
                .execute(
                    &mut text,
                    command(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)).unwrap()
                )
                .copy
                .as_deref(),
            Some("👩🏽‍💻")
        );
        for width in [0, 1, 2, 4, 20] {
            let mut terminal = Terminal::new(TestBackend::new(24, 3)).unwrap();
            terminal
                .draw(|frame| {
                    frame
                        .buffer_mut()
                        .set_string(0, 0, "retained", Style::default());
                    frame.buffer_mut().set_stringn(
                        2,
                        1,
                        "obsolete field suffix",
                        20,
                        Style::default(),
                    );
                    paint(
                        frame,
                        Field {
                            text: &text,
                            cursor,
                            area: Rect::new(2, 1, width, 1),
                            active: true,
                        },
                        Style::default(),
                        Style::default().bg(Color::Blue),
                    );
                })
                .unwrap();
            assert_eq!(terminal.backend().buffer()[(0, 0)].symbol(), "r");
            if width == 20 {
                assert_eq!(terminal.backend().buffer()[(2, 1)].symbol(), "a");
                assert_eq!(terminal.backend().buffer()[(3, 1)].symbol(), "é");
                assert_eq!(terminal.backend().buffer()[(4, 1)].bg, Color::Blue);
                assert_eq!(terminal.backend().buffer()[(10, 1)].symbol(), " ");
            }
        }
    }
    #[test]
    fn hidden_characters_are_visible_and_copy_preserves_the_original() {
        let mut text = "a\u{202e}\x1b[31mZ".to_string();
        let mut cursor = Cursor::default();
        cursor.execute(&mut text, Command::SelectAll);
        let mut terminal = Terminal::new(TestBackend::new(20, 2)).unwrap();
        terminal
            .draw(|frame| {
                paint(
                    frame,
                    Field {
                        text: &text,
                        cursor,
                        area: Rect::new(0, 0, 20, 1),
                        active: true,
                    },
                    Style::default(),
                    Style::default().bg(Color::Blue),
                )
            })
            .unwrap();
        let painted = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(!painted.contains(['\u{202e}', '\x1b']));
        assert!(painted.contains("a��[31mZ"));
        assert_eq!(
            cursor.execute(&mut text, Command::Copy).copy.as_deref(),
            Some("a\u{202e}\x1b[31mZ")
        );
    }
}
