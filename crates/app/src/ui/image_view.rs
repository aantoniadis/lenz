//! Image Comparisons: both Sides decoded, shown in the Side panes (2-up) or across them (Swipe, Onion skin,
//! Difference), with synced zoom and pan and Changed Regions to step through.

use gpui::RenderImage;
use image::RgbaImage;

use super::*;
pub(super) use crate::image_session::Mode;
use crate::image_session::{ImageCommand, ImageState, Viewport};
use crate::images::{self, Decoded};

/// GPUI resources stay in the adapter; decoded state belongs to the shared core.
pub(super) struct ImageDoc {
    core: crate::image_session::ImageDoc,
    renders: [Option<Arc<RenderImage>>; 2],
    diff_render: std::sync::OnceLock<Arc<RenderImage>>,
}
impl std::ops::Deref for ImageDoc {
    type Target = crate::image_session::ImageDoc;
    fn deref(&self) -> &Self::Target {
        &self.core
    }
}
use lenz_gpui::image::{bgra_texture as bgra, checker, texture as render};
impl ImageDoc {
    pub fn from_core(core: crate::image_session::ImageDoc) -> Self {
        let renders = [
            core.sides[0].as_ref().map(|d| render(d.pixels.clone())),
            core.sides[1].as_ref().map(|d| render(d.pixels.clone())),
        ];
        Self {
            core,
            renders,
            diff_render: Default::default(),
        }
    }
}

/// Zoom, pan and mode of the image Comparison on screen.
#[derive(Debug, Clone, Default)]
pub(super) struct ImageView {
    state: ImageState,
    /// The onion-skin blend for its opacity, held only as the texture (BGRA).
    blend: Option<(f32, Arc<RenderImage>)>,
    /// Sharp enlargements of the visible part above 300 %: (source, factor, rect) → image.
    crisp: Vec<((usize, u32, images::Rect), Arc<RenderImage>)>,
    pub drag: Option<(f32, f32)>,
}

impl ImageView {
    /// A new image on screen: 2-up, fitted, opacity and split at the middle.
    pub fn state(&self) -> &ImageState {
        &self.state
    }
    pub fn mode(&self) -> Mode {
        self.state.mode()
    }
    pub fn command(&mut self, doc: &ImageDoc, command: ImageCommand, viewport: Viewport) {
        self.state.execute(doc, command, viewport);
    }
    pub fn fresh() -> Self {
        ImageView::default()
    }
}

impl Viewer {
    pub(super) fn image_doc(&self) -> Option<Arc<ImageDoc>> {
        self.doc().and_then(|d| d.image.clone())
    }

    /// The stage: the panes' area (both Sides and the gutter) in window coordinates.
    fn stage(&self, window: &Window) -> (f32, f32, f32, f32) {
        let x0 = self.diff_x();
        let w = (f32::from(window.viewport_size().width) - x0).max(0.0);
        let h = (self.content_bottom() - self.top() - self.panel_h() - self.picker_v_height()).max(0.0);
        (x0, self.top(), w, h)
    }

    pub(super) fn image_viewport(&self, window: &Window) -> Viewport {
        let (_, _, w, h) = self.stage(window);
        Viewport {
            width: if self.iview.state.mode() == Mode::TwoUp {
                self.comparison_widths(w).into_iter().fold(f32::INFINITY,f32::min)
            } else {
                w
            },
            height: h,
        }
    }

    pub(super) fn image_zoom(&self, img: &ImageDoc, window: &Window) -> f32 {
        self.iview.state.zoom(img, self.image_viewport(window))
    }

    pub(super) fn zoom_image_at(&mut self, factor: f32, sx: f32, sy: f32, window: &Window) {
        let Some(img) = self.image_doc() else { return };
        let (cx, cy) = self.pane_center(sx, window);
        let viewport = self.image_viewport(window);
        self.iview.state.execute(
            &img,
            ImageCommand::Zoom {
                factor,
                offset: (sx - cx, sy - cy),
            },
            viewport,
        );
    }

    /// The centre of the pane (or stage, in overlay modes) holding window x `sx`.
    fn pane_center(&self, sx: f32, window: &Window) -> (f32, f32) {
        let (x0, y0, w, h) = self.stage(window);
        if self.iview.state.mode() != Mode::TwoUp {
            return (x0 + w / 2.0, y0 + h / 2.0);
        }
        let widths=self.comparison_widths(w);
        let pane_w=widths[0];
        if sx < x0 + pane_w {
            (x0 + pane_w / 2.0, y0 + h / 2.0)
        } else {
            (x0 + pane_w + GUTTER_W + widths[1] / 2.0, y0 + h / 2.0)
        }
    }

    /// Image keys: `m` mode, `0` fit, `1` 1:1, `+`/`-` zoom, `,`/`.` split or opacity, `n`/`p` Changed Regions.
    pub(super) fn image_key(&mut self, key: &str, window: &Window, cx: &mut Context<Self>) -> bool {
        let Some(img) = self.image_doc() else {
            return false;
        };
        let (x0, y0, w, h) = self.stage(window);
        let middle = (x0 + w / 4.0, y0 + h / 2.0);
        let viewport = self.image_viewport(window);
        let command = match key {
            "m" => ImageCommand::CycleMode,
            "0" => ImageCommand::Fit,
            "1" => ImageCommand::ActualSize,
            "+" | "=" => {
                self.zoom_image_at(1.5, middle.0, middle.1, window);
                cx.notify();
                return true;
            }
            "-" => {
                self.zoom_image_at(1.0 / 1.5, middle.0, middle.1, window);
                cx.notify();
                return true;
            }
            "," => ImageCommand::Nudge(-0.1),
            "." => ImageCommand::Nudge(0.1),
            "n" | "p" => ImageCommand::Region {
                forward: key == "n",
            },
            _ => return false,
        };
        let moved = self.iview.state.execute(&img, command, viewport);
        if key == "m" {
            self.toast(
                format!("Image mode: {}", self.iview.state.mode().name()),
                cx,
            );
        }
        if matches!(key, "n" | "p") {
            let message = if moved {
                format!(
                    "Changed region {}/{}",
                    self.iview.state.region().unwrap() + 1,
                    img.diff.regions.len()
                )
            } else {
                "No changed regions".into()
            };
            self.toast(message, cx);
        }
        cx.notify();
        true
    }

    /// Drag pans; returns whether the pointer event belonged to the image stage.
    pub(super) fn image_drag(
        &mut self,
        x: f32,
        y: f32,
        pressed: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(img) = self.image_doc() else {
            return false;
        };
        let (x0, y0, w, h) = self.stage(window);
        let inside = x >= x0 && y >= y0 && x < x0 + w && y < y0 + h;
        if self.iview.state.mode() == Mode::Swipe && inside && !pressed {
            let viewport = self.image_viewport(window);
            self.iview
                .state
                .execute(&img, ImageCommand::SetSplit((x - x0) / w), viewport);
            cx.notify();
        }
        match (self.iview.drag, pressed) {
            (Some((lx, ly)), true) => {
                let viewport = self.image_viewport(window);
                self.iview.state.execute(
                    &img,
                    ImageCommand::Pan {
                        delta: (x - lx, y - ly),
                    },
                    viewport,
                );
                self.iview.drag = Some((x, y));
                cx.notify();
                true
            }
            (None, true) if inside => {
                self.iview.drag = Some((x, y));
                true
            }
            _ => {
                self.iview.drag = None;
                inside
            }
        }
    }

    /// What to paint, prepared on the UI thread (the onion-skin blend is cached per opacity).
    pub(super) fn image_frame(&mut self, window: &mut Window) -> Option<ImageFrame> {
        let Some(img) = self.image_doc() else {
            self.keep_image_uploads(Vec::new(), window);
            return None;
        };
        let difference = (self.iview.state.mode() == Mode::Difference).then(|| {
            img.diff_render
                .get_or_init(|| {
                    let (w, h) = img.size;
                    render(img.difference(images::Rect { x: 0, y: 0, w, h }))
                })
                .clone()
        });
        let zoom = self.image_zoom(&img, window);
        let center = self.iview.state.center(&img);
        if self.iview.state.mode() == Mode::Onion
            && self
                .iview
                .blend
                .as_ref()
                .is_none_or(|b| b.0 != self.iview.state.alpha())
        {
            let none = RgbaImage::new(0, 0);
            let px = |i: usize| img.sides[i].as_ref().map_or(&none, |d| &d.pixels);
            let blend = render(images::blend(px(0), px(1), self.iview.state.alpha()));
            self.iview.blend = Some((self.iview.state.alpha(), blend));
        }
        // above 300 %, the visible part enlarged pixel by pixel (gpui would smooth it)
        let mut crisp: [Option<(images::Rect, Arc<RenderImage>)>; 4] = Default::default();
        let mut keep = Vec::new();
        if zoom >= 3.0 {
            let (_, _, w, h) = self.stage(window);
            let pane_w = if self.iview.state.mode() == Mode::TwoUp {
                self.comparison_widths(w).into_iter().fold(0.0,f32::max)
            } else {
                w
            };
            let factor = (zoom.floor() as u32).min(16);
            let sources: &[usize] = match self.iview.state.mode() {
                Mode::TwoUp | Mode::Swipe => &[0, 1],
                Mode::Onion => &[3],
                Mode::Difference => &[2],
            };
            for &src in sources {
                let dims = match src {
                    0 | 1 => img.sides[src]
                        .as_ref()
                        .filter(|d| !d.downscaled)
                        .map(|d| d.pixels.dimensions()),
                    2 => Some(img.size),
                    _ => self.iview.blend.as_ref().map(|b| {
                        let s = b.1.size(0);
                        (s.width.0 as u32, s.height.0 as u32)
                    }),
                };
                let Some((iw, ih)) = dims else { continue };
                let (x0, x1) = (
                    (center.0 - pane_w / 2.0 / zoom).floor().max(0.0),
                    (center.0 + pane_w / 2.0 / zoom).ceil().min(iw as f32),
                );
                let (y0, y1) = (
                    (center.1 - h / 2.0 / zoom).floor().max(0.0),
                    (center.1 + h / 2.0 / zoom).ceil().min(ih as f32),
                );
                if x1 <= x0 || y1 <= y0 {
                    continue;
                }
                let rect = images::Rect {
                    x: x0 as u32,
                    y: y0 as u32,
                    w: (x1 - x0) as u32,
                    h: (y1 - y0) as u32,
                };
                let key = (src, factor, rect);
                let r = match self.iview.crisp.iter().find(|(k, _)| *k == key) {
                    Some((_, r)) => r.clone(),
                    None => match src {
                        0 | 1 => render(images::upscale_nearest(&img.sides[src].as_ref().unwrap().pixels, rect, factor)),
                        2 => {
                            let part = img.difference(rect);
                            render(images::upscale_nearest(&part, images::Rect { x: 0, y: 0, ..rect }, factor))
                        }
                        // the blend texture is already BGRA
                        _ => {
                            let b = &self.iview.blend.as_ref().unwrap().1;
                            let view = image::ImageBuffer::<image::Rgba<u8>, &[u8]>::from_raw(iw, ih, b.as_bytes(0).unwrap()).unwrap();
                            bgra(images::upscale_nearest(&view, rect, factor))
                        }
                    },
                };
                keep.push((key, r.clone()));
                crisp[src] = Some((rect, r));
            }
        }
        self.iview.crisp = keep;
        let mut uploads: Vec<_> = img.renders.iter().flatten().cloned().collect();
        uploads.extend(img.diff_render.get().cloned());
        uploads.extend(self.iview.blend.as_ref().map(|b| b.1.clone()));
        uploads.extend(self.iview.crisp.iter().map(|(_, r)| r.clone()));
        self.keep_image_uploads(uploads, window);
        Some(ImageFrame {
            widths:self.comparison_widths(f32::from(window.viewport_size().width)-self.diff_x()),
            img: img.clone(),
            mode: self.iview.state.mode(),
            zoom,
            center,
            split: self.iview.state.split(),
            blend: self.iview.blend.as_ref().map(|b| b.1.clone()),
            difference,
            crisp,
            region: self.iview.state.region(),
            label: format!(
                "{} · {:.0} %   (m mode · 0 fit · 1 1:1 · +/- zoom · , . {} · n/p regions)",
                self.iview.state.mode().name(),
                zoom * 100.0,
                if self.iview.state.mode() == Mode::Onion {
                    "opacity"
                } else {
                    "split"
                }
            ),
        })
    }

    /// Dropping RenderImage's Arc only frees CPU pixels; GPUI's atlas needs an explicit removal.
    fn keep_image_uploads(&mut self, keep: Vec<Arc<RenderImage>>, window: &mut Window) {
        self.image_uploads.keep(keep, window);
    }
}

pub(super) struct ImageFrame {
    widths:[f32;2],
    img: Arc<ImageDoc>,
    mode: Mode,
    zoom: f32,
    center: (f32, f32),
    split: f32,
    blend: Option<Arc<RenderImage>>,
    difference: Option<Arc<RenderImage>>,
    /// Sharp enlargements per source (left, right, difference, blend): the image rect they cover.
    crisp: [Option<(images::Rect, Arc<RenderImage>)>; 4],
    region: Option<usize>,
    label: String,
}


/// Paint the image Comparison on the stage `(x0, y0, w, h)`.
pub(super) fn paint_images(palette: &theme::Palette,
    window: &mut Window,
    cx: &mut App,
    font: &Font,
    (x0, y0, w, h): (f32, f32, f32, f32),
    f: &ImageFrame,
) {
    let p = palette;
    fill(window, x0, y0, w, h, p.bg);
    let pane_w = f.widths[0];
    // where image point (0, 0) lands for a stage centred at (cx0, cy0)
    let place = |img: Option<&Decoded>, cx0: f32, cy0: f32| {
        img.map(|d| {
            let (iw, ih) = (d.pixels.width() as f32, d.pixels.height() as f32);
            // a downscaled image still covers its original size
            let s = if d.downscaled {
                d.width as f32 / iw
            } else {
                1.0
            };
            (
                cx0 - f.center.0 * f.zoom,
                cy0 - f.center.1 * f.zoom,
                iw * s * f.zoom,
                ih * s * f.zoom,
            )
        })
    };
    let draw = |window: &mut Window,
                clip: Bounds<Pixels>,
                at: Option<(f32, f32, f32, f32)>,
                r: Option<&Arc<RenderImage>>,
                src: usize| {
        let (Some((ix, iy, iw, ih)), Some(r)) = (at, r) else {
            return;
        };
        checker(window, clip, (ix, iy, iw, ih));
        let (image_bounds, r) = match &f.crisp[src] {
            Some((rect, crisp)) => {
                let (x, y) = (ix + rect.x as f32 * f.zoom, iy + rect.y as f32 * f.zoom);
                (
                    Bounds {
                        origin: point(px(x), px(y)),
                        size: size(px(rect.w as f32 * f.zoom), px(rect.h as f32 * f.zoom)),
                    },
                    crisp,
                )
            }
            None => (
                Bounds {
                    origin: point(px(ix), px(iy)),
                    size: size(px(iw), px(ih)),
                },
                r,
            ),
        };
        let _ = window.paint_image(clip, image_bounds, Default::default(), r.clone(), 0, false);
    };
    let bounds = |x: f32, w: f32| Bounds {
        origin: point(px(x), px(y0)),
        size: size(px(w), px(h)),
    };
    let absent = |window: &mut Window, cx: &mut App, x: f32, w: f32| {
        text(
            window,
            cx,
            font,
            x + w / 2.0 - 30.0,
            y0 + h / 2.0,
            "(absent)",
            p.dim,
        )
    };
    let regions = |window: &mut Window, at: (f32, f32, f32, f32)| {
        for (i, r) in f.img.diff.regions.iter().enumerate() {
            let (ox, oy) = (at.0 + r.x as f32 * f.zoom, at.1 + r.y as f32 * f.zoom);
            let (rw, rh) = (r.w as f32 * f.zoom, r.h as f32 * f.zoom);
            let c = if Some(i) == f.region {
                0xFF5ACD
            } else {
                0xE0C46C
            };
            for (x, y, w, h) in [
                (ox - 2.0, oy - 2.0, rw + 4.0, 2.0),
                (ox - 2.0, oy + rh, rw + 4.0, 2.0),
                (ox - 2.0, oy, 2.0, rh),
                (ox + rw, oy, 2.0, rh),
            ] {
                fill(window, x, y, w, h, c);
            }
        }
    };
    let stage = bounds(x0, w);
    match f.mode {
        Mode::TwoUp => {
            for (i, px0) in [(0, x0), (1, x0 + pane_w + GUTTER_W)] {
                let width=f.widths[i];
                let clip = bounds(px0, width);
                let at = place(f.img.sides[i].as_ref(), px0 + width / 2.0, y0 + h / 2.0);
                window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
                    draw(window, clip, at, f.img.renders[i].as_ref(), i);
                    if at.is_none() {
                        absent(window, cx, px0, width);
                    }
                    if let (Some(at), Some(_)) = (at, f.region) {
                        regions(window, at);
                    }
                });
            }
            fill(window, x0 + pane_w, y0, GUTTER_W, h, p.gutter_bg);
        }
        Mode::Swipe => {
            let split = x0 + w * f.split;
            let (cx0, cy0) = (x0 + w / 2.0, y0 + h / 2.0);
            window.with_content_mask(Some(ContentMask { bounds: stage }), |window| {
                draw(
                    window,
                    bounds(x0, split - x0),
                    place(f.img.sides[0].as_ref(), cx0, cy0),
                    f.img.renders[0].as_ref(),
                    0,
                );
                draw(
                    window,
                    bounds(split, x0 + w - split),
                    place(f.img.sides[1].as_ref(), cx0, cy0),
                    f.img.renders[1].as_ref(),
                    1,
                );
            });
            fill(window, split - 1.0, y0, 2.0, h, 0xE0C46C);
            text(window, cx, font, split - 46.0, y0 + 40.0, "old", p.dim);
            text(window, cx, font, split + 10.0, y0 + 40.0, "new", p.dim);
        }
        Mode::Onion | Mode::Difference => {
            let (cx0, cy0) = (x0 + w / 2.0, y0 + h / 2.0);
            let d = f.img.sides[1].as_ref().or(f.img.sides[0].as_ref());
            let at = place(d, cx0, cy0).map(|(x, y, _, _)| {
                (
                    x,
                    y,
                    f.img.size.0 as f32 * f.zoom,
                    f.img.size.1 as f32 * f.zoom,
                )
            });
            let (r, src) = if f.mode == Mode::Onion {
                (f.blend.as_ref(), 3)
            } else {
                (f.difference.as_ref(), 2)
            };
            window.with_content_mask(Some(ContentMask { bounds: stage }), |window| {
                draw(window, stage, at, r, src);
                if let (Mode::Difference, Some(at)) = (f.mode, at) {
                    regions(window, at);
                }
            });
        }
    }
    // the mode and zoom line
    let lw = f.label.chars().count() as f32 * 7.8 + 16.0;
    fill(window, x0 + 8.0, y0 + 8.0, lw, line_h(), p.status_bg);
    text(window, cx, font, x0 + 16.0, y0 + 8.0, &f.label, p.fg);
}

#[cfg(all(test, feature = "gui-test"))]
mod tests {
    use super::super::parity_tests::{fixture_named, key, workflow_wait};
    use super::*;
    use gpui::TestAppContext;

    fn bgra(p: [u8; 4]) -> [u8; 4] {
        [p[2], p[1], p[0], p[3]]
    }
    fn texel(r: &RenderImage, x: u32, y: u32) -> [u8; 4] {
        let w = r.size(0).width.0 as usize;
        let i = (y as usize * w + x as usize) * 4;
        r.as_bytes(0).unwrap()[i..i + 4].try_into().unwrap()
    }

    /// Difference and Onion skin are drawn from textures built on demand; their sharp enlargements
    /// must show the same pixels, in gpui's BGRA order.
    #[gpui::test]
    fn gui_difference_and_onion_textures_match_the_shared_images(cx: &mut TestAppContext) {
        let f = fixture_named(cx, "old", "new", "txt");
        let (lp, rp) = (f.dir.path().join("left.png"), f.dir.path().join("right.png"));
        let old = RgbaImage::from_pixel(8, 6, image::Rgba([100, 120, 140, 255]));
        let mut new = old.clone();
        new.put_pixel(2, 3, image::Rgba([250, 10, 20, 255]));
        old.save(&lp).unwrap();
        new.save(&rp).unwrap();
        let spec = Spec::Plain { left: lp, right: rp };
        let set = ChangeSet::load(&spec, f.dir.path()).unwrap();
        f.viewer.update(cx, |v, cx| {
            v.spec = spec;
            v.show_change_set(set, cx);
        });
        workflow_wait(cx, &f, |v| v.image_doc().is_some());
        let frame = |cx: &mut TestAppContext| f.window.update(cx, |v, w, _| v.image_frame(w).unwrap()).unwrap();
        for _ in 0..3 {
            key(cx, &f, "+", false, false, false); // 337 %: sharp enlargements
        }
        // 2-up → Swipe → Onion skin
        key(cx, &f, "m", false, false, false);
        key(cx, &f, "m", false, false, false);
        let onion = frame(cx);
        assert_eq!(onion.mode, Mode::Onion);
        let blend = images::blend(&old, &new, 0.5);
        let texture = onion.blend.as_ref().unwrap();
        assert_eq!(texel(texture, 2, 3), bgra(blend.get_pixel(2, 3).0));
        let (rect, crisp) = onion.crisp[3].as_ref().unwrap();
        assert_eq!(texel(crisp, 0, 0), bgra(blend.get_pixel(rect.x, rect.y).0));
        // → Difference
        key(cx, &f, "m", false, false, false);
        let difference = frame(cx);
        assert_eq!(difference.mode, Mode::Difference);
        let expected = images::difference(&old, &new, images::Rect { x: 0, y: 0, w: 8, h: 6 });
        let texture = difference.difference.as_ref().unwrap();
        for (x, y) in [(0, 0), (2, 3), (7, 5)] {
            assert_eq!(texel(texture, x, y), bgra(expected.get_pixel(x, y).0));
        }
        let (rect, crisp) = difference.crisp[2].as_ref().unwrap();
        assert_eq!(texel(crisp, 0, 0), bgra(expected.get_pixel(rect.x, rect.y).0));
        let factor = crisp.size(0).width.0 as u32 / rect.w;
        assert_eq!(texel(crisp, (2 - rect.x) * factor, (3 - rect.y) * factor), bgra(expected.get_pixel(2, 3).0));
    }
}
