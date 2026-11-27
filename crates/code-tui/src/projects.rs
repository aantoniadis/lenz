//! Project tabs in the terminal: the independent views (one [`App`] and session each) shown one at a time under a
//! project tab strip, or Side by Side as columns. Each shown view draws into its own off-screen buffer the size of its
//! column, which is copied into place, so a view lays itself out exactly as it would in a terminal of that size;
//! pointer input is translated into the view's own coordinates. With one view nothing changes: no strip, no copy.
use code_core::projects::{self, Layout, Projects};
use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    Frame,
    backend::TestBackend,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};
use unicode_width::UnicodeWidthStr;

use crate::app::App;

/// A divider column between Side by Side projects.
const DIVIDER: u16 = 1;
/// The longest title a project tab shows before it is shortened.
const TITLE_COLUMNS: usize = 24;

/// What the strip and the columns were drawn as, for the pointer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Geometry {
    /// Each project tab: its columns, its close mark's column, and the project.
    pub tabs: Vec<(std::ops::Range<u16>, u16, usize)>,
    /// The New Project Tab and Side by Side buttons.
    pub new: std::ops::Range<u16>,
    pub toggle: std::ops::Range<u16>,
    /// Each shown project's area, left to right.
    pub columns: Vec<(usize, Rect)>,
}

/// What a pointer press on the strip asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StripHit {
    Activate(usize),
    Close(usize),
    New,
    ToggleLayout,
}

fn color(rgb: u32) -> Color { Color::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8) }
fn style(fg: u32, bg: u32) -> Style { Style::default().fg(color(fg)).bg(color(bg)) }

/// Shorten `text` to at most `columns`, ending in `…` when cut.
fn shorten(text: &str, columns: usize) -> String {
    if text.width() <= columns { return text.to_owned(); }
    let mut out = String::new();
    for c in text.chars() {
        if out.width() + unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) + 1 > columns { break; }
        out.push(c);
    }
    out.push('…');
    out
}

/// Tell every session where it stands among the projects (menus resolve project commands from it).
pub(crate) fn sync(views: &mut [App], projects: &Projects) {
    for (i, view) in views.iter_mut().enumerate() {
        view.session.set_projects(projects.info(i));
        // The strip numbers the projects; the status bar's View i/n is for one view at a time.
        view.independent_view = None;
    }
}

/// Draw the projects: the active one alone as before when it is the only one; otherwise the strip on the first row
/// and the shown projects below it.
pub(crate) fn draw(frame: &mut Frame, views: &mut [App], projects: &Projects, geometry: &mut Geometry) {
    *geometry = Geometry::default();
    let Some(active) = views.get(projects.active()).map(|_| projects.active()) else { return };
    if !projects.strip() {
        views[active].draw(frame);
        return;
    }
    let area = frame.area();
    let palette = **views[active].session.state().palette();
    draw_strip(frame.buffer_mut(), Rect::new(area.x, area.y, area.width, 1.min(area.height)), views, projects, &palette, geometry);
    let body = Rect::new(area.x, area.y.saturating_add(1), area.width, area.height.saturating_sub(1));
    if body.is_empty() { return; }
    let capacity = projects::capacity(u32::from(body.width), projects::MIN_COLUMN_CELLS, u32::from(DIVIDER));
    let shown = projects.shown(capacity);
    let columns = projects::columns(u32::from(body.width), shown.len(), u32::from(DIVIDER));
    for (index, column) in shown.zip(columns) {
        let x = body.x.saturating_add(u16::try_from(column.start).unwrap_or(u16::MAX));
        let width = u16::try_from(column.end - column.start).unwrap_or(0);
        let rect = Rect::new(x, body.y, width, body.height);
        if x > body.x {
            let divider = Rect::new(x - DIVIDER, body.y, DIVIDER, body.height);
            for y in divider.top()..divider.bottom() {
                frame.buffer_mut()[(divider.x, y)].set_symbol("│").set_style(style(if index == active || index == active + 1 { palette.accent } else { palette.border }, palette.bg));
            }
        }
        draw_offscreen(frame, &mut views[index], rect, index == active);
        geometry.columns.push((index, rect));
    }
}

/// The project tab strip: each project's number, title and unsaved mark with its close mark; the active one raised.
/// The New Project Tab (`+`) and layout buttons sit at the right end.
fn draw_strip(buf: &mut Buffer, area: Rect, views: &[App], projects: &Projects, p: &lenz_content::theme::Palette, geometry: &mut Geometry) {
    if area.is_empty() { return; }
    for x in area.left()..area.right() { buf[(x, area.y)].set_symbol(" ").set_style(style(p.fg, p.header_bg)); }
    let toggle = match projects.layout() { Layout::Tabs => " ◫ Side by Side ", Layout::SideBySide => " ▭ Tabs " };
    let buttons = toggle.width() as u16 + 3;
    let room = area.width.saturating_sub(buttons);
    let labels: Vec<String> = views.iter().enumerate().map(|(i, v)| {
        let dirty = if v.session.project_dirty() { " ●" } else { "" };
        format!(" {} {}{dirty} ", i + 1, shorten(&v.session.project_title(), TITLE_COLUMNS))
    }).collect();
    let widths: Vec<u16> = labels.iter().map(|l| l.width() as u16 + 2).collect();
    // Scroll the tabs so the active one is in view when they do not all fit.
    let mut first = 0;
    while first < projects.active() && widths[first..=projects.active()].iter().sum::<u16>() > room { first += 1; }
    let mut x = area.x;
    for (i, label) in labels.iter().enumerate().skip(first) {
        let width = widths[i];
        if x.saturating_add(width) > area.x + room { break; }
        let active = i == projects.active();
        let st = if active { style(p.fg, p.bg).add_modifier(Modifier::BOLD) } else { style(p.dim, p.header_bg) };
        buf.set_stringn(x, area.y, label, width as usize, st);
        let close = x + width - 2;
        buf.set_stringn(close, area.y, "× ", 2, if active { style(p.dim, p.bg) } else { style(p.dim, p.header_bg) });
        if active { buf[(x, area.y)].set_symbol("▎").set_style(style(p.accent, p.bg)); }
        geometry.tabs.push((x..x + width, close, i));
        x += width;
    }
    let new_x = area.right().saturating_sub(buttons);
    buf.set_stringn(new_x, area.y, " + ", 3, style(p.accent, p.header_bg).add_modifier(Modifier::BOLD));
    geometry.new = new_x..new_x + 3;
    buf.set_stringn(new_x + 3, area.y, toggle, toggle.width(), style(p.fg, p.status_bg));
    geometry.toggle = new_x + 3..area.right();
}

/// Draw `view` as if the terminal were `rect`, then copy it into place; the active view's cursor goes with it.
fn draw_offscreen(frame: &mut Frame, view: &mut App, rect: Rect, active: bool) {
    if rect.is_empty() { return; }
    let mut terminal = match view.offscreen.take() {
        Some(mut t) => {
            if t.backend().buffer().area.width != rect.width || t.backend().buffer().area.height != rect.height {
                t.backend_mut().resize(rect.width, rect.height);
            }
            t
        }
        None => match ratatui::Terminal::new(TestBackend::new(rect.width, rect.height)) { Ok(t) => t, Err(never) => match never {} },
    };
    if let Err(never) = terminal.draw(|f| view.draw(f)) { match never {} }
    let source = terminal.backend().buffer();
    for y in 0..rect.height {
        for x in 0..rect.width {
            frame.buffer_mut()[(rect.x + x, rect.y + y)] = source[(x, y)].clone();
        }
    }
    if active && terminal.backend().cursor_visible() {
        let at = terminal.backend().cursor_position();
        if at.x < rect.width && at.y < rect.height { frame.set_cursor_position((rect.x + at.x, rect.y + at.y)); }
    }
    view.offscreen = Some(terminal);
}

impl Geometry {
    /// What a press at `(x, y)` on the strip row hits.
    pub(crate) fn strip_hit(&self, x: u16) -> Option<StripHit> {
        if self.new.contains(&x) { return Some(StripHit::New); }
        if self.toggle.contains(&x) { return Some(StripHit::ToggleLayout); }
        self.tabs.iter().find(|(range, _, _)| range.contains(&x)).map(|(_, close, i)| if x == *close { StripHit::Close(*i) } else { StripHit::Activate(*i) })
    }

    /// The shown project under `(x, y)` and its area.
    pub(crate) fn column_at(&self, x: u16, y: u16) -> Option<(usize, Rect)> {
        self.columns.iter().copied().find(|(_, r)| x >= r.left() && x < r.right() && y >= r.top() && y < r.bottom())
    }

    pub(crate) fn column_of(&self, index: usize) -> Option<Rect> {
        self.columns.iter().find(|(i, _)| *i == index).map(|(_, r)| *r)
    }
}

/// Where a pointer event goes once the strip has had its say: the project it is for, and the event in that view's
/// own coordinates. A press or wheel goes to the project under the pointer; a drag, release or move stays with the
/// active project (so a selection drag can leave its column); a move over another column goes nowhere.
pub(crate) fn route(geometry: &Geometry, active: usize, mouse: MouseEvent) -> Option<(usize, MouseEvent)> {
    let under = geometry.column_at(mouse.column, mouse.row);
    let (index, rect) = match mouse.kind {
        MouseEventKind::Down(_) | MouseEventKind::ScrollDown | MouseEventKind::ScrollUp | MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => under?,
        MouseEventKind::Moved => under.filter(|(i, _)| *i == active)?,
        MouseEventKind::Drag(_) | MouseEventKind::Up(_) => (active, geometry.column_of(active)?),
    };
    let column = mouse.column.saturating_sub(rect.x).min(rect.width.saturating_sub(1));
    let row = mouse.row.saturating_sub(rect.y).min(rect.height.saturating_sub(1));
    Some((index, MouseEvent { column, row, ..mouse }))
}

/// Whether `event` is a left press (it moves the keys to the project under it).
pub(crate) fn activates(event: &Event) -> bool {
    matches!(event, Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left | MouseButton::Right | MouseButton::Middle), .. }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent { MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE } }

    #[test]
    fn pointer_input_goes_to_the_column_under_it_in_that_views_coordinates() {
        let geometry = Geometry { columns: vec![(0, Rect::new(0, 1, 40, 20)), (1, Rect::new(41, 1, 39, 20))], ..Geometry::default() };
        let down = route(&geometry, 0, mouse(MouseEventKind::Down(MouseButton::Left), 50, 5)).unwrap();
        assert_eq!((down.0, down.1.column, down.1.row), (1, 9, 4));
        let drag = route(&geometry, 0, mouse(MouseEventKind::Drag(MouseButton::Left), 60, 5)).unwrap();
        assert_eq!((drag.0, drag.1.column), (0, 39), "a drag stays with the active project, clamped to it");
        assert!(route(&geometry, 0, mouse(MouseEventKind::Moved, 60, 5)).is_none(), "moving over another column");
        assert!(route(&geometry, 0, mouse(MouseEventKind::Down(MouseButton::Left), 40, 5)).is_none(), "the divider");
        let wheel = route(&geometry, 0, mouse(MouseEventKind::ScrollDown, 45, 2)).unwrap();
        assert_eq!(wheel.0, 1, "the wheel scrolls the project under the pointer");
    }

    #[test]
    fn strip_hits_name_the_tab_its_close_mark_and_the_buttons() {
        let geometry = Geometry { tabs: vec![(0..10, 8, 0), (10..20, 18, 1)], new: 60..63, toggle: 63..80, columns: Vec::new() };
        assert_eq!(geometry.strip_hit(3), Some(StripHit::Activate(0)));
        assert_eq!(geometry.strip_hit(18), Some(StripHit::Close(1)));
        assert_eq!(geometry.strip_hit(61), Some(StripHit::New));
        assert_eq!(geometry.strip_hit(70), Some(StripHit::ToggleLayout));
        assert_eq!(geometry.strip_hit(40), None);
        assert_eq!(shorten("a-very-long-project-name", 8), "a-very-…");
    }
}
