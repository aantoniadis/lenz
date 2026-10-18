//! Terminal presentation. Coordinates here are cells; every source hit keeps its UTF-8 byte.
use crate::app::{App, Focus, Prompt};
use lenz_content::icons::Icon;
use lenz_content::{markdown::BlockKind, theme::Palette};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};
use reader_core::{
    navigation::{Kind, Scope},
    session::{Command, CommandId, MenuEntry, Rendering, Split, State},
    tabs::{Tab, TabId},
};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Default)]
pub(crate) struct Geometry {
    pub controls: Vec<(CommandId, Rect)>,
    pub tree: Rect,
    pub results: Rect,
    pub panes: Vec<PaneGeometry>,
    pub modal: Rect,
    pub modal_rows: Rect,
    pub modal_content_rows: usize,
    pub modal_items: Vec<usize>,
    pub navigation_scopes: Vec<(Scope, Rect)>,
    pub navigation_revision: Option<u64>,
}

#[derive(Default)]
pub(crate) struct PaneGeometry {
    pub controls: Vec<(CommandId, Rect)>,
    pub pane: u8,
    pub preview: bool,
    pub preview_top: usize,
    pub preview_total: usize,
    pub preview_source_lines: std::sync::Arc<[usize]>,
    markdown: Option<std::sync::Arc<MarkdownLayout>>,
    pub tabs: Vec<(TabId, Rect)>,
    pub crumbs: Vec<(usize, Rect)>,
    pub text: Rect,
    pub rows: Vec<(u16, usize, usize)>,
    pub hits: Vec<TextHit>,
}

#[derive(Clone, Debug)]
pub(crate) struct TextHit {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub line: usize,
    pub byte: usize,
}

pub(crate) fn color(rgb: u32) -> Color {
    Color::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}
fn style(fg: u32, bg: u32) -> Style {
    Style::default().fg(color(fg)).bg(color(bg))
}
fn line(buf: &mut Buffer, area: Rect, text: &str, st: Style) {
    if area.height > 0 {
        buf.set_stringn(area.x, area.y, text, area.width as usize, st);
    }
}
fn row(area: Rect, offset: u16) -> Rect {
    Rect::new(
        area.x,
        area.y.saturating_add(offset),
        area.width,
        u16::from(offset < area.height),
    )
}
fn fill(buf: &mut Buffer, area: Rect, st: Style) {
    buf.set_style(area, st);
}
fn inner(area: Rect) -> Rect {
    Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    )
}
fn clamp(at: &mut usize, top: &mut usize, len: usize, height: usize) {
    *at = (*at).min(len.saturating_sub(1));
    *top = (*top).min(len.saturating_sub(height));
    if *at < *top {
        *top = *at;
    }
    if height > 0 && *at >= top.saturating_add(height) {
        *top = *at + 1 - height;
    }
}

impl App {
    pub fn draw(&mut self, frame: &mut ratatui::Frame) {
        self.show_documentation();
        let area = frame.area();
        let palette = **self.session.state().palette();
        let p = &palette;
        let mut geometry = Geometry::default();
        fill(frame.buffer_mut(), area, style(p.fg, p.bg));
        if area.is_empty() {
            self.layout = geometry;
            return;
        }
        let body = Rect::new(
            area.x,
            area.y,
            area.width,
            area.height.saturating_sub(u16::from(area.height >= 2)),
        );
        let tree_width =
            if self.tree_shown && self.session.state().project().is_some() && area.width >= 36 {
                (area.width / 4).clamp(16, 32)
            } else {
                0
            };
        geometry.tree = Rect::new(body.x, body.y, tree_width, body.height);
        let main = Rect::new(
            body.x + tree_width,
            body.y,
            body.width.saturating_sub(tree_width),
            body.height,
        );
        let results_height = if self.session.state().results().is_some() && main.height >= 6 {
            (main.height / 3).max(4)
        } else {
            0
        };
        geometry.results = Rect::new(
            main.x,
            main.bottom().saturating_sub(results_height),
            main.width,
            results_height,
        );
        let reading = Rect::new(
            main.x,
            main.y,
            main.width,
            main.height.saturating_sub(results_height),
        );
        let panels = match self.session.state().split() {
            Some(Split::Columns) => {
                let half = reading.width / 2;
                vec![
                    Rect::new(reading.x, reading.y, half, reading.height),
                    Rect::new(
                        reading.x + half,
                        reading.y,
                        reading.width - half,
                        reading.height,
                    ),
                ]
            }
            Some(Split::Rows) => {
                let half = reading.height / 2;
                vec![
                    Rect::new(reading.x, reading.y, reading.width, half),
                    Rect::new(
                        reading.x,
                        reading.y + half,
                        reading.width,
                        reading.height - half,
                    ),
                ]
            }
            None => vec![reading],
        };
        // Resize commands own view clamping and wrap state in the core.
        for (pane, rect) in panels.iter().copied().enumerate() {
            let text = text_area(self.session.state(), pane as u8, rect);
            if self
                .layout
                .panes
                .get(pane)
                .is_none_or(|old| (old.text.width, old.text.height) != (text.width, text.height))
            {
                self.dispatch(Command::SetTextSize {
                    pane: pane as u8,
                    cols: text.width as usize,
                    rows: text.height as usize,
                });
            }
        }
        self.draw_tree(frame, geometry.tree, p);
        if geometry.tree.width >= 3 && geometry.tree.height > 0 {
            let rect = Rect::new(geometry.tree.right() - 3, geometry.tree.y, 3, 1);
            geometry
                .controls
                .push((reader_core::session::CommandId::LocateFile, rect));
            line(
                frame.buffer_mut(),
                rect,
                " ◎ ",
                style(
                    if self
                        .session
                        .disabled(reader_core::session::CommandId::LocateFile)
                        .is_none()
                    {
                        p.fg
                    } else {
                        p.dim
                    },
                    p.header_bg,
                ),
            );
        }
        geometry.tree.y = geometry
            .tree
            .y
            .saturating_add(u16::from(geometry.tree.height > 0));
        geometry.tree.height = geometry.tree.height.saturating_sub(1);
        for (pane, rect) in panels.into_iter().enumerate() {
            geometry
                .panes
                .push(draw_pane(frame, self, pane as u8, rect, p));
        }
        for pane in &geometry.panes {
            geometry.controls.extend(pane.controls.iter().copied());
            if pane.preview
                && let Some(tab) = self.session.state().shown(pane.pane)
            {
                self.preview_scroll
                    .insert(tab.id(), (tab.version(), pane.preview_top));
            }
        }
        self.draw_results(frame, geometry.results, p);
        if geometry.results.width >= 64
            && self
                .session
                .state()
                .results()
                .is_some_and(|r| r.preview().is_some())
        {
            geometry.results.width /= 2;
        }
        geometry.results.y = geometry
            .results
            .y
            .saturating_add(u16::from(geometry.results.height > 0));
        geometry.results.height = geometry.results.height.saturating_sub(1);
        let state = self.session.state();
        let status = self
            .notice
            .clone()
            .or_else(|| state.asking())
            .or_else(|| state.notices().last().cloned())
            .unwrap_or_else(|| {
                let position = state
                    .active()
                    .map(|t| {
                        if let Some(image) = t
                            .document()
                            .and_then(|d| d.image())
                            .filter(|_| !t.image_source())
                        {
                            return format!(
                                "{}×{} · {:?} · {} bytes{}",
                                image.width,
                                image.height,
                                image.format,
                                image.byte_size,
                                if image.downscaled {
                                    " · downscaled"
                                } else {
                                    ""
                                }
                            );
                        }
                        let at = t.selection().and_then(|s| {
                            t.document().map(|d| {
                                (
                                    d.line_of(s.head),
                                    s.head - d.line_range(d.line_of(s.head)).start,
                                )
                            })
                        });
                        at.map_or_else(
                            || format!("Line {}", t.top_line() + 1),
                            |(l, b)| format!("{}:{}", l + 1, b + 1),
                        )
                    })
                    .unwrap_or_else(|| "Open a file or folder with F10".into());
                format!(
                    "{position}  {}",
                    state.provider().map_or_else(String::new, |v| v.summary())
                )
            });
        if area.height >= 2 {
            line(
                frame.buffer_mut(),
                row(area, area.height - 1),
                &format!(
                    " {} [F2] · Lenz Reader  {}  F10 Commands",
                    state.language_server_label(),
                    Icon::Position.label(self.nerd_icons, &status)
                ),
                style(p.fg, p.status_bg),
            );
            let control = Rect::new(
                area.x,
                area.bottom() - 1,
                (state.language_server_label().width() as u16 + 6).min(area.width),
                1,
            );
            let attention = state.provider().is_some_and(|v| {
                matches!(
                    v.status,
                    reader_core::semantics::Status::Missing(_)
                        | reader_core::semantics::Status::Failed(_)
                        | reader_core::semantics::Status::Disabled
                )
            });
            frame.buffer_mut().set_style(
                control,
                style(if attention { p.error } else { p.accent }, p.status_bg)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            );
            geometry.controls.push((CommandId::LanguageServer, control));
        }
        if self.modal.is_some() {
            self.draw_modal(frame, &mut geometry, p);
        } else if let Some(doc) = self.session.state().documentation() {
            let popup = centered(
                area,
                area.width.saturating_sub(4).min(88),
                area.height.saturating_sub(4).min(24),
            );
            frame.render_widget(Clear, popup);
            frame.render_widget(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Documentation · Esc closes ")
                    .style(style(p.fg, p.status_bg)),
                popup,
            );
            frame.render_widget(
                Paragraph::new(doc.preview.doc.text.as_str())
                    .wrap(Wrap { trim: false })
                    .style(style(p.fg, p.status_bg)),
                inner(popup),
            );
        }
        self.layout = geometry;
    }

    fn draw_tree(&mut self, frame: &mut ratatui::Frame, area: Rect, p: &Palette) {
        if area.is_empty() {
            return;
        }
        fill(frame.buffer_mut(), area, style(p.fg, p.sidebar_bg));
        let Some(project) = self.session.state().project() else {
            line(
                frame.buffer_mut(),
                area,
                " No folder open",
                style(p.dim, p.sidebar_bg),
            );
            return;
        };
        line(
            frame.buffer_mut(),
            area,
            &format!(
                " {}",
                Icon::FolderOpen.label(
                    self.nerd_icons,
                    &project
                        .root()
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                )
            ),
            style(
                if self.focus == Focus::Tree {
                    p.accent
                } else {
                    p.fg
                },
                p.header_bg,
            )
            .add_modifier(Modifier::BOLD),
        );
        let rows = project.rows();
        clamp(
            &mut self.tree_at,
            &mut self.tree_top,
            rows.len(),
            area.height.saturating_sub(1) as usize,
        );
        for (i, entry) in rows
            .iter()
            .enumerate()
            .skip(self.tree_top)
            .take(area.height.saturating_sub(1) as usize)
        {
            let bg = if i == self.tree_at && self.focus == Focus::Tree {
                p.selected_bg
            } else {
                p.sidebar_bg
            };
            let r = row(area, (i - self.tree_top + 1) as u16);
            fill(frame.buffer_mut(), r, style(p.fg, bg));
            let mark = match entry.dir {
                Some(true) => "▾",
                Some(false) => "▸",
                None => " ",
            };
            let icon = match entry.dir {
                Some(open) => lenz_content::file_icons::folder(open),
                None => lenz_content::file_icons::file(&entry.name),
            };
            let name = icon.label(self.nerd_icons, &entry.name);
            line(
                frame.buffer_mut(),
                r,
                &format!(" {}{mark} {}", "  ".repeat(entry.depth.min(20)), name),
                style(if entry.hidden() { p.dim } else { p.fg }, bg).add_modifier(
                    if i == self.tree_at && self.focus == Focus::Tree {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    },
                ),
            );
            let icon_x = r.x.saturating_add(3 + 2 * entry.depth.min(20) as u16);
            if self.nerd_icons && icon_x < r.right() {
                frame.buffer_mut()[(icon_x, r.y)].set_fg(color(icon.color_on(p, bg)));
            }
        }
    }

    fn draw_results(&mut self, frame: &mut ratatui::Frame, area: Rect, p: &Palette) {
        if area.is_empty() {
            return;
        }
        let Some(results) = self.session.state().results() else {
            return;
        };
        fill(frame.buffer_mut(), area, style(p.fg, p.sidebar_bg));
        let title = format!(
            " {} · {} results{}{}",
            results.kind(),
            results.hits().len(),
            if results.done() {
                ""
            } else {
                " · searching…"
            },
            if results.limited() {
                " · limit reached"
            } else {
                ""
            }
        );
        line(
            frame.buffer_mut(),
            area,
            &Icon::Search.label(self.nerd_icons, &title),
            style(
                if self.focus == Focus::Results {
                    p.accent
                } else {
                    p.fg
                },
                p.header_bg,
            )
            .add_modifier(Modifier::BOLD),
        );
        let list_width = if results.preview().is_some() && area.width >= 64 {
            area.width / 2
        } else {
            area.width
        };
        let list = Rect::new(
            area.x,
            area.y + 1,
            list_width,
            area.height.saturating_sub(1),
        );
        clamp(
            &mut self.results_at,
            &mut self.results_top,
            results.rows().len(),
            list.height as usize,
        );
        for (i, entry) in results
            .rows()
            .iter()
            .enumerate()
            .skip(self.results_top)
            .take(list.height as usize)
        {
            use reader_core::search::Row;
            let text = match entry {
                Row::Dir { dir, count, open } => {
                    format!(" {} {dir}/ ({count})", if *open { "▾" } else { "▸" })
                }
                Row::File { path, count, open } => {
                    format!("   {} {path} ({count})", if *open { "▾" } else { "▸" })
                }
                Row::Hit(h) => results
                    .hits()
                    .get(*h)
                    .map(|hit| {
                        let call = results.call(*h).map_or(String::new(), |c| {
                            format!(
                                "{}{} ",
                                "  ".repeat(c.depth.min(16) as usize),
                                if c.fold == reader_core::search::Fold::Closed {
                                    "▸"
                                } else {
                                    "▾"
                                }
                            )
                        });
                        format!(
                            " {call}{}:{} {}{}",
                            hit.path,
                            hit.line + 1,
                            hit.text,
                            if results.is_stale(*h) {
                                " [changed]"
                            } else {
                                ""
                            }
                        )
                    })
                    .unwrap_or_default(),
            };
            let r = row(list, (i - self.results_top) as u16);
            let bg = if i == self.results_at && self.focus == Focus::Results {
                p.selected_bg
            } else {
                p.sidebar_bg
            };
            fill(frame.buffer_mut(), r, style(p.fg, bg));
            line(frame.buffer_mut(), r, &text, style(p.fg, bg));
        }
        if let Some(error) = results.error() {
            line(
                frame.buffer_mut(),
                list,
                error,
                style(p.error, p.sidebar_bg),
            );
        }
        if list_width < area.width
            && let Some(preview) = results.preview()
        {
            let hit = results.hits().get(preview.index);
            let first = hit.map_or(0, |h| h.line.saturating_sub(list.height as usize / 2));
            let preview_area = Rect::new(
                area.x + list_width + 1,
                list.y,
                area.width.saturating_sub(list_width + 1),
                list.height,
            );
            for (n, l) in (first..preview.doc.line_count())
                .take(preview_area.height as usize)
                .enumerate()
            {
                let bg = if hit.is_some_and(|h| h.line == l) {
                    p.found_bg
                } else {
                    p.bg
                };
                line(
                    frame.buffer_mut(),
                    row(preview_area, n as u16),
                    &format!("{:>4} ", l + 1),
                    style(p.fg, bg),
                );
                let shown = reader_core::shown::shown_line(preview.doc.line(l), &[]);
                let spans = preview
                    .highlights()
                    .and_then(|parsed| parsed.spans(l..l + 1).pop())
                    .unwrap_or_default();
                for cell in source_cells(
                    &shown,
                    0..shown.text.len(),
                    0,
                    preview_area.width.saturating_sub(5) as usize,
                ) {
                    let fg = spans
                        .iter()
                        .find(|(range, _)| range.contains(&cell.byte))
                        .map_or(p.fg, |(_, capture)| p.syntax_color(*capture));
                    let selected = hit.is_some_and(|h| h.line == l && h.range.contains(&cell.byte));
                    frame.buffer_mut().set_stringn(
                        preview_area.x + 5 + cell.x,
                        preview_area.y + n as u16,
                        cell.text,
                        cell.width as usize,
                        style(fg, if selected { p.found_bg } else { p.bg }),
                    );
                }
            }
        }
    }

    fn draw_navigation(
        &mut self,
        frame: &mut ratatui::Frame,
        geometry: &mut Geometry,
        p: &Palette,
    ) {
        let status = self.session.navigation_status();
        let Some(search) = self.session.state().navigation() else {
            return;
        };
        let Some(modal) = &mut self.modal else { return };
        let area = centered(
            frame.area(),
            frame.area().width.saturating_sub(2).min(100),
            frame.area().height.saturating_sub(2).min(28),
        );
        geometry.modal = area;
        geometry.navigation_revision = Some(search.revision);
        if area.is_empty() {
            return;
        }
        frame.render_widget(Clear, area);
        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(color(p.accent)))
                .title(if search.scope == Scope::Recent {
                    " Recent Files "
                } else {
                    " Search Everywhere "
                })
                .title_style(
                    Style::default()
                        .fg(color(p.accent))
                        .add_modifier(Modifier::BOLD),
                )
                .style(style(p.fg, p.status_bg)),
            area,
        );
        let content = inner(area);
        if content.is_empty() {
            return;
        }
        let mut x = content.x;
        let scopes: &[Scope] = if search.scope == Scope::Recent {
            &[Scope::Recent]
        } else {
            &Scope::ALL
        };
        for &scope in scopes {
            if content.width < 42 && scope != search.scope {
                continue;
            }
            let label = format!(" {} ", scope.label());
            let width = (label.width() as u16).min(content.right().saturating_sub(x));
            if width == 0 {
                break;
            }
            let rect = Rect::new(x, content.y, width, 1);
            let selected = scope == search.scope;
            line(
                frame.buffer_mut(),
                rect,
                &label,
                style(
                    if selected { p.accent } else { p.dim },
                    if selected { p.selected_bg } else { p.status_bg },
                )
                .add_modifier(if selected {
                    Modifier::BOLD | Modifier::UNDERLINED
                } else {
                    Modifier::empty()
                }),
            );
            geometry.navigation_scopes.push((scope, rect));
            x = x.saturating_add(width + 1);
        }
        if content.height > 1 {
            let query = tail_columns(&modal.input, content.width.saturating_sub(3) as usize);
            line(
                frame.buffer_mut(),
                row(content, 1),
                &format!("› {query}"),
                style(p.fg, p.status_bg),
            );
            if content.width > 2 {
                frame.set_cursor_position((
                    content.x
                        + 2
                        + query.width().min(content.width.saturating_sub(3) as usize) as u16,
                    content.y + 1,
                ));
            }
        }
        let list_y = 2.min(content.height);
        let reserve = u16::from(content.height > list_y);
        let list = Rect::new(
            content.x,
            content.y + list_y,
            content.width,
            content.height.saturating_sub(list_y + reserve),
        );
        geometry.modal_rows = list;
        let row_height = if list.height >= 4 { 2 } else { 1 };
        let visible = (list.height / row_height).max(1) as usize;
        modal.at = modal.at.min(search.rows.len().saturating_sub(1));
        modal.top = modal.top.min(search.rows.len().saturating_sub(visible));
        if modal.reveal_selection {
            clamp(&mut modal.at, &mut modal.top, search.rows.len(), visible);
            modal.reveal_selection = false;
        }
        geometry.modal_content_rows = search.rows.len();
        for (index, result) in search.rows.iter().enumerate().skip(modal.top).take(visible) {
            let screen = ((index - modal.top) * row_height as usize) as u16;
            if screen >= list.height {
                break;
            }
            let selected = index == modal.at;
            let bg = if selected { p.selected_bg } else { p.status_bg };
            let fg = if result.disabled.is_some() {
                p.dim
            } else {
                p.fg
            };
            let icon = match result.kind {
                Kind::File => lenz_content::file_icons::file(&result.label)
                    .label(self.nerd_icons, &result.label),
                Kind::Symbol => Icon::Book.label(self.nerd_icons, &result.label),
                Kind::Action => Icon::Commands.label(self.nerd_icons, &result.label),
            };
            let setting = result.on.map_or("", |on| if on { "✓ " } else { "○ " });
            let label = format!(
                "{}{}{}  {}{}",
                if selected { "› " } else { "  " },
                setting,
                icon,
                result.kind.label(),
                result
                    .shortcut
                    .map(|s| format!("  {s}"))
                    .unwrap_or_default()
            );
            let rect = row(list, screen);
            fill(frame.buffer_mut(), rect, style(fg, bg));
            line(
                frame.buffer_mut(),
                rect,
                &label,
                style(fg, bg).add_modifier(if selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
            );
            geometry.modal_items.push(index);
            if row_height > 1 {
                let detail = result.disabled.unwrap_or(&result.detail);
                let rect = row(list, screen + 1);
                fill(frame.buffer_mut(), rect, style(p.dim, bg));
                line(
                    frame.buffer_mut(),
                    rect,
                    &format!("  {detail}"),
                    style(p.dim, bg),
                );
                geometry.modal_items.push(index);
            }
        }
        if search.rows.is_empty() && list.height > 0 {
            line(
                frame.buffer_mut(),
                list,
                if search.pending {
                    "  Searching…"
                } else {
                    "  No matching results"
                },
                style(p.dim, p.status_bg),
            );
        }
        if reserve > 0 {
            line(
                frame.buffer_mut(),
                row(content, content.height - 1),
                &status,
                style(p.dim, p.status_bg),
            );
        }
    }

    fn draw_modal(&mut self, frame: &mut ratatui::Frame, geometry: &mut Geometry, p: &Palette) {
        if self
            .modal
            .as_ref()
            .is_some_and(|m| m.kind == Prompt::Navigation)
        {
            self.draw_navigation(frame, geometry, p);
            return;
        }
        let title = self.modal_title();
        let runtime_message = self
            .session
            .state()
            .server_runtime()
            .map(|m| m.message.clone());
        let choices = if self
            .modal
            .as_ref()
            .is_some_and(|m| matches!(m.kind, Prompt::LanguageServer | Prompt::ServerRuntime))
        {
            self.choices_for(
                frame
                    .area()
                    .width
                    .saturating_sub(4)
                    .min(98)
                    .saturating_sub(2)
                    .max(1) as usize,
            )
        } else {
            self.choices()
        };
        let Some(modal) = &mut self.modal else {
            return;
        };
        let context = matches!(modal.kind, Prompt::Context { .. });
        let mut separators = Vec::new();
        let area = if let Prompt::Context { x, y, ref entries } = modal.kind {
            let width = (choices.iter().map(|c| c.label.width()).max().unwrap_or(0) + 4)
                .max(20)
                .min(frame.area().width as usize) as u16;
            let height = (entries.len() + 3).min(frame.area().height as usize) as u16;
            let mut choice = 0;
            for entry in entries {
                match entry {
                    MenuEntry::Separator => separators.push(choice),
                    MenuEntry::Command(_) => choice += 1,
                }
            }
            Rect::new(
                x.min(frame.area().right().saturating_sub(width)),
                y.min(frame.area().bottom().saturating_sub(height)),
                width,
                height,
            )
        } else {
            centered(
                frame.area(),
                frame.area().width.saturating_sub(2).min(100),
                frame.area().height.saturating_sub(2).min(28),
            )
        };
        geometry.modal = area;
        if area.is_empty() {
            return;
        }
        frame.render_widget(Clear, area);
        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(color(p.accent)))
                .title(format!(" {title} "))
                .title_style(
                    Style::default()
                        .fg(color(p.accent))
                        .add_modifier(Modifier::BOLD),
                )
                .style(style(p.fg, p.status_bg)),
            area,
        );
        let content = inner(area);
        if matches!(modal.kind, Prompt::Help | Prompt::Details(_)) {
            let body = match &modal.kind {
                Prompt::Details(body) => body.as_str(),
                _ => HELP,
            };
            let wrapped = wrap_text(body, content.width.max(1) as usize);
            geometry.modal_content_rows = wrapped.len();
            geometry.modal_rows = content;
            modal.top = modal
                .top
                .min(wrapped.len().saturating_sub(content.height as usize));
            for (n, text) in wrapped
                .iter()
                .skip(modal.top)
                .take(content.height as usize)
                .enumerate()
            {
                line(
                    frame.buffer_mut(),
                    row(content, n as u16),
                    text,
                    style(p.fg, p.status_bg),
                );
            }
            return;
        }
        let input_rows = if context || modal.kind == Prompt::LanguageServer {
            0
        } else {
            1 + usize::from(
                matches!(modal.kind, Prompt::Search)
                    || !modal.secondary.is_empty()
                    || modal.secondary_focus,
            )
        };
        for (i, text) in std::iter::once(&modal.input)
            .chain((input_rows > 1).then_some(&modal.secondary))
            .take(input_rows)
            .enumerate()
        {
            let active = (i == 1) == modal.secondary_focus;
            let prefix = if active { "› " } else { "  " };
            let room = content.width.saturating_sub(2) as usize;
            let tail = tail_columns(text, room);
            line(
                frame.buffer_mut(),
                row(content, i as u16),
                &format!("{prefix}{tail}"),
                style(if active { p.fg } else { p.dim }, p.status_bg),
            );
        }
        let list_y = input_rows.min(content.height as usize) as u16;
        let reserve = u16::from(content.height > list_y);
        geometry.modal_rows = Rect::new(
            content.x,
            content.y + list_y,
            content.width,
            content.height.saturating_sub(list_y + reserve),
        );
        let list = geometry.modal_rows;
        // Wrapped details participate in scrolling: selection remains a choice index, top is a visual row.
        let width = list.width.max(1) as usize;
        let mut rendered = Vec::new();
        let mut selected = 0..0;
        modal.at = modal.at.min(choices.len().saturating_sub(1));
        for (i, choice) in choices.iter().enumerate() {
            if separators.contains(&i) {
                rendered.push((usize::MAX, "─".repeat(width), true));
            }
            let start = rendered.len();
            let suffix = if context {
                String::new()
            } else {
                choice
                    .disabled
                    .map(|why| format!(" — {why}"))
                    .or_else(|| choice.detail.as_ref().map(|d| format!(" — {d}")))
                    .unwrap_or_default()
            };
            for text in wrap_text(
                &format!(
                    "{}{}{}",
                    if i == modal.at { "› " } else { "  " },
                    choice.label,
                    suffix
                ),
                width,
            ) {
                rendered.push((i, text, choice.disabled.is_some()));
            }
            if i == modal.at {
                selected = start..rendered.len();
            }
        }
        geometry.modal_content_rows = rendered.len();
        modal.top = modal
            .top
            .min(rendered.len().saturating_sub(list.height as usize));
        if modal.reveal_selection {
            if selected.start < modal.top {
                modal.top = selected.start;
            }
            if list.height > 0 && selected.start >= modal.top + list.height as usize {
                modal.top = selected.start + 1 - list.height as usize;
            }
            modal.reveal_selection = false;
        }
        for (n, (i, text, disabled)) in rendered
            .iter()
            .skip(modal.top)
            .take(list.height as usize)
            .enumerate()
        {
            geometry.modal_items.push(*i);
            let bg = if *i == modal.at {
                p.selected_bg
            } else {
                p.status_bg
            };
            let r = row(list, n as u16);
            fill(frame.buffer_mut(), r, style(p.fg, bg));
            line(
                frame.buffer_mut(),
                r,
                text,
                style(if *disabled { p.dim } else { p.fg }, bg),
            );
        }
        if reserve > 0 {
            let footer = modal
                .error
                .as_deref()
                .or_else(|| {
                    context
                        .then(|| choices.get(modal.at).and_then(|c| c.disabled))
                        .flatten()
                })
                .or_else(|| {
                    (modal.kind == Prompt::ServerRuntime)
                        .then_some(runtime_message.as_deref())
                        .flatten()
                })
                .unwrap_or(if modal.kind == Prompt::LanguageServer {
                    "Ctrl+C copies details · Enter runs action · Esc closes"
                } else {
                    "Enter accepts · Esc closes · ↑↓ selects · Tab field"
                });
            line(
                frame.buffer_mut(),
                row(content, content.height - 1),
                footer,
                style(
                    if modal.error.is_some() {
                        p.error
                    } else {
                        p.dim
                    },
                    p.status_bg,
                ),
            );
        }
        if !context
            && modal.kind != Prompt::LanguageServer
            && content.height > 0
            && content.width > 2
        {
            let text = if modal.secondary_focus {
                &modal.secondary
            } else {
                &modal.input
            };
            frame.set_cursor_position((
                content.x
                    + 2
                    + UnicodeWidthStr::width(tail_columns(
                        text,
                        content.width.saturating_sub(3) as usize,
                    ))
                    .min(content.width.saturating_sub(3) as usize) as u16,
                content.y + u16::from(modal.secondary_focus).min(content.height - 1),
            ));
        }
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width.min(area.width),
        height.min(area.height),
    )
}

fn text_area(state: &State, pane: u8, area: Rect) -> Rect {
    let bars = 1 + u16::from(state.breadcrumbs());
    let gutter = state
        .shown(pane)
        .and_then(|t| t.document())
        .map_or(3, |d| d.line_count().to_string().len() as u16 + 3)
        .min(area.width);
    Rect::new(
        area.x + gutter,
        area.y + bars.min(area.height),
        area.width.saturating_sub(gutter),
        area.height.saturating_sub(bars),
    )
}

fn draw_pane(
    frame: &mut ratatui::Frame,
    app: &App,
    pane: u8,
    area: Rect,
    p: &Palette,
) -> PaneGeometry {
    let state = app.session.state();
    let focus = app.focus;
    let crumb_at = app.crumb_at;
    let preview_selection = app.preview_selection;
    let preview_scroll = state
        .shown(pane)
        .and_then(|tab| app.preview_scroll.get(&tab.id()).copied());
    let old_markdown = app
        .layout
        .panes
        .get(pane as usize)
        .and_then(|pane| pane.markdown.clone());
    let mut geometry = PaneGeometry {
        pane,
        text: text_area(state, pane, area),
        ..Default::default()
    };
    if area.is_empty() {
        return geometry;
    }
    let active = state.shown(pane).map(Tab::id);
    fill(frame.buffer_mut(), row(area, 0), style(p.fg, p.header_bg));
    let mut x = area.x;
    if pane == 0 {
        let reserve = if state.pane_tabs(pane).next().is_some() {
            area.width.min(8)
        } else {
            0
        };
        for (id, label) in app.session.navigation() {
            let label = if id == CommandId::ToggleExplorer {
                "▥"
            } else {
                label
            };
            let width = (label.width() + 2) as u16;
            if width > area.right().saturating_sub(x).saturating_sub(reserve) {
                break;
            }
            let rect = Rect::new(x, area.y, width, 1);
            geometry.controls.push((id, rect));
            let st = style(
                if app.session.disabled(id).is_some() {
                    p.dim
                } else {
                    p.fg
                },
                p.header_bg,
            );
            line(frame.buffer_mut(), rect, &format!(" {label} "), st);
            x += width;
        }
    }
    let tab_width = area.right().saturating_sub(x);
    let tabs: Vec<_> = state.pane_tabs(pane).collect();
    let widths: Vec<_> = tabs
        .iter()
        .map(|tab| {
            (UnicodeWidthStr::width(tab.title().as_str()) as u16
                + 4
                + if app.nerd_icons { 2 } else { 0 })
            .min(32)
            .min(tab_width)
        })
        .collect();
    let selected = tabs
        .iter()
        .position(|tab| Some(tab.id()) == active)
        .unwrap_or(0);
    let mut first = selected;
    let mut used = widths.get(selected).copied().unwrap_or(0);
    while first > 0 && used.saturating_add(widths[first - 1]) <= tab_width {
        first -= 1;
        used += widths[first];
    }
    for (i, tab) in tabs.iter().enumerate().skip(first) {
        let text = format!(
            " {}{} ",
            lenz_content::file_icons::file(&tab.title()).label(app.nerd_icons, &tab.title()),
            if tab.retained() { "" } else { " ◇" }
        );
        let width = widths[i].min(area.right().saturating_sub(x));
        if width == 0 {
            break;
        }
        let r = Rect::new(x, area.y, width, 1);
        geometry.tabs.push((tab.id(), r));
        line(
            frame.buffer_mut(),
            r,
            &text,
            style(
                if Some(tab.id()) == active {
                    p.accent
                } else {
                    p.dim
                },
                if Some(tab.id()) == active {
                    p.bg
                } else {
                    p.header_bg
                },
            )
            .add_modifier(if Some(tab.id()) == active {
                Modifier::BOLD
                    | if focus == Focus::Text && pane == state.focused_pane() {
                        Modifier::UNDERLINED
                    } else {
                        Modifier::empty()
                    }
            } else {
                Modifier::empty()
            }),
        );
        if app.nerd_icons && width > 1 {
            let icon = lenz_content::file_icons::file(&tab.title());
            let bg = if Some(tab.id()) == active {
                p.bg
            } else {
                p.header_bg
            };
            frame.buffer_mut()[(r.x + 1, r.y)].set_fg(color(icon.color_on(p, bg)));
        }
        x += width;
    }
    if state.breadcrumbs() && area.height > 1 {
        let mut x = area.x;
        if let Some(bar) = state.breadcrumbs_for(pane) {
            for (i, crumb) in bar.crumbs.iter().enumerate() {
                use reader_core::breadcrumbs::Crumb;
                let name = match crumb {
                    Crumb::Folder { name, .. } | Crumb::File { name } => name.as_ref(),
                    Crumb::Decl { name, .. } => name.as_ref(),
                };
                let text = format!("{name} › ");
                let width = (UnicodeWidthStr::width(text.as_str()) as u16)
                    .min(area.right().saturating_sub(x));
                if width == 0 {
                    break;
                }
                let r = Rect::new(x, area.y + 1, width, 1);
                geometry.crumbs.push((i, r));
                line(
                    frame.buffer_mut(),
                    r,
                    &text,
                    style(
                        if focus == Focus::Breadcrumbs
                            && pane == state.focused_pane()
                            && i == crumb_at
                        {
                            p.accent
                        } else {
                            p.dim
                        },
                        p.header_bg,
                    ),
                );
                x += width;
            }
        }
    }
    let Some(tab) = state.shown(pane) else {
        line(
            frame.buffer_mut(),
            geometry.text,
            "Open a file with Ctrl+P or F10",
            style(p.dim, p.bg),
        );
        return geometry;
    };
    let Some(doc) = tab.document() else {
        line(
            frame.buffer_mut(),
            geometry.text,
            "Loading…",
            style(p.dim, p.bg),
        );
        return geometry;
    };
    if let Some(image) = doc.image().filter(|_| !tab.image_source()) {
        match &image.pixels {
            Ok(pixels) => draw_image(frame.buffer_mut(), geometry.text, pixels, p),
            Err(why) => line(
                frame.buffer_mut(),
                geometry.text,
                &format!("Image {}×{} · {why}", image.width, image.height),
                style(p.dim, p.bg),
            ),
        }
        return geometry;
    }
    if let Some(rendering) = tab.rendering() {
        match rendering {
            Rendering::Preview(preview) => {
                geometry.preview = true;
                let remap = old_markdown.as_ref().is_some_and(|layout| {
                    layout.width != geometry.text.width
                        || !std::sync::Arc::ptr_eq(&layout.preview, preview)
                });
                let anchor = preview_scroll
                    .filter(|(version, _)| *version == tab.version())
                    .and_then(|(_, top)| {
                        old_markdown
                            .as_ref()
                            .filter(|old| std::sync::Arc::ptr_eq(&old.preview.doc, &preview.doc))
                            .and_then(|old| old.rows.get(top))
                            .map(|row| (row.block, row.range.start))
                    });
                let layout = old_markdown
                    .filter(|layout| {
                        layout.width == geometry.text.width
                            && std::sync::Arc::ptr_eq(&layout.preview, preview)
                    })
                    .unwrap_or_else(|| {
                        std::sync::Arc::new(MarkdownLayout::new(
                            preview.clone(),
                            geometry.text.width,
                            doc,
                        ))
                    });
                geometry.preview_total = layout.rows.len();
                geometry.preview_source_lines = layout.source_lines.clone();
                geometry.preview_top = preview_scroll
                    .filter(|(version, _)| *version == tab.version())
                    .map(|(_, top)| {
                        if remap && let Some((block, offset)) = anchor {
                            layout
                                .rows
                                .iter()
                                .position(|row| {
                                    row.block >= block
                                        && (row.range.end > offset || row.range.start >= offset)
                                })
                                .unwrap_or(top)
                        } else {
                            top
                        }
                    })
                    .unwrap_or_else(|| {
                        layout
                            .source_lines
                            .partition_point(|&line| line < tab.top_line())
                    })
                    .min(
                        layout
                            .rows
                            .len()
                            .saturating_sub(geometry.text.height as usize),
                    );
                draw_markdown(
                    frame.buffer_mut(),
                    &mut geometry,
                    preview_selection
                        .filter(|(id, version, _)| *id == tab.id() && *version == tab.version())
                        .map(|(_, _, selection)| selection),
                    &layout,
                    p,
                );
                geometry.markdown = Some(layout);
                return geometry;
            }
            Rendering::Pending => {
                line(
                    frame.buffer_mut(),
                    geometry.text,
                    "Preparing Markdown preview…",
                    style(p.dim, p.bg),
                );
                return geometry;
            }
            Rendering::Source => {}
        }
    }
    let cols = state.wrap_cols(tab);
    let sticky = if state.sticky_scroll() {
        tab.sticky(
            reader_core::tabs::sticky_cap(geometry.text.height as usize),
            cols,
        )
    } else {
        std::sync::Arc::from([])
    };
    let rows = tab.rows(geometry.text.height as usize, cols);
    for (n, r) in rows.iter().enumerate() {
        let y = geometry.text.y + n as u16;
        draw_source_row(
            frame.buffer_mut(),
            state,
            tab,
            r,
            geometry.text,
            y,
            area.x,
            false,
            &mut geometry.hits,
            p,
        );
        geometry.rows.push((y, r.line, r.index));
    }
    for pinned in sticky.iter() {
        let Some(r) = tab.line_rows(pinned.line, cols).first().cloned() else {
            continue;
        };
        let y = geometry.text.y + pinned.row as u16;
        geometry.hits.retain(|h| h.y != y);
        geometry.rows.retain(|(cy, ..)| *cy != y);
        draw_source_row(
            frame.buffer_mut(),
            state,
            tab,
            &r,
            geometry.text,
            y,
            area.x,
            true,
            &mut geometry.hits,
            p,
        );
        geometry.rows.push((y, r.line, r.index));
    }
    geometry
}

struct SourceCell<'a> {
    x: u16,
    width: u16,
    byte: usize,
    display: usize,
    text: &'a str,
}
fn source_cells(
    shown: &reader_core::shown::Shown,
    range: Range<usize>,
    skip: usize,
    width: usize,
) -> Vec<SourceCell<'_>> {
    let mut cells = Vec::new();
    let mut col = 0usize;
    let Some(text) = shown.text.get(range.clone()) else {
        return cells;
    };
    for (byte, grapheme) in text.grapheme_indices(true) {
        let w = UnicodeWidthStr::width(grapheme);
        if col >= skip && col + w <= skip.saturating_add(width) {
            let display = range.start + byte;
            let source = shown
                .hints
                .iter()
                .find(|(_, r, _)| r.contains(&display))
                .map_or_else(
                    || {
                        shown
                            .map
                            .partition_point(|&d| d <= display)
                            .saturating_sub(1)
                    },
                    |(b, ..)| *b,
                );
            cells.push(SourceCell {
                x: (col - skip) as u16,
                width: w as u16,
                byte: source,
                display,
                text: grapheme,
            });
        }
        col += w;
        if col >= skip.saturating_add(width) {
            break;
        }
    }
    cells
}

#[allow(clippy::too_many_arguments)]
fn draw_source_row(
    buf: &mut Buffer,
    state: &State,
    tab: &Tab,
    r: &reader_core::wrap::Row,
    area: Rect,
    y: u16,
    gutter_x: u16,
    sticky: bool,
    hits: &mut Vec<TextHit>,
    p: &Palette,
) {
    if area.width == 0 || y >= area.bottom() {
        return;
    }
    let Some(doc) = tab.document() else {
        return;
    };
    let shown = tab.shown_line(r.line);
    let base = if sticky { p.header_bg } else { p.bg };
    fill(
        buf,
        Rect::new(gutter_x, y, area.right().saturating_sub(gutter_x), 1),
        style(p.fg, base),
    );
    let bookmark = state.places().bookmark(tab.source(), r.line);
    let fold = if r.index == 0 {
        tab.fold_at(r.line)
    } else {
        None
    };
    let mark = if bookmark.is_some() { "●" } else { " " };
    let fold_mark = fold.map_or(" ", |(_, shut)| if shut { "▸" } else { "▾" });
    let gutter = area.x.saturating_sub(gutter_x) as usize;
    let number = if r.index == 0 {
        format!(
            "{}{:>width$}{}",
            mark,
            r.line + 1,
            fold_mark,
            width = gutter.saturating_sub(2)
        )
    } else {
        " ".repeat(gutter)
    };
    line(
        buf,
        Rect::new(gutter_x, y, area.x - gutter_x, 1),
        &number,
        style(
            if bookmark.is_some() {
                p.accent
            } else {
                p.num_fg
            },
            p.gutter_bg,
        ),
    );
    let start = doc.line_range(r.line).start;
    let selection = tab.selection();
    let spans = tab.spans(r.line);
    let find = tab.find();
    let occurrences = (state.active().is_some_and(|a| a.id() == tab.id()))
        .then(|| state.occurrences())
        .flatten();
    let skip = if state.wrap_cols(tab).is_some() {
        0
    } else {
        tab.column()
    };
    let indent = r.indent.min(area.width as usize);
    let cells = source_cells(
        &shown,
        r.display.clone(),
        skip,
        (area.width as usize).saturating_sub(indent),
    );
    let mut end_x = area.x + indent as u16;
    for cell in cells {
        let source = start + cell.byte;
        let mut st = style(p.fg, base);
        if let Some((_, capture, mods)) =
            spans.iter().find(|(range, ..)| range.contains(&cell.byte))
        {
            if let Some(capture) = capture {
                st = st.fg(color(p.syntax_color(*capture)));
            }
            if mods & reader_core::decor::MUTABLE != 0 {
                st = st.add_modifier(Modifier::UNDERLINED);
            }
            if mods & reader_core::decor::DEPRECATED != 0 {
                st = st.add_modifier(Modifier::CROSSED_OUT);
            }
        }
        let hint = shown
            .hints
            .iter()
            .any(|(_, range, _)| range.contains(&cell.display));
        if hint {
            st = style(p.dim, base).add_modifier(Modifier::ITALIC);
        }
        if !hint && occurrences.is_some_and(|o| o.ranges.iter().any(|(r, _)| r.contains(&source))) {
            st = st.bg(color(p.fold_bg));
        }
        if !hint
            && find.is_some_and(|f| {
                f.in_line(r.line)
                    .iter()
                    .any(|(_, range)| range.contains(&cell.byte))
            })
        {
            st = st.bg(color(p.found_bg));
        }
        if !hint && selection.is_some_and(|s| s.range().contains(&source)) {
            st = st.bg(color(p.sel_bg));
        }
        if selection.is_some_and(|s| s.anchor == s.head && s.head == source)
            && shown.caret(cell.byte) == cell.display
        {
            st = st.add_modifier(Modifier::REVERSED);
        }
        let x = area.x + indent as u16 + cell.x;
        buf.set_stringn(x, y, cell.text, cell.width as usize, st);
        hits.push(TextHit {
            x,
            y,
            width: cell.width.max(1),
            line: r.line,
            byte: cell.byte,
        });
        end_x = x + cell.width;
    }
    if end_x < area.right() {
        hits.push(TextHit {
            x: end_x,
            y,
            width: area.right() - end_x,
            line: r.line,
            byte: r.source.end.min(doc.line(r.line).len()),
        });
        if selection.is_some_and(|s| s.anchor == s.head && s.head == start + r.source.end) {
            buf.set_stringn(
                end_x,
                y,
                " ",
                1,
                style(p.fg, base).add_modifier(Modifier::REVERSED),
            );
        }
        if fold.is_some_and(|(_, shut)| shut) {
            buf.set_stringn(
                end_x,
                y,
                " …",
                (area.right() - end_x) as usize,
                style(p.dim, p.fold_bg),
            );
        }
    }
}

fn draw_image(buf: &mut Buffer, area: Rect, pixels: &lenz_content::image::Pixels, p: &Palette) {
    if area.is_empty() || pixels.width() == 0 || pixels.height() == 0 {
        return;
    }
    let scale = (area.width as f64 / pixels.width() as f64)
        .min(area.height as f64 * 2.0 / pixels.height() as f64);
    let width = (pixels.width() as f64 * scale).floor().max(1.0) as u16;
    let height = (pixels.height() as f64 * scale / 2.0).ceil().max(1.0) as u16;
    let blend = |pixel: &[u8; 4]| {
        let a = pixel[3] as u32;
        let channel = |i: usize, shift: u32| {
            ((pixel[i] as u32 * a + ((p.bg >> shift) & 255) * (255 - a)) / 255) as u8
        };
        Color::Rgb(channel(0, 16), channel(1, 8), channel(2, 0))
    };
    for y in 0..height.min(area.height) {
        for x in 0..width.min(area.width) {
            let sx = ((x as f64 / scale) as u32).min(pixels.width() - 1);
            let sy = ((y as f64 * 2.0 / scale) as u32).min(pixels.height() - 1);
            let sy2 = (((y as f64 * 2.0 + 1.0) / scale) as u32).min(pixels.height() - 1);
            buf.set_stringn(
                area.x + x,
                area.y + y,
                "▀",
                1,
                Style::default()
                    .fg(blend(&pixels.get_pixel(sx, sy).0))
                    .bg(blend(&pixels.get_pixel(sx, sy2).0)),
            );
        }
    }
}

struct MarkdownLayout {
    preview: std::sync::Arc<reader_core::markdown::Preview>,
    width: u16,
    rows: Vec<MarkdownRow>,
    source_lines: std::sync::Arc<[usize]>,
}

struct MarkdownRow {
    block: usize,
    range: Range<usize>,
    kind: MarkdownRowKind,
}

enum MarkdownRowKind {
    Text { first: bool },
    Image(usize),
    MissingImage,
    Blank,
}

impl MarkdownLayout {
    fn new(
        preview: std::sync::Arc<reader_core::markdown::Preview>,
        width: u16,
        source: &reader_core::document::Document,
    ) -> Self {
        let mut rows = Vec::new();
        let mut lines = Vec::new();
        for (index, block) in preview.doc.blocks.iter().enumerate() {
            let mut push = |range: Range<usize>, kind| {
                let offset =
                    preview_source(&preview.doc, range.start).unwrap_or(block.source.start);
                let line = source.line_of(offset.min(source.text().len()));
                lines.push(line.max(lines.last().copied().unwrap_or(0)));
                rows.push(MarkdownRow {
                    block: index,
                    range,
                    kind,
                });
            };
            if matches!(block.kind, BlockKind::Image { .. }) {
                if let Some(Ok(image)) = preview
                    .images
                    .as_ref()
                    .and_then(|images| images.get(&index))
                {
                    let (_, _, height) = markdown_image_size(image, width);
                    for row in 0..height {
                        push(block.text.clone(), MarkdownRowKind::Image(row));
                    }
                } else {
                    push(block.text.clone(), MarkdownRowKind::MissingImage);
                }
            } else {
                let prefix = markdown_prefix(block, width);
                let cols = (width as usize)
                    .saturating_sub(UnicodeWidthStr::width(prefix.as_str()))
                    .max(1);
                for (i, range) in wrap_ranges(
                    preview.doc.text.get(block.text.clone()).unwrap_or_default(),
                    cols,
                )
                .into_iter()
                .enumerate()
                {
                    push(
                        block.text.start + range.start..block.text.start + range.end,
                        MarkdownRowKind::Text { first: i == 0 },
                    );
                }
            }
            push(block.text.end..block.text.end, MarkdownRowKind::Blank);
        }
        Self {
            preview,
            width,
            rows,
            source_lines: lines.into(),
        }
    }
}

fn markdown_image_size(image: &lenz_content::image::Pixels, width: u16) -> (f64, u16, usize) {
    // Bound image layout metadata even for extremely tall, narrow decoded images.
    const MAX_ROWS: usize = 20;
    let scale = (width as f64 / image.width().max(1) as f64)
        .min((MAX_ROWS * 2) as f64 / image.height().max(1) as f64)
        .min(1.0);
    (
        scale,
        (image.width() as f64 * scale).floor().max(1.0) as u16,
        (image.height() as f64 * scale / 2.0).ceil().max(1.0) as usize,
    )
}

fn preview_source(doc: &lenz_content::markdown::MarkdownDocument, offset: usize) -> Option<usize> {
    use lenz_content::markdown::Origin;
    let i = doc
        .segments
        .partition_point(|segment| segment.text.end <= offset);
    let segment = doc.segments.get(i)?;
    if !segment.text.contains(&offset) {
        return None;
    }
    match &segment.origin {
        Origin::Direct(range) => Some(range.start + offset - segment.text.start),
        Origin::Normalized(range) => Some(range.start),
        Origin::Synthetic => None,
    }
}

fn markdown_prefix(block: &lenz_content::markdown::Block, width: u16) -> String {
    match &block.kind {
        BlockKind::ListItem { marker, checked } => format!(
            "{}{} ",
            "  ".repeat(block.list_depth.min(12) as usize),
            checked.map_or_else(
                || marker.clone(),
                |checked| if checked { "☑".into() } else { "☐".into() }
            )
        ),
        BlockKind::Rule => "─".repeat(width as usize),
        _ if block.quote_depth > 0 => "│ ".into(),
        _ => String::new(),
    }
}

fn wrap_ranges(text: &str, columns: usize) -> Vec<Range<usize>> {
    let (mut first, mut width) = (0, 0);
    let mut rows = Vec::new();
    for (at, g) in text.grapheme_indices(true) {
        if g == "\n" {
            rows.push(first..at);
            first = at + 1;
            width = 0;
            continue;
        }
        let w = if g == "\t" {
            4
        } else {
            UnicodeWidthStr::width(g)
        };
        if width + w > columns.max(1) && width > 0 {
            rows.push(first..at);
            first = at;
            width = 0;
        }
        width += w;
    }
    rows.push(first..text.len());
    rows
}

fn draw_markdown(
    buf: &mut Buffer,
    geometry: &mut PaneGeometry,
    selection: Option<reader_core::session::Selection>,
    layout: &MarkdownLayout,
    p: &Palette,
) {
    let area = geometry.text;
    let preview = &layout.preview;
    for (n, row) in layout
        .rows
        .iter()
        .skip(geometry.preview_top)
        .take(area.height as usize)
        .enumerate()
    {
        let y = area.y + n as u16;
        let block = &preview.doc.blocks[row.block];
        match row.kind {
            MarkdownRowKind::Blank => continue,
            MarkdownRowKind::Image(image_row) => {
                if let Some(Ok(image)) = preview
                    .images
                    .as_ref()
                    .and_then(|images| images.get(&row.block))
                {
                    let (scale, width, _) = markdown_image_size(image, area.width);
                    if scale <= 0.0 {
                        continue;
                    }
                    for x in 0..width.min(area.width) {
                        let sx = ((x as f64 / scale) as u32).min(image.width() - 1);
                        let sy = ((image_row as f64 * 2.0 / scale) as u32).min(image.height() - 1);
                        let sy2 = (((image_row as f64 * 2.0 + 1.0) / scale) as u32)
                            .min(image.height() - 1);
                        let pixel = |sy| {
                            let rgba = image.get_pixel(sx, sy).0;
                            let alpha = rgba[3] as u32;
                            let ch = |i: usize, shift: u32| {
                                ((rgba[i] as u32 * alpha + ((p.bg >> shift) & 255) * (255 - alpha))
                                    / 255) as u8
                            };
                            Color::Rgb(ch(0, 16), ch(1, 8), ch(2, 0))
                        };
                        buf.set_stringn(
                            area.x + x,
                            y,
                            "▀",
                            1,
                            Style::default().fg(pixel(sy)).bg(pixel(sy2)),
                        );
                    }
                }
                continue;
            }
            MarkdownRowKind::MissingImage => {
                if let BlockKind::Image { alt, .. } = &block.kind {
                    let why = preview
                        .images
                        .as_ref()
                        .and_then(|images| images.get(&row.block))
                        .and_then(|image| image.as_ref().err())
                        .copied()
                        .unwrap_or("loading image…");
                    line(
                        buf,
                        Rect::new(area.x, y, area.width, 1),
                        &format!("[Image: {alt} · {why}]"),
                        style(p.dim, p.bg),
                    );
                }
                continue;
            }
            MarkdownRowKind::Text { first } => {
                let prefix = markdown_prefix(block, area.width);
                let indent =
                    UnicodeWidthStr::width(prefix.as_str()).min(area.width as usize) as u16;
                if first {
                    line(
                        buf,
                        Rect::new(area.x, y, area.width, 1),
                        &prefix,
                        style(p.accent, p.bg),
                    );
                }
                let mut x = area.x + indent;
                geometry.rows.push((y, n, 0));
                for (local, grapheme) in preview.doc.text[row.range.clone()].grapheme_indices(true)
                {
                    let offset = row.range.start + local;
                    let mut st = style(p.fg, p.bg);
                    if matches!(block.kind, BlockKind::Heading(_)) {
                        st = st.fg(color(p.accent)).add_modifier(Modifier::BOLD);
                    }
                    if let Some(run) = block.runs.iter().find(|run| run.text.contains(&offset)) {
                        let s = run.style;
                        if s.strong {
                            st = st.add_modifier(Modifier::BOLD);
                        }
                        if s.emphasis {
                            st = st.add_modifier(Modifier::ITALIC);
                        }
                        if s.strike {
                            st = st.add_modifier(Modifier::CROSSED_OUT);
                        }
                        if s.link || s.wikilink {
                            st = st
                                .fg(color(p.syntax.link))
                                .add_modifier(Modifier::UNDERLINED);
                        }
                        if s.code {
                            st = st.bg(color(p.gutter_bg));
                        }
                        if s.highlight {
                            st = st.bg(color(p.found_bg));
                        }
                    }
                    if let Some(spans) = preview.syntax.get(&row.block)
                        && let Some((_, capture)) =
                            spans.iter().find(|(range, _)| range.contains(&offset))
                    {
                        st = st.fg(color(p.syntax_color(*capture)));
                    }
                    if selection.is_some_and(|s| s.range().contains(&offset)) {
                        st = st.bg(color(p.sel_bg));
                    }
                    if selection.is_some_and(|s| s.anchor == s.head && s.head == offset) {
                        st = st.add_modifier(Modifier::REVERSED);
                    }
                    let width = if grapheme == "\t" {
                        4
                    } else {
                        UnicodeWidthStr::width(grapheme) as u16
                    };
                    if x.saturating_add(width) <= area.right() {
                        buf.set_stringn(
                            x,
                            y,
                            if grapheme == "\t" { "    " } else { grapheme },
                            width as usize,
                            st,
                        );
                        geometry.hits.push(TextHit {
                            x,
                            y,
                            width: width.max(1),
                            line: n,
                            byte: offset,
                        });
                    }
                    x = x.saturating_add(width);
                }
                if x < area.right() {
                    geometry.hits.push(TextHit {
                        x,
                        y,
                        width: area.right() - x,
                        line: n,
                        byte: row.range.end,
                    });
                }
            }
        }
    }
}

fn tail_columns(text: &str, columns: usize) -> &str {
    let mut width = 0;
    let mut start = text.len();
    for (i, g) in text.grapheme_indices(true).rev() {
        width += UnicodeWidthStr::width(g);
        if width > columns {
            break;
        }
        start = i;
    }
    &text[start..]
}

pub(crate) fn wrap_text(text: &str, columns: usize) -> Vec<String> {
    let mut rows = vec![String::new()];
    let mut width = 0;
    for g in text.graphemes(true) {
        if g == "\n" {
            rows.push(String::new());
            width = 0;
            continue;
        }
        let w = UnicodeWidthStr::width(g);
        if width + w > columns.max(1) && width > 0 {
            rows.push(String::new());
            width = 0;
        }
        // rows starts nonempty and only grows.
        rows.last_mut().unwrap().push_str(g);
        width += w;
    }
    rows
}

const HELP: &str = "Lenz Reader\n\nF10 opens every shared reader command, including its availability and disabled reason. Type to filter, use arrows to choose, Enter to run.\n\nCtrl+K / Double Shift  Search Everywhere\nCtrl+P / Ctrl+Shift+N  Find file (name:line:column)\nCtrl+Alt+Shift+N  Find symbol\nCtrl+Shift+A / Ctrl+Shift+P  Find action\nTab / Shift+Tab in Search Everywhere  Change scope\nCtrl+O  Open file\nF10 → Open folder  Browse folder\nCtrl+F  Find in file\nF3 / Shift+F3  Next / previous match\nAlt+C / W / R  Find case / word / regex\nCtrl+Shift+F  Search project\nCtrl+G  Go to line\nCtrl+Shift+E  Focus explorer\nTab  Cycle explorer, text and results\nArrows / PageUp / PageDown  Navigate\nShift+arrows  Extend selection\nCtrl+A / Ctrl+C  Select all / copy\nClick or drag  Select source text\nRight-click text  Shared context actions\nClick a fold mark  Fold or unfold\nCtrl+F4  Close tab\nCtrl+W / Ctrl+Shift+W  Grow / shrink selection\nCtrl+Q  Quit\nF2  Notices, language server status and recovery\nEsc  Close dialog, documentation or cancel a pending request\n\nF10 also provides bookmarks, history, results, provider controls, themes, split panes, breadcrumbs, hints, sticky scroll and source/preview presentation.\n\nTerminal capabilities\nImages use true-color half-block cells; Markdown images fit within 20 terminal rows. Text uses terminal fonts and cell widths; italic, underline and strike depend on the terminal. Markdown is rendered from the same parsed document and local images as the GUI. Source selection keeps UTF-8 coordinates. Clipboard uses OSC 52 and requires terminal support. Double Shift needs modifier key reports from a terminal supporting the enhanced keyboard protocol; Ctrl+K always opens Search Everywhere. Independent windows retain their own views.\n\nThis reader never edits source files.\n\nEsc closes this help.";

#[cfg(test)]
#[path = "render_tests.rs"]
mod render_tests;
