//! Terminal resources for shared images, compact syntax captures and blame annotations.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use lenz::{
    annotation_session::{Annotations, CommitInspection},
    blame::{self, BlameLine, Version},
    changeset::{ChangeSet, Spec},
    document::{Placeholder, TextDocument},
    highlight::{self, Capture, Parsed},
    image_session::{ImageCommand, ImageDoc, ImageState, Mode, Viewport},
    view::Side,
    work,
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};
use std::{
    ops::Range,
    path::Path,
    sync::{
        Arc,
        mpsc::{Receiver, TryRecvError},
    },
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

type ImageResult = Option<Result<ImageDoc, String>>;
type SyntaxResult = [Option<Parsed>; 2];
type AnnotationContext = (Arc<ChangeSet>, usize, [Option<Version>; 2], [usize; 2]);
#[derive(Default)]
pub struct Visual {
    pub(crate) pane_ratio:Option<f32>,
    generation: Option<Arc<()>>,
    image_job: Option<Receiver<Option<ImageResult>>>,
    syntax_job: Option<Receiver<Option<SyntaxResult>>>,
    image: Option<Arc<ImageDoc>>,
    state: ImageState,
    syntax: [Option<Parsed>; 2],
    annotations: [Annotations; 2],
    inspection: CommitInspection,
    context: Option<AnnotationContext>,
    notice: Option<String>,
    selected_blame: Option<BlameLine>,
}
#[derive(Debug, Clone)]
pub enum BlameEffect {
    Copy(String),
    OpenCommit(String),
    CompareCommit(String),
    AnnotateParent {
        sha: String,
        path: String,
        line: usize,
    },
}
impl Visual {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    #[cfg(test)]
    pub(crate) fn cancellation_weak(&self) -> Option<std::sync::Weak<()>> {
        self.generation.as_ref().map(Arc::downgrade)
    }
    #[cfg(test)]
    pub fn load(&mut self, set: Arc<ChangeSet>, index: usize, doc: Arc<TextDocument>, spec: &Spec) {
        self.load_inner(set, index, doc, spec, None);
    }
    pub fn load_prepared(
        &mut self,
        set: Arc<ChangeSet>,
        index: usize,
        doc: Arc<TextDocument>,
        spec: &Spec,
        image: Option<Result<Arc<ImageDoc>, String>>,
    ) {
        self.load_inner(set, index, doc, spec, Some(image));
    }
    fn load_inner(
        &mut self,
        set: Arc<ChangeSet>,
        index: usize,
        doc: Arc<TextDocument>,
        spec: &Spec,
        prepared: Option<Option<Result<Arc<ImageDoc>, String>>>,
    ) {
        self.clear();
        let token = Arc::new(());
        let c = &set.comparisons[index];
        self.context = Some((
            set.clone(),
            index,
            [
                blame::version(spec, c, Side::Left),
                blame::version(spec, c, Side::Right),
            ],
            [
                doc.lines_on(Side::Left).len(),
                doc.lines_on(Side::Right).len(),
            ],
        ));
        let image_candidate = matches!(doc.placeholder_kind(), Some(Placeholder::Binary { .. }))
            || [&c.old_path, &c.path]
                .iter()
                .any(|p| p.to_ascii_lowercase().ends_with(".svg"));
        if prepared.is_none()
            && image_candidate
            && !matches!(doc.placeholder_kind(), Some(Placeholder::Large { .. }))
        {
            let image_set = set.clone();
            self.image_job = Some(work::submit(Arc::downgrade(&token), move || {
                let [l, r] = match image_set.sides(index) {
                    Ok(s) => s,
                    Err(e) => return Some(Err(e)),
                };
                let c = &image_set.comparisons[index];
                ImageDoc::read(
                    (&l.bytes, Path::new(&c.old_path), l.binary),
                    (&r.bytes, Path::new(&c.path), r.binary),
                )
            }));
        }
        if doc.placeholder_kind().is_none()
            && [Side::Left, Side::Right]
                .iter()
                .any(|&side| doc.language(side).is_some())
        {
            let current = Arc::downgrade(&token);
            self.syntax_job = Some(work::detached(current.clone(), move || {
                // Both sides at once: the syntax pool parses them in parallel.
                std::thread::scope(|scope| {
                    let parse = |side: Side| {
                        let (doc, weak) = (&doc, current.clone());
                        scope.spawn(move || {
                            doc.language(side).and_then(|lang| {
                                highlight::parse_while(lang, doc.text_arc(side), move || weak.strong_count() > 0)
                            })
                        })
                    };
                    let (left, right) = (parse(Side::Left), parse(Side::Right));
                    [left.join().ok().flatten(), right.join().ok().flatten()]
                })
            }));
        }
        if let Some(Some(image)) = prepared {
            match image {
                Ok(image) => self.image = Some(image),
                Err(error) => self.notice = Some(error),
            }
        }
        self.generation = Some(token);
    }
    pub fn busy(&self) -> bool {
        self.image_job.is_some()
            || self.syntax_job.is_some()
            || self.annotations.iter().any(Annotations::busy)
            || self.inspection.busy()
    }
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.image_job {
            match rx.try_recv() {
                Ok(Some(Some(Ok(doc)))) => {
                    self.image = Some(Arc::new(doc));
                    self.image_job = None;
                    changed = true;
                }
                Ok(Some(Some(Err(e)))) => {
                    self.notice = Some(e);
                    self.image_job = None;
                    changed = true;
                }
                Ok(_) | Err(TryRecvError::Disconnected) => {
                    self.image_job = None;
                    changed = true;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Some(rx) = &self.syntax_job {
            match rx.try_recv() {
                Ok(Some(syntax)) => {
                    self.syntax = syntax;
                    self.syntax_job = None;
                    changed = true;
                }
                Ok(None) | Err(TryRecvError::Disconnected) => {
                    self.syntax_job = None;
                    changed = true;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        for annotation in &mut self.annotations {
            changed |= annotation.poll();
        }
        changed | self.inspection.poll()
    }
    pub fn image(&self) -> Option<&ImageDoc> {
        self.image.as_deref()
    }
    pub fn image_state(&self) -> &ImageState {
        &self.state
    }
    pub fn notice(&self) -> Option<&str> {
        self.notice
            .as_deref()
            .or_else(|| self.annotations.iter().find_map(Annotations::error))
    }
    pub fn syntax(&self, side: Side, line: usize) -> Vec<(Range<usize>, Capture)> {
        self.syntax[side as usize]
            .as_ref()
            .and_then(|p| p.spans(line..line + 1).pop())
            .unwrap_or_default()
    }
    pub fn blame_available(&self, side: Side) -> bool {
        self.context
            .as_ref()
            .is_some_and(|(_, _, versions, _)| versions[side as usize].is_some())
    }
    pub fn toggle_blame(&mut self, side: Side) -> bool {
        let Some((set, index, versions, counts)) = &self.context else {
            return false;
        };
        self.annotations[side as usize].toggle(
            set.clone(),
            *index,
            side,
            versions[side as usize].clone(),
            counts[side as usize],
        )
    }
    pub fn blame_enabled(&self, side: Side) -> bool {
        self.annotations[side as usize].enabled()
    }
    pub fn annotation(&self, side: Side, line: usize) -> Option<&BlameLine> {
        self.annotations[side as usize].line(line)
    }
    pub fn inspect_blame(&mut self, side: Side, line: usize) -> bool {
        let Some(annotation) = self.annotation(side, line).cloned() else {
            return false;
        };
        if !annotation.commit.is_committed() {
            self.notice = Some("Not committed".into());
            return true;
        }
        let Some(root) = self
            .context
            .as_ref()
            .and_then(|(set, ..)| set.git_root())
            .map(Path::to_path_buf)
        else {
            return false;
        };
        self.inspection.open(root, annotation.commit.sha.clone());
        self.selected_blame = Some(annotation);
        true
    }
    pub fn commit_visible(&self) -> bool {
        self.inspection.visible()
    }
    /// Root routes these effects through its existing clipboard, browser and repository adapters.
    pub fn commit_key(&mut self, key: KeyEvent) -> Option<BlameEffect> {
        if !self.inspection.visible() {
            return None;
        }
        let sha = self.inspection.sha()?.to_string();
        match key.code {
            KeyCode::Esc => {
                self.inspection.close();
                None
            }
            KeyCode::Char('c') => Some(BlameEffect::Copy(sha)),
            KeyCode::Char('o') => Some(BlameEffect::OpenCommit(sha)),
            KeyCode::Char('d') | KeyCode::Char('s') | KeyCode::Enter => {
                Some(BlameEffect::CompareCommit(sha))
            }
            KeyCode::Char('p') | KeyCode::Char('a') => {
                self.selected_blame
                    .as_ref()
                    .map(|l| BlameEffect::AnnotateParent {
                        sha,
                        path: l.original_path.to_string(),
                        line: l.original_line,
                    })
            }
            _ => None,
        }
    }
    fn viewport(&self, area: Rect) -> Viewport {
        image_viewport_ratio(&self.state, area,self.pane_ratio.unwrap_or(0.5))
    }
    pub fn image_key(&mut self, key: KeyEvent, area: Rect) -> bool {
        if self.image.is_none() {
            return false;
        }
        let Some(command) = image_command(key) else {
            return false;
        };
        self.image_execute(command, area);
        true
    }
    pub fn image_execute(&mut self, command: ImageCommand, area: Rect) -> bool {
        let Some(doc) = &self.image else { return false };
        let viewport = self.viewport(area);
        let moved = self.state.execute(doc, command, viewport);
        if !moved {
            self.notice = Some("No changed regions".into());
        } else {
            self.notice = None;
        }
        moved
    }
    pub fn draw_image(&self, buf: &mut Buffer, area: Rect,palette:&lenz::theme::Palette) {
        if let Some(doc) = &self.image {
            draw_image_ratio(palette,doc, &self.state, buf, area,self.pane_ratio.unwrap_or(0.5));
        }
    }
    pub fn draw_commit(&self, buf: &mut Buffer, area: Rect,palette:&lenz::theme::Palette) {
        if !self.inspection.visible() || area.width < 6 || area.height < 4 {
            return;
        }
        let width = area.width.saturating_sub(4).min(100);
        let height = area.height.saturating_sub(2).min(25);
        let rect = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        let style = crate::chrome::style_with(palette).bg(crate::chrome::color(palette.status_bg));
        for y in rect.y..rect.bottom() {
            buf.set_stringn(rect.x, y, " ".repeat(width as usize), width as usize, style);
        }
        let mut lines = vec![format!("Commit {}", self.inspection.sha().unwrap_or(""))];
        if let Some(details) = self.inspection.details() {
            lines.push(format!(
                "{} <{}> · {}",
                details.author,
                details.email,
                blame::date(details.author_time)
            ));
            let mut message = details.message.lines();
            lines.extend(message.by_ref().take(8).map(str::to_string));
            if message.next().is_some() {
                lines.push("…".into());
            }
            lines.push(String::new());
            lines.extend(
                details
                    .files
                    .iter()
                    .take(10)
                    .map(|(status, path)| format!("{status} {path}")),
            );
            if details.files.len() > 10 {
                lines.push(format!("… {} more files", details.files.len() - 10));
            }
        } else {
            lines.push(self.inspection.error().unwrap_or("Loading commit…").into());
        }
        for (i, line) in lines
            .iter()
            .take(height.saturating_sub(2) as usize)
            .enumerate()
        {
            safe_text(
                buf,
                Rect::new(rect.x + 1, rect.y + i as u16, width - 2, 1),
                line,
                style,
            );
        }
        safe_text(
            buf,
            Rect::new(rect.x + 1, rect.bottom() - 1, width - 2, 1),
            "c copy SHA · o link · d changes · p annotate parent · Esc close",
            style.fg(crate::chrome::color(palette.accent)),
        );
    }
}

/// Universal half-block raster fallback. Only visible cells are sampled; no blend bitmap is retained.
#[cfg(test)]
pub fn image_viewport(state: &ImageState, area: Rect) -> Viewport {
    image_viewport_ratio(state,area,0.5)
}
pub(crate) fn image_viewport_ratio(state:&ImageState,area:Rect,ratio:f32)->Viewport {
    let width = if state.mode() == Mode::TwoUp {
        {let left=crate::panes::left(area.width,ratio);left.min(area.width.saturating_sub(left+1))}
    } else {
        area.width
    };
    Viewport {
        width: width.max(1) as f32,
        height: area.height.saturating_sub(4).max(1) as f32 * 2.0,
    }
}
pub fn image_command(key: KeyEvent) -> Option<ImageCommand> {
    if key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return None;
    }
    Some(match key.code {
        KeyCode::Char('m') => ImageCommand::CycleMode,
        KeyCode::Char('0') => ImageCommand::Fit,
        KeyCode::Char('1') => ImageCommand::ActualSize,
        KeyCode::Char('+') | KeyCode::Char('=') => ImageCommand::Zoom {
            factor: 1.5,
            offset: (0.0, 0.0),
        },
        KeyCode::Char('-') => ImageCommand::Zoom {
            factor: 1.0 / 1.5,
            offset: (0.0, 0.0),
        },
        KeyCode::Char(',') => ImageCommand::Nudge(-0.1),
        KeyCode::Char('.') => ImageCommand::Nudge(0.1),
        KeyCode::Char('n') | KeyCode::F(7) if !key.modifiers.contains(KeyModifiers::SHIFT) => {
            ImageCommand::Region { forward: true }
        }
        KeyCode::Char('p') | KeyCode::F(7) => ImageCommand::Region { forward: false },
        KeyCode::Left => ImageCommand::Pan { delta: (4.0, 0.0) },
        KeyCode::Right => ImageCommand::Pan { delta: (-4.0, 0.0) },
        KeyCode::Up | KeyCode::Char('k') => ImageCommand::Pan { delta: (0.0, 4.0) },
        KeyCode::Down | KeyCode::Char('j') => ImageCommand::Pan { delta: (0.0, -4.0) },
        KeyCode::PageUp => ImageCommand::Pan { delta: (0.0, 16.0) },
        KeyCode::PageDown => ImageCommand::Pan {
            delta: (0.0, -16.0),
        },
        _ => return None,
    })
}
#[cfg(test)]
pub fn draw_image(palette:&lenz::theme::Palette,doc: &ImageDoc, state: &ImageState, buf: &mut Buffer, area: Rect) {
    draw_image_ratio(palette,doc,state,buf,area,0.5);
}
fn draw_image_ratio(palette:&lenz::theme::Palette,doc:&ImageDoc,state:&ImageState,buf:&mut Buffer,area:Rect,ratio:f32) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let viewport = image_viewport_ratio(state, area,ratio);
    let zoom = state.zoom(doc, viewport);
    let center = state.center(doc);
    let stage = Rect::new(
        area.x,
        area.y + area.height.min(4),
        area.width,
        area.height.saturating_sub(4),
    );
    buf.set_style(
        area,
        crate::chrome::style_with(palette),
    );
    let selected = state
        .region()
        .map(|r| format!(" · region {}/{}", r + 1, doc.diff.regions.len()))
        .unwrap_or_default();
    safe_text(
        buf,
        Rect::new(area.x, area.y, area.width, 1),
        &format!(
            "{} · {:.0}% · {} {:.0}%{} · half-block raster",
            state.mode().name(),
            zoom * 100.0,
            if state.mode() == Mode::Onion {
                "opacity"
            } else {
                "split"
            },
            if state.mode() == Mode::Onion {
                state.alpha()
            } else {
                state.split()
            } * 100.0,
            selected
        ),
        crate::chrome::style_with(palette).fg(crate::chrome::color(palette.accent)),
    );
    for (i, side) in [Side::Left, Side::Right].into_iter().enumerate() {
        if i as u16 + 1 >= area.height {
            break;
        }
        safe_text(
            buf,
            Rect::new(area.x, area.y + i as u16 + 1, area.width, 1),
            &format!(
                "{}: {}",
                if i == 0 { "Old" } else { "New" },
                doc.info(side).unwrap_or_else(|| "(absent)".into())
            ),
            crate::chrome::style_with(palette).fg(crate::chrome::color(palette.dim)),
        );
    }
    if area.height > 3 {
        safe_text(
            buf,
            Rect::new(area.x, area.y + 3, area.width, 1),
            &format!(
                "{}{}",
                if doc.diff.changed == 0 {
                    "Identical · "
                } else {
                    ""
                },
                doc.notes().join(" · ")
            ),
            crate::chrome::style_with(palette).fg(crate::chrome::color(palette.accent)),
        );
    }
    let pane_w = crate::panes::left(stage.width,ratio);
    for y in 0..stage.height {
        for x in 0..stage.width {
            if state.mode() == Mode::TwoUp && x == pane_w {
                buf.set_string(
                    stage.x + x,
                    stage.y + y,
                    "│",
                    crate::chrome::style_with(palette).fg(crate::chrome::color(palette.border)),
                );
                continue;
            }
            let (side, local_x, width) = if state.mode() == Mode::TwoUp {
                if x < pane_w {
                    (0, x, pane_w)
                } else {
                    (
                        1,
                        x.saturating_sub(pane_w + 1),
                        stage.width.saturating_sub(pane_w + 1),
                    )
                }
            } else {
                (
                    usize::from(x as f32 >= stage.width as f32 * state.split()),
                    x,
                    stage.width,
                )
            };
            let ix = center.0 + (local_x as f32 + 0.5 - width as f32 / 2.0) / zoom;
            let iy = center.1 + (y as f32 * 2.0 + 0.5 - stage.height as f32) / zoom;
            let sample = |iy: f32| {
                let inside = match state.mode() {
                    Mode::TwoUp | Mode::Swipe => doc.sides[side].as_ref().is_some_and(|d| {
                        ix >= 0.0 && iy >= 0.0 && ix < d.width as f32 && iy < d.height as f32
                    }),
                    Mode::Onion | Mode::Difference => {
                        ix >= 0.0 && iy >= 0.0 && ix < doc.size.0 as f32 && iy < doc.size.1 as f32
                    }
                };
                if !inside {
                    return crate::chrome::color(palette.bg);
                }
                let pixel = match state.mode() {
                    Mode::TwoUp | Mode::Swipe => side_pixel(doc, side, ix, iy),
                    Mode::Onion => {
                        // The GUI blends the capped decode buffers directly; only
                        // the individual-side modes restore original dimensions.
                        let old = decoded_pixel(doc, 0, ix, iy);
                        let new = decoded_pixel(doc, 1, ix, iy);
                        let alpha = state.alpha();
                        std::array::from_fn(|i| {
                            (old[i] as f32 * (1.0 - alpha) + new[i] as f32 * alpha).round() as u8
                        })
                    }
                    Mode::Difference => doc.difference_pixel(ix as u32, iy as u32),
                };
                let selected_region = state.region().and_then(|i| doc.diff.regions.get(i));
                let outline = selected_region.is_some_and(|r| {
                    ix >= r.x as f32
                        && ix < (r.x + r.w) as f32
                        && iy >= r.y as f32
                        && iy < (r.y + r.h) as f32
                        && ((ix - r.x as f32).min((r.x + r.w) as f32 - ix) < 1.0 / zoom
                            || (iy - r.y as f32).min((r.y + r.h) as f32 - iy) < 1.0 / zoom)
                });
                if outline {
                    crate::chrome::color(palette.accent)
                } else {
                    composite(pixel, x, y)
                }
            };
            let top = sample(iy);
            let bottom = sample(iy + 1.0 / zoom);
            buf.set_string(
                stage.x + x,
                stage.y + y,
                "▀",
                Style::default().fg(top).bg(bottom),
            );
        }
    }
    if state.mode() == Mode::TwoUp && stage.height > 0 {
        for side in 0..2 {
            if doc.sides[side].is_none() {
                let x = if side == 0 {
                    stage.x
                } else {
                    stage.x + pane_w + 1
                };
                let width = if side == 0 {
                    pane_w
                } else {
                    stage.width.saturating_sub(pane_w + 1)
                };
                safe_text(
                    buf,
                    Rect::new(x, stage.y + stage.height / 2, width, 1),
                    "(absent)",
                    crate::chrome::style_with(palette).fg(crate::chrome::color(palette.dim)),
                );
            }
        }
    }
}

fn side_pixel(doc: &ImageDoc, side: usize, x: f32, y: f32) -> [u8; 4] {
    let Some(d) = &doc.sides[side] else {
        return [0; 4];
    };
    let scale = if d.downscaled {
        d.pixels.width() as f32 / d.width as f32
    } else {
        1.0
    };
    // Shared decoder keeps the original metadata while its pixels may be capped.
    pixel_at(&d.pixels, x * scale, y * scale)
}
fn decoded_pixel(doc: &ImageDoc, side: usize, x: f32, y: f32) -> [u8; 4] {
    doc.sides[side]
        .as_ref()
        .map_or([0; 4], |d| pixel_at(&d.pixels, x, y))
}
// Image buffers are inferred through the shared type, so the terminal adds no image dependency.
fn pixel_at(pixels: &lenz::image_session::Pixels, x: f32, y: f32) -> [u8; 4] {
    if x < 0.0 || y < 0.0 {
        return [0; 4];
    }
    pixels
        .get_pixel_checked(x as u32, y as u32)
        .map_or([0; 4], |p| p.0)
}
fn composite(p: [u8; 4], x: u16, y: u16) -> Color {
    let checker = if (x / 3 + y / 2).is_multiple_of(2) {
        58
    } else {
        74
    };
    let alpha = p[3] as u32;
    let rgb: [u8; 3] =
        std::array::from_fn(|i| ((p[i] as u32 * alpha + checker * (255 - alpha)) / 255) as u8);
    Color::Rgb(rgb[0], rgb[1], rgb[2])
}

/// Maps every shared capture; diff backgrounds and match overlays stay with the text renderer.
pub fn syntax_style(palette:&lenz::theme::Palette,capture: Capture) -> Style {
    use Capture::*;
    let style = Style::default().fg(crate::chrome::color(palette.syntax_color(capture)));
    match capture {
        Comment | DocComment => style.add_modifier(Modifier::ITALIC),
        Heading => style.add_modifier(Modifier::BOLD),
        Link => style.add_modifier(Modifier::UNDERLINED),
        _ => style,
    }
}
pub fn safe_text(buf: &mut Buffer, area: Rect, text: &str, style: Style) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    // Ratatui already splits graphemes and checks cell widths. Avoid doing both
    // twice for ordinary labels; keep the explicit replacement/zero-width path.
    if text.chars().all(|c|!lenz::document::hides_text(c)&&unicode_width::UnicodeWidthChar::width(c).is_some_and(|w|w>0)) {
        buf.set_stringn(area.x,area.y,text,usize::from(area.width),style);return;
    }
    let mut used = 0;
    for grapheme in text.graphemes(true) {
        let capped = lenz::document::cap_marks(grapheme);
        let display = if grapheme.chars().any(lenz::document::hides_text) {
            "�"
        } else {
            capped.as_ref()
        };
        let width = display.width().max(1);
        if used + width > area.width as usize {
            break;
        }
        buf.set_stringn(area.x + used as u16, area.y, display, width, style);
        used += width;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn image_visual() -> Visual {
        let old = br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#ff0000"/></svg>"##;
        let new = br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#0000ff"/></svg>"##;
        Visual {
            image: Some(Arc::new(
                ImageDoc::read(
                    (old, Path::new("a.svg"), false),
                    (new, Path::new("b.svg"), false),
                )
                .unwrap()
                .unwrap(),
            )),
            ..Default::default()
        }
    }
    #[test]
    fn pane_image_two_up_geometry_and_fit_use_the_unequal_split() {
        let mut visual=image_visual();let area=Rect::new(0,0,120,20);visual.pane_ratio=Some(0.7);
        let left=crate::panes::left(area.width,0.7);let viewport=visual.viewport(area);assert_eq!(viewport.width,f32::from(area.width-left-1));
        let mut buf=Buffer::empty(area);visual.draw_image(&mut buf,area,crate::chrome::palette());
        assert_eq!(buf[(left,8)].symbol(),"│");assert_ne!(buf[(area.width/2,8)].symbol(),"│");
        visual.image_key(KeyEvent::new(KeyCode::Char('0'),KeyModifiers::NONE),area);assert_eq!(visual.viewport(area).width,viewport.width);
    }
    #[test]
    fn real_image_keys_drive_modes_opacity_zoom_pan_and_regions() {
        let mut visual = image_visual();
        let area = Rect::new(0, 0, 40, 16);
        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        for mode in [Mode::Swipe, Mode::Onion, Mode::Difference, Mode::TwoUp] {
            assert!(visual.image_key(key(KeyCode::Char('m')), area));
            assert_eq!(visual.image_state().mode(), mode);
        }
        visual.image_key(key(KeyCode::Char('1')), area);
        assert_eq!(
            visual
                .image_state()
                .zoom(visual.image().unwrap(), visual.viewport(area)),
            1.0
        );
        visual.image_key(key(KeyCode::Right), area);
        assert_eq!(
            visual.image_state().center(visual.image().unwrap()),
            (8.0, 4.0)
        );
        visual.image_key(key(KeyCode::Char('n')), area);
        assert_eq!(visual.image_state().region(), Some(0));
        visual.image_key(key(KeyCode::Char('0')), area);
        assert_eq!(visual.image_state().region(), None);
        assert!(!visual.image_key(key(KeyCode::Char('q')), area));
    }
    #[test]
    fn terminal_image_samples_both_sides_and_blend_endpoints() {
        let mut visual = image_visual();
        let area = Rect::new(0, 0, 40, 16);
        let mut buf = Buffer::empty(area);
        visual.draw_image(&mut buf, area,crate::chrome::palette());
        let foregrounds = |buf: &Buffer| {
            buf.content
                .iter()
                .filter(|cell| cell.symbol() == "▀")
                .map(|cell| cell.fg)
                .collect::<Vec<_>>()
        };
        let colors = foregrounds(&buf);
        assert!(colors.contains(&Color::Rgb(255, 0, 0)));
        assert!(colors.contains(&Color::Rgb(0, 0, 255)));
        let key = |code| KeyEvent::new(KeyCode::Char(code), KeyModifiers::NONE);
        visual.image_key(key('m'), area);
        visual.image_key(key('m'), area);
        visual.image_key(key('1'), area);
        for _ in 0..6 {
            visual.image_key(key(','), area);
        }
        visual.draw_image(&mut buf, area,crate::chrome::palette());
        assert!(foregrounds(&buf).contains(&Color::Rgb(255, 0, 0)));
        for _ in 0..11 {
            visual.image_key(key('.'), area);
        }
        visual.draw_image(&mut buf, area,crate::chrome::palette());
        assert!(foregrounds(&buf).contains(&Color::Rgb(0, 0, 255)));
        // Difference, drawn from both Sides on demand: every pixel changed, in magenta
        visual.image_key(key('m'), area);
        visual.draw_image(&mut buf, area,crate::chrome::palette());
        assert!(foregrounds(&buf).contains(&Color::Rgb(255, 40, 200)));
    }
    #[test]
    fn capped_images_use_original_size_only_for_individual_side_modes() {
        let mut visual = image_visual();
        let mut doc = Arc::try_unwrap(visual.image.take().unwrap()).ok().unwrap();
        let old = doc.sides[0].as_mut().unwrap();
        old.width = 16;
        old.height = 16;
        old.downscaled = true;
        for x in 4..8 {
            for y in 0..8 {
                old.pixels.get_pixel_mut(x, y).0 = [0, 255, 0, 255];
            }
        }
        assert_eq!(side_pixel(&doc, 0, 6.0, 2.0), [255, 0, 0, 255]);
        assert_eq!(decoded_pixel(&doc, 0, 6.0, 2.0), [0, 255, 0, 255]);
        assert!(doc.notes().iter().any(|note| note.contains("downscaled")));
        visual.image = Some(Arc::new(doc));
        let area = Rect::new(0, 0, 16, 8);
        visual.image_execute(ImageCommand::SetMode(Mode::Onion), area);
        visual.image_execute(ImageCommand::ActualSize, area);
        visual.image_execute(ImageCommand::Nudge(-0.5), area);
        let mut buf = Buffer::empty(area);
        visual.draw_image(&mut buf, area,crate::chrome::palette());
        assert!(
            buf.content
                .iter()
                .any(|cell| cell.symbol() == "▀" && cell.fg == Color::Rgb(0, 255, 0))
        );
    }
    #[test]
    fn commit_popup_keys_preserve_hash_path_and_one_based_original_line() {
        let root = tempfile::tempdir().unwrap();
        let sha = "a".repeat(40);
        let mut visual = Visual {
            selected_blame: Some(BlameLine {
                commit: Arc::new(lenz::blame::Commit {
                    sha: sha.clone(),
                    ..Default::default()
                }),
                original_line: 42,
                original_path: "renamed/λ.rs".into(),
            }),
            ..Default::default()
        };
        visual.inspection.open(root.path().into(), sha.clone());
        let key = |code| KeyEvent::new(KeyCode::Char(code), KeyModifiers::NONE);
        assert!(matches!(visual.commit_key(key('c')), Some(BlameEffect::Copy(s)) if s == sha));
        assert!(
            matches!(visual.commit_key(key('o')), Some(BlameEffect::OpenCommit(s)) if s == sha)
        );
        assert!(
            matches!(visual.commit_key(key('d')), Some(BlameEffect::CompareCommit(s)) if s == sha)
        );
        assert!(
            matches!(visual.commit_key(key('p')), Some(BlameEffect::AnnotateParent { sha: s, path, line: 42 }) if s == sha && path == "renamed/λ.rs")
        );
        let area = Rect::new(0, 0, 100, 25);
        let mut buf = Buffer::empty(area);
        visual.draw_commit(&mut buf, area,crate::chrome::palette());
        let screen = buf
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(screen.contains(&format!("Commit {sha}")));
        assert!(screen.contains("p annotate parent"));
        visual.commit_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!visual.commit_visible());
        assert!(!visual.busy());
    }
    #[test]
    fn open_commit_overlay_survives_zero_and_narrow_terminal_resizes() {
        use ratatui::{Terminal, backend::TestBackend};
        let root = tempfile::tempdir().unwrap();
        let mut visual = Visual::default();
        visual.inspection.open(root.path().into(), "a".repeat(40));
        assert!(visual.commit_visible());
        for width in 0..=6 {
            for height in 0..=6 {
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal.draw(|frame| {
                    let area = frame.area();
                    visual.draw_commit(frame.buffer_mut(), area,crate::chrome::palette());
                }).unwrap();
            }
        }
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        terminal.draw(|frame| {
            let area = frame.area();
            visual.draw_commit(frame.buffer_mut(), area,crate::chrome::palette());
        }).unwrap();
        assert!(terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect::<String>().contains("Commit"));
        visual.commit_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!visual.commit_visible());
    }
    #[test]
    fn absent_side_and_identical_images_have_observable_metadata() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="#f00"/></svg>"##;
        let absent = ImageDoc::read(
            (b"", Path::new("old.svg"), false),
            (svg, Path::new("new.svg"), false),
        )
        .unwrap()
        .unwrap();
        let identical = ImageDoc::read(
            (svg, Path::new("old.svg"), false),
            (svg, Path::new("new.svg"), false),
        )
        .unwrap()
        .unwrap();
        let state = ImageState::default();
        let area = Rect::new(0, 0, 80, 16);
        let mut buf = Buffer::empty(area);
        draw_image(crate::chrome::palette(),&absent, &state, &mut buf, area);
        let screen = buf
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(screen.contains("Old: (absent)"));
        assert!(screen.contains("4 px changed"));
        buf.reset();
        draw_image(crate::chrome::palette(),&identical, &state, &mut buf, area);
        let screen = buf
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(screen.contains("Identical"));
        assert!(screen.contains("0 px changed"));
    }

    #[test]
    fn cell_text_is_grapheme_safe_and_never_emits_terminal_controls() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 8, 2));
        safe_text(&mut buf, Rect::new(0, 0, 3, 1), "é界Z", Style::default());
        assert_eq!(buf[(0, 0)].symbol(), "é");
        assert_eq!(buf[(1, 0)].symbol(), "界");
        assert_eq!(buf[(3, 0)].symbol(), " ");
        let mut unicode=Buffer::empty(Rect::new(0,0,8,1));
        safe_text(&mut unicode,Rect::new(0,0,5,1),"a\u{301}👩‍💻界Z",Style::default());
        assert_eq!(unicode[(0,0)].symbol(),"a\u{301}");
        assert_eq!(unicode[(1,0)].symbol(),"👩‍💻");
        assert_eq!(unicode[(3,0)].symbol(),"界");
        assert_eq!(unicode[(5,0)].symbol()," ");
        let mut detached=Buffer::empty(Rect::new(0,0,4,1));
        safe_text(&mut detached,Rect::new(0,0,4,1),"\u{301}abc",Style::default());
        assert_eq!(detached[(0,0)].symbol()," ");assert_eq!(detached[(1,0)].symbol(),"a");

        safe_text(
            &mut buf,
            Rect::new(0, 1, 8, 1),
            "a\u{1b}[31m",
            Style::default(),
        );
        assert_eq!(buf[(1, 1)].symbol(), "�");
        assert!(
            buf.content
                .iter()
                .all(|cell| !cell.symbol().chars().any(char::is_control))
        );
    }
    #[test]
    fn prepared_image_shares_pixels_and_never_rereads_sources() {
        let dir = tempfile::tempdir().unwrap();
        let left = dir.path().join("old.svg");
        let right = dir.path().join("new.svg");
        std::fs::write(&left, b"").unwrap();
        std::fs::write(&right,br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#f00"/></svg>"##).unwrap();
        let spec = Spec::Plain {
            left: left.clone(),
            right: right.clone(),
        };
        let set = Arc::new(ChangeSet::load(&spec, dir.path()).unwrap());
        let prepared =
            lenz::document::PreparedDocument::load(&set, 0, &left, &right, false).unwrap();
        let image = Arc::new(prepared.image.unwrap().unwrap());
        let doc = Arc::new(prepared.text);
        std::fs::remove_file(&left).unwrap();
        std::fs::remove_file(&right).unwrap();
        let mut visual = Visual::default();
        visual.load_prepared(set, 0, doc, &spec, Some(Ok(image.clone())));
        assert!(!visual.busy());
        assert!(Arc::ptr_eq(visual.image.as_ref().unwrap(), &image));
        assert_eq!(visual.image().unwrap().diff.changed, 64);
        assert!(visual.notice().is_none());
    }
    #[test]
    fn prepared_nonimage_binary_skips_image_jobs_and_retains_placeholder() {
        let dir = tempfile::tempdir().unwrap();
        let left = dir.path().join("old.bin");
        let right = dir.path().join("new.bin");
        std::fs::write(&left, [0, 1, 2]).unwrap();
        std::fs::write(&right, [0, 3, 2]).unwrap();
        let spec = Spec::Plain {
            left: left.clone(),
            right: right.clone(),
        };
        let set = Arc::new(ChangeSet::load(&spec, dir.path()).unwrap());
        let prepared =
            lenz::document::PreparedDocument::load(&set, 0, &left, &right, false).unwrap();
        assert!(prepared.image.is_none());
        assert!(matches!(
            prepared.text.placeholder_kind(),
            Some(Placeholder::Binary { .. })
        ));
        std::fs::remove_file(&left).unwrap();
        std::fs::remove_file(&right).unwrap();
        let mut visual = Visual::default();
        visual.load_prepared(set, 0, Arc::new(prepared.text), &spec, None);
        assert!(!visual.busy());
        assert!(visual.image().is_none());
        assert!(visual.notice().is_none());
    }

    #[test]
    fn superseded_image_cannot_replace_active_text_and_syntax_is_compact() {
        let dir = tempfile::tempdir().unwrap();
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#f00"/></svg>"##;
        let open = |name: &str, bytes: &[u8]| {
            let left = dir.path().join(format!("old-{name}"));
            let right = dir.path().join(format!("new-{name}"));
            std::fs::write(&left, b"").unwrap();
            std::fs::write(&right, bytes).unwrap();
            let spec = Spec::Plain {
                left: left.clone(),
                right: right.clone(),
            };
            let set = Arc::new(ChangeSet::load(&spec, dir.path()).unwrap());
            let doc = Arc::new(TextDocument::load(&set, 0, &left, &right, false).unwrap());
            (set, doc, spec)
        };
        let mut visual = Visual::default();
        let (set, doc, spec) = open("image.svg", svg);
        visual.load(set, 0, doc, &spec);
        let (set, doc, spec) = open("code.rs", b"fn main() { let x = 42; }\n");
        visual.load(set, 0, doc, &spec);
        let started = std::time::Instant::now();
        while visual.busy() {
            visual.poll();
            assert!(started.elapsed().as_secs() < 10);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(visual.image().is_none());
        let spans = visual.syntax(Side::Right, 0);
        assert!(
            spans
                .iter()
                .any(|(range, capture)| *range == (0..2) && *capture == Capture::Keyword)
        );
        assert!(visual.syntax(Side::Right, 100).is_empty());
    }

    /// Sampling stays in the test target. Linux uses its kB RSS field rather than
    /// spawning a process on each pending-job poll; other systems keep the portable probe.
    fn sample_rss() -> Option<f64> {
        #[cfg(target_os = "linux")]
        if let Ok(status) = std::fs::read_to_string("/proc/self/status")
            && let Some(kb) = status.lines().find_map(|line| {
                line.strip_prefix("VmRSS:")
                    .and_then(|value| value.split_whitespace().next())
                    .and_then(|value| value.parse::<f64>().ok())
            })
        {
            return Some(kb / 1024.0);
        }
        crate::perf::rss_mib()
    }

    #[test]
    #[ignore = "release image and syntax terminal performance workload"]
    fn terminal_perf_images_syntax() {
        use crate::perf;
        let old = br##"<svg xmlns="http://www.w3.org/2000/svg" width="4096" height="4096"><rect width="4096" height="4096" fill="#aa2233"/></svg>"##;
        let new = br##"<svg xmlns="http://www.w3.org/2000/svg" width="4096" height="4096"><rect width="4096" height="4096" fill="#aa2233"/><rect x="512" y="512" width="1024" height="1024" fill="#3366dd"/></svg>"##;
        let started = std::time::Instant::now();
        let doc = ImageDoc::read(
            (old, Path::new("old.svg"), false),
            (new, Path::new("new.svg"), false),
        )
        .unwrap()
        .unwrap();
        perf::record(
            "tui/images/decode-diff-4k",
            "ms",
            &[started.elapsed().as_secs_f64() * 1000.0],
        );
        assert_eq!(doc.diff.changed, 1024 * 1024);
        let mut state = ImageState::default();
        let area = Rect::new(0, 0, 160, 50);
        let mut buf = Buffer::empty(area);
        let before = sample_rss();
        for mode in [Mode::TwoUp, Mode::Swipe, Mode::Onion, Mode::Difference] {
            let viewport = image_viewport(&state, area);
            state.execute(&doc, ImageCommand::SetMode(mode), viewport);
            let mut samples = Vec::new();
            for _ in 0..80 {
                let started = std::time::Instant::now();
                draw_image(crate::chrome::palette(),&doc, &state, &mut buf, area);
                samples.push(started.elapsed().as_secs_f64() * 1000.0);
            }
            perf::record(
                &format!("tui/images/{}/render", mode.name()),
                "ms",
                &samples,
            );
        }
        let mut resized = Vec::new();
        for (width, height) in [(80, 24), (200, 60), (30, 10)] {
            let area = Rect::new(0, 0, width, height);
            let mut buf = Buffer::empty(area);
            let started = std::time::Instant::now();
            draw_image(crate::chrome::palette(),&doc, &state, &mut buf, area);
            resized.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        perf::record("tui/images/resize", "ms", &resized);
        if let (Some(before), Some(after)) = (before, sample_rss()) {
            perf::record("tui/images/mode-cycle-resident-before", "MiB", &[before]);
            perf::record("tui/images/mode-cycle-resident-after", "MiB", &[after]);
        }
        let text: Arc<str> = (0..100_000)
            .map(|i| format!("fn item_{i}() {{ let text = \"café\"; }}\n"))
            .collect::<String>()
            .into();
        let parsed = highlight::parse_while(highlight::Language::Rust, text, || true).unwrap();
        let mut samples = Vec::new();
        for line in (0..99_900).step_by(997) {
            let started = std::time::Instant::now();
            let spans = parsed.spans(line..line + 50);
            for line in spans {
                for (_, capture) in line {
                    std::hint::black_box(syntax_style(crate::chrome::palette(),capture));
                }
            }
            samples.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        perf::record("tui/syntax/visible-50-rows", "ms", &samples);
        perf::record(
            "tui/syntax/retained-spans",
            "bytes",
            &[parsed.bytes() as f64],
        );
    }

    #[test]
    #[ignore = "release repeated image cancellation and two-side annotation lifecycle"]
    fn terminal_perf_visual_lifecycle() {
        use crate::perf;
        let dir = tempfile::tempdir().unwrap();
        let left_path = dir.path().join("old.svg");
        let right_path = dir.path().join("new.svg");
        let old = br##"<svg xmlns="http://www.w3.org/2000/svg" width="4096" height="4096"><rect width="4096" height="4096" fill="#aa2233"/></svg>"##;
        let new = br##"<svg xmlns="http://www.w3.org/2000/svg" width="4096" height="4096"><rect width="4096" height="4096" fill="#2266dd"/></svg>"##;
        std::fs::write(&left_path, old).unwrap();
        std::fs::write(&right_path, new).unwrap();
        let spec = Spec::Plain {
            left: left_path.clone(),
            right: right_path.clone(),
        };
        let set = Arc::new(ChangeSet::load(&spec, dir.path()).unwrap());
        let doc = Arc::new(TextDocument::load(&set, 0, &left_path, &right_path, false).unwrap());
        let mut visual = Visual::default();
        let mut times = Vec::new();
        let mut peak = sample_rss();
        let area = Rect::new(0, 0, 160, 50);
        let mut buf = Buffer::empty(area);
        for _ in 0..12 {
            let started = std::time::Instant::now();
            for _ in 0..5 {
                visual.load(set.clone(), 0, doc.clone(), &spec);
            }
            while visual.busy() {
                visual.poll();
                if let Some(rss) = sample_rss() {
                    peak = Some(peak.unwrap_or(rss).max(rss));
                }
                assert!(started.elapsed().as_secs() < 30);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            assert_eq!(visual.image().unwrap().diff.changed, 4096 * 4096);
            visual.draw_image(&mut buf, area,crate::chrome::palette());
            times.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        perf::record("tui/images/repeated-load-cancel", "ms", &times);
        let settled = sample_rss();
        if let Some(rss) = settled {
            peak = Some(peak.unwrap_or(rss).max(rss));
        }
        if let Some(peak) = peak {
            perf::record("tui/images/load-cancel-sampled-peak", "MiB", &[peak]);
        }
        if let Some(rss) = settled {
            perf::record("tui/images/load-cancel-settled", "MiB", &[rss]);
        }
        visual.clear();
        let repo = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .current_dir(repo.path())
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .args([
                    "-c",
                    "init.defaultBranch=main",
                    "-c",
                    "core.hooksPath=/dev/null",
                    "-c",
                    "commit.gpgsign=false",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init", "-q"]);
        git(&["config", "user.name", "Fixture"]);
        git(&["config", "user.email", "fixture@example.test"]);
        let mut lines = (0..1000)
            .map(|i| format!("fn item_{i}() {{ let value = {i}; }}"))
            .collect::<Vec<_>>();
        std::fs::write(repo.path().join("a.rs"), lines.join("\n") + "\n").unwrap();
        git(&["add", "a.rs"]);
        git(&["commit", "-qm", "base"]);
        for revision in 1..=10 {
            lines[revision * 13] = format!(
                "fn item_{}() {{ let changed = {revision}; }}",
                revision * 13
            );
            std::fs::write(repo.path().join("a.rs"), lines.join("\n") + "\n").unwrap();
            git(&["add", "a.rs"]);
            git(&["commit", "-qm", &format!("update{revision}")]);
        }
        let spec = Spec::Git {
            cached: false,
            revs: vec!["HEAD~".into(), "HEAD".into()],
            paths: vec![],
            untracked: true,
        };
        let set = Arc::new(ChangeSet::load(&spec, repo.path()).unwrap());
        let index = set
            .comparisons
            .iter()
            .position(|c| c.path == "a.rs")
            .unwrap();
        let doc = Arc::new(
            TextDocument::load(&set, index, Path::new("a.rs"), Path::new("a.rs"), false).unwrap(),
        );
        let mut times = Vec::new();
        let mut peak = sample_rss();
        for _ in 0..30 {
            let started = std::time::Instant::now();
            // Half these generations are abandoned with both annotation jobs already queued.
            for _ in 0..2 {
                visual.load(set.clone(), index, doc.clone(), &spec);
                visual.toggle_blame(Side::Left);
                visual.toggle_blame(Side::Right);
            }
            while visual.busy() {
                visual.poll();
                if let Some(rss) = sample_rss() {
                    peak = Some(peak.unwrap_or(rss).max(rss));
                }
                assert!(started.elapsed().as_secs() < 30);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            assert_eq!(
                visual.annotation(Side::Right, 130).unwrap().commit.summary,
                "update10"
            );
            assert_eq!(
                visual.annotation(Side::Left, 130).unwrap().commit.summary,
                "base"
            );
            assert!(!visual.syntax(Side::Right, 130).is_empty());
            times.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        perf::record("tui/blame/two-side-load-cancel", "ms", &times);
        let settled = sample_rss();
        if let Some(rss) = settled {
            peak = Some(peak.unwrap_or(rss).max(rss));
        }
        if let Some(peak) = peak {
            perf::record("tui/blame/load-cancel-sampled-peak", "MiB", &[peak]);
        }
        if let Some(rss) = settled {
            perf::record("tui/blame/load-cancel-settled", "MiB", &[rss]);
        }
        visual.clear();
        assert!(!visual.busy());
    }

    #[test]
    fn every_shared_syntax_class_has_a_foreground() {
        for capture in [
            Capture::Keyword,
            Capture::String,
            Capture::Escape,
            Capture::Comment,
            Capture::DocComment,
            Capture::Number,
            Capture::Constant,
            Capture::ConstantBuiltin,
            Capture::Function,
            Capture::Macro,
            Capture::Type,
            Capture::TypeBuiltin,
            Capture::Variable,
            Capture::VariableBuiltin,
            Capture::Parameter,
            Capture::Property,
            Capture::Operator,
            Capture::Punctuation,
            Capture::Tag,
            Capture::Attribute,
            Capture::Label,
            Capture::Module,
            Capture::Heading,
            Capture::Link,
        ] {
            let syntax=syntax_style(crate::chrome::palette(),capture);
            assert_eq!(syntax.fg,Some(crate::chrome::color(crate::chrome::palette().syntax_color(capture))),"{capture:?}");
            let diff_background=Color::Rgb(11,22,33);
            assert_eq!(Style::default().bg(diff_background).patch(syntax).bg,Some(diff_background),"{capture:?}");
        }
    }
}
