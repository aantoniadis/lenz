//! Image comparison data and semantic commands, independent of display resources.
use image::RgbaImage;
use std::path::Path;
/// Decoded raster buffer, exported for adapters without an image dependency.
pub type Pixels = RgbaImage;
use crate::{
    images::{self, Decoded, Format},
    view::Side,
};
fn kilobytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} bytes")
    } else {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    }
}
/// Comparison modes.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Mode {
    #[default]
    TwoUp,
    Swipe,
    Onion,
    Difference,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::TwoUp => "2-up",
            Mode::Swipe => "Swipe",
            Mode::Onion => "Onion skin",
            Mode::Difference => "Difference",
        }
    }

    pub fn next(self) -> Mode {
        match self {
            Mode::TwoUp => Mode::Swipe,
            Mode::Swipe => Mode::Onion,
            Mode::Onion => Mode::Difference,
            Mode::Difference => Mode::TwoUp,
        }
    }
}

/// Both Sides as images (an absent Side is `None`), and their decoded pixel difference.
pub struct ImageDoc {
    pub sides: [Option<Decoded>; 2],
    pub diff: images::Diff,
    /// Union of both sizes, in image pixels.
    pub size: (u32, u32),
}

impl ImageDoc {
    /// Decode two Sides when they are images (or one is absent); `None` when they aren't.
    pub fn read(
        left: (&[u8], &Path, bool),
        right: (&[u8], &Path, bool),
    ) -> Option<Result<ImageDoc, String>> {
        let format = |(bytes, path, binary): (&[u8], &Path, bool)| -> Option<Option<Format>> {
            if bytes.is_empty() {
                return Some(None); // an absent Side
            }
            images::detect_file(&path.to_string_lossy(), bytes, binary).map(Some)
        };
        let (fl, fr) = (format(left)?, format(right)?);
        if fl.is_none() && fr.is_none() {
            return None;
        }
        let decode =
            |bytes: &[u8], f: Option<Format>| f.map(|f| images::decode(bytes, f)).transpose();
        let sides = match (decode(left.0, fl), decode(right.0, fr)) {
            (Ok(l), Ok(r)) => [l, r],
            (Err(e), _) | (_, Err(e)) => return Some(Err(e)),
        };
        let none = RgbaImage::new(0, 0);
        let [old, new] = [0, 1].map(|i| sides[i].as_ref().map_or(&none, |d| &d.pixels));
        let diff = images::diff(old, new);
        let size = (old.width().max(new.width()), old.height().max(new.height()));
        Some(Ok(ImageDoc { sides, diff, size }))
    }

    /// The Difference image of `rect`, built when shown rather than kept.
    pub fn difference(&self, rect: images::Rect) -> RgbaImage {
        let none = RgbaImage::new(0, 0);
        let px = |i: usize| self.sides[i].as_ref().map_or(&none, |d| &d.pixels);
        images::difference(px(0), px(1), rect)
    }

    /// One pixel of the Difference image; transparent outside it.
    pub fn difference_pixel(&self, x: u32, y: u32) -> [u8; 4] {
        if x >= self.size.0 || y >= self.size.1 {
            return [0; 4];
        }
        let px = |i: usize| self.sides[i].as_ref().and_then(|d| d.pixels.get_pixel_checked(x, y));
        images::difference_pixel(px(0), px(1)).0
    }

    /// The header line of a Side: format, dimensions (old ones when they changed), byte size, size change.
    pub fn info(&self, side: Side) -> Option<String> {
        let i = if side == Side::Left { 0 } else { 1 };
        let d = self.sides[i].as_ref()?;
        let mut s = format!(
            "{} · {}×{} · {}",
            d.format.name(),
            d.width,
            d.height,
            kilobytes(d.byte_size as u64)
        );
        if let (Side::Right, Some(old)) = (side, &self.sides[0]) {
            if (old.width, old.height) != (d.width, d.height) {
                s += &format!(" (was {}×{})", old.width, old.height);
            }
            let delta = d.byte_size as i64 - old.byte_size as i64;
            if delta != 0 {
                s += &format!(
                    " · {}{}",
                    if delta > 0 { "+" } else { "−" },
                    kilobytes(delta.unsigned_abs())
                );
            }
        }
        Some(s)
    }

    pub fn notes(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.sides.iter().flatten().any(|d| d.downscaled) {
            out.push(format!(
                "shown downscaled to {} px a side",
                images::MAX_SIDE
            ));
        }
        let pct = 100.0 * self.diff.changed as f64 / self.diff.total.max(1) as f64;
        out.push(format!(
            "{} px changed ({pct:.2} %) · {} regions",
            self.diff.changed,
            self.diff.regions.len()
        ));
        out
    }
}

/// Available image-space extent in display units. Adapters choose pane dimensions.
#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ImageCommand {
    CycleMode,
    SetMode(Mode),
    Fit,
    ActualSize,
    Nudge(f32),
    /// Display offset from the pane centre; preserves the image point under it.
    Zoom {
        factor: f32,
        offset: (f32, f32),
    },
    /// Drag delta in display units, shared by both panes.
    Pan {
        delta: (f32, f32),
    },
    SetSplit(f32),
    Region {
        forward: bool,
    },
}

/// Shared image menu entry. Adapters present these labels and dispatch the semantic command.
#[derive(Debug, Clone, Copy)]
pub struct ImageMenuEntry {
    pub label: &'static str,
    pub command: ImageCommand,
}
impl ImageMenuEntry {
    pub fn disabled(&self, doc: &ImageDoc, state: &ImageState) -> Option<&'static str> {
        match self.command {
            ImageCommand::Region { .. } if doc.diff.regions.is_empty() => {
                Some("No changed regions")
            }
            ImageCommand::SetMode(mode) if state.mode() == mode => Some("Current image mode"),
            _ => None,
        }
    }
}
pub fn image_commands() -> &'static [ImageMenuEntry] {
    const ENTRIES: &[ImageMenuEntry] = &[
        ImageMenuEntry {
            label: "Cycle image mode",
            command: ImageCommand::CycleMode,
        },
        ImageMenuEntry {
            label: "Image: 2-up",
            command: ImageCommand::SetMode(Mode::TwoUp),
        },
        ImageMenuEntry {
            label: "Image: Swipe",
            command: ImageCommand::SetMode(Mode::Swipe),
        },
        ImageMenuEntry {
            label: "Image: Onion skin",
            command: ImageCommand::SetMode(Mode::Onion),
        },
        ImageMenuEntry {
            label: "Image: Difference",
            command: ImageCommand::SetMode(Mode::Difference),
        },
        ImageMenuEntry {
            label: "Fit image",
            command: ImageCommand::Fit,
        },
        ImageMenuEntry {
            label: "Image at 1:1",
            command: ImageCommand::ActualSize,
        },
        ImageMenuEntry {
            label: "Zoom image in",
            command: ImageCommand::Zoom {
                factor: 1.5,
                offset: (0.0, 0.0),
            },
        },
        ImageMenuEntry {
            label: "Zoom image out",
            command: ImageCommand::Zoom {
                factor: 1.0 / 1.5,
                offset: (0.0, 0.0),
            },
        },
        ImageMenuEntry {
            label: "Pan image left",
            command: ImageCommand::Pan { delta: (4.0, 0.0) },
        },
        ImageMenuEntry {
            label: "Pan image right",
            command: ImageCommand::Pan { delta: (-4.0, 0.0) },
        },
        ImageMenuEntry {
            label: "Pan image up",
            command: ImageCommand::Pan { delta: (0.0, 4.0) },
        },
        ImageMenuEntry {
            label: "Pan image down",
            command: ImageCommand::Pan { delta: (0.0, -4.0) },
        },
        ImageMenuEntry {
            label: "Decrease split / opacity",
            command: ImageCommand::Nudge(-0.1),
        },
        ImageMenuEntry {
            label: "Increase split / opacity",
            command: ImageCommand::Nudge(0.1),
        },
        ImageMenuEntry {
            label: "Previous changed image region",
            command: ImageCommand::Region { forward: false },
        },
        ImageMenuEntry {
            label: "Next changed image region",
            command: ImageCommand::Region { forward: true },
        },
    ];
    ENTRIES
}

#[derive(Debug, Clone)]
pub struct ImageState {
    mode: Mode,
    zoom: Option<f32>,
    center: Option<(f32, f32)>,
    alpha: f32,
    split: f32,
    region: Option<usize>,
}
impl Default for ImageState {
    fn default() -> Self {
        Self {
            mode: Mode::TwoUp,
            zoom: None,
            center: None,
            alpha: 0.5,
            split: 0.5,
            region: None,
        }
    }
}
impl ImageState {
    pub fn mode(&self) -> Mode {
        self.mode
    }
    pub fn alpha(&self) -> f32 {
        self.alpha
    }
    pub fn split(&self) -> f32 {
        self.split
    }
    pub fn region(&self) -> Option<usize> {
        self.region
    }
    pub fn center(&self, doc: &ImageDoc) -> (f32, f32) {
        self.center
            .unwrap_or((doc.size.0 as f32 / 2.0, doc.size.1 as f32 / 2.0))
    }
    pub fn zoom(&self, doc: &ImageDoc, viewport: Viewport) -> f32 {
        self.zoom.unwrap_or_else(|| {
            (viewport.width / doc.size.0.max(1) as f32)
                .min(viewport.height / doc.size.1.max(1) as f32)
                .min(1.0)
                * 0.95
        })
    }
    /// False only when region navigation has no destination. All state writes live here.
    pub fn execute(&mut self, doc: &ImageDoc, command: ImageCommand, viewport: Viewport) -> bool {
        match command {
            ImageCommand::CycleMode => self.mode = self.mode.next(),
            ImageCommand::SetMode(mode) => self.mode = mode,
            ImageCommand::Fit => {
                self.zoom = None;
                self.center = None;
                self.region = None;
            }
            ImageCommand::ActualSize => self.zoom = Some(1.0),
            ImageCommand::Nudge(delta) => match self.mode {
                Mode::Onion => self.alpha = (self.alpha + delta).clamp(0.0, 1.0),
                Mode::TwoUp | Mode::Swipe | Mode::Difference => {
                    self.split = (self.split + delta).clamp(0.0, 1.0)
                }
            },
            ImageCommand::SetSplit(split) => self.split = split.clamp(0.0, 1.0),
            ImageCommand::Zoom {
                factor,
                offset: (x, y),
            } => {
                let z = self.zoom(doc, viewport);
                let nz = (z * factor).clamp(0.05, 32.0);
                let (cx, cy) = self.center(doc);
                self.center = Some((cx + x / z - x / nz, cy + y / z - y / nz));
                self.zoom = Some(nz);
            }
            ImageCommand::Pan { delta: (x, y) } => {
                let z = self.zoom(doc, viewport);
                let (cx, cy) = self.center(doc);
                self.center = Some((cx - x / z, cy - y / z));
            }
            ImageCommand::Region { forward } => {
                let n = doc.diff.regions.len();
                if n == 0 {
                    return false;
                }
                let i = match (self.region, forward) {
                    (None, true) => 0,
                    (None, false) => n - 1,
                    (Some(i), true) => (i + 1) % n,
                    (Some(i), false) => (i + n - 1) % n,
                };
                let r = doc.diff.regions[i];
                self.region = Some(i);
                self.zoom = Some(
                    (viewport.width / (r.w as f32 * 2.5))
                        .min(viewport.height / (r.h as f32 * 2.5))
                        .clamp(0.05, 8.0),
                );
                self.center = Some((r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0));
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn doc() -> ImageDoc {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48"><rect width="24" height="24" fill="#f00"/></svg>"##;
        ImageDoc::read(
            (b"", Path::new("old.svg"), false),
            (svg, Path::new("new.svg"), false),
        )
        .unwrap()
        .unwrap()
    }
    #[test]
    fn modes_pan_fit_and_region_navigation_share_state() {
        let doc = doc();
        let viewport = Viewport {
            width: 80.0,
            height: 24.0,
        };
        let mut s = ImageState::default();
        assert!(doc.sides[0].is_none());
        assert!(doc.diff.changed > 0);
        assert_eq!(doc.size, (48, 48));
        let whole = doc.difference(images::Rect { x: 0, y: 0, w: 48, h: 48 });
        for (x, y) in [(0, 0), (30, 30), (47, 47)] {
            assert_eq!(doc.difference_pixel(x, y), whole.get_pixel(x, y).0);
        }
        assert_eq!(doc.difference_pixel(48, 0), [0; 4]);
        let fit = s.zoom(&doc, viewport);
        s.execute(&doc, ImageCommand::ActualSize, viewport);
        s.execute(&doc, ImageCommand::Pan { delta: (3.0, 4.0) }, viewport);
        assert_eq!(s.center(&doc), (21.0, 20.0));
        s.execute(&doc, ImageCommand::Region { forward: false }, viewport);
        assert_eq!(s.region(), Some(0));
        s.execute(&doc, ImageCommand::Fit, viewport);
        assert_eq!(s.zoom(&doc, viewport), fit);
        assert_eq!(s.center(&doc), (24.0, 24.0));
        for mode in [Mode::Swipe, Mode::Onion, Mode::Difference, Mode::TwoUp] {
            s.execute(&doc, ImageCommand::CycleMode, viewport);
            assert_eq!(s.mode(), mode);
        }
    }
    #[test]
    fn prepared_loader_preserves_decode_errors_and_large_file_guard() {
        use crate::{
            changeset::{ChangeSet, Spec},
            document::{LARGE, Placeholder, PreparedDocument},
        };
        let dir = tempfile::tempdir().unwrap();
        let left = dir.path().join("old.svg");
        let right = dir.path().join("new.svg");
        std::fs::write(&left, b"").unwrap();
        std::fs::write(&right, b"<svg invalid").unwrap();
        let spec = Spec::Plain {
            left: left.clone(),
            right: right.clone(),
        };
        let set = ChangeSet::load(&spec, dir.path()).unwrap();
        let prepared = PreparedDocument::load(&set, 0, &left, &right, false).unwrap();
        assert!(matches!(prepared.image, Some(Err(_))));
        assert!(matches!(
            prepared.text.placeholder_kind(),
            Some(Placeholder::Binary { .. })
        ));
        assert!(
            prepared
                .text
                .notes()
                .iter()
                .any(|note| note.contains("cannot show as an image"))
        );
        std::fs::OpenOptions::new()
            .write(true)
            .open(&right)
            .unwrap()
            .set_len(LARGE + 1)
            .unwrap();
        let set = ChangeSet::load(&spec, dir.path()).unwrap();
        let prepared = PreparedDocument::load(&set, 0, &left, &right, false).unwrap();
        assert!(prepared.image.is_none());
        assert!(matches!(
            prepared.text.placeholder_kind(),
            Some(Placeholder::Large { .. })
        ));
    }

    #[test]
    fn shared_menu_includes_every_mode_and_disables_empty_region_navigation() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="#f00"/></svg>"##;
        let doc = ImageDoc::read(
            (svg, Path::new("a.svg"), false),
            (svg, Path::new("b.svg"), false),
        )
        .unwrap()
        .unwrap();
        let state = ImageState::default();
        let entries = image_commands();
        assert_eq!(
            entries
                .iter()
                .filter(|e| matches!(e.command, ImageCommand::SetMode(_)))
                .count(),
            4
        );
        for entry in entries {
            assert!(!entry.label.is_empty());
            if matches!(entry.command, ImageCommand::Region { .. }) {
                assert_eq!(entry.disabled(&doc, &state), Some("No changed regions"));
            }
        }
        assert!(doc.notes().iter().any(|n| n.starts_with("0 px changed")));
    }

    #[test]
    fn zoom_preserves_pointer_and_opacity_endpoints() {
        let doc = doc();
        let viewport = Viewport {
            width: 80.0,
            height: 24.0,
        };
        let mut s = ImageState::default();
        s.execute(&doc, ImageCommand::ActualSize, viewport);
        s.execute(
            &doc,
            ImageCommand::Zoom {
                factor: 2.0,
                offset: (4.0, 6.0),
            },
            viewport,
        );
        assert_eq!(s.center(&doc), (26.0, 27.0));
        s.execute(&doc, ImageCommand::CycleMode, viewport);
        s.execute(&doc, ImageCommand::CycleMode, viewport);
        s.execute(&doc, ImageCommand::Nudge(2.0), viewport);
        assert_eq!(s.alpha(), 1.0);
        s.execute(&doc, ImageCommand::Nudge(-2.0), viewport);
        assert_eq!(s.alpha(), 0.0);
    }
}
