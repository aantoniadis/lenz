//! Terminal Change Set search adapter. The shared model owns jobs, cancellation and navigation.
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use lenz::find::FindOption;
use lenz::{
    changeset::ChangeSet,
    find::{Ended, Hit},
    search_session::{ChangeSetSearch, PanelRow, SearchCommand},
};
use ratatui::{
    Frame,
    layout::Rect,
    style::Modifier,
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph},
};
use std::sync::Arc;

pub struct Search {
    pub model: ChangeSetSearch,
    pub closed: bool,
    pub focused: bool,
    set: Arc<ChangeSet>,
    hit: Option<Hit>,
    size: Rect,
    pub(crate) desired_height:Option<f32>,
    pub(crate) resizer:crate::panes::Resizer,
}
impl Search {
    /// The query prompt: the options on, the query, then the reason a bad one cannot search, and the keys.
    pub(crate) fn prompt_text(&self) -> String {
        let changed = if self.model.changed_only() { "on" } else { "off" };
        let error = self.model.query_error().map_or(String::new(), |e| format!(" · {e}"));
        format!(
            "Find in Change Set{}: {}▏{error}\nAlt-O changed lines only: {changed} · Alt-C/W/R case, word, regex · Enter searches · Esc closes",
            crate::render::option_marks(self.model.options()),
            safe(self.model.query())
        )
    }

    pub fn open(set: Arc<ChangeSet>, changed_only: bool, options: lenz::find::FindOptions) -> Self {
        Self {
            model: ChangeSetSearch::new(String::new(), changed_only, options),
            closed: false,
            focused: true,
            set,
            hit: None,
            size: Rect::new(0, 0, 80, 24),desired_height:None,resizer:Default::default(),
        }
    }
    pub fn poll(&mut self) -> bool {
        self.model.poll()
    }
    pub fn busy(&self) -> bool {
        self.model.busy()
    }
    pub fn take_hit(&mut self) -> Option<Hit> {
        self.hit.take()
    }
    pub fn edit(&mut self) {
        self.model.execute(SearchCommand::Edit, &self.set, 6.0);
        self.focused = true;
    }
    pub fn area(&self, area: Rect) -> Rect {
        if self.model.editing() || self.closed {
            return area;
        }
        let height = self.panel(area).height;
        Rect::new(
            area.x,
            area.y,
            area.width,
            area.height.saturating_sub(height),
        )
    }
    fn panel(&self, area: Rect) -> Rect {
        let height = crate::panes::size(area.height,self.desired_height.unwrap_or(f32::from((area.height / 3).max(3))),3.0,4.0);
        Rect::new(
            area.x,
            area.bottom().saturating_sub(height),
            area.width,
            height,
        )
    }
    pub fn resize_input(&mut self,event:&Event,blocked:bool)->bool {
        match self.resizer.handle(event,blocked||self.model.editing()||self.closed) {
            crate::panes::Resize::None=>false,crate::panes::Resize::Captured=>true,
            crate::panes::Resize::Move(_,top)=>{self.desired_height=Some((f32::from(self.size.height)-top).max(0.0));true}
        }
    }
    fn execute(&mut self, command: SearchCommand) {
        if let Some(hit) = self.model.execute(
            command,
            &self.set,
            self.panel(self.size).height.saturating_sub(2) as f32,
        ) {
            self.hit = Some(hit);
        }
    }
    fn type_text(&mut self, text: &str) {
        let mut query = self.model.query().to_string();
        for character in text.chars().filter(|character| !character.is_control()) {
            if query.len() >= 4096 {
                break;
            }
            query.push(character);
        }
        self.execute(SearchCommand::Query(query));
    }
    pub fn handle(&mut self, event: Event) -> bool {
        match event {
            Event::Resize(width, height) => {
                self.size = Rect::new(0, 0, width, height);
                false
            }
            Event::Paste(text) if self.model.editing() => {
                self.type_text(&text);
                true
            }
            Event::Mouse(mouse)
                if !self.model.editing()
                    && self
                        .panel(self.size)
                        .contains((mouse.column, mouse.row).into()) =>
            {
                match mouse.kind {
                    MouseEventKind::ScrollUp => self
                        .model
                        .scroll_by(-3.0, self.panel(self.size).height.saturating_sub(2) as f32),
                    MouseEventKind::ScrollDown => self
                        .model
                        .scroll_by(3.0, self.panel(self.size).height.saturating_sub(2) as f32),
                    MouseEventKind::Down(MouseButton::Left) => {
                        self.focused = true;
                        let row = mouse.row.saturating_sub(self.panel(self.size).y + 1) as usize
                            + self.model.scroll() as usize;
                        if let Some(PanelRow::Hit(index)) = self.model.rows().get(row).copied() {
                            self.execute(SearchCommand::Select(index));
                        }
                    }
                    _ => return false,
                }
                true
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                let alt = key.modifiers.contains(KeyModifiers::ALT);
                let shift = key.modifiers.contains(KeyModifiers::SHIFT);
                if key.code == KeyCode::Esc {
                    self.execute(SearchCommand::Cancel);
                    self.closed = true;
                    return true;
                }
                if key.code == KeyCode::F(3) && !self.model.editing() {
                    self.execute(SearchCommand::Step(!shift));
                    return true;
                }
                if self.model.editing() {
                    match key.code {
                        KeyCode::Enter => self.execute(SearchCommand::Start),
                        KeyCode::Backspace => {
                            let mut query = self.model.query().to_string();
                            query.pop();
                            self.execute(SearchCommand::Query(query));
                        }
                        KeyCode::Char('o') if alt => self.execute(SearchCommand::ToggleChangedOnly),
                        KeyCode::Char(c @ ('c' | 'w' | 'r')) if alt => self.model.toggle_option(match c { 'c' => FindOption::Case, 'w' => FindOption::Word, _ => FindOption::Regex }),
                        KeyCode::Char(character) if !ctrl && !alt => {
                            self.type_text(&character.to_string())
                        }
                        _ => return false,
                    }
                    return true;
                }
                if key.code == KeyCode::Tab {
                    self.focused = !self.focused;
                    return true;
                }
                if !self.focused {
                    return false;
                }
                match key.code {
                    KeyCode::Left | KeyCode::Right if !key.modifiers.contains(KeyModifiers::ALT | KeyModifiers::SHIFT) => {},
                    KeyCode::Home => self.model.scroll_by(f32::NEG_INFINITY, self.panel(self.size).height.saturating_sub(2) as f32),
                    KeyCode::End => self.model.scroll_by(f32::INFINITY, self.panel(self.size).height.saturating_sub(2) as f32),
                    KeyCode::Up => self.execute(SearchCommand::Step(false)),
                    KeyCode::Down => self.execute(SearchCommand::Step(true)),
                    KeyCode::Enter => {
                        self.execute(SearchCommand::Select(self.model.at().unwrap_or(0)))
                    }
                    KeyCode::PageUp => self.model.scroll_by(
                        -(self.panel(self.size).height as f32),
                        self.panel(self.size).height.saturating_sub(2) as f32,
                    ),
                    KeyCode::PageDown => self.model.scroll_by(
                        self.panel(self.size).height as f32,
                        self.panel(self.size).height.saturating_sub(2) as f32,
                    ),
                    KeyCode::Char('/') => self.edit(),
                    KeyCode::Char('o') if alt => {
                        self.execute(SearchCommand::ToggleChangedOnly);
                        self.execute(SearchCommand::Start);
                    }
                    _ => return false,
                }
                true
            }
            _ => false,
        }
    }
    #[cfg(test)]
    pub fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();
        self.render_in(frame, area,crate::chrome::initial_palette());
    }
    pub fn render_in(&mut self, frame: &mut Frame, area: Rect,palette:&lenz::theme::Palette) {
        self.size = area;
        self.resizer.grips.clear();
        if !self.model.editing()&&!self.closed {let panel=self.panel(area);if panel.height>0 {self.resizer.grips.push(crate::panes::Grip::horizontal(Rect::new(panel.x,panel.y,panel.width,1),0,area.y));}}
        if self.closed {
            return;
        }
        if self.model.editing() {
            let popup = Rect::new(
                area.x + 1,
                area.y + 1,
                area.width.saturating_sub(2),
                4.min(area.height.saturating_sub(2)),
            );
            let text = self.prompt_text();
            frame.render_widget(Clear, popup);
            frame.render_widget(
                Paragraph::new(text).block(Block::default().borders(Borders::ALL)),
                popup,
            );
            return;
        }
        let panel = self.panel(area);
        let end = match self.model.ended() {
            None => " · searching…".into(),
            Some(Ended::Done) => String::new(),
            Some(Ended::Capped) => format!(" · stopped at {} matches", lenz::find::MAX_HITS),
        };
        let title = format!(
            "Find in Change Set · {} matches{}{} · {}",
            self.model.hits().len(),
            if self.model.changed_only() {
                " · changed lines only"
            } else {
                ""
            },
            end,
            safe(self.model.query())
        );
        let lines: Vec<_> = self
            .model
            .rows()
            .iter()
            .skip(self.model.scroll() as usize)
            .take(panel.height.saturating_sub(2) as usize)
            .map(|row| match row {
                PanelRow::File(index, count) => Line::styled(
                    format!("▾ {} ({count})", safe(&self.set.comparisons[*index].path)),
                    crate::chrome::style_with(palette).fg(crate::chrome::color(palette.accent)),
                ),
                PanelRow::Hit(index) => {
                    let hit = &self.model.hits()[*index];
                    let side = if hit.side == lenz::view::Side::Left {
                        "L"
                    } else {
                        "R"
                    };
                    let selected = self.model.at() == Some(*index);
                    Line::styled(
                        format!(
                            "  {side} {:>5}: {}",
                            hit.line + 1,
                            safe(hit.preview.trim_start())
                        ),
                        if selected {
                            crate::chrome::style_with(palette)
                                .bg(crate::chrome::color(palette.selected_bg))
                                .add_modifier(Modifier::BOLD)
                        } else {
                            crate::chrome::style_with(palette)
                        },
                    )
                }
            })
            .collect();
        frame.render_widget(Clear, panel);
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .title_bottom("F3/Shift-F3 results · Tab comparison · / edit · Esc closes"),
            ),
            panel,
        );
        if let Some(notice) = self.model.notice() {
            frame.render_widget(
                Paragraph::new(notice).style(crate::chrome::style_with(palette).fg(crate::chrome::color(palette.accent))),
                Rect::new(
                    panel.x + 1,
                    panel.bottom().saturating_sub(1),
                    panel.width.saturating_sub(2),
                    1,
                ),
            );
        }
        self.resizer.draw(frame.buffer_mut(),palette);
    }
}
fn safe(text: &str) -> String {
    text.chars()
        .map(|character| {
            if lenz::document::hides_text(character) {
                '�'
            } else {
                character
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;
    use ratatui::{Terminal, backend::TestBackend};
    fn fixture() -> (tempfile::TempDir, Search) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("left"), "needle common\nneedle old\n").unwrap();
        std::fs::write(dir.path().join("right"), "needle common\nneedle new\n").unwrap();
        let set = ChangeSet::load(
            &lenz::changeset::Spec::Plain {
                left: dir.path().join("left"),
                right: dir.path().join("right"),
            },
            dir.path(),
        )
        .unwrap();
        (dir, Search::open(Arc::new(set), false, Default::default()))
    }
    fn key(search: &mut Search, key: KeyCode, modifiers: KeyModifiers) {
        assert!(search.handle(Event::Key(KeyEvent::new(key, modifiers))));
    }
    fn ready(search: &mut Search) {
        let started = std::time::Instant::now();
        while search.busy() {
            search.poll();
            assert!(started.elapsed().as_secs() < 10);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    fn screen(search: &mut Search) -> String {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| search.render(frame)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<Vec<_>>()
            .join("")
    }
    #[test]
    fn typed_query_streams_grouped_results_and_real_keys_open_side_and_line() {
        let (_dir, mut search) = fixture();
        for character in "needle".chars() {
            key(&mut search, KeyCode::Char(character), KeyModifiers::NONE);
        }
        key(&mut search, KeyCode::Enter, KeyModifiers::NONE);
        ready(&mut search);
        assert!(screen(&mut search).contains("4 matches"));
        key(&mut search, KeyCode::F(3), KeyModifiers::NONE);
        let hit = search.take_hit().unwrap();
        assert_eq!(
            (hit.index, hit.side, hit.line, hit.range),
            (0, lenz::view::Side::Left, 0, 0..6)
        );
        key(&mut search, KeyCode::Tab, KeyModifiers::NONE);
        assert!(!search.focused);
        assert!(!search.handle(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))));
        key(&mut search, KeyCode::Esc, KeyModifiers::NONE);
        assert!(search.closed && !search.busy());
    }
    #[test]
    fn changed_only_and_query_editing_share_gui_rules() {
        let (_dir, mut search) = fixture();
        search.handle(Event::Paste("needle".into()));
        key(&mut search, KeyCode::Char('o'), KeyModifiers::ALT);
        assert!(screen(&mut search).contains("only: on"));
        key(&mut search, KeyCode::Enter, KeyModifiers::NONE);
        ready(&mut search);
        assert_eq!(search.model.hits().len(), 2);
        assert!(search.model.hits().iter().all(|hit| hit.line == 1));
        key(&mut search, KeyCode::Char('/'), KeyModifiers::NONE);
        assert!(search.model.editing());
        assert!(screen(&mut search).contains("needle"));
        assert_eq!(safe("path\x1b\n"), "path��");
    }
}
