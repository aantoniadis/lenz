//! The scrollbar column at each pane's right edge, through the real pointer path: thumb drag, track paging, mark
//! clicks, the wheel, a split's divider, previews and tables.
use super::*;
use lenz_content::{goto::LineColumn, scrollbar::{place, MarkKind}};

fn mouse_at(a: &mut App, kind: MouseEventKind, column: u16, row: u16) {
    a.handle(Event::Mouse(MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE }));
}
fn press(a: &mut App, column: u16, row: u16) {
    mouse_at(a, MouseEventKind::Down(MouseButton::Left), column, row);
    mouse_at(a, MouseEventKind::Up(MouseButton::Left), column, row);
    draw(a, 80, 20);
}
fn opened(dir: &Path, name: &str, text: &str) -> App {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    let mut a = app(dir);
    a.dispatch(Command::Open(path));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    draw(&mut a, 80, 20);
    a
}
fn caret_line(a: &App) -> usize {
    let s = a.session.state();
    s.view(s.active().unwrap().id()).unwrap().snapshot().line_of(s.selection().unwrap().head)
}
fn go_to(a: &mut App, line: u32) { a.dispatch(Command::GoToLine(LineColumn { line, column: None })); }
/// The strip row of the first jumpable mark for source `line`, as the model places it.
fn mark_row(a: &App, pane: u8, line: usize) -> u16 {
    let strip = a.layout.panes[usize::from(pane)].strip;
    let (bar, marks) = a.session.state().scrollbar(pane).unwrap();
    let placed = place(&marks, bar.total, usize::from(strip.height));
    strip.y + placed.iter().position(|p| p.is_some_and(|i| marks[i].target == line && marks[i].kind.jumps())).unwrap() as u16
}

#[test]
fn scrollbar_drag_page_and_mark_click() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = opened(dir.path(), "long.txt", &(0..300).map(|i| format!("line {i}\n")).collect::<String>());
    go_to(&mut a, 251);
    a.dispatch(Command::ToggleBookmark);
    go_to(&mut a, 1);
    a.dispatch(Command::Scroll(0));
    draw(&mut a, 80, 20);
    let strip = a.layout.panes[0].strip;
    let (bar, marks) = a.session.state().scrollbar(0).unwrap();
    assert!(marks.iter().any(|m| m.kind == MarkKind::Bookmark && m.target == 250), "{marks:?}");
    mouse_at(&mut a, MouseEventKind::Moved, strip.x, strip.y + 3);
    assert!(!a.pointer_text, "the strip is not text");
    // The thumb at the top, dragged to the bottom: the last page.
    mouse_at(&mut a, MouseEventKind::Down(MouseButton::Left), strip.x, strip.y);
    assert!(a.scrollbar_drag.is_some(), "a press on the thumb grabs it");
    mouse_at(&mut a, MouseEventKind::Drag(MouseButton::Left), strip.x, strip.bottom() - 1);
    assert_eq!(a.session.state().top_line(), (bar.total - bar.visible) as usize, "the last page");
    mouse_at(&mut a, MouseEventKind::Up(MouseButton::Left), strip.x, strip.bottom() - 1);
    assert!(a.scrollbar_drag.is_none());
    assert_eq!(caret_line(&a), 0, "a drag moves the view, not the caret");
    // The track below the thumb: one page down.
    a.dispatch(Command::Scroll(0));
    draw(&mut a, 80, 20);
    press(&mut a, strip.x, strip.y + strip.height * 3 / 5);
    assert_eq!(a.session.state().top_line(), usize::from(a.layout.panes[0].text.height), "one page down");
    press(&mut a, strip.x, strip.y);
    assert_eq!(a.session.state().top_line(), 0, "and back up");
    // A mark: the caret on its line, and Back returns.
    let row = mark_row(&a, 0, 250);
    press(&mut a, strip.x, row);
    assert_eq!(caret_line(&a), 250, "the mark's line");
    assert!(a.scrollbar_drag.is_none());
    a.dispatch(Command::Back);
    assert_eq!(caret_line(&a), 0, "Back returns");
    // A press on the thumb that does not move goes to the mark under it.
    a.dispatch(Command::Scroll(240));
    draw(&mut a, 80, 20);
    let row = mark_row(&a, 0, 250);
    mouse_at(&mut a, MouseEventKind::Down(MouseButton::Left), strip.x, row);
    assert!(a.scrollbar_drag.is_some(), "the mark is under the thumb");
    mouse_at(&mut a, MouseEventKind::Up(MouseButton::Left), strip.x, row);
    assert_eq!((caret_line(&a), a.scrollbar_drag.is_none()), (250, true));
    // A drag whose pane shows another tab is dropped.
    a.dispatch(Command::Scroll(0));
    draw(&mut a, 80, 20);
    mouse_at(&mut a, MouseEventKind::Down(MouseButton::Left), strip.x, strip.y);
    assert!(a.scrollbar_drag.is_some());
    let other = dir.path().join("other.txt");
    std::fs::write(&other, "x\n".repeat(300)).unwrap();
    a.dispatch(Command::Open(other));
    settle(&mut a, |a| a.session.state().current_source().is_some_and(|s| s.source().ends_with("other.txt")));
    draw(&mut a, 80, 20);
    mouse_at(&mut a, MouseEventKind::Drag(MouseButton::Left), strip.x, strip.bottom() - 1);
    assert_eq!((a.session.state().top_line(), a.scrollbar_drag.is_none()), (0, true), "stale: the other tab stays");
}

#[test]
fn split_strip_does_not_take_the_divider() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = opened(dir.path(), "long.txt", &"text\n".repeat(300));
    a.dispatch(Command::Split(code_core::session::Split::Columns));
    draw(&mut a, 80, 20);
    assert_eq!(a.session.state().focused_pane(), 1);
    let (s0, divider) = (a.layout.panes[0].strip, a.layout.divider);
    assert_eq!((s0.width, s0.x + 1), (1, divider.x));
    let y = s0.y + 4;
    mouse_at(&mut a, MouseEventKind::Down(MouseButton::Left), divider.x, y);
    assert!(a.split_drag.is_some() && a.scrollbar_drag.is_none(), "the divider keeps its drag");
    mouse_at(&mut a, MouseEventKind::Up(MouseButton::Left), divider.x, y);
    // The first pane's strip is its scrollbar: it focuses that pane and pages it.
    mouse_at(&mut a, MouseEventKind::Down(MouseButton::Left), s0.x, s0.bottom() - 1);
    assert!(a.split_drag.is_none());
    assert_eq!(a.session.state().focused_pane(), 0);
    mouse_at(&mut a, MouseEventKind::Up(MouseButton::Left), s0.x, s0.bottom() - 1);
    assert!(a.session.state().top_line() > 0, "paged");
    // The wheel over the second pane's strip scrolls that pane, as over its text.
    let s1 = a.layout.panes[1].strip;
    let before = a.session.state().shown(1).unwrap().top_line();
    mouse_at(&mut a, MouseEventKind::ScrollDown, s1.x, s1.y + 2);
    assert_eq!(a.session.state().focused_pane(), 1);
    assert!(a.session.state().shown(1).unwrap().top_line() > before);
}

#[test]
fn scrollbar_pages_a_preview_and_a_table() {
    let dir = tempfile::tempdir().unwrap();
    let md = format!("# Top\n\n{}", (0..120).map(|i| format!("Paragraph {i}.\n\n")).collect::<String>());
    let mut a = opened(dir.path(), "doc.md", &md);
    if !matches!(a.session.state().rendering(), Some(Rendering::Preview(_))) { a.dispatch(Command::ToggleMarkdownSource); }
    settle(&mut a, |a| matches!(a.session.state().rendering(), Some(Rendering::Preview(_))));
    draw(&mut a, 80, 20);
    let g = &a.layout.panes[0];
    assert!(g.preview && g.preview_top == 0 && !g.strip.is_empty());
    let strip = g.strip;
    press(&mut a, strip.x, strip.y + strip.height * 4 / 5);
    assert_eq!(a.layout.panes[0].preview_top, usize::from(a.layout.panes[0].text.height), "a page down the preview");

    let rows: String = std::iter::once("a,b\n".to_string()).chain((0..200).map(|i| format!("{i},x\n"))).collect();
    let mut a = opened(dir.path(), "data.csv", &rows);
    a.run(CommandId::CsvTable);
    settle(&mut a, |a| a.session.state().active().and_then(|t| a.session.state().csv(t.id())).is_some_and(|t| !t.pending));
    draw(&mut a, 80, 20);
    let strip = a.layout.panes[0].strip;
    assert!(!strip.is_empty());
    press(&mut a, strip.x, strip.y + strip.height * 4 / 5);
    let table = a.session.state().csv(a.session.state().active().unwrap().id()).unwrap();
    assert_eq!(table.top, table.rows, "a page down the table");
}

#[test]
fn a_held_preview_thumb_scrolls_its_own_pane_after_focus_moves() {
    let dir = tempfile::tempdir().unwrap();
    let md = format!("# Top\n\n{}", (0..120).map(|i| format!("Paragraph {i}.\n\n")).collect::<String>());
    let mut a = opened(dir.path(), "doc.md", &md);
    if !matches!(a.session.state().rendering(), Some(Rendering::Preview(_))) { a.dispatch(Command::ToggleMarkdownSource); }
    settle(&mut a, |a| matches!(a.session.state().rendering(), Some(Rendering::Preview(_))));
    a.dispatch(Command::Split(code_core::session::Split::Columns));
    let long = dir.path().join("long.txt");
    std::fs::write(&long, "text\n".repeat(300)).unwrap();
    a.dispatch(Command::Open(long));
    settle(&mut a, |a| a.session.state().current_source().is_some_and(|s| s.source().ends_with("long.txt")));
    draw(&mut a, 80, 20);
    assert_eq!(a.session.state().focused_pane(), 1);
    let g = &a.layout.panes[0];
    assert!(g.preview && g.preview_top == 0 && !g.strip.is_empty());
    let strip = g.strip;
    mouse_at(&mut a, MouseEventKind::Down(MouseButton::Left), strip.x, strip.y);
    assert!(a.scrollbar_drag.is_some(), "a press on the preview's thumb grabs it");
    a.dispatch(Command::FocusPane(1));
    mouse_at(&mut a, MouseEventKind::Drag(MouseButton::Left), strip.x, strip.bottom() - 1);
    mouse_at(&mut a, MouseEventKind::Up(MouseButton::Left), strip.x, strip.bottom() - 1);
    draw(&mut a, 80, 20);
    assert!(a.layout.panes[0].preview_top > 0, "the preview follows its thumb");
    assert_eq!(a.session.state().shown(1).unwrap().top_line(), 0, "the focused pane's text stays");
}
