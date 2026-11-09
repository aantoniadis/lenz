//! Native keyboard and clipboard transport for shared one-line input commands.
use gpui::{App, KeyDownEvent};
use lenz_content::field_edit::{Command, Key};

/// GPUI's macOS backend consults this before routing printable keys to an IME. Active marked input
/// already goes to the platform first; an Option shortcut must reach the application before a new phase.
pub fn prefers_native_input(stale: bool, macos: bool, alt: bool) -> bool {
    !stale && !(macos && alt)
}

/// AppKit must interpret Option-produced text itself: a spacing accent in `key_char` may still be
/// a dead key. Synthetic/unregistered events retain the semantic fallback; an active preedit stays native.
pub fn defer_option_input(registered: bool, macos: bool, marked: bool) -> bool {
    marked || registered && macos
}

/// XKB clears `key_char` while composing but may retain a printable physical key name (for example
/// apostrophe for dead_acute). That name is not committed text and must not enter the semantic fallback.
pub fn defer_uncommitted_key(registered: bool, key: &gpui::Keystroke) -> bool {
    registered
        && key.key_char.is_none()
        && (key.key.chars().count() == 1 || key.key == "space")
        && !key.modifiers.control
        && !key.modifiers.platform
}

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

/// Resolve a click using the same bounded, shaped viewport as paint. Hidden/control bytes remain in source coordinates.
pub fn hit_test(
    view: View<'_>,
    point: gpui::Point<gpui::Pixels>,
    window: &gpui::Window,
    metrics: crate::Metrics<'_>,
) -> usize {
    let width = f32::from(view.bounds.size.width).max(0.0);
    let shown = view
        .cursor
        .visible(view.text, (width / view.char_w.max(1.0)).floor() as usize);
    let start = view
        .cursor
        .caret(view.text)
        .saturating_sub(shown.prefix.len());
    let line = shape(shown.text, window, metrics);
    let position = |at| {
        f32::from(line.x_for_index(
            lenz_content::display::field_text(shown.text.get(..at).unwrap_or_default()).len(),
        ))
    };
    let shift = (position(shown.prefix.len()) - (width - 2.0).max(0.0)).max(0.0);
    let wanted = f32::from(point.x - view.bounds.origin.x) + shift;
    lenz_content::line_edit::boundaries(shown.text)
        .min_by(|a, b| {
            (position(*a) - wanted)
                .abs()
                .total_cmp(&(position(*b) - wanted).abs())
        })
        .map_or(start, |at| start + at)
}

pub fn bounds_for_range(
    view: View<'_>,
    range: std::ops::Range<usize>,
    window: &gpui::Window,
    metrics: crate::Metrics<'_>,
) -> Option<gpui::Bounds<gpui::Pixels>> {
    let width = f32::from(view.bounds.size.width).max(0.0);
    let shown = view
        .cursor
        .visible(view.text, (width / view.char_w.max(1.0)).floor() as usize);
    let start = view
        .cursor
        .caret(view.text)
        .saturating_sub(shown.prefix.len());
    let line = shape(shown.text, window, metrics);
    let position = |at| {
        Some(f32::from(line.x_for_index(
            lenz_content::display::field_text(shown.text.get(..at)?).len(),
        )))
    };
    let shift = (position(shown.prefix.len())? - (width - 2.0).max(0.0)).max(0.0);
    let a = range.start.saturating_sub(start).min(shown.text.len());
    let b = range.end.saturating_sub(start).min(shown.text.len());
    let ax = (position(a)? - shift).clamp(0.0, width);
    let bx = (position(b)? - shift).clamp(0.0, width);
    let (left, right) = (ax.min(bx), ax.max(bx));
    Some(gpui::Bounds {
        origin: gpui::point(view.bounds.origin.x + gpui::px(left), view.bounds.origin.y),
        size: gpui::size(gpui::px((right - left).max(2.0)), view.bounds.size.height),
    })
}

fn shape(text: &str, window: &gpui::Window, metrics: crate::Metrics<'_>) -> gpui::ShapedLine {
    let rendered = lenz_content::display::field_text(text).into_owned();
    let run = gpui::TextRun {
        len: rendered.len(),
        font: metrics.font.clone(),
        color: gpui::rgb(0).into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(rendered.into(), gpui::px(metrics.font_size), &[run], None)
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
                        x + position(a).min(position(b)) - shift,
                        y,
                        (position(b) - position(a)).abs(),
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

#[cfg(test)]
mod preference_tests {
    #[test]
    fn explicit_option_keys_and_stale_fields_go_to_the_app_before_new_native_input() {
        assert!(super::prefers_native_input(false, true, false));
        assert!(!super::prefers_native_input(false, true, true));
        assert!(!super::prefers_native_input(true, true, false));
        assert!(super::prefers_native_input(false, false, true));
    }
    #[test]
    fn registered_macos_option_symbols_and_dead_keys_reach_appkit() {
        assert!(super::defer_option_input(true, true, false));
        assert!(super::defer_option_input(true, true, true));
        assert!(!super::defer_option_input(true, false, false));
        assert!(!super::defer_option_input(false, true, false));
        assert!(super::defer_option_input(false, false, true));
    }
    #[test]
    fn registered_uncommitted_physical_names_do_not_become_text() {
        let mut key = gpui::Keystroke {
            key: "'".into(),
            key_char: None,
            modifiers: Default::default(),
        };
        assert!(super::defer_uncommitted_key(true, &key));
        assert!(!super::defer_uncommitted_key(false, &key));
        key.key_char = Some("'".into());
        assert!(!super::defer_uncommitted_key(true, &key));
        key.key = "space".into();
        key.key_char = None;
        assert!(super::defer_uncommitted_key(true, &key));
        key.key_char = Some(" ".into());
        assert!(!super::defer_uncommitted_key(true, &key));
        key.key = "left".into();
        key.key_char = None;
        assert!(!super::defer_uncommitted_key(true, &key));
        key.key = "c".into();
        key.modifiers.control = true;
        assert!(!super::defer_uncommitted_key(true, &key));
    }
}
