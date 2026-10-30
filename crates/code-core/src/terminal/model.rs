use std::sync::{Arc, Mutex};

use alacritty_terminal::{
    Term,
    event::{Event, EventListener, WindowSize},
    grid::{Dimensions, Scroll},
    index::{Boundary, Column, Line, Point, Side},
    selection::{Selection, SelectionType},
    term::{Config, Osc52, TermMode, cell::Flags, search::Match},
    vte::ansi::{Color, NamedColor, Processor, Rgb},
};

use super::{Cell, Frame, MouseKind};

const HISTORY_BYTES: usize = 16 * 1024 * 1024;
const CELL_BUDGET: usize = 256;
const MAX_CONTROL: usize = 4096;
const MAX_COMBINING: usize = 8;

#[derive(Clone, Copy)]
struct Size {
    columns: usize,
    rows: usize,
}
impl Dimensions for Size {
    fn columns(&self) -> usize {
        self.columns
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn total_lines(&self) -> usize {
        self.rows
    }
}

#[derive(Clone)]
struct Listener {
    responses: Arc<Mutex<Vec<u8>>>,
    size: Arc<Mutex<Size>>,
    colors: (u32, u32),
    queries: Arc<Mutex<Vec<Event>>>,
}
impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let response = match event {
            Event::PtyWrite(text) => text,
            event @ Event::ColorRequest(..) => {
                self.queries
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(event);
                return;
            }
            Event::TextAreaSizeRequest(format) => {
                let size = *self
                    .size
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                format(WindowSize {
                    num_lines: size.rows as u16,
                    num_cols: size.columns as u16,
                    cell_width: 0,
                    cell_height: 0,
                })
            }
            // Child output cannot interact with the host clipboard, title or programs.
            _ => return,
        };
        let mut responses = self
            .responses
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if responses.len() + response.len() <= 64 * 1024 {
            responses.extend_from_slice(response.as_bytes());
        }
    }
}

/// Limit heap-bearing string controls before handing bytes to the maintained parser.
#[derive(Default)]
struct Controls {
    escape: bool,
    string: bool,
    string_escape: bool,
    overflow: bool,
    buffer: Vec<u8>,
}
impl Controls {
    fn reset(&mut self) {
        self.string = false;
        self.string_escape = false;
        self.overflow = false;
        self.buffer.clear();
    }
    fn finish(&mut self, output: &mut Vec<u8>) {
        // Only palette controls are useful OSCs here; titles, links and clipboard are host-owned.
        let allowed = !self.buffer.starts_with(b"\x1b]")
            || self.buffer.get(2..).is_some_and(|payload| {
                let command = payload
                    .split(|byte| matches!(byte, b';' | 7 | 27))
                    .next()
                    .unwrap_or_default();
                matches!(
                    command,
                    b"4" | b"10" | b"11" | b"12" | b"104" | b"110" | b"111" | b"112"
                )
            });
        if !self.overflow && allowed {
            output.extend_from_slice(&self.buffer);
        }
        self.reset();
    }
    fn append(&mut self, byte: u8) {
        if !self.overflow {
            if self.buffer.len() == MAX_CONTROL {
                self.buffer.clear();
                self.overflow = true;
            } else {
                self.buffer.push(byte);
            }
        }
    }
    fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        for &byte in input {
            if self.string {
                if matches!(byte, 24 | 26) {
                    self.reset();
                    output.push(byte);
                    continue;
                }
                if self.string_escape {
                    if byte == b'\\' {
                        self.append(27);
                        self.append(byte);
                        self.finish(output);
                        continue;
                    }
                    // ESC aborts the string and starts a new escape, even when it is not ST.
                    self.reset();
                    self.escape = true;
                } else if byte == 27 {
                    self.string_escape = true;
                    continue;
                } else {
                    self.append(byte);
                    if byte == 7 {
                        self.finish(output);
                    }
                    continue;
                }
            }
            if self.escape {
                self.escape = false;
                if matches!(byte, b']' | b'P' | b'_' | b'^' | b'X') {
                    self.string = true;
                    self.buffer.extend_from_slice(&[27, byte]);
                } else if byte == 27 {
                    self.escape = true;
                } else {
                    output.extend_from_slice(&[27, byte]);
                }
            } else if byte == 27 {
                self.escape = true;
            } else {
                output.push(byte);
            }
        }
    }
}

pub(super) struct Model {
    term: Term<Listener>,
    parser: Processor,
    controls: Controls,
    listener: Listener,
    filtered: Vec<u8>,
    search_chars: Vec<char>,
    search_prefix: Vec<usize>,
    search_points: Vec<Point>,
    search_query: String,
    search_match: Option<Match>,
    search_result: Option<bool>,
    #[cfg(test)]
    search_runs: usize,
}
impl Model {
    pub(super) fn new(colors: (u32, u32)) -> Self {
        let size = Size {
            columns: 80,
            rows: 24,
        };
        let listener = Listener {
            responses: Arc::default(),
            size: Arc::new(Mutex::new(size)),
            colors,
            queries: Arc::default(),
        };
        let term = Term::new(Self::config(size.columns), &size, listener.clone());
        Self {
            term,
            parser: Processor::new(),
            controls: Controls::default(),
            listener,
            filtered: Vec::with_capacity(4096),
            search_chars: Vec::new(),
            search_prefix: Vec::new(),
            search_points: Vec::new(),
            search_query: String::new(),
            search_match: None,
            search_result: None,
            #[cfg(test)]
            search_runs: 0,
        }
    }
    fn config(columns: usize) -> Config {
        Config {
            scrolling_history: (HISTORY_BYTES / CELL_BUDGET / columns).min(10_000),
            kitty_keyboard: false,
            osc52: Osc52::Disabled,
            ..Config::default()
        }
    }
    fn invalidate_search(&mut self) {
        self.search_match = None;
        self.search_result = None;
    }
    pub(super) fn search(&mut self, query: &str, backwards: bool, advance: bool) {
        if query != self.search_query {
            self.search_query = query.to_owned();
            self.invalidate_search();
            self.search_chars = query.chars().collect();
            self.search_prefix = vec![0; self.search_chars.len()];
            self.search_points = vec![Point::new(Line(0),Column(0)); self.search_chars.len()];
            for at in 1..self.search_chars.len() {
                let mut prefix = self.search_prefix[at-1];
                while prefix > 0 && self.search_chars[at] != self.search_chars[prefix] { prefix = self.search_prefix[prefix-1]; }
                if self.search_chars[at] == self.search_chars[prefix] { prefix += 1; }
                self.search_prefix[at] = prefix;
            }
        }
        if self.search_chars.is_empty() { self.invalidate_search(); return; }
        let origin = if advance && let Some(found) = &self.search_match {
            if backwards { found.start().sub(&self.term, Boundary::None, 1) }
            else { found.start().add(&self.term, Boundary::None, 1) }
        } else { Point::new(Line(-(self.term.grid().display_offset() as i32)), Column(0)) };
        #[cfg(test)]
        { self.search_runs += 1; }
        let grid = self.term.grid();
        let (mut matched,mut consumed) = (0,0);
        let (mut found,mut wrapped) = (None,None);
        // Stream exact displayed scalars: Alacritty's regex iterator omits per-cell combining marks.
        'rows: for line in -(grid.history_size() as i32)..grid.screen_lines() as i32 {
            for column in 0..grid.columns() {
                let point = Point::new(Line(line),Column(column));
                let cell = &grid[point];
                if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) { continue; }
                if cell.flags.contains(Flags::HIDDEN) { matched = 0; continue; }
                for c in std::iter::once(cell.c).chain(cell.zerowidth().unwrap_or_default().iter().copied()) {
                    self.search_points[consumed % self.search_chars.len()] = point;
                    consumed += 1;
                    while matched > 0 && self.search_chars[matched] != c { matched = self.search_prefix[matched-1]; }
                    if self.search_chars[matched] == c { matched += 1; }
                    if matched != self.search_chars.len() { continue; }
                    let start = self.search_points[(consumed-self.search_chars.len()) % self.search_chars.len()];
                    let end = if cell.flags.contains(Flags::WIDE_CHAR) { point.add(&self.term,Boundary::None,1) } else { point };
                    let candidate = start..=end;
                    if backwards {
                        if start <= origin { found = Some(candidate); } else { wrapped = Some(candidate); }
                    } else if start >= origin { found = Some(candidate); break 'rows; }
                    else if wrapped.is_none() { wrapped = Some(candidate); }
                    matched = self.search_prefix[matched-1];
                }
            }
            if !grid[Line(line)][Column(grid.columns()-1)].flags.contains(Flags::WRAPLINE) { matched = 0; }
        }
        self.search_match = found.or(wrapped);
        self.search_result = Some(self.search_match.is_some());
        if let Some(found) = &self.search_match { self.term.scroll_to_point(*found.start()); }
    }
    pub(super) fn advance(&mut self, bytes: &[u8]) {
        if !bytes.is_empty() { self.invalidate_search(); }
        for chunk in bytes.chunks(4096) {
            self.filtered.clear();
            self.controls.filter(chunk, &mut self.filtered);
            let mut start = 0;
            for index in 0..self.filtered.len() {
                let byte = self.filtered[index];
                // Stop after each complete non-ASCII scalar, not each of its bytes: a combining mark is capped
                // before the next one, and box-drawing or CJK output does not pay the stop per byte.
                let scalar_end = byte >= 128
                    && self.filtered.get(index + 1).is_none_or(|next| next & 0xc0 != 0x80);
                if !scalar_end
                    && byte != 7
                    && !(byte == b'\\' && index > 0 && self.filtered[index - 1] == 27)
                {
                    continue;
                }
                self.parser
                    .advance(&mut self.term, &self.filtered[start..=index]);
                self.parser.stop_sync(&mut self.term);
                self.answer_colors();
                if byte >= 128 {
                    self.cap_combining();
                }
                start = index + 1;
            }
            self.parser.advance(&mut self.term, &self.filtered[start..]);
            // Do not retain synchronized output waiting for a child-controlled timeout.
            self.parser.stop_sync(&mut self.term);
            self.answer_colors();
        }
    }
    fn answer_colors(&self) {
        let queries = std::mem::take(
            &mut *self
                .listener
                .queries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for event in queries {
            if let Event::ColorRequest(index, format) = event {
                let color = self.term.colors()[index]
                    .unwrap_or_else(|| rgb(palette(index, self.listener.colors)));
                self.listener.send_event(Event::PtyWrite(format(color)));
            }
        }
    }
    fn cap_combining(&mut self) {
        let grid = self.term.grid_mut();
        let point = grid.cursor.point;
        // A combining mark attaches to this cell or its preceding wide glyph.
        for column in point.column.0.saturating_sub(2)..=point.column.0 {
            let cell = &mut grid[Point::new(point.line, Column(column))];
            if let Some(extra) = cell.zerowidth().filter(|chars| chars.len() > MAX_COMBINING) {
                let mut kept = ['\0'; MAX_COMBINING];
                kept.copy_from_slice(&extra[..MAX_COMBINING]);
                let underline = cell.underline_color();
                cell.extra = None;
                cell.set_underline_color(underline);
                for c in kept {
                    cell.push_zerowidth(c);
                }
            }
        }
    }
    pub(super) fn resize(&mut self, columns: usize, rows: usize) -> (usize, usize) {
        let size = Size {
            columns: columns.clamp(2, 512),
            rows: rows.clamp(1, 256),
        };
        if self.term.grid().columns() == size.columns && self.term.grid().screen_lines() == size.rows {
            return (size.columns,size.rows);
        }
        self.invalidate_search();
        *self
            .listener
            .size
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = size;
        // Set both active and inactive grid history budgets through Term's options.
        self.term.set_options(Self::config(size.columns));
        self.term.resize(size);
        self.term.selection = None;
        (size.columns, size.rows)
    }
    #[cfg(test)]
    pub(super) fn bracketed(&self) -> bool {
        self.term.mode().contains(TermMode::BRACKETED_PASTE)
    }
    pub(super) fn app_cursor(&self) -> bool {
        self.term.mode().contains(TermMode::APP_CURSOR)
    }
    pub(super) fn modes(&self) -> u32 {
        self.term.mode().bits()
    }
    pub(super) fn scroll(&mut self, lines: i32) {
        self.term
            .scroll_display(Scroll::Delta(lines.clamp(-10_000, 10_000)));
    }
    pub(super) fn scroll_to(&mut self, top: bool) {
        self.term
            .scroll_display(if top { Scroll::Top } else { Scroll::Bottom });
    }
    pub(super) fn select(&mut self, row: usize, column: usize, extend: bool) {
        let grid = self.term.grid();
        let point = Point::new(
            Line(row.min(grid.screen_lines() - 1) as i32 - grid.display_offset() as i32),
            Column(column.min(grid.columns() - 1)),
        );
        if extend && let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, Side::Right);
        } else {
            self.term.selection = Some(Selection::new(SelectionType::Simple, point, Side::Left));
        }
    }
    pub(super) fn copy(&self) -> Option<String> {
        self.term.selection_to_string()
    }
    pub(super) fn responses(&self) -> Vec<u8> {
        std::mem::take(
            &mut *self
                .listener
                .responses
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }
    pub(super) fn mouse(
        &mut self,
        row: usize,
        column: usize,
        kind: MouseKind,
        ctrl: bool,
        alt: bool,
        shift: bool,
    ) -> Vec<u8> {
        let valid = match kind {
            MouseKind::Press(button) => matches!(button, 0..=2 | 64 | 65),
            MouseKind::Release(button) => button <= 2,
            MouseKind::Move(button) => button <= 3,
        };
        if !valid {
            return Vec::new();
        }
        let mode = *self.term.mode();
        if shift || !mode.intersects(TermMode::MOUSE_MODE) {
            if !shift && mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
                match kind {
                    MouseKind::Press(64) => {
                        return if self.app_cursor() {
                            b"\x1bOA"
                        } else {
                            b"\x1b[A"
                        }
                        .to_vec();
                    }
                    MouseKind::Press(65) => {
                        return if self.app_cursor() {
                            b"\x1bOB"
                        } else {
                            b"\x1b[B"
                        }
                        .to_vec();
                    }
                    _ => {}
                }
            }
            match kind {
                MouseKind::Press(64) => self.scroll(3),
                MouseKind::Press(65) => self.scroll(-3),
                MouseKind::Press(0) => self.select(row, column, false),
                MouseKind::Move(0) | MouseKind::Release(0) => self.select(row, column, true),
                _ => {}
            }
            return Vec::new();
        }
        let (button, release, motion) = match kind {
            MouseKind::Press(button) => (button, false, false),
            MouseKind::Release(button) => (button, true, false),
            MouseKind::Move(button) => (button, false, true),
        };
        if motion
            && (!mode.intersects(TermMode::MOUSE_MOTION | TermMode::MOUSE_DRAG)
                || (button == 3 && !mode.contains(TermMode::MOUSE_MOTION)))
        {
            return Vec::new();
        }
        if release && button >= 64 {
            return Vec::new();
        }
        let modifiers = u8::from(ctrl) * 16 + u8::from(alt) * 8 + u8::from(shift) * 4;
        let button = button.saturating_add(if motion { 32 } else { 0 }) + modifiers;
        let row = row.min(self.term.grid().screen_lines() - 1) + 1;
        let column = column.min(self.term.grid().columns() - 1) + 1;
        if mode.contains(TermMode::SGR_MOUSE) {
            return format!(
                "\x1b[<{button};{column};{row}{}",
                if release { 'm' } else { 'M' }
            )
            .into_bytes();
        }
        let button = if release { 3 + modifiers } else { button };
        if mode.contains(TermMode::UTF8_MOUSE) {
            let mut bytes = vec![27, b'[', b'M', button + 32];
            for value in [column + 32, row + 32] {
                if let Some(c) = char::from_u32(value as u32) {
                    bytes.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
                }
            }
            bytes
        } else if row < 224 && column < 224 {
            vec![
                27,
                b'[',
                b'M',
                button + 32,
                column as u8 + 32,
                row as u8 + 32,
            ]
        } else {
            Vec::new()
        }
    }
    pub(super) fn frame(&self) -> Arc<Frame> {
        let grid = self.term.grid();
        let content = self.term.renderable_content();
        let mut cells = Vec::with_capacity(grid.columns() * grid.screen_lines());
        for indexed in content.display_iter {
            let cell = indexed.cell;
            let mut text = String::new();
            if !cell.flags.contains(Flags::HIDDEN) {
                text.push(cell.c);
                if let Some(chars) = cell.zerowidth() {
                    text.extend(chars.iter().take(MAX_COMBINING));
                }
            }
            let resolve = |color| match color {
                Color::Spec(rgb) => Some(pack(rgb)),
                Color::Indexed(index) => Some(
                    content.colors[index as usize]
                        .map(pack)
                        .unwrap_or_else(|| palette(index as usize, self.listener.colors)),
                ),
                Color::Named(name) => content.colors[name].map(pack).or_else(|| {
                    (!matches!(name, NamedColor::Foreground | NamedColor::Background))
                        .then(|| palette(name as usize, self.listener.colors))
                }),
            };
            let (mut fg, mut bg) = (resolve(cell.fg), resolve(cell.bg));
            if cell.flags.contains(Flags::INVERSE) {
                (fg, bg) = (
                    Some(bg.unwrap_or(self.listener.colors.1)),
                    Some(fg.unwrap_or(self.listener.colors.0)),
                );
            }
            cells.push(Cell {
                text,
                fg,
                bg,
                bold: cell.flags.contains(Flags::BOLD),
                italic: cell.flags.contains(Flags::ITALIC),
                underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
                strikeout: cell.flags.contains(Flags::STRIKEOUT),
                dim: cell.flags.contains(Flags::DIM),
                search_match: self.search_match.as_ref().is_some_and(|found| found.contains(&indexed.point)),
                selected: content
                    .selection
                    .is_some_and(|selection| selection.contains(indexed.point)),
                wide_spacer: cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER),
            });
        }
        let point = content.cursor.point;
        let cursor_row = point.line.0 + content.display_offset as i32;
        let cursor = (content.mode.contains(TermMode::SHOW_CURSOR)
            && cursor_row >= 0
            && cursor_row < grid.screen_lines() as i32)
            .then_some((cursor_row.max(0) as usize, point.column.0));
        Arc::new(Frame {
            columns: grid.columns(),
            rows: grid.screen_lines(),
            cells,
            cursor,
            scrollback: grid.history_size(),
            search: (!self.search_query.is_empty()).then(|| (self.search_query.clone(), self.search_result)),
        })
    }
}
fn pack(color: Rgb) -> u32 {
    (color.r as u32) << 16 | (color.g as u32) << 8 | color.b as u32
}
fn rgb(color: u32) -> Rgb {
    Rgb {
        r: (color >> 16) as u8,
        g: (color >> 8) as u8,
        b: color as u8,
    }
}
fn palette(index: usize, defaults: (u32, u32)) -> u32 {
    const ANSI: [u32; 16] = [
        0, 0xcd0000, 0x00cd00, 0xcdcd00, 0x0000ee, 0xcd00cd, 0x00cdcd, 0xe5e5e5, 0x7f7f7f,
        0xff0000, 0x00ff00, 0xffff00, 0x5c5cff, 0xff00ff, 0x00ffff, 0xffffff,
    ];
    match index {
        0..=15 => ANSI[index],
        16..=231 => {
            let n = index - 16;
            let level = |n| if n == 0 { 0 } else { 55 + n * 40 };
            (level(n / 36) << 16 | level(n / 6 % 6) << 8 | level(n % 6)) as u32
        }
        232..=255 => {
            let n = (8 + (index - 232) * 10) as u32;
            n * 0x010101
        }
        257 => defaults.1,
        259..=266 => (ANSI[index - 259] & 0xfefefe) >> 1,
        _ => defaults.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_model_scalars_split_across_reads_and_combining_marks_stay_bounded() {
        let mut model = Model::new((0xffffff, 0));
        model.resize(8, 2);
        let text = "│界😀".as_bytes();
        for byte in text { model.advance(std::slice::from_ref(byte)); }
        let frame = model.frame();
        let shown: Vec<_> = frame.cells.iter().filter(|c| !c.wide_spacer).take(3).map(|c| c.text.as_str()).collect();
        assert_eq!(shown, ["│", "界", "😀"]);
        model.advance(format!("\r\ne{}", "\u{301}".repeat(64)).as_bytes());
        let marks = model.frame().cells[8].text.chars().count();
        assert_eq!(marks, 1 + MAX_COMBINING);
    }

    #[test]
    fn terminal_model_find_literal_history_wrap_and_invalidation() {
        let mut model = Model::new((0xffffff, 0));
        model.resize(12, 2);
        model.advance(b"first [a]\r\nother\r\nlast [a]\r\nend");
        model.search("[a]", false, false);
        assert!(model.frame().cells.iter().any(|c| c.search_match));
        let first = model.search_match.clone().unwrap();
        model.search("[a]", false, true);
        let second = model.search_match.clone().unwrap();
        assert_ne!(first, second);
        model.search("[a]", false, true);
        assert_eq!(model.search_match.as_ref(), Some(&first));
        model.search("[a]", true, true);
        assert_eq!(model.search_match.as_ref(), Some(&second));
        model.search("界", false, false);
        assert_eq!(model.search_result, Some(false));
        model.advance("\r\n界.*".as_bytes());
        model.search("界.*", false, false);
        assert_eq!(model.search_result, Some(true));
        assert_eq!(model.frame().cells.iter().filter(|c| c.search_match).count(), 4);
        let runs = model.search_runs;
        for _ in 0..30 { model.frame(); }
        model.advance(b" ");
        assert_eq!(model.search_runs, runs, "frame/output must not rescan history");
        model.search("missing", false, false);
        assert_eq!(model.frame().search.as_ref().unwrap().1, Some(false));
        model.search("[a]", false, false);
        model.advance(b"!");
        assert!(model.search_match.is_none(), "output cannot leave stale highlight coordinates");
        assert_eq!(model.frame().search.as_ref().unwrap().1, None);
        model.search("", false, false);
        assert!(model.frame().search.is_none());
    }

    #[test]
    fn terminal_model_find_combining_marks_soft_wrap_and_overlapping_literals() {
        let mut model = Model::new((0xffffff,0)); model.resize(6,3);
        model.advance("plain e\r\ne\u{301}x\r\nbanana".as_bytes());
        model.search("e\u{301}x",false,false);
        assert_eq!(model.search_result,Some(true));
        assert_eq!(model.frame().cells.iter().filter(|c|c.search_match).count(),2);
        model.search("\u{301}",false,false);
        assert_eq!(model.search_result,Some(true));
        assert_eq!(model.frame().cells.iter().filter(|c|c.search_match).count(),1);
        model.search("e",false,false);
        assert_eq!(model.search_result,Some(true),"base-prefix search remains literal");
        model.search("e\u{300}x",false,false);
        assert_eq!(model.search_result,Some(false));
        model.search("ex",false,false);
        assert_eq!(model.search_result,Some(false),"a mark must not be stripped from intervening text");
        model.search("ana",false,false);
        let first = model.search_match.clone().unwrap();
        model.search("ana",false,true);
        let second = model.search_match.clone().unwrap();
        assert_ne!(first,second,"next must visit overlapping literal occurrences");
        model.search("ana",true,true);
        assert_eq!(model.search_match,Some(first));
        model.advance("\r\n12345界.*".as_bytes());
        model.search("5界.*",false,false);
        assert_eq!(model.search_result,Some(true),"soft-wrapped wide glyph is contiguous text");
        model.advance("\r\n❤\u{fe0f}".as_bytes());
        model.search("❤\u{fe0f}",false,false);
        assert_eq!(model.search_result,Some(true));
        let selected = model.search_match.clone();
        model.resize(6,3);
        assert_eq!(model.search_match,selected,"duplicate adapter resize is not a content change");
        model.search("❤\u{fe0e}",false,false);
        assert_eq!(model.search_result,Some(false));
        model.search("e\u{301}xbanana",false,false);
        assert_eq!(model.search_result,Some(false),"a hard line break separates text");
    }

    #[test]
    #[ignore = "Feature-only 10k-history release search probe; emits raw timing and RSS samples"]
    fn terminal_model_find_release_probe() {
        fn rss_kib() -> u64 {
            std::fs::read_to_string("/proc/self/status").unwrap().lines()
                .find_map(|line|line.strip_prefix("VmRSS:").and_then(|v|v.split_whitespace().next()).and_then(|v|v.parse().ok())).unwrap()
        }
        let mut model = Model::new((0xffffff,0));
        model.resize(6,2);
        for _ in 0..10_020 { model.advance("界.*\r\n".as_bytes()); }
        assert_eq!(model.frame().scrollback,10_000);
        let rss_start = rss_kib();
        let mut samples = Vec::new();
        let started = std::time::Instant::now();
        for iteration in 0..120 {
            for (kind,query,backwards,advance,expected) in [
                ("literal_unicode","界.*",false,false,true),
                ("next","界.*",false,true,true),
                ("previous","界.*",true,true,true),
                ("absent","absent",false,false,false),
            ] {
                let start = std::time::Instant::now();
                model.search(query,backwards,advance);
                let ns = start.elapsed().as_nanos() as u64;
                assert_eq!(model.search_result,Some(expected));
                samples.push(serde_json::json!({"iteration":iteration,"kind":kind,"ns":ns}));
            }
            let searches = model.search_runs;
            for _ in 0..10 { model.frame(); }
            model.advance("界.*\r\n".as_bytes());
            assert_eq!(model.search_runs,searches);
            assert!(model.search_match.is_none());
            assert_eq!(model.frame().scrollback,10_000);
        }
        let rss_end = rss_kib();
        assert!(rss_end.saturating_sub(rss_start) < 16*1024);
        // Largest supported viewport plus retained history, with the per-cell mark limit filled.
        let mut maximum = Model::new((0xffffff,0)); maximum.resize(512,256);
        let cell = format!("x{}","\u{301}".repeat(MAX_COMBINING));
        maximum.advance(cell.repeat(512*(256+129)).as_bytes());
        assert_eq!(maximum.frame().scrollback,128);
        let max_rss_start = rss_kib();
        let prefix = format!("{}z",cell.repeat(60));
        assert!(prefix.len() <= 1024);
        let mut maximum_samples = Vec::new();
        for _ in 0..16 {
            for query in ["absent",prefix.as_str()] {
                let start = std::time::Instant::now(); maximum.search(query,false,false);
                maximum_samples.push(serde_json::json!({"query_bytes":query.len(),"ns":start.elapsed().as_nanos() as u64}));
                assert_eq!(maximum.search_result,Some(false));
            }
        }
        let max_rss_end = rss_kib();
        assert!(max_rss_end.saturating_sub(max_rss_start) < 16*1024);
        println!("{}",serde_json::json!({"schema":1,"probe":"terminal_find_model","history":10_000,"columns":6,"rows":2,"query_limit_bytes":1024,"elapsed_ns":started.elapsed().as_nanos() as u64,"rss_start_kib":rss_start,"rss_end_kib":rss_end,"samples":samples,"max_retention":{"columns":512,"rows":256,"history":128,"marks_per_cell":MAX_COMBINING,"rss_start_kib":max_rss_start,"rss_end_kib":max_rss_end,"samples":maximum_samples}}));
        model.search("",false,false);
        assert!(model.search_chars.is_empty());
    }

    #[test]
    fn terminal_model_unicode_modes_selection_and_queries() {
        let mut model = Model::new((0xffffff, 0));
        model.advance("a界e\u{301}\x1b[31mZ\x1b[0m\x1b[?2004h\x1b[?1h\x1b[6n".as_bytes());
        assert!(model.bracketed());
        assert!(model.app_cursor());
        let frame = model.frame();
        assert_eq!(frame.cells[1].text, "界");
        assert!(frame.cells[2].wide_spacer);
        assert_eq!(frame.cells[3].text, "e\u{301}");
        assert_eq!(frame.cells[4].fg, Some(0xcd0000));
        model.select(0, 0, false);
        model.select(0, 4, true);
        assert_eq!(model.copy().as_deref(), Some("a界e\u{301}Z"));
        assert_eq!(model.responses(), b"\x1b[1;6R");
    }

    #[test]
    fn terminal_model_bounds_string_controls_and_combining_storage() {
        let mut model = Model::new((0xffffff, 0));
        model.advance(
            format!(
                "\x1b]0;{}\x07a{}",
                "x".repeat(100_000),
                "\u{301}".repeat(100_000)
            )
            .as_bytes(),
        );
        let frame = model.frame();
        assert!(frame.cells[0].text.len() <= 1 + 8 * 4);
        assert_eq!(frame.cells[0].text.chars().next(), Some('a'));
        model.advance(b"\x1b[?1049hother\x1b[?1049l");
        assert_eq!(model.frame().cells[0].text.chars().next(), Some('a'));
    }
    #[test]
    fn terminal_model_controls_cancel_and_do_not_retain_host_title_or_clipboard() {
        let mut model = Model::new((0x123456, 0x654321));
        for cancel in [24, 26] {
            let mut bytes = format!("\x1b]0;{}", "x".repeat(MAX_CONTROL + 1)).into_bytes();
            bytes.push(cancel);
            bytes.extend_from_slice(b"VISIBLE\r\n");
            model.advance(&bytes);
        }
        model.advance(b"\x1b]0;host title\x07\x1b]52;c;c2VjcmV0\x07\x1b]52;c;?\x07\x1b[>1u\x1b[?u");
        let text: String = model
            .frame()
            .cells
            .iter()
            .map(|c| c.text.as_str())
            .collect();
        assert!(text.contains("VISIBLE"));
        assert!(!text.contains("host title"));
        assert!(model.responses().is_empty());
        assert!(
            !TermMode::from_bits_retain(model.modes())
                .intersects(TermMode::KITTY_KEYBOARD_PROTOCOL)
        );
    }

    #[test]
    fn terminal_model_dynamic_colors_mouse_and_tiny_resize() {
        let mut model = Model::new((0x123456, 0x654321));
        model.advance(
            b"\x1b]10;?\x07\x1b]4;1;rgb:12/34/56\x07\x1b]4;1;?\x07\x1b]10;rgb:11/22/33\x07X",
        );
        assert_eq!(
            String::from_utf8(model.responses()).unwrap(),
            "\x1b]10;rgb:1212/3434/5656\x07\x1b]4;1;rgb:1212/3434/5656\x07"
        );
        assert_eq!(model.frame().cells[0].fg, Some(0x112233));
        model.advance(b"\x1b[?1002h\x1b[?1006h");
        assert_eq!(
            model.mouse(2, 3, MouseKind::Press(0), true, false, false),
            b"\x1b[<16;4;3M"
        );
        assert_eq!(
            model.mouse(2, 3, MouseKind::Release(0), false, false, false),
            b"\x1b[<0;4;3m"
        );
        assert_eq!(
            model.mouse(2, 3, MouseKind::Press(64), false, false, false),
            b"\x1b[<64;4;3M"
        );
        assert!(
            model
                .mouse(2, 3, MouseKind::Move(3), false, false, false)
                .is_empty()
        );
        assert!(
            model
                .mouse(2, 3, MouseKind::Press(255), true, true, true)
                .is_empty()
        );
        assert_eq!(model.resize(0, 0), (2, 1));
        model.select(usize::MAX, usize::MAX, false);
        model.select(usize::MAX, usize::MAX, true);
        let frame = model.frame();
        assert_eq!(frame.cells.len(), 2);
        assert!(
            frame
                .cursor
                .is_none_or(|(row, column)| row < 1 && column < 2)
        );
    }

    #[test]
    fn terminal_model_history_and_alternate_screen_remain_bounded() {
        let mut model = Model::new((0xffffff, 0));
        model.resize(512, 2);
        for _ in 0..300 {
            model.advance("a\u{301}\u{301}\u{301}\r\n".as_bytes());
        }
        assert!(model.frame().scrollback <= HISTORY_BYTES / CELL_BUDGET / 512);
        model.scroll(i32::MAX);
        model.scroll(i32::MAX);
        model.scroll(i32::MIN);
        model.scroll_to(true);
        model.scroll(-1);
        model.resize(usize::MAX, usize::MAX);
        let frame = model.frame();
        assert_eq!((frame.columns, frame.rows), (512, 256));
        model.advance(b"\x1b[?1049h\x1b[?1007h\x1b[?1h");
        assert_eq!(
            model.mouse(0, 0, MouseKind::Press(64), false, false, false),
            b"\x1bOA"
        );
        assert!(
            model
                .mouse(0, 0, MouseKind::Press(64), false, false, true)
                .is_empty()
        );
        model.advance(b"\x1b[?1049l");
        assert!(model.frame().scrollback <= HISTORY_BYTES / CELL_BUDGET / 512);
    }
    #[test]
    fn terminal_model_nested_osc_and_ignored_controls_cannot_retain_hyperlinks() {
        let mut model = Model::new((0xffffff, 0));
        model.advance(b"\x1b]4;1;red\x1b]8;;https://example.invalid/hidden\x1b\\A");
        model.advance(b"\x1b]8\0;;https://example.invalid/hidden\x1b\\B");
        assert!(
            model.term.grid()[Point::new(Line(0), Column(0))]
                .hyperlink()
                .is_none()
        );
        assert!(
            model.term.grid()[Point::new(Line(0), Column(1))]
                .hyperlink()
                .is_none()
        );
        assert_eq!(model.frame().cells[0].text, "A");
        assert_eq!(model.frame().cells[1].text, "B");
        model.advance(b"\x1b]4;1;rgb:12/34/56\x1b\\\x1b]4;1;?\x1b\\");
        assert_eq!(
            String::from_utf8(model.responses()).unwrap(),
            "\x1b]4;1;rgb:1212/3434/5656\x1b\\"
        );
    }
}
