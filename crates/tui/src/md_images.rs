//! Local images of the Markdown Preview: found and decoded in the background once per document (the shared
//! `image_sources` / `decode_image`), laid out as cell boxes and drawn as half-block colour cells.
use lenz::image_session::Pixels;
use lenz::markdown::ParsedPair;
use ratatui::{buffer::Buffer, layout::Rect, style::{Color, Style}};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Weak, mpsc::Receiver};

pub(crate) type Picture = Arc<Pixels>;
/// kitty's image placeholder: a cell showing part of an image, its row and column given by diacritics and the image
/// id by the foreground colour (kitty graphics protocol, Unicode placeholders; kitty and Ghostty).
const PLACEHOLDER: char = '\u{10EEEE}';
/// The protocol's row/column diacritics, from kitty's `gen/rowcolumn-diacritics.txt`: the n-th encodes n.
const DIACRITICS: [u32; 297] = [
    0x0305, 0x030D, 0x030E, 0x0310, 0x0312, 0x033D, 0x033E, 0x033F, 0x0346, 0x034A, 0x034B, 0x034C, 0x0350, 0x0351,
    0x0352, 0x0357, 0x035B, 0x0363, 0x0364, 0x0365, 0x0366, 0x0367, 0x0368, 0x0369, 0x036A, 0x036B, 0x036C, 0x036D,
    0x036E, 0x036F, 0x0483, 0x0484, 0x0485, 0x0486, 0x0487, 0x0592, 0x0593, 0x0594, 0x0595, 0x0597, 0x0598, 0x0599,
    0x059C, 0x059D, 0x059E, 0x059F, 0x05A0, 0x05A1, 0x05A8, 0x05A9, 0x05AB, 0x05AC, 0x05AF, 0x05C4, 0x0610, 0x0611,
    0x0612, 0x0613, 0x0614, 0x0615, 0x0616, 0x0617, 0x0657, 0x0658, 0x0659, 0x065A, 0x065B, 0x065D, 0x065E, 0x06D6,
    0x06D7, 0x06D8, 0x06D9, 0x06DA, 0x06DB, 0x06DC, 0x06DF, 0x06E0, 0x06E1, 0x06E2, 0x06E4, 0x06E7, 0x06E8, 0x06EB,
    0x06EC, 0x0730, 0x0732, 0x0733, 0x0735, 0x0736, 0x073A, 0x073D, 0x073F, 0x0740, 0x0741, 0x0743, 0x0745, 0x0747,
    0x0749, 0x074A, 0x07EB, 0x07EC, 0x07ED, 0x07EE, 0x07EF, 0x07F0, 0x07F1, 0x07F3, 0x0816, 0x0817, 0x0818, 0x0819,
    0x081B, 0x081C, 0x081D, 0x081E, 0x081F, 0x0820, 0x0821, 0x0822, 0x0823, 0x0825, 0x0826, 0x0827, 0x0829, 0x082A,
    0x082B, 0x082C, 0x082D, 0x0951, 0x0953, 0x0954, 0x0F82, 0x0F83, 0x0F86, 0x0F87, 0x135D, 0x135E, 0x135F, 0x17DD,
    0x193A, 0x1A17, 0x1A75, 0x1A76, 0x1A77, 0x1A78, 0x1A79, 0x1A7A, 0x1A7B, 0x1A7C, 0x1B6B, 0x1B6D, 0x1B6E, 0x1B6F,
    0x1B70, 0x1B71, 0x1B72, 0x1B73, 0x1CD0, 0x1CD1, 0x1CD2, 0x1CDA, 0x1CDB, 0x1CE0, 0x1DC0, 0x1DC1, 0x1DC3, 0x1DC4,
    0x1DC5, 0x1DC6, 0x1DC7, 0x1DC8, 0x1DC9, 0x1DCB, 0x1DCC, 0x1DD1, 0x1DD2, 0x1DD3, 0x1DD4, 0x1DD5, 0x1DD6, 0x1DD7,
    0x1DD8, 0x1DD9, 0x1DDA, 0x1DDB, 0x1DDC, 0x1DDD, 0x1DDE, 0x1DDF, 0x1DE0, 0x1DE1, 0x1DE2, 0x1DE3, 0x1DE4, 0x1DE5,
    0x1DE6, 0x1DFE, 0x20D0, 0x20D1, 0x20D4, 0x20D5, 0x20D6, 0x20D7, 0x20DB, 0x20DC, 0x20E1, 0x20E7, 0x20E9, 0x20F0,
    0x2CEF, 0x2CF0, 0x2CF1, 0x2DE0, 0x2DE1, 0x2DE2, 0x2DE3, 0x2DE4, 0x2DE5, 0x2DE6, 0x2DE7, 0x2DE8, 0x2DE9, 0x2DEA,
    0x2DEB, 0x2DEC, 0x2DED, 0x2DEE, 0x2DEF, 0x2DF0, 0x2DF1, 0x2DF2, 0x2DF3, 0x2DF4, 0x2DF5, 0x2DF6, 0x2DF7, 0x2DF8,
    0x2DF9, 0x2DFA, 0x2DFB, 0x2DFC, 0x2DFD, 0x2DFE, 0x2DFF, 0xA66F, 0xA67C, 0xA67D, 0xA6F0, 0xA6F1, 0xA8E0, 0xA8E1,
    0xA8E2, 0xA8E3, 0xA8E4, 0xA8E5, 0xA8E6, 0xA8E7, 0xA8E8, 0xA8E9, 0xA8EA, 0xA8EB, 0xA8EC, 0xA8ED, 0xA8EE, 0xA8EF,
    0xA8F0, 0xA8F1, 0xAAB0, 0xAAB2, 0xAAB3, 0xAAB7, 0xAAB8, 0xAABE, 0xAABF, 0xAAC1, 0xFE20, 0xFE21, 0xFE22, 0xFE23,
    0xFE24, 0xFE25, 0xFE26, 0x10A0F, 0x10A38, 0x1D185, 0x1D186, 0x1D187, 0x1D188, 0x1D189, 0x1D1AA, 0x1D1AB,
    0x1D1AC, 0x1D1AD, 0x1D242, 0x1D243, 0x1D244,
];
/// Image ids are per process, so independent views never share one.
static NEXT_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

/// Whether the terminal shows kitty graphics placeholders: kitty or Ghostty, outside tmux (which needs passthrough),
/// unless `LENZ_IMAGES=cells` (or `kitty` to force them).
pub(crate) fn kitty_supported(env: impl Fn(&str) -> Option<String>) -> bool {
    match env("LENZ_IMAGES").as_deref() {
        Some("cells" | "sixel") => return false,
        Some("kitty") => return true,
        _ => {}
    }
    if env("TMUX").is_some() { return false; }
    env("KITTY_WINDOW_ID").is_some()
        || matches!(env("TERM").as_deref(), Some("xterm-kitty" | "xterm-ghostty"))
        || env("TERM_PROGRAM").as_deref() == Some("ghostty")
}

/// Images sent to the terminal: one per picture and cell box, and the escapes still to write.
#[derive(Default)]
struct Kitty {
    placed: HashMap<(usize, u16, u16), u32>,
    queue: Vec<String>,
}
impl Kitty {
    fn forget(&mut self, keep: impl Fn(&(usize, u16, u16)) -> bool) {
        let queue = &mut self.queue;
        self.placed.retain(|key, id| keep(key) || { queue.push(format!("\x1b_Ga=d,d=I,i={id},q=2\x1b\\")); false });
    }
}

/// The escapes that send `picture` as image `id` (RGBA scaled down to at most sixteen by thirty-two pixels a cell,
/// base64 in chunks of at most 4096), then place it virtually over `cols` × `rows` cells. Sending and placing
/// are separate commands: a combined transmit-and-display also shows the image at the cursor.
fn transmit(id: u32, picture: &Pixels, cols: u16, rows: u16) -> Vec<String> {
    let (w, h) = (picture.width().max(1), picture.height().max(1));
    let scale = (f64::from(cols) * 16. / f64::from(w)).min(f64::from(rows) * 32. / f64::from(h)).min(1.);
    let (tw, th) = (((f64::from(w) * scale) as u32).max(1), ((f64::from(h) * scale) as u32).max(1));
    let mut rgba = Vec::with_capacity((tw * th * 4) as usize);
    for y in 0..th {
        for x in 0..tw {
            rgba.extend_from_slice(&picture.get_pixel((u64::from(x) * u64::from(w) / u64::from(tw)) as u32, (u64::from(y) * u64::from(h) / u64::from(th)) as u32).0);
        }
    }
    let data = crate::clipboard::base64(&rgba);
    let chunks: Vec<&str> = data.as_bytes().chunks(4096).map(|c| std::str::from_utf8(c).unwrap_or_default()).collect();
    let mut out: Vec<String> = chunks.iter().enumerate().map(|(n, chunk)| {
        let more = u8::from(n + 1 < chunks.len());
        if n == 0 { format!("\x1b_Ga=t,i={id},q=2,f=32,s={tw},v={th},m={more};{chunk}\x1b\\") }
        else { format!("\x1b_Gm={more};{chunk}\x1b\\") }
    }).collect();
    out.push(format!("\x1b_Ga=p,U=1,i={id},c={cols},r={rows},q=2\x1b\\"));
    out
}
/// The tallest an image is drawn, in rows.
const MAX_ROWS: u16 = 20;

#[derive(Default)]
pub(crate) struct Images {
    pair: Option<Weak<ParsedPair>>,
    sources: [Vec<(usize, PathBuf)>; 2],
    decoded: HashMap<PathBuf, Picture>,
    jobs: Vec<(PathBuf, Receiver<Option<Option<Pixels>>>)>,
    token: Arc<()>,
    bytes: usize,
    /// Bumped when an image arrives, so the layout makes room for it.
    pub(crate) revision: u64,
    /// Kitty graphics, when the terminal has them; the image id of each shown (document, block).
    kitty: Option<Kitty>,
    ids: HashMap<(usize, usize), u32>,
}

impl Images {
    /// Start decoding the images of `pair` (comparison `current` of `set`) unless they are already loaded or loading.
    pub(crate) fn load(&mut self, pair: &Arc<ParsedPair>, set: &lenz::changeset::ChangeSet, current: usize, spec: &lenz::changeset::Spec, cwd: &std::path::Path) {
        if self.pair.as_ref().is_some_and(|p| Weak::ptr_eq(p, &Arc::downgrade(pair))) { return; }
        self.clear();
        self.pair = Some(Arc::downgrade(pair));
        self.sources = lenz::markdown::image_sources(pair, set, current, spec, cwd);
        let mut paths: Vec<PathBuf> = self.sources.iter().flatten().map(|(_, p)| p.clone()).collect();
        paths.sort();
        paths.dedup();
        for path in paths {
            let file = path.clone();
            self.jobs.push((path, lenz::work::submit(Arc::downgrade(&self.token), move || lenz::markdown::decode_image(&file))));
        }
    }
    /// Take decoded images; true when one arrived.
    pub(crate) fn poll(&mut self) -> bool {
        let mut arrived = false;
        let (decoded, bytes) = (&mut self.decoded, &mut self.bytes);
        self.jobs.retain(|(path, job)| match job.try_recv() {
            Ok(result) => {
                // within the preview's memory budget, or not at all
                if let Some(pixels) = result.flatten() && *bytes + pixels.len() <= lenz::markdown::IMAGE_BYTES {
                    *bytes += pixels.len();
                    decoded.insert(path.clone(), Arc::new(pixels));
                    arrived = true;
                }
                false
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => false,
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
        });
        if arrived { self.revision = self.revision.wrapping_add(1); }
        arrived
    }
    pub(crate) fn busy(&self) -> bool { !self.jobs.is_empty() }
    /// Drop the pictures (and the terminal's copies) and cancel decoding; the terminal's kitty support stays.
    pub(crate) fn clear(&mut self) {
        let mut kitty = self.kitty.take();
        if let Some(k) = &mut kitty { k.forget(|_| false); }
        *self = Self { revision: self.revision.wrapping_add(1), kitty, ..Self::default() };
    }
    /// Draw pictures as kitty graphics placeholders from now on.
    pub(crate) fn enable_kitty(&mut self) { self.kitty.get_or_insert_with(Kitty::default); }
    /// The escapes to write to the terminal (images to send or delete), once.
    pub(crate) fn take_graphics(&mut self) -> Vec<String> { self.kitty.as_mut().map(|k| std::mem::take(&mut k.queue)).unwrap_or_default() }
    /// After a layout: images whose cell box is gone are deleted from the terminal; boxes already sent keep their
    /// image. New boxes are sent by [`Images::show`] when first drawn, so off-screen pictures cost nothing.
    pub(crate) fn keep<'a>(&mut self, boxes: impl Iterator<Item = (usize, usize, &'a Picture, u16, u16)>) {
        self.ids.clear();
        let Some(k) = &mut self.kitty else { return };
        let mut used = std::collections::HashSet::new();
        for (side, block, picture, cols, rows) in boxes {
            let key = (Arc::as_ptr(picture) as usize, cols, rows);
            if let Some(&id) = k.placed.get(&key) { self.ids.insert((side, block), id); }
            used.insert(key);
        }
        k.forget(|key| used.contains(key));
    }
    /// The kitty image for a box being drawn, sent to the terminal the first time it is on screen.
    pub(crate) fn show(&mut self, side: usize, block: usize, picture: &Picture, cols: u16, rows: u16) -> Option<u32> {
        if let Some(&id) = self.ids.get(&(side, block)) { return Some(id); }
        let k = self.kitty.as_mut()?;
        let key = (Arc::as_ptr(picture) as usize, cols, rows);
        let id = match k.placed.get(&key) {
            Some(&id) => id,
            None => {
                let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 0xff_ffff + 1;
                k.queue.extend(transmit(id, picture, cols, rows));
                k.placed.insert(key, id);
                id
            }
        };
        self.ids.insert((side, block), id);
        Some(id)
    }
    /// Each side's decoded pictures by image block.
    pub(crate) fn pictures(&self) -> [HashMap<usize, Picture>; 2] {
        std::array::from_fn(|i| self.sources[i].iter().filter_map(|(b, p)| Some((*b, self.decoded.get(p)?.clone()))).collect())
    }
}

/// Columns and rows for a `size` pixel image in `width` columns: cells are about twice as tall as wide, at most
/// [`MAX_ROWS`] rows, never wider than the image at roughly eight pixels a column.
pub(crate) fn cell_box(size: [u32; 2], width: u16) -> (u16, u16) {
    let [w, h] = size.map(|v| v.max(1) as f32);
    let mut cols = (w / 8.).ceil().min(f32::from(width.max(1))).max(1.);
    let mut rows = (cols * h / w / 2.).round().max(1.);
    if rows > f32::from(MAX_ROWS) {
        rows = f32::from(MAX_ROWS);
        cols = (rows * 2. * w / h).round().clamp(1., f32::from(width.max(1)));
    }
    (cols as u16, rows as u16)
}

/// Row `row` of a `cols` × `rows` picture at (x, y), clipped to `clip`: each cell's upper half is the foreground of
/// a `▀`, its lower half the background; transparency blends with `bg`.
#[allow(clippy::too_many_arguments)] // a cell box and the picture it shows
pub(crate) fn draw_row(buf: &mut Buffer, clip: Rect, x: u16, y: u16, row: u16, rows: u16, cols: u16, picture: &Pixels, bg: u32, id: Option<u32>) {
    let (w, h) = (picture.width() as u64, picture.height() as u64);
    if w == 0 || h == 0 || !(clip.y..clip.bottom()).contains(&y) { return; }
    let sample = |cx: u16, half: u64| {
        let px = ((u64::from(cx) * 2 + 1) * w / (u64::from(cols.max(1)) * 2)).min(w - 1);
        let py = ((u64::from(row) * 2 + half) * h / (u64::from(rows.max(1)) * 2)).min(h - 1);
        let [r, g, b, a] = picture.get_pixel(px as u32, py as u32).0;
        let mix = |c: u8, under: u32| ((u32::from(c) * u32::from(a) + under * (255 - u32::from(a))) / 255) as u8;
        Color::Rgb(mix(r, bg >> 16 & 0xff), mix(g, bg >> 8 & 0xff), mix(b, bg & 0xff))
    };
    for cx in 0..cols {
        let column = x.saturating_add(cx);
        if column < clip.x { continue; }
        if column >= clip.right() { break; }
        // a placeholder carries its row and column; past the diacritics' range, colour cells
        if let Some(id) = id
            && let (Some(&r), Some(&c)) = (DIACRITICS.get(usize::from(row)), DIACRITICS.get(usize::from(cx)))
            && let (Some(r), Some(c)) = (char::from_u32(r), char::from_u32(c))
        {
            let symbol: String = [PLACEHOLDER, r, c].into_iter().collect();
            buf[(column, y)].set_symbol(&symbol).set_style(Style::default().fg(Color::Rgb((id >> 16) as u8, (id >> 8) as u8, id as u8)));
            continue;
        }
        buf[(column, y)].set_symbol("▀").set_style(Style::default().fg(sample(cx, 0)).bg(sample(cx, 1)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clearing_deletes_the_images_sent_to_the_terminal() {
        let mut images = Images::default();
        images.enable_kitty();
        let picture: Picture = Arc::new(Pixels::new(16, 16));
        images.keep([(1usize, 0usize, &picture, 2u16, 1u16)].into_iter());
        assert!(images.take_graphics().is_empty(), "nothing is sent before the box is drawn");
        let id = images.show(1, 0, &picture, 2, 1).unwrap();
        assert!(images.take_graphics().concat().contains(&format!("i={id}")));
        images.clear();
        assert!(images.take_graphics().concat().contains(&format!("a=d,d=I,i={id}")), "a closed view or exit deletes them");
    }
    #[test]
    fn cell_box_keeps_the_aspect_and_the_row_limit() {
        assert_eq!(cell_box([400, 200], 80), (50, 13));
        assert_eq!(cell_box([400, 200], 20), (20, 5));
        // tall: twenty rows, narrower
        let (cols, rows) = cell_box([100, 1000], 80);
        assert_eq!(rows, MAX_ROWS);
        assert!((1..=13).contains(&cols));
        assert_eq!(cell_box([0, 0], 0), (1, 1));
    }
}
