use super::*;

#[test]
fn losing_focus_cancels_annotation_drag() {
    let (_repo, mut a, _) = annotated_app();
    let pane = &a.layout.panes[0];
    let (x, y) = (pane.gutter_x + pane.ann, pane.text.y + 1);
    a.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: x, row: y, modifiers: KeyModifiers::NONE }));
    a.handle(Event::FocusLost);
    a.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Drag(MouseButton::Left), column: x + 10, row: y, modifiers: KeyModifiers::NONE }));
    assert_eq!(a.session.state().layout().annotation_cols(), 23);
}

#[test]
fn dragging_the_annotation_edge_changes_width_without_moving_the_caret() {
    let (_repo, mut a, _) = annotated_app();
    let pane = &a.layout.panes[0];
    let (x, y) = (pane.gutter_x + pane.ann, pane.text.y + 1);
    let selected = a.session.state().selection();
    for (kind, column) in [(MouseEventKind::Down(MouseButton::Left), x), (MouseEventKind::Drag(MouseButton::Left), x + 10), (MouseEventKind::Up(MouseButton::Left), x + 10)] {
        a.handle(Event::Mouse(MouseEvent { kind, column, row: y, modifiers: KeyModifiers::NONE }));
    }
    assert_eq!(a.session.state().layout().annotation_cols(), 33);
    assert_eq!(a.session.state().selection(), selected);
    assert!(a.session.state().commit_popup().is_none());
}
