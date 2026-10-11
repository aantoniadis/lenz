# Images in gpui (Zed v1.22.0)

Ticket: `.scratch/lenz-0.2/issues/04-images-in-gpui-research.md`. Probe: `prototypes/image-probe/`
(its own workspace, not a Lenz member; no longer in the tree, see
[archived experiments](README.md#archived-experiments)). Screenshot: `docs/research/images-in-gpui-probe.png`.

Sources read: gpui at tag v1.22.0 (`76659a55`): `crates/gpui/src/elements/img.rs`, `assets.rs`,
`platform.rs` (`Image`, `ImageFormat`, `decode_static_image`), `svg_renderer.rs`, `window.rs`
(`paint_image`, `with_content_mask`), `style.rs` (`ObjectFit`), `elements/image_cache.rs`,
`gpui_wgpu/src/wgpu_atlas.rs`; Zed's `Cargo.toml` (workspace `image`/`resvg` features) and
`crates/image_viewer` (how Zed zooms). Everything below was either read there or run in the probe.

## What gpui gives us

**Element.** `img(source)` → `Img` (Styled, InteractiveElement). `ImageSource` has four variants:

| variant | from | decode |
|---|---|---|
| `Resource(Path/Uri/Embedded)` | `&str`, `PathBuf` | gpui, async, `image::guess_format` + SVG fallback |
| `Image(Arc<gpui::Image>)` | `Image::from_bytes(gpui::ImageFormat, Vec<u8>)` | gpui, async background, cached by hash of the bytes |
| `Render(Arc<RenderImage>)` | our own pixels | none: already decoded |
| `Custom(Fn(&mut Window,&mut App) -> Option<Result<Arc<RenderImage>>>)` | closure | ours |

In-memory bytes (git blobs) work through `Image` or `Render`; no file needed.
`gpui::ImageFormat` = Png, Jpeg, Webp, Gif, Svg, Bmp, Tiff, Ico, Pnm. You must name the format;
`image::guess_format` sniffs magic bytes but cannot detect SVG (sniff `<svg` / use the extension).

**RenderImage** (`assets.rs`): `RenderImage::new(impl Into<SmallVec<[image::Frame; 1]>>)` (a `Vec<Frame>`
works), pixels are **BGRA** (swap bytes 0 and 2 of `image`'s RGBA or red/blue invert).
`size(frame)`, `as_bytes(frame)` (BGRA back out), `frame_count()`, `delay(frame)`.
`scale_factor` is `pub(crate)` and 1.0 for raster images: one image pixel = one *logical* pixel.

**Formats and decoding path.** gpui depends on `image 0.25` with Zed's workspace feature set:
`bmp dds exr ff gif hdr ico jpeg png pnm qoi rayon tga tiff webp` (default minus avif), and
`resvg 0.46` with `text system-fonts memmap-fonts raster-images`. Features are additive from gpui,
so Lenz cannot turn any off without forking gpui. All seven 0.2 formats are already compiled into
Lenz today. Decode = `ImageReader::with_format(..).into_decoder()`, EXIF orientation applied,
`into_rgba8()`, swap to BGRA (`platform.rs::decode_static_image`). Verified in the probe: PNG,
JPEG, GIF, WebP, BMP, ICO and SVG all display via both the `Image` route and our own decode.

**SVG.** Colour SVG goes through `img()` (the `svg()` element is the tinted alpha-mask icon path).
`cx.svg_renderer()` is public: `render_single_frame(bytes, scale) -> Arc<RenderImage>` (BGRA,
rasterised at `scale × 2` for smoothness, so a 320×200 SVG gives a 640×400 buffer at
`scale_factor` 2), or `parse_svg(bytes)` once + `render_parsed(&parsed, SvgSize::ScaleFactor(z))` to
re-rasterise crisply when zooming. Text in SVGs renders with system fonts (probe: "after svg").
gpui caps SVG rasters at 8192 px per side (`rasterize_tree`).

**Animated GIF / WebP.** The `Image` and `Resource` routes decode **all** frames eagerly into RAM.
`img` only animates when it has an element id (`.id(..)`), the window is active and reduce-motion
is off; without `.id()` it paints frame 0 (probe: gpui GIF tile stayed on frame 0 across 4 grabs
0.3 s apart). For "first frame only" our own decode is cheaper: `DynamicImage::from_decoder(GifDecoder)`
and `image::load_from_memory` return frame 0 without decoding the rest.

**Scaling / zoom / fit.** `.object_fit(ObjectFit::{Fill, Contain (default), Cover, ScaleDown, None})`.
No zoom primitive: Zed's image viewer sizes the `img` to `px(w × zoom)` inside a scrolled container
(`image_viewer.rs:441,491`, zoom 0.1–20×). `checkerboard(color, size)` exists as a background for
transparency. Sampling is bilinear; there is no nearest-neighbour option (the 64×40 ICO tile in the
screenshot is visibly soft), so pixel-peeping at high zoom needs our own nearest upscale of the
visible region. HiDPI: at scale 1.6 a 320 px image laid out at 320 logical px is upsampled; for
1:1 device pixels size it `px(w / window.scale_factor())`.

**Onion skin.** `opacity` on the `img` itself works (Interactivity paint wraps `paint_image` in
`with_element_opacity`, and `paint_image` stamps the sprite with it):

```rust
div().relative().w(px(W)).h(px(H))
    .child(img(before.clone()).absolute().top_0().left_0().w(px(W)).h(px(H)))
    .child(img(after.clone()).absolute().top_0().left_0().w(px(W)).h(px(H)).opacity(alpha))
```

**Swipe.** `overflow_hidden()` installs a content mask; `paint_image` clips the sprite to it:

```rust
div().relative().w(px(W)).h(px(H))
    .child(full(&before))
    .child(div().absolute().top_0().left_0().h_full().w(px(split)).overflow_hidden().child(full(&after)))
    .child(div().absolute().top_0().left(px(split - 1.)).w(px(2.)).h_full().bg(rgb(0x000000)))
```

(`full(s)` = the absolute W×H `img` above.) The imperative route also exists: `canvas()` +
`window.with_content_mask(Some(ContentMask { bounds }), ..)` + `window.paint_image(bounds,
image_bounds, corners, Arc<RenderImage>, frame, grayscale)`, all `pub`.

**Per-pixel access / difference mode.** Decode with `image` ourselves, keep the `RgbaImage`,
compute a diff buffer, hand it over as a `RenderImage`. Lenz needs a direct
`image = { version = "0.25", default-features = false }` (gpui does not re-export `image` or
`Frame`); it unifies with gpui's copy (one `image 0.25.10` in the lock, nothing new to compile).

```rust
fn to_render(mut buf: image::RgbaImage) -> Arc<gpui::RenderImage> {
    for px in buf.chunks_exact_mut(4) { px.swap(0, 2); } // RGBA -> BGRA
    Arc::new(gpui::RenderImage::new(vec![image::Frame::new(buf)]))
}
// diff: changed pixels magenta over a faded `before` (probe: 10 983 px changed)
for (x, y, p) in out.enumerate_pixels_mut() {
    *p = match (a.get_pixel_checked(x, y), b.get_pixel_checked(x, y)) {
        (Some(pa), Some(pb)) if pa == pb => Rgba([pa[0], pa[1], pa[2], pa[3] / 4]),
        _ => Rgba([255, 0, 255, 255]),
    };
}
```

A pixel diff is ~20 lines and the same decode feeds dimensions and format, so it is cheap to include.

## Limits and memory

- **Max dimension.** Each `RenderImage` frame is uploaded as one atlas tile. The wgpu atlas grows a
  texture up to `device.limits().max_texture_dimension_2d` (requested via
  `downlevel_defaults().using_resolution(adapter.limits())`, so the adapter's real limit, typically
  8192–16384). A larger image makes `allocate` return `None` → `paint_image` errors
  ("failed to allocate"), `img` logs it and paints **nothing**. No tiling fallback. The limit is not
  exposed publicly: downscale anything over 8192 px per side ourselves before building the
  `RenderImage` (gpui does the same for SVG).
- **Memory.** 4 B/px in CPU (the frames inside `RenderImage`) plus 4 B/px in the GPU atlas:
  a 4000×3000 photo ≈ 48 MB + 48 MB; an all-frames GIF multiplies by the frame count. Keeping our
  own `RgbaImage` for the diff adds another 4 B/px per side. `image`'s default decoder `Limits`
  (512 MiB max allocation) guard against decompression bombs; keep them on.
- **Lifetime.** Atlas tiles are never evicted automatically. For `Render` images call
  `cx.drop_image(arc, Some(window))` when a file is left; for the `Image` route also
  `ImageSource::remove_asset(cx)` (the asset cache keeps the decoded result until removed).
  `ImageCache`/`RetainAllImageCache` only manage `Resource` sources, not in-memory bytes.
- **Orientation.** gpui applies EXIF orientation; `image::load_from_memory` does **not**. Our decode
  must do `let o = decoder.orientation()?; img.apply_orientation(o)` like gpui does.
- **Async.** The `Image` route decodes on the background executor and paints nothing on the first
  frame. Our own decode should also run in `cx.background_spawn` for big images.

## Binary size and build time (measured)

`cargo build --release -p lenz` in this worktree, `strip`ped: **58 265 048 B (58.3 MB)**. Today
Lenz links none of the image decoders (no `img()` use, so they are dead-stripped), even though
`image`, `zune-jpeg`, `image-webp`, `png`, `gif`, `tiff`, `exr`, `resvg`/`usvg` already compile as gpui
dependencies.

Probe variants (same gpui, `--release`, stripped, `prototypes/image-probe` features):

| variant | stripped bytes | Δ vs baseline |
|---|---:|---:|
| baseline (window + text, no images) | 27 303 976 | — |
| `gpui_route` (`img(Arc<Image>)`, 7 formats + SVG) | 30 355 784 | +3.05 MB |
| `own_route,explicit_codecs` (6 named decoders + SVG + diff) | 30 608 840 | +3.30 MB |
| `own_route` (`image::load_from_memory`, every enabled codec) | 30 970 864 | +3.67 MB |
| `gpui_route,own_route` (everything) | 30 972 016 | +3.67 MB |

Where the ~3.7 MB goes (symbol bytes, full vs baseline, KB): image_webp 435, zune_jpeg 381,
image 292, exr 228, tiny_skia 177, tiff 128, png 89, resvg 70, gpui 66, weezl 28, fdeflate 19, gif 17.
Naming only the six 0.2 decoders instead of `load_from_memory` saves ~360 KB (exr, tiff, extra
`image` glue): the whole "cost per extra format" lever, and it needs no gpui fork.

Build time: zero new crates (every codec is already in Lenz's 498-crate graph); each probe variant
re-linked in 1–2 s. The direct `image` dependency adds nothing to compile.

**Projection:** Lenz with images ≈ 58.3 + 3.7 ≈ **62 MB stripped**, ~18 MB under the 80 MB budget.

## Recommendation

1. One decode path owned by Lenz: direct `image 0.25` dep (default-features off), decode the blob
   bytes with `ImageReader::new(Cursor).with_guessed_format()` → `into_decoder()` → orientation →
   first frame → `RgbaImage`; accept only PNG/JPEG/GIF/WebP/BMP/ICO (everything else keeps the size
   placeholder). SVG (by extension or `<svg` sniff) through `cx.svg_renderer()`. Run it on the
   background executor; cap each side at 8192 px (downscale, note it in the metadata line).
2. Display with `img(Arc<RenderImage>)` (`ImageSource::Render`): side by side with `ObjectFit::Contain`
   on a `checkerboard` background; onion skin = `.opacity(α)` on the top `img`; swipe =
   `overflow_hidden()` div of width `split`. Size/dimensions/format come from the blob length and the
   decoder; no second decode.
3. Include the pixel-difference mode: the decoded `RgbaImage`s are already in hand, the diff is a
   loop plus `to_render`.
4. Call `cx.drop_image` for every `RenderImage` when the window leaves an image file.
5. Don't use the `Image` route for 0.2: it decodes all GIF frames, keeps an asset-cache entry per
   blob until removed, and gives no pixel access. Skip a gpui fork for codec trimming (≤360 KB).
6. Open question for the spec: zoom beyond fit (bilinear only; nearest-neighbour would need our own
   upscale of the visible region).

## Reproduce

Check out the image-probe archive from [archived experiments](README.md#archived-experiments), then:

```sh
cd prototypes/image-probe
CARGO_TARGET_DIR=../../target cargo build --release            # all features
CARGO_TARGET_DIR=../../target cargo build --release --no-default-features   # baseline
../../target/release/image-probe                               # window, app_id image-probe
```

Screenshot taken with `grim -g "<at> <size>"` of the focused `image-probe` window on Hyprland
(output scale 1.6). Test images were made with ImageMagick from `assets/before.png`/`after.png`;
`after.gif` has two frames (after, then before).
