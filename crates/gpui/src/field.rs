//! Native keyboard and clipboard transport for shared one-line input commands.
use gpui::{App, KeyDownEvent};
use lenz_content::field_edit::{Command, Key};

pub fn command(event: &KeyDownEvent, cx: &mut App) -> Option<Command> {
    let k = &event.keystroke;
    let name = k.key.to_ascii_lowercase();
    let command = Key {
        name: &name,
        text: k.key_char.as_deref(),
        control: k.modifiers.control,
        command: k.modifiers.platform,
        alt: k.modifiers.alt,
        shift: k.modifiers.shift,
    }
    .command()?;
    Some(match command {
        Command::Paste => cx
            .read_from_clipboard()
            .and_then(|c| c.text())
            .filter(|s| !s.is_empty())
            .map_or(Command::Paste, Command::Insert),
        other => other,
    })
}

pub struct View<'a> {
    pub text: &'a str,
    pub cursor: lenz_content::field_edit::Cursor,
    pub bounds: gpui::Bounds<gpui::Pixels>,
    pub char_w: f32,
    pub active: bool,
}

pub fn paint(
    view: View<'_>,
    window: &mut gpui::Window,
    cx: &mut App,
    metrics: crate::Metrics,
    palette: &lenz_content::theme::Palette,
) {
    use gpui::{ContentMask, SharedString, TextAlign, TextRun, px};
    let width = f32::from(view.bounds.size.width).max(0.0);
    let shown = view
        .cursor
        .visible(view.text, (width / view.char_w.max(1.0)).floor() as usize);
    let start = view
        .cursor
        .caret(view.text)
        .saturating_sub(shown.prefix.len());
    let display = |s: &str| lenz_content::display::field_text(s).into_owned();
    let rendered = display(shown.text);
    let run = TextRun {
        len: rendered.len(),
        font: metrics.font.clone(),
        color: crate::color(palette.fg),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window.text_system().shape_line(
        SharedString::from(rendered),
        px(metrics.font_size),
        &[run],
        None,
    );
    let x = f32::from(view.bounds.origin.x);
    let y = f32::from(view.bounds.origin.y);
    let position = |at: usize| {
        f32::from(line.x_for_index(display(shown.text.get(..at).unwrap_or_default()).len()))
    };
    // Fallback fonts can advance wider than the terminal-cell estimate (notably emoji).
    let caret_x = position(shown.prefix.len());
    let shift = (caret_x - (width - 2.0).max(0.0)).max(0.0);
    window.with_content_mask(
        Some(ContentMask {
            bounds: view.bounds,
        }),
        |window| {
            if let Some(range) = view.cursor.selection(view.text) {
                let a = range
                    .start
                    .max(start)
                    .saturating_sub(start)
                    .min(shown.text.len());
                let b = range.end.saturating_sub(start).min(shown.text.len());
                if a < b {
                    crate::fill(
                        window,
                        x + position(a) - shift,
                        y,
                        (position(b) - position(a)).max(0.0),
                        metrics.line_h,
                        palette.selected_bg,
                    );
                }
            }
            let _ = line.paint(
                gpui::point(px(x - shift), view.bounds.origin.y),
                px(metrics.line_h),
                TextAlign::Left,
                None,
                window,
                cx,
            );
            if view.active {
                crate::fill(
                    window,
                    x + caret_x - shift,
                    y + 1.0,
                    2.0,
                    (metrics.line_h - 2.0).max(0.0),
                    palette.caret,
                );
            }
        },
    );
}
